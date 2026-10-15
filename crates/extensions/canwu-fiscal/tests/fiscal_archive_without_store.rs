//! No boundary reads the fiscal archive store.
//!
//! Every boundary decision is made from the hot fiscal state: partition
//! uniqueness from the watermarks, idempotency from the outcome rings, and an
//! archive commit from the books it binds, rebuilt from the state. A run whose
//! attached package archive store fails every read therefore settles every
//! boundary exactly as a run whose store works: closing, archiving, settling
//! receipts, and publishing reports alike. Only the host's own archive
//! operations read a store, and they read the one the host passes them.

mod support;

use canwu_api::{
    BoundaryReceipt, Canwu, CanwuError, ErrorCode, PersonId, PluginArchiveObjectProvider,
};
use canwu_fiscal::{FiscalArchiveStore, enqueue_execution_receipts};
use std::rc::Rc;
use support::{
    Draft, Fixture, MemoryArchiveStore, PRINCIPAL, TREASURY_AUTHORITY, archive, assessment, close,
    collection, execution_claim, fiscal_report, fiscal_state, receipt_packet, record_results,
    scenario, settle, submit, treasury_scope,
};

/// The package archive store a run consults, if it ever does: one that
/// serves the host's fiscal store, or one that fails every read.
struct Provider(Option<Rc<MemoryArchiveStore>>);

impl PluginArchiveObjectProvider for Provider {
    fn load_plugin_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, CanwuError> {
        match &self.0 {
            Some(store) => Ok(store.load_fiscal_archive_object(namespace, object_id)?),
            None => Err(CanwuError::new(
                ErrorCode::InvalidArchive,
                "the archive store is unavailable",
            )),
        }
    }
}

fn by_principal(drafts: impl IntoIterator<Item = Draft>) -> Vec<(PersonId, Draft)> {
    drafts.into_iter().map(|draft| (PRINCIPAL, draft)).collect()
}

/// Runs two accounting cycles of two scopes through assessment, collection,
/// receipt, close, and archive into `store`, with reports published along
/// the way, and returns the receipts of every boundary.
fn run(canwu: &mut Canwu, store: &MemoryArchiveStore) -> Vec<BoundaryReceipt> {
    let mut receipts = Vec::new();
    for cycle in ["2026", "2027"] {
        for index in 0..2 {
            let scope = treasury_scope(index);
            let prefix = format!("{cycle}.{index}");
            receipts.extend(submit(
                canwu,
                by_principal([assessment(
                    &format!("{prefix}.assess"),
                    TREASURY_AUTHORITY,
                    &scope,
                    cycle,
                )]),
            ));
            let request = format!("{prefix}.request");
            receipts.extend(submit(
                canwu,
                by_principal([collection(
                    &format!("{prefix}.authorize"),
                    &scope,
                    &format!("{prefix}.assess.assessment"),
                    &request,
                    50,
                )]),
            ));
            let evidence = record_results(
                canwu,
                &[(
                    false,
                    execution_claim(&format!("{prefix}.result"), &scope, &request, 50),
                )],
            );
            let now = canwu.time();
            enqueue_execution_receipts(
                canwu,
                now,
                &[receipt_packet(
                    &format!("{prefix}.receipt"),
                    &scope,
                    &request,
                    evidence,
                )],
            )
            .expect("receipt");
            receipts.extend(settle(canwu));
            receipts.extend(submit(
                canwu,
                by_principal([close(&format!("{prefix}.close"), &scope, cycle)]),
            ));
        }
        archive(canwu, store, 64);
    }
    receipts
}

#[test]
fn every_boundary_settles_alike_whether_or_not_the_archive_store_can_be_read() {
    let fixture = Fixture::new();
    let working_store = Rc::new(MemoryArchiveStore::default());
    let mut working = fixture.runtime(scenario(|_| {}));
    working.set_plugin_archive_object_provider(Rc::new(Provider(Some(working_store.clone()))));
    let working_receipts = run(&mut working, &working_store);

    let failing_store = MemoryArchiveStore::default();
    let mut failing = fixture.runtime(scenario(|_| {}));
    failing.set_plugin_archive_object_provider(Rc::new(Provider(None)));
    let failing_receipts = run(&mut failing, &failing_store);

    assert_eq!(failing_receipts, working_receipts);
    assert_eq!(failing.checkpoint_hash(), working.checkpoint_hash());
    assert_eq!(
        failing.snapshot_json().expect("snapshot"),
        working.snapshot_json().expect("snapshot")
    );
    let state = fiscal_state(&failing);
    assert_eq!(state.archive.archived_book_count, 4);
    assert!(state.books().next().is_none());
    assert_eq!(
        fiscal_state(&working).archive.directory_root,
        state.archive.directory_root
    );
    assert!(
        fiscal_report(&failing, PRINCIPAL)
            .projection
            .facts
            .is_empty()
    );

    // A restore and an exact replay need no store either.
    let mut restored = Canwu::from_snapshot_json_with_plugins(
        &failing.snapshot_json().expect("snapshot"),
        &fixture.plugins(),
    )
    .expect("restored without a store");
    restored.set_plugin_archive_object_provider(Rc::new(Provider(None)));
    assert_eq!(restored.checkpoint_hash(), failing.checkpoint_hash());
    let replayed = Canwu::replay_from_journal(&fixture.plugins(), &failing.replay_journal())
        .expect("replayed without a store");
    assert_eq!(replayed.checkpoint_hash(), failing.checkpoint_hash());
    // The run goes on without one: a new cycle settles and publishes.
    let receipts = submit(
        &mut restored,
        by_principal([assessment(
            "2028.0.assess",
            TREASURY_AUTHORITY,
            &treasury_scope(0),
            "2028",
        )]),
    );
    assert!(
        receipts
            .iter()
            .any(|receipt| receipt.knowledge_record_count > 0)
    );
}
