mod information_stress;
mod peak_rss;

use canwu_api::{
    ArchiveProvider, ArchiveStore, ArchiveStoreOutcome, Army, ArmyId, BoundaryContext,
    BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundaryRequest, BoundarySystemContract,
    Canwu, CanwuError, Command, CommandEnvelope, CommandOutcome, CommandRequest, CommandRequestId,
    CompactedCanwu, ENGINE_VERSION, EntityRef, ErrorCode, EvidenceCursor, EvidenceJournalSegment,
    Government, GovernmentId, Issuer, KnowledgeHistoryView, KnowledgeHolderRef, KnowledgeLimitsV1,
    KnowledgeOrigin, KnowledgeQuery, KnowledgeRecordDraft, KnowledgeRecordKind, KnowledgeSchemaId,
    KnowledgeSnapshot, KnowledgeWriteGrant, MapPoint, PayloadSchema, Person, PersonId,
    PluginKnowledgeSchema, PluginRegistrar, RandomStreamKey, ReplayJournal,
    SNAPSHOT_FORMAT_VERSION, Scenario, SimDuration, SimulationCheckpoint, SimulationPlugin,
    SimulationView, StateKey, StateVisibility, SystemCadence, Territory, TerritoryId,
    WorldSnapshot,
};
use canwu_information::InformationRecordSet;
use serde_json::{Value, json};
#[cfg(feature = "allocation-counting")]
use std::alloc::{GlobalAlloc, Layout, System};
#[cfg(feature = "allocation-counting")]
use std::cell::Cell;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::fs::{self, File};
use std::hint::black_box;
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{self, Command as ProcessCommand};
use std::time::Instant;

const DEFAULT_SCALES: &[usize] = &[8, 32, 128, 512];
const INFORMATION_SMOKE_SCALES: &[usize] = &[100, 1_000, 10_000];
const INFORMATION_FULL_SCALES: &[usize] = &[10_000, 100_000, 1_000_000];
const DEFAULT_SAMPLES: usize = 5;
const DEFAULT_GROWTH_SAMPLES: usize = 3;
const DEFAULT_WARMUP: usize = 1;
const PROFILE_SEED: u64 = 0xCA6E_2026;
const PROFILE_PLUGIN_HASH: &str =
    "8aa5d87954e0ca0a624c2e9ccde27893f149078026cc070b3ea3df46328a84d5";
const INFORMATION_PLUGIN_HASH: &str =
    "ec708dd81dd24882502727be4485831f0ec6c9cb613af1e87cb0eec2721661cc";
const SCHEMA_STRESS_PLUGIN_HASH: &str =
    "c99cfc08ec9718f6d81a695be99d4f6b926dd1a4cc918785652e7a4a49c6609d";
const SCHEMA_STRESS_COUNT: usize = 100;
const INFORMATION_SYSTEM_PREFIX: &str = "knowledge-growth-";
const INFORMATION_HOLDER_COUNT: usize = 60;
const INFORMATION_SEED: u64 = 0x1F10_2026;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BenchmarkSuite {
    Architecture,
    InformationFlow,
}

impl BenchmarkSuite {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Architecture => "architecture",
            Self::InformationFlow => "information-flow",
        }
    }

    const fn report_name(self) -> &'static str {
        match self {
            Self::Architecture => "canwu-architecture-baseline",
            Self::InformationFlow => "canwu-information-flow-baseline",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InformationPreset {
    Smoke,
    Full,
}

impl InformationPreset {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Smoke => "smoke",
            Self::Full => "full",
        }
    }

    const fn scales(self) -> &'static [usize] {
        match self {
            Self::Smoke => INFORMATION_SMOKE_SCALES,
            Self::Full => INFORMATION_FULL_SCALES,
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
    suite: BenchmarkSuite,
    information_preset: InformationPreset,
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

struct SchemaStressPlugin;

impl SimulationPlugin for SchemaStressPlugin {
    fn name(&self) -> &'static str {
        "canwu-schema-stress-profile"
    }

    fn version(&self) -> &'static str {
        "1.0.0"
    }

    fn semantic_hash(&self) -> &'static str {
        SCHEMA_STRESS_PLUGIN_HASH
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        for index in 0..SCHEMA_STRESS_COUNT {
            registrar.register_knowledge_schema(PluginKnowledgeSchema {
                id: KnowledgeSchemaId::new(
                    KnowledgeRecordKind::new("benchmark.schema", format!("knowledge-{index:03}")),
                    1,
                ),
                schema_hash: SCHEMA_STRESS_PLUGIN_HASH.to_owned(),
                writable: true,
                payload_schema: PayloadSchema::Any,
                subjects: Vec::new(),
            })?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct InformationFlowPlugin {
    target_records: usize,
}

impl SimulationPlugin for InformationFlowPlugin {
    fn name(&self) -> &'static str {
        "canwu-information-flow-profile"
    }

    fn version(&self) -> &'static str {
        "1.0.0"
    }

    fn semantic_hash(&self) -> &'static str {
        INFORMATION_PLUGIN_HASH
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_knowledge_schema(PluginKnowledgeSchema {
            id: information_schema(),
            schema_hash: "88902ab24869a59e3459b1fcfd782df80a1e6bf64f3cae2b5e38020dc49d0f2d"
                .to_owned(),
            writable: true,
            payload_schema: PayloadSchema::Any,
            subjects: Vec::new(),
        })?;
        let mut contract = BoundarySystemContract::new(
            information_system_name(self.target_records),
            BoundaryPhase::PerspectiveAndReportMaterialization,
            SystemCadence::Daily,
        );
        contract.knowledge_writes = vec![KnowledgeWriteGrant {
            schema: information_schema(),
            visibilities: vec![StateVisibility::SameBoundary],
        }];
        registrar.register_boundary_system(contract, publish_information_growth)
    }
}

struct InformationFixture {
    simulation: Canwu,
    plugin: InformationFlowPlugin,
}

struct InformationBenchmarkInputs {
    snapshot: PathBuf,
    journal: PathBuf,
}

impl InformationBenchmarkInputs {
    fn new(scale: usize) -> Result<Self, Box<dyn Error>> {
        let prefix = format!("canwu-information-flow-{}-{scale}", process::id());
        let directory = env::temp_dir();
        let inputs = Self {
            snapshot: directory.join(format!("{prefix}-snapshot.json")),
            journal: directory.join(format!("{prefix}-journal.json")),
        };
        for path in [&inputs.snapshot, &inputs.journal] {
            if path.exists() {
                fs::remove_file(path)?;
            }
        }
        Ok(inputs)
    }
}

impl Drop for InformationBenchmarkInputs {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.snapshot);
        let _ = fs::remove_file(&self.journal);
    }
}

#[derive(Clone)]
struct ArchiveReplayFixture {
    checkpoint: SimulationCheckpoint,
    segments: Vec<EvidenceJournalSegment>,
}

#[derive(Default)]
struct MemoryArchive {
    segments: RefCell<Vec<EvidenceJournalSegment>>,
}

impl MemoryArchive {
    fn segments(&self) -> Vec<EvidenceJournalSegment> {
        self.segments.borrow().clone()
    }
}

impl ArchiveProvider for MemoryArchive {
    fn load_evidence_segment(
        &self,
        segment_id: &str,
    ) -> Result<Option<EvidenceJournalSegment>, CanwuError> {
        Ok(self
            .segments
            .borrow()
            .iter()
            .find(|segment| {
                segment
                    .archive
                    .as_ref()
                    .is_some_and(|archive| archive.header.segment_id == segment_id)
            })
            .cloned())
    }
}

impl ArchiveStore for MemoryArchive {
    fn store_evidence_segment(
        &self,
        segment: &EvidenceJournalSegment,
    ) -> Result<ArchiveStoreOutcome, CanwuError> {
        let segment_id = segment
            .archive
            .as_ref()
            .ok_or_else(|| benchmark_error("archive provider received an unindexed segment"))?
            .header
            .segment_id
            .clone();
        let mut segments = self.segments.borrow_mut();
        if let Some(existing) = segments.iter().find(|candidate| {
            candidate
                .archive
                .as_ref()
                .is_some_and(|archive| archive.header.segment_id == segment_id)
        }) {
            return if existing == segment {
                Ok(ArchiveStoreOutcome::AlreadyPresent)
            } else {
                Err(benchmark_error(
                    "archive provider found different bytes for one segment ID",
                ))
            };
        }
        segments.push(segment.clone());
        Ok(ArchiveStoreOutcome::Stored)
    }
}

fn information_schema() -> KnowledgeSchemaId {
    KnowledgeSchemaId::new(
        KnowledgeRecordKind::new("canwu.performance", "observation"),
        1,
    )
}

fn information_system_name(target_records: usize) -> String {
    format!("{INFORMATION_SYSTEM_PREFIX}{target_records:020}")
}

fn information_target(system: &str) -> Result<usize, CanwuError> {
    system
        .strip_prefix(INFORMATION_SYSTEM_PREFIX)
        .ok_or_else(|| benchmark_error("information profile system name is invalid"))?
        .parse()
        .map_err(|_| benchmark_error("information profile target is invalid"))
}

fn information_draft(sequence: usize, context: &BoundaryContext) -> KnowledgeRecordDraft {
    KnowledgeRecordDraft {
        schema: information_schema(),
        subjects: Vec::new(),
        payload: json!({ "sequence": sequence }),
        as_of: Some(context.at),
        confidence_per_mille: 1_000,
        origin: KnowledgeOrigin {
            method: "profile_observation".to_owned(),
            evidence: Vec::new(),
        },
        supersedes: Vec::new(),
        contradicts: Vec::new(),
    }
}

fn publication_directive(
    holder: usize,
    first_sequence: usize,
    record_count: usize,
    context: &BoundaryContext,
    batch: usize,
) -> Result<BoundaryDirective, CanwuError> {
    let holder_id =
        u64::try_from(holder).map_err(|_| benchmark_error("information holder ID exceeds u64"))?;
    let records = (0..record_count)
        .map(|offset| information_draft(first_sequence + offset, context))
        .collect();
    Ok(BoundaryDirective::PublishKnowledge {
        holder: KnowledgeHolderRef::Person(PersonId::new(holder_id)),
        visibility: StateVisibility::SameBoundary,
        producer_correlation: Some(format!(
            "boundary-{}-holder-{holder}-batch-{batch}",
            context.boundary_id.get()
        )),
        records,
        summary: "Publish deterministic information-flow profile records".to_owned(),
    })
}

fn publish_information_growth(
    _view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let target_records = information_target(&context.system)?;
    let boundary_index = usize::try_from(context.boundary_id.get().saturating_sub(1))
        .map_err(|_| benchmark_error("boundary index exceeds usize"))?;
    let records_per_boundary = KnowledgeLimitsV1::CURRENT.records_per_boundary;
    let already_published = boundary_index
        .checked_mul(records_per_boundary)
        .ok_or_else(|| benchmark_error("information record count overflow"))?;
    if already_published >= target_records {
        return Ok(BoundaryProposal::default());
    }
    let boundary_records = (target_records - already_published).min(records_per_boundary);
    let hot_records = if boundary_records == 1 {
        1
    } else {
        boundary_records / 2
    };
    let mut directives = Vec::new();
    let mut hot_offset = 0;
    let mut hot_batch = 0;
    while hot_offset < hot_records {
        let count = (hot_records - hot_offset).min(KnowledgeLimitsV1::CURRENT.records_per_batch);
        directives.push(publication_directive(
            1,
            already_published + hot_offset,
            count,
            context,
            hot_batch,
        )?);
        hot_offset += count;
        hot_batch += 1;
    }

    let cold_records = boundary_records - hot_records;
    let cold_holders = cold_records.min(INFORMATION_HOLDER_COUNT.saturating_sub(1));
    if cold_holders != 0 {
        let base = cold_records
            .checked_div(cold_holders)
            .ok_or_else(|| benchmark_error("cold holder division failed"))?;
        let extra = cold_records
            .checked_rem(cold_holders)
            .ok_or_else(|| benchmark_error("cold holder remainder failed"))?;
        let mut cold_offset = hot_records;
        for index in 0..cold_holders {
            let count = base + usize::from(index < extra);
            directives.push(publication_directive(
                index + 2,
                already_published + cold_offset,
                count,
                context,
                0,
            )?);
            cold_offset += count;
        }
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
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
    if options.suite == BenchmarkSuite::InformationFlow {
        return run_information_flow(&options);
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
        let checkpoint = fixture.simulation.checkpoint()?;
        let checkpoint_json = serde_json::to_string_pretty(&checkpoint)?;
        let full_segment = fixture
            .simulation
            .journal_segment_since(EvidenceCursor::default())?;
        let full_segment_json = serde_json::to_string_pretty(&full_segment)?;
        let journal_end = fixture.simulation.evidence_cursor()?;
        let checkpoint_journal_json = fixture.simulation.checkpoint_journal_json()?;
        let journal = fixture.simulation.replay_journal();
        let checkpoint_hash = fixture.simulation.checkpoint_hash().to_owned();
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
        let checkpoint_create = measure_case(
            options.mode,
            options.warmup,
            options.samples,
            || restore(&snapshot_json),
            |simulation| simulation.checkpoint(),
        )?;
        let checkpoint_serialize = measure_case(
            options.mode,
            options.warmup,
            options.samples,
            || Ok::<_, serde_json::Error>(checkpoint.clone()),
            |checkpoint| serde_json::to_string_pretty(checkpoint),
        )?;
        let full_journal_segment = measure_case(
            options.mode,
            options.warmup,
            options.samples,
            || restore(&snapshot_json),
            |simulation| simulation.journal_segment_since(EvidenceCursor::default()),
        )?;
        let empty_journal_segment = measure_case(
            options.mode,
            options.warmup,
            options.samples,
            || restore(&snapshot_json),
            |simulation| simulation.journal_segment_since(journal_end),
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
            || Ok::<_, CanwuError>(journal.clone()),
            |journal| Canwu::replay_from_journal(&[&ProfilePlugin], journal),
        )?;
        let live_archive_seal = measure_case(
            options.mode,
            options.warmup,
            options.samples,
            || prepare_live_archive(&snapshot_json).map(Some),
            |simulation| {
                let simulation = simulation
                    .take()
                    .ok_or_else(|| benchmark_error("live archive setup was already consumed"))?;
                let mut compact = simulation.into_compacted()?;
                let segment = compact
                    .seal_evidence()?
                    .ok_or_else(|| benchmark_error("populated live evidence tail was empty"))?;
                Ok((compact, segment))
            },
        )?;
        let live_archive_segment_release = measure_case(
            options.mode,
            options.warmup,
            options.samples,
            || {
                let mut compact = prepare_live_archive(&snapshot_json)?.into_compacted()?;
                let segment = compact
                    .seal_evidence()?
                    .ok_or_else(|| benchmark_error("populated live evidence tail was empty"))?;
                Ok((compact, Some(segment)))
            },
            |(_, segment)| {
                drop(segment.take().ok_or_else(|| {
                    benchmark_error("live archive segment setup was already consumed")
                })?);
                Ok::<(), CanwuError>(())
            },
        )?;
        let live_archive_cycle_growth = measure_case(
            options.mode,
            options.warmup,
            options.growth_samples,
            || Ok(()),
            |_| build_compacted_growth(scale),
        )?;
        let live_archive_repeated_seal = measure_case(
            options.mode,
            options.warmup,
            options.growth_samples,
            || Ok(()),
            |_| build_repeated_seal_fixture(scale),
        )?;
        let mut compact_storage = prepare_live_archive(&snapshot_json)?.into_compacted()?;
        let compact_segment = compact_storage
            .seal_evidence()?
            .ok_or_else(|| benchmark_error("populated live evidence tail was empty"))?;
        let compact_checkpoint = compact_storage.checkpoint()?;
        let compact_checkpoint_json = serde_json::to_string_pretty(&compact_checkpoint)?;
        let compact_segment_json = serde_json::to_string_pretty(&compact_segment)?;

        scale_reports.push(json!({
            "scale": scale,
            "history": history_json(&snapshot, snapshot_json.len(), &checkpoint_hash),
            "checkpoint_storage": {
                "current_state_checkpoint_bytes": checkpoint_json.len(),
                "full_journal_segment_bytes": full_segment_json.len(),
                "full_checkpoint_journal_bytes": checkpoint_journal_json.len(),
                "compacted_current_checkpoint_bytes": compact_checkpoint_json.len(),
                "released_archive_segment_bytes": compact_segment_json.len(),
            },
            "cases": [
                case_json("history_growth", &growth, options.mode),
                case_json("accepted_command", &accepted, options.mode),
                case_json("rejected_command", &rejected, options.mode),
                case_json("empty_boundary", &empty_boundary, options.mode),
                case_json("populated_boundary", &populated_boundary, options.mode),
                case_json("snapshot_create", &snapshot_create, options.mode),
                case_json("snapshot_serialize_pretty", &snapshot_serialize, options.mode),
                case_json("checkpoint_create", &checkpoint_create, options.mode),
                case_json("checkpoint_serialize_pretty", &checkpoint_serialize, options.mode),
                case_json("journal_segment_full", &full_journal_segment, options.mode),
                case_json("journal_segment_empty_tail", &empty_journal_segment, options.mode),
                case_json("snapshot_load_validate", &snapshot_load_validate, options.mode),
                case_json("exact_replay", &exact_replay, options.mode),
                case_json("live_archive_seal", &live_archive_seal, options.mode),
                case_json(
                    "live_archive_segment_release",
                    &live_archive_segment_release,
                    options.mode,
                ),
                case_json("live_archive_cycle_growth", &live_archive_cycle_growth, options.mode),
                case_json("live_archive_repeated_seal", &live_archive_repeated_seal, options.mode),
            ],
        }));
    }

    let report = json!({
        "schema_version": 2,
        "benchmark": options.suite.report_name(),
        "suite": options.suite.as_str(),
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
    write_report(&report, options.mode, options.output.as_deref())
}

fn write_report(
    report: &Value,
    mode: MetricMode,
    output: Option<&Path>,
) -> Result<(), Box<dyn Error>> {
    let encoded = serde_json::to_string_pretty(report)?;
    print_summary(report, mode);
    if let Some(output) = output {
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

fn measure_information_stress(options: &Options) -> Result<Value, Box<dyn Error>> {
    let mut rss_checkpoints = vec![rss_checkpoint("before_extension_workloads")];

    let schema_scenario = profile_scenario(1)?;
    let schema_registration = measure_case(
        options.mode,
        options.warmup,
        options.samples,
        || Canwu::new(PROFILE_SEED, schema_scenario.clone()),
        |simulation| {
            simulation.register_plugin(&SchemaStressPlugin)?;
            let count = simulation
                .plugin_descriptors()
                .find(|descriptor| descriptor.name == SchemaStressPlugin.name())
                .map_or(0, |descriptor| descriptor.knowledge_schemas.len());
            if count != SCHEMA_STRESS_COUNT {
                return Err(benchmark_error(&format!(
                    "schema registration retained {count} schemas instead of {SCHEMA_STRESS_COUNT}"
                )));
            }
            Ok(count)
        },
    )?;
    rss_checkpoints.push(rss_checkpoint("after_knowledge_schema_100_registration"));

    let addressed_fixture = information_stress::addressed_dispatch_fixture()?;
    let addressed_dispatch = measure_case(
        options.mode,
        options.warmup,
        options.samples,
        || Ok::<_, String>(addressed_fixture.clone()),
        |fixture| {
            let plan = information_stress::plan_lifecycle(fixture)?;
            if plan.mutations.len() != 1 {
                return Err("addressed dispatch plan did not produce exactly one record".to_owned());
            }
            Ok(plan)
        },
    )?;
    rss_checkpoints.push(rss_checkpoint("after_addressed_dispatch_10000"));

    let audience_fixture = information_stress::explicit_audience_fixture()?;
    let explicit_audience = measure_case(
        options.mode,
        options.warmup,
        options.samples,
        || Ok::<_, String>(audience_fixture.clone()),
        |fixture| {
            let plan = information_stress::plan_lifecycle(fixture)?;
            if plan.mutations.len() != 1 {
                return Err("explicit audience plan did not produce exactly one record".to_owned());
            }
            Ok(plan)
        },
    )?;
    rss_checkpoints.push(rss_checkpoint("after_explicit_audience_10000"));

    let lineage_fixture = information_stress::mixed_lineage_fixture();
    let mixed_lineage = measure_case(
        options.mode,
        options.warmup,
        options.samples,
        || Ok::<_, String>(lineage_fixture.clone()),
        |fixture| {
            let count = fixture.validate()?;
            if count != information_stress::MIXED_LINEAGE_NODE_COUNT {
                return Err(format!(
                    "mixed lineage fixture validated {count} nodes instead of {}",
                    information_stress::MIXED_LINEAGE_NODE_COUNT
                ));
            }
            Ok(count)
        },
    )?;
    rss_checkpoints.push(rss_checkpoint("after_mixed_lineage_1000"));

    let access_records = information_stress::access_records()?;
    let access_index_build = measure_case(
        options.mode,
        options.warmup,
        options.samples,
        || Ok::<_, String>(access_records.clone()),
        |records| {
            let record_set = InformationRecordSet::from_records(std::mem::take(records))?;
            if record_set.len() != information_stress::ACCESS_RECORD_COUNT {
                return Err("access index did not retain all 100,000 records".to_owned());
            }
            Ok(record_set)
        },
    )?;
    rss_checkpoints.push(rss_checkpoint("after_access_100000_index_build"));

    let indexed_access = InformationRecordSet::from_records(access_records)?;
    let access_query = information_stress::access_query();
    let access_scan = measure_case(
        options.mode,
        options.warmup,
        options.samples,
        || Ok::<_, String>(indexed_access.clone()),
        |record_set| {
            let matches = record_set.query(&access_query)?;
            if matches.len() != information_stress::ACCESS_RECORD_COUNT {
                return Err(format!(
                    "access query returned {} records instead of {}",
                    matches.len(),
                    information_stress::ACCESS_RECORD_COUNT
                ));
            }
            Ok(matches.len())
        },
    )?;
    rss_checkpoints.push(rss_checkpoint("after_access_100000_query"));

    let holder_queries = information_stress::access_holder_queries();
    let access_holder_queries = measure_case(
        options.mode,
        options.warmup,
        options.samples,
        || Ok::<_, String>(indexed_access.clone()),
        |record_set| {
            let expected_per_holder =
                information_stress::ACCESS_RECORD_COUNT / information_stress::ACCESS_HOLDER_COUNT;
            let mut match_count = 0_usize;
            for query in &holder_queries {
                let matches = record_set.query(query)?;
                if matches.len() != expected_per_holder {
                    return Err(format!(
                        "holder query returned {} records instead of {expected_per_holder}",
                        matches.len()
                    ));
                }
                match_count = match_count
                    .checked_add(matches.len())
                    .ok_or_else(|| "holder query match count overflow".to_owned())?;
            }
            if match_count != information_stress::ACCESS_RECORD_COUNT {
                return Err(format!(
                    "holder queries returned {match_count} records instead of {}",
                    information_stress::ACCESS_RECORD_COUNT
                ));
            }
            Ok(match_count)
        },
    )?;
    rss_checkpoints.push(rss_checkpoint("after_access_100000_1000_holder_queries"));

    let archive_build = measure_case(
        options.mode,
        options.warmup,
        options.samples,
        || Ok::<_, Box<dyn Error>>(()),
        |_| build_archive_replay_fixture(100),
    )?;
    rss_checkpoints.push(rss_checkpoint("after_archive_provider_100_segment_build"));

    let archive_fixture = build_archive_replay_fixture(100)?;
    let archive_restore = measure_case(
        options.mode,
        options.warmup,
        options.samples,
        || Ok::<_, CanwuError>(archive_fixture.clone()),
        |fixture| {
            let restored = CompactedCanwu::from_checkpoint_and_journal_with_plugins(
                fixture.checkpoint.clone(),
                fixture.segments.clone(),
                &[&ProfilePlugin],
            )?;
            Ok(restored.checkpoint_hash().to_owned())
        },
    )?;
    rss_checkpoints.push(rss_checkpoint("after_archive_provider_100_segment_restore"));

    Ok(json!({
        "limits": {
            "knowledge_schemas": SCHEMA_STRESS_COUNT,
            "addressed_recipients": information_stress::ADDRESSED_RECIPIENT_COUNT,
            "explicit_audience_members": information_stress::EXPLICIT_AUDIENCE_MEMBER_COUNT,
            "mixed_lineage_nodes": information_stress::MIXED_LINEAGE_NODE_COUNT,
            "mixed_lineage_operations": 4,
            "access_records": information_stress::ACCESS_RECORD_COUNT,
            "access_holders": information_stress::ACCESS_HOLDER_COUNT,
            "archive_segments": 100,
        },
        "cases": [
            case_json("knowledge_schema_100_registration", &schema_registration, options.mode),
            case_json("addressed_dispatch_10000_plan", &addressed_dispatch, options.mode),
            case_json("explicit_audience_10000_plan", &explicit_audience, options.mode),
            case_json("mixed_lineage_1000_validate", &mixed_lineage, options.mode),
            case_json("access_100000_index_build", &access_index_build, options.mode),
            case_json("access_100000_query", &access_scan, options.mode),
            case_json("access_100000_1000_holder_queries", &access_holder_queries, options.mode),
            case_json("archive_provider_100_segment_build", &archive_build, options.mode),
            case_json("archive_provider_100_segment_restore", &archive_restore, options.mode),
        ],
        "peak_rss_checkpoints": rss_checkpoints,
        "notes": [
            "The 100-schema case measures public plugin registration, schema validation, canonical ordering, registry ownership, and identity commitment refresh.",
            "The 1,000 lineage nodes are validated across four bounded operations because the public operation contract deliberately caps one operation at 256 output slots.",
            "The 100,000 access workload is distributed evenly across exactly 1,000 holders; the holder-query case executes one public filtered query per holder and validates 100 records per holder.",
            "The access workloads build and query the detached public InformationRecordSet index; they do not bypass the published record wire shape.",
            "Peak RSS checkpoints are cumulative process high-water marks and must not be subtracted as isolated case allocations.",
        ],
    }))
}

fn build_archive_replay_fixture(
    segment_count: usize,
) -> Result<ArchiveReplayFixture, Box<dyn Error>> {
    let scenario = profile_scenario(1)?;
    let mut simulation = Canwu::new(PROFILE_SEED, scenario)?;
    simulation.register_plugin(&ProfilePlugin)?;
    let mut compacted = simulation.into_compacted()?;
    let archive = MemoryArchive::default();
    for index in 0..segment_count {
        let day = i64::try_from(index.checked_add(1).ok_or("archive day overflow")?)?;
        compacted.settle_boundary(
            BoundaryRequest::at(canwu_api::SimTime::EPOCH + SimDuration::days(day))
                .with_cadence(SystemCadence::Daily),
        )?;
        compacted.settle_boundary(BoundaryRequest::at(compacted.time()))?;
        let prepared = compacted
            .prepare_evidence_seal()?
            .ok_or_else(|| benchmark_error("archive workload produced an empty evidence tail"))?;
        if archive.store_evidence_segment(&prepared.segment)? != ArchiveStoreOutcome::Stored {
            return Err("archive workload produced a duplicate segment".into());
        }
        compacted.commit_evidence_seal(&prepared.token, &archive)?;
    }
    let segments = archive.segments();
    if segments.len() != segment_count {
        return Err(format!(
            "archive provider retained {} segments instead of {segment_count}",
            segments.len()
        )
        .into());
    }
    Ok(ArchiveReplayFixture {
        checkpoint: compacted.checkpoint()?,
        segments,
    })
}

fn rss_checkpoint(after: &str) -> Value {
    json!({
        "after": after,
        "sample": peak_rss::sample().to_json(),
    })
}

fn run_information_flow(options: &Options) -> Result<(), Box<dyn Error>> {
    let source_before = source_metadata()?;
    let process_peak_rss_before = peak_rss::sample().to_json();
    let publication_batches = measure_publication_batches(options)?;
    let stress_workloads = measure_information_stress(options)?;
    let mut scale_reports = Vec::new();
    for &scale in &options.scales {
        eprintln!("profiling information-flow scale {scale}");
        let growth = measure_case(
            options.mode,
            options.warmup,
            options.growth_samples,
            || Ok::<_, Box<dyn Error>>(()),
            |_| build_information_fixture(scale),
        )?;
        let InformationFixture { simulation, plugin } = build_information_fixture(scale)?;
        validate_information_counts(&simulation, scale)?;
        let inputs = InformationBenchmarkInputs::new(scale)?;
        let checkpoint_hash = simulation.checkpoint_hash().to_owned();

        let snapshot_json = simulation.snapshot_json()?;
        let snapshot_size_bytes = snapshot_json.len();
        fs::write(&inputs.snapshot, snapshot_json.as_bytes())?;
        drop(snapshot_json);

        let snapshot = simulation.snapshot();
        let history = information_history_json(&snapshot, snapshot_size_bytes, &checkpoint_hash);
        drop(snapshot);

        let checkpoint_journal_bytes = simulation.checkpoint_journal_json()?.len();
        let journal = simulation.replay_journal();
        let journal_file = File::create(&inputs.journal)?;
        let mut journal_writer = BufWriter::new(journal_file);
        serde_json::to_writer(&mut journal_writer, &journal)?;
        journal_writer.flush()?;
        drop(journal_writer);
        drop(journal);
        drop(simulation);

        let hot_holder = KnowledgeHolderRef::Person(PersonId::new(1));
        let boundary_count = scale.div_ceil(KnowledgeLimitsV1::CURRENT.records_per_boundary);
        let delta_cutoff = if boundary_count > 1 {
            Some(canwu_api::SimTime::EPOCH + SimDuration::days(i64::try_from(boundary_count - 1)?))
        } else {
            None
        };

        let snapshot_create = measure_case(
            options.mode,
            options.warmup,
            options.samples,
            || restore_information_file(&inputs.snapshot, &plugin),
            |simulation| Ok::<_, Box<dyn Error>>(simulation.snapshot()),
        )?;
        let snapshot_serialize = measure_case(
            options.mode,
            options.warmup,
            options.samples,
            || {
                let file = File::open(&inputs.snapshot)?;
                Ok::<canwu_api::SimulationSnapshot, Box<dyn Error>>(serde_json::from_reader(
                    BufReader::new(file),
                )?)
            },
            |snapshot| Ok::<_, Box<dyn Error>>(serde_json::to_string_pretty(snapshot)?),
        )?;
        let current_heads_first_page = measure_case(
            options.mode,
            options.warmup,
            options.samples,
            || restore_information_file(&inputs.snapshot, &plugin),
            |simulation| {
                Ok::<_, Box<dyn Error>>(simulation.admin_query_knowledge(
                    hot_holder.clone(),
                    &KnowledgeQuery {
                        view: KnowledgeHistoryView::CurrentHeads,
                        limit: KnowledgeLimitsV1::CURRENT.max_page_size,
                        ..KnowledgeQuery::default()
                    },
                )?)
            },
        )?;
        let full_history_hot_holder = measure_case(
            options.mode,
            options.warmup,
            options.samples,
            || restore_information_file(&inputs.snapshot, &plugin),
            |simulation| {
                Ok::<_, Box<dyn Error>>(query_holder_pages(
                    simulation,
                    hot_holder.clone(),
                    KnowledgeHistoryView::FullHistory,
                    None,
                )?)
            },
        )?;
        let delta_hot_holder = measure_case(
            options.mode,
            options.warmup,
            options.samples,
            || restore_information_file(&inputs.snapshot, &plugin),
            |simulation| {
                Ok::<_, Box<dyn Error>>(query_holder_pages(
                    simulation,
                    hot_holder.clone(),
                    KnowledgeHistoryView::FullHistory,
                    delta_cutoff,
                )?)
            },
        )?;
        let snapshot_load_validate = measure_case(
            options.mode,
            options.warmup,
            options.samples,
            || Ok::<_, Box<dyn Error>>(fs::read_to_string(&inputs.snapshot)?),
            |snapshot_json| Ok::<_, Box<dyn Error>>(restore_information(snapshot_json, &plugin)?),
        )?;
        let checkpoint_journal = measure_case(
            options.mode,
            options.warmup,
            options.samples,
            || restore_information_file(&inputs.snapshot, &plugin),
            |simulation| Ok::<_, Box<dyn Error>>(simulation.checkpoint_journal_json()?),
        )?;
        let compact_initial_seal = measure_case(
            options.mode,
            options.warmup,
            options.samples,
            || {
                let mut simulation = restore_information_file(&inputs.snapshot, &plugin)?;
                simulation.settle_boundary(BoundaryRequest::at(simulation.time()))?;
                Ok::<_, Box<dyn Error>>(Some(simulation))
            },
            |slot| {
                let mut compact = slot
                    .take()
                    .ok_or_else(|| benchmark_error("compact input was already consumed"))?
                    .into_compacted()?;
                Ok::<_, Box<dyn Error>>(compact.seal_evidence()?.ok_or_else(|| {
                    benchmark_error("compact information evidence tail was empty")
                })?)
            },
        )?;
        let exact_replay = measure_case(
            options.mode,
            options.warmup,
            options.samples,
            || {
                let file = File::open(&inputs.journal)?;
                let journal: ReplayJournal = serde_json::from_reader(BufReader::new(file))?;
                Ok::<_, Box<dyn Error>>(journal)
            },
            |journal| Ok::<_, Box<dyn Error>>(Canwu::replay_from_journal(&[&plugin], journal)?),
        )?;
        let replay_ns = median(&exact_replay.elapsed_ns);
        let replay_records_per_second = if replay_ns == 0 {
            0
        } else {
            u64::try_from((u128::try_from(scale)? * 1_000_000_000_u128) / u128::from(replay_ns))
                .unwrap_or(u64::MAX)
        };

        scale_reports.push(json!({
            "scale": scale,
            "history": history,
            "checkpoint_storage": {
                "full_checkpoint_journal_bytes": checkpoint_journal_bytes,
            },
            "derived": {
                "exact_replay_records_per_second_median": replay_records_per_second,
            },
            "process_peak_rss_after_scale": peak_rss::sample().to_json(),
            "cases": [
                case_json("knowledge_history_growth", &growth, options.mode),
                case_json("knowledge_snapshot_create", &snapshot_create, options.mode),
                case_json("knowledge_snapshot_serialize_pretty", &snapshot_serialize, options.mode),
                case_json("knowledge_current_heads_first_page", &current_heads_first_page, options.mode),
                case_json("knowledge_full_history_hot_holder_paged", &full_history_hot_holder, options.mode),
                case_json("knowledge_delta_hot_holder_paged", &delta_hot_holder, options.mode),
                case_json("knowledge_snapshot_load_validate", &snapshot_load_validate, options.mode),
                case_json("knowledge_exact_replay", &exact_replay, options.mode),
                case_json("knowledge_checkpoint_journal_serialize", &checkpoint_journal, options.mode),
                case_json("knowledge_compact_initial_seal", &compact_initial_seal, options.mode),
            ],
        }));
    }

    let source_after = source_metadata()?;
    if source_before != source_after {
        return Err("benchmark source changed while the information-flow suite was running".into());
    }

    let report = json!({
        "schema_version": 4,
        "benchmark": options.suite.report_name(),
        "suite": options.suite.as_str(),
        "preset": options.information_preset.as_str(),
        "metric_mode": options.mode.as_str(),
        "recorded_on": options.recorded_on,
        "machine": options.machine,
        "engine_version": ENGINE_VERSION,
        "snapshot_format_version": SNAPSHOT_FORMAT_VERSION,
        "source": source_before,
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
            "records_per_boundary_limit": KnowledgeLimitsV1::CURRENT.records_per_boundary,
            "records_per_batch_limit": KnowledgeLimitsV1::CURRENT.records_per_batch,
            "hot_holder_share": "approximately one half of every boundary publication",
            "cold_holder_count": INFORMATION_HOLDER_COUNT - 1,
            "allocation_scope": if options.mode == MetricMode::Allocations { "thread-local allocator calls and requested bytes during the operation; setup and post-measurement drops are excluded" } else { "not collected in this uninstrumented build" },
            "elapsed_scope": if options.mode == MetricMode::Elapsed { "wall-clock nanoseconds in a build with the default system allocator; setup and post-measurement drops are excluded" } else { "not collected in the allocation-instrumented build" },
        },
        "publication_batches": publication_batches,
        "extension_workloads": stress_workloads,
        "process_peak_rss_before": process_peak_rss_before,
        "process_peak_rss_after": peak_rss::sample().to_json(),
        "limitations": [
            "peak RSS is available on Windows, Linux, and macOS as a process-lifetime high-water mark; unsupported targets report an explicit null sample and reason",
            "the kernel publisher contract caps one system at 64 batches per boundary, so the kernel growth profile uses one hot holder and up to 59 additional holders; separate extension cases cover 10,000-recipient and 10,000-member fan-out validation",
            "the access workload measures detached public index construction and query; authoritative admission of 100,000 individual access operations would also include operation and boundary overhead and is not represented by this case",
        ],
        "scales": scale_reports,
    });
    write_report(&report, options.mode, options.output.as_deref())
}

fn measure_publication_batches(options: &Options) -> Result<Vec<Value>, Box<dyn Error>> {
    let mut reports = Vec::new();
    for record_count in [1_usize, 10, 100, 1_000] {
        let measurements = measure_case(
            options.mode,
            options.warmup,
            options.samples,
            || build_information_runtime(record_count),
            |fixture| {
                let receipt = settle_information_boundary(&mut fixture.simulation, 0)?;
                if receipt.knowledge_record_count != record_count {
                    return Err(format!(
                        "publication case expected {record_count} records but observed {}",
                        receipt.knowledge_record_count
                    )
                    .into());
                }
                Ok::<_, Box<dyn Error>>(receipt)
            },
        )?;
        reports.push(json!({
            "record_count": record_count,
            "case": case_json(
                &format!("knowledge_publication_{record_count}_records"),
                &measurements,
                options.mode,
            ),
        }));
    }
    Ok(reports)
}

fn build_information_runtime(target_records: usize) -> Result<InformationFixture, Box<dyn Error>> {
    if target_records == 0 {
        return Err("information-flow scale must be greater than zero".into());
    }
    let scenario = information_scenario()?;
    let plugin = InformationFlowPlugin { target_records };
    let mut simulation = Canwu::new(INFORMATION_SEED, scenario.clone())?;
    simulation.register_plugin(&plugin)?;
    Ok(InformationFixture { simulation, plugin })
}

fn build_information_fixture(target_records: usize) -> Result<InformationFixture, Box<dyn Error>> {
    let mut fixture = build_information_runtime(target_records)?;
    let boundary_count = target_records.div_ceil(KnowledgeLimitsV1::CURRENT.records_per_boundary);
    let mut published = 0_usize;
    for boundary_index in 0..boundary_count {
        published = published
            .checked_add(
                settle_information_boundary(&mut fixture.simulation, boundary_index)?
                    .knowledge_record_count,
            )
            .ok_or("published knowledge count overflow")?;
    }
    if published != target_records {
        return Err(format!(
            "information-flow fixture expected {target_records} published records but observed {published}"
        )
        .into());
    }
    Ok(fixture)
}

fn settle_information_boundary(
    simulation: &mut Canwu,
    boundary_index: usize,
) -> Result<canwu_api::BoundaryReceipt, Box<dyn Error>> {
    let day = i64::try_from(
        boundary_index
            .checked_add(1)
            .ok_or("boundary day overflow")?,
    )?;
    Ok(simulation.settle_boundary(
        BoundaryRequest::at(canwu_api::SimTime::EPOCH + SimDuration::days(day))
            .with_cadence(SystemCadence::Daily),
    )?)
}

fn information_scenario() -> Result<Scenario, Box<dyn Error>> {
    let government = GovernmentId::new(1);
    let territory = TerritoryId::new(1);
    let mut people = Vec::with_capacity(INFORMATION_HOLDER_COUNT);
    for index in 1..=INFORMATION_HOLDER_COUNT {
        let id = checked_u64(index, "information holder ID")?;
        people.push(Person {
            id: PersonId::new(id),
            name: format!("Profile Holder {id}"),
            government,
            current_location: territory,
            roles: vec!["profile_holder".to_owned()],
            transit: None,
        });
    }
    let world = WorldSnapshot {
        people,
        governments: vec![Government {
            id: government,
            name: "Profile Institution".to_owned(),
            capital: territory,
        }],
        territories: vec![Territory {
            id: territory,
            name: "Profile Location".to_owned(),
            controller: government,
            position: MapPoint { x: 0.0, y: 0.0 },
        }],
        routes: Vec::new(),
        armies: Vec::new(),
        letters: Vec::new(),
    };
    Ok(Scenario {
        start_time: canwu_api::SimTime::EPOCH,
        entities: world.entities(),
        world,
        knowledge: KnowledgeSnapshot::default(),
        domain_records: Vec::new(),
    })
}

fn validate_information_counts(
    simulation: &Canwu,
    target_records: usize,
) -> Result<(), Box<dyn Error>> {
    let observed_records = knowledge_record_count(simulation.knowledge());
    let expected_boundaries =
        target_records.div_ceil(KnowledgeLimitsV1::CURRENT.records_per_boundary);
    if observed_records != target_records || simulation.boundaries().len() != expected_boundaries {
        return Err(format!(
            "information-flow fixture diverged at scale {target_records}: records={observed_records}, boundaries={}",
            simulation.boundaries().len(),
        )
        .into());
    }
    Ok(())
}

fn knowledge_record_count(knowledge: &KnowledgeSnapshot) -> usize {
    knowledge.records.values().map(BTreeMap::len).sum()
}

fn query_holder_pages(
    simulation: &Canwu,
    holder: KnowledgeHolderRef,
    view: KnowledgeHistoryView,
    learned_after: Option<canwu_api::SimTime>,
) -> Result<usize, CanwuError> {
    let mut query = KnowledgeQuery {
        learned_after,
        view,
        limit: KnowledgeLimitsV1::CURRENT.max_page_size,
        ..KnowledgeQuery::default()
    };
    let mut records = 0_usize;
    loop {
        let result = simulation.admin_query_knowledge(holder.clone(), &query)?;
        records = records.saturating_add(result.records.len());
        let Some(next) = result.next else {
            return Ok(records);
        };
        query.after = Some(next);
    }
}

fn restore_information(
    snapshot_json: &str,
    plugin: &InformationFlowPlugin,
) -> Result<Canwu, CanwuError> {
    Canwu::from_snapshot_json_with_plugins(snapshot_json, &[plugin])
}

fn restore_information_file(
    snapshot_path: &Path,
    plugin: &InformationFlowPlugin,
) -> Result<Canwu, Box<dyn Error>> {
    let snapshot_json = fs::read_to_string(snapshot_path)?;
    Ok(restore_information(&snapshot_json, plugin)?)
}

fn information_history_json(
    snapshot: &canwu_api::SimulationSnapshot,
    snapshot_size_bytes: usize,
    checkpoint_hash: &str,
) -> Value {
    let total_records = knowledge_record_count(&snapshot.knowledge);
    let hot_holder_records = snapshot
        .knowledge
        .records
        .get(&KnowledgeHolderRef::Person(PersonId::new(1)))
        .map_or(0, BTreeMap::len);
    json!({
        "world_entities": snapshot.world.people.len()
            + snapshot.world.governments.len()
            + snapshot.world.territories.len()
            + snapshot.world.routes.len()
            + snapshot.world.armies.len(),
        "domain_records": snapshot.domain_records.len(),
        "plugin_components": snapshot.plugin_components.len(),
        "commands": snapshot.commands.len(),
        "command_attempts": snapshot.command_attempts.len(),
        "ingress": snapshot.ingress.len(),
        "events": snapshot.events.len(),
        "boundaries": snapshot.boundaries.len(),
        "random_draws": snapshot.random_draws.len(),
        "knowledge_holders": snapshot.knowledge.records.len(),
        "knowledge_records": total_records,
        "hot_holder_records": hot_holder_records,
        "snapshot_size_bytes": snapshot_size_bytes,
        "checkpoint_hash": checkpoint_hash,
    })
}

fn restore(snapshot_json: &str) -> Result<Canwu, CanwuError> {
    Canwu::from_snapshot_json_with_plugins(snapshot_json, &[&ProfilePlugin])
}

fn prepare_live_archive(snapshot_json: &str) -> Result<Canwu, CanwuError> {
    let mut simulation = restore(snapshot_json)?;
    simulation.settle_boundary(BoundaryRequest::at(simulation.time()))?;
    Ok(simulation)
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
    Ok(GrowthFixture { simulation })
}

fn build_compacted_growth(scale: usize) -> Result<(), Box<dyn Error>> {
    let scenario = profile_scenario(scale)?;
    let mut simulation = Canwu::new(PROFILE_SEED, scenario)?;
    simulation.register_plugin(&ProfilePlugin)?;
    let mut simulation = simulation.into_compacted()?;
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
            return Err(format!("compacted accepted command {index} was rejected").into());
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
            return Err(format!("compacted rejected command {index} was accepted").into());
        }
        simulation.settle_boundary(
            BoundaryRequest::at(simulation.time()).with_cadence(SystemCadence::Daily),
        )?;
        simulation.settle_boundary(BoundaryRequest::at(simulation.time()))?;
        let segment = simulation
            .seal_evidence()?
            .ok_or_else(|| benchmark_error("compacted growth tail was empty"))?;
        black_box(&segment);
        drop(segment);
    }
    Ok(())
}

fn build_repeated_seal_fixture(scale: usize) -> Result<(), Box<dyn Error>> {
    let scenario = profile_scenario(1)?;
    let simulation = Canwu::new(PROFILE_SEED, scenario)?;
    let mut simulation = simulation.into_compacted()?;
    for index in 0..scale {
        let request_id = checked_u64(
            index.checked_add(1).ok_or("request ID overflow")?,
            "request ID",
        )?;
        let outcome = simulation.process_command(CommandRequest::new(
            CommandRequestId::new(request_id),
            simulation.revision(),
            CommandEnvelope::new(
                Issuer::Debug,
                Command::DebugSetArmyMorale {
                    army: ArmyId::new(1),
                    morale: u16::try_from(index % 101)?,
                },
            ),
        ))?;
        if !matches!(outcome, CommandOutcome::Accepted { .. }) {
            return Err(format!("repeated-seal command {index} was rejected").into());
        }
        simulation.settle_boundary(BoundaryRequest::at(simulation.time()))?;
        let segment = simulation
            .seal_evidence()?
            .ok_or_else(|| benchmark_error("repeated-seal tail was empty"))?;
        black_box(&segment);
        drop(segment);
    }
    Ok(())
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
            transit: None,
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
    let world = WorldSnapshot {
        people,
        governments: vec![Government {
            id: government_id,
            name: "Profile Government".to_owned(),
            capital: TerritoryId::new(1),
        }],
        territories,
        routes: Vec::new(),
        armies,
        letters: Vec::new(),
    };
    Ok(Scenario {
        start_time: canwu_api::SimTime::EPOCH,
        entities: world.entities(),
        world,
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
                "elapsed_ns_p95": percentile_95(&measurements.elapsed_ns),
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
            let net_retained_bytes_delta: Vec<_> = measurements
                .allocations
                .iter()
                .map(|sample| {
                    i128::from(sample.allocated_bytes) - i128::from(sample.deallocated_bytes)
                })
                .collect();
            json!({
                "name": name,
                "summary": {
                    "alloc_calls_median": median(&alloc_calls),
                    "realloc_calls_median": median(&realloc_calls),
                    "allocation_operations_median": median(&allocation_operations),
                    "allocated_bytes_median": median(&allocated_bytes),
                    "net_retained_bytes_delta_median": median_i128(&net_retained_bytes_delta),
                },
                "allocation_samples": measurements.allocations.iter().map(|sample| json!({
                    "alloc_calls": sample.alloc_calls,
                    "realloc_calls": sample.realloc_calls,
                    "allocation_operations": sample.alloc_calls.saturating_add(sample.realloc_calls),
                    "dealloc_calls": sample.dealloc_calls,
                    "allocated_bytes": sample.allocated_bytes,
                    "deallocated_bytes": sample.deallocated_bytes,
                    "net_retained_bytes_delta": i128::from(sample.allocated_bytes) - i128::from(sample.deallocated_bytes),
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

fn percentile_95(values: &[u64]) -> u64 {
    if values.is_empty() {
        return 0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let rank = sorted.len().saturating_mul(95).div_ceil(100);
    sorted.get(rank.saturating_sub(1)).copied().unwrap_or(0)
}

fn median_i128(values: &[i128]) -> i128 {
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
            "crates/api/canwu-api/src",
            "crates/foundation/canwu-core/src",
            "crates/model/canwu-event/src",
            "crates/model/canwu-knowledge/src",
            "crates/runtime/canwu-sim/src",
            "crates/foundation/canwu-time/src",
            "crates/integrations/canwu-reference-world/src",
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
    ];
    collect_source_files(&root.join("benchmarks/performance-harness/src"), &mut paths)?;
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
        suite: BenchmarkSuite::Architecture,
        information_preset: InformationPreset::Smoke,
        mode: MetricMode::Elapsed,
        scales: DEFAULT_SCALES.to_vec(),
        samples: DEFAULT_SAMPLES,
        growth_samples: DEFAULT_GROWTH_SAMPLES,
        warmup: DEFAULT_WARMUP,
        output: None,
        machine: "local-machine".to_owned(),
        recorded_on: "unspecified".to_owned(),
    };
    let mut scales_explicit = false;
    let mut arguments = env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--suite" => {
                options.suite = match arguments
                    .next()
                    .ok_or("--suite requires architecture or information-flow")?
                    .as_str()
                {
                    "architecture" => BenchmarkSuite::Architecture,
                    "information-flow" => BenchmarkSuite::InformationFlow,
                    value => return Err(format!("unsupported benchmark suite: {value}").into()),
                };
            }
            "--preset" => {
                options.information_preset = match arguments
                    .next()
                    .ok_or("--preset requires smoke or full")?
                    .as_str()
                {
                    "smoke" => InformationPreset::Smoke,
                    "full" => InformationPreset::Full,
                    value => return Err(format!("unsupported information preset: {value}").into()),
                };
            }
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
                scales_explicit = true;
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
    if options.suite == BenchmarkSuite::InformationFlow && !scales_explicit {
        options.scales = options.information_preset.scales().to_vec();
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
           --suite SUITE           architecture or information-flow\n\
           --preset PRESET         information-flow smoke or full scale preset\n\
           --mode MODE             elapsed or allocations\n\
           --scales LIST           Override comma-separated workload scales\n\
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
        let replayed = Canwu::replay_from_journal(&[&ProfilePlugin], &journal)
            .expect("fixture should replay exactly");
        assert_eq!(replayed.snapshot(), second.simulation.snapshot());
    }

    #[test]
    fn information_fixture_is_queryable_and_exactly_replayable() {
        let fixture = build_information_fixture(1_000).expect("information fixture should build");
        validate_information_counts(&fixture.simulation, 1_000)
            .expect("information fixture counts should match");
        let hot_records = query_holder_pages(
            &fixture.simulation,
            KnowledgeHolderRef::Person(PersonId::new(1)),
            KnowledgeHistoryView::FullHistory,
            None,
        )
        .expect("hot holder query should succeed");
        assert_eq!(hot_records, 500);

        let journal = fixture.simulation.replay_journal();
        let replayed = Canwu::replay_from_journal(&[&fixture.plugin], &journal)
            .expect("information fixture should replay exactly");
        assert_eq!(replayed.snapshot(), fixture.simulation.snapshot());
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
