use super::{
    BoundaryPersonAvailabilityChange, BoundaryPersonCreation, CanwuError, CreatedPerson,
    DecisionOptionWeight, DomainRecordChange, DomainRecordMutation, PersonAvailability,
    PersonDraft, PolicyDecision, RandomSample, RandomStreamKey, SimulationView, StateKey,
    StateVisibility, SystemCadence,
};
use canwu_core::{
    BoundaryId, CommandAttemptId, CommandId, CommandRequestId, DecisionRequestId, DecisionTicketId,
    EntityRef, EvaluationTraceRecord, EventId, IngressId, KnowledgeHolderRef, KnowledgeSchemaId,
    PersonId, RandomDrawId,
};
use canwu_knowledge::{KnowledgeRecord, KnowledgeRecordDraft};
use canwu_time::{SimDuration, SimTime};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ReservationPoolKey {
    pub state: StateKey,
    pub entity: EntityRef,
    pub resource: String,
}

impl ReservationPoolKey {
    #[must_use]
    pub fn new(state: StateKey, entity: EntityRef, resource: impl Into<String>) -> Self {
        Self {
            state,
            entity,
            resource: resource.into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ReservationRef {
    pub plugin: String,
    pub system: String,
    pub request: String,
}

impl ReservationRef {
    #[must_use]
    pub fn new(
        plugin: impl Into<String>,
        system: impl Into<String>,
        request: impl Into<String>,
    ) -> Self {
        Self {
            plugin: plugin.into(),
            system: system.into(),
            request: request.into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReservationOffer {
    pub pool: ReservationPoolKey,
    pub capacity: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReservationRequest {
    pub request: String,
    pub pool: ReservationPoolKey,
    pub quantity: u64,
    pub priority: i32,
    pub tie_break: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReservationOfferRecord {
    pub plugin: String,
    pub system: String,
    pub offer: ReservationOffer,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReservationRequestRecord {
    pub reservation: ReservationRef,
    pub request: ReservationRequest,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReservationDisposition {
    Fulfilled,
    Partial,
    Rejected,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReservationAllocation {
    pub reservation: ReservationRef,
    pub pool: ReservationPoolKey,
    pub requested: u64,
    pub granted: u64,
    pub remaining_after: u64,
    pub disposition: ReservationDisposition,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BoundaryDirective {
    SetComponent {
        state: StateKey,
        entity: EntityRef,
        component: String,
        value: Value,
        summary: String,
    },
    MutateRecord {
        mutation: DomainRecordMutation,
        summary: String,
    },
    Emit {
        event_type: String,
        summary: String,
        affected: Vec<EntityRef>,
    },
    ScheduleIngress {
        after: SimDuration,
        packet_type: String,
        priority: i32,
        payload: Value,
        affected: Vec<EntityRef>,
    },
    SchedulePluginIngress {
        target_plugin: String,
        after: SimDuration,
        packet_type: String,
        priority: i32,
        payload: Value,
        affected: Vec<EntityRef>,
    },
    /// Resolves an open ticket with an operation-keyed random draw bound to
    /// the ticket and its version, either for a controller with random policy
    /// identity or as a guarded utility policy's random tie-break. The
    /// boundary generates the `Resolve` decision ingress, admitted at the next
    /// boundary.
    ///
    /// Before any draw is committed, the directive fails the boundary when
    /// the ticket's person decision maker
    /// ([`crate::ErrorCode::DecisionMakerUnavailable`]) or its assigned
    /// controller's authority person ([`crate::ErrorCode::IssuerUnavailable`])
    /// is unavailable in the availability committed before this boundary. The
    /// authority person is the actor of an actor authority or the responsible
    /// actor of an institution authority. Because the end-of-boundary sweep
    /// and `Open` admission keep such tickets from staying open, this is a
    /// safeguard. Any availability change made in the same boundary does not
    /// fail the directive, because failing would roll the change back and
    /// repeat on every retry; the draw is then committed, the end-of-boundary
    /// sweep cancels the ticket (see
    /// [`BoundaryDirective::SetPersonAvailability`]), and the generated
    /// resolution is rejected at admission. To avoid that wasted draw,
    /// tie-break and random-policy systems should skip tickets whose decision
    /// maker or controller authority person is unavailable, read through
    /// [`crate::SimulationView::person_availability`] (which requires
    /// declaring `StateKey::core_person_availability()` in the contract's
    /// reads).
    ResolveDecisionRandomly {
        resolution: RandomDecisionResolution,
    },
    PublishKnowledge {
        holder: KnowledgeHolderRef,
        visibility: StateVisibility,
        producer_correlation: Option<String>,
        records: Vec<KnowledgeRecordDraft>,
        summary: String,
    },
    /// Replaces one person's core life and custody state. Accepted from a
    /// phase-7 or phase-10 system that declares
    /// `StateKey::core_person_availability()` as a write; two writes for the
    /// same person in one boundary fail the boundary.
    ///
    /// Making a person unavailable cancels, at the end of the same boundary
    /// and after the boundary's random decisions are materialized, every open
    /// decision ticket whose decision maker is that person
    /// ([`crate::DECISION_MAKER_UNAVAILABLE_REASON`]) and then every remaining
    /// open ticket whose assigned controller's authority person is that
    /// person ([`crate::CONTROLLER_AUTHORITY_UNAVAILABLE_REASON`]). The
    /// authority person is the actor of an actor authority or the responsible
    /// actor of an institution authority; council and no-responsible-actor
    /// authorities are never affected. A ticket that qualifies for both
    /// reasons carries the decision-maker reason. The cancelled IDs are
    /// recorded in ticket-ID order on the boundary's
    /// [`crate::BoundaryPersonAvailabilityChange`].
    SetPersonAvailability {
        person: PersonId,
        availability: PersonAvailability,
        summary: String,
    },
    /// Creates a person with an engine-allocated ID. Accepted from a phase-7
    /// system that declares `StateKey::core_people()` as a write. The person
    /// is committed at the end of the boundary and becomes visible to systems
    /// at the next boundary; `correlation` is unique per plugin, system, and
    /// boundary and binds the receipt's allocated ID.
    CreatePerson {
        draft: PersonDraft,
        correlation: String,
        summary: String,
    },
    /// Withdraws one still-pending plugin ingress item that this system's
    /// plugin scheduled inside the engine (through `ScheduleIngress`,
    /// `SchedulePluginIngress`, or a plugin command), strictly before the
    /// item's due time. The boundary records a terminal
    /// [`crate::IngressPayload::PluginCancellation`] entry among its generated
    /// ingress; the withdrawn item is never admitted.
    ///
    /// Take targets from [`crate::SimulationView::cancellable_plugin_ingress`]
    /// in the same boundary. A target that is foreign, already due, admitted,
    /// or cancelled, or that another proposal already cancels in this
    /// boundary, fails the whole boundary deterministically.
    CancelPluginIngress {
        ingress_id: IngressId,
        reason: String,
    },
    /// Records how an application rule produced one result for one subject.
    /// Accepted from any phase-7 or phase-12 system without a contract
    /// declaration. The trace must name this boundary, its subject identity
    /// must exist, and every term's evidence must be committed evidence
    /// visible to the proposal.
    ///
    /// The trace is recorded, with its producing system, in
    /// [`crate::BoundaryRecord::evaluation_traces`] as hash-chained boundary
    /// evidence. It is never state: it changes nothing, no system can read it
    /// back, and it is sealed and archived with its boundary record. The run
    /// configuration's [`crate::EvaluationLimitsV1`] bound the traces of the
    /// whole boundary and the terms of each trace; a proposal set that
    /// exceeds either bound fails the boundary with
    /// [`crate::ErrorCode::EvaluationTraceLimitExceeded`].
    RecordEvaluationTrace { trace: EvaluationTraceRecord },
    /// Registers a transition manifest coordinated by this system's plugin.
    /// Accepted from a phase-7, phase-10, or phase-12 system that declares
    /// `StateKey::core_transitions()` as a write.
    ///
    /// A manifest registered in phase 7 may be ready in the same boundary;
    /// one registered in phase 10 or 12 must name a later boundary, and none
    /// may be ready more than [`crate::MAX_TRANSITION_READY_HORIZON`]
    /// boundaries ahead. Every participant must be a registered plugin with a
    /// phase-10 system that declares the same write, and every expected record
    /// must be of a registered kind. One coordinator may have one pending
    /// manifest per lineage and at most
    /// [`crate::MAX_PENDING_TRANSITION_MANIFESTS_PER_COORDINATOR`] pending
    /// manifests, within [`crate::MAX_PENDING_TRANSITION_MANIFESTS`] overall.
    /// The registration becomes visible to the coordinator and participants
    /// from the next phase through
    /// [`crate::SimulationView::transition_manifests`] and is recorded in
    /// [`BoundaryRecord::transition_manifests`].
    RegisterTransitionManifest { manifest: crate::TransitionManifest },
    /// Stages ordinary directives as this plugin's part of a transition
    /// manifest that is ready at this boundary. Accepted only from a phase-10
    /// system of a listed participant that declares
    /// `StateKey::core_transitions()` as a write; another plugin's staging
    /// fails the boundary with [`crate::ErrorCode::InvalidAuthority`].
    ///
    /// Each staged write must be a directive this system could propose
    /// directly: its declared writes, ownership, and phase rules apply, and it
    /// commits with the system's visibility. An empty `writes` list records
    /// the participant's presence without writing. Before phase 11 commits,
    /// the kernel audits every ready manifest: when some, but not all,
    /// participants staged ([`crate::ErrorCode::TransitionParticipantMissing`])
    /// or an expected version differs
    /// ([`crate::ErrorCode::TransitionVersionMismatch`]), the whole boundary
    /// fails closed; when none staged, the manifest expires, so a
    /// single-participant manifest cannot fail for omission. The audit is
    /// recorded in [`BoundaryRecord::transition_audits`] and readable through
    /// [`crate::SimulationView::transition_audits`].
    StageTransitionWrite {
        manifest_id: crate::TransitionManifestId,
        writes: Vec<BoundaryDirective>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RandomDecisionResolution {
    pub priority: i32,
    pub decision_request_id: DecisionRequestId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_request_id: Option<CommandRequestId>,
    pub ticket_id: DecisionTicketId,
    pub expected_version: u64,
    pub controller_id: String,
    pub sample: RandomSample,
    pub option_weights: Vec<DecisionOptionWeight>,
    /// The pending decision of a utility-policy controller whose
    /// `PendingRandom` candidates this draw resolves. `None` for a
    /// random-policy controller, whose weights cover every available option.
    /// When present, `option_weights` must equal the pending candidates, and
    /// the generated resolution keeps the pending evaluations, fired guards,
    /// and random stage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tie_break: Option<Box<PolicyDecision>>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct BoundaryProposal {
    pub offers: Vec<ReservationOffer>,
    pub requests: Vec<ReservationRequest>,
    pub directives: Vec<BoundaryDirective>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KnowledgeWriteGrant {
    pub schema: KnowledgeSchemaId,
    pub visibilities: Vec<StateVisibility>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct PluginIngressTarget {
    pub target_plugin: String,
    pub packet_type: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BoundarySystemContract {
    pub name: String,
    pub phase: crate::BoundaryPhase,
    pub cadence: SystemCadence,
    pub reads: Vec<StateKey>,
    pub writes: Vec<StateKey>,
    pub emits: Vec<String>,
    pub reservation_offers: Vec<StateKey>,
    pub reservation_requests: Vec<StateKey>,
    pub reservation_reads: Vec<ReservationRef>,
    #[serde(default)]
    pub random_streams: Vec<RandomStreamKey>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub knowledge_writes: Vec<KnowledgeWriteGrant>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plugin_ingress_targets: Vec<PluginIngressTarget>,
    pub visibility: StateVisibility,
}

impl BoundarySystemContract {
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        phase: crate::BoundaryPhase,
        cadence: SystemCadence,
    ) -> Self {
        Self {
            name: name.into(),
            phase,
            cadence,
            reads: Vec::new(),
            writes: Vec::new(),
            emits: Vec::new(),
            reservation_offers: Vec::new(),
            reservation_requests: Vec::new(),
            reservation_reads: Vec::new(),
            random_streams: Vec::new(),
            knowledge_writes: Vec::new(),
            plugin_ingress_targets: Vec::new(),
            visibility: StateVisibility::NextBoundary,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BoundaryContext {
    pub boundary_id: BoundaryId,
    pub at: SimTime,
    pub phase: crate::BoundaryPhase,
    pub plugin: String,
    pub system: String,
    pub admitted_attempts: Vec<CommandAttemptId>,
    pub admitted_commands: Vec<CommandId>,
    pub admitted_ingress: Vec<IngressId>,
    pub admitted_events: Vec<EventId>,
    pub emitted_events: Vec<EventId>,
}

pub type BoundarySystemHandler =
    fn(&SimulationView<'_>, &BoundaryContext) -> Result<BoundaryProposal, CanwuError>;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BoundaryRequest {
    pub at: SimTime,
    pub cadences: Vec<SystemCadence>,
}

impl BoundaryRequest {
    #[must_use]
    pub const fn at(at: SimTime) -> Self {
        Self {
            at,
            cadences: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_cadence(mut self, cadence: SystemCadence) -> Self {
        self.cadences.push(cadence);
        self
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct BoundaryChange {
    pub plugin: String,
    pub system: String,
    pub state: StateKey,
    pub entity: EntityRef,
    pub component: String,
    pub previous: Option<Value>,
    pub value: Value,
    pub visibility: StateVisibility,
    pub summary: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BoundaryEmissionKind {
    Change { change_index: u64 },
    RecordChange { change_index: u64 },
    KnowledgeChange { change_index: u64 },
    Explicit,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BoundaryEmission {
    pub plugin: String,
    pub system: String,
    pub event: EventId,
    pub kind: BoundaryEmissionKind,
}

/// Durable, idempotent external-delivery identity derived from committed
/// boundary evidence. The engine creates one entry for every emission; a host
/// may deliver it at least once and use `delivery_id` as its idempotency key.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OutboxEntry {
    pub delivery_id: String,
    pub boundary: BoundaryId,
    pub event: EventId,
    pub emission_index: u64,
    pub plugin: String,
    pub system: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BoundaryIngressGeneration {
    pub ingress: IngressId,
    pub plugin: String,
    pub system: String,
    pub phase: crate::BoundaryPhase,
    pub visibility: StateVisibility,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct BoundaryKnowledgeChange {
    pub plugin: String,
    pub system: String,
    pub phase: crate::BoundaryPhase,
    pub holder: KnowledgeHolderRef,
    pub producer_correlation: Option<String>,
    pub records: Vec<KnowledgeRecord>,
    pub visibility: StateVisibility,
    pub summary: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct BoundaryRecord {
    pub id: BoundaryId,
    pub at: SimTime,
    pub correlation_id: u64,
    pub cadences: Vec<SystemCadence>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub admitted_attempts: Vec<CommandAttemptId>,
    pub admitted_commands: Vec<CommandId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub admitted_ingress: Vec<IngressId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub generated_ingress: Vec<BoundaryIngressGeneration>,
    pub admitted_events: Vec<EventId>,
    pub reservation_offers: Vec<ReservationOfferRecord>,
    pub reservation_requests: Vec<ReservationRequestRecord>,
    pub allocations: Vec<ReservationAllocation>,
    #[serde(default)]
    pub random_draws: Vec<RandomDrawId>,
    pub changes: Vec<BoundaryChange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub record_changes: Vec<DomainRecordChange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub knowledge_changes: Vec<BoundaryKnowledgeChange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub maintenance_changes: Vec<crate::MaintenanceChangeRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maintenance_terminal_root: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub person_availability_changes: Vec<BoundaryPersonAvailabilityChange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub created_persons: Vec<BoundaryPersonCreation>,
    /// Rule-evaluation traces recorded by this boundary's systems, in system
    /// execution order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evaluation_traces: Vec<crate::BoundaryEvaluationTrace>,
    /// Transition manifests registered in this boundary, in admission order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transition_manifests: Vec<crate::PendingTransitionManifest>,
    /// Manifests settled at this, their ready boundary, in manifest-ID order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transition_audits: Vec<crate::TransitionAuditRecord>,
    pub emissions: Vec<BoundaryEmission>,
    #[serde(default)]
    /// Untagged legacy full-state hash or a `v1:` incremental state commitment.
    pub state_hash: Option<String>,
    #[serde(default)]
    pub previous_hash: String,
    #[serde(default)]
    pub hash: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BoundaryReceipt {
    pub boundary_id: BoundaryId,
    pub settled_at: SimTime,
    pub emitted_events: Vec<EventId>,
    pub generated_ingress: Vec<IngressId>,
    pub random_draws: Vec<RandomDrawId>,
    pub boundary_hash: String,
    pub change_count: usize,
    pub record_change_count: usize,
    pub knowledge_batch_count: usize,
    pub knowledge_record_count: usize,
    pub allocations: Vec<ReservationAllocation>,
    /// Engine-allocated IDs of persons created in this boundary, keyed by
    /// producing plugin, system, and correlation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub created_persons: Vec<CreatedPerson>,
    /// Transition manifests settled at this boundary, in manifest-ID order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transition_audits: Vec<crate::TransitionAuditRecord>,
}
