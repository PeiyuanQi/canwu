use super::{
    CanwuError, Command, CommandAuthority, CommandEnvelope, CommandIngress, CommandOutcome,
    CommandRequest, CommandRequestId, ControllerDecision, DecisionAction, DecisionAttemptErrorCode,
    DecisionAttemptOutcome, DecisionAttemptRecord, DecisionAuthority, DecisionController,
    DecisionError, DecisionMutation, DecisionPolicy, DecisionPolicyKind, DecisionRequestId,
    DecisionTicket, DecisionTicketId, DecisionTrace, DecisionTraceId, EntityRef, ErrorCode,
    IngressClass, IngressPayload, IngressReceipt, Issuer, SimTime, Simulation, canonical_hash,
    claim_counter, invalid_snapshot_error, runtime_entity_identity_exists,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct DecisionIngressRequest {
    pub request_id: DecisionRequestId,
    pub expected_revision: u64,
    pub mutation: DecisionMutation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<Box<CommandRequest>>,
}

impl DecisionIngressRequest {
    #[must_use]
    pub const fn new(
        request_id: DecisionRequestId,
        expected_revision: u64,
        mutation: DecisionMutation,
    ) -> Self {
        Self {
            request_id,
            expected_revision,
            mutation,
            command: None,
        }
    }

    #[must_use]
    pub fn with_command(mut self, command: CommandRequest) -> Self {
        self.command = Some(Box::new(command));
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum DecisionEvaluation {
    Pending(super::PolicyDecision),
    Prepared(PreparedDecisionIngress),
}

#[derive(Clone, Debug, PartialEq)]
pub struct PreparedDecisionIngress {
    pub request: DecisionIngressRequest,
    pub selected_action: Option<DecisionAction>,
}

impl Simulation {
    #[must_use]
    pub const fn decision_state(&self) -> &super::DecisionState {
        &self.state.current.decisions
    }

    #[must_use]
    pub fn decision_ticket(&self, id: DecisionTicketId) -> Option<&DecisionTicket> {
        self.state.current.decisions.ticket(id)
    }

    #[must_use]
    pub fn decision_traces(&self) -> &[DecisionTrace] {
        &self.state.current.decisions.traces
    }

    #[must_use]
    pub fn decision_attempts(&self) -> &[DecisionAttemptRecord] {
        &self.state.current.decisions.attempts
    }

    pub fn prepare_decision(
        &self,
        decision_request_id: DecisionRequestId,
        command_request_id: Option<CommandRequestId>,
        ticket_id: DecisionTicketId,
        policy: &dyn DecisionPolicy,
    ) -> Result<DecisionEvaluation, CanwuError> {
        self.prepare_decision_at(
            self.state.scheduler.now,
            decision_request_id,
            command_request_id,
            ticket_id,
            policy,
        )
    }

    pub fn prepare_decision_at(
        &self,
        due_at: SimTime,
        decision_request_id: DecisionRequestId,
        command_request_id: Option<CommandRequestId>,
        ticket_id: DecisionTicketId,
        policy: &dyn DecisionPolicy,
    ) -> Result<DecisionEvaluation, CanwuError> {
        self.ensure_runtime_ready()?;
        if due_at < self.state.scheduler.now {
            return Err(CanwuError::new(
                ErrorCode::SimulationTimeConflict,
                "a decision cannot be prepared behind committed simulation time",
            ));
        }
        let ticket = self.decision_ticket(ticket_id).ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidDecision,
                format!("decision ticket {ticket_id} was not found"),
            )
        })?;
        if ticket.deadline.is_some_and(|deadline| deadline < due_at) {
            return Err(CanwuError::new(
                ErrorCode::InvalidDecision,
                format!("decision ticket {ticket_id} has expired"),
            ));
        }
        let controller = self
            .state
            .current
            .decisions
            .controller(&ticket.assigned_controller)
            .ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::InvalidDecision,
                    "decision ticket names an unknown controller",
                )
            })?;
        match DecisionController::evaluate(ticket, controller, policy).map_err(decision_error)? {
            ControllerDecision::Pending(decision) => Ok(DecisionEvaluation::Pending(decision)),
            ControllerDecision::Authoritative { decision, action } => {
                let command = match &action {
                    Some(DecisionAction::Command { command }) => {
                        let request_id = command_request_id.ok_or_else(|| {
                            CanwuError::new(
                                ErrorCode::InvalidDecision,
                                "a selected command option requires a command request ID",
                            )
                        })?;
                        let command: Command =
                            serde_json::from_value(command.clone()).map_err(|error| {
                                CanwuError::new(
                                    ErrorCode::InvalidDecision,
                                    format!("decision option contains an invalid command: {error}"),
                                )
                            })?;
                        Some(CommandRequest::new(
                            request_id,
                            self.revision(),
                            CommandEnvelope::new(controller_issuer(controller), command)
                                .with_authority(controller_authority(controller))
                                .at_time(due_at),
                        ))
                    }
                    Some(DecisionAction::None) | None => {
                        if command_request_id.is_some() {
                            return Err(CanwuError::new(
                                ErrorCode::InvalidDecision,
                                "a non-command option cannot reserve a command request ID",
                            ));
                        }
                        None
                    }
                };
                let mutation = DecisionMutation::Resolve {
                    ticket_id,
                    expected_version: ticket.version,
                    controller_id: controller.id.clone(),
                    policy: controller.policy.clone(),
                    decision,
                    command_request_id,
                };
                let request = DecisionIngressRequest {
                    request_id: decision_request_id,
                    expected_revision: self.revision(),
                    mutation,
                    command: command.map(Box::new),
                };
                Ok(DecisionEvaluation::Prepared(PreparedDecisionIngress {
                    request,
                    selected_action: action,
                }))
            }
        }
    }

    pub fn enqueue_decision(
        &mut self,
        due_at: SimTime,
        priority: i32,
        request: DecisionIngressRequest,
    ) -> Result<IngressReceipt, CanwuError> {
        self.ensure_runtime_ready()?;
        self.ensure_canonical_ingress_can_start()?;
        if self
            .state
            .metadata
            .run_configuration
            .declared()
            .is_some_and(|configuration| {
                configuration.interaction == super::InteractionPolicy::ReadOnly
            })
        {
            return Err(CanwuError::new(
                ErrorCode::InteractionReadOnly,
                "the run interaction policy rejects newly authored decision ingress",
            ));
        }
        if request.request_id.get() == 0 {
            return Err(CanwuError::new(
                ErrorCode::InvalidDecision,
                "decision request IDs must be nonzero",
            ));
        }
        let input_hash = canonical_hash(
            "canwu.ingress.decision-request.v1",
            &(due_at, priority, &request),
        )?;
        if let Some(existing) = self
            .state
            .evidence
            .archived_decision_requests
            .get(&request.request_id)
        {
            if existing.input_hash == input_hash {
                return Ok(existing.receipt.clone());
            }
            return Err(CanwuError::new(
                ErrorCode::IdempotencyConflict,
                format!(
                    "decision request {} is already queued with different content",
                    request.request_id
                ),
            ));
        }
        for record in &self.state.evidence.ingress {
            let IngressPayload::Decision { request: existing } = &record.payload else {
                continue;
            };
            if existing.request_id != request.request_id {
                continue;
            }
            if existing.as_ref() == &request
                && record.due_at == due_at
                && record.priority == priority
            {
                return Ok(IngressReceipt {
                    ingress_id: record.id,
                    issued_at: record.issued_at,
                    due_at: record.due_at,
                });
            }
            return Err(CanwuError::new(
                ErrorCode::IdempotencyConflict,
                format!(
                    "decision request {} is already queued with different content",
                    request.request_id
                ),
            ));
        }
        if request.expected_revision != self.revision() {
            return Err(CanwuError::new(
                ErrorCode::SimulationRevisionConflict,
                format!(
                    "decision request {} expected revision {}, current revision is {}",
                    request.request_id,
                    request.expected_revision,
                    self.revision()
                ),
            ));
        }
        if let Some(command) = &request.command {
            if command.request_id.get() == 0 {
                return Err(CanwuError::new(
                    ErrorCode::InvalidDecision,
                    "nested decision command request IDs must be nonzero",
                ));
            }
            if command.expected_revision != request.expected_revision
                || command.envelope.expected_time != Some(due_at)
            {
                return Err(CanwuError::new(
                    ErrorCode::InvalidDecision,
                    "nested decision command must use the decision request revision and due-time guards",
                ));
            }
            if self.command_request_id_is_in_use(command.request_id) {
                return Err(CanwuError::new(
                    ErrorCode::IdempotencyConflict,
                    format!(
                        "nested decision command request {} is already reserved or processed",
                        command.request_id
                    ),
                ));
            }
        }
        self.append_ingress(
            due_at,
            IngressClass::Decision,
            priority,
            IngressPayload::Decision {
                request: Box::new(request),
            },
            None,
            false,
        )
    }

    pub fn drive_decision(
        &mut self,
        due_at: SimTime,
        priority: i32,
        decision_request_id: DecisionRequestId,
        command_request_id: Option<CommandRequestId>,
        ticket_id: DecisionTicketId,
        policy: &dyn DecisionPolicy,
    ) -> Result<DecisionEvaluation, CanwuError> {
        let evaluation = self.prepare_decision_at(
            due_at,
            decision_request_id,
            command_request_id,
            ticket_id,
            policy,
        )?;
        if let DecisionEvaluation::Prepared(prepared) = &evaluation {
            self.enqueue_decision(due_at, priority, prepared.request.clone())?;
        }
        Ok(evaluation)
    }

    pub(super) fn apply_decision_request(
        &mut self,
        request: DecisionIngressRequest,
    ) -> Result<Option<CommandOutcome>, CanwuError> {
        let decision_request_id = request.request_id;
        let decision_expected_revision = request.expected_revision;
        let revision_before = self.revision();
        if request.expected_revision != self.revision() {
            return Ok(self.record_decision_rejection(
                request.request_id,
                request.expected_revision,
                DecisionAttemptErrorCode::SimulationRevisionConflict,
                format!(
                    "decision request {} expected revision {}, current revision is {}",
                    request.request_id,
                    request.expected_revision,
                    self.revision()
                ),
            ));
        }
        if let Some(command) = &request.command
            && !self.command_request_id_is_unique_for_admitted_decision(command.request_id)
        {
            return Ok(self.record_decision_rejection(
                request.request_id,
                request.expected_revision,
                DecisionAttemptErrorCode::CommandRequestConflict,
                format!(
                    "nested decision command request {} is not unique at admission",
                    command.request_id
                ),
            ));
        }
        if let Err(error) = self.validate_decision_mutation_entities(&request.mutation) {
            return Ok(self.record_decision_rejection(
                request.request_id,
                request.expected_revision,
                DecisionAttemptErrorCode::EntityUnavailable,
                error.message,
            ));
        }
        let trace_claim = if matches!(request.mutation, DecisionMutation::Resolve { .. }) {
            let (id, next_id) = claim_counter(
                self.state.counters.next_decision_trace_id,
                "decision trace ID",
            )?;
            Some((DecisionTraceId::new(id), next_id))
        } else {
            None
        };
        let mut decisions = self.state.current.decisions.clone();
        let prepared = match decisions.apply(
            request.mutation,
            self.state.scheduler.now,
            trace_claim.map(|(id, _)| id),
        ) {
            Ok(prepared) => prepared,
            Err(error) => {
                return Ok(self.record_decision_rejection(
                    request.request_id,
                    request.expected_revision,
                    error.code.into(),
                    error.message,
                ));
            }
        };
        let controller = prepared
            .trace
            .as_ref()
            .and_then(|trace| decisions.controller(&trace.controller_id));
        let decision_controller_id = controller.map(|controller| controller.id.clone());
        match (&prepared.action, &request.command) {
            (Some(DecisionAction::Command { command }), Some(request)) => {
                let expected: Command = match serde_json::from_value(command.clone()) {
                    Ok(command) => command,
                    Err(error) => {
                        return Ok(self.record_decision_rejection(
                            decision_request_id,
                            decision_expected_revision,
                            DecisionAttemptErrorCode::InvalidDecision,
                            format!("decision option contains an invalid command: {error}"),
                        ));
                    }
                };
                if request.envelope.command != expected
                    || request.expected_revision != self.revision()
                    || prepared
                        .trace
                        .as_ref()
                        .and_then(|trace| trace.command_request_id)
                        != Some(request.request_id)
                {
                    return Ok(self.record_decision_rejection(
                        decision_request_id,
                        decision_expected_revision,
                        DecisionAttemptErrorCode::InvalidDecision,
                        "nested command does not match the selected decision option".to_owned(),
                    ));
                }
                let controller = controller.ok_or_else(|| {
                    invalid_snapshot_error("decision trace does not resolve its controller binding")
                })?;
                if request.envelope.issuer != controller_issuer(controller)
                    || request.envelope.authority.as_ref()
                        != Some(&controller_authority(controller))
                    || request.envelope.expected_time != Some(self.state.scheduler.now)
                {
                    return Ok(self.record_decision_rejection(
                        decision_request_id,
                        decision_expected_revision,
                        DecisionAttemptErrorCode::InvalidDecision,
                        "nested command issuer, authority, or time guard was not derived from the decision controller".to_owned(),
                    ));
                }
            }
            (Some(DecisionAction::None) | None, None) => {}
            _ => {
                return Ok(self.record_decision_rejection(
                    decision_request_id,
                    decision_expected_revision,
                    DecisionAttemptErrorCode::InvalidDecision,
                    "decision action and nested command disagree".to_owned(),
                ));
            }
        }
        let trace_id = prepared.trace.as_ref().map(|trace| trace.id);
        let command_request_id = request.command.as_ref().map(|request| request.request_id);
        decisions.attempts.push(DecisionAttemptRecord {
            request_id: decision_request_id,
            at: self.state.scheduler.now,
            revision_before,
            expected_revision: decision_expected_revision,
            outcome: DecisionAttemptOutcome::Accepted {
                trace_id,
                command_request_id,
            },
        });
        if let Some((_, next_id)) = trace_claim {
            self.state.counters.next_decision_trace_id = next_id;
        }
        self.state.current.decisions = decisions;
        self.invalidate_commitments(super::CommitmentDomains::DECISIONS);
        let Some(command) = request.command else {
            return Ok(None);
        };
        let CommandRequest {
            request_id,
            expected_revision,
            envelope,
        } = *command;
        self.admit_command(
            Some(request_id),
            Some(expected_revision),
            envelope,
            CommandIngress::LiveRequest,
            decision_controller_id,
            true,
        )
        .map(Some)
    }

    fn record_decision_rejection(
        &mut self,
        request_id: DecisionRequestId,
        expected_revision: u64,
        code: DecisionAttemptErrorCode,
        message: String,
    ) -> Option<CommandOutcome> {
        self.state
            .current
            .decisions
            .attempts
            .push(DecisionAttemptRecord {
                request_id,
                at: self.state.scheduler.now,
                revision_before: self.revision(),
                expected_revision,
                outcome: DecisionAttemptOutcome::Rejected { code, message },
            });
        self.invalidate_commitments(super::CommitmentDomains::DECISIONS);
        None
    }

    pub(super) fn command_request_id_is_in_use(&self, request_id: CommandRequestId) -> bool {
        self.state
            .evidence
            .archived_command_requests
            .contains_key(&request_id)
            || self
                .state
                .evidence
                .archived_ingress_requests
                .contains_key(&request_id)
            || self
                .state
                .evidence
                .archived_decision_command_requests
                .contains(&request_id)
            || self
                .state
                .evidence
                .command_attempts
                .iter()
                .any(|attempt| attempt.request_id == Some(request_id))
            || self
                .state
                .evidence
                .ingress
                .iter()
                .any(|record| ingress_command_request_id(record) == Some(request_id))
    }

    fn command_request_id_is_unique_for_admitted_decision(
        &self,
        request_id: CommandRequestId,
    ) -> bool {
        !self
            .state
            .evidence
            .archived_command_requests
            .contains_key(&request_id)
            && !self
                .state
                .evidence
                .archived_ingress_requests
                .contains_key(&request_id)
            && !self
                .state
                .evidence
                .archived_decision_command_requests
                .contains(&request_id)
            && !self
                .state
                .evidence
                .command_attempts
                .iter()
                .any(|attempt| attempt.request_id == Some(request_id))
            && self
                .state
                .evidence
                .ingress
                .iter()
                .filter(|record| ingress_command_request_id(record) == Some(request_id))
                .count()
                == 1
    }

    fn validate_decision_mutation_entities(
        &self,
        mutation: &DecisionMutation,
    ) -> Result<(), CanwuError> {
        let entity_exists =
            |entity: &EntityRef| runtime_entity_identity_exists(&self.state, entity);
        let validate_authority = |authority: &DecisionAuthority| {
            let valid = match authority {
                DecisionAuthority::Actor { actor } => entity_exists(&EntityRef::Person(*actor)),
                DecisionAuthority::Institution {
                    institution,
                    responsible_actor,
                } => {
                    entity_exists(institution)
                        && responsible_actor
                            .is_none_or(|actor| entity_exists(&EntityRef::Person(actor)))
                }
                DecisionAuthority::Council { .. }
                | DecisionAuthority::NoResponsibleActor { .. } => true,
            };
            valid.then_some(()).ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::InvalidDecision,
                    "decision controller authority references an unknown entity",
                )
            })
        };
        match mutation {
            DecisionMutation::RegisterController { controller } => {
                validate_authority(&controller.authority)?;
                if controller
                    .command_subject
                    .as_ref()
                    .is_some_and(|entity| !entity_exists(entity))
                {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidDecision,
                        "decision controller command subject references an unknown entity",
                    ));
                }
            }
            DecisionMutation::Open { ticket } if !entity_exists(&ticket.decision_maker) => {
                return Err(CanwuError::new(
                    ErrorCode::InvalidDecision,
                    "decision maker references an unknown entity",
                ));
            }
            DecisionMutation::Open { .. }
            | DecisionMutation::ReplaceOptions { .. }
            | DecisionMutation::Resolve { .. }
            | DecisionMutation::Cancel { .. } => {}
        }
        Ok(())
    }
}

fn ingress_command_request_id(record: &super::IngressRecord) -> Option<CommandRequestId> {
    match &record.payload {
        IngressPayload::Command { request } => Some(request.request_id),
        IngressPayload::Decision { request } => {
            request.command.as_ref().map(|request| request.request_id)
        }
        IngressPayload::Plugin { .. } | IngressPayload::Calendar { .. } => None,
    }
}

pub(super) fn controller_issuer(controller: &super::DecisionControllerBinding) -> Issuer {
    match controller.policy.kind {
        DecisionPolicyKind::Human => Issuer::Human(controller.id.clone()),
        DecisionPolicyKind::Utility
        | DecisionPolicyKind::Rule
        | DecisionPolicyKind::External
        | DecisionPolicyKind::Llm => Issuer::Ai(controller.id.clone()),
    }
}

pub(super) fn controller_authority(
    controller: &super::DecisionControllerBinding,
) -> CommandAuthority {
    let decision_origin = match &controller.authority {
        DecisionAuthority::Actor { actor } => super::DecisionOrigin::Actor { actor: *actor },
        DecisionAuthority::Institution {
            institution,
            responsible_actor,
        } => super::DecisionOrigin::Institution {
            institution: institution.clone(),
            responsible_actor: *responsible_actor,
        },
        DecisionAuthority::Council { council_id } => super::DecisionOrigin::Council {
            council_id: council_id.clone(),
        },
        DecisionAuthority::NoResponsibleActor { reason } => {
            super::DecisionOrigin::NoResponsibleActor {
                reason: reason.clone(),
            }
        }
    };
    CommandAuthority {
        decision_origin,
        seat_id: controller.seat_id.clone(),
        permission_profile_id: controller.permission_profile_id.clone(),
        command_subject: controller.command_subject.clone(),
    }
}

#[allow(clippy::needless_pass_by_value)]
pub(super) fn decision_error(error: DecisionError) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidDecision, error.to_string())
}
