//! Inspectable, serializable events with compact causal provenance.

use canwu_core::{ArmyId, CommandId, EntityRef, EventId, PersonId, TerritoryId};
use canwu_time::SimTime;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "id", rename_all = "snake_case")]
pub enum CauseRef {
    Command(CommandId),
    Event(EventId),
    System(String),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventKind {
    MoveOrdered {
        army: ArmyId,
        from: TerritoryId,
        to: TerritoryId,
        arrival_at: SimTime,
    },
    ArmyArrived {
        army: ArmyId,
        territory: TerritoryId,
    },
    ReportDispatched {
        recipient: PersonId,
        army: ArmyId,
        arrives_at: SimTime,
    },
    KnowledgeUpdated {
        recipient: PersonId,
        army: ArmyId,
        known_location: TerritoryId,
    },
    DebugFieldChanged {
        entity: EntityRef,
        field: String,
        old_value: String,
        new_value: String,
    },
    Plugin {
        plugin: String,
        event_type: String,
    },
}

impl EventKind {
    #[must_use]
    pub const fn event_type(&self) -> &'static str {
        match self {
            Self::MoveOrdered { .. } => "move_ordered",
            Self::ArmyArrived { .. } => "army_arrived",
            Self::ReportDispatched { .. } => "report_dispatched",
            Self::KnowledgeUpdated { .. } => "knowledge_updated",
            Self::DebugFieldChanged { .. } => "debug_field_changed",
            Self::Plugin { .. } => "plugin",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SimEvent {
    pub id: EventId,
    pub timestamp: SimTime,
    pub kind: EventKind,
    pub affected_entities: Vec<EntityRef>,
    pub summary: String,
    pub cause: Option<CauseRef>,
    pub correlation_id: u64,
}
