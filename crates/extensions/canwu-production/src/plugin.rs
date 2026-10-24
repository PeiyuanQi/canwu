use crate::model::{
    ProductionBatchEnvelopeV1, ProductionCommandEnvelope, ProductionOperationDisposition,
    ProductionOperationOutcome, ProductionOutputAcknowledgement, ProductionRuntimeRecord,
    ProductionScopeId, ProductionState, invalid, production_root_reference,
};
use crate::root::{
    ProductionBoundaryActivityRecord, ProductionBoundaryActivityV1, ProductionRootActivityV1,
    RootKey, active_roots, load_root, payload_scope, production_boundary_activity_reference,
    production_root_kind, root_of_record, with_scope, without_scope,
};
use crate::{PLUGIN_NAME, PLUGIN_NAMESPACE};
use canwu_api::{
    ArchiveReachabilityManifest, BoundaryContext, BoundaryDirective, BoundaryPhase,
    BoundaryProposal, BoundarySystemContract, CanwuError, CauseRef, Command, CommandContext,
    CommandIngress, DecisionAction, DecisionTicketState, DomainRecord, DomainRecordMutation,
    DomainRecordSchema, ErrorCode, EvidenceRef, IngressClass, IngressId, IngressPayload, Issuer,
    KnowledgeHistoryView, KnowledgeHolderRef, KnowledgeOrigin, KnowledgeQuery,
    KnowledgeRecordDraft, KnowledgeRecordKind, KnowledgeSchemaId, KnowledgeSubject,
    KnowledgeSubjectSchema, KnowledgeSubjectTarget, KnowledgeSubjectTargetKind,
    KnowledgeWriteGrant, MAX_KNOWLEDGE_PAGE_SIZE, PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD,
    PayloadSchema, PluginActionDescriptor, PluginArchiveObjectProvider, PluginArchiveRetention,
    PluginIngressDescriptor, PluginIngressPermit, PluginIngressRequest, PluginIngressTarget,
    PluginKnowledgeSchema, PluginRegistrar, RandomOperationTarget, RandomStreamKey, SimDuration,
    SimTime, SimulationPlugin, SimulationView, StateKey, StateVisibility, SystemCadence,
    SystemDirective, canonical_hash, payload_required_evidence_continuation_property_v1,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::OnceLock,
};

pub const PRODUCTION_COMMAND: &str = "apply_production_operation_v1";
/// The tracked command that carries a [`ProductionBatchEnvelopeV1`]: many
/// operations of one holder against one production root, settled in one
/// boundary with one outcome per item.
pub const PRODUCTION_BATCH_COMMAND: &str = "apply_production_batch_v1";
/// Event of a production packet that settles nothing because it does not
/// decode, does not name an existing production root, does not match an
/// admitted tracked production command, or fails production's checks.
const INGRESS_REFUSED_EVENT: &str = "canwu.production.ingress_refused.v1";
/// Event of an output acknowledgement that settles nothing.
const OUTPUT_ACK_REFUSED_EVENT: &str = "canwu.production.output_ack_refused.v1";
/// Event of a settled production batch, with its item counts.
const BATCH_SETTLED_EVENT: &str = "canwu.production.batch_settled.v1";
const PRODUCTION_COMMAND_INGRESS: &str = "production_operation_v1";
const PRODUCTION_BATCH_INGRESS: &str = "production_batch_v1";
pub const PRODUCTION_OUTPUT_ACK_INGRESS: &str = "production_output_ack_v1";
pub const PRODUCTION_COMPLETION_INGRESS: &str = "production_completion_operation_v1";
pub const PRODUCTION_OBSERVATION_WAKE_INGRESS: &str = "production_observation_wake_v1";
pub const PRODUCTION_RESOURCE_CONTINUATION_INGRESS: &str = "production_resource_continuation_v1";
pub const PRODUCTION_ARCHIVE_COMMIT_INGRESS: &str = "production_archive_commit_v1";
pub const PRODUCTION_ARCHIVE_RETENTION_ACK_INGRESS: &str = "production_archive_retention_ack_v1";
const APPLY_SYSTEM: &str = "production_lifecycle_apply_v1";
const VALIDATE_SYSTEM: &str = "production_capacity_and_lifecycle_validate_v1";
const INCIDENT_EVALUATION_SYSTEM: &str = "production_incident_candidate_evaluation_v1";
const INCIDENT_COMMIT_AUDIT_SYSTEM: &str = "production_incident_commit_audit_v1";
const OUTPUT_DISPATCH_SYSTEM: &str = "production_output_dispatch_v1";
const REPORT_SYSTEM: &str = "production_holder_report_publish_v1";
const PRODUCTION_REPORT_KNOWLEDGE: &str = "holder_report";
/// BLAKE3, lowercase hex, of `canwu-production 0.20.0 semantics: asset binding
/// and application spec reads declared, cited record kinds checked before
/// reading, pending output sources declared identity-only, report holders that
/// cannot receive knowledge skipped` (one line, one space after each comma and
/// colon).
pub const PRODUCTION_SEMANTIC_HASH: &str =
    "eded50444992d0a08392ecf601ab9b38954d244e65aa8f06f83ed3ef3f8f6d9e";
const REPORT_SCHEMA_HASH: &str = "2e84d66c85841a251a94aa15fa4fd477d29136ed31c8434c5dd61dc92156fdf8";
static PRODUCTION_ARCHIVE_COMMIT_PERMIT: OnceLock<PluginIngressPermit> = OnceLock::new();
static PRODUCTION_ARCHIVE_ACK_PERMIT: OnceLock<PluginIngressPermit> = OnceLock::new();
static PRODUCTION_COMPLETION_INGRESS_PERMIT: OnceLock<PluginIngressPermit> = OnceLock::new();
static PRODUCTION_RESOURCE_CONTINUATION_PERMIT: OnceLock<PluginIngressPermit> = OnceLock::new();

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProductionArchiveIngressReceiptV1 {
    pub ingress: canwu_api::IngressReceipt,
    /// The production root the commit applies to.
    pub scope: Option<ProductionScopeId>,
    pub retention_handle_id: String,
    pub directory_root: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct ProductionArchiveRetentionAcknowledgementV1 {
    handle_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scope: Option<ProductionScopeId>,
}

#[must_use]
pub fn production_incident_random_stream() -> RandomStreamKey {
    RandomStreamKey::new(PLUGIN_NAME, "facility-incident", 1)
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct AdmittedProductionOperation {
    envelope: ProductionCommandEnvelope,
    command: canwu_api::CommandId,
    input_hash: String,
    decision_receipt: Option<crate::ProductionDecisionReceiptV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct AdmittedProductionBatch {
    batch: ProductionBatchEnvelopeV1,
    command: canwu_api::CommandId,
    input_hash: String,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ProductionPlugin;

fn production_runtime_state_key() -> StateKey {
    DomainRecordSchema::for_record::<ProductionRuntimeRecord>().state_key()
}

fn production_activity_state_key() -> StateKey {
    DomainRecordSchema::for_record::<ProductionBoundaryActivityRecord>().state_key()
}

fn command_reads() -> Vec<StateKey> {
    let mut reads = vec![StateKey::core_decisions(), production_runtime_state_key()];
    reads.extend(production_external_state_keys());
    reads.sort();
    reads.dedup();
    reads
}

#[must_use]
pub fn production_command_descriptor() -> PluginActionDescriptor {
    PluginActionDescriptor {
        name: PRODUCTION_COMMAND.to_owned(),
        description: "Submit one authority-checked production lifecycle operation".to_owned(),
        payload_schema: PayloadSchema::Any,
        reads: command_reads(),
        writes: Vec::new(),
    }
}

/// The descriptor of [`PRODUCTION_BATCH_COMMAND`].
#[must_use]
pub fn production_batch_command_descriptor() -> PluginActionDescriptor {
    PluginActionDescriptor {
        name: PRODUCTION_BATCH_COMMAND.to_owned(),
        description: "Submit many authority-checked production operations of one holder against one production root"
            .to_owned(),
        payload_schema: PayloadSchema::Any,
        reads: command_reads(),
        writes: Vec::new(),
    }
}

fn production_external_state_keys() -> Vec<StateKey> {
    vec![
        // Realization evidence is an application record of any kind; its
        // exact version is validated against current or retained evidence.
        StateKey::core_domain_records(),
        StateKey::core_evidence(),
        DomainRecordSchema::for_record::<canwu_resource::ResourceRuntimeRecord>().state_key(),
        canwu_resource::resource_hot_terminal_state_key(),
        DomainRecordSchema::for_record::<canwu_technology::TechniqueRevision>().state_key(),
        DomainRecordSchema::for_record::<canwu_technology::CapabilityQualification>().state_key(),
        DomainRecordSchema::for_record::<canwu_technology::ImplementationRecord>().state_key(),
        DomainRecordSchema::for_record::<canwu_technology::AdoptionRecord>().state_key(),
        DomainRecordSchema::for_record::<canwu_technology::ExperimentAttempt>().state_key(),
        DomainRecordSchema::for_record::<canwu_technology::AttemptObservation>().state_key(),
        DomainRecordSchema::for_record::<canwu_technology::AssetBinding>().state_key(),
        DomainRecordSchema::for_record::<canwu_technology::ApplicationSpec>().state_key(),
    ]
}

#[must_use]
pub fn production_output_ack_ingress_descriptor() -> PluginIngressDescriptor {
    PluginIngressDescriptor {
        name: PRODUCTION_OUTPUT_ACK_INGRESS.to_owned(),
        description: "Acknowledge one exact terminal canwu-resource output outcome".to_owned(),
        class: IngressClass::Acknowledgement,
        payload_schema: PayloadSchema::Any,
    }
}

#[must_use]
pub fn production_report_knowledge_schema_id() -> KnowledgeSchemaId {
    KnowledgeSchemaId::new(
        KnowledgeRecordKind::new(PLUGIN_NAMESPACE, PRODUCTION_REPORT_KNOWLEDGE),
        1,
    )
}

fn register_permit(
    cell: &OnceLock<PluginIngressPermit>,
    permit: &PluginIngressPermit,
    label: &str,
) -> Result<(), CanwuError> {
    if cell.set(permit.clone()).is_err() && cell.get() != Some(permit) {
        return Err(CanwuError::new(
            ErrorCode::InvalidPluginRegistration,
            format!("production {label} permit changed across registrations"),
        ));
    }
    Ok(())
}

impl SimulationPlugin for ProductionPlugin {
    fn name(&self) -> &'static str {
        PLUGIN_NAME
    }

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    fn semantic_hash(&self) -> &'static str {
        PRODUCTION_SEMANTIC_HASH
    }

    fn validate_activation(&self, records: &[DomainRecord]) -> Result<(), CanwuError> {
        let mut roots = BTreeSet::new();
        for record in records {
            if record
                .reference
                .kind
                .matches_type::<ProductionBoundaryActivityRecord>()
            {
                if record.reference != production_boundary_activity_reference().into_untyped() {
                    return Err(invalid(
                        "production activation contains a misidentified boundary activity record",
                    ));
                }
                record.decode_payload::<ProductionBoundaryActivityRecord>()?;
                continue;
            }
            if !record
                .reference
                .kind
                .matches_type::<ProductionRuntimeRecord>()
            {
                continue;
            }
            let state = record.decode_payload::<ProductionRuntimeRecord>()?;
            if record.reference != state.runtime_reference().into_untyped()
                || root_of_record(&record.reference).as_ref() != Some(&state.scope)
                || !roots.insert(state.scope.clone())
            {
                return Err(invalid(
                    "production activation contains a duplicated or misidentified runtime root",
                ));
            }
            state.validate()?;
            if state.draft()?.payload != record.payload {
                return Err(invalid(
                    "production activation root is not canonically encoded",
                ));
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut runtime_schema = DomainRecordSchema::for_record::<ProductionRuntimeRecord>();
        runtime_schema.payload_schema = PayloadSchema::Object {
            properties: std::collections::BTreeMap::from([
                (
                    PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD.to_owned(),
                    payload_required_evidence_continuation_property_v1(),
                ),
                (
                    canwu_api::IDENTITY_EVIDENCE_DEPENDENCIES_FIELD.to_owned(),
                    canwu_api::identity_evidence_dependencies_property_v1(),
                ),
            ]),
            allow_additional: true,
        };
        registrar.register_record_schema(runtime_schema)?;
        registrar.register_record_schema(DomainRecordSchema::for_record::<
            ProductionBoundaryActivityRecord,
        >())?;
        let report_schema = PluginKnowledgeSchema {
            id: production_report_knowledge_schema_id(),
            schema_hash: REPORT_SCHEMA_HASH.to_owned(),
            writable: true,
            payload_schema: PayloadSchema::Any,
            subjects: vec![KnowledgeSubjectSchema {
                role: "site".to_owned(),
                targets: vec![KnowledgeSubjectTargetKind::AnyEntity],
                required: true,
                multiple: false,
            }],
            retention: canwu_api::KnowledgeRetention::Keep,
        };
        registrar.register_knowledge_schema(report_schema.clone())?;
        registrar.register_archive_reachability_participant(production_archive_reachability)?;
        registrar.register_command(production_command_descriptor(), admit_production_operation)?;
        registrar.register_command(
            production_batch_command_descriptor(),
            admit_production_batch,
        )?;
        registrar.register_ingress(PluginIngressDescriptor {
            name: PRODUCTION_COMMAND_INGRESS.to_owned(),
            description: "Apply one admitted production lifecycle operation".to_owned(),
            class: IngressClass::Decision,
            payload_schema: PayloadSchema::Any,
        })?;
        registrar.register_ingress(PluginIngressDescriptor {
            name: PRODUCTION_BATCH_INGRESS.to_owned(),
            description: "Apply one admitted production batch, item by item".to_owned(),
            class: IngressClass::Decision,
            payload_schema: PayloadSchema::Any,
        })?;
        registrar.register_ingress(production_output_ack_ingress_descriptor())?;
        let completion_permit = registrar.register_internal_ingress(PluginIngressDescriptor {
            name: PRODUCTION_COMPLETION_INGRESS.to_owned(),
            description: "Apply one package-owned production completion coordinator transition"
                .to_owned(),
            class: IngressClass::ScheduledSystem,
            payload_schema: PayloadSchema::Any,
        })?;
        register_permit(
            &PRODUCTION_COMPLETION_INGRESS_PERMIT,
            &completion_permit,
            "completion ingress",
        )?;
        let continuation_permit = registrar.register_internal_ingress(PluginIngressDescriptor {
            name: PRODUCTION_RESOURCE_CONTINUATION_INGRESS.to_owned(),
            description: "Persist one provider-authenticated resource archive continuation witness"
                .to_owned(),
            class: IngressClass::Acknowledgement,
            payload_schema: PayloadSchema::Any,
        })?;
        register_permit(
            &PRODUCTION_RESOURCE_CONTINUATION_PERMIT,
            &continuation_permit,
            "resource continuation ingress",
        )?;
        let archive_commit_permit = registrar.register_internal_ingress_with_archive_retention(
            PluginIngressDescriptor {
                name: PRODUCTION_ARCHIVE_COMMIT_INGRESS.to_owned(),
                description: "Commit one provider-verified production terminal archive batch"
                    .to_owned(),
                class: IngressClass::ScheduledSystem,
                payload_schema: PayloadSchema::Any,
            },
            crate::PRODUCTION_ARCHIVE_INDEX_DIRECTORY_NAMESPACE,
            vec!["/commit/directory_root".to_owned()],
        )?;
        register_permit(
            &PRODUCTION_ARCHIVE_COMMIT_PERMIT,
            &archive_commit_permit,
            "archive commit",
        )?;
        let archive_ack_permit = registrar.register_internal_ingress(PluginIngressDescriptor {
            name: PRODUCTION_ARCHIVE_RETENTION_ACK_INGRESS.to_owned(),
            description: "Acknowledge terminal provider-side production archive retention"
                .to_owned(),
            class: IngressClass::Acknowledgement,
            payload_schema: PayloadSchema::Any,
        })?;
        register_permit(
            &PRODUCTION_ARCHIVE_ACK_PERMIT,
            &archive_ack_permit,
            "archive retention acknowledgement",
        )?;
        registrar.register_ingress(PluginIngressDescriptor {
            name: PRODUCTION_OBSERVATION_WAKE_INGRESS.to_owned(),
            description:
                "Deliver only production observation heads whose persisted delay has elapsed"
                    .to_owned(),
            class: IngressClass::ScheduledSystem,
            payload_schema: PayloadSchema::Any,
        })?;

        let runtime = production_runtime_state_key();
        let activity = production_activity_state_key();
        let mut apply = BoundarySystemContract::new(
            APPLY_SYSTEM,
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        apply.reads = vec![
            StateKey::core_commands(),
            StateKey::core_ingress(),
            runtime.clone(),
            activity.clone(),
        ];
        apply.reads.extend(production_external_state_keys());
        apply.reads.sort();
        apply.reads.dedup();
        apply.writes = vec![runtime.clone(), activity.clone()];
        apply.plugin_ingress_targets = vec![PluginIngressTarget {
            target_plugin: canwu_resource::PLUGIN_NAME.to_owned(),
            packet_type: canwu_resource::RESOURCE_COMPLETION_INGRESS.to_owned(),
        }];
        apply.emits = vec![
            "canwu.production.operation_applied.v1".to_owned(),
            "canwu.production.operation_rejected.v1".to_owned(),
            "canwu.production.output_settled.v1".to_owned(),
            OUTPUT_ACK_REFUSED_EVENT.to_owned(),
            INGRESS_REFUSED_EVENT.to_owned(),
            BATCH_SETTLED_EVENT.to_owned(),
            "canwu.production.archive_maintenance.v1".to_owned(),
        ];
        apply.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(apply, apply_production_ingress)?;

        let mut validate = BoundarySystemContract::new(
            VALIDATE_SYSTEM,
            BoundaryPhase::InvariantValidation,
            SystemCadence::EventDriven,
        );
        validate.reads = vec![runtime.clone(), activity.clone()];
        validate.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(validate, validate_production_candidate)?;

        let mut incidents = BoundarySystemContract::new(
            INCIDENT_EVALUATION_SYSTEM,
            BoundaryPhase::HistoricalCandidateEvaluation,
            SystemCadence::EventDriven,
        );
        incidents.reads = vec![
            StateKey::core_evidence(),
            StateKey::core_ingress(),
            runtime.clone(),
            activity.clone(),
        ];
        incidents.writes = vec![runtime.clone()];
        incidents.emits = vec!["canwu.production.incident_committed.v1".to_owned()];
        incidents.random_streams = vec![production_incident_random_stream()];
        incidents.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(incidents, evaluate_incident_candidates)?;

        let mut incident_commit_audit = BoundarySystemContract::new(
            INCIDENT_COMMIT_AUDIT_SYSTEM,
            BoundaryPhase::ConditionalTransitionCommit,
            SystemCadence::EventDriven,
        );
        incident_commit_audit.reads = vec![runtime.clone(), activity.clone()];
        incident_commit_audit.visibility = StateVisibility::SameBoundary;
        registrar
            .register_boundary_system(incident_commit_audit, audit_incident_transition_commit)?;

        let mut output_dispatch = BoundarySystemContract::new(
            OUTPUT_DISPATCH_SYSTEM,
            BoundaryPhase::StrategicAggregation,
            SystemCadence::EventDriven,
        );
        output_dispatch.reads = vec![StateKey::core_ingress(), runtime.clone(), activity.clone()];
        output_dispatch.writes = vec![runtime.clone()];
        output_dispatch.plugin_ingress_targets = vec![PluginIngressTarget {
            target_plugin: canwu_resource::PLUGIN_NAME.to_owned(),
            packet_type: canwu_resource::RESOURCE_PRODUCTION_OUTPUT_BATCH_INGRESS.to_owned(),
        }];
        output_dispatch.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(output_dispatch, dispatch_pending_outputs)?;

        let mut reports = BoundarySystemContract::new(
            REPORT_SYSTEM,
            BoundaryPhase::PerspectiveAndReportMaterialization,
            SystemCadence::EventDriven,
        );
        reports.reads = vec![
            StateKey::core_ingress(),
            StateKey::core_knowledge(),
            runtime.clone(),
            activity,
        ];
        reports.writes = vec![runtime];
        reports.plugin_ingress_targets = vec![PluginIngressTarget {
            target_plugin: PLUGIN_NAME.to_owned(),
            packet_type: PRODUCTION_OBSERVATION_WAKE_INGRESS.to_owned(),
        }];
        reports.knowledge_writes = vec![KnowledgeWriteGrant {
            schema: report_schema.id,
            visibilities: vec![StateVisibility::SameBoundary],
        }];
        reports.emits = vec![
            "canwu.production.report_capacity_rejected.v1".to_owned(),
            INGRESS_REFUSED_EVENT.to_owned(),
        ];
        reports.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(reports, publish_holder_reports)
    }
}

/// The root a command names, decoded and validated. A command that names no
/// installed root is a recorded command rejection (`InvalidPayload`), never
/// a failed boundary.
fn admission_root(
    view: &SimulationView<'_>,
    scope: Option<&ProductionScopeId>,
) -> Result<ProductionState, CanwuError> {
    load_root(view, scope)?
        .map(|(_, state)| state)
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidPayload,
                "production runtime is not configured for the named root",
            )
        })
}

fn admit_production_operation(
    view: &SimulationView<'_>,
    context: &CommandContext,
    payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    if context.ingress == CommandIngress::LegacyDirect {
        return Err(CanwuError::new(
            ErrorCode::MixedCommandIngress,
            "production operations require tracked canonical command ingress",
        ));
    }
    let envelope: ProductionCommandEnvelope = decode(payload, "production operation")?;
    require_canonical_payload(&envelope, payload, "production operation")?;
    require_holder_authority(context, &envelope.holder)?;
    let state = admission_root(view, envelope.scope.as_ref())?;
    let input_hash = canonical_hash("canwu.production.operation-input.v1", &envelope)?;
    if let Some(existing) = state.operation_outcomes.get(&envelope.operation_id) {
        if existing.canonical_input_hash == input_hash {
            return Ok(Vec::new());
        }
        return Err(CanwuError::new(
            ErrorCode::IdempotencyConflict,
            "production operation ID was reused with different input",
        ));
    }
    state
        .ensure_operation_outcome_admission_capacity(&envelope.operation)
        .map_err(admission_rejection)?;
    if state
        .project_operation_uses_reserved_outcome_at_capacity(&envelope.operation)
        .map_err(admission_rejection)?
    {
        validate_external_operation_evidence(view, &state, &envelope.holder, &envelope.operation)
            .map_err(admission_rejection)?;
        let mut candidate = state.clone();
        candidate
            .apply_operation(&envelope, context.simulation_time)
            .map_err(admission_rejection)?;
        candidate.validate().map_err(admission_rejection)?;
    }
    let decision_receipt = validate_degraded_decision(view, context, &state, &envelope)
        .map_err(admission_rejection)?;
    Ok(vec![SystemDirective::EnqueuePluginIngress {
        after: SimDuration::ZERO,
        packet_type: PRODUCTION_COMMAND_INGRESS.to_owned(),
        priority: 0,
        payload: serde_json::to_value(AdmittedProductionOperation {
            envelope,
            command: context.command_id,
            input_hash,
            decision_receipt,
        })
        .map_err(|error| encode_error(&error))?,
        affected: Vec::new(),
    }])
}

/// Refuses a command payload that is not the canonical encoding of what it
/// decodes to: an explicit `null` or unknown member would be admitted but
/// would not match its packet at settlement.
fn require_canonical_payload<T: Serialize>(
    decoded: &T,
    payload: &Value,
    label: &str,
) -> Result<(), CanwuError> {
    if serde_json::to_value(decoded).map_err(|error| encode_error(&error))? != *payload {
        return Err(CanwuError::new(
            ErrorCode::InvalidPayload,
            format!("{label} payload is not its canonical encoding"),
        ));
    }
    Ok(())
}

/// Whether `existing` is the outcome of `item` of `batch`: the same operation
/// of the same holder against the same root.
fn batch_item_settled(
    existing: &ProductionOperationOutcome,
    batch: &ProductionBatchEnvelopeV1,
    item: &crate::ProductionBatchItemV1,
) -> bool {
    existing.command.holder == batch.holder
        && existing.command.scope == batch.scope
        && existing.command.operation == item.operation
}

/// Admits one production batch: the issuer's authority over its holder once,
/// its shape, and outcome room for every item not settled yet. Each item is
/// otherwise checked when it settles.
fn admit_production_batch(
    view: &SimulationView<'_>,
    context: &CommandContext,
    payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    if context.ingress == CommandIngress::LegacyDirect {
        return Err(CanwuError::new(
            ErrorCode::MixedCommandIngress,
            "production batches require tracked canonical command ingress",
        ));
    }
    let batch: ProductionBatchEnvelopeV1 = decode(payload, "production batch")?;
    require_canonical_payload(&batch, payload, "production batch")?;
    require_holder_authority(context, &batch.holder)?;
    let state = admission_root(view, batch.scope.as_ref())?;
    if batch.items.is_empty() || batch.items.len() > state.limits.max_batch_items {
        return Err(CanwuError::new(
            ErrorCode::ValueOutOfRange,
            "production batch must carry between one item and the root's batch item bound",
        ));
    }
    let mut operation_ids = BTreeSet::new();
    for item in &batch.items {
        if !operation_ids.insert(&item.operation_id) {
            return Err(CanwuError::new(
                ErrorCode::InvalidPayload,
                "production batch names one operation ID twice",
            ));
        }
        if matches!(
            item.operation,
            crate::ProductionOperation::ResolveDegradedFacility { .. }
        ) {
            return Err(CanwuError::new(
                ErrorCode::InvalidPayload,
                "a degraded-facility choice is executed by its own decision, not in a batch",
            ));
        }
    }
    let mut fresh = Vec::new();
    for item in &batch.items {
        match state.operation_outcomes.get(&item.operation_id) {
            Some(existing) if batch_item_settled(existing, &batch, item) => {}
            Some(_) => {
                return Err(CanwuError::new(
                    ErrorCode::IdempotencyConflict,
                    "production batch reuses an operation ID with different input",
                ));
            }
            None => fresh.push(&item.operation),
        }
    }
    if fresh.is_empty() {
        return Ok(Vec::new());
    }
    state
        .ensure_operations_outcome_admission_capacity(&fresh)
        .map_err(admission_rejection)?;
    for operation in &fresh {
        if state
            .project_operation_uses_reserved_outcome_at_capacity(operation)
            .map_err(admission_rejection)?
        {
            return Err(CanwuError::new(
                ErrorCode::ValueOutOfRange,
                "archive_backpressure: a facility-project step that uses its reserved outcome while the outcome table is full is sent as its own command",
            ));
        }
    }
    let input_hash = canonical_hash("canwu.production.batch-input.v1", &batch)?;
    Ok(vec![SystemDirective::EnqueuePluginIngress {
        after: SimDuration::ZERO,
        packet_type: PRODUCTION_BATCH_INGRESS.to_owned(),
        priority: 0,
        payload: serde_json::to_value(AdmittedProductionBatch {
            batch,
            command: context.command_id,
            input_hash,
        })
        .map_err(|error| encode_error(&error))?,
        affected: Vec::new(),
    }])
}

/// The refusal event of `packet_type`.
fn refusal_event(packet_type: &str) -> &'static str {
    if packet_type == PRODUCTION_OUTPUT_ACK_INGRESS {
        OUTPUT_ACK_REFUSED_EVENT
    } else {
        INGRESS_REFUSED_EVENT
    }
}

/// The production root a packet belongs to.
///
/// Most packets name it in their payload; an output acknowledgement by the
/// production source its execution pinned; and a resource participant's
/// completion acknowledgement, which the resource plugin sends without a
/// scope, by the coordinator source of the participant it acknowledges. A
/// packet whose root cannot be read this way is refused.
fn route_packet(
    view: &SimulationView<'_>,
    packet_type: &str,
    payload: &Value,
) -> Result<RootKey, CanwuError> {
    let member = |name: &str| payload.get(name);
    match packet_type {
        PRODUCTION_COMMAND_INGRESS => {
            payload_scope(member("envelope").and_then(|envelope| envelope.get("scope")))
        }
        PRODUCTION_BATCH_INGRESS => {
            payload_scope(member("batch").and_then(|batch| batch.get("scope")))
        }
        PRODUCTION_ARCHIVE_COMMIT_INGRESS => {
            payload_scope(member("commit").and_then(|commit| commit.get("scope")))
        }
        PRODUCTION_OUTPUT_ACK_INGRESS => {
            let record: canwu_api::DomainRecordRef = decode(
                &member("production_source")
                    .and_then(|source| source.get("record"))
                    .cloned()
                    .unwrap_or(Value::Null),
                "production output acknowledgement source",
            )?;
            root_of_record(&record).ok_or_else(|| {
                invalid("production output acknowledgement source is not a production root")
            })
        }
        PRODUCTION_COMPLETION_INGRESS
            if member("completion").and_then(Value::as_str)
                == Some("acknowledge_participant_completed") =>
        {
            let acquisition: canwu_resource::CompletionLeaseAcquisitionId = decode(
                &member("request")
                    .and_then(|request| request.get("acquisition"))
                    .cloned()
                    .unwrap_or(Value::Null),
                "production completion acknowledgement acquisition",
            )?;
            completion_participant_root(view, &acquisition)
        }
        PRODUCTION_COMPLETION_INGRESS
        | PRODUCTION_RESOURCE_CONTINUATION_INGRESS
        | PRODUCTION_ARCHIVE_RETENTION_ACK_INGRESS
        | PRODUCTION_OBSERVATION_WAKE_INGRESS => payload_scope(member("scope")),
        _ => Err(invalid(format!(
            "production packet type {packet_type} is not registered"
        ))),
    }
}

/// The production root a resource completion participant was granted for:
/// the record of its coordinator source.
fn completion_participant_root(
    view: &SimulationView<'_>,
    acquisition: &canwu_resource::CompletionLeaseAcquisitionId,
) -> Result<RootKey, CanwuError> {
    let resource = resource_root(
        view,
        [
            canwu_resource::ResourceTerminalRecordKeyV1::ExternalCompletionParticipant(
                acquisition.clone(),
            ),
        ],
        "resource completion runtime is unavailable",
    )?;
    let participant = resource
        .external_completion_participants
        .participant(acquisition)
        .ok_or_else(|| invalid("completed resource participant is unavailable"))?;
    root_of_record(&participant.coordinator_source.record)
        .ok_or_else(|| invalid("resource participant coordinator is not a production root"))
}

/// One production packet admitted to the boundary, routed to its root.
struct RoutedPacket {
    ingress: IngressId,
    packet_type: String,
    payload: Value,
    cause: Option<CauseRef>,
}

/// What settling the packets of one root produced.
struct RootSettlement {
    state: ProductionState,
    changed: bool,
    trigger: Option<IngressId>,
    /// Operations and batch items this boundary recorded an outcome for.
    operations: usize,
    directives: Vec<BoundaryDirective>,
}

impl RootSettlement {
    fn applied(&mut self, ingress: IngressId) {
        self.changed = true;
        self.trigger.get_or_insert(ingress);
    }

    fn refuse(&mut self, packet_type: &str, error: &CanwuError) {
        refuse_packet(
            &mut self.directives,
            refusal_event(packet_type),
            packet_type,
            error,
        );
    }
}

/// Whether an error raised while a production packet is checked or applied
/// is a failed read of a store or an engine invariant, which fails the
/// boundary, rather than a finding about the packet, which refuses it. A
/// boundary that fails for a store read settles once the store serves the
/// object.
fn fails_boundary(error: &CanwuError) -> bool {
    matches!(
        error.code,
        ErrorCode::InvalidArchive
            | ErrorCode::InvalidBoundary
            | ErrorCode::IdentifierExhausted
            | ErrorCode::PluginPanicked
            | ErrorCode::StatePageUnavailable
            | ErrorCode::UndeclaredStateRead
            | ErrorCode::UndeclaredStateWrite
            | ErrorCode::UndeclaredRandomStream
            | ErrorCode::UndeclaredKnowledgeWrite
    )
}

#[allow(clippy::too_many_lines)]
fn apply_production_ingress(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut refusals = Vec::new();
    let mut routed = BTreeMap::<RootKey, (Vec<RoutedPacket>, Vec<RoutedPacket>)>::new();
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
        // Wakes only drive the report system.
        if plugin != PLUGIN_NAME || packet_type == PRODUCTION_OBSERVATION_WAKE_INGRESS {
            continue;
        }
        match route_packet(view, packet_type, payload) {
            Ok(root) => {
                let packet = RoutedPacket {
                    ingress: *ingress_id,
                    packet_type: packet_type.clone(),
                    payload: payload.clone(),
                    cause: ingress.cause.clone(),
                };
                let (commits, others) = routed.entry(root).or_default();
                // Within one root, archive commits settle before the root's
                // other packets, each group in its admitted order: every other
                // packet advances the runtime revision the archive source root
                // binds, so a commit prepared from the root the boundary
                // started from applies even when another packet was admitted
                // ahead of it.
                if packet_type == PRODUCTION_ARCHIVE_COMMIT_INGRESS {
                    commits.push(packet);
                } else {
                    others.push(packet);
                }
            }
            Err(error) if !fails_boundary(&error) => {
                refuse_packet(
                    &mut refusals,
                    refusal_event(packet_type),
                    packet_type,
                    &error,
                );
            }
            Err(error) => return Err(error),
        }
    }
    let mut mutations = Vec::new();
    let mut directives = refusals;
    let mut activity = Vec::new();
    for (root, (commits, others)) in routed {
        let Some((record, state)) = load_root(view, root.as_ref())? else {
            let error = CanwuError::new(
                ErrorCode::DomainRecordNotFound,
                "production packet names a production root that is not installed",
            );
            for packet in commits.iter().chain(&others) {
                refuse_packet(
                    &mut directives,
                    refusal_event(&packet.packet_type),
                    &packet.packet_type,
                    &error,
                );
            }
            continue;
        };
        let mut settlement = RootSettlement {
            state,
            changed: false,
            trigger: None,
            operations: 0,
            directives: Vec::new(),
        };
        for packet in commits.iter().chain(&others) {
            settle_packet(view, context, &mut settlement, packet)?;
        }
        if settlement.changed {
            mutations.push(BoundaryDirective::MutateRecord {
                mutation: DomainRecordMutation::Update {
                    record: settlement.state.draft()?,
                    expected_version: record.version,
                },
                summary: "Apply bounded production lifecycle mutations".to_owned(),
            });
            activity.push(ProductionRootActivityV1 {
                scope: root,
                trigger: settlement.trigger,
            });
        }
        directives.extend(settlement.directives);
    }
    if !activity.is_empty() {
        let activity = ProductionBoundaryActivityV1 {
            boundary: context.boundary_id,
            roots: activity,
        };
        let draft = canwu_api::DomainRecordDraft::from_typed(
            production_boundary_activity_reference(),
            &activity,
        )?;
        let mutation = match view.typed_domain_record(&production_boundary_activity_reference())? {
            Some(existing) => DomainRecordMutation::Update {
                record: draft,
                expected_version: existing.version,
            },
            None => DomainRecordMutation::Create { record: draft },
        };
        mutations.push(BoundaryDirective::MutateRecord {
            mutation,
            summary: "Record the production roots this boundary settled".to_owned(),
        });
    }
    mutations.extend(directives);
    Ok(BoundaryProposal {
        directives: mutations,
        ..BoundaryProposal::default()
    })
}

/// Settles one production packet against its root.
#[allow(clippy::too_many_lines)]
fn settle_packet(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    root: &mut RootSettlement,
    packet: &RoutedPacket,
) -> Result<(), CanwuError> {
    let packet_type = packet.packet_type.as_str();
    match packet_type {
        PRODUCTION_COMMAND_INGRESS => {
            let admitted: AdmittedProductionOperation =
                match decode(&packet.payload, "admitted production operation") {
                    Ok(admitted) => admitted,
                    Err(error) => {
                        root.refuse(packet_type, &error);
                        return Ok(());
                    }
                };
            // The packet type is public, so only a packet that the admission
            // of a tracked production command queued settles: that admission
            // checked the issuer's authority, the run policy, the outcome
            // capacity, and any degraded-facility decision. Any other packet
            // settles nothing.
            if let Err(error) = validate_admitted_command(
                view,
                packet.cause.as_ref(),
                admitted.command,
                PRODUCTION_COMMAND,
                &serde_json::to_value(&admitted.envelope).map_err(|error| encode_error(&error))?,
                &canonical_hash("canwu.production.operation-input.v1", &admitted.envelope)?,
                &admitted.input_hash,
            ) {
                root.refuse(packet_type, &unmatched_command(&error));
                return Ok(());
            }
            if root
                .state
                .operation_outcomes
                .contains_key(&admitted.envelope.operation_id)
            {
                return Ok(());
            }
            let settled = settle_operation(
                view,
                context,
                root,
                &admitted.envelope,
                &admitted.input_hash,
                admitted.decision_receipt.as_ref(),
            )?;
            match settled {
                Settled::Refused(error) => root.refuse(packet_type, &error),
                Settled::Recorded(disposition) => {
                    if disposition == ProductionOperationDisposition::Applied {
                        root.applied(packet.ingress);
                    } else {
                        root.changed = true;
                    }
                    root.directives.push(BoundaryDirective::Emit {
                        event_type: if disposition == ProductionOperationDisposition::Applied {
                            "canwu.production.operation_applied.v1"
                        } else {
                            "canwu.production.operation_rejected.v1"
                        }
                        .to_owned(),
                        summary: format!(
                            "production operation {} reached a terminal outcome",
                            admitted.envelope.operation_id
                        ),
                        affected: Vec::new(),
                    });
                }
            }
        }
        PRODUCTION_BATCH_INGRESS => {
            let admitted: AdmittedProductionBatch =
                match decode(&packet.payload, "admitted production batch") {
                    Ok(admitted) => admitted,
                    Err(error) => {
                        root.refuse(packet_type, &error);
                        return Ok(());
                    }
                };
            if let Err(error) = validate_admitted_command(
                view,
                packet.cause.as_ref(),
                admitted.command,
                PRODUCTION_BATCH_COMMAND,
                &serde_json::to_value(&admitted.batch).map_err(|error| encode_error(&error))?,
                &canonical_hash("canwu.production.batch-input.v1", &admitted.batch)?,
                &admitted.input_hash,
            ) {
                root.refuse(packet_type, &unmatched_command(&error));
                return Ok(());
            }
            settle_batch(view, context, root, packet.ingress, &admitted.batch)?;
        }
        PRODUCTION_OUTPUT_ACK_INGRESS => {
            let acknowledgement: ProductionOutputAcknowledgement =
                match decode(&packet.payload, "production output acknowledgement") {
                    Ok(acknowledgement) => acknowledgement,
                    Err(error) => {
                        root.refuse(packet_type, &error);
                        return Ok(());
                    }
                };
            let mut candidate = root.state.clone();
            // An acknowledgement that does not settle its execution exactly
            // is refused with a trace and changes nothing; it never fails the
            // boundary.
            let accepted = validate_output_ack_source(view, &acknowledgement).and_then(|()| {
                let completion = candidate
                    .executions
                    .get(&acknowledgement.execution)
                    .map(|execution| {
                        (
                            execution.completion_certificate.acquisition.clone(),
                            execution.completion_certificate.operation_key.clone(),
                        )
                    })
                    .ok_or_else(|| invalid("output acknowledgement execution is unavailable"))?;
                candidate
                    .acknowledge_output(&acknowledgement, context.at)
                    .map(|changed| (changed, completion))
            });
            let (settled, completion) = match accepted {
                Ok(accepted) => accepted,
                Err(error) if !fails_boundary(&error) => {
                    root.directives.push(BoundaryDirective::Emit {
                        event_type: OUTPUT_ACK_REFUSED_EVENT.to_owned(),
                        summary: format!(
                            "production output acknowledgement for execution {} was refused: {}",
                            acknowledgement.execution, error.message
                        ),
                        affected: Vec::new(),
                    });
                    return Ok(());
                }
                Err(error) => return Err(error),
            };
            if settled {
                root.state = candidate;
                root.applied(packet.ingress);
                root.directives
                    .push(BoundaryDirective::SchedulePluginIngress {
                    target_plugin: canwu_resource::PLUGIN_NAME.to_owned(),
                    after: SimDuration::ZERO,
                    packet_type: canwu_resource::RESOURCE_COMPLETION_INGRESS.to_owned(),
                    priority: 0,
                    payload: serde_json::to_value(
                        canwu_resource::ResourceCompletionOperationV1::CompleteExternalParticipant(
                            canwu_resource::CompleteExternalCompletionParticipantGrantV1 {
                                acquisition: completion.0,
                                operation_key: completion.1,
                            },
                        ),
                    )
                    .map_err(|error| encode_error(&error))?,
                    affected: Vec::new(),
                });
                root.directives.push(BoundaryDirective::Emit {
                    event_type: "canwu.production.output_settled.v1".to_owned(),
                    summary: format!(
                        "production execution {} settled its resource output",
                        acknowledgement.execution
                    ),
                    affected: Vec::new(),
                });
            }
        }
        PRODUCTION_COMPLETION_INGRESS => {
            let operation: crate::ProductionCompletionIngressV1 = match decode(
                &without_scope(&packet.payload),
                "production completion operation",
            ) {
                Ok(operation) => operation,
                Err(error) => {
                    root.refuse(packet_type, &error);
                    return Ok(());
                }
            };
            // A step that production's checks refuse settles nothing and is
            // recorded, whoever queued it; the boundary settles.
            match settle_completion_step(view, context, &root.state, &operation) {
                Ok(candidate) => {
                    root.state = candidate;
                    root.applied(packet.ingress);
                }
                Err(error) if !fails_boundary(&error) => root.refuse(packet_type, &error),
                Err(error) => return Err(error),
            }
        }
        PRODUCTION_RESOURCE_CONTINUATION_INGRESS => {
            let witness: crate::ProductionResourceContinuationWitnessV1 = match decode(
                &without_scope(&packet.payload),
                "production resource continuation witness",
            ) {
                Ok(witness) => witness,
                Err(error) => {
                    root.refuse(packet_type, &error);
                    return Ok(());
                }
            };
            // A witness that is stale or forged when it settles is refused
            // and recorded; the boundary settles.
            // A witness records evidence and changes no lifecycle, so it is
            // no trigger.
            match settle_continuation_witness(view, &root.state, witness) {
                Ok(candidate) => {
                    root.state = candidate;
                    root.changed = true;
                }
                Err(error) if !fails_boundary(&error) => root.refuse(packet_type, &error),
                Err(error) => return Err(error),
            }
        }
        PRODUCTION_ARCHIVE_COMMIT_INGRESS => {
            let commit: crate::VerifiedProductionArchiveCommitV1 = match packet
                .payload
                .get("commit")
                .cloned()
                .ok_or_else(|| invalid("production archive commit payload is absent"))
                .and_then(|value| decode(&value, "production archive commit"))
            {
                Ok(commit) => commit,
                Err(error) => {
                    root.refuse(packet_type, &error);
                    return Ok(());
                }
            };
            // A commit whose content production's checks refuse, whoever
            // scheduled it, settles nothing and is recorded. Archive
            // maintenance changes the root but drives no incident draw,
            // dispatch, or report.
            let mut candidate = root.state.clone();
            match candidate.apply_production_archive_commit(&commit, context.at) {
                Ok(receipt) => {
                    root.state = candidate;
                    root.changed = true;
                    root.directives.push(BoundaryDirective::Emit {
                        event_type: "canwu.production.archive_maintenance.v1".to_owned(),
                        summary: format!(
                            "production archive maintenance {} reached {:?}",
                            receipt.id, receipt.disposition
                        ),
                        affected: Vec::new(),
                    });
                }
                Err(error) if !fails_boundary(&error) => root.refuse(packet_type, &error),
                Err(error) => return Err(error),
            }
        }
        PRODUCTION_ARCHIVE_RETENTION_ACK_INGRESS => {
            let acknowledgement: ProductionArchiveRetentionAcknowledgementV1 = match decode(
                &packet.payload,
                "production archive retention acknowledgement",
            ) {
                Ok(acknowledgement) => acknowledgement,
                Err(error) => {
                    root.refuse(packet_type, &error);
                    return Ok(());
                }
            };
            let mut candidate = root.state.clone();
            match candidate.acknowledge_production_archive_retention(&acknowledgement.handle_id) {
                Ok(true) => {
                    root.state = candidate;
                    root.changed = true;
                }
                Ok(false) => {}
                Err(error) if !fails_boundary(&error) => root.refuse(packet_type, &error),
                Err(error) => return Err(error),
            }
        }
        _ => {}
    }
    Ok(())
}

/// How one operation or batch item settled.
enum Settled {
    /// Its outcome was recorded with this disposition.
    Recorded(ProductionOperationDisposition),
    /// Nothing was recorded: the root had no room for its outcome, or
    /// recording it would break a bound of the root.
    Refused(CanwuError),
}

/// Settles one admitted operation against `root` and records its outcome.
///
/// The operation is applied to one copy of the root, which is validated once
/// with the outcome in it. An expected rejection is recorded on the root
/// instead. When the root has no room for the outcome, or recording even a
/// rejection would break one of its bounds, nothing is recorded and the
/// operation is refused, so no bound of the root can fail the boundary.
fn settle_operation(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    root: &mut RootSettlement,
    envelope: &ProductionCommandEnvelope,
    input_hash: &str,
    decision_receipt: Option<&crate::ProductionDecisionReceiptV1>,
) -> Result<Settled, CanwuError> {
    if root.operations >= root.state.limits.max_operations_per_boundary {
        return Ok(Settled::Refused(CanwuError::new(
            ErrorCode::ValueOutOfRange,
            "the root settled its operation bound for this boundary",
        )));
    }
    if let Err(error) = root
        .state
        .ensure_operation_outcome_admission_capacity(&envelope.operation)
    {
        return Ok(Settled::Refused(error));
    }
    root.operations += 1;
    let outcome = |disposition, rejection: Option<&CanwuError>| ProductionOperationOutcome {
        id: envelope.operation_id.clone(),
        canonical_input_hash: input_hash.to_owned(),
        command: envelope.clone(),
        disposition,
        work_order: operation_work_order(&envelope.operation),
        execution: operation_execution(&envelope.operation),
        project: operation_project(&envelope.operation),
        rejection_code: rejection.map(|error| error_code(&error.code).to_owned()),
        rejection_message: rejection.map(|error| error.message.clone()),
        settled_at: context.at,
    };
    let mut candidate = root.state.clone();
    let result = validate_external_operation_evidence(
        view,
        &root.state,
        &envelope.holder,
        &envelope.operation,
    )
    .and_then(|()| candidate.apply_operation_in_place(envelope, context.at))
    .and_then(|()| {
        if let Some(receipt) = decision_receipt
            && candidate
                .decision_receipts
                .insert(receipt.ticket_id, receipt.clone())
                .is_some()
        {
            return Err(CanwuError::new(
                ErrorCode::IdempotencyConflict,
                "production decision ticket already has a receipt",
            ));
        }
        candidate.operation_outcomes.insert(
            envelope.operation_id.clone(),
            outcome(ProductionOperationDisposition::Applied, None),
        );
        candidate.validate()
    });
    let rejection = match result {
        Ok(()) => {
            root.state = candidate;
            if let crate::ProductionOperation::AcceptFacilityCommissioning { project } =
                &envelope.operation
            {
                let project = root
                    .state
                    .facility_projects
                    .get(project)
                    .ok_or_else(|| invalid("accepted facility project disappeared"))?;
                root.directives
                    .push(BoundaryDirective::SchedulePluginIngress {
                    target_plugin: canwu_resource::PLUGIN_NAME.to_owned(),
                    after: SimDuration::ZERO,
                    packet_type: canwu_resource::RESOURCE_COMPLETION_INGRESS.to_owned(),
                    priority: 0,
                    payload: serde_json::to_value(
                        canwu_resource::ResourceCompletionOperationV1::CompleteExternalParticipant(
                            canwu_resource::CompleteExternalCompletionParticipantGrantV1 {
                                acquisition: project.completion_certificate.acquisition.clone(),
                                operation_key: project.operation_key.clone(),
                            },
                        ),
                    )
                    .map_err(|error| encode_error(&error))?,
                    affected: Vec::new(),
                });
            }
            return Ok(Settled::Recorded(ProductionOperationDisposition::Applied));
        }
        Err(error) if expected_domain_rejection(&error) => error,
        Err(error) => return Err(error),
    };
    let revision = root.state.revision;
    root.state.revision = revision.checked_add(1).ok_or_else(|| {
        invalid("production runtime revision overflowed while rejecting an operation")
    })?;
    root.state.operation_outcomes.insert(
        envelope.operation_id.clone(),
        outcome(ProductionOperationDisposition::Rejected, Some(&rejection)),
    );
    match root.state.validate() {
        Ok(()) => Ok(Settled::Recorded(ProductionOperationDisposition::Rejected)),
        Err(error) if expected_domain_rejection(&error) => {
            root.state.operation_outcomes.remove(&envelope.operation_id);
            root.state.revision = revision;
            root.operations -= 1;
            Ok(Settled::Refused(error))
        }
        Err(error) => Err(error),
    }
}

/// Refused item IDs a batch summary lists before it only counts the rest.
const BATCH_SUMMARY_REFUSED_IDS: usize = 16;

/// Settles one production batch item by item, in order, against `root`.
///
/// A batch whose expected revision is not the root's, or whose items would
/// take the root past the operations it settles in one boundary, is refused
/// whole with a trace: nothing is recorded, so it can be sent again.
fn settle_batch(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    root: &mut RootSettlement,
    ingress: IngressId,
    batch: &ProductionBatchEnvelopeV1,
) -> Result<(), CanwuError> {
    if let Some(expected) = batch.expected_runtime_revision
        && expected != root.state.revision
    {
        root.refuse(
            PRODUCTION_BATCH_INGRESS,
            &CanwuError::new(
                ErrorCode::DomainRecordVersionConflict,
                format!(
                    "the batch expected runtime revision {expected}, but the root is at revision {}",
                    root.state.revision
                ),
            ),
        );
        return Ok(());
    }
    let fresh = batch
        .items
        .iter()
        .filter(|item| {
            !root
                .state
                .operation_outcomes
                .contains_key(&item.operation_id)
        })
        .count();
    if root
        .operations
        .checked_add(fresh)
        .is_none_or(|total| total > root.state.limits.max_operations_per_boundary)
    {
        root.refuse(
            PRODUCTION_BATCH_INGRESS,
            &CanwuError::new(
                ErrorCode::ValueOutOfRange,
                "the batch would take the root past the operations it settles in one boundary",
            ),
        );
        return Ok(());
    }
    let (mut applied, mut rejected, mut settled_before) = (0_usize, 0_usize, 0_usize);
    let mut refused = Vec::new();
    let mut first_refusal = None;
    for item in &batch.items {
        if let Some(existing) = root.state.operation_outcomes.get(&item.operation_id) {
            if batch_item_settled(existing, batch, item) {
                settled_before += 1;
            } else {
                refused.push(item.operation_id.to_string());
                first_refusal.get_or_insert_with(|| {
                    "an item reuses an operation ID with different input".to_owned()
                });
            }
            continue;
        }
        // Each item's outcome records the envelope as it applied: the
        // revision it settled against.
        let envelope = ProductionCommandEnvelope {
            operation_id: item.operation_id.clone(),
            holder: batch.holder.clone(),
            expected_runtime_revision: root.state.revision,
            operation: item.operation.clone(),
            scope: batch.scope.clone(),
        };
        let input_hash = canonical_hash("canwu.production.operation-input.v1", &envelope)?;
        match settle_operation(view, context, root, &envelope, &input_hash, None)? {
            Settled::Recorded(ProductionOperationDisposition::Applied) => {
                applied += 1;
                root.applied(ingress);
            }
            Settled::Recorded(_) => {
                rejected += 1;
                root.changed = true;
            }
            Settled::Refused(error) => {
                refused.push(item.operation_id.to_string());
                first_refusal.get_or_insert(error.message);
            }
        }
    }
    let listed = refused
        .iter()
        .take(BATCH_SUMMARY_REFUSED_IDS)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    root.directives.push(BoundaryDirective::Emit {
        event_type: BATCH_SETTLED_EVENT.to_owned(),
        summary: format!(
            "production batch of {} items settled: {applied} applied, {rejected} rejected, {settled_before} settled before, {} refused{}{}",
            batch.items.len(),
            refused.len(),
            if refused.is_empty() {
                String::new()
            } else {
                format!(
                    " [{listed}{}]",
                    if refused.len() > BATCH_SUMMARY_REFUSED_IDS {
                        ", ..."
                    } else {
                        ""
                    }
                )
            },
            first_refusal
                .map(|reason| format!(" (first refusal: {reason})"))
                .unwrap_or_default()
        ),
        affected: Vec::new(),
    });
    Ok(())
}

/// Settles one completion coordinator step on a copy of `state` and returns
/// the copy. The step may come from the resource plugin, from
/// `enqueue_production_completion_operation`, which the host application
/// calls with any step, or from another plugin.
fn settle_completion_step(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    state: &ProductionState,
    operation: &crate::ProductionCompletionIngressV1,
) -> Result<ProductionState, CanwuError> {
    validate_production_completion_ingress(view, context, state, operation)?;
    let mut candidate = state.clone();
    if let crate::ProductionCompletionIngressV1::AcknowledgeParticipantCompleted {
        acquisition,
        participant: _,
    } = operation
    {
        let resource = resource_root(
            view,
            [
                canwu_resource::ResourceTerminalRecordKeyV1::ExternalCompletionParticipant(
                    acquisition.clone(),
                ),
            ],
            "resource completion runtime is unavailable",
        )?;
        let authoritative = resource
            .external_completion_participants
            .participant(acquisition)
            .ok_or_else(|| invalid("completed resource participant is unavailable"))?;
        let provider_source = view
            .current_domain_record_version(
                &canwu_resource::resource_runtime_reference().into_untyped(),
            )?
            .ok_or_else(|| invalid("completed resource participant exact source is unavailable"))?;
        if candidate.facility_projects.values().any(|project| {
            project.completion_certificate.acquisition == *acquisition
                && project.lifecycle == crate::FacilityProjectLifecycle::CompletionPending
        }) {
            candidate.finalize_facility_project_completion(
                acquisition,
                &provider_source,
                &authoritative.grant,
                context.at,
            )?;
        } else {
            candidate.finalize_execution_resource_completion(
                acquisition,
                &provider_source,
                &authoritative.grant,
            )?;
        }
    } else {
        candidate.apply_completion_ingress(operation)?;
    }
    candidate.revision = candidate
        .revision
        .checked_add(1)
        .ok_or_else(|| invalid("production completion revision overflowed"))?;
    candidate.validate()?;
    Ok(candidate)
}

/// Records a continuation witness on a copy of `state` and returns the copy.
/// A witness checked when it was queued can be stale when it settles: its
/// project may have ended in between.
fn settle_continuation_witness(
    view: &SimulationView<'_>,
    state: &ProductionState,
    witness: crate::ProductionResourceContinuationWitnessV1,
) -> Result<ProductionState, CanwuError> {
    validate_resource_continuation_witness(view, state, &witness)?;
    let mut candidate = state.clone();
    candidate
        .resource_continuation_witnesses
        .insert(witness.project.clone(), witness);
    candidate.revision = candidate
        .revision
        .checked_add(1)
        .ok_or_else(|| invalid("production continuation revision overflowed"))?;
    candidate.validate()?;
    Ok(candidate)
}

/// Records a production packet that does not decode, or that production's
/// checks refuse: it settles nothing, and the boundary settles. The host
/// application can queue the public packet types and completion steps, and
/// another plugin can schedule any production packet type.
fn refuse_packet(
    directives: &mut Vec<BoundaryDirective>,
    event_type: &str,
    packet_type: &str,
    error: &CanwuError,
) {
    directives.push(BoundaryDirective::Emit {
        event_type: event_type.to_owned(),
        summary: format!(
            "production packet {packet_type} was refused: {}",
            error.message
        ),
        affected: Vec::new(),
    });
}

/// The refusal of a packet no admitted tracked command queued.
fn unmatched_command(error: &CanwuError) -> CanwuError {
    CanwuError::new(
        error.code.clone(),
        "it does not match an admitted tracked production command",
    )
}

/// Whether a packet is the one the admission of a tracked production command
/// queued: its cause is that command, the command is `command_name` of this
/// plugin with `expected_payload` as its payload, and the packet's input
/// hash is the one admission computed. The check reads the command's
/// retained record, which a seal keeps while the packet is queued.
fn validate_admitted_command(
    view: &SimulationView<'_>,
    cause: Option<&CauseRef>,
    admitted_command: canwu_api::CommandId,
    command_name: &str,
    expected_payload: &Value,
    input_hash: &str,
    admitted_hash: &str,
) -> Result<(), CanwuError> {
    let Some(CauseRef::Command(command)) = cause else {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "production operation packet lacks command evidence",
        ));
    };
    let record = view.command(*command)?.ok_or_else(|| {
        CanwuError::new(
            ErrorCode::EvidenceUnavailable,
            "production command evidence is unavailable",
        )
    })?;
    let matches = *command == admitted_command
        && matches!(
            &record.envelope.command,
            Command::Plugin { plugin, command, payload }
                if plugin == PLUGIN_NAME
                    && command == command_name
                    && payload == expected_payload
        )
        && input_hash == admitted_hash;
    if !matches {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "production operation packet does not match its tracked command",
        ));
    }
    Ok(())
}

/// The roots the lifecycle system wrote in this boundary, each validated
/// again as proposed.
fn validate_production_candidate(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    for root in active_roots(view, context)?.keys() {
        load_root(view, root.as_ref())?;
    }
    Ok(BoundaryProposal::default())
}

/// The roots whose production lifecycle a packet of this boundary changed,
/// and the roots whose observation wake was admitted, with the refusal of
/// each wake that names no root.
fn triggered_or_woken_roots(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<(BTreeSet<RootKey>, Vec<CanwuError>), CanwuError> {
    let mut roots = active_roots(view, context)?
        .into_iter()
        .filter(|(_, trigger)| trigger.is_some())
        .map(|(root, _)| root)
        .collect::<BTreeSet<_>>();
    let mut refusals = Vec::new();
    for ingress_id in &context.admitted_ingress {
        let Some(ingress) = view.ingress(*ingress_id)? else {
            continue;
        };
        if let IngressPayload::Plugin {
            plugin,
            packet_type,
            payload,
            ..
        } = &ingress.payload
            && plugin == PLUGIN_NAME
            && packet_type == PRODUCTION_OBSERVATION_WAKE_INGRESS
        {
            match payload_scope(payload.get("scope")) {
                Ok(root)
                    if view
                        .typed_domain_record(&production_root_reference(root.as_ref()))?
                        .is_some() =>
                {
                    roots.insert(root);
                }
                Ok(_) => refusals.push(CanwuError::new(
                    ErrorCode::InvalidPayload,
                    "the wake names a production root that is not installed",
                )),
                Err(error) => refusals.push(error),
            }
        }
    }
    Ok((roots, refusals))
}

/// Pins the production source of every execution whose output waits for its
/// first dispatch, in every root a packet of this boundary changed or a wake
/// names: an output becomes pending when an operation completes its
/// execution, and a root installed or restored with pending output resumes
/// on its next wake.
fn dispatch_pending_outputs(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut mutations = Vec::new();
    let mut directives = Vec::new();
    for root in triggered_or_woken_roots(view, context)?.0 {
        let Some((record, mut state)) = load_root(view, root.as_ref())? else {
            continue;
        };
        let pending = state
            .executions
            .values()
            .filter(|execution| {
                execution.lifecycle == crate::WorkOrderLifecycle::CompletedPendingOutputSettlement
                    && execution.output_outcomes.is_empty()
                    && execution.output_source.is_none()
            })
            .map(|execution| execution.id.clone())
            .collect::<Vec<_>>();
        if pending.is_empty() {
            continue;
        }
        let source = view
            .current_domain_record_version(&record.reference)?
            .ok_or_else(|| invalid("production output source version is unavailable"))?;
        for execution_id in pending {
            let execution = state
                .executions
                .get_mut(&execution_id)
                .expect("pending production output execution was selected");
            execution.output_source = Some(source.clone());
            let request = canwu_resource::ResourceProductionOutputBatchV1 {
                provider_plugin: PLUGIN_NAME.to_owned(),
                provider_source: source.clone(),
                requests: execution
                    .output_requests
                    .iter()
                    .map(|output| {
                        output.resource_credit_request(
                            source.clone(),
                            execution.completion_certificate.clone(),
                            context.at,
                        )
                    })
                    .collect(),
            };
            directives.push(BoundaryDirective::SchedulePluginIngress {
                target_plugin: canwu_resource::PLUGIN_NAME.to_owned(),
                after: SimDuration::ZERO,
                packet_type: canwu_resource::RESOURCE_PRODUCTION_OUTPUT_BATCH_INGRESS.to_owned(),
                priority: 0,
                payload: serde_json::to_value(request).map_err(|error| encode_error(&error))?,
                affected: Vec::new(),
            });
        }
        state.revision = state
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("production output dispatch revision overflowed"))?;
        mutations.push(BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Update {
                record: state.draft()?,
                expected_version: record.version,
            },
            summary: "Pin exact production sources for pending output settlement".to_owned(),
        });
    }
    mutations.extend(directives);
    Ok(BoundaryProposal {
        directives: mutations,
        ..BoundaryProposal::default()
    })
}

/// Evaluates the incident candidates of every root that a packet of this
/// boundary changed. A root changed only by refused packets or recorded
/// rejections draws nothing, so queued junk cannot cause incident draws.
fn evaluate_incident_candidates(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut mutations = Vec::new();
    let mut directives = Vec::new();
    for (root, trigger) in active_roots(view, context)? {
        let Some(trigger) = trigger else {
            continue;
        };
        let Some((record, mut state)) = load_root(view, root.as_ref())? else {
            continue;
        };
        let operation_evidence = EvidenceRef::Ingress(trigger);
        let folded = state.fold_incident_receipts(context.boundary_id)?;
        let source_record_digest = canonical_hash("canwu.production.runtime-cut.v1", &record)?;
        let candidates = state
            .incident_due_index
            .iter()
            .take(state.limits.max_incidents_per_boundary)
            .cloned()
            .collect::<Vec<_>>();
        let evaluated = candidates.last().cloned();
        let mut committed = 0_usize;
        for facility_id in candidates {
            let facility = state
                .facilities
                .get(&facility_id)
                .ok_or_else(|| invalid("incident due index lost its facility"))?
                .clone();
            // An incident marks its site's reports dirty; while the dirty
            // index is full the remaining candidates wait for a later round.
            if state.observation_dirty_index.len() >= state.limits.max_observation_dirty {
                break;
            }
            let operation_key = format!(
                "canwu.production:incident:{}:{}:{}:{}:{}",
                record.reference.id,
                facility.id,
                facility.generation,
                record.version,
                context.boundary_id.get()
            );
            if state.incident_receipts.contains_key(&operation_key) {
                continue;
            }
            let target = RandomOperationTarget::CanonicalKey(facility.id.to_string());
            let trigger = view.random_sample_for_operation(
                &production_incident_random_stream(),
                operation_evidence.clone(),
                "production_facility_incident",
                &operation_key,
                target.clone(),
                0,
                1_000,
                "select a configured production facility incident",
            )?;
            if trigger.value >= u64::from(facility.incident_risk_per_mille) {
                continue;
            }
            let severity = view.random_sample_for_operation(
                &production_incident_random_stream(),
                operation_evidence.clone(),
                "production_facility_incident",
                &operation_key,
                target,
                1,
                u64::from(facility.incident_max_severity_per_mille),
                "determine the selected production facility incident severity",
            )?;
            let loss = u16::try_from(severity.value.saturating_add(1))
                .map_err(|_| invalid("production incident severity overflowed"))?
                .min(facility.condition_per_mille);
            let condition_after = facility.condition_per_mille.saturating_sub(loss);
            let mut transition = crate::ProductionIncidentTransitionV1 {
                id: crate::ProductionIncidentId::new(format!(
                    "canwu.production:incident-receipt:{}:{}:{}",
                    facility.id,
                    facility.generation,
                    context.boundary_id.get()
                ))?,
                operation_key,
                facility: facility.id,
                expected_generation: facility.generation,
                condition_before: facility.condition_per_mille,
                condition_after,
                lifecycle_after: if condition_after <= 250 {
                    crate::FacilityLifecycle::Damaged
                } else {
                    crate::FacilityLifecycle::Degraded
                },
                source_record_revision: record.version,
                source_record_digest: source_record_digest.clone(),
                random: crate::ProductionIncidentRandomEvidenceV1 {
                    stream: production_incident_random_stream(),
                    trigger,
                    severity: Some(severity),
                    operation_evidence: operation_evidence.clone(),
                },
                evaluated_at: context.at,
                evaluation_boundary: context.boundary_id,
                canonical_digest: String::new(),
            };
            transition.canonical_digest =
                canonical_hash("canwu.production.incident-transition.v1", &transition)?;
            state.apply_incident_transition(&transition)?;
            committed = committed.saturating_add(1);
        }
        if evaluated.is_none() && !folded {
            continue;
        }
        state.advance_incident_cursor(evaluated)?;
        mutations.push(BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Update {
                record: state.draft()?,
                expected_version: record.version,
            },
            summary: "Persist the bounded production incident fairness cursor and any operation-keyed transition bundles for kernel commit".to_owned(),
        });
        if committed > 0 {
            directives.push(BoundaryDirective::Emit {
                event_type: "canwu.production.incident_committed.v1".to_owned(),
                summary: format!("staged {committed} bounded production incident transition(s) for atomic conditional commit"),
                affected: Vec::new(),
            });
        }
    }
    mutations.extend(directives);
    Ok(BoundaryProposal {
        directives: mutations,
        ..BoundaryProposal::default()
    })
}

fn audit_incident_transition_commit(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    for root in active_roots(view, context)?.keys() {
        load_root(view, root.as_ref())?;
    }
    Ok(BoundaryProposal::default())
}

/// Publishes the holder reports of every root a packet of this boundary
/// changed and of every root whose wake was admitted, within the engine's
/// publication budget and the roots' report bounds. A report that does not
/// fit waits as due work: for the publication budget, a wake one minute
/// later retries it; for a knowledge bound, it waits for the root's next
/// activity.
#[allow(clippy::too_many_lines)]
fn publish_holder_reports(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let (roots, wake_refusals) = triggered_or_woken_roots(view, context)?;
    let mut mutations = Vec::new();
    let mut directives = Vec::new();
    for error in &wake_refusals {
        refuse_packet(
            &mut directives,
            INGRESS_REFUSED_EVENT,
            PRODUCTION_OBSERVATION_WAKE_INGRESS,
            error,
        );
    }
    if roots.is_empty() {
        return Ok(BoundaryProposal {
            directives,
            ..BoundaryProposal::default()
        });
    }
    let mut budget = view.knowledge_publication_budget();
    let mut namespace_count = view.knowledge_record_count_in_namespace(PLUGIN_NAMESPACE)?;
    let mut holder_counts = HolderReportCounts::default();
    for root in roots {
        let Some((record, mut state)) = load_root(view, root.as_ref())? else {
            continue;
        };
        let provider_source = view
            .current_domain_record_version(&record.reference)?
            .ok_or_else(|| invalid("production observation provider source is unavailable"))?;
        let limits = state.limits;
        let mut reserved_total = 0_usize;
        let mut reserved_by_holder = BTreeMap::new();
        let dirty = state
            .observation_dirty_index
            .iter()
            .take(limits.max_reports_per_boundary)
            .cloned()
            .collect::<Vec<_>>();
        let mut state_changed = false;
        // Work this boundary could not take for want of room in its own
        // bounds is retried by a wake one minute later: work past the
        // publication budget always, and work past the report bound per
        // boundary while the pass made progress. Work blocked by a knowledge
        // bound waits for the root's next activity instead, since a wake
        // would find the bound still reached.
        let mut overflow = state.observation_dirty_index.len() > dirty.len();
        let mut progress = false;
        let mut budget_deferred = false;
        // The pairs due now prune first: their newest eligible heads can free
        // room in the root's shared rollover for a dirty pair of another
        // holder or site, which would otherwise wait for a later run.
        let due_now = state
            .observation_due_index
            .range(..=context.at)
            .flat_map(|(_, keys)| keys.iter().cloned())
            .take(limits.max_reports_per_boundary)
            .collect::<Vec<_>>();
        for key in &due_now {
            state_changed |= state.prune_observation_key(key, context.at)?;
        }
        for key in dirty {
            let holder_count = holder_counts.count(view, &key.holder, &limits)?;
            let holder_reserved = reserved_by_holder
                .get(&key.holder)
                .copied()
                .unwrap_or_default();
            state_changed |= state.prune_observation_key(&key, context.at)?;
            if namespace_count.saturating_add(reserved_total) < limits.max_observation_records
                && holder_count.saturating_add(holder_reserved)
                    < limits.max_observation_records_per_holder
                && state.observation_head_fits(&key)?
            {
                state.materialize_observation_head(
                    &key,
                    context.at,
                    EvidenceRef::DomainRecordVersion(provider_source.clone()),
                )?;
                reserved_total = reserved_total.saturating_add(1);
                reserved_by_holder.insert(key.holder.clone(), holder_reserved.saturating_add(1));
                state_changed = true;
                progress = true;
            } else {
                directives.push(BoundaryDirective::Emit {
                    event_type: "canwu.production.report_capacity_rejected.v1".to_owned(),
                    summary: "Retain dirty production observation work because bounded knowledge capacity is exhausted".to_owned(),
                    affected: Vec::new(),
                });
            }
        }
        let due_times = state
            .observation_due_index
            .range(..=context.at)
            .map(|(due, _)| *due)
            .collect::<Vec<_>>();
        let mut due_keys = Vec::new();
        for due in due_times {
            if let Some(keys) = state.observation_due_index.remove(&due) {
                for key in keys {
                    if due_keys.len() == limits.max_reports_per_boundary {
                        overflow = true;
                        state
                            .observation_due_index
                            .entry(due)
                            .or_default()
                            .insert(key);
                    } else {
                        due_keys.push(key);
                    }
                }
                state_changed = true;
            }
        }
        let mut candidates = Vec::new();
        for key in due_keys {
            // A due pair's newest eligible head dominates its older rolled-out
            // heads from now on.
            state_changed |= state.prune_observation_key(&key, context.at)?;
            let Some(site) = state.sites.get(&key.scope) else {
                continue;
            };
            let head = crate::query::eligible_observation_head(
                &state,
                &key.holder,
                &key.scope,
                context.at,
            )?;
            let report = crate::query::production_report_from_head(head, context.at)?;
            candidates.push((
                key.clone(),
                key.holder,
                site.place.clone(),
                report,
                head.source_evidence.clone(),
            ));
        }
        candidates.sort_by(|left, right| {
            (&left.1, &left.3.scope, &left.3.id).cmp(&(&right.1, &right.3.scope, &right.3.id))
        });
        for (key, holder, place, report, source_evidence) in candidates {
            // A holder the engine would refuse knowledge (a core
            // organization, a dead person, a retired entity) gets no report,
            // rather than failing the boundary (downstream gap G-75).
            if !view.knowledge_holder_accepts_publication(&holder)? {
                continue;
            }
            let holder_count = holder_counts.count(view, &holder, &limits)?;
            let payload = serde_json::to_value(&report).map_err(|error| encode_error(&error))?;
            let payload_bytes = serde_json::to_vec(&payload)
                .map_err(|error| encode_error(&error))?
                .len();
            let within_bounds = namespace_count < limits.max_observation_records
                && holder_count < limits.max_observation_records_per_holder;
            if within_bounds && budget.take(1, 1, payload_bytes) {
                directives.push(BoundaryDirective::PublishKnowledge {
                    visibility: StateVisibility::SameBoundary,
                    producer_correlation: Some(report.id.to_string()),
                    supersession: canwu_api::KnowledgeSupersession::Explicit,
                    groups: vec![canwu_api::KnowledgePublicationGroup {
                        holders: vec![holder.clone()],
                        records: vec![KnowledgeRecordDraft {
                            schema: production_report_knowledge_schema_id(),
                            subjects: vec![KnowledgeSubject {
                                role: "site".to_owned(),
                                target: KnowledgeSubjectTarget::Entity(place),
                            }],
                            payload,
                            as_of: Some(report.observed_at),
                            confidence_per_mille: match report.role {
                                crate::ProductionObservationRole::Operator => 1_000,
                                crate::ProductionObservationRole::LocalOwner => 900,
                                crate::ProductionObservationRole::RemoteOwner => 700,
                            },
                            origin: KnowledgeOrigin {
                                method: "production_holder_report_v1".to_owned(),
                                evidence: source_evidence,
                            },
                            supersedes: Vec::new(),
                            contradicts: Vec::new(),
                        }],
                    }],
                    summary: "Publish one bounded holder-relative production report".to_owned(),
                });
                namespace_count = namespace_count.saturating_add(1);
                holder_counts.published(holder);
                progress = true;
            } else {
                budget_deferred |= within_bounds;
                state
                    .observation_due_index
                    .entry(context.at)
                    .or_default()
                    .insert(key);
                state_changed = true;
                directives.push(BoundaryDirective::Emit {
                    event_type: "canwu.production.report_capacity_rejected.v1".to_owned(),
                    summary: if within_bounds {
                        "Defer periodic production report because the boundary's knowledge publication budget is exhausted"
                    } else {
                        "Reject periodic production report because its bounded knowledge capacity is exhausted"
                    }
                    .to_owned(),
                    affected: Vec::new(),
                });
            }
        }
        let next_due = state
            .observation_due_index
            .range((
                std::ops::Bound::Excluded(context.at),
                std::ops::Bound::Unbounded,
            ))
            .next()
            .map(|(due, _)| *due);
        let retry_at = (budget_deferred || (overflow && progress))
            .then(|| context.at.checked_add(SimDuration::minutes(1)))
            .flatten();
        let wake_at = match (next_due, retry_at) {
            (Some(due), Some(retry)) => Some(due.min(retry)),
            (due, retry) => due.or(retry),
        };
        if let Some(wake_at) = wake_at {
            directives.push(BoundaryDirective::SchedulePluginIngress {
                target_plugin: PLUGIN_NAME.to_owned(),
                after: wake_at
                    .checked_sub(context.at)
                    .ok_or_else(|| invalid("production observation wake delay overflowed"))?,
                packet_type: PRODUCTION_OBSERVATION_WAKE_INGRESS.to_owned(),
                priority: 0,
                payload: with_scope(serde_json::json!({ "due_at": wake_at }), root.as_ref()),
                affected: Vec::new(),
            });
        }
        if state_changed {
            state.revision = state
                .revision
                .checked_add(1)
                .ok_or_else(|| invalid("production runtime revision overflowed"))?;
            mutations.push(BoundaryDirective::MutateRecord {
                mutation: DomainRecordMutation::Update {
                    record: state.draft()?,
                    expected_version: record.version,
                },
                summary: "Persist bounded production observation cuts and delivery indexes"
                    .to_owned(),
            });
        }
    }
    mutations.extend(directives);
    Ok(BoundaryProposal {
        directives: mutations,
        ..BoundaryProposal::default()
    })
}

/// The production reports each holder keeps, as far as the report bounds of
/// the roots a boundary publishes for need them counted, plus the reports
/// this boundary proposed for the holder so far.
#[derive(Default)]
struct HolderReportCounts {
    /// Per holder: the bound counted up to and the count, which is exact
    /// when it is below the bound.
    ledger: BTreeMap<KnowledgeHolderRef, (usize, usize)>,
    published: BTreeMap<KnowledgeHolderRef, usize>,
}

impl HolderReportCounts {
    fn count(
        &mut self,
        view: &SimulationView<'_>,
        holder: &KnowledgeHolderRef,
        limits: &crate::ProductionLimitsV1,
    ) -> Result<usize, CanwuError> {
        let bound = limits.max_observation_records_per_holder;
        let counted = match self.ledger.get(holder) {
            Some((counted_to, count)) if *counted_to >= bound || count < counted_to => *count,
            _ => {
                let count = production_holder_knowledge_count(view, holder, limits)?;
                self.ledger.insert(holder.clone(), (bound, count));
                count
            }
        };
        Ok(counted.saturating_add(self.published.get(holder).copied().unwrap_or_default()))
    }

    fn published(&mut self, holder: KnowledgeHolderRef) {
        *self.published.entry(holder).or_default() += 1;
    }
}

fn production_holder_knowledge_count(
    view: &SimulationView<'_>,
    holder: &KnowledgeHolderRef,
    limits: &crate::ProductionLimitsV1,
) -> Result<usize, CanwuError> {
    let maximum = limits.max_observation_records_per_holder;
    let mut after = None;
    let mut count = 0_usize;
    loop {
        let page = view.knowledge_records(
            holder.clone(),
            &KnowledgeQuery {
                schemas: vec![production_report_knowledge_schema_id()],
                view: KnowledgeHistoryView::FullHistory,
                after,
                limit: MAX_KNOWLEDGE_PAGE_SIZE,
                ..KnowledgeQuery::default()
            },
        )?;
        count = count
            .checked_add(page.records.len())
            .ok_or_else(|| invalid("production holder knowledge count overflowed"))?;
        if count >= maximum || page.next.is_none() {
            return Ok(count);
        }
        after = page.next;
    }
}

struct PluginProductionArchiveProvider<'a>(&'a dyn PluginArchiveObjectProvider);

impl crate::ProductionArchiveStore for PluginProductionArchiveProvider<'_> {
    fn store_production_archive_object(
        &self,
        _namespace: &str,
        _object_id: &str,
        _bytes: &[u8],
    ) -> Result<(), CanwuError> {
        Err(CanwuError::new(
            ErrorCode::InvalidArchive,
            "production reachability provider is read-only",
        ))
    }

    fn load_production_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, CanwuError> {
        self.0.load_plugin_archive_object(namespace, object_id)
    }

    fn persist_production_archive_retention(
        &self,
        _handle: &crate::ProductionArchiveRetentionHandleV1,
    ) -> Result<(), CanwuError> {
        Err(CanwuError::new(
            ErrorCode::InvalidArchive,
            "production reachability provider cannot persist retention",
        ))
    }

    fn finalize_production_archive_retention(
        &self,
        _handle_id: &str,
        _phase: crate::ProductionArchiveRetentionPhaseV1,
    ) -> Result<(), CanwuError> {
        Err(CanwuError::new(
            ErrorCode::InvalidArchive,
            "production reachability provider cannot finalize retention",
        ))
    }
}

/// Every production root the view holds, in record order.
fn production_roots_in_view(view: &SimulationView<'_>) -> Result<Vec<DomainRecord>, CanwuError> {
    const PAGE: usize = 256;
    let kind = production_root_kind();
    let mut roots = Vec::new();
    let mut after = None;
    loop {
        let page = view.domain_records_of_kind_after(&kind, after.as_ref(), PAGE)?;
        let full = page.len() == PAGE;
        after = page.last().map(|record| record.reference.clone());
        roots.extend(page);
        if !full {
            return Ok(roots);
        }
    }
}

/// Marks every production archive object a restore, a replay, a fork, or a
/// queued commit can still read: for every production root, the commits
/// reachable from its archive head and from each unacknowledged retention
/// handle, and every commit the kernel lists as queued, with their blobs,
/// pages, and lookup pages. Each root's archive is audited first, as at
/// restore.
fn production_archive_reachability(
    view: &SimulationView<'_>,
    provider: &dyn PluginArchiveObjectProvider,
    manifest: &mut ArchiveReachabilityManifest,
) -> Result<(), CanwuError> {
    let provider = PluginProductionArchiveProvider(provider);
    // Queued commits: the kernel marks the directory each queued archive
    // commit retains. Each may become the head of its root, so its index
    // must be whole.
    let queued = manifest
        .plugin_objects
        .get(crate::PRODUCTION_ARCHIVE_INDEX_DIRECTORY_NAMESPACE)
        .cloned()
        .unwrap_or_default();
    let mut roots = queued.clone();
    let mut complete = queued;
    let mut bound = roots.len();
    for record in production_roots_in_view(view)? {
        let state = record.decode_payload::<ProductionRuntimeRecord>()?;
        state.validate()?;
        crate::validate_production_archive(&provider, &state)?;
        if let Some(root) = &state.archive.directory_root {
            roots.insert(root.clone());
            complete.insert(root.clone());
        }
        roots.extend(
            state
                .archive
                .pending_handles
                .values()
                .map(|handle| handle.target_directory_root.clone()),
        );
        bound = usize::try_from(state.archive.committed_batch_count)
            .unwrap_or(usize::MAX)
            .saturating_add(state.archive.pending_handles.len())
            .saturating_add(bound);
    }
    if roots.is_empty() {
        return Ok(());
    }
    let objects =
        crate::archive::reachable_production_archive_objects(&provider, roots, &complete, bound)?;
    for (namespace, ids) in objects {
        for id in ids {
            manifest.insert_plugin_object(namespace, id);
        }
    }
    Ok(())
}

/// The production root `scope` names, decoded from a run.
fn host_root<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &H,
    scope: Option<&ProductionScopeId>,
) -> Result<ProductionState, CanwuError> {
    let state = canwu_api::typed_domain_record(canwu, &production_root_reference(scope))
        .ok_or_else(|| CanwuError::new(ErrorCode::PluginNotActive, "production runtime is absent"))?
        .decode_payload::<ProductionRuntimeRecord>()?;
    if state.scope.as_ref() != scope {
        return Err(invalid(
            "production root names another scope than its record",
        ));
    }
    Ok(state)
}

/// The production root `scope` names, as the run holds it: `None` for the
/// primary root. Takes either runtime handle.
pub fn production_root_state<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &H,
    scope: Option<&ProductionScopeId>,
) -> Result<Option<ProductionState>, CanwuError> {
    if canwu_api::typed_domain_record(canwu, &production_root_reference(scope)).is_none() {
        return Ok(None);
    }
    host_root(canwu, scope).map(Some)
}

/// Stores, reads back, verifies, and queues a production cold-archive commit
/// through a plugin-owned ingress capability.
///
/// The batch must equal what
/// [`crate::ProductionState::prepare_production_archive`] derives from the
/// current production root it names (`prepared.scope`) and `store` for the
/// same number of records, including its lookup-index pages. The boundary
/// that settles the commit reads no archive object: it applies the commit
/// when the root still is the one the batch was prepared from, and otherwise
/// records it as stale.
pub fn enqueue_production_archive<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &mut H,
    prepared: &crate::PreparedProductionArchiveBatchV1,
    store: &dyn crate::ProductionArchiveStore,
) -> Result<ProductionArchiveIngressReceiptV1, CanwuError> {
    let state = host_root(&*canwu, prepared.scope.as_ref())?;
    let canonical = state.prepare_production_archive(prepared.selected_count(), store)?;
    if &canonical != prepared {
        return Err(CanwuError::new(
            ErrorCode::InvalidArchive,
            "production archive batch differs from the exact current terminal candidates",
        ));
    }
    let mut commit = prepared.store_and_verify(store)?;
    let permit = PRODUCTION_ARCHIVE_COMMIT_PERMIT.get().ok_or_else(|| {
        CanwuError::new(
            ErrorCode::PluginNotActive,
            "production plugin must be registered before archive ingress",
        )
    })?;
    commit.retention.phase = crate::ProductionArchiveRetentionPhaseV1::DurableIngress;
    commit.retention.semantic_digest.clear();
    commit.retention.semantic_digest =
        canonical_hash("canwu.production.archive-retention.v1", &commit.retention)?;
    store.persist_production_archive_retention(&commit.retention)?;
    let retention_handle_id = commit.retention.handle_id.clone();
    let directory_root = commit.directory_root.clone();
    let scope = commit.scope.clone();
    let request = PluginIngressRequest::new(
        PLUGIN_NAME,
        PRODUCTION_ARCHIVE_COMMIT_INGRESS,
        canwu.time(),
        serde_json::json!({ "commit": commit }),
    )
    .with_archive_retention(vec![PluginArchiveRetention {
        namespace: crate::PRODUCTION_ARCHIVE_INDEX_DIRECTORY_NAMESPACE.to_owned(),
        object_id: directory_root.clone(),
    }]);
    match canwu.enqueue_permitted_plugin_ingress(request, permit) {
        Ok(ingress) => Ok(ProductionArchiveIngressReceiptV1 {
            ingress,
            scope,
            retention_handle_id,
            directory_root,
        }),
        Err(error) => {
            let _ = store.finalize_production_archive_retention(
                &retention_handle_id,
                crate::ProductionArchiveRetentionPhaseV1::Abandoned,
            );
            Err(error)
        }
    }
}

/// Queues one canonical production-owned completion coordinator transition
/// for the primary production root. Participant state remains authoritative
/// in the participant plugin and is accepted here only through an exact
/// current provider record acknowledgement.
pub fn enqueue_production_completion_operation<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &mut H,
    due_at: canwu_api::SimTime,
    operation: &crate::ProductionCompletionIngressV1,
) -> Result<canwu_api::IngressReceipt, CanwuError> {
    enqueue_production_completion_operation_in_scope(canwu, None, due_at, operation)
}

/// Queues one completion coordinator transition for the production root
/// `scope` names (`None` for the primary root).
pub fn enqueue_production_completion_operation_in_scope<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &mut H,
    scope: Option<&ProductionScopeId>,
    due_at: canwu_api::SimTime,
    operation: &crate::ProductionCompletionIngressV1,
) -> Result<canwu_api::IngressReceipt, CanwuError> {
    let permit = PRODUCTION_COMPLETION_INGRESS_PERMIT.get().ok_or_else(|| {
        CanwuError::new(
            ErrorCode::PluginNotActive,
            "production plugin must be active before completion ingress",
        )
    })?;
    canwu.enqueue_permitted_plugin_ingress(
        PluginIngressRequest::new(
            PLUGIN_NAME,
            PRODUCTION_COMPLETION_INGRESS,
            due_at,
            with_scope(
                serde_json::to_value(operation).map_err(|error| encode_error(&error))?,
                scope,
            ),
        ),
        permit,
    )
}

/// Authenticates every compacted resource input for one active facility
/// project of the primary production root against the resource archive
/// provider, then queues a package-owned witness used by later canonical
/// project ingress.
pub fn enqueue_production_resource_continuation<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &mut H,
    project_id: &crate::FacilityProjectId,
    store: &dyn canwu_resource::ResourceArchiveStore,
) -> Result<canwu_api::IngressReceipt, CanwuError> {
    enqueue_production_resource_continuation_in_scope(canwu, None, project_id, store)
}

/// [`enqueue_production_resource_continuation`] for the production root
/// `scope` names.
pub fn enqueue_production_resource_continuation_in_scope<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &mut H,
    scope: Option<&ProductionScopeId>,
    project_id: &crate::FacilityProjectId,
    store: &dyn canwu_resource::ResourceArchiveStore,
) -> Result<canwu_api::IngressReceipt, CanwuError> {
    let production = host_root(&*canwu, scope)?;
    let project = production
        .facility_projects
        .get(project_id)
        .ok_or_else(|| invalid("production continuation project is unavailable"))?;
    if matches!(
        project.lifecycle,
        crate::FacilityProjectLifecycle::Completed
            | crate::FacilityProjectLifecycle::Cancelled
            | crate::FacilityProjectLifecycle::Failed
    ) {
        return Err(invalid(
            "production continuation witness requires an active facility project",
        ));
    }
    let (_, resource) = canwu_resource::resource_state(canwu)?
        .ok_or_else(|| invalid("resource runtime is unavailable"))?;
    let root = resource
        .archive_head
        .directory_root
        .clone()
        .ok_or_else(|| invalid("resource archive head is unavailable"))?;
    // The witness names the archive head; authenticating that one commit is
    // enough, because each compacted input below resolves through the head's
    // lookup index, and a hit authenticates the blob and record it reads.
    let head: canwu_resource::ResourceArchiveIndexDirectoryV1 = store
        .load_resource_archive_object(
            canwu_resource::RESOURCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE,
            &root,
        )
        .map_err(|error| invalid(format!("resource archive validation failed: {error}")))?
        .ok_or_else(|| invalid("resource archive head directory is unavailable"))
        .and_then(|bytes| {
            serde_json::from_slice(&bytes).map_err(|error| {
                invalid(format!(
                    "resource archive head directory could not decode: {error}"
                ))
            })
        })?;
    canwu_resource::authenticate_resource_archive_directory(store, &head)
        .map_err(|error| invalid(format!("resource archive validation failed: {error}")))?;
    if head.id != root || head.archived_record_count != resource.archive_head.archived_record_count
    {
        return Err(invalid(
            "resource archive head directory differs from the resource archive head",
        ));
    }
    for input in &project.inputs {
        crate::validate_production_resource_continuation(&resource, store, input)?;
    }
    let mut witness = crate::ProductionResourceContinuationWitnessV1 {
        project: project_id.clone(),
        resource_archive_directory_root: root,
        resource_archive_record_count: resource.archive_head.archived_record_count,
        input_bindings_digest: crate::model::resource_input_bindings_digest(&project.inputs)?,
        semantic_digest: String::new(),
    };
    witness.semantic_digest = canonical_hash(
        "canwu.production.resource-continuation-witness.v1",
        &witness,
    )?;
    let permit = PRODUCTION_RESOURCE_CONTINUATION_PERMIT
        .get()
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::PluginNotActive,
                "production plugin must be active before continuation ingress",
            )
        })?;
    canwu.enqueue_permitted_plugin_ingress(
        PluginIngressRequest::new(
            PLUGIN_NAME,
            PRODUCTION_RESOURCE_CONTINUATION_INGRESS,
            canwu.time(),
            with_scope(
                serde_json::to_value(witness).map_err(|error| encode_error(&error))?,
                scope,
            ),
        ),
        permit,
    )
}

/// Finalizes a terminal production archive retention handle and queues the
/// package-owned acknowledgement that clears it, and the stale receipt
/// recorded under it, from the root the commit applied to.
///
/// The handle must still be in that root when this is called. An
/// acknowledgement that settles after another one retired the handle is a
/// no-op, so calling this twice before the next boundary is harmless.
pub fn finalize_production_archive_retention<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &mut H,
    store: &dyn crate::ProductionArchiveStore,
    receipt: &ProductionArchiveIngressReceiptV1,
) -> Result<canwu_api::IngressReceipt, CanwuError> {
    let state = host_root(&*canwu, receipt.scope.as_ref())?;
    let handle = state
        .archive
        .pending_handles
        .get(&receipt.retention_handle_id)
        .ok_or_else(|| invalid("production archive retention handle is not terminal"))?;
    if handle.target_directory_root != receipt.directory_root
        || !matches!(
            handle.phase,
            crate::ProductionArchiveRetentionPhaseV1::Committed
                | crate::ProductionArchiveRetentionPhaseV1::RejectedStale
                | crate::ProductionArchiveRetentionPhaseV1::Abandoned
        )
    {
        return Err(invalid(
            "production archive retention terminal state differs from the enqueue receipt",
        ));
    }
    store.finalize_production_archive_retention(&handle.handle_id, handle.phase)?;
    let permit = PRODUCTION_ARCHIVE_ACK_PERMIT.get().ok_or_else(|| {
        CanwuError::new(
            ErrorCode::PluginNotActive,
            "production plugin archive acknowledgement permit is unavailable",
        )
    })?;
    canwu.enqueue_permitted_plugin_ingress(
        PluginIngressRequest::new(
            PLUGIN_NAME,
            PRODUCTION_ARCHIVE_RETENTION_ACK_INGRESS,
            canwu.time(),
            serde_json::to_value(ProductionArchiveRetentionAcknowledgementV1 {
                handle_id: handle.handle_id.clone(),
                scope: receipt.scope.clone(),
            })
            .map_err(|error| encode_error(&error))?,
        ),
        permit,
    )
}

fn require_holder_authority(
    context: &CommandContext,
    holder: &KnowledgeHolderRef,
) -> Result<(), CanwuError> {
    let authorized = match holder {
        KnowledgeHolderRef::Person(person) => {
            context.issuer == Issuer::Actor(*person)
                || (context.decision_controller_id.is_some()
                    && matches!(
                        context.authority.decision_origin,
                        canwu_api::DecisionOrigin::Actor { actor } if actor == *person
                    ))
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
            "production command issuer is not authorized for its holder",
        ))
    }
}

fn validate_degraded_decision(
    view: &SimulationView<'_>,
    command_context: &CommandContext,
    state: &ProductionState,
    envelope: &ProductionCommandEnvelope,
) -> Result<Option<crate::ProductionDecisionReceiptV1>, CanwuError> {
    let crate::ProductionOperation::ResolveDegradedFacility {
        work_order,
        facility,
        choice,
        decision_ticket,
    } = &envelope.operation
    else {
        return Ok(None);
    };
    let controller_id = command_context
        .decision_controller_id
        .as_deref()
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidDecision,
                "degraded-facility choice must be executed by a persisted Canwu decision",
            )
        })?;
    let ticket = view
        .decision_ticket(*decision_ticket)?
        .ok_or_else(|| CanwuError::new(ErrorCode::InvalidDecision, "decision ticket is absent"))?;
    let (selected_option, trace_id) = match &ticket.state {
        DecisionTicketState::Resolved {
            option_id,
            trace_id,
        } => (option_id, *trace_id),
        _ => {
            return Err(CanwuError::new(
                ErrorCode::InvalidDecision,
                "degraded-facility decision ticket is not resolved",
            ));
        }
    };
    let expected_option = match choice {
        crate::DegradedFacilityChoice::ContinueDegraded => "continue_degraded",
        crate::DegradedFacilityChoice::StopForRepair => "stop_for_repair",
        crate::DegradedFacilityChoice::DeferOrder => "defer_order",
    };
    let decision_context: crate::DegradedFacilityDecisionContextV1 = decode(
        &ticket.context.payload,
        "degraded facility decision context",
    )?;
    let order = state
        .work_orders
        .get(work_order)
        .ok_or_else(|| invalid("degraded decision work order is unavailable"))?;
    let asset = state
        .facilities
        .get(facility)
        .ok_or_else(|| invalid("degraded decision facility is unavailable"))?;
    let observation_key = crate::ProductionObservationHeadKeyV1 {
        holder: envelope.holder.clone(),
        scope: order.site.clone(),
    };
    let observation_storage_key =
        crate::model::production_observation_head_storage_key(&observation_key)?;
    let holder_cut_exists = state
        .observation_heads
        .get(&observation_storage_key)
        .is_some_and(|heads| {
            heads
                .iter()
                .any(|head| head.canonical_digest == decision_context.holder_facts_digest)
        });
    if ticket.definition != "canwu.production.degraded-facility-choice.v1"
        || ticket.context.schema != "canwu.production.degraded-facility-choice.v1"
        || ticket.assigned_controller != controller_id
        || view.decision_controller(controller_id)?.is_none()
        || selected_option != expected_option
        || decision_context.holder != envelope.holder
        || decision_context.work_order != *work_order
        || decision_context.facility != *facility
        || decision_context.facility_generation != asset.generation
        || decision_context.expected_runtime_revision != envelope.expected_runtime_revision
        || decision_context.expected_runtime_revision != state.revision
        || !holder_cut_exists
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidDecision,
            "degraded-facility decision ticket, selected option, or holder observation cut is stale",
        ));
    }
    let selected = ticket.option(selected_option).ok_or_else(|| {
        CanwuError::new(
            ErrorCode::InvalidDecision,
            "degraded-facility decision selected an unknown option",
        )
    })?;
    let expected_command = serde_json::to_value(Command::Plugin {
        plugin: PLUGIN_NAME.to_owned(),
        command: PRODUCTION_COMMAND.to_owned(),
        payload: serde_json::to_value(envelope).map_err(|error| encode_error(&error))?,
    })
    .map_err(|error| encode_error(&error))?;
    if selected.action
        != (DecisionAction::Command {
            command: expected_command,
        })
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidDecision,
            "degraded-facility decision option does not contain this exact production command",
        ));
    }
    let mut receipt = crate::ProductionDecisionReceiptV1 {
        ticket_id: *decision_ticket,
        ticket_version: ticket.version,
        trace_id,
        controller_id: controller_id.to_owned(),
        selected_option: selected_option.clone(),
        holder_facts_digest: decision_context.holder_facts_digest,
        command_request_id: None,
        command_attempt_id: command_context.attempt_id,
        decided_at: ticket.updated_at,
        canonical_digest: String::new(),
    };
    receipt.canonical_digest = canonical_hash("canwu.production.decision-receipt.v1", &receipt)?;
    Ok(Some(receipt))
}

fn operation_work_order(operation: &crate::ProductionOperation) -> Option<crate::WorkOrderId> {
    match operation {
        crate::ProductionOperation::CreateWorkOrder { work_order } => Some(work_order.id.clone()),
        crate::ProductionOperation::AuthorizeWorkOrder { work_order }
        | crate::ProductionOperation::CancelWorkOrder { work_order, .. }
        | crate::ProductionOperation::ResolveDegradedFacility { work_order, .. } => {
            Some(work_order.clone())
        }
        crate::ProductionOperation::StartExecution { execution, .. } => {
            Some(execution.work_order.clone())
        }
        crate::ProductionOperation::RequestCompletionLease { .. }
        | crate::ProductionOperation::AbortCompletionLease { .. }
        | crate::ProductionOperation::AdvanceExecution { .. }
        | crate::ProductionOperation::CompleteExecution { .. }
        | crate::ProductionOperation::CreateFacilityProject { .. }
        | crate::ProductionOperation::AuthorizeFacilityProject { .. }
        | crate::ProductionOperation::AdvanceFacilityProject { .. }
        | crate::ProductionOperation::AcceptFacilityCommissioning { .. }
        | crate::ProductionOperation::RetireFacility { .. } => None,
    }
}

fn operation_execution(
    operation: &crate::ProductionOperation,
) -> Option<crate::ProductionExecutionId> {
    match operation {
        crate::ProductionOperation::StartExecution { execution, .. } => Some(execution.id.clone()),
        crate::ProductionOperation::AdvanceExecution { execution, .. }
        | crate::ProductionOperation::CompleteExecution { execution, .. } => {
            Some(execution.clone())
        }
        _ => None,
    }
}

fn operation_project(operation: &crate::ProductionOperation) -> Option<crate::FacilityProjectId> {
    match operation {
        crate::ProductionOperation::CreateFacilityProject { project } => Some(project.id.clone()),
        crate::ProductionOperation::AuthorizeFacilityProject { project }
        | crate::ProductionOperation::AdvanceFacilityProject { project, .. }
        | crate::ProductionOperation::AcceptFacilityCommissioning { project } => {
            Some(project.clone())
        }
        _ => None,
    }
}

/// The archive objects a boundary may read, seen as a read-only resource
/// archive store.
struct ViewResourceArchive<'a>(&'a SimulationView<'a>);

impl canwu_resource::ResourceArchiveStore for ViewResourceArchive<'_> {
    fn store_resource_archive_object(
        &self,
        _namespace: &str,
        _object_id: &str,
        _bytes: &[u8],
    ) -> Result<(), canwu_resource::ResourceError> {
        Err(canwu_resource::ResourceError::InvalidLifecycle(
            "the boundary's resource archive view is read-only".to_owned(),
        ))
    }

    fn load_resource_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, canwu_resource::ResourceError> {
        self.0
            .plugin_archive_object(namespace, object_id)
            .map_err(|error| canwu_resource::ResourceError::InvalidDefinition(error.to_string()))
    }

    fn persist_resource_archive_retention(
        &self,
        _handle: &canwu_resource::ResourceArchiveRetentionHandleV1,
    ) -> Result<(), canwu_resource::ResourceError> {
        Err(canwu_resource::ResourceError::InvalidLifecycle(
            "the boundary's resource archive view keeps no retention".to_owned(),
        ))
    }

    fn finalize_resource_archive_retention(
        &self,
        _handle_id: &str,
        _phase: canwu_resource::ResourceArchiveRetentionPhaseV1,
    ) -> Result<(), canwu_resource::ResourceError> {
        Err(canwu_resource::ResourceError::InvalidLifecycle(
            "the boundary's resource archive view keeps no retention".to_owned(),
        ))
    }
}

/// Refuses an execution output the resource root could never settle: an
/// output set larger than a batch the root can hold under its limits, even
/// with an empty archive backlog (`InvalidPayload`), and an output key that a
/// resource operation outcome already holds, in the resource root or in its
/// archive (`IdempotencyConflict`). The resource plugin skips such a batch
/// and settles nothing, so the execution could never settle.
///
/// The archive is read through the cumulative lookup index, as the resource
/// plugin itself reads it for every operation: one directory, one page per
/// index level, and one blob on a hit. A lookup that cannot read the archive
/// fails the boundary, which settles once the store serves the object; it is
/// never taken for a free key. A key that a resource operation queued by the
/// host application takes after the execution started is not covered: the
/// credit then settles nothing.
fn refuse_output_keys_held_by_resource(
    view: &SimulationView<'_>,
    execution: &crate::ProductionExecution,
) -> Result<(), CanwuError> {
    let Some((_, resource)) = canwu_resource::resource_root_in_view(
        view,
        execution.output_requests.iter().map(|request| {
            canwu_resource::ResourceTerminalRecordKeyV1::Outcome(request.operation_key.clone())
        }),
    )?
    else {
        return Ok(());
    };
    // The output batch must be one the resource root can ever settle: one it
    // can never hold is skipped and lost, and the execution would never
    // settle.
    if !resource.production_output_batch_can_fit(execution.output_requests.len()) {
        return Err(CanwuError::new(
            ErrorCode::InvalidPayload,
            "execution output is a batch the resource root can never hold under its limits",
        ));
    }
    let archive = ViewResourceArchive(view);
    for request in &execution.output_requests {
        let held = resource.outcomes.contains_key(&request.operation_key)
            || canwu_resource::archived_resource_operation_outcome(
                &resource,
                &archive,
                &request.operation_key,
            )
            .map_err(|error| {
                CanwuError::new(
                    ErrorCode::InvalidArchive,
                    format!("resource archive lookup of an output key failed: {error}"),
                )
            })?
            .is_some();
        if held {
            return Err(CanwuError::new(
                ErrorCode::IdempotencyConflict,
                "execution output names an operation key a resource outcome already holds",
            ));
        }
    }
    Ok(())
}

/// Checks the evidence an operation cites outside the production root
/// against `production`, the root as the operations settled before it left
/// it.
fn validate_external_operation_evidence(
    view: &SimulationView<'_>,
    production: &ProductionState,
    holder: &KnowledgeHolderRef,
    operation: &crate::ProductionOperation,
) -> Result<(), CanwuError> {
    if let crate::ProductionOperation::CompleteExecution {
        execution,
        realized_output_per_mille,
        realization_evidence: Some(evidence),
    } = operation
    {
        // Holder, lifecycle, ratio, and declared-kind rules run first, so only
        // an admissible completion ever resolves its evidence record.
        production.completion_admission(
            holder,
            execution,
            *realized_output_per_mille,
            Some(evidence),
        )?;
        // Every seal keeps a record's current version, but drops an earlier
        // version's receipt unless a live dependency declares it, while exact
        // replay still finds that version. Only the current version is
        // therefore admitted the same way in a sealed run and its replay.
        if !view.domain_record_version_is_current(evidence)? {
            return Err(CanwuError::new(
                ErrorCode::DomainRecordNotFound,
                "production realization evidence is not the current exact version of its record",
            ));
        }
        return Ok(());
    }
    if let crate::ProductionOperation::CreateFacilityProject { project } = operation {
        validate_external_project_evidence(view, production, project)?;
        return Ok(());
    }
    if let crate::ProductionOperation::AdvanceFacilityProject { project, .. } = operation {
        let project = production
            .facility_projects
            .get(project)
            .ok_or_else(|| invalid("facility project provider validation record is unavailable"))?;
        // Creation validated the provider and technology evidence, and
        // neither its exact bodies nor the project's holder, site, process,
        // and time can change. A seal can remove those bodies while exact
        // replay keeps them, so only the live resource evidence is rechecked.
        validate_project_resource_evidence(view, production, project)?;
        return Ok(());
    }
    let crate::ProductionOperation::StartExecution { execution, .. } = operation else {
        return Ok(());
    };
    // Output keys that could never settle are refused before any evidence is
    // resolved: a key reserved for resource completion operations, one that
    // another unsettled execution names, and one a resource outcome already
    // holds, hot or archived.
    crate::model::refuse_reserved_output_keys(execution)?;
    production.refuse_output_keys_held_by_unsettled_executions(execution)?;
    refuse_output_keys_held_by_resource(view, execution)?;
    let order = production
        .work_orders
        .get(&execution.work_order)
        .ok_or_else(|| invalid("production provider validation work order is unavailable"))?;
    let process = production
        .processes
        .get(&execution.process)
        .ok_or_else(|| invalid("production provider validation process is unavailable"))?;
    let site = production
        .sites
        .get(&execution.site)
        .ok_or_else(|| invalid("production provider validation site is unavailable"))?;
    for binding in &execution.evidence {
        validate_authoritative_provider_binding(
            view,
            process,
            &order.holder,
            execution.started_at,
            site,
            binding,
        )?;
    }

    require_technology_binding_kinds(&execution.technology)?;
    let mut technology_records = Vec::new();
    for reference in [
        Some(&execution.technology.technique_revision),
        execution.technology.capability_qualification.as_ref(),
        execution.technology.implementation.as_ref(),
        execution.technology.adoption.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        technology_records.push(current_version_body(
            view,
            reference,
            "technology evidence",
        )?);
    }
    let technique_record = current_version_body(
        view,
        &execution.technology.technique_revision,
        "technique revision",
    )?;
    let _technique = technique_record.decode_payload::<canwu_technology::TechniqueRevision>()?;
    let qualification = execution
        .technology
        .capability_qualification
        .as_ref()
        .map(|reference| {
            current_version_body(view, reference, "capability qualification")?
                .decode_payload::<canwu_technology::CapabilityQualification>()
        })
        .transpose()?;
    if let Some(qualification) = &qualification
        && (qualification.holder != order.holder
            || qualification.site != site.place
            || qualification.revision != execution.technology.technique_revision
            || !qualification.active
            || qualification.valid_from > execution.started_at
            || qualification
                .valid_until
                .is_some_and(|until| execution.started_at >= until)
            || qualification
                .operator
                .as_ref()
                .is_some_and(|operator| operator != &holder_entity_ref(&order.holder)))
    {
        return Err(invalid(
            "production capability qualification does not authorize this holder/operator/site/revision/time",
        ));
    }
    let implementation = execution
        .technology
        .implementation
        .as_ref()
        .map(|reference| {
            current_version_body(view, reference, "implementation")?
                .decode_payload::<canwu_technology::ImplementationRecord>()
        })
        .transpose()?;
    if let Some(implementation) = &implementation
        && (implementation.owner != order.holder
            || implementation.site != site.place
            || implementation.revision != execution.technology.technique_revision
            || execution
                .technology
                .capability_qualification
                .as_ref()
                .is_none_or(|qualification| implementation.qualification != *qualification)
            || !implementation.active
            || implementation.capacity == 0
            || implementation.installed_at > execution.started_at)
    {
        return Err(invalid(
            "production implementation does not authorize this holder/site/revision/qualification/time",
        ));
    }
    let adoption = execution
        .technology
        .adoption
        .as_ref()
        .map(|reference| {
            current_version_body(view, reference, "adoption")?
                .decode_payload::<canwu_technology::AdoptionRecord>()
        })
        .transpose()?;
    if let Some(adoption) = &adoption {
        require_kind::<canwu_technology::ApplicationSpec>(
            &adoption.application,
            "adoption application",
        )?;
        let application =
            current_version_body(view, &adoption.application, "adoption application")?
                .decode_payload::<canwu_technology::ApplicationSpec>()?;
        if adoption.adopter != order.holder
            || adoption.site != site.place
            || adoption.status != canwu_technology::AdoptionStatus::Committed
            || adoption.scale == 0
            || application.technique != execution.technology.technique_revision
            || execution
                .technology
                .implementation
                .as_ref()
                .is_none_or(|implementation| !adoption.implementations.contains(implementation))
        {
            return Err(invalid(
                "production adoption does not authorize this holder/site/application/implementation",
            ));
        }
    }
    if process.adoption_required != adoption.is_some() {
        return Err(invalid(
            "production process adoption requirement differs from the exact adoption evidence",
        ));
    }
    if execution.technology.semantic_digest
        != canonical_hash(
            "canwu.production.technology-binding.v1",
            &technology_records,
        )?
    {
        return Err(invalid(
            "production technology binding digest differs from its exact record bodies",
        ));
    }

    let resource_state = resource_root(
        view,
        resource_input_keys(&execution.inputs),
        "production input resource runtime is unavailable",
    )?;
    resource_state.validate().map_err(|error| {
        invalid(format!(
            "production input resource state is invalid: {error}"
        ))
    })?;
    resource_state
        .external_completion_participants
        .validate(&resource_state.run_budget)
        .map_err(|error| {
            invalid(format!(
                "production input resource completion state is invalid: {error}"
            ))
        })?;
    let participant = resource_state
        .external_completion_participants
        .grants
        .get(&execution.completion_certificate.acquisition)
        .ok_or_else(|| invalid("production execution resource participant grant is absent"))?;
    let resource_grant = &participant.grant;
    if participant.certificate.as_ref() != Some(&execution.completion_certificate)
        || resource_grant.id != execution.resource_completion_grant
        || resource_grant.acquisition != execution.completion_certificate.acquisition
        || resource_grant.operation_key != execution.completion_certificate.operation_key
        || resource_grant.owner_plugin != canwu_resource::PLUGIN_NAME
        || !matches!(
            resource_grant.state,
            canwu_resource::CompletionGrantStateV1::Consumed
                | canwu_resource::CompletionGrantStateV1::Completed
        )
        || !execution.completion_certificate.prepared_grants.iter().any(
            |(id, prepared_revision)| {
                id == &resource_grant.id
                    && (resource_grant.revision == *prepared_revision
                        || resource_grant.revision.get()
                            == prepared_revision.get().saturating_add(1))
            },
        )
        || execution.output_requests.iter().any(|request| {
            !execution
                .completion_certificate
                .locked_target_versions
                .contains(&canwu_resource::CompletionLockedTargetV1::Account {
                    id: request.account.clone(),
                    revision: request.expected_account_revision,
                })
        })
    {
        return Err(invalid(
            "production execution lacks the exact authoritative resource completion activation",
        ));
    }
    for input in &execution.inputs {
        let consumption = resource_state
            .consumptions
            .get(&input.consumption.id)
            .ok_or_else(|| invalid("production input consumption is unavailable"))?;
        if canwu_resource::ResourceConsumptionVersionV1::from(consumption) != input.consumption {
            return Err(invalid(
                "production input consumption differs from its exact resource evidence",
            ));
        }
        let leg = resource_state
            .allocation_legs
            .get(&input.allocation_leg.id)
            .ok_or_else(|| invalid("production input allocation leg is unavailable"))?;
        if leg.account != input.allocation_leg.account
            || leg.resource_revision != input.allocation_leg.resource_revision
            || leg.unit_revision != input.allocation_leg.unit_revision
            || leg.quantity != input.allocation_leg.quantity
            || leg.status != canwu_resource::AllocationLegStatus::Consumed
        {
            return Err(invalid(
                "production input allocation is not the exact consumed resource leg",
            ));
        }
        if !execution
            .completion_certificate
            .locked_target_versions
            .contains(&canwu_resource::CompletionLockedTargetV1::AllocationLeg {
                id: input.allocation_leg.id.clone(),
                revision: input.allocation_leg.revision,
            })
        {
            return Err(invalid(
                "production input allocation is absent from the activation certificate locks",
            ));
        }
        let outcome = resource_state
            .outcomes
            .get(&input.consumption_outcome.operation_key)
            .ok_or_else(|| invalid("production input resource outcome is unavailable"))?;
        if canwu_resource::ResourceOperationOutcomeVersionV1::from(outcome)
            != input.consumption_outcome
        {
            return Err(invalid(
                "production input outcome differs from its exact resource acknowledgement",
            ));
        }
    }
    Ok(())
}

fn validate_external_project_evidence(
    view: &SimulationView<'_>,
    production: &crate::ProductionState,
    project: &crate::FacilityProject,
) -> Result<(), CanwuError> {
    let process = production
        .processes
        .get(&project.process)
        .ok_or_else(|| invalid("facility project provider validation process is unavailable"))?;
    let site = production
        .sites
        .get(&project.site)
        .ok_or_else(|| invalid("facility project provider validation site is unavailable"))?;
    for binding in &project.evidence {
        validate_authoritative_provider_binding(
            view,
            process,
            &project.holder,
            project.created_at,
            site,
            binding,
        )?;
    }
    validate_authoritative_technology_binding(
        view,
        process,
        &project.holder,
        project.created_at,
        site,
        &project.technology,
    )?;
    validate_project_resource_evidence(view, production, project)
}

fn validate_project_resource_evidence(
    view: &SimulationView<'_>,
    production: &crate::ProductionState,
    project: &crate::FacilityProject,
) -> Result<(), CanwuError> {
    if authoritative_resource_inputs_are_hot(view, &project.inputs)? {
        validate_authoritative_resource_inputs(view, &project.inputs)?;
    } else {
        let witness = production
            .resource_continuation_witnesses
            .get(&project.id)
            .ok_or_else(|| {
                invalid(
                    "facility project compacted resource evidence lacks an authenticated continuation witness",
                )
            })?;
        validate_resource_continuation_witness(view, production, witness)?;
    }
    validate_project_resource_completion(view, project)
}

fn authoritative_resource_inputs_are_hot(
    view: &SimulationView<'_>,
    inputs: &[crate::ResourceInputBinding],
) -> Result<bool, CanwuError> {
    let state = resource_root(
        view,
        resource_input_keys(inputs),
        "production input resource runtime is unavailable",
    )?;
    Ok(inputs.iter().all(|input| {
        state.consumptions.contains_key(&input.consumption.id)
            && state
                .outcomes
                .contains_key(&input.consumption_outcome.operation_key)
    }))
}

fn validate_resource_continuation_witness(
    view: &SimulationView<'_>,
    production: &crate::ProductionState,
    witness: &crate::ProductionResourceContinuationWitnessV1,
) -> Result<(), CanwuError> {
    let project = production
        .facility_projects
        .get(&witness.project)
        .ok_or_else(|| invalid("production continuation witness project is unavailable"))?;
    let mut detached = witness.clone();
    let recorded = std::mem::take(&mut detached.semantic_digest);
    // The archive head is in the live root.
    let resource = resource_root(
        view,
        [],
        "production continuation resource runtime is unavailable",
    )?;
    if resource.archive_head.archived_record_count < witness.resource_archive_record_count
        || (resource.archive_head.archived_record_count == witness.resource_archive_record_count
            && resource.archive_head.directory_root.as_deref()
                != Some(witness.resource_archive_directory_root.as_str()))
        || witness.input_bindings_digest
            != crate::model::resource_input_bindings_digest(&project.inputs)?
        || recorded
            != canonical_hash(
                "canwu.production.resource-continuation-witness.v1",
                &detached,
            )?
    {
        return Err(invalid(
            "production resource continuation witness is stale or forged",
        ));
    }
    Ok(())
}

fn validate_project_resource_completion(
    view: &SimulationView<'_>,
    project: &crate::FacilityProject,
) -> Result<(), CanwuError> {
    // The participant grant is active: it is in the live root.
    let state = resource_root(
        view,
        [],
        "production project resource runtime is unavailable",
    )?;
    state.validate().map_err(|error| {
        invalid(format!(
            "production project resource state is invalid: {error}"
        ))
    })?;
    let participant = state
        .external_completion_participants
        .grants
        .get(&project.completion_certificate.acquisition)
        .ok_or_else(|| invalid("production project resource participant grant is absent"))?;
    if participant.certificate.as_ref() != Some(&project.completion_certificate)
        || participant.grant.id != project.resource_completion_grant
        || participant.grant.operation_key != project.operation_key
        || participant.grant.owner_plugin != canwu_resource::PLUGIN_NAME
        || participant.grant.state != canwu_resource::CompletionGrantStateV1::Consumed
        || project.inputs.iter().any(|input| {
            !project
                .completion_certificate
                .locked_target_versions
                .contains(&canwu_resource::CompletionLockedTargetV1::AllocationLeg {
                    id: input.allocation_leg.id.clone(),
                    revision: input.allocation_leg.revision,
                })
        })
    {
        return Err(invalid(
            "production project lacks exact authoritative resource completion activation",
        ));
    }
    Ok(())
}

fn validate_authoritative_technology_binding(
    view: &SimulationView<'_>,
    process: &crate::ProcessRevision,
    holder: &canwu_api::KnowledgeHolderRef,
    effective_at: SimTime,
    site: &crate::ProductionSite,
    binding: &crate::TechnologyEvidenceBinding,
) -> Result<(), CanwuError> {
    require_technology_binding_kinds(binding)?;
    let mut records = Vec::new();
    for reference in [
        Some(&binding.technique_revision),
        binding.capability_qualification.as_ref(),
        binding.implementation.as_ref(),
        binding.adoption.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        records.push(current_version_body(
            view,
            reference,
            "technology evidence",
        )?);
    }
    let technique_record =
        current_version_body(view, &binding.technique_revision, "technique revision")?;
    let _technique = technique_record.decode_payload::<canwu_technology::TechniqueRevision>()?;
    let qualification = binding
        .capability_qualification
        .as_ref()
        .map(|reference| {
            current_version_body(view, reference, "capability qualification")?
                .decode_payload::<canwu_technology::CapabilityQualification>()
        })
        .transpose()?;
    if let Some(qualification) = &qualification
        && (qualification.holder != *holder
            || qualification.site != site.place
            || qualification.revision != binding.technique_revision
            || !qualification.active
            || qualification.valid_from > effective_at
            || qualification
                .valid_until
                .is_some_and(|until| effective_at >= until)
            || qualification
                .operator
                .as_ref()
                .is_some_and(|operator| operator != &holder_entity_ref(holder)))
    {
        return Err(invalid(
            "production capability qualification does not authorize this holder/operator/site/revision/time",
        ));
    }
    let implementation = binding
        .implementation
        .as_ref()
        .map(|reference| {
            current_version_body(view, reference, "implementation")?
                .decode_payload::<canwu_technology::ImplementationRecord>()
        })
        .transpose()?;
    if let Some(implementation) = &implementation
        && (implementation.owner != *holder
            || implementation.site != site.place
            || implementation.revision != binding.technique_revision
            || binding
                .capability_qualification
                .as_ref()
                .is_none_or(|qualification| implementation.qualification != *qualification)
            || !implementation.active
            || implementation.capacity == 0
            || implementation.installed_at > effective_at)
    {
        return Err(invalid(
            "production implementation does not authorize this holder/site/revision/qualification/time",
        ));
    }
    let adoption = binding
        .adoption
        .as_ref()
        .map(|reference| {
            current_version_body(view, reference, "adoption")?
                .decode_payload::<canwu_technology::AdoptionRecord>()
        })
        .transpose()?;
    if let Some(adoption) = &adoption {
        require_kind::<canwu_technology::ApplicationSpec>(
            &adoption.application,
            "adoption application",
        )?;
        let application =
            current_version_body(view, &adoption.application, "adoption application")?
                .decode_payload::<canwu_technology::ApplicationSpec>()?;
        if adoption.adopter != *holder
            || adoption.site != site.place
            || adoption.status != canwu_technology::AdoptionStatus::Committed
            || adoption.scale == 0
            || application.technique != binding.technique_revision
            || binding
                .implementation
                .as_ref()
                .is_none_or(|implementation| !adoption.implementations.contains(implementation))
        {
            return Err(invalid(
                "production adoption does not authorize this holder/site/application/implementation",
            ));
        }
    }
    if process.adoption_required != adoption.is_some()
        || binding.semantic_digest
            != canonical_hash("canwu.production.technology-binding.v1", &records)?
    {
        return Err(invalid(
            "production technology binding differs from its authoritative record closure",
        ));
    }
    Ok(())
}

fn validate_authoritative_resource_inputs(
    view: &SimulationView<'_>,
    inputs: &[crate::ResourceInputBinding],
) -> Result<(), CanwuError> {
    let state = resource_root(
        view,
        resource_input_keys(inputs),
        "production input resource runtime is unavailable",
    )?;
    state.validate().map_err(|error| {
        invalid(format!(
            "production input resource state is invalid: {error}"
        ))
    })?;
    for input in inputs {
        let consumption = state
            .consumptions
            .get(&input.consumption.id)
            .ok_or_else(|| invalid("production input consumption is unavailable"))?;
        let leg = state
            .allocation_legs
            .get(&input.allocation_leg.id)
            .ok_or_else(|| invalid("production input allocation leg is unavailable"))?;
        let outcome = state
            .outcomes
            .get(&input.consumption_outcome.operation_key)
            .ok_or_else(|| invalid("production input resource outcome is unavailable"))?;
        if canwu_resource::ResourceConsumptionVersionV1::from(consumption) != input.consumption
            || leg.account != input.allocation_leg.account
            || leg.resource_revision != input.allocation_leg.resource_revision
            || leg.unit_revision != input.allocation_leg.unit_revision
            || leg.quantity != input.allocation_leg.quantity
            || leg.status != canwu_resource::AllocationLegStatus::Consumed
            || canwu_resource::ResourceOperationOutcomeVersionV1::from(outcome)
                != input.consumption_outcome
        {
            return Err(invalid(
                "production project input differs from its exact consumed resource evidence",
            ));
        }
    }
    Ok(())
}

fn validate_authoritative_provider_binding(
    view: &SimulationView<'_>,
    process: &crate::ProcessRevision,
    holder: &canwu_api::KnowledgeHolderRef,
    effective_at: SimTime,
    site: &crate::ProductionSite,
    binding: &crate::ProductionEvidenceBinding,
) -> Result<(), CanwuError> {
    // A record of another kind is refused before it is read: the plugin
    // declares the reads of the provider kinds only.
    let kind = &binding.version.record.kind;
    if !(kind.matches_type::<canwu_technology::AssetBinding>()
        || kind.matches_type::<canwu_technology::CapabilityQualification>()
        || kind.matches_type::<canwu_technology::ImplementationRecord>()
        || kind.matches_type::<canwu_technology::AdoptionRecord>()
        || kind.matches_type::<canwu_technology::AttemptObservation>())
    {
        return Err(invalid(
            "production provider evidence is not a supported typed provider record",
        ));
    }
    let record = current_version_body(view, &binding.version, "provider evidence")?;
    let (allowed_kinds, explicit_capabilities, available_quantity) =
        if record
            .reference
            .kind
            .matches_type::<canwu_technology::AssetBinding>()
        {
            let asset = record.decode_payload::<canwu_technology::AssetBinding>()?;
            if asset.owner != *holder
                || asset.site != site.place
                || !asset.active
                || asset.condition_per_mille == 0
            {
                return Err(invalid(
                    "production asset binding is inactive or belongs to another holder/site",
                ));
            }
            (
                vec![
                    crate::ProductionRequirementKind::ToolsMachines,
                    crate::ProductionRequirementKind::Energy,
                    crate::ProductionRequirementKind::Maintenance,
                    crate::ProductionRequirementKind::Security,
                    crate::ProductionRequirementKind::Access,
                    crate::ProductionRequirementKind::Authorization,
                    crate::ProductionRequirementKind::FinanceOrganization,
                ],
                asset.capabilities,
                1,
            )
        } else if record
            .reference
            .kind
            .matches_type::<canwu_technology::CapabilityQualification>()
        {
            let qualification =
                record.decode_payload::<canwu_technology::CapabilityQualification>()?;
            if qualification.holder != *holder
                || qualification.site != site.place
                || !qualification.active
                || qualification.valid_from > SimTime::EPOCH.max(effective_at)
                || qualification
                    .valid_until
                    .is_some_and(|until| effective_at >= until)
            {
                return Err(invalid(
                    "production capability qualification is inactive or belongs to another holder/site/time",
                ));
            }
            (
                vec![crate::ProductionRequirementKind::LaborCapability],
                vec![qualification.operation],
                1,
            )
        } else if record
            .reference
            .kind
            .matches_type::<canwu_technology::ImplementationRecord>()
        {
            let implementation =
                record.decode_payload::<canwu_technology::ImplementationRecord>()?;
            if implementation.owner != *holder
                || implementation.site != site.place
                || !implementation.active
                || implementation.capacity == 0
                || implementation.installed_at > effective_at
            {
                return Err(invalid(
                    "production implementation is inactive or belongs to another holder/site/time",
                ));
            }
            (
                vec![crate::ProductionRequirementKind::TechnologyImplementation],
                Vec::new(),
                implementation.capacity,
            )
        } else if record
            .reference
            .kind
            .matches_type::<canwu_technology::AdoptionRecord>()
        {
            let adoption = record.decode_payload::<canwu_technology::AdoptionRecord>()?;
            if adoption.adopter != *holder
                || adoption.site != site.place
                || adoption.status != canwu_technology::AdoptionStatus::Committed
                || adoption.scale == 0
            {
                return Err(invalid(
                    "production adoption is not committed for this holder/site",
                ));
            }
            (
                vec![crate::ProductionRequirementKind::TechnologyImplementation],
                Vec::new(),
                adoption.scale,
            )
        } else if record
            .reference
            .kind
            .matches_type::<canwu_technology::AttemptObservation>()
        {
            let observation = record.decode_payload::<canwu_technology::AttemptObservation>()?;
            require_kind::<canwu_technology::ExperimentAttempt>(
                &observation.attempt,
                "environment observation attempt",
            )?;
            let attempt = current_version_body(
                view,
                &observation.attempt,
                "environment observation attempt",
            )?
            .decode_payload::<canwu_technology::ExperimentAttempt>()?;
            if observation.observer != *holder
                || observation.observed_at > effective_at
                || observation.uncertainty_per_mille > 1_000
                || attempt.operator != *holder
                || attempt.site != site.place
                || attempt.ended_at > observation.observed_at
                || !attempt.evaluation.passed
            {
                return Err(invalid(
                    "production environment observation does not bind the holder/site/time/passed attempt",
                ));
            }
            let environment_requirements = process
                .requirements
                .iter()
                .filter(|group| group.kind == crate::ProductionRequirementKind::EnvironmentSeason)
                .flat_map(|group| group.any_of.iter())
                .collect::<Vec<_>>();
            let matches = observation
                .values
                .iter()
                .filter_map(|value| {
                    let capability = value.metric.record.id.clone();
                    let required = environment_requirements
                        .iter()
                        .any(|alternative| alternative.capability == capability);
                    let exact_attempt_value = attempt.environment.iter().any(|environment| {
                        environment.metric == value.metric && environment.value == value.value
                    });
                    (required && exact_attempt_value && value.value > 0)
                        .then_some((capability, u64::try_from(value.value).ok()?))
                })
                .collect::<Vec<_>>();
            let [(capability, quantity)] = matches.as_slice() else {
                return Err(invalid(
                    "production environment observation must resolve one exact process metric",
                ));
            };
            (
                vec![crate::ProductionRequirementKind::EnvironmentSeason],
                vec![capability.clone()],
                *quantity,
            )
        } else {
            return Err(invalid(
                "production provider evidence is not a supported typed provider record",
            ));
        };
    let mut candidates = BTreeSet::new();
    for group in &process.requirements {
        if !allowed_kinds.contains(&group.kind) {
            continue;
        }
        for alternative in &group.any_of {
            if explicit_capabilities.is_empty()
                || explicit_capabilities.contains(&alternative.capability)
            {
                candidates.insert((group.kind, alternative.capability.clone()));
            }
        }
    }
    if candidates.len() != 1 {
        return Err(invalid(
            "production typed provider evidence does not resolve one unambiguous process requirement",
        ));
    }
    let (kind, capability) = candidates.into_iter().next().expect("one candidate");
    let expected_digest = canonical_hash("canwu.production.provider-evidence.v1", &record)?;
    if binding.kind != kind
        || binding.capability != capability
        || binding.available_quantity != available_quantity
        || binding.semantic_digest != expected_digest
    {
        return Err(invalid(
            "production provider binding differs from its authoritative typed record body",
        ));
    }
    Ok(())
}

/// Refuses a cited version that is not of the record kind `T`, before it is
/// read: a read of a kind the plugin does not declare would fail the
/// boundary, and the citation is the packet's error.
fn require_kind<T: canwu_api::DomainRecordType>(
    version: &canwu_api::DomainRecordVersionRef,
    label: &str,
) -> Result<(), CanwuError> {
    if version.record.kind.matches_type::<T>() {
        Ok(())
    } else {
        Err(invalid(format!(
            "production {label} is not a record of its expected kind"
        )))
    }
}

/// Refuses a technology binding whose citations are not of their kinds
/// (technique revision, capability qualification, implementation, adoption)
/// before any of them is read.
fn require_technology_binding_kinds(
    binding: &crate::TechnologyEvidenceBinding,
) -> Result<(), CanwuError> {
    require_kind::<canwu_technology::TechniqueRevision>(
        &binding.technique_revision,
        "technique revision",
    )?;
    if let Some(reference) = &binding.capability_qualification {
        require_kind::<canwu_technology::CapabilityQualification>(
            reference,
            "capability qualification",
        )?;
    }
    if let Some(reference) = &binding.implementation {
        require_kind::<canwu_technology::ImplementationRecord>(reference, "implementation")?;
    }
    if let Some(reference) = &binding.adoption {
        require_kind::<canwu_technology::AdoptionRecord>(reference, "adoption")?;
    }
    Ok(())
}

/// Reads the body of an exact version that a new execution or facility project
/// cites, directly or through another cited record. Only a record's current
/// version, or one proposed earlier in this boundary, qualifies: its body
/// survives every seal, while a seal can remove an earlier version's body that
/// exact replay still reads.
fn current_version_body(
    view: &SimulationView<'_>,
    version: &canwu_api::DomainRecordVersionRef,
    label: &str,
) -> Result<DomainRecord, CanwuError> {
    if !view.domain_record_version_is_current(version)? {
        return Err(invalid(format!(
            "production {label} is not the current exact version of its record"
        )));
    }
    view.domain_record_version(version)?.ok_or_else(|| {
        CanwuError::new(
            ErrorCode::EvidenceContentUnavailable,
            format!("production {label} body is unavailable"),
        )
    })
}

/// The resource root as a production boundary or admission reads it: the
/// live root, with the hot terminal records `keys` name attached
/// ([`canwu_resource::resource_root_in_view`]).
fn resource_root(
    view: &SimulationView<'_>,
    keys: impl IntoIterator<Item = canwu_resource::ResourceTerminalRecordKeyV1>,
    unavailable: &str,
) -> Result<canwu_resource::ResourceState, CanwuError> {
    canwu_resource::resource_root_in_view(view, keys)?
        .map(|(_, state)| state)
        .ok_or_else(|| invalid(unavailable))
}

/// The resource records production inputs cite: each consumption, its
/// allocation leg, and the outcome that settled it.
fn resource_input_keys(
    inputs: &[crate::ResourceInputBinding],
) -> Vec<canwu_resource::ResourceTerminalRecordKeyV1> {
    inputs
        .iter()
        .flat_map(|input| {
            [
                canwu_resource::ResourceTerminalRecordKeyV1::Consumption(
                    input.consumption.id.clone(),
                ),
                canwu_resource::ResourceTerminalRecordKeyV1::AllocationLeg(
                    input.allocation_leg.id.clone(),
                ),
                canwu_resource::ResourceTerminalRecordKeyV1::Outcome(
                    input.consumption_outcome.operation_key.clone(),
                ),
            ]
        })
        .collect()
}

fn holder_entity_ref(holder: &KnowledgeHolderRef) -> canwu_api::EntityRef {
    match holder {
        KnowledgeHolderRef::Person(person) => canwu_api::EntityRef::Person(*person),
        KnowledgeHolderRef::Entity(entity) => entity.clone(),
    }
}

fn validate_production_completion_ingress(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    state: &ProductionState,
    operation: &crate::ProductionCompletionIngressV1,
) -> Result<(), CanwuError> {
    use crate::ProductionCompletionIngressV1 as Ingress;
    match operation {
        Ingress::GrantLocal(request) => {
            if request.current_boundary != context.boundary_id.get()
                || request.owner_plugin != PLUGIN_NAME
            {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "production completion grant must use the canonical boundary and owner",
                ));
            }
            validate_production_locked_targets(view, state, &request.target_versions)?;
        }
        Ingress::PrepareLocal(request) => {
            if request.current_boundary != context.boundary_id.get() {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "production completion prepare must use the canonical boundary",
                ));
            }
        }
        Ingress::Activate {
            acquisition,
            current_boundary,
            ..
        } => {
            if *current_boundary != context.boundary_id.get() {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "production completion activation must use the canonical boundary",
                ));
            }
            if state
                .completion_acquisitions
                .get(acquisition)
                .is_none_or(|value| value.eligibility_time != context.at)
            {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "production completion activation must occur at its canonical eligibility time",
                ));
            }
        }
        Ingress::Expire(request) => {
            if request.current_boundary != context.boundary_id.get() || request.at != context.at {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "production completion expiry must use the canonical boundary time",
                ));
            }
        }
        Ingress::AcknowledgeParticipantGrant {
            acquisition,
            participant,
            provider_source,
            grant,
            ..
        }
        | Ingress::AcknowledgeParticipantPrepared {
            acquisition,
            participant,
            provider_source,
            grant,
            ..
        }
        | Ingress::AcknowledgeParticipantConsumed {
            acquisition,
            participant,
            provider_source,
            grant,
            ..
        }
        | Ingress::AcknowledgeParticipantReleased {
            acquisition,
            participant,
            provider_source,
            grant,
            ..
        } => {
            if participant != canwu_resource::PLUGIN_NAME
                || grant.owner_plugin != canwu_resource::PLUGIN_NAME
                || grant.acquisition != *acquisition
            {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "production completion participant acknowledgement has the wrong owner",
                ));
            }
            // A source of another kind is refused before it is read: only the
            // resource runtime's read is declared.
            if !provider_source
                .record
                .kind
                .matches_type::<canwu_resource::ResourceRuntimeRecord>()
            {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "production completion participant source is not the resource runtime",
                ));
            }
            if !view.domain_record_version_is_current(provider_source)? {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "production completion participant source is not the current exact version",
                ));
            }
            let record = view
                .domain_record_version(provider_source)?
                .ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::EvidenceContentUnavailable,
                        "production completion participant source body is unavailable",
                    )
                })?;
            if record.owner != canwu_resource::PLUGIN_NAME
                || !record
                    .reference
                    .kind
                    .matches_type::<canwu_resource::ResourceRuntimeRecord>()
            {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "production completion participant source is not the resource runtime",
                ));
            }
            // An active participant grant is in the live root.
            let resource = record
                .decode_payload::<canwu_resource::ResourceRuntimeRecord>()?
                .into_live();
            let authoritative = resource
                .external_completion_participants
                .grants
                .get(acquisition)
                .ok_or_else(|| invalid("resource participant grant is unavailable"))?;
            if authoritative.grant != *grant
                || authoritative.coordinator_plugin != PLUGIN_NAME
                || authoritative.coordinator_source.record
                    != state.runtime_reference().into_untyped()
            {
                return Err(invalid(
                    "production completion participant acknowledgement differs from the authoritative resource grant",
                ));
            }
        }
        Ingress::AcknowledgeParticipantCompleted {
            acquisition,
            participant,
        } => {
            if participant != canwu_resource::PLUGIN_NAME {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "production completion acknowledgement has the wrong participant",
                ));
            }
            let resource = resource_root(
                view,
                [
                    canwu_resource::ResourceTerminalRecordKeyV1::ExternalCompletionParticipant(
                        acquisition.clone(),
                    ),
                ],
                "resource completion runtime is unavailable",
            )?;
            let authoritative = resource
                .external_completion_participants
                .participant(acquisition)
                .ok_or_else(|| invalid("completed resource participant is unavailable"))?;
            if authoritative.coordinator_plugin != PLUGIN_NAME
                || authoritative.coordinator_source.record
                    != state.runtime_reference().into_untyped()
                || authoritative.grant.state != canwu_resource::CompletionGrantStateV1::Completed
            {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "resource participant has not authoritatively completed for production",
                ));
            }
        }
    }
    Ok(())
}

fn validate_production_locked_targets(
    view: &SimulationView<'_>,
    state: &ProductionState,
    targets: &[canwu_resource::CompletionLockedTargetV1],
) -> Result<(), CanwuError> {
    if targets.is_empty() {
        return Err(invalid("production completion local target set is empty"));
    }
    let own = state.runtime_reference().into_untyped();
    for target in targets {
        let canwu_resource::CompletionLockedTargetV1::ExternalRecord { version } = target else {
            return Err(invalid(
                "production completion local capacity may lock only exact production records",
            ));
        };
        // A root's lease locks only that root's own record.
        if version.record != own {
            return Err(CanwuError::new(
                ErrorCode::InvalidAuthority,
                "production completion local grant may lock only its own root",
            ));
        }
        if !view.domain_record_version_is_current(version)? {
            return Err(CanwuError::new(
                ErrorCode::InvalidAuthority,
                "production completion target is not the current exact record version",
            ));
        }
        let record = view.domain_record_version(version)?.ok_or_else(|| {
            CanwuError::new(
                ErrorCode::EvidenceContentUnavailable,
                "production completion target body is unavailable",
            )
        })?;
        if record.owner != PLUGIN_NAME {
            return Err(CanwuError::new(
                ErrorCode::InvalidAuthority,
                "production completion local grant cannot lock another plugin's record",
            ));
        }
    }
    Ok(())
}

fn validate_output_ack_source(
    view: &SimulationView<'_>,
    acknowledgement: &ProductionOutputAcknowledgement,
) -> Result<(), CanwuError> {
    let resource_state = resource_root(
        view,
        acknowledgement.outcomes.iter().map(|outcome| {
            canwu_resource::ResourceTerminalRecordKeyV1::Outcome(outcome.operation_key.clone())
        }),
        "production output resource runtime is unavailable",
    )?;
    if acknowledgement.outcomes.is_empty()
        || acknowledgement
            .outcomes
            .iter()
            .any(|outcome| resource_state.outcomes.get(&outcome.operation_key) != Some(outcome))
    {
        return Err(invalid(
            "production output acknowledgement differs from the resource-owned outcome",
        ));
    }
    // The execution pinned this exact source when its output was dispatched,
    // and the acknowledgement must name it (`acknowledge_output`), so no
    // retained body of that version is read here.
    Ok(())
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

/// A finding of the admission checks about the command's own content, as a
/// code the engine records as a command rejection (`InvalidPayload`): the
/// engine fails a boundary for any other code a command handler returns.
fn admission_rejection(error: CanwuError) -> CanwuError {
    match error.code {
        ErrorCode::InvalidDomainRecord
        | ErrorCode::InvalidDecision
        | ErrorCode::DomainRecordNotFound
        | ErrorCode::DomainRecordVersionConflict
        | ErrorCode::DuplicateDomainRecord => {
            CanwuError::new(ErrorCode::InvalidPayload, error.message)
        }
        _ => error,
    }
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
        _ => "production_operation_rejected",
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

fn encode_error(error: &serde_json::Error) -> CanwuError {
    CanwuError::new(
        ErrorCode::InvalidPayload,
        format!("production payload could not be encoded: {error}"),
    )
}
