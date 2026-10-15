use crate::model::{
    ProductionCommandEnvelope, ProductionOperationDisposition, ProductionOperationOutcome,
    ProductionOutputAcknowledgement, ProductionRuntimeRecord, ProductionState, invalid,
    production_runtime_reference,
};
use crate::{PLUGIN_NAME, PLUGIN_NAMESPACE};
use canwu_api::{
    ArchiveReachabilityManifest, BoundaryContext, BoundaryDirective, BoundaryPhase,
    BoundaryProposal, BoundarySystemContract, Canwu, CanwuError, CauseRef, Command, CommandContext,
    CommandIngress, DecisionAction, DecisionTicketState, DomainRecord, DomainRecordMutation,
    DomainRecordSchema, ErrorCode, EvidenceRef, IngressClass, IngressPayload, Issuer,
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
use std::{collections::BTreeSet, sync::OnceLock};

pub const PRODUCTION_COMMAND: &str = "apply_production_operation_v1";
/// Event of a production packet that settles nothing because it does not
/// decode, or because an operation packet does not match an admitted tracked
/// production command.
const INGRESS_REFUSED_EVENT: &str = "canwu.production.ingress_refused.v1";
/// Event of an output acknowledgement that settles nothing.
const OUTPUT_ACK_REFUSED_EVENT: &str = "canwu.production.output_ack_refused.v1";
const PRODUCTION_COMMAND_INGRESS: &str = "production_operation_v1";
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
pub const PRODUCTION_SEMANTIC_HASH: &str =
    "197b4a0e784ffdf94e0e87e021f9420d121171d76c840951572685802d6c85a4";
const REPORT_SCHEMA_HASH: &str = "2e84d66c85841a251a94aa15fa4fd477d29136ed31c8434c5dd61dc92156fdf8";
static PRODUCTION_ARCHIVE_COMMIT_PERMIT: OnceLock<PluginIngressPermit> = OnceLock::new();
static PRODUCTION_ARCHIVE_ACK_PERMIT: OnceLock<PluginIngressPermit> = OnceLock::new();
static PRODUCTION_COMPLETION_INGRESS_PERMIT: OnceLock<PluginIngressPermit> = OnceLock::new();
static PRODUCTION_RESOURCE_CONTINUATION_PERMIT: OnceLock<PluginIngressPermit> = OnceLock::new();

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProductionArchiveIngressReceiptV1 {
    pub ingress: canwu_api::IngressReceipt,
    pub retention_handle_id: String,
    pub directory_root: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct ProductionArchiveRetentionAcknowledgementV1 {
    handle_id: String,
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

#[derive(Clone, Copy, Debug, Default)]
pub struct ProductionPlugin;

#[must_use]
pub fn production_command_descriptor() -> PluginActionDescriptor {
    let mut reads = vec![
        StateKey::core_decisions(),
        StateKey::new(PLUGIN_NAMESPACE, "runtime"),
    ];
    reads.extend(production_external_state_keys());
    reads.sort();
    reads.dedup();
    PluginActionDescriptor {
        name: PRODUCTION_COMMAND.to_owned(),
        description: "Submit one authority-checked production lifecycle operation".to_owned(),
        payload_schema: PayloadSchema::Any,
        reads,
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
        DomainRecordSchema::for_record::<canwu_technology::TechniqueRevision>().state_key(),
        DomainRecordSchema::for_record::<canwu_technology::CapabilityQualification>().state_key(),
        DomainRecordSchema::for_record::<canwu_technology::ImplementationRecord>().state_key(),
        DomainRecordSchema::for_record::<canwu_technology::AdoptionRecord>().state_key(),
        DomainRecordSchema::for_record::<canwu_technology::ExperimentAttempt>().state_key(),
        DomainRecordSchema::for_record::<canwu_technology::AttemptObservation>().state_key(),
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
        let mut found = false;
        for record in records.iter().filter(|record| {
            record
                .reference
                .kind
                .matches_type::<ProductionRuntimeRecord>()
        }) {
            if found || record.reference != production_runtime_reference().into_untyped() {
                return Err(invalid(
                    "production activation contains multiple or misidentified runtime roots",
                ));
            }
            let state = record.decode_payload::<ProductionRuntimeRecord>()?;
            state.validate()?;
            if state.draft()?.payload != record.payload {
                return Err(invalid(
                    "production activation root is not canonically encoded",
                ));
            }
            found = true;
        }
        Ok(())
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut runtime_schema = DomainRecordSchema::for_record::<ProductionRuntimeRecord>();
        runtime_schema.payload_schema = PayloadSchema::Object {
            properties: std::collections::BTreeMap::from([(
                PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD.to_owned(),
                payload_required_evidence_continuation_property_v1(),
            )]),
            allow_additional: true,
        };
        registrar.register_record_schema(runtime_schema)?;
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
        };
        registrar.register_knowledge_schema(report_schema.clone())?;
        registrar.register_archive_reachability_participant(production_archive_reachability)?;
        registrar.register_command(production_command_descriptor(), admit_production_operation)?;
        registrar.register_ingress(PluginIngressDescriptor {
            name: PRODUCTION_COMMAND_INGRESS.to_owned(),
            description: "Apply one admitted production lifecycle operation".to_owned(),
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
        if PRODUCTION_COMPLETION_INGRESS_PERMIT
            .set(completion_permit.clone())
            .is_err()
            && PRODUCTION_COMPLETION_INGRESS_PERMIT.get() != Some(&completion_permit)
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                "production completion ingress permit changed across registrations",
            ));
        }
        let continuation_permit = registrar.register_internal_ingress(PluginIngressDescriptor {
            name: PRODUCTION_RESOURCE_CONTINUATION_INGRESS.to_owned(),
            description: "Persist one provider-authenticated resource archive continuation witness"
                .to_owned(),
            class: IngressClass::Acknowledgement,
            payload_schema: PayloadSchema::Any,
        })?;
        if PRODUCTION_RESOURCE_CONTINUATION_PERMIT
            .set(continuation_permit.clone())
            .is_err()
            && PRODUCTION_RESOURCE_CONTINUATION_PERMIT.get() != Some(&continuation_permit)
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                "production resource continuation ingress permit changed across registrations",
            ));
        }
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
        if PRODUCTION_ARCHIVE_COMMIT_PERMIT
            .set(archive_commit_permit.clone())
            .is_err()
            && PRODUCTION_ARCHIVE_COMMIT_PERMIT.get() != Some(&archive_commit_permit)
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                "production archive commit permit changed across registrations",
            ));
        }
        let archive_ack_permit = registrar.register_internal_ingress(PluginIngressDescriptor {
            name: PRODUCTION_ARCHIVE_RETENTION_ACK_INGRESS.to_owned(),
            description: "Acknowledge terminal provider-side production archive retention"
                .to_owned(),
            class: IngressClass::Acknowledgement,
            payload_schema: PayloadSchema::Any,
        })?;
        if PRODUCTION_ARCHIVE_ACK_PERMIT
            .set(archive_ack_permit.clone())
            .is_err()
            && PRODUCTION_ARCHIVE_ACK_PERMIT.get() != Some(&archive_ack_permit)
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                "production archive retention acknowledgement permit changed across registrations",
            ));
        }
        registrar.register_ingress(PluginIngressDescriptor {
            name: PRODUCTION_OBSERVATION_WAKE_INGRESS.to_owned(),
            description:
                "Deliver only production observation heads whose persisted delay has elapsed"
                    .to_owned(),
            class: IngressClass::ScheduledSystem,
            payload_schema: PayloadSchema::Any,
        })?;

        let mut apply = BoundarySystemContract::new(
            APPLY_SYSTEM,
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        apply.reads = vec![
            StateKey::core_commands(),
            StateKey::core_ingress(),
            StateKey::new(PLUGIN_NAMESPACE, "runtime"),
        ];
        apply.reads.extend(production_external_state_keys());
        apply.reads.sort();
        apply.reads.dedup();
        apply.writes = vec![StateKey::new(PLUGIN_NAMESPACE, "runtime")];
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
            "canwu.production.archive_maintenance.v1".to_owned(),
        ];
        apply.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(apply, apply_production_ingress)?;

        let mut validate = BoundarySystemContract::new(
            VALIDATE_SYSTEM,
            BoundaryPhase::InvariantValidation,
            SystemCadence::EventDriven,
        );
        validate.reads = vec![StateKey::new(PLUGIN_NAMESPACE, "runtime")];
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
            StateKey::new(PLUGIN_NAMESPACE, "runtime"),
        ];
        incidents.writes = vec![StateKey::new(PLUGIN_NAMESPACE, "runtime")];
        incidents.emits = vec!["canwu.production.incident_committed.v1".to_owned()];
        incidents.random_streams = vec![production_incident_random_stream()];
        incidents.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(incidents, evaluate_incident_candidates)?;

        let mut incident_commit_audit = BoundarySystemContract::new(
            INCIDENT_COMMIT_AUDIT_SYSTEM,
            BoundaryPhase::ConditionalTransitionCommit,
            SystemCadence::EventDriven,
        );
        incident_commit_audit.reads = vec![StateKey::new(PLUGIN_NAMESPACE, "runtime")];
        incident_commit_audit.visibility = StateVisibility::SameBoundary;
        registrar
            .register_boundary_system(incident_commit_audit, audit_incident_transition_commit)?;

        let mut output_dispatch = BoundarySystemContract::new(
            OUTPUT_DISPATCH_SYSTEM,
            BoundaryPhase::StrategicAggregation,
            SystemCadence::EventDriven,
        );
        output_dispatch.reads = vec![StateKey::new(PLUGIN_NAMESPACE, "runtime")];
        output_dispatch.writes = vec![StateKey::new(PLUGIN_NAMESPACE, "runtime")];
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
            StateKey::new(PLUGIN_NAMESPACE, "runtime"),
        ];
        reports.writes = vec![StateKey::new(PLUGIN_NAMESPACE, "runtime")];
        reports.plugin_ingress_targets = vec![PluginIngressTarget {
            target_plugin: PLUGIN_NAME.to_owned(),
            packet_type: PRODUCTION_OBSERVATION_WAKE_INGRESS.to_owned(),
        }];
        reports.knowledge_writes = vec![KnowledgeWriteGrant {
            schema: report_schema.id,
            visibilities: vec![StateVisibility::SameBoundary],
        }];
        reports.emits = vec!["canwu.production.report_capacity_rejected.v1".to_owned()];
        reports.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(reports, publish_holder_reports)
    }
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
    require_holder_authority(context, &envelope.holder)?;
    let (_, state) = load_state(view)?.ok_or_else(|| {
        CanwuError::new(
            ErrorCode::DomainRecordNotFound,
            "production runtime is not configured",
        )
    })?;
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
    state.ensure_operation_outcome_admission_capacity(&envelope.operation)?;
    if state.project_operation_uses_reserved_outcome_at_capacity(&envelope.operation)? {
        validate_external_operation_evidence(view, &envelope.holder, &envelope.operation)
            .map_err(structured_reserved_project_rejection)?;
        let mut candidate = state.clone();
        candidate
            .apply_operation(&envelope, context.simulation_time)
            .map_err(structured_reserved_project_rejection)?;
        candidate
            .validate()
            .map_err(structured_reserved_project_rejection)?;
    }
    let decision_receipt = validate_degraded_decision(view, context, &state, &envelope)?;
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

#[allow(clippy::too_many_lines)]
fn apply_production_ingress(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let Some((record, mut state)) = load_state(view)? else {
        return Ok(BoundaryProposal::default());
    };
    let mut changed = false;
    let mut directives = Vec::new();
    for ingress_id in &archive_commits_first(view, &context.admitted_ingress)? {
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
            PRODUCTION_COMMAND_INGRESS => {
                let admitted: AdmittedProductionOperation =
                    match decode(payload, "admitted production operation") {
                        Ok(admitted) => admitted,
                        Err(error) => {
                            refuse_packet(
                                &mut directives,
                                INGRESS_REFUSED_EVENT,
                                packet_type,
                                &error,
                            );
                            continue;
                        }
                    };
                // The packet type is public, so only a packet that the
                // admission of a tracked production command queued settles:
                // that admission checked the issuer's authority, the run
                // policy, the outcome capacity, and any degraded-facility
                // decision. Any other packet settles nothing.
                if validate_admitted_command(view, ingress.cause.as_ref(), &admitted).is_err() {
                    directives.push(BoundaryDirective::Emit {
                        event_type: INGRESS_REFUSED_EVENT.to_owned(),
                        summary: format!(
                            "production packet {packet_type} was refused: it does not match an admitted tracked production command"
                        ),
                        affected: Vec::new(),
                    });
                    continue;
                }
                if state
                    .operation_outcomes
                    .contains_key(&admitted.envelope.operation_id)
                {
                    continue;
                }
                let mut candidate = state.clone();
                let result = validate_external_operation_evidence(
                    view,
                    &admitted.envelope.holder,
                    &admitted.envelope.operation,
                )
                .and_then(|()| candidate.apply_operation(&admitted.envelope, context.at))
                .and_then(|()| {
                    if let Some(receipt) = &admitted.decision_receipt
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
                    candidate.validate()
                });
                let (disposition, rejection_code, rejection_message) = match result {
                    Ok(()) => {
                        state = candidate;
                        (ProductionOperationDisposition::Applied, None, None)
                    }
                    Err(error) if expected_domain_rejection(&error) => {
                        state.revision = state.revision.checked_add(1).ok_or_else(|| {
                            invalid("production runtime revision overflowed while rejecting an operation")
                        })?;
                        (
                            ProductionOperationDisposition::Rejected,
                            Some(error_code(&error.code).to_owned()),
                            Some(error.message),
                        )
                    }
                    Err(error) => return Err(error),
                };
                let outcome = ProductionOperationOutcome {
                    id: admitted.envelope.operation_id.clone(),
                    canonical_input_hash: admitted.input_hash,
                    command: admitted.envelope.clone(),
                    disposition,
                    work_order: operation_work_order(&admitted.envelope.operation),
                    execution: operation_execution(&admitted.envelope.operation),
                    project: operation_project(&admitted.envelope.operation),
                    rejection_code,
                    rejection_message,
                    settled_at: context.at,
                };
                state.operation_outcomes.insert(outcome.id.clone(), outcome);
                if disposition == ProductionOperationDisposition::Applied
                    && let crate::ProductionOperation::AcceptFacilityCommissioning { project } =
                        &admitted.envelope.operation
                {
                    let project = state
                        .facility_projects
                        .get(project)
                        .ok_or_else(|| invalid("accepted facility project disappeared"))?;
                    directives.push(BoundaryDirective::SchedulePluginIngress {
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
                state.validate()?;
                changed = true;
                directives.push(BoundaryDirective::Emit {
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
            PRODUCTION_OUTPUT_ACK_INGRESS => {
                let acknowledgement: ProductionOutputAcknowledgement =
                    match decode(payload, "production output acknowledgement") {
                        Ok(acknowledgement) => acknowledgement,
                        Err(error) => {
                            refuse_packet(
                                &mut directives,
                                OUTPUT_ACK_REFUSED_EVENT,
                                packet_type,
                                &error,
                            );
                            continue;
                        }
                    };
                let mut candidate = state.clone();
                // An acknowledgement that does not settle its execution
                // exactly is refused with a trace and changes nothing; it
                // never fails the boundary.
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
                        .ok_or_else(|| {
                            invalid("output acknowledgement execution is unavailable")
                        })?;
                    candidate
                        .acknowledge_output(&acknowledgement, context.at)
                        .map(|changed| (changed, completion))
                });
                let (settled, completion) = match accepted {
                    Ok(accepted) => accepted,
                    Err(error) if expected_domain_rejection(&error) => {
                        directives.push(BoundaryDirective::Emit {
                            event_type: OUTPUT_ACK_REFUSED_EVENT.to_owned(),
                            summary: format!(
                                "production output acknowledgement for execution {} was refused: {}",
                                acknowledgement.execution, error.message
                            ),
                            affected: Vec::new(),
                        });
                        continue;
                    }
                    Err(error) => return Err(error),
                };
                if settled {
                    state = candidate;
                    changed = true;
                    directives.push(BoundaryDirective::SchedulePluginIngress {
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
                    directives.push(BoundaryDirective::Emit {
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
                let operation: crate::ProductionCompletionIngressV1 =
                    match decode(payload, "production completion operation") {
                        Ok(operation) => operation,
                        Err(error) => {
                            refuse_packet(
                                &mut directives,
                                INGRESS_REFUSED_EVENT,
                                packet_type,
                                &error,
                            );
                            continue;
                        }
                    };
                // A step that production's checks refuse settles nothing and
                // is recorded; the boundary settles.
                match settle_completion_step(view, context, &state, &operation) {
                    Ok(candidate) => {
                        state = candidate;
                        changed = true;
                    }
                    Err(error) if expected_domain_rejection(&error) => {
                        refuse_packet(&mut directives, INGRESS_REFUSED_EVENT, packet_type, &error);
                    }
                    Err(error) => return Err(error),
                }
            }
            PRODUCTION_RESOURCE_CONTINUATION_INGRESS => {
                let witness: crate::ProductionResourceContinuationWitnessV1 =
                    match decode(payload, "production resource continuation witness") {
                        Ok(witness) => witness,
                        Err(error) => {
                            refuse_packet(
                                &mut directives,
                                INGRESS_REFUSED_EVENT,
                                packet_type,
                                &error,
                            );
                            continue;
                        }
                    };
                // A witness that is stale or forged when it settles is refused
                // and recorded; the boundary settles.
                match settle_continuation_witness(view, &state, witness) {
                    Ok(candidate) => {
                        state = candidate;
                        changed = true;
                    }
                    Err(error) if expected_domain_rejection(&error) => {
                        refuse_packet(&mut directives, INGRESS_REFUSED_EVENT, packet_type, &error);
                    }
                    Err(error) => return Err(error),
                }
            }
            PRODUCTION_ARCHIVE_COMMIT_INGRESS => {
                let commit: crate::VerifiedProductionArchiveCommitV1 = match payload
                    .get("commit")
                    .cloned()
                    .ok_or_else(|| invalid("production archive commit payload is absent"))
                    .and_then(|value| decode(&value, "production archive commit"))
                {
                    Ok(commit) => commit,
                    Err(error) => {
                        refuse_packet(&mut directives, INGRESS_REFUSED_EVENT, packet_type, &error);
                        continue;
                    }
                };
                let receipt = state.apply_production_archive_commit(&commit, context.at)?;
                changed = true;
                directives.push(BoundaryDirective::Emit {
                    event_type: "canwu.production.archive_maintenance.v1".to_owned(),
                    summary: format!(
                        "production archive maintenance {} reached {:?}",
                        receipt.id, receipt.disposition
                    ),
                    affected: Vec::new(),
                });
            }
            PRODUCTION_ARCHIVE_RETENTION_ACK_INGRESS => {
                let acknowledgement: ProductionArchiveRetentionAcknowledgementV1 =
                    match decode(payload, "production archive retention acknowledgement") {
                        Ok(acknowledgement) => acknowledgement,
                        Err(error) => {
                            refuse_packet(
                                &mut directives,
                                INGRESS_REFUSED_EVENT,
                                packet_type,
                                &error,
                            );
                            continue;
                        }
                    };
                state.acknowledge_production_archive_retention(&acknowledgement.handle_id)?;
                changed = true;
            }
            _ => {}
        }
    }
    if changed {
        let draft = state.draft()?;
        directives.insert(
            0,
            BoundaryDirective::MutateRecord {
                mutation: DomainRecordMutation::Update {
                    record: draft,
                    expected_version: record.version,
                },
                summary: "Apply bounded production lifecycle mutations".to_owned(),
            },
        );
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
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
    validate_production_completion_ingress(view, context, operation)?;
    let mut candidate = state.clone();
    if let crate::ProductionCompletionIngressV1::AcknowledgeParticipantCompleted {
        acquisition,
        participant: _,
    } = operation
    {
        let resource_record = view
            .typed_domain_record(&canwu_resource::resource_runtime_reference())?
            .ok_or_else(|| invalid("resource completion runtime is unavailable"))?;
        let resource = resource_record.decode_payload::<canwu_resource::ResourceRuntimeRecord>()?;
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

/// Whether an operation packet is the one the admission of a tracked
/// production command queued: its cause is that command, and the command's
/// payload is the packet's envelope. The check reads the command's retained
/// record, which a seal keeps while the packet is queued.
fn validate_admitted_command(
    view: &SimulationView<'_>,
    cause: Option<&CauseRef>,
    admitted: &AdmittedProductionOperation,
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
    let matches = *command == admitted.command
        && matches!(
            &record.envelope.command,
            Command::Plugin { plugin, command, payload }
                if plugin == PLUGIN_NAME
                    && command == PRODUCTION_COMMAND
                    && decode::<ProductionCommandEnvelope>(payload, "production operation")
                        .is_ok_and(|envelope| envelope == admitted.envelope)
        )
        && canonical_hash("canwu.production.operation-input.v1", &admitted.envelope)?
            == admitted.input_hash;
    if !matches {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "production operation packet does not match its tracked command",
        ));
    }
    Ok(())
}

/// Orders the ingress one boundary admitted so that production archive
/// commits settle before every other production packet, each group in its
/// admitted order.
///
/// Every other production packet advances the runtime revision, which the
/// archive source root binds. A commit prepared from the root the boundary
/// started from therefore applies even when an operation or an
/// acknowledgement was admitted ahead of it, instead of going stale behind
/// it. A boundary that admits no archive commit keeps its order.
fn archive_commits_first(
    view: &SimulationView<'_>,
    admitted: &[canwu_api::IngressId],
) -> Result<Vec<canwu_api::IngressId>, CanwuError> {
    let mut commits = Vec::new();
    let mut others = Vec::new();
    for ingress_id in admitted {
        let is_commit = matches!(
            view.ingress(*ingress_id)?.map(|ingress| &ingress.payload),
            Some(IngressPayload::Plugin {
                plugin,
                packet_type,
                ..
            }) if plugin == PLUGIN_NAME && packet_type == PRODUCTION_ARCHIVE_COMMIT_INGRESS
        );
        if is_commit {
            commits.push(*ingress_id);
        } else {
            others.push(*ingress_id);
        }
    }
    commits.extend(others);
    Ok(commits)
}

fn validate_production_candidate(
    view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    if let Some((_, state)) = load_state(view)? {
        state.validate()?;
    }
    Ok(BoundaryProposal::default())
}

fn dispatch_pending_outputs(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let Some((record, mut state)) = load_state(view)? else {
        return Ok(BoundaryProposal::default());
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
        return Ok(BoundaryProposal::default());
    }
    let source = view
        .current_domain_record_version(&production_runtime_reference().into_untyped())?
        .ok_or_else(|| invalid("production output source version is unavailable"))?;
    let mut directives = Vec::with_capacity(pending.len().saturating_add(1));
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
    state.validate()?;
    directives.insert(
        0,
        BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Update {
                record: state.draft()?,
                expected_version: record.version,
            },
            summary: "Pin exact production sources for pending output settlement".to_owned(),
        },
    );
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

fn evaluate_incident_candidates(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let Some((record, mut state)) = load_state(view)? else {
        return Ok(BoundaryProposal::default());
    };
    let operation_evidence = context.admitted_ingress.iter().find_map(|ingress_id| {
        let ingress = view.ingress(*ingress_id).ok().flatten()?;
        matches!(
            &ingress.payload,
            IngressPayload::Plugin { plugin, .. } if plugin == PLUGIN_NAME
        )
        .then_some(EvidenceRef::Ingress(*ingress_id))
    });
    let Some(operation_evidence) = operation_evidence else {
        return Ok(BoundaryProposal::default());
    };
    let source_record_digest = canonical_hash("canwu.production.runtime-cut.v1", &record)?;
    let candidates = state
        .incident_due_index
        .iter()
        .take(crate::ProductionLimitsV1::canonical().max_incidents_per_boundary)
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
        let operation_key = format!(
            "canwu.production:incident:{}:{}:{}:{}",
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
        state.apply_incident_transition(transition)?;
        committed = committed.saturating_add(1);
    }
    if evaluated.is_none() {
        return Ok(BoundaryProposal::default());
    }
    state.advance_incident_cursor(evaluated)?;
    let draft = state.draft()?;
    let mut directives = vec![BoundaryDirective::MutateRecord {
        mutation: DomainRecordMutation::Update {
            record: draft,
            expected_version: record.version,
        },
        summary: "Persist the bounded production incident fairness cursor and any operation-keyed transition bundles for kernel commit".to_owned(),
    }];
    if committed > 0 {
        directives.push(BoundaryDirective::Emit {
                event_type: "canwu.production.incident_committed.v1".to_owned(),
                summary: format!("staged {committed} bounded production incident transition(s) for atomic conditional commit"),
                affected: Vec::new(),
            });
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

fn audit_incident_transition_commit(
    view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    if let Some((_, state)) = load_state(view)? {
        state.validate()?;
    }
    Ok(BoundaryProposal::default())
}

fn publish_holder_reports(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let triggers = context
        .admitted_ingress
        .iter()
        .filter_map(|ingress_id| {
            let ingress = view.ingress(*ingress_id).ok().flatten()?;
            matches!(
                &ingress.payload,
                IngressPayload::Plugin { plugin, .. } if plugin == PLUGIN_NAME
            )
            .then_some(EvidenceRef::Ingress(*ingress_id))
        })
        .collect::<Vec<_>>();
    if triggers.is_empty() {
        return Ok(BoundaryProposal::default());
    }
    let Some((record, mut state)) = load_state(view)? else {
        return Ok(BoundaryProposal::default());
    };
    let provider_source = view
        .current_domain_record_version(&production_runtime_reference().into_untyped())?
        .ok_or_else(|| invalid("production observation provider source is unavailable"))?;
    let limits = crate::ProductionLimitsV1::canonical();
    let mut namespace_count = view.knowledge_record_count_in_namespace(PLUGIN_NAMESPACE)?;
    let mut holder_counts = std::collections::BTreeMap::new();
    let mut reserved_total = 0_usize;
    let mut reserved_by_holder = std::collections::BTreeMap::new();
    let mut directives = Vec::new();
    let dirty = state
        .observation_dirty_index
        .iter()
        .take(limits.max_reports_per_boundary)
        .cloned()
        .collect::<Vec<_>>();
    let mut state_changed = false;
    for key in dirty {
        let holder_count = if let Some(count) = holder_counts.get(&key.holder) {
            *count
        } else {
            let count = production_holder_knowledge_count(view, &key.holder)?;
            holder_counts.insert(key.holder.clone(), count);
            count
        };
        let holder_reserved = reserved_by_holder
            .get(&key.holder)
            .copied()
            .unwrap_or_default();
        if namespace_count.saturating_add(reserved_total) < limits.max_observation_records
            && holder_count.saturating_add(holder_reserved)
                < limits.max_observation_records_per_holder
        {
            state.materialize_observation_head(
                &key,
                context.at,
                EvidenceRef::DomainRecordVersion(provider_source.clone()),
            )?;
            reserved_total = reserved_total.saturating_add(1);
            reserved_by_holder.insert(key.holder.clone(), holder_reserved.saturating_add(1));
            state_changed = true;
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
        let Some(site) = state.sites.get(&key.scope) else {
            continue;
        };
        let head =
            crate::query::eligible_observation_head(&state, &key.holder, &key.scope, context.at)?;
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
        let holder_count = if let Some(count) = holder_counts.get(&holder) {
            *count
        } else {
            let count = production_holder_knowledge_count(view, &holder)?;
            holder_counts.insert(holder.clone(), count);
            count
        };
        if namespace_count < limits.max_observation_records
            && holder_count < limits.max_observation_records_per_holder
        {
            directives.push(BoundaryDirective::PublishKnowledge {
                holder: holder.clone(),
                visibility: StateVisibility::SameBoundary,
                producer_correlation: Some(report.id.to_string()),
                records: vec![KnowledgeRecordDraft {
                    schema: production_report_knowledge_schema_id(),
                    subjects: vec![KnowledgeSubject {
                        role: "site".to_owned(),
                        target: KnowledgeSubjectTarget::Entity(place),
                    }],
                    payload: serde_json::to_value(&report).map_err(|error| encode_error(&error))?,
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
                summary: "Publish one bounded holder-relative production report".to_owned(),
            });
            namespace_count = namespace_count.saturating_add(1);
            holder_counts.insert(holder, holder_count.saturating_add(1));
        } else {
            state
                .observation_due_index
                .entry(context.at)
                .or_default()
                .insert(key);
            state_changed = true;
            directives.push(BoundaryDirective::Emit {
                event_type: "canwu.production.report_capacity_rejected.v1".to_owned(),
                summary: "Reject periodic production report because its bounded knowledge capacity is exhausted".to_owned(),
                affected: Vec::new(),
            });
        }
    }
    if let Some(next_due) = state.observation_due_index.keys().next().copied()
        && next_due > context.at
    {
        directives.push(BoundaryDirective::SchedulePluginIngress {
            target_plugin: PLUGIN_NAME.to_owned(),
            after: next_due
                .checked_sub(context.at)
                .ok_or_else(|| invalid("production observation wake delay overflowed"))?,
            packet_type: PRODUCTION_OBSERVATION_WAKE_INGRESS.to_owned(),
            priority: 0,
            payload: serde_json::json!({ "due_at": next_due }),
            affected: Vec::new(),
        });
    }
    if state_changed {
        state.revision = state
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("production runtime revision overflowed"))?;
        state.validate()?;
        directives.insert(
            0,
            BoundaryDirective::MutateRecord {
                mutation: DomainRecordMutation::Update {
                    record: state.draft()?,
                    expected_version: record.version,
                },
                summary: "Persist bounded production observation cuts and delivery indexes"
                    .to_owned(),
            },
        );
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

fn production_holder_knowledge_count(
    view: &SimulationView<'_>,
    holder: &KnowledgeHolderRef,
) -> Result<usize, CanwuError> {
    let maximum = crate::ProductionLimitsV1::canonical().max_observation_records_per_holder;
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

/// Marks every production archive object a restore, a replay, a fork, or a
/// queued commit can still read: the commits reachable from the archive
/// head, from each unacknowledged retention handle, and from each commit the
/// kernel lists as queued, with their blobs, pages, and lookup pages. The
/// archive is audited first, as at restore.
fn production_archive_reachability(
    view: &SimulationView<'_>,
    provider: &dyn PluginArchiveObjectProvider,
    manifest: &mut ArchiveReachabilityManifest,
) -> Result<(), CanwuError> {
    let provider = PluginProductionArchiveProvider(provider);
    // Queued commits: the kernel marks the directory each queued archive
    // commit retains. Each may become the head, so its index must be whole.
    let queued = manifest
        .plugin_objects
        .get(crate::PRODUCTION_ARCHIVE_INDEX_DIRECTORY_NAMESPACE)
        .cloned()
        .unwrap_or_default();
    let mut roots = queued.clone();
    let mut complete = queued;
    let mut bound = roots.len();
    if let Some((_, state)) = load_state(view)? {
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

/// Stores, reads back, verifies, and queues a production cold-archive commit
/// through a plugin-owned ingress capability.
///
/// The batch must equal what
/// [`crate::ProductionState::prepare_production_archive`] derives from the
/// current production root and `store` for the same number of records,
/// including its lookup-index pages. The boundary that settles the commit
/// reads no archive object: it applies the commit when the root still is the
/// one the batch was prepared from, and otherwise records it as stale.
pub fn enqueue_production_archive(
    canwu: &mut Canwu,
    prepared: &crate::PreparedProductionArchiveBatchV1,
    store: &dyn crate::ProductionArchiveStore,
) -> Result<ProductionArchiveIngressReceiptV1, CanwuError> {
    let state = canwu
        .typed_domain_record(&production_runtime_reference())
        .ok_or_else(|| CanwuError::new(ErrorCode::PluginNotActive, "production runtime is absent"))?
        .decode_payload::<ProductionRuntimeRecord>()?;
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

/// Queues one canonical production-owned completion coordinator transition.
/// Participant state remains authoritative in the participant plugin and is
/// accepted here only through an exact current provider record acknowledgement.
pub fn enqueue_production_completion_operation(
    canwu: &mut Canwu,
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
            serde_json::to_value(operation).map_err(|error| encode_error(&error))?,
        ),
        permit,
    )
}

/// Authenticates every compacted resource input for one active facility
/// project against the resource archive provider, then queues a package-owned
/// witness used by later canonical project ingress.
pub fn enqueue_production_resource_continuation(
    canwu: &mut Canwu,
    project_id: &crate::FacilityProjectId,
    store: &dyn canwu_resource::ResourceArchiveStore,
) -> Result<canwu_api::IngressReceipt, CanwuError> {
    let production_record = canwu
        .typed_domain_record(&production_runtime_reference())
        .ok_or_else(|| invalid("production runtime is unavailable"))?;
    let production = production_record.decode_payload::<ProductionRuntimeRecord>()?;
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
    let resource_record = canwu
        .typed_domain_record(&canwu_resource::resource_runtime_reference())
        .ok_or_else(|| invalid("resource runtime is unavailable"))?;
    let resource = resource_record.decode_payload::<canwu_resource::ResourceRuntimeRecord>()?;
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
            serde_json::to_value(witness).map_err(|error| encode_error(&error))?,
        ),
        permit,
    )
}

/// Finalizes a terminal production archive retention handle and queues the
/// package-owned acknowledgement that clears it, and every stale receipt
/// recorded under it, from authoritative hot state.
///
/// The handle must still be in the production root when this is called. An
/// acknowledgement that settles after another one retired the handle is a
/// no-op, so calling this twice before the next boundary is harmless.
pub fn finalize_production_archive_retention(
    canwu: &mut Canwu,
    store: &dyn crate::ProductionArchiveStore,
    receipt: &ProductionArchiveIngressReceiptV1,
) -> Result<canwu_api::IngressReceipt, CanwuError> {
    let record = canwu
        .typed_domain_record(&production_runtime_reference())
        .ok_or_else(|| invalid("production runtime is unavailable"))?;
    let state = record.decode_payload::<ProductionRuntimeRecord>()?;
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
            })
            .map_err(|error| encode_error(&error))?,
        ),
        permit,
    )
}

pub(crate) fn load_state(
    view: &SimulationView<'_>,
) -> Result<Option<(DomainRecord, ProductionState)>, CanwuError> {
    let Some(record) = view.typed_domain_record(&production_runtime_reference())? else {
        return Ok(None);
    };
    let state = record.decode_payload::<ProductionRuntimeRecord>()?;
    state.validate()?;
    Ok(Some((record.clone(), state)))
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
    let Some(record) = view.typed_domain_record(&canwu_resource::resource_runtime_reference())?
    else {
        return Ok(());
    };
    let resource = record.decode_payload::<canwu_resource::ResourceRuntimeRecord>()?;
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

fn validate_external_operation_evidence(
    view: &SimulationView<'_>,
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
        let (_, production) = load_state(view)?
            .ok_or_else(|| invalid("production provider validation runtime is unavailable"))?;
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
    let (_, production) = load_state(view)?
        .ok_or_else(|| invalid("production provider validation runtime is unavailable"))?;
    if let crate::ProductionOperation::CreateFacilityProject { project } = operation {
        validate_external_project_evidence(view, &production, project)?;
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
        validate_project_resource_evidence(view, &production, project)?;
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

    let resource_record = view
        .typed_domain_record(&canwu_resource::resource_runtime_reference())?
        .ok_or_else(|| invalid("production input resource runtime is unavailable"))?;
    let resource_state =
        resource_record.decode_payload::<canwu_resource::ResourceRuntimeRecord>()?;
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
    let record = view
        .typed_domain_record(&canwu_resource::resource_runtime_reference())?
        .ok_or_else(|| invalid("production input resource runtime is unavailable"))?;
    let state = record.decode_payload::<canwu_resource::ResourceRuntimeRecord>()?;
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
    let resource_record = view
        .typed_domain_record(&canwu_resource::resource_runtime_reference())?
        .ok_or_else(|| invalid("production continuation resource runtime is unavailable"))?;
    let resource = resource_record.decode_payload::<canwu_resource::ResourceRuntimeRecord>()?;
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
    let record = view
        .typed_domain_record(&canwu_resource::resource_runtime_reference())?
        .ok_or_else(|| invalid("production project resource runtime is unavailable"))?;
    let state = record.decode_payload::<canwu_resource::ResourceRuntimeRecord>()?;
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
    let record = view
        .typed_domain_record(&canwu_resource::resource_runtime_reference())?
        .ok_or_else(|| invalid("production input resource runtime is unavailable"))?;
    let state = record.decode_payload::<canwu_resource::ResourceRuntimeRecord>()?;
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

fn holder_entity_ref(holder: &KnowledgeHolderRef) -> canwu_api::EntityRef {
    match holder {
        KnowledgeHolderRef::Person(person) => canwu_api::EntityRef::Person(*person),
        KnowledgeHolderRef::Entity(entity) => entity.clone(),
    }
}

fn validate_production_completion_ingress(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
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
            validate_production_locked_targets(view, &request.target_versions)?;
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
            let (_, state) = load_state(view)?.ok_or_else(|| {
                invalid("production completion coordinator runtime is unavailable")
            })?;
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
            let resource = record.decode_payload::<canwu_resource::ResourceRuntimeRecord>()?;
            let authoritative = resource
                .external_completion_participants
                .grants
                .get(acquisition)
                .ok_or_else(|| invalid("resource participant grant is unavailable"))?;
            if authoritative.grant != *grant || authoritative.coordinator_plugin != PLUGIN_NAME {
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
            let resource_record = view
                .typed_domain_record(&canwu_resource::resource_runtime_reference())?
                .ok_or_else(|| invalid("resource completion runtime is unavailable"))?;
            let resource =
                resource_record.decode_payload::<canwu_resource::ResourceRuntimeRecord>()?;
            let authoritative = resource
                .external_completion_participants
                .participant(acquisition)
                .ok_or_else(|| invalid("completed resource participant is unavailable"))?;
            if authoritative.coordinator_plugin != PLUGIN_NAME
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
    targets: &[canwu_resource::CompletionLockedTargetV1],
) -> Result<(), CanwuError> {
    if targets.is_empty() {
        return Err(invalid("production completion local target set is empty"));
    }
    for target in targets {
        let canwu_resource::CompletionLockedTargetV1::ExternalRecord { version } = target else {
            return Err(invalid(
                "production completion local capacity may lock only exact production records",
            ));
        };
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
    let resource_record = view
        .typed_domain_record(&canwu_resource::resource_runtime_reference())?
        .ok_or_else(|| invalid("production output resource runtime is unavailable"))?;
    let resource_state =
        resource_record.decode_payload::<canwu_resource::ResourceRuntimeRecord>()?;
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

fn structured_reserved_project_rejection(error: CanwuError) -> CanwuError {
    match error.code {
        ErrorCode::InvalidDomainRecord
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
