//! Persistent holder ledgers and their derived current-record index.

use crate::{KnowledgeRecord, KnowledgeSubject};
use canwu_core::{KnowledgeHolderRef, KnowledgeRecordId, KnowledgeRecordKind};
use im::{OrdMap, OrdSet};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::cmp::Reverse;
use std::fmt;
use std::ops::{Bound, Index};
use std::sync::Arc;

/// A current-record set, keyed newest first so that the newest-first walk is
/// a forward iteration. `im` 15.1's ordered collections drop elements when
/// iterated in reverse once a tree is three levels deep (from about 2,177
/// entries), so nothing in this module iterates one backwards.
type NewestFirst = OrdSet<Reverse<KnowledgeRecordId>>;

/// The generic knowledge records of every holder, keyed by holder and then by
/// global record ID.
///
/// The ledgers are a persistent map. A clone shares all of its structure with
/// the original and costs the same whatever the ledgers hold; inserting or
/// removing a record copies only the paths it changes. A runtime can therefore
/// keep a copy for rollback, or beside a view, without copying records.
///
/// Every record is filed under its own `holder` and `id`, so a record ID is
/// unique across holders and the key of a record always matches the record.
/// Besides holder order, the records can be walked in global ID order
/// ([`Self::records`]), which is the order the runtime allocates them in, and
/// counted by schema namespace ([`Self::record_count_in_namespace`]) without a
/// scan.
///
/// The encoding is a list of `{ "holder", "records" }` entries in holder
/// order, each holding its records in ID order: the same bytes the ordered-map
/// form of earlier releases wrote. Decoding refuses a record filed under
/// another holder's entry, a record ID or a holder that appears twice, and an
/// entry without records.
///
/// Iteration is forward only: no iterator of this module implements
/// `DoubleEndedIterator`. The highest entry is still a direct read
/// ([`Self::last_record_id`], [`HolderLedger::last_key_value`]).
#[derive(Clone, Default)]
pub struct HolderLedgers {
    holders: OrdMap<KnowledgeHolderRef, HolderLedger>,
    by_id: OrdMap<KnowledgeRecordId, Arc<KnowledgeRecord>>,
    namespaces: OrdMap<String, usize>,
}

/// One holder's generic knowledge records, keyed by global record ID, with an
/// index of its current records.
///
/// A record is current when no record of this ledger names it in
/// `supersedes`. The index files current records by schema kind and by
/// subject. It is derived from the records alone, kept up to date by every
/// insertion and removal through [`HolderLedgers`], rebuilt when the ledgers
/// are decoded, and never encoded, so two ledgers with the same records are
/// equal whatever order they were built in.
#[derive(Clone, Default)]
pub struct HolderLedger {
    records: OrdMap<KnowledgeRecordId, Arc<KnowledgeRecord>>,
    /// How many records of this ledger name each ID in `supersedes`.
    superseded: OrdMap<KnowledgeRecordId, u32>,
    /// Current records by schema kind, newest first.
    heads: OrdMap<KnowledgeRecordKind, NewestFirst>,
    /// Current records by schema kind and by each subject they carry, newest
    /// first.
    subject_heads: OrdMap<KnowledgeRecordKind, OrdMap<KnowledgeSubject, NewestFirst>>,
}

impl HolderLedgers {
    /// Ledgers that hold no record.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether no holder holds a record.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// The number of holders that hold at least one record.
    #[must_use]
    pub fn len(&self) -> usize {
        self.holders.len()
    }

    /// One holder's ledger, or `None` when it holds no record.
    #[must_use]
    pub fn get(&self, holder: &KnowledgeHolderRef) -> Option<&HolderLedger> {
        self.holders.get(holder)
    }

    /// Whether the holder holds at least one record.
    #[must_use]
    pub fn contains_key(&self, holder: &KnowledgeHolderRef) -> bool {
        self.holders.contains_key(holder)
    }

    /// Every holder with its ledger, in holder order.
    #[must_use]
    pub fn iter(&self) -> HolderLedgersIter<'_> {
        HolderLedgersIter {
            inner: self.holders.iter(),
        }
    }

    /// Every holder that holds a record, in holder order.
    pub fn keys(&self) -> impl ExactSizeIterator<Item = &KnowledgeHolderRef> {
        self.holders.keys()
    }

    /// Every holder's ledger, in holder order.
    pub fn values(&self) -> impl ExactSizeIterator<Item = &HolderLedger> {
        self.holders.values()
    }

    /// The number of records across every holder.
    #[must_use]
    pub fn record_count(&self) -> usize {
        self.by_id.len()
    }

    /// The record with a global ID, whichever holder holds it.
    #[must_use]
    pub fn record(&self, id: KnowledgeRecordId) -> Option<&KnowledgeRecord> {
        self.by_id.get(&id).map(Arc::as_ref)
    }

    /// Every record across every holder, in global ID order.
    #[must_use]
    pub fn records(&self) -> RecordsById<'_> {
        RecordsById {
            inner: self.by_id.range(..),
        }
    }

    /// The records whose global ID is greater than `after`, in global ID
    /// order; every record when `after` is `None`.
    #[must_use]
    pub fn records_after(&self, after: Option<KnowledgeRecordId>) -> RecordsById<'_> {
        let inner = match after {
            Some(after) => self.by_id.range((Bound::Excluded(after), Bound::Unbounded)),
            None => self.by_id.range(..),
        };
        RecordsById { inner }
    }

    /// The highest global record ID held.
    #[must_use]
    pub fn last_record_id(&self) -> Option<KnowledgeRecordId> {
        self.by_id.get_max().map(|(id, _)| *id)
    }

    /// The number of records, across every holder, whose schema kind is in
    /// `namespace`.
    #[must_use]
    pub fn record_count_in_namespace(&self, namespace: &str) -> usize {
        self.namespaces.get(namespace).copied().unwrap_or_default()
    }

    /// Files a record under its holder and ID, and returns the record it
    /// replaces: the one held under the same global ID, whichever holder
    /// held it.
    pub fn insert(&mut self, record: KnowledgeRecord) -> Option<KnowledgeRecord> {
        let replaced = self.remove(record.id);
        let record = Arc::new(record);
        *entry_or_default(&mut self.namespaces, &record.schema.kind.namespace) += 1;
        self.by_id.insert(record.id, Arc::clone(&record));
        entry_or_default(&mut self.holders, &record.holder).insert(record);
        replaced
    }

    /// Removes the record with a global ID and returns it. A holder whose
    /// last record is removed is no longer listed.
    pub fn remove(&mut self, id: KnowledgeRecordId) -> Option<KnowledgeRecord> {
        let record = self.by_id.remove(&id)?;
        let namespace = &record.schema.kind.namespace;
        if let Some(count) = self.namespaces.get_mut(namespace) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.namespaces.remove(namespace);
            }
        }
        if let Some(ledger) = self.holders.get_mut(&record.holder) {
            ledger.remove(&record);
            if ledger.is_empty() {
                self.holders.remove(&record.holder);
            }
        }
        Some(Arc::unwrap_or_clone(record))
    }

    /// Whether both ledgers share all of their structure, which a clone does
    /// until either side changes. Equal ledgers built apart do not.
    #[must_use]
    pub fn ptr_eq(&self, other: &Self) -> bool {
        self.by_id.ptr_eq(&other.by_id)
            && self.holders.ptr_eq(&other.holders)
            && self.namespaces.ptr_eq(&other.namespaces)
    }
}

impl HolderLedger {
    /// The number of records the holder holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether the holder holds no record.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// The holder's record with a global ID.
    #[must_use]
    pub fn get(&self, id: &KnowledgeRecordId) -> Option<&KnowledgeRecord> {
        self.records.get(id).map(Arc::as_ref)
    }

    /// Whether the holder holds the record with a global ID.
    #[must_use]
    pub fn contains_key(&self, id: &KnowledgeRecordId) -> bool {
        self.records.contains_key(id)
    }

    /// The holder's records with their IDs, in ID order.
    #[must_use]
    pub fn iter(&self) -> HolderRecordsIter<'_> {
        HolderRecordsIter {
            inner: self.records.iter(),
        }
    }

    /// The holder's record IDs, in ascending order.
    pub fn keys(&self) -> impl ExactSizeIterator<Item = &KnowledgeRecordId> {
        self.records.keys()
    }

    /// The holder's records, in ID order.
    pub fn values(&self) -> impl ExactSizeIterator<Item = &KnowledgeRecord> {
        self.records.values().map(Arc::as_ref)
    }

    /// The holder's record with the lowest ID.
    #[must_use]
    pub fn first_key_value(&self) -> Option<(&KnowledgeRecordId, &KnowledgeRecord)> {
        self.records
            .get_min()
            .map(|(id, record)| (id, record.as_ref()))
    }

    /// The holder's record with the highest ID.
    #[must_use]
    pub fn last_key_value(&self) -> Option<(&KnowledgeRecordId, &KnowledgeRecord)> {
        self.records
            .get_max()
            .map(|(id, record)| (id, record.as_ref()))
    }

    /// Whether the holder holds the record and no record of the holder
    /// supersedes it.
    #[must_use]
    pub fn is_current(&self, id: &KnowledgeRecordId) -> bool {
        self.records.contains_key(id) && !self.superseded.contains_key(id)
    }

    /// Whether some record of the holder names `id` in `supersedes`, whether
    /// or not the holder holds `id`.
    #[must_use]
    pub fn is_superseded(&self, id: &KnowledgeRecordId) -> bool {
        self.superseded.contains_key(id)
    }

    /// The holder's current records of `kind` that carry every one of
    /// `subjects`, newest first; with no subjects, every current record of
    /// `kind`.
    ///
    /// The walk reads the index, not the ledger: it visits the current
    /// records of `kind` that carry the listed subject held by the fewest of
    /// them, so it never reads a superseded record, and a caller that stops
    /// after `n` matches pays for the matches and for the current records of
    /// that subject it skipped.
    #[must_use]
    pub fn current_records<'a>(
        &'a self,
        kind: &KnowledgeRecordKind,
        subjects: &'a [KnowledgeSubject],
    ) -> CurrentRecords<'a> {
        let candidates = if subjects.is_empty() {
            self.heads.get(kind)
        } else {
            let by_subject = self.subject_heads.get(kind);
            let mut narrowest: Option<&NewestFirst> = None;
            let mut missing = by_subject.is_none();
            for subject in subjects {
                match by_subject.and_then(|index| index.get(subject)) {
                    Some(set) => {
                        if narrowest.is_none_or(|current| set.len() < current.len()) {
                            narrowest = Some(set);
                        }
                    }
                    None => missing = true,
                }
            }
            if missing { None } else { narrowest }
        };
        CurrentRecords {
            ledger: self,
            subjects,
            candidates: candidates.map(OrdSet::iter),
        }
    }

    fn insert(&mut self, record: Arc<KnowledgeRecord>) {
        for target in &record.supersedes {
            let count = self.superseded.entry(*target).or_default();
            *count = count.saturating_add(1);
            if *count == 1
                && let Some(target) = self.records.get(target).cloned()
            {
                self.unindex(&target);
            }
        }
        if !self.superseded.contains_key(&record.id) {
            self.index(&record);
        }
        self.records.insert(record.id, record);
    }

    fn remove(&mut self, record: &KnowledgeRecord) {
        if self.records.remove(&record.id).is_none() {
            return;
        }
        if !self.superseded.contains_key(&record.id) {
            self.unindex(record);
        }
        for target in &record.supersedes {
            let Some(count) = self.superseded.get_mut(target) else {
                continue;
            };
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.superseded.remove(target);
                if let Some(target) = self.records.get(target).cloned() {
                    self.index(&target);
                }
            }
        }
    }

    fn index(&mut self, record: &KnowledgeRecord) {
        let kind = &record.schema.kind;
        entry_or_default(&mut self.heads, kind).insert(Reverse(record.id));
        if record.subjects.is_empty() {
            return;
        }
        let by_subject = entry_or_default(&mut self.subject_heads, kind);
        for subject in &record.subjects {
            entry_or_default(by_subject, subject).insert(Reverse(record.id));
        }
    }

    fn unindex(&mut self, record: &KnowledgeRecord) {
        let kind = &record.schema.kind;
        if let Some(heads) = self.heads.get_mut(kind) {
            heads.remove(&Reverse(record.id));
            if heads.is_empty() {
                self.heads.remove(kind);
            }
        }
        if let Some(by_subject) = self.subject_heads.get_mut(kind) {
            for subject in &record.subjects {
                if let Some(heads) = by_subject.get_mut(subject) {
                    heads.remove(&Reverse(record.id));
                    if heads.is_empty() {
                        by_subject.remove(subject);
                    }
                }
            }
            if by_subject.is_empty() {
                self.subject_heads.remove(kind);
            }
        }
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
    inner: im::ordmap::Iter<'a, KnowledgeHolderRef, HolderLedger>,
}

impl<'a> Iterator for HolderLedgersIter<'a> {
    type Item = (&'a KnowledgeHolderRef, &'a HolderLedger);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
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

/// Iterator over one holder's records with their IDs, from
/// [`HolderLedger::iter`].
pub struct HolderRecordsIter<'a> {
    inner: im::ordmap::Iter<'a, KnowledgeRecordId, Arc<KnowledgeRecord>>,
}

impl<'a> Iterator for HolderRecordsIter<'a> {
    type Item = (&'a KnowledgeRecordId, &'a KnowledgeRecord);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|(id, record)| (id, record.as_ref()))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl ExactSizeIterator for HolderRecordsIter<'_> {}

impl<'a> IntoIterator for &'a HolderLedger {
    type Item = (&'a KnowledgeRecordId, &'a KnowledgeRecord);
    type IntoIter = HolderRecordsIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Iterator over records in global ID order, from [`HolderLedgers::records`]
/// and [`HolderLedgers::records_after`].
pub struct RecordsById<'a> {
    inner: im::ordmap::Iter<'a, KnowledgeRecordId, Arc<KnowledgeRecord>>,
}

impl<'a> Iterator for RecordsById<'a> {
    type Item = &'a KnowledgeRecord;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|(_, record)| record.as_ref())
    }
}

/// Iterator over a holder's current records of one kind, newest first, from
/// [`HolderLedger::current_records`].
pub struct CurrentRecords<'a> {
    ledger: &'a HolderLedger,
    subjects: &'a [KnowledgeSubject],
    candidates: Option<im::ordset::Iter<'a, Reverse<KnowledgeRecordId>>>,
}

impl<'a> Iterator for CurrentRecords<'a> {
    type Item = &'a KnowledgeRecord;

    fn next(&mut self) -> Option<Self::Item> {
        let candidates = self.candidates.as_mut()?;
        for Reverse(id) in candidates.by_ref() {
            let Some(record) = self.ledger.get(id) else {
                continue;
            };
            if self
                .subjects
                .iter()
                .all(|subject| record.subjects.contains(subject))
            {
                return Some(record);
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
    /// Panics when the holder holds no record, as an ordered map does for a
    /// missing key.
    fn index(&self, holder: &KnowledgeHolderRef) -> &Self::Output {
        self.get(holder)
            .expect("the knowledge holder holds no record")
    }
}

impl Index<&KnowledgeRecordId> for HolderLedger {
    type Output = KnowledgeRecord;

    /// Returns the holder's record with a global ID.
    ///
    /// # Panics
    ///
    /// Panics when the holder does not hold the record, as an ordered map
    /// does for a missing key.
    fn index(&self, id: &KnowledgeRecordId) -> &Self::Output {
        self.get(id)
            .expect("the knowledge holder does not hold the record")
    }
}

impl PartialEq for HolderLedgers {
    fn eq(&self, other: &Self) -> bool {
        self.by_id == other.by_id
    }
}

impl Eq for HolderLedgers {}

impl PartialEq for HolderLedger {
    fn eq(&self, other: &Self) -> bool {
        self.records == other.records
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
        formatter.debug_map().entries(self.iter()).finish()
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

#[derive(Serialize)]
struct HolderLedgerEntryRef<'a> {
    holder: &'a KnowledgeHolderRef,
    records: Vec<&'a KnowledgeRecord>,
}

#[derive(Deserialize)]
struct HolderLedgerEntry {
    holder: KnowledgeHolderRef,
    records: Vec<KnowledgeRecord>,
}

impl Serialize for HolderLedgers {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.iter().map(|(holder, ledger)| HolderLedgerEntryRef {
            holder,
            records: ledger.values().collect(),
        }))
    }
}

impl<'de> Deserialize<'de> for HolderLedgers {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let entries = Vec::<HolderLedgerEntry>::deserialize(deserializer)?;
        let mut ledgers = Self::new();
        for entry in entries {
            if entry.records.is_empty() || ledgers.contains_key(&entry.holder) {
                return Err(D::Error::custom(
                    "knowledge snapshot contains an empty or duplicate holder ledger",
                ));
            }
            for record in entry.records {
                if record.holder != entry.holder || ledgers.by_id.contains_key(&record.id) {
                    return Err(D::Error::custom(
                        "holder ledger contains a mismatched holder or duplicate record ID",
                    ));
                }
                ledgers.insert(record);
            }
        }
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

    fn record(
        id: u64,
        holder: KnowledgeHolderRef,
        kind: KnowledgeRecordKind,
        subjects: Vec<KnowledgeSubject>,
        supersedes: Vec<KnowledgeRecordId>,
    ) -> KnowledgeRecord {
        KnowledgeRecord {
            id: KnowledgeRecordId::new(id),
            holder,
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
            supersedes,
            contradicts: Vec::new(),
        }
    }

    /// What the index must answer, recomputed from the records alone: the
    /// records of `holder` and `kind` that carry every subject and that no
    /// record of the same holder supersedes, newest first.
    fn expected_current(
        model: &BTreeMap<KnowledgeRecordId, KnowledgeRecord>,
        holder: &KnowledgeHolderRef,
        kind: &KnowledgeRecordKind,
        subjects: &[KnowledgeSubject],
    ) -> Vec<KnowledgeRecordId> {
        let held: Vec<_> = model
            .values()
            .filter(|record| &record.holder == holder)
            .collect();
        let superseded: BTreeSet<_> = held
            .iter()
            .flat_map(|record| record.supersedes.iter().copied())
            .collect();
        held.iter()
            .rev()
            .filter(|record| {
                &record.schema.kind == kind
                    && !superseded.contains(&record.id)
                    && subjects
                        .iter()
                        .all(|subject| record.subjects.contains(subject))
            })
            .map(|record| record.id)
            .collect()
    }

    fn assert_matches(
        ledgers: &HolderLedgers,
        model: &BTreeMap<KnowledgeRecordId, KnowledgeRecord>,
    ) {
        assert_eq!(ledgers.record_count(), model.len());
        assert!(ledgers.records().eq(model.values()));
        for namespace in ["fixture.ledger0", "fixture.ledger1"] {
            assert_eq!(
                ledgers.record_count_in_namespace(namespace),
                model
                    .values()
                    .filter(|record| record.schema.kind.namespace == namespace)
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
        for holder in (0..3).map(holder) {
            let ledger = ledgers.get(&holder);
            assert_eq!(
                ledger.is_some(),
                model.values().any(|record| record.holder == holder),
                "a holder is listed exactly while it holds a record"
            );
            for kind in (0..2).map(kind) {
                for subjects in &filters {
                    let actual: Vec<_> = ledger
                        .map(|ledger| {
                            ledger
                                .current_records(&kind, subjects)
                                .map(|record| record.id)
                                .collect()
                        })
                        .unwrap_or_default();
                    assert_eq!(
                        actual,
                        expected_current(model, &holder, &kind, subjects),
                        "{holder:?} {kind:?} {subjects:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn current_record_index_follows_every_edit_and_every_build_order() {
        let mut sequence = Sequence(0x6B6E_6F77);
        let mut ledgers = HolderLedgers::new();
        let mut model = BTreeMap::<KnowledgeRecordId, KnowledgeRecord>::new();
        let mut next_id = 1;
        for _ in 0..600 {
            let existing: Vec<_> = model.keys().copied().collect();
            let pick = |sequence: &mut Sequence| {
                (!existing.is_empty()).then(|| {
                    let bound = u64::try_from(existing.len()).unwrap();
                    existing[usize::try_from(sequence.next(bound)).unwrap()]
                })
            };
            match sequence.next(10) {
                // Mostly appends, as a run makes them. Some name an ID twice,
                // a record of another holder, or a record that does not exist
                // yet, as a tampered ledger can.
                0..=6 => {
                    let holder_index = sequence.next(3);
                    let mut supersedes = Vec::new();
                    for _ in 0..sequence.next(3) {
                        if sequence.next(8) == 0 {
                            supersedes.push(KnowledgeRecordId::new(next_id + 5));
                        } else {
                            supersedes.extend(pick(&mut sequence));
                        }
                    }
                    let subjects = (0..4)
                        .filter(|_| sequence.next(2) == 0)
                        .map(subject)
                        .collect();
                    let added = record(
                        next_id,
                        holder(holder_index),
                        kind(sequence.next(2)),
                        subjects,
                        supersedes,
                    );
                    next_id += 1;
                    assert_eq!(ledgers.insert(added.clone()), None);
                    model.insert(added.id, added);
                }
                // Replaces a record in place, possibly moving it to another
                // holder and changing what it supersedes.
                7 | 8 => {
                    let Some(id) = pick(&mut sequence) else {
                        continue;
                    };
                    let mut changed = model[&id].clone();
                    changed.holder = holder(sequence.next(3));
                    changed.schema.kind = kind(sequence.next(2));
                    changed.supersedes = pick(&mut sequence).into_iter().collect();
                    changed.subjects = vec![subject(sequence.next(4))];
                    assert_eq!(ledgers.insert(changed.clone()).as_ref(), model.get(&id));
                    model.insert(id, changed);
                }
                _ => {
                    let Some(id) = pick(&mut sequence) else {
                        continue;
                    };
                    assert_eq!(ledgers.remove(id), model.remove(&id));
                }
            }
            assert_matches(&ledgers, &model);
        }
        assert!(
            model.len() > 100,
            "the sequence should leave a sizeable ledger"
        );

        // The index is a function of the records: a decoded copy and a copy
        // built newest first give the same answers and compare equal.
        let decoded: HolderLedgers =
            serde_json::from_value(serde_json::to_value(&ledgers).unwrap()).unwrap();
        assert_eq!(decoded, ledgers);
        assert_matches(&decoded, &model);
        let reversed: HolderLedgers = model.values().rev().cloned().collect();
        assert_eq!(reversed, ledgers);
        assert_matches(&reversed, &model);

        // A clone shares everything until one side changes, and a change to
        // the clone leaves the original as it was.
        let mut copy = ledgers.clone();
        assert!(copy.ptr_eq(&ledgers));
        copy.insert(record(next_id, holder(0), kind(0), Vec::new(), Vec::new()));
        assert!(!copy.ptr_eq(&ledgers));
        assert_matches(&ledgers, &model);
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
    assert_forward_only!(RecordsById<'static>);
    assert_forward_only!(CurrentRecords<'static>);

    /// Every read stays exact past the depth at which `im` 15.1's backward
    /// iteration drops elements (about 2,177 entries): one holder with 5,000
    /// current records, even IDs on one line, odd IDs on another, and odd IDs
    /// 2,051 to 2,071 on both, which a backward walk of the narrowest line
    /// used to skip.
    #[test]
    fn reads_stay_exact_in_deep_ledgers() {
        let (a, b) = (subject(0), subject(1));
        let lines = |id: u64| match id {
            2_051..=2_071 if id % 2 == 1 => vec![a.clone(), b.clone()],
            _ if id.is_multiple_of(2) => vec![a.clone()],
            _ => vec![b.clone()],
        };
        let built = |ids: &mut dyn Iterator<Item = u64>| -> HolderLedgers {
            ids.map(|id| record(id, holder(0), kind(0), lines(id), Vec::new()))
                .collect()
        };
        let ledgers = built(&mut (1..=5_000));
        let ledger = ledgers.get(&holder(0)).expect("the holder holds records");
        let current = |ledger: &HolderLedger, subjects: &[KnowledgeSubject]| -> Vec<u64> {
            ledger
                .current_records(&kind(0), subjects)
                .map(|record| record.id.get())
                .collect()
        };
        let both: Vec<u64> = (2_051..=2_071).rev().step_by(2).collect();
        assert_eq!(both.len(), 11);
        assert_eq!(current(ledger, &[a.clone(), b.clone()]), both);
        assert_eq!(current(ledger, &[b.clone(), a.clone()]), both);
        assert_eq!(
            current(ledger, std::slice::from_ref(&a)),
            (1..=5_000)
                .rev()
                .filter(|id| lines(*id).contains(&a))
                .collect::<Vec<_>>()
        );
        assert_eq!(current(ledger, &[]), (1..=5_000).rev().collect::<Vec<_>>());

        let ascending: Vec<u64> = (1..=5_000).collect();
        let ids = |records: &mut dyn Iterator<Item = &KnowledgeRecord>| -> Vec<u64> {
            records.map(|record| record.id.get()).collect()
        };
        assert_eq!(ids(&mut ledger.values()), ascending);
        assert_eq!(ids(&mut ledger.iter().map(|(_, record)| record)), ascending);
        assert_eq!(
            ledger.keys().map(|id| id.get()).collect::<Vec<_>>(),
            ascending
        );
        assert_eq!(ids(&mut ledgers.records()), ascending);
        assert_eq!(
            ids(&mut ledgers.records_after(Some(KnowledgeRecordId::new(2_500)))),
            (2_501..=5_000).collect::<Vec<_>>()
        );
        assert_eq!(ledger.first_key_value().map(|(id, _)| id.get()), Some(1));
        assert_eq!(ledger.last_key_value().map(|(id, _)| id.get()), Some(5_000));
        assert_eq!(
            ledgers.last_record_id().map(KnowledgeRecordId::get),
            Some(5_000)
        );

        // A decoded copy and a copy built newest first read the same and are
        // equal; one changed record deep in the tree makes them unequal.
        let decoded: HolderLedgers =
            serde_json::from_value(serde_json::to_value(&ledgers).unwrap()).unwrap();
        let reversed = built(&mut (1..=5_000).rev());
        for copy in [&decoded, &reversed] {
            assert_eq!(copy, &ledgers);
            let ledger = copy.get(&holder(0)).expect("the holder holds records");
            assert_eq!(current(ledger, &[a.clone(), b.clone()]), both);
        }
        let mut changed = reversed;
        let mut deep = changed
            .record(KnowledgeRecordId::new(3_333))
            .cloned()
            .expect("the record is held");
        deep.payload = serde_json::json!({ "changed": true });
        changed.insert(deep);
        assert_ne!(changed, ledgers);
    }
}
