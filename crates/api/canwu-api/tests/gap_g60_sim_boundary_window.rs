//! Gap G-60: a boundary system can count the boundaries a window of simulated
//! time has settled, is settling, and still has queued, the run keeps settled
//! boundaries as far back as a system declares, and the settled ledger is the
//! same in a live, sealed, restored, and replayed run.

#![allow(clippy::too_many_lines, clippy::unnecessary_wraps)]

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryIngressCount, BoundaryIngressKind,
    BoundaryIngressOrigin, BoundaryPhase, BoundaryProposal, BoundaryRequest,
    BoundarySystemContract, BoundaryWindow, Canwu, CanwuError, CommandContext, ErrorCode,
    IngressClass, IngressPayload, MAX_BOUNDARY_SCHEDULE_HORIZON, PayloadSchema,
    PluginActionDescriptor, PluginIngressDescriptor, PluginIngressRequest, PluginRegistrar,
    Scenario, SimDuration, SimTime, SimulationPlugin, SimulationView, StateKey, SystemCadence,
    SystemDirective,
};
use serde_json::json;

const CLOCK_PLUGIN: &str = "fixture-clock";

/// Records what `boundaries_between` shows for the boundary's day inside each
/// boundary that admits ingress, and turns a `work` packet that asks for a
/// follow-on into a packet one hour later. A boundary that only admits events
/// exits early.
fn observe_day(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    if context.admitted_ingress.is_empty() {
        return Ok(BoundaryProposal::default());
    }
    let (from, to) = day_window(context.at.day());
    let day = view.boundaries_between(from, to)?;
    let current = day
        .current
        .as_ref()
        .expect("a boundary system sees the boundary it settles");
    assert_eq!(current.boundary_id, Some(context.boundary_id));
    assert_eq!(current.at, context.at);
    let mut proposal = BoundaryProposal::default();
    proposal.directives.push(BoundaryDirective::Emit {
        event_type: "observed".to_owned(),
        summary: format!(
            "settled={} current={} scheduled={}",
            day.settled.len(),
            usize::from(day.current.is_some()),
            day.queued.len()
        ),
        affected: Vec::new(),
    });
    for id in &context.admitted_ingress {
        let Some(record) = view.ingress(*id)? else {
            continue;
        };
        if let IngressPayload::Plugin { payload, .. } = &record.payload
            && payload["followOn"] == json!(true)
        {
            proposal
                .directives
                .push(BoundaryDirective::ScheduleIngress {
                    after: SimDuration::hours(1),
                    packet_type: "work".to_owned(),
                    priority: 0,
                    payload: json!({ "followOn": false }),
                    affected: Vec::new(),
                });
        }
    }
    Ok(proposal)
}

struct ClockPlugin;

impl SimulationPlugin for ClockPlugin {
    fn name(&self) -> &'static str {
        CLOCK_PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000000a60"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_ingress(PluginIngressDescriptor {
            name: "work".to_owned(),
            description: "fixture work packet".to_owned(),
            class: IngressClass::ScheduledSystem,
            payload_schema: PayloadSchema::Any,
        })?;
        let mut contract = BoundarySystemContract::new(
            "observe-day",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        contract.reads = vec![StateKey::core_boundary_schedule(), StateKey::core_ingress()];
        contract.reads.sort();
        contract.boundary_schedule_horizon = SimDuration::days(1);
        contract.emits = vec!["observed".to_owned()];
        registrar.register_boundary_system(contract, observe_day)?;
        let mut peek = BoundarySystemContract::new(
            "peek-back",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        peek.reads = vec![StateKey::core_boundary_schedule()];
        peek.emits = vec!["peeked".to_owned()];
        registrar.register_boundary_system(peek, peek_back)
    }
}

/// Declares the read with no horizon and records whether it may look one
/// minute back: a system is held to its own horizon, not the run's.
fn peek_back(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    if context.admitted_ingress.is_empty() {
        return Ok(BoundaryProposal::default());
    }
    let one_minute_back = SimTime::from_minutes(context.at.as_minutes() - 1);
    let summary = match view.boundaries_between(one_minute_back, context.at) {
        Ok(_) => "allowed",
        Err(error) if error.code == ErrorCode::EvidenceUnavailable => "refused",
        Err(error) => return Err(error),
    };
    let mut proposal = BoundaryProposal::default();
    proposal.directives.push(BoundaryDirective::Emit {
        event_type: "peeked".to_owned(),
        summary: summary.to_owned(),
        affected: Vec::new(),
    });
    Ok(proposal)
}

fn at(day: i64, hour: i64) -> SimTime {
    SimTime::EPOCH
        .checked_add(SimDuration::days(day))
        .and_then(|time| time.checked_add(SimDuration::hours(hour)))
        .expect("fixture time should stay in range")
}

fn enqueue_work(canwu: &mut Canwu, due: SimTime, follow_on: bool) {
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            CLOCK_PLUGIN,
            "work",
            due,
            json!({ "followOn": follow_on }),
        ))
        .expect("host work should enqueue");
}

fn observations(canwu: &Canwu) -> Vec<String> {
    events_of(canwu, "observed")
}

fn events_of(canwu: &Canwu, event_type: &str) -> Vec<String> {
    canwu
        .events()
        .iter()
        .filter(|event| event.kind.field("event_type") == Some(json!(event_type)))
        .map(|event| event.summary.clone())
        .collect()
}

fn host_work(count: u64) -> BoundaryIngressCount {
    BoundaryIngressCount {
        class: IngressClass::ScheduledSystem,
        kind: BoundaryIngressKind::Plugin {
            plugin: CLOCK_PLUGIN.to_owned(),
            packet_type: "work".to_owned(),
        },
        origin: BoundaryIngressOrigin::Host,
        count,
    }
}

/// The window of simulated day `day`.
fn day_window(day: i64) -> (SimTime, SimTime) {
    (at(day, 0), at(day + 1, 0))
}

fn settled_times(day: &BoundaryWindow) -> Vec<SimTime> {
    day.settled.iter().map(|summary| summary.at).collect()
}

#[test]
fn gap_g60_sim_boundary_window() {
    let clock = ClockPlugin;
    let plugins: [&dyn SimulationPlugin; 1] = [&clock];
    let mut canwu =
        Canwu::new_with_plugins(60, Scenario::new(SimTime::EPOCH, Vec::new()), &plugins)
            .expect("the clock plugin should register");

    // Day 0: a Daily calendar boundary with two host packets at 00:00 (one
    // asks for a follow-on at 01:00), and one host packet at 12:00.
    canwu
        .schedule_calendar_boundary(at(0, 0), vec![SystemCadence::Daily])
        .expect("calendar boundary should queue");
    enqueue_work(&mut canwu, at(0, 0), true);
    enqueue_work(&mut canwu, at(0, 0), false);
    enqueue_work(&mut canwu, at(0, 12), false);

    let planned = canwu
        .boundaries_between(at(0, 0), at(1, 0))
        .expect("today's plan");
    assert!(planned.settled.is_empty() && planned.current.is_none());
    assert_eq!(
        planned
            .queued
            .iter()
            .map(|summary| (summary.at, summary.cadences.clone()))
            .collect::<Vec<_>>(),
        vec![
            (at(0, 0), vec![SystemCadence::Daily]),
            (at(0, 12), Vec::new())
        ]
    );
    assert_eq!(planned.queued[0].ingress[1], host_work(2));

    canwu
        .advance_canonical(SimDuration::hours(23))
        .expect("day 0 should settle");
    // Inside each boundary the system saw the boundaries before it, itself,
    // and the queue: the 01:00 follow-on is listed from the boundary after
    // the one that scheduled it.
    assert_eq!(
        observations(&canwu),
        vec![
            "settled=0 current=1 scheduled=1",
            "settled=1 current=1 scheduled=1",
            "settled=2 current=1 scheduled=0",
        ]
    );
    // The run keeps a day, but the zero-horizon system may not look back at
    // all; the host reads the same minute with the run's horizon.
    let peeked = events_of(&canwu, "peeked");
    assert_eq!(peeked.len(), 3);
    assert!(peeked.iter().all(|summary| summary == "refused"));
    assert_eq!(canwu.boundary_schedule_horizon(), SimDuration::days(1));
    assert!(
        canwu
            .boundaries_between(SimTime::from_minutes(at(0, 12).as_minutes() - 1), at(0, 12))
            .is_ok()
    );
    // The host settles one more boundary directly, which admits the day's
    // observations so the evidence can be sealed; it admits no ingress.
    canwu
        .settle_boundary(BoundaryRequest::at(at(0, 23)))
        .expect("a direct boundary should settle");
    let day0 = canwu
        .boundaries_between(at(0, 0), at(1, 0))
        .expect("day 0 ledger");
    assert_eq!(
        settled_times(&day0),
        vec![at(0, 0), at(0, 1), at(0, 12), at(0, 23)]
    );
    assert!(day0.current.is_none() && day0.queued.is_empty());
    assert_eq!(day0.boundary_count(), 4);
    assert!(day0.settled[3].ingress.is_empty() && day0.settled[3].cadences.is_empty());
    let opening = &day0.settled[0];
    assert_eq!(opening.cadences, vec![SystemCadence::Daily]);
    assert_eq!(
        opening.ingress[0].kind,
        BoundaryIngressKind::Calendar,
        "calendar ingress is admitted first"
    );
    assert_eq!(opening.ingress[1], host_work(2));
    assert_eq!(
        day0.settled[1].ingress[0].origin,
        BoundaryIngressOrigin::BoundarySystem,
        "the 01:00 follow-on was scheduled by a boundary system"
    );

    // Sealing removes the retained boundary records but not the ledger, and a
    // restore from the sealed checkpoint rebuilds the same ledger.
    let mut compacted = canwu.fork().into_compacted().expect("compact runtime");
    let segment = compacted
        .seal_evidence()
        .expect("an empty queue should seal")
        .expect("the retained tail should seal");
    assert_eq!(
        compacted
            .boundaries_between(at(0, 0), at(1, 0))
            .expect("sealed ledger"),
        day0
    );
    // One more day-0 boundary after the seal: the ledger then spans sealed and
    // retained boundaries, and a restore from both segments rebuilds it.
    compacted
        .settle_boundary(BoundaryRequest::at(at(0, 23) + SimDuration::minutes(30)))
        .expect("a boundary after the seal should settle");
    let continued = compacted
        .boundaries_between(at(0, 0), at(1, 0))
        .expect("continued ledger");
    assert_eq!(continued.settled.len(), 5);
    assert_eq!(continued.settled[..4], day0.settled[..]);
    let tail = compacted
        .journal_segment_since(segment.end)
        .expect("the retained tail");
    let resumed = Canwu::from_checkpoint_and_journal(
        compacted.checkpoint().expect("compact checkpoint"),
        vec![segment, tail],
    )
    .expect("the sealed run should restore");
    assert_eq!(
        resumed
            .boundaries_between(at(0, 0), at(1, 0))
            .expect("resumed ledger"),
        continued
    );

    // The run keeps one day of settled boundaries, the longest horizon its
    // system declares: on day 1 a window may still reach back one day, not
    // to the start of day 0.
    enqueue_work(&mut canwu, at(1, 0), false);
    let tomorrow = canwu
        .boundaries_between(at(1, 0), at(2, 0))
        .expect("tomorrow's queue");
    assert!(tomorrow.settled.is_empty());
    assert_eq!(tomorrow.queued.len(), 1);
    canwu
        .advance_canonical(SimDuration::hours(2))
        .expect("day 1 should open");
    let last_day = canwu
        .boundaries_between(at(0, 1), at(1, 1))
        .expect("one day back is kept");
    assert_eq!(
        settled_times(&last_day),
        vec![at(0, 1), at(0, 12), at(0, 23), at(1, 0)]
    );
    assert_eq!(
        canwu
            .boundaries_between(at(0, 0), at(1, 0))
            .expect_err("the start of day 0 is beyond the horizon")
            .code,
        ErrorCode::EvidenceUnavailable
    );
    let day1 = canwu
        .boundaries_between(at(1, 0), at(2, 0))
        .expect("day 1 ledger");
    assert_eq!(settled_times(&day1), vec![at(1, 0)]);

    // Save/load and exact replay rebuild the same ledger, a day back.
    let saved = canwu.snapshot_json().expect("the run should serialize");
    let restored =
        Canwu::from_snapshot_json_with_plugins(&saved, &plugins).expect("the run should load");
    assert_eq!(
        restored
            .boundaries_between(at(1, 0), at(2, 0))
            .expect("restored ledger"),
        day1
    );
    assert_eq!(
        restored
            .boundaries_between(at(0, 1), at(1, 1))
            .expect("restored day back"),
        last_day
    );
    let journal = serde_json::to_string(&canwu.replay_journal()).expect("journal json");
    let replayed =
        Canwu::replay_from_journal_json(&plugins, &journal).expect("the run should replay");
    assert_eq!(
        replayed
            .boundaries_between(at(1, 0), at(2, 0))
            .expect("replayed ledger"),
        day1
    );
    assert_eq!(
        replayed
            .boundaries_between(at(0, 1), at(1, 1))
            .expect("replayed day back"),
        last_day
    );
    assert_eq!(observations(&replayed), observations(&canwu));
    assert_eq!(canwu.checkpoint_hash(), replayed.checkpoint_hash());
}

fn peek_schedule(
    _view: &SimulationView<'_>,
    _context: &CommandContext,
    _payload: &serde_json::Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    Ok(Vec::new())
}

struct PeekingCommandPlugin;

impl SimulationPlugin for PeekingCommandPlugin {
    fn name(&self) -> &'static str {
        "fixture-peek"
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000000a61"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_command(
            PluginActionDescriptor {
                name: "peek".to_owned(),
                description: "a command that would reply with the queue".to_owned(),
                payload_schema: PayloadSchema::Any,
                reads: vec![StateKey::core_boundary_schedule()],
                writes: Vec::new(),
            },
            peek_schedule,
        )
    }
}

/// The read names other plugins' queued work, so a command handler, whose
/// outcome reaches the issuing actor, may not declare it.
#[test]
fn gap_g60_plugin_commands_cannot_read_the_boundary_schedule() {
    let peek = PeekingCommandPlugin;
    let plugins: [&dyn SimulationPlugin; 1] = [&peek];
    let error = Canwu::new_with_plugins(61, Scenario::new(SimTime::EPOCH, Vec::new()), &plugins)
        .err()
        .expect("the registration should be refused");
    assert_eq!(error.code, ErrorCode::InvalidPluginRegistration);
}

struct HorizonPlugin {
    horizon: SimDuration,
    reads_schedule: bool,
}

impl SimulationPlugin for HorizonPlugin {
    fn name(&self) -> &'static str {
        "fixture-horizon"
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000000a62"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut contract = BoundarySystemContract::new(
            "look-back",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        if self.reads_schedule {
            contract.reads = vec![StateKey::core_boundary_schedule()];
        }
        contract.boundary_schedule_horizon = self.horizon;
        registrar.register_boundary_system(contract, observe_day)
    }
}

/// A horizon is bounded and needs the read it serves.
#[test]
fn gap_g60_boundary_schedule_horizon_is_declared_with_its_read() {
    for (horizon, reads_schedule) in [
        (SimDuration::days(1), false),
        (
            MAX_BOUNDARY_SCHEDULE_HORIZON + SimDuration::minutes(1),
            true,
        ),
        (SimDuration::minutes(-1), true),
    ] {
        let plugin = HorizonPlugin {
            horizon,
            reads_schedule,
        };
        let plugins: [&dyn SimulationPlugin; 1] = [&plugin];
        let error =
            Canwu::new_with_plugins(62, Scenario::new(SimTime::EPOCH, Vec::new()), &plugins)
                .err()
                .expect("the registration should be refused");
        assert_eq!(error.code, ErrorCode::InvalidPluginRegistration);
    }
    let plugin = HorizonPlugin {
        horizon: MAX_BOUNDARY_SCHEDULE_HORIZON,
        reads_schedule: true,
    };
    let plugins: [&dyn SimulationPlugin; 1] = [&plugin];
    assert!(
        Canwu::new_with_plugins(62, Scenario::new(SimTime::EPOCH, Vec::new()), &plugins).is_ok()
    );
}
