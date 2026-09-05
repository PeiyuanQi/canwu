use crate::model::*;
use crate::{PLUGIN_NAME, PLUGIN_NAMESPACE};
use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundarySystemContract,
    Canwu, CanwuError, Command, CommandContext, CommandIngress, DomainRecordDraft,
    DomainRecordMutation, DomainRecordSchema, DomainRecordType, ErrorCode, EvidenceRef,
    IngressClass, IngressPayload, KnowledgeRecordKind, KnowledgeSchemaId, KnowledgeSubjectSchema,
    KnowledgeSubjectTargetKind, KnowledgeWriteGrant, PayloadSchema, PluginActionDescriptor,
    PluginIngressDescriptor, PluginIngressRequest, PluginRegistrar, RandomOperationTarget,
    RandomStreamKey, SimDuration, SimTime, SimulationPlugin, SimulationView, StateKey,
    StateVisibility, SystemCadence, SystemDirective,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::collections::BTreeMap;

pub const MILITARY_COMMAND: &str = "military_command_v1";
pub const MILITARY_COMMAND_INGRESS: &str = "military_command_v1";
pub const MILITARY_PROVIDER_ACK_INGRESS: &str = "military_provider_ack_v1";
pub const MILITARY_REPORT_KNOWLEDGE: &str = "military_report";
const VERSION: &str = "0.1.0";
const SEMANTIC_HASH: &str = "2a4b5d2d3f16ef3a37035f3c9c66e1742d3e2acfbfdc5f6e01b2b3be20d7d6f1";

#[derive(Clone, Debug, Deserialize, Serialize)]
struct AdmittedCommand {
    envelope: MilitaryCommandEnvelope,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ProviderAck {
    outcome: ProviderOutcome,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct MilitaryPlugin;

impl SimulationPlugin for MilitaryPlugin {
    fn name(&self) -> &'static str {
        PLUGIN_NAME
    }
    fn version(&self) -> &'static str {
        VERSION
    }
    fn semantic_hash(&self) -> &'static str {
        SEMANTIC_HASH
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        for schema in [
            DomainRecordSchema::for_record::<MilitaryCatalogRecord>(),
            DomainRecordSchema::for_record::<ForceStateRecord>(),
            DomainRecordSchema::for_record::<OperationStateRecord>(),
            DomainRecordSchema::for_record::<CombatStateRecord>(),
            DomainRecordSchema::for_record::<OccupationStateRecord>(),
            DomainRecordSchema::for_record::<MilitaryKnowledgeRecord>(),
            DomainRecordSchema::for_record::<ProviderOutcomeRecord>(),
            DomainRecordSchema::for_record::<MilitaryLedgerRecord>(),
        ] {
            registrar.register_record_schema(schema)?;
        }

        registrar.register_knowledge_schema(report_schema())?;
        registrar.register_command(
            PluginActionDescriptor {
                name: MILITARY_COMMAND.to_owned(),
                description: "Admit one military domain command".to_owned(),
                payload_schema: PayloadSchema::Any,
                reads: military_state_keys(),
                writes: Vec::new(),
            },
            admit_command,
        )?;
        registrar.register_ingress(PluginIngressDescriptor {
            name: MILITARY_COMMAND_INGRESS.to_owned(),
            description: "Apply one admitted military command".to_owned(),
            class: IngressClass::Decision,
            payload_schema: PayloadSchema::Any,
        })?;
        registrar.register_ingress(PluginIngressDescriptor {
            name: MILITARY_PROVIDER_ACK_INGRESS.to_owned(),
            description: "Acknowledge one exact pending military provider effect".to_owned(),
            class: IngressClass::Acknowledgement,
            payload_schema: PayloadSchema::Any,
        })?;

        let mut apply = BoundarySystemContract::new(
            "apply-military-ingress-v1",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        apply.reads = military_state_keys();
        apply.reads.push(StateKey::core_ingress());
        apply.writes = military_state_keys();
        apply.visibility = StateVisibility::SameBoundary;
        apply.random_streams = vec![military_random_stream()];
        apply.emits = vec!["canwu.military.transition_applied.v1".to_owned()];
        registrar.register_boundary_system(apply, apply_ingress)?;

        for (name, phase, handler) in [
            (
                "advance-military-operations-v1",
                BoundaryPhase::HistoricalCandidateEvaluation,
                advance_operations as canwu_api::BoundarySystemHandler,
            ),
            (
                "resolve-military-combat-v1",
                BoundaryPhase::ConditionalTransitionCommit,
                resolve_combat as canwu_api::BoundarySystemHandler,
            ),
            (
                "advance-military-occupation-v1",
                BoundaryPhase::StrategicAggregation,
                advance_occupation as canwu_api::BoundarySystemHandler,
            ),
        ] {
            let mut system = BoundarySystemContract::new(name, phase, SystemCadence::EventDriven);
            system.reads = military_state_keys();
            // The ingress system is the sole military writer. These systems
            // are reserved for derived work and must not claim duplicate
            // ownership of the same domain-record keys.
            system.writes = Vec::new();
            system.visibility = StateVisibility::SameBoundary;
            registrar.register_boundary_system(system, handler)?;
        }
        let mut report = BoundarySystemContract::new(
            "materialize-military-reports-v1",
            BoundaryPhase::PerspectiveAndReportMaterialization,
            SystemCadence::EventDriven,
        );
        report.reads = military_state_keys();
        report.knowledge_writes = vec![KnowledgeWriteGrant {
            schema: report_schema_id(),
            visibilities: vec![StateVisibility::SameBoundary],
        }];
        report.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(report, materialize_reports)
    }
}

pub fn military_plugin() -> MilitaryPlugin {
    MilitaryPlugin
}

pub fn military_command(command: MilitaryCommand) -> Result<Command, CanwuError> {
    let envelope = MilitaryCommandEnvelope {
        input_digest: input_digest(&command)?,
        command,
    };
    Ok(Command::Plugin {
        plugin: PLUGIN_NAME.to_owned(),
        command: MILITARY_COMMAND.to_owned(),
        payload: serde_json::to_value(envelope).map_err(encode)?,
    })
}

pub fn enqueue_provider_outcome(
    canwu: &mut Canwu,
    due_at: SimTime,
    outcome: ProviderOutcome,
) -> Result<canwu_api::IngressReceipt, CanwuError> {
    Ok(canwu.enqueue_plugin_ingress(PluginIngressRequest::new(
        PLUGIN_NAME,
        MILITARY_PROVIDER_ACK_INGRESS,
        due_at,
        serde_json::to_value(ProviderAck { outcome }).map_err(encode)?,
    ))?)
}

pub fn military_random_stream() -> RandomStreamKey {
    RandomStreamKey::new(PLUGIN_NAME, "military-operation", 1)
}
pub fn report_schema_id() -> KnowledgeSchemaId {
    KnowledgeSchemaId::new(
        KnowledgeRecordKind::new(PLUGIN_NAMESPACE, MILITARY_REPORT_KNOWLEDGE),
        1,
    )
}

fn report_schema() -> canwu_api::PluginKnowledgeSchema {
    canwu_api::PluginKnowledgeSchema {
        id: report_schema_id(),
        schema_hash: "6e4bf5e8cf2dff7fddc8c2e75d0d5f0bbdf43ce43d6e19e2a6c3ef0a6d8f4c1b".to_owned(),
        writable: true,
        payload_schema: PayloadSchema::Any,
        subjects: vec![KnowledgeSubjectSchema {
            role: "military_subject".to_owned(),
            targets: vec![KnowledgeSubjectTargetKind::AnyEntity],
            required: true,
            multiple: false,
        }],
    }
}

fn military_state_keys() -> Vec<StateKey> {
    [
        DomainRecordSchema::for_record::<MilitaryCatalogRecord>(),
        DomainRecordSchema::for_record::<ForceStateRecord>(),
        DomainRecordSchema::for_record::<OperationStateRecord>(),
        DomainRecordSchema::for_record::<CombatStateRecord>(),
        DomainRecordSchema::for_record::<OccupationStateRecord>(),
        DomainRecordSchema::for_record::<ProviderOutcomeRecord>(),
        DomainRecordSchema::for_record::<MilitaryLedgerRecord>(),
    ]
    .into_iter()
    .map(|s| s.state_key())
    .collect()
}

fn admit_command(
    view: &SimulationView<'_>,
    context: &CommandContext,
    payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    if context.ingress == CommandIngress::LegacyDirect {
        return Err(err(
            ErrorCode::MixedCommandIngress,
            "military commands require canonical command ingress",
        ));
    }
    let envelope: MilitaryCommandEnvelope = decode(payload, "military command")?;
    if envelope.input_digest != input_digest(&envelope.command)? {
        return Err(err(
            ErrorCode::InvalidPayload,
            "military command semantic digest mismatch",
        ));
    }
    validate_command(view, &envelope.command)?;
    Ok(vec![SystemDirective::EnqueuePluginIngress {
        after: SimDuration::ZERO,
        packet_type: MILITARY_COMMAND_INGRESS.to_owned(),
        priority: 0,
        payload: serde_json::to_value(AdmittedCommand { envelope }).map_err(encode)?,
        affected: Vec::new(),
    }])
}

fn validate_command(
    view: &SimulationView<'_>,
    command: &MilitaryCommand,
) -> Result<(), CanwuError> {
    let operation = command_operation(command);
    if operation.as_str().is_empty() {
        return Err(err(
            ErrorCode::InvalidPayload,
            "military operation key is empty",
        ));
    }
    if let Some(force) = command_force(command) {
        if let Some(record) = view.typed_domain_record(&force_reference(force))? {
            let state = record.decode_payload::<ForceStateRecord>()?;
            let expected = command_expected_revision(command);
            if expected != Some(state.meta.revision) && expected.is_some() {
                return Err(err(
                    ErrorCode::DomainRecordVersionConflict,
                    "military force revision is stale",
                ));
            }
        }
    }
    if let MilitaryCommand::SetOccupationPolicy {
        security_per_mille,
        collaboration_per_mille,
        extraction_burden_per_mille,
        ..
    } = command
    {
        validate_per_mille(*security_per_mille, "security")?;
        validate_per_mille(*collaboration_per_mille, "collaboration")?;
        validate_per_mille(*extraction_burden_per_mille, "extraction burden")?;
    }
    Ok(())
}

fn apply_ingress(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut directives = Vec::new();
    for id in &context.admitted_ingress {
        let Some(ingress) = view.ingress(*id)? else {
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
        if packet_type == MILITARY_COMMAND_INGRESS {
            let admitted: AdmittedCommand = decode(payload, "admitted military command")?;
            apply_command(view, context, &admitted.envelope.command, &mut directives)?;
        }
        if packet_type == MILITARY_PROVIDER_ACK_INGRESS {
            let ack: ProviderAck = decode(payload, "military provider acknowledgement")?;
            apply_ack(view, context, &ack.outcome, &mut directives)?;
        }
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

fn apply_command(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    command: &MilitaryCommand,
    out: &mut Vec<BoundaryDirective>,
) -> Result<(), CanwuError> {
    let at = context.at;
    match command {
        MilitaryCommand::CreateForce {
            force,
            owner,
            location,
            authorized_strength,
            branch,
            commander,
            ..
        } => {
            if view.typed_domain_record(&force_reference(force))?.is_some() {
                return Err(err(
                    ErrorCode::DuplicateDomainRecord,
                    "force already exists",
                ));
            }
            let unit = SubunitState {
                id: SubunitId::new(format!("{}:initial", force.as_str()))?,
                branch: branch.clone(),
                strength: *authorized_strength,
                training_per_mille: 0,
                equipment_per_mille: 0,
                fatigue_per_mille: 0,
                status: SubunitStatus::Active,
            };
            let mut state = ForceState {
                meta: MilitaryRecordMeta::new(1, at, &())?,
                id: force.clone(),
                owner: owner.clone(),
                formation_parent: None,
                location: location.clone(),
                commander: *commander,
                subunits: BTreeMap::from([(unit.id.clone(), unit)]),
                authorized_strength: *authorized_strength,
                actual_strength: *authorized_strength,
                training_per_mille: 0,
                equipment_per_mille: 0,
                fatigue_per_mille: 0,
                supply_per_mille: 1_000,
                morale_per_mille: 500,
                discipline_per_mille: 500,
                cohesion_per_mille: 500,
                loyalty_per_mille: 500,
                casualties: 0,
                missing: 0,
                prisoners: 0,
                deserters: 0,
                replacements_pending: 0,
                transport_capacity: 0,
                active_operation: None,
                active_order: None,
                status: ForceStatus::Forming,
            };
            state.meta = MilitaryRecordMeta::new(1, at, &state)?;
            state.validate()?;
            create(out, force_reference(force), &state, "Create military force")?;
        }
        MilitaryCommand::AssignCommander {
            force, commander, ..
        } => update_force(view, out, force, at, |s| {
            s.commander = Some(*commander);
            Ok(())
        })?,
        MilitaryCommand::Recruit {
            force,
            subunit,
            branch,
            quantity,
            ..
        } => update_force(view, out, force, at, |s| {
            if s.subunits.contains_key(subunit) {
                return Err(err(
                    ErrorCode::IdempotencyConflict,
                    "subunit already exists",
                ));
            }
            s.subunits.insert(
                subunit.clone(),
                SubunitState {
                    id: subunit.clone(),
                    branch: branch.clone(),
                    strength: *quantity,
                    training_per_mille: 0,
                    equipment_per_mille: 0,
                    fatigue_per_mille: 0,
                    status: SubunitStatus::Active,
                },
            );
            s.actual_strength = s
                .actual_strength
                .saturating_add(*quantity)
                .min(s.authorized_strength);
            Ok(())
        })?,
        MilitaryCommand::TrainAndEquip {
            force,
            training_delta,
            equipment_delta,
            ..
        } => update_force(view, out, force, at, |s| {
            s.training_per_mille = s
                .training_per_mille
                .saturating_add(*training_delta)
                .min(1_000);
            s.equipment_per_mille = s
                .equipment_per_mille
                .saturating_add(*equipment_delta)
                .min(1_000);
            Ok(())
        })?,
        MilitaryCommand::OrderMarch {
            force,
            operation_id,
            destination,
            objective,
            ..
        } => {
            update_force(view, out, force, at, |s| {
                s.active_operation = Some(operation_id.clone());
                s.status = ForceStatus::Moving;
                Ok(())
            })?;
            let operation = OperationState {
                meta: MilitaryRecordMeta::new(1, at, &())?,
                id: operation_id.clone(),
                key: command_operation(command).clone(),
                owner: force_owner(view, force)?,
                objective: objective.clone(),
                forces: vec![force.clone()],
                phase: OperationPhase::Moving,
                from: force_location(view, force)?,
                destination: destination.clone(),
                route_digest: digest(&(force, destination))?,
                terrain: String::new(),
                weather: String::new(),
                started_at: at,
                due_at: at
                    .checked_add(SimDuration::minutes(1))
                    .ok_or_else(|| err(ErrorCode::InvalidDuration, "military march overflow"))?,
                command_delay_minutes: 0,
                supply_line: None,
                exit_condition: String::new(),
            };
            create(
                out,
                operation_reference(operation_id),
                &operation,
                "Order military march",
            )?;
        }
        MilitaryCommand::PlanOperation {
            operation_id,
            owner,
            objective,
            force,
            from,
            destination,
            ..
        } => {
            let op = OperationState {
                meta: MilitaryRecordMeta::new(1, at, &())?,
                id: operation_id.clone(),
                key: command_operation(command).clone(),
                owner: owner.clone(),
                objective: objective.clone(),
                forces: vec![force.clone()],
                phase: OperationPhase::Planned,
                from: from.clone(),
                destination: destination.clone(),
                route_digest: digest(&(from, destination))?,
                terrain: String::new(),
                weather: String::new(),
                started_at: at,
                due_at: at,
                command_delay_minutes: 0,
                supply_line: None,
                exit_condition: String::new(),
            };
            create(
                out,
                operation_reference(operation_id),
                &op,
                "Plan military operation",
            )?;
        }
        MilitaryCommand::EstablishOccupation {
            occupation,
            force,
            node,
            ..
        } => {
            let occ = OccupationState {
                meta: MilitaryRecordMeta::new(1, at, &())?,
                id: occupation.clone(),
                node: node.clone(),
                occupying_force: force.clone(),
                military_control_per_mille: 1_000,
                garrison_strength: 0,
                administrative_reach_per_mille: 0,
                security_per_mille: 500,
                fiscal_capacity_per_mille: 0,
                legitimacy_per_mille: 0,
                collaboration_per_mille: 0,
                resistance_per_mille: 500,
                extraction_burden_per_mille: 0,
                integration: IntegrationStage::MilitaryControl,
                policy_revision: 1,
                pending_provider_outcomes: Default::default(),
            };
            create(
                out,
                occupation_reference(occupation),
                &occ,
                "Establish military occupation",
            )?;
        }
        MilitaryCommand::SetOccupationPolicy {
            occupation,
            policy_revision,
            security_per_mille,
            collaboration_per_mille,
            extraction_burden_per_mille,
            ..
        } => update_occupation(view, out, occupation, at, |s| {
            if s.policy_revision != *policy_revision {
                return Err(err(
                    ErrorCode::DomainRecordVersionConflict,
                    "occupation policy revision is stale",
                ));
            }
            s.security_per_mille = *security_per_mille;
            s.collaboration_per_mille = *collaboration_per_mille;
            s.extraction_burden_per_mille = *extraction_burden_per_mille;
            s.policy_revision += 1;
            Ok(())
        })?,
        MilitaryCommand::MilitaryAdministrationAction {
            occupation,
            provider_plugin,
            expected_provider_version,
            ..
        } => {
            let ledger = ledger(view)?;
            let key = command_operation(command).clone();
            let mut next = ledger.clone();
            next.pending.insert(
                key.clone(),
                PendingMilitaryEffect {
                    operation: key.clone(),
                    provider_plugin: provider_plugin.clone(),
                    kind: "administration".to_owned(),
                    expected_source_version: *expected_provider_version,
                    state: PendingEffectState::Pending,
                },
            );
            next.meta = MilitaryRecordMeta::new(next.meta.revision + 1, at, &next)?;
            upsert(
                view,
                out,
                ledger_reference(),
                &next,
                "Queue military provider effect",
            )?;
            let _ = occupation;
        }
        MilitaryCommand::Recon { .. }
        | MilitaryCommand::PrepareAmbush { .. }
        | MilitaryCommand::ExecuteSpecialOperation { .. } => {
            let _ = view.random_range_for_operation(
                &military_random_stream(),
                EvidenceRef::Boundary(context.boundary_id),
                "military_command",
                command_operation(command).as_str(),
                RandomOperationTarget::CanonicalKey(command_operation(command).to_string()),
                0,
                1_000,
                "resolve military operation uncertainty",
            )?;
            out.push(BoundaryDirective::Emit {
                event_type: "canwu.military.transition_applied.v1".to_owned(),
                summary: "Resolve military operation uncertainty".to_owned(),
                affected: Vec::new(),
            });
        }
    }
    Ok(())
}

fn err(code: ErrorCode, message: impl Into<String>) -> CanwuError {
    CanwuError::new(code, message.into())
}
fn encode(error: serde_json::Error) -> CanwuError {
    err(ErrorCode::InvalidPayload, error.to_string())
}
fn decode<T: DeserializeOwned>(value: &Value, label: &str) -> Result<T, CanwuError> {
    serde_json::from_value(value.clone()).map_err(|e| {
        err(
            ErrorCode::InvalidPayload,
            format!("{label} is invalid: {e}"),
        )
    })
}
fn command_operation(command: &MilitaryCommand) -> &MilitaryOperationKey {
    match command {
        MilitaryCommand::CreateForce { operation, .. }
        | MilitaryCommand::AssignCommander { operation, .. }
        | MilitaryCommand::Recruit { operation, .. }
        | MilitaryCommand::TrainAndEquip { operation, .. }
        | MilitaryCommand::OrderMarch { operation, .. }
        | MilitaryCommand::PlanOperation { operation, .. }
        | MilitaryCommand::Recon { operation, .. }
        | MilitaryCommand::PrepareAmbush { operation, .. }
        | MilitaryCommand::ExecuteSpecialOperation { operation, .. }
        | MilitaryCommand::EstablishOccupation { operation, .. }
        | MilitaryCommand::SetOccupationPolicy { operation, .. }
        | MilitaryCommand::MilitaryAdministrationAction { operation, .. } => operation,
    }
}
fn command_force(command: &MilitaryCommand) -> Option<&ForceId> {
    match command {
        MilitaryCommand::CreateForce { force, .. }
        | MilitaryCommand::AssignCommander { force, .. }
        | MilitaryCommand::Recruit { force, .. }
        | MilitaryCommand::TrainAndEquip { force, .. }
        | MilitaryCommand::OrderMarch { force, .. }
        | MilitaryCommand::Recon { force, .. }
        | MilitaryCommand::PrepareAmbush { force, .. }
        | MilitaryCommand::ExecuteSpecialOperation { force, .. }
        | MilitaryCommand::EstablishOccupation { force, .. }
        | MilitaryCommand::PlanOperation { force, .. } => Some(force),
        MilitaryCommand::SetOccupationPolicy { .. }
        | MilitaryCommand::MilitaryAdministrationAction { .. } => None,
    }
}
fn command_expected_revision(command: &MilitaryCommand) -> Option<u64> {
    match command {
        MilitaryCommand::AssignCommander {
            expected_force_revision,
            ..
        }
        | MilitaryCommand::Recruit {
            expected_force_revision,
            ..
        }
        | MilitaryCommand::TrainAndEquip {
            expected_force_revision,
            ..
        }
        | MilitaryCommand::OrderMarch {
            expected_force_revision,
            ..
        }
        | MilitaryCommand::Recon {
            expected_force_revision,
            ..
        }
        | MilitaryCommand::PrepareAmbush {
            expected_force_revision,
            ..
        }
        | MilitaryCommand::EstablishOccupation {
            expected_force_revision,
            ..
        } => Some(*expected_force_revision),
        _ => None,
    }
}
fn input_digest<T: Serialize>(value: &T) -> Result<String, CanwuError> {
    crate::model::input_digest(value)
}
fn create<T: DomainRecordType>(
    out: &mut Vec<BoundaryDirective>,
    reference: canwu_api::TypedDomainRecordRef<T>,
    payload: &T::Payload,
    summary: &str,
) -> Result<(), CanwuError>
where
    T::Payload: Serialize,
{
    out.push(BoundaryDirective::MutateRecord {
        mutation: DomainRecordMutation::Create {
            record: DomainRecordDraft::from_typed(reference, payload)?,
        },
        summary: summary.to_owned(),
    });
    Ok(())
}
fn upsert<T: DomainRecordType>(
    view: &SimulationView<'_>,
    out: &mut Vec<BoundaryDirective>,
    reference: canwu_api::TypedDomainRecordRef<T>,
    payload: &T::Payload,
    summary: &str,
) -> Result<(), CanwuError>
where
    T::Payload: Serialize,
{
    let current = view.typed_domain_record(&reference)?.ok_or_else(|| {
        err(
            ErrorCode::DomainRecordNotFound,
            "military record is unavailable",
        )
    })?;
    out.push(BoundaryDirective::MutateRecord {
        mutation: DomainRecordMutation::Update {
            record: DomainRecordDraft::from_typed(reference, payload)?,
            expected_version: current.version,
        },
        summary: summary.to_owned(),
    });
    Ok(())
}
fn update_force(
    view: &SimulationView<'_>,
    out: &mut Vec<BoundaryDirective>,
    id: &ForceId,
    at: SimTime,
    change: impl FnOnce(&mut ForceState) -> Result<(), CanwuError>,
) -> Result<(), CanwuError> {
    let reference = force_reference(id);
    let record = view
        .typed_domain_record(&reference)?
        .ok_or_else(|| err(ErrorCode::DomainRecordNotFound, "force is unavailable"))?;
    let mut state = record.decode_payload::<ForceStateRecord>()?;
    change(&mut state)?;
    state.meta.revision = record.version + 1;
    state.meta.established_at = at;
    state.meta.semantic_digest = digest(&state)?;
    state.validate()?;
    upsert(view, out, reference, &state, "Update military force")
}
fn update_occupation(
    view: &SimulationView<'_>,
    out: &mut Vec<BoundaryDirective>,
    id: &OccupationId,
    at: SimTime,
    change: impl FnOnce(&mut OccupationState) -> Result<(), CanwuError>,
) -> Result<(), CanwuError> {
    let reference = occupation_reference(id);
    let record = view
        .typed_domain_record(&reference)?
        .ok_or_else(|| err(ErrorCode::DomainRecordNotFound, "occupation is unavailable"))?;
    let mut state = record.decode_payload::<OccupationStateRecord>()?;
    change(&mut state)?;
    state.meta.revision = record.version + 1;
    state.meta.established_at = at;
    state.meta.semantic_digest = digest(&state)?;
    state.validate()?;
    upsert(view, out, reference, &state, "Update military occupation")
}
fn force_owner(
    view: &SimulationView<'_>,
    id: &ForceId,
) -> Result<canwu_api::EntityRef, CanwuError> {
    Ok(view
        .typed_domain_record(&force_reference(id))?
        .ok_or_else(|| err(ErrorCode::DomainRecordNotFound, "force is unavailable"))?
        .decode_payload::<ForceStateRecord>()?
        .owner)
}
fn force_location(view: &SimulationView<'_>, id: &ForceId) -> Result<MilitaryNodeId, CanwuError> {
    Ok(view
        .typed_domain_record(&force_reference(id))?
        .ok_or_else(|| err(ErrorCode::DomainRecordNotFound, "force is unavailable"))?
        .decode_payload::<ForceStateRecord>()?
        .location)
}
fn ledger(view: &SimulationView<'_>) -> Result<MilitaryLedger, CanwuError> {
    Ok(view
        .typed_domain_record(&ledger_reference())?
        .ok_or_else(|| {
            err(
                ErrorCode::DomainRecordNotFound,
                "military ledger is unavailable",
            )
        })?
        .decode_payload::<MilitaryLedgerRecord>()?)
}
fn apply_ack(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    outcome: &ProviderOutcome,
    out: &mut Vec<BoundaryDirective>,
) -> Result<(), CanwuError> {
    let reference = ledger_reference();
    let record = view.typed_domain_record(&reference)?.ok_or_else(|| {
        err(
            ErrorCode::DomainRecordNotFound,
            "military ledger is unavailable",
        )
    })?;
    let mut state = record.decode_payload::<MilitaryLedgerRecord>()?;
    let pending = state.pending.get(&outcome.operation).ok_or_else(|| {
        err(
            ErrorCode::InvalidAuthority,
            "provider outcome has no matching pending military effect",
        )
    })?;
    if pending.provider_plugin != outcome.provider_plugin
        || pending.expected_source_version != outcome.provider_version
    {
        return Err(err(
            ErrorCode::InvalidAuthority,
            "provider outcome identity does not match pending effect",
        ));
    }
    state.pending.remove(&outcome.operation);
    state.outcomes.insert(
        outcome.operation.clone(),
        MilitaryOutcome {
            operation: outcome.operation.clone(),
            disposition: match outcome.disposition {
                ProviderDisposition::Rejected => OutcomeDisposition::Rejected,
                _ => OutcomeDisposition::Accepted,
            },
            record: outcome.provider_record.clone(),
            message: "Provider outcome acknowledged".to_owned(),
            at: context.at,
        },
    );
    state.meta.revision = record.version + 1;
    state.meta.established_at = context.at;
    state.meta.semantic_digest = digest(&state)?;
    upsert(
        view,
        out,
        reference,
        &state,
        "Acknowledge military provider outcome",
    )
}
fn advance_operations(
    _: &SimulationView<'_>,
    _: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    Ok(BoundaryProposal::default())
}
fn resolve_combat(
    _: &SimulationView<'_>,
    _: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    Ok(BoundaryProposal::default())
}
fn advance_occupation(
    _: &SimulationView<'_>,
    _: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    Ok(BoundaryProposal::default())
}
fn materialize_reports(
    _: &SimulationView<'_>,
    _: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    Ok(BoundaryProposal::default())
}
