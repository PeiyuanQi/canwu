//! Downstream gap G-41: a boundary that admits no fiscal ingress settles
//! without decoding the fiscal records.
//!
//! The cost is a measurement and lives in the fiscal boundary profile
//! (`benchmarks/performance-harness`, `fiscal-profile`). This fixture pins the
//! contract that makes skipping the decode safe: a fiscal record is validated
//! where it enters a run, an idle boundary proposes nothing, and the only work
//! that does not depend on fiscal ingress, publishing the report work the
//! queue holds (here the first report of an authored observer), still
//! happens.

mod support;

use canwu_api::{BoundaryReceipt, Canwu, SimTime};
use canwu_fiscal::{FiscalActionDisposition, FiscalAssessment, FiscalPaymentForm, FiscalState};
use support::{
    Fixture, PRINCIPAL, RULE, TREASURY_AUTHORITY, assessment, base_state, catalog, enqueue_tick,
    fiscal_record_versions, fiscal_report, fiscal_state, outcome, scenario_from_records,
    scenario_from_state, settle, submit, treasury_scope,
};

/// A state with one authored assessment and no derived field computed.
fn authored_state() -> FiscalState {
    let mut state = base_state();
    state
        .add_assessment(FiscalAssessment {
            id: "authored.assessment".to_owned(),
            rule_id: RULE.to_owned(),
            scope_binding_id: treasury_scope(0),
            accounting_cycle_id: "cycle.authored".to_owned(),
            quantity: 500,
            unit: "grain".to_owned(),
            payment_form: FiscalPaymentForm::Grain,
            commutation_quote: None,
            created_at: SimTime::EPOCH,
            source_action_id: None,
        })
        .expect("the authored assessment");
    state
}

/// Settles a boundary that admits ingress of another plugin only.
fn idle_boundary(canwu: &mut Canwu) -> Vec<BoundaryReceipt> {
    enqueue_tick(canwu);
    let receipts = settle(canwu);
    assert!(!receipts.is_empty(), "the tick must open a boundary");
    receipts
}

fn changes(receipts: &[BoundaryReceipt]) -> (usize, usize) {
    receipts.iter().fold((0, 0), |(records, reports), receipt| {
        (
            records + receipt.record_change_count,
            reports + receipt.knowledge_record_count,
        )
    })
}

#[test]
fn gap_g41_fiscal_idle_early_exit() {
    let fixture = Fixture::new();

    // Idle boundaries no longer validate the fiscal records, so a record that
    // fails validation is refused when the run is created, before it can settle
    // a boundary.
    let catalog = catalog();
    let mut invalid = authored_state()
        .into_records(&catalog)
        .expect("fiscal records");
    let ledger = invalid
        .iter_mut()
        .find(|record| record.reference.kind.name == "ledger")
        .expect("a ledger record");
    ledger.payload["revision"] = serde_json::json!(0);
    assert!(
        fixture
            .try_runtime(scenario_from_records(
                catalog.into_record().expect("catalog record"),
                invalid,
            ))
            .is_err(),
        "an invalid fiscal record must be refused at activation"
    );
    // So is a state that approves an execution evidence kind the plugin was
    // not configured to read: settlement could read none of its receipts.
    let mut unread_evidence = authored_state();
    unread_evidence
        .root
        .execution_evidence_kinds
        .insert(canwu_api::DomainRecordKind::new(
            "fixture.execution",
            "unread",
        ));
    let error = fixture
        .try_runtime(scenario_from_state(unread_evidence))
        .err()
        .expect("an evidence kind the plugin does not read must be refused at activation");
    assert!(
        error.message.contains("not declared by FiscalPlugin::new"),
        "{}",
        error.message
    );

    let mut canwu = fixture.runtime(scenario_from_state(authored_state()));
    assert_eq!(fiscal_state(&canwu).report_queue.len(), 1);

    // The first report of an authored observer does not wait for fiscal
    // ingress: the first event-bearing boundary publishes the work the queue
    // holds, its summary and its one part, and writes the observer record,
    // the emptied queue page, the report schedule, and the root's earliest
    // due time.
    assert_eq!(changes(&idle_boundary(&mut canwu)), (4, 2));
    let published = fiscal_state(&canwu);
    assert!(published.report_queue.is_empty());
    assert_eq!(fiscal_report(&canwu, PRINCIPAL).projection.facts.len(), 1);
    let versions = fiscal_record_versions(&canwu);

    // From then on a boundary without fiscal ingress changes no record and
    // publishes no report.
    for _ in 0..3 {
        assert_eq!(changes(&idle_boundary(&mut canwu)), (0, 0));
    }
    assert_eq!(fiscal_state(&canwu), published);
    assert_eq!(fiscal_record_versions(&canwu), versions);

    // Fiscal ingress still runs every system: the action settles, and the
    // observer gets the part that changed with a new summary in the same
    // boundary.
    let action = assessment(
        "action.assess",
        TREASURY_AUTHORITY,
        &treasury_scope(1),
        "cycle.1",
    );
    let (_, reports) = changes(&submit(&mut canwu, vec![(PRINCIPAL, action)]));
    assert_eq!(reports, 2);
    let settled = fiscal_state(&canwu);
    assert_eq!(
        outcome(&canwu, "action.assess")
            .expect("the assessment's outcome")
            .disposition,
        FiscalActionDisposition::Applied
    );
    assert_eq!(settled.assessments().count(), 2);
    assert_eq!(fiscal_report(&canwu, PRINCIPAL).projection.facts.len(), 2);

    let versions = fiscal_record_versions(&canwu);
    for _ in 0..3 {
        assert_eq!(changes(&idle_boundary(&mut canwu)), (0, 0));
    }
    assert_eq!(fiscal_state(&canwu), settled);
    assert_eq!(fiscal_record_versions(&canwu), versions);

    fixture.assert_restore_and_replay(&canwu);
}
