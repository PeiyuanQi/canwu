//! The resource root persists as its live root and one record per hot
//! terminal bucket, and a boundary writes what it changes.
//!
//! The first fixture settles the same operation against a root without hot
//! terminal records and against one with a backlog of several hundred: the
//! boundary writes the live root and the one bucket the new outcome lands in,
//! and its journal grows by the same few kilobytes whatever the backlog.
//!
//! The second settles operations of several kinds (outcomes, and the closure
//! of a cancelled demand), archives part of the backlog in a commit that
//! splits that closure, settles operations that read the remaining part, and
//! seals in between: a sealed run, an unsealed fork, a run restored from the
//! sealed segments, and an exact replay agree, and the archived records are
//! gone from their buckets.
//!
//! The third tampers with the persisted split: an entry in another bucket, a
//! ledger that does not count the entries, and an entry that is not in its
//! canonical encoding are refused when the root is installed.

use canwu_api::{
    BoundaryRequest, Canwu, CompactedCanwu, DomainRecord, EntityRef, IngressClass,
    KnowledgeHolderRef, PayloadSchema, PersonId, PluginIngressDescriptor, PluginIngressRequest,
    PluginRegistrar, Scenario, SimDuration, SimTime, SimulationPlugin,
};
use canwu_resource::*;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::RwLock;

/// Stored archive objects by namespace and object id.
type ArchiveObjects = BTreeMap<(String, String), Vec<u8>>;

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
    ) -> Result<Option<Vec<u8>>, canwu_api::CanwuError> {
        Ok(self
            .objects
            .read()
            .unwrap()
            .get(&(namespace.to_owned(), object_id.to_owned()))
            .cloned())
    }
}

const CUSTODIAN: u64 = 1;
const TICK_PLUGIN: &str = "fixture-tick";
const TICK_INGRESS: &str = "tick_v1";

/// Owns one ingress type and no system: a tick makes a boundary event-bearing
/// with nothing due for the resource plugin.
struct TickPlugin;

impl SimulationPlugin for TickPlugin {
    fn name(&self) -> &'static str {
        TICK_PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "00000000000000000000000000000000000000000000000000000000000000f3"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), canwu_api::CanwuError> {
        registrar.register_ingress(PluginIngressDescriptor {
            name: TICK_INGRESS.to_owned(),
            description: "Make one boundary event-bearing".to_owned(),
            class: IngressClass::Information,
            payload_schema: PayloadSchema::Any,
        })
    }
}

fn holder() -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(PersonId::new(CUSTODIAN))
}

fn key(value: &str) -> ResourceOperationKey {
    ResourceOperationKey::new(value).expect("operation key")
}

fn account(name: &str) -> ResourceAccountId {
    ResourceAccountId::new(format!("fixture:account:{name}")).expect("account")
}

fn unit() -> ResourceUnitRevisionId {
    ResourceUnitRevisionId::new("fixture:unit:measure:v1").expect("unit")
}

fn resource() -> ResourceDefinitionRevisionId {
    ResourceDefinitionRevisionId::new("fixture:resource:stock:v1").expect("resource")
}

fn demand_id(name: &str) -> ResourceDemandId {
    ResourceDemandId::new(format!("fixture:demand:{name}")).expect("demand")
}

/// A root under `limits` with one custodian and two of its accounts.
fn world(limits: ResourceLimitsV1) -> ResourceState {
    let mut state = ResourceState::empty(limits).expect("state");
    state
        .install_unit(ResourceUnitRevision {
            id: unit(),
            revision: ResourceRevision::INITIAL,
            symbol: "u".to_owned(),
            scale_numerator: 1,
            scale_denominator: 1,
            semantic_digest: "0".repeat(64),
        })
        .expect("unit");
    state
        .install_definition(ResourceDefinitionRevision {
            id: resource(),
            resource: ResourceDefinitionId::new("fixture:resource:stock").expect("definition"),
            revision: ResourceRevision::INITIAL,
            canonical_unit: unit(),
            quality: ResourceQualityId::new("fixture:quality:standard").expect("quality"),
            scope: ResourceScopeId::new("fixture:scope:market").expect("scope"),
            effective_from: SimTime::EPOCH,
            effective_until: None,
            process_suitability: BTreeSet::new(),
            semantic_digest: "1".repeat(64),
        })
        .expect("definition");
    for name in ["left", "right"] {
        state
            .install_opening_account(ResourceAccount {
                id: account(name),
                revision: ResourceRevision::INITIAL,
                custodian: holder(),
                resource_revision: resource(),
                unit_revision: unit(),
                balance: 100,
                capacity: None,
                protected_floor_policy: None,
                closed: false,
                place_scope: None,
            })
            .expect("account");
    }
    state
}

/// A demand of the custodian on its own two accounts.
fn demand(name: &str, requested: u64) -> ResourceDemand {
    ResourceDemand {
        id: demand_id(name),
        revision: ResourceRevision::INITIAL,
        requester: holder(),
        source_policy: ResourceDemandSourcePolicyV1::ExactAccounts(vec![
            account("left"),
            account("right"),
        ]),
        resource_revision: resource(),
        unit_revision: unit(),
        requested,
        fulfilled: 0,
        minimum_useful: 1,
        partial_fulfillment: PartialFulfillmentPolicy::AcceptPartial,
        alternative_group: None,
        due_at: SimTime::EPOCH,
        expires_at: SimTime::EPOCH + SimDuration::days(10),
        priority: 1,
        tie_break: ResourceTieBreakKey::new(format!("fixture:tie:{name}")).expect("tie"),
        admitted_sequence: 0,
        protected_floor_policy: None,
        protection_override_class: None,
        status: DemandStatus::Open,
        rejection_reason: None,
    }
}

/// One durable rejection: cancelling a demand that does not exist leaves a
/// terminal outcome and nothing else.
fn reject(state: &mut ResourceState, label: &str, index: usize) {
    state
        .apply_operation(&ResourceOperationRequestV1::CancelDemand(
            ResourceCancelDemandRequestV1 {
                operation_key: key(&format!("fixture:{label}:{index}")),
                demand: ResourceDemandId::new(format!("fixture:missing:{index}")).expect("demand"),
                expected_demand_revision: ResourceRevision::INITIAL,
            },
        ))
        .expect("durable rejection");
}

/// A cancelled demand whose allocation left a leg and a reservation on each
/// account: its closure (the demand, both legs, both reservations) is hot
/// terminal, a group of five records spread over the buckets.
fn cancelled_demand(state: &mut ResourceState, name: &str) {
    state
        .apply_operation(&ResourceOperationRequestV1::SubmitDemand(
            ResourceSubmitDemandRequestV1 {
                operation_key: key(&format!("fixture:submit:{name}")),
                demand: demand(name, 150),
            },
        ))
        .expect("submit");
    let revision = state.state_revision;
    state
        .apply_operation(&ResourceOperationRequestV1::Allocate(
            ResourceAllocationRequestV1 {
                operation_key: key(&format!("fixture:allocate:{name}")),
                expected_state_revision: revision,
                at: SimTime::EPOCH,
                candidate_limit: 4,
            },
        ))
        .expect("allocate");
    let revision = state.demands[&demand_id(name)].revision;
    state
        .apply_operation(&ResourceOperationRequestV1::CancelDemand(
            ResourceCancelDemandRequestV1 {
                operation_key: key(&format!("fixture:cancel:{name}")),
                demand: demand_id(name),
                expected_demand_revision: revision,
            },
        ))
        .expect("cancel");
}

/// One requester-scoped allocation pass. The requester owns no open demand,
/// so the pass settles one terminal outcome.
fn enqueue_pass(canwu: &mut Canwu, label: &str) {
    let at = canwu.time();
    let (_, root) = resource_state(canwu)
        .expect("resource query")
        .expect("resource root");
    enqueue_resource_allocation(
        canwu,
        at,
        &holder(),
        &ResourceAllocationRequestV1 {
            operation_key: key(label),
            expected_state_revision: root.state_revision,
            at,
            candidate_limit: 1,
        },
    )
    .expect("allocation pass");
}

/// The resource records `canwu` holds, by reference.
fn resource_records(canwu: &Canwu) -> BTreeMap<String, DomainRecord> {
    canwu
        .domain_records()
        .filter(|record| record.owner == PLUGIN_NAME)
        .map(|record| (record.reference.to_string(), record.clone()))
        .collect()
}

/// Settles one boundary and returns the resource records it wrote and the
/// encoded bytes of the journal evidence it appended.
fn measured_boundary(canwu: &mut Canwu) -> (BTreeSet<String>, usize) {
    let before = resource_records(canwu);
    let cursor = canwu.evidence_cursor().expect("cursor");
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("boundary");
    let journal = serde_json::to_vec(&canwu.journal_segment_since(cursor).expect("journal"))
        .expect("journal encodes")
        .len();
    let written = resource_records(canwu)
        .into_iter()
        .filter(|(reference, record)| before.get(reference) != Some(record))
        .map(|(reference, _)| reference)
        .collect();
    (written, journal)
}

#[test]
fn a_boundary_writes_the_live_root_and_the_buckets_it_changes_whatever_the_backlog() {
    const BACKLOG: usize = 600;
    let mut limits = ResourceLimitsV1::canonical();
    limits.max_archive_candidates = 1_024;
    limits.max_archive_maintenance_receipts = 1_024;
    let tick = TickPlugin;
    let plugin = ResourcePlugin::default();
    let plugins: [&dyn SimulationPlugin; 2] = [&tick, &plugin];
    let mut measured = Vec::new();
    for backlog in [0, BACKLOG] {
        let mut state = world(limits);
        for index in 0..backlog {
            reject(&mut state, "backlog", index);
        }
        assert_eq!(state.hot_terminal_records(), backlog);
        let records = state.into_records().expect("the root persists");
        // The live root and one record per bucket that holds an entry: a
        // root of 1,024 candidates has 1,024 buckets.
        assert!(records.len() <= 1 + backlog.min(1_024));
        let scenario = Scenario::new(SimTime::EPOCH, vec![EntityRef::Person(PersonId::new(1))])
            .with_domain_records(records);
        let mut canwu = Canwu::new_with_plugins(7, scenario, &plugins).expect("runtime");

        enqueue_pass(&mut canwu, "fixture:pass:measured");
        let (written, journal) = measured_boundary(&mut canwu);
        let bucket = resource_hot_terminal_bucket(
            &ResourceTerminalRecordKeyV1::Outcome(key("fixture:pass:measured")),
            resource_hot_terminal_bucket_count(&limits),
        );
        assert_eq!(
            written,
            BTreeSet::from([
                resource_runtime_reference().into_untyped().to_string(),
                resource_hot_terminal_bucket_reference(bucket)
                    .into_untyped()
                    .to_string(),
            ]),
            "the boundary writes the live root and the new outcome's bucket"
        );
        let (_, root) = resource_state(&canwu)
            .expect("resource query")
            .expect("resource root");
        assert_eq!(root.hot_terminal_records(), backlog + 1);
        assert!(root.outcomes.contains_key(&key("fixture:pass:measured")));

        // A boundary with nothing due for the resource plugin writes nothing
        // of the root.
        canwu
            .enqueue_plugin_ingress(PluginIngressRequest::new(
                TICK_PLUGIN,
                TICK_INGRESS,
                canwu.time(),
                serde_json::json!({}),
            ))
            .expect("tick");
        let (idle_written, _) = measured_boundary(&mut canwu);
        assert!(idle_written.is_empty());
        measured.push(journal);
    }
    // Without the split, each hot record added about 1.1 KB to the journal of
    // every boundary that wrote the root: some 660 KB here.
    assert!(
        measured[1].abs_diff(measured[0]) < 4_096,
        "journal bytes of one operation boundary: {} without a backlog, {} with {BACKLOG} hot records",
        measured[0],
        measured[1]
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn the_split_root_agrees_across_a_seal_a_fork_a_restore_and_an_exact_replay() {
    let mut limits = ResourceLimitsV1::canonical();
    limits.max_archive_candidates = 64;
    limits.max_archive_maintenance_receipts = 64;
    let mut state = world(limits);
    for index in 0..12 {
        reject(&mut state, "opening", index);
    }
    // Two closures of five records after the rejections. The commit below
    // archives the first fifteen candidates, which splits the first closure:
    // part of it is archived, the demand stays hot.
    cancelled_demand(&mut state, "first");
    cancelled_demand(&mut state, "second");
    let opening_candidates = state.hot_terminal_records();
    let first_revision = state.demands[&demand_id("first")].revision;
    let archive = MemoryArchive::default();
    let plugin = ResourcePlugin::default();
    let plugins: [&dyn SimulationPlugin; 1] = [&plugin];
    let scenario = Scenario::new(SimTime::EPOCH, vec![EntityRef::Person(PersonId::new(1))])
        .with_domain_records(state.into_records().expect("the root persists"));
    let mut base = Canwu::new_with_plugins(9, scenario, &plugins).expect("runtime");
    base.set_plugin_archive_object_provider(Arc::new(archive.clone()));
    seals(&base);
    for index in 0..2 {
        enqueue_pass(&mut base, &format!("fixture:pass:before:{index}"));
        base.settle_boundary(BoundaryRequest::at(base.time()))
            .expect("boundary");
        seals(&base);
    }
    let (_, root) = resource_state(&base)
        .expect("resource query")
        .expect("root");
    let prepared = root
        .prepare_resource_archive(15, &archive)
        .expect("archive batch");
    let archived = prepared.selected.clone();
    let receipt = enqueue_resource_archive(&mut base, &prepared, &archive).expect("archive commit");
    base.settle_boundary(BoundaryRequest::at(base.time()))
        .expect("the commit applies");
    seals(&base);
    finalize_resource_archive_retention(&mut base, &archive, &receipt).expect("finalize");
    enqueue_pass(&mut base, "fixture:pass:after-commit");
    // The second boundary admits the events of the first, so the history
    // seals.
    for _ in 0..2 {
        base.settle_boundary(BoundaryRequest::at(base.time()))
            .expect("boundary");
        seals(&base);
    }

    // Seal, and keep an unsealed fork and a run restored from the sealed
    // segments.
    let mut unsealed = base.fork().into_compacted().expect("compact interface");
    let mut sealed = base.into_compacted().expect("compact interface");
    let segment = sealed
        .seal_evidence()
        .expect("the settled history seals")
        .expect("the settled history holds evidence");
    let restored = CompactedCanwu::from_checkpoint_and_journal_with_plugins(
        sealed.checkpoint().expect("sealed checkpoint"),
        vec![segment.clone()],
        &plugins,
    )
    .expect("the sealed segments restore the run");
    assert_eq!(sealed.checkpoint_hash(), unsealed.checkpoint_hash());
    assert_eq!(restored.checkpoint_hash(), unsealed.checkpoint_hash());

    // In the runs that read the archive: a cancellation of the first
    // demand, which reads it and what remains of its closure from their
    // buckets, and an account opened by a tracked command. (A restored
    // compacted run has no archive provider to look operation keys up in.)
    for run in [&mut unsealed, &mut sealed] {
        let at = run.time();
        let revision = run.revision();
        run.enqueue_command(at, 0, cancel_again(1, revision, at, first_revision))
            .expect("tracked command");
        run.enqueue_command(at, 0, open_account(2, revision + 1, at))
            .expect("tracked command");
        for _ in 0..2 {
            run.settle_boundary(BoundaryRequest::at(at))
                .expect("boundary");
        }
    }
    assert_eq!(sealed.checkpoint_hash(), unsealed.checkpoint_hash());

    // Exact replay of the sealed run regenerates every boundary and equals
    // the unsealed fork.
    let journal = sealed
        .replay_journal_with_segments(vec![segment.clone()])
        .expect("the sealed run produces an exact replay journal");
    let replayed =
        Canwu::replay_from_journal_with_archive_provider(&plugins, &journal, Arc::new(archive))
            .expect("exact replay");
    let reconstructed = sealed
        .snapshot_with_segments(vec![segment])
        .expect("the sealed run reconstructs a full snapshot");
    assert_eq!(replayed.snapshot(), reconstructed);
    assert_eq!(
        unsealed
            .snapshot_with_segments(Vec::new())
            .expect("the unsealed fork's snapshot"),
        reconstructed
    );

    // The archived records left their buckets; the rest of the first
    // closure stayed hot, and the cancellation found the demand closed.
    let root = resource_state(&replayed)
        .expect("resource query")
        .expect("root")
        .1;
    assert_eq!(archived.len(), 15);
    assert!(archived.iter().any(|key| matches!(
        key,
        ResourceTerminalRecordKeyV1::Reservation(_) | ResourceTerminalRecordKeyV1::AllocationLeg(_)
    )));
    assert!(
        !archived.contains(&ResourceTerminalRecordKeyV1::Demand(demand_id("first"))),
        "the commit leaves the first demand hot"
    );
    // The opening candidates, less the fifteen archived, plus the commit's
    // receipt and the outcomes of five operations: two passes before the
    // commit, one after it, the cancellation, and the new account.
    assert_eq!(root.hot_terminal_records(), opening_candidates - 15 + 1 + 5);
    let buckets = resource_hot_terminal_bucket_count(&limits);
    for key in &archived {
        let bucket = replayed
            .domain_record(
                resource_hot_terminal_bucket_reference(resource_hot_terminal_bucket(key, buckets))
                    .as_untyped(),
            )
            .expect("the bucket of an archived record stays, emptied of it");
        assert!(
            !bucket.payload["entries"]
                .as_array()
                .expect("entries")
                .iter()
                .any(|entry| entry["key"] == serde_json::to_value(key).expect("key")),
            "{key:?} left its bucket"
        );
    }
    let cancelled_again = &root.outcomes[&key("fixture:cancel:first:again")];
    assert_eq!(cancelled_again.status, ResourceOperationStatus::Rejected);
    assert_eq!(
        cancelled_again.rejection_code.as_deref(),
        Some("invalid_lifecycle"),
        "the hot terminal demand was read from its bucket, not missed"
    );
    assert!(root.accounts.contains_key(&account("opened")));
}

/// Seals a compacted fork of `canwu`. Neither the live root nor a bucket
/// declares a record version the seal cannot carry, so a run seals before
/// its first boundary and after any later one. Sealing reads those
/// declarations first; a run whose last boundary emitted events that no
/// later boundary has admitted yet is refused after that, for timing alone.
fn seals(canwu: &Canwu) {
    match canwu
        .fork()
        .into_compacted()
        .expect("compact interface")
        .seal_evidence()
    {
        Ok(_) => {}
        Err(error)
            if error.code == canwu_api::ErrorCode::ArchiveNotReady
                && error.message.contains(
                    "every retained event to be admitted by a later completed boundary",
                ) => {}
        Err(error) => panic!("the run seals at this boundary: {error:?}"),
    }
}

/// A tracked command that cancels the first demand again: the demand is
/// closed and hot terminal, so its settlement reads it from its bucket and
/// records the lifecycle refusal.
fn cancel_again(
    request: u64,
    revision: u64,
    at: SimTime,
    demand_revision: ResourceRevision,
) -> canwu_api::CommandRequest {
    tracked(
        request,
        revision,
        at,
        ResourceOperationRequestV1::CancelDemand(ResourceCancelDemandRequestV1 {
            operation_key: key("fixture:cancel:first:again"),
            demand: demand_id("first"),
            expected_demand_revision: demand_revision,
        }),
    )
}

/// A tracked command that opens an empty account.
fn open_account(request: u64, revision: u64, at: SimTime) -> canwu_api::CommandRequest {
    tracked(
        request,
        revision,
        at,
        ResourceOperationRequestV1::CreateAccount(ResourceCreateAccountRequestV1 {
            operation_key: key("fixture:open:opened"),
            account: ResourceAccount {
                id: account("opened"),
                revision: ResourceRevision::INITIAL,
                custodian: holder(),
                resource_revision: resource(),
                unit_revision: unit(),
                balance: 0,
                capacity: None,
                protected_floor_policy: None,
                closed: false,
                place_scope: None,
            },
        }),
    )
}

fn tracked(
    request: u64,
    revision: u64,
    at: SimTime,
    operation: ResourceOperationRequestV1,
) -> canwu_api::CommandRequest {
    canwu_api::CommandRequest::new(
        canwu_api::CommandRequestId::new(request),
        revision,
        canwu_api::CommandEnvelope::new(
            canwu_api::Issuer::Actor(PersonId::new(CUSTODIAN)),
            resource_command(&ResourceCommandV1 {
                subject: holder(),
                request: operation,
            })
            .expect("command"),
        )
        .at_time(at),
    )
}

#[test]
fn a_tampered_hot_terminal_bucket_is_refused_when_the_root_is_installed() {
    let mut state = world(ResourceLimitsV1::canonical());
    for index in 0..8 {
        reject(&mut state, "tamper", index);
    }
    let records = state.into_records().expect("the root persists");
    let plugin = ResourcePlugin::default();
    let install = |records: Vec<DomainRecord>| {
        let scenario = Scenario::new(SimTime::EPOCH, Vec::new()).with_domain_records(records);
        Canwu::new_with_plugins(3, scenario, &[&plugin]).map(|_| ())
    };
    install(records.clone()).expect("the untouched root installs");
    let bucket_at = records
        .iter()
        .position(|record| record.payload.get("entries").is_some())
        .expect("a bucket");

    // An entry moved to a bucket its identity does not hash to.
    let mut moved = records.clone();
    let entry = moved[bucket_at].payload["entries"][0].clone();
    let other = (moved[bucket_at].payload["bucket"].as_u64().expect("bucket") + 1)
        % u64::from(resource_hot_terminal_bucket_count(
            &ResourceLimitsV1::canonical(),
        ));
    let mut stray = moved[bucket_at].clone();
    stray.reference =
        resource_hot_terminal_bucket_reference(u32::try_from(other).expect("bucket index"))
            .into_untyped();
    stray.payload = serde_json::json!({
        "format_version": RESOURCE_HOT_TERMINAL_BUCKET_FORMAT_VERSION,
        "bucket": other,
        "entries": [entry],
    });
    moved[bucket_at].payload["entries"]
        .as_array_mut()
        .expect("entries")
        .remove(0);
    moved.retain(|record| record.reference != stray.reference);
    moved.push(stray);

    // A ledger that does not count the stored entries.
    let mut miscounted = records.clone();
    miscounted[0].payload["hot_terminal"]["records"] = serde_json::json!(7);

    // An entry with a field its type does not have.
    let mut padded = records.clone();
    padded[bucket_at].payload["entries"][0]["padding"] = serde_json::json!(0);

    for forged in [moved, miscounted, padded] {
        let error = install(forged).expect_err("a tampered split root is refused");
        assert_eq!(error.code, canwu_api::ErrorCode::InvalidDomainRecord);
    }
}
