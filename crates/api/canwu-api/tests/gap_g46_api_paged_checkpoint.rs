//! Gap G-46: a host writes and restores the paged checkpoint through the
//! public API.
//!
//! The host persists one evidence segment per boundary, prepares a paged
//! checkpoint into a page store, and restores it with the contiguous segment
//! prefix and its plugins. The restored run equals the live run and continues
//! identically. A later checkpoint against the same store emits only pages
//! the store lacks, and the run restored from the earlier checkpoint reaches
//! the same root. A missing inner page or a short evidence prefix fails
//! closed. A compact host restores from its sealed segments plus the retained
//! tail and continues identically; the restored runtime's next seal replaces
//! the earlier sealed segment, and that replacement alone restores the next
//! checkpoint.

#![allow(clippy::too_many_lines)]

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundaryRequest,
    BoundarySystemContract, Canwu, CanwuError, CompactedCanwu, DomainRecordClass,
    DomainRecordDraft, DomainRecordKind, DomainRecordMutation, DomainRecordRef, DomainRecordSchema,
    ErrorCode, EvidenceCursor, EvidenceJournalSegment, PluginRegistrar, Scenario, SimDuration,
    SimTime, SimulationPlugin, SimulationView, StateKey, StatePageBlob, StatePageProvider,
    StateVisibility, SystemCadence,
};
use serde_json::json;
use std::cell::RefCell;
use std::collections::BTreeMap;

const LEDGER: &str = "fixture-ledger";
const ACCOUNTS: u64 = 128;

fn account_kind() -> DomainRecordKind {
    DomainRecordKind::new(LEDGER, "account")
}

fn account(ordinal: u64) -> DomainRecordRef {
    DomainRecordRef {
        kind: account_kind(),
        id: format!("account-{ordinal:03}"),
    }
}

fn ledger_state() -> StateKey {
    DomainRecordSchema::new(account_kind(), DomainRecordClass::Record).state_key()
}

/// Opens every account at boundary 1, then posts to one account per boundary.
fn keep_ledger(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let boundary = context.boundary_id.get();
    let directives = if boundary == 1 {
        (0..ACCOUNTS)
            .map(|ordinal| BoundaryDirective::MutateRecord {
                mutation: DomainRecordMutation::Create {
                    record: DomainRecordDraft::new(account(ordinal), json!({ "balance": 0 })),
                },
                summary: format!("Opened account {ordinal}"),
            })
            .collect()
    } else {
        let reference = account((boundary * 7) % ACCOUNTS);
        let current = view
            .domain_record(&reference)?
            .expect("accounts exist after the opening boundary")
            .version;
        vec![BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Update {
                record: DomainRecordDraft::new(reference, json!({ "balance": boundary })),
                expected_version: current,
            },
            summary: format!("Posted boundary {boundary}"),
        }]
    };
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

struct LedgerPlugin;

impl SimulationPlugin for LedgerPlugin {
    fn name(&self) -> &'static str {
        LEDGER
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000004601"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_record_schema(DomainRecordSchema::new(
            account_kind(),
            DomainRecordClass::Record,
        ))?;
        let mut contract = BoundarySystemContract::new(
            "keep-ledger",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        contract.reads = vec![ledger_state()];
        contract.writes = vec![ledger_state()];
        contract.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(contract, keep_ledger)
    }
}

/// A content-addressed page store, as a host backs it.
#[derive(Default)]
struct PageStore(RefCell<BTreeMap<String, StatePageBlob>>);

impl StatePageProvider for PageStore {
    fn load_state_page(&self, page_id: &str) -> Result<Option<StatePageBlob>, CanwuError> {
        Ok(self.0.borrow().get(page_id).cloned())
    }
}

impl canwu_api::StatePageStore for PageStore {
    fn store_state_page(
        &self,
        page: &StatePageBlob,
    ) -> Result<canwu_api::ArchiveStoreOutcome, CanwuError> {
        page.validate()?;
        let mut pages = self.0.borrow_mut();
        if let Some(existing) = pages.get(&page.page_id) {
            assert_eq!(existing, page, "one page ID must name one byte string");
            return Ok(canwu_api::ArchiveStoreOutcome::AlreadyPresent);
        }
        pages.insert(page.page_id.clone(), page.clone());
        Ok(canwu_api::ArchiveStoreOutcome::Stored)
    }
}

fn daily(day: i64) -> BoundaryRequest {
    BoundaryRequest::at(SimTime::EPOCH + SimDuration::days(day)).with_cadence(SystemCadence::Daily)
}

/// The host's delta journal: one evidence segment per settled boundary.
struct Host {
    canwu: Canwu,
    cursor: EvidenceCursor,
    segments: Vec<EvidenceJournalSegment>,
}

impl Host {
    fn settle(&mut self, day: i64) {
        self.canwu
            .settle_boundary(daily(day))
            .expect("ledger boundary settles");
        self.segments.push(
            self.canwu
                .journal_segment_since(self.cursor)
                .expect("boundary evidence segment"),
        );
        self.cursor = self.canwu.evidence_cursor().expect("evidence cursor");
    }
}

fn assert_same_run(restored: &Canwu, live: &Canwu) {
    assert_eq!(restored.checkpoint_hash(), live.checkpoint_hash());
    assert_eq!(
        restored
            .authoritative_state_hash()
            .expect("restored state hash"),
        live.authoritative_state_hash().expect("live state hash")
    );
    assert_eq!(restored.revision(), live.revision());
    assert_eq!(restored.time(), live.time());
    assert_eq!(restored.boundary_head_hash(), live.boundary_head_hash());
    assert_eq!(
        restored.evidence_cursor().expect("restored cursor"),
        live.evidence_cursor().expect("live cursor")
    );
}

#[test]
fn paged_checkpoint_round_trips_through_the_public_api_and_stores_only_changed_pages() {
    paged_round_trip();
}

/// The same round trip on a 1 MiB thread, the Windows main-thread default:
/// a debug build must keep room on the stack for the runtime and its
/// transaction checkpoints, which hold the commitment cache by box.
#[test]
fn the_paged_round_trip_fits_a_one_mebibyte_stack() {
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(paged_round_trip)
        .expect("the thread starts")
        .join()
        .expect("the round trip runs on a 1 MiB stack");
}

fn paged_round_trip() {
    let plugin = LedgerPlugin;
    let mut live = Host {
        canwu: Canwu::new_with_plugins(4646, Scenario::new(SimTime::EPOCH, Vec::new()), &[&plugin])
            .expect("ledger run"),
        cursor: EvidenceCursor::default(),
        segments: Vec::new(),
    };
    for day in 1..=6 {
        live.settle(day);
    }

    let store = PageStore::default();
    let checkpoint_before = live.canwu.checkpoint_hash().to_owned();
    let revision_before = live.canwu.revision();
    let first = live
        .canwu
        .prepare_paged_checkpoint(None, &store)
        .expect("prepare the first paged checkpoint");
    assert_eq!(live.canwu.checkpoint_hash(), checkpoint_before);
    assert_eq!(live.canwu.revision(), revision_before);
    assert_eq!(first.checkpoint.checkpoint_hash, checkpoint_before);
    assert_eq!(first.delta.target_root, first.checkpoint.root_page_id);
    assert!(!first.delta.new_pages.is_empty());
    first
        .store_and_verify(&store)
        .expect("store the first paged checkpoint");

    let restored = Canwu::from_paged_checkpoint_and_journal_with_plugins(
        &first.checkpoint,
        &store,
        live.segments.clone(),
        &[&plugin],
    )
    .expect("restore the first paged checkpoint");
    assert_same_run(&restored, &live.canwu);
    assert_eq!(
        restored.domain_record(&account(42)),
        live.canwu.domain_record(&account(42))
    );

    let short_prefix = live.segments[..live.segments.len() - 1].to_vec();
    let short = Canwu::from_paged_checkpoint_and_journal_with_plugins(
        &first.checkpoint,
        &store,
        short_prefix,
        &[&plugin],
    )
    .err()
    .expect("a prefix that stops short of the checkpoint cut is rejected");
    assert_eq!(short.code, ErrorCode::InvalidSnapshot);

    // The restored run continues exactly as the live run does.
    let mut resumed = Host {
        canwu: restored,
        cursor: live.cursor,
        segments: live.segments.clone(),
    };
    for day in 7..=9 {
        live.settle(day);
        resumed.settle(day);
    }
    assert_same_run(&resumed.canwu, &live.canwu);
    assert_eq!(resumed.segments, live.segments);

    // The store already holds the first checkpoint's pages.
    let second = live
        .canwu
        .prepare_paged_checkpoint(Some(&first.checkpoint), &store)
        .expect("prepare the second paged checkpoint");
    assert_eq!(second.delta.source_root, first.checkpoint.root_page_id);
    assert!(!second.delta.new_pages.is_empty());
    assert!(
        second.delta.new_pages.len() < first.delta.new_pages.len(),
        "the second delta holds {} of the first delta's {} pages",
        second.delta.new_pages.len(),
        first.delta.new_pages.len()
    );
    for page in &second.delta.new_pages {
        assert!(
            store
                .load_state_page(&page.page_id)
                .expect("load")
                .is_none(),
            "the delta repeats a page the store already holds"
        );
    }
    // A store that kept the second root but lost the rest of its delta fails
    // closed at the first missing inner page; no earlier page stands in.
    let torn = PageStore::default();
    for page in first.delta.new_pages.iter().chain(
        second
            .delta
            .new_pages
            .iter()
            .filter(|page| page.page_id == second.checkpoint.root_page_id),
    ) {
        torn.0
            .borrow_mut()
            .insert(page.page_id.clone(), page.clone());
    }
    let missing = Canwu::from_paged_checkpoint_and_journal_with_plugins(
        &second.checkpoint,
        &torn,
        live.segments.clone(),
        &[&plugin],
    )
    .err()
    .expect("a checkpoint whose inner pages are missing is rejected");
    assert_eq!(missing.code, ErrorCode::StatePageUnavailable);
    second
        .store_and_verify(&store)
        .expect("store the second paged checkpoint");

    let restored_again = Canwu::from_paged_checkpoint_and_journal_with_plugins(
        &second.checkpoint,
        &store,
        live.segments.clone(),
        &[&plugin],
    )
    .expect("restore the second paged checkpoint");
    assert_same_run(&restored_again, &live.canwu);

    // The resumed run's state is the live run's, so its checkpoint names the
    // same root and has nothing left to store.
    let resumed_checkpoint = resumed
        .canwu
        .prepare_paged_checkpoint(Some(&first.checkpoint), &store)
        .expect("prepare the resumed run's paged checkpoint");
    assert_eq!(resumed_checkpoint.checkpoint, second.checkpoint);
    assert!(resumed_checkpoint.delta.new_pages.is_empty());

    // A compact host seals at a boundary where the ledger is quiet, settles
    // on, and checkpoints with part of its evidence still retained.
    let mut compact = live
        .canwu
        .fork()
        .into_compacted()
        .expect("enter the compact interface");
    compact
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH + SimDuration::days(10)))
        .expect("quiet boundary admits the last ledger events");
    let sealed = compact
        .seal_evidence()
        .expect("seal the settled prefix")
        .expect("the prefix is sealable");
    for day in 11..=12 {
        compact
            .settle_boundary(daily(day))
            .expect("compact boundary settles");
    }
    let compact_store = PageStore::default();
    let compact_checkpoint = compact
        .prepare_paged_checkpoint(None, &compact_store)
        .expect("prepare the compact paged checkpoint");
    assert_eq!(
        compact_checkpoint.checkpoint.checkpoint_hash,
        compact.checkpoint_hash()
    );
    compact_checkpoint
        .store_and_verify(&compact_store)
        .expect("store the compact paged checkpoint");
    let tail = compact
        .journal_segment_since(sealed.end)
        .expect("retained evidence after the seal");
    let sealed_only = CompactedCanwu::from_paged_checkpoint_and_journal_with_plugins(
        &compact_checkpoint.checkpoint,
        &compact_store,
        vec![sealed.clone()],
        &[&plugin],
    )
    .err()
    .expect("the sealed segments alone stop short of the checkpoint cut");
    assert_eq!(sealed_only.code, ErrorCode::InvalidSnapshot);
    let mut compact_restored = CompactedCanwu::from_paged_checkpoint_and_journal_with_plugins(
        &compact_checkpoint.checkpoint,
        &compact_store,
        vec![sealed.clone(), tail],
        &[&plugin],
    )
    .expect("restore the compact paged checkpoint");
    assert_eq!(
        compact_restored.checkpoint_hash(),
        compact.checkpoint_hash()
    );
    assert_eq!(
        compact_restored.evidence_cursor().expect("restored cursor"),
        compact.evidence_cursor().expect("compact cursor")
    );
    compact_restored
        .settle_boundary(daily(13))
        .expect("restored compact boundary settles");
    compact
        .settle_boundary(daily(13))
        .expect("compact boundary settles");
    assert_eq!(
        compact_restored.checkpoint_hash(),
        compact.checkpoint_hash()
    );
    assert_eq!(
        compact_restored.boundary_head_hash(),
        compact.boundary_head_hash()
    );

    // The restored runtime retains its whole prefix, so its next seal starts
    // at the zero cut and replaces the earlier sealed segment: appending the
    // replacement no longer forms a prefix, and the replacement alone restores
    // the next checkpoint.
    compact_restored
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH + SimDuration::days(14)))
        .expect("quiet boundary admits the last ledger events");
    let replacement = compact_restored
        .seal_evidence()
        .expect("seal the restored prefix")
        .expect("the restored prefix is sealable");
    assert_eq!(replacement.start, EvidenceCursor::default());
    let resealed = compact_restored
        .prepare_paged_checkpoint(Some(&compact_checkpoint.checkpoint), &compact_store)
        .expect("prepare the resealed paged checkpoint");
    resealed
        .store_and_verify(&compact_store)
        .expect("store the resealed paged checkpoint");
    let appended = CompactedCanwu::from_paged_checkpoint_and_journal_with_plugins(
        &resealed.checkpoint,
        &compact_store,
        vec![sealed, replacement.clone()],
        &[&plugin],
    )
    .err()
    .expect("the earlier sealed segment overlaps its replacement");
    assert_eq!(appended.code, ErrorCode::InvalidSnapshot);
    let mut restarted = CompactedCanwu::from_paged_checkpoint_and_journal_with_plugins(
        &resealed.checkpoint,
        &compact_store,
        vec![replacement],
        &[&plugin],
    )
    .expect("the replacement alone restores the resealed checkpoint");
    restarted
        .settle_boundary(daily(15))
        .expect("restarted boundary settles");
    compact_restored
        .settle_boundary(daily(15))
        .expect("resealed boundary settles");
    assert_eq!(
        restarted.checkpoint_hash(),
        compact_restored.checkpoint_hash()
    );
}
