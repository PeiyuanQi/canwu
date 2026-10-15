//! The reference integration runs a century with an archive cadence.
//!
//! Every simulated year runs one sample cycle (assessment, authorization,
//! execution evidence, receipt) in a cycle of its own and closes it; every
//! ten years the closed books are archived. The fiscal records stay bounded
//! by one decade of books, and their size over the run is printed
//! (`--nocapture`) and recorded in the world-scale fiscal proposal, with the
//! snapshot's, which grows with the run's retained history (sealing bounds
//! that, and is not this test's subject), and the archive's.
//! Restore and exact replay of runs that archive are held by the fiscal
//! crate's own tests.

use canwu_api::{Canwu, SimDuration, SimTime};
use canwu_fiscal::{
    FiscalArchiveError, FiscalArchiveKeyV1, FiscalArchiveRetentionHandleV1,
    FiscalArchiveRetentionPhaseV1, FiscalArchiveStore, FiscalArchivedRecordV1,
    MAX_FISCAL_LEDGER_RECORD_BYTES, archived_fiscal_record, audit_fiscal_archive,
    audit_fiscal_records,
};
use canwu_ming_fiscal_reference::{
    DEFAULT_SEED, archive_ming_fiscal_books, close_ming_fiscal_sample_cycle,
    new_ming_fiscal_reference, run_ming_fiscal_sample_cycle,
};
use std::cell::RefCell;
use std::collections::BTreeMap;

const YEARS: i64 = 100;
const ARCHIVE_EVERY_YEARS: i64 = 10;

/// A fiscal archive store in memory.
#[derive(Default)]
struct MemoryStore {
    objects: RefCell<BTreeMap<(String, String), Vec<u8>>>,
    handles: RefCell<BTreeMap<String, FiscalArchiveRetentionPhaseV1>>,
}

impl MemoryStore {
    fn bytes(&self) -> usize {
        self.objects.borrow().values().map(Vec::len).sum()
    }
}

impl FiscalArchiveStore for MemoryStore {
    fn store_fiscal_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
        bytes: &[u8],
    ) -> Result<(), FiscalArchiveError> {
        self.objects
            .borrow_mut()
            .insert((namespace.to_owned(), object_id.to_owned()), bytes.to_vec());
        Ok(())
    }

    fn load_fiscal_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, FiscalArchiveError> {
        Ok(self
            .objects
            .borrow()
            .get(&(namespace.to_owned(), object_id.to_owned()))
            .cloned())
    }

    fn persist_fiscal_archive_retention(
        &self,
        handle: &FiscalArchiveRetentionHandleV1,
    ) -> Result<(), FiscalArchiveError> {
        self.handles
            .borrow_mut()
            .insert(handle.handle_id.clone(), handle.phase);
        Ok(())
    }

    fn finalize_fiscal_archive_retention(
        &self,
        handle_id: &str,
        phase: FiscalArchiveRetentionPhaseV1,
    ) -> Result<(), FiscalArchiveError> {
        self.handles
            .borrow_mut()
            .insert(handle_id.to_owned(), phase);
        Ok(())
    }
}

fn fiscal_state(canwu: &Canwu) -> canwu_fiscal::FiscalState {
    audit_fiscal_records(canwu).expect("fiscal records audit")
}

/// The bytes of every fiscal record a run writes, and of the largest; the
/// create-only catalog and index pages aside.
fn state_bytes(canwu: &Canwu) -> (usize, usize) {
    canwu
        .domain_records()
        .filter(|record| {
            record.reference.kind.namespace == "canwu.fiscal"
                && !matches!(record.reference.kind.name.as_str(), "catalog" | "index")
        })
        .map(|record| serde_json::to_vec(&record.payload).expect("bytes").len())
        .fold((0, 0), |(total, largest), bytes| {
            (total + bytes, largest.max(bytes))
        })
}

#[test]
fn a_century_with_an_archive_cadence_keeps_the_fiscal_records_bounded() {
    let mut canwu = new_ming_fiscal_reference(DEFAULT_SEED, "hongwu-1391").expect("runtime");
    let store = MemoryStore::default();
    let mut largest = 0;
    let mut sizes = Vec::new();
    for year in 1..=YEARS {
        let prefix = format!("year.{year:03}");
        run_ming_fiscal_sample_cycle(&mut canwu, &prefix).expect("the year's cycle");
        close_ming_fiscal_sample_cycle(&mut canwu, &prefix).expect("the year's close");
        largest = largest.max(state_bytes(&canwu).1);
        if year % ARCHIVE_EVERY_YEARS == 0 {
            let archived = archive_ming_fiscal_books(&mut canwu, &store).expect("the decade");
            assert_eq!(archived, u64::try_from(ARCHIVE_EVERY_YEARS).expect("years"));
        }
        let year_end = SimTime::EPOCH
            .checked_add(SimDuration::days(year * 36_525 / 100))
            .expect("the year's end");
        let remaining = year_end
            .checked_sub(canwu.time())
            .expect("the year is not over");
        canwu
            .advance_canonical(remaining)
            .expect("the rest of the year");
        if matches!(year, 10 | 25 | 50 | 100) {
            sizes.push((
                year,
                state_bytes(&canwu).0,
                canwu.snapshot_json().expect("snapshot").len(),
                store.bytes(),
            ));
        }
    }
    for (year, state, snapshot, archive) in &sizes {
        eprintln!(
            "year {year}: fiscal records {state} bytes, snapshot {snapshot} bytes, archive {archive} bytes"
        );
    }
    eprintln!("largest fiscal record {largest} bytes");

    assert!(largest < MAX_FISCAL_LEDGER_RECORD_BYTES);
    // The records hold at most one decade of books, so they do not grow with
    // the run: after a century they are no larger than after its first
    // decade, give or take the digits of their counters.
    let state_after = |year: i64| {
        sizes
            .iter()
            .find(|(at, ..)| *at == year)
            .map(|(_, state, ..)| *state)
            .expect("a recorded year")
    };
    assert!(state_after(100) < state_after(10) + 1_024);
    let state = fiscal_state(&canwu);
    assert!(state.books().next().is_none());
    assert_eq!(
        state.archive.archived_book_count,
        u64::try_from(YEARS).expect("years")
    );
    assert!(state.archive.pending_handles.is_empty());
    audit_fiscal_archive(&store, &state).expect("the archive audits");
    let first = state
        .scope_bindings()
        .find_map(|scope| {
            archived_fiscal_record(
                &state,
                &store,
                &FiscalArchiveKeyV1::Book {
                    scope_binding_id: scope.id.clone(),
                    accounting_cycle_id: state
                        .ledger(&scope.id)
                        .and_then(|ledger| ledger.watermark.clone())?
                        .replace("year.100", "year.001"),
                },
            )
            .expect("a lookup")
        })
        .expect("the first year's book");
    let FiscalArchivedRecordV1::Book(first) = first else {
        panic!("a book");
    };
    assert_eq!(first.totals[0].collected, 70);
}
