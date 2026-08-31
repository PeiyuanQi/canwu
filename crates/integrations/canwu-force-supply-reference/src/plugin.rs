use crate::{
    ExternalityOutcomePacketV1, ForceCommandEnvelopeV1, ForceOperationOutcomeV1,
    ForceSupplyRuntimeRecord, ForceSupplyStateV1, PLUGIN_NAME, PLUGIN_NAMESPACE,
    ResourceOutcomePacketV1, force_externality_outcome_reference, force_supply_runtime_reference,
    invalid,
};
use canwu_api::{
    ArchiveReachabilityManifest, BoundaryContext, BoundaryDirective, BoundaryPhase,
    BoundaryProposal, BoundarySystemContract, Canwu, CanwuError, Command, CommandContext,
    CommandIngress, DecisionAction, DecisionOrigin, DecisionTicketState, DomainRecord,
    DomainRecordMutation, DomainRecordSchema, ErrorCode, IngressClass, IngressPayload, Issuer,
    KnowledgeHolderRef, PayloadSchema, PluginActionDescriptor, PluginArchiveObjectProvider,
    PluginArchiveRetention, PluginIngressDescriptor, PluginIngressPermit, PluginIngressRequest,
    PluginRegistrar, SimDuration, SimulationPlugin, SimulationView, StateKey, StateVisibility,
    SystemCadence, SystemDirective, canonical_hash,
};
use canwu_resource::{
    ResourceOperationOutcomeVersionV1, ResourceRuntimeRecord, resource_runtime_reference,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::OnceLock;

pub const FORCE_SUPPLY_COMMAND: &str = "apply_force_supply_operation_v1";
const FORCE_OPERATION_INGRESS: &str = "force_supply_operation_v1";
pub const FORCE_RESOURCE_OUTCOME_INGRESS: &str = "force_resource_outcome_v1";
pub const FORCE_EXTERNALITY_OUTCOME_INGRESS: &str = "force_externality_outcome_v1";
pub const FORCE_ARCHIVE_COMMIT_INGRESS: &str = "force_archive_commit_v1";
pub const FORCE_ARCHIVE_RETENTION_ACK_INGRESS: &str = "force_archive_retention_ack_v1";
const APPLY_SYSTEM: &str = "force_supply_lifecycle_apply_v1";
const RESOURCE_DISPATCH_SYSTEM: &str = "force_supply_resource_dispatch_v1";
const DUE_EVALUATION_SYSTEM: &str = "force_supply_due_evaluation_v1";
const VALIDATE_SYSTEM: &str = "force_supply_lifecycle_validate_v1";
pub const FORCE_SUPPLY_SEMANTIC_HASH: &str =
    "6c5239c3bb16c2a2c33194907e2bd398afb4f29ed286233fd876355a60960c8d";
static FORCE_ARCHIVE_PERMIT: OnceLock<PluginIngressPermit> = OnceLock::new();
static FORCE_ARCHIVE_ACK_PERMIT: OnceLock<PluginIngressPermit> = OnceLock::new();

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ForceArchiveIngressReceiptV1 {
    pub ingress: canwu_api::IngressReceipt,
    pub retention_handle_id: String,
    pub directory_root: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct ForceArchiveRetentionAcknowledgementV1 {
    receipt: crate::ForceArchiveMaintenanceReceiptV1,
}

pub fn force_supply_command(value: &ForceCommandEnvelopeV1) -> Result<Command, serde_json::Error> {
    Ok(Command::Plugin {
        plugin: PLUGIN_NAME.to_owned(),
        command: FORCE_SUPPLY_COMMAND.to_owned(),
        payload: serde_json::to_value(value)?,
    })
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct AdmittedForceOperationV1 {
    envelope: ForceCommandEnvelopeV1,
    input_digest: String,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ForceSupplyReferencePlugin;

fn force_archive_reachability(
    view: &SimulationView<'_>,
    provider: &dyn PluginArchiveObjectProvider,
    manifest: &mut ArchiveReachabilityManifest,
) -> Result<(), CanwuError> {
    let Some((_, state)) = load_state(view)? else {
        return Ok(());
    };
    let roots = state.archive_head.directory_root.iter().cloned().chain(
        state
            .archive_retention_handles
            .values()
            .map(|handle| handle.directory_root.clone()),
    );
    crate::extend_archive_reachability::<crate::ForceArchiveKeyV1, crate::ForceArchivePayloadV1>(
        crate::FORCE_ARCHIVE_DOMAIN,
        roots,
        provider,
        manifest,
    )
}

#[must_use]
pub fn force_supply_command_descriptor() -> PluginActionDescriptor {
    PluginActionDescriptor {
        name: FORCE_SUPPLY_COMMAND.to_owned(),
        description: "Submit one authority-checked force-supply lifecycle operation".to_owned(),
        payload_schema: PayloadSchema::Any,
        reads: vec![StateKey::new(PLUGIN_NAMESPACE, "runtime")],
        writes: Vec::new(),
    }
}

#[must_use]
pub fn force_resource_outcome_ingress_descriptor() -> PluginIngressDescriptor {
    PluginIngressDescriptor {
        name: FORCE_RESOURCE_OUTCOME_INGRESS.to_owned(),
        description: "Acknowledge one exact terminal resource-consumption outcome".to_owned(),
        class: IngressClass::Acknowledgement,
        payload_schema: PayloadSchema::Any,
    }
}

#[must_use]
pub fn force_externality_outcome_ingress_descriptor() -> PluginIngressDescriptor {
    PluginIngressDescriptor {
        name: FORCE_EXTERNALITY_OUTCOME_INGRESS.to_owned(),
        description: "Acknowledge one exact economy-owned requisition externality outcome"
            .to_owned(),
        class: IngressClass::Acknowledgement,
        payload_schema: PayloadSchema::Any,
    }
}

impl SimulationPlugin for ForceSupplyReferencePlugin {
    fn name(&self) -> &'static str {
        PLUGIN_NAME
    }
    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }
    fn semantic_hash(&self) -> &'static str {
        FORCE_SUPPLY_SEMANTIC_HASH
    }

    fn validate_activation(&self, records: &[DomainRecord]) -> Result<(), CanwuError> {
        let mut found = false;
        for record in records.iter().filter(|record| {
            record
                .reference
                .kind
                .matches_type::<ForceSupplyRuntimeRecord>()
        }) {
            if found || record.reference != force_supply_runtime_reference().into_untyped() {
                return Err(invalid(
                    "force-supply activation contains multiple or misidentified runtime roots",
                ));
            }
            let state = record.decode_payload::<ForceSupplyRuntimeRecord>()?;
            state.validate()?;
            if state.provider_record_version != record.version
                || state.draft()?.payload != record.payload
            {
                return Err(invalid(
                    "force-supply activation root version or canonical encoding differs",
                ));
            }
            found = true;
        }
        Ok(())
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_archive_reachability_participant(force_archive_reachability)?;
        registrar
            .register_record_schema(DomainRecordSchema::for_record::<ForceSupplyRuntimeRecord>())?;
        registrar.register_command(force_supply_command_descriptor(), admit_force_operation)?;
        registrar.register_ingress(PluginIngressDescriptor {
            name: FORCE_OPERATION_INGRESS.to_owned(),
            description: "Apply one admitted force-supply operation".to_owned(),
            class: IngressClass::Decision,
            payload_schema: PayloadSchema::Any,
        })?;
        registrar.register_ingress(force_resource_outcome_ingress_descriptor())?;
        registrar.register_ingress(force_externality_outcome_ingress_descriptor())?;
        let archive_permit = registrar.register_internal_ingress_with_archive_retention(
            PluginIngressDescriptor {
                name: FORCE_ARCHIVE_COMMIT_INGRESS.to_owned(),
                description: "Commit one verified force terminal archive batch".to_owned(),
                class: IngressClass::ScheduledSystem,
                payload_schema: PayloadSchema::Any,
            },
            crate::FORCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE,
            vec![
                "/directory_root".to_owned(),
                "/retention/directory_root".to_owned(),
            ],
        )?;
        let _ = FORCE_ARCHIVE_PERMIT.set(archive_permit);
        let ack_permit = registrar.register_internal_ingress(PluginIngressDescriptor {
            name: FORCE_ARCHIVE_RETENTION_ACK_INGRESS.to_owned(),
            description: "Acknowledge force archive retention finalization".to_owned(),
            class: IngressClass::Acknowledgement,
            payload_schema: PayloadSchema::Any,
        })?;
        let _ = FORCE_ARCHIVE_ACK_PERMIT.set(ack_permit);

        let mut apply = BoundarySystemContract::new(
            APPLY_SYSTEM,
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        apply.reads = vec![
            StateKey::core_commands(),
            StateKey::core_ingress(),
            StateKey::core_decisions(),
            StateKey::new(PLUGIN_NAMESPACE, "runtime"),
            StateKey::new("canwu.resource", "runtime"),
            StateKey::new(PLUGIN_NAMESPACE, "externality-completion-participant"),
            StateKey::new(PLUGIN_NAMESPACE, "externality-outcome"),
        ];
        apply.writes = vec![StateKey::new(PLUGIN_NAMESPACE, "runtime")];
        apply.emits = vec![
            "canwu.force-supply.operation_applied.v1".to_owned(),
            "canwu.force-supply.operation_rejected.v1".to_owned(),
            "canwu.force-supply.resource_consequence.v1".to_owned(),
            "canwu.force-supply.externality_acknowledged.v1".to_owned(),
        ];
        apply.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(apply, apply_force_ingress)?;

        let mut resource_dispatch = BoundarySystemContract::new(
            RESOURCE_DISPATCH_SYSTEM,
            BoundaryPhase::StrategicAggregation,
            SystemCadence::EventDriven,
        );
        resource_dispatch.reads = vec![
            StateKey::new(PLUGIN_NAMESPACE, "runtime"),
            StateKey::new("canwu.resource", "runtime"),
            StateKey::core_ingress(),
        ];
        resource_dispatch.plugin_ingress_targets = vec![canwu_api::PluginIngressTarget {
            target_plugin: canwu_resource::PLUGIN_NAME.to_owned(),
            packet_type: canwu_resource::RESOURCE_ADAPTER_INGRESS.to_owned(),
        }];
        resource_dispatch.visibility = StateVisibility::SameBoundary;
        registrar
            .register_boundary_system(resource_dispatch, dispatch_pending_resource_consumptions)?;

        let mut due_evaluation = BoundarySystemContract::new(
            DUE_EVALUATION_SYSTEM,
            BoundaryPhase::StrategicAggregation,
            SystemCadence::EventDriven,
        );
        due_evaluation.reads = vec![StateKey::new(PLUGIN_NAMESPACE, "runtime")];
        due_evaluation.emits = vec!["canwu.force-supply.requirement_due.v1".to_owned()];
        due_evaluation.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(due_evaluation, evaluate_due_requirements)?;

        let mut validate = BoundarySystemContract::new(
            VALIDATE_SYSTEM,
            BoundaryPhase::InvariantValidation,
            SystemCadence::EventDriven,
        );
        validate.reads = vec![StateKey::new(PLUGIN_NAMESPACE, "runtime")];
        validate.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(validate, validate_force_candidate)
    }
}

fn evaluate_due_requirements(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let Some((_, state)) = load_state(view)? else {
        return Ok(BoundaryProposal::default());
    };
    let directives = state
        .due_requirements(context.at, crate::MAX_DUE_CANDIDATES_PER_TICK)?
        .into_iter()
        .map(
            |(force, requirement, scheduled_due, due_count, requested_quantity)| {
                BoundaryDirective::Emit {
                    event_type: "canwu.force-supply.requirement_due.v1".to_owned(),
                    summary: format!(
                        "force {force} requirement {requirement} is due from {scheduled_due} for {due_count} cycle(s), requesting {requested_quantity} units"
                    ),
                    affected: Vec::new(),
                }
            },
        )
        .collect();
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

fn dispatch_pending_resource_consumptions(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let Some(record) = view.typed_domain_record(&force_supply_runtime_reference())? else {
        return Ok(BoundaryProposal::default());
    };
    let state = record.decode_payload::<ForceSupplyRuntimeRecord>()?;
    state.validate()?;
    let resource_outcomes = view
        .typed_domain_record(&resource_runtime_reference())?
        .map(DomainRecord::decode_payload::<ResourceRuntimeRecord>)
        .transpose()?;
    let in_flight = context
        .admitted_ingress
        .iter()
        .filter_map(|id| view.ingress(*id).ok().flatten())
        .filter_map(|ingress| match &ingress.payload {
            canwu_api::IngressPayload::Plugin {
                plugin,
                packet_type,
                payload,
                ..
            } if plugin == canwu_resource::PLUGIN_NAME
                && packet_type == canwu_resource::RESOURCE_ADAPTER_INGRESS =>
            {
                serde_json::from_value::<canwu_resource::ResourceAdapterOperationV1>(
                    payload.clone(),
                )
                .ok()
            }
            _ => None,
        })
        .map(|packet| packet.request.operation_key())
        .collect::<std::collections::BTreeSet<_>>();
    let current_source = view
        .current_domain_record_version(&force_supply_runtime_reference().into_untyped())?
        .ok_or_else(|| invalid("force consumption provider source version is unavailable"))?;
    let directives = state
        .intents
        .values()
        .filter(|intent| {
            intent.status == crate::ForceConsumptionIntentStatus::PendingResourceConsumption
                && intent.resource_outcome.is_none()
                && resource_outcomes.as_ref().is_none_or(|resource| {
                    !resource
                        .outcomes
                        .contains_key(&intent.resource_operation_key)
                })
                && !in_flight.contains(&intent.resource_operation_key)
        })
        .map(|intent| {
            let consumer_evidence = intent
                .completion_certificate
                .locked_target_versions
                .iter()
                .find_map(|target| match target {
                    canwu_resource::CompletionLockedTargetV1::ExternalRecord { version }
                        if version.record == force_supply_runtime_reference().into_untyped() =>
                    {
                        Some(version.clone())
                    }
                    _ => None,
                })
                .ok_or_else(|| {
                    invalid("force consumption certificate lacks its provider evidence")
                })?;
            let mut request = crate::resource_consumption_request(intent, current_source.clone());
            request.consumer_evidence = consumer_evidence;
            let packet = canwu_resource::ResourceAdapterOperationV1 {
                provider_plugin: PLUGIN_NAME.to_owned(),
                provider_source: current_source.clone(),
                request: canwu_resource::ResourceOperationRequestV1::Consume(request),
            };
            Ok(BoundaryDirective::SchedulePluginIngress {
                target_plugin: canwu_resource::PLUGIN_NAME.to_owned(),
                after: SimDuration::ZERO,
                packet_type: canwu_resource::RESOURCE_ADAPTER_INGRESS.to_owned(),
                priority: 0,
                payload: serde_json::to_value(packet).map_err(crate::encode_error)?,
                affected: Vec::new(),
            })
        })
        .collect::<Result<Vec<_>, CanwuError>>()?;
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

fn admit_force_operation(
    view: &SimulationView<'_>,
    context: &CommandContext,
    payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    if context.ingress == CommandIngress::LegacyDirect {
        return Err(CanwuError::new(
            ErrorCode::MixedCommandIngress,
            "force-supply operations require tracked canonical command ingress",
        ));
    }
    let envelope: ForceCommandEnvelopeV1 = decode(payload, "force-supply operation")?;
    require_holder_authority(context, &envelope.holder)?;
    let (_, state) = load_state(view)?.ok_or_else(|| {
        CanwuError::new(
            ErrorCode::DomainRecordNotFound,
            "force-supply runtime is not configured",
        )
    })?;
    let input_digest = canonical_hash("canwu.force-supply.operation-input.v1", &envelope)?;
    if let Some(existing) = state.outcomes.get(&envelope.operation_id) {
        if existing.input_digest == input_digest {
            return Ok(Vec::new());
        }
        return Err(CanwuError::new(
            ErrorCode::IdempotencyConflict,
            "force operation ID was reused with different input",
        ));
    }
    if let Some(record) = archived_force_record(
        view,
        &state,
        &crate::ForceArchiveKeyV1::OperationOutcome(envelope.operation_id.clone()),
    )? {
        let crate::ForceArchivePayloadV1::OperationOutcome(existing) = record.payload else {
            return Err(invalid(
                "force archive operation membership has the wrong payload",
            ));
        };
        if existing.input_digest == input_digest {
            return Ok(Vec::new());
        }
        return Err(CanwuError::new(
            ErrorCode::IdempotencyConflict,
            "archived force operation ID was reused with different input",
        ));
    }
    if let crate::ForceOperationV1::SubmitConsumptionIntent { intent } = &envelope.operation
        && (archived_force_record(
            view,
            &state,
            &crate::ForceArchiveKeyV1::TerminalIntent(intent.id.clone()),
        )?
        .is_some()
            || archived_force_record(
                view,
                &state,
                &crate::ForceArchiveKeyV1::TerminalOperation(intent.resource_operation_key.clone()),
            )?
            .is_some())
    {
        return Err(CanwuError::new(
            ErrorCode::IdempotencyConflict,
            "force intent identity or resource operation key is already archived",
        ));
    }
    Ok(vec![SystemDirective::EnqueuePluginIngress {
        after: SimDuration::ZERO,
        packet_type: FORCE_OPERATION_INGRESS.to_owned(),
        priority: 0,
        payload: serde_json::to_value(AdmittedForceOperationV1 {
            envelope,
            input_digest,
        })
        .map_err(crate::encode_error)?,
        affected: Vec::new(),
    }])
}

#[allow(clippy::too_many_lines)]
fn apply_force_ingress(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let Some((record, mut state)) = load_state(view)? else {
        return Ok(BoundaryProposal::default());
    };
    let mut changed = false;
    let mut directives = Vec::new();
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
            FORCE_ARCHIVE_COMMIT_INGRESS => {
                let commit: crate::VerifiedForceArchiveCommitV1 =
                    decode(payload, "verified force archive commit")?;
                state.apply_force_archive_commit(&commit)?;
                changed = true;
            }
            FORCE_ARCHIVE_RETENTION_ACK_INGRESS => {
                let acknowledgement: ForceArchiveRetentionAcknowledgementV1 =
                    decode(payload, "force archive retention acknowledgement")?;
                let persisted = state
                    .archive_maintenance_receipts
                    .get(&acknowledgement.receipt.sequence)
                    .ok_or_else(|| invalid("force archive maintenance receipt is unavailable"))?;
                if persisted != &acknowledgement.receipt {
                    return Err(invalid("force archive acknowledgement is forged"));
                }
                state
                    .archive_retention_handles
                    .remove(&acknowledgement.receipt.retention_handle_id);
                changed = true;
            }
            FORCE_OPERATION_INGRESS => {
                let admitted: AdmittedForceOperationV1 =
                    decode(payload, "admitted force operation")?;
                if state.outcomes.contains_key(&admitted.envelope.operation_id) {
                    continue;
                }
                if let Some(record) = archived_force_record(
                    view,
                    &state,
                    &crate::ForceArchiveKeyV1::OperationOutcome(
                        admitted.envelope.operation_id.clone(),
                    ),
                )? {
                    let crate::ForceArchivePayloadV1::OperationOutcome(existing) = record.payload
                    else {
                        return Err(invalid(
                            "force archive operation membership has the wrong payload",
                        ));
                    };
                    if existing.input_digest != admitted.input_digest {
                        return Err(CanwuError::new(
                            ErrorCode::IdempotencyConflict,
                            "archived force operation ID was reused with different input",
                        ));
                    }
                    continue;
                }
                let mut candidate = state.clone();
                let result = match &admitted.envelope.operation {
                    crate::ForceOperationV1::SelectSupplyPosture { decision, .. } => {
                        validate_force_decision_command(view, &admitted.envelope, decision)
                            .and_then(|()| {
                                candidate.apply_operation(&admitted.envelope, context.at)
                            })
                    }
                    crate::ForceOperationV1::RecordSupplyObservation { force, observation } => {
                        validate_force_observation_source(view, &state, force, observation)
                            .and_then(|()| {
                                candidate.apply_operation(&admitted.envelope, context.at)
                            })
                    }
                    crate::ForceOperationV1::Completion {
                        operation:
                            crate::ForceCompletionOperationV1::AcknowledgeExternalParticipant {
                                owner_source,
                                participant,
                            },
                    } => validate_completion_participant_source(view, owner_source, participant)
                        .and_then(|()| candidate.apply_operation(&admitted.envelope, context.at)),
                    _ => candidate.apply_operation(&admitted.envelope, context.at),
                };
                let (applied, rejection_code, rejection_reason) = match result {
                    Ok(()) => {
                        state = candidate;
                        (true, None, None)
                    }
                    Err(error) if expected_domain_rejection(&error) => {
                        state.revision = state.revision.checked_add(1).ok_or_else(|| {
                            invalid("force-supply revision overflowed while rejecting an operation")
                        })?;
                        (
                            false,
                            Some(error_code(&error.code).to_owned()),
                            Some(error.message),
                        )
                    }
                    Err(error) => return Err(error),
                };
                state.outcomes.insert(
                    admitted.envelope.operation_id.clone(),
                    ForceOperationOutcomeV1 {
                        id: admitted.envelope.operation_id.clone(),
                        input_digest: admitted.input_digest,
                        applied,
                        rejection_code,
                        rejection_reason,
                        settled_at: context.at,
                    },
                );
                state.validate()?;
                changed = true;
                directives.push(BoundaryDirective::Emit {
                    event_type: if applied {
                        "canwu.force-supply.operation_applied.v1"
                    } else {
                        "canwu.force-supply.operation_rejected.v1"
                    }
                    .to_owned(),
                    summary: format!(
                        "force-supply operation {} reached a terminal outcome",
                        admitted.envelope.operation_id
                    ),
                    affected: Vec::new(),
                });
            }
            FORCE_RESOURCE_OUTCOME_INGRESS => {
                let packet: ResourceOutcomePacketV1 = decode(payload, "force resource outcome")?;
                if !state.intents.contains_key(&packet.intent)
                    && !state
                        .terminal_receipts
                        .values()
                        .any(|receipt| receipt.intent == packet.intent)
                {
                    authenticate_archived_resource_outcome(view, &state, &packet)?;
                    continue;
                }
                let (authoritative, participant) = resolve_resource_outcome(view, &state, &packet)?;
                let acquisition = participant.grant.acquisition.clone();
                let holder = state.completion_leases.acquisitions[&acquisition]
                    .holder
                    .clone();
                state.acknowledge_external_participant(&holder, participant)?;
                state.acknowledge_resource_outcome(&packet, &authoritative, context.at)?;
                changed = true;
                directives.push(BoundaryDirective::Emit {
                    event_type: "canwu.force-supply.resource_consequence.v1".to_owned(),
                    summary: format!(
                        "force intent {} committed its resource-linked consequence",
                        packet.intent
                    ),
                    affected: Vec::new(),
                });
            }
            FORCE_EXTERNALITY_OUTCOME_INGRESS => {
                let packet: ExternalityOutcomePacketV1 =
                    decode(payload, "force externality outcome")?;
                if !state.sagas.contains_key(&packet.saga)
                    && !state
                        .terminal_receipts
                        .values()
                        .any(|receipt| receipt.saga.as_ref() == Some(&packet.saga))
                {
                    authenticate_archived_externality_outcome(view, &state, &packet)?;
                    continue;
                }
                let (authoritative, participant) =
                    resolve_externality_outcome(view, &state, &packet)?;
                let acquisition = participant.grant.acquisition.clone();
                let holder = state.completion_leases.acquisitions[&acquisition]
                    .holder
                    .clone();
                state.acknowledge_external_participant(&holder, participant)?;
                state.acknowledge_externality_outcome(&packet, &authoritative, context.at)?;
                changed = true;
                directives.push(BoundaryDirective::Emit {
                    event_type: "canwu.force-supply.externality_acknowledged.v1".to_owned(),
                    summary: format!(
                        "requisition saga {} acknowledged its externality outcome",
                        packet.saga
                    ),
                    affected: Vec::new(),
                });
            }
            _ => {}
        }
    }
    if changed {
        state.provider_record_version = record
            .version
            .checked_add(1)
            .ok_or_else(|| invalid("force provider record version overflowed"))?;
        state.validate()?;
        directives.insert(
            0,
            BoundaryDirective::MutateRecord {
                mutation: DomainRecordMutation::Update {
                    record: state.draft()?,
                    expected_version: record.version,
                },
                summary: "Apply bounded force-supply lifecycle mutations".to_owned(),
            },
        );
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

fn validate_force_decision_command(
    view: &SimulationView<'_>,
    envelope: &ForceCommandEnvelopeV1,
    selection: &crate::ForceDecisionSelectionV1,
) -> Result<(), CanwuError> {
    let ticket = view
        .decision_ticket(selection.ticket)?
        .ok_or_else(|| invalid("force supply posture decision ticket is unavailable"))?;
    let DecisionTicketState::Resolved { option_id, .. } = &ticket.state else {
        return Err(invalid(
            "force supply posture decision ticket is not resolved",
        ));
    };
    let option = ticket
        .option(&selection.option_id)
        .ok_or_else(|| invalid("force supply posture decision option is unavailable"))?;
    let DecisionAction::Command { command } = &option.action else {
        return Err(invalid(
            "force supply posture decision option has no command",
        ));
    };
    let expected =
        serde_json::to_value(force_supply_command(envelope).map_err(crate::encode_error)?)
            .map_err(crate::encode_error)?;
    if option_id != &selection.option_id || command != &expected {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "force supply posture command is not the exact resolved decision action",
        ));
    }
    Ok(())
}

fn archived_force_record(
    provider: &dyn PluginArchiveObjectProvider,
    state: &ForceSupplyStateV1,
    key: &crate::ForceArchiveKeyV1,
) -> Result<
    Option<crate::PackageArchiveRecordV1<crate::ForceArchiveKeyV1, crate::ForceArchivePayloadV1>>,
    CanwuError,
> {
    crate::load_package_archive_record(
        crate::FORCE_ARCHIVE_DOMAIN,
        &state.archive_head,
        provider,
        key,
    )
}

fn authenticate_archived_resource_outcome(
    view: &SimulationView<'_>,
    state: &ForceSupplyStateV1,
    packet: &ResourceOutcomePacketV1,
) -> Result<(), CanwuError> {
    let record = archived_force_record(
        view,
        state,
        &crate::ForceArchiveKeyV1::TerminalIntent(packet.intent.clone()),
    )?
    .ok_or_else(|| invalid("force resource outcome intent is unavailable"))?;
    let crate::ForceArchivePayloadV1::TerminalLifecycle(lifecycle) = record.payload else {
        return Err(invalid(
            "force terminal intent membership has the wrong payload",
        ));
    };
    let receipt = lifecycle.receipt;
    let provider = view
        .domain_record_version(&packet.authoritative_resource_state)?
        .ok_or_else(|| invalid("archived force resource provider body is unavailable"))?;
    if provider.owner != canwu_resource::PLUGIN_NAME
        || provider.reference != resource_runtime_reference().into_untyped()
    {
        return Err(invalid("archived force resource provider identity differs"));
    }
    let resource = provider.decode_payload::<ResourceRuntimeRecord>()?;
    let exact = resource
        .outcomes
        .get(&receipt.resource_outcome.operation_key)
        .map(ResourceOperationOutcomeVersionV1::from)
        .ok_or_else(|| invalid("archived force resource outcome is unavailable"))?;
    if receipt.resource_outcome_source != packet.authoritative_resource_state
        || receipt.resource_outcome.id != packet.outcome_id
        || receipt.resource_outcome != exact
    {
        return Err(CanwuError::new(
            ErrorCode::IdempotencyConflict,
            "archived force intent received a different resource outcome",
        ));
    }
    Ok(())
}

fn authenticate_archived_externality_outcome(
    view: &SimulationView<'_>,
    state: &ForceSupplyStateV1,
    packet: &ExternalityOutcomePacketV1,
) -> Result<(), CanwuError> {
    let record = archived_force_record(
        view,
        state,
        &crate::ForceArchiveKeyV1::TerminalSaga(packet.saga.clone()),
    )?
    .ok_or_else(|| invalid("force externality outcome saga is unavailable"))?;
    let crate::ForceArchivePayloadV1::TerminalSagaAlias { intent, .. } = record.payload else {
        return Err(invalid(
            "force terminal saga membership has the wrong payload",
        ));
    };
    let lifecycle = archived_force_record(
        view,
        state,
        &crate::ForceArchiveKeyV1::TerminalIntent(intent),
    )?
    .ok_or_else(|| invalid("force archived saga lost its lifecycle"))?;
    let crate::ForceArchivePayloadV1::TerminalLifecycle(lifecycle) = lifecycle.payload else {
        return Err(invalid("force archived saga lifecycle payload differs"));
    };
    let expected = lifecycle
        .receipt
        .externality_outcome
        .ok_or_else(|| invalid("force archived saga has no externality outcome"))?;
    let expected_source = lifecycle
        .receipt
        .externality_outcome_source
        .ok_or_else(|| invalid("force archived saga has no externality provider"))?;
    let provider = view
        .domain_record_version(&packet.authoritative_outcome)?
        .ok_or_else(|| invalid("archived force externality provider body is unavailable"))?;
    let expected_owner = lifecycle
        .external_participants
        .keys()
        .find(|owner| {
            owner.as_str() != PLUGIN_NAME && owner.as_str() != canwu_resource::PLUGIN_NAME
        })
        .ok_or_else(|| invalid("archived force saga lost its externality provider"))?;
    if packet.authoritative_outcome != expected_source
        || provider.owner != *expected_owner
        || provider.reference
            != crate::force_externality_outcome_reference(&expected.id).into_untyped()
        || provider.decode_payload::<crate::ForceExternalityOutcomeProviderRecord>()? != expected
    {
        return Err(CanwuError::new(
            ErrorCode::IdempotencyConflict,
            "archived force saga received a different externality outcome",
        ));
    }
    Ok(())
}

fn validate_force_observation_source(
    view: &SimulationView<'_>,
    state: &ForceSupplyStateV1,
    force_id: &crate::ReferenceForceId,
    observation: &crate::ForceSupplyObservationV1,
) -> Result<(), CanwuError> {
    let force = state
        .forces
        .get(force_id)
        .ok_or_else(|| invalid("force observation target is unavailable"))?;
    let profile = state
        .profiles
        .get(&force.profile)
        .ok_or_else(|| invalid("force observation profile is unavailable"))?;
    let requirement = profile
        .requirements
        .iter()
        .find(|requirement| requirement.id == observation.requirement)
        .ok_or_else(|| invalid("force observation requirement is unavailable"))?;
    let mut authenticated = false;
    for source in &observation.source_versions {
        let record = view
            .domain_record_version(source)?
            .ok_or_else(|| invalid("force observation source body is unavailable"))?;
        let matches_source = match observation.source {
            crate::ForceSupplyObservationSourceV1::ResourceProvider => {
                if record.owner != canwu_resource::PLUGIN_NAME
                    || record.reference != resource_runtime_reference().into_untyped()
                {
                    false
                } else {
                    let resource = record.decode_payload::<ResourceRuntimeRecord>()?;
                    resource.validate().map_err(|error| {
                        invalid(format!(
                            "force resource observation source is invalid: {error}"
                        ))
                    })?;
                    let exact_stock = resource
                        .accounts
                        .values()
                        .filter(|account| {
                            !account.closed
                                && account.custodian == force.holder
                                && account.resource_revision == requirement.resource_revision
                                && account.unit_revision == requirement.unit_revision
                        })
                        .try_fold(0_u64, |total, account| {
                            total.checked_add(account.balance).ok_or_else(|| {
                                invalid("force resource observation stock overflowed")
                            })
                        })?;
                    if exact_stock < observation.known_stock_low
                        || exact_stock > observation.known_stock_high
                    {
                        return Err(CanwuError::new(
                            ErrorCode::InvalidAuthority,
                            "force resource observation interval excludes its provider stock",
                        ));
                    }
                    true
                }
            }
            crate::ForceSupplyObservationSourceV1::TransportProvider => false,
            crate::ForceSupplyObservationSourceV1::ForceConsequence => {
                if record.owner != PLUGIN_NAME
                    || record.reference != force_supply_runtime_reference().into_untyped()
                {
                    false
                } else {
                    let provider = record.decode_payload::<ForceSupplyRuntimeRecord>()?;
                    provider.consequences.values().any(|consequence| {
                        consequence.force == *force_id
                            && consequence.attribution.requirement == observation.requirement
                    })
                }
            }
            crate::ForceSupplyObservationSourceV1::EconomyExternality => {
                record
                    .reference
                    .kind
                    .matches_type::<crate::ForceExternalityOutcomeProviderRecord>()
            }
        };
        authenticated |= matches_source;
    }
    if authenticated {
        Ok(())
    } else {
        Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "force observation has no provider-authenticated source of its declared type",
        ))
    }
}

fn validate_completion_participant_source(
    view: &SimulationView<'_>,
    source: &canwu_api::DomainRecordVersionRef,
    participant: &canwu_resource::ExternalCompletionParticipantGrantV1,
) -> Result<(), CanwuError> {
    let record = view
        .domain_record_version(source)?
        .ok_or_else(|| invalid("completion participant owner source is unavailable"))?;
    if record.owner != participant.grant.owner_plugin {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "completion participant acknowledgement cites another owner",
        ));
    }
    let authoritative = if record.owner == canwu_resource::PLUGIN_NAME {
        if record.reference != resource_runtime_reference().into_untyped() {
            return Err(CanwuError::new(
                ErrorCode::InvalidAuthority,
                "resource completion participant source is not the resource runtime",
            ));
        }
        record
            .decode_payload::<ResourceRuntimeRecord>()?
            .external_completion_participants
            .grants
            .get(&participant.grant.acquisition)
            .cloned()
    } else if record
        .reference
        .kind
        .matches_type::<crate::ForceExternalityCompletionParticipantProviderRecord>()
    {
        let provider = record
            .decode_payload::<crate::ForceExternalityCompletionParticipantProviderRecord>()?;
        let sealed = provider.clone().seal()?;
        if sealed != provider
            || provider.provider_plugin != record.owner
            || record.reference
                != crate::force_externality_completion_participant_reference(
                    &participant.grant.acquisition,
                )
                .into_untyped()
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidAuthority,
                "externality completion participant provider is forged",
            ));
        }
        Some(provider.participant)
    } else {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "completion participant source is not a recognized owner runtime",
        ));
    };
    if authoritative.as_ref() != Some(participant) {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "completion participant acknowledgement differs from the exact owner body",
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn resolve_resource_outcome(
    view: &SimulationView<'_>,
    force_state: &ForceSupplyStateV1,
    packet: &ResourceOutcomePacketV1,
) -> Result<
    (
        crate::ForceResourceSettlementEvidenceV1,
        canwu_resource::ExternalCompletionParticipantGrantV1,
    ),
    CanwuError,
> {
    if packet.authoritative_resource_state.record != resource_runtime_reference().into_untyped() {
        return Err(invalid(
            "force resource acknowledgement does not cite the authoritative resource root",
        ));
    }
    let record = view
        .domain_record_version(&packet.authoritative_resource_state)?
        .ok_or_else(|| invalid("authoritative resource outcome version is not retained"))?;
    if record.owner != "canwu-resource" {
        return Err(invalid(
            "force resource acknowledgement cites a non-resource provider",
        ));
    }
    let resource_state = record.decode_payload::<ResourceRuntimeRecord>()?;
    resource_state
        .validate()
        .map_err(|error| invalid(format!("authoritative resource state is invalid: {error}")))?;
    let intent = force_state
        .intents
        .get(&packet.intent)
        .ok_or_else(|| invalid("resource outcome intent is unavailable"))?;
    let outcome = resource_state
        .outcomes
        .get(&intent.resource_operation_key)
        .ok_or_else(|| invalid("authoritative resource outcome is unavailable"))?;
    if outcome.id != packet.outcome_id {
        return Err(invalid(
            "resource outcome packet ID differs from the authoritative provider outcome",
        ));
    }
    let exact_outcome = ResourceOperationOutcomeVersionV1::from(outcome);
    let consumption = resource_state
        .consumptions
        .get(&intent.consumption_id)
        .ok_or_else(|| invalid("authoritative force resource consumption is unavailable"))?;
    if consumption.operation_key != intent.resource_operation_key
        || consumption.account != intent.stock_custody.destination_account
        || consumption.allocation_leg != intent.allocation.id
    {
        return Err(invalid(
            "authoritative force consumption is not bound to the accepted destination stock",
        ));
    }
    let fulfillment = resource_state
        .fulfillments
        .values()
        .find(|fulfillment| {
            fulfillment.operation_key == intent.resource_operation_key
                && fulfillment.allocation_legs == vec![intent.allocation.id.clone()]
        })
        .ok_or_else(|| invalid("authoritative force resource fulfillment is unavailable"))?;
    let account = resource_state
        .accounts
        .get(&intent.stock_custody.destination_account)
        .ok_or_else(|| invalid("force destination resource account is unavailable"))?;
    if account.custodian != intent.stock_custody.destination_custodian
        || account.resource_revision != intent.allocation.resource_revision
        || account.unit_revision != intent.allocation.unit_revision
    {
        return Err(invalid(
            "force destination account custody or resource identity differs",
        ));
    }
    let accepted_transfer = intent
        .stock_custody
        .accepted_transfer
        .as_ref()
        .map(|expected| {
            let transfer = resource_state
                .transfers
                .get(&expected.transfer)
                .ok_or_else(|| invalid("force accepted transfer is unavailable"))?;
            let transport = transfer
                .transport
                .clone()
                .ok_or_else(|| invalid("force accepted transfer lacks transport custody"))?;
            if transfer.revision != expected.transfer_revision
                || transfer.state != canwu_resource::ResourceTransferState::Accepted
                || transfer.destination.as_ref() != Some(&intent.stock_custody.destination_account)
                || transfer.accepted < consumption.quantity
                || !transfer
                    .exact_evidence
                    .contains(&expected.acceptance_source)
            {
                return Err(invalid(
                    "force supply cannot benefit before exact destination acceptance",
                ));
            }
            let mut evidence = crate::ForceAcceptedTransferEvidenceV1 {
                transfer: transfer.id.clone(),
                transfer_revision: transfer.revision,
                destination: intent.stock_custody.destination_account.clone(),
                accepted_quantity: transfer.accepted,
                transport,
                acceptance_source: expected.acceptance_source.clone(),
                semantic_digest: String::new(),
            };
            evidence.semantic_digest =
                canonical_hash("canwu.force-supply.accepted-transfer.v1", &evidence)?;
            Ok(evidence)
        })
        .transpose()?;
    let completion_participant = resource_state
        .external_completion_participants
        .grants
        .get(&intent.completion_certificate.acquisition)
        .or_else(|| {
            resource_state
                .external_completion_participants
                .terminal_grants
                .get(&intent.completion_certificate.acquisition)
        })
        .cloned()
        .ok_or_else(|| {
            invalid("authoritative resource state lacks the force completion participant")
        })?;
    let mut settlement = crate::ForceResourceSettlementEvidenceV1 {
        provider_state: packet.authoritative_resource_state.clone(),
        outcome: exact_outcome,
        consumption: consumption.into(),
        fulfillment: fulfillment.into(),
        destination_account_revision: account.revision,
        destination_custodian: account.custodian.clone(),
        accepted_transfer,
        semantic_digest: String::new(),
    };
    settlement.semantic_digest =
        canonical_hash("canwu.force-supply.resource-settlement.v1", &settlement)?;
    Ok((settlement, completion_participant))
}

fn resolve_externality_outcome(
    view: &SimulationView<'_>,
    force_state: &ForceSupplyStateV1,
    packet: &ExternalityOutcomePacketV1,
) -> Result<
    (
        crate::EconomyExternalityOutcomeVersionV1,
        canwu_resource::ExternalCompletionParticipantGrantV1,
    ),
    CanwuError,
> {
    let record = view
        .domain_record_version(&packet.authoritative_outcome)?
        .ok_or_else(|| invalid("authoritative economy outcome version is not retained"))?;
    let outcome = exact_externality_outcome_from_record(&record)?;
    let saga = force_state
        .sagas
        .get(&packet.saga)
        .ok_or_else(|| invalid("externality acknowledgement saga is unavailable"))?;
    let intent = force_state
        .intents
        .get(&saga.intent)
        .ok_or_else(|| invalid("externality acknowledgement intent is unavailable"))?;
    let acquisition = &intent.completion_certificate.acquisition;
    let expected_owner = force_state.completion_leases.acquisitions[acquisition]
        .expected_participants
        .iter()
        .find(|owner| {
            owner.as_str() != PLUGIN_NAME && owner.as_str() != canwu_resource::PLUGIN_NAME
        })
        .ok_or_else(|| invalid("externality acknowledgement provider is unavailable"))?;
    if record.owner != *expected_owner {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "externality acknowledgement cites a different participant owner",
        ));
    }
    let participant_record = view
        .domain_record_version(&packet.authoritative_participant)?
        .ok_or_else(|| invalid("externality completion participant version is not retained"))?;
    let provider = participant_record
        .decode_payload::<crate::ForceExternalityCompletionParticipantProviderRecord>()?;
    if participant_record.owner != *expected_owner
        || provider.provider_plugin != *expected_owner
        || participant_record.reference
            != crate::force_externality_completion_participant_reference(acquisition).into_untyped()
        || provider.clone().seal()? != provider
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "externality completion participant provider is forged or misbound",
        ));
    }
    let participant = provider.participant;
    Ok((outcome, participant))
}

fn exact_externality_outcome_from_record(
    record: &DomainRecord,
) -> Result<crate::EconomyExternalityOutcomeVersionV1, CanwuError> {
    if record.owner != crate::ECONOMY_EXTERNALITY_PROVIDER {
        return Err(invalid(
            "force externality acknowledgement cites a non-authoritative provider record",
        ));
    }
    if !record
        .reference
        .kind
        .matches_type::<crate::ForceExternalityOutcomeProviderRecord>()
    {
        return Err(invalid(
            "force externality acknowledgement cites a non-authoritative provider record",
        ));
    }
    let outcome = record.decode_payload::<crate::ForceExternalityOutcomeProviderRecord>()?;
    if record.reference != force_externality_outcome_reference(&outcome.id).into_untyped()
        || record.version != outcome.revision
    {
        return Err(invalid(
            "economy outcome provider record identity or revision is inconsistent",
        ));
    }
    let mut detached = outcome.clone();
    let recorded = std::mem::take(&mut detached.semantic_digest);
    if recorded
        != canonical_hash(
            "canwu.force-supply.economy-externality-outcome.v1",
            &detached,
        )?
    {
        return Err(invalid("authoritative economy outcome digest is forged"));
    }
    Ok(outcome)
}

fn validate_force_candidate(
    view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    if let Some((_, state)) = load_state(view)? {
        state.validate()?;
    }
    Ok(BoundaryProposal::default())
}

pub(crate) fn load_state(
    view: &SimulationView<'_>,
) -> Result<Option<(DomainRecord, ForceSupplyStateV1)>, CanwuError> {
    let Some(record) = view.typed_domain_record(&force_supply_runtime_reference())? else {
        return Ok(None);
    };
    let state = record.decode_payload::<ForceSupplyRuntimeRecord>()?;
    state.validate()?;
    if state.provider_record_version != record.version {
        return Err(invalid(
            "force-supply runtime payload does not bind its exact provider record version",
        ));
    }
    Ok(Some((record.clone(), state)))
}

fn require_holder_authority(
    context: &CommandContext,
    holder: &KnowledgeHolderRef,
) -> Result<(), CanwuError> {
    let authorized = match holder {
        KnowledgeHolderRef::Person(person) => {
            let origin_matches =
                context.authority.decision_origin == DecisionOrigin::Actor { actor: *person };
            let issuer_matches = match &context.issuer {
                Issuer::Actor(actor) => actor == person,
                Issuer::Human(controller) | Issuer::Ai(controller) => {
                    context.decision_controller_id.as_deref() == Some(controller)
                }
                _ => false,
            };
            origin_matches && issuer_matches
        }
        KnowledgeHolderRef::Entity(entity) => {
            context.authority.command_subject.as_ref() == Some(entity)
                && context.decision_controller_id.is_some()
        }
    };
    if authorized {
        Ok(())
    } else {
        Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "force-supply command issuer is not authorized for its holder",
        ))
    }
}

fn expected_domain_rejection(error: &CanwuError) -> bool {
    matches!(
        error.code,
        ErrorCode::InvalidAuthority
            | ErrorCode::InvalidDomainRecord
            | ErrorCode::DomainRecordNotFound
            | ErrorCode::DomainRecordVersionConflict
            | ErrorCode::DuplicateDomainRecord
            | ErrorCode::IdempotencyConflict
            | ErrorCode::InvalidPayload
            | ErrorCode::ValueOutOfRange
    )
}

pub fn enqueue_force_archive(
    canwu: &mut Canwu,
    prepared: &crate::PreparedForceArchiveBatchV1,
    store: &dyn crate::PackageArchiveStore,
) -> Result<ForceArchiveIngressReceiptV1, CanwuError> {
    let record = canwu
        .typed_domain_record(&force_supply_runtime_reference())
        .ok_or_else(|| invalid("force runtime is unavailable"))?;
    let state = record.decode_payload::<ForceSupplyRuntimeRecord>()?;
    if state.prepare_force_archive(prepared.selected.len())? != *prepared {
        return Err(invalid("force archive batch is stale or non-canonical"));
    }
    let commit = prepared.store_and_verify(crate::FORCE_ARCHIVE_DOMAIN, store)?;
    let permit = FORCE_ARCHIVE_PERMIT
        .get()
        .ok_or_else(|| invalid("force archive ingress is not registered"))?;
    let mut durable = commit.retention.clone();
    durable.phase = crate::PackageArchiveRetentionPhaseV1::DurableIngress;
    durable.semantic_digest.clear();
    durable.semantic_digest = canonical_hash(
        "canwu.force-supply-reference.archive-retention.v1",
        &durable,
    )?;
    store.persist_package_archive_retention(&durable)?;
    let ingress = canwu.enqueue_permitted_plugin_ingress(
        PluginIngressRequest::new(
            PLUGIN_NAME,
            FORCE_ARCHIVE_COMMIT_INGRESS,
            canwu.time(),
            serde_json::to_value(&commit).map_err(crate::encode_error)?,
        )
        .with_archive_retention([PluginArchiveRetention {
            namespace: crate::FORCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE.to_owned(),
            object_id: commit.directory_root.clone(),
        }]),
        permit,
    )?;
    Ok(ForceArchiveIngressReceiptV1 {
        ingress,
        retention_handle_id: commit.retention.id,
        directory_root: commit.directory_root,
    })
}

pub fn finalize_force_archive_retention(
    canwu: &mut Canwu,
    store: &dyn crate::PackageArchiveStore,
    ingress: &ForceArchiveIngressReceiptV1,
) -> Result<canwu_api::IngressReceipt, CanwuError> {
    let state = canwu
        .typed_domain_record(&force_supply_runtime_reference())
        .ok_or_else(|| invalid("force runtime is unavailable"))?
        .decode_payload::<ForceSupplyRuntimeRecord>()?;
    let receipt = state
        .archive_maintenance_receipts
        .values()
        .find(|receipt| {
            receipt.retention_handle_id == ingress.retention_handle_id
                && receipt.directory_root == ingress.directory_root
        })
        .cloned()
        .ok_or_else(|| invalid("force archive terminal disposition is unavailable"))?;
    let phase = match receipt.disposition {
        crate::PackageArchiveMaintenanceDispositionV1::Applied => {
            crate::PackageArchiveRetentionPhaseV1::Committed
        }
        crate::PackageArchiveMaintenanceDispositionV1::RejectedStale => {
            crate::PackageArchiveRetentionPhaseV1::RejectedStale
        }
    };
    let stored = store
        .load_package_archive_retention(&ingress.retention_handle_id)?
        .ok_or_else(|| invalid("force archive retention handle is unavailable"))?;
    let finalized = crate::sealed_archive_retention(
        crate::FORCE_ARCHIVE_DOMAIN,
        crate::PackageArchiveRetentionHandleV1 { phase, ..stored },
    )?;
    store.finalize_package_archive_retention(&finalized)?;
    let permit = FORCE_ARCHIVE_ACK_PERMIT
        .get()
        .ok_or_else(|| invalid("force archive acknowledgement ingress is not registered"))?;
    canwu.enqueue_permitted_plugin_ingress(
        PluginIngressRequest::new(
            PLUGIN_NAME,
            FORCE_ARCHIVE_RETENTION_ACK_INGRESS,
            canwu.time(),
            serde_json::to_value(ForceArchiveRetentionAcknowledgementV1 { receipt })
                .map_err(crate::encode_error)?,
        ),
        permit,
    )
}

fn error_code(code: &ErrorCode) -> &'static str {
    match code {
        ErrorCode::InvalidAuthority => "invalid_authority",
        ErrorCode::InvalidDomainRecord => "invalid_domain_record",
        ErrorCode::DomainRecordNotFound => "domain_record_not_found",
        ErrorCode::DomainRecordVersionConflict => "domain_record_version_conflict",
        ErrorCode::DuplicateDomainRecord => "duplicate_domain_record",
        ErrorCode::IdempotencyConflict => "idempotency_conflict",
        ErrorCode::InvalidPayload => "invalid_payload",
        ErrorCode::ValueOutOfRange => "value_out_of_range",
        _ => "force_operation_rejected",
    }
}

fn decode<T: serde::de::DeserializeOwned>(value: &Value, label: &str) -> Result<T, CanwuError> {
    serde_json::from_value(value.clone()).map_err(|error| {
        CanwuError::new(
            ErrorCode::InvalidPayload,
            format!("{label} could not be decoded: {error}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use canwu_api::{
        DomainRecordClass, DomainRecordDraft, DomainRecordLifecycle, DomainRecordVersionRef,
        DomainRecordVersionSource,
    };

    fn provider_version(
        record: canwu_api::DomainRecordRef,
        version: u64,
    ) -> DomainRecordVersionRef {
        DomainRecordVersionRef {
            record,
            version,
            established_by: DomainRecordVersionSource::InitialScenario,
        }
    }

    #[test]
    fn forged_economy_provider_owner_is_rejected_before_acknowledgement() {
        let mut outcome = crate::EconomyExternalityOutcomeVersionV1 {
            id: crate::ExternalityOutcomeId::new(
                "canwu.force-supply-reference:economy-outcome:forged-provider-test",
            )
            .expect("ID"),
            revision: 1,
            intent: crate::ForceExternalityIntentId::new(
                "canwu.force-supply-reference:externality:forged-provider-test",
            )
            .expect("ID"),
            disposition: crate::ExternalityOutcomeDisposition::Applied,
            expected_target: provider_version(
                canwu_api::DomainRecordRef::new(
                    "canwu.economy-reference",
                    "runtime",
                    "canwu.economy-reference:runtime:v1",
                ),
                1,
            ),
            resulting_target_revision: Some(2),
            blocker: None,
            semantic_digest: String::new(),
        };
        outcome.semantic_digest = canonical_hash(
            "canwu.force-supply.economy-externality-outcome.v1",
            &outcome,
        )
        .expect("digest");
        let reference = crate::force_externality_outcome_reference(&outcome.id);
        let draft = DomainRecordDraft::from_typed(reference, &outcome).expect("draft");
        let forged = DomainRecord {
            reference: draft.reference,
            owner: "forged-packet-sender".to_owned(),
            class: DomainRecordClass::Record,
            version: 1,
            lifecycle: DomainRecordLifecycle::Active,
            payload: draft.payload,
            references: draft.references,
        };
        let error = exact_externality_outcome_from_record(&forged)
            .expect_err("forged owner must fail closed");
        assert!(error.message.contains("non-authoritative provider"));
    }
}
