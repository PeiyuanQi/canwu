use super::{
    ArmyId, BoundaryReceipt, BoundaryRequest, CanwuError, CauseRef, CommandAttemptId, CommandId,
    CommandPolicyContext, CommandRequestId, CommandTransactionCheckpoint, Deserialize, EntityRef,
    ErrorCode, EventId, IngressId, IngressTransactionCheckpoint, InteractionPolicy, LetterId,
    PayloadSchema, PersonId, RejectionTransactionCheckpoint, Serialize, SimDuration, SimTime,
    Simulation, SystemCadence, TerritoryId, Value, canonical_hash, claim_counter,
    invalid_snapshot_error, is_expected_command_rejection, resolve_command_authority,
    runtime_entity_exists, runtime_entity_identity_exists, runtime_has_unqueued_command_history,
    validate_command_ingress_policy, validate_runtime_cause,
};
use std::cmp::Reverse;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "id", rename_all = "snake_case")]
pub enum Issuer {
    Actor(PersonId),
    Human(String),
    Ai(String),
    Institution(String),
    Replay(String),
    Experiment(String),
    Debug,
    System(String),
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandIngress {
    LegacyDirect,
    LiveRequest,
    FrozenReplay,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DecisionOrigin {
    Actor {
        actor: PersonId,
    },
    Institution {
        institution: EntityRef,
        responsible_actor: Option<PersonId>,
    },
    Council {
        council_id: String,
    },
    NoResponsibleActor {
        reason: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CommandAuthority {
    pub decision_origin: DecisionOrigin,
    pub seat_id: Option<String>,
    pub permission_profile_id: Option<String>,
    pub command_subject: Option<EntityRef>,
}

impl CommandAuthority {
    #[must_use]
    pub const fn for_actor(actor: PersonId) -> Self {
        Self {
            decision_origin: DecisionOrigin::Actor { actor },
            seat_id: None,
            permission_profile_id: None,
            command_subject: None,
        }
    }

    #[must_use]
    pub fn no_responsible_actor(reason: impl Into<String>) -> Self {
        Self {
            decision_origin: DecisionOrigin::NoResponsibleActor {
                reason: reason.into(),
            },
            seat_id: None,
            permission_profile_id: None,
            command_subject: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CommandContext {
    pub issuer: Issuer,
    pub authority: CommandAuthority,
    /// Present only when the engine admitted this command as the selected
    /// action of a validated `DecisionTicket` controller.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_controller_id: Option<String>,
    pub run_policy: CommandPolicyContext,
    pub ingress: CommandIngress,
    pub attempt_id: Option<CommandAttemptId>,
    pub command_id: CommandId,
    pub request_id: Option<CommandRequestId>,
    pub revision: u64,
    pub simulation_time: SimTime,
    pub expected_revision: Option<u64>,
    pub expected_time: Option<SimTime>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    OrderMovement {
        subject: EntityRef,
        destination: TerritoryId,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        cargo: Vec<LetterId>,
    },
    DebugSetArmyMorale {
        army: ArmyId,
        morale: u16,
    },
    Plugin {
        plugin: String,
        command: String,
        payload: Value,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CommandEnvelope {
    pub issuer: Issuer,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority: Option<CommandAuthority>,
    pub command: Command,
    pub expected_time: Option<SimTime>,
}

impl CommandEnvelope {
    #[must_use]
    pub const fn new(issuer: Issuer, command: Command) -> Self {
        Self {
            issuer,
            authority: None,
            command,
            expected_time: None,
        }
    }

    #[must_use]
    pub const fn at_time(mut self, expected_time: SimTime) -> Self {
        self.expected_time = Some(expected_time);
        self
    }

    #[must_use]
    pub fn with_authority(mut self, authority: CommandAuthority) -> Self {
        self.authority = Some(authority);
        self
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CommandRequest {
    pub request_id: CommandRequestId,
    /// Must equal the persisted authoritative revision at command admission.
    ///
    /// Accepted commands, persisted expected rejections, and completed
    /// settlement boundaries advance the revision. Bare clock movement does
    /// not, so declared external commands also carry `envelope.expected_time`.
    pub expected_revision: u64,
    pub envelope: CommandEnvelope,
}

impl CommandRequest {
    #[must_use]
    pub const fn new(
        request_id: CommandRequestId,
        expected_revision: u64,
        envelope: CommandEnvelope,
    ) -> Self {
        Self {
            request_id,
            expected_revision,
            envelope,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct CommandAdmission {
    pub(super) request_id: Option<CommandRequestId>,
    pub(super) expected_revision: Option<u64>,
    pub(super) expected_time: Option<SimTime>,
    pub(super) revision_before: u64,
    pub(super) ingress: CommandIngress,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CommandRecord {
    pub id: CommandId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_id: Option<CommandAttemptId>,
    pub accepted_at: SimTime,
    pub envelope: CommandEnvelope,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub emitted_events: Vec<EventId>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CommandReceipt {
    pub attempt_id: Option<CommandAttemptId>,
    pub command_id: CommandId,
    pub request_id: Option<CommandRequestId>,
    /// Authoritative revision after the accepted command commits.
    pub revision: u64,
    pub accepted_at: SimTime,
    pub emitted_events: Vec<EventId>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CommandRejection {
    pub attempt_id: Option<CommandAttemptId>,
    pub request_id: Option<CommandRequestId>,
    /// Authoritative revision after persisted rejection evidence commits.
    /// Non-persisted conflicts retain the already committed current revision.
    pub retained_revision: u64,
    pub rejected_at: SimTime,
    pub error: CanwuError,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum CommandOutcome {
    Accepted { receipt: CommandReceipt },
    Rejected { rejection: CommandRejection },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum CommandAttemptOutcome {
    Accepted { command_id: CommandId },
    Rejected { error: CanwuError },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CommandAttemptRecord {
    pub id: CommandAttemptId,
    pub at: SimTime,
    /// Authoritative revision immediately before this attempt transaction.
    pub revision_before: u64,
    pub ingress: CommandIngress,
    pub request_id: Option<CommandRequestId>,
    pub expected_revision: Option<u64>,
    pub envelope: CommandEnvelope,
    pub outcome: CommandAttemptOutcome,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IngressClass {
    Command,
    Communication,
    Acknowledgement,
    Information,
    Decision,
    ScheduledSystem,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PluginIngressDescriptor {
    pub name: String,
    pub description: String,
    pub class: IngressClass,
    pub payload_schema: PayloadSchema,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PluginIngressRequest {
    pub plugin: String,
    pub packet_type: String,
    pub due_at: SimTime,
    pub priority: i32,
    pub payload: Value,
    pub affected_entities: Vec<EntityRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<CauseRef>,
}

impl PluginIngressRequest {
    #[must_use]
    pub fn new(
        plugin: impl Into<String>,
        packet_type: impl Into<String>,
        due_at: SimTime,
        payload: Value,
    ) -> Self {
        Self {
            plugin: plugin.into(),
            packet_type: packet_type.into(),
            due_at,
            priority: 0,
            payload,
            affected_entities: Vec::new(),
            cause: None,
        }
    }

    #[must_use]
    pub const fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    #[must_use]
    pub fn with_entity(mut self, entity: EntityRef) -> Self {
        self.affected_entities.push(entity);
        self
    }

    #[must_use]
    pub fn caused_by(mut self, cause: CauseRef) -> Self {
        self.cause = Some(cause);
        self
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IngressPayload {
    Command {
        request: Box<CommandRequest>,
    },
    Plugin {
        plugin: String,
        packet_type: String,
        payload: Value,
        affected_entities: Vec<EntityRef>,
    },
    Calendar {
        cadences: Vec<SystemCadence>,
    },
    Decision {
        request: Box<super::DecisionIngressRequest>,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct IngressRecord {
    pub id: IngressId,
    pub issued_at: SimTime,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub eligible_boundary_count: u64,
    pub due_at: SimTime,
    pub class: IngressClass,
    pub priority: i32,
    pub payload: IngressPayload,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<CauseRef>,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_zero(value: &u64) -> bool {
    *value == 0
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct IngressReceipt {
    pub ingress_id: IngressId,
    pub issued_at: SimTime,
    pub due_at: SimTime,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct IngressQueueKey {
    pub due_at: SimTime,
    pub class: IngressClass,
    pub priority: Reverse<i32>,
    pub issued_at: SimTime,
    pub id: IngressId,
}

impl IngressQueueKey {
    #[must_use]
    pub(crate) const fn from_record(record: &IngressRecord) -> Self {
        Self {
            due_at: record.due_at,
            class: record.class,
            priority: Reverse(record.priority),
            issued_at: record.issued_at,
            id: record.id,
        }
    }
}

impl Simulation {
    pub fn submit(&mut self, envelope: CommandEnvelope) -> Result<CommandReceipt, CanwuError> {
        match self.admit_command(
            None,
            None,
            envelope,
            CommandIngress::LegacyDirect,
            None,
            false,
        )? {
            CommandOutcome::Accepted { receipt } => Ok(receipt),
            CommandOutcome::Rejected { rejection } => Err(rejection.error),
        }
    }

    pub fn enqueue_command(
        &mut self,
        due_at: SimTime,
        priority: i32,
        request: CommandRequest,
    ) -> Result<IngressReceipt, CanwuError> {
        self.ensure_runtime_ready()?;
        self.ensure_canonical_ingress_can_start()?;
        self.ensure_command_ingress_family(CommandIngress::LiveRequest)?;
        if let Some(existing) = self
            .state
            .evidence
            .archived_ingress_requests
            .get(&request.request_id)
        {
            let input_hash = canonical_hash(
                "canwu.archive.ingress.command.v1",
                &(due_at, priority, &request),
            )?;
            if existing.input_hash == input_hash {
                return Ok(existing.receipt.clone());
            }
            return Err(CanwuError::new(
                ErrorCode::IdempotencyConflict,
                format!(
                    "command request {} is already queued with different ingress content",
                    request.request_id
                ),
            ));
        }
        for record in &self.state.evidence.ingress {
            let IngressPayload::Command { request: existing } = &record.payload else {
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
                    "command request {} is already queued with different ingress content",
                    request.request_id
                ),
            ));
        }
        if self.command_request_id_is_in_use(request.request_id) {
            return Err(CanwuError::new(
                ErrorCode::IdempotencyConflict,
                format!(
                    "command request {} is already reserved or processed",
                    request.request_id
                ),
            ));
        }
        if request
            .envelope
            .expected_time
            .is_some_and(|expected| expected != due_at)
        {
            return Err(CanwuError::new(
                ErrorCode::SimulationTimeConflict,
                "queued command expected time must equal its due simulation time",
            ));
        }
        self.append_ingress(
            due_at,
            IngressClass::Command,
            priority,
            IngressPayload::Command {
                request: Box::new(request),
            },
            None,
            false,
        )
    }

    pub fn enqueue_plugin_ingress(
        &mut self,
        mut request: PluginIngressRequest,
    ) -> Result<IngressReceipt, CanwuError> {
        self.ensure_runtime_ready()?;
        self.ensure_canonical_ingress_can_start()?;
        if self
            .state
            .metadata
            .run_configuration
            .declared()
            .is_some_and(|configuration| configuration.interaction == InteractionPolicy::ReadOnly)
        {
            return Err(CanwuError::new(
                ErrorCode::InteractionReadOnly,
                "the run interaction policy rejects newly authored plugin ingress",
            ));
        }
        let key = (request.plugin.clone(), request.packet_type.clone());
        let descriptor = self.plugins.ingress.get(&key).ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidPayload,
                format!(
                    "plugin ingress type {}.{} is not registered",
                    request.plugin, request.packet_type
                ),
            )
        })?;
        descriptor.payload_schema.validate(&request.payload)?;
        request.affected_entities.sort();
        request.affected_entities.dedup();
        if request
            .affected_entities
            .iter()
            .any(|entity| !runtime_entity_identity_exists(&self.state, entity))
        {
            return Err(CanwuError::new(
                ErrorCode::EntityNotFound,
                "plugin ingress references an unknown entity identity",
            ));
        }
        if let Some(cause) = &request.cause {
            if matches!(
                cause,
                CauseRef::Boundary(_) | CauseRef::Command(_) | CauseRef::Event(_)
            ) {
                return Err(CanwuError::new(
                    ErrorCode::InvalidPayload,
                    "boundary, command, and event causes are reserved for plugin-generated ingress",
                ));
            }
            validate_runtime_cause(&self.state, cause)?;
        }
        self.append_ingress(
            request.due_at,
            descriptor.class,
            request.priority,
            IngressPayload::Plugin {
                plugin: request.plugin,
                packet_type: request.packet_type,
                payload: request.payload,
                affected_entities: request.affected_entities,
            },
            request.cause,
            false,
        )
    }

    pub fn schedule_calendar_boundary(
        &mut self,
        due_at: SimTime,
        mut cadences: Vec<SystemCadence>,
    ) -> Result<IngressReceipt, CanwuError> {
        self.ensure_runtime_ready()?;
        self.ensure_canonical_ingress_can_start()?;
        if cadences.contains(&SystemCadence::EventDriven) {
            return Err(CanwuError::new(
                ErrorCode::InvalidBoundary,
                "calendar ingress cannot declare event-driven cadence",
            ));
        }
        cadences.sort();
        cadences.dedup();
        if cadences.is_empty() {
            return Err(CanwuError::new(
                ErrorCode::InvalidBoundary,
                "calendar ingress requires at least one scheduled cadence",
            ));
        }
        self.append_ingress(
            due_at,
            IngressClass::ScheduledSystem,
            0,
            IngressPayload::Calendar { cadences },
            Some(CauseRef::System("canwu.core.calendar".to_owned())),
            false,
        )
    }

    pub(super) fn append_ingress(
        &mut self,
        due_at: SimTime,
        class: IngressClass,
        priority: i32,
        payload: IngressPayload,
        cause: Option<CauseRef>,
        after_current_boundary: bool,
    ) -> Result<IngressReceipt, CanwuError> {
        if due_at < self.state.scheduler.now {
            return Err(CanwuError::new(
                ErrorCode::LateIngress,
                format!(
                    "ingress due at {due_at} cannot be queued after committed time {}",
                    self.state.scheduler.now
                ),
            ));
        }
        let transaction = IngressTransactionCheckpoint::capture(&self.state);
        let (id, next_id) = claim_counter(self.state.counters.next_ingress_id, "ingress ID")?;
        let boundary_count = self
            .state
            .evidence
            .archived
            .boundary_count
            .checked_add(
                u64::try_from(self.state.evidence.boundaries.len()).map_err(|_| {
                    CanwuError::new(
                        ErrorCode::IdentifierExhausted,
                        "boundary count exceeds the ingress journal range",
                    )
                })?,
            )
            .ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::IdentifierExhausted,
                    "boundary count exceeds the ingress journal range",
                )
            })?;
        let eligible_boundary_count = if after_current_boundary {
            boundary_count.checked_add(1).ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::IdentifierExhausted,
                    "ingress boundary eligibility exceeds the journal range",
                )
            })?
        } else {
            boundary_count
        };
        let record = IngressRecord {
            id: IngressId::new(id),
            issued_at: self.state.scheduler.now,
            eligible_boundary_count,
            due_at,
            class,
            priority,
            payload,
            cause,
        };
        let queue_key = IngressQueueKey::from_record(&record);
        self.state.counters.next_ingress_id = next_id;
        self.state.scheduler.pending_ingress.insert(queue_key);
        self.state.evidence.ingress.push(record.clone());
        self.state.metadata.plugin_registration_closed = true;
        if let Err(error) = self.refresh_checkpoint_hash() {
            transaction.restore(&mut self.state, &queue_key);
            return Err(error);
        }
        Ok(IngressReceipt {
            ingress_id: record.id,
            issued_at: record.issued_at,
            due_at: record.due_at,
        })
    }

    pub fn process_command(
        &mut self,
        request: CommandRequest,
    ) -> Result<CommandOutcome, CanwuError> {
        self.ensure_runtime_ready()?;
        if self.state.evidence.archived.ingress_count != 0
            || !self.state.evidence.ingress.is_empty()
        {
            return Err(CanwuError::new(
                ErrorCode::MixedCommandIngress,
                "direct command requests cannot bypass an active canonical ingress journal",
            ));
        }
        self.admit_command(
            Some(request.request_id),
            Some(request.expected_revision),
            request.envelope,
            CommandIngress::LiveRequest,
            None,
            true,
        )
    }

    pub(super) fn admit_command(
        &mut self,
        request_id: Option<CommandRequestId>,
        expected_revision: Option<u64>,
        envelope: CommandEnvelope,
        ingress: CommandIngress,
        decision_controller_id: Option<String>,
        record_attempt: bool,
    ) -> Result<CommandOutcome, CanwuError> {
        self.ensure_runtime_ready()?;
        self.ensure_command_ingress_family(ingress)?;
        if let Some(cached) =
            self.cached_command_outcome(request_id, expected_revision, &envelope)?
        {
            return Ok(cached);
        }

        let revision_before = self.revision();
        let admission = CommandAdmission {
            request_id,
            expected_revision,
            expected_time: envelope.expected_time,
            revision_before,
            ingress,
        };
        let attempt_id = if record_attempt {
            let (value, _) = claim_counter(
                self.state.counters.next_command_attempt_id,
                "command attempt ID",
            )?;
            CommandAttemptId::new(value)
        } else {
            CommandAttemptId::default()
        };
        let authority = match resolve_command_authority(&envelope) {
            Ok(authority) => authority,
            Err(error) if is_expected_command_rejection(&error.code) && record_attempt => {
                return self.record_command_rejection(attempt_id, admission, envelope, error);
            }
            Err(error) => return Err(error),
        };
        if let Err(error) = self.validate_command_ingress(&envelope.issuer, &authority, admission) {
            if is_expected_command_rejection(&error.code) && record_attempt {
                return self.record_command_rejection(attempt_id, admission, envelope, error);
            }
            return Err(error);
        }
        if let Some(expected_time) = envelope.expected_time
            && expected_time != self.state.scheduler.now
        {
            let error = CanwuError::new(
                ErrorCode::SimulationTimeConflict,
                format!(
                    "command expected time {expected_time}, but simulation is at {}",
                    self.state.scheduler.now
                ),
            );
            if record_attempt {
                return self.record_command_rejection(attempt_id, admission, envelope, error);
            }
            return Err(error);
        }

        let (command_id_value, next_command_id) =
            claim_counter(self.state.counters.next_command_id, "command ID")?;
        let (correlation_id, next_correlation_id) =
            claim_counter(self.state.counters.next_correlation_id, "correlation ID")?;
        let command_id = CommandId::new(command_id_value);
        let context = CommandContext {
            issuer: envelope.issuer.clone(),
            authority,
            decision_controller_id,
            run_policy: self.state.metadata.run_configuration.command_policy(),
            ingress: admission.ingress,
            attempt_id: record_attempt.then_some(attempt_id),
            command_id,
            request_id: admission.request_id,
            revision: admission.revision_before,
            simulation_time: self.state.scheduler.now,
            expected_revision: admission.expected_revision,
            expected_time: envelope.expected_time,
        };
        let prepared = match self.prepare_command(&envelope, &context) {
            Ok(prepared) => prepared,
            Err(error) if is_expected_command_rejection(&error.code) && record_attempt => {
                return self.record_command_rejection(attempt_id, admission, envelope, error);
            }
            Err(error) => return Err(error),
        };
        let next_attempt_id = if record_attempt {
            let (claimed_id, next_attempt_id) = claim_counter(
                self.state.counters.next_command_attempt_id,
                "command attempt ID",
            )?;
            if claimed_id != attempt_id.get() {
                return Err(CanwuError::new(
                    ErrorCode::InvalidSnapshot,
                    "command attempt allocation changed during application",
                ));
            }
            Some(next_attempt_id)
        } else {
            None
        };
        let revision = self.next_state_revision()?;
        let transaction = CommandTransactionCheckpoint::capture(&self.state);
        let event_start = self.state.evidence.events.len();
        self.state.counters.next_command_id = next_command_id;
        self.state.counters.next_correlation_id = next_correlation_id;
        self.invalidate_commitments(prepared.commitment_invalidation());

        if let Err(error) = self.apply_prepared(prepared, command_id, correlation_id) {
            transaction.restore(&mut self.state);
            if is_expected_command_rejection(&error.code) && record_attempt {
                return self.record_command_rejection(attempt_id, admission, envelope, error);
            }
            return Err(error);
        }
        let emitted_events: Vec<_> = self.state.evidence.events[event_start..]
            .iter()
            .map(|event| event.id)
            .collect();
        self.state.metadata.plugin_registration_closed = true;
        self.state.evidence.commands.push(CommandRecord {
            id: command_id,
            attempt_id: record_attempt.then_some(attempt_id),
            accepted_at: self.state.scheduler.now,
            envelope: envelope.clone(),
            emitted_events: if record_attempt {
                emitted_events.clone()
            } else {
                Vec::new()
            },
        });
        if let Some(next_attempt_id) = next_attempt_id {
            self.state.counters.next_command_attempt_id = next_attempt_id;
            self.state
                .evidence
                .command_attempts
                .push(CommandAttemptRecord {
                    id: attempt_id,
                    at: self.state.scheduler.now,
                    revision_before: admission.revision_before,
                    ingress: admission.ingress,
                    request_id: admission.request_id,
                    expected_revision: admission.expected_revision,
                    envelope,
                    outcome: CommandAttemptOutcome::Accepted { command_id },
                });
        }
        self.state.counters.state_revision = revision;
        if let Err(error) = self.refresh_checkpoint_hash() {
            transaction.restore(&mut self.state);
            return Err(error);
        }

        Ok(CommandOutcome::Accepted {
            receipt: CommandReceipt {
                attempt_id: record_attempt.then_some(attempt_id),
                command_id,
                request_id: admission.request_id,
                revision,
                accepted_at: self.state.scheduler.now,
                emitted_events,
            },
        })
    }

    fn ensure_command_ingress_family(&self, ingress: CommandIngress) -> Result<(), CanwuError> {
        let has_legacy_commands = self.state.evidence.archived_legacy_commands
            || self
                .state
                .evidence
                .commands
                .iter()
                .any(|record| record.attempt_id.is_none());
        let has_tracked_attempts = self.state.evidence.archived_tracked_attempts
            || !self.state.evidence.command_attempts.is_empty()
            || !self.state.evidence.ingress.is_empty();
        if (ingress == CommandIngress::LegacyDirect && has_tracked_attempts)
            || (ingress != CommandIngress::LegacyDirect && has_legacy_commands)
        {
            return Err(CanwuError::new(
                ErrorCode::MixedCommandIngress,
                "legacy-direct commands and tracked request/replay attempts cannot coexist in one run",
            ));
        }
        Ok(())
    }

    pub(super) fn ensure_canonical_ingress_can_start(&self) -> Result<(), CanwuError> {
        if runtime_has_unqueued_command_history(&self.state) {
            return Err(CanwuError::new(
                ErrorCode::MixedCommandIngress,
                "canonical ingress cannot be added after direct command history",
            ));
        }
        Ok(())
    }

    fn cached_command_outcome(
        &self,
        request_id: Option<CommandRequestId>,
        expected_revision: Option<u64>,
        envelope: &CommandEnvelope,
    ) -> Result<Option<CommandOutcome>, CanwuError> {
        let Some(request_id) = request_id else {
            return Ok(None);
        };
        if let Some(cached) = self
            .state
            .evidence
            .archived_command_requests
            .get(&request_id)
        {
            let input_hash = canonical_hash(
                "canwu.archive.command.request.v1",
                &(expected_revision, envelope),
            )?;
            if cached.input_hash != input_hash {
                return Ok(Some(CommandOutcome::Rejected {
                    rejection: CommandRejection {
                        attempt_id: None,
                        request_id: Some(request_id),
                        retained_revision: self.revision(),
                        rejected_at: self.state.scheduler.now,
                        error: CanwuError::new(
                            ErrorCode::IdempotencyConflict,
                            "this command request ID was already used for different input",
                        ),
                    },
                }));
            }
            return Ok(Some(cached.outcome.clone()));
        }
        let Some(attempt) = self
            .state
            .evidence
            .command_attempts
            .iter()
            .find(|attempt| attempt.request_id == Some(request_id))
        else {
            return Ok(None);
        };
        if attempt.expected_revision != expected_revision || &attempt.envelope != envelope {
            return Ok(Some(CommandOutcome::Rejected {
                rejection: CommandRejection {
                    attempt_id: None,
                    request_id: Some(request_id),
                    retained_revision: self.revision(),
                    rejected_at: self.state.scheduler.now,
                    error: CanwuError::new(
                        ErrorCode::IdempotencyConflict,
                        "this command request ID was already used for different input",
                    ),
                },
            }));
        }
        Ok(Some(self.command_outcome_from_attempt(attempt)?))
    }

    pub(super) fn command_outcome_from_attempt(
        &self,
        attempt: &CommandAttemptRecord,
    ) -> Result<CommandOutcome, CanwuError> {
        let request_id = attempt.request_id.ok_or_else(|| {
            invalid_snapshot_error("tracked command attempt is missing its request ID")
        })?;
        let committed_revision = attempt.revision_before.checked_add(1).ok_or_else(|| {
            invalid_snapshot_error("cached command attempt revision is exhausted")
        })?;
        match &attempt.outcome {
            CommandAttemptOutcome::Accepted { command_id } => {
                let retained_number = command_id
                    .get()
                    .checked_sub(self.state.evidence.archived.command_count)
                    .and_then(|value| value.checked_sub(1))
                    .ok_or_else(|| {
                        invalid_snapshot_error(
                            "accepted command attempt references archived command evidence",
                        )
                    })?;
                let index = usize::try_from(retained_number).map_err(|_| {
                    invalid_snapshot_error(
                        "accepted command attempt exceeds the retained command index space",
                    )
                })?;
                let record = self
                    .state
                    .evidence
                    .commands
                    .get(index)
                    .filter(|record| record.id == *command_id)
                    .ok_or_else(|| {
                        invalid_snapshot_error(
                            "accepted command attempt references a missing command",
                        )
                    })?;
                Ok(CommandOutcome::Accepted {
                    receipt: CommandReceipt {
                        attempt_id: Some(attempt.id),
                        command_id: *command_id,
                        request_id: Some(request_id),
                        revision: committed_revision,
                        accepted_at: record.accepted_at,
                        emitted_events: record.emitted_events.clone(),
                    },
                })
            }
            CommandAttemptOutcome::Rejected { error } => Ok(CommandOutcome::Rejected {
                rejection: CommandRejection {
                    attempt_id: Some(attempt.id),
                    request_id: Some(request_id),
                    retained_revision: committed_revision,
                    rejected_at: attempt.at,
                    error: error.clone(),
                },
            }),
        }
    }

    fn record_command_rejection(
        &mut self,
        attempt_id: CommandAttemptId,
        admission: CommandAdmission,
        envelope: CommandEnvelope,
        error: CanwuError,
    ) -> Result<CommandOutcome, CanwuError> {
        let (claimed_id, next_attempt_id) = claim_counter(
            self.state.counters.next_command_attempt_id,
            "command attempt ID",
        )?;
        if claimed_id != attempt_id.get() {
            return Err(CanwuError::new(
                ErrorCode::InvalidSnapshot,
                "command attempt allocation changed during rejection",
            ));
        }
        let revision = self.next_state_revision()?;
        let attempt = CommandAttemptRecord {
            id: attempt_id,
            at: self.state.scheduler.now,
            revision_before: admission.revision_before,
            ingress: admission.ingress,
            request_id: admission.request_id,
            expected_revision: admission.expected_revision,
            envelope,
            outcome: CommandAttemptOutcome::Rejected {
                error: error.clone(),
            },
        };
        let transaction = RejectionTransactionCheckpoint::capture(&self.state);
        self.state.counters.next_command_attempt_id = next_attempt_id;
        self.state.counters.state_revision = revision;
        self.state.metadata.plugin_registration_closed = true;
        self.state.evidence.command_attempts.push(attempt);
        if let Err(hash_error) = self.refresh_checkpoint_hash() {
            transaction.restore(&mut self.state);
            return Err(hash_error);
        }
        Ok(CommandOutcome::Rejected {
            rejection: CommandRejection {
                attempt_id: Some(attempt_id),
                request_id: admission.request_id,
                retained_revision: revision,
                rejected_at: self.state.scheduler.now,
                error,
            },
        })
    }

    fn validate_command_ingress(
        &self,
        issuer: &Issuer,
        authority: &CommandAuthority,
        admission: CommandAdmission,
    ) -> Result<(), CanwuError> {
        validate_command_ingress_policy(
            &self.state.metadata.run_configuration,
            issuer,
            authority,
            admission,
            &|entity| runtime_entity_exists(&self.state, entity),
        )
    }

    pub fn advance_canonical(
        &mut self,
        duration: SimDuration,
    ) -> Result<Vec<BoundaryReceipt>, CanwuError> {
        self.ensure_runtime_ready()?;
        if duration.is_negative() {
            return Err(CanwuError::new(
                ErrorCode::InvalidDuration,
                "canonical simulation time cannot advance by a negative duration",
            ));
        }
        let target = self
            .state
            .scheduler
            .now
            .checked_add(duration)
            .ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::InvalidDuration,
                    "canonical simulation target time exceeds the supported range",
                )
            })?;
        let mut receipts = Vec::new();
        while let Some(next_due) = self.next_canonical_due_time()
            && next_due <= target
        {
            let at = next_due.max(self.state.scheduler.now);
            receipts.push(self.settle_boundary(BoundaryRequest::at(at))?);
        }
        if self.state.scheduler.now < target {
            self.advance_to(target)?;
        }
        Ok(receipts)
    }

    pub fn step_canonical(&mut self) -> Result<Option<BoundaryReceipt>, CanwuError> {
        self.ensure_runtime_ready()?;
        let Some(next_due) = self.next_canonical_due_time() else {
            return Ok(None);
        };
        self.settle_boundary(BoundaryRequest::at(next_due.max(self.state.scheduler.now)))
            .map(Some)
    }

    fn next_canonical_due_time(&self) -> Option<SimTime> {
        let scheduled = self.state.scheduler.actions.keys().next().map(|key| key.at);
        let ingress = self
            .state
            .scheduler
            .pending_ingress
            .first()
            .map(|key| key.due_at);
        match (scheduled, ingress) {
            (Some(left), Some(right)) => Some(left.min(right)),
            (Some(value), None) | (None, Some(value)) => Some(value),
            (None, None) => None,
        }
    }

    pub(super) fn take_due_ingress(&mut self, at: SimTime) -> Vec<IngressId> {
        let mut admitted = Vec::new();
        while self
            .state
            .scheduler
            .pending_ingress
            .first()
            .is_some_and(|key| key.due_at <= at)
        {
            let key = self
                .state
                .scheduler
                .pending_ingress
                .pop_first()
                .expect("pending ingress was checked as non-empty");
            admitted.push(key.id);
        }
        admitted
    }
}
