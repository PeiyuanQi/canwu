//! An observer's fiscal report is a summary and parts, each its own line of
//! knowledge records on the observer's own record, so its size is not
//! bounded by one record.
//!
//! A part is sixteen scope rows of one accounting cycle. The first run
//! reports several hundred facts to one observer, well past what one
//! 65,536-byte record held. Every boundary commits, the report the observer
//! assembles equals what the fiscal records derive, a boundary republishes
//! only the parts whose digest changed, each superseding exactly the record
//! before it on its line, and restore and exact replay agree. The second run
//! owes more than a boundary publishes: the rest stays queued, and the next
//! boundary, which the publishing boundary schedules at once, publishes it
//! without any change of the fiscal records; until then the summary names
//! the parts that are pending. A third run owes more payload bytes than a
//! boundary publishes, and the bytes, not the count, cut each publication.
//! The last runs cover what cannot be published at once or at all: a report
//! for an observer who has died, completed by the boundary that revives him,
//! in a run that sealed its evidence as in one that did not; an observer who
//! does not exist; a queue that names one; and a run with direct command
//! history, which cannot schedule a drain.

mod support;

use canwu_api::{
    BoundaryReceipt, BoundaryRequest, Canwu, Command, CommandEnvelope, CompactedCanwu, ErrorCode,
    IngressPayload, Issuer, KnowledgeHolderRef, KnowledgeOrigin, KnowledgeRecord,
    KnowledgeRecordId, KnowledgeSubject, KnowledgeSubjectTarget, PersonId, SimDuration, SimTime,
    SimulationGranularity, TerritoryId,
};
use canwu_fiscal::{
    FISCAL_REPORT_DRAIN_INGRESS, FISCAL_REPORT_ROWS_PER_PART, FiscalAction, FiscalActionBatch,
    FiscalActionDisposition, FiscalAdoptionStage, FiscalAdoptionState, FiscalAssessment,
    FiscalAuditSeverity, FiscalMechanism, FiscalObserverBinding, FiscalPaymentForm, FiscalReport,
    FiscalReportBudget, FiscalReportFact, FiscalReportQueueEntry, FiscalReportRecord,
    FiscalScopeBinding, FiscalSettledAction, FiscalState, compute_projection,
    fiscal_observer_reference, fiscal_report_knowledge_schema_id, fiscal_reports,
};
use std::collections::{BTreeMap, BTreeSet};
use support::{
    AUDITOR, DEPUTY, Draft, Fixture, GRANARY_AUTHORITY, GRANARY_PRINCIPAL, GRANARY_SCOPE, OBSERVER,
    PRINCIPAL, RULE, TREASURY_AUTHORITY, TREASURY_SCOPES, assessment, base_state, draft,
    enqueue_life, enqueue_requests, enqueue_tick, fiscal_record_versions, fiscal_state, granary,
    ledger_revision, life_at, observer_binding, outcome, published_report_records, scenario,
    scenario_from_state, settle, submit, submit_sealed, treasury, treasury_scope,
};

/// The second binding of the treasury's observer: the same actor, the
/// granary, and a lower confidence.
const SECOND_BINDING: &str = "observer.principal.granary";
const AUDITOR_BINDING: &str = "observer.auditor";
/// One 65,536-byte report record held about 196 facts.
const FACTS_IN_ONE_OLD_RECORD: usize = 196;
/// Units of the authored assessments: one book holds one of each.
const UNITS: usize = 4;

fn observer(
    id: &str,
    actor: PersonId,
    institution: canwu_api::EntityRef,
    confidence: u16,
) -> FiscalObserverBinding {
    FiscalObserverBinding {
        confidence_per_mille: confidence,
        ..observer_binding(id, actor, BTreeSet::from([institution]))
    }
}

fn extra_scope(index: usize) -> String {
    format!("scope.treasury.x.{index:02}")
}

/// Adds `count` operational land scopes to the treasury. Its scopes are
/// ranked by ID: its four base scopes, its pending scope, then these.
fn add_treasury_scopes(state: &mut FiscalState, count: usize) {
    for index in 0..count {
        let id = extra_scope(index);
        state.add_scope(FiscalScopeBinding {
            id: id.clone(),
            institution: treasury(),
            jurisdiction_id: "region.a".to_owned(),
            subject_scope: "registered_land".to_owned(),
            mechanism: FiscalMechanism::LandTax,
            authoritative_granularity: SimulationGranularity::Aggregate,
        });
        state
            .add_adoption(FiscalAdoptionState {
                id: format!("adopt.x.{index:02}"),
                rule_id: RULE.to_owned(),
                scope_binding_id: id,
                stage: FiscalAdoptionStage::Implemented,
                generation: 1,
                changed_at: SimTime::EPOCH,
                source_action_id: None,
            })
            .expect("an adoption of a new scope");
    }
}

/// Every operational treasury scope of a state with `extra` added scopes.
fn assessed_scopes(extra: usize) -> Vec<String> {
    (0..TREASURY_SCOPES)
        .map(treasury_scope)
        .chain((0..extra).map(extra_scope))
        .collect()
}

/// Installs, for each scope and each of `cycles` cycles, a book of one
/// assessment of each of `units` units, all created when the run starts.
fn author_assessments(state: &mut FiscalState, scopes: &[String], cycles: usize, units: usize) {
    for scope in scopes {
        for cycle in 0..cycles {
            for unit in 0..units {
                state
                    .add_assessment(FiscalAssessment {
                        id: format!("authored.{cycle:03}.{unit}"),
                        rule_id: RULE.to_owned(),
                        scope_binding_id: scope.clone(),
                        accounting_cycle_id: format!("authored.{cycle:03}"),
                        quantity: 500,
                        unit: format!("grain.{unit}"),
                        payment_form: FiscalPaymentForm::Grain,
                        commutation_quote: None,
                        created_at: SimTime::EPOCH,
                        source_action_id: None,
                    })
                    .expect("an authored assessment");
            }
        }
    }
}

fn treasury_assessment(action_id: &str, scope: &str, cycle: &str) -> (PersonId, Draft) {
    (
        PRINCIPAL,
        assessment(action_id, TREASURY_AUTHORITY, scope, cycle),
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
/// what the fiscal records derive for its binding, and returns the reports
/// by observer binding ID. An observer told nothing yet holds no report.
fn assert_reports_match_state(canwu: &Canwu) -> BTreeMap<String, FiscalReport> {
    let state = fiscal_state(canwu);
    let mut reports = BTreeMap::new();
    for actor in state
        .observer_bindings()
        .filter_map(|binding| binding.actor)
        .collect::<BTreeSet<_>>()
    {
        reports.extend(reports_of(canwu, actor));
    }
    for (observer_id, report) in &reports {
        assert!(report.is_complete(), "{observer_id} is complete");
        assert_eq!(
            Some(&report.projection),
            compute_projection(&state, observer_id, report.projection.as_of)
                .expect("projection")
                .as_ref(),
            "{observer_id} reports what the records derive"
        );
    }
    reports
}

/// The report line a record is on: its observer binding, and the slot and
/// number of its part, or `None` for the summary.
type Line = (String, Option<(u16, u32)>);

fn line(record: &FiscalReportRecord) -> Line {
    match record {
        FiscalReportRecord::Summary(summary) => (summary.observer_id.clone(), None),
        FiscalReportRecord::Part(part) => (part.observer_id.clone(), Some((part.slot, part.part))),
    }
}

/// Follows the report records published to `actor` since `seen`, asserts that
/// each supersedes exactly the record before it on its line, and returns the
/// lines published, in order.
fn published_lines(
    canwu: &Canwu,
    actor: PersonId,
    seen: &mut usize,
    heads: &mut BTreeMap<Line, KnowledgeRecord>,
) -> Vec<Line> {
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

fn lines_of(observer: &str, parts: impl IntoIterator<Item = (u16, u32)>) -> Vec<Line> {
    std::iter::once(None)
        .chain(parts.into_iter().map(Some))
        .map(|part| (observer.to_owned(), part))
        .collect()
}

#[test]
#[allow(clippy::too_many_lines)]
fn a_report_past_one_record_is_published_in_parts() {
    // Forty-eight treasury rows, three parts to a cycle.
    const EXTRA_SCOPES: usize = 43;
    const AUTHORED_CYCLES: usize = 5;
    let scopes = assessed_scopes(EXTRA_SCOPES);
    let authored = scopes.len() * AUTHORED_CYCLES * UNITS;
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(scenario(|state| {
        add_treasury_scopes(state, EXTRA_SCOPES);
        author_assessments(state, &scopes, AUTHORED_CYCLES, UNITS);
        state.add_observer(observer(SECOND_BINDING, PRINCIPAL, granary(), 500));
        state.add_observer(observer(AUDITOR_BINDING, AUDITOR, granary(), 900));
    }));
    let mut seen = 0;
    let mut heads = BTreeMap::new();

    // The first boundary publishes the authored report whole: one summary
    // and three parts of each of five cycles, far more than one record
    // held; the granary's observers, whose institution has no book yet, are
    // told nothing. The boundary that settles the first action publishes
    // its new cycle's one part with a new summary.
    let receipts = submit(
        &mut canwu,
        vec![treasury_assessment("first", &treasury_scope(0), "new.000")],
    );
    assert!(authored > FACTS_IN_ONE_OLD_RECORD);
    assert_eq!(published(&receipts), 1 + 5 * 3 + 2);
    let reports = assert_reports_match_state(&canwu);
    assert_eq!(reports[OBSERVER].projection.facts.len(), authored + 1);
    assert!(!reports.contains_key(SECOND_BINDING));
    assert!(!reports.contains_key(AUDITOR_BINDING));
    let mut lines = lines_of(
        OBSERVER,
        (0..5_u16).flat_map(|slot| (0..3).map(move |part| (slot, part))),
    );
    lines.extend(lines_of(OBSERVER, [(5, 0)]));
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
    assert!(largest < 65_536 / 2, "largest record is {largest} bytes");

    // A new book joins the part of the first one. Only that part and the
    // summary are published again, each superseding the record before it.
    let receipts = submit(
        &mut canwu,
        vec![treasury_assessment("second", &treasury_scope(1), "new.000")],
    );
    assert_eq!(published(&receipts), 2);
    assert_eq!(
        published_lines(&canwu, PRINCIPAL, &mut seen, &mut heads),
        lines_of(OBSERVER, [(5, 0)])
    );

    // A remission changes one of the authored books: only its part follows.
    let receipts = submit(
        &mut canwu,
        vec![(
            PRINCIPAL,
            draft(
                "remit",
                TREASURY_AUTHORITY,
                FiscalAction::GrantRemission {
                    remission_id: "remit.remission".to_owned(),
                    scope_binding_id: treasury_scope(0),
                    assessment_id: "authored.000.0".to_owned(),
                    quantity: 100,
                    reason: "flood".to_owned(),
                },
            ),
        )],
    );
    assert_eq!(published(&receipts), 2);
    assert_eq!(
        published_lines(&canwu, PRINCIPAL, &mut seen, &mut heads),
        lines_of(OBSERVER, [(0, 0)])
    );

    // A change of the granary reaches the two observers whose bindings name
    // the granary, each at its own confidence, and nothing of the treasury's
    // report is published again.
    let receipts = submit(&mut canwu, vec![granary_assessment("granary", "new.000")]);
    assert_eq!(published(&receipts), 4);
    assert_eq!(
        published_lines(&canwu, PRINCIPAL, &mut seen, &mut heads),
        lines_of(SECOND_BINDING, [(0, 0)])
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
    let behind = ledger_revision(&canwu, &treasury_scope(0)) - 1;
    enqueue_requests(
        &mut canwu,
        &[(
            PRINCIPAL,
            assessment("stale", TREASURY_AUTHORITY, &treasury_scope(0), "new.001").against(behind),
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
                    scope_binding_id: treasury_scope(0),
                    target_id: "authored.000.0".to_owned(),
                    severity: FiscalAuditSeverity::Notice,
                    finding: "checked".to_owned(),
                    evidence: BTreeSet::new(),
                },
            ),
        )],
    );
    assert_eq!(published(&receipts), 0);
    assert_eq!(
        outcome(&canwu, "stale").expect("stale outcome").disposition,
        FiscalActionDisposition::Rejected
    );
    assert_eq!(
        outcome(&canwu, "audit").expect("audit outcome").disposition,
        FiscalActionDisposition::Applied
    );

    // One batch assesses twenty-one more scopes, rows 5 to 25: they fill
    // the new cycle's first part and start its second, and only those two
    // are published.
    let entries = (0..21)
        .map(|index| {
            let scope = extra_scope(index);
            let revision = ledger_revision(&canwu, &scope);
            assessment(
                &format!("batch.{index:02}"),
                TREASURY_AUTHORITY,
                &scope,
                "new.000",
            )
            .entry(revision)
        })
        .collect();
    enqueue_requests(
        &mut canwu,
        &[(
            PRINCIPAL,
            FiscalActionBatch {
                authority_binding_id: TREASURY_AUTHORITY.to_owned(),
                entries,
            },
        )],
    );
    let receipts = settle(&mut canwu);
    assert_eq!(published(&receipts), 3);
    assert_eq!(
        published_lines(&canwu, PRINCIPAL, &mut seen, &mut heads),
        lines_of(OBSERVER, [(5, 0), (5, 1)])
    );
    let reports = assert_reports_match_state(&canwu);
    assert_eq!(reports[OBSERVER].projection.facts.len(), authored + 2 + 21);

    fixture.assert_restore_and_replay(&canwu);
}

/// Seventy observers of the treasury, all held by the principal, each with
/// sixteen cycles of authored books, one part each: seventeen records each,
/// far more than one boundary publishes.
fn crowded_state() -> FiscalState {
    let mut state = base_state();
    author_assessments(&mut state, &assessed_scopes(0), 16, 1);
    state.remove_observer(OBSERVER);
    for index in 0..CROWD {
        let confidence = if index % 2 == 0 { 1_000 } else { 600 };
        state.add_observer(observer(&crowd(index), PRINCIPAL, treasury(), confidence));
    }
    state
}

/// The report bodies one boundary publishes in the tests of the share: a
/// share well below what a crowd of reports owes, set with
/// `FiscalPlugin::with_report_budget`.
const SHARE: usize = 1_024;

/// The fixture with fiscal's share cut to [`SHARE`] bodies and entries.
fn shared() -> Fixture {
    Fixture::with_report_budget(FiscalReportBudget {
        bodies: SHARE,
        entries: SHARE,
        ..FiscalReportBudget::DEFAULT
    })
}

const CROWD: usize = 70;
const RECORDS_PER_OBSERVER: usize = 1 + 16;

fn crowd(index: usize) -> String {
    format!("observer.{index:02}")
}

fn drains(canwu: &Canwu) -> usize {
    canwu
        .ingress_log()
        .iter()
        .filter(|ingress| {
            matches!(
                &ingress.payload,
                IngressPayload::Plugin { packet_type, .. }
                    if packet_type == FISCAL_REPORT_DRAIN_INGRESS
            )
        })
        .count()
}

/// When more is owed than fiscal's share of a boundary's publication budget
/// (here cut to [`SHARE`] bodies) lets it publish, the boundary still
/// commits, and the report it could publish only in part says which parts are
/// pending. The boundary schedules a drain packet due at once, so the next
/// boundary, at the same instant, publishes the rest. Nothing in the fiscal
/// records changes for that but the report bookkeeping: no revision, no
/// action, no receipt. After it nothing is owed, nothing more is queued, and
/// a boundary without fiscal ingress writes and publishes nothing.
#[test]
#[allow(clippy::too_many_lines)]
fn parts_past_the_report_share_follow_in_the_next_boundary() {
    let fixture = shared();
    let mut canwu = fixture.runtime(scenario_from_state(crowded_state()));
    let wanted = CROWD * RECORDS_PER_OBSERVER;
    assert!(wanted > SHARE);
    let full = SHARE / RECORDS_PER_OBSERVER;
    let split_published = SHARE - full * RECORDS_PER_OBSERVER - 1;
    let split_pending = RECORDS_PER_OBSERVER - 1 - split_published;

    // The first event-bearing boundary owes every report whole. It publishes
    // as many records as it may and commits: `full` reports whole, and the
    // next one in part. The observers after them keep their queued work, and
    // a queue page whose work did not change is not written again.
    let queue_pages = |canwu: &Canwu| -> BTreeMap<String, (u64, serde_json::Value)> {
        canwu
            .domain_records()
            .filter(|record| record.reference.kind.name == "report-queue")
            .map(|record| {
                (
                    record.reference.to_string(),
                    (record.version, record.payload.clone()),
                )
            })
            .collect()
    };
    let pages_before = queue_pages(&canwu);
    enqueue_tick(&mut canwu);
    let first = canwu
        .step_canonical()
        .expect("the first boundary")
        .expect("the tick opens a boundary");
    assert_eq!(first.knowledge_record_count, SHARE);
    assert_eq!(drains(&canwu), 1, "the boundary schedules a drain");
    let unchanged: Vec<_> = queue_pages(&canwu)
        .into_iter()
        .filter(|(page, (_, payload))| {
            pages_before
                .get(page)
                .is_some_and(|(_, before)| before == payload)
        })
        .collect();
    assert!(!unchanged.is_empty());
    for (page, (version, _)) in &unchanged {
        assert_eq!(
            *version, pages_before[page].0,
            "{page} is not written again"
        );
    }
    let reports = reports_of(&canwu, PRINCIPAL);
    assert_eq!(reports.len(), full + 1);
    let state = fiscal_state(&canwu);
    for (observer_id, report) in &reports {
        let derived = compute_projection(&state, observer_id, report.projection.as_of)
            .expect("projection")
            .expect("an observer's projection");
        if report.is_complete() {
            assert_eq!(report.projection, derived);
        } else {
            assert!(report.projection.facts.len() < derived.facts.len());
        }
    }
    let split = &reports[&crowd(full)];
    let pending: usize = split.pending.values().map(BTreeSet::len).sum();
    assert_eq!(pending, split_pending);
    assert!(
        split
            .projection
            .facts
            .keys()
            .all(|fact| !split.is_stale(fact))
    );
    assert_eq!(state.report_queue.len(), CROWD - full);

    // The next boundary comes at once, at the same instant, and publishes
    // what was left, and nothing else. A run restored while the drain is
    // queued, and the run's exact replay, carry it out alike.
    fixture.assert_restore_and_replay(&canwu);
    let mut restored = Canwu::from_snapshot_json_with_plugins(
        &canwu.snapshot_json().expect("snapshot with a queued drain"),
        &fixture.plugins(),
    )
    .expect("restored with a queued drain");
    restored.step_canonical().expect("the restored drain");
    let outcomes = FiscalSettledAction::from_events(canwu.events()).len();
    let revision = ledger_revision(&canwu, &treasury_scope(0));
    let at = canwu.time();
    let drain = canwu
        .step_canonical()
        .expect("the drain boundary")
        .expect("the drain packet opens a boundary");
    assert_eq!(drain.knowledge_record_count, wanted - SHARE + 1);
    assert_eq!(canwu.time(), at);
    assert_eq!(
        restored.snapshot_json().expect("restored snapshot"),
        canwu.snapshot_json().expect("snapshot")
    );
    let reports = assert_reports_match_state(&canwu);
    assert_eq!(reports.len(), CROWD);
    // Every report, whole in the first boundary or finished by the drain,
    // stands as of the one instant both boundaries share.
    assert!(reports.values().all(|report| report.projection.as_of == at));
    let drained = fiscal_state(&canwu);
    assert!(drained.report_queue.is_empty());
    assert!(drained.report_schedule.page_due.is_empty());
    assert_eq!(drained.root.earliest_due, None);
    assert_eq!(ledger_revision(&canwu, &treasury_scope(0)), revision);
    assert_eq!(
        FiscalSettledAction::from_events(canwu.events()).len(),
        outcomes
    );
    assert_eq!(drained.ledgers, state.ledgers);

    // Nothing is owed and nothing more is queued. A boundary without fiscal
    // ingress then writes and publishes nothing.
    assert_eq!(drains(&canwu), 1);
    assert!(
        canwu
            .step_canonical()
            .expect("an empty canonical queue")
            .is_none()
    );
    let versions = fiscal_record_versions(&canwu);
    enqueue_tick(&mut canwu);
    assert_eq!(published(&settle(&mut canwu)), 0);
    assert_eq!(fiscal_record_versions(&canwu), versions);

    fixture.assert_restore_and_replay(&canwu);
}

/// Observers of the byte-share test: six, all of the treasury and held by
/// the principal.
const WIDE_OBSERVERS: usize = 6;
/// Extra treasury scopes of the byte-share test: with the four base scopes
/// and the pending one, 600 rows, thirty-eight parts to a report.
const WIDE_EXTRA_SCOPES: usize = 595;

fn wide_observer(index: usize) -> String {
    format!("observer.wide.{index}")
}

/// Every treasury scope with a book of one authored cycle, and the six
/// observers, each owed its whole report of thirty-eight parts.
fn wide_state() -> FiscalState {
    let mut state = base_state();
    add_treasury_scopes(&mut state, WIDE_EXTRA_SCOPES);
    author_assessments(&mut state, &assessed_scopes(WIDE_EXTRA_SCOPES), 1, 1);
    state.remove_observer(OBSERVER);
    for index in 0..WIDE_OBSERVERS {
        state.add_observer(observer(
            &wide_observer(index),
            PRINCIPAL,
            treasury(),
            1_000,
        ));
    }
    state
}

/// When the parts' bytes, not their count, pass fiscal's share (here the
/// minimum share's bytes and 20,000 more, with the default counts of bodies
/// and entries), every boundary still commits: it publishes no more payload
/// than the share, and far fewer records than the share's count. A report
/// cut by bytes names in its summary exactly the parts left, those after the
/// last one published. The drains at the same instant complete every report,
/// each what the fiscal records derive, and a run restored mid-drain and the
/// run's exact replay agree.
#[test]
#[allow(clippy::too_many_lines)]
fn parts_past_the_byte_share_follow_at_the_same_instant() {
    let share = FiscalReportBudget {
        payload_bytes: FiscalReportBudget::MINIMUM.payload_bytes + 20_000,
        ..FiscalReportBudget::DEFAULT
    };
    let fixture = Fixture::with_report_budget(share);
    let mut canwu = fixture.runtime(scenario_from_state(wide_state()));
    let rows = TREASURY_SCOPES + 1 + WIDE_EXTRA_SCOPES;
    let parts = u32::try_from(rows.div_ceil(FISCAL_REPORT_ROWS_PER_PART)).expect("parts");
    assert_eq!(parts, 38);
    let cycle = "authored.000";
    let at = canwu.time();
    enqueue_tick(&mut canwu);
    let mut seen = 0;
    let mut boundaries = 0;
    let mut cut = 0;
    while let Some(receipt) = canwu.step_canonical().expect("every boundary commits") {
        boundaries += 1;
        assert!(boundaries <= 64, "the drains at the instant end");
        assert_eq!(canwu.time(), at);
        let records = published_report_records(&canwu, PRINCIPAL);
        let published = &records[seen..];
        seen = records.len();
        assert_eq!(published.len(), receipt.knowledge_record_count);
        assert!(!published.is_empty(), "every boundary publishes some");
        let bytes: usize = published
            .iter()
            .map(|(record, _)| {
                serde_json::to_vec(&record.payload)
                    .expect("a payload")
                    .len()
            })
            .sum();
        assert!(bytes <= share.payload_bytes, "{bytes} bytes");
        assert!(published.len() * 100 < share.bodies, "bytes cut, not count");
        for (observer_id, report) in reports_of(&canwu, PRINCIPAL) {
            if report.is_complete() {
                continue;
            }
            cut += 1;
            let shown: BTreeSet<u32> = report
                .projection
                .facts
                .keys()
                .filter_map(|fact| FiscalReportFact::part_of(fact))
                .map(|(_, part)| part)
                .collect();
            let next = u32::try_from(shown.len()).expect("parts shown");
            assert!(next > 0 && next < parts, "{observer_id}");
            assert_eq!(shown, (0..next).collect(), "{observer_id}");
            assert_eq!(
                report.pending,
                BTreeMap::from([(cycle.to_owned(), (next..parts).collect())]),
                "{observer_id} names the parts left"
            );
        }
        if boundaries == 1 {
            fixture.assert_restore_and_replay(&canwu);
        }
    }
    assert!(boundaries > 2, "{boundaries} boundaries");
    assert!(cut > 0, "some report was cut by bytes");
    assert_eq!(drains(&canwu), boundaries - 1);
    let reports = assert_reports_match_state(&canwu);
    assert_eq!(reports.len(), WIDE_OBSERVERS);
    assert!(reports.values().all(|report| report.projection.as_of == at));
    let state = fiscal_state(&canwu);
    assert!(state.report_queue.is_empty());
    assert_eq!(state.root.earliest_due, None);
    fixture.assert_restore_and_replay(&canwu);
}

/// Work queued to repeat a cut publication is kept part by part, within the
/// report's 64 cycle slots. While its holder is dead, work for further
/// cycles merges into it; past the slots, the whole report is owed instead,
/// the boundary commits, and the holder gets the whole report once he is
/// alive again.
#[test]
fn a_held_republication_past_the_slots_is_owed_whole() {
    const HELD: &str = "observer.held";
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(scenario(|state| {
        state.add_observer(observer(HELD, AUDITOR, treasury(), 1_000));
        state.report_queue.insert(
            HELD.to_owned(),
            FiscalReportQueueEntry {
                due_at: SimTime::EPOCH,
                cycles: (0..64)
                    .map(|cycle| (format!("c{cycle:02}"), BTreeSet::from([0_u32])))
                    .collect(),
                whole: false,
                republish: true,
                parked: false,
            },
        );
    }));
    enqueue_life(&mut canwu, AUDITOR, false);
    settle(&mut canwu);
    assert!(queued(&canwu, HELD).is_some_and(|entry| entry.parked && entry.republish));

    // A book of a 65th cycle in a scope the observer sees.
    submit(
        &mut canwu,
        vec![treasury_assessment("c64", &treasury_scope(0), "c64")],
    );
    let held = queued(&canwu, HELD).expect("the held work");
    assert!(held.whole && held.cycles.is_empty() && held.parked);

    enqueue_life(&mut canwu, AUDITOR, true);
    settle(&mut canwu);
    assert!(queued(&canwu, HELD).is_none());
    let reports = assert_reports_match_state(&canwu);
    assert_eq!(reports[HELD].projection.facts.len(), 1);
    fixture.assert_restore_and_replay(&canwu);
}

/// A report cut by fiscal's share of the budget can leave more parts of
/// one cycle than queued work names. They are owed as the whole cycle
/// instead, which the drain publishes the same way: the parts the boundary
/// published match their digests and are skipped. Eleven observers of a
/// treasury of 1,601 scopes owe 101 parts and a summary each when every
/// scope is assessed in one boundary: ten reports fit, and the eleventh is
/// cut after three parts, leaving 98.
#[test]
fn a_cycle_cut_past_the_queued_parts_is_owed_whole() {
    const EXTRA: usize = 1_596;
    const OBSERVERS: usize = 11;
    let fixture = shared();
    let mut canwu = fixture.runtime(scenario(|state| {
        state.remove_observer(OBSERVER);
        add_treasury_scopes(state, EXTRA);
        for index in 0..OBSERVERS {
            state.add_observer(observer(&crowd(index), PRINCIPAL, treasury(), 1_000));
        }
    }));
    // The first boundary has nothing to report.
    enqueue_tick(&mut canwu);
    settle(&mut canwu);
    assert!(published_report_records(&canwu, PRINCIPAL).is_empty());

    let scopes = assessed_scopes(EXTRA);
    let batch = FiscalActionBatch {
        authority_binding_id: TREASURY_AUTHORITY.to_owned(),
        entries: scopes
            .iter()
            .map(|scope| {
                assessment(&format!("burst.{scope}"), TREASURY_AUTHORITY, scope, "2026")
                    .entry(ledger_revision(&canwu, scope))
            })
            .collect(),
    };
    enqueue_requests(&mut canwu, &[(PRINCIPAL, batch)]);
    canwu.step_canonical().expect("the admitting boundary");
    let settling = canwu
        .step_canonical()
        .expect("the settling boundary")
        .expect("a boundary settles the batch");
    assert_eq!(settling.knowledge_record_count, SHARE);
    let state = fiscal_state(&canwu);
    let (cut, owed) = state
        .report_queue
        .iter()
        .next()
        .expect("one report is owed the rest");
    assert_eq!(state.report_queue.len(), 1);
    assert!(!owed.republish && !owed.whole);
    assert_eq!(owed.cycles.get("2026"), Some(&BTreeSet::new()));
    let pending = &reports_of(&canwu, PRINCIPAL)[cut].pending["2026"];
    assert_eq!(pending.len(), 98);

    // The drain publishes the 98 parts the cut report still owes, and no
    // part it already published.
    let drain = canwu
        .step_canonical()
        .expect("the drain boundary")
        .expect("the drain packet opens a boundary");
    assert_eq!(drain.knowledge_record_count, 98 + 1);
    let reports = assert_reports_match_state(&canwu);
    assert_eq!(reports.len(), OBSERVERS);
    assert!(fiscal_state(&canwu).report_queue.is_empty());
}

const DEPUTY_BINDING: &str = "observer.deputy";

/// The base scenario with the deputy observing the treasury and the auditor
/// the granary.
fn deputy_and_auditor() -> canwu_api::Scenario {
    scenario(|state| {
        state.add_observer(observer(DEPUTY_BINDING, DEPUTY, treasury(), 1_000));
        state.add_observer(observer(AUDITOR_BINDING, AUDITOR, granary(), 900));
    })
}

/// What the queue holds for an observer.
fn queued(canwu: &Canwu, observer: &str) -> Option<FiscalReportQueueEntry> {
    fiscal_state(canwu).report_queue.get(observer).cloned()
}

/// The engine publishes no knowledge to a dead person. When an observer's
/// actor has died, a change that his report would show still commits, the
/// other observers get their parts, and his report stays as he last had it.
/// What he missed is kept queued, by part and so without growing, and the
/// boundary that revives him publishes it, without waiting for a change of
/// the fiscal records.
#[test]
fn a_dead_observer_is_skipped_until_he_is_alive_again() {
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(deputy_and_auditor());
    let held_for_deputy = FiscalReportQueueEntry {
        cycles: BTreeMap::from([("c1".to_owned(), BTreeSet::from([0_u32]))]),
        parked: true,
        ..FiscalReportQueueEntry::default()
    };
    let owed = |canwu: &Canwu| {
        queued(canwu, DEPUTY_BINDING).map(|entry| FiscalReportQueueEntry {
            due_at: SimTime::EPOCH,
            ..entry
        })
    };

    // The two observers of the treasury get a first report, a summary and a
    // part each; the auditor of the granary is told nothing yet.
    let receipts = submit(
        &mut canwu,
        vec![treasury_assessment("first", &treasury_scope(0), "c1")],
    );
    assert_eq!(published(&receipts), 4);
    assert_reports_match_state(&canwu);

    enqueue_life(&mut canwu, DEPUTY, false);
    settle(&mut canwu);
    let at_death = reports_of(&canwu, DEPUTY);
    assert_eq!(at_death[DEPUTY_BINDING].projection.facts.len(), 1);

    // A change his report would show. The boundary commits and the other
    // observer of the treasury gets his part.
    let receipts = submit(
        &mut canwu,
        vec![treasury_assessment("second", &treasury_scope(1), "c1")],
    );
    assert_eq!(published(&receipts), 2);
    assert_eq!(
        outcome(&canwu, "second")
            .expect("second outcome")
            .disposition,
        FiscalActionDisposition::Applied
    );
    let principal = &reports_of(&canwu, PRINCIPAL)[OBSERVER];
    assert!(principal.is_complete());
    assert_eq!(principal.projection.facts.len(), 2);
    assert_eq!(owed(&canwu), Some(held_for_deputy.clone()));
    assert_eq!(reports_of(&canwu, DEPUTY), at_death);

    // Later changes add nothing for him but the parts they touch, and take
    // nothing from the others, whose changes still go out in full.
    let receipts = submit(
        &mut canwu,
        vec![treasury_assessment("third", &treasury_scope(2), "c1")],
    );
    assert_eq!(published(&receipts), 2);
    let receipts = submit(&mut canwu, vec![granary_assessment("granary", "c1")]);
    assert_eq!(published(&receipts), 2);
    assert_eq!(owed(&canwu), Some(held_for_deputy));
    assert_eq!(reports_of(&canwu, DEPUTY), at_death);

    // While he is dead, a boundary without fiscal ingress writes and
    // publishes nothing, although parts are owed to him.
    let versions = fiscal_record_versions(&canwu);
    enqueue_tick(&mut canwu);
    assert_eq!(published(&settle(&mut canwu)), 0);
    assert_eq!(fiscal_record_versions(&canwu), versions);

    // Alive again. The boundary that revives him publishes what he missed,
    // with a new summary, and nothing else: no revision advances, and no
    // action becomes stale.
    let revision = ledger_revision(&canwu, &treasury_scope(0));
    enqueue_life(&mut canwu, DEPUTY, true);
    assert_eq!(published(&settle(&mut canwu)), 2);
    assert!(queued(&canwu, DEPUTY_BINDING).is_none());
    assert_eq!(ledger_revision(&canwu, &treasury_scope(0)), revision);
    let reports = assert_reports_match_state(&canwu);
    assert_eq!(reports[DEPUTY_BINDING].projection.facts.len(), 3);

    // A later change he cannot see publishes nothing to him.
    let receipts = submit(&mut canwu, vec![granary_assessment("granary.again", "c2")]);
    assert_eq!(published(&receipts), 2);
    assert_reports_match_state(&canwu);

    fixture.assert_restore_and_replay(&canwu);
}

/// Whether report work is owed, and to whom, is read from the committed
/// fiscal records and person availability, never from retained evidence. A
/// run that sealed its evidence while parts were owed to a dead observer
/// publishes them when he is revived exactly as an unsealed fork, a restore
/// from the sealed segments, and its own exact replay do.
#[test]
fn a_sealed_run_publishes_what_a_revived_observer_is_owed_as_its_replay_does() {
    let fixture = Fixture::new();
    let plugins = fixture.plugins();
    let mut base = fixture.runtime(deputy_and_auditor());
    submit(
        &mut base,
        vec![treasury_assessment("first", &treasury_scope(0), "c1")],
    );
    enqueue_life(&mut base, DEPUTY, false);
    settle(&mut base);
    submit(
        &mut base,
        vec![treasury_assessment("second", &treasury_scope(1), "c1")],
    );
    assert!(queued(&base, DEPUTY_BINDING).is_some_and(|entry| entry.parked));

    // A quiet boundary admits what the last boundary emitted, and then the
    // whole history seals.
    base.settle_boundary(BoundaryRequest::at(base.time()))
        .expect("a quiet boundary before sealing");
    let mut unsealed = base.fork();
    let mut sealed = base.into_compacted().expect("compact interface");
    let segment = sealed
        .seal_evidence()
        .expect("the settled history seals")
        .expect("the settled history holds evidence");
    let mut from_sealed = CompactedCanwu::from_checkpoint_and_journal_with_plugins(
        sealed.checkpoint().expect("compact checkpoint"),
        vec![segment.clone()],
        &plugins,
    )
    .expect("the sealed segments restore the compact run");

    // Each run revives the deputy, and the reviving boundary publishes what
    // he is owed: his summary and his one part.
    unsealed
        .enqueue_plugin_ingress(life_at(unsealed.time(), DEPUTY, true))
        .expect("revive in the unsealed run");
    assert_eq!(published(&settle(&mut unsealed)), 2);
    for run in [&mut sealed, &mut from_sealed] {
        run.enqueue_plugin_ingress(life_at(run.time(), DEPUTY, true))
            .expect("revive in a compact run");
        run.advance_canonical(canwu_api::SimDuration::minutes(1))
            .expect("the reviving boundary");
    }
    let report = &reports_of(&unsealed, DEPUTY)[DEPUTY_BINDING];
    assert!(report.is_complete());
    assert_eq!(report.projection.facts.len(), 2);
    let deputy = KnowledgeHolderRef::Person(DEPUTY);
    let ledger = |knowledge: &canwu_api::KnowledgeSnapshot| {
        support::held_records(knowledge, &deputy).expect("the deputy's ledger")
    };
    assert_eq!(ledger(sealed.knowledge()), ledger(unsealed.knowledge()));
    assert_eq!(
        ledger(from_sealed.knowledge()),
        ledger(unsealed.knowledge())
    );
    assert_eq!(sealed.checkpoint_hash(), unsealed.checkpoint_hash());
    assert_eq!(from_sealed.checkpoint_hash(), unsealed.checkpoint_hash());

    // A later change in the sealed run settles as in the unsealed one.
    let later = vec![treasury_assessment("third", &treasury_scope(2), "c1")];
    submit(&mut unsealed, later.clone());
    submit_sealed(&mut sealed, later);
    assert_eq!(sealed.checkpoint_hash(), unsealed.checkpoint_hash());

    let journal = sealed
        .replay_journal_with_segments(vec![segment.clone()])
        .expect("the sealed run produces an exact replay journal");
    let replayed =
        Canwu::replay_from_journal(&plugins, &journal).expect("the sealed run replays exactly");
    assert_eq!(replayed.snapshot(), unsealed.snapshot());
}

/// An observer binding that names a person who does not exist is refused
/// when the run is created: the observer record references its holder, and
/// the engine checks references against its entity registry.
#[test]
fn an_observer_who_does_not_exist_is_refused_when_the_run_is_created() {
    let fixture = Fixture::new();
    let nobody = PersonId::new(777);
    let error = fixture
        .try_runtime(scenario(|state| {
            state.add_observer(observer("observer.nobody", nobody, treasury(), 1_000));
        }))
        .err()
        .expect("an observer without a person must be refused");
    assert!(
        error.message.contains("references missing core entity"),
        "{error}"
    );
}

/// The report queue is part of the fiscal records, and a run is refused
/// when it is created with a queue page that owes work to an observer
/// binding that does not exist.
#[test]
fn a_queue_naming_an_unknown_observer_is_refused_when_the_run_is_created() {
    let fixture = Fixture::new();
    let mut scenario = scenario(|_| {});
    let page = scenario
        .domain_records
        .iter_mut()
        .find(|record| record.reference.kind.name == "report-queue")
        .expect("the authored observer's queue page");
    page.payload["entries"]["observer.nobody"] =
        serde_json::to_value(FiscalReportQueueEntry::default()).expect("entry");
    let error = fixture
        .try_runtime(scenario)
        .err()
        .expect("a queue naming an unknown observer must be refused");
    assert!(error.message.contains("unknown observer"), "{error}");
}

/// A run that holds direct command history cannot add canonical ingress, so
/// a boundary of it cannot schedule a drain. A boundary that owes more than
/// it may publish then commits with the rest queued instead of failing, and
/// the next boundary that runs the fiscal systems publishes more of it.
#[test]
fn owed_reports_wait_in_a_run_with_direct_command_history() {
    let fixture = shared();
    let mut canwu = fixture.runtime(scenario_from_state(crowded_state()));
    let direct_command_boundary = |canwu: &mut Canwu| {
        canwu
            .submit(CommandEnvelope::new(
                Issuer::Actor(PRINCIPAL),
                Command::OrderMovement {
                    subject: canwu_api::EntityRef::Person(PRINCIPAL),
                    destination: TerritoryId::new(1),
                    cargo: Vec::new(),
                },
            ))
            .expect("a direct command");
        let at = canwu.time() + SimDuration::minutes(1);
        canwu
            .settle_boundary(BoundaryRequest::at(at))
            .expect("the boundary commits")
    };

    let first = direct_command_boundary(&mut canwu);
    assert_eq!(first.knowledge_record_count, SHARE);
    assert!(first.generated_ingress.is_empty(), "no drain is scheduled");
    let owed = |canwu: &Canwu| fiscal_state(canwu).report_queue.len();
    let owed_after_first = owed(&canwu);
    assert!(owed_after_first > 0);

    let second = direct_command_boundary(&mut canwu);
    assert!(second.knowledge_record_count > 0);
    assert!(second.generated_ingress.is_empty());
    assert!(owed(&canwu) < owed_after_first);
    fixture.assert_restore_and_replay(&canwu);
}

/// A scenario whose knowledge carries a report record of schema version 1,
/// the one-record report of 0.14.0, is refused when the run is created: only
/// version 5 is registered, and the engine accepts no record of a schema it
/// does not know.
#[test]
fn a_scenario_with_a_version_one_report_record_is_refused() {
    let fixture = Fixture::new();
    let holder = KnowledgeHolderRef::Person(PRINCIPAL);
    let mut version_one = fiscal_report_knowledge_schema_id();
    assert_eq!(version_one.version, 6);
    version_one.version = 1;
    let mut scenario = scenario(|_| {});
    let id = KnowledgeRecordId::new(1);
    support::seed_knowledge_record(
        &mut scenario,
        KnowledgeRecord {
            id,
            holder,
            schema: version_one,
            subjects: vec![KnowledgeSubject {
                role: "fiscal_observer".to_owned(),
                target: KnowledgeSubjectTarget::DomainRecord(
                    fiscal_observer_reference(OBSERVER).into_untyped(),
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
    );
    let error = fixture
        .try_runtime(scenario)
        .err()
        .expect("a version-1 report record must be refused");
    assert_eq!(error.code, ErrorCode::InvalidKnowledgeRecord, "{error}");
}

/// A full part of the longest texts a row may carry, at the most facts a row
/// may hold, fits one knowledge record with room to spare.
#[test]
fn a_full_part_of_the_longest_rows_fits_one_knowledge_record() {
    let long = |prefix: &str| {
        let mut text = prefix.to_owned();
        text.push_str(&"x".repeat(canwu_fiscal::MAX_FISCAL_IDENTIFIER_BYTES - prefix.len()));
        text
    };
    let fact = canwu_fiscal::FiscalReportRowFact {
        unit: long("unit."),
        payment_form: FiscalPaymentForm::Grain,
        assessed: canwu_fiscal::FiscalAmountEstimate {
            minimum: u64::MAX - 1,
            maximum: u64::MAX,
        },
        collected: canwu_fiscal::FiscalAmountEstimate {
            minimum: u64::MAX - 1,
            maximum: u64::MAX,
        },
        outstanding: canwu_fiscal::FiscalAmountEstimate {
            minimum: u64::MAX - 1,
            maximum: u64::MAX,
        },
    };
    let rows = (0..u32::try_from(FISCAL_REPORT_ROWS_PER_PART).expect("rows"))
        .map(|row| canwu_fiscal::FiscalReportRow {
            row: u32::MAX - row,
            institution: canwu_api::EntityRef::Domain(canwu_api::DomainRecordRef::new(
                "n".repeat(50),
                "k".repeat(50),
                "i".repeat(canwu_fiscal::MAX_FISCAL_IDENTIFIER_BYTES - 100),
            )),
            mechanism: FiscalMechanism::LandTax,
            scope_binding_id: long("scope."),
            facts: vec![fact.clone(); canwu_fiscal::MAX_FISCAL_PARTITION_KEYS],
        })
        .collect();
    let part = FiscalReportRecord::Part(canwu_fiscal::FiscalReportPart {
        observer_id: long("observer."),
        accounting_cycle_id: long("cycle."),
        slot: u16::MAX,
        part: u32::MAX,
        rows,
        books_summed: Some(u32::MAX),
    });
    let bytes = serde_json::to_vec(&part).expect("part bytes").len();
    assert!(bytes < 65_536, "a full part takes {bytes} bytes");
    let _ = Draft::against;
}
