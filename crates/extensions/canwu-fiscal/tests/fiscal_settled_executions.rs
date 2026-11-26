//! A settled execution request leaves the hot book.
//!
//! The receipt that brings a request's fulfilled quantity to its own moves
//! the request, with all of its receipts, from its book into its scope's
//! settled bucket (`canwu.fiscal:settled`, one of `FISCAL_SETTLED_BUCKETS`
//! per scope, chosen by the request ID), and the book keeps per-assessment
//! figures of what moved out. A ledger write then costs its own entry, not
//! the scope's settled history; the bucket write costs about one bucket's
//! share of it. Totals, reports, closing, audits, caps, every receipt
//! refusal, and the resend rule read the same as when the request was hot.
//! Moving out raises no watermark and archives nothing; the archive takes
//! the whole book, settled requests merged back, and empties their buckets.

mod support;

use canwu_api::{
    BoundaryRequest, Canwu, CompactedCanwu, DomainRecord, EntityRef, EvidenceRef,
    PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD, PayloadRequiredEvidenceContinuationV1, PersonId,
};
use canwu_api::{DomainRecordVersionRef, ResourceId, SimDuration, SimTime};
use canwu_fiscal::{
    FISCAL_RECEIPT_CYCLE_CLOSED, FISCAL_RECEIPT_EVIDENCE_REUSED, FISCAL_RECEIPT_ID_CONFLICT,
    FISCAL_RECEIPT_OPERATION_REUSED, FISCAL_RECEIPT_QUANTITY_EXCEEDED, FISCAL_SETTLED_BUCKETS,
    FiscalAction, FiscalActionDisposition, FiscalArchiveError, FiscalArchiveKeyV1,
    FiscalArchivedRecordV1, FiscalAssessment, FiscalAuditSeverity, FiscalCycleTotal,
    FiscalExecutionKind, FiscalExecutionReceiptPacket, FiscalExecutionRequest, FiscalPaymentForm,
    FiscalReceiptDisposition, FiscalSettledRecord, FiscalState,
    MAX_FISCAL_EXECUTION_REQUESTS_PER_BOOK, MAX_FISCAL_LEDGER_RECORD_BYTES, archived_fiscal_record,
    enqueue_execution_receipts, enqueue_fiscal_archive, finalize_fiscal_archive_retention,
    fiscal_ledger_reference, fiscal_settled_reference, prepare_fiscal_archive,
};
use std::collections::BTreeSet;
use support::{
    Draft, Fixture, MemoryArchiveStore, MemoryEvidenceArchive, PRINCIPAL, RULE, TREASURY_AUTHORITY,
    archive, assessment, base_state, close, collection, draft, enqueue_results, execution_claim,
    fiscal_report, fiscal_state, ledger_revision, outcome, receipt_packet, record_results,
    refusals, result_versions, scenario, scenario_from_state, seal, sealed_fiscal_state, settle,
    submit, submit_sealed, treasury, treasury_scope,
};

const CYCLE: &str = "2026";
/// The cycles of the authored books, in order.
const CYCLES: [&str; 4] = ["2026", "2027", "2028", "2029"];

fn by_principal(drafts: impl IntoIterator<Item = Draft>) -> Vec<(PersonId, Draft)> {
    drafts.into_iter().map(|draft| (PRINCIPAL, draft)).collect()
}

fn applied(canwu: &Canwu, action_id: &str) -> bool {
    outcome(canwu, action_id)
        .is_some_and(|settled| settled.disposition == FiscalActionDisposition::Applied)
}

fn authored(cycle: &str) -> String {
    format!("authored.{cycle}")
}

fn request_id(cycle: &str, index: usize) -> String {
    format!("request.{cycle}.{index:03}")
}

/// The base state with, in each of the first `books` cycles of `CYCLES`, one
/// assessment of 1,000 grain on the first treasury scope and `count`
/// collection requests of 2 against it, all hot.
fn state_with_requests(books: usize, count: usize) -> FiscalState {
    let scope = treasury_scope(0);
    let mut state = base_state();
    for cycle in &CYCLES[..books] {
        state
            .add_assessment(FiscalAssessment {
                id: authored(cycle),
                rule_id: RULE.to_owned(),
                scope_binding_id: scope.clone(),
                accounting_cycle_id: (*cycle).to_owned(),
                quantity: 1_000,
                unit: "grain".to_owned(),
                payment_form: FiscalPaymentForm::Grain,
                commutation_quote: None,
                created_at: SimTime::EPOCH,
                source_action_id: None,
            })
            .expect("the assessment");
        for index in 0..count {
            state
                .add_execution_request(
                    &scope,
                    FiscalExecutionRequest {
                        id: request_id(cycle, index),
                        assessment_id: authored(cycle),
                        institution: treasury(),
                        kind: FiscalExecutionKind::Collect,
                        quantity: 2,
                        unit: "grain".to_owned(),
                        payment_form: FiscalPaymentForm::Grain,
                        resource: ResourceId::new(1),
                        source: EntityRef::Person(PRINCIPAL),
                        target: treasury(),
                        purpose: "fixture collection".to_owned(),
                        requested_at: SimTime::EPOCH,
                        source_action_id: None,
                    },
                )
                .expect("a request");
        }
    }
    state
}

/// Records one result per request of the first `books` books and settles
/// one receipt of each, all in one packet: every request is fulfilled.
fn settle_every_request(canwu: &mut Canwu, books: usize, count: usize) {
    let scope = treasury_scope(0);
    let requests: Vec<String> = CYCLES[..books]
        .iter()
        .flat_map(|cycle| (0..count).map(move |index| request_id(cycle, index)))
        .collect();
    let results: Vec<_> = requests
        .iter()
        .map(|request| {
            (
                false,
                execution_claim(&format!("result.{request}"), &scope, request, 2),
            )
        })
        .collect();
    let versions = record_results(canwu, &results);
    let receipts: Vec<FiscalExecutionReceiptPacket> = versions
        .into_iter()
        .zip(&requests)
        .map(|(version, request)| {
            receipt_packet(&format!("receipt.{request}"), &scope, request, [version])
        })
        .collect();
    let now = canwu.time();
    enqueue_execution_receipts(canwu, now, &receipts).expect("the receipts");
    settle(canwu);
    assert!(refusals(canwu).is_empty(), "{:#?}", refusals(canwu));
}

fn payload_bytes(record: &DomainRecord) -> usize {
    serde_json::to_vec(&record.payload)
        .expect("payload JSON")
        .len()
}

/// The payload bytes of a scope's ledger record.
fn ledger_bytes(canwu: &Canwu, scope: &str) -> usize {
    canwu
        .domain_record(fiscal_ledger_reference(scope).as_untyped())
        .map(payload_bytes)
        .expect("the ledger record")
}

/// The payload bytes of every settled bucket record of a scope.
fn bucket_bytes(canwu: &Canwu, scope: &str) -> Vec<usize> {
    let prefix = format!("{scope}/");
    canwu
        .domain_records()
        .filter(|record| {
            record.reference.kind.matches_type::<FiscalSettledRecord>()
                && record.reference.id.starts_with(&prefix)
        })
        .map(payload_bytes)
        .collect()
}

/// The payload-required evidence a record's payload declares.
fn declared(record: &DomainRecord) -> PayloadRequiredEvidenceContinuationV1 {
    serde_json::from_value(record.payload[PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD].clone())
        .expect("the record declares a continuation")
}

/// Whether a record declares `version` as payload-required evidence.
fn declares(record: &DomainRecord, version: &DomainRecordVersionRef) -> bool {
    declared(record)
        .dependencies
        .contains(&EvidenceRef::DomainRecordVersion(version.clone()))
}

/// The hot books' totals of one scope.
fn totals(state: &FiscalState, scope: &str) -> Vec<FiscalCycleTotal> {
    state
        .books()
        .filter(|(ledger, _)| ledger.scope.id == scope)
        .flat_map(|(ledger, book)| book.totals(ledger.scope.mechanism).expect("totals"))
        .collect()
}

/// A total's assessed, collected, and outstanding quantities.
fn figures(total: &FiscalCycleTotal) -> (u64, u64, u64) {
    (total.assessed, total.collected, total.outstanding)
}

/// One measure of a ledger before and after its requests settle.
struct Measure {
    /// The ledger's payload bytes while every request was hot.
    hot: usize,
    /// The ledger's payload bytes once every request settled.
    ledger: usize,
    /// The payload bytes of each settled bucket record.
    buckets: Vec<usize>,
}

/// As collections settle, within one book or across books, the ledger keeps
/// only the figures of what moved out, so its record stays the same size and
/// smaller than with one open collection; each settled request goes to one
/// of the scope's buckets, so the bucket a receipt writes holds about
/// `1 / FISCAL_SETTLED_BUCKETS` of the scope's settled history. The per-book
/// request cap counts settled requests.
#[test]
#[allow(clippy::too_many_lines)]
fn a_ledger_stays_flat_as_collections_settle() {
    let fixture = Fixture::new();
    let scope = treasury_scope(0);
    let measure = |books: usize, count: usize| {
        let mut canwu = fixture.runtime(scenario_from_state(state_with_requests(books, count)));
        let hot = ledger_bytes(&canwu, &scope);
        settle_every_request(&mut canwu, books, count);

        let state = fiscal_state(&canwu);
        let ledger = state.ledger(&scope).expect("the ledger");
        for cycle in &CYCLES[..books] {
            let book = &ledger.books[*cycle];
            assert!(book.execution_requests.is_empty());
            assert!(book.execution_receipts.is_empty());
            let figures = &book.settled[&authored(cycle)];
            assert_eq!(usize::try_from(figures.requests).unwrap(), count);
            assert_eq!(usize::try_from(figures.receipts).unwrap(), count);
            assert_eq!(figures.collected, 2 * u64::try_from(count).unwrap());
        }
        assert_eq!(state.settled_executions().count(), books * count);
        assert_eq!(state.execution_requests().count(), books * count);
        assert_eq!(state.execution_receipts().count(), books * count);

        let measured = Measure {
            hot,
            ledger: ledger_bytes(&canwu, &scope),
            buckets: bucket_bytes(&canwu, &scope),
        };
        let largest = measured.buckets.iter().copied().max().unwrap_or_default();
        let all: usize = measured.buckets.iter().sum();
        assert!(!measured.buckets.is_empty() && measured.buckets.len() <= FISCAL_SETTLED_BUCKETS);
        assert!(largest <= MAX_FISCAL_LEDGER_RECORD_BYTES);
        eprintln!(
            "{books} book(s) x {count} settled collections: ledger payload {} bytes \
             ({hot} with every request hot); largest settled bucket {largest} bytes; \
             {} buckets, {all} bytes in all",
            measured.ledger,
            measured.buckets.len()
        );
        (canwu, measured)
    };

    let (_, one) = measure(1, 1);
    let (_, twelve) = measure(1, 12);
    let (many_run, many) = measure(1, 48);
    fixture.assert_restore_and_replay(&many_run);
    // Only the figures' digits grow: the ledger stays flat, and below the
    // ledger that holds one open collection.
    assert!(
        many.ledger <= one.ledger + 32,
        "{} > {} + 32",
        many.ledger,
        one.ledger
    );
    assert!(many.ledger < one.hot, "{} >= {}", many.ledger, one.hot);
    assert!(twelve.ledger < twelve.hot && many.ledger < many.hot);
    // One settled collection writes one bucket of one entry.
    assert_eq!(one.buckets.len(), 1);
    let sum = |measured: &Measure| measured.buckets.iter().sum::<usize>();
    assert!(sum(&twelve) > one.buckets[0]);
    // Forty-eight spread over the buckets: the one a receipt writes holds
    // far less than the scope's whole settled history.
    assert!(sum(&many) > sum(&twelve));
    let largest = many.buckets.iter().copied().max().unwrap_or_default();
    assert!(largest * 2 <= sum(&many), "{largest} * 2 > {}", sum(&many));

    // Across books: four books of twelve settled collections each keep the
    // ledger as flat as four books of one.
    let (_, four_one) = measure(4, 1);
    let (_, four_twelve) = measure(4, 12);
    assert!(
        four_twelve.ledger <= four_one.ledger + 32,
        "{} > {} + 32",
        four_twelve.ledger,
        four_one.ledger
    );
    assert!(four_twelve.ledger < four_twelve.hot);
    assert!(sum(&four_twelve) > sum(&four_one));

    // The per-book request cap counts settled requests: a book whose
    // requests all settled at the cap takes no more.
    let (mut full, _) = measure(1, MAX_FISCAL_EXECUTION_REQUESTS_PER_BOOK);
    submit(
        &mut full,
        by_principal([collection(
            "one.more",
            &scope,
            &authored(CYCLE),
            "request.more",
            2,
        )]),
    );
    let rejected = outcome(&full, "one.more").expect("an outcome");
    assert_eq!(rejected.disposition, FiscalActionDisposition::Rejected);
    assert!(
        rejected.reason.contains("exceeds its bounded capacity"),
        "{}",
        rejected.reason
    );
}

/// A citation that moved to a settled bucket is declared by the bucket's
/// record, no longer by the ledger's: a continuation checkpoint taken then
/// restores and continues as the uninterrupted run, and the run seals,
/// restores from its sealed segments, and archives exactly as its unsealed
/// fork. Moving out raises no watermark, and a later book of the scope opens
/// before the archive; the archived book is the whole book, with the same
/// outstanding, and the archive empties the bucket, which then declares
/// nothing and restores from a continuation checkpoint, and raises the
/// watermarks as it always did.
#[test]
#[allow(clippy::too_many_lines)]
fn a_moved_out_citation_survives_seal_restore_and_archive() {
    let fixture = Fixture::new();
    let plugins = fixture.plugins();
    let mut base = fixture.runtime(scenario(|_| {}));
    let scope = treasury_scope(0);
    submit(
        &mut base,
        by_principal([assessment("assess", TREASURY_AUTHORITY, &scope, CYCLE)]),
    );
    submit(
        &mut base,
        by_principal([collection(
            "authorize",
            &scope,
            "assess.assessment",
            "request.a",
            60,
        )]),
    );
    let evidence = record_results(
        &mut base,
        &[(false, execution_claim("result.a", &scope, "request.a", 60))],
    );
    let now = base.time();
    enqueue_execution_receipts(
        &mut base,
        now,
        &[receipt_packet(
            "receipt.a",
            &scope,
            "request.a",
            evidence.clone(),
        )],
    )
    .expect("receipt");
    settle(&mut base);

    let state = fiscal_state(&base);
    let ledger = state.ledger(&scope).expect("the ledger");
    let book = &ledger.books[CYCLE];
    assert!(book.execution_requests.is_empty());
    assert!(book.execution_receipts.is_empty());
    let entry = state
        .settled_execution(&scope, "request.a")
        .expect("the settled request");
    assert_eq!(
        entry.receipts["receipt.a"].external_evidence,
        evidence.iter().cloned().collect::<BTreeSet<_>>()
    );
    // The bucket's record declares the citation as payload-required
    // evidence; the ledger's no longer does.
    let bucket = base
        .domain_record(fiscal_settled_reference(&scope, "request.a").as_untyped())
        .expect("the bucket record");
    assert!(declares(bucket, &evidence[0]));
    assert!(declared(bucket).active);
    assert!(!declares(
        base.domain_record(fiscal_ledger_reference(&scope).as_untyped())
            .expect("the ledger record"),
        &evidence[0]
    ));
    assert_eq!(ledger.watermark, None);
    assert_eq!(state.office(&treasury()).expect("office").watermark, None);
    let before_close = totals(&state, &scope);
    assert_eq!(figures(&before_close[0]), (100, 60, 40));

    // A continuation checkpoint taken while the bucket holds the entry
    // restores, and the restored run continues as the uninterrupted one.
    let (checkpoint, hash) = base
        .continuation_checkpoint()
        .expect("a continuation checkpoint");
    let mut continued =
        CompactedCanwu::from_continuation_checkpoint_verified(checkpoint, &hash, &plugins)
            .expect("the save with a settled bucket restores");
    assert_eq!(continued.checkpoint_hash(), base.checkpoint_hash());
    assert_eq!(
        sealed_fiscal_state(&continued).settled_execution(&scope, "request.a"),
        Some(entry)
    );

    // A later book of the scope opens; the settled one closes.
    let later = || by_principal([assessment("later", TREASURY_AUTHORITY, &scope, "2027")]);
    let closing = || by_principal([close("close", &scope, CYCLE)]);
    submit(&mut base, later());
    assert!(applied(&base, "later"));
    submit(&mut base, closing());
    assert!(applied(&base, "close"));
    base.settle_boundary(BoundaryRequest::at(base.time()))
        .expect("a quiet boundary before sealing");
    submit_sealed(&mut continued, later());
    submit_sealed(&mut continued, closing());
    continued
        .settle_boundary(BoundaryRequest::at(continued.time()))
        .expect("the same quiet boundary");
    assert_eq!(continued.checkpoint_hash(), base.checkpoint_hash());
    assert_eq!(
        continued.authoritative_state_hash().expect("state hash"),
        base.authoritative_state_hash().expect("state hash")
    );
    drop(continued);

    let mut unsealed = base.fork();
    let mut sealed = base.into_compacted().expect("compact interface");
    let segment = seal(&mut sealed, &MemoryEvidenceArchive::default());
    let mut from_sealed = CompactedCanwu::from_checkpoint_and_journal_with_plugins(
        sealed.checkpoint().expect("compact checkpoint"),
        vec![segment.clone()],
        &plugins,
    )
    .expect("the sealed segments restore the compact run");
    assert_eq!(
        sealed_fiscal_state(&from_sealed).settled_execution(&scope, "request.a"),
        fiscal_state(&unsealed).settled_execution(&scope, "request.a")
    );
    // Closing changes no total.
    assert_eq!(totals(&fiscal_state(&unsealed), &scope)[0], before_close[0]);

    let unsealed_store = MemoryArchiveStore::default();
    archive(&mut unsealed, &unsealed_store, 64);
    for run in [&mut sealed, &mut from_sealed] {
        let store = MemoryArchiveStore::default();
        let prepared = prepare_fiscal_archive(&*run, 64, &store).expect("the archive batch");
        let receipt =
            enqueue_fiscal_archive(run, &prepared, &store).expect("queued on the compact run");
        run.advance_canonical(SimDuration::minutes(1))
            .expect("the commit settles");
        finalize_fiscal_archive_retention(run, &store, &receipt).expect("finalized");
        run.advance_canonical(SimDuration::minutes(1))
            .expect("the acknowledgement settles");
    }
    assert_eq!(sealed.checkpoint_hash(), unsealed.checkpoint_hash());
    assert_eq!(from_sealed.checkpoint_hash(), unsealed.checkpoint_hash());

    let state = fiscal_state(&unsealed);
    assert_eq!(state.archive.archived_book_count, 1);
    assert!(state.settled_executions().next().is_none());
    // The emptied bucket stays as a record and declares nothing.
    let emptied = unsealed
        .domain_record(fiscal_settled_reference(&scope, "request.a").as_untyped())
        .expect("the emptied bucket record");
    assert_eq!(
        declared(emptied),
        PayloadRequiredEvidenceContinuationV1::completed()
    );
    let ledger = state.ledger(&scope).expect("the ledger");
    assert_eq!(ledger.books.keys().collect::<Vec<_>>(), ["2027"]);
    assert_eq!(ledger.watermark.as_deref(), Some(CYCLE));
    let Some(FiscalArchivedRecordV1::Book(archived)) = archived_fiscal_record(
        &state,
        &unsealed_store,
        &FiscalArchiveKeyV1::Book {
            scope_binding_id: scope.clone(),
            accounting_cycle_id: CYCLE.to_owned(),
        },
    )
    .expect("an archive lookup") else {
        panic!("the archived book resolves");
    };
    assert!(archived.book.execution_requests.contains_key("request.a"));
    assert!(archived.book.execution_receipts.contains_key("receipt.a"));
    assert!(archived.book.settled.is_empty());
    // The archived whole book has the hot book's totals, outstanding
    // included.
    assert_eq!(archived.totals[0], before_close[0]);

    // A continuation checkpoint taken after the archive, with the emptied
    // bucket, restores too.
    let (checkpoint, hash) = unsealed
        .continuation_checkpoint()
        .expect("the checkpoint after the archive");
    let after = CompactedCanwu::from_continuation_checkpoint_verified(checkpoint, &hash, &plugins)
        .expect("the save with an emptied bucket restores");
    assert_eq!(after.checkpoint_hash(), unsealed.checkpoint_hash());

    let journal = sealed
        .replay_journal_with_segments(vec![segment])
        .expect("an exact replay journal");
    let replayed = Canwu::replay_from_journal(&plugins, &journal).expect("the sealed run replays");
    assert_eq!(replayed.snapshot(), unsealed.snapshot());
}

/// On a run that seals its evidence, a closed book whose only citation newer
/// than the last seal moved out to a settled bucket is refused at archive
/// preparation until a seal covers it, then archives; the sealed run replays.
#[test]
fn a_moved_out_citation_holds_its_book_until_it_is_sealed() {
    let fixture = Fixture::new();
    let plugins = fixture.plugins();
    let scope = treasury_scope(0);
    let mut canwu = fixture.runtime(scenario(|_| {}));
    submit(
        &mut canwu,
        by_principal([assessment("assess", TREASURY_AUTHORITY, &scope, CYCLE)]),
    );
    submit(
        &mut canwu,
        by_principal([collection(
            "authorize",
            &scope,
            "assess.assessment",
            "request.a",
            60,
        )]),
    );
    let mut run = canwu.into_compacted().expect("compact interface");
    let evidence = MemoryEvidenceArchive::default();
    // A seal made here, before the result the receipt cites exists.
    let earlier = seal(&mut run, &evidence);
    let claims = [(false, execution_claim("result.a", &scope, "request.a", 60))];
    enqueue_results(&mut run, &claims);
    run.advance_canonical(SimDuration::minutes(1))
        .expect("the result is recorded");
    let cited = result_versions(&run, &claims);
    let now = run.time();
    enqueue_execution_receipts(
        &mut run,
        now,
        &[receipt_packet("receipt.a", &scope, "request.a", cited)],
    )
    .expect("receipt");
    run.advance_canonical(SimDuration::minutes(1))
        .expect("the receipt settles");
    submit_sealed(&mut run, by_principal([close("close", &scope, CYCLE)]));
    let state = sealed_fiscal_state(&run);
    assert!(state.settled_execution(&scope, "request.a").is_some());
    assert!(!state.ledger(&scope).expect("the ledger").books[CYCLE].is_open());

    // The receipt is the book's only citation, and it is in the bucket.
    let store = MemoryArchiveStore::default();
    let error = prepare_fiscal_archive(&run, 64, &store)
        .expect_err("the settled receipt's citation is newer than the last seal");
    assert_eq!(
        error,
        FiscalArchiveError::UnsealedCitations {
            scope_binding_id: scope.clone(),
            accounting_cycle_id: CYCLE.to_owned(),
        }
    );

    // Sealed again, the citation is in a segment, and the book archives.
    let segment = seal(&mut run, &evidence);
    let prepared = prepare_fiscal_archive(&run, 64, &store).expect("every citation is sealed");
    let receipt =
        enqueue_fiscal_archive(&mut run, &prepared, &store).expect("queued on the compact run");
    run.advance_canonical(SimDuration::minutes(1))
        .expect("the commit settles");
    finalize_fiscal_archive_retention(&mut run, &store, &receipt).expect("finalized");
    run.advance_canonical(SimDuration::minutes(1))
        .expect("the acknowledgement settles");
    let state = sealed_fiscal_state(&run);
    assert!(state.books().next().is_none());
    assert!(state.settled_executions().next().is_none());

    let segments = vec![earlier, segment];
    let journal = run
        .replay_journal_with_segments(segments.clone())
        .expect("an exact replay journal");
    let replayed = Canwu::replay_from_journal(&plugins, &journal).expect("the sealed run replays");
    assert_eq!(
        replayed.snapshot(),
        run.snapshot_with_segments(segments)
            .expect("the full snapshot")
    );
}

/// A run whose book mixes a settled request with a hot one, and a run whose
/// requests are both hot, with the same collections, read the same totals
/// and report facts, and both books close with them; the mixed run's ledger
/// is smaller. A closed book refuses a later receipt of its settled request
/// and takes a finding on it. The partly collected book keeps its
/// outstanding before close, after close, in the archived book, and in its
/// office's archived totals.
#[test]
#[allow(clippy::too_many_lines)]
fn settled_requests_read_the_same_totals_reports_and_close() {
    let fixture = Fixture::new();
    let scope = treasury_scope(0);
    let run = |requested: u64| {
        let mut canwu = fixture.runtime(scenario(|_| {}));
        submit(
            &mut canwu,
            by_principal([
                assessment("assess", TREASURY_AUTHORITY, &scope, CYCLE),
                // Another book of the office's cycle keeps the cycle hot
                // after the first book is archived.
                assessment("elsewhere", TREASURY_AUTHORITY, &treasury_scope(1), CYCLE),
            ]),
        );
        submit(
            &mut canwu,
            by_principal([
                collection(
                    "authorize",
                    &scope,
                    "assess.assessment",
                    "request.a",
                    requested,
                ),
                collection("authorize.b", &scope, "assess.assessment", "request.b", 20),
            ]),
        );
        let evidence = record_results(
            &mut canwu,
            &[
                (false, execution_claim("result.a", &scope, "request.a", 60)),
                (false, execution_claim("result.b", &scope, "request.b", 10)),
            ],
        );
        let now = canwu.time();
        enqueue_execution_receipts(
            &mut canwu,
            now,
            &[
                receipt_packet("receipt.a", &scope, "request.a", [evidence[0].clone()]),
                receipt_packet("receipt.b", &scope, "request.b", [evidence[1].clone()]),
            ],
        )
        .expect("receipts");
        settle(&mut canwu);
        settle(&mut canwu);
        canwu
    };
    let mut settled = run(60);
    let mut hot = run(70);
    let settled_state = fiscal_state(&settled);
    let hot_state = fiscal_state(&hot);
    // The settled run's book mixes a settled request with a hot one.
    let book = &settled_state.ledger(&scope).expect("ledger").books[CYCLE];
    assert_eq!(
        book.execution_requests.keys().collect::<Vec<_>>(),
        ["request.b"]
    );
    assert_eq!(book.settled["assess.assessment"].requests, 1);
    assert!(hot_state.settled_executions().next().is_none());
    assert_eq!(totals(&settled_state, &scope), totals(&hot_state, &scope));
    let before_close = totals(&settled_state, &scope);
    assert_eq!(figures(&before_close[0]), (100, 70, 30));
    assert_eq!(
        fiscal_report(&settled, PRINCIPAL).projection.facts,
        fiscal_report(&hot, PRINCIPAL).projection.facts
    );
    assert!(ledger_bytes(&settled, &scope) < ledger_bytes(&hot, &scope));

    for canwu in [&mut settled, &mut hot] {
        submit(canwu, by_principal([close("close", &scope, CYCLE)]));
        assert!(applied(canwu, "close"));
        settle(canwu);
    }
    let after_close = totals(&fiscal_state(&settled), &scope);
    assert_eq!(after_close, totals(&fiscal_state(&hot), &scope));
    assert_eq!(after_close, before_close);
    assert_eq!(
        fiscal_report(&settled, PRINCIPAL).projection.facts,
        fiscal_report(&hot, PRINCIPAL).projection.facts
    );

    // The closed book refuses a later receipt of its settled request, and
    // takes a finding on the request and on its receipt.
    let mut rejected = execution_claim("result.late", &scope, "request.a", 0);
    rejected.disposition = FiscalReceiptDisposition::Rejected;
    let late = record_results(&mut settled, &[(false, rejected)]);
    let refused = refusals(&settled).len();
    let now = settled.time();
    enqueue_execution_receipts(
        &mut settled,
        now,
        &[receipt_packet("receipt.late", &scope, "request.a", late)],
    )
    .expect("late receipt");
    settle(&mut settled);
    let summaries = refusals(&settled);
    assert_eq!(summaries.len(), refused + 1, "{summaries:#?}");
    assert!(
        summaries[refused].contains("receipt.late)")
            && summaries[refused].ends_with(FISCAL_RECEIPT_CYCLE_CLOSED),
        "{}",
        summaries[refused]
    );
    submit(
        &mut settled,
        by_principal(["request.a", "receipt.a"].map(|target| {
            draft(
                &format!("audit.{target}"),
                TREASURY_AUTHORITY,
                FiscalAction::RecordAudit {
                    audit_id: format!("finding.{target}"),
                    scope_binding_id: scope.clone(),
                    target_id: target.to_owned(),
                    severity: FiscalAuditSeverity::Notice,
                    finding: "checked after settlement".to_owned(),
                    evidence: BTreeSet::new(),
                },
            )
        })),
    );
    assert!(applied(&settled, "audit.request.a"));
    assert!(applied(&settled, "audit.receipt.a"));
    let state = fiscal_state(&settled);
    let audits = &state.ledger(&scope).expect("ledger").books[CYCLE].audits;
    assert_eq!(audits.len(), 2);
    fixture.assert_restore_and_replay(&settled);

    // Archived, the whole book keeps its outstanding, and so do the office's
    // archived totals of the cycle.
    let store = MemoryArchiveStore::default();
    archive(&mut settled, &store, 64);
    let state = fiscal_state(&settled);
    let Some(FiscalArchivedRecordV1::Book(archived)) = archived_fiscal_record(
        &state,
        &store,
        &FiscalArchiveKeyV1::Book {
            scope_binding_id: scope.clone(),
            accounting_cycle_id: CYCLE.to_owned(),
        },
    )
    .expect("an archive lookup") else {
        panic!("the archived book resolves");
    };
    assert_eq!(archived.book.execution_requests.len(), 2);
    assert_eq!(archived.totals[0], before_close[0]);
    let cycle = &state.office(&treasury()).expect("office").cycles[CYCLE];
    assert_eq!(cycle.archived_books, 1);
    assert_eq!(figures(&cycle.archived_totals[0]), (100, 70, 30));
}

/// Receipts of a settled request, and receipts of another request of its
/// scope, keep every refusal and the resend rule: a resend of a settled
/// receipt changes nothing; its ID with other evidence, its evidence, or its
/// operation is refused; a receipt past the settled request's quantity is
/// refused; a rejection of it joins it in its bucket. Its request ID stays
/// taken while its book is hot, and its collection still counts against
/// what its assessment has left.
#[test]
#[allow(clippy::too_many_lines)]
fn receipts_of_settled_requests_keep_every_refusal_and_resend_rule() {
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(scenario(|_| {}));
    let scope = treasury_scope(0);
    submit(
        &mut canwu,
        by_principal([assessment("assess", TREASURY_AUTHORITY, &scope, CYCLE)]),
    );
    submit(
        &mut canwu,
        by_principal([
            collection("authorize.a", &scope, "assess.assessment", "request.a", 30),
            collection("authorize.b", &scope, "assess.assessment", "request.b", 30),
        ]),
    );
    let [settling] = <[_; 1]>::try_from(record_results(
        &mut canwu,
        &[(false, execution_claim("result.a", &scope, "request.a", 30))],
    ))
    .expect("one version");
    let now = canwu.time();
    enqueue_execution_receipts(
        &mut canwu,
        now,
        &[receipt_packet(
            "receipt.a",
            &scope,
            "request.a",
            [settling.clone()],
        )],
    )
    .expect("receipt");
    settle(&mut canwu);
    assert!(
        fiscal_state(&canwu)
            .settled_execution(&scope, "request.a")
            .is_some()
    );

    let mut twin = execution_claim("result.twin", &scope, "request.b", 5);
    twin.external_operation_id = "operation.result.a".to_owned();
    let mut rejection = execution_claim("result.rejected", &scope, "request.a", 0);
    rejection.disposition = FiscalReceiptDisposition::Rejected;
    let [other, twin, more, rejection, partial] = <[_; 5]>::try_from(record_results(
        &mut canwu,
        &[
            (
                false,
                execution_claim("result.other", &scope, "request.a", 30),
            ),
            (false, twin),
            (
                false,
                execution_claim("result.more", &scope, "request.a", 5),
            ),
            (false, rejection),
            (false, execution_claim("result.b", &scope, "request.b", 10)),
        ],
    ))
    .expect("five versions");
    let revision = ledger_revision(&canwu, &scope);
    let refused = refusals(&canwu).len();
    let now = canwu.time();
    enqueue_execution_receipts(
        &mut canwu,
        now,
        &[
            receipt_packet("receipt.a", &scope, "request.a", [settling.clone()]),
            receipt_packet("receipt.a", &scope, "request.a", [other]),
            receipt_packet("receipt.b.reused", &scope, "request.b", [settling]),
            receipt_packet("receipt.b.twin", &scope, "request.b", [twin]),
            receipt_packet("receipt.a.more", &scope, "request.a", [more]),
            receipt_packet("receipt.a.rejected", &scope, "request.a", [rejection]),
            receipt_packet("receipt.b", &scope, "request.b", [partial]),
        ],
    )
    .expect("receipts");
    settle(&mut canwu);

    let summaries = refusals(&canwu)[refused..].to_vec();
    let expected = [
        ("receipt.a)", FISCAL_RECEIPT_ID_CONFLICT),
        ("receipt.b.reused)", FISCAL_RECEIPT_EVIDENCE_REUSED),
        ("receipt.b.twin)", FISCAL_RECEIPT_OPERATION_REUSED),
        ("receipt.a.more)", FISCAL_RECEIPT_QUANTITY_EXCEEDED),
    ];
    assert_eq!(summaries.len(), expected.len(), "{summaries:#?}");
    for ((receipt, reason), summary) in expected.iter().zip(&summaries) {
        assert!(
            summary.contains(receipt) && summary.ends_with(reason),
            "{summary} should name {receipt} and end with {reason}"
        );
    }
    // The resend changed nothing; the rejection and the other request's
    // receipt each advanced the ledger.
    assert_eq!(ledger_revision(&canwu, &scope), revision + 2);
    let state = fiscal_state(&canwu);
    let entry = state
        .settled_execution(&scope, "request.a")
        .expect("the settled request");
    assert_eq!(
        entry.receipts.keys().collect::<Vec<_>>(),
        ["receipt.a", "receipt.a.rejected"]
    );
    let book = &state.ledger(&scope).expect("ledger").books[CYCLE];
    assert_eq!(
        book.execution_receipts.keys().collect::<Vec<_>>(),
        ["receipt.b"]
    );
    assert_eq!(book.settled["assess.assessment"].requests, 1);
    assert_eq!(book.settled["assess.assessment"].receipts, 2);
    assert_eq!(book.settled["assess.assessment"].collected, 30);

    // The settled request's ID is still taken; its receipt and the request
    // take audit findings.
    submit(
        &mut canwu,
        by_principal([
            collection(
                "authorize.again",
                &scope,
                "assess.assessment",
                "request.a",
                5,
            ),
            // 30 settled and 30 hot leave 40 of the assessment's 100: a
            // collection of 50 claims more than is left.
            collection(
                "authorize.over",
                &scope,
                "assess.assessment",
                "request.c",
                50,
            ),
            draft(
                "audit",
                TREASURY_AUTHORITY,
                FiscalAction::RecordAudit {
                    audit_id: "finding".to_owned(),
                    scope_binding_id: scope.clone(),
                    target_id: "receipt.a".to_owned(),
                    severity: FiscalAuditSeverity::Notice,
                    finding: "checked after settlement".to_owned(),
                    evidence: BTreeSet::new(),
                },
            ),
        ]),
    );
    let again = outcome(&canwu, "authorize.again").expect("an outcome");
    assert_eq!(again.disposition, FiscalActionDisposition::Rejected);
    assert!(
        again
            .reason
            .contains("fiscal execution request identity already exists"),
        "{}",
        again.reason
    );
    let over = outcome(&canwu, "authorize.over").expect("an outcome");
    assert_eq!(over.disposition, FiscalActionDisposition::Rejected);
    assert!(
        over.reason.contains("over-remitted or double-claimed"),
        "{}",
        over.reason
    );
    assert!(applied(&canwu, "audit"));
    fixture.assert_restore_and_replay(&canwu);
}
