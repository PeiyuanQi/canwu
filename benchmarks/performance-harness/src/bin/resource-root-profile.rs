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
//!
//! `--accounts` adds a second dimension: each case also installs that many
//! live accounts (opening balances of one resource), so the cases show what
//! the live part of the root costs a boundary beside the hot terminal
//! backlog.
//!
//! `--households` runs the lease-budget cases on a detached root (no run):
//! that many households, each with one account and one allocated demand,
//! settle one consumption each. `per_household` takes one single-step
//! completion lease per consumption and reports how many leases fit the
//! candidate budget at once, what one lease adds to the live root, and the
//! terminal records the consumptions leave; `batch` settles the same
//! consumptions as consumption batches, each under one lease.
//!
//! The scale cases measure single boundaries that a raised limit allows,
//! one sample each on a fresh root (`--samples` roots per size):
//! `--pooled` settles one allocation pass of a pooled demand beside that
//! many accounts, which attaches every account; `--due` settles one
//! allocation pass over that many due demands, each on an account of its
//! own; `--boundary-batch` settles one consumption batch of that many items
//! through a provider's adapter ingress, under one four-step lease.

use canwu_api::{
    BoundaryRequest, Canwu, CanwuError, DomainRecord, DomainRecordClass, DomainRecordKind,
    DomainRecordLifecycle, DomainRecordRef, DomainRecordSchema, DomainRecordType,
    DomainRecordVersionRef, DomainRecordVersionSource, DomainValueKindClass, EntityRef,
    IngressClass, KnowledgeHolderRef, PayloadSchema, PersonId, PluginIngressDescriptor,
    PluginIngressRequest, PluginRegistrar, Scenario, SimDuration, SimTime, SimulationPlugin,
};
use canwu_resource::{
    ActivateCompletionLeaseV1, GrantCompletionCapacityV1, PrepareCompletionCapacityV1,
    ResourceAdapterOperationV1, ResourceConsumptionIntentId, ResourceConsumptionIntentStatusV1,
    ResourceConsumptionIntentV1, enqueue_resource_adapter_operation,
    enqueue_resource_completion_operation, resource_consumption_batch_item_key,
};
use canwu_resource::{
    CompletionCapacityGrantId, CompletionCapacityPartitionV1, CompletionCapacityRecipeV1,
    CompletionLeaseAcquisitionId, CompletionLeaseActivationCertificateV1, CompletionLockedTargetV1,
    CompletionPolicyClassV1, DemandStatus, EligibilityEnvelopeV1,
    MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE, MAX_RESOURCE_CONSUMPTION_BATCH_ITEMS,
    PartialFulfillmentPolicy, RequestCompletionLeaseV1, ResourceAccount, ResourceAccountId,
    ResourceAllocationLeg, ResourceAllocationLegVersionV1, ResourceAllocationRequestV1,
    ResourceArchiveRetentionHandleV1, ResourceArchiveRetentionPhaseV1, ResourceArchiveStore,
    ResourceCancelDemandRequestV1, ResourceCompletionOperationV1, ResourceConsumptionBatchItemV1,
    ResourceConsumptionBatchRequestV1, ResourceConsumptionId, ResourceConsumptionRequestV1,
    ResourceDefinitionId, ResourceDefinitionRevision, ResourceDefinitionRevisionId, ResourceDemand,
    ResourceDemandId, ResourceDemandSourcePolicyV1, ResourceError, ResourceLimitsV1,
    ResourceOperationKey, ResourceOperationOutcome, ResourceOperationOutcomeId,
    ResourceOperationRequestV1, ResourceOperationStatus, ResourcePlugin, ResourceQualityId,
    ResourceRevision, ResourceScopeId, ResourceState, ResourceTerminalRecordKeyV1,
    ResourceTieBreakKey, ResourceUnitRevision, ResourceUnitRevisionId, RunBudgetRevisionV1,
    SingleStepCompletionLeaseV1, canonical_digest, enqueue_resource_allocation,
    enqueue_resource_archive, resource_runtime_reference, resource_state,
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
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::sync::RwLock;
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
    objects: Arc<RwLock<ArchiveObjects>>,
}

impl ResourceArchiveStore for MemoryArchive {
    fn store_resource_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
        bytes: &[u8],
    ) -> Result<(), ResourceError> {
        self.objects
            .write()
            .unwrap()
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
            .read()
            .unwrap()
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
            .read()
            .unwrap()
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

fn milliseconds(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1_000.0
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

fn unit_revision() -> Result<ResourceUnitRevisionId, ResourceError> {
    ResourceUnitRevisionId::new("profile:unit:measure:v1")
}

fn resource_revision() -> Result<ResourceDefinitionRevisionId, ResourceError> {
    ResourceDefinitionRevisionId::new("profile:resource:grain:v1")
}

/// Installs the one resource and unit every profile account holds.
fn install_catalog(state: &mut ResourceState) -> Result<(), ResourceError> {
    state.install_unit(ResourceUnitRevision {
        id: unit_revision()?,
        revision: ResourceRevision::INITIAL,
        symbol: "u".to_owned(),
        scale_numerator: 1,
        scale_denominator: 1,
        semantic_digest: "0".repeat(64),
    })?;
    state.install_definition(ResourceDefinitionRevision {
        id: resource_revision()?,
        resource: ResourceDefinitionId::new("profile:resource:grain")?,
        revision: ResourceRevision::INITIAL,
        canonical_unit: unit_revision()?,
        quality: ResourceQualityId::new("profile:quality:standard")?,
        scope: ResourceScopeId::new("profile:scope:world")?,
        effective_from: SimTime::EPOCH,
        effective_until: None,
        process_suitability: std::collections::BTreeSet::new(),
        semantic_digest: "1".repeat(64),
    })
}

fn account_id(label: &str, index: usize) -> Result<ResourceAccountId, ResourceError> {
    ResourceAccountId::new(format!("profile:account:{label}:{index:06}"))
}

/// Installs `count` live accounts, each with an opening balance.
fn install_accounts(
    state: &mut ResourceState,
    label: &str,
    count: usize,
    custodian: impl Fn(usize) -> KnowledgeHolderRef,
) -> Result<(), ResourceError> {
    for index in 0..count {
        state.install_opening_account(ResourceAccount {
            id: account_id(label, index)?,
            revision: ResourceRevision::INITIAL,
            custodian: custodian(index),
            resource_revision: resource_revision()?,
            unit_revision: unit_revision()?,
            balance: 1_000,
            capacity: None,
            protected_floor_policy: None,
            closed: false,
            place_scope: None,
        })?;
    }
    Ok(())
}

fn case(
    template: &ResourceOperationOutcome,
    backlog: usize,
    accounts: usize,
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
    limits.max_accounts = limits.max_accounts.max(accounts);
    let mut state = ResourceState::empty(limits)?;
    if accounts > 0 {
        install_catalog(&mut state)?;
        install_accounts(&mut state, "live", accounts, |index| {
            KnowledgeHolderRef::Person(PersonId::new(index as u64 + 2))
        })?;
    }
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
    canwu.set_plugin_archive_object_provider(Arc::new(archive.clone()));
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
    let live_root_bytes = canwu
        .domain_record(resource_runtime_reference().as_untyped())
        .map(|record| serde_json::to_vec(&record.payload).map(|bytes| bytes.len()))
        .transpose()?
        .unwrap_or_default();

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
        "live_accounts": accounts,
        "live_root_bytes": live_root_bytes,
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

fn household(index: usize) -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(PersonId::new(index as u64 + 1))
}

const HOUSEHOLD_NAMESPACE: &str = "profile.household";

fn partition(authority: KnowledgeHolderRef) -> CompletionCapacityPartitionV1 {
    CompletionCapacityPartitionV1 {
        authority,
        operation_namespace: HOUSEHOLD_NAMESPACE.to_owned(),
        guaranteed_units: 40_000,
        reserved_pending_slots: 1,
        maximum_burst_units: 40_000,
        request_token_capacity: 1,
        request_token_refill_minutes: 1,
        reacquire_cooldown_minutes: 1,
        root_acquisition_cap_per_sim_time: 1,
        guaranteed_max_wait_boundaries: 1,
    }
}

/// The settlement record a consumption cites. Two leases cannot lock the
/// same record version at once, so each per-household lease cites a record
/// of its own; a batch cites one for all its items.
fn consumer_evidence(label: &str) -> DomainRecordVersionRef {
    DomainRecordVersionRef {
        record: DomainRecordRef::new(
            "profile.provider",
            "settlement",
            format!("profile:settlement:{label}"),
        ),
        version: 1,
        established_by: DomainRecordVersionSource::InitialScenario,
    }
}

fn lease_recipe() -> CompletionCapacityRecipeV1 {
    CompletionCapacityRecipeV1 {
        receipts: MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE,
        mutations: 2,
        reports_per_holder: 0,
        holders: 0,
        bytes: 1_024,
    }
}

/// A single-step lease for `operation_key`, held by `holder`, locking
/// `targets` and the consumer evidence `evidence`.
fn single_step(
    holder: KnowledgeHolderRef,
    suffix: &str,
    operation_key: &ResourceOperationKey,
    mut targets: Vec<CompletionLockedTargetV1>,
    evidence: &DomainRecordVersionRef,
) -> Result<ResourceOperationRequestV1, ResourceError> {
    targets.push(CompletionLockedTargetV1::ExternalRecord {
        version: evidence.clone(),
    });
    Ok(ResourceOperationRequestV1::Completion(
        ResourceCompletionOperationV1::SingleStepLease(SingleStepCompletionLeaseV1 {
            lease: RequestCompletionLeaseV1 {
                id: CompletionLeaseAcquisitionId::new(format!("profile:lease:{suffix}"))?,
                operation_key: operation_key.clone(),
                holder,
                operation_namespace: HOUSEHOLD_NAMESPACE.to_owned(),
                eligibility_time: SimTime::EPOCH,
                eligibility_envelope: EligibilityEnvelopeV1::new(
                    vec![evidence.clone()],
                    BTreeMap::new(),
                    std::collections::BTreeSet::new(),
                    Vec::new(),
                    Vec::new(),
                )?,
                recipe: lease_recipe(),
                expected_participants: std::collections::BTreeSet::from([
                    canwu_resource::PLUGIN_NAME.to_owned(),
                ]),
                policy_class: CompletionPolicyClassV1::Guaranteed,
            },
            grant_id: CompletionCapacityGrantId::new(format!("profile:grant:{suffix}"))?,
            target_versions: targets,
            current_boundary: 1,
        }),
    ))
}

/// The targets one consumption of `leg` locks, beside its evidence.
fn consumption_targets(
    state: &ResourceState,
    leg: &ResourceAllocationLeg,
) -> Vec<CompletionLockedTargetV1> {
    vec![
        CompletionLockedTargetV1::Account {
            id: leg.account.clone(),
            revision: state.accounts[&leg.account].revision,
        },
        CompletionLockedTargetV1::AllocationLeg {
            id: leg.id.clone(),
            revision: leg.revision,
        },
        CompletionLockedTargetV1::Demand {
            id: leg.demand.clone(),
            revision: leg.demand_revision,
        },
    ]
}

/// A detached root holding `households` accounts, one allocated demand
/// each, whose requester is `requester(index)`, and a run budget with a
/// partition for every lease holder in `holders`.
fn household_root(
    households: usize,
    requester: &dyn Fn(usize) -> KnowledgeHolderRef,
    holders: &[KnowledgeHolderRef],
) -> Result<ResourceState, Box<dyn Error>> {
    let mut limits = ResourceLimitsV1::canonical();
    limits.max_archive_candidates = ResourceLimitsV1::MAX_ARCHIVE_CANDIDATES;
    limits.max_archive_maintenance_receipts = ResourceLimitsV1::MAX_ARCHIVE_CANDIDATES;
    limits.max_operation_outcomes = ResourceLimitsV1::MAX_OPERATION_OUTCOMES;
    limits.max_accounts = limits.max_accounts.max(households);
    limits.max_demands = limits.max_demands.max(households);
    limits.max_batch_items_per_boundary = ResourceLimitsV1::MAX_BATCH_ITEMS_PER_BOUNDARY;
    let mut state = ResourceState::empty(limits)?;
    state.install_run_budget(
        RunBudgetRevisionV1 {
            revision: ResourceRevision::INITIAL,
            total_completion_units: 40_000 * holders.len() as u64,
            shared_pending_slots: 0,
            partitions: holders.iter().cloned().map(partition).collect(),
            semantic_digest: String::new(),
        }
        .seal()?,
    )?;
    install_catalog(&mut state)?;
    install_accounts(&mut state, "household", households, requester)?;
    for index in 0..households {
        state.install_demand(ResourceDemand {
            id: ResourceDemandId::new(format!("profile:demand:household:{index:06}"))?,
            revision: ResourceRevision::INITIAL,
            requester: requester(index),
            source_policy: ResourceDemandSourcePolicyV1::ExactAccounts(vec![account_id(
                "household",
                index,
            )?]),
            resource_revision: resource_revision()?,
            unit_revision: unit_revision()?,
            requested: 10,
            fulfilled: 0,
            minimum_useful: 10,
            partial_fulfillment: PartialFulfillmentPolicy::RejectPartial,
            alternative_group: None,
            due_at: SimTime::EPOCH,
            expires_at: SimTime::EPOCH + SimDuration::minutes(60 * 24 * 30),
            priority: 0,
            tie_break: ResourceTieBreakKey::new(format!("profile:tie:{index:06}"))?,
            admitted_sequence: 0,
            protected_floor_policy: None,
            protection_override_class: None,
            status: DemandStatus::Open,
            rejection_reason: None,
        })?;
    }
    let pass = state.apply_operation(&ResourceOperationRequestV1::Allocate(
        ResourceAllocationRequestV1 {
            operation_key: operation_key("allocate", 0)?,
            expected_state_revision: state.state_revision,
            at: SimTime::EPOCH,
            candidate_limit: households,
        },
    ))?;
    if pass.status != ResourceOperationStatus::Applied || pass.quantity != 10 * households as u64 {
        return Err(format!("the allocation pass did not reserve every demand: {pass:?}").into());
    }
    Ok(state)
}

/// Encoded bytes of the live root of `state` and of all its records.
fn root_bytes(state: &ResourceState) -> Result<(usize, usize), Box<dyn Error>> {
    let records = state.clone().into_records()?;
    let mut total = 0;
    for record in &records {
        total += serde_json::to_vec(&record.payload)?.len();
    }
    Ok((serde_json::to_vec(&records[0].payload)?.len(), total))
}

fn applied(
    state: &mut ResourceState,
    request: &ResourceOperationRequestV1,
) -> Result<ResourceOperationOutcome, Box<dyn Error>> {
    let outcome = state.apply_operation(request)?;
    if outcome.status != ResourceOperationStatus::Applied {
        return Err(format!(
            "{} was refused: {:?}",
            outcome.operation_key, outcome.rejection_reason
        )
        .into());
    }
    Ok(outcome)
}

/// The same consumptions as consumption batches of at most
/// `MAX_RESOURCE_CONSUMPTION_BATCH_ITEMS` items, each under one single-step
/// lease held by one settlement office that requested every demand. One
/// holder takes at most one lease per simulated minute, so batch `k` is
/// leased and settled at minute `k`.
fn batch_case(households: usize) -> Result<Value, Box<dyn Error>> {
    let office = household(0);
    let holders = [office.clone()];
    let mut state = household_root(households, &|_| office.clone(), &holders)?;
    let legs: Vec<ResourceAllocationLeg> = state.allocation_legs.values().cloned().collect();
    let (live_before, _) = root_bytes(&state)?;
    let hot_before = state.hot_terminal_records();
    let mut lease_ms = 0.0;
    let mut batch_ms = 0.0;
    let mut live_leased = 0;
    let mut batches = 0;
    let mut refused = 0;
    for (chunk, legs) in legs
        .chunks(MAX_RESOURCE_CONSUMPTION_BATCH_ITEMS)
        .enumerate()
    {
        let at = SimTime::EPOCH + SimDuration::minutes(chunk as i64);
        let evidence = consumer_evidence(&format!("batch:{chunk}"));
        let items = legs
            .iter()
            .map(|leg| {
                Ok(ResourceConsumptionBatchItemV1 {
                    consumption_id: ResourceConsumptionId::new(format!(
                        "profile:consumption:{}",
                        leg.id
                    ))?,
                    allocation: ResourceAllocationLegVersionV1::from(leg),
                    demand: leg.demand.clone(),
                    demand_revision: leg.demand_revision,
                    expected_account_revision: state.accounts[&leg.account].revision,
                })
            })
            .collect::<Result<Vec<_>, ResourceError>>()?;
        let mut batch = ResourceConsumptionBatchRequestV1 {
            operation_key: operation_key("batch", chunk)?,
            items,
            consumer_evidence: evidence.clone(),
            at,
            completion_certificate: placeholder_certificate()?,
        };
        let targets = batch
            .locked_targets()
            .into_iter()
            .filter(|target| !matches!(target, CompletionLockedTargetV1::ExternalRecord { .. }))
            .collect();
        let suffix = format!("batch:{chunk}");
        let mut lease = single_step(
            office.clone(),
            &suffix,
            &batch.operation_key,
            targets,
            &evidence,
        )?;
        if let ResourceOperationRequestV1::Completion(
            ResourceCompletionOperationV1::SingleStepLease(single),
        ) = &mut lease
        {
            single.lease.eligibility_time = at;
        }
        let started = Instant::now();
        applied(&mut state, &lease)?;
        lease_ms += milliseconds(started);
        live_leased = live_leased.max(root_bytes(&state)?.0);
        batch.completion_certificate = state
            .completion_leases
            .certificate(&CompletionLeaseAcquisitionId::new(format!(
                "profile:lease:{suffix}"
            ))?)
            .cloned()
            .ok_or("an activated lease has no certificate")?;
        let started = Instant::now();
        let outcome = applied(&mut state, &ResourceOperationRequestV1::ConsumeBatch(batch))?;
        batch_ms += milliseconds(started);
        refused += outcome.remainder;
        batches += 1;
    }
    let (live_after, _) = root_bytes(&state)?;
    Ok(json!({
        "households": households,
        "batches": batches,
        "refused_items": refused,
        "lease_ms_per_batch": lease_ms / batches.max(1) as f64,
        "batch_ms": batch_ms / batches.max(1) as f64,
        "batch_ms_per_item": batch_ms / households.max(1) as f64,
        "live_root_bytes_before": live_before,
        "live_root_bytes_with_an_active_batch_lease": live_leased,
        "live_root_bytes_after_consumptions": live_after,
        "finished_leases_kept_in_live_root": state.completion_leases.acquisitions.len(),
        "hot_terminal_records_added": state.hot_terminal_records() - hot_before,
        "hot_terminal_records_per_consumption":
            (state.hot_terminal_records() - hot_before) as f64 / households.max(1) as f64,
    }))
}

/// A certificate to replace once the batch's lease is active.
fn placeholder_certificate() -> Result<CompletionLeaseActivationCertificateV1, ResourceError> {
    Ok(CompletionLeaseActivationCertificateV1 {
        acquisition: CompletionLeaseAcquisitionId::new("profile:lease:placeholder")?,
        acquisition_revision: ResourceRevision::INITIAL,
        operation_key: operation_key("placeholder", 0)?,
        prepared_grants: Vec::new(),
        locked_target_versions: Vec::new(),
        recipe_digest: String::new(),
        eligibility_time: SimTime::EPOCH,
        eligibility_envelope_digest: String::new(),
        activation_boundary: 0,
        semantic_digest: String::new(),
    })
}

/// One single-step lease and one consumption per household.
fn per_household_case(households: usize) -> Result<Value, Box<dyn Error>> {
    let holders: Vec<_> = (0..households).map(household).collect();
    let mut state = household_root(households, &household, &holders)?;
    let legs: Vec<ResourceAllocationLeg> = state.allocation_legs.values().cloned().collect();
    let (live_before, _) = root_bytes(&state)?;
    let hot_before = state.hot_terminal_records();
    // Leases taken until the candidate budget refuses the next one.
    let started = Instant::now();
    let mut leased = Vec::new();
    for (index, leg) in legs.iter().enumerate() {
        let key = operation_key("consume", index)?;
        let request = single_step(
            household(index),
            &format!("household:{index:06}"),
            &key,
            consumption_targets(&state, leg),
            &consumer_evidence(&format!("household:{index:06}")),
        )?;
        match state.apply_operation(&request) {
            Ok(outcome) if outcome.status == ResourceOperationStatus::Applied => {
                leased.push((index, key));
            }
            Ok(outcome) => {
                return Err(
                    format!("lease {index} refused: {:?}", outcome.rejection_reason).into(),
                );
            }
            Err(ResourceError::LimitExceeded(_)) => break,
            Err(error) => return Err(error.into()),
        }
    }
    let lease_ms = milliseconds(started) / leased.len().max(1) as f64;
    let (live_leased, _) = root_bytes(&state)?;
    let started = Instant::now();
    for (index, key) in &leased {
        let leg = &legs[*index];
        let certificate = state
            .completion_leases
            .certificate(&CompletionLeaseAcquisitionId::new(format!(
                "profile:lease:household:{index:06}"
            ))?)
            .cloned()
            .ok_or("an activated lease has no certificate")?;
        let expected_account_revision = state.accounts[&leg.account].revision;
        applied(
            &mut state,
            &ResourceOperationRequestV1::Consume(ResourceConsumptionRequestV1 {
                operation_key: key.clone(),
                consumption_id: ResourceConsumptionId::new(format!(
                    "profile:consumption:{index:06}"
                ))?,
                allocation: ResourceAllocationLegVersionV1::from(leg),
                expected_account_revision,
                consumer_evidence: consumer_evidence(&format!("household:{index:06}")),
                at: SimTime::EPOCH,
                completion_certificate: certificate,
            }),
        )?;
    }
    let consume_ms = milliseconds(started) / leased.len().max(1) as f64;
    let (live_after, _) = root_bytes(&state)?;
    let finished_leases = state.completion_leases.acquisitions.len();
    Ok(json!({
        "households": households,
        "leases_that_fit": leased.len(),
        "candidate_budget": state.limits.max_archive_candidates,
        "lease_ms": lease_ms,
        "consume_ms": consume_ms,
        "live_root_bytes_before": live_before,
        "live_root_bytes_with_active_leases": live_leased,
        "live_root_bytes_per_active_lease":
            (live_leased.saturating_sub(live_before)) as f64 / leased.len().max(1) as f64,
        "live_root_bytes_after_consumptions": live_after,
        "finished_leases_kept_in_live_root": finished_leases,
        "hot_terminal_records_added": state.hot_terminal_records() - hot_before,
        "hot_terminal_records_per_consumption":
            (state.hot_terminal_records() - hot_before) as f64 / leased.len().max(1) as f64,
    }))
}

// ---------------------------------------------------------------------------
// Scale cases: single boundaries that raised limits allow.

const PROVIDER_PLUGIN: &str = "resource-root-profile-provider";

/// The provider's settlement record: the typed consumption intents of one
/// batch.
struct SettlementRecord;

impl DomainRecordType for SettlementRecord {
    type Payload = Value;
    type Class = DomainValueKindClass;
    const NAMESPACE: &'static str = "profile.provider";
    const NAME: &'static str = "settlement";
}

/// Owns the settlement record schema and nothing else.
struct ProviderPlugin;

impl SimulationPlugin for ProviderPlugin {
    fn name(&self) -> &'static str {
        PROVIDER_PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "00000000000000000000000000000000000000000000000000000000000000f3"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut schema = DomainRecordSchema::for_record::<SettlementRecord>();
        schema.payload_schema = PayloadSchema::Any;
        registrar.register_record_schema(schema)
    }
}

fn person(id: u64) -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(PersonId::new(id))
}

/// A run of `state` with the tick-free resource plugin and the provider.
fn run_of(
    state: ResourceState,
    extra: Vec<DomainRecord>,
    holders: u64,
) -> Result<Canwu, Box<dyn Error>> {
    let resource = ResourcePlugin::new([DomainRecordKind::for_type::<SettlementRecord>()]);
    let provider = ProviderPlugin;
    let plugins: [&dyn SimulationPlugin; 2] = [&resource, &provider];
    let mut records = root_records(state)?;
    records.extend(extra);
    let scenario = Scenario::new(
        SimTime::EPOCH,
        (1..=holders)
            .map(|id| EntityRef::Person(PersonId::new(id)))
            .collect(),
    )
    .with_domain_records(records);
    Ok(Canwu::new_with_plugins(PROFILE_SEED, scenario, &plugins)?)
}

/// One open demand of `requester` for 10 units, due at `due_at`.
fn open_demand(
    id: ResourceDemandId,
    requester: KnowledgeHolderRef,
    source_policy: ResourceDemandSourcePolicyV1,
    tie: &str,
    due_at: SimTime,
) -> Result<ResourceDemand, ResourceError> {
    Ok(ResourceDemand {
        id,
        revision: ResourceRevision::INITIAL,
        requester,
        source_policy,
        resource_revision: resource_revision()?,
        unit_revision: unit_revision()?,
        requested: 10,
        fulfilled: 0,
        minimum_useful: 10,
        partial_fulfillment: PartialFulfillmentPolicy::RejectPartial,
        alternative_group: None,
        due_at,
        expires_at: SimTime::EPOCH + SimDuration::minutes(60 * 24 * 30),
        priority: 0,
        tie_break: ResourceTieBreakKey::new(format!("profile:tie:{tie}"))?,
        admitted_sequence: 0,
        protected_floor_policy: None,
        protection_override_class: None,
        status: DemandStatus::Open,
        rejection_reason: None,
    })
}

/// Allocation passes of one pooled demand each beside `accounts` accounts
/// of other custodians, under the canonical limits: every pass attaches
/// every account. Pass `k` runs at minute `k`, when demand `k` falls due.
fn pooled_case(accounts: usize, options: &Options) -> Result<Value, Box<dyn Error>> {
    let mut limits = ResourceLimitsV1::canonical();
    limits.max_accounts = limits.max_accounts.max(accounts);
    let mut state = ResourceState::empty(limits)?;
    install_catalog(&mut state)?;
    install_accounts(&mut state, "pooled", accounts, |index| {
        person(index as u64 + 2)
    })?;
    let passes = options.warmup + options.samples;
    for index in 0..passes {
        state.install_demand(open_demand(
            ResourceDemandId::new(format!("profile:demand:pooled:{index:03}"))?,
            person(1),
            ResourceDemandSourcePolicyV1::Pooled,
            &format!("pooled:{index:03}"),
            SimTime::EPOCH + SimDuration::minutes(index as i64),
        )?)?;
    }
    state.validate()?;
    let mut canwu = run_of(state, Vec::new(), 1)?;
    let mut series = Series::new();
    for index in 0..passes {
        let at = SimTime::EPOCH + SimDuration::minutes(index as i64);
        let (_, root) = resource_state(&canwu)?.ok_or("the resource root is missing")?;
        enqueue_resource_allocation(
            &mut canwu,
            at,
            &person(1),
            &ResourceAllocationRequestV1 {
                operation_key: operation_key("pooled-pass", index)?,
                expected_state_revision: root.state_revision,
                at,
                // The demands allocated before stay open, so they are
                // candidates too.
                candidate_limit: index + 1,
            },
        )?;
        let cursor = canwu.evidence_cursor()?;
        let meter = Meter::start();
        canwu.settle_boundary(BoundaryRequest::at(at))?;
        let sample = meter.finish();
        let journal = serde_json::to_vec(&canwu.journal_segment_since(cursor)?)?.len();
        let measured = (sample, journal);
        if index >= options.warmup {
            series.push(measured);
        }
    }
    let (_, root) = resource_state(&canwu)?.ok_or("the resource root is missing")?;
    if root.allocation_legs.len() != passes {
        let outcomes: Vec<_> = (0..passes)
            .filter_map(|index| {
                operation_key("pooled-pass", index)
                    .ok()
                    .and_then(|key| root.outcomes.get(&key))
                    .map(|outcome| (outcome.status, outcome.rejection_reason.clone()))
            })
            .collect();
        return Err(format!(
            "a pooled pass did not allocate its demand: {} legs, {outcomes:?}",
            root.allocation_legs.len()
        )
        .into());
    }
    Ok(json!({
        "accounts": accounts,
        "pass": series.report(options.count_allocations),
    }))
}

/// One allocation pass over `demands` due demands of one requester, each on
/// an account of its own, under the canonical limits raised to hold them and
/// to consider them all in one pass.
fn due_case(demands: usize, options: &Options) -> Result<Value, Box<dyn Error>> {
    let mut passes = Vec::with_capacity(options.samples);
    let mut live_root_bytes = 0;
    for _ in 0..options.samples {
        let mut limits = ResourceLimitsV1::canonical();
        limits.max_demands = limits.max_demands.max(demands);
        limits.max_accounts = limits.max_accounts.max(demands);
        limits.max_allocation_candidates = limits.max_allocation_candidates.max(demands);
        limits.max_dirty_demands = limits.max_dirty_demands.max(demands);
        let mut state = ResourceState::empty(limits)?;
        install_catalog(&mut state)?;
        install_accounts(&mut state, "due", demands, |_| person(1))?;
        for index in 0..demands {
            state.install_demand(open_demand(
                ResourceDemandId::new(format!("profile:demand:due:{index:06}"))?,
                person(1),
                ResourceDemandSourcePolicyV1::ExactAccounts(vec![account_id("due", index)?]),
                &format!("due:{index:06}"),
                SimTime::EPOCH,
            )?)?;
        }
        let mut canwu = run_of(state, Vec::new(), 1)?;
        let (_, root) = resource_state(&canwu)?.ok_or("the resource root is missing")?;
        enqueue_resource_allocation(
            &mut canwu,
            SimTime::EPOCH,
            &person(1),
            &ResourceAllocationRequestV1 {
                operation_key: operation_key("due-pass", 0)?,
                expected_state_revision: root.state_revision,
                at: SimTime::EPOCH,
                candidate_limit: demands,
            },
        )?;
        passes.push(measured_boundary(&mut canwu)?);
        let (_, root) = resource_state(&canwu)?.ok_or("the resource root is missing")?;
        if root.allocation_legs.len() != demands {
            return Err("the due pass did not allocate every demand".into());
        }
        live_root_bytes = canwu
            .domain_record(resource_runtime_reference().as_untyped())
            .map(|record| serde_json::to_vec(&record.payload).map(|bytes| bytes.len()))
            .transpose()?
            .unwrap_or_default();
    }
    let mut series = Series::new();
    for measured in passes {
        series.push(measured);
    }
    Ok(json!({
        "demands": demands,
        "pass": series.report(options.count_allocations),
        "live_root_bytes_after": live_root_bytes,
    }))
}

/// Settles one completion step at the epoch.
fn completion_step(
    canwu: &mut Canwu,
    operation: ResourceCompletionOperationV1,
) -> Result<(), Box<dyn Error>> {
    enqueue_resource_completion_operation(canwu, SimTime::EPOCH, &operation)?;
    canwu.settle_boundary(BoundaryRequest::at(SimTime::EPOCH))?;
    Ok(())
}

fn next_boundary(canwu: &Canwu) -> u64 {
    canwu
        .boundaries()
        .last()
        .map_or(1, |boundary| boundary.id.get().saturating_add(1))
}

/// One consumption batch of `items` items through the provider's adapter
/// ingress, under one four-step lease held by the office that requested
/// every demand, on a root with the largest archive budget.
fn boundary_batch_case(items: usize, options: &Options) -> Result<Value, Box<dyn Error>> {
    let office = household(0);
    let mut batches = Series::new();
    let mut hot_added = 0;
    for sample in 0..options.samples {
        let state = household_root(items, &|_| office.clone(), std::slice::from_ref(&office))?;
        let batch_key = operation_key("boundary-batch", sample)?;
        let reference =
            DomainRecordRef::new("profile.provider", "settlement", "profile:settlement:batch");
        let legs: Vec<ResourceAllocationLeg> = state.allocation_legs.values().cloned().collect();
        let mut intents = serde_json::Map::new();
        let mut batch_items = Vec::with_capacity(legs.len());
        for (index, leg) in legs.iter().enumerate() {
            let consumption_id =
                ResourceConsumptionId::new(format!("profile:consumption:{}", leg.id))?;
            let expected_account_revision = state.accounts[&leg.account].revision;
            let intent = ResourceConsumptionIntentV1 {
                id: ResourceConsumptionIntentId::new(format!("profile:intent:{index:06}"))?,
                provider_plugin: PROVIDER_PLUGIN.to_owned(),
                demand: leg.demand.clone(),
                demand_revision: leg.demand_revision,
                allocation: leg.into(),
                account: leg.account.clone(),
                expected_account_revision,
                consumption_id: consumption_id.clone(),
                operation_key: resource_consumption_batch_item_key(&batch_key, index),
                quantity: leg.quantity,
                status: ResourceConsumptionIntentStatusV1::Authorized,
                semantic_digest: String::new(),
            }
            .seal()?;
            intents.insert(intent.id.to_string(), serde_json::to_value(&intent)?);
            batch_items.push(ResourceConsumptionBatchItemV1 {
                consumption_id,
                allocation: ResourceAllocationLegVersionV1::from(leg),
                demand: leg.demand.clone(),
                demand_revision: leg.demand_revision,
                expected_account_revision,
            });
        }
        let provider_record = DomainRecord {
            reference: reference.clone(),
            owner: PROVIDER_PLUGIN.to_owned(),
            class: DomainRecordClass::Record,
            version: 1,
            lifecycle: DomainRecordLifecycle::Active,
            payload: json!({ "resource_consumption_intents": intents }),
            references: Vec::new(),
        };
        let mut canwu = run_of(state, vec![provider_record], 1)?;
        let source = canwu
            .current_domain_record_version(&reference)?
            .ok_or("the settlement record has no current version")?;
        let mut batch = ResourceConsumptionBatchRequestV1 {
            operation_key: batch_key.clone(),
            items: batch_items,
            consumer_evidence: source.clone(),
            at: SimTime::EPOCH,
            completion_certificate: placeholder_certificate()?,
        };
        // The four steps: acquire, a resource grant and a provider grant,
        // their preparation, and the activation.
        let id = CompletionLeaseAcquisitionId::new(format!("profile:lease:boundary-{sample}"))?;
        let envelope = EligibilityEnvelopeV1::new(
            vec![source.clone()],
            BTreeMap::new(),
            std::collections::BTreeSet::new(),
            Vec::new(),
            Vec::new(),
        )?;
        completion_step(
            &mut canwu,
            ResourceCompletionOperationV1::Acquire(RequestCompletionLeaseV1 {
                id: id.clone(),
                operation_key: batch_key.clone(),
                holder: office.clone(),
                operation_namespace: HOUSEHOLD_NAMESPACE.to_owned(),
                eligibility_time: SimTime::EPOCH,
                eligibility_envelope: envelope.clone(),
                recipe: lease_recipe(),
                expected_participants: std::collections::BTreeSet::from([
                    canwu_resource::PLUGIN_NAME.to_owned(),
                    PROVIDER_PLUGIN.to_owned(),
                ]),
                policy_class: CompletionPolicyClassV1::Guaranteed,
            }),
        )?;
        let resource_grant =
            CompletionCapacityGrantId::new(format!("profile:grant:resource-{sample}"))?;
        let provider_grant =
            CompletionCapacityGrantId::new(format!("profile:grant:provider-{sample}"))?;
        let resource_targets: Vec<_> = batch
            .locked_targets()
            .into_iter()
            .filter(|target| !matches!(target, CompletionLockedTargetV1::ExternalRecord { .. }))
            .collect();
        for (grant, owner, targets) in [
            (
                resource_grant.clone(),
                canwu_resource::PLUGIN_NAME,
                resource_targets,
            ),
            (
                provider_grant.clone(),
                PROVIDER_PLUGIN,
                vec![CompletionLockedTargetV1::ExternalRecord {
                    version: source.clone(),
                }],
            ),
        ] {
            let (_, root) = resource_state(&canwu)?.ok_or("the resource root is missing")?;
            let current_boundary = next_boundary(&canwu);
            completion_step(
                &mut canwu,
                ResourceCompletionOperationV1::Grant(GrantCompletionCapacityV1 {
                    grant_id: grant,
                    acquisition: id.clone(),
                    expected_acquisition_revision: root.completion_leases.acquisitions[&id]
                        .revision,
                    owner_plugin: owner.to_owned(),
                    target_versions: targets,
                    current_boundary,
                }),
            )?;
        }
        for grant in [&resource_grant, &provider_grant] {
            let (_, root) = resource_state(&canwu)?.ok_or("the resource root is missing")?;
            let current_boundary = next_boundary(&canwu);
            completion_step(
                &mut canwu,
                ResourceCompletionOperationV1::Prepare(PrepareCompletionCapacityV1 {
                    acquisition: id.clone(),
                    expected_acquisition_revision: root.completion_leases.acquisitions[&id]
                        .revision,
                    grant: grant.clone(),
                    expected_grant_revision: root.completion_leases.grants[grant].revision,
                    current_boundary,
                    eligibility_envelope_digest: envelope.digest.clone(),
                }),
            )?;
        }
        let (_, root) = resource_state(&canwu)?.ok_or("the resource root is missing")?;
        let current_boundary = next_boundary(&canwu);
        completion_step(
            &mut canwu,
            ResourceCompletionOperationV1::Activate(ActivateCompletionLeaseV1 {
                acquisition: id.clone(),
                expected_acquisition_revision: root.completion_leases.acquisitions[&id].revision,
                grant: resource_grant.clone(),
                expected_grant_revision: root.completion_leases.grants[&resource_grant].revision,
                at: SimTime::EPOCH,
                current_boundary,
                eligibility_envelope_digest: envelope.digest.clone(),
            }),
        )?;
        let (_, root) = resource_state(&canwu)?.ok_or("the resource root is missing")?;
        batch.completion_certificate = root
            .completion_leases
            .certificate(&id)
            .cloned()
            .ok_or("the batch lease is not active")?;
        let hot_before = root.hot_terminal_records();
        enqueue_resource_adapter_operation(
            &mut canwu,
            SimTime::EPOCH,
            &ResourceAdapterOperationV1 {
                provider_plugin: PROVIDER_PLUGIN.to_owned(),
                provider_source: source,
                request: ResourceOperationRequestV1::ConsumeBatch(batch),
            },
        )?;
        batches.push(measured_boundary(&mut canwu)?);
        let (_, root) = resource_state(&canwu)?.ok_or("the resource root is missing")?;
        let outcome = root
            .outcomes
            .get(&batch_key)
            .ok_or("the batch settled nothing")?;
        if outcome.status != ResourceOperationStatus::Applied || outcome.remainder != 0 {
            return Err(format!("the batch did not consume every item: {outcome:?}").into());
        }
        hot_added = root.hot_terminal_records() - hot_before;
    }
    Ok(json!({
        "items": items,
        "batch": batches.report(options.count_allocations),
        "hot_terminal_records_added": hot_added,
    }))
}

struct Options {
    count_allocations: bool,
    backlogs: Vec<usize>,
    accounts: Vec<usize>,
    households: Vec<usize>,
    pooled: Vec<usize>,
    due: Vec<usize>,
    boundary_batch: Vec<usize>,
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
        accounts: vec![0],
        households: Vec::new(),
        pooled: Vec::new(),
        due: Vec::new(),
        boundary_batch: Vec::new(),
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
            "--accounts" => {
                options.accounts = value("--accounts")?
                    .split(',')
                    .map(str::parse)
                    .collect::<Result<_, _>>()?;
            }
            "--households" => {
                options.households = value("--households")?
                    .split(',')
                    .filter(|value| !value.is_empty())
                    .map(str::parse)
                    .collect::<Result<_, _>>()?;
            }
            "--pooled" => {
                options.pooled = value("--pooled")?
                    .split(',')
                    .filter(|value| !value.is_empty())
                    .map(str::parse)
                    .collect::<Result<_, _>>()?;
            }
            "--due" => {
                options.due = value("--due")?
                    .split(',')
                    .filter(|value| !value.is_empty())
                    .map(str::parse)
                    .collect::<Result<_, _>>()?;
            }
            "--boundary-batch" => {
                options.boundary_batch = value("--boundary-batch")?
                    .split(',')
                    .filter(|value| !value.is_empty())
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
                     --accounts 0,16384 --households 500\n\
                     --pooled 65536 --due 2048,8192 --boundary-batch 256\n\
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
    let mut cases = Vec::with_capacity(options.backlogs.len() * options.accounts.len());
    for accounts in &options.accounts {
        for backlog in &options.backlogs {
            eprintln!("case: {backlog} hot terminal records, {accounts} live accounts");
            cases.push(case(&template, *backlog, *accounts, &options)?);
        }
    }
    let mut lease_cases = Vec::with_capacity(options.households.len());
    for households in &options.households {
        eprintln!("lease case: {households} households");
        lease_cases.push(json!({
            "households": households,
            "per_household": per_household_case(*households)?,
            "batch": batch_case(*households)?,
        }));
    }
    let mut scale_cases = json!({ "pooled": [], "due": [], "boundary_batch": [] });
    for accounts in &options.pooled {
        eprintln!("scale case: pooled pass beside {accounts} accounts");
        scale_cases["pooled"]
            .as_array_mut()
            .expect("array")
            .push(pooled_case(*accounts, &options)?);
    }
    for demands in &options.due {
        eprintln!("scale case: allocation pass over {demands} due demands");
        scale_cases["due"]
            .as_array_mut()
            .expect("array")
            .push(due_case(*demands, &options)?);
    }
    for items in &options.boundary_batch {
        eprintln!("scale case: boundary batch of {items} items");
        scale_cases["boundary_batch"]
            .as_array_mut()
            .expect("array")
            .push(boundary_batch_case(*items, &options)?);
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
        "lease_cases": lease_cases,
        "scale_cases": scale_cases,
    });
    let text = serde_json::to_string_pretty(&report)?;
    match options.output {
        Some(path) => fs::write(path, text)?,
        None => println!("{text}"),
    }
    Ok(())
}
