use crate::LegalRuntime;
use crate::{
    LAW_SEMANTIC_HASH, LegalArchiveHeadStateRecord, LegalArchiveStore, LegalDirectoryStateRecord,
    LegalPlanState, LegalPlanStateRecord, LegalShardKey, LegalShardStateRecord, PLUGIN_NAME,
    PreparedLegalArchiveBatch, VerifiedLegalArchiveCommit, legal_archive_head_state_reference,
    legal_directory_state_reference, legal_plan_state_reference, legal_shard_state_reference,
};
use canwu_api::{
    ArchiveReachabilityManifest, BoundaryContext, BoundaryDirective, BoundaryPhase,
    BoundaryProposal, Canwu, CanwuError, CauseRef, Command, CommandContext, DecisionOrigin,
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
pub fn enqueue_legal_mutation(
    canwu: &mut Canwu,
    mutation: &crate::LegalMutation,
) -> Result<canwu_api::IngressReceipt, CanwuError> {
    enqueue_legal_mutation_at(canwu, canwu.time(), mutation)
}

/// Queue one legal mutation for a declared future boundary.
pub fn enqueue_legal_mutation_at(
    canwu: &mut Canwu,
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
pub fn enqueue_legal_archive(
    canwu: &mut Canwu,
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
pub fn finalize_legal_archive_retention(
    canwu: &mut Canwu,
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

fn admit_legal_ingress(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut intents = Vec::new();
    let mut acknowledgements = BTreeMap::<u64, OutboxAcknowledgementAdmission>::new();
    let mut preparations = BTreeMap::<u64, u64>::new();
    let mut actor_context_queries = Vec::new();
    let mut mutations = Vec::new();
    let mut wake_requested = Vec::new();
    let mut archive_commits = Vec::new();
    let mut archive_retention_acknowledgements =
        BTreeMap::<String, ArchiveRetentionAcknowledgementAdmission>::new();
    let mut header = None;
    let mut selected_shards = BTreeSet::new();
    let mut collected_mutations = 0;
    for ingress_id in &context.admitted_ingress {
        let Some(ingress) = view.ingress(*ingress_id)? else {
            continue;
        };
        let IngressPayload::Plugin {
            plugin,
            packet_type,
            payload,
            archive_retention,
            ..
        } = &ingress.payload
        else {
            continue;
        };
        if plugin != PLUGIN_NAME {
            continue;
        }
        if ![
            LAW_INTENT_INGRESS,
            LAW_ACTOR_CONTEXT_INGRESS,
            LAW_MUTATION_INGRESS,
            LAW_WAKE_INGRESS,
            LAW_OUTBOX_PREPARE_INGRESS,
            LAW_OUTBOX_ACK_INGRESS,
            LAW_ARCHIVE_COMMIT_INGRESS,
            LAW_ARCHIVE_RETENTION_ACK_INGRESS,
        ]
        .contains(&packet_type.as_str())
        {
            continue;
        }
        if header.is_none() {
            header = Some(load_legal_header_from_view(view)?);
        }
        let (_, _, _, directory, plan) = header.as_ref().ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidDomainRecord,
                "legal persistence header could not be loaded for admitted legal ingress",
            )
        })?;
        selected_shards.insert(LegalShardKey::coordinator(plan.definition_id.clone()));
        let mutation_budget = plan.budgets.max_mutations_per_boundary;
        if packet_type == LAW_ARCHIVE_COMMIT_INGRESS {
            let commit = serde_json::from_value::<VerifiedLegalArchiveCommit>(
                payload.get("commit").cloned().ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::InvalidPayload,
                        "legal archive ingress requires a verified commit",
                    )
                })?,
            )
            .map_err(|error| {
                CanwuError::new(
                    ErrorCode::InvalidPayload,
                    format!("invalid verified legal archive commit: {error}"),
                )
            })?;
            commit.validate()?;
            if archive_retention.as_slice() != legal_archive_ingress_retention(&commit).as_slice() {
                return Err(CanwuError::new(
                    ErrorCode::InvalidPayload,
                    "legal archive ingress retention is not bound to its committed directory root",
                ));
            }
            reserve_collected_mutation(&mut collected_mutations, mutation_budget)?;
            selected_shards.insert(commit.compaction.shard.clone());
            archive_commits.push(commit);
        } else if packet_type == LAW_ARCHIVE_RETENTION_ACK_INGRESS {
            let retention_handle_id = payload
                .get("retention_handle_id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::InvalidPayload,
                        "legal archive retention acknowledgement requires a handle ID",
                    )
                })?
                .to_owned();
            let acknowledgement = ArchiveRetentionAcknowledgementAdmission {
                compaction_token: payload
                    .get("compaction_token")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        CanwuError::new(
                            ErrorCode::InvalidPayload,
                            "legal archive retention acknowledgement requires a compaction token",
                        )
                    })?
                    .to_owned(),
                directory_root: payload
                    .get("directory_root")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        CanwuError::new(
                            ErrorCode::InvalidPayload,
                            "legal archive retention acknowledgement requires a directory root",
                        )
                    })?
                    .to_owned(),
                disposition: serde_json::from_value(
                    payload.get("disposition").cloned().ok_or_else(|| {
                        CanwuError::new(
                            ErrorCode::InvalidPayload,
                            "legal archive retention acknowledgement requires a disposition",
                        )
                    })?,
                )
                .map_err(|error| {
                    CanwuError::new(
                        ErrorCode::InvalidPayload,
                        format!("invalid legal archive retention disposition: {error}"),
                    )
                })?,
                chain_root: payload
                    .get("chain_root")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        CanwuError::new(
                            ErrorCode::InvalidPayload,
                            "legal archive retention acknowledgement requires a chain root",
                        )
                    })?
                    .to_owned(),
            };
            reserve_collected_mutation(&mut collected_mutations, mutation_budget)?;
            if let Some(existing) = archive_retention_acknowledgements
                .insert(retention_handle_id, acknowledgement.clone())
                && existing != acknowledgement
            {
                return Err(CanwuError::new(
                    ErrorCode::IdempotencyConflict,
                    "conflicting legal archive retention acknowledgements share one handle",
                ));
            }
        } else if packet_type == LAW_INTENT_INGRESS {
            let intent = serde_json::from_value::<crate::PendingLegalIntent>(payload.clone())
                .map_err(|error| {
                    CanwuError::new(
                        ErrorCode::InvalidPayload,
                        format!("invalid admitted legal intent: {error}"),
                    )
                })?;
            verify_intent_command(view, ingress.cause.as_ref(), &intent)?;
            reserve_collected_mutation(&mut collected_mutations, mutation_budget)?;
            select_object_route(directory, &intent.procedure.id, &mut selected_shards)?;
            intents.push(intent);
        } else if packet_type == LAW_ACTOR_CONTEXT_INGRESS {
            let requirement = serde_json::from_value::<crate::LegalActorContextRequirement>(
                payload.get("requirement").cloned().ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::InvalidPayload,
                        "legal actor context requires a seat requirement",
                    )
                })?,
            )
            .map_err(|error| {
                CanwuError::new(
                    ErrorCode::InvalidPayload,
                    format!("invalid legal actor context requirement: {error}"),
                )
            })?;
            let query = serde_json::from_value::<canwu_api::KnowledgeQuery>(
                payload.get("query").cloned().ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::InvalidPayload,
                        "legal actor context requires a knowledge query",
                    )
                })?,
            )
            .map_err(|error| {
                CanwuError::new(
                    ErrorCode::InvalidPayload,
                    format!("invalid legal actor knowledge query: {error}"),
                )
            })?;
            reserve_collected_mutation(&mut collected_mutations, mutation_budget)?;
            select_object_route(directory, &requirement.procedure, &mut selected_shards)?;
            actor_context_queries.push((requirement, query));
        } else if packet_type == LAW_MUTATION_INGRESS {
            let mutation = serde_json::from_value::<crate::LegalMutation>(
                payload.get("mutation").cloned().ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::InvalidPayload,
                        "legal mutation ingress requires a mutation",
                    )
                })?,
            )
            .map_err(|error| {
                CanwuError::new(
                    ErrorCode::InvalidPayload,
                    format!("invalid legal mutation: {error}"),
                )
            })?;
            reserve_collected_mutation(&mut collected_mutations, mutation_budget)?;
            select_mutation_scope(&mutation, plan, directory, &mut selected_shards)?;
            mutations.push((ingress.id, mutation));
        } else if packet_type == LAW_WAKE_INGRESS {
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
            reserve_collected_mutation(&mut collected_mutations, mutation_budget)?;
            let due_routes =
                directory.directory.due_shards.get(&due_at).ok_or_else(|| {
                    invalid_record("legal wake has no persisted bounded shard route")
                })?;
            selected_shards.extend(due_routes.iter().cloned());
            wake_requested.push(due_at);
        } else if packet_type == LAW_OUTBOX_PREPARE_INGRESS {
            let sequence = payload_u64(payload, "sequence", "legal outbox preparation")?;
            let expected_revision =
                payload_u64(payload, "expected_revision", "legal outbox preparation")?;
            match preparations.entry(sequence) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    reserve_collected_mutation(&mut collected_mutations, mutation_budget)?;
                    entry.insert(expected_revision);
                }
                std::collections::btree_map::Entry::Occupied(entry) => {
                    if *entry.get() != expected_revision {
                        return Err(CanwuError::new(
                            ErrorCode::IdempotencyConflict,
                            "conflicting legal outbox preparations share one sequence",
                        ));
                    }
                }
            }
        } else if packet_type == LAW_OUTBOX_ACK_INGRESS {
            let (sequence, expected_revision) =
                sequence_and_revision(payload, "legal outbox acknowledgement")?;
            let candidate = OutboxAcknowledgementAdmission {
                expected_revision,
                ingress_id: ingress.id,
                controller_request_id: payload_optional_u64(
                    payload,
                    "controller_request_id",
                    "legal outbox acknowledgement",
                )?,
                create_request_id: payload_u64(
                    payload,
                    "create_request_id",
                    "legal outbox acknowledgement",
                )?,
                ticket_id: payload_u64(payload, "ticket_id", "legal outbox acknowledgement")?,
                draft_hash: payload
                    .get("draft_hash")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        CanwuError::new(
                            ErrorCode::InvalidPayload,
                            "legal outbox acknowledgement requires a draft hash",
                        )
                    })?
                    .to_owned(),
                outcome_commitment: payload
                    .get("outcome_commitment")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        CanwuError::new(
                            ErrorCode::InvalidPayload,
                            "legal outbox acknowledgement requires an outcome commitment",
                        )
                    })?
                    .to_owned(),
            };
            match acknowledgements.entry(sequence) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    reserve_collected_mutation(&mut collected_mutations, mutation_budget)?;
                    entry.insert(candidate);
                }
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    if !entry.get().semantically_matches(&candidate) {
                        return Err(CanwuError::new(
                            ErrorCode::IdempotencyConflict,
                            "conflicting legal outbox acknowledgements share one sequence",
                        ));
                    }
                    if candidate.ingress_id < entry.get().ingress_id {
                        entry.get_mut().ingress_id = candidate.ingress_id;
                    }
                }
            }
        }
    }
    if intents.is_empty()
        && acknowledgements.is_empty()
        && preparations.is_empty()
        && actor_context_queries.is_empty()
        && mutations.is_empty()
        && wake_requested.is_empty()
        && archive_commits.is_empty()
        && archive_retention_acknowledgements.is_empty()
    {
        return Ok(BoundaryProposal::default());
    }
    intents.sort_by(|left, right| left.id.cmp(&right.id));
    actor_context_queries.sort_by(|left, right| {
        (&left.0.procedure, left.0.stage, left.0.round, &left.0.seat).cmp(&(
            &right.0.procedure,
            right.0.stage,
            right.0.round,
            &right.0.seat,
        ))
    });
    let (plan_record, plan_state, directory_record, directory, plan) = header.ok_or_else(|| {
        CanwuError::new(
            ErrorCode::InvalidDomainRecord,
            "legal persistence header could not be loaded for admitted legal ingress",
        )
    })?;
    let mut runtime = load_legal_runtime_from_view_scope(
        view,
        plan_record,
        plan_state,
        directory_record,
        directory,
        &plan,
        &selected_shards,
        true,
    )?;
    runtime.validate_live_plan_binding(&plan)?;
    archive_commits.sort_by(|left, right| left.compaction.token.cmp(&right.compaction.token));
    for commit in archive_commits {
        runtime.commit_verified_legal_archive(&commit)?;
    }
    for (retention_handle_id, acknowledgement) in archive_retention_acknowledgements {
        runtime.acknowledge_archive_retention_terminal(
            &retention_handle_id,
            &acknowledgement.compaction_token,
            &acknowledgement.directory_root,
            acknowledgement.disposition,
            &acknowledgement.chain_root,
        )?;
    }
    wake_requested.sort();
    wake_requested.dedup();
    for due_at in wake_requested {
        runtime.consume_wake(due_at)?;
    }
    let mut signals = Vec::new();
    for (ingress_id, mutation) in mutations {
        verify_legal_mutation(view, &plan, &mutation)?;
        match mutation {
            crate::LegalMutation::SubmitProposal { proposal } => {
                runtime.submit_proposal_within_boundary(&plan, proposal)?;
            }
            crate::LegalMutation::AdmitNonProceduralSource { proposal } => {
                let admitted_signal_kinds = verified_required_signal_kinds(view, &plan, &proposal)?;
                runtime.admit_non_procedural_source_within_boundary(
                    &plan,
                    proposal,
                    &admitted_signal_kinds,
                    context.at,
                )?;
            }
            crate::LegalMutation::Signal { signal } => {
                let proposal = runtime.proposals.get(&signal.proposal_id).ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::InvalidDomainRecord,
                        "legal signal references an unknown proposal",
                    )
                })?;
                verify_proposal_inputs(view, proposal)?;
                let active_procedure = proposal.active_procedure.clone();
                if let Some(procedure) = active_procedure {
                    runtime.dirty_procedures.insert(procedure);
                }
                signals.push(signal);
            }
            crate::LegalMutation::RetireCulturalTarget { target, reason } => runtime
                .retire_cultural_target_from_ingress(
                    &plan,
                    &target,
                    context.at,
                    reason,
                    EvidenceRef::Ingress(ingress_id),
                )?,
            crate::LegalMutation::RecordCase { case } => runtime.record_case(&plan, case)?,
            crate::LegalMutation::RecordFinding { finding } => {
                runtime.record_finding(&plan, finding)?;
            }
            crate::LegalMutation::RecordRuling { ruling } => {
                runtime.record_ruling(&plan, ruling)?;
            }
            crate::LegalMutation::RecordConflict { conflict } => {
                runtime.record_conflict(&plan, conflict)?;
            }
            crate::LegalMutation::RecordPublicity { publicity } => {
                runtime.record_publicity_at(&plan, publicity, context.at)?;
            }
            crate::LegalMutation::RecordSuccession { succession } => {
                runtime.record_succession_for_plan(&plan, succession)?;
            }
            crate::LegalMutation::AdmitCapacity { allocation } => {
                runtime.admit_capacity_allocation(&plan, allocation)?;
            }
        }
    }
    for (requirement, query) in actor_context_queries {
        if query.after.is_some()
            || query.limit == 0
            || usize::try_from(query.limit)
                .ok()
                .is_none_or(|limit| limit > runtime.budgets.max_evidence_per_record)
        {
            return Err(CanwuError::new(
                ErrorCode::KnowledgeLimitExceeded,
                "legal actor context requires one bounded, unpaginated knowledge query",
            ));
        }
        let result = view.knowledge_records(requirement.holder.clone(), &query)?;
        if result.next.is_some() || result.holder != requirement.holder {
            return Err(CanwuError::new(
                ErrorCode::KnowledgeLimitExceeded,
                "legal actor context query must fit in one holder-bound page",
            ));
        }
        let actor_context = crate::runtime::actor_context_from_query_result(&result)?;
        runtime.stage_actor_context(&requirement, actor_context)?;
    }
    for (sequence, expected_revision) in preparations {
        if runtime
            .outbox
            .get(&sequence)
            .and_then(|item| item.enqueue_expected_revision)
            .is_some_and(|existing| existing != expected_revision)
        {
            verify_outbox_can_reprepare(view, &runtime, sequence)?;
        }
        runtime.stage_outbox_expected_revision(sequence, expected_revision)?;
    }
    for (sequence, acknowledgement) in acknowledgements {
        verify_outbox_enqueue(
            view,
            &runtime,
            sequence,
            acknowledgement.expected_revision,
            acknowledgement.controller_request_id,
            acknowledgement.create_request_id,
            acknowledgement.ticket_id,
            &acknowledgement.draft_hash,
            &acknowledgement.outcome_commitment,
        )?;
        runtime.mark_outbox_enqueued(
            sequence,
            acknowledgement.expected_revision,
            EvidenceRef::Ingress(acknowledgement.ingress_id),
            acknowledgement.outcome_commitment,
        )?;
    }
    for intent in intents {
        runtime.queue_authorized_pending_intent(intent)?;
    }
    verify_pending_adoption_guards(view, &runtime)?;
    runtime.settle_boundary(&plan, context.at, &signals)?;
    let mut directives = Vec::new();
    if let Some(due_at) = runtime.next_due_time()
        && !runtime.scheduled_wakes.contains(&due_at)
    {
        let after = due_at.checked_sub(context.at).ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidBoundary,
                "legal due time precedes this boundary",
            )
        })?;
        runtime.mark_wake_scheduled(due_at)?;
        directives.push(BoundaryDirective::ScheduleIngress {
            after,
            packet_type: LAW_WAKE_INGRESS.to_owned(),
            priority: 0,
            payload: serde_json::json!({ "due_at": due_at.as_minutes() }),
            affected: Vec::new(),
        });
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
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

fn reserve_collected_mutation(count: &mut usize, budget: usize) -> Result<(), CanwuError> {
    if *count >= budget {
        return Err(CanwuError::new(
            ErrorCode::ValueOutOfRange,
            "legal ingress mutation budget exhausted",
        ));
    }
    *count += 1;
    Ok(())
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

fn verify_evidence(
    view: &SimulationView<'_>,
    evidence: impl IntoIterator<Item = EvidenceRef>,
) -> Result<(), CanwuError> {
    for reference in evidence {
        if !view.evidence_exists(&reference)? {
            return Err(CanwuError::new(
                ErrorCode::InvalidAuthority,
                "legal mutation references unavailable evidence",
            ));
        }
    }
    Ok(())
}

fn verify_expected_versions(
    view: &SimulationView<'_>,
    versions: &[canwu_api::DomainRecordVersionRef],
) -> Result<(), CanwuError> {
    for version in versions {
        if !view.domain_record_version_is_current(version)? {
            return Err(CanwuError::new(
                ErrorCode::InvalidAuthority,
                "legal proposal host record compare-and-set failed",
            ));
        }
    }
    Ok(())
}

fn verify_proposal_inputs(
    view: &SimulationView<'_>,
    proposal: &crate::LegalProposal,
) -> Result<(), CanwuError> {
    verify_expected_versions(view, &proposal.expected_versions)?;
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
    mutation: &crate::LegalMutation,
) -> Result<(), CanwuError> {
    match mutation {
        crate::LegalMutation::SubmitProposal { proposal }
        | crate::LegalMutation::AdmitNonProceduralSource { proposal } => {
            verify_proposal_inputs(view, proposal)
        }
        crate::LegalMutation::Signal { signal } => {
            verify_evidence(view, signal.evidence.iter().cloned())?;
            verify_signal_provenance(view, plan, &signal.kind, &signal.evidence)
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
            verify_evidence(view, publicity.evidence.iter().cloned())?;
            verify_publicity_provenance(view, plan, publicity)
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
    view: &SimulationView<'_>,
    plan: &crate::CompiledLawPlan,
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
        verify_signal_provenance(view, plan, kind, &proposal.evidence)?;
    }
    Ok(profile.required_signal_kinds.clone())
}

fn verify_signal_provenance(
    view: &SimulationView<'_>,
    plan: &crate::CompiledLawPlan,
    kind: &str,
    evidence: &[EvidenceRef],
) -> Result<(), CanwuError> {
    let provider = plan.signal_provider_by_kind.get(kind).ok_or_else(|| {
        CanwuError::new(
            ErrorCode::InvalidAuthority,
            format!("legal signal kind {kind} has no compiled provider"),
        )
    })?;
    for evidence in evidence {
        if let EvidenceRef::Ingress(id) = evidence
            && view.plugin_ingress_matches(*id, &provider.plugin, &provider.packet_type)?
        {
            return Ok(());
        }
    }
    Err(CanwuError::new(
        ErrorCode::InvalidAuthority,
        format!(
            "legal signal kind {kind} lacks ingress from provider {}:{}",
            provider.plugin, provider.packet_type
        ),
    ))
}

fn verify_publicity_provenance(
    view: &SimulationView<'_>,
    plan: &crate::CompiledLawPlan,
    publicity: &crate::LegalPublicityEvent,
) -> Result<(), CanwuError> {
    let provider = plan
        .signal_provider_by_kind
        .get(&publicity.signal_kind)
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidAuthority,
                format!(
                    "legal publicity signal kind {} has no compiled provider",
                    publicity.signal_kind
                ),
            )
        })?;
    let expected_payload = serde_json::json!({
        "proposal": publicity.proposal.id,
        "at": publicity.at.as_minutes(),
        "signal_kind": publicity.signal_kind,
        "medium": publicity.medium,
        "scope": publicity.scope,
    });
    for evidence in &publicity.evidence {
        if let EvidenceRef::Ingress(id) = evidence
            && view.plugin_ingress_payload_matches(
                *id,
                &provider.plugin,
                &provider.packet_type,
                publicity.at,
                &expected_payload,
            )?
        {
            return Ok(());
        }
    }
    Err(CanwuError::new(
        ErrorCode::InvalidAuthority,
        format!(
            "legal publicity {} lacks an exact retained provider payload from {}:{}",
            publicity.id, provider.plugin, provider.packet_type
        ),
    ))
}

fn verify_pending_adoption_guards(
    view: &SimulationView<'_>,
    runtime: &LegalRuntime,
) -> Result<(), CanwuError> {
    for procedure_id in &runtime.dirty_procedures {
        let Some(proposal) = runtime
            .procedures
            .get(procedure_id)
            .and_then(|procedure| runtime.proposals.get(&procedure.proposal.id))
        else {
            continue;
        };
        verify_proposal_inputs(view, proposal)?;
        if proposal.cultural_dependencies.iter().any(|dependency| {
            runtime
                .retired_cultural_targets
                .contains(&dependency.target)
        }) {
            return Err(CanwuError::new(
                ErrorCode::InvalidAuthority,
                "legal adoption depends on a retired culture generation",
            ));
        }
    }
    Ok(())
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
pub fn load_legal_runtime(
    canwu: &Canwu,
    plan: &crate::CompiledLawPlan,
) -> Result<Option<LegalRuntime>, CanwuError> {
    validate_law_records(canwu)?;
    let Some(plan_record) = canwu.typed_domain_record(&legal_plan_state_reference()) else {
        return Ok(None);
    };
    let directory_record = canwu
        .typed_domain_record(&legal_directory_state_reference())
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
        let shard_record = canwu
            .typed_domain_record(&legal_shard_state_reference(shard)?)
            .ok_or_else(|| invalid_record("law directory references a missing shard record"))?;
        let archive_record = canwu
            .typed_domain_record(&legal_archive_head_state_reference(shard)?)
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
pub fn load_law_state_for_plan(
    canwu: &Canwu,
    plan: &crate::CompiledLawPlan,
) -> Result<Option<LegalRuntime>, CanwuError> {
    load_legal_runtime(canwu, plan)
}

fn validate_law_records(canwu: &Canwu) -> Result<(), CanwuError> {
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
