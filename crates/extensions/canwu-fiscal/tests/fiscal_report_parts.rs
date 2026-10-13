//! An observer's fiscal report is a summary and parts, each its own line of
//! knowledge records, so its size is not bounded by one record.
//!
//! The first run reports several hundred aggregates to one observer, well
//! past what one 65,536-byte record held. Every boundary commits, the report
//! the observer assembles equals what the fiscal state derives, a boundary
//! republishes only the parts whose facts changed, each superseding exactly
//! the record before it on its line, and restore and exact replay agree. The
//! second run changes more parts in one boundary than a boundary publishes:
//! the rest follow in the next fiscal boundary, and until then the summary
//! names the parts that are pending. The last runs cover what cannot be
//! published at all: a report for an observer who has died, an observer who
//! does not exist, and a report plan that came with a scenario.

mod support;

use canwu_api::{
    BoundaryReceipt, Canwu, ErrorCode, KnowledgeHolderRef, KnowledgeOrigin, KnowledgeRecord,
    KnowledgeRecordId, KnowledgeSubject, KnowledgeSubjectTarget, PersonId, SimTime,
};
use canwu_fiscal::{
    FISCAL_REPORT_FACTS_PER_PART, FiscalAction, FiscalActionDisposition, FiscalAssessment,
    FiscalAuditSeverity, FiscalObserverBinding, FiscalPaymentForm, FiscalReport, FiscalReportPlan,
    FiscalReportRecord, FiscalState, MAX_FISCAL_OBSERVERS, MAX_FISCAL_REPORT_RECORDS_PER_BOUNDARY,
    compute_projections, fiscal_report_knowledge_schema_id, fiscal_reports, fiscal_state_reference,
};
use std::collections::{BTreeMap, BTreeSet};
use support::{
    AUDITOR, DEPUTY, Draft, Fixture, GRANARY_AUTHORITY, GRANARY_PRINCIPAL, GRANARY_SCOPE, OBSERVER,
    PRINCIPAL, RULE, TREASURY_AUTHORITY, TREASURY_SCOPES, assessment, base_state, draft,
    enqueue_life, enqueue_requests, enqueue_tick, fiscal_state, granary, procedure_revision,
    published_report_records, scenario, scenario_from_state, settle, submit, treasury,
    treasury_scope,
};

/// The second binding of the treasury's observer: the same actor, the
/// granary, and a lower confidence.
const SECOND_BINDING: &str = "observer.principal.granary";
const AUDITOR_BINDING: &str = "observer.auditor";
/// One 65,536-byte report record held about 196 facts.
const FACTS_IN_ONE_OLD_RECORD: usize = 196;

fn observer(
    id: &str,
    actor: PersonId,
    institution: canwu_api::EntityRef,
    confidence: u16,
) -> FiscalObserverBinding {
    FiscalObserverBinding {
        id: id.to_owned(),
        actor,
        knowledge_holder: canwu_api::KnowledgeHolderRef::Person(actor),
        visible_institutions: BTreeSet::from([institution]),
        confidence_per_mille: confidence,
    }
}

/// Installs one assessment of every treasury scope for each of `cycles`
/// accounting cycles, all created when the run starts.
fn author_treasury_assessments(state: &mut FiscalState, cycles: usize) {
    for scope in 0..TREASURY_SCOPES {
        for cycle in 0..cycles {
            let id = format!("authored.{scope}.{cycle:03}");
            state.assessments.insert(
                id.clone(),
                FiscalAssessment {
                    id,
                    rule_id: RULE.to_owned(),
                    scope_binding_id: treasury_scope(scope),
                    accounting_cycle_id: format!("authored.{cycle:03}"),
                    quantity: 500,
                    unit: "grain".to_owned(),
                    payment_form: FiscalPaymentForm::Grain,
                    commutation_quote: None,
                    created_at: SimTime::EPOCH,
                },
            );
        }
    }
}

fn treasury_assessment(action_id: &str, scope: usize, cycle: &str) -> (PersonId, Draft) {
    (
        PRINCIPAL,
        assessment(action_id, TREASURY_AUTHORITY, &treasury_scope(scope), cycle),
    )
}

fn granary_assessment(action_id: &str, cycle: &str) -> (PersonId, Draft) {
    (
        GRANARY_PRINCIPAL,
        assessment(action_id, GRANARY_AUTHORITY, GRANARY_SCOPE, cycle),
    )
}

fn published(receipts: &[BoundaryReceipt]) -> usize {
    receipts
        .iter()
        .map(|receipt| receipt.knowledge_record_count)
        .sum()
}

fn reports_of(canwu: &Canwu, actor: PersonId) -> BTreeMap<String, FiscalReport> {
    let viewer = canwu.viewer_for_actor(actor).expect("observer viewer");
    fiscal_reports(&viewer).expect("observer reports")
}

/// Asserts that every report the run's observers hold is complete and equals
/// what the fiscal state derives for its binding, and returns the reports by
/// observer binding ID.
fn assert_reports_match_state(canwu: &Canwu) -> BTreeMap<String, FiscalReport> {
    let state = fiscal_state(canwu);
    let mut reports = BTreeMap::new();
    for actor in state
        .observer_bindings
        .values()
        .map(|binding| binding.actor)
        .collect::<BTreeSet<_>>()
    {
        reports.extend(reports_of(canwu, actor));
    }
    assert_eq!(
        reports.keys().collect::<Vec<_>>(),
        state.observer_bindings.keys().collect::<Vec<_>>()
    );
    for (observer_id, report) in &reports {
        assert!(report.is_complete(), "{observer_id} is complete");
        assert_eq!(
            report.projection,
            compute_projections(&state, report.projection.as_of)[observer_id],
            "{observer_id} reports what the state derives"
        );
    }
    reports
}

/// The report line a record is on: its observer binding, and its part number
/// or `None` for the summary.
fn line(record: &FiscalReportRecord) -> (String, Option<u32>) {
    match record {
        FiscalReportRecord::Summary(summary) => (summary.observer_id.clone(), None),
        FiscalReportRecord::Part(part) => (part.observer_id.clone(), Some(part.part)),
    }
}

/// Follows the report records published to `actor` since `seen`, asserts that
/// each supersedes exactly the record before it on its line, and returns the
/// lines published, in order.
fn published_lines(
    canwu: &Canwu,
    actor: PersonId,
    seen: &mut usize,
    heads: &mut BTreeMap<(String, Option<u32>), KnowledgeRecord>,
) -> Vec<(String, Option<u32>)> {
    let records = published_report_records(canwu, actor);
    let lines = records[*seen..]
        .iter()
        .map(|(record, payload)| {
            let line = line(payload);
            let expected: Vec<_> = heads.get(&line).map(|head| head.id).into_iter().collect();
            assert_eq!(record.supersedes, expected, "line {line:?}");
            heads.insert(line.clone(), record.clone());
            line
        })
        .collect();
    *seen = records.len();
    lines
}

fn treasury_lines(parts: impl IntoIterator<Item = u32>) -> Vec<(String, Option<u32>)> {
    std::iter::once(None)
        .chain(parts.into_iter().map(Some))
        .map(|part| (OBSERVER.to_owned(), part))
        .collect()
}

#[test]
#[allow(clippy::too_many_lines)]
fn a_report_past_one_record_is_published_in_parts() {
    const AUTHORED_CYCLES: usize = 80;
    let authored = TREASURY_SCOPES * AUTHORED_CYCLES;
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(scenario(|state| {
        author_treasury_assessments(state, AUTHORED_CYCLES);
        for binding in [
            observer(SECOND_BINDING, PRINCIPAL, granary(), 500),
            observer(AUDITOR_BINDING, AUDITOR, granary(), 900),
        ] {
            state.observer_bindings.insert(binding.id.clone(), binding);
        }
    }));
    let mut seen = 0;
    let mut heads = BTreeMap::new();

    // The first fiscal change publishes every report whole. The treasury's
    // observer gets one summary and 21 parts for 321 facts, far more than one
    // record held; the two granary observers get a summary of nothing.
    let receipts = submit(&mut canwu, vec![treasury_assessment("first", 0, "new.000")]);
    let facts = authored + 1;
    assert!(facts > FACTS_IN_ONE_OLD_RECORD);
    let parts = facts.div_ceil(FISCAL_REPORT_FACTS_PER_PART);
    assert_eq!(published(&receipts), 1 + parts + 2);
    let reports = assert_reports_match_state(&canwu);
    assert_eq!(reports[OBSERVER].projection.facts.len(), facts);
    assert!(reports[SECOND_BINDING].projection.facts.is_empty());
    assert!(reports[AUDITOR_BINDING].projection.facts.is_empty());
    let mut lines = treasury_lines((0..parts).map(|part| u32::try_from(part).expect("part")));
    lines.push((SECOND_BINDING.to_owned(), None));
    assert_eq!(
        published_lines(&canwu, PRINCIPAL, &mut seen, &mut heads),
        lines
    );
    // Every record is far below the engine's payload limit.
    let largest = published_report_records(&canwu, PRINCIPAL)
        .iter()
        .map(|(record, _)| {
            serde_json::to_vec(&record.payload)
                .expect("payload bytes")
                .len()
        })
        .max()
        .expect("published records");
    assert!(largest < 65_536 / 4, "largest record is {largest} bytes");

    // A new aggregate joins the last part. Only that part and the summary
    // are published again, each superseding the record before it.
    let receipts = submit(
        &mut canwu,
        vec![treasury_assessment("second", 1, "new.000")],
    );
    assert_eq!(published(&receipts), 2);
    assert_eq!(
        published_lines(&canwu, PRINCIPAL, &mut seen, &mut heads),
        treasury_lines([20])
    );

    // A remission changes one of the oldest aggregates: only the first part
    // follows.
    let receipts = submit(
        &mut canwu,
        vec![(
            PRINCIPAL,
            draft(
                "remit",
                TREASURY_AUTHORITY,
                FiscalAction::GrantRemission {
                    remission_id: "remit.remission".to_owned(),
                    assessment_id: "authored.0.000".to_owned(),
                    quantity: 100,
                    reason: "flood".to_owned(),
                },
            ),
        )],
    );
    assert_eq!(published(&receipts), 2);
    assert_eq!(
        published_lines(&canwu, PRINCIPAL, &mut seen, &mut heads),
        treasury_lines([0])
    );

    // A change of the granary reaches the two observers whose bindings name
    // the granary, each at its own confidence, and nothing of the treasury's
    // report is published again.
    let receipts = submit(&mut canwu, vec![granary_assessment("granary", "new.000")]);
    assert_eq!(published(&receipts), 4);
    assert_eq!(
        published_lines(&canwu, PRINCIPAL, &mut seen, &mut heads),
        [
            (SECOND_BINDING.to_owned(), None),
            (SECOND_BINDING.to_owned(), Some(0))
        ]
    );
    let reports = assert_reports_match_state(&canwu);
    for binding in [SECOND_BINDING, AUDITOR_BINDING] {
        let facts = &reports[binding].projection.facts;
        assert_eq!(facts.len(), 1);
        assert!(facts.values().all(|fact| fact.institution == granary()));
    }
    assert!(
        reports[OBSERVER]
            .projection
            .facts
            .values()
            .all(|fact| fact.institution == treasury())
    );
    assert_ne!(
        reports[SECOND_BINDING].projection.facts,
        reports[AUDITOR_BINDING].projection.facts
    );

    // A boundary that changes no fact an observer can see publishes nothing:
    // here a rejected action, then an audit finding.
    let ahead = procedure_revision(&canwu) + 1;
    enqueue_requests(
        &mut canwu,
        &[(
            PRINCIPAL,
            assessment("stale", TREASURY_AUTHORITY, &treasury_scope(0), "new.001").against(ahead),
        )],
    );
    assert_eq!(published(&settle(&mut canwu)), 0);
    let receipts = submit(
        &mut canwu,
        vec![(
            PRINCIPAL,
            draft(
                "audit",
                TREASURY_AUTHORITY,
                FiscalAction::RecordAudit {
                    audit_id: "audit.finding".to_owned(),
                    target_id: "authored.0.000".to_owned(),
                    severity: FiscalAuditSeverity::Notice,
                    finding: "checked".to_owned(),
                    evidence: BTreeSet::new(),
                },
            ),
        )],
    );
    assert_eq!(published(&receipts), 0);
    let state = fiscal_state(&canwu);
    assert_eq!(
        state.action_outcomes["stale"].disposition,
        FiscalActionDisposition::Rejected
    );
    assert_eq!(
        state.action_outcomes["audit"].disposition,
        FiscalActionDisposition::Applied
    );

    // A full boundary of assessments adds 64 aggregates. They fill the last
    // part and four new ones, and only those five are published.
    let batch: Vec<_> = (0..16)
        .flat_map(|cycle| {
            (0..TREASURY_SCOPES).map(move |scope| {
                treasury_assessment(
                    &format!("batch.{scope}.{cycle:02}"),
                    scope,
                    &format!("batch.{cycle:02}"),
                )
            })
        })
        .collect();
    assert_eq!(batch.len(), 64);
    let receipts = submit(&mut canwu, batch);
    assert_eq!(published(&receipts), 6);
    assert_eq!(
        published_lines(&canwu, PRINCIPAL, &mut seen, &mut heads),
        treasury_lines(20..25)
    );
    let reports = assert_reports_match_state(&canwu);
    assert_eq!(reports[OBSERVER].projection.facts.len(), authored + 2 + 64);

    fixture.assert_restore_and_replay(&canwu);
}

/// When more parts change in one boundary than a boundary publishes, the
/// boundary still commits. The parts that did not fit are published by the
/// next boundary that changes the fiscal state, and until then the reports
/// they belong to say how many parts are pending.
#[test]
fn parts_past_the_boundary_limit_follow_in_the_next_fiscal_boundary() {
    const AUTHORED_CYCLES: usize = 64;
    let facts = TREASURY_SCOPES * AUTHORED_CYCLES;
    let parts = facts / FISCAL_REPORT_FACTS_PER_PART;
    let binding = |index: usize| format!("observer.{index:02}");
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(scenario(|state| {
        author_treasury_assessments(state, AUTHORED_CYCLES);
        // Every observer binding names the same actor, so one holder holds
        // every line of every report.
        state.observer_bindings.clear();
        for index in 0..MAX_FISCAL_OBSERVERS {
            let confidence = if index % 2 == 0 { 1_000 } else { 600 };
            let observer = observer(&binding(index), PRINCIPAL, treasury(), confidence);
            state
                .observer_bindings
                .insert(observer.id.clone(), observer);
        }
    }));
    let wanted = MAX_FISCAL_OBSERVERS * (1 + parts);
    assert!(wanted > MAX_FISCAL_REPORT_RECORDS_PER_BOUNDARY);
    let deferred = wanted - MAX_FISCAL_REPORT_RECORDS_PER_BOUNDARY;

    // The first fiscal change plans every part of every report. The boundary
    // publishes as many records as it may and commits.
    let receipts = submit(&mut canwu, vec![granary_assessment("first", "new.000")]);
    assert_eq!(published(&receipts), MAX_FISCAL_REPORT_RECORDS_PER_BOUNDARY);
    let reports = reports_of(&canwu, PRINCIPAL);
    assert_eq!(reports.len(), MAX_FISCAL_OBSERVERS);
    let pending: usize = reports
        .values()
        .map(|report| report.pending_parts.len())
        .sum();
    assert_eq!(pending, deferred);
    // The reports that are complete are right; the others hold fewer facts
    // than their summary counts and say so.
    let state = fiscal_state(&canwu);
    for (observer_id, report) in &reports {
        let derived = &compute_projections(&state, report.projection.as_of)[observer_id];
        if report.is_complete() {
            assert_eq!(&report.projection, derived);
        } else {
            assert!(report.projection.facts.len() < derived.facts.len());
        }
    }
    assert!(reports[&binding(0)].is_complete());
    let last = &reports[&binding(MAX_FISCAL_OBSERVERS - 1)];
    assert_eq!(
        last.pending_parts,
        (0..parts)
            .map(|part| u32::try_from(part).expect("part"))
            .collect()
    );

    // The next fiscal change publishes what was left, with a new summary for
    // each report it completes, and nothing else.
    let incomplete = reports
        .values()
        .filter(|report| !report.is_complete())
        .count();
    let receipts = submit(&mut canwu, vec![granary_assessment("second", "new.001")]);
    assert_eq!(published(&receipts), incomplete + deferred);
    assert_reports_match_state(&canwu);
    assert!(fiscal_state(&canwu).report_plan.deferred.is_empty());

    fixture.assert_restore_and_replay(&canwu);
}

/// The engine publishes no knowledge to a dead person. When an observer's
/// actor has died, a change that his report would show still commits, the
/// other observers get their parts, and his report stays as he last had it.
/// What he missed is kept, by part and so without growing, and is published
/// by the first fiscal change after he is alive again.
#[test]
fn a_dead_observer_is_skipped_until_he_is_alive_again() {
    const DEPUTY_BINDING: &str = "observer.deputy";
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(scenario(|state| {
        for binding in [
            observer(DEPUTY_BINDING, DEPUTY, treasury(), 1_000),
            observer(AUDITOR_BINDING, AUDITOR, granary(), 900),
        ] {
            state.observer_bindings.insert(binding.id.clone(), binding);
        }
    }));
    let deferred = |canwu: &Canwu| fiscal_state(canwu).report_plan.deferred;
    let held_for_deputy = BTreeMap::from([(DEPUTY_BINDING.to_owned(), BTreeSet::from([0_u32]))]);

    // Everyone gets a first report: two observers of the treasury a summary
    // and a part each, the auditor of the granary a summary of nothing.
    let receipts = submit(&mut canwu, vec![treasury_assessment("first", 0, "c1")]);
    assert_eq!(published(&receipts), 5);
    assert_reports_match_state(&canwu);

    enqueue_life(&mut canwu, DEPUTY, false);
    settle(&mut canwu);
    let at_death = reports_of(&canwu, DEPUTY);
    assert_eq!(at_death[DEPUTY_BINDING].projection.facts.len(), 1);

    // A change his report would show. The boundary commits and the other
    // observer of the treasury gets his part.
    let receipts = submit(&mut canwu, vec![treasury_assessment("second", 1, "c1")]);
    assert_eq!(published(&receipts), 2);
    assert_eq!(
        fiscal_state(&canwu).action_outcomes["second"].disposition,
        FiscalActionDisposition::Applied
    );
    let principal = &reports_of(&canwu, PRINCIPAL)[OBSERVER];
    assert!(principal.is_complete());
    assert_eq!(principal.projection.facts.len(), 2);
    assert_eq!(deferred(&canwu), held_for_deputy);
    assert_eq!(reports_of(&canwu, DEPUTY), at_death);

    // Later changes add nothing for him but the parts they touch, and take
    // nothing from the others, whose changes still go out in full.
    let receipts = submit(&mut canwu, vec![treasury_assessment("third", 2, "c1")]);
    assert_eq!(published(&receipts), 2);
    let receipts = submit(&mut canwu, vec![granary_assessment("granary", "c1")]);
    assert_eq!(published(&receipts), 2);
    assert_eq!(deferred(&canwu), held_for_deputy);
    assert_eq!(reports_of(&canwu, DEPUTY), at_death);

    // Alive again. The next fiscal change, here one he cannot see, publishes
    // what he missed with a new summary.
    enqueue_life(&mut canwu, DEPUTY, true);
    settle(&mut canwu);
    assert_eq!(deferred(&canwu), held_for_deputy);
    let receipts = submit(&mut canwu, vec![granary_assessment("granary.again", "c2")]);
    assert_eq!(published(&receipts), 4);
    assert!(deferred(&canwu).is_empty());
    let reports = assert_reports_match_state(&canwu);
    assert_eq!(reports[DEPUTY_BINDING].projection.facts.len(), 3);

    fixture.assert_restore_and_replay(&canwu);
}

/// A state whose observer binding names a person who does not exist is
/// refused when the run is created: the state record references every
/// observer's person, and the engine checks references against its entity
/// registry.
#[test]
fn an_observer_who_does_not_exist_is_refused_when_the_run_is_created() {
    let fixture = Fixture::new();
    let nobody = PersonId::new(777);
    let error = fixture
        .try_runtime(scenario(|state| {
            let binding = observer("observer.nobody", nobody, treasury(), 1_000);
            state.observer_bindings.insert(binding.id.clone(), binding);
        }))
        .err()
        .expect("an observer without a person must be refused");
    assert!(
        error.message.contains("references missing core entity"),
        "{error}"
    );
}

/// A report plan means something only in a state a boundary has written. A
/// scenario that brings one, as a state exported from another run does, has
/// it ignored: the first derivation publishes every report whole, whichever
/// system writes the state first.
#[test]
fn a_report_plan_that_came_with_a_scenario_is_ignored() {
    const AUTHORED_CYCLES: usize = 10;
    let facts = TREASURY_SCOPES * AUTHORED_CYCLES;
    let parts = facts.div_ceil(FISCAL_REPORT_FACTS_PER_PART);
    let used_plan = || FiscalReportPlan {
        revision: 1,
        publish: BTreeMap::from([(OBSERVER.to_owned(), BTreeSet::from([0]))]),
        deferred: BTreeMap::from([(OBSERVER.to_owned(), BTreeSet::from([1]))]),
    };
    let fixture = Fixture::new();

    // The state's derived fields are right, so its first writer is the
    // settlement of the first action.
    let mut canwu = fixture.runtime(scenario(|state| {
        author_treasury_assessments(state, AUTHORED_CYCLES);
        state.report_plan = used_plan();
    }));
    let receipts = submit(&mut canwu, vec![granary_assessment("first", "c1")]);
    assert_eq!(published(&receipts), 1 + parts);
    let reports = assert_reports_match_state(&canwu);
    assert_eq!(reports[OBSERVER].projection.facts.len(), facts);
    fixture.assert_restore_and_replay(&canwu);

    // The state's aggregates are missing, so its first writer is the
    // aggregate system, in a boundary without fiscal ingress.
    let mut state = base_state();
    author_treasury_assessments(&mut state, AUTHORED_CYCLES);
    state.report_plan = used_plan();
    let mut canwu = fixture.runtime(scenario_from_state(state));
    enqueue_tick(&mut canwu);
    assert_eq!(published(&settle(&mut canwu)), 1 + parts);
    let reports = assert_reports_match_state(&canwu);
    assert_eq!(reports[OBSERVER].projection.facts.len(), facts);
}

/// A scenario whose knowledge carries a report record of schema version 1,
/// the one-record report of 0.14.0, is refused when the run is created: only
/// version 2 is registered, and the engine accepts no record of a schema it
/// does not know.
#[test]
fn a_scenario_with_a_version_one_report_record_is_refused() {
    let fixture = Fixture::new();
    let holder = KnowledgeHolderRef::Person(PRINCIPAL);
    let mut version_one = fiscal_report_knowledge_schema_id();
    assert_eq!(version_one.version, 2);
    version_one.version = 1;
    let mut scenario = scenario(|_| {});
    let id = KnowledgeRecordId::new(1);
    scenario.knowledge.records.insert(
        holder.clone(),
        BTreeMap::from([(
            id,
            KnowledgeRecord {
                id,
                holder,
                schema: version_one,
                subjects: vec![KnowledgeSubject {
                    role: "fiscal_state".to_owned(),
                    target: KnowledgeSubjectTarget::DomainRecord(
                        fiscal_state_reference().into_untyped(),
                    ),
                }],
                payload: serde_json::json!({
                    "actor": PRINCIPAL,
                    "as_of": SimTime::EPOCH,
                    "confidence_per_mille": 1_000,
                    "facts": {},
                }),
                as_of: Some(SimTime::EPOCH),
                learned_at: SimTime::EPOCH,
                confidence_per_mille: 1_000,
                origin: KnowledgeOrigin {
                    method: "fiscal_authority_report_v1".to_owned(),
                    evidence: Vec::new(),
                },
                supersedes: Vec::new(),
                contradicts: Vec::new(),
            },
        )]),
    );
    let error = fixture
        .try_runtime(scenario)
        .err()
        .expect("a version-1 report record must be refused");
    assert_eq!(error.code, ErrorCode::InvalidKnowledgeRecord, "{error}");
}
