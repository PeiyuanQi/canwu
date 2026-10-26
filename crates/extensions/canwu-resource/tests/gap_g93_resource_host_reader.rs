//! Downstream gap G-93: a host reads named accounts and records of the
//! resource root from a run handle as a boundary system reads them, and
//! reads a kept root again by re-reading only the buckets that changed.
//!
//! The first fixture runs a probe system before the resource plugin's
//! systems in every boundary (the same phase, dispatched first by plugin
//! name). It reads the root with `ResourceRootReader::load` and
//! records what it holds after attaching one custodian's accounts, an
//! account of another custodian, and two named hot terminal records. Before
//! every boundary the host makes the same requests with
//! `ResourceRootReader::load_run` on a full-history run, a run that seals
//! every boundary, a run restored from a continuation checkpoint, and a run
//! reloaded from a snapshot, while the operations change the custodian's
//! accounts, add one, and archive the hot records; the host must hold what
//! the probe then records, and the helpers that read named records must
//! answer as the whole root does.
//!
//! The second keeps a whole reader and re-attaches it after a new live
//! bucket, changed buckets, and buckets an archive commit empties: the
//! reader equals the whole root `resource_state` reads and decodes exactly
//! the buckets whose records changed (none when nothing did). A partial
//! reader re-attaches to what a fresh one reads. The original and a fork of
//! it each write the same account once, differently, so a bucket and the
//! live root hold different content under the same version numbers in the
//! two runs; a re-attach of the original's kept root to the fork equals the
//! fork's whole root, and one to a restored run re-reads everything.
//!
//! The third reads through run handles that differ from the engine's: one
//! whose bucket is malformed, where a reader kept after a failed request
//! must fail again on re-attach as a fresh reader does, and one that returns
//! a fresh copy from `domain_record_shared`, which is read correctly with
//! nothing reused.

#![allow(clippy::too_many_lines)]

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundaryRequest,
    BoundarySystemContract, BoundaryWindow, Canwu, CanwuError, CanwuRun, CommandEnvelope,
    CommandRequest, CommandRequestId, CompactedCanwu, DomainRecord, DomainRecordClass,
    DomainRecordDraft, DomainRecordKind, DomainRecordMutation, DomainRecordRef, DomainRecordSchema,
    DomainRecordVersionRef, EntityRef, EvidenceCursor, IngressReceipt, IngressRecord, Issuer,
    KnowledgeHeads, KnowledgeHolderRef, KnowledgeQuery, KnowledgeQueryResult, KnowledgeRecordKind,
    KnowledgeRecordView, KnowledgeSubject, PayloadSchema, PersonId, PluginIngressPermit,
    PluginIngressRequest, PluginRegistrar, Scenario, SimDuration, SimTime, SimulationPlugin,
    SimulationView, StateVisibility, SystemCadence,
};
use canwu_resource::*;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, RwLock};

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
    ) -> Result<Option<Vec<u8>>, CanwuError> {
        Ok(self
            .objects
            .read()
            .unwrap()
            .get(&(namespace.to_owned(), object_id.to_owned()))
            .cloned())
    }
}

const CUSTODIAN: u64 = 1;
/// Sorts before `canwu-resource`, so its system runs first in their phase.
const PROBE: &str = "a-fixture-resource-probe";

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

fn floor() -> ProtectedFloorPolicyRevisionId {
    ProtectedFloorPolicyRevisionId::new("fixture:floor:reserve:v1").expect("floor")
}

fn other_floor() -> ProtectedFloorPolicyRevisionId {
    ProtectedFloorPolicyRevisionId::new("fixture:floor:reserve:v2").expect("floor")
}

fn demand_id(name: &str) -> ResourceDemandId {
    ResourceDemandId::new(format!("fixture:demand:{name}")).expect("demand")
}

fn new_account(
    id: ResourceAccountId,
    custodian: KnowledgeHolderRef,
    balance: u64,
) -> ResourceAccount {
    ResourceAccount {
        id,
        revision: ResourceRevision::INITIAL,
        custodian,
        resource_revision: resource(),
        unit_revision: unit(),
        balance,
        capacity: None,
        protected_floor_policy: None,
        closed: false,
        place_scope: None,
    }
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
        expires_at: SimTime::EPOCH + SimDuration::days(30),
        priority: 1,
        tie_break: ResourceTieBreakKey::new(format!("fixture:tie:{name}")).expect("tie"),
        admitted_sequence: 0,
        protected_floor_policy: None,
        protection_override_class: None,
        status: DemandStatus::Open,
        rejection_reason: None,
    }
}

/// A root with the custodian's two accounts, twenty accounts of other
/// custodians, a backlog of durable rejections, and the closure of a
/// cancelled demand, all hot terminal.
fn opening_root() -> Vec<DomainRecord> {
    let mut limits = ResourceLimitsV1::canonical();
    limits.max_archive_candidates = 256;
    limits.max_archive_maintenance_receipts = 256;
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
    state
        .install_protected_floor_policy(ProtectedFloorPolicyRevision {
            id: floor(),
            revision: ResourceRevision::INITIAL,
            floor: 10,
            override_classes: BTreeSet::new(),
            semantic_digest: "2".repeat(64),
        })
        .expect("floor policy");
    state
        .install_protected_floor_policy(ProtectedFloorPolicyRevision {
            id: other_floor(),
            revision: ResourceRevision::INITIAL,
            floor: 20,
            override_classes: BTreeSet::new(),
            semantic_digest: "3".repeat(64),
        })
        .expect("floor policy");
    for name in ["left", "right"] {
        state
            .install_opening_account(new_account(account(name), holder(), 100))
            .expect("account");
    }
    for index in 0..20_u64 {
        state
            .install_opening_account(new_account(
                account(&format!("world:{index}")),
                KnowledgeHolderRef::Person(PersonId::new(index + 2)),
                7,
            ))
            .expect("account");
    }
    for index in 0..24 {
        state
            .apply_operation(&ResourceOperationRequestV1::CancelDemand(
                ResourceCancelDemandRequestV1 {
                    operation_key: key(&format!("fixture:backlog:{index}")),
                    demand: ResourceDemandId::new(format!("fixture:missing:{index}"))
                        .expect("demand"),
                    expected_demand_revision: ResourceRevision::INITIAL,
                },
            ))
            .expect("durable rejection");
    }
    state
        .apply_operation(&ResourceOperationRequestV1::SubmitDemand(
            ResourceSubmitDemandRequestV1 {
                operation_key: key("fixture:submit:first"),
                demand: demand("first", 150),
            },
        ))
        .expect("submit");
    let revision = state.state_revision;
    state
        .apply_operation(&ResourceOperationRequestV1::Allocate(
            ResourceAllocationRequestV1 {
                operation_key: key("fixture:allocate:first"),
                expected_state_revision: revision,
                at: SimTime::EPOCH,
                candidate_limit: 4,
            },
        ))
        .expect("allocate");
    let revision = state.demands[&demand_id("first")].revision;
    state
        .apply_operation(&ResourceOperationRequestV1::CancelDemand(
            ResourceCancelDemandRequestV1 {
                operation_key: key("fixture:cancel:first"),
                demand: demand_id("first"),
                expected_demand_revision: revision,
            },
        ))
        .expect("cancel");
    state.into_records().expect("the root persists")
}

/// The requests both readers make.
fn read(reader: &mut ResourceRootReader<'_, '_>) -> Result<(), CanwuError> {
    reader.attach_custodian_accounts(&holder())?;
    reader.attach_live([ResourceLiveRecordKeyV1::Account(account("world:3"))])?;
    reader.attach([
        ResourceTerminalRecordKeyV1::Outcome(key("fixture:backlog:5")),
        ResourceTerminalRecordKeyV1::Demand(demand_id("first")),
    ])
}

fn answer(reader: ResourceRootReader<'_, '_>) -> Value {
    json!({
        "live_version": reader.live_record().version,
        "state": reader.into_state(),
    })
}

fn probe_kind() -> DomainRecordKind {
    DomainRecordKind::new(PROBE, "probe")
}

fn probe_ref() -> DomainRecordRef {
    DomainRecordRef {
        kind: probe_kind(),
        id: "probe".to_owned(),
    }
}

/// Records what the boundary's reader holds before the resource plugin
/// settles anything in this boundary.
fn probe(view: &SimulationView<'_>, _: &BoundaryContext) -> Result<BoundaryProposal, CanwuError> {
    let mut reader = ResourceRootReader::load(view)?.expect("the run has a resource root");
    read(&mut reader)?;
    let record = DomainRecordDraft::new(probe_ref(), answer(reader));
    let mutation = match view.domain_record(&probe_ref())? {
        None => DomainRecordMutation::Create { record },
        Some(stored) => DomainRecordMutation::Update {
            record,
            expected_version: stored.version,
        },
    };
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::MutateRecord {
            mutation,
            summary: "what the boundary read".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

struct ProbePlugin;

impl SimulationPlugin for ProbePlugin {
    fn name(&self) -> &'static str {
        PROBE
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000009301"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut schema = DomainRecordSchema::new(probe_kind(), DomainRecordClass::Record);
        schema.payload_schema = PayloadSchema::Any;
        let own = schema.state_key();
        registrar.register_record_schema(schema)?;
        let mut contract = BoundarySystemContract::new(
            "probe-resource-root",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        contract.reads = vec![
            own.clone(),
            DomainRecordSchema::for_record::<ResourceRuntimeRecord>().state_key(),
            resource_hot_terminal_state_key(),
        ];
        contract.writes = vec![own];
        contract.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(contract, probe)
    }
}

fn plugins() -> [&'static dyn SimulationPlugin; 2] {
    static RESOURCE: std::sync::OnceLock<ResourcePlugin> = std::sync::OnceLock::new();
    [RESOURCE.get_or_init(ResourcePlugin::default), &ProbePlugin]
}

fn new_run(archive: &MemoryArchive) -> Canwu {
    let scenario = Scenario::new(SimTime::EPOCH, vec![EntityRef::Person(PersonId::new(1))])
        .with_domain_records(opening_root());
    let mut run = Canwu::new_with_plugins(93, scenario, &plugins()).expect("runtime");
    run.set_plugin_archive_object_provider(Arc::new(archive.clone()));
    run
}

enum Handle {
    Full(Canwu),
    Compact { run: CompactedCanwu, seals: bool },
}

/// A run, its archive store, and the next command request ID.
struct Run {
    name: &'static str,
    handle: Handle,
    archive: MemoryArchive,
    next_request: u64,
}

impl Run {
    fn run(&self) -> &dyn CanwuRun {
        match &self.handle {
            Handle::Full(run) => run,
            Handle::Compact { run, .. } => run,
        }
    }

    fn run_mut(&mut self) -> &mut dyn CanwuRun {
        match &mut self.handle {
            Handle::Full(run) => run,
            Handle::Compact { run, .. } => run,
        }
    }

    /// Settles one boundary at the run's time, the probe included, after the
    /// host reads what the probe will read.
    fn settle(&mut self) {
        let mut reader = ResourceRootReader::load_run(self.run())
            .expect("the host reads the root")
            .expect("the run has a resource root");
        read(&mut reader).expect("the host attaches");
        let host = answer(reader);
        let at = self.run().time();
        let request = BoundaryRequest::at(at).with_cadence(SystemCadence::Daily);
        match &mut self.handle {
            Handle::Full(run) => {
                run.settle_boundary(request).expect("the boundary settles");
            }
            Handle::Compact { run, seals } => {
                run.settle_boundary(request).expect("the boundary settles");
                if *seals {
                    run.seal_evidence().expect("the run seals");
                }
            }
        }
        let probed = self
            .run()
            .domain_record(&probe_ref())
            .expect("the probe recorded its read")
            .payload
            .clone();
        assert_eq!(
            host, probed,
            "{}: the host reader differs from the boundary's",
            self.name
        );
    }

    /// Queues one tracked resource command of the custodian.
    fn command(&mut self, request: ResourceOperationRequestV1) {
        let at = self.run().time();
        let id = CommandRequestId::new(self.next_request);
        self.next_request += 1;
        let request = CommandRequest::new(
            id,
            self.run().revision(),
            CommandEnvelope::new(
                Issuer::Actor(PersonId::new(CUSTODIAN)),
                resource_command(&ResourceCommandV1 {
                    subject: holder(),
                    request,
                })
                .expect("command"),
            )
            .at_time(at),
        );
        match &mut self.handle {
            Handle::Full(run) => run.enqueue_command(at, 0, request),
            Handle::Compact { run, .. } => run.enqueue_command(at, 0, request),
        }
        .expect("the command queues");
    }

    fn state(&self) -> ResourceState {
        resource_state(self.run())
            .expect("resource query")
            .expect("resource root")
            .1
    }

    /// Points the custodian's account `left` at `policy` and settles it.
    fn set_left_floor(&mut self, policy: ProtectedFloorPolicyRevisionId) {
        let expected_account_revision = self.state().accounts[&account("left")].revision;
        self.command(ResourceOperationRequestV1::SetProtectedFloor(
            ResourceProtectedFloorRequestV1 {
                operation_key: key("fixture:refloor:left"),
                account: account("left"),
                expected_account_revision,
                policy: Some(policy),
                install: None,
            },
        ));
        self.settle();
        self.settle();
    }

    /// One host operation and the boundaries that settle it.
    fn operate(&mut self, step: usize) {
        match step {
            // Two allocation passes: new outcomes in their buckets.
            0 | 3 => {
                let at = self.run().time();
                let request = ResourceAllocationRequestV1 {
                    operation_key: key(&format!("fixture:pass:{step}")),
                    expected_state_revision: self.state().state_revision,
                    at,
                    candidate_limit: 4,
                };
                enqueue_resource_allocation(self.run_mut(), at, &holder(), &request)
                    .expect("allocation pass");
            }
            // A new account of the custodian, which grows its index.
            1 => self.command(ResourceOperationRequestV1::CreateAccount(
                ResourceCreateAccountRequestV1 {
                    operation_key: key("fixture:create"),
                    account: new_account(account("created"), holder(), 0),
                },
            )),
            // A protected floor on one of the custodian's accounts, which
            // changes the account, and a demand on its accounts, which the
            // pass of step 3 allocates.
            2 => {
                let expected_account_revision = self.state().accounts[&account("left")].revision;
                self.command(ResourceOperationRequestV1::SetProtectedFloor(
                    ResourceProtectedFloorRequestV1 {
                        operation_key: key("fixture:floor:left"),
                        account: account("left"),
                        expected_account_revision,
                        policy: Some(floor()),
                        install: None,
                    },
                ));
                self.command(ResourceOperationRequestV1::SubmitDemand(
                    ResourceSubmitDemandRequestV1 {
                        operation_key: key("fixture:submit:second"),
                        demand: demand("second", 30),
                    },
                ));
            }
            // Archive every hot terminal record: their buckets empty.
            4 => {
                let state = self.state();
                let archive = self.archive.clone();
                let prepared = state
                    .prepare_resource_archive(state.terminal_archive_candidates.len(), &archive)
                    .expect("prepare");
                enqueue_resource_archive(self.run_mut(), &prepared, &archive).expect("commit");
            }
            _ => unreachable!("the fixture has five steps"),
        }
        self.settle();
        self.settle();
    }
}

/// The resource bucket records of a run, by reference.
fn buckets(run: &dyn CanwuRun) -> BTreeMap<String, DomainRecord> {
    run.domain_records()
        .filter(|record| record.reference.kind.matches_type::<ResourceBucketRecord>())
        .map(|record| (record.reference.to_string(), record.clone()))
        .collect()
}

#[test]
fn a_host_reader_reads_what_a_boundary_reader_reads_in_every_run() {
    let full_archive = MemoryArchive::default();
    let sealed_archive = MemoryArchive::default();
    let mut runs = vec![
        Run {
            name: "full",
            handle: Handle::Full(new_run(&full_archive)),
            archive: full_archive,
            next_request: 1,
        },
        Run {
            name: "sealed",
            handle: Handle::Compact {
                run: new_run(&sealed_archive)
                    .into_compacted()
                    .expect("the run compacts"),
                seals: true,
            },
            archive: sealed_archive,
            next_request: 1,
        },
    ];
    let before = runs[0].state();
    let (mut legs_seen, mut outcomes_seen) = (false, false);
    for step in 0..5 {
        if step == 2 {
            let Handle::Full(full) = &runs[0].handle else {
                unreachable!("the first run is the full one")
            };
            let (checkpoint, hash) = full
                .continuation_checkpoint()
                .expect("the continuation checkpoint builds");
            let restored_archive = MemoryArchive::default();
            let mut restored = CompactedCanwu::from_continuation_checkpoint_verified(
                checkpoint,
                &hash,
                &plugins(),
            )
            .expect("the run restores");
            restored.set_plugin_archive_object_provider(Arc::new(restored_archive.clone()));
            let reloaded_archive = MemoryArchive::default();
            let mut reloaded = Canwu::from_snapshot_json_with_plugins(
                &full.snapshot_json().expect("the snapshot encodes"),
                &plugins(),
            )
            .expect("the run reloads");
            reloaded.set_plugin_archive_object_provider(Arc::new(reloaded_archive.clone()));
            let next_request = runs[0].next_request;
            runs.push(Run {
                name: "continuation",
                handle: Handle::Compact {
                    run: restored,
                    seals: false,
                },
                archive: restored_archive,
                next_request,
            });
            runs.push(Run {
                name: "reloaded",
                handle: Handle::Full(reloaded),
                archive: reloaded_archive,
                next_request,
            });
        }
        for run in &mut runs {
            run.operate(step);
            let (legs, outcomes) = helpers_answer_as_the_whole_root(run.run());
            legs_seen |= legs;
            outcomes_seen |= outcomes;
        }
        let first = runs[0].state();
        for run in &runs[1..] {
            assert_eq!(run.state(), first, "{} after step {step}", run.name);
        }
    }

    // The custodian's accounts changed, one was added, and the hot records
    // left for the archive.
    let after = runs[0].state();
    assert_ne!(
        after.accounts[&account("left")],
        before.accounts[&account("left")]
    );
    assert!(after.custodian_accounts[&holder()].contains(&account("created")));
    assert!(!before.outcomes.is_empty() && after.outcomes.len() < before.outcomes.len());

    assert!(
        legs_seen && outcomes_seen,
        "the helpers met allocation legs and outcomes"
    );
}

/// Checks that the helpers that read named records answer as the whole root
/// does, and returns whether the root held allocation legs and outcomes.
fn helpers_answer_as_the_whole_root(run: &dyn CanwuRun) -> (bool, bool) {
    let (_, whole) = resource_state(run).expect("resource query").expect("root");
    for leg in whole.allocation_legs.values() {
        assert_eq!(
            resource_allocation_leg(run, &leg.id),
            Some(ResourceAllocationLegVersionV1::from(leg))
        );
        assert_eq!(
            exact_resource_allocation_leg(run, &leg.into()).as_ref(),
            Ok(leg)
        );
    }
    for (operation_key, outcome) in &whole.outcomes {
        assert_eq!(
            resource_operation_outcome(run, operation_key),
            Some(outcome.into())
        );
        assert_eq!(
            exact_resource_operation_outcome(run, &outcome.into()).as_ref(),
            Ok(outcome)
        );
    }
    assert_eq!(
        resource_operation_outcome(run, &key("fixture:backlog:5")),
        whole
            .outcomes
            .get(&key("fixture:backlog:5"))
            .map(Into::into)
    );
    (
        !whole.allocation_legs.is_empty(),
        !whole.outcomes.is_empty(),
    )
}

/// The bucket records `after` holds that `before` did not hold as they are.
fn changed(
    before: &BTreeMap<String, DomainRecord>,
    after: &BTreeMap<String, DomainRecord>,
) -> usize {
    after
        .iter()
        .filter(|(reference, record)| before.get(*reference) != Some(*record))
        .count()
}

/// A kept root borrows nothing and may move to another thread, as a host's
/// save or worker thread takes it between steps.
fn _a_kept_root_is_send(kept: KeptResourceRoot) -> impl Send + 'static {
    kept
}

/// Re-attaches `kept` to `run` and checks it against the whole root, and
/// that it decoded `decoded` buckets.
fn reattached(run: &dyn CanwuRun, kept: KeptResourceRoot, decoded: usize) -> KeptResourceRoot {
    let reader = ResourceRootReader::reattach(run, kept)
        .expect("the reader re-attaches")
        .expect("the run has a resource root");
    let (live, whole) = resource_state(run).expect("resource query").expect("root");
    assert_eq!(reader.state(), &whole);
    assert_eq!(reader.live_record(), &live);
    assert_eq!(reader.buckets_decoded(), decoded);
    reader.keep()
}

#[test]
fn a_reattached_reader_equals_a_full_load_and_decodes_only_changed_buckets() {
    for seals in [false, true] {
        let archive = MemoryArchive::default();
        let handle = if seals {
            Handle::Compact {
                run: new_run(&archive).into_compacted().expect("compacts"),
                seals: true,
            }
        } else {
            Handle::Full(new_run(&archive))
        };
        let mut run = Run {
            name: if seals { "sealed" } else { "full" },
            handle,
            archive,
            next_request: 1,
        };
        let mut whole = ResourceRootReader::load_run(run.run())
            .expect("load")
            .expect("root");
        whole.attach_all().expect("the whole root attaches");
        let stored = buckets(run.run());
        assert_eq!(whole.buckets_decoded(), stored.len());
        assert_eq!(whole.state(), &run.state());
        let mut kept = whole.keep();

        // A partial reader, kept beside the whole one.
        let mut partial = ResourceRootReader::load_run(run.run())
            .expect("load")
            .expect("root");
        read(&mut partial).expect("attach");
        let mut partial_kept = partial.keep();

        // Nothing changed: nothing is decoded.
        kept = reattached(run.run(), kept, 0);

        let (mut new_bucket, mut changed_bucket, mut emptied_bucket) = (false, false, false);
        for step in 0..5 {
            let before = buckets(run.run());
            run.operate(step);
            let after = buckets(run.run());
            new_bucket |= after
                .keys()
                .any(|reference| !before.contains_key(reference));
            changed_bucket |= after
                .iter()
                .any(|(reference, record)| before.get(reference).is_some_and(|old| old != record));
            emptied_bucket |= after.iter().any(|(reference, record)| {
                record.payload["entries"]
                    .as_array()
                    .is_some_and(Vec::is_empty)
                    && before.get(reference).is_some_and(|old| {
                        old.payload["entries"]
                            .as_array()
                            .is_some_and(|entries| !entries.is_empty())
                    })
            });
            kept = reattached(run.run(), kept, changed(&before, &after));

            let reader = ResourceRootReader::reattach(run.run(), partial_kept)
                .expect("the partial reader re-attaches")
                .expect("root");
            let mut fresh = ResourceRootReader::load_run(run.run())
                .expect("load")
                .expect("root");
            read(&mut fresh).expect("attach");
            assert_eq!(reader.state(), fresh.state(), "{} step {step}", run.name);
            assert!(reader.buckets_decoded() <= fresh.buckets_decoded());
            partial_kept = reader.keep();
        }
        assert!(
            new_bucket && changed_bucket && emptied_bucket,
            "{}: the steps add, change, and empty buckets",
            run.name
        );
    }

    // The original and a fork each write account `left` once, with
    // different floors: its live bucket, and the live root, hold different
    // content under the same version numbers in the two runs. What was kept
    // from the original is not taken for the fork's.
    let archive = MemoryArchive::default();
    let mut original = Run {
        name: "original",
        handle: Handle::Full(new_run(&archive)),
        archive,
        next_request: 1,
    };
    original.operate(0);
    let Handle::Full(base) = &original.handle else {
        unreachable!("a full run")
    };
    let mut fork = Run {
        name: "fork",
        handle: Handle::Full(base.fork()),
        archive: original.archive.clone(),
        next_request: original.next_request,
    };
    original.set_left_floor(floor());
    fork.set_left_floor(other_floor());
    let mut whole = ResourceRootReader::load_run(original.run())
        .expect("load")
        .expect("root");
    whole.attach_all().expect("attach");
    let kept = whole.keep();
    let forked = buckets(fork.run());
    let same_version_other_content = |left: &DomainRecord, right: &DomainRecord| {
        left.version == right.version && left.payload != right.payload
    };
    let live_root = |run: &Run| {
        run.run()
            .domain_record(resource_runtime_reference().as_untyped())
            .cloned()
            .expect("root")
    };
    assert!(
        same_version_other_content(&live_root(&original), &live_root(&fork)),
        "the live roots differ under one version number"
    );
    let left_bucket = resource_live_bucket_reference(resource_live_bucket(
        &ResourceLiveRecordKeyV1::Account(account("left")),
        resource_live_bucket_count(&original.state().limits),
    ))
    .into_untyped()
    .to_string();
    assert!(
        same_version_other_content(
            &buckets(original.run())[&left_bucket],
            &forked[&left_bucket]
        ),
        "the bucket of `left` differs under one version number"
    );
    let reader = ResourceRootReader::reattach(fork.run(), kept)
        .expect("re-attach")
        .expect("root");
    assert_eq!(reader.state(), &fork.state());
    assert!(reader.buckets_decoded() > 0 && reader.buckets_decoded() < forked.len());

    // A run restored from a snapshot holds none of the original's records.
    let kept = reader.keep();
    let Handle::Full(fork_run) = &fork.handle else {
        unreachable!("a full run")
    };
    let reloaded = Canwu::from_snapshot_json_with_plugins(
        &fork_run.snapshot_json().expect("snapshot"),
        &plugins(),
    )
    .expect("reload");
    reattached(&reloaded, kept, forked.len());
}

/// A run handle that reads an engine run but answers for some records with
/// its own copies, and may hand out a fresh copy as a record's shared copy.
struct Altered<'r> {
    run: &'r Canwu,
    replaced: BTreeMap<DomainRecordRef, Arc<DomainRecord>>,
    fresh_copies: bool,
}

impl Altered<'_> {
    fn read<'a>(&'a self, record: &'a DomainRecord) -> &'a DomainRecord {
        self.replaced
            .get(&record.reference)
            .map_or(record, |replaced| replaced.as_ref())
    }
}

impl CanwuRun for Altered<'_> {
    fn time(&self) -> SimTime {
        self.run.time()
    }
    fn revision(&self) -> u64 {
        self.run.revision()
    }
    fn checkpoint_hash(&self) -> &str {
        self.run.checkpoint_hash()
    }
    fn entity_exists(&self, entity: &EntityRef) -> bool {
        self.run.entity_exists(entity)
    }
    fn domain_record(&self, reference: &DomainRecordRef) -> Option<&DomainRecord> {
        self.run
            .domain_record(reference)
            .map(|record| self.read(record))
    }
    fn domain_records(&self) -> Box<dyn Iterator<Item = &DomainRecord> + '_> {
        Box::new(self.run.domain_records().map(|record| self.read(record)))
    }
    fn current_domain_record_version(
        &self,
        reference: &DomainRecordRef,
    ) -> Result<Option<DomainRecordVersionRef>, CanwuError> {
        self.run.current_domain_record_version(reference)
    }
    fn domain_record_version_evidence_exists(&self, reference: &DomainRecordVersionRef) -> bool {
        self.run.domain_record_version_evidence_exists(reference)
    }
    fn replay_stable_domain_record_version(
        &self,
        reference: &DomainRecordVersionRef,
    ) -> Option<(DomainRecord, SimTime)> {
        self.run.replay_stable_domain_record_version(reference)
    }
    fn evidence_cursor(&self) -> Result<EvidenceCursor, CanwuError> {
        self.run.evidence_cursor()
    }
    fn sealed_evidence_cursor(&self) -> Option<EvidenceCursor> {
        self.run.sealed_evidence_cursor()
    }
    fn boundaries_between(&self, from: SimTime, to: SimTime) -> Result<BoundaryWindow, CanwuError> {
        self.run.boundaries_between(from, to)
    }
    fn boundary_schedule_horizon(&self) -> SimDuration {
        self.run.boundary_schedule_horizon()
    }
    fn pending_ingress(&self) -> Box<dyn Iterator<Item = &IngressRecord> + '_> {
        Box::new(self.run.pending_ingress())
    }
    fn enqueue_plugin_ingress(
        &mut self,
        _request: PluginIngressRequest,
    ) -> Result<IngressReceipt, CanwuError> {
        unreachable!("the handle only reads")
    }
    fn enqueue_permitted_plugin_ingress(
        &mut self,
        _request: PluginIngressRequest,
        _permit: &PluginIngressPermit,
    ) -> Result<IngressReceipt, CanwuError> {
        unreachable!("the handle only reads")
    }
    fn admin_query_knowledge(
        &self,
        holder: KnowledgeHolderRef,
        query: &KnowledgeQuery,
    ) -> Result<KnowledgeQueryResult, CanwuError> {
        self.run.admin_query_knowledge(holder, query)
    }
    fn knowledge_heads(
        &self,
        holder: &KnowledgeHolderRef,
        kind: &KnowledgeRecordKind,
        subjects: &[KnowledgeSubject],
    ) -> Result<KnowledgeHeads, CanwuError> {
        self.run.knowledge_heads(holder, kind, subjects)
    }
    fn knowledge_current_records<'a>(
        &'a self,
        holder: &'a KnowledgeHolderRef,
        kind: &'a KnowledgeRecordKind,
        subjects: &'a [KnowledgeSubject],
    ) -> Box<dyn Iterator<Item = KnowledgeRecordView> + 'a> {
        Box::new(self.run.knowledge_current_records(holder, kind, subjects))
    }
    fn domain_record_shared(&self, reference: &DomainRecordRef) -> Option<Arc<DomainRecord>> {
        if self.fresh_copies {
            return self
                .domain_record(reference)
                .map(|record| Arc::new(record.clone()));
        }
        self.replaced
            .get(reference)
            .cloned()
            .or_else(|| self.run.domain_record_shared(reference))
    }
    fn domain_records_of_kind(
        &self,
        kind: &DomainRecordKind,
    ) -> Box<dyn Iterator<Item = &DomainRecord> + '_> {
        Box::new(
            self.run
                .domain_records_of_kind(kind)
                .map(|record| self.read(record)),
        )
    }
}

#[test]
fn a_reader_kept_after_a_failed_request_fails_again_on_reattach() {
    let archive = MemoryArchive::default();
    let run = new_run(&archive);
    let whole = run_state(&run);

    // The live bucket of `world:7` with that account's balance malformed.
    let malformed = ResourceLiveRecordKeyV1::Account(account("world:7"));
    let reference = resource_live_bucket_reference(resource_live_bucket(
        &malformed,
        resource_live_bucket_count(&whole.limits),
    ))
    .into_untyped();
    let mut record = run.domain_record(&reference).expect("bucket").clone();
    let entry = record.payload["entries"]
        .as_array_mut()
        .expect("entries")
        .iter_mut()
        .find(|entry| entry["key"]["id"] == "fixture:account:world:7")
        .expect("the account's entry");
    entry["payload"]["record"]["balance"] = json!("malformed");
    let altered = Altered {
        run: &run,
        replaced: BTreeMap::from([(reference, Arc::new(record))]),
        fresh_copies: false,
    };

    // A fresh reader fails on the whole root and on the account.
    let mut fresh = ResourceRootReader::load_run(&altered)
        .expect("load")
        .expect("root");
    assert!(fresh.attach_all().is_err());
    let mut fresh = ResourceRootReader::load_run(&altered)
        .expect("load")
        .expect("root");
    assert!(fresh.attach_live([malformed.clone()]).is_err());

    // A reader kept after the same failure fails again on the unchanged run,
    // rather than answering with what it attached before the failure.
    let mut whole_reader = ResourceRootReader::load_run(&altered)
        .expect("load")
        .expect("root");
    assert!(whole_reader.attach_all().is_err());
    assert!(ResourceRootReader::reattach(&altered, whole_reader.keep()).is_err());
    let mut targeted = ResourceRootReader::load_run(&altered)
        .expect("load")
        .expect("root");
    read(&mut targeted).expect("the other requests read");
    assert!(targeted.attach_live([malformed.clone()]).is_err());
    let kept = targeted.keep();
    assert!(ResourceRootReader::reattach(&altered, kept.clone()).is_err());

    // Re-attached to the engine's run, which holds the account as it is, the
    // kept requests read what a fresh reader reads.
    let reader = ResourceRootReader::reattach(&run, kept)
        .expect("the requests read")
        .expect("root");
    let mut fresh = ResourceRootReader::load_run(&run)
        .expect("load")
        .expect("root");
    read(&mut fresh).expect("attach");
    fresh.attach_live([malformed]).expect("attach");
    assert_eq!(reader.state(), fresh.state());

    // A handle whose shared copy is a fresh one each call is read correctly,
    // with nothing reused.
    let copies = Altered {
        run: &run,
        replaced: BTreeMap::new(),
        fresh_copies: true,
    };
    let mut whole_reader = ResourceRootReader::load_run(&copies)
        .expect("load")
        .expect("root");
    whole_reader.attach_all().expect("attach");
    let reader = ResourceRootReader::reattach(&copies, whole_reader.keep())
        .expect("re-attach")
        .expect("root");
    assert_eq!(reader.state(), &whole);
    assert_eq!(reader.buckets_decoded(), buckets(&run).len());
}

fn run_state(run: &dyn CanwuRun) -> ResourceState {
    resource_state(run)
        .expect("resource query")
        .expect("root")
        .1
}
