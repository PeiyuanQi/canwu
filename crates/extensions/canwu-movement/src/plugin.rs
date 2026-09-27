use crate::PLUGIN_NAME;
use crate::lifecycle::{EvidenceReader, OperationActor, holder_entity};
use crate::model::{
    MovementCommandV1, MovementError, MovementErrorCode, MovementIncidentV1, MovementLegDueV1,
    MovementLimitsV1, MovementObservationHeadV1, MovementOperation, MovementOperationDisposition,
    MovementOperationOutcomeV1, MovementOperationScopeV1, MovementOperationSourceV1,
    MovementRuntimeRecord, MovementState, conflict_outcome_key, is_label,
    movement_runtime_reference,
};
use crate::report::{
    MOVEMENT_REPORT_KNOWLEDGE, MovementReportV1, event_times, movement_report_knowledge_schema_id,
    observer_cut, project,
};
use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundarySystemContract,
    Canwu, CanwuError, CauseRef, Command, CommandContext, CommandIngress, DomainRecord,
    DomainRecordKind, DomainRecordMutation, DomainRecordRef, DomainRecordSchema,
    DomainRecordVersionRef, ErrorCode, EvidenceRef, IngressClass, IngressPayload, Issuer,
    KnowledgeHolderRef, KnowledgeLimitsV1, KnowledgeOrigin, KnowledgeRecordDraft, KnowledgeSubject,
    KnowledgeSubjectSchema, KnowledgeSubjectTarget, KnowledgeSubjectTargetKind,
    KnowledgeWriteGrant, LifeState, PayloadSchema, PluginActionDescriptor, PluginIngressDescriptor,
    PluginIngressRequest, PluginIngressTarget, PluginKnowledgeSchema, PluginRegistrar, SimDuration,
    SimTime, SimulationPlugin, SimulationView, StateKey, StateVisibility, SystemCadence,
    SystemDirective, TransportExecutionId, canonical_hash,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Tracked command carrying one [`MovementCommandV1`].
pub const MOVEMENT_COMMAND: &str = "apply_movement_operation_v1";
/// Command-caused packet settled by the lifecycle writer.
pub const MOVEMENT_OPERATION_INGRESS: &str = "movement_operation_v1";
/// Application-reported leg failure, reroute, or handoff ([`MovementIncidentV1`]).
pub const MOVEMENT_INCIDENT_INGRESS: &str = "movement_incident_v1";
/// Plugin-scheduled departure or arrival of the current leg at its due time.
pub const MOVEMENT_LEG_DUE_INGRESS: &str = "movement_leg_due_v1";
/// Plugin-scheduled wake that publishes delayed remote reports.
pub const MOVEMENT_REPORT_WAKE_INGRESS: &str = "movement_report_wake_v1";
pub const MOVEMENT_SEMANTIC_HASH: &str =
    "2d3c7d8e4c526ca9ce231e0271818e62476c343e330ac9f914a01584e0dea383";

const REPORT_SCHEMA_HASH: &str = "31cf5c80ace3b83a2ffd5e3e227b2b4ee7284a0e9e5fd9794aee0b6f55ac1e3a";
const APPLY_SYSTEM: &str = "movement_lifecycle_apply_v1";
const VALIDATE_SYSTEM: &str = "movement_lifecycle_validate_v1";
const REPORT_SYSTEM: &str = "movement_report_publish_v1";
const INPUT_HASH_DOMAIN: &str = "canwu.movement.operation-input.v1";
const MAX_RECORDS_PER_KNOWLEDGE_BATCH: usize = 1_000;
/// Emitted when an operation reaches settlement after the runtime's outcome
/// budget is spent, so it is not silently lost.
pub const MOVEMENT_OPERATION_DROPPED_EVENT: &str = "canwu.movement.operation_dropped.v1";
/// Emitted when a report exceeds the knowledge payload limit and is withheld.
pub const MOVEMENT_REPORT_WITHHELD_EVENT: &str = "canwu.movement.report_withheld.v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct AdmittedMovementCommandV1 {
    command: canwu_api::CommandId,
    value: MovementCommandV1,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum InputIdentity<'a> {
    Command {
        holder: &'a KnowledgeHolderRef,
    },
    Incident {
        evidence: &'a DomainRecordVersionRef,
    },
}

/// The movement lifecycle plugin.
///
/// `evidence_kinds` lists the application record kinds the plugin may read
/// as evidence: delivery attempts named by orders, external-condition records
/// cited by reroutes, and incident evidence. They are the plugin's exact read
/// set, so a citation of any other kind is rejected.
#[derive(Clone, Debug, Default)]
pub struct MovementPlugin {
    evidence_kinds: Vec<DomainRecordKind>,
}

impl MovementPlugin {
    #[must_use]
    pub fn new(evidence_kinds: impl IntoIterator<Item = DomainRecordKind>) -> Self {
        let mut evidence_kinds = evidence_kinds.into_iter().collect::<Vec<_>>();
        evidence_kinds.sort();
        evidence_kinds.dedup();
        Self { evidence_kinds }
    }

    fn evidence_state_keys(&self) -> Vec<StateKey> {
        self.evidence_kinds
            .iter()
            .map(|kind| StateKey::new(kind.namespace.clone(), kind.name.clone()))
            .collect()
    }
}

fn runtime_state_key() -> StateKey {
    DomainRecordSchema::for_record::<MovementRuntimeRecord>().state_key()
}

impl SimulationPlugin for MovementPlugin {
    fn name(&self) -> &'static str {
        PLUGIN_NAME
    }

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    fn semantic_hash(&self) -> &'static str {
        MOVEMENT_SEMANTIC_HASH
    }

    fn validate_activation(&self, records: &[DomainRecord]) -> Result<(), CanwuError> {
        let mut found = false;
        for record in records.iter().filter(|record| {
            record
                .reference
                .kind
                .matches_type::<MovementRuntimeRecord>()
        }) {
            if found || record.reference != movement_runtime_reference().into_untyped() {
                return Err(invalid(
                    "movement activation contains multiple or misidentified runtime roots",
                ));
            }
            let state = record.decode_payload::<MovementRuntimeRecord>()?;
            if state.draft()?.payload != record.payload {
                return Err(invalid("movement runtime root is not canonically encoded"));
            }
            found = true;
        }
        Ok(())
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut runtime_schema = DomainRecordSchema::for_record::<MovementRuntimeRecord>();
        runtime_schema.payload_schema = PayloadSchema::Any;
        registrar.register_record_schema(runtime_schema)?;
        let report_schema = PluginKnowledgeSchema {
            id: movement_report_knowledge_schema_id(),
            schema_hash: REPORT_SCHEMA_HASH.to_owned(),
            writable: true,
            payload_schema: PayloadSchema::Any,
            subjects: vec![KnowledgeSubjectSchema {
                role: "movement_state".to_owned(),
                targets: vec![KnowledgeSubjectTargetKind::Domain(
                    DomainRecordKind::for_type::<MovementRuntimeRecord>(),
                )],
                required: true,
                multiple: false,
            }],
        };
        registrar.register_knowledge_schema(report_schema.clone())?;
        registrar.register_command(
            PluginActionDescriptor {
                name: MOVEMENT_COMMAND.to_owned(),
                description: "Admit one authority-bound movement lifecycle operation".to_owned(),
                payload_schema: PayloadSchema::Any,
                reads: vec![runtime_state_key()],
                writes: Vec::new(),
            },
            admit_movement_command,
        )?;
        // Only the command handler queues admitted operations; hosts cannot
        // author them.
        registrar.register_internal_ingress(PluginIngressDescriptor {
            name: MOVEMENT_OPERATION_INGRESS.to_owned(),
            description: "Settle one admitted movement command".to_owned(),
            class: IngressClass::Decision,
            payload_schema: PayloadSchema::Any,
        })?;
        registrar.register_ingress(PluginIngressDescriptor {
            name: MOVEMENT_INCIDENT_INGRESS.to_owned(),
            description: "Record one evidence-backed leg failure, reroute, or handoff".to_owned(),
            class: IngressClass::Information,
            payload_schema: PayloadSchema::Any,
        })?;
        registrar.register_internal_ingress(PluginIngressDescriptor {
            name: MOVEMENT_LEG_DUE_INGRESS.to_owned(),
            description: "Depart or arrive the current movement leg at its due time".to_owned(),
            class: IngressClass::ScheduledSystem,
            payload_schema: PayloadSchema::Any,
        })?;
        registrar.register_internal_ingress(PluginIngressDescriptor {
            name: MOVEMENT_REPORT_WAKE_INGRESS.to_owned(),
            description: "Publish delayed movement reports whose delay has elapsed".to_owned(),
            class: IngressClass::ScheduledSystem,
            payload_schema: PayloadSchema::Any,
        })?;

        let mut apply = BoundarySystemContract::new(
            APPLY_SYSTEM,
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        apply.reads = vec![
            StateKey::core_ingress(),
            StateKey::core_person_availability(),
            runtime_state_key(),
        ];
        apply.reads.extend(self.evidence_state_keys());
        apply.reads.sort();
        apply.reads.dedup();
        apply.writes = vec![runtime_state_key()];
        apply.plugin_ingress_targets = vec![PluginIngressTarget {
            target_plugin: PLUGIN_NAME.to_owned(),
            packet_type: MOVEMENT_LEG_DUE_INGRESS.to_owned(),
        }];
        apply.emits = vec![MOVEMENT_OPERATION_DROPPED_EVENT.to_owned()];
        apply.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(apply, settle_movement_lifecycle)?;

        let mut validate = BoundarySystemContract::new(
            VALIDATE_SYSTEM,
            BoundaryPhase::InvariantValidation,
            SystemCadence::EventDriven,
        );
        validate.reads = vec![runtime_state_key()];
        validate.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(validate, validate_movement_candidate)?;

        let mut reports = BoundarySystemContract::new(
            REPORT_SYSTEM,
            BoundaryPhase::PerspectiveAndReportMaterialization,
            SystemCadence::EventDriven,
        );
        reports.reads = vec![
            StateKey::core_ingress(),
            StateKey::core_person_availability(),
            runtime_state_key(),
        ];
        reports.writes = vec![runtime_state_key()];
        reports.knowledge_writes = vec![KnowledgeWriteGrant {
            schema: report_schema.id,
            visibilities: vec![StateVisibility::SameBoundary],
        }];
        reports.plugin_ingress_targets = vec![PluginIngressTarget {
            target_plugin: PLUGIN_NAME.to_owned(),
            packet_type: MOVEMENT_REPORT_WAKE_INGRESS.to_owned(),
        }];
        reports.emits = vec![MOVEMENT_REPORT_WITHHELD_EVENT.to_owned()];
        reports.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(reports, publish_movement_reports)
    }
}

/// Builds the tracked command for one movement operation.
pub fn movement_command(value: &MovementCommandV1) -> Result<Command, serde_json::Error> {
    Ok(Command::Plugin {
        plugin: PLUGIN_NAME.to_owned(),
        command: MOVEMENT_COMMAND.to_owned(),
        payload: serde_json::to_value(value)?,
    })
}

/// Builds the host ingress request for one application-reported incident.
/// Boundary systems deliver the same payload with `SchedulePluginIngress`
/// after declaring [`movement_incident_target`].
pub fn movement_incident_ingress(
    incident: &MovementIncidentV1,
    due_at: SimTime,
) -> Result<PluginIngressRequest, serde_json::Error> {
    Ok(PluginIngressRequest::new(
        PLUGIN_NAME,
        MOVEMENT_INCIDENT_INGRESS,
        due_at,
        serde_json::to_value(incident)?,
    ))
}

#[must_use]
pub fn movement_incident_target() -> PluginIngressTarget {
    PluginIngressTarget {
        target_plugin: PLUGIN_NAME.to_owned(),
        packet_type: MOVEMENT_INCIDENT_INGRESS.to_owned(),
    }
}

/// Reads the committed movement runtime, if one exists.
pub fn movement_state(canwu: &Canwu) -> Result<Option<(DomainRecord, MovementState)>, CanwuError> {
    let Some(record) = canwu
        .typed_domain_record(&movement_runtime_reference())
        .cloned()
    else {
        return Ok(None);
    };
    let state = record.decode_payload::<MovementRuntimeRecord>()?;
    Ok(Some((record, state)))
}

fn admit_movement_command(
    view: &SimulationView<'_>,
    context: &CommandContext,
    payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    if context.ingress == CommandIngress::LegacyDirect {
        return Err(CanwuError::new(
            ErrorCode::MixedCommandIngress,
            "movement operations require tracked command ingress",
        ));
    }
    if operation_bytes(payload)? > MovementLimitsV1::CURRENT.operation_bytes {
        return Err(CanwuError::new(
            ErrorCode::ValueOutOfRange,
            "movement command payload exceeds its size limit",
        ));
    }
    let value: MovementCommandV1 = decode(payload, "movement command")?;
    validate_operation_key(&value.operation_key)?;
    validate_holder_issuer(context, &value.holder)?;
    let state = load_state(view)?.map(|(_, state)| state);
    if let Some(state) = &state {
        let scope = MovementOperationScopeV1::Holder(value.holder.clone());
        if let Some(existing) = state.operation_outcome(&scope, &value.operation_key)? {
            let hash = input_hash(
                &InputIdentity::Command {
                    holder: &value.holder,
                },
                &value.operation,
            )?;
            if existing.canonical_input_hash == hash {
                return Ok(Vec::new());
            }
            return Err(CanwuError::new(
                ErrorCode::IdempotencyConflict,
                "movement operation key was reused with different input",
            ));
        }
        let limits = MovementLimitsV1::CURRENT;
        let held = state
            .operation_outcomes
            .values()
            .filter(|outcome| outcome.source.scope() == scope)
            .count();
        if held >= limits.operation_outcomes_per_holder
            || state.operation_outcomes.len()
                >= limits
                    .operation_outcomes_per_runtime
                    .saturating_sub(limits.incident_outcome_headroom)
        {
            return Err(CanwuError::new(
                ErrorCode::ValueOutOfRange,
                "movement runtime has no room for another operation outcome from this holder",
            ));
        }
    }
    // Operator and observer holders must exist; the kernel checks affected
    // entities before it queues the packet.
    let mut affected = Vec::new();
    if let MovementOperation::Order(request) = &value.operation {
        affected.extend(
            request
                .operator
                .iter()
                .chain(request.remote_observers.iter().map(|remote| &remote.holder))
                .map(holder_entity),
        );
    }
    Ok(vec![SystemDirective::EnqueuePluginIngress {
        after: SimDuration::ZERO,
        packet_type: MOVEMENT_OPERATION_INGRESS.to_owned(),
        priority: 0,
        payload: encode(&AdmittedMovementCommandV1 {
            command: context.command_id,
            value,
        })?,
        affected,
    }])
}

struct ViewEvidence<'a, 'b>(&'a SimulationView<'b>);

impl EvidenceReader for ViewEvidence<'_, '_> {
    fn version_exists(&self, version: &DomainRecordVersionRef) -> Result<bool, CanwuError> {
        self.0.domain_record_version_evidence_exists(version)
    }

    fn current_record(
        &self,
        version: &DomainRecordVersionRef,
    ) -> Result<Option<DomainRecord>, CanwuError> {
        if !self.0.domain_record_version_is_current(version)? {
            return Ok(None);
        }
        Ok(self
            .0
            .domain_record(&version.record)?
            .filter(|record| record.is_active())
            .cloned())
    }

    fn condition_exists(&self, record: &DomainRecordRef, version: u64) -> Result<bool, CanwuError> {
        Ok(self
            .0
            .domain_record(record)?
            .is_some_and(|current| current.version >= version))
    }
}

fn settle_movement_lifecycle(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let loaded = load_state(view)?;
    let (record, mut state) = match loaded {
        Some((record, state)) => (Some(record), state),
        None => (None, MovementState::default()),
    };
    let reader = ViewEvidence(view);
    let mut changed = false;
    let mut scheduled = Vec::new();
    let mut dues = Vec::new();
    let mut events = Vec::new();
    for ingress_id in &context.admitted_ingress {
        let Some(ingress) = view.ingress(*ingress_id)? else {
            continue;
        };
        let IngressPayload::Plugin {
            plugin,
            packet_type,
            payload,
            ..
        } = &ingress.payload
        else {
            continue;
        };
        if plugin != PLUGIN_NAME {
            continue;
        }
        match packet_type.as_str() {
            MOVEMENT_OPERATION_INGRESS => {
                // A packet that is not the exact product of its accepted
                // command carries no authority and is ignored rather than
                // failing the boundary.
                let Ok(admitted) = decode::<AdmittedMovementCommandV1>(payload, "command") else {
                    continue;
                };
                if ingress.cause != Some(CauseRef::Command(admitted.command)) {
                    continue;
                }
                let value = admitted.value;
                match settle_operation(
                    &mut state,
                    &value.operation_key,
                    &value.operation,
                    &OperationInput {
                        source: MovementOperationSourceV1::Command {
                            command: admitted.command,
                            holder: value.holder.clone(),
                        },
                        oversized: false,
                    },
                    context.at,
                    &reader,
                    &mut scheduled,
                )? {
                    Settlement::Unchanged => {}
                    Settlement::Changed => changed = true,
                    Settlement::Dropped => events.push(dropped(&value.operation_key)),
                }
            }
            MOVEMENT_INCIDENT_INGRESS => {
                // Malformed public packets are ignored; the ingress journal
                // still records what was sent.
                let Ok(incident) = decode::<MovementIncidentV1>(payload, "incident") else {
                    continue;
                };
                if validate_operation_key(&incident.operation_key).is_err() {
                    continue;
                }
                let oversized =
                    operation_bytes(payload)? > MovementLimitsV1::CURRENT.operation_bytes;
                match settle_operation(
                    &mut state,
                    &incident.operation_key,
                    &incident.operation,
                    &OperationInput {
                        source: MovementOperationSourceV1::Incident {
                            ingress: *ingress_id,
                            evidence: incident.evidence.clone(),
                        },
                        oversized,
                    },
                    context.at,
                    &reader,
                    &mut scheduled,
                )? {
                    Settlement::Unchanged => {}
                    Settlement::Changed => changed = true,
                    Settlement::Dropped => events.push(dropped(&incident.operation_key)),
                }
            }
            MOVEMENT_LEG_DUE_INGRESS => {
                if let Ok(due) = decode::<MovementLegDueV1>(payload, "movement leg due") {
                    dues.push(due);
                }
            }
            _ => {}
        }
    }
    changed |= state
        .allocate_pools(context.at, context.boundary_id)
        .map_err(CanwuError::from)?;
    for due in &dues {
        changed |= state.apply_due(due, context.at, &mut scheduled);
    }
    let is_dead = |person| {
        view.person_availability(person)
            .ok()
            .flatten()
            .is_some_and(|availability| availability.life == LifeState::Dead)
    };
    changed |= state
        .retire_closed(context.at, &is_dead)
        .map_err(CanwuError::from)?;
    changed |= state.prune_outcomes(context.at);
    if !changed {
        return Ok(BoundaryProposal {
            directives: events,
            ..BoundaryProposal::default()
        });
    }
    state.revision = state
        .revision
        .checked_add(1)
        .ok_or_else(|| invalid("movement runtime revision overflowed"))?;
    let draft = state.draft()?;
    let mut directives = vec![BoundaryDirective::MutateRecord {
        mutation: match &record {
            Some(record) => DomainRecordMutation::Update {
                record: draft,
                expected_version: record.version,
            },
            None => DomainRecordMutation::Create { record: draft },
        },
        summary: "Apply movement lifecycle operations and due legs".to_owned(),
    }];
    directives.extend(events);
    for due in scheduled {
        let pending = state
            .order_for_execution(due.execution)
            .and_then(|order| order.pending_due.as_ref());
        if pending != Some(&due) {
            continue;
        }
        directives.push(BoundaryDirective::SchedulePluginIngress {
            target_plugin: PLUGIN_NAME.to_owned(),
            after: due
                .due_at
                .checked_sub(context.at)
                .filter(|delay| !delay.is_negative())
                .ok_or_else(|| invalid("movement due work precedes its boundary"))?,
            packet_type: MOVEMENT_LEG_DUE_INGRESS.to_owned(),
            priority: 0,
            payload: encode(&due)?,
            affected: Vec::new(),
        });
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

struct OperationInput {
    source: MovementOperationSourceV1,
    oversized: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Settlement {
    Unchanged,
    Changed,
    /// The outcome budget is spent.
    Dropped,
}

/// Settles one keyed operation into a durable applied or rejected outcome.
///
/// Keys are unique per source scope. An exact retry is a no-op; a reuse of a
/// key with different input is recorded as a separate `Conflict` rejection.
fn settle_operation(
    state: &mut MovementState,
    operation_key: &str,
    operation: &MovementOperation,
    input: &OperationInput,
    at: SimTime,
    reader: &dyn EvidenceReader,
    scheduled: &mut Vec<MovementLegDueV1>,
) -> Result<Settlement, CanwuError> {
    let source = &input.source;
    let identity = match source {
        MovementOperationSourceV1::Command { holder, .. } => InputIdentity::Command { holder },
        MovementOperationSourceV1::Incident { evidence, .. } => {
            InputIdentity::Incident { evidence }
        }
    };
    let hash = input_hash(&identity, operation)?;
    let scoped = source.scope().outcome_key(operation_key)?;
    let (stored_key, result) = match state.operation_outcomes.get(&scoped) {
        Some(existing) if existing.canonical_input_hash == hash => {
            return Ok(Settlement::Unchanged);
        }
        Some(_) => (
            conflict_outcome_key(&scoped, source),
            Err(MovementError::new(
                MovementErrorCode::Conflict,
                "movement operation key was reused with different input",
            )),
        ),
        None => (scoped, Ok(())),
    };
    if state.operation_outcomes.contains_key(&stored_key) {
        return Ok(Settlement::Unchanged);
    }
    if state.operation_outcomes.len() >= MovementLimitsV1::CURRENT.operation_outcomes_per_runtime {
        return Ok(Settlement::Dropped);
    }
    let result = result.and_then(|()| {
        if input.oversized {
            Err(MovementError::new(
                MovementErrorCode::LimitExceeded,
                "movement operation payload exceeds its size limit",
            ))
        } else {
            Ok(())
        }
    });
    let actor = match source {
        MovementOperationSourceV1::Command { holder, .. } => OperationActor::Holder(holder),
        MovementOperationSourceV1::Incident { evidence, .. } => OperationActor::Incident(evidence),
    };
    let mut candidate = state.clone();
    let mut local = Vec::new();
    let result = result.and_then(|()| {
        candidate
            .apply_operation(operation, operation_key, &actor, at, reader, &mut local)
            .and_then(|()| candidate.validate())
    });
    let (disposition, rejection) = match result {
        Ok(()) => {
            *state = candidate;
            scheduled.extend(local);
            (MovementOperationDisposition::Applied, None)
        }
        Err(error) => (MovementOperationDisposition::Rejected, Some(error)),
    };
    state.operation_outcomes.insert(
        stored_key,
        MovementOperationOutcomeV1 {
            operation_key: operation_key.to_owned(),
            canonical_input_hash: hash,
            source: source.clone(),
            disposition,
            rejection_code: rejection.as_ref().map(|error| error.code),
            rejection_message: rejection.map(|error| error.message),
            execution: operation.execution(),
            settled_at: at,
        },
    );
    Ok(Settlement::Changed)
}

fn validate_movement_candidate(
    view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let reference = movement_runtime_reference();
    let record = match view.proposed_typed_domain_record(&reference)? {
        Some(record) => Some(record),
        None => view.typed_domain_record(&reference)?,
    };
    if let Some(record) = record {
        let state = record.decode_payload::<MovementRuntimeRecord>()?;
        state.validate().map_err(CanwuError::from)?;
        if state.draft()?.payload != record.payload {
            return Err(invalid(
                "movement runtime candidate is not canonically encoded",
            ));
        }
    }
    Ok(BoundaryProposal::default())
}

#[allow(clippy::too_many_lines)]
fn publish_movement_reports(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut triggered = false;
    for ingress_id in &context.admitted_ingress {
        if let Some(ingress) = view.ingress(*ingress_id)?
            && matches!(&ingress.payload, IngressPayload::Plugin { plugin, .. } if plugin == PLUGIN_NAME)
        {
            triggered = true;
            break;
        }
    }
    if !triggered {
        return Ok(BoundaryProposal::default());
    }
    let Some((record, mut state)) = load_state(view)? else {
        return Ok(BoundaryProposal::default());
    };
    let at = context.at;
    let limits = MovementLimitsV1::CURRENT;
    let provider = view
        .current_domain_record_version(&movement_runtime_reference().into_untyped())?
        .ok_or_else(|| invalid("movement report source version is unavailable"))?;
    let mut due: Vec<(KnowledgeHolderRef, TransportExecutionId, MovementReportV1)> = Vec::new();
    for order in state.orders.values() {
        for grant in &order.observers {
            let KnowledgeHolderRef::Person(person) = grant.holder else {
                continue;
            };
            let head = find_head(&state, &grant.holder, order.execution);
            if let (Some(closed), Some(head)) = (order.closed_at, head)
                && state.observation_heads[head].observed_as_of >= closed
            {
                continue;
            }
            if view
                .person_availability(person)?
                .is_some_and(|availability| availability.life == LifeState::Dead)
            {
                continue;
            }
            let Some(report) = project(&state, order, grant, at).map_err(CanwuError::from)? else {
                continue;
            };
            if head.is_some_and(|head| state.observation_heads[head].report_digest == report.digest)
            {
                continue;
            }
            due.push((grant.holder.clone(), order.execution, report));
        }
    }
    due.sort_by(|left, right| (&left.0, left.1).cmp(&(&right.0, right.1)));

    let mut directives = Vec::new();
    let mut heads_changed = false;
    let mut deferred = false;
    let mut published = 0_usize;
    let mut holders = 0_usize;
    let mut index = 0_usize;
    while index < due.len() {
        let holder = due[index].0.clone();
        let group_end = due[index..]
            .iter()
            .position(|item| item.0 != holder)
            .map_or(due.len(), |offset| index + offset);
        let room = limits
            .reports_per_boundary
            .saturating_sub(published)
            .min(MAX_RECORDS_PER_KNOWLEDGE_BATCH);
        if holders == limits.report_holders_per_boundary || room == 0 {
            deferred = true;
            break;
        }
        let take_end = group_end.min(index + room);
        let mut records = Vec::new();
        for (holder, execution, report) in &due[index..take_end] {
            let payload = encode(report)?;
            let head = MovementObservationHeadV1 {
                holder: holder.clone(),
                execution: *execution,
                role: report.role,
                observed_as_of: report.observed_as_of,
                published_at: at,
                report_digest: report.digest.clone(),
            };
            if serde_json::to_vec(&payload).map_or(true, |bytes| {
                bytes.len() > KnowledgeLimitsV1::CURRENT.payload_bytes_per_record
            }) {
                // An oversized report is withheld instead of failing the
                // boundary. The head still advances, so the report is not
                // recomputed and the execution can retire, and an event
                // records the withheld report.
                upsert_head(&mut state, head);
                heads_changed = true;
                directives.push(BoundaryDirective::Emit {
                    event_type: MOVEMENT_REPORT_WITHHELD_EVENT.to_owned(),
                    summary: format!(
                        "movement report on execution {} exceeds the knowledge payload limit",
                        execution.0
                    ),
                    affected: Vec::new(),
                });
                continue;
            }
            records.push(KnowledgeRecordDraft {
                schema: movement_report_knowledge_schema_id(),
                subjects: vec![KnowledgeSubject {
                    role: "movement_state".to_owned(),
                    target: KnowledgeSubjectTarget::DomainRecord(
                        movement_runtime_reference().into_untyped(),
                    ),
                }],
                payload,
                as_of: Some(report.observed_as_of),
                confidence_per_mille: match report.role {
                    crate::MovementObserverRole::Operator | crate::MovementObserverRole::Owner => {
                        1_000
                    }
                    crate::MovementObserverRole::DelayedRemote => 700,
                },
                origin: KnowledgeOrigin {
                    method: "movement_report_v1".to_owned(),
                    // A delayed report cites nothing current, which would
                    // reveal how recently the runtime changed.
                    evidence: match report.role {
                        crate::MovementObserverRole::DelayedRemote => Vec::new(),
                        crate::MovementObserverRole::Operator
                        | crate::MovementObserverRole::Owner => {
                            vec![EvidenceRef::DomainRecordVersion(provider.clone())]
                        }
                    },
                },
                supersedes: Vec::new(),
                contradicts: Vec::new(),
            });
            upsert_head(&mut state, head);
            heads_changed = true;
        }
        if records.is_empty() {
            if take_end < group_end {
                deferred = true;
                break;
            }
            index = group_end;
            continue;
        }
        directives.push(BoundaryDirective::PublishKnowledge {
            holder,
            visibility: StateVisibility::SameBoundary,
            producer_correlation: Some(format!(
                "{MOVEMENT_REPORT_KNOWLEDGE}::{}::{holders}",
                context.boundary_id
            )),
            records,
            summary: "Publish holder-relative movement reports".to_owned(),
        });
        published += take_end - index;
        holders += 1;
        if take_end < group_end {
            deferred = true;
            break;
        }
        index = group_end;
    }

    let wake = if deferred {
        Some(at)
    } else {
        next_remote_visibility(&state, at).map_err(CanwuError::from)?
    };
    let mut wake_changed = false;
    match wake {
        Some(time)
            if state
                .report_wake_at
                .is_none_or(|pending| pending <= at || time < pending) =>
        {
            directives.push(BoundaryDirective::SchedulePluginIngress {
                target_plugin: PLUGIN_NAME.to_owned(),
                after: time
                    .checked_sub(at)
                    .ok_or_else(|| invalid("movement report wake precedes its boundary"))?,
                packet_type: MOVEMENT_REPORT_WAKE_INGRESS.to_owned(),
                priority: 0,
                payload: serde_json::json!({ "due_at": time }),
                affected: Vec::new(),
            });
            state.report_wake_at = Some(time);
            wake_changed = true;
        }
        None if state.report_wake_at.is_some_and(|pending| pending <= at) => {
            state.report_wake_at = None;
            wake_changed = true;
        }
        _ => {}
    }
    if heads_changed || wake_changed {
        state.revision = state
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("movement runtime revision overflowed"))?;
        directives.insert(
            0,
            BoundaryDirective::MutateRecord {
                mutation: DomainRecordMutation::Update {
                    record: state.draft()?,
                    expected_version: record.version,
                },
                summary: "Persist movement report heads and delayed-report wake".to_owned(),
            },
        );
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

/// The earliest time after `at` at which a delayed remote observer's report
/// can change.
fn next_remote_visibility(
    state: &MovementState,
    at: SimTime,
) -> Result<Option<SimTime>, MovementError> {
    let mut next: Option<SimTime> = None;
    for order in state.orders.values() {
        let remotes = order
            .observers
            .iter()
            .filter(|grant| grant.role == crate::MovementObserverRole::DelayedRemote)
            .collect::<Vec<_>>();
        if remotes.is_empty() {
            continue;
        }
        let times = event_times(state, order, false)?;
        for grant in remotes {
            let Some(cut) = observer_cut(at, grant) else {
                continue;
            };
            if let Some(event) = times.iter().find(|time| **time > cut)
                && let Some(visible) = event.checked_add(grant.delay)
            {
                next = Some(next.map_or(visible, |current| current.min(visible)));
            }
        }
    }
    Ok(next)
}

fn find_head(
    state: &MovementState,
    holder: &KnowledgeHolderRef,
    execution: TransportExecutionId,
) -> Option<usize> {
    state
        .observation_heads
        .binary_search_by(|head| (&head.holder, head.execution).cmp(&(holder, execution)))
        .ok()
}

fn upsert_head(state: &mut MovementState, head: MovementObservationHeadV1) {
    match state.observation_heads.binary_search_by(|item| {
        (&item.holder, item.execution).cmp(&(&head.holder, head.execution))
    }) {
        Ok(position) => state.observation_heads[position] = head,
        Err(position) => state.observation_heads.insert(position, head),
    }
}

pub(crate) fn load_state(
    view: &SimulationView<'_>,
) -> Result<Option<(DomainRecord, MovementState)>, CanwuError> {
    let Some(record) = view.typed_domain_record(&movement_runtime_reference())? else {
        return Ok(None);
    };
    let state = record.decode_payload::<MovementRuntimeRecord>()?;
    state.validate().map_err(CanwuError::from)?;
    Ok(Some((record.clone(), state)))
}

fn input_hash(
    identity: &InputIdentity<'_>,
    operation: &MovementOperation,
) -> Result<String, CanwuError> {
    canonical_hash(INPUT_HASH_DOMAIN, &(identity, operation))
}

fn validate_operation_key(key: &str) -> Result<(), CanwuError> {
    if is_label(key, MovementLimitsV1::CURRENT.text_bytes) {
        Ok(())
    } else {
        Err(CanwuError::new(
            ErrorCode::InvalidPayload,
            "movement operation key must be a non-empty, trimmed, bounded label",
        ))
    }
}

fn validate_holder_issuer(
    context: &CommandContext,
    holder: &KnowledgeHolderRef,
) -> Result<(), CanwuError> {
    let subject = holder_entity(holder);
    let authorized = match (&context.issuer, holder) {
        (Issuer::Actor(actor), KnowledgeHolderRef::Person(person)) => actor == person,
        (Issuer::Human(_) | Issuer::Ai(_) | Issuer::Institution(_), _) => {
            context.authority.command_subject.as_ref() == Some(&subject)
        }
        _ => false,
    };
    if authorized {
        Ok(())
    } else {
        Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "movement command issuer does not control its holder",
        ))
    }
}

fn dropped(operation_key: &str) -> BoundaryDirective {
    BoundaryDirective::Emit {
        event_type: MOVEMENT_OPERATION_DROPPED_EVENT.to_owned(),
        summary: format!(
            "movement operation {operation_key} was dropped: the outcome budget is spent"
        ),
        affected: Vec::new(),
    }
}

fn operation_bytes(payload: &Value) -> Result<usize, CanwuError> {
    serde_json::to_vec(payload)
        .map(|bytes| bytes.len())
        .map_err(|error| {
            CanwuError::new(
                ErrorCode::InvalidPayload,
                format!("movement payload could not be measured: {error}"),
            )
        })
}

fn invalid(message: &str) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidDomainRecord, message)
}

fn decode<T: serde::de::DeserializeOwned>(value: &Value, label: &str) -> Result<T, CanwuError> {
    serde_json::from_value(value.clone()).map_err(|error| {
        CanwuError::new(
            ErrorCode::InvalidPayload,
            format!("{label} payload is invalid: {error}"),
        )
    })
}

fn encode<T: Serialize>(value: &T) -> Result<Value, CanwuError> {
    serde_json::to_value(value).map_err(|error| {
        CanwuError::new(
            ErrorCode::InvalidPayload,
            format!("movement payload could not be encoded: {error}"),
        )
    })
}
