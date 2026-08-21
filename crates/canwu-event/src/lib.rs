//! Inspectable, serializable events with compact causal provenance.

use canwu_core::{
    ArmyId, BoundaryId, CommandId, EntityRef, EventId, KnowledgeHolderRef, PersonId, TerritoryId,
};
use canwu_time::SimTime;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "id", rename_all = "snake_case")]
pub enum CauseRef {
    Boundary(BoundaryId),
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
    KnowledgePublished {
        holder: KnowledgeHolderRef,
        record_count: u32,
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

/// Declarative audience for a persisted event projection.
///
/// This is intentionally separate from plugin-to-plugin dispatch permissions:
/// it controls only whether a trusted viewer may receive the event through a
/// player-facing observation projection. `Private` is the safe default for
/// plugin events that do not declare an audience.
#[derive(Clone, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum EventAudience {
    Public,
    Actor(PersonId),
    Actors(Vec<PersonId>),
    KnowledgeHolder(KnowledgeHolderRef),
    /// The event is visible to actors represented by `EntityRef::Person` in
    /// the event's affected-entity list.
    AffectedActors,
    #[default]
    Private,
}

impl EventKind {
    #[must_use]
    pub const fn event_type(&self) -> &'static str {
        match self {
            Self::MoveOrdered { .. } => "move_ordered",
            Self::ArmyArrived { .. } => "army_arrived",
            Self::ReportDispatched { .. } => "report_dispatched",
            Self::KnowledgeUpdated { .. } => "knowledge_updated",
            Self::KnowledgePublished { .. } => "knowledge_published",
            Self::DebugFieldChanged { .. } => "debug_field_changed",
            Self::Plugin { .. } => "plugin",
        }
    }

    /// Returns the stable, display-oriented type identity for this event.
    ///
    /// Built-in event kinds retain the same snake-case labels returned by
    /// [`Self::event_type`]. Plugin events are qualified with their registered
    /// plugin name and event type, separated by a dot. The two plugin-provided
    /// components are preserved as registered; no case folding or additional
    /// normalization is applied.
    #[must_use]
    pub fn qualified_event_type(&self) -> String {
        match self {
            Self::Plugin { plugin, event_type } => format!("{plugin}.{event_type}"),
            _ => self.event_type().to_owned(),
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

#[cfg(test)]
mod tests {
    use super::EventKind;
    use canwu_core::{ArmyId, TerritoryId};

    #[test]
    fn qualified_event_type_preserves_legacy_labels_and_disambiguates_plugins() {
        let built_in = EventKind::ArmyArrived {
            army: ArmyId::new(1),
            territory: TerritoryId::new(2),
        };
        assert_eq!(built_in.event_type(), "army_arrived");
        assert_eq!(built_in.qualified_event_type(), "army_arrived");

        let supply = EventKind::Plugin {
            plugin: "example-supply".to_owned(),
            event_type: "grain_allocated".to_owned(),
        };
        let demand = EventKind::Plugin {
            plugin: "example-demand".to_owned(),
            event_type: "grain_allocated".to_owned(),
        };

        assert_eq!(supply.event_type(), "plugin");
        assert_eq!(
            supply.qualified_event_type(),
            "example-supply.grain_allocated"
        );
        assert_eq!(
            demand.qualified_event_type(),
            "example-demand.grain_allocated"
        );
        assert_ne!(supply.qualified_event_type(), demand.qualified_event_type());
    }
}
