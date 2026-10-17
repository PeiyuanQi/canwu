//! Write and paged-checkpoint cost of large domain records.
//!
//! The workload measures two things a plugin that keeps its state in a few
//! large records pays:
//!
//! - `fork`: `Canwu::fork` of the filled run, and `fork_first_boundary`, the
//!   first Daily boundary on a fresh fork, which writes one record (the
//!   unmeasured boundary before each write below), so that a cost a fork
//!   defers to its first boundary is measured too;
//! - `idle_boundary`: one Daily boundary whose system writes nothing, on the
//!   same forks as the writes, the fixed cost every write below includes;
//! - `write_<size>`: one Daily boundary whose one system replaces one record
//!   of a store of `--records` small records with a record whose payload
//!   text is `<size>` bytes, measured on a fork of the filled run after one
//!   unmeasured boundary on that fork;
//! - for each `--paged` record set, a run whose store holds exactly those
//!   records: `paged_prepare`, preparing its paged checkpoint into an empty
//!   page store, and `paged_restore`, restoring it with its evidence prefix
//!   and plugin from a store that holds the prepared pages. A set the engine
//!   cannot page is reported with the error it returned.
//!
//! Elapsed time and allocation traffic are collected in separate release
//! builds, as in the main harness: `--mode elapsed` without the
//! `allocation-counting` feature, `--mode allocations` with it.
//!
//! The binary uses only the public API, so the same source measures any
//! engine version it compiles against.

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundaryRequest,
    BoundarySystemContract, Canwu, CanwuError, DomainRecordClass, DomainRecordDraft,
    DomainRecordKind, DomainRecordMutation, DomainRecordRef, DomainRecordSchema, ENGINE_VERSION,
    EvidenceCursor, EvidenceJournalSegment, PagedRestoreBudget, PluginRegistrar, Scenario,
    SimDuration, SimTime, SimulationPlugin, SimulationView, StatePageBlob, StatePageProvider,
    StatePageStore, StateVisibility, SystemCadence,
};
use serde_json::{Value, json};
#[cfg(feature = "allocation-counting")]
use std::alloc::{GlobalAlloc, Layout, System};
#[cfg(feature = "allocation-counting")]
use std::cell::Cell;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::sync::Mutex;
use std::time::Instant;

const PLUGIN: &str = "profile-paged-records";
const PLUGIN_HASH: &str = "00000000000000000000000000000000000000000000000000000000000ba9ed";
/// Small records one filling boundary creates.
const FILL_BATCH: usize = 4_096;
/// The record every measured write replaces.
const WRITTEN_RECORD: &str = "record-000000";

/// The directives the next boundary proposes. Boundary system handlers are
/// plain function pointers, so the handler takes its work from here; the
/// payloads are built before the measured boundary starts.
static PENDING: Mutex<Vec<PendingWrite>> = Mutex::new(Vec::new());

enum PendingWrite {
    Create(String, Value),
    Update(String, Value),
}

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

fn kind() -> DomainRecordKind {
    DomainRecordKind::new(PLUGIN, "volume")
}

fn reference(id: &str) -> DomainRecordRef {
    DomainRecordRef {
        kind: kind(),
        id: id.to_owned(),
    }
}

fn record_name(ordinal: usize) -> String {
    format!("record-{ordinal:06}")
}

/// Numbered payload text of exactly `bytes` bytes. `salt` makes each sample
/// write a different value.
fn payload(bytes: usize, salt: usize) -> Value {
    let mut text = String::with_capacity(bytes + 16);
    let mut ordinal = salt.wrapping_mul(1_000_003);
    while text.len() < bytes {
        let _ = write!(text, "{:015},", ordinal % 1_000_000_000_000_000);
        ordinal = ordinal.wrapping_add(1);
    }
    text.truncate(bytes);
    json!({ "text": text })
}

/// Proposes the pending writes.
fn keep(
    view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let pending = std::mem::take(&mut *PENDING.lock().expect("pending writes"));
    let mut directives = Vec::with_capacity(pending.len());
    for write in pending {
        directives.push(match write {
            PendingWrite::Create(id, payload) => BoundaryDirective::MutateRecord {
                mutation: DomainRecordMutation::Create {
                    record: DomainRecordDraft::new(reference(&id), payload),
                },
                summary: "Created a profile record".to_owned(),
            },
            PendingWrite::Update(id, payload) => {
                let current = view
                    .domain_record(&reference(&id))?
                    .map_or(0, |record| record.version);
                BoundaryDirective::MutateRecord {
                    mutation: DomainRecordMutation::Update {
                        record: DomainRecordDraft::new(reference(&id), payload),
                        expected_version: current,
                    },
                    summary: "Replaced a profile record".to_owned(),
                }
            }
        });
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

struct RecordPlugin;

impl SimulationPlugin for RecordPlugin {
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
        let schema = DomainRecordSchema::new(kind(), DomainRecordClass::Record);
        let state = schema.state_key();
        registrar.register_record_schema(schema)?;
        let mut contract = BoundarySystemContract::new(
            "keep",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        contract.reads = vec![state.clone()];
        contract.writes = vec![state];
        contract.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(contract, keep)
    }
}

/// A content-addressed page store, as a host backs it.
#[derive(Default)]
struct PageStore(RefCell<BTreeMap<String, StatePageBlob>>);

impl StatePageProvider for PageStore {
    fn load_state_page(&self, page_id: &str) -> Result<Option<StatePageBlob>, CanwuError> {
        Ok(self.0.borrow().get(page_id).cloned())
    }
}

impl StatePageStore for PageStore {
    fn store_state_page(
        &self,
        page: &StatePageBlob,
    ) -> Result<canwu_api::ArchiveStoreOutcome, CanwuError> {
        let mut pages = self.0.borrow_mut();
        if pages.contains_key(&page.page_id) {
            return Ok(canwu_api::ArchiveStoreOutcome::AlreadyPresent);
        }
        pages.insert(page.page_id.clone(), page.clone());
        Ok(canwu_api::ArchiveStoreOutcome::Stored)
    }
}

struct Options {
    mode: MetricMode,
    records: usize,
    write_bytes: Vec<usize>,
    paged: Vec<Vec<usize>>,
    samples: usize,
    warmup: usize,
    paged_samples: usize,
    output: Option<PathBuf>,
    machine: String,
    recorded_on: String,
    source_label: String,
}

/// One run and the evidence prefix a restore needs.
struct Run {
    canwu: Canwu,
    cursor: EvidenceCursor,
    segments: Vec<EvidenceJournalSegment>,
}

impl Run {
    fn new() -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            canwu: Canwu::new_with_plugins(
                0x9A_6E_D0,
                Scenario::new(SimTime::EPOCH, Vec::new()),
                &[&RecordPlugin],
            )?,
            cursor: EvidenceCursor::default(),
            segments: Vec::new(),
        })
    }

    fn settle(&mut self, writes: Vec<PendingWrite>) -> Result<(), Box<dyn Error>> {
        *PENDING.lock().expect("pending writes") = writes;
        let at = next_day(self.canwu.time())?;
        self.canwu
            .settle_boundary(BoundaryRequest::at(at).with_cadence(SystemCadence::Daily))?;
        self.segments
            .push(self.canwu.journal_segment_since(self.cursor)?);
        self.cursor = self.canwu.evidence_cursor()?;
        Ok(())
    }
}

fn next_day(now: SimTime) -> Result<SimTime, Box<dyn Error>> {
    now.checked_add(SimDuration::days(1))
        .ok_or_else(|| "profile time overflow".into())
}

/// Builds a run holding `records` small records.
fn build_store(records: usize) -> Result<Run, Box<dyn Error>> {
    let mut run = Run::new()?;
    for start in (0..records).step_by(FILL_BATCH) {
        let writes = (start..records.min(start + FILL_BATCH))
            .map(|ordinal| {
                PendingWrite::Create(
                    record_name(ordinal),
                    json!({ "ordinal": ordinal, "balance": 0 }),
                )
            })
            .collect();
        run.settle(writes)?;
    }
    Ok(run)
}

/// Measures one record write per sample, each on its own fork of `run`. A
/// fork's journals are exactly full, and the first append after a fork
/// copies them once; an unmeasured boundary that writes another record, and
/// so appends to every journal a write appends to, absorbs that copy first.
fn measure_writes(
    run: &Run,
    bytes: Option<usize>,
    options: &Options,
) -> Result<Measurements, Box<dyn Error>> {
    let mut measurements = Measurements::default();
    let mut discarded = Measurements::default();
    for sample in 0..options.warmup + options.samples {
        let mut fork = run.canwu.fork();
        *PENDING.lock().expect("pending writes") = vec![PendingWrite::Update(
            record_name(1),
            json!({ "ordinal": 1, "balance": sample }),
        )];
        let at = next_day(fork.time())?;
        fork.settle_boundary(BoundaryRequest::at(at).with_cadence(SystemCadence::Daily))?;
        *PENDING.lock().expect("pending writes") = bytes
            .into_iter()
            .map(|bytes| PendingWrite::Update(WRITTEN_RECORD.to_owned(), payload(bytes, sample)))
            .collect();
        let at = next_day(at)?;
        let target = if sample < options.warmup {
            &mut discarded
        } else {
            &mut measurements
        };
        let receipt = measure(options.mode, target, || {
            fork.settle_boundary(BoundaryRequest::at(at).with_cadence(SystemCadence::Daily))
        })?;
        black_box(receipt);
        drop(black_box(fork));
    }
    Ok(measurements)
}

/// Measures `Canwu::fork` of `run`, and the first boundary on a fresh fork,
/// which writes one record as the unmeasured boundary of `measure_writes`
/// does. The fork and the forked run are dropped outside the measurement.
fn measure_fork(run: &Run, options: &Options) -> Result<[Measurements; 2], Box<dyn Error>> {
    let mut fork_cost = Measurements::default();
    let mut first_boundary = Measurements::default();
    let mut discarded = [Measurements::default(), Measurements::default()];
    for sample in 0..options.warmup + options.samples {
        let [fork_target, boundary_target] = if sample < options.warmup {
            let [fork, boundary] = &mut discarded;
            [fork, boundary]
        } else {
            [&mut fork_cost, &mut first_boundary]
        };
        let mut fork = measure(options.mode, fork_target, || run.canwu.fork());
        *PENDING.lock().expect("pending writes") = vec![PendingWrite::Update(
            record_name(1),
            json!({ "ordinal": 1, "balance": sample }),
        )];
        let at = next_day(fork.time())?;
        let receipt = measure(options.mode, boundary_target, || {
            fork.settle_boundary(BoundaryRequest::at(at).with_cadence(SystemCadence::Daily))
        })?;
        black_box(receipt);
        drop(black_box(fork));
    }
    Ok([fork_cost, first_boundary])
}

/// Builds a run whose store holds one record per entry of `sizes`, then
/// measures preparing its paged checkpoint and restoring it.
fn measure_paged(sizes: &[usize], options: &Options) -> Result<Value, Box<dyn Error>> {
    let mut run = Run::new()?;
    run.settle(
        sizes
            .iter()
            .enumerate()
            .map(|(index, bytes)| {
                PendingWrite::Create(format!("large-{index}"), payload(*bytes, index))
            })
            .collect(),
    )?;
    let mut prepare = Measurements::default();
    let mut restore = Measurements::default();
    let mut delta = None;
    for _ in 0..options.paged_samples {
        let store = PageStore::default();
        let prepared = match measure(options.mode, &mut prepare, || {
            run.canwu.prepare_paged_checkpoint(None, &store)
        }) {
            Ok(prepared) => prepared,
            Err(error) => {
                return Ok(json!({
                    "payload_bytes": sizes,
                    "refused": { "code": format!("{:?}", error.code), "message": error.message },
                }));
            }
        };
        delta = Some((
            prepared.delta.new_pages.len(),
            prepared
                .delta
                .new_pages
                .iter()
                .map(|page| page.bytes.len())
                .sum::<usize>(),
            prepared
                .delta
                .new_pages
                .iter()
                .map(|page| page.bytes.len())
                .max()
                .unwrap_or(0),
        ));
        prepared.store_and_verify(&store)?;
        let restored = measure(options.mode, &mut restore, || {
            Canwu::from_paged_checkpoint_and_journal_with_plugins(
                &prepared.checkpoint,
                &store,
                run.segments.clone(),
                &[&RecordPlugin],
                PagedRestoreBudget::new(prepared.checkpoint.assembled_bytes),
            )
        })?;
        if restored.checkpoint_hash() != run.canwu.checkpoint_hash() {
            return Err("the restored run has a different checkpoint hash".into());
        }
        drop(black_box(restored));
    }
    let (pages, bytes, largest) = delta.unwrap_or_default();
    Ok(json!({
        "payload_bytes": sizes,
        "delta_pages": pages,
        "delta_page_bytes": bytes,
        "largest_page_bytes": largest,
        "cases": [
            case_json("paged_prepare", &prepare, options.mode),
            case_json("paged_restore", &restore, options.mode),
        ],
    }))
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
        return Err("run the paged-record profile with --release".into());
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
    let run = build_store(options.records)?;
    if run
        .canwu
        .domain_record(&reference(&record_name(options.records - 1)))
        .is_none()
    {
        return Err("the write store is missing its last record".into());
    }
    eprintln!("{} records in the write store", options.records);
    let [fork_cost, fork_first_boundary] = measure_fork(&run, &options)?;
    let mut writes = vec![
        case_json("fork", &fork_cost, options.mode),
        case_json("fork_first_boundary", &fork_first_boundary, options.mode),
        case_json(
            "idle_boundary",
            &measure_writes(&run, None, &options)?,
            options.mode,
        ),
    ];
    for &bytes in &options.write_bytes {
        writes.push(case_json(
            &format!("write_{bytes}"),
            &measure_writes(&run, Some(bytes), &options)?,
            options.mode,
        ));
    }
    drop(run);
    let mut paged = Vec::new();
    for sizes in &options.paged {
        paged.push(measure_paged(sizes, &options)?);
    }
    let report = json!({
        "schema_version": 1,
        "benchmark": "canwu-paged-records",
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
            "write_store_records": options.records,
            "write_payload_bytes": options.write_bytes,
            "records_per_filling_boundary": FILL_BATCH,
            "warmup_iterations": options.warmup,
            "write_samples": options.samples,
            "paged_samples": options.paged_samples,
            "fork": "Canwu::fork of the filled run; the fork is dropped outside the measurement",
            "fork_first_boundary": "the first Daily boundary on a fresh fork of the filled run, which replaces one small record",
            "idle_boundary": "one Daily boundary whose system proposes nothing, measured as the writes are",
            "write": "one Daily boundary that replaces one small record with a record whose payload text has the stated bytes, on a fork of the filled run, after one unmeasured boundary on that fork",
            "paged_prepare": "Canwu::prepare_paged_checkpoint into an empty page store, of a run whose store holds exactly the listed records",
            "paged_restore": "Canwu::from_paged_checkpoint_and_journal_with_plugins from a store that holds the prepared pages",
            "allocation_scope": if options.mode == MetricMode::Allocations { "thread-local allocator calls and requested bytes during the operation; setup and post-measurement drops are excluded" } else { "not collected in this uninstrumented build" },
            "elapsed_scope": if options.mode == MetricMode::Elapsed { "wall-clock nanoseconds in a build with the default system allocator; setup and post-measurement drops are excluded" } else { "not collected in the allocation-instrumented build" },
        },
        "writes": writes,
        "paged": paged,
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
    for case in report["writes"].as_array().into_iter().flatten() {
        eprintln!(
            "{:<28} {:>14} {unit}",
            case["name"].as_str().unwrap_or_default(),
            case["summary"][key]
        );
    }
    for set in report["paged"].as_array().into_iter().flatten() {
        if let Some(refused) = set.get("refused") {
            eprintln!(
                "paged {} refused: {}",
                set["payload_bytes"], refused["message"]
            );
            continue;
        }
        for case in set["cases"].as_array().into_iter().flatten() {
            eprintln!(
                "paged {} {:<14} {:>14} {unit} ({} pages, {} bytes)",
                set["payload_bytes"],
                case["name"].as_str().unwrap_or_default(),
                case["summary"][key],
                set["delta_pages"],
                set["delta_page_bytes"]
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
        "crates/runtime/canwu-sim/src",
        "crates/api/canwu-api/src",
        "benchmarks/performance-harness/src/bin/paged-record-profile.rs",
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
        records: 65_536,
        write_bytes: vec![1_024, 8_192, 65_536, 1_048_576],
        paged: vec![vec![900_000], vec![1_200_000, 32_000_000]],
        samples: 31,
        warmup: 3,
        paged_samples: 5,
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
            "--records" => options.records = value()?.parse()?,
            "--write-bytes" => options.write_bytes = counts(&value()?, "--write-bytes")?,
            "--paged" => {
                options.paged = value()?
                    .split(';')
                    .map(|set| counts(set, "--paged"))
                    .collect::<Result<_, _>>()?;
            }
            "--samples" => options.samples = positive_odd(&value()?, "--samples")?,
            "--paged-samples" => {
                options.paged_samples = positive_odd(&value()?, "--paged-samples")?;
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
    if options.records == 0 {
        return Err("--records must be positive".into());
    }
    Ok(options)
}

/// A list of positive counts in the order given.
fn counts(value: &str, flag: &str) -> Result<Vec<usize>, Box<dyn Error>> {
    let parsed = value
        .split(',')
        .map(str::parse)
        .collect::<Result<Vec<usize>, _>>()
        .map_err(|_| format!("{flag} requires comma-separated counts"))?;
    if parsed.is_empty() || parsed.contains(&0) {
        return Err(format!("{flag} requires positive counts").into());
    }
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
        "Canwu paged-record profile\n\n\
         Usage: cargo run --release --manifest-path benchmarks/performance-harness/Cargo.toml \
         --bin paged-record-profile -- [options]\n\n\
         Options:\n\
           --mode MODE             elapsed or allocations\n\
           --records N             Small records in the store the writes measure\n\
           --write-bytes LIST      Comma-separated payload sizes of the measured writes\n\
           --paged SETS            Record sets to page, ';'-separated lists of payload sizes\n\
           --samples N             Positive odd sample count for each write\n\
           --paged-samples N       Positive odd sample count for each paged case\n\
           --warmup N              Unmeasured writes before the samples\n\
           --output PATH           Write the JSON report to PATH\n\
           --machine LABEL         Stable description of the measurement machine\n\
           --recorded-on DATE      Recording date or build label\n\
           --source-label LABEL    What source tree the binary was built from\n"
    );
}
