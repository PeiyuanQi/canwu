use crate::{CanwuError, ErrorCode, PayloadSchema, StateKey, StateVisibility};
use canwu_core::{
    CoreEntityKind, DomainEntityType, DomainKindClass, DomainRecordKind, DomainRecordRef,
    DomainRecordType, DomainValueType, EntityRef, KnowledgeHolderPolicy, TypedDomainRecordRef,
};
use canwu_time::SimTime;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DomainRecordClass {
    Entity,
    Record,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "type", content = "kind", rename_all = "snake_case")]
pub enum DomainReferenceTargetKind {
    Core(CoreEntityKind),
    Domain(DomainRecordKind),
    AnyEntity,
}

impl DomainReferenceTargetKind {
    #[must_use]
    pub fn for_domain<T: DomainRecordType>() -> Self {
        Self::Domain(DomainRecordKind::for_type::<T>())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DomainReferenceSchema {
    pub role: String,
    pub targets: Vec<DomainReferenceTargetKind>,
    pub required: bool,
    pub multiple: bool,
    pub allow_retired: bool,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DomainRecordMutationPolicy {
    #[default]
    Versioned,
    CreateOnly,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DomainRecordSchema {
    pub kind: DomainRecordKind,
    pub class: DomainRecordClass,
    #[serde(default)]
    pub holder_policy: KnowledgeHolderPolicy,
    #[serde(default)]
    pub mutation_policy: DomainRecordMutationPolicy,
    pub payload_schema: PayloadSchema,
    pub references: Vec<DomainReferenceSchema>,
}

impl DomainRecordSchema {
    #[must_use]
    pub fn new(kind: DomainRecordKind, class: DomainRecordClass) -> Self {
        Self {
            kind,
            class,
            holder_policy: KnowledgeHolderPolicy::Disallowed,
            mutation_policy: DomainRecordMutationPolicy::Versioned,
            payload_schema: PayloadSchema::Any,
            references: Vec::new(),
        }
    }

    #[must_use]
    pub fn for_type<T: DomainRecordType>() -> Self {
        let class = if T::Class::IS_ENTITY {
            DomainRecordClass::Entity
        } else {
            DomainRecordClass::Record
        };
        Self::new(DomainRecordKind::for_type::<T>(), class)
    }

    #[must_use]
    pub fn for_entity<T: DomainEntityType>() -> Self {
        Self::for_type::<T>()
    }

    #[must_use]
    pub fn for_record<T: DomainValueType>() -> Self {
        Self::for_type::<T>()
    }

    #[must_use]
    pub fn state_key(&self) -> StateKey {
        record_state_key(&self.kind)
    }

    pub(crate) fn canonicalize(&mut self) {
        for reference in &mut self.references {
            reference.targets.sort();
            reference.targets.dedup();
        }
        self.references
            .sort_by(|left, right| left.role.cmp(&right.role));
    }

    pub(crate) fn validate(&self) -> Result<(), CanwuError> {
        validate_kind(&self.kind)?;
        if self.holder_policy == KnowledgeHolderPolicy::Allowed
            && self.class != DomainRecordClass::Entity
        {
            return invalid_record("only domain entity schemas may allow knowledge holders");
        }
        if let PayloadSchema::Object { properties, .. } = &self.payload_schema
            && properties.keys().any(|name| !canonical_text(name))
        {
            return invalid_record(
                "record payload-schema property names must be non-empty and canonical",
            );
        }
        if self
            .references
            .windows(2)
            .any(|pair| pair[0].role >= pair[1].role)
        {
            return invalid_record("record-schema reference roles must be unique and sorted");
        }
        for reference in &self.references {
            if !canonical_text(&reference.role)
                || reference.targets.is_empty()
                || reference.targets.windows(2).any(|pair| pair[0] >= pair[1])
            {
                return invalid_record(
                    "record-schema references require canonical roles and unique sorted targets",
                );
            }
            for target in &reference.targets {
                if let DomainReferenceTargetKind::Domain(kind) = target {
                    validate_kind(kind)?;
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "type", content = "reference", rename_all = "snake_case")]
pub enum DomainReferenceTarget {
    Core(EntityRef),
    Domain(DomainRecordRef),
}

impl DomainReferenceTarget {
    #[must_use]
    pub fn from_typed<T: DomainRecordType>(reference: TypedDomainRecordRef<T>) -> Self {
        Self::Domain(reference.into_untyped())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct DomainReference {
    pub role: String,
    pub target: DomainReferenceTarget,
}

impl DomainReference {
    #[must_use]
    pub fn from_typed<T: DomainRecordType>(
        role: impl Into<String>,
        reference: TypedDomainRecordRef<T>,
    ) -> Self {
        Self {
            role: role.into(),
            target: DomainReferenceTarget::from_typed(reference),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct DomainRecordDraft {
    pub reference: DomainRecordRef,
    pub payload: Value,
    pub references: Vec<DomainReference>,
}

impl DomainRecordDraft {
    #[must_use]
    pub fn new(reference: DomainRecordRef, payload: Value) -> Self {
        Self {
            reference,
            payload,
            references: Vec::new(),
        }
    }

    pub fn from_typed<T: DomainRecordType>(
        reference: TypedDomainRecordRef<T>,
        payload: &T::Payload,
    ) -> Result<Self, CanwuError>
    where
        T::Payload: Serialize,
    {
        let payload = serde_json::to_value(payload).map_err(|error| {
            CanwuError::new(
                ErrorCode::InvalidDomainRecord,
                format!(
                    "typed domain payload for {} could not be encoded: {error}",
                    DomainRecordKind::for_type::<T>()
                ),
            )
        })?;
        Ok(Self::new(reference.into_untyped(), payload))
    }

    pub(crate) fn canonicalize(&mut self) {
        self.references.sort();
        self.references.dedup();
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DomainRecordLifecycle {
    Active,
    Retired {
        at: SimTime,
        successor: Option<DomainRecordRef>,
    },
    Deleted {
        at: SimTime,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct DomainRecord {
    pub reference: DomainRecordRef,
    pub owner: String,
    pub class: DomainRecordClass,
    pub version: u64,
    pub lifecycle: DomainRecordLifecycle,
    pub payload: Value,
    pub references: Vec<DomainReference>,
}

impl DomainRecord {
    #[must_use]
    pub const fn is_deleted(&self) -> bool {
        matches!(self.lifecycle, DomainRecordLifecycle::Deleted { .. })
    }

    #[must_use]
    pub const fn is_active(&self) -> bool {
        matches!(self.lifecycle, DomainRecordLifecycle::Active)
    }

    #[must_use]
    pub fn typed_reference<T: DomainRecordType>(&self) -> Option<TypedDomainRecordRef<T>> {
        TypedDomainRecordRef::from_untyped(self.reference.clone()).ok()
    }

    pub fn decode_payload<T: DomainRecordType>(&self) -> Result<T::Payload, CanwuError>
    where
        T::Payload: DeserializeOwned,
    {
        if !self.reference.kind.matches_type::<T>() {
            return Err(CanwuError::new(
                ErrorCode::InvalidDomainRecord,
                format!(
                    "domain record {} cannot be decoded as kind {}",
                    self.reference,
                    DomainRecordKind::for_type::<T>()
                ),
            ));
        }
        T::Payload::deserialize(&self.payload).map_err(|error| {
            CanwuError::new(
                ErrorCode::InvalidDomainRecord,
                format!(
                    "domain record {} has an incompatible typed payload: {error}",
                    self.reference
                ),
            )
        })
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum DomainRecordMutation {
    Create {
        record: DomainRecordDraft,
    },
    Update {
        record: DomainRecordDraft,
        expected_version: u64,
    },
    Retire {
        record: DomainRecordRef,
        expected_version: u64,
        successor: Option<DomainRecordRef>,
    },
    Delete {
        record: DomainRecordRef,
        expected_version: u64,
    },
}

impl DomainRecordMutation {
    #[must_use]
    pub const fn target(&self) -> &DomainRecordRef {
        match self {
            Self::Create { record } | Self::Update { record, .. } => &record.reference,
            Self::Retire { record, .. } | Self::Delete { record, .. } => record,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DomainRecordOperation {
    Created,
    Updated,
    Retired,
    Deleted,
}

impl DomainRecordOperation {
    pub(crate) const fn event_type(self) -> &'static str {
        match self {
            Self::Created => "domain_record_created",
            Self::Updated => "domain_record_updated",
            Self::Retired => "domain_record_retired",
            Self::Deleted => "domain_record_deleted",
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct DomainRecordChange {
    pub plugin: String,
    pub system: String,
    pub operation: DomainRecordOperation,
    pub previous: Option<DomainRecord>,
    pub current: DomainRecord,
    pub visibility: StateVisibility,
    pub summary: String,
}

pub(crate) type DomainRecordSchemas = BTreeMap<DomainRecordKind, (String, DomainRecordSchema)>;

pub(crate) struct DomainMutationRequest<'a> {
    pub plugin: &'a str,
    pub system: &'a str,
    pub visibility: StateVisibility,
    pub mutation: &'a DomainRecordMutation,
    pub summary: &'a str,
}

pub(crate) fn record_state_key(kind: &DomainRecordKind) -> StateKey {
    StateKey::new(&kind.namespace, &kind.name)
}

pub(crate) fn validate_initial_records(
    records: &[DomainRecord],
    now: SimTime,
    core_exists: &dyn Fn(&EntityRef) -> bool,
) -> Result<(), CanwuError> {
    let mut store = BTreeMap::new();
    for record in records {
        validate_record_shape(record, now)?;
        if store
            .insert(record.reference.clone(), record.clone())
            .is_some()
        {
            return Err(CanwuError::new(
                ErrorCode::DuplicateDomainRecord,
                format!("domain record {} is duplicated", record.reference),
            ));
        }
    }
    for record in store.values().filter(|record| !record.is_deleted()) {
        validate_reference_targets_basic(record, &store, core_exists)?;
        validate_successor(record, &store)?;
    }
    validate_successor_graph(&store)?;
    Ok(())
}

pub(crate) fn validate_record_store(
    records: &BTreeMap<DomainRecordRef, DomainRecord>,
    schemas: &DomainRecordSchemas,
    now: SimTime,
    core_exists: &dyn Fn(&EntityRef) -> bool,
) -> Result<(), CanwuError> {
    for (reference, record) in records {
        if reference != &record.reference {
            return invalid_record("domain record map key disagrees with its stable reference");
        }
        validate_record_shape(record, now)?;
        let Some((owner, schema)) = schemas.get(&record.reference.kind) else {
            return Err(CanwuError::new(
                ErrorCode::PluginNotActive,
                format!(
                    "domain record kind {} has no active schema owner",
                    record.reference.kind
                ),
            ));
        };
        if &record.owner != owner || record.class != schema.class {
            return invalid_record(format!(
                "domain record {} disagrees with its schema owner or class",
                record.reference
            ));
        }
        schema.payload_schema.validate(&record.payload)?;
        validate_record_references(record, schema, records, core_exists)?;
        validate_successor(record, records)?;
    }
    validate_successor_graph(records)?;
    Ok(())
}

pub(crate) fn validate_records_for_owner(
    records: &BTreeMap<DomainRecordRef, DomainRecord>,
    schemas: &DomainRecordSchemas,
    owner: &str,
    now: SimTime,
    core_exists: &dyn Fn(&EntityRef) -> bool,
) -> Result<(), CanwuError> {
    for record in records.values().filter(|record| record.owner == owner) {
        validate_record_shape(record, now)?;
        let Some((schema_owner, schema)) = schemas.get(&record.reference.kind) else {
            return invalid_record(format!(
                "plugin {owner} did not register schema for owned record kind {}",
                record.reference.kind
            ));
        };
        if schema_owner != owner || record.class != schema.class {
            return invalid_record(format!(
                "domain record {} disagrees with its registered owner or class",
                record.reference
            ));
        }
        schema.payload_schema.validate(&record.payload)?;
        validate_record_references(record, schema, records, core_exists)?;
        validate_successor(record, records)?;
    }
    validate_successor_graph(records)?;
    Ok(())
}

pub(crate) fn apply_mutation_bundle(
    records: &BTreeMap<DomainRecordRef, DomainRecord>,
    schemas: &DomainRecordSchemas,
    now: SimTime,
    core_exists: &dyn Fn(&EntityRef) -> bool,
    mut requests: Vec<DomainMutationRequest<'_>>,
) -> Result<
    (
        BTreeMap<DomainRecordRef, DomainRecord>,
        Vec<DomainRecordChange>,
    ),
    CanwuError,
> {
    requests.sort_by(|left, right| left.mutation.target().cmp(right.mutation.target()));
    if requests
        .windows(2)
        .any(|pair| pair[0].mutation.target() == pair[1].mutation.target())
    {
        return invalid_record("a boundary cannot mutate the same domain record twice");
    }
    let created_records = requests
        .iter()
        .filter_map(|request| match request.mutation {
            DomainRecordMutation::Create { record } => Some(record.reference.clone()),
            DomainRecordMutation::Update { .. }
            | DomainRecordMutation::Retire { .. }
            | DomainRecordMutation::Delete { .. } => None,
        })
        .collect::<BTreeSet<_>>();

    let mut next = records.clone();
    let mut changes = Vec::with_capacity(requests.len());
    for request in requests {
        if !canonical_text(request.summary) {
            return invalid_record("domain record mutations require a canonical summary");
        }
        let target = request.mutation.target();
        validate_reference(target)?;
        let Some((owner, schema)) = schemas.get(&target.kind) else {
            return invalid_record(format!(
                "domain record kind {} has no registered schema",
                target.kind
            ));
        };
        if owner != request.plugin {
            return Err(CanwuError::new(
                ErrorCode::UndeclaredStateWrite,
                format!(
                    "plugin {} cannot mutate domain record kind {} owned by {owner}",
                    request.plugin, target.kind
                ),
            ));
        }
        if schema.mutation_policy == DomainRecordMutationPolicy::CreateOnly
            && !matches!(request.mutation, DomainRecordMutation::Create { .. })
        {
            return invalid_record(format!("domain record kind {} is create-only", target.kind));
        }

        let (operation, previous, current) = match request.mutation {
            DomainRecordMutation::Create { record } => {
                let mut record = record.clone();
                record.canonicalize();
                if next.contains_key(&record.reference) {
                    return Err(CanwuError::new(
                        ErrorCode::DuplicateDomainRecord,
                        format!("domain record {} already exists", record.reference),
                    ));
                }
                let current = DomainRecord {
                    reference: record.reference,
                    owner: owner.clone(),
                    class: schema.class,
                    version: 1,
                    lifecycle: DomainRecordLifecycle::Active,
                    payload: record.payload,
                    references: record.references,
                };
                next.insert(current.reference.clone(), current.clone());
                (DomainRecordOperation::Created, None, current)
            }
            DomainRecordMutation::Update {
                record,
                expected_version,
            } => {
                let mut draft = record.clone();
                draft.canonicalize();
                let previous = require_mutable_record(&next, target, *expected_version)?.clone();
                let version = next_record_version(previous.version)?;
                let current = DomainRecord {
                    reference: draft.reference,
                    owner: previous.owner.clone(),
                    class: previous.class,
                    version,
                    lifecycle: DomainRecordLifecycle::Active,
                    payload: draft.payload,
                    references: draft.references,
                };
                next.insert(current.reference.clone(), current.clone());
                (DomainRecordOperation::Updated, Some(previous), current)
            }
            DomainRecordMutation::Retire {
                record,
                expected_version,
                successor,
            } => {
                let previous = require_mutable_record(&next, record, *expected_version)?.clone();
                validate_new_successor(record, successor.as_ref(), records, &created_records)?;
                let mut current = previous.clone();
                current.version = next_record_version(previous.version)?;
                current.lifecycle = DomainRecordLifecycle::Retired {
                    at: now,
                    successor: successor.clone(),
                };
                next.insert(record.clone(), current.clone());
                (DomainRecordOperation::Retired, Some(previous), current)
            }
            DomainRecordMutation::Delete {
                record,
                expected_version,
            } => {
                let previous = next.get(record).ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::DomainRecordNotFound,
                        format!("domain record {record} was not found"),
                    )
                })?;
                if previous.version != *expected_version {
                    return Err(version_conflict(
                        record,
                        *expected_version,
                        previous.version,
                    ));
                }
                if !matches!(previous.lifecycle, DomainRecordLifecycle::Retired { .. }) {
                    return invalid_record("domain records must be retired before deletion");
                }
                let previous = previous.clone();
                let mut current = previous.clone();
                current.version = next_record_version(previous.version)?;
                current.lifecycle = DomainRecordLifecycle::Deleted { at: now };
                current.references.clear();
                next.insert(record.clone(), current.clone());
                (DomainRecordOperation::Deleted, Some(previous), current)
            }
        };
        changes.push(DomainRecordChange {
            plugin: request.plugin.to_owned(),
            system: request.system.to_owned(),
            operation,
            previous,
            current,
            visibility: request.visibility,
            summary: request.summary.to_owned(),
        });
    }
    validate_record_store(&next, schemas, now, core_exists)?;
    Ok((next, changes))
}

pub(crate) fn domain_entity_exists(
    records: &BTreeMap<DomainRecordRef, DomainRecord>,
    reference: &DomainRecordRef,
) -> bool {
    records
        .get(reference)
        .is_some_and(|record| record.class == DomainRecordClass::Entity && !record.is_deleted())
}

pub(crate) fn mutation_from_change(change: &DomainRecordChange) -> DomainRecordMutation {
    match change.operation {
        DomainRecordOperation::Created => DomainRecordMutation::Create {
            record: DomainRecordDraft {
                reference: change.current.reference.clone(),
                payload: change.current.payload.clone(),
                references: change.current.references.clone(),
            },
        },
        DomainRecordOperation::Updated => DomainRecordMutation::Update {
            record: DomainRecordDraft {
                reference: change.current.reference.clone(),
                payload: change.current.payload.clone(),
                references: change.current.references.clone(),
            },
            expected_version: change.previous.as_ref().map_or(0, |record| record.version),
        },
        DomainRecordOperation::Retired => DomainRecordMutation::Retire {
            record: change.current.reference.clone(),
            expected_version: change.previous.as_ref().map_or(0, |record| record.version),
            successor: match &change.current.lifecycle {
                DomainRecordLifecycle::Retired { successor, .. } => successor.clone(),
                DomainRecordLifecycle::Active | DomainRecordLifecycle::Deleted { .. } => None,
            },
        },
        DomainRecordOperation::Deleted => DomainRecordMutation::Delete {
            record: change.current.reference.clone(),
            expected_version: change.previous.as_ref().map_or(0, |record| record.version),
        },
    }
}

fn validate_record_shape(record: &DomainRecord, now: SimTime) -> Result<(), CanwuError> {
    validate_reference(&record.reference)?;
    if !canonical_text(&record.owner)
        || record.version == 0
        || record.references.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return invalid_record(format!(
            "domain record {} has noncanonical identity, owner, version, or references",
            record.reference
        ));
    }
    for reference in &record.references {
        if !canonical_text(&reference.role) {
            return invalid_record("domain record reference roles must be canonical");
        }
        validate_target(&reference.target)?;
    }
    match &record.lifecycle {
        DomainRecordLifecycle::Active => {}
        DomainRecordLifecycle::Retired { at, successor } => {
            if *at > now {
                return invalid_record("domain record retirement cannot be future-dated");
            }
            if let Some(successor) = successor {
                validate_reference(successor)?;
            }
        }
        DomainRecordLifecycle::Deleted { at } => {
            if *at > now || !record.references.is_empty() {
                return invalid_record(
                    "deleted domain-record tombstones cannot be future-dated or retain references",
                );
            }
        }
    }
    Ok(())
}

fn validate_record_references(
    record: &DomainRecord,
    schema: &DomainRecordSchema,
    records: &BTreeMap<DomainRecordRef, DomainRecord>,
    core_exists: &dyn Fn(&EntityRef) -> bool,
) -> Result<(), CanwuError> {
    if record.is_deleted() {
        return Ok(());
    }
    let mut by_role = BTreeMap::<&str, Vec<&DomainReferenceTarget>>::new();
    for reference in &record.references {
        by_role
            .entry(reference.role.as_str())
            .or_default()
            .push(&reference.target);
    }
    for reference_schema in &schema.references {
        let targets = by_role
            .remove(reference_schema.role.as_str())
            .unwrap_or_default();
        if (reference_schema.required && targets.is_empty())
            || (!reference_schema.multiple && targets.len() > 1)
        {
            return invalid_record(format!(
                "domain record {} violates reference cardinality for role {}",
                record.reference, reference_schema.role
            ));
        }
        for target in targets {
            validate_reference_target(
                target,
                reference_schema,
                records,
                core_exists,
                &record.reference,
            )?;
        }
    }
    if !by_role.is_empty() {
        return invalid_record(format!(
            "domain record {} contains undeclared reference roles",
            record.reference
        ));
    }
    Ok(())
}

fn validate_reference_target(
    target: &DomainReferenceTarget,
    schema: &DomainReferenceSchema,
    records: &BTreeMap<DomainRecordRef, DomainRecord>,
    core_exists: &dyn Fn(&EntityRef) -> bool,
    source: &DomainRecordRef,
) -> Result<(), CanwuError> {
    let (kind, is_entity) = match target {
        DomainReferenceTarget::Core(entity) => {
            let Some(kind) = entity.core_kind() else {
                return invalid_record(
                    "domain entities use domain references rather than core-reference aliases",
                );
            };
            if !core_exists(entity) {
                return invalid_record(format!(
                    "domain record {source} references missing core entity {entity}"
                ));
            }
            (DomainReferenceTargetKind::Core(kind), true)
        }
        DomainReferenceTarget::Domain(reference) => {
            let target_record = records.get(reference).ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::DomainRecordNotFound,
                    format!("domain record {source} references missing record {reference}"),
                )
            })?;
            if target_record.is_deleted() || (!schema.allow_retired && !target_record.is_active()) {
                return Err(CanwuError::new(
                    ErrorCode::DomainRecordReferenced,
                    format!("domain record {source} references unavailable record {reference}"),
                ));
            }
            (
                DomainReferenceTargetKind::Domain(reference.kind.clone()),
                target_record.class == DomainRecordClass::Entity,
            )
        }
    };
    if !(schema.targets.contains(&kind)
        || is_entity
            && schema
                .targets
                .contains(&DomainReferenceTargetKind::AnyEntity))
    {
        return invalid_record(format!(
            "domain record {source} reference target does not match role {}",
            schema.role
        ));
    }
    Ok(())
}

fn validate_reference_targets_basic(
    record: &DomainRecord,
    records: &BTreeMap<DomainRecordRef, DomainRecord>,
    core_exists: &dyn Fn(&EntityRef) -> bool,
) -> Result<(), CanwuError> {
    for reference in &record.references {
        match &reference.target {
            DomainReferenceTarget::Core(entity) => {
                if entity.core_kind().is_none() || !core_exists(entity) {
                    return invalid_record(format!(
                        "domain record {} references missing core entity {entity}",
                        record.reference
                    ));
                }
            }
            DomainReferenceTarget::Domain(target) => {
                if records.get(target).is_none_or(DomainRecord::is_deleted) {
                    return invalid_record(format!(
                        "domain record {} references unavailable record {target}",
                        record.reference
                    ));
                }
            }
        }
    }
    Ok(())
}

fn validate_successor(
    record: &DomainRecord,
    records: &BTreeMap<DomainRecordRef, DomainRecord>,
) -> Result<(), CanwuError> {
    let DomainRecordLifecycle::Retired {
        successor: Some(successor),
        ..
    } = &record.lifecycle
    else {
        return Ok(());
    };
    let Some(target) = records.get(successor) else {
        return invalid_record(format!(
            "retired domain record {} has a missing successor {successor}",
            record.reference
        ));
    };
    if successor == &record.reference
        || successor.kind != record.reference.kind
        || target.is_deleted()
    {
        return invalid_record(
            "domain record successors must be distinct available records of the same kind",
        );
    }
    Ok(())
}

fn validate_new_successor(
    record: &DomainRecordRef,
    successor: Option<&DomainRecordRef>,
    records: &BTreeMap<DomainRecordRef, DomainRecord>,
    created_records: &BTreeSet<DomainRecordRef>,
) -> Result<(), CanwuError> {
    let Some(successor) = successor else {
        return Ok(());
    };
    if successor == record || successor.kind != record.kind {
        return invalid_record(
            "domain record successors must be distinct active records of the same kind",
        );
    }
    if created_records.contains(successor) {
        return Ok(());
    }
    let Some(target) = records.get(successor) else {
        return invalid_record(format!(
            "retired domain record {record} has a missing successor {successor}",
        ));
    };
    if !target.is_active() {
        return invalid_record("new domain record successors must be active when admitted");
    }
    Ok(())
}

fn validate_successor_graph(
    records: &BTreeMap<DomainRecordRef, DomainRecord>,
) -> Result<(), CanwuError> {
    let mut complete = BTreeSet::new();
    for start in records.keys() {
        if complete.contains(start) {
            continue;
        }
        let mut visited = BTreeSet::new();
        let mut path = Vec::new();
        let mut current = start;
        loop {
            if complete.contains(current) {
                break;
            }
            if !visited.insert(current.clone()) {
                return invalid_record("domain record successor chains cannot contain cycles");
            }
            path.push(current.clone());
            let Some(DomainRecord {
                lifecycle:
                    DomainRecordLifecycle::Retired {
                        successor: Some(successor),
                        ..
                    },
                ..
            }) = records.get(current)
            else {
                break;
            };
            current = successor;
        }
        complete.extend(path);
    }
    Ok(())
}

fn require_mutable_record<'a>(
    records: &'a BTreeMap<DomainRecordRef, DomainRecord>,
    reference: &DomainRecordRef,
    expected_version: u64,
) -> Result<&'a DomainRecord, CanwuError> {
    let record = records.get(reference).ok_or_else(|| {
        CanwuError::new(
            ErrorCode::DomainRecordNotFound,
            format!("domain record {reference} was not found"),
        )
    })?;
    if record.version != expected_version {
        return Err(version_conflict(
            reference,
            expected_version,
            record.version,
        ));
    }
    if !record.is_active() {
        return invalid_record("only active domain records can be updated or retired");
    }
    Ok(record)
}

fn next_record_version(version: u64) -> Result<u64, CanwuError> {
    version.checked_add(1).ok_or_else(|| {
        CanwuError::new(
            ErrorCode::IdentifierExhausted,
            "domain record version space is exhausted",
        )
    })
}

fn version_conflict(reference: &DomainRecordRef, expected: u64, actual: u64) -> CanwuError {
    CanwuError::new(
        ErrorCode::DomainRecordVersionConflict,
        format!(
            "domain record {reference} expected version {expected}, but current version is {actual}"
        ),
    )
}

fn validate_target(target: &DomainReferenceTarget) -> Result<(), CanwuError> {
    match target {
        DomainReferenceTarget::Core(entity) => {
            if entity.core_kind().is_none() {
                return invalid_record(
                    "domain entities must use domain-record references in record fields",
                );
            }
            Ok(())
        }
        DomainReferenceTarget::Domain(reference) => validate_reference(reference),
    }
}

fn validate_reference(reference: &DomainRecordRef) -> Result<(), CanwuError> {
    validate_kind(&reference.kind)?;
    if !canonical_text(&reference.id) {
        return invalid_record("domain record IDs must be non-empty canonical strings");
    }
    Ok(())
}

fn validate_kind(kind: &DomainRecordKind) -> Result<(), CanwuError> {
    if !canonical_text(&kind.namespace) || !canonical_text(&kind.name) {
        return invalid_record("domain record kinds require canonical namespace and name values");
    }
    Ok(())
}

fn canonical_text(value: &str) -> bool {
    !value.is_empty() && value == value.trim()
}

fn invalid_record<T>(message: impl Into<String>) -> Result<T, CanwuError> {
    Err(CanwuError::new(ErrorCode::InvalidDomainRecord, message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture_kind(name: &str) -> DomainRecordKind {
        DomainRecordKind::new("fixture.records", name)
    }

    fn fixture_record(reference: DomainRecordRef, class: DomainRecordClass) -> DomainRecord {
        DomainRecord {
            reference,
            owner: "fixture".to_owned(),
            class,
            version: 1,
            lifecycle: DomainRecordLifecycle::Active,
            payload: json!(null),
            references: vec![],
        }
    }

    fn mutation_request(mutation: &DomainRecordMutation) -> DomainMutationRequest<'_> {
        DomainMutationRequest {
            plugin: "fixture",
            system: "fixture-system",
            visibility: StateVisibility::NextBoundary,
            mutation,
            summary: "fixture mutation",
        }
    }

    #[test]
    fn create_only_policy_rejects_raw_mutations_and_preserves_create() {
        let kind = fixture_kind("immutable");
        let existing_ref = DomainRecordRef::new(&kind.namespace, &kind.name, "existing");
        let existing = fixture_record(existing_ref.clone(), DomainRecordClass::Record);
        let records = BTreeMap::from([(existing_ref.clone(), existing.clone())]);
        let mut schema = DomainRecordSchema::new(kind.clone(), DomainRecordClass::Record);
        schema.mutation_policy = DomainRecordMutationPolicy::CreateOnly;
        let schemas = BTreeMap::from([(kind.clone(), ("fixture".to_owned(), schema))]);
        let core_exists = |_: &EntityRef| true;

        let update = DomainRecordMutation::Update {
            record: DomainRecordDraft::new(existing_ref.clone(), json!("changed")),
            expected_version: 1,
        };
        let retire = DomainRecordMutation::Retire {
            record: existing_ref.clone(),
            expected_version: 1,
            successor: None,
        };
        let delete = DomainRecordMutation::Delete {
            record: existing_ref.clone(),
            expected_version: 1,
        };
        for mutation in [&update, &retire, &delete] {
            let error = apply_mutation_bundle(
                &records,
                &schemas,
                SimTime::EPOCH,
                &core_exists,
                vec![mutation_request(mutation)],
            )
            .expect_err("create-only kinds must reject non-create raw mutations");
            assert_eq!(error.code, ErrorCode::InvalidDomainRecord);
            assert_eq!(records.get(&existing_ref), Some(&existing));
        }

        let new_ref = DomainRecordRef::new(&kind.namespace, &kind.name, "new");
        let create = DomainRecordMutation::Create {
            record: DomainRecordDraft::new(new_ref.clone(), json!(null)),
        };
        let (next, changes) = apply_mutation_bundle(
            &records,
            &schemas,
            SimTime::EPOCH,
            &core_exists,
            vec![mutation_request(&create)],
        )
        .expect("create-only kinds must still accept creates");
        assert!(next.contains_key(&new_ref));
        assert_eq!(changes[0].operation, DomainRecordOperation::Created);
    }

    #[test]
    fn any_entity_domain_reference_is_typed_by_registered_record_class() {
        let entity_kind = fixture_kind("future-entity");
        let value_kind = fixture_kind("future-value");
        let entity_ref = DomainRecordRef::new(&entity_kind.namespace, &entity_kind.name, "one");
        let value_ref = DomainRecordRef::new(&value_kind.namespace, &value_kind.name, "one");
        let records = BTreeMap::from([
            (
                entity_ref.clone(),
                fixture_record(entity_ref.clone(), DomainRecordClass::Entity),
            ),
            (
                value_ref.clone(),
                fixture_record(value_ref.clone(), DomainRecordClass::Record),
            ),
        ]);
        let schema = DomainReferenceSchema {
            role: "target".to_owned(),
            targets: vec![DomainReferenceTargetKind::AnyEntity],
            required: true,
            multiple: false,
            allow_retired: false,
        };
        let source = DomainRecordRef::new("fixture.records", "source", "one");

        assert!(
            validate_reference_target(
                &DomainReferenceTarget::Domain(entity_ref),
                &schema,
                &records,
                &|_: &EntityRef| false,
                &source,
            )
            .is_ok()
        );
        let error = validate_reference_target(
            &DomainReferenceTarget::Domain(value_ref),
            &schema,
            &records,
            &|_: &EntityRef| false,
            &source,
        )
        .expect_err("AnyEntity must reject domain value records");
        assert_eq!(error.code, ErrorCode::InvalidDomainRecord);
    }
}
