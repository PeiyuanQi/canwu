//! Stable identifiers, deterministic utilities, and lightweight schema metadata.

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

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
define_id!(DecisionRequestId);
define_id!(DecisionTicketId);
define_id!(DecisionTraceId);
define_id!(EventId);
define_id!(GovernmentId);
define_id!(IngressId);
define_id!(HolderKnowledgeRecordId);
define_id!(LetterId);
define_id!(OrganizationId);
define_id!(PersonId);
define_id!(RandomDrawId);
define_id!(KnowledgeRecordId);
define_id!(ResourceId);
define_id!(RouteId);
define_id!(TerritoryId);

/// Generic simulation granularity used by host applications to map aggregate,
/// group, and individual actors onto the same authoritative engine.
///
/// The engine deliberately does not call these levels "population", "special
/// group", or "character". Those are content terms owned by a reference
/// integration such as Celestial Mandate. A host may map its population model
/// to [`Self::Aggregate`], its special groups to [`Self::Group`], and its
/// characters to [`Self::Actor`] without changing the kernel's identity wire
/// format.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SimulationGranularity {
    Aggregate,
    Group,
    Actor,
}

impl SimulationGranularity {
    /// Returns the stable public label used in manifests and diagnostics.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Aggregate => "aggregate",
            Self::Group => "group",
            Self::Actor => "actor",
        }
    }
}

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

    #[must_use]
    pub fn for_type<T: DomainRecordType>() -> Self {
        Self::new(T::NAMESPACE, T::NAME)
    }

    #[must_use]
    pub fn matches_type<T: DomainRecordType>(&self) -> bool {
        self.namespace == T::NAMESPACE && self.name == T::NAME
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

/// Persisted identity of the operation that established one domain-record
/// version. Version zero is reserved and rejected by runtime validation.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DomainRecordVersionSource {
    InitialScenario,
    BoundaryChange {
        boundary: BoundaryId,
        change_index: u64,
    },
}

/// Exact historical identity for an application-defined record version.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct DomainRecordVersionRef {
    pub record: DomainRecordRef,
    pub version: u64,
    pub established_by: DomainRecordVersionSource,
}

/// Shared persisted-evidence identity used by knowledge, decisions, random
/// operations, replay, and compact archive receipts.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum EvidenceRef {
    Command(CommandId),
    CommandAttempt(CommandAttemptId),
    Event(EventId),
    Ingress(IngressId),
    Boundary(BoundaryId),
    RandomDraw(RandomDrawId),
    DomainRecordVersion(DomainRecordVersionRef),
}

/// Stable namespace and kind for a holder-relative knowledge record.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct KnowledgeRecordKind {
    pub namespace: String,
    pub name: String,
}

impl KnowledgeRecordKind {
    #[must_use]
    pub fn new(namespace: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            namespace: namespace.into(),
            name: name.into(),
        }
    }
}

impl Display for KnowledgeRecordKind {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}.{}", self.namespace, self.name)
    }
}

/// Exact version of one registered knowledge schema.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct KnowledgeSchemaId {
    pub kind: KnowledgeRecordKind,
    pub version: u32,
}

impl KnowledgeSchemaId {
    #[must_use]
    pub fn new(kind: KnowledgeRecordKind, version: u32) -> Self {
        Self { kind, version }
    }
}

/// Stable holder identity shared by people and eligible institutional entities.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum KnowledgeHolderRef {
    Person(PersonId),
    Entity(EntityRef),
}

impl KnowledgeHolderRef {
    #[must_use]
    pub fn is_person_entity(&self) -> bool {
        matches!(self, Self::Entity(EntityRef::Person(_)))
    }
}

/// Whether a domain entity schema may receive holder-relative knowledge.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeHolderPolicy {
    #[default]
    Disallowed,
    Allowed,
}

/// Compile-time identity for one versioned holder-relative knowledge schema.
pub trait KnowledgeRecordType {
    type Payload;

    const NAMESPACE: &'static str;
    const NAME: &'static str;
    const SCHEMA_VERSION: u32;
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

/// Compile-time identity for one namespaced application-defined record kind.
///
/// The associated payload stays outside the kernel's type graph. Domain
/// packages use this trait to bind stable identities and payload codecs while
/// Canwu persists the existing schema-validated [`DomainRecordRef`] shape.
pub trait DomainRecordType {
    type Payload;
    type Class: DomainKindClass;

    const NAMESPACE: &'static str;
    const NAME: &'static str;
}

mod domain_kind_class {
    pub trait Sealed {}
}

/// Sealed type-level classification for application-defined record kinds.
pub trait DomainKindClass: domain_kind_class::Sealed {
    const IS_ENTITY: bool;
}

/// Type-level class for domain kinds whose instances are entity identities.
pub enum DomainEntityKindClass {}

impl domain_kind_class::Sealed for DomainEntityKindClass {}

impl DomainKindClass for DomainEntityKindClass {
    const IS_ENTITY: bool = true;
}

/// Type-level class for domain kinds whose instances are non-entity records.
pub enum DomainValueKindClass {}

impl domain_kind_class::Sealed for DomainValueKindClass {}

impl DomainKindClass for DomainValueKindClass {
    const IS_ENTITY: bool = false;
}

/// Marker implemented automatically for entity-class domain record types.
pub trait DomainEntityType: DomainRecordType<Class = DomainEntityKindClass> {}

impl<T: DomainRecordType<Class = DomainEntityKindClass>> DomainEntityType for T {}

/// Marker implemented automatically for non-entity domain record types.
pub trait DomainValueType: DomainRecordType<Class = DomainValueKindClass> {}

impl<T: DomainRecordType<Class = DomainValueKindClass>> DomainValueType for T {}

/// Typed façade over a stable application-defined record identity.
///
/// Its serialized representation is exactly the wrapped [`DomainRecordRef`];
/// the marker exists only at compile time.
#[derive(Serialize)]
#[serde(transparent, bound = "")]
pub struct TypedDomainRecordRef<T: DomainRecordType> {
    reference: DomainRecordRef,
    #[serde(skip)]
    marker: PhantomData<fn() -> T>,
}

impl<T: DomainRecordType> Clone for TypedDomainRecordRef<T> {
    fn clone(&self) -> Self {
        Self {
            reference: self.reference.clone(),
            marker: PhantomData,
        }
    }
}

impl<T: DomainRecordType> std::fmt::Debug for TypedDomainRecordRef<T> {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("TypedDomainRecordRef")
            .field(&self.reference)
            .finish()
    }
}

impl<T: DomainRecordType> PartialEq for TypedDomainRecordRef<T> {
    fn eq(&self, other: &Self) -> bool {
        self.reference == other.reference
    }
}

impl<T: DomainRecordType> Eq for TypedDomainRecordRef<T> {}

impl<T: DomainRecordType> PartialOrd for TypedDomainRecordRef<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T: DomainRecordType> Ord for TypedDomainRecordRef<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.reference.cmp(&other.reference)
    }
}

impl<T: DomainRecordType> Hash for TypedDomainRecordRef<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.reference.hash(state);
    }
}

impl<'de, T: DomainRecordType> Deserialize<'de> for TypedDomainRecordRef<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let reference = DomainRecordRef::deserialize(deserializer)?;
        Self::from_untyped(reference).map_err(|reference| {
            serde::de::Error::custom(format!(
                "domain record reference {reference} does not match typed kind {}",
                DomainRecordKind::for_type::<T>()
            ))
        })
    }
}

impl<T: DomainRecordType> TypedDomainRecordRef<T> {
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            reference: DomainRecordRef {
                kind: DomainRecordKind::for_type::<T>(),
                id: id.into(),
            },
            marker: PhantomData,
        }
    }

    #[must_use]
    pub const fn as_untyped(&self) -> &DomainRecordRef {
        &self.reference
    }

    #[must_use]
    pub fn into_untyped(self) -> DomainRecordRef {
        self.reference
    }

    /// Converts an untyped reference when its namespaced kind matches `T`.
    ///
    /// # Errors
    ///
    /// Returns the original reference when it belongs to another kind.
    pub fn from_untyped(reference: DomainRecordRef) -> Result<Self, DomainRecordRef> {
        if !reference.kind.matches_type::<T>() {
            return Err(reference);
        }
        Ok(Self {
            reference,
            marker: PhantomData,
        })
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.reference.id
    }
}

impl<T: DomainRecordType> Display for TypedDomainRecordRef<T> {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.reference, formatter)
    }
}

impl<T: DomainRecordType> From<TypedDomainRecordRef<T>> for DomainRecordRef {
    fn from(reference: TypedDomainRecordRef<T>) -> Self {
        reference.into_untyped()
    }
}

impl<T: DomainEntityType> From<TypedDomainRecordRef<T>> for EntityRef {
    fn from(reference: TypedDomainRecordRef<T>) -> Self {
        Self::Domain(reference.into_untyped())
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

#[cfg(test)]
mod tests {
    use super::*;

    struct Office;

    impl DomainRecordType for Office {
        type Payload = String;
        type Class = DomainEntityKindClass;

        const NAMESPACE: &'static str = "fixture.governance";
        const NAME: &'static str = "office";
    }

    struct Obligation;

    impl DomainRecordType for Obligation {
        type Payload = String;
        type Class = DomainValueKindClass;

        const NAMESPACE: &'static str = "fixture.governance";
        const NAME: &'static str = "obligation";
    }

    struct Assessment;

    impl KnowledgeRecordType for Assessment {
        type Payload = String;

        const NAMESPACE: &'static str = "fixture.knowledge";
        const NAME: &'static str = "assessment";
        const SCHEMA_VERSION: u32 = 2;
    }

    #[test]
    fn typed_domain_identity_preserves_wire_shape_and_kind_boundary() {
        let typed = TypedDomainRecordRef::<Office>::new("secretariat");
        let raw = DomainRecordRef::new("fixture.governance", "office", "secretariat");

        assert_eq!(
            serde_json::to_value(&typed).expect("typed identity should serialize"),
            serde_json::to_value(&raw).expect("raw identity should serialize")
        );
        let round_trip: TypedDomainRecordRef<Office> = serde_json::from_value(
            serde_json::to_value(&typed).expect("typed identity should serialize"),
        )
        .expect("typed identity should deserialize");
        assert_eq!(round_trip.as_untyped(), &raw);
        assert_eq!(EntityRef::from(round_trip), EntityRef::Domain(raw.clone()));

        let wrong_kind =
            DomainRecordRef::new("fixture.governance", "obligation", "secretariat-duty");
        assert_eq!(
            TypedDomainRecordRef::<Office>::from_untyped(wrong_kind.clone()),
            Err(wrong_kind)
        );
        assert!(TypedDomainRecordRef::<Obligation>::from_untyped(raw).is_err());
        assert!(
            serde_json::from_value::<TypedDomainRecordRef<Office>>(serde_json::json!({
                "kind": {
                    "namespace": "fixture.governance",
                    "name": "obligation"
                },
                "id": "secretariat-duty"
            }))
            .is_err()
        );
    }

    #[test]
    fn knowledge_identity_and_holder_wire_shapes_are_stable() {
        let kind = KnowledgeRecordKind::new(Assessment::NAMESPACE, Assessment::NAME);
        let schema = KnowledgeSchemaId::new(kind.clone(), Assessment::SCHEMA_VERSION);

        assert_eq!(kind.to_string(), "fixture.knowledge.assessment");
        assert_eq!(schema.version, 2);
        assert_eq!(schema.kind, kind);
        assert_eq!(
            KnowledgeHolderPolicy::default(),
            KnowledgeHolderPolicy::Disallowed
        );

        assert_eq!(
            serde_json::to_value(KnowledgeHolderRef::Person(PersonId::new(7)))
                .expect("person holder should serialize"),
            serde_json::json!({ "type": "person", "value": 7 })
        );
        let invalid_shape = KnowledgeHolderRef::Entity(EntityRef::Person(PersonId::new(7)));
        assert!(invalid_shape.is_person_entity());
        let institution =
            KnowledgeHolderRef::Entity(EntityRef::Organization(OrganizationId::new(3)));
        assert!(!institution.is_person_entity());
        assert_eq!(
            serde_json::to_value(institution).expect("institution holder should serialize"),
            serde_json::json!({
                "type": "entity",
                "value": { "type": "organization", "id": 3 }
            })
        );
    }

    #[test]
    fn exact_domain_record_evidence_has_a_stable_wire_identity() {
        let evidence = EvidenceRef::DomainRecordVersion(DomainRecordVersionRef {
            record: DomainRecordRef::new("fixture.information", "dispatch", "dispatch-7"),
            version: 2,
            established_by: DomainRecordVersionSource::BoundaryChange {
                boundary: BoundaryId::new(12),
                change_index: 3,
            },
        });

        assert_eq!(
            serde_json::to_value(evidence).expect("evidence should serialize"),
            serde_json::json!({
                "type": "domain_record_version",
                "value": {
                    "record": {
                        "kind": {
                            "namespace": "fixture.information",
                            "name": "dispatch"
                        },
                        "id": "dispatch-7"
                    },
                    "version": 2,
                    "established_by": {
                        "type": "boundary_change",
                        "boundary": 12,
                        "change_index": 3
                    }
                }
            })
        );
    }
}
