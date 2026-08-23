//! Typed payloads for the built-in compatibility slice.
//!
//! These types stay private to the runtime. The public event crate owns only
//! the generic envelope and flattened wire representation.

use super::{
    ArmyId, EntityRef, EventKind, KnowledgeHolderRef, LetterId, PersonId, SimTime, TerritoryId,
};
use canwu_event::EventKindError;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

pub(super) const MOVE_ORDERED: &str = "move_ordered";
pub(super) const ARMY_ARRIVED: &str = "army_arrived";
pub(super) const PERSON_MOVE_ORDERED: &str = "person_move_ordered";
pub(super) const PERSON_ARRIVED: &str = "person_arrived";
pub(super) const LETTER_DELIVERED: &str = "letter_delivered";
pub(super) const REPORT_DISPATCHED: &str = "report_dispatched";
pub(super) const KNOWLEDGE_UPDATED: &str = "knowledge_updated";
pub(super) const KNOWLEDGE_PUBLISHED: &str = "knowledge_published";
pub(super) const DEBUG_FIELD_CHANGED: &str = "debug_field_changed";
pub(super) const PLUGIN: &str = "plugin";

pub(super) trait RuntimeEventPayload: Serialize + DeserializeOwned + Sized {
    const EVENT_TYPE: &'static str;

    fn into_kind(self) -> EventKind {
        EventKind::from_payload(Self::EVENT_TYPE, &self)
            .expect("typed runtime event payload must serialize as an object")
    }

    fn decode(kind: &EventKind) -> Result<Self, EventKindError> {
        if !kind.is_type(Self::EVENT_TYPE) {
            return Err(EventKindError::UnexpectedEventType {
                expected: Self::EVENT_TYPE,
                actual: kind.event_type().to_owned(),
            });
        }
        kind.decode_payload()
    }
}

macro_rules! runtime_event_payload {
    ($name:ident, $event_type:ident, { $($field:ident: $field_type:ty),* $(,)? }) => {
        #[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
        #[serde(deny_unknown_fields)]
        pub(super) struct $name {
            $(pub(super) $field: $field_type,)*
        }

        impl RuntimeEventPayload for $name {
            const EVENT_TYPE: &'static str = $event_type;
        }
    };
}

runtime_event_payload!(MoveOrdered, MOVE_ORDERED, {
    army: ArmyId,
    from: TerritoryId,
    to: TerritoryId,
    arrival_at: SimTime,
});
runtime_event_payload!(ArmyArrived, ARMY_ARRIVED, {
    army: ArmyId,
    territory: TerritoryId,
});
runtime_event_payload!(PersonMoveOrdered, PERSON_MOVE_ORDERED, {
    person: PersonId,
    from: TerritoryId,
    to: TerritoryId,
    arrival_at: SimTime,
});
runtime_event_payload!(PersonArrived, PERSON_ARRIVED, {
    person: PersonId,
    territory: TerritoryId,
});
runtime_event_payload!(LetterDelivered, LETTER_DELIVERED, {
    letter: LetterId,
    carrier: PersonId,
    recipient: PersonId,
    territory: TerritoryId,
});
runtime_event_payload!(ReportDispatched, REPORT_DISPATCHED, {
    recipient: PersonId,
    army: ArmyId,
    arrives_at: SimTime,
});
runtime_event_payload!(KnowledgeUpdated, KNOWLEDGE_UPDATED, {
    recipient: PersonId,
    army: ArmyId,
    known_location: TerritoryId,
});
runtime_event_payload!(KnowledgePublished, KNOWLEDGE_PUBLISHED, {
    holder: KnowledgeHolderRef,
    record_count: u32,
});
runtime_event_payload!(DebugFieldChanged, DEBUG_FIELD_CHANGED, {
    entity: EntityRef,
    field: String,
    old_value: String,
    new_value: String,
});

pub(super) fn canonicalize_event_kind(kind: &mut EventKind) -> Result<(), EventKindError> {
    let canonical = match kind.event_type() {
        MOVE_ORDERED => MoveOrdered::decode(kind)?.into_kind(),
        ARMY_ARRIVED => ArmyArrived::decode(kind)?.into_kind(),
        PERSON_MOVE_ORDERED => PersonMoveOrdered::decode(kind)?.into_kind(),
        PERSON_ARRIVED => PersonArrived::decode(kind)?.into_kind(),
        LETTER_DELIVERED => LetterDelivered::decode(kind)?.into_kind(),
        REPORT_DISPATCHED => ReportDispatched::decode(kind)?.into_kind(),
        KNOWLEDGE_UPDATED => KnowledgeUpdated::decode(kind)?.into_kind(),
        KNOWLEDGE_PUBLISHED => KnowledgePublished::decode(kind)?.into_kind(),
        DEBUG_FIELD_CHANGED => DebugFieldChanged::decode(kind)?.into_kind(),
        PLUGIN => {
            let (plugin, event_type) =
                kind.plugin_identity()
                    .ok_or_else(|| EventKindError::UnexpectedEventType {
                        expected: PLUGIN,
                        actual: kind.event_type().to_owned(),
                    })?;
            EventKind::plugin(plugin, event_type)
        }
        _ => return Ok(()),
    };
    *kind = canonical;
    Ok(())
}
