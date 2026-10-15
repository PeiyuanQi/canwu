//! Boundary cost against retained operation-keyed random draws.
//!
//! The workload keeps `--retained` operation-keyed draws in a run and then
//! measures, for each `--systems` count and each retained count:
//!
//! - `live_boundary`: one Daily boundary in which that many boundary systems
//!   are due and each makes `--draws` fresh operation-keyed draws, on a run
//!   that still retains every draw record;
//! - `sealed_boundary`: the same boundary after the evidence was sealed, when
//!   every earlier draw is a keyed reservation (skipped when nothing is
//!   retained, because there is nothing to seal);
//! - `snapshot_restore`: loading the run from its snapshot JSON, which
//!   validates every draw and builds whatever the engine derives from them;
//! - `checkpoint_journal_restore`: loading the run from its portable
//!   checkpoint journal, which rebuilds the snapshot from the evidence prefix;
//! - on the run after one seal, when every keyed draw is sealed:
//!   `compact_checkpoint` (`checkpoint()`), `compact_paged_checkpoint`
//!   (`prepare_paged_checkpoint` against an empty page store), `compact_seal`
//!   (sealing the evidence of one more boundary), and, where the engine has
//!   it, `compact_fork` (`CompactedCanwu::fork`, behind the default
//!   `compact-fork` feature, which an engine without that call builds
//!   without).
//!
//! Elapsed time and allocation traffic are collected in separate release
//! builds, as in the main harness: `--mode elapsed` without the
//! `allocation-counting` feature, `--mode allocations` with it. Allocation
//! counts do not depend on machine load. The allocation build also reports
//! `run_heap_bytes`, the bytes the built run holds, so two engine versions
//! can be compared for what they keep per retained draw.
//!
//! The binary uses only the public API, so the same source measures any
//! engine version it compiles against.

use canwu_api::{
    BoundaryContext, BoundaryPhase, BoundaryProposal, BoundarySystemContract, Canwu, CanwuError,
    CompactedCanwu, ENGINE_VERSION, ErrorCode, EvidenceRef, PluginRegistrar, RandomOperationTarget,
    RandomStreamKey, Scenario, SimDuration, SimTime, SimulationPlugin, SimulationView,
    StatePageBlob, StatePageProvider, SystemCadence,
};
use serde_json::{Value, json};
#[cfg(feature = "allocation-counting")]
use std::alloc::{GlobalAlloc, Layout, System};
#[cfg(feature = "allocation-counting")]
use std::cell::Cell;
use std::env;
use std::error::Error;
use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

const PLUGIN: &str = "profile-keyed-draws";
const PLUGIN_HASH: &str = "00000000000000000000000000000000000000000000000000000000000ca650";
/// Keyed draws one filling boundary retains.
const FILL_BATCH: usize = 1_000;
/// Fresh keyed draws each due probe system makes; set once from `--draws`.
/// Boundary system handlers are plain function pointers, so the handler
/// reads its workload from here.
static FRESH_DRAWS: AtomicUsize = AtomicUsize::new(1);

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

impl AllocationSample {
    /// Bytes allocated and not freed during the sample.
    fn net_bytes(self) -> i128 {
        i128::from(self.allocated_bytes) - i128::from(self.deallocated_bytes)
    }
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

#[cfg(feature = "allocation-counting")]
fn saturating_usize(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
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
struct Measurements {
    elapsed_ns: Vec<u64>,
    allocations: Vec<AllocationSample>,
}

/// Runs `operation` once and records its wall time or its allocation traffic.
fn measure<O>(
    mode: MetricMode,
    measurements: &mut Measurements,
    operation: impl FnOnce() -> O,
) -> O {
    match mode {
        MetricMode::Elapsed => {
            let started = Instant::now();
            let output = operation();
            let elapsed = started.elapsed();
            measurements
                .elapsed_ns
                .push(u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX));
            output
        }
        MetricMode::Allocations => {
            start_allocation_sample();
            let output = operation();
            measurements.allocations.push(finish_allocation_sample());
            output
        }
    }
}

/// A random stream has one owning system, so each system draws from its own.
fn stream(system: &str) -> RandomStreamKey {
    RandomStreamKey::new(PLUGIN, system, 1)
}

/// The calendar ingress that opened the boundary, as retained evidence.
fn calendar_evidence(context: &BoundaryContext) -> Result<EvidenceRef, CanwuError> {
    context
        .admitted_ingress
        .first()
        .copied()
        .map(EvidenceRef::Ingress)
        .ok_or_else(|| CanwuError::new(ErrorCode::EvidenceUnavailable, "no calendar ingress"))
}

fn draw(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    kind: &str,
    index: usize,
) -> Result<(), CanwuError> {
    view.random_range_for_operation(
        &stream(&context.system),
        calendar_evidence(context)?,
        kind,
        &format!("{}-{}-{index}", context.system, context.boundary_id.get()),
        RandomOperationTarget::CanonicalKey(format!("target-{index}")),
        0,
        10_000,
        "keyed draw retention profile",
    )
    .map(|_| ())
}

/// Monthly: retains one batch of keyed draws.
fn fill(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    for index in 0..FILL_BATCH {
        draw(view, context, "fill", index)?;
    }
    Ok(BoundaryProposal::default())
}

/// Daily: a due system that makes `--draws` fresh keyed draws.
fn probe(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    for index in 0..FRESH_DRAWS.load(Ordering::Relaxed) {
        draw(view, context, "probe", index)?;
    }
    Ok(BoundaryProposal::default())
}

struct KeyedDrawPlugin {
    systems: usize,
}

impl SimulationPlugin for KeyedDrawPlugin {
    fn name(&self) -> &'static str {
        PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        PLUGIN_HASH
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut filling = BoundarySystemContract::new(
            "fill",
            BoundaryPhase::StrategicAggregation,
            SystemCadence::Monthly,
        );
        filling.random_streams = vec![stream("fill")];
        registrar.register_boundary_system(filling, fill)?;
        for index in 0..self.systems {
            let name = format!("probe-{index:02}");
            let mut contract = BoundarySystemContract::new(
                name.clone(),
                BoundaryPhase::StrategicAggregation,
                SystemCadence::Daily,
            );
            contract.random_streams = vec![stream(&name)];
            registrar.register_boundary_system(contract, probe)?;
        }
        Ok(())
    }
}

struct Options {
    mode: MetricMode,
    retained: Vec<usize>,
    systems: Vec<usize>,
    draws: usize,
    samples: usize,
    warmup: usize,
    restore_samples: usize,
    output: Option<PathBuf>,
    machine: String,
    recorded_on: String,
    source_label: String,
}

fn next_minute(now: SimTime) -> Result<SimTime, Box<dyn Error>> {
    now.checked_add(SimDuration::minutes(1))
        .ok_or_else(|| "profile time overflow".into())
}

/// Builds a run that retains `retained` keyed draws, rounded up to whole
/// filling boundaries.
fn build(plugin: &KeyedDrawPlugin, retained: usize) -> Result<Canwu, Box<dyn Error>> {
    let plugins: [&dyn SimulationPlugin; 1] = [plugin];
    let mut canwu = Canwu::new_with_plugins(
        0xCA_6E_50,
        Scenario::new(SimTime::EPOCH, Vec::new()),
        &plugins,
    )?;
    for _ in 0..retained.div_ceil(FILL_BATCH) {
        let at = next_minute(canwu.time())?;
        canwu.schedule_calendar_boundary(at, vec![SystemCadence::Monthly])?;
        canwu
            .step_canonical()?
            .ok_or("the filling boundary did not settle")?;
    }
    Ok(canwu)
}

/// Measures one Daily boundary per sample, each on its own fork of `canwu`,
/// so every sample sees the same retained draws. A fork's journals are
/// exactly full, and the first append after a fork copies them once; an
/// unmeasured boundary absorbs that copy before the measured one.
fn measure_live(canwu: &Canwu, options: &Options) -> Result<Measurements, Box<dyn Error>> {
    let mut measurements = Measurements::default();
    let mut discarded = Measurements::default();
    for sample in 0..options.warmup + options.samples {
        let mut fork = canwu.fork();
        let at = next_minute(fork.time())?;
        fork.schedule_calendar_boundary(at, vec![SystemCadence::Daily])?;
        fork.step_canonical()?
            .ok_or("the unmeasured boundary did not settle")?;
        let at = next_minute(at)?;
        fork.schedule_calendar_boundary(at, vec![SystemCadence::Daily])?;
        let target = if sample < options.warmup {
            &mut discarded
        } else {
            &mut measurements
        };
        let receipt = measure(options.mode, target, || fork.step_canonical())?;
        black_box(receipt.ok_or("the measured boundary did not settle")?);
    }
    Ok(measurements)
}

/// Measures Daily boundaries on a sealed run. Before each sample the
/// evidence is sealed again, so every draw kept so far is a keyed
/// reservation, and one unmeasured boundary follows the seal, as one follows
/// the fork in the live case, so the sample does not include the seal's
/// aftermath.
fn measure_sealed(
    compacted: &mut CompactedCanwu,
    options: &Options,
) -> Result<Measurements, Box<dyn Error>> {
    let mut measurements = Measurements::default();
    let mut discarded = Measurements::default();
    for sample in 0..options.warmup + options.samples {
        compacted.seal_evidence()?;
        let at = next_minute(compacted.time())?;
        compacted.schedule_calendar_boundary(at, vec![SystemCadence::Daily])?;
        compacted
            .step_canonical()?
            .ok_or("the unmeasured boundary did not settle")?;
        let at = next_minute(at)?;
        compacted.schedule_calendar_boundary(at, vec![SystemCadence::Daily])?;
        let target = if sample < options.warmup {
            &mut discarded
        } else {
            &mut measurements
        };
        let receipt = measure(options.mode, target, || compacted.step_canonical())?;
        black_box(receipt.ok_or("the measured boundary did not settle")?);
    }
    Ok(measurements)
}

/// Measures loading the run from its portable checkpoint journal.
fn measure_checkpoint_journal_restore(
    canwu: &Canwu,
    plugin: &KeyedDrawPlugin,
    options: &Options,
) -> Result<Measurements, Box<dyn Error>> {
    let bundle = canwu.checkpoint_journal()?;
    let plugins: [&dyn SimulationPlugin; 1] = [plugin];
    let mut measurements = Measurements::default();
    for _ in 0..options.restore_samples {
        let input = bundle.clone();
        let restored = measure(options.mode, &mut measurements, || {
            Canwu::from_checkpoint_journal_with_plugins(input, &plugins)
        })?;
        drop(black_box(restored));
    }
    Ok(measurements)
}

/// A page store that holds nothing, so a paged checkpoint emits every page.
struct NoPages;

impl StatePageProvider for NoPages {
    fn load_state_page(&self, _page_id: &str) -> Result<Option<StatePageBlob>, CanwuError> {
        Ok(None)
    }
}

/// One measured case, or the error an engine returned for it.
type CompactCase = (&'static str, Measurements, Option<String>);

/// Measures the persistence operations of a compact run whose keyed draws
/// are all sealed: a checkpoint, a paged checkpoint, a fork where the engine
/// has one, and a seal of one more boundary's evidence. An operation that an
/// engine refuses (0.15.0 refuses a paged checkpoint whose keyed reservations
/// overflow one state page) is reported with its error instead of samples.
fn measure_compact_persistence(
    compacted: &mut CompactedCanwu,
    options: &Options,
) -> Result<Vec<CompactCase>, Box<dyn Error>> {
    compacted.seal_evidence()?;
    let mut checkpoint = Measurements::default();
    let mut paged = Measurements::default();
    let mut paged_error = None;
    for _ in 0..options.samples {
        let value = measure(options.mode, &mut checkpoint, || compacted.checkpoint())?;
        drop(black_box(value));
        if paged_error.is_none() {
            match measure(options.mode, &mut paged, || {
                compacted.prepare_paged_checkpoint(None, &NoPages)
            }) {
                Ok(value) => drop(black_box(value)),
                Err(error) => paged_error = Some(format!("{:?}: {}", error.code, error.message)),
            }
        }
    }
    let mut cases = vec![
        ("compact_checkpoint", checkpoint, None),
        ("compact_paged_checkpoint", paged, paged_error),
    ];
    #[cfg(feature = "compact-fork")]
    {
        let mut fork = Measurements::default();
        for _ in 0..options.samples {
            let value = measure(options.mode, &mut fork, || compacted.fork());
            drop(black_box(value));
        }
        cases.push(("compact_fork", fork, None));
    }
    let mut seal = Measurements::default();
    let mut discarded = Measurements::default();
    for sample in 0..options.warmup + options.samples {
        let at = next_minute(compacted.time())?;
        compacted.schedule_calendar_boundary(at, vec![SystemCadence::Daily])?;
        compacted
            .step_canonical()?
            .ok_or("the boundary before the seal did not settle")?;
        let target = if sample < options.warmup {
            &mut discarded
        } else {
            &mut seal
        };
        let segment = measure(options.mode, target, || compacted.seal_evidence())?;
        drop(black_box(segment.ok_or("the seal found nothing to seal")?));
    }
    cases.push(("compact_seal", seal, None));
    Ok(cases)
}

/// Measures loading the run from its snapshot JSON with its plugin.
fn measure_restore(
    canwu: &Canwu,
    plugin: &KeyedDrawPlugin,
    options: &Options,
) -> Result<Measurements, Box<dyn Error>> {
    let snapshot = canwu.snapshot_json()?;
    let plugins: [&dyn SimulationPlugin; 1] = [plugin];
    let mut measurements = Measurements::default();
    for _ in 0..options.restore_samples {
        let restored = measure(options.mode, &mut measurements, || {
            Canwu::from_snapshot_json_with_plugins(&snapshot, &plugins)
        })?;
        drop(black_box(restored));
    }
    Ok(measurements)
}

fn median(values: &[u64]) -> u64 {
    let mut ordered = values.to_vec();
    ordered.sort_unstable();
    ordered.get(ordered.len() / 2).copied().unwrap_or_default()
}

fn percentile_95(values: &[u64]) -> u64 {
    let mut ordered = values.to_vec();
    ordered.sort_unstable();
    let index = (ordered.len() * 95).div_ceil(100).saturating_sub(1);
    ordered.get(index).copied().unwrap_or_default()
}

fn case_json(name: &str, measurements: &Measurements, mode: MetricMode) -> Value {
    match mode {
        MetricMode::Elapsed => json!({
            "name": name,
            "summary": {
                "elapsed_ns_median": median(&measurements.elapsed_ns),
                "elapsed_ns_p95": percentile_95(&measurements.elapsed_ns),
                "elapsed_ns_min": measurements.elapsed_ns.iter().copied().min().unwrap_or(0),
                "elapsed_ns_max": measurements.elapsed_ns.iter().copied().max().unwrap_or(0),
            },
            "elapsed_samples_ns": measurements.elapsed_ns,
        }),
        MetricMode::Allocations => {
            let operations: Vec<_> = measurements
                .allocations
                .iter()
                .map(|sample| sample.alloc_calls.saturating_add(sample.realloc_calls))
                .collect();
            let bytes: Vec<_> = measurements
                .allocations
                .iter()
                .map(|sample| sample.allocated_bytes)
                .collect();
            json!({
                "name": name,
                "summary": {
                    "allocation_operations_median": median(&operations),
                    "allocated_bytes_median": median(&bytes),
                },
                "allocation_samples": measurements.allocations.iter().map(|sample| json!({
                    "alloc_calls": sample.alloc_calls,
                    "realloc_calls": sample.realloc_calls,
                    "allocation_operations": sample.alloc_calls.saturating_add(sample.realloc_calls),
                    "dealloc_calls": sample.dealloc_calls,
                    "allocated_bytes": sample.allocated_bytes,
                    "deallocated_bytes": sample.deallocated_bytes,
                    "net_retained_bytes_delta": sample.net_bytes(),
                })).collect::<Vec<_>>(),
            })
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let options = options()?;
    if cfg!(debug_assertions) {
        return Err("run the keyed-draw profile with --release".into());
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
    FRESH_DRAWS.store(options.draws, Ordering::Relaxed);
    let mut scales = Vec::new();
    let combinations = options.systems.iter().flat_map(|&systems| {
        options
            .retained
            .iter()
            .map(move |&retained| (systems, retained))
    });
    for (systems, retained) in combinations {
        let plugin = KeyedDrawPlugin { systems };
        // In the allocation build, the bytes allocated while building the run
        // and still held by it afterwards.
        let (canwu, run_heap_bytes) = if options.mode == MetricMode::Allocations {
            start_allocation_sample();
            let built = build(&plugin, retained);
            let sample = finish_allocation_sample();
            (built?, Some(sample.net_bytes()))
        } else {
            (build(&plugin, retained)?, None)
        };
        let kept = canwu
            .random_draws()
            .iter()
            .filter(|draw| draw.address.sequential_position().is_none())
            .count();
        eprintln!("{systems} due systems, {kept} retained keyed draws");
        let mut cases = vec![case_json(
            "live_boundary",
            &measure_live(&canwu, &options)?,
            options.mode,
        )];
        cases.push(case_json(
            "snapshot_restore",
            &measure_restore(&canwu, &plugin, &options)?,
            options.mode,
        ));
        cases.push(case_json(
            "checkpoint_journal_restore",
            &measure_checkpoint_journal_restore(&canwu, &plugin, &options)?,
            options.mode,
        ));
        if kept > 0 {
            let mut compacted = canwu.into_compacted()?;
            for (name, measurements, error) in
                measure_compact_persistence(&mut compacted, &options)?
            {
                let mut case = case_json(name, &measurements, options.mode);
                if let Some(error) = error {
                    case["error"] = json!(error);
                }
                cases.push(case);
            }
            cases.push(case_json(
                "sealed_boundary",
                &measure_sealed(&mut compacted, &options)?,
                options.mode,
            ));
        }
        let mut scale = json!({
            "due_systems": systems,
            "retained_keyed_draws": kept,
            "cases": cases,
        });
        if let Some(bytes) = run_heap_bytes {
            scale["run_heap_bytes"] = json!(bytes);
        }
        scales.push(scale);
    }
    let report = json!({
        "schema_version": 1,
        "benchmark": "canwu-keyed-draw-retention",
        "metric_mode": options.mode.as_str(),
        "recorded_on": options.recorded_on,
        "machine": options.machine,
        "engine_version": ENGINE_VERSION,
        "source": {
            "label": options.source_label,
            "content_hash": source_content_hash(),
        },
        "environment": {
            "os": env::consts::OS,
            "arch": env::consts::ARCH,
            "logical_cpus": std::thread::available_parallelism().map_or(1, usize::from),
            "rustc": toolchain_output("rustc", &["--version", "--verbose"]),
            "cargo": toolchain_output("cargo", &["--version"]),
            "profile": "release",
            "cargo_features": if cfg!(feature = "allocation-counting") { vec!["allocation-counting"] } else { Vec::<&str>::new() },
        },
        "method": {
            "due_systems": options.systems,
            "fresh_keyed_draws_per_system": options.draws,
            "keyed_draws_per_filling_boundary": FILL_BATCH,
            "warmup_iterations": options.warmup,
            "boundary_samples": options.samples,
            "restore_samples": options.restore_samples,
            "live_boundary": "one Daily boundary on a fork of the run, after one unmeasured boundary on that fork",
            "sealed_boundary": "one Daily boundary on the sealed run; before each sample the evidence is sealed again and one unmeasured boundary settles",
            "snapshot_restore": "Canwu::from_snapshot_json_with_plugins of the run's snapshot JSON",
            "checkpoint_journal_restore": "Canwu::from_checkpoint_journal_with_plugins of the run's checkpoint journal",
            "compact_persistence": "after one seal of the whole run: checkpoint(), prepare_paged_checkpoint against an empty page store, CompactedCanwu::fork where compiled, and seal_evidence of one more Daily boundary",
            "run_heap_bytes": "allocation build only: bytes allocated while building the run and still held by it",
            "allocation_scope": if options.mode == MetricMode::Allocations { "thread-local allocator calls and requested bytes during the operation; setup and post-measurement drops are excluded" } else { "not collected in this uninstrumented build" },
            "elapsed_scope": if options.mode == MetricMode::Elapsed { "wall-clock nanoseconds in a build with the default system allocator; setup and post-measurement drops are excluded" } else { "not collected in the allocation-instrumented build" },
        },
        "scales": scales,
    });
    print_summary(&report, options.mode);
    let encoded = serde_json::to_string_pretty(&report)?;
    if let Some(output) = &options.output {
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(output, format!("{encoded}\n"))?;
        eprintln!("wrote {}", output.display());
    } else {
        println!("{encoded}");
    }
    Ok(())
}

fn print_summary(report: &Value, mode: MetricMode) {
    let (key, unit) = match mode {
        MetricMode::Elapsed => ("elapsed_ns_median", "ns"),
        MetricMode::Allocations => ("allocation_operations_median", "allocations"),
    };
    for scale in report["scales"].as_array().into_iter().flatten() {
        for case in scale["cases"].as_array().into_iter().flatten() {
            eprintln!(
                "{:>3} systems {:>9} retained  {:<17} {:>12} {unit}",
                scale["due_systems"],
                scale["retained_keyed_draws"],
                case["name"],
                case["summary"][key]
            );
        }
        if !scale["run_heap_bytes"].is_null() {
            eprintln!(
                "{:>3} systems {:>9} retained  {:<17} {:>12} bytes",
                scale["due_systems"],
                scale["retained_keyed_draws"],
                "run_heap_bytes",
                scale["run_heap_bytes"]
            );
        }
    }
}

/// Asks the toolchain that built this binary for its version. The command
/// runs in the binary's own manifest directory, so the toolchain that source
/// tree pins answers, not the one the working directory resolves to; under
/// `cargo run` the building toolchain is also passed on in the environment.
fn toolchain_output(program: &str, arguments: &[&str]) -> String {
    ProcessCommand::new(program)
        .args(arguments)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_default()
}

/// A BLAKE3 hash over the engine sources this binary measures and its own
/// source, so a report names its source without a Git checkout.
fn source_content_hash() -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for relative in [
        "crates/foundation/canwu-core/src",
        "crates/runtime/canwu-sim/src",
        "crates/api/canwu-api/src",
        "benchmarks/performance-harness/src/bin/keyed-draw-profile.rs",
    ] {
        collect_source_files(&root.join(relative), &mut files);
    }
    files.sort();
    let mut hasher = blake3::Hasher::new();
    for path in files {
        if let (Ok(relative), Ok(bytes)) = (path.strip_prefix(&root), fs::read(&path)) {
            hasher.update(relative.to_string_lossy().replace('\\', "/").as_bytes());
            hasher.update(&[0]);
            // Line endings differ between a checkout and an export.
            let text: Vec<u8> = bytes.into_iter().filter(|byte| *byte != b'\r').collect();
            hasher.update(&text);
            hasher.update(&[0]);
        }
    }
    hasher.finalize().to_hex().to_string()
}

fn collect_source_files(path: &Path, files: &mut Vec<PathBuf>) {
    if path.is_file() {
        files.push(path.to_path_buf());
    } else if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            collect_source_files(&entry.path(), files);
        }
    }
}

fn options() -> Result<Options, Box<dyn Error>> {
    let mut options = Options {
        mode: MetricMode::Elapsed,
        retained: vec![0, 1_000, 10_000, 100_000],
        systems: vec![8],
        draws: 1,
        samples: 31,
        warmup: 3,
        restore_samples: 3,
        output: None,
        machine: "local-machine".to_owned(),
        recorded_on: "unspecified".to_owned(),
        source_label: "working tree".to_owned(),
    };
    let mut arguments = env::args().skip(1);
    while let Some(argument) = arguments.next() {
        let mut value = || {
            arguments
                .next()
                .ok_or_else(|| format!("{argument} requires a value"))
        };
        match argument.as_str() {
            "--mode" => {
                options.mode = match value()?.as_str() {
                    "elapsed" => MetricMode::Elapsed,
                    "allocations" => MetricMode::Allocations,
                    other => return Err(format!("unsupported metric mode: {other}").into()),
                };
            }
            "--retained" => options.retained = counts(&value()?, "--retained")?,
            "--systems" => options.systems = counts(&value()?, "--systems")?,
            "--draws" => options.draws = value()?.parse()?,
            "--samples" => options.samples = positive_odd(&value()?, "--samples")?,
            "--restore-samples" => {
                options.restore_samples = positive_odd(&value()?, "--restore-samples")?;
            }
            "--warmup" => options.warmup = value()?.parse()?,
            "--output" => options.output = Some(PathBuf::from(value()?)),
            "--machine" => options.machine = value()?,
            "--recorded-on" => options.recorded_on = value()?,
            "--source-label" => options.source_label = value()?,
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }
    Ok(options)
}

/// A sorted list of distinct counts.
fn counts(value: &str, flag: &str) -> Result<Vec<usize>, Box<dyn Error>> {
    let mut parsed = value
        .split(',')
        .map(str::parse)
        .collect::<Result<Vec<usize>, _>>()
        .map_err(|_| format!("{flag} requires comma-separated counts"))?;
    parsed.sort_unstable();
    parsed.dedup();
    Ok(parsed)
}

fn positive_odd(value: &str, flag: &str) -> Result<usize, Box<dyn Error>> {
    let parsed: usize = value.parse()?;
    if parsed == 0 || parsed % 2 == 0 {
        return Err(format!("{flag} must be a positive odd integer").into());
    }
    Ok(parsed)
}

fn print_help() {
    println!(
        "Canwu keyed-draw retention profile\n\n\
         Usage: cargo run --release --manifest-path benchmarks/performance-harness/Cargo.toml \
         --bin keyed-draw-profile -- [options]\n\n\
         Options:\n\
           --mode MODE             elapsed or allocations\n\
           --retained LIST         Comma-separated retained keyed-draw counts\n\
           --systems LIST          Comma-separated counts of boundary systems due\n\
           --draws N               Fresh keyed draws each due system makes\n\
           --samples N             Positive odd sample count for each boundary case\n\
           --restore-samples N     Positive odd sample count for the snapshot restore\n\
           --warmup N              Unmeasured boundaries before the samples\n\
           --output PATH           Write the JSON report to PATH\n\
           --machine LABEL         Stable description of the measurement machine\n\
           --recorded-on DATE      Recording date or build label\n\
           --source-label LABEL    What source tree the binary was built from\n"
    );
}
