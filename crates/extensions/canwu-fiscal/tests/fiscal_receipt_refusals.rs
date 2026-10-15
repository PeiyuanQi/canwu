//! No execution receipt or historical-context change fails a boundary.
//!
//! Each one settlement cannot accept is refused with one
//! `canwu.fiscal.ingress_rejected.v1` event whose summary names the packet
//! and ends with a stable reason; it changes nothing and consumes nothing,
//! and the packets after it in the same boundary settle. (A receipt or change
//! whose result would not fit the state's byte budget is refused the same
//! way, with `state_budget_exhausted`; the crate's unit tests reach that
//! budget on a small state.)

mod support;

use canwu_api::{Canwu, PersonId, PluginIngressRequest};
use canwu_fiscal::{
    FISCAL_CONTEXT_OUTSIDE_PACK, FISCAL_EXECUTION_RECEIPT_INGRESS,
    FISCAL_HISTORICAL_CONTEXT_INGRESS, FISCAL_INGRESS_MALFORMED_REJECTION,
    FISCAL_RECEIPT_CLAIM_MISMATCH, FISCAL_RECEIPT_CYCLE_CLOSED, FISCAL_RECEIPT_EVIDENCE_COUNT,
    FISCAL_RECEIPT_EVIDENCE_KIND_UNAPPROVED, FISCAL_RECEIPT_EVIDENCE_NOT_CURRENT,
    FISCAL_RECEIPT_EVIDENCE_REUSED, FISCAL_RECEIPT_ID_CONFLICT, FISCAL_RECEIPT_OPERATION_REUSED,
    FISCAL_RECEIPT_QUANTITY_EXCEEDED, FISCAL_RECEIPT_REQUEST_UNKNOWN, FiscalExecutionReceiptPacket,
    FiscalHistoricalContextPacket, FiscalHistoricalMode, PLUGIN_NAME, enqueue_execution_receipt,
    fiscal_historical_context_ingress,
};
use support::{
    Draft, Fixture, MemoryArchiveStore, PRINCIPAL, TREASURY_AUTHORITY, archive, assessment, close,
    collection, execution_claim, fiscal_state, receipt_packet, record_results, refusals, scenario,
    settle, submit, treasury_scope,
};

fn by_principal(drafts: impl IntoIterator<Item = Draft>) -> Vec<(PersonId, Draft)> {
    drafts.into_iter().map(|draft| (PRINCIPAL, draft)).collect()
}

/// Queues a receipt as raw host ingress, so that settlement, not the
/// enqueue helper's checks, decides on it.
fn enqueue_raw(canwu: &mut Canwu, payload: serde_json::Value) {
    let now = canwu.time();
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            PLUGIN_NAME,
            FISCAL_EXECUTION_RECEIPT_INGRESS,
            now,
            payload,
        ))
        .expect("receipt ingress");
}

fn enqueue_receipt(canwu: &mut Canwu, packet: &FiscalExecutionReceiptPacket) {
    enqueue_raw(
        canwu,
        serde_json::to_value(packet).expect("receipt payload"),
    );
}

/// Opens a book of `cycle` in `scope` with one assessment of 100 and the
/// collection requests listed, each of `quantity`.
fn open_book(canwu: &mut Canwu, scope: usize, cycle: &str, requests: &[(&str, u64)]) {
    let scope_id = treasury_scope(scope);
    let action = format!("{cycle}.{scope}.assess");
    submit(
        canwu,
        by_principal([assessment(&action, TREASURY_AUTHORITY, &scope_id, cycle)]),
    );
    submit(
        canwu,
        by_principal(requests.iter().map(|(request, quantity)| {
            collection(
                &format!("{request}.authorize"),
                &scope_id,
                &format!("{action}.assessment"),
                request,
                *quantity,
            )
        })),
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn every_refused_receipt_or_context_is_an_event_and_the_rest_settle() {
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(scenario(|_| {}));
    let open = treasury_scope(0);
    let archived = treasury_scope(1);
    let closed = treasury_scope(3);
    let store = MemoryArchiveStore::default();
    open_book(
        &mut canwu,
        0,
        "2026",
        &[("request.1", 30), ("request.2", 30)],
    );
    // A request whose book is archived, and one whose book is closed.
    open_book(&mut canwu, 1, "2025", &[("request.archived", 30)]);
    submit(
        &mut canwu,
        by_principal([close("close.archived", &archived, "2025")]),
    );
    archive(&mut canwu, &store, 64);
    open_book(&mut canwu, 3, "2026", &[("request.closed", 30)]);
    submit(
        &mut canwu,
        by_principal([close("close.closed", &closed, "2026")]),
    );

    let mut unit_mismatch = execution_claim("result.mismatch", &open, "request.1", 5);
    unit_mismatch.unit = "silver".to_owned();
    let mut operation_twin = execution_claim("result.twin", &open, "request.1", 5);
    operation_twin.external_operation_id = "operation.result.ok-1".to_owned();
    let evidence = record_results(
        &mut canwu,
        &[
            (
                false,
                execution_claim("result.ok-1", &open, "request.1", 20),
            ),
            (
                false,
                execution_claim("result.ok-2", &open, "request.2", 30),
            ),
            (
                true,
                execution_claim("result.unapproved", &open, "request.1", 5),
            ),
            (false, unit_mismatch),
            (false, execution_claim("result.big", &open, "request.2", 40)),
            (false, operation_twin),
            (
                false,
                execution_claim("result.closed", &closed, "request.closed", 5),
            ),
            (
                false,
                execution_claim("result.archived", &archived, "request.archived", 5),
            ),
        ],
    );
    let [
        ok_1,
        ok_2,
        unapproved,
        mismatch,
        big,
        twin,
        in_closed,
        in_archived,
    ] = <[_; 8]>::try_from(evidence).expect("eight versions");
    let mut not_current = ok_2.clone();
    not_current.version = 99;
    let revision = fiscal_state(&canwu).procedure_revision;
    let receipts_before = fiscal_state(&canwu).execution_receipts().count();

    enqueue_receipt(
        &mut canwu,
        &receipt_packet("receipt.ok-1", &open, "request.1", [ok_1.clone()]),
    );
    enqueue_raw(&mut canwu, serde_json::json!({ "receipt_id": "typo" }));
    enqueue_receipt(
        &mut canwu,
        &receipt_packet(
            "receipt.nowhere",
            "scope.nowhere",
            "request.1",
            [ok_2.clone()],
        ),
    );
    enqueue_receipt(
        &mut canwu,
        &receipt_packet("receipt.unknown", &open, "request.missing", [ok_2.clone()]),
    );
    enqueue_receipt(
        &mut canwu,
        &receipt_packet(
            "receipt.archived",
            &archived,
            "request.archived",
            [in_archived],
        ),
    );
    enqueue_receipt(
        &mut canwu,
        &receipt_packet("receipt.closed", &closed, "request.closed", [in_closed]),
    );
    enqueue_receipt(
        &mut canwu,
        &receipt_packet("receipt.empty", &open, "request.1", []),
    );
    enqueue_receipt(
        &mut canwu,
        &receipt_packet("receipt.unapproved", &open, "request.1", [unapproved]),
    );
    enqueue_receipt(
        &mut canwu,
        &receipt_packet("receipt.stale", &open, "request.2", [not_current]),
    );
    enqueue_receipt(
        &mut canwu,
        &receipt_packet("receipt.mismatch", &open, "request.1", [mismatch]),
    );
    enqueue_receipt(
        &mut canwu,
        &receipt_packet("receipt.big", &open, "request.2", [big]),
    );
    enqueue_receipt(
        &mut canwu,
        &receipt_packet("receipt.twin", &open, "request.1", [twin]),
    );
    enqueue_receipt(
        &mut canwu,
        &receipt_packet("receipt.again", &open, "request.1", [ok_1]),
    );
    enqueue_receipt(
        &mut canwu,
        &receipt_packet("receipt.ok-1", &open, "request.2", [ok_2.clone()]),
    );
    let now = canwu.time();
    canwu
        .enqueue_plugin_ingress(
            fiscal_historical_context_ingress(
                now,
                &FiscalHistoricalContextPacket {
                    year: 999,
                    mode: FiscalHistoricalMode::Counterfactual,
                },
            )
            .expect("context"),
        )
        .expect("context ingress");
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            PLUGIN_NAME,
            FISCAL_HISTORICAL_CONTEXT_INGRESS,
            now,
            serde_json::json!({ "year": "late" }),
        ))
        .expect("malformed context ingress");
    enqueue_execution_receipt(
        &mut canwu,
        now,
        &receipt_packet("receipt.ok-2", &open, "request.2", [ok_2]),
    )
    .expect("the last receipt");
    let refused_before = refusals(&canwu).len();
    settle(&mut canwu);

    let summaries = refusals(&canwu)[refused_before..].to_vec();
    let expected = [
        ("receipt_v1)", FISCAL_INGRESS_MALFORMED_REJECTION),
        ("receipt.nowhere)", FISCAL_RECEIPT_REQUEST_UNKNOWN),
        ("receipt.unknown)", FISCAL_RECEIPT_REQUEST_UNKNOWN),
        ("receipt.archived)", FISCAL_RECEIPT_REQUEST_UNKNOWN),
        ("receipt.closed)", FISCAL_RECEIPT_CYCLE_CLOSED),
        ("receipt.empty)", FISCAL_RECEIPT_EVIDENCE_COUNT),
        (
            "receipt.unapproved)",
            FISCAL_RECEIPT_EVIDENCE_KIND_UNAPPROVED,
        ),
        ("receipt.stale)", FISCAL_RECEIPT_EVIDENCE_NOT_CURRENT),
        ("receipt.mismatch)", FISCAL_RECEIPT_CLAIM_MISMATCH),
        ("receipt.big)", FISCAL_RECEIPT_QUANTITY_EXCEEDED),
        ("receipt.twin)", FISCAL_RECEIPT_OPERATION_REUSED),
        ("receipt.again)", FISCAL_RECEIPT_EVIDENCE_REUSED),
        ("receipt.ok-1)", FISCAL_RECEIPT_ID_CONFLICT),
        ("historical_context_v1)", FISCAL_CONTEXT_OUTSIDE_PACK),
        ("historical_context_v1)", FISCAL_INGRESS_MALFORMED_REJECTION),
    ];
    assert_eq!(summaries.len(), expected.len(), "{summaries:#?}");
    for ((packet, reason), summary) in expected.iter().zip(&summaries) {
        assert!(
            summary.contains(packet) && summary.ends_with(reason),
            "{summary} should name {packet} and end with {reason}"
        );
    }

    // The two valid receipts settled, around every refusal; nothing a
    // refused packet named changed, and only the two advanced the revision.
    let state = fiscal_state(&canwu);
    assert_eq!(state.procedure_revision, revision + 2);
    assert_eq!(state.execution_receipts().count(), receipts_before + 2);
    let quantities: Vec<_> = state
        .execution_receipts()
        .map(|receipt| (receipt.id.as_str(), receipt.quantity))
        .collect();
    assert_eq!(quantities, [("receipt.ok-1", 20), ("receipt.ok-2", 30)]);
    assert_eq!(state.historical_context.year, 150);

    // A refused receipt consumed nothing: its evidence settles a receipt of
    // its own later.
    let mut later = execution_claim("result.later", &open, "request.1", 10);
    later.external_operation_id = "operation.result.later".to_owned();
    let later = record_results(&mut canwu, &[(false, later)]);
    let now = canwu.time();
    enqueue_execution_receipt(
        &mut canwu,
        now,
        &receipt_packet("receipt.later", &open, "request.1", later),
    )
    .expect("a later receipt");
    settle(&mut canwu);
    assert_eq!(
        fiscal_state(&canwu).execution_receipts().count(),
        receipts_before + 3
    );
    fixture.assert_restore_and_replay(&canwu);
}
