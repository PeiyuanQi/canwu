//! Resource root profile: what one boundary costs the resource plugin as the
//! hot terminal backlog of its root grows.
//!
//! Each case installs a resource root that holds `N` hot terminal records
//! (rejected operation outcomes of about 1.1 KB, each its own archive
//! candidate, placed in the root directly so that building a large backlog
//! does not pay one whole-root validation per record) and then times three
//! kinds of boundary through the public API:
//!
//! - `operation`: one requester-scoped allocation pass settled through the
//!   resource plugin. The requester owns no demand, so the pass is a durable
//!   rejection: one operation, one new terminal outcome.
//! - `batch`: eight such passes settled in one boundary.
//! - `idle`: a boundary that admits one packet of a tick plugin and nothing
//!   for the resource plugin, so every event-driven system of the resource
//!   plugin runs with nothing due.
//! - `archive_commit`: after the samples, the boundary that applies one
//!   archive commit of every hot terminal record (one sample).
//!
//! Only `settle_boundary` is measured. The evidence that boundary appends to
//! the journal is reported as encoded JSON bytes. `--mode allocations`, in a
//! build with the `allocation-counting` feature, counts allocator calls and
//! requested bytes instead of timing.

use canwu_api::{
    BoundaryRequest, Canwu, CanwuError, DomainRecord, EntityRef, IngressClass, KnowledgeHolderRef,
    PayloadSchema, PersonId, PluginIngressDescriptor, PluginIngressRequest, PluginRegistrar,
    Scenario, SimTime, SimulationPlugin,
};
use canwu_resource::{
    ResourceAllocationRequestV1, ResourceArchiveRetentionHandleV1, ResourceArchiveRetentionPhaseV1,
    ResourceArchiveStore, ResourceCancelDemandRequestV1, ResourceDemandId, ResourceError,
    ResourceLimitsV1, ResourceOperationKey, ResourceOperationOutcome, ResourceOperationOutcomeId,
    ResourceOperationRequestV1, ResourcePlugin, ResourceRevision, ResourceState,
    ResourceTerminalRecordKeyV1, canonical_digest, enqueue_resource_allocation,
    enqueue_resource_archive, resource_state,
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
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

const TICK_PLUGIN: &str = "resource-root-profile-tick";
const TICK_INGRESS: &str = "tick_v1";
const PROFILE_SEED: u64 = 0x00CA_4E55;
const BATCH: usize = 8;

/// Owns one ingress type and no system: a tick makes a boundary event-bearing,
/// so the event-driven systems of every plugin run, with nothing due for the
/// resource plugin.
struct TickPlugin;

impl SimulationPlugin for TickPlugin {
    fn name(&self) -> &'static str {
        TICK_PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "00000000000000000000000000000000000000000000000000000000000000f2"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_ingress(PluginIngressDescriptor {
            name: TICK_INGRESS.to_owned(),
            description: "Make one boundary event-bearing".to_owned(),
            class: IngressClass::Information,
            payload_schema: PayloadSchema::Any,
        })
    }
}

/// Stored archive objects by namespace and object id.
type ArchiveObjects = BTreeMap<(String, String), Vec<u8>>;

/// An in-memory archive store, also served to the runtime as its archive
/// object provider.
#[derive(Clone, Default)]
struct MemoryArchive {
    objects: Rc<RefCell<ArchiveObjects>>,
}

impl ResourceArchiveStore for MemoryArchive {
    fn store_resource_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
        bytes: &[u8],
    ) -> Result<(), ResourceError> {
        self.objects
            .borrow_mut()
            .insert((namespace.to_owned(), object_id.to_owned()), bytes.to_vec());
        Ok(())
    }

    fn load_resource_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, ResourceError> {
        Ok(self
            .objects
            .borrow()
            .get(&(namespace.to_owned(), object_id.to_owned()))
            .cloned())
    }

    fn persist_resource_archive_retention(
        &self,
        _handle: &ResourceArchiveRetentionHandleV1,
    ) -> Result<(), ResourceError> {
        Ok(())
    }

    fn finalize_resource_archive_retention(
        &self,
        _handle_id: &str,
        _phase: ResourceArchiveRetentionPhaseV1,
    ) -> Result<(), ResourceError> {
        Ok(())
    }
}

impl canwu_api::PluginArchiveObjectProvider for MemoryArchive {
    fn load_plugin_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, CanwuError> {
        Ok(self
            .objects
            .borrow()
            .get(&(namespace.to_owned(), object_id.to_owned()))
            .cloned())
    }
}

#[cfg(feature = "allocation-counting")]
struct CountingAllocator;

#[cfg(feature = "allocation-counting")]
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[cfg(feature = "allocation-counting")]
thread_local! {
    /// Whether this thread counts, the allocator calls, and the bytes requested.
    static ALLOCATIONS: Cell<(bool, u64, u64)> = const { Cell::new((false, 0, 0)) };
}

#[cfg(feature = "allocation-counting")]
fn count_allocation(bytes: usize) {
    let _ = ALLOCATIONS.try_with(|state| {
        let (counting, calls, total) = state.get();
        if counting {
            state.set((
                true,
                calls.saturating_add(1),
                total.saturating_add(bytes as u64),
            ));
        }
    });
}

// SAFETY: every operation delegates to `System` with the original pointer and
// layout; the thread-local counter only observes allocator calls.
#[cfg(feature = "allocation-counting")]
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count_allocation(layout.size());
        // SAFETY: the caller supplies the layout required by `GlobalAlloc`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count_allocation(layout.size());
        // SAFETY: the caller supplies the layout required by `GlobalAlloc`.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: the caller returns the pointer with its original layout.
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count_allocation(new_size);
        // SAFETY: the caller supplies the pointer, original layout, and size.
        unsafe { System.realloc(pointer, layout, new_size) }
    }
}

/// Set once from `--mode`: count allocator calls instead of timing.
static COUNT_ALLOCATIONS: AtomicBool = AtomicBool::new(false);

/// One measurement: milliseconds in elapsed mode; allocator calls and
/// requested bytes in allocation mode.
#[derive(Clone, Copy)]
struct Sample {
    milliseconds: f64,
    allocations: u64,
    allocated_bytes: u64,
}

struct Meter(Instant);

impl Meter {
    fn start() -> Self {
        #[cfg(feature = "allocation-counting")]
        if COUNT_ALLOCATIONS.load(Ordering::Relaxed) {
            ALLOCATIONS.with(|state| state.set((true, 0, 0)));
        }
        Self(Instant::now())
    }

    fn finish(self) -> Sample {
        let milliseconds = self.0.elapsed().as_secs_f64() * 1_000.0;
        #[cfg(feature = "allocation-counting")]
        if COUNT_ALLOCATIONS.load(Ordering::Relaxed) {
            let (_, allocations, allocated_bytes) =
                ALLOCATIONS.with(|state| state.replace((false, 0, 0)));
            return Sample {
                milliseconds,
                allocations,
                allocated_bytes,
            };
        }
        Sample {
            milliseconds,
            allocations: 0,
            allocated_bytes: 0,
        }
    }
}

fn operation_key(label: &str, index: usize) -> Result<ResourceOperationKey, ResourceError> {
    ResourceOperationKey::new(format!("profile:{label}:{index}"))
}

/// One rejected operation outcome to copy: cancelling a demand that does not
/// exist settles as a durable rejection.
fn template() -> Result<ResourceOperationOutcome, Box<dyn Error>> {
    let mut state = ResourceState::empty(ResourceLimitsV1::canonical())?;
    Ok(
        state.apply_operation(&ResourceOperationRequestV1::CancelDemand(
            ResourceCancelDemandRequestV1 {
                operation_key: operation_key("template", 0)?,
                demand: ResourceDemandId::new("profile:missing:0")?,
                expected_demand_revision: ResourceRevision::INITIAL,
            },
        ))?,
    )
}

/// Adds one hot terminal outcome: `template` under its own operation key.
fn add_hot_outcome(
    state: &mut ResourceState,
    template: &ResourceOperationOutcome,
    index: usize,
) -> Result<(), ResourceError> {
    let mut outcome = template.clone();
    outcome.operation_key = operation_key("hot", index)?;
    outcome.id = ResourceOperationOutcomeId::new(format!(
        "resource:outcome:{}:{}",
        outcome.operation_key.as_str().replace(':', "-"),
        &outcome.request_digest[..16]
    ))?;
    outcome.sequence = state.next_admitted_sequence;
    state.next_admitted_sequence += 1;
    outcome.semantic_digest.clear();
    outcome.semantic_digest = canonical_digest("canwu.resource.operation-outcome.v1", &outcome)?;
    state.terminal_archive_candidates.insert(
        outcome.sequence,
        ResourceTerminalRecordKeyV1::Outcome(outcome.operation_key.clone()),
    );
    state
        .outcomes
        .insert(outcome.operation_key.clone(), outcome);
    Ok(())
}

/// The domain records that install `state` in a scenario: its live root and
/// its hot terminal buckets.
fn root_records(state: ResourceState) -> Result<Vec<DomainRecord>, CanwuError> {
    state.into_records()
}

/// Encoded bytes of every domain record the resource plugin owns.
fn resource_record_bytes(canwu: &Canwu) -> Result<usize, Box<dyn Error>> {
    let mut bytes = 0;
    for record in canwu.domain_records() {
        if record.owner == canwu_resource::PLUGIN_NAME {
            bytes += serde_json::to_vec(&record.payload)?.len();
        }
    }
    Ok(bytes)
}

fn enqueue_passes(
    canwu: &mut Canwu,
    requester: &KnowledgeHolderRef,
    label: &str,
    count: usize,
) -> Result<(), Box<dyn Error>> {
    let at = canwu.time();
    let (_, root) = resource_state(canwu)?.ok_or("the resource root is missing")?;
    for index in 0..count {
        enqueue_resource_allocation(
            canwu,
            at,
            requester,
            &ResourceAllocationRequestV1 {
                operation_key: operation_key(label, index)?,
                expected_state_revision: root.state_revision,
                at,
                candidate_limit: 1,
            },
        )?;
    }
    Ok(())
}

/// Settles one boundary at the current time and measures it and the journal
/// evidence it appends.
fn measured_boundary(canwu: &mut Canwu) -> Result<(Sample, usize), Box<dyn Error>> {
    let cursor = canwu.evidence_cursor()?;
    let meter = Meter::start();
    canwu.settle_boundary(BoundaryRequest::at(canwu.time()))?;
    let sample = meter.finish();
    let journal = serde_json::to_vec(&canwu.journal_segment_since(cursor)?)?.len();
    Ok((sample, journal))
}

struct Series {
    samples: Vec<Sample>,
    journal_bytes: Vec<usize>,
}

impl Series {
    fn new() -> Self {
        Self {
            samples: Vec::new(),
            journal_bytes: Vec::new(),
        }
    }

    fn push(&mut self, (sample, journal): (Sample, usize)) {
        self.samples.push(sample);
        self.journal_bytes.push(journal);
    }

    fn report(&self, count_allocations: bool) -> Value {
        let median = |mut values: Vec<f64>| {
            values.sort_by(f64::total_cmp);
            values[values.len() / 2]
        };
        let mut journal = self.journal_bytes.clone();
        journal.sort_unstable();
        let mut report = json!({
            "journal_bytes": journal[journal.len() / 2],
        });
        if count_allocations {
            report["allocations"] = json!(median(
                self.samples.iter().map(|s| s.allocations as f64).collect()
            ));
            report["allocated_bytes"] = json!(median(
                self.samples
                    .iter()
                    .map(|s| s.allocated_bytes as f64)
                    .collect()
            ));
            report["raw_allocations"] = json!(
                self.samples
                    .iter()
                    .map(|s| s.allocations)
                    .collect::<Vec<_>>()
            );
        } else {
            report["ms"] = json!(median(
                self.samples.iter().map(|s| s.milliseconds).collect()
            ));
            report["raw_ms"] = json!(
                self.samples
                    .iter()
                    .map(|s| s.milliseconds)
                    .collect::<Vec<_>>()
            );
        }
        report
    }
}

fn case(
    template: &ResourceOperationOutcome,
    backlog: usize,
    options: &Options,
) -> Result<Value, Box<dyn Error>> {
    let mut limits = ResourceLimitsV1::canonical();
    // A budget of 4,096 up to a backlog of 4,000 (the 0.15.0 ceiling, which
    // every build measured supports), and the largest budget above it.
    let budget = if backlog <= 4_000 {
        4_096
    } else {
        ResourceLimitsV1::MAX_ARCHIVE_CANDIDATES
    };
    limits.max_archive_candidates = budget;
    limits.max_archive_maintenance_receipts = budget;
    if backlog + 64 > limits.max_operation_outcomes {
        limits.max_operation_outcomes = 16_384;
    }
    let mut state = ResourceState::empty(limits)?;
    for index in 0..backlog {
        add_hot_outcome(&mut state, template, index)?;
    }
    state.refresh_hot_terminal_ledger()?;
    state.validate()?;

    let requester = KnowledgeHolderRef::Person(PersonId::new(1));
    let tick = TickPlugin;
    let resource = ResourcePlugin::default();
    let plugins: [&dyn SimulationPlugin; 2] = [&tick, &resource];
    let scenario = Scenario::new(SimTime::EPOCH, vec![EntityRef::Person(PersonId::new(1))])
        .with_domain_records(root_records(state)?);
    let mut canwu = Canwu::new_with_plugins(PROFILE_SEED, scenario, &plugins)?;
    let archive = MemoryArchive::default();
    canwu.set_plugin_archive_object_provider(Rc::new(archive.clone()));
    let installed_bytes = resource_record_bytes(&canwu)?;

    let (mut operation, mut batch, mut idle) = (Series::new(), Series::new(), Series::new());
    let mut settled = 0;
    for sample in 0..options.warmup + options.samples {
        let keep = sample >= options.warmup;

        enqueue_passes(&mut canwu, &requester, &format!("operation-{sample}"), 1)?;
        let measured = measured_boundary(&mut canwu)?;
        settled += 1;
        if keep {
            operation.push(measured);
        }

        enqueue_passes(&mut canwu, &requester, &format!("batch-{sample}"), BATCH)?;
        let measured = measured_boundary(&mut canwu)?;
        settled += BATCH;
        if keep {
            batch.push(measured);
        }

        canwu.enqueue_plugin_ingress(PluginIngressRequest::new(
            TICK_PLUGIN,
            TICK_INGRESS,
            canwu.time(),
            json!({}),
        ))?;
        let measured = measured_boundary(&mut canwu)?;
        if keep {
            idle.push(measured);
        }
    }
    let (_, root) = resource_state(&canwu)?.ok_or("the resource root is missing")?;
    if root.outcomes.len() != backlog + settled {
        return Err("a sampled boundary did not settle its operations".into());
    }

    // One commit of every hot terminal record, prepared and stored by the
    // host application; only the boundary that applies it is measured.
    let candidates = root.terminal_archive_candidates.len();
    let prepared = root.prepare_resource_archive(candidates, &archive)?;
    enqueue_resource_archive(&mut canwu, &prepared, &archive)?;
    let mut commit = Series::new();
    commit.push(measured_boundary(&mut canwu)?);
    let (_, root) = resource_state(&canwu)?.ok_or("the resource root is missing")?;
    if root.archive_head.archived_record_count != candidates as u64 {
        return Err("the archive commit did not apply".into());
    }
    Ok(json!({
        "hot_terminal_records": backlog,
        "installed_resource_record_bytes": installed_bytes,
        "operation": operation.report(options.count_allocations),
        "batch_of_8": batch.report(options.count_allocations),
        "idle": idle.report(options.count_allocations),
        "archive_commit": {
            "archived_records": candidates,
            "boundary": commit.report(options.count_allocations),
        },
    }))
}

struct Options {
    count_allocations: bool,
    backlogs: Vec<usize>,
    samples: usize,
    warmup: usize,
    machine: String,
    recorded_on: String,
    label: String,
    output: Option<PathBuf>,
}

fn options() -> Result<Options, Box<dyn Error>> {
    let mut options = Options {
        count_allocations: false,
        backlogs: vec![0, 256, 1_024, 4_000],
        samples: 5,
        warmup: 1,
        machine: "local-machine".to_owned(),
        recorded_on: "unspecified".to_owned(),
        label: "unspecified".to_owned(),
        output: None,
    };
    let mut arguments = env::args().skip(1);
    while let Some(argument) = arguments.next() {
        let mut value = |name: &str| arguments.next().ok_or(format!("{name} requires a value"));
        match argument.as_str() {
            "--mode" => {
                options.count_allocations = match value("--mode")?.as_str() {
                    "elapsed" => false,
                    "allocations" => true,
                    other => return Err(format!("unsupported mode: {other}").into()),
                }
            }
            "--backlogs" => {
                options.backlogs = value("--backlogs")?
                    .split(',')
                    .map(str::parse)
                    .collect::<Result<_, _>>()?;
            }
            "--samples" => options.samples = value("--samples")?.parse()?,
            "--warmup" => options.warmup = value("--warmup")?.parse()?,
            "--machine" => options.machine = value("--machine")?,
            "--recorded-on" => options.recorded_on = value("--recorded-on")?,
            "--label" => options.label = value("--label")?,
            "--output" => options.output = Some(PathBuf::from(value("--output")?)),
            "--help" | "-h" => {
                println!(
                    "Canwu resource root profile\n\n\
                     --mode elapsed|allocations --backlogs 0,256,1024,4000\n\
                     --samples ODD --warmup N --label TEXT\n\
                     --machine LABEL --recorded-on DATE --output PATH"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unsupported argument: {other}").into()),
        }
    }
    if options.samples == 0 || options.samples % 2 == 0 {
        return Err("--samples must be a positive odd number".into());
    }
    Ok(options)
}

fn command_output(program: &str, arguments: &[&str]) -> String {
    Command::new(program)
        .args(arguments)
        .output()
        .ok()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_default()
}

/// BLAKE3 over the measured sources: the resource extension and this file.
fn source_content_hash() -> String {
    let root = PathBuf::from(command_output("git", &["rev-parse", "--show-toplevel"]));
    let mut files =
        vec![root.join("benchmarks/performance-harness/src/bin/resource-root-profile.rs")];
    if let Ok(entries) = fs::read_dir(root.join("crates/extensions/canwu-resource/src")) {
        files.extend(entries.flatten().map(|entry| entry.path()));
    }
    files.sort();
    let mut hasher = blake3::Hasher::new();
    for path in files {
        if let Ok(relative) = path.strip_prefix(&root)
            && let Ok(bytes) = fs::read(&path)
        {
            hasher.update(relative.to_string_lossy().replace('\\', "/").as_bytes());
            hasher.update(&[0]);
            hasher.update(&bytes);
            hasher.update(&[0]);
        }
    }
    hasher.finalize().to_hex().to_string()
}

fn main() -> Result<(), Box<dyn Error>> {
    let options = options()?;
    if options.count_allocations && !cfg!(feature = "allocation-counting") {
        return Err("allocation mode requires --features allocation-counting".into());
    }
    if !options.count_allocations && cfg!(feature = "allocation-counting") {
        return Err("elapsed mode must be built without the allocation-counting feature".into());
    }
    COUNT_ALLOCATIONS.store(options.count_allocations, Ordering::Relaxed);
    let template = template()?;
    let mut cases = Vec::with_capacity(options.backlogs.len());
    for backlog in &options.backlogs {
        eprintln!("case: {backlog} hot terminal records");
        cases.push(case(&template, *backlog, &options)?);
    }
    let report = json!({
        "benchmark": "resource-root-profile",
        "mode": if options.count_allocations { "allocations" } else { "elapsed" },
        "label": options.label,
        "machine": options.machine,
        "recorded_on": options.recorded_on,
        "samples": options.samples,
        "warmup": options.warmup,
        "source": {
            "git_commit": command_output("git", &["rev-parse", "HEAD"]),
            "git_dirty": !command_output("git", &["status", "--porcelain"]).is_empty(),
            "content_hash": source_content_hash(),
            "rustc": command_output("rustc", &["-vV"]),
        },
        "cases": cases,
    });
    let text = serde_json::to_string_pretty(&report)?;
    match options.output {
        Some(path) => fs::write(path, text)?,
        None => println!("{text}"),
    }
    Ok(())
}
