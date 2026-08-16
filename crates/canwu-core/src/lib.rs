//! Stable identifiers, deterministic utilities, and lightweight schema metadata.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};

macro_rules! define_id {
    ($name:ident) => {
        #[derive(
            Clone,
            Copy,
            Debug,
            Default,
            Deserialize,
            Eq,
            Hash,
            Ord,
            PartialEq,
            PartialOrd,
            Serialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub u64);

        impl $name {
            #[must_use]
            pub const fn new(value: u64) -> Self {
                Self(value)
            }

            #[must_use]
            pub const fn get(self) -> u64 {
                self.0
            }
        }

        impl Display for $name {
            fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
                Display::fmt(&self.0, formatter)
            }
        }
    };
}

define_id!(ArmyId);
define_id!(BoundaryId);
define_id!(CommandAttemptId);
define_id!(CommandId);
define_id!(CommandRequestId);
define_id!(EventId);
define_id!(GovernmentId);
define_id!(IngressId);
define_id!(OrganizationId);
define_id!(PersonId);
define_id!(RandomDrawId);
define_id!(ResourceId);
define_id!(RouteId);
define_id!(TerritoryId);

/// Stable application-defined record kind. Namespaces and names are validated
/// by the simulation package registry before authoritative use.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct DomainRecordKind {
    pub namespace: String,
    pub name: String,
}

impl DomainRecordKind {
    #[must_use]
    pub fn new(namespace: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            namespace: namespace.into(),
            name: name.into(),
        }
    }
}

impl Display for DomainRecordKind {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}.{}", self.namespace, self.name)
    }
}

/// Stable string identity for an application-defined entity or record.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct DomainRecordRef {
    pub kind: DomainRecordKind,
    pub id: String,
}

impl DomainRecordRef {
    #[must_use]
    pub fn new(
        namespace: impl Into<String>,
        kind: impl Into<String>,
        id: impl Into<String>,
    ) -> Self {
        Self {
            kind: DomainRecordKind::new(namespace, kind),
            id: id.into(),
        }
    }
}

impl Display for DomainRecordRef {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}:{}", self.kind, self.id)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreEntityKind {
    Army,
    Government,
    Organization,
    Person,
    Resource,
    Route,
    Territory,
}

/// Serializable entity reference used by events, queries, and generic tools.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "type", content = "id", rename_all = "snake_case")]
pub enum EntityRef {
    Army(ArmyId),
    Domain(DomainRecordRef),
    Government(GovernmentId),
    Organization(OrganizationId),
    Person(PersonId),
    Resource(ResourceId),
    Route(RouteId),
    Territory(TerritoryId),
}

impl Display for EntityRef {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Army(id) => write!(formatter, "army:{id}"),
            Self::Domain(reference) => write!(formatter, "domain:{reference}"),
            Self::Government(id) => write!(formatter, "government:{id}"),
            Self::Organization(id) => write!(formatter, "organization:{id}"),
            Self::Person(id) => write!(formatter, "person:{id}"),
            Self::Resource(id) => write!(formatter, "resource:{id}"),
            Self::Route(id) => write!(formatter, "route:{id}"),
            Self::Territory(id) => write!(formatter, "territory:{id}"),
        }
    }
}

impl EntityRef {
    #[must_use]
    pub const fn core_kind(&self) -> Option<CoreEntityKind> {
        match self {
            Self::Army(_) => Some(CoreEntityKind::Army),
            Self::Domain(_) => None,
            Self::Government(_) => Some(CoreEntityKind::Government),
            Self::Organization(_) => Some(CoreEntityKind::Organization),
            Self::Person(_) => Some(CoreEntityKind::Person),
            Self::Resource(_) => Some(CoreEntityKind::Resource),
            Self::Route(_) => Some(CoreEntityKind::Route),
            Self::Territory(_) => Some(CoreEntityKind::Territory),
        }
    }
}

/// `SplitMix64` is compact, deterministic, serializable, and sufficient for the
/// initial movement slice.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DeterministicRng {
    state: u64,
}

impl DeterministicRng {
    const STEP: u64 = 0x9E37_79B9_7F4A_7C15;

    #[must_use]
    pub const fn from_seed(seed: u64) -> Self {
        Self { state: seed }
    }

    #[must_use]
    pub const fn state(self) -> u64 {
        self.state
    }

    #[must_use]
    pub const fn state_after(seed: u64, draws: u64) -> u64 {
        seed.wrapping_add(Self::STEP.wrapping_mul(draws))
    }

    #[must_use]
    pub const fn seed_before(state: u64, draws: u64) -> u64 {
        state.wrapping_sub(Self::STEP.wrapping_mul(draws))
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(Self::STEP);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        value ^ (value >> 31)
    }

    /// Returns a value in `[0, upper_exclusive)`. Zero returns zero.
    pub fn range(&mut self, upper_exclusive: u64) -> u64 {
        if upper_exclusive == 0 {
            return 0;
        }
        self.next_u64() % upper_exclusive
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FieldSchema {
    pub name: String,
    pub value_type: String,
    pub description: String,
    pub reference_type: Option<String>,
    pub writable_via_debug_command: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TypeSchema {
    pub type_name: String,
    pub description: String,
    pub fields: Vec<FieldSchema>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SchemaRegistry {
    types: BTreeMap<String, TypeSchema>,
}

impl SchemaRegistry {
    pub fn register(&mut self, schema: TypeSchema) {
        self.types.insert(schema.type_name.clone(), schema);
    }

    #[must_use]
    pub fn get(&self, type_name: &str) -> Option<&TypeSchema> {
        self.types.get(type_name)
    }

    pub fn iter(&self) -> impl Iterator<Item = &TypeSchema> {
        self.types.values()
    }
}
