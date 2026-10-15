//! Downstream gap G-54: archive volume and archive lifetime.
//!
//! A resource root may hold thousands of hot terminal candidates, and one
//! archive commit carries all of them as several pages: each page is
//! authenticated, every record of every page resolves through the lookup
//! index, and a restored run and an exact replay agree. A run may also make
//! any number of archive commits. The second fixture passes the 2,048
//! commits at which every 0.14.0 lookup failed its boundary, and the
//! operation archived by the very first commit still resolves.
//!
//! An archive commit never needs budget it cannot have. The third fixture
//! queues two commits against a root whose candidate budget is full: the
//! first applies, the second is stale and terminates as a no-op instead of
//! failing the boundary for ever, and acknowledgements that arrive twice or
//! after their receipt was archived settle too. The fourth fixture queues one
//! batch twice: its acknowledgement retires the stale receipt of the
//! duplicate. The fifth queues an operation that does not fit a full budget:
//! it is skipped, and sent again with the commit that makes room queued
//! behind it, the commit settles first and the operation after it.

use canwu_api::{
    BoundaryRequest, Canwu, EntityRef, KnowledgeHolderRef, PersonId, Scenario, SimTime,
};
use canwu_resource::*;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

/// Hot terminal records of the multi-page fixture: one full page and a part,
/// more than a 0.14.0 root could hold or one 0.14.0 commit could carry.
const MULTI_PAGE_RECORDS: usize = MAX_RESOURCE_ARCHIVE_PAGE_ENTRIES + 44;
/// Archive commits of the lifetime fixture, past the 0.14.0 limit of 2,048.
const LIFETIME_COMMITS: u64 = 2_048 + 12;

/// Stored archive objects by namespace and object id.
type ArchiveObjects = BTreeMap<(String, String), Vec<u8>>;

#[derive(Clone, Default)]
struct MemoryArchive {
    objects: Rc<RefCell<ArchiveObjects>>,
    retention: Rc<RefCell<BTreeMap<String, ResourceArchiveRetentionHandleV1>>>,
}

impl ResourceArchiveStore for MemoryArchive {
    fn store_resource_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
        bytes: &[u8],
    ) -> Result<(), ResourceError> {
        self.objects
            .borrow_mut()
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
            .borrow()
            .get(&(namespace.to_owned(), object_id.to_owned()))
            .cloned())
    }

    fn persist_resource_archive_retention(
        &self,
        handle: &ResourceArchiveRetentionHandleV1,
    ) -> Result<(), ResourceError> {
        self.retention
            .borrow_mut()
            .insert(handle.id.clone(), handle.clone());
        Ok(())
    }

    fn finalize_resource_archive_retention(
        &self,
        handle_id: &str,
        phase: ResourceArchiveRetentionPhaseV1,
    ) -> Result<(), ResourceError> {
        self.retention
            .borrow_mut()
            .get_mut(handle_id)
            .ok_or_else(|| ResourceError::NotFound("retention handle unavailable".to_owned()))?
            .phase = phase;
        Ok(())
    }
}

impl canwu_api::PluginArchiveObjectProvider for MemoryArchive {
    fn load_plugin_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, canwu_api::CanwuError> {
        Ok(self
            .objects
            .borrow()
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

fn operation_key(index: usize) -> ResourceOperationKey {
    ResourceOperationKey::new(format!("fixture:cancel:{index:04}")).expect("operation key")
}

/// One durable rejection: cancelling a demand that does not exist leaves a
/// terminal outcome and nothing else.
fn reject(resource: &mut ResourceState, index: usize) -> ResourceOperationOutcome {
    resource
        .apply_operation(&ResourceOperationRequestV1::CancelDemand(
            ResourceCancelDemandRequestV1 {
                operation_key: operation_key(index),
                demand: ResourceDemandId::new(format!("fixture:missing:{index:04}"))
                    .expect("demand"),
                expected_demand_revision: ResourceRevision::INITIAL,
            },
        ))
        .expect("durable rejection")
}

fn runtime(plugin: &ResourcePlugin, resource: ResourceState, archive: &MemoryArchive) -> Canwu {
    let scenario = Scenario::new(SimTime::EPOCH, vec![EntityRef::Person(PersonId::new(1))])
        .with_domain_records(vec![resource.into_record().expect("resource record")]);
    let mut canwu = Canwu::new_with_plugins(54, scenario, &[plugin]).expect("runtime");
    canwu.set_plugin_archive_object_provider(Rc::new(archive.clone()));
    canwu
}

fn settle(canwu: &mut Canwu) {
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("boundary");
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

fn settle_allocation(canwu: &mut Canwu, request: &ResourceAllocationRequestV1) {
    let at = canwu.time();
    enqueue_resource_allocation(canwu, at, &holder(), request).expect("enqueue allocation");
    settle(canwu);
}

#[test]
#[allow(clippy::too_many_lines)]
fn one_archive_commit_carries_several_pages_under_a_budget_in_the_thousands() {
    let mut limits = ResourceLimitsV1::canonical();
    limits.max_archive_candidates = ResourceLimitsV1::MAX_ARCHIVE_CANDIDATES;
    assert!(
        limits.validate().is_err(),
        "every hot receipt of an applied commit is a candidate, so the receipt budget may not be smaller"
    );
    limits.max_archive_maintenance_receipts = ResourceLimitsV1::MAX_ARCHIVE_CANDIDATES;
    limits.validate().expect("the hard maximum is configurable");
    limits.max_archive_candidates += 1;
    limits.max_archive_maintenance_receipts += 1;
    assert!(limits.validate().is_err());
    limits.max_archive_candidates = 2_048;
    limits.max_archive_maintenance_receipts = 2_048;
    let mut resource = ResourceState::empty(limits).expect("resource state");
    let outcomes: Vec<_> = (0..MULTI_PAGE_RECORDS)
        .map(|index| reject(&mut resource, index))
        .collect();
    assert_eq!(
        resource.terminal_archive_candidates.len(),
        MULTI_PAGE_RECORDS
    );

    let plugin = ResourcePlugin::default();
    let archive = MemoryArchive::default();
    let mut canwu = runtime(&plugin, resource, &archive);
    let hot = state(&canwu);
    let prepared = hot
        .prepare_resource_archive(hot.terminal_archive_candidates.len(), &archive)
        .expect("one commit for every candidate");
    assert_eq!(prepared.selected.len(), MULTI_PAGE_RECORDS);
    assert_eq!(
        prepared.blob.records.len(),
        MAX_RESOURCE_ARCHIVE_PAGE_ENTRIES
    );
    assert_eq!(prepared.additional_pages.len(), 1);
    assert_eq!(prepared.additional_pages[0].blob.records.len(), 44);
    assert_eq!(prepared.directory.blob_ids.len(), 2);
    assert_eq!(prepared.directory.membership_pages.len(), 2);
    assert_eq!(prepared.directory.temporal_pages.len(), 2);
    // A shorter batch from the same root shares the first page. Both
    // retention handles name its blob, which is why releasing a handle never
    // deletes an object.
    let shorter = hot
        .prepare_resource_archive(MAX_RESOURCE_ARCHIVE_PAGE_ENTRIES, &archive)
        .expect("a one-page batch from the same root");
    assert_eq!(shorter.blob.content_id, prepared.blob.content_id);
    assert_ne!(shorter.retention.id, prepared.retention.id);
    assert!(
        shorter.retention.object_ids[RESOURCE_ARCHIVE_BLOB_NAMESPACE]
            .is_subset(&prepared.retention.object_ids[RESOURCE_ARCHIVE_BLOB_NAMESPACE])
    );

    let receipt = enqueue_resource_archive(&mut canwu, &prepared, &archive).expect("enqueue");
    settle(&mut canwu);
    finalize_resource_archive_retention(&mut canwu, &archive, &receipt).expect("finalize");
    settle(&mut canwu);
    let archived = state(&canwu);
    assert!(
        archived.outcomes.is_empty(),
        "every candidate left the root"
    );
    assert_eq!(archived.archive_head.revision, 1);
    assert_eq!(
        archived.archive_head.archived_record_count,
        u64::try_from(MULTI_PAGE_RECORDS).expect("record count")
    );
    // Records of the first page and of the second both resolve.
    for index in [
        0,
        MAX_RESOURCE_ARCHIVE_PAGE_ENTRIES - 1,
        MULTI_PAGE_RECORDS - 1,
    ] {
        assert_eq!(
            archived_resource_operation_outcome(&archived, &archive, &operation_key(index))
                .expect("authenticated cold lookup")
                .as_ref(),
            Some(&outcomes[index])
        );
    }

    let snapshot = canwu.snapshot_json().expect("snapshot");
    let journal = canwu.replay_journal();
    from_resource_snapshot_json_with_archive_store(&snapshot, &[&plugin], Rc::new(archive.clone()))
        .expect("restore authenticates every page of the commit");
    let replayed = replay_resource_from_journal_with_archive_store(
        &[&plugin],
        &journal,
        Rc::new(archive.clone()),
    )
    .expect("exact replay with the archive store");
    assert_eq!(replayed.snapshot_json().expect("replay snapshot"), snapshot);
    assert_eq!(replayed.boundary_head_hash(), canwu.boundary_head_hash());

    // Every page is authenticated, not only the first.
    let last_blob = &prepared.additional_pages[0].blob;
    let mut forged = last_blob.clone();
    forged.records[0].quantity += 1;
    archive.objects.borrow_mut().insert(
        (
            RESOURCE_ARCHIVE_BLOB_NAMESPACE.to_owned(),
            last_blob.content_id.clone(),
        ),
        serde_json::to_vec(&forged).expect("forged blob bytes"),
    );
    assert!(
        from_resource_snapshot_json_with_archive_store(
            &snapshot,
            &[&plugin],
            Rc::new(archive.clone()),
        )
        .is_err()
    );
    assert!(
        archived_resource_operation_outcome(
            &archived,
            &archive,
            &operation_key(MULTI_PAGE_RECORDS - 1)
        )
        .is_err()
    );
}

#[test]
fn a_run_passes_2048_archive_commits_and_still_resolves_its_first_operation() {
    let plugin = ResourcePlugin::default();
    let archive = MemoryArchive::default();
    let resource = ResourceState::empty(ResourceLimitsV1::canonical()).expect("resource state");
    let mut canwu = runtime(&plugin, resource, &archive);

    // The first commit archives one real operation. Every later commit
    // archives the maintenance receipt of the commit before it, so the chain
    // grows by one commit per boundary without further operations.
    let first = allocation(&canwu, "fixture:allocate:first");
    settle_allocation(&mut canwu, &first);
    let first_outcome = state(&canwu).outcomes[&first.operation_key].clone();

    let mut unacknowledged = None;
    for _ in 0..LIFETIME_COMMITS {
        if let Some(receipt) = unacknowledged.take() {
            finalize_resource_archive_retention(&mut canwu, &archive, &receipt)
                .expect("finalize the previous commit");
        }
        let hot = state(&canwu);
        let prepared = hot
            .prepare_resource_archive(hot.terminal_archive_candidates.len(), &archive)
            .expect("prepare archive commit");
        unacknowledged =
            Some(enqueue_resource_archive(&mut canwu, &prepared, &archive).expect("enqueue"));
        settle(&mut canwu);
    }
    let archived = state(&canwu);
    assert_eq!(archived.archive_head.revision, LIFETIME_COMMITS);

    // A re-sent archived operation is still a no-op, a new one still
    // settles, and the first operation still resolves exactly.
    settle_allocation(&mut canwu, &first);
    assert!(!state(&canwu).outcomes.contains_key(&first.operation_key));
    let later = allocation(&canwu, "fixture:allocate:later");
    settle_allocation(&mut canwu, &later);
    let current = state(&canwu);
    assert!(current.outcomes.contains_key(&later.operation_key));
    assert_eq!(
        archived_resource_operation_outcome(&current, &archive, &first.operation_key)
            .expect("authenticated cold lookup"),
        Some(first_outcome)
    );

    from_resource_snapshot_json_with_archive_store(
        &canwu.snapshot_json().expect("snapshot"),
        &[&plugin],
        Rc::new(archive.clone()),
    )
    .expect("restore authenticates the whole chain and its lookup index");
}

#[test]
fn a_stale_archive_commit_terminates_against_a_full_candidate_budget() {
    let mut limits = ResourceLimitsV1::canonical();
    limits.max_archive_candidates = 4;
    let mut resource = ResourceState::empty(limits).expect("resource state");
    for index in 0..4 {
        reject(&mut resource, index);
    }
    let plugin = ResourcePlugin::default();
    let archive = MemoryArchive::default();
    let mut canwu = runtime(&plugin, resource, &archive);

    // Two commits prepared from the same full root and queued together: the
    // first applies, which makes the second stale.
    let full = state(&canwu);
    assert_eq!(full.terminal_archive_candidates.len(), 4);
    let first = full
        .prepare_resource_archive(1, &archive)
        .expect("first commit");
    let second = full
        .prepare_resource_archive(2, &archive)
        .expect("second commit");
    let first_receipt = enqueue_resource_archive(&mut canwu, &first, &archive).expect("enqueue");
    let second_receipt = enqueue_resource_archive(&mut canwu, &second, &archive).expect("enqueue");
    settle(&mut canwu);

    let settled = state(&canwu);
    assert_eq!(settled.archive_head.revision, 1);
    assert_eq!(settled.archive_head.archived_record_count, 1);
    assert_eq!(
        settled.archive_head.directory_root.as_deref(),
        Some(first.directory.id.as_str())
    );
    // The budget is still full: three outcomes and the applied commit's
    // receipt. The stale receipt is in the root but is not a candidate.
    assert_eq!(settled.terminal_archive_candidates.len(), 4);
    assert_eq!(
        settled
            .archive_maintenance_receipts
            .values()
            .map(|receipt| (receipt.disposition, receipt.archived_records))
            .collect::<Vec<_>>(),
        [
            (ResourceArchiveMaintenanceDispositionV1::Applied, 1),
            (ResourceArchiveMaintenanceDispositionV1::RejectedStale, 0),
        ]
    );
    let stale_sequence = *settled
        .archive_maintenance_receipts
        .keys()
        .next_back()
        .expect("stale receipt");
    assert!(
        !settled
            .terminal_archive_candidates
            .contains_key(&stale_sequence)
    );
    assert_eq!(
        settled.archive_retention_handles[&second_receipt.retention_handle_id].phase,
        ResourceArchiveRetentionPhaseV1::RejectedStale
    );

    // The stale commit is acknowledged twice before its boundary: the second
    // acknowledgement finds neither receipt nor handle and is a no-op.
    for _ in 0..2 {
        finalize_resource_archive_retention(&mut canwu, &archive, &second_receipt)
            .expect("finalize the stale commit");
    }
    settle(&mut canwu);
    let acknowledged = state(&canwu);
    assert_eq!(acknowledged.archive_maintenance_receipts.len(), 1);
    assert!(
        !acknowledged
            .archive_retention_handles
            .contains_key(&second_receipt.retention_handle_id)
    );
    assert_eq!(
        archive.retention.borrow()[&second_receipt.retention_handle_id].phase,
        ResourceArchiveRetentionPhaseV1::RejectedStale
    );

    // The next commit archives the first commit's receipt, and the first
    // commit is acknowledged behind it in the same boundary: the
    // acknowledgement still retires the handle.
    let third = acknowledged
        .prepare_resource_archive(acknowledged.terminal_archive_candidates.len(), &archive)
        .expect("third commit");
    let third_receipt = enqueue_resource_archive(&mut canwu, &third, &archive).expect("enqueue");
    finalize_resource_archive_retention(&mut canwu, &archive, &first_receipt)
        .expect("finalize the first commit");
    settle(&mut canwu);
    let drained = state(&canwu);
    assert_eq!(drained.archive_head.revision, 2);
    assert_eq!(drained.archive_head.archived_record_count, 5);
    assert!(drained.outcomes.is_empty());
    assert_eq!(
        drained.archive_retention_handles.keys().collect::<Vec<_>>(),
        [&third_receipt.retention_handle_id]
    );

    let snapshot = canwu.snapshot_json().expect("snapshot");
    from_resource_snapshot_json_with_archive_store(&snapshot, &[&plugin], Rc::new(archive.clone()))
        .expect("restore after a stale commit");
    let replayed = replay_resource_from_journal_with_archive_store(
        &[&plugin],
        &canwu.replay_journal(),
        Rc::new(archive.clone()),
    )
    .expect("exact replay settles the stale commit the same way");
    assert_eq!(replayed.snapshot_json().expect("replay snapshot"), snapshot);
    assert_eq!(replayed.boundary_head_hash(), canwu.boundary_head_hash());
}

#[test]
fn the_same_archive_batch_queued_twice_leaves_no_stale_receipt() {
    let mut limits = ResourceLimitsV1::canonical();
    limits.max_archive_candidates = 4;
    let mut resource = ResourceState::empty(limits).expect("resource state");
    for index in 0..4 {
        reject(&mut resource, index);
    }
    let plugin = ResourcePlugin::default();
    let archive = MemoryArchive::default();
    let mut canwu = runtime(&plugin, resource, &archive);

    // One batch queued twice settles once as applied and once as stale, under
    // one retention handle, which stays the handle of the applied commit.
    let batch = state(&canwu)
        .prepare_resource_archive(4, &archive)
        .expect("commit of every candidate");
    let queued = [(); 2].map(|()| {
        enqueue_resource_archive(&mut canwu, &batch, &archive).expect("enqueue the same batch")
    });
    let handle = &queued[0].retention_handle_id;
    assert_eq!(handle, &queued[1].retention_handle_id);
    settle(&mut canwu);
    let doubled = state(&canwu);
    assert_eq!(doubled.archive_head.revision, 1);
    assert_eq!(
        doubled
            .archive_maintenance_receipts
            .values()
            .map(|receipt| (&receipt.retention_handle_id, receipt.disposition))
            .collect::<Vec<_>>(),
        [
            (handle, ResourceArchiveMaintenanceDispositionV1::Applied),
            (
                handle,
                ResourceArchiveMaintenanceDispositionV1::RejectedStale
            ),
        ]
    );
    assert_eq!(
        doubled.archive_retention_handles[handle].phase,
        ResourceArchiveRetentionPhaseV1::Committed
    );

    // The root validates both halves of that: the receipt of an applied
    // commit is an archive candidate and the receipt of a stale one is not.
    let sequences: Vec<_> = doubled
        .archive_maintenance_receipts
        .keys()
        .copied()
        .collect();
    let mut without_applied = doubled.clone();
    without_applied
        .terminal_archive_candidates
        .remove(&sequences[0]);
    let mut with_stale = doubled.clone();
    with_stale.terminal_archive_candidates.insert(
        sequences[1],
        ResourceTerminalRecordKeyV1::ArchiveMaintenanceReceipt(sequences[1]),
    );
    for forged in [without_applied, with_stale] {
        let error = forged.validate().expect_err("candidacy is validated");
        assert!(error.to_string().contains("candidates disagree"));
    }

    // Finalizing either receipt names the applied commit, and its
    // acknowledgement retires the stale receipt as well.
    for receipt in &queued {
        finalize_resource_archive_retention(&mut canwu, &archive, receipt)
            .expect("finalize the batch queued twice");
    }
    settle(&mut canwu);
    let retired = state(&canwu);
    assert_eq!(
        retired
            .archive_maintenance_receipts
            .values()
            .map(|receipt| receipt.disposition)
            .collect::<Vec<_>>(),
        [ResourceArchiveMaintenanceDispositionV1::Applied]
    );
    assert!(retired.archive_retention_handles.is_empty());
    assert_eq!(
        archive.retention.borrow()[handle].phase,
        ResourceArchiveRetentionPhaseV1::Committed
    );

    let snapshot = canwu.snapshot_json().expect("snapshot");
    from_resource_snapshot_json_with_archive_store(&snapshot, &[&plugin], Rc::new(archive.clone()))
        .expect("restore after the duplicate commit");
    let replayed = replay_resource_from_journal_with_archive_store(
        &[&plugin],
        &canwu.replay_journal(),
        Rc::new(archive.clone()),
    )
    .expect("exact replay settles the duplicate commit the same way");
    assert_eq!(replayed.snapshot_json().expect("replay snapshot"), snapshot);
    assert_eq!(replayed.boundary_head_hash(), canwu.boundary_head_hash());
}

#[test]
fn an_archive_commit_settles_ahead_of_an_operation_queued_before_it() {
    let mut limits = ResourceLimitsV1::canonical();
    limits.max_archive_candidates = 4;
    let mut resource = ResourceState::empty(limits).expect("resource state");
    for index in 0..4 {
        reject(&mut resource, index);
    }
    let plugin = ResourcePlugin::default();
    let archive = MemoryArchive::default();
    let mut canwu = runtime(&plugin, resource, &archive);

    // The budget is full, so the operation does not fit: it settles nothing,
    // its boundary commits, and a skip event names it and says that an
    // archive commit makes room.
    let full = state(&canwu);
    let blocked = allocation(&canwu, "fixture:allocate:blocked");
    let at = canwu.time();
    let skipped = enqueue_resource_allocation(&mut canwu, at, &holder(), &blocked)
        .expect("enqueue allocation");
    settle(&mut canwu);
    assert_eq!(state(&canwu), full);
    assert_eq!(
        canwu
            .events()
            .iter()
            .filter_map(ResourceIngressSkipV1::from_event)
            .collect::<Vec<_>>(),
        [ResourceIngressSkipV1 {
            ingress: skipped.ingress_id,
            reason: ResourceIngressSkipReasonV1::ArchiveBacklog,
            operation_key: Some(blocked.operation_key.clone()),
            deferred_until: None,
        }]
    );

    // The operation is sent again, and the commit that makes room is queued
    // behind it. The commit settles first, against the root it was prepared
    // from, and the operation after it in the same boundary.
    let commit = full
        .prepare_resource_archive(4, &archive)
        .expect("commit of every candidate");
    enqueue_resource_allocation(&mut canwu, at, &holder(), &blocked).expect("enqueue allocation");
    enqueue_resource_archive(&mut canwu, &commit, &archive).expect("enqueue");
    settle(&mut canwu);
    let settled = state(&canwu);
    assert_eq!(settled.archive_head.revision, 1);
    assert_eq!(settled.archive_head.archived_record_count, 4);
    assert_eq!(
        settled
            .archive_maintenance_receipts
            .values()
            .map(|receipt| receipt.disposition)
            .collect::<Vec<_>>(),
        [ResourceArchiveMaintenanceDispositionV1::Applied]
    );
    assert_eq!(
        settled.outcomes.keys().collect::<Vec<_>>(),
        [&blocked.operation_key]
    );
    // The commit's receipt and the operation's outcome.
    assert_eq!(settled.terminal_archive_candidates.len(), 2);

    let snapshot = canwu.snapshot_json().expect("snapshot");
    from_resource_snapshot_json_with_archive_store(&snapshot, &[&plugin], Rc::new(archive.clone()))
        .expect("restore after the reordered boundary");
    let replayed = replay_resource_from_journal_with_archive_store(
        &[&plugin],
        &canwu.replay_journal(),
        Rc::new(archive.clone()),
    )
    .expect("exact replay settles the commit ahead of the operation too");
    assert_eq!(replayed.snapshot_json().expect("replay snapshot"), snapshot);
    assert_eq!(replayed.boundary_head_hash(), canwu.boundary_head_hash());
}
