//! Downstream gap G-41: a boundary that admits no fiscal ingress settles
//! without decoding the fiscal records.
//!
//! The cost is a measurement and lives in the fiscal boundary profile
//! (`benchmarks/performance-harness`, `fiscal-profile`). This fixture pins the
//! contract that makes skipping the decode safe: a fiscal record is validated
//! where it enters a run, an idle boundary proposes nothing, and the only work
//! that does not depend on fiscal ingress, deriving the fields of a state no
//! boundary has written yet, still happens.

mod support;

use canwu_api::{BoundaryReceipt, Canwu, SimTime};
use canwu_fiscal::{FiscalActionDisposition, FiscalAssessment, FiscalPaymentForm, FiscalState};
use support::{
    Fixture, PRINCIPAL, RULE, TREASURY_AUTHORITY, assessment, base_state, catalog, enqueue_tick,
    fiscal_record_version, fiscal_report, fiscal_state, scenario_from_records, scenario_from_state,
    settle, submit, treasury_scope,
};

/// A state with one authored assessment and no derived field computed.
fn authored_state() -> FiscalState {
    let mut state = base_state();
    state.assessments.insert(
        "authored.assessment".to_owned(),
        FiscalAssessment {
            id: "authored.assessment".to_owned(),
            rule_id: RULE.to_owned(),
            scope_binding_id: treasury_scope(0),
            accounting_cycle_id: "cycle.authored".to_owned(),
            quantity: 500,
            unit: "grain".to_owned(),
            payment_form: FiscalPaymentForm::Grain,
            commutation_quote: None,
            created_at: SimTime::EPOCH,
        },
    );
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
        .into_record(&catalog)
        .expect("fiscal state record");
    invalid.payload["procedure_revision"] = serde_json::json!(0);
    assert!(
        fixture
            .try_runtime(scenario_from_records(
                catalog.into_record().expect("catalog record"),
                invalid,
            ))
            .is_err(),
        "an invalid fiscal state must be refused at activation"
    );

    let mut canwu = fixture.runtime(scenario_from_state(authored_state()));
    assert!(fiscal_state(&canwu).aggregates.is_empty());
    let authored_version = fiscal_record_version(&canwu);

    // The derived fields of an authored state do not wait for fiscal ingress:
    // the first event-bearing boundary computes them and reports them, as it
    // always has. The report is its summary and its one part.
    assert_eq!(changes(&idle_boundary(&mut canwu)), (1, 2));
    let derived = fiscal_state(&canwu);
    assert_eq!(derived.aggregates.len(), 1);
    assert_eq!(fiscal_record_version(&canwu), authored_version + 1);
    assert_eq!(fiscal_report(&canwu, PRINCIPAL).projection.facts.len(), 1);

    // From then on a boundary without fiscal ingress changes no record,
    // publishes no report, and leaves the procedure revision alone.
    for _ in 0..3 {
        assert_eq!(changes(&idle_boundary(&mut canwu)), (0, 0));
    }
    assert_eq!(fiscal_state(&canwu), derived);
    assert_eq!(fiscal_record_version(&canwu), authored_version + 1);

    // Fiscal ingress still runs every system: the action settles, the
    // aggregate follows in the same boundary, and the observer gets the part
    // that changed with a new summary.
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
        settled.action_outcomes["action.assess"].disposition,
        FiscalActionDisposition::Applied
    );
    assert_eq!(settled.aggregates.len(), 2);
    assert_eq!(fiscal_report(&canwu, PRINCIPAL).projection.facts.len(), 2);

    let settled_version = fiscal_record_version(&canwu);
    for _ in 0..3 {
        assert_eq!(changes(&idle_boundary(&mut canwu)), (0, 0));
    }
    assert_eq!(fiscal_state(&canwu), settled);
    assert_eq!(fiscal_record_version(&canwu), settled_version);

    fixture.assert_restore_and_replay(&canwu);
}
