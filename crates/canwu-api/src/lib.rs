//! Public programmatic, query, semantic-agent, explanation, and debug interfaces.

#![allow(clippy::missing_errors_doc, clippy::module_name_repetitions)]

pub use canwu_core::{
    ArmyId, BoundaryId, CommandAttemptId, CommandId, CommandRequestId, CoreEntityKind,
    DecisionRequestId, DecisionTicketId, DecisionTraceId, DomainEntityKindClass, DomainEntityType,
    DomainKindClass, DomainRecordKind, DomainRecordRef, DomainRecordType, DomainValueKindClass,
    DomainValueType, EntityRef, EventId, GovernmentId, IngressId, PersonId, RandomDrawId, RouteId,
    SchemaRegistry, TerritoryId, TypeSchema, TypedDomainRecordRef,
};
pub use canwu_event::{CauseRef, EventAudience, EventKind, SimEvent};
pub use canwu_knowledge::{
    ActorKnowledge, ArmyKnowledge, EstimateRange, KnowledgeSnapshot, KnowledgeSource,
};
pub use canwu_sim::{
    ADMISSION_CURSOR_FORMAT_VERSION, ArtifactManifest, BoundaryChange, BoundaryContext,
    BoundaryDirective, BoundaryEmission, BoundaryEmissionKind, BoundaryIngressGeneration,
    BoundaryPhase, BoundaryProposal, BoundaryReceipt, BoundaryRecord, BoundaryRequest,
    BoundarySystemContract, BoundarySystemHandler, CHECKPOINT_JOURNAL_FORMAT_VERSION,
    COMMITMENT_FORMAT_VERSION, CanwuError, CheckpointJournal, Command, CommandAttemptOutcome,
    CommandAttemptRecord, CommandAuthority, CommandContext, CommandEnvelope, CommandIngress,
    CommandOutcome, CommandPolicyContext, CommandReceipt, CommandRecord, CommandRejection,
    CommandRequest, CommitmentRoots, CompactedSimulation, ControllerDecision, ControllerPolicy,
    DecisionAction, DecisionAttemptErrorCode, DecisionAttemptOutcome, DecisionAttemptRecord,
    DecisionAuthority, DecisionContext, DecisionController, DecisionControllerBinding,
    DecisionError, DecisionErrorCode, DecisionEvaluation, DecisionExternalEvidence,
    DecisionFactorContribution, DecisionIngressRequest, DecisionMutation, DecisionOption,
    DecisionOptionEvaluation, DecisionOrigin, DecisionOutcome, DecisionPolicy,
    DecisionPolicyIdentity, DecisionPolicyKind, DecisionRule, DecisionState, DecisionTicket,
    DecisionTicketDraft, DecisionTicketState, DecisionTrace, DemoIds, DomainRecord,
    DomainRecordChange, DomainRecordClass, DomainRecordDraft, DomainRecordLifecycle,
    DomainRecordMutation, DomainRecordOperation, DomainRecordSchema, DomainReference,
    DomainReferenceSchema, DomainReferenceTarget, DomainReferenceTargetKind, ENGINE_VERSION,
    ErrorCode, EvidenceCursor, EvidenceJournalSegment, ExternalDecisionOption,
    ExternalDecisionRequest, ExternalDecisionResponse, ExternalPolicy, HumanDecisionResponse,
    HumanPolicy, IngressClass, IngressPayload, IngressReceipt, IngressRecord, InteractionPolicy,
    Issuer, LlmModelIdentity, LlmPolicy, ObservationPolicy, OrderedRulePolicy, PayloadProperty,
    PayloadSchema, PayloadValueType, PluginActionDescriptor, PluginCommandHandler,
    PluginComponentRecord, PluginDescriptor, PluginIngressDescriptor, PluginIngressRequest,
    PluginRegistrar, PluginRegistry, PolicyDecision, PreparedDecisionIngress, QueuedExternalPolicy,
    QueuedHumanPolicy, QueuedLlmPolicy, RUN_CONFIGURATION_FORMAT_VERSION,
    RUN_MANIFEST_FORMAT_VERSION, RandomAlgorithm, RandomDrawOutcome, RandomDrawProducer,
    RandomDrawRecord, RandomStreamKey, RandomStreamState, ReplayJournal, ReservationAllocation,
    ReservationDisposition, ReservationOffer, ReservationOfferRecord, ReservationPoolKey,
    ReservationRef, ReservationRequest, ReservationRequestRecord, RuleChoice, RulePolicy,
    RunConfiguration, RunConfigurationSnapshot, RunManifest, RunPurpose, SNAPSHOT_FORMAT_VERSION,
    STATE_REVISION_FORMAT_VERSION, Scenario, SeatBinding, SeatPolicy, SimulationCheckpoint,
    SimulationPlugin, SimulationSnapshot, SimulationSystemHandler, SimulationView, StateKey,
    StateVisibility, SystemCadence, SystemContract, SystemDirective, TracePolicy, UtilityEvaluator,
    UtilityPolicy, UtilityProfile, WeightedUtilityEvaluator, WeightedUtilityPolicy,
};
pub use canwu_time::{SimDuration, SimTime};
pub use canwu_world::{
    Army, Government, MapPoint, Person, Route, Territory, TransitState, WorldDiff, WorldSnapshot,
};

use canwu_sim::Simulation;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Main in-process API. All returned world values are detached snapshots.
pub struct Canwu {
    simulation: Simulation,
}

/// Public facade for a live runtime whose sealed evidence segments are stored by the caller.
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

    pub fn demo(seed: u64) -> Result<Self, CanwuError> {
        let (simulation, _) = Simulation::demo(seed)?;
        Ok(Self { simulation })
    }

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

    #[must_use]
    pub fn world(&self) -> WorldSnapshot {
        self.simulation.world()
    }

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

    pub fn replay(
        seed: u64,
        scenario: Scenario,
        commands: &[CommandRecord],
        final_time: SimTime,
    ) -> Result<Self, CanwuError> {
        Ok(Self {
            simulation: Simulation::replay(seed, scenario, commands, final_time)?,
        })
    }

    pub fn replay_with_plugins(
        seed: u64,
        scenario: Scenario,
        plugins: &[&dyn SimulationPlugin],
        commands: &[CommandRecord],
        final_time: SimTime,
    ) -> Result<Self, CanwuError> {
        Ok(Self {
            simulation: Simulation::replay_with_plugins(
                seed, scenario, plugins, commands, final_time,
            )?,
        })
    }

    pub fn replay_with_boundaries(
        seed: u64,
        scenario: Scenario,
        plugins: &[&dyn SimulationPlugin],
        commands: &[CommandRecord],
        boundaries: &[BoundaryRecord],
        final_time: SimTime,
    ) -> Result<Self, CanwuError> {
        Ok(Self {
            simulation: Simulation::replay_with_boundaries(
                seed, scenario, plugins, commands, boundaries, final_time,
            )?,
        })
    }

    pub fn replay_with_run_manifest(
        seed: u64,
        scenario: Scenario,
        run_manifest: RunManifest,
        plugins: &[&dyn SimulationPlugin],
        commands: &[CommandRecord],
        boundaries: &[BoundaryRecord],
        final_time: SimTime,
    ) -> Result<Self, CanwuError> {
        Ok(Self {
            simulation: Simulation::replay_with_run_manifest(
                seed,
                scenario,
                run_manifest,
                plugins,
                commands,
                boundaries,
                final_time,
            )?,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn replay_with_run_configuration(
        seed: u64,
        scenario: Scenario,
        run_manifest: RunManifest,
        run_configuration: RunConfiguration,
        plugins: &[&dyn SimulationPlugin],
        commands: &[CommandRecord],
        command_attempts: &[CommandAttemptRecord],
        boundaries: &[BoundaryRecord],
        final_time: SimTime,
    ) -> Result<Self, CanwuError> {
        Ok(Self {
            simulation: Simulation::replay_with_run_configuration(
                seed,
                scenario,
                run_manifest,
                run_configuration,
                plugins,
                commands,
                command_attempts,
                boundaries,
                final_time,
            )?,
        })
    }

    pub fn replay_from_journal(
        scenario: Scenario,
        plugins: &[&dyn SimulationPlugin],
        journal: &ReplayJournal,
    ) -> Result<Self, CanwuError> {
        Ok(Self {
            simulation: Simulation::replay_from_journal(scenario, plugins, journal)?,
        })
    }

    #[must_use]
    pub fn fork(&self) -> Self {
        Self {
            simulation: self.simulation.fork(),
        }
    }

    #[must_use]
    pub fn diff(&self, other: &Self) -> WorldDiff {
        WorldDiff::between(&self.world(), &other.world())
    }

    #[must_use]
    pub fn query(&self, query: &Query) -> QueryResult {
        run_query(&self.world(), self.events(), query)
    }

    pub fn query_as(&self, actor: PersonId, query: &Query) -> Result<QueryResult, CanwuError> {
        if self.world().person(actor).is_none() {
            return Err(CanwuError::new(
                ErrorCode::ActorNotFound,
                format!("actor {actor} was not found"),
            ));
        }
        Ok(run_actor_query(
            &self.world(),
            actor,
            self.knowledge().for_actor(actor),
            query,
        ))
    }

    /// Builds an authorized viewer context from the persisted run policy.
    ///
    /// Callers cannot select a stronger observation policy through an
    /// observation request; the run configuration is the input-control
    /// boundary for actor-relative versus research projections.
    pub fn viewer_context(&self, actor: PersonId) -> Result<ViewerContext, CanwuError> {
        if self.world().person(actor).is_none() {
            return Err(CanwuError::new(
                ErrorCode::ActorNotFound,
                format!("actor {actor} was not found"),
            ));
        }
        let observation = match self.run_configuration().declared() {
            Some(configuration) => {
                if configuration.observation == ObservationPolicy::ActorBound
                    && configuration
                        .seat_binding
                        .as_ref()
                        .and_then(|binding| binding.actor)
                        != Some(actor)
                {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidAuthority,
                        format!("actor {actor} is not bound to the active observation seat"),
                    ));
                }
                configuration.observation
            }
            None => ObservationPolicy::ActorBound,
        };
        Ok(ViewerContext { actor, observation })
    }

    pub fn observe(
        &self,
        actor: PersonId,
        request: &ObserveRequest,
    ) -> Result<AgentContext, CanwuError> {
        let viewer = self.viewer_context(actor)?;
        self.observe_with_viewer(&viewer, request)
    }

    /// Projects the simulation for a previously authorized viewer context.
    ///
    /// The context controls only the player-facing projection. Plugin system
    /// subscriptions and state read permissions remain enforced by the
    /// simulation runtime and are not widened by this method.
    pub fn observe_with_viewer(
        &self,
        viewer: &ViewerContext,
        request: &ObserveRequest,
    ) -> Result<AgentContext, CanwuError> {
        let authorized = self.viewer_context(viewer.actor)?;
        if authorized != *viewer {
            return Err(CanwuError::new(
                ErrorCode::InvalidAuthority,
                format!(
                    "actor {} is not authorized for this observation context",
                    viewer.actor
                ),
            ));
        }
        let actor = viewer.actor;
        let world = self.world();
        let person = world.person(actor).ok_or_else(|| {
            CanwuError::new(
                ErrorCode::ActorNotFound,
                format!("actor {actor} was not found"),
            )
        })?;
        let knowledge = self.knowledge().for_actor(actor);
        let known_armies = match knowledge {
            Some(records) => records
                .armies
                .values()
                .map(|record| known_army_view(self.time(), record))
                .collect::<Result<Vec<_>, _>>()?,
            None => Vec::new(),
        };
        let changes_since = request.since.map_or_else(Vec::new, |since| {
            self.events()
                .iter()
                .filter(|event| event.timestamp > since)
                .filter_map(|event| {
                    let audience = self.simulation.event_audience(event);
                    visible_change(viewer, event, &audience)
                })
                .collect()
        });
        let pending_actions = world
            .armies
            .iter()
            .filter(|army| army.commander == actor)
            .filter_map(|army| {
                army.transit.as_ref().map(|transit| PendingCommitment {
                    summary: format!(
                        "{} is moving from {} to {}",
                        army.name, transit.from, transit.to
                    ),
                    due_at: transit.arrives_at,
                })
            })
            .collect();
        Ok(AgentContext {
            identity: AgentIdentity {
                person: person.id,
                name: person.name.clone(),
                roles: person.roles.clone(),
            },
            current_time: self.time(),
            current_location: person.current_location,
            focus: request.focus.clone(),
            known_armies,
            changes_since,
            pending_actions,
            available_actions: self.available_actions(actor)?,
        })
    }

    pub fn inspect(
        &self,
        actor: PersonId,
        entity: &EntityRef,
        detail: DetailLevel,
    ) -> Result<Inspection, CanwuError> {
        let world = self.world();
        let actor_state = world.person(actor).ok_or_else(|| {
            CanwuError::new(
                ErrorCode::ActorNotFound,
                format!("actor {actor} was not found"),
            )
        })?;
        let fields = match entity {
            EntityRef::Army(army_id) => {
                let record = self
                    .knowledge()
                    .for_actor(actor)
                    .and_then(|knowledge| knowledge.armies.get(army_id));
                let Some(record) = record else {
                    return Ok(Inspection {
                        entity: entity.clone(),
                        detail,
                        summary: "No reliable information is available about this army".to_owned(),
                        fields: BTreeMap::new(),
                    });
                };
                let mut fields = BTreeMap::from([
                    ("known_name".to_owned(), json!(record.known_name)),
                    ("known_location".to_owned(), json!(record.known_location)),
                    (
                        "estimated_strength".to_owned(),
                        json!(record.estimated_strength),
                    ),
                    ("observed_at".to_owned(), json!(record.observed_at)),
                    (
                        "confidence_per_mille".to_owned(),
                        json!(record.confidence_per_mille),
                    ),
                ]);
                if matches!(detail, DetailLevel::RawFields) {
                    fields.insert("source".to_owned(), json!(record.source));
                    fields.insert("learned_at".to_owned(), json!(record.learned_at));
                }
                fields
            }
            EntityRef::Person(person_id) => {
                if *person_id != actor_state.id {
                    return Ok(no_knowledge_inspection(entity, detail));
                }
                let Some(person) = world.person(*person_id) else {
                    return Ok(missing_inspection(entity, detail));
                };
                BTreeMap::from([
                    ("name".to_owned(), json!(person.name)),
                    ("roles".to_owned(), json!(person.roles)),
                    ("government".to_owned(), json!(person.government)),
                    (
                        "current_location".to_owned(),
                        json!(person.current_location),
                    ),
                ])
            }
            EntityRef::Territory(_)
            | EntityRef::Domain(_)
            | EntityRef::Government(_)
            | EntityRef::Route(_)
            | EntityRef::Organization(_)
            | EntityRef::Resource(_) => return Ok(no_knowledge_inspection(entity, detail)),
        };
        Ok(Inspection {
            entity: entity.clone(),
            detail,
            summary: format!("Actor-relative inspection of {entity}"),
            fields,
        })
    }

    pub fn available_actions(&self, actor: PersonId) -> Result<Vec<AvailableAction>, CanwuError> {
        let world = self.world();
        if world.person(actor).is_none() {
            return Err(CanwuError::new(
                ErrorCode::ActorNotFound,
                format!("actor {actor} was not found"),
            ));
        }
        let mut actions = Vec::new();
        for army in world.armies.iter().filter(|army| army.commander == actor) {
            if army.transit.is_some() {
                continue;
            }
            for route in &world.routes {
                if let Some(destination) = route.other_end(army.location) {
                    actions.push(AvailableAction {
                        action_type: "move_army".to_owned(),
                        description: format!("Move {} to territory {destination}", army.name),
                        payload: json!({
                            "army": army.id,
                            "destination": destination,
                        }),
                        legal_reason: format!("Actor {actor} commands army {}", army.id),
                    });
                }
            }
        }
        Ok(actions)
    }

    pub fn act(
        &mut self,
        actor: PersonId,
        action: SemanticAction,
    ) -> Result<CommandReceipt, CanwuError> {
        let command = match action {
            SemanticAction::MoveArmy { army, destination } => {
                Command::MoveArmy { army, destination }
            }
            SemanticAction::Plugin {
                plugin,
                action,
                payload,
            } => Command::Plugin {
                plugin,
                command: action,
                payload,
            },
        };
        self.submit(CommandEnvelope::new(Issuer::Actor(actor), command))
    }

    #[must_use]
    pub fn explain(&self, request: &ExplanationRequest) -> Explanation {
        match request {
            ExplanationRequest::Event(event_id) => self.explain_event(*event_id),
            ExplanationRequest::ArmyMorale(army_id) => self.explain_army_morale(*army_id),
            ExplanationRequest::Failure(error) => Explanation {
                summary: error.message.clone(),
                causal_chain: vec![ExplanationStep {
                    label: format!("Validation failed: {:?}", error.code),
                    event: None,
                }],
            },
        }
    }

    #[must_use]
    pub fn describe_capabilities(&self) -> CapabilityDescription {
        CapabilityDescription {
            operations: vec![
                "observe",
                "inspect",
                "query",
                "available_actions",
                "act",
                "explain",
                "wait",
                "describe_capabilities",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            notes: vec![
                "Agent reads are actor-relative and never fall back to ground truth".to_owned(),
                "All actions become validated commands".to_owned(),
                "Use progressive inspection detail to control response size".to_owned(),
            ],
            plugin_actions: self
                .plugin_descriptors()
                .flat_map(|plugin| {
                    plugin
                        .commands
                        .iter()
                        .map(move |action| format!("{}.{}", plugin.name, action.name))
                })
                .collect(),
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

    fn explain_army_morale(&self, army_id: ArmyId) -> Explanation {
        let world = self.world();
        let Some(army) = world.army(army_id) else {
            return Explanation {
                summary: format!("Army {army_id} was not found"),
                causal_chain: Vec::new(),
            };
        };
        let provenance = self.events().iter().rev().find(|event| {
            matches!(
                &event.kind,
                EventKind::DebugFieldChanged { entity: EntityRef::Army(id), field, .. }
                    if *id == army_id && field == "morale"
            )
        });
        provenance.map_or_else(
            || Explanation {
                summary: format!(
                    "{} morale is {}; no post-scenario morale-changing event is recorded",
                    army.name, army.morale
                ),
                causal_chain: Vec::new(),
            },
            |event| self.explain_event(event.id),
        )
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

    pub fn seal_evidence(&mut self) -> Result<Option<EvidenceJournalSegment>, CanwuError> {
        self.simulation.seal_evidence()
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryEntity {
    Person,
    Government,
    Territory,
    Route,
    Army,
    Event,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterOperator {
    Equal,
    Contains,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct QueryFilter {
    pub field: String,
    pub operator: FilterOperator,
    pub value: Value,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Query {
    pub entity: QueryEntity,
    pub filters: Vec<QueryFilter>,
    pub select: Vec<String>,
    pub limit: usize,
}

impl Query {
    #[must_use]
    pub const fn all(entity: QueryEntity) -> Self {
        Self {
            entity,
            filters: Vec::new(),
            select: Vec::new(),
            limit: 100,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct QueryResult {
    pub rows: Vec<BTreeMap<String, Value>>,
    pub truncated: bool,
}

fn run_query(world: &WorldSnapshot, events: &[SimEvent], query: &Query) -> QueryResult {
    let rows: Vec<_> = match query.entity {
        QueryEntity::Person => world
            .people
            .iter()
            .map(|person| value_to_row(&json!(person)))
            .collect(),
        QueryEntity::Government => world
            .governments
            .iter()
            .map(|government| value_to_row(&json!(government)))
            .collect(),
        QueryEntity::Territory => world
            .territories
            .iter()
            .map(|territory| value_to_row(&json!(territory)))
            .collect(),
        QueryEntity::Route => world
            .routes
            .iter()
            .map(|route| value_to_row(&json!(route)))
            .collect(),
        QueryEntity::Army => world
            .armies
            .iter()
            .map(|army| value_to_row(&json!(army)))
            .collect(),
        QueryEntity::Event => events
            .iter()
            .map(|event| value_to_row(&json!(event)))
            .collect(),
    };
    finalize_query(rows, query)
}

fn run_actor_query(
    world: &WorldSnapshot,
    actor: PersonId,
    knowledge: Option<&ActorKnowledge>,
    query: &Query,
) -> QueryResult {
    match query.entity {
        QueryEntity::Army => {
            let rows = knowledge.map_or_else(Vec::new, |knowledge| {
                knowledge
                    .armies
                    .values()
                    .map(|record| value_to_row(&json!(record)))
                    .collect()
            });
            finalize_query(rows, query)
        }
        QueryEntity::Person => {
            let rows = world
                .person(actor)
                .map_or_else(Vec::new, |person| vec![value_to_row(&json!(person))]);
            finalize_query(rows, query)
        }
        QueryEntity::Event => QueryResult::default(),
        QueryEntity::Government | QueryEntity::Territory | QueryEntity::Route => {
            QueryResult::default()
        }
    }
}

fn finalize_query(rows: Vec<BTreeMap<String, Value>>, query: &Query) -> QueryResult {
    let filtered: Vec<_> = rows
        .into_iter()
        .filter(|row| {
            query
                .filters
                .iter()
                .all(|filter| matches_filter(row, filter))
        })
        .collect();
    let truncated = filtered.len() > query.limit;
    let rows = filtered
        .into_iter()
        .take(query.limit)
        .map(|row| select_fields(row, &query.select))
        .collect();
    QueryResult { rows, truncated }
}

fn matches_filter(row: &BTreeMap<String, Value>, filter: &QueryFilter) -> bool {
    let Some(actual) = row.get(&filter.field) else {
        return false;
    };
    match filter.operator {
        FilterOperator::Equal => actual == &filter.value,
        FilterOperator::Contains => value_text(actual)
            .to_lowercase()
            .contains(&value_text(&filter.value).to_lowercase()),
    }
}

fn value_text(value: &Value) -> String {
    value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_owned)
}

fn select_fields(mut row: BTreeMap<String, Value>, select: &[String]) -> BTreeMap<String, Value> {
    if select.is_empty() {
        return row;
    }
    row.retain(|field, _| select.contains(field));
    row
}

fn value_to_row(value: &Value) -> BTreeMap<String, Value> {
    value.as_object().map_or_else(BTreeMap::new, |object| {
        object
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()
    })
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationFocus {
    CurrentSituation,
    Military,
    Changes,
}

/// An observation identity authorized by the run's persisted observation
/// policy. This type is intentionally constructed through
/// [`Canwu::viewer_context`] so an observation request cannot self-escalate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ViewerContext {
    actor: PersonId,
    observation: ObservationPolicy,
}

impl ViewerContext {
    #[must_use]
    pub const fn actor(self) -> PersonId {
        self.actor
    }

    #[must_use]
    pub const fn observation(self) -> ObservationPolicy {
        self.observation
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ObserveRequest {
    pub focus: ObservationFocus,
    pub since: Option<SimTime>,
}

impl Default for ObserveRequest {
    fn default() -> Self {
        Self {
            focus: ObservationFocus::CurrentSituation,
            since: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentIdentity {
    pub person: PersonId,
    pub name: String,
    pub roles: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KnownArmyView {
    pub army: ArmyId,
    pub name: String,
    pub known_location: Option<TerritoryId>,
    pub estimated_strength: EstimateRange,
    pub information_age_minutes: i64,
    pub confidence_per_mille: u16,
    pub source: KnowledgeSource,
}

fn known_army_view(now: SimTime, record: &ArmyKnowledge) -> Result<KnownArmyView, CanwuError> {
    let information_age = now.checked_sub(record.observed_at).ok_or_else(|| {
        CanwuError::new(
            ErrorCode::InvalidDuration,
            "knowledge age exceeds the supported simulation-duration range",
        )
    })?;
    Ok(KnownArmyView {
        army: record.army,
        name: record
            .known_name
            .clone()
            .unwrap_or_else(|| format!("Army {}", record.army)),
        known_location: record.known_location,
        estimated_strength: record.estimated_strength,
        information_age_minutes: information_age.as_minutes(),
        confidence_per_mille: record.confidence_per_mille,
        source: record.source.clone(),
    })
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
    let actor = viewer.actor;
    let visible = match &event.kind {
        EventKind::MoveOrdered { .. } => {
            event.affected_entities.contains(&EntityRef::Person(actor))
        }
        EventKind::KnowledgeUpdated { recipient, .. } => *recipient == actor,
        EventKind::ArmyArrived { .. }
        | EventKind::ReportDispatched { .. }
        | EventKind::DebugFieldChanged { .. } => matches!(
            viewer.observation,
            ObservationPolicy::ResearchFull | ObservationPolicy::DeveloperDiagnostic
        ),
        EventKind::Plugin { .. } => event_visible_to(viewer, event, plugin_audience),
    };
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
        EventAudience::Actor(actor) => *actor == viewer.actor,
        EventAudience::Actors(actors) => actors.binary_search(&viewer.actor).is_ok(),
        EventAudience::AffectedActors => event
            .affected_entities
            .contains(&EntityRef::Person(viewer.actor)),
        EventAudience::Private => false,
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PendingCommitment {
    pub summary: String,
    pub due_at: SimTime,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AvailableAction {
    pub action_type: String,
    pub description: String,
    pub payload: Value,
    pub legal_reason: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AgentContext {
    pub identity: AgentIdentity,
    pub current_time: SimTime,
    pub current_location: TerritoryId,
    pub focus: ObservationFocus,
    pub known_armies: Vec<KnownArmyView>,
    pub changes_since: Vec<VisibleChange>,
    pub pending_actions: Vec<PendingCommitment>,
    pub available_actions: Vec<AvailableAction>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DetailLevel {
    Summary,
    Domain,
    Entity,
    RawFields,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Inspection {
    pub entity: EntityRef,
    pub detail: DetailLevel,
    pub summary: String,
    pub fields: BTreeMap<String, Value>,
}

fn missing_inspection(entity: &EntityRef, detail: DetailLevel) -> Inspection {
    Inspection {
        entity: entity.clone(),
        detail,
        summary: format!("{entity} was not found"),
        fields: BTreeMap::new(),
    }
}

fn no_knowledge_inspection(entity: &EntityRef, detail: DetailLevel) -> Inspection {
    Inspection {
        entity: entity.clone(),
        detail,
        summary: "No actor-scoped knowledge is available for this entity".to_owned(),
        fields: BTreeMap::new(),
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SemanticAction {
    MoveArmy {
        army: ArmyId,
        destination: TerritoryId,
    },
    Plugin {
        plugin: String,
        action: String,
        payload: Value,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum ExplanationRequest {
    Event(EventId),
    ArmyMorale(ArmyId),
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CapabilityDescription {
    pub operations: Vec<String>,
    pub notes: Vec<String>,
    pub plugin_actions: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    struct VisibilityPlugin {
        audience: EventAudience,
    }

    #[allow(clippy::unnecessary_wraps)]
    fn visibility_system(
        _view: &SimulationView<'_>,
        event: &SimEvent,
    ) -> Result<Vec<SystemDirective>, CanwuError> {
        if !matches!(event.kind, EventKind::MoveOrdered { .. }) {
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
            kind: EventKind::Plugin {
                plugin: "visibility-test".to_owned(),
                event_type: "notice".to_owned(),
            },
            affected_entities: vec![EntityRef::Person(ids.commander)],
            summary: "notice".to_owned(),
            cause: None,
            correlation_id: 1,
        };
        let actor = ViewerContext {
            actor: ids.commander,
            observation: ObservationPolicy::ActorBound,
        };
        let observer = ViewerContext {
            actor: ids.observer,
            observation: ObservationPolicy::ActorBound,
        };
        let public_observer = ViewerContext {
            actor: ids.observer,
            observation: ObservationPolicy::PublicObserver,
        };
        let research = ViewerContext {
            actor: ids.observer,
            observation: ObservationPolicy::ResearchFull,
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
                SemanticAction::MoveArmy {
                    army: ids.army,
                    destination: ids.eastern_territory,
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
        let ids = Canwu::demo_ids();
        let canwu = Canwu::demo(35).expect("demo should load");
        let escalated = ViewerContext {
            actor: ids.observer,
            observation: ObservationPolicy::ResearchFull,
        };

        let error = canwu
            .observe_with_viewer(&escalated, &ObserveRequest::default())
            .expect_err("a caller cannot self-escalate the observation policy");
        assert_eq!(error.code, ErrorCode::InvalidAuthority);
    }

    #[test]
    fn actor_relative_observation_does_not_leak_arrival() {
        let mut canwu = Canwu::demo(35).expect("demo should load");
        let ids = Canwu::demo_ids();
        canwu
            .act(
                ids.commander,
                SemanticAction::MoveArmy {
                    army: ids.army,
                    destination: ids.eastern_territory,
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
                Command::MoveArmy {
                    army: ids.army,
                    destination: ids.eastern_territory,
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
            .expect("checkpoint journal should restore through the public facade");
        assert_eq!(restored.snapshot(), canwu.snapshot());

        canwu
            .settle_boundary(BoundaryRequest::at(canwu.time()))
            .expect("a public boundary should complete the live evidence tail");
        let expected = canwu.snapshot();
        let mut compact = canwu
            .into_compacted()
            .expect("the public facade should enter compact mode");
        let segment = compact
            .seal_evidence()
            .expect("the public compact facade should seal evidence")
            .expect("the public compact facade should return a segment");
        let compact_checkpoint = compact
            .checkpoint()
            .expect("the public compact facade should checkpoint");
        assert_eq!(
            compact
                .snapshot_with_segments(vec![segment.clone()])
                .expect("the public compact facade should reconstruct its snapshot"),
            expected
        );
        let restored_compact =
            CompactedCanwu::from_checkpoint_and_journal(compact_checkpoint, vec![segment])
                .expect("the public compact facade should restore from its archive");
        assert_eq!(
            restored_compact
                .snapshot_with_segments(Vec::new())
                .expect("the restored compact facade should retain validated evidence"),
            expected
        );
    }
}
