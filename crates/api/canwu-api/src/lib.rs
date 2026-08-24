//! Public programmatic, query, semantic-agent, explanation, and debug interfaces.

#![allow(clippy::missing_errors_doc, clippy::module_name_repetitions)]

pub use canwu_core::{
    ArmyId, BoundaryId, CommandAttemptId, CommandId, CommandRequestId, CoreEntityKind,
    DecisionRequestId, DecisionTicketId, DecisionTraceId, DomainEntityKindClass, DomainEntityType,
    DomainKindClass, DomainRecordKind, DomainRecordRef, DomainRecordType, DomainRecordVersionRef,
    DomainRecordVersionSource, DomainValueKindClass, DomainValueType, EntityRef, EventId,
    EvidenceRef, GovernmentId, HolderKnowledgeRecordId, IngressId, KnowledgeHolderPolicy,
    KnowledgeHolderRef, KnowledgeRecordId, KnowledgeRecordKind, KnowledgeSchemaId, LetterId,
    PersonId, RandomDrawId, ResourceId, RouteId, SchemaRegistry, SimulationGranularity,
    TerritoryId, TypeSchema, TypedDomainRecordRef,
};
pub use canwu_event::{CauseRef, EventAudience, EventKind, EventKindError, SimEvent};
pub use canwu_knowledge::{
    ActorKnowledge, ArmyKnowledge, EstimateRange, KnowledgeCursor, KnowledgeHistoryView,
    KnowledgeOrigin, KnowledgeQuery, KnowledgeQueryError, KnowledgeQueryResult, KnowledgeReadCut,
    KnowledgeRecord, KnowledgeRecordDraft, KnowledgeRecordView, KnowledgeSnapshot, KnowledgeSource,
    KnowledgeSubject, KnowledgeSubjectTarget, MAX_KNOWLEDGE_PAGE_SIZE,
};
pub use canwu_routing::{
    DepartureSlot, DurationSample, PlanningSnapshot, ROUTING_ALGORITHM_VERSION, RouteCost,
    RouteLeg, RoutePlan, RoutingAlgorithm, RoutingCache, RoutingConnection, RoutingConnectionRef,
    RoutingEndpoint, RoutingEndpointKind, RoutingError, RoutingNetwork, RoutingNodeRef,
    RoutingPolicy, RoutingRequest, TransferMode, TraversalModel, plan_route,
};
use canwu_sim::Simulation;
pub use canwu_sim::{
    ADMISSION_CURSOR_FORMAT_VERSION, ArchiveProvider, ArchiveStore, ArchiveStoreOutcome,
    ArchivedEvidenceLocator, ArchivedEvidenceReceipt, ArchivedSegmentHeader, Army,
    ArtifactManifest, BoundaryChange, BoundaryContext, BoundaryDirective, BoundaryEmission,
    BoundaryEmissionKind, BoundaryIngressGeneration, BoundaryKnowledgeChange, BoundaryPhase,
    BoundaryProposal, BoundaryReceipt, BoundaryRecord, BoundaryRequest, BoundarySystemContract,
    BoundarySystemHandler, CHECKPOINT_JOURNAL_FORMAT_VERSION, COMMITMENT_FORMAT_VERSION,
    CanwuError, CheckpointJournal, Command, CommandAttemptOutcome, CommandAttemptRecord,
    CommandAuthority, CommandContext, CommandEnvelope, CommandIngress, CommandOutcome,
    CommandPolicyContext, CommandReceipt, CommandRecord, CommandRejection, CommandRequest,
    CommitmentRoots, CompactedSimulation, ControllerDecision, ControllerPolicy, DecisionAction,
    DecisionAttemptErrorCode, DecisionAttemptOutcome, DecisionAttemptRecord, DecisionAuthority,
    DecisionContext, DecisionController, DecisionControllerBinding, DecisionError,
    DecisionErrorCode, DecisionEvaluation, DecisionExternalEvidence, DecisionFactorContribution,
    DecisionIngressRequest, DecisionMutation, DecisionOption, DecisionOptionEvaluation,
    DecisionOrigin, DecisionOutcome, DecisionPolicy, DecisionPolicyIdentity, DecisionPolicyKind,
    DecisionRule, DecisionState, DecisionTicket, DecisionTicketDraft, DecisionTicketState,
    DecisionTrace, DemoIds, DomainRecord, DomainRecordChange, DomainRecordClass, DomainRecordDraft,
    DomainRecordLifecycle, DomainRecordMutation, DomainRecordMutationPolicy, DomainRecordOperation,
    DomainRecordPage, DomainRecordSchema, DomainReference, DomainReferenceSchema,
    DomainReferenceTarget, DomainReferenceTargetKind, ENGINE_VERSION, ErrorCode,
    EvidenceArchiveIndex, EvidenceCursor, EvidenceIndexEntry, EvidenceItemLocator,
    EvidenceJournalKind, EvidenceJournalRoots, EvidenceJournalSegment, EvidenceNestedLocator,
    EvidenceSealToken, ExternalDecisionOption, ExternalDecisionRequest, ExternalDecisionResponse,
    ExternalPolicy, Government, HumanDecisionResponse, HumanPolicy, IngressClass, IngressPayload,
    IngressReceipt, IngressRecord, InteractionPolicy, Issuer, KnowledgeLimitsV1,
    KnowledgeSubjectSchema, KnowledgeSubjectTargetKind, KnowledgeWriteGrant, LetterCargo,
    LetterStatus, LlmModelIdentity, LlmPolicy, MapPoint, ObservationPolicy, OrderedRulePolicy,
    OutboxEntry, PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD,
    PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FORMAT_VERSION, PayloadProperty,
    PayloadRequiredEvidenceContinuationV1, PayloadSchema, PayloadValueType, Person,
    PersonTransitState, PluginActionDescriptor, PluginCommandHandler, PluginComponentRecord,
    PluginDescriptor, PluginIngressDescriptor, PluginIngressRequest, PluginIngressTarget,
    PluginKnowledgeSchema, PluginRegistrar, PluginRegistry, PolicyDecision,
    PreparedDecisionIngress, PreparedEvidenceSeal, QueuedExternalPolicy, QueuedHumanPolicy,
    QueuedLlmPolicy, RUN_CONFIGURATION_FORMAT_VERSION, RUN_MANIFEST_FORMAT_VERSION,
    RandomAlgorithm, RandomDrawAddress, RandomDrawOutcome, RandomDrawProducer, RandomDrawRecord,
    RandomOperationAddressV1, RandomOperationTarget, RandomStreamKey, RandomStreamState,
    ReplayJournal, ReservationAllocation, ReservationDisposition, ReservationOffer,
    ReservationOfferRecord, ReservationPoolKey, ReservationRef, ReservationRequest,
    ReservationRequestRecord, Route, RuleChoice, RulePolicy, RunConfiguration,
    RunConfigurationSnapshot, RunManifest, RunPurpose, SNAPSHOT_FORMAT_VERSION,
    STATE_REVISION_FORMAT_VERSION, Scenario, SeatBinding, SeatPolicy, SimulationCheckpoint,
    SimulationPlugin, SimulationSnapshot, SimulationSystemHandler, SimulationView, StateKey,
    StateVisibility, SystemCadence, SystemContract, SystemDirective, Territory, TracePolicy,
    TransitState, UtilityEvaluator, UtilityPolicy, UtilityProfile, WeightedUtilityEvaluator,
    WeightedUtilityPolicy, WorldSnapshot, canonical_byte_hash, canonical_hash,
    payload_required_evidence_continuation_property_v1,
};
pub use canwu_time::{SimDuration, SimTime};
pub use canwu_transport::{
    CapacityBooking, CapacityBookingId, CapacityBookingStatus, DeliveryCompletionRequest,
    DeliverySaga, Handoff, HandoffId, ItineraryRevision, ItineraryRevisionId,
    ItineraryRevisionReason, LegExecution, LegExecutionId, LegExecutionStatus, MovementInitiative,
    MovementOrder, MovementOrderError, MovementOrderId, MovementSubject, MovementSubjectRole,
    SagaState, TRANSPORT_SEMANTIC_VERSION, TransportError, TransportExecution,
    TransportExecutionId, TransportExecutionState, delivery_completion_operation_key,
};
use serde::{Deserialize, Serialize};

/// Main in-process API. All returned world values are detached snapshots.
pub struct Canwu {
    simulation: Simulation,
}

/// Public API for a live runtime whose sealed evidence segments are stored by the caller.
pub struct CompactedCanwu {
    simulation: CompactedSimulation,
}

impl Canwu {
    #[must_use]
    pub const fn version() -> &'static str {
        ENGINE_VERSION
    }

    pub fn new(seed: u64, scenario: Scenario) -> Result<Self, CanwuError> {
        Ok(Self {
            simulation: Simulation::new(seed, scenario)?,
        })
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
        Ok(Self {
            simulation: Simulation::new_with_plugins(seed, scenario, plugins)?,
        })
    }

    pub fn new_with_manifest(
        seed: u64,
        scenario: Scenario,
        run_manifest: RunManifest,
    ) -> Result<Self, CanwuError> {
        Ok(Self {
            simulation: Simulation::new_with_manifest(seed, scenario, run_manifest)?,
        })
    }

    pub fn new_with_manifest_and_plugins(
        seed: u64,
        scenario: Scenario,
        run_manifest: RunManifest,
        plugins: &[&dyn SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        Ok(Self {
            simulation: Simulation::new_with_manifest_and_plugins(
                seed,
                scenario,
                run_manifest,
                plugins,
            )?,
        })
    }

    pub fn new_with_run_configuration(
        seed: u64,
        scenario: Scenario,
        run_manifest: RunManifest,
        run_configuration: RunConfiguration,
    ) -> Result<Self, CanwuError> {
        Ok(Self {
            simulation: Simulation::new_with_run_configuration(
                seed,
                scenario,
                run_manifest,
                run_configuration,
            )?,
        })
    }

    pub fn new_with_run_configuration_and_plugins(
        seed: u64,
        scenario: Scenario,
        run_manifest: RunManifest,
        run_configuration: RunConfiguration,
        plugins: &[&dyn SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        Ok(Self {
            simulation: Simulation::new_with_run_configuration_and_plugins(
                seed,
                scenario,
                run_manifest,
                run_configuration,
                plugins,
            )?,
        })
    }

    /// Deprecated compatibility scenario. New hosts should use an integration-owned scenario.
    pub fn demo(seed: u64) -> Result<Self, CanwuError> {
        let (simulation, _) = Simulation::demo(seed)?;
        Ok(Self { simulation })
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

    /// Returns when retained evidence first became authoritative.
    ///
    /// Compacted identity-only receipts return `None`; load the archived
    /// evidence body before making decisions that require temporal ordering.
    #[must_use]
    pub fn evidence_time(&self, reference: &EvidenceRef) -> Option<SimTime> {
        self.simulation.evidence_time(reference)
    }

    /// Resolves the retained record body for one exact domain-record version.
    ///
    /// A compacted archive receipt proves existence but does not expose the
    /// version body through this trusted-host query.
    #[must_use]
    pub fn domain_record_version(
        &self,
        reference: &DomainRecordVersionRef,
    ) -> Option<DomainRecord> {
        self.simulation.domain_record_version(reference)
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
    pub const fn decision_state(&self) -> &DecisionState {
        self.simulation.decision_state()
    }

    #[must_use]
    pub fn decision_ticket(&self, id: DecisionTicketId) -> Option<&DecisionTicket> {
        self.simulation.decision_ticket(id)
    }

    #[must_use]
    pub fn decision_traces(&self) -> &[DecisionTrace] {
        self.simulation.decision_traces()
    }

    #[must_use]
    pub fn decision_attempts(&self) -> &[DecisionAttemptRecord] {
        self.simulation.decision_attempts()
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

    pub fn checkpoint(&self) -> Result<SimulationCheckpoint, CanwuError> {
        self.simulation.checkpoint()
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

    pub fn register_plugin<P: SimulationPlugin + ?Sized>(
        &mut self,
        plugin: &P,
    ) -> Result<(), CanwuError> {
        self.simulation.register_plugin(plugin)
    }

    pub fn submit(&mut self, command: CommandEnvelope) -> Result<CommandReceipt, CanwuError> {
        self.simulation.submit(command)
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

    pub fn advance_canonical(
        &mut self,
        duration: SimDuration,
    ) -> Result<Vec<BoundaryReceipt>, CanwuError> {
        self.simulation.advance_canonical(duration)
    }

    pub fn step_canonical(&mut self) -> Result<Option<BoundaryReceipt>, CanwuError> {
        self.simulation.step_canonical()
    }

    pub fn advance(&mut self, duration: SimDuration) -> Result<Vec<SimEvent>, CanwuError> {
        self.simulation.advance(duration)
    }

    pub fn settle_boundary(
        &mut self,
        request: BoundaryRequest,
    ) -> Result<BoundaryReceipt, CanwuError> {
        self.simulation.settle_boundary(request)
    }

    pub fn wait(&mut self, duration: SimDuration) -> Result<Vec<SimEvent>, CanwuError> {
        self.advance(duration)
    }

    pub fn step(&mut self) -> Result<Vec<SimEvent>, CanwuError> {
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
        Ok(Self {
            simulation: Simulation::from_snapshot_json(json)?,
        })
    }

    pub fn from_snapshot_json_with_plugins(
        json: &str,
        plugins: &[&dyn SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        Ok(Self {
            simulation: Simulation::from_snapshot_json_with_plugins(json, plugins)?,
        })
    }

    pub fn from_checkpoint_and_journal(
        checkpoint: SimulationCheckpoint,
        segments: Vec<EvidenceJournalSegment>,
    ) -> Result<Self, CanwuError> {
        Ok(Self {
            simulation: Simulation::from_checkpoint_and_journal(checkpoint, segments)?,
        })
    }

    pub fn from_checkpoint_journal(bundle: CheckpointJournal) -> Result<Self, CanwuError> {
        Ok(Self {
            simulation: Simulation::from_checkpoint_journal(bundle)?,
        })
    }

    pub fn from_checkpoint_journal_with_plugins(
        bundle: CheckpointJournal,
        plugins: &[&dyn SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        Ok(Self {
            simulation: Simulation::from_checkpoint_journal_with_plugins(bundle, plugins)?,
        })
    }

    pub fn from_checkpoint_journal_json(json: &str) -> Result<Self, CanwuError> {
        Ok(Self {
            simulation: Simulation::from_checkpoint_journal_json(json)?,
        })
    }

    pub fn from_checkpoint_journal_json_with_plugins(
        json: &str,
        plugins: &[&dyn SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        Ok(Self {
            simulation: Simulation::from_checkpoint_journal_json_with_plugins(json, plugins)?,
        })
    }

    pub fn replay_from_journal(
        plugins: &[&dyn SimulationPlugin],
        journal: &ReplayJournal,
    ) -> Result<Self, CanwuError> {
        Ok(Self {
            simulation: Simulation::replay_from_journal(plugins, journal)?,
        })
    }

    pub fn replay_from_journal_json(
        plugins: &[&dyn SimulationPlugin],
        json: &str,
    ) -> Result<Self, CanwuError> {
        Ok(Self {
            simulation: Simulation::replay_from_journal_json(plugins, json)?,
        })
    }

    #[must_use]
    pub fn fork(&self) -> Self {
        Self {
            simulation: self.simulation.fork(),
        }
    }

    /// Trusted host/admin holder query. Player-facing callers must use a
    /// restricted [`CanwuViewer`].
    pub fn admin_query_knowledge(
        &self,
        holder: KnowledgeHolderRef,
        query: &KnowledgeQuery,
    ) -> Result<KnowledgeQueryResult, CanwuError> {
        self.simulation
            .knowledge()
            .query_current(
                holder,
                query,
                self.simulation.boundaries().last().map(|value| value.id),
            )
            .map_err(map_knowledge_query_error)
    }

    /// Creates the restricted viewer dictated entirely by the persisted run
    /// policy and seat binding.
    pub fn viewer(&self) -> Result<CanwuViewer<'_>, CanwuError> {
        let principal = self.declared_observation_principal()?;
        Ok(CanwuViewer {
            canwu: self,
            context: KnowledgeViewContext { principal },
        })
    }

    /// Character-seat and compatibility convenience. It never upgrades an
    /// institution, public, research, or developer policy to a person.
    pub fn viewer_for_actor(&self, actor: PersonId) -> Result<CanwuViewer<'_>, CanwuError> {
        if !self.entity_exists(&EntityRef::Person(actor)) {
            return Err(CanwuError::new(
                ErrorCode::ActorNotFound,
                format!("actor {actor} was not found"),
            ));
        }
        let principal = match self.run_configuration().declared() {
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
            canwu: self,
            context: KnowledgeViewContext { principal },
        })
    }

    pub fn viewer_context(&self, actor: PersonId) -> Result<ViewerContext, CanwuError> {
        let viewer = self.viewer_for_actor(actor)?;
        Ok(ViewerContext {
            principal: viewer.context.principal.clone(),
            observation: ObservationPolicy::ActorBound,
            checkpoint_hash: self.checkpoint_hash().to_owned(),
        })
    }

    fn declared_observation_principal(&self) -> Result<ObservationPrincipal, CanwuError> {
        let Some(configuration) = self.run_configuration().declared() else {
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

    #[must_use]
    pub fn explain(&self, request: &ExplanationRequest) -> Explanation {
        match request {
            ExplanationRequest::Event(event_id) => self.explain_event(*event_id),
            ExplanationRequest::Failure(error) => Explanation {
                summary: error.message.clone(),
                causal_chain: vec![ExplanationStep {
                    label: format!("Validation failed: {:?}", error.code),
                    event: None,
                }],
            },
        }
    }

    fn explain_event(&self, event_id: EventId) -> Explanation {
        let mut chain = Vec::new();
        let events = self.events();
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
}

fn event_by_id(events: &[SimEvent], event_id: EventId) -> Option<&SimEvent> {
    let index = usize::try_from(event_id.get().checked_sub(1)?).ok()?;
    events.get(index).filter(|event| event.id == event_id)
}

impl CompactedCanwu {
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

    pub fn evidence_cursor(&self) -> Result<EvidenceCursor, CanwuError> {
        self.simulation.evidence_cursor()
    }

    pub fn checkpoint(&self) -> Result<SimulationCheckpoint, CanwuError> {
        self.simulation.checkpoint()
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

    #[must_use]
    pub fn knowledge(&self) -> &KnowledgeSnapshot {
        self.simulation.knowledge()
    }

    #[must_use]
    pub fn domain_record(&self, reference: &DomainRecordRef) -> Option<&DomainRecord> {
        self.simulation.domain_record(reference)
    }

    #[must_use]
    pub fn typed_domain_record<T: DomainRecordType>(
        &self,
        reference: &TypedDomainRecordRef<T>,
    ) -> Option<&DomainRecord> {
        self.simulation.typed_domain_record(reference)
    }

    #[must_use]
    pub const fn decision_state(&self) -> &DecisionState {
        self.simulation.decision_state()
    }

    #[must_use]
    pub fn decision_ticket(&self, id: DecisionTicketId) -> Option<&DecisionTicket> {
        self.simulation.decision_ticket(id)
    }

    #[must_use]
    pub fn decision_traces(&self) -> &[DecisionTrace] {
        self.simulation.decision_traces()
    }

    #[must_use]
    pub fn decision_attempts(&self) -> &[DecisionAttemptRecord] {
        self.simulation.decision_attempts()
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
    canwu: &'a Canwu,
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
        self.canwu.admin_query_knowledge(holder, query)
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
        if !knowledge_holder_exists(self.canwu, &holder) {
            return Err(CanwuError::new(
                ErrorCode::InvalidKnowledgeHolder,
                "the requested knowledge holder does not exist",
            ));
        }
        self.canwu.admin_query_knowledge(holder, query)
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
        if !knowledge_holder_exists(self.canwu, holder) {
            return Err(CanwuError::new(
                ErrorCode::InvalidKnowledgeHolder,
                "the requested knowledge holder does not exist",
            ));
        }
        let index = usize::try_from(record.get().saturating_sub(1)).map_err(|_| {
            CanwuError::new(
                ErrorCode::KnowledgeRecordNotFound,
                "holder-relative knowledge record ID is outside the supported range",
            )
        })?;
        self.canwu
            .knowledge()
            .for_holder(holder)
            .and_then(|records| records.values().nth(index))
            .cloned()
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
            checkpoint_hash: self.canwu.checkpoint_hash().to_owned(),
        };
        self.canwu
            .events()
            .iter()
            .filter(|event| event.timestamp > since)
            .filter_map(|event| {
                let audience = self.canwu.simulation.event_audience(event);
                visible_change(&context, event, &audience)
            })
            .collect()
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

fn knowledge_holder_exists(canwu: &Canwu, holder: &KnowledgeHolderRef) -> bool {
    match holder {
        KnowledgeHolderRef::Person(actor) => canwu.entity_exists(&EntityRef::Person(*actor)),
        KnowledgeHolderRef::Entity(entity) => canwu.entity_exists(entity),
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
        EventAudience::AffectedActors => viewer
            .principal
            .person()
            .is_some_and(|actor| event.affected_entities.contains(&EntityRef::Person(actor))),
        EventAudience::Private => false,
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

// The former public API tests exercised the retired built-in reference world.
// Integration coverage now lives with `canwu-reference-world`.
#[cfg(any())]
mod tests {
    use super::*;

    fn manifest_for_configuration(
        scenario: &Scenario,
        configuration: &RunConfiguration,
    ) -> RunManifest {
        let scenario_manifest =
            ArtifactManifest::for_scenario("fixture", "viewer-scenario", "1", scenario)
                .expect("scenario manifest should hash");
        let configuration_manifest = ArtifactManifest::for_run_configuration(
            "fixture",
            "viewer-configuration",
            "1",
            configuration,
        )
        .expect("run configuration manifest should hash");
        RunManifest::declared(scenario_manifest, configuration_manifest)
    }

    struct VisibilityPlugin {
        audience: EventAudience,
    }

    #[allow(clippy::unnecessary_wraps)]
    fn visibility_system(
        _view: &SimulationView<'_>,
        event: &SimEvent,
    ) -> Result<Vec<SystemDirective>, CanwuError> {
        if !event.kind.is_type("move_ordered") {
            return Ok(Vec::new());
        }
        Ok(vec![SystemDirective::Emit {
            event_type: "notice".to_owned(),
            summary: "a plugin visibility notice".to_owned(),
            affected: vec![EntityRef::Person(PersonId::new(1))],
        }])
    }

    impl SimulationPlugin for VisibilityPlugin {
        fn name(&self) -> &'static str {
            "visibility-test"
        }

        fn version(&self) -> &'static str {
            "test-v1"
        }

        fn semantic_hash(&self) -> &'static str {
            "0000000000000000000000000000000000000000000000000000000000000001"
        }

        fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
            registrar.register_event_audience("notice", self.audience.clone())?;
            registrar.register_system(
                SystemContract::event_driven(
                    "emit-notice",
                    BoundaryPhase::PerspectiveAndReportMaterialization,
                ),
                visibility_system,
            )
        }
    }

    #[test]
    fn plugin_event_visibility_respects_public_actor_and_private_audiences() {
        let ids = Canwu::demo_ids();
        let event = SimEvent {
            id: EventId::new(1),
            timestamp: SimTime::EPOCH,
            kind: EventKind::plugin("visibility-test", "notice"),
            affected_entities: vec![EntityRef::Person(ids.commander)],
            summary: "notice".to_owned(),
            cause: None,
            correlation_id: 1,
        };
        let actor = ViewerContext {
            principal: ObservationPrincipal::Person(ids.commander),
            observation: ObservationPolicy::ActorBound,
            checkpoint_hash: String::new(),
        };
        let observer = ViewerContext {
            principal: ObservationPrincipal::Person(ids.observer),
            observation: ObservationPolicy::ActorBound,
            checkpoint_hash: String::new(),
        };
        let public_observer = ViewerContext {
            principal: ObservationPrincipal::Public,
            observation: ObservationPolicy::PublicObserver,
            checkpoint_hash: String::new(),
        };
        let research = ViewerContext {
            principal: ObservationPrincipal::Research,
            observation: ObservationPolicy::ResearchFull,
            checkpoint_hash: String::new(),
        };

        assert!(visible_change(&actor, &event, &EventAudience::Public).is_some());
        assert!(visible_change(&public_observer, &event, &EventAudience::Public).is_some());
        assert!(visible_change(&actor, &event, &EventAudience::Actor(ids.commander)).is_some());
        assert!(visible_change(&observer, &event, &EventAudience::Actor(ids.commander)).is_none());
        assert!(visible_change(&observer, &event, &EventAudience::Private).is_none());
        assert!(visible_change(&research, &event, &EventAudience::Private).is_some());
    }

    #[test]
    fn observe_changes_since_uses_persisted_plugin_audience() {
        let ids = Canwu::demo_ids();
        let mut canwu = Canwu::demo(35).expect("demo should load");
        canwu
            .register_plugin(&VisibilityPlugin {
                audience: EventAudience::Public,
            })
            .expect("visibility plugin should register");
        let since = SimTime::from_minutes(-1);
        canwu
            .act(
                ids.commander,
                SemanticAction::MoveEntity {
                    subject: EntityRef::Army(ids.army),
                    destination: ids.eastern_territory,
                    cargo: Vec::new(),
                },
            )
            .expect("movement should emit plugin notice");

        let observer = canwu
            .observe(
                ids.observer,
                &ObserveRequest {
                    focus: ObservationFocus::Changes,
                    since: Some(since),
                },
            )
            .expect("observer should be authorized");
        assert!(
            observer
                .changes_since
                .iter()
                .any(|change| change.summary == "a plugin visibility notice")
        );

        let snapshot_json = canwu
            .snapshot_json()
            .expect("audience declaration should serialize");
        let restored = Canwu::from_snapshot_json_with_plugins(
            &snapshot_json,
            &[&VisibilityPlugin {
                audience: EventAudience::Public,
            }],
        )
        .expect("audience declaration should survive snapshot loading");
        let restored_observer = restored
            .observe(
                ids.observer,
                &ObserveRequest {
                    focus: ObservationFocus::Changes,
                    since: Some(since),
                },
            )
            .expect("restored observer should be authorized");
        assert!(
            restored_observer
                .changes_since
                .iter()
                .any(|change| change.summary == "a plugin visibility notice")
        );
    }

    #[test]
    fn observe_with_viewer_revalidates_input_control_context() {
        let canwu = Canwu::demo(35).expect("demo should load");
        let escalated = ViewerContext {
            principal: ObservationPrincipal::Research,
            observation: ObservationPolicy::ResearchFull,
            checkpoint_hash: canwu.checkpoint_hash().to_owned(),
        };

        let error = canwu
            .observe_with_viewer(&escalated, &ObserveRequest::default())
            .expect_err("a caller cannot self-escalate the observation policy");
        assert_eq!(error.code, ErrorCode::InvalidAuthority);
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn restricted_viewer_derives_principal_and_rejects_public_private_reads() {
        let (scenario, ids) = canwu_sim::demo_scenario();
        let actor = Canwu::demo(69).expect("actor viewer fixture should initialize");
        let actor_viewer = actor
            .viewer_for_actor(ids.commander)
            .expect("legacy character viewer should derive");
        let error = actor_viewer
            .audit_knowledge_record(
                &KnowledgeHolderRef::Person(ids.commander),
                HolderKnowledgeRecordId::new(1),
            )
            .expect_err("actor viewers cannot read audit-bearing records");
        assert_eq!(error.code, ErrorCode::InvalidKnowledgeAuthority);

        let public_configuration = RunConfiguration::read_only_observer();
        let public_manifest = manifest_for_configuration(&scenario, &public_configuration);
        let public = Canwu::new_with_run_configuration(
            71,
            scenario.clone(),
            public_manifest,
            public_configuration,
        )
        .expect("public viewer fixture should initialize");
        let public_viewer = public.viewer().expect("public principal should derive");
        assert_eq!(public_viewer.principal(), &ObservationPrincipal::Public);
        let error = public_viewer
            .query_knowledge(&KnowledgeQuery::default())
            .expect_err("public principal cannot read a private ledger");
        assert_eq!(error.code, ErrorCode::InvalidKnowledgeAuthority);
        let error = public_viewer
            .query_holder_knowledge(
                KnowledgeHolderRef::Person(ids.commander),
                &KnowledgeQuery::default(),
            )
            .expect_err("an arbitrary valid actor ID cannot upgrade a public viewer");
        assert_eq!(error.code, ErrorCode::InvalidKnowledgeAuthority);
        let error = public_viewer
            .audit_knowledge_record(
                &KnowledgeHolderRef::Person(ids.commander),
                HolderKnowledgeRecordId::new(1),
            )
            .expect_err("public viewers cannot read audit-bearing records");
        assert_eq!(error.code, ErrorCode::InvalidKnowledgeAuthority);

        let institution_configuration = RunConfiguration {
            format_version: RUN_CONFIGURATION_FORMAT_VERSION,
            purpose: RunPurpose::Play,
            controller: ControllerPolicy::HumanRoleBound,
            seat: SeatPolicy::InstitutionBound,
            observation: ObservationPolicy::ActorBound,
            interaction: InteractionPolicy::EraInternalCommands,
            trace: TracePolicy::Causal,
            seat_binding: Some(SeatBinding {
                seat_id: "institution-seat".to_owned(),
                controller_id: "institution-controller".to_owned(),
                actor: Some(ids.commander),
                institution: Some(EntityRef::Government(ids.government)),
                permission_profile_id: "institution-profile".to_owned(),
            }),
            declared_interventions: Vec::new(),
            diagnostic_commands_enabled: false,
            require_idempotency_keys: true,
        };
        let institution_manifest =
            manifest_for_configuration(&scenario, &institution_configuration);
        let institution = Canwu::new_with_run_configuration(
            73,
            scenario.clone(),
            institution_manifest,
            institution_configuration,
        )
        .expect("institution viewer fixture should initialize");
        let institution_viewer = institution
            .viewer()
            .expect("institution principal should derive");
        assert_eq!(
            institution_viewer.principal(),
            &ObservationPrincipal::Institution(EntityRef::Government(ids.government))
        );
        assert_eq!(
            institution_viewer
                .query_knowledge(&KnowledgeQuery::default())
                .expect("institution may query only its bound ledger")
                .holder,
            KnowledgeHolderRef::Entity(EntityRef::Government(ids.government))
        );
        assert!(institution.viewer_for_actor(ids.commander).is_err());
        let error = institution_viewer
            .audit_knowledge_record(
                &KnowledgeHolderRef::Entity(EntityRef::Government(ids.government)),
                HolderKnowledgeRecordId::new(1),
            )
            .expect_err("institution viewers cannot read audit-bearing records");
        assert_eq!(error.code, ErrorCode::InvalidKnowledgeAuthority);

        let mut research_configuration = RunConfiguration::read_only_observer();
        research_configuration.observation = ObservationPolicy::ResearchFull;
        let research_manifest = manifest_for_configuration(&scenario, &research_configuration);
        let research = Canwu::new_with_run_configuration(
            79,
            scenario,
            research_manifest,
            research_configuration,
        )
        .expect("research viewer fixture should initialize");
        let research_viewer = research.viewer().expect("research principal should derive");
        assert_eq!(research_viewer.principal(), &ObservationPrincipal::Research);
        assert_eq!(
            research_viewer
                .query_holder_knowledge(
                    KnowledgeHolderRef::Person(ids.commander),
                    &KnowledgeQuery::default(),
                )
                .expect("research may explicitly select an existing holder")
                .holder,
            KnowledgeHolderRef::Person(ids.commander)
        );
    }

    #[test]
    fn detached_viewer_context_is_bound_to_the_authorized_checkpoint() {
        let mut canwu = Canwu::demo(83).expect("demo should load");
        let ids = Canwu::demo_ids();
        let context = canwu
            .viewer_context(ids.commander)
            .expect("the commander should receive a detached viewer context");

        canwu
            .act(
                ids.commander,
                SemanticAction::MoveEntity {
                    subject: EntityRef::Army(ids.army),
                    destination: ids.eastern_territory,
                    cargo: Vec::new(),
                },
            )
            .expect("the authoritative checkpoint should advance");
        let error = canwu
            .observe_with_viewer(&context, &ObserveRequest::default())
            .expect_err("a context from an older checkpoint must be rejected");
        assert_eq!(error.code, ErrorCode::InvalidAuthority);

        let refreshed = canwu
            .viewer_context(ids.commander)
            .expect("the current checkpoint should issue a fresh context");
        canwu
            .observe_with_viewer(&refreshed, &ObserveRequest::default())
            .expect("the refreshed context should remain authorized");
    }

    #[test]
    fn actor_relative_observation_does_not_leak_arrival() {
        let mut canwu = Canwu::demo(35).expect("demo should load");
        let ids = Canwu::demo_ids();
        canwu
            .act(
                ids.commander,
                SemanticAction::MoveEntity {
                    subject: EntityRef::Army(ids.army),
                    destination: ids.eastern_territory,
                    cargo: Vec::new(),
                },
            )
            .expect("commander can move army");
        canwu
            .advance(SimDuration::days(1))
            .expect("arrival should execute");

        assert_eq!(
            canwu.world().army(ids.army).expect("army exists").location,
            ids.eastern_territory
        );
        let observer = canwu
            .observe(ids.observer, &ObserveRequest::default())
            .expect("observer exists");
        assert_eq!(
            observer.known_armies[0].known_location,
            Some(ids.central_territory)
        );
        let person_rows = canwu
            .query_as(ids.observer, &Query::all(QueryEntity::Person))
            .expect("actor query should succeed");
        assert_eq!(person_rows.rows.len(), 1);
        assert_eq!(person_rows.rows[0].get("id"), Some(&json!(ids.observer)));
        for entity in [
            QueryEntity::Government,
            QueryEntity::Territory,
            QueryEntity::Route,
        ] {
            assert!(
                canwu
                    .query_as(ids.observer, &Query::all(entity))
                    .expect("actor query should succeed")
                    .rows
                    .is_empty()
            );
        }
        assert!(
            canwu
                .inspect(
                    ids.observer,
                    &EntityRef::Person(ids.commander),
                    DetailLevel::RawFields,
                )
                .expect("inspection should succeed")
                .fields
                .is_empty()
        );
        assert!(
            canwu
                .inspect(
                    ids.observer,
                    &EntityRef::Territory(ids.eastern_territory),
                    DetailLevel::RawFields,
                )
                .expect("inspection should succeed")
                .fields
                .is_empty()
        );

        canwu
            .advance(SimDuration::days(3))
            .expect("report should arrive");
        let updated = canwu
            .observe(ids.observer, &ObserveRequest::default())
            .expect("observer exists");
        assert_eq!(
            updated.known_armies[0].known_location,
            Some(ids.eastern_territory)
        );
    }

    #[test]
    fn self_move_is_an_actor_bound_order_movement() {
        let mut canwu = Canwu::demo(35).expect("demo should load");
        let ids = Canwu::demo_ids();
        let actions = canwu
            .available_actions(ids.commander)
            .expect("commander actions should be available");
        assert!(actions.iter().any(|action| {
            action.action_type == "self_move"
                && action.payload["destination"] == json!(ids.eastern_territory)
        }));

        canwu
            .act(
                ids.commander,
                SemanticAction::SelfMove {
                    destination: ids.eastern_territory,
                    cargo: Vec::new(),
                },
            )
            .expect("a person may order their own movement");
        assert!(
            canwu
                .world()
                .person(ids.commander)
                .expect("commander exists")
                .transit
                .is_some()
        );
    }

    #[test]
    fn debug_mutation_uses_validated_command_and_provenance() {
        let mut canwu = Canwu::demo(35).expect("demo should load");
        let ids = Canwu::demo_ids();
        let result = canwu.submit(CommandEnvelope::new(
            Issuer::Debug,
            Command::DebugSetArmyMorale {
                army: ids.army,
                morale: 37,
            },
        ));
        let receipt = result.expect("debug command should validate");
        assert_eq!(
            canwu.world().army(ids.army).expect("army exists").morale,
            37
        );
        let explanation = canwu.explain(&ExplanationRequest::Event(receipt.emitted_events[0]));
        assert!(explanation.causal_chain.len() >= 2);
    }

    #[test]
    fn public_checkpoint_journal_round_trip_is_exact() {
        let mut canwu = Canwu::demo(35).expect("demo should load");
        let ids = Canwu::demo_ids();
        canwu
            .submit(CommandEnvelope::new(
                Issuer::Actor(ids.commander),
                Command::OrderMovement {
                    subject: EntityRef::Army(ids.army),
                    destination: ids.eastern_territory,
                    cargo: Vec::new(),
                },
            ))
            .expect("movement should be accepted");
        canwu
            .advance(SimDuration::days(1))
            .expect("scheduled work should execute");

        let checkpoint = canwu.checkpoint().expect("current state should checkpoint");
        assert!(checkpoint.state.events.is_empty());
        assert_eq!(
            checkpoint.journal_end,
            canwu
                .evidence_cursor()
                .expect("journal cursor should be representable")
        );
        let json = canwu
            .checkpoint_journal_json()
            .expect("checkpoint journal should serialize");
        let restored = Canwu::from_checkpoint_journal_json(&json)
            .expect("checkpoint journal should restore through the public API");
        assert_eq!(restored.snapshot(), canwu.snapshot());

        canwu
            .settle_boundary(BoundaryRequest::at(canwu.time()))
            .expect("a public boundary should complete the live evidence tail");
        let expected = canwu.snapshot();
        let mut compact = canwu
            .into_compacted()
            .expect("the public API should enter compact mode");
        let segment = compact
            .seal_evidence()
            .expect("the public compact API should seal evidence")
            .expect("the public compact API should return a segment");
        let compact_checkpoint = compact
            .checkpoint()
            .expect("the public compact API should checkpoint");
        assert_eq!(
            compact
                .snapshot_with_segments(vec![segment.clone()])
                .expect("the public compact API should reconstruct its snapshot"),
            expected
        );
        let restored_compact =
            CompactedCanwu::from_checkpoint_and_journal(compact_checkpoint, vec![segment])
                .expect("the public compact API should restore from its archive");
        assert_eq!(
            restored_compact
                .snapshot_with_segments(Vec::new())
                .expect("the restored compact API should retain validated evidence"),
            expected
        );
    }
}
