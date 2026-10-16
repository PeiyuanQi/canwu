//! Report cadence: an `Interval` observer's report is due at
//! `offset + k * every`, and is published by the first boundary at or after
//! each due time that runs the report system, never in the boundary that
//! changes what it shows. Its one queue entry carries its next due time;
//! settlement merges what a change owes into it without moving that time,
//! and the report system queues it again after every due time, whether or
//! not anything was published. At a due time with nothing changed, the
//! summary alone is published, so that its `as_of` advances. No future-dated
//! packet is ever scheduled. The report schedule and the root's earliest due
//! time say which queue pages are due, so a boundary before every due time
//! writes and publishes nothing. A host whose boundaries can pass without
//! events also runs the report system on a cadence of its own
//! (`FiscalPlugin::with_report_cadence`), which the plugin descriptor
//! records. Parked holders are checked again in rotation, so a revived
//! holder is published to within `ceil(parked / 16)` report-system
//! boundaries. Every step is audited (`fiscal_state`), and a sealed run
//! publishes what its replay does.

mod support;

use canwu_api::{
    BoundaryReceipt, BoundaryRequest, Canwu, CompactedCanwu, ErrorCode, KnowledgeHolderRef,
    PersonId, SimDuration, SimTime, SystemCadence,
};
use canwu_fiscal::{
    FISCAL_PARKED_CHECKS_PER_BOUNDARY, FiscalAction, FiscalObserverBinding, FiscalReport,
    FiscalReportCadence, FiscalReportRecord, FiscalState, compute_projection, fiscal_reports,
};
use std::collections::BTreeSet;
use support::{
    AUDITOR, DEPUTY, Fixture, GRANARY_AUTHORITY, GRANARY_PRINCIPAL, GRANARY_SCOPE, PRINCIPAL,
    SUCCESSOR, TREASURY_AUTHORITY, assessment, draft, enqueue, enqueue_life, enqueue_tick,
    fiscal_record_versions, fiscal_state, granary, life_at, observer_binding,
    published_report_records, scenario, settle, submit_sealed, tick_at, treasury, treasury_scope,
};

/// The deputy's interval report of the treasury, due at 5, 15, 25, ...
/// minutes.
const TREASURY_INTERVAL: &str = "observer.interval.treasury";
/// The auditor's interval report of the granary, which has no book at first.
const GRANARY_INTERVAL: &str = "observer.interval.granary";

fn interval(id: &str, actor: PersonId, institution: canwu_api::EntityRef) -> FiscalObserverBinding {
    FiscalObserverBinding {
        cadence: FiscalReportCadence::Interval {
            every: SimDuration::minutes(10),
            offset: SimDuration::minutes(5),
        },
        ..observer_binding(id, actor, BTreeSet::from([institution]))
    }
}

fn interval_scenario() -> canwu_api::Scenario {
    scenario(|state| {
        state.add_observer(interval(TREASURY_INTERVAL, DEPUTY, treasury()));
        state.add_observer(interval(GRANARY_INTERVAL, AUDITOR, granary()));
    })
}

fn minute(minutes: i64) -> SimTime {
    SimTime::from_minutes(minutes)
}

/// Moves the run's clock to `at` without settling a boundary.
fn wait_until(canwu: &mut Canwu, at: i64) {
    let receipts = canwu
        .advance_canonical(SimDuration::minutes(at - canwu.time().as_minutes()))
        .expect("the clock moves");
    assert!(receipts.is_empty(), "nothing was due before {at}");
    assert_eq!(canwu.time(), minute(at));
}

/// An event-bearing boundary at the run's time that changes no fiscal
/// record.
fn tick(canwu: &mut Canwu) -> Vec<BoundaryReceipt> {
    enqueue_tick(canwu);
    settle(canwu)
}

fn treasury_assessment(action_id: &str, scope: usize) -> (PersonId, support::Draft) {
    (
        PRINCIPAL,
        assessment(action_id, TREASURY_AUTHORITY, &treasury_scope(scope), "c1"),
    )
}

/// The report of `observer` that `actor` holds, if any.
fn report(canwu: &Canwu, actor: PersonId, observer: &str) -> Option<FiscalReport> {
    let viewer = canwu.viewer_for_actor(actor).expect("viewer");
    fiscal_reports(&viewer).expect("reports").remove(observer)
}

/// The report records published to `actor` so far.
fn published(canwu: &Canwu, actor: PersonId) -> usize {
    published_report_records(canwu, actor).len()
}

/// Asserts that the report is complete, stands as of `at`, and holds what
/// the records derive now: the projection invariant at the publishing
/// boundary.
fn assert_published_at(
    canwu: &Canwu,
    state: &FiscalState,
    at: i64,
    actor: PersonId,
    observer: &str,
) {
    let report = report(canwu, actor, observer).expect("a report");
    assert!(report.is_complete());
    assert_eq!(report.projection.as_of, minute(at), "{observer}");
    let derived = compute_projection(state, observer, minute(at))
        .expect("projection")
        .expect("an observer");
    assert_eq!(report.projection.facts, derived.facts, "{observer}");
}

fn due_at(state: &FiscalState, observer: &str) -> SimTime {
    state.report_queue[observer].due_at
}

#[test]
#[allow(clippy::too_many_lines)]
fn an_interval_report_is_published_at_its_due_times_only() {
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(interval_scenario());
    let mut receipts = Vec::new();
    let state = fiscal_state(&canwu);
    assert_eq!(due_at(&state, TREASURY_INTERVAL), minute(5));
    assert_eq!(state.root.earliest_due, Some(minute(0)));

    // A change at 0 publishes the on-change report at once; the interval
    // report waits for 5, with the change merged into its entry.
    enqueue(&mut canwu, vec![treasury_assessment("first", 0)]);
    receipts.extend(settle(&mut canwu));
    assert!(report(&canwu, PRINCIPAL, support::OBSERVER).is_some());
    assert!(report(&canwu, DEPUTY, TREASURY_INTERVAL).is_none());
    let state = fiscal_state(&canwu);
    assert_eq!(due_at(&state, TREASURY_INTERVAL), minute(5));
    assert!(state.report_queue[TREASURY_INTERVAL].whole);
    assert_eq!(state.root.earliest_due, Some(minute(5)));

    // Boundaries before the due time read the root and write nothing.
    tick(&mut canwu);
    let versions = fiscal_record_versions(&canwu);
    for _ in 0..2 {
        receipts.extend(tick(&mut canwu));
    }
    assert_eq!(fiscal_record_versions(&canwu), versions);
    assert!(report(&canwu, DEPUTY, TREASURY_INTERVAL).is_none());

    // At 5 the interval report is published as of its due boundary; the
    // granary's, with no book, is not, and both are due again at 15.
    wait_until(&mut canwu, 5);
    receipts.extend(tick(&mut canwu));
    let state = fiscal_state(&canwu);
    assert_eq!(
        report(&canwu, DEPUTY, TREASURY_INTERVAL)
            .expect("the treasury report")
            .projection
            .as_of,
        minute(5)
    );
    assert!(report(&canwu, AUDITOR, GRANARY_INTERVAL).is_none());
    assert_eq!(due_at(&state, TREASURY_INTERVAL), minute(15));
    assert_eq!(due_at(&state, GRANARY_INTERVAL), minute(15));
    assert_eq!(state.root.earliest_due, Some(minute(15)));
    let at_five = report(&canwu, DEPUTY, TREASURY_INTERVAL).expect("the treasury report");
    assert_eq!(at_five.projection.facts.len(), 1);

    // Changes between due times go out on change to the on-change report
    // only, and neither interval entry's due time moves.
    wait_until(&mut canwu, 7);
    enqueue(&mut canwu, vec![treasury_assessment("second", 1)]);
    receipts.extend(settle(&mut canwu));
    enqueue(
        &mut canwu,
        vec![(
            GRANARY_PRINCIPAL,
            assessment("granary", GRANARY_AUTHORITY, GRANARY_SCOPE, "c1"),
        )],
    );
    receipts.extend(settle(&mut canwu));
    let state = fiscal_state(&canwu);
    assert_eq!(due_at(&state, TREASURY_INTERVAL), minute(15));
    assert_eq!(due_at(&state, GRANARY_INTERVAL), minute(15));
    assert_eq!(
        report(&canwu, DEPUTY, TREASURY_INTERVAL).expect("the treasury report"),
        at_five
    );
    assert!(report(&canwu, AUDITOR, GRANARY_INTERVAL).is_none());
    assert_eq!(
        report(&canwu, PRINCIPAL, support::OBSERVER)
            .expect("the on-change report")
            .projection
            .facts
            .len(),
        2
    );

    // At 15 both interval reports are published with what changed.
    wait_until(&mut canwu, 15);
    receipts.extend(tick(&mut canwu));
    let state = fiscal_state(&canwu);
    assert_published_at(&canwu, &state, 15, DEPUTY, TREASURY_INTERVAL);
    assert_published_at(&canwu, &state, 15, AUDITOR, GRANARY_INTERVAL);
    assert_eq!(
        report(&canwu, DEPUTY, TREASURY_INTERVAL)
            .expect("the treasury report")
            .projection
            .facts
            .len(),
        2
    );

    // At 25 nothing changed: each holder gets its summary alone, as of 25.
    wait_until(&mut canwu, 25);
    let before = (published(&canwu, DEPUTY), published(&canwu, AUDITOR));
    receipts.extend(tick(&mut canwu));
    assert_eq!(
        (published(&canwu, DEPUTY), published(&canwu, AUDITOR)),
        (before.0 + 1, before.1 + 1)
    );
    let summary = published_report_records(&canwu, DEPUTY)
        .pop()
        .expect("the last record")
        .1;
    assert!(matches!(summary, FiscalReportRecord::Summary(ref summary)
        if summary.as_of == minute(25) && summary.cadence.is_interval()));
    let state = fiscal_state(&canwu);
    assert_published_at(&canwu, &state, 25, DEPUTY, TREASURY_INTERVAL);

    // No boundary at 35: the first event-bearing boundary after it publishes,
    // and the next due time is 45, not ten minutes after that boundary.
    wait_until(&mut canwu, 38);
    receipts.extend(tick(&mut canwu));
    let state = fiscal_state(&canwu);
    assert_published_at(&canwu, &state, 38, DEPUTY, TREASURY_INTERVAL);
    assert_eq!(due_at(&state, TREASURY_INTERVAL), minute(45));

    // No report was ever scheduled as a future-dated packet.
    assert!(
        receipts
            .iter()
            .all(|receipt| receipt.generated_ingress.is_empty())
    );
    fixture.assert_restore_and_replay(&canwu);
}

/// A report due at a time no event-bearing boundary follows is published at
/// the next boundary of the host's report cadence, which wakes the report
/// system with a packet due at once, and not without it; a boundary of the
/// cadence with nothing due wakes nothing. The cadence is in the plugin
/// descriptor, and `EventDriven` is refused.
#[test]
fn a_report_due_without_events_waits_for_the_host_report_cadence() {
    let cadence = Fixture::with_report_cadence(SystemCadence::Monthly);
    let plain = Fixture::new();
    let mut runs = [
        cadence.runtime(interval_scenario()),
        plain.runtime(interval_scenario()),
    ];
    let monthly = |canwu: &mut Canwu| {
        let receipt = canwu
            .settle_boundary(BoundaryRequest {
                at: canwu.time(),
                cadences: vec![SystemCadence::Monthly],
            })
            .expect("a monthly boundary without events");
        // The boundaries the wake brings at the same instant.
        let woken = canwu
            .advance_canonical(SimDuration::ZERO)
            .expect("the woken boundaries");
        (receipt.generated_ingress.len(), woken.len())
    };
    for (index, canwu) in runs.iter_mut().enumerate() {
        enqueue(canwu, vec![treasury_assessment("first", 0)]);
        settle(canwu);
        // A boundary that admits what the last one emitted, and emits
        // nothing: the next one has no event.
        canwu
            .settle_boundary(BoundaryRequest::at(canwu.time()))
            .expect("a quiet boundary");
        assert_eq!(monthly(canwu), (0, 0), "nothing is due before 5");
        wait_until(canwu, 6);
        let quiet = canwu
            .settle_boundary(BoundaryRequest::at(canwu.time()))
            .expect("a boundary without events or cadence");
        assert_eq!(quiet.knowledge_record_count, 0);
        let expected = if index == 0 { (1, 1) } else { (0, 0) };
        assert_eq!(monthly(canwu), expected);
        assert_eq!(canwu.time(), minute(6));
    }
    let [with_cadence, without] = runs;
    let state = fiscal_state(&with_cadence);
    assert_published_at(&with_cadence, &state, 6, DEPUTY, TREASURY_INTERVAL);
    assert_eq!(due_at(&state, TREASURY_INTERVAL), minute(15));
    assert!(report(&without, DEPUTY, TREASURY_INTERVAL).is_none());
    assert_eq!(
        due_at(&fiscal_state(&without), TREASURY_INTERVAL),
        minute(5)
    );

    // The cadence is part of the plugin descriptor.
    let snapshot = with_cadence.snapshot_json().expect("snapshot");
    assert!(Canwu::from_snapshot_json_with_plugins(&snapshot, &plain.plugins()).is_err());
    cadence.assert_restore_and_replay(&with_cadence);
    let error = Fixture::with_report_cadence(SystemCadence::EventDriven)
        .try_runtime(interval_scenario())
        .err()
        .expect("an event-driven report cadence is refused");
    assert_eq!(error.code, ErrorCode::InvalidPluginRegistration, "{error}");
}

/// Due times and the rotation are read from the fiscal records alone: a run
/// that sealed its evidence between due times publishes at the next one
/// exactly as its unsealed fork, a restore from the sealed segments, and
/// its own replay do.
#[test]
fn a_sealed_run_publishes_interval_reports_as_its_replay_does() {
    let fixture = Fixture::new();
    let plugins = fixture.plugins();
    let mut base = fixture.runtime(interval_scenario());
    enqueue(&mut base, vec![treasury_assessment("first", 0)]);
    settle(&mut base);
    wait_until(&mut base, 5);
    tick(&mut base);
    wait_until(&mut base, 7);
    enqueue(&mut base, vec![treasury_assessment("second", 1)]);
    settle(&mut base);
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

    wait_until(&mut unsealed, 15);
    tick(&mut unsealed);
    for run in [&mut sealed, &mut from_sealed] {
        run.advance_canonical(SimDuration::minutes(15 - run.time().as_minutes()))
            .expect("the clock moves");
        run.enqueue_plugin_ingress(tick_at(run.time()))
            .expect("a tick");
        run.advance_canonical(SimDuration::minutes(1))
            .expect("the due boundary");
    }
    let state = fiscal_state(&unsealed);
    assert_published_at(&unsealed, &state, 15, DEPUTY, TREASURY_INTERVAL);
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

    let later = vec![treasury_assessment("third", 2)];
    enqueue(&mut unsealed, later.clone());
    settle(&mut unsealed);
    submit_sealed(&mut sealed, later);
    assert_eq!(sealed.checkpoint_hash(), unsealed.checkpoint_hash());
    let journal = sealed
        .replay_journal_with_segments(vec![segment])
        .expect("the sealed run produces an exact replay journal");
    let replayed =
        Canwu::replay_from_journal(&plugins, &journal).expect("the sealed run replays exactly");
    assert_eq!(replayed.snapshot(), unsealed.snapshot());
}

/// Forty observers of one dead holder are parked. Each run of the report
/// system checks the next sixteen after the schedule's cursor, so once the
/// holder lives again the rotation reaches every one of them within
/// `ceil(40 / 16)` report-system boundaries, sixteen in each, not all in the
/// first. A sealed run does it as its replay does.
#[test]
fn the_parked_rotation_reaches_a_revived_holder_within_its_bound() {
    const PARKED: usize = 40;
    let fixture = Fixture::new();
    let plugins = fixture.plugins();
    let mut base = fixture.runtime(scenario(|state| {
        for index in 0..PARKED {
            state.add_observer(observer_binding(
                &format!("observer.deputy.{index:02}"),
                DEPUTY,
                BTreeSet::from([treasury()]),
            ));
        }
    }));
    enqueue_life(&mut base, DEPUTY, false);
    settle(&mut base);
    enqueue(&mut base, vec![treasury_assessment("first", 0)]);
    settle(&mut base);
    let state = fiscal_state(&base);
    assert_eq!(
        state
            .report_queue
            .values()
            .filter(|entry| entry.parked)
            .count(),
        PARKED
    );
    assert!(state.root.parked);
    assert!(report(&base, DEPUTY, "observer.deputy.00").is_none());
    // While he is dead, each run moves the cursor and publishes nothing.
    tick(&mut base);
    let cursor = fiscal_state(&base).root.parked_cursor;
    assert!(cursor.is_some());
    tick(&mut base);
    assert_ne!(fiscal_state(&base).root.parked_cursor, cursor);
    assert_eq!(published(&base, DEPUTY), 0);

    let mut unsealed = base.fork();
    let mut sealed = base.into_compacted().expect("compact interface");
    let segment = sealed
        .seal_evidence()
        .expect("the settled history seals")
        .expect("the settled history holds evidence");
    let reports = |canwu: &Canwu| {
        let viewer = canwu.viewer_for_actor(DEPUTY).expect("viewer");
        fiscal_reports(&viewer).expect("reports").len()
    };
    // The reviving boundary, then report-system boundaries of a tick each.
    let mut reached = Vec::new();
    for round in 0..PARKED.div_ceil(FISCAL_PARKED_CHECKS_PER_BOUNDARY) {
        let ingress = |at| {
            if round == 0 {
                life_at(at, DEPUTY, true)
            } else {
                tick_at(at)
            }
        };
        unsealed
            .enqueue_plugin_ingress(ingress(unsealed.time()))
            .expect("unsealed ingress");
        sealed
            .enqueue_plugin_ingress(ingress(sealed.time()))
            .expect("sealed ingress");
        settle(&mut unsealed);
        sealed
            .advance_canonical(SimDuration::minutes(1))
            .expect("the sealed boundary");
        assert_eq!(sealed.checkpoint_hash(), unsealed.checkpoint_hash());
        reached.push(reports(&unsealed));
    }
    assert_eq!(reached, [16, 32, 40]);
    let state = fiscal_state(&unsealed);
    assert!(!state.root.parked);
    assert_eq!(state.root.parked_cursor, None);
    let viewer = unsealed.viewer_for_actor(DEPUTY).expect("viewer");
    for (observer, report) in fiscal_reports(&viewer).expect("reports") {
        let derived = compute_projection(&state, &observer, unsealed.time())
            .expect("projection")
            .expect("an observer");
        assert!(report.is_complete());
        assert_eq!(report.projection.facts, derived.facts, "{observer}");
    }
    let journal = sealed
        .replay_journal_with_segments(vec![segment])
        .expect("the sealed run produces an exact replay journal");
    let replayed =
        Canwu::replay_from_journal(&plugins, &journal).expect("the sealed run replays exactly");
    assert_eq!(replayed.snapshot(), unsealed.snapshot());
}

/// A parked holder alone wakes nothing on the host's report cadence: with
/// nothing due, a quiet boundary of the cadence schedules no packet and
/// writes nothing, however often it comes. The rotation advances in the
/// runs of the report system that happen anyway: the revival is itself an
/// event, and the boundary that admits it publishes what he is owed.
#[test]
fn a_parked_holder_alone_wakes_nothing_on_the_host_cadence() {
    const PARKED: &str = "observer.deputy";
    let fixture = Fixture::with_report_cadence(SystemCadence::Monthly);
    let mut canwu = fixture.runtime(scenario(|state| {
        state.add_observer(observer_binding(
            PARKED,
            DEPUTY,
            BTreeSet::from([treasury()]),
        ));
    }));
    enqueue_life(&mut canwu, DEPUTY, false);
    settle(&mut canwu);
    enqueue(&mut canwu, vec![treasury_assessment("first", 0)]);
    settle(&mut canwu);
    let state = fiscal_state(&canwu);
    assert!(state.root.parked && state.report_queue[PARKED].parked);
    assert_eq!(state.root.earliest_due, None);
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("a quiet boundary");
    let versions = fiscal_record_versions(&canwu);
    for _ in 0..3 {
        let receipt = canwu
            .settle_boundary(BoundaryRequest {
                at: canwu.time(),
                cadences: vec![SystemCadence::Monthly],
            })
            .expect("a monthly boundary without events");
        assert!(receipt.generated_ingress.is_empty());
        let later = canwu
            .advance_canonical(SimDuration::minutes(1))
            .expect("the clock moves");
        assert!(later.is_empty(), "no boundary was woken");
    }
    assert_eq!(fiscal_record_versions(&canwu), versions);

    enqueue_life(&mut canwu, DEPUTY, true);
    settle(&mut canwu);
    assert!(
        report(&canwu, DEPUTY, PARKED).is_some_and(|report| report.is_complete()),
        "the reviving boundary publishes what he is owed"
    );
    assert!(!fiscal_state(&canwu).root.parked);
}

/// Removing an interval binding withdraws its report at once, since a
/// removed binding has no due times, and leaves no entry, then or after a
/// later change of what it saw. Re-pointing one keeps its due time: the old
/// holder keeps its report, and the new one gets the whole report, at the
/// next due time.
#[test]
fn a_removed_interval_binding_is_withdrawn_at_once_and_a_rebound_one_at_its_due_time() {
    const REMOVED: &str = "observer.interval.removed";
    const REBOUND: &str = "observer.interval.rebound";
    let controlled = |id: &str, actor| FiscalObserverBinding {
        controlled_by: Some(treasury()),
        ..interval(id, actor, treasury())
    };
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(scenario(|state| {
        state.add_observer(controlled(REMOVED, DEPUTY));
        state.add_observer(controlled(REBOUND, AUDITOR));
    }));
    enqueue(&mut canwu, vec![treasury_assessment("first", 0)]);
    settle(&mut canwu);
    wait_until(&mut canwu, 5);
    tick(&mut canwu);
    assert!(report(&canwu, DEPUTY, REMOVED).is_some());
    let before = report(&canwu, AUDITOR, REBOUND).expect("the rebound binding's report");

    // Between due times, one binding is removed and the other re-pointed.
    wait_until(&mut canwu, 7);
    enqueue(
        &mut canwu,
        vec![
            (
                PRINCIPAL,
                draft(
                    "remove",
                    TREASURY_AUTHORITY,
                    FiscalAction::RemoveObserver {
                        observer_binding_id: REMOVED.to_owned(),
                    },
                ),
            ),
            (
                PRINCIPAL,
                draft(
                    "rebind",
                    TREASURY_AUTHORITY,
                    FiscalAction::RebindObserver {
                        observer_binding_id: REBOUND.to_owned(),
                        actor: Some(SUCCESSOR),
                        knowledge_holder: KnowledgeHolderRef::Person(SUCCESSOR),
                    },
                ),
            ),
        ],
    );
    settle(&mut canwu);
    let state = fiscal_state(&canwu);
    assert!(
        report(&canwu, DEPUTY, REMOVED).is_none(),
        "withdrawn at once"
    );
    assert!(!state.report_queue.contains_key(REMOVED));
    assert_eq!(report(&canwu, AUDITOR, REBOUND), Some(before));
    assert!(report(&canwu, SUCCESSOR, REBOUND).is_none());
    assert_eq!(due_at(&state, REBOUND), minute(15));

    // A later change of the institution queues nothing for the removed one.
    enqueue(&mut canwu, vec![treasury_assessment("second", 1)]);
    settle(&mut canwu);
    assert!(!fiscal_state(&canwu).report_queue.contains_key(REMOVED));

    // At the due time the old holder's report is withdrawn and the new
    // holder gets the whole report.
    wait_until(&mut canwu, 15);
    tick(&mut canwu);
    let state = fiscal_state(&canwu);
    assert!(report(&canwu, AUDITOR, REBOUND).is_none());
    assert_published_at(&canwu, &state, 15, SUCCESSOR, REBOUND);
    fixture.assert_restore_and_replay(&canwu);
}
