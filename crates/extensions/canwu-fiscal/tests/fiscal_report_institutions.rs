//! Institution granularity: an observer at
//! `FiscalReportGranularity::Institution` has one row for each visible
//! institution and each mechanism of its scopes, whose facts are the
//! institution's hot books of the cycle of that mechanism summed per unit
//! and payment form, rendered at the observer's confidence. A part of such
//! rows cites the current version of each office its rows belong to and
//! counts the books it summed. A change of one book republishes the one part
//! that holds its institution's row, and a change below the observer's
//! precision publishes nothing; the report always equals what the records
//! derive at the boundary that published it.

mod support;

use canwu_api::{
    Canwu, DomainRecordVersionRef, EvidenceRef, KnowledgeRecord, PersonId, SimulationGranularity,
};
use canwu_fiscal::{
    FiscalAction, FiscalActionDisposition, FiscalMechanism, FiscalObserverBinding, FiscalReport,
    FiscalReportGranularity, FiscalReportPart, FiscalReportRecord, FiscalScopeBinding,
    compute_projection, fiscal_office_reference, fiscal_reports,
};
use std::collections::BTreeSet;
use support::{
    AUDITOR, Fixture, PRINCIPAL, TREASURY_AUTHORITY, assessment, fiscal_state, granary,
    observer_binding, outcome, published_report_records, scenario, submit, treasury,
    treasury_scope,
};

/// The auditor's report of both institutions, one row per institution and
/// mechanism, at a confidence that renders amounts to a tenth of their
/// magnitude.
const COURT: &str = "observer.court";
/// A treasury scope of a second mechanism, which holds no book: the
/// treasury has two rows, its second always empty.
const TRIBUTE_SCOPE: &str = "scope.treasury.tribute";

fn court_scenario() -> canwu_api::Scenario {
    scenario(|state| {
        state.add_scope(FiscalScopeBinding {
            id: TRIBUTE_SCOPE.to_owned(),
            institution: treasury(),
            jurisdiction_id: "region.a".to_owned(),
            subject_scope: "registered_land".to_owned(),
            mechanism: FiscalMechanism::GrainTribute,
            authoritative_granularity: SimulationGranularity::Aggregate,
        });
        state.add_observer(FiscalObserverBinding {
            granularity: FiscalReportGranularity::Institution,
            confidence_per_mille: 850,
            ..observer_binding(COURT, AUDITOR, BTreeSet::from([treasury(), granary()]))
        });
    })
}

fn assess(action_id: &str, scope: usize, quantity: u64) -> (PersonId, support::Draft) {
    let mut draft = assessment(action_id, TREASURY_AUTHORITY, &treasury_scope(scope), "c1");
    if let FiscalAction::OpenAssessment {
        quantity: assessed, ..
    } = &mut draft.action
    {
        *assessed = quantity;
    }
    (PRINCIPAL, draft)
}

fn court_report(canwu: &Canwu) -> FiscalReport {
    let viewer = canwu.viewer_for_actor(AUDITOR).expect("viewer");
    fiscal_reports(&viewer)
        .expect("reports")
        .remove(COURT)
        .expect("the court's report")
}

/// The court's report records published so far.
fn published(canwu: &Canwu) -> Vec<(KnowledgeRecord, FiscalReportRecord)> {
    published_report_records(canwu, AUDITOR)
}

/// The last part published to the court, with the record that carried it.
fn last_part(canwu: &Canwu) -> (KnowledgeRecord, FiscalReportPart) {
    published(canwu)
        .into_iter()
        .rev()
        .find_map(|(record, payload)| match payload {
            FiscalReportRecord::Part(part) => Some((record, part)),
            FiscalReportRecord::Summary(_) => None,
        })
        .expect("a part")
}

fn treasury_office_version(canwu: &Canwu) -> DomainRecordVersionRef {
    canwu
        .current_domain_record_version(fiscal_office_reference(&treasury()).as_untyped())
        .expect("office version read")
        .expect("the treasury's office")
}

/// The court's report is complete and is what the records derive now.
fn assert_current(canwu: &Canwu) -> FiscalReport {
    let report = court_report(canwu);
    assert!(report.is_complete());
    let derived = compute_projection(&fiscal_state(canwu), COURT, report.projection.as_of)
        .expect("projection")
        .expect("the court");
    assert_eq!(report.projection.facts, derived.facts);
    report
}

#[test]
fn a_court_is_told_the_sums_of_its_institutions_books() {
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(court_scenario());
    let state = fiscal_state(&canwu);
    let rows = &state.observers[COURT].rows;
    assert_eq!(
        rows[0].mechanisms,
        [FiscalMechanism::LandTax, FiscalMechanism::GrainTribute]
    );
    assert_eq!(
        (rows[1].first_row, state.observers[COURT].row_count()),
        (2, 3)
    );

    // Two books of the treasury: one row, their sum at the court's
    // precision, citing the treasury's office and counting two books.
    submit(&mut canwu, vec![assess("first", 0, 1_000)]);
    submit(&mut canwu, vec![assess("second", 1, 1_000)]);
    let report = assert_current(&canwu);
    assert_eq!(report.projection.facts.len(), 1);
    let fact = report.projection.facts.values().next().expect("one fact");
    assert_eq!(
        (
            fact.institution.clone(),
            fact.mechanism,
            fact.scope_binding_id.as_str()
        ),
        (treasury(), FiscalMechanism::LandTax, "")
    );
    assert_eq!(
        (fact.assessed.minimum, fact.assessed.maximum),
        (2_000, 2_099)
    );
    let (record, part) = last_part(&canwu);
    assert_eq!(part.books_summed, Some(2));
    assert_eq!(part.rows.len(), 1);
    assert_eq!(
        record.origin.evidence,
        [EvidenceRef::DomainRecordVersion(treasury_office_version(
            &canwu
        ))]
    );

    // A book below the court's precision changes nothing it sees: nothing
    // is published to it, and the part it holds still counts two books.
    let before = published(&canwu).len();
    submit(&mut canwu, vec![assess("small", 2, 1)]);
    assert_eq!(published(&canwu).len(), before);
    assert_eq!(court_report(&canwu), report);
    assert_eq!(last_part(&canwu).1.books_summed, Some(2));

    // A change of one book it can see republishes the one part, with the
    // summary, summed over every book.
    submit(&mut canwu, vec![assess("third", 3, 1_000)]);
    assert_eq!(published(&canwu).len(), before + 2);
    let report = assert_current(&canwu);
    let fact = report.projection.facts.values().next().expect("one fact");
    assert_eq!(
        (fact.assessed.minimum, fact.assessed.maximum),
        (3_000, 3_099)
    );
    assert_eq!(last_part(&canwu).1.books_summed, Some(4));

    // The granary's row joins the same part; its office is cited too.
    let granary_assessment = (
        support::GRANARY_PRINCIPAL,
        assessment(
            "granary",
            support::GRANARY_AUTHORITY,
            support::GRANARY_SCOPE,
            "c1",
        ),
    );
    submit(&mut canwu, vec![granary_assessment]);
    let report = assert_current(&canwu);
    assert_eq!(report.projection.facts.len(), 2);
    let (record, part) = last_part(&canwu);
    assert_eq!(part.books_summed, Some(5));
    assert_eq!(
        part.rows.iter().map(|row| row.row).collect::<Vec<_>>(),
        [0, 2]
    );
    assert_eq!(record.origin.evidence.len(), 2);

    fixture.assert_restore_and_replay(&canwu);
}

/// Each book holds an amount the scope's own row reports; their sum over
/// the institution can pass `u64::MAX`. The court's row saturates there
/// instead of failing every boundary that would publish it: both
/// assessments settle, the report is published and equals its projection,
/// and the run restores and replays.
#[test]
fn an_institution_sum_past_the_largest_amount_saturates() {
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(court_scenario());
    let half = u64::MAX / 2 + 1;
    submit(&mut canwu, vec![assess("first", 0, half)]);
    submit(&mut canwu, vec![assess("second", 1, half)]);
    for action in ["first", "second"] {
        assert_eq!(
            outcome(&canwu, action).expect("an outcome").disposition,
            FiscalActionDisposition::Applied
        );
    }
    let report = assert_current(&canwu);
    let fact = report.projection.facts.values().next().expect("one fact");
    assert_eq!(fact.assessed.maximum, u64::MAX);
    assert_eq!(fact.outstanding.maximum, u64::MAX);
    assert_eq!(last_part(&canwu).1.books_summed, Some(2));
    fixture.assert_restore_and_replay(&canwu);
}
