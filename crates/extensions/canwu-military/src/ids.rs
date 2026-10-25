//! Identifiers, record kinds, record references, and scenario installation.

use crate::{
    CombatRegistryV1, CombatRulebookV1, CrossingStateV1, EngagementStateV1, EngagementStepsV1,
    EngagementSummaryV1, ForceStateV1, PLUGIN_NAME, PLUGIN_NAMESPACE, SiegeStateV1,
};
use canwu_api::{
    CanwuError, DomainEntityKindClass, DomainKindClass, DomainRecord, DomainRecordClass,
    DomainRecordDraft, DomainRecordLifecycle, DomainRecordType, DomainValueKindClass, ErrorCode,
    TypedDomainRecordRef, canonical_hash,
};
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt::{Display, Formatter};

/// The schema version every payload carries: 2 since 0.21.0, when the
/// engagement record gained its own `max_minutes`.
pub const SCHEMA_VERSION: u32 = 2;
/// The longest identifier the plugin accepts.
pub const MAX_ID_BYTES: usize = 192;
/// The longest siege or crossing identifier: the plugin derives the
/// identifiers of the engagements it opens from it.
pub const MAX_PARENT_ID_BYTES: usize = 160;

pub(crate) fn invalid(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidDomainRecord, message.into())
}

/// Checks a namespaced identifier: 1 to [`MAX_ID_BYTES`] bytes of ASCII
/// letters, digits, and `. _ - : /`, with at least one `:`.
pub fn validate_id(value: &str, label: &str) -> Result<(), CanwuError> {
    if value.is_empty()
        || value.len() > MAX_ID_BYTES
        || !value.contains(':')
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':' | b'/')
        })
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidPayload,
            format!("{label} is not a namespaced identifier"),
        ));
    }
    Ok(())
}

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident, $label:literal) => {
        $(#[$doc])*
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, CanwuError> {
                let value = value.into();
                validate_id(&value, $label)?;
                Ok(Self(value))
            }
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl Display for $name {
            fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(&self.0)
            }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                Self::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
            }
        }
    };
}

id_type!(
    /// One installed rulebook.
    RulebookId,
    "rulebook"
);
id_type!(
    /// One force as combat knows it.
    ForceId,
    "force"
);
id_type!(
    /// One component of a force (a unit, a battery, a squadron).
    ComponentId,
    "component"
);
id_type!(
    /// One engagement.
    EngagementId,
    "engagement"
);
id_type!(
    /// One siege.
    SiegeId,
    "siege"
);
id_type!(
    /// One contested crossing.
    CrossingId,
    "crossing"
);
id_type!(
    /// One step profile of the rulebook.
    ProfileId,
    "step profile"
);
id_type!(
    /// One terrain class of the rulebook.
    TerrainClassId,
    "terrain class"
);
id_type!(
    /// One side trait (for example a mounted-only side).
    TraitId,
    "trait"
);
id_type!(
    /// One traced term: a forecast term or a step term.
    TermId,
    "term"
);
id_type!(
    /// One host-declared condition channel.
    ChannelId,
    "condition channel"
);
id_type!(
    /// One posture of the rulebook.
    PostureId,
    "posture"
);
id_type!(
    /// One ground class (firm, soft, frozen, ...).
    GroundClassId,
    "ground class"
);
id_type!(
    /// One concealment class (clear, fog, ...).
    ConcealmentClassId,
    "concealment class"
);
id_type!(
    /// One forecast band of the rulebook.
    ForecastBandId,
    "forecast band"
);
id_type!(
    /// One ration level the host names for a besieged site.
    RationLevelId,
    "ration level"
);

macro_rules! record_type {
    ($(#[$doc:meta])* $name:ident, $payload:ty, $record_name:literal, $class:ty) => {
        $(#[$doc])*
        pub struct $name;
        impl DomainRecordType for $name {
            type Payload = $payload;
            type Class = $class;
            const NAMESPACE: &'static str = PLUGIN_NAMESPACE;
            const NAME: &'static str = $record_name;
        }
    };
}

record_type!(
    /// The installed rulebook (create-only, one per run).
    RulebookRecord,
    CombatRulebookV1,
    "rulebook",
    DomainValueKindClass
);
record_type!(
    /// The registry of open work and queues (one, id `root`).
    RegistryRecord,
    CombatRegistryV1,
    "registry",
    DomainValueKindClass
);
record_type!(
    /// A force; an entity, so it may hold knowledge.
    ForceRecord,
    ForceStateV1,
    "force",
    DomainEntityKindClass
);
record_type!(
    /// An engagement.
    EngagementRecord,
    EngagementStateV1,
    "engagement",
    DomainValueKindClass
);
record_type!(
    /// The steps of an engagement's latest stepping boundary.
    EngagementStepsRecord,
    EngagementStepsV1,
    "engagement_steps",
    DomainValueKindClass
);
record_type!(
    /// The durable summary of a closed engagement (create-only).
    EngagementSummaryRecord,
    EngagementSummaryV1,
    "engagement_summary",
    DomainValueKindClass
);
record_type!(
    /// A siege.
    SiegeRecord,
    SiegeStateV1,
    "siege",
    DomainValueKindClass
);
record_type!(
    /// A contested crossing.
    CrossingRecord,
    CrossingStateV1,
    "crossing",
    DomainValueKindClass
);

#[must_use]
pub fn rulebook_reference(id: &RulebookId) -> TypedDomainRecordRef<RulebookRecord> {
    TypedDomainRecordRef::new(id.as_str())
}
#[must_use]
pub fn registry_reference() -> TypedDomainRecordRef<RegistryRecord> {
    TypedDomainRecordRef::new("root")
}
#[must_use]
pub fn force_reference(id: &ForceId) -> TypedDomainRecordRef<ForceRecord> {
    TypedDomainRecordRef::new(id.as_str())
}
#[must_use]
pub fn engagement_reference(id: &EngagementId) -> TypedDomainRecordRef<EngagementRecord> {
    TypedDomainRecordRef::new(id.as_str())
}
#[must_use]
pub fn engagement_steps_reference(
    id: &EngagementId,
) -> TypedDomainRecordRef<EngagementStepsRecord> {
    TypedDomainRecordRef::new(id.as_str())
}
#[must_use]
pub fn engagement_summary_reference(
    id: &EngagementId,
) -> TypedDomainRecordRef<EngagementSummaryRecord> {
    TypedDomainRecordRef::new(id.as_str())
}
#[must_use]
pub fn siege_reference(id: &SiegeId) -> TypedDomainRecordRef<SiegeRecord> {
    TypedDomainRecordRef::new(id.as_str())
}
#[must_use]
pub fn crossing_reference(id: &CrossingId) -> TypedDomainRecordRef<CrossingRecord> {
    TypedDomainRecordRef::new(id.as_str())
}

/// Builds an initial-scenario record of this plugin (version 1, active).
pub fn record_from<T: DomainRecordType>(
    reference: TypedDomainRecordRef<T>,
    payload: &T::Payload,
) -> Result<DomainRecord, CanwuError>
where
    T::Payload: Serialize,
{
    let draft = DomainRecordDraft::from_typed(reference, payload)?;
    Ok(DomainRecord {
        reference: draft.reference,
        owner: PLUGIN_NAME.to_owned(),
        class: if <T::Class as DomainKindClass>::IS_ENTITY {
            DomainRecordClass::Entity
        } else {
            DomainRecordClass::Record
        },
        version: 1,
        lifecycle: DomainRecordLifecycle::Active,
        payload: draft.payload,
        references: draft.references,
    })
}

/// The canonical digest of a value, as the step's input digests use it.
pub fn digest<T: Serialize>(value: &T) -> Result<String, CanwuError> {
    canonical_hash("canwu.military.value.v1", value)
}
