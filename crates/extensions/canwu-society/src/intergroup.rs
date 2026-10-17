//! Intergroup relations: directed, typed, bounded relations between two
//! cohort classes in a region.
//!
//! A cohort class is the set of cohorts that carry one classification value
//! ([`crate::SocietyCohort::classification`]). An
//! [`IntergroupRelationEdge`] states how the source class regards the target
//! class within a region: acceptance, tension, and trust per mille, with the
//! evidence the application gave and a revision that counts its changes. The
//! edges live in their own record, `canwu.society:intergroup-relations`, so
//! a society settlement never rewrites them and a revision never rewrites the
//! society root.
//!
//! The relations are a host-readable state primitive: the society settlement
//! neither reads nor writes them, and no transition, mobilization, or
//! projection depends on them. A scenario authors them with the record
//! ([`IntergroupRelations::into_record`]); at runtime they change only
//! through the public [`INTERGROUP_RELATION_INGRESS`] packet, which the
//! society applies, all of it or none of it, in the boundary that admits it.

use crate::PLUGIN_NAME;
use crate::model::{SocietyState, invalid, validate_identifier, validate_per_mille};
use canwu_api::{
    CanwuError, DomainRecord, DomainRecordClass, DomainRecordDraft, DomainRecordLifecycle,
    DomainRecordType, DomainValueKindClass, EntityRef, EvidenceRef, IngressId, SimTime,
    TypedDomainRecordRef,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Public society ingress carrying one [`IntergroupRelationRevisionV1`].
pub const INTERGROUP_RELATION_INGRESS: &str = "intergroup_relation_revision_v1";
/// Event the society emits for an admitted relation revision it refuses. The
/// summary names the ingress and, when one change caused the refusal, that
/// relation, and ends with the rejection code; the boundary commits.
pub const INTERGROUP_RELATION_REJECTED_EVENT: &str = "society_intergroup_relation_rejected_v1";
/// Most edges one society holds.
pub const MAX_INTERGROUP_RELATIONS: usize = 4_096;
/// Most changes one [`IntergroupRelationRevisionV1`] carries.
pub const MAX_INTERGROUP_RELATION_CHANGES: usize = 1_024;
/// Most evidence references one edge carries.
pub const MAX_INTERGROUP_RELATION_EVIDENCE: usize = 16;
/// Most bytes of a [`CohortClass`] dimension or value.
pub const MAX_COHORT_CLASS_TEXT_BYTES: usize = 4_096;

/// Rejection code for a change whose `expected_revision` is not the edge's
/// current revision (zero while the edge does not exist).
pub const INTERGROUP_RELATION_STALE_REJECTION: &str = "stale_revision";
/// Rejection code for a change that would alter an edge's kind, classes, or
/// region, or create a second edge of one kind between the same classes in
/// the same region.
pub const INTERGROUP_RELATION_IDENTITY_REJECTION: &str = "identity_conflict";
/// Rejection code for a new edge whose kind is not in
/// [`IntergroupRelations::kinds`].
pub const INTERGROUP_RELATION_KIND_REJECTION: &str = "unknown_kind";
/// Rejection code for a new edge naming a class no cohort carries.
pub const INTERGROUP_RELATION_CLASS_REJECTION: &str = "unknown_class";
/// Rejection code for a new edge whose region is not a live territory, a
/// government, or an active domain entity.
pub const INTERGROUP_RELATION_REGION_REJECTION: &str = "unknown_region";
/// Rejection code for a revision that would hold more than
/// [`MAX_INTERGROUP_RELATIONS`] edges.
pub const INTERGROUP_RELATIONS_FULL_REJECTION: &str = "relations_full";

/// The record of a society's intergroup relations.
pub struct IntergroupRelationsRecord;

impl DomainRecordType for IntergroupRelationsRecord {
    type Payload = IntergroupRelations;
    type Class = DomainValueKindClass;

    const NAMESPACE: &'static str = "canwu.society";
    const NAME: &'static str = "intergroup-relations";
}

#[must_use]
pub fn intergroup_relations_reference() -> TypedDomainRecordRef<IntergroupRelationsRecord> {
    TypedDomainRecordRef::new("root")
}

/// The cohorts that carry one classification value: every cohort whose
/// `classification[dimension]` is `value`.
///
/// Both are the classification's authored text, compared exactly: any
/// non-empty text of at most [`MAX_COHORT_CLASS_TEXT_BYTES`] bytes, spaces
/// and non-ASCII text included, so a relation can name any class a cohort
/// was authored with (`canwu-culture` bounds its classification text at
/// 4,096 bytes by default too). The record's JSON encoding escapes what it
/// needs.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CohortClass {
    pub dimension: String,
    pub value: String,
}

impl CohortClass {
    #[must_use]
    pub fn new(dimension: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            dimension: dimension.into(),
            value: value.into(),
        }
    }

    fn validate(&self) -> Result<(), CanwuError> {
        for (text, label) in [
            (&self.dimension, "cohort class dimension"),
            (&self.value, "cohort class value"),
        ] {
            if text.is_empty() || text.len() > MAX_COHORT_CLASS_TEXT_BYTES {
                return Err(invalid(format!(
                    "{label} must be non-empty text of at most {MAX_COHORT_CLASS_TEXT_BYTES} bytes"
                )));
            }
        }
        Ok(())
    }
}

impl SocietyState {
    /// Every class some cohort carries, one per classification entry.
    #[must_use]
    pub fn cohort_classes(&self) -> BTreeSet<CohortClass> {
        cohort_classes(self.cohorts.values())
    }
}

pub(crate) fn cohort_classes<'a>(
    cohorts: impl IntoIterator<Item = &'a crate::SocietyCohort>,
) -> BTreeSet<CohortClass> {
    cohorts
        .into_iter()
        .flat_map(|cohort| &cohort.classification)
        .map(|(dimension, value)| CohortClass::new(dimension.clone(), value.clone()))
        .collect()
}

/// How the cohorts of one class regard those of another within a region.
///
/// The edge is directed: it is the source class's stance toward the target
/// class, so a relation whose two sides differ is two edges. Its kind, its
/// classes, and its region are its identity and never change; its values
/// and evidence change through revisions, each of which advances
/// `revision`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IntergroupRelationEdge {
    pub id: String,
    /// What the relation is about: a canonical identifier, and one of
    /// [`IntergroupRelations::kinds`] when that set is not empty.
    pub kind: String,
    /// The class whose stance the edge states.
    pub source_class: CohortClass,
    /// The class the stance is held toward; never the source class.
    pub target_class: CohortClass,
    /// Where the relation holds: a territory, a government (its
    /// jurisdiction), or a domain entity record an application uses for a
    /// region. Absent: wherever the two classes live.
    ///
    /// The region is an identifier, not a record reference. The society
    /// checks it when the edge is created (an existing territory or
    /// government, or an active domain entity record) and never again, so
    /// the region's owner stays free to retire or delete it. An edge whose
    /// region was retired or deleted since keeps it as recorded history:
    /// [`crate::load_intergroup_relations`] returns it as stored, a revision
    /// of the edge still applies (its identity is unchanged), and a new edge
    /// cannot name that region. A host that needs to know whether a region
    /// still exists asks the engine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<EntityRef>,
    pub acceptance_per_mille: u16,
    pub tension_per_mille: u16,
    pub trust_per_mille: u16,
    /// The application's evidence for the current values, as given; the
    /// society does not resolve it. At most
    /// [`MAX_INTERGROUP_RELATION_EVIDENCE`] references, without repeats.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<EvidenceRef>,
    /// One for an authored or newly created edge; each applied change adds
    /// one.
    pub revision: u64,
    /// The admitted packet that applied the latest revision; absent for an
    /// authored edge no packet has revised.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revised_by: Option<IngressId>,
    /// When `revised_by` was applied; present exactly with it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revised_at: Option<SimTime>,
}

impl IntergroupRelationEdge {
    fn identity(&self) -> RelationIdentity {
        (
            self.kind.clone(),
            self.source_class.clone(),
            self.target_class.clone(),
            self.region.clone(),
        )
    }
}

type RelationIdentity = (String, CohortClass, CohortClass, Option<EntityRef>);

/// Every intergroup relation of a society (schema version 1).
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IntergroupRelations {
    pub schema_version: u32,
    /// The kinds an edge may name. While it is empty any canonical
    /// identifier is admitted. It is authored with the record and never
    /// changes at runtime.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub kinds: BTreeSet<String>,
    /// Edges keyed by their ID.
    #[serde(default)]
    pub edges: BTreeMap<String, IntergroupRelationEdge>,
}

impl IntergroupRelations {
    pub const SCHEMA_VERSION: u32 = 1;

    /// No relations, at the current schema version.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            schema_version: Self::SCHEMA_VERSION,
            ..Self::default()
        }
    }

    /// Validates the relations on their own: identifiers, kinds, regions,
    /// bounds, revisions, and that no two edges share a kind, classes, and
    /// region. Whether some cohort carries each class is checked against the
    /// society root when a run is created or restored and when a revision
    /// creates an edge.
    ///
    /// # Errors
    ///
    /// Returns `InvalidDomainRecord` when an invariant is violated.
    pub fn validate(&self) -> Result<(), CanwuError> {
        if self.schema_version != Self::SCHEMA_VERSION {
            return Err(invalid("unsupported intergroup relations schema version"));
        }
        for kind in &self.kinds {
            validate_identifier(kind, "intergroup relation kind")?;
        }
        if self.edges.len() > MAX_INTERGROUP_RELATIONS {
            return Err(invalid(format!(
                "a society holds at most {MAX_INTERGROUP_RELATIONS} intergroup relations"
            )));
        }
        let mut identities = BTreeSet::new();
        for (key, edge) in &self.edges {
            if key != &edge.id {
                return Err(invalid(format!(
                    "intergroup relation key {key} does not match its stored identity {}",
                    edge.id
                )));
            }
            validate_identifier(&edge.id, "intergroup relation")?;
            self.validate_kind(&edge.kind)?;
            validate_values(
                &edge.id,
                &edge.source_class,
                &edge.target_class,
                edge.region.as_ref(),
                [
                    edge.acceptance_per_mille,
                    edge.tension_per_mille,
                    edge.trust_per_mille,
                ],
                &edge.evidence,
            )?;
            if edge.revision == 0 || edge.revised_by.is_some() != edge.revised_at.is_some() {
                return Err(invalid(format!(
                    "intergroup relation {} has an invalid revision",
                    edge.id
                )));
            }
            if !identities.insert(edge.identity()) {
                return Err(invalid(format!(
                    "intergroup relation {} repeats the kind, classes, and region of another",
                    edge.id
                )));
            }
        }
        Ok(())
    }

    /// Checks that some cohort carries each class an edge names.
    ///
    /// # Errors
    ///
    /// Returns `InvalidDomainRecord` naming the first edge whose class no
    /// cohort in `classes` carries.
    pub fn validate_classes(&self, classes: &BTreeSet<CohortClass>) -> Result<(), CanwuError> {
        for edge in self.edges.values() {
            if !classes.contains(&edge.source_class) || !classes.contains(&edge.target_class) {
                return Err(invalid(format!(
                    "intergroup relation {} names a class no cohort carries",
                    edge.id
                )));
            }
        }
        Ok(())
    }

    fn validate_kind(&self, kind: &str) -> Result<(), CanwuError> {
        validate_identifier(kind, "intergroup relation kind")?;
        if !self.kinds.is_empty() && !self.kinds.contains(kind) {
            return Err(invalid(format!(
                "intergroup relation kind {kind} is not in the relation kinds"
            )));
        }
        Ok(())
    }

    /// Encodes validated relations as the plugin-owned record a scenario
    /// authors beside the society root. The run's creation checks the
    /// classes against that root.
    ///
    /// # Errors
    ///
    /// Returns an error when the relations are invalid or cannot be encoded.
    pub fn into_record(self) -> Result<DomainRecord, CanwuError> {
        self.validate()?;
        let draft = self.record_draft()?;
        Ok(DomainRecord {
            reference: draft.reference,
            owner: PLUGIN_NAME.to_owned(),
            class: DomainRecordClass::Record,
            version: 1,
            lifecycle: DomainRecordLifecycle::Active,
            payload: draft.payload,
            references: draft.references,
        })
    }

    pub(crate) fn record_draft(&self) -> Result<DomainRecordDraft, CanwuError> {
        DomainRecordDraft::from_typed(intergroup_relations_reference(), self)
    }

    /// The record binds no references: a region is an identifier the
    /// society checks when an edge is created, never a reference that would
    /// stop the region's owner from retiring or deleting it.
    pub(crate) fn validate_record_binding(record: &DomainRecord) -> Result<(), CanwuError> {
        if !record.references.is_empty() {
            return Err(invalid(
                "the intergroup relations record binds no references; regions are identifiers",
            ));
        }
        Ok(())
    }

    /// Applies one revision, every change or none, and returns the rejection
    /// of a refused one.
    ///
    /// `classes` are the classes some cohort carries and `live_regions` the
    /// regions the boundary found live; both are checked only for an edge
    /// the revision creates, since an existing edge's identity never
    /// changes.
    pub(crate) fn apply_revision(
        &mut self,
        revision: &IntergroupRelationRevisionV1,
        classes: &BTreeSet<CohortClass>,
        live_regions: &BTreeSet<EntityRef>,
        ingress: IngressId,
        at: SimTime,
    ) -> Result<(), RelationRejection> {
        let mut identities = self
            .edges
            .values()
            .map(IntergroupRelationEdge::identity)
            .collect::<BTreeSet<_>>();
        let mut created = 0_usize;
        let mut revised = Vec::with_capacity(revision.changes.len());
        for change in &revision.changes {
            let reject = |reason| RelationRejection {
                reason,
                relation: Some(change.id.clone()),
            };
            let current = self.edges.get(&change.id);
            // An edge at the last revision cannot be revised again; that is a
            // refusal like any other stale expectation, never an overflow.
            let next_revision = change.expected_revision.checked_add(1);
            if change.expected_revision != current.map_or(0, |edge| edge.revision) {
                return Err(reject(INTERGROUP_RELATION_STALE_REJECTION));
            }
            let Some(next_revision) = next_revision else {
                return Err(reject(INTERGROUP_RELATION_STALE_REJECTION));
            };
            let edge = IntergroupRelationEdge {
                id: change.id.clone(),
                kind: change.kind.clone(),
                source_class: change.source_class.clone(),
                target_class: change.target_class.clone(),
                region: change.region.clone(),
                acceptance_per_mille: change.acceptance_per_mille,
                tension_per_mille: change.tension_per_mille,
                trust_per_mille: change.trust_per_mille,
                evidence: change.evidence.clone(),
                revision: next_revision,
                revised_by: Some(ingress),
                revised_at: Some(at),
            };
            match current {
                Some(current) if current.identity() != edge.identity() => {
                    return Err(reject(INTERGROUP_RELATION_IDENTITY_REJECTION));
                }
                Some(_) => {}
                None => {
                    if self.validate_kind(&edge.kind).is_err() {
                        return Err(reject(INTERGROUP_RELATION_KIND_REJECTION));
                    }
                    if !classes.contains(&edge.source_class)
                        || !classes.contains(&edge.target_class)
                    {
                        return Err(reject(INTERGROUP_RELATION_CLASS_REJECTION));
                    }
                    if edge
                        .region
                        .as_ref()
                        .is_some_and(|region| !live_regions.contains(region))
                    {
                        return Err(reject(INTERGROUP_RELATION_REGION_REJECTION));
                    }
                    if !identities.insert(edge.identity()) {
                        return Err(reject(INTERGROUP_RELATION_IDENTITY_REJECTION));
                    }
                    created += 1;
                }
            }
            revised.push(edge);
        }
        if self.edges.len() + created > MAX_INTERGROUP_RELATIONS {
            return Err(RelationRejection {
                reason: INTERGROUP_RELATIONS_FULL_REJECTION,
                relation: None,
            });
        }
        for edge in revised {
            self.edges.insert(edge.id.clone(), edge);
        }
        Ok(())
    }
}

/// Why the society refused an admitted relation revision.
pub(crate) struct RelationRejection {
    pub(crate) reason: &'static str,
    /// The change that caused it, when one did.
    pub(crate) relation: Option<String>,
}

/// One change to one intergroup relation.
///
/// It creates the edge when `expected_revision` is zero and revises it
/// otherwise. It states the edge's identity (kind, classes, region) and its
/// new values and evidence in full; a revision replaces the values and the
/// evidence and must restate the identity unchanged.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IntergroupRelationChange {
    pub id: String,
    /// The edge's current revision, or zero to create it.
    pub expected_revision: u64,
    pub kind: String,
    pub source_class: CohortClass,
    pub target_class: CohortClass,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<EntityRef>,
    pub acceptance_per_mille: u16,
    pub tension_per_mille: u16,
    pub trust_per_mille: u16,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<EvidenceRef>,
}

/// One revision of a society's intergroup relations: the payload of the
/// public [`INTERGROUP_RELATION_INGRESS`] packet.
///
/// The society applies it in the boundary that admits it, in admission order
/// with the other revisions of that boundary, before any later phase reads
/// the relations. It applies as a whole or not at all: a refused revision
/// changes nothing and is reported by one
/// [`INTERGROUP_RELATION_REJECTED_EVENT`]; the boundary commits.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IntergroupRelationRevisionV1 {
    /// Between one and [`MAX_INTERGROUP_RELATION_CHANGES`] changes, in
    /// strictly ascending ID order.
    pub changes: Vec<IntergroupRelationChange>,
}

impl IntergroupRelationRevisionV1 {
    /// Validates the revision's shape without reading society state.
    ///
    /// # Errors
    ///
    /// Returns `InvalidDomainRecord` when the revision carries no change or
    /// more than [`MAX_INTERGROUP_RELATION_CHANGES`], its IDs are not
    /// canonical and strictly ascending, a kind is not canonical, a class
    /// text is empty or longer than [`MAX_COHORT_CLASS_TEXT_BYTES`], the two
    /// classes are equal, a value exceeds 1,000 per mille, or the
    /// evidence repeats or exceeds [`MAX_INTERGROUP_RELATION_EVIDENCE`].
    pub fn validate(&self) -> Result<(), CanwuError> {
        if self.changes.is_empty() || self.changes.len() > MAX_INTERGROUP_RELATION_CHANGES {
            return Err(invalid(format!(
                "an intergroup relation revision carries between 1 and \
                 {MAX_INTERGROUP_RELATION_CHANGES} changes"
            )));
        }
        if self.changes.windows(2).any(|pair| pair[0].id >= pair[1].id) {
            return Err(invalid(
                "intergroup relation changes are not in strictly ascending ID order",
            ));
        }
        for change in &self.changes {
            validate_identifier(&change.id, "intergroup relation")?;
            validate_identifier(&change.kind, "intergroup relation kind")?;
            validate_values(
                &change.id,
                &change.source_class,
                &change.target_class,
                None,
                [
                    change.acceptance_per_mille,
                    change.tension_per_mille,
                    change.trust_per_mille,
                ],
                &change.evidence,
            )?;
        }
        Ok(())
    }

    /// Every region the revision names.
    pub(crate) fn regions(&self) -> BTreeSet<EntityRef> {
        self.changes
            .iter()
            .filter_map(|change| change.region.clone())
            .collect()
    }
}

fn validate_values(
    id: &str,
    source: &CohortClass,
    target: &CohortClass,
    region: Option<&EntityRef>,
    values: [u16; 3],
    evidence: &[EvidenceRef],
) -> Result<(), CanwuError> {
    source.validate()?;
    target.validate()?;
    if source == target {
        return Err(invalid(format!(
            "intergroup relation {id} relates a class to itself"
        )));
    }
    if region.is_some_and(|region| !is_region_kind(region)) {
        return Err(invalid(format!(
            "intergroup relation {id} region must be a territory, a government, or a domain entity"
        )));
    }
    for (value, label) in values.into_iter().zip([
        "intergroup acceptance",
        "intergroup tension",
        "intergroup trust",
    ]) {
        validate_per_mille(value, label)?;
    }
    if evidence.len() > MAX_INTERGROUP_RELATION_EVIDENCE
        || evidence.iter().collect::<BTreeSet<_>>().len() != evidence.len()
    {
        return Err(invalid(format!(
            "intergroup relation {id} carries more than {MAX_INTERGROUP_RELATION_EVIDENCE} \
             evidence references or repeats one"
        )));
    }
    Ok(())
}

/// Whether an entity kind can name a region: a territory, a government, or a
/// domain entity.
pub(crate) const fn is_region_kind(region: &EntityRef) -> bool {
    matches!(
        region,
        EntityRef::Territory(_) | EntityRef::Government(_) | EntityRef::Domain(_)
    )
}
