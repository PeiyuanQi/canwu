use crate::model::{canonicalize_options, require_text};
use crate::{
    DecisionAction, DecisionAttemptOutcome, DecisionAttemptRecord, DecisionControllerBinding,
    DecisionError, DecisionErrorCode, DecisionMutation, DecisionOutcome, DecisionPolicy,
    DecisionPolicyIdentity, DecisionTicket, DecisionTicketState, DecisionTrace, PolicyDecision,
};
use canwu_core::{CommandRequestId, DecisionTicketId, DecisionTraceId};
use canwu_time::SimTime;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct DecisionState {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub controllers: BTreeMap<String, DecisionControllerBinding>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tickets: BTreeMap<DecisionTicketId, DecisionTicket>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub traces: Vec<DecisionTrace>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attempts: Vec<DecisionAttemptRecord>,
}

impl DecisionState {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.controllers.is_empty()
            && self.tickets.is_empty()
            && self.traces.is_empty()
            && self.attempts.is_empty()
    }

    #[must_use]
    pub fn controller(&self, id: &str) -> Option<&DecisionControllerBinding> {
        self.controllers.get(id)
    }

    #[must_use]
    pub fn ticket(&self, id: DecisionTicketId) -> Option<&DecisionTicket> {
        self.tickets.get(&id)
    }

    pub fn open_tickets(&self) -> impl Iterator<Item = &DecisionTicket> {
        self.tickets.values().filter(|ticket| ticket.is_open())
    }

    pub fn validate(&self) -> Result<(), DecisionError> {
        for (id, controller) in &self.controllers {
            if id != &controller.id {
                return Err(DecisionError::new(
                    DecisionErrorCode::InvalidController,
                    "controller map key does not match its persisted identity",
                ));
            }
            controller.validate()?;
        }
        for (id, ticket) in &self.tickets {
            if id != &ticket.id || !self.controllers.contains_key(&ticket.assigned_controller) {
                return Err(DecisionError::new(
                    DecisionErrorCode::InvalidDecision,
                    "ticket identity or assigned controller is invalid",
                ));
            }
            ticket.validate()?;
        }
        let mut expected_trace_id = 1_u64;
        for trace in &self.traces {
            if trace.id.get() != expected_trace_id {
                return Err(DecisionError::new(
                    DecisionErrorCode::InvalidDecision,
                    "decision traces must use contiguous IDs in journal order",
                ));
            }
            expected_trace_id = expected_trace_id.checked_add(1).ok_or_else(|| {
                DecisionError::new(
                    DecisionErrorCode::InvalidDecision,
                    "decision trace ID range is exhausted",
                )
            })?;
            let ticket = self.tickets.get(&trace.ticket_id).ok_or_else(|| {
                DecisionError::new(
                    DecisionErrorCode::InvalidDecision,
                    "decision trace references an unknown ticket",
                )
            })?;
            if trace.ticket_version == 0
                || trace.ticket_version > ticket.version
                || !self.controllers.contains_key(&trace.controller_id)
            {
                return Err(DecisionError::new(
                    DecisionErrorCode::InvalidDecision,
                    "decision trace version or controller is invalid",
                ));
            }
        }
        for ticket in self.tickets.values() {
            if let DecisionTicketState::Resolved { trace_id, .. } = ticket.state {
                let trace_index =
                    usize::try_from(trace_id.get().saturating_sub(1)).map_err(|_| {
                        DecisionError::new(
                            DecisionErrorCode::InvalidDecision,
                            "decision trace ID exceeds platform range",
                        )
                    })?;
                if self
                    .traces
                    .get(trace_index)
                    .is_none_or(|trace| trace.id != trace_id || trace.ticket_id != ticket.id)
                {
                    return Err(DecisionError::new(
                        DecisionErrorCode::InvalidDecision,
                        "resolved ticket does not reference its persisted trace",
                    ));
                }
            }
        }
        let mut request_ids = std::collections::BTreeSet::new();
        for attempt in &self.attempts {
            if attempt.request_id.get() == 0 || !request_ids.insert(attempt.request_id) {
                return Err(DecisionError::new(
                    DecisionErrorCode::InvalidDecision,
                    "decision attempts must use unique nonzero request IDs",
                ));
            }
            match &attempt.outcome {
                DecisionAttemptOutcome::Accepted {
                    trace_id,
                    command_request_id,
                } => {
                    if command_request_id.is_some() && trace_id.is_none() {
                        return Err(DecisionError::new(
                            DecisionErrorCode::InvalidDecision,
                            "accepted decision commands require a decision trace",
                        ));
                    }
                }
                DecisionAttemptOutcome::Rejected { message, .. } => {
                    require_text(message, "decision rejection message")?;
                }
            }
        }
        Ok(())
    }

    pub fn apply(
        &mut self,
        mutation: DecisionMutation,
        at: SimTime,
        trace_id: Option<DecisionTraceId>,
    ) -> Result<PreparedDecision, DecisionError> {
        let prepared = match mutation {
            DecisionMutation::RegisterController { controller } => {
                controller.validate()?;
                if self.controllers.contains_key(&controller.id) {
                    return Err(DecisionError::new(
                        DecisionErrorCode::DuplicateController,
                        format!(
                            "decision controller {} is already registered",
                            controller.id
                        ),
                    ));
                }
                self.controllers.insert(controller.id.clone(), controller);
                PreparedDecision::default()
            }
            DecisionMutation::Open { mut ticket } => {
                ticket.validate()?;
                if self.tickets.contains_key(&ticket.id) {
                    return Err(DecisionError::new(
                        DecisionErrorCode::DuplicateTicket,
                        format!("decision ticket {} is already present", ticket.id),
                    ));
                }
                if !self.controllers.contains_key(&ticket.assigned_controller) {
                    return Err(DecisionError::new(
                        DecisionErrorCode::InvalidController,
                        format!(
                            "decision ticket {} names unknown controller {}",
                            ticket.id, ticket.assigned_controller
                        ),
                    ));
                }
                if ticket.deadline.is_some_and(|deadline| deadline < at) {
                    return Err(DecisionError::new(
                        DecisionErrorCode::InvalidDecision,
                        "decision deadline precedes its admission time",
                    ));
                }
                let persisted = DecisionTicket {
                    id: ticket.id,
                    definition: ticket.definition,
                    decision_maker: ticket.decision_maker,
                    assigned_controller: ticket.assigned_controller,
                    summary: ticket.summary,
                    context: ticket.context,
                    options: std::mem::take(&mut ticket.options),
                    opened_at: at,
                    updated_at: at,
                    deadline: ticket.deadline,
                    version: 1,
                    state: DecisionTicketState::Open,
                };
                self.tickets.insert(persisted.id, persisted);
                PreparedDecision::default()
            }
            DecisionMutation::ReplaceOptions {
                ticket_id,
                expected_version,
                context,
                mut options,
            } => {
                context.validate()?;
                canonicalize_options(&mut options)?;
                let ticket = self.open_ticket_mut(ticket_id, expected_version, at)?;
                ticket.context = context;
                ticket.options = options;
                ticket.updated_at = at;
                ticket.version = ticket.version.checked_add(1).ok_or_else(|| {
                    DecisionError::new(
                        DecisionErrorCode::InvalidDecision,
                        "decision ticket version is exhausted",
                    )
                })?;
                PreparedDecision::default()
            }
            DecisionMutation::Resolve {
                ticket_id,
                expected_version,
                controller_id,
                policy,
                decision,
                command_request_id,
            } => self.resolve(
                ticket_id,
                expected_version,
                &controller_id,
                policy,
                decision,
                command_request_id,
                at,
                trace_id.ok_or_else(|| {
                    DecisionError::new(
                        DecisionErrorCode::InvalidDecision,
                        "decision resolution requires a claimed trace ID",
                    )
                })?,
            )?,
            DecisionMutation::Cancel {
                ticket_id,
                expected_version,
                reason,
            } => {
                require_text(&reason, "decision cancellation reason")?;
                let ticket = self.open_ticket_mut(ticket_id, expected_version, at)?;
                ticket.updated_at = at;
                ticket.version = ticket.version.checked_add(1).ok_or_else(|| {
                    DecisionError::new(
                        DecisionErrorCode::InvalidDecision,
                        "decision ticket version is exhausted",
                    )
                })?;
                ticket.state = DecisionTicketState::Cancelled { reason };
                PreparedDecision::default()
            }
        };
        self.advance_time(at)?;
        self.validate()?;
        Ok(prepared)
    }

    fn open_ticket_mut(
        &mut self,
        ticket_id: DecisionTicketId,
        expected_version: u64,
        at: SimTime,
    ) -> Result<&mut DecisionTicket, DecisionError> {
        let ticket = self.tickets.get_mut(&ticket_id).ok_or_else(|| {
            DecisionError::new(
                DecisionErrorCode::TicketNotFound,
                format!("decision ticket {ticket_id} was not found"),
            )
        })?;
        if !ticket.is_open() || ticket.deadline.is_some_and(|deadline| deadline < at) {
            return Err(DecisionError::new(
                DecisionErrorCode::ClosedTicket,
                format!("decision ticket {ticket_id} is not open"),
            ));
        }
        if ticket.version != expected_version {
            return Err(DecisionError::new(
                DecisionErrorCode::VersionConflict,
                format!(
                    "decision ticket {ticket_id} is at version {}, expected {expected_version}",
                    ticket.version
                ),
            ));
        }
        Ok(ticket)
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve(
        &mut self,
        ticket_id: DecisionTicketId,
        expected_version: u64,
        controller_id: &str,
        policy: DecisionPolicyIdentity,
        decision: PolicyDecision,
        command_request_id: Option<CommandRequestId>,
        at: SimTime,
        trace_id: DecisionTraceId,
    ) -> Result<PreparedDecision, DecisionError> {
        let controller = self.controllers.get(controller_id).ok_or_else(|| {
            DecisionError::new(
                DecisionErrorCode::InvalidController,
                format!("decision controller {controller_id} was not found"),
            )
        })?;
        if controller.policy != policy {
            return Err(DecisionError::new(
                DecisionErrorCode::PolicyMismatch,
                "decision resolution policy does not match the persisted controller binding",
            ));
        }
        let ticket = self.open_ticket_mut(ticket_id, expected_version, at)?;
        if ticket.assigned_controller != controller_id {
            return Err(DecisionError::new(
                DecisionErrorCode::InvalidController,
                "decision resolution came from a controller not assigned to the ticket",
            ));
        }
        decision.validate(ticket)?;
        let action = match &decision.outcome {
            DecisionOutcome::Selected { option_id } => {
                ticket.option(option_id).map(|option| option.action.clone())
            }
            DecisionOutcome::Deferred { .. } => None,
            DecisionOutcome::Pending { .. } => {
                return Err(DecisionError::new(
                    DecisionErrorCode::InvalidDecision,
                    "pending policy outcomes are not authoritative decision mutations",
                ));
            }
        };
        if matches!(action, Some(DecisionAction::Command { .. })) != command_request_id.is_some() {
            return Err(DecisionError::new(
                DecisionErrorCode::InvalidDecision,
                "command actions require exactly one command request ID",
            ));
        }
        let trace = DecisionTrace {
            id: trace_id,
            ticket_id,
            ticket_version: ticket.version,
            controller_id: controller_id.to_owned(),
            policy,
            decided_at: at,
            outcome: decision.outcome.clone(),
            summary: decision.summary,
            evaluations: decision.evaluations,
            external: decision.external,
            command_request_id,
        };
        ticket.updated_at = at;
        ticket.version = ticket.version.checked_add(1).ok_or_else(|| {
            DecisionError::new(
                DecisionErrorCode::InvalidDecision,
                "decision ticket version is exhausted",
            )
        })?;
        if let DecisionOutcome::Selected { option_id } = &trace.outcome {
            ticket.state = DecisionTicketState::Resolved {
                option_id: option_id.clone(),
                trace_id,
            };
        }
        self.traces.push(trace.clone());
        Ok(PreparedDecision {
            trace: Some(trace),
            action,
        })
    }

    pub fn advance_time(&mut self, at: SimTime) -> Result<(), DecisionError> {
        for ticket in self.tickets.values_mut() {
            if ticket.is_open() && ticket.deadline.is_some_and(|deadline| deadline < at) {
                ticket.updated_at = at;
                ticket.version = ticket.version.checked_add(1).ok_or_else(|| {
                    DecisionError::new(
                        DecisionErrorCode::InvalidDecision,
                        "decision ticket version is exhausted",
                    )
                })?;
                ticket.state = DecisionTicketState::Expired;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PreparedDecision {
    pub trace: Option<DecisionTrace>,
    pub action: Option<DecisionAction>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ControllerDecision {
    Authoritative {
        decision: PolicyDecision,
        action: Option<DecisionAction>,
    },
    Pending(PolicyDecision),
}

pub struct DecisionController;

impl DecisionController {
    pub fn evaluate(
        ticket: &DecisionTicket,
        controller: &DecisionControllerBinding,
        policy: &dyn DecisionPolicy,
    ) -> Result<ControllerDecision, DecisionError> {
        if !ticket.is_open() {
            return Err(DecisionError::new(
                DecisionErrorCode::ClosedTicket,
                "only open tickets can be evaluated",
            ));
        }
        if ticket.assigned_controller != controller.id || policy.identity() != controller.policy {
            return Err(DecisionError::new(
                DecisionErrorCode::PolicyMismatch,
                "runtime policy identity does not match the ticket controller binding",
            ));
        }
        let decision = policy.decide(ticket)?;
        decision.validate(ticket)?;
        if matches!(decision.outcome, DecisionOutcome::Pending { .. }) {
            return Ok(ControllerDecision::Pending(decision));
        }
        let action = match &decision.outcome {
            DecisionOutcome::Selected { option_id } => {
                ticket.option(option_id).map(|option| option.action.clone())
            }
            DecisionOutcome::Deferred { .. } | DecisionOutcome::Pending { .. } => None,
        };
        Ok(ControllerDecision::Authoritative { action, decision })
    }
}
