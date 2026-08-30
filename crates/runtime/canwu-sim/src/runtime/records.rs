use super::{
    CanwuError, ErrorCode, MAX_STATE_DELTA_PAGES, PayloadSchema, StateKey, StatePageBlob,
    StatePageProvider, StateVisibility, canonical_byte_hash, canonical_hash, state_page_id,
};
use canwu_core::{
    CoreEntityKind, DomainEntityType, DomainKindClass, DomainRecordKind, DomainRecordRef,
    DomainRecordType, DomainValueType, EntityRef, KnowledgeHolderPolicy, TypedDomainRecordRef,
};
use canwu_time::SimTime;
use im::{HashMap as PersistentHashMap, HashSet as PersistentHashSet, Vector};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::mem::size_of;
use std::sync::Arc;

/// Independent commitment roots for the Format-8 domain-record store.
///
/// The current runtime still materializes the public snapshot in canonical
/// reference order, but mutations and forks can carry this immutable handle
/// without treating reverse/successor indexes as uncommitted caches.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DomainRecordCommitmentRoots {
    pub primary: String,
    pub reverse_references: String,
    pub successor_of: String,
    pub predecessors_of: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DomainRecordPageRoots {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reverse_references: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub successor_of: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predecessors_of: Option<String>,
    pub commitment_roots: DomainRecordCommitmentRoots,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PatriciaStoreMetrics {
    pub entries: u64,
    pub logical_nodes: u64,
    pub leaf_nodes: u64,
    pub branch_nodes: u64,
    pub structural_bytes: u64,
    pub estimated_resident_bytes: u64,
    pub depth_p50: u16,
    pub depth_p95: u16,
    pub depth_p99: u16,
    pub max_depth: u16,
    pub root_hash: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DomainStoreScaleMetrics {
    pub records: u64,
    pub key_pages: u64,
    pub record_hamt_entries: u64,
    pub reverse_lookup_keys: u64,
    pub reverse_reference_edges: u64,
    pub successor_lookup_entries: u64,
    pub predecessor_lookup_keys: u64,
    pub predecessor_edges: u64,
    pub total_patricia_nodes: u64,
    pub total_patricia_structural_bytes: u64,
    pub total_patricia_estimated_resident_bytes: u64,
    pub primary: PatriciaStoreMetrics,
    pub reverse_references: PatriciaStoreMetrics,
    pub successor_of: PatriciaStoreMetrics,
    pub predecessors_of: PatriciaStoreMetrics,
}

#[derive(Clone, Debug)]
pub struct PersistentDomainRecordStore {
    records: PersistentHashMap<DomainRecordRef, Arc<DomainRecord>>,
    record_key_pages: Vector<Arc<Vec<DomainRecordRef>>>,
    reverse_lookup: PersistentHashMap<DomainRecordRef, PersistentHashSet<DomainRecordRef>>,
    successor_lookup: PersistentHashMap<DomainRecordRef, DomainRecordRef>,
    predecessor_lookup: PersistentHashMap<DomainRecordRef, PersistentHashSet<DomainRecordRef>>,
    primary: PersistentPatricia,
    reverse_references: PersistentPatricia,
    successor_of: PersistentPatricia,
    predecessors_of: PersistentPatricia,
    roots: DomainRecordCommitmentRoots,
    primary_leaf_count: usize,
}

const DOMAIN_RECORD_KEY_PAGE_CAPACITY: usize = 256;
const MAX_AFFECTED_RECORD_VALIDATION_CLOSURE: usize = 16_384;

impl PersistentDomainRecordStore {
    pub fn from_records(
        records: BTreeMap<DomainRecordRef, DomainRecord>,
    ) -> Result<Self, CanwuError> {
        let mut store = Self::empty();
        for (reference, record) in &records {
            store.primary.insert(reference, record)?;
            store.add_indexes(record)?;
        }
        let sorted = records.into_iter().collect::<Vec<_>>();
        for chunk in sorted.chunks(DOMAIN_RECORD_KEY_PAGE_CAPACITY) {
            store.record_key_pages.push_back(Arc::new(
                chunk
                    .iter()
                    .map(|(reference, _)| reference.clone())
                    .collect(),
            ));
            for (reference, record) in chunk {
                store
                    .records
                    .insert(reference.clone(), Arc::new(record.clone()));
                store.add_lookup_indexes(record);
            }
        }
        store.refresh_roots();
        Ok(store)
    }

    #[must_use]
    pub fn get(&self, reference: &DomainRecordRef) -> Option<&DomainRecord> {
        self.records.get(reference).map(Arc::as_ref)
    }

    #[must_use]
    pub fn contains_key(&self, reference: &DomainRecordRef) -> bool {
        self.records.contains_key(reference)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&DomainRecordRef, &DomainRecord)> {
        self.record_key_pages.iter().flat_map(|page| {
            page.iter().filter_map(|reference| {
                self.records
                    .get(reference)
                    .map(|record| (reference, record.as_ref()))
            })
        })
    }

    pub fn values(&self) -> impl Iterator<Item = &DomainRecord> {
        self.iter().map(|(_, record)| record)
    }

    #[must_use]
    pub fn roots(&self) -> &DomainRecordCommitmentRoots {
        &self.roots
    }

    /// Number of canonical primary leaves. It is independent of insertion
    /// order and is useful for preflight budgets before a cold page is loaded.
    #[must_use]
    pub const fn primary_leaf_count(&self) -> usize {
        self.primary_leaf_count
    }

    #[must_use]
    pub fn materialize(&self) -> BTreeMap<DomainRecordRef, DomainRecord> {
        self.iter()
            .map(|(reference, record)| (reference.clone(), record.clone()))
            .collect()
    }

    /// True when two handles share the same persistent root. This is exposed
    /// for allocation/fork conformance tests without exposing trie nodes.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn shares_root_with(&self, other: &Self) -> bool {
        match (&self.primary.root, &other.primary.root) {
            (None, None) => true,
            (Some(left), Some(right)) => Arc::ptr_eq(left, right),
            _ => false,
        }
    }

    pub(crate) fn commitment_root(&self) -> Result<String, CanwuError> {
        canonical_hash("canwu.format8.domain-record.roots.v1", &self.roots)
    }

    pub fn state_pages(&self) -> Result<(DomainRecordPageRoots, Vec<StatePageBlob>), CanwuError> {
        let mut pages = BTreeMap::new();
        let roots = DomainRecordPageRoots {
            primary: collect_patricia_pages(&self.primary, &mut pages)?,
            reverse_references: collect_patricia_pages(&self.reverse_references, &mut pages)?,
            successor_of: collect_patricia_pages(&self.successor_of, &mut pages)?,
            predecessors_of: collect_patricia_pages(&self.predecessors_of, &mut pages)?,
            commitment_roots: self.roots.clone(),
        };
        if pages.len() > MAX_STATE_DELTA_PAGES {
            return Err(invalid_patricia_page(
                "domain record store exceeds the bounded state-page count",
            ));
        }
        Ok((roots, pages.into_values().collect()))
    }

    /// Emits only page paths whose content addresses are not already present.
    /// An existing node page is a closure boundary: its child page identities
    /// are committed by its bytes and were retained with that prior root.
    pub fn missing_state_pages(
        &self,
        provider: &dyn StatePageProvider,
    ) -> Result<(DomainRecordPageRoots, Vec<StatePageBlob>), CanwuError> {
        let mut pages = BTreeMap::new();
        let roots = DomainRecordPageRoots {
            primary: collect_missing_patricia_pages(&self.primary, provider, &mut pages)?,
            reverse_references: collect_missing_patricia_pages(
                &self.reverse_references,
                provider,
                &mut pages,
            )?,
            successor_of: collect_missing_patricia_pages(&self.successor_of, provider, &mut pages)?,
            predecessors_of: collect_missing_patricia_pages(
                &self.predecessors_of,
                provider,
                &mut pages,
            )?,
            commitment_roots: self.roots.clone(),
        };
        if pages.len() > MAX_STATE_DELTA_PAGES {
            return Err(invalid_patricia_page(
                "domain record delta exceeds the bounded state-page count",
            ));
        }
        Ok((roots, pages.into_values().collect()))
    }

    #[must_use]
    pub fn primary_metrics(&self) -> PatriciaStoreMetrics {
        self.primary.metrics()
    }

    pub fn from_state_pages(
        roots: &DomainRecordPageRoots,
        provider: &dyn StatePageProvider,
    ) -> Result<Self, CanwuError> {
        let mut cache = BTreeMap::new();
        let primary = load_patricia_root(
            "canwu.format8.domain-record.primary.v1",
            roots.primary.as_deref(),
            provider,
            &mut cache,
        )?;
        let reverse_references = load_patricia_root(
            "canwu.format8.domain-record.reverse.v1",
            roots.reverse_references.as_deref(),
            provider,
            &mut cache,
        )?;
        let successor_of = load_patricia_root(
            "canwu.format8.domain-record.successor.v1",
            roots.successor_of.as_deref(),
            provider,
            &mut cache,
        )?;
        let predecessors_of = load_patricia_root(
            "canwu.format8.domain-record.predecessors.v1",
            roots.predecessors_of.as_deref(),
            provider,
            &mut cache,
        )?;
        let mut materialized = BTreeMap::new();
        if let Some(root) = &primary.root {
            collect_primary_records(root, &mut materialized)?;
        }
        let rebuilt = Self::from_records(materialized)?;
        let loaded = Self {
            records: rebuilt.records,
            record_key_pages: rebuilt.record_key_pages,
            reverse_lookup: rebuilt.reverse_lookup,
            successor_lookup: rebuilt.successor_lookup,
            predecessor_lookup: rebuilt.predecessor_lookup,
            primary,
            reverse_references,
            successor_of,
            predecessors_of,
            roots: roots.commitment_roots.clone(),
            primary_leaf_count: rebuilt.primary_leaf_count,
        };
        let actual_roots = DomainRecordCommitmentRoots {
            primary: loaded.primary.root_hash(),
            reverse_references: loaded.reverse_references.root_hash(),
            successor_of: loaded.successor_of.root_hash(),
            predecessors_of: loaded.predecessors_of.root_hash(),
        };
        if actual_roots != roots.commitment_roots || rebuilt.roots != roots.commitment_roots {
            return Err(invalid_patricia_page(
                "state pages do not reconstruct the committed domain-record indexes",
            ));
        }
        loaded.validate_internal_indexes()?;
        Ok(loaded)
    }

    /// Performs the cold-load consistency audit that ordinary COW mutations
    /// intentionally avoid. Every HAMT entry, ordered key page, lookup index,
    /// leaf count, and Patricia commitment must describe the same record set.
    pub(crate) fn validate_internal_indexes(&self) -> Result<(), CanwuError> {
        if self.records.len() != self.primary_leaf_count {
            return Err(invalid_patricia_page(
                "domain record HAMT count disagrees with the primary leaf count",
            ));
        }
        let mut paged_keys = BTreeSet::new();
        let mut previous: Option<&DomainRecordRef> = None;
        for page in &self.record_key_pages {
            if page.is_empty()
                || page.len() > DOMAIN_RECORD_KEY_PAGE_CAPACITY
                || page.windows(2).any(|pair| pair[0] >= pair[1])
            {
                return Err(invalid_patricia_page(
                    "domain record ordered key page is malformed",
                ));
            }
            for reference in page.iter() {
                if previous.is_some_and(|prior| prior >= reference)
                    || !paged_keys.insert(reference.clone())
                    || self
                        .records
                        .get(reference)
                        .is_none_or(|record| record.reference != *reference)
                {
                    return Err(invalid_patricia_page(
                        "domain record ordered key pages disagree with the HAMT",
                    ));
                }
                previous = Some(reference);
            }
        }
        let hamt_keys = self.records.keys().cloned().collect::<BTreeSet<_>>();
        if paged_keys != hamt_keys {
            return Err(invalid_patricia_page(
                "domain record ordered key pages omit or invent HAMT entries",
            ));
        }

        let mut expected_reverse = BTreeMap::<DomainRecordRef, BTreeSet<DomainRecordRef>>::new();
        let mut expected_successors = BTreeMap::<DomainRecordRef, DomainRecordRef>::new();
        let mut expected_predecessors =
            BTreeMap::<DomainRecordRef, BTreeSet<DomainRecordRef>>::new();
        for record in self.records.values().map(Arc::as_ref) {
            for target in record.references.iter().filter_map(|reference| {
                if let DomainReferenceTarget::Domain(target) = &reference.target {
                    Some(target)
                } else {
                    None
                }
            }) {
                expected_reverse
                    .entry(target.clone())
                    .or_default()
                    .insert(record.reference.clone());
            }
            if let DomainRecordLifecycle::Retired {
                successor: Some(successor),
                ..
            } = &record.lifecycle
            {
                expected_successors.insert(record.reference.clone(), successor.clone());
                expected_predecessors
                    .entry(successor.clone())
                    .or_default()
                    .insert(record.reference.clone());
            }
        }
        let actual_reverse = self
            .reverse_lookup
            .iter()
            .map(|(target, sources)| {
                (
                    target.clone(),
                    sources.iter().cloned().collect::<BTreeSet<_>>(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let actual_successors = self
            .successor_lookup
            .iter()
            .map(|(source, successor)| (source.clone(), successor.clone()))
            .collect::<BTreeMap<_, _>>();
        let actual_predecessors = self
            .predecessor_lookup
            .iter()
            .map(|(target, sources)| {
                (
                    target.clone(),
                    sources.iter().cloned().collect::<BTreeSet<_>>(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        if actual_reverse != expected_reverse
            || actual_successors != expected_successors
            || actual_predecessors != expected_predecessors
        {
            return Err(invalid_patricia_page(
                "domain record COW lookup indexes disagree with authoritative records",
            ));
        }
        let actual_roots = DomainRecordCommitmentRoots {
            primary: self.primary.root_hash(),
            reverse_references: self.reverse_references.root_hash(),
            successor_of: self.successor_of.root_hash(),
            predecessors_of: self.predecessors_of.root_hash(),
        };
        if actual_roots != self.roots {
            return Err(invalid_patricia_page(
                "domain record Patricia roots disagree with cached commitments",
            ));
        }
        Ok(())
    }

    fn empty() -> Self {
        let primary = PersistentPatricia::new("canwu.format8.domain-record.primary.v1");
        let reverse_references = PersistentPatricia::new("canwu.format8.domain-record.reverse.v1");
        let successor_of = PersistentPatricia::new("canwu.format8.domain-record.successor.v1");
        let predecessors_of =
            PersistentPatricia::new("canwu.format8.domain-record.predecessors.v1");
        let roots = DomainRecordCommitmentRoots {
            primary: primary.root_hash(),
            reverse_references: reverse_references.root_hash(),
            successor_of: successor_of.root_hash(),
            predecessors_of: predecessors_of.root_hash(),
        };
        Self {
            records: PersistentHashMap::new(),
            record_key_pages: Vector::new(),
            reverse_lookup: PersistentHashMap::new(),
            successor_lookup: PersistentHashMap::new(),
            predecessor_lookup: PersistentHashMap::new(),
            primary,
            reverse_references,
            successor_of,
            predecessors_of,
            roots,
            primary_leaf_count: 0,
        }
    }

    fn insert_record(
        &mut self,
        reference: DomainRecordRef,
        record: DomainRecord,
    ) -> Result<(), CanwuError> {
        let previous = self.records.get(&reference).cloned();
        if let Some(previous) = previous.as_deref() {
            self.remove_indexes(previous)?;
            self.remove_lookup_indexes(previous);
        }
        self.primary.insert(&reference, &record)?;
        self.add_indexes(&record)?;
        self.add_lookup_indexes(&record);
        if previous.is_none() {
            self.insert_record_key(reference.clone());
        }
        self.records.insert(reference, Arc::new(record));
        self.refresh_roots();
        Ok(())
    }

    fn insert_record_key(&mut self, reference: DomainRecordRef) {
        if self.record_key_pages.is_empty() {
            self.record_key_pages.push_back(Arc::new(vec![reference]));
            return;
        }
        let page_index = self.record_key_page_for(&reference);
        let mut page = self.record_key_pages[page_index].as_ref().clone();
        let position = page
            .binary_search(&reference)
            .expect_err("new domain record keys are unique");
        page.insert(position, reference);
        if page.len() <= DOMAIN_RECORD_KEY_PAGE_CAPACITY {
            self.record_key_pages.set(page_index, Arc::new(page));
            return;
        }
        let right = page.split_off(page.len() / 2);
        self.record_key_pages.set(page_index, Arc::new(page));
        self.record_key_pages
            .insert(page_index.saturating_add(1), Arc::new(right));
    }

    fn record_key_page_for(&self, reference: &DomainRecordRef) -> usize {
        let mut lower = 0usize;
        let mut upper = self.record_key_pages.len();
        while lower < upper {
            let middle = lower + (upper - lower) / 2;
            let last = self.record_key_pages[middle]
                .last()
                .expect("record key pages are never empty");
            if last < reference {
                lower = middle.saturating_add(1);
            } else {
                upper = middle;
            }
        }
        lower.min(self.record_key_pages.len().saturating_sub(1))
    }

    fn record_key_start(&self, reference: &DomainRecordRef, excluded: bool) -> (usize, usize) {
        if self.record_key_pages.is_empty() {
            return (0, 0);
        }
        let page_index = self.record_key_page_for(reference);
        let page = &self.record_key_pages[page_index];
        let key_index = match page.binary_search(reference) {
            Ok(index) if excluded => index.saturating_add(1),
            Ok(index) | Err(index) => index,
        };
        if key_index == page.len() {
            (page_index.saturating_add(1), 0)
        } else {
            (page_index, key_index)
        }
    }

    fn add_lookup_indexes(&mut self, record: &DomainRecord) {
        for target in record.references.iter().filter_map(|reference| {
            if let DomainReferenceTarget::Domain(target) = &reference.target {
                Some(target)
            } else {
                None
            }
        }) {
            self.reverse_lookup
                .entry(target.clone())
                .or_default()
                .insert(record.reference.clone());
        }
        if let DomainRecordLifecycle::Retired {
            successor: Some(successor),
            ..
        } = &record.lifecycle
        {
            self.successor_lookup
                .insert(record.reference.clone(), successor.clone());
            self.predecessor_lookup
                .entry(successor.clone())
                .or_default()
                .insert(record.reference.clone());
        }
    }

    fn remove_lookup_indexes(&mut self, record: &DomainRecord) {
        for target in record.references.iter().filter_map(|reference| {
            if let DomainReferenceTarget::Domain(target) = &reference.target {
                Some(target)
            } else {
                None
            }
        }) {
            if let Some(mut sources) = self.reverse_lookup.get(target).cloned() {
                sources.remove(&record.reference);
                if sources.is_empty() {
                    self.reverse_lookup.remove(target);
                } else {
                    self.reverse_lookup.insert(target.clone(), sources);
                }
            }
        }
        if let Some(successor) = self.successor_lookup.remove(&record.reference)
            && let Some(mut predecessors) = self.predecessor_lookup.get(&successor).cloned()
        {
            predecessors.remove(&record.reference);
            if predecessors.is_empty() {
                self.predecessor_lookup.remove(&successor);
            } else {
                self.predecessor_lookup.insert(successor, predecessors);
            }
        }
    }

    fn add_indexes(&mut self, record: &DomainRecord) -> Result<(), CanwuError> {
        let mut reverse = BTreeSet::new();
        for reference in &record.references {
            if let DomainReferenceTarget::Domain(target) = &reference.target {
                reverse.insert(target.clone());
            }
        }
        for target in reverse {
            self.reverse_references
                .insert(&(target, record.reference.clone()), &())?;
        }
        if let DomainRecordLifecycle::Retired {
            successor: Some(target),
            ..
        } = &record.lifecycle
        {
            self.successor_of.insert(&record.reference, target)?;
            self.predecessors_of
                .insert(&(target.clone(), record.reference.clone()), &())?;
        }
        Ok(())
    }

    fn remove_indexes(&mut self, record: &DomainRecord) -> Result<(), CanwuError> {
        let mut reverse = BTreeSet::new();
        for reference in &record.references {
            if let DomainReferenceTarget::Domain(target) = &reference.target {
                reverse.insert(target.clone());
            }
        }
        for target in reverse {
            self.reverse_references
                .remove(&(target, record.reference.clone()))?;
        }
        if let DomainRecordLifecycle::Retired {
            successor: Some(target),
            ..
        } = &record.lifecycle
        {
            self.successor_of.remove(&record.reference)?;
            self.predecessors_of
                .remove(&(target.clone(), record.reference.clone()))?;
        }
        Ok(())
    }

    fn refresh_roots(&mut self) {
        self.primary_leaf_count = self.primary.entry_count();
        self.roots = DomainRecordCommitmentRoots {
            primary: self.primary.root_hash(),
            reverse_references: self.reverse_references.root_hash(),
            successor_of: self.successor_of.root_hash(),
            predecessors_of: self.predecessors_of.root_hash(),
        };
    }
}

#[derive(Clone, Debug)]
struct PersistentPatricia {
    domain: &'static str,
    root: Option<Arc<PatriciaNode>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredPatriciaEntry {
    key_hash: String,
    key_bytes: Vec<u8>,
    value_hash: String,
    value_bytes: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "node", rename_all = "snake_case")]
enum StoredPatriciaPage {
    Leaf {
        domain: String,
        key_hash: String,
        entries: Vec<StoredPatriciaEntry>,
        entry_count: u64,
        structural_bytes: u64,
    },
    Branch {
        domain: String,
        bit: u16,
        left_page: String,
        right_page: String,
        entry_count: u64,
        structural_bytes: u64,
    },
}

impl PersistentPatricia {
    const fn new(domain: &'static str) -> Self {
        Self { domain, root: None }
    }

    fn insert<K: Serialize, V: Serialize>(&mut self, key: &K, value: &V) -> Result<(), CanwuError> {
        let entry = PatriciaEntry::new(self.domain, key, value)?;
        self.root = Some(insert_patricia(self.domain, self.root.as_ref(), entry));
        Ok(())
    }

    fn remove<K: Serialize>(&mut self, key: &K) -> Result<(), CanwuError> {
        let key_bytes = serde_json::to_vec(key).map_err(|error| {
            CanwuError::new(
                ErrorCode::InvalidSnapshot,
                format!("cannot encode Patricia key: {error}"),
            )
        })?;
        let key_hash = compact_canonical_byte_hash(&format!("{}.key", self.domain), &key_bytes);
        self.root = remove_patricia(self.domain, self.root.as_ref(), key_hash, &key_bytes);
        Ok(())
    }

    fn root_hash(&self) -> String {
        self.root.as_ref().map_or_else(
            || canonical_byte_hash(&format!("{}.empty", self.domain), &[]),
            |root| root.hash().to_hex(),
        )
    }

    fn entry_count(&self) -> usize {
        self.root.as_ref().map_or(0, |root| root.entry_count())
    }

    fn metrics(&self) -> PatriciaStoreMetrics {
        let Some(root) = &self.root else {
            return PatriciaStoreMetrics {
                entries: 0,
                logical_nodes: 0,
                leaf_nodes: 0,
                branch_nodes: 0,
                structural_bytes: 0,
                estimated_resident_bytes: 0,
                depth_p50: 0,
                depth_p95: 0,
                depth_p99: 0,
                max_depth: 0,
                root_hash: self.root_hash(),
            };
        };
        let mut stack = vec![(Arc::clone(root), 0_u16)];
        let mut leaf_nodes = 0_u64;
        let mut branch_nodes = 0_u64;
        let mut depths = Vec::with_capacity(root.entry_count());
        let mut resident_bytes = 0_u64;
        while let Some((node, depth)) = stack.pop() {
            resident_bytes = resident_bytes.saturating_add(match node.as_ref() {
                PatriciaNode::Leaf { entries, .. } => {
                    leaf_nodes += 1;
                    depths.extend(std::iter::repeat_n(depth, entries.len()));
                    size_of::<PatriciaNode>() as u64
                        + size_of::<Vec<PatriciaEntry>>() as u64
                        + entries
                            .capacity()
                            .saturating_mul(size_of::<PatriciaEntry>())
                            as u64
                        + entries
                            .iter()
                            .map(|entry| entry.key_bytes.capacity() as u64)
                            .sum::<u64>()
                }
                PatriciaNode::Branch { left, right, .. } => {
                    branch_nodes += 1;
                    let next_depth = depth.saturating_add(1);
                    stack.push((Arc::clone(left), next_depth));
                    stack.push((Arc::clone(right), next_depth));
                    size_of::<PatriciaNode>() as u64
                }
            });
        }
        depths.sort_unstable();
        let percentile = |numerator: usize, denominator: usize| {
            if depths.is_empty() {
                return 0;
            }
            let index = depths
                .len()
                .saturating_mul(numerator)
                .saturating_add(denominator - 1)
                / denominator;
            depths[index.saturating_sub(1).min(depths.len() - 1)]
        };
        PatriciaStoreMetrics {
            entries: root.entry_count() as u64,
            logical_nodes: leaf_nodes.saturating_add(branch_nodes),
            leaf_nodes,
            branch_nodes,
            structural_bytes: root.structural_bytes() as u64,
            estimated_resident_bytes: resident_bytes,
            depth_p50: percentile(50, 100),
            depth_p95: percentile(95, 100),
            depth_p99: percentile(99, 100),
            max_depth: depths.last().copied().unwrap_or(0),
            root_hash: root.hash().to_hex(),
        }
    }
}

/// Builds the actual Format-8 domain-record store, including the HAMT,
/// ordered key pages, primary Patricia tree, reverse-reference tree, and
/// successor/predecessor trees. The returned metrics describe every production
/// Patricia index plus the materialized HAMT/key-page cardinalities.
pub fn format8_patricia_scale_probe(
    key_count: usize,
) -> Result<DomainStoreScaleMetrics, CanwuError> {
    let mut store = PersistentDomainRecordStore::empty();
    let key_count_u64 = u64::try_from(key_count)
        .map_err(|_| invalid_patricia_page("scale probe key count exceeds u64"))?;
    for ordinal in 0..key_count {
        let ordinal = u64::try_from(ordinal)
            .map_err(|_| invalid_patricia_page("scale probe key exceeds u64"))?;
        let reference =
            DomainRecordRef::new("canwu.format8.scale", "record", format!("{ordinal:016x}"));
        let predecessor = (ordinal > 0).then(|| {
            DomainRecordRef::new(
                "canwu.format8.scale",
                "record",
                format!("{:016x}", ordinal - 1),
            )
        });
        let successor = (ordinal % 1_024 == 0 && ordinal + 1 < key_count_u64).then(|| {
            DomainRecordRef::new(
                "canwu.format8.scale",
                "record",
                format!("{:016x}", ordinal + 1),
            )
        });
        let record = DomainRecord {
            reference: reference.clone(),
            owner: "canwu-format8-scale".to_owned(),
            class: DomainRecordClass::Record,
            version: 1,
            lifecycle: successor.map_or(DomainRecordLifecycle::Active, |successor| {
                DomainRecordLifecycle::Retired {
                    at: SimTime::EPOCH,
                    successor: Some(successor),
                }
            }),
            payload: serde_json::json!({ "ordinal": ordinal }),
            references: predecessor
                .map(|target| {
                    vec![DomainReference {
                        role: "previous".to_owned(),
                        target: DomainReferenceTarget::Domain(target),
                    }]
                })
                .unwrap_or_default(),
        };
        store.insert_record(reference, record)?;
    }
    store.validate_internal_indexes()?;
    let primary = store.primary.metrics();
    let reverse_references = store.reverse_references.metrics();
    let successor_of = store.successor_of.metrics();
    let predecessors_of = store.predecessors_of.metrics();
    let patricia = [
        &primary,
        &reverse_references,
        &successor_of,
        &predecessors_of,
    ];
    Ok(DomainStoreScaleMetrics {
        records: store.records.len() as u64,
        key_pages: store.record_key_pages.len() as u64,
        record_hamt_entries: store.records.len() as u64,
        reverse_lookup_keys: store.reverse_lookup.len() as u64,
        reverse_reference_edges: store
            .reverse_lookup
            .iter()
            .map(|(_, sources)| sources.len() as u64)
            .sum(),
        successor_lookup_entries: store.successor_lookup.len() as u64,
        predecessor_lookup_keys: store.predecessor_lookup.len() as u64,
        predecessor_edges: store
            .predecessor_lookup
            .iter()
            .map(|(_, predecessors)| predecessors.len() as u64)
            .sum(),
        total_patricia_nodes: patricia.iter().map(|metrics| metrics.logical_nodes).sum(),
        total_patricia_structural_bytes: patricia
            .iter()
            .map(|metrics| metrics.structural_bytes)
            .sum(),
        total_patricia_estimated_resident_bytes: patricia
            .iter()
            .map(|metrics| metrics.estimated_resident_bytes)
            .sum(),
        primary,
        reverse_references,
        successor_of,
        predecessors_of,
    })
}

#[derive(Clone, Debug)]
struct PatriciaEntry {
    key_bytes: Vec<u8>,
    value_hash: CompactHash,
    value_bytes: Vec<u8>,
}

impl PatriciaEntry {
    fn new<K: Serialize, V: Serialize>(
        domain: &str,
        key: &K,
        value: &V,
    ) -> Result<Self, CanwuError> {
        let mut key_bytes = serde_json::to_vec(key).map_err(|error| {
            CanwuError::new(
                ErrorCode::InvalidSnapshot,
                format!("cannot encode Patricia key: {error}"),
            )
        })?;
        key_bytes.shrink_to_fit();
        let mut value_bytes = serde_json::to_vec(value).map_err(|error| {
            CanwuError::new(
                ErrorCode::InvalidSnapshot,
                format!("cannot encode Patricia value: {error}"),
            )
        })?;
        value_bytes.shrink_to_fit();
        Ok(Self {
            key_bytes,
            value_hash: compact_canonical_byte_hash(&format!("{domain}.value"), &value_bytes),
            value_bytes,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct CompactHash([u8; 32]);

impl CompactHash {
    fn parse(value: &str) -> Result<Self, CanwuError> {
        if value.len() != 64 {
            return Err(invalid_patricia_page(
                "Patricia hash must contain exactly 64 hexadecimal characters",
            ));
        }
        let mut bytes = [0_u8; 32];
        let (pairs, remainder) = value.as_bytes().as_chunks::<2>();
        if !remainder.is_empty() {
            return Err(invalid_patricia_page(
                "Patricia hash must contain complete hexadecimal byte pairs",
            ));
        }
        for (index, pair) in pairs.iter().enumerate() {
            let high = decode_hex_nibble(pair[0])?;
            let low = decode_hex_nibble(pair[1])?;
            bytes[index] = (high << 4) | low;
        }
        Ok(Self(bytes))
    }

    fn to_hex(self) -> String {
        let mut encoded = String::with_capacity(64);
        for byte in self.0 {
            write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
        }
        encoded
    }
}

fn decode_hex_nibble(byte: u8) -> Result<u8, CanwuError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(invalid_patricia_page(
            "Patricia hash must use lowercase hexadecimal",
        )),
    }
}

fn compact_canonical_byte_hash(domain: &str, bytes: &[u8]) -> CompactHash {
    CompactHash::parse(&canonical_byte_hash(domain, bytes))
        .expect("canonical_byte_hash always returns a lowercase 32-byte digest")
}

fn compact_state_page_id(stored: &StoredPatriciaPage) -> CompactHash {
    let bytes =
        serde_json::to_vec(stored).expect("canonical Patricia page values always serialize");
    CompactHash::parse(&state_page_id(&bytes))
        .expect("state_page_id always returns a lowercase 32-byte digest")
}

#[derive(Clone, Debug)]
enum PatriciaNode {
    Leaf {
        key_hash: CompactHash,
        entries: Arc<Vec<PatriciaEntry>>,
        hash: CompactHash,
        entry_count: usize,
        structural_bytes: usize,
    },
    Branch {
        bit: u16,
        left: Arc<Self>,
        right: Arc<Self>,
        hash: CompactHash,
        entry_count: usize,
        structural_bytes: usize,
    },
}

impl PatriciaNode {
    const fn hash(&self) -> CompactHash {
        match self {
            Self::Leaf { hash, .. } | Self::Branch { hash, .. } => *hash,
        }
    }

    fn sample_key_hash(&self) -> CompactHash {
        match self {
            Self::Leaf { key_hash, .. } => *key_hash,
            Self::Branch { left, .. } => left.sample_key_hash(),
        }
    }

    const fn entry_count(&self) -> usize {
        match self {
            Self::Leaf { entry_count, .. } | Self::Branch { entry_count, .. } => *entry_count,
        }
    }

    const fn structural_bytes(&self) -> usize {
        match self {
            Self::Leaf {
                structural_bytes, ..
            }
            | Self::Branch {
                structural_bytes, ..
            } => *structural_bytes,
        }
    }
}

fn patricia_leaf(
    domain: &str,
    key_hash: CompactHash,
    mut entries: Vec<PatriciaEntry>,
) -> Arc<PatriciaNode> {
    entries.sort_by(|left, right| left.key_bytes.cmp(&right.key_bytes));
    let structural_bytes = 32
        + entries
            .iter()
            .map(|entry| 8 + entry.key_bytes.len() + 32)
            .sum::<usize>();
    let entry_count = entries.len();
    let stored = StoredPatriciaPage::Leaf {
        domain: domain.to_owned(),
        key_hash: key_hash.to_hex(),
        entries: entries
            .iter()
            .map(|entry| StoredPatriciaEntry {
                key_hash: key_hash.to_hex(),
                key_bytes: entry.key_bytes.clone(),
                value_hash: entry.value_hash.to_hex(),
                value_bytes: entry.value_bytes.clone(),
            })
            .collect(),
        entry_count: entry_count as u64,
        structural_bytes: structural_bytes as u64,
    };
    let hash = compact_state_page_id(&stored);
    Arc::new(PatriciaNode::Leaf {
        key_hash,
        entries: Arc::new(entries),
        hash,
        entry_count,
        structural_bytes,
    })
}

fn patricia_branch(
    domain: &str,
    bit: usize,
    left: Arc<PatriciaNode>,
    right: Arc<PatriciaNode>,
) -> Arc<PatriciaNode> {
    let bit = u16::try_from(bit).expect("Patricia bit index is bounded to 256 bits");
    let entry_count = left.entry_count() + right.entry_count();
    let structural_bytes = 2 + 64 + 16 + left.structural_bytes() + right.structural_bytes();
    let stored = StoredPatriciaPage::Branch {
        domain: domain.to_owned(),
        bit,
        left_page: left.hash().to_hex(),
        right_page: right.hash().to_hex(),
        entry_count: entry_count as u64,
        structural_bytes: structural_bytes as u64,
    };
    let hash = compact_state_page_id(&stored);
    Arc::new(PatriciaNode::Branch {
        bit,
        left,
        right,
        hash,
        entry_count,
        structural_bytes,
    })
}

fn insert_patricia(
    domain: &str,
    node: Option<&Arc<PatriciaNode>>,
    entry: PatriciaEntry,
) -> Arc<PatriciaNode> {
    let entry_key_hash = compact_canonical_byte_hash(&format!("{domain}.key"), &entry.key_bytes);
    let Some(root) = node else {
        return patricia_leaf(domain, entry_key_hash, vec![entry]);
    };
    let mut current = Arc::clone(root);
    let mut path = Vec::new();
    let mut entry = Some(entry);
    let mut replacement = loop {
        let difference = first_discriminating_bit(current.sample_key_hash(), entry_key_hash);
        if let PatriciaNode::Branch {
            bit, left, right, ..
        } = current.as_ref()
        {
            let branch_bit = usize::from(*bit);
            if difference.is_none_or(|difference| difference >= branch_bit) {
                if bit_at(entry_key_hash, branch_bit) == 0 {
                    path.push((branch_bit, Arc::clone(right), true));
                    current = Arc::clone(left);
                } else {
                    path.push((branch_bit, Arc::clone(left), false));
                    current = Arc::clone(right);
                }
                continue;
            }
        }
        let Some(difference) = difference else {
            let PatriciaNode::Leaf { entries, .. } = current.as_ref() else {
                unreachable!("a branch with an equal sample hash was descended above");
            };
            let mut next = entries.as_ref().clone();
            let entry = entry
                .take()
                .expect("Patricia insertion consumes its entry exactly once");
            match next.binary_search_by(|candidate| candidate.key_bytes.cmp(&entry.key_bytes)) {
                Ok(index) => next[index] = entry,
                Err(index) => next.insert(index, entry),
            }
            break patricia_leaf(domain, current.sample_key_hash(), next);
        };
        let leaf = patricia_leaf(
            domain,
            entry_key_hash,
            vec![
                entry
                    .take()
                    .expect("Patricia insertion consumes its entry exactly once"),
            ],
        );
        break if bit_at(entry_key_hash, difference) == 0 {
            patricia_branch(domain, difference, leaf, Arc::clone(&current))
        } else {
            patricia_branch(domain, difference, Arc::clone(&current), leaf)
        };
    };
    while let Some((bit, sibling, descended_left)) = path.pop() {
        replacement = if descended_left {
            patricia_branch(domain, bit, replacement, sibling)
        } else {
            patricia_branch(domain, bit, sibling, replacement)
        };
    }
    replacement
}

fn remove_patricia(
    domain: &str,
    node: Option<&Arc<PatriciaNode>>,
    key_hash: CompactHash,
    key_bytes: &[u8],
) -> Option<Arc<PatriciaNode>> {
    let node = node?;
    match node.as_ref() {
        PatriciaNode::Leaf {
            key_hash: leaf_hash,
            entries,
            ..
        } => {
            if *leaf_hash != key_hash {
                return Some(Arc::clone(node));
            }
            let mut next = entries.as_ref().clone();
            let Ok(index) =
                next.binary_search_by(|entry| entry.key_bytes.as_slice().cmp(key_bytes))
            else {
                return Some(Arc::clone(node));
            };
            next.remove(index);
            (!next.is_empty()).then(|| patricia_leaf(domain, *leaf_hash, next))
        }
        PatriciaNode::Branch {
            bit, left, right, ..
        } => {
            let bit = usize::from(*bit);
            if bit_at(key_hash, bit) == 0 {
                let next_left = remove_patricia(domain, Some(left), key_hash, key_bytes);
                next_left.map_or_else(
                    || Some(Arc::clone(right)),
                    |next_left| Some(patricia_branch(domain, bit, next_left, Arc::clone(right))),
                )
            } else {
                let next_right = remove_patricia(domain, Some(right), key_hash, key_bytes);
                next_right.map_or_else(
                    || Some(Arc::clone(left)),
                    |next_right| Some(patricia_branch(domain, bit, Arc::clone(left), next_right)),
                )
            }
        }
    }
}

fn first_discriminating_bit(left: CompactHash, right: CompactHash) -> Option<usize> {
    (0..256).find(|bit| bit_at(left, *bit) != bit_at(right, *bit))
}

fn bit_at(hash: CompactHash, bit: usize) -> u8 {
    let byte = hash.0.get(bit / 8).copied().unwrap_or(0);
    (byte >> (7 - (bit % 8))) & 1
}

fn collect_patricia_pages(
    tree: &PersistentPatricia,
    pages: &mut BTreeMap<String, StatePageBlob>,
) -> Result<Option<String>, CanwuError> {
    tree.root
        .as_ref()
        .map(|root| collect_patricia_node_pages(tree.domain, root, pages))
        .transpose()
}

fn collect_missing_patricia_pages(
    tree: &PersistentPatricia,
    provider: &dyn StatePageProvider,
    pages: &mut BTreeMap<String, StatePageBlob>,
) -> Result<Option<String>, CanwuError> {
    tree.root
        .as_ref()
        .map(|root| collect_missing_patricia_node_pages(tree.domain, root, provider, pages))
        .transpose()
}

fn collect_missing_patricia_node_pages(
    domain: &str,
    node: &Arc<PatriciaNode>,
    provider: &dyn StatePageProvider,
    pages: &mut BTreeMap<String, StatePageBlob>,
) -> Result<String, CanwuError> {
    let page_id = node.hash().to_hex();
    if let Some(existing) = provider.load_state_page(&page_id)? {
        existing.validate()?;
        if existing.page_id != page_id {
            return Err(invalid_patricia_page(
                "state-page provider returned the wrong Patricia page",
            ));
        }
        return Ok(page_id);
    }
    if let PatriciaNode::Branch { left, right, .. } = node.as_ref() {
        collect_missing_patricia_node_pages(domain, left, provider, pages)?;
        collect_missing_patricia_node_pages(domain, right, provider, pages)?;
    }
    let page = patricia_node_page(domain, node)?;
    if page.page_id != page_id {
        return Err(invalid_patricia_page(
            "Patricia node commitment disagrees with its state page",
        ));
    }
    pages.insert(page_id.clone(), page);
    if pages.len() > MAX_STATE_DELTA_PAGES {
        return Err(invalid_patricia_page(
            "Patricia delta exceeds the bounded page count",
        ));
    }
    Ok(page_id)
}

fn collect_patricia_node_pages(
    domain: &str,
    node: &Arc<PatriciaNode>,
    pages: &mut BTreeMap<String, StatePageBlob>,
) -> Result<String, CanwuError> {
    if let PatriciaNode::Branch { left, right, .. } = node.as_ref() {
        collect_patricia_node_pages(domain, left, pages)?;
        collect_patricia_node_pages(domain, right, pages)?;
    }
    let page = patricia_node_page(domain, node)?;
    let page_id = page.page_id.clone();
    pages.insert(page_id.clone(), page);
    if pages.len() > MAX_STATE_DELTA_PAGES {
        return Err(invalid_patricia_page(
            "Patricia page graph exceeds the bounded page count",
        ));
    }
    Ok(page_id)
}

fn patricia_node_page(domain: &str, node: &Arc<PatriciaNode>) -> Result<StatePageBlob, CanwuError> {
    let stored = match node.as_ref() {
        PatriciaNode::Leaf {
            key_hash,
            entries,
            entry_count,
            structural_bytes,
            ..
        } => StoredPatriciaPage::Leaf {
            domain: domain.to_owned(),
            key_hash: key_hash.to_hex(),
            entries: entries
                .iter()
                .map(|entry| StoredPatriciaEntry {
                    key_hash: key_hash.to_hex(),
                    key_bytes: entry.key_bytes.clone(),
                    value_hash: entry.value_hash.to_hex(),
                    value_bytes: entry.value_bytes.clone(),
                })
                .collect(),
            entry_count: u64::try_from(*entry_count)
                .map_err(|_| invalid_patricia_page("Patricia entry count exceeds u64"))?,
            structural_bytes: u64::try_from(*structural_bytes)
                .map_err(|_| invalid_patricia_page("Patricia byte count exceeds u64"))?,
        },
        PatriciaNode::Branch {
            bit,
            left,
            right,
            entry_count,
            structural_bytes,
            ..
        } => StoredPatriciaPage::Branch {
            domain: domain.to_owned(),
            bit: *bit,
            left_page: left.hash().to_hex(),
            right_page: right.hash().to_hex(),
            entry_count: u64::try_from(*entry_count)
                .map_err(|_| invalid_patricia_page("Patricia entry count exceeds u64"))?,
            structural_bytes: u64::try_from(*structural_bytes)
                .map_err(|_| invalid_patricia_page("Patricia byte count exceeds u64"))?,
        },
    };
    let bytes = serde_json::to_vec(&stored).map_err(|error| {
        invalid_patricia_page(format!("cannot encode canonical Patricia page: {error}"))
    })?;
    StatePageBlob::new(bytes)
}

fn load_patricia_root(
    domain: &'static str,
    page_id: Option<&str>,
    provider: &dyn StatePageProvider,
    cache: &mut BTreeMap<String, Arc<PatriciaNode>>,
) -> Result<PersistentPatricia, CanwuError> {
    let Some(page_id) = page_id else {
        return Ok(PersistentPatricia::new(domain));
    };
    let mut active = BTreeSet::new();
    let root = load_patricia_node(domain, page_id, provider, cache, &mut active, None, 0)?;
    Ok(PersistentPatricia {
        domain,
        root: Some(root),
    })
}

fn load_patricia_node(
    domain: &'static str,
    page_id: &str,
    provider: &dyn StatePageProvider,
    cache: &mut BTreeMap<String, Arc<PatriciaNode>>,
    active: &mut BTreeSet<String>,
    parent_bit: Option<u16>,
    depth: usize,
) -> Result<Arc<PatriciaNode>, CanwuError> {
    if depth > 256 || cache.len() >= MAX_STATE_DELTA_PAGES {
        return Err(invalid_patricia_page(
            "Patricia page graph exceeds its depth or page budget",
        ));
    }
    if let Some(node) = cache.get(page_id) {
        validate_child_bit(node, parent_bit)?;
        return Ok(Arc::clone(node));
    }
    if !active.insert(page_id.to_owned()) {
        return Err(invalid_patricia_page(
            "Patricia page graph contains a cycle",
        ));
    }
    let page = provider.load_state_page(page_id)?.ok_or_else(|| {
        CanwuError::new(
            ErrorCode::StatePageUnavailable,
            format!("state page {page_id} is unavailable"),
        )
    })?;
    page.validate()?;
    if page.page_id != page_id {
        return Err(invalid_patricia_page(
            "state-page provider returned the wrong content address",
        ));
    }
    let stored: StoredPatriciaPage = serde_json::from_slice(&page.bytes)
        .map_err(|error| invalid_patricia_page(format!("invalid Patricia state page: {error}")))?;
    let node = match stored {
        StoredPatriciaPage::Leaf {
            domain: stored_domain,
            key_hash,
            entries,
            entry_count,
            structural_bytes,
        } => {
            if stored_domain != domain || entries.is_empty() {
                return Err(invalid_patricia_page(
                    "Patricia leaf domain or collision membership is invalid",
                ));
            }
            let mut decoded = Vec::with_capacity(entries.len());
            for entry in entries {
                if entry.key_hash != key_hash
                    || entry.key_hash
                        != canonical_byte_hash(&format!("{domain}.key"), &entry.key_bytes)
                    || entry.value_hash
                        != canonical_byte_hash(&format!("{domain}.value"), &entry.value_bytes)
                {
                    return Err(invalid_patricia_page(
                        "Patricia leaf entry hash is inconsistent",
                    ));
                }
                decoded.push(PatriciaEntry {
                    key_bytes: entry.key_bytes,
                    value_hash: CompactHash::parse(&entry.value_hash)?,
                    value_bytes: entry.value_bytes,
                });
            }
            if decoded
                .windows(2)
                .any(|pair| pair[0].key_bytes >= pair[1].key_bytes)
            {
                return Err(invalid_patricia_page(
                    "Patricia collision entries are not strictly ordered",
                ));
            }
            let rebuilt = patricia_leaf(domain, CompactHash::parse(&key_hash)?, decoded);
            if rebuilt.hash().to_hex() != page_id
                || rebuilt.entry_count() as u64 != entry_count
                || rebuilt.structural_bytes() as u64 != structural_bytes
            {
                return Err(invalid_patricia_page(
                    "Patricia leaf metadata or commitment is inconsistent",
                ));
            }
            rebuilt
        }
        StoredPatriciaPage::Branch {
            domain: stored_domain,
            bit,
            left_page,
            right_page,
            entry_count,
            structural_bytes,
        } => {
            if stored_domain != domain || left_page == right_page || usize::from(bit) >= 256 {
                return Err(invalid_patricia_page(
                    "Patricia branch domain, bit, or child identity is invalid",
                ));
            }
            if parent_bit.is_some_and(|parent| bit <= parent) {
                return Err(invalid_patricia_page(
                    "Patricia discriminating bits must increase down a path",
                ));
            }
            let left = load_patricia_node(
                domain,
                &left_page,
                provider,
                cache,
                active,
                Some(bit),
                depth + 1,
            )?;
            let right = load_patricia_node(
                domain,
                &right_page,
                provider,
                cache,
                active,
                Some(bit),
                depth + 1,
            )?;
            if bit_at(left.sample_key_hash(), usize::from(bit)) != 0
                || bit_at(right.sample_key_hash(), usize::from(bit)) != 1
            {
                return Err(invalid_patricia_page(
                    "Patricia children occupy the wrong discriminating slots",
                ));
            }
            let rebuilt = patricia_branch(domain, usize::from(bit), left, right);
            if rebuilt.hash().to_hex() != page_id
                || rebuilt.entry_count() as u64 != entry_count
                || rebuilt.structural_bytes() as u64 != structural_bytes
            {
                return Err(invalid_patricia_page(
                    "Patricia branch metadata or commitment is inconsistent",
                ));
            }
            rebuilt
        }
    };
    validate_child_bit(&node, parent_bit)?;
    active.remove(page_id);
    cache.insert(page_id.to_owned(), Arc::clone(&node));
    Ok(node)
}

fn validate_child_bit(node: &PatriciaNode, parent_bit: Option<u16>) -> Result<(), CanwuError> {
    if let (Some(parent), PatriciaNode::Branch { bit: child_bit, .. }) = (parent_bit, node)
        && *child_bit <= parent
    {
        return Err(invalid_patricia_page(
            "Patricia discriminating bits must increase down a path",
        ));
    }
    Ok(())
}

fn collect_primary_records(
    node: &PatriciaNode,
    records: &mut BTreeMap<DomainRecordRef, DomainRecord>,
) -> Result<(), CanwuError> {
    match node {
        PatriciaNode::Leaf { entries, .. } => {
            for entry in entries.iter() {
                let reference: DomainRecordRef =
                    serde_json::from_slice(&entry.key_bytes).map_err(|error| {
                        invalid_patricia_page(format!(
                            "cannot decode domain-record key page: {error}"
                        ))
                    })?;
                let record: DomainRecord =
                    serde_json::from_slice(&entry.value_bytes).map_err(|error| {
                        invalid_patricia_page(format!(
                            "cannot decode domain-record value page: {error}"
                        ))
                    })?;
                if record.reference != reference || records.insert(reference, record).is_some() {
                    return Err(invalid_patricia_page(
                        "domain-record primary page contains a duplicate or mismatched key",
                    ));
                }
            }
        }
        PatriciaNode::Branch { left, right, .. } => {
            collect_primary_records(left, records)?;
            collect_primary_records(right, records)?;
        }
    }
    Ok(())
}

fn invalid_patricia_page(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidArchive, message)
}

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

pub(crate) trait DomainRecordRead {
    fn get(&self, reference: &DomainRecordRef) -> Option<&DomainRecord>;
    fn iter(&self) -> Box<dyn Iterator<Item = (&DomainRecordRef, &DomainRecord)> + '_>;
    fn range_from(
        &self,
        lower: DomainRecordRef,
        excluded: bool,
    ) -> Box<dyn Iterator<Item = (&DomainRecordRef, &DomainRecord)> + '_>;

    fn contains_key(&self, reference: &DomainRecordRef) -> bool {
        self.get(reference).is_some()
    }
}

trait DomainRecordWrite: DomainRecordRead {
    fn insert(&mut self, reference: DomainRecordRef, record: DomainRecord);
}

impl DomainRecordRead for BTreeMap<DomainRecordRef, DomainRecord> {
    fn get(&self, reference: &DomainRecordRef) -> Option<&DomainRecord> {
        BTreeMap::get(self, reference)
    }

    fn iter(&self) -> Box<dyn Iterator<Item = (&DomainRecordRef, &DomainRecord)> + '_> {
        Box::new(BTreeMap::iter(self))
    }

    fn range_from(
        &self,
        lower: DomainRecordRef,
        excluded: bool,
    ) -> Box<dyn Iterator<Item = (&DomainRecordRef, &DomainRecord)> + '_> {
        use std::ops::Bound::{Excluded, Included, Unbounded};
        let lower = if excluded {
            Excluded(lower)
        } else {
            Included(lower)
        };
        Box::new(self.range((lower, Unbounded)))
    }
}

impl DomainRecordWrite for BTreeMap<DomainRecordRef, DomainRecord> {
    fn insert(&mut self, reference: DomainRecordRef, record: DomainRecord) {
        let _ = BTreeMap::insert(self, reference, record);
    }
}

impl DomainRecordRead for PersistentDomainRecordStore {
    fn get(&self, reference: &DomainRecordRef) -> Option<&DomainRecord> {
        self.get(reference)
    }

    fn iter(&self) -> Box<dyn Iterator<Item = (&DomainRecordRef, &DomainRecord)> + '_> {
        Box::new(self.iter())
    }

    fn range_from(
        &self,
        lower: DomainRecordRef,
        excluded: bool,
    ) -> Box<dyn Iterator<Item = (&DomainRecordRef, &DomainRecord)> + '_> {
        let (start_page, start_key) = self.record_key_start(&lower, excluded);
        Box::new(
            self.record_key_pages
                .iter()
                .enumerate()
                .skip(start_page)
                .flat_map(move |(page_index, page)| {
                    page.iter()
                        .skip(if page_index == start_page {
                            start_key
                        } else {
                            0
                        })
                        .filter_map(|reference| {
                            self.records
                                .get(reference)
                                .map(|record| (reference, record.as_ref()))
                        })
                }),
        )
    }
}

impl DomainRecordWrite for PersistentDomainRecordStore {
    fn insert(&mut self, reference: DomainRecordRef, record: DomainRecord) {
        self.insert_record(reference, record)
            .expect("validated domain records must have canonical Patricia encodings");
    }
}

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
    records: &impl DomainRecordRead,
    schemas: &DomainRecordSchemas,
    now: SimTime,
    core_exists: &dyn Fn(&EntityRef) -> bool,
) -> Result<(), CanwuError> {
    for (reference, record) in records.iter() {
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
    records: &impl DomainRecordRead,
    schemas: &DomainRecordSchemas,
    owner: &str,
    now: SimTime,
    core_exists: &dyn Fn(&EntityRef) -> bool,
) -> Result<(), CanwuError> {
    for (_, record) in records.iter().filter(|(_, record)| record.owner == owner) {
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
    requests: Vec<DomainMutationRequest<'_>>,
) -> Result<
    (
        BTreeMap<DomainRecordRef, DomainRecord>,
        Vec<DomainRecordChange>,
    ),
    CanwuError,
> {
    let mut next = records.clone();
    let changes =
        apply_mutation_bundle_in_place(records, &mut next, schemas, now, core_exists, requests)?;
    validate_record_store(&next, schemas, now, core_exists)?;
    Ok((next, changes))
}

/// Applies one canonical boundary mutation bundle to a structurally shared
/// record root. Cloning the input store is O(1); each insertion copies only
/// the persistent tree path and leaves the captured root untouched.
pub(crate) fn apply_mutation_bundle_cow(
    records: &PersistentDomainRecordStore,
    schemas: &DomainRecordSchemas,
    now: SimTime,
    core_exists: &dyn Fn(&EntityRef) -> bool,
    requests: Vec<DomainMutationRequest<'_>>,
) -> Result<(PersistentDomainRecordStore, Vec<DomainRecordChange>), CanwuError> {
    let mut next = records.clone();
    let changes =
        apply_mutation_bundle_in_place(records, &mut next, schemas, now, core_exists, requests)?;
    validate_affected_record_store(&next, schemas, now, core_exists, &changes)?;
    Ok((next, changes))
}

/// Applies a boundary mutation bundle over a small, already-validated record
/// overlay without materializing or cloning the untouched persistent store.
///
/// Boundary proposal validation can call this repeatedly while systems in the
/// same phase accumulate visible changes. Replaying only those changed records
/// into a structurally shared root keeps the validation cost proportional to
/// the overlay and the affected-reference closure, not to unrelated payloads.
pub(crate) fn apply_mutation_bundle_cow_with_overlay(
    records: &PersistentDomainRecordStore,
    overlay: &BTreeMap<DomainRecordRef, DomainRecord>,
    schemas: &DomainRecordSchemas,
    now: SimTime,
    core_exists: &dyn Fn(&EntityRef) -> bool,
    requests: Vec<DomainMutationRequest<'_>>,
) -> Result<(PersistentDomainRecordStore, Vec<DomainRecordChange>), CanwuError> {
    let mut base = records.clone();
    for (reference, record) in overlay {
        base.insert_record(reference.clone(), record.clone())?;
    }
    apply_mutation_bundle_cow(&base, schemas, now, core_exists, requests)
}

fn apply_mutation_bundle_in_place<R, N>(
    records: &R,
    next: &mut N,
    schemas: &DomainRecordSchemas,
    now: SimTime,
    _core_exists: &dyn Fn(&EntityRef) -> bool,
    mut requests: Vec<DomainMutationRequest<'_>>,
) -> Result<Vec<DomainRecordChange>, CanwuError>
where
    R: DomainRecordRead,
    N: DomainRecordWrite,
{
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
                let previous = require_mutable_record(next, target, *expected_version)?.clone();
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
                let previous = require_mutable_record(next, record, *expected_version)?.clone();
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
    Ok(changes)
}

fn validate_affected_record_store(
    records: &PersistentDomainRecordStore,
    schemas: &DomainRecordSchemas,
    now: SimTime,
    core_exists: &dyn Fn(&EntityRef) -> bool,
    changes: &[DomainRecordChange],
) -> Result<(), CanwuError> {
    let mut affected = changes
        .iter()
        .map(|change| change.current.reference.clone())
        .collect::<BTreeSet<_>>();
    let mut pending = affected.iter().cloned().collect::<Vec<_>>();
    while let Some(reference) = pending.pop() {
        for related in records
            .reverse_lookup
            .get(&reference)
            .into_iter()
            .flatten()
            .chain(
                records
                    .predecessor_lookup
                    .get(&reference)
                    .into_iter()
                    .flatten(),
            )
        {
            if affected.insert(related.clone()) {
                if affected.len() > MAX_AFFECTED_RECORD_VALIDATION_CLOSURE {
                    return invalid_record(
                        "domain record mutation exceeded the affected-reference closure budget",
                    );
                }
                pending.push(related.clone());
            }
        }
    }
    for reference in &affected {
        let record = records.get(reference).ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidDomainRecord,
                "affected domain record disappeared during validation",
            )
        })?;
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
    for change in changes {
        validate_successor_chain_from(records, &change.current.reference)?;
    }
    Ok(())
}

fn validate_successor_chain_from(
    records: &PersistentDomainRecordStore,
    start: &DomainRecordRef,
) -> Result<(), CanwuError> {
    let mut visited = BTreeSet::new();
    let mut current = start;
    loop {
        if !visited.insert(current.clone()) {
            return invalid_record("domain record successor chains cannot contain cycles");
        }
        if visited.len() > MAX_AFFECTED_RECORD_VALIDATION_CLOSURE {
            return invalid_record(
                "domain record successor validation exceeded the affected-closure budget",
            );
        }
        let Some(successor) = records.successor_lookup.get(current) else {
            return Ok(());
        };
        current = successor;
    }
}

pub(crate) fn domain_entity_exists(
    records: &impl DomainRecordRead,
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
    records: &impl DomainRecordRead,
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
    records: &impl DomainRecordRead,
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
    records: &impl DomainRecordRead,
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
    records: &impl DomainRecordRead,
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

fn validate_successor_graph(records: &impl DomainRecordRead) -> Result<(), CanwuError> {
    let mut complete = BTreeSet::new();
    for (start, _) in records.iter() {
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
    records: &'a impl DomainRecordRead,
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

    #[test]
    fn persistent_store_updates_leave_captured_root_unchanged() {
        let kind = fixture_kind("persistent");
        let first_ref = DomainRecordRef::new(&kind.namespace, &kind.name, "first");
        let second_ref = DomainRecordRef::new(&kind.namespace, &kind.name, "second");
        let records = BTreeMap::from([
            (
                first_ref.clone(),
                fixture_record(first_ref.clone(), DomainRecordClass::Record),
            ),
            (
                second_ref.clone(),
                fixture_record(second_ref, DomainRecordClass::Record),
            ),
        ]);
        let store = PersistentDomainRecordStore::from_records(records).unwrap();
        let captured = store.clone();
        assert!(captured.shares_root_with(&store));

        let schema = DomainRecordSchema::new(kind.clone(), DomainRecordClass::Record);
        let schemas = BTreeMap::from([(kind, ("fixture".to_owned(), schema))]);
        let update = DomainRecordMutation::Update {
            record: DomainRecordDraft::new(first_ref.clone(), json!("changed")),
            expected_version: 1,
        };
        let (updated, changes) = apply_mutation_bundle_cow(
            &store,
            &schemas,
            SimTime::EPOCH,
            &|_: &EntityRef| true,
            vec![mutation_request(&update)],
        )
        .unwrap();

        assert!(!updated.shares_root_with(&captured));
        assert_eq!(captured.get(&first_ref).unwrap().version, 1);
        assert_eq!(updated.get(&first_ref).unwrap().version, 2);
        assert_ne!(
            captured.commitment_root().unwrap(),
            updated.commitment_root().unwrap()
        );
        assert_eq!(changes.len(), 1);
    }

    #[test]
    fn persistent_overlay_validation_shares_unrelated_record_payloads() {
        let kind = fixture_kind("overlay");
        let first_ref = DomainRecordRef::new(&kind.namespace, &kind.name, "first");
        let second_ref = DomainRecordRef::new(&kind.namespace, &kind.name, "second");
        let cold_ref = DomainRecordRef::new(&kind.namespace, &kind.name, "cold");
        let records = BTreeMap::from([
            (
                first_ref.clone(),
                fixture_record(first_ref.clone(), DomainRecordClass::Record),
            ),
            (
                second_ref.clone(),
                fixture_record(second_ref.clone(), DomainRecordClass::Record),
            ),
            (
                cold_ref.clone(),
                DomainRecord {
                    payload: json!({ "cold": "x".repeat(1_000_000) }),
                    ..fixture_record(cold_ref.clone(), DomainRecordClass::Record)
                },
            ),
        ]);
        let store = PersistentDomainRecordStore::from_records(records).unwrap();
        let mut first_overlay = store.get(&first_ref).unwrap().clone();
        first_overlay.version = 2;
        first_overlay.payload = json!("first-overlay");
        let overlay = BTreeMap::from([(first_ref.clone(), first_overlay)]);
        let schema = DomainRecordSchema::new(kind.clone(), DomainRecordClass::Record);
        let schemas = BTreeMap::from([(kind, ("fixture".to_owned(), schema))]);
        let update = DomainRecordMutation::Update {
            record: DomainRecordDraft::new(second_ref.clone(), json!("second-update")),
            expected_version: 1,
        };

        let (updated, changes) = apply_mutation_bundle_cow_with_overlay(
            &store,
            &overlay,
            &schemas,
            SimTime::EPOCH,
            &|_: &EntityRef| true,
            vec![mutation_request(&update)],
        )
        .unwrap();

        assert_eq!(store.get(&first_ref).unwrap().version, 1);
        assert_eq!(updated.get(&first_ref).unwrap().version, 2);
        assert_eq!(updated.get(&second_ref).unwrap().version, 2);
        assert!(Arc::ptr_eq(
            store.records.get(&cold_ref).unwrap(),
            updated.records.get(&cold_ref).unwrap()
        ));
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].current.reference, second_ref);
    }
}
