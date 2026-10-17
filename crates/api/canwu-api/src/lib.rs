//! Public programmatic, query, semantic-agent, explanation, and debug interfaces.
//!
//! The `continuation-gate` feature is a release test, not part of the
//! supported API: applications must never enable it. It shadows every
//! [`Canwu`] with a sealed run and panics on any divergence.

#![allow(clippy::missing_errors_doc, clippy::module_name_repetitions)]

pub use canwu_core::{
    ArmyId, BoundaryId, CommandAttemptId, CommandId, CommandRequestId, CoreEntityKind,
    DecisionRequestId, DecisionTicketId, DecisionTraceId, DomainEntityKindClass, DomainEntityType,
    DomainKindClass, DomainRecordKind, DomainRecordRef, DomainRecordType, DomainRecordVersionRef,
    DomainRecordVersionSource, DomainValueKindClass, DomainValueType, EntityRef, EvaluationTerm,
    EvaluationTraceRecord, EventId, EvidenceRef, GovernmentId, HolderKnowledgeRecordId, IngressId,
    KnowledgeHolderPolicy, KnowledgeHolderRef, KnowledgeRecordId, KnowledgeRecordKind,
    KnowledgeSchemaId, LetterId, OrganizationId, PersonId, RandomDrawId, ResourceId, RouteId,
    SchemaRegistry, SchemaRegistryError, SimulationGranularity, TerritoryId, TypeSchema,
    TypedDomainRecordRef,
};
pub use canwu_event::{CauseRef, EventAudience, EventKind, EventKindError, SimEvent};
pub use canwu_knowledge::{
    ActorKnowledge, ArmyKnowledge, BodiesById, CurrentRecords, EstimateRange, HolderEntriesIter,
    HolderEntry, HolderLedger, HolderLedgers, HolderLedgersIter, HolderRecordsIter,
    KNOWLEDGE_COMMITMENT_CHUNK_ENTRIES, KnowledgeBodyDigest, KnowledgeCursor,
    KnowledgeEncodingError, KnowledgeHistoryView, KnowledgeLedgerError, KnowledgeOrigin,
    KnowledgeQuery, KnowledgeQueryError, KnowledgeQueryResult, KnowledgeReadCut, KnowledgeRecord,
    KnowledgeRecordBody, KnowledgeRecordDraft, KnowledgeRecordView, KnowledgeRetention,
    KnowledgeSnapshot, KnowledgeSource, KnowledgeSubject, KnowledgeSubjectTarget,
    MAX_KNOWLEDGE_PAGE_SIZE,
};
pub use canwu_routing::{
    DepartureSlot, DurationSample, EdgeConditionEffect, EdgeConditionOverlay, EdgeConditionRef,
    EdgeConditionSet, MAX_CONDITION_PER_MILLE, MAX_EDGE_CONDITIONS,
    MAX_EDGE_CONDITIONS_PER_CONNECTION, MAX_RECURRING_OFFSETS, PlanningSnapshot,
    ROUTING_ALGORITHM_VERSION, RouteCost, RouteLeg, RoutePlan, RoutingAlgorithm, RoutingCache,
    RoutingConnection, RoutingConnectionRef, RoutingEndpoint, RoutingEndpointKind, RoutingError,
    RoutingIndex, RoutingNetwork, RoutingNodeRef, RoutingPolicy, RoutingRequest, TransferMode,
    TraversalModel, plan_route,
};
use canwu_sim::Simulation;
pub use canwu_sim::{
    ADMISSION_CURSOR_FORMAT_VERSION, ArchiveProvider, ArchiveReachabilityManifest, ArchiveStore,
    ArchiveStoreOutcome, ArchivedEvidenceLocator, ArchivedEvidenceReceipt,
    ArchivedPluginIngressProvenance, ArchivedSegmentHeader, Army, ArtifactManifest, BoundaryChange,
    BoundaryContext, BoundaryDirective, BoundaryEmission, BoundaryEmissionKind,
    BoundaryEvaluationTrace, BoundaryIngressGeneration, BoundaryKnowledgeChange,
    BoundaryPersonAvailabilityChange, BoundaryPersonCreation, BoundaryPhase, BoundaryProposal,
    BoundaryReceipt, BoundaryRecord, BoundaryRequest, BoundarySystemContract,
    BoundarySystemHandler, CHECKPOINT_JOURNAL_FORMAT_VERSION, COMMITMENT_FORMAT_VERSION,
    CONTROLLER_AUTHORITY_UNAVAILABLE_REASON, CanwuError, CheckpointJournal, Command,
    CommandAttemptOutcome, CommandAttemptRecord, CommandAuthority, CommandContext, CommandEnvelope,
    CommandIngress, CommandOutcome, CommandPolicyContext, CommandReceipt, CommandRecord,
    CommandRejection, CommandRequest, CommitmentRoots, CompactedSimulation, ControllerDecision,
    ControllerPolicy, CreatedPerson, CustodyState, DECISION_ARCHIVE_BUCKET_PAGE_FORMAT_VERSION,
    DECISION_ARCHIVE_FORMAT_VERSION, DECISION_MAKER_UNAVAILABLE_REASON,
    DECISION_REQUEST_COMMITMENT_DOMAIN, DecisionAction, DecisionArchiveBlob,
    DecisionArchiveBucketPage, DecisionArchiveProvider, DecisionArchiveReceipt,
    DecisionArchiveRecord, DecisionArchiveStore, DecisionArchiveStoreOutcome,
    DecisionAttemptErrorCode, DecisionAttemptOutcome, DecisionAttemptRecord, DecisionAuthority,
    DecisionContext, DecisionController, DecisionControllerBinding, DecisionError,
    DecisionErrorCode, DecisionEvaluation, DecisionExternalEvidence, DecisionFactorContribution,
    DecisionHistoryCursor, DecisionHistoryKey, DecisionHistoryLocation, DecisionHistoryPage,
    DecisionHistoryQueryBudget, DecisionHotState, DecisionIngressRequest,
    DecisionLocatorScaleMetrics, DecisionMutation, DecisionOption, DecisionOptionEvaluation,
    DecisionOptionWeight, DecisionOrigin, DecisionOutcome, DecisionPolicy, DecisionPolicyIdentity,
    DecisionPolicyKind, DecisionRandomEvidence, DecisionRule, DecisionStage, DecisionState,
    DecisionTicket, DecisionTicketDraft, DecisionTicketState, DecisionTrace, DemoIds, DomainRecord,
    DomainRecordChange, DomainRecordClass, DomainRecordCommitmentRoots, DomainRecordDraft,
    DomainRecordLifecycle, DomainRecordMutation, DomainRecordMutationPolicy, DomainRecordOperation,
    DomainRecordPage, DomainRecordPageRoots, DomainRecordSchema, DomainReference,
    DomainReferenceSchema, DomainReferenceTarget, DomainReferenceTargetKind, ENGINE_VERSION,
    ErrorCode, EvaluationLimitsV1, EvidenceArchiveIndex, EvidenceCursor, EvidenceIndexEntry,
    EvidenceItemLocator, EvidenceJournalKind, EvidenceJournalRoots, EvidenceJournalSegment,
    EvidenceNestedLocator, EvidenceSealToken, ExternalDecisionOption, ExternalDecisionRequest,
    ExternalDecisionResponse, ExternalPolicy, Government, GuardedUtilityPolicy, HolderPublication,
    HumanDecisionResponse, HumanPolicy, IDENTITY_EVIDENCE_DEPENDENCIES_FIELD,
    IDENTITY_EVIDENCE_DEPENDENCIES_FORMAT_VERSION, IdentityEvidenceDependenciesV1,
    IngressCancellationAuthority, IngressClass, IngressPayload, IngressReceipt, IngressRecord,
    InteractionPolicy, Issuer, KnowledgeLimitsV1, KnowledgePublicationBudget,
    KnowledgePublicationGroup, KnowledgeSubjectSchema, KnowledgeSubjectTargetKind,
    KnowledgeSupersession, KnowledgeWriteGrant, LetterCargo, LetterStatus, LifeState,
    LlmModelIdentity, LlmPolicy, MAX_DECISION_ARCHIVE_BATCH_ENTRIES,
    MAX_DECISION_HISTORY_PAGE_BYTES, MAX_DECISION_HISTORY_PAGE_SIZE,
    MAX_INGRESS_CANCELLATION_REASON_BYTES, MAX_OWNER_AUTHORIZED_MUTATIONS,
    MAX_OWNER_AUTHORIZED_PARTICIPANTS, MAX_PAGED_RECORD_BYTES, MAX_STATE_DELTA_PAGES,
    MAX_STATE_PAGE_BYTES, MaintenanceChangeRecord, MaintenanceDependencyResolverDescriptor,
    MaintenanceDisposition, MaintenanceIngressRequest, MaintenanceRejectionReceipt, MapPoint,
    OWNER_AUTHORIZED_MAINTENANCE_FORMAT_VERSION, ObservationPolicy, OrderedRulePolicy, OutboxEntry,
    OwnerAuthorizedMaintenanceDraft, OwnerAuthorizedMaintenanceParticipant,
    OwnerAuthorizedMaintenanceRequest, OwnerAuthorizedMutation, OwnerAuthorizedParticipantDraft,
    OwnerAuthorizedParticipantProposal, OwnerAuthorizedParticipantRole,
    OwnerAuthorizedRecordExpectation, PAGED_CHECKPOINT_FORMAT_VERSION,
    PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD,
    PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FORMAT_VERSION, PLUGIN_DESCRIPTOR_FORMAT_VERSION,
    PagedSimulationCheckpoint, PatriciaStoreMetrics, PayloadProperty,
    PayloadRequiredEvidenceContinuationV1, PayloadSchema, PayloadValueType,
    PersistentDomainRecordStore, Person, PersonAvailability, PersonDraft, PersonTransitState,
    PluginActionDescriptor, PluginArchiveObjectProvider, PluginArchiveReachabilityParticipant,
    PluginArchiveRetention, PluginCommandHandler, PluginComponentRecord, PluginDescriptor,
    PluginIngressDescriptor, PluginIngressPermit, PluginIngressRequest, PluginIngressTarget,
    PluginKnowledgeSchema, PluginRegistrar, PluginRegistry, PolicyDecision,
    PortablePagedSimulationCheckpoint, PreparedDecisionArchive, PreparedDecisionIngress,
    PreparedEvidenceSeal, PreparedPagedSimulationCheckpoint, PreparedStateDelta,
    QueuedExternalPolicy, QueuedHumanPolicy, QueuedLlmPolicy, RUN_CONFIGURATION_FORMAT_VERSION,
    RUN_MANIFEST_FORMAT_VERSION, RandomAlgorithm, RandomDecisionResolution, RandomDrawAddress,
    RandomDrawOutcome, RandomDrawProducer, RandomDrawRecord, RandomOperationAddressV1,
    RandomOperationTarget, RandomSample, RandomStreamKey, RandomStreamState, ReplayJournal,
    ReservationAllocation, ReservationDisposition, ReservationOffer, ReservationOfferRecord,
    ReservationPoolKey, ReservationRef, ReservationRequest, ReservationRequestRecord, Route,
    RuleChoice, RulePolicy, RunConfiguration, RunConfigurationSnapshot, RunManifest, RunPurpose,
    SNAPSHOT_FORMAT_VERSION, STATE_PAGE_CODEC, STATE_PAGE_FORMAT_VERSION,
    STATE_PAGE_VALUE_CHUNK_BYTES, STATE_REVISION_FORMAT_VERSION, Scenario, SeatBinding, SeatPolicy,
    SimulationCheckpoint, SimulationPlugin, SimulationSnapshot, SimulationSystemHandler,
    SimulationView, StateKey, StatePageBlob, StatePageProvider, StatePageRetentionHandle,
    StatePageRetentionLedger, StatePageRetentionPhase, StatePageStore, StateVisibility,
    SystemCadence, SystemContract, SystemDirective, Territory, TracePolicy, TransitState,
    UtilityEvaluator, UtilityPolicy, UtilityProfile, VerifiedDecisionArchiveCommit,
    VerifiedOwnerAuthorizedMaintenanceCommit, WeightedUtilityEvaluator, WeightedUtilityPolicy,
    WorldSnapshot, canonical_byte_hash, canonical_hash, format8_decision_locator_scale_probe,
    format8_patricia_scale_probe, identity_evidence_dependencies_property_v1,
    payload_required_evidence_continuation_property_v1, prepare_state_delta, state_page_id,
    verify_state_delta,
};
pub use canwu_sim::{
    BoundaryIngressProvenance, CONTINUATION_CHECKPOINT_FORMAT_VERSION, ContinuationBase,
    ContinuationCheckpoint, CurrentRecordVersion, EvidenceTimeRun, EvidenceTimes, HistoryFlags,
    JournalFrontier, JournalFrontiers, KeyedDrawEntry, PinnedEvidenceRecords, PinnedRecordVersion,
    PluginIngressProvenanceFact, PluginIngressStatus, RequestIndex, RetiredRequestRange,
    SettledCommandRequest, SettledDecisionRequest, SettledQueuedCommand, UnadmittedEvidence,
};
// How long a tracked command request's exact outcome answers a retry (E-28).
pub use canwu_sim::{DEFAULT_REQUEST_OUTCOME_RETENTION, MAX_REQUEST_OUTCOME_RETENTION};
// The boundaries of a window of simulated time (gap G-60).
pub use canwu_sim::{
    BoundaryIngressCount, BoundaryIngressKind, BoundaryIngressOrigin, BoundarySummary,
    BoundaryWindow, MAX_BOUNDARY_SCHEDULE_HORIZON,
};
// The opt-in wall-time profile of settled boundaries (gap G-47), with the
// simulation core's own time split by engine phase.
pub use canwu_sim::{
    BoundaryEnginePhase, BoundaryEngineTiming, BoundaryHandlerKind, BoundaryHandlerTiming,
    BoundaryProfile, BoundaryProfileBatch, MAX_RETAINED_BOUNDARY_PROFILES,
};
// An operation-keyed draw with the ID of its draw evidence (gap G-51).
pub use canwu_sim::RandomOperationDraw;
// Transition manifests and their audit (gap G-04).
pub use canwu_sim::{
    MAX_PENDING_TRANSITION_MANIFESTS, MAX_PENDING_TRANSITION_MANIFESTS_PER_COORDINATOR,
    MAX_TRANSITION_EXPECTED_VERSIONS, MAX_TRANSITION_LINEAGE_ID_BYTES, MAX_TRANSITION_PARTICIPANTS,
    MAX_TRANSITION_READY_HORIZON, PendingTransitionManifest, TransitionAuditOutcome,
    TransitionAuditRecord, TransitionManifest, TransitionManifestId, TransitionParticipant,
    TransitionParticipantAudit, TransitionRecordVersion,
};
// Persons created with or without a placement (gap G-49).
pub use canwu_sim::{BoundaryCreatedPerson, PersonPlacement};
// A holder's current records, for naming supersession targets (gap G-58).
pub use canwu_sim::KnowledgeHeads;
pub use canwu_time::{SimDuration, SimTime};
pub use canwu_transport::{
    BookingAllocationV1, CAPACITY_BOOKING_ALLOCATION_DIGEST_DOMAIN, CapacityAllocationFailureV1,
    CapacityBooking, CapacityBookingAllocationEvidenceV1, CapacityBookingId,
    CapacityBookingRequestV1, CapacityBookingStatus, DeliveryCompletionRequest, DeliverySaga,
    Handoff, HandoffId, HandoffKind, ItineraryRevision, ItineraryRevisionId,
    ItineraryRevisionReason, LegExecution, LegExecutionId, LegExecutionStatus, MovementInitiative,
    MovementOrder, MovementOrderError, MovementOrderId, MovementSubject, MovementSubjectRole,
    ReconciliationOutcome, SagaState, TRANSPORT_SEMANTIC_VERSION, TransportCapacityPoolV1,
    TransportError, TransportExecution, TransportExecutionId, TransportExecutionState,
    allocate_capacity_bookings, capacity_booking_allocation_operation_key,
    delivery_completion_operation_key,
};
use serde::{Deserialize, Serialize};

/// Main in-process API. All returned world values are detached snapshots.
///
/// `Canwu` is `Send`: a host may move a run, or a [`fork`](Canwu::fork) of
/// it, to another thread, for example to save or checkpoint it on a
/// persistence worker while the original keeps running. It is not promised to
/// be `Sync`.
pub struct Canwu {
    simulation: Simulation,
    #[cfg(feature = "continuation-gate")]
    gate: continuation_gate::Gate,
}

#[cfg(feature = "continuation-gate")]
mod continuation_gate;

/// Runs a mutation on the run, and with the continuation gate also on its
/// sealed shadow with clones of the bracketed arguments, comparing the two.
macro_rules! gated {
    ($self:ident, $name:literal, [$($arg:ident),*], |$sim:ident| $call:expr) => {{
        #[cfg(feature = "continuation-gate")]
        {
            let _ = $self.gate.prepare(&$self.simulation);
            let shadow_call = {
                $(let $arg = $arg.clone();)*
                move |$sim: &mut CompactedSimulation| $call
            };
            let result = {
                let $sim = &mut $self.simulation;
                $call
            };
            let sealed = $self.gate.shadow().map(shadow_call);
            $self.gate.check($name, &$self.simulation, &result, sealed);
            result
        }
        #[cfg(not(feature = "continuation-gate"))]
        {
            let $sim = &mut $self.simulation;
            $call
        }
    }};
}

/// Public API for a live runtime whose sealed evidence segments are stored by the caller.
///
/// Like [`Canwu`], it is `Send` and not promised to be `Sync`.
pub struct CompactedCanwu {
    simulation: CompactedSimulation,
}

impl PluginArchiveObjectProvider for Canwu {
    fn load_plugin_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, CanwuError> {
        self.plugin_archive_object(namespace, object_id)
    }
}

impl PluginArchiveObjectProvider for CompactedCanwu {
    fn load_plugin_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, CanwuError> {
        self.plugin_archive_object(namespace, object_id)
    }
}

/// The reads and canonical ingress that host-facing helpers use on one
/// simulation run, shared by [`Canwu`] and [`CompactedCanwu`].
///
/// It is a read-and-ingress contract: it settles nothing, admits nothing,
/// and changes no state except by queuing canonical ingress, which is checked
/// at admission and again at settlement as from either handle. Its reads are
/// detached trusted-host reads; do not pass them to player, agent, observer,
/// or remote clients. Every method behaves as the method of the same name on
/// the handle. Typed record reads are free functions over it, such as
/// [`typed_domain_record`], so the trait stays usable as `dyn CanwuRun`.
///
/// Extension helpers take `&H` or `&mut H` with `H: CanwuRun + ?Sized`, so a
/// host drives them with either handle.
pub trait CanwuRun {
    /// The committed simulation time.
    fn time(&self) -> SimTime;
    /// The persisted authoritative transaction revision.
    fn revision(&self) -> u64;
    /// The checkpoint hash of the current state.
    fn checkpoint_hash(&self) -> &str;
    /// Whether an entity is part of the run.
    fn entity_exists(&self, entity: &EntityRef) -> bool;
    /// The current version of one domain record.
    fn domain_record(&self, reference: &DomainRecordRef) -> Option<&DomainRecord>;
    /// Every current domain record.
    fn domain_records(&self) -> Box<dyn Iterator<Item = &DomainRecord> + '_>;
    /// The exact identity of a record's current version.
    fn current_domain_record_version(
        &self,
        reference: &DomainRecordRef,
    ) -> Result<Option<DomainRecordVersionRef>, CanwuError>;
    /// Whether an exact domain-record version exists in current, retained,
    /// or kept evidence.
    ///
    /// On a sealed run a superseded version that is not declared as a
    /// dependency is known only while an earlier seal kept its receipt, so the
    /// answer can differ between a sealed run and an unsealed one; a rule that
    /// must decide alike in both cites a current version or declares the
    /// version as an identity-only dependency.
    fn domain_record_version_evidence_exists(&self, reference: &DomainRecordVersionRef) -> bool;
    /// The body and establishment time of a record's committed current
    /// version or of an initial-scenario version; `None` for any other
    /// version, even a retained one, so a seal cannot change the answer.
    fn replay_stable_domain_record_version(
        &self,
        reference: &DomainRecordVersionRef,
    ) -> Option<(DomainRecord, SimTime)>;
    /// How many items each evidence journal holds, sealed and retained
    /// together.
    fn evidence_cursor(&self) -> Result<EvidenceCursor, CanwuError>;
    /// The cut through the evidence the run no longer retains (where its
    /// last seal ended), or `None` while it retains all of it; see
    /// [`CompactedCanwu::sealed_evidence_cursor`].
    fn sealed_evidence_cursor(&self) -> Option<EvidenceCursor>;
    /// The boundaries of the half-open window `from..to`.
    fn boundaries_between(&self, from: SimTime, to: SimTime) -> Result<BoundaryWindow, CanwuError>;
    /// How far back the run keeps settled boundaries.
    fn boundary_schedule_horizon(&self) -> SimDuration;
    /// Queues a public plugin packet.
    fn enqueue_plugin_ingress(
        &mut self,
        request: PluginIngressRequest,
    ) -> Result<IngressReceipt, CanwuError>;
    /// Queues a plugin packet of an internal packet type through its owning
    /// plugin's opaque registration permit.
    fn enqueue_permitted_plugin_ingress(
        &mut self,
        request: PluginIngressRequest,
        permit: &PluginIngressPermit,
    ) -> Result<IngressReceipt, CanwuError>;
    /// Trusted host/admin query of one holder's knowledge, as
    /// [`Canwu::admin_query_knowledge`] does. Player-facing callers must use a
    /// restricted [`CanwuViewer`].
    fn admin_query_knowledge(
        &self,
        holder: KnowledgeHolderRef,
        query: &KnowledgeQuery,
    ) -> Result<KnowledgeQueryResult, CanwuError>;
}

macro_rules! impl_canwu_run {
    ($handle:ty) => {
        impl CanwuRun for $handle {
            fn time(&self) -> SimTime {
                Self::time(self)
            }
            fn revision(&self) -> u64 {
                Self::revision(self)
            }
            fn checkpoint_hash(&self) -> &str {
                Self::checkpoint_hash(self)
            }
            fn entity_exists(&self, entity: &EntityRef) -> bool {
                Self::entity_exists(self, entity)
            }
            fn domain_record(&self, reference: &DomainRecordRef) -> Option<&DomainRecord> {
                Self::domain_record(self, reference)
            }
            fn domain_records(&self) -> Box<dyn Iterator<Item = &DomainRecord> + '_> {
                Box::new(Self::domain_records(self))
            }
            fn current_domain_record_version(
                &self,
                reference: &DomainRecordRef,
            ) -> Result<Option<DomainRecordVersionRef>, CanwuError> {
                Self::current_domain_record_version(self, reference)
            }
            fn domain_record_version_evidence_exists(
                &self,
                reference: &DomainRecordVersionRef,
            ) -> bool {
                Self::domain_record_version_evidence_exists(self, reference)
            }
            fn replay_stable_domain_record_version(
                &self,
                reference: &DomainRecordVersionRef,
            ) -> Option<(DomainRecord, SimTime)> {
                Self::replay_stable_domain_record_version(self, reference)
            }
            fn evidence_cursor(&self) -> Result<EvidenceCursor, CanwuError> {
                Self::evidence_cursor(self)
            }
            fn sealed_evidence_cursor(&self) -> Option<EvidenceCursor> {
                Self::sealed_evidence_cursor(self)
            }
            fn boundaries_between(
                &self,
                from: SimTime,
                to: SimTime,
            ) -> Result<BoundaryWindow, CanwuError> {
                Self::boundaries_between(self, from, to)
            }
            fn boundary_schedule_horizon(&self) -> SimDuration {
                Self::boundary_schedule_horizon(self)
            }
            fn enqueue_plugin_ingress(
                &mut self,
                request: PluginIngressRequest,
            ) -> Result<IngressReceipt, CanwuError> {
                Self::enqueue_plugin_ingress(self, request)
            }
            fn enqueue_permitted_plugin_ingress(
                &mut self,
                request: PluginIngressRequest,
                permit: &PluginIngressPermit,
            ) -> Result<IngressReceipt, CanwuError> {
                Self::enqueue_permitted_plugin_ingress(self, request, permit)
            }
            fn admin_query_knowledge(
                &self,
                holder: KnowledgeHolderRef,
                query: &KnowledgeQuery,
            ) -> Result<KnowledgeQueryResult, CanwuError> {
                Self::admin_query_knowledge(self, holder, query)
            }
        }
    };
}

impl_canwu_run!(Canwu);
impl_canwu_run!(CompactedCanwu);

/// The current version of a typed domain record, from either handle.
pub fn typed_domain_record<'a, T: DomainRecordType, R: CanwuRun + ?Sized>(
    run: &'a R,
    reference: &TypedDomainRecordRef<T>,
) -> Option<&'a DomainRecord> {
    run.domain_record(reference.as_untyped())
}

impl Canwu {
    fn wrap(simulation: Simulation) -> Self {
        Self {
            simulation,
            #[cfg(feature = "continuation-gate")]
            gate: continuation_gate::Gate::default(),
        }
    }

    #[must_use]
    pub const fn version() -> &'static str {
        ENGINE_VERSION
    }

    pub fn new(seed: u64, scenario: Scenario) -> Result<Self, CanwuError> {
        Ok(Self::wrap(Simulation::new(seed, scenario)?))
    }

    /// Enters the explicit compact-journal interface without discarding evidence.
    pub fn into_compacted(self) -> Result<CompactedCanwu, CanwuError> {
        Ok(CompactedCanwu {
            simulation: self.simulation.into_compacted()?,
        })
    }

    pub fn new_with_plugins(
        seed: u64,
        scenario: Scenario,
        plugins: &[&dyn SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        Ok(Self::wrap(Simulation::new_with_plugins(
            seed, scenario, plugins,
        )?))
    }

    pub fn new_with_manifest(
        seed: u64,
        scenario: Scenario,
        run_manifest: RunManifest,
    ) -> Result<Self, CanwuError> {
        Ok(Self::wrap(Simulation::new_with_manifest(
            seed,
            scenario,
            run_manifest,
        )?))
    }

    pub fn new_with_manifest_and_plugins(
        seed: u64,
        scenario: Scenario,
        run_manifest: RunManifest,
        plugins: &[&dyn SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        Ok(Self::wrap(Simulation::new_with_manifest_and_plugins(
            seed,
            scenario,
            run_manifest,
            plugins,
        )?))
    }

    pub fn new_with_run_configuration(
        seed: u64,
        scenario: Scenario,
        run_manifest: RunManifest,
        run_configuration: RunConfiguration,
    ) -> Result<Self, CanwuError> {
        Ok(Self::wrap(Simulation::new_with_run_configuration(
            seed,
            scenario,
            run_manifest,
            run_configuration,
        )?))
    }

    pub fn new_with_run_configuration_and_plugins(
        seed: u64,
        scenario: Scenario,
        run_manifest: RunManifest,
        run_configuration: RunConfiguration,
        plugins: &[&dyn SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        Ok(Self::wrap(
            Simulation::new_with_run_configuration_and_plugins(
                seed,
                scenario,
                run_manifest,
                run_configuration,
                plugins,
            )?,
        ))
    }

    /// Deprecated compatibility scenario. New hosts should use an integration-owned scenario.
    pub fn demo(seed: u64) -> Result<Self, CanwuError> {
        let (simulation, _) = Simulation::demo(seed)?;
        Ok(Self::wrap(simulation))
    }

    /// IDs for the deprecated compatibility scenario.
    #[must_use]
    pub fn demo_ids() -> DemoIds {
        let (_, ids) = canwu_sim::demo_scenario();
        ids
    }

    #[must_use]
    pub const fn time(&self) -> SimTime {
        self.simulation.time()
    }

    #[must_use]
    pub const fn run_manifest(&self) -> &RunManifest {
        self.simulation.run_manifest()
    }

    #[must_use]
    pub const fn run_configuration(&self) -> &RunConfigurationSnapshot {
        self.simulation.run_configuration()
    }

    #[must_use]
    /// Returns the persisted authoritative transaction revision.
    ///
    /// Accepted commands, persisted expected rejections, and completed
    /// settlement boundaries each advance it exactly once. Failed work, exact
    /// retries, bare clock movement, queued but unadmitted ingress, and plugin
    /// setup do not advance it; combine it with command expected-time guards.
    pub fn revision(&self) -> u64 {
        self.simulation.revision()
    }

    #[must_use]
    pub fn run_manifest_hash(&self) -> &str {
        self.simulation.run_manifest_hash()
    }

    #[must_use]
    pub fn checkpoint_hash(&self) -> &str {
        self.simulation.checkpoint_hash()
    }

    pub fn authoritative_state_hash(&self) -> Result<String, CanwuError> {
        self.simulation.authoritative_state_hash()
    }

    pub fn entities(&self) -> impl Iterator<Item = &EntityRef> {
        self.simulation.entities()
    }

    #[must_use]
    pub fn entity_exists(&self, entity: &EntityRef) -> bool {
        self.simulation.entity_exists(entity)
    }

    /// Deprecated detached format-5 compatibility projection.
    #[must_use]
    pub fn world(&self) -> WorldSnapshot {
        self.simulation.world()
    }

    /// Trusted host access to a person's committed life and custody state.
    /// `None` means no change has been committed: the person is alive and free.
    #[must_use]
    pub fn person_availability(&self, person: PersonId) -> Option<&PersonAvailability> {
        self.simulation.person_availability(person)
    }

    /// Trusted host access to every committed person availability, in
    /// person-ID order. Persons without an entry are alive and free.
    pub fn person_availabilities(&self) -> impl Iterator<Item = (&PersonId, &PersonAvailability)> {
        self.simulation.person_availabilities()
    }

    /// Trusted host access to the registered transition manifests whose
    /// ready boundary has not settled, in manifest-ID order. Settled
    /// manifests leave their audit on the boundary record and receipt.
    pub fn pending_transition_manifests(&self) -> impl Iterator<Item = &PendingTransitionManifest> {
        self.simulation.pending_transition_manifests()
    }

    /// Trusted host access to the boundaries of the half-open window
    /// `from..to`: those settled in it and those still queued in it. Settled
    /// boundaries are kept for the longest horizon a registered boundary
    /// system declares. Boundary systems read the same ledger through
    /// [`SimulationView::boundaries_between`] after declaring
    /// [`StateKey::core_boundary_schedule`].
    ///
    /// The result names every plugin's queued packet types. Do not expose it
    /// to player, agent, observer, or remote clients.
    pub fn boundaries_between(
        &self,
        from: SimTime,
        to: SimTime,
    ) -> Result<BoundaryWindow, CanwuError> {
        self.simulation.boundaries_between(from, to)
    }

    /// How far back the run keeps settled boundaries: the longest
    /// [`BoundarySystemContract::boundary_schedule_horizon`] a registered
    /// boundary system declares, or zero.
    #[must_use]
    pub fn boundary_schedule_horizon(&self) -> SimDuration {
        self.simulation.boundary_schedule_horizon()
    }

    /// Trusted host/admin access to the complete knowledge snapshot.
    ///
    /// Do not expose this public API to player, agent, observer, or remote clients;
    /// use [`Canwu::viewer`] or [`Canwu::viewer_for_actor`] instead.
    #[must_use]
    pub fn knowledge(&self) -> &KnowledgeSnapshot {
        self.simulation.knowledge()
    }

    #[must_use]
    pub fn events(&self) -> &[SimEvent] {
        self.simulation.events()
    }

    #[must_use]
    pub fn commands(&self) -> &[CommandRecord] {
        self.simulation.command_log()
    }

    #[must_use]
    pub fn boundaries(&self) -> &[BoundaryRecord] {
        self.simulation.boundaries()
    }

    #[must_use]
    pub fn command_attempts(&self) -> &[CommandAttemptRecord] {
        self.simulation.command_attempts()
    }

    #[must_use]
    pub fn ingress_log(&self) -> &[IngressRecord] {
        self.simulation.ingress_log()
    }

    #[must_use]
    pub fn domain_record(&self, reference: &DomainRecordRef) -> Option<&DomainRecord> {
        self.simulation.domain_record(reference)
    }

    /// Returns whether an exact domain-record version exists in current or retained evidence.
    #[must_use]
    pub fn domain_record_version_evidence_exists(
        &self,
        reference: &DomainRecordVersionRef,
    ) -> bool {
        self.simulation
            .domain_record_version_evidence_exists(reference)
    }

    /// Returns whether a generic evidence identity is retained or archived.
    #[must_use]
    pub fn evidence_exists(&self, reference: &EvidenceRef) -> bool {
        self.simulation.evidence_exists(reference)
    }

    /// Returns when evidence first became authoritative: a retained record's
    /// own time, the sealed time index's time for a sealed ordinal record,
    /// and a domain-record version's establishing time whenever the version
    /// is known to exist.
    #[must_use]
    pub fn evidence_time(&self, reference: &EvidenceRef) -> Option<SimTime> {
        self.simulation.evidence_time(reference)
    }

    /// Resolves the retained record body for one exact domain-record version.
    ///
    /// A compacted archive receipt proves existence but does not expose the
    /// version body through this trusted-host query. The current version
    /// resolves to its live body.
    #[must_use]
    pub fn domain_record_version(
        &self,
        reference: &DomainRecordVersionRef,
    ) -> Option<DomainRecord> {
        self.simulation.domain_record_version(reference)
    }

    /// Resolves an exact domain-record version whose body and establishment
    /// time a seal cannot change: the committed current version of its
    /// record, or an initial-scenario version. Any other version returns
    /// `None`, even while it is retained.
    ///
    /// A host check that must give the same answer on a sealed run, an
    /// unsealed fork, a restore, and an exact replay reads versions through
    /// this, as a boundary system reads them through
    /// [`SimulationView::replay_stable_domain_record_version`].
    #[must_use]
    pub fn replay_stable_domain_record_version(
        &self,
        reference: &DomainRecordVersionRef,
    ) -> Option<(DomainRecord, SimTime)> {
        self.simulation
            .replay_stable_domain_record_version(reference)
    }

    /// Returns the retained or archive-resolvable exact identity for the
    /// authoritative current record version. Missing provenance fails closed.
    pub fn current_domain_record_version(
        &self,
        reference: &DomainRecordRef,
    ) -> Result<Option<DomainRecordVersionRef>, CanwuError> {
        self.simulation.current_domain_record_version(reference)
    }

    #[must_use]
    pub fn typed_domain_record<T: DomainRecordType>(
        &self,
        reference: &TypedDomainRecordRef<T>,
    ) -> Option<&DomainRecord> {
        self.simulation.typed_domain_record(reference)
    }

    pub fn domain_records(&self) -> impl Iterator<Item = &DomainRecord> {
        self.simulation.domain_records()
    }

    /// Returns one trusted-host page bound to an authoritative revision.
    ///
    /// Use the returned revision as `expected_revision` on subsequent pages.
    pub fn domain_record_page(
        &self,
        kind: &DomainRecordKind,
        after: Option<&DomainRecordRef>,
        limit: usize,
        expected_revision: Option<u64>,
    ) -> Result<DomainRecordPage, CanwuError> {
        self.simulation
            .domain_record_page(kind, after, limit, expected_revision)
    }

    #[must_use]
    pub fn decision_ticket(&self, id: DecisionTicketId) -> Option<&DecisionTicket> {
        self.simulation.decision_ticket(id)
    }

    #[must_use]
    pub fn decision_controller(&self, id: &str) -> Option<&DecisionControllerBinding> {
        self.simulation.decision_controller(id)
    }

    #[must_use]
    pub fn decision_trace(&self, id: DecisionTraceId) -> Option<&DecisionTrace> {
        self.simulation.decision_trace(id)
    }

    #[must_use]
    pub fn decision_attempt(&self, id: DecisionRequestId) -> Option<&DecisionAttemptRecord> {
        self.simulation.decision_attempt(id)
    }

    #[must_use]
    pub fn decision_hot_state(&self) -> DecisionHotState {
        self.simulation.decision_hot_state()
    }

    #[must_use]
    pub fn decision_history_location(&self, key: &DecisionHistoryKey) -> DecisionHistoryLocation {
        self.simulation.decision_history_location(key)
    }

    pub fn decision_history_location_with_provider(
        &self,
        key: &DecisionHistoryKey,
        provider: &dyn DecisionArchiveProvider,
    ) -> Result<DecisionHistoryLocation, CanwuError> {
        self.simulation
            .decision_history_location_with_provider(key, provider)
    }

    #[must_use]
    pub fn random_draws(&self) -> &[RandomDrawRecord] {
        self.simulation.random_draws()
    }

    #[must_use]
    pub fn boundary_head_hash(&self) -> Option<&str> {
        self.simulation.boundary_head_hash()
    }

    #[must_use]
    pub const fn schema(&self) -> &SchemaRegistry {
        self.simulation.schema()
    }

    pub fn plugin_descriptors(&self) -> impl Iterator<Item = &PluginDescriptor> {
        self.simulation.plugin_descriptors()
    }

    #[must_use]
    pub fn replay_journal(&self) -> ReplayJournal {
        self.simulation.replay_journal()
    }

    pub fn outbox_entries(&self) -> Result<Vec<OutboxEntry>, CanwuError> {
        self.simulation.outbox_entries()
    }

    pub fn evidence_cursor(&self) -> Result<EvidenceCursor, CanwuError> {
        self.simulation.evidence_cursor()
    }

    /// The cut through the evidence this run no longer retains, or `None`
    /// while it retains all its evidence, as a run of this handle does; a
    /// run seals through [`CompactedCanwu`]. See
    /// [`CompactedCanwu::sealed_evidence_cursor`].
    #[must_use]
    pub fn sealed_evidence_cursor(&self) -> Option<EvidenceCursor> {
        self.simulation.sealed_evidence_cursor()
    }

    pub fn checkpoint(&self) -> Result<SimulationCheckpoint, CanwuError> {
        self.simulation.checkpoint()
    }

    /// Builds the continuation checkpoint of this run at its settled cut and
    /// returns it with its continuation hash, which the host stores as the
    /// trust anchor for [`CompactedCanwu::from_continuation_checkpoint_verified`].
    ///
    /// It equals the checkpoint any sealed copy of this run writes at the
    /// same cut. Building changes nothing.
    pub fn continuation_checkpoint(&self) -> Result<(ContinuationCheckpoint, String), CanwuError> {
        self.simulation.continuation_checkpoint()
    }

    pub fn archive_reachability_manifest(
        &self,
        retained_checkpoints: &[SimulationCheckpoint],
        page_retention: &StatePageRetentionLedger,
        decision_provider: &dyn DecisionArchiveProvider,
        plugin_provider: &dyn PluginArchiveObjectProvider,
    ) -> Result<ArchiveReachabilityManifest, CanwuError> {
        self.simulation.archive_reachability_manifest(
            retained_checkpoints,
            page_retention,
            decision_provider,
            plugin_provider,
        )
    }

    pub fn journal_segment_since(
        &self,
        start: EvidenceCursor,
    ) -> Result<EvidenceJournalSegment, CanwuError> {
        self.simulation.journal_segment_since(start)
    }

    pub fn checkpoint_journal(&self) -> Result<CheckpointJournal, CanwuError> {
        self.simulation.checkpoint_journal()
    }

    pub fn checkpoint_journal_json(&self) -> Result<String, CanwuError> {
        self.simulation.checkpoint_journal_json()
    }

    /// Prepares a content-addressed paged checkpoint of current state without
    /// changing authoritative state, the revision, or any journal.
    ///
    /// The checkpoint carries no append-only evidence; it names the evidence
    /// cut that [`Self::evidence_cursor`] reports. Pages already readable
    /// from `provider` are left out of the returned delta, so a provider that
    /// holds an earlier checkpoint's pages receives only the changed ones. A
    /// runtime restored from a paged checkpoint also leaves out the archived
    /// decision pages it never loaded, whether or not `provider` holds them,
    /// so `provider` must already hold every page of the checkpoint it was
    /// restored from. `source`, when given, is the earlier checkpoint whose
    /// root the delta continues and must be readable from `provider`.
    /// [`PreparedPagedSimulationCheckpoint::store_and_verify`] stores the delta
    /// but checks only its new pages and the root; check the checkpoint's
    /// complete page set with [`StatePageRetentionLedger::prepare`] and
    /// [`StatePageRetentionLedger::verify`] before relying on it.
    ///
    /// A domain record longer than [`STATE_PAGE_VALUE_CHUNK_BYTES`] is stored
    /// as record-value chunk pages that its leaf names, so a record of any
    /// length up to [`MAX_PAGED_RECORD_BYTES`] can be paged; a longer one
    /// fails the preparation with `value_out_of_range` and nothing else.
    pub fn prepare_paged_checkpoint(
        &self,
        source: Option<&PagedSimulationCheckpoint>,
        provider: &dyn StatePageProvider,
    ) -> Result<PreparedPagedSimulationCheckpoint, CanwuError> {
        self.simulation.prepare_paged_checkpoint(source, provider)
    }

    pub fn register_plugin<P: SimulationPlugin + ?Sized>(
        &mut self,
        plugin: &P,
    ) -> Result<(), CanwuError> {
        #[cfg(feature = "continuation-gate")]
        self.gate.drop_shadow();
        self.simulation.register_plugin(plugin)
    }

    /// Attaches caller-owned package archive storage for authenticated cold
    /// history resolution during normal admissions, settlement, and queries.
    ///
    /// The provider must be `Send + Sync`: [`Canwu::fork`] shares it, and a
    /// fork may move to another thread and load from it there while the
    /// original keeps loading on its own thread.
    pub fn set_plugin_archive_object_provider(
        &mut self,
        provider: std::sync::Arc<dyn PluginArchiveObjectProvider + Send + Sync>,
    ) {
        #[cfg(feature = "continuation-gate")]
        let provider = self.gate.wrap_provider(provider);
        self.simulation.set_plugin_archive_object_provider(provider);
    }

    /// Loads one opaque package archive object from the attached provider.
    /// Package integrations authenticate the bytes against their committed
    /// archive roots before use.
    pub fn plugin_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, CanwuError> {
        self.simulation.plugin_archive_object(namespace, object_id)
    }

    pub fn submit(&mut self, command: CommandEnvelope) -> Result<CommandReceipt, CanwuError> {
        gated!(self, "submit", [command], |sim| sim.submit(command))
    }

    pub fn process_command(
        &mut self,
        request: CommandRequest,
    ) -> Result<CommandOutcome, CanwuError> {
        gated!(self, "process_command", [request], |sim| sim
            .process_command(request))
    }

    pub fn enqueue_command(
        &mut self,
        due_at: SimTime,
        priority: i32,
        request: CommandRequest,
    ) -> Result<IngressReceipt, CanwuError> {
        gated!(self, "enqueue_command", [request], |sim| sim
            .enqueue_command(due_at, priority, request))
    }

    pub fn enqueue_plugin_ingress(
        &mut self,
        request: PluginIngressRequest,
    ) -> Result<IngressReceipt, CanwuError> {
        gated!(self, "enqueue_plugin_ingress", [request], |sim| sim
            .enqueue_plugin_ingress(request))
    }

    pub fn enqueue_permitted_plugin_ingress(
        &mut self,
        request: PluginIngressRequest,
        permit: &PluginIngressPermit,
    ) -> Result<IngressReceipt, CanwuError> {
        gated!(self, "enqueue_permitted_plugin_ingress", [request], |sim| {
            sim.enqueue_permitted_plugin_ingress(request, permit)
        })
    }

    /// Withdraws a still-pending plugin ingress item that the host enqueued
    /// with [`Self::enqueue_plugin_ingress`], strictly before its due time.
    ///
    /// Due, admitted, archived, or already cancelled items fail with
    /// [`ErrorCode::LateIngress`]; items of internal packet types or items a
    /// plugin scheduled inside the engine fail with
    /// [`ErrorCode::InvalidAuthority`]; unknown IDs fail with
    /// [`ErrorCode::EvidenceUnavailable`]; non-plugin targets and reasons that
    /// are empty, untrimmed, or longer than
    /// [`MAX_INGRESS_CANCELLATION_REASON_BYTES`] fail with
    /// [`ErrorCode::InvalidPayload`]; declared read-only runs fail with
    /// [`ErrorCode::InteractionReadOnly`]. The returned receipt names the
    /// terminal [`IngressPayload::PluginCancellation`] journal record. The
    /// withdrawn item is never admitted, never settles, and nothing is rolled
    /// back; snapshots, checkpoint journals, and exact replay preserve the
    /// cancellation.
    pub fn cancel_plugin_ingress(
        &mut self,
        ingress_id: IngressId,
        reason: impl Into<String>,
    ) -> Result<IngressReceipt, CanwuError> {
        let reason: String = reason.into();
        gated!(self, "cancel_plugin_ingress", [reason], |sim| sim
            .cancel_plugin_ingress(ingress_id, reason))
    }

    /// Withdraws a still-pending item of an internal packet type through the
    /// owning plugin's opaque registration permit. The permit covers
    /// host-enqueued items of that exact type and items the same plugin
    /// scheduled inside the engine; timing rules match
    /// [`Self::cancel_plugin_ingress`].
    pub fn cancel_permitted_plugin_ingress(
        &mut self,
        ingress_id: IngressId,
        permit: &PluginIngressPermit,
        reason: impl Into<String>,
    ) -> Result<IngressReceipt, CanwuError> {
        let reason: String = reason.into();
        gated!(self, "cancel_permitted_plugin_ingress", [reason], |sim| sim
            .cancel_permitted_plugin_ingress(ingress_id, permit, reason))
    }

    pub fn prepare_decision(
        &self,
        decision_request_id: DecisionRequestId,
        command_request_id: Option<CommandRequestId>,
        ticket_id: DecisionTicketId,
        policy: &dyn DecisionPolicy,
    ) -> Result<DecisionEvaluation, CanwuError> {
        self.simulation
            .prepare_decision(decision_request_id, command_request_id, ticket_id, policy)
    }

    pub fn prepare_decision_at(
        &self,
        due_at: SimTime,
        decision_request_id: DecisionRequestId,
        command_request_id: Option<CommandRequestId>,
        ticket_id: DecisionTicketId,
        policy: &dyn DecisionPolicy,
    ) -> Result<DecisionEvaluation, CanwuError> {
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
        due_at: SimTime,
        priority: i32,
        request: DecisionIngressRequest,
    ) -> Result<IngressReceipt, CanwuError> {
        gated!(self, "enqueue_decision", [request], |sim| sim
            .enqueue_decision(due_at, priority, request))
    }

    pub fn drive_decision(
        &mut self,
        due_at: SimTime,
        priority: i32,
        decision_request_id: DecisionRequestId,
        command_request_id: Option<CommandRequestId>,
        ticket_id: DecisionTicketId,
        policy: &dyn DecisionPolicy,
    ) -> Result<DecisionEvaluation, CanwuError> {
        gated!(self, "drive_decision", [], |sim| sim.drive_decision(
            due_at,
            priority,
            decision_request_id,
            command_request_id,
            ticket_id,
            policy,
        ))
    }

    pub fn schedule_calendar_boundary(
        &mut self,
        due_at: SimTime,
        cadences: Vec<SystemCadence>,
    ) -> Result<IngressReceipt, CanwuError> {
        gated!(self, "schedule_calendar_boundary", [cadences], |sim| sim
            .schedule_calendar_boundary(due_at, cadences))
    }

    pub fn advance_canonical(
        &mut self,
        duration: SimDuration,
    ) -> Result<Vec<BoundaryReceipt>, CanwuError> {
        gated!(self, "advance_canonical", [], |sim| sim
            .advance_canonical(duration))
    }

    pub fn step_canonical(&mut self) -> Result<Option<BoundaryReceipt>, CanwuError> {
        gated!(self, "step_canonical", [], |sim| sim.step_canonical())
    }

    pub fn advance(&mut self, duration: SimDuration) -> Result<Vec<SimEvent>, CanwuError> {
        gated!(self, "advance", [], |sim| sim.advance(duration))
    }

    pub fn settle_boundary(
        &mut self,
        request: BoundaryRequest,
    ) -> Result<BoundaryReceipt, CanwuError> {
        gated!(self, "settle_boundary", [request], |sim| sim
            .settle_boundary(request))
    }

    /// Turns the wall-time profile of settled boundaries on or off. It is off
    /// in a new, restored, replayed, or forked runtime.
    ///
    /// While it is on, every boundary that settles leaves one
    /// [`BoundaryProfile`] for [`Self::take_boundary_profiles`]: the wall
    /// time of the settlement and of each plugin handler it ran, which are
    /// its boundary systems by phase, the plugin command handlers of the
    /// queued commands it admitted, and the event-driven plugin systems those
    /// commands and the scheduled actions due at the boundary triggered. A
    /// boundary that fails leaves none. Owner-authorized maintenance
    /// participants run when the host schedules the maintenance, outside
    /// settlement, and are in no profile.
    ///
    /// An exact replay settles its boundaries inside the replay call, on a
    /// runtime that starts with profiling off, so a replay is not profiled;
    /// the runtime it returns can be profiled from then on.
    ///
    /// Profiling is a host diagnostic. It reads the wall clock and changes
    /// nothing else: receipts, evidence, hashes, snapshots, checkpoints, and
    /// exact replay are the same with it on or off, and no plugin can read a
    /// profile. While it is off, settlement reads no clock. Do not feed
    /// profile values back into the simulation.
    pub fn set_boundary_profiling(&mut self, enabled: bool) {
        self.simulation.set_boundary_profiling(enabled);
    }

    /// Whether settled boundaries are being profiled.
    #[must_use]
    pub const fn boundary_profiling(&self) -> bool {
        self.simulation.boundary_profiling()
    }

    /// Takes the profiles of the boundaries settled since the last call, in
    /// settlement order. The runtime keeps at most
    /// [`MAX_RETAINED_BOUNDARY_PROFILES`] profiles that were not taken yet;
    /// [`BoundaryProfileBatch::dropped`] counts the older ones it dropped
    /// since the last call.
    pub fn take_boundary_profiles(&mut self) -> BoundaryProfileBatch {
        self.simulation.take_boundary_profiles()
    }

    pub fn wait(&mut self, duration: SimDuration) -> Result<Vec<SimEvent>, CanwuError> {
        self.advance(duration)
    }

    pub fn step(&mut self) -> Result<Vec<SimEvent>, CanwuError> {
        #[cfg(feature = "continuation-gate")]
        self.gate.drop_shadow();
        self.simulation.step()
    }

    #[must_use]
    pub fn snapshot(&self) -> SimulationSnapshot {
        self.simulation.snapshot()
    }

    pub fn snapshot_json(&self) -> Result<String, CanwuError> {
        self.simulation.snapshot_json()
    }

    pub fn from_snapshot_json(json: &str) -> Result<Self, CanwuError> {
        let simulation = Simulation::from_snapshot_json(json)?;
        Ok(Self::wrap(simulation))
    }

    pub fn from_snapshot_json_with_plugins(
        json: &str,
        plugins: &[&dyn SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        Ok(Self::wrap(Simulation::from_snapshot_json_with_plugins(
            json, plugins,
        )?))
    }

    pub fn from_checkpoint_and_journal(
        checkpoint: SimulationCheckpoint,
        segments: Vec<EvidenceJournalSegment>,
    ) -> Result<Self, CanwuError> {
        Ok(Self::wrap(Simulation::from_checkpoint_and_journal(
            checkpoint, segments,
        )?))
    }

    pub fn from_checkpoint_journal(bundle: CheckpointJournal) -> Result<Self, CanwuError> {
        Ok(Self::wrap(Simulation::from_checkpoint_journal(bundle)?))
    }

    pub fn from_checkpoint_journal_with_plugins(
        bundle: CheckpointJournal,
        plugins: &[&dyn SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        Ok(Self::wrap(
            Simulation::from_checkpoint_journal_with_plugins(bundle, plugins)?,
        ))
    }

    pub fn from_checkpoint_journal_json(json: &str) -> Result<Self, CanwuError> {
        Ok(Self::wrap(Simulation::from_checkpoint_journal_json(json)?))
    }

    pub fn from_checkpoint_journal_json_with_plugins(
        json: &str,
        plugins: &[&dyn SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        Ok(Self::wrap(
            Simulation::from_checkpoint_journal_json_with_plugins(json, plugins)?,
        ))
    }

    /// Restores a paged checkpoint after proving the contiguous evidence
    /// prefix it names.
    ///
    /// `segments` must start at the zero cut, stay contiguous, advance at least
    /// one journal each, and end exactly at the checkpoint's evidence cut, so
    /// skip a segment whose start equals its end; an empty list is valid only
    /// for a checkpoint taken before any evidence was recorded. A page the
    /// restore reads that is missing from `provider` fails with
    /// [`ErrorCode::StatePageUnavailable`] instead of reading as absent state.
    /// The restore reads only the archived decision bucket pages that hot
    /// decision state depends on; history in any other bucket reports
    /// [`DecisionHistoryLocation::Unresolved`] until
    /// [`Self::decision_history_location_with_provider`] resolves it through a
    /// [`DecisionArchiveProvider`] that serves the same bucket pages.
    /// Continuation stays blocked until every plugin the checkpoint names is
    /// registered; use [`Self::from_paged_checkpoint_and_journal_with_plugins`]
    /// to do both.
    pub fn from_paged_checkpoint_and_journal(
        checkpoint: &PagedSimulationCheckpoint,
        provider: &dyn StatePageProvider,
        segments: Vec<EvidenceJournalSegment>,
    ) -> Result<Self, CanwuError> {
        Ok(Self::wrap(Simulation::from_paged_checkpoint_and_journal(
            checkpoint, provider, segments,
        )?))
    }

    /// Restores a paged checkpoint and its evidence prefix, then rehydrates
    /// the exact executable plugins it names.
    pub fn from_paged_checkpoint_and_journal_with_plugins(
        checkpoint: &PagedSimulationCheckpoint,
        provider: &dyn StatePageProvider,
        segments: Vec<EvidenceJournalSegment>,
        plugins: &[&dyn SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        Ok(Self::wrap(
            Simulation::from_paged_checkpoint_and_journal_with_plugins(
                checkpoint, provider, segments, plugins,
            )?,
        ))
    }

    pub fn replay_from_journal(
        plugins: &[&dyn SimulationPlugin],
        journal: &ReplayJournal,
    ) -> Result<Self, CanwuError> {
        let simulation = Simulation::replay_from_journal(plugins, journal)?;
        Ok(Self::wrap(simulation))
    }

    /// Replays with package archive storage attached before the first
    /// recorded boundary, so package-owned cold history participates in
    /// ordinary admission and settlement throughout replay. The provider must
    /// be `Send + Sync`, as for [`Canwu::set_plugin_archive_object_provider`].
    pub fn replay_from_journal_with_archive_provider(
        plugins: &[&dyn SimulationPlugin],
        journal: &ReplayJournal,
        archive_provider: std::sync::Arc<dyn PluginArchiveObjectProvider + Send + Sync>,
    ) -> Result<Self, CanwuError> {
        let simulation = Simulation::replay_from_journal_with_archive_provider(
            plugins,
            journal,
            archive_provider,
        )?;
        Ok(Self::wrap(simulation))
    }

    /// Replays exactly, as [`Self::replay_from_journal_with_archive_provider`]
    /// does, with the boundary profile on from before the first recorded
    /// boundary: `on_profile` receives the [`BoundaryProfile`] of each
    /// replayed boundary as soon as it settles, in journal order. Pass
    /// `Arc::new(())` as `archive_provider` for a run without package
    /// archives; the provider must be `Send + Sync`, as for
    /// [`Canwu::set_plugin_archive_object_provider`].
    ///
    /// Profiling changes nothing the replay checks: every regenerated
    /// boundary and the final checkpoint must match the journal exactly as in
    /// an unprofiled replay. The profiles are handed over, not kept, so
    /// [`MAX_RETAINED_BOUNDARY_PROFILES`] does not limit a long replay, and
    /// the returned runtime has profiling off. When the replay fails,
    /// `on_profile` has received the profiles of the boundaries that settled
    /// before the failure.
    pub fn replay_from_journal_with_boundary_profiles(
        plugins: &[&dyn SimulationPlugin],
        journal: &ReplayJournal,
        archive_provider: std::sync::Arc<dyn PluginArchiveObjectProvider + Send + Sync>,
        on_profile: &mut dyn FnMut(BoundaryProfile),
    ) -> Result<Self, CanwuError> {
        let simulation = Simulation::replay_from_journal_with_boundary_profiles(
            plugins,
            journal,
            archive_provider,
            on_profile,
        )?;
        Ok(Self::wrap(simulation))
    }

    pub fn replay_from_journal_json(
        plugins: &[&dyn SimulationPlugin],
        json: &str,
    ) -> Result<Self, CanwuError> {
        Ok(Self::wrap(Simulation::replay_from_journal_json(
            plugins, json,
        )?))
    }

    /// An independent copy of the run for a look-ahead, an alternative
    /// reality, or a save. It shares the attached plugin archive provider and
    /// starts with profiling off. The copy is `Send`, so a host may advance,
    /// checkpoint, or save it on another thread.
    #[must_use]
    pub fn fork(&self) -> Self {
        Self::wrap(self.simulation.fork())
    }

    /// Trusted host/admin holder query. Player-facing callers must use a
    /// restricted [`CanwuViewer`].
    pub fn admin_query_knowledge(
        &self,
        holder: KnowledgeHolderRef,
        query: &KnowledgeQuery,
    ) -> Result<KnowledgeQueryResult, CanwuError> {
        admin_query_knowledge(&self.simulation, holder, query)
    }

    /// Creates the restricted viewer dictated entirely by the persisted run
    /// policy and seat binding.
    pub fn viewer(&self) -> Result<CanwuViewer<'_>, CanwuError> {
        viewer(&self.simulation)
    }

    /// Character-seat and compatibility convenience. It never upgrades an
    /// institution, public, research, or developer policy to a person.
    pub fn viewer_for_actor(&self, actor: PersonId) -> Result<CanwuViewer<'_>, CanwuError> {
        viewer_for_actor(&self.simulation, actor)
    }

    pub fn viewer_context(&self, actor: PersonId) -> Result<ViewerContext, CanwuError> {
        viewer_context(&self.simulation, actor)
    }

    #[must_use]
    pub fn explain(&self, request: &ExplanationRequest) -> Explanation {
        explain(&self.simulation, request)
    }
}

/// The reads the restricted viewer and the explanation make, shared by a
/// full-history runtime and a compact one. Evidence reads see what the
/// runtime retains.
trait ViewerSource {
    fn source_knowledge(&self) -> &KnowledgeSnapshot;
    fn source_knowledge_retention(&self, schema: &KnowledgeSchemaId) -> Option<KnowledgeRetention>;
    fn source_checkpoint_hash(&self) -> &str;
    fn source_events(&self) -> &[SimEvent];
    fn source_event_audience(&self, event: &SimEvent) -> EventAudience;
    fn source_boundaries(&self) -> &[BoundaryRecord];
    fn source_entity_exists(&self, entity: &EntityRef) -> bool;
    fn source_latest_boundary_id(&self) -> Option<BoundaryId>;
    fn source_run_configuration(&self) -> &RunConfigurationSnapshot;
}

impl ViewerSource for Simulation {
    fn source_knowledge(&self) -> &KnowledgeSnapshot {
        self.knowledge()
    }
    fn source_knowledge_retention(&self, schema: &KnowledgeSchemaId) -> Option<KnowledgeRetention> {
        self.knowledge_retention(schema)
    }
    fn source_checkpoint_hash(&self) -> &str {
        self.checkpoint_hash()
    }
    fn source_events(&self) -> &[SimEvent] {
        self.events()
    }
    fn source_event_audience(&self, event: &SimEvent) -> EventAudience {
        self.event_audience(event)
    }
    fn source_boundaries(&self) -> &[BoundaryRecord] {
        self.boundaries()
    }
    fn source_entity_exists(&self, entity: &EntityRef) -> bool {
        self.entity_exists(entity)
    }
    fn source_latest_boundary_id(&self) -> Option<BoundaryId> {
        self.latest_boundary_id()
    }
    fn source_run_configuration(&self) -> &RunConfigurationSnapshot {
        self.run_configuration()
    }
}

impl ViewerSource for CompactedSimulation {
    fn source_knowledge(&self) -> &KnowledgeSnapshot {
        self.knowledge()
    }
    fn source_knowledge_retention(&self, schema: &KnowledgeSchemaId) -> Option<KnowledgeRetention> {
        self.knowledge_retention(schema)
    }
    fn source_checkpoint_hash(&self) -> &str {
        self.checkpoint_hash()
    }
    fn source_events(&self) -> &[SimEvent] {
        self.events()
    }
    fn source_event_audience(&self, event: &SimEvent) -> EventAudience {
        self.event_audience(event)
    }
    fn source_boundaries(&self) -> &[BoundaryRecord] {
        self.boundaries()
    }
    fn source_entity_exists(&self, entity: &EntityRef) -> bool {
        self.entity_exists(entity)
    }
    fn source_latest_boundary_id(&self) -> Option<BoundaryId> {
        self.latest_boundary_id()
    }
    fn source_run_configuration(&self) -> &RunConfigurationSnapshot {
        self.run_configuration()
    }
}

fn admin_query_knowledge(
    source: &dyn ViewerSource,
    holder: KnowledgeHolderRef,
    query: &KnowledgeQuery,
) -> Result<KnowledgeQueryResult, CanwuError> {
    source
        .source_knowledge()
        .query_current(holder, query, source.source_latest_boundary_id())
        .map(|result| {
            result.with_retention(query, |schema| source.source_knowledge_retention(schema))
        })
        .map_err(map_knowledge_query_error)
}

fn viewer(source: &dyn ViewerSource) -> Result<CanwuViewer<'_>, CanwuError> {
    let principal = declared_observation_principal(source)?;
    Ok(CanwuViewer {
        source,
        context: KnowledgeViewContext { principal },
    })
}

fn viewer_for_actor(
    source: &dyn ViewerSource,
    actor: PersonId,
) -> Result<CanwuViewer<'_>, CanwuError> {
    if !source.source_entity_exists(&EntityRef::Person(actor)) {
        return Err(CanwuError::new(
            ErrorCode::ActorNotFound,
            format!("actor {actor} was not found"),
        ));
    }
    let principal = match source.source_run_configuration().declared() {
        Some(configuration)
            if configuration.observation == ObservationPolicy::ActorBound
                && configuration.seat == SeatPolicy::CharacterBound
                && configuration
                    .seat_binding
                    .as_ref()
                    .and_then(|binding| binding.actor)
                    == Some(actor) =>
        {
            ObservationPrincipal::Person(actor)
        }
        Some(_) => {
            return Err(CanwuError::new(
                ErrorCode::InvalidAuthority,
                "the persisted run policy does not authorize a character viewer",
            ));
        }
        None => ObservationPrincipal::Person(actor),
    };
    Ok(CanwuViewer {
        source,
        context: KnowledgeViewContext { principal },
    })
}

fn viewer_context(source: &dyn ViewerSource, actor: PersonId) -> Result<ViewerContext, CanwuError> {
    let viewer = viewer_for_actor(source, actor)?;
    Ok(ViewerContext {
        principal: viewer.context.principal.clone(),
        observation: ObservationPolicy::ActorBound,
        checkpoint_hash: source.source_checkpoint_hash().to_owned(),
    })
}

fn declared_observation_principal(
    source: &dyn ViewerSource,
) -> Result<ObservationPrincipal, CanwuError> {
    let Some(configuration) = source.source_run_configuration().declared() else {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "legacy runs require viewer_for_actor with an existing character",
        ));
    };
    match configuration.observation {
        ObservationPolicy::ActorBound => match configuration.seat {
            SeatPolicy::CharacterBound => configuration
                .seat_binding
                .as_ref()
                .and_then(|binding| binding.actor)
                .map(ObservationPrincipal::Person)
                .ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::InvalidAuthority,
                        "character-bound observation lacks an actor binding",
                    )
                }),
            SeatPolicy::InstitutionBound => configuration
                .seat_binding
                .as_ref()
                .and_then(|binding| binding.institution.clone())
                .map(ObservationPrincipal::Institution)
                .ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::InvalidAuthority,
                        "institution-bound observation lacks an institution binding",
                    )
                }),
            SeatPolicy::ObserverSeat | SeatPolicy::AdvisorSeat | SeatPolicy::None => {
                Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "actor-bound observation requires a character or institution seat",
                ))
            }
        },
        ObservationPolicy::PublicObserver => Ok(ObservationPrincipal::Public),
        ObservationPolicy::ResearchFull => Ok(ObservationPrincipal::Research),
        ObservationPolicy::DeveloperDiagnostic => Ok(ObservationPrincipal::Developer),
    }
}

fn explain(source: &dyn ViewerSource, request: &ExplanationRequest) -> Explanation {
    match request {
        ExplanationRequest::Event(event_id) => explain_event(source, *event_id),
        ExplanationRequest::Failure(error) => Explanation {
            summary: error.message.clone(),
            causal_chain: vec![ExplanationStep {
                label: format!("Validation failed: {:?}", error.code),
                event: None,
            }],
        },
    }
}

fn explain_event(source: &dyn ViewerSource, event_id: EventId) -> Explanation {
    let mut chain = Vec::new();
    let events = source.source_events();
    let mut current = event_by_id(events, event_id);
    while let Some(event) = current {
        chain.push(ExplanationStep {
            label: event.summary.clone(),
            event: Some(event.id),
        });
        current = match &event.cause {
            Some(CauseRef::Boundary(boundary)) => {
                chain.push(ExplanationStep {
                    label: format!("Committed by boundary {boundary}"),
                    event: None,
                });
                None
            }
            Some(CauseRef::Event(parent)) => event_by_id(events, *parent),
            Some(CauseRef::Command(command)) => {
                chain.push(ExplanationStep {
                    label: format!("Accepted command {command}"),
                    event: None,
                });
                None
            }
            Some(CauseRef::System(system)) => {
                chain.push(ExplanationStep {
                    label: format!("Produced by system {system}"),
                    event: None,
                });
                None
            }
            None => None,
        };
    }
    Explanation {
        summary: chain.first().map_or_else(
            || "Event was not found".to_owned(),
            |step| step.label.clone(),
        ),
        causal_chain: chain,
    }
}

/// Finds an event in a retained, ID-ordered event tail, which starts after
/// the last seal in a compact runtime.
fn event_by_id(events: &[SimEvent], event_id: EventId) -> Option<&SimEvent> {
    let first = events.first()?.id.get();
    let index = usize::try_from(event_id.get().checked_sub(first)?).ok()?;
    events.get(index).filter(|event| event.id == event_id)
}

impl CompactedCanwu {
    /// Restores a run from a continuation checkpoint whose state names no
    /// plugin; one that names a plugin fails with
    /// [`ErrorCode::PluginNotActive`].
    ///
    /// The restored run continues exactly as the uninterrupted run, but has
    /// no history before the checkpoint: [`Self::checkpoint`],
    /// [`Self::prepare_paged_checkpoint`], [`Self::snapshot_with_segments`],
    /// and [`Self::replay_journal_with_segments`] return
    /// [`ErrorCode::HistoryUnavailable`], and retained-evidence reads see only
    /// evidence after the base. The checkpoint's own hash detects
    /// corruption only; use [`Self::from_continuation_checkpoint_verified`]
    /// with the hash stored when the checkpoint was written.
    pub fn from_continuation_checkpoint(
        checkpoint: ContinuationCheckpoint,
    ) -> Result<Self, CanwuError> {
        Ok(Self {
            simulation: CompactedSimulation::from_continuation_checkpoint(checkpoint)?,
        })
    }

    /// Restores a run from a continuation checkpoint and rehydrates its
    /// exact executable plugins.
    pub fn from_continuation_checkpoint_with_plugins(
        checkpoint: ContinuationCheckpoint,
        plugins: &[&dyn SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        Ok(Self {
            simulation: CompactedSimulation::from_continuation_checkpoint_with_plugins(
                checkpoint, plugins,
            )?,
        })
    }

    /// Restores a run from a continuation checkpoint only if its
    /// continuation hash is the one the host stored when it was written.
    pub fn from_continuation_checkpoint_verified(
        checkpoint: ContinuationCheckpoint,
        expected_continuation_hash: &str,
        plugins: &[&dyn SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        Ok(Self {
            simulation: CompactedSimulation::from_continuation_checkpoint_verified(
                checkpoint,
                expected_continuation_hash,
                plugins,
            )?,
        })
    }

    /// Restores a run from the JSON of a continuation checkpoint.
    pub fn from_continuation_checkpoint_json_with_plugins(
        json: &str,
        plugins: &[&dyn SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        Ok(Self {
            simulation: CompactedSimulation::from_continuation_checkpoint_json_with_plugins(
                json, plugins,
            )?,
        })
    }

    /// Restores a run from the JSON of a continuation checkpoint only if its
    /// continuation hash is the one the host stored.
    pub fn from_continuation_checkpoint_json_verified(
        json: &str,
        expected_continuation_hash: &str,
        plugins: &[&dyn SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        Ok(Self {
            simulation: CompactedSimulation::from_continuation_checkpoint_json_verified(
                json,
                expected_continuation_hash,
                plugins,
            )?,
        })
    }

    /// Builds the continuation checkpoint of this run at its settled cut and
    /// returns it with its continuation hash, the value to store as the
    /// trust anchor. It fails with [`ErrorCode::ArchiveNotReady`] exactly
    /// when a seal would.
    pub fn continuation_checkpoint(&self) -> Result<(ContinuationCheckpoint, String), CanwuError> {
        self.simulation.continuation_checkpoint()
    }

    /// Where this run's history begins when it was restored from a
    /// continuation checkpoint; `None` for a run that holds its history.
    #[must_use]
    pub fn continuation_base(&self) -> Option<&ContinuationBase> {
        self.simulation.continuation_base()
    }

    pub fn from_checkpoint_and_journal(
        checkpoint: SimulationCheckpoint,
        segments: Vec<EvidenceJournalSegment>,
    ) -> Result<Self, CanwuError> {
        Ok(Self {
            simulation: CompactedSimulation::from_checkpoint_and_journal(checkpoint, segments)?,
        })
    }

    pub fn from_checkpoint_and_journal_with_plugins(
        checkpoint: SimulationCheckpoint,
        segments: Vec<EvidenceJournalSegment>,
        plugins: &[&dyn SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        Ok(Self {
            simulation: CompactedSimulation::from_checkpoint_and_journal_with_plugins(
                checkpoint, segments, plugins,
            )?,
        })
    }

    /// Restores a paged checkpoint plus its contiguous evidence prefix and
    /// enters the compact interface with that whole prefix retained as live
    /// evidence.
    ///
    /// The prefix is the host's sealed segments in cursor order followed by
    /// [`Self::journal_segment_since`] the end of the last one, omitted when
    /// nothing was recorded after it; the page and prefix rules of
    /// [`Canwu::from_paged_checkpoint_and_journal`] apply. Because the restored
    /// runtime retains the whole prefix, its next seal returns one segment
    /// from the zero cut that replaces every earlier sealed segment. Prefixes
    /// for later checkpoints start from that segment; only checkpoints taken
    /// before it still need the earlier ones. A compact runtime cannot
    /// register plugins after it is built, so this fails with
    /// [`ErrorCode::PluginNotActive`] when the checkpoint names any plugin; use
    /// [`Self::from_paged_checkpoint_and_journal_with_plugins`].
    /// A paged root also commits to which evidence is sealed, so any restore
    /// from sealed segments, through this call or
    /// [`Canwu::from_paged_checkpoint_and_journal`], keeps the original
    /// checkpoint hash but gets a different paged root, which a later seal
    /// does not bring back; compare checkpoint hashes instead. A runtime that
    /// has never sealed evidence restores to the same paged root.
    pub fn from_paged_checkpoint_and_journal(
        checkpoint: &PagedSimulationCheckpoint,
        provider: &dyn StatePageProvider,
        segments: Vec<EvidenceJournalSegment>,
    ) -> Result<Self, CanwuError> {
        Ok(Self {
            simulation: CompactedSimulation::from_paged_checkpoint_and_journal(
                checkpoint, provider, segments,
            )?,
        })
    }

    /// Restores a paged compact runtime and rehydrates the exact executable
    /// plugins it names.
    pub fn from_paged_checkpoint_and_journal_with_plugins(
        checkpoint: &PagedSimulationCheckpoint,
        provider: &dyn StatePageProvider,
        segments: Vec<EvidenceJournalSegment>,
        plugins: &[&dyn SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        Ok(Self {
            simulation: CompactedSimulation::from_paged_checkpoint_and_journal_with_plugins(
                checkpoint, provider, segments, plugins,
            )?,
        })
    }

    pub fn evidence_cursor(&self) -> Result<EvidenceCursor, CanwuError> {
        self.simulation.evidence_cursor()
    }

    /// The cut through the evidence this run no longer retains: the end of
    /// its last sealed segment, or the settled cut of the continuation
    /// checkpoint it was restored from. `None` while it retains all its
    /// evidence: it has never sealed, or it was restored with its whole
    /// journal (whose first seal starts again at the zero cut).
    #[must_use]
    pub fn sealed_evidence_cursor(&self) -> Option<EvidenceCursor> {
        self.simulation.sealed_evidence_cursor()
    }

    /// Copies the compact runtime for a look-ahead or an alternative
    /// reality, as [`Canwu::fork`] does. The copy keeps the seal tokens
    /// already committed, shares the attached plugin archive provider, starts
    /// with profiling off, and may move to another thread.
    #[must_use]
    pub fn fork(&self) -> Self {
        Self {
            simulation: self.simulation.fork(),
        }
    }

    /// Attaches caller-owned package archive storage for authenticated cold
    /// history resolution during admission, settlement, and queries, as
    /// [`Canwu::set_plugin_archive_object_provider`] does. The provider must
    /// be `Send + Sync`.
    pub fn set_plugin_archive_object_provider(
        &mut self,
        provider: std::sync::Arc<dyn PluginArchiveObjectProvider + Send + Sync>,
    ) {
        self.simulation.set_plugin_archive_object_provider(provider);
    }

    /// Loads one opaque package archive object from the attached provider.
    /// Package integrations authenticate the bytes against their committed
    /// archive roots before use.
    pub fn plugin_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, CanwuError> {
        self.simulation.plugin_archive_object(namespace, object_id)
    }

    /// The commitment-format hash of authoritative state. A compact run and
    /// an unsealed run with the same state have the same hash.
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
    pub const fn run_configuration(&self) -> &RunConfigurationSnapshot {
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

    /// Whether an evidence identity exists at this cut. A sealed event,
    /// command, command attempt, ingress record, boundary, or draw exists
    /// whether or not the seal kept its receipt; see
    /// [`Canwu::evidence_exists`].
    #[must_use]
    pub fn evidence_exists(&self, reference: &EvidenceRef) -> bool {
        self.simulation.evidence_exists(reference)
    }

    /// When evidence became authoritative, answered for sealed ordinal
    /// evidence as an unsealed run answers; see [`Canwu::evidence_time`].
    #[must_use]
    pub fn evidence_time(&self, reference: &EvidenceRef) -> Option<SimTime> {
        self.simulation.evidence_time(reference)
    }

    /// The body of one exact domain-record version. `None` covers both a
    /// version that does not exist and one whose body a seal removed, so a
    /// host check built on it depends on what the run has sealed. The
    /// current version always resolves.
    #[must_use]
    pub fn domain_record_version(
        &self,
        reference: &DomainRecordVersionRef,
    ) -> Option<DomainRecord> {
        self.simulation.domain_record_version(reference)
    }

    /// Whether an exact domain-record version exists in current, retained,
    /// or kept evidence.
    ///
    /// On a sealed run a superseded version that is not declared as a
    /// dependency is known only while an earlier seal kept its receipt, so the
    /// answer can differ between a sealed run and an unsealed one; a rule that
    /// must decide alike in both cites a current version or declares the
    /// version as an identity-only dependency.
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

    /// Returns one trusted-host page bound to an authoritative revision, as
    /// [`Canwu::domain_record_page`] does.
    pub fn domain_record_page(
        &self,
        kind: &DomainRecordKind,
        after: Option<&DomainRecordRef>,
        limit: usize,
        expected_revision: Option<u64>,
    ) -> Result<DomainRecordPage, CanwuError> {
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
    pub fn commands(&self) -> &[CommandRecord] {
        self.simulation.command_log()
    }

    /// The retained boundary tail, after the last seal.
    #[must_use]
    pub fn boundaries(&self) -> &[BoundaryRecord] {
        self.simulation.boundaries()
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

    /// The retained random-draw tail, after the last seal.
    #[must_use]
    pub fn random_draws(&self) -> &[RandomDrawRecord] {
        self.simulation.random_draws()
    }

    /// Trusted host/admin holder query, as [`Canwu::admin_query_knowledge`]
    /// does. Player-facing callers must use a restricted [`CanwuViewer`].
    pub fn admin_query_knowledge(
        &self,
        holder: KnowledgeHolderRef,
        query: &KnowledgeQuery,
    ) -> Result<KnowledgeQueryResult, CanwuError> {
        admin_query_knowledge(&self.simulation, holder, query)
    }

    /// Creates the restricted viewer dictated by the persisted run policy
    /// and seat binding, as [`Canwu::viewer`] does. Its evidence-backed reads
    /// (`visible_changes_since`, `evaluation_traces`) see the retained tail.
    pub fn viewer(&self) -> Result<CanwuViewer<'_>, CanwuError> {
        viewer(&self.simulation)
    }

    /// Character-seat convenience, as [`Canwu::viewer_for_actor`] does.
    pub fn viewer_for_actor(&self, actor: PersonId) -> Result<CanwuViewer<'_>, CanwuError> {
        viewer_for_actor(&self.simulation, actor)
    }

    /// The viewer context of a character seat, as [`Canwu::viewer_context`]
    /// returns it.
    pub fn viewer_context(&self, actor: PersonId) -> Result<ViewerContext, CanwuError> {
        viewer_context(&self.simulation, actor)
    }

    /// Explains an event from the retained tail, as [`Canwu::explain`] does.
    #[must_use]
    pub fn explain(&self, request: &ExplanationRequest) -> Explanation {
        explain(&self.simulation, request)
    }

    pub fn checkpoint(&self) -> Result<SimulationCheckpoint, CanwuError> {
        self.simulation.checkpoint()
    }

    /// Clones the retained evidence after `start`, which must not precede this
    /// runtime's last sealed cut.
    ///
    /// A checkpoint of current state restores from the host's prefix followed
    /// by this segment from that prefix's end, omitted when it is empty. The
    /// prefix is the segments this runtime has sealed, in cursor order. A
    /// restored runtime that has not sealed yet uses the prefix it was
    /// restored from; its first seal starts at the zero cut and replaces that
    /// prefix.
    pub fn journal_segment_since(
        &self,
        start: EvidenceCursor,
    ) -> Result<EvidenceJournalSegment, CanwuError> {
        self.simulation.journal_segment_since(start)
    }

    /// Prepares a content-addressed paged checkpoint of current state without
    /// rehydrating sealed evidence or archived decision payloads. See
    /// [`Canwu::prepare_paged_checkpoint`].
    pub fn prepare_paged_checkpoint(
        &self,
        source: Option<&PagedSimulationCheckpoint>,
        provider: &dyn StatePageProvider,
    ) -> Result<PreparedPagedSimulationCheckpoint, CanwuError> {
        self.simulation.prepare_paged_checkpoint(source, provider)
    }

    pub fn outbox_entries(&self) -> Result<Vec<OutboxEntry>, CanwuError> {
        self.simulation.outbox_entries()
    }

    pub fn outbox_entries_for_segment(
        &self,
        segment: &EvidenceJournalSegment,
    ) -> Result<Vec<OutboxEntry>, CanwuError> {
        self.simulation.outbox_entries_for_segment(segment)
    }

    #[must_use]
    pub fn archived_evidence_receipt(
        &self,
        reference: &EvidenceRef,
    ) -> Option<&ArchivedEvidenceReceipt> {
        self.simulation.archived_evidence_receipt(reference)
    }

    pub fn load_archived_evidence_segment(
        &self,
        reference: &EvidenceRef,
        provider: &dyn ArchiveProvider,
    ) -> Result<EvidenceJournalSegment, CanwuError> {
        self.simulation
            .load_archived_evidence_segment(reference, provider)
    }

    pub fn seal_evidence(&mut self) -> Result<Option<EvidenceJournalSegment>, CanwuError> {
        self.simulation.seal_evidence()
    }

    pub fn prepare_evidence_seal(&self) -> Result<Option<PreparedEvidenceSeal>, CanwuError> {
        self.simulation.prepare_evidence_seal()
    }

    pub fn commit_evidence_seal(
        &mut self,
        token: &EvidenceSealToken,
        provider: &dyn ArchiveProvider,
    ) -> Result<(), CanwuError> {
        self.simulation.commit_evidence_seal(token, provider)
    }

    pub fn snapshot_with_segments(
        &self,
        segments: Vec<EvidenceJournalSegment>,
    ) -> Result<SimulationSnapshot, CanwuError> {
        self.simulation.snapshot_with_segments(segments)
    }

    pub fn replay_journal_with_segments(
        &self,
        segments: Vec<EvidenceJournalSegment>,
    ) -> Result<ReplayJournal, CanwuError> {
        self.simulation.replay_journal_with_segments(segments)
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

    pub fn entities(&self) -> impl Iterator<Item = &EntityRef> {
        self.simulation.entities()
    }

    #[must_use]
    pub fn entity_exists(&self, entity: &EntityRef) -> bool {
        self.simulation.entity_exists(entity)
    }

    /// Deprecated detached format-5 compatibility projection.
    #[must_use]
    pub fn world(&self) -> WorldSnapshot {
        self.simulation.world()
    }

    /// Trusted host access to a person's committed life and custody state.
    /// `None` means no change has been committed: the person is alive and free.
    #[must_use]
    pub fn person_availability(&self, person: PersonId) -> Option<&PersonAvailability> {
        self.simulation.person_availability(person)
    }

    /// Trusted host access to every committed person availability, in
    /// person-ID order. Persons without an entry are alive and free.
    pub fn person_availabilities(&self) -> impl Iterator<Item = (&PersonId, &PersonAvailability)> {
        self.simulation.person_availabilities()
    }

    /// Trusted host access to the registered transition manifests whose
    /// ready boundary has not settled, in manifest-ID order. Settled
    /// manifests leave their audit on the boundary record and receipt.
    pub fn pending_transition_manifests(&self) -> impl Iterator<Item = &PendingTransitionManifest> {
        self.simulation.pending_transition_manifests()
    }

    /// Trusted host access to the boundaries of the half-open window
    /// `from..to`: those settled in it and those still queued in it. Settled
    /// boundaries are kept for the longest horizon a registered boundary
    /// system declares. Boundary systems read the same ledger through
    /// [`SimulationView::boundaries_between`] after declaring
    /// [`StateKey::core_boundary_schedule`].
    ///
    /// The result names every plugin's queued packet types. Do not expose it
    /// to player, agent, observer, or remote clients.
    pub fn boundaries_between(
        &self,
        from: SimTime,
        to: SimTime,
    ) -> Result<BoundaryWindow, CanwuError> {
        self.simulation.boundaries_between(from, to)
    }

    /// How far back the run keeps settled boundaries: the longest
    /// [`BoundarySystemContract::boundary_schedule_horizon`] a registered
    /// boundary system declares, or zero.
    #[must_use]
    pub fn boundary_schedule_horizon(&self) -> SimDuration {
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

    /// The body and establishment time of a record's committed current
    /// version or of an initial-scenario version, which a seal cannot
    /// change; see [`Canwu::replay_stable_domain_record_version`].
    #[must_use]
    pub fn replay_stable_domain_record_version(
        &self,
        reference: &DomainRecordVersionRef,
    ) -> Option<(DomainRecord, SimTime)> {
        self.simulation
            .replay_stable_domain_record_version(reference)
    }

    #[must_use]
    pub fn typed_domain_record<T: DomainRecordType>(
        &self,
        reference: &TypedDomainRecordRef<T>,
    ) -> Option<&DomainRecord> {
        self.simulation.typed_domain_record(reference)
    }

    #[must_use]
    pub fn decision_ticket(&self, id: DecisionTicketId) -> Option<&DecisionTicket> {
        self.simulation.decision_ticket(id)
    }

    #[must_use]
    pub fn decision_controller(&self, id: &str) -> Option<&DecisionControllerBinding> {
        self.simulation.decision_controller(id)
    }

    #[must_use]
    pub fn decision_trace(&self, id: DecisionTraceId) -> Option<&DecisionTrace> {
        self.simulation.decision_trace(id)
    }

    #[must_use]
    pub fn decision_attempt(&self, id: DecisionRequestId) -> Option<&DecisionAttemptRecord> {
        self.simulation.decision_attempt(id)
    }

    #[must_use]
    pub fn decision_hot_state(&self) -> DecisionHotState {
        self.simulation.decision_hot_state()
    }

    #[must_use]
    pub fn decision_history_location(&self, key: &DecisionHistoryKey) -> DecisionHistoryLocation {
        self.simulation.decision_history_location(key)
    }

    pub fn decision_history_location_with_provider(
        &self,
        key: &DecisionHistoryKey,
        provider: &dyn DecisionArchiveProvider,
    ) -> Result<DecisionHistoryLocation, CanwuError> {
        self.simulation
            .decision_history_location_with_provider(key, provider)
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
        permit: &PluginIngressPermit,
    ) -> Result<IngressReceipt, CanwuError> {
        self.simulation
            .enqueue_permitted_plugin_ingress(request, permit)
    }

    /// Withdraws a still-pending plugin ingress item that the host enqueued
    /// with [`Self::enqueue_plugin_ingress`], strictly before its due time.
    ///
    /// Due, admitted, archived, or already cancelled items fail with
    /// [`ErrorCode::LateIngress`]; items of internal packet types or items a
    /// plugin scheduled inside the engine fail with
    /// [`ErrorCode::InvalidAuthority`]; unknown IDs fail with
    /// [`ErrorCode::EvidenceUnavailable`]; non-plugin targets and reasons that
    /// are empty, untrimmed, or longer than
    /// [`MAX_INGRESS_CANCELLATION_REASON_BYTES`] fail with
    /// [`ErrorCode::InvalidPayload`]; declared read-only runs fail with
    /// [`ErrorCode::InteractionReadOnly`]. The returned receipt names the
    /// terminal [`IngressPayload::PluginCancellation`] journal record. The
    /// withdrawn item is never admitted, never settles, and nothing is rolled
    /// back; snapshots, checkpoint journals, and exact replay preserve the
    /// cancellation.
    pub fn cancel_plugin_ingress(
        &mut self,
        ingress_id: IngressId,
        reason: impl Into<String>,
    ) -> Result<IngressReceipt, CanwuError> {
        self.simulation.cancel_plugin_ingress(ingress_id, reason)
    }

    /// Withdraws a still-pending item of an internal packet type through the
    /// owning plugin's opaque registration permit. The permit covers
    /// host-enqueued items of that exact type and items the same plugin
    /// scheduled inside the engine; timing rules match
    /// [`Self::cancel_plugin_ingress`].
    pub fn cancel_permitted_plugin_ingress(
        &mut self,
        ingress_id: IngressId,
        permit: &PluginIngressPermit,
        reason: impl Into<String>,
    ) -> Result<IngressReceipt, CanwuError> {
        self.simulation
            .cancel_permitted_plugin_ingress(ingress_id, permit, reason)
    }

    pub fn prepare_decision(
        &self,
        decision_request_id: DecisionRequestId,
        command_request_id: Option<CommandRequestId>,
        ticket_id: DecisionTicketId,
        policy: &dyn DecisionPolicy,
    ) -> Result<DecisionEvaluation, CanwuError> {
        self.simulation
            .prepare_decision(decision_request_id, command_request_id, ticket_id, policy)
    }

    pub fn prepare_decision_at(
        &self,
        due_at: SimTime,
        decision_request_id: DecisionRequestId,
        command_request_id: Option<CommandRequestId>,
        ticket_id: DecisionTicketId,
        policy: &dyn DecisionPolicy,
    ) -> Result<DecisionEvaluation, CanwuError> {
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
        due_at: SimTime,
        priority: i32,
        request: DecisionIngressRequest,
    ) -> Result<IngressReceipt, CanwuError> {
        self.simulation.enqueue_decision(due_at, priority, request)
    }

    pub fn drive_decision(
        &mut self,
        due_at: SimTime,
        priority: i32,
        decision_request_id: DecisionRequestId,
        command_request_id: Option<CommandRequestId>,
        ticket_id: DecisionTicketId,
        policy: &dyn DecisionPolicy,
    ) -> Result<DecisionEvaluation, CanwuError> {
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

    /// Turns the wall-time profile of settled boundaries on or off. It is off
    /// in a new, restored, replayed, or forked runtime;
    /// [`Canwu::into_compacted`] keeps the setting and the profiles not yet
    /// taken.
    ///
    /// While it is on, every boundary that settles leaves one
    /// [`BoundaryProfile`] for [`Self::take_boundary_profiles`]: the wall
    /// time of the settlement and of each plugin handler it ran, which are
    /// its boundary systems by phase, the plugin command handlers of the
    /// queued commands it admitted, and the event-driven plugin systems those
    /// commands and the scheduled actions due at the boundary triggered. A
    /// boundary that fails leaves none. Sealing evidence neither changes the
    /// setting nor drops profiles. Owner-authorized maintenance
    /// participants run when the host schedules the maintenance, outside
    /// settlement, and are in no profile.
    ///
    /// An exact replay settles its boundaries inside the replay call, on a
    /// runtime that starts with profiling off, so a replay is not profiled;
    /// the runtime it returns can be profiled from then on.
    ///
    /// Profiling is a host diagnostic. It reads the wall clock and changes
    /// nothing else: receipts, evidence, hashes, snapshots, checkpoints, and
    /// exact replay are the same with it on or off, and no plugin can read a
    /// profile. While it is off, settlement reads no clock. Do not feed
    /// profile values back into the simulation.
    pub fn set_boundary_profiling(&mut self, enabled: bool) {
        self.simulation.set_boundary_profiling(enabled);
    }

    /// Whether settled boundaries are being profiled.
    #[must_use]
    pub const fn boundary_profiling(&self) -> bool {
        self.simulation.boundary_profiling()
    }

    /// Takes the profiles of the boundaries settled since the last call, in
    /// settlement order. The runtime keeps at most
    /// [`MAX_RETAINED_BOUNDARY_PROFILES`] profiles that were not taken yet;
    /// [`BoundaryProfileBatch::dropped`] counts the older ones it dropped
    /// since the last call.
    pub fn take_boundary_profiles(&mut self) -> BoundaryProfileBatch {
        self.simulation.take_boundary_profiles()
    }
}

/// An observation identity authorized by the run's persisted observation
/// policy. This type is intentionally constructed through
/// [`Canwu::viewer_context`] so an observation request cannot self-escalate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ObservationPrincipal {
    Person(PersonId),
    Institution(EntityRef),
    Public,
    Research,
    Developer,
}

impl ObservationPrincipal {
    const fn person(&self) -> Option<PersonId> {
        match self {
            Self::Person(actor) => Some(*actor),
            Self::Institution(_) | Self::Public | Self::Research | Self::Developer => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ViewerContext {
    principal: ObservationPrincipal,
    observation: ObservationPolicy,
    checkpoint_hash: String,
}

impl ViewerContext {
    #[must_use]
    pub const fn principal(&self) -> &ObservationPrincipal {
        &self.principal
    }

    #[must_use]
    pub const fn actor(&self) -> Option<PersonId> {
        self.principal.person()
    }

    #[must_use]
    pub const fn observation(&self) -> ObservationPolicy {
        self.observation
    }
}

#[derive(Clone, Debug)]
struct KnowledgeViewContext {
    principal: ObservationPrincipal,
}

/// Restricted player/agent/observer API. It deliberately exposes no raw
/// snapshot, event, boundary, domain-record, or audit-origin access.
pub struct CanwuViewer<'a> {
    source: &'a dyn ViewerSource,
    context: KnowledgeViewContext,
}

impl CanwuViewer<'_> {
    #[must_use]
    pub const fn principal(&self) -> &ObservationPrincipal {
        &self.context.principal
    }

    /// Queries only the holder selected by a bound person or institution
    /// principal. Public and diagnostic principals must use their separately
    /// named capabilities.
    pub fn query_knowledge(
        &self,
        query: &KnowledgeQuery,
    ) -> Result<KnowledgeQueryResult, CanwuError> {
        let holder = match &self.context.principal {
            ObservationPrincipal::Person(actor) => KnowledgeHolderRef::Person(*actor),
            ObservationPrincipal::Institution(entity) => KnowledgeHolderRef::Entity(entity.clone()),
            ObservationPrincipal::Public => return Err(invalid_knowledge_authority()),
            ObservationPrincipal::Research | ObservationPrincipal::Developer => {
                return Err(CanwuError::new(
                    ErrorCode::InvalidKnowledgeAuthority,
                    "diagnostic viewers must select a holder explicitly",
                ));
            }
        };
        admin_query_knowledge(self.source, holder, query)
    }

    /// Selects an existing holder under an explicit research/developer policy.
    /// Returned records remain the origin-free holder projection.
    pub fn query_holder_knowledge(
        &self,
        holder: KnowledgeHolderRef,
        query: &KnowledgeQuery,
    ) -> Result<KnowledgeQueryResult, CanwuError> {
        match self.context.principal {
            ObservationPrincipal::Research | ObservationPrincipal::Developer => {}
            ObservationPrincipal::Person(_)
            | ObservationPrincipal::Institution(_)
            | ObservationPrincipal::Public => return Err(invalid_knowledge_authority()),
        }
        if !knowledge_holder_exists(self.source, &holder) {
            return Err(CanwuError::new(
                ErrorCode::InvalidKnowledgeHolder,
                "the requested knowledge holder does not exist",
            ));
        }
        admin_query_knowledge(self.source, holder, query)
    }

    /// Returns one audit-bearing stored record only for research/developer
    /// principals. Normal holder queries never expose origin evidence.
    pub fn audit_knowledge_record(
        &self,
        holder: &KnowledgeHolderRef,
        record: HolderKnowledgeRecordId,
    ) -> Result<KnowledgeRecord, CanwuError> {
        match self.context.principal {
            ObservationPrincipal::Research | ObservationPrincipal::Developer => {}
            ObservationPrincipal::Person(_)
            | ObservationPrincipal::Institution(_)
            | ObservationPrincipal::Public => return Err(invalid_knowledge_authority()),
        }
        if !knowledge_holder_exists(self.source, holder) {
            return Err(CanwuError::new(
                ErrorCode::InvalidKnowledgeHolder,
                "the requested knowledge holder does not exist",
            ));
        }
        // Local IDs are stored and never reused, so a record keeps its local
        // ID when other entries of the holder are retired.
        self.source
            .source_knowledge()
            .for_holder(holder)
            .and_then(|records| records.id_of(record).and_then(|id| records.get(&id)))
            .ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::KnowledgeRecordNotFound,
                    "holder-relative knowledge record was not found",
                )
            })
    }

    #[must_use]
    pub fn visible_changes_since(&self, since: SimTime) -> Vec<VisibleChange> {
        let context = ViewerContext {
            principal: self.context.principal.clone(),
            observation: observation_for_principal(&self.context.principal),
            checkpoint_hash: self.source.source_checkpoint_hash().to_owned(),
        };
        self.source
            .source_events()
            .iter()
            .filter(|event| event.timestamp > since)
            .filter_map(|event| {
                let audience = self.source.source_event_audience(event);
                visible_change(&context, event, &audience)
            })
            .collect()
    }

    /// Returns the holder-facing projection of the retained rule-evaluation
    /// traces of `subject` that this principal may see, from boundaries after
    /// `after` (every retained boundary when `None`), in boundary and
    /// recording order.
    ///
    /// Visibility follows the holder ledger that [`Self::query_knowledge`]
    /// reads. A person or institution principal sees a trace when the subject
    /// is its own entity, or when a knowledge publication to its ledger that
    /// names the subject (as an entity target or, for a domain entity, as a
    /// domain-record target) was visible before the trace was evaluated: in
    /// an earlier boundary, or as a same-boundary publication in phase 4 of
    /// the same boundary. A holder
    /// therefore never gains the breakdowns of a subject evaluated before it
    /// learned of it. Research and developer principals see every trace; a
    /// public principal cannot read traces, as it cannot read a private
    /// ledger.
    ///
    /// Like holder knowledge views, the projection omits evidence. Term
    /// evidence and the producing plugin and system stay on the trusted
    /// [`BoundaryRecord::evaluation_traces`] read through [`Canwu::boundaries`].
    pub fn evaluation_traces(
        &self,
        subject: &EntityRef,
        after: Option<BoundaryId>,
    ) -> Result<Vec<EvaluationTraceView>, CanwuError> {
        let (own, holder) = match &self.context.principal {
            ObservationPrincipal::Person(actor) => (
                EntityRef::Person(*actor),
                KnowledgeHolderRef::Person(*actor),
            ),
            ObservationPrincipal::Institution(entity) => {
                (entity.clone(), KnowledgeHolderRef::Entity(entity.clone()))
            }
            ObservationPrincipal::Research | ObservationPrincipal::Developer => {
                return Ok(evaluation_trace_views(self.source, subject, after, None));
            }
            ObservationPrincipal::Public => return Err(invalid_knowledge_authority()),
        };
        if &own == subject {
            return Ok(evaluation_trace_views(self.source, subject, after, None));
        }
        // Every generic ledger record is committed by exactly one boundary
        // knowledge change, so the first change naming the subject is the
        // exact cut at which the holder learned of it. A next-boundary
        // publication becomes known only once its boundary has settled.
        let learned = self.source.source_boundaries().iter().find_map(|boundary| {
            boundary
                .knowledge_changes
                .iter()
                .filter(|change| {
                    change
                        .bodies_of(&holder)
                        .any(|body| knowledge_names_subject(&body.subjects, subject))
                })
                .map(|change| match change.visibility {
                    StateVisibility::SameBoundary => (boundary.id, change.phase),
                    StateVisibility::NextBoundary => {
                        (boundary.id, BoundaryPhase::SaveReplayAndDiagnosticHashing)
                    }
                })
                .min()
        });
        Ok(learned.map_or_else(Vec::new, |learned| {
            evaluation_trace_views(self.source, subject, after, Some(learned))
        }))
    }
}

fn evaluation_trace_views(
    source: &dyn ViewerSource,
    subject: &EntityRef,
    after: Option<BoundaryId>,
    learned: Option<(BoundaryId, BoundaryPhase)>,
) -> Vec<EvaluationTraceView> {
    let boundaries = source.source_boundaries();
    let start = after.map_or(0, |after| {
        boundaries.partition_point(|boundary| boundary.id <= after)
    });
    boundaries[start..]
        .iter()
        .flat_map(|boundary| {
            boundary
                .evaluation_traces
                .iter()
                .map(move |entry| (boundary.id, entry))
        })
        .filter(|(boundary, entry)| {
            &entry.trace.subject == subject
                && learned.is_none_or(|learned| learned < (*boundary, entry.phase))
        })
        .map(|(_, entry)| EvaluationTraceView::from(&entry.trace))
        .collect()
}

fn knowledge_names_subject(subjects: &[KnowledgeSubject], subject: &EntityRef) -> bool {
    subjects.iter().any(|named| match &named.target {
        KnowledgeSubjectTarget::Entity(entity) => entity == subject,
        KnowledgeSubjectTarget::DomainRecord(reference) => {
            matches!(subject, EntityRef::Domain(domain) if domain == reference)
        }
        KnowledgeSubjectTarget::Event(_) => false,
    })
}

/// Holder-facing projection of one [`EvaluationTraceRecord`], returned by
/// [`CanwuViewer::evaluation_traces`]. It keeps the rule, subject, boundary,
/// result, and each term's contribution, and omits evidence identities.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EvaluationTraceView {
    pub rule_id: String,
    pub rule_version: String,
    pub subject: EntityRef,
    pub terms: Vec<EvaluationTermView>,
    pub result: i64,
    pub boundary: BoundaryId,
}

/// One term of an [`EvaluationTraceView`].
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EvaluationTermView {
    pub term_id: String,
    pub contribution: i64,
}

impl From<&EvaluationTraceRecord> for EvaluationTraceView {
    fn from(record: &EvaluationTraceRecord) -> Self {
        Self {
            rule_id: record.rule_id.clone(),
            rule_version: record.rule_version.clone(),
            subject: record.subject.clone(),
            terms: record
                .terms
                .iter()
                .map(|term| EvaluationTermView {
                    term_id: term.term_id.clone(),
                    contribution: term.contribution,
                })
                .collect(),
            result: record.result,
            boundary: record.boundary,
        }
    }
}

const fn observation_for_principal(principal: &ObservationPrincipal) -> ObservationPolicy {
    match principal {
        ObservationPrincipal::Person(_) | ObservationPrincipal::Institution(_) => {
            ObservationPolicy::ActorBound
        }
        ObservationPrincipal::Public => ObservationPolicy::PublicObserver,
        ObservationPrincipal::Research => ObservationPolicy::ResearchFull,
        ObservationPrincipal::Developer => ObservationPolicy::DeveloperDiagnostic,
    }
}

fn invalid_knowledge_authority() -> CanwuError {
    CanwuError::new(
        ErrorCode::InvalidKnowledgeAuthority,
        "this observation principal cannot read a private knowledge ledger",
    )
}

fn knowledge_holder_exists(source: &dyn ViewerSource, holder: &KnowledgeHolderRef) -> bool {
    match holder {
        KnowledgeHolderRef::Person(actor) => {
            source.source_entity_exists(&EntityRef::Person(*actor))
        }
        KnowledgeHolderRef::Entity(entity) => source.source_entity_exists(entity),
    }
}

fn map_knowledge_query_error(error: KnowledgeQueryError) -> CanwuError {
    match error {
        KnowledgeQueryError::ReadCutUnavailable => CanwuError::new(
            ErrorCode::KnowledgeReadCutUnavailable,
            "knowledge cursor read cut is no longer available",
        ),
        KnowledgeQueryError::InvalidLimit => CanwuError::new(
            ErrorCode::KnowledgeLimitExceeded,
            "knowledge query page size is outside the supported range",
        ),
        KnowledgeQueryError::InvalidCursor
        | KnowledgeQueryError::InvalidLedger
        | KnowledgeQueryError::Encoding => CanwuError::new(
            ErrorCode::InvalidKnowledgeRecord,
            "knowledge query, cursor, or ledger is invalid",
        ),
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct VisibleChange {
    pub timestamp: SimTime,
    pub summary: String,
    pub source_event: EventId,
}

fn visible_change(
    viewer: &ViewerContext,
    event: &SimEvent,
    plugin_audience: &EventAudience,
) -> Option<VisibleChange> {
    let visible = event_visible_to(viewer, event, plugin_audience);
    visible.then(|| VisibleChange {
        timestamp: event.timestamp,
        summary: event.summary.clone(),
        source_event: event.id,
    })
}

fn event_visible_to(viewer: &ViewerContext, event: &SimEvent, audience: &EventAudience) -> bool {
    if matches!(
        viewer.observation,
        ObservationPolicy::ResearchFull | ObservationPolicy::DeveloperDiagnostic
    ) {
        return true;
    }
    match audience {
        EventAudience::Public => true,
        EventAudience::Actor(actor) => viewer.principal.person() == Some(*actor),
        EventAudience::Actors(actors) => viewer
            .principal
            .person()
            .is_some_and(|actor| actors.binary_search(&actor).is_ok()),
        EventAudience::KnowledgeHolder(holder) => {
            principal_matches_holder(&viewer.principal, holder)
        }
        EventAudience::KnowledgeHolders(holders) => principal_holder(&viewer.principal)
            .is_some_and(|holder| holders.binary_search(&holder).is_ok()),
        EventAudience::AffectedActors => viewer
            .principal
            .person()
            .is_some_and(|actor| event.affected_entities.contains(&EntityRef::Person(actor))),
        EventAudience::Private => false,
    }
}

/// The knowledge holder a person or institution principal reads as.
fn principal_holder(principal: &ObservationPrincipal) -> Option<KnowledgeHolderRef> {
    match principal {
        ObservationPrincipal::Person(actor) => Some(KnowledgeHolderRef::Person(*actor)),
        ObservationPrincipal::Institution(institution) => {
            Some(KnowledgeHolderRef::Entity(institution.clone()))
        }
        ObservationPrincipal::Research
        | ObservationPrincipal::Developer
        | ObservationPrincipal::Public => None,
    }
}

fn principal_matches_holder(principal: &ObservationPrincipal, holder: &KnowledgeHolderRef) -> bool {
    match (principal, holder) {
        (ObservationPrincipal::Person(actor), KnowledgeHolderRef::Person(holder)) => {
            actor == holder
        }
        (ObservationPrincipal::Institution(institution), KnowledgeHolderRef::Entity(holder)) => {
            institution == holder
        }
        (ObservationPrincipal::Research | ObservationPrincipal::Developer, _) => true,
        (
            ObservationPrincipal::Person(_)
            | ObservationPrincipal::Institution(_)
            | ObservationPrincipal::Public,
            _,
        ) => false,
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum ExplanationRequest {
    Event(EventId),
    Failure(CanwuError),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExplanationStep {
    pub label: String,
    pub event: Option<EventId>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Explanation {
    pub summary: String,
    pub causal_chain: Vec<ExplanationStep>,
}
