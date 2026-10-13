use super::records::{DomainRecordClass, DomainRecordSchemas};
use super::{
    CanwuError, ErrorCode, PayloadSchema, PersistentDomainRecordStore, RuntimeCurrentState,
    SimulationSnapshot, canonical_text, component_key, is_canonical_hash,
};
use canwu_core::{
    CoreEntityKind, DomainRecordKind, EntityRef, KnowledgeHolderPolicy, KnowledgeHolderRef,
    KnowledgeRecordId, KnowledgeRecordKind, KnowledgeSchemaId,
};
use canwu_knowledge::{
    DEFAULT_KNOWLEDGE_PAGE_SIZE, KnowledgeRecord, KnowledgeRecordDraft, KnowledgeSubject,
    KnowledgeSubjectTarget, MAX_KNOWLEDGE_PAGE_SIZE,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KnowledgeLimitsV1 {
    pub schemas_per_plugin: usize,
    pub records_per_batch: usize,
    pub batches_per_system_boundary: usize,
    pub records_per_boundary: usize,
    pub payload_bytes_per_record: usize,
    pub relations_per_record: usize,
    pub text_bytes: usize,
    pub max_page_size: u32,
    pub queries_per_batch: usize,
    pub ids_per_direct_get: usize,
    pub default_page_size: u32,
    pub relation_trace_depth: u32,
    pub relation_graph_records: usize,
}

impl KnowledgeLimitsV1 {
    pub const CURRENT: Self = Self {
        schemas_per_plugin: 256,
        records_per_batch: 1_000,
        batches_per_system_boundary: 64,
        records_per_boundary: 10_000,
        payload_bytes_per_record: 65_536,
        relations_per_record: 64,
        text_bytes: 1_024,
        max_page_size: MAX_KNOWLEDGE_PAGE_SIZE,
        queries_per_batch: 64,
        ids_per_direct_get: 1_000,
        default_page_size: DEFAULT_KNOWLEDGE_PAGE_SIZE,
        relation_trace_depth: 32,
        relation_graph_records: 10_000,
    };
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "type", content = "kind", rename_all = "snake_case")]
pub enum KnowledgeSubjectTargetKind {
    Core(CoreEntityKind),
    Domain(DomainRecordKind),
    AnyEntity,
    Event,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KnowledgeSubjectSchema {
    pub role: String,
    pub targets: Vec<KnowledgeSubjectTargetKind>,
    pub required: bool,
    pub multiple: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PluginKnowledgeSchema {
    pub id: KnowledgeSchemaId,
    pub schema_hash: String,
    pub writable: bool,
    pub payload_schema: PayloadSchema,
    pub subjects: Vec<KnowledgeSubjectSchema>,
}

impl PluginKnowledgeSchema {
    pub(crate) fn canonicalize(&mut self) {
        for subject in &mut self.subjects {
            subject.targets.sort();
            subject.targets.dedup();
        }
        self.subjects
            .sort_by(|left, right| left.role.cmp(&right.role));
    }

    pub(crate) fn validate(&self) -> Result<(), CanwuError> {
        validate_kind(&self.id.kind)?;
        if self.id.version == 0 {
            return Err(invalid_schema("knowledge schema versions must be nonzero"));
        }
        if !is_canonical_hash(&self.schema_hash) {
            return Err(invalid_schema(
                "knowledge schemas require a canonical semantic hash",
            ));
        }
        if self
            .subjects
            .windows(2)
            .any(|pair| pair[0].role >= pair[1].role)
        {
            return Err(invalid_schema(
                "knowledge subject roles must be unique and sorted",
            ));
        }
        for subject in &self.subjects {
            if !canonical_text(&subject.role)
                || subject.targets.is_empty()
                || subject.targets.windows(2).any(|pair| pair[0] >= pair[1])
            {
                return Err(invalid_schema(
                    "knowledge subjects require canonical roles and unique sorted targets",
                ));
            }
            for target in &subject.targets {
                if let KnowledgeSubjectTargetKind::Domain(kind) = target {
                    validate_domain_kind(kind)?;
                }
            }
        }
        Ok(())
    }
}

pub(crate) type KnowledgeSchemas = BTreeMap<KnowledgeSchemaId, (String, PluginKnowledgeSchema)>;
pub(crate) type KnowledgeKindOwners = BTreeMap<KnowledgeRecordKind, String>;

/// Knowledge records keyed by holder, then by global record ID.
pub(crate) type HolderKnowledgeRecords =
    BTreeMap<KnowledgeHolderRef, BTreeMap<KnowledgeRecordId, KnowledgeRecord>>;

/// Why a record may not supersede or contradict a named target.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum KnowledgeRelationError {
    /// The target is not in the holder's ledger at the publisher's cut.
    Unresolved,
    /// The target has a different schema kind from the record naming it.
    CrossKind,
}

/// The one rule for the records a published record may supersede or
/// contradict, shared by settlement and by snapshot validation.
///
/// Every target must be a record of the same `holder`, of the same schema
/// `kind`, that is visible at the publisher's cut:
///
/// - `committed`: the ledger committed by earlier boundaries, whatever the
///   target's learned time, so a target learned at the same instant in an
///   earlier boundary resolves; or
/// - `visible`: the records the settling boundary staged with same-boundary
///   visibility in a phase before the publisher's.
///
/// A record of the publisher's own phase, a record staged for the next
/// boundary, a record that does not exist yet, and a record of another holder
/// are in neither set. Record IDs are allocated in staging order, so every
/// admitted target has a lower ID than the record naming it: a relation can
/// never point forward, at itself, or around a cycle.
pub(crate) fn validate_relation_targets<'a>(
    holder: &KnowledgeHolderRef,
    kind: &KnowledgeRecordKind,
    related: impl IntoIterator<Item = &'a KnowledgeRecordId>,
    committed: &HolderKnowledgeRecords,
    visible: &HolderKnowledgeRecords,
) -> Result<(), KnowledgeRelationError> {
    let committed = committed.get(holder);
    let visible = visible.get(holder);
    for id in related {
        let Some(target) = committed
            .and_then(|records| records.get(id))
            .or_else(|| visible.and_then(|records| records.get(id)))
        else {
            return Err(KnowledgeRelationError::Unresolved);
        };
        if target.schema.kind != *kind {
            return Err(KnowledgeRelationError::CrossKind);
        }
    }
    Ok(())
}

/// A holder's current records of one schema kind at a read cut, as returned
/// by [`crate::SimulationView::knowledge_heads`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KnowledgeHeads {
    /// Global record IDs in ascending order. When more than
    /// [`KnowledgeLimitsV1::relations_per_record`] records match, the number
    /// one record may supersede, these are the newest of them.
    pub records: Vec<KnowledgeRecordId>,
    /// Whether more current records matched than `records` holds. The older
    /// ones that were left out stay current.
    pub truncated: bool,
}

/// Returns the holder's records of `kind` that carry every one of `subjects`
/// and that no record of the holder supersedes, over the same two sets the
/// relation rule reads: the ledger `committed` by earlier boundaries and the
/// records `visible` at the reader's cut. Every returned ID is therefore a
/// target [`validate_relation_targets`] admits for the same holder and kind.
///
/// The newest `limit` matches are returned, in ascending ID order. Records of
/// the settling boundary have higher IDs than committed ones, so both sets
/// are walked from the highest ID down and the result is reversed.
///
/// The holder's records are read in place, twice: once to collect the IDs
/// that records of `kind` supersede (a relation never crosses kinds, so no
/// other record can supersede one of them), and once to select. The second
/// pass stops as soon as it has `limit + 1` matches.
pub(crate) fn current_heads_at_cut(
    holder: &KnowledgeHolderRef,
    kind: &KnowledgeRecordKind,
    subjects: &[KnowledgeSubject],
    committed: &HolderKnowledgeRecords,
    visible: Option<&HolderKnowledgeRecords>,
    limit: usize,
) -> KnowledgeHeads {
    let ledgers = [
        visible.and_then(|records| records.get(holder)),
        committed.get(holder),
    ];
    let newest_first_of_kind = || {
        ledgers
            .iter()
            .flatten()
            .flat_map(|ledger| ledger.values().rev())
            .filter(|record| record.schema.kind == *kind)
    };
    let superseded: std::collections::BTreeSet<KnowledgeRecordId> = newest_first_of_kind()
        .flat_map(|record| record.supersedes.iter().copied())
        .collect();
    let mut heads: Vec<KnowledgeRecordId> = newest_first_of_kind()
        .filter(|record| {
            !superseded.contains(&record.id)
                && subjects
                    .iter()
                    .all(|subject| record.subjects.contains(subject))
        })
        .map(|record| record.id)
        .take(limit.saturating_add(1))
        .collect();
    let truncated = heads.len() > limit;
    heads.truncate(limit);
    heads.reverse();
    KnowledgeHeads {
        records: heads,
        truncated,
    }
}

pub(crate) fn validate_schema_set(
    schemas: &KnowledgeSchemas,
    owners: &KnowledgeKindOwners,
) -> Result<(), CanwuError> {
    let mut writable = BTreeMap::<KnowledgeRecordKind, usize>::new();
    for (id, (owner, schema)) in schemas {
        if id != &schema.id || owners.get(&id.kind) != Some(owner) {
            return Err(invalid_schema(
                "knowledge schema registry ownership is inconsistent",
            ));
        }
        schema.validate()?;
        if schema.writable {
            *writable.entry(id.kind.clone()).or_default() += 1;
        }
    }
    if owners.keys().any(|kind| {
        !schemas.keys().any(|schema| &schema.kind == kind)
            || writable.get(kind).copied().unwrap_or_default() != 1
    }) {
        return Err(invalid_schema(
            "each knowledge kind requires exactly one writable version",
        ));
    }
    Ok(())
}

pub(crate) fn validate_draft(
    draft: &KnowledgeRecordDraft,
    schema: &PluginKnowledgeSchema,
    holder: &KnowledgeHolderRef,
    current: &RuntimeCurrentState,
    record_schemas: &DomainRecordSchemas,
) -> Result<(), CanwuError> {
    validate_record_shape(draft, schema, holder, current, record_schemas, true)
}

fn validate_record_shape(
    draft: &KnowledgeRecordDraft,
    schema: &PluginKnowledgeSchema,
    holder: &KnowledgeHolderRef,
    current: &RuntimeCurrentState,
    record_schemas: &DomainRecordSchemas,
    require_writable: bool,
) -> Result<(), CanwuError> {
    if draft.schema != schema.id {
        return Err(invalid_knowledge(
            "knowledge draft names the wrong schema version",
        ));
    }
    if require_writable && !schema.writable {
        return Err(invalid_knowledge(
            "knowledge draft uses a read-only schema version",
        ));
    }
    if require_writable {
        validate_holder_for_publication(holder, current, record_schemas)?;
    } else {
        validate_historical_holder(holder, current, record_schemas)?;
    }
    if draft.confidence_per_mille > 1_000 {
        return Err(invalid_knowledge(
            "knowledge confidence exceeds 1000 per mille",
        ));
    }
    if !canonical_text(&draft.origin.method)
        || draft.origin.method.len() > KnowledgeLimitsV1::CURRENT.text_bytes
    {
        return Err(invalid_knowledge(
            "knowledge origin method is not canonical or exceeds its limit",
        ));
    }
    if draft.subjects.len() > KnowledgeLimitsV1::CURRENT.relations_per_record
        || draft.origin.evidence.len() > KnowledgeLimitsV1::CURRENT.relations_per_record
        || draft.supersedes.len() > KnowledgeLimitsV1::CURRENT.relations_per_record
        || draft.contradicts.len() > KnowledgeLimitsV1::CURRENT.relations_per_record
    {
        return Err(invalid_knowledge(
            "knowledge record relation limit exceeded",
        ));
    }
    if !strictly_sorted(&draft.subjects) || !strictly_sorted(&draft.origin.evidence) {
        return Err(invalid_knowledge(
            "knowledge subjects and evidence references must be sorted and unique",
        ));
    }
    let payload_bytes = serde_json::to_vec(&draft.payload).map_err(|error| {
        invalid_knowledge(format!("knowledge payload encoding failed: {error}"))
    })?;
    if payload_bytes.len() > KnowledgeLimitsV1::CURRENT.payload_bytes_per_record {
        return Err(invalid_knowledge("knowledge payload byte limit exceeded"));
    }
    schema.payload_schema.validate(&draft.payload)?;
    validate_subjects(&draft.subjects, schema, current, record_schemas)?;
    validate_relations(draft)?;
    Ok(())
}

pub(crate) fn validate_stored_record(
    record: &KnowledgeRecord,
    schemas: &KnowledgeSchemas,
    current: &RuntimeCurrentState,
    record_schemas: &DomainRecordSchemas,
) -> Result<(), CanwuError> {
    let Some((_, schema)) = schemas.get(&record.schema) else {
        return Err(invalid_knowledge(
            "knowledge record uses an unregistered schema",
        ));
    };
    let draft = KnowledgeRecordDraft {
        schema: record.schema.clone(),
        subjects: record.subjects.clone(),
        payload: record.payload.clone(),
        as_of: record.as_of,
        confidence_per_mille: record.confidence_per_mille,
        origin: record.origin.clone(),
        supersedes: record.supersedes.clone(),
        contradicts: record.contradicts.clone(),
    };
    validate_record_shape(
        &draft,
        schema,
        &record.holder,
        current,
        record_schemas,
        false,
    )
}

pub(crate) fn validate_snapshot_records(
    snapshot: &SimulationSnapshot,
    schemas: &KnowledgeSchemas,
    record_schemas: &DomainRecordSchemas,
) -> Result<u64, CanwuError> {
    let current = RuntimeCurrentState {
        entities: snapshot.entities.iter().cloned().collect(),
        people: snapshot
            .world
            .people
            .iter()
            .cloned()
            .map(|value| (value.id, value))
            .collect(),
        person_availability: snapshot.person_availability.clone(),
        created_persons: snapshot.created_persons.clone(),
        letters: snapshot
            .world
            .letters
            .iter()
            .cloned()
            .map(|value| (value.id, value))
            .collect(),
        governments: snapshot
            .world
            .governments
            .iter()
            .cloned()
            .map(|value| (value.id, value))
            .collect(),
        territories: snapshot
            .world
            .territories
            .iter()
            .cloned()
            .map(|value| (value.id, value))
            .collect(),
        routes: snapshot
            .world
            .routes
            .iter()
            .cloned()
            .map(|value| (value.id, value))
            .collect(),
        armies: snapshot
            .world
            .armies
            .iter()
            .cloned()
            .map(|value| (value.id, value))
            .collect(),
        knowledge: snapshot.knowledge.clone(),
        plugin_components: snapshot
            .plugin_components
            .iter()
            .cloned()
            .map(|record| {
                (
                    component_key(
                        &record.plugin,
                        &record.state,
                        &record.entity,
                        &record.component,
                    ),
                    record,
                )
            })
            .collect(),
        domain_records: PersistentDomainRecordStore::from_records(
            snapshot
                .domain_records
                .iter()
                .cloned()
                .map(|record| (record.reference.clone(), record))
                .collect(),
        )?,
        decisions: snapshot.decisions.clone(),
        root_seed: snapshot.root_seed,
        authority_root_seed: snapshot.authority_root_seed,
        random_streams: snapshot
            .random_streams
            .iter()
            .cloned()
            .map(|stream| (stream.key.clone(), stream))
            .collect(),
    };
    let mut global_ids = std::collections::BTreeSet::new();
    let mut max_id = 0_u64;
    for (holder, records) in &snapshot.knowledge.records {
        for (id, record) in records {
            if *id != record.id || &record.holder != holder || !global_ids.insert(*id) {
                return Err(invalid_knowledge(
                    "knowledge ledger keys, holders, and global IDs must be consistent and unique",
                ));
            }
            if record.learned_at > snapshot.now {
                return Err(invalid_knowledge(
                    "knowledge records cannot be learned after the snapshot time",
                ));
            }
            if record
                .supersedes
                .iter()
                .chain(&record.contradicts)
                .any(|related| !records.contains_key(related))
            {
                return Err(invalid_knowledge(
                    "knowledge relations must resolve within the same holder ledger",
                ));
            }
            validate_stored_record(record, schemas, &current, record_schemas)?;
            max_id = max_id.max(id.get());
        }
    }
    Ok(max_id)
}

pub(crate) fn validate_holder_for_publication(
    holder: &KnowledgeHolderRef,
    current: &RuntimeCurrentState,
    record_schemas: &DomainRecordSchemas,
) -> Result<(), CanwuError> {
    let allowed = match holder {
        KnowledgeHolderRef::Person(id) => {
            current.entities.contains(&EntityRef::Person(*id)) || current.people.contains_key(id)
        }
        KnowledgeHolderRef::Entity(EntityRef::Army(id)) => {
            current.entities.contains(&EntityRef::Army(*id)) || current.armies.contains_key(id)
        }
        KnowledgeHolderRef::Entity(EntityRef::Government(id)) => {
            current.entities.contains(&EntityRef::Government(*id))
                || current.governments.contains_key(id)
        }
        KnowledgeHolderRef::Entity(
            EntityRef::Organization(_)
            | EntityRef::Person(_)
            | EntityRef::Resource(_)
            | EntityRef::Route(_)
            | EntityRef::Territory(_),
        ) => false,
        KnowledgeHolderRef::Entity(EntityRef::Domain(reference)) => {
            current.domain_records.get(reference).is_some_and(|record| {
                !record.is_deleted()
                    && record.is_active()
                    && record.class == DomainRecordClass::Entity
                    && record_schemas
                        .get(&reference.kind)
                        .is_some_and(|(_, schema)| {
                            schema.holder_policy == KnowledgeHolderPolicy::Allowed
                        })
            })
        }
    };
    if !allowed {
        return Err(CanwuError::new(
            ErrorCode::InvalidKnowledgeHolder,
            "knowledge holder is missing, retired, deleted, duplicated as a person entity, or ineligible",
        ));
    }
    if let KnowledgeHolderRef::Person(id) = holder
        && current
            .person_availability
            .get(id)
            .is_some_and(|availability| availability.life == super::LifeState::Dead)
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidKnowledgeHolder,
            "knowledge cannot be published to a dead person's holder ledger",
        ));
    }
    Ok(())
}

fn validate_historical_holder(
    holder: &KnowledgeHolderRef,
    current: &RuntimeCurrentState,
    record_schemas: &DomainRecordSchemas,
) -> Result<(), CanwuError> {
    let allowed = match holder {
        KnowledgeHolderRef::Person(id) => {
            current.entities.contains(&EntityRef::Person(*id)) || current.people.contains_key(id)
        }
        KnowledgeHolderRef::Entity(EntityRef::Army(id)) => {
            current.entities.contains(&EntityRef::Army(*id)) || current.armies.contains_key(id)
        }
        KnowledgeHolderRef::Entity(EntityRef::Government(id)) => {
            current.entities.contains(&EntityRef::Government(*id))
                || current.governments.contains_key(id)
        }
        KnowledgeHolderRef::Entity(
            EntityRef::Organization(_)
            | EntityRef::Person(_)
            | EntityRef::Resource(_)
            | EntityRef::Route(_)
            | EntityRef::Territory(_),
        ) => false,
        KnowledgeHolderRef::Entity(EntityRef::Domain(reference)) => {
            current.domain_records.get(reference).is_some_and(|record| {
                record.class == DomainRecordClass::Entity
                    && record_schemas
                        .get(&reference.kind)
                        .is_some_and(|(_, schema)| {
                            schema.holder_policy == KnowledgeHolderPolicy::Allowed
                        })
            })
        }
    };
    if !allowed {
        return Err(CanwuError::new(
            ErrorCode::InvalidKnowledgeHolder,
            "knowledge record names an unknown or ineligible historical holder",
        ));
    }
    Ok(())
}

fn validate_subjects(
    subjects: &[KnowledgeSubject],
    schema: &PluginKnowledgeSchema,
    current: &RuntimeCurrentState,
    record_schemas: &DomainRecordSchemas,
) -> Result<(), CanwuError> {
    let mut counts = BTreeMap::<&str, usize>::new();
    for subject in subjects {
        if !canonical_text(&subject.role) {
            return Err(invalid_knowledge(
                "knowledge subject roles must be canonical",
            ));
        }
        let Some(declaration) = schema
            .subjects
            .iter()
            .find(|item| item.role == subject.role)
        else {
            return Err(invalid_knowledge(
                "knowledge record contains an undeclared subject role",
            ));
        };
        if !subject_matches(subject, declaration, current, record_schemas) {
            return Err(invalid_knowledge(
                "knowledge subject target does not match its schema role",
            ));
        }
        let count = counts.entry(&subject.role).or_default();
        *count += 1;
        if !declaration.multiple && *count > 1 {
            return Err(invalid_knowledge(
                "knowledge singleton subject role is repeated",
            ));
        }
    }
    if schema.subjects.iter().any(|declaration| {
        declaration.required
            && counts
                .get(declaration.role.as_str())
                .copied()
                .unwrap_or_default()
                == 0
    }) {
        return Err(invalid_knowledge(
            "knowledge record is missing a required subject role",
        ));
    }
    Ok(())
}

fn subject_matches(
    subject: &KnowledgeSubject,
    schema: &KnowledgeSubjectSchema,
    current: &RuntimeCurrentState,
    _record_schemas: &DomainRecordSchemas,
) -> bool {
    match &subject.target {
        KnowledgeSubjectTarget::Event(_) => {
            schema.targets.contains(&KnowledgeSubjectTargetKind::Event)
        }
        KnowledgeSubjectTarget::Entity(entity) => {
            let exact = entity.core_kind().is_some_and(|kind| {
                schema
                    .targets
                    .contains(&KnowledgeSubjectTargetKind::Core(kind))
            }) && entity_exists(current, entity);
            let any = schema
                .targets
                .contains(&KnowledgeSubjectTargetKind::AnyEntity)
                && entity_exists(current, entity);
            exact || any
        }
        KnowledgeSubjectTarget::DomainRecord(reference) => {
            let exact = schema
                .targets
                .contains(&KnowledgeSubjectTargetKind::Domain(reference.kind.clone()));
            let any = schema
                .targets
                .contains(&KnowledgeSubjectTargetKind::AnyEntity)
                && current.domain_records.get(reference).is_some_and(|record| {
                    !record.is_deleted() && record.class == DomainRecordClass::Entity
                });
            (exact || any) && current.domain_records.contains_key(reference)
        }
    }
}

fn entity_exists(current: &RuntimeCurrentState, entity: &EntityRef) -> bool {
    match entity {
        EntityRef::Army(id) => current.entities.contains(entity) || current.armies.contains_key(id),
        EntityRef::Government(id) => {
            current.entities.contains(entity) || current.governments.contains_key(id)
        }
        EntityRef::Organization(_) | EntityRef::Resource(_) => current.entities.contains(entity),
        EntityRef::Person(id) => {
            current.entities.contains(entity) || current.people.contains_key(id)
        }
        EntityRef::Domain(reference) => {
            current.domain_records.get(reference).is_some_and(|record| {
                !record.is_deleted() && record.class == DomainRecordClass::Entity
            })
        }
        EntityRef::Route(id) => {
            current.entities.contains(entity) || current.routes.contains_key(id)
        }
        EntityRef::Territory(id) => {
            current.entities.contains(entity) || current.territories.contains_key(id)
        }
    }
}

fn validate_relations(draft: &KnowledgeRecordDraft) -> Result<(), CanwuError> {
    if !strictly_sorted(&draft.supersedes)
        || !strictly_sorted(&draft.contradicts)
        || draft
            .supersedes
            .iter()
            .any(|id| draft.contradicts.contains(id))
    {
        return Err(invalid_knowledge(
            "knowledge supersedes and contradicts relations must be sorted, unique, and disjoint",
        ));
    }
    Ok(())
}

fn strictly_sorted<T: Ord>(values: &[T]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

fn validate_kind(kind: &KnowledgeRecordKind) -> Result<(), CanwuError> {
    if !canonical_text(&kind.namespace) || !canonical_text(&kind.name) {
        return Err(invalid_schema("knowledge schema kind must be canonical"));
    }
    Ok(())
}

fn validate_domain_kind(kind: &DomainRecordKind) -> Result<(), CanwuError> {
    if !canonical_text(&kind.namespace) || !canonical_text(&kind.name) {
        return Err(invalid_schema(
            "knowledge subject domain kind must be canonical",
        ));
    }
    Ok(())
}

fn invalid_schema(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidKnowledgeSchema, message)
}

fn invalid_knowledge(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidKnowledgeRecord, message)
}

#[cfg(test)]
mod tests {
    use super::super::records::{DomainRecord, DomainRecordLifecycle, DomainRecordSchema};
    use super::*;
    use crate::DecisionState;
    use crate::runtime::{Government, MapPoint, Route, Territory};
    use canwu_core::{
        DomainRecordRef, EventId, EvidenceRef, GovernmentId, KnowledgeRecordId, PersonId,
        ResourceId, RouteId, TerritoryId,
    };
    use canwu_knowledge::{KnowledgeOrigin, KnowledgeSnapshot};
    use canwu_time::SimTime;
    use serde_json::json;
    use std::collections::{BTreeMap, BTreeSet};

    fn knowledge_kind() -> KnowledgeRecordKind {
        KnowledgeRecordKind::new("fixture.knowledge", "assessment")
    }

    fn knowledge_schema(subjects: Vec<KnowledgeSubjectSchema>) -> PluginKnowledgeSchema {
        PluginKnowledgeSchema {
            id: KnowledgeSchemaId::new(knowledge_kind(), 1),
            schema_hash: "1000000000000000000000000000000000000000000000000000000000000000"
                .to_owned(),
            writable: true,
            payload_schema: PayloadSchema::Any,
            subjects,
        }
    }

    fn current_state(domain_records: Vec<DomainRecord>) -> RuntimeCurrentState {
        RuntimeCurrentState {
            entities: BTreeSet::new(),
            people: BTreeMap::new(),
            person_availability: BTreeMap::new(),
            created_persons: Vec::new(),
            letters: BTreeMap::new(),
            governments: BTreeMap::new(),
            territories: BTreeMap::new(),
            routes: BTreeMap::new(),
            armies: BTreeMap::new(),
            knowledge: KnowledgeSnapshot::default(),
            plugin_components: BTreeMap::new(),
            domain_records: PersistentDomainRecordStore::from_records(
                domain_records
                    .into_iter()
                    .map(|record| (record.reference.clone(), record))
                    .collect(),
            )
            .unwrap(),
            decisions: DecisionState::default(),
            root_seed: 1,
            authority_root_seed: 1,
            random_streams: BTreeMap::new(),
        }
    }

    fn domain_record(
        reference: DomainRecordRef,
        class: DomainRecordClass,
        lifecycle: DomainRecordLifecycle,
    ) -> DomainRecord {
        DomainRecord {
            reference,
            owner: "fixture-domain".to_owned(),
            class,
            version: 1,
            lifecycle,
            payload: json!(null),
            references: vec![],
        }
    }

    fn holder_schemas(
        kind: &DomainRecordKind,
        policy: KnowledgeHolderPolicy,
    ) -> DomainRecordSchemas {
        let mut schema = DomainRecordSchema::new(kind.clone(), DomainRecordClass::Entity);
        schema.holder_policy = policy;
        BTreeMap::from([(kind.clone(), ("fixture-domain".to_owned(), schema))])
    }

    fn base_draft() -> KnowledgeRecordDraft {
        KnowledgeRecordDraft {
            schema: KnowledgeSchemaId::new(knowledge_kind(), 1),
            subjects: vec![],
            payload: json!(null),
            as_of: None,
            confidence_per_mille: 1_000,
            origin: KnowledgeOrigin {
                method: "fixture".to_owned(),
                evidence: vec![],
            },
            supersedes: vec![],
            contradicts: vec![],
        }
    }

    fn government_current() -> (KnowledgeHolderRef, RuntimeCurrentState) {
        let government = Government {
            id: GovernmentId::new(1),
            name: "Fixture".to_owned(),
            capital: TerritoryId::new(1),
        };
        let mut current = current_state(vec![]);
        current.governments.insert(government.id, government);
        (
            KnowledgeHolderRef::Entity(EntityRef::Government(GovernmentId::new(1))),
            current,
        )
    }

    #[test]
    fn holder_eligibility_distinguishes_publication_from_retained_history() {
        let kind = DomainRecordKind::new("fixture.organization", "office");
        let active_ref = DomainRecordRef::new(&kind.namespace, &kind.name, "active");
        let retired_ref = DomainRecordRef::new(&kind.namespace, &kind.name, "retired");
        let deleted_ref = DomainRecordRef::new(&kind.namespace, &kind.name, "deleted");
        let current = current_state(vec![
            domain_record(
                active_ref.clone(),
                DomainRecordClass::Entity,
                DomainRecordLifecycle::Active,
            ),
            domain_record(
                retired_ref.clone(),
                DomainRecordClass::Entity,
                DomainRecordLifecycle::Retired {
                    at: SimTime::EPOCH,
                    successor: Some(active_ref.clone()),
                },
            ),
            domain_record(
                deleted_ref.clone(),
                DomainRecordClass::Entity,
                DomainRecordLifecycle::Deleted { at: SimTime::EPOCH },
            ),
        ]);
        let schemas = holder_schemas(&kind, KnowledgeHolderPolicy::Allowed);
        let active = KnowledgeHolderRef::Entity(EntityRef::Domain(active_ref));
        let retired = KnowledgeHolderRef::Entity(EntityRef::Domain(retired_ref));
        let deleted = KnowledgeHolderRef::Entity(EntityRef::Domain(deleted_ref));

        assert!(validate_holder_for_publication(&active, &current, &schemas).is_ok());
        assert!(validate_holder_for_publication(&retired, &current, &schemas).is_err());
        assert!(validate_holder_for_publication(&deleted, &current, &schemas).is_err());
        assert!(validate_historical_holder(&retired, &current, &schemas).is_ok());
        assert!(validate_historical_holder(&deleted, &current, &schemas).is_ok());

        let disallowed = holder_schemas(&kind, KnowledgeHolderPolicy::Disallowed);
        assert!(validate_holder_for_publication(&active, &current, &disallowed).is_err());
        for holder in [
            KnowledgeHolderRef::Entity(EntityRef::Person(PersonId::new(1))),
            KnowledgeHolderRef::Entity(EntityRef::Resource(ResourceId::new(1))),
            KnowledgeHolderRef::Entity(EntityRef::Route(RouteId::new(1))),
            KnowledgeHolderRef::Entity(EntityRef::Territory(TerritoryId::new(1))),
        ] {
            let error = validate_holder_for_publication(&holder, &current, &schemas)
                .expect_err("ineligible core holder aliases must be rejected");
            assert_eq!(error.code, ErrorCode::InvalidKnowledgeHolder);
        }
    }

    #[test]
    fn generic_person_registry_is_an_eligible_person_holder() {
        let person = PersonId::new(7);
        let holder = KnowledgeHolderRef::Person(person);
        let mut current = current_state(vec![]);
        current.entities.insert(EntityRef::Person(person));

        assert!(validate_holder_for_publication(&holder, &current, &BTreeMap::new()).is_ok());
        assert!(validate_historical_holder(&holder, &current, &BTreeMap::new()).is_ok());
    }

    #[test]
    fn any_entity_accepts_future_entity_kinds_but_rejects_value_records() {
        let entity_kind = DomainRecordKind::new("future.organization", "institution");
        let value_kind = DomainRecordKind::new("future.organization", "memo");
        let entity_ref = DomainRecordRef::new(&entity_kind.namespace, &entity_kind.name, "one");
        let value_ref = DomainRecordRef::new(&value_kind.namespace, &value_kind.name, "one");
        let mut current = current_state(vec![
            domain_record(
                entity_ref.clone(),
                DomainRecordClass::Entity,
                DomainRecordLifecycle::Active,
            ),
            domain_record(
                value_ref.clone(),
                DomainRecordClass::Record,
                DomainRecordLifecycle::Active,
            ),
        ]);
        current.territories.insert(
            TerritoryId::new(1),
            Territory {
                id: TerritoryId::new(1),
                name: "Territory".to_owned(),
                controller: GovernmentId::new(1),
                position: MapPoint::default(),
            },
        );
        current.routes.insert(
            RouteId::new(1),
            Route {
                id: RouteId::new(1),
                name: "Route".to_owned(),
                from: TerritoryId::new(1),
                to: TerritoryId::new(2),
                travel_minutes: 1,
                terrain: "road".to_owned(),
            },
        );
        let schemas = holder_schemas(&entity_kind, KnowledgeHolderPolicy::Allowed);
        let schema = knowledge_schema(vec![KnowledgeSubjectSchema {
            role: "subject".to_owned(),
            targets: vec![KnowledgeSubjectTargetKind::AnyEntity],
            required: true,
            multiple: false,
        }]);
        let holder = KnowledgeHolderRef::Entity(EntityRef::Domain(entity_ref.clone()));
        let mut draft = base_draft();
        draft.subjects = vec![KnowledgeSubject {
            role: "subject".to_owned(),
            target: KnowledgeSubjectTarget::Entity(EntityRef::Domain(entity_ref)),
        }];
        assert!(validate_draft(&draft, &schema, &holder, &current, &schemas).is_ok());

        draft.subjects[0].target =
            KnowledgeSubjectTarget::Entity(EntityRef::Territory(TerritoryId::new(1)));
        assert!(validate_draft(&draft, &schema, &holder, &current, &schemas).is_ok());
        draft.subjects[0].target =
            KnowledgeSubjectTarget::Entity(EntityRef::Route(RouteId::new(1)));
        assert!(validate_draft(&draft, &schema, &holder, &current, &schemas).is_ok());

        draft.subjects[0].target = KnowledgeSubjectTarget::DomainRecord(value_ref);
        let error = validate_draft(&draft, &schema, &holder, &current, &schemas)
            .expect_err("AnyEntity must not accept ordinary domain value records");
        assert_eq!(error.code, ErrorCode::InvalidKnowledgeRecord);

        let exact_person = knowledge_schema(vec![KnowledgeSubjectSchema {
            role: "subject".to_owned(),
            targets: vec![KnowledgeSubjectTargetKind::Core(CoreEntityKind::Person)],
            required: true,
            multiple: false,
        }]);
        draft.subjects[0].target =
            KnowledgeSubjectTarget::Entity(EntityRef::Person(PersonId::new(999)));
        let error = validate_draft(&draft, &exact_person, &holder, &current, &schemas)
            .expect_err("an exact core target must still name an existing entity");
        assert_eq!(error.code, ErrorCode::InvalidKnowledgeRecord);
    }

    #[test]
    fn schema_set_requires_exact_ownership_and_one_writable_version() {
        let writable = knowledge_schema(vec![]);
        let read_only = PluginKnowledgeSchema {
            id: KnowledgeSchemaId::new(knowledge_kind(), 2),
            writable: false,
            ..writable.clone()
        };
        let owners = BTreeMap::from([(knowledge_kind(), "fixture".to_owned())]);
        let schemas = BTreeMap::from([
            (
                writable.id.clone(),
                ("fixture".to_owned(), writable.clone()),
            ),
            (
                read_only.id.clone(),
                ("fixture".to_owned(), read_only.clone()),
            ),
        ]);
        assert!(validate_schema_set(&schemas, &owners).is_ok());

        let only_read_only =
            BTreeMap::from([(read_only.id.clone(), ("fixture".to_owned(), read_only))]);
        assert!(validate_schema_set(&only_read_only, &owners).is_err());

        let second_writable = PluginKnowledgeSchema {
            id: KnowledgeSchemaId::new(knowledge_kind(), 2),
            ..writable.clone()
        };
        let two_writable = BTreeMap::from([
            (writable.id.clone(), ("fixture".to_owned(), writable)),
            (
                second_writable.id.clone(),
                ("fixture".to_owned(), second_writable),
            ),
        ]);
        assert!(validate_schema_set(&two_writable, &owners).is_err());
    }

    #[test]
    fn current_heads_follow_the_cut_the_selection_and_the_bound() {
        let kind = knowledge_kind();
        let other_kind = KnowledgeRecordKind::new("fixture.knowledge", "rumour");
        let holder = KnowledgeHolderRef::Person(PersonId::new(1));
        let neighbour = KnowledgeHolderRef::Person(PersonId::new(2));
        let subject = |event: u64| KnowledgeSubject {
            role: "subject".to_owned(),
            target: KnowledgeSubjectTarget::Event(EventId::new(event)),
        };
        let record = |id: u64,
                      holder: &KnowledgeHolderRef,
                      kind: &KnowledgeRecordKind,
                      event: u64,
                      supersedes: &[u64]| {
            let draft = base_draft();
            let stored = KnowledgeRecord {
                id: KnowledgeRecordId::new(id),
                holder: holder.clone(),
                schema: KnowledgeSchemaId::new(kind.clone(), 1),
                subjects: vec![subject(event)],
                payload: draft.payload,
                as_of: None,
                learned_at: SimTime::EPOCH,
                confidence_per_mille: 1_000,
                origin: draft.origin,
                supersedes: supersedes
                    .iter()
                    .copied()
                    .map(KnowledgeRecordId::new)
                    .collect(),
                contradicts: Vec::new(),
            };
            (stored.id, stored)
        };
        let committed: HolderKnowledgeRecords = BTreeMap::from([
            (
                holder.clone(),
                BTreeMap::from([
                    record(1, &holder, &kind, 7, &[]),
                    record(2, &holder, &kind, 7, &[1]),
                    record(3, &holder, &kind, 8, &[]),
                    record(4, &holder, &other_kind, 7, &[]),
                ]),
            ),
            (
                neighbour.clone(),
                BTreeMap::from([record(5, &neighbour, &kind, 7, &[])]),
            ),
        ]);
        // Records the settling boundary staged for the same boundary.
        let visible: HolderKnowledgeRecords = BTreeMap::from([(
            holder.clone(),
            BTreeMap::from([
                record(6, &holder, &kind, 8, &[3]),
                record(7, &holder, &kind, 9, &[]),
            ]),
        )]);
        let heads = |holder: &KnowledgeHolderRef,
                     kind: &KnowledgeRecordKind,
                     subjects: &[KnowledgeSubject],
                     visible: Option<&HolderKnowledgeRecords>,
                     limit: usize| {
            let heads = current_heads_at_cut(holder, kind, subjects, &committed, visible, limit);
            (
                heads.records.iter().map(|id| id.get()).collect::<Vec<_>>(),
                heads.truncated,
            )
        };

        // Committed state alone: the superseded record is not current.
        assert_eq!(heads(&holder, &kind, &[], None, 64), (vec![2, 3], false));
        // A visible record of the boundary replaces the record it supersedes.
        assert_eq!(
            heads(&holder, &kind, &[], Some(&visible), 64),
            (vec![2, 6, 7], false)
        );
        // Subjects, kind, and holder select; none of them leaks across.
        assert_eq!(
            heads(&holder, &kind, &[subject(8)], Some(&visible), 64),
            (vec![6], false)
        );
        assert_eq!(
            heads(&holder, &kind, &[subject(7), subject(8)], None, 64),
            (vec![], false)
        );
        assert_eq!(heads(&holder, &other_kind, &[], None, 64), (vec![4], false));
        assert_eq!(
            heads(&neighbour, &kind, &[], Some(&visible), 64),
            (vec![5], false)
        );
        assert_eq!(
            heads(
                &KnowledgeHolderRef::Person(PersonId::new(3)),
                &kind,
                &[],
                None,
                64
            ),
            (vec![], false)
        );
        // The result is bounded: it keeps the newest records, in ascending
        // order, and says when older current records were left out.
        assert_eq!(
            heads(&holder, &kind, &[], Some(&visible), 3),
            (vec![2, 6, 7], false)
        );
        assert_eq!(
            heads(&holder, &kind, &[], Some(&visible), 2),
            (vec![6, 7], true)
        );
        assert_eq!(
            heads(&holder, &kind, &[], Some(&visible), 1),
            (vec![7], true)
        );
        assert_eq!(heads(&holder, &kind, &[], None, 1), (vec![3], true));
    }

    #[test]
    fn per_record_limits_accept_boundary_and_reject_boundary_plus_one() {
        let (holder, current) = government_current();
        let schemas = BTreeMap::new();
        let no_subjects = knowledge_schema(vec![]);

        let mut payload = base_draft();
        payload.payload =
            json!("a".repeat(KnowledgeLimitsV1::CURRENT.payload_bytes_per_record - 2));
        assert!(validate_draft(&payload, &no_subjects, &holder, &current, &schemas).is_ok());
        payload.payload =
            json!("a".repeat(KnowledgeLimitsV1::CURRENT.payload_bytes_per_record - 1));
        assert!(validate_draft(&payload, &no_subjects, &holder, &current, &schemas).is_err());

        let mut method = base_draft();
        method.origin.method = "m".repeat(KnowledgeLimitsV1::CURRENT.text_bytes);
        assert!(validate_draft(&method, &no_subjects, &holder, &current, &schemas).is_ok());
        method.origin.method.push('m');
        assert!(validate_draft(&method, &no_subjects, &holder, &current, &schemas).is_err());

        let mut evidence = base_draft();
        evidence.origin.evidence = (1..=KnowledgeLimitsV1::CURRENT.relations_per_record)
            .map(|id| EvidenceRef::Event(EventId::new(id as u64)))
            .collect();
        assert!(validate_draft(&evidence, &no_subjects, &holder, &current, &schemas).is_ok());
        evidence
            .origin
            .evidence
            .push(EvidenceRef::Event(EventId::new(
                KnowledgeLimitsV1::CURRENT.relations_per_record as u64 + 1,
            )));
        assert!(validate_draft(&evidence, &no_subjects, &holder, &current, &schemas).is_err());

        let mut supersedes = base_draft();
        supersedes.supersedes = (1..=KnowledgeLimitsV1::CURRENT.relations_per_record)
            .map(|id| KnowledgeRecordId::new(id as u64))
            .collect();
        assert!(validate_draft(&supersedes, &no_subjects, &holder, &current, &schemas).is_ok());
        supersedes.supersedes.push(KnowledgeRecordId::new(
            KnowledgeLimitsV1::CURRENT.relations_per_record as u64 + 1,
        ));
        assert!(validate_draft(&supersedes, &no_subjects, &holder, &current, &schemas).is_err());

        let mut contradicts = base_draft();
        contradicts.contradicts = (1..=KnowledgeLimitsV1::CURRENT.relations_per_record)
            .map(|id| KnowledgeRecordId::new(id as u64))
            .collect();
        assert!(validate_draft(&contradicts, &no_subjects, &holder, &current, &schemas).is_ok());
        contradicts.contradicts.push(KnowledgeRecordId::new(
            KnowledgeLimitsV1::CURRENT.relations_per_record as u64 + 1,
        ));
        assert!(validate_draft(&contradicts, &no_subjects, &holder, &current, &schemas).is_err());

        let event_subject = knowledge_schema(vec![KnowledgeSubjectSchema {
            role: "subject".to_owned(),
            targets: vec![KnowledgeSubjectTargetKind::Event],
            required: true,
            multiple: true,
        }]);
        let mut subjects = base_draft();
        subjects.subjects = (1..=KnowledgeLimitsV1::CURRENT.relations_per_record)
            .map(|id| KnowledgeSubject {
                role: "subject".to_owned(),
                target: KnowledgeSubjectTarget::Event(EventId::new(id as u64)),
            })
            .collect();
        assert!(validate_draft(&subjects, &event_subject, &holder, &current, &schemas).is_ok());
        subjects.subjects.push(KnowledgeSubject {
            role: "subject".to_owned(),
            target: KnowledgeSubjectTarget::Event(EventId::new(
                KnowledgeLimitsV1::CURRENT.relations_per_record as u64 + 1,
            )),
        });
        assert!(validate_draft(&subjects, &event_subject, &holder, &current, &schemas).is_err());
    }
}
