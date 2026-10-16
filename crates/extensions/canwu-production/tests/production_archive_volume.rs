//! Production archive volume, lifetime, and lookup.
//!
//! One archive commit carries several pages, and every record of every page
//! resolves through the cumulative lookup index, which a restore audits; a
//! missing lookup page is an error, never a miss. A run may make any number
//! of commits: the receipt of an applied commit is archived by the next one,
//! so the root holds one receipt and one handle however long the chain, and
//! the execution archived by the first commit and every archived receipt
//! still resolve. (`production-archive-profile --preset full` runs the chain
//! past the 8,192 commits at which 0.15.0 refused every later preparation.)
//!
//! A stale commit terminates. The third fixture fills the budget of
//! unacknowledged applied commits and queues two commits from one root: the
//! first applies, the second is stale and settles as a no-op instead of
//! failing the boundary. Its receipt is not an archive candidate, and the
//! acknowledgement of its handle retires it, also when it arrives twice. The
//! fourth queues one batch twice. The fifth queues a commit behind an
//! operation admitted before it: the commit settles first and applies.
//!
//! Until the archive removes it, hot work declares the record versions
//! restore reads for it, but an ended execution no longer declares its
//! inputs' consumer evidence, which restore compares by reference only.

#![allow(clippy::trivially_copy_pass_by_ref)]

mod support;

use canwu_production::*;
use std::sync::Arc;
use support::*;

/// Settled executions of the multi-page fixture: one full page and a part.
const MULTI_PAGE_RECORDS: usize = MAX_ARCHIVE_PAGE_ENTRIES + 6;
/// Archive commits of the chain fixture.
const CHAIN_COMMITS: u64 = 6;

fn execution_key(id: &ProductionExecutionId) -> ProductionTerminalArchiveKeyV1 {
    ProductionTerminalArchiveKeyV1::Execution(id.clone())
}

fn applied_receipts(state: &ProductionState) -> usize {
    state
        .archive
        .maintenance_receipts
        .values()
        .filter(|receipt| receipt.disposition == ProductionArchiveRetentionPhaseV1::Committed)
        .count()
}

fn stale_receipts(state: &ProductionState) -> usize {
    state.archive.maintenance_receipts.len() - applied_receipts(state)
}

/// Prepares and queues a commit of every terminal candidate.
fn commit_everything(
    canwu: &mut canwu_api::Canwu,
    archive: &TestArchive,
) -> ProductionArchiveIngressReceiptV1 {
    let prepared = production_state(canwu)
        .prepare_production_archive(MAX_ARCHIVE_PREPARE_CANDIDATES, archive)
        .expect("prepare archive commit");
    enqueue_production_archive(canwu, &prepared, archive).expect("enqueue archive commit")
}

/// Restores the run from its snapshot and replays its journal, both through
/// the checked archive wrappers, and requires both to equal it.
fn restore_and_replay(plugin: &ProductionPlugin, canwu: &canwu_api::Canwu, archive: &TestArchive) {
    let snapshot = canwu.snapshot_json().expect("snapshot");
    let restored = from_production_snapshot_json_with_archives(
        &snapshot,
        &[plugin],
        Arc::new(archive.clone()),
        Arc::new(archive.clone()),
    )
    .expect("restore audits the archive chain and its lookup index");
    assert_eq!(
        restored.snapshot_json().expect("restored snapshot"),
        snapshot
    );
    let replayed = replay_production_from_journal_with_archives(
        &[plugin],
        &canwu.replay_journal(),
        Arc::new(archive.clone()),
        Arc::new(archive.clone()),
    )
    .expect("exact replay");
    assert_eq!(
        replayed.snapshot_json().expect("replayed snapshot"),
        snapshot
    );
}

#[test]
fn one_archive_commit_carries_several_pages_and_every_record_resolves() {
    let plugin = ProductionPlugin;
    let archive = TestArchive::default();
    let state = settled_state(MULTI_PAGE_RECORDS);
    let executions = state.archive_due_index.iter().cloned().collect::<Vec<_>>();
    assert_eq!(executions.len(), MULTI_PAGE_RECORDS);
    let mut canwu = runtime(&plugin, state, &archive);

    let prepared = production_state(&canwu)
        .prepare_production_archive(MAX_ARCHIVE_PREPARE_CANDIDATES, &archive)
        .expect("one commit of every candidate");
    assert_eq!(prepared.selected, executions);
    assert_eq!(prepared.directory.blob_ids.len(), 2);
    assert_eq!(prepared.additional_pages.len(), 1);
    assert_eq!(prepared.blob.records.len(), MAX_ARCHIVE_PAGE_ENTRIES);
    enqueue_production_archive(&mut canwu, &prepared, &archive).expect("enqueue");
    settle(&mut canwu);

    let archived = production_state(&canwu);
    assert!(archived.executions.is_empty());
    assert!(archived.archive_due_index.is_empty());
    assert_eq!(
        archived.archive.archived_execution_count,
        MULTI_PAGE_RECORDS as u64
    );
    assert_eq!(
        archived.archive.directory_root.as_deref(),
        Some(prepared.directory.id.as_str())
    );
    for id in &executions {
        match archived_production_terminal_record(&archived, &archive, &execution_key(id))
            .expect("authenticated lookup")
        {
            Some(ProductionArchivedRecordV1::Execution(record)) => {
                assert_eq!(&record.execution_record.id, id);
                assert_eq!(record.lifecycle, WorkOrderLifecycle::Settled);
            }
            other => panic!("{id} did not resolve to its execution record: {other:?}"),
        }
    }
    let never = ProductionExecutionId::new("production:execution:never-archived").expect("ID");
    assert_eq!(
        archived_production_terminal_record(&archived, &archive, &execution_key(&never))
            .expect("a miss reads authenticated pages only"),
        None
    );
    restore_and_replay(&plugin, &canwu, &archive);

    // Every lookup page the head reaches is live. Without one of them a
    // lookup that needs it fails instead of answering a miss, and the
    // restore audit refuses the archive.
    let live =
        live_production_archive_lookup_pages(&archived, &archive, []).expect("live lookup pages");
    assert_eq!(live, archive.ids(PRODUCTION_ARCHIVE_LOOKUP_PAGE_NAMESPACE));
    let root_page = prepared.directory.lookup_root.clone();
    let bytes = archive
        .remove(PRODUCTION_ARCHIVE_LOOKUP_PAGE_NAMESPACE, &root_page)
        .expect("stored lookup root");
    assert!(
        archived_production_terminal_record(&archived, &archive, &execution_key(&never)).is_err()
    );
    assert!(validate_production_archive(&archive, &archived).is_err());
    archive.put(PRODUCTION_ARCHIVE_LOOKUP_PAGE_NAMESPACE, &root_page, bytes);
    validate_production_archive(&archive, &archived).expect("the restored page authenticates");
}

#[test]
fn a_chain_of_commits_keeps_one_receipt_and_resolves_every_archived_record() {
    let plugin = ProductionPlugin;
    let archive = TestArchive::default();
    let state = settled_state(1);
    let first = state
        .archive_due_index
        .first()
        .cloned()
        .expect("settled execution");
    let mut canwu = runtime(&plugin, state, &archive);

    // The first commit archives the execution. Every later commit archives
    // the maintenance receipt of the commit before it, so the chain grows by
    // one commit per boundary, and each commit is acknowledged in the
    // boundary of the next.
    let mut unacknowledged = None;
    for _ in 0..CHAIN_COMMITS {
        if let Some(receipt) = unacknowledged.take() {
            finalize_production_archive_retention(&mut canwu, &archive, &receipt)
                .expect("finalize the previous commit");
        }
        unacknowledged = Some(commit_everything(&mut canwu, &archive));
        settle(&mut canwu);
    }
    let archived = production_state(&canwu);
    assert_eq!(archived.archive.committed_batch_count, CHAIN_COMMITS);
    assert_eq!(archived.archive.archived_execution_count, 1);
    assert_eq!(archived.archive.archived_receipt_count, CHAIN_COMMITS - 1);
    // The root holds the receipt and the handle of the last commit only.
    assert_eq!(archived.archive.maintenance_receipts.len(), 1);
    assert_eq!(archived.archive.pending_handles.len(), 1);
    assert_eq!(archived.archive.receipt_sequence, CHAIN_COMMITS);

    match archived_production_terminal_record(&archived, &archive, &execution_key(&first))
        .expect("authenticated lookup")
    {
        Some(ProductionArchivedRecordV1::Execution(record)) => {
            assert_eq!(record.execution_record.id, first);
        }
        other => panic!("the first execution did not resolve: {other:?}"),
    }
    // Every receipt but the head's was archived by the commit after it, and
    // names the commit before that one.
    for sequence in 1..CHAIN_COMMITS {
        let receipt =
            ProductionArchiveReceiptId::new(format!("canwu.production:archive-receipt:{sequence}"))
                .expect("receipt ID");
        match archived_production_terminal_record(
            &archived,
            &archive,
            &ProductionTerminalArchiveKeyV1::ArchiveMaintenanceReceipt(receipt.clone()),
        )
        .expect("authenticated lookup")
        {
            Some(ProductionArchivedRecordV1::ArchiveMaintenanceReceipt(found)) => {
                assert_eq!(found.id, receipt);
                assert_eq!(
                    found.disposition,
                    ProductionArchiveRetentionPhaseV1::Committed
                );
                assert_eq!(
                    (found.archived_executions, found.archived_receipts),
                    (usize::from(sequence == 1), usize::from(sequence > 1))
                );
            }
            other => panic!("receipt {sequence} did not resolve: {other:?}"),
        }
    }
    restore_and_replay(&plugin, &canwu, &archive);
}

#[test]
fn a_stale_archive_commit_terminates_when_the_applied_handles_are_full() {
    let plugin = ProductionPlugin;
    let archive = TestArchive::default();
    let state = settled_state(2);
    let mut canwu = runtime(&plugin, state, &archive);
    let limit = ProductionLimitsV1::canonical().max_pending_retention_handles;

    // Fill the budget of unacknowledged applied commits but one: the first
    // commit archives one execution, each later one the receipt before it.
    let mut queued = Vec::new();
    for _ in 0..limit - 1 {
        let prepared = production_state(&canwu)
            .prepare_production_archive(1, &archive)
            .expect("prepare a one-record commit");
        queued.push(enqueue_production_archive(&mut canwu, &prepared, &archive).expect("enqueue"));
        settle(&mut canwu);
    }
    let full = production_state(&canwu);
    assert_eq!(full.archive.pending_handles.len(), limit - 1);
    assert_eq!(full.archive_due_index.len(), 1);

    // Two commits prepared from the same root and queued together: the
    // first applies and fills the budget, the second is stale.
    let first = full
        .prepare_production_archive(1, &archive)
        .expect("first commit");
    let second = full
        .prepare_production_archive(2, &archive)
        .expect("second commit");
    assert_eq!(second.selected.len(), 1);
    enqueue_production_archive(&mut canwu, &first, &archive).expect("enqueue first");
    let stale_queued =
        enqueue_production_archive(&mut canwu, &second, &archive).expect("enqueue second");
    settle(&mut canwu);
    let settled = production_state(&canwu);
    assert_eq!(
        settled.archive.directory_root.as_deref(),
        Some(first.directory.id.as_str())
    );
    assert_eq!(settled.archive.pending_handles.len(), limit + 1);
    assert_eq!(
        settled.archive.pending_handles[&stale_queued.retention_handle_id].phase,
        ProductionArchiveRetentionPhaseV1::RejectedStale
    );
    assert_eq!(
        (applied_receipts(&settled), stale_receipts(&settled)),
        (1, 1)
    );
    // The stale receipt is not a candidate: the next commit would archive
    // the applied receipt and the execution, once a handle is acknowledged.
    assert!(
        settled
            .prepare_production_archive(MAX_ARCHIVE_PREPARE_CANDIDATES, &archive)
            .is_err_and(|error| error.message.contains("archive_backpressure"))
    );

    // The stale commit is acknowledged twice before its boundary; the second
    // acknowledgement finds no handle and settles as a no-op.
    for _ in 0..2 {
        finalize_production_archive_retention(&mut canwu, &archive, &stale_queued)
            .expect("finalize the stale commit");
    }
    settle(&mut canwu);
    let acknowledged = production_state(&canwu);
    assert_eq!(stale_receipts(&acknowledged), 0);
    assert_eq!(acknowledged.archive.pending_handles.len(), limit);
    assert_eq!(
        archive.retention.read().unwrap()[&stale_queued.retention_handle_id],
        ProductionArchiveRetentionPhaseV1::RejectedStale
    );

    // Acknowledging the earlier commits frees the budget, and the next commit
    // archives the head's receipt and the last execution.
    for receipt in &queued {
        finalize_production_archive_retention(&mut canwu, &archive, receipt)
            .expect("finalize an earlier commit");
    }
    settle(&mut canwu);
    let drained = production_state(&canwu);
    assert_eq!(drained.archive.pending_handles.len(), 1);
    let last = drained
        .prepare_production_archive(MAX_ARCHIVE_PREPARE_CANDIDATES, &archive)
        .expect("the budget has room again");
    assert_eq!((last.selected_receipts.len(), last.selected.len()), (1, 1));
    enqueue_production_archive(&mut canwu, &last, &archive).expect("enqueue the last commit");
    settle(&mut canwu);
    let empty = production_state(&canwu);
    assert!(empty.executions.is_empty());
    assert_eq!(empty.archive.archived_execution_count, 2);
    restore_and_replay(&plugin, &canwu, &archive);
}

#[test]
fn the_same_archive_batch_queued_twice_leaves_no_stale_receipt() {
    let plugin = ProductionPlugin;
    let archive = TestArchive::default();
    let mut canwu = runtime(&plugin, settled_state(1), &archive);

    // One batch queued twice settles once as applied and once as stale under
    // one retention handle, which stays the handle of the applied commit.
    let batch = production_state(&canwu)
        .prepare_production_archive(MAX_ARCHIVE_PREPARE_CANDIDATES, &archive)
        .expect("commit of every candidate");
    let queued = [(); 2].map(|()| {
        enqueue_production_archive(&mut canwu, &batch, &archive).expect("enqueue the same batch")
    });
    let handle = &queued[0].retention_handle_id;
    assert_eq!(handle, &queued[1].retention_handle_id);
    settle(&mut canwu);
    let doubled = production_state(&canwu);
    assert_eq!(doubled.archive.committed_batch_count, 1);
    assert_eq!(
        (applied_receipts(&doubled), stale_receipts(&doubled)),
        (1, 1)
    );
    assert_eq!(
        doubled.archive.pending_handles[handle].phase,
        ProductionArchiveRetentionPhaseV1::Committed
    );

    // Its acknowledgement retires the handle and the stale receipt; the
    // applied receipt stays for the next commit to archive.
    finalize_production_archive_retention(&mut canwu, &archive, &queued[1])
        .expect("finalize the batch queued twice");
    settle(&mut canwu);
    let retired = production_state(&canwu);
    assert_eq!(
        (applied_receipts(&retired), stale_receipts(&retired)),
        (1, 0)
    );
    assert!(retired.archive.pending_handles.is_empty());
    assert_eq!(
        archive.retention.read().unwrap()[handle],
        ProductionArchiveRetentionPhaseV1::Committed
    );
    restore_and_replay(&plugin, &canwu, &archive);
}

#[test]
fn an_archive_commit_settles_ahead_of_an_operation_admitted_before_it() {
    let plugin = ProductionPlugin;
    let archive = TestArchive::default();
    let mut canwu = runtime(&plugin, settled_state(1), &archive);

    // An operation that settles as a rejection still advances the runtime
    // revision, which the archive source root binds. Its command boundary
    // queues it, and the commit is queued behind it for the next boundary.
    enqueue_operation(
        &mut canwu,
        1,
        "production:authorize:missing",
        ProductionOperation::AuthorizeWorkOrder {
            work_order: WorkOrderId::new("production:order:missing").expect("order ID"),
        },
    );
    settle(&mut canwu);
    let receipt = commit_everything(&mut canwu, &archive);
    settle(&mut canwu);
    let settled = production_state(&canwu);
    assert!(settled.executions.is_empty());
    assert_eq!(
        settled.archive.pending_handles[&receipt.retention_handle_id].phase,
        ProductionArchiveRetentionPhaseV1::Committed
    );
    let operation = &settled.operation_outcomes
        [&ProductionOperationOutcomeId::new("production:authorize:missing").expect("ID")];
    assert_eq!(
        operation.disposition,
        ProductionOperationDisposition::Rejected
    );
    restore_and_replay(&plugin, &canwu, &archive);
}

/// The record versions `root` declares as payload-required evidence.
fn declared_evidence(root: ProductionState) -> Vec<canwu_api::EvidenceRef> {
    let record = root.into_initial_record().expect("production root");
    serde_json::from_value::<canwu_api::PayloadRequiredEvidenceContinuationV1>(
        record.payload[canwu_api::PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD].clone(),
    )
    .expect("evidence continuation")
    .dependencies
}

/// A run whose root holds a running execution, every evidence version of
/// which is an initial-scenario version, seals before its first boundary and
/// continues as its unsealed fork does. The root declares none of those
/// versions: they stay readable after any seal, and the engine refuses to
/// have them declared.
#[test]
fn a_run_with_running_work_seals_before_its_first_boundary() {
    let plugin = ProductionPlugin;
    let archive = TestArchive::default();
    let mut state = base_state();
    let lease = activate_lease(&mut state, "sealed");
    state
        .apply_operation(
            &command(&state, "production:start:sealed", start_operation(&lease)),
            lease.at,
        )
        .expect("the execution starts");
    let mut unsealed = runtime(&plugin, state, &archive);
    enqueue_operation(
        &mut unsealed,
        1,
        "production:authorize:sealed-missing",
        ProductionOperation::AuthorizeWorkOrder {
            work_order: WorkOrderId::new("production:order:sealed-missing").expect("order ID"),
        },
    );
    let mut sealed = unsealed.fork().into_compacted().expect("compact interface");
    sealed
        .seal_evidence()
        .expect("the run seals before its first boundary")
        .expect("the queued command seals");
    for _ in 0..3 {
        let at = unsealed.time();
        assert_eq!(
            sealed
                .settle_boundary(canwu_api::BoundaryRequest::at(at))
                .expect("sealed boundary"),
            unsealed
                .settle_boundary(canwu_api::BoundaryRequest::at(at))
                .expect("unsealed boundary")
        );
        assert_eq!(sealed.checkpoint_hash(), unsealed.checkpoint_hash());
    }
}

/// Rewrites every execution input's consumer evidence, and its work in
/// progress's copy, to `version`.
fn with_consumer_evidence(
    mut state: ProductionState,
    version: &canwu_api::DomainRecordVersionRef,
) -> ProductionState {
    for execution in state.executions.values_mut() {
        for input in &mut execution.inputs {
            input.consumption.consumer_evidence = version.clone();
        }
    }
    for wip in state.work_in_progress.values_mut() {
        for input in &mut wip.consumed_input_evidence {
            input.consumption.consumer_evidence = version.clone();
        }
    }
    state
}

#[test]
fn only_unfinished_executions_declare_their_input_consumer_evidence() {
    let mut running = base_state();
    let lease = activate_lease(&mut running, "running");
    running
        .apply_operation(
            &command(
                &running,
                "production:start:running",
                start_operation(&lease),
            ),
            lease.at,
        )
        .expect("the execution starts");
    // The fixture's consumer evidence is an initial-scenario version, which
    // stays readable after any seal: not even a running execution declares
    // it.
    let initial = lease.input.consumption.consumer_evidence.clone();
    assert_eq!(
        initial.established_by,
        canwu_api::DomainRecordVersionSource::InitialScenario
    );
    assert!(!declared_evidence(running.clone()).contains(
        &canwu_api::EvidenceRef::DomainRecordVersion(initial.clone())
    ));

    // A running execution declares a consumer version a boundary
    // established.
    let established = canwu_api::DomainRecordVersionRef {
        established_by: canwu_api::DomainRecordVersionSource::BoundaryChange {
            boundary: canwu_api::BoundaryId::new(3),
            change_index: 0,
        },
        ..initial
    };
    let consumer = canwu_api::EvidenceRef::DomainRecordVersion(established.clone());
    assert!(declared_evidence(with_consumer_evidence(running, &established)).contains(&consumer));

    // A settled one, whose consumer evidence restore compares by reference
    // only, does not, so a seal may drop that body.
    let settled = with_consumer_evidence(settled_state(1), &established);
    let execution = settled
        .executions
        .values()
        .next()
        .expect("settled execution");
    assert_eq!(execution.lifecycle, WorkOrderLifecycle::Settled);
    assert_eq!(
        canwu_api::EvidenceRef::DomainRecordVersion(
            execution.inputs[0].consumption.consumer_evidence.clone()
        ),
        consumer
    );
    assert!(!declared_evidence(settled).contains(&consumer));
}
