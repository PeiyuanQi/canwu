//! Inspectable, serializable events with compact causal provenance.

use canwu_core::{BoundaryId, CommandId, EntityRef, EventId, KnowledgeHolderRef, PersonId};
use canwu_time::SimTime;
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{self, DeserializeOwned, MapAccess, SeqAccess, Visitor},
    ser::SerializeMap,
};
use serde_json::{Number, Value};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{self, Display, Formatter};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "id", rename_all = "snake_case")]
pub enum CauseRef {
    Boundary(BoundaryId),
    Command(CommandId),
    Event(EventId),
    System(String),
}

/// Domain-neutral event identity and structured fields.
///
/// The `type` tag and flattened field layout deliberately preserve the wire
/// shape used by earlier concrete event variants. Domain crates and runtime
/// compatibility modules own typed payloads and encode them through
/// [`Self::from_payload`]; this crate does not own their vocabulary.
#[derive(Clone, Debug)]
pub struct EventKind {
    event_type: String,
    fields: Vec<(String, EventValue)>,
}

#[derive(Clone, Debug)]
enum EventValue {
    Null,
    Bool(bool),
    Number(Number),
    String(String),
    Array(Vec<Self>),
    Object(Vec<(String, Self)>),
}

impl PartialEq for EventValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Null, Self::Null) => true,
            (Self::Bool(left), Self::Bool(right)) => left == right,
            (Self::Number(left), Self::Number(right)) => left == right,
            (Self::String(left), Self::String(right)) => left == right,
            (Self::Array(left), Self::Array(right)) => left == right,
            (Self::Object(left), Self::Object(right)) => {
                left.len() == right.len()
                    && left.iter().all(|(key, value)| {
                        right
                            .iter()
                            .find_map(|(candidate, value)| (candidate == key).then_some(value))
                            == Some(value)
                    })
            }
            _ => false,
        }
    }
}

impl Eq for EventValue {}

impl PartialEq for EventKind {
    fn eq(&self, other: &Self) -> bool {
        self.event_type == other.event_type
            && self.fields.len() == other.fields.len()
            && self.fields.iter().all(|(key, value)| {
                other
                    .field_value(key)
                    .is_some_and(|candidate| candidate == value)
            })
    }
}

impl Eq for EventKind {}

impl EventValue {
    fn from_json(value: Value) -> Self {
        match value {
            Value::Null => Self::Null,
            Value::Bool(value) => Self::Bool(value),
            Value::Number(value) => Self::Number(value),
            Value::String(value) => Self::String(value),
            Value::Array(values) => Self::Array(values.into_iter().map(Self::from_json).collect()),
            Value::Object(fields) => Self::Object(
                fields
                    .into_iter()
                    .map(|(key, value)| (key, Self::from_json(value)))
                    .collect(),
            ),
        }
    }

    fn into_json(self) -> Value {
        match self {
            Self::Null => Value::Null,
            Self::Bool(value) => Value::Bool(value),
            Self::Number(value) => Value::Number(value),
            Self::String(value) => Value::String(value),
            Self::Array(values) => Value::Array(values.into_iter().map(Self::into_json).collect()),
            Self::Object(fields) => Value::Object(
                fields
                    .into_iter()
                    .map(|(key, value)| (key, value.into_json()))
                    .collect(),
            ),
        }
    }

    fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }
}

impl Serialize for EventValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Null => serializer.serialize_none(),
            Self::Bool(value) => serializer.serialize_bool(*value),
            Self::Number(value) => value.serialize(serializer),
            Self::String(value) => serializer.serialize_str(value),
            Self::Array(values) => values.serialize(serializer),
            Self::Object(fields) => {
                let mut map = serializer.serialize_map(Some(fields.len()))?;
                let mut ordered = fields.iter().collect::<Vec<_>>();
                ordered.sort_by(|left, right| left.0.cmp(&right.0));
                for (key, value) in ordered {
                    map.serialize_entry(key, value)?;
                }
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for EventValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct EventValueVisitor;

        impl<'de> Visitor<'de> for EventValueVisitor {
            type Value = EventValue;

            fn expecting(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
                formatter.write_str("a JSON-compatible event field value")
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(EventValue::Null)
            }

            fn visit_none<E>(self) -> Result<Self::Value, E> {
                Ok(EventValue::Null)
            }

            fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
            where
                D: Deserializer<'de>,
            {
                EventValue::deserialize(deserializer)
            }

            fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
                Ok(EventValue::Bool(value))
            }

            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
                Ok(EventValue::Number(value.into()))
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
                Ok(EventValue::Number(value.into()))
            }

            fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Number::from_f64(value)
                    .map(EventValue::Number)
                    .ok_or_else(|| E::custom("event field contains a non-finite number"))
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
                Ok(EventValue::String(value.to_owned()))
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
                Ok(EventValue::String(value))
            }

            fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut values = Vec::with_capacity(sequence.size_hint().unwrap_or(0));
                while let Some(value) = sequence.next_element()? {
                    values.push(value);
                }
                Ok(EventValue::Array(values))
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut fields = Vec::with_capacity(map.size_hint().unwrap_or(0));
                while let Some(key) = map.next_key::<String>()? {
                    if fields.iter().any(|(existing, _)| existing == &key) {
                        return Err(de::Error::custom(format!(
                            "event field object contains duplicate key {key}"
                        )));
                    }
                    fields.push((key, map.next_value()?));
                }
                Ok(EventValue::Object(fields))
            }
        }

        deserializer.deserialize_any(EventValueVisitor)
    }
}

impl Serialize for EventKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(self.fields.len() + 1))?;
        map.serialize_entry("type", &self.event_type)?;
        let mut ordered = self.fields.iter().collect::<Vec<_>>();
        ordered.sort_by(|left, right| left.0.cmp(&right.0));
        for (key, value) in ordered {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for EventKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct EventKindVisitor;

        impl<'de> Visitor<'de> for EventKindVisitor {
            type Value = EventKind;

            fn expecting(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
                formatter.write_str("an event object with a type field")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut event_type = None;
                let mut fields = Vec::with_capacity(map.size_hint().unwrap_or(1).saturating_sub(1));
                while let Some(key) = map.next_key::<String>()? {
                    if key == "type" {
                        if event_type.is_some() {
                            return Err(de::Error::duplicate_field("type"));
                        }
                        event_type = Some(map.next_value::<String>()?);
                    } else {
                        if fields.iter().any(|(existing, _)| existing == &key) {
                            return Err(de::Error::custom(format!(
                                "event payload contains duplicate field {key}"
                            )));
                        }
                        fields.push((key, map.next_value()?));
                    }
                }
                let event_type = event_type.ok_or_else(|| de::Error::missing_field("type"))?;
                if event_type.is_empty() {
                    return Err(de::Error::custom("event type cannot be empty"));
                }
                Ok(EventKind { event_type, fields })
            }
        }

        deserializer.deserialize_map(EventKindVisitor)
    }
}

#[derive(Deserialize)]
struct OrderedFields(
    #[serde(deserialize_with = "deserialize_ordered_fields")] Vec<(String, EventValue)>,
);

fn deserialize_ordered_fields<'de, D>(
    deserializer: D,
) -> Result<Vec<(String, EventValue)>, D::Error>
where
    D: Deserializer<'de>,
{
    struct OrderedFieldsVisitor;

    impl<'de> Visitor<'de> for OrderedFieldsVisitor {
        type Value = Vec<(String, EventValue)>;

        fn expecting(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
            formatter.write_str("an event payload object")
        }

        fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
        where
            A: MapAccess<'de>,
        {
            let mut fields = Vec::with_capacity(map.size_hint().unwrap_or(0));
            while let Some(key) = map.next_key::<String>()? {
                if fields.iter().any(|(existing, _)| existing == &key) {
                    return Err(de::Error::custom(format!(
                        "event payload contains duplicate field {key}"
                    )));
                }
                fields.push((key, map.next_value()?));
            }
            Ok(fields)
        }
    }

    deserializer.deserialize_map(OrderedFieldsVisitor)
}

#[derive(Debug)]
pub enum EventKindError {
    EmptyEventType,
    ReservedTypeField,
    PayloadMustBeObject,
    MissingField(String),
    UnexpectedEventType {
        expected: &'static str,
        actual: String,
    },
    InvalidPayload(serde_json::Error),
}

impl Display for EventKindError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyEventType => formatter.write_str("event type cannot be empty"),
            Self::ReservedTypeField => {
                formatter.write_str("event payload cannot contain the reserved type field")
            }
            Self::PayloadMustBeObject => {
                formatter.write_str("event payload must serialize as an object")
            }
            Self::MissingField(field) => {
                write!(formatter, "event payload is missing field {field}")
            }
            Self::UnexpectedEventType { expected, actual } => {
                write!(formatter, "expected event type {expected}, found {actual}")
            }
            Self::InvalidPayload(error) => Display::fmt(error, formatter),
        }
    }
}

impl Error for EventKindError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidPayload(error) => Some(error),
            Self::EmptyEventType
            | Self::ReservedTypeField
            | Self::PayloadMustBeObject
            | Self::MissingField(_)
            | Self::UnexpectedEventType { .. } => None,
        }
    }
}

impl From<serde_json::Error> for EventKindError {
    fn from(error: serde_json::Error) -> Self {
        Self::InvalidPayload(error)
    }
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
    /// Builds an event record from an explicit type and flattened fields.
    ///
    /// # Errors
    ///
    /// Returns an error when the type is empty or the fields contain the
    /// reserved `type` key.
    pub fn from_fields(
        event_type: impl Into<String>,
        fields: BTreeMap<String, Value>,
    ) -> Result<Self, EventKindError> {
        let event_type = event_type.into();
        if event_type.is_empty() {
            return Err(EventKindError::EmptyEventType);
        }
        if fields.contains_key("type") {
            return Err(EventKindError::ReservedTypeField);
        }
        Ok(Self {
            event_type,
            fields: fields
                .into_iter()
                .map(|(key, value)| (key, EventValue::from_json(value)))
                .collect(),
        })
    }

    /// Serializes a typed domain payload into an event record.
    ///
    /// # Errors
    ///
    /// Returns an error when serialization fails, the payload is not an
    /// object, the type is empty, or the payload contains the reserved `type`
    /// key.
    pub fn from_payload<T: Serialize>(
        event_type: impl Into<String>,
        payload: &T,
    ) -> Result<Self, EventKindError> {
        let encoded = serde_json::to_vec(payload)?;
        let OrderedFields(fields) =
            serde_json::from_slice(&encoded).map_err(|_| EventKindError::PayloadMustBeObject)?;
        let event_type = event_type.into();
        if event_type.is_empty() {
            return Err(EventKindError::EmptyEventType);
        }
        if fields.iter().any(|(key, _)| key == "type") {
            return Err(EventKindError::ReservedTypeField);
        }
        Ok(Self { event_type, fields })
    }

    /// Constructs the compatibility wire identity for a plugin event.
    #[must_use]
    pub fn plugin(plugin: impl Into<String>, event_type: impl Into<String>) -> Self {
        Self {
            event_type: "plugin".to_owned(),
            fields: vec![
                ("plugin".to_owned(), EventValue::String(plugin.into())),
                (
                    "event_type".to_owned(),
                    EventValue::String(event_type.into()),
                ),
            ],
        }
    }

    #[must_use]
    pub fn event_type(&self) -> &str {
        &self.event_type
    }

    #[must_use]
    pub fn is_type(&self, event_type: &str) -> bool {
        self.event_type == event_type
    }

    #[must_use]
    pub fn fields(&self) -> BTreeMap<String, Value> {
        self.fields
            .iter()
            .cloned()
            .map(|(key, value)| (key, value.into_json()))
            .collect()
    }

    #[must_use]
    pub fn field(&self, name: &str) -> Option<Value> {
        self.field_value(name).cloned().map(EventValue::into_json)
    }

    /// Replaces one existing field with a serializable value.
    ///
    /// # Errors
    ///
    /// Returns an error when the field is absent or the value cannot be
    /// represented as structured JSON data.
    pub fn set_field<T: Serialize>(&mut self, name: &str, value: &T) -> Result<(), EventKindError> {
        let encoded = serde_json::to_vec(value)?;
        let replacement = serde_json::from_slice::<EventValue>(&encoded)?;
        let field = self
            .fields
            .iter_mut()
            .find(|(key, _)| key == name)
            .ok_or_else(|| EventKindError::MissingField(name.to_owned()))?;
        field.1 = replacement;
        Ok(())
    }

    /// Decodes all flattened fields into a domain-owned payload type.
    ///
    /// # Errors
    ///
    /// Returns an error when the fields do not deserialize as `T`.
    pub fn decode_payload<T: DeserializeOwned>(&self) -> Result<T, EventKindError> {
        struct Payload<'a>(&'a [(String, EventValue)]);

        impl Serialize for Payload<'_> {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                let mut map = serializer.serialize_map(Some(self.0.len()))?;
                for (key, value) in self.0 {
                    map.serialize_entry(key, value)?;
                }
                map.end()
            }
        }

        let encoded = serde_json::to_vec(&Payload(&self.fields))?;
        Ok(serde_json::from_slice(&encoded)?)
    }

    /// Decodes one named field.
    ///
    /// # Errors
    ///
    /// Returns an error when the field is absent or does not deserialize as
    /// `T`.
    pub fn decode_field<T: DeserializeOwned>(&self, name: &str) -> Result<T, EventKindError> {
        let value = self
            .field_value(name)
            .ok_or_else(|| EventKindError::MissingField(name.to_owned()))?;
        let encoded = serde_json::to_vec(value)?;
        Ok(serde_json::from_slice(&encoded)?)
    }

    #[must_use]
    pub fn plugin_identity(&self) -> Option<(&str, &str)> {
        if self.event_type != "plugin" || self.fields.len() != 2 {
            return None;
        }
        Some((
            self.field_value("plugin")?.as_str()?,
            self.field_value("event_type")?.as_str()?,
        ))
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
        self.plugin_identity().map_or_else(
            || self.event_type.clone(),
            |(plugin, event_type)| format!("{plugin}.{event_type}"),
        )
    }

    fn field_value(&self, name: &str) -> Option<&EventValue> {
        self.fields
            .iter()
            .find_map(|(key, value)| (key == name).then_some(value))
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
    use serde::{Deserialize, Serialize};
    use serde_json::json;

    #[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
    struct ArmyArrived {
        army: ArmyId,
        territory: TerritoryId,
    }

    #[test]
    fn qualified_event_type_preserves_legacy_labels_and_disambiguates_plugins() {
        let payload = ArmyArrived {
            army: ArmyId::new(1),
            territory: TerritoryId::new(2),
        };
        let built_in = EventKind::from_payload("army_arrived", &payload).unwrap();
        assert_eq!(built_in.event_type(), "army_arrived");
        assert_eq!(built_in.qualified_event_type(), "army_arrived");
        assert_eq!(built_in.decode_payload::<ArmyArrived>().unwrap(), payload);
        assert_eq!(
            serde_json::to_value(&built_in).unwrap(),
            json!({"type": "army_arrived", "army": 1, "territory": 2})
        );

        let supply = EventKind::plugin("example-supply", "grain_allocated");
        let demand = EventKind::plugin("example-demand", "grain_allocated");

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
        assert_eq!(
            serde_json::to_value(&supply).unwrap(),
            json!({
                "type": "plugin",
                "plugin": "example-supply",
                "event_type": "grain_allocated"
            })
        );
        assert_eq!(
            serde_json::to_string(&supply).unwrap(),
            r#"{"type":"plugin","event_type":"grain_allocated","plugin":"example-supply"}"#
        );
    }

    #[test]
    fn event_wire_shapes_round_trip_in_canonical_field_order() {
        let fixtures = [
            r#"{"type":"move_ordered","army":1,"arrival_at":60,"from":2,"to":3}"#,
            r#"{"type":"army_arrived","army":1,"territory":3}"#,
            r#"{"type":"person_move_ordered","arrival_at":60,"from":2,"person":4,"to":3}"#,
            r#"{"type":"person_arrived","person":4,"territory":3}"#,
            r#"{"type":"letter_delivered","carrier":4,"letter":5,"recipient":6,"territory":3}"#,
            r#"{"type":"report_dispatched","army":1,"arrives_at":90,"recipient":4}"#,
            r#"{"type":"knowledge_updated","army":1,"known_location":3,"recipient":4}"#,
            r#"{"type":"knowledge_published","holder":{"id":4,"type":"person"},"record_count":2}"#,
            r#"{"type":"debug_field_changed","entity":{"id":1,"type":"army"},"field":"morale","new_value":"75","old_value":"70"}"#,
            r#"{"type":"plugin","event_type":"changed","plugin":"example"}"#,
        ];

        for fixture in fixtures {
            let event: EventKind = serde_json::from_str(fixture).unwrap();
            assert_eq!(serde_json::to_string(&event).unwrap(), fixture);
        }

        let ordered: EventKind =
            serde_json::from_str(r#"{"type":"army_arrived","army":1,"territory":3}"#).unwrap();
        let reordered: EventKind =
            serde_json::from_str(r#"{"territory":3,"army":1,"type":"army_arrived"}"#).unwrap();
        assert_eq!(ordered, reordered);

        let plugin_with_extra: EventKind = serde_json::from_str(
            r#"{"type":"plugin","plugin":"example","event_type":"changed","extra":true}"#,
        )
        .unwrap();
        assert_eq!(plugin_with_extra.plugin_identity(), None);
    }
}
