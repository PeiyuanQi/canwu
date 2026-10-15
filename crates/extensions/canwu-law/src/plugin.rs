use crate::LegalRuntime;
use crate::{
    LAW_SEMANTIC_HASH, LegalArchiveHeadStateRecord, LegalArchiveStore, LegalDirectoryStateRecord,
    LegalPlanState, LegalPlanStateRecord, LegalShardKey, LegalShardStateRecord, PLUGIN_NAME,
    PreparedLegalArchiveBatch, VerifiedLegalArchiveCommit, legal_archive_head_state_reference,
    legal_directory_state_reference, legal_plan_state_reference, legal_shard_state_reference,
};
use canwu_api::{
    ArchiveReachabilityManifest, BoundaryContext, BoundaryDirective, BoundaryPhase,
    BoundaryProposal, CanwuError, CauseRef, Command, CommandContext, DecisionOrigin,
    DecisionRequestId, DecisionTicketId, DomainRecord, DomainRecordMutation, DomainRecordRef,
    DomainRecordSchema, EntityRef, ErrorCode, EvidenceRef, IDENTITY_EVIDENCE_DEPENDENCIES_FIELD,
    IngressClass, IngressPayload, KnowledgeHolderRef, OwnerAuthorizedMaintenanceRequest,
    OwnerAuthorizedParticipantDraft, OwnerAuthorizedParticipantRole,
    OwnerAuthorizedRecordExpectation, PayloadProperty, PayloadSchema, PayloadValueType,
    PluginActionDescriptor, PluginArchiveObjectProvider, PluginArchiveRetention,
    PluginIngressDescriptor, PluginIngressPermit, PluginRegistrar, SimulationPlugin,
    SimulationView, StateKey, StateVisibility, SystemCadence, SystemDirective, canonical_hash,
    identity_evidence_dependencies_property_v1,
};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

pub const LAW_COMMAND: &str = "submit_pending_intent";
/// Removed Format-7 aggregate kind. It remains named so loaders can reject it
/// with a precise migration error instead of treating it as an unknown plugin.
pub const LAW_RUNTIME_STATE: &str = "runtime";
pub const LAW_PLAN_STATE: &str = "plan_state";
pub const LAW_DIRECTORY_STATE: &str = "directory_state";
pub const LAW_SHARD_STATE: &str = "shard_state";
pub const LAW_ARCHIVE_HEAD_STATE: &str = "archive_head_state";
pub const LAW_INTENT_INGRESS: &str = "pending_legal_intent";
pub const LAW_ACTOR_CONTEXT_INGRESS: &str = "legal_actor_context";
pub const LAW_OUTBOX_ACK_INGRESS: &str = "legal_outbox_enqueued";
pub const LAW_OUTBOX_PREPARE_INGRESS: &str = "prepare_legal_outbox_enqueue";
pub const LAW_MUTATION_INGRESS: &str = "legal_mutation";
pub const LAW_WAKE_INGRESS: &str = "legal_due_work";
pub const LAW_ARCHIVE_COMMIT_INGRESS: &str = "legal_archive_commit";
pub const LAW_ARCHIVE_RETENTION_ACK_INGRESS: &str = "legal_archive_retention_ack";
/// Event the law admission system records when it refuses one admitted law
/// packet instead of failing the boundary. Its summary names the packet type,
/// the ingress ID, and the reason; the packet changes nothing else.
pub const LAW_INGRESS_REJECTED_EVENT: &str = "canwu.law.ingress_rejected.v1";
/// Event the law admission system records when it rejects a pending proposal
/// whose host compare-and-set no longer holds or that depends on a retired
/// culture generation. The proposal is `Rejected` and its procedure closed.
pub const LAW_PROPOSAL_REJECTED_EVENT: &str = "canwu.law.proposal_rejected.v1";
const LAW_ADMISSION_SYSTEM: &str = "admit_legal_ingress";
static LAW_ARCHIVE_INGRESS_PERMIT: OnceLock<PluginIngressPermit> = OnceLock::new();
static LAW_ARCHIVE_RETENTION_ACK_PERMIT: OnceLock<PluginIngressPermit> = OnceLock::new();

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LegalArchiveIngressReceipt {
    pub ingress: canwu_api::IngressReceipt,
    pub retention_handle_id: String,
    pub compaction_token: String,
    pub directory_root: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OutboxAcknowledgementAdmission {
    expected_revision: u64,
    ingress_id: canwu_api::IngressId,
    controller_request_id: Option<u64>,
    create_request_id: u64,
    ticket_id: u64,
    draft_hash: String,
    outcome_commitment: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ArchiveRetentionAcknowledgementAdmission {
    compaction_token: String,
    directory_root: String,
    disposition: crate::LegalArchiveMaintenanceDisposition,
    chain_root: String,
}

impl OutboxAcknowledgementAdmission {
    fn semantically_matches(&self, other: &Self) -> bool {
        self.expected_revision == other.expected_revision
            && self.controller_request_id == other.controller_request_id
            && self.create_request_id == other.create_request_id
            && self.ticket_id == other.ticket_id
            && self.draft_hash == other.draft_hash
            && self.outcome_commitment == other.outcome_commitment
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct LawPlugin;

impl SimulationPlugin for LawPlugin {
    fn name(&self) -> &'static str {
        PLUGIN_NAME
    }
    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }
    fn semantic_hash(&self) -> &'static str {
        LAW_SEMANTIC_HASH
    }

    fn validate_activation(&self, records: &[DomainRecord]) -> Result<(), CanwuError> {
        validate_law_activation_records(records)
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let schemas = law_record_schemas();
        for schema in &schemas {
            registrar.register_record_schema(schema.clone())?;
        }
        registrar.register_maintenance_dependency_resolver("canwu.culture")?;
        registrar.register_owner_authorized_maintenance_participant(law_maintenance_participant)?;
        registrar.register_archive_reachability_participant(law_archive_reachability)?;
        registrar.register_ingress(PluginIngressDescriptor {
            name: LAW_ACTOR_CONTEXT_INGRESS.to_owned(),
            description: "Derive one legal seat context from holder-relative knowledge".to_owned(),
            class: IngressClass::Information,
            payload_schema: PayloadSchema::Any,
        })?;
        let archive_permit = registrar.register_internal_ingress_with_archive_retention(
            PluginIngressDescriptor {
                name: LAW_ARCHIVE_COMMIT_INGRESS.to_owned(),
                description: "Commit one provider-verified legal cold-archive batch".to_owned(),
                class: IngressClass::ScheduledSystem,
                payload_schema: PayloadSchema::Any,
            },
            crate::LEGAL_ARCHIVE_INDEX_DIRECTORY_NAMESPACE,
            vec![
                "/commit/archive_head/membership_root".to_owned(),
                "/commit/pending_reachability/directory_root".to_owned(),
            ],
        )?;
        if LAW_ARCHIVE_INGRESS_PERMIT
            .set(archive_permit.clone())
            .is_err()
            && LAW_ARCHIVE_INGRESS_PERMIT.get() != Some(&archive_permit)
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                "law archive ingress permit changed across registrations",
            ));
        }
        let retention_ack_permit =
            registrar.register_internal_ingress(PluginIngressDescriptor {
                name: LAW_ARCHIVE_RETENTION_ACK_INGRESS.to_owned(),
                description:
                    "Acknowledge store-side finalization of one legal archive retention handle"
                        .to_owned(),
                class: IngressClass::Acknowledgement,
                payload_schema: PayloadSchema::Any,
            })?;
        if LAW_ARCHIVE_RETENTION_ACK_PERMIT
            .set(retention_ack_permit.clone())
            .is_err()
            && LAW_ARCHIVE_RETENTION_ACK_PERMIT.get() != Some(&retention_ack_permit)
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                "law archive retention acknowledgement permit changed across registrations",
            ));
        }
        registrar.register_ingress(PluginIngressDescriptor {
            name: LAW_MUTATION_INGRESS.to_owned(),
            description: "Apply one plan-bound legal mutation at an atomic boundary".to_owned(),
            class: IngressClass::ScheduledSystem,
            payload_schema: PayloadSchema::Any,
        })?;
        registrar.register_ingress(PluginIngressDescriptor {
            name: LAW_WAKE_INGRESS.to_owned(),
            description: "Advance indexed legal deadline or effective-time work".to_owned(),
            class: IngressClass::ScheduledSystem,
            payload_schema: PayloadSchema::Any,
        })?;
        registrar.register_ingress(PluginIngressDescriptor {
            name: LAW_INTENT_INGRESS.to_owned(),
            description: "Admit one authority-checked legal decision intent".to_owned(),
            class: IngressClass::ScheduledSystem,
            payload_schema: PayloadSchema::Any,
        })?;
        registrar.register_ingress(PluginIngressDescriptor {
            name: LAW_OUTBOX_ACK_INGRESS.to_owned(),
            description: "Persist successful host enqueue of one legal outbox item".to_owned(),
            class: IngressClass::Acknowledgement,
            payload_schema: PayloadSchema::Any,
        })?;
        registrar.register_ingress(PluginIngressDescriptor {
            name: LAW_OUTBOX_PREPARE_INGRESS.to_owned(),
            description: "Persist the revision for a later legal decision enqueue".to_owned(),
            class: IngressClass::Information,
            payload_schema: PayloadSchema::Any,
        })?;
        registrar.register_command(law_command_descriptor(), submit_pending_intent)?;
        let persisted_states = law_record_schemas()
            .into_iter()
            .map(|schema| schema.state_key())
            .collect::<Vec<_>>();
        let mut admission = canwu_api::BoundarySystemContract::new(
            LAW_ADMISSION_SYSTEM,
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        admission.reads = vec![
            StateKey::core_commands(),
            StateKey::core_ingress(),
            StateKey::core_knowledge(),
            StateKey::core_decisions(),
            StateKey::core_domain_records(),
            StateKey::core_evidence(),
        ];
        admission.reads.extend(persisted_states.iter().cloned());
        admission.writes = persisted_states;
        admission.visibility = StateVisibility::SameBoundary;
        admission.emits = vec![
            LAW_INGRESS_REJECTED_EVENT.to_owned(),
            LAW_PROPOSAL_REJECTED_EVENT.to_owned(),
        ];
        registrar.register_boundary_system(admission, admit_legal_ingress)
    }
}

struct PluginLegalArchiveProvider<'a>(&'a dyn PluginArchiveObjectProvider);

impl PluginLegalArchiveProvider<'_> {
    fn load<T: DeserializeOwned>(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<T>, CanwuError> {
        self.0
            .load_plugin_archive_object(namespace, object_id)?
            .map(|bytes| {
                serde_json::from_slice(&bytes).map_err(|error| {
                    CanwuError::new(
                        ErrorCode::InvalidSnapshot,
                        format!("plugin archive object cannot be decoded: {error}"),
                    )
                })
            })
            .transpose()
    }
}

impl crate::LegalArchiveProvider for PluginLegalArchiveProvider<'_> {
    fn load_legal_archive(
        &self,
        blob_id: &str,
    ) -> Result<Option<crate::LegalArchiveBlob>, CanwuError> {
        self.load(crate::LEGAL_ARCHIVE_BLOB_NAMESPACE, blob_id)
    }

    fn load_legal_archive_index_directory(
        &self,
        directory_id: &str,
    ) -> Result<Option<crate::LegalArchiveIndexDirectory>, CanwuError> {
        self.load(crate::LEGAL_ARCHIVE_INDEX_DIRECTORY_NAMESPACE, directory_id)
    }

    fn load_legal_archive_membership_page(
        &self,
        page_id: &str,
    ) -> Result<Option<crate::LegalArchiveMembershipPage>, CanwuError> {
        self.load(crate::LEGAL_ARCHIVE_MEMBERSHIP_PAGE_NAMESPACE, page_id)
    }

    fn load_legal_archive_temporal_page(
        &self,
        page_id: &str,
    ) -> Result<Option<crate::LegalArchiveTemporalPage>, CanwuError> {
        self.load(crate::LEGAL_ARCHIVE_TEMPORAL_PAGE_NAMESPACE, page_id)
    }
}

fn law_archive_reachability(
    view: &SimulationView<'_>,
    provider: &dyn PluginArchiveObjectProvider,
    manifest: &mut ArchiveReachabilityManifest,
) -> Result<(), CanwuError> {
    let provider = PluginLegalArchiveProvider(provider);
    let pending_directory_roots = manifest
        .plugin_objects
        .get(crate::LEGAL_ARCHIVE_INDEX_DIRECTORY_NAMESPACE)
        .cloned()
        .unwrap_or_default();
    for directory_root in pending_directory_roots {
        extend_directory_root_reachability(manifest, &provider, &directory_root)?;
    }
    let (plan_record, plan_state, directory_record, directory, plan) =
        load_legal_header_from_view(view)?;
    let scope = directory
        .directory
        .active_shards
        .iter()
        .chain(directory.directory.archive_only_shards.iter())
        .cloned()
        .collect::<BTreeSet<_>>();
    let runtime = load_legal_runtime_from_view_scope(
        view,
        plan_record,
        plan_state,
        directory_record,
        directory,
        &plan,
        &scope,
        false,
    )?;
    runtime.extend_archive_reachability(manifest, &provider)
}

fn extend_directory_root_reachability(
    manifest: &mut ArchiveReachabilityManifest,
    provider: &dyn crate::LegalArchiveProvider,
    directory_root: &str,
) -> Result<(), CanwuError> {
    let reachable = crate::storage::authenticate_legal_archive_root(provider, directory_root)
        .map_err(|error| {
            CanwuError::new(
                if error.code == ErrorCode::InvalidDomainRecord {
                    ErrorCode::InvalidArchive
                } else {
                    error.code
                },
                error.message,
            )
        })?;
    for directory_id in reachable.directory_ids {
        manifest.insert_plugin_object(crate::LEGAL_ARCHIVE_INDEX_DIRECTORY_NAMESPACE, directory_id);
    }
    for page_id in reachable.membership_page_ids {
        manifest.insert_plugin_object(crate::LEGAL_ARCHIVE_MEMBERSHIP_PAGE_NAMESPACE, page_id);
    }
    for page_id in reachable.temporal_page_ids {
        manifest.insert_plugin_object(crate::LEGAL_ARCHIVE_TEMPORAL_PAGE_NAMESPACE, page_id);
    }
    for object in reachable.objects {
        manifest.insert_plugin_object("canwu.law.archive.content", object.content_id);
        manifest.insert_plugin_object(crate::LEGAL_ARCHIVE_BLOB_NAMESPACE, object.blob_id);
    }
    Ok(())
}

fn law_maintenance_participant(
    view: &SimulationView<'_>,
    request: &OwnerAuthorizedMaintenanceRequest,
    role: OwnerAuthorizedParticipantRole,
) -> Result<OwnerAuthorizedParticipantDraft, CanwuError> {
    if role != OwnerAuthorizedParticipantRole::DependentOwner
        || request.payload.get("operation").and_then(Value::as_str) != Some("retire_plugin_state")
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidDomainRecord,
            "law dependency owner did not authorize the requested culture maintenance operation",
        ));
    }
    let plan = view
        .typed_domain_record(&legal_plan_state_reference())?
        .ok_or_else(|| invalid_record("law plan state is missing during maintenance"))?;
    Ok(OwnerAuthorizedParticipantDraft {
        plugin: PLUGIN_NAME.to_owned(),
        role,
        accepted: true,
        rejection_reason: None,
        expected_records: vec![OwnerAuthorizedRecordExpectation {
            record: plan.reference.clone(),
            version: plan.version,
        }],
        mutations: Vec::new(),
    })
}

#[must_use]
pub fn law_command_descriptor() -> PluginActionDescriptor {
    PluginActionDescriptor {
        name: LAW_COMMAND.to_owned(),
        description: "Append one controller-bound pending legal intent".to_owned(),
        payload_schema: PayloadSchema::Object {
            properties: BTreeMap::from([(
                "intent".to_owned(),
                PayloadProperty {
                    value_type: PayloadValueType::Object,
                    required: true,
                },
            )]),
            allow_additional: false,
        },
        reads: Vec::new(),
        writes: Vec::new(),
    }
}

/// Queue one legal mutation through Canwu's canonical ingress.
pub fn enqueue_legal_mutation<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &mut H,
    mutation: &crate::LegalMutation,
) -> Result<canwu_api::IngressReceipt, CanwuError> {
    enqueue_legal_mutation_at(canwu, canwu.time(), mutation)
}

/// Queue one legal mutation for a declared future boundary.
pub fn enqueue_legal_mutation_at<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &mut H,
    at: canwu_api::SimTime,
    mutation: &crate::LegalMutation,
) -> Result<canwu_api::IngressReceipt, CanwuError> {
    canwu.enqueue_plugin_ingress(canwu_api::PluginIngressRequest::new(
        PLUGIN_NAME,
        LAW_MUTATION_INGRESS,
        at,
        serde_json::json!({ "mutation": mutation }),
    ))
}

/// Stores, reads back, verifies, and queues one legal cold-archive transition
/// through plugin-owned canonical ingress. Callers never receive an ingress
/// capability that can serialize an arbitrary commit.
pub fn enqueue_legal_archive<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &mut H,
    prepared: &PreparedLegalArchiveBatch,
    store: &dyn LegalArchiveStore,
) -> Result<LegalArchiveIngressReceipt, CanwuError> {
    let commit = prepared.store_and_verify(store)?;
    let permit = LAW_ARCHIVE_INGRESS_PERMIT.get().ok_or_else(|| {
        CanwuError::new(
            ErrorCode::PluginNotActive,
            "law plugin must be registered before legal archive ingress",
        )
    })?;
    let retention = legal_archive_ingress_retention(&commit);
    store.mark_legal_archive_retention_durable(&commit.retention_handle_id)?;
    let ingress = canwu.enqueue_permitted_plugin_ingress(
        canwu_api::PluginIngressRequest::new(
            PLUGIN_NAME,
            LAW_ARCHIVE_COMMIT_INGRESS,
            canwu.time(),
            serde_json::json!({ "commit": commit }),
        )
        .with_archive_retention(retention),
        permit,
    );
    if ingress.is_err() {
        let _ = store.abandon_legal_archive_retention(&commit.retention_handle_id);
    }
    ingress.map(|ingress| LegalArchiveIngressReceipt {
        ingress,
        retention_handle_id: commit.retention_handle_id,
        compaction_token: commit.compaction.token,
        directory_root: commit.pending_reachability.directory_root,
    })
}

/// Transfers or releases the store-side retention lease after the canonical
/// legal boundary records its terminal disposition. This operation is
/// idempotent and may be replayed after a host restart.
pub fn finalize_legal_archive_retention<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &mut H,
    plan: &crate::CompiledLawPlan,
    store: &dyn LegalArchiveStore,
    receipt: &LegalArchiveIngressReceipt,
) -> Result<canwu_api::IngressReceipt, CanwuError> {
    let runtime = load_legal_runtime(canwu, plan)?
        .ok_or_else(|| CanwuError::new(ErrorCode::PluginNotActive, "legal runtime is absent"))?;
    let terminal = runtime
        .archive_retention_terminal(
            &receipt.retention_handle_id,
            &receipt.compaction_token,
            &receipt.directory_root,
        )?
        .clone();
    match terminal.disposition {
        crate::LegalArchiveMaintenanceDisposition::Applied => store
            .commit_legal_archive_retention(&receipt.retention_handle_id, &receipt.directory_root),
        crate::LegalArchiveMaintenanceDisposition::RejectedStale => {
            store.reject_stale_legal_archive_retention(&receipt.retention_handle_id)
        }
    }?;
    let permit = LAW_ARCHIVE_RETENTION_ACK_PERMIT.get().ok_or_else(|| {
        CanwuError::new(
            ErrorCode::PluginNotActive,
            "law plugin must be registered before legal archive retention acknowledgement",
        )
    })?;
    canwu.enqueue_permitted_plugin_ingress(
        canwu_api::PluginIngressRequest::new(
            PLUGIN_NAME,
            LAW_ARCHIVE_RETENTION_ACK_INGRESS,
            canwu.time(),
            serde_json::json!({
                "retention_handle_id": receipt.retention_handle_id,
                "compaction_token": receipt.compaction_token,
                "directory_root": receipt.directory_root,
                "disposition": terminal.disposition,
                "chain_root": terminal.chain_root,
            }),
        ),
        permit,
    )
}

pub(crate) fn legal_archive_ingress_retention(
    commit: &crate::VerifiedLegalArchiveCommit,
) -> [PluginArchiveRetention; 1] {
    [PluginArchiveRetention {
        namespace: crate::LEGAL_ARCHIVE_INDEX_DIRECTORY_NAMESPACE.to_owned(),
        object_id: commit.pending_reachability.directory_root.clone(),
    }]
}

fn submit_pending_intent(
    _view: &SimulationView<'_>,
    context: &CommandContext,
    payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    if context.decision_controller_id.is_none() || context.attempt_id.is_none() {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "legal intents require a validated decision controller and attempt",
        ));
    }
    let intent_value = payload.get("intent").cloned().ok_or_else(|| {
        CanwuError::new(
            ErrorCode::InvalidPayload,
            "legal intent command requires intent",
        )
    })?;
    let mut intent =
        serde_json::from_value::<crate::PendingLegalIntent>(intent_value).map_err(|error| {
            CanwuError::new(
                ErrorCode::InvalidPayload,
                format!("invalid pending legal intent: {error}"),
            )
        })?;
    if context.authority.seat_id.as_deref() != Some(intent.seat.as_str())
        || context.authority.permission_profile_id.is_none()
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "pending legal intent seat or permission does not match command authority",
        ));
    }
    let controller = holder_for_origin(&context.authority.decision_origin)?;
    if controller != intent.controller {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "pending legal intent holder does not match the decision origin",
        ));
    }
    intent.command = EvidenceRef::Command(context.command_id);
    intent.attempt = context.attempt_id.map(EvidenceRef::CommandAttempt);
    intent.request_id = context.request_id.map(canwu_api::CommandRequestId::get);
    intent.admitted_at = context.simulation_time;

    let value = serde_json::to_value(intent).map_err(|error| {
        CanwuError::new(
            ErrorCode::InvalidPayload,
            format!("pending legal intent cannot be encoded: {error}"),
        )
    })?;
    Ok(vec![SystemDirective::EnqueuePluginIngress {
        after: canwu_api::SimDuration::ZERO,
        packet_type: LAW_INTENT_INGRESS.to_owned(),
        priority: 0,
        payload: value,
        affected: Vec::new(),
    }])
}

/// The law packet types the admission system settles, in no particular order.
const LAW_PACKET_TYPES: [&str; 8] = [
    LAW_INTENT_INGRESS,
    LAW_ACTOR_CONTEXT_INGRESS,
    LAW_MUTATION_INGRESS,
    LAW_WAKE_INGRESS,
    LAW_OUTBOX_PREPARE_INGRESS,
    LAW_OUTBOX_ACK_INGRESS,
    LAW_ARCHIVE_COMMIT_INGRESS,
    LAW_ARCHIVE_RETENTION_ACK_INGRESS,
];

/// One admitted law packet, decoded and checked on its own.
enum LawPacket {
    ArchiveCommit(Box<VerifiedLegalArchiveCommit>),
    RetentionAck {
        handle: String,
        acknowledgement: ArchiveRetentionAcknowledgementAdmission,
    },
    Intent(Box<crate::PendingLegalIntent>),
    ActorContext {
        requirement: Box<crate::LegalActorContextRequirement>,
        query: Box<canwu_api::KnowledgeQuery>,
    },
    Mutation(Box<crate::LegalMutation>),
    Wake(canwu_api::SimTime),
    Prepare {
        sequence: u64,
        expected_revision: u64,
    },
    Ack {
        sequence: u64,
        acknowledgement: OutboxAcknowledgementAdmission,
    },
}

/// One unit of legal work this boundary applies: one packet, or the packets
/// that agree on one wake time, outbox sequence, or retention handle. A
/// refused unit refuses all its packets.
struct LawUnit {
    packets: Vec<(canwu_api::IngressId, &'static str)>,
    work: LawWork,
}

enum LawWork {
    ArchiveCommit(Box<VerifiedLegalArchiveCommit>),
    RetentionAck {
        handle: String,
        acknowledgement: ArchiveRetentionAcknowledgementAdmission,
    },
    Wake(canwu_api::SimTime),
    Mutation {
        ingress: canwu_api::IngressId,
        mutation: Box<crate::LegalMutation>,
    },
    ActorContext {
        requirement: Box<crate::LegalActorContextRequirement>,
        query: Box<canwu_api::KnowledgeQuery>,
    },
    Prepare {
        sequence: u64,
        expected_revision: u64,
    },
    Ack {
        sequence: u64,
        acknowledgement: OutboxAcknowledgementAdmission,
    },
    Intent(Box<crate::PendingLegalIntent>),
}

/// Why a unit failed. A failure found before the unit changed the working
/// runtime needs no rollback; one raised after a runtime operation changed it
/// sends the pass back to the runtime as it was before the first unit.
struct UnitFailure {
    error: CanwuError,
    changed_runtime: bool,
}

impl UnitFailure {
    fn checked(error: CanwuError) -> Self {
        Self {
            error,
            changed_runtime: false,
        }
    }

    fn applied(error: CanwuError) -> Self {
        Self {
            error,
            changed_runtime: true,
        }
    }
}

/// Runs one legal runtime operation and classifies its failure.
///
/// Every runtime operation run here checks its input and reserves its
/// serialized-state growth (always positive) before its first change. A
/// failure that left the reservation where it was therefore changed nothing
/// and is `checked`. A failure after the reservation (a non-procedural source
/// whose adoption fails, or a broken invariant) is `applied` and restarts the
/// pass. Archive commits do not run here: they may change the runtime before
/// they fail, so their failure is always `applied`.
fn run_runtime_op(
    runtime: &mut LegalRuntime,
    op: impl FnOnce(&mut LegalRuntime) -> Result<(), CanwuError>,
) -> Result<(), UnitFailure> {
    let reserved = runtime.reserved_state_bytes;
    op(runtime).map_err(|error| {
        if runtime.reserved_state_bytes == reserved {
            UnitFailure::checked(error)
        } else {
            UnitFailure::applied(error)
        }
    })
}

/// One refused packet, recorded with [`LAW_INGRESS_REJECTED_EVENT`].
struct Refusal {
    ingress: canwu_api::IngressId,
    packet_type: &'static str,
    error: CanwuError,
}

/// Settles every law packet this boundary admits and records the compiled
/// provider packets it admits in the legal provider index.
///
/// A packet whose content is refused (it does not decode, exceeds the
/// boundary's legal ingress budget, conflicts with another packet, fails a
/// compare-and-set, cites evidence or a provider packet the rule does not
/// accept, would exceed a lifetime budget, or is refused by the legal runtime)
/// changes nothing and is recorded with a [`LAW_INGRESS_REJECTED_EVENT`]; the
/// boundary settles the other packets. A refused archive commit also records a
/// `RejectedStale` terminal naming the refusal, so the host finalizes its
/// retention handle. A pending proposal whose host compare-and-set no longer
/// holds, whose culture dependency was retired, whose adoption guard can never
/// hold, or whose adoption would exceed a lifetime budget is rejected and its
/// procedure closed, recorded with a [`LAW_PROPOSAL_REJECTED_EVENT`]. Work past
/// the per-boundary budgets waits for a continuation wake at the same time.
///
/// Every decision reads committed state (the provider index among it), this
/// boundary's admitted packets and the commands that caused them, and the
/// engine's existence answer for cited ordinal evidence; never another
/// retained payload, provenance, or record version body. A sealed run
/// therefore decides as its unsealed replay does, as far as the engine
/// answers ordinal existence alike in both.
///
/// The runtime is loaded once. A refusal found before a unit changed the
/// working runtime costs nothing more; one found after it restarts the pass
/// from a copy taken after loading, without the refused unit.
#[allow(clippy::too_many_lines)]
fn admit_legal_ingress(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut refusals = Vec::new();
    let mut decoded = Vec::new();
    let mut plugin_packets = Vec::new();
    for ingress_id in &context.admitted_ingress {
        // The view shows a law system only law packets; any other admitted
        // packet may be a compiled provider packet.
        let Some(ingress) = view.ingress(*ingress_id)? else {
            plugin_packets.push(*ingress_id);
            continue;
        };
        let IngressPayload::Plugin { packet_type, .. } = &ingress.payload else {
            continue;
        };
        let Some(packet_type) = LAW_PACKET_TYPES
            .iter()
            .copied()
            .find(|known| *known == packet_type.as_str())
        else {
            continue;
        };
        match decode_law_packet(view, context, ingress, packet_type) {
            Ok(packet) => decoded.push((ingress.id, packet_type, packet)),
            Err(error) => refuse(&mut refusals, ingress.id, packet_type, error)?,
        }
    }
    let mut directives = Vec::new();
    let Some(installed_plan) = view.typed_domain_record(&legal_plan_state_reference())? else {
        // Without an installed legal plan there is no legal work: every law
        // packet is refused and no provider packet is recorded.
        for (ingress, packet_type, _) in decoded {
            refuse(
                &mut refusals,
                ingress,
                packet_type,
                invalid_record("no legal plan is installed in this run"),
            )?;
        }
        push_refusal_events(&mut directives, refusals);
        return Ok(BoundaryProposal {
            directives,
            ..BoundaryProposal::default()
        });
    };
    let provider_packets = admitted_provider_packets(view, installed_plan, plugin_packets)?;
    if decoded.is_empty() && provider_packets.is_empty() {
        push_refusal_events(&mut directives, refusals);
        return Ok(BoundaryProposal {
            directives,
            ..BoundaryProposal::default()
        });
    }
    let (plan_record, plan_state, directory_record, directory, plan) =
        load_legal_header_from_view(view)?;
    let units = group_law_units(
        decoded,
        plan.budgets.max_mutations_per_boundary,
        &mut refusals,
    )?;
    let mut selected_shards =
        BTreeSet::from([LegalShardKey::coordinator(plan.definition_id.clone())]);
    let mut selected_units = Vec::new();
    for unit in units {
        let mut scope = BTreeSet::new();
        match select_unit_scope(&unit.work, &plan, &directory, &mut scope)
            .and_then(|()| require_held_shards(&directory, &scope))
        {
            Ok(()) => {
                selected_shards.extend(scope);
                selected_units.push(unit);
            }
            Err(error) => refuse_unit(&mut refusals, &unit, &error)?,
        }
    }
    if selected_units.is_empty() && provider_packets.is_empty() {
        push_refusal_events(&mut directives, refusals);
        return Ok(BoundaryProposal {
            directives,
            ..BoundaryProposal::default()
        });
    }
    let load = |shards: &BTreeSet<LegalShardKey>| -> Result<LegalRuntime, CanwuError> {
        let runtime = load_legal_runtime_from_view_scope(
            view,
            plan_record.clone(),
            plan_state.clone(),
            directory_record.clone(),
            directory.clone(),
            &plan,
            shards,
            true,
        )?;
        runtime.validate_live_plan_binding(&plan)?;
        Ok(runtime)
    };
    let mut runtime = load(&selected_shards)?;
    if !selected_units.is_empty() {
        // Work an earlier boundary deferred stays pending in the coordinator
        // while its records live in their own shards; a boundary that may
        // settle loads them too, at most once more.
        let pending = runtime.pending_work_routes(context.at);
        if !pending.is_subset(&selected_shards) {
            require_held_shards(&directory, &pending)?;
            selected_shards.extend(pending);
            runtime = load(&selected_shards)?;
        }
    }
    let mut changed = false;
    for (id, packet) in provider_packets {
        changed |= runtime.record_provider_packet(id, packet)?;
    }
    // Apply the units in their canonical order. A unit that fails is refused
    // and dropped. A failure found before the unit changed the working
    // runtime leaves it as it was; one found after starts the pass again from
    // the copy taken here, without the refused unit, so a refused unit leaves
    // nothing behind. A refused archive commit records its terminal in place
    // of its work. Each restart drops one more unit.
    let loaded = runtime.clone();
    let mut refused_units = BTreeMap::<usize, String>::new();
    let mut unrecorded_terminals = BTreeSet::new();
    loop {
        let mut restart = false;
        for (index, unit) in selected_units.iter().enumerate() {
            if let Some(reason) = refused_units.get(&index) {
                if let LawWork::ArchiveCommit(commit) = &unit.work
                    && !unrecorded_terminals.contains(&index)
                    && let Err(failure) = run_runtime_op(&mut runtime, |runtime| {
                        runtime.record_refused_archive_commit(commit, reason.clone())
                    })
                {
                    unrecorded_terminals.insert(index);
                    if failure.changed_runtime {
                        restart = true;
                        break;
                    }
                }
                continue;
            }
            if let Err(failure) = apply_law_unit(view, context, &plan, &mut runtime, &unit.work) {
                refuse_unit(&mut refusals, unit, &failure.error)?;
                refused_units.insert(index, failure.error.message.clone());
                if failure.changed_runtime {
                    restart = true;
                    break;
                }
            }
        }
        if !restart {
            break;
        }
        runtime = loaded.clone();
    }
    let settles = refused_units.len() < selected_units.len();
    changed |= settles
        || selected_units.iter().enumerate().any(|(index, unit)| {
            matches!(unit.work, LawWork::ArchiveCommit(_))
                && refused_units.contains_key(&index)
                && !unrecorded_terminals.contains(&index)
        });
    if !changed {
        push_refusal_events(&mut directives, refusals);
        return Ok(BoundaryProposal {
            directives,
            ..BoundaryProposal::default()
        });
    }
    let mut rejected_proposals = Vec::new();
    if settles {
        rejected_proposals = reject_stale_pending_proposals(view, &mut runtime)?;
        let result = runtime.settle_boundary(&plan, context.at, &[])?;
        rejected_proposals.extend(
            result
                .rejected_proposals
                .into_iter()
                .map(|rejection| (rejection.proposal, rejection.reason)),
        );
        // Deferred work continues at a boundary at this same time; due work
        // wakes at its own time. A wake whose tracking does not fit the state
        // budget is not scheduled: its work settles at the next boundary that
        // settles law work.
        let mut wakes = BTreeSet::new();
        if result.deferred {
            wakes.insert(context.at);
        }
        if let Some(due_at) = runtime.next_due_time_after(context.at) {
            wakes.insert(due_at);
        }
        for due_at in wakes {
            if runtime.scheduled_wakes.contains(&due_at) {
                continue;
            }
            let after = due_at.checked_sub(context.at).ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::InvalidBoundary,
                    "legal due time precedes this boundary",
                )
            })?;
            if runtime.mark_wake_scheduled(due_at).is_err() {
                continue;
            }
            directives.push(BoundaryDirective::ScheduleIngress {
                after,
                packet_type: LAW_WAKE_INGRESS.to_owned(),
                priority: 0,
                payload: serde_json::json!({ "due_at": due_at.as_minutes() }),
                affected: Vec::new(),
            });
        }
    }
    let mut state_directives = Vec::new();
    let mut selected_records = BTreeSet::from([
        legal_plan_state_reference().into_untyped(),
        legal_directory_state_reference().into_untyped(),
    ]);
    for shard in &selected_shards {
        selected_records.insert(legal_shard_state_reference(shard)?.into_untyped());
        selected_records.insert(legal_archive_head_state_reference(shard)?.into_untyped());
    }
    for mut draft in runtime.to_record_drafts()? {
        if !selected_records.contains(&draft.reference) {
            continue;
        }
        match view.domain_record(&draft.reference)? {
            Some(record)
                if record.payload == draft.payload && record.references == draft.references => {}
            Some(record) => {
                draft.references.clone_from(&record.references);
                state_directives.push(BoundaryDirective::MutateRecord {
                    mutation: DomainRecordMutation::Update {
                        record: draft,
                        expected_version: record.version,
                    },
                    summary: "Persist one atomic legal state shard".to_owned(),
                });
            }
            None => state_directives.push(BoundaryDirective::MutateRecord {
                mutation: DomainRecordMutation::Create { record: draft },
                summary: "Create one atomic legal state shard".to_owned(),
            }),
        }
    }
    state_directives.extend(directives);
    directives = state_directives;
    push_refusal_events(&mut directives, refusals);
    for (proposal, reason) in rejected_proposals {
        directives.push(BoundaryDirective::Emit {
            event_type: LAW_PROPOSAL_REJECTED_EVENT.to_owned(),
            summary: format!("legal proposal {proposal} rejected: {reason}"),
            affected: Vec::new(),
        });
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

/// The compiled provider packets this boundary admits, each with the payload
/// hash the legal provider index records for it.
///
/// The provider pairs are read from the installed plan record without
/// decoding the plan, so a boundary that admits no other packet pays nothing
/// and one that does pays one record lookup and one engine match per packet
/// and provider. Only a packet the engine matches to its provider plugin's own
/// boundary output is recorded, as the provider rule always required.
fn admitted_provider_packets(
    view: &SimulationView<'_>,
    installed_plan: &DomainRecord,
    candidates: Vec<canwu_api::IngressId>,
) -> Result<Vec<(canwu_api::IngressId, crate::LegalProviderPacket)>, CanwuError> {
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let providers = installed_plan
        .payload
        .pointer("/fields/plan/signal_provider_by_kind")
        .and_then(Value::as_object)
        .map(|providers| {
            providers
                .values()
                .filter_map(|provider| {
                    Some((
                        provider.get("plugin")?.as_str()?,
                        provider.get("packet_type")?.as_str()?,
                    ))
                })
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    let mut packets = Vec::new();
    for id in candidates {
        for (plugin, packet_type) in &providers {
            if let Some(payload_hash) = view.plugin_ingress_payload_hash(
                id,
                plugin,
                packet_type,
                crate::LEGAL_PROVIDER_PAYLOAD_DOMAIN,
            )? {
                packets.push((
                    id,
                    crate::LegalProviderPacket {
                        plugin: (*plugin).to_owned(),
                        packet_type: (*packet_type).to_owned(),
                        payload_hash,
                    },
                ));
                break;
            }
        }
    }
    Ok(packets)
}

/// Records a refusal. A payload read outside what every run holds is an
/// engine-rule violation, not a refusal, so it still fails the boundary.
fn refuse(
    refusals: &mut Vec<Refusal>,
    ingress: canwu_api::IngressId,
    packet_type: &'static str,
    error: CanwuError,
) -> Result<(), CanwuError> {
    if error.code == ErrorCode::EvidenceContentUnavailable {
        return Err(error);
    }
    refusals.push(Refusal {
        ingress,
        packet_type,
        error,
    });
    Ok(())
}

fn refuse_unit(
    refusals: &mut Vec<Refusal>,
    unit: &LawUnit,
    error: &CanwuError,
) -> Result<(), CanwuError> {
    for (ingress, packet_type) in &unit.packets {
        refuse(refusals, *ingress, packet_type, error.clone())?;
    }
    Ok(())
}

fn push_refusal_events(directives: &mut Vec<BoundaryDirective>, mut refusals: Vec<Refusal>) {
    refusals.sort_by_key(|refusal| refusal.ingress);
    for refusal in refusals {
        directives.push(BoundaryDirective::Emit {
            event_type: LAW_INGRESS_REJECTED_EVENT.to_owned(),
            summary: format!(
                "legal {} ingress {} rejected: {}",
                refusal.packet_type, refusal.ingress, refusal.error
            ),
            affected: Vec::new(),
        });
    }
}

fn required_field<'a>(
    payload: &'a Value,
    field: &str,
    context: &str,
) -> Result<&'a Value, CanwuError> {
    payload.get(field).ok_or_else(|| {
        CanwuError::new(
            ErrorCode::InvalidPayload,
            format!("{context} requires {field}"),
        )
    })
}

fn required_text(payload: &Value, field: &str, context: &str) -> Result<String, CanwuError> {
    required_field(payload, field, context)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidPayload,
                format!("{context} requires {field} as text"),
            )
        })
}

fn decode_field<T: DeserializeOwned>(
    payload: &Value,
    field: &str,
    context: &str,
) -> Result<T, CanwuError> {
    serde_json::from_value(required_field(payload, field, context)?.clone()).map_err(|error| {
        CanwuError::new(
            ErrorCode::InvalidPayload,
            format!("{context} has an invalid {field}: {error}"),
        )
    })
}

/// Decodes and checks one admitted law packet on its own.
fn decode_law_packet(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    ingress: &canwu_api::IngressRecord,
    packet_type: &'static str,
) -> Result<LawPacket, CanwuError> {
    let IngressPayload::Plugin {
        payload,
        archive_retention,
        ..
    } = &ingress.payload
    else {
        return Err(CanwuError::new(
            ErrorCode::InvalidPayload,
            "legal ingress is not a plugin packet",
        ));
    };
    match packet_type {
        LAW_ARCHIVE_COMMIT_INGRESS => {
            let commit: VerifiedLegalArchiveCommit =
                decode_field(payload, "commit", "legal archive ingress")?;
            commit.validate()?;
            if archive_retention.as_slice() != legal_archive_ingress_retention(&commit).as_slice() {
                return Err(CanwuError::new(
                    ErrorCode::InvalidPayload,
                    "legal archive ingress retention is not bound to its committed directory root",
                ));
            }
            Ok(LawPacket::ArchiveCommit(Box::new(commit)))
        }
        LAW_ARCHIVE_RETENTION_ACK_INGRESS => {
            let label = "legal archive retention acknowledgement";
            Ok(LawPacket::RetentionAck {
                handle: required_text(payload, "retention_handle_id", label)?,
                acknowledgement: ArchiveRetentionAcknowledgementAdmission {
                    compaction_token: required_text(payload, "compaction_token", label)?,
                    directory_root: required_text(payload, "directory_root", label)?,
                    disposition: decode_field(payload, "disposition", label)?,
                    chain_root: required_text(payload, "chain_root", label)?,
                },
            })
        }
        LAW_INTENT_INGRESS => {
            let intent = serde_json::from_value::<crate::PendingLegalIntent>(payload.clone())
                .map_err(|error| {
                    CanwuError::new(
                        ErrorCode::InvalidPayload,
                        format!("invalid admitted legal intent: {error}"),
                    )
                })?;
            verify_intent_command(view, ingress.cause.as_ref(), &intent)?;
            Ok(LawPacket::Intent(Box::new(intent)))
        }
        LAW_ACTOR_CONTEXT_INGRESS => {
            let label = "legal actor context";
            Ok(LawPacket::ActorContext {
                requirement: Box::new(decode_field(payload, "requirement", label)?),
                query: Box::new(decode_field(payload, "query", label)?),
            })
        }
        LAW_MUTATION_INGRESS => Ok(LawPacket::Mutation(Box::new(decode_field(
            payload,
            "mutation",
            "legal mutation ingress",
        )?))),
        LAW_WAKE_INGRESS => {
            let due_at = payload
                .get("due_at")
                .and_then(Value::as_i64)
                .map(canwu_api::SimTime::from_minutes)
                .ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::InvalidPayload,
                        "legal wake ingress requires due_at",
                    )
                })?;
            if due_at > context.at {
                return Err(CanwuError::new(
                    ErrorCode::InvalidPayload,
                    "legal wake cannot be admitted before its due time",
                ));
            }
            Ok(LawPacket::Wake(due_at))
        }
        LAW_OUTBOX_PREPARE_INGRESS => {
            let (sequence, expected_revision) =
                sequence_and_revision(payload, "legal outbox preparation")?;
            Ok(LawPacket::Prepare {
                sequence,
                expected_revision,
            })
        }
        LAW_OUTBOX_ACK_INGRESS => {
            let label = "legal outbox acknowledgement";
            let (sequence, expected_revision) = sequence_and_revision(payload, label)?;
            Ok(LawPacket::Ack {
                sequence,
                acknowledgement: OutboxAcknowledgementAdmission {
                    expected_revision,
                    ingress_id: ingress.id,
                    controller_request_id: payload_optional_u64(
                        payload,
                        "controller_request_id",
                        label,
                    )?,
                    create_request_id: payload_u64(payload, "create_request_id", label)?,
                    ticket_id: payload_u64(payload, "ticket_id", label)?,
                    draft_hash: required_text(payload, "draft_hash", label)?,
                    outcome_commitment: required_text(payload, "outcome_commitment", label)?,
                },
            })
        }
        _ => Err(CanwuError::new(
            ErrorCode::InvalidPayload,
            "unknown legal ingress packet type",
        )),
    }
}

/// Groups decoded packets into units in their canonical order: archive
/// commits by token, retention acknowledgements by handle, wakes by due time,
/// mutations in admission order, actor contexts by seat, outbox preparations
/// and acknowledgements by sequence, intents by ID.
///
/// Packets count against the boundary's legal ingress budget in admission
/// order; one beyond it is refused. A wake carries no input of its own and
/// does not count. A packet that repeats an outbox sequence or retention
/// handle joins its unit, and one that conflicts with it is refused.
#[allow(clippy::too_many_lines)]
fn group_law_units(
    decoded: Vec<(canwu_api::IngressId, &'static str, LawPacket)>,
    budget: usize,
    refusals: &mut Vec<Refusal>,
) -> Result<Vec<LawUnit>, CanwuError> {
    let mut counted = 0_usize;
    let mut within_budget = |refusals: &mut Vec<Refusal>,
                             ingress: canwu_api::IngressId,
                             packet_type: &'static str|
     -> Result<bool, CanwuError> {
        if counted >= budget {
            refuse(
                refusals,
                ingress,
                packet_type,
                CanwuError::new(
                    ErrorCode::ValueOutOfRange,
                    "legal ingress mutation budget exhausted",
                ),
            )?;
            return Ok(false);
        }
        counted += 1;
        Ok(true)
    };
    let mut commits = Vec::new();
    let mut retention_acks = BTreeMap::<
        String,
        (
            Vec<canwu_api::IngressId>,
            ArchiveRetentionAcknowledgementAdmission,
        ),
    >::new();
    let mut wakes = BTreeMap::<canwu_api::SimTime, Vec<canwu_api::IngressId>>::new();
    let mut mutations = Vec::new();
    let mut contexts = Vec::new();
    let mut preparations = BTreeMap::<u64, (Vec<canwu_api::IngressId>, u64)>::new();
    let mut acknowledgements =
        BTreeMap::<u64, (Vec<canwu_api::IngressId>, OutboxAcknowledgementAdmission)>::new();
    let mut intents = Vec::new();
    for (ingress, packet_type, packet) in decoded {
        match packet {
            LawPacket::Wake(due_at) => wakes.entry(due_at).or_default().push(ingress),
            LawPacket::RetentionAck {
                handle,
                acknowledgement,
            } => match retention_acks.get_mut(&handle) {
                Some((packets, existing)) if *existing == acknowledgement => packets.push(ingress),
                Some(_) => refuse(
                    refusals,
                    ingress,
                    packet_type,
                    CanwuError::new(
                        ErrorCode::IdempotencyConflict,
                        "conflicting legal archive retention acknowledgements share one handle",
                    ),
                )?,
                None => {
                    if within_budget(refusals, ingress, packet_type)? {
                        retention_acks.insert(handle, (vec![ingress], acknowledgement));
                    }
                }
            },
            LawPacket::Prepare {
                sequence,
                expected_revision,
            } => match preparations.get_mut(&sequence) {
                Some((packets, existing)) if *existing == expected_revision => {
                    packets.push(ingress);
                }
                Some(_) => refuse(
                    refusals,
                    ingress,
                    packet_type,
                    CanwuError::new(
                        ErrorCode::IdempotencyConflict,
                        "conflicting legal outbox preparations share one sequence",
                    ),
                )?,
                None => {
                    if within_budget(refusals, ingress, packet_type)? {
                        preparations.insert(sequence, (vec![ingress], expected_revision));
                    }
                }
            },
            LawPacket::Ack {
                sequence,
                acknowledgement,
            } => match acknowledgements.get_mut(&sequence) {
                Some((packets, existing)) if existing.semantically_matches(&acknowledgement) => {
                    packets.push(ingress);
                    if acknowledgement.ingress_id < existing.ingress_id {
                        existing.ingress_id = acknowledgement.ingress_id;
                    }
                }
                Some(_) => refuse(
                    refusals,
                    ingress,
                    packet_type,
                    CanwuError::new(
                        ErrorCode::IdempotencyConflict,
                        "conflicting legal outbox acknowledgements share one sequence",
                    ),
                )?,
                None => {
                    if within_budget(refusals, ingress, packet_type)? {
                        acknowledgements.insert(sequence, (vec![ingress], acknowledgement));
                    }
                }
            },
            packet => {
                if !within_budget(refusals, ingress, packet_type)? {
                    continue;
                }
                match packet {
                    LawPacket::ArchiveCommit(commit) => commits.push((ingress, commit)),
                    LawPacket::Mutation(mutation) => mutations.push((ingress, mutation)),
                    LawPacket::ActorContext { requirement, query } => {
                        contexts.push((ingress, requirement, query));
                    }
                    LawPacket::Intent(intent) => intents.push((ingress, intent)),
                    LawPacket::Wake(_)
                    | LawPacket::RetentionAck { .. }
                    | LawPacket::Prepare { .. }
                    | LawPacket::Ack { .. } => {
                        unreachable!("grouped packet kinds are handled above")
                    }
                }
            }
        }
    }
    commits.sort_by(|left, right| left.1.compaction.token.cmp(&right.1.compaction.token));
    contexts.sort_by(|left, right| {
        (
            &left.1.procedure,
            left.1.stage,
            left.1.round,
            &left.1.seat,
            left.0,
        )
            .cmp(&(
                &right.1.procedure,
                right.1.stage,
                right.1.round,
                &right.1.seat,
                right.0,
            ))
    });
    intents.sort_by(|left, right| (&left.1.id, left.0).cmp(&(&right.1.id, right.0)));
    let mut units = Vec::new();
    units.extend(commits.into_iter().map(|(ingress, commit)| LawUnit {
        packets: vec![(ingress, LAW_ARCHIVE_COMMIT_INGRESS)],
        work: LawWork::ArchiveCommit(commit),
    }));
    units.extend(
        retention_acks
            .into_iter()
            .map(|(handle, (packets, acknowledgement))| LawUnit {
                packets: packets
                    .into_iter()
                    .map(|ingress| (ingress, LAW_ARCHIVE_RETENTION_ACK_INGRESS))
                    .collect(),
                work: LawWork::RetentionAck {
                    handle,
                    acknowledgement,
                },
            }),
    );
    units.extend(wakes.into_iter().map(|(due_at, packets)| {
        LawUnit {
            packets: packets
                .into_iter()
                .map(|ingress| (ingress, LAW_WAKE_INGRESS))
                .collect(),
            work: LawWork::Wake(due_at),
        }
    }));
    units.extend(mutations.into_iter().map(|(ingress, mutation)| LawUnit {
        packets: vec![(ingress, LAW_MUTATION_INGRESS)],
        work: LawWork::Mutation { ingress, mutation },
    }));
    units.extend(
        contexts
            .into_iter()
            .map(|(ingress, requirement, query)| LawUnit {
                packets: vec![(ingress, LAW_ACTOR_CONTEXT_INGRESS)],
                work: LawWork::ActorContext { requirement, query },
            }),
    );
    units.extend(
        preparations
            .into_iter()
            .map(|(sequence, (packets, expected_revision))| LawUnit {
                packets: packets
                    .into_iter()
                    .map(|ingress| (ingress, LAW_OUTBOX_PREPARE_INGRESS))
                    .collect(),
                work: LawWork::Prepare {
                    sequence,
                    expected_revision,
                },
            }),
    );
    units.extend(
        acknowledgements
            .into_iter()
            .map(|(sequence, (packets, acknowledgement))| LawUnit {
                packets: packets
                    .into_iter()
                    .map(|ingress| (ingress, LAW_OUTBOX_ACK_INGRESS))
                    .collect(),
                work: LawWork::Ack {
                    sequence,
                    acknowledgement,
                },
            }),
    );
    units.extend(intents.into_iter().map(|(ingress, intent)| LawUnit {
        packets: vec![(ingress, LAW_INTENT_INGRESS)],
        work: LawWork::Intent(intent),
    }));
    Ok(units)
}

/// Refuses a working set that names a shard the legal directory does not
/// hold, such as one of an order or jurisdiction the plan does not compile;
/// loading it would fail the whole boundary.
fn require_held_shards(
    directory: &crate::LegalDirectoryState,
    scope: &BTreeSet<LegalShardKey>,
) -> Result<(), CanwuError> {
    for shard in scope {
        if !directory.directory.active_shards.contains(shard)
            && !directory.directory.archive_only_shards.contains(shard)
        {
            return Err(invalid_record(
                "legal ingress names a shard the legal directory does not hold",
            ));
        }
        legal_shard_state_reference(shard)?;
        legal_archive_head_state_reference(shard)?;
    }
    Ok(())
}

/// Adds the shards one unit needs to the working set.
fn select_unit_scope(
    work: &LawWork,
    plan: &crate::CompiledLawPlan,
    directory: &crate::LegalDirectoryState,
    scope: &mut BTreeSet<LegalShardKey>,
) -> Result<(), CanwuError> {
    match work {
        LawWork::ArchiveCommit(commit) => {
            scope.insert(commit.compaction.shard.clone());
        }
        LawWork::Wake(due_at) => {
            // A continuation wake has no due-time route of its own; the work
            // it continues is loaded with the boundary's pending work routes.
            if let Some(due_routes) = directory.directory.due_shards.get(due_at) {
                scope.extend(due_routes.iter().cloned());
            }
        }
        LawWork::Mutation { mutation, .. } => {
            select_mutation_scope(mutation, plan, directory, scope)?;
        }
        LawWork::ActorContext { requirement, .. } => {
            select_object_route(directory, &requirement.procedure, scope)?;
        }
        LawWork::Intent(intent) => {
            // A procedure whose history was archived has no hot route; its late
            // seat response settles in the coordinator as a rejected outcome.
            if directory
                .directory
                .object_routes
                .contains_key(&intent.procedure.id)
            {
                select_object_route(directory, &intent.procedure.id, scope)?;
            }
        }
        LawWork::RetentionAck { .. } | LawWork::Prepare { .. } | LawWork::Ack { .. } => {}
    }
    Ok(())
}

/// Applies one unit to the working runtime, or says why it is refused.
#[allow(clippy::too_many_lines)]
fn apply_law_unit(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    plan: &crate::CompiledLawPlan,
    runtime: &mut LegalRuntime,
    work: &LawWork,
) -> Result<(), UnitFailure> {
    // Seat work that expired with its stage or procedure is terminal; a
    // preparation or acknowledgement that arrives for it afterwards is a no-op.
    let expired = |runtime: &LegalRuntime, sequence: u64| {
        runtime
            .outbox
            .get(&sequence)
            .is_some_and(|item| item.dispatch == crate::DispatchState::Expired)
    };
    match work {
        // An archive commit may change the runtime before it fails, so its
        // failure always restarts the pass; the refusal names the retention
        // handle and records a terminal the host finalizes.
        LawWork::ArchiveCommit(commit) => runtime
            .commit_verified_legal_archive(commit)
            .map(|_| ())
            .map_err(|error| {
                UnitFailure::applied(CanwuError::new(
                    error.code.clone(),
                    format!(
                        "legal archive retention handle {} was not committed: {}",
                        commit.retention_handle_id, error.message
                    ),
                ))
            }),
        LawWork::RetentionAck {
            handle,
            acknowledgement,
        } => run_runtime_op(runtime, |runtime| {
            runtime.acknowledge_archive_retention_terminal(
                handle,
                &acknowledgement.compaction_token,
                &acknowledgement.directory_root,
                acknowledgement.disposition,
                &acknowledgement.chain_root,
            )
        }),
        LawWork::Wake(due_at) => run_runtime_op(runtime, |runtime| runtime.consume_wake(*due_at)),
        LawWork::Mutation { ingress, mutation } => {
            let admitted_signal_kinds = check_legal_mutation(view, plan, runtime, mutation)
                .map_err(UnitFailure::checked)?;
            run_runtime_op(runtime, |runtime| {
                apply_legal_mutation(
                    context,
                    plan,
                    runtime,
                    *ingress,
                    mutation,
                    &admitted_signal_kinds,
                )
            })
        }
        LawWork::ActorContext { requirement, query } => {
            if query.after.is_some()
                || query.limit == 0
                || usize::try_from(query.limit)
                    .ok()
                    .is_none_or(|limit| limit > runtime.budgets.max_evidence_per_record)
            {
                return Err(UnitFailure::checked(CanwuError::new(
                    ErrorCode::KnowledgeLimitExceeded,
                    "legal actor context requires one bounded, unpaginated knowledge query",
                )));
            }
            let result = view
                .knowledge_records(requirement.holder.clone(), query)
                .map_err(UnitFailure::checked)?;
            if result.next.is_some() || result.holder != requirement.holder {
                return Err(UnitFailure::checked(CanwuError::new(
                    ErrorCode::KnowledgeLimitExceeded,
                    "legal actor context query must fit in one holder-bound page",
                )));
            }
            let actor_context = crate::runtime::actor_context_from_query_result(&result)
                .map_err(UnitFailure::checked)?;
            run_runtime_op(runtime, |runtime| {
                runtime.stage_actor_context(requirement, actor_context)
            })
        }
        LawWork::Prepare {
            sequence,
            expected_revision,
        } => {
            if expired(runtime, *sequence) {
                return Ok(());
            }
            if runtime
                .outbox
                .get(sequence)
                .and_then(|item| item.enqueue_expected_revision)
                .is_some_and(|existing| existing != *expected_revision)
            {
                verify_outbox_can_reprepare(view, runtime, *sequence)
                    .map_err(UnitFailure::checked)?;
            }
            run_runtime_op(runtime, |runtime| {
                runtime.stage_outbox_expected_revision(*sequence, *expected_revision)
            })
        }
        LawWork::Ack {
            sequence,
            acknowledgement,
        } => {
            if expired(runtime, *sequence) {
                return Ok(());
            }
            verify_outbox_enqueue(
                view,
                runtime,
                *sequence,
                acknowledgement.expected_revision,
                acknowledgement.controller_request_id,
                acknowledgement.create_request_id,
                acknowledgement.ticket_id,
                &acknowledgement.draft_hash,
                &acknowledgement.outcome_commitment,
            )
            .map_err(UnitFailure::checked)?;
            run_runtime_op(runtime, |runtime| {
                runtime.mark_outbox_enqueued(
                    *sequence,
                    acknowledgement.expected_revision,
                    EvidenceRef::Ingress(acknowledgement.ingress_id),
                    acknowledgement.outcome_commitment.clone(),
                )
            })
        }
        LawWork::Intent(intent) => run_runtime_op(runtime, |runtime| {
            runtime.queue_authorized_pending_intent((**intent).clone())
        }),
    }
}

/// Checks one legal mutation against the view and the working runtime
/// without changing either. Returns the provider signal kinds a non-procedural
/// source is admitted with.
fn check_legal_mutation(
    view: &SimulationView<'_>,
    plan: &crate::CompiledLawPlan,
    runtime: &LegalRuntime,
    mutation: &crate::LegalMutation,
) -> Result<Vec<String>, CanwuError> {
    match mutation {
        crate::LegalMutation::AdmitNonProceduralSource { proposal } => {
            let kinds = verified_required_signal_kinds(plan, runtime, proposal)?;
            verify_proposal_inputs(view, proposal)?;
            Ok(kinds)
        }
        crate::LegalMutation::Signal { signal } => {
            verify_legal_mutation(view, plan, runtime, mutation)?;
            let proposal = runtime.proposals.get(&signal.proposal_id).ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::InvalidDomainRecord,
                    "legal signal references an unknown proposal",
                )
            })?;
            if !host_versions_are_current(view, &proposal.expected_versions)? {
                return Err(CanwuError::new(
                    ErrorCode::DomainRecordVersionConflict,
                    "legal signal targets a proposal whose host compare-and-set no longer holds",
                ));
            }
            Ok(Vec::new())
        }
        _ => {
            verify_legal_mutation(view, plan, runtime, mutation)?;
            Ok(Vec::new())
        }
    }
}

/// Applies one checked legal mutation to the working runtime.
fn apply_legal_mutation(
    context: &BoundaryContext,
    plan: &crate::CompiledLawPlan,
    runtime: &mut LegalRuntime,
    ingress: canwu_api::IngressId,
    mutation: &crate::LegalMutation,
    admitted_signal_kinds: &[String],
) -> Result<(), CanwuError> {
    match mutation.clone() {
        crate::LegalMutation::SubmitProposal { proposal } => {
            runtime.submit_proposal_within_boundary(plan, proposal)
        }
        crate::LegalMutation::AdmitNonProceduralSource { proposal } => runtime
            .admit_non_procedural_source_within_boundary(
                plan,
                proposal,
                admitted_signal_kinds,
                context.at,
            ),
        crate::LegalMutation::Signal { signal } => {
            runtime.admit_signal_within_boundary(plan, signal)
        }
        crate::LegalMutation::RetireCulturalTarget { target, reason } => runtime
            .retire_cultural_target_from_ingress(
                plan,
                &target,
                context.at,
                reason,
                EvidenceRef::Ingress(ingress),
            ),
        crate::LegalMutation::RecordCase { case } => runtime.record_case(plan, case),
        crate::LegalMutation::RecordFinding { finding } => runtime.record_finding(plan, finding),
        crate::LegalMutation::RecordRuling { ruling } => runtime.record_ruling(plan, ruling),
        crate::LegalMutation::RecordConflict { conflict } => {
            runtime.record_conflict(plan, conflict)
        }
        crate::LegalMutation::RecordPublicity { publicity } => {
            runtime.record_publicity_at(plan, publicity, context.at)
        }
        crate::LegalMutation::RecordSuccession { succession } => {
            runtime.record_succession_for_plan(plan, succession)
        }
        crate::LegalMutation::AdmitCapacity { allocation } => {
            runtime.admit_capacity_allocation(plan, allocation)
        }
    }
}

fn select_object_route(
    directory: &crate::LegalDirectoryState,
    object_id: &str,
    scope: &mut BTreeSet<LegalShardKey>,
) -> Result<(), CanwuError> {
    let route = directory
        .directory
        .object_routes
        .get(object_id)
        .cloned()
        .ok_or_else(|| invalid_record(format!("legal object {object_id} has no shard route")))?;
    scope.insert(route);
    Ok(())
}

fn select_mutation_scope(
    mutation: &crate::LegalMutation,
    plan: &crate::CompiledLawPlan,
    directory: &crate::LegalDirectoryState,
    scope: &mut BTreeSet<LegalShardKey>,
) -> Result<(), CanwuError> {
    match mutation {
        crate::LegalMutation::SubmitProposal { proposal }
        | crate::LegalMutation::AdmitNonProceduralSource { proposal } => {
            scope.insert(LegalShardKey::order(proposal.legal_order.clone()));
            for jurisdiction in &proposal.jurisdictions {
                scope.insert(LegalShardKey::jurisdiction(
                    plan.definition_id.clone(),
                    jurisdiction.clone(),
                ));
            }
            if !proposal.cultural_dependencies.is_empty() {
                scope.insert(LegalShardKey::culture_dependency(
                    plan.definition_id.clone(),
                ));
            }
        }
        crate::LegalMutation::Signal { signal } => {
            select_object_route(directory, &signal.proposal_id, scope)?;
        }
        crate::LegalMutation::RetireCulturalTarget { .. } => {
            scope.insert(LegalShardKey::culture_dependency(
                plan.definition_id.clone(),
            ));
        }
        crate::LegalMutation::RecordCase { case } => {
            scope.insert(LegalShardKey::order(case.legal_order.clone()));
        }
        crate::LegalMutation::RecordFinding { finding } => {
            select_object_route(directory, &finding.case_id, scope)?;
        }
        crate::LegalMutation::RecordRuling { ruling } => {
            select_object_route(directory, &ruling.case_id, scope)?;
        }
        crate::LegalMutation::RecordConflict { conflict } => {
            if let Some(jurisdiction) = &conflict.jurisdiction {
                scope.insert(LegalShardKey::jurisdiction(
                    plan.definition_id.clone(),
                    jurisdiction.clone(),
                ));
            }
            for version in &conflict.versions {
                select_object_route(directory, &version.id, scope)?;
            }
        }
        crate::LegalMutation::RecordPublicity { publicity } => {
            select_object_route(directory, &publicity.proposal.id, scope)?;
        }
        crate::LegalMutation::RecordSuccession { succession } => {
            scope.extend(
                succession
                    .predecessors
                    .iter()
                    .chain(&succession.successors)
                    .cloned()
                    .map(LegalShardKey::order),
            );
        }
        crate::LegalMutation::AdmitCapacity { allocation } => {
            select_object_route(directory, &allocation.procedure, scope)?;
        }
    }
    Ok(())
}

fn payload_u64(payload: &Value, field: &str, context: &str) -> Result<u64, CanwuError> {
    payload.get(field).and_then(Value::as_u64).ok_or_else(|| {
        CanwuError::new(
            ErrorCode::InvalidPayload,
            format!("{context} requires {field}"),
        )
    })
}

fn payload_optional_u64(
    payload: &Value,
    field: &str,
    context: &str,
) -> Result<Option<u64>, CanwuError> {
    match payload.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_u64().map(Some).ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidPayload,
                format!("{context} requires {field} to be an unsigned integer or null"),
            )
        }),
    }
}

/// Checks that every cited evidence reference exists at this boundary's cut.
///
/// An exact domain-record version counts only when
/// `replay_stable_domain_record_version` resolves it: its record's committed
/// current version, a version proposed earlier in this boundary, or an
/// initial-scenario version. Whether a sealed run still knows any other
/// version depends on what its seals kept, so such a citation is refused in
/// every run. An event, command, attempt, ingress record, boundary, or draw is
/// answered by the engine's existence rule for ordinal evidence.
fn verify_evidence(
    view: &SimulationView<'_>,
    evidence: impl IntoIterator<Item = EvidenceRef>,
) -> Result<(), CanwuError> {
    for reference in evidence {
        let exists = match &reference {
            EvidenceRef::DomainRecordVersion(version) => {
                view.replay_stable_domain_record_version(version)?.is_some()
            }
            _ => view.evidence_exists(&reference)?,
        };
        if !exists {
            return Err(CanwuError::new(
                ErrorCode::InvalidAuthority,
                "legal mutation references unavailable evidence",
            ));
        }
    }
    Ok(())
}

/// Whether each host record version a proposal compare-and-sets is still its
/// record's current version at this boundary's cut.
///
/// The current version comes from this boundary's proposals or the committed
/// provenance index, never from retained evidence, so a sealed run answers as
/// its unsealed replay does.
fn host_versions_are_current(
    view: &SimulationView<'_>,
    versions: &[canwu_api::DomainRecordVersionRef],
) -> Result<bool, CanwuError> {
    for version in versions {
        if view
            .current_domain_record_version(&version.record)?
            .as_ref()
            != Some(version)
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn verify_proposal_inputs(
    view: &SimulationView<'_>,
    proposal: &crate::LegalProposal,
) -> Result<(), CanwuError> {
    if !host_versions_are_current(view, &proposal.expected_versions)? {
        return Err(CanwuError::new(
            ErrorCode::DomainRecordVersionConflict,
            "legal proposal host record compare-and-set failed",
        ));
    }
    verify_evidence(view, proposal.evidence.iter().cloned())?;
    if proposal
        .cultural_dependencies
        .iter()
        .any(|dependency| !proposal.evidence.contains(&dependency.evidence))
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "legal culture dependency lacks admitted proposal evidence",
        ));
    }
    Ok(())
}

fn verify_legal_mutation(
    view: &SimulationView<'_>,
    plan: &crate::CompiledLawPlan,
    runtime: &LegalRuntime,
    mutation: &crate::LegalMutation,
) -> Result<(), CanwuError> {
    match mutation {
        crate::LegalMutation::SubmitProposal { proposal }
        | crate::LegalMutation::AdmitNonProceduralSource { proposal } => {
            verify_proposal_inputs(view, proposal)
        }
        crate::LegalMutation::Signal { signal } => {
            verify_signal_provenance(plan, runtime, &signal.kind, &signal.evidence)?;
            verify_evidence(view, signal.evidence.iter().cloned())
        }
        crate::LegalMutation::RetireCulturalTarget { .. }
        | crate::LegalMutation::RecordConflict { .. } => Ok(()),
        crate::LegalMutation::RecordCase { case } => {
            verify_evidence(view, case.allegations.iter().cloned())
        }
        crate::LegalMutation::RecordFinding { finding } => {
            verify_evidence(view, finding.evidence.iter().cloned())
        }
        crate::LegalMutation::RecordRuling { ruling } => {
            verify_evidence(view, ruling.evidence.iter().cloned())
        }
        crate::LegalMutation::RecordPublicity { publicity } => {
            verify_publicity_provenance(plan, runtime, publicity)?;
            verify_evidence(view, publicity.evidence.iter().cloned())
        }
        crate::LegalMutation::RecordSuccession { succession } => {
            verify_evidence(view, succession.evidence.iter().cloned())
        }
        crate::LegalMutation::AdmitCapacity { allocation } => {
            verify_evidence(view, [allocation.evidence.clone()])
        }
    }
}

fn verified_required_signal_kinds(
    plan: &crate::CompiledLawPlan,
    runtime: &LegalRuntime,
    proposal: &crate::LegalProposal,
) -> Result<Vec<String>, CanwuError> {
    let profile = plan
        .source_profile_by_id
        .get(&proposal.source_profile)
        .and_then(|key| plan.source_profiles.get(key.get() as usize))
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidDomainRecord,
                "non-procedural source profile is missing",
            )
        })?;
    for kind in &profile.required_signal_kinds {
        verify_signal_provenance(plan, runtime, kind, &proposal.evidence)?;
    }
    Ok(profile.required_signal_kinds.clone())
}

/// The compiled provider of one signal kind.
fn signal_provider<'a>(
    plan: &'a crate::CompiledLawPlan,
    kind: &str,
) -> Result<&'a crate::LegalSignalProviderDefinition, CanwuError> {
    plan.signal_provider_by_kind.get(kind).ok_or_else(|| {
        CanwuError::new(
            ErrorCode::InvalidAuthority,
            format!("legal signal kind {kind} has no compiled provider"),
        )
    })
}

/// Checks that the evidence cites a packet of the signal kind's provider.
///
/// The check reads the legal provider index: the provider packets the
/// admission system recorded when their boundary admitted them. Every run
/// records the same packets, so a citation decides alike in a sealed run, an
/// unsealed run, and exact replay, whichever boundary admitted the packet. A
/// packet the index never recorded (admitted before the plan was installed,
/// or when its record did not fit the state budget) or has forgotten (it keeps
/// `max_provider_packets`, forgetting the lowest ingress ID first) is refused.
fn verify_signal_provenance(
    plan: &crate::CompiledLawPlan,
    runtime: &LegalRuntime,
    kind: &str,
    evidence: &[EvidenceRef],
) -> Result<(), CanwuError> {
    let provider = signal_provider(plan, kind)?;
    if evidence.iter().any(|evidence| {
        matches!(evidence, EvidenceRef::Ingress(id) if runtime
            .provider_packets
            .get(id)
            .is_some_and(|packet| packet.plugin == provider.plugin
                && packet.packet_type == provider.packet_type))
    }) {
        return Ok(());
    }
    Err(CanwuError::new(
        ErrorCode::InvalidAuthority,
        format!(
            "legal signal kind {kind} cites no packet of provider {}:{} in the legal provider index",
            provider.plugin, provider.packet_type
        ),
    ))
}

/// Checks that a publicity event cites the exact provider packet it records:
/// the provider index holds the packet with the payload hash of the event's
/// time and content.
fn verify_publicity_provenance(
    plan: &crate::CompiledLawPlan,
    runtime: &LegalRuntime,
    publicity: &crate::LegalPublicityEvent,
) -> Result<(), CanwuError> {
    let provider = signal_provider(plan, &publicity.signal_kind)?;
    let expected_payload = serde_json::json!({
        "proposal": publicity.proposal.id,
        "at": publicity.at.as_minutes(),
        "signal_kind": publicity.signal_kind,
        "medium": publicity.medium,
        "scope": publicity.scope,
    });
    let expected_hash = canonical_hash(
        crate::LEGAL_PROVIDER_PAYLOAD_DOMAIN,
        &(publicity.at, &expected_payload),
    )?;
    if publicity.evidence.iter().any(|evidence| {
        matches!(evidence, EvidenceRef::Ingress(id) if runtime
            .provider_packets
            .get(id)
            .is_some_and(|packet| packet.plugin == provider.plugin
                && packet.packet_type == provider.packet_type
                && packet.payload_hash == expected_hash))
    }) {
        return Ok(());
    }
    Err(CanwuError::new(
        ErrorCode::InvalidAuthority,
        format!(
            "legal publicity {} cites no exact payload of provider {}:{} in the legal provider index",
            publicity.id, provider.plugin, provider.packet_type
        ),
    ))
}

/// Rejects every pending proposal this boundary will work on whose host
/// compare-and-set no longer holds or that depends on a retired culture
/// generation, and closes its procedure, before the boundary settles.
///
/// Such a proposal can never be adopted. Rejecting it, as a failed law-local
/// head compare-and-set already is at adoption, keeps it from failing every
/// boundary that touches it. Returns each rejected proposal with the reason.
fn reject_stale_pending_proposals(
    view: &SimulationView<'_>,
    runtime: &mut LegalRuntime,
) -> Result<Vec<(String, String)>, CanwuError> {
    let mut rejected = Vec::new();
    for procedure_id in runtime.dirty_procedures.clone() {
        let Some(proposal) = runtime
            .procedures
            .get(&procedure_id)
            .filter(|procedure| !procedure.closed)
            .and_then(|procedure| runtime.proposals.get(&procedure.proposal.id))
        else {
            continue;
        };
        let reason = if !host_versions_are_current(view, &proposal.expected_versions)? {
            "a host record version it compare-and-sets is no longer current"
        } else if proposal.cultural_dependencies.iter().any(|dependency| {
            runtime
                .retired_cultural_targets
                .contains(&dependency.target)
        }) {
            "it depends on a retired culture generation"
        } else {
            continue;
        };
        let proposal_id = proposal.id.clone();
        runtime.reject_open_procedure(&procedure_id)?;
        rejected.push((proposal_id, reason.to_owned()));
    }
    Ok(rejected)
}

fn verify_outbox_can_reprepare(
    view: &SimulationView<'_>,
    runtime: &LegalRuntime,
    sequence: u64,
) -> Result<(), CanwuError> {
    let item = runtime.outbox.get(&sequence).ok_or_else(|| {
        CanwuError::new(
            ErrorCode::InvalidDomainRecord,
            "legal outbox preparation references an unknown sequence",
        )
    })?;
    let controller_request_id = item.refresh_request_id.ok_or_else(|| {
        CanwuError::new(
            ErrorCode::InvalidDomainRecord,
            "legal outbox controller request ID is missing",
        )
    })?;
    if view
        .decision_attempt(DecisionRequestId::new(controller_request_id))?
        .is_some()
        || view
            .decision_attempt(DecisionRequestId::new(item.create_request_id))?
            .is_some()
        || view
            .decision_ticket(DecisionTicketId::new(item.ticket_id))?
            .is_some()
    {
        return Err(CanwuError::new(
            ErrorCode::IdempotencyConflict,
            "legal outbox cannot reprepare after a core decision outcome exists",
        ));
    }
    if view
        .decision_controller(&item.decision_controller_id)?
        .is_some_and(|controller| {
            !crate::runtime::decision_controller_matches_outbox(item, controller)
        })
    {
        return Err(CanwuError::new(
            ErrorCode::IdempotencyConflict,
            "legal outbox controller binding conflicts with the persisted draft",
        ));
    }
    Ok(())
}

fn sequence_and_revision(payload: &Value, context: &str) -> Result<(u64, u64), CanwuError> {
    Ok((
        payload_u64(payload, "sequence", context)?,
        payload_u64(payload, "expected_revision", context)?,
    ))
}

#[allow(clippy::too_many_arguments)]
fn verify_outbox_enqueue(
    view: &SimulationView<'_>,
    runtime: &LegalRuntime,
    sequence: u64,
    expected_revision: u64,
    controller_request_id_from_payload: Option<u64>,
    create_request_id: u64,
    ticket_id: u64,
    draft_hash: &str,
    outcome_commitment: &str,
) -> Result<(), CanwuError> {
    let item = runtime.outbox.get(&sequence).ok_or_else(|| {
        CanwuError::new(
            ErrorCode::InvalidDomainRecord,
            "legal outbox acknowledgement references an unknown sequence",
        )
    })?;
    let controller_request_id = item.refresh_request_id.ok_or_else(|| {
        CanwuError::new(
            ErrorCode::InvalidAuthority,
            "legal outbox controller request ID is missing",
        )
    })?;
    let controller_attempt =
        view.decision_attempt(DecisionRequestId::new(controller_request_id))?;
    let open_attempt = view
        .decision_attempt(DecisionRequestId::new(create_request_id))?
        .ok_or_else(|| CanwuError::new(ErrorCode::InvalidAuthority, "ticket request missing"))?;
    let controller = view
        .decision_controller(&item.decision_controller_id)?
        .ok_or_else(|| CanwuError::new(ErrorCode::InvalidAuthority, "controller missing"))?;
    let ticket = view
        .decision_ticket(DecisionTicketId::new(ticket_id))?
        .ok_or_else(|| CanwuError::new(ErrorCode::InvalidAuthority, "ticket missing"))?;
    crate::runtime::verify_accepted_outbox_state(
        item,
        expected_revision,
        controller_attempt,
        open_attempt,
        controller,
        ticket,
    )?;
    let expected_outcome_commitment = crate::runtime::outbox_outcome_commitment(
        controller_attempt,
        open_attempt,
        controller,
        ticket,
    )?;
    let exact_hash = canonical_hash("canwu.law.decision-draft.v1", &item.draft)?;
    if item.enqueue_expected_revision != Some(expected_revision)
        || controller_attempt.map(|attempt| attempt.request_id.get())
            != controller_request_id_from_payload
        || item.create_request_id != create_request_id
        || item.ticket_id != ticket_id
        || exact_hash != draft_hash
        || outcome_commitment != expected_outcome_commitment
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "legal outbox acknowledgement does not match the persisted enqueue request",
        ));
    }
    Ok(())
}

fn verify_intent_command(
    view: &SimulationView<'_>,
    cause: Option<&CauseRef>,
    intent: &crate::PendingLegalIntent,
) -> Result<(), CanwuError> {
    let Some(CauseRef::Command(command_id)) = cause else {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "legal intent ingress lacks command evidence",
        ));
    };
    let command = view.command(*command_id)?.ok_or_else(|| {
        CanwuError::new(
            ErrorCode::InvalidAuthority,
            "legal intent command evidence is unavailable",
        )
    })?;
    let Command::Plugin {
        plugin,
        command: command_name,
        payload,
    } = &command.envelope.command
    else {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "legal intent evidence is not a plugin command",
        ));
    };
    let mut authored = payload
        .get("intent")
        .cloned()
        .ok_or_else(|| CanwuError::new(ErrorCode::InvalidPayload, "legal intent is missing"))
        .and_then(|value| {
            serde_json::from_value::<crate::PendingLegalIntent>(value).map_err(|error| {
                CanwuError::new(
                    ErrorCode::InvalidPayload,
                    format!("invalid command-evidence legal intent: {error}"),
                )
            })
        })?;
    authored.command.clone_from(&intent.command);
    authored.attempt.clone_from(&intent.attempt);
    authored.request_id = intent.request_id;
    authored.admitted_at = intent.admitted_at;
    if plugin != PLUGIN_NAME || command_name != LAW_COMMAND || &authored != intent {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "legal intent ingress does not match its authorized command",
        ));
    }
    Ok(())
}

fn holder_for_origin(origin: &DecisionOrigin) -> Result<KnowledgeHolderRef, CanwuError> {
    match origin {
        DecisionOrigin::Actor { actor } => Ok(KnowledgeHolderRef::Person(*actor)),
        DecisionOrigin::Institution { institution, .. } => {
            Ok(KnowledgeHolderRef::Entity(institution.clone()))
        }
        DecisionOrigin::Council { council_id } => {
            Ok(KnowledgeHolderRef::Entity(EntityRef::Domain(
                DomainRecordRef::new(crate::PLUGIN_NAMESPACE, "council", council_id),
            )))
        }
        DecisionOrigin::NoResponsibleActor { .. } => Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "legal intent requires a responsible controller",
        )),
    }
}

fn invalid_record(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidDomainRecord, message)
}

fn decode_compiled_plan(plan_state: &LegalPlanState) -> Result<crate::CompiledLawPlan, CanwuError> {
    let plan = plan_state
        .fields
        .get("plan")
        .cloned()
        .ok_or_else(|| invalid_record("law plan state is missing its compiled plan"))?;
    let plan = serde_json::from_value::<crate::CompiledLawPlan>(plan).map_err(|error| {
        invalid_record(format!(
            "law plan state contains an invalid compiled plan: {error}"
        ))
    })?;
    if plan.content_hash != plan_state.plan_hash {
        return Err(invalid_record(
            "law plan state hash does not match the compiled plan",
        ));
    }
    Ok(plan)
}

fn assemble_runtime_from_records(
    records: &BTreeMap<DomainRecordRef, DomainRecord>,
    plan: &crate::CompiledLawPlan,
) -> Result<LegalRuntime, CanwuError> {
    let plan_record = records
        .get(&legal_plan_state_reference().into_untyped())
        .ok_or_else(|| invalid_record("law plan state is missing"))?;
    let directory_record = records
        .get(&legal_directory_state_reference().into_untyped())
        .ok_or_else(|| invalid_record("law directory state is missing"))?;
    let plan_state = plan_record.decode_payload::<LegalPlanStateRecord>()?;
    let directory_state = directory_record.decode_payload::<LegalDirectoryStateRecord>()?;
    let mut shards = BTreeMap::new();
    let mut heads = BTreeMap::new();
    for shard in directory_state
        .directory
        .active_shards
        .iter()
        .chain(directory_state.directory.archive_only_shards.iter())
    {
        let shard_ref = legal_shard_state_reference(shard)?.into_untyped();
        let head_ref = legal_archive_head_state_reference(shard)?.into_untyped();
        if directory_state.shard_record_ids.get(shard) != Some(&shard_ref.id)
            || directory_state.archive_head_record_ids.get(shard) != Some(&head_ref.id)
        {
            return Err(invalid_record(
                "law directory contains a non-canonical record route",
            ));
        }
        let shard_record = records
            .get(&shard_ref)
            .ok_or_else(|| invalid_record("law directory references a missing shard"))?;
        let head_record = records
            .get(&head_ref)
            .ok_or_else(|| invalid_record("law directory references a missing archive head"))?;
        if shards
            .insert(
                shard.clone(),
                shard_record.decode_payload::<LegalShardStateRecord>()?,
            )
            .is_some()
            || heads
                .insert(
                    shard.clone(),
                    head_record.decode_payload::<LegalArchiveHeadStateRecord>()?,
                )
                .is_some()
        {
            return Err(invalid_record(
                "law directory contains a duplicate shard route",
            ));
        }
    }
    LegalRuntime::from_persistence_image(plan, plan_state, directory_state, &shards, heads)
}

fn load_legal_header_from_view(
    view: &SimulationView<'_>,
) -> Result<
    (
        DomainRecord,
        LegalPlanState,
        DomainRecord,
        crate::LegalDirectoryState,
        crate::CompiledLawPlan,
    ),
    CanwuError,
> {
    let plan_record = view
        .typed_domain_record(&legal_plan_state_reference())?
        .ok_or_else(|| invalid_record("law plan state is missing"))?
        .clone();
    let plan_state = plan_record.decode_payload::<LegalPlanStateRecord>()?;
    let plan = decode_compiled_plan(&plan_state)?;
    let directory_record = view
        .typed_domain_record(&legal_directory_state_reference())?
        .ok_or_else(|| invalid_record("law directory state is missing"))?
        .clone();
    let directory = directory_record.decode_payload::<LegalDirectoryStateRecord>()?;
    Ok((plan_record, plan_state, directory_record, directory, plan))
}

#[allow(clippy::too_many_arguments)]
fn load_legal_runtime_from_view_scope(
    view: &SimulationView<'_>,
    plan_record: DomainRecord,
    plan_state: LegalPlanState,
    directory_record: DomainRecord,
    directory: crate::LegalDirectoryState,
    plan: &crate::CompiledLawPlan,
    scope: &BTreeSet<LegalShardKey>,
    partial: bool,
) -> Result<LegalRuntime, CanwuError> {
    let mut records = BTreeMap::from([
        (plan_record.reference.clone(), plan_record),
        (directory_record.reference.clone(), directory_record),
    ]);
    for shard in scope {
        if !directory.directory.active_shards.contains(shard)
            && !directory.directory.archive_only_shards.contains(shard)
        {
            return Err(invalid_record(
                "legal working set requests an unknown shard",
            ));
        }
        for reference in [
            legal_shard_state_reference(shard)?.into_untyped(),
            legal_archive_head_state_reference(shard)?.into_untyped(),
        ] {
            let record = view
                .domain_record(&reference)?
                .ok_or_else(|| invalid_record("law directory references a missing record"))?
                .clone();
            records.insert(reference, record);
        }
    }
    if partial {
        let mut shards = BTreeMap::new();
        let mut heads = BTreeMap::new();
        for shard in scope {
            let shard_record = records
                .get(&legal_shard_state_reference(shard)?.into_untyped())
                .ok_or_else(|| invalid_record("legal working-set shard is missing"))?;
            let head_record = records
                .get(&legal_archive_head_state_reference(shard)?.into_untyped())
                .ok_or_else(|| invalid_record("legal working-set archive head is missing"))?;
            shards.insert(
                shard.clone(),
                shard_record.decode_payload::<LegalShardStateRecord>()?,
            );
            heads.insert(
                shard.clone(),
                head_record.decode_payload::<LegalArchiveHeadStateRecord>()?,
            );
        }
        LegalRuntime::from_scoped_persistence_image(
            plan,
            plan_state,
            directory,
            &shards,
            heads,
            Some(scope),
        )
    } else {
        assemble_runtime_from_records(&records, plan)
    }
}

#[must_use]
pub fn law_record_schemas() -> Vec<DomainRecordSchema> {
    fn schema<T: canwu_api::DomainValueType>() -> DomainRecordSchema {
        let mut schema = DomainRecordSchema::for_record::<T>();
        schema.payload_schema = PayloadSchema::Object {
            properties: BTreeMap::from([(
                IDENTITY_EVIDENCE_DEPENDENCIES_FIELD.to_owned(),
                identity_evidence_dependencies_property_v1(),
            )]),
            allow_additional: true,
        };
        schema
    }
    vec![
        schema::<LegalPlanStateRecord>(),
        schema::<LegalDirectoryStateRecord>(),
        schema::<LegalShardStateRecord>(),
        schema::<LegalArchiveHeadStateRecord>(),
    ]
}

fn encoded_runtime_payload_len(record: &DomainRecord) -> Result<usize, CanwuError> {
    serde_json::to_vec(&record.payload)
        .map(|encoded| encoded.len())
        .map_err(|error| CanwuError::new(ErrorCode::InvalidDomainRecord, error.to_string()))
}

/// Load the plugin-owned runtime record and fail closed on a plan mismatch.
pub fn load_legal_runtime<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &H,
    plan: &crate::CompiledLawPlan,
) -> Result<Option<LegalRuntime>, CanwuError> {
    validate_law_records(canwu)?;
    let Some(plan_record) = canwu_api::typed_domain_record(canwu, &legal_plan_state_reference())
    else {
        return Ok(None);
    };
    let directory_record =
        canwu_api::typed_domain_record(canwu, &legal_directory_state_reference())
            .ok_or_else(|| invalid_record("law directory record is missing"))?;
    let plan_state = plan_record.decode_payload::<LegalPlanStateRecord>()?;
    let directory_state = directory_record.decode_payload::<LegalDirectoryStateRecord>()?;
    let mut encoded_bytes = encoded_runtime_payload_len(plan_record)?
        .checked_add(encoded_runtime_payload_len(directory_record)?)
        .ok_or_else(|| invalid_record("law persisted byte count overflowed"))?;
    let mut shards = BTreeMap::new();
    let mut archive_heads = BTreeMap::new();
    for shard in directory_state
        .directory
        .active_shards
        .iter()
        .chain(directory_state.directory.archive_only_shards.iter())
    {
        let shard_record =
            canwu_api::typed_domain_record(canwu, &legal_shard_state_reference(shard)?)
                .ok_or_else(|| invalid_record("law directory references a missing shard record"))?;
        let archive_record =
            canwu_api::typed_domain_record(canwu, &legal_archive_head_state_reference(shard)?)
                .ok_or_else(|| {
                    invalid_record("law directory references a missing archive-head record")
                })?;
        let shard_bytes = encoded_runtime_payload_len(shard_record)?;
        let archive_bytes = encoded_runtime_payload_len(archive_record)?;
        encoded_bytes = encoded_bytes
            .checked_add(shard_bytes)
            .and_then(|bytes| bytes.checked_add(archive_bytes))
            .ok_or_else(|| invalid_record("law persisted byte count overflowed"))?;
        let payload = shard_record.decode_payload::<LegalShardStateRecord>()?;
        let head = archive_record.decode_payload::<LegalArchiveHeadStateRecord>()?;
        if shards.insert(shard.clone(), payload).is_some()
            || archive_heads.insert(shard.clone(), head).is_some()
        {
            return Err(invalid_record("law directory contains a duplicate shard"));
        }
    }
    if encoded_bytes > plan.budgets.max_state_bytes || encoded_bytes > plan.budgets.max_memory_bytes
    {
        return Err(CanwuError::new(
            ErrorCode::ValueOutOfRange,
            "law sharded state exceeds its decode budget",
        ));
    }
    Ok(Some(LegalRuntime::from_persistence_image(
        plan,
        plan_state,
        directory_state,
        &shards,
        archive_heads,
    )?))
}

/// Decode every law-owned record before exposing the plan-bound runtime.
pub fn load_law_state_for_plan<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &H,
    plan: &crate::CompiledLawPlan,
) -> Result<Option<LegalRuntime>, CanwuError> {
    load_legal_runtime(canwu, plan)
}

fn validate_law_records<H: canwu_api::CanwuRun + ?Sized>(canwu: &H) -> Result<(), CanwuError> {
    for record in canwu
        .domain_records()
        .filter(|record| record.reference.kind.namespace == crate::PLUGIN_NAMESPACE)
    {
        match record.reference.kind.name.as_str() {
            LAW_RUNTIME_STATE => {
                return Err(invalid_record(
                    "Format-7 aggregate law records are unsupported; export into a new Format-8 run",
                ));
            }
            LAW_PLAN_STATE if record.reference == legal_plan_state_reference().into_untyped() => {}
            LAW_DIRECTORY_STATE
                if record.reference == legal_directory_state_reference().into_untyped() => {}
            LAW_SHARD_STATE | LAW_ARCHIVE_HEAD_STATE => {}
            unknown => {
                return Err(CanwuError::new(
                    ErrorCode::InvalidDomainRecord,
                    format!("unrecognized law-owned record kind {unknown}"),
                ));
            }
        }
    }
    Ok(())
}

fn validate_law_activation_records(records: &[DomainRecord]) -> Result<(), CanwuError> {
    let mut plan_seen = false;
    let mut directory_seen = false;
    let mut encoded_bytes = 0_usize;
    for record in records
        .iter()
        .filter(|record| record.reference.kind.namespace == crate::PLUGIN_NAMESPACE)
    {
        match record.reference.kind.name.as_str() {
            LAW_RUNTIME_STATE => {
                return Err(invalid_record(
                    "Format-7 aggregate law records are unsupported; export into a new Format-8 run",
                ));
            }
            LAW_PLAN_STATE => {
                if plan_seen || record.reference != legal_plan_state_reference().into_untyped() {
                    return Err(invalid_record(
                        "law plan state must use one canonical identity",
                    ));
                }
                plan_seen = true;
                record.decode_payload::<LegalPlanStateRecord>()?;
            }
            LAW_DIRECTORY_STATE => {
                if directory_seen
                    || record.reference != legal_directory_state_reference().into_untyped()
                {
                    return Err(invalid_record(
                        "law directory state must use one canonical identity",
                    ));
                }
                directory_seen = true;
                record.decode_payload::<LegalDirectoryStateRecord>()?;
            }
            LAW_SHARD_STATE => {
                record.decode_payload::<LegalShardStateRecord>()?;
            }
            LAW_ARCHIVE_HEAD_STATE => {
                record.decode_payload::<LegalArchiveHeadStateRecord>()?;
            }
            unknown => {
                return Err(CanwuError::new(
                    ErrorCode::InvalidDomainRecord,
                    format!("unrecognized law-owned record kind {unknown}"),
                ));
            }
        }
        encoded_bytes = encoded_bytes
            .checked_add(encoded_runtime_payload_len(record)?)
            .ok_or_else(|| invalid_record("law activation byte count overflowed"))?;
        if encoded_bytes > crate::MAX_LEGAL_STATE_BYTES {
            return Err(CanwuError::new(
                ErrorCode::ValueOutOfRange,
                "law sharded state exceeds the absolute activation payload ceiling",
            ));
        }
    }
    if plan_seen != directory_seen {
        return Err(invalid_record(
            "law plan and directory records must be activated together",
        ));
    }
    if plan_seen {
        let plan_record = records
            .iter()
            .find(|record| record.reference == legal_plan_state_reference().into_untyped())
            .ok_or_else(|| invalid_record("law plan record is missing"))?;
        let plan_state = plan_record.decode_payload::<LegalPlanStateRecord>()?;
        let plan = decode_compiled_plan(&plan_state)?;
        let records_by_ref = records
            .iter()
            .map(|record| (record.reference.clone(), record.clone()))
            .collect::<BTreeMap<_, _>>();
        assemble_runtime_from_records(&records_by_ref, &plan)?;
    }
    Ok(())
}
