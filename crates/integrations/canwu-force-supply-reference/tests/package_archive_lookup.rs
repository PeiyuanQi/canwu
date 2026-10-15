//! The package archive answers a lookup through its cumulative lookup index:
//! the head directory, one index page per level, and one blob on a hit,
//! however many commits the archive holds. A missing or forged index page is
//! an error, never a miss; the restore audit rebuilds the head's index from
//! the directory chain; and the reachability participant keeps every page
//! that the head or a queued commit reads.

use canwu_api::{ArchiveReachabilityManifest, CanwuError, PluginArchiveObjectProvider};
use canwu_force_supply_reference::{
    FORCE_ARCHIVE_BLOB_NAMESPACE, FORCE_ARCHIVE_DOMAIN, FORCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE,
    FORCE_ARCHIVE_LOOKUP_PAGE_NAMESPACE, MAX_PACKAGE_ARCHIVE_DIRECTORY_BYTES,
    PackageArchiveHeadStateV1, PackageArchiveIndexDirectoryV1, PackageArchiveRecordV1,
    PackageArchiveRetentionHandleV1, PackageArchiveStore, PreparedPackageArchiveBatchV1,
    extend_archive_reachability, load_package_archive_record, prepare_package_archive,
    sealed_archive_head, validate_package_archive_store,
};
use serde::{Deserialize, Serialize};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct Payload {
    key: u64,
    generation: u64,
}

#[derive(Default)]
struct Store {
    objects: RefCell<BTreeMap<(String, String), Vec<u8>>>,
    handles: RefCell<BTreeMap<String, PackageArchiveRetentionHandleV1>>,
    reads: RefCell<BTreeMap<String, u64>>,
    counting: Cell<bool>,
}

impl Store {
    fn bytes(&self, namespace: &str, id: &str) -> Option<Vec<u8>> {
        self.objects
            .borrow()
            .get(&(namespace.to_owned(), id.to_owned()))
            .cloned()
    }

    fn put(&self, namespace: &str, id: &str, bytes: Vec<u8>) {
        self.objects
            .borrow_mut()
            .insert((namespace.to_owned(), id.to_owned()), bytes);
    }

    fn remove(&self, namespace: &str, id: &str) -> Vec<u8> {
        self.objects
            .borrow_mut()
            .remove(&(namespace.to_owned(), id.to_owned()))
            .expect("the object is stored")
    }

    fn ids(&self, namespace: &str) -> BTreeSet<String> {
        self.objects
            .borrow()
            .keys()
            .filter(|(stored, _)| stored == namespace)
            .map(|(_, id)| id.clone())
            .collect()
    }

    /// Counts the reads `lookup` makes, by namespace.
    fn count_reads<T>(&self, lookup: impl FnOnce() -> T) -> (T, BTreeMap<String, u64>) {
        self.reads.borrow_mut().clear();
        self.counting.set(true);
        let result = lookup();
        self.counting.set(false);
        (result, std::mem::take(&mut *self.reads.borrow_mut()))
    }

    /// A store that holds only the objects `manifest` marks, as an offline
    /// collection leaves it.
    fn collected(&self, manifest: &ArchiveReachabilityManifest) -> Self {
        let kept = Self::default();
        for ((namespace, id), bytes) in self.objects.borrow().iter() {
            if manifest
                .plugin_objects
                .get(namespace)
                .is_some_and(|ids| ids.contains(id))
            {
                kept.put(namespace, id, bytes.clone());
            }
        }
        kept
    }
}

impl PackageArchiveStore for Store {
    fn store_package_archive_object(
        &self,
        namespace: &str,
        id: &str,
        bytes: &[u8],
    ) -> Result<(), CanwuError> {
        self.put(namespace, id, bytes.to_vec());
        Ok(())
    }

    fn load_package_archive_object(
        &self,
        namespace: &str,
        id: &str,
    ) -> Result<Option<Vec<u8>>, CanwuError> {
        if self.counting.get() {
            *self
                .reads
                .borrow_mut()
                .entry(namespace.to_owned())
                .or_default() += 1;
        }
        Ok(self.bytes(namespace, id))
    }

    fn persist_package_archive_retention(
        &self,
        handle: &PackageArchiveRetentionHandleV1,
    ) -> Result<(), CanwuError> {
        self.handles
            .borrow_mut()
            .insert(handle.id.clone(), handle.clone());
        Ok(())
    }

    fn load_package_archive_retention(
        &self,
        id: &str,
    ) -> Result<Option<PackageArchiveRetentionHandleV1>, CanwuError> {
        Ok(self.handles.borrow().get(id).cloned())
    }

    fn finalize_package_archive_retention(
        &self,
        handle: &PackageArchiveRetentionHandleV1,
    ) -> Result<(), CanwuError> {
        self.persist_package_archive_retention(handle)
    }
}

impl PluginArchiveObjectProvider for Store {
    fn load_plugin_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, CanwuError> {
        self.load_package_archive_object(namespace, object_id)
    }
}

fn record(key: u64, generation: u64) -> PackageArchiveRecordV1<u64, Payload> {
    let payload = Payload { key, generation };
    PackageArchiveRecordV1 {
        key,
        terminal_sequence: key,
        semantic_digest: canwu_api::canonical_hash(
            "canwu.force-supply-reference.archive-record-payload.v1",
            &payload,
        )
        .expect("payload digest"),
        payload,
    }
}

fn prepare(
    store: &Store,
    head: &PackageArchiveHeadStateV1,
    records: Vec<PackageArchiveRecordV1<u64, Payload>>,
) -> PreparedPackageArchiveBatchV1<u64, Payload> {
    let source = canwu_api::canonical_hash("package-archive-test.source", &head.revision)
        .expect("source root");
    prepare_package_archive(FORCE_ARCHIVE_DOMAIN, source, head, records, store)
        .expect("the commit prepares")
}

/// Stores a prepared commit and returns the head that applies it.
fn apply(
    store: &Store,
    head: &PackageArchiveHeadStateV1,
    batch: &PreparedPackageArchiveBatchV1<u64, Payload>,
) -> PackageArchiveHeadStateV1 {
    let verified = batch
        .store_and_verify(FORCE_ARCHIVE_DOMAIN, store)
        .expect("the commit stores and verifies");
    sealed_archive_head(
        FORCE_ARCHIVE_DOMAIN,
        PackageArchiveHeadStateV1 {
            revision: head.revision + 1,
            directory_root: Some(verified.directory_root),
            archived_record_count: head.archived_record_count
                + u64::from(verified.archived_records),
            semantic_digest: String::new(),
        },
    )
    .expect("the head seals")
}

/// An archive of one-record commits for keys `1..=commits`.
fn archive(store: &Store, commits: u64) -> PackageArchiveHeadStateV1 {
    let mut head = sealed_archive_head(FORCE_ARCHIVE_DOMAIN, PackageArchiveHeadStateV1::default())
        .expect("empty head");
    for key in 1..=commits {
        let batch = prepare(store, &head, vec![record(key, 0)]);
        head = apply(store, &head, &batch);
    }
    head
}

fn lookup(
    store: &Store,
    head: &PackageArchiveHeadStateV1,
    key: u64,
) -> Result<Option<Payload>, CanwuError> {
    load_package_archive_record::<u64, Payload>(FORCE_ARCHIVE_DOMAIN, head, store, &key)
        .map(|found| found.map(|record| record.payload))
}

fn audit(store: &Store, head: &PackageArchiveHeadStateV1) -> Result<(), CanwuError> {
    validate_package_archive_store::<u64, Payload>(
        FORCE_ARCHIVE_DOMAIN,
        head,
        &BTreeMap::new(),
        &BTreeMap::new(),
        store,
    )
}

fn directory(store: &Store, id: &str) -> PackageArchiveIndexDirectoryV1 {
    serde_json::from_slice(
        &store
            .bytes(FORCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE, id)
            .expect("the directory is stored"),
    )
    .expect("the directory decodes")
}

fn head_lookup_root(store: &Store, head: &PackageArchiveHeadStateV1) -> String {
    directory(store, head.directory_root.as_deref().expect("a head")).lookup_root
}

#[test]
fn a_lookup_reads_one_index_path_whatever_the_number_of_commits() {
    let store = Store::default();
    let mut head = archive(&store, 400);
    // Key 7 is archived again, with a newer record, by a commit of several
    // records.
    let batch = prepare(
        &store,
        &head,
        vec![record(7, 1), record(1_000, 0), record(1_001, 0)],
    );
    assert!(
        batch
            .superseded_lookup_pages
            .contains(&head_lookup_root(&store, &head)),
        "the new index replaces the previous root"
    );
    head = apply(&store, &head, &batch);
    assert_eq!(head.revision, 401);
    audit(&store, &head).expect("the restore audit rebuilds the head's index");

    let (missed, miss_reads) = store.count_reads(|| lookup(&store, &head, 5_000));
    assert_eq!(missed.expect("a miss is not an error"), None);
    let (oldest, hit_reads) = store.count_reads(|| lookup(&store, &head, 1));
    assert_eq!(
        oldest.expect("the oldest record resolves"),
        Some(Payload {
            key: 1,
            generation: 0
        })
    );
    assert_eq!(
        lookup(&store, &head, 7).expect("a re-archived key resolves"),
        Some(Payload {
            key: 7,
            generation: 1
        }),
        "a key archived twice resolves to its newest record"
    );

    // One directory and one page per index level for a miss, and the blob
    // that holds the record besides for a hit. Before the index, a miss read
    // every directory, blob, and page of the 401 commits.
    for reads in [&miss_reads, &hit_reads] {
        assert_eq!(reads.get(FORCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE), Some(&1));
        let pages = reads
            .get(FORCE_ARCHIVE_LOOKUP_PAGE_NAMESPACE)
            .copied()
            .unwrap_or(0);
        assert!(
            (1..=4).contains(&pages),
            "{pages} index pages read for 403 keys"
        );
    }
    assert_eq!(miss_reads.get(FORCE_ARCHIVE_BLOB_NAMESPACE), None);
    assert_eq!(hit_reads.get(FORCE_ARCHIVE_BLOB_NAMESPACE), Some(&1));
    for reads in [&miss_reads, &hit_reads] {
        assert!(
            reads.keys().all(|namespace| [
                FORCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE,
                FORCE_ARCHIVE_LOOKUP_PAGE_NAMESPACE,
                FORCE_ARCHIVE_BLOB_NAMESPACE
            ]
            .contains(&namespace.as_str())),
            "a lookup reads no membership or temporal page: {reads:?}"
        );
    }
}

#[test]
fn a_missing_or_forged_index_page_is_an_error_never_a_miss() {
    let store = Store::default();
    let first = archive(&store, 1);
    let head = {
        let mut head = first.clone();
        for key in 2..=40 {
            let batch = prepare(&store, &head, vec![record(key, 0)]);
            head = apply(&store, &head, &batch);
        }
        head
    };
    let root = head_lookup_root(&store, &head);
    let first_root = head_lookup_root(&store, &first);

    // A missing root page fails every lookup, for a stored key and for a key
    // that was never archived, and fails the restore audit.
    let bytes = store.remove(FORCE_ARCHIVE_LOOKUP_PAGE_NAMESPACE, &root);
    assert!(lookup(&store, &head, 3).is_err());
    assert!(lookup(&store, &head, 5_000).is_err());
    assert!(audit(&store, &head).is_err());

    // A valid page stored under another page's id is forged.
    let other = store
        .bytes(FORCE_ARCHIVE_LOOKUP_PAGE_NAMESPACE, &first_root)
        .expect("the first commit's root page is kept");
    store.put(FORCE_ARCHIVE_LOOKUP_PAGE_NAMESPACE, &root, other);
    assert!(lookup(&store, &head, 5_000).is_err());
    assert!(audit(&store, &head).is_err());

    store.put(FORCE_ARCHIVE_LOOKUP_PAGE_NAMESPACE, &root, bytes);
    assert_eq!(
        lookup(&store, &head, 3).expect("the index is whole again"),
        Some(Payload {
            key: 3,
            generation: 0
        })
    );
    audit(&store, &head).expect("the restored store audits");

    // A directory is bounded before it is decoded: the same JSON padded past
    // the bound decodes to the same authentic directory, and is refused.
    let head_id = head.directory_root.clone().expect("a head");
    let honest = store
        .bytes(FORCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE, &head_id)
        .expect("the head directory is stored");
    let mut padded = honest.clone();
    padded.resize(MAX_PACKAGE_ARCHIVE_DIRECTORY_BYTES + 1, b' ');
    store.put(FORCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE, &head_id, padded);
    assert!(lookup(&store, &head, 3).is_err());
    store.put(FORCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE, &head_id, honest);

    // An index that does not hold every archived key fails the audit even
    // though each of its pages authenticates: a head that names the first
    // commit's index over the full chain.
    let mut detached = directory(&store, head.directory_root.as_deref().expect("a head"));
    detached.lookup_root = first_root;
    detached.id.clear();
    detached.semantic_digest.clear();
    let forged_id = canwu_api::canonical_hash(
        "canwu.force-supply-reference.archive-directory.v1",
        &detached,
    )
    .expect("directory digest");
    detached.id.clone_from(&forged_id);
    detached.semantic_digest.clone_from(&forged_id);
    store.put(
        FORCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE,
        &forged_id,
        serde_json::to_vec(&detached).expect("directory encodes"),
    );
    let forged_head = sealed_archive_head(
        FORCE_ARCHIVE_DOMAIN,
        PackageArchiveHeadStateV1 {
            directory_root: Some(forged_id),
            ..head
        },
    )
    .expect("forged head seals");
    assert!(audit(&store, &forged_head).is_err());
}

#[test]
fn reachability_keeps_every_page_the_head_or_a_queued_commit_reads() {
    let store = Store::default();
    let head = archive(&store, 30);
    // A commit stored and queued but not yet applied: the kernel marks its
    // directory before the participant runs.
    let queued = prepare(&store, &head, vec![record(31, 0), record(32, 0)]);
    let queued_head = apply(&store, &head, &queued);
    let mut manifest = ArchiveReachabilityManifest::default();
    manifest.insert_plugin_object(
        FORCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE,
        queued.directory.id.clone(),
    );
    extend_archive_reachability::<u64, Payload>(
        FORCE_ARCHIVE_DOMAIN,
        head.directory_root.as_deref(),
        std::iter::empty(),
        &store,
        &mut manifest,
    )
    .expect("the archive is reachable");

    // After a collection that keeps only marked objects, the head and the
    // queued commit, once it applies, still resolve every key and audit.
    let collected = store.collected(&manifest);
    for (current, keys) in [(&head, 1..=30), (&queued_head, 1..=32)] {
        audit(&collected, current).expect("the collected store audits");
        for key in keys {
            assert_eq!(
                lookup(&collected, current, key).expect("the key resolves"),
                Some(Payload { key, generation: 0 })
            );
        }
    }

    // Pages an earlier commit's index no longer shares with the head may be
    // deleted; reachability still marks the head's index and does not refuse
    // the gap.
    let first_root = head_lookup_root(&store, &archive(&Store::default(), 1));
    let superseded = store
        .ids(FORCE_ARCHIVE_LOOKUP_PAGE_NAMESPACE)
        .into_iter()
        .filter(|id| id == &first_root)
        .collect::<Vec<_>>();
    assert_eq!(
        superseded.len(),
        1,
        "the first commit's root page is stored"
    );
    store.remove(FORCE_ARCHIVE_LOOKUP_PAGE_NAMESPACE, &first_root);
    let mut after_prune = ArchiveReachabilityManifest::default();
    extend_archive_reachability::<u64, Payload>(
        FORCE_ARCHIVE_DOMAIN,
        head.directory_root.as_deref(),
        std::iter::empty(),
        &store,
        &mut after_prune,
    )
    .expect("a pruned earlier index is not refused");
    audit(&store.collected(&after_prune), &head).expect("the head's index is whole");

    // A page the head's index reads must be present.
    store.remove(
        FORCE_ARCHIVE_LOOKUP_PAGE_NAMESPACE,
        &head_lookup_root(&store, &head),
    );
    assert!(
        extend_archive_reachability::<u64, Payload>(
            FORCE_ARCHIVE_DOMAIN,
            head.directory_root.as_deref(),
            std::iter::empty(),
            &store,
            &mut ArchiveReachabilityManifest::default(),
        )
        .is_err()
    );
}
