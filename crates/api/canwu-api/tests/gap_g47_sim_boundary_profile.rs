//! Gap G-47: a host can ask for a wall-time profile of each settled boundary,
//! listing every plugin handler the boundary ran (boundary systems, plugin
//! command handlers, and event-driven plugin systems) with its plugin and
//! elapsed time. The profile is a host diagnostic beside the runtime:
//! turning it on changes no receipt, evidence, hash, snapshot, or replay
//! journal, a failed boundary leaves none, and it is off unless asked for.

#![allow(clippy::too_many_lines, clippy::unnecessary_wraps)]

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryHandlerKind, BoundaryPhase, BoundaryProfile,
    BoundaryProposal, BoundaryReceipt, BoundaryRequest, BoundarySystemContract, Canwu, CanwuError,
    Command, CommandContext, CommandEnvelope, CommandRequest, CommandRequestId, ErrorCode, Issuer,
    MAX_RETAINED_BOUNDARY_PROFILES, PayloadSchema, PluginActionDescriptor, PluginRegistrar,
    Scenario, SimDuration, SimEvent, SimTime, SimulationPlugin, SimulationView, SystemCadence,
    SystemContract, SystemDirective,
};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

const LEDGER: &str = "fixture-profile-ledger";
const WATCH: &str = "fixture-profile-watch";
/// How long the slow system keeps running: a floor for its profiled time.
const SLOW: Duration = Duration::from_millis(2);

/// Runs until `SLOW` has passed on the monotonic clock, the clock the
/// profiler reads before and after this call, so the profiled time cannot be
/// shorter.
fn slow(
    _view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let started = Instant::now();
    while started.elapsed() < SLOW {
        std::hint::spin_loop();
    }
    Ok(BoundaryProposal::default())
}

fn idle(
    _view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    Ok(BoundaryProposal::default())
}

fn watch(
    _view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut proposal = BoundaryProposal::default();
    proposal.directives.push(BoundaryDirective::Emit {
        event_type: "watched".to_owned(),
        summary: format!("watched boundary {}", context.boundary_id.get()),
        affected: Vec::new(),
    });
    Ok(proposal)
}

fn veto(
    _view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    Err(CanwuError::new(
        ErrorCode::InvalidBoundary,
        "the fixture vetoes Annual boundaries",
    ))
}

/// A plugin command whose accepted effect is one event.
fn note(
    _view: &SimulationView<'_>,
    _context: &CommandContext,
    _payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    Ok(vec![SystemDirective::Emit {
        event_type: "noted".to_owned(),
        summary: "the host left a note".to_owned(),
        affected: Vec::new(),
    }])
}

/// An event-driven system: it runs for every event a command emits.
fn echo(_view: &SimulationView<'_>, _event: &SimEvent) -> Result<Vec<SystemDirective>, CanwuError> {
    Ok(Vec::new())
}

struct LedgerPlugin;

impl SimulationPlugin for LedgerPlugin {
    fn name(&self) -> &'static str {
        LEDGER
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000000a47"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_boundary_system(
            BoundarySystemContract::new(
                "post",
                BoundaryPhase::DomainDeltaProposal,
                SystemCadence::Daily,
            ),
            slow,
        )?;
        registrar.register_boundary_system(
            BoundarySystemContract::new(
                "summarize",
                BoundaryPhase::StrategicAggregation,
                SystemCadence::Daily,
            ),
            idle,
        )?;
        registrar.register_boundary_system(
            BoundarySystemContract::new(
                "close-month",
                BoundaryPhase::StrategicAggregation,
                SystemCadence::Monthly,
            ),
            idle,
        )
    }
}

struct WatchPlugin;

impl SimulationPlugin for WatchPlugin {
    fn name(&self) -> &'static str {
        WATCH
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000000a48"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut watching = BoundarySystemContract::new(
            "watch",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        watching.emits = vec!["watched".to_owned()];
        registrar.register_boundary_system(watching, watch)?;
        registrar.register_boundary_system(
            BoundarySystemContract::new(
                "veto",
                BoundaryPhase::PerspectiveAndReportMaterialization,
                SystemCadence::Annual,
            ),
            veto,
        )?;
        registrar.register_command(
            PluginActionDescriptor {
                name: "note".to_owned(),
                description: "emit one event".to_owned(),
                payload_schema: PayloadSchema::Any,
                reads: Vec::new(),
                writes: Vec::new(),
            },
            note,
        )?;
        registrar.register_system(
            SystemContract::event_driven(
                "echo",
                BoundaryPhase::PerspectiveAndReportMaterialization,
            ),
            echo,
        )
    }
}

fn day(day: i64) -> SimTime {
    SimTime::EPOCH + SimDuration::days(day)
}

/// Two days: a Daily boundary, then a Daily and Monthly one that also admits
/// a queued `note` command.
fn run_two_days(canwu: &mut Canwu) -> Vec<BoundaryReceipt> {
    canwu
        .schedule_calendar_boundary(day(0), vec![SystemCadence::Daily])
        .expect("day 0 should queue");
    canwu
        .schedule_calendar_boundary(day(1), vec![SystemCadence::Daily, SystemCadence::Monthly])
        .expect("day 1 should queue");
    let first = canwu
        .step_canonical()
        .expect("day 0 should settle")
        .expect("day 0 has a boundary");
    canwu
        .enqueue_command(
            day(1),
            0,
            CommandRequest::new(
                CommandRequestId::new(1),
                canwu.revision(),
                CommandEnvelope::new(
                    Issuer::System("fixture-host".to_owned()),
                    Command::Plugin {
                        plugin: WATCH.to_owned(),
                        command: "note".to_owned(),
                        payload: json!({}),
                    },
                ),
            ),
        )
        .expect("the note should queue");
    let second = canwu
        .step_canonical()
        .expect("day 1 should settle")
        .expect("day 1 has a boundary");
    vec![first, second]
}

type Ran<'a> = (BoundaryHandlerKind, Option<BoundaryPhase>, &'a str, &'a str);

fn ran(profile: &BoundaryProfile) -> Vec<Ran<'_>> {
    profile
        .handlers
        .iter()
        .map(|timing| {
            (
                timing.kind,
                timing.phase,
                timing.plugin.as_str(),
                timing.name.as_str(),
            )
        })
        .collect()
}

fn system(phase: BoundaryPhase, plugin: &'static str, name: &'static str) -> Ran<'static> {
    (
        BoundaryHandlerKind::BoundarySystem,
        Some(phase),
        plugin,
        name,
    )
}

#[test]
fn gap_g47_sim_boundary_profile() {
    let ledger = LedgerPlugin;
    let watcher = WatchPlugin;
    let plugins: [&dyn SimulationPlugin; 2] = [&ledger, &watcher];
    let new_run = || {
        Canwu::new_with_plugins(47, Scenario::new(SimTime::EPOCH, Vec::new()), &plugins)
            .expect("the profile plugins should register")
    };

    // Off unless asked for, and then nothing is kept.
    let mut plain = new_run();
    assert!(!plain.boundary_profiling());
    let plain_receipts = run_two_days(&mut plain);
    let nothing = plain.take_boundary_profiles();
    assert!(nothing.profiles.is_empty() && nothing.dropped == 0);

    // Profiling leaves no trace in the run: the same receipts, evidence,
    // hashes, snapshot, and replay journal as the unprofiled twin.
    let mut timed = new_run();
    timed.set_boundary_profiling(true);
    let receipts = run_two_days(&mut timed);
    assert_eq!(receipts, plain_receipts);
    assert_eq!(timed.checkpoint_hash(), plain.checkpoint_hash());
    assert_eq!(
        timed.snapshot_json().expect("timed snapshot"),
        plain.snapshot_json().expect("plain snapshot")
    );
    let journal = serde_json::to_string(&timed.replay_journal()).expect("journal json");
    assert_eq!(
        journal,
        serde_json::to_string(&plain.replay_journal()).expect("journal json")
    );

    // One profile per settled boundary, in settlement order, listing every
    // plugin handler the boundary ran in the order it ran them: the command
    // handler and the event system it triggered while the boundary admitted
    // its input, then the boundary systems that were due.
    let batch = timed.take_boundary_profiles();
    assert_eq!(batch.dropped, 0);
    let profiles = batch.profiles;
    assert_eq!(profiles.len(), 2);
    for (profile, receipt) in profiles.iter().zip(&receipts) {
        assert_eq!(profile.boundary_id, receipt.boundary_id);
        assert_eq!(profile.settled_at, receipt.settled_at);
    }
    let post = system(BoundaryPhase::DomainDeltaProposal, LEDGER, "post");
    let watching = system(BoundaryPhase::DomainDeltaProposal, WATCH, "watch");
    let summarize = system(BoundaryPhase::StrategicAggregation, LEDGER, "summarize");
    let daily = vec![post, watching, summarize];
    assert_eq!(ran(&profiles[0]), daily);
    assert_eq!(
        ran(&profiles[1]),
        vec![
            (BoundaryHandlerKind::CommandHandler, None, WATCH, "note"),
            (BoundaryHandlerKind::EventSystem, None, WATCH, "echo"),
            post,
            watching,
            system(BoundaryPhase::StrategicAggregation, LEDGER, "close-month"),
            summarize,
        ]
    );
    let slow_micros = u64::try_from(SLOW.as_micros()).expect("the floor fits");
    for profile in &profiles {
        let handlers: u64 = profile
            .handlers
            .iter()
            .map(|timing| timing.elapsed_micros)
            .sum();
        assert!(profile.total_micros >= handlers);
        assert_eq!(profile.remaining_micros(), profile.total_micros - handlers);
        assert_eq!(
            profile.plugin_micros(LEDGER) + profile.plugin_micros(WATCH),
            handlers
        );
        // The slow system spins on the clock the profiler reads around it.
        assert!(profile.plugin_micros(LEDGER) >= slow_micros);
    }
    assert!(
        timed.take_boundary_profiles().profiles.is_empty(),
        "taken once"
    );

    // A boundary that fails leaves no profile.
    let error = timed
        .settle_boundary(BoundaryRequest::at(day(1)).with_cadence(SystemCadence::Annual))
        .expect_err("the Annual veto should fail the boundary");
    assert_eq!(error.code, ErrorCode::InvalidBoundary);
    assert!(timed.take_boundary_profiles().profiles.is_empty());

    // The setting is host context, not run state: a restored, replayed, or
    // forked run starts with it off.
    let restored =
        Canwu::from_snapshot_json_with_plugins(&timed.snapshot_json().expect("snapshot"), &plugins)
            .expect("the run should load");
    assert!(!restored.boundary_profiling());
    let replayed =
        Canwu::replay_from_journal_json(&plugins, &journal).expect("the run should replay");
    assert!(!replayed.boundary_profiling());
    timed
        .settle_boundary(BoundaryRequest::at(day(1)).with_cadence(SystemCadence::Daily))
        .expect("a daily boundary should settle");
    let mut fork = timed.fork();
    assert!(!fork.boundary_profiling());
    fork.settle_boundary(BoundaryRequest::at(day(1)).with_cadence(SystemCadence::Daily))
        .expect("the fork should settle");
    assert!(fork.take_boundary_profiles().profiles.is_empty());

    // The compact runtime keeps the setting and the profiles not yet taken,
    // and a seal drops neither. The direct boundary runs no handler; it
    // admits the emitted events so the evidence can be sealed.
    let mut compacted = timed.into_compacted().expect("compact runtime");
    assert!(compacted.boundary_profiling());
    compacted
        .settle_boundary(BoundaryRequest::at(day(1)))
        .expect("a direct boundary should settle");
    compacted
        .seal_evidence()
        .expect("the retained evidence should seal")
        .expect("the retained evidence is not empty");
    let kept = compacted.take_boundary_profiles().profiles;
    assert_eq!(kept.len(), 2);
    assert_eq!(ran(&kept[0]), daily);
    assert!(kept[1].handlers.is_empty());

    // The runtime keeps the latest profiles up to its cap and reports how
    // many older ones it dropped since the host last took them.
    let overflow = 3;
    for _ in 0..MAX_RETAINED_BOUNDARY_PROFILES + overflow {
        compacted
            .settle_boundary(BoundaryRequest::at(day(1)))
            .expect("a direct boundary should settle");
    }
    let capped = compacted.take_boundary_profiles();
    assert_eq!(capped.profiles.len(), MAX_RETAINED_BOUNDARY_PROFILES);
    assert_eq!(capped.dropped, 3);
    assert_eq!(
        capped.profiles[0].boundary_id.get(),
        kept[1].boundary_id.get() + 1 + 3,
        "the three oldest were dropped"
    );
    compacted.set_boundary_profiling(false);
    compacted
        .settle_boundary(BoundaryRequest::at(day(1)).with_cadence(SystemCadence::Daily))
        .expect("an unprofiled boundary should settle");
    let after = compacted.take_boundary_profiles();
    assert!(after.profiles.is_empty() && after.dropped == 0);
}
