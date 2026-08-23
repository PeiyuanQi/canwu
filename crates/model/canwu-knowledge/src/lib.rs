//! Actor-relative information kept separate from world ground truth.

use canwu_core::{
    ArmyId, BoundaryId, DomainRecordRef, EventId, EvidenceRef, HolderKnowledgeRecordId,
    KnowledgeHolderRef, KnowledgeRecordId, KnowledgeSchemaId, PersonId, TerritoryId,
};
use canwu_time::SimTime;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const DEFAULT_KNOWLEDGE_PAGE_SIZE: u32 = 100;
pub const MAX_KNOWLEDGE_PAGE_SIZE: u32 = 1_000;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum KnowledgeSource {
    DirectObservation,
    CommandResponsibility,
    Report { source_event: EventId },
    ScenarioRecord,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EstimateRange {
    pub minimum: u32,
    pub maximum: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ArmyKnowledge {
    pub army: ArmyId,
    pub known_name: Option<String>,
    pub known_location: Option<TerritoryId>,
    pub estimated_strength: EstimateRange,
    pub observed_at: SimTime,
    pub learned_at: SimTime,
    pub confidence_per_mille: u16,
    pub source: KnowledgeSource,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ActorKnowledge {
    pub actor: PersonId,
    pub armies: BTreeMap<ArmyId, ArmyKnowledge>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum KnowledgeSubjectTarget {
    Entity(canwu_core::EntityRef),
    DomainRecord(DomainRecordRef),
    Event(EventId),
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct KnowledgeSubject {
    pub role: String,
    pub target: KnowledgeSubjectTarget,
}

/// Compatibility name for knowledge-origin evidence. The persisted type is
/// shared by every evidence-producing subsystem in `canwu-core`.
pub type KnowledgeEvidenceRef = EvidenceRef;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KnowledgeOrigin {
    pub method: String,
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KnowledgeRecordDraft {
    pub schema: KnowledgeSchemaId,
    pub subjects: Vec<KnowledgeSubject>,
    pub payload: Value,
    pub as_of: Option<SimTime>,
    pub confidence_per_mille: u16,
    pub origin: KnowledgeOrigin,
    pub supersedes: Vec<KnowledgeRecordId>,
    pub contradicts: Vec<KnowledgeRecordId>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KnowledgeRecord {
    pub id: KnowledgeRecordId,
    pub holder: KnowledgeHolderRef,
    pub schema: KnowledgeSchemaId,
    pub subjects: Vec<KnowledgeSubject>,
    pub payload: Value,
    pub as_of: Option<SimTime>,
    pub learned_at: SimTime,
    pub confidence_per_mille: u16,
    pub origin: KnowledgeOrigin,
    pub supersedes: Vec<KnowledgeRecordId>,
    pub contradicts: Vec<KnowledgeRecordId>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KnowledgeRecordView {
    pub id: HolderKnowledgeRecordId,
    pub holder: KnowledgeHolderRef,
    pub schema: KnowledgeSchemaId,
    pub subjects: Vec<KnowledgeSubject>,
    pub payload: Value,
    pub as_of: Option<SimTime>,
    pub learned_at: SimTime,
    pub confidence_per_mille: u16,
    pub supersedes: Vec<HolderKnowledgeRecordId>,
    pub contradicts: Vec<HolderKnowledgeRecordId>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeHistoryView {
    CurrentHeads,
    FullHistory,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KnowledgeReadCut {
    pub boundary: Option<BoundaryId>,
    pub holder_projection_root: String,
    pub holder_overlay_root: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KnowledgeCursor {
    pub holder: KnowledgeHolderRef,
    pub query_hash: String,
    pub read_cut: KnowledgeReadCut,
    pub binding_hash: String,
    pub learned_at: SimTime,
    pub record: HolderKnowledgeRecordId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KnowledgeQuery {
    #[serde(default)]
    pub schemas: Vec<KnowledgeSchemaId>,
    #[serde(default)]
    pub subjects: Vec<KnowledgeSubject>,
    pub learned_after: Option<SimTime>,
    pub learned_at_or_before: Option<SimTime>,
    pub view: KnowledgeHistoryView,
    pub after: Option<KnowledgeCursor>,
    pub limit: u32,
}

impl Default for KnowledgeQuery {
    fn default() -> Self {
        Self {
            schemas: Vec::new(),
            subjects: Vec::new(),
            learned_after: None,
            learned_at_or_before: None,
            view: KnowledgeHistoryView::CurrentHeads,
            after: None,
            limit: DEFAULT_KNOWLEDGE_PAGE_SIZE,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KnowledgeQueryResult {
    pub holder: KnowledgeHolderRef,
    pub read_cut: KnowledgeReadCut,
    pub records: Vec<KnowledgeRecordView>,
    pub next: Option<KnowledgeCursor>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
/// Holder-relative generic knowledge ledger used by the format-5 snapshot.
pub struct GenericKnowledgeLedger {
    pub records: BTreeMap<KnowledgeHolderRef, BTreeMap<KnowledgeRecordId, KnowledgeRecord>>,
}

/// Atomic append validation failures for the standalone generic ledger.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KnowledgeLedgerError {
    HolderMismatch,
    DuplicateRecordId,
}

impl GenericKnowledgeLedger {
    #[must_use]
    pub fn for_holder(
        &self,
        holder: &KnowledgeHolderRef,
    ) -> Option<&BTreeMap<KnowledgeRecordId, KnowledgeRecord>> {
        self.records.get(holder)
    }

    /// Atomically appends records after validating holder and global ID invariants.
    ///
    /// # Errors
    ///
    /// Returns [`KnowledgeLedgerError::HolderMismatch`] when any record names a
    /// different holder, or [`KnowledgeLedgerError::DuplicateRecordId`] when an
    /// ID already exists anywhere in the ledger or repeats within the batch.
    pub fn insert_records(
        &mut self,
        holder: KnowledgeHolderRef,
        records: impl IntoIterator<Item = KnowledgeRecord>,
    ) -> Result<(), KnowledgeLedgerError> {
        let records = records.into_iter().collect::<Vec<_>>();
        let mut incoming_ids = BTreeSet::new();
        for record in &records {
            if record.holder != holder {
                return Err(KnowledgeLedgerError::HolderMismatch);
            }
            if !incoming_ids.insert(record.id)
                || self
                    .records
                    .values()
                    .any(|existing| existing.contains_key(&record.id))
            {
                return Err(KnowledgeLedgerError::DuplicateRecordId);
            }
        }
        let entry = self.records.entry(holder).or_default();
        for record in records {
            entry.insert(record.id, record);
        }
        Ok(())
    }

    /// Returns one deterministic page from a holder's ledger at the supplied cut.
    ///
    /// # Errors
    ///
    /// Returns [`KnowledgeQueryError::InvalidLimit`] for a zero or oversized
    /// page and [`KnowledgeQueryError::InvalidCursor`] when a cursor belongs to
    /// another holder, query, or ledger position. It returns
    /// [`KnowledgeQueryError::ReadCutUnavailable`] when the cursor's committed
    /// knowledge or overlay root is no longer the supplied view. It returns
    /// [`KnowledgeQueryError::InvalidLedger`] for internally inconsistent stored
    /// records and [`KnowledgeQueryError::Encoding`] if query hashing fails.
    pub fn query(
        &self,
        holder: KnowledgeHolderRef,
        query: &KnowledgeQuery,
        read_cut: KnowledgeReadCut,
    ) -> Result<KnowledgeQueryResult, KnowledgeQueryError> {
        query_records(&self.records, holder, query, read_cut)
    }
}

fn query_records(
    ledger: &BTreeMap<KnowledgeHolderRef, BTreeMap<KnowledgeRecordId, KnowledgeRecord>>,
    holder: KnowledgeHolderRef,
    query: &KnowledgeQuery,
    read_cut: KnowledgeReadCut,
) -> Result<KnowledgeQueryResult, KnowledgeQueryError> {
    if query.limit == 0 || query.limit > MAX_KNOWLEDGE_PAGE_SIZE {
        return Err(KnowledgeQueryError::InvalidLimit);
    }
    let query_hash = query_hash(query)?;
    let binding_hash = cursor_binding_hash(&holder, &query_hash, &read_cut)?;
    if let Some(cursor) = &query.after {
        validate_cursor(cursor, &holder, &query_hash, &read_cut)?;
    }

    let Some(records) = ledger.get(&holder) else {
        if query.after.is_some() {
            return Err(KnowledgeQueryError::InvalidCursor);
        }
        return Ok(KnowledgeQueryResult {
            holder,
            read_cut,
            records: Vec::new(),
            next: None,
        });
    };
    if records.iter().any(|(id, record)| {
        *id != record.id
            || record.id.get() == 0
            || record.holder != holder
            || record
                .supersedes
                .iter()
                .chain(&record.contradicts)
                .any(|related| !records.contains_key(related))
    }) {
        return Err(KnowledgeQueryError::InvalidLedger);
    }
    let local_ids = holder_local_ids(records);
    if let Some(cursor) = &query.after
        && !records.iter().any(|(id, record)| {
            local_ids[id] == cursor.record && record.learned_at == cursor.learned_at
        })
    {
        return Err(KnowledgeQueryError::InvalidCursor);
    }
    let current = if query.view == KnowledgeHistoryView::CurrentHeads {
        current_heads(records)
    } else {
        records.keys().copied().collect()
    };
    let mut candidates = records
        .iter()
        .filter(|(id, record)| {
            current.contains(id)
                && (query.schemas.is_empty() || query.schemas.contains(&record.schema))
                && query
                    .subjects
                    .iter()
                    .all(|subject| record.subjects.contains(subject))
                && query
                    .learned_after
                    .is_none_or(|time| record.learned_at > time)
                && query
                    .learned_at_or_before
                    .is_none_or(|time| record.learned_at <= time)
        })
        .map(|(id, record)| (*id, record))
        .collect::<Vec<_>>();
    candidates.sort_by_key(|(_, record)| (record.learned_at, record.id));

    if let Some(cursor) = &query.after {
        candidates.retain(|(id, record)| {
            (record.learned_at, local_ids[id]) > (cursor.learned_at, cursor.record)
        });
    }

    let limit = usize::try_from(query.limit).unwrap_or(MAX_KNOWLEDGE_PAGE_SIZE as usize);
    let has_more = candidates.len() > limit;
    let page = candidates.into_iter().take(limit).collect::<Vec<_>>();
    let next = if has_more {
        page.last().map(|(_, record)| KnowledgeCursor {
            holder: holder.clone(),
            query_hash: query_hash.clone(),
            read_cut: read_cut.clone(),
            binding_hash: binding_hash.clone(),
            learned_at: record.learned_at,
            record: local_ids[&record.id],
        })
    } else {
        None
    };
    let views = page
        .iter()
        .map(|(id, record)| to_view(record, local_ids[id], &local_ids))
        .collect();
    Ok(KnowledgeQueryResult {
        holder,
        read_cut,
        records: views,
        next,
    })
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct KnowledgeSnapshot {
    pub actors: BTreeMap<PersonId, ActorKnowledge>,
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        with = "holder_records_wire"
    )]
    pub records: BTreeMap<KnowledgeHolderRef, BTreeMap<KnowledgeRecordId, KnowledgeRecord>>,
}

mod holder_records_wire {
    use super::{BTreeMap, BTreeSet, KnowledgeHolderRef, KnowledgeRecord, KnowledgeRecordId};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Deserialize, Serialize)]
    struct HolderLedgerEntry {
        holder: KnowledgeHolderRef,
        records: Vec<KnowledgeRecord>,
    }

    pub fn serialize<S>(
        value: &BTreeMap<KnowledgeHolderRef, BTreeMap<KnowledgeRecordId, KnowledgeRecord>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut global_ids = BTreeSet::new();
        let mut entries = Vec::with_capacity(value.len());
        for (holder, records) in value {
            let mut ordered = Vec::with_capacity(records.len());
            for (id, record) in records {
                if id != &record.id || &record.holder != holder || !global_ids.insert(*id) {
                    return Err(serde::ser::Error::custom(
                        "knowledge snapshot contains inconsistent holders or record IDs",
                    ));
                }
                ordered.push(record.clone());
            }
            entries.push(HolderLedgerEntry {
                holder: holder.clone(),
                records: ordered,
            });
        }
        entries.serialize(serializer)
    }

    pub fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<BTreeMap<KnowledgeHolderRef, BTreeMap<KnowledgeRecordId, KnowledgeRecord>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let entries = Vec::<HolderLedgerEntry>::deserialize(deserializer)?;
        let mut ledger = BTreeMap::new();
        let mut global_ids = BTreeSet::new();
        for entry in entries {
            let holder = entry.holder;
            let mut records = BTreeMap::new();
            for record in entry.records {
                if record.holder != holder
                    || !global_ids.insert(record.id)
                    || records.insert(record.id, record).is_some()
                {
                    return Err(serde::de::Error::custom(
                        "holder ledger contains a mismatched holder or duplicate record ID",
                    ));
                }
            }
            if ledger.insert(holder, records).is_some() {
                return Err(serde::de::Error::custom(
                    "knowledge snapshot contains a duplicate holder ledger",
                ));
            }
        }
        Ok(ledger)
    }
}

impl KnowledgeSnapshot {
    /// Counts all retained holder-relative records in one schema namespace.
    #[must_use]
    pub fn record_count_in_namespace(&self, namespace: &str) -> usize {
        self.records
            .values()
            .flat_map(BTreeMap::values)
            .filter(|record| record.schema.kind.namespace == namespace)
            .count()
    }

    #[must_use]
    pub fn for_actor(&self, actor: PersonId) -> Option<&ActorKnowledge> {
        self.actors.get(&actor)
    }

    #[must_use]
    pub fn for_holder(
        &self,
        holder: &KnowledgeHolderRef,
    ) -> Option<&BTreeMap<KnowledgeRecordId, KnowledgeRecord>> {
        self.records.get(holder)
    }

    /// Returns one deterministic holder-facing page at the supplied read cut.
    ///
    /// # Errors
    ///
    /// Returns the same validation errors as [`GenericKnowledgeLedger::query`].
    pub fn query(
        &self,
        holder: KnowledgeHolderRef,
        query: &KnowledgeQuery,
        read_cut: KnowledgeReadCut,
    ) -> Result<KnowledgeQueryResult, KnowledgeQueryError> {
        query_records(&self.records, holder, query, read_cut)
    }

    /// Queries the current settled holder projection using an engine-derived
    /// read cut. Callers cannot substitute a cross-holder or stale root.
    ///
    /// # Errors
    ///
    /// Returns an error when the holder ledger is inconsistent or the query or
    /// cursor does not match the derived read cut.
    pub fn query_current(
        &self,
        holder: KnowledgeHolderRef,
        query: &KnowledgeQuery,
        boundary: Option<BoundaryId>,
    ) -> Result<KnowledgeQueryResult, KnowledgeQueryError> {
        let read_cut = KnowledgeReadCut {
            boundary,
            holder_projection_root: holder_projection_root(&holder, &self.records)?,
            holder_overlay_root: None,
        };
        query_records(&self.records, holder, query, read_cut)
    }

    /// Queries an omniscient system view with a same-boundary holder overlay.
    /// The returned values are owned so no mutable runtime ledger is exposed.
    ///
    /// # Errors
    ///
    /// Returns an error when the overlay collides with a settled record ID,
    /// when either ledger is inconsistent, or when the query or cursor does not
    /// match the derived read cut.
    pub fn query_with_overlay(
        &self,
        holder: KnowledgeHolderRef,
        query: &KnowledgeQuery,
        boundary: Option<BoundaryId>,
        overlay: &BTreeMap<KnowledgeHolderRef, BTreeMap<KnowledgeRecordId, KnowledgeRecord>>,
    ) -> Result<KnowledgeQueryResult, KnowledgeQueryError> {
        let mut merged = self.records.clone();
        if let Some(records) = overlay.get(&holder) {
            let entry = merged.entry(holder.clone()).or_default();
            for (id, record) in records {
                if entry.insert(*id, record.clone()).is_some() {
                    return Err(KnowledgeQueryError::InvalidLedger);
                }
            }
        }
        let read_cut = KnowledgeReadCut {
            boundary,
            holder_projection_root: holder_projection_root(&holder, &self.records)?,
            holder_overlay_root: Some(holder_overlay_root(&holder, &merged, overlay)?),
        };
        query_records(&merged, holder, query, read_cut)
    }
}

#[derive(Serialize)]
struct HolderProjectionCutMaterial<'a> {
    holder: &'a KnowledgeHolderRef,
    full_history_views: Vec<KnowledgeRecordView>,
}

#[derive(Serialize)]
struct HolderOverlayCutMaterial<'a> {
    holder: &'a KnowledgeHolderRef,
    visible_projected_records: Vec<KnowledgeRecordView>,
}

fn holder_projection_root(
    holder: &KnowledgeHolderRef,
    ledger: &BTreeMap<KnowledgeHolderRef, BTreeMap<KnowledgeRecordId, KnowledgeRecord>>,
) -> Result<String, KnowledgeQueryError> {
    let records = ledger.get(holder).cloned().unwrap_or_default();
    let local_ids = holder_local_ids(&records);
    let views = records
        .values()
        .map(|record| to_view(record, local_ids[&record.id], &local_ids))
        .collect();
    hash_material(
        b"canwu.knowledge.holder-projection.v1",
        &HolderProjectionCutMaterial {
            holder,
            full_history_views: views,
        },
    )
}

fn holder_overlay_root(
    holder: &KnowledgeHolderRef,
    merged: &BTreeMap<KnowledgeHolderRef, BTreeMap<KnowledgeRecordId, KnowledgeRecord>>,
    overlay: &BTreeMap<KnowledgeHolderRef, BTreeMap<KnowledgeRecordId, KnowledgeRecord>>,
) -> Result<String, KnowledgeQueryError> {
    let merged_records = merged.get(holder).cloned().unwrap_or_default();
    let local_ids = holder_local_ids(&merged_records);
    let views = overlay
        .get(holder)
        .into_iter()
        .flat_map(|records| records.values())
        .map(|record| to_view(record, local_ids[&record.id], &local_ids))
        .collect();
    hash_material(
        b"canwu.knowledge.holder-overlay.v1",
        &HolderOverlayCutMaterial {
            holder,
            visible_projected_records: views,
        },
    )
}

fn hash_material<T: Serialize>(domain: &[u8], value: &T) -> Result<String, KnowledgeQueryError> {
    let bytes = serde_json::to_vec(value).map_err(|_| KnowledgeQueryError::Encoding)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain);
    hasher.update(&[0]);
    hasher.update(&bytes);
    Ok(hasher.finalize().to_hex().to_string())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KnowledgeQueryError {
    InvalidLimit,
    InvalidCursor,
    ReadCutUnavailable,
    InvalidLedger,
    Encoding,
}

#[derive(Serialize)]
struct KnowledgeQueryHashMaterial {
    schemas: Vec<KnowledgeSchemaId>,
    subjects: Vec<KnowledgeSubject>,
    learned_after: Option<SimTime>,
    learned_at_or_before: Option<SimTime>,
    view: KnowledgeHistoryView,
}

fn query_hash(query: &KnowledgeQuery) -> Result<String, KnowledgeQueryError> {
    let mut schemas = query.schemas.clone();
    schemas.sort();
    schemas.dedup();
    let mut subjects = query.subjects.clone();
    subjects.sort();
    subjects.dedup();
    let material = KnowledgeQueryHashMaterial {
        schemas,
        subjects,
        learned_after: query.learned_after,
        learned_at_or_before: query.learned_at_or_before,
        view: query.view,
    };
    let bytes = serde_json::to_vec(&material).map_err(|_| KnowledgeQueryError::Encoding)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"canwu.knowledge.query.v1");
    hasher.update(&[0]);
    hasher.update(&bytes);
    Ok(hasher.finalize().to_hex().to_string())
}

#[derive(Serialize)]
struct KnowledgeCursorBindingMaterial<'a> {
    holder: &'a KnowledgeHolderRef,
    query_hash: &'a str,
    read_cut: &'a KnowledgeReadCut,
}

fn cursor_binding_hash(
    holder: &KnowledgeHolderRef,
    query_hash: &str,
    read_cut: &KnowledgeReadCut,
) -> Result<String, KnowledgeQueryError> {
    let bytes = serde_json::to_vec(&KnowledgeCursorBindingMaterial {
        holder,
        query_hash,
        read_cut,
    })
    .map_err(|_| KnowledgeQueryError::Encoding)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"canwu.knowledge.cursor.v1");
    hasher.update(&[0]);
    hasher.update(&bytes);
    Ok(hasher.finalize().to_hex().to_string())
}

fn validate_cursor(
    cursor: &KnowledgeCursor,
    holder: &KnowledgeHolderRef,
    query_hash: &str,
    read_cut: &KnowledgeReadCut,
) -> Result<(), KnowledgeQueryError> {
    if cursor.binding_hash
        != cursor_binding_hash(&cursor.holder, &cursor.query_hash, &cursor.read_cut)?
    {
        return Err(KnowledgeQueryError::InvalidCursor);
    }
    if cursor.read_cut != *read_cut {
        return Err(KnowledgeQueryError::ReadCutUnavailable);
    }
    if cursor.holder != *holder || cursor.query_hash != query_hash {
        return Err(KnowledgeQueryError::InvalidCursor);
    }
    Ok(())
}

fn holder_local_ids(
    records: &BTreeMap<KnowledgeRecordId, KnowledgeRecord>,
) -> BTreeMap<KnowledgeRecordId, HolderKnowledgeRecordId> {
    records
        .keys()
        .enumerate()
        .map(|(index, id)| {
            (
                *id,
                HolderKnowledgeRecordId::new(u64::try_from(index + 1).unwrap_or(u64::MAX)),
            )
        })
        .collect()
}

fn current_heads(
    records: &BTreeMap<KnowledgeRecordId, KnowledgeRecord>,
) -> std::collections::BTreeSet<KnowledgeRecordId> {
    let mut superseded = std::collections::BTreeSet::new();
    for record in records.values() {
        superseded.extend(record.supersedes.iter().copied());
    }
    records
        .keys()
        .filter(|id| !superseded.contains(id))
        .copied()
        .collect()
}

fn to_view(
    record: &KnowledgeRecord,
    local_id: HolderKnowledgeRecordId,
    local_ids: &BTreeMap<KnowledgeRecordId, HolderKnowledgeRecordId>,
) -> KnowledgeRecordView {
    KnowledgeRecordView {
        id: local_id,
        holder: record.holder.clone(),
        schema: record.schema.clone(),
        subjects: record.subjects.clone(),
        payload: record.payload.clone(),
        as_of: record.as_of,
        learned_at: record.learned_at,
        confidence_per_mille: record.confidence_per_mille,
        supersedes: record
            .supersedes
            .iter()
            .filter_map(|id| local_ids.get(id).copied())
            .collect(),
        contradicts: record
            .contradicts
            .iter()
            .filter_map(|id| local_ids.get(id).copied())
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canwu_core::{KnowledgeRecordKind, KnowledgeSchemaId, OrganizationId};
    use serde_json::json;

    fn schema() -> KnowledgeSchemaId {
        KnowledgeSchemaId::new(KnowledgeRecordKind::new("fixture.knowledge", "claim"), 1)
    }

    fn record(
        holder: &KnowledgeHolderRef,
        id: u64,
        learned_at: SimTime,
        supersedes: Vec<KnowledgeRecordId>,
        contradicts: Vec<KnowledgeRecordId>,
    ) -> KnowledgeRecord {
        KnowledgeRecord {
            id: KnowledgeRecordId::new(id),
            holder: holder.clone(),
            schema: schema(),
            subjects: vec![],
            payload: json!({ "value": id }),
            as_of: None,
            learned_at,
            confidence_per_mille: 900,
            origin: KnowledgeOrigin {
                method: "fixture".to_owned(),
                evidence: vec![],
            },
            supersedes,
            contradicts,
        }
    }

    fn history_query(
        schemas: Vec<KnowledgeSchemaId>,
        limit: u32,
        after: Option<KnowledgeCursor>,
    ) -> KnowledgeQuery {
        KnowledgeQuery {
            schemas,
            limit,
            view: KnowledgeHistoryView::FullHistory,
            after,
            ..KnowledgeQuery::default()
        }
    }

    fn read_cut(root: &str) -> KnowledgeReadCut {
        KnowledgeReadCut {
            boundary: Some(BoundaryId::new(2)),
            holder_projection_root: root.to_owned(),
            holder_overlay_root: None,
        }
    }

    #[test]
    fn current_heads_and_full_history_are_distinct() {
        let holder =
            KnowledgeHolderRef::Entity(canwu_core::EntityRef::Organization(OrganizationId::new(4)));
        let mut ledger = GenericKnowledgeLedger::default();
        ledger
            .insert_records(
                holder.clone(),
                [
                    record(&holder, 1, SimTime::from_minutes(1), vec![], vec![]),
                    record(
                        &holder,
                        2,
                        SimTime::from_minutes(2),
                        vec![KnowledgeRecordId::new(1)],
                        vec![],
                    ),
                    record(
                        &holder,
                        3,
                        SimTime::from_minutes(3),
                        vec![],
                        vec![KnowledgeRecordId::new(1)],
                    ),
                ],
            )
            .expect("fixture records should be admitted");

        let current = ledger
            .query(
                holder.clone(),
                &KnowledgeQuery::default(),
                read_cut("holder-root-1"),
            )
            .expect("current-head query should succeed");
        assert_eq!(
            current
                .records
                .iter()
                .map(|record| record.payload["value"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            vec![2, 3]
        );
        assert!(current.records.iter().all(|record| record.id.get() > 0));
        let holder_view = serde_json::to_value(&current.records[0])
            .expect("holder-facing record should serialize");
        assert!(holder_view.get("origin").is_none());
        assert!(holder_view.get("evidence").is_none());

        let history = ledger
            .query(
                holder,
                &KnowledgeQuery {
                    view: KnowledgeHistoryView::FullHistory,
                    ..KnowledgeQuery::default()
                },
                read_cut("holder-root-1"),
            )
            .expect("history query should succeed");
        assert_eq!(history.records.len(), 3);
    }

    #[test]
    fn holder_local_ids_hide_global_gaps_and_cursor_is_stable() {
        let holder = KnowledgeHolderRef::Person(PersonId::new(9));
        let mut ledger = GenericKnowledgeLedger::default();
        ledger
            .insert_records(
                holder.clone(),
                [
                    record(&holder, 10, SimTime::from_minutes(1), vec![], vec![]),
                    record(&holder, 42, SimTime::from_minutes(1), vec![], vec![]),
                ],
            )
            .expect("fixture records should be admitted");
        let cut = read_cut("holder-root-4");
        let first = ledger
            .query(
                holder.clone(),
                &history_query(vec![schema(), schema()], 1, None),
                cut.clone(),
            )
            .expect("first page should succeed");
        assert_eq!(first.records[0].id.get(), 1);
        let cursor = first.next.clone().expect("first page should have a cursor");
        let mut forged_cursor = cursor.clone();
        forged_cursor.binding_hash = "forged".to_owned();
        assert_eq!(
            ledger.query(
                holder.clone(),
                &history_query(vec![schema()], 2, Some(forged_cursor)),
                cut.clone(),
            ),
            Err(KnowledgeQueryError::InvalidCursor)
        );
        let second = ledger
            .query(
                holder.clone(),
                &history_query(vec![schema()], 2, Some(cursor.clone())),
                cut.clone(),
            )
            .expect("second page should succeed");
        assert_eq!(second.records[0].id.get(), 2);

        assert_eq!(
            ledger.query(
                holder,
                &history_query(vec![schema()], 2, Some(cursor)),
                read_cut("newer-holder-root"),
            ),
            Err(KnowledgeQueryError::ReadCutUnavailable)
        );
    }

    #[test]
    fn generic_ledger_admission_is_atomic_and_globally_append_only() {
        let first_holder = KnowledgeHolderRef::Person(PersonId::new(1));
        let second_holder = KnowledgeHolderRef::Person(PersonId::new(2));
        let mut ledger = GenericKnowledgeLedger::default();
        ledger
            .insert_records(
                first_holder.clone(),
                [record(
                    &first_holder,
                    1,
                    SimTime::from_minutes(1),
                    vec![],
                    vec![],
                )],
            )
            .expect("the first global ID should be admitted");

        let before = ledger.clone();
        assert_eq!(
            ledger.insert_records(
                first_holder.clone(),
                [record(
                    &second_holder,
                    2,
                    SimTime::from_minutes(2),
                    vec![],
                    vec![],
                )],
            ),
            Err(KnowledgeLedgerError::HolderMismatch)
        );
        assert_eq!(ledger, before);

        assert_eq!(
            ledger.insert_records(
                second_holder.clone(),
                [record(
                    &second_holder,
                    1,
                    SimTime::from_minutes(2),
                    vec![],
                    vec![],
                )],
            ),
            Err(KnowledgeLedgerError::DuplicateRecordId)
        );
        assert_eq!(ledger, before);

        assert_eq!(
            ledger.insert_records(
                second_holder.clone(),
                [
                    record(&second_holder, 2, SimTime::from_minutes(2), vec![], vec![],),
                    record(&second_holder, 2, SimTime::from_minutes(3), vec![], vec![],),
                ],
            ),
            Err(KnowledgeLedgerError::DuplicateRecordId)
        );
        assert_eq!(ledger, before);
    }

    #[test]
    fn empty_snapshot_preserves_wire_and_holder_ledgers_are_canonical() {
        assert_eq!(
            serde_json::to_value(KnowledgeSnapshot::default())
                .expect("empty knowledge should serialize"),
            json!({ "actors": {} })
        );

        let first_holder = KnowledgeHolderRef::Person(PersonId::new(1));
        let second_holder = KnowledgeHolderRef::Person(PersonId::new(2));
        let mut ledger = GenericKnowledgeLedger::default();
        ledger
            .insert_records(
                second_holder.clone(),
                [record(
                    &second_holder,
                    3,
                    SimTime::from_minutes(3),
                    vec![],
                    vec![],
                )],
            )
            .expect("second holder fixture should be admitted");
        ledger
            .insert_records(
                first_holder.clone(),
                [
                    record(&first_holder, 2, SimTime::from_minutes(2), vec![], vec![]),
                    record(&first_holder, 1, SimTime::from_minutes(1), vec![], vec![]),
                ],
            )
            .expect("first holder fixture should be admitted");
        let snapshot = KnowledgeSnapshot {
            actors: BTreeMap::new(),
            records: ledger.records,
        };

        let encoded = serde_json::to_value(&snapshot).expect("knowledge snapshot should serialize");
        assert_eq!(
            encoded["records"][0]["holder"],
            json!({ "type": "person", "value": 1 })
        );
        assert_eq!(encoded["records"][0]["records"][0]["id"], json!(1));
        assert_eq!(encoded["records"][0]["records"][1]["id"], json!(2));
        assert_eq!(
            serde_json::from_value::<KnowledgeSnapshot>(encoded.clone())
                .expect("canonical holder ledger should deserialize"),
            snapshot
        );

        let mut duplicate_global_id = encoded;
        duplicate_global_id["records"][1]["records"][0]["id"] = json!(1);
        assert!(serde_json::from_value::<KnowledgeSnapshot>(duplicate_global_id).is_err());

        let mut inconsistent = snapshot;
        let record = inconsistent
            .records
            .get_mut(&first_holder)
            .expect("first holder exists")
            .remove(&KnowledgeRecordId::new(1))
            .expect("record exists");
        inconsistent
            .records
            .get_mut(&first_holder)
            .expect("first holder exists")
            .insert(KnowledgeRecordId::new(9), record);
        assert!(serde_json::to_value(inconsistent).is_err());
    }

    #[test]
    fn successor_holders_do_not_inherit_and_page_limit_is_closed() {
        let retired = KnowledgeHolderRef::Entity(canwu_core::EntityRef::Domain(
            DomainRecordRef::new("fixture.organization", "office", "retired"),
        ));
        let successor = KnowledgeHolderRef::Entity(canwu_core::EntityRef::Domain(
            DomainRecordRef::new("fixture.organization", "office", "successor"),
        ));
        let mut ledger = GenericKnowledgeLedger::default();
        ledger
            .insert_records(
                retired.clone(),
                [record(
                    &retired,
                    1,
                    SimTime::from_minutes(1),
                    vec![],
                    vec![],
                )],
            )
            .expect("retired holder history should remain addressable");
        assert!(ledger.for_holder(&successor).is_none());

        assert!(
            ledger
                .query(
                    successor.clone(),
                    &KnowledgeQuery {
                        limit: 1_000,
                        ..KnowledgeQuery::default()
                    },
                    read_cut("successor-root"),
                )
                .is_ok()
        );
        assert_eq!(
            ledger.query(
                successor,
                &KnowledgeQuery {
                    limit: 1_001,
                    ..KnowledgeQuery::default()
                },
                read_cut("successor-root"),
            ),
            Err(KnowledgeQueryError::InvalidLimit)
        );
    }
}
