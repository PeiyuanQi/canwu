//! Each hot accounting cycle an observer sees takes a slot of its report's
//! lines, and leaves it when its books are archived.
//!
//! A cycle whose last visible book is archived leaves the report: its parts
//! are published empty, on the lines they held, the summary no longer lists
//! it, and its slot is free for the next cycle. A report shows at most
//! `MAX_FISCAL_REPORT_CYCLE_SLOTS` cycles at once; one past them is counted
//! as omitted and takes the first slot that frees, in a drain boundary at the
//! same instant. A part whose rows come from the scopes of two offices is
//! published like any other, citing each ledger it read once.

mod support;

use canwu_api::{Canwu, EvidenceRef, KnowledgeRecord, SimTime, SimulationGranularity};
use canwu_fiscal::{
    FiscalAdoptionStage, FiscalAdoptionState, FiscalAssessment, FiscalMechanism,
    FiscalObserverBinding, FiscalPaymentForm, FiscalReport, FiscalReportRecord, FiscalScopeBinding,
    FiscalState, MAX_FISCAL_REPORT_CYCLE_SLOTS, compute_projection, fiscal_reports,
};
use std::collections::{BTreeMap, BTreeSet};
use support::{
    Fixture, GRANARY_AUTHORITY, GRANARY_PRINCIPAL, GRANARY_SCOPE, MemoryArchiveStore, OBSERVER,
    PRINCIPAL, RULE, TREASURY_AUTHORITY, archive, assessment, close, fiscal_state, granary,
    observer_binding, published_report_records, scenario, submit, treasury, treasury_scope,
};

fn report(canwu: &Canwu, observer: &str) -> FiscalReport {
    let viewer = canwu.viewer_for_actor(PRINCIPAL).expect("viewer");
    fiscal_reports(&viewer)
        .expect("reports")
        .remove(observer)
        .unwrap_or_else(|| panic!("{observer} holds a report"))
}

fn assert_report_matches(canwu: &Canwu, observer: &str) -> FiscalReport {
    let report = report(canwu, observer);
    assert!(report.is_complete());
    assert_eq!(
        Some(report.projection.clone()),
        compute_projection(&fiscal_state(canwu), observer, report.projection.as_of)
            .expect("projection")
    );
    report
}

fn cycles(report: &FiscalReport) -> BTreeSet<String> {
    report
        .projection
        .facts
        .values()
        .map(|fact| fact.accounting_cycle_id.clone())
        .collect()
}

/// The report records published to the principal since `seen`.
fn published_since(canwu: &Canwu, seen: usize) -> Vec<(KnowledgeRecord, FiscalReportRecord)> {
    published_report_records(canwu, PRINCIPAL)[seen..].to_vec()
}

#[test]
fn an_archived_cycle_leaves_the_report_and_frees_its_slot() {
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(scenario(|_| {}));
    let store = MemoryArchiveStore::default();
    let assess = |action: &str, cycle: &str| {
        (
            PRINCIPAL,
            assessment(action, TREASURY_AUTHORITY, &treasury_scope(0), cycle),
        )
    };
    submit(&mut canwu, vec![assess("a.2026", "2026")]);
    submit(&mut canwu, vec![assess("a.2027", "2027")]);
    let slots = |canwu: &Canwu| -> BTreeMap<String, u16> {
        fiscal_state(canwu).observers[OBSERVER]
            .report
            .cycles
            .iter()
            .map(|(cycle, slot)| (cycle.clone(), slot.slot))
            .collect()
    };
    assert_eq!(
        slots(&canwu),
        BTreeMap::from([("2026".to_owned(), 0), ("2027".to_owned(), 1)])
    );
    assert_eq!(
        cycles(&assert_report_matches(&canwu, OBSERVER)),
        BTreeSet::from(["2026".to_owned(), "2027".to_owned()])
    );
    let part_2026 = published_report_records(&canwu, PRINCIPAL)
        .into_iter()
        .rev()
        .find(|(_, record)| {
            matches!(record, FiscalReportRecord::Part(part) if part.accounting_cycle_id == "2026")
        })
        .expect("the part of 2026")
        .0;

    // The book of 2026 is closed and archived: the cycle leaves the
    // treasury's hot cycles, and its part is published empty on its line,
    // superseding the part it held, with a summary that no longer lists it.
    submit(
        &mut canwu,
        vec![(PRINCIPAL, close("close.2026", &treasury_scope(0), "2026"))],
    );
    let seen = published_report_records(&canwu, PRINCIPAL).len();
    archive(&mut canwu, &store, 64);
    let withdrawn = published_since(&canwu, seen);
    assert_eq!(withdrawn.len(), 2);
    let FiscalReportRecord::Part(empty) = &withdrawn[1].1 else {
        panic!("a part follows the summary");
    };
    assert_eq!(empty.accounting_cycle_id, "2026");
    assert!(empty.rows.is_empty());
    assert_eq!(withdrawn[1].0.supersedes, vec![part_2026.id]);
    let FiscalReportRecord::Summary(summary) = &withdrawn[0].1 else {
        panic!("a summary first");
    };
    assert_eq!(summary.cycles, BTreeMap::from([("2027".to_owned(), 1)]));
    assert_eq!(slots(&canwu), BTreeMap::from([("2027".to_owned(), 1)]));
    assert_eq!(
        cycles(&assert_report_matches(&canwu, OBSERVER)),
        BTreeSet::from(["2027".to_owned()])
    );

    // The next cycle takes the free slot, and its part supersedes the empty
    // record on the line.
    submit(&mut canwu, vec![assess("a.2028", "2028")]);
    assert_eq!(
        slots(&canwu),
        BTreeMap::from([("2027".to_owned(), 1), ("2028".to_owned(), 0)])
    );
    let reused = published_report_records(&canwu, PRINCIPAL)
        .into_iter()
        .rev()
        .find(|(_, record)| {
            matches!(record, FiscalReportRecord::Part(part) if part.accounting_cycle_id == "2028")
        })
        .expect("the part of 2028");
    assert_eq!(reused.0.supersedes, vec![withdrawn[1].0.id]);
    assert_eq!(
        cycles(&assert_report_matches(&canwu, OBSERVER)),
        BTreeSet::from(["2027".to_owned(), "2028".to_owned()])
    );
    fixture.assert_restore_and_replay(&canwu);
}

fn authored(state: &mut FiscalState, scope: &str, cycle: &str) {
    state
        .add_assessment(FiscalAssessment {
            id: format!("authored.{scope}.{cycle}"),
            rule_id: RULE.to_owned(),
            scope_binding_id: scope.to_owned(),
            accounting_cycle_id: cycle.to_owned(),
            quantity: 100,
            unit: "grain".to_owned(),
            payment_form: FiscalPaymentForm::Grain,
            commutation_quote: None,
            created_at: SimTime::EPOCH,
            source_action_id: None,
        })
        .expect("an authored assessment");
}

#[test]
fn a_cycle_past_the_slots_is_omitted_until_one_frees() {
    const BOTH: &str = "observer.both";
    let granary_cycles = 25;
    let treasury_cycles = MAX_FISCAL_REPORT_CYCLE_SLOTS + 1 - granary_cycles;
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(scenario(|state| {
        state.add_observer(FiscalObserverBinding {
            id: BOTH.to_owned(),
            ..observer_binding(BOTH, PRINCIPAL, BTreeSet::from([treasury(), granary()]))
        });
        for cycle in 0..granary_cycles {
            authored(state, GRANARY_SCOPE, &format!("g.{cycle:02}"));
        }
        for cycle in 0..treasury_cycles {
            authored(state, &treasury_scope(0), &format!("t.{cycle:02}"));
        }
    }));
    let store = MemoryArchiveStore::default();

    // The first boundary publishes the report of every slot; the last cycle
    // in order has none, and the summary counts it.
    submit(&mut canwu, Vec::new());
    support::enqueue_tick(&mut canwu);
    support::settle(&mut canwu);
    let last = format!("t.{:02}", treasury_cycles - 1);
    let first = assert_report_matches(&canwu, BOTH);
    assert_eq!(first.omitted_cycles, 1);
    assert_eq!(cycles(&first).len(), MAX_FISCAL_REPORT_CYCLE_SLOTS);
    assert!(!cycles(&first).contains(&last));

    // Archiving one of the granary's cycles frees a slot: the boundary
    // withdraws the cycle, and a drain boundary at the same instant gives
    // the omitted cycle the free slot.
    submit(
        &mut canwu,
        vec![(GRANARY_PRINCIPAL, {
            let mut draft = close("close.g", GRANARY_SCOPE, "g.00");
            draft.binding = GRANARY_AUTHORITY.to_owned();
            draft
        })],
    );
    let at = canwu.time();
    archive(&mut canwu, &store, 64);
    let after = assert_report_matches(&canwu, BOTH);
    assert_eq!(after.omitted_cycles, 0);
    assert!(cycles(&after).contains(&last));
    assert!(!cycles(&after).contains("g.00"));
    assert!(fiscal_state(&canwu).report_queue.is_empty());
    assert!(after.projection.as_of >= at);
    fixture.assert_restore_and_replay(&canwu);
}

/// A part's rows follow the observer's offices, and each office's scopes in
/// rank order. With a granary scope whose ID sorts after the treasury's
/// scopes, the ledgers of one part are out of ID order whichever office
/// comes first; the part cites each once, sorted, and is published.
#[test]
fn a_part_over_the_scopes_of_two_offices_is_published() {
    const BOTH: &str = "observer.both";
    const LAST_SCOPE: &str = "scope.zz.granary";
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(scenario(|state| {
        state.add_scope(FiscalScopeBinding {
            id: LAST_SCOPE.to_owned(),
            institution: granary(),
            jurisdiction_id: "region.a".to_owned(),
            subject_scope: "registered_land".to_owned(),
            mechanism: FiscalMechanism::LandTax,
            authoritative_granularity: SimulationGranularity::Aggregate,
        });
        state
            .add_adoption(FiscalAdoptionState {
                id: "adopt.zz.granary".to_owned(),
                rule_id: RULE.to_owned(),
                scope_binding_id: LAST_SCOPE.to_owned(),
                stage: FiscalAdoptionStage::Implemented,
                generation: 1,
                changed_at: SimTime::EPOCH,
                source_action_id: None,
            })
            .expect("an adoption of the new scope");
        state.add_observer(observer_binding(
            BOTH,
            PRINCIPAL,
            BTreeSet::from([treasury(), granary()]),
        ));
        for scope in [
            treasury_scope(0),
            GRANARY_SCOPE.to_owned(),
            LAST_SCOPE.to_owned(),
        ] {
            authored(state, &scope, "2026");
        }
    }));
    support::enqueue_tick(&mut canwu);
    support::settle(&mut canwu);
    let report = assert_report_matches(&canwu, BOTH);
    assert_eq!(report.projection.facts.len(), 3);
    let (record, _) = published_report_records(&canwu, PRINCIPAL)
        .into_iter()
        .rev()
        .find(|(_, record)| {
            matches!(record, FiscalReportRecord::Part(part) if part.observer_id == BOTH)
        })
        .expect("the part");
    let cited: Vec<String> = record
        .origin
        .evidence
        .iter()
        .filter_map(|evidence| match evidence {
            EvidenceRef::DomainRecordVersion(version) => Some(version.record.id.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        cited,
        vec![
            GRANARY_SCOPE.to_owned(),
            treasury_scope(0),
            LAST_SCOPE.to_owned()
        ]
    );
    fixture.assert_restore_and_replay(&canwu);
}
