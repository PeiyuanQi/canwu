use canwu_api::{
    Army, ArmyId, BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal,
    BoundaryRequest, BoundarySystemContract, Canwu, CanwuError, Command, CommandEnvelope,
    CommandOutcome, CommandRequest, CommandRequestId, ENGINE_VERSION, EntityRef, ErrorCode,
    Government, GovernmentId, Issuer, KnowledgeSnapshot, MapPoint, Person, PersonId,
    RandomStreamKey, SNAPSHOT_FORMAT_VERSION, Scenario, SimulationPlugin, SimulationView, StateKey,
    StateVisibility, SystemCadence, Territory, TerritoryId, WorldSnapshot,
};
use serde_json::{Value, json};
#[cfg(feature = "allocation-counting")]
use std::alloc::{GlobalAlloc, Layout, System};
#[cfg(feature = "allocation-counting")]
use std::cell::Cell;
use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::time::Instant;

const DEFAULT_SCALES: &[usize] = &[8, 32, 128, 512];
const DEFAULT_SAMPLES: usize = 5;
const DEFAULT_GROWTH_SAMPLES: usize = 3;
const DEFAULT_WARMUP: usize = 1;
const PROFILE_SEED: u64 = 0xCA6E_2026;
const PROFILE_PLUGIN_HASH: &str =
    "8aa5d87954e0ca0a624c2e9ccde27893f149078026cc070b3ea3df46328a84d5";

#[cfg(feature = "allocation-counting")]
struct CountingAllocator;

#[cfg(feature = "allocation-counting")]
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[cfg(feature = "allocation-counting")]
thread_local! {
    static ALLOCATION_STATE: Cell<AllocationState> = const {
        Cell::new(AllocationState::disabled())
    };
}

// SAFETY: every operation delegates to `System` with the original pointer and
// layout. Thread-local counters only observe allocation traffic on the
// benchmark thread and never alter allocator inputs or ownership.
#[cfg(feature = "allocation-counting")]
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_allocation(|sample| {
            sample.alloc_calls = sample.alloc_calls.saturating_add(1);
            sample.allocated_bytes = sample
                .allocated_bytes
                .saturating_add(saturating_usize(layout.size()));
        });
        // SAFETY: the caller supplies the layout required by `GlobalAlloc`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record_allocation(|sample| {
            sample.alloc_calls = sample.alloc_calls.saturating_add(1);
            sample.allocated_bytes = sample
                .allocated_bytes
                .saturating_add(saturating_usize(layout.size()));
        });
        // SAFETY: the caller supplies the layout required by `GlobalAlloc`.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        record_allocation(|sample| {
            sample.dealloc_calls = sample.dealloc_calls.saturating_add(1);
            sample.deallocated_bytes = sample
                .deallocated_bytes
                .saturating_add(saturating_usize(layout.size()));
        });
        // SAFETY: the caller returns the pointer with its original layout.
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record_allocation(|sample| {
            sample.realloc_calls = sample.realloc_calls.saturating_add(1);
            sample.deallocated_bytes = sample
                .deallocated_bytes
                .saturating_add(saturating_usize(layout.size()));
            sample.allocated_bytes = sample
                .allocated_bytes
                .saturating_add(saturating_usize(new_size));
        });
        // SAFETY: the caller supplies the pointer, original layout, and desired
        // size required by `GlobalAlloc::realloc`.
        unsafe { System.realloc(pointer, layout, new_size) }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct AllocationSample {
    alloc_calls: u64,
    realloc_calls: u64,
    dealloc_calls: u64,
    allocated_bytes: u64,
    deallocated_bytes: u64,
}

#[cfg(feature = "allocation-counting")]
#[derive(Clone, Copy, Debug)]
struct AllocationState {
    tracking: bool,
    sample: AllocationSample,
}

#[cfg(feature = "allocation-counting")]
impl AllocationState {
    const fn disabled() -> Self {
        Self {
            tracking: false,
            sample: AllocationSample {
                alloc_calls: 0,
                realloc_calls: 0,
                dealloc_calls: 0,
                allocated_bytes: 0,
                deallocated_bytes: 0,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MetricMode {
    Elapsed,
    Allocations,
}

impl MetricMode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Elapsed => "elapsed",
            Self::Allocations => "allocations",
        }
    }
}

#[derive(Debug, Default)]
struct CaseMeasurements {
    elapsed_ns: Vec<u64>,
    allocations: Vec<AllocationSample>,
}

#[derive(Debug)]
struct Options {
    mode: MetricMode,
    scales: Vec<usize>,
    samples: usize,
    growth_samples: usize,
    warmup: usize,
    output: Option<PathBuf>,
    machine: String,
    recorded_on: String,
}

struct GrowthFixture {
    simulation: Canwu,
    scenario: Scenario,
}

struct ProfilePlugin;

impl SimulationPlugin for ProfilePlugin {
    fn name(&self) -> &'static str {
        "canwu-performance-profile"
    }

    fn version(&self) -> &'static str {
        "1.0.0"
    }

    fn semantic_hash(&self) -> &'static str {
        PROFILE_PLUGIN_HASH
    }

    fn register(&self, registrar: &mut canwu_api::PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut contract = BoundarySystemContract::new(
            "daily-growth",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        contract.writes = vec![profile_state()];
        contract.emits = vec!["profile_boundary".to_owned()];
        contract.random_streams = vec![profile_random_stream()];
        contract.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(contract, populate_boundary)
    }
}

fn profile_state() -> StateKey {
    StateKey::new("canwu-performance-profile", "boundary-components")
}

fn profile_random_stream() -> RandomStreamKey {
    RandomStreamKey::new("canwu-performance-profile", "daily-growth", 1)
}

fn populate_boundary(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let roll = view.random_range(&profile_random_stream(), 1_000_000, "profile boundary draw")?;
    let entity = EntityRef::Territory(TerritoryId::new(1));
    Ok(BoundaryProposal {
        directives: vec![
            BoundaryDirective::SetComponent {
                state: profile_state(),
                entity: entity.clone(),
                component: format!("boundary-{}", context.boundary_id.get()),
                value: Value::from(roll),
                summary: format!("Recorded profile boundary {}", context.boundary_id),
            },
            BoundaryDirective::Emit {
                event_type: "profile_boundary".to_owned(),
                summary: format!("Profile boundary {} completed", context.boundary_id),
                affected: vec![entity],
            },
        ],
        ..BoundaryProposal::default()
    })
}

fn main() -> Result<(), Box<dyn Error>> {
    let options = parse_options()?;
    if cfg!(debug_assertions) {
        return Err("run the performance harness with --release".into());
    }
    match options.mode {
        MetricMode::Elapsed if cfg!(feature = "allocation-counting") => {
            return Err(
                "elapsed mode must be built without the allocation-counting feature".into(),
            );
        }
        MetricMode::Allocations if !cfg!(feature = "allocation-counting") => {
            return Err("allocation mode requires --features allocation-counting".into());
        }
        MetricMode::Elapsed | MetricMode::Allocations => {}
    }

    let mut scale_reports = Vec::new();
    for &scale in &options.scales {
        eprintln!("profiling scale {scale}");
        let growth = measure_case(
            options.mode,
            options.warmup,
            options.growth_samples,
            || Ok(()),
            |_| build_growth_fixture(scale),
        )?;
        let fixture = build_growth_fixture(scale)?;
        validate_growth_counts(&fixture.simulation, scale)?;
        let snapshot = fixture.simulation.snapshot();
        let snapshot_json = fixture.simulation.snapshot_json()?;
        let journal = fixture.simulation.replay_journal();
        let checkpoint_hash = fixture.simulation.checkpoint_hash().to_owned();
        let scenario = fixture.scenario.clone();
        let next_request_id = checked_request_id(scale)?;
        let accepted = measure_case(
            options.mode,
            options.warmup,
            options.samples,
            || restore(&snapshot_json),
            |simulation| {
                let request = accepted_request(simulation, next_request_id);
                let outcome = simulation.process_command(request)?;
                if !matches!(outcome, CommandOutcome::Accepted { .. }) {
                    return Err(benchmark_error("accepted command case was rejected"));
                }
                Ok(outcome)
            },
        )?;
        let rejected = measure_case(
            options.mode,
            options.warmup,
            options.samples,
            || restore(&snapshot_json),
            |simulation| {
                let request = rejected_request(simulation, next_request_id);
                let outcome = simulation.process_command(request)?;
                if !matches!(outcome, CommandOutcome::Rejected { .. }) {
                    return Err(benchmark_error("rejected command case was accepted"));
                }
                Ok(outcome)
            },
        )?;
        let empty_boundary = measure_case(
            options.mode,
            options.warmup,
            options.samples,
            || restore(&snapshot_json),
            |simulation| {
                let receipt = simulation.settle_boundary(BoundaryRequest::at(simulation.time()))?;
                if receipt.change_count != 0
                    || receipt.record_change_count != 0
                    || !receipt.emitted_events.is_empty()
                    || !receipt.generated_ingress.is_empty()
                    || !receipt.random_draws.is_empty()
                    || !receipt.allocations.is_empty()
                {
                    return Err(benchmark_error("empty boundary case produced work"));
                }
                Ok(receipt)
            },
        )?;
        let populated_boundary = measure_case(
            options.mode,
            options.warmup,
            options.samples,
            || restore(&snapshot_json),
            |simulation| {
                let receipt = simulation.settle_boundary(
                    BoundaryRequest::at(simulation.time()).with_cadence(SystemCadence::Daily),
                )?;
                if receipt.change_count != 1
                    || receipt.record_change_count != 0
                    || receipt.emitted_events.len() != 2
                    || !receipt.generated_ingress.is_empty()
                    || receipt.random_draws.len() != 1
                    || !receipt.allocations.is_empty()
                {
                    return Err(benchmark_error(
                        "populated boundary case produced unexpected evidence",
                    ));
                }
                Ok(receipt)
            },
        )?;
        let snapshot_create = measure_case(
            options.mode,
            options.warmup,
            options.samples,
            || restore(&snapshot_json),
            |simulation| Ok::<_, CanwuError>(simulation.snapshot()),
        )?;
        let snapshot_serialize = measure_case(
            options.mode,
            options.warmup,
            options.samples,
            || Ok::<_, serde_json::Error>(snapshot.clone()),
            |snapshot| serde_json::to_string_pretty(snapshot),
        )?;
        let snapshot_load_validate = measure_case(
            options.mode,
            options.warmup,
            options.samples,
            || Ok::<_, CanwuError>(snapshot_json.clone()),
            |json| Canwu::from_snapshot_json_with_plugins(json, &[&ProfilePlugin]),
        )?;
        let exact_replay = measure_case(
            options.mode,
            options.warmup,
            options.samples,
            || Ok::<_, CanwuError>((scenario.clone(), journal.clone())),
            |(scenario, journal)| {
                Canwu::replay_from_journal(scenario.clone(), &[&ProfilePlugin], journal)
            },
        )?;

        scale_reports.push(json!({
            "scale": scale,
            "history": history_json(&snapshot, snapshot_json.len(), &checkpoint_hash),
            "cases": [
                case_json("history_growth", &growth, options.mode),
                case_json("accepted_command", &accepted, options.mode),
                case_json("rejected_command", &rejected, options.mode),
                case_json("empty_boundary", &empty_boundary, options.mode),
                case_json("populated_boundary", &populated_boundary, options.mode),
                case_json("snapshot_create", &snapshot_create, options.mode),
                case_json("snapshot_serialize_pretty", &snapshot_serialize, options.mode),
                case_json("snapshot_load_validate", &snapshot_load_validate, options.mode),
                case_json("exact_replay", &exact_replay, options.mode),
            ],
        }));
    }

    let report = json!({
        "schema_version": 2,
        "benchmark": "canwu-architecture-baseline",
        "metric_mode": options.mode.as_str(),
        "recorded_on": options.recorded_on,
        "machine": options.machine,
        "engine_version": ENGINE_VERSION,
        "snapshot_format_version": SNAPSHOT_FORMAT_VERSION,
        "source": source_metadata()?,
        "environment": {
            "os": env::consts::OS,
            "arch": env::consts::ARCH,
            "logical_cpus": std::thread::available_parallelism().map_or(1, usize::from),
            "rustc": command_output("rustc", &["--version", "--verbose"]),
            "cargo": command_output("cargo", &["--version"]),
            "profile": "release",
            "cargo_features": if cfg!(feature = "allocation-counting") { vec!["allocation-counting"] } else { Vec::<&str>::new() },
            "rustflags": env::var("RUSTFLAGS").unwrap_or_default(),
            "encoded_rustflags": env::var("CARGO_ENCODED_RUSTFLAGS").unwrap_or_default(),
            "build_target": env::var("CARGO_BUILD_TARGET").unwrap_or_default(),
        },
        "method": {
            "warmup_iterations": options.warmup,
            "operation_samples": options.samples,
            "growth_samples": options.growth_samples,
            "allocation_scope": if options.mode == MetricMode::Allocations { "thread-local allocator calls and requested bytes during the operation; setup and post-measurement drops are excluded" } else { "not collected in this uninstrumented build" },
            "elapsed_scope": if options.mode == MetricMode::Elapsed { "wall-clock nanoseconds in a build with the default system allocator; setup and post-measurement drops are excluded" } else { "not collected in the allocation-instrumented build" },
        },
        "scales": scale_reports,
    });
    let encoded = serde_json::to_string_pretty(&report)?;
    print_summary(&report, options.mode);
    if let Some(output) = options.output {
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&output, format!("{encoded}\n"))?;
        eprintln!("wrote {}", output.display());
    } else {
        println!("{encoded}");
    }
    Ok(())
}

fn restore(snapshot_json: &str) -> Result<Canwu, CanwuError> {
    Canwu::from_snapshot_json_with_plugins(snapshot_json, &[&ProfilePlugin])
}

fn benchmark_error(message: &str) -> CanwuError {
    CanwuError::new(ErrorCode::ReplayMismatch, message)
}

fn build_growth_fixture(scale: usize) -> Result<GrowthFixture, Box<dyn Error>> {
    let scenario = profile_scenario(scale)?;
    let mut simulation = Canwu::new(PROFILE_SEED, scenario.clone())?;
    simulation.register_plugin(&ProfilePlugin)?;
    for index in 0..scale {
        let accepted_id = checked_u64(
            index
                .checked_mul(2)
                .and_then(|value| value.checked_add(1))
                .ok_or("accepted request ID overflow")?,
            "request ID",
        )?;
        let rejected_id = checked_u64(
            index
                .checked_mul(2)
                .and_then(|value| value.checked_add(2))
                .ok_or("rejected request ID overflow")?,
            "request ID",
        )?;
        let army = ArmyId::new(checked_u64(
            index.checked_add(1).ok_or("army ID overflow")?,
            "army ID",
        )?);
        let accepted = simulation.process_command(CommandRequest::new(
            CommandRequestId::new(accepted_id),
            simulation.revision(),
            CommandEnvelope::new(
                Issuer::Debug,
                Command::DebugSetArmyMorale {
                    army,
                    morale: u16::try_from(index % 101)?,
                },
            ),
        ))?;
        if !matches!(accepted, CommandOutcome::Accepted { .. }) {
            return Err(format!("accepted command {index} was rejected").into());
        }
        let rejected = simulation.process_command(CommandRequest::new(
            CommandRequestId::new(rejected_id),
            simulation.revision(),
            CommandEnvelope::new(
                Issuer::Debug,
                Command::DebugSetArmyMorale { army, morale: 101 },
            ),
        ))?;
        if !matches!(rejected, CommandOutcome::Rejected { .. }) {
            return Err(format!("rejected command {index} was accepted").into());
        }
        simulation.settle_boundary(
            BoundaryRequest::at(simulation.time()).with_cadence(SystemCadence::Daily),
        )?;
    }
    Ok(GrowthFixture {
        simulation,
        scenario,
    })
}

fn profile_scenario(scale: usize) -> Result<Scenario, Box<dyn Error>> {
    if scale == 0 {
        return Err("profile scale must be greater than zero".into());
    }
    let government_id = GovernmentId::new(1);
    let mut people = Vec::with_capacity(scale);
    let mut territories = Vec::with_capacity(scale);
    let mut armies = Vec::with_capacity(scale);
    for index in 1..=scale {
        let id = checked_u64(index, "entity ID")?;
        let territory = TerritoryId::new(id);
        let person = PersonId::new(id);
        people.push(Person {
            id: person,
            name: format!("Profile Person {id}"),
            government: government_id,
            current_location: territory,
            roles: vec!["profile".to_owned()],
        });
        territories.push(Territory {
            id: territory,
            name: format!("Profile Territory {id}"),
            controller: government_id,
            position: MapPoint {
                x: index as f32,
                y: (index % 17) as f32,
            },
        });
        armies.push(Army {
            id: ArmyId::new(id),
            name: format!("Profile Army {id}"),
            government: government_id,
            commander: person,
            location: territory,
            strength: 1_000,
            morale: 50,
            transit: None,
        });
    }
    Ok(Scenario {
        start_time: canwu_api::SimTime::EPOCH,
        world: WorldSnapshot {
            people,
            governments: vec![Government {
                id: government_id,
                name: "Profile Government".to_owned(),
                capital: TerritoryId::new(1),
            }],
            territories,
            routes: Vec::new(),
            armies,
        },
        knowledge: KnowledgeSnapshot::default(),
        domain_records: Vec::new(),
    })
}

fn validate_growth_counts(simulation: &Canwu, scale: usize) -> Result<(), Box<dyn Error>> {
    let snapshot = simulation.snapshot();
    let expected_events = scale.checked_mul(3).ok_or("event count overflow")?;
    let expected_attempts = scale.checked_mul(2).ok_or("attempt count overflow")?;
    if simulation.commands().len() != scale
        || simulation.command_attempts().len() != expected_attempts
        || simulation.boundaries().len() != scale
        || simulation.random_draws().len() != scale
        || simulation.events().len() != expected_events
        || snapshot.plugin_components.len() != scale
    {
        return Err(format!(
            "growth fixture counts diverged at scale {scale}: commands={}, attempts={}, events={}, boundaries={}, random_draws={}, components={}",
            simulation.commands().len(),
            simulation.command_attempts().len(),
            simulation.events().len(),
            simulation.boundaries().len(),
            simulation.random_draws().len(),
            snapshot.plugin_components.len(),
        )
        .into());
    }
    Ok(())
}

fn accepted_request(simulation: &Canwu, request_id: u64) -> CommandRequest {
    CommandRequest::new(
        CommandRequestId::new(request_id),
        simulation.revision(),
        CommandEnvelope::new(
            Issuer::Debug,
            Command::DebugSetArmyMorale {
                army: ArmyId::new(1),
                morale: 73,
            },
        ),
    )
}

fn rejected_request(simulation: &Canwu, request_id: u64) -> CommandRequest {
    CommandRequest::new(
        CommandRequestId::new(request_id),
        simulation.revision(),
        CommandEnvelope::new(
            Issuer::Debug,
            Command::DebugSetArmyMorale {
                army: ArmyId::new(1),
                morale: 101,
            },
        ),
    )
}

fn measure_case<I, O, E, Setup, Operation>(
    mode: MetricMode,
    warmup: usize,
    samples: usize,
    mut setup: Setup,
    mut operation: Operation,
) -> Result<CaseMeasurements, E>
where
    Setup: FnMut() -> Result<I, E>,
    Operation: FnMut(&mut I) -> Result<O, E>,
{
    for _ in 0..warmup {
        let mut input = setup()?;
        black_box(operation(&mut input)?);
    }
    let mut results = CaseMeasurements::default();
    match mode {
        MetricMode::Elapsed => {
            results.elapsed_ns.reserve(samples);
            for _ in 0..samples {
                let mut input = setup()?;
                let started = Instant::now();
                let output = operation(&mut input)?;
                black_box(&output);
                results
                    .elapsed_ns
                    .push(saturating_u128(started.elapsed().as_nanos()));
                drop(output);
                drop(input);
            }
        }
        MetricMode::Allocations => {
            results.allocations.reserve(samples);
            for _ in 0..samples {
                let mut input = setup()?;
                start_allocation_sample();
                let operation_result = operation(&mut input);
                let allocations = finish_allocation_sample();
                let output = operation_result?;
                black_box(&output);
                results.allocations.push(allocations);
                drop(output);
                drop(input);
            }
        }
    }
    Ok(results)
}

#[cfg(feature = "allocation-counting")]
fn record_allocation(update: impl FnOnce(&mut AllocationSample)) {
    let _ = ALLOCATION_STATE.try_with(|state| {
        let mut current = state.get();
        if current.tracking {
            update(&mut current.sample);
            state.set(current);
        }
    });
}

#[cfg(feature = "allocation-counting")]
fn start_allocation_sample() {
    ALLOCATION_STATE.with(|state| {
        state.set(AllocationState {
            tracking: true,
            sample: AllocationSample::default(),
        });
    });
}

#[cfg(feature = "allocation-counting")]
fn finish_allocation_sample() -> AllocationSample {
    ALLOCATION_STATE.with(|state| {
        let current = state.get();
        state.set(AllocationState::disabled());
        current.sample
    })
}

#[cfg(not(feature = "allocation-counting"))]
fn start_allocation_sample() {
    unreachable!("allocation mode is rejected before measurement")
}

#[cfg(not(feature = "allocation-counting"))]
fn finish_allocation_sample() -> AllocationSample {
    unreachable!("allocation mode is rejected before measurement")
}

fn case_json(name: &str, measurements: &CaseMeasurements, mode: MetricMode) -> Value {
    match mode {
        MetricMode::Elapsed => json!({
            "name": name,
            "summary": {
                "elapsed_ns_median": median(&measurements.elapsed_ns),
                "elapsed_ns_min": measurements.elapsed_ns.iter().copied().min().unwrap_or(0),
                "elapsed_ns_max": measurements.elapsed_ns.iter().copied().max().unwrap_or(0),
            },
            "elapsed_samples_ns": measurements.elapsed_ns,
        }),
        MetricMode::Allocations => {
            let alloc_calls: Vec<_> = measurements
                .allocations
                .iter()
                .map(|sample| sample.alloc_calls)
                .collect();
            let realloc_calls: Vec<_> = measurements
                .allocations
                .iter()
                .map(|sample| sample.realloc_calls)
                .collect();
            let allocation_operations: Vec<_> = measurements
                .allocations
                .iter()
                .map(|sample| sample.alloc_calls.saturating_add(sample.realloc_calls))
                .collect();
            let allocated_bytes: Vec<_> = measurements
                .allocations
                .iter()
                .map(|sample| sample.allocated_bytes)
                .collect();
            json!({
                "name": name,
                "summary": {
                    "alloc_calls_median": median(&alloc_calls),
                    "realloc_calls_median": median(&realloc_calls),
                    "allocation_operations_median": median(&allocation_operations),
                    "allocated_bytes_median": median(&allocated_bytes),
                },
                "allocation_samples": measurements.allocations.iter().map(|sample| json!({
                    "alloc_calls": sample.alloc_calls,
                    "realloc_calls": sample.realloc_calls,
                    "allocation_operations": sample.alloc_calls.saturating_add(sample.realloc_calls),
                    "dealloc_calls": sample.dealloc_calls,
                    "allocated_bytes": sample.allocated_bytes,
                    "deallocated_bytes": sample.deallocated_bytes,
                })).collect::<Vec<_>>(),
            })
        }
    }
}

fn history_json(
    snapshot: &canwu_api::SimulationSnapshot,
    snapshot_size_bytes: usize,
    checkpoint_hash: &str,
) -> Value {
    let world_entities = snapshot.world.people.len()
        + snapshot.world.governments.len()
        + snapshot.world.territories.len()
        + snapshot.world.routes.len()
        + snapshot.world.armies.len();
    json!({
        "world_entities": world_entities,
        "domain_records": snapshot.domain_records.len(),
        "plugin_components": snapshot.plugin_components.len(),
        "commands": snapshot.commands.len(),
        "command_attempts": snapshot.command_attempts.len(),
        "ingress": snapshot.ingress.len(),
        "events": snapshot.events.len(),
        "boundaries": snapshot.boundaries.len(),
        "random_draws": snapshot.random_draws.len(),
        "snapshot_size_bytes": snapshot_size_bytes,
        "checkpoint_hash": checkpoint_hash,
    })
}

fn median(values: &[u64]) -> u64 {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    sorted.get(sorted.len() / 2).copied().unwrap_or(0)
}

fn print_summary(report: &Value, mode: MetricMode) {
    match mode {
        MetricMode::Elapsed => eprintln!(
            "scale entities components commands attempts events boundaries draws snapshot_bytes case median_ms"
        ),
        MetricMode::Allocations => eprintln!(
            "scale entities components commands attempts events boundaries draws snapshot_bytes case allocation_operations allocated_bytes"
        ),
    }
    let Some(scales) = report.get("scales").and_then(Value::as_array) else {
        return;
    };
    for scale in scales {
        let history = &scale["history"];
        let Some(cases) = scale.get("cases").and_then(Value::as_array) else {
            continue;
        };
        for case in cases {
            let summary = &case["summary"];
            let prefix = format!(
                "{} {} {} {} {} {} {} {} {} {}",
                scale["scale"].as_u64().unwrap_or(0),
                history["world_entities"].as_u64().unwrap_or(0),
                history["plugin_components"].as_u64().unwrap_or(0),
                history["commands"].as_u64().unwrap_or(0),
                history["command_attempts"].as_u64().unwrap_or(0),
                history["events"].as_u64().unwrap_or(0),
                history["boundaries"].as_u64().unwrap_or(0),
                history["random_draws"].as_u64().unwrap_or(0),
                history["snapshot_size_bytes"].as_u64().unwrap_or(0),
                case["name"].as_str().unwrap_or("unknown"),
            );
            match mode {
                MetricMode::Elapsed => {
                    let elapsed_ns = summary["elapsed_ns_median"].as_u64().unwrap_or(0);
                    eprintln!("{prefix} {:.3}", elapsed_ns as f64 / 1_000_000.0);
                }
                MetricMode::Allocations => eprintln!(
                    "{prefix} {} {}",
                    summary["allocation_operations_median"]
                        .as_u64()
                        .unwrap_or(0),
                    summary["allocated_bytes_median"].as_u64().unwrap_or(0),
                ),
            }
        }
    }
}

fn source_metadata() -> Result<Value, Box<dyn Error>> {
    let root = repository_root();
    let commit = command_output_in(&root, "git", &["rev-parse", "HEAD"]);
    let status = command_output_in(&root, "git", &["status", "--porcelain"]);
    let engine_status = command_output_in(
        &root,
        "git",
        &[
            "status",
            "--porcelain",
            "--",
            "crates/canwu-api/src",
            "crates/canwu-core/src",
            "crates/canwu-event/src",
            "crates/canwu-knowledge/src",
            "crates/canwu-sim/src",
            "crates/canwu-time/src",
            "crates/canwu-world/src",
        ],
    );
    let mut source_file_hashes = BTreeMap::new();
    for path in source_input_paths(&root)? {
        let relative = path
            .strip_prefix(&root)?
            .to_string_lossy()
            .replace('\\', "/");
        source_file_hashes.insert(relative, git_blob_hash(&root, &path)?);
    }
    Ok(json!({
        "commit": commit,
        "working_tree_dirty": !status.trim().is_empty(),
        "engine_source_dirty": !engine_status.trim().is_empty(),
        "source_file_hashes": source_file_hashes,
    }))
}

fn source_input_paths(root: &Path) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let mut paths = vec![
        root.join("Cargo.toml"),
        root.join("Cargo.lock"),
        root.join("benchmarks/performance-harness/Cargo.toml"),
        root.join("benchmarks/performance-harness/Cargo.lock"),
        root.join("benchmarks/performance-harness/src/main.rs"),
    ];
    collect_source_files(&root.join("crates"), &mut paths)?;
    let cargo_config = root.join(".cargo");
    if cargo_config.is_dir() {
        collect_all_files(&cargo_config, &mut paths)?;
    }
    for name in ["rust-toolchain", "rust-toolchain.toml"] {
        let candidate = root.join(name);
        if candidate.is_file() {
            paths.push(candidate);
        }
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn collect_source_files(directory: &Path, paths: &mut Vec<PathBuf>) -> Result<(), Box<dyn Error>> {
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_source_files(&path, paths)?;
        } else if path.file_name().is_some_and(|name| name == "Cargo.toml")
            || path.extension().is_some_and(|extension| extension == "rs")
        {
            paths.push(path);
        }
    }
    Ok(())
}

fn collect_all_files(directory: &Path, paths: &mut Vec<PathBuf>) -> Result<(), Box<dyn Error>> {
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_all_files(&path, paths)?;
        } else {
            paths.push(path);
        }
    }
    Ok(())
}

fn git_blob_hash(root: &Path, path: &Path) -> Result<String, Box<dyn Error>> {
    let output = ProcessCommand::new("git")
        .current_dir(root)
        .args(["hash-object", "--no-filters", "--"])
        .arg(path)
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "git hash-object failed for {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("performance harness must remain under benchmarks/performance-harness")
        .to_path_buf()
}

fn command_output(program: &str, arguments: &[&str]) -> String {
    ProcessCommand::new(program)
        .args(arguments)
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_else(|error| format!("unavailable: {error}"))
}

fn command_output_in(directory: &Path, program: &str, arguments: &[&str]) -> String {
    ProcessCommand::new(program)
        .current_dir(directory)
        .args(arguments)
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_else(|error| format!("unavailable: {error}"))
}

fn parse_options() -> Result<Options, Box<dyn Error>> {
    let mut options = Options {
        mode: MetricMode::Elapsed,
        scales: DEFAULT_SCALES.to_vec(),
        samples: DEFAULT_SAMPLES,
        growth_samples: DEFAULT_GROWTH_SAMPLES,
        warmup: DEFAULT_WARMUP,
        output: None,
        machine: "local-machine".to_owned(),
        recorded_on: "unspecified".to_owned(),
    };
    let mut arguments = env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--mode" => {
                options.mode = match arguments
                    .next()
                    .ok_or("--mode requires elapsed or allocations")?
                    .as_str()
                {
                    "elapsed" => MetricMode::Elapsed,
                    "allocations" => MetricMode::Allocations,
                    value => return Err(format!("unsupported metric mode: {value}").into()),
                };
            }
            "--scales" => {
                let value = arguments.next().ok_or("--scales requires a value")?;
                options.scales = value
                    .split(',')
                    .map(str::parse)
                    .collect::<Result<Vec<usize>, _>>()?;
                if options.scales.is_empty() || options.scales.contains(&0) {
                    return Err("--scales must contain positive comma-separated integers".into());
                }
            }
            "--samples" => {
                options.samples = parse_positive_odd(arguments.next(), "--samples")?;
            }
            "--growth-samples" => {
                options.growth_samples = parse_positive_odd(arguments.next(), "--growth-samples")?;
            }
            "--warmup" => {
                options.warmup = arguments
                    .next()
                    .ok_or("--warmup requires a value")?
                    .parse()?;
            }
            "--output" => {
                options.output = Some(PathBuf::from(
                    arguments.next().ok_or("--output requires a path")?,
                ));
            }
            "--machine" => {
                options.machine = arguments.next().ok_or("--machine requires a value")?;
            }
            "--recorded-on" => {
                options.recorded_on = arguments.next().ok_or("--recorded-on requires a value")?;
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            _ => return Err(format!("unknown argument: {argument}").into()),
        }
    }
    options.scales.sort_unstable();
    options.scales.dedup();
    Ok(options)
}

fn parse_positive_odd(value: Option<String>, flag: &str) -> Result<usize, Box<dyn Error>> {
    let parsed = value
        .ok_or_else(|| format!("{flag} requires a value"))?
        .parse()?;
    if parsed == 0 || parsed % 2 == 0 {
        return Err(format!("{flag} must be a positive odd integer").into());
    }
    Ok(parsed)
}

fn print_help() {
    println!(
        "Canwu deterministic performance harness\n\n\
         Usage: cargo run --release --manifest-path benchmarks/performance-harness/Cargo.toml -- [options]\n\n\
         Options:\n\
           --mode MODE              elapsed or allocations\n\
           --scales 8,32,128,512  History scales to profile\n\
           --samples N             Positive odd sample count for each operation\n\
           --growth-samples N      Positive odd sample count for history construction\n\
           --warmup N              Warmup iterations per case\n\
           --output PATH           Write the JSON report to PATH\n\
           --machine LABEL         Stable description of the measurement machine\n\
           --recorded-on DATE      Recording date or build label\n"
    );
}

fn checked_request_id(scale: usize) -> Result<u64, Box<dyn Error>> {
    checked_u64(
        scale
            .checked_mul(2)
            .and_then(|value| value.checked_add(1))
            .ok_or("request ID overflow")?,
        "request ID",
    )
}

fn checked_u64(value: usize, label: &str) -> Result<u64, Box<dyn Error>> {
    u64::try_from(value).map_err(|_| format!("{label} exceeds u64").into())
}

#[cfg(feature = "allocation-counting")]
fn saturating_usize(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn saturating_u128(value: u128) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn growth_fixture_is_deterministic_and_exactly_replayable() {
        let first = build_growth_fixture(4).expect("first fixture should build");
        let second = build_growth_fixture(4).expect("second fixture should build");
        validate_growth_counts(&first.simulation, 4).expect("first fixture counts should match");
        validate_growth_counts(&second.simulation, 4).expect("second fixture counts should match");
        assert_eq!(first.simulation.snapshot(), second.simulation.snapshot());

        let journal = first.simulation.replay_journal();
        let replayed = Canwu::replay_from_journal(first.scenario, &[&ProfilePlugin], &journal)
            .expect("fixture should replay exactly");
        assert_eq!(replayed.snapshot(), second.simulation.snapshot());
    }

    #[cfg(feature = "allocation-counting")]
    #[test]
    fn operation_measurement_excludes_setup_allocations() {
        let samples = measure_case(
            MetricMode::Allocations,
            0,
            1,
            || Ok(vec![0_u8; 1024]),
            |_| Ok::<_, std::convert::Infallible>(Box::new(7_u64)),
        )
        .expect("infallible measurement should succeed");
        assert_eq!(samples.allocations.len(), 1);
        assert_eq!(samples.allocations[0].alloc_calls, 1);
        assert!(samples.allocations[0].allocated_bytes >= size_of::<u64>() as u64);
        assert!(samples.allocations[0].allocated_bytes < 1024);
    }

    #[test]
    fn elapsed_measurement_records_only_elapsed_samples() {
        let samples = measure_case(
            MetricMode::Elapsed,
            0,
            1,
            || Ok::<_, std::convert::Infallible>(7_u64),
            |value| Ok::<_, std::convert::Infallible>(value.saturating_mul(2)),
        )
        .expect("infallible measurement should succeed");
        assert_eq!(samples.elapsed_ns.len(), 1);
        assert!(samples.allocations.is_empty());
    }
}
