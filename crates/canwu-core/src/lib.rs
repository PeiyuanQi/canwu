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
define_id!(CommandId);
define_id!(EventId);
define_id!(GovernmentId);
define_id!(OrganizationId);
define_id!(PersonId);
define_id!(ResourceId);
define_id!(RouteId);
define_id!(TerritoryId);

/// Serializable entity reference used by events, queries, and generic tools.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "type", content = "id", rename_all = "snake_case")]
pub enum EntityRef {
    Army(ArmyId),
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
            Self::Government(id) => write!(formatter, "government:{id}"),
            Self::Organization(id) => write!(formatter, "organization:{id}"),
            Self::Person(id) => write!(formatter, "person:{id}"),
            Self::Resource(id) => write!(formatter, "resource:{id}"),
            Self::Route(id) => write!(formatter, "route:{id}"),
            Self::Territory(id) => write!(formatter, "territory:{id}"),
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
    #[must_use]
    pub const fn from_seed(seed: u64) -> Self {
        Self { state: seed }
    }

    #[must_use]
    pub const fn state(self) -> u64 {
        self.state
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
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
