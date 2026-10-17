//! World-scale fiscal profile.
//!
//! Builds a generated fiscal world through the public API, deterministic from
//! a seed, and measures the boundaries a world-scale campaign settles on the
//! partitioned fiscal records: idle boundaries, assessing bursts of up to
//! every scope at once, receipts, a close of every book, the report
//! publication and drains those boundaries cause, and archive commits. It
//! also records what the world costs to hold: fiscal record bytes per kind,
//! the largest record, snapshot bytes, activation, fork, and restore time,
//! and the host's audit of every fiscal record.
//!
//! The generator splits the scopes over partitions with a heavy-tailed size
//! (the largest about 6,000 scopes in the `world` preset), each partition
//! with one region of the catalog, three central institutions and eighteen
//! regional offices, and twenty observers: one seeing every institution of
//! its partition, and one for each of nineteen institutions. Identifiers are
//! generated (`scope.000017`, `authority.0003`).
//!
//! Every boundary of the last sample of each measurement is followed, outside
//! the timed part, by `audit_fiscal_records`, so a profile is also a check of
//! the cross-record invariants.
//!
//! `--mode allocations`, in a build with the `allocation-counting` feature,
//! counts allocator calls instead of timing, and reports
//! `*_allocation_operations` where the elapsed build reports `*_ms`.

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundaryReceipt,
    BoundaryRequest, BoundarySystemContract, Canwu, CanwuError, CanwuRun, CommandEnvelope,
    CommandRequest, CommandRequestId, CompactedCanwu, DomainRecordDraft, DomainRecordKind,
    DomainRecordMutation, DomainRecordSchema, DomainRecordType, DomainValueKindClass, EntityRef,
    Government, GovernmentId, IngressClass, IngressPayload, Issuer, KnowledgeHolderRef,
    KnowledgeSnapshot, MapPoint, OrganizationId, PayloadSchema, Person, PersonId,
    PluginIngressDescriptor, PluginIngressRequest, PluginRegistrar, ResourceId, Scenario,
    SimDuration, SimTime, SimulationGranularity, SimulationPlugin, SimulationView, StateKey,
    StateVisibility, SystemCadence, Territory, TerritoryId, TypedDomainRecordRef, WorldSnapshot,
};
use canwu_fiscal::{
    CompiledFiscalCatalog, FiscalAction, FiscalActionBatch, FiscalActionDisposition,
    FiscalActionEntry, FiscalActionTarget, FiscalAdoptionStage, FiscalAdoptionState,
    FiscalArchiveError, FiscalArchiveRetentionHandleV1, FiscalArchiveRetentionPhaseV1,
    FiscalArchiveStore, FiscalAssessmentBasis, FiscalAuthorityBinding, FiscalCommutationPolicy,
    FiscalContentPack, FiscalContentSelection, FiscalCoverageDeclaration, FiscalCoverageSelector,
    FiscalCoverageStatus, FiscalExecutionEvidence, FiscalExecutionKind,
    FiscalExecutionReceiptPacket, FiscalHistoricalMode, FiscalMechanism, FiscalObserverBinding,
    FiscalPackManifest, FiscalPaymentForm, FiscalPeriodDefinition, FiscalPlugin, FiscalProvenance,
    FiscalReceiptDisposition, FiscalRegionDefinition, FiscalReportCadence, FiscalReportGranularity,
    FiscalRuleDefinition, FiscalScopeBinding, FiscalSettledAction, FiscalState,
    HistoricalConfidence, HistoricalYearWindow, MAX_FISCAL_BATCH_ENTRIES,
    MAX_FISCAL_RECEIPTS_PER_PACKET, PLUGIN_NAME, audit_fiscal_records, compile_fiscal_content,
    enqueue_execution_receipts, enqueue_fiscal_archive, finalize_fiscal_archive_retention,
    fiscal_action_command, fiscal_target_revision, prepare_fiscal_archive,
};
use serde_json::{Value, json};
#[cfg(feature = "allocation-counting")]
use std::alloc::{GlobalAlloc, Layout, System};
#[cfg(feature = "allocation-counting")]
use std::cell::Cell;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

const SUPPORT_PLUGIN: &str = "fiscal-world-support";
const TICK_INGRESS: &str = "tick_v1";
const EVIDENCE_INGRESS: &str = "evidence_v1";
const RULE: &str = "rule.land";
const CYCLE: &str = "cycle.0001";
const CENTRAL_INSTITUTIONS: usize = 3;
const REGIONAL_OFFICES: usize = 18;
const INSTITUTIONS_PER_PARTITION: usize = CENTRAL_INSTITUTIONS + REGIONAL_OFFICES;
const OBSERVERS_PER_PARTITION: usize = 20;
/// Evidence records one support packet creates.
const EVIDENCE_PER_PACKET: usize = 1_024;

type Fallible<T> = Result<T, Box<dyn Error>>;

/// An external execution result, which a fiscal receipt cites as evidence.
struct ExecutionResult;

impl DomainRecordType for ExecutionResult {
    type Payload = FiscalExecutionEvidence;
    type Class = DomainValueKindClass;

    const NAMESPACE: &'static str = "profile.execution";
    const NAME: &'static str = "result";
}

/// Owns a tick, which makes a boundary event-bearing without touching fiscal
/// state, and the recording of execution results.
struct SupportPlugin;

impl SimulationPlugin for SupportPlugin {
    fn name(&self) -> &'static str {
        SUPPORT_PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "00000000000000000000000000000000000000000000000000000000000000f2"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_record_schema(DomainRecordSchema::for_record::<ExecutionResult>())?;
        for (name, description) in [
            (TICK_INGRESS, "Make one boundary event-bearing"),
            (EVIDENCE_INGRESS, "Record the listed execution results"),
        ] {
            registrar.register_ingress(PluginIngressDescriptor {
                name: name.to_owned(),
                description: description.to_owned(),
                class: IngressClass::Information,
                payload_schema: PayloadSchema::Any,
            })?;
        }
        let key = DomainRecordSchema::for_record::<ExecutionResult>().state_key();
        let mut record = BoundarySystemContract::new(
            "record-results-v1",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        record.reads = vec![StateKey::core_ingress(), key.clone()];
        record.reads.sort();
        record.writes = vec![key];
        record.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(record, record_results)
    }
}

fn record_results(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut directives = Vec::new();
    for ingress_id in &context.admitted_ingress {
        if let Some(ingress) = view.ingress(*ingress_id)?
            && let IngressPayload::Plugin {
                plugin,
                packet_type,
                payload,
                ..
            } = &ingress.payload
            && plugin == SUPPORT_PLUGIN
            && packet_type == EVIDENCE_INGRESS
        {
            let results = serde_json::from_value::<Vec<FiscalExecutionEvidence>>(payload.clone())
                .map_err(|error| {
                CanwuError::new(canwu_api::ErrorCode::InvalidPayload, error.to_string())
            })?;
            for result in results {
                directives.push(BoundaryDirective::MutateRecord {
                    mutation: DomainRecordMutation::Create {
                        record: DomainRecordDraft::from_typed(
                            TypedDomainRecordRef::<ExecutionResult>::new(&result.id),
                            &result,
                        )?,
                    },
                    summary: "Record one execution result".to_owned(),
                });
            }
        }
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

#[cfg(feature = "allocation-counting")]
struct CountingAllocator;

#[cfg(feature = "allocation-counting")]
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[cfg(feature = "allocation-counting")]
thread_local! {
    /// Whether this thread counts, and the allocator calls counted so far.
    static ALLOCATIONS: Cell<(bool, u64)> = const { Cell::new((false, 0)) };
    /// While counting, the bytes allocated less those freed since the
    /// measurement started, and the most they reached: the heap peak above
    /// the start.
    static HEAP: Cell<(i64, i64)> = const { Cell::new((0, 0)) };
}

#[cfg(feature = "allocation-counting")]
fn count_allocation() {
    let _ = ALLOCATIONS.try_with(|state| {
        let (counting, calls) = state.get();
        if counting {
            state.set((true, calls.saturating_add(1)));
        }
    });
}

/// Adds `bytes` (negative when freed) to the heap of the measurement in
/// progress.
#[cfg(feature = "allocation-counting")]
fn count_heap(bytes: i64) {
    let counting = ALLOCATIONS.try_with(|state| state.get().0).unwrap_or(false);
    if counting {
        let _ = HEAP.try_with(|heap| {
            let (live, peak) = heap.get();
            let live = live.saturating_add(bytes);
            heap.set((live, peak.max(live)));
        });
    }
}

/// The heap peak above the start of the last measurement, in bytes; zero in
/// an elapsed build.
#[allow(clippy::cast_precision_loss)]
fn heap_peak() -> f64 {
    #[cfg(feature = "allocation-counting")]
    {
        return HEAP.with(|heap| heap.get().1) as f64;
    }
    #[allow(unreachable_code)]
    0.0
}

// SAFETY: every operation delegates to `System` with the original pointer and
// layout; the thread-local counter only observes allocator calls.
#[cfg(feature = "allocation-counting")]
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count_allocation();
        count_heap(i64::try_from(layout.size()).unwrap_or(i64::MAX));
        // SAFETY: the caller supplies the layout required by `GlobalAlloc`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count_allocation();
        count_heap(i64::try_from(layout.size()).unwrap_or(i64::MAX));
        // SAFETY: the caller supplies the layout required by `GlobalAlloc`.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        count_heap(-i64::try_from(layout.size()).unwrap_or(i64::MAX));
        // SAFETY: the caller returns the pointer with its original layout.
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count_allocation();
        count_heap(
            i64::try_from(new_size)
                .unwrap_or(i64::MAX)
                .saturating_sub(i64::try_from(layout.size()).unwrap_or(i64::MAX)),
        );
        // SAFETY: the caller supplies the pointer, original layout, and size.
        unsafe { System.realloc(pointer, layout, new_size) }
    }
}

/// Set once from `--mode`: count allocator calls instead of timing.
static COUNT_ALLOCATIONS: AtomicBool = AtomicBool::new(false);

/// One measurement in progress: wall time in milliseconds, or allocator
/// calls in allocation mode.
struct Meter(Instant);

impl Meter {
    fn start() -> Self {
        #[cfg(feature = "allocation-counting")]
        if COUNT_ALLOCATIONS.load(Ordering::Relaxed) {
            HEAP.with(|heap| heap.set((0, 0)));
            ALLOCATIONS.with(|state| state.set((true, 0)));
        }
        Self(Instant::now())
    }

    #[allow(clippy::cast_precision_loss)]
    fn finish(self) -> f64 {
        #[cfg(feature = "allocation-counting")]
        if COUNT_ALLOCATIONS.load(Ordering::Relaxed) {
            let (_, calls) = ALLOCATIONS.with(|state| state.replace((false, 0)));
            return calls as f64;
        }
        self.0.elapsed().as_secs_f64() * 1_000.0
    }
}

fn counting() -> bool {
    COUNT_ALLOCATIONS.load(Ordering::Relaxed)
}

/// In allocation mode, renames every `*_ms` key to `*_allocation_operations`.
fn relabel(value: &mut Value) {
    match value {
        Value::Object(map) => {
            let entries = std::mem::take(map);
            for (key, mut entry) in entries {
                relabel(&mut entry);
                let key = match key.strip_suffix("_ms") {
                    Some(stem) => format!("{stem}_allocation_operations"),
                    None => key,
                };
                map.insert(key, entry);
            }
        }
        Value::Array(entries) => entries.iter_mut().for_each(relabel),
        _ => {}
    }
}

/// A fiscal archive store in memory.
#[derive(Default)]
struct MemoryArchiveStore {
    objects: RefCell<BTreeMap<(String, String), Vec<u8>>>,
    handles: RefCell<BTreeMap<String, FiscalArchiveRetentionPhaseV1>>,
}

impl FiscalArchiveStore for MemoryArchiveStore {
    fn store_fiscal_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
        bytes: &[u8],
    ) -> Result<(), FiscalArchiveError> {
        self.objects
            .borrow_mut()
            .insert((namespace.to_owned(), object_id.to_owned()), bytes.to_vec());
        Ok(())
    }

    fn load_fiscal_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, FiscalArchiveError> {
        Ok(self
            .objects
            .borrow()
            .get(&(namespace.to_owned(), object_id.to_owned()))
            .cloned())
    }

    fn persist_fiscal_archive_retention(
        &self,
        handle: &FiscalArchiveRetentionHandleV1,
    ) -> Result<(), FiscalArchiveError> {
        self.handles
            .borrow_mut()
            .insert(handle.handle_id.clone(), handle.phase);
        Ok(())
    }

    fn finalize_fiscal_archive_retention(
        &self,
        handle_id: &str,
        phase: FiscalArchiveRetentionPhaseV1,
    ) -> Result<(), FiscalArchiveError> {
        self.handles
            .borrow_mut()
            .insert(handle_id.to_owned(), phase);
        Ok(())
    }
}

/// What a preset measures.
struct Preset {
    name: &'static str,
    partitions: usize,
    scopes: usize,
    largest_partition: usize,
    bursts: Vec<(&'static str, usize)>,
    receipts: Vec<usize>,
    archive_commits: Vec<usize>,
    samples: usize,
    warmup: usize,
    idle_samples: usize,
}

fn preset(name: &str) -> Fallible<Preset> {
    match name {
        "smoke" => Ok(Preset {
            name: "smoke",
            partitions: 20,
            scopes: 2_000,
            largest_partition: 600,
            bursts: vec![
                ("64", 64),
                ("spread_day", 6),
                ("1024", 1_024),
                ("all", 2_000),
            ],
            receipts: vec![1_024],
            archive_commits: vec![256],
            samples: 5,
            warmup: 1,
            idle_samples: 31,
        }),
        "world" => Ok(Preset {
            name: "world",
            partitions: 200,
            scopes: 50_000,
            largest_partition: 6_000,
            bursts: vec![
                ("64", 64),
                ("spread_day", 137),
                ("1024", 1_024),
                ("4096", 4_096),
                ("16384", 16_384),
                ("all", 50_000),
            ],
            receipts: vec![1_024, 16_384],
            archive_commits: vec![256, 4_096],
            samples: 3,
            warmup: 1,
            idle_samples: 21,
        }),
        other => Err(format!("unknown preset {other}; use smoke or world").into()),
    }
}

/// The generated world: each partition's institutions and scopes.
struct World {
    /// The scopes of each partition, by global scope index.
    partitions: Vec<Vec<usize>>,
    /// The institution of every scope, by global institution index.
    scope_institution: Vec<usize>,
    institutions: usize,
}

/// A splitmix64 stream.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        value ^ (value >> 31)
    }

    fn below(&mut self, bound: usize) -> usize {
        usize::try_from(self.next() % u64::try_from(bound).unwrap_or(u64::MAX)).unwrap_or(0)
    }

    fn shuffle<T>(&mut self, values: &mut [T]) {
        for index in (1..values.len()).rev() {
            values.swap(index, self.below(index + 1));
        }
    }
}

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn generate(preset: &Preset, random: &mut Random) -> World {
    // Heavy-tailed partition sizes: weight (rank + 1)^-0.9, capped, at least
    // one scope for every institution.
    let weights: Vec<f64> = (0..preset.partitions)
        .map(|rank| ((rank + 1) as f64).powf(-0.9))
        .collect();
    let total: f64 = weights.iter().sum();
    let mut sizes: Vec<usize> = weights
        .iter()
        .map(|weight| {
            ((preset.scopes as f64 * weight / total) as usize)
                .clamp(INSTITUTIONS_PER_PARTITION, preset.largest_partition)
        })
        .collect();
    let mut assigned: usize = sizes.iter().sum();
    let mut cursor = 0;
    while assigned != preset.scopes {
        let index = cursor % preset.partitions;
        if assigned < preset.scopes && sizes[index] < preset.largest_partition {
            sizes[index] += 1;
            assigned += 1;
        } else if assigned > preset.scopes && sizes[index] > INSTITUTIONS_PER_PARTITION {
            sizes[index] -= 1;
            assigned -= 1;
        }
        cursor += 1;
    }
    random.shuffle(&mut sizes);
    let mut partitions = Vec::with_capacity(preset.partitions);
    let mut scope_institution = Vec::with_capacity(preset.scopes);
    for (partition, size) in sizes.into_iter().enumerate() {
        let first_scope = scope_institution.len();
        for offset in 0..size {
            scope_institution
                .push(partition * INSTITUTIONS_PER_PARTITION + offset % INSTITUTIONS_PER_PARTITION);
        }
        partitions.push((first_scope..first_scope + size).collect());
    }
    World {
        partitions,
        scope_institution,
        institutions: preset.partitions * INSTITUTIONS_PER_PARTITION,
    }
}

fn scope_id(scope: usize) -> String {
    format!("scope.{scope:06}")
}

fn authority_id(institution: usize) -> String {
    format!("authority.{institution:04}")
}

fn region_id(partition: usize) -> String {
    format!("region.{partition:03}")
}

fn principal(institution: usize) -> Fallible<PersonId> {
    Ok(PersonId::new(u64::try_from(institution)? + 1))
}

fn institution_entity(institution: usize) -> Fallible<EntityRef> {
    Ok(EntityRef::Organization(OrganizationId::new(
        u64::try_from(institution)? + 1,
    )))
}

fn catalog(partitions: usize) -> Fallible<CompiledFiscalCatalog> {
    let window = HistoricalYearWindow {
        start: 100,
        end: 200,
    };
    let provenance = BTreeSet::from(["source.profile".to_owned()]);
    let regions: Vec<String> = (0..partitions).map(region_id).collect();
    let pack = FiscalContentPack {
        manifest: FiscalPackManifest {
            pack_id: "profile.world".to_owned(),
            pack_version: "1.0.0".to_owned(),
            schema_version: 1,
            license: "Apache-2.0".to_owned(),
            title: "World fiscal profile".to_owned(),
            historical_scope: window.clone(),
            period_ids: vec!["period.a".to_owned()],
            region_ids: regions.clone(),
            mechanisms: vec![FiscalMechanism::LandTax],
        },
        periods: vec![FiscalPeriodDefinition {
            id: "period.a".to_owned(),
            label: "Profile period".to_owned(),
            window: window.clone(),
            branch: None,
        }],
        regions: regions
            .iter()
            .map(|id| FiscalRegionDefinition {
                id: id.clone(),
                label: format!("Profile {id}"),
            })
            .collect(),
        institutions: Vec::new(),
        rules: vec![FiscalRuleDefinition {
            id: RULE.to_owned(),
            revision: 1,
            label: "Land levy".to_owned(),
            mechanism: FiscalMechanism::LandTax,
            legal_window: window,
            jurisdiction_ids: regions.into_iter().collect(),
            subject_scope: "registered_land".to_owned(),
            assessment_basis: FiscalAssessmentBasis::RegisteredLand,
            payment_forms: BTreeSet::from([FiscalPaymentForm::Grain]),
            commutation: FiscalCommutationPolicy::Disabled,
            earmark_ids: BTreeSet::new(),
            provenance_ids: provenance.clone(),
            confidence: HistoricalConfidence::Medium,
        }],
        transitions: Vec::new(),
        coverage: vec![FiscalCoverageDeclaration {
            id: "coverage.a".to_owned(),
            priority: 1,
            selector: FiscalCoverageSelector {
                period_ids: BTreeSet::new(),
                region_ids: BTreeSet::new(),
                mechanisms: BTreeSet::new(),
            },
            status: FiscalCoverageStatus::Supported,
            definition_ids: BTreeSet::from([RULE.to_owned()]),
            provenance_ids: provenance,
        }],
        provenance: vec![FiscalProvenance {
            id: "source.profile".to_owned(),
            citation: "Synthetic profile content".to_owned(),
            url: "https://example.invalid/profile".to_owned(),
            claim_scope: "performance profile only".to_owned(),
            confidence: HistoricalConfidence::Medium,
            forbidden_inferences: vec!["no historical claim".to_owned()],
        }],
    };
    Ok(compile_fiscal_content(
        &pack,
        FiscalContentSelection {
            historical_year: 150,
            ..FiscalContentSelection::default()
        },
    )?)
}

/// How the generated observers are configured: the granularity of each
/// partition's court (its observer of every institution) and every
/// observer's cadence, by its index among all observers.
struct Observers {
    courts: FiscalReportGranularity,
    cadence: fn(u64) -> FiscalReportCadence,
}

/// Thirty days, the interval of the generated interval cadences.
const MONTH: SimDuration = SimDuration::days(30);

/// Monthly, observer `index` due on day `index % 30`: about one
/// thirtieth of the observers due each day.
fn staggered(index: u64) -> FiscalReportCadence {
    FiscalReportCadence::staggered(MONTH, index, 30)
}

/// Monthly, every observer due at the same instants.
fn aligned(_: u64) -> FiscalReportCadence {
    FiscalReportCadence::Interval {
        every: MONTH,
        offset: SimDuration::ZERO,
    }
}

fn on_change(_: u64) -> FiscalReportCadence {
    FiscalReportCadence::OnChange
}

fn fiscal_state(world: &World, observers: &Observers) -> Fallible<FiscalState> {
    let mut state = FiscalState::new(150, FiscalHistoricalMode::Counterfactual, SimTime::EPOCH);
    state
        .root
        .execution_evidence_kinds
        .insert(DomainRecordKind::for_type::<ExecutionResult>());
    for institution in 0..world.institutions {
        let binding: FiscalAuthorityBinding = serde_json::from_value(json!({
            "id": authority_id(institution),
            "institution": institution_entity(institution)?,
            "authorized_actor": principal(institution)?,
        }))?;
        state.add_authority_binding(binding);
    }
    for (partition, scopes) in world.partitions.iter().enumerate() {
        for &scope in scopes {
            state.add_scope(FiscalScopeBinding {
                id: scope_id(scope),
                institution: institution_entity(world.scope_institution[scope])?,
                jurisdiction_id: region_id(partition),
                subject_scope: "registered_land".to_owned(),
                mechanism: FiscalMechanism::LandTax,
                authoritative_granularity: SimulationGranularity::Aggregate,
            });
            state.add_adoption(FiscalAdoptionState {
                id: format!("adopt.{scope:06}"),
                rule_id: RULE.to_owned(),
                scope_binding_id: scope_id(scope),
                stage: FiscalAdoptionStage::Implemented,
                generation: 1,
                changed_at: SimTime::EPOCH,
                source_action_id: None,
            })?;
        }
        let first = partition * INSTITUTIONS_PER_PARTITION;
        for observer in 0..OBSERVERS_PER_PARTITION {
            let holder = principal(first + observer)?;
            let visible = if observer == 0 {
                (first..first + INSTITUTIONS_PER_PARTITION)
                    .map(institution_entity)
                    .collect::<Result<_, _>>()?
            } else {
                BTreeSet::from([institution_entity(first + observer + 1)?])
            };
            let index = u64::try_from(partition * OBSERVERS_PER_PARTITION + observer)?;
            state.add_observer(FiscalObserverBinding {
                id: format!("observer.{partition:03}.{observer:02}"),
                actor: Some(holder),
                knowledge_holder: KnowledgeHolderRef::Person(holder),
                visible_institutions: visible,
                confidence_per_mille: 850,
                granularity: if observer == 0 {
                    observers.courts
                } else {
                    FiscalReportGranularity::Scope
                },
                cadence: (observers.cadence)(index),
                controlled_by: None,
                rebound_by: None,
                removed: None,
            });
        }
    }
    Ok(state)
}

fn scenario(world: &World, records: Vec<canwu_api::DomainRecord>) -> Fallible<Scenario> {
    let government = GovernmentId::new(1);
    let territory = TerritoryId::new(1);
    let people = (0..world.institutions)
        .map(|institution| {
            Ok(Person {
                id: principal(institution)?,
                name: format!("Principal {institution}"),
                government,
                current_location: territory,
                roles: Vec::new(),
                transit: None,
            })
        })
        .collect::<Fallible<Vec<_>>>()?;
    let world_snapshot = WorldSnapshot {
        people,
        governments: vec![Government {
            id: government,
            name: "Profile authority".to_owned(),
            capital: territory,
        }],
        territories: vec![Territory {
            id: territory,
            name: "Profile district".to_owned(),
            controller: government,
            position: MapPoint::default(),
        }],
        routes: Vec::new(),
        armies: Vec::new(),
        letters: Vec::new(),
    };
    let mut entities = world_snapshot.entities();
    entities.push(EntityRef::Resource(ResourceId::new(1)));
    for institution in 0..world.institutions {
        entities.push(institution_entity(institution)?);
    }
    Ok(Scenario {
        start_time: SimTime::EPOCH,
        entities,
        world: world_snapshot,
        knowledge: KnowledgeSnapshot::default(),
        domain_records: records,
    })
}

/// Fiscal record bytes and counts per kind, and the largest record.
fn record_sizes(canwu: &Canwu) -> Fallible<Value> {
    let mut kinds: BTreeMap<String, (usize, usize, usize)> = BTreeMap::new();
    let mut largest = (0, String::new());
    for record in canwu.domain_records() {
        if record.reference.kind.namespace != "canwu.fiscal" {
            continue;
        }
        let bytes = serde_json::to_vec(&record.payload)?.len();
        let entry = kinds.entry(record.reference.kind.name.clone()).or_default();
        entry.0 += 1;
        entry.1 += bytes;
        entry.2 = entry.2.max(bytes);
        if bytes > largest.0 {
            largest = (bytes, record.reference.to_string());
        }
    }
    let total: usize = kinds.values().map(|(_, bytes, _)| bytes).sum();
    let records: usize = kinds.values().map(|(count, _, _)| count).sum();
    Ok(json!({
        "fiscal_records": records,
        "fiscal_record_json_bytes": total,
        "largest_record": { "json_bytes": largest.0, "record": largest.1 },
        "per_kind": kinds
            .into_iter()
            .map(|(kind, (count, bytes, largest))| {
                (kind, json!({ "records": count, "json_bytes": bytes, "largest_json_bytes": largest }))
            })
            .collect::<serde_json::Map<_, _>>(),
    }))
}

/// One measured boundary sequence: what it did and what each of its parts
/// cost, per sample.
#[derive(Default)]
struct Samples {
    admission: Vec<f64>,
    admission_fiscal: Vec<f64>,
    settlement: Vec<f64>,
    settlement_fiscal: Vec<f64>,
    settlement_handlers: BTreeMap<String, Vec<f64>>,
    settlement_engine: Vec<f64>,
    drains: Vec<f64>,
    host: Vec<f64>,
    audit: Vec<f64>,
    drain_boundaries: usize,
    records_published: usize,
    record_changes: usize,
    applied: usize,
    rejected: usize,
    /// In allocation mode, the heap peak of the measured boundary above its
    /// start, in bytes.
    heap_peak: Vec<f64>,
    /// What the measured boundary published, when it is recorded.
    publication: Option<Value>,
}

/// What one boundary's profile says: the fiscal handlers' time, the
/// engine's own, and each fiscal handler's.
struct Split {
    fiscal: f64,
    engine: f64,
    handlers: BTreeMap<String, f64>,
}

impl Samples {
    fn record_split(&mut self, split: Option<Split>) {
        if let Some(split) = split {
            self.settlement_fiscal.push(split.fiscal);
            self.settlement_engine.push(split.engine);
            for (name, ms) in split.handlers {
                self.settlement_handlers.entry(name).or_default().push(ms);
            }
        }
    }

    fn render(&self) -> Value {
        let mut value = json!({
            "record_changes": self.record_changes,
            "report_records_published": self.records_published,
            "drain_boundaries": self.drain_boundaries,
            "applied": self.applied,
            "rejected": self.rejected,
        });
        let map = value.as_object_mut().expect("an object");
        for (key, samples) in [
            ("admission_boundary_ms", &self.admission),
            ("admission_fiscal_handlers_ms", &self.admission_fiscal),
            ("settlement_boundary_ms", &self.settlement),
            ("settlement_fiscal_handlers_ms", &self.settlement_fiscal),
            ("settlement_engine_ms", &self.settlement_engine),
            ("drain_boundaries_ms", &self.drains),
            ("host_ms", &self.host),
            ("audit_after_ms", &self.audit),
            ("heap_peak_bytes", &self.heap_peak),
        ] {
            if !samples.is_empty() {
                map.insert(key.to_owned(), summary(samples));
            }
        }
        if !self.settlement_handlers.is_empty() {
            map.insert(
                "settlement_fiscal_handler_ms".to_owned(),
                self.settlement_handlers
                    .iter()
                    .map(|(name, samples)| (name.clone(), summary(samples)))
                    .collect::<serde_json::Map<_, _>>()
                    .into(),
            );
        }
        if let Some(publication) = &self.publication {
            map.insert("publication".to_owned(), publication.clone());
        }
        value
    }
}

/// Settles one boundary at the run's time and returns its receipt and cost,
/// with the fiscal handlers' and the engine's share in elapsed mode.
fn timed_boundary(canwu: &mut Canwu) -> Fallible<(BoundaryReceipt, f64, Option<Split>)> {
    let profiling = !counting();
    canwu.set_boundary_profiling(profiling);
    let started = Meter::start();
    let receipt = canwu.settle_boundary(BoundaryRequest::at(canwu.time()))?;
    let cost = started.finish();
    let split = if profiling {
        let batch = canwu.take_boundary_profiles();
        batch.profiles.last().map(|profile| {
            #[allow(clippy::cast_precision_loss)]
            let ms = |micros: u64| micros as f64 / 1_000.0;
            let mut handlers = BTreeMap::new();
            for timing in profile
                .handlers
                .iter()
                .filter(|timing| timing.plugin == PLUGIN_NAME)
            {
                *handlers.entry(timing.name.clone()).or_insert(0.0) += ms(timing.elapsed_micros);
            }
            Split {
                fiscal: ms(profile.plugin_micros(PLUGIN_NAME)),
                engine: ms(profile.remaining_micros()),
                handlers,
            }
        })
    } else {
        None
    };
    canwu.set_boundary_profiling(false);
    Ok((receipt, cost, split))
}

/// Settles the drain boundaries due at the run's instant, and returns how
/// many, their total cost, and the report records they published.
fn drain(canwu: &mut Canwu, audit: bool, samples: &mut Samples) -> Fallible<(usize, f64, usize)> {
    let at = canwu.time();
    let mut drained = (0, 0.0, 0);
    loop {
        let started = Meter::start();
        let Some(receipt) = canwu.step_canonical()? else {
            break;
        };
        drained.1 += started.finish();
        if receipt.settled_at != at {
            return Err("a drain boundary left the instant".into());
        }
        drained.0 += 1;
        drained.2 += receipt.knowledge_record_count;
        if audit {
            audit_after(canwu, samples)?;
        }
    }
    Ok(drained)
}

fn audit_after(canwu: &Canwu, samples: &mut Samples) -> Fallible<()> {
    let started = Instant::now();
    audit_fiscal_records(canwu)?;
    #[allow(clippy::cast_precision_loss)]
    samples
        .audit
        .push(started.elapsed().as_secs_f64() * 1_000.0);
    Ok(())
}

/// Enqueues `actions` as tracked commands: one batch per institution and
/// chunk of `MAX_FISCAL_BATCH_ENTRIES`, each entry against its ledger's
/// revision now.
fn enqueue_actions(
    canwu: &mut Canwu,
    world: &World,
    actions: Vec<(usize, String, FiscalAction)>,
) -> Fallible<()> {
    for request in action_requests(&*canwu, world, actions)? {
        canwu.enqueue_command(canwu.time(), 0, request)?;
    }
    Ok(())
}

/// The tracked commands that carry `actions`, through either run handle:
/// one batch per institution and chunk of `MAX_FISCAL_BATCH_ENTRIES`, each
/// entry against its ledger's revision now.
fn action_requests<H: CanwuRun + ?Sized>(
    canwu: &H,
    world: &World,
    actions: Vec<(usize, String, FiscalAction)>,
) -> Fallible<Vec<CommandRequest>> {
    let mut by_institution: BTreeMap<usize, Vec<FiscalActionEntry>> = BTreeMap::new();
    for (scope, action_id, action) in actions {
        let expected_revision =
            fiscal_target_revision(canwu, &FiscalActionTarget::Ledger(scope_id(scope)))?
                .ok_or("a profile scope has no ledger")?;
        by_institution
            .entry(world.scope_institution[scope])
            .or_default()
            .push(FiscalActionEntry {
                action_id,
                expected_revision,
                action,
            });
    }
    let revision = canwu.revision();
    let mut offset = 0_u64;
    let mut requests = Vec::new();
    for (institution, entries) in by_institution {
        for chunk in entries.chunks(MAX_FISCAL_BATCH_ENTRIES) {
            let batch = FiscalActionBatch {
                authority_binding_id: authority_id(institution),
                entries: chunk.to_vec(),
            };
            requests.push(CommandRequest::new(
                CommandRequestId::new(revision + offset + 1),
                revision + offset,
                CommandEnvelope::new(
                    Issuer::Actor(principal(institution)?),
                    fiscal_action_command(&batch)?,
                )
                .at_time(canwu.time()),
            ));
            offset += 1;
        }
    }
    Ok(requests)
}

fn assessments(scopes: &[usize]) -> Vec<(usize, String, FiscalAction)> {
    scopes
        .iter()
        .map(|&scope| {
            (
                scope,
                format!("assess.{scope:06}"),
                FiscalAction::OpenAssessment {
                    assessment_id: format!("assess.{scope:06}"),
                    rule_id: RULE.to_owned(),
                    scope_binding_id: scope_id(scope),
                    accounting_cycle_id: CYCLE.to_owned(),
                    quantity: 1_000,
                    unit: "grain".to_owned(),
                    payment_form: FiscalPaymentForm::Grain,
                    commutation_quote: None,
                },
            )
        })
        .collect()
}

fn authorizations(world: &World, scopes: &[usize]) -> Fallible<Vec<(usize, String, FiscalAction)>> {
    scopes
        .iter()
        .map(|&scope| {
            Ok((
                scope,
                format!("authorize.{scope:06}"),
                FiscalAction::AuthorizeExecution {
                    request_id: format!("request.{scope:06}"),
                    scope_binding_id: scope_id(scope),
                    assessment_id: format!("assess.{scope:06}"),
                    kind: FiscalExecutionKind::Collect,
                    quantity: 600,
                    unit: "grain".to_owned(),
                    resource: ResourceId::new(1),
                    source: EntityRef::Territory(TerritoryId::new(1)),
                    target: institution_entity(world.scope_institution[scope])?,
                    purpose: "Profile collection".to_owned(),
                },
            ))
        })
        .collect()
}

fn closes(scopes: &[usize]) -> Vec<(usize, String, FiscalAction)> {
    scopes
        .iter()
        .map(|&scope| {
            (
                scope,
                format!("close.{scope:06}"),
                FiscalAction::CloseAccountingCycle {
                    scope_binding_id: scope_id(scope),
                    accounting_cycle_id: CYCLE.to_owned(),
                },
            )
        })
        .collect()
}

/// Admits and settles `actions` on a fork of `base`, with the drains that
/// follow; measured unless `sample` is `None`. Returns the run.
fn action_boundaries(
    base: &Canwu,
    world: &World,
    actions: Vec<(usize, String, FiscalAction)>,
    measure: Option<(&mut Samples, bool)>,
) -> Fallible<Canwu> {
    let mut run = base.fork();
    let events = run.events().len();
    enqueue_actions(&mut run, world, actions)?;
    let (_, admission, admission_split) = timed_boundary(&mut run)?;
    let (receipt, settlement, split) = timed_boundary(&mut run)?;
    if let Some((samples, last)) = measure {
        if last {
            audit_after(&run, samples)?;
        }
        let (drains, drain_cost, published) = drain(&mut run, last, samples)?;
        samples.admission.push(admission);
        if let Some(split) = admission_split {
            samples.admission_fiscal.push(split.fiscal);
        }
        samples.settlement.push(settlement);
        samples.record_split(split);
        samples.drains.push(drain_cost);
        samples.drain_boundaries = drains;
        samples.records_published = receipt.knowledge_record_count + published;
        samples.record_changes = receipt.record_change_count;
        let outcomes = FiscalSettledAction::from_events(&run.events()[events..]);
        samples.applied = outcomes
            .iter()
            .filter(|outcome| outcome.disposition == FiscalActionDisposition::Applied)
            .count();
        samples.rejected = outcomes.len() - samples.applied;
    } else {
        let mut ignored = Samples::default();
        drain(&mut run, false, &mut ignored)?;
    }
    Ok(run)
}

fn measured_actions(
    base: &Canwu,
    world: &World,
    preset: &Preset,
    actions: &dyn Fn() -> Fallible<Vec<(usize, String, FiscalAction)>>,
) -> Fallible<(Samples, Canwu)> {
    let mut samples = Samples::default();
    let mut kept = None;
    for sample in 0..preset.warmup + preset.samples {
        let last = sample + 1 == preset.warmup + preset.samples;
        if sample < preset.warmup {
            action_boundaries(base, world, actions()?, None)?;
        } else {
            kept = Some(action_boundaries(
                base,
                world,
                actions()?,
                Some((&mut samples, last)),
            )?);
        }
    }
    Ok((samples, kept.ok_or("no sample")?))
}

/// Settles the first boundary, which publishes every authored observer's
/// report whole, and its drains, and returns their cost.
fn publish_initial_reports(canwu: &mut Canwu) -> Fallible<Value> {
    canwu.enqueue_plugin_ingress(PluginIngressRequest::new(
        SUPPORT_PLUGIN,
        TICK_INGRESS,
        canwu.time(),
        json!({}),
    ))?;
    let mut samples = Samples::default();
    let (receipt, cost, split) = timed_boundary(canwu)?;
    let (drains, drain_cost, published) = drain(canwu, false, &mut samples)?;
    audit_after(canwu, &mut samples)?;
    samples.settlement.push(cost);
    samples.record_split(split);
    samples.drains.push(drain_cost);
    samples.drain_boundaries = drains;
    samples.records_published = receipt.knowledge_record_count + published;
    samples.record_changes = receipt.record_change_count;
    Ok(samples.render())
}

/// Times idle boundaries: one tick of another plugin, nothing fiscal due.
fn idle(base: &Canwu, samples: usize) -> Fallible<Value> {
    let mut run = base.fork();
    let mut costs = Vec::with_capacity(samples);
    let mut fiscal = Vec::with_capacity(samples);
    for sample in 0..samples + 3 {
        run.enqueue_plugin_ingress(PluginIngressRequest::new(
            SUPPORT_PLUGIN,
            TICK_INGRESS,
            run.time(),
            json!({}),
        ))?;
        let (receipt, cost, split) = timed_boundary(&mut run)?;
        if receipt.record_change_count != 0 || receipt.knowledge_record_count != 0 {
            return Err("an idle boundary wrote fiscal records or reports".into());
        }
        if sample >= 3 {
            costs.push(cost);
            if let Some(split) = split {
                fiscal.push(split.fiscal);
            }
        }
    }
    let mut value = json!({ "boundary_ms": summary(&costs) });
    if !fiscal.is_empty() {
        value["fiscal_handlers_ms"] = summary(&fiscal);
    }
    Ok(value)
}

/// Records one execution result for each scope, in support packets, and
/// returns the evidence version of each.
fn record_evidence(
    canwu: &mut Canwu,
    world: &World,
    scopes: &[usize],
) -> Fallible<Vec<canwu_api::DomainRecordVersionRef>> {
    for chunk in scopes.chunks(EVIDENCE_PER_PACKET) {
        let results = chunk
            .iter()
            .map(|&scope| {
                Ok(FiscalExecutionEvidence {
                    id: format!("result.{scope:06}"),
                    external_operation_id: format!("operation.{scope:06}"),
                    scope_binding_id: scope_id(scope),
                    request_id: format!("request.{scope:06}"),
                    quantity: 600,
                    unit: "grain".to_owned(),
                    payment_form: FiscalPaymentForm::Grain,
                    execution_kind: FiscalExecutionKind::Collect,
                    resource: ResourceId::new(1),
                    source: EntityRef::Territory(TerritoryId::new(1)),
                    target: institution_entity(world.scope_institution[scope])?,
                    disposition: FiscalReceiptDisposition::Fulfilled,
                })
            })
            .collect::<Fallible<Vec<_>>>()?;
        canwu.enqueue_plugin_ingress(PluginIngressRequest::new(
            SUPPORT_PLUGIN,
            EVIDENCE_INGRESS,
            canwu.time(),
            serde_json::to_value(results)?,
        ))?;
    }
    canwu.settle_boundary(BoundaryRequest::at(canwu.time()))?;
    scopes
        .iter()
        .map(|&scope| {
            Ok(canwu
                .current_domain_record_version(
                    &TypedDomainRecordRef::<ExecutionResult>::new(format!("result.{scope:06}"))
                        .into_untyped(),
                )?
                .ok_or("an evidence record is missing")?)
        })
        .collect()
}

fn measured_receipts(
    base: &Canwu,
    preset: &Preset,
    scopes: &[usize],
    evidence: &[canwu_api::DomainRecordVersionRef],
) -> Fallible<Samples> {
    let mut samples = Samples::default();
    for sample in 0..preset.warmup + preset.samples {
        let last = sample + 1 == preset.warmup + preset.samples;
        let mut run = base.fork();
        let refused = refusals(&run);
        let packets: Vec<FiscalExecutionReceiptPacket> = scopes
            .iter()
            .zip(evidence)
            .map(|(&scope, version)| FiscalExecutionReceiptPacket {
                receipt_id: format!("receipt.{scope:06}"),
                scope_binding_id: scope_id(scope),
                request_id: format!("request.{scope:06}"),
                external_evidence: BTreeSet::from([version.clone()]),
            })
            .collect();
        let now = run.time();
        for chunk in packets.chunks(MAX_FISCAL_RECEIPTS_PER_PACKET) {
            enqueue_execution_receipts(&mut run, now, chunk)?;
        }
        let (receipt, cost, split) = timed_boundary(&mut run)?;
        if refusals(&run) != refused {
            return Err("a profile receipt was refused".into());
        }
        if sample < preset.warmup {
            continue;
        }
        if last {
            audit_after(&run, &mut samples)?;
        }
        let (drains, drain_cost, published) = drain(&mut run, last, &mut samples)?;
        samples.settlement.push(cost);
        samples.record_split(split);
        samples.drains.push(drain_cost);
        samples.drain_boundaries = drains;
        samples.records_published = receipt.knowledge_record_count + published;
        samples.record_changes = receipt.record_change_count;
        samples.applied = scopes.len();
    }
    Ok(samples)
}

fn refusals(canwu: &Canwu) -> usize {
    canwu
        .events()
        .iter()
        .filter(|event| {
            event.kind.plugin_identity()
                == Some((PLUGIN_NAME, canwu_fiscal::FISCAL_INGRESS_REJECTED_EVENT))
        })
        .count()
}

/// Prepares, stores, verifies, and queues archive commits of `books` closed
/// books, and settles them, with the retention acknowledgement after.
fn measured_archive(base: &Canwu, preset: &Preset, books: usize) -> Fallible<Value> {
    let mut prepare = Vec::new();
    let mut enqueue = Vec::new();
    let mut finalize = Vec::new();
    let mut samples = Samples::default();
    let mut objects = 0;
    let mut object_bytes = 0;
    for sample in 0..preset.warmup + preset.samples {
        let last = sample + 1 == preset.warmup + preset.samples;
        let mut run = base.fork();
        let store = MemoryArchiveStore::default();
        let started = Meter::start();
        let prepared = prepare_fiscal_archive(&run, books, &store)?;
        let prepared_cost = started.finish();
        let started = Meter::start();
        let receipt = enqueue_fiscal_archive(&mut run, &prepared, &store)?;
        let enqueued = started.finish();
        let (commit, settled, split) = timed_boundary(&mut run)?;
        if last {
            audit_after(&run, &mut samples)?;
        }
        let (drains, drain_cost, published) = drain(&mut run, last, &mut samples)?;
        let started = Meter::start();
        finalize_fiscal_archive_retention(&mut run, &store, &receipt)?;
        let finalized = started.finish();
        let (_, acknowledged, _) = timed_boundary(&mut run)?;
        if last {
            audit_after(&run, &mut samples)?;
        }
        if sample < preset.warmup {
            continue;
        }
        objects = store.objects.borrow().len();
        object_bytes = store.objects.borrow().values().map(Vec::len).sum();
        prepare.push(prepared_cost);
        enqueue.push(enqueued);
        finalize.push(finalized);
        samples.settlement.push(settled);
        samples.record_split(split);
        samples.drains.push(drain_cost);
        samples.host.push(acknowledged);
        samples.drain_boundaries = drains;
        samples.records_published = commit.knowledge_record_count + published;
        samples.record_changes = commit.record_change_count;
        samples.applied = books;
    }
    let mut value = samples.render();
    let map = value.as_object_mut().ok_or("an object")?;
    map.insert("books".to_owned(), json!(books));
    map.insert("store_objects".to_owned(), json!(objects));
    map.insert("store_object_bytes".to_owned(), json!(object_bytes));
    map.insert("prepare_ms".to_owned(), summary(&prepare));
    map.insert("verify_and_enqueue_ms".to_owned(), summary(&enqueue));
    map.insert("finalize_retention_ms".to_owned(), summary(&finalize));
    if let Some(host) = map.remove("host_ms") {
        map.insert("acknowledgement_boundary_ms".to_owned(), host);
    }
    Ok(value)
}

/// Partitions an authored state and starts a run of it; returns the run and
/// the cost of partitioning and of activation.
fn start(
    world: &World,
    catalog: &CompiledFiscalCatalog,
    state: FiscalState,
    plugins: &[&dyn SimulationPlugin],
) -> Fallible<(Canwu, f64, f64)> {
    let started = Meter::start();
    let records = state.into_records(catalog)?;
    let partition_cost = started.finish();
    let mut domain_records = vec![catalog.clone().into_record()?];
    domain_records.extend(records);
    let scenario = scenario(world, domain_records)?;
    let started = Meter::start();
    let run = Canwu::new_with_plugins(41, scenario, plugins)?;
    Ok((run, partition_cost, started.finish()))
}

/// The assessment of every scope in one monthly cycle, and the close of each
/// book of it.
fn cycle_actions(scopes: usize, cycle: usize) -> [Vec<(usize, String, FiscalAction)>; 2] {
    let id = format!("month.{cycle:04}");
    [
        (0..scopes)
            .map(|scope| {
                (
                    scope,
                    format!("assess.{cycle:04}.{scope:06}"),
                    FiscalAction::OpenAssessment {
                        assessment_id: format!("assess.{cycle:04}.{scope:06}"),
                        rule_id: RULE.to_owned(),
                        scope_binding_id: scope_id(scope),
                        accounting_cycle_id: id.clone(),
                        quantity: 1_000 + u64::try_from(cycle).unwrap_or(0),
                        unit: "grain".to_owned(),
                        payment_form: FiscalPaymentForm::Grain,
                        commutation_quote: None,
                    },
                )
            })
            .collect(),
        (0..scopes)
            .map(|scope| {
                (
                    scope,
                    format!("close.{cycle:04}.{scope:06}"),
                    FiscalAction::CloseAccountingCycle {
                        scope_binding_id: scope_id(scope),
                        accounting_cycle_id: id.clone(),
                    },
                )
            })
            .collect(),
    ]
}

/// Moves a sealed run's clock to `at` and settles a boundary there that a
/// tick makes event-bearing, with the drains at its instant.
fn tick_compact(run: &mut CompactedCanwu, at: SimTime) -> Fallible<()> {
    let wait = at.checked_sub(run.time()).ok_or("the day has passed")?;
    run.advance_canonical(wait)?;
    run.enqueue_plugin_ingress(PluginIngressRequest::new(
        SUPPORT_PLUGIN,
        TICK_INGRESS,
        run.time(),
        json!({}),
    ))?;
    run.advance_canonical(SimDuration::ZERO)?;
    Ok(())
}

/// Moves a sealed run's clock to `at` without a boundary.
fn wait_compact(run: &mut CompactedCanwu, at: SimTime) -> Fallible<()> {
    let wait = at.checked_sub(run.time()).ok_or("the day has passed")?;
    if !run.advance_canonical(wait)?.is_empty() {
        return Err("a boundary came before the day".into());
    }
    Ok(())
}

/// Held knowledge over simulated years: `cycles` monthly cycles on a run
/// that seals its evidence after each, the observers on their monthly
/// interval cadences. Each month every scope is assessed on its first day,
/// a tick each day lets the observers due that day publish, and on its last
/// day every book is closed and archived. After each month: the record
/// bodies, holder entries, and payload bytes the run holds, the bytes of its
/// continuation checkpoint, and the month's wall time up to its seal (not
/// these measurements or the audit after them); after the last, the time to
/// restore that checkpoint. Held knowledge must stay flat after the second
/// month.
#[allow(clippy::too_many_lines)]
fn held_over_cycles(
    base: &Canwu,
    world: &World,
    plugins: &[&dyn SimulationPlugin],
    cycles: usize,
) -> Fallible<Value> {
    let mut run = base.fork().into_compacted()?;
    let store = MemoryArchiveStore::default();
    let scopes = world.scope_institution.len();
    let day = |cycle: usize, day: i64| -> Fallible<SimTime> {
        Ok(SimTime::EPOCH + SimDuration::days(i64::try_from(cycle)? * 30 + day))
    };
    let mut rows = Vec::new();
    for cycle in 0..cycles {
        let started = Instant::now();
        let [assess, close] = cycle_actions(scopes, cycle);
        wait_compact(&mut run, day(cycle, 1)?)?;
        for request in action_requests(&run, world, assess)? {
            run.enqueue_command(run.time(), 0, request)?;
        }
        run.advance_canonical(SimDuration::ZERO)?;
        for date in 2..29 {
            tick_compact(&mut run, day(cycle, date)?)?;
        }
        wait_compact(&mut run, day(cycle, 29)?)?;
        for request in action_requests(&run, world, close)? {
            run.enqueue_command(run.time(), 0, request)?;
        }
        run.advance_canonical(SimDuration::ZERO)?;
        loop {
            // A commit of the head receipt alone waits for the next month.
            let prepared = match prepare_fiscal_archive(&run, 4_096, &store) {
                Ok(prepared) if !prepared.selected_books.is_empty() => prepared,
                Ok(_) | Err(FiscalArchiveError::NothingToArchive) => break,
                Err(error) => return Err(error.into()),
            };
            let receipt = enqueue_fiscal_archive(&mut run, &prepared, &store)?;
            run.advance_canonical(SimDuration::ZERO)?;
            finalize_fiscal_archive_retention(&mut run, &store, &receipt)?;
            run.advance_canonical(SimDuration::ZERO)?;
        }
        tick_compact(&mut run, day(cycle, 29)? + SimDuration::hours(12))?;
        drop(run.seal_evidence()?);
        #[allow(clippy::cast_precision_loss)]
        let month_ms = started.elapsed().as_secs_f64() * 1_000.0;
        let ledgers = &run.knowledge().records;
        let mut payload_bytes = 0_usize;
        for body in ledgers.bodies() {
            payload_bytes += body.payload_bytes()?;
        }
        let (bodies, entries) = (ledgers.body_count(), ledgers.entry_count());
        let (checkpoint, _) = run.continuation_checkpoint()?;
        let checkpoint_bytes = checkpoint.to_json()?.len();
        drop(checkpoint);
        audit_fiscal_records(&run)?;
        let row = json!({
            "cycle": cycle + 1,
            "bodies": bodies,
            "entries": entries,
            "payload_bytes": payload_bytes,
            "continuation_checkpoint_bytes": checkpoint_bytes,
            "month_wall_ms": round(month_ms),
        });
        eprintln!("held after cycle {}: {row}", cycle + 1);
        rows.push(row);
    }
    let (checkpoint, hash) = run.continuation_checkpoint()?;
    let json = checkpoint.to_json()?;
    drop(checkpoint);
    let started = Instant::now();
    let restored =
        CompactedCanwu::from_continuation_checkpoint_json_verified(&json, &hash, plugins)?;
    #[allow(clippy::cast_precision_loss)]
    let restore_ms = started.elapsed().as_secs_f64() * 1_000.0;
    if restored.checkpoint_hash() != run.checkpoint_hash() {
        return Err("the restored run differs from the run it was written from".into());
    }
    Ok(json!({
        "cycles": rows,
        "continuation_checkpoint_bytes": json.len(),
        "continuation_restore_ms": round(restore_ms),
        "archive_store_objects": store.objects.borrow().len(),
    }))
}

/// What the run's last boundary published: record bodies, holder entries,
/// payload bytes (as the engine's budget charges them), directives (each one
/// `KnowledgePublished` event), and distinct holders.
fn publication(canwu: &Canwu, receipt: &BoundaryReceipt) -> Fallible<Value> {
    let boundary = canwu.boundaries().last().ok_or("the run has no boundary")?;
    let mut payload_bytes = 0_usize;
    let mut holders = BTreeSet::new();
    for change in &boundary.knowledge_changes {
        for body in &change.records {
            payload_bytes += body.payload_bytes()?;
        }
        holders.extend(change.holders.iter().map(|publication| &publication.holder));
    }
    Ok(json!({
        "bodies": receipt.knowledge_record_count,
        "entries": receipt.knowledge_entry_count,
        "payload_bytes": payload_bytes,
        "directives": receipt.knowledge_batch_count,
        "holders": holders.len(),
    }))
}

/// Settles, on forks of `base`, the boundary at `at` that a tick of another
/// plugin makes event-bearing, and the drains that follow at its instant:
/// the interval report work due at `at`, and nothing else fiscal.
fn measured_due(base: &Canwu, preset: &Preset, at: SimTime) -> Fallible<Samples> {
    let mut samples = Samples::default();
    for sample in 0..preset.warmup + preset.samples {
        let last = sample + 1 == preset.warmup + preset.samples;
        let mut run = base.fork();
        let wait = at
            .checked_sub(run.time())
            .ok_or("the due time has passed")?;
        if !run.advance_canonical(wait)?.is_empty() {
            return Err("a boundary came before the due time".into());
        }
        run.enqueue_plugin_ingress(PluginIngressRequest::new(
            SUPPORT_PLUGIN,
            TICK_INGRESS,
            run.time(),
            json!({}),
        ))?;
        let (receipt, cost, split) = timed_boundary(&mut run)?;
        if sample < preset.warmup {
            continue;
        }
        if counting() {
            samples.heap_peak.push(heap_peak());
        }
        samples.publication = Some(publication(&run, &receipt)?);
        if last {
            audit_after(&run, &mut samples)?;
        }
        let (drains, drain_cost, published) = drain(&mut run, last, &mut samples)?;
        samples.settlement.push(cost);
        samples.record_split(split);
        samples.drains.push(drain_cost);
        samples.drain_boundaries = drains;
        samples.records_published = receipt.knowledge_record_count + published;
        samples.record_changes = receipt.record_change_count;
    }
    Ok(samples)
}

/// The world with every observer due monthly at the same instants: the
/// observers settled at 0 (no book to report yet, so only queued for their
/// next due time), every scope assessed (one dirty cycle owed to every
/// observer, and nothing published, since none is due), and the measured
/// boundary at the next due time, every observer due at once.
fn due_instant(
    world: &World,
    preset: &Preset,
    courts: FiscalReportGranularity,
    order: &[usize],
    plugins: &[&dyn SimulationPlugin],
) -> Fallible<Value> {
    let catalog = catalog(preset.partitions)?;
    let state = fiscal_state(
        world,
        &Observers {
            courts,
            cadence: aligned,
        },
    )?;
    let (mut base, _, _) = start(world, &catalog, state, plugins)?;
    let initial = publish_initial_reports(&mut base)?;
    let assessed = action_boundaries(&base, world, assessments(order), None)?;
    drop(base);
    let samples = measured_due(&assessed, preset, SimTime::EPOCH + MONTH)?;
    Ok(json!({
        "observers_due": preset.partitions * OBSERVERS_PER_PARTITION,
        "initial_report_publication": initial,
        "boundaries": samples.render(),
    }))
}

struct Options {
    count_allocations: bool,
    preset: String,
    /// The courts' granularity: `scope` (the default) or `institution`.
    courts: FiscalReportGranularity,
    /// Monthly interval cadences, staggered over the month, instead of
    /// on-change reports.
    interval: bool,
    /// Monthly cycles of the held-knowledge measurement; none when zero.
    held_cycles: usize,
    /// Only the rows of interval reports (`--rows interval`): the idle
    /// boundaries, the spread due day, and the due instant, on a run whose
    /// every scope is assessed without measuring the bursts.
    interval_rows_only: bool,
    seed: u64,
    machine: String,
    recorded_on: String,
    output: Option<PathBuf>,
}

fn options() -> Fallible<Options> {
    let mut options = Options {
        count_allocations: false,
        preset: "smoke".to_owned(),
        courts: FiscalReportGranularity::Scope,
        interval: false,
        held_cycles: 0,
        interval_rows_only: false,
        seed: 0xF15C_0016,
        machine: "unspecified".to_owned(),
        recorded_on: "unspecified".to_owned(),
        output: None,
    };
    let mut arguments = env::args().skip(1);
    while let Some(flag) = arguments.next() {
        let mut value = || {
            arguments
                .next()
                .ok_or_else(|| format!("{flag} requires a value"))
        };
        match flag.as_str() {
            "--mode" => {
                options.count_allocations = match value()?.as_str() {
                    "elapsed" => false,
                    "allocations" => true,
                    other => return Err(format!("unsupported mode {other}").into()),
                };
            }
            "--preset" => options.preset = value()?,
            "--courts" => {
                options.courts = match value()?.as_str() {
                    "scope" => FiscalReportGranularity::Scope,
                    "institution" => FiscalReportGranularity::Institution,
                    other => return Err(format!("unsupported court granularity {other}").into()),
                };
            }
            "--held-cycles" => options.held_cycles = value()?.parse()?,
            "--rows" => {
                options.interval_rows_only = match value()?.as_str() {
                    "all" => false,
                    "interval" => true,
                    other => return Err(format!("unsupported rows {other}").into()),
                };
            }
            "--cadence" => {
                options.interval = match value()?.as_str() {
                    "on-change" => false,
                    "interval" => true,
                    other => return Err(format!("unsupported report cadence {other}").into()),
                };
            }
            "--seed" => options.seed = value()?.parse()?,
            "--machine" => options.machine = value()?,
            "--recorded-on" => options.recorded_on = value()?,
            "--output" => options.output = Some(PathBuf::from(value()?)),
            "--help" | "-h" => {
                println!(
                    "fiscal-world-profile [--mode elapsed|allocations] [--preset smoke|world] \
                     [--courts scope|institution] [--cadence on-change|interval] \
                     [--held-cycles N] [--rows all|interval] [--seed N] [--machine LABEL] \
                     [--recorded-on DATE] [--output PATH]"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument {other}").into()),
        }
    }
    Ok(options)
}

#[allow(clippy::too_many_lines)]
fn main() -> Fallible<()> {
    let options = options()?;
    if options.count_allocations && !cfg!(feature = "allocation-counting") {
        return Err("allocation mode requires --features allocation-counting".into());
    }
    if !options.count_allocations && cfg!(feature = "allocation-counting") {
        return Err("elapsed mode must be built without the allocation-counting feature".into());
    }
    if options.interval_rows_only && (!options.interval || options.held_cycles > 0) {
        return Err("--rows interval needs --cadence interval and no --held-cycles".into());
    }
    COUNT_ALLOCATIONS.store(options.count_allocations, Ordering::Relaxed);
    let preset = preset(&options.preset)?;
    let mut random = Random(options.seed);
    let world = generate(&preset, &mut random);
    let support = SupportPlugin;
    let fiscal = FiscalPlugin::new([DomainRecordKind::for_type::<ExecutionResult>()]);
    let plugins: [&dyn SimulationPlugin; 2] = [&support, &fiscal];

    // Partition the authored state and start the run.
    let catalog = catalog(preset.partitions)?;
    let observers = Observers {
        courts: options.courts,
        cadence: if options.interval {
            staggered
        } else {
            on_change
        },
    };
    let state = fiscal_state(&world, &observers)?;
    let (mut base, partition_cost, activation) = start(&world, &catalog, state, &plugins)?;
    let started = Instant::now();
    audit_fiscal_records(&base)?;
    #[allow(clippy::cast_precision_loss)]
    let audit_base = started.elapsed().as_secs_f64() * 1_000.0;
    let sizes_initial = record_sizes(&base)?;
    let initial_reports = publish_initial_reports(&mut base)?;
    let base = base;
    let idle_initial = idle(&base, preset.idle_samples)?;

    // Assessing bursts on scopes in a seeded order, spread over every
    // partition; the largest burst assesses every scope.
    let mut order: Vec<usize> = (0..preset.scopes).collect();
    random.shuffle(&mut order);
    let mut bursts = serde_json::Map::new();
    let mut assessed = None;
    if options.interval_rows_only {
        assessed = Some(action_boundaries(&base, &world, assessments(&order), None)?);
    }
    for (label, count) in preset.bursts.iter().filter(|_| !options.interval_rows_only) {
        let scopes = order[..*count].to_vec();
        let (samples, run) =
            measured_actions(&base, &world, &preset, &|| Ok(assessments(&scopes)))?;
        eprintln!("assess {label}: {}", samples.render());
        bursts.insert(
            (*label).to_owned(),
            json!({ "entries": count, "boundaries": samples.render() }),
        );
        if *count == preset.scopes {
            assessed = Some(run);
        }
    }
    let assessed = assessed.ok_or("no burst assessed every scope")?;
    let sizes_assessed = record_sizes(&assessed)?;
    let idle_assessed = idle(&assessed, preset.idle_samples)?;
    // With monthly cadences staggered over the month, the observers due on
    // the first day after the burst: about a thirtieth of them, each owing
    // its first report with the burst's cycle in it.
    let mut cadence_rows = serde_json::Map::new();
    if options.interval {
        let samples = measured_due(&assessed, &preset, SimTime::EPOCH + SimDuration::days(1))?;
        eprintln!("spread due day: {}", samples.render());
        let due = (0..preset.partitions * OBSERVERS_PER_PARTITION)
            .filter(|index| index % 30 == 1)
            .count();
        cadence_rows.insert(
            "spread_due_day".to_owned(),
            json!({ "day": 1, "observers_due": due, "boundaries": samples.render() }),
        );
    }
    if options.interval_rows_only {
        drop((assessed, base));
        let value = due_instant(&world, &preset, options.courts, &order, &plugins)?;
        eprintln!("due instant: {value}");
        cadence_rows.insert("due_instant".to_owned(), value);
        let mut report = json!({
            "profile": "fiscal-world",
            "preset": preset.name,
            "rows": "interval",
            "metric": if options.count_allocations { "allocation_operations" } else { "elapsed_ms" },
            "machine": options.machine,
            "recorded_on": options.recorded_on,
            "seed": options.seed,
            "dimensions": {
                "partitions": preset.partitions,
                "scopes": preset.scopes,
                "observers": preset.partitions * OBSERVERS_PER_PARTITION,
                "courts": options.courts,
                "report_cadence": "monthly, staggered over thirty days",
            },
            "samples": preset.samples,
            "state": {
                "partition_ms": partition_cost,
                "activation_ms": activation,
                "audit_every_record_ms": audit_base,
                "initial": sizes_initial,
                "every_scope_assessed": sizes_assessed,
            },
            "initial_report_publication": initial_reports,
            "idle_boundary": {
                "initial": idle_initial,
                "every_scope_assessed": idle_assessed,
            },
            "interval_reports": cadence_rows,
        });
        return write_report(&options, &mut report);
    }
    // Forking the unsealed run once every scope is assessed, the fork
    // dropped outside the measurement.
    let started = Meter::start();
    let fork = assessed.fork();
    let fork_cost = started.finish();
    drop(fork);
    let snapshot_started = Instant::now();
    let snapshot = assessed.snapshot_json()?;
    #[allow(clippy::cast_precision_loss)]
    let snapshot_ms = snapshot_started.elapsed().as_secs_f64() * 1_000.0;
    let started = Meter::start();
    let restored = Canwu::from_snapshot_json_with_plugins(&snapshot, &plugins)?;
    let restore = started.finish();
    drop(restored);
    let snapshot_bytes = snapshot.len();
    drop(snapshot);

    // Receipts: collections authorized and their evidence recorded first.
    let largest_receipts = preset.receipts.iter().copied().max().unwrap_or(0);
    let receipt_scopes = order[..largest_receipts].to_vec();
    let authorized = action_boundaries(
        &assessed,
        &world,
        authorizations(&world, &receipt_scopes)?,
        None,
    )?;
    let mut with_evidence = authorized.fork();
    let evidence = record_evidence(&mut with_evidence, &world, &receipt_scopes)?;
    let mut receipts = serde_json::Map::new();
    for count in &preset.receipts {
        let samples = measured_receipts(
            &with_evidence,
            &preset,
            &receipt_scopes[..*count],
            &evidence[..*count],
        )?;
        eprintln!("receipts {count}: {}", samples.render());
        receipts.insert(count.to_string(), samples.render());
    }
    drop((authorized, with_evidence));

    // A close of every book.
    let (close_samples, closed) =
        measured_actions(&assessed, &world, &preset, &|| Ok(closes(&order)))?;
    eprintln!("close all: {}", close_samples.render());

    // Archive commits of closed books.
    let mut archive = serde_json::Map::new();
    for books in &preset.archive_commits {
        let value = measured_archive(&closed, &preset, *books)?;
        eprintln!("archive {books}: {value}");
        archive.insert(books.to_string(), value);
    }
    // Held knowledge over simulated years, on a run of the base world.
    let held = if options.held_cycles > 0 {
        Some(held_over_cycles(
            &base,
            &world,
            &plugins,
            options.held_cycles,
        )?)
    } else {
        None
    };
    drop((closed, assessed, base));

    // Every observer due at one instant, owing one dirty cycle: the gate
    // case, on a run of the same world whose observers share their due
    // times.
    if options.interval {
        let value = due_instant(&world, &preset, options.courts, &order, &plugins)?;
        eprintln!("due instant: {value}");
        cadence_rows.insert("due_instant".to_owned(), value);
    }

    let largest_partition = world.partitions.iter().map(Vec::len).max().unwrap_or(0);
    let mut report = json!({
        "profile": "fiscal-world",
        "preset": preset.name,
        "metric": if options.count_allocations { "allocation_operations" } else { "elapsed_ms" },
        "machine": options.machine,
        "recorded_on": options.recorded_on,
        "seed": options.seed,
        "dimensions": {
            "partitions": preset.partitions,
            "scopes": preset.scopes,
            "largest_partition_scopes": largest_partition,
            "institutions": world.institutions,
            "observers": preset.partitions * OBSERVERS_PER_PARTITION,
            "cycle": "annual, one cycle",
            "courts": options.courts,
            "report_cadence": if options.interval {
                "monthly, staggered over thirty days"
            } else {
                "on change"
            },
        },
        "samples": preset.samples,
        "state": {
            "partition_ms": partition_cost,
            "activation_ms": activation,
            "audit_every_record_ms": audit_base,
            "initial": sizes_initial,
            "every_scope_assessed": sizes_assessed,
            "fork_ms": fork_cost,
            "snapshot_json_bytes": snapshot_bytes,
            "snapshot_encode_ms": snapshot_ms,
            "restore_ms": restore,
        },
        "initial_report_publication": initial_reports,
        "idle_boundary": {
            "initial": idle_initial,
            "every_scope_assessed": idle_assessed,
        },
        "assessing_bursts": bursts,
        "receipt_boundaries": receipts,
        "close_every_book": close_samples.render(),
        "archive_commits": archive,
    });
    if !cadence_rows.is_empty() {
        report["interval_reports"] = Value::Object(cadence_rows);
    }
    if let Some(held) = held {
        report["held_knowledge"] = held;
    }
    write_report(&options, &mut report)
}

/// Prints the report, and writes it to `--output` when one is given; in
/// allocation mode its `*_ms` keys are relabelled first.
fn write_report(options: &Options, report: &mut Value) -> Fallible<()> {
    if options.count_allocations {
        relabel(report);
    }
    let rendered = serde_json::to_string_pretty(report)?;
    if let Some(path) = &options.output {
        fs::write(path, format!("{rendered}\n"))?;
    }
    println!("{rendered}");
    Ok(())
}

fn summary(samples: &[f64]) -> Value {
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let pick = |numerator: usize, denominator: usize| {
        sorted
            .get((sorted.len().saturating_sub(1)) * numerator / denominator)
            .copied()
            .unwrap_or_default()
    };
    json!({
        "median": round(pick(1, 2)),
        "p95": round(pick(19, 20)),
        "min": round(pick(0, 1)),
    })
}

fn round(value: f64) -> f64 {
    (value * 1_000.0).round() / 1_000.0
}
