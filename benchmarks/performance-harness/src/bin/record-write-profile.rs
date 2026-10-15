//! Engine cost of writing domain records.
//!
//! A probe plugin keeps a store of `--store` domain records of one kind and,
//! in one boundary, updates the first N of them with payloads of a given size.
//! The workload separates what the engine does for each written record from
//! what the plugin does to build it, and fits the engine's share as
//! `a + b * kilobytes` per write. It measures, for every payload size in
//! `--sizes` and every write count in `--counts`:
//!
//! - `idle`: a boundary in which the probe system runs and writes nothing;
//! - `update`: a boundary that updates N records whose previous version has
//!   the same size as the new one;
//! - `update_from_small`: the same N records updated from a small previous
//!   version, so that the difference to `update` is what the engine does with
//!   the previous record (journal, boundary hash, copies).
//!
//! Every measured boundary is followed by one that writes nothing: a boundary
//! that writes records emits events that only a later boundary admits, and the
//! evidence can be sealed only after that. The follow-up boundary is timed on
//! its own, because a write costs it too.
//!
//! Each boundary is timed as a whole, and the engine's boundary profile gives
//! the time spent inside the probe's handler, so `engine = total - handler`.
//! In the allocation build the handler's own allocations are subtracted the
//! same way. Elapsed time and allocations come from separate release builds,
//! as in the main harness: `--mode elapsed` without the `allocation-counting`
//! feature, `--mode allocations` with it.
//!
//! Two further measurements need no boundary:
//!
//! - `reproduced_terms`: the per-record encodings the engine performs on a
//!   write, rebuilt here with the same encodings (the record's JSON, the
//!   Patricia leaf page whose canonical bytes carry the record as a JSON
//!   array of byte values, the branch pages on the path, and the record change
//!   with its previous and current record that the boundary hash covers), each
//!   serialized and hashed with BLAKE3. They are not measured inside the
//!   engine; they say how the engine's per-write time divides.
//! - `paged_checkpoint`: whether a run holding one record of
//!   `--paged-record-bytes` can prepare a paged checkpoint, and, by bisection,
//!   the largest record payload that can.
//!
//! The binary uses only the public API. The store is built by the probe plugin
//! in boundaries of `FILL_CHUNK` creations, then the run is compacted and its
//! evidence sealed after every boundary so that retained journals do not grow
//! with the samples.

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundarySystemContract,
    Canwu, CanwuError, CompactedCanwu, DomainRecord, DomainRecordClass, DomainRecordDraft,
    DomainRecordKind, DomainRecordMutation, DomainRecordRef, DomainRecordSchema, ENGINE_VERSION,
    ErrorCode, PluginRegistrar, Scenario, SimDuration, SimTime, SimulationPlugin, SimulationView,
    StateKey, StatePageBlob, StatePageProvider, StateVisibility, SystemCadence,
};
use serde::Serialize;
use serde_json::{Map, Value, json};
#[cfg(feature = "allocation-counting")]
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::{Cell, RefCell};
use std::env;
use std::error::Error;
use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;

const PLUGIN: &str = "profile-record-writes";
const PLUGIN_HASH: &str = "00000000000000000000000000000000000000000000000000000000000f1c01";
const RECORD_NAMESPACE: &str = "profile-record-writes";
const RECORD_KIND: &str = "ledger";
const SYSTEM: &str = "write-records";
const SEED: u64 = 0x5EC0_2D01;
/// Records one filling boundary creates.
const FILL_CHUNK: usize = 8_192;
/// Bytes of one generated ledger entry before its filler, measured once.
const ENTRY_TARGET_BYTES: usize = 350;
/// The domain the engine's primary record map hashes its pages under.
const PRIMARY_DOMAIN: &str = "canwu.format8.domain-record.primary.v1";

/// The probe system's workload, set by the driver before each boundary.
/// Boundary system handlers are plain function pointers, so the handler reads
/// its workload from here.
static FIRST: AtomicUsize = AtomicUsize::new(0);
static COUNT: AtomicUsize = AtomicUsize::new(0);
static CREATE: AtomicBool = AtomicBool::new(false);
static GENERATION: AtomicU64 = AtomicU64::new(0);
/// Whether payloads are one string (`--shape text`) instead of entries.
static TEXT_SHAPE: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// The `entries` object every written payload carries.
    static TEMPLATE: RefCell<Value> = const { RefCell::new(Value::Null) };
    /// Allocation traffic inside the probe handler since the last reset.
    static HANDLER_ALLOCATIONS: Cell<AllocationSample> = const {
        Cell::new(AllocationSample::zero())
    };
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
    const fn zero() -> Self {
        Self {
            alloc_calls: 0,
            realloc_calls: 0,
            dealloc_calls: 0,
            allocated_bytes: 0,
            deallocated_bytes: 0,
        }
    }

    fn operations(self) -> u64 {
        self.alloc_calls.saturating_add(self.realloc_calls)
    }

    fn minus(self, earlier: Self) -> Self {
        Self {
            alloc_calls: self.alloc_calls.saturating_sub(earlier.alloc_calls),
            realloc_calls: self.realloc_calls.saturating_sub(earlier.realloc_calls),
            dealloc_calls: self.dealloc_calls.saturating_sub(earlier.dealloc_calls),
            allocated_bytes: self.allocated_bytes.saturating_sub(earlier.allocated_bytes),
            deallocated_bytes: self
                .deallocated_bytes
                .saturating_sub(earlier.deallocated_bytes),
        }
    }

    fn plus(self, other: Self) -> Self {
        Self {
            alloc_calls: self.alloc_calls.saturating_add(other.alloc_calls),
            realloc_calls: self.realloc_calls.saturating_add(other.realloc_calls),
            dealloc_calls: self.dealloc_calls.saturating_add(other.dealloc_calls),
            allocated_bytes: self.allocated_bytes.saturating_add(other.allocated_bytes),
            deallocated_bytes: self
                .deallocated_bytes
                .saturating_add(other.deallocated_bytes),
        }
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
            sample: AllocationSample::zero(),
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

/// The traffic counted so far in the running sample, without ending it.
#[cfg(feature = "allocation-counting")]
fn peek_allocation_sample() -> AllocationSample {
    ALLOCATION_STATE.with(|state| state.get().sample)
}

#[cfg(not(feature = "allocation-counting"))]
fn start_allocation_sample() {
    unreachable!("allocation mode is rejected before measurement")
}

#[cfg(not(feature = "allocation-counting"))]
fn finish_allocation_sample() -> AllocationSample {
    unreachable!("allocation mode is rejected before measurement")
}

#[cfg(not(feature = "allocation-counting"))]
fn peek_allocation_sample() -> AllocationSample {
    AllocationSample::zero()
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

fn record_kind() -> DomainRecordKind {
    DomainRecordKind::new(RECORD_NAMESPACE, RECORD_KIND)
}

fn record_state() -> StateKey {
    DomainRecordSchema::new(record_kind(), DomainRecordClass::Record).state_key()
}

fn record_reference(ordinal: usize) -> DomainRecordRef {
    DomainRecordRef {
        kind: record_kind(),
        id: format!("record.{ordinal:06}"),
    }
}

/// One generated ledger entry: the mix of short identifiers, numbers, and one
/// longer text that a fiscal book object carries.
fn entry(index: usize, note_bytes: usize) -> Value {
    let mut note = format!("entry {index} note ");
    while note.len() < note_bytes {
        note.push_str("filler-text ");
    }
    note.truncate(note_bytes);
    json!({
        "id": format!("entry.{index:04}"),
        "kind": "assessment",
        "scope": format!("scope.{:06}", index * 7),
        "cycle": "2026-01",
        "unit": "unit.alpha",
        "quantity": 100_000 + index,
        "collected": 40_000 + index,
        "note": note,
    })
}

/// The `entries` value of a payload of about `target` bytes: an object of
/// generated ledger entries, or, with `--shape text`, one string of the same
/// size, which separates the cost of a payload's bytes from that of its JSON
/// tree.
fn entries_for(target: usize) -> Value {
    let envelope = 64;
    if TEXT_SHAPE.load(Ordering::Relaxed) {
        let mut text = String::with_capacity(target);
        while text.len() < target.saturating_sub(envelope).max(ENTRY_TARGET_BYTES) {
            text.push_str("ledger text ");
        }
        text.truncate(target.saturating_sub(envelope).max(ENTRY_TARGET_BYTES));
        return Value::String(text);
    }
    let base = serde_json::to_vec(&entry(0, 0)).map_or(0, |bytes| bytes.len());
    let note_bytes = ENTRY_TARGET_BYTES.saturating_sub(base);
    let count = (target.saturating_sub(envelope) + ENTRY_TARGET_BYTES / 2) / ENTRY_TARGET_BYTES;
    let mut entries = Map::new();
    for index in 0..count.max(1) {
        entries.insert(format!("entry.{index:04}"), entry(index, note_bytes));
    }
    Value::Object(entries)
}

fn payload(entries: &Value, ordinal: usize, generation: u64) -> Value {
    json!({
        "record": format!("record.{ordinal:06}"),
        "generation": generation,
        "entries": entries.clone(),
    })
}

fn set_template(entries: Value) {
    TEMPLATE.with(|template| *template.borrow_mut() = entries);
}

fn configure(first: usize, count: usize, create: bool) {
    FIRST.store(first, Ordering::Relaxed);
    COUNT.store(count, Ordering::Relaxed);
    CREATE.store(create, Ordering::Relaxed);
}

/// Creates or updates the configured records, each with a fresh payload.
fn write_records(
    view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let before = peek_allocation_sample();
    let first = FIRST.load(Ordering::Relaxed);
    let count = COUNT.load(Ordering::Relaxed);
    let create = CREATE.load(Ordering::Relaxed);
    let generation = GENERATION.fetch_add(1, Ordering::Relaxed);
    let directives = TEMPLATE.with(|template| {
        let template = template.borrow();
        (first..first + count)
            .map(|ordinal| {
                let reference = record_reference(ordinal);
                let record = DomainRecordDraft::new(
                    reference.clone(),
                    payload(&template, ordinal, generation),
                );
                let mutation = if create {
                    DomainRecordMutation::Create { record }
                } else {
                    let expected_version = view
                        .domain_record(&reference)?
                        .map(|current| current.version)
                        .ok_or_else(|| {
                            CanwuError::new(
                                ErrorCode::DomainRecordNotFound,
                                "probe record is missing",
                            )
                        })?;
                    DomainRecordMutation::Update {
                        record,
                        expected_version,
                    }
                };
                Ok(BoundaryDirective::MutateRecord {
                    mutation,
                    summary: "probe write".to_owned(),
                })
            })
            .collect::<Result<Vec<_>, CanwuError>>()
    })?;
    let spent = peek_allocation_sample().minus(before);
    HANDLER_ALLOCATIONS.with(|total| total.set(total.get().plus(spent)));
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

struct RecordWritePlugin;

impl SimulationPlugin for RecordWritePlugin {
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
        registrar.register_record_schema(DomainRecordSchema::new(
            record_kind(),
            DomainRecordClass::Record,
        ))?;
        let mut contract = BoundarySystemContract::new(
            SYSTEM,
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        contract.reads = vec![record_state()];
        contract.writes = vec![record_state()];
        contract.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(contract, write_records)
    }
}

fn next_minute(now: SimTime) -> Result<SimTime, Box<dyn Error>> {
    now.checked_add(SimDuration::minutes(1))
        .ok_or_else(|| "profile time overflow".into())
}

/// Builds a run whose store holds `store` records with payloads of the current
/// template, and returns it compacted with its evidence sealed.
fn build(store: usize) -> Result<CompactedCanwu, Box<dyn Error>> {
    let plugin = RecordWritePlugin;
    let plugins: [&dyn SimulationPlugin; 1] = [&plugin];
    let mut canwu =
        Canwu::new_with_plugins(SEED, Scenario::new(SimTime::EPOCH, Vec::new()), &plugins)?;
    let mut first = 0;
    while first < store {
        let count = FILL_CHUNK.min(store - first);
        configure(first, count, true);
        let at = next_minute(canwu.time())?;
        canwu.schedule_calendar_boundary(at, vec![SystemCadence::Daily])?;
        canwu
            .step_canonical()?
            .ok_or("the filling boundary did not settle")?;
        first += count;
    }
    // A boundary that writes records leaves events that only a later
    // boundary admits, and evidence is sealable only once they are admitted.
    configure(0, 0, false);
    let at = next_minute(canwu.time())?;
    canwu.schedule_calendar_boundary(at, vec![SystemCadence::Daily])?;
    canwu
        .step_canonical()?
        .ok_or("the closing boundary did not settle")?;
    let mut compacted = canwu.into_compacted()?;
    drop(compacted.seal_evidence()?);
    Ok(compacted)
}

/// Settles one Daily boundary of the probe system, then a boundary that
/// writes nothing and admits what the first left, unmeasured, and seals.
fn settle_unmeasured(compacted: &mut CompactedCanwu) -> Result<(), Box<dyn Error>> {
    let at = next_minute(compacted.time())?;
    compacted.schedule_calendar_boundary(at, vec![SystemCadence::Daily])?;
    compacted
        .step_canonical()?
        .ok_or("the unmeasured boundary did not settle")?;
    configure(0, 0, false);
    let at = next_minute(compacted.time())?;
    compacted.schedule_calendar_boundary(at, vec![SystemCadence::Daily])?;
    compacted
        .step_canonical()?
        .ok_or("the follow-up boundary did not settle")?;
    drop(compacted.seal_evidence()?);
    Ok(())
}

/// One measured boundary, and the boundary after it, which writes nothing and
/// admits the events the measured boundary emitted.
#[derive(Clone, Copy, Debug, Default)]
struct BoundarySample {
    total_ns: u64,
    handler_us: u64,
    profile_total_us: u64,
    allocations: AllocationSample,
    handler_allocations: AllocationSample,
    follow_up_ns: u64,
    follow_up_allocations: AllocationSample,
}

impl BoundarySample {
    /// Wall time outside the probe handler, in microseconds.
    fn engine_us(self) -> f64 {
        self.profile_total_us.saturating_sub(self.handler_us) as f64
    }

    fn engine_allocations(self) -> AllocationSample {
        self.allocations.minus(self.handler_allocations)
    }
}

/// Settles one measured Daily boundary and seals afterwards.
fn settle_measured(
    compacted: &mut CompactedCanwu,
    mode: MetricMode,
) -> Result<BoundarySample, Box<dyn Error>> {
    let at = next_minute(compacted.time())?;
    compacted.schedule_calendar_boundary(at, vec![SystemCadence::Daily])?;
    compacted.set_boundary_profiling(true);
    HANDLER_ALLOCATIONS.with(|total| total.set(AllocationSample::zero()));
    let mut sample = BoundarySample::default();
    let receipt = match mode {
        MetricMode::Elapsed => {
            let started = Instant::now();
            let receipt = compacted.step_canonical();
            sample.total_ns = u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX);
            receipt
        }
        MetricMode::Allocations => {
            start_allocation_sample();
            let receipt = compacted.step_canonical();
            sample.allocations = finish_allocation_sample();
            receipt
        }
    }?;
    black_box(receipt.ok_or("the measured boundary did not settle")?);
    sample.handler_allocations = HANDLER_ALLOCATIONS.with(Cell::get);
    let batch = compacted.take_boundary_profiles();
    compacted.set_boundary_profiling(false);
    let profile = batch
        .profiles
        .last()
        .ok_or("the measured boundary has no profile")?;
    sample.handler_us = profile.plugin_micros(PLUGIN);
    sample.profile_total_us = profile.total_micros;
    // The next boundary admits the events the writes emitted; it is part of
    // what a write costs, so it is measured on its own.
    configure(0, 0, false);
    let at = next_minute(compacted.time())?;
    compacted.schedule_calendar_boundary(at, vec![SystemCadence::Daily])?;
    let receipt = match mode {
        MetricMode::Elapsed => {
            let started = Instant::now();
            let receipt = compacted.step_canonical();
            sample.follow_up_ns = u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX);
            receipt
        }
        MetricMode::Allocations => {
            start_allocation_sample();
            let receipt = compacted.step_canonical();
            sample.follow_up_allocations = finish_allocation_sample();
            receipt
        }
    }?;
    black_box(receipt.ok_or("the follow-up boundary did not settle")?);
    drop(compacted.seal_evidence()?);
    Ok(sample)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Variant {
    Idle,
    Update,
    UpdateFromSmall,
}

impl Variant {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Update => "update",
            Self::UpdateFromSmall => "update_from_small",
        }
    }
}

/// Measures one case on `compacted`: `warmup` unmeasured and `samples`
/// measured boundaries that write the first `count` records.
fn measure_case(
    compacted: &mut CompactedCanwu,
    variant: Variant,
    count: usize,
    entries: &Value,
    small: &Value,
    options: &Options,
) -> Result<Vec<BoundarySample>, Box<dyn Error>> {
    let mut samples = Vec::with_capacity(options.samples);
    for sample in 0..options.warmup + options.samples {
        if variant == Variant::UpdateFromSmall {
            set_template(small.clone());
            configure(0, count, false);
            settle_unmeasured(compacted)?;
        }
        set_template(entries.clone());
        configure(0, if variant == Variant::Idle { 0 } else { count }, false);
        let measured = settle_measured(compacted, options.mode)?;
        if sample >= options.warmup {
            samples.push(measured);
        }
    }
    Ok(samples)
}

fn median_f64(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values.get(values.len() / 2).copied().unwrap_or_default()
}

fn median_u64(values: &mut [u64]) -> u64 {
    values.sort_unstable();
    values.get(values.len() / 2).copied().unwrap_or_default()
}

/// The median engine figures of a case: microseconds, allocation operations,
/// and allocated bytes.
#[derive(Clone, Copy, Debug)]
struct CaseMedians {
    engine_us: f64,
    total_us: f64,
    handler_us: f64,
    engine_alloc_operations: f64,
    engine_allocated_bytes: f64,
    follow_up_us: f64,
    follow_up_alloc_operations: f64,
    follow_up_allocated_bytes: f64,
}

fn case_medians(samples: &[BoundarySample]) -> CaseMedians {
    let mut follow_up: Vec<f64> = samples
        .iter()
        .map(|sample| sample.follow_up_ns as f64 / 1_000.0)
        .collect();
    let mut follow_up_operations: Vec<u64> = samples
        .iter()
        .map(|sample| sample.follow_up_allocations.operations())
        .collect();
    let mut follow_up_bytes: Vec<u64> = samples
        .iter()
        .map(|sample| sample.follow_up_allocations.allocated_bytes)
        .collect();
    let mut engine: Vec<f64> = samples.iter().map(|sample| sample.engine_us()).collect();
    let mut total: Vec<f64> = samples
        .iter()
        .map(|sample| sample.total_ns as f64 / 1_000.0)
        .collect();
    let mut handler: Vec<f64> = samples
        .iter()
        .map(|sample| sample.handler_us as f64)
        .collect();
    let mut operations: Vec<u64> = samples
        .iter()
        .map(|sample| sample.engine_allocations().operations())
        .collect();
    let mut bytes: Vec<u64> = samples
        .iter()
        .map(|sample| sample.engine_allocations().allocated_bytes)
        .collect();
    CaseMedians {
        engine_us: median_f64(&mut engine),
        total_us: median_f64(&mut total),
        handler_us: median_f64(&mut handler),
        engine_alloc_operations: median_u64(&mut operations) as f64,
        engine_allocated_bytes: median_u64(&mut bytes) as f64,
        follow_up_us: median_f64(&mut follow_up),
        follow_up_alloc_operations: median_u64(&mut follow_up_operations) as f64,
        follow_up_allocated_bytes: median_u64(&mut follow_up_bytes) as f64,
    }
}

fn case_json(
    variant: Variant,
    count: usize,
    samples: &[BoundarySample],
    mode: MetricMode,
) -> Value {
    let medians = case_medians(samples);
    let mut case = json!({
        "variant": variant.as_str(),
        "writes": count,
    });
    match mode {
        MetricMode::Elapsed => {
            case["summary"] = json!({
                "boundary_us_median": round(medians.total_us),
                "engine_us_median": round(medians.engine_us),
                "handler_us_median": round(medians.handler_us),
                "engine_us_per_write": if count == 0 { Value::Null } else { json!(round(medians.engine_us / count as f64)) },
                "follow_up_boundary_us_median": round(medians.follow_up_us),
            });
            case["samples"] = samples
                .iter()
                .map(|sample| {
                    json!({
                        "boundary_ns": sample.total_ns,
                        "profile_total_us": sample.profile_total_us,
                        "handler_us": sample.handler_us,
                        "follow_up_boundary_ns": sample.follow_up_ns,
                    })
                })
                .collect();
        }
        MetricMode::Allocations => {
            case["summary"] = json!({
                "engine_allocation_operations_median": medians.engine_alloc_operations,
                "engine_allocated_bytes_median": medians.engine_allocated_bytes,
                "follow_up_allocation_operations_median": medians.follow_up_alloc_operations,
                "follow_up_allocated_bytes_median": medians.follow_up_allocated_bytes,
            });
            case["samples"] = samples
                .iter()
                .map(|sample| {
                    json!({
                        "allocation_operations": sample.allocations.operations(),
                        "allocated_bytes": sample.allocations.allocated_bytes,
                        "deallocated_bytes": sample.allocations.deallocated_bytes,
                        "handler_allocation_operations": sample.handler_allocations.operations(),
                        "handler_allocated_bytes": sample.handler_allocations.allocated_bytes,
                        "follow_up_allocation_operations": sample.follow_up_allocations.operations(),
                        "follow_up_allocated_bytes": sample.follow_up_allocations.allocated_bytes,
                    })
                })
                .collect();
        }
    }
    case
}

/// Least-squares line through `points`: (intercept, slope).
fn fit(points: &[(f64, f64)]) -> (f64, f64) {
    let count = points.len() as f64;
    if points.len() < 2 {
        return (points.first().map_or(0.0, |point| point.1), 0.0);
    }
    let mean_x = points.iter().map(|point| point.0).sum::<f64>() / count;
    let mean_y = points.iter().map(|point| point.1).sum::<f64>() / count;
    let covariance: f64 = points
        .iter()
        .map(|point| (point.0 - mean_x) * (point.1 - mean_y))
        .sum();
    let variance: f64 = points.iter().map(|point| (point.0 - mean_x).powi(2)).sum();
    let slope = if variance == 0.0 {
        0.0
    } else {
        covariance / variance
    };
    (mean_y - slope * mean_x, slope)
}

fn round(value: f64) -> f64 {
    (value * 1_000.0).round() / 1_000.0
}

/// Serialized size of the first record of the store: payload and whole record.
fn record_sizes(compacted: &CompactedCanwu) -> Result<(usize, usize), Box<dyn Error>> {
    let record = compacted
        .domain_record(&record_reference(0))
        .ok_or("the store has no first record")?;
    Ok((
        serde_json::to_vec(&record.payload)?.len(),
        serde_json::to_vec(record)?.len(),
    ))
}

fn blake3_hex(domain: &str, bytes: &[u8]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain.as_bytes());
    hasher.update(&[0]);
    hasher.update(bytes);
    hasher.finalize().to_hex().to_string()
}

/// A Patricia page as the engine encodes it for its page identity.
#[derive(Serialize)]
#[serde(tag = "node", rename_all = "snake_case")]
enum ReproducedPage<'a> {
    Leaf {
        domain: &'a str,
        key_hash: String,
        entries: Vec<ReproducedEntry>,
        entry_count: u64,
        structural_bytes: u64,
    },
    Branch {
        domain: &'a str,
        bit: u16,
        left_page: String,
        right_page: String,
        entry_count: u64,
        structural_bytes: u64,
    },
}

#[derive(Serialize)]
struct ReproducedEntry {
    key_hash: String,
    key_bytes: Vec<u8>,
    value_hash: String,
    value_bytes: Vec<u8>,
}

/// A record change as the boundary hash covers it.
#[derive(Serialize)]
struct ReproducedChange<'a> {
    plugin: &'a str,
    system: &'a str,
    operation: &'a str,
    previous: &'a DomainRecord,
    current: &'a DomainRecord,
    visibility: &'a str,
    summary: &'a str,
}

/// Times, per written record, the encodings the engine performs on a write,
/// rebuilt with the same encodings: nanoseconds for each term, and the leaf
/// page's size against the record's.
fn reproduced_terms(record: &DomainRecord, depth: usize) -> Result<Value, Box<dyn Error>> {
    const ROUNDS: u32 = 200;
    let key_bytes = serde_json::to_vec(&record.reference)?;
    let key_hash = blake3_hex(&format!("{PRIMARY_DOMAIN}.key"), &key_bytes);
    let mut record_encode = 0_u128;
    let mut value_hash = 0_u128;
    let mut leaf_page = 0_u128;
    let mut branch_path = 0_u128;
    let mut change_hash = 0_u128;
    let mut record_clone_drop = 0_u128;
    let mut leaf_page_bytes = 0;
    let mut record_bytes = 0;
    for _ in 0..ROUNDS {
        let started = Instant::now();
        let value_bytes = black_box(serde_json::to_vec(record)?);
        record_encode += started.elapsed().as_nanos();
        record_bytes = value_bytes.len();

        let started = Instant::now();
        let hash = black_box(blake3_hex(&format!("{PRIMARY_DOMAIN}.value"), &value_bytes));
        value_hash += started.elapsed().as_nanos();

        let started = Instant::now();
        let page = ReproducedPage::Leaf {
            domain: PRIMARY_DOMAIN,
            key_hash: key_hash.clone(),
            entries: vec![ReproducedEntry {
                key_hash: key_hash.clone(),
                key_bytes: key_bytes.clone(),
                value_hash: hash,
                value_bytes: value_bytes.clone(),
            }],
            entry_count: 1,
            structural_bytes: 72,
        };
        let page_bytes = serde_json::to_vec(&page)?;
        black_box(blake3_hex("canwu.state-page.v1", &page_bytes));
        leaf_page += started.elapsed().as_nanos();
        leaf_page_bytes = page_bytes.len();

        let started = Instant::now();
        for bit in 0..depth {
            let page = ReproducedPage::Branch {
                domain: PRIMARY_DOMAIN,
                bit: u16::try_from(bit)?,
                left_page: key_hash.clone(),
                right_page: key_hash.clone(),
                entry_count: 65_536,
                structural_bytes: 4_194_304,
            };
            let page_bytes = serde_json::to_vec(&page)?;
            black_box(blake3_hex("canwu.state-page.v1", &page_bytes));
        }
        branch_path += started.elapsed().as_nanos();

        let started = Instant::now();
        let change = ReproducedChange {
            plugin: PLUGIN,
            system: SYSTEM,
            operation: "updated",
            previous: record,
            current: record,
            visibility: "same_boundary",
            summary: "probe write",
        };
        black_box(blake3_hex(
            "canwu.boundary-record.v1",
            &serde_json::to_vec(&change)?,
        ));
        change_hash += started.elapsed().as_nanos();

        let started = Instant::now();
        drop(black_box(record.clone()));
        record_clone_drop += started.elapsed().as_nanos();
    }
    let per_write = |total: u128| round(total as f64 / f64::from(ROUNDS));
    Ok(json!({
        "record_bytes": record_bytes,
        "leaf_page_bytes": leaf_page_bytes,
        "leaf_page_bytes_per_record_byte": round(leaf_page_bytes as f64 / record_bytes as f64),
        "branch_pages_on_path": depth,
        "ns_per_write": {
            "record_encode": per_write(record_encode),
            "value_hash": per_write(value_hash),
            "leaf_page_encode_and_hash": per_write(leaf_page),
            "branch_path_encode_and_hash": per_write(branch_path),
            "change_encode_and_hash_previous_and_current": per_write(change_hash),
            "record_clone_and_drop": per_write(record_clone_drop),
        },
    }))
}

/// A page provider that holds nothing, so a paged checkpoint emits every page.
struct NoPages;

impl StatePageProvider for NoPages {
    fn load_state_page(&self, _page_id: &str) -> Result<Option<StatePageBlob>, CanwuError> {
        Ok(None)
    }
}

/// Builds a run with one record of about `target` payload bytes and prepares
/// a paged checkpoint of it. Returns the record's payload bytes and the
/// outcome.
fn paged_checkpoint_of(target: usize) -> Result<(usize, Result<(), String>), Box<dyn Error>> {
    set_template(entries_for(target));
    let plugin = RecordWritePlugin;
    let plugins: [&dyn SimulationPlugin; 1] = [&plugin];
    let mut canwu =
        Canwu::new_with_plugins(SEED, Scenario::new(SimTime::EPOCH, Vec::new()), &plugins)?;
    configure(0, 1, true);
    let at = next_minute(canwu.time())?;
    canwu.schedule_calendar_boundary(at, vec![SystemCadence::Daily])?;
    canwu
        .step_canonical()?
        .ok_or("the record boundary did not settle")?;
    let payload_bytes = serde_json::to_vec(
        &canwu
            .domain_record(&record_reference(0))
            .ok_or("the record was not created")?
            .payload,
    )?
    .len();
    let outcome = canwu
        .prepare_paged_checkpoint(None, &NoPages)
        .map(|_| ())
        .map_err(|error| error.to_string());
    Ok((payload_bytes, outcome))
}

fn paged_checkpoint(options: &Options) -> Result<Value, Box<dyn Error>> {
    let (requested_bytes, requested) = paged_checkpoint_of(options.paged_record_bytes)?;
    // Bisect on the requested payload size between a size that fits and one
    // that does not.
    let mut fits = 64 * 1_024;
    let mut fails = 4 * 1_024 * 1_024;
    let (_, low) = paged_checkpoint_of(fits)?;
    let (_, high) = paged_checkpoint_of(fails)?;
    let bracket = low.is_ok() && high.is_err();
    let mut largest_fitting = None;
    let mut smallest_failing = None;
    if bracket {
        while fails - fits > 2 * ENTRY_TARGET_BYTES {
            let middle = fits + (fails - fits) / 2;
            let (bytes, outcome) = paged_checkpoint_of(middle)?;
            if outcome.is_ok() {
                fits = middle;
                largest_fitting = Some(bytes);
            } else {
                fails = middle;
                smallest_failing = Some(bytes);
            }
        }
    }
    Ok(json!({
        "requested_payload_bytes": options.paged_record_bytes,
        "record_payload_bytes": requested_bytes,
        "prepared": requested.is_ok(),
        "error": requested.err(),
        "bisection": {
            "bracketed": bracket,
            "largest_fitting_payload_bytes": largest_fitting,
            "smallest_failing_payload_bytes": smallest_failing,
        },
    }))
}

struct Options {
    mode: MetricMode,
    sizes: Vec<usize>,
    counts: Vec<usize>,
    store: usize,
    samples: usize,
    warmup: usize,
    paged_record_bytes: usize,
    text_shape: bool,
    output: Option<PathBuf>,
    machine: String,
    recorded_on: String,
    source_label: String,
}

fn main() -> Result<(), Box<dyn Error>> {
    let options = options()?;
    if cfg!(debug_assertions) {
        return Err("run the record-write profile with --release".into());
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
    TEXT_SHAPE.store(options.text_shape, Ordering::Relaxed);
    let depth = usize::try_from(options.store.max(2).ilog2())? + 1;
    let small = entries_for(0);

    // The engine floor: an idle boundary with an empty store.
    set_template(small.clone());
    let mut empty = build(0)?;
    let empty_idle = measure_case(&mut empty, Variant::Idle, 0, &small, &small, &options)?;
    drop(empty);
    eprintln!(
        "empty store idle: {:.1} us engine",
        case_medians(&empty_idle).engine_us
    );

    let mut sizes = Vec::new();
    let mut per_write_points = Vec::new();
    let mut previous_points = Vec::new();
    let mut operation_points = Vec::new();
    let mut byte_points = Vec::new();
    let mut follow_up_points = Vec::new();
    let mut follow_up_operation_points = Vec::new();
    for &size in &options.sizes {
        let entries = entries_for(size);
        set_template(entries.clone());
        let mut compacted = build(options.store)?;
        let (payload_bytes, record_bytes) = record_sizes(&compacted)?;
        let record_kib = record_bytes as f64 / 1_024.0;
        eprintln!("size {size}: payload {payload_bytes} B, record {record_bytes} B");
        let mut cases = vec![case_json(
            Variant::Idle,
            0,
            &measure_case(&mut compacted, Variant::Idle, 0, &entries, &small, &options)?,
            options.mode,
        )];
        let mut fits = serde_json::Map::new();
        for variant in [Variant::Update, Variant::UpdateFromSmall] {
            let mut time_points = Vec::new();
            let mut operations = Vec::new();
            let mut bytes = Vec::new();
            let mut follow_up_time = Vec::new();
            let mut follow_up_operations = Vec::new();
            let mut follow_up_bytes = Vec::new();
            for &count in &options.counts {
                let samples =
                    measure_case(&mut compacted, variant, count, &entries, &small, &options)?;
                let medians = case_medians(&samples);
                eprintln!(
                    "size {size} {:<17} {count:>6} writes: engine {:>12.1} us, handler {:>10.1} us, follow-up {:>10.1} us",
                    variant.as_str(),
                    medians.engine_us,
                    medians.handler_us,
                    medians.follow_up_us
                );
                let writes = count as f64;
                time_points.push((writes, medians.engine_us));
                operations.push((writes, medians.engine_alloc_operations));
                bytes.push((writes, medians.engine_allocated_bytes));
                follow_up_time.push((writes, medians.follow_up_us));
                follow_up_operations.push((writes, medians.follow_up_alloc_operations));
                follow_up_bytes.push((writes, medians.follow_up_allocated_bytes));
                cases.push(case_json(variant, count, &samples, options.mode));
            }
            let (time_intercept, time_slope) = fit(&time_points);
            let (_, operation_slope) = fit(&operations);
            let (_, byte_slope) = fit(&bytes);
            let (_, follow_up_slope) = fit(&follow_up_time);
            let (_, follow_up_operation_slope) = fit(&follow_up_operations);
            let (_, follow_up_byte_slope) = fit(&follow_up_bytes);
            match variant {
                Variant::Update => {
                    per_write_points.push((record_kib, time_slope));
                    operation_points.push((record_kib, operation_slope));
                    byte_points.push((record_kib, byte_slope));
                    follow_up_points.push((record_kib, follow_up_slope));
                    follow_up_operation_points.push((record_kib, follow_up_operation_slope));
                }
                Variant::UpdateFromSmall => previous_points.push((record_kib, time_slope)),
                Variant::Idle => {}
            }
            fits.insert(
                variant.as_str().to_owned(),
                match options.mode {
                    MetricMode::Elapsed => json!({
                        "engine_us_per_write": round(time_slope),
                        "engine_us_per_boundary": round(time_intercept),
                        "follow_up_us_per_write": round(follow_up_slope),
                    }),
                    MetricMode::Allocations => json!({
                        "engine_allocation_operations_per_write": round(operation_slope),
                        "engine_allocated_bytes_per_write": round(byte_slope),
                        "follow_up_allocation_operations_per_write": round(follow_up_operation_slope),
                        "follow_up_allocated_bytes_per_write": round(follow_up_byte_slope),
                    }),
                },
            );
        }
        let mut scale = json!({
            "target_payload_bytes": size,
            "payload_bytes": payload_bytes,
            "record_bytes": record_bytes,
            "store_records": options.store,
            "cases": cases,
            "fit_over_writes": fits,
        });
        if options.mode == MetricMode::Elapsed {
            let record = compacted
                .domain_record(&record_reference(0))
                .ok_or("the store has no first record")?
                .clone();
            scale["reproduced_terms"] = reproduced_terms(&record, depth)?;
        }
        sizes.push(scale);
        drop(compacted);
    }

    let fit_over_bytes = match options.mode {
        MetricMode::Elapsed => {
            let (a, b) = fit(&per_write_points);
            let previous: Vec<_> = previous_points
                .iter()
                .zip(&per_write_points)
                .map(|(small, same)| (same.0, same.1 - small.1))
                .collect();
            let (previous_a, previous_b) = fit(&previous);
            let (follow_up_a, follow_up_b) = fit(&follow_up_points);
            json!({
                "update_engine_us_per_write": { "a_us": round(a), "b_us_per_kib": round(b) },
                "previous_record_term_us_per_write": { "a_us": round(previous_a), "b_us_per_kib": round(previous_b) },
                "follow_up_boundary_us_per_write": { "a_us": round(follow_up_a), "b_us_per_kib": round(follow_up_b) },
                "points_kib_us": per_write_points.iter().map(|(x, y)| json!([round(*x), round(*y)])).collect::<Vec<_>>(),
            })
        }
        MetricMode::Allocations => {
            let (operations_a, operations_b) = fit(&operation_points);
            let (bytes_a, bytes_b) = fit(&byte_points);
            let (follow_up_a, follow_up_b) = fit(&follow_up_operation_points);
            json!({
                "update_engine_allocation_operations_per_write": { "a": round(operations_a), "b_per_kib": round(operations_b) },
                "update_engine_allocated_bytes_per_write": { "a": round(bytes_a), "b_per_kib": round(bytes_b) },
                "follow_up_allocation_operations_per_write": { "a": round(follow_up_a), "b_per_kib": round(follow_up_b) },
            })
        }
    };
    let paged = if options.mode == MetricMode::Elapsed {
        paged_checkpoint(&options)?
    } else {
        Value::Null
    };
    let report = json!({
        "schema_version": 1,
        "benchmark": "canwu-record-write-cost",
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
            "store_records": options.store,
            "target_payload_bytes": options.sizes,
            "writes_per_boundary": options.counts,
            "warmup_iterations": options.warmup,
            "boundary_samples": options.samples,
            "payload_shape": if options.text_shape { "text" } else { "entries" },
            "payload": "entries: an object of generated ledger entries of about 350 bytes each (identifiers, numbers, one note); text: one string of the same size; either with the record's ordinal and a generation that changes every write",
            "boundary": "one Daily boundary on the compacted run in which the probe system proposes N record updates; the evidence is sealed after every boundary",
            "engine_share": "boundary profile total minus the probe handler's own time (elapsed), or the boundary's allocations minus those made inside the handler (allocations)",
            "update_from_small": "before each sample an unmeasured boundary rewrites the N records with a one-entry payload, so the measured update has a small previous version",
            "follow_up": "the boundary after each measured one writes nothing and admits the events the writes emitted; it is timed on its own and fitted per write",
            "fit_over_writes": "least squares of the engine share against N for each size: slope per write, intercept per boundary",
            "fit_over_bytes": "least squares of the per-write slopes against the record's serialized KiB",
            "reproduced_terms": "elapsed build only: the engine's per-write encodings rebuilt in the harness with the same encodings and timed outside the engine",
            "paged_checkpoint": "elapsed build only: prepare_paged_checkpoint of a run holding one record, against a provider that holds no page",
            "allocation_scope": if options.mode == MetricMode::Allocations { "thread-local allocator calls and requested bytes during the boundary; setup, sealing, and post-measurement drops are excluded" } else { "not collected in this uninstrumented build" },
            "elapsed_scope": if options.mode == MetricMode::Elapsed { "wall-clock nanoseconds in a build with the default system allocator; setup and sealing are excluded" } else { "not collected in the allocation-instrumented build" },
        },
        "empty_store_idle": case_json(Variant::Idle, 0, &empty_idle, options.mode),
        "sizes": sizes,
        "fit_over_bytes": fit_over_bytes,
        "paged_checkpoint": paged,
    });
    let encoded = serde_json::to_string_pretty(&report)?;
    eprintln!("fit over bytes: {}", report["fit_over_bytes"]);
    eprintln!("paged checkpoint: {}", report["paged_checkpoint"]);
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

/// Asks the toolchain that built this binary for its version, from the
/// binary's own manifest directory so the pinned toolchain answers.
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
        "benchmarks/performance-harness/src/bin/record-write-profile.rs",
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
        sizes: vec![1_024, 2_048, 4_096, 8_192],
        counts: vec![1_000, 10_000, 50_000],
        store: 65_536,
        samples: 5,
        warmup: 1,
        paged_record_bytes: 1_200_000,
        text_shape: false,
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
            "--sizes" => options.sizes = counts(&value()?, "--sizes")?,
            "--counts" => options.counts = counts(&value()?, "--counts")?,
            "--store" => options.store = value()?.parse()?,
            "--samples" => options.samples = positive_odd(&value()?, "--samples")?,
            "--warmup" => options.warmup = value()?.parse()?,
            "--paged-record-bytes" => options.paged_record_bytes = value()?.parse()?,
            "--shape" => {
                options.text_shape = match value()?.as_str() {
                    "entries" => false,
                    "text" => true,
                    other => return Err(format!("unsupported payload shape: {other}").into()),
                };
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
    if options.counts.iter().any(|count| *count > options.store) {
        return Err("every write count must fit the store".into());
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
        "Canwu record-write cost profile\n\n\
         Usage: cargo run --release --manifest-path benchmarks/performance-harness/Cargo.toml \
         --bin record-write-profile -- [options]\n\n\
         Options:\n\
           --mode MODE               elapsed or allocations\n\
           --sizes LIST              Comma-separated target payload bytes\n\
           --counts LIST             Comma-separated records written per boundary\n\
           --store N                 Records in the store\n\
           --samples N               Positive odd sample count for each case\n\
           --warmup N                Unmeasured boundaries before the samples\n\
           --paged-record-bytes N    Payload bytes of the paged-checkpoint record\n\
           --shape SHAPE             entries (default) or text\n\
           --output PATH             Write the JSON report to PATH\n\
           --machine LABEL           Stable description of the measurement machine\n\
           --recorded-on DATE        Recording date or build label\n\
           --source-label LABEL      What source tree the binary was built from\n"
    );
}
