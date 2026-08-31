use crate::{
    DeliveryDispositionV1, EconomyDeliveryAttemptV1, EconomyObservationGrantV1,
    EconomyObservationHeadV1, EconomyOperationId, EconomyOperationOutcomeV1,
    EconomyPriceObservationV1, EconomyProfileV1, EconomyReferenceRuntimeRecord,
    EconomyReferenceStateV1, EconomyRouteObservationV1, LocalEconomyV1, MonthlyEconomyEvidenceV1,
    MonthlyEconomyFrameV1, PLUGIN_NAME, PLUGIN_NAMESPACE, economy_reference_runtime_reference,
    validate_delivery_attempt, validate_price_observation, validate_route_observation,
};
use canwu_api::{
    ArchiveReachabilityManifest, BoundaryContext, BoundaryDirective, BoundaryPhase,
    BoundaryProposal, BoundarySystemContract, Canwu, CanwuError, Command, CommandContext,
    CommandIngress, DecisionAction, DecisionOrigin, DecisionTicketState, DomainRecord,
    DomainRecordDraft, DomainRecordMutation, DomainRecordSchema, ErrorCode, IngressClass,
    IngressPayload, Issuer, PayloadSchema, PluginActionDescriptor, PluginArchiveObjectProvider,
    PluginArchiveRetention, PluginIngressDescriptor, PluginIngressPermit, PluginIngressRequest,
    PluginIngressTarget, PluginRegistrar, SimDuration, SimulationPlugin, SimulationView, StateKey,
    StateVisibility, SystemCadence, SystemDirective, canonical_hash,
};
use canwu_economy_reference_content::ExternalityApplicability;
use canwu_force_supply_reference::{
    EconomyExternalityOutcomeVersionV1, ExternalityOutcomeDisposition, ExternalityOutcomeId,
    ExternalityOutcomePacketV1, ForceExternalityCompletionParticipantProviderRecord,
    ForceExternalityCompletionParticipantProviderV1, ForceExternalityOutcomeProviderRecord,
    ForceSupplyRuntimeRecord, force_externality_completion_participant_reference,
    force_externality_outcome_reference, force_supply_runtime_reference,
};
use canwu_resource::{
    ResourceOperationStatus, ResourceRuntimeRecord, ResourceTransferState,
    resource_runtime_reference,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::OnceLock;

pub const ECONOMY_COMMAND: &str = "apply_economy_reference_operation_v1";
pub const ECONOMY_COMMAND_INGRESS: &str = "economy_reference_operation_v1";
pub const ECONOMY_ARCHIVE_COMMIT_INGRESS: &str = "economy_archive_commit_v1";
pub const ECONOMY_ARCHIVE_RETENTION_ACK_INGRESS: &str = "economy_archive_retention_ack_v1";
pub const ECONOMY_SEMANTIC_HASH: &str =
    "41ce9f999320db6c724753f81af44a31237bf78ec17b2df7a56943eb10f12a7f";
static ECONOMY_ARCHIVE_PERMIT: OnceLock<PluginIngressPermit> = OnceLock::new();
static ECONOMY_ARCHIVE_ACK_PERMIT: OnceLock<PluginIngressPermit> = OnceLock::new();

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EconomyArchiveIngressReceiptV1 {
    pub ingress: canwu_api::IngressReceipt,
    pub retention_handle_id: String,
    pub directory_root: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct EconomyArchiveRetentionAcknowledgementV1 {
    receipt: crate::EconomyArchiveMaintenanceReceiptV1,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[allow(clippy::large_enum_variant)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum EconomyOperationV1 {
    RegisterProfile {
        profile: EconomyProfileV1,
    },
    RegisterLocalEconomy {
        economy: LocalEconomyV1,
    },
    GrantObservation {
        grant: EconomyObservationGrantV1,
    },
    AuthorizeResourceConsumption {
        intent: canwu_resource::ResourceConsumptionIntentV1,
    },
    RetireResourceConsumption {
        intent: canwu_resource::ResourceConsumptionIntentId,
        authoritative_resource_state: canwu_api::DomainRecordVersionRef,
        outcome_id: canwu_resource::ResourceOperationOutcomeId,
    },
    SelectDecision {
        economy: crate::LocalEconomyId,
        decision: crate::GrainDecision,
        selection: EconomyDecisionSelectionV1,
    },
    SelectResiliencePosture {
        economy: crate::LocalEconomyId,
        posture: String,
        selection: EconomyDecisionSelectionV1,
    },
    GrantCompletionParticipant {
        request: canwu_resource::RequestExternalCompletionParticipantGrantV1,
    },
    PrepareCompletionParticipant {
        request: canwu_resource::PrepareExternalCompletionParticipantGrantV1,
    },
    ConsumeCompletionParticipant {
        request: canwu_resource::ConsumeExternalCompletionParticipantGrantV1,
    },
    CompleteCompletionParticipant {
        request: canwu_resource::CompleteExternalCompletionParticipantGrantV1,
    },
    ReleaseCompletionParticipant {
        request: canwu_resource::ReleaseExternalCompletionParticipantGrantV1,
    },
    ExpireCompletionParticipants {
        request: canwu_resource::ExpireExternalCompletionParticipantGrantsV1,
    },
    RecordRouteObservation {
        observation: EconomyRouteObservationV1,
    },
    PublishRouteProvider {
        payload: crate::EconomyRouteProviderPayloadV1,
    },
    RecordPriceObservation {
        observation: EconomyPriceObservationV1,
    },
    PublishPriceProvider {
        payload: crate::EconomyPriceProviderPayloadV1,
    },
    RecordDeliveryAttempt {
        attempt: EconomyDeliveryAttemptV1,
    },
    CloseMonth {
        economy: crate::LocalEconomyId,
        evidence: MonthlyEconomyEvidenceV1,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EconomyDecisionSelectionV1 {
    pub ticket: canwu_api::DecisionTicketId,
    pub option_id: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct EconomyCommandV1 {
    pub holder: canwu_api::KnowledgeHolderRef,
    pub operation_id: EconomyOperationId,
    pub expected_runtime_revision: u64,
    pub operation: EconomyOperationV1,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct AdmittedEconomyCommandV1 {
    value: EconomyCommandV1,
    input_digest: String,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct EconomyReferencePlugin;

fn economy_archive_reachability(
    view: &SimulationView<'_>,
    provider: &dyn PluginArchiveObjectProvider,
    manifest: &mut ArchiveReachabilityManifest,
) -> Result<(), CanwuError> {
    let Some(record) = view.typed_domain_record(&economy_reference_runtime_reference())? else {
        return Ok(());
    };
    let state = record.decode_payload::<EconomyReferenceRuntimeRecord>()?;
    let roots = state.archive_head.directory_root.iter().cloned().chain(
        state
            .archive_retention_handles
            .values()
            .map(|handle| handle.directory_root.clone()),
    );
    canwu_force_supply_reference::extend_archive_reachability::<
        crate::EconomyArchiveKeyV1,
        crate::EconomyArchivePayloadV1,
    >(crate::ECONOMY_ARCHIVE_DOMAIN, roots, provider, manifest)
}

impl SimulationPlugin for EconomyReferencePlugin {
    fn name(&self) -> &'static str {
        PLUGIN_NAME
    }

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    fn semantic_hash(&self) -> &'static str {
        ECONOMY_SEMANTIC_HASH
    }

    fn validate_activation(&self, records: &[DomainRecord]) -> Result<(), CanwuError> {
        let matching: Vec<_> = records
            .iter()
            .filter(|record| {
                record
                    .reference
                    .kind
                    .matches_type::<EconomyReferenceRuntimeRecord>()
            })
            .collect();
        if matching.len() > 1 {
            return Err(invalid(
                "economy-reference activation contains multiple runtime roots",
            ));
        }
        if let Some(record) = matching.first() {
            let state = record.decode_payload::<EconomyReferenceRuntimeRecord>()?;
            state.validate()?;
            if record.reference != economy_reference_runtime_reference().into_untyped()
                || record.version != state.revision
                || state.draft()?.payload != record.payload
            {
                return Err(invalid(
                    "economy-reference activation runtime identity or payload differs",
                ));
            }
        }
        for record in records.iter().filter(|record| {
            record
                .reference
                .kind
                .matches_type::<ForceExternalityOutcomeProviderRecord>()
        }) {
            let outcome = record.decode_payload::<ForceExternalityOutcomeProviderRecord>()?;
            let mut detached = outcome.clone();
            let recorded = std::mem::take(&mut detached.semantic_digest);
            if record.owner != PLUGIN_NAME
                || record.reference
                    != force_externality_outcome_reference(&outcome.id).into_untyped()
                || record.version != outcome.revision
                || recorded
                    != canonical_hash(
                        "canwu.force-supply.economy-externality-outcome.v1",
                        &detached,
                    )?
            {
                return Err(invalid(
                    "economy externality provider record is forged or misidentified",
                ));
            }
        }
        for record in records.iter().filter(|record| {
            record
                .reference
                .kind
                .matches_type::<ForceExternalityCompletionParticipantProviderRecord>()
        }) {
            let payload =
                record.decode_payload::<ForceExternalityCompletionParticipantProviderRecord>()?;
            if record.owner != PLUGIN_NAME
                || payload.provider_plugin != PLUGIN_NAME
                || record.reference
                    != force_externality_completion_participant_reference(
                        &payload.participant.grant.acquisition,
                    )
                    .into_untyped()
                || payload.clone().seal()? != payload
            {
                return Err(invalid(
                    "economy externality completion participant provider is forged",
                ));
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_archive_reachability_participant(economy_archive_reachability)?;
        registrar.register_record_schema(DomainRecordSchema::for_record::<
            EconomyReferenceRuntimeRecord,
        >())?;
        registrar.register_record_schema(DomainRecordSchema::for_record::<
            ForceExternalityOutcomeProviderRecord,
        >())?;
        registrar.register_record_schema(DomainRecordSchema::for_record::<
            ForceExternalityCompletionParticipantProviderRecord,
        >())?;
        registrar.register_record_schema(DomainRecordSchema::for_record::<
            crate::EconomyRouteProviderRecord,
        >())?;
        registrar.register_record_schema(DomainRecordSchema::for_record::<
            crate::EconomyPriceProviderRecord,
        >())?;
        registrar.register_command(
            PluginActionDescriptor {
                name: ECONOMY_COMMAND.to_owned(),
                description: "Submit one holder-authorized local-economy operation".to_owned(),
                payload_schema: PayloadSchema::Any,
                reads: vec![economy_state_key()],
                writes: Vec::new(),
            },
            admit_economy_command,
        )?;
        registrar.register_ingress(PluginIngressDescriptor {
            name: ECONOMY_COMMAND_INGRESS.to_owned(),
            description: "Settle one admitted local-economy operation".to_owned(),
            class: IngressClass::Decision,
            payload_schema: PayloadSchema::Any,
        })?;
        let archive_permit = registrar.register_internal_ingress_with_archive_retention(
            PluginIngressDescriptor {
                name: ECONOMY_ARCHIVE_COMMIT_INGRESS.to_owned(),
                description: "Commit one verified economy archive batch".to_owned(),
                class: IngressClass::ScheduledSystem,
                payload_schema: PayloadSchema::Any,
            },
            crate::ECONOMY_ARCHIVE_INDEX_DIRECTORY_NAMESPACE,
            vec![
                "/directory_root".to_owned(),
                "/retention/directory_root".to_owned(),
            ],
        )?;
        let _ = ECONOMY_ARCHIVE_PERMIT.set(archive_permit);
        let ack_permit = registrar.register_internal_ingress(PluginIngressDescriptor {
            name: ECONOMY_ARCHIVE_RETENTION_ACK_INGRESS.to_owned(),
            description: "Acknowledge economy archive retention finalization".to_owned(),
            class: IngressClass::Acknowledgement,
            payload_schema: PayloadSchema::Any,
        })?;
        let _ = ECONOMY_ARCHIVE_ACK_PERMIT.set(ack_permit);

        let mut apply = BoundarySystemContract::new(
            "apply-economy-reference-operation-v1",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        apply.reads = vec![
            StateKey::core_decisions(),
            StateKey::core_ingress(),
            economy_state_key(),
            StateKey::new(canwu_resource::PLUGIN_NAMESPACE, "runtime"),
            StateKey::new(canwu_force_supply_reference::PLUGIN_NAMESPACE, "runtime"),
            StateKey::new(PLUGIN_NAMESPACE, "route-observation-provider"),
            StateKey::new(PLUGIN_NAMESPACE, "price-observation-provider"),
            externality_outcome_state_key(),
            externality_participant_state_key(),
        ];
        apply.writes = vec![
            economy_state_key(),
            StateKey::new(PLUGIN_NAMESPACE, "route-observation-provider"),
            StateKey::new(PLUGIN_NAMESPACE, "price-observation-provider"),
            externality_outcome_state_key(),
            externality_participant_state_key(),
        ];
        apply.emits = vec![
            "canwu.economy-reference.operation_applied.v1".to_owned(),
            "canwu.economy-reference.operation_rejected.v1".to_owned(),
            "canwu.economy-reference.month_closed.v1".to_owned(),
        ];
        apply.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(apply, apply_economy_ingress)?;

        let mut externalities = BoundarySystemContract::new(
            "apply-force-requisition-externalities-v1",
            BoundaryPhase::HistoricalCandidateEvaluation,
            SystemCadence::EventDriven,
        );
        externalities.reads = vec![
            economy_state_key(),
            StateKey::new(canwu_resource::PLUGIN_NAMESPACE, "runtime"),
            StateKey::new(canwu_force_supply_reference::PLUGIN_NAMESPACE, "runtime"),
            externality_outcome_state_key(),
        ];
        externalities.writes = vec![economy_state_key(), externality_outcome_state_key()];
        externalities.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(externalities, apply_force_externalities)?;

        let mut acknowledge = BoundarySystemContract::new(
            "acknowledge-force-requisition-externalities-v1",
            BoundaryPhase::StrategicAggregation,
            SystemCadence::EventDriven,
        );
        acknowledge.reads = vec![
            StateKey::new(canwu_force_supply_reference::PLUGIN_NAMESPACE, "runtime"),
            economy_state_key(),
            externality_outcome_state_key(),
            externality_participant_state_key(),
        ];
        acknowledge.plugin_ingress_targets = vec![PluginIngressTarget {
            target_plugin: canwu_force_supply_reference::PLUGIN_NAME.to_owned(),
            packet_type: canwu_force_supply_reference::FORCE_EXTERNALITY_OUTCOME_INGRESS.to_owned(),
        }];
        acknowledge.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(acknowledge, acknowledge_force_externalities)?;

        let mut validate = BoundarySystemContract::new(
            "validate-economy-reference-state-v1",
            BoundaryPhase::InvariantValidation,
            SystemCadence::EventDriven,
        );
        validate.reads = vec![economy_state_key()];
        validate.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(validate, validate_economy_candidate)
    }
}

pub fn economy_command(value: &EconomyCommandV1) -> Result<Command, serde_json::Error> {
    Ok(Command::Plugin {
        plugin: PLUGIN_NAME.to_owned(),
        command: ECONOMY_COMMAND.to_owned(),
        payload: serde_json::to_value(value)?,
    })
}

fn admit_economy_command(
    view: &SimulationView<'_>,
    context: &CommandContext,
    payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    if context.ingress == CommandIngress::LegacyDirect {
        return Err(CanwuError::new(
            ErrorCode::MixedCommandIngress,
            "economy-reference operations require tracked command ingress",
        ));
    }
    let value: EconomyCommandV1 = decode(payload, "economy-reference command")?;
    require_holder_authority(context, &value.holder)?;
    let input_digest = canonical_hash("canwu.economy.operation-input.v1", &value)?;
    let record = view
        .typed_domain_record(&economy_reference_runtime_reference())?
        .ok_or_else(|| missing("economy-reference runtime is unavailable"))?;
    let state = record.decode_payload::<EconomyReferenceRuntimeRecord>()?;
    if let Some(existing) = state.outcomes.get(&value.operation_id) {
        if existing.input_digest == input_digest {
            return Ok(Vec::new());
        }
        return Err(CanwuError::new(
            ErrorCode::IdempotencyConflict,
            "economy operation ID was reused with different input",
        ));
    }
    if let Some(record) = archived_economy_record(
        view,
        &state,
        &crate::EconomyArchiveKeyV1::OperationOutcome(value.operation_id.clone()),
    )? {
        let crate::EconomyArchivePayloadV1::OperationOutcome(existing) = record.payload else {
            return Err(invalid(
                "economy archive operation membership has the wrong payload",
            ));
        };
        if existing.input_digest == input_digest {
            return Ok(Vec::new());
        }
        return Err(CanwuError::new(
            ErrorCode::IdempotencyConflict,
            "archived economy operation ID was reused with different input",
        ));
    }
    Ok(vec![SystemDirective::EnqueuePluginIngress {
        after: SimDuration::ZERO,
        packet_type: ECONOMY_COMMAND_INGRESS.to_owned(),
        priority: 0,
        payload: encode(&AdmittedEconomyCommandV1 {
            value: value.clone(),
            input_digest,
        })?,
        affected: vec![holder_entity(&value.holder)],
    }])
}

#[allow(clippy::too_many_lines)]
fn apply_economy_ingress(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let Some(record) = view
        .typed_domain_record(&economy_reference_runtime_reference())?
        .cloned()
    else {
        return Ok(BoundaryProposal::default());
    };
    let mut state = record.decode_payload::<EconomyReferenceRuntimeRecord>()?;
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
        if packet_type == ECONOMY_ARCHIVE_COMMIT_INGRESS {
            let commit: crate::VerifiedEconomyArchiveCommitV1 =
                decode(payload, "verified economy archive commit")?;
            let externality_records = archive_externality_retirements(view, &state, &commit)?;
            state.apply_economy_archive_commit(&commit)?;
            state.revision = state
                .revision
                .checked_add(1)
                .ok_or_else(|| invalid("economy archive revision overflowed"))?;
            changed = true;
            directives.extend(externality_records);
            continue;
        }
        if packet_type == ECONOMY_ARCHIVE_RETENTION_ACK_INGRESS {
            let acknowledgement: EconomyArchiveRetentionAcknowledgementV1 =
                decode(payload, "economy archive retention acknowledgement")?;
            let persisted = state
                .archive_maintenance_receipts
                .get(&acknowledgement.receipt.sequence)
                .ok_or_else(|| invalid("economy archive maintenance receipt is unavailable"))?;
            if persisted != &acknowledgement.receipt {
                return Err(invalid("economy archive acknowledgement is forged"));
            }
            state
                .archive_retention_handles
                .remove(&acknowledgement.receipt.retention_handle_id);
            state.revision = state
                .revision
                .checked_add(1)
                .ok_or_else(|| invalid("economy archive revision overflowed"))?;
            changed = true;
            continue;
        }
        if packet_type != ECONOMY_COMMAND_INGRESS {
            continue;
        }
        let admitted: AdmittedEconomyCommandV1 = decode(payload, "admitted economy operation")?;
        if let Some(existing) = state.outcomes.get(&admitted.value.operation_id) {
            if existing.input_digest != admitted.input_digest {
                return Err(CanwuError::new(
                    ErrorCode::IdempotencyConflict,
                    "economy operation ID was reused with different input",
                ));
            }
            continue;
        }
        if let Some(record) = archived_economy_record(
            view,
            &state,
            &crate::EconomyArchiveKeyV1::OperationOutcome(admitted.value.operation_id.clone()),
        )? {
            let crate::EconomyArchivePayloadV1::OperationOutcome(existing) = record.payload else {
                return Err(invalid(
                    "economy archive operation membership has the wrong payload",
                ));
            };
            if existing.input_digest != admitted.input_digest {
                return Err(CanwuError::new(
                    ErrorCode::IdempotencyConflict,
                    "archived economy operation ID was reused with different input",
                ));
            }
            continue;
        }
        if admitted.value.expected_runtime_revision != state.revision {
            persist_rejection(
                &mut state,
                &admitted,
                context.at,
                &ErrorCode::DomainRecordVersionConflict,
                "economy runtime revision is stale".to_owned(),
            )?;
            changed = true;
            continue;
        }
        let mut candidate = state.clone();
        let mut operation_directives = Vec::new();
        let result = apply_operation(
            view,
            &mut candidate,
            &admitted.value,
            context.at,
            &mut operation_directives,
        );
        match result {
            Ok(month_closed) => {
                candidate.revision = candidate
                    .revision
                    .checked_add(1)
                    .ok_or_else(|| invalid("economy runtime revision overflowed"))?;
                candidate.outcomes.insert(
                    admitted.value.operation_id.clone(),
                    EconomyOperationOutcomeV1 {
                        id: admitted.value.operation_id.clone(),
                        input_digest: admitted.input_digest,
                        applied: true,
                        rejection_code: None,
                        rejection_reason: None,
                        settled_at: context.at,
                    },
                );
                candidate.validate()?;
                state = candidate;
                changed = true;
                directives.extend(operation_directives);
                directives.push(BoundaryDirective::Emit {
                    event_type: "canwu.economy-reference.operation_applied.v1".to_owned(),
                    summary: format!("economy operation {} applied", admitted.value.operation_id),
                    affected: vec![holder_entity(&admitted.value.holder)],
                });
                if month_closed {
                    directives.push(BoundaryDirective::Emit {
                        event_type: "canwu.economy-reference.month_closed.v1".to_owned(),
                        summary: "local economy monthly evidence closed".to_owned(),
                        affected: vec![holder_entity(&admitted.value.holder)],
                    });
                }
            }
            Err(error) if expected_domain_rejection(&error) => {
                persist_rejection(
                    &mut state,
                    &admitted,
                    context.at,
                    &error.code,
                    error.message,
                )?;
                changed = true;
                directives.push(BoundaryDirective::Emit {
                    event_type: "canwu.economy-reference.operation_rejected.v1".to_owned(),
                    summary: format!("economy operation {} rejected", admitted.value.operation_id),
                    affected: vec![holder_entity(&admitted.value.holder)],
                });
            }
            Err(error) => return Err(error),
        }
    }
    if changed {
        directives.push(BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Update {
                record: state.draft()?,
                expected_version: record.version,
            },
            summary: "settle economy-reference operations".to_owned(),
        });
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

fn persist_rejection(
    state: &mut EconomyReferenceStateV1,
    admitted: &AdmittedEconomyCommandV1,
    settled_at: canwu_api::SimTime,
    code: &ErrorCode,
    reason: String,
) -> Result<(), CanwuError> {
    state.revision = state
        .revision
        .checked_add(1)
        .ok_or_else(|| invalid("economy runtime revision overflowed on rejection"))?;
    state.outcomes.insert(
        admitted.value.operation_id.clone(),
        EconomyOperationOutcomeV1 {
            id: admitted.value.operation_id.clone(),
            input_digest: admitted.input_digest.clone(),
            applied: false,
            rejection_code: Some(error_code(code).to_owned()),
            rejection_reason: Some(reason),
            settled_at,
        },
    );
    state.validate()
}

fn archived_economy_record(
    provider: &dyn PluginArchiveObjectProvider,
    state: &EconomyReferenceStateV1,
    key: &crate::EconomyArchiveKeyV1,
) -> Result<
    Option<
        canwu_force_supply_reference::PackageArchiveRecordV1<
            crate::EconomyArchiveKeyV1,
            crate::EconomyArchivePayloadV1,
        >,
    >,
    CanwuError,
> {
    canwu_force_supply_reference::load_package_archive_record(
        crate::ECONOMY_ARCHIVE_DOMAIN,
        &state.archive_head,
        provider,
        key,
    )
}

#[allow(clippy::too_many_lines)]
fn apply_operation(
    view: &SimulationView<'_>,
    state: &mut EconomyReferenceStateV1,
    command: &EconomyCommandV1,
    now: canwu_api::SimTime,
    directives: &mut Vec<BoundaryDirective>,
) -> Result<bool, CanwuError> {
    match &command.operation {
        EconomyOperationV1::RegisterProfile { profile } => {
            profile.validate()?;
            if state
                .profiles
                .insert(profile.id.clone(), profile.clone())
                .is_some()
            {
                return Err(CanwuError::new(
                    ErrorCode::DuplicateDomainRecord,
                    "economy profile already exists",
                ));
            }
            Ok(false)
        }
        EconomyOperationV1::RegisterLocalEconomy { economy } => {
            if economy.manager != command.holder
                || economy.revision == 0
                || economy.population_wellbeing_per_mille > 1_000
                || economy.cooperation_per_mille > 1_000
                || economy.pending_harvest_penalty_per_mille > 1_000
                || !state.profiles.contains_key(&economy.profile)
            {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "local economy manager or profile is invalid",
                ));
            }
            if state
                .local_economies
                .insert(economy.id.clone(), economy.clone())
                .is_some()
            {
                return Err(CanwuError::new(
                    ErrorCode::DuplicateDomainRecord,
                    "local economy already exists",
                ));
            }
            Ok(false)
        }
        EconomyOperationV1::GrantObservation { grant } => {
            if grant.confidence_per_mille > 1_000
                || grant.scopes.is_empty()
                || grant.scopes.iter().any(|scope| {
                    !state
                        .local_economies
                        .values()
                        .any(|economy| &economy.scope == scope && economy.manager == command.holder)
                })
            {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "observation grant is outside the manager's local economy scopes",
                ));
            }
            state
                .observation_grants
                .insert(grant.id.clone(), grant.clone());
            for scope in &grant.scopes {
                let key = crate::holder_scope_index_key(&grant.holder, scope)?;
                if state
                    .observation_grant_by_holder_scope
                    .insert(key, grant.id.clone())
                    .is_some()
                {
                    return Err(CanwuError::new(
                        ErrorCode::DuplicateDomainRecord,
                        "economy holder/scope already has an observation grant",
                    ));
                }
            }
            Ok(false)
        }
        EconomyOperationV1::AuthorizeResourceConsumption { intent } => {
            intent
                .validate()
                .map_err(|error| invalid(error.to_string()))?;
            if intent.provider_plugin != PLUGIN_NAME
                || intent.status != canwu_resource::ResourceConsumptionIntentStatusV1::Authorized
                || state.resource_consumption_intents.contains_key(&intent.id)
                || state.resource_consumption_intents.values().any(|existing| {
                    existing.operation_key == intent.operation_key
                        || existing.consumption_id == intent.consumption_id
                })
            {
                return Err(invalid(
                    "economy resource consumption intent is invalid or duplicated",
                ));
            }
            state
                .resource_consumption_intents
                .insert(intent.id.clone(), intent.clone());
            Ok(false)
        }
        EconomyOperationV1::RetireResourceConsumption {
            intent,
            authoritative_resource_state,
            outcome_id,
        } => {
            let authorized = state
                .resource_consumption_intents
                .get(intent)
                .cloned()
                .ok_or_else(|| missing("economy resource consumption intent is unavailable"))?;
            let record = view
                .domain_record_version(authoritative_resource_state)?
                .ok_or_else(|| missing("resource consumption outcome source is unavailable"))?;
            if record.owner != canwu_resource::PLUGIN_NAME
                || record.reference != resource_runtime_reference().into_untyped()
            {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "resource consumption retirement cites another provider",
                ));
            }
            let resource = record.decode_payload::<ResourceRuntimeRecord>()?;
            let outcome = resource
                .outcomes
                .get(&authorized.operation_key)
                .ok_or_else(|| missing("resource consumption outcome is unavailable"))?;
            if &outcome.id != outcome_id
                || outcome.status != ResourceOperationStatus::Applied
                || outcome.result_ref
                    != Some(canwu_resource::ResourceRecordRefV1::Consumption(
                        authorized.consumption_id.clone(),
                    ))
            {
                return Err(invalid(
                    "resource consumption retirement does not cite its applied outcome",
                ));
            }
            state.resource_consumption_intents.remove(intent);
            Ok(false)
        }
        EconomyOperationV1::SelectDecision {
            economy,
            decision,
            selection,
        } => {
            validate_economy_decision_command(view, command, selection)?;
            if selection.option_id != grain_decision_option_id(*decision) {
                return Err(invalid("economy decision option does not match its branch"));
            }
            let value = state
                .local_economies
                .get_mut(economy)
                .ok_or_else(|| missing("local economy is unavailable"))?;
            require_manager(value, &command.holder)?;
            value.latest_decision = *decision;
            value.revision = value
                .revision
                .checked_add(1)
                .ok_or_else(|| invalid("local economy revision overflowed"))?;
            Ok(false)
        }
        EconomyOperationV1::SelectResiliencePosture {
            economy,
            posture,
            selection,
        } => {
            validate_economy_decision_command(view, command, selection)?;
            let value = state
                .local_economies
                .get(economy)
                .ok_or_else(|| missing("local economy is unavailable"))?;
            require_manager(value, &command.holder)?;
            if posture.is_empty() || posture.len() > 192 || selection.option_id != *posture {
                return Err(invalid("economy resilience posture is invalid"));
            }
            state
                .resilience_postures
                .insert(economy.clone(), posture.clone());
            Ok(false)
        }
        EconomyOperationV1::GrantCompletionParticipant { request } => {
            validate_completion_coordinator(view, request)?;
            state.grant_completion_participant(request.clone())?;
            publish_externality_participant_provider(
                view,
                state,
                &request.acquisition,
                directives,
            )?;
            Ok(false)
        }
        EconomyOperationV1::PrepareCompletionParticipant { request } => {
            let participant = state
                .completion_participants
                .get(&request.acquisition)
                .ok_or_else(|| missing("economy completion participant is unavailable"))?;
            validate_completion_coordinator_transition(
                view,
                &request.coordinator_source,
                participant,
                None,
            )?;
            state.prepare_completion_participant(request.clone())?;
            publish_externality_participant_provider(
                view,
                state,
                &request.acquisition,
                directives,
            )?;
            Ok(false)
        }
        EconomyOperationV1::ConsumeCompletionParticipant { request } => {
            let participant = state
                .completion_participants
                .get(&request.certificate.acquisition)
                .ok_or_else(|| missing("economy completion participant is unavailable"))?;
            validate_completion_coordinator_transition(
                view,
                &request.coordinator_source,
                participant,
                Some(&request.certificate),
            )?;
            state.consume_completion_participant(request.clone())?;
            publish_externality_participant_provider(
                view,
                state,
                &request.certificate.acquisition,
                directives,
            )?;
            Ok(false)
        }
        EconomyOperationV1::CompleteCompletionParticipant { request } => {
            state.complete_completion_participant(request)?;
            publish_externality_participant_provider(
                view,
                state,
                &request.acquisition,
                directives,
            )?;
            Ok(false)
        }
        EconomyOperationV1::ReleaseCompletionParticipant { request } => {
            let participant = state
                .completion_participants
                .get(&request.acquisition)
                .ok_or_else(|| missing("economy completion participant is unavailable"))?;
            validate_completion_coordinator_transition(
                view,
                &request.coordinator_source,
                participant,
                None,
            )?;
            state.release_completion_participant(request.clone())?;
            publish_externality_participant_provider(
                view,
                state,
                &request.acquisition,
                directives,
            )?;
            Ok(false)
        }
        EconomyOperationV1::ExpireCompletionParticipants { request } => {
            let before = state.completion_participants.clone();
            state.expire_completion_participants(request)?;
            let changed = state
                .completion_participants
                .iter()
                .filter(|(id, participant)| before.get(*id) != Some(*participant))
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>();
            for acquisition in changed {
                publish_externality_participant_provider(view, state, &acquisition, directives)?;
            }
            Ok(false)
        }
        EconomyOperationV1::RecordRouteObservation { observation } => {
            require_observation_grant(
                state,
                &command.holder,
                &observation.target_scope,
                &observation.holder,
            )?;
            let mut sealed = observation.clone();
            sealed.semantic_digest.clear();
            sealed.semantic_digest = canonical_hash("canwu.economy.route-observation.v1", &sealed)?;
            validate_route_observation(&sealed)?;
            validate_source_versions(view, &sealed.source_versions)?;
            validate_route_provider_payload(view, &sealed)?;
            state.index_route_observation(&sealed)?;
            state
                .route_heads_by_holder_scope
                .entry(crate::holder_scope_index_key(
                    &sealed.holder,
                    &sealed.target_scope,
                )?)
                .or_default()
                .insert(
                    crate::route_head_index_key(&sealed.route_key, &sealed.source_scope),
                    sealed.id.clone(),
                );
            state.route_observations.insert(sealed.id.clone(), sealed);
            Ok(false)
        }
        EconomyOperationV1::PublishRouteProvider { payload } => {
            require_observation_grant(
                state,
                &command.holder,
                &payload.target_scope,
                &payload.holder,
            )?;
            validate_source_versions(view, &payload.source_versions)?;
            let sealed = payload.clone().seal()?;
            let reference = crate::economy_route_provider_reference(&sealed.id);
            if view
                .domain_record(&reference.clone().into_untyped())?
                .is_some()
            {
                return Err(CanwuError::new(
                    ErrorCode::DuplicateDomainRecord,
                    "economy route provider record already exists",
                ));
            }
            directives.push(BoundaryDirective::MutateRecord {
                mutation: DomainRecordMutation::Create {
                    record: DomainRecordDraft::from_typed(reference, &sealed)?,
                },
                summary: format!("publish economy route provider {}", sealed.id),
            });
            Ok(false)
        }
        EconomyOperationV1::RecordPriceObservation { observation } => {
            require_observation_grant(
                state,
                &command.holder,
                &observation.scope,
                &observation.holder,
            )?;
            let mut sealed = observation.clone();
            sealed.semantic_digest.clear();
            sealed.semantic_digest = canonical_hash("canwu.economy.price-observation.v1", &sealed)?;
            validate_price_observation(&sealed, state)?;
            validate_source_versions(view, &sealed.source_versions)?;
            validate_price_provider_payload(view, &sealed)?;
            state.index_price_observation(&sealed)?;
            state
                .price_heads_by_holder_scope
                .entry(crate::holder_scope_index_key(
                    &sealed.holder,
                    &sealed.scope,
                )?)
                .or_default()
                .insert(
                    crate::price_head_index_key(sealed.kind, &sealed.interpretation_rule_revision),
                    sealed.id.clone(),
                );
            state.price_observations.insert(sealed.id.clone(), sealed);
            Ok(false)
        }
        EconomyOperationV1::PublishPriceProvider { payload } => {
            require_observation_grant(state, &command.holder, &payload.scope, &payload.holder)?;
            validate_source_versions(view, &payload.source_versions)?;
            let sealed = payload.clone().seal()?;
            let reference = crate::economy_price_provider_reference(&sealed.id);
            if view
                .domain_record(&reference.clone().into_untyped())?
                .is_some()
            {
                return Err(CanwuError::new(
                    ErrorCode::DuplicateDomainRecord,
                    "economy price provider record already exists",
                ));
            }
            directives.push(BoundaryDirective::MutateRecord {
                mutation: DomainRecordMutation::Create {
                    record: DomainRecordDraft::from_typed(reference, &sealed)?,
                },
                summary: format!("publish economy price provider {}", sealed.id),
            });
            Ok(false)
        }
        EconomyOperationV1::RecordDeliveryAttempt { attempt } => {
            let economy = state
                .local_economies
                .get(&attempt.economy)
                .ok_or_else(|| missing("delivery local economy is unavailable"))?;
            require_manager(economy, &command.holder)?;
            validate_source_versions(view, &attempt.source_versions)?;
            validate_resource_transfer(view, attempt)?;
            let mut sealed = attempt.clone();
            sealed.semantic_digest.clear();
            sealed.semantic_digest = canonical_hash("canwu.economy.delivery-attempt.v1", &sealed)?;
            validate_delivery_attempt(&sealed)?;
            state.delivery_attempts.insert(sealed.id.clone(), sealed);
            Ok(false)
        }
        EconomyOperationV1::CloseMonth { economy, evidence } => {
            close_month(view, state, economy, evidence, &command.holder, now)?;
            Ok(true)
        }
    }
}

fn publish_externality_participant_provider(
    view: &SimulationView<'_>,
    state: &EconomyReferenceStateV1,
    acquisition: &canwu_resource::CompletionLeaseAcquisitionId,
    directives: &mut Vec<BoundaryDirective>,
) -> Result<(), CanwuError> {
    let participant = state
        .completion_participants
        .get(acquisition)
        .cloned()
        .ok_or_else(|| missing("economy completion participant is unavailable"))?;
    let payload = ForceExternalityCompletionParticipantProviderV1 {
        provider_plugin: PLUGIN_NAME.to_owned(),
        participant,
        semantic_digest: String::new(),
    }
    .seal()?;
    let reference = force_externality_completion_participant_reference(acquisition);
    let mutation = match view.domain_record(&reference.clone().into_untyped())? {
        Some(record) => {
            if record.owner != PLUGIN_NAME {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "externality participant provider record is owned by another plugin",
                ));
            }
            DomainRecordMutation::Update {
                record: DomainRecordDraft::from_typed(reference, &payload)?,
                expected_version: record.version,
            }
        }
        None => DomainRecordMutation::Create {
            record: DomainRecordDraft::from_typed(reference, &payload)?,
        },
    };
    directives.push(BoundaryDirective::MutateRecord {
        mutation,
        summary: format!("publish externality completion participant {acquisition}"),
    });
    Ok(())
}

fn validate_economy_decision_command(
    view: &SimulationView<'_>,
    command: &EconomyCommandV1,
    selection: &EconomyDecisionSelectionV1,
) -> Result<(), CanwuError> {
    let ticket = view
        .decision_ticket(selection.ticket)?
        .ok_or_else(|| invalid("economy decision ticket is unavailable"))?;
    let DecisionTicketState::Resolved { option_id, .. } = &ticket.state else {
        return Err(invalid("economy decision ticket is not resolved"));
    };
    let option = ticket
        .option(&selection.option_id)
        .ok_or_else(|| invalid("economy decision option is unavailable"))?;
    let DecisionAction::Command { command: action } = &option.action else {
        return Err(invalid("economy decision option has no command"));
    };
    let expected = serde_json::to_value(
        economy_command(command)
            .map_err(|error| CanwuError::new(ErrorCode::InvalidPayload, error.to_string()))?,
    )
    .map_err(|error| CanwuError::new(ErrorCode::InvalidPayload, error.to_string()))?;
    if option_id != &selection.option_id || action != &expected {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "economy command is not the exact resolved decision action",
        ));
    }
    Ok(())
}

const fn grain_decision_option_id(decision: crate::GrainDecision) -> &'static str {
    match decision {
        crate::GrainDecision::ReliefFirst => "relief_first",
        crate::GrainDecision::ForceFirst => "force_first",
        crate::GrainDecision::Balanced => "balanced",
        crate::GrainDecision::RequisitionForForce => "requisition_for_force",
    }
}

#[allow(clippy::too_many_lines)]
fn close_month(
    view: &SimulationView<'_>,
    state: &mut EconomyReferenceStateV1,
    economy_id: &crate::LocalEconomyId,
    evidence: &MonthlyEconomyEvidenceV1,
    holder: &canwu_api::KnowledgeHolderRef,
    now: canwu_api::SimTime,
) -> Result<(), CanwuError> {
    validate_source_versions(view, &evidence.source_versions)?;
    let resource_record = view
        .typed_domain_record(&resource_runtime_reference())?
        .ok_or_else(|| missing("resource runtime is unavailable"))?;
    let resource_state = resource_record.decode_payload::<ResourceRuntimeRecord>()?;
    let civilian = resource_state
        .demands
        .get(&evidence.civilian_demand)
        .ok_or_else(|| missing("civilian demand is unavailable"))?;
    let relief = resource_state
        .demands
        .get(&evidence.relief_demand)
        .ok_or_else(|| missing("relief demand is unavailable"))?;
    let force = resource_state
        .demands
        .get(&evidence.force_demand)
        .ok_or_else(|| missing("force demand is unavailable"))?;
    if let Some(exact) = &evidence.harvest_credit {
        let actual = resource_state
            .outcomes
            .get(&exact.operation_key)
            .ok_or_else(|| missing("harvest credit outcome is unavailable"))?;
        let actual_exact: canwu_resource::ResourceOperationOutcomeVersionV1 = actual.into();
        if &actual_exact != exact || exact.status != ResourceOperationStatus::Applied {
            return Err(CanwuError::new(
                ErrorCode::EvidenceUnavailable,
                "harvest credit outcome does not match authoritative resource state",
            ));
        }
    }
    if let Some(force_operation) = &evidence.force_operation {
        let force_record = view
            .typed_domain_record(&force_supply_runtime_reference())?
            .ok_or_else(|| missing("force-supply runtime is unavailable"))?;
        let force_state = force_record.decode_payload::<ForceSupplyRuntimeRecord>()?;
        let outcome = force_state
            .outcomes
            .get(force_operation)
            .ok_or_else(|| missing("force-supply operation outcome is unavailable"))?;
        if !outcome.applied {
            return Err(CanwuError::new(
                ErrorCode::EvidenceUnavailable,
                "force-supply evidence names a rejected operation",
            ));
        }
    }

    let economy = state
        .local_economies
        .get_mut(economy_id)
        .ok_or_else(|| missing("local economy is unavailable"))?;
    require_manager(economy, holder)?;
    let profile = state
        .profiles
        .get(&economy.profile)
        .ok_or_else(|| missing("local economy profile is unavailable"))?;
    if civilian.requested != profile.consumption.monthly_need
        || relief.requested != profile.relief.monthly_target
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidDomainRecord,
            "monthly demand evidence differs from the compiled economy profile",
        ));
    }
    let month = economy
        .month
        .checked_add(1)
        .ok_or_else(|| invalid("local economy month overflowed"))?;
    let civilian_shortage = civilian.requested.saturating_sub(civilian.fulfilled);
    let wellbeing_cost = civilian_shortage.saturating_mul(u64::from(
        profile.consumption.shortage_wellbeing_cost_per_unit,
    ));
    let relief_gain = relief.fulfilled.saturating_mul(u64::from(
        profile.consumption.relief_wellbeing_gain_per_unit,
    ));
    economy.population_wellbeing_per_mille = economy
        .population_wellbeing_per_mille
        .saturating_sub(u16::try_from(wellbeing_cost).unwrap_or(u16::MAX))
        .saturating_add(u16::try_from(relief_gain).unwrap_or(u16::MAX))
        .min(1_000);
    // Requisition costs are applied only by the exact force-externality ACK path.
    // Monthly closing observes the resulting economy state and must not apply a
    // second copy of the same compiled consequence.
    if economy.latest_decision != crate::GrainDecision::RequisitionForForce {
        economy.cooperation_per_mille = economy.cooperation_per_mille.saturating_add(3).min(1_000);
    }
    let harvest_output = evidence
        .harvest_credit
        .as_ref()
        .map_or(0, |outcome| outcome.quantity);
    economy.month = month;
    economy.revision = economy
        .revision
        .checked_add(1)
        .ok_or_else(|| invalid("local economy revision overflowed"))?;

    let mut frame = MonthlyEconomyFrameV1 {
        economy: economy_id.clone(),
        month,
        at: now,
        decision: economy.latest_decision,
        civilian_requested: civilian.requested,
        civilian_fulfilled: civilian.fulfilled,
        civilian_remainder: civilian.remainder(),
        relief_requested: relief.requested,
        relief_fulfilled: relief.fulfilled,
        relief_remainder: relief.remainder(),
        force_requested: force.requested,
        force_fulfilled: force.fulfilled,
        force_remainder: force.remainder(),
        harvest_output,
        population_wellbeing_per_mille: economy.population_wellbeing_per_mille,
        cooperation_per_mille: economy.cooperation_per_mille,
        evidence: evidence.clone(),
        semantic_digest: String::new(),
    };
    frame.semantic_digest = canonical_hash("canwu.economy.monthly-frame.v1", &frame)?;
    let frames = state.frames.entry(economy_id.clone()).or_default();
    if frames.len() >= state.limits.max_frames_per_economy {
        return Err(CanwuError::new(
            ErrorCode::QueryBudgetExceeded,
            "economy frame hot cap requires archive progress",
        ));
    }
    frames.push(frame);
    let mut head = EconomyObservationHeadV1 {
        economy: economy_id.clone(),
        scope: economy.scope.clone(),
        observed_at: now,
        population_wellbeing_per_mille: economy.population_wellbeing_per_mille,
        cooperation_per_mille: economy.cooperation_per_mille,
        relief_open: relief.requested > 0,
        rationed: civilian.remainder() > 0 || relief.remainder() > 0,
        requisitioned: economy.latest_decision == crate::GrainDecision::RequisitionForForce,
        reserve_release_allowed: economy.latest_decision != crate::GrainDecision::ForceFirst,
        source_versions: evidence.source_versions.clone(),
        semantic_digest: String::new(),
    };
    head.semantic_digest = canonical_hash("canwu.economy.observation-head.v1", &head)?;
    state.push_observation_head(holder, head)
}

fn validate_resource_transfer(
    view: &SimulationView<'_>,
    attempt: &EconomyDeliveryAttemptV1,
) -> Result<(), CanwuError> {
    let resource_record = view
        .typed_domain_record(&resource_runtime_reference())?
        .ok_or_else(|| missing("resource runtime is unavailable"))?;
    let state = resource_record.decode_payload::<ResourceRuntimeRecord>()?;
    let transfer = state
        .transfers
        .get(&attempt.resource_transfer)
        .ok_or_else(|| missing("resource transfer is unavailable"))?;
    let disposition_matches = match attempt.disposition {
        DeliveryDispositionV1::Pending => matches!(
            transfer.state,
            ResourceTransferState::PendingDispatch
                | ResourceTransferState::InTransit
                | ResourceTransferState::ArrivalPending
                | ResourceTransferState::ReturnPending
        ),
        DeliveryDispositionV1::Accepted => transfer.state == ResourceTransferState::Accepted,
        DeliveryDispositionV1::Lost => transfer.state == ResourceTransferState::Lost,
        DeliveryDispositionV1::Returned => transfer.state == ResourceTransferState::Returned,
        DeliveryDispositionV1::CancelledBeforeDebit => {
            transfer.state == ResourceTransferState::Cancelled && transfer.escrow == 0
        }
        DeliveryDispositionV1::ExternalOutflow => {
            transfer.state == ResourceTransferState::ExternalOutflowSettled
        }
    };
    if !disposition_matches
        || transfer
            .transport
            .as_ref()
            .is_some_and(|transport| transport.execution != attempt.execution.id)
    {
        return Err(CanwuError::new(
            ErrorCode::EvidenceUnavailable,
            "delivery attempt differs from the authoritative resource transfer",
        ));
    }
    Ok(())
}

fn validate_source_versions(
    view: &SimulationView<'_>,
    versions: &[canwu_api::DomainRecordVersionRef],
) -> Result<(), CanwuError> {
    if versions.len() > crate::MAX_OBSERVATION_FACTS {
        return Err(CanwuError::new(
            ErrorCode::QueryBudgetExceeded,
            "economy evidence exceeds its bounded source-version budget",
        ));
    }
    for version in versions {
        if !view.domain_record_version_evidence_exists(version)? {
            return Err(CanwuError::new(
                ErrorCode::EvidenceUnavailable,
                "economy evidence source version is unavailable",
            ));
        }
    }
    Ok(())
}

fn require_observation_grant(
    state: &EconomyReferenceStateV1,
    command_holder: &canwu_api::KnowledgeHolderRef,
    scope: &canwu_resource::ResourceScopeId,
    observation_holder: &canwu_api::KnowledgeHolderRef,
) -> Result<(), CanwuError> {
    let manager_authorized = state
        .local_economies
        .values()
        .any(|economy| &economy.scope == scope && &economy.manager == command_holder);
    if !manager_authorized
        || state
            .observation_grant_by_holder_scope
            .get(&crate::holder_scope_index_key(observation_holder, scope)?)
            .and_then(|grant| state.observation_grants.get(grant))
            .is_none_or(|grant| {
                &grant.holder != observation_holder || !grant.scopes.contains(scope)
            })
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "economy observation requires a scope manager and an independently granted holder",
        ));
    }
    Ok(())
}

fn validate_route_provider_payload(
    view: &SimulationView<'_>,
    observation: &EconomyRouteObservationV1,
) -> Result<(), CanwuError> {
    let record = view
        .domain_record_version(&observation.provider_source)?
        .ok_or_else(|| missing("economy route provider body is unavailable"))?;
    if record.owner != PLUGIN_NAME
        || !record
            .reference
            .kind
            .matches_type::<crate::EconomyRouteProviderRecord>()
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "economy route observation cites a non-authoritative provider",
        ));
    }
    let payload = record.decode_payload::<crate::EconomyRouteProviderRecord>()?;
    let mut detached = payload.clone();
    let recorded = std::mem::take(&mut detached.semantic_digest);
    if recorded != canonical_hash("canwu.economy.route-provider-payload.v1", &detached)?
        || payload.holder != observation.holder
        || payload.route_key != observation.route_key
        || payload.target_scope != observation.target_scope
        || payload.source_scope != observation.source_scope
        || payload.observed_at != observation.observed_at
        || payload.reachable != observation.reachable
        || payload.delay_minutes != observation.delay_minutes
        || payload.confidence_per_mille != observation.confidence_per_mille
        || payload.source_versions != observation.source_versions
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "economy route provider payload does not authorize every observation field",
        ));
    }
    Ok(())
}

fn validate_completion_coordinator_source(
    view: &SimulationView<'_>,
    source: &canwu_api::DomainRecordVersionRef,
) -> Result<canwu_force_supply_reference::ForceSupplyStateV1, CanwuError> {
    let record = view
        .domain_record_version(source)?
        .ok_or_else(|| missing("force completion coordinator source is unavailable"))?;
    if record.owner != canwu_force_supply_reference::PLUGIN_NAME
        || record.reference != force_supply_runtime_reference().into_untyped()
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "economy completion participant cites a non-force coordinator",
        ));
    }
    record.decode_payload::<ForceSupplyRuntimeRecord>()
}

fn validate_completion_coordinator(
    view: &SimulationView<'_>,
    request: &canwu_resource::RequestExternalCompletionParticipantGrantV1,
) -> Result<(), CanwuError> {
    let state = validate_completion_coordinator_source(view, &request.coordinator_source)?;
    let acquisition = state
        .completion_leases
        .acquisitions
        .get(&request.acquisition)
        .ok_or_else(|| missing("force completion acquisition is unavailable"))?;
    if acquisition.revision != request.coordinator_acquisition_revision
        || acquisition.operation_key != request.operation_key
        || acquisition.holder != request.holder
        || acquisition.operation_namespace != request.operation_namespace
        || acquisition.eligibility_time != request.eligibility_time
        || acquisition.eligibility_envelope.digest != request.eligibility_envelope_digest
        || acquisition.recipe != request.recipe
        || acquisition.policy_class != request.policy_class
        || !acquisition.expected_participants.contains(PLUGIN_NAME)
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "economy completion participant request differs from its exact force acquisition",
        ));
    }
    Ok(())
}

fn validate_completion_coordinator_transition(
    view: &SimulationView<'_>,
    source: &canwu_api::DomainRecordVersionRef,
    participant: &canwu_resource::ExternalCompletionParticipantGrantV1,
    certificate: Option<&canwu_resource::CompletionLeaseActivationCertificateV1>,
) -> Result<(), CanwuError> {
    let state = validate_completion_coordinator_source(view, source)?;
    let acquisition = state
        .completion_leases
        .acquisitions
        .get(&participant.grant.acquisition)
        .ok_or_else(|| missing("force completion acquisition is unavailable"))?;
    let authoritative = state
        .completion_participant_grants
        .get(&participant.grant.acquisition)
        .and_then(|participants| participants.get(PLUGIN_NAME))
        .ok_or_else(|| missing("force coordinator has no exact economy participant"))?;
    if authoritative != participant
        || acquisition.operation_key != participant.grant.operation_key
        || acquisition.holder != participant.holder
        || acquisition.operation_namespace != participant.operation_namespace
        || acquisition.eligibility_time != participant.eligibility_time
        || acquisition.eligibility_envelope.digest != participant.eligibility_envelope_digest
        || acquisition.recipe != participant.recipe
        || acquisition.policy_class != participant.policy_class
        || certificate.is_some_and(|certificate| {
            state.completion_leases.certificate(&acquisition.id) != Some(certificate)
                || certificate.prepared_grants.iter().all(|(grant, revision)| {
                    grant != &participant.grant.id || revision != &participant.grant.revision
                })
        })
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "economy completion transition differs from the exact force coordinator body",
        ));
    }
    Ok(())
}

fn validate_price_provider_payload(
    view: &SimulationView<'_>,
    observation: &EconomyPriceObservationV1,
) -> Result<(), CanwuError> {
    let record = view
        .domain_record_version(&observation.provider_source)?
        .ok_or_else(|| missing("economy price provider body is unavailable"))?;
    if record.owner != PLUGIN_NAME
        || !record
            .reference
            .kind
            .matches_type::<crate::EconomyPriceProviderRecord>()
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "economy price observation cites a non-authoritative provider",
        ));
    }
    let payload = record.decode_payload::<crate::EconomyPriceProviderRecord>()?;
    let mut detached = payload.clone();
    let recorded = std::mem::take(&mut detached.semantic_digest);
    if recorded != canonical_hash("canwu.economy.price-provider-payload.v1", &detached)?
        || payload.holder != observation.holder
        || payload.scope != observation.scope
        || payload.observed_at != observation.observed_at
        || payload.kind != observation.kind
        || payload.resource_revision != observation.resource_revision
        || payload.quality != observation.quality
        || payload.unit_revision != observation.unit_revision
        || payload.observed_scaled != observation.observed_scaled
        || payload.baseline_scaled != observation.baseline_scaled
        || payload.scale != observation.scale
        || payload.effective_from != observation.effective_from
        || payload.effective_until != observation.effective_until
        || payload.interpretation_rule_revision != observation.interpretation_rule_revision
        || payload.confidence_per_mille != observation.confidence_per_mille
        || payload.source_versions != observation.source_versions
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "economy price provider payload does not authorize every observation field",
        ));
    }
    Ok(())
}

fn require_manager(
    economy: &LocalEconomyV1,
    holder: &canwu_api::KnowledgeHolderRef,
) -> Result<(), CanwuError> {
    if &economy.manager == holder {
        Ok(())
    } else {
        Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "holder does not manage this local economy",
        ))
    }
}

#[allow(clippy::too_many_lines)]
fn apply_force_externalities(
    view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let Some(economy_record) = view
        .typed_domain_record(&economy_reference_runtime_reference())?
        .cloned()
    else {
        return Ok(BoundaryProposal::default());
    };
    let Some(force_record) = view
        .typed_domain_record(&force_supply_runtime_reference())?
        .cloned()
    else {
        return Ok(BoundaryProposal::default());
    };
    let mut economy_state = economy_record.decode_payload::<EconomyReferenceRuntimeRecord>()?;
    let force_state = force_record.decode_payload::<ForceSupplyRuntimeRecord>()?;
    economy_state.validate()?;
    force_state.validate()?;
    let mut intents: Vec<_> = force_state.externality_intents.values().cloned().collect();
    intents.sort_by(|left, right| left.id.cmp(&right.id));
    let mut directives = Vec::new();
    let mut economy_changed = false;
    for intent in intents {
        let saga = force_state
            .sagas
            .get(&intent.saga)
            .ok_or_else(|| invalid("force externality saga is unavailable"))?;
        if saga.externality_intent.as_ref() != Some(&intent.id) {
            return Err(invalid(
                "force externality is not the exact intent retained by its saga",
            ));
        }
        if saga.externality_outcome_source.is_some() {
            continue;
        }
        let outcome_id = ExternalityOutcomeId::new(format!(
            "canwu.economy-reference:externality-outcome:{}",
            intent.id.as_str()
        ))?;
        let reference = force_externality_outcome_reference(&outcome_id);
        if view.typed_domain_record(&reference)?.is_some() {
            continue;
        }
        let force_intent = force_state
            .intents
            .get(&saga.intent)
            .ok_or_else(|| invalid("force externality source intent is unavailable"))?;
        let acquisition = &force_intent.completion_certificate.acquisition;
        if economy_state
            .completion_participants
            .contains_key(acquisition)
        {
            let coordinator_source = view
                .current_domain_record_version(&force_record.reference)?
                .ok_or_else(|| invalid("force coordinator exact version is unavailable"))?;
            economy_state.consume_completion_participant(
                canwu_resource::ConsumeExternalCompletionParticipantGrantV1 {
                    coordinator_source,
                    certificate: force_intent.completion_certificate.clone(),
                    at: force_intent.completion_certificate.eligibility_time,
                },
            )?;
            economy_state.complete_completion_participant(
                &canwu_resource::CompleteExternalCompletionParticipantGrantV1 {
                    acquisition: acquisition.clone(),
                    operation_key: force_intent.resource_operation_key.clone(),
                },
            )?;
        }
        let authoritative_scope =
            authoritative_force_externality_scope(view, &force_state, &intent)?;
        let policy = force_state
            .requisition_policies
            .get(&intent.policy)
            .ok_or_else(|| invalid("force externality intent policy is unavailable"))?;
        let candidate_ids: Vec<_> = economy_state
            .local_economies
            .values()
            .filter(|_| {
                intent.expected_economy_target.record
                    == economy_reference_runtime_reference().into_untyped()
                    && intent.expected_economy_target.version == economy_record.version
            })
            .filter(|economy| {
                economy_state
                    .profiles
                    .get(&economy.profile)
                    .is_some_and(|profile| profile.compiled_content_hash == policy.content_hash)
            })
            .filter(|economy| economy.scope == authoritative_scope)
            .map(|economy| economy.id.clone())
            .collect();
        let (disposition, resulting_target_revision, blocker) = match policy.applicability {
            ExternalityApplicability::ExternalityNotApplicable => {
                (ExternalityOutcomeDisposition::NotApplicable, None, None)
            }
            ExternalityApplicability::ExplicitUnknown => (
                ExternalityOutcomeDisposition::Rejected,
                None,
                Some("compiled externality applicability is explicitly unknown".to_owned()),
            ),
            ExternalityApplicability::Required if candidate_ids.len() == 1 => {
                let economy = economy_state
                    .local_economies
                    .get_mut(&candidate_ids[0])
                    .ok_or_else(|| invalid("matched economy target disappeared"))?;
                economy.cooperation_per_mille = apply_per_mille_delta(
                    economy.cooperation_per_mille,
                    intent.cooperation_delta_per_mille,
                );
                economy.pending_harvest_penalty_per_mille = apply_cost_delta(
                    economy.pending_harvest_penalty_per_mille,
                    intent.harvest_input_delta_per_mille,
                );
                economy.revision = economy
                    .revision
                    .checked_add(1)
                    .ok_or_else(|| invalid("economy externality target revision overflowed"))?;
                (
                    ExternalityOutcomeDisposition::Applied,
                    Some(economy.revision),
                    None,
                )
            }
            ExternalityApplicability::Required => (
                ExternalityOutcomeDisposition::Rejected,
                None,
                Some(
                    if candidate_ids.is_empty() {
                        "no exact economy target matched the expected revision, content, and authoritative resource scope"
                    } else {
                        "more than one economy target matched the expected revision, content, and authoritative resource scope"
                    }
                    .to_owned(),
                ),
            ),
        };
        let mut outcome = EconomyExternalityOutcomeVersionV1 {
            id: outcome_id,
            revision: 1,
            intent: intent.id.clone(),
            disposition,
            expected_target: intent.expected_economy_target.clone(),
            resulting_target_revision,
            blocker,
            semantic_digest: String::new(),
        };
        outcome.semantic_digest = canonical_hash(
            "canwu.force-supply.economy-externality-outcome.v1",
            &outcome,
        )?;
        economy_state
            .externality_outcomes
            .insert(outcome.id.clone(), outcome.clone());
        economy_changed = true;
        directives.push(BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Create {
                record: DomainRecordDraft::from_typed(reference, &outcome)?,
            },
            summary: format!(
                "materialize exact requisition externality outcome {}",
                outcome.id
            ),
        });
    }
    if economy_changed {
        economy_state.revision = economy_state
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("economy runtime revision overflowed on externality"))?;
        economy_state.validate()?;
        directives.push(BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Update {
                record: economy_state.draft()?,
                expected_version: economy_record.version,
            },
            summary: "apply compiled force requisition externalities".to_owned(),
        });
    }
    if directives.is_empty() {
        return Ok(BoundaryProposal::default());
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

fn archive_externality_retirements(
    view: &SimulationView<'_>,
    state: &EconomyReferenceStateV1,
    commit: &crate::VerifiedEconomyArchiveCommitV1,
) -> Result<Vec<BoundaryDirective>, CanwuError> {
    let force = view
        .typed_domain_record(&force_supply_runtime_reference())?
        .map(canwu_api::DomainRecord::decode_payload::<ForceSupplyRuntimeRecord>)
        .transpose()?;
    let mut directives = Vec::new();
    for key in &commit.selected {
        let crate::EconomyArchiveKeyV1::ExternalityOutcome(id) = key else {
            continue;
        };
        let outcome = state
            .externality_outcomes
            .get(id)
            .ok_or_else(|| invalid("economy archive externality outcome disappeared"))?;
        if force.as_ref().is_some_and(|force| {
            force
                .sagas
                .values()
                .any(|saga| saga.externality_outcome.as_ref().map(|value| &value.id) == Some(id))
                || force.terminal_receipts.values().any(|receipt| {
                    receipt.externality_outcome.as_ref().map(|value| &value.id) == Some(id)
                })
        }) {
            return Err(CanwuError::new(
                ErrorCode::InvalidArchive,
                "economy externality outcome still has a hot force continuation",
            ));
        }
        let reference = force_externality_outcome_reference(id).into_untyped();
        let record = view
            .domain_record(&reference)?
            .ok_or_else(|| invalid("economy externality provider record disappeared"))?;
        if record.version != outcome.revision
            || record.decode_payload::<ForceExternalityOutcomeProviderRecord>()? != *outcome
        {
            return Err(invalid(
                "economy externality provider record differs from archive payload",
            ));
        }
        directives.push(BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Retire {
                record: reference,
                expected_version: record.version,
                successor: None,
            },
            summary: format!("retire archived economy externality outcome {id}"),
        });
    }
    Ok(directives)
}

#[allow(clippy::too_many_lines)]
fn authoritative_force_externality_scope(
    view: &SimulationView<'_>,
    force_state: &canwu_force_supply_reference::ForceSupplyStateV1,
    externality: &canwu_force_supply_reference::ForceExternalityIntent,
) -> Result<canwu_resource::ResourceScopeId, CanwuError> {
    let saga = force_state
        .sagas
        .get(&externality.saga)
        .ok_or_else(|| invalid("force externality saga is unavailable"))?;
    if saga.externality_intent.as_ref() != Some(&externality.id) {
        return Err(invalid(
            "force externality is not the exact intent retained by its saga",
        ));
    }
    let consumption_intent = force_state
        .intents
        .get(&saga.intent)
        .ok_or_else(|| invalid("force externality source consumption intent is unavailable"))?;
    if consumption_intent.resource_operation_key != externality.operation_key
        || consumption_intent.resource_outcome.as_ref() != Some(&externality.resource_outcome)
    {
        return Err(invalid(
            "force externality differs from its exact resource consumption outcome",
        ));
    }
    let source = consumption_intent
        .resource_outcome_source
        .as_ref()
        .ok_or_else(|| invalid("force externality lacks an exact resource provider version"))?;
    if source.record != resource_runtime_reference().into_untyped() {
        return Err(invalid(
            "force externality does not cite the authoritative resource root",
        ));
    }
    let resource_record = view
        .domain_record_version(source)?
        .ok_or_else(|| invalid("force externality resource provider version is not retained"))?;
    if resource_record.owner != canwu_resource::PLUGIN_NAME {
        return Err(invalid(
            "force externality resource provider is not resource-owned",
        ));
    }
    let resource_state = resource_record.decode_payload::<ResourceRuntimeRecord>()?;
    resource_state.validate().map_err(|error| {
        invalid(format!(
            "force externality resource state is invalid: {error}"
        ))
    })?;
    if resource_record.version != resource_state.state_revision.get() {
        return Err(invalid(
            "force externality resource provider record and state revisions differ",
        ));
    }
    let outcome = resource_state
        .outcomes
        .get(&externality.operation_key)
        .ok_or_else(|| invalid("force externality resource outcome is unavailable"))?;
    if canwu_resource::ResourceOperationOutcomeVersionV1::from(outcome)
        != externality.resource_outcome
    {
        return Err(invalid(
            "force externality resource outcome differs from its provider version",
        ));
    }
    let allocation = resource_state
        .allocation_legs
        .get(&consumption_intent.allocation.id)
        .ok_or_else(|| invalid("force externality allocation is unavailable"))?;
    let exact = &consumption_intent.allocation;
    let consumed_leg_revision = exact
        .revision
        .get()
        .checked_add(1)
        .ok_or_else(|| invalid("force externality allocation revision overflowed"))?;
    if allocation.revision.get() != consumed_leg_revision
        || allocation.account != exact.account
        || allocation.account_revision != exact.account_revision
        || allocation.resource_revision != exact.resource_revision
        || allocation.unit_revision != exact.unit_revision
        || allocation.quantity != exact.quantity
        || allocation.status != canwu_resource::AllocationLegStatus::Consumed
    {
        return Err(invalid(
            "force externality allocation is not the authoritative consumed successor of its exact reservation",
        ));
    }
    let Some(canwu_resource::ResourceRecordRefV1::Consumption(consumption_id)) =
        &outcome.result_ref
    else {
        return Err(invalid(
            "force externality resource outcome does not identify a consumption",
        ));
    };
    let consumption = resource_state
        .consumptions
        .get(consumption_id)
        .ok_or_else(|| invalid("force externality resource consumption is unavailable"))?;
    let force_evidence = view
        .domain_record_version(&consumption.consumer_evidence)?
        .ok_or_else(|| invalid("force externality force provider version is not retained"))?;
    let expected_force_evidence = consumption_intent
        .completion_certificate
        .locked_target_versions
        .iter()
        .find_map(|target| match target {
            canwu_resource::CompletionLockedTargetV1::ExternalRecord { version }
                if version.record == force_supply_runtime_reference().into_untyped() =>
            {
                Some(version)
            }
            _ => None,
        })
        .ok_or_else(|| invalid("force intent completion certificate lacks force evidence"))?;
    if consumption.operation_key != externality.operation_key
        || consumption.id != consumption_intent.consumption_id
        || consumption.allocation_leg != allocation.id
        || consumption.account != allocation.account
        || consumption.resource_revision != allocation.resource_revision
        || consumption.unit_revision != allocation.unit_revision
        || consumption.quantity != allocation.quantity
        || consumption.consumer_evidence.record != force_supply_runtime_reference().into_untyped()
        || &consumption.consumer_evidence != expected_force_evidence
        || force_evidence.owner != canwu_force_supply_reference::PLUGIN_NAME
        || consumption.status != canwu_resource::ConsumptionStatus::Settled
    {
        return Err(invalid(
            "force externality consumption differs from its exact authoritative binding",
        ));
    }
    let account = resource_state
        .accounts
        .get(&allocation.account)
        .ok_or_else(|| invalid("force externality allocation account is unavailable"))?;
    let consumed_account_revision = exact
        .account_revision
        .get()
        .checked_add(1)
        .ok_or_else(|| invalid("force externality account revision overflowed"))?;
    if account.revision.get() < consumed_account_revision
        || account.resource_revision != allocation.resource_revision
        || account.unit_revision != allocation.unit_revision
    {
        return Err(invalid(
            "force externality allocation account differs from its exact binding",
        ));
    }
    resource_state
        .definitions
        .get(&account.resource_revision)
        .map(|definition| definition.scope.clone())
        .ok_or_else(|| invalid("force externality resource definition is unavailable"))
}

fn acknowledge_force_externalities(
    view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let Some(force_record) = view
        .typed_domain_record(&force_supply_runtime_reference())?
        .cloned()
    else {
        return Ok(BoundaryProposal::default());
    };
    let force_state = force_record.decode_payload::<ForceSupplyRuntimeRecord>()?;
    let mut sagas: Vec<_> = force_state.sagas.values().collect();
    sagas.sort_by(|left, right| left.id.cmp(&right.id));
    let mut directives = Vec::new();
    for saga in sagas {
        let Some(intent_id) = &saga.externality_intent else {
            continue;
        };
        if saga.externality_outcome_source.is_some() {
            continue;
        }
        let outcome_id = ExternalityOutcomeId::new(format!(
            "canwu.economy-reference:externality-outcome:{}",
            intent_id.as_str()
        ))?;
        let reference = force_externality_outcome_reference(&outcome_id).into_untyped();
        let Some(authoritative_outcome) = view.proposed_domain_record_version(&reference)? else {
            continue;
        };
        let acquisition = &force_state.intents[&saga.intent]
            .completion_certificate
            .acquisition;
        let participant_reference =
            force_externality_completion_participant_reference(acquisition).into_untyped();
        let authoritative_participant = view
            .current_domain_record_version(&participant_reference)?
            .ok_or_else(|| invalid("externality completion participant source is unavailable"))?;
        directives.push(BoundaryDirective::SchedulePluginIngress {
            target_plugin: canwu_force_supply_reference::PLUGIN_NAME.to_owned(),
            after: SimDuration::ZERO,
            packet_type: canwu_force_supply_reference::FORCE_EXTERNALITY_OUTCOME_INGRESS.to_owned(),
            priority: 0,
            payload: encode(&ExternalityOutcomePacketV1 {
                saga: saga.id.clone(),
                authoritative_outcome,
                authoritative_participant,
            })?,
            affected: force_state
                .forces
                .get(&saga.force)
                .map(|force| holder_entity(&force.holder))
                .into_iter()
                .collect(),
        });
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

fn apply_per_mille_delta(value: u16, delta: i16) -> u16 {
    if delta >= 0 {
        value.saturating_add(delta.unsigned_abs()).min(1_000)
    } else {
        value.saturating_sub(delta.unsigned_abs())
    }
}

fn apply_cost_delta(value: u16, delta: i16) -> u16 {
    if delta <= 0 {
        value.saturating_add(delta.unsigned_abs()).min(1_000)
    } else {
        value.saturating_sub(delta.unsigned_abs())
    }
}

fn validate_economy_candidate(
    view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    if let Some(record) = view.typed_domain_record(&economy_reference_runtime_reference())? {
        let state = record.decode_payload::<EconomyReferenceRuntimeRecord>()?;
        state.validate()?;
        if record.version != state.revision || state.draft()?.payload != record.payload {
            return Err(invalid(
                "economy-reference candidate record differs from its canonical state",
            ));
        }
    }
    Ok(BoundaryProposal::default())
}

fn economy_state_key() -> StateKey {
    StateKey::new(PLUGIN_NAMESPACE, "runtime")
}

fn externality_outcome_state_key() -> StateKey {
    StateKey::new(
        canwu_force_supply_reference::PLUGIN_NAMESPACE,
        "externality-outcome",
    )
}

fn externality_participant_state_key() -> StateKey {
    StateKey::new(
        canwu_force_supply_reference::PLUGIN_NAMESPACE,
        "externality-completion-participant",
    )
}

fn require_holder_authority(
    context: &CommandContext,
    holder: &canwu_api::KnowledgeHolderRef,
) -> Result<(), CanwuError> {
    let authorized = match holder {
        canwu_api::KnowledgeHolderRef::Person(person) => {
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
        canwu_api::KnowledgeHolderRef::Entity(entity) => {
            context.authority.command_subject.as_ref() == Some(entity)
                && context.decision_controller_id.is_some()
        }
    };
    if authorized {
        Ok(())
    } else {
        Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "economy-reference command issuer is not authorized for its holder",
        ))
    }
}

fn holder_entity(holder: &canwu_api::KnowledgeHolderRef) -> canwu_api::EntityRef {
    match holder {
        canwu_api::KnowledgeHolderRef::Person(person) => canwu_api::EntityRef::Person(*person),
        canwu_api::KnowledgeHolderRef::Entity(entity) => entity.clone(),
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
            | ErrorCode::EvidenceUnavailable
            | ErrorCode::QueryBudgetExceeded
    )
}

pub fn enqueue_economy_archive(
    canwu: &mut Canwu,
    prepared: &crate::PreparedEconomyArchiveBatchV1,
    store: &dyn canwu_force_supply_reference::PackageArchiveStore,
) -> Result<EconomyArchiveIngressReceiptV1, CanwuError> {
    let state = canwu
        .typed_domain_record(&economy_reference_runtime_reference())
        .ok_or_else(|| invalid("economy runtime is unavailable"))?
        .decode_payload::<EconomyReferenceRuntimeRecord>()?;
    if state.prepare_economy_archive(prepared.selected.len())? != *prepared {
        return Err(invalid("economy archive batch is stale or non-canonical"));
    }
    let commit = prepared.store_and_verify(crate::ECONOMY_ARCHIVE_DOMAIN, store)?;
    let permit = ECONOMY_ARCHIVE_PERMIT
        .get()
        .ok_or_else(|| invalid("economy archive ingress is not registered"))?;
    let mut durable = commit.retention.clone();
    durable.phase = canwu_force_supply_reference::PackageArchiveRetentionPhaseV1::DurableIngress;
    durable.semantic_digest.clear();
    durable.semantic_digest =
        canonical_hash("canwu.economy-reference.archive-retention.v1", &durable)?;
    store.persist_package_archive_retention(&durable)?;
    let ingress = canwu.enqueue_permitted_plugin_ingress(
        PluginIngressRequest::new(
            PLUGIN_NAME,
            ECONOMY_ARCHIVE_COMMIT_INGRESS,
            canwu.time(),
            serde_json::to_value(&commit).map_err(|error| invalid(error.to_string()))?,
        )
        .with_archive_retention([PluginArchiveRetention {
            namespace: crate::ECONOMY_ARCHIVE_INDEX_DIRECTORY_NAMESPACE.to_owned(),
            object_id: commit.directory_root.clone(),
        }]),
        permit,
    )?;
    Ok(EconomyArchiveIngressReceiptV1 {
        ingress,
        retention_handle_id: commit.retention.id,
        directory_root: commit.directory_root,
    })
}

pub fn finalize_economy_archive_retention(
    canwu: &mut Canwu,
    store: &dyn canwu_force_supply_reference::PackageArchiveStore,
    ingress: &EconomyArchiveIngressReceiptV1,
) -> Result<canwu_api::IngressReceipt, CanwuError> {
    let state = canwu
        .typed_domain_record(&economy_reference_runtime_reference())
        .ok_or_else(|| invalid("economy runtime is unavailable"))?
        .decode_payload::<EconomyReferenceRuntimeRecord>()?;
    let receipt = state
        .archive_maintenance_receipts
        .values()
        .find(|receipt| {
            receipt.retention_handle_id == ingress.retention_handle_id
                && receipt.directory_root == ingress.directory_root
        })
        .cloned()
        .ok_or_else(|| invalid("economy archive terminal disposition is unavailable"))?;
    let phase = match receipt.disposition {
        canwu_force_supply_reference::PackageArchiveMaintenanceDispositionV1::Applied => {
            canwu_force_supply_reference::PackageArchiveRetentionPhaseV1::Committed
        }
        canwu_force_supply_reference::PackageArchiveMaintenanceDispositionV1::RejectedStale => {
            canwu_force_supply_reference::PackageArchiveRetentionPhaseV1::RejectedStale
        }
    };
    let stored = store
        .load_package_archive_retention(&ingress.retention_handle_id)?
        .ok_or_else(|| invalid("economy archive retention handle is unavailable"))?;
    let finalized = canwu_force_supply_reference::sealed_archive_retention(
        crate::ECONOMY_ARCHIVE_DOMAIN,
        canwu_force_supply_reference::PackageArchiveRetentionHandleV1 { phase, ..stored },
    )?;
    store.finalize_package_archive_retention(&finalized)?;
    let permit = ECONOMY_ARCHIVE_ACK_PERMIT
        .get()
        .ok_or_else(|| invalid("economy archive acknowledgement is not registered"))?;
    canwu.enqueue_permitted_plugin_ingress(
        PluginIngressRequest::new(
            PLUGIN_NAME,
            ECONOMY_ARCHIVE_RETENTION_ACK_INGRESS,
            canwu.time(),
            serde_json::to_value(EconomyArchiveRetentionAcknowledgementV1 { receipt })
                .map_err(|error| invalid(error.to_string()))?,
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
        ErrorCode::EvidenceUnavailable => "evidence_unavailable",
        ErrorCode::QueryBudgetExceeded => "query_budget_exceeded",
        _ => "economy_operation_rejected",
    }
}

fn missing(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::DomainRecordNotFound, message)
}

fn invalid(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidDomainRecord, message)
}

fn decode<T: serde::de::DeserializeOwned>(value: &Value, label: &str) -> Result<T, CanwuError> {
    serde_json::from_value(value.clone()).map_err(|error| {
        CanwuError::new(
            ErrorCode::InvalidPayload,
            format!("{label} could not be decoded: {error}"),
        )
    })
}

fn encode<T: Serialize>(value: &T) -> Result<Value, CanwuError> {
    serde_json::to_value(value).map_err(|error| {
        CanwuError::new(
            ErrorCode::InvalidPayload,
            format!("economy-reference payload could not be encoded: {error}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use canwu_api::{KnowledgeHolderRef, PersonId};
    use std::collections::BTreeSet;

    #[test]
    fn scope_manager_can_publish_for_an_independently_granted_observer() {
        let manager = KnowledgeHolderRef::Person(PersonId::new(1));
        let observer = KnowledgeHolderRef::Person(PersonId::new(2));
        let scope = canwu_resource::ResourceScopeId::new(
            "canwu.economy-reference:scope:independent-observer",
        )
        .expect("scope");
        let mut state = EconomyReferenceStateV1::default();
        state.local_economies.insert(
            crate::LocalEconomyId::new("canwu.economy-reference:economy:observer-test")
                .expect("economy"),
            LocalEconomyV1 {
                id: crate::LocalEconomyId::new("canwu.economy-reference:economy:observer-test")
                    .expect("economy"),
                revision: 1,
                manager: manager.clone(),
                scope: scope.clone(),
                profile: crate::EconomyProfileId::new(
                    "canwu.economy-reference:profile:observer-test",
                )
                .expect("profile"),
                month: 0,
                population_wellbeing_per_mille: 1_000,
                cooperation_per_mille: 1_000,
                pending_harvest_penalty_per_mille: 0,
                latest_decision: crate::GrainDecision::Balanced,
            },
        );
        let grant = crate::EconomyObservationGrantV1 {
            id: crate::EconomyObservationGrantId::new(
                "canwu.economy-reference:observation-grant:observer-test",
            )
            .expect("grant"),
            holder: observer.clone(),
            scopes: BTreeSet::from([scope.clone()]),
            delay_minutes: 0,
            confidence_per_mille: 900,
        };
        state.observation_grant_by_holder_scope.insert(
            crate::holder_scope_index_key(&observer, &scope).expect("key"),
            grant.id.clone(),
        );
        state.observation_grants.insert(grant.id.clone(), grant);

        require_observation_grant(&state, &manager, &scope, &observer)
            .expect("manager-authenticated publication");
        assert!(require_observation_grant(&state, &observer, &scope, &observer).is_err());
    }
}
