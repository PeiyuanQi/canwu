//! A boundary profile splits the simulation core's own time into named
//! engine phases, which together with the plugin handlers partition the
//! settlement's wall time; and an exact replay can be profiled boundary by
//! boundary. Profiling changes no receipt, record, hash, snapshot, or replay
//! result.

#![allow(clippy::unnecessary_wraps)]

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryEnginePhase, BoundaryPhase, BoundaryProfile,
    BoundaryProposal, BoundaryRequest, BoundarySystemContract, Canwu, CanwuError, Command,
    CommandContext, CommandEnvelope, CommandRequest, CommandRequestId, Issuer,
    MAX_RETAINED_BOUNDARY_PROFILES, PayloadSchema, PluginActionDescriptor, PluginRegistrar,
    ReplayJournal, Scenario, SimDuration, SimTime, SimulationPlugin, SimulationView, SystemCadence,
    SystemDirective,
};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::{Duration, Instant};

const PLUGIN: &str = "fixture-engine-profile";
/// How long the slow system runs: handler time, never engine time.
const SLOW: Duration = Duration::from_millis(1);

fn slow(
    _view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let started = Instant::now();
    while started.elapsed() < SLOW {
        std::hint::spin_loop();
    }
    let mut proposal = BoundaryProposal::default();
    proposal.directives.push(BoundaryDirective::Emit {
        event_type: "posted".to_owned(),
        summary: format!("posted boundary {}", context.boundary_id.get()),
        affected: Vec::new(),
    });
    Ok(proposal)
}

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

struct ProfiledPlugin;

impl SimulationPlugin for ProfiledPlugin {
    fn name(&self) -> &'static str {
        PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000000e18"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut post = BoundarySystemContract::new(
            "post",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        post.emits = vec!["posted".to_owned()];
        registrar.register_boundary_system(post, slow)?;
        registrar.register_command(
            PluginActionDescriptor {
                name: "note".to_owned(),
                description: "emit one event".to_owned(),
                payload_schema: PayloadSchema::Any,
                reads: Vec::new(),
                writes: Vec::new(),
            },
            note,
        )
    }
}

fn day(day: i64) -> SimTime {
    SimTime::EPOCH + SimDuration::days(day)
}

/// `days` daily boundaries; every third admits a queued command.
fn run(canwu: &mut Canwu, days: i64) {
    for index in 0..days {
        if index % 3 == 1 {
            canwu
                .enqueue_command(
                    day(index),
                    0,
                    CommandRequest::new(
                        CommandRequestId::new(u64::try_from(index).expect("positive")),
                        canwu.revision(),
                        CommandEnvelope::new(
                            Issuer::System("fixture-host".to_owned()),
                            Command::Plugin {
                                plugin: PLUGIN.to_owned(),
                                command: "note".to_owned(),
                                payload: json!({}),
                            },
                        ),
                    ),
                )
                .expect("the note queues");
        }
        canwu
            .settle_boundary(BoundaryRequest::at(day(index)).with_cadence(SystemCadence::Daily))
            .expect("the boundary settles");
    }
}

/// The handlers and engine phases partition the settlement: their rounded
/// values add up to the total, short by less than one microsecond each.
fn assert_partitioned(profile: &BoundaryProfile) {
    assert_eq!(
        profile
            .engine
            .iter()
            .map(|timing| timing.phase)
            .collect::<Vec<_>>(),
        BoundaryEnginePhase::ALL,
        "every engine phase, in order"
    );
    let handlers: u64 = profile
        .handlers
        .iter()
        .map(|timing| timing.elapsed_micros)
        .sum();
    let engine: u64 = profile
        .engine
        .iter()
        .map(|timing| timing.elapsed_micros)
        .sum();
    let entries = u64::try_from(profile.handlers.len() + profile.engine.len()).expect("small");
    assert!(handlers + engine <= profile.total_micros);
    assert!(
        profile.total_micros - (handlers + engine) < entries,
        "{} µs of {} unaccounted",
        profile.total_micros - (handlers + engine),
        profile.total_micros
    );
    assert!(engine <= profile.remaining_micros());
    assert!(profile.remaining_micros() - engine < entries);
    for &phase in BoundaryEnginePhase::ALL {
        assert_eq!(
            profile.engine_micros(phase),
            profile
                .engine
                .iter()
                .find(|timing| timing.phase == phase)
                .expect("listed")
                .elapsed_micros
        );
    }
}

fn profiled_replay(
    journal: &ReplayJournal,
    plugins: &[&dyn SimulationPlugin],
) -> (Canwu, Vec<BoundaryProfile>) {
    let mut profiles = Vec::new();
    let replayed = Canwu::replay_from_journal_with_boundary_profiles(
        plugins,
        journal,
        Arc::new(()),
        &mut |profile| profiles.push(profile),
    )
    .expect("the profiled replay matches its journal");
    (replayed, profiles)
}

#[test]
fn engine_phases_partition_each_boundary_and_a_replay_is_profiled_boundary_by_boundary() {
    let plugin = ProfiledPlugin;
    let plugins: [&dyn SimulationPlugin; 1] = [&plugin];
    let new_run = || {
        Canwu::new_with_plugins(18, Scenario::new(SimTime::EPOCH, Vec::new()), &plugins)
            .expect("the plugin registers")
    };

    let mut live = new_run();
    live.set_boundary_profiling(true);
    run(&mut live, 6);
    let live_profiles = live.take_boundary_profiles().profiles;
    assert_eq!(live_profiles.len(), 6);
    let slow_micros = u64::try_from(SLOW.as_micros()).expect("fits");
    for profile in &live_profiles {
        assert_partitioned(profile);
        // The slow system's time is handler time: the engine phases hold
        // only what is left.
        assert!(profile.plugin_micros(PLUGIN) >= slow_micros);
        assert!(profile.total_micros - profile.remaining_micros() >= slow_micros);
    }
    let journal = live.replay_journal();

    // A profiled replay hands over one profile per journal boundary, in
    // order, as each settles; it runs the same handlers in the same order
    // as the live run did, and partitions its time the same way.
    let (replayed, profiles) = profiled_replay(&journal, &plugins);
    assert_eq!(profiles.len(), journal.boundaries.len());
    for ((profile, boundary), live) in profiles.iter().zip(&journal.boundaries).zip(&live_profiles)
    {
        assert_eq!(profile.boundary_id, boundary.id);
        assert_eq!(profile.settled_at, boundary.at);
        assert_partitioned(profile);
        let calls = |profile: &BoundaryProfile| {
            profile
                .handlers
                .iter()
                .map(|timing| (timing.kind, timing.phase, timing.name.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(calls(profile), calls(live));
    }

    // Profiling changed nothing: the profiled replay, an unprofiled replay,
    // and the live run hold the same snapshot and hashes, and the replayed
    // runtime has profiling off with nothing left to take.
    let plain = Canwu::replay_from_journal(&plugins, &journal).expect("the plain replay");
    let snapshot = replayed.snapshot_json().expect("snapshot");
    assert_eq!(snapshot, plain.snapshot_json().expect("snapshot"));
    assert_eq!(snapshot, live.snapshot_json().expect("snapshot"));
    assert_eq!(replayed.checkpoint_hash(), live.checkpoint_hash());
    let mut replayed = replayed;
    assert!(!replayed.boundary_profiling());
    let left = replayed.take_boundary_profiles();
    assert!(left.profiles.is_empty() && left.dropped == 0);

    // A replay longer than the runtime keeps profiles for loses none: they
    // are handed over as they settle, not queued.
    let mut long = new_run();
    let days = i64::try_from(MAX_RETAINED_BOUNDARY_PROFILES).expect("fits") + 4;
    run(&mut long, days);
    let (long_replayed, long_profiles) = profiled_replay(&long.replay_journal(), &plugins);
    assert_eq!(
        long_profiles.len(),
        MAX_RETAINED_BOUNDARY_PROFILES + 4,
        "no profile dropped"
    );
    assert!(
        long_profiles
            .windows(2)
            .all(|pair| pair[0].boundary_id.get() + 1 == pair[1].boundary_id.get())
    );
    assert_eq!(long_replayed.checkpoint_hash(), long.checkpoint_hash());

    // A replay that does not match its journal fails as an unprofiled one
    // does, after handing over the profiles of the boundaries that settled.
    let mut forged = journal.clone();
    let last = forged.boundaries.last_mut().expect("a boundary");
    last.hash = "0".repeat(64);
    let mut before_failure = Vec::new();
    let error = Canwu::replay_from_journal_with_boundary_profiles(
        &plugins,
        &forged,
        Arc::new(()),
        &mut |profile| before_failure.push(profile.boundary_id),
    )
    .err()
    .expect("a forged journal does not replay");
    let plain_error = Canwu::replay_from_journal(&plugins, &forged)
        .err()
        .expect("a forged journal does not replay");
    assert_eq!(error.code, plain_error.code);
    assert_eq!(
        before_failure,
        journal
            .boundaries
            .iter()
            .map(|boundary| boundary.id)
            .collect::<Vec<_>>(),
        "the last boundary settles before the mismatch is found"
    );
}

/// A run that admitted no command replays through the journal path without
/// command attempts; its boundaries are profiled and handed over too.
#[test]
fn a_command_free_replay_is_profiled_boundary_by_boundary() {
    let plugin = ProfiledPlugin;
    let plugins: [&dyn SimulationPlugin; 1] = [&plugin];
    let mut live = Canwu::new_with_plugins(19, Scenario::new(SimTime::EPOCH, Vec::new()), &plugins)
        .expect("the plugin registers");
    for index in 0..4 {
        live.settle_boundary(BoundaryRequest::at(day(index)).with_cadence(SystemCadence::Daily))
            .expect("the boundary settles");
    }
    let journal = live.replay_journal();
    assert!(journal.command_attempts.is_empty() && journal.commands.is_empty());
    let (replayed, profiles) = profiled_replay(&journal, &plugins);
    assert_eq!(
        profiles
            .iter()
            .map(|profile| profile.boundary_id)
            .collect::<Vec<_>>(),
        journal
            .boundaries
            .iter()
            .map(|boundary| boundary.id)
            .collect::<Vec<_>>()
    );
    for profile in &profiles {
        assert_partitioned(profile);
    }
    assert_eq!(replayed.checkpoint_hash(), live.checkpoint_hash());
}
