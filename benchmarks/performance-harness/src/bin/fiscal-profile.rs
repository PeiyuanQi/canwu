//! Fiscal boundary profile.
//!
//! Measures, through the public API, what `canwu-fiscal` adds to a boundary:
//! one that admits no fiscal ingress, and a pair that admits and then settles a
//! batch of fiscal actions. The fiscal state is built in the scenario with a
//! history of settled assessments and execution requests, so every decode and
//! validation pays for a mid-campaign record rather than an empty one.
//!
//! `--mode allocations`, in a build with the `allocation-counting` feature,
//! counts the allocator calls each measured operation makes instead of timing
//! it, and reports `*_allocation_operations` where the elapsed build reports
//! `*_ms`.

use canwu_api::{
    BoundaryRequest, Canwu, CanwuError, CommandEnvelope, CommandId, CommandRequest,
    CommandRequestId, EntityRef, Government, GovernmentId, IngressClass, Issuer,
    KnowledgeHolderRef, KnowledgeSnapshot, MapPoint, OrganizationId, PayloadSchema, Person,
    PersonId, PluginIngressDescriptor, PluginIngressRequest, PluginRegistrar, ResourceId, Scenario,
    SimTime, SimulationGranularity, SimulationPlugin, Territory, TerritoryId, WorldSnapshot,
};
use canwu_fiscal::{
    CompiledFiscalCatalog, FiscalAction, FiscalActionDisposition, FiscalActionOutcome,
    FiscalActionRequest, FiscalAdoptionStage, FiscalAdoptionState, FiscalAssessment,
    FiscalAssessmentBasis, FiscalAuthorityBinding, FiscalCommutationPolicy, FiscalContentPack,
    FiscalContentSelection, FiscalCoverageDeclaration, FiscalCoverageSelector,
    FiscalCoverageStatus, FiscalExecutionKind, FiscalExecutionRequest, FiscalHistoricalMode,
    FiscalMechanism, FiscalObserverBinding, FiscalPackManifest, FiscalPaymentForm,
    FiscalPeriodDefinition, FiscalPlugin, FiscalProvenance, FiscalRegionDefinition,
    FiscalRuleDefinition, FiscalScopeBinding, FiscalState, FiscalStateRecord, HistoricalConfidence,
    HistoricalYearWindow, compile_fiscal_content, compute_aggregates,
    compute_transition_candidates, fiscal_action_command, fiscal_state_reference,
    recompute_derived,
};
use serde_json::{Value, json};
#[cfg(feature = "allocation-counting")]
use std::alloc::{GlobalAlloc, Layout, System};
#[cfg(feature = "allocation-counting")]
use std::cell::Cell;
use std::collections::BTreeSet;
use std::env;
use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

const TICK_PLUGIN: &str = "fiscal-profile-tick";
const TICK_INGRESS: &str = "tick_v1";
const PROFILE_SEED: u64 = 0xF15C_2026;
const NEW_CYCLE: &str = "cycle.new";
const BATCH_ACTION_PREFIX: &str = "profile.cycle.new.";

/// Owns one ingress type and no system: a tick makes a boundary event-bearing,
/// so event-driven systems of every plugin run, without touching fiscal state.
struct TickPlugin;

impl SimulationPlugin for TickPlugin {
    fn name(&self) -> &'static str {
        TICK_PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "00000000000000000000000000000000000000000000000000000000000000f1"
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

#[cfg(feature = "allocation-counting")]
struct CountingAllocator;

#[cfg(feature = "allocation-counting")]
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[cfg(feature = "allocation-counting")]
thread_local! {
    /// Whether this thread counts, and the allocator calls counted so far.
    static ALLOCATIONS: Cell<(bool, u64)> = const { Cell::new((false, 0)) };
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

// SAFETY: every operation delegates to `System` with the original pointer and
// layout; the thread-local counter only observes allocator calls.
#[cfg(feature = "allocation-counting")]
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count_allocation();
        // SAFETY: the caller supplies the layout required by `GlobalAlloc`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count_allocation();
        // SAFETY: the caller supplies the layout required by `GlobalAlloc`.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: the caller returns the pointer with its original layout.
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count_allocation();
        // SAFETY: the caller supplies the pointer, original layout, and size.
        unsafe { System.realloc(pointer, layout, new_size) }
    }
}

/// Set once from `--mode`: count allocator calls instead of timing.
static COUNT_ALLOCATIONS: AtomicBool = AtomicBool::new(false);

/// One measurement in progress: wall time in milliseconds, or allocator
/// calls (allocations and reallocations) in allocation mode.
struct Meter(Instant);

impl Meter {
    fn start() -> Self {
        #[cfg(feature = "allocation-counting")]
        if COUNT_ALLOCATIONS.load(Ordering::Relaxed) {
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

struct Options {
    count_allocations: bool,
    institutions: usize,
    cycles: usize,
    batch: usize,
    observers: usize,
    observed_institutions: usize,
    samples: usize,
    warmup: usize,
    machine: String,
    recorded_on: String,
    output: Option<PathBuf>,
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
    let tick = TickPlugin;
    let fiscal = FiscalPlugin::default();
    let plugins: [&dyn SimulationPlugin; 2] = [&tick, &fiscal];

    // Engine floor: the same world and plugins with no fiscal records.
    let mut floor = Canwu::new_with_plugins(PROFILE_SEED, scenario(&options, false)?, &plugins)?;
    let floor_ms = measure_idle(&mut floor, &options)?;

    // A fiscal state no boundary has written yet.
    let mut initial = Canwu::new_with_plugins(PROFILE_SEED, scenario(&options, true)?, &plugins)?;
    let state_bytes = serde_json::to_vec(&fiscal_state(&initial)?)?.len();
    let components = state_component_costs(&initial, &options)?;
    let idle_initial_ms = measure_idle(&mut initial, &options)?;

    // The same state after one settled fiscal action, as in a running campaign.
    // That action is the first change of the fiscal state, so its boundary
    // publishes every observer's report whole.
    let mut settled = initial.fork();
    settle_actions(&mut settled, 0, 1, "cycle.prime")?;
    let (report_records, report_bytes) = observer_reports(&settled, 1)?;
    let idle_settled_ms = measure_idle(&mut settled, &options)?;

    let mut admission_ms = Vec::with_capacity(options.samples);
    let mut settlement_ms = Vec::with_capacity(options.samples);
    let mut applied = 0;
    let mut published = (0, 0);
    let held = observer_reports(&settled, options.observers)?;
    for sample in 0..options.warmup + options.samples {
        let mut run = settled.fork();
        let (admit, settle) = settle_actions(&mut run, 1, options.batch, NEW_CYCLE)?;
        let now_held = observer_reports(&run, options.observers)?;
        published = (now_held.0 - held.0, now_held.1 - held.1);
        if sample >= options.warmup {
            admission_ms.push(admit);
            settlement_ms.push(settle);
        }
        applied = fiscal_state(&run)?
            .action_outcomes
            .values()
            .filter(|outcome| {
                outcome.action_id.starts_with(BATCH_ACTION_PREFIX)
                    && outcome.disposition == FiscalActionDisposition::Applied
            })
            .count();
    }

    let mut report = json!({
        "profile": "fiscal-boundary",
        "metric": if options.count_allocations { "allocation_operations" } else { "elapsed_ms" },
        "machine": options.machine,
        "recorded_on": options.recorded_on,
        "dimensions": {
            "institutions": options.institutions,
            "settled_cycles": options.cycles,
            "assessments": options.institutions * options.cycles,
            "execution_requests": options.institutions * options.cycles,
            "observers": options.observers,
            "observed_institutions": options.observed_institutions,
            "fiscal_state_json_bytes": state_bytes,
            "report_facts_per_observer": options.observed_institutions * options.cycles,
            "report_records_per_observer": report_records,
            "report_json_bytes_per_observer": report_bytes,
        },
        "samples": options.samples,
        "fiscal_state_component_ms": components,
        "idle_boundary_ms": {
            "without_fiscal_records": summary(&floor_ms),
            "initial_fiscal_state": summary(&idle_initial_ms),
            "settled_fiscal_state": summary(&idle_settled_ms),
        },
        "action_batch": {
            "actions": options.batch,
            "applied": applied,
            "report_records_published": published.0,
            "report_json_bytes_published": published.1,
            "admission_boundary_ms": summary(&admission_ms),
            "settlement_boundary_ms": summary(&settlement_ms),
        },
    });
    if options.count_allocations {
        relabel(&mut report);
    }
    let rendered = serde_json::to_string_pretty(&report)?;
    if let Some(path) = &options.output {
        fs::write(path, format!("{rendered}\n"))?;
    }
    println!("{rendered}");
    Ok(())
}

/// Times the operations a fiscal handler can perform on the whole state, so
/// a boundary time can be read as a count of them.
fn state_component_costs(canwu: &Canwu, options: &Options) -> Result<Value, Box<dyn Error>> {
    let catalog = catalog()?;
    let record = canwu
        .typed_domain_record(&fiscal_state_reference())
        .ok_or("the profile run has no fiscal state")?;
    let state = record.decode_payload::<FiscalStateRecord>()?;
    let mut decode = Vec::with_capacity(options.samples);
    let mut clone = Vec::with_capacity(options.samples);
    let mut validate = Vec::with_capacity(options.samples);
    let mut encode = Vec::with_capacity(options.samples);
    let mut measure = Vec::with_capacity(options.samples);
    for _ in 0..options.samples {
        let started = Meter::start();
        let decoded = record.decode_payload::<FiscalStateRecord>()?;
        decode.push(started.finish());
        let started = Meter::start();
        let copy = decoded.clone();
        clone.push(started.finish());
        let started = Meter::start();
        copy.validate(&catalog)?;
        validate.push(started.finish());
        let started = Meter::start();
        let encoded = serde_json::to_value(&state)?;
        encode.push(started.finish());
        // What settlement adds to its one encoding of the state to measure the
        // records the derived systems of its boundary will write: the derived
        // candidates and aggregates, and the encoded size of those fields
        // before and after.
        let started = Meter::start();
        let candidates = compute_transition_candidates(&state, &catalog, SimTime::EPOCH);
        let aggregates = compute_aggregates(&state, &catalog)?;
        let sizes = [
            serde_json::to_vec(&state.transition_candidates)?.len(),
            serde_json::to_vec(&candidates)?.len(),
            serde_json::to_vec(&state.aggregates)?.len(),
            serde_json::to_vec(&aggregates)?.len(),
        ];
        measure.push(started.finish());
        drop((decoded, copy, encoded, candidates, aggregates, sizes));
    }
    Ok(json!({
        "decode": summary(&decode),
        "clone": summary(&clone),
        "validate": summary(&validate),
        "encode": summary(&encode),
        "settlement_measure": summary(&measure),
    }))
}

/// Times boundaries that admit one tick and no fiscal ingress.
fn measure_idle(canwu: &mut Canwu, options: &Options) -> Result<Vec<f64>, Box<dyn Error>> {
    let mut samples = Vec::with_capacity(options.samples);
    for sample in 0..options.warmup + options.samples {
        canwu.enqueue_plugin_ingress(PluginIngressRequest::new(
            TICK_PLUGIN,
            TICK_INGRESS,
            canwu.time(),
            json!({}),
        ))?;
        let started = Meter::start();
        canwu.settle_boundary(BoundaryRequest::at(canwu.time()))?;
        let elapsed = started.finish();
        if sample >= options.warmup {
            samples.push(elapsed);
        }
    }
    Ok(samples)
}

/// Enqueues one assessment action per institution in `first..first + count`,
/// each from that institution's own principal, then times the boundary that
/// admits the commands and the boundary that settles their actions.
fn settle_actions(
    canwu: &mut Canwu,
    first: usize,
    count: usize,
    cycle: &str,
) -> Result<(f64, f64), Box<dyn Error>> {
    let procedure_revision = fiscal_state(canwu)?.procedure_revision;
    let revision = canwu.revision();
    for (offset, institution) in (first..first + count).enumerate() {
        let request = FiscalActionRequest {
            action_id: format!("profile.{cycle}.{institution}"),
            authority_binding_id: authority_id(institution),
            expected_procedure_revision: procedure_revision,
            action: FiscalAction::OpenAssessment {
                assessment_id: format!("assess.{institution}.{cycle}"),
                rule_id: "rule.land".to_owned(),
                scope_binding_id: scope_id(institution),
                accounting_cycle_id: cycle.to_owned(),
                quantity: 1_000,
                unit: "grain".to_owned(),
                payment_form: FiscalPaymentForm::Grain,
                commutation_quote: None,
            },
        };
        let offset = u64::try_from(offset)?;
        canwu.enqueue_command(
            canwu.time(),
            0,
            CommandRequest::new(
                CommandRequestId::new(revision + offset + 1),
                revision + offset,
                CommandEnvelope::new(
                    Issuer::Actor(principal(institution)?),
                    fiscal_action_command(&request)?,
                )
                .at_time(canwu.time()),
            ),
        )?;
    }
    let started = Meter::start();
    canwu.settle_boundary(BoundaryRequest::at(canwu.time()))?;
    let admission = started.finish();
    let started = Meter::start();
    canwu.settle_boundary(BoundaryRequest::at(canwu.time()))?;
    Ok((admission, started.finish()))
}

/// The knowledge records the first `observers` observers hold, and the bytes
/// of their payloads. Nothing but fiscal reports is published to them.
fn observer_reports(canwu: &Canwu, observers: usize) -> Result<(usize, usize), Box<dyn Error>> {
    let mut held = (0, 0);
    for observer in 0..observers {
        let holder = KnowledgeHolderRef::Person(principal(observer)?);
        for record in canwu.knowledge().for_holder(&holder).into_iter().flatten() {
            held.0 += 1;
            held.1 += serde_json::to_vec(&record.1.payload)?.len();
        }
    }
    Ok(held)
}

fn fiscal_state(canwu: &Canwu) -> Result<FiscalState, Box<dyn Error>> {
    Ok(canwu
        .typed_domain_record(&fiscal_state_reference())
        .ok_or("the profile run has no fiscal state")?
        .decode_payload::<FiscalStateRecord>()?)
}

fn authority_id(institution: usize) -> String {
    format!("authority.{institution}")
}

fn scope_id(institution: usize) -> String {
    format!("scope.{institution}")
}

fn principal(institution: usize) -> Result<PersonId, Box<dyn Error>> {
    Ok(PersonId::new(u64::try_from(institution)? + 1))
}

fn institution_entity(institution: usize) -> Result<EntityRef, Box<dyn Error>> {
    Ok(EntityRef::Organization(OrganizationId::new(
        u64::try_from(institution)? + 1,
    )))
}

fn scenario(options: &Options, with_fiscal_records: bool) -> Result<Scenario, Box<dyn Error>> {
    let government = GovernmentId::new(1);
    let territory = TerritoryId::new(1);
    let mut people = Vec::with_capacity(options.institutions);
    for institution in 0..options.institutions {
        people.push(Person {
            id: principal(institution)?,
            name: format!("Principal {institution}"),
            government,
            current_location: territory,
            roles: Vec::new(),
            transit: None,
        });
    }
    let world = WorldSnapshot {
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
    let mut entities = world.entities();
    entities.push(EntityRef::Resource(ResourceId::new(1)));
    for institution in 0..options.institutions {
        entities.push(institution_entity(institution)?);
    }
    let mut domain_records = Vec::new();
    if with_fiscal_records {
        let catalog = catalog()?;
        let state = fiscal_history(options, &catalog)?;
        domain_records.push(catalog.clone().into_record()?);
        domain_records.push(state.into_record(&catalog)?);
    }
    Ok(Scenario {
        start_time: SimTime::EPOCH,
        entities,
        world,
        knowledge: KnowledgeSnapshot::default(),
        domain_records,
    })
}

fn fiscal_history(
    options: &Options,
    catalog: &CompiledFiscalCatalog,
) -> Result<FiscalState, Box<dyn Error>> {
    let territory = EntityRef::Territory(TerritoryId::new(1));
    let mut state = FiscalState::new(150, FiscalHistoricalMode::Counterfactual, SimTime::EPOCH);
    for institution in 0..options.institutions {
        let entity = institution_entity(institution)?;
        // Decoded rather than written as a literal so the same source measures
        // releases whose binding has a different set of optional fields.
        let binding: FiscalAuthorityBinding = serde_json::from_value(json!({
            "id": authority_id(institution),
            "institution": entity,
            "authorized_actor": principal(institution)?,
        }))?;
        state.authority_bindings.insert(binding.id.clone(), binding);
        state.scope_bindings.insert(
            scope_id(institution),
            FiscalScopeBinding {
                id: scope_id(institution),
                institution: entity.clone(),
                jurisdiction_id: "region.a".to_owned(),
                subject_scope: "registered_land".to_owned(),
                mechanism: FiscalMechanism::LandTax,
                authoritative_granularity: SimulationGranularity::Aggregate,
            },
        );
        let adoption_id = format!("adopt.{institution}");
        state.adoptions.insert(
            adoption_id.clone(),
            FiscalAdoptionState {
                id: adoption_id,
                rule_id: "rule.land".to_owned(),
                scope_binding_id: scope_id(institution),
                stage: FiscalAdoptionStage::Implemented,
                generation: 1,
                changed_at: SimTime::EPOCH,
                source_action_id: None,
            },
        );
        for cycle in 0..options.cycles {
            let assessment_id = format!("assess.{institution}.{cycle}");
            let request_id = format!("request.{institution}.{cycle}");
            state.assessments.insert(
                assessment_id.clone(),
                FiscalAssessment {
                    id: assessment_id.clone(),
                    rule_id: "rule.land".to_owned(),
                    scope_binding_id: scope_id(institution),
                    accounting_cycle_id: format!("cycle.{cycle}"),
                    quantity: 1_000,
                    unit: "grain".to_owned(),
                    payment_form: FiscalPaymentForm::Grain,
                    commutation_quote: None,
                    created_at: SimTime::EPOCH,
                },
            );
            state.execution_requests.insert(
                request_id.clone(),
                FiscalExecutionRequest {
                    id: request_id.clone(),
                    assessment_id: assessment_id.clone(),
                    institution: entity.clone(),
                    kind: FiscalExecutionKind::Collect,
                    quantity: 600,
                    unit: "grain".to_owned(),
                    payment_form: FiscalPaymentForm::Grain,
                    resource: ResourceId::new(1),
                    source: territory.clone(),
                    target: entity.clone(),
                    purpose: "Profile collection".to_owned(),
                    requested_at: SimTime::EPOCH,
                },
            );
            for action_id in [
                format!("history.{assessment_id}"),
                format!("history.{request_id}"),
            ] {
                state.action_outcomes.insert(
                    action_id.clone(),
                    FiscalActionOutcome {
                        action_id,
                        disposition: FiscalActionDisposition::Applied,
                        reason: "applied".to_owned(),
                        command: CommandId::new(1),
                        settled_at: SimTime::EPOCH,
                    },
                );
            }
        }
    }
    // Each observer sees its own run of institutions. Two of them keep its
    // report inside one knowledge record, which is all 0.14.0 could publish.
    for observer in 0..options.observers {
        let actor = principal(observer)?;
        let id = format!("observer.{observer}");
        let first = observer * options.observed_institutions;
        state.observer_bindings.insert(
            id.clone(),
            FiscalObserverBinding {
                id,
                actor,
                knowledge_holder: KnowledgeHolderRef::Person(actor),
                visible_institutions: (first..first + options.observed_institutions)
                    .map(institution_entity)
                    .collect::<Result<_, _>>()?,
                confidence_per_mille: 850,
            },
        );
    }
    recompute_derived(&mut state, catalog, SimTime::EPOCH)?;
    Ok(state)
}

fn catalog() -> Result<CompiledFiscalCatalog, Box<dyn Error>> {
    let window = HistoricalYearWindow {
        start: 100,
        end: 200,
    };
    let provenance = BTreeSet::from(["source.profile".to_owned()]);
    let pack = FiscalContentPack {
        manifest: FiscalPackManifest {
            pack_id: "profile.fiscal".to_owned(),
            pack_version: "1.0.0".to_owned(),
            schema_version: 1,
            license: "Apache-2.0".to_owned(),
            title: "Fiscal boundary profile".to_owned(),
            historical_scope: window.clone(),
            period_ids: vec!["period.a".to_owned()],
            region_ids: vec!["region.a".to_owned()],
            mechanisms: vec![FiscalMechanism::LandTax],
        },
        periods: vec![FiscalPeriodDefinition {
            id: "period.a".to_owned(),
            label: "Profile period".to_owned(),
            window: window.clone(),
            branch: None,
        }],
        regions: vec![FiscalRegionDefinition {
            id: "region.a".to_owned(),
            label: "Profile region".to_owned(),
        }],
        institutions: Vec::new(),
        rules: vec![FiscalRuleDefinition {
            id: "rule.land".to_owned(),
            revision: 1,
            label: "Land levy".to_owned(),
            mechanism: FiscalMechanism::LandTax,
            legal_window: window,
            jurisdiction_ids: BTreeSet::from(["region.a".to_owned()]),
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
            definition_ids: BTreeSet::from(["rule.land".to_owned()]),
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

fn options() -> Result<Options, Box<dyn Error>> {
    let mut options = Options {
        count_allocations: false,
        institutions: 48,
        cycles: 40,
        batch: 40,
        observers: 4,
        observed_institutions: 2,
        samples: 31,
        warmup: 3,
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
            "--institutions" => options.institutions = value()?.parse()?,
            "--cycles" => options.cycles = value()?.parse()?,
            "--batch" => options.batch = value()?.parse()?,
            "--observers" => options.observers = value()?.parse()?,
            "--observed-institutions" => options.observed_institutions = value()?.parse()?,
            "--samples" => options.samples = value()?.parse()?,
            "--warmup" => options.warmup = value()?.parse()?,
            "--machine" => options.machine = value()?,
            "--recorded-on" => options.recorded_on = value()?,
            "--output" => options.output = Some(PathBuf::from(value()?)),
            "--help" | "-h" => {
                println!(
                    "fiscal-profile [--mode elapsed|allocations] [--institutions N] [--cycles N] \
                     [--batch N] [--observers N] \
                     [--observed-institutions N] [--samples N] [--warmup N] \
                     [--machine LABEL] [--recorded-on DATE] [--output PATH]"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument {other}").into()),
        }
    }
    if options.institutions < 2
        || options.cycles == 0
        || options.samples == 0
        || options.batch == 0
        || options.batch >= options.institutions
        || options.observed_institutions == 0
        || options.observers * options.observed_institutions > options.institutions
    {
        return Err(
            "the profile needs at least two institutions, one settled cycle, one sample, a \
             batch smaller than the institution count, and its own institutions for every \
             observer"
                .into(),
        );
    }
    Ok(options)
}
