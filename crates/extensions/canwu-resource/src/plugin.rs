use crate::{
    PLUGIN_NAME, PLUGIN_NAMESPACE, ResourceOperationRequestV1, ResourceRuntimeRecord,
    holder_entity, materialize_resource_report, resource_canwu_error, resource_runtime_reference,
};
use canwu_api::{
    ArchiveReachabilityManifest, BoundaryContext, BoundaryDirective, BoundaryPhase,
    BoundaryProposal, BoundarySystemContract, CanwuError, CauseRef, Command, CommandContext,
    CommandIngress, DomainRecord, DomainRecordKind, DomainRecordSchema, DomainRecordType,
    EntityRef, ErrorCode, EventAudience, EvidenceRef, IngressClass, IngressId, IngressPayload,
    IngressRecord, Issuer, KnowledgeOrigin, KnowledgeRecordDraft, KnowledgeRecordKind,
    KnowledgeSchemaId, KnowledgeSubject, KnowledgeSubjectSchema, KnowledgeSubjectTarget,
    KnowledgeSubjectTargetKind, KnowledgeWriteGrant, PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD,
    PayloadSchema, PluginActionDescriptor, PluginArchiveObjectProvider, PluginArchiveRetention,
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
/// Internal packet the resource plugin queues to itself for work another
/// plugin scheduled (a production output batch, or a completion step not
/// bound to its boundary time, such as an external participant's completion)
/// whose result did not fit the root while waiting can make room for it.
/// It carries the deferred packet and the count of its deferrals, and comes
/// due one minute after the first deferral, the delay doubling with each
/// further one up to 1,024 minutes. Work waiting on the byte cap is skipped
/// after the 1,024-minute retry. It is internal ingress whose permit is never
/// handed out, so no host can queue it; the resource plugin's lifecycle
/// system schedules it.
pub const RESOURCE_DEFERRED_INGRESS: &str = "resource_deferred_packet_v1";
pub const RESOURCE_REPORT_KNOWLEDGE: &str = "resource_report";
pub const RESOURCE_SEMANTIC_HASH: &str =
    "56b1daef8b4a6a39b04f9454a5a7911d4b59ecbc1f3d4321ef5fce725e6a6778";

const RESOURCE_REPORT_SCHEMA_HASH: &str =
    "2e271b9ea79404e662ba51360ce8061421ae896c1e70b09e99feb8d0de01a007";
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

/// The acknowledgement of one settled archive commit's retention. It names
/// the commit by its maintenance receipt while the receipt is in the root,
/// and by its retention handle once a later commit has archived the receipt.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ResourceArchiveRetentionAcknowledgementV1 {
    Receipt(crate::ResourceArchiveMaintenanceReceiptV1),
    Handle {
        retention_handle_id: String,
        directory_root: String,
    },
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

/// In a debug build the resource plugin settles each boundary's packets a
/// second time, on the whole root, and requires the same result. The second
/// settlement reads every archive object the first one read from what the
/// first one read, so the archive store serves one settlement's reads, as in
/// a release build.
mod archive_reads {
    #[cfg(debug_assertions)]
    use std::cell::RefCell;
    #[cfg(debug_assertions)]
    use std::collections::BTreeMap;

    #[cfg(debug_assertions)]
    type Reads = BTreeMap<(String, String), Option<Vec<u8>>>;

    #[cfg(debug_assertions)]
    thread_local! {
        /// While a settlement records, its reads; while the second one
        /// replays them, the same reads marked as replayed.
        static READS: RefCell<Option<(bool, Reads)>> = const { RefCell::new(None) };
    }

    /// Runs the first settlement and records the archive objects it reads.
    #[cfg(debug_assertions)]
    pub(super) fn recording<T>(settle: impl FnOnce() -> T) -> T {
        READS.with(|reads| *reads.borrow_mut() = Some((false, BTreeMap::new())));
        let result = settle();
        READS.with(|reads| {
            if let Some((replay, _)) = reads.borrow_mut().as_mut() {
                *replay = true;
            }
        });
        result
    }

    /// Runs the second settlement against the reads the first recorded, and
    /// forgets them.
    #[cfg(debug_assertions)]
    pub(super) fn replaying<T>(settle: impl FnOnce() -> T) -> T {
        let result = settle();
        READS.with(|reads| *reads.borrow_mut() = None);
        result
    }

    #[cfg(not(debug_assertions))]
    pub(super) fn recording<T>(settle: impl FnOnce() -> T) -> T {
        settle()
    }

    pub(super) fn load(
        namespace: &str,
        object_id: &str,
        fetch: impl FnOnce() -> Result<Option<Vec<u8>>, crate::ResourceError>,
    ) -> Result<Option<Vec<u8>>, crate::ResourceError> {
        #[cfg(debug_assertions)]
        {
            let key = (namespace.to_owned(), object_id.to_owned());
            let replayed = READS.with(|reads| match reads.borrow().as_ref() {
                Some((true, recorded)) => recorded.get(&key).cloned(),
                _ => None,
            });
            if let Some(value) = replayed {
                return Ok(value);
            }
            let value = fetch()?;
            READS.with(|reads| {
                if let Some((false, recorded)) = reads.borrow_mut().as_mut() {
                    recorded.insert(key, value.clone());
                }
            });
            Ok(value)
        }
        #[cfg(not(debug_assertions))]
        {
            let _ = (namespace, object_id);
            fetch()
        }
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
        archive_reads::load(namespace, object_id, || {
            self.0
                .plugin_archive_object(namespace, object_id)
                .map_err(|error| crate::ResourceError::InvalidDefinition(error.to_string()))
        })
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

/// Who holds a request's operation key when its packet settles.
enum OperationKeyHolder {
    Nobody,
    /// The outcome of this request is in the root.
    ThisRequestInRoot,
    /// The outcome of this request is in the archive.
    ThisRequestInArchive,
    /// The key belongs to a different request, in the root or the archive.
    AnotherRequest,
}

/// Resolves the holder of a request's operation key from the root and, when
/// the root has one, the archive. Unlike [`archived_operation_outcome`] a key
/// held by a different request is an answer, not an error, so the caller can
/// leave the packet unsettled without failing its boundary.
fn operation_key_holder(
    view: &SimulationView<'_>,
    state: &crate::ResourceState,
    request: &ResourceOperationRequestV1,
    request_digest: &str,
) -> Result<OperationKeyHolder, CanwuError> {
    let operation_key = request.operation_key();
    if let Some(outcome) = state.outcomes.get(&operation_key) {
        return Ok(if outcome.request_digest == request_digest {
            OperationKeyHolder::ThisRequestInRoot
        } else {
            OperationKeyHolder::AnotherRequest
        });
    }
    if state.archive_head.directory_root.is_none() {
        return Ok(OperationKeyHolder::Nobody);
    }
    let provider = ViewResourceArchiveProvider(view);
    Ok(
        match crate::archived_resource_operation_outcome(state, &provider, &operation_key)
            .map_err(resource_canwu_error)?
        {
            None => OperationKeyHolder::Nobody,
            Some(outcome) if outcome.request_digest == request_digest => {
                OperationKeyHolder::ThisRequestInArchive
            }
            Some(_) => OperationKeyHolder::AnotherRequest,
        },
    )
}

/// The refusal of a lease start whose acquisition identity an earlier lease
/// took under the other way of starting a lease, hot or archived
/// ([`crate::ResourceCompletionOperationV1::sibling_lease_operation_key`]).
fn lease_identity_refusal(
    view: &SimulationView<'_>,
    state: &crate::ResourceState,
    operation: &crate::ResourceCompletionOperationV1,
) -> Result<Option<crate::ResourceError>, CanwuError> {
    let Some(key) = operation.sibling_lease_operation_key() else {
        return Ok(None);
    };
    // A refused lease start took nothing; only an applied one used the
    // identity.
    let applied = |outcome: &crate::ResourceOperationOutcome| {
        outcome.status == crate::ResourceOperationStatus::Applied
    };
    let taken = match state.outcomes.get(&key) {
        Some(outcome) => applied(outcome),
        None => {
            state.archive_head.directory_root.is_some()
                && crate::archived_resource_operation_outcome(
                    state,
                    &ViewResourceArchiveProvider(view),
                    &key,
                )
                .map_err(resource_canwu_error)?
                .is_some_and(|outcome| applied(&outcome))
        }
    };
    Ok(taken.then(crate::lease_identity_taken))
}

/// Refuses a request that names an operation key reserved for completion
/// operations, before it can be queued.
fn reject_reserved_operation_key(request: &ResourceOperationRequestV1) -> Result<(), CanwuError> {
    if request.names_reserved_operation_key() {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            RESERVED_OPERATION_KEY_DETAIL,
        ));
    }
    Ok(())
}

/// Refuses a completion operation whose identities do not form a valid
/// operation key, before it can be queued.
fn reject_invalid_completion_identity(
    operation: &crate::ResourceCompletionOperationV1,
) -> Result<(), CanwuError> {
    if !operation.has_canonical_operation_key() {
        return Err(CanwuError::new(
            ErrorCode::InvalidPayload,
            "completion operation identities do not form a valid operation key",
        ));
    }
    Ok(())
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
    // Every lookup page reachable from a chain, queued, or unacknowledged
    // directory is marked. The head and every queued commit need their whole
    // index: a queued commit may become the head. The index of any other
    // directory (an earlier commit, or one settled and waiting for its
    // retention acknowledgement) is marked as far as the store still holds
    // it, so a collection never drops a page that a restore, a replay, or a
    // fork through that commit could read, and a host application that
    // deleted superseded pages is not refused.
    let mut live = roots.clone();
    if let Some(record) = view.typed_domain_record(&resource_runtime_reference())? {
        // The archive head and retention handles are in the live root.
        let state = crate::decode_live_root(record)?;
        if let Some(root) = &state.archive_head.directory_root {
            roots.insert(root.clone());
            live.insert(root.clone());
        }
        roots.extend(
            state
                .archive_retention_handles
                .values()
                .map(|handle| handle.directory_root.clone()),
        );
    }
    let mut lookup_roots = Vec::new();
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
        lookup_roots.push((!live.contains(&root), directory.lookup_root.clone()));
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
    // Complete indexes first: a page they share with another index is then
    // already known to be whole.
    lookup_roots.sort();
    let mut lookup_pages = std::collections::BTreeSet::new();
    for (earlier, lookup_root) in &lookup_roots {
        crate::collect_resource_archive_lookup_pages(
            &provider,
            lookup_root,
            "",
            !earlier,
            &mut lookup_pages,
        )
        .map_err(resource_canwu_error)?;
    }
    for page in lookup_pages {
        manifest.insert_plugin_object(crate::RESOURCE_ARCHIVE_LOOKUP_PAGE_NAMESPACE, page);
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
        registrar.register_record_schema(DomainRecordSchema::for_record::<
            crate::ResourceBucketRecord,
        >())?;
        registrar.register_knowledge_schema(resource_report_knowledge_schema())?;
        registrar.register_event_audience(
            "canwu.resource.operation_settled.v1",
            EventAudience::AffectedActors,
        )?;
        registrar.register_event_audience(
            crate::RESOURCE_INGRESS_SKIPPED_EVENT,
            EventAudience::AffectedActors,
        )?;
        registrar.register_archive_reachability_participant(resource_archive_reachability)?;

        // The administrative domain-record read lets admission resolve an
        // access grant's application authority evidence of any record kind.
        let mut command_reads = vec![
            StateKey::core_domain_records(),
            StateKey::core_evidence(),
            resource_state_key(),
            resource_hot_terminal_state_key(),
        ];
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
        // Its permit is never handed out: only the lifecycle system queues
        // it, to the resource plugin itself.
        registrar.register_internal_ingress(PluginIngressDescriptor {
            name: RESOURCE_DEFERRED_INGRESS.to_owned(),
            description: "Settle plugin-scheduled resource work deferred for want of room"
                .to_owned(),
            class: IngressClass::ScheduledSystem,
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
            resource_hot_terminal_state_key(),
            // External completion participant requests coordinated by the
            // production package bind an exact current production runtime.
            StateKey::new("canwu.production", "runtime"),
        ];
        lifecycle.reads.extend(self.adapter_state_keys());
        lifecycle.reads.sort();
        lifecycle.reads.dedup();
        lifecycle.writes = vec![resource_state_key(), resource_hot_terminal_state_key()];
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
            PluginIngressTarget {
                target_plugin: PLUGIN_NAME.to_owned(),
                packet_type: RESOURCE_DEFERRED_INGRESS.to_owned(),
            },
        ];
        lifecycle.emits = vec![
            "canwu.resource.operation_settled.v1".to_owned(),
            crate::RESOURCE_INGRESS_SKIPPED_EVENT.to_owned(),
        ];
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
        reports.reads = vec![
            StateKey::core_knowledge(),
            resource_state_key(),
            resource_hot_terminal_state_key(),
        ];
        reports.writes = vec![resource_state_key(), resource_hot_terminal_state_key()];
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
    // Both are refused before any key is looked up: a key under the
    // completion prefix belongs to the completion operation that derives it,
    // and a completion operation whose identities form no key cannot settle.
    reject_reserved_operation_key(&value.request)?;
    if let ResourceOperationRequestV1::Completion(operation) = &value.request {
        reject_invalid_completion_identity(operation)?;
    }
    let live = view
        .typed_domain_record(&resource_runtime_reference())?
        .ok_or_else(|| invalid("resource runtime is unavailable"))?;
    // The live root, with the outcome under the command's key and every hot
    // terminal record the command names, and what those name, attached.
    let mut work = crate::ResourceRootWork::new(view, live)?;
    work.attach([crate::ResourceTerminalRecordKeyV1::Outcome(
        value.request.operation_key(),
    )])?;
    work.attach_named(payload)?;
    let admitted =
        archive_reads::recording(|| admit_resource_command_on(view, context, &value, &work.state));
    #[cfg(debug_assertions)]
    {
        let mut whole = crate::ResourceRootWork::new(view, live)?;
        whole.attach_all()?;
        let expected = archive_reads::replaying(|| {
            admit_resource_command_on(view, context, &value, &whole.state)
        });
        assert_eq!(
            admitted, expected,
            "a resource command was admitted differently on the live root than on the whole root"
        );
    }
    admitted
}

fn admit_resource_command_on(
    view: &SimulationView<'_>,
    context: &CommandContext,
    value: &ResourceCommandV1,
    state: &crate::ResourceState,
) -> Result<Vec<SystemDirective>, CanwuError> {
    let request_digest =
        crate::canonical_digest("canwu.resource.operation-request.v1", &value.request)
            .map_err(resource_canwu_error)?;
    if archived_operation_outcome(view, state, &value.request, &request_digest)?.is_some() {
        return Ok(Vec::new());
    }
    validate_resource_authority(view, context, value, state)?;
    validate_tracked_acquisition(view, context, &value.request)?;
    // A key whose outcome is still in the root belongs to the request that
    // settled it. Refusing a different request here records the refusal on
    // the command, as for an archived key. Authority is checked first, so
    // only a holder who may issue the operation learns that its key is taken.
    // A command admitted before the request that takes its key has settled
    // passes this check; its packet is then left unsettled at settlement.
    if state
        .outcomes
        .get(&value.request.operation_key())
        .is_some_and(|outcome| outcome.request_digest != request_digest)
    {
        return Err(CanwuError::new(
            ErrorCode::IdempotencyConflict,
            "resource operation key was reused with a different request",
        ));
    }
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

/// What the lifecycle system settles in one boundary: whether the root
/// changed, the holders its settlement event names, and the directives it
/// proposes, skip events and deferred work included.
#[derive(Default)]
struct LifecycleSettlement {
    changed: bool,
    affected: Vec<EntityRef>,
    directives: Vec<BoundaryDirective>,
    /// Whether this boundary already schedules a completion expiry tick.
    expiry_tick: bool,
    /// Consumption batch items this boundary has settled.
    batch_items: usize,
}

const OPERATION_KEY_TAKEN_DETAIL: &str =
    "another request holds the operation key; the outcome under the key is that request's";
const RESERVED_OPERATION_KEY_DETAIL: &str = "operation keys under `resource:completion:` are reserved for completion operations, and keys under `resource:consume-batch:` for consumption batch items";

/// Most doublings of the delay before deferred work is settled again: the
/// delay grows from one minute to 1,024 minutes and stays there.
const MAX_DEFERRAL_BACKOFF_DOUBLINGS: u32 = 10;

/// Where an admitted resource packet came from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PacketOrigin {
    /// The host application or a tracked command. A packet that does not fit
    /// the root is skipped, and its sender sends it again.
    Host,
    /// A plugin's boundary system, which may never send it again. A packet
    /// that does not fit the root is deferred instead; `deferrals` counts the
    /// times the resource plugin has deferred it so far.
    Plugin { deferrals: u32 },
}

impl PacketOrigin {
    /// A packet whose cause is a boundary was scheduled by a plugin: the
    /// kernel refuses that cause on ingress a host application queues.
    fn of(ingress: &IngressRecord) -> Self {
        match ingress.cause {
            Some(CauseRef::Boundary(_)) => Self::Plugin { deferrals: 0 },
            _ => Self::Host,
        }
    }
}

/// Work another plugin scheduled that the resource plugin deferred because
/// it did not fit the root, as the packet it queues to itself.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct ResourceDeferredPacketV1 {
    /// [`RESOURCE_PRODUCTION_OUTPUT_BATCH_INGRESS`],
    /// [`RESOURCE_COMPLETION_INGRESS`], or, for a consumption batch past the
    /// boundary's batch budget, [`RESOURCE_ADAPTER_INGRESS`].
    packet_type: String,
    payload: Value,
    /// The times the work has been deferred, this deferral included.
    deferrals: u32,
}

/// Who a skip event names.
#[derive(Clone, Copy)]
enum EventHolder<'a> {
    /// Nobody.
    Nobody,
    /// A holder the run admitted: a tracked command's subject, which the
    /// kernel admitted with the command, or an allocation pass's requester,
    /// which `enqueue_resource_allocation` checks. The event names it as its
    /// affected entity when it is a registered one ([`registered_holder`]).
    Admitted(&'a canwu_api::KnowledgeHolderRef),
    /// The holder of an external participant, which the coordinator plugin
    /// vouched for but the run never checked against its entities. An event
    /// naming an entity the run does not hold fails its boundary, and a
    /// boundary system cannot ask the registry, so the holder is named in
    /// the event's detail only.
    Participant(&'a canwu_api::KnowledgeHolderRef),
}

/// The entity a resource event may name for `holder`: one of the engine's
/// entity registry, which never loses an identity. An event that names a
/// missing entity fails its boundary, and the entity of a domain record may
/// be gone by the time a packet settles, so such a holder is not named.
fn registered_holder(holder: &canwu_api::KnowledgeHolderRef) -> Option<EntityRef> {
    let entity = holder_entity(holder);
    (!matches!(entity, EntityRef::Domain(_))).then_some(entity)
}

/// The reason code and detail of a skip or deferral for want of room.
const fn shortage_reason(
    shortage: crate::RoomShortage,
) -> (crate::ResourceIngressSkipReasonV1, &'static str) {
    match shortage {
        crate::RoomShortage::ArchiveBacklog => (
            crate::ResourceIngressSkipReasonV1::ArchiveBacklog,
            "the result does not fit the archive budgets of the root; an archive commit of the hot candidates makes room",
        ),
        crate::RoomShortage::ReservedCapacity { permanent: false } => (
            crate::ResourceIngressSkipReasonV1::ReservedCapacity,
            "the result does not fit beside the room admitted completion leases, or finished participants whose reports wait, reserve; an archive commit alone does not make room",
        ),
        crate::RoomShortage::ReservedCapacity { permanent: true } => (
            crate::ResourceIngressSkipReasonV1::ReservedCapacity,
            "the result does not fit this budget even beside an empty backlog; no archive commit or lease release makes room for it",
        ),
        crate::RoomShortage::RootLimit => (
            crate::ResourceIngressSkipReasonV1::RootLimit,
            "the result passes a limit of the root",
        ),
    }
}

/// Whether plugin-scheduled work that found no room is worth waiting for.
///
/// An archive backlog is cured by an archive commit, and room reserved by
/// leases by their completion, release, or expiry, or, for the slots of
/// finished participants, by the publication of their reports and an
/// archive commit: such work waits as long as it takes. Room no commit or
/// release can make never comes, so that work is skipped at once. A root limit, in practice the byte cap, may or may not
/// pass, so the work waits until its delay has grown to the 1,024-minute cap
/// and is skipped when it still finds no room after that wait.
const fn deferral_is_worthwhile(shortage: crate::RoomShortage, deferrals: u32) -> bool {
    match shortage {
        crate::RoomShortage::ArchiveBacklog => true,
        crate::RoomShortage::ReservedCapacity { permanent } => !permanent,
        crate::RoomShortage::RootLimit => deferrals <= MAX_DEFERRAL_BACKOFF_DOUBLINGS,
    }
}

/// Whether a completion step can settle at a later instant than the one it
/// was scheduled for. A step that names its boundary time or boundary, or
/// starts a lease or a grant at it (an acquisition, a grant, an activation,
/// a consumption, an expiry, a single step), would fail its time checks
/// later, so it is never deferred.
const fn completion_step_can_wait(operation: &crate::ResourceCompletionOperationV1) -> bool {
    matches!(
        operation,
        crate::ResourceCompletionOperationV1::Prepare(_)
            | crate::ResourceCompletionOperationV1::Abort(_)
            | crate::ResourceCompletionOperationV1::Release(_)
            | crate::ResourceCompletionOperationV1::PrepareExternalParticipant(_)
            | crate::ResourceCompletionOperationV1::CompleteExternalParticipant(_)
            | crate::ResourceCompletionOperationV1::ReleaseExternalParticipant(_)
    )
}

/// Whether a completion or external participant expiry is due by `boundary`.
fn expiry_overdue(state: &crate::ResourceState, boundary: u64) -> bool {
    state
        .completion_leases
        .expiry_due
        .range(..=boundary)
        .any(|(_, values)| !values.is_empty())
        || state
            .external_completion_participants
            .expiry_due
            .range(..=boundary)
            .any(|(_, values)| !values.is_empty())
}

/// The holder of the external participant a completion step belongs to, as
/// the root records it. The coordinator plugin vouched for that holder when
/// the participant was granted.
fn external_participant_holder(
    state: &crate::ResourceState,
    operation: &crate::ResourceCompletionOperationV1,
) -> Option<canwu_api::KnowledgeHolderRef> {
    let acquisition = match operation {
        crate::ResourceCompletionOperationV1::PrepareExternalParticipant(request) => {
            &request.acquisition
        }
        crate::ResourceCompletionOperationV1::ConsumeExternalParticipant(request) => {
            &request.certificate.acquisition
        }
        crate::ResourceCompletionOperationV1::CompleteExternalParticipant(request) => {
            &request.acquisition
        }
        crate::ResourceCompletionOperationV1::ReleaseExternalParticipant(request) => {
            &request.acquisition
        }
        _ => return None,
    };
    state
        .external_completion_participants
        .participant(acquisition)
        .map(|participant| participant.holder.clone())
}

impl LifecycleSettlement {
    /// Records that the packet `ingress` settled nothing, in one
    /// [`crate::RESOURCE_INGRESS_SKIPPED_EVENT`] that names `holder` as
    /// [`EventHolder`] says.
    fn skip(
        &mut self,
        ingress: IngressId,
        reason: crate::ResourceIngressSkipReasonV1,
        operation_key: Option<&crate::ResourceOperationKey>,
        detail: &str,
        holder: EventHolder<'_>,
    ) {
        self.emit_skip(
            crate::ResourceIngressSkipV1 {
                ingress,
                reason,
                operation_key: operation_key.cloned(),
                deferred_until: None,
            },
            detail,
            holder,
        );
    }

    fn emit_skip(
        &mut self,
        skip: crate::ResourceIngressSkipV1,
        detail: &str,
        holder: EventHolder<'_>,
    ) {
        let (affected, detail) = match holder {
            EventHolder::Nobody => (Vec::new(), detail.to_owned()),
            EventHolder::Admitted(holder) => (
                registered_holder(holder).into_iter().collect(),
                detail.to_owned(),
            ),
            EventHolder::Participant(holder) => (
                Vec::new(),
                format!(
                    "{detail}; the work belongs to holder {}",
                    holder_entity(holder)
                ),
            ),
        };
        self.directives.push(BoundaryDirective::Emit {
            event_type: crate::RESOURCE_INGRESS_SKIPPED_EVENT.to_owned(),
            summary: skip.summary(&detail),
            affected,
        });
    }

    /// Records that the packet `ingress` settled nothing because it does not
    /// decode as a `label`. The detail is fixed: the decoder's message could
    /// repeat text the sender supplied.
    fn skip_undecodable(&mut self, ingress: IngressId, label: &str) {
        self.skip(
            ingress,
            crate::ResourceIngressSkipReasonV1::FailedSettlementChecks,
            None,
            &format!("the packet does not decode as a {label}"),
            EventHolder::Nobody,
        );
    }

    /// Records that the packet `ingress` settled nothing because its result
    /// does not fit the root.
    fn skip_for_room(
        &mut self,
        ingress: IngressId,
        shortage: crate::RoomShortage,
        operation_key: Option<&crate::ResourceOperationKey>,
        holder: EventHolder<'_>,
    ) {
        let (reason, detail) = shortage_reason(shortage);
        self.skip(ingress, reason, operation_key, detail, holder);
    }

    /// Settles nothing of plugin-scheduled work that does not fit the root,
    /// and queues it to the resource plugin itself for a later instant, as a
    /// [`RESOURCE_DEFERRED_INGRESS`] packet. The plugin that scheduled the
    /// work may never send it again, so it is not handed to the host
    /// application. The callers defer only work that can settle later and
    /// whose shortage can pass ([`deferral_is_worthwhile`]).
    ///
    /// The delay is one minute after the first deferral and doubles with each
    /// further one, up to 1,024 minutes: never the same instant, which would
    /// meet the same root without end, and one small packet per retry however
    /// long the shortage lasts. A production output batch is retimed to the
    /// instant it is due: each credit's `at` becomes that instant. The event
    /// names the instant and `holder`.
    fn defer(
        &mut self,
        ingress: IngressId,
        at: SimTime,
        shortage: crate::RoomShortage,
        work: DeferredWork<'_>,
        holder: EventHolder<'_>,
    ) -> Result<(), CanwuError> {
        let delay = canwu_api::SimDuration::minutes(
            1_i64 << work.deferrals.min(MAX_DEFERRAL_BACKOFF_DOUBLINGS),
        );
        let Some(due) = at.checked_add(delay) else {
            self.skip_for_room(ingress, shortage, work.operation_key, holder);
            return Ok(());
        };
        let payload = match work.packet {
            DeferredPacket::OutputBatch(batch) => {
                let mut batch = batch.clone();
                for request in &mut batch.requests {
                    request.at = due;
                }
                (RESOURCE_PRODUCTION_OUTPUT_BATCH_INGRESS, encode(&batch)?)
            }
            DeferredPacket::Completion(operation) => {
                (RESOURCE_COMPLETION_INGRESS, encode(operation)?)
            }
        };
        self.directives
            .push(BoundaryDirective::SchedulePluginIngress {
                target_plugin: PLUGIN_NAME.to_owned(),
                after: delay,
                packet_type: RESOURCE_DEFERRED_INGRESS.to_owned(),
                priority: 0,
                payload: encode(&ResourceDeferredPacketV1 {
                    packet_type: payload.0.to_owned(),
                    payload: payload.1,
                    deferrals: work.deferrals.saturating_add(1),
                })?,
                affected: Vec::new(),
            });
        let (reason, detail) = shortage_reason(shortage);
        self.emit_skip(
            crate::ResourceIngressSkipV1 {
                ingress,
                reason,
                operation_key: work.operation_key.cloned(),
                deferred_until: Some(due),
            },
            detail,
            holder,
        );
        Ok(())
    }

    /// Settles nothing of a consumption batch the boundary has no batch
    /// budget left for, and queues it to the resource plugin itself for the
    /// next boundary at this instant, as a [`RESOURCE_DEFERRED_INGRESS`]
    /// packet with no delay: the batch's time and certificate bind it to this
    /// instant, and the kernel settles no later boundary before it. That
    /// boundary settles it, since a boundary always settles a batch that fits
    /// the budget alone, so the deferral never repeats without end. The event
    /// names the instant.
    fn defer_to_next_boundary(
        &mut self,
        ingress: IngressId,
        at: SimTime,
        packet: &ResourceAdapterOperationV1,
        deferrals: u32,
        operation_key: &crate::ResourceOperationKey,
    ) -> Result<(), CanwuError> {
        self.directives
            .push(BoundaryDirective::SchedulePluginIngress {
                target_plugin: PLUGIN_NAME.to_owned(),
                after: canwu_api::SimDuration::ZERO,
                packet_type: RESOURCE_DEFERRED_INGRESS.to_owned(),
                priority: 0,
                payload: encode(&ResourceDeferredPacketV1 {
                    packet_type: RESOURCE_ADAPTER_INGRESS.to_owned(),
                    payload: encode(packet)?,
                    deferrals: deferrals.saturating_add(1),
                })?,
                affected: Vec::new(),
            });
        self.emit_skip(
            crate::ResourceIngressSkipV1 {
                ingress,
                reason: crate::ResourceIngressSkipReasonV1::BoundaryBudget,
                operation_key: Some(operation_key.clone()),
                deferred_until: Some(at),
            },
            "the boundary has settled as many consumption batch items as its budget allows; the batch settles in the next boundary at this instant",
            EventHolder::Nobody,
        );
        Ok(())
    }

    /// Schedules one completion expiry tick in this boundary, however many
    /// settlements ask for one.
    fn schedule_expiry_tick(&mut self) {
        if !self.expiry_tick {
            self.expiry_tick = true;
            self.directives.push(completion_expiry_tick_directive());
        }
    }

    /// Whether `request`'s operation key is free for it. A key whose outcome
    /// is this request's settles nothing and leaves no event: the outcome is
    /// the resend's own. A key another request holds settles nothing with a
    /// skip event.
    fn operation_key_is_free(
        &mut self,
        view: &SimulationView<'_>,
        state: &crate::ResourceState,
        request: &ResourceOperationRequestV1,
        request_digest: &str,
        ingress: IngressId,
        holder: EventHolder<'_>,
    ) -> Result<bool, CanwuError> {
        Ok(
            match operation_key_holder(view, state, request, request_digest)? {
                OperationKeyHolder::Nobody => true,
                OperationKeyHolder::ThisRequestInRoot
                | OperationKeyHolder::ThisRequestInArchive => false,
                OperationKeyHolder::AnotherRequest => {
                    self.skip(
                        ingress,
                        crate::ResourceIngressSkipReasonV1::OperationKeyTaken,
                        Some(&request.operation_key()),
                        OPERATION_KEY_TAKEN_DETAIL,
                        holder,
                    );
                    false
                }
            },
        )
    }
}

/// Plugin-scheduled work to defer.
struct DeferredWork<'a> {
    packet: DeferredPacket<'a>,
    /// The times the work has been deferred before this deferral.
    deferrals: u32,
    operation_key: Option<&'a crate::ResourceOperationKey>,
}

enum DeferredPacket<'a> {
    OutputBatch(&'a ResourceProductionOutputBatchV1),
    Completion(&'a crate::ResourceCompletionOperationV1),
}

/// Settles the resource packets one boundary admitted, archive maintenance
/// first.
///
/// No packet a host application or a holder can send fails the boundary. A
/// packet that cannot settle for an expected reason (its key is taken or
/// reserved, it fails its settlement checks, or its result does not fit the
/// root) changes nothing and leaves a [`crate::RESOURCE_INGRESS_SKIPPED_EVENT`].
/// Work another plugin scheduled that does not fit the root is deferred to a
/// later instant instead of skipped. What still fails the boundary is an
/// invalid record or an engine invariant: an archive object that is missing,
/// forged, or unreadable during a key lookup, a forged archive commit or
/// retention acknowledgement, an invalid root after settlement, and
/// arithmetic overflow.
///
/// A boundary that admits no resource packet reads nothing of the root. One
/// that does reads the live root and the hot terminal records its packets
/// name ([`crate::ResourceRootWork::attach_named`]), and writes the live root
/// and the buckets whose records changed.
fn settle_resource_lifecycle(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let admitted = resource_packets_maintenance_first(view, &context.admitted_ingress)?;
    if admitted.is_empty() {
        return Ok(BoundaryProposal::default());
    }
    let Some(live) = view.typed_domain_record(&resource_runtime_reference())? else {
        return Ok(BoundaryProposal::default());
    };
    let mut work = crate::ResourceRootWork::new(view, live)?;
    let proposal =
        archive_reads::recording(|| settle_lifecycle_on(view, context, &admitted, &mut work))?;
    // Every debug build settles the same packets on the whole root as well
    // and requires the same proposal, so the test suites check that the
    // records a packet reads are attached before it settles.
    #[cfg(debug_assertions)]
    {
        let mut whole = crate::ResourceRootWork::new(view, live)?;
        whole.attach_all()?;
        let expected =
            archive_reads::replaying(|| settle_lifecycle_on(view, context, &admitted, &mut whole))?;
        assert_eq!(
            proposal, expected,
            "the resource lifecycle settled differently on the live root than on the whole root"
        );
    }
    Ok(proposal)
}

/// The live demands an allocation pass of `packet` may read: the requester's
/// demands that are due or dirty by its time, or that expire by it.
fn allocation_demands(
    state: &crate::ResourceState,
    packet: &ResourceAuthorizedAllocationV1,
) -> std::collections::BTreeSet<crate::ResourceDemandId> {
    let at = packet.request.at;
    state
        .demand_due_index
        .range(..=at)
        .chain(state.demand_expiry_index.range(..=at))
        .flat_map(|(_, ids)| ids.iter())
        .chain(state.dirty_demands.iter())
        .filter(|id| {
            state
                .demands
                .get(*id)
                .is_some_and(|demand| demand.requester == packet.requester)
        })
        .cloned()
        .collect()
}

/// The operation keys a resource packet of `packet_type` settles under, as
/// far as its payload decodes; a packet that does not decode settles none.
fn packet_operation_keys(packet_type: &str, payload: &Value) -> Vec<crate::ResourceOperationKey> {
    fn decoded<T: serde::de::DeserializeOwned>(payload: &Value) -> Option<T> {
        T::deserialize(payload).ok()
    }
    // A lease start also reads the key the other way of starting a lease
    // takes for its identity.
    let with_sibling = |request: &ResourceOperationRequestV1| {
        let mut keys = vec![request.operation_key()];
        if let ResourceOperationRequestV1::Completion(operation) = request {
            keys.extend(operation.sibling_lease_operation_key());
        }
        keys
    };
    match packet_type {
        RESOURCE_COMMAND_INGRESS => decoded::<AdmittedResourceCommandV1>(payload)
            .map(|admitted| with_sibling(&admitted.value.request))
            .unwrap_or_default(),
        // A consumption batch also settles under each item's reserved key.
        RESOURCE_ADAPTER_INGRESS => decoded::<ResourceAdapterOperationV1>(payload)
            .map(|packet| {
                let mut keys = vec![packet.request.operation_key()];
                if let ResourceOperationRequestV1::ConsumeBatch(batch) = &packet.request {
                    keys.extend(
                        (0..batch.items.len()).map(|index| batch.item_operation_key(index)),
                    );
                }
                keys
            })
            .unwrap_or_default(),
        RESOURCE_PRODUCTION_OUTPUT_BATCH_INGRESS => {
            decoded::<ResourceProductionOutputBatchV1>(payload)
                .map(|batch| {
                    batch
                        .requests
                        .into_iter()
                        .map(|request| request.operation_key)
                        .collect()
                })
                .unwrap_or_default()
        }
        RESOURCE_ALLOCATION_INGRESS => decoded::<ResourceAuthorizedAllocationV1>(payload)
            .map(|packet| vec![packet.request.operation_key])
            .unwrap_or_default(),
        RESOURCE_COMPLETION_INGRESS => decoded::<crate::ResourceCompletionOperationV1>(payload)
            .map(|operation| with_sibling(&ResourceOperationRequestV1::Completion(operation)))
            .unwrap_or_default(),
        RESOURCE_DEFERRED_INGRESS => decoded::<ResourceDeferredPacketV1>(payload)
            .map(|deferred| packet_operation_keys(&deferred.packet_type, &deferred.payload))
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Settles `admitted` on `work` and proposes what changed.
fn settle_lifecycle_on<S: crate::HotTerminalSource + ?Sized>(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    admitted: &[IngressId],
    work: &mut crate::ResourceRootWork<'_, S>,
) -> Result<BoundaryProposal, CanwuError> {
    let mut settlement = LifecycleSettlement::default();
    for ingress_id in admitted {
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
        // Every hot terminal record the packet names, and what those name,
        // is attached before anything reads the root for it: the outcomes
        // under the operation keys it settles, which a completion operation
        // derives from its identities and an expiry tick from the boundary,
        // and every record its payload names by identity. An archive commit
        // reads the records it selects only when it applies, and archive
        // maintenance names receipts by sequence.
        work.attach(
            packet_operation_keys(packet_type, payload)
                .into_iter()
                .chain(
                    (packet_type == RESOURCE_COMPLETION_EXPIRY_TICK_INGRESS)
                        .then(|| expiry_tick_operation_keys(context))
                        .into_iter()
                        .flatten(),
                )
                .map(crate::ResourceTerminalRecordKeyV1::Outcome),
        )?;
        if packet_type != RESOURCE_ARCHIVE_COMMIT_INGRESS {
            work.attach_named(payload)?;
        }
        match packet_type.as_str() {
            // An archive commit settles on the work: the records it selects
            // leave their buckets without being decoded.
            RESOURCE_ARCHIVE_COMMIT_INGRESS => {
                let commit: crate::VerifiedResourceArchiveCommitV1 =
                    decode(payload, "verified resource archive commit")?;
                let receipt = work.apply_archive_commit(&commit)?;
                // An expiry skipped for want of room resumes once a commit
                // makes room. A skipped expiry schedules no successor, so
                // this cannot spin.
                if receipt.disposition == crate::ResourceArchiveMaintenanceDispositionV1::Applied
                    && expiry_overdue(&work.state, context.boundary_id.get())
                {
                    settlement.schedule_expiry_tick();
                }
                settlement.changed = true;
                continue;
            }
            // An allocation pass reads the source accounts and grants of the
            // requester's demands it may settle, and every account when one
            // of them draws on the pool.
            RESOURCE_ALLOCATION_INGRESS => {
                if let Ok(packet) = ResourceAuthorizedAllocationV1::deserialize(payload) {
                    let demands = allocation_demands(&work.state, &packet);
                    work.attach_demand_sources(&demands)?;
                }
            }
            // An account's creation adds it to its custodian's index. Only
            // a consumption batch is ever deferred, so a creation is always
            // one of these packets, never a deferred one.
            RESOURCE_COMMAND_INGRESS | RESOURCE_ADAPTER_INGRESS => {
                let request = if packet_type == RESOURCE_COMMAND_INGRESS {
                    AdmittedResourceCommandV1::deserialize(payload)
                        .ok()
                        .map(|admitted| admitted.value.request)
                } else {
                    ResourceAdapterOperationV1::deserialize(payload)
                        .ok()
                        .map(|packet| packet.request)
                };
                if let Some(ResourceOperationRequestV1::CreateAccount(request)) = request {
                    work.attach_live([crate::ResourceLiveRecordKeyV1::Custodian(
                        request.account.custodian,
                    )])?;
                }
            }
            RESOURCE_ARCHIVE_RETENTION_ACK_INGRESS => {
                if let Ok(ResourceArchiveRetentionAcknowledgementV1::Receipt(receipt)) =
                    ResourceArchiveRetentionAcknowledgementV1::deserialize(payload)
                {
                    work.attach([
                        crate::ResourceTerminalRecordKeyV1::ArchiveMaintenanceReceipt(
                            receipt.sequence,
                        ),
                    ])?;
                }
            }
            _ => {}
        }
        let state = &mut work.state;
        let origin = PacketOrigin::of(ingress);
        match packet_type.as_str() {
            RESOURCE_COMMAND_INGRESS => {
                settle_command_packet(view, context, ingress, payload, state, &mut settlement)?;
            }
            RESOURCE_ADAPTER_INGRESS => {
                settle_adapter_packet(
                    view,
                    context,
                    Admitted {
                        ingress,
                        payload,
                        origin,
                    },
                    state,
                    &mut settlement,
                )?;
            }
            RESOURCE_PRODUCTION_OUTPUT_BATCH_INGRESS => settle_output_batch_packet(
                view,
                context,
                Admitted {
                    ingress,
                    payload,
                    origin,
                },
                state,
                &mut settlement,
            )?,
            RESOURCE_ALLOCATION_INGRESS => {
                settle_allocation_packet(view, context, ingress, payload, state, &mut settlement)?;
            }
            RESOURCE_COMPLETION_INGRESS => settle_completion_packet(
                view,
                context,
                Admitted {
                    ingress,
                    payload,
                    origin,
                },
                state,
                &mut settlement,
            )?,
            RESOURCE_DEFERRED_INGRESS => {
                settle_deferred_packet(view, context, ingress, payload, state, &mut settlement)?;
            }
            RESOURCE_COMPLETION_EXPIRY_TICK_INGRESS => {
                settle_expiry_tick(view, context, ingress.id, state, &mut settlement)?;
            }
            RESOURCE_ARCHIVE_RETENTION_ACK_INGRESS => {
                let acknowledgement: ResourceArchiveRetentionAcknowledgementV1 =
                    decode(payload, "resource archive retention acknowledgement")?;
                match &acknowledgement {
                    ResourceArchiveRetentionAcknowledgementV1::Receipt(receipt) => {
                        state.acknowledge_archive_retention(receipt)
                    }
                    ResourceArchiveRetentionAcknowledgementV1::Handle {
                        retention_handle_id,
                        directory_root,
                    } => state
                        .acknowledge_archive_retention_handle(retention_handle_id, directory_root),
                }
                .map_err(|error| CanwuError::new(ErrorCode::InvalidArchive, error.to_string()))?;
                settlement.changed = true;
            }
            _ => {}
        }
    }
    let LifecycleSettlement {
        changed,
        affected,
        mut directives,
        expiry_tick: _,
        batch_items: _,
    } = settlement;
    if !changed {
        return Ok(BoundaryProposal {
            directives,
            ..BoundaryProposal::default()
        });
    }
    work.state.state_revision =
        crate::ResourceRevision::new(work.live_version.checked_add(1).ok_or_else(|| {
            CanwuError::new(ErrorCode::ValueOutOfRange, "resource version overflow")
        })?)
        .map_err(resource_canwu_error)?;
    work.state.validate().map_err(resource_canwu_error)?;
    let mut writes = work.persist("Settle resource lifecycle ingress")?;
    writes.append(&mut directives);
    let mut directives = writes;
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

/// One admitted packet whose origin decides what happens when it does not
/// fit the root.
#[derive(Clone, Copy)]
struct Admitted<'a> {
    ingress: &'a IngressRecord,
    payload: &'a Value,
    origin: PacketOrigin,
}

/// Settles one tracked command.
///
/// It settles nothing, with a skip event, when the packet does not match its
/// command, its key is reserved or held by another request, a completion
/// command fails the checks the completion ingress makes, or its result does
/// not fit the root. A command whose outcome is already recorded settles
/// nothing and leaves no event.
fn settle_command_packet(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    ingress: &IngressRecord,
    payload: &Value,
    state: &mut crate::ResourceState,
    out: &mut LifecycleSettlement,
) -> Result<(), CanwuError> {
    let Ok(admitted) = decode::<AdmittedResourceCommandV1>(payload, "admitted resource command")
    else {
        out.skip_undecodable(ingress.id, "resource command packet");
        return Ok(());
    };
    let request = &admitted.value.request;
    let operation_key = request.operation_key();
    // A packet that does not match its tracked command names no holder: its
    // subject was never admitted. The check reads the command's retained
    // evidence, which a seal may have removed for a command the packet only
    // claims, so the event carries one fixed detail whatever failed.
    if validate_admitted_command(view, ingress.cause.as_ref(), &admitted).is_err() {
        out.skip(
            ingress.id,
            crate::ResourceIngressSkipReasonV1::FailedSettlementChecks,
            Some(&operation_key),
            "the packet does not match an admitted tracked resource command",
            EventHolder::Nobody,
        );
        return Ok(());
    }
    let subject = EventHolder::Admitted(&admitted.value.subject);
    if request.names_reserved_operation_key() {
        out.skip(
            ingress.id,
            crate::ResourceIngressSkipReasonV1::ReservedOperationKey,
            Some(&operation_key),
            RESERVED_OPERATION_KEY_DETAIL,
            subject,
        );
        return Ok(());
    }
    let request_digest = request.request_digest().map_err(resource_canwu_error)?;
    // An operation key belongs to the first request that settles under it.
    if !out.operation_key_is_free(view, state, request, &request_digest, ingress.id, subject)? {
        return Ok(());
    }
    // A tracked completion command passes the checks the completion ingress
    // makes of the same operation: an acquisition's eligibility time, its
    // report reservation, and the currency of the evidence it persists.
    if let ResourceOperationRequestV1::Completion(operation) = request
        && let Err(error) = validate_completion_ingress(view, context, operation, state)
    {
        out.skip(
            ingress.id,
            crate::ResourceIngressSkipReasonV1::FailedSettlementChecks,
            Some(&operation_key),
            &error.message,
            subject,
        );
        return Ok(());
    }
    let refusal = match request {
        ResourceOperationRequestV1::Completion(operation) => {
            lease_identity_refusal(view, state, operation)?
        }
        _ => None,
    };
    match state
        .settle_operation_refused_by(request, Some(context.at), refusal.as_ref())
        .map_err(resource_canwu_error)?
    {
        crate::OperationSettlement::Settled(_) => {
            out.changed = true;
            out.affected
                .extend(registered_holder(&admitted.value.subject));
        }
        crate::OperationSettlement::NoRoom(shortage) => {
            out.skip_for_room(ingress.id, shortage, Some(&operation_key), subject);
        }
    }
    Ok(())
}

/// Settles one adapter packet: the same skips as a tracked command, with the
/// provider binding as its settlement check.
///
/// A plugin that schedules adapter packets, as the force-supply reference
/// does, dispatches again every consumption that has no outcome, so a
/// packet skipped for room is sent again and is never deferred: a deferred
/// copy would settle beside the one sent again. The one deferral is of a
/// consumption batch past the boundary's batch budget, to the next boundary
/// at the same instant ([`LifecycleSettlement::defer_to_next_boundary`]); a
/// copy sent again beside it is a resend of the same request.
fn settle_adapter_packet(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    admitted: Admitted<'_>,
    state: &mut crate::ResourceState,
    out: &mut LifecycleSettlement,
) -> Result<(), CanwuError> {
    let ingress = admitted.ingress;
    let Ok(packet) =
        decode::<ResourceAdapterOperationV1>(admitted.payload, "resource adapter operation")
    else {
        out.skip_undecodable(ingress.id, "resource adapter operation");
        return Ok(());
    };
    let operation_key = packet.request.operation_key();
    // `resource_adapter_ingress` refuses such a packet. One queued around it
    // settles nothing and takes no key.
    if packet.request.names_reserved_operation_key() {
        out.skip(
            ingress.id,
            crate::ResourceIngressSkipReasonV1::ReservedOperationKey,
            Some(&operation_key),
            RESERVED_OPERATION_KEY_DETAIL,
            EventHolder::Nobody,
        );
        return Ok(());
    }
    let request_digest = packet
        .request
        .request_digest()
        .map_err(resource_canwu_error)?;
    if !out.operation_key_is_free(
        view,
        state,
        &packet.request,
        &request_digest,
        ingress.id,
        EventHolder::Nobody,
    )? {
        return Ok(());
    }
    if let Err(error) = validate_adapter_packet(view, context.at, &packet, state) {
        out.skip(
            ingress.id,
            crate::ResourceIngressSkipReasonV1::FailedSettlementChecks,
            Some(&operation_key),
            &error.message,
            EventHolder::Nobody,
        );
        return Ok(());
    }
    // A boundary settles at most `max_batch_items_per_boundary` batch items.
    // A batch that fits that budget alone but not beside the batches the
    // boundary applied waits for the next boundary at this instant, where it
    // settles: a boundary always settles the first batch that fits alone,
    // and the batch's time and certificate bind it to this instant. One that
    // never fits is the state's recorded refusal.
    let batch_items = match &packet.request {
        ResourceOperationRequestV1::ConsumeBatch(batch) => batch.items.len(),
        _ => 0,
    };
    if batch_items > 0
        && batch_items <= state.limits.max_batch_items_per_boundary
        && out.batch_items.saturating_add(batch_items) > state.limits.max_batch_items_per_boundary
    {
        let deferrals = match admitted.origin {
            PacketOrigin::Plugin { deferrals } => deferrals,
            PacketOrigin::Host => 0,
        };
        return out.defer_to_next_boundary(
            ingress.id,
            context.at,
            &packet,
            deferrals,
            &operation_key,
        );
    }
    match state
        .settle_operation_at(&packet.request, context.at)
        .map_err(resource_canwu_error)?
    {
        crate::OperationSettlement::Settled(outcome) => {
            // A batch refused whole consumed nothing of the budget.
            if outcome.status == crate::ResourceOperationStatus::Applied {
                out.batch_items = out.batch_items.saturating_add(batch_items);
            }
            out.changed = true;
        }
        crate::OperationSettlement::NoRoom(shortage) => {
            out.skip_for_room(
                ingress.id,
                shortage,
                Some(&operation_key),
                EventHolder::Nobody,
            );
        }
    }
    Ok(())
}

/// Settles one production output batch, every credit or none.
///
/// The batch settles nothing, with a skip event, when a credit names a
/// reserved key or a key another request holds, the batch fails its
/// settlement checks, or a credit is refused. Room is measured for the whole
/// batch against the root before it. When the batch does not fit, one the
/// production plugin scheduled is deferred to a later instant if the shortage
/// can pass ([`deferral_is_worthwhile`]) and skipped otherwise, and one the
/// host application queued is skipped. A batch whose credits are all
/// recorded for it settled before and settles nothing again; one with only
/// some of them recorded is skipped. The events name the holder of the
/// batch's external participant in their detail. The acknowledgement to the
/// production plugin carries `settled_at`, the instant the credits settled.
fn settle_output_batch_packet(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    admitted: Admitted<'_>,
    state: &mut crate::ResourceState,
    out: &mut LifecycleSettlement,
) -> Result<(), CanwuError> {
    let ingress = admitted.ingress;
    let Ok(packet) =
        decode::<ResourceProductionOutputBatchV1>(admitted.payload, "production output batch")
    else {
        out.skip_undecodable(ingress.id, "production output batch");
        return Ok(());
    };
    let first_key = packet
        .requests
        .first()
        .map(|request| &request.operation_key);
    let holder = packet.requests.first().and_then(|request| {
        state
            .external_completion_participants
            .participant(&request.completion_certificate.acquisition)
            .map(|participant| participant.holder.clone())
    });
    let holder = holder
        .as_ref()
        .map_or(EventHolder::Nobody, EventHolder::Participant);
    // A credit may not take a completion operation's key.
    if let Some(reserved) = packet
        .requests
        .iter()
        .find(|request| crate::is_reserved_operation_key(request.operation_key.as_str()))
    {
        out.skip(
            ingress.id,
            crate::ResourceIngressSkipReasonV1::ReservedOperationKey,
            Some(&reserved.operation_key),
            RESERVED_OPERATION_KEY_DETAIL,
            holder,
        );
        return Ok(());
    }
    if packet.requests.len() > crate::MAX_PRODUCTION_OUTPUT_BATCH_CREDITS {
        out.skip(
            ingress.id,
            crate::ResourceIngressSkipReasonV1::FailedSettlementChecks,
            first_key,
            "production output batch carries more than 64 credits",
            holder,
        );
        return Ok(());
    }
    // The credits of a batch settle together, so all of them are recorded
    // for this request, which is a resend, or none of them is.
    let mut recorded = 0_usize;
    for request in &packet.requests {
        let credit = ResourceOperationRequestV1::Credit(request.clone());
        let request_digest = credit.request_digest().map_err(resource_canwu_error)?;
        match operation_key_holder(view, state, &credit, &request_digest)? {
            OperationKeyHolder::Nobody => {}
            OperationKeyHolder::ThisRequestInRoot | OperationKeyHolder::ThisRequestInArchive => {
                recorded += 1;
            }
            OperationKeyHolder::AnotherRequest => {
                out.skip(
                    ingress.id,
                    crate::ResourceIngressSkipReasonV1::OperationKeyTaken,
                    Some(&request.operation_key),
                    OPERATION_KEY_TAKEN_DETAIL,
                    holder,
                );
                return Ok(());
            }
        }
    }
    if recorded == packet.requests.len() && recorded > 0 {
        return Ok(());
    }
    if recorded > 0 {
        out.skip(
            ingress.id,
            crate::ResourceIngressSkipReasonV1::FailedSettlementChecks,
            first_key,
            "only some credits of the production output batch are recorded for it",
            holder,
        );
        return Ok(());
    }
    let execution = match validate_production_output_batch(view, context.at, &packet, state) {
        Ok(execution) => execution,
        Err(error) => {
            out.skip(
                ingress.id,
                crate::ResourceIngressSkipReasonV1::FailedSettlementChecks,
                first_key,
                &error.message,
                holder,
            );
            return Ok(());
        }
    };
    match state
        .settle_production_output_batch(&packet.requests)
        .map_err(resource_canwu_error)?
    {
        crate::OutputBatchSettlement::Settled(outcomes) => {
            out.directives
                .push(BoundaryDirective::SchedulePluginIngress {
                    target_plugin: "canwu-production".to_owned(),
                    after: canwu_api::SimDuration::ZERO,
                    packet_type: "production_output_ack_v1".to_owned(),
                    priority: 0,
                    // `settled_at` is the instant the credits settled, which
                    // is their `at`: a deferred batch is retimed to it.
                    payload: serde_json::json!({
                        "execution": execution,
                        "production_source": packet.provider_source,
                        "outcomes": outcomes,
                        "settled_at": context.at,
                    }),
                    affected: Vec::new(),
                });
            out.changed = true;
        }
        crate::OutputBatchSettlement::NoRoom(shortage) => match admitted.origin {
            PacketOrigin::Plugin { deferrals } if deferral_is_worthwhile(shortage, deferrals) => {
                out.defer(
                    ingress.id,
                    context.at,
                    shortage,
                    DeferredWork {
                        packet: DeferredPacket::OutputBatch(&packet),
                        deferrals,
                        operation_key: first_key,
                    },
                    holder,
                )?;
            }
            PacketOrigin::Plugin { .. } | PacketOrigin::Host => {
                out.skip_for_room(ingress.id, shortage, first_key, holder);
            }
        },
        crate::OutputBatchSettlement::Refused(operation_key, reason) => out.skip(
            ingress.id,
            crate::ResourceIngressSkipReasonV1::FailedSettlementChecks,
            Some(&operation_key),
            &format!("a credit of the production output batch was refused: {reason}"),
            holder,
        ),
    }
    Ok(())
}

/// Settles one requester-scoped allocation pass.
fn settle_allocation_packet(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    ingress: &IngressRecord,
    payload: &Value,
    state: &mut crate::ResourceState,
    out: &mut LifecycleSettlement,
) -> Result<(), CanwuError> {
    let Ok(packet) =
        decode::<ResourceAuthorizedAllocationV1>(payload, "authorized resource allocation")
    else {
        out.skip_undecodable(ingress.id, "resource allocation pass");
        return Ok(());
    };
    let requester = EventHolder::Admitted(&packet.requester);
    let operation = ResourceOperationRequestV1::Allocate(packet.request.clone());
    let operation_key = operation.operation_key();
    // `enqueue_resource_allocation` refuses such a packet.
    if operation.names_reserved_operation_key() {
        out.skip(
            ingress.id,
            crate::ResourceIngressSkipReasonV1::ReservedOperationKey,
            Some(&operation_key),
            RESERVED_OPERATION_KEY_DETAIL,
            requester,
        );
        return Ok(());
    }
    let request_digest = crate::canonical_digest(
        "canwu.resource.authorized-allocation-request.v1",
        &(&packet.requester, &packet.request),
    )
    .map_err(resource_canwu_error)?;
    if !out.operation_key_is_free(
        view,
        state,
        &operation,
        &request_digest,
        ingress.id,
        requester,
    )? {
        return Ok(());
    }
    match state
        .settle_authorized_allocation(&packet.requester, &packet.request, context.at)
        .map_err(resource_canwu_error)?
    {
        crate::OperationSettlement::Settled(_) => {
            out.changed = true;
            out.affected.extend(registered_holder(&packet.requester));
        }
        crate::OperationSettlement::NoRoom(shortage) => {
            out.skip_for_room(ingress.id, shortage, Some(&operation_key), requester);
        }
    }
    Ok(())
}

/// Settles one completion operation from the completion ingress.
///
/// A resend answered by its own outcome settles nothing. A packet whose key
/// another request holds, or that fails the checks of its step, settles
/// nothing with a skip event: a holder's tracked `Acquire`, or `Abort` of its
/// own lease, can take a key first, and that must not fail the boundary. A
/// packet whose result does not fit the root is deferred when a plugin
/// scheduled it, the step can settle later ([`completion_step_can_wait`]),
/// and the shortage can pass ([`deferral_is_worthwhile`]); it is skipped
/// otherwise. A single step records what it finds against the view as its
/// rejection instead.
///
/// What an applied step sets in motion, and only an applied one, follows the
/// settlement: the coordinator's acknowledgement of a completed external
/// participant, the expiry tick a grant starts, and the tick that cleans up an
/// aborting acquisition. A recorded rejection sets nothing in motion.
fn settle_completion_packet(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    admitted: Admitted<'_>,
    state: &mut crate::ResourceState,
    out: &mut LifecycleSettlement,
) -> Result<(), CanwuError> {
    let ingress = admitted.ingress;
    let Ok(operation) =
        decode::<crate::ResourceCompletionOperationV1>(admitted.payload, "completion operation")
    else {
        out.skip_undecodable(ingress.id, "resource completion operation");
        return Ok(());
    };
    let request = ResourceOperationRequestV1::Completion(operation.clone());
    let operation_key = request.operation_key();
    let holder = external_participant_holder(state, &operation);
    let holder = holder
        .as_ref()
        .map_or(EventHolder::Nobody, EventHolder::Participant);
    let request_digest = request.request_digest().map_err(resource_canwu_error)?;
    if !out.operation_key_is_free(view, state, &request, &request_digest, ingress.id, holder)? {
        return Ok(());
    }
    let skip_failed = |out: &mut LifecycleSettlement, error: &CanwuError| {
        out.skip(
            ingress.id,
            crate::ResourceIngressSkipReasonV1::FailedSettlementChecks,
            Some(&operation_key),
            &error.message,
            holder,
        );
    };
    let no_room = |out: &mut LifecycleSettlement, shortage| match admitted.origin {
        PacketOrigin::Plugin { deferrals }
            if completion_step_can_wait(&operation)
                && deferral_is_worthwhile(shortage, deferrals) =>
        {
            out.defer(
                ingress.id,
                context.at,
                shortage,
                DeferredWork {
                    packet: DeferredPacket::Completion(&operation),
                    deferrals,
                    operation_key: Some(&operation_key),
                },
                holder,
            )
        }
        PacketOrigin::Plugin { .. } | PacketOrigin::Host => {
            out.skip_for_room(ingress.id, shortage, Some(&operation_key), holder);
            Ok(())
        }
    };
    if let crate::ResourceCompletionOperationV1::SingleStepLease(lease) = &operation {
        // Every domain refusal of a single-step lease is its recorded
        // rejection, and needs room for that outcome only. The step leaves an
        // activated lease or nothing, so it starts no expiry clock.
        let refusal = match single_step_lease_refusal(view, context.at, lease, state) {
            Ok(refusal) => refusal,
            Err(error) => {
                skip_failed(out, &error);
                return Ok(());
            }
        };
        match state
            .settle_single_step_lease_with_external_validation(lease, refusal.as_ref())
            .map_err(resource_canwu_error)?
        {
            crate::OperationSettlement::Settled(_) => out.changed = true,
            crate::OperationSettlement::NoRoom(shortage) => no_room(out, shortage)?,
        }
        return Ok(());
    }
    if let Err(error) = validate_completion_ingress(view, context, &operation, state) {
        skip_failed(out, &error);
        return Ok(());
    }
    let settled = match &operation {
        crate::ResourceCompletionOperationV1::Prepare(request) => {
            let external_targets_current =
                match prepare_external_targets_current(view, state, request) {
                    Ok(current) => current,
                    Err(error) => {
                        skip_failed(out, &error);
                        return Ok(());
                    }
                };
            state.settle_prepare_with_external_revalidation(request, external_targets_current)
        }
        crate::ResourceCompletionOperationV1::Acquire(_) => {
            let refusal = lease_identity_refusal(view, state, &operation)?;
            state.settle_operation_refused_by(&request, None, refusal.as_ref())
        }
        _ => state.settle_operation(&request),
    }
    .map_err(resource_canwu_error)?;
    let outcome = match settled {
        crate::OperationSettlement::Settled(outcome) => outcome,
        crate::OperationSettlement::NoRoom(shortage) => return no_room(out, shortage),
    };
    out.changed = true;
    if outcome.status != crate::ResourceOperationStatus::Applied {
        return Ok(());
    }
    match &operation {
        crate::ResourceCompletionOperationV1::Prepare(request)
            if state
                .completion_leases
                .acquisitions
                .get(&request.acquisition)
                .is_some_and(|value| {
                    value.state == crate::CompletionLeaseAcquisitionStateV1::Aborting
                }) =>
        {
            out.schedule_expiry_tick();
        }
        crate::ResourceCompletionOperationV1::Grant(_)
        | crate::ResourceCompletionOperationV1::GrantExternalParticipant(_)
            if !state.completion_leases.expiry_due.is_empty()
                || !state.external_completion_participants.expiry_due.is_empty() =>
        {
            out.schedule_expiry_tick();
        }
        crate::ResourceCompletionOperationV1::CompleteExternalParticipant(request) => {
            // The completion applied, so the root holds the participant.
            let participant = state
                .external_completion_participants
                .participant(&request.acquisition)
                .ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::InvalidDomainRecord,
                        "an applied external participant completion left no participant",
                    )
                })?;
            if participant.coordinator_plugin == "canwu-production" {
                out.directives
                    .push(BoundaryDirective::SchedulePluginIngress {
                        target_plugin: participant.coordinator_plugin.clone(),
                        after: canwu_api::SimDuration::ZERO,
                        packet_type: "production_completion_operation_v1".to_owned(),
                        priority: 0,
                        payload: serde_json::json!({
                            "completion": "acknowledge_participant_completed",
                            "request": {
                                "acquisition": request.acquisition,
                                "participant": PLUGIN_NAME,
                            },
                        }),
                        affected: Vec::new(),
                    });
            }
        }
        _ => {}
    }
    Ok(())
}

/// Settles work the resource plugin deferred, as the packet it wraps and
/// with the count of its deferrals. No host application can queue such a
/// packet: its permit is never handed out, so only a plugin's boundary system
/// can schedule it, and one that is not from a boundary is skipped. Another
/// plugin that declares it as a target gains nothing but a deferral count.
fn settle_deferred_packet(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    ingress: &IngressRecord,
    payload: &Value,
    state: &mut crate::ResourceState,
    out: &mut LifecycleSettlement,
) -> Result<(), CanwuError> {
    let Ok(deferred) = decode::<ResourceDeferredPacketV1>(payload, "deferred resource packet")
    else {
        out.skip_undecodable(ingress.id, "deferred resource packet");
        return Ok(());
    };
    if PacketOrigin::of(ingress) == PacketOrigin::Host {
        out.skip(
            ingress.id,
            crate::ResourceIngressSkipReasonV1::FailedSettlementChecks,
            None,
            "a deferred resource packet was not scheduled by a plugin",
            EventHolder::Nobody,
        );
        return Ok(());
    }
    let admitted = Admitted {
        ingress,
        payload: &deferred.payload,
        origin: PacketOrigin::Plugin {
            deferrals: deferred.deferrals,
        },
    };
    match deferred.packet_type.as_str() {
        RESOURCE_PRODUCTION_OUTPUT_BATCH_INGRESS => {
            settle_output_batch_packet(view, context, admitted, state, out)
        }
        RESOURCE_COMPLETION_INGRESS => {
            settle_completion_packet(view, context, admitted, state, out)
        }
        // Only a consumption batch past the boundary's batch budget.
        RESOURCE_ADAPTER_INGRESS => settle_adapter_packet(view, context, admitted, state, out),
        _ => {
            out.skip(
                ingress.id,
                crate::ResourceIngressSkipReasonV1::FailedSettlementChecks,
                None,
                "a deferred resource packet wraps a packet type that is never deferred",
                EventHolder::Nobody,
            );
            Ok(())
        }
    }
}

/// Whether every external record a prepared grant locks is still its
/// record's current version.
fn prepare_external_targets_current(
    view: &SimulationView<'_>,
    state: &crate::ResourceState,
    request: &crate::PrepareCompletionCapacityV1,
) -> Result<bool, CanwuError> {
    state
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
            crate::CompletionLockedTargetV1::ExternalRecord { version } => Some(version),
            _ => None,
        })
        .try_fold(true, |all_current, version| {
            Ok(all_current && is_current_version(view, version)?)
        })
}

/// The expiries an expiry tick settles at `context`: of completion grants and
/// of external participants. Their operation keys name the instant and the
/// boundary, not anything the tick's payload names.
fn expiry_tick_operations(context: &BoundaryContext) -> [crate::ResourceCompletionOperationV1; 2] {
    [
        crate::ResourceCompletionOperationV1::Expire(crate::ExpireCompletionCapacityV1 {
            at: context.at,
            current_boundary: context.boundary_id.get(),
            candidate_limit: 1_024,
        }),
        crate::ResourceCompletionOperationV1::ExpireExternalParticipants(
            crate::ExpireExternalCompletionParticipantGrantsV1 {
                at: context.at,
                current_boundary: context.boundary_id.get(),
                candidate_limit: 1_024,
            },
        ),
    ]
}

/// The operation keys an expiry tick settles under at `context`.
fn expiry_tick_operation_keys(context: &BoundaryContext) -> Vec<crate::ResourceOperationKey> {
    expiry_tick_operations(context)
        .into_iter()
        .map(|operation| ResourceOperationRequestV1::Completion(operation).operation_key())
        .collect()
}

/// Advances completion-lease expiry and the cleanup of aborting leases.
///
/// An expiry whose result does not fit the root settles nothing, with a skip
/// event, and the tick does not schedule itself again: a tick scheduled at
/// the same instant would meet the same root, so the run would settle
/// boundaries at that instant without end. Expiry resumes with the tick the
/// next applied archive commit schedules while an expiry is due, which makes
/// room for it, with the tick the next grant schedules, or with one the host
/// application queues ([`enqueue_resource_completion_expiry_tick`]).
///
/// An expiry's key names the instant and the boundary, and a host
/// application's completion operation may have taken it first. The same
/// expiry recorded before is a resend and settles nothing; an expiry another
/// request holds the key of settles nothing with a skip event, and the tick
/// is scheduled again, at a boundary whose key is another.
fn settle_expiry_tick(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    ingress: IngressId,
    state: &mut crate::ResourceState,
    out: &mut LifecycleSettlement,
) -> Result<(), CanwuError> {
    let boundary = context.boundary_id.get();
    let mut reschedule = true;
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
        state.retire_finished_leases();
        out.changed = true;
    }
    let [local, external] = expiry_tick_operations(context);
    let mut expiries = Vec::new();
    if state
        .completion_leases
        .expiry_due
        .range(..=boundary)
        .any(|(_, values)| !values.is_empty())
    {
        expiries.push(local);
    }
    if state
        .external_completion_participants
        .expiry_due
        .range(..=boundary)
        .any(|(_, values)| !values.is_empty())
    {
        expiries.push(external);
    }
    for expiry in expiries {
        let request = ResourceOperationRequestV1::Completion(expiry);
        let request_digest = request.request_digest().map_err(resource_canwu_error)?;
        match operation_key_holder(view, state, &request, &request_digest)? {
            OperationKeyHolder::Nobody => {}
            OperationKeyHolder::ThisRequestInRoot | OperationKeyHolder::ThisRequestInArchive => {
                continue;
            }
            OperationKeyHolder::AnotherRequest => {
                out.skip(
                    ingress,
                    crate::ResourceIngressSkipReasonV1::OperationKeyTaken,
                    Some(&request.operation_key()),
                    OPERATION_KEY_TAKEN_DETAIL,
                    EventHolder::Nobody,
                );
                continue;
            }
        }
        match state
            .settle_operation(&request)
            .map_err(resource_canwu_error)?
        {
            crate::OperationSettlement::Settled(_) => out.changed = true,
            crate::OperationSettlement::NoRoom(shortage) => {
                out.skip_for_room(
                    ingress,
                    shortage,
                    Some(&request.operation_key()),
                    EventHolder::Nobody,
                );
                reschedule = false;
            }
        }
    }
    if reschedule
        && (!state.completion_leases.expiry_due.is_empty()
            || !state.external_completion_participants.expiry_due.is_empty()
            || state
                .completion_leases
                .acquisitions
                .values()
                .any(|value| value.state == crate::CompletionLeaseAcquisitionStateV1::Aborting))
    {
        out.schedule_expiry_tick();
    }
    Ok(())
}

/// The resource packets among `admitted`: archive commits and retention
/// acknowledgements first, then every other one, each group in its admitted
/// order. Packets of other plugins are left out.
///
/// A full candidate budget skips the next operation, and the remedy is an
/// archive commit. Settling the commit first lets it apply against the root
/// the boundary started from, the root it was prepared from, and makes room
/// for the operations admitted beside it, instead of going stale after one of
/// them. A boundary that admitted no archive packet, or nothing else, keeps
/// its order.
fn resource_packets_maintenance_first(
    view: &SimulationView<'_>,
    admitted: &[canwu_api::IngressId],
) -> Result<Vec<canwu_api::IngressId>, CanwuError> {
    let mut maintenance = Vec::new();
    let mut others = Vec::new();
    for ingress_id in admitted {
        let Some(IngressPayload::Plugin {
            plugin,
            packet_type,
            ..
        }) = view.ingress(*ingress_id)?.map(|ingress| &ingress.payload)
        else {
            continue;
        };
        if plugin != PLUGIN_NAME {
            continue;
        }
        if matches!(
            packet_type.as_str(),
            RESOURCE_ARCHIVE_COMMIT_INGRESS | RESOURCE_ARCHIVE_RETENTION_ACK_INGRESS
        ) {
            maintenance.push(*ingress_id);
        } else {
            others.push(*ingress_id);
        }
    }
    maintenance.extend(others);
    Ok(maintenance)
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

/// Validates the live root a boundary proposed. The resource plugin
/// validates each hot terminal record as it settles and attaches it, so this
/// reads the live root alone, and nothing in a boundary that wrote none.
fn validate_resource_invariants(
    view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let Some(record) = view.proposed_typed_domain_record(&resource_runtime_reference())? else {
        return Ok(BoundaryProposal::default());
    };
    crate::decode_live_root(record)?
        .validate()
        .map_err(resource_canwu_error)?;
    Ok(BoundaryProposal::default())
}

/// Whether the live root `payload` holds a dirty report grant or a report due
/// by minute `now`, read from its encoded value without decoding the root.
fn report_work_due(payload: &Value, now: i64) -> bool {
    let dirty = payload
        .get("report_dirty_grants")
        .and_then(Value::as_array)
        .is_some_and(|grants| !grants.is_empty());
    // A key that is not a minute is decoded, and refused, with the root.
    let due = payload
        .get("report_due_index")
        .and_then(Value::as_object)
        .is_some_and(|index| {
            index
                .keys()
                .any(|at| at.parse::<i64>().map_or(true, |at| at <= now))
        });
    dirty || due
}

/// Publishes the holder reports that are due or dirty. A boundary in which
/// none is reads only two fields of the live root's encoded value.
fn materialize_resource_reports(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let Some(live) = view.typed_domain_record(&resource_runtime_reference())? else {
        return Ok(BoundaryProposal::default());
    };
    if !report_work_due(&live.payload, context.at.as_minutes()) {
        return Ok(BoundaryProposal::default());
    }
    let mut work = crate::ResourceRootWork::new(view, live)?;
    let proposal = materialize_reports_on(view, context, &mut work)?;
    // As for the lifecycle: a debug build requires the whole root to
    // publish the same reports and write the same records.
    #[cfg(debug_assertions)]
    {
        let mut whole = crate::ResourceRootWork::new(view, live)?;
        whole.attach_all()?;
        let expected = materialize_reports_on(view, context, &mut whole)?;
        assert_eq!(
            proposal, expected,
            "resource reports materialized differently on the live root than on the whole root"
        );
    }
    Ok(proposal)
}

fn materialize_reports_on<S: crate::HotTerminalSource + ?Sized>(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    work: &mut crate::ResourceRootWork<'_, S>,
) -> Result<BoundaryProposal, CanwuError> {
    let state = &mut work.state;
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
    // A head is checked against the records it cites while its revision is
    // current, so they are attached first.
    let heads = grants_by_holder
        .values()
        .flatten()
        .filter_map(|(grant_id, _)| {
            state
                .observation_head_by_grant
                .get(grant_id)
                .and_then(|head| state.observation_heads.get(head))
        })
        .map(encode)
        .collect::<Result<Vec<_>, _>>()?;
    for head in &heads {
        work.attach_named(head)?;
    }
    // A report reads the accounts of its grant.
    let accounts = grants_by_holder
        .values()
        .flatten()
        .filter_map(|(grant_id, _)| work.state.report_grants.get(grant_id))
        .flat_map(|grant| grant.accounts.iter().cloned())
        .map(crate::ResourceLiveRecordKeyV1::Account)
        .collect::<Vec<_>>();
    work.attach_live(accounts)?;
    let state = &mut work.state;
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
                materialize_resource_report(state, &holder, &grant_id, context.at, context.at)
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
            visibility: StateVisibility::SameBoundary,
            producer_correlation: Some(format!(
                "resource-report-batch::{}::{batch_ordinal}",
                context.boundary_id
            )),
            supersession: canwu_api::KnowledgeSupersession::Explicit,
            groups: vec![canwu_api::KnowledgePublicationGroup {
                holders: vec![holder],
                records,
            }],
            summary: "Publish holder-relative resource reports".to_owned(),
        });
    }
    if changed {
        state.retire_finished_leases();
        state.state_revision = crate::ResourceRevision::new(
            work.live_version
                .checked_add(1)
                .ok_or_else(|| invalid("resource report state revision overflowed"))?,
        )
        .map_err(resource_canwu_error)?;
        work.state.validate().map_err(resource_canwu_error)?;
        let mut writes = work.persist("Persist resource report due/dirty fairness state")?;
        writes.append(&mut directives);
        directives = writes;
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
) -> Result<(), CanwuError> {
    reject_adapter_production_credit(&packet.request)?;
    if packet.provider_plugin == "canwu-force-supply-reference"
        && matches!(packet.request, ResourceOperationRequestV1::ConsumeBatch(_))
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "force supply consumptions settle one at a time, not as a batch",
        ));
    }
    // Only the live body is read, and a provider must cite its current
    // version, whose body that is. A force consumption may also cite an
    // earlier version of the force runtime, which the resource does not
    // persist: the pending intent in the live body keeps the key, allocation,
    // and certificate it had then until the force acknowledges this
    // consumption's outcome. Its persisted consumer evidence must be the force
    // version its certificate locked.
    let force_consumption = packet.provider_plugin == "canwu-force-supply-reference"
        && matches!(packet.request, ResourceOperationRequestV1::Consume(_));
    let source = if force_consumption {
        require_known_version(view, &packet.provider_source)?;
        live_body(view, &packet.provider_source)?
    } else {
        current_version_body(view, &packet.provider_source)?
    };
    if let ResourceOperationRequestV1::Consume(request) = &packet.request
        && force_consumption
        && !request
            .completion_certificate
            .locked_target_versions
            .contains(&crate::CompletionLockedTargetV1::ExternalRecord {
                version: request.consumer_evidence.clone(),
            })
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "force consumption evidence is not the force version its certificate locked",
        ));
    }
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
    let typed_consumption = match &packet.request {
        ResourceOperationRequestV1::Consume(request)
            if packet.provider_plugin != "canwu-force-supply-reference" =>
        {
            Some((&request.operation_key, &request.completion_certificate))
        }
        ResourceOperationRequestV1::ConsumeBatch(request) => {
            Some((&request.operation_key, &request.completion_certificate))
        }
        _ => None,
    };
    if let Some((operation_key, certificate)) = typed_consumption
        && (source.lifecycle != canwu_api::DomainRecordLifecycle::Active
            || operation_key != &certificate.operation_key
            || !certificate.locked_target_versions.contains(
                &crate::CompletionLockedTargetV1::ExternalRecord {
                    version: packet.provider_source.clone(),
                },
            ))
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "typed consumption requires an active source and exact operation/source lease binding",
        ));
    }
    validate_request_certificate_evidence(view, &packet.request)?;
    validate_local_provider_participant(resource_state, packet)?;
    authoritative_provider_operation(&source.payload, packet, resource_state)
}

/// Production output credits settle only through
/// [`RESOURCE_PRODUCTION_OUTPUT_BATCH_INGRESS`], which binds every output leg
/// of one execution to the source version production pinned on it. Adapter
/// ingress never credits production output.
fn reject_adapter_production_credit(
    request: &ResourceOperationRequestV1,
) -> Result<(), CanwuError> {
    if let ResourceOperationRequestV1::Credit(credit) = request
        && matches!(credit.source, crate::ResourceCreditSourceV1::Production(_))
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "production output credits settle only through the production output batch ingress",
        ));
    }
    Ok(())
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
    // The coordinator pins the cited version on the execution as it
    // dispatches this batch, which advances its runtime, so the cited version
    // is never current here. Read the live body instead: until this batch
    // settles, it must still show the pinned execution awaiting output
    // settlement, with the certificate and output requests it had then.
    require_known_version(view, &packet.provider_source)?;
    let source = live_body(view, &packet.provider_source)?;
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
    let execution = authoritative_production_output_batch(&source.payload, packet)?;
    require_pinned_output_source(&source.payload, &packet.provider_source, &execution)?;
    Ok(execution)
}

/// The completion lease locked an older coordinator version, and the resource
/// runtime accepts a production credit source as that record at or after the
/// locked version. Exactness therefore comes from the coordinator itself: the
/// credit must cite the exact source version the coordinator pinned on the
/// execution (`output_source`) when it dispatched the output.
fn require_pinned_output_source(
    current: &Value,
    provider_source: &canwu_api::DomainRecordVersionRef,
    execution: &Value,
) -> Result<(), CanwuError> {
    let pinned = current
        .get("executions")
        .and_then(|executions| executions.get(execution.as_str()?))
        .and_then(|entry| entry.get("output_source"))
        .cloned();
    if pinned != Some(encode(provider_source)?) {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "production output credit does not cite the coordinator's pinned output source",
        ));
    }
    Ok(())
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
        ResourceOperationRequestV1::ConsumeBatch(value) => Some(&value.completion_certificate),
        ResourceOperationRequestV1::Credit(value)
            if !matches!(value.source, crate::ResourceCreditSourceV1::Production(_)) =>
        {
            Some(&value.completion_certificate)
        }
        ResourceOperationRequestV1::ExternalOutflow(value) => Some(&value.completion_certificate),
        ResourceOperationRequestV1::RecordLoss(value) => Some(&value.completion_certificate),
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
        if matches!(packet.request, ResourceOperationRequestV1::ConsumeBatch(_)) {
            return Err(CanwuError::new(
                ErrorCode::InvalidAuthority,
                "a resource consumption batch runs under a local completion lease",
            ));
        }
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
    // Every item's demand belongs to the lease holder. An item whose leg or
    // demand is gone is refused alone when the batch settles.
    if let ResourceOperationRequestV1::ConsumeBatch(request) = &packet.request
        && request.items.iter().any(|item| {
            state
                .demands
                .get(&item.demand)
                .is_some_and(|demand| demand.requester != acquisition.holder)
        })
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "a resource consumption batch names a demand its lease holder did not request",
        ));
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
) -> Result<(), CanwuError> {
    match &packet.request {
        ResourceOperationRequestV1::Consume(request)
            if packet.provider_plugin == "canwu-force-supply-reference" =>
        {
            authoritative_force_consumption(payload, request)
        }
        ResourceOperationRequestV1::Consume(request) => {
            authoritative_typed_consumption(payload, packet, request, resource_state)
        }
        ResourceOperationRequestV1::ConsumeBatch(request) => {
            authoritative_typed_batch(payload, packet, request, resource_state)
        }
        _ => Ok(()),
    }
}

/// Checks that the provider authorizes every item of a consumption batch
/// with exactly one typed consumption intent under the item's key. The
/// intent names the item's request; whether the leg, account, and demand are
/// still those revisions is the item's own check when it settles.
fn authoritative_typed_batch(
    payload: &Value,
    packet: &ResourceAdapterOperationV1,
    request: &crate::ResourceConsumptionBatchRequestV1,
    state: &crate::ResourceState,
) -> Result<(), CanwuError> {
    if request.consumer_evidence != packet.provider_source {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "consumption evidence is not the exact provider payload version",
        ));
    }
    let intents = payload
        .get("resource_consumption_intents")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidDomainRecord,
                "provider has no typed resource consumption intents",
            )
        })?;
    if intents.len() > state.limits.max_operation_outcomes {
        return Err(CanwuError::new(
            ErrorCode::ValueOutOfRange,
            "resource consumption intent map exceeds max_operation_outcomes",
        ));
    }
    let mut authorized = std::collections::BTreeMap::<
        crate::ResourceOperationKey,
        Vec<crate::ResourceConsumptionIntentV1>,
    >::new();
    for (map_id, value) in intents {
        let intent: crate::ResourceConsumptionIntentV1 = serde_json::from_value(value.clone())
            .map_err(|error| {
                CanwuError::new(
                    ErrorCode::InvalidDomainRecord,
                    format!("resource consumption intent cannot be decoded: {error}"),
                )
            })?;
        intent.validate().map_err(resource_canwu_error)?;
        if map_id != intent.id.as_str() {
            return Err(CanwuError::new(
                ErrorCode::InvalidDomainRecord,
                "resource consumption intent map key differs from its identity",
            ));
        }
        if intent.status == crate::ResourceConsumptionIntentStatusV1::Authorized
            && intent.provider_plugin == packet.provider_plugin
        {
            authorized
                .entry(intent.operation_key.clone())
                .or_default()
                .push(intent);
        }
    }
    for (index, item) in request.items.iter().enumerate() {
        let key = request.item_operation_key(index);
        let matching = authorized
            .get(&key)
            .into_iter()
            .flatten()
            .filter(|intent| {
                intent.demand == item.demand
                    && intent.demand_revision == item.demand_revision
                    && intent.allocation == item.allocation
                    && intent.account == item.allocation.account
                    && intent.expected_account_revision == item.expected_account_revision
                    && intent.consumption_id == item.consumption_id
                    && intent.quantity == item.allocation.quantity
            })
            .count();
        if matching != 1 {
            return Err(CanwuError::new(
                ErrorCode::InvalidAuthority,
                format!(
                    "provider payload does not uniquely authorize consumption batch item {index} (matches={matching})"
                ),
            ));
        }
    }
    Ok(())
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

fn authoritative_typed_consumption(
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
                "provider has no typed resource consumption intents",
            )
        })?;
    if request.consumer_evidence != packet.provider_source {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "consumption evidence is not the exact provider payload version",
        ));
    }
    if intents.len() > state.limits.max_operation_outcomes {
        return Err(CanwuError::new(
            ErrorCode::ValueOutOfRange,
            "resource consumption intent map exceeds max_operation_outcomes",
        ));
    }
    let mut matching = 0_usize;
    for (map_id, value) in intents {
        let intent: crate::ResourceConsumptionIntentV1 = serde_json::from_value(value.clone())
            .map_err(|error| {
                CanwuError::new(
                    ErrorCode::InvalidDomainRecord,
                    format!("resource consumption intent cannot be decoded: {error}"),
                )
            })?;
        intent.validate().map_err(resource_canwu_error)?;
        if map_id != intent.id.as_str() {
            return Err(CanwuError::new(
                ErrorCode::InvalidDomainRecord,
                "resource consumption intent map key differs from its identity",
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
                    "resource consumption intent match count overflowed",
                )
            })?;
        }
    }
    if matching != 1 {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            format!(
                "provider payload does not uniquely authorize the exact typed consumption intent (matches={matching})"
            ),
        ));
    }
    Ok(())
}

const fn request_time(request: &ResourceOperationRequestV1) -> Option<SimTime> {
    match request {
        ResourceOperationRequestV1::Consume(value) => Some(value.at),
        ResourceOperationRequestV1::ConsumeBatch(value) => Some(value.at),
        ResourceOperationRequestV1::BeginTransfer(value) => Some(value.at),
        ResourceOperationRequestV1::CancelTransfer(value) => Some(value.at),
        ResourceOperationRequestV1::CompleteTransfer(value) => Some(value.at),
        ResourceOperationRequestV1::Credit(value) => Some(value.at),
        ResourceOperationRequestV1::ExternalOutflow(value) => Some(value.at),
        ResourceOperationRequestV1::RecordLoss(value) => Some(value.at),
        _ => None,
    }
}

fn validate_completion_ingress(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    operation: &crate::ResourceCompletionOperationV1,
    state: &crate::ResourceState,
) -> Result<(), CanwuError> {
    let at = context.at;
    let boundary = context.boundary_id.get();
    match operation {
        crate::ResourceCompletionOperationV1::Acquire(request) => {
            if request.eligibility_time != at {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "completion acquisition eligibility time must equal its canonical boundary",
                ));
            }
            validate_completion_report_reservation(view, state, &request.recipe)?;
            // The acquisition persists this evidence as a payload-required
            // dependency, so each version must be proven, which only the
            // current version can be without retained evidence.
            for version in request
                .eligibility_envelope
                .exact_evidence
                .iter()
                .chain(&request.eligibility_envelope.capability_bindings)
                .chain(&request.eligibility_envelope.route_evidence)
            {
                require_current_version(view, version)?;
            }
        }
        crate::ResourceCompletionOperationV1::Grant(request) => {
            for target in &request.target_versions {
                if let crate::CompletionLockedTargetV1::ExternalRecord { version } = target {
                    require_current_version(view, version)?;
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
            if request.at != at || request.current_boundary != boundary {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "completion expiry must name its settling boundary and that boundary's simulation time",
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
        // A single-step lease is checked by `single_step_lease_refusal`,
        // whose findings settle as a recorded rejection.
        crate::ResourceCompletionOperationV1::Abort(_)
        | crate::ResourceCompletionOperationV1::Release(_)
        | crate::ResourceCompletionOperationV1::CompleteExternalParticipant(_)
        | crate::ResourceCompletionOperationV1::SingleStepLease(_) => {}
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
                    require_current_version(view, version)?;
                }
            }
        }
        crate::ResourceCompletionOperationV1::PrepareExternalParticipant(request) => {
            let source = current_version_body(view, &request.coordinator_source)?;
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
            let source = current_version_body(view, &request.coordinator_source)?;
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
            let source = current_version_body(view, &request.coordinator_source)?;
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
            if request.at != at || request.current_boundary != boundary {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "external participant expiry must name its settling boundary and that boundary's simulation time",
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
    if !completion_report_reservation_fits(view, state, recipe)? {
        return Err(CanwuError::new(
            ErrorCode::KnowledgeLimitExceeded,
            "completion admission cannot reserve its bounded terminal report path",
        ));
    }
    Ok(())
}

/// Whether the hot report capacity has room for the reports `recipe`
/// reserves, beside those already published and reserved.
fn completion_report_reservation_fits(
    view: &SimulationView<'_>,
    state: &crate::ResourceState,
    recipe: &crate::CompletionCapacityRecipeV1,
) -> Result<bool, CanwuError> {
    let requested = usize::from(recipe.reports_per_holder).checked_mul(usize::from(recipe.holders));
    let projected = view
        .knowledge_record_count_in_namespace(PLUGIN_NAMESPACE)?
        .checked_add(
            state
                .reserved_knowledge_report_slots()
                .map_err(resource_canwu_error)?,
        )
        .zip(requested)
        .and_then(|(held, requested)| held.checked_add(requested));
    Ok(projected.is_some_and(|projected| projected <= RESOURCE_REPORT_HOT_CAPACITY))
}

/// What the simulation view holds against one single-step lease, or `None`.
///
/// These are the checks the acquire, grant, prepare, and activate steps make
/// against the view: the eligibility time is the boundary time, the reserved
/// reports fit, and every external record version the lease persists or
/// locks is its record's current version. The four-step path skips its
/// packet on some of them. For a single step each finding becomes the
/// operation's recorded rejection instead. Only live state and
/// current-version provenance are read, never retained evidence, so a sealed
/// run and its exact replay find the same.
///
/// A recorded finding needs room for its outcome only. A request that does
/// not fit even that, or an applied lease that does not fit, is skipped, and
/// an error here (a view read other than an undeclared record kind) skips the
/// packet too.
fn single_step_lease_refusal(
    view: &SimulationView<'_>,
    at: SimTime,
    request: &crate::SingleStepCompletionLeaseV1,
    state: &crate::ResourceState,
) -> Result<Option<crate::ResourceError>, CanwuError> {
    let lease = &request.lease;
    if let Some(refusal) = lease_identity_refusal(
        view,
        state,
        &crate::ResourceCompletionOperationV1::SingleStepLease(request.clone()),
    )? {
        return Ok(Some(refusal));
    }
    if lease.eligibility_time != at {
        return Ok(Some(crate::ResourceError::Authority(
            "single-step completion lease eligibility time must equal its canonical boundary"
                .to_owned(),
        )));
    }
    if !completion_report_reservation_fits(view, state, &lease.recipe)? {
        return Ok(Some(crate::ResourceError::LimitExceeded(
            "completion admission cannot reserve its bounded terminal report path".to_owned(),
        )));
    }
    let envelope = &lease.eligibility_envelope;
    let locked = request
        .target_versions
        .iter()
        .filter_map(|target| match target {
            crate::CompletionLockedTargetV1::ExternalRecord { version } => Some(version),
            _ => None,
        });
    for version in envelope
        .exact_evidence
        .iter()
        .chain(&envelope.capability_bindings)
        .chain(&envelope.route_evidence)
        .chain(locked)
    {
        match is_current_version(view, version) {
            Ok(true) => {}
            Ok(false) => {
                return Ok(Some(crate::ResourceError::VersionConflict(
                    "single-step completion lease cites an external record version that is not current"
                        .to_owned(),
                )));
            }
            // The plugin reads only the record kinds it was configured
            // with, so a version of any other kind cannot be proven.
            Err(error) if error.code == ErrorCode::UndeclaredStateRead => {
                return Ok(Some(crate::ResourceError::Authority(
                    "single-step completion lease cites a record kind the resource plugin does not read"
                        .to_owned(),
                )));
            }
            Err(error) => return Err(error),
        }
    }
    Ok(None)
}

fn require_external_coordinator_source(
    view: &SimulationView<'_>,
    coordinator_plugin: &str,
    source: &canwu_api::DomainRecordVersionRef,
) -> Result<canwu_api::DomainRecord, CanwuError> {
    let record = current_version_body(view, source)?;
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
    let certificates = match request {
        ResourceOperationRequestV1::Consume(value) => vec![&value.completion_certificate],
        ResourceOperationRequestV1::ConsumeBatch(value) => vec![&value.completion_certificate],
        ResourceOperationRequestV1::BeginTransfer(value) => vec![&value.completion_certificate],
        ResourceOperationRequestV1::CompleteTransfer(value) => vec![&value.completion_certificate],
        ResourceOperationRequestV1::Credit(value) => vec![&value.completion_certificate],
        ResourceOperationRequestV1::ExternalOutflow(value) => vec![&value.completion_certificate],
        ResourceOperationRequestV1::RecordLoss(value) => vec![&value.completion_certificate],
        ResourceOperationRequestV1::BeginExchange(value) => vec![
            &value.leg_a.completion_certificate,
            &value.leg_b.completion_certificate,
        ],
        _ => Vec::new(),
    };
    for certificate in certificates {
        for target in &certificate.locked_target_versions {
            if let crate::CompletionLockedTargetV1::ExternalRecord { version } = target {
                require_known_version(view, version)?;
            }
        }
    }
    Ok(())
}

// Exact version checks read only the current-version provenance index and
// live records. Sealing drops every retained body and every receipt no live
// record depends on, while exact replay retains them all, so a check that
// reads evidence could settle differently in a sealed run and its replay.

/// Requires `version` to be consistent with its live record's current-version
/// index: the current version must match it exactly, and an older version
/// must precede it in both number and establishment. An older version's
/// establishment source is not proven, so this suits only a citation that is
/// not persisted or is bound to persisted state elsewhere; a persisted
/// unproven citation would fail restore validation or a later seal.
fn require_known_version(
    view: &SimulationView<'_>,
    version: &canwu_api::DomainRecordVersionRef,
) -> Result<(), CanwuError> {
    let known = view
        .current_domain_record_version(&version.record)?
        .is_some_and(|current| {
            if version.version == current.version {
                *version == current
            } else {
                (1..current.version).contains(&version.version)
                    && version.established_by < current.established_by
            }
        });
    if !known {
        return Err(CanwuError::new(
            ErrorCode::EvidenceUnavailable,
            "resource exact completion evidence version is unknown",
        ));
    }
    Ok(())
}

/// Whether `version` is its record's current version. Evidence existence
/// cannot decide this: a seal keeps a current version's receipt but not
/// necessarily its boundary's, so it can report that version missing.
fn is_current_version(
    view: &SimulationView<'_>,
    version: &canwu_api::DomainRecordVersionRef,
) -> Result<bool, CanwuError> {
    Ok(view
        .current_domain_record_version(&version.record)?
        .as_ref()
        == Some(version))
}

fn require_current_version(
    view: &SimulationView<'_>,
    version: &canwu_api::DomainRecordVersionRef,
) -> Result<(), CanwuError> {
    if !is_current_version(view, version)? {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "resource locked external target is not the current exact version",
        ));
    }
    Ok(())
}

/// The body of `version`, which must be its record's current version. The
/// live record is that body, while retained evidence holds it only until its
/// establishing boundary is sealed.
fn current_version_body(
    view: &SimulationView<'_>,
    version: &canwu_api::DomainRecordVersionRef,
) -> Result<DomainRecord, CanwuError> {
    require_current_version(view, version)?;
    live_body(view, version)
}

fn live_body(
    view: &SimulationView<'_>,
    version: &canwu_api::DomainRecordVersionRef,
) -> Result<DomainRecord, CanwuError> {
    view.domain_record(&version.record)?
        .cloned()
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidDomainRecord,
                "resource exact source record is unavailable",
            )
        })
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
        ResourceOperationRequestV1::ConsumeBatch(value) => {
            value.consumer_evidence == packet.provider_source
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
        ResourceOperationRequestV1::RecordLoss(value) => matches!(
            &value.cause,
            canwu_api::EvidenceRef::DomainRecordVersion(cause) if cause == &packet.provider_source
        ),
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
            ) || matches!(
                &value.disposition,
                crate::ResourceTransferDispositionV1::AcceptLocal { handover_evidence, .. }
                    if handover_evidence == &packet.provider_source
            )
        }
        ResourceOperationRequestV1::CreateAccount(_)
        | ResourceOperationRequestV1::SubmitDemand(_)
        | ResourceOperationRequestV1::AmendDemand(_)
        | ResourceOperationRequestV1::Allocate(_)
        | ResourceOperationRequestV1::BeginTransfer(_)
        | ResourceOperationRequestV1::BeginExchange(_)
        | ResourceOperationRequestV1::CancelTransfer(_)
        | ResourceOperationRequestV1::SetProtectedFloor(_)
        | ResourceOperationRequestV1::CancelDemand(_)
        | ResourceOperationRequestV1::Completion(_)
        | ResourceOperationRequestV1::IssueAccessGrant(_)
        | ResourceOperationRequestV1::RevokeAccessGrant(_) => false,
    }
}

/// Checks that the command's subject controls the holder its target names in
/// `state`, which holds every hot terminal record the command names.
fn validate_resource_authority(
    view: &SimulationView<'_>,
    context: &CommandContext,
    value: &ResourceCommandV1,
    state: &crate::ResourceState,
) -> Result<(), CanwuError> {
    validate_resource_subject_issuer(context, &value.subject)?;
    let target_holder = match &value.request {
        ResourceOperationRequestV1::CreateAccount(request) => {
            if request.account.place_scope.is_some() {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "resource account place scope is host-declared and cannot be set by a tracked command",
                ));
            }
            Some(request.account.custodian.clone())
        }
        ResourceOperationRequestV1::SubmitDemand(request) => {
            state
                .validate_demand_admission(&request.demand, false)
                .map_err(|error| CanwuError::new(ErrorCode::InvalidAuthority, error.to_string()))?;
            Some(request.demand.requester.clone())
        }
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
            let drawn = current.fulfilled > 0
                || state
                    .reservation_by_demand
                    .get(&current.id)
                    .is_some_and(|ids| !ids.is_empty());
            state
                .validate_demand_admission(&request.replacement, drawn)
                .map_err(|error| CanwuError::new(ErrorCode::InvalidAuthority, error.to_string()))?;
            Some(current.requester.clone())
        }
        // A leg reserved under an access grant is debited by the grantee;
        // any other leg by its source custodian.
        ResourceOperationRequestV1::BeginTransfer(request) => {
            state.allocation_debit_holder(&request.allocation)
        }
        // The subject initiates through `leg_a`; settlement additionally
        // requires each leg's completion lease to be held by that leg's own
        // source custodian (or grantee for a granted leg), so the
        // counterparty's consent is its own lease.
        ResourceOperationRequestV1::BeginExchange(request) => {
            state.allocation_debit_holder(&request.leg_a.allocation)
        }
        // The applied outcome keeps this citation as exact evidence, which
        // resource restore checks against full history. Only the current
        // version is both provable here without retained evidence and certain
        // to exist in that history.
        ResourceOperationRequestV1::IssueAccessGrant(request) => {
            if !is_current_version(view, &request.grant.authority_evidence)? {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "resource access grant authority evidence is not the current exact version of its record",
                ));
            }
            Some(request.grant.grantor_custodian.clone())
        }
        ResourceOperationRequestV1::RevokeAccessGrant(request) => state
            .access_grants
            .get(&request.grant_id)
            .map(|record| record.grant.grantor_custodian.clone()),
        ResourceOperationRequestV1::RecordLoss(request) => state
            .accounts
            .get(&request.account)
            .map(|account| account.custodian.clone()),
        ResourceOperationRequestV1::CancelTransfer(request) => {
            let transfer = state.transfers.get(&request.transfer).ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "resource transfer target is unavailable",
                )
            })?;
            state.transfer_controller(transfer)
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
            match &request.disposition {
                crate::ResourceTransferDispositionV1::Accept { destination, .. }
                | crate::ResourceTransferDispositionV1::AcceptLocal { destination, .. } => state
                    .accounts
                    .get(destination)
                    .map(|account| account.custodian.clone()),
                crate::ResourceTransferDispositionV1::Lose { .. }
                | crate::ResourceTransferDispositionV1::Return { .. }
                | crate::ResourceTransferDispositionV1::ExternalOutflow { .. } => {
                    state.transfer_controller(transfer)
                }
            }
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
        // The holder of the lease in the root, not the holder the request
        // names: an abort leaves an outcome under the lease's abort key, so
        // only the lease's own holder may send one.
        ResourceOperationRequestV1::Completion(crate::ResourceCompletionOperationV1::Abort(
            request,
        )) => state
            .completion_leases
            .acquisitions
            .get(&request.acquisition)
            .map(|acquisition| acquisition.holder.clone()),
        ResourceOperationRequestV1::Allocate(_)
        | ResourceOperationRequestV1::Consume(_)
        | ResourceOperationRequestV1::ConsumeBatch(_)
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

/// Refuses a tracked `Acquire` that its settlement would refuse for what the
/// view holds now.
///
/// Its packet settles at the instant the command is admitted, so the
/// acquisition must be eligible at that instant, and the evidence its
/// envelope persists must be the current version of its record. Settlement
/// checks both again, with the report reservation, as the completion ingress
/// does; a packet that fails them there settles nothing.
fn validate_tracked_acquisition(
    view: &SimulationView<'_>,
    context: &CommandContext,
    request: &ResourceOperationRequestV1,
) -> Result<(), CanwuError> {
    let ResourceOperationRequestV1::Completion(crate::ResourceCompletionOperationV1::Acquire(
        acquisition,
    )) = request
    else {
        return Ok(());
    };
    if acquisition.eligibility_time != context.simulation_time {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "a tracked completion acquisition must be eligible at the time it is admitted",
        ));
    }
    let envelope = &acquisition.eligibility_envelope;
    for version in envelope
        .exact_evidence
        .iter()
        .chain(&envelope.capability_bindings)
        .chain(&envelope.route_evidence)
    {
        require_current_version(view, version)?;
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
    let buckets = records.iter().filter(|record| {
        record
            .reference
            .kind
            .matches_type::<crate::ResourceBucketRecord>()
    });
    match resource_records.first() {
        // The whole root: the live root, every bucket, and the ledger that
        // counts their entries, validated as one state.
        Some(record) => {
            crate::ResourceState::from_records(record, buckets)?;
        }
        None => {
            if buckets.count() != 0 {
                return Err(CanwuError::new(
                    ErrorCode::InvalidDomainRecord,
                    "resource hot terminal buckets exist without a resource runtime root",
                ));
            }
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
        reads: vec![
            StateKey::core_domain_records(),
            StateKey::core_evidence(),
            resource_state_key(),
            resource_hot_terminal_state_key(),
        ],
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
    reject_adapter_production_credit(&packet.request)?;
    reject_reserved_operation_key(&packet.request)?;
    Ok(PluginIngressRequest::new(
        PLUGIN_NAME,
        RESOURCE_ADAPTER_INGRESS,
        due_at,
        encode(packet)?,
    ))
}

/// Queues a resource adapter operation whose provider source version
/// exists. Takes either runtime handle, [`canwu_api::Canwu`] or [`canwu_api::CompactedCanwu`].
///
/// On a [`canwu_api::CompactedCanwu`], whether a cited superseded version that
/// no dependency declares still exists depends on the receipts earlier seals
/// kept; see [`canwu_api::CanwuRun::domain_record_version_evidence_exists`].
pub fn enqueue_resource_adapter_operation<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &mut H,
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
///
/// The requester must be an entity of the run: the settlement and skip
/// events name it, and an event naming a missing entity would fail its
/// boundary. A requester that is not is refused here.
pub fn enqueue_resource_allocation<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &mut H,
    due_at: SimTime,
    requester: &canwu_api::KnowledgeHolderRef,
    request: &crate::ResourceAllocationRequestV1,
) -> Result<canwu_api::IngressReceipt, CanwuError> {
    reject_reserved_operation_key(&ResourceOperationRequestV1::Allocate(request.clone()))?;
    let entity = holder_entity(requester);
    if !canwu.entity_exists(&entity) {
        return Err(CanwuError::new(
            ErrorCode::EntityNotFound,
            "resource allocation requester is not an entity of the run",
        )
        .with_entity(entity));
    }
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
/// canonical ingress. This is the host application's path for the grant,
/// prepare, activate, expiry, and release operations and for the single-step
/// lease, none of which a tracked command can carry.
///
/// It is not the only source of such packets. A plugin whose boundary system
/// declares this ingress among its `plugin_ingress_targets`, as
/// `canwu-production` does, can schedule the same operations, the single step
/// included. The resource plugin trusts those packets as it trusts the host
/// application's.
///
/// An operation whose identities do not form a valid operation key
/// ([`crate::ResourceCompletionOperationV1::has_canonical_operation_key`]) is
/// refused here. One scheduled around this call settles as a recorded
/// rejection.
pub fn enqueue_resource_completion_operation<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &mut H,
    due_at: SimTime,
    operation: &crate::ResourceCompletionOperationV1,
) -> Result<canwu_api::IngressReceipt, CanwuError> {
    reject_invalid_completion_identity(operation)?;
    // An expiry names the boundary that settles it, which a host cannot know
    // when it queues the packet: other ingress may open boundaries first, at
    // the same instant or before it. The expiry tick names no boundary; the
    // boundary that settles it derives the expiry from its own identity.
    if matches!(
        operation,
        crate::ResourceCompletionOperationV1::Expire(_)
            | crate::ResourceCompletionOperationV1::ExpireExternalParticipants(_)
    ) {
        return Err(CanwuError::new(
            ErrorCode::InvalidPayload,
            "a completion expiry names its settling boundary, which a host cannot know when it queues it; queue enqueue_resource_completion_expiry_tick instead",
        ));
    }
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
/// with one host-retained authenticated directory root. The batch must equal
/// what [`crate::ResourceState::prepare_resource_archive`] derives from the
/// current resource root and `store`, including its lookup index pages.
pub fn enqueue_resource_archive<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &mut H,
    prepared: &crate::PreparedResourceArchiveBatchV1,
    store: &dyn crate::ResourceArchiveStore,
) -> Result<ResourceArchiveIngressReceiptV1, CanwuError> {
    let (_, state) = crate::resource_state(canwu)?
        .ok_or_else(|| CanwuError::new(ErrorCode::PluginNotActive, "resource runtime is absent"))?;
    let canonical = state
        .prepare_resource_archive(prepared.selected.len(), store)
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
/// state records an applied or stale terminal archive disposition, and queues
/// the acknowledgement that retires the state-side handle and every stale
/// receipt recorded under it. The same batch queued twice has one handle:
/// either of its ingress receipts finalizes it.
///
/// The disposition is read from the state-side retention handle, which stays
/// in `archive_retention_handles` until this acknowledgement settles and
/// records how the commit settled: `Committed` once any commit of the batch
/// applied, `RejectedStale` otherwise. The call may therefore come at any
/// time after the commit settled, also after a later commit archived the
/// commit's maintenance receipt; the acknowledgement then names the handle
/// instead of the receipt. Once the handle is gone, an applied commit whose
/// receipt is still in the root is finalized again from that receipt, which
/// settles as a no-op. A call fails only when the commit has not settled, or
/// when both the handle and the receipt are gone because an earlier call was
/// acknowledged and a later commit archived the receipt. No call fails a
/// boundary, and an acknowledgement may settle twice, or behind a commit that
/// archives the receipt, without harm.
pub fn finalize_resource_archive_retention<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &mut H,
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
        .cloned();
    let handle_phase = state
        .archive_retention_handles
        .get(&receipt.retention_handle_id)
        .filter(|handle| handle.directory_root == receipt.directory_root)
        .map(|handle| handle.phase)
        .filter(|phase| {
            matches!(
                phase,
                crate::ResourceArchiveRetentionPhaseV1::Committed
                    | crate::ResourceArchiveRetentionPhaseV1::RejectedStale
            )
        });
    let receipt_phase = terminal
        .as_ref()
        .map(|terminal| match terminal.disposition {
            crate::ResourceArchiveMaintenanceDispositionV1::Applied => {
                crate::ResourceArchiveRetentionPhaseV1::Committed
            }
            crate::ResourceArchiveMaintenanceDispositionV1::RejectedStale => {
                crate::ResourceArchiveRetentionPhaseV1::RejectedStale
            }
        });
    let phase = handle_phase.or(receipt_phase).ok_or_else(|| {
        CanwuError::new(
            ErrorCode::InvalidArchive,
            "resource archive terminal disposition is unavailable",
        )
    })?;
    store
        .finalize_resource_archive_retention(&receipt.retention_handle_id, phase)
        .map_err(resource_canwu_error)?;
    let permit = RESOURCE_ARCHIVE_RETENTION_ACK_PERMIT.get().ok_or_else(|| {
        CanwuError::new(
            ErrorCode::PluginNotActive,
            "resource plugin must be active before archive acknowledgement",
        )
    })?;
    let acknowledgement = match terminal {
        Some(terminal) => ResourceArchiveRetentionAcknowledgementV1::Receipt(terminal),
        None => ResourceArchiveRetentionAcknowledgementV1::Handle {
            retention_handle_id: receipt.retention_handle_id.clone(),
            directory_root: receipt.directory_root.clone(),
        },
    };
    canwu.enqueue_permitted_plugin_ingress(
        PluginIngressRequest::new(
            PLUGIN_NAME,
            RESOURCE_ARCHIVE_RETENTION_ACK_INGRESS,
            canwu.time(),
            encode(&acknowledgement)?,
        ),
        permit,
    )
}

/// Queues one completion expiry tick: the next boundary expires every
/// completion grant and external participant grant due by then, and cleans
/// up aborting acquisitions.
///
/// The resource plugin schedules these ticks itself while expiry is pending.
/// An expiry whose result does not fit the root is skipped with a
/// [`crate::RESOURCE_INGRESS_SKIPPED_EVENT`] and stops that schedule, since a
/// tick at the same instant would meet the same root. The next applied
/// archive commit, which makes room for it, schedules a tick while an expiry
/// is due; a host application may also queue one with this function.
pub fn enqueue_resource_completion_expiry_tick<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &mut H,
    due_at: SimTime,
) -> Result<canwu_api::IngressReceipt, CanwuError> {
    canwu.enqueue_plugin_ingress(PluginIngressRequest::new(
        PLUGIN_NAME,
        RESOURCE_COMPLETION_EXPIRY_TICK_INGRESS,
        due_at,
        serde_json::json!({ "format_version": 1 }),
    ))
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
        retention: canwu_api::KnowledgeRetention::Keep,
    }
}

fn resource_state_key() -> StateKey {
    StateKey::new(
        ResourceRuntimeRecord::NAMESPACE,
        ResourceRuntimeRecord::NAME,
    )
}

/// The state key of the hot terminal buckets, which every reader of the
/// resource root declares beside the live root's (`canwu.resource`
/// `runtime`).
#[must_use]
pub fn resource_hot_terminal_state_key() -> StateKey {
    DomainRecordSchema::for_record::<crate::ResourceBucketRecord>().state_key()
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
