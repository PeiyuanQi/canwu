//! What a save costs as a campaign grows: the full portable save against the
//! continuation checkpoint.
//!
//! The workload settles one Daily boundary per simulated day. Each day the
//! host dispatches `--commands` plugin commands, and each command queues a
//! packet due 1 to 90 days later and a follow-up due within 9 days. A boundary admits
//! the due packets: each moves a ledger record to its next version and makes
//! `--keyed` operation-keyed draws, and some schedule a follow-up packet up to
//! 30 days ahead. Every boundary publishes a standing record that supersedes
//! the last one, and a holder record keeps a payload-required continuation on
//! an admitted event, rotated monthly. A churn system moves `--churn` records
//! to their next version every day, so the full history grows at about the
//! rate of a long campaign while current state does not.
//!
//! At each sampled month the report records, on the unsealed run:
//!
//! - `full_save`: the bytes of `checkpoint_journal_json`, its write time, and
//!   the time of `Canwu::from_checkpoint_journal_json_with_plugins`;
//! - `continuation`: the bytes of the continuation checkpoint's JSON, its
//!   state bytes and its other bytes by part, the time to build and encode
//!   it, and the time of
//!   `CompactedCanwu::from_continuation_checkpoint_json_verified`, with that
//!   restore's allocations in the allocation build.
//!
//! The allocation build also reports, at the last month, the heap a compact
//! run holds when it seals every month against the unsealed run that never
//! seals, each built alone under the allocation counter.
//!
//! Elapsed time and allocation traffic are collected in separate release
//! builds: `--mode elapsed` without the `allocation-counting` feature,
//! `--mode allocations` with it.

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundaryRequest,
    BoundarySystemContract, Canwu, CanwuError, Command, CommandContext, CommandEnvelope,
    CommandRequest, CommandRequestId, CompactedCanwu, ContinuationCheckpoint, DomainRecordClass,
    DomainRecordDraft, DomainRecordKind, DomainRecordMutation, DomainRecordRef, DomainRecordSchema,
    ENGINE_VERSION, EntityRef, EvidenceRef, IDENTITY_EVIDENCE_DEPENDENCIES_FIELD,
    IdentityEvidenceDependenciesV1, IngressClass, IngressPayload, Issuer, KnowledgeHolderRef,
    KnowledgeOrigin, KnowledgeRecordDraft, KnowledgeRecordKind, KnowledgeSchemaId,
    KnowledgeSubject, KnowledgeSubjectSchema, KnowledgeSubjectTarget, KnowledgeSubjectTargetKind,
    KnowledgeWriteGrant, OrganizationId, PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD,
    PayloadRequiredEvidenceContinuationV1, PayloadSchema, PersonId, PluginActionDescriptor,
    PluginIngressDescriptor, PluginKnowledgeSchema, PluginRegistrar, RandomOperationTarget,
    RandomStreamKey, Scenario, SimDuration, SimTime, SimulationPlugin, SimulationView, StateKey,
    StateVisibility, SystemCadence, SystemDirective, identity_evidence_dependencies_property_v1,
    payload_required_evidence_continuation_property_v1,
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
use std::path::PathBuf;
use std::process::Command as ProcessCommand;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

const PLUGIN: &str = "profile-continuation";
const PLUGIN_HASH: &str = "00000000000000000000000000000000000000000000000000000000000c0a71";
const DAYS_PER_MONTH: i64 = 30;
/// Keyed draws each admitted packet makes; set once from `--keyed`.
static KEYED_DRAWS: AtomicUsize = AtomicUsize::new(0);
/// Records the churn system moves every day; set once from `--churn`.
static CHURN_RECORDS: AtomicUsize = AtomicUsize::new(0);

#[cfg(feature = "allocation-counting")]
struct CountingAllocator;

#[cfg(feature = "allocation-counting")]
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[cfg(feature = "allocation-counting")]
thread_local! {
    static ALLOCATION_STATE: Cell<(bool, i128, u64)> = const { Cell::new((false, 0, 0)) };
}

#[cfg(feature = "allocation-counting")]
fn record(delta: i128, operations: u64) {
    let _ = ALLOCATION_STATE.try_with(|state| {
        let (tracking, net, count) = state.get();
        if tracking {
            state.set((tracking, net + delta, count + operations));
        }
    });
}

#[cfg(feature = "allocation-counting")]
fn bytes(size: usize) -> i128 {
    i128::try_from(size).unwrap_or(i128::MAX)
}

// SAFETY: every operation delegates to `System` with the original pointer and
// layout. The thread-local counters only observe allocation traffic on the
// benchmark thread and never alter allocator inputs or ownership.
#[cfg(feature = "allocation-counting")]
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record(bytes(layout.size()), 1);
        // SAFETY: the caller supplies the layout required by `GlobalAlloc`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record(bytes(layout.size()), 1);
        // SAFETY: the caller supplies the layout required by `GlobalAlloc`.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        record(-bytes(layout.size()), 0);
        // SAFETY: the caller returns the pointer with its original layout.
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record(bytes(new_size) - bytes(layout.size()), 1);
        // SAFETY: the caller supplies the pointer, original layout, and desired
        // size required by `GlobalAlloc::realloc`.
        unsafe { System.realloc(pointer, layout, new_size) }
    }
}

/// Runs `operation` under the allocation counter and returns its output,
/// the net bytes it left allocated, and its allocation operations.
#[cfg(feature = "allocation-counting")]
fn counted<O>(operation: impl FnOnce() -> O) -> (O, i128, u64) {
    ALLOCATION_STATE.with(|state| state.set((true, 0, 0)));
    let output = operation();
    let (_, net, operations) = ALLOCATION_STATE.with(|state| {
        let current = state.get();
        state.set((false, 0, 0));
        current
    });
    (output, net, operations)
}

#[cfg(not(feature = "allocation-counting"))]
fn counted<O>(operation: impl FnOnce() -> O) -> (O, i128, u64) {
    (operation(), 0, 0)
}

fn ledger_kind() -> DomainRecordKind {
    DomainRecordKind::new(PLUGIN, "ledger")
}

fn holder_kind() -> DomainRecordKind {
    DomainRecordKind::new(PLUGIN, "holder")
}

fn item_kind() -> DomainRecordKind {
    DomainRecordKind::new(PLUGIN, "item")
}

fn record_ref(kind: DomainRecordKind, id: &str) -> DomainRecordRef {
    DomainRecordRef {
        kind,
        id: id.to_owned(),
    }
}

fn record_state(kind: DomainRecordKind) -> StateKey {
    DomainRecordSchema::new(kind, DomainRecordClass::Record).state_key()
}

fn stream() -> RandomStreamKey {
    RandomStreamKey::new(PLUGIN, "draws", 1)
}

fn knowledge_schema() -> KnowledgeSchemaId {
    KnowledgeSchemaId::new(
        KnowledgeRecordKind::new("profile.continuation", "standing"),
        1,
    )
}

const fn knowledge_holder() -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(PersonId::new(1))
}

fn about() -> KnowledgeSubject {
    KnowledgeSubject {
        role: "about".to_owned(),
        target: KnowledgeSubjectTarget::Entity(EntityRef::Organization(OrganizationId::new(10))),
    }
}

fn mutation(
    current: Option<&canwu_api::DomainRecord>,
    draft: DomainRecordDraft,
) -> DomainRecordMutation {
    match current {
        None => DomainRecordMutation::Create { record: draft },
        Some(record) => DomainRecordMutation::Update {
            record: draft,
            expected_version: record.version,
        },
    }
}

fn dispatch(
    _view: &SimulationView<'_>,
    _context: &CommandContext,
    payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    let n = payload["n"].as_i64().unwrap_or_default();
    Ok(vec![
        SystemDirective::EnqueuePluginIngress {
            after: SimDuration::days(1 + (n * 37).rem_euclid(90)),
            packet_type: "work".to_owned(),
            priority: 0,
            payload: json!({ "n": n }),
            affected: Vec::new(),
        },
        // A handler cannot schedule a directive that enqueues plugin
        // ingress; the follow-up is queued directly with its own delay.
        SystemDirective::EnqueuePluginIngress {
            after: SimDuration::days(2 + n.rem_euclid(7)),
            packet_type: "work".to_owned(),
            priority: 0,
            payload: json!({ "n": n + 1_000_000 }),
            affected: Vec::new(),
        },
    ])
}

/// Admits the due packets: moves the ledger, draws, and schedules
/// follow-ups.
fn admit(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut directives = Vec::new();
    let mut admitted = 0_i64;
    for id in &context.admitted_ingress {
        let Some(record) = view.ingress(*id)? else {
            continue;
        };
        let IngressPayload::Plugin { payload, .. } = &record.payload else {
            continue;
        };
        let n = payload["n"].as_i64().unwrap_or_default();
        admitted += 1;
        for slot in 0..KEYED_DRAWS.load(Ordering::Relaxed) {
            let operation = format!("packet-{}-{slot}", id.get());
            view.random_draw_for_operation(
                &stream(),
                EvidenceRef::Ingress(*id),
                "work",
                &operation,
                RandomOperationTarget::CanonicalKey(operation.clone()),
                0,
                1_000,
                "continuation profile draw",
            )?;
        }
        if n % 5 == 0 && n < 1_000_000 {
            directives.push(BoundaryDirective::ScheduleIngress {
                after: SimDuration::days(1 + n.rem_euclid(30)),
                packet_type: "work".to_owned(),
                priority: 0,
                payload: json!({ "n": n + 2_000_000 }),
                affected: Vec::new(),
            });
        }
    }
    let ledger = record_ref(ledger_kind(), "ledger");
    let current = view.domain_record(&ledger)?;
    let total = current
        .as_ref()
        .and_then(|record| record.payload["total"].as_i64())
        .unwrap_or_default()
        + admitted;
    directives.push(BoundaryDirective::MutateRecord {
        mutation: mutation(
            current,
            DomainRecordDraft::new(ledger, json!({ "total": total })),
        ),
        summary: "The ledger moves on".to_owned(),
    });

    let holder = record_ref(holder_kind(), "holder");
    let previous = view.domain_record(&holder)?;
    let payload = previous
        .as_ref()
        .map_or(Value::Null, |record| record.payload.clone());
    let continuation: PayloadRequiredEvidenceContinuationV1 =
        serde_json::from_value(payload[PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD].clone())
            .unwrap_or_else(|_| PayloadRequiredEvidenceContinuationV1::completed());
    for dependency in &continuation.dependencies {
        if let EvidenceRef::Event(event) = dependency {
            black_box(view.event(*event)?);
        }
    }
    let since = payload["since"].as_u64().unwrap_or_default();
    let boundary = context.boundary_id.get();
    let (continuation, since) = if continuation.active && boundary < since + 30 {
        (continuation, since)
    } else {
        context.admitted_events.first().map_or_else(
            || (PayloadRequiredEvidenceContinuationV1::completed(), boundary),
            |event| {
                (
                    PayloadRequiredEvidenceContinuationV1::active(vec![EvidenceRef::Event(*event)]),
                    boundary,
                )
            },
        )
    };
    directives.push(BoundaryDirective::MutateRecord {
        mutation: mutation(
            previous,
            DomainRecordDraft::new(
                holder,
                json!({
                    PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD: continuation,
                    IDENTITY_EVIDENCE_DEPENDENCIES_FIELD: IdentityEvidenceDependenciesV1::new(Vec::new()),
                    "since": since,
                }),
            ),
        ),
        summary: "The holder keeps its continuation".to_owned(),
    });
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

/// Moves every churned record to its next version: the bulk of the
/// evidence a long campaign settles, which its current state does not grow
/// with.
fn churn(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut directives = Vec::new();
    for index in 0..CHURN_RECORDS.load(Ordering::Relaxed) {
        let item = record_ref(item_kind(), &format!("item-{index:05}"));
        let current = view.domain_record(&item)?;
        directives.push(BoundaryDirective::MutateRecord {
            mutation: mutation(
                current,
                DomainRecordDraft::new(
                    item,
                    json!({
                        "boundary": context.boundary_id.get(),
                        "stock": (context.boundary_id.get() * 31 + index as u64) % 10_007,
                        "note": "a settled change that only the full history keeps",
                    }),
                ),
            ),
            summary: "An item moves on".to_owned(),
        });
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

/// Publishes a standing record that supersedes the holder's current ones.
fn know(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let heads = view
        .knowledge_heads(&knowledge_holder(), &knowledge_schema().kind, &[about()])?
        .records;
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::PublishKnowledge {
            holder: knowledge_holder(),
            visibility: StateVisibility::SameBoundary,
            producer_correlation: None,
            records: vec![KnowledgeRecordDraft {
                schema: knowledge_schema(),
                subjects: vec![about()],
                payload: json!({ "boundary": context.boundary_id.get() }),
                as_of: Some(context.at),
                confidence_per_mille: 1_000,
                origin: KnowledgeOrigin {
                    method: "profile.continuation".to_owned(),
                    evidence: Vec::new(),
                },
                supersedes: heads,
                contradicts: Vec::new(),
            }],
            summary: "The standing record".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

struct ContinuationProfilePlugin;

impl SimulationPlugin for ContinuationProfilePlugin {
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
            ledger_kind(),
            DomainRecordClass::Record,
        ))?;
        let mut holder_schema = DomainRecordSchema::new(holder_kind(), DomainRecordClass::Record);
        holder_schema.payload_schema = PayloadSchema::Object {
            properties: std::collections::BTreeMap::from([
                (
                    PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD.to_owned(),
                    payload_required_evidence_continuation_property_v1(),
                ),
                (
                    IDENTITY_EVIDENCE_DEPENDENCIES_FIELD.to_owned(),
                    identity_evidence_dependencies_property_v1(),
                ),
            ]),
            allow_additional: true,
        };
        registrar.register_record_schema(holder_schema)?;
        registrar.register_record_schema(DomainRecordSchema::new(
            item_kind(),
            DomainRecordClass::Record,
        ))?;
        registrar.register_knowledge_schema(PluginKnowledgeSchema {
            id: knowledge_schema(),
            schema_hash: "00000000000000000000000000000000000000000000000000000000000c0a72"
                .to_owned(),
            writable: true,
            payload_schema: PayloadSchema::Any,
            subjects: vec![KnowledgeSubjectSchema {
                role: "about".to_owned(),
                targets: vec![KnowledgeSubjectTargetKind::AnyEntity],
                required: true,
                multiple: false,
            }],
        })?;
        registrar.register_ingress(PluginIngressDescriptor {
            name: "work".to_owned(),
            description: "profile work packet".to_owned(),
            class: IngressClass::Information,
            payload_schema: PayloadSchema::Any,
        })?;
        registrar.register_command(
            PluginActionDescriptor {
                name: "dispatch".to_owned(),
                description: "Queue work and schedule a follow-up".to_owned(),
                payload_schema: PayloadSchema::Any,
                reads: Vec::new(),
                writes: Vec::new(),
            },
            dispatch,
        )?;
        let mut admitting = BoundarySystemContract::new(
            "admit",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        admitting.reads = vec![
            StateKey::core_ingress(),
            StateKey::core_events(),
            record_state(ledger_kind()),
            record_state(holder_kind()),
        ];
        admitting.writes = vec![record_state(ledger_kind()), record_state(holder_kind())];
        admitting.random_streams = vec![stream()];
        registrar.register_boundary_system(admitting, admit)?;
        let mut churning = BoundarySystemContract::new(
            "churn",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        churning.reads = vec![record_state(item_kind())];
        churning.writes = vec![record_state(item_kind())];
        registrar.register_boundary_system(churning, churn)?;
        let mut knowing = BoundarySystemContract::new(
            "know",
            BoundaryPhase::PerspectiveAndReportMaterialization,
            SystemCadence::Daily,
        );
        knowing.reads = vec![StateKey::core_knowledge()];
        knowing.knowledge_writes = vec![KnowledgeWriteGrant {
            schema: knowledge_schema(),
            visibilities: vec![StateVisibility::SameBoundary],
        }];
        registrar.register_boundary_system(knowing, know)
    }
}

fn scenario() -> Scenario {
    Scenario::new(
        SimTime::EPOCH,
        vec![
            EntityRef::Person(PersonId::new(1)),
            EntityRef::Organization(OrganizationId::new(10)),
        ],
    )
}

fn plugins() -> [&'static dyn SimulationPlugin; 1] {
    [&ContinuationProfilePlugin]
}

/// The host input and boundary of one day, on either handle.
trait Daily {
    fn revision(&self) -> u64;
    fn queue(&mut self, at: SimTime, request: CommandRequest) -> Result<(), CanwuError>;
    fn settle(&mut self, at: SimTime) -> Result<(), CanwuError>;
}

impl Daily for Canwu {
    fn revision(&self) -> u64 {
        Self::revision(self)
    }
    fn queue(&mut self, at: SimTime, request: CommandRequest) -> Result<(), CanwuError> {
        self.enqueue_command(at, 0, request).map(|_| ())
    }
    fn settle(&mut self, at: SimTime) -> Result<(), CanwuError> {
        self.settle_boundary(BoundaryRequest::at(at).with_cadence(SystemCadence::Daily))
            .map(|_| ())
    }
}

impl Daily for CompactedCanwu {
    fn revision(&self) -> u64 {
        Self::revision(self)
    }
    fn queue(&mut self, at: SimTime, request: CommandRequest) -> Result<(), CanwuError> {
        self.enqueue_command(at, 0, request).map(|_| ())
    }
    fn settle(&mut self, at: SimTime) -> Result<(), CanwuError> {
        self.settle_boundary(BoundaryRequest::at(at).with_cadence(SystemCadence::Daily))
            .map(|_| ())
    }
}

fn day(day: i64) -> SimTime {
    SimTime::EPOCH + SimDuration::days(day)
}

/// Runs one day: the host dispatches its commands, and the boundary settles.
fn run_day(run: &mut impl Daily, d: i64, commands: u64) -> Result<(), CanwuError> {
    let at = day(d);
    for index in 0..commands {
        let id = u64::try_from(d).unwrap_or_default() * 1_000 + index + 1;
        let request = CommandRequest::new(
            CommandRequestId::new(id),
            run.revision(),
            CommandEnvelope::new(
                Issuer::Debug,
                Command::Plugin {
                    plugin: PLUGIN.to_owned(),
                    command: "dispatch".to_owned(),
                    payload: json!({ "n": id }),
                },
            ),
        );
        run.queue(at, request)?;
    }
    run.settle(at)
}

/// The JSON length of a value.
fn encoded_len<T: serde::Serialize>(value: &T) -> Result<usize, serde_json::Error> {
    serde_json::to_vec(value).map(|bytes| bytes.len())
}

/// The JSON size of each part of a continuation checkpoint.
fn part_bytes(checkpoint: &ContinuationCheckpoint) -> Result<Value, Box<dyn Error>> {
    Ok(json!({
        "state": encoded_len(&checkpoint.state)?,
        "pinned": encoded_len(&checkpoint.pinned)?,
        "pinned_ingress": encoded_len(&checkpoint.pinned.ingress)?,
        "pending_ingress": encoded_len(&checkpoint.pending_ingress)?,
        "unadmitted": encoded_len(&checkpoint.unadmitted)?,
        "ingress_provenance": encoded_len(&checkpoint.ingress_provenance)?,
        "known_record_versions": encoded_len(&checkpoint.known_record_versions)?,
        "current_record_versions": encoded_len(&checkpoint.current_record_versions)?,
        "keyed_draw_ledger": encoded_len(&checkpoint.keyed_draws)?,
        "request_index": encoded_len(&checkpoint.requests)?,
        "time_index": encoded_len(&checkpoint.evidence_times)?,
        "boundary_ledger": encoded_len(&checkpoint.settled_boundaries)?,
        "journal_frontiers": encoded_len(&checkpoint.journal_frontiers)?,
    }))
}

struct Options {
    mode: &'static str,
    months: Vec<i64>,
    keyed: Vec<usize>,
    commands: u64,
    churn: usize,
    samples: usize,
    output: Option<PathBuf>,
    machine: String,
    recorded_on: String,
}

fn options() -> Result<Options, Box<dyn Error>> {
    let mut options = Options {
        mode: "elapsed",
        months: vec![1, 2, 4, 7, 10, 14],
        keyed: vec![0, 10],
        commands: 8,
        churn: 140,
        samples: 3,
        output: None,
        machine: "local-machine".to_owned(),
        recorded_on: "unspecified".to_owned(),
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
                    "elapsed" => "elapsed",
                    "allocations" => "allocations",
                    other => return Err(format!("unsupported metric mode: {other}").into()),
                };
            }
            "--months" => {
                options.months = value()?
                    .split(',')
                    .map(str::parse)
                    .collect::<Result<_, _>>()?;
            }
            "--keyed" => {
                options.keyed = value()?
                    .split(',')
                    .map(str::parse)
                    .collect::<Result<_, _>>()?;
            }
            "--commands" => options.commands = value()?.parse()?,
            "--churn" => options.churn = value()?.parse()?,
            "--samples" => options.samples = value()?.parse()?,
            "--output" => options.output = Some(PathBuf::from(value()?)),
            "--machine" => options.machine = value()?,
            "--recorded-on" => options.recorded_on = value()?,
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }
    if options.months.windows(2).any(|pair| pair[0] >= pair[1]) || options.samples == 0 {
        return Err("--months must increase and --samples must be positive".into());
    }
    Ok(options)
}

fn median(mut values: Vec<u64>) -> u64 {
    values.sort_unstable();
    values.get(values.len() / 2).copied().unwrap_or_default()
}

fn nanos(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

/// Samples one month on the unsealed run: the full save and the
/// continuation checkpoint, written and restored.
fn sample_month(canwu: &Canwu, options: &Options) -> Result<Value, Box<dyn Error>> {
    let plugins = plugins();
    let mut full_write = Vec::new();
    let mut full_restore = Vec::new();
    let mut full_bytes = 0;
    let mut cont_write = Vec::new();
    let mut cont_restore = Vec::new();
    let mut cont_bytes = 0;
    let mut restore_allocations = (0_i128, 0_u64);
    let mut parts = Value::Null;
    for _ in 0..options.samples {
        let started = Instant::now();
        let json = canwu.checkpoint_journal_json()?;
        full_write.push(nanos(started));
        full_bytes = json.len();
        let started = Instant::now();
        let restored = Canwu::from_checkpoint_journal_json_with_plugins(&json, &plugins)?;
        full_restore.push(nanos(started));
        drop(black_box(restored));

        let started = Instant::now();
        let (checkpoint, hash) = canwu.continuation_checkpoint()?;
        let json = checkpoint.to_json()?;
        cont_write.push(nanos(started));
        cont_bytes = json.len();
        parts = part_bytes(&checkpoint)?;
        let started = Instant::now();
        let (restored, net, operations) = counted(|| {
            CompactedCanwu::from_continuation_checkpoint_json_verified(&json, &hash, &plugins)
        });
        cont_restore.push(nanos(started));
        let restored = restored?;
        if restored.checkpoint_hash() != canwu.checkpoint_hash() {
            return Err("the restored run differs from the run it was written from".into());
        }
        restore_allocations = (net, operations);
        drop(black_box(restored));
    }
    let state_bytes = parts["state"].as_u64().unwrap_or_default();
    Ok(json!({
        "full_save": {
            "bytes": full_bytes,
            "write_ns_median": median(full_write),
            "restore_ns_median": median(full_restore),
        },
        "continuation": {
            "bytes": cont_bytes,
            "non_state_bytes": u64::try_from(cont_bytes).unwrap_or_default().saturating_sub(state_bytes),
            "part_bytes": parts,
            "write_ns_median": median(cont_write),
            "restore_ns_median": median(cont_restore),
            "restore_net_bytes": restore_allocations.0,
            "restore_allocation_operations": restore_allocations.1,
        },
    }))
}

/// Builds a run to `days` alone under the allocation counter and returns the
/// bytes it holds, sealing every month when `seal_monthly`.
fn run_heap(days: i64, options: &Options, seal_monthly: bool) -> Result<i128, Box<dyn Error>> {
    let plugins = plugins();
    let (result, net, _) = counted(|| -> Result<Box<dyn std::any::Any>, Box<dyn Error>> {
        let canwu = Canwu::new_with_plugins(1_701, scenario(), &plugins)?;
        if seal_monthly {
            let mut compact = canwu.into_compacted()?;
            for d in 1..=days {
                run_day(&mut compact, d, options.commands)?;
                if d % DAYS_PER_MONTH == 0 {
                    drop(compact.seal_evidence()?);
                }
            }
            Ok(Box::new(compact))
        } else {
            let mut canwu = canwu;
            for d in 1..=days {
                run_day(&mut canwu, d, options.commands)?;
            }
            Ok(Box::new(canwu))
        }
    });
    drop(result?);
    Ok(net)
}

fn main() -> Result<(), Box<dyn Error>> {
    let options = options()?;
    if cfg!(debug_assertions) {
        return Err("run the continuation profile with --release".into());
    }
    if (options.mode == "allocations") != cfg!(feature = "allocation-counting") {
        return Err(
            "allocation mode needs --features allocation-counting, and elapsed mode a build without it"
                .into(),
        );
    }
    CHURN_RECORDS.store(options.churn, Ordering::Relaxed);
    let mut series = Vec::new();
    for &keyed in &options.keyed {
        KEYED_DRAWS.store(keyed, Ordering::Relaxed);
        let mut canwu = Canwu::new_with_plugins(1_701, scenario(), &plugins())?;
        let mut samples = Vec::new();
        let mut d = 0;
        for &month in &options.months {
            while d < month * DAYS_PER_MONTH {
                d += 1;
                run_day(&mut canwu, d, options.commands)?;
            }
            let mut sample = sample_month(&canwu, &options)?;
            sample["month"] = json!(month);
            eprintln!(
                "keyed {keyed:>2} month {month:>2}: full save {:>11} B, continuation {:>9} B ({:>8} B non-state), restore {:>10} ns against {:>12} ns",
                sample["full_save"]["bytes"],
                sample["continuation"]["bytes"],
                sample["continuation"]["non_state_bytes"],
                sample["continuation"]["restore_ns_median"],
                sample["full_save"]["restore_ns_median"],
            );
            samples.push(sample);
        }
        let mut entry = json!({ "keyed_draws_per_packet": keyed, "months": samples });
        if options.mode == "allocations" {
            let days = options.months.last().copied().unwrap_or(1) * DAYS_PER_MONTH;
            entry["heap_bytes_unsealed"] = json!(run_heap(days, &options, false)?);
            entry["heap_bytes_sealed_monthly"] = json!(run_heap(days, &options, true)?);
        }
        series.push(entry);
    }
    let report = json!({
        "schema_version": 1,
        "benchmark": "canwu-continuation-checkpoint",
        "metric_mode": options.mode,
        "recorded_on": options.recorded_on,
        "machine": options.machine,
        "engine_version": ENGINE_VERSION,
        "environment": {
            "os": env::consts::OS,
            "arch": env::consts::ARCH,
            "logical_cpus": std::thread::available_parallelism().map_or(1, usize::from),
            "rustc": toolchain_output("rustc", &["--version"]),
            "profile": "release",
            "cargo_features": if cfg!(feature = "allocation-counting") { vec!["allocation-counting"] } else { Vec::<&str>::new() },
        },
        "method": {
            "days_per_month": DAYS_PER_MONTH,
            "host_commands_per_day": options.commands,
            "churned_records_per_day": options.churn,
            "samples": options.samples,
            "full_save": "checkpoint_journal_json of the unsealed run, and Canwu::from_checkpoint_journal_json_with_plugins of it",
            "continuation": "continuation_checkpoint and to_json of the unsealed run, and CompactedCanwu::from_continuation_checkpoint_json_verified of it",
            "part_bytes": "the JSON length of each part of the continuation checkpoint",
            "heap": "allocation build only: net bytes held by a run built alone to the last month, unsealed or sealing every month",
        },
        "series": series,
    });
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
