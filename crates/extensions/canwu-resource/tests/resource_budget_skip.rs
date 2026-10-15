//! A resource operation whose result does not fit the root is skipped, never
//! a failed boundary.
//!
//! Room is measured on what an operation would leave: a refused operation
//! needs room for its own outcome only, an applied one for every terminal
//! record it adds and every reservation it makes. An operation that does not
//! fit changes nothing, its boundary commits, and one skip event names its
//! packet, its operation key, and whether an archive commit makes room. The
//! key stays free, so the same request settles when it is sent again.
//!
//! The first fixture queues more operations in one boundary than an archive
//! commit can make room for: the ones that fit settle, the rest are skipped,
//! and every one settles exactly once as the host application sends the
//! skipped ones again behind further commits. The second fixture fills the
//! candidate budget with the reservations of active single-step leases: the
//! next lease is skipped, an archive commit does not make room for it, a
//! refused lease is still recorded because it needs room for its outcome
//! only, and releasing a lease makes room. The third settles the same skips
//! in a sealed run, an unsealed fork, a run restored from the sealed
//! segments, and an exact replay, and all four agree.

use canwu_api::{
    BoundaryRequest, Canwu, CommandEnvelope, CommandRequest, CommandRequestId, CompactedCanwu,
    EntityRef, IngressId, Issuer, KnowledgeHolderRef, PersonId, PluginIngressRequest, Scenario,
    SimEvent, SimTime,
};
use canwu_resource::*;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

/// Stored archive objects by namespace and object id.
type ArchiveObjects = BTreeMap<(String, String), Vec<u8>>;

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
    ) -> Result<Option<Vec<u8>>, canwu_api::CanwuError> {
        Ok(self
            .objects
            .borrow()
            .get(&(namespace.to_owned(), object_id.to_owned()))
            .cloned())
    }
}

const NAMESPACE: &str = "fixture.resource";
const CUSTODIAN: u64 = 1;

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

/// A root under `limits` with one custodian, a completion-capacity partition
/// for it, and four of its accounts.
fn world(limits: ResourceLimitsV1) -> ResourceState {
    let mut state = ResourceState::empty(limits).expect("state");
    state
        .install_run_budget(
            RunBudgetRevisionV1 {
                revision: ResourceRevision::INITIAL,
                total_completion_units: 2_000_000,
                shared_pending_slots: 4,
                partitions: vec![CompletionCapacityPartitionV1 {
                    authority: holder(),
                    operation_namespace: NAMESPACE.to_owned(),
                    guaranteed_units: 400_000,
                    reserved_pending_slots: 8,
                    maximum_burst_units: 100_000,
                    request_token_capacity: 16,
                    request_token_refill_minutes: 1,
                    reacquire_cooldown_minutes: 1,
                    root_acquisition_cap_per_sim_time: 8,
                    guaranteed_max_wait_boundaries: 4,
                }],
                semantic_digest: String::new(),
            }
            .seal()
            .expect("budget"),
        )
        .expect("install budget");
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
    for index in 0..4 {
        state
            .install_opening_account(new_account(&format!("opening-{index}"), 10))
            .expect("account");
    }
    state
}

fn new_account(name: &str, balance: u64) -> ResourceAccount {
    ResourceAccount {
        id: account(name),
        revision: ResourceRevision::INITIAL,
        custodian: holder(),
        resource_revision: resource(),
        unit_revision: unit(),
        balance,
        capacity: None,
        protected_floor_policy: None,
        closed: false,
        place_scope: None,
    }
}

/// One durable rejection: cancelling a demand that does not exist leaves a
/// terminal outcome and nothing else.
fn reject(state: &mut ResourceState, index: usize) {
    state
        .apply_operation(&ResourceOperationRequestV1::CancelDemand(
            ResourceCancelDemandRequestV1 {
                operation_key: key(&format!("fixture:cancel:{index}")),
                demand: ResourceDemandId::new(format!("fixture:missing:{index}")).expect("demand"),
                expected_demand_revision: ResourceRevision::INITIAL,
            },
        ))
        .expect("durable rejection");
}

fn runtime(resource: ResourceState, archive: &MemoryArchive) -> Canwu {
    let scenario = Scenario::new(SimTime::EPOCH, vec![EntityRef::Person(PersonId::new(1))])
        .with_domain_records(vec![resource.into_record().expect("resource record")]);
    let mut canwu =
        Canwu::new_with_plugins(16, scenario, &[&ResourcePlugin::default()]).expect("runtime");
    canwu.set_plugin_archive_object_provider(Rc::new(archive.clone()));
    canwu
}

fn state(canwu: &Canwu) -> ResourceState {
    resource_state(canwu)
        .expect("resource query")
        .expect("resource state")
        .1
}

fn settle(canwu: &mut Canwu) {
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("a resource boundary never fails in this fixture");
}

fn skips(events: &[SimEvent]) -> Vec<ResourceIngressSkipV1> {
    events
        .iter()
        .filter_map(ResourceIngressSkipV1::from_event)
        .collect()
}

/// Queues one commit of every hot candidate.
fn queue_archive_commit(canwu: &mut Canwu, archive: &MemoryArchive) {
    let hot = state(canwu);
    let prepared = hot
        .prepare_resource_archive(hot.terminal_archive_candidates.len(), archive)
        .expect("prepare archive commit");
    enqueue_resource_archive(canwu, &prepared, archive).expect("enqueue archive commit");
}

/// The outcome of `operation_key`, in the root or the archive.
fn recorded(canwu: &Canwu, archive: &MemoryArchive, operation_key: &ResourceOperationKey) -> bool {
    let root = state(canwu);
    root.outcomes.contains_key(operation_key)
        || archived_resource_operation_outcome(&root, archive, operation_key)
            .expect("archive lookup")
            .is_some()
}

#[test]
fn operations_past_the_room_of_one_archive_commit_are_skipped_and_settle_when_sent_again() {
    let mut limits = ResourceLimitsV1::canonical();
    limits.max_archive_candidates = 4;
    let mut resource = world(limits);
    for index in 0..4 {
        reject(&mut resource, index);
    }
    let archive = MemoryArchive::default();
    let mut canwu = runtime(resource, &archive);

    // Seven allocation passes and the commit of the four hot records, in one
    // boundary. The commit settles first and leaves its own receipt, so it
    // makes room for three operations, fewer than are due: before this
    // change, such a boundary could never settle.
    let at = canwu.time();
    let mut pending: Vec<ResourceAllocationRequestV1> = (0..7)
        .map(|index| ResourceAllocationRequestV1 {
            operation_key: key(&format!("fixture:allocate:{index}")),
            expected_state_revision: state(&canwu).state_revision,
            at,
            candidate_limit: 1,
        })
        .collect();
    queue_archive_commit(&mut canwu, &archive);
    let mut settled_per_round = Vec::new();
    while !pending.is_empty() {
        assert!(settled_per_round.len() < 4, "the skipped operations settle");
        let events = canwu.events().len();
        let queued: BTreeMap<IngressId, ResourceAllocationRequestV1> = pending
            .iter()
            .map(|request| {
                let receipt = enqueue_resource_allocation(&mut canwu, at, &holder(), request)
                    .expect("enqueue allocation");
                (receipt.ingress_id, request.clone())
            })
            .collect();
        settle(&mut canwu);

        // Every packet that did not fit is named once, with its key, as an
        // archive backlog: a commit makes room for it. Nothing of it was
        // recorded, so its key is still free.
        let skipped = skips(&canwu.events()[events..]);
        for skip in &skipped {
            assert_eq!(skip.reason, ResourceIngressSkipReasonV1::ArchiveBacklog);
            assert_eq!(
                skip.operation_key.as_ref(),
                Some(&queued[&skip.ingress].operation_key)
            );
            assert!(!recorded(
                &canwu,
                &archive,
                &queued[&skip.ingress].operation_key
            ));
        }
        let skipped_ingress: BTreeSet<_> = skipped.iter().map(|skip| skip.ingress).collect();
        assert_eq!(skipped_ingress.len(), skipped.len());
        for (ingress, request) in &queued {
            if !skipped_ingress.contains(ingress) {
                assert!(recorded(&canwu, &archive, &request.operation_key));
            }
        }
        settled_per_round.push(queued.len() - skipped.len());
        pending = skipped
            .iter()
            .map(|skip| queued[&skip.ingress].clone())
            .collect();
        // The host application archives and sends the skipped requests again.
        if !pending.is_empty() {
            queue_archive_commit(&mut canwu, &archive);
        }
    }
    // Each commit makes room for three outcomes beside its receipt.
    assert_eq!(settled_per_round, [3, 3, 1]);
    for index in 0..7 {
        assert!(recorded(
            &canwu,
            &archive,
            &key(&format!("fixture:allocate:{index}"))
        ));
    }
}

/// The custodian's acquisition `name`, whose only participant is the
/// resource plugin.
fn lease_request(name: &str, eligible_at: SimTime) -> RequestCompletionLeaseV1 {
    RequestCompletionLeaseV1 {
        id: CompletionLeaseAcquisitionId::new(format!("fixture:lease:{name}"))
            .expect("acquisition"),
        operation_key: key(&format!("fixture:operation:{name}")),
        holder: holder(),
        operation_namespace: NAMESPACE.to_owned(),
        eligibility_time: eligible_at,
        eligibility_envelope: EligibilityEnvelopeV1::new(
            Vec::new(),
            BTreeMap::new(),
            BTreeSet::new(),
            Vec::new(),
            Vec::new(),
        )
        .expect("envelope"),
        recipe: CompletionCapacityRecipeV1 {
            receipts: MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE,
            mutations: 2,
            reports_per_holder: 0,
            holders: 0,
            bytes: 1_024,
        },
        expected_participants: BTreeSet::from([PLUGIN_NAME.to_owned()]),
        policy_class: CompletionPolicyClassV1::Guaranteed,
    }
}

/// A single-step lease of the custodian on one opening account.
fn lease(
    canwu: &Canwu,
    name: &str,
    target: usize,
    eligible_at: SimTime,
) -> SingleStepCompletionLeaseV1 {
    let current = state(canwu);
    let id = account(&format!("opening-{target}"));
    let targets = vec![CompletionLockedTargetV1::Account {
        id: id.clone(),
        revision: current.accounts[&id].revision,
    }];
    SingleStepCompletionLeaseV1 {
        lease: lease_request(name, eligible_at),
        grant_id: CompletionCapacityGrantId::new(format!("fixture:grant:{name}")).expect("grant"),
        target_versions: targets,
        current_boundary: canwu
            .boundaries()
            .last()
            .map_or(1, |boundary| boundary.id.get() + 1),
    }
}

/// Settles one completion operation in a boundary of its own at `at` and
/// returns the skips that boundary emitted.
fn complete(
    canwu: &mut Canwu,
    at: SimTime,
    operation: &ResourceCompletionOperationV1,
) -> Vec<ResourceIngressSkipV1> {
    let events = canwu.events().len();
    let receipt =
        enqueue_resource_completion_operation(canwu, at, operation).expect("completion ingress");
    canwu
        .settle_boundary(BoundaryRequest::at(at))
        .expect("a resource boundary never fails in this fixture");
    let skipped = skips(&canwu.events()[events..]);
    for skip in &skipped {
        assert_eq!(skip.ingress, receipt.ingress_id);
        assert_eq!(skip.operation_key, Some(operation.operation_key()));
    }
    skipped
}

fn reasons(skips: &[ResourceIngressSkipV1]) -> Vec<ResourceIngressSkipReasonV1> {
    skips.iter().map(|skip| skip.reason).collect()
}

#[test]
fn a_lease_without_room_beside_reserved_capacity_is_skipped_and_a_refused_one_is_recorded() {
    // Each active lease leaves two records and reserves sixteen slots, so
    // two leases fit a budget of 40 and a third does not. Each lease is
    // taken a minute after the last, as the custodian's partition allows.
    let mut limits = ResourceLimitsV1::canonical();
    limits.max_archive_candidates = 40;
    let archive = MemoryArchive::default();
    let mut canwu = runtime(world(limits), &archive);
    let minute = |value: i64| SimTime::EPOCH + canwu_api::SimDuration::minutes(value);
    for (at, name, target) in [(minute(0), "first", 0), (minute(1), "second", 1)] {
        let operation =
            ResourceCompletionOperationV1::SingleStepLease(lease(&canwu, name, target, at));
        assert!(complete(&mut canwu, at, &operation).is_empty());
    }
    let two_leases = state(&canwu);
    assert_eq!(two_leases.completion_leases.certificates.len(), 2);
    assert_eq!(two_leases.terminal_archive_candidates.len(), 4);

    // The third lease would hold 4 + 2 records beside 48 reserved slots. It
    // is skipped, its boundary commits, and nothing of it is recorded. Before
    // this change the boundary failed.
    let third = |canwu: &Canwu, at: SimTime| {
        ResourceCompletionOperationV1::SingleStepLease(lease(canwu, "third", 2, at))
    };
    let operation = third(&canwu, minute(2));
    assert_eq!(
        reasons(&complete(&mut canwu, minute(2), &operation)),
        [ResourceIngressSkipReasonV1::ReservedCapacity]
    );
    assert_eq!(state(&canwu), two_leases);

    // A lease the simulation view refuses, eligible a minute after its
    // boundary, needs room for its outcome only, so its rejection is
    // recorded. Before this change it asked for the room of a whole lease
    // and failed the boundary.
    let refused =
        ResourceCompletionOperationV1::SingleStepLease(lease(&canwu, "refused", 3, minute(4)));
    assert!(complete(&mut canwu, minute(3), &refused).is_empty());
    let outcome = &state(&canwu).outcomes[&refused.operation_key()];
    assert_eq!(outcome.status, ResourceOperationStatus::Rejected);
    assert_eq!(outcome.rejection_code.as_deref(), Some("invalid_authority"));

    // An archive commit does not make room for the third lease: the room is
    // reserved by the two active leases.
    queue_archive_commit(&mut canwu, &archive);
    settle(&mut canwu);
    assert_eq!(state(&canwu).terminal_archive_candidates.len(), 1);
    let operation = third(&canwu, minute(4));
    assert_eq!(
        reasons(&complete(&mut canwu, minute(4), &operation)),
        [ResourceIngressSkipReasonV1::ReservedCapacity]
    );

    // Releasing the first lease frees its reservation, and the third lease,
    // sent again under the same key, settles.
    let current = state(&canwu);
    let first = CompletionLeaseAcquisitionId::new("fixture:lease:first").expect("acquisition");
    let (grant, expected_grant_revision) =
        current.completion_leases.certificates[&first].prepared_grants[0].clone();
    let release = ResourceCompletionOperationV1::Release(ReleaseCompletionCapacityV1 {
        acquisition: first.clone(),
        expected_acquisition_revision: current.completion_leases.acquisitions[&first].revision,
        grant,
        expected_grant_revision,
        reason: "the custodian gives the lease up".to_owned(),
    });
    assert!(complete(&mut canwu, minute(5), &release).is_empty());
    let operation = third(&canwu, minute(6));
    assert!(complete(&mut canwu, minute(6), &operation).is_empty());
    let third_id = CompletionLeaseAcquisitionId::new("fixture:lease:third").expect("acquisition");
    assert!(
        state(&canwu)
            .completion_leases
            .certificates
            .contains_key(&third_id)
    );
}

/// One tracked `CreateAccount` of the custodian.
fn create_account(
    request_id: u64,
    revision: u64,
    at: SimTime,
    operation_key: &str,
    name: &str,
) -> CommandRequest {
    CommandRequest::new(
        CommandRequestId::new(request_id),
        revision,
        CommandEnvelope::new(
            Issuer::Actor(PersonId::new(CUSTODIAN)),
            resource_command(&ResourceCommandV1 {
                subject: holder(),
                request: ResourceOperationRequestV1::CreateAccount(
                    ResourceCreateAccountRequestV1 {
                        operation_key: key(operation_key),
                        account: new_account(name, 0),
                    },
                ),
            })
            .expect("command"),
        )
        .at_time(at),
    )
}

/// The commands and packets after the seal: one that fits the last free
/// slot, one admitted with the same key that settles behind it, one that
/// does not fit, and a resource command packet that no command produced.
fn after_seal(run: &mut CompactedCanwu) {
    let at = run.time();
    let forged = ResourceCommandV1 {
        subject: holder(),
        request: ResourceOperationRequestV1::CancelDemand(ResourceCancelDemandRequestV1 {
            operation_key: key("fixture:forged"),
            demand: ResourceDemandId::new("fixture:demand:forged").expect("demand"),
            expected_demand_revision: ResourceRevision::INITIAL,
        }),
    };
    run.enqueue_plugin_ingress(PluginIngressRequest::new(
        PLUGIN_NAME,
        RESOURCE_COMMAND_INGRESS,
        at,
        serde_json::json!({ "command": 1, "value": forged }),
    ))
    .expect("an untrusted resource command packet queues");
    // Each admitted command advances the revision the next one expects.
    let revision = run.revision();
    for (offset, (operation_key, name)) in (0_u64..).zip([
        ("fixture:create:third", "third"),
        ("fixture:create:third", "duplicate"),
        ("fixture:create:fifth", "fifth"),
    ]) {
        run.enqueue_command(
            at,
            0,
            create_account(3 + offset, revision + offset, at, operation_key, name),
        )
        .expect("tracked command ingress");
    }
    // The first boundary admits the commands and skips the forged packet;
    // the second settles the commands.
    for _ in 0..2 {
        run.settle_boundary(BoundaryRequest::at(at))
            .expect("a resource boundary never fails in this fixture");
    }
}

#[test]
fn skips_agree_across_a_seal_a_fork_a_restore_and_an_exact_replay() {
    let mut limits = ResourceLimitsV1::canonical();
    limits.max_archive_candidates = 3;
    let plugin = ResourcePlugin::default();
    let plugins: [&dyn canwu_api::SimulationPlugin; 1] = [&plugin];
    let scenario = Scenario::new(SimTime::EPOCH, vec![EntityRef::Person(PersonId::new(1))])
        .with_domain_records(vec![world(limits).into_record().expect("resource record")]);
    let mut base = Canwu::new_with_plugins(16, scenario, &plugins).expect("runtime");
    let revision = base.revision();
    for (offset, (operation_key, name)) in (0_u64..).zip([
        ("fixture:create:first", "first"),
        ("fixture:create:second", "second"),
    ]) {
        base.enqueue_command(
            SimTime::EPOCH,
            0,
            create_account(
                1 + offset,
                revision + offset,
                SimTime::EPOCH,
                operation_key,
                name,
            ),
        )
        .expect("tracked command ingress");
    }
    for _ in 0..3 {
        settle(&mut base);
    }
    assert!(skips(base.events()).is_empty());

    // Seal everything settled so far, and keep an unsealed fork and a run
    // restored from the sealed segments.
    let mut unsealed = base.fork().into_compacted().expect("compact interface");
    let mut sealed = base.into_compacted().expect("compact interface");
    let segment = sealed
        .seal_evidence()
        .expect("the settled history seals")
        .expect("the settled history holds evidence");
    let mut restored = CompactedCanwu::from_checkpoint_and_journal_with_plugins(
        sealed.checkpoint().expect("sealed checkpoint"),
        vec![segment.clone()],
        &plugins,
    )
    .expect("the sealed segments restore the run");

    for run in [&mut unsealed, &mut sealed, &mut restored] {
        after_seal(run);
    }
    assert_eq!(sealed.checkpoint_hash(), unsealed.checkpoint_hash());
    assert_eq!(restored.checkpoint_hash(), unsealed.checkpoint_hash());

    // Exact replay of the sealed run regenerates every boundary, skip events
    // included, and equals the unsealed fork.
    let journal = sealed
        .replay_journal_with_segments(vec![segment.clone()])
        .expect("the sealed run produces an exact replay journal");
    let replayed = Canwu::replay_from_journal(&plugins, &journal).expect("exact replay");
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
    let skipped = skips(replayed.events());
    assert_eq!(
        skipped
            .iter()
            .map(|skip| (skip.reason, skip.operation_key.clone()))
            .collect::<Vec<_>>(),
        [
            (
                ResourceIngressSkipReasonV1::FailedSettlementChecks,
                Some(key("fixture:forged"))
            ),
            (
                ResourceIngressSkipReasonV1::OperationKeyTaken,
                Some(key("fixture:create:third"))
            ),
            (
                ResourceIngressSkipReasonV1::ArchiveBacklog,
                Some(key("fixture:create:fifth"))
            ),
        ]
    );
    let root = resource_state(&replayed)
        .expect("resource query")
        .expect("resource state")
        .1;
    assert_eq!(
        root.accounts
            .keys()
            .filter(|id| !id.as_str().contains("opening"))
            .collect::<Vec<_>>(),
        [&account("first"), &account("second"), &account("third")]
    );
}

#[test]
fn an_expiry_without_room_is_skipped_and_schedules_no_tick_at_the_same_instant() {
    // A requested acquisition and a held grant, whose expiry is due at the
    // eighth boundary, in a root restored with a candidate budget that its
    // records fill.
    let mut resource = world(ResourceLimitsV1::canonical());
    let request = lease_request("expiring", SimTime::EPOCH);
    let acquisition = request.id.clone();
    resource
        .apply_operation(&ResourceOperationRequestV1::Completion(
            ResourceCompletionOperationV1::Acquire(request),
        ))
        .expect("acquire");
    let account_id = account("opening-0");
    resource
        .apply_operation(&ResourceOperationRequestV1::Completion(
            ResourceCompletionOperationV1::Grant(GrantCompletionCapacityV1 {
                grant_id: CompletionCapacityGrantId::new("fixture:grant:expiring").expect("grant"),
                acquisition: acquisition.clone(),
                expected_acquisition_revision: resource.completion_leases.acquisitions
                    [&acquisition]
                    .revision,
                owner_plugin: PLUGIN_NAME.to_owned(),
                target_versions: vec![CompletionLockedTargetV1::Account {
                    id: account_id.clone(),
                    revision: resource.accounts[&account_id].revision,
                }],
                current_boundary: 0,
            }),
        ))
        .expect("grant");
    assert!(!resource.completion_leases.expiry_due.is_empty());
    resource.limits.max_archive_candidates = resource.terminal_archive_candidates.len();
    resource.validate().expect("a full root is a valid root");
    let archive = MemoryArchive::default();
    let mut canwu = runtime(resource, &archive);
    while canwu.boundaries().len() < 8 {
        settle(&mut canwu);
    }

    // The expiry does not fit. It is skipped, the boundary commits, and the
    // tick schedules no successor: one at this instant would meet the same
    // root, and canonical advancement would never leave the instant.
    let at = canwu.time();
    let tick = enqueue_resource_completion_expiry_tick(&mut canwu, at).expect("tick");
    let full = state(&canwu);
    let events = canwu.events().len();
    settle(&mut canwu);
    let skipped = skips(&canwu.events()[events..]);
    assert_eq!(skipped.len(), 1);
    assert_eq!(skipped[0].ingress, tick.ingress_id);
    assert!(skipped[0].reason.is_capacity());
    assert_eq!(state(&canwu), full);
    assert!(canwu.step_canonical().expect("nothing is due").is_none());

    // The archive commit that makes room schedules a tick while the expiry is
    // due, so expiry resumes without the host application: the tick settles
    // at the same instant and expires the grant.
    queue_archive_commit(&mut canwu, &archive);
    settle(&mut canwu);
    let events = canwu.events().len();
    assert!(canwu.step_canonical().expect("the tick settles").is_some());
    assert!(skips(&canwu.events()[events..]).is_empty());
    assert!(state(&canwu).completion_leases.expiry_due.is_empty());
    // A host application may still queue a tick; with nothing due it changes
    // nothing.
    let quiet = state(&canwu);
    enqueue_resource_completion_expiry_tick(&mut canwu, at).expect("tick");
    settle(&mut canwu);
    assert_eq!(state(&canwu), quiet);
}

/// The production plugin's name: the resource plugin settles output batches
/// only from it. This fixture owns a production runtime record and, when a
/// host packet asks it to, schedules the output batch or the completion step
/// that packet carries, as the production plugin schedules a completed
/// execution's batch and its participant's completion.
const PRODUCTION: &str = "canwu-production";
const DISPATCH_OUTPUT: &str = "fixture_dispatch_output_v1";
const DISPATCH_COMPLETION: &str = "fixture_dispatch_completion_v1";

struct ProductionRecord;

impl canwu_api::DomainRecordType for ProductionRecord {
    type Payload = serde_json::Value;
    type Class = canwu_api::DomainValueKindClass;
    const NAMESPACE: &'static str = "canwu.production";
    const NAME: &'static str = "runtime";
}

struct ProductionFixture;

impl canwu_api::SimulationPlugin for ProductionFixture {
    fn name(&self) -> &'static str {
        PRODUCTION
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "00000000000000000000000000000000000000000000000000000000000000f7"
    }

    fn register(
        &self,
        registrar: &mut canwu_api::PluginRegistrar<'_>,
    ) -> Result<(), canwu_api::CanwuError> {
        let mut schema = canwu_api::DomainRecordSchema::for_record::<ProductionRecord>();
        schema.payload_schema = canwu_api::PayloadSchema::Any;
        registrar.register_record_schema(schema)?;
        for name in [
            DISPATCH_OUTPUT,
            DISPATCH_COMPLETION,
            "production_output_ack_v1",
            "production_completion_operation_v1",
        ] {
            registrar.register_ingress(canwu_api::PluginIngressDescriptor {
                name: name.to_owned(),
                description: "Production fixture ingress".to_owned(),
                class: canwu_api::IngressClass::ScheduledSystem,
                payload_schema: canwu_api::PayloadSchema::Any,
            })?;
        }
        let mut dispatch = canwu_api::BoundarySystemContract::new(
            "dispatch-fixture-work",
            canwu_api::BoundaryPhase::StrategicAggregation,
            canwu_api::SystemCadence::EventDriven,
        );
        dispatch.reads = vec![canwu_api::StateKey::core_ingress()];
        dispatch.plugin_ingress_targets = [
            RESOURCE_PRODUCTION_OUTPUT_BATCH_INGRESS,
            RESOURCE_COMPLETION_INGRESS,
        ]
        .map(|packet_type| canwu_api::PluginIngressTarget {
            target_plugin: PLUGIN_NAME.to_owned(),
            packet_type: packet_type.to_owned(),
        })
        .to_vec();
        dispatch.visibility = canwu_api::StateVisibility::SameBoundary;
        registrar.register_boundary_system(dispatch, dispatch_fixture_work)
    }
}

/// Schedules to the resource plugin the work each admitted dispatch packet
/// carries: a dispatch of output is an output batch, a dispatch of a
/// completion a completion step.
fn dispatch_fixture_work(
    view: &canwu_api::SimulationView<'_>,
    context: &canwu_api::BoundaryContext,
) -> Result<canwu_api::BoundaryProposal, canwu_api::CanwuError> {
    let mut directives = Vec::new();
    for ingress_id in &context.admitted_ingress {
        let Some(ingress) = view.ingress(*ingress_id)? else {
            continue;
        };
        let canwu_api::IngressPayload::Plugin {
            plugin,
            packet_type,
            payload,
            ..
        } = &ingress.payload
        else {
            continue;
        };
        let target = match packet_type.as_str() {
            DISPATCH_OUTPUT => RESOURCE_PRODUCTION_OUTPUT_BATCH_INGRESS,
            DISPATCH_COMPLETION => RESOURCE_COMPLETION_INGRESS,
            _ => continue,
        };
        if plugin == PRODUCTION {
            directives.push(canwu_api::BoundaryDirective::SchedulePluginIngress {
                target_plugin: PLUGIN_NAME.to_owned(),
                after: canwu_api::SimDuration::ZERO,
                packet_type: target.to_owned(),
                priority: 0,
                payload: payload.clone(),
                affected: Vec::new(),
            });
        }
    }
    Ok(canwu_api::BoundaryProposal {
        directives,
        ..canwu_api::BoundaryProposal::default()
    })
}

/// The production record version the participant and the batch cite.
fn production_source() -> canwu_api::DomainRecordVersionRef {
    canwu_api::DomainRecordVersionRef {
        record: canwu_api::DomainRecordRef::new("canwu.production", "runtime", "fixture"),
        version: 1,
        established_by: canwu_api::DomainRecordVersionSource::InitialScenario,
    }
}

fn output_account(index: usize) -> ResourceAccountId {
    account(&format!("output-{index}"))
}

/// A root whose one external participant, coordinated by the production
/// plugin, has consumed its grant for an execution that credits five units
/// to each of `outputs` accounts, and whose candidate budget its records
/// fill.
fn consumed_participant(outputs: usize) -> (ResourceState, CompletionLeaseActivationCertificateV1) {
    let (mut state, certificate) = production_participant(true, outputs);
    state.limits.max_archive_candidates = state.terminal_archive_candidates.len();
    state.validate().expect("a full root is a valid root");
    (state, certificate)
}

/// A root whose one external participant, coordinated by the production
/// plugin and locking `outputs` output accounts of ten units, is prepared,
/// and has consumed its grant when `consume` is set, with the certificate of
/// that consumption.
fn production_participant(
    consume: bool,
    outputs: usize,
) -> (ResourceState, CompletionLeaseActivationCertificateV1) {
    let mut state = world(ResourceLimitsV1::canonical());
    for index in 0..outputs {
        state
            .install_opening_account(new_account(&format!("output-{index}"), 10))
            .expect("output account");
    }
    let targets = (0..outputs)
        .map(|index| CompletionLockedTargetV1::Account {
            id: output_account(index),
            revision: ResourceRevision::INITIAL,
        })
        .chain(std::iter::once(CompletionLockedTargetV1::ExternalRecord {
            version: production_source(),
        }))
        .collect();
    let recipe = CompletionCapacityRecipeV1 {
        receipts: MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE,
        mutations: 1,
        reports_per_holder: 0,
        holders: 0,
        bytes: 1_024,
    };
    let certificate = external_participant(&mut state, "output", targets, &recipe, consume);
    (state, certificate)
}

/// Grants and prepares the external participant `name` of the production
/// plugin, locking `targets`, and consumes its grant when `consume` is set.
/// Returns the certificate of that consumption.
fn external_participant(
    state: &mut ResourceState,
    name: &str,
    targets: Vec<CompletionLockedTargetV1>,
    recipe: &CompletionCapacityRecipeV1,
    consume: bool,
) -> CompletionLeaseActivationCertificateV1 {
    let acquisition =
        CompletionLeaseAcquisitionId::new(format!("fixture:lease:{name}")).expect("id");
    let source = production_source();
    let envelope_digest = "e".repeat(64);
    let operation_key = key(&format!("fixture:execution:{name}"));
    state
        .apply_operation(&ResourceOperationRequestV1::Completion(
            ResourceCompletionOperationV1::GrantExternalParticipant(
                RequestExternalCompletionParticipantGrantV1 {
                    coordinator_plugin: PRODUCTION.to_owned(),
                    coordinator_source: source.clone(),
                    coordinator_acquisition_revision: ResourceRevision::INITIAL,
                    acquisition: acquisition.clone(),
                    operation_key: operation_key.clone(),
                    holder: holder(),
                    operation_namespace: NAMESPACE.to_owned(),
                    eligibility_time: SimTime::EPOCH,
                    eligibility_envelope_digest: envelope_digest.clone(),
                    recipe: recipe.clone(),
                    policy_class: CompletionPolicyClassV1::Guaranteed,
                    grant_id: CompletionCapacityGrantId::new(format!("fixture:grant:{name}"))
                        .expect("grant"),
                    target_versions: targets.clone(),
                    current_boundary: 1,
                },
            ),
        ))
        .expect("grant the participant");
    let held = state.external_completion_participants.grants[&acquisition]
        .grant
        .clone();
    state
        .apply_operation(&ResourceOperationRequestV1::Completion(
            ResourceCompletionOperationV1::PrepareExternalParticipant(
                PrepareExternalCompletionParticipantGrantV1 {
                    coordinator_source: source.clone(),
                    acquisition: acquisition.clone(),
                    expected_grant_revision: held.revision,
                    current_boundary: 2,
                    eligibility_envelope_digest: envelope_digest.clone(),
                },
            ),
        ))
        .expect("prepare the participant");
    let prepared = state.external_completion_participants.grants[&acquisition]
        .grant
        .clone();
    let mut certificate = CompletionLeaseActivationCertificateV1 {
        acquisition,
        acquisition_revision: ResourceRevision::INITIAL,
        operation_key,
        prepared_grants: vec![(prepared.id.clone(), prepared.revision)],
        locked_target_versions: targets,
        recipe_digest: recipe.digest().expect("recipe digest"),
        eligibility_time: SimTime::EPOCH,
        eligibility_envelope_digest: envelope_digest,
        activation_boundary: 3,
        semantic_digest: String::new(),
    };
    certificate.semantic_digest = canonical_digest(
        "canwu.resource.completion-activation-certificate.v1",
        &certificate,
    )
    .expect("certificate digest");
    if consume {
        state
            .apply_operation(&ResourceOperationRequestV1::Completion(
                ResourceCompletionOperationV1::ConsumeExternalParticipant(
                    ConsumeExternalCompletionParticipantGrantV1 {
                        coordinator_source: source,
                        certificate: certificate.clone(),
                        at: SimTime::EPOCH,
                    },
                ),
            ))
            .expect("consume the participant");
    }
    certificate
}

/// Completes in `state` a second external participant of the production
/// plugin whose holder's one report must publish before the participant's
/// record can become an archive candidate, and schedules that report to
/// publish at minute `publish_at`. Until then the participant holds one
/// candidate slot that no archive commit frees.
fn complete_participant_with_waiting_report(state: &mut ResourceState, publish_at: i64) {
    let report = ResourceReportGrantId::new("fixture:report:waiting").expect("report grant");
    state
        .install_report_grant(ResourceReportGrantV1 {
            id: report.clone(),
            holder: holder(),
            scope: ResourceScopeId::new("fixture:scope:market").expect("scope"),
            accounts: BTreeSet::new(),
            demands: BTreeSet::new(),
            include_transfer_details: false,
            confidence_per_mille: 1_000,
            cadence_minutes: 10_000,
            delay_minutes: 0,
        })
        .expect("report grant");
    let head = |revision: ResourceRevision, provider_state_revision: ResourceRevision| {
        ResourceObservationHeadV1 {
            id: ResourceObservationHeadId::new("fixture:observation:waiting").expect("head"),
            revision,
            provider_plugin: PRODUCTION.to_owned(),
            provider_version: "1".to_owned(),
            provider_semantic_hash: "f".repeat(64),
            provider_source: production_source(),
            holder: holder(),
            grant: report.clone(),
            provider_state_revision,
            observed_at: SimTime::EPOCH,
            confidence_per_mille: 1_000,
            stock: Vec::new(),
            demands: Vec::new(),
            allocations: Vec::new(),
            fulfillments: Vec::new(),
            transfers: Vec::new(),
            consumptions: Vec::new(),
            losses: Vec::new(),
            source_versions: vec![production_source()],
            semantic_digest: String::new(),
        }
        .seal()
        .expect("observation head")
    };
    state
        .record_observation_head(head(ResourceRevision::INITIAL, state.state_revision))
        .expect("record the observation");
    let certificate = external_participant(
        state,
        "reporting",
        vec![CompletionLockedTargetV1::Account {
            id: account("opening-0"),
            revision: ResourceRevision::INITIAL,
        }],
        &CompletionCapacityRecipeV1 {
            receipts: MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE,
            mutations: 1,
            reports_per_holder: 1,
            holders: 1,
            bytes: 1_024,
        },
        true,
    );
    state
        .apply_operation(&ResourceOperationRequestV1::Completion(
            ResourceCompletionOperationV1::CompleteExternalParticipant(
                CompleteExternalCompletionParticipantGrantV1 {
                    acquisition: certificate.acquisition,
                    operation_key: certificate.operation_key,
                },
            ),
        ))
        .expect("complete the participant");
    // An observation as fresh as the completion, so the report the
    // completion waits for can publish; it is due at `publish_at`.
    state
        .record_observation_head(head(
            ResourceRevision::INITIAL.next().expect("revision"),
            state.state_revision,
        ))
        .expect("record the fresh observation");
    state.report_dirty_grants.remove(&report);
    state
        .report_due_index
        .entry(SimTime::from_minutes(publish_at).as_minutes())
        .or_default()
        .insert(report);
}

/// The output batch of five units to each of `outputs` accounts.
fn output_batch(
    certificate: &CompletionLeaseActivationCertificateV1,
    at: SimTime,
    outputs: usize,
) -> ResourceProductionOutputBatchV1 {
    ResourceProductionOutputBatchV1 {
        provider_plugin: PRODUCTION.to_owned(),
        provider_source: production_source(),
        requests: (0..outputs)
            .map(|index| ResourceCreditRequestV1 {
                operation_key: key(&format!("fixture:output:{index}")),
                account: output_account(index),
                expected_account_revision: ResourceRevision::INITIAL,
                resource_revision: resource(),
                unit_revision: unit(),
                quantity: 5,
                source: ResourceCreditSourceV1::Production(production_source()),
                at,
                completion_certificate: certificate.clone(),
            })
            .collect(),
    }
}

/// The production record: one completed execution waiting for its output,
/// pinned to the record's own first version.
fn production_record(
    certificate: &CompletionLeaseActivationCertificateV1,
    outputs: usize,
) -> canwu_api::DomainRecord {
    let source = production_source();
    let output_requests: Vec<_> = (0..outputs)
        .map(|index| {
            serde_json::json!({
                "operation_key": format!("fixture:output:{index}"),
                "account": output_account(index),
                "expected_account_revision": ResourceRevision::INITIAL,
                "resource": resource(),
                "unit": unit(),
                "quantity": 5,
            })
        })
        .collect();
    canwu_api::DomainRecord {
        reference: source.record.clone(),
        owner: PRODUCTION.to_owned(),
        class: canwu_api::DomainRecordClass::Record,
        version: 1,
        lifecycle: canwu_api::DomainRecordLifecycle::Active,
        payload: serde_json::json!({
            "executions": {
                "fixture:execution": {
                    "id": "fixture:execution",
                    "lifecycle": "completed_pending_output_settlement",
                    "output_outcomes": [],
                    "completion_certificate": certificate,
                    "output_requests": output_requests,
                    "output_source": source,
                },
            },
        }),
        references: Vec::new(),
    }
}

/// A run of the production fixture and the resource plugin over `resource`
/// and the production record of `outputs` outputs.
fn production_run(
    resource: ResourceState,
    certificate: &CompletionLeaseActivationCertificateV1,
    outputs: usize,
    archive: &MemoryArchive,
) -> Canwu {
    let scenario = Scenario::new(SimTime::EPOCH, vec![EntityRef::Person(PersonId::new(1))])
        .with_domain_records(vec![
            production_record(certificate, outputs),
            resource.into_record().expect("resource record"),
        ]);
    let mut canwu = Canwu::new_with_plugins(
        18,
        scenario,
        &[&ProductionFixture, &ResourcePlugin::default()],
    )
    .expect("runtime");
    canwu.set_plugin_archive_object_provider(Rc::new(archive.clone()));
    canwu
}

/// Settles one boundary at `minute` and returns its skip events.
fn settle_at(canwu: &mut Canwu, minute: i64) -> Vec<ResourceIngressSkipV1> {
    let events = canwu.events().len();
    canwu
        .settle_boundary(BoundaryRequest::at(SimTime::from_minutes(minute)))
        .expect("a resource boundary never fails in this fixture");
    skips(&canwu.events()[events..])
}

/// The summaries of the skip events since `events`.
fn skip_summaries(canwu: &Canwu, events: usize) -> Vec<String> {
    canwu.events()[events..]
        .iter()
        .filter(|event| ResourceIngressSkipV1::from_event(event).is_some())
        .map(|event| {
            assert!(event.affected_entities.is_empty());
            event.summary.clone()
        })
        .collect()
}

fn dispatch(packet_type: &str, at: SimTime, payload: serde_json::Value) -> PluginIngressRequest {
    PluginIngressRequest::new(PRODUCTION, packet_type, at, payload)
}

fn dispatch_output(at: SimTime, batch: &ResourceProductionOutputBatchV1) -> PluginIngressRequest {
    dispatch(
        DISPATCH_OUTPUT,
        at,
        serde_json::to_value(batch).expect("batch payload"),
    )
}

fn balance(root: &ResourceState, index: usize) -> u64 {
    root.accounts[&output_account(index)].balance
}

#[test]
fn a_plugin_scheduled_output_batch_without_room_is_deferred_until_it_fits() {
    let (resource, certificate) = consumed_participant(1);
    let archive = MemoryArchive::default();
    let plugin = ResourcePlugin::default();
    let plugins: [&dyn canwu_api::SimulationPlugin; 2] = [&ProductionFixture, &plugin];
    let mut canwu = production_run(resource, &certificate, 1, &archive);
    let full = state(&canwu);

    // The production plugin schedules the batch of a completed execution.
    // Its credit does not fit, and the production plugin would never send
    // the batch again, so the resource plugin defers it a minute and names
    // the participant's holder in the event's detail. Nothing is credited.
    canwu
        .enqueue_plugin_ingress(dispatch_output(
            SimTime::EPOCH,
            &output_batch(&certificate, SimTime::EPOCH, 1),
        ))
        .expect("dispatch");
    assert!(settle_at(&mut canwu, 0).is_empty());
    let events = canwu.events().len();
    let deferred = settle_at(&mut canwu, 0);
    assert_eq!(deferred.len(), 1);
    assert_eq!(
        deferred[0].reason,
        ResourceIngressSkipReasonV1::ArchiveBacklog
    );
    assert_eq!(deferred[0].operation_key, Some(key("fixture:output:0")));
    assert_eq!(deferred[0].deferred_until, Some(SimTime::from_minutes(1)));
    assert!(skip_summaries(&canwu, events)[0].ends_with("the work belongs to holder person:1"));
    assert_eq!(state(&canwu), full);
    // While the work waits, the run seals: the waiting batch and the evidence
    // its acknowledgement reads stay pinned in the continuation closure.
    assert!(
        canwu
            .fork()
            .into_compacted()
            .expect("compact interface")
            .seal_evidence()
            .is_ok()
    );

    // Still no room a minute later: deferred again, two minutes this time.
    let again = settle_at(&mut canwu, 1);
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].deferred_until, Some(SimTime::from_minutes(3)));
    assert_eq!(state(&canwu), full);

    // An archive commit makes room, and the batch settles when it comes due,
    // retimed to that instant. The acknowledgement to the production plugin
    // carries that instant.
    queue_archive_commit(&mut canwu, &archive);
    assert!(settle_at(&mut canwu, 1).is_empty());
    assert!(settle_at(&mut canwu, 3).is_empty());
    let credited = state(&canwu);
    assert_eq!(balance(&credited, 0), 15);
    let outcome = &credited.outcomes[&key("fixture:output:0")];
    assert_eq!(outcome.status, ResourceOperationStatus::Applied);
    let acknowledgement = canwu
        .ingress_log()
        .iter()
        .find_map(|record| match &record.payload {
            canwu_api::IngressPayload::Plugin {
                plugin,
                packet_type,
                payload,
                ..
            } if plugin == PRODUCTION && packet_type == "production_output_ack_v1" => {
                Some(payload.clone())
            }
            _ => None,
        })
        .expect("the acknowledgement");
    assert_eq!(
        acknowledgement["settled_at"],
        serde_json::to_value(SimTime::from_minutes(3)).expect("time")
    );
    // The production plugin's acknowledgement is admitted.
    assert!(settle_at(&mut canwu, 3).is_empty());
    assert!(canwu.step_canonical().expect("nothing is due").is_none());

    // A seal in the middle: the sealed run, an unsealed fork, and an exact
    // replay of the sealed run agree as the batch is sent again, which its
    // recorded outcome answers.
    let mut unsealed = canwu.fork();
    unsealed.set_plugin_archive_object_provider(Rc::new(archive.clone()));
    let mut sealed = canwu.into_compacted().expect("compact interface");
    let segment = sealed
        .seal_evidence()
        .expect("the settled history seals")
        .expect("the settled history holds evidence");
    let resend = dispatch_output(
        SimTime::from_minutes(3),
        &output_batch(&certificate, SimTime::from_minutes(3), 1),
    );
    unsealed
        .enqueue_plugin_ingress(resend.clone())
        .expect("dispatch");
    sealed.enqueue_plugin_ingress(resend).expect("dispatch");
    for _ in 0..2 {
        assert!(settle_at(&mut unsealed, 3).is_empty());
        sealed
            .settle_boundary(BoundaryRequest::at(SimTime::from_minutes(3)))
            .expect("the sealed run settles");
    }
    assert_eq!(balance(&state(&unsealed), 0), 15);
    assert_eq!(sealed.checkpoint_hash(), unsealed.checkpoint_hash());
    let journal = sealed
        .replay_journal_with_segments(vec![segment.clone()])
        .expect("the sealed run produces an exact replay journal");
    let replayed =
        Canwu::replay_from_journal_with_archive_provider(&plugins, &journal, Rc::new(archive))
            .expect("exact replay");
    let reconstructed = sealed
        .snapshot_with_segments(vec![segment])
        .expect("the sealed run reconstructs a full snapshot");
    assert_eq!(replayed.snapshot(), reconstructed);
    assert_eq!(unsealed.snapshot(), reconstructed);
}

#[test]
fn a_plugin_scheduled_output_batch_that_can_never_fit_is_skipped_and_the_run_seals() {
    // A batch of as many credits as the candidate budget holds: even after
    // an archive commit, beside that commit's receipt, it cannot fit, and no
    // lease release helps work its own lease certifies.
    let outputs = production_participant(true, 1)
        .0
        .terminal_archive_candidates
        .len();
    let (resource, certificate) = consumed_participant(outputs);
    assert_eq!(resource.limits.max_archive_candidates, outputs);
    let archive = MemoryArchive::default();
    let mut canwu = production_run(resource, &certificate, outputs, &archive);
    queue_archive_commit(&mut canwu, &archive);
    assert!(settle_at(&mut canwu, 0).is_empty());
    let empty = state(&canwu);
    assert_eq!(empty.terminal_archive_candidates.len(), 1);

    // The batch is skipped at once, as room no commit can make. Before, each
    // credit was measured beside the earlier ones and every retry was told an
    // archive commit would help, so the batch was deferred without end and
    // the run could never seal again.
    canwu
        .enqueue_plugin_ingress(dispatch_output(
            SimTime::EPOCH,
            &output_batch(&certificate, SimTime::EPOCH, outputs),
        ))
        .expect("dispatch");
    assert!(settle_at(&mut canwu, 0).is_empty());
    let skipped = settle_at(&mut canwu, 0);
    assert_eq!(
        skipped,
        [ResourceIngressSkipV1 {
            ingress: skipped[0].ingress,
            reason: ResourceIngressSkipReasonV1::ReservedCapacity,
            operation_key: Some(key("fixture:output:0")),
            deferred_until: None,
        }]
    );
    assert_eq!(state(&canwu), empty);
    assert!(canwu.step_canonical().expect("nothing is due").is_none());
    // A later boundary admits the skip event, and nothing waits in the
    // queue, so the run seals.
    assert!(settle_at(&mut canwu, 1).is_empty());
    canwu
        .into_compacted()
        .expect("compact interface")
        .seal_evidence()
        .expect("the queue is empty, so the run seals")
        .expect("the settled history holds evidence");
}

#[test]
fn a_plugin_scheduled_output_batch_beside_waiting_reports_is_deferred_until_they_publish() {
    // The candidate budget holds the hot records, the slot of a finished
    // participant whose report waits, and nothing more. A batch with as many
    // credits as there are hot records fits only once the report publishes
    // and an archive commit clears the backlog.
    let build = |outputs| {
        let (mut resource, certificate) = production_participant(true, outputs);
        complete_participant_with_waiting_report(&mut resource, 1);
        (resource, certificate)
    };
    let outputs = build(1).0.terminal_archive_candidates.len();
    let (mut resource, certificate) = build(outputs);
    assert_eq!(resource.terminal_archive_candidates.len(), outputs);
    assert_eq!(resource.completion_report_ready.len(), 1);
    resource.limits.max_archive_candidates = outputs + 1;
    resource.validate().expect("a full root is a valid root");
    let archive = MemoryArchive::default();
    let mut canwu = production_run(resource, &certificate, outputs, &archive);
    let full = state(&canwu);

    // The shortage is reserved capacity, held by the waiting report's slot.
    // It is curable, so the batch is deferred. Before, room reserved for
    // certified work was taken for room that never returns: the batch was
    // skipped and its execution stayed pending for good.
    canwu
        .enqueue_plugin_ingress(dispatch_output(
            SimTime::EPOCH,
            &output_batch(&certificate, SimTime::EPOCH, outputs),
        ))
        .expect("dispatch");
    assert!(settle_at(&mut canwu, 0).is_empty());
    let deferred = settle_at(&mut canwu, 0);
    assert_eq!(
        deferred,
        [ResourceIngressSkipV1 {
            ingress: deferred[0].ingress,
            reason: ResourceIngressSkipReasonV1::ReservedCapacity,
            operation_key: Some(key("fixture:output:0")),
            deferred_until: Some(SimTime::from_minutes(1)),
        }]
    );
    assert_eq!(state(&canwu), full);

    // At minute 1 the batch still meets the waiting slot and is deferred
    // again; the report then publishes, and its participant's record becomes
    // an ordinary archive candidate.
    let again = settle_at(&mut canwu, 1);
    assert_eq!(again.len(), 1);
    assert_eq!(
        again[0].reason,
        ResourceIngressSkipReasonV1::ReservedCapacity
    );
    assert_eq!(again[0].deferred_until, Some(SimTime::from_minutes(3)));
    let published = state(&canwu);
    assert!(published.completion_report_ready.is_empty());
    assert_eq!(published.terminal_archive_candidates.len(), outputs + 1);

    // An archive commit clears the backlog, and the batch settles when it
    // comes due.
    queue_archive_commit(&mut canwu, &archive);
    assert!(settle_at(&mut canwu, 1).is_empty());
    assert!(settle_at(&mut canwu, 3).is_empty());
    let credited = state(&canwu);
    for index in 0..outputs {
        assert_eq!(balance(&credited, index), 15);
    }
    assert_eq!(
        credited.outcomes[&key("fixture:output:0")].status,
        ResourceOperationStatus::Applied
    );
}

#[test]
fn a_plugin_scheduled_output_batch_kept_out_by_the_byte_cap_waits_a_bounded_time() {
    // A root whose byte cap leaves operations no room for one more outcome.
    let (mut resource, certificate) = production_participant(true, 1);
    let bytes = serde_json::to_vec(&resource).expect("root bytes").len();
    let mut limits = resource.limits;
    limits.max_state_bytes = bytes;
    while limits.operation_state_bytes() < bytes {
        limits.max_state_bytes += 1;
    }
    resource.limits = limits;
    resource.validate().expect("the root fits its cap");
    let archive = MemoryArchive::default();
    let mut canwu = production_run(resource, &certificate, 1, &archive);
    let before = state(&canwu);
    canwu
        .enqueue_plugin_ingress(dispatch_output(
            SimTime::EPOCH,
            &output_batch(&certificate, SimTime::EPOCH, 1),
        ))
        .expect("dispatch");

    // The byte cap may never pass, so the batch waits only until its delay
    // has reached the 1,024-minute cap: eleven deferrals, then a skip. Before,
    // it was deferred without end and the run could never seal again.
    let mut trace = Vec::new();
    for _ in 0..16 {
        let events = canwu.events().len();
        if canwu.step_canonical().expect("no boundary fails").is_none() {
            break;
        }
        trace.extend(skips(&canwu.events()[events..]));
    }
    let mut due = 0;
    let mut expected: Vec<_> = (0..11)
        .map(|deferral| {
            due += 1_i64 << deferral;
            Some(SimTime::from_minutes(due))
        })
        .collect();
    expected.push(None);
    assert_eq!(
        trace
            .iter()
            .map(|skip| skip.deferred_until)
            .collect::<Vec<_>>(),
        expected
    );
    assert!(
        trace
            .iter()
            .all(|skip| skip.reason == ResourceIngressSkipReasonV1::RootLimit)
    );
    assert_eq!(due, 2_047);
    assert_eq!(state(&canwu).accounts, before.accounts);
    assert!(
        !state(&canwu)
            .outcomes
            .contains_key(&key("fixture:output:0"))
    );
    assert!(canwu.step_canonical().expect("nothing is due").is_none());
}

#[test]
fn a_plugin_scheduled_completion_step_without_room_is_deferred_until_it_fits() {
    let (resource, certificate) = consumed_participant(1);
    let archive = MemoryArchive::default();
    let mut canwu = production_run(resource, &certificate, 1, &archive);
    let complete = ResourceCompletionOperationV1::CompleteExternalParticipant(
        CompleteExternalCompletionParticipantGrantV1 {
            acquisition: certificate.acquisition.clone(),
            operation_key: certificate.operation_key.clone(),
        },
    );

    // The production plugin completes its resource participant. The step
    // does not fit, it can settle later, and an archive commit would make
    // room, so it is deferred a minute.
    canwu
        .enqueue_plugin_ingress(dispatch(
            DISPATCH_COMPLETION,
            SimTime::EPOCH,
            serde_json::to_value(&complete).expect("completion payload"),
        ))
        .expect("dispatch");
    assert!(settle_at(&mut canwu, 0).is_empty());
    let events = canwu.events().len();
    let deferred = settle_at(&mut canwu, 0);
    assert_eq!(deferred.len(), 1);
    assert_eq!(
        deferred[0].reason,
        ResourceIngressSkipReasonV1::ArchiveBacklog
    );
    assert_eq!(deferred[0].operation_key, Some(complete.operation_key()));
    assert_eq!(deferred[0].deferred_until, Some(SimTime::from_minutes(1)));
    assert!(skip_summaries(&canwu, events)[0].ends_with("the work belongs to holder person:1"));
    assert!(
        !state(&canwu)
            .outcomes
            .contains_key(&complete.operation_key())
    );

    // The commit makes room, and the step settles when it comes due; the
    // production plugin's acknowledgement of the completion follows.
    queue_archive_commit(&mut canwu, &archive);
    assert!(settle_at(&mut canwu, 0).is_empty());
    assert!(settle_at(&mut canwu, 1).is_empty());
    let completed = state(&canwu);
    assert_eq!(
        completed.outcomes[&complete.operation_key()].status,
        ResourceOperationStatus::Applied
    );
    assert!(canwu.ingress_log().iter().any(|record| matches!(
        &record.payload,
        canwu_api::IngressPayload::Plugin { plugin, packet_type, .. }
            if plugin == PRODUCTION && packet_type == "production_completion_operation_v1"
    )));
    assert!(settle_at(&mut canwu, 1).is_empty());
    assert!(canwu.step_canonical().expect("nothing is due").is_none());
}

#[test]
fn a_plugin_scheduled_completion_step_bound_to_its_boundary_is_skipped_not_deferred() {
    let (resource, certificate) = consumed_participant(1);
    let archive = MemoryArchive::default();
    let mut canwu = production_run(resource, &certificate, 1, &archive);
    let full = state(&canwu);
    let expire = ResourceCompletionOperationV1::Expire(ExpireCompletionCapacityV1 {
        at: SimTime::EPOCH,
        current_boundary: 1,
        candidate_limit: 1,
    });

    // The expiry must settle at the instant it names, so it could not settle
    // later as the same step. Although an archive commit would make room, it
    // is skipped, not deferred.
    canwu
        .enqueue_plugin_ingress(dispatch(
            DISPATCH_COMPLETION,
            SimTime::EPOCH,
            serde_json::to_value(&expire).expect("completion payload"),
        ))
        .expect("dispatch");
    assert!(settle_at(&mut canwu, 0).is_empty());
    let skipped = settle_at(&mut canwu, 0);
    assert_eq!(
        skipped,
        [ResourceIngressSkipV1 {
            ingress: skipped[0].ingress,
            reason: ResourceIngressSkipReasonV1::ArchiveBacklog,
            operation_key: Some(expire.operation_key()),
            deferred_until: None,
        }]
    );
    assert_eq!(state(&canwu), full);
    assert!(canwu.step_canonical().expect("nothing is due").is_none());
}

#[test]
fn a_refused_completion_step_sets_nothing_in_motion() {
    // The run has no production plugin, so an acknowledgement scheduled to
    // it would fail the boundary.
    let (resource, _) = production_participant(false, 1);
    let mut canwu = runtime(resource, &MemoryArchive::default());
    let at = canwu.time();
    // An unknown participant, and a known one whose grant is only prepared:
    // both completions are recorded rejections, and neither boundary fails
    // or tells the coordinator that the participant completed.
    for acquisition in ["fixture:lease:unknown", "fixture:lease:output"] {
        let complete = ResourceCompletionOperationV1::CompleteExternalParticipant(
            CompleteExternalCompletionParticipantGrantV1 {
                acquisition: CompletionLeaseAcquisitionId::new(acquisition).expect("id"),
                operation_key: key("fixture:execution:output"),
            },
        );
        enqueue_resource_completion_operation(&mut canwu, at, &complete).expect("completion");
        settle(&mut canwu);
        let outcome = &state(&canwu).outcomes[&complete.operation_key()];
        assert_eq!(
            outcome.status,
            ResourceOperationStatus::Rejected,
            "{acquisition}"
        );
    }
    assert!(!canwu.ingress_log().iter().any(|record| matches!(
        &record.payload,
        canwu_api::IngressPayload::Plugin { plugin, .. } if plugin == PRODUCTION
    )));
    assert_eq!(
        state(&canwu).external_completion_participants.grants
            [&CompletionLeaseAcquisitionId::new("fixture:lease:output").expect("id")]
            .grant
            .state,
        CompletionGrantStateV1::Prepared
    );
}

#[test]
fn a_result_past_the_byte_reserve_is_skipped_and_leaves_the_root_exact() {
    // A root whose byte cap leaves operations no room for one more outcome:
    // the operation share of the cap ends just above the root.
    let mut resource = world(ResourceLimitsV1::canonical());
    reject(&mut resource, 0);
    let bytes = serde_json::to_vec(&resource).expect("root bytes").len();
    let mut limits = resource.limits;
    limits.max_state_bytes = bytes;
    while limits.operation_state_bytes() < bytes {
        limits.max_state_bytes += 1;
    }
    assert!(limits.max_state_bytes - limits.operation_state_bytes() >= bytes / 4);
    resource.limits = limits;
    resource.validate().expect("the root fits its cap");
    let before = resource.clone();

    // A refusal and an applied operation that would pass the share record
    // nothing and leave the root exactly as it was: revision, sequence,
    // outcomes, candidates, and continuation.
    let refusal = resource.apply_operation(&ResourceOperationRequestV1::CancelDemand(
        ResourceCancelDemandRequestV1 {
            operation_key: key("fixture:cancel:bytes"),
            demand: ResourceDemandId::new("fixture:missing:bytes").expect("demand"),
            expected_demand_revision: ResourceRevision::INITIAL,
        },
    ));
    assert!(matches!(refusal, Err(ResourceError::LimitExceeded(_))));
    assert_eq!(resource, before);
    let applied = resource.apply_operation(&ResourceOperationRequestV1::CreateAccount(
        ResourceCreateAccountRequestV1 {
            operation_key: key("fixture:create:bytes"),
            account: new_account("bytes", 0),
        },
    ));
    assert!(matches!(applied, Err(ResourceError::LimitExceeded(_))));
    assert_eq!(resource, before);

    // The plugin skips such a packet as `root_limit`.
    let archive = MemoryArchive::default();
    let mut canwu = runtime(resource, &archive);
    let at = canwu.time();
    let pass = ResourceAllocationRequestV1 {
        operation_key: key("fixture:allocate:bytes"),
        expected_state_revision: state(&canwu).state_revision,
        at,
        candidate_limit: 1,
    };
    let queued = enqueue_resource_allocation(&mut canwu, at, &holder(), &pass).expect("enqueue");
    let events = canwu.events().len();
    settle(&mut canwu);
    assert_eq!(
        skips(&canwu.events()[events..]),
        [ResourceIngressSkipV1 {
            ingress: queued.ingress_id,
            reason: ResourceIngressSkipReasonV1::RootLimit,
            operation_key: Some(pass.operation_key.clone()),
            deferred_until: None,
        }]
    );
    assert_eq!(state(&canwu), before);
}
