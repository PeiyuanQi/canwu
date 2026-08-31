use crate::{
    PLUGIN_NAME, PLUGIN_NAMESPACE, ResourceOperationRequestV1, ResourceRuntimeRecord,
    holder_entity, materialize_resource_report, resource_canwu_error, resource_runtime_reference,
};
use canwu_api::{
    ArchiveReachabilityManifest, BoundaryContext, BoundaryDirective, BoundaryPhase,
    BoundaryProposal, BoundarySystemContract, Canwu, CanwuError, CauseRef, Command, CommandContext,
    CommandIngress, DomainRecord, DomainRecordKind, DomainRecordMutation, DomainRecordSchema,
    DomainRecordType, ErrorCode, EventAudience, EvidenceRef, IngressClass, IngressPayload, Issuer,
    KnowledgeOrigin, KnowledgeRecordDraft, KnowledgeRecordKind, KnowledgeSchemaId,
    KnowledgeSubject, KnowledgeSubjectSchema, KnowledgeSubjectTarget, KnowledgeSubjectTargetKind,
    KnowledgeWriteGrant, PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD, PayloadSchema,
    PluginActionDescriptor, PluginArchiveObjectProvider, PluginArchiveRetention,
    PluginIngressDescriptor, PluginIngressPermit, PluginIngressRequest, PluginIngressTarget,
    PluginKnowledgeSchema, PluginRegistrar, SimTime, SimulationPlugin, SimulationView, StateKey,
    StateVisibility, SystemCadence, SystemDirective,
    payload_required_evidence_continuation_property_v1,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::OnceLock;

pub const RESOURCE_COMMAND: &str = "apply_resource_operation_v1";
pub const RESOURCE_COMMAND_INGRESS: &str = "resource_command_v1";
pub const RESOURCE_ADAPTER_INGRESS: &str = "resource_adapter_operation_v1";
pub const RESOURCE_PRODUCTION_OUTPUT_BATCH_INGRESS: &str = "resource_production_output_batch_v1";
pub const RESOURCE_ALLOCATION_INGRESS: &str = "resource_authorized_allocation_v1";
pub const RESOURCE_COMPLETION_INGRESS: &str = "resource_completion_operation_v1";
pub const RESOURCE_ARCHIVE_COMMIT_INGRESS: &str = "resource_archive_commit_v1";
pub const RESOURCE_ARCHIVE_RETENTION_ACK_INGRESS: &str = "resource_archive_retention_ack_v1";
pub const RESOURCE_COMPLETION_EXPIRY_TICK_INGRESS: &str = "resource_completion_expiry_tick_v1";
pub const RESOURCE_REPORT_WAKE_INGRESS: &str = "resource_report_wake_v1";
pub const RESOURCE_REPORT_KNOWLEDGE: &str = "resource_report";
pub const RESOURCE_SEMANTIC_HASH: &str =
    "62931530fdc87cb8c56ab78f617e3d2d20468b3fdcfbbfc2c74abe37c385fc44";

const RESOURCE_REPORT_SCHEMA_HASH: &str =
    "c4aed3aebb1f4cb54f889c644647d15671be3c5338731330d6fc693c3933493b";
const RESOURCE_REPORT_HOT_CAPACITY: usize = 8_192;
static RESOURCE_COMPLETION_INGRESS_PERMIT: OnceLock<PluginIngressPermit> = OnceLock::new();
static RESOURCE_ALLOCATION_INGRESS_PERMIT: OnceLock<PluginIngressPermit> = OnceLock::new();
static RESOURCE_ARCHIVE_INGRESS_PERMIT: OnceLock<PluginIngressPermit> = OnceLock::new();
static RESOURCE_ARCHIVE_RETENTION_ACK_PERMIT: OnceLock<PluginIngressPermit> = OnceLock::new();

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceArchiveIngressReceiptV1 {
    pub ingress: canwu_api::IngressReceipt,
    pub retention_handle_id: String,
    pub directory_root: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct ResourceArchiveRetentionAcknowledgementV1 {
    receipt: crate::ResourceArchiveMaintenanceReceiptV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceCommandV1 {
    pub subject: canwu_api::KnowledgeHolderRef,
    pub request: ResourceOperationRequestV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceAdapterOperationV1 {
    pub provider_plugin: String,
    pub provider_source: canwu_api::DomainRecordVersionRef,
    pub request: ResourceOperationRequestV1,
}

/// One production-owned, all-or-nothing output settlement batch. Every credit
/// is validated against the same exact pending production execution body and
/// is applied on a detached resource state before any result is published.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceProductionOutputBatchV1 {
    pub provider_plugin: String,
    pub provider_source: canwu_api::DomainRecordVersionRef,
    pub requests: Vec<crate::ResourceCreditRequestV1>,
}

/// Plugin-owned allocation packet. The requester is an authoritative target,
/// not presentation metadata: allocation only considers demands owned by this
/// exact holder, and the request must name the current resource-state revision.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceAuthorizedAllocationV1 {
    pub requester: canwu_api::KnowledgeHolderRef,
    pub request: crate::ResourceAllocationRequestV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct AdmittedResourceCommandV1 {
    command: canwu_api::CommandId,
    value: ResourceCommandV1,
}

#[derive(Clone, Debug, Default)]
pub struct ResourcePlugin {
    adapter_evidence_kinds: Vec<DomainRecordKind>,
}

impl ResourcePlugin {
    #[must_use]
    pub fn new(adapter_evidence_kinds: impl IntoIterator<Item = DomainRecordKind>) -> Self {
        let mut adapter_evidence_kinds: Vec<_> = adapter_evidence_kinds.into_iter().collect();
        adapter_evidence_kinds.sort();
        adapter_evidence_kinds.dedup();
        Self {
            adapter_evidence_kinds,
        }
    }

    fn adapter_state_keys(&self) -> Vec<StateKey> {
        self.adapter_evidence_kinds
            .iter()
            .map(|kind| StateKey::new(kind.namespace.clone(), kind.name.clone()))
            .collect()
    }
}

struct PluginResourceArchiveProvider<'a>(&'a dyn PluginArchiveObjectProvider);

impl crate::ResourceArchiveStore for PluginResourceArchiveProvider<'_> {
    fn store_resource_archive_object(
        &self,
        _namespace: &str,
        _object_id: &str,
        _bytes: &[u8],
    ) -> Result<(), crate::ResourceError> {
        Err(crate::ResourceError::InvalidLifecycle(
            "archive reachability provider is read-only".to_owned(),
        ))
    }

    fn load_resource_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, crate::ResourceError> {
        self.0
            .load_plugin_archive_object(namespace, object_id)
            .map_err(|error| crate::ResourceError::InvalidDefinition(error.to_string()))
    }

    fn persist_resource_archive_retention(
        &self,
        _handle: &crate::ResourceArchiveRetentionHandleV1,
    ) -> Result<(), crate::ResourceError> {
        Err(crate::ResourceError::InvalidLifecycle(
            "archive reachability provider cannot persist retention".to_owned(),
        ))
    }

    fn finalize_resource_archive_retention(
        &self,
        _handle_id: &str,
        _phase: crate::ResourceArchiveRetentionPhaseV1,
    ) -> Result<(), crate::ResourceError> {
        Err(crate::ResourceError::InvalidLifecycle(
            "archive reachability provider cannot finalize retention".to_owned(),
        ))
    }
}

struct ViewResourceArchiveProvider<'a>(&'a SimulationView<'a>);

impl crate::ResourceArchiveStore for ViewResourceArchiveProvider<'_> {
    fn store_resource_archive_object(
        &self,
        _namespace: &str,
        _object_id: &str,
        _bytes: &[u8],
    ) -> Result<(), crate::ResourceError> {
        Err(crate::ResourceError::InvalidLifecycle(
            "resource archive view is read-only".to_owned(),
        ))
    }

    fn load_resource_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, crate::ResourceError> {
        self.0
            .plugin_archive_object(namespace, object_id)
            .map_err(|error| crate::ResourceError::InvalidDefinition(error.to_string()))
    }

    fn persist_resource_archive_retention(
        &self,
        _handle: &crate::ResourceArchiveRetentionHandleV1,
    ) -> Result<(), crate::ResourceError> {
        Err(crate::ResourceError::InvalidLifecycle(
            "resource archive view cannot persist retention".to_owned(),
        ))
    }

    fn finalize_resource_archive_retention(
        &self,
        _handle_id: &str,
        _phase: crate::ResourceArchiveRetentionPhaseV1,
    ) -> Result<(), crate::ResourceError> {
        Err(crate::ResourceError::InvalidLifecycle(
            "resource archive view cannot finalize retention".to_owned(),
        ))
    }
}

fn archived_operation_outcome(
    view: &SimulationView<'_>,
    state: &crate::ResourceState,
    request: &ResourceOperationRequestV1,
    request_digest: &str,
) -> Result<Option<crate::ResourceOperationOutcome>, CanwuError> {
    if state.archive_head.directory_root.is_none() {
        return Ok(None);
    }
    let provider = ViewResourceArchiveProvider(view);
    let Some(outcome) =
        crate::archived_resource_operation_outcome(state, &provider, &request.operation_key())
            .map_err(resource_canwu_error)?
    else {
        return Ok(None);
    };
    if outcome.request_digest != request_digest {
        return Err(CanwuError::new(
            ErrorCode::IdempotencyConflict,
            "archived resource operation key was reused with a different request",
        ));
    }
    Ok(Some(outcome))
}

fn resource_archive_reachability(
    view: &SimulationView<'_>,
    provider: &dyn PluginArchiveObjectProvider,
    manifest: &mut ArchiveReachabilityManifest,
) -> Result<(), CanwuError> {
    let provider = PluginResourceArchiveProvider(provider);
    let mut roots = manifest
        .plugin_objects
        .get(crate::RESOURCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE)
        .cloned()
        .unwrap_or_default();
    if let Some(record) = view.typed_domain_record(&resource_runtime_reference())? {
        let state = record.decode_payload::<ResourceRuntimeRecord>()?;
        if let Some(root) = &state.archive_head.directory_root {
            roots.insert(root.clone());
        }
        roots.extend(
            state
                .archive_retention_handles
                .values()
                .map(|handle| handle.directory_root.clone()),
        );
    }
    let mut visited = std::collections::BTreeSet::new();
    while let Some(root) = roots.pop_first() {
        if !visited.insert(root.clone()) {
            continue;
        }
        let bytes = crate::ResourceArchiveStore::load_resource_archive_object(
            &provider,
            crate::RESOURCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE,
            &root,
        )
        .map_err(resource_canwu_error)?
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidArchive,
                "resource archive directory is unavailable",
            )
        })?;
        let directory: crate::ResourceArchiveIndexDirectoryV1 = serde_json::from_slice(&bytes)
            .map_err(|error| {
                CanwuError::new(
                    ErrorCode::InvalidArchive,
                    format!("resource archive directory cannot be decoded: {error}"),
                )
            })?;
        crate::authenticate_resource_archive_directory(&provider, &directory)
            .map_err(resource_canwu_error)?;
        manifest.insert_plugin_object(crate::RESOURCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE, root);
        for page in &directory.membership_pages {
            manifest.insert_plugin_object(
                crate::RESOURCE_ARCHIVE_MEMBERSHIP_PAGE_NAMESPACE,
                page.clone(),
            );
        }
        for page in &directory.temporal_pages {
            manifest.insert_plugin_object(
                crate::RESOURCE_ARCHIVE_TEMPORAL_PAGE_NAMESPACE,
                page.clone(),
            );
        }
        for blob in &directory.blob_ids {
            manifest.insert_plugin_object(crate::RESOURCE_ARCHIVE_BLOB_NAMESPACE, blob.clone());
        }
        if let Some(previous) = directory.previous_root {
            roots.insert(previous);
        }
    }
    Ok(())
}

impl SimulationPlugin for ResourcePlugin {
    fn name(&self) -> &'static str {
        PLUGIN_NAME
    }

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    fn semantic_hash(&self) -> &'static str {
        RESOURCE_SEMANTIC_HASH
    }

    fn validate_activation(&self, records: &[DomainRecord]) -> Result<(), CanwuError> {
        validate_resource_activation_records(records)
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_record_schema(resource_record_schema())?;
        registrar.register_knowledge_schema(resource_report_knowledge_schema())?;
        registrar.register_event_audience(
            "canwu.resource.operation_settled.v1",
            EventAudience::AffectedActors,
        )?;
        registrar.register_archive_reachability_participant(resource_archive_reachability)?;

        let mut command_reads = vec![StateKey::core_evidence(), resource_state_key()];
        command_reads.extend(self.adapter_state_keys());
        command_reads.sort();
        command_reads.dedup();
        registrar.register_command(
            PluginActionDescriptor {
                name: RESOURCE_COMMAND.to_owned(),
                description: "Admit one authority-bound resource operation".to_owned(),
                payload_schema: PayloadSchema::Any,
                reads: command_reads,
                writes: Vec::new(),
            },
            admit_resource_command,
        )?;
        registrar.register_ingress(PluginIngressDescriptor {
            name: RESOURCE_COMMAND_INGRESS.to_owned(),
            description: "Settle one tracked resource command".to_owned(),
            class: IngressClass::Decision,
            payload_schema: PayloadSchema::Any,
        })?;
        registrar.register_ingress(PluginIngressDescriptor {
            name: RESOURCE_PRODUCTION_OUTPUT_BATCH_INGRESS.to_owned(),
            description: "Atomically settle every output of one exact production execution"
                .to_owned(),
            class: IngressClass::Acknowledgement,
            payload_schema: PayloadSchema::Any,
        })?;
        let allocation_permit = registrar.register_internal_ingress(PluginIngressDescriptor {
            name: RESOURCE_ALLOCATION_INGRESS.to_owned(),
            description: "Allocate bounded due/dirty resource demand for one exact requester"
                .to_owned(),
            class: IngressClass::ScheduledSystem,
            payload_schema: PayloadSchema::Any,
        })?;
        if RESOURCE_ALLOCATION_INGRESS_PERMIT
            .set(allocation_permit.clone())
            .is_err()
            && RESOURCE_ALLOCATION_INGRESS_PERMIT.get() != Some(&allocation_permit)
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                "resource allocation ingress permit changed across registrations",
            ));
        }
        let completion_permit = registrar.register_internal_ingress(PluginIngressDescriptor {
            name: RESOURCE_COMPLETION_INGRESS.to_owned(),
            description: "Apply one canonical persisted resource completion-lease transition"
                .to_owned(),
            class: IngressClass::ScheduledSystem,
            payload_schema: PayloadSchema::Any,
        })?;
        if RESOURCE_COMPLETION_INGRESS_PERMIT
            .set(completion_permit.clone())
            .is_err()
            && RESOURCE_COMPLETION_INGRESS_PERMIT.get() != Some(&completion_permit)
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                "resource completion ingress permit changed across registrations",
            ));
        }
        let archive_permit = registrar.register_internal_ingress_with_archive_retention(
            PluginIngressDescriptor {
                name: RESOURCE_ARCHIVE_COMMIT_INGRESS.to_owned(),
                description: "Commit one provider-verified resource terminal archive batch"
                    .to_owned(),
                class: IngressClass::ScheduledSystem,
                payload_schema: PayloadSchema::Any,
            },
            crate::RESOURCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE,
            vec![
                "/directory_root".to_owned(),
                "/retention/directory_root".to_owned(),
            ],
        )?;
        if RESOURCE_ARCHIVE_INGRESS_PERMIT
            .set(archive_permit.clone())
            .is_err()
            && RESOURCE_ARCHIVE_INGRESS_PERMIT.get() != Some(&archive_permit)
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                "resource archive ingress permit changed across registrations",
            ));
        }
        let archive_ack_permit = registrar.register_internal_ingress(PluginIngressDescriptor {
            name: RESOURCE_ARCHIVE_RETENTION_ACK_INGRESS.to_owned(),
            description: "Acknowledge resource archive store-side retention finalization"
                .to_owned(),
            class: IngressClass::Acknowledgement,
            payload_schema: PayloadSchema::Any,
        })?;
        if RESOURCE_ARCHIVE_RETENTION_ACK_PERMIT
            .set(archive_ack_permit.clone())
            .is_err()
            && RESOURCE_ARCHIVE_RETENTION_ACK_PERMIT.get() != Some(&archive_ack_permit)
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                "resource archive acknowledgement permit changed across registrations",
            ));
        }
        registrar.register_ingress(PluginIngressDescriptor {
            name: RESOURCE_ADAPTER_INGRESS.to_owned(),
            description: "Accept one exact adapter-bound resource operation".to_owned(),
            class: IngressClass::Acknowledgement,
            payload_schema: PayloadSchema::Any,
        })?;
        registrar.register_ingress(PluginIngressDescriptor {
            name: RESOURCE_COMPLETION_EXPIRY_TICK_INGRESS.to_owned(),
            description: "Advance bounded completion-lease expiry without coordinator liveness"
                .to_owned(),
            class: IngressClass::ScheduledSystem,
            payload_schema: PayloadSchema::Any,
        })?;
        registrar.register_ingress(PluginIngressDescriptor {
            name: RESOURCE_REPORT_WAKE_INGRESS.to_owned(),
            description: "Wake delayed and periodic reports at an otherwise quiet boundary"
                .to_owned(),
            class: IngressClass::ScheduledSystem,
            payload_schema: PayloadSchema::Any,
        })?;

        let mut lifecycle = BoundarySystemContract::new(
            "settle-resource-lifecycle-v1",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        lifecycle.reads = vec![
            StateKey::core_commands(),
            StateKey::core_evidence(),
            StateKey::core_ingress(),
            StateKey::core_knowledge(),
            resource_state_key(),
            // External completion participant requests coordinated by the
            // production package bind an exact current production runtime.
            StateKey::new("canwu.production", "runtime"),
        ];
        lifecycle.reads.extend(self.adapter_state_keys());
        lifecycle.reads.sort();
        lifecycle.reads.dedup();
        lifecycle.writes = vec![resource_state_key()];
        lifecycle.plugin_ingress_targets = vec![
            PluginIngressTarget {
                target_plugin: "canwu-production".to_owned(),
                packet_type: "production_output_ack_v1".to_owned(),
            },
            PluginIngressTarget {
                target_plugin: "canwu-production".to_owned(),
                packet_type: "production_completion_operation_v1".to_owned(),
            },
            PluginIngressTarget {
                target_plugin: PLUGIN_NAME.to_owned(),
                packet_type: RESOURCE_COMPLETION_EXPIRY_TICK_INGRESS.to_owned(),
            },
        ];
        lifecycle.emits = vec!["canwu.resource.operation_settled.v1".to_owned()];
        lifecycle.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(lifecycle, settle_resource_lifecycle)?;

        let mut invariant = BoundarySystemContract::new(
            "validate-resource-invariants-v1",
            BoundaryPhase::InvariantValidation,
            SystemCadence::EventDriven,
        );
        invariant.reads = vec![resource_state_key()];
        invariant.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(invariant, validate_resource_invariants)?;

        let mut aggregation = BoundarySystemContract::new(
            "maintain-resource-summary-v1",
            BoundaryPhase::StrategicAggregation,
            SystemCadence::EventDriven,
        );
        aggregation.reads = vec![resource_state_key()];
        aggregation.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(aggregation, validate_resource_invariants)?;

        let mut reports = BoundarySystemContract::new(
            "materialize-resource-reports-v1",
            BoundaryPhase::PerspectiveAndReportMaterialization,
            SystemCadence::EventDriven,
        );
        reports.reads = vec![StateKey::core_knowledge(), resource_state_key()];
        reports.writes = vec![resource_state_key()];
        reports.knowledge_writes = vec![KnowledgeWriteGrant {
            schema: resource_report_knowledge_schema_id(),
            visibilities: vec![StateVisibility::SameBoundary],
        }];
        reports.plugin_ingress_targets = vec![PluginIngressTarget {
            target_plugin: PLUGIN_NAME.to_owned(),
            packet_type: RESOURCE_REPORT_WAKE_INGRESS.to_owned(),
        }];
        reports.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(reports, materialize_resource_reports)
    }
}

fn resource_record_schema() -> DomainRecordSchema {
    let mut schema = DomainRecordSchema::for_record::<ResourceRuntimeRecord>();
    schema.payload_schema = PayloadSchema::Object {
        properties: std::collections::BTreeMap::from([(
            PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD.to_owned(),
            payload_required_evidence_continuation_property_v1(),
        )]),
        allow_additional: true,
    };
    schema
}

fn admit_resource_command(
    view: &SimulationView<'_>,
    context: &CommandContext,
    payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    if context.ingress == CommandIngress::LegacyDirect {
        return Err(CanwuError::new(
            ErrorCode::MixedCommandIngress,
            "resource operations require tracked command ingress",
        ));
    }
    let value: ResourceCommandV1 = decode(payload, "resource command")?;
    validate_resource_subject_issuer(context, &value.subject)?;
    let record = view
        .typed_domain_record(&resource_runtime_reference())?
        .ok_or_else(|| invalid("resource runtime is unavailable"))?;
    let state = record.decode_payload::<ResourceRuntimeRecord>()?;
    let request_digest =
        crate::canonical_digest("canwu.resource.operation-request.v1", &value.request)
            .map_err(resource_canwu_error)?;
    if archived_operation_outcome(view, &state, &value.request, &request_digest)?.is_some() {
        return Ok(Vec::new());
    }
    validate_resource_authority(view, context, &value)?;
    Ok(vec![SystemDirective::EnqueuePluginIngress {
        after: canwu_api::SimDuration::ZERO,
        packet_type: RESOURCE_COMMAND_INGRESS.to_owned(),
        priority: 0,
        payload: encode(&AdmittedResourceCommandV1 {
            command: context.command_id,
            value: value.clone(),
        })?,
        affected: vec![holder_entity(&value.subject)],
    }])
}

fn settle_resource_lifecycle(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let Some(record) = view
        .typed_domain_record(&resource_runtime_reference())?
        .cloned()
    else {
        return Ok(BoundaryProposal::default());
    };
    let mut state = record.decode_payload::<ResourceRuntimeRecord>()?;
    let mut changed = false;
    let mut affected = Vec::new();
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
            RESOURCE_COMMAND_INGRESS => {
                let admitted: AdmittedResourceCommandV1 =
                    decode(payload, "admitted resource command")?;
                validate_admitted_command(view, ingress.cause.as_ref(), &admitted)?;
                let request_digest = crate::canonical_digest(
                    "canwu.resource.operation-request.v1",
                    &admitted.value.request,
                )
                .map_err(resource_canwu_error)?;
                if archived_operation_outcome(
                    view,
                    &state,
                    &admitted.value.request,
                    &request_digest,
                )?
                .is_some()
                {
                    continue;
                }
                state
                    .apply_operation(&admitted.value.request)
                    .map_err(resource_canwu_error)?;
                affected.push(holder_entity(&admitted.value.subject));
                changed = true;
            }
            RESOURCE_ADAPTER_INGRESS => {
                let packet: ResourceAdapterOperationV1 =
                    decode(payload, "resource adapter operation")?;
                let request_digest =
                    crate::canonical_digest("canwu.resource.operation-request.v1", &packet.request)
                        .map_err(resource_canwu_error)?;
                if archived_operation_outcome(view, &state, &packet.request, &request_digest)?
                    .is_some()
                {
                    continue;
                }
                let production_execution =
                    validate_adapter_packet(view, context.at, &packet, &state)?;
                state
                    .apply_operation(&packet.request)
                    .map_err(resource_canwu_error)?;
                if let Some(execution) = production_execution {
                    let outcome = state
                        .outcomes
                        .get(&packet.request.operation_key())
                        .ok_or_else(|| {
                            CanwuError::new(
                                ErrorCode::InvalidDomainRecord,
                                "resource production credit did not persist its exact outcome",
                            )
                        })?;
                    directives.push(BoundaryDirective::SchedulePluginIngress {
                        target_plugin: "canwu-production".to_owned(),
                        after: canwu_api::SimDuration::ZERO,
                        packet_type: "production_output_ack_v1".to_owned(),
                        priority: 0,
                        payload: serde_json::json!({
                            "execution": execution,
                            "production_source": packet.provider_source,
                            "outcome": outcome,
                        }),
                        affected: Vec::new(),
                    });
                }
                changed = true;
            }
            RESOURCE_PRODUCTION_OUTPUT_BATCH_INGRESS => {
                let packet: ResourceProductionOutputBatchV1 =
                    decode(payload, "production output batch")?;
                let execution =
                    validate_production_output_batch(view, context.at, &packet, &state)?;
                let mut candidate = state.clone();
                let outcomes = candidate
                    .apply_production_output_batch(&packet.requests)
                    .map_err(resource_canwu_error)?;
                state = candidate;
                directives.push(BoundaryDirective::SchedulePluginIngress {
                    target_plugin: "canwu-production".to_owned(),
                    after: canwu_api::SimDuration::ZERO,
                    packet_type: "production_output_ack_v1".to_owned(),
                    priority: 0,
                    payload: serde_json::json!({
                        "execution": execution,
                        "production_source": packet.provider_source,
                        "outcomes": outcomes,
                    }),
                    affected: Vec::new(),
                });
                changed = true;
            }
            RESOURCE_ALLOCATION_INGRESS => {
                let packet: ResourceAuthorizedAllocationV1 =
                    decode(payload, "authorized resource allocation")?;
                let operation = ResourceOperationRequestV1::Allocate(packet.request.clone());
                let request_digest = crate::canonical_digest(
                    "canwu.resource.authorized-allocation-request.v1",
                    &(&packet.requester, &packet.request),
                )
                .map_err(resource_canwu_error)?;
                if archived_operation_outcome(view, &state, &operation, &request_digest)?.is_some()
                {
                    continue;
                }
                state
                    .apply_authorized_allocation(&packet.requester, &packet.request)
                    .map_err(resource_canwu_error)?;
                affected.push(holder_entity(&packet.requester));
                changed = true;
            }
            RESOURCE_COMPLETION_INGRESS => {
                let operation: crate::ResourceCompletionOperationV1 =
                    decode(payload, "resource completion operation")?;
                let request = ResourceOperationRequestV1::Completion(operation.clone());
                let request_digest =
                    crate::canonical_digest("canwu.resource.operation-request.v1", &request)
                        .map_err(resource_canwu_error)?;
                if archived_operation_outcome(view, &state, &request, &request_digest)?.is_some() {
                    continue;
                }
                validate_completion_ingress(view, context.at, &operation, &state)?;
                let completed_external = match &operation {
                    crate::ResourceCompletionOperationV1::CompleteExternalParticipant(request) => {
                        Some(request.acquisition.clone())
                    }
                    _ => None,
                };
                let preparing_local = match &operation {
                    crate::ResourceCompletionOperationV1::Prepare(request) => {
                        Some(request.acquisition.clone())
                    }
                    _ => None,
                };
                let starts_expiry_clock = matches!(
                    &operation,
                    crate::ResourceCompletionOperationV1::Grant(_)
                        | crate::ResourceCompletionOperationV1::GrantExternalParticipant(_)
                );
                match &operation {
                    crate::ResourceCompletionOperationV1::Prepare(request) => {
                        let external_targets_current = state
                            .completion_leases
                            .grants
                            .get(&request.grant)
                            .ok_or_else(|| {
                                CanwuError::new(
                                    ErrorCode::InvalidAuthority,
                                    "completion prepare grant is unavailable",
                                )
                            })?
                            .target_versions
                            .iter()
                            .filter_map(|target| match target {
                                crate::CompletionLockedTargetV1::ExternalRecord { version } => {
                                    Some(version)
                                }
                                _ => None,
                            })
                            .try_fold(true, |all_current, version| {
                                Ok::<_, CanwuError>(
                                    all_current
                                        && view.domain_record_version_is_current(version)?,
                                )
                            })?;
                        state
                            .apply_prepare_with_external_revalidation(
                                request,
                                external_targets_current,
                            )
                            .map_err(resource_canwu_error)?;
                    }
                    _ => {
                        state
                            .apply_operation(&ResourceOperationRequestV1::Completion(operation))
                            .map_err(resource_canwu_error)?;
                    }
                }
                if preparing_local.as_ref().is_some_and(|acquisition| {
                    state
                        .completion_leases
                        .acquisitions
                        .get(acquisition)
                        .is_some_and(|value| {
                            value.state == crate::CompletionLeaseAcquisitionStateV1::Aborting
                        })
                }) {
                    directives.push(completion_expiry_tick_directive());
                }
                if let Some(acquisition) = completed_external {
                    let participant = state
                        .external_completion_participants
                        .participant(&acquisition)
                        .ok_or_else(|| {
                            CanwuError::new(
                                ErrorCode::InvalidDomainRecord,
                                "completed external participant disappeared",
                            )
                        })?;
                    if participant.coordinator_plugin == "canwu-production" {
                        directives.push(BoundaryDirective::SchedulePluginIngress {
                            target_plugin: participant.coordinator_plugin.clone(),
                            after: canwu_api::SimDuration::ZERO,
                            packet_type: "production_completion_operation_v1".to_owned(),
                            priority: 0,
                            payload: serde_json::json!({
                                "completion": "acknowledge_participant_completed",
                                "request": {
                                    "acquisition": acquisition,
                                    "participant": PLUGIN_NAME,
                                },
                            }),
                            affected: Vec::new(),
                        });
                    }
                }
                if starts_expiry_clock
                    && (!state.completion_leases.expiry_due.is_empty()
                        || !state.external_completion_participants.expiry_due.is_empty())
                {
                    directives.push(completion_expiry_tick_directive());
                }
                changed = true;
            }
            RESOURCE_COMPLETION_EXPIRY_TICK_INGRESS => {
                let boundary = context.boundary_id.get();
                if state
                    .completion_leases
                    .acquisitions
                    .values()
                    .any(|value| value.state == crate::CompletionLeaseAcquisitionStateV1::Aborting)
                {
                    let released = state
                        .completion_leases
                        .cleanup_aborting(1_024)
                        .map_err(resource_canwu_error)?;
                    for acquisition in released {
                        state.completion_report_reservations.remove(&acquisition);
                        state.completion_report_ready.remove(&acquisition);
                    }
                    changed = true;
                }
                if state
                    .completion_leases
                    .expiry_due
                    .range(..=boundary)
                    .any(|(_, values)| !values.is_empty())
                {
                    state
                        .apply_operation(&ResourceOperationRequestV1::Completion(
                            crate::ResourceCompletionOperationV1::Expire(
                                crate::ExpireCompletionCapacityV1 {
                                    at: context.at,
                                    current_boundary: boundary,
                                    candidate_limit: 1_024,
                                },
                            ),
                        ))
                        .map_err(resource_canwu_error)?;
                    changed = true;
                }
                if state
                    .external_completion_participants
                    .expiry_due
                    .range(..=boundary)
                    .any(|(_, values)| !values.is_empty())
                {
                    state
                        .apply_operation(&ResourceOperationRequestV1::Completion(
                            crate::ResourceCompletionOperationV1::ExpireExternalParticipants(
                                crate::ExpireExternalCompletionParticipantGrantsV1 {
                                    at: context.at,
                                    current_boundary: boundary,
                                    candidate_limit: 1_024,
                                },
                            ),
                        ))
                        .map_err(resource_canwu_error)?;
                    changed = true;
                }
                if !state.completion_leases.expiry_due.is_empty()
                    || !state.external_completion_participants.expiry_due.is_empty()
                    || state.completion_leases.acquisitions.values().any(|value| {
                        value.state == crate::CompletionLeaseAcquisitionStateV1::Aborting
                    })
                {
                    directives.push(completion_expiry_tick_directive());
                }
            }
            RESOURCE_ARCHIVE_COMMIT_INGRESS => {
                let commit: crate::VerifiedResourceArchiveCommitV1 =
                    decode(payload, "verified resource archive commit")?;
                state
                    .apply_archive_commit(&commit)
                    .map_err(resource_canwu_error)?;
                changed = true;
            }
            RESOURCE_ARCHIVE_RETENTION_ACK_INGRESS => {
                let acknowledgement: ResourceArchiveRetentionAcknowledgementV1 =
                    decode(payload, "resource archive retention acknowledgement")?;
                let persisted = state
                    .archive_maintenance_receipts
                    .get(&acknowledgement.receipt.sequence)
                    .ok_or_else(|| {
                        CanwuError::new(
                            ErrorCode::InvalidArchive,
                            "resource archive terminal receipt is unavailable",
                        )
                    })?;
                if persisted != &acknowledgement.receipt {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidArchive,
                        "resource archive acknowledgement receipt is forged",
                    ));
                }
                state
                    .archive_retention_handles
                    .remove(&acknowledgement.receipt.retention_handle_id);
                changed = true;
            }
            _ => {}
        }
    }
    if !changed {
        return Ok(BoundaryProposal {
            directives,
            ..BoundaryProposal::default()
        });
    }
    state.state_revision =
        crate::ResourceRevision::new(record.version.checked_add(1).ok_or_else(|| {
            CanwuError::new(ErrorCode::ValueOutOfRange, "resource version overflow")
        })?)
        .map_err(resource_canwu_error)?;
    state.validate().map_err(resource_canwu_error)?;
    directives.insert(
        0,
        BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Update {
                record: state.record_draft()?,
                expected_version: record.version,
            },
            summary: "Settle resource lifecycle ingress".to_owned(),
        },
    );
    directives.push(BoundaryDirective::Emit {
        event_type: "canwu.resource.operation_settled.v1".to_owned(),
        summary: "Settled one or more resource operations".to_owned(),
        affected,
    });
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

fn completion_expiry_tick_directive() -> BoundaryDirective {
    BoundaryDirective::SchedulePluginIngress {
        target_plugin: PLUGIN_NAME.to_owned(),
        after: canwu_api::SimDuration::ZERO,
        packet_type: RESOURCE_COMPLETION_EXPIRY_TICK_INGRESS.to_owned(),
        priority: 0,
        payload: serde_json::json!({ "format_version": 1 }),
        affected: Vec::new(),
    }
}

fn validate_resource_invariants(
    view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let Some(record) = view.proposed_typed_domain_record(&resource_runtime_reference())? else {
        return Ok(BoundaryProposal::default());
    };
    let state = record.decode_payload::<ResourceRuntimeRecord>()?;
    state.validate().map_err(resource_canwu_error)?;
    Ok(BoundaryProposal::default())
}

fn materialize_resource_reports(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let Some(record) = view.typed_domain_record(&resource_runtime_reference())? else {
        return Ok(BoundaryProposal::default());
    };
    let mut state = record.decode_payload::<ResourceRuntimeRecord>()?;
    let mut changed = false;
    let due_times = state
        .report_due_index
        .range(..=context.at.as_minutes())
        .map(|(at, _)| *at)
        .collect::<Vec<_>>();
    let mut due_ready = std::collections::BTreeSet::new();
    for at in due_times {
        if let Some(grants) = state.report_due_index.remove(&at) {
            due_ready.extend(grants.iter().cloned());
            state.report_dirty_grants.extend(grants);
            changed = true;
        }
    }
    let existing = view.knowledge_record_count_in_namespace(PLUGIN_NAMESPACE)?;
    let ordinary_publication_capacity = RESOURCE_REPORT_HOT_CAPACITY
        .saturating_sub(existing)
        .saturating_sub(
            state
                .reserved_knowledge_report_slots()
                .map_err(resource_canwu_error)?,
        )
        .min(state.limits.max_reports_per_boundary);
    if !changed && state.report_dirty_grants.is_empty() {
        return Ok(BoundaryProposal::default());
    }
    let reference = resource_runtime_reference().into_untyped();
    let version = if let Some(proposed) = view.proposed_domain_record_version(&reference)? {
        proposed
    } else {
        view.current_domain_record_version(&reference)?
            .ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::InvalidDomainRecord,
                    "resource report source version is unavailable",
                )
            })?
    };
    let scheduled = state
        .report_due_index
        .values()
        .flatten()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let mut candidates = state
        .report_dirty_grants
        .iter()
        .filter(|grant| !scheduled.contains(*grant))
        .cloned()
        .collect::<Vec<_>>();
    candidates.sort();
    if let Some(cursor) = &state.report_cursor {
        candidates.sort_by_key(|grant| (grant <= cursor, grant.clone()));
    }
    let mut grants_by_holder = std::collections::BTreeMap::new();
    let mut published = 0_usize;
    let mut directives = Vec::new();
    for grant_id in candidates {
        let mandatory_binding =
            state
                .completion_report_reservations
                .iter()
                .find_map(|(acquisition, grants)| {
                    grants.contains(&grant_id).then(|| {
                        state
                            .completion_report_ready
                            .get(acquisition)
                            .copied()
                            .map(|required| (acquisition.clone(), required))
                    })?
                });
        let mandatory = mandatory_binding.as_ref().is_some_and(|(_, required)| {
            state
                .observation_head_by_grant
                .get(&grant_id)
                .and_then(|head| state.observation_heads.get(head))
                .is_some_and(|head| head.provider_state_revision >= *required)
        });
        if !report_publication_slot_available(
            existing,
            published,
            ordinary_publication_capacity,
            state.limits.max_reports_per_boundary,
            mandatory,
        ) {
            continue;
        }
        let grant = state
            .report_grants
            .get(&grant_id)
            .cloned()
            .ok_or_else(|| invalid("resource dirty report index lost its grant"))?;
        if !state.observation_head_by_grant.contains_key(&grant_id) {
            if mandatory_binding.is_none() {
                state.report_dirty_grants.remove(&grant_id);
            } else {
                directives.push(report_wake_directive(grant.cadence_minutes));
            }
            state.report_cursor = Some(grant_id);
            changed = true;
            continue;
        }
        if grant.delay_minutes > 0 && !due_ready.contains(&grant_id) {
            let delay = i64::try_from(grant.delay_minutes)
                .map_err(|_| invalid("resource report delay exceeds simulation time"))?;
            let due = context
                .at
                .checked_add(canwu_api::SimDuration::minutes(delay))
                .ok_or_else(|| invalid("resource report delay overflowed"))?;
            state
                .report_due_index
                .entry(due.as_minutes())
                .or_default()
                .insert(grant_id.clone());
            state.report_dirty_grants.remove(&grant_id);
            state.report_cursor = Some(grant_id);
            directives.push(report_wake_directive(grant.delay_minutes));
            changed = true;
            continue;
        }
        grants_by_holder
            .entry(grant.holder)
            .or_insert_with(Vec::new)
            .push((
                grant_id.clone(),
                mandatory.then(|| mandatory_binding.expect("mandatory binding exists").0),
            ));
        state.report_dirty_grants.remove(&grant_id);
        let cadence = i64::try_from(grant.cadence_minutes)
            .map_err(|_| invalid("resource report cadence exceeds simulation time"))?;
        let next_due = context
            .at
            .checked_add(canwu_api::SimDuration::minutes(cadence))
            .ok_or_else(|| invalid("resource report cadence overflowed"))?;
        state
            .report_due_index
            .entry(next_due.as_minutes())
            .or_default()
            .insert(grant_id.clone());
        state.report_cursor = Some(grant_id);
        directives.push(report_wake_directive(grant.cadence_minutes));
        published += 1;
        changed = true;
    }
    for (batch_ordinal, (holder, grant_ids)) in grants_by_holder.into_iter().enumerate() {
        let mut records = Vec::with_capacity(grant_ids.len());
        for (grant_id, mandatory_acquisition) in grant_ids {
            let head_id = state
                .observation_head_by_grant
                .get(&grant_id)
                .ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::InvalidDomainRecord,
                        "resource observation head index is unavailable during materialization",
                    )
                })?;
            let head = state.observation_heads.get(head_id).ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::InvalidDomainRecord,
                    "resource observation head is unavailable during materialization",
                )
            })?;
            let report =
                materialize_resource_report(&state, &holder, &grant_id, context.at, context.at)
                    .map_err(resource_canwu_error)?;
            let mut origin_evidence =
                std::iter::once(EvidenceRef::DomainRecordVersion(version.clone()))
                    .chain(
                        head.source_versions
                            .iter()
                            .cloned()
                            .map(EvidenceRef::DomainRecordVersion),
                    )
                    .collect::<Vec<_>>();
            origin_evidence.sort();
            origin_evidence.dedup();
            records.push(KnowledgeRecordDraft {
                schema: resource_report_knowledge_schema_id(),
                subjects: vec![KnowledgeSubject {
                    role: "resource_state".to_owned(),
                    target: KnowledgeSubjectTarget::DomainRecord(
                        resource_runtime_reference().into_untyped(),
                    ),
                }],
                payload: encode(&report)?,
                as_of: Some(report.observed_at),
                confidence_per_mille: report.confidence_per_mille,
                origin: KnowledgeOrigin {
                    method: "resource_holder_report_v1".to_owned(),
                    evidence: origin_evidence,
                },
                supersedes: Vec::new(),
                contradicts: Vec::new(),
            });
            if let Some(acquisition) = mandatory_acquisition {
                if let Some(grants) = state.completion_report_reservations.get_mut(&acquisition) {
                    grants.remove(&grant_id);
                }
                if state
                    .completion_report_reservations
                    .get(&acquisition)
                    .is_some_and(std::collections::BTreeSet::is_empty)
                {
                    state.completion_report_reservations.remove(&acquisition);
                    state.completion_report_ready.remove(&acquisition);
                    state
                        .mark_external_participant_archive_ready(&acquisition)
                        .map_err(resource_canwu_error)?;
                }
            }
        }
        directives.push(BoundaryDirective::PublishKnowledge {
            holder,
            visibility: StateVisibility::SameBoundary,
            producer_correlation: Some(format!(
                "resource-report-batch::{}::{batch_ordinal}",
                context.boundary_id
            )),
            records,
            summary: "Publish holder-relative resource reports".to_owned(),
        });
    }
    if changed {
        state.state_revision = crate::ResourceRevision::new(
            record
                .version
                .checked_add(1)
                .ok_or_else(|| invalid("resource report state revision overflowed"))?,
        )
        .map_err(resource_canwu_error)?;
        state.validate().map_err(resource_canwu_error)?;
        directives.insert(
            0,
            BoundaryDirective::MutateRecord {
                mutation: DomainRecordMutation::Update {
                    record: state.record_draft()?,
                    expected_version: record.version,
                },
                summary: "Persist resource report due/dirty fairness state".to_owned(),
            },
        );
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

fn report_publication_slot_available(
    existing: usize,
    published: usize,
    ordinary_capacity: usize,
    boundary_capacity: usize,
    mandatory: bool,
) -> bool {
    published < boundary_capacity
        && (published < ordinary_capacity
            || (mandatory && existing.saturating_add(published) < RESOURCE_REPORT_HOT_CAPACITY))
}

#[cfg(test)]
mod report_capacity_tests {
    use super::{RESOURCE_REPORT_HOT_CAPACITY, report_publication_slot_available};

    #[test]
    fn named_terminal_reservation_is_the_only_usable_slot_at_full_pressure() {
        let existing = RESOURCE_REPORT_HOT_CAPACITY - 1;
        assert!(!report_publication_slot_available(existing, 0, 0, 1, false));
        assert!(report_publication_slot_available(existing, 0, 0, 1, true));
        assert!(!report_publication_slot_available(
            RESOURCE_REPORT_HOT_CAPACITY,
            0,
            0,
            1,
            true,
        ));
    }
}

fn report_wake_directive(after_minutes: u64) -> BoundaryDirective {
    BoundaryDirective::SchedulePluginIngress {
        target_plugin: PLUGIN_NAME.to_owned(),
        after: canwu_api::SimDuration::minutes(
            i64::try_from(after_minutes).expect("validated report delay fits simulation time"),
        ),
        packet_type: RESOURCE_REPORT_WAKE_INGRESS.to_owned(),
        priority: 0,
        payload: serde_json::json!({ "format_version": 1 }),
        affected: Vec::new(),
    }
}

fn validate_admitted_command(
    view: &SimulationView<'_>,
    cause: Option<&CauseRef>,
    admitted: &AdmittedResourceCommandV1,
) -> Result<(), CanwuError> {
    let Some(CauseRef::Command(command)) = cause else {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "resource command ingress lacks command evidence",
        ));
    };
    let record = view.command(*command)?.ok_or_else(|| {
        CanwuError::new(
            ErrorCode::EvidenceUnavailable,
            "resource command evidence is unavailable",
        )
    })?;
    let matches = *command == admitted.command
        && matches!(
            &record.envelope.command,
            Command::Plugin { plugin, command, payload }
                if plugin == PLUGIN_NAME
                    && command == RESOURCE_COMMAND
                    && decode::<ResourceCommandV1>(payload, "resource command")
                        .is_ok_and(|value| value == admitted.value)
        );
    if !matches {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "resource command ingress does not match its tracked command",
        ));
    }
    Ok(())
}

fn validate_adapter_packet(
    view: &SimulationView<'_>,
    at: SimTime,
    packet: &ResourceAdapterOperationV1,
    resource_state: &crate::ResourceState,
) -> Result<Option<Value>, CanwuError> {
    let source = view
        .domain_record_version(&packet.provider_source)?
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::EvidenceContentUnavailable,
                "resource adapter source exact body is unavailable",
            )
        })?;
    if source.owner != packet.provider_plugin || !adapter_source_matches(packet) {
        return Err(CanwuError::new(
            ErrorCode::InvalidDomainRecord,
            "resource adapter provider or exact source binding differs",
        ));
    }
    if request_time(&packet.request).is_some_and(|request_at| request_at != at) {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "resource irreversible adapter operation must use its canonical boundary time",
        ));
    }
    if packet.provider_plugin == "canwu-force-supply-reference" {
        require_exact_body(view, &packet.provider_source)?;
    } else {
        require_exact_current_body(view, &packet.provider_source)?;
    }
    validate_request_certificate_evidence(view, &packet.request)?;
    validate_local_provider_participant(resource_state, packet)?;
    authoritative_provider_operation(&source.payload, packet, resource_state)
}

fn validate_production_output_batch(
    view: &SimulationView<'_>,
    at: SimTime,
    packet: &ResourceProductionOutputBatchV1,
    state: &crate::ResourceState,
) -> Result<Value, CanwuError> {
    if packet.provider_plugin != "canwu-production"
        || packet.requests.is_empty()
        || packet.requests.len() > 64
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "production output batch has an invalid provider or bounded leg set",
        ));
    }
    let source = view
        .domain_record_version(&packet.provider_source)?
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::EvidenceContentUnavailable,
                "production output batch exact source body is unavailable",
            )
        })?;
    require_exact_body(view, &packet.provider_source)?;
    if source.owner != packet.provider_plugin {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "production output batch source is not owned by the production provider",
        ));
    }
    let first = &packet.requests[0];
    let certificate = &first.completion_certificate;
    let mut operation_keys = std::collections::BTreeSet::new();
    let mut accounts = std::collections::BTreeSet::new();
    if packet.requests.iter().any(|request| {
        request.at != at
            || request.completion_certificate != *certificate
            || request.source
                != crate::ResourceCreditSourceV1::Production(packet.provider_source.clone())
            || !operation_keys.insert(request.operation_key.clone())
            || !accounts.insert(request.account.clone())
    }) {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "production output batch legs do not share one exact certificate, time, and source",
        ));
    }
    validate_request_certificate_evidence(
        view,
        &ResourceOperationRequestV1::Credit(first.clone()),
    )?;
    let participant = state
        .external_completion_participants
        .grants
        .get(&certificate.acquisition)
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidAuthority,
                "production output batch resource participant is unavailable",
            )
        })?;
    if participant.coordinator_plugin != packet.provider_plugin
        || participant.coordinator_source.record != packet.provider_source.record
        || participant.coordinator_source.version > packet.provider_source.version
        || participant.certificate.as_ref() != Some(certificate)
        || participant.grant.operation_key != certificate.operation_key
        || participant.grant.state != crate::CompletionGrantStateV1::Consumed
        || packet.requests.iter().any(|request| {
            !certificate.locked_target_versions.contains(
                &crate::CompletionLockedTargetV1::Account {
                    id: request.account.clone(),
                    revision: request.expected_account_revision,
                },
            )
        })
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "production output batch does not match its consumed resource participant grant",
        ));
    }
    authoritative_production_output_batch(&source.payload, packet)
}

fn authoritative_production_output_batch(
    payload: &Value,
    packet: &ResourceProductionOutputBatchV1,
) -> Result<Value, CanwuError> {
    let executions = payload
        .get("executions")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidDomainRecord,
                "production provider payload has no authoritative execution table",
            )
        })?;
    let certificate = serde_json::to_value(&packet.requests[0].completion_certificate)
        .map_err(|error| CanwuError::new(ErrorCode::InvalidPayload, error.to_string()))?;
    let mut matched = executions.values().filter(|execution| {
        let Some(outputs) = execution.get("output_requests").and_then(Value::as_array) else {
            return false;
        };
        execution.get("lifecycle").and_then(Value::as_str)
            == Some("completed_pending_output_settlement")
            && execution
                .get("output_outcomes")
                .and_then(Value::as_array)
                .is_some_and(Vec::is_empty)
            && execution.get("completion_certificate") == Some(&certificate)
            && outputs.len() == packet.requests.len()
            && outputs
                .iter()
                .zip(&packet.requests)
                .all(|(output, request)| {
                    output.get("operation_key")
                        == serde_json::to_value(&request.operation_key).ok().as_ref()
                        && output.get("account")
                            == serde_json::to_value(&request.account).ok().as_ref()
                        && output.get("expected_account_revision")
                            == serde_json::to_value(request.expected_account_revision)
                                .ok()
                                .as_ref()
                        && output.get("resource")
                            == serde_json::to_value(&request.resource_revision)
                                .ok()
                                .as_ref()
                        && output.get("unit")
                            == serde_json::to_value(&request.unit_revision).ok().as_ref()
                        && output.get("quantity").and_then(Value::as_u64) == Some(request.quantity)
                })
    });
    let execution = matched.next().ok_or_else(|| {
        CanwuError::new(
            ErrorCode::InvalidAuthority,
            "production provider payload does not authorize the exact output batch",
        )
    })?;
    if matched.next().is_some() {
        return Err(CanwuError::new(
            ErrorCode::InvalidDomainRecord,
            "production provider payload ambiguously authorizes the output batch",
        ));
    }
    execution.get("id").cloned().ok_or_else(|| {
        CanwuError::new(
            ErrorCode::InvalidDomainRecord,
            "production output batch execution has no canonical identity",
        )
    })
}

fn validate_local_provider_participant(
    state: &crate::ResourceState,
    packet: &ResourceAdapterOperationV1,
) -> Result<(), CanwuError> {
    let certificate = match &packet.request {
        ResourceOperationRequestV1::Consume(value) => Some(&value.completion_certificate),
        ResourceOperationRequestV1::Credit(value)
            if !matches!(value.source, crate::ResourceCreditSourceV1::Production(_)) =>
        {
            Some(&value.completion_certificate)
        }
        ResourceOperationRequestV1::ExternalOutflow(value) => Some(&value.completion_certificate),
        _ => None,
    };
    let Some(certificate) = certificate else {
        return Ok(());
    };
    if let Some(participant) = state
        .external_completion_participants
        .grants
        .get(&certificate.acquisition)
    {
        if participant.coordinator_plugin != packet.provider_plugin
            || participant.coordinator_source.record != packet.provider_source.record
            || participant.coordinator_source.version > packet.provider_source.version
            || participant.certificate.as_ref() != Some(certificate)
            || participant.grant.state != crate::CompletionGrantStateV1::Consumed
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidAuthority,
                format!(
                    "external resource participant does not bind the exact provider/certificate (coord_plugin={}, packet_plugin={}, coord_source={:?}, packet_source={:?}, cert_equal={}, state={:?})",
                    participant.coordinator_plugin,
                    packet.provider_plugin,
                    participant.coordinator_source,
                    packet.provider_source,
                    participant.certificate.as_ref() == Some(certificate),
                    participant.grant.state,
                ),
            ));
        }
        if let ResourceOperationRequestV1::Consume(request) = &packet.request {
            let leg = state
                .allocation_legs
                .get(&request.allocation.id)
                .ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::InvalidAuthority,
                        "resource adapter allocation is unavailable for external participant authority",
                    )
                })?;
            let demand = state.demands.get(&leg.demand).ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "resource adapter demand is unavailable for external participant authority",
                )
            })?;
            if participant.holder != demand.requester {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "external resource participant holder differs from the exact demand requester",
                ));
            }
        }
        return Ok(());
    }
    let acquisition = state
        .completion_leases
        .acquisitions
        .get(&certificate.acquisition)
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidAuthority,
                "resource adapter completion acquisition is unavailable",
            )
        })?;
    let grant_id = acquisition
        .grants
        .get(&packet.provider_plugin)
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidAuthority,
                "resource adapter provider has no authoritative completion participant grant",
            )
        })?;
    let grant = state
        .completion_leases
        .grants
        .get(grant_id)
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidAuthority,
                "resource adapter provider participant grant is unavailable",
            )
        })?;
    if let ResourceOperationRequestV1::Consume(request) = &packet.request {
        let leg = state
            .allocation_legs
            .get(&request.allocation.id)
            .ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "resource adapter allocation is unavailable for participant authority",
                )
            })?;
        let demand = state.demands.get(&leg.demand).ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidAuthority,
                "resource adapter demand is unavailable for participant authority",
            )
        })?;
        if acquisition.holder != demand.requester {
            return Err(CanwuError::new(
                ErrorCode::InvalidAuthority,
                "resource adapter participant holder differs from the exact demand requester",
            ));
        }
    }
    if grant.acquisition != certificate.acquisition
        || grant.owner_plugin != packet.provider_plugin
        || grant.state != crate::CompletionGrantStateV1::Prepared
        || !certificate
            .prepared_grants
            .contains(&(grant.id.clone(), grant.revision))
        || !grant
            .target_versions
            .contains(&crate::CompletionLockedTargetV1::ExternalRecord {
                version: packet.provider_source.clone(),
            })
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "resource adapter provider participant grant does not bind the exact source body",
        ));
    }
    Ok(())
}

fn authoritative_provider_operation(
    payload: &Value,
    packet: &ResourceAdapterOperationV1,
    resource_state: &crate::ResourceState,
) -> Result<Option<Value>, CanwuError> {
    match &packet.request {
        ResourceOperationRequestV1::Credit(request)
            if matches!(request.source, crate::ResourceCreditSourceV1::Production(_)) =>
        {
            authoritative_production_credit(payload, packet, request)
        }
        ResourceOperationRequestV1::Consume(request)
            if packet.provider_plugin == "canwu-force-supply-reference" =>
        {
            authoritative_force_consumption(payload, request)?;
            Ok(None)
        }
        ResourceOperationRequestV1::Consume(request)
            if packet.provider_plugin == "canwu-economy-reference" =>
        {
            authoritative_economy_consumption(payload, packet, request, resource_state)?;
            Ok(None)
        }
        ResourceOperationRequestV1::Consume(_) => Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            format!(
                "resource consumption provider {} has no typed authoritative adapter",
                packet.provider_plugin
            ),
        )),
        _ => Ok(None),
    }
}

fn authoritative_production_credit(
    payload: &Value,
    packet: &ResourceAdapterOperationV1,
    request: &crate::ResourceCreditRequestV1,
) -> Result<Option<Value>, CanwuError> {
    if packet.provider_plugin != "canwu-production" {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "resource production credit requires the canonical production provider",
        ));
    }
    let executions = payload
        .get("executions")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidDomainRecord,
                "production provider payload has no authoritative execution table",
            )
        })?;
    let certificate = serde_json::to_value(&request.completion_certificate).map_err(|error| {
        CanwuError::new(
            ErrorCode::InvalidPayload,
            format!("resource production certificate could not be encoded: {error}"),
        )
    })?;
    let mut matched = executions.values().filter(|execution| {
        let output = execution.get("output_request");
        execution.get("lifecycle").and_then(Value::as_str)
            == Some("completed_pending_output_settlement")
            && execution.get("output_outcome").is_none_or(Value::is_null)
            && execution.get("completion_certificate") == Some(&certificate)
            && output.and_then(|value| value.get("operation_key"))
                == serde_json::to_value(&request.operation_key).ok().as_ref()
            && output.and_then(|value| value.get("account"))
                == serde_json::to_value(&request.account).ok().as_ref()
            && output.and_then(|value| value.get("expected_account_revision"))
                == serde_json::to_value(request.expected_account_revision)
                    .ok()
                    .as_ref()
            && output.and_then(|value| value.get("resource"))
                == serde_json::to_value(&request.resource_revision)
                    .ok()
                    .as_ref()
            && output.and_then(|value| value.get("unit"))
                == serde_json::to_value(&request.unit_revision).ok().as_ref()
            && output
                .and_then(|value| value.get("quantity"))
                .and_then(Value::as_u64)
                == Some(request.quantity)
    });
    let execution = matched.next().ok_or_else(|| {
        CanwuError::new(
            ErrorCode::InvalidAuthority,
            "production provider payload does not authorize the exact resource credit",
        )
    })?;
    if matched.next().is_some() {
        return Err(CanwuError::new(
            ErrorCode::InvalidDomainRecord,
            "production provider payload ambiguously authorizes the resource credit",
        ));
    }
    execution.get("id").cloned().map(Some).ok_or_else(|| {
        CanwuError::new(
            ErrorCode::InvalidDomainRecord,
            "production provider execution has no canonical identity",
        )
    })
}

fn authoritative_force_consumption(
    payload: &Value,
    request: &crate::ResourceConsumptionRequestV1,
) -> Result<(), CanwuError> {
    let intents = payload
        .get("intents")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidDomainRecord,
                "force provider payload has no authoritative consumption intents",
            )
        })?;
    let allocation = serde_json::to_value(&request.allocation)
        .map_err(|error| CanwuError::new(ErrorCode::InvalidPayload, error.to_string()))?;
    let certificate = serde_json::to_value(&request.completion_certificate)
        .map_err(|error| CanwuError::new(ErrorCode::InvalidPayload, error.to_string()))?;
    let operation_key = serde_json::to_value(&request.operation_key)
        .map_err(|error| CanwuError::new(ErrorCode::InvalidPayload, error.to_string()))?;
    let consumption_id = serde_json::to_value(&request.consumption_id)
        .map_err(|error| CanwuError::new(ErrorCode::InvalidPayload, error.to_string()))?;
    let matches = intents.values().filter(|intent| {
        intent.get("status").and_then(Value::as_str) == Some("pending_resource_consumption")
            && intent.get("resource_operation_key") == Some(&operation_key)
            && intent.get("consumption_id") == Some(&consumption_id)
            && intent.get("allocation") == Some(&allocation)
            && intent.get("completion_certificate") == Some(&certificate)
    });
    let matching_count = matches.count();
    if matching_count != 1 {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            format!(
                "force provider payload does not uniquely authorize the exact persisted consumption intent (matches={matching_count}, operation_key={}, consumption_id={})",
                request.operation_key, request.consumption_id,
            ),
        ));
    }
    Ok(())
}

fn authoritative_economy_consumption(
    payload: &Value,
    packet: &ResourceAdapterOperationV1,
    request: &crate::ResourceConsumptionRequestV1,
    state: &crate::ResourceState,
) -> Result<(), CanwuError> {
    let leg = state
        .allocation_legs
        .get(&request.allocation.id)
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidDomainRecord,
                "resource allocation is unavailable",
            )
        })?;
    let demand = state.demands.get(&leg.demand).ok_or_else(|| {
        CanwuError::new(
            ErrorCode::InvalidDomainRecord,
            "resource demand is unavailable",
        )
    })?;
    let intents = payload
        .get("resource_consumption_intents")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidDomainRecord,
                "economy provider has no typed resource consumption intents",
            )
        })?;
    if request.consumer_evidence != packet.provider_source {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "economy consumption evidence is not the exact provider payload version",
        ));
    }
    let mut matching = 0_usize;
    for (map_id, value) in intents {
        let intent: crate::ResourceConsumptionIntentV1 = serde_json::from_value(value.clone())
            .map_err(|error| {
                CanwuError::new(
                    ErrorCode::InvalidDomainRecord,
                    format!("economy resource consumption intent cannot be decoded: {error}"),
                )
            })?;
        intent.validate().map_err(resource_canwu_error)?;
        if map_id != intent.id.as_str() {
            return Err(CanwuError::new(
                ErrorCode::InvalidDomainRecord,
                "economy resource consumption intent map key differs from its identity",
            ));
        }
        if intent.status == crate::ResourceConsumptionIntentStatusV1::Authorized
            && intent.provider_plugin == packet.provider_plugin
            && intent.demand == leg.demand
            && intent.demand_revision == leg.demand_revision
            && intent.demand_revision == demand.revision
            && intent.allocation == request.allocation
            && intent.account == leg.account
            && intent.expected_account_revision == request.expected_account_revision
            && intent.consumption_id == request.consumption_id
            && intent.operation_key == request.operation_key
            && intent.quantity == leg.quantity
        {
            matching = matching.checked_add(1).ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::ValueOutOfRange,
                    "economy resource consumption intent match count overflowed",
                )
            })?;
        }
    }
    if matching != 1 {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            format!(
                "economy provider payload does not uniquely authorize the exact typed consumption intent (matches={matching})"
            ),
        ));
    }
    Ok(())
}

const fn request_time(request: &ResourceOperationRequestV1) -> Option<SimTime> {
    match request {
        ResourceOperationRequestV1::Consume(value) => Some(value.at),
        ResourceOperationRequestV1::BeginTransfer(value) => Some(value.at),
        ResourceOperationRequestV1::CancelTransfer(value) => Some(value.at),
        ResourceOperationRequestV1::CompleteTransfer(value) => Some(value.at),
        ResourceOperationRequestV1::Credit(value) => Some(value.at),
        ResourceOperationRequestV1::ExternalOutflow(value) => Some(value.at),
        _ => None,
    }
}

fn validate_completion_ingress(
    view: &SimulationView<'_>,
    at: SimTime,
    operation: &crate::ResourceCompletionOperationV1,
    state: &crate::ResourceState,
) -> Result<(), CanwuError> {
    match operation {
        crate::ResourceCompletionOperationV1::Acquire(request) => {
            if request.eligibility_time != at {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "completion acquisition eligibility time must equal its canonical boundary",
                ));
            }
            validate_completion_report_reservation(view, state, &request.recipe)?;
            for version in request
                .eligibility_envelope
                .exact_evidence
                .iter()
                .chain(&request.eligibility_envelope.capability_bindings)
                .chain(&request.eligibility_envelope.route_evidence)
            {
                require_exact_body(view, version)?;
            }
        }
        crate::ResourceCompletionOperationV1::Grant(request) => {
            for target in &request.target_versions {
                if let crate::CompletionLockedTargetV1::ExternalRecord { version } = target {
                    require_exact_current_body(view, version)?;
                }
            }
        }
        crate::ResourceCompletionOperationV1::Activate(request) => {
            if request.at != at {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "completion activation must use the canonical boundary simulation time",
                ));
            }
        }
        crate::ResourceCompletionOperationV1::Expire(request) => {
            if request.at != at {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "completion expiry must use the canonical boundary simulation time",
                ));
            }
        }
        crate::ResourceCompletionOperationV1::Prepare(request) => {
            if !state
                .completion_leases
                .acquisitions
                .contains_key(&request.acquisition)
                || !state.completion_leases.grants.contains_key(&request.grant)
            {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "completion prepare acquisition or grant is unavailable",
                ));
            }
        }
        crate::ResourceCompletionOperationV1::Abort(_)
        | crate::ResourceCompletionOperationV1::Release(_)
        | crate::ResourceCompletionOperationV1::CompleteExternalParticipant(_) => {}
        crate::ResourceCompletionOperationV1::GrantExternalParticipant(request) => {
            if request.eligibility_time != at {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "external participant grant eligibility time must equal its canonical boundary",
                ));
            }
            validate_completion_report_reservation(view, state, &request.recipe)?;
            let source = require_external_coordinator_source(
                view,
                &request.coordinator_plugin,
                &request.coordinator_source,
            )?;
            validate_external_grant_authorization(&source.payload, request)?;
            for target in &request.target_versions {
                if let crate::CompletionLockedTargetV1::ExternalRecord { version } = target {
                    require_exact_current_body(view, version)?;
                }
            }
        }
        crate::ResourceCompletionOperationV1::PrepareExternalParticipant(request) => {
            let source = &request.coordinator_source;
            let record = view.domain_record_version(source)?.ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::InvalidDomainRecord,
                    "external completion coordinator source is unavailable",
                )
            })?;
            let source = require_external_coordinator_source(view, &record.owner, source)?;
            let participant = state
                .external_completion_participants
                .grants
                .get(&request.acquisition)
                .ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::InvalidAuthority,
                        "external completion participant grant is unavailable for prepare authorization",
                    )
                })?;
            validate_external_participant_authorization(
                &source.payload,
                participant,
                ExternalCoordinatorTransition::Prepare,
            )?;
        }
        crate::ResourceCompletionOperationV1::ReleaseExternalParticipant(request) => {
            let source = &request.coordinator_source;
            let record = view.domain_record_version(source)?.ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::InvalidDomainRecord,
                    "external completion coordinator source is unavailable",
                )
            })?;
            let source = require_external_coordinator_source(view, &record.owner, source)?;
            let participant = state
                .external_completion_participants
                .grants
                .get(&request.acquisition)
                .ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::InvalidAuthority,
                        "external completion participant grant is unavailable for release authorization",
                    )
                })?;
            validate_external_participant_authorization(
                &source.payload,
                participant,
                ExternalCoordinatorTransition::Release,
            )?;
        }
        crate::ResourceCompletionOperationV1::ConsumeExternalParticipant(request) => {
            if request.at != at {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "external participant consumption must use canonical boundary time",
                ));
            }
            let record = view
                .domain_record_version(&request.coordinator_source)?
                .ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::InvalidDomainRecord,
                        "external completion coordinator source is unavailable",
                    )
                })?;
            let source = require_external_coordinator_source(
                view,
                &record.owner,
                &request.coordinator_source,
            )?;
            let participant = state
                .external_completion_participants
                .grants
                .get(&request.certificate.acquisition)
                .ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::InvalidAuthority,
                        "external completion participant grant is unavailable for consume authorization",
                    )
                })?;
            validate_external_participant_authorization(
                &source.payload,
                participant,
                ExternalCoordinatorTransition::Consume(&request.certificate),
            )?;
        }
        crate::ResourceCompletionOperationV1::ExpireExternalParticipants(request) => {
            if request.at != at {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "external participant expiry must use canonical boundary time",
                ));
            }
        }
    }
    Ok(())
}

fn validate_completion_report_reservation(
    view: &SimulationView<'_>,
    state: &crate::ResourceState,
    recipe: &crate::CompletionCapacityRecipeV1,
) -> Result<(), CanwuError> {
    let requested = usize::from(recipe.reports_per_holder)
        .checked_mul(usize::from(recipe.holders))
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::KnowledgeLimitExceeded,
                "completion report reservation overflowed",
            )
        })?;
    let projected = view
        .knowledge_record_count_in_namespace(PLUGIN_NAMESPACE)?
        .checked_add(
            state
                .reserved_knowledge_report_slots()
                .map_err(resource_canwu_error)?,
        )
        .and_then(|value| value.checked_add(requested))
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::KnowledgeLimitExceeded,
                "completion report reservation overflowed",
            )
        })?;
    if projected > RESOURCE_REPORT_HOT_CAPACITY {
        return Err(CanwuError::new(
            ErrorCode::KnowledgeLimitExceeded,
            "completion admission cannot reserve its bounded terminal report path",
        ));
    }
    Ok(())
}

fn require_external_coordinator_source(
    view: &SimulationView<'_>,
    coordinator_plugin: &str,
    source: &canwu_api::DomainRecordVersionRef,
) -> Result<canwu_api::DomainRecord, CanwuError> {
    require_exact_current_body(view, source)?;
    let record = view.domain_record_version(source)?.ok_or_else(|| {
        CanwuError::new(
            ErrorCode::InvalidDomainRecord,
            "external completion coordinator source is unavailable",
        )
    })?;
    if coordinator_plugin.is_empty() || record.owner != coordinator_plugin {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "external completion coordinator source owner differs from the declared coordinator",
        ));
    }
    Ok(record)
}

#[derive(Clone, Copy)]
enum ExternalCoordinatorTransition<'a> {
    Prepare,
    Consume(&'a crate::CompletionLeaseActivationCertificateV1),
    Release,
}

fn validate_external_grant_authorization(
    payload: &Value,
    request: &crate::RequestExternalCompletionParticipantGrantV1,
) -> Result<(), CanwuError> {
    let acquisition = coordinator_acquisition(payload, &request.acquisition)?;
    let exact = json_field_matches(acquisition, "id", &request.acquisition)?
        && json_field_matches(
            acquisition,
            "revision",
            request.coordinator_acquisition_revision,
        )?
        && json_field_matches(acquisition, "operation_key", &request.operation_key)?
        && json_field_matches(acquisition, "holder", &request.holder)?
        && json_field_matches(
            acquisition,
            "operation_namespace",
            &request.operation_namespace,
        )?
        && json_field_matches(acquisition, "eligibility_time", request.eligibility_time)?
        && json_field_matches(acquisition, "recipe", &request.recipe)?
        && json_field_matches(acquisition, "policy_class", request.policy_class)?
        && acquisition
            .get("eligibility_envelope")
            .and_then(|value| value.get("digest"))
            .and_then(Value::as_str)
            == Some(request.eligibility_envelope_digest.as_str())
        && acquisition
            .get("expected_participants")
            .and_then(Value::as_array)
            .is_some_and(|participants| {
                participants
                    .iter()
                    .any(|participant| participant.as_str() == Some(crate::PLUGIN_NAME))
            });
    if !exact {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "coordinator payload does not authorize the exact external resource participant grant",
        ));
    }
    Ok(())
}

fn validate_external_participant_authorization(
    payload: &Value,
    participant: &crate::ExternalCompletionParticipantGrantV1,
    transition: ExternalCoordinatorTransition<'_>,
) -> Result<(), CanwuError> {
    let acquisition = coordinator_acquisition(payload, &participant.grant.acquisition)?;
    let exact_acquisition = json_field_matches(
        acquisition,
        "operation_key",
        &participant.grant.operation_key,
    )? && json_field_matches(acquisition, "holder", &participant.holder)?
        && json_field_matches(
            acquisition,
            "operation_namespace",
            &participant.operation_namespace,
        )?
        && json_field_matches(
            acquisition,
            "eligibility_time",
            participant.eligibility_time,
        )?
        && json_field_matches(acquisition, "recipe", &participant.recipe)?
        && json_field_matches(acquisition, "policy_class", participant.policy_class)?
        && acquisition
            .get("eligibility_envelope")
            .and_then(|value| value.get("digest"))
            .and_then(Value::as_str)
            == Some(participant.eligibility_envelope_digest.as_str())
        && acquisition
            .get("expected_participants")
            .and_then(Value::as_array)
            .is_some_and(|participants| {
                participants
                    .iter()
                    .any(|value| value.as_str() == Some(crate::PLUGIN_NAME))
            });
    if !exact_acquisition {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "coordinator payload does not bind the exact external participant acquisition",
        ));
    }
    let coordinator_grant = coordinator_participant_grant(
        payload,
        &participant.grant.acquisition,
        crate::PLUGIN_NAME,
        &participant.grant.id,
    )?;
    let exact_stable_grant = json_field_matches(coordinator_grant, "id", &participant.grant.id)?
        && json_field_matches(
            coordinator_grant,
            "acquisition",
            &participant.grant.acquisition,
        )?
        && json_field_matches(
            coordinator_grant,
            "operation_key",
            &participant.grant.operation_key,
        )?
        && json_field_matches(coordinator_grant, "owner_plugin", crate::PLUGIN_NAME)?
        && json_field_matches(
            coordinator_grant,
            "target_versions",
            &participant.grant.target_versions,
        )?
        && json_field_matches(
            coordinator_grant,
            "recipe_digest",
            &participant.grant.recipe_digest,
        )?
        && json_field_matches(
            coordinator_grant,
            "reserved_units",
            participant.grant.reserved_units,
        )?;
    if !exact_stable_grant {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "coordinator payload does not bind the exact external participant grant and targets",
        ));
    }
    match transition {
        ExternalCoordinatorTransition::Prepare => {
            if !json_field_matches(coordinator_grant, "revision", participant.grant.revision)?
                || coordinator_grant.get("state").and_then(Value::as_str) != Some("held")
            {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "coordinator payload does not authorize preparing the exact held resource grant",
                ));
            }
        }
        ExternalCoordinatorTransition::Consume(certificate) => {
            if !json_field_matches(coordinator_grant, "revision", participant.grant.revision)?
                || coordinator_grant.get("state").and_then(Value::as_str) != Some("prepared")
                || coordinator_certificate(payload, &certificate.acquisition)
                    != Some(&serde_json::to_value(certificate).map_err(|error| {
                        CanwuError::new(ErrorCode::InvalidPayload, error.to_string())
                    })?)
            {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "coordinator payload does not authorize consuming the exact prepared resource grant",
                ));
            }
        }
        ExternalCoordinatorTransition::Release => {
            let state = acquisition.get("state").and_then(Value::as_str);
            if !matches!(state, Some("aborting" | "released" | "expired")) {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "coordinator payload does not authorize releasing the external resource grant",
                ));
            }
        }
    }
    Ok(())
}

fn coordinator_acquisition<'a>(
    payload: &'a Value,
    acquisition: &crate::CompletionLeaseAcquisitionId,
) -> Result<&'a Value, CanwuError> {
    payload
        .get("completion_acquisitions")
        .or_else(|| payload.get("completion_leases")?.get("acquisitions"))
        .and_then(Value::as_object)
        .and_then(|values| values.get(acquisition.as_str()))
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidAuthority,
                "external coordinator payload has no exact completion acquisition",
            )
        })
}

fn coordinator_participant_grant<'a>(
    payload: &'a Value,
    acquisition: &crate::CompletionLeaseAcquisitionId,
    participant: &str,
    grant: &crate::CompletionCapacityGrantId,
) -> Result<&'a Value, CanwuError> {
    let production_grant = payload
        .get("completion_participant_grants")
        .and_then(Value::as_object)
        .and_then(|values| values.get(acquisition.as_str()))
        .and_then(Value::as_object)
        .and_then(|values| values.get(participant))
        .and_then(|value| value.get("grant"));
    let force_grant = payload
        .get("completion_leases")
        .and_then(|value| value.get("grants"))
        .and_then(Value::as_object)
        .and_then(|values| values.get(grant.as_str()));
    let value = production_grant.or(force_grant).ok_or_else(|| {
        CanwuError::new(
            ErrorCode::InvalidAuthority,
            "external coordinator payload has no exact resource participant grant",
        )
    })?;
    let acquisition_grant = coordinator_acquisition(payload, acquisition)?
        .get("grants")
        .and_then(Value::as_object)
        .and_then(|values| values.get(participant));
    if production_grant.is_none()
        && acquisition_grant
            != Some(
                &serde_json::to_value(grant).map_err(|error| {
                    CanwuError::new(ErrorCode::InvalidPayload, error.to_string())
                })?,
            )
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "external coordinator acquisition does not name the exact resource participant grant",
        ));
    }
    Ok(value)
}

fn coordinator_certificate<'a>(
    payload: &'a Value,
    acquisition: &crate::CompletionLeaseAcquisitionId,
) -> Option<&'a Value> {
    payload
        .get("production_completion_certificates")
        .or_else(|| payload.get("completion_leases")?.get("certificates"))
        .and_then(Value::as_object)
        .and_then(|values| values.get(acquisition.as_str()))
}

fn json_field_matches<T: Serialize>(
    object: &Value,
    field: &str,
    expected: T,
) -> Result<bool, CanwuError> {
    Ok(object.get(field)
        == Some(
            &serde_json::to_value(expected)
                .map_err(|error| CanwuError::new(ErrorCode::InvalidPayload, error.to_string()))?,
        ))
}

fn validate_request_certificate_evidence(
    view: &SimulationView<'_>,
    request: &ResourceOperationRequestV1,
) -> Result<(), CanwuError> {
    let certificate = match request {
        ResourceOperationRequestV1::Consume(value) => Some(&value.completion_certificate),
        ResourceOperationRequestV1::BeginTransfer(value) => Some(&value.completion_certificate),
        ResourceOperationRequestV1::CompleteTransfer(value) => Some(&value.completion_certificate),
        ResourceOperationRequestV1::Credit(value) => Some(&value.completion_certificate),
        ResourceOperationRequestV1::ExternalOutflow(value) => Some(&value.completion_certificate),
        _ => None,
    };
    if let Some(certificate) = certificate {
        for target in &certificate.locked_target_versions {
            if let crate::CompletionLockedTargetV1::ExternalRecord { version } = target {
                require_exact_body(view, version)?;
            }
        }
    }
    Ok(())
}

fn require_exact_body(
    view: &SimulationView<'_>,
    version: &canwu_api::DomainRecordVersionRef,
) -> Result<(), CanwuError> {
    if view.domain_record_version(version)?.is_none() {
        return Err(CanwuError::new(
            ErrorCode::EvidenceContentUnavailable,
            "resource exact completion evidence body is unavailable",
        ));
    }
    Ok(())
}

fn require_exact_current_body(
    view: &SimulationView<'_>,
    version: &canwu_api::DomainRecordVersionRef,
) -> Result<(), CanwuError> {
    require_exact_body(view, version)?;
    if !view.domain_record_version_is_current(version)? {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "resource locked external target is not the current exact version",
        ));
    }
    Ok(())
}

fn adapter_source_matches(packet: &ResourceAdapterOperationV1) -> bool {
    match &packet.request {
        ResourceOperationRequestV1::Consume(value) => {
            if packet.provider_plugin == "canwu-force-supply-reference" {
                value.consumer_evidence.record == packet.provider_source.record
                    && value.consumer_evidence.version <= packet.provider_source.version
            } else {
                value.consumer_evidence == packet.provider_source
            }
        }
        ResourceOperationRequestV1::Credit(value) => match &value.source {
            crate::ResourceCreditSourceV1::Production(source)
            | crate::ResourceCreditSourceV1::ExternalInflow(
                canwu_api::EvidenceRef::DomainRecordVersion(source),
            ) => source == &packet.provider_source,
            crate::ResourceCreditSourceV1::ExternalInflow(_) => false,
        },
        ResourceOperationRequestV1::ExternalOutflow(value) => {
            value.authority_evidence == packet.provider_source
        }
        ResourceOperationRequestV1::AdvanceTransfer(value) => {
            value.transport_evidence == packet.provider_source
        }
        ResourceOperationRequestV1::RecordObservation(value) => {
            value.head.provider_source == packet.provider_source
        }
        ResourceOperationRequestV1::CompleteTransfer(value) => {
            matches!(
                &value.disposition,
                crate::ResourceTransferDispositionV1::ExternalOutflow { authority_evidence }
                    if authority_evidence == &packet.provider_source
            ) || matches!(
                &value.disposition,
                crate::ResourceTransferDispositionV1::Accept { acceptance, .. }
                    if acceptance.evidence == packet.provider_source
            )
        }
        ResourceOperationRequestV1::CreateAccount(_)
        | ResourceOperationRequestV1::SubmitDemand(_)
        | ResourceOperationRequestV1::AmendDemand(_)
        | ResourceOperationRequestV1::Allocate(_)
        | ResourceOperationRequestV1::BeginTransfer(_)
        | ResourceOperationRequestV1::CancelTransfer(_)
        | ResourceOperationRequestV1::SetProtectedFloor(_)
        | ResourceOperationRequestV1::CancelDemand(_)
        | ResourceOperationRequestV1::Completion(_) => false,
    }
}

fn validate_resource_authority(
    view: &SimulationView<'_>,
    context: &CommandContext,
    value: &ResourceCommandV1,
) -> Result<(), CanwuError> {
    validate_resource_subject_issuer(context, &value.subject)?;
    let record = view
        .typed_domain_record(&resource_runtime_reference())?
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidDomainRecord,
                "resource runtime is unavailable for target-bound authority",
            )
        })?;
    let state = record.decode_payload::<ResourceRuntimeRecord>()?;
    let target_holder = match &value.request {
        ResourceOperationRequestV1::CreateAccount(request) => {
            Some(request.account.custodian.clone())
        }
        ResourceOperationRequestV1::SubmitDemand(request) => Some(request.demand.requester.clone()),
        ResourceOperationRequestV1::AmendDemand(request) => {
            let current = state.demands.get(&request.replacement.id).ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "resource demand target is unavailable",
                )
            })?;
            if current.requester != request.replacement.requester {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "resource demand amendment changes its authority holder",
                ));
            }
            Some(current.requester.clone())
        }
        ResourceOperationRequestV1::BeginTransfer(request) => state
            .accounts
            .get(&request.allocation.account)
            .map(|account| account.custodian.clone()),
        ResourceOperationRequestV1::CancelTransfer(request) => {
            let transfer = state.transfers.get(&request.transfer).ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "resource transfer target is unavailable",
                )
            })?;
            state
                .accounts
                .get(&transfer.source)
                .map(|account| account.custodian.clone())
        }
        ResourceOperationRequestV1::CompleteTransfer(request) => {
            if matches!(
                request.disposition,
                crate::ResourceTransferDispositionV1::Accept { .. }
                    | crate::ResourceTransferDispositionV1::ExternalOutflow { .. }
            ) {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "resource destination acceptance and transfer outflow require canonical provider ingress",
                ));
            }
            let transfer = state.transfers.get(&request.transfer).ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "resource transfer target is unavailable",
                )
            })?;
            let account_id = match &request.disposition {
                crate::ResourceTransferDispositionV1::Accept { destination, .. } => destination,
                crate::ResourceTransferDispositionV1::Lose { .. }
                | crate::ResourceTransferDispositionV1::Return { .. }
                | crate::ResourceTransferDispositionV1::ExternalOutflow { .. } => &transfer.source,
            };
            state
                .accounts
                .get(account_id)
                .map(|account| account.custodian.clone())
        }
        ResourceOperationRequestV1::ExternalOutflow(request) => state
            .accounts
            .get(&request.account)
            .map(|account| account.custodian.clone()),
        ResourceOperationRequestV1::SetProtectedFloor(request) => state
            .accounts
            .get(&request.account)
            .map(|account| account.custodian.clone()),
        ResourceOperationRequestV1::CancelDemand(request) => state
            .demands
            .get(&request.demand)
            .map(|demand| demand.requester.clone()),
        ResourceOperationRequestV1::Completion(crate::ResourceCompletionOperationV1::Acquire(
            request,
        )) => Some(request.holder.clone()),
        ResourceOperationRequestV1::Completion(crate::ResourceCompletionOperationV1::Abort(
            request,
        )) => Some(request.holder.clone()),
        ResourceOperationRequestV1::Allocate(_)
        | ResourceOperationRequestV1::Consume(_)
        | ResourceOperationRequestV1::AdvanceTransfer(_)
        | ResourceOperationRequestV1::Credit(_)
        | ResourceOperationRequestV1::RecordObservation(_)
        | ResourceOperationRequestV1::Completion(_) => {
            return Err(CanwuError::new(
                ErrorCode::InvalidAuthority,
                "resource operation is restricted to canonical provider ingress",
            ));
        }
    }
    .ok_or_else(|| {
        CanwuError::new(
            ErrorCode::InvalidAuthority,
            "resource operation target holder is unavailable",
        )
    })?;
    if target_holder != value.subject {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "resource operation subject does not control the actual target holder",
        ));
    }
    Ok(())
}

fn validate_resource_subject_issuer(
    context: &CommandContext,
    subject_holder: &canwu_api::KnowledgeHolderRef,
) -> Result<(), CanwuError> {
    let subject = holder_entity(subject_holder);
    let authorized = match (&context.issuer, subject_holder) {
        (Issuer::Actor(actor), canwu_api::KnowledgeHolderRef::Person(holder)) => actor == holder,
        (Issuer::Human(_) | Issuer::Ai(_) | Issuer::Institution(_), _) => {
            context.authority.command_subject.as_ref() == Some(&subject)
        }
        _ => false,
    };
    if !authorized {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "resource operation issuer does not control its holder-bound subject",
        ));
    }
    Ok(())
}

fn validate_resource_activation_records(records: &[DomainRecord]) -> Result<(), CanwuError> {
    let resource_records: Vec<_> = records
        .iter()
        .filter(|record| {
            record
                .reference
                .kind
                .matches_type::<ResourceRuntimeRecord>()
        })
        .collect();
    if resource_records.len() > 1 {
        return Err(CanwuError::new(
            ErrorCode::InvalidDomainRecord,
            "resource plugin owns more than one runtime root",
        ));
    }
    if let Some(record) = resource_records.first() {
        let state = record.decode_payload::<ResourceRuntimeRecord>()?;
        state.validate().map_err(resource_canwu_error)?;
        if record.reference != resource_runtime_reference().into_untyped()
            || record.version != state.state_revision.get()
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidDomainRecord,
                "resource runtime root identity or revision differs",
            ));
        }
    }
    Ok(())
}

#[must_use]
pub fn resource_command_descriptor() -> PluginActionDescriptor {
    PluginActionDescriptor {
        name: RESOURCE_COMMAND.to_owned(),
        description: "Admit one authority-bound resource operation".to_owned(),
        payload_schema: PayloadSchema::Any,
        reads: vec![StateKey::core_evidence(), resource_state_key()],
        writes: Vec::new(),
    }
}

pub fn resource_command(value: &ResourceCommandV1) -> Result<Command, serde_json::Error> {
    Ok(Command::Plugin {
        plugin: PLUGIN_NAME.to_owned(),
        command: RESOURCE_COMMAND.to_owned(),
        payload: serde_json::to_value(value)?,
    })
}

pub fn resource_adapter_ingress(
    due_at: SimTime,
    packet: &ResourceAdapterOperationV1,
) -> Result<PluginIngressRequest, CanwuError> {
    Ok(PluginIngressRequest::new(
        PLUGIN_NAME,
        RESOURCE_ADAPTER_INGRESS,
        due_at,
        encode(packet)?,
    ))
}

pub fn enqueue_resource_adapter_operation(
    canwu: &mut Canwu,
    due_at: SimTime,
    packet: &ResourceAdapterOperationV1,
) -> Result<canwu_api::IngressReceipt, CanwuError> {
    if !canwu.domain_record_version_evidence_exists(&packet.provider_source) {
        return Err(CanwuError::new(
            ErrorCode::EvidenceUnavailable,
            "resource adapter source exact version is unavailable",
        ));
    }
    canwu.enqueue_plugin_ingress(resource_adapter_ingress(due_at, packet)?)
}

/// Queue one bounded deterministic allocation pass for the exact requester.
/// The opaque plugin permit prevents a host-authored packet from bypassing this
/// helper; settlement additionally checks the current state revision and
/// filters every due/dirty candidate by `ResourceDemand::requester`.
pub fn enqueue_resource_allocation(
    canwu: &mut Canwu,
    due_at: SimTime,
    requester: &canwu_api::KnowledgeHolderRef,
    request: &crate::ResourceAllocationRequestV1,
) -> Result<canwu_api::IngressReceipt, CanwuError> {
    let permit = RESOURCE_ALLOCATION_INGRESS_PERMIT.get().ok_or_else(|| {
        CanwuError::new(
            ErrorCode::PluginNotActive,
            "resource plugin must be active before allocation ingress",
        )
    })?;
    canwu.enqueue_permitted_plugin_ingress(
        PluginIngressRequest::new(
            PLUGIN_NAME,
            RESOURCE_ALLOCATION_INGRESS,
            due_at,
            encode(&ResourceAuthorizedAllocationV1 {
                requester: requester.clone(),
                request: request.clone(),
            })?,
        ),
        permit,
    )
}

/// Queue one persisted completion lease transition through the plugin-owned
/// canonical ingress. This is the only public non-command path for grant,
/// prepare, activate, expiry, and release operations.
pub fn enqueue_resource_completion_operation(
    canwu: &mut Canwu,
    due_at: SimTime,
    operation: &crate::ResourceCompletionOperationV1,
) -> Result<canwu_api::IngressReceipt, CanwuError> {
    let permit = RESOURCE_COMPLETION_INGRESS_PERMIT.get().ok_or_else(|| {
        CanwuError::new(
            ErrorCode::PluginNotActive,
            "resource plugin must be active before completion ingress",
        )
    })?;
    canwu.enqueue_permitted_plugin_ingress(
        PluginIngressRequest::new(
            PLUGIN_NAME,
            RESOURCE_COMPLETION_INGRESS,
            due_at,
            encode(operation)?,
        ),
        permit,
    )
}

/// Stores, reads back, verifies, and enqueues a bounded terminal archive batch
/// with one host-retained authenticated directory root.
pub fn enqueue_resource_archive(
    canwu: &mut Canwu,
    prepared: &crate::PreparedResourceArchiveBatchV1,
    store: &dyn crate::ResourceArchiveStore,
) -> Result<ResourceArchiveIngressReceiptV1, CanwuError> {
    let (_, state) = crate::resource_state(canwu)?
        .ok_or_else(|| CanwuError::new(ErrorCode::PluginNotActive, "resource runtime is absent"))?;
    let canonical = state
        .prepare_resource_archive(prepared.selected.len())
        .map_err(resource_canwu_error)?;
    if &canonical != prepared {
        return Err(CanwuError::new(
            ErrorCode::InvalidArchive,
            "resource archive batch differs from the exact current terminal candidates",
        ));
    }
    let commit = prepared
        .store_and_verify(store)
        .map_err(resource_canwu_error)?;
    let permit = RESOURCE_ARCHIVE_INGRESS_PERMIT.get().ok_or_else(|| {
        CanwuError::new(
            ErrorCode::PluginNotActive,
            "resource plugin must be active before archive ingress",
        )
    })?;
    let retention = [PluginArchiveRetention {
        namespace: crate::RESOURCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE.to_owned(),
        object_id: commit.directory_root.clone(),
    }];
    let mut durable = commit.retention.clone();
    durable.phase = crate::ResourceArchiveRetentionPhaseV1::DurableIngress;
    durable.semantic_digest.clear();
    durable.semantic_digest =
        crate::canonical_digest("canwu.resource.archive-retention.v1", &durable)
            .map_err(resource_canwu_error)?;
    store
        .persist_resource_archive_retention(&durable)
        .map_err(resource_canwu_error)?;
    let ingress = canwu.enqueue_permitted_plugin_ingress(
        PluginIngressRequest::new(
            PLUGIN_NAME,
            RESOURCE_ARCHIVE_COMMIT_INGRESS,
            canwu.time(),
            encode(&commit)?,
        )
        .with_archive_retention(retention),
        permit,
    );
    if ingress.is_err() {
        let _ = store.finalize_resource_archive_retention(
            &commit.retention.id,
            crate::ResourceArchiveRetentionPhaseV1::Abandoned,
        );
    }
    ingress.map(|ingress| ResourceArchiveIngressReceiptV1 {
        ingress,
        retention_handle_id: commit.retention.id,
        directory_root: commit.directory_root,
    })
}

/// Finalizes the store-side retention handle after the authoritative resource
/// state records an applied or stale terminal archive disposition.
pub fn finalize_resource_archive_retention(
    canwu: &mut Canwu,
    store: &dyn crate::ResourceArchiveStore,
    receipt: &ResourceArchiveIngressReceiptV1,
) -> Result<canwu_api::IngressReceipt, CanwuError> {
    let (_, state) = crate::resource_state(canwu)?
        .ok_or_else(|| CanwuError::new(ErrorCode::PluginNotActive, "resource runtime is absent"))?;
    let terminal = state
        .archive_maintenance_receipts
        .values()
        .find(|terminal| {
            terminal.retention_handle_id == receipt.retention_handle_id
                && terminal.directory_root == receipt.directory_root
        })
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidArchive,
                "resource archive terminal disposition is unavailable",
            )
        })?
        .clone();
    let phase = match terminal.disposition {
        crate::ResourceArchiveMaintenanceDispositionV1::Applied => {
            crate::ResourceArchiveRetentionPhaseV1::Committed
        }
        crate::ResourceArchiveMaintenanceDispositionV1::RejectedStale => {
            crate::ResourceArchiveRetentionPhaseV1::RejectedStale
        }
    };
    store
        .finalize_resource_archive_retention(&receipt.retention_handle_id, phase)
        .map_err(resource_canwu_error)?;
    let permit = RESOURCE_ARCHIVE_RETENTION_ACK_PERMIT.get().ok_or_else(|| {
        CanwuError::new(
            ErrorCode::PluginNotActive,
            "resource plugin must be active before archive acknowledgement",
        )
    })?;
    canwu.enqueue_permitted_plugin_ingress(
        PluginIngressRequest::new(
            PLUGIN_NAME,
            RESOURCE_ARCHIVE_RETENTION_ACK_INGRESS,
            canwu.time(),
            encode(&ResourceArchiveRetentionAcknowledgementV1 { receipt: terminal })?,
        ),
        permit,
    )
}

#[must_use]
pub fn resource_report_knowledge_schema_id() -> KnowledgeSchemaId {
    KnowledgeSchemaId::new(
        KnowledgeRecordKind::new(PLUGIN_NAMESPACE, RESOURCE_REPORT_KNOWLEDGE),
        1,
    )
}

fn resource_report_knowledge_schema() -> PluginKnowledgeSchema {
    PluginKnowledgeSchema {
        id: resource_report_knowledge_schema_id(),
        schema_hash: RESOURCE_REPORT_SCHEMA_HASH.to_owned(),
        writable: true,
        payload_schema: PayloadSchema::Any,
        subjects: vec![KnowledgeSubjectSchema {
            role: "resource_state".to_owned(),
            targets: vec![KnowledgeSubjectTargetKind::Domain(
                DomainRecordKind::for_type::<ResourceRuntimeRecord>(),
            )],
            required: true,
            multiple: false,
        }],
    }
}

fn resource_state_key() -> StateKey {
    StateKey::new(
        ResourceRuntimeRecord::NAMESPACE,
        ResourceRuntimeRecord::NAME,
    )
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
            format!("resource value could not be encoded: {error}"),
        )
    })
}

fn invalid(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidDomainRecord, message)
}
