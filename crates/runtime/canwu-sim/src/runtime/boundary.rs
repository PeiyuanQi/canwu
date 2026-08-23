use super::{
    CanwuError, DomainRecordChange, DomainRecordMutation, RandomStreamKey, SimulationView,
    StateKey, StateVisibility, SystemCadence,
};
use canwu_core::{
    BoundaryId, CommandAttemptId, CommandId, EntityRef, EventId, IngressId, KnowledgeHolderRef,
    KnowledgeSchemaId, RandomDrawId,
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
    PublishKnowledge {
        holder: KnowledgeHolderRef,
        visibility: StateVisibility,
        producer_correlation: Option<String>,
        records: Vec<KnowledgeRecordDraft>,
        summary: String,
    },
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
}
