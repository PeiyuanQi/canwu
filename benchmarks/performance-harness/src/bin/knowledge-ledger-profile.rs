//! Boundary cost against held generic knowledge.
//!
//! The workload fills a run with `--held` knowledge records kept the way a
//! reporting plugin keeps them: every holder has `--lines` lines, each fill
//! boundary publishes `--per-line` records on every line of every holder,
//! and each of them supersedes the line's current records, found through
//! `SimulationView::knowledge_heads`. Most held records are therefore
//! superseded history. On that run it measures, for each held count:
//!
//! - `idle_boundary`: a boundary in which no system is due;
//! - `publish_boundary`: a boundary in which one system publishes one record
//!   that supersedes its line's current record;
//! - `heads_boundary`: a boundary in which one system calls
//!   `knowledge_heads` for `--queries` lines and publishes nothing;
//! - `snapshot_restore`: loading the run from its snapshot JSON, which
//!   validates every record and rebuilds what the engine derives from them.
//!
//! Every sample settles on its own fork of the built run, after one
//! unmeasured boundary on that fork, so every sample sees the same ledger.
//!
//! Elapsed time and allocation traffic are collected in separate release
//! builds, as in the main harness: `--mode elapsed` without the
//! `allocation-counting` feature, `--mode allocations` with it. The
//! allocation build also reports `run_heap_bytes`, the bytes the built run
//! holds. The binary publishes through the 0.17.0 directive (one group of
//! one holder per line), so it measures 0.17.0 and later.

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundaryRequest,
    BoundarySystemContract, Canwu, CanwuError, ENGINE_VERSION, EntityRef, EventId,
    KnowledgeHolderRef, KnowledgeOrigin, KnowledgeRecordDraft, KnowledgeRecordKind,
    KnowledgeSchemaId, KnowledgeSubject, KnowledgeSubjectSchema, KnowledgeSubjectTarget,
    KnowledgeSubjectTargetKind, KnowledgeWriteGrant, PayloadSchema, PersonId,
    PluginKnowledgeSchema, PluginRegistrar, Scenario, SimDuration, SimTime, SimulationPlugin,
    SimulationView, StateKey, StateVisibility, SystemCadence,
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

const PLUGIN: &str = "profile-knowledge-ledger";
const PLUGIN_HASH: &str = "00000000000000000000000000000000000000000000000000000000000c1ed6";
const SCHEMA_HASH: &str = "00000000000000000000000000000000000000000000000000000000000c1ed7";

/// Workload shape, set once from the options. Boundary system handlers are
/// plain function pointers, so they read it from here.
static HOLDERS: AtomicUsize = AtomicUsize::new(4);
static LINES: AtomicUsize = AtomicUsize::new(16);
static PER_LINE: AtomicUsize = AtomicUsize::new(32);
static PAYLOAD_BYTES: AtomicUsize = AtomicUsize::new(256);
static QUERIES: AtomicUsize = AtomicUsize::new(64);

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

fn kind() -> KnowledgeRecordKind {
    KnowledgeRecordKind::new("profile.knowledge", "report")
}

fn schema() -> KnowledgeSchemaId {
    KnowledgeSchemaId::new(kind(), 1)
}

fn holder(index: usize) -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(PersonId::new(u64::try_from(index).unwrap_or(u64::MAX) + 1))
}

/// A line is told apart by its subject, as a reporting plugin does.
fn line(index: usize) -> KnowledgeSubject {
    KnowledgeSubject {
        role: "line".to_owned(),
        target: KnowledgeSubjectTarget::Event(EventId::new(
            u64::try_from(index).unwrap_or(u64::MAX) + 1,
        )),
    }
}

fn draft(
    line_index: usize,
    copy: usize,
    supersedes: Vec<canwu_api::KnowledgeRecordId>,
    context: &BoundaryContext,
) -> KnowledgeRecordDraft {
    KnowledgeRecordDraft {
        schema: schema(),
        subjects: vec![line(line_index)],
        payload: json!({
            "boundary": context.boundary_id.get(),
            "copy": copy,
            "pad": "x".repeat(PAYLOAD_BYTES.load(Ordering::Relaxed)),
        }),
        as_of: None,
        confidence_per_mille: 1_000,
        origin: KnowledgeOrigin {
            method: "profile".to_owned(),
            evidence: Vec::new(),
        },
        supersedes,
        contradicts: Vec::new(),
    }
}

/// Monthly: `--per-line` records on every line of every holder, each
/// superseding the line's current records.
fn fill(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut directives = Vec::new();
    for holder_index in 0..HOLDERS.load(Ordering::Relaxed) {
        let holder = holder(holder_index);
        for line_index in 0..LINES.load(Ordering::Relaxed) {
            let heads = view.knowledge_heads(&holder, &kind(), &[line(line_index)])?;
            let records = (0..PER_LINE.load(Ordering::Relaxed))
                .map(|copy| draft(line_index, copy, heads.records.clone(), context))
                .collect();
            directives.push(BoundaryDirective::PublishKnowledge {
                visibility: StateVisibility::SameBoundary,
                producer_correlation: None,
                supersession: canwu_api::KnowledgeSupersession::Explicit,
                groups: vec![canwu_api::KnowledgePublicationGroup {
                    holders: vec![holder.clone()],
                    records,
                }],
                summary: "Fill one line".to_owned(),
            });
        }
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

/// Daily: one record on the first line of the first holder, superseding the
/// line's current records.
fn publish(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let holder = holder(0);
    let heads = view.knowledge_heads(&holder, &kind(), &[line(0)])?;
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::PublishKnowledge {
            visibility: StateVisibility::SameBoundary,
            producer_correlation: None,
            supersession: canwu_api::KnowledgeSupersession::Explicit,
            groups: vec![canwu_api::KnowledgePublicationGroup {
                holders: vec![holder],
                records: vec![draft(0, 0, heads.records, context)],
            }],
            summary: "Publish one record".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

/// Seasonal: `--queries` current-record reads across the holders' lines.
fn heads(
    view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let holders = HOLDERS.load(Ordering::Relaxed).max(1);
    let lines = LINES.load(Ordering::Relaxed).max(1);
    for query in 0..QUERIES.load(Ordering::Relaxed) {
        let heads = view.knowledge_heads(
            &holder(query % holders),
            &kind(),
            &[line((query / holders) % lines)],
        )?;
        black_box(heads);
    }
    Ok(BoundaryProposal::default())
}

struct KnowledgeLedgerPlugin;

impl SimulationPlugin for KnowledgeLedgerPlugin {
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
        registrar.register_knowledge_schema(PluginKnowledgeSchema {
            id: schema(),
            schema_hash: SCHEMA_HASH.to_owned(),
            writable: true,
            payload_schema: PayloadSchema::Any,
            subjects: vec![KnowledgeSubjectSchema {
                role: "line".to_owned(),
                targets: vec![KnowledgeSubjectTargetKind::Event],
                required: true,
                multiple: false,
            }],
            retention: canwu_api::KnowledgeRetention::Keep,
        })?;
        registrar.register_boundary_system(contract("fill", SystemCadence::Monthly, true), fill)?;
        registrar
            .register_boundary_system(contract("publish", SystemCadence::Daily, true), publish)?;
        registrar.register_boundary_system(contract("heads", SystemCadence::Seasonal, false), heads)
    }
}

fn contract(name: &str, cadence: SystemCadence, writes: bool) -> BoundarySystemContract {
    let mut contract =
        BoundarySystemContract::new(name, BoundaryPhase::PerceptionAndAttentionRefresh, cadence);
    contract.reads = vec![StateKey::core_knowledge()];
    if writes {
        contract.knowledge_writes = vec![KnowledgeWriteGrant {
            schema: schema(),
            visibilities: vec![StateVisibility::SameBoundary],
        }];
    }
    contract
}

struct Options {
    mode: MetricMode,
    held: Vec<usize>,
    holders: usize,
    lines: usize,
    per_line: usize,
    payload_bytes: usize,
    queries: usize,
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

fn held_records(canwu: &Canwu) -> usize {
    canwu
        .knowledge()
        .records
        .values()
        .map(|ledger| ledger.len())
        .sum()
}

/// Builds a run that holds at least `held` records, in whole fill boundaries.
fn build(
    plugin: &KnowledgeLedgerPlugin,
    options: &Options,
    held: usize,
) -> Result<Canwu, Box<dyn Error>> {
    let plugins: [&dyn SimulationPlugin; 1] = [plugin];
    let entities = (0..options.holders)
        .map(|index| match holder(index) {
            KnowledgeHolderRef::Person(person) => EntityRef::Person(person),
            KnowledgeHolderRef::Entity(entity) => entity,
        })
        .collect();
    let mut canwu = Canwu::new_with_plugins(
        0x6B_4C_ED,
        Scenario::new(SimTime::EPOCH, entities),
        &plugins,
    )?;
    let per_fill = options.holders * options.lines * options.per_line;
    for _ in 0..held.div_ceil(per_fill.max(1)) {
        let at = next_minute(canwu.time())?;
        canwu.settle_boundary(BoundaryRequest::at(at).with_cadence(SystemCadence::Monthly))?;
    }
    Ok(canwu)
}

/// Measures one boundary per sample, each on its own fork of `canwu`, after
/// one unmeasured boundary of the same request on that fork.
fn measure_case(
    canwu: &Canwu,
    options: &Options,
    cadence: Option<SystemCadence>,
) -> Result<Measurements, Box<dyn Error>> {
    let request = |at: SimTime| match &cadence {
        Some(cadence) => BoundaryRequest::at(at).with_cadence(cadence.clone()),
        None => BoundaryRequest::at(at),
    };
    let mut measurements = Measurements::default();
    let mut discarded = Measurements::default();
    for sample in 0..options.warmup + options.samples {
        let mut fork = canwu.fork();
        let at = next_minute(fork.time())?;
        fork.settle_boundary(request(at))?;
        let at = next_minute(at)?;
        let target = if sample < options.warmup {
            &mut discarded
        } else {
            &mut measurements
        };
        let measured = request(at);
        let receipt = measure(options.mode, target, || fork.settle_boundary(measured))?;
        black_box(receipt);
    }
    Ok(measurements)
}

/// Measures loading the run from its snapshot JSON with its plugin.
fn measure_restore(
    canwu: &Canwu,
    plugin: &KnowledgeLedgerPlugin,
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
        return Err("run the knowledge-ledger profile with --release".into());
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
    HOLDERS.store(options.holders, Ordering::Relaxed);
    LINES.store(options.lines, Ordering::Relaxed);
    PER_LINE.store(options.per_line, Ordering::Relaxed);
    PAYLOAD_BYTES.store(options.payload_bytes, Ordering::Relaxed);
    QUERIES.store(options.queries, Ordering::Relaxed);
    let plugin = KnowledgeLedgerPlugin;
    let mut scales = Vec::new();
    for &held in &options.held {
        let (canwu, run_heap_bytes) = if options.mode == MetricMode::Allocations {
            start_allocation_sample();
            let built = build(&plugin, &options, held);
            let sample = finish_allocation_sample();
            (built?, Some(sample.net_bytes()))
        } else {
            (build(&plugin, &options, held)?, None)
        };
        let records = held_records(&canwu);
        let snapshot_bytes = canwu.snapshot_json()?.len();
        eprintln!("{records} held knowledge records, {snapshot_bytes} snapshot bytes");
        let cases = vec![
            case_json(
                "idle_boundary",
                &measure_case(&canwu, &options, None)?,
                options.mode,
            ),
            case_json(
                "publish_boundary",
                &measure_case(&canwu, &options, Some(SystemCadence::Daily))?,
                options.mode,
            ),
            case_json(
                "heads_boundary",
                &measure_case(&canwu, &options, Some(SystemCadence::Seasonal))?,
                options.mode,
            ),
            case_json(
                "snapshot_restore",
                &measure_restore(&canwu, &plugin, &options)?,
                options.mode,
            ),
        ];
        let mut scale = json!({
            "held_records": records,
            "current_records": if records == 0 { 0 } else { options.holders * options.lines * options.per_line },
            "snapshot_json_bytes": snapshot_bytes,
            "cases": cases,
        });
        if let Some(bytes) = run_heap_bytes {
            scale["run_heap_bytes"] = json!(bytes);
        }
        scales.push(scale);
    }
    let report = json!({
        "schema_version": 1,
        "benchmark": "canwu-knowledge-ledger",
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
            "holders": options.holders,
            "lines_per_holder": options.lines,
            "records_per_line_per_fill": options.per_line,
            "payload_pad_bytes": options.payload_bytes,
            "heads_queries": options.queries,
            "warmup_iterations": options.warmup,
            "boundary_samples": options.samples,
            "restore_samples": options.restore_samples,
            "idle_boundary": "a boundary with no due system, on a fork of the run, after one unmeasured boundary on that fork",
            "publish_boundary": "a Daily boundary whose one system publishes one record superseding its line, measured as above",
            "heads_boundary": "a Seasonal boundary whose one system calls knowledge_heads --queries times and publishes nothing, measured as above",
            "snapshot_restore": "Canwu::from_snapshot_json_with_plugins of the run's snapshot JSON",
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
                "{:>9} held  {:<17} {:>12} {unit}",
                scale["held_records"], case["name"], case["summary"][key]
            );
        }
        if !scale["run_heap_bytes"].is_null() {
            eprintln!(
                "{:>9} held  {:<17} {:>12} bytes",
                scale["held_records"], "run_heap_bytes", scale["run_heap_bytes"]
            );
        }
    }
}

/// Asks the toolchain that built this binary for its version. The command
/// runs in the binary's own manifest directory, so the toolchain that source
/// tree pins answers, not the one the working directory resolves to.
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
        "crates/model/canwu-knowledge/src",
        "crates/runtime/canwu-sim/src",
        "crates/api/canwu-api/src",
        "benchmarks/performance-harness/src/bin/knowledge-ledger-profile.rs",
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
        held: vec![0, 2_048, 16_384, 65_536],
        holders: 4,
        lines: 16,
        per_line: 32,
        payload_bytes: 256,
        queries: 64,
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
            "--held" => options.held = counts(&value()?, "--held")?,
            "--holders" => options.holders = value()?.parse()?,
            "--lines" => options.lines = value()?.parse()?,
            "--per-line" => options.per_line = value()?.parse()?,
            "--payload-bytes" => options.payload_bytes = value()?.parse()?,
            "--queries" => options.queries = value()?.parse()?,
            "--samples" => options.samples = positive_odd(&value()?, "--samples")?,
            "--warmup" => options.warmup = value()?.parse()?,
            "--restore-samples" => {
                options.restore_samples = positive_odd(&value()?, "--restore-samples")?;
            }
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
    if options.holders == 0 || options.lines == 0 || options.per_line == 0 {
        return Err("--holders, --lines, and --per-line must be positive".into());
    }
    if options.per_line > 64 {
        return Err("--per-line must be at most 64, the records one fill can supersede".into());
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
        "Canwu knowledge-ledger profile\n\n\
         Usage: cargo run --release --manifest-path benchmarks/performance-harness/Cargo.toml \
         --bin knowledge-ledger-profile -- [options]\n\n\
         Options:\n\
           --mode MODE             elapsed or allocations\n\
           --held LIST             Comma-separated held-record counts\n\
           --holders N             Knowledge holders\n\
           --lines N               Lines per holder\n\
           --per-line N            Records each fill boundary publishes on a line (at most 64)\n\
           --payload-bytes N       Padding bytes in each record payload\n\
           --queries N             knowledge_heads calls in the heads boundary\n\
           --samples N             Positive odd sample count for each boundary case\n\
           --warmup N              Unmeasured samples before the measured ones\n\
           --restore-samples N     Positive odd sample count for the snapshot restore\n\
           --output PATH           Write the JSON report to PATH\n\
           --machine LABEL         Stable description of the measurement machine\n\
           --recorded-on DATE      Recording date or build label\n\
           --source-label LABEL    What source tree the binary was built from\n"
    );
}
