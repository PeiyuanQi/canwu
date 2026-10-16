use super::state::{ArchivedCommandRequestOutcome, ArchivedIngressRequest};
use super::{
    ADMISSION_CURSOR_FORMAT_VERSION, BoundaryReceipt, BoundaryRecord, BoundaryRequest, CanwuError,
    CauseRef, CommandAttemptOutcome, CommandAttemptRecord, CommandEnvelope, CommandOutcome,
    CommandReceipt, CommandRecord, CommandRequest, CommitmentRoots, DecisionArchiveBlob,
    DecisionArchiveBucketPage, DecisionArchiveProvider, DecisionHistoryKey,
    DecisionHistoryLocation, DecisionState, DeterministicRng, DomainRecord, DomainRecordPageRoots,
    DomainRecordRef, DomainRecordSchema, DomainRecordType, DomainRecordVersionRef,
    DomainRecordVersionSource, ENGINE_VERSION, ErrorCode, EvidenceRef, IngressPayload,
    IngressReceipt, IngressRecord, KnowledgeSnapshot, OutboxEntry, PayloadProperty, PayloadSchema,
    PayloadValueType, PluginComponentRecord, PluginDescriptor, PluginIngressRequest,
    PreparedDecisionArchive, PreparedStateDelta, RandomDrawRecord, RandomStreamState,
    RunConfigurationSnapshot, RunManifest, RuntimeEvidence, SNAPSHOT_FORMAT_VERSION,
    STATE_REVISION_FORMAT_VERSION, Scenario, ScheduledAction, ScheduledRecord, SchemaRegistry,
    SimDuration, SimEvent, SimTime, Simulation, SimulationPlugin, StatePageBlob, StatePageProvider,
    StatePageRetentionLedger, StatePageStore, SystemCadence, TypedDomainRecordRef, WorldSnapshot,
    canonical_byte_hash, has_unqueued_command_history, invalid_snapshot_error, is_canonical_hash,
    is_one_u64, is_zero_u32, is_zero_u64, one_u64, prepare_state_delta, verify_state_delta,
};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// The index of a snapshot's initial-scenario records, built the first time
/// validation resolves evidence against it and then shared with the runtime
/// restored from the snapshot. A clone starts empty, since it may be edited;
/// the cache never takes part in equality or the encoding.
#[derive(Default)]
pub(super) struct InitialRecordIndexCache(
    std::sync::OnceLock<Arc<super::state::InitialRecordIndex>>,
);

impl Clone for InitialRecordIndexCache {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl PartialEq for InitialRecordIndexCache {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl InitialRecordIndexCache {
    fn seeded(index: Arc<super::state::InitialRecordIndex>) -> Self {
        Self(std::sync::OnceLock::from(index))
    }
}

impl SimulationSnapshot {
    /// The index of this snapshot's initial-scenario records, built once.
    pub(super) fn initial_record_index(&self) -> Arc<super::state::InitialRecordIndex> {
        Arc::clone(
            self.initial_record_index.0.get_or_init(|| {
                super::state::InitialRecordIndex::of(self.initial_scenario.as_ref())
            }),
        )
    }
}

impl std::fmt::Debug for InitialRecordIndexCache {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("InitialRecordIndexCache")
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SimulationSnapshot {
    pub engine_version: String,
    pub snapshot_format_version: u32,
    #[serde(default)]
    pub run_manifest: Option<RunManifest>,
    #[serde(default)]
    pub run_manifest_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_configuration: Option<RunConfigurationSnapshot>,
    #[serde(default)]
    pub checkpoint_hash: String,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    /// Version of the domain-separated checkpoint commitment contract.
    pub commitment_format_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Persisted canonical roots verified before a snapshot becomes live.
    pub commitment_roots: Option<CommitmentRoots>,
    #[serde(default)]
    /// Version of the revision and checkpoint sub-contract.
    pub revision_format_version: u32,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    /// Monotonic revision after all persisted attempt and boundary transactions.
    pub state_revision: u64,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    /// Revision-evidence format available to exact replay.
    pub replay_revision_format_version: u32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    /// Version of the persisted boundary-admission cursor contract.
    pub admission_cursor_format_version: u32,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    /// Number of command-attempt records consumed by completed boundaries.
    pub admitted_attempt_count: u64,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    /// Number of accepted-command records consumed by completed boundaries.
    pub admitted_command_count: u64,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    /// Number of event records consumed as boundary ingress.
    pub admitted_event_count: u64,
    pub initial_time: SimTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_scenario: Option<Scenario>,
    #[serde(skip)]
    pub(super) initial_record_index: InitialRecordIndexCache,
    pub now: SimTime,
    pub plugin_registration_closed: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entities: Vec<super::EntityRef>,
    #[serde(default, skip_serializing_if = "legacy_world_is_empty")]
    pub world: WorldSnapshot,
    /// Core person life and custody state; absent persons are alive and free.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub person_availability: BTreeMap<super::PersonId, super::PersonAvailability>,
    /// Runtime-created persons in allocation order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub created_persons: Vec<super::CreatedPerson>,
    pub knowledge: KnowledgeSnapshot,
    pub events: Vec<SimEvent>,
    pub commands: Vec<CommandRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command_attempts: Vec<CommandAttemptRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ingress: Vec<IngressRecord>,
    #[serde(default)]
    pub boundaries: Vec<BoundaryRecord>,
    pub plugin_components: Vec<PluginComponentRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub domain_records: Vec<DomainRecord>,
    #[serde(default, skip_serializing_if = "DecisionState::is_empty")]
    pub decisions: DecisionState,
    pub plugin_descriptors: Vec<PluginDescriptor>,
    pub schema: SchemaRegistry,
    #[serde(default)]
    pub root_seed: u64,
    /// Authority credential root, intentionally independent from simulation RNG.
    #[serde(default)]
    pub authority_root_seed: u64,
    #[serde(default)]
    pub random_streams: Vec<RandomStreamState>,
    #[serde(default)]
    pub random_draws: Vec<RandomDrawRecord>,
    pub(super) scheduled: Vec<ScheduledRecord>,
    /// Registered transition manifests awaiting their ready boundary, in
    /// manifest-ID order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_transition_manifests: Vec<super::PendingTransitionManifest>,
    #[serde(default, rename = "rng", skip_serializing_if = "Option::is_none")]
    pub(super) legacy_rng: Option<DeterministicRng>,
    pub(super) next_event_id: u64,
    pub(super) next_command_id: u64,
    #[serde(default = "one_u64", skip_serializing_if = "is_one_u64")]
    pub(super) next_command_attempt_id: u64,
    #[serde(default = "one_u64", skip_serializing_if = "is_one_u64")]
    pub(super) next_ingress_id: u64,
    #[serde(default)]
    pub(super) next_boundary_id: u64,
    #[serde(default)]
    pub(super) next_random_draw_id: u64,
    #[serde(default = "one_u64", skip_serializing_if = "is_one_u64")]
    pub(super) next_knowledge_record_id: u64,
    pub(super) next_schedule_sequence: u64,
    pub(super) next_correlation_id: u64,
    #[serde(default = "one_u64", skip_serializing_if = "is_one_u64")]
    pub(super) next_decision_trace_id: u64,
    /// Zero until the first runtime-created person.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub(super) next_person_id: u64,
}

/// Format of the paged-checkpoint envelope and of the decision pages it
/// names. Format 5 names domain-record pages in state-page codec
/// `raw-canonical-v2`, whose records may span record-value chunk pages, and
/// embeds a `SimulationCheckpoint` without keyed-draw reservations. Format 6
/// embeds snapshot format 9, whose generic knowledge is bodies and holder
/// entries; the knowledge ledger stays inside the envelope page.
pub const PAGED_CHECKPOINT_FORMAT_VERSION: u32 = 6;
const MAX_PAGED_DECISION_DIRECTORY_ENTRIES: usize = 1_024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PagedSimulationCheckpoint {
    pub format_version: u32,
    pub root_page_id: String,
    pub checkpoint_hash: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedPagedSimulationCheckpoint {
    pub checkpoint: PagedSimulationCheckpoint,
    pub delta: PreparedStateDelta,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PortablePagedSimulationCheckpoint {
    pub checkpoint: PagedSimulationCheckpoint,
    pub pages: Vec<StatePageBlob>,
}

/// Unified offline GC mark set for kernel-owned pages, evidence, decision
/// blobs, and namespaced plugin archive objects.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArchiveReachabilityManifest {
    pub state_page_ids: BTreeSet<String>,
    pub evidence_segment_ids: BTreeSet<String>,
    pub decision_blob_ids: BTreeSet<String>,
    pub plugin_objects: BTreeMap<String, BTreeSet<String>>,
}

impl ArchiveReachabilityManifest {
    pub fn insert_plugin_object(&mut self, namespace: impl Into<String>, id: impl Into<String>) {
        self.plugin_objects
            .entry(namespace.into())
            .or_default()
            .insert(id.into());
    }

    pub fn merge(&mut self, other: Self) {
        self.state_page_ids.extend(other.state_page_ids);
        self.evidence_segment_ids.extend(other.evidence_segment_ids);
        self.decision_blob_ids.extend(other.decision_blob_ids);
        for (namespace, ids) in other.plugin_objects {
            self.plugin_objects
                .entry(namespace)
                .or_default()
                .extend(ids);
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct PagedCheckpointEnvelope {
    format_version: u32,
    checkpoint_without_paged_state: SimulationCheckpoint,
    domain_records: DomainRecordPageRoots,
    decision_manifest_page_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct PagedDecisionManifest {
    format_version: u32,
    hot_page_id: String,
    archive_receipt_root: String,
    archive_receipt_count: u64,
    archive_directory_page_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct PagedDecisionDirectoryPage {
    format_version: u32,
    archive_bucket_pages: Vec<(super::DecisionArchivePageKey, String)>,
}

impl PagedDecisionDirectoryPage {
    fn validate(&self) -> Result<(), CanwuError> {
        if self.format_version != PAGED_CHECKPOINT_FORMAT_VERSION
            || self.archive_bucket_pages.is_empty()
            || self.archive_bucket_pages.len() > MAX_PAGED_DECISION_DIRECTORY_ENTRIES
            || self
                .archive_bucket_pages
                .windows(2)
                .any(|pair| pair[0].0 >= pair[1].0)
        {
            return Err(invalid_snapshot_error(
                "paged decision directory page is malformed",
            ));
        }
        for (_, page_id) in &self.archive_bucket_pages {
            if !is_canonical_hash(page_id) {
                return Err(invalid_snapshot_error(
                    "paged decision archive bucket page ID is malformed",
                ));
            }
        }
        Ok(())
    }
}

fn validate_paged_decision_manifest(manifest: &PagedDecisionManifest) -> Result<(), CanwuError> {
    let unique_directory_ids = manifest
        .archive_directory_page_ids
        .iter()
        .collect::<BTreeSet<_>>();
    if manifest.format_version != PAGED_CHECKPOINT_FORMAT_VERSION
        || !is_canonical_hash(&manifest.hot_page_id)
        || !is_canonical_hash(&manifest.archive_receipt_root)
        || manifest
            .archive_directory_page_ids
            .iter()
            .any(|page_id| !is_canonical_hash(page_id))
        || unique_directory_ids.len() != manifest.archive_directory_page_ids.len()
        || (manifest.archive_receipt_count == 0) != manifest.archive_directory_page_ids.is_empty()
    {
        return Err(invalid_snapshot_error(
            "paged decision manifest directory is malformed",
        ));
    }
    Ok(())
}

fn assemble_paged_decision_directory(
    pages: Vec<PagedDecisionDirectoryPage>,
) -> Result<BTreeMap<super::DecisionArchivePageKey, String>, CanwuError> {
    let page_count = pages.len();
    let mut archive_bucket_pages = BTreeMap::new();
    let mut previous_page_key = None;
    for (directory_ordinal, directory_page) in pages.into_iter().enumerate() {
        directory_page.validate()?;
        let expected_len = if directory_ordinal + 1 == page_count {
            1..=MAX_PAGED_DECISION_DIRECTORY_ENTRIES
        } else {
            MAX_PAGED_DECISION_DIRECTORY_ENTRIES..=MAX_PAGED_DECISION_DIRECTORY_ENTRIES
        };
        if !expected_len.contains(&directory_page.archive_bucket_pages.len()) {
            return Err(invalid_snapshot_error(
                "paged decision directory uses noncanonical page chunking",
            ));
        }
        for (page_key, page_id) in directory_page.archive_bucket_pages {
            if previous_page_key.is_some_and(|previous| previous >= page_key) {
                return Err(invalid_snapshot_error(
                    "paged decision directory is not globally strictly ordered",
                ));
            }
            previous_page_key = Some(page_key);
            if archive_bucket_pages.insert(page_key, page_id).is_some() {
                return Err(invalid_snapshot_error(
                    "paged decision directory contains a duplicate bucket key",
                ));
            }
        }
    }
    Ok(archive_bucket_pages)
}

impl PreparedPagedSimulationCheckpoint {
    pub fn store_and_verify(&self, store: &dyn StatePageStore) -> Result<(), CanwuError> {
        if self.checkpoint.format_version != PAGED_CHECKPOINT_FORMAT_VERSION
            || self.delta.target_root != self.checkpoint.root_page_id
        {
            return Err(invalid_snapshot_error(
                "paged checkpoint descriptor disagrees with its prepared delta",
            ));
        }
        for page in &self.delta.new_pages {
            let _ = store.store_state_page(page)?;
        }
        verify_state_delta(&self.delta, store)?;
        let envelope = store
            .load_state_page(&self.checkpoint.root_page_id)?
            .ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::StatePageUnavailable,
                    "paged checkpoint root is unavailable after storage",
                )
            })?;
        envelope.validate()?;
        Ok(())
    }
}

#[derive(Default)]
struct EmbeddedPageProvider {
    pages: BTreeMap<String, StatePageBlob>,
}

impl StatePageProvider for EmbeddedPageProvider {
    fn load_state_page(&self, page_id: &str) -> Result<Option<StatePageBlob>, CanwuError> {
        Ok(self.pages.get(page_id).cloned())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PagedCheckpointScaleMetrics {
    pub decision_entries: u64,
    pub decision_locator: super::DecisionLocatorScaleMetrics,
    pub state_pages: u64,
    pub decision_directory_pages: u64,
    pub max_state_page_bytes: u64,
    pub initial_delta_pages: u64,
    pub repeat_delta_pages: u64,
    pub single_page_change_delta_pages: u64,
    pub initial_provider_calls: u64,
    pub repeat_provider_calls: u64,
    pub single_page_change_provider_calls: u64,
    pub exact_restart_queries: u64,
    pub restored_root_matches: bool,
    pub replayed_root_matches: bool,
    pub root_page_id: String,
}

struct Format8ScalePageStore {
    pages: RefCell<BTreeMap<String, StatePageBlob>>,
    decision_blobs: RefCell<BTreeMap<String, DecisionArchiveBlob>>,
    provider_calls: RefCell<u64>,
}

impl StatePageProvider for Format8ScalePageStore {
    fn load_state_page(&self, page_id: &str) -> Result<Option<StatePageBlob>, CanwuError> {
        *self.provider_calls.borrow_mut() += 1;
        Ok(self.pages.borrow().get(page_id).cloned())
    }
}

impl StatePageStore for Format8ScalePageStore {
    fn store_state_page(&self, page: &StatePageBlob) -> Result<ArchiveStoreOutcome, CanwuError> {
        page.validate()?;
        let mut pages = self.pages.borrow_mut();
        if let Some(existing) = pages.get(&page.page_id) {
            if existing != page {
                return Err(invalid_snapshot_error(
                    "Format-8 scale store page ID contains different bytes",
                ));
            }
            return Ok(ArchiveStoreOutcome::AlreadyPresent);
        }
        pages.insert(page.page_id.clone(), page.clone());
        Ok(ArchiveStoreOutcome::Stored)
    }
}

impl DecisionArchiveProvider for Format8ScalePageStore {
    fn load_decision_archive(
        &self,
        locator: &str,
    ) -> Result<Option<DecisionArchiveBlob>, super::DecisionError> {
        Ok(self.decision_blobs.borrow().get(locator).cloned())
    }

    fn load_decision_archive_bucket_page(
        &self,
        page_id: &str,
    ) -> Result<Option<DecisionArchiveBucketPage>, super::DecisionError> {
        let Some(page) = self.pages.borrow().get(page_id).cloned() else {
            return Ok(None);
        };
        serde_json::from_slice(&page.bytes)
            .map(Some)
            .map_err(|error| {
                super::DecisionError::new(
                    super::DecisionErrorCode::DecisionHistoryUnavailable,
                    format!("cannot decode decision bucket state page: {error}"),
                )
            })
    }
}

fn legacy_world_is_empty(world: &WorldSnapshot) -> bool {
    world.people.is_empty()
        && world.governments.is_empty()
        && world.territories.is_empty()
        && world.routes.is_empty()
        && world.armies.is_empty()
        && world.letters.is_empty()
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
/// Complete recorded environment and input journal for exact replay.
pub struct ReplayJournal {
    pub engine_version: String,
    pub snapshot_format_version: u32,
    pub root_seed: u64,
    /// Self-contained initial scenario used by exact replay. Callers never
    /// provide a second scenario that could diverge from the journal.
    pub initial_scenario: Scenario,
    pub authority_root_seed: u64,
    pub run_manifest: RunManifest,
    pub run_manifest_hash: String,
    pub run_configuration: RunConfigurationSnapshot,
    pub plugin_descriptors: Vec<PluginDescriptor>,
    pub plugin_registration_closed: bool,
    pub commands: Vec<CommandRecord>,
    pub command_attempts: Vec<CommandAttemptRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ingress: Vec<IngressRecord>,
    pub boundaries: Vec<BoundaryRecord>,
    pub final_time: SimTime,
    pub checkpoint_hash: String,
    /// Checkpoint commitment format reproduced by exact replay.
    pub commitment_format_version: u32,
    /// Revision-evidence format verified by this exact replay journal.
    pub revision_format_version: u32,
    /// Final persisted authoritative revision after replay.
    pub final_revision: u64,
}

/// Version of current-state checkpoints plus append-only evidence segments.
/// Format 6 journals each knowledge publication as its bodies and per-holder
/// entries, with one `KnowledgePublished` event naming every holder.
pub const CHECKPOINT_JOURNAL_FORMAT_VERSION: u32 = 6;
const ARCHIVED_SEGMENT_MANIFEST_DOMAIN: &str = "canwu.evidence.archived-segment-manifest.v1";
pub(crate) const ARCHIVED_RECEIPT_DOMAIN: &str = "canwu.evidence.archived-receipts.v2";
const EVIDENCE_DEPENDENCY_DOMAIN: &str = "canwu.evidence.dependencies.v1";
/// Compact checkpoint identity. Version 2 drops the keyed-reservation root:
/// sealed keyed draws stay in the keyed-draw ledger instead.
const COMPACT_CHECKPOINT_DOMAIN: &str = "canwu.compact-checkpoint.v2";
/// Two-phase seal token, bumped with the compact checkpoint identity it binds.
const SEAL_TOKEN_DOMAIN: &str = "canwu.evidence.seal-token.v2";
/// Reserved domain-record payload field declaring a payload-reading continuation.
pub const PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD: &str =
    "canwu_payload_required_evidence_continuation";
/// Current wire version of [`PayloadRequiredEvidenceContinuationV1`].
pub const PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FORMAT_VERSION: u32 = 1;
/// Reserved domain-record payload field declaring retained identity proofs.
pub const IDENTITY_EVIDENCE_DEPENDENCIES_FIELD: &str = "canwu_identity_evidence_dependencies";
/// Current wire version of [`IdentityEvidenceDependenciesV1`].
pub const IDENTITY_EVIDENCE_DEPENDENCIES_FORMAT_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceJournalKind {
    Event,
    Command,
    CommandAttempt,
    Ingress,
    Boundary,
    RandomDraw,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EvidenceNestedLocator {
    None,
    BoundaryRecordChange { change_index: u64 },
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct EvidenceItemLocator {
    pub journal: EvidenceJournalKind,
    pub absolute_index: u64,
    pub nested: EvidenceNestedLocator,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ArchivedEvidenceLocator {
    pub segment_id: String,
    pub item: EvidenceItemLocator,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ArchivedEvidenceReceipt {
    pub evidence: EvidenceRef,
    pub locator: ArchivedEvidenceLocator,
    pub evidence_index_leaf: u64,
    pub item_commitment: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_ingress_provenance: Option<ArchivedPluginIngressProvenance>,
    pub merkle_path: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ArchivedPluginIngressProvenance {
    pub plugin: String,
    pub packet_type: String,
    pub producer_boundary: super::BoundaryId,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct EvidenceIndexEntry {
    pub reference: EvidenceRef,
    pub item: EvidenceItemLocator,
    pub item_commitment: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_ingress_provenance: Option<ArchivedPluginIngressProvenance>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EvidenceJournalRoots {
    pub events: String,
    pub commands: String,
    pub command_attempts: String,
    pub ingress: String,
    pub boundaries: String,
    pub random_draws: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ArchivedSegmentHeader {
    pub segment_id: String,
    pub start: EvidenceCursor,
    pub end: EvidenceCursor,
    pub journal_roots: EvidenceJournalRoots,
    pub evidence_index_root: String,
    pub evidence_index_entry_count: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceRequirement {
    /// Only the committed evidence identity and causal cut must remain provable.
    IdentityOnly,
    /// An active schema-declared continuation must be able to reload the payload.
    PayloadRequired,
}

/// Authoritative pending-continuation contract for rules that must inspect old payload bytes.
///
/// A plugin opts in by declaring [`PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD`]
/// as a required object in a versioned domain-record schema. The current active
/// domain record stores this object in its payload. Because both schema and
/// record participate in plugin identity, state commitments, replay, and normal
/// mutation validation, dependencies cannot be supplied as an ephemeral seal hint.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PayloadRequiredEvidenceContinuationV1 {
    pub format_version: u32,
    pub active: bool,
    pub dependencies: Vec<EvidenceRef>,
}

impl PayloadRequiredEvidenceContinuationV1 {
    /// Builds an active V1 continuation. Dependencies must be sorted and unique.
    #[must_use]
    pub fn active(dependencies: Vec<EvidenceRef>) -> Self {
        Self {
            format_version: PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FORMAT_VERSION,
            active: true,
            dependencies,
        }
    }

    /// Builds the canonical terminal form, which retains no payload dependency.
    #[must_use]
    pub const fn completed() -> Self {
        Self {
            format_version: PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FORMAT_VERSION,
            active: false,
            dependencies: Vec::new(),
        }
    }
}

/// Returns the reserved property a domain-record schema must declare to
/// authoritatively produce `PayloadRequired` dependencies.
#[must_use]
pub fn payload_required_evidence_continuation_property_v1() -> PayloadProperty {
    PayloadProperty {
        value_type: PayloadValueType::Object,
        required: true,
    }
}

/// Authoritative identity-only evidence dependencies for a live domain record.
///
/// Unlike a payload-required continuation, this compact contract retains only
/// the Merkle receipt needed to prove identity and typed provenance. Empty
/// dependencies are the canonical terminal form and release old receipts.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IdentityEvidenceDependenciesV1 {
    pub format_version: u32,
    pub dependencies: Vec<EvidenceRef>,
}

impl IdentityEvidenceDependenciesV1 {
    /// Builds a V1 dependency declaration. Dependencies must be sorted and unique.
    #[must_use]
    pub const fn new(dependencies: Vec<EvidenceRef>) -> Self {
        Self {
            format_version: IDENTITY_EVIDENCE_DEPENDENCIES_FORMAT_VERSION,
            dependencies,
        }
    }
}

/// Returns the reserved property a domain-record schema must declare to
/// authoritatively produce `IdentityOnly` dependencies.
#[must_use]
pub fn identity_evidence_dependencies_property_v1() -> PayloadProperty {
    PayloadProperty {
        value_type: PayloadValueType::Object,
        required: true,
    }
}
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct EvidenceDependency {
    pub reference: EvidenceRef,
    pub requirement: EvidenceRequirement,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EvidenceArchiveIndex {
    pub header: ArchivedSegmentHeader,
    pub entries: Vec<EvidenceIndexEntry>,
}

pub trait ArchiveProvider {
    fn load_evidence_segment(
        &self,
        segment_id: &str,
    ) -> Result<Option<EvidenceJournalSegment>, CanwuError>;
}

pub trait ArchiveStore: ArchiveProvider {
    fn store_evidence_segment(
        &self,
        segment: &EvidenceJournalSegment,
    ) -> Result<ArchiveStoreOutcome, CanwuError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArchiveStoreOutcome {
    Stored,
    AlreadyPresent,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EvidenceSealToken {
    pub source_state_hash: String,
    pub source_checkpoint_hash: String,
    pub source_end: EvidenceCursor,
    pub segment_id: String,
    pub target_checkpoint_hash: String,
    pub token_hash: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PreparedEvidenceSeal {
    pub token: EvidenceSealToken,
    pub segment: EvidenceJournalSegment,
}

#[derive(Serialize)]
struct EvidenceIndexLeafMaterial<'a> {
    format_version: u32,
    reference: &'a EvidenceRef,
    item: &'a EvidenceItemLocator,
    item_commitment: &'a str,
    plugin_ingress_provenance: &'a Option<ArchivedPluginIngressProvenance>,
}

#[derive(Serialize)]
struct ArchivedSegmentHeaderMaterial<'a> {
    start: EvidenceCursor,
    end: EvidenceCursor,
    journal_roots: &'a EvidenceJournalRoots,
    evidence_index_root: &'a str,
    evidence_index_entry_count: u64,
}

fn archive_error(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidArchive, message)
}

fn decode_hash(value: &str, label: &str) -> Result<[u8; 32], CanwuError> {
    if value.len() != 64
        || value
            .bytes()
            .any(|byte| !byte.is_ascii_hexdigit() || byte.is_ascii_uppercase())
    {
        return Err(archive_error(format!(
            "{label} must be 32-byte lower-case hex"
        )));
    }
    let mut bytes = [0_u8; 32];
    for (index, pair) in value.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let digit = |byte: u8| match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            _ => None,
        };
        bytes[index] = digit(pair[0])
            .and_then(|high| digit(pair[1]).map(|low| (high << 4) | low))
            .ok_or_else(|| archive_error(format!("{label} contains invalid hex")))?;
    }
    Ok(bytes)
}

fn archive_node(left: [u8; 32], right: [u8; 32]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"canwu.evidence.index.node.v1");
    hasher.update(&[0]);
    hasher.update(&left);
    hasher.update(&right);
    *hasher.finalize().as_bytes()
}

fn archive_empty_root() -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"canwu.evidence.index.empty.v1");
    hasher.update(&[0]);
    hasher.finalize().to_hex().to_string()
}

pub(crate) fn skipped_commitment_root<T: Serialize>(
    domain: &str,
    values: &[T],
) -> Result<Option<String>, CanwuError> {
    if values.is_empty() {
        Ok(None)
    } else {
        super::canonical_hash(domain, values).map(Some)
    }
}

fn validate_skipped_commitment_root<T: Serialize>(
    root: Option<&str>,
    domain: &str,
    values: &[T],
    label: &str,
) -> Result<(), CanwuError> {
    let expected = skipped_commitment_root(domain, values)?;
    if root != expected.as_deref() {
        return Err(invalid_snapshot_error(format!(
            "compact continuation {label} does not match its canonical material"
        )));
    }
    Ok(())
}

fn promote_dependency(
    dependencies: &mut BTreeMap<EvidenceRef, EvidenceRequirement>,
    reference: EvidenceRef,
    requirement: EvidenceRequirement,
) {
    dependencies
        .entry(reference)
        .and_modify(|current| *current = (*current).max(requirement))
        .or_insert(requirement);
}
fn schema_declares_payload_required_continuation(schema: &DomainRecordSchema) -> bool {
    matches!(
        &schema.payload_schema,
        PayloadSchema::Object { properties, .. }
            if properties.get(PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD)
                == Some(&payload_required_evidence_continuation_property_v1())
    )
}

fn schema_declares_identity_evidence_dependencies(schema: &DomainRecordSchema) -> bool {
    matches!(
        &schema.payload_schema,
        PayloadSchema::Object { properties, .. }
            if properties.get(IDENTITY_EVIDENCE_DEPENDENCIES_FIELD)
                == Some(&identity_evidence_dependencies_property_v1())
    )
}

fn add_identity_evidence_dependencies(
    dependencies: &mut BTreeMap<EvidenceRef, EvidenceRequirement>,
    record: &DomainRecord,
    schema: &DomainRecordSchema,
) -> Result<(), CanwuError> {
    if !record.is_active() || !schema_declares_identity_evidence_dependencies(schema) {
        return Ok(());
    }
    let declaration = record
        .payload
        .get(IDENTITY_EVIDENCE_DEPENDENCIES_FIELD)
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::ArchiveNotReady,
                format!(
                    "identity-evidence record {} is missing its schema-declared field",
                    record.reference
                ),
            )
        })?;
    let declaration: IdentityEvidenceDependenciesV1 = serde_json::from_value(declaration.clone())
        .map_err(|error| {
        CanwuError::new(
            ErrorCode::ArchiveNotReady,
            format!(
                "identity-evidence record {} is invalid: {error}",
                record.reference
            ),
        )
    })?;
    if declaration.format_version != IDENTITY_EVIDENCE_DEPENDENCIES_FORMAT_VERSION {
        return Err(CanwuError::new(
            ErrorCode::ArchiveNotReady,
            format!(
                "identity-evidence record {} uses unsupported format {}",
                record.reference, declaration.format_version
            ),
        ));
    }
    if declaration
        .dependencies
        .windows(2)
        .any(|pair| pair[0] >= pair[1])
    {
        return Err(CanwuError::new(
            ErrorCode::ArchiveNotReady,
            format!(
                "identity-evidence record {} needs sorted unique dependencies",
                record.reference
            ),
        ));
    }
    if declaration.dependencies.iter().any(|reference| {
        matches!(
            reference,
            EvidenceRef::DomainRecordVersion(version)
                if version.record == record.reference && version.version == record.version
        )
    }) {
        return Err(CanwuError::new(
            ErrorCode::ArchiveNotReady,
            format!(
                "identity-evidence record {} cannot depend on its own version",
                record.reference
            ),
        ));
    }
    for reference in declaration.dependencies {
        if matches!(
            &reference,
            EvidenceRef::DomainRecordVersion(version)
                if matches!(version.established_by, DomainRecordVersionSource::InitialScenario)
        ) {
            return Err(CanwuError::new(
                ErrorCode::ArchiveNotReady,
                format!(
                    "identity-evidence record {} cannot archive initial-scenario evidence",
                    record.reference
                ),
            ));
        }
        promote_dependency(dependencies, reference, EvidenceRequirement::IdentityOnly);
    }
    Ok(())
}

fn add_payload_required_continuation_dependencies(
    dependencies: &mut BTreeMap<EvidenceRef, EvidenceRequirement>,
    record: &DomainRecord,
    schema: &DomainRecordSchema,
) -> Result<(), CanwuError> {
    if !record.is_active() || !schema_declares_payload_required_continuation(schema) {
        return Ok(());
    }
    let continuation = record
        .payload
        .get(PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD)
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::ArchiveNotReady,
                format!(
                    "payload-required continuation record {} is missing its schema-declared field",
                    record.reference
                ),
            )
        })?;
    let continuation: PayloadRequiredEvidenceContinuationV1 =
        serde_json::from_value(continuation.clone()).map_err(|error| {
            CanwuError::new(
                ErrorCode::ArchiveNotReady,
                format!(
                    "payload-required continuation record {} is invalid: {error}",
                    record.reference
                ),
            )
        })?;
    if continuation.format_version != PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FORMAT_VERSION {
        return Err(CanwuError::new(
            ErrorCode::ArchiveNotReady,
            format!(
                "payload-required continuation record {} uses unsupported format {}",
                record.reference, continuation.format_version
            ),
        ));
    }
    if !continuation.active {
        if continuation == PayloadRequiredEvidenceContinuationV1::completed() {
            return Ok(());
        }
        return Err(CanwuError::new(
            ErrorCode::ArchiveNotReady,
            format!(
                "completed payload-required continuation record {} retains dependencies",
                record.reference
            ),
        ));
    }
    let continuation = PayloadRequiredEvidenceContinuationV1::active(continuation.dependencies);
    if continuation.dependencies.is_empty()
        || continuation
            .dependencies
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
    {
        return Err(CanwuError::new(
            ErrorCode::ArchiveNotReady,
            format!(
                "active payload-required continuation record {} needs sorted unique dependencies",
                record.reference
            ),
        ));
    }
    if continuation.dependencies.iter().any(|reference| {
        matches!(
            reference,
            EvidenceRef::DomainRecordVersion(version)
                if version.record == record.reference && version.version == record.version
        )
    }) {
        return Err(CanwuError::new(
            ErrorCode::ArchiveNotReady,
            format!(
                "payload-required continuation record {} cannot depend on its own version",
                record.reference
            ),
        ));
    }
    for reference in continuation.dependencies {
        if matches!(
            &reference,
            EvidenceRef::DomainRecordVersion(version)
                if matches!(version.established_by, DomainRecordVersionSource::InitialScenario)
        ) {
            return Err(CanwuError::new(
                ErrorCode::ArchiveNotReady,
                format!(
                    "payload-required continuation record {} cannot depend on an initial-scenario payload",
                    record.reference
                ),
            ));
        }
        promote_dependency(
            dependencies,
            reference,
            EvidenceRequirement::PayloadRequired,
        );
    }
    Ok(())
}

fn reachable_archived_receipt_references(
    dependencies: &[EvidenceDependency],
) -> BTreeSet<EvidenceRef> {
    dependencies
        .iter()
        .map(|dependency| dependency.reference.clone())
        .collect()
}

/// Exact idempotency entries for tracked requests, built from journal slices
/// by a seal and by a continuation checkpoint of an unsealed run.
#[derive(Default)]
pub(super) struct ArchivedRequestTables {
    pub(super) commands: Vec<(super::CommandRequestId, ArchivedCommandRequestOutcome)>,
    pub(super) ingress: Vec<(super::CommandRequestId, ArchivedIngressRequest)>,
    pub(super) decisions: Vec<(super::DecisionRequestId, ArchivedIngressRequest)>,
    pub(super) decision_commands: Vec<super::CommandRequestId>,
}

/// Keeps the receipts of current dependencies. A dependency first cited after
/// the seal that archived its evidence has no receipt to keep; see
/// [`Simulation::receipt_required_references`] for which ones must have one.
fn retain_reachable_archived_evidence_receipts(
    receipts: &mut im::OrdMap<EvidenceRef, Arc<ArchivedEvidenceReceipt>>,
    dependencies: &[EvidenceDependency],
) {
    let reachable = reachable_archived_receipt_references(dependencies);
    if receipts
        .keys()
        .all(|reference| reachable.contains(reference))
    {
        return;
    }
    // Rebuilt rather than removed from: a removal from the persistent map
    // holds several of its nodes on the stack at once in a debug build, which
    // overflows a 1 MiB thread.
    *receipts = receipts
        .iter()
        .filter(|(reference, _)| reachable.contains(*reference))
        .map(|(reference, receipt)| (reference.clone(), Arc::clone(receipt)))
        .collect();
}

pub(crate) fn load_verified_archived_evidence_segment(
    receipt: &ArchivedEvidenceReceipt,
    provider: &dyn ArchiveProvider,
) -> Result<EvidenceJournalSegment, CanwuError> {
    let segment = provider
        .load_evidence_segment(&receipt.locator.segment_id)?
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::EvidenceContentUnavailable,
                "the archive provider did not return the required evidence segment",
            )
        })?;
    let receipts = verify_archived_segment(&segment).map_err(|error| {
        CanwuError::new(
            ErrorCode::InvalidArchive,
            format!(
                "archive provider returned invalid content: {}",
                error.message
            ),
        )
    })?;
    if !receipts.iter().any(|candidate| candidate == receipt) {
        return Err(archive_error(
            "archive provider segment does not reproduce the committed evidence receipt",
        ));
    }
    Ok(segment)
}

fn add_cause_dependency(
    dependencies: &mut BTreeMap<EvidenceRef, EvidenceRequirement>,
    cause: &CauseRef,
) {
    let reference = match cause {
        CauseRef::Event(id) => Some(EvidenceRef::Event(*id)),
        CauseRef::Command(id) => Some(EvidenceRef::Command(*id)),
        CauseRef::Boundary(id) => Some(EvidenceRef::Boundary(*id)),
        CauseRef::System(_) => None,
    };
    if let Some(reference) = reference {
        promote_dependency(dependencies, reference, EvidenceRequirement::IdentityOnly);
    }
}

fn add_command_outcome_dependencies(
    dependencies: &mut BTreeMap<EvidenceRef, EvidenceRequirement>,
    outcome: &CommandOutcome,
) {
    let (attempt_id, command_id, emitted_events) = match outcome {
        CommandOutcome::Accepted { receipt } => (
            receipt.attempt_id,
            Some(receipt.command_id),
            receipt.emitted_events.as_slice(),
        ),
        CommandOutcome::Rejected { rejection } => (rejection.attempt_id, None, &[][..]),
    };
    if let Some(id) = attempt_id {
        promote_dependency(
            dependencies,
            EvidenceRef::CommandAttempt(id),
            EvidenceRequirement::IdentityOnly,
        );
    }
    if let Some(id) = command_id {
        promote_dependency(
            dependencies,
            EvidenceRef::Command(id),
            EvidenceRequirement::IdentityOnly,
        );
    }
    for id in emitted_events {
        promote_dependency(
            dependencies,
            EvidenceRef::Event(*id),
            EvidenceRequirement::IdentityOnly,
        );
    }
}

fn archive_leaf(entry: &EvidenceIndexEntry) -> Result<[u8; 32], CanwuError> {
    let hash = super::canonical_hash(
        "canwu.evidence.index.leaf.v2",
        &EvidenceIndexLeafMaterial {
            format_version: 2,
            reference: &entry.reference,
            item: &entry.item,
            item_commitment: &entry.item_commitment,
            plugin_ingress_provenance: &entry.plugin_ingress_provenance,
        },
    )?;
    decode_hash(&hash, "evidence-index leaf")
}

fn archive_merkle(
    entries: &[EvidenceIndexEntry],
) -> Result<(String, Vec<Vec<String>>), CanwuError> {
    if entries.is_empty() {
        return Ok((archive_empty_root(), Vec::new()));
    }
    let mut level: Vec<[u8; 32]> = entries.iter().map(archive_leaf).collect::<Result<_, _>>()?;
    let mut positions: Vec<usize> = (0..entries.len()).collect();
    let mut proofs = vec![Vec::new(); entries.len()];
    while level.len() > 1 {
        for (leaf, position) in positions.iter().copied().enumerate() {
            let sibling = if position % 2 == 0 {
                (position + 1).min(level.len() - 1)
            } else {
                position - 1
            };
            proofs[leaf].push(
                blake3::Hash::from_bytes(level[sibling])
                    .to_hex()
                    .to_string(),
            );
        }
        let mut next = Vec::with_capacity(level.len().div_ceil(2));
        for pair in level.chunks(2) {
            next.push(archive_node(pair[0], *pair.get(1).unwrap_or(&pair[0])));
        }
        level = next;
        for position in &mut positions {
            *position /= 2;
        }
    }
    Ok((
        blake3::Hash::from_bytes(level[0]).to_hex().to_string(),
        proofs,
    ))
}

fn item_commitment<T: Serialize>(
    journal: EvidenceJournalKind,
    item: &T,
) -> Result<String, CanwuError> {
    let domain = match journal {
        EvidenceJournalKind::Event => "canwu.evidence.item.event.v1",
        EvidenceJournalKind::Command => "canwu.evidence.item.command.v1",
        EvidenceJournalKind::CommandAttempt => "canwu.evidence.item.command_attempt.v1",
        EvidenceJournalKind::Ingress => "canwu.evidence.item.ingress.v1",
        EvidenceJournalKind::Boundary => "canwu.evidence.item.boundary.v1",
        EvidenceJournalKind::RandomDraw => "canwu.evidence.item.random_draw.v1",
    };
    super::canonical_hash(domain, item)
}

pub(crate) fn evidence_archive_index(
    segment: &EvidenceJournalSegment,
) -> Result<(EvidenceArchiveIndex, Vec<ArchivedEvidenceReceipt>), CanwuError> {
    let roots = EvidenceJournalRoots {
        events: super::canonical_hash("canwu.evidence.journal.events.v1", &segment.events)?,
        commands: super::canonical_hash("canwu.evidence.journal.commands.v1", &segment.commands)?,
        command_attempts: super::canonical_hash(
            "canwu.evidence.journal.command_attempts.v1",
            &segment.command_attempts,
        )?,
        ingress: super::canonical_hash("canwu.evidence.journal.ingress.v1", &segment.ingress)?,
        boundaries: super::canonical_hash(
            "canwu.evidence.journal.boundaries.v1",
            &segment.boundaries,
        )?,
        random_draws: super::canonical_hash(
            "canwu.evidence.journal.random_draws.v1",
            &segment.random_draws,
        )?,
    };
    let mut entries = Vec::new();
    let mut add = |reference: EvidenceRef,
                   journal,
                   absolute_index,
                   nested,
                   commitment: String,
                   plugin_ingress_provenance| {
        entries.push(EvidenceIndexEntry {
            reference,
            item: EvidenceItemLocator {
                journal,
                absolute_index,
                nested,
            },
            item_commitment: commitment,
            plugin_ingress_provenance,
        });
    };
    for (offset, event) in segment.events.iter().enumerate() {
        add(
            EvidenceRef::Event(event.id),
            EvidenceJournalKind::Event,
            segment.start.event_count + offset as u64 + 1,
            EvidenceNestedLocator::None,
            item_commitment(EvidenceJournalKind::Event, event)?,
            None,
        );
    }
    for (offset, command) in segment.commands.iter().enumerate() {
        add(
            EvidenceRef::Command(command.id),
            EvidenceJournalKind::Command,
            segment.start.command_count + offset as u64 + 1,
            EvidenceNestedLocator::None,
            item_commitment(EvidenceJournalKind::Command, command)?,
            None,
        );
    }
    for (offset, attempt) in segment.command_attempts.iter().enumerate() {
        add(
            EvidenceRef::CommandAttempt(attempt.id),
            EvidenceJournalKind::CommandAttempt,
            segment.start.command_attempt_count + offset as u64 + 1,
            EvidenceNestedLocator::None,
            item_commitment(EvidenceJournalKind::CommandAttempt, attempt)?,
            None,
        );
    }
    let cancelled_ingress: BTreeSet<_> = segment
        .ingress
        .iter()
        .filter_map(|record| match &record.payload {
            IngressPayload::PluginCancellation { cancelled, .. } => Some(*cancelled),
            _ => None,
        })
        .collect();
    for (offset, ingress) in segment.ingress.iter().enumerate() {
        let plugin_ingress_provenance = match (&ingress.payload, ingress.cause.as_ref()) {
            (
                IngressPayload::Plugin {
                    plugin,
                    packet_type,
                    ..
                },
                Some(CauseRef::Boundary(producer_boundary)),
            ) if !cancelled_ingress.contains(&ingress.id)
                && segment.boundaries.iter().any(|boundary| {
                    boundary.id == *producer_boundary
                        && boundary.generated_ingress.iter().any(|generation| {
                            generation.ingress == ingress.id && generation.plugin == *plugin
                        })
                }) =>
            {
                Some(ArchivedPluginIngressProvenance {
                    plugin: plugin.clone(),
                    packet_type: packet_type.clone(),
                    producer_boundary: *producer_boundary,
                })
            }
            _ => None,
        };
        add(
            EvidenceRef::Ingress(ingress.id),
            EvidenceJournalKind::Ingress,
            segment.start.ingress_count + offset as u64 + 1,
            EvidenceNestedLocator::None,
            item_commitment(EvidenceJournalKind::Ingress, ingress)?,
            plugin_ingress_provenance,
        );
    }
    for (offset, boundary) in segment.boundaries.iter().enumerate() {
        let absolute_index = segment.start.boundary_count + offset as u64 + 1;
        let commitment = item_commitment(EvidenceJournalKind::Boundary, boundary)?;
        add(
            EvidenceRef::Boundary(boundary.id),
            EvidenceJournalKind::Boundary,
            absolute_index,
            EvidenceNestedLocator::None,
            commitment.clone(),
            None,
        );
        for (change_index, change) in boundary.record_changes.iter().enumerate() {
            add(
                EvidenceRef::DomainRecordVersion(DomainRecordVersionRef {
                    record: change.current.reference.clone(),
                    version: change.current.version,
                    established_by: DomainRecordVersionSource::BoundaryChange {
                        boundary: boundary.id,
                        change_index: change_index as u64,
                    },
                }),
                EvidenceJournalKind::Boundary,
                absolute_index,
                EvidenceNestedLocator::BoundaryRecordChange {
                    change_index: change_index as u64,
                },
                commitment.clone(),
                None,
            );
        }
    }
    for (offset, draw) in segment.random_draws.iter().enumerate() {
        add(
            EvidenceRef::RandomDraw(draw.id),
            EvidenceJournalKind::RandomDraw,
            segment.start.random_draw_count + offset as u64 + 1,
            EvidenceNestedLocator::None,
            item_commitment(EvidenceJournalKind::RandomDraw, draw)?,
            None,
        );
    }
    entries.sort();
    if entries
        .windows(2)
        .any(|window| window[0].reference == window[1].reference)
    {
        return Err(archive_error(
            "evidence archive contains duplicate references",
        ));
    }
    let (evidence_index_root, proofs) = archive_merkle(&entries)?;
    let entry_count = u64::try_from(entries.len())
        .map_err(|_| archive_error("evidence-index entry count exceeds u64"))?;
    let segment_id = super::canonical_hash(
        "canwu.evidence.segment.v3",
        &ArchivedSegmentHeaderMaterial {
            start: segment.start,
            end: segment.end,
            journal_roots: &roots,
            evidence_index_root: &evidence_index_root,
            evidence_index_entry_count: entry_count,
        },
    )?;
    let header = ArchivedSegmentHeader {
        segment_id: segment_id.clone(),
        start: segment.start,
        end: segment.end,
        journal_roots: roots,
        evidence_index_root,
        evidence_index_entry_count: entry_count,
    };
    let receipts = entries
        .iter()
        .zip(proofs)
        .enumerate()
        .map(|(index, (entry, merkle_path))| ArchivedEvidenceReceipt {
            evidence: entry.reference.clone(),
            locator: ArchivedEvidenceLocator {
                segment_id: segment_id.clone(),
                item: entry.item.clone(),
            },
            evidence_index_leaf: index as u64,
            item_commitment: entry.item_commitment.clone(),
            plugin_ingress_provenance: entry.plugin_ingress_provenance.clone(),
            merkle_path,
        })
        .collect();
    Ok((EvidenceArchiveIndex { header, entries }, receipts))
}

fn verify_archive_receipt(
    receipt: &ArchivedEvidenceReceipt,
    header: &ArchivedSegmentHeader,
) -> Result<(), CanwuError> {
    if receipt.locator.segment_id != header.segment_id
        || receipt.evidence_index_leaf >= header.evidence_index_entry_count
    {
        return Err(archive_error(
            "archived evidence receipt does not belong to its segment header",
        ));
    }
    let entry = EvidenceIndexEntry {
        reference: receipt.evidence.clone(),
        item: receipt.locator.item.clone(),
        item_commitment: receipt.item_commitment.clone(),
        plugin_ingress_provenance: receipt.plugin_ingress_provenance.clone(),
    };
    let mut hash = archive_leaf(&entry)?;
    let mut position = receipt.evidence_index_leaf;
    let mut width = header.evidence_index_entry_count;
    let mut path_at = 0usize;
    while width > 1 {
        let sibling = receipt
            .merkle_path
            .get(path_at)
            .ok_or_else(|| archive_error("archived evidence receipt Merkle path is too short"))?;
        let sibling = decode_hash(sibling, "receipt Merkle sibling")?;
        hash = if position.is_multiple_of(2) {
            archive_node(hash, sibling)
        } else {
            archive_node(sibling, hash)
        };
        position /= 2;
        width = width.div_ceil(2);
        path_at += 1;
    }
    if path_at != receipt.merkle_path.len()
        || blake3::Hash::from_bytes(hash).to_hex().as_str() != header.evidence_index_root
    {
        return Err(archive_error(
            "archived evidence receipt Merkle proof is invalid",
        ));
    }
    let (expected_journal, expected_nested) = match &receipt.evidence {
        EvidenceRef::Event(_) => (EvidenceJournalKind::Event, EvidenceNestedLocator::None),
        EvidenceRef::Command(_) => (EvidenceJournalKind::Command, EvidenceNestedLocator::None),
        EvidenceRef::CommandAttempt(_) => (
            EvidenceJournalKind::CommandAttempt,
            EvidenceNestedLocator::None,
        ),
        EvidenceRef::Ingress(_) => (EvidenceJournalKind::Ingress, EvidenceNestedLocator::None),
        EvidenceRef::Boundary(_) => (EvidenceJournalKind::Boundary, EvidenceNestedLocator::None),
        EvidenceRef::RandomDraw(_) => {
            (EvidenceJournalKind::RandomDraw, EvidenceNestedLocator::None)
        }
        EvidenceRef::DomainRecordVersion(version) => match version.established_by {
            DomainRecordVersionSource::BoundaryChange { change_index, .. } => (
                EvidenceJournalKind::Boundary,
                EvidenceNestedLocator::BoundaryRecordChange { change_index },
            ),
            DomainRecordVersionSource::InitialScenario => {
                return Err(archive_error(
                    "initial-scenario record versions cannot be archived",
                ));
            }
        },
    };
    if receipt.locator.item.journal != expected_journal
        || receipt.locator.item.nested != expected_nested
    {
        return Err(archive_error(
            "archived evidence receipt uses an illegal typed locator",
        ));
    }
    let (start, end) = match expected_journal {
        EvidenceJournalKind::Event => (header.start.event_count, header.end.event_count),
        EvidenceJournalKind::Command => (header.start.command_count, header.end.command_count),
        EvidenceJournalKind::CommandAttempt => (
            header.start.command_attempt_count,
            header.end.command_attempt_count,
        ),
        EvidenceJournalKind::Ingress => (header.start.ingress_count, header.end.ingress_count),
        EvidenceJournalKind::Boundary => (header.start.boundary_count, header.end.boundary_count),
        EvidenceJournalKind::RandomDraw => {
            (header.start.random_draw_count, header.end.random_draw_count)
        }
    };
    if receipt.locator.item.absolute_index <= start || receipt.locator.item.absolute_index > end {
        return Err(archive_error(
            "archived evidence locator lies outside its segment cursor range",
        ));
    }
    Ok(())
}

pub(crate) fn verify_archived_segment(
    segment: &EvidenceJournalSegment,
) -> Result<Vec<ArchivedEvidenceReceipt>, CanwuError> {
    let stored = segment
        .archive
        .as_ref()
        .ok_or_else(|| archive_error("archived segment is missing its evidence index"))?;
    let mut plain = segment.clone();
    plain.archive = None;
    let (rebuilt, receipts) = evidence_archive_index(&plain)?;
    if &rebuilt != stored {
        return Err(archive_error(
            "archived segment header or evidence index does not match its journal content",
        ));
    }
    for receipt in &receipts {
        verify_archive_receipt(receipt, &stored.header)?;
    }
    Ok(receipts)
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
/// Monotonic cuts through every append-only evidence journal.
pub struct EvidenceCursor {
    pub event_count: u64,
    pub command_count: u64,
    pub command_attempt_count: u64,
    pub ingress_count: u64,
    pub boundary_count: u64,
    pub random_draw_count: u64,
}

impl EvidenceCursor {
    fn from_evidence(evidence: &RuntimeEvidence) -> Result<Self, CanwuError> {
        let count = |len: usize, label: &str| {
            u64::try_from(len).map_err(|_| {
                CanwuError::new(
                    ErrorCode::IdentifierExhausted,
                    format!("{label} journal length exceeds the persistent cursor space"),
                )
            })
        };
        Ok(Self {
            event_count: evidence
                .archived
                .event_count
                .checked_add(count(evidence.events.len(), "event")?)
                .ok_or_else(|| invalid_snapshot_error("event journal cursor is exhausted"))?,
            command_count: evidence
                .archived
                .command_count
                .checked_add(count(evidence.commands.len(), "command")?)
                .ok_or_else(|| invalid_snapshot_error("command journal cursor is exhausted"))?,
            command_attempt_count: evidence
                .archived
                .command_attempt_count
                .checked_add(count(evidence.command_attempts.len(), "command-attempt")?)
                .ok_or_else(|| {
                    invalid_snapshot_error("command-attempt journal cursor is exhausted")
                })?,
            ingress_count: evidence
                .archived
                .ingress_count
                .checked_add(count(evidence.ingress.len(), "ingress")?)
                .ok_or_else(|| invalid_snapshot_error("ingress journal cursor is exhausted"))?,
            boundary_count: evidence
                .archived
                .boundary_count
                .checked_add(count(evidence.boundaries.len(), "boundary")?)
                .ok_or_else(|| invalid_snapshot_error("boundary journal cursor is exhausted"))?,
            random_draw_count: evidence
                .archived
                .random_draw_count
                .checked_add(count(evidence.random_draws.len(), "random-draw")?)
                .ok_or_else(|| invalid_snapshot_error("random-draw journal cursor is exhausted"))?,
        })
    }

    pub(super) fn checked_advance(
        self,
        segment: &EvidenceJournalSegment,
    ) -> Result<Self, CanwuError> {
        let advance = |value: u64, len: usize, label: &str| {
            value
                .checked_add(u64::try_from(len).map_err(|_| {
                    invalid_snapshot_error(format!(
                        "{label} journal segment exceeds the persistent cursor space"
                    ))
                })?)
                .ok_or_else(|| {
                    invalid_snapshot_error(format!(
                        "{label} journal cursor exceeds the persistent cursor space"
                    ))
                })
        };
        Ok(Self {
            event_count: advance(self.event_count, segment.events.len(), "event")?,
            command_count: advance(self.command_count, segment.commands.len(), "command")?,
            command_attempt_count: advance(
                self.command_attempt_count,
                segment.command_attempts.len(),
                "command-attempt",
            )?,
            ingress_count: advance(self.ingress_count, segment.ingress.len(), "ingress")?,
            boundary_count: advance(self.boundary_count, segment.boundaries.len(), "boundary")?,
            random_draw_count: advance(
                self.random_draw_count,
                segment.random_draws.len(),
                "random-draw",
            )?,
        })
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
/// Current authoritative state plus the journal cut required to validate it.
///
/// `state` deliberately contains empty append-only evidence arrays. It is not a
/// standalone `SimulationSnapshot`; load it only with the contiguous evidence
/// segments ending at `journal_end`.
pub struct SimulationCheckpoint {
    pub format_version: u32,
    pub journal_end: EvidenceCursor,
    pub state: SimulationSnapshot,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub archived_segment_headers: Vec<ArchivedSegmentHeader>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_segment_manifest_root: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub archived_evidence_receipts: Vec<ArchivedEvidenceReceipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_receipt_root: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_dependencies: Vec<EvidenceDependency>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_dependency_root: Option<String>,
}

#[derive(Serialize)]
struct CompactCheckpointHashMaterial<'a> {
    state_checkpoint_hash: &'a str,
    journal_end: EvidenceCursor,
    archived_segment_manifest_root: Option<&'a str>,
    archived_receipt_root: Option<&'a str>,
    evidence_dependency_root: Option<&'a str>,
}

fn validate_compact_continuation(checkpoint: &SimulationCheckpoint) -> Result<(), CanwuError> {
    if checkpoint
        .archived_segment_headers
        .windows(2)
        .any(|headers| headers[0].end != headers[1].start)
        || checkpoint
            .archived_segment_headers
            .iter()
            .map(|header| &header.segment_id)
            .collect::<BTreeSet<_>>()
            .len()
            != checkpoint.archived_segment_headers.len()
    {
        return Err(invalid_snapshot_error(
            "compact archived-segment manifest is duplicated or noncontiguous",
        ));
    }
    if checkpoint
        .archived_evidence_receipts
        .windows(2)
        .any(|receipts| receipts[0].evidence >= receipts[1].evidence)
    {
        return Err(invalid_snapshot_error(
            "compact archived receipts must be sorted by unique evidence reference",
        ));
    }
    if checkpoint
        .evidence_dependencies
        .windows(2)
        .any(|dependencies| dependencies[0].reference >= dependencies[1].reference)
    {
        return Err(invalid_snapshot_error(
            "compact evidence dependencies must be sorted by unique evidence reference",
        ));
    }
    validate_skipped_commitment_root(
        checkpoint.archived_segment_manifest_root.as_deref(),
        ARCHIVED_SEGMENT_MANIFEST_DOMAIN,
        &checkpoint.archived_segment_headers,
        "archived-segment manifest root",
    )?;
    validate_skipped_commitment_root(
        checkpoint.archived_receipt_root.as_deref(),
        ARCHIVED_RECEIPT_DOMAIN,
        &checkpoint.archived_evidence_receipts,
        "archived-receipt root",
    )?;
    validate_skipped_commitment_root(
        checkpoint.evidence_dependency_root.as_deref(),
        EVIDENCE_DEPENDENCY_DOMAIN,
        &checkpoint.evidence_dependencies,
        "evidence-dependency root",
    )
}

impl SimulationCheckpoint {
    /// Returns every segment ID reachable from all retained checkpoint manifests.
    ///
    /// Hosts must include every checkpoint they still promise to restore before
    /// treating a stored segment absent from this set as an orphan candidate.
    pub fn reachable_archive_segment_ids(
        retained_checkpoints: &[Self],
    ) -> Result<BTreeSet<String>, CanwuError> {
        let mut reachable = BTreeSet::new();
        for checkpoint in retained_checkpoints {
            validate_compact_continuation(checkpoint)?;
            reachable.extend(
                checkpoint
                    .archived_segment_headers
                    .iter()
                    .map(|header| header.segment_id.clone()),
            );
        }
        Ok(reachable)
    }

    /// Returns sorted stored segment IDs that no retained manifest can reach.
    ///
    /// This method identifies host-GC candidates only; it never deletes content.
    pub fn orphaned_archive_segment_ids(
        retained_checkpoints: &[Self],
        stored_segment_ids: &[String],
    ) -> Result<Vec<String>, CanwuError> {
        let reachable = Self::reachable_archive_segment_ids(retained_checkpoints)?;
        Ok(stored_segment_ids
            .iter()
            .filter(|segment_id| !reachable.contains(segment_id.as_str()))
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect())
    }
}

impl Simulation {
    /// The compact identity a two-phase seal token binds: the state
    /// checkpoint hash, the journal end, and the roots of the segment
    /// manifest, the kept receipts, and the evidence dependencies. It equals
    /// the identity of the compact checkpoint this runtime would write, but
    /// is computed from the runtime's fields, so a continuation runtime,
    /// which cannot write that checkpoint, has one too.
    fn compact_identity_hash(&self) -> Result<String, CanwuError> {
        let dependencies = self.evidence_dependencies()?;
        let reachable = reachable_archived_receipt_references(&dependencies);
        let evidence = &self.state.evidence;
        let headers: Vec<_> = evidence.archived_segment_headers.iter().collect();
        let receipts: Vec<_> = evidence
            .archived_evidence_receipts
            .iter()
            .filter(|(reference, _)| reachable.contains(*reference))
            .map(|(_, receipt)| receipt.as_ref())
            .collect();
        let manifest_root = skipped_commitment_root(ARCHIVED_SEGMENT_MANIFEST_DOMAIN, &headers)?;
        let receipt_root = skipped_commitment_root(ARCHIVED_RECEIPT_DOMAIN, &receipts)?;
        let dependency_root = skipped_commitment_root(EVIDENCE_DEPENDENCY_DOMAIN, &dependencies)?;
        super::canonical_hash(
            COMPACT_CHECKPOINT_DOMAIN,
            &CompactCheckpointHashMaterial {
                state_checkpoint_hash: &self.state.metadata.checkpoint_hash,
                journal_end: self.evidence_cursor()?,
                archived_segment_manifest_root: manifest_root.as_deref(),
                archived_receipt_root: receipt_root.as_deref(),
                evidence_dependency_root: dependency_root.as_deref(),
            },
        )
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
/// One contiguous append-only evidence range for incremental archival.
pub struct EvidenceJournalSegment {
    pub format_version: u32,
    pub start: EvidenceCursor,
    pub end: EvidenceCursor,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<SimEvent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<CommandRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command_attempts: Vec<CommandAttemptRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ingress: Vec<IngressRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub boundaries: Vec<BoundaryRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub random_draws: Vec<RandomDrawRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive: Option<EvidenceArchiveIndex>,
}

/// A full snapshot rebuilt from a checkpoint and its evidence prefix, with
/// the compact continuation the checkpoint claimed and the one rebuilt from
/// the prefix.
struct ReconstructedCheckpoint {
    rebuilt_headers: Vec<ArchivedSegmentHeader>,
    rebuilt_receipts: BTreeMap<EvidenceRef, ArchivedEvidenceReceipt>,
    expected_headers: Vec<ArchivedSegmentHeader>,
    expected_receipts: Vec<ArchivedEvidenceReceipt>,
    expected_dependencies: Vec<EvidenceDependency>,
}

impl ReconstructedCheckpoint {
    /// Checks the claimed continuation against the runtime built from the
    /// reconstructed snapshot, so that a restore builds one runtime.
    ///
    /// A live seal keeps the receipts of the dependencies current at that seal,
    /// and a dependency first cited by identity after its seal needs none, so
    /// the claimed receipts may be fewer than the prefix can rebuild. Each
    /// claimed receipt must still equal the rebuilt one and name a current
    /// dependency.
    fn verify(self, simulation: &Simulation) -> Result<(), CanwuError> {
        let rebuilt_dependencies = simulation.evidence_dependencies()?;
        let reachable = reachable_archived_receipt_references(&rebuilt_dependencies);
        let genuine_receipts = self.expected_receipts.iter().all(|receipt| {
            reachable.contains(&receipt.evidence)
                && self.rebuilt_receipts.get(&receipt.evidence) == Some(receipt)
        });
        if self.rebuilt_headers != self.expected_headers
            || !genuine_receipts
            || rebuilt_dependencies != self.expected_dependencies
        {
            return Err(invalid_snapshot_error(
                "checkpoint compact continuation does not match its reconstructed authoritative indexes",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
/// Portable full-save bundle built from a current-state checkpoint and journal segments.
pub struct CheckpointJournal {
    pub checkpoint: SimulationCheckpoint,
    pub segments: Vec<EvidenceJournalSegment>,
}

/// A live simulation whose sealed evidence prefixes are owned by the caller.
///
/// This opt-in runtime preserves current authoritative state, deterministic
/// commitments, idempotency, and continuation behavior while retaining only
/// the evidence appended since the most recent seal. Every returned segment is
/// part of the permanent replay record and must be stored contiguously by the
/// caller. A restored runtime retains its whole supplied prefix, so its first
/// seal starts at the zero cut and replaces every earlier segment for later
/// checkpoints.
///
/// Like [`Simulation`], it is `Send`, so a host may advance, checkpoint, or
/// seal it, or a fork of it, on another thread.
pub struct CompactedSimulation {
    simulation: Simulation,
    committed_seal_tokens: im::OrdSet<String>,
}

impl CompactedSimulation {
    /// The wrapped runtime (continuation gate only).
    #[cfg(feature = "continuation-gate")]
    #[doc(hidden)]
    #[must_use]
    pub const fn gate_runtime(&self) -> &Simulation {
        &self.simulation
    }

    /// The wrapped runtime, for in-crate tests of derived runtime state.
    #[cfg(test)]
    pub(super) const fn runtime(&self) -> &Simulation {
        &self.simulation
    }

    /// Returns the monotonic cut through sealed and retained evidence.
    pub fn evidence_cursor(&self) -> Result<EvidenceCursor, CanwuError> {
        self.simulation.evidence_cursor()
    }

    /// Copies the compact runtime, sharing persistent structures, with
    /// profiling off. The copy keeps the seal tokens already committed, so it
    /// treats a repeated commit of one of them as the original does. It
    /// shares the attached plugin archive provider and may move to another
    /// thread.
    #[must_use]
    pub fn fork(&self) -> Self {
        Self {
            simulation: self.simulation.fork(),
            committed_seal_tokens: self.committed_seal_tokens.clone(),
        }
    }

    /// Attaches the caller-owned provider used to resolve package cold
    /// archives during admission, settlement, and detached queries. The
    /// provider must be `Send + Sync`, as for
    /// [`Simulation::set_plugin_archive_object_provider`].
    pub fn set_plugin_archive_object_provider(
        &mut self,
        provider: std::sync::Arc<dyn super::PluginArchiveObjectProvider + Send + Sync>,
    ) {
        self.simulation.set_plugin_archive_object_provider(provider);
    }

    /// Loads one package-owned archive object from the attached provider.
    pub fn plugin_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, CanwuError> {
        self.simulation.plugin_archive_object(namespace, object_id)
    }

    /// The commitment-format state hash, equal to the hash of an unsealed run
    /// with the same state.
    pub fn authoritative_state_hash(&self) -> Result<String, CanwuError> {
        self.simulation.authoritative_state_hash()
    }

    /// The hash of the run manifest this run was created with.
    #[must_use]
    pub fn run_manifest_hash(&self) -> &str {
        self.simulation.run_manifest_hash()
    }

    /// The persisted run configuration.
    #[must_use]
    pub const fn run_configuration(&self) -> &super::RunConfigurationSnapshot {
        self.simulation.run_configuration()
    }

    /// The run's type schema.
    #[must_use]
    pub const fn schema(&self) -> &SchemaRegistry {
        self.simulation.schema()
    }

    /// The persisted descriptors of the run's plugins.
    pub fn plugin_descriptors(&self) -> impl Iterator<Item = &PluginDescriptor> {
        self.simulation.plugin_descriptors()
    }

    /// Whether an evidence identity exists at this cut; see
    /// [`Simulation::evidence_exists`].
    #[must_use]
    pub fn evidence_exists(&self, reference: &EvidenceRef) -> bool {
        self.simulation.evidence_exists(reference)
    }

    /// When evidence became authoritative; see [`Simulation::evidence_time`].
    #[must_use]
    pub fn evidence_time(&self, reference: &EvidenceRef) -> Option<SimTime> {
        self.simulation.evidence_time(reference)
    }

    /// The body of one exact domain-record version; see
    /// [`Simulation::domain_record_version`].
    #[must_use]
    pub fn domain_record_version(
        &self,
        reference: &DomainRecordVersionRef,
    ) -> Option<DomainRecord> {
        self.simulation.domain_record_version(reference)
    }

    /// Whether an exact domain-record version exists in current, retained,
    /// or kept evidence.
    #[must_use]
    pub fn domain_record_version_evidence_exists(
        &self,
        reference: &DomainRecordVersionRef,
    ) -> bool {
        self.simulation
            .domain_record_version_evidence_exists(reference)
    }

    /// The exact identity of a record's current version, which no seal can
    /// change.
    pub fn current_domain_record_version(
        &self,
        reference: &DomainRecordRef,
    ) -> Result<Option<DomainRecordVersionRef>, CanwuError> {
        self.simulation.current_domain_record_version(reference)
    }

    /// Every current domain record.
    pub fn domain_records(&self) -> impl Iterator<Item = &DomainRecord> {
        self.simulation.domain_records()
    }

    /// One page of records of one kind, bound to an authoritative revision.
    pub fn domain_record_page(
        &self,
        kind: &super::DomainRecordKind,
        after: Option<&DomainRecordRef>,
        limit: usize,
        expected_revision: Option<u64>,
    ) -> Result<super::DomainRecordPage, CanwuError> {
        self.simulation
            .domain_record_page(kind, after, limit, expected_revision)
    }

    /// The retained event tail, after the last seal.
    #[must_use]
    pub fn events(&self) -> &[SimEvent] {
        self.simulation.events()
    }

    /// The retained accepted-command tail, after the last seal.
    #[must_use]
    pub fn command_log(&self) -> &[CommandRecord] {
        self.simulation.command_log()
    }

    /// The retained command-attempt tail, after the last seal.
    #[must_use]
    pub fn command_attempts(&self) -> &[CommandAttemptRecord] {
        self.simulation.command_attempts()
    }

    /// The retained ingress tail, after the last seal.
    #[must_use]
    pub fn ingress_log(&self) -> &[IngressRecord] {
        self.simulation.ingress_log()
    }

    /// The retained draw tail, after the last seal.
    #[must_use]
    pub fn random_draws(&self) -> &[RandomDrawRecord] {
        self.simulation.random_draws()
    }

    /// The latest committed boundary, retained or sealed.
    #[must_use]
    pub fn latest_boundary_id(&self) -> Option<super::BoundaryId> {
        self.simulation.latest_boundary_id()
    }

    /// The persisted audience declaration for an event.
    #[must_use]
    pub fn event_audience(&self, event: &SimEvent) -> canwu_event::EventAudience {
        self.simulation.event_audience(event)
    }

    /// Captures current state and the total journal cut without cloning sealed evidence.
    ///
    /// A continuation-restored run returns
    /// [`ErrorCode::HistoryUnavailable`]: restoring a compact checkpoint
    /// needs every segment since genesis. Use
    /// [`Self::continuation_checkpoint`] instead.
    pub fn checkpoint(&self) -> Result<SimulationCheckpoint, CanwuError> {
        self.simulation.checkpoint()
    }

    /// Builds the continuation checkpoint of this run at its settled cut and
    /// returns it with its continuation hash, which the host stores as the
    /// trust anchor for [`Self::from_continuation_checkpoint_verified`].
    ///
    /// Building changes nothing and fails with `ArchiveNotReady` exactly when
    /// a seal would. Every part is a function of state and history at the
    /// cut, so an unsealed run, any sealed run, and any continuation-restored
    /// run write the same checkpoint there.
    pub fn continuation_checkpoint(
        &self,
    ) -> Result<(super::ContinuationCheckpoint, String), CanwuError> {
        self.simulation.continuation_checkpoint()
    }

    /// Where this run's history begins when it was restored from a
    /// continuation checkpoint; `None` for a run that holds its history.
    #[must_use]
    pub const fn continuation_base(&self) -> Option<&super::ContinuationBase> {
        self.simulation.state.evidence.continuation_base.as_ref()
    }

    /// Restores a run from a continuation checkpoint whose state names no
    /// plugin. A checkpoint that names one fails with `PluginNotActive`; use
    /// [`Self::from_continuation_checkpoint_with_plugins`].
    ///
    /// The restore checks the checkpoint's shape and own hash, validates its
    /// state without history, recomputes every keyed draw, requires the
    /// rebuilt runtime to reproduce the checkpoint hash, and requires the
    /// checkpoint derived from the restored runtime to be the input. The own
    /// hash detects corruption only; compare it with the value stored when
    /// the checkpoint was written through
    /// [`Self::from_continuation_checkpoint_verified`].
    pub fn from_continuation_checkpoint(
        checkpoint: super::ContinuationCheckpoint,
    ) -> Result<Self, CanwuError> {
        Self::from_continuation_checkpoint_with_plugins(checkpoint, &[])
    }

    /// Restores a run from a continuation checkpoint and rehydrates its
    /// exact executable plugins.
    pub fn from_continuation_checkpoint_with_plugins(
        checkpoint: super::ContinuationCheckpoint,
        plugins: &[&dyn super::SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        let continuation_hash = checkpoint.continuation_hash.clone();
        let mut simulation = Simulation::restore_continuation(checkpoint)?;
        for plugin in plugins {
            simulation.register_plugin(*plugin)?;
        }
        simulation.ensure_runtime_ready()?;
        simulation.verify_continuation_round_trip(&continuation_hash)?;
        simulation.into_compacted()
    }

    /// Restores a run from a continuation checkpoint only if its continuation
    /// hash is the one the host stored when the checkpoint was written. This
    /// covers the carried evidence, the indexes, and the settled cut, which
    /// the checkpoint hash of the state does not.
    pub fn from_continuation_checkpoint_verified(
        checkpoint: super::ContinuationCheckpoint,
        expected_continuation_hash: &str,
        plugins: &[&dyn super::SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        if checkpoint.compute_continuation_hash()? != expected_continuation_hash {
            return Err(invalid_snapshot_error(
                "continuation checkpoint does not match the stored continuation hash",
            ));
        }
        Self::from_continuation_checkpoint_with_plugins(checkpoint, plugins)
    }

    /// Restores a run from the JSON of a continuation checkpoint.
    pub fn from_continuation_checkpoint_json_with_plugins(
        json: &str,
        plugins: &[&dyn super::SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        Self::from_continuation_checkpoint_with_plugins(
            super::ContinuationCheckpoint::from_json(json)?,
            plugins,
        )
    }

    /// Restores a run from the JSON of a continuation checkpoint only if its
    /// continuation hash is the one the host stored.
    pub fn from_continuation_checkpoint_json_verified(
        json: &str,
        expected_continuation_hash: &str,
        plugins: &[&dyn super::SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        Self::from_continuation_checkpoint_verified(
            super::ContinuationCheckpoint::from_json(json)?,
            expected_continuation_hash,
            plugins,
        )
    }

    /// Clones the retained evidence tail after `start`. Caller-owned sealed
    /// prefixes must be supplied separately when restoring the checkpoint.
    pub fn journal_segment_since(
        &self,
        start: EvidenceCursor,
    ) -> Result<EvidenceJournalSegment, CanwuError> {
        self.simulation.journal_segment_since(start)
    }

    /// Prepares an incremental content-addressed checkpoint for the compact
    /// runtime without rehydrating archived evidence or decision payloads.
    pub fn prepare_paged_checkpoint(
        &self,
        source: Option<&PagedSimulationCheckpoint>,
        provider: &dyn StatePageProvider,
    ) -> Result<PreparedPagedSimulationCheckpoint, CanwuError> {
        self.simulation.prepare_paged_checkpoint(source, provider)
    }

    /// Returns the retained canonical boundary tail. Sealed prefixes remain
    /// caller-owned evidence segments and are intentionally not rehydrated.
    #[must_use]
    pub fn boundaries(&self) -> &[BoundaryRecord] {
        self.simulation.boundaries()
    }

    /// Returns stable identities for committed boundary emissions still
    /// retained by this compact runtime. Sealed prefixes remain owned by the
    /// caller as archive segments and can be replayed from those segments.
    pub fn outbox_entries(&self) -> Result<Vec<OutboxEntry>, CanwuError> {
        self.simulation.outbox_entries()
    }

    /// Reconstructs durable delivery identities for a caller-owned sealed
    /// evidence segment. Sealing does not remove these identities; the caller
    /// can retain the segment and regenerate the same at-least-once delivery
    /// keys after restart.
    pub fn outbox_entries_for_segment(
        &self,
        segment: &EvidenceJournalSegment,
    ) -> Result<Vec<OutboxEntry>, CanwuError> {
        Simulation::outbox_entries_for_boundaries(
            &self.simulation.state.metadata.run_manifest_hash,
            &segment.boundaries,
        )
    }

    /// Returns the committed receipt for an archived evidence identity.
    #[must_use]
    pub fn archived_evidence_receipt(
        &self,
        reference: &EvidenceRef,
    ) -> Option<&ArchivedEvidenceReceipt> {
        self.simulation
            .state
            .evidence
            .archived_evidence_receipts
            .get(reference)
            .map(Arc::as_ref)
    }

    /// Loads and fully verifies the segment containing archived evidence whose
    /// payload must be inspected.
    pub fn load_archived_evidence_segment(
        &self,
        reference: &EvidenceRef,
        provider: &dyn ArchiveProvider,
    ) -> Result<EvidenceJournalSegment, CanwuError> {
        let receipt = self.archived_evidence_receipt(reference).ok_or_else(|| {
            CanwuError::new(
                ErrorCode::EvidenceUnavailable,
                "no committed archive receipt exists for the requested evidence",
            )
        })?;
        load_verified_archived_evidence_segment(receipt, provider)
    }

    /// Prepares a bounded terminal decision-history archive without mutating
    /// the authoritative checkpoint. The returned blobs must be stored and
    /// read back through the provider before commit.
    pub fn prepare_decision_archive(
        &self,
        keys: &[DecisionHistoryKey],
    ) -> Result<PreparedDecisionArchive, CanwuError> {
        self.simulation
            .state
            .current
            .decisions
            .prepare_decision_archive(keys)
            .map_err(super::decision::decision_error)
    }

    /// Verifies stored terminal decision payloads and queues the compact
    /// receipt transition as canonical maintenance ingress. Hot payloads are
    /// released only when that ingress is admitted at a normal boundary, so
    /// replay reproduces the same archive transition and checkpoint root.
    pub fn commit_decision_archive(
        &mut self,
        prepared: &PreparedDecisionArchive,
        provider: &dyn DecisionArchiveProvider,
    ) -> Result<IngressReceipt, CanwuError> {
        let verified = self
            .simulation
            .state
            .current
            .decisions
            .verify_decision_archive(prepared, provider)
            .map_err(super::decision::decision_error)?;
        let at = self.simulation.time();
        self.simulation
            .enqueue_decision_archive_commit(at, 0, verified)
    }

    /// Seals and releases the settled part of the retained evidence.
    ///
    /// Each journal moves to its settled cut: events, commands, and attempts
    /// up to the admission cursors, the others to their end. Work in flight
    /// does not block a seal: unadmitted records stay retained, and the
    /// continuation closure (pending ingress, the causes of pending work,
    /// payload-required declarations, and the identity facts of declared
    /// versions and packets) is pinned in memory, so a run with
    /// payload-required dependencies seals in one phase too. A dependency
    /// declared after its evidence was sealed fails with `ArchiveNotReady`,
    /// and the run is unchanged. Nothing settled returns `None`. The caller
    /// owns persistence and must keep all non-empty segments in exact cursor
    /// order for save restoration or replay. After a checkpoint-and-journal
    /// restore, the first seal starts at the zero cut and replaces the
    /// restored prefix.
    pub fn seal_evidence(&mut self) -> Result<Option<EvidenceJournalSegment>, CanwuError> {
        // The persistent evidence structures make this copy constant-size
        // apart from the retained tail.
        let before = self.simulation.fork();
        match self.simulation.seal_retained_evidence() {
            Ok(segment) => Ok(segment),
            Err(error) => {
                self.simulation.restore_from_fork(before);
                Err(error)
            }
        }
    }

    /// Builds an immutable, content-addressed archive candidate.
    ///
    /// The returned segment must be durably stored before
    /// [`Self::commit_evidence_seal`] is called. Preparing never changes the
    /// live simulation.
    pub fn prepare_evidence_seal(&self) -> Result<Option<PreparedEvidenceSeal>, CanwuError> {
        let source_state_hash = self.simulation.authoritative_state_hash()?;
        let source_checkpoint_hash = self.simulation.compact_identity_hash()?;
        let source_end = self.simulation.evidence_cursor()?;
        let mut candidate = self.simulation.fork();
        let Some(segment) = candidate.seal_retained_evidence()? else {
            return Ok(None);
        };
        let segment_id = segment
            .archive
            .as_ref()
            .ok_or_else(|| archive_error("prepared segment has no archive index"))?
            .header
            .segment_id
            .clone();
        let target_checkpoint_hash = candidate.compact_identity_hash()?;
        let token_hash = super::canonical_hash(
            SEAL_TOKEN_DOMAIN,
            &(
                &source_state_hash,
                &source_checkpoint_hash,
                source_end,
                &segment_id,
                &target_checkpoint_hash,
            ),
        )?;
        Ok(Some(PreparedEvidenceSeal {
            token: EvidenceSealToken {
                source_state_hash,
                source_checkpoint_hash,
                source_end,
                segment_id,
                target_checkpoint_hash,
                token_hash,
            },
            segment,
        }))
    }

    /// Atomically commits a previously prepared segment after reading the
    /// exact stored bytes back through the archive provider.
    pub fn commit_evidence_seal(
        &mut self,
        token: &EvidenceSealToken,
        provider: &dyn ArchiveProvider,
    ) -> Result<(), CanwuError> {
        if self.committed_seal_tokens.contains(&token.token_hash) {
            return Ok(());
        }
        let expected_token_hash = super::canonical_hash(
            SEAL_TOKEN_DOMAIN,
            &(
                &token.source_state_hash,
                &token.source_checkpoint_hash,
                token.source_end,
                &token.segment_id,
                &token.target_checkpoint_hash,
            ),
        )?;
        if expected_token_hash != token.token_hash {
            return Err(archive_error("evidence seal token hash is invalid"));
        }
        if self.simulation.authoritative_state_hash()? != token.source_state_hash
            || self.simulation.compact_identity_hash()? != token.source_checkpoint_hash
            || self.simulation.evidence_cursor()? != token.source_end
        {
            return Err(CanwuError::new(
                ErrorCode::StaleSealToken,
                "evidence seal token no longer names the live source cut",
            ));
        }
        let stored = provider
            .load_evidence_segment(&token.segment_id)?
            .ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::ArchiveNotReady,
                    "prepared evidence segment is not available from the archive provider",
                )
            })?;
        let archive = stored
            .archive
            .as_ref()
            .ok_or_else(|| archive_error("stored evidence segment has no archive index"))?;
        if archive.header.segment_id != token.segment_id {
            return Err(archive_error(
                "stored evidence segment ID does not match the seal token",
            ));
        }
        verify_archived_segment(&stored)?;

        let before = self.simulation.fork();
        let result = (|| {
            let actual = self.simulation.seal_retained_evidence()?.ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::StaleSealToken,
                    "the prepared evidence tail is no longer retained",
                )
            })?;
            if actual != stored {
                return Err(archive_error(
                    "stored evidence segment differs from the prepared live cut",
                ));
            }
            self.simulation
                .validate_payload_required_archive(provider)?;
            if self.simulation.compact_identity_hash()? != token.target_checkpoint_hash {
                return Err(archive_error(
                    "committed checkpoint hash differs from the seal token",
                ));
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.simulation.restore_from_fork(before);
            return Err(error);
        }
        self.committed_seal_tokens.insert(token.token_hash.clone());
        Ok(())
    }

    #[must_use]
    pub const fn time(&self) -> SimTime {
        self.simulation.time()
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.simulation.revision()
    }

    #[must_use]
    pub fn checkpoint_hash(&self) -> &str {
        self.simulation.checkpoint_hash()
    }

    #[must_use]
    pub fn boundary_head_hash(&self) -> Option<&str> {
        self.simulation.boundary_head_hash()
    }

    pub fn entities(&self) -> impl Iterator<Item = &super::EntityRef> {
        self.simulation.entities()
    }

    #[must_use]
    pub fn entity_exists(&self, entity: &super::EntityRef) -> bool {
        self.simulation.entity_exists(entity)
    }

    /// The retention a registered knowledge schema declares, or `None` for a
    /// schema this run does not register.
    #[must_use]
    pub fn knowledge_retention(
        &self,
        schema: &super::KnowledgeSchemaId,
    ) -> Option<super::KnowledgeRetention> {
        self.simulation.knowledge_retention(schema)
    }

    #[must_use]
    pub fn world(&self) -> super::WorldSnapshot {
        self.simulation.world()
    }

    /// Returns committed availability for a person. `None` means alive and free.
    #[must_use]
    pub fn person_availability(
        &self,
        person: super::PersonId,
    ) -> Option<&super::PersonAvailability> {
        self.simulation.person_availability(person)
    }

    /// Returns every committed person availability in person-ID order.
    pub fn person_availabilities(
        &self,
    ) -> impl Iterator<Item = (&super::PersonId, &super::PersonAvailability)> {
        self.simulation.person_availabilities()
    }

    /// Returns the registered transition manifests whose ready boundary has
    /// not settled, in manifest-ID order.
    pub fn pending_transition_manifests(
        &self,
    ) -> impl Iterator<Item = &super::PendingTransitionManifest> {
        self.simulation.pending_transition_manifests()
    }

    /// Returns the settled and queued boundaries of the half-open window
    /// `from..to`. A seal keeps the settled ledger, so the result does not
    /// depend on how much evidence is retained. The result names every
    /// plugin's queued packet types, so do not expose it to player, agent,
    /// observer, or remote clients.
    pub fn boundaries_between(
        &self,
        from: SimTime,
        to: SimTime,
    ) -> Result<super::BoundaryWindow, CanwuError> {
        self.simulation.boundaries_between(from, to)
    }

    /// How far back the run keeps settled boundaries: the longest horizon a
    /// registered boundary system declares, or zero.
    #[must_use]
    pub fn boundary_schedule_horizon(&self) -> super::SimDuration {
        self.simulation.boundary_schedule_horizon()
    }

    #[must_use]
    pub fn knowledge(&self) -> &KnowledgeSnapshot {
        self.simulation.knowledge()
    }

    #[must_use]
    pub fn domain_record(&self, reference: &DomainRecordRef) -> Option<&DomainRecord> {
        self.simulation.domain_record(reference)
    }

    /// See [`Simulation::replay_stable_domain_record_version`]: the body and
    /// establishment time of a record's committed current version or of an
    /// initial-scenario version, which a seal cannot change.
    #[must_use]
    pub fn replay_stable_domain_record_version(
        &self,
        reference: &DomainRecordVersionRef,
    ) -> Option<(DomainRecord, SimTime)> {
        self.simulation
            .replay_stable_domain_record_version(reference)
    }

    #[must_use]
    pub fn decision_ticket(&self, id: super::DecisionTicketId) -> Option<&super::DecisionTicket> {
        self.simulation.decision_ticket(id)
    }

    #[must_use]
    pub fn decision_controller(&self, id: &str) -> Option<&super::DecisionControllerBinding> {
        self.simulation.decision_controller(id)
    }

    #[must_use]
    pub fn decision_trace(&self, id: super::DecisionTraceId) -> Option<&super::DecisionTrace> {
        self.simulation.decision_trace(id)
    }

    #[must_use]
    pub fn decision_attempt(
        &self,
        id: super::DecisionRequestId,
    ) -> Option<&super::DecisionAttemptRecord> {
        self.simulation.decision_attempt(id)
    }

    #[must_use]
    pub fn decision_hot_state(&self) -> super::DecisionHotState {
        self.simulation.decision_hot_state()
    }

    #[must_use]
    pub fn decision_history_location(
        &self,
        key: &super::DecisionHistoryKey,
    ) -> super::DecisionHistoryLocation {
        self.simulation.decision_history_location(key)
    }

    pub fn decision_history_location_with_provider(
        &self,
        key: &super::DecisionHistoryKey,
        provider: &dyn super::DecisionArchiveProvider,
    ) -> Result<super::DecisionHistoryLocation, CanwuError> {
        self.simulation
            .decision_history_location_with_provider(key, provider)
    }

    #[must_use]
    pub fn typed_domain_record<T: DomainRecordType>(
        &self,
        reference: &TypedDomainRecordRef<T>,
    ) -> Option<&DomainRecord> {
        self.simulation.typed_domain_record(reference)
    }

    pub fn submit(&mut self, envelope: CommandEnvelope) -> Result<CommandReceipt, CanwuError> {
        self.simulation.submit(envelope)
    }

    pub fn process_command(
        &mut self,
        request: CommandRequest,
    ) -> Result<CommandOutcome, CanwuError> {
        self.simulation.process_command(request)
    }

    pub fn enqueue_command(
        &mut self,
        due_at: SimTime,
        priority: i32,
        request: CommandRequest,
    ) -> Result<IngressReceipt, CanwuError> {
        self.simulation.enqueue_command(due_at, priority, request)
    }

    pub fn enqueue_plugin_ingress(
        &mut self,
        request: PluginIngressRequest,
    ) -> Result<IngressReceipt, CanwuError> {
        self.simulation.enqueue_plugin_ingress(request)
    }

    pub fn enqueue_permitted_plugin_ingress(
        &mut self,
        request: PluginIngressRequest,
        permit: &super::PluginIngressPermit,
    ) -> Result<IngressReceipt, CanwuError> {
        self.simulation
            .enqueue_permitted_plugin_ingress(request, permit)
    }

    /// See [`Simulation::cancel_plugin_ingress`].
    pub fn cancel_plugin_ingress(
        &mut self,
        ingress_id: super::IngressId,
        reason: impl Into<String>,
    ) -> Result<IngressReceipt, CanwuError> {
        self.simulation.cancel_plugin_ingress(ingress_id, reason)
    }

    /// See [`Simulation::cancel_permitted_plugin_ingress`].
    pub fn cancel_permitted_plugin_ingress(
        &mut self,
        ingress_id: super::IngressId,
        permit: &super::PluginIngressPermit,
        reason: impl Into<String>,
    ) -> Result<IngressReceipt, CanwuError> {
        self.simulation
            .cancel_permitted_plugin_ingress(ingress_id, permit, reason)
    }

    pub fn prepare_decision(
        &self,
        decision_request_id: super::DecisionRequestId,
        command_request_id: Option<super::CommandRequestId>,
        ticket_id: super::DecisionTicketId,
        policy: &dyn super::DecisionPolicy,
    ) -> Result<super::DecisionEvaluation, CanwuError> {
        self.simulation
            .prepare_decision(decision_request_id, command_request_id, ticket_id, policy)
    }

    pub fn prepare_decision_at(
        &self,
        due_at: super::SimTime,
        decision_request_id: super::DecisionRequestId,
        command_request_id: Option<super::CommandRequestId>,
        ticket_id: super::DecisionTicketId,
        policy: &dyn super::DecisionPolicy,
    ) -> Result<super::DecisionEvaluation, CanwuError> {
        self.simulation.prepare_decision_at(
            due_at,
            decision_request_id,
            command_request_id,
            ticket_id,
            policy,
        )
    }

    pub fn enqueue_decision(
        &mut self,
        due_at: super::SimTime,
        priority: i32,
        request: super::DecisionIngressRequest,
    ) -> Result<IngressReceipt, CanwuError> {
        self.simulation.enqueue_decision(due_at, priority, request)
    }

    pub fn drive_decision(
        &mut self,
        due_at: super::SimTime,
        priority: i32,
        decision_request_id: super::DecisionRequestId,
        command_request_id: Option<super::CommandRequestId>,
        ticket_id: super::DecisionTicketId,
        policy: &dyn super::DecisionPolicy,
    ) -> Result<super::DecisionEvaluation, CanwuError> {
        self.simulation.drive_decision(
            due_at,
            priority,
            decision_request_id,
            command_request_id,
            ticket_id,
            policy,
        )
    }

    pub fn schedule_calendar_boundary(
        &mut self,
        due_at: SimTime,
        cadences: Vec<SystemCadence>,
    ) -> Result<IngressReceipt, CanwuError> {
        self.simulation.schedule_calendar_boundary(due_at, cadences)
    }

    pub fn advance(&mut self, duration: SimDuration) -> Result<Vec<SimEvent>, CanwuError> {
        self.simulation.advance(duration)
    }

    pub fn advance_canonical(
        &mut self,
        duration: SimDuration,
    ) -> Result<Vec<BoundaryReceipt>, CanwuError> {
        self.simulation.advance_canonical(duration)
    }

    pub fn step_canonical(&mut self) -> Result<Option<BoundaryReceipt>, CanwuError> {
        self.simulation.step_canonical()
    }

    pub fn settle_boundary(
        &mut self,
        request: BoundaryRequest,
    ) -> Result<BoundaryReceipt, CanwuError> {
        self.simulation.settle_boundary(request)
    }

    /// See [`Simulation::set_boundary_profiling`]. Sealing evidence neither
    /// changes the setting nor drops profiles that were not taken yet.
    pub fn set_boundary_profiling(&mut self, enabled: bool) {
        self.simulation.set_boundary_profiling(enabled);
    }

    /// See [`Simulation::boundary_profiling`].
    #[must_use]
    pub const fn boundary_profiling(&self) -> bool {
        self.simulation.boundary_profiling()
    }

    /// See [`Simulation::take_boundary_profiles`].
    pub fn take_boundary_profiles(&mut self) -> super::BoundaryProfileBatch {
        self.simulation.take_boundary_profiles()
    }

    /// Reconstructs a validated full snapshot from the supplied sealed prefix
    /// plus the currently retained tail.
    pub fn snapshot_with_segments(
        &self,
        segments: Vec<EvidenceJournalSegment>,
    ) -> Result<SimulationSnapshot, CanwuError> {
        let (snapshot, _) = self.restore_with_segments(segments, true)?;
        snapshot.ok_or_else(|| invalid_snapshot_error("reconstructed snapshot was not kept"))
    }

    /// Produces the ordinary exact-replay journal after validating the supplied archive.
    pub fn replay_journal_with_segments(
        &self,
        segments: Vec<EvidenceJournalSegment>,
    ) -> Result<ReplayJournal, CanwuError> {
        let (_, simulation) = self.restore_with_segments(segments, false)?;
        Ok(simulation.replay_journal())
    }

    /// Rebuilds and validates the full run from the supplied sealed prefix
    /// plus the retained tail, building one runtime, and keeps a copy of the
    /// reconstructed snapshot when asked.
    fn restore_with_segments(
        &self,
        mut segments: Vec<EvidenceJournalSegment>,
        keep_snapshot: bool,
    ) -> Result<(Option<SimulationSnapshot>, Simulation), CanwuError> {
        self.simulation
            .ensure_full_history("a snapshot or replay journal from segments")?;
        let tail = self
            .simulation
            .journal_segment_since(self.simulation.state.evidence.archived)?;
        if tail.start != tail.end {
            segments.push(tail);
        }
        let (snapshot, reconstructed) =
            Simulation::reconstruct_checkpoint_snapshot(self.checkpoint()?, segments)?;
        let kept = keep_snapshot.then(|| snapshot.clone());
        let simulation = Simulation::from_snapshot(snapshot)?;
        reconstructed.verify(&simulation)?;
        Ok((kept, simulation))
    }

    /// Restores and validates a checkpoint plus its archive, then enters the
    /// compact interface with that evidence retained until the caller seals it.
    /// The next seal starts at the zero cut and replaces the supplied segments.
    pub fn from_checkpoint_and_journal(
        checkpoint: SimulationCheckpoint,
        segments: Vec<EvidenceJournalSegment>,
    ) -> Result<Self, CanwuError> {
        Simulation::from_checkpoint_and_journal(checkpoint, segments)?.into_compacted()
    }

    /// Restores a compact runtime and rehydrates its exact executable plugins.
    pub fn from_checkpoint_and_journal_with_plugins(
        checkpoint: SimulationCheckpoint,
        segments: Vec<EvidenceJournalSegment>,
        plugins: &[&dyn SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        let mut simulation = Simulation::from_checkpoint_and_journal(checkpoint, segments)?;
        for plugin in plugins {
            simulation.register_plugin(*plugin)?;
        }
        simulation.ensure_runtime_ready()?;
        simulation.into_compacted()
    }

    /// Restores a paged checkpoint plus its contiguous evidence prefix, then
    /// enters the compact interface with that evidence retained until the
    /// caller seals it. The next seal starts at the zero cut and replaces the
    /// supplied segments.
    pub fn from_paged_checkpoint_and_journal(
        checkpoint: &PagedSimulationCheckpoint,
        provider: &dyn StatePageProvider,
        segments: Vec<EvidenceJournalSegment>,
    ) -> Result<Self, CanwuError> {
        Simulation::from_paged_checkpoint_and_journal(checkpoint, provider, segments)?
            .into_compacted()
    }

    /// Restores a paged compact runtime and rehydrates its exact executable
    /// plugins.
    pub fn from_paged_checkpoint_and_journal_with_plugins(
        checkpoint: &PagedSimulationCheckpoint,
        provider: &dyn StatePageProvider,
        segments: Vec<EvidenceJournalSegment>,
        plugins: &[&dyn SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        Simulation::from_paged_checkpoint_and_journal_with_plugins(
            checkpoint, provider, segments, plugins,
        )?
        .into_compacted()
    }
}

impl Simulation {
    /// Converts this runtime into the opt-in compact journal interface.
    ///
    /// Conversion itself preserves the complete retained history. Call
    /// [`CompactedSimulation::seal_evidence`] to release a validated segment
    /// explicitly.
    pub fn into_compacted(self) -> Result<CompactedSimulation, CanwuError> {
        self.ensure_runtime_ready()?;
        Ok(CompactedSimulation {
            simulation: self,
            committed_seal_tokens: im::OrdSet::new(),
        })
    }

    pub(super) fn evidence_dependencies(&self) -> Result<Vec<EvidenceDependency>, CanwuError> {
        let mut dependencies = BTreeMap::new();
        let identity = EvidenceRequirement::IdentityOnly;

        for body in self.state.current.knowledge.records.bodies() {
            for reference in &body.origin.evidence {
                promote_dependency(&mut dependencies, reference.clone(), identity);
            }
        }

        for record in self.state.current.domain_records.values() {
            let schema = self
                .plugins
                .record_schemas
                .get(&record.reference.kind)
                .map(|(_, schema)| schema)
                .ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::ArchiveNotReady,
                        format!(
                            "current domain record {} has no registered schema",
                            record.reference
                        ),
                    )
                })?;
            add_identity_evidence_dependencies(&mut dependencies, record, schema)?;
            add_payload_required_continuation_dependencies(&mut dependencies, record, schema)?;
            let version = self
                .state
                .metadata
                .current_domain_record_versions
                .get(&record.reference)
                .map(|current| &current.version)
                .filter(|version| version.version == record.version)
                .ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::ArchiveNotReady,
                        format!(
                            "current domain record {} has no exact version provenance",
                            record.reference
                        ),
                    )
                })?;
            if !matches!(
                version.established_by,
                DomainRecordVersionSource::InitialScenario
            ) {
                promote_dependency(
                    &mut dependencies,
                    EvidenceRef::DomainRecordVersion(version.clone()),
                    identity,
                );
            } else if !self.bound_initial_scenario().is_some_and(|scenario| {
                scenario.domain_records.iter().any(|initial| {
                    initial.reference == record.reference && initial.version == record.version
                })
            }) {
                return Err(CanwuError::new(
                    ErrorCode::ArchiveNotReady,
                    format!(
                        "current domain record {} has invalid initial-scenario provenance",
                        record.reference
                    ),
                ));
            }
        }

        // Keyed draws are not dependencies: the keyed-draw ledger answers an
        // exact retry without reading evidence, and ordinal identities stay
        // provable from the journal counters after a seal.

        for archived in self.state.evidence.archived_command_requests.values() {
            add_command_outcome_dependencies(&mut dependencies, &archived.outcome);
        }
        for archived in self.state.evidence.archived_ingress_requests.values() {
            promote_dependency(
                &mut dependencies,
                EvidenceRef::Ingress(archived.receipt.ingress_id),
                identity,
            );
        }
        for archived in self.state.evidence.archived_decision_requests.values() {
            promote_dependency(
                &mut dependencies,
                EvidenceRef::Ingress(archived.receipt.ingress_id),
                identity,
            );
        }
        for attempt in &self.state.evidence.command_attempts {
            if attempt.request_id.is_none() {
                continue;
            }
            promote_dependency(
                &mut dependencies,
                EvidenceRef::CommandAttempt(attempt.id),
                identity,
            );
            if let CommandAttemptOutcome::Accepted { command_id } = attempt.outcome {
                promote_dependency(
                    &mut dependencies,
                    EvidenceRef::Command(command_id),
                    identity,
                );
                if let Some(command) = self
                    .state
                    .evidence
                    .commands
                    .iter()
                    .find(|command| command.id == command_id)
                {
                    for event in &command.emitted_events {
                        promote_dependency(&mut dependencies, EvidenceRef::Event(*event), identity);
                    }
                }
            }
        }
        for ingress in &self.state.evidence.ingress {
            if matches!(
                ingress.payload,
                IngressPayload::Command { .. } | IngressPayload::Decision { .. }
            ) {
                promote_dependency(
                    &mut dependencies,
                    EvidenceRef::Ingress(ingress.id),
                    identity,
                );
            }
        }

        for action in self.state.scheduler.actions.values() {
            match action {
                ScheduledAction::ArmyArrival { order_event, .. }
                | ScheduledAction::PersonArrival { order_event, .. } => promote_dependency(
                    &mut dependencies,
                    EvidenceRef::Event(*order_event),
                    identity,
                ),
                ScheduledAction::KnowledgeReport { dispatch_event, .. } => promote_dependency(
                    &mut dependencies,
                    EvidenceRef::Event(*dispatch_event),
                    identity,
                ),
                ScheduledAction::PluginDirective { cause, .. } => {
                    add_cause_dependency(&mut dependencies, cause);
                }
            }
        }

        Ok(dependencies
            .into_iter()
            .map(|(reference, requirement)| EvidenceDependency {
                reference,
                requirement,
            })
            .collect())
    }

    /// Whether the runtime holds the payload of a piece of evidence: a
    /// retained or pinned record, or a record version body it can read.
    pub(super) fn payload_is_held(&self, reference: &EvidenceRef) -> bool {
        let evidence = &self.state.evidence;
        match reference {
            EvidenceRef::Event(id) => evidence.event_record(*id).is_some(),
            EvidenceRef::Command(id) => evidence.command_record(*id).is_some(),
            EvidenceRef::CommandAttempt(id) => evidence.command_attempt_record(*id).is_some(),
            EvidenceRef::Ingress(id) => evidence.ingress_record(*id).is_some(),
            EvidenceRef::Boundary(id) => evidence.boundary_record(*id).is_some(),
            EvidenceRef::RandomDraw(id) => evidence.random_draw_record(*id).is_some(),
            EvidenceRef::DomainRecordVersion(version) => {
                super::retained_domain_record_version(&self.state, version).is_some()
            }
        }
    }

    /// Checks that every payload-required dependency is held, as the
    /// continuation closure keeps it, or else receipted and loadable from the
    /// provider.
    fn validate_payload_required_archive(
        &self,
        provider: &dyn ArchiveProvider,
    ) -> Result<(), CanwuError> {
        for dependency in self
            .evidence_dependencies()?
            .into_iter()
            .filter(|dependency| dependency.requirement == EvidenceRequirement::PayloadRequired)
        {
            if self.payload_is_held(&dependency.reference) {
                continue;
            }
            let receipt = self
                .state
                .evidence
                .archived_evidence_receipts
                .get(&dependency.reference)
                .ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::ArchiveNotReady,
                        "payload-required evidence has no committed archive receipt",
                    )
                })?;
            load_verified_archived_evidence_segment(receipt, provider)?;
        }
        Ok(())
    }

    /// The cut a seal moves each journal to: events, commands, and attempts up
    /// to the admission cursors (a later boundary has admitted them), and
    /// ingress, boundaries, and draws to their end. Ingress and boundaries
    /// share the cut, so a boundary-generated packet and its producing
    /// boundary always land in the same segment.
    pub(super) fn settled_cut(&self) -> Result<EvidenceCursor, CanwuError> {
        let end = self.evidence_cursor()?;
        let archived = &self.state.evidence.archived;
        let counters = &self.state.counters;
        let within = |cursor: u64, archived: u64, end: u64| archived <= cursor && cursor <= end;
        if !within(
            counters.admitted_event_count,
            archived.event_count,
            end.event_count,
        ) || !within(
            counters.admitted_command_count,
            archived.command_count,
            end.command_count,
        ) || !within(
            counters.admitted_attempt_count,
            archived.command_attempt_count,
            end.command_attempt_count,
        ) {
            return Err(invalid_snapshot_error(
                "admission cursors lie outside the retained journals",
            ));
        }
        Ok(EvidenceCursor {
            event_count: counters.admitted_event_count,
            command_count: counters.admitted_command_count,
            command_attempt_count: counters.admitted_attempt_count,
            ..end
        })
    }

    /// Builds the exact idempotency tables for tracked requests whose
    /// attempts and ingress lie in the given slices, as a seal records them.
    pub(super) fn archived_request_tables(
        &self,
        attempts: &[CommandAttemptRecord],
        ingress: &[IngressRecord],
    ) -> Result<ArchivedRequestTables, CanwuError> {
        let mut tables = ArchivedRequestTables::default();
        for attempt in attempts {
            let Some(request_id) = attempt.request_id else {
                continue;
            };
            let outcome = self.command_outcome_from_attempt(attempt)?;
            tables.commands.push((
                request_id,
                ArchivedCommandRequestOutcome {
                    input_hash: super::canonical_hash(
                        "canwu.archive.command.request.v1",
                        &(attempt.expected_revision, &attempt.envelope),
                    )?,
                    outcome,
                },
            ));
        }
        for record in ingress {
            let receipt = IngressReceipt {
                ingress_id: record.id,
                issued_at: record.issued_at,
                due_at: record.due_at,
            };
            match &record.payload {
                IngressPayload::Command { request } => {
                    tables.ingress.push((
                        request.request_id,
                        ArchivedIngressRequest {
                            input_hash: super::canonical_hash(
                                "canwu.archive.ingress.command.v1",
                                &(record.due_at, record.priority, request.as_ref()),
                            )?,
                            receipt,
                        },
                    ));
                }
                IngressPayload::Decision { request } => {
                    if let Some(command) = &request.command {
                        tables.decision_commands.push(command.request_id);
                    }
                    tables.decisions.push((
                        request.request_id,
                        ArchivedIngressRequest {
                            input_hash: super::canonical_hash(
                                "canwu.ingress.decision-request.v1",
                                &(record.due_at, record.priority, request.as_ref()),
                            )?,
                            receipt,
                        },
                    ));
                }
                IngressPayload::Plugin { .. }
                | IngressPayload::Calendar { .. }
                | IngressPayload::Maintenance { .. }
                | IngressPayload::PluginCancellation { .. } => {}
            }
        }
        Ok(tables)
    }

    /// Whether a command request was queued as command or decision ingress
    /// that a seal has already archived.
    pub(super) fn archived_request_was_queued(&self, request: super::CommandRequestId) -> bool {
        self.state.evidence.archived_request_was_queued(request)
    }

    /// The history flags of the given slices, as a seal records them. A
    /// request queued as ingress counts as queued whether its ingress is in
    /// the slices or was archived by an earlier seal, because a pending
    /// command ingress can be sealed before the boundary that admits it.
    pub(super) fn history_flags_of(
        &self,
        commands: &[CommandRecord],
        attempts: &[CommandAttemptRecord],
        ingress: &[IngressRecord],
    ) -> (bool, bool, bool) {
        (
            commands.iter().any(|record| record.attempt_id.is_none()),
            !attempts.is_empty() || !ingress.is_empty(),
            has_unqueued_command_history(commands, attempts, ingress, &|request| {
                self.archived_request_was_queued(request)
            }),
        )
    }

    /// Seals the settled part of every journal and keeps the continuation
    /// closure below the new cut as pinned evidence.
    ///
    /// Pending ingress, the unadmitted tail, and work in flight do not block
    /// a seal: the unadmitted events, commands, and attempts stay retained,
    /// and the closure keeps what pending work and declarations still read.
    /// A dependency whose evidence the run can no longer read (declared after
    /// its evidence was sealed) fails with `ArchiveNotReady`.
    fn seal_retained_evidence(&mut self) -> Result<Option<EvidenceJournalSegment>, CanwuError> {
        let start = self.state.evidence.archived;
        let cut = self.settled_cut()?;
        if start == cut {
            return Ok(None);
        }
        let dependencies = self.evidence_dependencies()?;
        let needs = self.continuation_needs(&dependencies)?;
        let pins = self.closure_pins(&needs, &cut)?;
        let sealed = |count: u64, archived: u64| {
            usize::try_from(count - archived)
                .map_err(|_| invalid_snapshot_error("sealed journal slice exceeds platform range"))
        };
        let event_count = sealed(cut.event_count, start.event_count)?;
        let command_count = sealed(cut.command_count, start.command_count)?;
        let attempt_count = sealed(cut.command_attempt_count, start.command_attempt_count)?;
        let evidence = &self.state.evidence;
        let mut segment = EvidenceJournalSegment {
            format_version: CHECKPOINT_JOURNAL_FORMAT_VERSION,
            start,
            end: cut,
            events: evidence.events[..event_count].to_vec(),
            commands: evidence.commands[..command_count].to_vec(),
            command_attempts: evidence.command_attempts[..attempt_count].to_vec(),
            ingress: evidence.ingress.to_vec(),
            boundaries: evidence.boundaries.clone(),
            random_draws: evidence.random_draws.clone(),
            archive: None,
        };
        let checkpoint_hash = self.state.metadata.checkpoint_hash.clone();
        let commitment_roots = self.state.metadata.commitment_roots.clone();
        let commitment_cache = self.state.metadata.commitment_cache.clone();
        let tables = self.refresh_checkpoint_hash().and_then(|()| {
            self.archived_request_tables(&segment.command_attempts, &segment.ingress)
        });
        let tables = match tables {
            Ok(tables) => tables,
            Err(error) => {
                self.state.metadata.checkpoint_hash = checkpoint_hash;
                self.state.metadata.commitment_roots = commitment_roots;
                self.state.metadata.commitment_cache = commitment_cache;
                return Err(error);
            }
        };
        let (archive, receipts) = evidence_archive_index(&segment)?;
        segment.archive = Some(archive.clone());
        verify_archived_segment(&segment)?;
        let (legacy, tracked, unqueued) = self.history_flags_of(
            &segment.commands,
            &segment.command_attempts,
            &segment.ingress,
        );

        let evidence = &mut self.state.evidence;
        evidence.archived_boundary_head = segment
            .boundaries
            .last()
            .map(|record| record.hash.clone())
            .or_else(|| evidence.archived_boundary_head.clone());
        evidence.archived_legacy_commands |= legacy;
        evidence.archived_tracked_attempts |= tracked;
        evidence.archived_unqueued_command_history |= unqueued;
        evidence.archived_command_requests.extend(tables.commands);
        evidence.archived_ingress_requests.extend(tables.ingress);
        evidence.archived_decision_requests.extend(tables.decisions);
        evidence
            .archived_decision_command_requests
            .extend(tables.decision_commands);
        let mut sealed_times = evidence.sealed_times.clone();
        sealed_times.extend_from_segment(&segment);
        evidence.sealed_times = sealed_times;
        evidence.archived = cut;
        evidence.events.drain(..event_count);
        evidence.seal_command_history(command_count, attempt_count);
        evidence.boundaries.clear();
        evidence.random_draws.clear();
        self.state.scheduler.cancelled_ingress.clear();
        let evidence = &mut self.state.evidence;
        evidence.archived_segment_headers.push_back(archive.header);
        for receipt in receipts {
            if evidence
                .archived_evidence_receipts
                .insert(receipt.evidence.clone(), Arc::new(receipt))
                .is_some()
            {
                return Err(archive_error("archived evidence receipt was duplicated"));
            }
        }
        retain_reachable_archived_evidence_receipts(
            &mut evidence.archived_evidence_receipts,
            &dependencies,
        );
        evidence.pinned = pins;
        Ok(Some(segment))
    }

    pub(super) fn checkpoint_state(&self) -> SimulationSnapshot {
        self.checkpoint_state_with_paged_payloads(true)
    }

    fn checkpoint_state_with_paged_payloads(
        &self,
        include_paged_payloads: bool,
    ) -> SimulationSnapshot {
        SimulationSnapshot {
            engine_version: ENGINE_VERSION.to_owned(),
            snapshot_format_version: SNAPSHOT_FORMAT_VERSION,
            run_manifest: Some(self.state.metadata.run_manifest.clone()),
            run_manifest_hash: self.state.metadata.run_manifest_hash.clone(),
            run_configuration: Some(self.state.metadata.run_configuration.clone()),
            checkpoint_hash: self.state.metadata.checkpoint_hash.clone(),
            commitment_format_version: self.state.metadata.commitment_format_version,
            commitment_roots: self.state.metadata.commitment_roots.clone(),
            revision_format_version: STATE_REVISION_FORMAT_VERSION,
            state_revision: self.state.counters.state_revision,
            replay_revision_format_version: self.state.metadata.replay_revision_format_version,
            admission_cursor_format_version: ADMISSION_CURSOR_FORMAT_VERSION,
            admitted_attempt_count: self.state.counters.admitted_attempt_count,
            admitted_command_count: self.state.counters.admitted_command_count,
            admitted_event_count: self.state.counters.admitted_event_count,
            initial_time: self.state.scheduler.initial_time,
            initial_scenario: self.bound_initial_scenario().cloned(),
            initial_record_index: InitialRecordIndexCache::seeded(Arc::clone(
                &self.state.metadata.initial_domain_record_indexes,
            )),
            now: self.state.scheduler.now,
            plugin_registration_closed: self.state.metadata.plugin_registration_closed,
            entities: self.state.current.entities.iter().cloned().collect(),
            world: self.world(),
            person_availability: self.state.current.person_availability.clone(),
            created_persons: self.state.current.created_persons.clone(),
            knowledge: KnowledgeSnapshot::clone(&self.state.current.knowledge),
            events: Vec::new(),
            commands: Vec::new(),
            command_attempts: Vec::new(),
            ingress: Vec::new(),
            boundaries: Vec::new(),
            plugin_components: self
                .state
                .current
                .plugin_components
                .values()
                .cloned()
                .collect(),
            domain_records: if include_paged_payloads {
                self.state
                    .current
                    .domain_records
                    .values()
                    .cloned()
                    .collect()
            } else {
                Vec::new()
            },
            decisions: if include_paged_payloads {
                self.state.current.decisions.clone()
            } else {
                DecisionState::default()
            },
            plugin_descriptors: self.plugins.descriptors().cloned().collect(),
            schema: self.schema.clone(),
            root_seed: self.state.current.root_seed,
            authority_root_seed: self.state.current.authority_root_seed,
            random_streams: self
                .state
                .current
                .random_streams
                .values()
                .cloned()
                .collect(),
            random_draws: Vec::new(),
            scheduled: self
                .state
                .scheduler
                .actions
                .iter()
                .map(|(key, action)| ScheduledRecord {
                    key: key.clone(),
                    action: action.clone(),
                })
                .collect(),
            pending_transition_manifests: self
                .state
                .scheduler
                .transition_manifests
                .values()
                .cloned()
                .collect(),
            legacy_rng: None,
            next_event_id: self.state.counters.next_event_id,
            next_command_id: self.state.counters.next_command_id,
            next_command_attempt_id: self.state.counters.next_command_attempt_id,
            next_ingress_id: self.state.counters.next_ingress_id,
            next_boundary_id: self.state.counters.next_boundary_id,
            next_random_draw_id: self.state.counters.next_random_draw_id,
            next_knowledge_record_id: self.state.counters.next_knowledge_record_id,
            next_schedule_sequence: self.state.counters.next_schedule_sequence,
            next_correlation_id: self.state.counters.next_correlation_id,
            next_decision_trace_id: self.state.counters.next_decision_trace_id,
            next_person_id: self.state.counters.next_person_id,
        }
    }

    fn build_paged_checkpoint_pages(
        &self,
        provider: Option<&dyn StatePageProvider>,
    ) -> Result<(PagedSimulationCheckpoint, Vec<StatePageBlob>), CanwuError> {
        let (domain_records, domain_pages) = match provider {
            Some(provider) => self
                .state
                .current
                .domain_records
                .missing_state_pages(provider)?,
            None => self.state.current.domain_records.state_pages()?,
        };
        let decisions = &self.state.current.decisions;
        let hot_decisions = decisions.paged_checkpoint_hot_state();
        let hot_decision_page =
            StatePageBlob::new(serde_json::to_vec(&hot_decisions).map_err(|error| {
                invalid_snapshot_error(format!("cannot encode paged hot decision state: {error}"))
            })?)?;
        let decision_bucket_pages = decisions
            .decision_archive_bucket_page_ids()
            .iter()
            .map(|(bucket, page_id)| (*bucket, page_id.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut decision_pages = Vec::with_capacity(
            decision_bucket_pages
                .len()
                .saturating_add(
                    decision_bucket_pages
                        .len()
                        .div_ceil(MAX_PAGED_DECISION_DIRECTORY_ENTRIES),
                )
                .saturating_add(2),
        );
        for (bucket, expected_page_id) in &decision_bucket_pages {
            let Some(bucket_page) = decisions
                .decision_archive_bucket_page(*bucket)
                .map_err(|error| invalid_snapshot_error(error.to_string()))?
            else {
                if provider.is_none() {
                    return Err(invalid_snapshot_error(
                        "portable decision checkpoint requires every archive bucket to be resident",
                    ));
                }
                // A root-only restored state already authenticates these page IDs through
                // its archive-receipt root. Reusing the directory commitment avoids one
                // provider read per unchanged locator page; state-delta verification later
                // authenticates the transitive page closure before it becomes durable.
                continue;
            };
            let page = StatePageBlob::new(serde_json::to_vec(&bucket_page).map_err(|error| {
                invalid_snapshot_error(format!(
                    "cannot encode paged decision archive bucket: {error}"
                ))
            })?)?;
            if page.page_id != *expected_page_id {
                return Err(invalid_snapshot_error(
                    "decision archive bucket page disagrees with its cached commitment",
                ));
            }
            decision_pages.push(page);
        }
        let decision_bucket_directory = decision_bucket_pages.into_iter().collect::<Vec<_>>();
        let mut archive_directory_page_ids = Vec::with_capacity(
            decision_bucket_directory
                .len()
                .div_ceil(MAX_PAGED_DECISION_DIRECTORY_ENTRIES),
        );
        for chunk in decision_bucket_directory.chunks(MAX_PAGED_DECISION_DIRECTORY_ENTRIES) {
            let directory_page = PagedDecisionDirectoryPage {
                format_version: PAGED_CHECKPOINT_FORMAT_VERSION,
                archive_bucket_pages: chunk.to_vec(),
            };
            directory_page.validate()?;
            let page =
                StatePageBlob::new(serde_json::to_vec(&directory_page).map_err(|error| {
                    invalid_snapshot_error(format!(
                        "cannot encode paged decision directory page: {error}"
                    ))
                })?)?;
            archive_directory_page_ids.push(page.page_id.clone());
            decision_pages.push(page);
        }
        let archive_receipt_count = u64::try_from(decisions.archived_history_count())
            .map_err(|_| invalid_snapshot_error("decision archive receipt count exceeds u64"))?;
        let decision_manifest_page = StatePageBlob::new(
            serde_json::to_vec(&PagedDecisionManifest {
                format_version: PAGED_CHECKPOINT_FORMAT_VERSION,
                hot_page_id: hot_decision_page.page_id.clone(),
                archive_receipt_root: self
                    .state
                    .current
                    .decisions
                    .archive_receipt_commitment()
                    .map_err(|error| invalid_snapshot_error(error.to_string()))?,
                archive_receipt_count,
                archive_directory_page_ids,
            })
            .map_err(|error| {
                invalid_snapshot_error(format!("cannot encode paged decision manifest: {error}"))
            })?,
        )?;
        let checkpoint_without_paged_state = self.checkpoint_with_paged_payloads(false)?;
        let envelope = PagedCheckpointEnvelope {
            format_version: PAGED_CHECKPOINT_FORMAT_VERSION,
            checkpoint_without_paged_state,
            domain_records,
            decision_manifest_page_id: decision_manifest_page.page_id.clone(),
        };
        let envelope_page =
            StatePageBlob::new(serde_json::to_vec(&envelope).map_err(|error| {
                invalid_snapshot_error(format!("cannot encode paged checkpoint envelope: {error}"))
            })?)?;
        let checkpoint = PagedSimulationCheckpoint {
            format_version: PAGED_CHECKPOINT_FORMAT_VERSION,
            root_page_id: envelope_page.page_id.clone(),
            checkpoint_hash: envelope
                .checkpoint_without_paged_state
                .state
                .checkpoint_hash
                .clone(),
        };
        let mut pages = BTreeMap::new();
        for page in domain_pages.into_iter().chain(decision_pages).chain([
            hot_decision_page,
            decision_manifest_page,
            envelope_page,
        ]) {
            if let Some(existing) = pages.insert(page.page_id.clone(), page.clone())
                && existing != page
            {
                return Err(invalid_snapshot_error(
                    "one state-page ID resolved to conflicting canonical bytes",
                ));
            }
        }
        Ok((checkpoint, pages.into_values().collect()))
    }

    /// Prepares an incremental content-addressed checkpoint without changing
    /// authoritative simulation state. Pages already readable from the
    /// provider are omitted from the delta.
    pub fn prepare_paged_checkpoint(
        &self,
        source: Option<&PagedSimulationCheckpoint>,
        provider: &dyn StatePageProvider,
    ) -> Result<PreparedPagedSimulationCheckpoint, CanwuError> {
        self.ensure_full_history("a paged checkpoint")?;
        if let Some(source) = source {
            if source.format_version != PAGED_CHECKPOINT_FORMAT_VERSION {
                return Err(invalid_snapshot_error(
                    "paged checkpoint source uses an unsupported format",
                ));
            }
            let page = provider
                .load_state_page(&source.root_page_id)?
                .ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::StatePageUnavailable,
                        "paged checkpoint source root is unavailable",
                    )
                })?;
            page.validate()?;
            if page.page_id != source.root_page_id {
                return Err(invalid_snapshot_error(
                    "paged checkpoint source provider returned the wrong root",
                ));
            }
        }
        let (checkpoint, pages) = self.build_paged_checkpoint_pages(Some(provider))?;
        let mut new_pages = Vec::new();
        for page in pages {
            match provider.load_state_page(&page.page_id)? {
                Some(existing) => {
                    existing.validate()?;
                    if existing != page {
                        return Err(invalid_snapshot_error(
                            "state-page provider contains conflicting canonical bytes",
                        ));
                    }
                }
                None => new_pages.push(page),
            }
        }
        let source_root = source.map_or_else(
            || canonical_byte_hash("canwu.paged-checkpoint.genesis.v1", &[]),
            |source| source.root_page_id.clone(),
        );
        let delta = prepare_state_delta(&source_root, &checkpoint.root_page_id, new_pages)?;
        Ok(PreparedPagedSimulationCheckpoint { checkpoint, delta })
    }

    /// Builds a self-contained paged checkpoint suitable for transfer between
    /// hosts without an external page provider.
    pub fn portable_paged_checkpoint(
        &self,
    ) -> Result<PortablePagedSimulationCheckpoint, CanwuError> {
        let (checkpoint, pages) = self.build_paged_checkpoint_pages(None)?;
        Ok(PortablePagedSimulationCheckpoint { checkpoint, pages })
    }

    /// Restores a simulation from a verified paged checkpoint. Current domain
    /// records are reconstructed from the committed Patricia roots; missing
    /// pages fail closed instead of being interpreted as absent state.
    pub fn from_paged_checkpoint(
        checkpoint: &PagedSimulationCheckpoint,
        provider: &dyn StatePageProvider,
    ) -> Result<Self, CanwuError> {
        Self::from_paged_checkpoint_and_journal(checkpoint, provider, Vec::new())
    }

    /// Restores a paged current-state checkpoint after proving the contiguous
    /// evidence prefix named by its compact checkpoint metadata.
    pub fn from_paged_checkpoint_and_journal(
        checkpoint: &PagedSimulationCheckpoint,
        provider: &dyn StatePageProvider,
        segments: Vec<EvidenceJournalSegment>,
    ) -> Result<Self, CanwuError> {
        if checkpoint.format_version != PAGED_CHECKPOINT_FORMAT_VERSION {
            return Err(invalid_snapshot_error(
                "paged checkpoint uses an unsupported format",
            ));
        }
        let root_page = provider
            .load_state_page(&checkpoint.root_page_id)?
            .ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::StatePageUnavailable,
                    "paged checkpoint root is unavailable",
                )
            })?;
        root_page.validate()?;
        if root_page.page_id != checkpoint.root_page_id {
            return Err(invalid_snapshot_error(
                "paged checkpoint provider returned the wrong root page",
            ));
        }
        let envelope: PagedCheckpointEnvelope =
            serde_json::from_slice(&root_page.bytes).map_err(|error| {
                invalid_snapshot_error(format!("invalid paged checkpoint envelope: {error}"))
            })?;
        if envelope.format_version != PAGED_CHECKPOINT_FORMAT_VERSION
            || envelope
                .checkpoint_without_paged_state
                .state
                .checkpoint_hash
                != checkpoint.checkpoint_hash
            || !envelope
                .checkpoint_without_paged_state
                .state
                .domain_records
                .is_empty()
            || !envelope
                .checkpoint_without_paged_state
                .state
                .decisions
                .is_empty()
        {
            return Err(invalid_snapshot_error(
                "paged checkpoint envelope metadata is inconsistent",
            ));
        }
        let decision_manifest_page = provider
            .load_state_page(&envelope.decision_manifest_page_id)?
            .ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::StatePageUnavailable,
                    "paged decision manifest is unavailable",
                )
            })?;
        decision_manifest_page.validate()?;
        if decision_manifest_page.page_id != envelope.decision_manifest_page_id {
            return Err(invalid_snapshot_error(
                "paged decision provider returned the wrong manifest page",
            ));
        }
        let decision_manifest: PagedDecisionManifest =
            serde_json::from_slice(&decision_manifest_page.bytes).map_err(|error| {
                invalid_snapshot_error(format!("invalid paged decision manifest: {error}"))
            })?;
        validate_paged_decision_manifest(&decision_manifest)?;
        let hot_decision_page = provider
            .load_state_page(&decision_manifest.hot_page_id)?
            .ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::StatePageUnavailable,
                    "paged hot decision state is unavailable",
                )
            })?;
        hot_decision_page.validate()?;
        if hot_decision_page.page_id != decision_manifest.hot_page_id {
            return Err(invalid_snapshot_error(
                "paged decision provider returned the wrong hot-state page",
            ));
        }
        let hot_decisions: DecisionState = serde_json::from_slice(&hot_decision_page.bytes)
            .map_err(|error| {
                invalid_snapshot_error(format!("invalid paged hot decision state: {error}"))
            })?;
        let mut directory_pages =
            Vec::with_capacity(decision_manifest.archive_directory_page_ids.len());
        for directory_page_id in &decision_manifest.archive_directory_page_ids {
            let directory_state_page =
                provider
                    .load_state_page(directory_page_id)?
                    .ok_or_else(|| {
                        CanwuError::new(
                            ErrorCode::StatePageUnavailable,
                            "paged decision directory page is unavailable",
                        )
                    })?;
            directory_state_page.validate()?;
            if directory_state_page.page_id != *directory_page_id {
                return Err(invalid_snapshot_error(
                    "paged decision provider returned the wrong directory page",
                ));
            }
            let directory_page: PagedDecisionDirectoryPage =
                serde_json::from_slice(&directory_state_page.bytes).map_err(|error| {
                    invalid_snapshot_error(format!(
                        "invalid paged decision directory page: {error}"
                    ))
                })?;
            directory_pages.push(directory_page);
        }
        let archive_bucket_pages = assemble_paged_decision_directory(directory_pages)?;
        let required_dependency_pages = hot_decisions
            .required_archived_dependency_page_keys()
            .map_err(|error| invalid_snapshot_error(error.to_string()))?;
        let mut resident_dependency_pages = Vec::with_capacity(required_dependency_pages.len());
        for page_key in required_dependency_pages {
            let page_id = archive_bucket_pages.get(&page_key).ok_or_else(|| {
                invalid_snapshot_error(
                    "hot decision state references history absent from the archive directory",
                )
            })?;
            let state_page = provider.load_state_page(page_id)?.ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::StatePageUnavailable,
                    "decision dependency locator page is unavailable",
                )
            })?;
            state_page.validate()?;
            if state_page.page_id != *page_id {
                return Err(invalid_snapshot_error(
                    "paged decision provider returned the wrong dependency locator page",
                ));
            }
            let page: DecisionArchiveBucketPage = serde_json::from_slice(&state_page.bytes)
                .map_err(|error| {
                    invalid_snapshot_error(format!(
                        "invalid decision dependency locator page: {error}"
                    ))
                })?;
            page.validate()
                .map_err(|error| invalid_snapshot_error(error.to_string()))?;
            if page.bucket != page_key.bucket
                || page.segment != page_key.segment
                || page
                    .state_page_id()
                    .map_err(|error| invalid_snapshot_error(error.to_string()))?
                    != *page_id
            {
                return Err(invalid_snapshot_error(
                    "decision dependency locator page disagrees with the committed directory",
                ));
            }
            resident_dependency_pages.push(page);
        }
        let decisions = DecisionState::from_paged_checkpoint_root_with_resident_pages(
            hot_decisions,
            archive_bucket_pages.into(),
            decision_manifest.archive_receipt_count,
            &decision_manifest.archive_receipt_root,
            resident_dependency_pages,
        )
        .map_err(|error| invalid_snapshot_error(error.to_string()))?;
        let domain_records = super::PersistentDomainRecordStore::from_state_pages(
            &envelope.domain_records,
            provider,
        )?;
        let mut compact_checkpoint = envelope.checkpoint_without_paged_state;
        compact_checkpoint.state.domain_records = domain_records.values().cloned().collect();
        compact_checkpoint.state.decisions = decisions;
        let mut simulation = Self::from_checkpoint_and_journal(compact_checkpoint, segments)?;
        simulation.state.current.domain_records = domain_records;
        Ok(simulation)
    }

    /// Restores a paged checkpoint and its contiguous evidence prefix, then
    /// rehydrates its exact executable plugin contracts.
    pub fn from_paged_checkpoint_and_journal_with_plugins(
        checkpoint: &PagedSimulationCheckpoint,
        provider: &dyn StatePageProvider,
        segments: Vec<EvidenceJournalSegment>,
        plugins: &[&dyn SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        let mut simulation =
            Self::from_paged_checkpoint_and_journal(checkpoint, provider, segments)?;
        for plugin in plugins {
            simulation.register_plugin(*plugin)?;
        }
        simulation.ensure_runtime_ready()?;
        Ok(simulation)
    }

    pub fn from_portable_paged_checkpoint(
        portable: PortablePagedSimulationCheckpoint,
    ) -> Result<Self, CanwuError> {
        let mut provider = EmbeddedPageProvider::default();
        for page in portable.pages {
            page.validate()?;
            if let Some(existing) = provider.pages.insert(page.page_id.clone(), page.clone())
                && existing != page
            {
                return Err(invalid_snapshot_error(
                    "portable paged checkpoint contains conflicting duplicate pages",
                ));
            }
        }
        Self::from_paged_checkpoint(&portable.checkpoint, &provider)
    }

    /// Returns the current monotonic cut through every append-only journal.
    pub fn evidence_cursor(&self) -> Result<EvidenceCursor, CanwuError> {
        EvidenceCursor::from_evidence(&self.state.evidence)
    }

    fn checkpoint_with_paged_payloads(
        &self,
        include_paged_payloads: bool,
    ) -> Result<SimulationCheckpoint, CanwuError> {
        self.ensure_full_history("a compact checkpoint")?;
        let archived_segment_headers: Vec<_> = self
            .state
            .evidence
            .archived_segment_headers
            .iter()
            .cloned()
            .collect();
        let evidence_dependencies = self.evidence_dependencies()?;
        let required_receipts = reachable_archived_receipt_references(&evidence_dependencies);
        let archived_evidence_receipts: Vec<_> = self
            .state
            .evidence
            .archived_evidence_receipts
            .iter()
            .filter(|(reference, _)| required_receipts.contains(*reference))
            .map(|(_, receipt)| receipt.as_ref().clone())
            .collect();
        let checkpoint = SimulationCheckpoint {
            format_version: CHECKPOINT_JOURNAL_FORMAT_VERSION,
            journal_end: self.evidence_cursor()?,
            state: self.checkpoint_state_with_paged_payloads(include_paged_payloads),
            archived_segment_manifest_root: skipped_commitment_root(
                ARCHIVED_SEGMENT_MANIFEST_DOMAIN,
                &archived_segment_headers,
            )?,
            archived_segment_headers,
            archived_receipt_root: skipped_commitment_root(
                ARCHIVED_RECEIPT_DOMAIN,
                &archived_evidence_receipts,
            )?,
            archived_evidence_receipts,
            evidence_dependency_root: skipped_commitment_root(
                EVIDENCE_DEPENDENCY_DOMAIN,
                &evidence_dependencies,
            )?,
            evidence_dependencies,
        };
        if include_paged_payloads {
            validate_compact_continuation(&checkpoint)?;
        }
        Ok(checkpoint)
    }

    /// Captures current authoritative state without cloning accumulated evidence.
    pub fn checkpoint(&self) -> Result<SimulationCheckpoint, CanwuError> {
        self.checkpoint_with_paged_payloads(true)
    }

    /// Builds the complete kernel and plugin mark set used before offline
    /// archive garbage collection. Every registered plugin participant is
    /// invoked automatically; callers cannot accidentally sweep a plugin
    /// archive by forgetting a second, manual manifest-extension step.
    pub fn archive_reachability_manifest(
        &self,
        retained_checkpoints: &[SimulationCheckpoint],
        page_retention: &StatePageRetentionLedger,
        decision_provider: &dyn DecisionArchiveProvider,
        plugin_provider: &dyn super::PluginArchiveObjectProvider,
    ) -> Result<ArchiveReachabilityManifest, CanwuError> {
        let mut manifest = ArchiveReachabilityManifest {
            state_page_ids: page_retention.reachable_page_ids(),
            evidence_segment_ids: SimulationCheckpoint::reachable_archive_segment_ids(
                retained_checkpoints,
            )?,
            ..ArchiveReachabilityManifest::default()
        };
        manifest.evidence_segment_ids.extend(
            self.state
                .evidence
                .archived_segment_headers
                .iter()
                .map(|header| header.segment_id.clone()),
        );
        let decisions = self
            .state
            .current
            .decisions
            .archive_reachability(decision_provider)
            .map_err(super::decision::decision_error)?;
        manifest.state_page_ids.extend(decisions.bucket_page_ids);
        manifest.decision_blob_ids.extend(decisions.blob_locators);
        let pending_ingress_ids = self
            .state
            .scheduler
            .pending_ingress
            .iter()
            .map(|key| key.id)
            .collect::<BTreeSet<_>>();
        for record in self.state.evidence.ingress_records() {
            if let IngressPayload::Maintenance { request } = &record.payload
                && let super::MaintenanceIngressRequest::DecisionArchive { commit } =
                    request.as_ref()
            {
                manifest
                    .decision_blob_ids
                    .extend(commit.archive_locators().map(str::to_owned));
            }
            if pending_ingress_ids.contains(&record.id)
                && let IngressPayload::Plugin {
                    archive_retention, ..
                } = &record.payload
            {
                for retention in archive_retention {
                    manifest.insert_plugin_object(
                        retention.namespace.clone(),
                        retention.object_id.clone(),
                    );
                }
            }
        }
        for (plugin, participant) in &self.plugins.archive_reachability_participants {
            let reads = self
                .plugins
                .state_owners
                .iter()
                .filter_map(|(state, owner)| (owner == plugin).then_some(state.clone()))
                .collect::<Vec<_>>();
            let reader = format!("{plugin}.archive_reachability");
            let view = self.plugin_view(&reader, &reads);
            participant(&view, plugin_provider, &mut manifest)?;
        }
        Ok(manifest)
    }

    /// Clones only evidence appended after a previously persisted cursor.
    pub fn journal_segment_since(
        &self,
        start: EvidenceCursor,
    ) -> Result<EvidenceJournalSegment, CanwuError> {
        let end = self.evidence_cursor()?;
        let cut = |value: u64, archived: u64, len: usize, label: &str| {
            let value = value.checked_sub(archived).ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::InvalidSnapshot,
                    format!("{label} journal cursor precedes the retained live evidence window"),
                )
            })?;
            let value = usize::try_from(value).map_err(|_| {
                CanwuError::new(
                    ErrorCode::InvalidSnapshot,
                    format!("{label} journal cursor is not representable on this platform"),
                )
            })?;
            if value > len {
                return Err(CanwuError::new(
                    ErrorCode::InvalidSnapshot,
                    format!("{label} journal cursor exceeds the current evidence tail"),
                ));
            }
            Ok(value)
        };
        let archived = self.state.evidence.archived;
        let event_start = cut(
            start.event_count,
            archived.event_count,
            self.state.evidence.events.len(),
            "event",
        )?;
        let command_start = cut(
            start.command_count,
            archived.command_count,
            self.state.evidence.commands.len(),
            "command",
        )?;
        let attempt_start = cut(
            start.command_attempt_count,
            archived.command_attempt_count,
            self.state.evidence.command_attempts.len(),
            "command-attempt",
        )?;
        let ingress_start = cut(
            start.ingress_count,
            archived.ingress_count,
            self.state.evidence.ingress.len(),
            "ingress",
        )?;
        let boundary_start = cut(
            start.boundary_count,
            archived.boundary_count,
            self.state.evidence.boundaries.len(),
            "boundary",
        )?;
        let draw_start = cut(
            start.random_draw_count,
            archived.random_draw_count,
            self.state.evidence.random_draws.len(),
            "random-draw",
        )?;
        Ok(EvidenceJournalSegment {
            format_version: CHECKPOINT_JOURNAL_FORMAT_VERSION,
            start,
            end,
            events: self.state.evidence.events[event_start..].to_vec(),
            commands: self.state.evidence.commands[command_start..].to_vec(),
            command_attempts: self.state.evidence.command_attempts[attempt_start..].to_vec(),
            ingress: self.state.evidence.ingress[ingress_start..].to_vec(),
            boundaries: self.state.evidence.boundaries[boundary_start..].to_vec(),
            random_draws: self.state.evidence.random_draws[draw_start..].to_vec(),
            archive: None,
        })
    }

    /// Builds a portable full-save bundle with one segment from genesis.
    pub fn checkpoint_journal(&self) -> Result<CheckpointJournal, CanwuError> {
        self.ensure_full_history("a portable save")?;
        if self.state.evidence.archived != EvidenceCursor::default() {
            return Err(CanwuError::new(
                ErrorCode::InvalidSnapshot,
                "a compact live runtime requires its previously sealed evidence segments to build a portable save",
            ));
        }
        let segment = self.journal_segment_since(EvidenceCursor::default())?;
        Ok(CheckpointJournal {
            checkpoint: self.checkpoint()?,
            segments: (segment.start != segment.end)
                .then_some(segment)
                .into_iter()
                .collect(),
        })
    }

    /// Serializes the portable full-save checkpoint-journal bundle as JSON.
    pub fn checkpoint_journal_json(&self) -> Result<String, CanwuError> {
        serde_json::to_string_pretty(&self.checkpoint_journal()?).map_err(|error| {
            CanwuError::new(
                ErrorCode::InvalidSnapshot,
                format!("could not serialize checkpoint journal: {error}"),
            )
        })
    }

    /// Concatenates a checkpoint and its contiguous evidence prefix into a
    /// full snapshot and returns it with the compact continuation the
    /// checkpoint claims, which the caller checks against the one runtime it
    /// builds from the snapshot.
    fn reconstruct_checkpoint_snapshot(
        checkpoint: SimulationCheckpoint,
        segments: Vec<EvidenceJournalSegment>,
    ) -> Result<(SimulationSnapshot, ReconstructedCheckpoint), CanwuError> {
        if checkpoint.format_version != CHECKPOINT_JOURNAL_FORMAT_VERSION {
            return Err(invalid_snapshot_error(format!(
                "checkpoint-journal format {} is unsupported; this engine reads format {CHECKPOINT_JOURNAL_FORMAT_VERSION}",
                checkpoint.format_version
            )));
        }
        validate_compact_continuation(&checkpoint)?;
        let expected_headers = checkpoint.archived_segment_headers;
        let expected_receipts = checkpoint.archived_evidence_receipts;
        let expected_dependencies = checkpoint.evidence_dependencies;
        let mut snapshot = checkpoint.state;
        if snapshot.snapshot_format_version != SNAPSHOT_FORMAT_VERSION {
            return Err(invalid_snapshot_error(format!(
                "checkpoint-journal format {CHECKPOINT_JOURNAL_FORMAT_VERSION} requires snapshot format {SNAPSHOT_FORMAT_VERSION}"
            )));
        }
        if !snapshot.events.is_empty()
            || !snapshot.commands.is_empty()
            || !snapshot.command_attempts.is_empty()
            || !snapshot.ingress.is_empty()
            || !snapshot.boundaries.is_empty()
            || !snapshot.random_draws.is_empty()
        {
            return Err(invalid_snapshot_error(
                "checkpoint current state must not duplicate append-only evidence",
            ));
        }

        let mut cursor = EvidenceCursor::default();
        let mut rebuilt_headers = Vec::new();
        let mut rebuilt_receipts = BTreeMap::new();
        for segment in segments {
            if segment.format_version != CHECKPOINT_JOURNAL_FORMAT_VERSION {
                return Err(invalid_snapshot_error(format!(
                    "evidence-journal format {} is unsupported; this engine reads format {CHECKPOINT_JOURNAL_FORMAT_VERSION}",
                    segment.format_version
                )));
            }
            if segment.start != cursor {
                return Err(invalid_snapshot_error(
                    "evidence-journal segments must form one contiguous global prefix",
                ));
            }
            let end = cursor.checked_advance(&segment)?;
            if end == cursor {
                return Err(invalid_snapshot_error(
                    "evidence-journal segments must advance at least one journal cursor",
                ));
            }
            if segment.end != end {
                return Err(invalid_snapshot_error(
                    "evidence-journal segment end does not match its encoded records",
                ));
            }
            if let Some(archive) = &segment.archive {
                let receipts = verify_archived_segment(&segment).map_err(|error| {
                    invalid_snapshot_error(format!(
                        "archived evidence segment is invalid: {}",
                        error.message
                    ))
                })?;
                rebuilt_headers.push(archive.header.clone());
                for receipt in receipts {
                    if rebuilt_receipts
                        .insert(receipt.evidence.clone(), receipt)
                        .is_some()
                    {
                        return Err(invalid_snapshot_error(
                            "archived evidence segments contain duplicate receipts",
                        ));
                    }
                }
            }
            snapshot.events.extend(segment.events);
            snapshot.commands.extend(segment.commands);
            snapshot.command_attempts.extend(segment.command_attempts);
            snapshot.ingress.extend(segment.ingress);
            snapshot.boundaries.extend(segment.boundaries);
            snapshot.random_draws.extend(segment.random_draws);
            cursor = end;
        }
        if cursor != checkpoint.journal_end {
            return Err(invalid_snapshot_error(
                "evidence-journal segments do not reach the checkpoint journal cut",
            ));
        }
        Ok((
            snapshot,
            ReconstructedCheckpoint {
                rebuilt_headers,
                rebuilt_receipts,
                expected_headers,
                expected_receipts,
                expected_dependencies,
            },
        ))
    }

    /// Restores a checkpoint after proving a contiguous journal prefix.
    pub fn from_checkpoint_and_journal(
        checkpoint: SimulationCheckpoint,
        segments: Vec<EvidenceJournalSegment>,
    ) -> Result<Self, CanwuError> {
        let (snapshot, reconstructed) =
            Self::reconstruct_checkpoint_snapshot(checkpoint, segments)?;
        let simulation = Self::from_snapshot(snapshot)?;
        reconstructed.verify(&simulation)?;
        Ok(simulation)
    }

    /// Restores a portable checkpoint-journal bundle.
    pub fn from_checkpoint_journal(bundle: CheckpointJournal) -> Result<Self, CanwuError> {
        Self::from_checkpoint_and_journal(bundle.checkpoint, bundle.segments)
    }

    /// Restores a bundle and rehydrates its exact executable plugin contracts.
    pub fn from_checkpoint_journal_with_plugins(
        bundle: CheckpointJournal,
        plugins: &[&dyn SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        let mut simulation = Self::from_checkpoint_journal(bundle)?;
        for plugin in plugins {
            simulation.register_plugin(*plugin)?;
        }
        simulation.ensure_runtime_ready()?;
        Ok(simulation)
    }

    /// Deserializes and restores a portable checkpoint-journal JSON bundle.
    pub fn from_checkpoint_journal_json(json: &str) -> Result<Self, CanwuError> {
        let bundle: CheckpointJournal =
            super::deserialize_current_json(json, "checkpoint journal")?;
        Self::from_checkpoint_journal(bundle)
    }

    /// Deserializes a bundle and rehydrates its exact plugin contracts.
    pub fn from_checkpoint_journal_json_with_plugins(
        json: &str,
        plugins: &[&dyn SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        let bundle: CheckpointJournal =
            super::deserialize_current_json(json, "checkpoint journal")?;
        Self::from_checkpoint_journal_with_plugins(bundle, plugins)
    }
}

/// Runs the real `Simulation` paged-checkpoint boundary over a decision
/// locator fixture, including storage, authenticated restore, empty-suffix
/// replay, exact provider-backed lookups, and a zero-page repeat delta.
pub fn format8_paged_checkpoint_scale_probe(
    decision_count: usize,
) -> Result<PagedCheckpointScaleMetrics, CanwuError> {
    let fixture = canwu_decision::format8_decision_locator_scale_fixture(decision_count)
        .map_err(|error| invalid_snapshot_error(error.to_string()))?;
    let decision_metrics = fixture.metrics.clone();
    let decision_blobs = fixture
        .archive_blobs
        .into_iter()
        .map(|blob| {
            blob.content_id()
                .map(|content_id| (content_id, blob))
                .map_err(|error| invalid_snapshot_error(error.to_string()))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let store = Format8ScalePageStore {
        pages: RefCell::new(BTreeMap::new()),
        decision_blobs: RefCell::new(decision_blobs),
        provider_calls: RefCell::new(0),
    };
    let mut simulation = Simulation::new(8, Scenario::new(SimTime::EPOCH, Vec::new()))?;
    simulation.state.current.decisions = fixture.state;
    simulation.state.metadata.plugin_registration_closed = true;
    simulation.state.metadata.commitment_cache = None;
    simulation.refresh_checkpoint_hash()?;

    let prepared = simulation.prepare_paged_checkpoint(None, &store)?;
    let initial_provider_calls = *store.provider_calls.borrow();
    let initial_delta_pages = prepared.delta.new_pages.len() as u64;
    prepared.store_and_verify(&store)?;
    let root_page = store
        .load_state_page(&prepared.checkpoint.root_page_id)?
        .ok_or_else(|| invalid_snapshot_error("Format-8 scale root page is unavailable"))?;
    let envelope: PagedCheckpointEnvelope = serde_json::from_slice(&root_page.bytes)
        .map_err(|error| invalid_snapshot_error(format!("invalid scale envelope: {error}")))?;
    let manifest_page = store
        .load_state_page(&envelope.decision_manifest_page_id)?
        .ok_or_else(|| invalid_snapshot_error("Format-8 scale manifest page is unavailable"))?;
    let manifest: PagedDecisionManifest = serde_json::from_slice(&manifest_page.bytes)
        .map_err(|error| invalid_snapshot_error(format!("invalid scale manifest: {error}")))?;

    let restored = Simulation::from_paged_checkpoint(&prepared.checkpoint, &store)?;
    let replayed =
        Simulation::from_paged_checkpoint_and_journal(&prepared.checkpoint, &store, Vec::new())?;
    let samples = [
        1_usize,
        decision_count.saturating_div(2).max(1),
        decision_count,
    ]
    .into_iter()
    .filter(|ordinal| *ordinal <= decision_count)
    .collect::<BTreeSet<_>>();
    for ordinal in &samples {
        let key = DecisionHistoryKey::Attempt(canwu_core::DecisionRequestId::new(
            u64::try_from(*ordinal)
                .map_err(|_| invalid_snapshot_error("decision scale sample exceeds u64"))?,
        ));
        if !matches!(
            restored.decision_history_location_with_provider(&key, &store)?,
            DecisionHistoryLocation::Archived { .. }
        ) || !matches!(
            replayed.decision_history_location_with_provider(&key, &store)?,
            DecisionHistoryLocation::Archived { .. }
        ) {
            return Err(invalid_snapshot_error(
                "paged checkpoint scale restore lost exact decision history",
            ));
        }
    }
    *store.provider_calls.borrow_mut() = 0;
    let repeat = restored.prepare_paged_checkpoint(Some(&prepared.checkpoint), &store)?;
    let repeat_provider_calls = *store.provider_calls.borrow();
    let replay_repeat = replayed.prepare_paged_checkpoint(Some(&prepared.checkpoint), &store)?;
    let mut changed = restored;
    let changed_request_id = canwu_core::DecisionRequestId::new(
        u64::try_from(decision_count)
            .map_err(|_| invalid_snapshot_error("decision scale count exceeds u64"))?
            .checked_add(1)
            .ok_or_else(|| invalid_snapshot_error("decision scale request ID overflowed"))?,
    );
    changed
        .state
        .current
        .decisions
        .append_attempt(super::DecisionAttemptRecord {
            request_id: changed_request_id,
            request_commitment: canonical_byte_hash(
                "canwu.format8.single-page-change.v1",
                &changed_request_id.get().to_be_bytes(),
            ),
            at: SimTime::from_minutes(
                i64::try_from(changed_request_id.get())
                    .map_err(|_| invalid_snapshot_error("decision scale time exceeds i64"))?,
            ),
            revision_before: 0,
            expected_revision: 0,
            outcome: super::DecisionAttemptOutcome::Rejected {
                code: super::DecisionAttemptErrorCode::InvalidDecision,
                message: "Format-8 single locator-page change".to_owned(),
            },
        })
        .map_err(|error| invalid_snapshot_error(error.to_string()))?;
    let changed_key = DecisionHistoryKey::Attempt(changed_request_id);
    let changed_archive = changed
        .state
        .current
        .decisions
        .prepare_decision_archive(std::slice::from_ref(&changed_key))
        .map_err(|error| invalid_snapshot_error(error.to_string()))?;
    for blob in &changed_archive.blobs {
        store.decision_blobs.borrow_mut().insert(
            blob.content_id()
                .map_err(|error| invalid_snapshot_error(error.to_string()))?,
            blob.clone(),
        );
    }
    let verified_change = changed
        .state
        .current
        .decisions
        .verify_decision_archive(&changed_archive, &store)
        .map_err(|error| invalid_snapshot_error(error.to_string()))?;
    changed.state.current.decisions = changed
        .state
        .current
        .decisions
        .commit_verified_decision_archive(&verified_change)
        .map_err(|error| invalid_snapshot_error(error.to_string()))?;
    changed.state.metadata.commitment_cache = None;
    changed.refresh_checkpoint_hash()?;
    *store.provider_calls.borrow_mut() = 0;
    let single_page_change =
        changed.prepare_paged_checkpoint(Some(&prepared.checkpoint), &store)?;
    let single_page_change_provider_calls = *store.provider_calls.borrow();
    let pages = store.pages.borrow();
    Ok(PagedCheckpointScaleMetrics {
        decision_entries: decision_metrics.entries,
        decision_locator: decision_metrics,
        state_pages: pages.len() as u64,
        decision_directory_pages: manifest.archive_directory_page_ids.len() as u64,
        max_state_page_bytes: pages
            .values()
            .map(|page| page.decoded_bytes)
            .max()
            .unwrap_or(0),
        initial_delta_pages,
        repeat_delta_pages: repeat.delta.new_pages.len() as u64,
        single_page_change_delta_pages: single_page_change.delta.new_pages.len() as u64,
        initial_provider_calls,
        repeat_provider_calls,
        single_page_change_provider_calls,
        exact_restart_queries: samples.len() as u64,
        restored_root_matches: repeat.checkpoint.root_page_id == prepared.checkpoint.root_page_id,
        replayed_root_matches: replay_repeat.checkpoint.root_page_id
            == prepared.checkpoint.root_page_id,
        root_page_id: prepared.checkpoint.root_page_id,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unnecessary_literal_bound, clippy::unnecessary_wraps)]
    use super::super::{
        BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal,
        BoundarySystemContract, Command, CommandContext, CommandRequestId, DomainRecordClass,
        DomainRecordDraft, DomainRecordMutation, Issuer, KnowledgeHolderRef, KnowledgeOrigin,
        KnowledgeRecordDraft, KnowledgeRecordKind, KnowledgeSchemaId, KnowledgeWriteGrant,
        PluginActionDescriptor, PluginKnowledgeSchema, PluginRegistrar, SimulationView, StateKey,
        StateVisibility, SystemDirective, demo_scenario,
    };
    use super::*;
    use canwu_core::{BoundaryId, CommandId, DomainRecordKind, PersonId};
    use serde_json::{Map, Value, json};
    use std::cell::RefCell;

    fn decision_directory_page(start: u32, len: usize) -> PagedDecisionDirectoryPage {
        PagedDecisionDirectoryPage {
            format_version: PAGED_CHECKPOINT_FORMAT_VERSION,
            archive_bucket_pages: (0..len)
                .map(|offset| {
                    let ordinal = start + u32::try_from(offset).expect("test offset fits u32");
                    (
                        super::super::DecisionArchivePageKey {
                            bucket: u16::try_from(ordinal / 256).expect("bucket fits"),
                            segment: u8::try_from(ordinal % 256).expect("segment fits"),
                        },
                        canonical_byte_hash(
                            "canwu.test.decision-directory-page.v1",
                            &ordinal.to_be_bytes(),
                        ),
                    )
                })
                .collect(),
        }
    }

    #[test]
    fn paged_decision_directory_rejects_noncanonical_cross_page_encodings() {
        let first = decision_directory_page(0, MAX_PAGED_DECISION_DIRECTORY_ENTRIES);
        let second = decision_directory_page(
            u32::try_from(MAX_PAGED_DECISION_DIRECTORY_ENTRIES).expect("directory bound fits u32"),
            1,
        );
        assemble_paged_decision_directory(vec![second.clone(), first.clone()])
            .expect_err("swapped directory pages must fail");
        assemble_paged_decision_directory(vec![decision_directory_page(0, 1), second])
            .expect_err("a short middle directory page must fail");

        let duplicate_id = canonical_byte_hash("canwu.test.duplicate-directory.v1", b"same");
        let manifest = PagedDecisionManifest {
            format_version: PAGED_CHECKPOINT_FORMAT_VERSION,
            hot_page_id: canonical_byte_hash("canwu.test.hot.v1", b"hot"),
            archive_receipt_root: canonical_byte_hash("canwu.test.archive.v1", b"archive"),
            archive_receipt_count: 1,
            archive_directory_page_ids: vec![duplicate_id.clone(), duplicate_id],
        };
        validate_paged_decision_manifest(&manifest)
            .expect_err("duplicate directory page IDs must fail");
    }

    #[test]
    fn archived_plugin_ingress_provenance_is_merkle_bound() {
        let provenance = ArchivedPluginIngressProvenance {
            plugin: "fixture-provider".to_owned(),
            packet_type: "recognized-practice".to_owned(),
            producer_boundary: BoundaryId::new(7),
        };
        let entry = EvidenceIndexEntry {
            reference: EvidenceRef::Ingress(super::super::IngressId::new(9)),
            item: EvidenceItemLocator {
                journal: EvidenceJournalKind::Ingress,
                absolute_index: 9,
                nested: EvidenceNestedLocator::None,
            },
            item_commitment: "0101010101010101010101010101010101010101010101010101010101010101"
                .to_owned(),
            plugin_ingress_provenance: Some(provenance.clone()),
        };
        let (root, proofs) = archive_merkle(std::slice::from_ref(&entry)).expect("build proof");
        let header = ArchivedSegmentHeader {
            segment_id: "0202020202020202020202020202020202020202020202020202020202020202"
                .to_owned(),
            start: EvidenceCursor {
                ingress_count: 8,
                ..EvidenceCursor::default()
            },
            end: EvidenceCursor {
                ingress_count: 9,
                ..EvidenceCursor::default()
            },
            journal_roots: EvidenceJournalRoots {
                events: archive_empty_root(),
                commands: archive_empty_root(),
                command_attempts: archive_empty_root(),
                ingress: archive_empty_root(),
                boundaries: archive_empty_root(),
                random_draws: archive_empty_root(),
            },
            evidence_index_root: root,
            evidence_index_entry_count: 1,
        };
        let mut receipt = ArchivedEvidenceReceipt {
            evidence: entry.reference,
            locator: ArchivedEvidenceLocator {
                segment_id: header.segment_id.clone(),
                item: entry.item,
            },
            evidence_index_leaf: 0,
            item_commitment: entry.item_commitment,
            plugin_ingress_provenance: Some(provenance),
            merkle_path: proofs.into_iter().next().expect("one proof"),
        };
        verify_archive_receipt(&receipt, &header).expect("canonical provenance proof");

        receipt
            .plugin_ingress_provenance
            .as_mut()
            .expect("provenance")
            .plugin = "forged-provider".to_owned();
        assert_eq!(
            verify_archive_receipt(&receipt, &header)
                .expect_err("tampered provenance must break the proof")
                .code,
            ErrorCode::InvalidArchive
        );
    }

    #[derive(Default)]
    struct TestArchive {
        segments: RefCell<BTreeMap<String, EvidenceJournalSegment>>,
    }

    impl TestArchive {
        fn segment_ids(&self) -> Vec<String> {
            self.segments.borrow().keys().cloned().collect()
        }
    }

    impl ArchiveProvider for TestArchive {
        fn load_evidence_segment(
            &self,
            segment_id: &str,
        ) -> Result<Option<EvidenceJournalSegment>, CanwuError> {
            Ok(self.segments.borrow().get(segment_id).cloned())
        }
    }

    impl ArchiveStore for TestArchive {
        fn store_evidence_segment(
            &self,
            segment: &EvidenceJournalSegment,
        ) -> Result<ArchiveStoreOutcome, CanwuError> {
            let segment_id = segment
                .archive
                .as_ref()
                .ok_or_else(|| archive_error("test archive segment has no index"))?
                .header
                .segment_id
                .clone();
            let mut segments = self.segments.borrow_mut();
            if let Some(existing) = segments.get(&segment_id) {
                return if existing == segment {
                    Ok(ArchiveStoreOutcome::AlreadyPresent)
                } else {
                    Err(archive_error(
                        "content-addressed test segment ID has conflicting bytes",
                    ))
                };
            }
            segments.insert(segment_id, segment.clone());
            Ok(ArchiveStoreOutcome::Stored)
        }
    }

    fn archived_identity_schema() -> KnowledgeSchemaId {
        KnowledgeSchemaId::new(
            KnowledgeRecordKind::new("fixture.archive", "archived_command_notice"),
            1,
        )
    }

    #[allow(clippy::unnecessary_wraps)]
    fn retain_archive_source_command(
        _view: &SimulationView<'_>,
        _context: &CommandContext,
        _payload: &Value,
    ) -> Result<Vec<SystemDirective>, CanwuError> {
        Ok(Vec::new())
    }

    #[allow(clippy::unnecessary_wraps)]
    fn publish_archived_command_identity(
        _view: &SimulationView<'_>,
        context: &BoundaryContext,
    ) -> Result<BoundaryProposal, CanwuError> {
        let record = KnowledgeRecordDraft {
            schema: archived_identity_schema(),
            subjects: Vec::new(),
            payload: json!({ "boundary": context.boundary_id.get() }),
            as_of: None,
            confidence_per_mille: 1_000,
            origin: KnowledgeOrigin {
                method: "archived_command_identity_v1".to_owned(),
                evidence: vec![EvidenceRef::Command(CommandId::new(1))],
            },
            supersedes: Vec::new(),
            contradicts: Vec::new(),
        };
        Ok(BoundaryProposal {
            directives: vec![BoundaryDirective::PublishKnowledge {
                visibility: StateVisibility::SameBoundary,
                producer_correlation: Some(format!(
                    "archived-command-boundary-{}",
                    context.boundary_id.get()
                )),
                supersession: crate::KnowledgeSupersession::Explicit,
                groups: vec![crate::KnowledgePublicationGroup {
                    holders: vec![KnowledgeHolderRef::Person(PersonId::new(1))],
                    records: vec![record],
                }],
                summary: "Publish knowledge from a retained or archived command identity"
                    .to_owned(),
            }],
            ..BoundaryProposal::default()
        })
    }

    struct ArchivedIdentityPublicationPlugin;

    impl SimulationPlugin for ArchivedIdentityPublicationPlugin {
        fn name(&self) -> &str {
            "fixture-archived-identity-publication"
        }

        fn version(&self) -> &str {
            "test-v1"
        }

        fn semantic_hash(&self) -> &str {
            "7100000000000000000000000000000000000000000000000000000000000000"
        }

        fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
            registrar.register_knowledge_schema(PluginKnowledgeSchema {
                id: archived_identity_schema(),
                schema_hash: "7200000000000000000000000000000000000000000000000000000000000000"
                    .to_owned(),
                writable: true,
                payload_schema: PayloadSchema::Any,
                subjects: Vec::new(),
                retention: crate::KnowledgeRetention::Keep,
            })?;
            registrar.register_command(
                PluginActionDescriptor {
                    name: "retain_archive_source_v1".to_owned(),
                    description: "Persist one neutral command identity for archive testing"
                        .to_owned(),
                    payload_schema: PayloadSchema::Any,
                    reads: Vec::new(),
                    writes: Vec::new(),
                },
                retain_archive_source_command,
            )?;
            let mut publisher = BoundarySystemContract::new(
                "publish-archived-command-identity",
                BoundaryPhase::PerspectiveAndReportMaterialization,
                SystemCadence::Daily,
            );
            publisher.knowledge_writes = vec![KnowledgeWriteGrant {
                schema: archived_identity_schema(),
                visibilities: vec![StateVisibility::SameBoundary],
            }];
            registrar.register_boundary_system(publisher, publish_archived_command_identity)
        }
    }

    fn archived_identity_two_segment_fixture() -> (SimulationCheckpoint, Vec<EvidenceJournalSegment>)
    {
        let (scenario, _) = demo_scenario();
        let plugin = ArchivedIdentityPublicationPlugin;
        let mut simulation = Simulation::new(711, scenario).expect("fixture scenario should load");
        simulation
            .register_plugin(&plugin)
            .expect("archived-identity plugin should register");
        simulation
            .enqueue_command(
                SimTime::EPOCH,
                0,
                CommandRequest::new(
                    CommandRequestId::new(1),
                    simulation.revision(),
                    CommandEnvelope::new(
                        Issuer::System("archive-identity-fixture".to_owned()),
                        Command::Plugin {
                            plugin: plugin.name().to_owned(),
                            command: "retain_archive_source_v1".to_owned(),
                            payload: json!({ "format_version": 1 }),
                        },
                    )
                    .at_time(SimTime::EPOCH),
                ),
            )
            .expect("archive source command should queue");
        simulation
            .settle_boundary(BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily))
            .expect("boundary one should retain the command-backed publication");
        simulation
            .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
            .expect("the following cut should admit boundary-one events before sealing");

        let mut compact = simulation
            .into_compacted()
            .expect("the fixture should enter compact mode");
        let first_segment = compact
            .seal_evidence()
            .expect("boundary-one evidence should seal")
            .expect("boundary one should produce an archive segment");
        let first_checkpoint = compact.checkpoint().expect("checkpoint one should build");
        assert!(
            first_checkpoint
                .archived_evidence_receipts
                .iter()
                .any(|receipt| receipt.evidence == EvidenceRef::Command(CommandId::new(1)))
        );

        let mut restored = CompactedSimulation::from_checkpoint_and_journal_with_plugins(
            first_checkpoint,
            vec![first_segment.clone()],
            &[&plugin],
        )
        .expect("checkpoint one should restore with its exact archive prefix");
        let restored_prefix = restored
            .seal_evidence()
            .expect("restored boundary-one evidence should reseal")
            .expect("restored boundary-one evidence should remain non-empty");
        assert_eq!(restored_prefix, first_segment);
        assert!(
            restored
                .archived_evidence_receipt(&EvidenceRef::Command(CommandId::new(1)))
                .is_some(),
            "the second publication must consume an archived identity, not retained payload"
        );
        let archived_reads = [StateKey::core_commands()];
        let archived_view = restored
            .simulation
            .plugin_view("archive-identity-probe", &archived_reads);
        let error = archived_view
            .command(CommandId::new(1))
            .expect_err("archived identity must not expose retained command payload");
        assert_eq!(error.code, ErrorCode::EvidenceContentUnavailable);
        restored
            .settle_boundary(
                BoundaryRequest::at(SimTime::EPOCH + SimDuration::days(1))
                    .with_cadence(SystemCadence::Daily),
            )
            .expect("boundary two should accept the archived command identity");
        let holder = KnowledgeHolderRef::Person(PersonId::new(1));
        let records = restored
            .knowledge()
            .for_holder(&holder)
            .expect("the holder should have both publications");
        assert_eq!(records.len(), 2);
        assert!(records.values().all(|record| {
            record.origin.evidence == vec![EvidenceRef::Command(CommandId::new(1))]
        }));
        restored
            .settle_boundary(BoundaryRequest::at(SimTime::EPOCH + SimDuration::days(1)))
            .expect("the following cut should admit boundary-two events before sealing");

        let second_segment = restored
            .seal_evidence()
            .expect("boundary-two evidence should seal")
            .expect("boundary two should produce an archive segment");
        let second_checkpoint = restored.checkpoint().expect("checkpoint two should build");
        (second_checkpoint, vec![restored_prefix, second_segment])
    }

    #[test]
    fn compact_restore_preserves_archived_identity_and_rejects_noncontiguous_segments() {
        let plugin = ArchivedIdentityPublicationPlugin;
        let (checkpoint, segments) = archived_identity_two_segment_fixture();
        let restored = CompactedSimulation::from_checkpoint_and_journal_with_plugins(
            checkpoint.clone(),
            segments.clone(),
            &[&plugin],
        )
        .expect("the complete two-segment archive should restore");
        assert_eq!(
            restored
                .knowledge()
                .for_holder(&KnowledgeHolderRef::Person(PersonId::new(1)))
                .expect("published holder ledger")
                .len(),
            2
        );

        let first = segments[0].clone();
        let second = segments[1].clone();
        for (label, tampered) in [
            ("omission", vec![first.clone()]),
            (
                "overlap",
                vec![first.clone(), first.clone(), second.clone()],
            ),
            ("reorder", vec![second, first]),
        ] {
            let error = Simulation::from_checkpoint_and_journal(checkpoint.clone(), tampered)
                .err()
                .expect(label);
            assert_eq!(error.code, ErrorCode::InvalidSnapshot, "{label}");
        }
    }

    struct PayloadContinuationPlugin;

    fn continuation_record_ref() -> DomainRecordRef {
        DomainRecordRef {
            kind: DomainRecordKind::new("fixture.archive", "payload_continuation"),
            id: "primary".to_owned(),
        }
    }

    fn continuation_payload(continuation: PayloadRequiredEvidenceContinuationV1) -> Value {
        Value::Object(Map::from_iter([(
            PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD.to_owned(),
            serde_json::to_value(continuation).expect("fixture continuation should encode"),
        )]))
    }

    fn mutate_payload_continuation(
        _view: &SimulationView<'_>,
        context: &BoundaryContext,
    ) -> Result<BoundaryProposal, CanwuError> {
        let directive = match context.boundary_id.get() {
            1 => Some(BoundaryDirective::MutateRecord {
                mutation: DomainRecordMutation::Create {
                    record: DomainRecordDraft::new(
                        continuation_record_ref(),
                        continuation_payload(PayloadRequiredEvidenceContinuationV1::active(vec![
                            EvidenceRef::Boundary(BoundaryId::new(1)),
                        ])),
                    ),
                },
                summary: "Create an active payload continuation".to_owned(),
            }),
            3 => Some(BoundaryDirective::MutateRecord {
                mutation: DomainRecordMutation::Update {
                    record: DomainRecordDraft::new(
                        continuation_record_ref(),
                        continuation_payload(PayloadRequiredEvidenceContinuationV1::completed()),
                    ),
                    expected_version: 1,
                },
                summary: "Complete the payload continuation".to_owned(),
            }),
            _ => None,
        };
        Ok(BoundaryProposal {
            directives: directive.into_iter().collect(),
            ..BoundaryProposal::default()
        })
    }

    impl SimulationPlugin for PayloadContinuationPlugin {
        fn name(&self) -> &str {
            "fixture-payload-continuation"
        }

        fn version(&self) -> &str {
            "test-v1"
        }

        fn semantic_hash(&self) -> &str {
            "7000000000000000000000000000000000000000000000000000000000000000"
        }

        fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
            let mut properties = BTreeMap::new();
            properties.insert(
                PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD.to_owned(),
                payload_required_evidence_continuation_property_v1(),
            );
            let mut schema =
                DomainRecordSchema::new(continuation_record_ref().kind, DomainRecordClass::Record);
            schema.payload_schema = PayloadSchema::Object {
                properties,
                allow_additional: false,
            };
            let state = schema.state_key();
            registrar.register_record_schema(schema)?;
            let mut contract = BoundarySystemContract::new(
                "payload-continuation",
                BoundaryPhase::DomainDeltaProposal,
                SystemCadence::Daily,
            );
            contract.writes = vec![state];
            contract.visibility = StateVisibility::SameBoundary;
            registrar.register_boundary_system(contract, mutate_payload_continuation)
        }
    }

    fn payload_continuation_runtime() -> CompactedSimulation {
        let (scenario, _) = demo_scenario();
        let mut simulation = Simulation::new(701, scenario).expect("fixture scenario should load");
        simulation
            .register_plugin(&PayloadContinuationPlugin)
            .expect("payload-continuation plugin should register");
        simulation
            .settle_boundary(BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily))
            .expect("the active continuation should be created");
        simulation
            .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
            .expect("a later boundary should admit the record-change event");
        simulation
            .into_compacted()
            .expect("the fixture should enter compact mode")
    }

    fn store_prepared(
        compact: &CompactedSimulation,
        archive: &TestArchive,
    ) -> PreparedEvidenceSeal {
        let prepared = compact
            .prepare_evidence_seal()
            .expect("the fixture should prepare a seal")
            .expect("the retained tail should be non-empty");
        assert_eq!(
            archive
                .store_evidence_segment(&prepared.segment)
                .expect("the prepared segment should store"),
            ArchiveStoreOutcome::Stored
        );
        prepared
    }

    #[test]
    fn payload_required_receipts_are_exactly_reachable_and_prune_after_completion() {
        let mut compact = payload_continuation_runtime();
        // A one-phase seal keeps the payload-required boundary pinned, so it
        // no longer needs the provider.
        let mut direct = compact.fork();
        direct
            .seal_evidence()
            .expect("a one-phase seal pins payload-required evidence")
            .expect("the settled tail should seal");
        assert!(
            direct
                .runtime()
                .state
                .evidence
                .pinned
                .boundaries
                .contains_key(&BoundaryId::new(1))
        );

        let archive = TestArchive::default();
        let first = store_prepared(&compact, &archive);
        compact
            .commit_evidence_seal(&first.token, &archive)
            .expect("provider-backed sealing should commit");
        let first_checkpoint = compact.checkpoint().expect("checkpoint should build");
        let first_references = first_checkpoint
            .archived_evidence_receipts
            .iter()
            .map(|receipt| receipt.evidence.clone())
            .collect::<BTreeSet<_>>();
        let first_dependencies = first_checkpoint
            .evidence_dependencies
            .iter()
            .map(|dependency| (dependency.reference.clone(), dependency.requirement))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            first_dependencies.get(&EvidenceRef::Boundary(BoundaryId::new(1))),
            Some(&EvidenceRequirement::PayloadRequired)
        );
        assert_eq!(first_references.len(), first_dependencies.len());
        assert_eq!(
            first_references,
            first_dependencies.keys().cloned().collect::<BTreeSet<_>>()
        );
        assert!(
            first
                .segment
                .archive
                .as_ref()
                .expect("prepared segment should have an archive index")
                .entries
                .len()
                > first_references.len(),
            "the full segment index must outlive the reachable-only receipt set"
        );

        compact
            .settle_boundary(BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily))
            .expect("the continuation should complete");
        compact
            .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
            .expect("a later boundary should admit the completion event");
        let second = store_prepared(&compact, &archive);
        compact
            .commit_evidence_seal(&second.token, &archive)
            .expect("the completed continuation should seal");
        let second_checkpoint = compact.checkpoint().expect("checkpoint should build");
        assert_eq!(second_checkpoint.archived_segment_headers.len(), 2);
        assert_eq!(second_checkpoint.archived_evidence_receipts.len(), 1);
        assert_eq!(second_checkpoint.evidence_dependencies.len(), 1);
        assert_eq!(
            second_checkpoint.archived_evidence_receipts[0].evidence,
            second_checkpoint.evidence_dependencies[0].reference
        );
        assert_eq!(
            second_checkpoint.evidence_dependencies[0].requirement,
            EvidenceRequirement::IdentityOnly
        );
        assert!(
            !second_checkpoint
                .archived_evidence_receipts
                .iter()
                .any(|receipt| receipt.evidence == EvidenceRef::Boundary(BoundaryId::new(1)))
        );

        Simulation::from_checkpoint_and_journal(
            second_checkpoint,
            vec![first.segment, second.segment],
        )
        .expect("reconstruction must filter full segment indexes to the compact receipt frontier");
    }

    #[test]
    fn payload_required_commit_accepts_a_pinned_payload_without_older_segments() {
        let mut compact = payload_continuation_runtime();
        let complete_archive = TestArchive::default();
        let first = store_prepared(&compact, &complete_archive);
        compact
            .commit_evidence_seal(&first.token, &complete_archive)
            .expect("the initial provider-backed seal should commit");

        compact
            .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
            .expect("a new tail should preserve the active continuation");
        // The provider holds only the new segment. The payload-required
        // boundary sealed by the first segment is pinned, so the commit does
        // not read the older segment back.
        let incomplete_archive = TestArchive::default();
        let second = store_prepared(&compact, &incomplete_archive);
        compact
            .commit_evidence_seal(&second.token, &incomplete_archive)
            .expect("a pinned payload satisfies the payload-required dependency");
        assert!(
            compact
                .runtime()
                .state
                .evidence
                .pinned
                .boundaries
                .contains_key(&BoundaryId::new(1))
        );
        assert_eq!(second.segment.start, first.segment.end);
    }

    #[test]
    fn host_orphan_candidates_follow_all_retained_manifests_without_deleting() {
        let mut compact = payload_continuation_runtime();
        let archive = TestArchive::default();
        let prepared = store_prepared(&compact, &archive);
        let stored_ids = archive.segment_ids();
        let before = compact.checkpoint().expect("checkpoint should build");
        assert_eq!(
            SimulationCheckpoint::orphaned_archive_segment_ids(
                std::slice::from_ref(&before),
                &stored_ids,
            )
            .expect("manifest reachability should validate"),
            vec![prepared.token.segment_id.clone()]
        );
        assert!(
            archive
                .load_evidence_segment(&prepared.token.segment_id)
                .expect("the store should remain readable")
                .is_some(),
            "the conformance API must not delete host content"
        );

        compact
            .commit_evidence_seal(&prepared.token, &archive)
            .expect("the stored segment should commit");
        let after = compact.checkpoint().expect("checkpoint should build");
        assert!(
            SimulationCheckpoint::orphaned_archive_segment_ids(
                std::slice::from_ref(&after),
                &stored_ids,
            )
            .expect("committed manifest reachability should validate")
            .is_empty()
        );
        assert_eq!(
            SimulationCheckpoint::reachable_archive_segment_ids(&[before, after])
                .expect("all retained manifests should be scanned"),
            BTreeSet::from([prepared.token.segment_id.clone()])
        );
    }
}
