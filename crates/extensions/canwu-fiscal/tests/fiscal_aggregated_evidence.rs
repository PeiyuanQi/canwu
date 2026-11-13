//! One aggregated execution evidence record receipts several scopes.
//!
//! An office's collection day across its scopes is one evidence record whose
//! parts name each scope's request and quantity, and one receipts packet with
//! one receipt per part, each citing that same record version. Each scope's
//! book is receipted at its part's quantity, and the books are the ones
//! per-scope evidence leaves, but for the record each receipt cites. A
//! receipt is refused, as a recorded rejection, when its scope is unknown,
//! when the record has no part for its scope or its parts do not add up to
//! the record's quantity, when its part asks more than its request, or when
//! its scope already settled a receipt citing that version; the packet's
//! other receipts settle, and the boundary commits.

mod support;

use canwu_api::{Canwu, DomainRecordVersionRef};
use canwu_fiscal::{
    FISCAL_RECEIPT_CLAIM_MISMATCH, FISCAL_RECEIPT_EVIDENCE_REUSED,
    FISCAL_RECEIPT_QUANTITY_EXCEEDED, FISCAL_RECEIPT_REQUEST_UNKNOWN, FiscalExecutionEvidence,
    FiscalExecutionEvidencePart, FiscalExecutionReceiptPacket, FiscalState,
    enqueue_execution_receipts,
};
use std::collections::BTreeSet;
use support::{
    Fixture, PRINCIPAL, TREASURY_AUTHORITY, assessment, collection, execution_claim, fiscal_state,
    ledger_revision, receipt_packet, record_results, refusals, scenario, settle, submit,
    treasury_scope,
};

const CYCLE: &str = "2026";
const SCOPES: usize = 3;
const DAY: [u64; SCOPES] = [20, 30, 10];

fn request(scope: usize) -> String {
    format!("request.{scope}")
}

/// Opens a book in each of the first treasury scopes, each with one
/// collection request of 30.
fn open_books(canwu: &mut Canwu) {
    submit(
        canwu,
        (0..SCOPES)
            .map(|scope| {
                let action = format!("assess.{scope}");
                let draft = assessment(&action, TREASURY_AUTHORITY, &treasury_scope(scope), CYCLE);
                (PRINCIPAL, draft)
            })
            .collect(),
    );
    submit(
        canwu,
        (0..SCOPES)
            .map(|scope| {
                let draft = collection(
                    &format!("authorize.{scope}"),
                    &treasury_scope(scope),
                    &format!("assess.{scope}.assessment"),
                    &request(scope),
                    30,
                );
                (PRINCIPAL, draft)
            })
            .collect(),
    );
}

fn part(scope: &str, request: &str, quantity: u64) -> FiscalExecutionEvidencePart {
    FiscalExecutionEvidencePart {
        scope_binding_id: scope.to_owned(),
        request_id: request.to_owned(),
        quantity,
    }
}

/// One execution across scopes: a record of the parts, under `operation`,
/// whose quantity is the parts' sum.
fn aggregated(
    id: &str,
    operation: &str,
    parts: Vec<FiscalExecutionEvidencePart>,
) -> FiscalExecutionEvidence {
    let mut evidence = execution_claim(id, "", "", parts.iter().map(|part| part.quantity).sum());
    operation.clone_into(&mut evidence.external_operation_id);
    evidence.parts = parts;
    evidence
}

/// Settles the receipts as one packet.
fn receipt(canwu: &mut Canwu, receipts: &[FiscalExecutionReceiptPacket]) {
    let now = canwu.time();
    enqueue_execution_receipts(canwu, now, receipts).expect("receipts packet");
    settle(canwu);
}

/// The state with each receipt's cited evidence set aside, and what each
/// cited, in book order.
fn without_citations(
    mut state: FiscalState,
) -> (FiscalState, Vec<BTreeSet<DomainRecordVersionRef>>) {
    let mut cited = Vec::new();
    for ledger in state.ledgers.values_mut() {
        for book in ledger.books.values_mut() {
            for receipt in book.execution_receipts.values_mut() {
                cited.push(std::mem::take(&mut receipt.external_evidence));
            }
        }
    }
    (state, cited)
}

#[test]
#[allow(clippy::too_many_lines)]
fn one_record_receipts_each_part_in_its_own_scope() {
    let fixture = Fixture::new();

    // The day as per-scope evidence: one record and one receipt per scope,
    // all of one external operation.
    let mut per_scope = fixture.runtime(scenario(|_| {}));
    open_books(&mut per_scope);
    let records: Vec<_> = (0..SCOPES)
        .map(|scope| {
            let mut claim = execution_claim(
                &format!("result.{scope}"),
                &treasury_scope(scope),
                &request(scope),
                DAY[scope],
            );
            claim.external_operation_id = "operation.day".to_owned();
            (false, claim)
        })
        .collect();
    let versions = record_results(&mut per_scope, &records);
    let receipts: Vec<_> = (0..SCOPES)
        .zip(versions)
        .map(|(scope, version)| {
            receipt_packet(
                &format!("receipt.{scope}"),
                &treasury_scope(scope),
                &request(scope),
                [version],
            )
        })
        .collect();
    receipt(&mut per_scope, &receipts);

    // The day as one aggregated record, cited by one receipt per scope.
    let mut canwu = fixture.runtime(scenario(|_| {}));
    open_books(&mut canwu);
    let parts = (0..SCOPES)
        .map(|scope| part(&treasury_scope(scope), &request(scope), DAY[scope]))
        .collect();
    let [day] = <[_; 1]>::try_from(record_results(
        &mut canwu,
        &[(false, aggregated("result.day", "operation.day", parts))],
    ))
    .expect("one version");
    let receipts: Vec<_> = (0..SCOPES)
        .map(|scope| {
            receipt_packet(
                &format!("receipt.{scope}"),
                &treasury_scope(scope),
                &request(scope),
                [day.clone()],
            )
        })
        .collect();
    receipt(&mut canwu, &receipts);
    assert!(refusals(&canwu).is_empty(), "{:#?}", refusals(&canwu));
    let receipted: Vec<_> = fiscal_state(&canwu)
        .execution_receipts()
        .map(|receipt| receipt.quantity)
        .collect();
    assert_eq!(receipted, DAY);
    let (books, cited) = without_citations(fiscal_state(&canwu));
    assert!(
        cited
            .iter()
            .all(|cited| *cited == BTreeSet::from([day.clone()]))
    );
    assert_eq!(books, without_citations(fiscal_state(&per_scope)).0);

    // A second day, whose record has a part for a scope that does not exist,
    // none for the second scope, and one that asks more than its request; a
    // record whose parts do not add up to its quantity; and the first day's
    // record cited again in a scope that already settled it.
    let second = aggregated(
        "result.second",
        "operation.second",
        vec![
            part(&treasury_scope(0), &request(0), 5),
            part("scope.nowhere", "request.nowhere", 5),
            part(&treasury_scope(2), &request(2), 40),
        ],
    );
    let mut uneven = aggregated(
        "result.uneven",
        "operation.uneven",
        vec![part(&treasury_scope(0), &request(0), 1)],
    );
    uneven.quantity = 2;
    let [second, uneven] = <[_; 2]>::try_from(record_results(
        &mut canwu,
        &[(false, second), (false, uneven)],
    ))
    .expect("two versions");
    let revisions: Vec<_> = (0..SCOPES)
        .map(|scope| ledger_revision(&canwu, &treasury_scope(scope)))
        .collect();
    let refused_before = refusals(&canwu).len();
    receipt(
        &mut canwu,
        &[
            receipt_packet(
                "receipt.second.0",
                &treasury_scope(0),
                &request(0),
                [second.clone()],
            ),
            receipt_packet(
                "receipt.second.nowhere",
                "scope.nowhere",
                "request.nowhere",
                [second.clone()],
            ),
            receipt_packet(
                "receipt.second.1",
                &treasury_scope(1),
                &request(1),
                [second.clone()],
            ),
            receipt_packet(
                "receipt.second.2",
                &treasury_scope(2),
                &request(2),
                [second],
            ),
            receipt_packet("receipt.uneven", &treasury_scope(0), &request(0), [uneven]),
            receipt_packet("receipt.again", &treasury_scope(0), &request(0), [day]),
        ],
    );

    let summaries = refusals(&canwu)[refused_before..].to_vec();
    let expected = [
        ("receipt.second.nowhere)", FISCAL_RECEIPT_REQUEST_UNKNOWN),
        ("receipt.second.1)", FISCAL_RECEIPT_CLAIM_MISMATCH),
        ("receipt.second.2)", FISCAL_RECEIPT_QUANTITY_EXCEEDED),
        ("receipt.uneven)", FISCAL_RECEIPT_CLAIM_MISMATCH),
        ("receipt.again)", FISCAL_RECEIPT_EVIDENCE_REUSED),
    ];
    assert_eq!(summaries.len(), expected.len(), "{summaries:#?}");
    for ((receipt, reason), summary) in expected.iter().zip(&summaries) {
        assert!(
            summary.contains(receipt) && summary.ends_with(reason),
            "{summary} should name {receipt} and end with {reason}"
        );
    }
    // Only the first scope's part settled; a refused receipt changed nothing.
    let receipted: Vec<_> = fiscal_state(&canwu)
        .execution_receipts()
        .map(|receipt| (receipt.id.clone(), receipt.quantity))
        .collect();
    assert_eq!(
        receipted,
        [
            ("receipt.0".to_owned(), 20),
            ("receipt.second.0".to_owned(), 5),
            ("receipt.1".to_owned(), 30),
            ("receipt.2".to_owned(), 10),
        ]
    );
    assert_eq!(
        (0..SCOPES)
            .map(|scope| ledger_revision(&canwu, &treasury_scope(scope)))
            .collect::<Vec<_>>(),
        [revisions[0] + 1, revisions[1], revisions[2]]
    );
    fixture.assert_restore_and_replay(&canwu);
}
