use crate::model::*;
use crate::{PLUGIN_NAME, PLUGIN_NAMESPACE};
use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundarySystemContract,
    Canwu, CanwuError, Command, CommandContext, CommandIngress, DomainRecordDraft,
    DomainRecordKind, DomainRecordMutation, DomainRecordSchema, DomainRecordType, ErrorCode,
    EvidenceRef, IngressClass, IngressPayload, Issuer, KnowledgeHolderRef, KnowledgeOrigin,
    KnowledgeRecordDraft, KnowledgeRecordKind, KnowledgeSchemaId, KnowledgeSubject,
    KnowledgeSubjectSchema, KnowledgeSubjectTarget, KnowledgeSubjectTargetKind,
    KnowledgeWriteGrant, PayloadSchema, PluginActionDescriptor, PluginIngressDescriptor,
    PluginIngressRequest, PluginRegistrar, RandomOperationTarget, RandomStreamKey, SimDuration,
    SimTime, SimulationPlugin, SimulationView, StateKey, StateVisibility, SystemCadence,
    SystemDirective,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

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
        apply.plugin_ingress_targets = vec![canwu_api::PluginIngressTarget {
            target_plugin: PLUGIN_NAME.to_owned(),
            packet_type: MILITARY_COMMAND_INGRESS.to_owned(),
        }];
        registrar.register_boundary_system(apply, apply_ingress)?;
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
            role: "force".to_owned(),
            targets: vec![KnowledgeSubjectTargetKind::Domain(
                DomainRecordKind::for_type::<ForceStateRecord>(),
            )],
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
    validate_command(view, context, &envelope.command)?;
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
    context: &CommandContext,
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
            if let Issuer::Actor(actor) = context.issuer {
                if state.commander != Some(actor) {
                    return Err(err(
                        ErrorCode::InvalidAuthority,
                        "actor does not command this force",
                    ));
                }
            } else {
                return Err(err(
                    ErrorCode::InvalidAuthority,
                    "military force commands require an actor issuer",
                ));
            }
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
    let operation_key = command_operation(command).clone();
    let command_digest = input_digest(command)?;
    let internal_tick = matches!(command, MilitaryCommand::AdvanceTick { .. });
    if !internal_tick {
        if let Some(record) = view.typed_domain_record(&ledger_reference())? {
            let ledger = record.decode_payload::<MilitaryLedgerRecord>()?;
            if let Some(existing) = ledger.outcomes.get(&operation_key) {
                if existing.input_digest == command_digest {
                    return Ok(());
                }
                return Err(err(
                    ErrorCode::IdempotencyConflict,
                    "military operation key was reused with different input",
                ));
            }
        }
    }
    match command {
        MilitaryCommand::CreateForce {
            force,
            owner,
            location,
            authorized_strength,
            initial_strength,
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
            let initial_strength = initial_strength.unwrap_or(*authorized_strength);
            if initial_strength > *authorized_strength {
                return Err(err(
                    ErrorCode::InvalidPayload,
                    "initial strength exceeds authorized strength",
                ));
            }
            let unit = SubunitState {
                id: SubunitId::new(format!("{}:initial", force.as_str()))?,
                branch: branch.clone(),
                strength: initial_strength,
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
                actual_strength: initial_strength,
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
                prepared_ambush: None,
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
            let recruitable =
                (*quantity).min(s.authorized_strength.saturating_sub(s.actual_strength));
            if recruitable == 0 {
                return Err(err(
                    ErrorCode::InvalidDecision,
                    "force has no remaining recruitment capacity",
                ));
            }
            s.subunits.insert(
                subunit.clone(),
                SubunitState {
                    id: subunit.clone(),
                    branch: branch.clone(),
                    strength: recruitable,
                    training_per_mille: 0,
                    equipment_per_mille: 0,
                    fatigue_per_mille: 0,
                    status: SubunitStatus::Active,
                },
            );
            s.actual_strength = s.actual_strength.saturating_add(recruitable);
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
            opposing_force,
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
                opposing_force: opposing_force.clone(),
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
            schedule_tick(
                out,
                SimDuration::minutes(1),
                Some(operation_id.clone()),
                None,
                command_operation(command).clone(),
            )?;
        }
        MilitaryCommand::PlanOperation {
            operation_id,
            owner,
            objective,
            force,
            from,
            destination,
            opposing_force,
            ..
        } => {
            let op = OperationState {
                meta: MilitaryRecordMeta::new(1, at, &())?,
                id: operation_id.clone(),
                key: command_operation(command).clone(),
                owner: owner.clone(),
                objective: objective.clone(),
                forces: vec![force.clone()],
                opposing_force: opposing_force.clone(),
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
            let force_record = view
                .typed_domain_record(&force_reference(force))?
                .ok_or_else(|| {
                    err(
                        ErrorCode::DomainRecordNotFound,
                        "occupation force is unavailable",
                    )
                })?;
            let force_state = force_record.decode_payload::<ForceStateRecord>()?;
            if force_state.location != *node || force_state.status == ForceStatus::Routing {
                return Err(err(
                    ErrorCode::InvalidDecision,
                    "force must be present and not routing before occupation",
                ));
            }
            let occ = OccupationState {
                meta: MilitaryRecordMeta::new(1, at, &())?,
                id: occupation.clone(),
                node: node.clone(),
                occupying_force: force.clone(),
                military_control_per_mille: 700,
                garrison_strength: force_state.actual_strength / 3,
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
            schedule_tick(
                out,
                SimDuration::days(1),
                None,
                Some(occupation.clone()),
                command_operation(command).clone(),
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
        MilitaryCommand::AdvanceTick {
            operation,
            occupation,
            ..
        } => advance_tick(view, context, out, operation.as_ref(), occupation.as_ref())?,
        MilitaryCommand::PrepareAmbush {
            force,
            node,
            tactic,
            ..
        } => {
            update_force(view, out, force, at, |state| {
                state.prepared_ambush = Some(AmbushPreparation {
                    node: node.clone(),
                    tactic: tactic.clone(),
                    concealment_per_mille: 700,
                    prepared_at: at,
                    expires_at: Some(at.checked_add(SimDuration::days(7)).ok_or_else(|| {
                        err(ErrorCode::InvalidDuration, "ambush expiry overflow")
                    })?),
                });
                Ok(())
            })?;
        }
        MilitaryCommand::Recon { .. } | MilitaryCommand::ExecuteSpecialOperation { .. } => {
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
    if !internal_tick
        && !matches!(
            command,
            MilitaryCommand::MilitaryAdministrationAction { .. }
        )
    {
        record_command_outcome(view, out, command, command_digest, at)?;
    }
    Ok(())
}

fn record_command_outcome(
    view: &SimulationView<'_>,
    out: &mut Vec<BoundaryDirective>,
    command: &MilitaryCommand,
    input_digest: String,
    at: SimTime,
) -> Result<(), CanwuError> {
    let key = command_operation(command).clone();
    let current = view.typed_domain_record(&ledger_reference())?;
    let mut ledger = current
        .as_ref()
        .map(|record| record.decode_payload::<MilitaryLedgerRecord>())
        .transpose()?
        .unwrap_or(MilitaryLedger {
            meta: MilitaryRecordMeta::new(1, at, &())?,
            outcomes: BTreeMap::new(),
            pending: BTreeMap::new(),
        });
    ledger.outcomes.insert(
        key.clone(),
        MilitaryOutcome {
            operation: key,
            input_digest,
            disposition: OutcomeDisposition::Accepted,
            record: "command".to_owned(),
            message: "Military command applied exactly once".to_owned(),
            at,
        },
    );
    ledger.meta.revision = current.map_or(1, |record| record.version + 1);
    ledger.meta.established_at = at;
    ledger.meta.semantic_digest = digest(&ledger)?;
    match current {
        Some(_record) => upsert(
            view,
            out,
            ledger_reference(),
            &ledger,
            "Record military command outcome",
        ),
        None => create(
            out,
            ledger_reference(),
            &ledger,
            "Create military command ledger",
        ),
    }
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
        | MilitaryCommand::MilitaryAdministrationAction { operation, .. }
        | MilitaryCommand::AdvanceTick {
            operation_key: operation,
            ..
        } => operation,
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
        | MilitaryCommand::MilitaryAdministrationAction { .. }
        | MilitaryCommand::AdvanceTick { .. } => None,
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
            input_digest: String::new(),
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
fn schedule_tick(
    out: &mut Vec<BoundaryDirective>,
    after: SimDuration,
    operation: Option<OperationId>,
    occupation: Option<OccupationId>,
    operation_key: MilitaryOperationKey,
) -> Result<(), CanwuError> {
    let command = MilitaryCommand::AdvanceTick {
        operation,
        occupation,
        operation_key: operation_key.clone(),
    };
    let envelope = MilitaryCommandEnvelope {
        input_digest: input_digest(&command)?,
        command,
    };
    out.push(BoundaryDirective::SchedulePluginIngress {
        target_plugin: PLUGIN_NAME.to_owned(),
        after,
        packet_type: MILITARY_COMMAND_INGRESS.to_owned(),
        priority: 0,
        payload: serde_json::to_value(AdmittedCommand { envelope }).map_err(encode)?,
        affected: Vec::new(),
    });
    Ok(())
}

fn advance_tick(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    out: &mut Vec<BoundaryDirective>,
    operation: Option<&OperationId>,
    occupation: Option<&OccupationId>,
) -> Result<(), CanwuError> {
    if let Some(operation) = operation {
        advance_operation(view, context, out, operation)?;
    }
    if let Some(occupation) = occupation {
        advance_occupation_state(view, context, out, occupation)?;
    }
    Ok(())
}

fn advance_operation(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    out: &mut Vec<BoundaryDirective>,
    id: &OperationId,
) -> Result<(), CanwuError> {
    let reference = operation_reference(id);
    let record = view
        .typed_domain_record(&reference)?
        .ok_or_else(|| err(ErrorCode::DomainRecordNotFound, "operation is unavailable"))?;
    let mut operation = record.decode_payload::<OperationStateRecord>()?;
    if matches!(
        operation.phase,
        OperationPhase::Completed | OperationPhase::Failed | OperationPhase::Cancelled
    ) {
        return Ok(());
    }
    let force_id = operation
        .forces
        .first()
        .cloned()
        .ok_or_else(|| err(ErrorCode::InvalidDomainRecord, "operation has no force"))?;
    let force_record = view
        .typed_domain_record(&force_reference(&force_id))?
        .ok_or_else(|| {
            err(
                ErrorCode::DomainRecordNotFound,
                "operation force is unavailable",
            )
        })?;
    let mut force = force_record.decode_payload::<ForceStateRecord>()?;
    let mut force_changed = false;
    if operation.phase == OperationPhase::Moving && context.at >= operation.due_at {
        force_changed = true;
        if force.supply_per_mille < 100 {
            force.status = ForceStatus::Routing;
            force.active_operation = None;
            operation.phase = OperationPhase::Failed;
        } else {
            force.supply_per_mille = force.supply_per_mille.saturating_sub(100);
            force.fatigue_per_mille = force.fatigue_per_mille.saturating_add(20).min(1_000);
            force.morale_per_mille = force.morale_per_mille.saturating_sub(10);
        }
        if operation.phase != OperationPhase::Failed {
            force.location = operation.destination.clone();
            force.status = if operation.opposing_force.is_some() {
                ForceStatus::Engaged
            } else {
                ForceStatus::Ready
            };
        }
        if operation.phase != OperationPhase::Failed && operation.opposing_force.is_none() {
            force.active_operation = None;
            operation.phase = OperationPhase::Completed;
        } else if operation.phase != OperationPhase::Failed {
            operation.phase = OperationPhase::Engaged;
            let defender = operation.opposing_force.clone().expect("checked above");
            let combat_id = CombatId::new(format!("canwu.military:combat:{}", operation.id))?;
            if view
                .typed_domain_record(&combat_reference(&combat_id))?
                .is_none()
            {
                let combat = CombatState {
                    meta: MilitaryRecordMeta::new(1, context.at, &())?,
                    id: combat_id.clone(),
                    operation: operation.id.clone(),
                    location: operation.destination.clone(),
                    attacker: force_id.clone(),
                    defender,
                    stage: CombatStage::Contact,
                    round: 0,
                    attacker_tactic: "screen-and-advance".to_owned(),
                    defender_tactic: "hold".to_owned(),
                    attacker_preparation_per_mille: 500,
                    defender_preparation_per_mille: 500,
                    attacker_visible_strength: force.actual_strength,
                    defender_visible_strength: 0,
                    attacker_casualties: 0,
                    defender_casualties: 0,
                    attacker_prisoners: 0,
                    defender_prisoners: 0,
                    result: None,
                    random_envelopes: Vec::new(),
                    causal_notes: vec!["contact confirmed at destination".to_owned()],
                };
                create(
                    out,
                    combat_reference(&combat_id),
                    &combat,
                    "Create military contact",
                )?;
            }
            schedule_tick(
                out,
                SimDuration::days(1),
                Some(operation.id.clone()),
                None,
                operation.key.clone(),
            )?;
        }
    } else if operation.phase == OperationPhase::Engaged {
        resolve_combat_round(view, context, out, &mut operation)?;
    }
    if force_changed {
        force.meta.revision = force_record.version + 1;
        force.meta.established_at = context.at;
        force.meta.semantic_digest = digest(&force)?;
        force.validate()?;
        upsert(
            view,
            out,
            force_reference(&force_id),
            &force,
            "Advance military force",
        )?;
    }
    operation.meta.revision = record.version + 1;
    operation.meta.established_at = context.at;
    operation.meta.semantic_digest = digest(&operation)?;
    upsert(
        view,
        out,
        reference,
        &operation,
        "Advance military operation",
    )?;
    Ok(())
}

fn resolve_combat_round(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    out: &mut Vec<BoundaryDirective>,
    operation: &mut OperationState,
) -> Result<(), CanwuError> {
    let combat_id = CombatId::new(format!("canwu.military:combat:{}", operation.id))?;
    let combat_reference = combat_reference(&combat_id);
    let combat_record = view
        .typed_domain_record(&combat_reference)?
        .ok_or_else(|| err(ErrorCode::DomainRecordNotFound, "combat is unavailable"))?;
    let mut combat = combat_record.decode_payload::<CombatStateRecord>()?;
    let attacker_record = view
        .typed_domain_record(&force_reference(&combat.attacker))?
        .ok_or_else(|| err(ErrorCode::DomainRecordNotFound, "attacker is unavailable"))?;
    let defender_record = view
        .typed_domain_record(&force_reference(&combat.defender))?
        .ok_or_else(|| err(ErrorCode::DomainRecordNotFound, "defender is unavailable"))?;
    let mut attacker = attacker_record.decode_payload::<ForceStateRecord>()?;
    let mut defender = defender_record.decode_payload::<ForceStateRecord>()?;
    let sample = view.random_range_for_operation(
        &military_random_stream(),
        context
            .admitted_ingress
            .first()
            .copied()
            .map(EvidenceRef::Ingress)
            .unwrap_or(EvidenceRef::Boundary(context.boundary_id)),
        "combat-round",
        operation.key.as_str(),
        RandomOperationTarget::DomainRecord {
            record: combat_reference.clone().into_untyped(),
            version: combat_record.version,
        },
        u32::from(combat.round),
        1_000,
        "combat-round-remainder",
    )?;
    let attacker_loss = (defender.actual_strength / 20).max(1) + (sample % 8) as u32;
    let defender_loss = (attacker.actual_strength / 18).max(1) + ((999 - sample) % 8) as u32;
    let attacker_loss = attacker_loss.min(attacker.actual_strength);
    let defender_loss = defender_loss.min(defender.actual_strength);
    attacker.actual_strength -= attacker_loss;
    attacker.casualties = attacker.casualties.saturating_add(attacker_loss);
    attacker.morale_per_mille = attacker
        .morale_per_mille
        .saturating_sub(u16::try_from(attacker_loss / 2).unwrap_or(u16::MAX));
    attacker.fatigue_per_mille = attacker.fatigue_per_mille.saturating_add(35).min(1_000);
    defender.actual_strength -= defender_loss;
    defender.casualties = defender.casualties.saturating_add(defender_loss);
    defender.morale_per_mille = defender
        .morale_per_mille
        .saturating_sub(u16::try_from(defender_loss / 2).unwrap_or(u16::MAX));
    defender.fatigue_per_mille = defender.fatigue_per_mille.saturating_add(35).min(1_000);
    let terminal =
        attacker.actual_strength == 0 || defender.actual_strength == 0 || combat.round >= 3;
    if terminal {
        combat.result = Some(if defender.actual_strength == 0 {
            CombatResult::AttackerVictory
        } else if attacker.actual_strength == 0 {
            CombatResult::DefenderVictory
        } else {
            CombatResult::MutualDisengagement
        });
        combat.stage = CombatStage::Closed;
        operation.phase = if matches!(combat.result, Some(CombatResult::AttackerVictory)) {
            OperationPhase::Completed
        } else {
            OperationPhase::Withdrawing
        };
        attacker.status = if attacker.actual_strength == 0 {
            ForceStatus::Routing
        } else {
            ForceStatus::Ready
        };
        defender.status = if defender.actual_strength == 0 {
            ForceStatus::Routing
        } else {
            ForceStatus::Ready
        };
        attacker.active_operation = None;
        defender.active_operation = None;
        if matches!(combat.result, Some(CombatResult::AttackerVictory)) {
            let occupation_id = OccupationId::new(format!(
                "canwu.military:occupation:{}",
                operation.id.as_str()
            ))?;
            if view
                .typed_domain_record(&occupation_reference(&occupation_id))?
                .is_none()
            {
                let occupation = OccupationState {
                    meta: MilitaryRecordMeta::new(1, context.at, &())?,
                    id: occupation_id.clone(),
                    node: operation.destination.clone(),
                    occupying_force: attacker.id.clone(),
                    military_control_per_mille: 700,
                    garrison_strength: attacker.actual_strength / 3,
                    administrative_reach_per_mille: 0,
                    security_per_mille: 500,
                    fiscal_capacity_per_mille: 0,
                    legitimacy_per_mille: 0,
                    collaboration_per_mille: 0,
                    resistance_per_mille: 500,
                    extraction_burden_per_mille: 0,
                    integration: IntegrationStage::MilitaryControl,
                    policy_revision: 1,
                    pending_provider_outcomes: BTreeSet::new(),
                };
                create(
                    out,
                    occupation_reference(&occupation_id),
                    &occupation,
                    "Establish military control after victory",
                )?;
                schedule_tick(
                    out,
                    SimDuration::days(1),
                    None,
                    Some(occupation_id),
                    operation.key.clone(),
                )?;
            }
        }
    } else {
        combat.round = combat.round.saturating_add(1);
        combat.stage = CombatStage::RoundResolved;
        schedule_tick(
            out,
            SimDuration::days(1),
            Some(operation.id.clone()),
            None,
            operation.key.clone(),
        )?;
    }
    combat.attacker_casualties = combat.attacker_casualties.saturating_add(attacker_loss);
    combat.defender_casualties = combat.defender_casualties.saturating_add(defender_loss);
    combat.random_envelopes.push(RandomEnvelope {
        purpose: "combat-round-remainder".to_owned(),
        input_digest: input_digest(&(
            attacker_record.version,
            defender_record.version,
            combat.round,
        ))?,
        ruleset_hash: "synthetic-runtime-v1".to_owned(),
        boundary_id: context.boundary_id.to_string(),
        draw_slot: u32::from(combat.round),
        native_value: sample,
        upper_exclusive: 1_000,
    });
    combat.meta.revision = combat_record.version + 1;
    combat.meta.established_at = context.at;
    combat.meta.semantic_digest = digest(&combat)?;
    upsert(view, out, combat_reference, &combat, "Resolve combat round")?;
    attacker.meta.revision = attacker_record.version + 1;
    attacker.meta.established_at = context.at;
    attacker.meta.semantic_digest = digest(&attacker)?;
    defender.meta.revision = defender_record.version + 1;
    defender.meta.established_at = context.at;
    defender.meta.semantic_digest = digest(&defender)?;
    upsert(
        view,
        out,
        force_reference(&attacker.id),
        &attacker,
        "Apply attacker combat result",
    )?;
    upsert(
        view,
        out,
        force_reference(&defender.id),
        &defender,
        "Apply defender combat result",
    )?;
    Ok(())
}

fn advance_occupation_state(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    out: &mut Vec<BoundaryDirective>,
    id: &OccupationId,
) -> Result<(), CanwuError> {
    let reference = occupation_reference(id);
    let record = view
        .typed_domain_record(&reference)?
        .ok_or_else(|| err(ErrorCode::DomainRecordNotFound, "occupation unavailable"))?;
    let mut occupation = record.decode_payload::<OccupationStateRecord>()?;
    occupation.security_per_mille = occupation.security_per_mille.saturating_add(25).min(1_000);
    occupation.administrative_reach_per_mille = occupation
        .administrative_reach_per_mille
        .saturating_add(20)
        .min(1_000);
    occupation.resistance_per_mille = occupation.resistance_per_mille.saturating_sub(10);
    occupation.collaboration_per_mille = occupation
        .collaboration_per_mille
        .saturating_add(10)
        .min(1_000);
    occupation.integration = match occupation.integration {
        IntegrationStage::MilitaryControl
            if occupation.administrative_reach_per_mille >= 300
                && occupation.security_per_mille >= 600 =>
        {
            IntegrationStage::AdministrativeTakeover
        }
        IntegrationStage::AdministrativeTakeover
            if occupation.administrative_reach_per_mille >= 500
                && occupation.security_per_mille >= 650 =>
        {
            IntegrationStage::LegalRecognition
        }
        IntegrationStage::LegalRecognition
            if occupation.administrative_reach_per_mille >= 650
                && occupation.security_per_mille >= 700 =>
        {
            IntegrationStage::FiscalIntegration
        }
        IntegrationStage::FiscalIntegration
            if occupation.administrative_reach_per_mille >= 800
                && occupation.security_per_mille >= 750 =>
        {
            IntegrationStage::SocialIntegration
        }
        IntegrationStage::SocialIntegration
            if occupation.administrative_reach_per_mille >= 900
                && occupation.security_per_mille >= 800 =>
        {
            IntegrationStage::CulturalPractice
        }
        IntegrationStage::CulturalPractice
            if occupation.administrative_reach_per_mille >= 1_000
                && occupation.security_per_mille >= 850 =>
        {
            IntegrationStage::Intergenerational
        }
        stage => stage,
    };
    occupation.meta.revision = record.version + 1;
    occupation.meta.established_at = context.at;
    occupation.meta.semantic_digest = digest(&occupation)?;
    upsert(
        view,
        out,
        reference,
        &occupation,
        "Advance occupation administration",
    )?;
    if occupation.integration != IntegrationStage::Intergenerational {
        schedule_tick(
            out,
            SimDuration::days(1),
            None,
            Some(id.clone()),
            MilitaryOperationKey::new(format!("canwu.military:occupation-tick:{}", id.as_str()))?,
        )?;
    }
    Ok(())
}

fn materialize_reports(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut directives = Vec::new();
    let kind = DomainRecordKind::for_type::<ForceStateRecord>();
    for record in view.domain_records_of_kind(&kind, 256)? {
        let force = record.decode_payload::<ForceStateRecord>()?;
        let Some(commander) = force.commander else {
            continue;
        };
        let report = serde_json::json!({
            "force": force.id,
            "location": force.location,
            "strength_low": force.actual_strength.saturating_sub(force.actual_strength / 10),
            "strength_high": force.actual_strength.saturating_add(force.actual_strength / 10),
            "supply_low": force.supply_per_mille.saturating_sub(100),
            "supply_high": force.supply_per_mille.saturating_add(100).min(1_000),
            "observed_at": context.at,
        });
        let source_version = view
            .current_domain_record_version(&record.reference)?
            .ok_or_else(|| {
                err(
                    ErrorCode::DomainRecordNotFound,
                    "force evidence is unavailable",
                )
            })?;
        directives.push(BoundaryDirective::PublishKnowledge {
            holder: KnowledgeHolderRef::Person(commander),
            visibility: StateVisibility::SameBoundary,
            producer_correlation: Some(format!("military-report:{}:{}", force.id, record.version)),
            records: vec![KnowledgeRecordDraft {
                schema: report_schema_id(),
                subjects: vec![KnowledgeSubject {
                    role: "force".to_owned(),
                    target: KnowledgeSubjectTarget::DomainRecord(record.reference.clone()),
                }],
                payload: report,
                as_of: Some(context.at),
                confidence_per_mille: 900,
                origin: KnowledgeOrigin {
                    method: "military-command-report-v1".to_owned(),
                    evidence: vec![EvidenceRef::DomainRecordVersion(source_version)],
                },
                supersedes: Vec::new(),
                contradicts: Vec::new(),
            }],
            summary: "Publish actor-relative military force report".to_owned(),
        });
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}
