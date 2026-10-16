//! Persistent holder ledgers: record bodies stored once, one entry for each
//! holder that holds a body, the derived current-record index, and the
//! chunked commitment.

use crate::encoding::{self, IndexKey};
use crate::{
    HolderEntry, KnowledgeBodyDigest, KnowledgeLedgerError, KnowledgeRecord, KnowledgeRecordBody,
    KnowledgeSubject,
};
use canwu_core::{
    HolderKnowledgeRecordId, KnowledgeHolderRef, KnowledgeRecordId, KnowledgeRecordKind,
};
use im::{OrdMap, OrdSet};
use serde::de::Error as _;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::fmt;
use std::ops::Index;
use std::sync::Arc;

/// A current-record set, keyed newest first so that the newest-first walk is
/// a forward iteration. `im` 15.1's ordered collections drop elements when
/// iterated in reverse once a tree is three levels deep (from about 2,177
/// entries), so nothing in this module iterates one backwards.
type NewestFirst = OrdSet<Reverse<KnowledgeRecordId>>;

/// The current records filed under one index key, with the exact kind or
/// subject first filed under it. Until two different values share a key in
/// a ledger, every record in a bucket has the bucket's exact value.
#[derive(Clone)]
struct Bucket<T> {
    exact: Arc<T>,
    records: NewestFirst,
}

impl<T> Bucket<T> {
    fn new(exact: T, id: KnowledgeRecordId) -> Self {
        Self {
            exact: Arc::new(exact),
            records: NewestFirst::unit(Reverse(id)),
        }
    }
}

/// Entries per commitment chunk: chunk `i` of a holder covers its entries
/// with local IDs from `256 i` up to, not including, `256 i + 256`.
pub const KNOWLEDGE_COMMITMENT_CHUNK_ENTRIES: u64 = 256;

/// A body with what the ledgers derive from it once, shared by every entry
/// that holds it.
struct SharedBody {
    body: Arc<KnowledgeRecordBody>,
    digest: KnowledgeBodyDigest,
    kind: IndexKey,
    /// Index keys of `body.subjects`, in the same order.
    subjects: Vec<IndexKey>,
}

impl SharedBody {
    fn new(body: Arc<KnowledgeRecordBody>, digest: KnowledgeBodyDigest) -> Arc<Self> {
        let kind = encoding::kind_key(&body.schema.kind);
        let subjects = body.subjects.iter().map(encoding::subject_key).collect();
        Arc::new(Self {
            body,
            digest,
            kind,
            subjects,
        })
    }
}

#[derive(Clone)]
struct BodySlot {
    shared: Arc<SharedBody>,
    /// How many holders hold the body; the body leaves at zero.
    holders: u32,
}

#[derive(Clone)]
struct StoredEntry {
    shared: Arc<SharedBody>,
    entry: HolderEntry,
}

impl PartialEq for StoredEntry {
    fn eq(&self, other: &Self) -> bool {
        self.entry == other.entry && self.shared.body == other.shared.body
    }
}

#[derive(Clone, Copy)]
struct LocalSlot {
    record: KnowledgeRecordId,
    leaf: [u8; 32],
}

/// The generic knowledge of every holder: record bodies, each stored once
/// under its global ID, and per holder one entry for each body it holds.
///
/// The ledgers are a persistent map. A clone shares all of its structure with
/// the original and costs the same whatever the ledgers hold; inserting or
/// removing an entry copies only the paths it changes. A runtime can therefore
/// keep a copy for rollback, or beside a view, without copying records.
///
/// A body is held by at least one holder: removing its last entry removes it.
/// A holder stays listed once it has held an entry, with its next local ID,
/// even when every entry has left it, so local IDs never restart.
///
/// The ledgers commit to their content through [`Self::commitment_root`]: a
/// leaf hash per entry (computed when the entry is inserted or its relations
/// change), a root per chunk of 256 local IDs, a root per holder over its
/// chunk roots and next local ID, and a root over every holder's root. Roots
/// are cached; a change marks its chunk and holder stale, and
/// [`Self::refresh_commitment`] recomputes only what is stale.
///
/// The encoding is `{ "bodies": [...], "holders": [{ "holder", "next_local",
/// "entries": [...] }] }`, bodies in ID order, holders in holder order, and
/// entries in local order. Decoding refuses a body no holder holds, an entry
/// naming a missing body, a local ID that is zero, out of order, or at or
/// above `next_local`, an empty holder whose `next_local` is 1, a relation
/// that does not resolve within its holder, and a duplicate or unordered body
/// or holder.
///
/// Iteration is forward only: no iterator of this module implements
/// `DoubleEndedIterator`.
#[derive(Clone, Default)]
pub struct HolderLedgers {
    bodies: OrdMap<KnowledgeRecordId, BodySlot>,
    /// Each holder's ledger behind an `Arc`, so a path copy of this map
    /// copies pointers, and a change copies only the holder it touches.
    holders: OrdMap<KnowledgeHolderRef, Arc<HolderLedger>>,
    namespaces: OrdMap<String, usize>,
    entry_count: usize,
    /// Holders whose cached commitment roots are stale.
    stale: OrdSet<KnowledgeHolderRef>,
}

/// One holder's entries, keyed by global record ID, with their stored local
/// IDs and an index of the holder's current records.
///
/// A record is current when no entry of this ledger names it in
/// `supersedes`. The index files current records by schema kind and by
/// subject under 16-byte digest keys, each with the exact kind or subject
/// first filed under it, so a lookup compares its query with a key's exact
/// value once; a ledger in which two different values ever shared a key
/// compares every candidate's kind and subjects exactly instead. The index
/// is derived from the entries alone, kept up to date by every insertion and
/// removal through [`HolderLedgers`], rebuilt when the ledgers are decoded,
/// and never encoded.
#[derive(Clone)]
pub struct HolderLedger {
    holder: KnowledgeHolderRef,
    holder_key: [u8; 32],
    entries: OrdMap<KnowledgeRecordId, Arc<StoredEntry>>,
    /// Entries by local ID, with each entry's leaf hash.
    by_local: OrdMap<u64, LocalSlot>,
    next_local: u64,
    /// How many entries of this ledger name each ID in `supersedes`.
    superseded: OrdMap<KnowledgeRecordId, u32>,
    /// The entries that name each ID in `supersedes` or `contradicts`.
    naming: OrdMap<KnowledgeRecordId, OrdSet<KnowledgeRecordId>>,
    /// Current records by schema kind, newest first.
    heads: OrdMap<IndexKey, Bucket<KnowledgeRecordKind>>,
    /// Current records by schema kind and by each subject they carry, newest
    /// first.
    subject_heads: OrdMap<IndexKey, OrdMap<IndexKey, Bucket<KnowledgeSubject>>>,
    /// Whether two different kinds, or two different subjects, were ever
    /// filed under one key of this ledger. It only selects how lookups
    /// compare, never what they return, and is not part of the content.
    key_collision: bool,
    /// Roots of the non-empty chunks, valid except for `stale_chunks`.
    chunk_roots: OrdMap<u64, [u8; 32]>,
    stale_chunks: OrdSet<u64>,
    /// The holder root, valid while `stale_chunks` is empty and the holder
    /// is not stale in its ledgers.
    root: [u8; 32],
}

const fn chunk_of(local: u64) -> u64 {
    local / KNOWLEDGE_COMMITMENT_CHUNK_ENTRIES
}

impl HolderLedgers {
    /// Ledgers that hold no record.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether no holder is listed. A holder is listed once it has held an
    /// entry.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.holders.is_empty()
    }

    /// The number of listed holders, including holders whose entries have
    /// all left.
    #[must_use]
    pub fn len(&self) -> usize {
        self.holders.len()
    }

    /// One holder's ledger, or `None` when the holder never held an entry.
    #[must_use]
    pub fn get(&self, holder: &KnowledgeHolderRef) -> Option<&HolderLedger> {
        self.holders.get(holder).map(Arc::as_ref)
    }

    /// Whether the holder is listed.
    #[must_use]
    pub fn contains_key(&self, holder: &KnowledgeHolderRef) -> bool {
        self.holders.contains_key(holder)
    }

    /// Every listed holder with its ledger, in holder order.
    #[must_use]
    pub fn iter(&self) -> HolderLedgersIter<'_> {
        HolderLedgersIter {
            inner: self.holders.iter(),
        }
    }

    /// Every listed holder, in holder order.
    pub fn keys(&self) -> impl ExactSizeIterator<Item = &KnowledgeHolderRef> {
        self.holders.keys()
    }

    /// Every listed holder's ledger, in holder order.
    pub fn values(&self) -> impl ExactSizeIterator<Item = &HolderLedger> {
        self.holders.values().map(Arc::as_ref)
    }

    /// The number of stored bodies.
    #[must_use]
    pub fn body_count(&self) -> usize {
        self.bodies.len()
    }

    /// The number of entries across every holder.
    #[must_use]
    pub const fn entry_count(&self) -> usize {
        self.entry_count
    }

    /// The body with a global ID, whichever holders hold it.
    #[must_use]
    pub fn body(&self, id: KnowledgeRecordId) -> Option<&Arc<KnowledgeRecordBody>> {
        self.bodies.get(&id).map(|slot| &slot.shared.body)
    }

    /// The digest of the body with a global ID.
    #[must_use]
    pub fn body_digest(&self, id: KnowledgeRecordId) -> Option<KnowledgeBodyDigest> {
        self.bodies.get(&id).map(|slot| slot.shared.digest)
    }

    /// How many holders hold the body with a global ID; zero when it is not
    /// stored.
    #[must_use]
    pub fn body_holder_count(&self, id: KnowledgeRecordId) -> u32 {
        self.bodies.get(&id).map_or(0, |slot| slot.holders)
    }

    /// Every stored body, in global ID order.
    #[must_use]
    pub fn bodies(&self) -> BodiesById<'_> {
        BodiesById {
            inner: self.bodies.iter(),
        }
    }

    /// The highest global ID of a stored body.
    #[must_use]
    pub fn last_body_id(&self) -> Option<KnowledgeRecordId> {
        self.bodies.get_max().map(|(id, _)| *id)
    }

    /// The number of stored bodies whose schema kind is in `namespace`.
    #[must_use]
    pub fn record_count_in_namespace(&self, namespace: &str) -> usize {
        self.namespaces.get(namespace).copied().unwrap_or_default()
    }

    /// Files an assembled record: its body under its global ID and one entry
    /// for its holder, with the holder's next local ID. When a body with the
    /// same ID is already stored, it and every entry holding it are removed
    /// first, as [`Self::remove_body`] does, and the holder's earlier record
    /// with that ID, if any, is returned.
    ///
    /// This is the authoring path for tests and tools. It does not require
    /// the record's relations to resolve; a runtime publication goes through
    /// [`Self::insert_publication`], which does.
    ///
    /// # Panics
    ///
    /// Panics if the body cannot be encoded as JSON, which a body of JSON
    /// values never fails.
    pub fn insert(&mut self, record: KnowledgeRecord) -> Option<KnowledgeRecord> {
        let previous = if self.bodies.contains_key(&record.id) {
            let previous = self
                .get(&record.holder)
                .and_then(|ledger| ledger.get(&record.id));
            self.remove_body(record.id);
            previous
        } else {
            None
        };
        let (holder, body, supersedes, contradicts) = record.into_parts();
        let digest = KnowledgeBodyDigest::of(&body).expect("a knowledge body encodes as JSON");
        let shared = SharedBody::new(Arc::new(body), digest);
        let ledger = self.holder_entry(&holder);
        let entry = HolderEntry {
            record: shared.body.id,
            local: HolderKnowledgeRecordId::new(ledger.next_local),
            supersedes,
            contradicts,
        };
        ledger
            .insert_entry(&shared, entry, false)
            .expect("the next local ID of a holder that does not hold the ID is in order");
        self.add_body(shared, 1);
        self.entry_count += 1;
        self.stale.insert(holder);
        previous
    }

    /// Stores one body and gives it to each listed holder, atomically: on
    /// error nothing changes. See [`Self::insert_publication`].
    ///
    /// # Errors
    ///
    /// As [`Self::insert_publication`].
    pub fn insert_body(
        &mut self,
        body: Arc<KnowledgeRecordBody>,
        digest: KnowledgeBodyDigest,
        entries: impl IntoIterator<Item = (KnowledgeHolderRef, HolderEntry)>,
    ) -> Result<(), KnowledgeLedgerError> {
        let mut by_holder = BTreeMap::<KnowledgeHolderRef, Vec<HolderEntry>>::new();
        for (holder, entry) in entries {
            by_holder.entry(holder).or_default().push(entry);
        }
        self.insert_publication(
            [(body, digest)],
            by_holder
                .iter()
                .map(|(holder, entries)| (holder, entries.as_slice())),
        )
    }

    /// Stores new bodies and the holder entries that hold them, atomically:
    /// on error nothing changes.
    ///
    /// `digest` must be the [`KnowledgeBodyDigest`] of its body: the
    /// commitment trusts it, and a load recomputes it. Each holder's entries
    /// are inserted in the order given; an entry's local ID must be above
    /// every local ID the holder has used, and its relations must name
    /// entries the holder already holds or receives earlier in the same call.
    ///
    /// # Errors
    ///
    /// [`KnowledgeLedgerError::DuplicateRecordId`] when a body ID is already
    /// stored or repeats, or a holder would hold one ID twice;
    /// [`KnowledgeLedgerError::MissingBody`] when an entry names a body that
    /// is neither stored nor given; [`KnowledgeLedgerError::UnheldBody`] when
    /// a given body receives no entry; [`KnowledgeLedgerError::LocalOutOfOrder`]
    /// for a local ID that is zero or not above the holder's previous one;
    /// [`KnowledgeLedgerError::UnresolvedRelation`] for a relation the holder
    /// cannot resolve.
    pub fn insert_publication<'a>(
        &mut self,
        bodies: impl IntoIterator<Item = (Arc<KnowledgeRecordBody>, KnowledgeBodyDigest)>,
        holders: impl IntoIterator<Item = (&'a KnowledgeHolderRef, &'a [HolderEntry])>,
    ) -> Result<(), KnowledgeLedgerError> {
        // Everything is checked before anything changes, so the ledgers are
        // never cloned for rollback: a runtime that cloned them once for its
        // boundary keeps every path it already copied.
        let mut new_bodies = BTreeMap::<KnowledgeRecordId, (Arc<SharedBody>, u32)>::new();
        for (body, digest) in bodies {
            let id = body.id;
            if id.get() == 0
                || self.bodies.contains_key(&id)
                || new_bodies
                    .insert(id, (SharedBody::new(body, digest), 0))
                    .is_some()
            {
                return Err(KnowledgeLedgerError::DuplicateRecordId);
            }
        }
        let holders: Vec<_> = holders.into_iter().collect();
        let mut shared_existing = BTreeMap::<KnowledgeRecordId, u32>::new();
        let mut planned = BTreeMap::<
            &KnowledgeHolderRef,
            (u64, std::collections::BTreeSet<KnowledgeRecordId>),
        >::new();
        for (holder, entries) in &holders {
            let ledger = self.get(holder);
            let (next_local, added) = planned.entry(*holder).or_insert_with(|| {
                (
                    ledger.map_or(1, |ledger| ledger.next_local),
                    std::collections::BTreeSet::new(),
                )
            });
            for entry in *entries {
                let id = entry.record;
                if let Some((_, count)) = new_bodies.get_mut(&id) {
                    *count += 1;
                } else if self.bodies.contains_key(&id) {
                    *shared_existing.entry(id).or_default() += 1;
                } else {
                    return Err(KnowledgeLedgerError::MissingBody);
                }
                let local = entry.local.get();
                if local == 0 || local < *next_local {
                    return Err(KnowledgeLedgerError::LocalOutOfOrder);
                }
                *next_local = local
                    .checked_add(1)
                    .ok_or(KnowledgeLedgerError::LocalOutOfOrder)?;
                let held = |target: &KnowledgeRecordId| {
                    ledger.is_some_and(|ledger| ledger.entries.contains_key(target))
                        || added.contains(target)
                };
                if entry
                    .supersedes
                    .iter()
                    .chain(&entry.contradicts)
                    .any(|target| !held(target))
                {
                    return Err(KnowledgeLedgerError::UnresolvedRelation);
                }
                if held(&id) {
                    return Err(KnowledgeLedgerError::DuplicateRecordId);
                }
                added.insert(id);
            }
        }
        if new_bodies.values().any(|(_, holders)| *holders == 0) {
            return Err(KnowledgeLedgerError::UnheldBody);
        }
        for (holder, entries) in holders {
            let shared = entries
                .iter()
                .map(|entry| {
                    new_bodies.get(&entry.record).map_or_else(
                        || {
                            self.bodies
                                .get(&entry.record)
                                .map(|slot| Arc::clone(&slot.shared))
                        },
                        |(shared, _)| Some(Arc::clone(shared)),
                    )
                })
                .collect::<Option<Vec<_>>>()
                .ok_or(KnowledgeLedgerError::MissingBody)?;
            let ledger = self.holder_entry(holder);
            for (entry, shared) in entries.iter().zip(&shared) {
                ledger.insert_entry(shared, entry.clone(), false)?;
            }
            self.entry_count += entries.len();
            self.stale.insert(holder.clone());
        }
        for (id, count) in shared_existing {
            if let Some(slot) = self.bodies.get_mut(&id) {
                slot.holders = slot.holders.saturating_add(count);
            }
        }
        for (shared, holders) in new_bodies.into_values() {
            self.add_body(shared, holders);
        }
        Ok(())
    }

    /// Removes one holder's entry, drops every name of it from the holder's
    /// other entries, and removes its body when no holder holds it any more.
    /// The holder stays listed. Returns the entry as it was.
    pub fn remove_entry(
        &mut self,
        holder: &KnowledgeHolderRef,
        id: KnowledgeRecordId,
    ) -> Option<HolderEntry> {
        let ledger = Arc::make_mut(self.holders.get_mut(holder)?);
        let (shared, entry) = ledger.remove_entry(id)?;
        self.entry_count = self.entry_count.saturating_sub(1);
        self.stale.insert(holder.clone());
        let unheld = self.bodies.get_mut(&id).is_some_and(|slot| {
            slot.holders = slot.holders.saturating_sub(1);
            slot.holders == 0
        });
        if unheld {
            self.bodies.remove(&id);
            let namespace = &shared.body.schema.kind.namespace;
            if let Some(count) = self.namespaces.get_mut(namespace) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    self.namespaces.remove(namespace);
                }
            }
        }
        Some(entry)
    }

    /// Removes a body and every entry that holds it, as
    /// [`Self::remove_entry`] does for each. It visits every holder.
    pub fn remove_body(&mut self, id: KnowledgeRecordId) -> Option<Arc<KnowledgeRecordBody>> {
        let body = Arc::clone(self.body(id)?);
        let holders: Vec<KnowledgeHolderRef> = self
            .holders
            .iter()
            .filter(|(_, ledger)| ledger.contains_key(&id))
            .map(|(holder, _)| holder.clone())
            .collect();
        for holder in &holders {
            self.remove_entry(holder, id);
        }
        Some(body)
    }

    /// Retires, from `holder`, every entry its entry `id` supersedes: the
    /// retention rule `RetireWhenSuperseded` applies this to each new entry
    /// of a retiring schema. Returns how many entries left.
    pub fn retire_superseded_by(
        &mut self,
        holder: &KnowledgeHolderRef,
        id: KnowledgeRecordId,
    ) -> usize {
        let Some(targets) = self
            .get(holder)
            .and_then(|ledger| ledger.entry(&id))
            .map(|entry| entry.supersedes.clone())
        else {
            return 0;
        };
        targets
            .into_iter()
            .filter(|target| self.remove_entry(holder, *target).is_some())
            .count()
    }

    /// The root of the chunked commitment over every holder, computed from
    /// the cached roots and, for anything stale, from the leaf hashes. It
    /// does not change the cache; [`Self::refresh_commitment`] does.
    #[must_use]
    pub fn commitment_root(&self) -> [u8; 32] {
        encoding::ledger_root(self.holders.iter().map(|(holder, ledger)| {
            let root = if self.stale.contains(holder) {
                ledger.fresh_root()
            } else {
                ledger.root
            };
            (&ledger.holder_key, root)
        }))
    }

    /// Recomputes the cached chunk and holder roots that changes left stale.
    pub fn refresh_commitment(&mut self) {
        let stale = std::mem::take(&mut self.stale);
        for holder in &stale {
            if let Some(ledger) = self.holders.get_mut(holder) {
                Arc::make_mut(ledger).refresh();
            }
        }
    }

    /// Whether both ledgers share all of their structure, which a clone does
    /// until either side changes. Equal ledgers built apart do not.
    #[must_use]
    pub fn ptr_eq(&self, other: &Self) -> bool {
        self.bodies.ptr_eq(&other.bodies)
            && self.holders.ptr_eq(&other.holders)
            && self.namespaces.ptr_eq(&other.namespaces)
    }

    /// The ledgers with `staged` entries of one holder added for a read, as a
    /// settling boundary's overlay. The result's commitment is meaningless:
    /// the staged bodies are not hashed.
    pub(crate) fn with_staged(
        &self,
        holder: &KnowledgeHolderRef,
        staged: &BTreeMap<KnowledgeRecordId, (Arc<KnowledgeRecordBody>, HolderEntry)>,
    ) -> Result<Self, KnowledgeLedgerError> {
        let mut merged = self.clone();
        for (id, (body, entry)) in staged {
            if *id != body.id || *id != entry.record || merged.bodies.contains_key(id) {
                return Err(KnowledgeLedgerError::DuplicateRecordId);
            }
            let shared = SharedBody::new(Arc::clone(body), KnowledgeBodyDigest::unhashed());
            merged
                .holder_entry(holder)
                .insert_entry(&shared, entry.clone(), false)?;
            merged.add_body(shared, 1);
            merged.entry_count += 1;
        }
        Ok(merged)
    }

    fn holder_entry(&mut self, holder: &KnowledgeHolderRef) -> &mut HolderLedger {
        if !self.holders.contains_key(holder) {
            self.holders
                .insert(holder.clone(), Arc::new(HolderLedger::new(holder.clone())));
        }
        Arc::make_mut(
            self.holders
                .get_mut(holder)
                .expect("the holder is listed: it was there or was just inserted"),
        )
    }

    fn add_body(&mut self, shared: Arc<SharedBody>, holders: u32) {
        *entry_or_default(&mut self.namespaces, &shared.body.schema.kind.namespace) += 1;
        self.bodies
            .insert(shared.body.id, BodySlot { shared, holders });
    }
}

impl HolderLedger {
    fn new(holder: KnowledgeHolderRef) -> Self {
        let holder_key = encoding::holder_key(&holder);
        Self {
            holder,
            holder_key,
            entries: OrdMap::new(),
            by_local: OrdMap::new(),
            next_local: 1,
            superseded: OrdMap::new(),
            naming: OrdMap::new(),
            heads: OrdMap::new(),
            subject_heads: OrdMap::new(),
            key_collision: false,
            chunk_roots: OrdMap::new(),
            stale_chunks: OrdSet::new(),
            root: encoding::holder_root(1, std::iter::empty()),
        }
    }

    /// The holder this ledger belongs to.
    #[must_use]
    pub const fn holder(&self) -> &KnowledgeHolderRef {
        &self.holder
    }

    /// The number of entries the holder holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the holder holds no entry.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The local ID the holder's next entry receives. It never decreases.
    #[must_use]
    pub const fn next_local(&self) -> HolderKnowledgeRecordId {
        HolderKnowledgeRecordId::new(self.next_local)
    }

    /// The local ID of the holder's entry for a global ID.
    #[must_use]
    pub fn local_of(&self, id: &KnowledgeRecordId) -> Option<HolderKnowledgeRecordId> {
        self.entries.get(id).map(|stored| stored.entry.local)
    }

    /// The global ID of the holder's entry with a local ID.
    #[must_use]
    pub fn id_of(&self, local: HolderKnowledgeRecordId) -> Option<KnowledgeRecordId> {
        self.by_local.get(&local.get()).map(|slot| slot.record)
    }

    /// The holder's entry for a global ID.
    #[must_use]
    pub fn entry(&self, id: &KnowledgeRecordId) -> Option<&HolderEntry> {
        self.entries.get(id).map(|stored| &stored.entry)
    }

    /// The body the holder's entry for a global ID holds.
    #[must_use]
    pub fn body(&self, id: &KnowledgeRecordId) -> Option<&Arc<KnowledgeRecordBody>> {
        self.entries.get(id).map(|stored| &stored.shared.body)
    }

    /// The holder's record for a global ID, assembled from its body and
    /// entry.
    #[must_use]
    pub fn get(&self, id: &KnowledgeRecordId) -> Option<KnowledgeRecord> {
        self.entries.get(id).map(|stored| self.assemble(stored))
    }

    /// Whether the holder holds the record with a global ID.
    #[must_use]
    pub fn contains_key(&self, id: &KnowledgeRecordId) -> bool {
        self.entries.contains_key(id)
    }

    /// The holder's entries with their bodies, in global ID order, without
    /// assembling records.
    #[must_use]
    pub fn entries(&self) -> HolderEntriesIter<'_> {
        HolderEntriesIter {
            inner: self.entries.values(),
        }
    }

    /// The holder's assembled records with their IDs, in ID order.
    #[must_use]
    pub fn iter(&self) -> HolderRecordsIter<'_> {
        HolderRecordsIter {
            ledger: self,
            inner: self.entries.iter(),
        }
    }

    /// The holder's record IDs, in ascending order.
    pub fn keys(&self) -> impl ExactSizeIterator<Item = &KnowledgeRecordId> {
        self.entries.keys()
    }

    /// The holder's assembled records, in ID order.
    pub fn values(&self) -> impl ExactSizeIterator<Item = KnowledgeRecord> + '_ {
        self.entries.values().map(|stored| self.assemble(stored))
    }

    /// Whether the holder holds the record and no entry of the holder
    /// supersedes it.
    #[must_use]
    pub fn is_current(&self, id: &KnowledgeRecordId) -> bool {
        self.entries.contains_key(id) && !self.superseded.contains_key(id)
    }

    /// Whether some entry of the holder names `id` in `supersedes`, whether
    /// or not the holder holds `id`.
    #[must_use]
    pub fn is_superseded(&self, id: &KnowledgeRecordId) -> bool {
        self.superseded.contains_key(id)
    }

    /// The bodies of the holder's current records of `kind` that carry every
    /// one of `subjects`, newest first; with no subjects, every current
    /// record of `kind`.
    ///
    /// The walk reads the index, not the ledger: it visits the current
    /// records filed under the kind's key that carry the listed subject held
    /// by the fewest of them, so it never reads a superseded record, and a
    /// caller that stops after `n` matches pays for the matches and for the
    /// candidates it skipped. The query's kind and subjects are compared
    /// exactly with the values filed under their keys, or, in a ledger where
    /// two values ever shared a key, with every candidate's, so two kinds or
    /// subjects whose digest keys collide are never merged.
    #[must_use]
    pub fn current_records<'a>(
        &'a self,
        kind: &'a KnowledgeRecordKind,
        subjects: &'a [KnowledgeSubject],
    ) -> CurrentRecords<'a> {
        let mut keys = Vec::new();
        let candidates = self.candidates(kind, subjects, &mut keys);
        let check = if self.key_collision {
            CandidateCheck::Exact { kind, subjects }
        } else if subjects.len() > 1 {
            CandidateCheck::Keys(keys)
        } else {
            CandidateCheck::Nothing
        };
        CurrentRecords {
            ledger: self,
            check,
            candidates: candidates.map(OrdSet::iter),
        }
    }

    /// The narrowest bucket of current records that can hold a match, with
    /// the subjects' keys pushed onto `keys`. While no two values shared a
    /// key, a query whose kind or subject differs from the value filed
    /// under its key matches nothing.
    fn candidates<'a>(
        &'a self,
        kind: &KnowledgeRecordKind,
        subjects: &[KnowledgeSubject],
        keys: &mut Vec<IndexKey>,
    ) -> Option<&'a NewestFirst> {
        let uniform = !self.key_collision;
        let kind_key = encoding::kind_key(kind);
        let heads = self.heads.get(&kind_key)?;
        if uniform && *heads.exact != *kind {
            return None;
        }
        if subjects.is_empty() {
            return Some(&heads.records);
        }
        let by_subject = self.subject_heads.get(&kind_key)?;
        let mut narrowest: Option<&NewestFirst> = None;
        for subject in subjects {
            let key = encoding::subject_key(subject);
            let bucket = by_subject.get(&key)?;
            if uniform && *bucket.exact != *subject {
                return None;
            }
            if narrowest.is_none_or(|current| bucket.records.len() < current.len()) {
                narrowest = Some(&bucket.records);
            }
            keys.push(key);
        }
        narrowest
    }

    fn assemble(&self, stored: &StoredEntry) -> KnowledgeRecord {
        KnowledgeRecord::assemble(&self.holder, &stored.entry, &stored.shared.body)
    }

    fn insert_entry(
        &mut self,
        shared: &Arc<SharedBody>,
        entry: HolderEntry,
        check_relations: bool,
    ) -> Result<(), KnowledgeLedgerError> {
        let id = entry.record;
        let local = entry.local.get();
        if id != shared.body.id {
            return Err(KnowledgeLedgerError::MissingBody);
        }
        if self.entries.contains_key(&id) {
            return Err(KnowledgeLedgerError::DuplicateRecordId);
        }
        if local == 0 || local < self.next_local {
            return Err(KnowledgeLedgerError::LocalOutOfOrder);
        }
        let next_local = local
            .checked_add(1)
            .ok_or(KnowledgeLedgerError::LocalOutOfOrder)?;
        if check_relations
            && entry
                .supersedes
                .iter()
                .chain(&entry.contradicts)
                .any(|target| !self.entries.contains_key(target))
        {
            return Err(KnowledgeLedgerError::UnresolvedRelation);
        }
        for target in &entry.supersedes {
            let count = entry_or_default(&mut self.superseded, target);
            *count = count.saturating_add(1);
            if *count == 1
                && let Some(target) = self.entries.get(target).cloned()
            {
                self.unindex(&target.shared);
            }
        }
        for target in entry.supersedes.iter().chain(&entry.contradicts) {
            entry_or_default(&mut self.naming, target).insert(id);
        }
        if !self.superseded.contains_key(&id) {
            self.index(shared);
        }
        let leaf = encoding::leaf_hash(&entry, &shared.digest);
        self.by_local.insert(local, LocalSlot { record: id, leaf });
        self.entries.insert(
            id,
            Arc::new(StoredEntry {
                shared: Arc::clone(shared),
                entry,
            }),
        );
        self.next_local = next_local;
        self.stale_chunks.insert(chunk_of(local));
        Ok(())
    }

    fn remove_entry(&mut self, id: KnowledgeRecordId) -> Option<(Arc<SharedBody>, HolderEntry)> {
        let stored = self.entries.remove(&id)?;
        let local = stored.entry.local.get();
        self.by_local.remove(&local);
        self.stale_chunks.insert(chunk_of(local));
        if !self.superseded.contains_key(&id) {
            self.unindex(&stored.shared);
        }
        for target in &stored.entry.supersedes {
            let Some(count) = self.superseded.get_mut(target) else {
                continue;
            };
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.superseded.remove(target);
                if let Some(target) = self.entries.get(target).cloned() {
                    self.index(&target.shared);
                }
            }
        }
        for target in stored
            .entry
            .supersedes
            .iter()
            .chain(&stored.entry.contradicts)
        {
            if let Some(namers) = self.naming.get_mut(target) {
                namers.remove(&id);
                if namers.is_empty() {
                    self.naming.remove(target);
                }
            }
        }
        // Every relation of an entry resolves within its holder: the
        // entries that named this one drop the name.
        self.superseded.remove(&id);
        if let Some(namers) = self.naming.remove(&id) {
            for namer in &namers {
                let Some(stored) = self.entries.get_mut(namer) else {
                    continue;
                };
                let stored = Arc::make_mut(stored);
                stored.entry.supersedes.retain(|target| *target != id);
                stored.entry.contradicts.retain(|target| *target != id);
                let local = stored.entry.local.get();
                let leaf = encoding::leaf_hash(&stored.entry, &stored.shared.digest);
                self.by_local.insert(
                    local,
                    LocalSlot {
                        record: *namer,
                        leaf,
                    },
                );
                self.stale_chunks.insert(chunk_of(local));
            }
        }
        let StoredEntry { shared, entry } = Arc::unwrap_or_clone(stored);
        Some((shared, entry))
    }

    /// Files a current record under its kind and subjects, noting a key
    /// that already holds a different exact value.
    fn index(&mut self, shared: &SharedBody) {
        let id = shared.body.id;
        let body = shared.body.as_ref();
        let mut collision = false;
        if let Some(bucket) = self.heads.get_mut(&shared.kind) {
            collision |= *bucket.exact != body.schema.kind;
            bucket.records.insert(Reverse(id));
        } else {
            self.heads
                .insert(shared.kind, Bucket::new(body.schema.kind.clone(), id));
        }
        if !shared.subjects.is_empty() {
            let by_subject = entry_or_default(&mut self.subject_heads, &shared.kind);
            for (key, subject) in shared.subjects.iter().zip(&body.subjects) {
                if let Some(bucket) = by_subject.get_mut(key) {
                    collision |= *bucket.exact != *subject;
                    bucket.records.insert(Reverse(id));
                } else {
                    by_subject.insert(*key, Bucket::new(subject.clone(), id));
                }
            }
        }
        self.key_collision |= collision;
    }

    fn unindex(&mut self, shared: &SharedBody) {
        let id = shared.body.id;
        if let Some(bucket) = self.heads.get_mut(&shared.kind) {
            bucket.records.remove(&Reverse(id));
            if bucket.records.is_empty() {
                self.heads.remove(&shared.kind);
            }
        }
        if let Some(by_subject) = self.subject_heads.get_mut(&shared.kind) {
            for subject in &shared.subjects {
                if let Some(bucket) = by_subject.get_mut(subject) {
                    bucket.records.remove(&Reverse(id));
                    if bucket.records.is_empty() {
                        by_subject.remove(subject);
                    }
                }
            }
            if by_subject.is_empty() {
                self.subject_heads.remove(&shared.kind);
            }
        }
    }

    fn chunk_root(&self, chunk: u64) -> Option<[u8; 32]> {
        let start = chunk * KNOWLEDGE_COMMITMENT_CHUNK_ENTRIES;
        let mut leaves = self
            .by_local
            .range(start..start.saturating_add(KNOWLEDGE_COMMITMENT_CHUNK_ENTRIES))
            .map(|(_, slot)| &slot.leaf)
            .peekable();
        leaves.peek()?;
        Some(encoding::chunk_root(leaves))
    }

    fn refresh(&mut self) {
        let stale = std::mem::take(&mut self.stale_chunks);
        for chunk in &stale {
            match self.chunk_root(*chunk) {
                Some(root) => {
                    self.chunk_roots.insert(*chunk, root);
                }
                None => {
                    self.chunk_roots.remove(chunk);
                }
            }
        }
        self.root = encoding::holder_root(self.next_local, self.chunk_roots.iter());
    }

    fn fresh_root(&self) -> [u8; 32] {
        if self.stale_chunks.is_empty() {
            return encoding::holder_root(self.next_local, self.chunk_roots.iter());
        }
        let mut roots: BTreeMap<u64, [u8; 32]> = self
            .chunk_roots
            .iter()
            .map(|(chunk, root)| (*chunk, *root))
            .collect();
        for chunk in &self.stale_chunks {
            match self.chunk_root(*chunk) {
                Some(root) => {
                    roots.insert(*chunk, root);
                }
                None => {
                    roots.remove(chunk);
                }
            }
        }
        encoding::holder_root(self.next_local, roots.iter())
    }
}

/// The value under `key`, inserted as the default first. The key is cloned
/// only when it is new, which keeps repeated kinds, subjects, and holders from
/// allocating on every insertion.
fn entry_or_default<'a, K: Ord + Clone, V: Default + Clone>(
    map: &'a mut OrdMap<K, V>,
    key: &K,
) -> &'a mut V {
    if !map.contains_key(key) {
        map.insert(key.clone(), V::default());
    }
    map.get_mut(key)
        .expect("the key is present: it was there or was just inserted")
}

/// Iterator over every holder and its ledger, from [`HolderLedgers::iter`].
pub struct HolderLedgersIter<'a> {
    inner: im::ordmap::Iter<'a, KnowledgeHolderRef, Arc<HolderLedger>>,
}

impl<'a> Iterator for HolderLedgersIter<'a> {
    type Item = (&'a KnowledgeHolderRef, &'a HolderLedger);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .next()
            .map(|(holder, ledger)| (holder, ledger.as_ref()))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl ExactSizeIterator for HolderLedgersIter<'_> {}

impl<'a> IntoIterator for &'a HolderLedgers {
    type Item = (&'a KnowledgeHolderRef, &'a HolderLedger);
    type IntoIter = HolderLedgersIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Iterator over one holder's assembled records with their IDs, from
/// [`HolderLedger::iter`].
pub struct HolderRecordsIter<'a> {
    ledger: &'a HolderLedger,
    inner: im::ordmap::Iter<'a, KnowledgeRecordId, Arc<StoredEntry>>,
}

impl<'a> Iterator for HolderRecordsIter<'a> {
    type Item = (&'a KnowledgeRecordId, KnowledgeRecord);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .next()
            .map(|(id, stored)| (id, self.ledger.assemble(stored)))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl ExactSizeIterator for HolderRecordsIter<'_> {}

impl<'a> IntoIterator for &'a HolderLedger {
    type Item = (&'a KnowledgeRecordId, KnowledgeRecord);
    type IntoIter = HolderRecordsIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Iterator over one holder's entries with their bodies, in global ID
/// order, from [`HolderLedger::entries`].
pub struct HolderEntriesIter<'a> {
    inner: im::ordmap::Values<'a, KnowledgeRecordId, Arc<StoredEntry>>,
}

impl<'a> Iterator for HolderEntriesIter<'a> {
    type Item = (&'a HolderEntry, &'a Arc<KnowledgeRecordBody>);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .next()
            .map(|stored| (&stored.entry, &stored.shared.body))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl ExactSizeIterator for HolderEntriesIter<'_> {}

/// Iterator over stored bodies in global ID order, from
/// [`HolderLedgers::bodies`].
pub struct BodiesById<'a> {
    inner: im::ordmap::Iter<'a, KnowledgeRecordId, BodySlot>,
}

impl<'a> Iterator for BodiesById<'a> {
    type Item = &'a Arc<KnowledgeRecordBody>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|(_, slot)| &slot.shared.body)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl ExactSizeIterator for BodiesById<'_> {}

/// Iterator over the bodies of a holder's current records of one kind,
/// newest first, from [`HolderLedger::current_records`].
pub struct CurrentRecords<'a> {
    ledger: &'a HolderLedger,
    check: CandidateCheck<'a>,
    candidates: Option<im::ordset::Iter<'a, Reverse<KnowledgeRecordId>>>,
}

/// What a candidate from the narrowest bucket must still show.
enum CandidateCheck<'a> {
    /// Nothing: the bucket holds exactly the query's kind and its one
    /// subject, or no subject was asked for.
    Nothing,
    /// The other subjects' keys: in a ledger where no two values shared a
    /// key, carrying a key means carrying the exact subject filed under it,
    /// which the query matched.
    Keys(Vec<IndexKey>),
    /// The exact kind and subjects, in a ledger where two values once
    /// shared a key.
    Exact {
        kind: &'a KnowledgeRecordKind,
        subjects: &'a [KnowledgeSubject],
    },
}

impl CandidateCheck<'_> {
    fn admits(&self, shared: &SharedBody) -> bool {
        match self {
            Self::Nothing => true,
            Self::Keys(keys) => keys.iter().all(|key| shared.subjects.contains(key)),
            Self::Exact { kind, subjects } => {
                shared.body.schema.kind == **kind
                    && subjects
                        .iter()
                        .all(|subject| shared.body.subjects.contains(subject))
            }
        }
    }
}

impl<'a> Iterator for CurrentRecords<'a> {
    type Item = &'a KnowledgeRecordBody;

    fn next(&mut self) -> Option<Self::Item> {
        let ledger = self.ledger;
        let candidates = self.candidates.as_mut()?;
        for Reverse(id) in candidates.by_ref() {
            let Some(stored) = ledger.entries.get(id) else {
                continue;
            };
            if self.check.admits(&stored.shared) {
                return Some(stored.shared.body.as_ref());
            }
        }
        None
    }
}

impl Index<&KnowledgeHolderRef> for HolderLedgers {
    type Output = HolderLedger;

    /// Returns a holder's ledger.
    ///
    /// # Panics
    ///
    /// Panics when the holder is not listed, as an ordered map does for a
    /// missing key.
    fn index(&self, holder: &KnowledgeHolderRef) -> &Self::Output {
        self.get(holder)
            .expect("the knowledge holder is not listed")
    }
}

impl PartialEq for HolderLedgers {
    fn eq(&self, other: &Self) -> bool {
        self.holders == other.holders && self.bodies.keys().eq(other.bodies.keys())
    }
}

impl Eq for HolderLedgers {}

impl PartialEq for HolderLedger {
    fn eq(&self, other: &Self) -> bool {
        self.holder == other.holder
            && self.next_local == other.next_local
            && self.entries == other.entries
    }
}

impl Eq for HolderLedger {}

impl fmt::Debug for HolderLedgers {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_map().entries(self.iter()).finish()
    }
}

impl fmt::Debug for HolderLedger {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HolderLedger")
            .field("holder", &self.holder)
            .field("next_local", &self.next_local)
            .field(
                "entries",
                &self
                    .entries()
                    .map(|(entry, body)| (entry, body.as_ref()))
                    .collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}

impl FromIterator<KnowledgeRecord> for HolderLedgers {
    fn from_iter<I: IntoIterator<Item = KnowledgeRecord>>(records: I) -> Self {
        let mut ledgers = Self::new();
        ledgers.extend(records);
        ledgers
    }
}

impl Extend<KnowledgeRecord> for HolderLedgers {
    fn extend<I: IntoIterator<Item = KnowledgeRecord>>(&mut self, records: I) {
        for record in records {
            self.insert(record);
        }
    }
}

struct BodiesWire<'a>(&'a HolderLedgers);

impl Serialize for BodiesWire<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.0.bodies().map(Arc::as_ref))
    }
}

struct EntriesWire<'a>(&'a HolderLedger);

impl Serialize for EntriesWire<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let ledger = self.0;
        serializer.collect_seq(
            ledger
                .by_local
                .values()
                .filter_map(|slot| ledger.entries.get(&slot.record))
                .map(|stored| &stored.entry),
        )
    }
}

#[derive(Serialize)]
struct HolderWireRef<'a> {
    holder: &'a KnowledgeHolderRef,
    next_local: u64,
    entries: EntriesWire<'a>,
}

struct HoldersWire<'a>(&'a HolderLedgers);

impl Serialize for HoldersWire<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.0.holders.iter().map(|(holder, ledger)| HolderWireRef {
            holder,
            next_local: ledger.next_local,
            entries: EntriesWire(ledger),
        }))
    }
}

impl Serialize for HolderLedgers {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut wire = serializer.serialize_struct("HolderLedgers", 2)?;
        wire.serialize_field("bodies", &BodiesWire(self))?;
        wire.serialize_field("holders", &HoldersWire(self))?;
        wire.end()
    }
}

#[derive(Deserialize)]
struct HolderWire {
    holder: KnowledgeHolderRef,
    next_local: u64,
    entries: Vec<HolderEntry>,
}

#[derive(Deserialize)]
struct LedgersWire {
    bodies: Vec<KnowledgeRecordBody>,
    holders: Vec<HolderWire>,
}

impl<'de> Deserialize<'de> for HolderLedgers {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = LedgersWire::deserialize(deserializer)?;
        let mut ledgers = Self::new();
        let mut shared = BTreeMap::<KnowledgeRecordId, (Arc<SharedBody>, u32)>::new();
        let mut previous = None;
        for body in wire.bodies {
            if body.id.get() == 0 || previous.is_some_and(|previous| previous >= body.id) {
                return Err(D::Error::custom(
                    "knowledge bodies are not in strictly ascending nonzero ID order",
                ));
            }
            previous = Some(body.id);
            let digest = KnowledgeBodyDigest::of(&body).map_err(D::Error::custom)?;
            shared.insert(body.id, (SharedBody::new(Arc::new(body), digest), 0));
        }
        let mut previous_holder: Option<KnowledgeHolderRef> = None;
        for holder in wire.holders {
            if previous_holder
                .as_ref()
                .is_some_and(|previous| previous >= &holder.holder)
            {
                return Err(D::Error::custom(
                    "knowledge holder ledgers are not in strictly ascending holder order",
                ));
            }
            previous_holder = Some(holder.holder.clone());
            if holder.next_local == 0 || (holder.entries.is_empty() && holder.next_local == 1) {
                return Err(D::Error::custom(
                    "an empty knowledge holder ledger must have used a local ID",
                ));
            }
            let ledger = ledgers.holder_entry(&holder.holder);
            for entry in holder.entries {
                let Some((body, holders)) = shared.get_mut(&entry.record) else {
                    return Err(D::Error::custom(
                        "a knowledge holder entry names a body that is not stored",
                    ));
                };
                *holders += 1;
                if entry.local.get() >= holder.next_local {
                    return Err(D::Error::custom(
                        "a knowledge holder entry's local ID is at or above the holder's next local ID",
                    ));
                }
                ledger
                    .insert_entry(body, entry, false)
                    .map_err(|error| D::Error::custom(format!("{error:?} in a holder ledger")))?;
            }
            if ledger.entries.values().any(|stored| {
                stored
                    .entry
                    .supersedes
                    .iter()
                    .chain(&stored.entry.contradicts)
                    .any(|target| !ledger.entries.contains_key(target))
            }) {
                return Err(D::Error::custom(
                    "a knowledge relation does not resolve within its holder",
                ));
            }
            ledger.next_local = holder.next_local;
            let held = ledger.entries.len();
            ledgers.entry_count += held;
            ledgers.stale.insert(holder.holder);
        }
        for (body, holders) in shared.into_values() {
            if holders == 0 {
                return Err(D::Error::custom("a knowledge body is held by no holder"));
            }
            ledgers.add_body(body, holders);
        }
        ledgers.refresh_commitment();
        Ok(ledgers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{KnowledgeOrigin, KnowledgeSubjectTarget};
    use canwu_core::{EventId, KnowledgeSchemaId, PersonId};
    use canwu_time::SimTime;
    use std::collections::{BTreeMap, BTreeSet};

    /// A small deterministic generator, so the edit sequence is the same on
    /// every platform and run.
    struct Sequence(u64);

    impl Sequence {
        fn next(&mut self, bound: u64) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 33) % bound
        }

        fn pick<T: Clone>(&mut self, values: &[T]) -> T {
            let bound = u64::try_from(values.len()).unwrap();
            values[usize::try_from(self.next(bound)).unwrap()].clone()
        }
    }

    fn kind(index: u64) -> KnowledgeRecordKind {
        KnowledgeRecordKind::new(format!("fixture.ledger{}", index % 2), "claim")
    }

    fn subject(index: u64) -> KnowledgeSubject {
        KnowledgeSubject {
            role: format!("line{index}"),
            target: KnowledgeSubjectTarget::Event(EventId::new(index + 1)),
        }
    }

    fn holder(index: u64) -> KnowledgeHolderRef {
        KnowledgeHolderRef::Person(PersonId::new(index + 1))
    }

    fn body(
        id: u64,
        kind: KnowledgeRecordKind,
        subjects: Vec<KnowledgeSubject>,
    ) -> KnowledgeRecordBody {
        KnowledgeRecordBody {
            id: KnowledgeRecordId::new(id),
            schema: KnowledgeSchemaId::new(kind, 1),
            subjects,
            payload: serde_json::json!({ "id": id }),
            as_of: None,
            learned_at: SimTime::EPOCH,
            confidence_per_mille: 1_000,
            origin: KnowledgeOrigin {
                method: "fixture".to_owned(),
                evidence: Vec::new(),
            },
        }
    }

    fn record(
        id: u64,
        holder: &KnowledgeHolderRef,
        kind: KnowledgeRecordKind,
        subjects: Vec<KnowledgeSubject>,
    ) -> KnowledgeRecord {
        let entry = HolderEntry {
            record: KnowledgeRecordId::new(id),
            local: HolderKnowledgeRecordId::new(1),
            supersedes: Vec::new(),
            contradicts: Vec::new(),
        };
        KnowledgeRecord::assemble(holder, &entry, &body(id, kind, subjects))
    }

    /// One holder entry as the model keeps it.
    #[derive(Clone, Debug)]
    struct ModelEntry {
        local: u64,
        kind: KnowledgeRecordKind,
        subjects: Vec<KnowledgeSubject>,
        supersedes: Vec<KnowledgeRecordId>,
        contradicts: Vec<KnowledgeRecordId>,
    }

    type ModelHolder = (u64, BTreeMap<KnowledgeRecordId, ModelEntry>);

    #[derive(Default)]
    struct Model {
        holders: BTreeMap<KnowledgeHolderRef, ModelHolder>,
    }

    impl Model {
        fn current(
            &self,
            holder: &KnowledgeHolderRef,
            kind: &KnowledgeRecordKind,
            subjects: &[KnowledgeSubject],
        ) -> Vec<KnowledgeRecordId> {
            let Some((_, entries)) = self.holders.get(holder) else {
                return Vec::new();
            };
            let superseded: BTreeSet<_> = entries
                .values()
                .flat_map(|entry| entry.supersedes.iter().copied())
                .collect();
            entries
                .iter()
                .rev()
                .filter(|(id, entry)| {
                    &entry.kind == kind
                        && !superseded.contains(id)
                        && subjects
                            .iter()
                            .all(|subject| entry.subjects.contains(subject))
                })
                .map(|(id, _)| *id)
                .collect()
        }

        /// Removes an entry and every name of it, as a retirement does.
        fn remove(&mut self, holder: &KnowledgeHolderRef, id: KnowledgeRecordId) -> bool {
            let Some((_, entries)) = self.holders.get_mut(holder) else {
                return false;
            };
            if entries.remove(&id).is_none() {
                return false;
            }
            for entry in entries.values_mut() {
                entry.supersedes.retain(|target| *target != id);
                entry.contradicts.retain(|target| *target != id);
            }
            true
        }

        fn holders_of(&self, id: KnowledgeRecordId) -> u32 {
            u32::try_from(
                self.holders
                    .values()
                    .filter(|(_, entries)| entries.contains_key(&id))
                    .count(),
            )
            .unwrap()
        }
    }

    fn assert_matches(ledgers: &HolderLedgers, model: &Model) {
        let held: BTreeSet<KnowledgeRecordId> = model
            .holders
            .values()
            .flat_map(|(_, entries)| entries.keys().copied())
            .collect();
        assert_eq!(ledgers.body_count(), held.len());
        assert!(
            ledgers
                .bodies()
                .map(|body| body.id)
                .eq(held.iter().copied())
        );
        for id in &held {
            assert_eq!(ledgers.body_holder_count(*id), model.holders_of(*id));
        }
        assert_eq!(
            ledgers.entry_count(),
            model
                .holders
                .values()
                .map(|(_, entries)| entries.len())
                .sum::<usize>()
        );
        for namespace in ["fixture.ledger0", "fixture.ledger1"] {
            assert_eq!(
                ledgers.record_count_in_namespace(namespace),
                held.iter()
                    .filter(|id| ledgers.body(**id).unwrap().schema.kind.namespace == namespace)
                    .count()
            );
        }
        let filters: Vec<Vec<KnowledgeSubject>> = vec![
            Vec::new(),
            vec![subject(0)],
            vec![subject(1)],
            vec![subject(0), subject(2)],
            vec![subject(3), subject(1), subject(2)],
        ];
        assert_eq!(ledgers.len(), model.holders.len());
        for (holder, (next_local, entries)) in &model.holders {
            let ledger = ledgers.get(holder).expect("a holder stays listed");
            assert_eq!(ledger.next_local().get(), *next_local);
            assert_eq!(ledger.len(), entries.len());
            for (id, entry) in entries {
                let stored = ledger.entry(id).expect("the holder holds the entry");
                assert_eq!(stored.local.get(), entry.local);
                assert_eq!(stored.supersedes, entry.supersedes);
                assert_eq!(stored.contradicts, entry.contradicts);
                assert_eq!(ledger.id_of(stored.local), Some(*id));
            }
            for kind in (0..2).map(kind) {
                for subjects in &filters {
                    assert_eq!(
                        current(ledger, &kind, subjects),
                        model.current(holder, &kind, subjects),
                        "{holder:?} {kind:?} {subjects:?}"
                    );
                }
            }
        }
    }

    fn current(
        ledger: &HolderLedger,
        kind: &KnowledgeRecordKind,
        subjects: &[KnowledgeSubject],
    ) -> Vec<KnowledgeRecordId> {
        ledger
            .current_records(kind, subjects)
            .map(|body| body.id)
            .collect()
    }

    /// One body published to one to three holders, each entry superseding
    /// and contradicting some entries of the same kind its holder holds.
    fn publish(
        sequence: &mut Sequence,
        ledgers: &mut HolderLedgers,
        model: &mut Model,
        id: KnowledgeRecordId,
    ) {
        let kind = kind(sequence.next(2));
        let subjects: Vec<_> = (0..4)
            .filter(|_| sequence.next(2) == 0)
            .map(subject)
            .collect();
        let first = sequence.next(3);
        let holders: BTreeSet<u64> = (0..=sequence.next(3))
            .map(|offset| (first + offset) % 3)
            .collect();
        let mut entries = Vec::new();
        for index in holders {
            let holder = holder(index);
            let (next_local, held) = model
                .holders
                .entry(holder.clone())
                .or_insert((1, BTreeMap::new()));
            let mut supersedes = Vec::new();
            let mut contradicts = Vec::new();
            for (target, entry) in held.iter() {
                if entry.kind != kind {
                    continue;
                }
                match sequence.next(6) {
                    0 => supersedes.push(*target),
                    1 => contradicts.push(*target),
                    _ => {}
                }
            }
            entries.push((
                holder,
                HolderEntry {
                    record: id,
                    local: HolderKnowledgeRecordId::new(*next_local),
                    supersedes: supersedes.clone(),
                    contradicts: contradicts.clone(),
                },
            ));
            held.insert(
                id,
                ModelEntry {
                    local: *next_local,
                    kind: kind.clone(),
                    subjects: subjects.clone(),
                    supersedes,
                    contradicts,
                },
            );
            *next_local += 1;
        }
        let published = Arc::new(body(id.get(), kind, subjects));
        let digest = KnowledgeBodyDigest::of(&published).unwrap();
        ledgers.insert_body(published, digest, entries).unwrap();
    }

    /// The index, the counts, the stored local IDs, and the commitment follow
    /// every publication and retirement: the model recomputes the current
    /// records from the entries alone, and the cached commitment equals one
    /// computed from scratch by a decoded copy.
    #[test]
    fn index_and_commitment_follow_every_publication_and_retirement() {
        follow_every_publication_and_retirement(false);
    }

    /// The same sequence with every kind under one index key and every
    /// subject under one key: each ledger notes the collision and its
    /// lookups compare every candidate exactly, so they still return what
    /// the model does.
    #[test]
    fn lookups_stay_exact_when_every_index_key_collides() {
        follow_every_publication_and_retirement(true);
    }

    /// Under colliding keys, a ledger that has filed one kind and one
    /// subject has not seen a collision, so a lookup compares the query with
    /// the values filed under the keys: another kind or subject under the
    /// same key matches nothing. Filing a second kind is a collision, after
    /// which the lookups compare every candidate.
    #[test]
    fn a_query_that_only_shares_a_key_matches_nothing() {
        encoding::COLLIDING_KEYS.set(true);
        let holder = holder(0);
        let mut ledgers = HolderLedgers::new();
        for id in 1..=3 {
            ledgers.insert(record(id, &holder, kind(0), vec![subject(0)]));
        }
        let ids = |ids: &[u64]| -> Vec<KnowledgeRecordId> {
            ids.iter().copied().map(KnowledgeRecordId::new).collect()
        };
        let lookups = |ledgers: &HolderLedgers| {
            let ledger = &ledgers[&holder];
            [
                current(ledger, &kind(0), &[]),
                current(ledger, &kind(0), &[subject(0)]),
                current(ledger, &kind(0), &[subject(1)]),
                current(ledger, &kind(1), &[]),
                current(ledger, &kind(1), &[subject(0)]),
            ]
        };
        assert!(!ledgers[&holder].key_collision);
        assert_eq!(
            lookups(&ledgers),
            [
                ids(&[3, 2, 1]),
                ids(&[3, 2, 1]),
                ids(&[]),
                ids(&[]),
                ids(&[])
            ]
        );
        ledgers.insert(record(4, &holder, kind(1), vec![subject(1)]));
        assert!(ledgers[&holder].key_collision);
        assert_eq!(
            lookups(&ledgers),
            [
                ids(&[3, 2, 1]),
                ids(&[3, 2, 1]),
                ids(&[]),
                ids(&[4]),
                ids(&[])
            ]
        );
        encoding::COLLIDING_KEYS.set(false);
    }

    fn follow_every_publication_and_retirement(colliding_keys: bool) {
        encoding::COLLIDING_KEYS.set(colliding_keys);
        let mut sequence = Sequence(0x6B6E_6F77);
        let mut ledgers = HolderLedgers::new();
        let mut model = Model::default();
        let mut next_id = 1;
        let mut retired = 0;
        for step in 0..700 {
            match sequence.next(10) {
                0..=5 => {
                    publish(
                        &mut sequence,
                        &mut ledgers,
                        &mut model,
                        KnowledgeRecordId::new(next_id),
                    );
                    next_id += 1;
                }
                // One entry retired from one holder.
                6 | 7 => {
                    let holder = holder(sequence.next(3));
                    let ids: Vec<_> = model
                        .holders
                        .get(&holder)
                        .map(|(_, entries)| entries.keys().copied().collect())
                        .unwrap_or_default();
                    if ids.is_empty() {
                        continue;
                    }
                    let id = sequence.pick(&ids);
                    assert!(model.remove(&holder, id));
                    assert!(ledgers.remove_entry(&holder, id).is_some());
                    retired += 1;
                }
                // What one entry supersedes retired, as the retention rule
                // does.
                8 => {
                    let holder = holder(sequence.next(3));
                    let candidates: Vec<_> = model
                        .holders
                        .get(&holder)
                        .map(|(_, entries)| {
                            entries
                                .iter()
                                .filter(|(_, entry)| !entry.supersedes.is_empty())
                                .map(|(id, entry)| (*id, entry.supersedes.clone()))
                                .collect()
                        })
                        .unwrap_or_default();
                    if candidates.is_empty() {
                        continue;
                    }
                    let (id, targets) = sequence.pick(&candidates);
                    for target in &targets {
                        assert!(model.remove(&holder, *target));
                    }
                    assert_eq!(ledgers.retire_superseded_by(&holder, id), targets.len());
                    let after = ledgers.get(&holder).unwrap().entry(&id).unwrap();
                    assert!(after.supersedes.is_empty());
                    retired += targets.len();
                }
                _ => {
                    let stale_root = ledgers.commitment_root();
                    ledgers.refresh_commitment();
                    assert_eq!(ledgers.commitment_root(), stale_root, "step {step}");
                }
            }
            assert_matches(&ledgers, &model);
        }
        assert!(
            ledgers.entry_count() > 150,
            "the sequence should leave a sizeable ledger"
        );
        assert!(retired > 50, "the sequence should retire entries");

        // A decoded copy has the same content, the same index, and the same
        // commitment, recomputed from scratch.
        let decoded: HolderLedgers =
            serde_json::from_value(serde_json::to_value(&ledgers).unwrap()).unwrap();
        assert_eq!(decoded, ledgers);
        assert_matches(&decoded, &model);
        for ledger in ledgers.holders.values().chain(decoded.holders.values()) {
            assert_eq!(ledger.key_collision, colliding_keys);
        }
        let root = ledgers.commitment_root();
        assert_eq!(decoded.commitment_root(), root);
        ledgers.refresh_commitment();
        assert_eq!(ledgers.commitment_root(), root);

        // A clone shares everything until one side changes, and a change to
        // the clone leaves the original as it was.
        let mut copy = ledgers.clone();
        assert!(copy.ptr_eq(&ledgers));
        copy.insert(record(next_id, &holder(0), kind(0), Vec::new()));
        assert!(!copy.ptr_eq(&ledgers));
        assert_ne!(copy.commitment_root(), root);
        assert_matches(&ledgers, &model);
        encoding::COLLIDING_KEYS.set(false);
    }

    /// Fails to compile when the type implements `DoubleEndedIterator`, in
    /// the way `static_assertions::assert_not_impl_any!` does: both blanket
    /// implementations then apply and the item path is ambiguous.
    macro_rules! assert_forward_only {
        ($type:ty) => {
            const _: fn() = || {
                trait AmbiguousIfDoubleEnded<A> {
                    fn some_item() {}
                }
                impl<T: ?Sized> AmbiguousIfDoubleEnded<()> for T {}
                struct DoubleEnded;
                impl<T: ?Sized + DoubleEndedIterator> AmbiguousIfDoubleEnded<DoubleEnded> for T {}
                let _ = <$type as AmbiguousIfDoubleEnded<_>>::some_item;
            };
        };
    }

    // `im` 15.1 drops elements when an ordered collection is walked backwards
    // past three tree levels, so the ledgers offer no backward walk at all.
    assert_forward_only!(HolderLedgersIter<'static>);
    assert_forward_only!(HolderRecordsIter<'static>);
    assert_forward_only!(HolderEntriesIter<'static>);
    assert_forward_only!(BodiesById<'static>);
    assert_forward_only!(CurrentRecords<'static>);

    /// Every read stays exact past the depth at which `im` 15.1's backward
    /// iteration drops elements (about 2,177 entries): one holder with 5,000
    /// current records, even IDs on one line, odd IDs on another, and odd IDs
    /// 2,051 to 2,071 on both, which a backward walk of the narrowest line
    /// used to skip. The commitment spans twenty chunks: the same ledger built
    /// beside another holder's records gives the same holder ledger, one
    /// changed body deep in a chunk changes the root, retiring an entry
    /// leaves later local IDs where they were, and retirement that empties
    /// whole chunks and drops a name across chunks leaves the cached root
    /// equal to one computed from scratch.
    #[test]
    fn reads_and_commitment_stay_exact_in_deep_ledgers() {
        let (a, b) = (subject(0), subject(1));
        let lines = |id: u64| match id {
            2_051..=2_071 if id % 2 == 1 => vec![a.clone(), b.clone()],
            _ if id.is_multiple_of(2) => vec![a.clone()],
            _ => vec![b.clone()],
        };
        let ledgers: HolderLedgers = (1..=5_000)
            .map(|id| record(id, &holder(0), kind(0), lines(id)))
            .collect();
        let ledger = ledgers.get(&holder(0)).expect("the holder holds records");
        let ids = |records: Vec<KnowledgeRecordId>| -> Vec<u64> {
            records.into_iter().map(KnowledgeRecordId::get).collect()
        };
        let both: Vec<u64> = (2_051..=2_071).rev().step_by(2).collect();
        assert_eq!(both.len(), 11);
        assert_eq!(
            ids(current(ledger, &kind(0), &[a.clone(), b.clone()])),
            both
        );
        assert_eq!(
            ids(current(ledger, &kind(0), &[b.clone(), a.clone()])),
            both
        );
        assert_eq!(
            ids(current(ledger, &kind(0), std::slice::from_ref(&a))),
            (1..=5_000)
                .rev()
                .filter(|id| lines(*id).contains(&a))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            ids(current(ledger, &kind(0), &[])),
            (1..=5_000).rev().collect::<Vec<_>>()
        );
        assert!(current(ledger, &kind(1), std::slice::from_ref(&a)).is_empty());

        let ascending: Vec<u64> = (1..=5_000).collect();
        assert_eq!(
            ledger
                .values()
                .map(|record| record.id.get())
                .collect::<Vec<_>>(),
            ascending
        );
        assert_eq!(
            ledger.iter().map(|(id, _)| id.get()).collect::<Vec<_>>(),
            ascending
        );
        assert_eq!(
            ledger
                .entries()
                .map(|(entry, _)| entry.local.get())
                .collect::<Vec<_>>(),
            ascending
        );
        assert_eq!(
            ledgers
                .bodies()
                .map(|body| body.id.get())
                .collect::<Vec<_>>(),
            ascending
        );
        assert_eq!(
            ledgers.last_body_id().map(KnowledgeRecordId::get),
            Some(5_000)
        );

        let mut interleaved = HolderLedgers::new();
        for id in 1..=5_000 {
            interleaved.insert(record(id, &holder(0), kind(0), lines(id)));
            if id % 1_000 == 0 {
                interleaved.insert(record(10_000 + id, &holder(1), kind(0), Vec::new()));
            }
        }
        assert_ne!(interleaved, ledgers);
        assert_eq!(interleaved.get(&holder(0)), ledgers.get(&holder(0)));
        for id in (1..=5).map(|step| KnowledgeRecordId::new(10_000 + step * 1_000)) {
            interleaved.remove_entry(&holder(1), id);
        }
        // The emptied holder stays listed with its counter, so the ledgers
        // still differ from ones it never joined.
        assert_eq!(interleaved.get(&holder(1)).unwrap().next_local().get(), 6);
        assert_ne!(interleaved.commitment_root(), ledgers.commitment_root());

        let root = ledgers.commitment_root();
        let mut decoded: HolderLedgers =
            serde_json::from_value(serde_json::to_value(&ledgers).unwrap()).unwrap();
        assert_eq!(decoded, ledgers);
        assert_eq!(decoded.commitment_root(), root);

        let mut wire = serde_json::to_value(&ledgers).unwrap();
        wire["bodies"][3_332]["payload"] = serde_json::json!({ "changed": true });
        let changed: HolderLedgers = serde_json::from_value(wire).unwrap();
        assert_ne!(changed, ledgers);
        assert_ne!(changed.commitment_root(), root);

        decoded.remove_entry(&holder(0), KnowledgeRecordId::new(3_333));
        let after = decoded.get(&holder(0)).unwrap();
        assert_eq!(
            after
                .local_of(&KnowledgeRecordId::new(3_334))
                .map(HolderKnowledgeRecordId::get),
            Some(3_334)
        );
        assert_eq!(after.id_of(HolderKnowledgeRecordId::new(3_333)), None);
        assert_ne!(decoded.commitment_root(), root);

        assert_retirement_across_chunks_refreshes_exactly(&a);
    }

    /// Retirement empties whole chunks and drops a name across chunks:
    /// 800 entries fill chunks 0 to 3, entry 600 (chunk 2) names entry 100
    /// (chunk 0), and every entry of chunks 0 and 1 retires. The refreshed
    /// cache drops the emptied chunks' roots and recomputes the namer's
    /// chunk, so its root is the one a decoded copy computes from scratch.
    fn assert_retirement_across_chunks_refreshes_exactly(line: &KnowledgeSubject) {
        let mut retiring: HolderLedgers = (1..=800)
            .map(|id| {
                let mut record = record(id, &holder(0), kind(0), vec![line.clone()]);
                if id == 600 {
                    record.contradicts = vec![KnowledgeRecordId::new(100)];
                }
                record
            })
            .collect();
        retiring.refresh_commitment();
        let full = retiring.commitment_root();
        for id in 1..=511 {
            assert!(
                retiring
                    .remove_entry(&holder(0), KnowledgeRecordId::new(id))
                    .is_some()
            );
        }
        let namer = retiring.get(&holder(0)).unwrap();
        assert!(
            namer
                .entry(&KnowledgeRecordId::new(600))
                .unwrap()
                .contradicts
                .is_empty()
        );
        assert_eq!(namer.len(), 289);
        retiring.refresh_commitment();
        let rebuilt: HolderLedgers =
            serde_json::from_value(serde_json::to_value(&retiring).unwrap()).unwrap();
        assert_eq!(rebuilt, retiring);
        assert_ne!(retiring.commitment_root(), full);
        assert_eq!(retiring.commitment_root(), rebuilt.commitment_root());
    }
}
