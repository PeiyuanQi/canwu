//! Downstream gap G-52: a cold archive lookup is decided from authenticated
//! index reads alone.
//!
//! Every resource operation asks the archive whether its operation key was
//! used before. The fixture drives a plugin-owned run through many archive
//! commits and counts, at the archive provider, exactly what one boundary
//! loads for a new operation and for a re-sent archived one. A miss reads the
//! head directory and one lookup page per index level, and no blob or
//! per-commit page, however many commits came before. A hit also reads and
//! authenticates the one blob that holds its record. A missing or forged
//! lookup page or blob fails the boundary and is never taken for a miss, and
//! a restored run and an exact replay settle the same way.
//!
//! The index is rewritten along the paths of each commit's keys, so the
//! fixture also pins what the store must keep: the reachability manifest marks
//! every lookup page the chain wrote, a host application that deletes the
//! pages each commit superseded keeps a working run and a working restore,
//! and it gives up exact replay across those commits. The live pages, those
//! of the head and of a queued commit, can be recomputed when a list is lost.
//! A second fixture pins the index itself: it is a function of its set of
//! entries, whatever commits they arrived in, and a key archived again points
//! at its newest record.

use canwu_api::{
    BoundaryRequest, Canwu, CanwuError, DecisionArchiveBlob, DecisionArchiveProvider,
    DecisionError, EntityRef, KnowledgeHolderRef, PersonId, PluginArchiveObjectProvider, Scenario,
    SimTime, StatePageRetentionLedger,
};
use canwu_resource::*;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::RwLock;

/// Archive commits after the first. The 0.14.0 lookup read five objects per
/// commit; the assertions below would see more than 200.
const LATER_COMMITS: usize = 40;

/// Stored archive objects by namespace and object id.
type ArchiveObjects = BTreeMap<(String, String), Vec<u8>>;

/// A content-addressed archive store that counts what the runtime loads
/// through its archive provider, by namespace. Reads the host application
/// makes itself are not counted: they belong to archive maintenance, not to a
/// boundary.
#[derive(Clone, Default)]
struct CountingArchive {
    objects: Arc<RwLock<ArchiveObjects>>,
    retention: Arc<RwLock<BTreeMap<String, ResourceArchiveRetentionHandleV1>>>,
    provider_loads: Arc<RwLock<BTreeMap<String, usize>>>,
}

impl CountingArchive {
    fn take_loads(&self) -> BTreeMap<String, usize> {
        std::mem::take(&mut *self.provider_loads.write().unwrap())
    }

    fn stored_lookup_pages(&self) -> BTreeSet<String> {
        self.objects
            .read()
            .unwrap()
            .keys()
            .filter(|(namespace, _)| namespace == RESOURCE_ARCHIVE_LOOKUP_PAGE_NAMESPACE)
            .map(|(_, id)| id.clone())
            .collect()
    }

    /// The lookup pages the archive reachability manifest of `canwu` marks.
    fn marked_lookup_pages(&self, canwu: &Canwu) -> BTreeSet<String> {
        canwu
            .archive_reachability_manifest(&[], &StatePageRetentionLedger::default(), self, self)
            .expect("archive reachability manifest")
            .plugin_objects
            .remove(RESOURCE_ARCHIVE_LOOKUP_PAGE_NAMESPACE)
            .unwrap_or_default()
    }

    /// Replaces one stored object, or removes it, and returns what it held.
    fn swap(&self, namespace: &str, id: &str, bytes: Option<Vec<u8>>) -> Option<Vec<u8>> {
        let key = (namespace.to_owned(), id.to_owned());
        let mut objects = self.objects.write().unwrap();
        match bytes {
            Some(bytes) => objects.insert(key, bytes),
            None => objects.remove(&key),
        }
    }
}

impl ResourceArchiveStore for CountingArchive {
    fn store_resource_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
        bytes: &[u8],
    ) -> Result<(), ResourceError> {
        self.objects
            .write()
            .unwrap()
            .insert((namespace.to_owned(), object_id.to_owned()), bytes.to_vec());
        Ok(())
    }

    fn load_resource_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, ResourceError> {
        Ok(self
            .objects
            .read()
            .unwrap()
            .get(&(namespace.to_owned(), object_id.to_owned()))
            .cloned())
    }

    fn persist_resource_archive_retention(
        &self,
        handle: &ResourceArchiveRetentionHandleV1,
    ) -> Result<(), ResourceError> {
        self.retention
            .write()
            .unwrap()
            .insert(handle.id.clone(), handle.clone());
        Ok(())
    }

    fn finalize_resource_archive_retention(
        &self,
        handle_id: &str,
        phase: ResourceArchiveRetentionPhaseV1,
    ) -> Result<(), ResourceError> {
        self.retention
            .write()
            .unwrap()
            .get_mut(handle_id)
            .ok_or_else(|| ResourceError::NotFound("retention handle unavailable".to_owned()))?
            .phase = phase;
        Ok(())
    }
}

impl DecisionArchiveProvider for CountingArchive {
    fn load_decision_archive(
        &self,
        _locator: &str,
    ) -> Result<Option<DecisionArchiveBlob>, DecisionError> {
        Ok(None)
    }
}

impl PluginArchiveObjectProvider for CountingArchive {
    fn load_plugin_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, CanwuError> {
        *self
            .provider_loads
            .write()
            .unwrap()
            .entry(namespace.to_owned())
            .or_default() += 1;
        Ok(self
            .objects
            .read()
            .unwrap()
            .get(&(namespace.to_owned(), object_id.to_owned()))
            .cloned())
    }
}

fn holder() -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(PersonId::new(1))
}

fn state(canwu: &Canwu) -> ResourceState {
    resource_state(canwu)
        .expect("resource query")
        .expect("resource state")
        .1
}

/// One requester-scoped allocation pass. The requester owns no demand, so the
/// pass settles as a durable rejection: one operation, one archive lookup,
/// one terminal outcome.
fn allocation(canwu: &Canwu, key: &str) -> ResourceAllocationRequestV1 {
    ResourceAllocationRequestV1 {
        operation_key: ResourceOperationKey::new(key).expect("operation key"),
        expected_state_revision: state(canwu).state_revision,
        at: canwu.time(),
        candidate_limit: 1,
    }
}

fn settle(canwu: &mut Canwu) -> Result<(), CanwuError> {
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .map(|_| ())
}

fn settle_allocation(
    canwu: &mut Canwu,
    request: &ResourceAllocationRequestV1,
) -> Result<(), CanwuError> {
    enqueue_resource_allocation(canwu, canwu.time(), &holder(), request)?;
    settle(canwu)
}

/// Archives every hot terminal candidate in one commit and acknowledges it.
fn archive_all(canwu: &mut Canwu, archive: &CountingArchive) -> PreparedResourceArchiveBatchV1 {
    let hot = state(canwu);
    let prepared = hot
        .prepare_resource_archive(hot.terminal_archive_candidates.len(), archive)
        .expect("prepare archive commit");
    let receipt = enqueue_resource_archive(canwu, &prepared, archive).expect("enqueue commit");
    settle(canwu).expect("archive commit boundary");
    finalize_resource_archive_retention(canwu, archive, &receipt).expect("finalize retention");
    settle(canwu).expect("retention acknowledgement boundary");
    prepared
}

/// Settles one new operation and returns what its boundary loaded.
fn loads_of_new_operation(
    canwu: &mut Canwu,
    archive: &CountingArchive,
    key: &str,
) -> BTreeMap<String, usize> {
    let request = allocation(canwu, key);
    archive.take_loads();
    settle_allocation(canwu, &request).expect("new operation settles");
    assert!(
        state(canwu).outcomes.contains_key(&request.operation_key),
        "a key the archive never held settles as a new operation"
    );
    archive.take_loads()
}

fn head_directory(canwu: &Canwu, archive: &CountingArchive) -> ResourceArchiveIndexDirectoryV1 {
    let root = state(canwu)
        .archive_head
        .directory_root
        .expect("archive head");
    let bytes = archive
        .load_resource_archive_object(RESOURCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE, &root)
        .expect("store read")
        .expect("head directory");
    serde_json::from_slice(&bytes).expect("head directory decodes")
}

#[test]
#[allow(clippy::too_many_lines)]
fn cold_lookup_reads_only_authenticated_index_pages_whatever_the_commit_count() {
    let plugin = ResourcePlugin::default();
    let archive = CountingArchive::default();
    let resource = ResourceState::empty(ResourceLimitsV1::canonical()).expect("resource state");
    let scenario = Scenario::new(SimTime::EPOCH, vec![EntityRef::Person(PersonId::new(1))])
        .with_domain_records(resource.into_records().expect("resource record"));
    let mut canwu = Canwu::new_with_plugins(52, scenario, &[&plugin]).expect("runtime");
    canwu.set_plugin_archive_object_provider(Arc::new(archive.clone()));

    // The first commit holds the operation that is re-sent at the end.
    let first = allocation(&canwu, "fixture:allocate:first");
    settle_allocation(&mut canwu, &first).expect("first operation");
    let first_outcome = state(&canwu).outcomes[&first.operation_key].clone();
    let first_commit = archive_all(&mut canwu, &archive);
    let after_one_commit =
        loads_of_new_operation(&mut canwu, &archive, "fixture:allocate:after-one-commit");

    let mut superseded = BTreeSet::new();
    for commit in 0..LATER_COMMITS {
        let request = allocation(&canwu, &format!("fixture:allocate:filler:{commit}"));
        settle_allocation(&mut canwu, &request).expect("filler operation");
        superseded.extend(archive_all(&mut canwu, &archive).superseded_lookup_pages);
    }
    let archived = state(&canwu);
    assert_eq!(
        archived.archive_head.revision,
        u64::try_from(LATER_COMMITS + 1).expect("commit count")
    );
    assert!(!archived.outcomes.contains_key(&first.operation_key));

    // A miss: the head directory and one lookup page per index level.
    let after_many_commits =
        loads_of_new_operation(&mut canwu, &archive, "fixture:allocate:after-many-commits");
    for loads in [&after_one_commit, &after_many_commits] {
        assert_eq!(
            loads.keys().map(String::as_str).collect::<Vec<_>>(),
            [
                RESOURCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE,
                RESOURCE_ARCHIVE_LOOKUP_PAGE_NAMESPACE
            ],
            "a miss reads no blob, membership page, or temporal page"
        );
        assert_eq!(loads[RESOURCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE], 1);
    }
    assert_eq!(after_one_commit[RESOURCE_ARCHIVE_LOOKUP_PAGE_NAMESPACE], 1);
    assert!(
        after_many_commits[RESOURCE_ARCHIVE_LOOKUP_PAGE_NAMESPACE] <= 3,
        "{} archived records fit three index levels, whatever the {} commits: {after_many_commits:?}",
        archived.archive_head.archived_record_count,
        archived.archive_head.revision,
    );

    // A hit: the same index path, then the one blob that holds the record.
    let before_hit = state(&canwu);
    archive.take_loads();
    settle_allocation(&mut canwu, &first).expect("a re-sent archived operation is a no-op");
    let hit = archive.take_loads();
    assert_eq!(hit[RESOURCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE], 1);
    assert_eq!(hit[RESOURCE_ARCHIVE_BLOB_NAMESPACE], 1);
    assert!(!hit.contains_key(RESOURCE_ARCHIVE_MEMBERSHIP_PAGE_NAMESPACE));
    assert!(!hit.contains_key(RESOURCE_ARCHIVE_TEMPORAL_PAGE_NAMESPACE));
    assert_eq!(state(&canwu), before_hit, "the re-send changed nothing");
    assert_eq!(
        archived_resource_operation_outcome(&before_hit, &archive, &first.operation_key)
            .expect("authenticated cold lookup"),
        Some(first_outcome)
    );

    // A hit authenticates what it returns: a forged blob fails the boundary.
    let blob_id = first_commit.blob.content_id.clone();
    let mut forged_blob = first_commit.blob.clone();
    forged_blob.records[0].quantity += 1;
    let original_blob = archive.swap(
        RESOURCE_ARCHIVE_BLOB_NAMESPACE,
        &blob_id,
        Some(serde_json::to_vec(&forged_blob).expect("forged blob bytes")),
    );
    assert!(settle_allocation(&mut canwu, &first).is_err());
    archive.swap(RESOURCE_ARCHIVE_BLOB_NAMESPACE, &blob_id, original_blob);
    settle(&mut canwu).expect("the queued re-send settles once its blob authenticates");
    assert_eq!(state(&canwu), before_hit);

    // A miss is decided by authenticated pages only: a missing or forged
    // lookup page fails the boundary instead of admitting the operation.
    let lookup_root = head_directory(&canwu, &archive).lookup_root;
    let guarded = allocation(&canwu, "fixture:allocate:guarded");
    let original_page = archive.swap(RESOURCE_ARCHIVE_LOOKUP_PAGE_NAMESPACE, &lookup_root, None);
    assert!(settle_allocation(&mut canwu, &guarded).is_err());
    let forged_page = ResourceArchiveLookupPageV1::default();
    archive.swap(
        RESOURCE_ARCHIVE_LOOKUP_PAGE_NAMESPACE,
        &lookup_root,
        Some(serde_json::to_vec(&forged_page).expect("forged page bytes")),
    );
    assert!(settle(&mut canwu).is_err());
    assert!(!state(&canwu).outcomes.contains_key(&guarded.operation_key));
    archive.swap(
        RESOURCE_ARCHIVE_LOOKUP_PAGE_NAMESPACE,
        &lookup_root,
        original_page,
    );
    settle(&mut canwu).expect("the queued operation settles once the index authenticates");
    assert!(state(&canwu).outcomes.contains_key(&guarded.operation_key));

    // Restore audits the chain and its index; exact replay settles the same.
    let snapshot = canwu.snapshot_json().expect("snapshot");
    from_resource_snapshot_json_with_archive_store(
        &snapshot,
        &[&plugin],
        Arc::new(archive.clone()),
    )
    .expect("restore authenticates the archive and its lookup index");
    let replayed = replay_resource_from_journal_with_archive_store(
        &[&plugin],
        &canwu.replay_journal(),
        Arc::new(archive.clone()),
    )
    .expect("exact replay with the archive store");
    assert_eq!(replayed.snapshot_json().expect("replay snapshot"), snapshot);
    assert_eq!(replayed.boundary_head_hash(), canwu.boundary_head_hash());

    // Retention. A collection keeps every lookup page the chain ever wrote,
    // because a restore or a replay reads the index of the head it passes.
    assert!(!superseded.is_empty());
    assert_eq!(
        archive.marked_lookup_pages(&canwu),
        archive.stored_lookup_pages()
    );
    // What a live run reads can be recomputed without the lists: every other
    // stored page is one that an applied commit superseded.
    let live = live_resource_archive_lookup_pages(&state(&canwu), &archive, std::iter::empty())
        .expect("live lookup pages");
    assert_eq!(
        archive
            .stored_lookup_pages()
            .difference(&live)
            .cloned()
            .collect::<BTreeSet<_>>(),
        superseded
    );
    // A host application that deletes what each applied commit superseded
    // keeps a whole head index: the run goes on, hits and misses included,
    // and restores.
    for id in &superseded {
        archive.swap(RESOURCE_ARCHIVE_LOOKUP_PAGE_NAMESPACE, id, None);
    }
    assert_eq!(
        archive.marked_lookup_pages(&canwu),
        archive.stored_lookup_pages()
    );
    let before_resend = state(&canwu);
    settle_allocation(&mut canwu, &first).expect("re-send after pruning");
    assert_eq!(state(&canwu), before_resend);
    loads_of_new_operation(&mut canwu, &archive, "fixture:allocate:after-pruning");
    from_resource_snapshot_json_with_archive_store(
        &canwu.snapshot_json().expect("snapshot after pruning"),
        &[&plugin],
        Arc::new(archive.clone()),
    )
    .expect("the head index is whole after pruning");
    // It gives up exact replay across the pruned commits: an earlier boundary
    // would read a page of the index its own head named.
    assert!(
        replay_resource_from_journal_with_archive_store(
            &[&plugin],
            &canwu.replay_journal(),
            Arc::new(archive.clone()),
        )
        .is_err()
    );

    // A commit that is queued and not yet settled: its pages are live, and
    // so are the head's pages it would supersede.
    let hot = state(&canwu);
    let queued = hot
        .prepare_resource_archive(hot.terminal_archive_candidates.len(), &archive)
        .expect("prepare a commit to queue");
    let queued_receipt =
        enqueue_resource_archive(&mut canwu, &queued, &archive).expect("queue the commit");
    let live = live_resource_archive_lookup_pages(
        &hot,
        &archive,
        [queued_receipt.directory_root.as_str()],
    )
    .expect("live lookup pages with a queued commit");
    assert_eq!(live, archive.stored_lookup_pages());
    assert!(!queued.superseded_lookup_pages.is_empty());
    assert!(queued.superseded_lookup_pages.is_subset(&live));
    assert!(queued.lookup_pages.keys().all(|id| live.contains(id)));
}

fn membership(index: usize, blob: char, ordinal: u16) -> ResourceArchiveMembershipV1 {
    ResourceArchiveMembershipV1 {
        key: ResourceTerminalRecordKeyV1::Outcome(
            ResourceOperationKey::new(format!("fixture:indexed:{index}")).expect("operation key"),
        ),
        blob_id: blob.to_string().repeat(64),
        ordinal,
        terminal_sequence: 1,
        semantic_digest: "0".repeat(64),
    }
}

/// Stores the pages of one extension and returns every page its root reaches.
fn store_and_walk(
    archive: &CountingArchive,
    lookup: &PreparedResourceArchiveLookupV1,
) -> BTreeSet<String> {
    for (id, page) in &lookup.pages {
        archive
            .store_resource_archive_object(
                RESOURCE_ARCHIVE_LOOKUP_PAGE_NAMESPACE,
                id,
                &serde_json::to_vec(page).expect("lookup page bytes"),
            )
            .expect("store lookup page");
    }
    let mut reached = BTreeSet::new();
    let mut pending = vec![lookup.root.clone()];
    while let Some(id) = pending.pop() {
        let bytes = archive
            .load_resource_archive_object(RESOURCE_ARCHIVE_LOOKUP_PAGE_NAMESPACE, &id)
            .expect("store read")
            .expect("reachable lookup page");
        let page: ResourceArchiveLookupPageV1 =
            serde_json::from_slice(&bytes).expect("lookup page decodes");
        assert_eq!(page.id().expect("lookup page id"), id);
        assert!(page.entries.len() <= MAX_RESOURCE_ARCHIVE_LOOKUP_LEAF_ENTRIES);
        pending.extend(page.children.into_values());
        reached.insert(id);
    }
    reached
}

#[test]
fn lookup_index_is_a_function_of_its_key_set() {
    let memberships: Vec<_> = (0..400).map(|index| membership(index, 'a', 0)).collect();
    let whole =
        prepare_resource_archive_lookup(&(), None, &memberships).expect("index from nothing");

    // The same keys in commits of any size, in either order, give the same
    // root, and each extension names exactly the pages it stops reaching.
    for commit_size in [1, 7, 64] {
        for reversed in [false, true] {
            let archive = CountingArchive::default();
            let mut root: Option<String> = None;
            let mut reached = BTreeSet::new();
            let commits: Vec<&[ResourceArchiveMembershipV1]> = if reversed {
                memberships.rchunks(commit_size).collect()
            } else {
                memberships.chunks(commit_size).collect()
            };
            for commit in commits {
                let lookup = prepare_resource_archive_lookup(&archive, root.as_deref(), commit)
                    .expect("extend the index");
                let now_reached = store_and_walk(&archive, &lookup);
                assert_eq!(
                    reached
                        .difference(&now_reached)
                        .cloned()
                        .collect::<BTreeSet<_>>(),
                    lookup.superseded
                );
                assert!(lookup.pages.keys().all(|id| now_reached.contains(id)));
                reached = now_reached;
                root = Some(lookup.root);
            }
            assert_eq!(root.as_deref(), Some(whole.root.as_str()));
        }
    }

    // A key archived again points at its newest record, as if the index had
    // been built from the newest record of every key.
    let archive = CountingArchive::default();
    store_and_walk(&archive, &whole);
    let again = [membership(7, 'b', 9), membership(400, 'b', 1)];
    let extended = prepare_resource_archive_lookup(&archive, Some(&whole.root), &again)
        .expect("extend with an indexed key");
    let mut newest = memberships;
    newest[7] = again[0].clone();
    newest.push(again[1].clone());
    assert_eq!(
        extended.root,
        prepare_resource_archive_lookup(&(), None, &newest)
            .expect("index of the newest records")
            .root
    );
    // One commit cannot name a key twice.
    assert!(
        prepare_resource_archive_lookup(
            &archive,
            Some(&whole.root),
            &[membership(1, 'c', 0), membership(1, 'c', 1)]
        )
        .is_err()
    );
}
