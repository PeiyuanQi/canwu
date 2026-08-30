mod boundary;
mod decision;
mod error;
mod event_payloads;
mod hashing;
mod ingress;
mod knowledge;
mod legacy_world;
mod maintenance;
mod manifest;
mod page_store;
mod persistence;
mod plugins;
mod policy;
mod random;
mod records;
mod replay;
mod revision;
mod scenario;
mod scheduling;
mod settlement;
mod state;
mod transactions;
mod validation;
mod view;

pub use hashing::{canonical_byte_hash, canonical_hash};

pub use boundary::{
    BoundaryChange, BoundaryContext, BoundaryDirective, BoundaryEmission, BoundaryEmissionKind,
    BoundaryIngressGeneration, BoundaryKnowledgeChange, BoundaryProposal, BoundaryReceipt,
    BoundaryRecord, BoundaryRequest, BoundarySystemContract, BoundarySystemHandler,
    KnowledgeWriteGrant, OutboxEntry, PluginIngressTarget, ReservationAllocation,
    ReservationDisposition, ReservationOffer, ReservationOfferRecord, ReservationPoolKey,
    ReservationRef, ReservationRequest, ReservationRequestRecord,
};
pub use canwu_core::{
    DomainRecordVersionRef, DomainRecordVersionSource, EvidenceRef, HolderKnowledgeRecordId,
    KnowledgeHolderPolicy, KnowledgeHolderRef, KnowledgeRecordId, KnowledgeRecordKind,
    KnowledgeSchemaId,
};
pub use canwu_decision::{
    ControllerDecision, DECISION_ARCHIVE_BUCKET_PAGE_FORMAT_VERSION,
    DECISION_ARCHIVE_FORMAT_VERSION, DecisionAction, DecisionArchiveBlob,
    DecisionArchiveBucketPage, DecisionArchivePageKey, DecisionArchiveProvider,
    DecisionArchiveReceipt, DecisionArchiveRecord, DecisionArchiveStore,
    DecisionArchiveStoreOutcome, DecisionAttemptErrorCode, DecisionAttemptOutcome,
    DecisionAttemptRecord, DecisionAuthority, DecisionContext, DecisionController,
    DecisionControllerBinding, DecisionError, DecisionErrorCode, DecisionExternalEvidence,
    DecisionFactorContribution, DecisionHistoryCursor, DecisionHistoryKey, DecisionHistoryLocation,
    DecisionHistoryPage, DecisionHistoryQueryBudget, DecisionHotState, DecisionLocatorScaleMetrics,
    DecisionMutation, DecisionOption, DecisionOptionEvaluation, DecisionOutcome, DecisionPolicy,
    DecisionPolicyIdentity, DecisionPolicyKind, DecisionRule, DecisionState, DecisionTicket,
    DecisionTicketDraft, DecisionTicketState, DecisionTrace, ExternalDecisionOption,
    ExternalDecisionRequest, ExternalDecisionResponse, ExternalPolicy, HumanDecisionResponse,
    HumanPolicy, LlmModelIdentity, LlmPolicy, MAX_DECISION_ARCHIVE_BATCH_ENTRIES,
    MAX_DECISION_HISTORY_PAGE_BYTES, MAX_DECISION_HISTORY_PAGE_SIZE, OrderedRulePolicy,
    PersistentDecisionLog, PolicyDecision, PreparedDecisionArchive, QueuedExternalPolicy,
    QueuedHumanPolicy, QueuedLlmPolicy, RuleChoice, RulePolicy, TraceLocatorScaleMetrics,
    UtilityEvaluator, UtilityPolicy, UtilityProfile, VerifiedDecisionArchiveCommit,
    WeightedUtilityEvaluator, WeightedUtilityPolicy, format8_decision_locator_scale_probe,
    format8_trace_locator_scale_probe,
};
pub use decision::{
    DECISION_REQUEST_COMMITMENT_DOMAIN, DecisionEvaluation, DecisionIngressRequest,
    PreparedDecisionIngress,
};
pub use ingress::{
    IngressClass, IngressPayload, IngressReceipt, IngressRecord, MaintenanceChangeRecord,
    MaintenanceDisposition, MaintenanceIngressRequest, MaintenanceRejectionReceipt,
    PluginArchiveRetention, PluginIngressDescriptor, PluginIngressPermit, PluginIngressRequest,
};
pub use knowledge::{
    KnowledgeLimitsV1, KnowledgeSubjectSchema, KnowledgeSubjectTargetKind, PluginKnowledgeSchema,
};
pub use maintenance::{
    MAX_OWNER_AUTHORIZED_MUTATIONS, MAX_OWNER_AUTHORIZED_PARTICIPANTS,
    OWNER_AUTHORIZED_MAINTENANCE_FORMAT_VERSION, OwnerAuthorizedMaintenanceDraft,
    OwnerAuthorizedMaintenanceRequest, OwnerAuthorizedMutation, OwnerAuthorizedParticipantDraft,
    OwnerAuthorizedParticipantProposal, OwnerAuthorizedParticipantRole,
    OwnerAuthorizedRecordExpectation, VerifiedOwnerAuthorizedMaintenanceCommit,
};
pub use manifest::{ArtifactManifest, RUN_MANIFEST_FORMAT_VERSION, RunManifest};
pub use page_store::{
    MAX_STATE_DELTA_PAGES, MAX_STATE_PAGE_BYTES, PreparedStateDelta, STATE_PAGE_CODEC,
    STATE_PAGE_FORMAT_VERSION, STATE_PAGE_RETENTION_FORMAT_VERSION, StatePageBlob,
    StatePageProvider, StatePageRetentionHandle, StatePageRetentionLedger, StatePageRetentionPhase,
    StatePageStore, prepare_state_delta, state_page_id, verify_state_delta,
};
pub use persistence::{
    ArchiveProvider, ArchiveReachabilityManifest, ArchiveStore, ArchiveStoreOutcome,
    ArchivedEvidenceLocator, ArchivedEvidenceReceipt, ArchivedPluginIngressProvenance,
    ArchivedSegmentHeader, CHECKPOINT_JOURNAL_FORMAT_VERSION, CheckpointJournal,
    CompactedSimulation, EvidenceArchiveIndex, EvidenceCursor, EvidenceDependency,
    EvidenceIndexEntry, EvidenceItemLocator, EvidenceJournalKind, EvidenceJournalRoots,
    EvidenceJournalSegment, EvidenceNestedLocator, EvidenceRequirement, EvidenceSealToken,
    IDENTITY_EVIDENCE_DEPENDENCIES_FIELD, IDENTITY_EVIDENCE_DEPENDENCIES_FORMAT_VERSION,
    IdentityEvidenceDependenciesV1, PAGED_CHECKPOINT_FORMAT_VERSION,
    PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD,
    PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FORMAT_VERSION, PagedCheckpointScaleMetrics,
    PagedSimulationCheckpoint, PayloadRequiredEvidenceContinuationV1,
    PortablePagedSimulationCheckpoint, PreparedEvidenceSeal, PreparedPagedSimulationCheckpoint,
    ReplayJournal, SimulationCheckpoint, SimulationSnapshot, format8_paged_checkpoint_scale_probe,
    identity_evidence_dependencies_property_v1, payload_required_evidence_continuation_property_v1,
};
pub use plugins::{
    MaintenanceDependencyResolverDescriptor, OwnerAuthorizedMaintenanceParticipant,
    PluginArchiveObjectProvider, PluginArchiveReachabilityParticipant,
};
pub use policy::{
    CommandPolicyContext, ControllerPolicy, InteractionPolicy, ObservationPolicy,
    RUN_CONFIGURATION_FORMAT_VERSION, RunConfiguration, RunConfigurationSnapshot, RunPurpose,
    SeatBinding, SeatPolicy, TracePolicy,
};
pub use random::{
    KeyedDrawReservation, RandomAlgorithm, RandomDrawAddress, RandomDrawOutcome,
    RandomDrawProducer, RandomDrawRecord, RandomOperationAddressV1, RandomOperationTarget,
    RandomStreamKey, RandomStreamState,
};
pub use records::{
    DomainRecord, DomainRecordChange, DomainRecordClass, DomainRecordCommitmentRoots,
    DomainRecordDraft, DomainRecordLifecycle, DomainRecordMutation, DomainRecordMutationPolicy,
    DomainRecordOperation, DomainRecordPageRoots, DomainRecordSchema, DomainReference,
    DomainReferenceSchema, DomainReferenceTarget, DomainReferenceTargetKind, PatriciaStoreMetrics,
    PersistentDomainRecordStore, format8_patricia_scale_probe,
};

use canwu_core::{
    ArmyId, BoundaryId, CommandAttemptId, CommandId, CommandRequestId, DecisionRequestId,
    DecisionTicketId, DecisionTraceId, DeterministicRng, DomainRecordKind, DomainRecordRef,
    DomainRecordType, EntityRef, EventId, FieldSchema, GovernmentId, IngressId, LetterId, PersonId,
    RandomDrawId, ResourceId, RouteId, SchemaRegistry, TerritoryId, TypeSchema,
    TypedDomainRecordRef,
};
pub use canwu_event::{CauseRef, EventAudience, EventKind, SimEvent};
pub use canwu_knowledge::{
    ActorKnowledge, ArmyKnowledge, EstimateRange, KnowledgeCursor, KnowledgeHistoryView,
    KnowledgeOrigin, KnowledgeQuery, KnowledgeReadCut, KnowledgeRecord, KnowledgeRecordDraft,
    KnowledgeRecordView, KnowledgeSnapshot, KnowledgeSource, KnowledgeSubject,
    KnowledgeSubjectTarget,
};
use canwu_time::{SimDuration, SimTime};
pub use legacy_world::{
    Army, Government, LetterCargo, LetterStatus, MapPoint, Person, PersonTransitState, Route,
    Territory, TransitState, WorldSnapshot,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};

use event_payloads::{
    DebugFieldChanged, KNOWLEDGE_PUBLISHED, KnowledgePublished, MoveOrdered, PLUGIN,
    PersonMoveOrdered, RuntimeEventPayload,
};
use hashing::{
    ControlCommitmentMaterial, StateHashMaterial, authoritative_run_identity,
    boundary_state_hash_for_commitments, checkpoint_hash_for_commitments,
    checkpoint_hash_for_configuration, commitment_roots_are_canonical, compute_boundary_hash,
    decision_commitment_root, identity_commitment_root, is_canonical_hash,
    knowledge_commitment_root, plugin_component_commitment_root, random_stream_commitment_root,
    runtime_commitment_roots, scheduler_commitment_root, snapshot_boundary_head_state_hash,
    snapshot_checkpoint_hash, snapshot_commitment_roots, snapshot_is_at_boundary_head, state_hash,
    world_commitment_root,
};
use ingress::IngressQueueKey;
use revision::{
    PersistedAdmissionCursors, authoritative_revision_count, boundaries_before_attempts,
};
use settlement::{PendingBoundaryRandomDraw, boundary_has_event_ingress, boundary_system_due};
use state::{
    CommitmentDomains, JournalCommitmentRoots, RuntimeCommitmentCache,
    RuntimeCommitmentRootUpdates, RuntimeCounters, RuntimeCurrentState,
    RuntimeDomainCommitmentRoots, RuntimeEvidence, RuntimeMetadata, RuntimeScheduler, RuntimeState,
};
use transactions::{
    BoundaryTransactionCheckpoint, ClockTransactionCheckpoint, CommandTransactionCheckpoint,
    IngressTransactionCheckpoint, RejectionTransactionCheckpoint,
    ScheduledBatchTransactionCheckpoint,
};
use validation::{
    RuntimeValidationContext, claim_counter, core_world_entity_exists,
    has_unqueued_command_history, proposal_entity_exists, proposal_entity_identity_exists,
    runtime_current_entity_exists, runtime_entity_exists,
    runtime_entity_exists_with_record_overlay, runtime_entity_identity_exists,
    runtime_has_unqueued_command_history, snapshot_entity_exists_in_history,
    validate_directives_with_context, validate_domain_dependents_with_records,
    validate_run_configuration_entities, validate_runtime_cause,
    validate_runtime_domain_dependents, validate_snapshot,
};

pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Format 8 binds every decision attempt to its complete ingress request and
/// activates content-addressed state-page persistence. Older
/// snapshots, journals, and sub-contract versions are rejected before any
/// mutable runtime state is constructed.
pub const SNAPSHOT_FORMAT_VERSION: u32 = 8;
/// Version of the authoritative revision commitment.
pub const STATE_REVISION_FORMAT_VERSION: u32 = 3;
/// Version of persisted monotonic boundary-admission cursors.
pub const ADMISSION_CURSOR_FORMAT_VERSION: u32 = 3;
/// Version of the domain-separated checkpoint commitment contract.
pub const COMMITMENT_FORMAT_VERSION: u32 = 4;
/// Maximum nested depth of the compatibility synchronous event-reactor path.
///
/// New plugin mechanics should use phased boundary systems instead of relying
/// on recursively emitted immediate events.
pub const MAX_SYNCHRONOUS_REACTION_DEPTH: usize = 32;
const CORE_STATE_NAMESPACE: &str = "canwu.core";
const MAX_DOMAIN_RECORD_QUERY_LIMIT: usize = 10_000;
const GENESIS_BOUNDARY_HASH: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";

fn fresh_authority_root_seed(root_seed: u64, run_manifest_hash: &str) -> Result<u64, CanwuError> {
    let digest =
        hashing::canonical_hash("canwu.authority-root.v1", &(root_seed, run_manifest_hash))?;
    u64::from_str_radix(&digest[..16], 16)
        .map_err(|error| {
            CanwuError::new(
                ErrorCode::InvalidAuthority,
                format!("invalid authority root derivation: {error}"),
            )
        })
        .and_then(|seed| {
            (seed != 0).then_some(seed).ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "the derived authority root cannot be zero",
                )
            })
        })
}

fn reject_unknown_current_fields(
    input: &Value,
    encoded: &Value,
    path: &str,
) -> Result<(), CanwuError> {
    match (input, encoded) {
        (Value::Object(input), Value::Object(encoded)) => {
            for (key, value) in input {
                let field_path = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                let Some(expected) = encoded.get(key) else {
                    return Err(invalid_snapshot_error(format!(
                        "format 8 wire contains unknown field `{field_path}`"
                    )));
                };
                reject_unknown_current_fields(value, expected, &field_path)?;
            }
        }
        (Value::Array(input), Value::Array(encoded)) => {
            if input.len() != encoded.len() {
                return Err(invalid_snapshot_error(format!(
                    "format 8 wire array `{path}` changed shape during decoding"
                )));
            }
            for (index, (value, expected)) in input.iter().zip(encoded).enumerate() {
                reject_unknown_current_fields(value, expected, &format!("{path}[{index}]"))?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn deserialize_current_json<T>(json: &str, label: &str) -> Result<T, CanwuError>
where
    T: for<'de> Deserialize<'de> + Serialize,
{
    let input: Value = serde_json::from_str(json).map_err(|error| {
        invalid_snapshot_error(format!("could not parse format 8 {label}: {error}"))
    })?;
    deserialize_current_value(&input, label)
}

fn deserialize_current_value<T>(input: &Value, label: &str) -> Result<T, CanwuError>
where
    T: for<'de> Deserialize<'de> + Serialize,
{
    let decoded: T = serde_json::from_value(input.clone()).map_err(|error| {
        invalid_snapshot_error(format!("could not deserialize format 8 {label}: {error}"))
    })?;
    let encoded = serde_json::to_value(&decoded).map_err(|error| {
        invalid_snapshot_error(format!("could not re-encode format 8 {label}: {error}"))
    })?;
    reject_unknown_current_fields(input, &encoded, "")?;
    Ok(decoded)
}

fn deserialize_current_snapshot_json(json: &str) -> Result<SimulationSnapshot, CanwuError> {
    let input: Value = serde_json::from_str(json).map_err(|error| {
        invalid_snapshot_error(format!("could not parse format 8 snapshot: {error}"))
    })?;
    let engine_version = input.get("engine_version").and_then(Value::as_str);
    let snapshot_format_version = input.get("snapshot_format_version").and_then(Value::as_u64);
    if engine_version != Some(ENGINE_VERSION)
        || snapshot_format_version != Some(u64::from(SNAPSHOT_FORMAT_VERSION))
    {
        return Err(CanwuError::new(
            ErrorCode::UnsupportedSnapshotVersion,
            format!(
                "the JSON snapshot loader accepts only engine {ENGINE_VERSION} format {SNAPSHOT_FORMAT_VERSION}; pre-8 formats are not supported"
            ),
        ));
    }
    deserialize_current_value(&input, "snapshot")
}

fn validate_current_snapshot_contract(snapshot: &SimulationSnapshot) -> Result<(), CanwuError> {
    if snapshot.commitment_format_version != COMMITMENT_FORMAT_VERSION
        || snapshot.revision_format_version != STATE_REVISION_FORMAT_VERSION
        || snapshot.replay_revision_format_version != STATE_REVISION_FORMAT_VERSION
        || snapshot.admission_cursor_format_version != ADMISSION_CURSOR_FORMAT_VERSION
        || snapshot.authority_root_seed == 0
        || snapshot.legacy_rng.is_some()
    {
        return Err(invalid_snapshot_error(
            "format 8 snapshots must use the current commitment, revision, admission, and authority contracts",
        ));
    }
    let Some(run_manifest @ RunManifest::Declared { .. }) = snapshot.run_manifest.as_ref() else {
        return Err(invalid_snapshot_error(
            "format 8 snapshots require a declared run manifest",
        ));
    };
    let Some(initial_scenario) = snapshot.initial_scenario.as_ref() else {
        return Err(invalid_snapshot_error(
            "format 8 snapshots must retain their canonical initial scenario",
        ));
    };
    if matches!(
        snapshot.run_configuration,
        Some(
            RunConfigurationSnapshot::LegacyUnspecified | RunConfigurationSnapshot::ManifestOnlyV1
        )
    ) {
        return Err(invalid_snapshot_error(
            "format 8 snapshots cannot use legacy or manifest-only run configuration provenance",
        ));
    }
    manifest::validate(run_manifest, Some(initial_scenario))?;
    let expected_manifest_hash = manifest::hash(run_manifest)?;
    if snapshot.run_manifest_hash != expected_manifest_hash {
        return Err(invalid_snapshot_error(
            "format 8 snapshot run manifest hash is inconsistent",
        ));
    }
    let run_configuration = snapshot
        .run_configuration
        .as_ref()
        .ok_or_else(|| invalid_snapshot_error("format 8 snapshots require run configuration"))?;
    let (_, authority_manifest_hash) =
        authoritative_run_identity(run_manifest, &expected_manifest_hash, run_configuration)?;
    let expected_authority_root =
        fresh_authority_root_seed(snapshot.root_seed, &authority_manifest_hash)?;
    if snapshot.authority_root_seed != expected_authority_root {
        return Err(invalid_snapshot_error(
            "format 8 snapshot authority root is not bound to its run identity",
        ));
    }
    Ok(())
}

/// One deterministic trusted-host page of records from an authoritative read cut.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct DomainRecordPage {
    pub kind: DomainRecordKind,
    pub revision: u64,
    pub records: Vec<DomainRecord>,
    pub next: Option<DomainRecordRef>,
}

fn validate_domain_record_page_request(
    kind: &DomainRecordKind,
    after: Option<&DomainRecordRef>,
    limit: usize,
) -> Result<(), CanwuError> {
    if limit == 0 || limit > MAX_DOMAIN_RECORD_QUERY_LIMIT {
        return Err(CanwuError::new(
            ErrorCode::ValueOutOfRange,
            format!(
                "domain-record query limit must be between 1 and {MAX_DOMAIN_RECORD_QUERY_LIMIT}"
            ),
        ));
    }
    if after.is_some_and(|cursor| cursor.kind != *kind) {
        return Err(CanwuError::new(
            ErrorCode::InvalidPayload,
            "domain-record page cursor has the wrong kind",
        ));
    }
    Ok(())
}

fn domain_record_candidates(
    records: &impl records::DomainRecordRead,
    kind: &DomainRecordKind,
    after: Option<&DomainRecordRef>,
    limit: usize,
) -> BTreeMap<DomainRecordRef, DomainRecord> {
    let (lower, excluded) = after.map_or_else(
        || {
            (
                DomainRecordRef {
                    kind: kind.clone(),
                    id: String::new(),
                },
                false,
            )
        },
        |cursor| (cursor.clone(), true),
    );
    records
        .range_from(lower, excluded)
        .take_while(|(reference, _)| reference.kind == *kind)
        .take(limit)
        .map(|(reference, record)| (reference.clone(), record.clone()))
        .collect()
}

fn retained_domain_record_version(
    state: &RuntimeState,
    reference: &DomainRecordVersionRef,
) -> Option<DomainRecord> {
    if reference.version == 0 {
        return None;
    }
    let record = match reference.established_by {
        DomainRecordVersionSource::InitialScenario => state
            .metadata
            .initial_scenario
            .as_ref()?
            .domain_records
            .get(
                *state
                    .metadata
                    .initial_domain_record_indexes
                    .get(&reference.record)?,
            )
            .filter(|record| record.version == reference.version),
        DomainRecordVersionSource::BoundaryChange {
            boundary,
            change_index,
        } => state
            .evidence
            .retained_boundary(boundary)?
            .record_changes
            .get(usize::try_from(change_index).ok()?)
            .map(|change| &change.current)
            .filter(|record| {
                record.reference == reference.record && record.version == reference.version
            }),
    }?;
    Some(record.clone())
}

fn retained_evidence_time(state: &RuntimeState, reference: &EvidenceRef) -> Option<SimTime> {
    match reference {
        EvidenceRef::Command(id) => state
            .evidence
            .retained_command(*id)
            .map(|record| record.accepted_at),
        EvidenceRef::CommandAttempt(id) => state
            .evidence
            .retained_command_attempt(*id)
            .map(|record| record.at),
        EvidenceRef::Event(id) => state
            .evidence
            .retained_event(*id)
            .map(|record| record.timestamp),
        EvidenceRef::Ingress(id) => state
            .evidence
            .retained_ingress(*id)
            .map(|record| record.issued_at),
        EvidenceRef::Boundary(id) => state
            .evidence
            .retained_boundary(*id)
            .map(|record| record.at),
        EvidenceRef::RandomDraw(id) => state
            .evidence
            .retained_random_draw(*id)
            .map(|record| record.at),
        EvidenceRef::DomainRecordVersion(version) => match version.established_by {
            DomainRecordVersionSource::InitialScenario => {
                retained_domain_record_version(state, version).map(|_| state.scheduler.initial_time)
            }
            DomainRecordVersionSource::BoundaryChange { boundary, .. } => {
                retained_domain_record_version(state, version).and_then(|_| {
                    state
                        .evidence
                        .retained_boundary(boundary)
                        .map(|record| record.at)
                })
            }
        },
    }
}

pub use error::{CanwuError, ErrorCode};
pub use hashing::CommitmentRoots;

use ingress::CommandAdmission;
pub use ingress::{
    Command, CommandAttemptOutcome, CommandAttemptRecord, CommandAuthority, CommandContext,
    CommandEnvelope, CommandIngress, CommandOutcome, CommandReceipt, CommandRecord,
    CommandRejection, CommandRequest, DecisionOrigin, Issuer,
};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[repr(u8)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryPhase {
    EventIngress = 1,
    BoundarySnapshot = 2,
    DerivedFieldSolve = 3,
    PerceptionAndAttentionRefresh = 4,
    DecisionAndAcceptedEffectIntake = 5,
    ReservationAndAllocation = 6,
    DomainDeltaProposal = 7,
    InvariantValidation = 8,
    AtomicDomainCommit = 9,
    HistoricalCandidateEvaluation = 10,
    ConditionalTransitionCommit = 11,
    StrategicAggregation = 12,
    PerspectiveAndReportMaterialization = 13,
    SaveReplayAndDiagnosticHashing = 14,
}

impl BoundaryPhase {
    pub const ALL: [Self; 14] = [
        Self::EventIngress,
        Self::BoundarySnapshot,
        Self::DerivedFieldSolve,
        Self::PerceptionAndAttentionRefresh,
        Self::DecisionAndAcceptedEffectIntake,
        Self::ReservationAndAllocation,
        Self::DomainDeltaProposal,
        Self::InvariantValidation,
        Self::AtomicDomainCommit,
        Self::HistoricalCandidateEvaluation,
        Self::ConditionalTransitionCommit,
        Self::StrategicAggregation,
        Self::PerspectiveAndReportMaterialization,
        Self::SaveReplayAndDiagnosticHashing,
    ];
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemCadence {
    EventDriven,
    SubDaily,
    Daily,
    Monthly,
    Seasonal,
    Annual,
    EraScheduled,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StateVisibility {
    SameBoundary,
    NextBoundary,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct StateKey {
    pub namespace: String,
    pub name: String,
}

impl StateKey {
    #[must_use]
    pub fn new(namespace: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            namespace: namespace.into(),
            name: name.into(),
        }
    }

    #[must_use]
    pub fn core_people() -> Self {
        Self::new(CORE_STATE_NAMESPACE, "people")
    }

    #[must_use]
    pub fn core_governments() -> Self {
        Self::new(CORE_STATE_NAMESPACE, "governments")
    }

    #[must_use]
    pub fn core_territories() -> Self {
        Self::new(CORE_STATE_NAMESPACE, "territories")
    }

    #[must_use]
    pub fn core_routes() -> Self {
        Self::new(CORE_STATE_NAMESPACE, "routes")
    }

    #[must_use]
    pub fn core_armies() -> Self {
        Self::new(CORE_STATE_NAMESPACE, "armies")
    }

    #[must_use]
    pub fn core_knowledge() -> Self {
        Self::new(CORE_STATE_NAMESPACE, "knowledge")
    }

    #[must_use]
    pub fn core_commands() -> Self {
        Self::new(CORE_STATE_NAMESPACE, "commands")
    }

    #[must_use]
    pub fn core_events() -> Self {
        Self::new(CORE_STATE_NAMESPACE, "events")
    }

    #[must_use]
    pub fn core_ingress() -> Self {
        Self::new(CORE_STATE_NAMESPACE, "ingress")
    }

    /// Current decision controllers, tickets, and request outcomes.
    #[must_use]
    pub fn core_decisions() -> Self {
        Self::new(CORE_STATE_NAMESPACE, "decisions")
    }

    /// Administrative read access to current plugin-owned domain records.
    #[must_use]
    pub fn core_domain_records() -> Self {
        Self::new(CORE_STATE_NAMESPACE, "domain_records")
    }

    /// Retained or archived boundary and random-draw evidence identities.
    #[must_use]
    pub fn core_evidence() -> Self {
        Self::new(CORE_STATE_NAMESPACE, "evidence")
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SystemContract {
    pub name: String,
    pub phase: BoundaryPhase,
    pub cadence: SystemCadence,
    pub reads: Vec<StateKey>,
    pub writes: Vec<StateKey>,
    pub visibility: StateVisibility,
}

impl SystemContract {
    #[must_use]
    pub fn event_driven(name: impl Into<String>, phase: BoundaryPhase) -> Self {
        Self {
            name: name.into(),
            phase,
            cadence: SystemCadence::EventDriven,
            reads: Vec::new(),
            writes: Vec::new(),
            visibility: StateVisibility::SameBoundary,
        }
    }
}

pub use scenario::{DemoIds, Scenario, demo_scenario};
use scenario::{
    base_schema, canonicalize_scenario, require_plugin_aware_initial_records, validate_scenario,
    validate_scenario_state, validate_strict_id_order,
};

use plugins::PluginComponentKey;
pub use plugins::{
    PLUGIN_DESCRIPTOR_FORMAT_VERSION, PayloadProperty, PayloadSchema, PayloadValueType,
    PluginActionDescriptor, PluginCommandHandler, PluginComponentRecord, PluginDescriptor,
    PluginRegistrar, PluginRegistry, SimulationPlugin, SimulationSystemHandler, SystemDirective,
};

pub use view::SimulationView;
use view::SimulationViewState;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum BoundaryWriteStage {
    Ordinary,
    Transition,
    Aggregation,
    Perspective,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum DomainRecordCommitStage {
    Ordinary,
    Transition,
    Aggregation,
    Perspective,
    Deferred,
}

impl DomainRecordCommitStage {
    const ALL: [Self; 5] = [
        Self::Ordinary,
        Self::Transition,
        Self::Aggregation,
        Self::Perspective,
        Self::Deferred,
    ];

    const fn ordinal(self) -> u8 {
        match self {
            Self::Ordinary => 1,
            Self::Transition => 2,
            Self::Aggregation => 3,
            Self::Perspective => 4,
            Self::Deferred => 5,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct DomainHistoryCut {
    boundary: usize,
    stage: u8,
}

impl DomainHistoryCut {
    const GENESIS: Self = Self {
        boundary: 0,
        stage: 0,
    };

    const fn after_boundaries(boundary: usize) -> Self {
        Self { boundary, stage: 5 }
    }

    const fn after_stage(boundary: usize, stage: DomainRecordCommitStage) -> Self {
        Self {
            boundary,
            stage: stage.ordinal(),
        }
    }
}

#[derive(Clone, Debug, Default)]
struct BoundaryDomainEntityCuts {
    changes: BTreeMap<DomainRecordRef, Vec<DomainEntityStageChange>>,
}

impl BoundaryDomainEntityCuts {
    fn record(&mut self, stage: DomainRecordCommitStage, change: &DomainRecordChange) {
        let previous_live = change
            .previous
            .as_ref()
            .is_some_and(domain_record_is_live_entity);
        let current_live = domain_record_is_live_entity(&change.current);
        if previous_live != current_live {
            self.changes
                .entry(change.current.reference.clone())
                .or_default()
                .push(DomainEntityStageChange {
                    stage,
                    plugin: change.plugin.clone(),
                    system: change.system.clone(),
                    previous_live,
                    current_live,
                });
        }
    }

    fn is_live(
        &self,
        final_records: &BTreeMap<DomainRecordRef, DomainRecord>,
        reference: &DomainRecordRef,
        stage: Option<DomainRecordCommitStage>,
    ) -> bool {
        let mut live = final_records
            .get(reference)
            .is_some_and(domain_record_is_live_entity);
        if let Some(changes) = self.changes.get(reference) {
            for change in changes.iter().rev() {
                if stage.is_some_and(|stage| change.stage <= stage) {
                    break;
                }
                live = change.previous_live;
            }
        }
        live
    }

    fn is_live_for_proposal(
        &self,
        final_records: &BTreeMap<DomainRecordRef, DomainRecord>,
        reference: &DomainRecordRef,
        phase: BoundaryPhase,
        commit_stage: DomainRecordCommitStage,
        plugin: &str,
        system: &str,
    ) -> bool {
        let visible_after = match phase {
            BoundaryPhase::DomainDeltaProposal => None,
            BoundaryPhase::HistoricalCandidateEvaluation => Some(DomainRecordCommitStage::Ordinary),
            BoundaryPhase::StrategicAggregation => Some(DomainRecordCommitStage::Transition),
            BoundaryPhase::PerspectiveAndReportMaterialization => {
                Some(DomainRecordCommitStage::Aggregation)
            }
            BoundaryPhase::EventIngress
            | BoundaryPhase::BoundarySnapshot
            | BoundaryPhase::DerivedFieldSolve
            | BoundaryPhase::PerceptionAndAttentionRefresh
            | BoundaryPhase::DecisionAndAcceptedEffectIntake
            | BoundaryPhase::ReservationAndAllocation
            | BoundaryPhase::InvariantValidation
            | BoundaryPhase::AtomicDomainCommit
            | BoundaryPhase::ConditionalTransitionCommit
            | BoundaryPhase::SaveReplayAndDiagnosticHashing => return false,
        };
        let before_proposal = self.is_live(final_records, reference, visible_after);
        self.changes
            .get(reference)
            .and_then(|changes| {
                changes.iter().find(|change| {
                    change.stage == commit_stage
                        && change.plugin == plugin
                        && change.system == system
                })
            })
            .map_or(before_proposal, |change| change.current_live)
    }

    fn identity_exists_for_proposal(
        &self,
        final_records: &BTreeMap<DomainRecordRef, DomainRecord>,
        reference: &DomainRecordRef,
        phase: BoundaryPhase,
        commit_stage: DomainRecordCommitStage,
        plugin: &str,
        system: &str,
    ) -> bool {
        if !final_records.contains_key(reference) {
            return false;
        }
        let visible_after = match phase {
            BoundaryPhase::DomainDeltaProposal => None,
            BoundaryPhase::HistoricalCandidateEvaluation => Some(DomainRecordCommitStage::Ordinary),
            BoundaryPhase::StrategicAggregation => Some(DomainRecordCommitStage::Transition),
            BoundaryPhase::PerspectiveAndReportMaterialization => {
                Some(DomainRecordCommitStage::Aggregation)
            }
            BoundaryPhase::EventIngress
            | BoundaryPhase::BoundarySnapshot
            | BoundaryPhase::DerivedFieldSolve
            | BoundaryPhase::PerceptionAndAttentionRefresh
            | BoundaryPhase::DecisionAndAcceptedEffectIntake
            | BoundaryPhase::ReservationAndAllocation
            | BoundaryPhase::InvariantValidation
            | BoundaryPhase::AtomicDomainCommit
            | BoundaryPhase::ConditionalTransitionCommit
            | BoundaryPhase::SaveReplayAndDiagnosticHashing => return false,
        };
        self.changes
            .get(reference)
            .and_then(|changes| {
                changes
                    .iter()
                    .find(|change| !change.previous_live && change.current_live)
            })
            .is_none_or(|creation| {
                visible_after.is_some_and(|stage| creation.stage <= stage)
                    || (creation.stage == commit_stage
                        && creation.plugin == plugin
                        && creation.system == system)
            })
    }
}

#[derive(Clone, Debug)]
struct DomainEntityStageChange {
    stage: DomainRecordCommitStage,
    plugin: String,
    system: String,
    previous_live: bool,
    current_live: bool,
}

#[derive(Clone, Debug)]
struct DomainRecordHistory {
    lifetimes: BTreeMap<DomainRecordRef, DomainEntityLifetime>,
}

impl DomainRecordHistory {
    fn from_initial_records(records: &BTreeMap<DomainRecordRef, DomainRecord>) -> Self {
        let lifetimes = records
            .values()
            .filter(|record| record.class == DomainRecordClass::Entity)
            .map(|record| {
                (
                    record.reference.clone(),
                    DomainEntityLifetime {
                        created_at: DomainHistoryCut::GENESIS,
                        deleted_at: record.is_deleted().then_some(DomainHistoryCut::GENESIS),
                    },
                )
            })
            .collect();
        Self { lifetimes }
    }

    fn apply_boundary(
        &mut self,
        boundary: usize,
        cuts: &BoundaryDomainEntityCuts,
    ) -> Result<(), CanwuError> {
        for (reference, changes) in &cuts.changes {
            for change in changes {
                let cut = DomainHistoryCut::after_stage(boundary, change.stage);
                match (change.previous_live, change.current_live) {
                    (false, true) => {
                        if self
                            .lifetimes
                            .insert(
                                reference.clone(),
                                DomainEntityLifetime {
                                    created_at: cut,
                                    deleted_at: None,
                                },
                            )
                            .is_some()
                        {
                            return invalid_snapshot(
                                "domain entity history recreates an existing stable identity",
                            );
                        }
                    }
                    (true, false) => {
                        let Some(lifetime) = self.lifetimes.get_mut(reference) else {
                            return invalid_snapshot(
                                "domain entity history deletes an identity before creation",
                            );
                        };
                        if lifetime.deleted_at.replace(cut).is_some() {
                            return invalid_snapshot(
                                "domain entity history deletes the same identity more than once",
                            );
                        }
                    }
                    (false, false) | (true, true) => {}
                }
            }
        }
        Ok(())
    }

    fn is_live(&self, reference: &DomainRecordRef, cut: DomainHistoryCut) -> bool {
        self.lifetimes.get(reference).is_some_and(|lifetime| {
            lifetime.created_at <= cut && lifetime.deleted_at.is_none_or(|deleted| cut < deleted)
        })
    }

    fn exists(&self, reference: &DomainRecordRef, cut: DomainHistoryCut) -> bool {
        self.lifetimes
            .get(reference)
            .is_some_and(|lifetime| lifetime.created_at <= cut)
    }

    fn before_time(snapshot: &SimulationSnapshot, at: SimTime) -> DomainHistoryCut {
        let count = snapshot
            .boundaries
            .partition_point(|boundary| boundary.at < at);
        DomainHistoryCut::after_boundaries(count)
    }
}

#[derive(Clone, Copy, Debug)]
struct DomainEntityLifetime {
    created_at: DomainHistoryCut,
    deleted_at: Option<DomainHistoryCut>,
}

fn domain_record_is_live_entity(record: &DomainRecord) -> bool {
    record.class == DomainRecordClass::Entity && !record.is_deleted()
}

const fn boundary_write_stage(phase: BoundaryPhase) -> Option<BoundaryWriteStage> {
    match phase {
        BoundaryPhase::DomainDeltaProposal => Some(BoundaryWriteStage::Ordinary),
        BoundaryPhase::HistoricalCandidateEvaluation => Some(BoundaryWriteStage::Transition),
        BoundaryPhase::StrategicAggregation => Some(BoundaryWriteStage::Aggregation),
        BoundaryPhase::PerspectiveAndReportMaterialization => Some(BoundaryWriteStage::Perspective),
        BoundaryPhase::EventIngress
        | BoundaryPhase::BoundarySnapshot
        | BoundaryPhase::DerivedFieldSolve
        | BoundaryPhase::PerceptionAndAttentionRefresh
        | BoundaryPhase::DecisionAndAcceptedEffectIntake
        | BoundaryPhase::ReservationAndAllocation
        | BoundaryPhase::InvariantValidation
        | BoundaryPhase::AtomicDomainCommit
        | BoundaryPhase::ConditionalTransitionCommit
        | BoundaryPhase::SaveReplayAndDiagnosticHashing => None,
    }
}

const fn domain_record_commit_stage(
    phase: BoundaryPhase,
    visibility: StateVisibility,
) -> Option<DomainRecordCommitStage> {
    let stage = match phase {
        BoundaryPhase::DomainDeltaProposal => DomainRecordCommitStage::Ordinary,
        BoundaryPhase::HistoricalCandidateEvaluation => DomainRecordCommitStage::Transition,
        BoundaryPhase::StrategicAggregation => DomainRecordCommitStage::Aggregation,
        BoundaryPhase::PerspectiveAndReportMaterialization => DomainRecordCommitStage::Perspective,
        BoundaryPhase::EventIngress
        | BoundaryPhase::BoundarySnapshot
        | BoundaryPhase::DerivedFieldSolve
        | BoundaryPhase::PerceptionAndAttentionRefresh
        | BoundaryPhase::DecisionAndAcceptedEffectIntake
        | BoundaryPhase::ReservationAndAllocation
        | BoundaryPhase::InvariantValidation
        | BoundaryPhase::AtomicDomainCommit
        | BoundaryPhase::ConditionalTransitionCommit
        | BoundaryPhase::SaveReplayAndDiagnosticHashing => return None,
    };
    Some(match visibility {
        StateVisibility::SameBoundary => stage,
        StateVisibility::NextBoundary => DomainRecordCommitStage::Deferred,
    })
}

fn validate_type_schema(schema: &TypeSchema) -> Result<(), CanwuError> {
    if schema.type_name.trim().is_empty() || schema.type_name != schema.type_name.trim() {
        return Err(CanwuError::new(
            ErrorCode::InvalidPluginRegistration,
            "plugin schema type name must be non-empty and have no surrounding whitespace",
        ));
    }
    let mut field_names = BTreeSet::new();
    for field in &schema.fields {
        if field.name.trim().is_empty()
            || field.name != field.name.trim()
            || field.value_type.trim().is_empty()
            || field.value_type != field.value_type.trim()
            || field
                .reference_type
                .as_ref()
                .is_some_and(|value| value.trim().is_empty() || value != value.trim())
            || !field_names.insert(&field.name)
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                format!("schema {} contains an invalid field", schema.type_name),
            ));
        }
    }
    Ok(())
}

use scheduling::{ScheduleKey, ScheduledAction, ScheduledRecord};

const fn one_u64() -> u64 {
    1
}

#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_zero_u64(value: &u64) -> bool {
    *value == 0
}

#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_one_u64(value: &u64) -> bool {
    *value == 1
}

fn command_attempt_slice_is_empty(value: &&[CommandAttemptRecord]) -> bool {
    value.is_empty()
}

fn command_attempt_id_slice_is_empty(value: &&[CommandAttemptId]) -> bool {
    value.is_empty()
}

fn domain_record_slice_is_empty(value: &&[DomainRecord]) -> bool {
    value.is_empty()
}

fn domain_record_change_slice_is_empty(value: &&[DomainRecordChange]) -> bool {
    value.is_empty()
}

fn ingress_record_slice_is_empty(value: &&[IngressRecord]) -> bool {
    value.is_empty()
}

fn maintenance_change_slice_is_empty(value: &&[MaintenanceChangeRecord]) -> bool {
    value.is_empty()
}

const BOUNDARY_STATE_HASH_V1_PREFIX: &str = "v1:";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BoundaryStateHashFormat {
    LegacyV0,
    CommitmentsV1,
}

fn boundary_state_hash_format(value: Option<&str>) -> Result<BoundaryStateHashFormat, CanwuError> {
    match value {
        Some(value) if value.starts_with(BOUNDARY_STATE_HASH_V1_PREFIX) => {
            let hash = &value[BOUNDARY_STATE_HASH_V1_PREFIX.len()..];
            if !is_canonical_hash(hash) {
                return invalid_snapshot("boundary state commitment v1 is not canonical");
            }
            Ok(BoundaryStateHashFormat::CommitmentsV1)
        }
        Some(value) if is_canonical_hash(value) => Ok(BoundaryStateHashFormat::LegacyV0),
        Some(_) => invalid_snapshot("boundary state commitment format is unsupported"),
        None => Ok(BoundaryStateHashFormat::LegacyV0),
    }
}

pub struct Simulation {
    state: RuntimeState,
    schema: SchemaRegistry,
    plugins: PluginRegistry,
    sync_reaction_depth: usize,
}

impl Simulation {
    /// Creates a simulation after validating that scenario references are sound.
    pub fn new(seed: u64, scenario: Scenario) -> Result<Self, CanwuError> {
        require_plugin_aware_initial_records(&scenario)?;
        let run_manifest = RunManifest::for_scenario("canwu.inline", "scenario", "1", &scenario)?;
        Self::new_with_configuration_snapshot(
            seed,
            scenario,
            run_manifest,
            RunConfigurationSnapshot::CompatibilityV1,
        )
    }

    /// Creates a simulation and activates the plugins required by initial
    /// application-defined records before returning a snapshot-capable runtime.
    pub fn new_with_plugins(
        seed: u64,
        scenario: Scenario,
        plugins: &[&dyn SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        let run_manifest = RunManifest::for_scenario("canwu.inline", "scenario", "1", &scenario)?;
        Self::new_with_manifest_and_plugins(seed, scenario, run_manifest, plugins)
    }

    /// Creates a simulation with an exact, persisted run environment identity.
    pub fn new_with_manifest(
        seed: u64,
        scenario: Scenario,
        run_manifest: RunManifest,
    ) -> Result<Self, CanwuError> {
        require_plugin_aware_initial_records(&scenario)?;
        Self::new_with_configuration_snapshot(
            seed,
            scenario,
            run_manifest,
            RunConfigurationSnapshot::CompatibilityV1,
        )
    }

    /// Creates a manifested run and activates all initial domain packages before
    /// the runtime can be observed or snapshotted.
    pub fn new_with_manifest_and_plugins(
        seed: u64,
        scenario: Scenario,
        run_manifest: RunManifest,
        plugins: &[&dyn SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        let simulation = Self::new_with_configuration_snapshot(
            seed,
            scenario,
            run_manifest,
            RunConfigurationSnapshot::CompatibilityV1,
        )?;
        Self::activate_initial_plugins(simulation, plugins)
    }

    /// Creates a run whose six policy dimensions are persisted and bound to
    /// the run-configuration artifact in `run_manifest`.
    pub fn new_with_run_configuration(
        seed: u64,
        scenario: Scenario,
        run_manifest: RunManifest,
        mut run_configuration: RunConfiguration,
    ) -> Result<Self, CanwuError> {
        require_plugin_aware_initial_records(&scenario)?;
        run_configuration.canonicalize();
        Self::new_with_configuration_snapshot(
            seed,
            scenario,
            run_manifest,
            RunConfigurationSnapshot::Declared(run_configuration),
        )
    }

    /// Creates a declared-policy run and activates all initial domain packages
    /// before the runtime can be observed or snapshotted.
    pub fn new_with_run_configuration_and_plugins(
        seed: u64,
        scenario: Scenario,
        run_manifest: RunManifest,
        mut run_configuration: RunConfiguration,
        plugins: &[&dyn SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        run_configuration.canonicalize();
        let simulation = Self::new_with_configuration_snapshot(
            seed,
            scenario,
            run_manifest,
            RunConfigurationSnapshot::Declared(run_configuration),
        )?;
        Self::activate_initial_plugins(simulation, plugins)
    }

    fn activate_initial_plugins(
        mut simulation: Self,
        plugins: &[&dyn SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        for plugin in plugins {
            simulation.register_plugin(*plugin)?;
        }
        simulation.ensure_runtime_ready()?;
        Ok(simulation)
    }

    fn new_with_configuration_snapshot(
        seed: u64,
        mut scenario: Scenario,
        mut run_manifest: RunManifest,
        run_configuration: RunConfigurationSnapshot,
    ) -> Result<Self, CanwuError> {
        canonicalize_scenario(&mut scenario);
        validate_scenario(&scenario)?;
        manifest::canonicalize(&mut run_manifest);
        manifest::validate(&run_manifest, Some(&scenario))?;
        manifest::validate_run_configuration(&run_manifest, &run_configuration)?;
        validate_run_configuration_entities(
            &run_configuration,
            &scenario.entities,
            &scenario.world,
            &scenario.domain_records,
        )?;
        let run_manifest_hash = manifest::hash(&run_manifest)?;
        if scenario
            .world
            .armies
            .iter()
            .any(|army| army.transit.is_some())
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidSnapshot,
                "initial scenarios cannot contain transit without admitted command/event/queue evidence",
            ));
        }
        if scenario
            .world
            .people
            .iter()
            .any(|person| person.transit.is_some())
            || scenario
                .world
                .letters
                .iter()
                .any(|letter| letter.status == LetterStatus::InTransit)
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidSnapshot,
                "initial scenarios cannot contain person or letter transit without admitted command/event/queue evidence",
            ));
        }
        let schema = base_schema();
        let plugins = PluginRegistry::default();
        let (_, authority_manifest_hash) =
            authoritative_run_identity(&run_manifest, &run_manifest_hash, &run_configuration)?;
        let authority_root_seed = fresh_authority_root_seed(seed, &authority_manifest_hash)?;
        let core_stream = RandomStreamState::initial(seed, random::core_report_delay_stream());
        let initial_scenario = Some(scenario.clone());
        let initial_domain_record_indexes = initial_scenario
            .as_ref()
            .map(|scenario| {
                scenario
                    .domain_records
                    .iter()
                    .enumerate()
                    .map(|(index, record)| (record.reference.clone(), index))
                    .collect()
            })
            .unwrap_or_default();
        let mut simulation = Self {
            state: RuntimeState {
                current: RuntimeCurrentState {
                    entities: scenario.entities.into_iter().collect(),
                    people: scenario
                        .world
                        .people
                        .into_iter()
                        .map(|value| (value.id, value))
                        .collect(),
                    letters: scenario
                        .world
                        .letters
                        .into_iter()
                        .map(|value| (value.id, value))
                        .collect(),
                    governments: scenario
                        .world
                        .governments
                        .into_iter()
                        .map(|value| (value.id, value))
                        .collect(),
                    territories: scenario
                        .world
                        .territories
                        .into_iter()
                        .map(|value| (value.id, value))
                        .collect(),
                    routes: scenario
                        .world
                        .routes
                        .into_iter()
                        .map(|value| (value.id, value))
                        .collect(),
                    armies: scenario
                        .world
                        .armies
                        .into_iter()
                        .map(|value| (value.id, value))
                        .collect(),
                    knowledge: scenario.knowledge,
                    plugin_components: BTreeMap::new(),
                    domain_records: PersistentDomainRecordStore::from_records(
                        scenario
                            .domain_records
                            .into_iter()
                            .map(|record| (record.reference.clone(), record))
                            .collect(),
                    )?,
                    decisions: DecisionState::default(),
                    root_seed: seed,
                    authority_root_seed,
                    random_streams: BTreeMap::from([(core_stream.key.clone(), core_stream)]),
                },
                scheduler: RuntimeScheduler {
                    initial_time: scenario.start_time,
                    now: scenario.start_time,
                    actions: BTreeMap::new(),
                    pending_ingress: BTreeSet::new(),
                },
                counters: RuntimeCounters {
                    next_event_id: 1,
                    next_command_id: 1,
                    next_command_attempt_id: 1,
                    next_ingress_id: 1,
                    next_boundary_id: 1,
                    next_random_draw_id: 1,
                    next_knowledge_record_id: 1,
                    next_schedule_sequence: 1,
                    next_correlation_id: 1,
                    next_decision_trace_id: 1,
                    state_revision: 0,
                    admitted_attempt_count: 0,
                    admitted_command_count: 0,
                    admitted_event_count: 0,
                },
                metadata: RuntimeMetadata {
                    initial_scenario,
                    initial_domain_record_indexes,
                    run_manifest,
                    run_manifest_hash,
                    run_configuration,
                    checkpoint_hash: String::new(),
                    commitment_format_version: COMMITMENT_FORMAT_VERSION,
                    commitment_roots: None,
                    commitment_cache: None,
                    plugin_registration_closed: false,
                    replay_revision_format_version: STATE_REVISION_FORMAT_VERSION,
                },
                evidence: RuntimeEvidence {
                    archived: EvidenceCursor::default(),
                    archived_boundary_head: None,
                    archived_legacy_commands: false,
                    archived_tracked_attempts: false,
                    archived_unqueued_command_history: false,
                    archived_command_requests: BTreeMap::new(),
                    archived_ingress_requests: BTreeMap::new(),
                    archived_decision_requests: BTreeMap::new(),
                    archived_decision_command_requests: BTreeSet::new(),
                    events: Vec::new(),
                    commands: Vec::new(),
                    command_attempts: Vec::new(),
                    ingress: Vec::new(),
                    boundaries: Vec::new(),
                    random_draws: Vec::new(),
                    archived_segment_headers: Vec::new(),
                    archived_evidence_receipts: BTreeMap::new(),
                    keyed_draw_reservations: Vec::new(),
                },
            },
            schema,
            plugins,
            sync_reaction_depth: 0,
        };
        simulation.refresh_checkpoint_hash()?;
        Ok(simulation)
    }

    pub fn demo(seed: u64) -> Result<(Self, DemoIds), CanwuError> {
        let (scenario, ids) = demo_scenario();
        Self::new(seed, scenario).map(|simulation| (simulation, ids))
    }

    pub fn register_plugin<P: SimulationPlugin + ?Sized>(
        &mut self,
        plugin: &P,
    ) -> Result<(), CanwuError> {
        let plugin_name = plugin.name().trim();
        if plugin_name.is_empty() || plugin_name != plugin.name() {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                "plugin name must be non-empty and have no surrounding whitespace",
            ));
        }
        let rehydrating = self.plugins.descriptors.contains_key(plugin_name)
            && !self.plugins.active_plugins.contains(plugin_name);
        if self.state.metadata.plugin_registration_closed && !rehydrating {
            return Err(CanwuError::new(
                ErrorCode::PluginRegistrationClosed,
                "new plugins must be registered before authoritative execution begins",
            ));
        }
        let state_start = self.state.clone();
        let schema_start = self.schema.clone();
        let plugins_start = self.plugins.clone();
        let result = (|| {
            self.plugins.register(plugin, &mut self.schema)?;
            self.invalidate_commitments(
                CommitmentDomains::RANDOM_STREAMS | CommitmentDomains::IDENTITY,
            );
            if !self.plugins.record_schemas.is_empty()
                && self.state.metadata.initial_scenario.is_none()
            {
                return Err(CanwuError::new(
                    ErrorCode::UnsupportedSnapshotVersion,
                    "this snapshot predates manifest-bound domain-record genesis and cannot activate record schemas",
                ));
            }
            records::validate_records_for_owner(
                &self.state.current.domain_records,
                &self.plugins.record_schemas,
                plugin_name,
                self.state.scheduler.now,
                &|entity| runtime_entity_exists(&self.state, entity),
            )?;
            let activation_records = self
                .state
                .current
                .domain_records
                .values()
                .filter(|record| record.owner == plugin_name)
                .cloned()
                .collect::<Vec<_>>();
            plugin.validate_activation(&activation_records)?;
            for stream in self.plugins.random_stream_owners.keys() {
                self.state
                    .current
                    .random_streams
                    .entry(stream.clone())
                    .or_insert_with(|| {
                        RandomStreamState::initial(self.state.current.root_seed, stream.clone())
                    });
            }
            self.refresh_checkpoint_hash()
        })();
        if let Err(error) = result {
            self.state = state_start;
            self.schema = schema_start;
            self.plugins = plugins_start;
            return Err(error);
        }
        Ok(())
    }

    fn ensure_runtime_ready(&self) -> Result<(), CanwuError> {
        // Initial construction, snapshot restore, and plugin activation perform
        // the complete domain-record audit. Thereafter all public mutations go
        // through affected-closure validation on the persistent store. Repeating
        // the cold audit here would deserialize every untouched plugin payload
        // before every ingress and boundary, defeating Format-8 shard isolation.
        self.plugins.ensure_active()
    }

    fn bound_initial_scenario(&self) -> Option<&Scenario> {
        self.state.metadata.initial_scenario.as_ref()
    }

    #[must_use]
    pub const fn time(&self) -> SimTime {
        self.state.scheduler.now
    }

    #[must_use]
    pub const fn run_manifest(&self) -> &RunManifest {
        &self.state.metadata.run_manifest
    }

    #[must_use]
    pub const fn run_configuration(&self) -> &RunConfigurationSnapshot {
        &self.state.metadata.run_configuration
    }

    #[must_use]
    /// Returns the persisted authoritative transaction revision.
    ///
    /// Accepted commands, persisted expected rejections, and completed
    /// settlement boundaries each advance it exactly once. Failed work, exact
    /// retries, bare clock movement, queued but unadmitted ingress, and plugin
    /// setup do not advance it; use the expected-time guard with external
    /// commands to detect clock and scheduled-work advancement.
    pub const fn revision(&self) -> u64 {
        self.state.counters.state_revision
    }

    #[must_use]
    pub fn run_manifest_hash(&self) -> &str {
        &self.state.metadata.run_manifest_hash
    }

    #[must_use]
    pub fn checkpoint_hash(&self) -> &str {
        &self.state.metadata.checkpoint_hash
    }

    /// Hash of simulated state and causal evidence. Run-purpose, controller,
    /// seat, observation, interaction, and trace policy remain save identity
    /// but are deliberately excluded from this authoritative result identity.
    pub fn authoritative_state_hash(&self) -> Result<String, CanwuError> {
        self.compute_boundary_state_hash()
    }

    pub fn entities(&self) -> impl Iterator<Item = &EntityRef> {
        self.state.current.entities.iter()
    }

    #[must_use]
    pub fn entity_exists(&self, entity: &EntityRef) -> bool {
        runtime_entity_exists(&self.state, entity)
    }

    #[must_use]
    pub fn world(&self) -> WorldSnapshot {
        WorldSnapshot {
            people: self.state.current.people.values().cloned().collect(),
            governments: self.state.current.governments.values().cloned().collect(),
            territories: self.state.current.territories.values().cloned().collect(),
            routes: self.state.current.routes.values().cloned().collect(),
            armies: self.state.current.armies.values().cloned().collect(),
            letters: self.state.current.letters.values().cloned().collect(),
        }
    }

    #[must_use]
    pub fn knowledge(&self) -> &KnowledgeSnapshot {
        &self.state.current.knowledge
    }

    #[must_use]
    pub fn events(&self) -> &[SimEvent] {
        &self.state.evidence.events
    }

    #[must_use]
    pub fn command_log(&self) -> &[CommandRecord] {
        &self.state.evidence.commands
    }

    #[must_use]
    pub fn command_attempts(&self) -> &[CommandAttemptRecord] {
        &self.state.evidence.command_attempts
    }

    #[must_use]
    pub fn ingress_log(&self) -> &[IngressRecord] {
        &self.state.evidence.ingress
    }

    #[must_use]
    pub fn domain_record(&self, reference: &DomainRecordRef) -> Option<&DomainRecord> {
        self.state.current.domain_records.get(reference)
    }

    /// Returns whether an exact domain-record version exists in current or retained evidence.
    #[must_use]
    pub fn domain_record_version_evidence_exists(
        &self,
        reference: &DomainRecordVersionRef,
    ) -> bool {
        !matches!(
            validation::resolve_evidence_reference(
                &validation::RuntimeValidationContext::new(&self.state),
                &EvidenceRef::DomainRecordVersion(reference.clone()),
            ),
            validation::EvidenceAvailability::Missing
        )
    }

    /// Returns whether a generic evidence identity is retained or archived.
    #[must_use]
    pub fn evidence_exists(&self, reference: &EvidenceRef) -> bool {
        !matches!(
            validation::resolve_evidence_reference(
                &validation::RuntimeValidationContext::new(&self.state),
                reference,
            ),
            validation::EvidenceAvailability::Missing
        )
    }

    /// Returns when retained evidence first became authoritative.
    ///
    /// `None` means the evidence is missing or only its compact archive
    /// receipt remains. Proposed same-boundary evidence is available through
    /// [`SimulationView::evidence_time`] while its boundary is being built.
    #[must_use]
    pub fn evidence_time(&self, reference: &EvidenceRef) -> Option<SimTime> {
        retained_evidence_time(&self.state, reference)
    }

    /// Resolves the retained record body for one exact domain-record version.
    ///
    /// Returns `None` when the version is unavailable or only its compacted
    /// archive receipt remains.
    #[must_use]
    pub fn domain_record_version(
        &self,
        reference: &DomainRecordVersionRef,
    ) -> Option<DomainRecord> {
        retained_domain_record_version(&self.state, reference)
    }

    #[must_use]
    pub fn typed_domain_record<T: DomainRecordType>(
        &self,
        reference: &TypedDomainRecordRef<T>,
    ) -> Option<&DomainRecord> {
        self.domain_record(reference.as_untyped())
    }

    pub fn domain_records(&self) -> impl Iterator<Item = &DomainRecord> {
        self.state.current.domain_records.values()
    }

    /// Returns one revision-bound page of authoritative domain records.
    ///
    /// Pass the revision returned by the first page on every subsequent page.
    /// A mutation between pages is rejected instead of mixing two read cuts.
    pub fn domain_record_page(
        &self,
        kind: &DomainRecordKind,
        after: Option<&DomainRecordRef>,
        limit: usize,
        expected_revision: Option<u64>,
    ) -> Result<DomainRecordPage, CanwuError> {
        validate_domain_record_page_request(kind, after, limit)?;
        let revision = self.revision();
        if expected_revision.is_some_and(|expected| expected != revision) {
            return Err(CanwuError::new(
                ErrorCode::SimulationRevisionConflict,
                format!(
                    "domain-record page expected revision {expected_revision:?}, current revision is {revision}"
                ),
            ));
        }
        let requested = limit.checked_add(1).unwrap_or(limit);
        let mut records =
            domain_record_candidates(&self.state.current.domain_records, kind, after, requested)
                .into_values()
                .collect::<Vec<_>>();
        let has_more = records.len() > limit;
        records.truncate(limit);
        let next = has_more
            .then(|| records.last().map(|record| record.reference.clone()))
            .flatten();
        Ok(DomainRecordPage {
            kind: kind.clone(),
            revision,
            records,
            next,
        })
    }

    #[must_use]
    pub fn boundaries(&self) -> &[BoundaryRecord] {
        &self.state.evidence.boundaries
    }

    #[must_use]
    pub fn random_draws(&self) -> &[RandomDrawRecord] {
        &self.state.evidence.random_draws
    }

    #[must_use]
    pub fn boundary_head_hash(&self) -> Option<&str> {
        self.state.evidence.boundary_head_hash()
    }

    #[must_use]
    pub const fn schema(&self) -> &SchemaRegistry {
        &self.schema
    }

    pub fn plugin_descriptors(&self) -> impl Iterator<Item = &PluginDescriptor> {
        self.plugins.descriptors()
    }

    /// Returns the persisted audience declaration for a plugin event.
    ///
    /// Built-in event visibility remains part of the public actor-relative
    /// projection. Unlisted plugin event types deliberately resolve to
    /// [`EventAudience::Private`].
    #[must_use]
    pub fn event_audience(&self, event: &SimEvent) -> EventAudience {
        match event.kind.event_type() {
            PLUGIN => event
                .kind
                .plugin_identity()
                .map_or(EventAudience::Private, |(plugin, event_type)| {
                    self.plugins.event_audience(plugin, event_type)
                }),
            KNOWLEDGE_PUBLISHED => KnowledgePublished::decode(&event.kind)
                .map_or(EventAudience::Private, |payload| {
                    EventAudience::KnowledgeHolder(payload.holder)
                }),
            _ => EventAudience::Private,
        }
    }

    ///
    /// # Panics
    ///
    /// Panics only if a runtime object was constructed without its required
    /// Format 8 initial scenario, which is prevented by the public loaders.
    #[must_use]
    pub fn replay_journal(&self) -> ReplayJournal {
        ReplayJournal {
            engine_version: ENGINE_VERSION.to_owned(),
            snapshot_format_version: SNAPSHOT_FORMAT_VERSION,
            root_seed: self.state.current.root_seed,
            initial_scenario: self
                .state
                .metadata
                .initial_scenario
                .clone()
                .expect("Format 8 runs always retain their initial scenario"),
            authority_root_seed: self.state.current.authority_root_seed,
            run_manifest: self.state.metadata.run_manifest.clone(),
            run_manifest_hash: self.state.metadata.run_manifest_hash.clone(),
            run_configuration: self.state.metadata.run_configuration.clone(),
            plugin_descriptors: self.plugins.descriptors().cloned().collect(),
            plugin_registration_closed: self.state.metadata.plugin_registration_closed,
            commands: self.state.evidence.commands.clone(),
            command_attempts: self.state.evidence.command_attempts.clone(),
            ingress: self.state.evidence.ingress.clone(),
            boundaries: self.state.evidence.boundaries.clone(),
            final_time: self.state.scheduler.now,
            checkpoint_hash: self.state.metadata.checkpoint_hash.clone(),
            commitment_format_version: self.state.metadata.commitment_format_version,
            revision_format_version: self.state.metadata.replay_revision_format_version,
            final_revision: self.state.counters.state_revision,
        }
    }

    /// Returns the durable external-delivery outbox derived from committed
    /// boundary emissions. Entries are deterministic and never re-sent by
    /// exact replay; the host owns delivery retries and acknowledgement.
    pub fn outbox_entries(&self) -> Result<Vec<OutboxEntry>, CanwuError> {
        Self::outbox_entries_for_boundaries(
            &self.state.metadata.run_manifest_hash,
            &self.state.evidence.boundaries,
        )
    }

    pub(crate) fn outbox_entries_for_boundaries(
        run_manifest_hash: &str,
        boundaries: &[BoundaryRecord],
    ) -> Result<Vec<OutboxEntry>, CanwuError> {
        let mut entries = Vec::new();
        for boundary in boundaries {
            for (index, emission) in boundary.emissions.iter().enumerate() {
                let emission_index = u64::try_from(index).map_err(|_| {
                    CanwuError::new(
                        ErrorCode::IdentifierExhausted,
                        "outbox emission index exceeds the persistent identifier space",
                    )
                })?;
                let delivery_id = canonical_hash(
                    "canwu.outbox.delivery.v1",
                    &(
                        run_manifest_hash,
                        boundary.id,
                        emission.event,
                        emission_index,
                    ),
                )?;
                entries.push(OutboxEntry {
                    delivery_id,
                    boundary: boundary.id,
                    event: emission.event,
                    emission_index,
                    plugin: emission.plugin.clone(),
                    system: emission.system.clone(),
                });
            }
        }
        Ok(entries)
    }

    fn compute_boundary_state_hash_for(
        &mut self,
        format: BoundaryStateHashFormat,
    ) -> Result<String, CanwuError> {
        match format {
            BoundaryStateHashFormat::LegacyV0 => self.compute_boundary_state_hash(),
            BoundaryStateHashFormat::CommitmentsV1 => {
                let roots = self.refresh_runtime_commitment_roots()?;
                boundary_state_hash_for_commitments(&roots)
            }
        }
    }

    fn compute_boundary_state_hash(&self) -> Result<String, CanwuError> {
        let world = self.world();
        let entities: Vec<_> = self.state.current.entities.iter().cloned().collect();
        let plugin_components: Vec<_> = self
            .state
            .current
            .plugin_components
            .values()
            .cloned()
            .collect();
        let domain_records: Vec<_> = self
            .state
            .current
            .domain_records
            .values()
            .cloned()
            .collect();
        let plugin_descriptors: Vec<_> = self.plugins.descriptors().cloned().collect();
        let scheduled: Vec<_> = self
            .state
            .scheduler
            .actions
            .iter()
            .map(|(key, action)| ScheduledRecord {
                key: key.clone(),
                action: action.clone(),
            })
            .collect();
        let random_streams: Vec<_> = self
            .state
            .current
            .random_streams
            .values()
            .cloned()
            .collect();
        let (authoritative_manifest, authoritative_manifest_hash) = authoritative_run_identity(
            &self.state.metadata.run_manifest,
            &self.state.metadata.run_manifest_hash,
            &self.state.metadata.run_configuration,
        )?;
        let initial_scenario = hashing::committed_initial_scenario(self.bound_initial_scenario());
        state_hash(&StateHashMaterial {
            engine_version: ENGINE_VERSION,
            snapshot_format_version: SNAPSHOT_FORMAT_VERSION,
            run_manifest: &authoritative_manifest,
            run_manifest_hash: &authoritative_manifest_hash,
            initial_time: self.state.scheduler.initial_time,
            initial_scenario: initial_scenario.as_ref(),
            now: self.state.scheduler.now,
            plugin_registration_closed: self.state.metadata.plugin_registration_closed,
            entities: hashing::committed_entities(&entities, &world),
            world: &world,
            knowledge: &self.state.current.knowledge,
            events: &self.state.evidence.events,
            commands: &self.state.evidence.commands,
            command_attempts: &self.state.evidence.command_attempts,
            ingress: &self.state.evidence.ingress,
            plugin_components: &plugin_components,
            domain_records: &domain_records,
            decisions: &self.state.current.decisions,
            plugin_descriptors: &plugin_descriptors,
            schema: &self.schema,
            scheduled: &scheduled,
            root_seed: self.state.current.root_seed,
            authority_root_seed: self.state.current.authority_root_seed,
            random_streams: &random_streams,
            random_draws: &self.state.evidence.random_draws,
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
        })
    }

    fn compute_commitment_root_updates(
        &self,
        needs: CommitmentDomains,
    ) -> Result<RuntimeCommitmentRootUpdates, CanwuError> {
        let world = needs
            .contains(CommitmentDomains::WORLD)
            .then(|| {
                let world = self.world();
                let entities: Vec<_> = self.state.current.entities.iter().cloned().collect();
                world_commitment_root(&world, &entities)
            })
            .transpose()?;
        let knowledge = needs
            .contains(CommitmentDomains::KNOWLEDGE)
            .then(|| knowledge_commitment_root(&self.state.current.knowledge))
            .transpose()?;
        let plugin_components = needs
            .contains(CommitmentDomains::PLUGIN_COMPONENTS)
            .then(|| {
                let values: Vec<_> = self
                    .state
                    .current
                    .plugin_components
                    .values()
                    .cloned()
                    .collect();
                plugin_component_commitment_root(&values)
            })
            .transpose()?;
        let domain_records = needs
            .contains(CommitmentDomains::DOMAIN_RECORDS)
            .then(|| self.state.current.domain_records.commitment_root())
            .transpose()?;
        let decisions = needs
            .contains(CommitmentDomains::DECISIONS)
            .then(|| decision_commitment_root(&self.state.current.decisions))
            .transpose()?;
        let scheduler = needs
            .contains(CommitmentDomains::SCHEDULER)
            .then(|| {
                let scheduled: Vec<_> = self
                    .state
                    .scheduler
                    .actions
                    .iter()
                    .map(|(key, action)| ScheduledRecord {
                        key: key.clone(),
                        action: action.clone(),
                    })
                    .collect();
                scheduler_commitment_root(self.state.scheduler.now, &scheduled)
            })
            .transpose()?;
        let random_streams = needs
            .contains(CommitmentDomains::RANDOM_STREAMS)
            .then(|| {
                let values: Vec<_> = self
                    .state
                    .current
                    .random_streams
                    .values()
                    .cloned()
                    .collect();
                random_stream_commitment_root(&values)
            })
            .transpose()?;
        let identity = if needs.contains(CommitmentDomains::IDENTITY) {
            let descriptors: Vec<_> = self.plugins.descriptors().cloned().collect();
            let (manifest, manifest_hash) = authoritative_run_identity(
                &self.state.metadata.run_manifest,
                &self.state.metadata.run_manifest_hash,
                &self.state.metadata.run_configuration,
            )?;
            let initial_scenario =
                hashing::committed_initial_scenario(self.bound_initial_scenario());
            Some(identity_commitment_root(
                ENGINE_VERSION,
                SNAPSHOT_FORMAT_VERSION,
                &manifest,
                &manifest_hash,
                self.state.scheduler.initial_time,
                initial_scenario.as_ref(),
                self.state.current.authority_root_seed,
                &descriptors,
                &self.schema,
            )?)
        } else {
            None
        };
        Ok(RuntimeCommitmentRootUpdates {
            world,
            knowledge,
            plugin_components,
            domain_records,
            decisions,
            scheduler,
            random_streams,
            identity,
        })
    }

    fn invalidate_commitments(&mut self, domains: CommitmentDomains) {
        if let Some(cache) = self.state.metadata.commitment_cache.as_mut() {
            cache.invalidate(domains);
        }
    }

    fn refresh_runtime_commitment_roots(&mut self) -> Result<CommitmentRoots, CanwuError> {
        if self.state.metadata.commitment_format_version != COMMITMENT_FORMAT_VERSION {
            return Err(CanwuError::new(
                ErrorCode::UnsupportedSnapshotVersion,
                format!(
                    "commitment format {} cannot produce boundary state commitment v1",
                    self.state.metadata.commitment_format_version
                ),
            ));
        }
        let needs = {
            if self.state.metadata.commitment_cache.is_none() {
                self.state.metadata.commitment_cache =
                    Some(RuntimeCommitmentCache::from_evidence(&self.state.evidence)?);
            }
            let cache = self
                .state
                .metadata
                .commitment_cache
                .as_mut()
                .ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::InvalidSnapshot,
                        "commitment cache is unavailable while refreshing runtime roots",
                    )
                })?;
            cache.sync(&self.state.evidence)?;
            cache.needs()
        };
        let updates = self.compute_commitment_root_updates(needs)?;
        let boundary_head = self.boundary_head_hash().map(str::to_owned);
        let control = ControlCommitmentMaterial {
            plugin_registration_closed: self.state.metadata.plugin_registration_closed,
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
        };
        let (domain_roots, journal_roots) = {
            let cache = self
                .state
                .metadata
                .commitment_cache
                .as_mut()
                .ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::InvalidSnapshot,
                        "commitment cache is unavailable while applying root updates",
                    )
                })?;
            cache.apply(updates);
            (cache.domain_roots()?, cache.roots())
        };
        runtime_commitment_roots(
            &domain_roots,
            &journal_roots,
            self.state.current.root_seed,
            boundary_head.as_deref(),
            &control,
        )
    }

    fn refresh_checkpoint_hash(&mut self) -> Result<(), CanwuError> {
        if self.state.metadata.commitment_format_version == COMMITMENT_FORMAT_VERSION {
            let roots = self.refresh_runtime_commitment_roots()?;
            self.state.metadata.checkpoint_hash = checkpoint_hash_for_commitments(
                &roots,
                &self.state.metadata.run_manifest_hash,
                self.state.metadata.commitment_format_version,
                STATE_REVISION_FORMAT_VERSION,
                self.state.counters.state_revision,
                self.state.metadata.replay_revision_format_version,
            )?;
            self.state.metadata.commitment_roots = Some(roots);
        } else if self.state.metadata.commitment_format_version == 0 {
            let state_hash = self.compute_boundary_state_hash()?;
            self.state.metadata.checkpoint_hash = checkpoint_hash_for_configuration(
                &state_hash,
                self.boundary_head_hash(),
                &self.state.metadata.run_manifest_hash,
                &self.state.metadata.run_configuration,
                STATE_REVISION_FORMAT_VERSION,
                self.state.counters.state_revision,
                self.state.metadata.replay_revision_format_version,
            )?;
            self.state.metadata.commitment_roots = None;
            self.state.metadata.commitment_cache = None;
        } else {
            return Err(CanwuError::new(
                ErrorCode::UnsupportedSnapshotVersion,
                format!(
                    "commitment format {} is unsupported; this engine writes format {COMMITMENT_FORMAT_VERSION}",
                    self.state.metadata.commitment_format_version
                ),
            ));
        }
        Ok(())
    }

    fn next_state_revision(&self) -> Result<u64, CanwuError> {
        self.state
            .counters
            .state_revision
            .checked_add(1)
            .ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::IdentifierExhausted,
                    "authoritative state revision space is exhausted",
                )
            })
    }

    fn advance_state_revision(&mut self) -> Result<u64, CanwuError> {
        let next = self.next_state_revision()?;
        self.state.counters.state_revision = next;
        Ok(next)
    }

    #[must_use]
    pub fn snapshot(&self) -> SimulationSnapshot {
        let mut snapshot = self.checkpoint_state();
        snapshot.events.clone_from(&self.state.evidence.events);
        snapshot.commands.clone_from(&self.state.evidence.commands);
        snapshot
            .command_attempts
            .clone_from(&self.state.evidence.command_attempts);
        snapshot.ingress.clone_from(&self.state.evidence.ingress);
        snapshot
            .boundaries
            .clone_from(&self.state.evidence.boundaries);
        snapshot
            .random_draws
            .clone_from(&self.state.evidence.random_draws);
        snapshot
    }

    pub fn snapshot_json(&self) -> Result<String, CanwuError> {
        serde_json::to_string_pretty(&self.snapshot()).map_err(|error| {
            CanwuError::new(
                ErrorCode::InvalidSnapshot,
                format!("could not serialize snapshot: {error}"),
            )
        })
    }

    pub fn from_snapshot(snapshot: SimulationSnapshot) -> Result<Self, CanwuError> {
        if snapshot.snapshot_format_version != SNAPSHOT_FORMAT_VERSION
            || snapshot.engine_version != ENGINE_VERSION
        {
            return Err(CanwuError::new(
                ErrorCode::UnsupportedSnapshotVersion,
                format!(
                    "the typed snapshot loader accepts only engine {ENGINE_VERSION} format {SNAPSHOT_FORMAT_VERSION}; pre-8 formats are not supported"
                ),
            ));
        }
        validate_current_snapshot_contract(&snapshot)?;
        validate_scenario_state(&Scenario {
            start_time: snapshot.now,
            entities: snapshot.entities.clone(),
            world: snapshot.world.clone(),
            knowledge: snapshot.knowledge.clone(),
            domain_records: snapshot.domain_records.clone(),
        })?;
        let plugins = PluginRegistry::from_descriptors(snapshot.plugin_descriptors.clone())?;
        validate_snapshot(&snapshot, &plugins)?;
        let admitted_ingress: BTreeSet<_> = snapshot
            .boundaries
            .iter()
            .flat_map(|boundary| boundary.admitted_ingress.iter().copied())
            .collect();
        let pending_ingress = snapshot
            .ingress
            .iter()
            .filter(|record| !admitted_ingress.contains(&record.id))
            .map(IngressQueueKey::from_record)
            .collect();
        let initial_scenario = Some(snapshot.initial_scenario.clone().ok_or_else(|| {
            invalid_snapshot_error("format 8 validation requires an initial scenario")
        })?);
        let initial_domain_record_indexes = initial_scenario
            .as_ref()
            .map(|scenario| {
                scenario
                    .domain_records
                    .iter()
                    .enumerate()
                    .map(|(index, record)| (record.reference.clone(), index))
                    .collect()
            })
            .unwrap_or_default();
        let mut simulation = Self {
            state: RuntimeState {
                current: RuntimeCurrentState {
                    entities: snapshot.entities.into_iter().collect(),
                    people: snapshot
                        .world
                        .people
                        .into_iter()
                        .map(|value| (value.id, value))
                        .collect(),
                    letters: snapshot
                        .world
                        .letters
                        .into_iter()
                        .map(|value| (value.id, value))
                        .collect(),
                    governments: snapshot
                        .world
                        .governments
                        .into_iter()
                        .map(|value| (value.id, value))
                        .collect(),
                    territories: snapshot
                        .world
                        .territories
                        .into_iter()
                        .map(|value| (value.id, value))
                        .collect(),
                    routes: snapshot
                        .world
                        .routes
                        .into_iter()
                        .map(|value| (value.id, value))
                        .collect(),
                    armies: snapshot
                        .world
                        .armies
                        .into_iter()
                        .map(|value| (value.id, value))
                        .collect(),
                    knowledge: snapshot.knowledge,
                    plugin_components: snapshot
                        .plugin_components
                        .into_iter()
                        .map(|record| {
                            (
                                component_key(
                                    &record.plugin,
                                    &record.state,
                                    &record.entity,
                                    &record.component,
                                ),
                                record,
                            )
                        })
                        .collect(),
                    domain_records: PersistentDomainRecordStore::from_records(
                        snapshot
                            .domain_records
                            .into_iter()
                            .map(|record| (record.reference.clone(), record))
                            .collect(),
                    )?,
                    decisions: snapshot.decisions,
                    root_seed: snapshot.root_seed,
                    authority_root_seed: snapshot.authority_root_seed,
                    random_streams: snapshot
                        .random_streams
                        .into_iter()
                        .map(|state| (state.key.clone(), state))
                        .collect(),
                },
                scheduler: RuntimeScheduler {
                    initial_time: snapshot.initial_time,
                    now: snapshot.now,
                    actions: snapshot
                        .scheduled
                        .into_iter()
                        .map(|record| (record.key, record.action))
                        .collect(),
                    pending_ingress,
                },
                counters: RuntimeCounters {
                    next_event_id: snapshot.next_event_id,
                    next_command_id: snapshot.next_command_id,
                    next_command_attempt_id: snapshot.next_command_attempt_id,
                    next_ingress_id: snapshot.next_ingress_id,
                    next_boundary_id: snapshot.next_boundary_id,
                    next_random_draw_id: snapshot.next_random_draw_id,
                    next_knowledge_record_id: snapshot.next_knowledge_record_id,
                    next_schedule_sequence: snapshot.next_schedule_sequence,
                    next_correlation_id: snapshot.next_correlation_id,
                    next_decision_trace_id: snapshot.next_decision_trace_id,
                    state_revision: snapshot.state_revision,
                    admitted_attempt_count: snapshot.admitted_attempt_count,
                    admitted_command_count: snapshot.admitted_command_count,
                    admitted_event_count: snapshot.admitted_event_count,
                },
                metadata: RuntimeMetadata {
                    initial_scenario,
                    initial_domain_record_indexes,
                    run_manifest: snapshot.run_manifest.clone().ok_or_else(|| {
                        invalid_snapshot_error("snapshot is missing its run manifest")
                    })?,
                    run_manifest_hash: snapshot.run_manifest_hash.clone(),
                    run_configuration: snapshot.run_configuration.clone().ok_or_else(|| {
                        invalid_snapshot_error("snapshot is missing its run configuration")
                    })?,
                    checkpoint_hash: snapshot.checkpoint_hash.clone(),
                    commitment_format_version: snapshot.commitment_format_version,
                    commitment_roots: snapshot.commitment_roots.clone(),
                    commitment_cache: None,
                    plugin_registration_closed: snapshot.plugin_registration_closed,
                    replay_revision_format_version: snapshot.replay_revision_format_version,
                },
                evidence: RuntimeEvidence {
                    archived: EvidenceCursor::default(),
                    archived_boundary_head: None,
                    archived_legacy_commands: false,
                    archived_tracked_attempts: false,
                    archived_unqueued_command_history: false,
                    archived_command_requests: BTreeMap::new(),
                    archived_ingress_requests: BTreeMap::new(),
                    archived_decision_requests: BTreeMap::new(),
                    archived_decision_command_requests: BTreeSet::new(),
                    events: snapshot.events,
                    commands: snapshot.commands,
                    command_attempts: snapshot.command_attempts,
                    ingress: snapshot.ingress,
                    boundaries: snapshot.boundaries,
                    random_draws: snapshot.random_draws,
                    archived_segment_headers: Vec::new(),
                    archived_evidence_receipts: BTreeMap::new(),
                    keyed_draw_reservations: Vec::new(),
                },
            },
            schema: snapshot.schema,
            plugins,
            sync_reaction_depth: 0,
        };
        simulation.refresh_checkpoint_hash()?;
        Ok(simulation)
    }

    pub fn from_snapshot_json(json: &str) -> Result<Self, CanwuError> {
        let snapshot = deserialize_current_snapshot_json(json)?;
        Self::from_snapshot(snapshot)
    }

    pub fn from_snapshot_with_plugins(
        snapshot: SimulationSnapshot,
        plugins: &[&dyn SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        let mut simulation = Self::from_snapshot(snapshot)?;
        for plugin in plugins {
            simulation.register_plugin(*plugin)?;
        }
        simulation.ensure_runtime_ready()?;
        Ok(simulation)
    }

    pub fn from_snapshot_json_with_plugins(
        json: &str,
        plugins: &[&dyn SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        let snapshot = deserialize_current_snapshot_json(json)?;
        Self::from_snapshot_with_plugins(snapshot, plugins)
    }

    #[must_use]
    pub fn fork(&self) -> Self {
        Self {
            state: self.state.clone(),
            schema: self.schema.clone(),
            plugins: self.plugins.clone(),
            sync_reaction_depth: 0,
        }
    }

    fn prepare_command(
        &self,
        envelope: &CommandEnvelope,
        context: &CommandContext,
    ) -> Result<PreparedCommand, CanwuError> {
        match &envelope.command {
            Command::OrderMovement {
                subject,
                destination,
                cargo,
            } => {
                let Some(actor) = decision_actor(&context.authority) else {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidAuthority,
                        "movement commands require an accountable actor origin",
                    ));
                };
                let person = self.state.current.people.get(&actor).ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::ActorNotFound,
                        format!("actor {actor} was not found"),
                    )
                    .with_entity(EntityRef::Person(actor))
                })?;
                if context
                    .authority
                    .command_subject
                    .as_ref()
                    .is_some_and(|bound| bound != subject)
                {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidAuthority,
                        "command subject does not match the movement subject",
                    )
                    .with_entity(subject.clone()));
                }
                if !self.state.current.territories.contains_key(destination) {
                    return Err(CanwuError::new(
                        ErrorCode::DestinationNotFound,
                        format!("destination {destination} was not found"),
                    )
                    .with_entity(EntityRef::Territory(*destination)));
                }
                if cargo.windows(2).any(|pair| pair[0] >= pair[1]) {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidPayload,
                        "movement cargo IDs must be sorted and unique",
                    ));
                }
                match subject {
                    EntityRef::Army(army) => {
                        if !cargo.is_empty() {
                            return Err(CanwuError::new(
                                ErrorCode::InvalidPayload,
                                "army movement does not accept letter cargo yet",
                            ));
                        }
                        let army_state = self.state.current.armies.get(army).ok_or_else(|| {
                            CanwuError::new(
                                ErrorCode::ArmyNotFound,
                                format!("army {army} was not found"),
                            )
                            .with_entity(EntityRef::Army(*army))
                        })?;
                        if army_state.commander != person.id {
                            return Err(CanwuError::new(
                                ErrorCode::InvalidAuthority,
                                format!("{} does not command {}", person.name, army_state.name),
                            )
                            .with_entity(EntityRef::Person(person.id))
                            .with_entity(EntityRef::Army(*army)));
                        }
                        if army_state.transit.is_some() {
                            return Err(CanwuError::new(
                                ErrorCode::InvalidAuthority,
                                format!("{} is already moving", army_state.name),
                            )
                            .with_entity(EntityRef::Army(*army)));
                        }
                        let arrival_at =
                            self.movement_arrival_time(army_state.location, *destination)?;
                        Ok(PreparedCommand::ArmyMovement {
                            army: *army,
                            actor,
                            from: army_state.location,
                            destination: *destination,
                            arrival_at,
                        })
                    }
                    EntityRef::Person(person_id) => {
                        if *person_id != actor
                            || context
                                .authority
                                .command_subject
                                .as_ref()
                                .is_some_and(|subject| subject != &EntityRef::Person(*person_id))
                        {
                            return Err(CanwuError::new(
                                ErrorCode::InvalidAuthority,
                                "self-directed movement must bind the actor to the person subject",
                            )
                            .with_entity(EntityRef::Person(*person_id)));
                        }
                        let person_state =
                            self.state.current.people.get(person_id).ok_or_else(|| {
                                CanwuError::new(
                                    ErrorCode::EntityNotFound,
                                    format!("person {person_id} was not found"),
                                )
                                .with_entity(EntityRef::Person(*person_id))
                            })?;
                        if person_state.transit.is_some() {
                            return Err(CanwuError::new(
                                ErrorCode::InvalidAuthority,
                                format!("person {person_id} is already moving"),
                            )
                            .with_entity(EntityRef::Person(*person_id)));
                        }
                        for letter_id in cargo {
                            let letter =
                                self.state.current.letters.get(letter_id).ok_or_else(|| {
                                    CanwuError::new(
                                        ErrorCode::EntityNotFound,
                                        format!("letter {letter_id} was not found"),
                                    )
                                    .with_entity(
                                        EntityRef::Resource(ResourceId::new(letter_id.get())),
                                    )
                                })?;
                            if letter.status != LetterStatus::HeldByPerson
                                || letter.carrier != Some(*person_id)
                                || !self.state.current.people.contains_key(&letter.sender)
                                || !self.state.current.people.contains_key(&letter.recipient)
                            {
                                return Err(CanwuError::new(
                                    ErrorCode::InvalidAuthority,
                                    format!("letter {letter_id} is not held by the moving person"),
                                )
                                .with_entity(EntityRef::Resource(ResourceId::new(
                                    letter_id.get(),
                                ))));
                            }
                        }
                        let arrival_at = self
                            .movement_arrival_time(person_state.current_location, *destination)?;
                        Ok(PreparedCommand::MovePerson {
                            person: *person_id,
                            from: person_state.current_location,
                            destination: *destination,
                            cargo: cargo.clone(),
                            arrival_at,
                        })
                    }
                    _ => Err(CanwuError::new(
                        ErrorCode::InvalidAuthority,
                        "only army and person subjects support built-in movement",
                    )
                    .with_entity(subject.clone())),
                }
            }
            Command::DebugSetArmyMorale { army, morale } => {
                if envelope.issuer != Issuer::Debug {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidAuthority,
                        "debug state edits require the explicit debug issuer",
                    ));
                }
                if *morale > 100 {
                    return Err(CanwuError::new(
                        ErrorCode::ValueOutOfRange,
                        "army morale must be between 0 and 100",
                    ));
                }
                let old_morale = self.state.current.armies.get(army).map_or_else(
                    || {
                        Err(CanwuError::new(
                            ErrorCode::ArmyNotFound,
                            format!("army {army} was not found"),
                        ))
                    },
                    |army_state| Ok(army_state.morale),
                )?;
                Ok(PreparedCommand::DebugMorale {
                    army: *army,
                    old_morale,
                    new_morale: *morale,
                })
            }
            Command::Plugin {
                plugin,
                command,
                payload,
            } => {
                let registered = self
                    .plugins
                    .commands
                    .get(&(plugin.clone(), command.clone()))
                    .ok_or_else(|| {
                        CanwuError::new(
                            ErrorCode::PluginCommandNotFound,
                            format!("plugin command {plugin}.{command} is not registered"),
                        )
                    })?;
                let handler = registered.handler;
                let descriptor = registered.descriptor.clone();
                descriptor.payload_schema.validate(payload)?;
                let reader = format!("{plugin}.{command}");
                let directives = catch_unwind(AssertUnwindSafe(|| {
                    handler(
                        &self.plugin_view(&reader, &descriptor.reads),
                        context,
                        payload,
                    )
                }))
                .map_err(|_| {
                    CanwuError::new(
                        ErrorCode::PluginPanicked,
                        format!("plugin command {plugin}.{command} panicked"),
                    )
                })??;
                validate_directives_with_context(
                    &RuntimeValidationContext::new(&self.state),
                    plugin,
                    &descriptor.writes,
                    &self.plugins.state_owners,
                    &self.plugins.record_schemas,
                    &directives,
                )?;
                Ok(PreparedCommand::Plugin {
                    plugin: plugin.clone(),
                    directives,
                    allowed_writes: descriptor.writes,
                })
            }
        }
    }

    fn movement_arrival_time(
        &self,
        from: TerritoryId,
        to: TerritoryId,
    ) -> Result<SimTime, CanwuError> {
        let travel_minutes = if from == to {
            1
        } else {
            self.state
                .current
                .routes
                .values()
                .find(|route| route.connects(from, to))
                .ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::NoRoute,
                        format!("no direct route connects territory {from} to {to}"),
                    )
                })?
                .travel_minutes
        };
        if travel_minutes <= 0 {
            return Err(CanwuError::new(
                ErrorCode::InvalidDuration,
                "movement route duration must be positive",
            ));
        }
        self.state
            .scheduler
            .now
            .checked_add(SimDuration::minutes(travel_minutes))
            .ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::InvalidDuration,
                    "movement arrival time exceeds the supported range",
                )
            })
    }

    fn apply_prepared(
        &mut self,
        prepared: PreparedCommand,
        command_id: CommandId,
        correlation_id: u64,
    ) -> Result<(), CanwuError> {
        match prepared {
            PreparedCommand::ArmyMovement {
                army,
                actor,
                from,
                destination,
                arrival_at,
            } => {
                let army_state = self.state.current.armies.get_mut(&army).ok_or_else(|| {
                    CanwuError::new(ErrorCode::ArmyNotFound, "validated army disappeared")
                })?;
                army_state.transit = Some(TransitState {
                    from,
                    to: destination,
                    departed_at: self.state.scheduler.now,
                    arrives_at: arrival_at,
                });
                let event = self.emit(
                    MoveOrdered {
                        army,
                        from,
                        to: destination,
                        arrival_at,
                    }
                    .into_kind(),
                    vec![
                        EntityRef::Army(army),
                        EntityRef::Person(actor),
                        EntityRef::Territory(from),
                        EntityRef::Territory(destination),
                    ],
                    format!("Army {army} was ordered from {from} to {destination}"),
                    Some(CauseRef::Command(command_id)),
                    correlation_id,
                )?;
                self.schedule_at(
                    arrival_at,
                    ScheduledAction::ArmyArrival {
                        army,
                        destination,
                        order_event: event,
                        correlation_id,
                    },
                )?;
            }
            PreparedCommand::MovePerson {
                person,
                from,
                destination,
                cargo,
                arrival_at,
            } => {
                self.invalidate_commitments(CommitmentDomains::WORLD);
                let person_state = self.state.current.people.get_mut(&person).ok_or_else(|| {
                    CanwuError::new(ErrorCode::EntityNotFound, "validated person disappeared")
                })?;
                person_state.transit = Some(PersonTransitState {
                    from,
                    to: destination,
                    departed_at: self.state.scheduler.now,
                    arrives_at: arrival_at,
                });
                for letter_id in &cargo {
                    let letter =
                        self.state
                            .current
                            .letters
                            .get_mut(letter_id)
                            .ok_or_else(|| {
                                CanwuError::new(
                                    ErrorCode::EntityNotFound,
                                    "validated letter disappeared",
                                )
                            })?;
                    letter.status = LetterStatus::InTransit;
                    letter.carrier = Some(person);
                    letter.location = None;
                }
                let event = self.emit(
                    PersonMoveOrdered {
                        person,
                        from,
                        to: destination,
                        arrival_at,
                    }
                    .into_kind(),
                    std::iter::once(EntityRef::Person(person))
                        .chain(
                            cargo
                                .iter()
                                .copied()
                                .map(|id| EntityRef::Resource(ResourceId::new(id.get()))),
                        )
                        .chain([
                            EntityRef::Territory(from),
                            EntityRef::Territory(destination),
                        ])
                        .collect(),
                    format!("Person {person} was ordered from {from} to {destination}"),
                    Some(CauseRef::Command(command_id)),
                    correlation_id,
                )?;
                self.schedule_at(
                    arrival_at,
                    ScheduledAction::PersonArrival {
                        person,
                        destination,
                        order_event: event,
                        cargo,
                        correlation_id,
                    },
                )?;
            }
            PreparedCommand::DebugMorale {
                army,
                old_morale,
                new_morale,
            } => {
                self.state
                    .current
                    .armies
                    .get_mut(&army)
                    .ok_or_else(|| {
                        CanwuError::new(ErrorCode::ArmyNotFound, "validated army disappeared")
                    })?
                    .morale = new_morale;
                self.emit(
                    DebugFieldChanged {
                        entity: EntityRef::Army(army),
                        field: "morale".to_owned(),
                        old_value: old_morale.to_string(),
                        new_value: new_morale.to_string(),
                    }
                    .into_kind(),
                    vec![EntityRef::Army(army)],
                    format!(
                        "Debug command changed army {army} morale {old_morale} -> {new_morale}"
                    ),
                    Some(CauseRef::Command(command_id)),
                    correlation_id,
                )?;
            }
            PreparedCommand::Plugin {
                plugin,
                directives,
                allowed_writes,
            } => {
                self.apply_directives(
                    &plugin,
                    directives,
                    &allowed_writes,
                    &CauseRef::Command(command_id),
                    correlation_id,
                )?;
            }
        }
        Ok(())
    }
}

enum PreparedCommand {
    ArmyMovement {
        army: ArmyId,
        actor: PersonId,
        from: TerritoryId,
        destination: TerritoryId,
        arrival_at: SimTime,
    },
    MovePerson {
        person: PersonId,
        from: TerritoryId,
        destination: TerritoryId,
        cargo: Vec<LetterId>,
        arrival_at: SimTime,
    },
    DebugMorale {
        army: ArmyId,
        old_morale: u16,
        new_morale: u16,
    },
    Plugin {
        plugin: String,
        directives: Vec<SystemDirective>,
        allowed_writes: Vec<StateKey>,
    },
}

impl PreparedCommand {
    fn commitment_invalidation(&self) -> CommitmentDomains {
        match self {
            Self::ArmyMovement { .. } => {
                CommitmentDomains::WORLD
                    | CommitmentDomains::KNOWLEDGE
                    | CommitmentDomains::PLUGIN_COMPONENTS
                    | CommitmentDomains::SCHEDULER
            }
            Self::MovePerson { .. } => CommitmentDomains::WORLD | CommitmentDomains::SCHEDULER,
            Self::DebugMorale { .. } => {
                CommitmentDomains::WORLD
                    | CommitmentDomains::PLUGIN_COMPONENTS
                    | CommitmentDomains::SCHEDULER
            }
            Self::Plugin { .. } => {
                CommitmentDomains::PLUGIN_COMPONENTS | CommitmentDomains::SCHEDULER
            }
        }
    }
}

fn validate_directives(
    plugin: &str,
    allowed_writes: &[StateKey],
    state_owners: &BTreeMap<StateKey, String>,
    record_schemas: &records::DomainRecordSchemas,
    entity_exists: &dyn Fn(&EntityRef) -> bool,
    directives: &[SystemDirective],
) -> Result<(), CanwuError> {
    for directive in directives {
        match directive {
            SystemDirective::SetComponent {
                state,
                entity,
                component,
                ..
            } => {
                if component.trim().is_empty() || component != component.trim() {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidPayload,
                        "plugin component name must be non-empty and canonical",
                    ));
                }
                if !allowed_writes.contains(state) {
                    return Err(CanwuError::new(
                        ErrorCode::UndeclaredStateWrite,
                        format!(
                            "plugin {plugin} did not declare write access to {}.{}",
                            state.namespace, state.name
                        ),
                    ));
                }
                if state_owners.get(state).is_none_or(|owner| owner != plugin) {
                    return Err(CanwuError::new(
                        ErrorCode::UndeclaredStateWrite,
                        format!(
                            "plugin {plugin} does not own state {}.{}",
                            state.namespace, state.name
                        ),
                    ));
                }
                if is_domain_record_state(record_schemas, state) {
                    return Err(CanwuError::new(
                        ErrorCode::UndeclaredStateWrite,
                        "domain record state cannot be written as an immediate component",
                    ));
                }
                if !entity_exists(entity) {
                    return Err(CanwuError::new(
                        ErrorCode::EntityNotFound,
                        format!("plugin {plugin} targeted missing entity {entity}"),
                    )
                    .with_entity(entity.clone()));
                }
            }
            SystemDirective::Emit { event_type, .. }
                if event_type.trim().is_empty() || event_type != event_type.trim() =>
            {
                return Err(CanwuError::new(
                    ErrorCode::InvalidPayload,
                    "plugin event type must be non-empty and canonical",
                ));
            }
            SystemDirective::Emit { affected, .. }
                if affected.iter().any(|entity| !entity_exists(entity)) =>
            {
                return Err(CanwuError::new(
                    ErrorCode::EntityNotFound,
                    format!("plugin {plugin} emitted an event for a missing entity"),
                ));
            }
            SystemDirective::Schedule { after, directive } => {
                if *after <= SimDuration::ZERO {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidDuration,
                        "plugin systems must schedule work strictly in the future",
                    ));
                }
                validate_directives(
                    plugin,
                    allowed_writes,
                    state_owners,
                    record_schemas,
                    entity_exists,
                    std::slice::from_ref(directive),
                )?;
            }
            SystemDirective::EnqueuePluginIngress {
                after,
                packet_type,
                affected,
                ..
            } => {
                if packet_type.trim().is_empty() || packet_type != packet_type.trim() {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidPayload,
                        "plugin ingress type must be non-empty and canonical",
                    ));
                }
                if *after < SimDuration::ZERO {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidDuration,
                        "plugin command ingress delay cannot be negative",
                    ));
                }
                if affected.iter().any(|entity| !entity_exists(entity)) {
                    return Err(CanwuError::new(
                        ErrorCode::EntityNotFound,
                        format!("plugin {plugin} queued ingress for a missing entity"),
                    ));
                }
            }
            SystemDirective::Emit { .. } => {}
        }
    }
    Ok(())
}

fn resolve_command_authority(envelope: &CommandEnvelope) -> Result<CommandAuthority, CanwuError> {
    if let Some(authority) = &envelope.authority {
        return Ok(authority.clone());
    }
    match &envelope.issuer {
        Issuer::Actor(actor) => Ok(CommandAuthority::for_actor(*actor)),
        Issuer::Debug => Ok(CommandAuthority::no_responsible_actor("debug-command")),
        Issuer::System(system) => Ok(CommandAuthority::no_responsible_actor(format!(
            "system:{system}"
        ))),
        Issuer::Human(_)
        | Issuer::Ai(_)
        | Issuer::Institution(_)
        | Issuer::Replay(_)
        | Issuer::Experiment(_) => Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "typed command origins require an explicit authority context",
        )),
    }
}

fn validate_command_ingress_policy(
    run_configuration: &RunConfigurationSnapshot,
    issuer: &Issuer,
    authority: &CommandAuthority,
    admission: CommandAdmission,
    entity_exists: &dyn Fn(&EntityRef) -> bool,
) -> Result<(), CanwuError> {
    let CommandAdmission {
        request_id,
        expected_revision,
        expected_time,
        revision_before: current_revision,
        ingress,
    } = admission;
    if request_id.is_some_and(|id| id.get() == 0) {
        return Err(CanwuError::new(
            ErrorCode::InvalidPayload,
            "command request IDs must be nonzero",
        ));
    }
    if let Some(expected) = expected_revision
        && expected != current_revision
    {
        return Err(CanwuError::new(
            ErrorCode::SimulationRevisionConflict,
            format!(
                "command expected revision {expected}, but simulation is at revision {current_revision}"
            ),
        ));
    }
    validate_command_authority(authority, entity_exists)?;
    if matches!(issuer, Issuer::Replay(_)) != (ingress == CommandIngress::FrozenReplay) {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "replay command origins are valid only for frozen replay ingress",
        ));
    }

    let RunConfigurationSnapshot::Declared(configuration) = run_configuration else {
        return Ok(());
    };
    if ingress == CommandIngress::LegacyDirect {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "declared runs require tracked request or frozen replay ingress",
        ));
    }
    let external = !matches!(issuer, Issuer::System(_));
    if configuration.require_idempotency_keys && external && request_id.is_none() {
        return Err(CanwuError::new(
            ErrorCode::MissingIdempotencyKey,
            "this run requires a stable command request ID",
        ));
    }
    if configuration.require_idempotency_keys && external && expected_revision.is_none() {
        return Err(CanwuError::new(
            ErrorCode::SimulationRevisionConflict,
            "this run requires an expected command revision",
        ));
    }
    if configuration.interaction == InteractionPolicy::ReadOnly
        && !matches!(issuer, Issuer::Replay(_) | Issuer::System(_))
    {
        return Err(CanwuError::new(
            ErrorCode::InteractionReadOnly,
            "the run interaction policy rejects newly authored authoritative commands",
        ));
    }
    if external && expected_time.is_none() {
        return Err(CanwuError::new(
            ErrorCode::SimulationTimeConflict,
            "declared external commands require an expected simulation time",
        ));
    }

    match issuer {
        Issuer::Actor(_) => Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "declared runs require a typed human, AI, institution, replay, experiment, debug, or system origin",
        )),
        Issuer::Human(controller) => {
            let Some(binding) = &configuration.seat_binding else {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "human commands require the run's exact seat binding",
                ));
            };
            if configuration.controller != ControllerPolicy::HumanRoleBound
                || controller != &binding.controller_id
                || authority.seat_id.as_deref() != Some(binding.seat_id.as_str())
                || authority.permission_profile_id.as_deref()
                    != Some(binding.permission_profile_id.as_str())
                || !authority_matches_seat_binding(configuration.seat, binding, authority)
            {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "human command origin does not match the active controller, seat binding, and permission profile",
                ));
            }
            Ok(())
        }
        Issuer::Ai(controller) | Issuer::Institution(controller) => {
            if !canonical_text(controller)
                || matches!(
                    authority.decision_origin,
                    DecisionOrigin::NoResponsibleActor { .. }
                )
            {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "AI and institutional commands require a canonical controller and responsible decision origin",
                ));
            }
            Ok(())
        }
        Issuer::Replay(source) => {
            if !canonical_text(source)
                || ingress != CommandIngress::FrozenReplay
                || configuration.purpose != RunPurpose::Replay
                || configuration.controller != ControllerPolicy::ReplayController
                || configuration.interaction != InteractionPolicy::ReadOnly
            {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "replay command sources require a replay-purpose, replay-controller, read-only run",
                ));
            }
            if let Some(binding) = &configuration.seat_binding
                && (source != &binding.controller_id
                    || authority.seat_id.as_deref() != Some(binding.seat_id.as_str())
                    || authority.permission_profile_id.as_deref()
                        != Some(binding.permission_profile_id.as_str())
                    || !authority_matches_seat_binding(configuration.seat, binding, authority))
            {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "frozen replay input does not match its recorded controller and seat binding",
                ));
            }
            Ok(())
        }
        Issuer::Experiment(intervention) => {
            if configuration.interaction != InteractionPolicy::VersionedExperiment
                || !configuration.declared_interventions.contains(intervention)
            {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "experiment commands must name an intervention declared by the run",
                ));
            }
            Ok(())
        }
        Issuer::Debug => {
            if !configuration.diagnostic_commands_enabled {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "debug command authority is disabled by the run configuration",
                ));
            }
            Ok(())
        }
        Issuer::System(system) => {
            if !canonical_text(system)
                || !matches!(
                    authority.decision_origin,
                    DecisionOrigin::NoResponsibleActor { .. }
                )
            {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "system commands require a canonical system ID and typed no-responsible-actor origin",
                ));
            }
            Ok(())
        }
    }
}

fn validate_command_authority(
    authority: &CommandAuthority,
    entity_exists: &dyn Fn(&EntityRef) -> bool,
) -> Result<(), CanwuError> {
    if authority
        .seat_id
        .as_ref()
        .is_some_and(|value| !canonical_text(value))
        || authority
            .permission_profile_id
            .as_ref()
            .is_some_and(|value| !canonical_text(value))
        || authority.seat_id.is_some() != authority.permission_profile_id.is_some()
        || authority
            .command_subject
            .as_ref()
            .is_some_and(|entity| !entity_exists(entity))
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "command authority contains an invalid seat, permission profile, or subject",
        ));
    }
    match &authority.decision_origin {
        DecisionOrigin::Actor { actor } => {
            if !entity_exists(&EntityRef::Person(*actor)) {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "command decision origin references a missing actor",
                ));
            }
        }
        DecisionOrigin::Institution {
            institution,
            responsible_actor,
        } => {
            if !entity_exists(institution)
                || responsible_actor.is_some_and(|actor| !entity_exists(&EntityRef::Person(actor)))
            {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "command decision origin references a missing institution or actor",
                ));
            }
        }
        DecisionOrigin::Council { council_id } if !canonical_text(council_id) => {
            return Err(CanwuError::new(
                ErrorCode::InvalidAuthority,
                "command council origin requires a canonical ID",
            ));
        }
        DecisionOrigin::NoResponsibleActor { reason } if !canonical_text(reason) => {
            return Err(CanwuError::new(
                ErrorCode::InvalidAuthority,
                "no-responsible-actor origins require a canonical reason",
            ));
        }
        DecisionOrigin::Council { .. } | DecisionOrigin::NoResponsibleActor { .. } => {}
    }
    Ok(())
}

fn authority_matches_seat_binding(
    seat: SeatPolicy,
    binding: &SeatBinding,
    authority: &CommandAuthority,
) -> bool {
    match (seat, &authority.decision_origin) {
        (SeatPolicy::CharacterBound, DecisionOrigin::Actor { actor }) => {
            binding.actor == Some(*actor) && binding.institution.is_none()
        }
        (
            SeatPolicy::InstitutionBound,
            DecisionOrigin::Institution {
                institution,
                responsible_actor,
            },
        ) => {
            binding.institution.as_ref() == Some(institution)
                && binding
                    .actor
                    .is_none_or(|actor| Some(actor) == *responsible_actor)
        }
        (SeatPolicy::ObserverSeat | SeatPolicy::AdvisorSeat, origin) => {
            let actor_matches = binding.actor.is_none_or(
                |expected| matches!(origin, DecisionOrigin::Actor { actor } if *actor == expected),
            );
            let institution_matches = binding.institution.as_ref().is_none_or(|expected| {
                matches!(
                    origin,
                    DecisionOrigin::Institution { institution, .. } if institution == expected
                )
            });
            actor_matches && institution_matches
        }
        _ => false,
    }
}

const fn decision_actor(authority: &CommandAuthority) -> Option<PersonId> {
    match &authority.decision_origin {
        DecisionOrigin::Actor { actor } => Some(*actor),
        DecisionOrigin::Institution {
            responsible_actor, ..
        } => *responsible_actor,
        DecisionOrigin::Council { .. } | DecisionOrigin::NoResponsibleActor { .. } => None,
    }
}

const fn is_expected_command_rejection(code: &ErrorCode) -> bool {
    matches!(
        code,
        ErrorCode::ActorNotFound
            | ErrorCode::ArmyNotFound
            | ErrorCode::DestinationNotFound
            | ErrorCode::EntityNotFound
            | ErrorCode::IdempotencyConflict
            | ErrorCode::InteractionReadOnly
            | ErrorCode::InvalidAuthority
            | ErrorCode::InvalidDuration
            | ErrorCode::InvalidPayload
            | ErrorCode::MissingIdempotencyKey
            | ErrorCode::MixedCommandIngress
            | ErrorCode::NoRoute
            | ErrorCode::PluginCommandNotFound
            | ErrorCode::SimulationRevisionConflict
            | ErrorCode::SimulationTimeConflict
            | ErrorCode::ValueOutOfRange
    )
}

fn canonical_text(value: &str) -> bool {
    !value.is_empty() && value == value.trim()
}

fn component_key(
    plugin: &str,
    state: &StateKey,
    entity: &EntityRef,
    component: &str,
) -> PluginComponentKey {
    PluginComponentKey {
        plugin: plugin.to_owned(),
        state: state.clone(),
        entity: entity.clone(),
        component: component.to_owned(),
    }
}

fn record_change_affected_entities(change: &DomainRecordChange) -> Vec<EntityRef> {
    (change.current.class == DomainRecordClass::Entity)
        .then(|| EntityRef::Domain(change.current.reference.clone()))
        .into_iter()
        .collect()
}

fn is_domain_record_state(schemas: &records::DomainRecordSchemas, state: &StateKey) -> bool {
    schemas.contains_key(&DomainRecordKind::new(&state.namespace, &state.name))
}

fn snapshot_command_attempt_preflight_error(
    snapshot: &SimulationSnapshot,
    attempt: &CommandAttemptRecord,
    history: &DomainRecordHistory,
    cut: DomainHistoryCut,
) -> Option<CanwuError> {
    let authority = match resolve_command_authority(&attempt.envelope) {
        Ok(authority) => authority,
        Err(error) => return Some(error),
    };
    let Some(run_configuration) = snapshot.run_configuration.as_ref() else {
        return Some(invalid_snapshot_error(
            "snapshot run configuration is required before command attempts",
        ));
    };
    if let Err(error) = validate_command_ingress_policy(
        run_configuration,
        &attempt.envelope.issuer,
        &authority,
        CommandAdmission {
            request_id: attempt.request_id,
            expected_revision: attempt.expected_revision,
            expected_time: attempt.envelope.expected_time,
            revision_before: attempt.revision_before,
            ingress: attempt.ingress,
        },
        &|entity| snapshot_entity_exists_in_history(snapshot, history, cut, entity),
    ) {
        return Some(error);
    }
    attempt.envelope.expected_time.and_then(|expected_time| {
        (expected_time != attempt.at).then(|| {
            CanwuError::new(
                ErrorCode::SimulationTimeConflict,
                format!(
                    "command expected time {expected_time}, but simulation is at {}",
                    attempt.at
                ),
            )
        })
    })
}

fn invalid_snapshot_error(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidSnapshot, message)
}

fn invalid_snapshot<T>(message: impl Into<String>) -> Result<T, CanwuError> {
    Err(invalid_snapshot_error(message))
}

#[cfg(test)]
mod tests;
