//! A record's byte budget is never a reason to fail a boundary.
//!
//! Every fiscal record has a serialized byte budget, and keeps
//! `FISCAL_RECORD_RING_RESERVE_BYTES` of it for its outcome ring. An action
//! whose result would put its record past the budget less that reserve is
//! rejected as its own outcome with `FISCAL_RECORD_BUDGET_EXHAUSTED`, and the
//! rejection is recorded in the reserve; an execution receipt that would put
//! its ledger past the budget is refused with `state_budget_exhausted`. The
//! boundary commits either way, and actions on other records settle in it.
//! A ledger's size is its record's payload, measured exactly, the evidence
//! continuation that names its citations included, so whatever settlement
//! accepts a restore accepts too, however long the cited records' IDs are.

mod support;

use canwu_api::{Canwu, EntityRef, ResourceId, SimTime};
use canwu_fiscal::{
    FISCAL_INGRESS_BUDGET_REJECTION, FISCAL_RECORD_BUDGET_EXHAUSTED,
    FISCAL_RECORD_RING_RESERVE_BYTES, FiscalAction, FiscalActionDisposition, FiscalAssessment,
    FiscalExecutionKind, FiscalExecutionRequest, FiscalPaymentForm, FiscalRemission, FiscalState,
    MAX_FISCAL_ACTION_TEXT_BYTES, MAX_FISCAL_LEDGER_RECORD_BYTES, enqueue_execution_receipts,
};
use support::{
    Fixture, PRINCIPAL, RULE, TREASURY_AUTHORITY, assessment, base_state, catalog, draft,
    execution_claim, fiscal_state, ledger_revision, outcome, receipt_packet, record_results,
    refusals, scenario_from_state, settle, submit, treasury, treasury_scope,
};

const ASSESSMENT: &str = "authored.full";
const REQUEST: &str = "request.full";
/// The reason length of the remissions that fill the ledger.
const PADDING_REASON: usize = 1_000;
/// What the full ledger leaves below its budget less the reserve: less than
/// any remission adds.
const MARGIN: usize = 32;
/// At least what a ledger's record payload adds to the ledger when it cites
/// nothing: the evidence continuation field.
const CONTINUATION: usize = 128;

/// The base state with one assessment and one collection request on the
/// treasury's first scope, and `padding` remissions with reasons of
/// `PADDING_REASON` bytes and one of `filler` bytes.
fn padded_state(padding: usize, filler: usize) -> FiscalState {
    let scope = treasury_scope(0);
    let mut state = base_state();
    state
        .add_assessment(FiscalAssessment {
            id: ASSESSMENT.to_owned(),
            rule_id: RULE.to_owned(),
            scope_binding_id: scope.clone(),
            accounting_cycle_id: "2026".to_owned(),
            quantity: 1_000_000,
            unit: "grain".to_owned(),
            payment_form: FiscalPaymentForm::Grain,
            commutation_quote: None,
            created_at: SimTime::EPOCH,
            source_action_id: None,
        })
        .expect("the assessment");
    state
        .add_execution_request(
            &scope,
            FiscalExecutionRequest {
                id: REQUEST.to_owned(),
                assessment_id: ASSESSMENT.to_owned(),
                institution: treasury(),
                kind: FiscalExecutionKind::Collect,
                quantity: 10,
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
        .expect("the request");
    let reasons = std::iter::repeat_n(PADDING_REASON, padding).chain(std::iter::once(filler));
    for (index, reason) in reasons.enumerate() {
        state
            .add_remission(
                &scope,
                FiscalRemission {
                    id: format!("padding.{index:03}"),
                    assessment_id: ASSESSMENT.to_owned(),
                    quantity: 1,
                    reason: "x".repeat(reason),
                    granted_at: SimTime::EPOCH,
                    source_action_id: None,
                },
            )
            .expect("a remission");
    }
    state
}

/// The serialized size of the first scope's ledger as a run starts it.
fn stored_bytes(state: FiscalState) -> usize {
    let records = state.into_records(&catalog()).expect("fiscal records");
    let state = FiscalState::from_records(&records).expect("assembled");
    serde_json::to_vec(state.ledger(&treasury_scope(0)).expect("the ledger"))
        .expect("ledger JSON")
        .len()
}

/// A state whose first treasury ledger is `MARGIN` bytes short of its budget
/// less the reserve.
fn full_state() -> FiscalState {
    let target =
        MAX_FISCAL_LEDGER_RECORD_BYTES - FISCAL_RECORD_RING_RESERVE_BYTES - CONTINUATION - MARGIN;
    let empty = stored_bytes(padded_state(0, 1));
    let each = stored_bytes(padded_state(1, 1)) - empty;
    let padding = (target - empty) / each;
    let filler = 1 + target - stored_bytes(padded_state(padding, 1));
    assert!(filler <= MAX_FISCAL_ACTION_TEXT_BYTES);
    let state = padded_state(padding, filler);
    assert_eq!(stored_bytes(state.clone()), target);
    state
}

#[test]
fn a_ledger_at_its_budget_rejects_an_action_and_refuses_a_receipt() {
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(scenario_from_state(full_state()));
    let scope = treasury_scope(0);
    let revision = ledger_revision(&canwu, &scope);

    // A remission would put the full ledger past its budget less the
    // reserve: it is rejected as its own outcome, which the reserve records.
    // An assessment of another scope settles in the same boundary.
    submit(
        &mut canwu,
        vec![
            (
                PRINCIPAL,
                draft(
                    "one.more",
                    TREASURY_AUTHORITY,
                    FiscalAction::GrantRemission {
                        remission_id: "one.more".to_owned(),
                        scope_binding_id: scope.clone(),
                        assessment_id: ASSESSMENT.to_owned(),
                        quantity: 1,
                        reason: "relief".to_owned(),
                    },
                ),
            ),
            (
                PRINCIPAL,
                assessment("elsewhere", TREASURY_AUTHORITY, &treasury_scope(1), "2026"),
            ),
        ],
    );
    let rejected = outcome(&canwu, "one.more").expect("an outcome");
    assert_eq!(rejected.disposition, FiscalActionDisposition::Rejected);
    assert!(
        rejected.reason.ends_with(FISCAL_RECORD_BUDGET_EXHAUSTED),
        "{}",
        rejected.reason
    );
    assert_eq!(
        outcome(&canwu, "elsewhere")
            .expect("an outcome")
            .disposition,
        FiscalActionDisposition::Applied
    );
    let state = fiscal_state(&canwu);
    let ledger = state.ledger(&scope).expect("the ledger");
    assert_eq!(ledger.revision, revision + 1);
    assert!(!ledger.books["2026"].remissions.contains_key("one.more"));
    let bytes = serde_json::to_vec(ledger).expect("ledger JSON").len();
    assert!(bytes + CONTINUATION <= MAX_FISCAL_LEDGER_RECORD_BYTES);

    // A receipt of the ledger's request would put it past its budget: it is
    // refused, the boundary commits, and nothing changes.
    let evidence = record_results(
        &mut canwu,
        &[(false, execution_claim("result.full", &scope, REQUEST, 10))],
    );
    let refused = refusals(&canwu).len();
    let now = canwu.time();
    enqueue_execution_receipts(
        &mut canwu,
        now,
        &[receipt_packet("receipt.full", &scope, REQUEST, evidence)],
    )
    .expect("the receipt");
    assert!(!settle(&mut canwu).is_empty());
    let refusals = refusals(&canwu);
    assert_eq!(refusals.len(), refused + 1);
    assert!(
        refusals[refused].ends_with(FISCAL_INGRESS_BUDGET_REJECTION),
        "{}",
        refusals[refused]
    );
    let state = fiscal_state(&canwu);
    assert!(
        state.ledger(&scope).expect("the ledger").books["2026"]
            .execution_receipts
            .is_empty()
    );
    assert_eq!(ledger_revision(&canwu, &scope), revision + 1);
    fixture.assert_restore_and_replay(&canwu);
}

/// The serialized bytes of the first treasury ledger, without its record's
/// evidence continuation.
fn ledger_json(canwu: &Canwu) -> usize {
    serde_json::to_vec(
        fiscal_state(canwu)
            .ledger(&treasury_scope(0))
            .expect("the ledger"),
    )
    .expect("ledger JSON")
    .len()
}

/// Records an execution result under `id` and settles a receipt of the
/// ledger's request that cites it.
fn receipt_citing(canwu: &mut Canwu, id: &str) {
    let scope = treasury_scope(0);
    let evidence = record_results(canwu, &[(false, execution_claim(id, &scope, REQUEST, 10))]);
    let now = canwu.time();
    enqueue_execution_receipts(
        canwu,
        now,
        &[receipt_packet("receipt.cited", &scope, REQUEST, evidence)],
    )
    .expect("the receipt");
    settle(canwu);
}

/// A receipt whose evidence record has a 4 KB ID adds as much to its ledger's
/// evidence continuation. Near the budget, it is refused for the payload it
/// would make, and the run restores; an estimate of the continuation would
/// have let it settle into a ledger that a restore refuses.
#[test]
fn a_citation_with_a_long_id_is_measured_and_the_run_restores() {
    let id = format!("result.{}", "x".repeat(4_000));
    let fixture = Fixture::new();
    // What the receipt adds to the ledger itself.
    let mut small = fixture.runtime(scenario_from_state(padded_state(0, 1)));
    let before = ledger_json(&small);
    receipt_citing(&mut small, &id);
    let growth = ledger_json(&small) - before;

    // A ledger that leaves room for the receipt and an estimate of 600
    // bytes for its citation, not for the citation itself.
    let target = MAX_FISCAL_LEDGER_RECORD_BYTES
        - FISCAL_RECORD_RING_RESERVE_BYTES
        - CONTINUATION
        - 600
        - growth
        - 16;
    let empty = stored_bytes(padded_state(0, 1));
    let each = stored_bytes(padded_state(1, 1)) - empty;
    let padding = (target - empty) / each;
    let filler = 1 + target - stored_bytes(padded_state(padding, 1));
    let mut canwu = fixture.runtime(scenario_from_state(padded_state(padding, filler)));
    let refused = refusals(&canwu).len();
    receipt_citing(&mut canwu, &id);
    let refusals = refusals(&canwu);
    assert_eq!(refusals.len(), refused + 1);
    assert!(
        refusals[refused].ends_with(FISCAL_INGRESS_BUDGET_REJECTION),
        "{}",
        refusals[refused]
    );
    let payload = canwu
        .domain_record(&canwu_fiscal::fiscal_ledger_reference(&treasury_scope(0)).into_untyped())
        .map(|record| serde_json::to_vec(&record.payload).expect("payload").len())
        .expect("the ledger record");
    assert!(payload <= MAX_FISCAL_LEDGER_RECORD_BYTES);
    Canwu::from_snapshot_json_with_plugins(
        &canwu.snapshot_json().expect("snapshot"),
        &fixture.plugins(),
    )
    .expect("the run restores");
}
