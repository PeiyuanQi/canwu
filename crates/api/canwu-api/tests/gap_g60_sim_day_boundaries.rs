//! Gap G-60: a boundary system can count the boundaries its simulated day has
//! settled, is settling, and still has queued, and the settled ledger is the
//! same in a live, sealed, restored, and replayed run.

#![allow(clippy::too_many_lines, clippy::unnecessary_wraps)]

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryIngressCount, BoundaryIngressKind,
    BoundaryIngressOrigin, BoundaryPhase, BoundaryProposal, BoundaryRequest,
    BoundarySystemContract, Canwu, CanwuError, CommandContext, DayBoundaries, ErrorCode,
    IngressClass, IngressPayload, PayloadSchema, PluginActionDescriptor, PluginIngressDescriptor,
    PluginIngressRequest, PluginRegistrar, Scenario, SimDuration, SimTime, SimulationPlugin,
    SimulationView, StateKey, SystemCadence, SystemDirective,
};
use serde_json::json;

const CLOCK_PLUGIN: &str = "fixture-clock";

/// Records what `boundaries_on_day` shows inside each boundary that admits
/// ingress, and turns a `work` packet that asks for a follow-on into a packet
/// one hour later. A boundary that only admits events exits early.
fn observe_day(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    if context.admitted_ingress.is_empty() {
        return Ok(BoundaryProposal::default());
    }
    let day = view.boundaries_on_day(context.at.day())?;
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
        contract.emits = vec!["observed".to_owned()];
        registrar.register_boundary_system(contract, observe_day)
    }
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
    canwu
        .events()
        .iter()
        .filter(|event| event.kind.field("event_type") == Some(json!("observed")))
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

fn settled_times(day: &DayBoundaries) -> Vec<SimTime> {
    day.settled.iter().map(|summary| summary.at).collect()
}

#[test]
fn gap_g60_sim_day_boundaries() {
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

    let planned = canwu.boundaries_on_day(0).expect("today's plan");
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
    // The host settles one more boundary directly, which admits the day's
    // observations so the evidence can be sealed; it admits no ingress.
    canwu
        .settle_boundary(BoundaryRequest::at(at(0, 23)))
        .expect("a direct boundary should settle");
    let day0 = canwu.boundaries_on_day(0).expect("day 0 ledger");
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
    assert_eq!(compacted.boundaries_on_day(0).expect("sealed ledger"), day0);
    // One more day-0 boundary after the seal: the ledger then spans sealed and
    // retained boundaries, and a restore from both segments rebuilds it.
    compacted
        .settle_boundary(BoundaryRequest::at(at(0, 23) + SimDuration::minutes(30)))
        .expect("a boundary after the seal should settle");
    let continued = compacted.boundaries_on_day(0).expect("continued ledger");
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
        resumed.boundaries_on_day(0).expect("resumed ledger"),
        continued
    );

    // Day 1 starts a new ledger; day 0 is no longer kept.
    enqueue_work(&mut canwu, at(1, 0), false);
    let tomorrow = canwu.boundaries_on_day(1).expect("tomorrow's queue");
    assert!(tomorrow.settled.is_empty());
    assert_eq!(tomorrow.queued.len(), 1);
    canwu
        .advance_canonical(SimDuration::hours(2))
        .expect("day 1 should open");
    assert_eq!(
        canwu
            .boundaries_on_day(0)
            .expect_err("day 0 has passed")
            .code,
        ErrorCode::EvidenceUnavailable
    );
    let day1 = canwu.boundaries_on_day(1).expect("day 1 ledger");
    assert_eq!(settled_times(&day1), vec![at(1, 0)]);

    // Save/load and exact replay rebuild the same ledger.
    let saved = canwu.snapshot_json().expect("the run should serialize");
    let restored =
        Canwu::from_snapshot_json_with_plugins(&saved, &plugins).expect("the run should load");
    assert_eq!(
        restored.boundaries_on_day(1).expect("restored ledger"),
        day1
    );
    let journal = serde_json::to_string(&canwu.replay_journal()).expect("journal json");
    let replayed =
        Canwu::replay_from_journal_json(&plugins, &journal).expect("the run should replay");
    assert_eq!(
        replayed.boundaries_on_day(1).expect("replayed ledger"),
        day1
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
