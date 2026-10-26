//! Actor-relative information kept separate from world ground truth.

use canwu_core::{
    ArmyId, BoundaryId, DomainRecordRef, EventId, EvidenceRef, HolderKnowledgeRecordId,
    KnowledgeHolderRef, KnowledgeRecordId, KnowledgeSchemaId, PersonId, TerritoryId,
};
use canwu_time::SimTime;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

mod encoding;
mod ledger;

pub use encoding::{KnowledgeBodyDigest, KnowledgeEncodingError};
pub use ledger::{
    BodiesById, CurrentRecords, HolderEntriesIter, HolderLedger, HolderLedgers, HolderLedgersIter,
    HolderRecordsIter, KNOWLEDGE_COMMITMENT_CHUNK_ENTRIES,
};

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

/// One holder's record as a reader gets it: the fields of its body with
/// the holder and the relations of the holder's entry. The ledgers store the
/// body once and an entry per holder; see [`KnowledgeRecordBody`] and
/// [`HolderEntry`].
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

impl KnowledgeRecord {
    /// The record `holder` holds through `entry`, assembled with its body.
    #[must_use]
    pub fn assemble(
        holder: &KnowledgeHolderRef,
        entry: &HolderEntry,
        body: &KnowledgeRecordBody,
    ) -> Self {
        Self {
            id: body.id,
            holder: holder.clone(),
            schema: body.schema.clone(),
            subjects: body.subjects.clone(),
            payload: body.payload.clone(),
            as_of: body.as_of,
            learned_at: body.learned_at,
            confidence_per_mille: body.confidence_per_mille,
            origin: body.origin.clone(),
            supersedes: entry.supersedes.clone(),
            contradicts: entry.contradicts.clone(),
        }
    }

    /// The record split into its holder, its body, and the relations of its
    /// entry (`supersedes`, then `contradicts`).
    #[must_use]
    pub fn into_parts(
        self,
    ) -> (
        KnowledgeHolderRef,
        KnowledgeRecordBody,
        Vec<KnowledgeRecordId>,
        Vec<KnowledgeRecordId>,
    ) {
        (
            self.holder,
            KnowledgeRecordBody {
                id: self.id,
                schema: self.schema,
                subjects: self.subjects,
                payload: self.payload,
                as_of: self.as_of,
                learned_at: self.learned_at,
                confidence_per_mille: self.confidence_per_mille,
                origin: self.origin,
            },
            self.supersedes,
            self.contradicts,
        )
    }
}

/// What one publication stores once, whichever holders hold it: everything
/// of a record but its holder and relations.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KnowledgeRecordBody {
    /// Global ID, allocated in staging order.
    pub id: KnowledgeRecordId,
    pub schema: KnowledgeSchemaId,
    pub subjects: Vec<KnowledgeSubject>,
    pub payload: Value,
    pub as_of: Option<SimTime>,
    pub learned_at: SimTime,
    pub confidence_per_mille: u16,
    pub origin: KnowledgeOrigin,
}

/// One holder's hold on one body: the holder's own local ID for it, from
/// the holder's counter, and the holder's own relations, which name global
/// IDs of entries of the same holder and schema kind.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HolderEntry {
    pub record: KnowledgeRecordId,
    pub local: HolderKnowledgeRecordId,
    pub supersedes: Vec<KnowledgeRecordId>,
    pub contradicts: Vec<KnowledgeRecordId>,
}

/// When an entry leaves its holder, as a knowledge schema declares it.
#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeRetention {
    /// Every entry stays for the life of the run.
    #[default]
    Keep,
    /// An entry leaves its holder in the commit of the boundary that
    /// publishes an entry of the same holder superseding it; its body leaves
    /// when no holder holds it.
    RetireWhenSuperseded,
}

/// Entries a settling boundary staged, by holder and global ID, each with its
/// body: the same-boundary overlay a boundary system's reads see beside the
/// committed ledgers.
pub type StagedKnowledgeEntries = BTreeMap<
    KnowledgeHolderRef,
    BTreeMap<KnowledgeRecordId, (Arc<KnowledgeRecordBody>, HolderEntry)>,
>;

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
    /// The retention of each schema the query is about: every schema the
    /// query names when it names any, otherwise every schema of the returned
    /// records. A schema the run does not register is left out. Under
    /// [`KnowledgeRetention::RetireWhenSuperseded`] a holder keeps only
    /// current entries, so a `FullHistory` view of that schema shows what a
    /// `CurrentHeads` view shows. The runtime fills it; a standalone ledger,
    /// which knows no schema registry, leaves it empty.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub retention: BTreeMap<KnowledgeSchemaId, KnowledgeRetention>,
}

impl KnowledgeQueryResult {
    /// Fills [`Self::retention`] for `query` from each schema's registered
    /// policy, `None` for a schema the run does not register.
    #[must_use]
    pub fn with_retention(
        mut self,
        query: &KnowledgeQuery,
        policy: impl Fn(&KnowledgeSchemaId) -> Option<KnowledgeRetention>,
    ) -> Self {
        let schemas: Vec<&KnowledgeSchemaId> = if query.schemas.is_empty() {
            self.records.iter().map(|record| &record.schema).collect()
        } else {
            query.schemas.iter().collect()
        };
        let retention = schemas
            .into_iter()
            .filter_map(|schema| policy(schema).map(|retention| (schema.clone(), retention)))
            .collect();
        self.retention = retention;
        self
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
/// Holder-relative generic knowledge ledger used by the current snapshot.
pub struct GenericKnowledgeLedger {
    /// Every holder's records; see [`HolderLedgers`].
    pub records: HolderLedgers,
}

/// Why the ledgers refused an insertion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KnowledgeLedgerError {
    /// An entry names a different holder from the one it is filed under.
    HolderMismatch,
    /// A body ID is already stored or repeats, or a holder would hold one
    /// ID twice.
    DuplicateRecordId,
    /// An entry names a body that is not stored.
    MissingBody,
    /// A published body received no entry.
    UnheldBody,
    /// A local ID is zero or not above the holder's previous one.
    LocalOutOfOrder,
    /// A relation names an entry its holder does not hold.
    UnresolvedRelation,
    /// A body could not be encoded.
    Encoding,
}

impl GenericKnowledgeLedger {
    /// One holder's records, or `None` when it holds none.
    #[must_use]
    pub fn for_holder(&self, holder: &KnowledgeHolderRef) -> Option<&HolderLedger> {
        self.records.get(holder)
    }

    /// Atomically stores one body and gives it to each listed holder, as
    /// [`HolderLedgers::insert_body`] does, computing the body's digest.
    ///
    /// # Errors
    ///
    /// [`KnowledgeLedgerError::Encoding`] when the body cannot be encoded,
    /// and the errors of [`HolderLedgers::insert_publication`].
    pub fn insert_records(
        &mut self,
        body: Arc<KnowledgeRecordBody>,
        entries: impl IntoIterator<Item = (KnowledgeHolderRef, HolderEntry)>,
    ) -> Result<(), KnowledgeLedgerError> {
        let digest = KnowledgeBodyDigest::of(&body).map_err(|_| KnowledgeLedgerError::Encoding)?;
        self.records.insert_body(body, digest, entries)
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
    ledger: &HolderLedgers,
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
            retention: BTreeMap::new(),
        });
    };
    if records.entries().any(|(entry, body)| {
        entry.record != body.id
            || body.id.get() == 0
            || entry.local.get() == 0
            || entry
                .supersedes
                .iter()
                .chain(&entry.contradicts)
                .any(|related| !records.contains_key(related))
    }) {
        return Err(KnowledgeQueryError::InvalidLedger);
    }
    if let Some(cursor) = &query.after
        && records
            .id_of(cursor.record)
            .and_then(|id| records.body(&id))
            .is_none_or(|body| body.learned_at != cursor.learned_at)
    {
        return Err(KnowledgeQueryError::InvalidCursor);
    }
    let current_only = query.view == KnowledgeHistoryView::CurrentHeads;
    let mut candidates = records
        .entries()
        .filter(|(entry, body)| {
            (!current_only || records.is_current(&entry.record))
                && (query.schemas.is_empty() || query.schemas.contains(&body.schema))
                && query
                    .subjects
                    .iter()
                    .all(|subject| body.subjects.contains(subject))
                && query
                    .learned_after
                    .is_none_or(|time| body.learned_at > time)
                && query
                    .learned_at_or_before
                    .is_none_or(|time| body.learned_at <= time)
        })
        .collect::<Vec<_>>();
    // Pages follow the holder's own order, learned time then local ID, so a
    // cursor never depends on another holder's records.
    candidates.sort_by_key(|(entry, body)| (body.learned_at, entry.local));

    if let Some(cursor) = &query.after {
        candidates.retain(|(entry, body)| {
            (body.learned_at, entry.local) > (cursor.learned_at, cursor.record)
        });
    }

    let limit = usize::try_from(query.limit).unwrap_or(MAX_KNOWLEDGE_PAGE_SIZE as usize);
    let has_more = candidates.len() > limit;
    let page = candidates.into_iter().take(limit).collect::<Vec<_>>();
    let next = if has_more {
        page.last().map(|(entry, body)| KnowledgeCursor {
            holder: holder.clone(),
            query_hash: query_hash.clone(),
            read_cut: read_cut.clone(),
            binding_hash: binding_hash.clone(),
            learned_at: body.learned_at,
            record: entry.local,
        })
    } else {
        None
    };
    let views = page
        .iter()
        .map(|(entry, body)| to_view(&holder, entry, body, records))
        .collect();
    Ok(KnowledgeQueryResult {
        holder,
        read_cut,
        records: views,
        next,
        retention: BTreeMap::new(),
    })
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct KnowledgeSnapshot {
    pub actors: BTreeMap<PersonId, ActorKnowledge>,
    /// Every holder's generic knowledge records. A clone shares them with the
    /// original; see [`HolderLedgers`].
    #[serde(default, skip_serializing_if = "HolderLedgers::is_empty")]
    pub records: HolderLedgers,
}

impl KnowledgeSnapshot {
    /// Counts all retained holder-relative records in one schema namespace.
    /// The ledgers keep the count, so this does not scan them.
    #[must_use]
    pub fn record_count_in_namespace(&self, namespace: &str) -> usize {
        self.records.record_count_in_namespace(namespace)
    }

    #[must_use]
    pub fn for_actor(&self, actor: PersonId) -> Option<&ActorKnowledge> {
        self.actors.get(&actor)
    }

    /// One holder's generic records, or `None` when it holds none.
    #[must_use]
    pub fn for_holder(&self, holder: &KnowledgeHolderRef) -> Option<&HolderLedger> {
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
    /// The settled ledgers are not copied: the overlay is applied to a clone
    /// that shares their structure.
    ///
    /// # Errors
    ///
    /// Returns an error when the overlay collides with a settled record ID or
    /// files a record under another holder, when either ledger is
    /// inconsistent, or when the query or cursor does not match the derived
    /// read cut.
    pub fn query_with_overlay(
        &self,
        holder: KnowledgeHolderRef,
        query: &KnowledgeQuery,
        boundary: Option<BoundaryId>,
        overlay: &StagedKnowledgeEntries,
    ) -> Result<KnowledgeQueryResult, KnowledgeQueryError> {
        let merged = match overlay.get(&holder) {
            Some(staged) => self
                .records
                .with_staged(&holder, staged)
                .map_err(|_| KnowledgeQueryError::InvalidLedger)?,
            None => self.records.clone(),
        };
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
    ledger: &HolderLedgers,
) -> Result<String, KnowledgeQueryError> {
    let views = ledger.get(holder).map_or_else(Vec::new, |records| {
        records
            .entries()
            .map(|(entry, body)| to_view(holder, entry, body, records))
            .collect()
    });
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
    merged: &HolderLedgers,
    overlay: &StagedKnowledgeEntries,
) -> Result<String, KnowledgeQueryError> {
    let views = match (merged.get(holder), overlay.get(holder)) {
        (Some(records), Some(staged)) => staged
            .values()
            .map(|(body, entry)| to_view(holder, entry, body, records))
            .collect(),
        _ => Vec::new(),
    };
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

pub(crate) fn to_view(
    holder: &KnowledgeHolderRef,
    entry: &HolderEntry,
    body: &KnowledgeRecordBody,
    records: &HolderLedger,
) -> KnowledgeRecordView {
    KnowledgeRecordView {
        id: entry.local,
        holder: holder.clone(),
        schema: body.schema.clone(),
        subjects: body.subjects.clone(),
        payload: body.payload.clone(),
        as_of: body.as_of,
        learned_at: body.learned_at,
        confidence_per_mille: body.confidence_per_mille,
        supersedes: entry
            .supersedes
            .iter()
            .filter_map(|id| records.local_of(id))
            .collect(),
        contradicts: entry
            .contradicts
            .iter()
            .filter_map(|id| records.local_of(id))
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

    fn body(id: u64) -> Arc<KnowledgeRecordBody> {
        let (_, body, _, _) = record(
            &KnowledgeHolderRef::Person(PersonId::new(1)),
            id,
            SimTime::from_minutes(i64::try_from(id).unwrap()),
            vec![],
            vec![],
        )
        .into_parts();
        Arc::new(body)
    }

    fn entry(id: u64, local: u64, supersedes: Vec<u64>) -> HolderEntry {
        HolderEntry {
            record: KnowledgeRecordId::new(id),
            local: HolderKnowledgeRecordId::new(local),
            supersedes: supersedes.into_iter().map(KnowledgeRecordId::new).collect(),
            contradicts: vec![],
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
        ledger.records.extend([
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
        ]);

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
        ledger.records.extend([
            record(&holder, 10, SimTime::from_minutes(1), vec![], vec![]),
            record(&holder, 42, SimTime::from_minutes(1), vec![], vec![]),
        ]);
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

    /// A publication is admitted whole or not at all: a duplicate body ID, an
    /// entry for another body, a local ID out of order, a relation the holder
    /// cannot resolve, and a body nobody holds each leave the ledger as it
    /// was.
    #[test]
    fn generic_ledger_admission_is_atomic() {
        let first = KnowledgeHolderRef::Person(PersonId::new(1));
        let second = KnowledgeHolderRef::Person(PersonId::new(2));
        let mut ledger = GenericKnowledgeLedger::default();
        ledger
            .insert_records(body(1), [(first.clone(), entry(1, 1, vec![]))])
            .expect("the first body should be admitted");
        let before = ledger.clone();
        let refused = |ledger: &mut GenericKnowledgeLedger,
                       body: Arc<KnowledgeRecordBody>,
                       entries: Vec<(KnowledgeHolderRef, HolderEntry)>| {
            let error = ledger
                .insert_records(body, entries)
                .expect_err("the publication must be refused");
            assert_eq!(ledger, &before);
            error
        };
        assert_eq!(
            refused(
                &mut ledger,
                body(1),
                vec![(second.clone(), entry(1, 1, vec![]))]
            ),
            KnowledgeLedgerError::DuplicateRecordId
        );
        assert_eq!(
            refused(
                &mut ledger,
                body(2),
                vec![(second.clone(), entry(3, 1, vec![]))]
            ),
            KnowledgeLedgerError::MissingBody
        );
        assert_eq!(
            refused(
                &mut ledger,
                body(2),
                vec![(first.clone(), entry(2, 1, vec![]))]
            ),
            KnowledgeLedgerError::LocalOutOfOrder
        );
        assert_eq!(
            refused(
                &mut ledger,
                body(2),
                vec![
                    (first.clone(), entry(2, 2, vec![1])),
                    (second.clone(), entry(2, 1, vec![1])),
                ]
            ),
            KnowledgeLedgerError::UnresolvedRelation
        );
        assert_eq!(
            refused(&mut ledger, body(2), vec![]),
            KnowledgeLedgerError::UnheldBody
        );

        // One body for two holders is stored once with an entry each.
        ledger
            .insert_records(
                body(2),
                [
                    (first.clone(), entry(2, 2, vec![1])),
                    (second.clone(), entry(2, 1, vec![])),
                ],
            )
            .expect("a shared body should be admitted");
        assert_eq!(ledger.records.body_count(), 2);
        assert_eq!(ledger.records.entry_count(), 3);
        assert_eq!(
            ledger.records.body_holder_count(KnowledgeRecordId::new(2)),
            2
        );
        assert!(Arc::ptr_eq(
            ledger.records[&first]
                .body(&KnowledgeRecordId::new(2))
                .unwrap(),
            ledger.records[&second]
                .body(&KnowledgeRecordId::new(2))
                .unwrap(),
        ));
    }

    /// The format-9 encoding of the ledgers and every refusal its decoding
    /// makes: a local ID at or above `next_local`, a relation to an ID the
    /// holder does not hold, a body no holder holds, an empty holder that
    /// never used a local ID, an entry for a missing body, and unordered or
    /// duplicate bodies and holders. An emptied holder that used local IDs
    /// decodes and continues its count.
    #[test]
    fn snapshot_wire_is_canonical_and_refuses_tampering() {
        assert_eq!(
            serde_json::to_value(KnowledgeSnapshot::default())
                .expect("empty knowledge should serialize"),
            json!({ "actors": {} })
        );

        let first = KnowledgeHolderRef::Person(PersonId::new(1));
        let second = KnowledgeHolderRef::Person(PersonId::new(2));
        let mut records = HolderLedgers::new();
        records.extend([
            record(&second, 3, SimTime::from_minutes(3), vec![], vec![]),
            record(&first, 2, SimTime::from_minutes(2), vec![], vec![]),
            record(
                &first,
                4,
                SimTime::from_minutes(4),
                vec![KnowledgeRecordId::new(2)],
                vec![],
            ),
        ]);
        let snapshot = KnowledgeSnapshot {
            actors: BTreeMap::new(),
            records,
        };
        let encoded = serde_json::to_value(&snapshot).expect("knowledge snapshot should serialize");
        assert_eq!(
            encoded["records"]["bodies"]
                .as_array()
                .unwrap()
                .iter()
                .map(|body| body["id"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            vec![2, 3, 4]
        );
        assert_eq!(
            encoded["records"]["holders"][0],
            json!({
                "holder": { "type": "person", "value": 1 },
                "next_local": 3,
                "entries": [
                    { "record": 2, "local": 1, "supersedes": [], "contradicts": [] },
                    { "record": 4, "local": 2, "supersedes": [2], "contradicts": [] },
                ],
            })
        );
        let decoded = serde_json::from_value::<KnowledgeSnapshot>(encoded.clone())
            .expect("canonical holder ledgers should deserialize");
        assert_eq!(decoded, snapshot);
        assert_eq!(
            decoded.records.commitment_root(),
            snapshot.records.commitment_root()
        );

        let refused = |edit: &dyn Fn(&mut Value)| {
            let mut wire = encoded.clone();
            edit(&mut wire["records"]);
            assert!(serde_json::from_value::<KnowledgeSnapshot>(wire).is_err());
        };
        refused(&|wire| wire["holders"][0]["entries"][1]["local"] = json!(3));
        refused(&|wire| wire["holders"][0]["entries"][1]["local"] = json!(1));
        refused(&|wire| wire["holders"][0]["entries"][0]["local"] = json!(0));
        refused(&|wire| wire["holders"][0]["entries"][1]["supersedes"] = json!([3]));
        refused(&|wire| wire["holders"][1]["entries"] = json!([]));
        refused(&|wire| wire["holders"][0]["entries"][0]["record"] = json!(5));
        refused(&|wire| wire["holders"][1]["holder"] = json!({ "type": "person", "value": 1 }));
        refused(&|wire| {
            wire["holders"] = json!([wire["holders"][1].clone(), wire["holders"][0].clone()]);
        });
        refused(&|wire| wire["bodies"][1]["id"] = json!(2));
        refused(&|wire| {
            wire["holders"][1]["entries"] = json!([]);
            wire["holders"][1]["next_local"] = json!(1);
        });

        // A holder whose entries all left keeps its next local ID.
        let mut emptied = encoded.clone();
        emptied["records"]["bodies"] = json!([
            encoded["records"]["bodies"][0].clone(),
            encoded["records"]["bodies"][2].clone(),
        ]);
        emptied["records"]["holders"][1]["entries"] = json!([]);
        let mut emptied = serde_json::from_value::<KnowledgeSnapshot>(emptied)
            .expect("an emptied holder decodes");
        assert!(emptied.records[&second].is_empty());
        assert_eq!(emptied.records[&second].next_local().get(), 2);
        emptied
            .records
            .insert(record(&second, 9, SimTime::from_minutes(9), vec![], vec![]));
        assert_eq!(
            emptied.records[&second].local_of(&KnowledgeRecordId::new(9)),
            Some(HolderKnowledgeRecordId::new(2))
        );
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
        ledger.records.extend([record(
            &retired,
            1,
            SimTime::from_minutes(1),
            vec![],
            vec![],
        )]);
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
