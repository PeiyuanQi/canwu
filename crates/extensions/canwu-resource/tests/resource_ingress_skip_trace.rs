//! A resource packet that settles nothing leaves a trace of its own, on every
//! ingress that can skip, and never fails its boundary.
//!
//! An operation key belongs to the first request that settles under it. The
//! first fixture sends, on each ingress, a packet whose key a different
//! request already holds, in the root and then in the archive: a tracked
//! command admitted beside the request that takes its key, an allocation
//! pass, an adapter packet, a production output batch, and a completion
//! operation. Each settles nothing, its boundary commits, the outcome under
//! the key stays the first request's, and one skip event names the packet,
//! its key, and the reason, with the holder of a tracked command or an
//! allocation pass as its affected entity.
//!
//! The second fixture sends packets that a host application or a plugin can
//! queue but that fail their settlement checks: a reserved key on adapter
//! ingress and in a production output batch, a payload that does not decode,
//! a resource command packet that no command produced, a production output
//! batch that no production record authorizes, and a completion step for an
//! acquisition the root does not hold. Each is skipped with its reason and
//! the root does not change.

#![allow(clippy::too_many_lines)]

use canwu_api::{
    BoundaryRequest, Canwu, CommandAttemptOutcome, CommandEnvelope, CommandRequest,
    CommandRequestId, DomainRecordRef, DomainRecordVersionRef, DomainRecordVersionSource,
    EntityRef, IngressId, IngressPayload, Issuer, KnowledgeHolderRef, PersonId,
    PluginIngressRequest, Scenario, SimTime,
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

fn new_account(name: &str) -> ResourceAccount {
    ResourceAccount {
        id: account(name),
        revision: ResourceRevision::INITIAL,
        custodian: holder(),
        resource_revision: resource(),
        unit_revision: unit(),
        balance: 0,
        capacity: None,
        protected_floor_policy: None,
        closed: false,
        place_scope: None,
    }
}

/// A run whose root holds one custodian with a completion-capacity
/// partition and two accounts.
fn runtime(archive: &MemoryArchive) -> Canwu {
    let mut state = ResourceState::empty(ResourceLimitsV1::canonical()).expect("state");
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
    for name in ["left", "right"] {
        state
            .install_opening_account(new_account(name))
            .expect("account");
    }
    let scenario = Scenario::new(SimTime::EPOCH, vec![EntityRef::Person(PersonId::new(1))])
        .with_domain_records(vec![state.into_record().expect("resource record")]);
    let mut canwu =
        Canwu::new_with_plugins(17, scenario, &[&ResourcePlugin::default()]).expect("runtime");
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

/// The skip events emitted since `events`, each with its affected entities.
fn skips_since(canwu: &Canwu, events: usize) -> Vec<(ResourceIngressSkipV1, Vec<EntityRef>)> {
    canwu.events()[events..]
        .iter()
        .filter_map(|event| {
            ResourceIngressSkipV1::from_event(event)
                .map(|skip| (skip, event.affected_entities.clone()))
        })
        .collect()
}

/// Settles one boundary that admits the packets queued before it and returns
/// the skips it emitted, in ingress order. The boundary admits packets by
/// ingress class before ingress id.
fn settle_skips(canwu: &mut Canwu) -> Vec<(ResourceIngressSkipV1, Vec<EntityRef>)> {
    let events = canwu.events().len();
    settle(canwu);
    let mut skips = skips_since(canwu, events);
    skips.sort_by_key(|(skip, _)| skip.ingress);
    skips
}

fn skip(
    ingress: IngressId,
    reason: ResourceIngressSkipReasonV1,
    operation_key: Option<&str>,
) -> ResourceIngressSkipV1 {
    ResourceIngressSkipV1 {
        ingress,
        reason,
        operation_key: operation_key.map(key),
        deferred_until: None,
    }
}

fn allocation(
    canwu: &Canwu,
    operation_key: &str,
    candidate_limit: usize,
) -> ResourceAllocationRequestV1 {
    ResourceAllocationRequestV1 {
        operation_key: key(operation_key),
        expected_state_revision: state(canwu).state_revision,
        at: canwu.time(),
        candidate_limit,
    }
}

fn provider_source(namespace: &str, name: &str) -> DomainRecordVersionRef {
    DomainRecordVersionRef {
        record: DomainRecordRef::new(namespace, name, "fixture"),
        version: 1,
        established_by: DomainRecordVersionSource::InitialScenario,
    }
}

/// An adapter packet for a request under `operation_key`. Settlement checks
/// the key before the provider binding, which this packet does not have.
fn adapter_packet(operation_key: &str) -> ResourceAdapterOperationV1 {
    ResourceAdapterOperationV1 {
        provider_plugin: "fixture-provider".to_owned(),
        provider_source: provider_source("fixture.provider", "record"),
        request: ResourceOperationRequestV1::CancelDemand(ResourceCancelDemandRequestV1 {
            operation_key: key(operation_key),
            demand: ResourceDemandId::new("fixture:demand:adapter").expect("demand"),
            expected_demand_revision: ResourceRevision::INITIAL,
        }),
    }
}

fn queue_adapter(canwu: &mut Canwu, payload: serde_json::Value) -> IngressId {
    let at = canwu.time();
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            PLUGIN_NAME,
            RESOURCE_ADAPTER_INGRESS,
            at,
            payload,
        ))
        .expect("adapter ingress queues")
        .ingress_id
}

/// A production output batch of one credit under `operation_key`, citing a
/// production record that does not exist.
fn output_batch(operation_key: &str) -> ResourceProductionOutputBatchV1 {
    let source = provider_source("canwu.production", "runtime");
    ResourceProductionOutputBatchV1 {
        provider_plugin: "canwu-production".to_owned(),
        provider_source: source.clone(),
        requests: vec![ResourceCreditRequestV1 {
            operation_key: key(operation_key),
            account: account("left"),
            expected_account_revision: ResourceRevision::INITIAL,
            resource_revision: resource(),
            unit_revision: unit(),
            quantity: 1,
            source: ResourceCreditSourceV1::Production(source),
            at: SimTime::EPOCH,
            completion_certificate: CompletionLeaseActivationCertificateV1 {
                acquisition: CompletionLeaseAcquisitionId::new("fixture:lease:output")
                    .expect("acquisition"),
                acquisition_revision: ResourceRevision::INITIAL,
                operation_key: key(operation_key),
                prepared_grants: Vec::new(),
                locked_target_versions: Vec::new(),
                recipe_digest: "2".repeat(64),
                eligibility_time: SimTime::EPOCH,
                eligibility_envelope_digest: "3".repeat(64),
                activation_boundary: 1,
                semantic_digest: "4".repeat(64),
            },
        }],
    }
}

fn queue_output_batch(canwu: &mut Canwu, batch: &ResourceProductionOutputBatchV1) -> IngressId {
    let at = canwu.time();
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            PLUGIN_NAME,
            RESOURCE_PRODUCTION_OUTPUT_BATCH_INGRESS,
            at,
            serde_json::to_value(batch).expect("batch payload"),
        ))
        .expect("output batch ingress queues")
        .ingress_id
}

/// A single-step lease of the custodian on one account.
fn single_step(canwu: &Canwu, name: &str, target: &str) -> ResourceCompletionOperationV1 {
    let current = state(canwu);
    let id = account(target);
    ResourceCompletionOperationV1::SingleStepLease(SingleStepCompletionLeaseV1 {
        lease: RequestCompletionLeaseV1 {
            id: CompletionLeaseAcquisitionId::new(format!("fixture:lease:{name}"))
                .expect("acquisition"),
            operation_key: key(&format!("fixture:operation:{name}:{target}")),
            holder: holder(),
            operation_namespace: NAMESPACE.to_owned(),
            eligibility_time: canwu.time(),
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
        },
        grant_id: CompletionCapacityGrantId::new(format!("fixture:grant:{name}:{target}"))
            .expect("grant"),
        target_versions: vec![CompletionLockedTargetV1::Account {
            id: id.clone(),
            revision: current.accounts[&id].revision,
        }],
        current_boundary: canwu
            .boundaries()
            .last()
            .map_or(1, |boundary| boundary.id.get() + 1),
    })
}

fn create_account_command(
    canwu: &Canwu,
    request_id: u64,
    expected_revision: u64,
    operation_key: &str,
    name: &str,
) -> CommandRequest {
    let at = canwu.time();
    CommandRequest::new(
        CommandRequestId::new(request_id),
        expected_revision,
        CommandEnvelope::new(
            Issuer::Actor(PersonId::new(CUSTODIAN)),
            resource_command(&ResourceCommandV1 {
                subject: holder(),
                request: ResourceOperationRequestV1::CreateAccount(
                    ResourceCreateAccountRequestV1 {
                        operation_key: key(operation_key),
                        account: new_account(name),
                    },
                ),
            })
            .expect("command"),
        )
        .at_time(at),
    )
}

/// The resource command packet the kernel queued for `command`.
fn command_packet(canwu: &Canwu, command: canwu_api::CommandId) -> IngressId {
    canwu
        .ingress_log()
        .iter()
        .find(|record| {
            record.cause == Some(canwu_api::CauseRef::Command(command))
                && matches!(
                    &record.payload,
                    IngressPayload::Plugin { packet_type, .. } if packet_type == RESOURCE_COMMAND_INGRESS
                )
        })
        .expect("the command's packet")
        .id
}

/// The request digest under `operation_key`, which must stay the first
/// request's.
fn digest_under(canwu: &Canwu, archive: &MemoryArchive, operation_key: &str) -> String {
    let root = state(canwu);
    root.outcomes.get(&key(operation_key)).map_or_else(
        || {
            archived_resource_operation_outcome(&root, archive, &key(operation_key))
                .expect("archive lookup")
                .expect("the key's outcome")
                .request_digest
        },
        |outcome| outcome.request_digest.clone(),
    )
}

#[test]
fn a_packet_whose_operation_key_another_request_holds_is_named_on_every_ingress() {
    let archive = MemoryArchive::default();
    let mut canwu = runtime(&archive);
    let person = vec![EntityRef::Person(PersonId::new(CUSTODIAN))];

    // Two commands with one key are admitted in one boundary: the first to
    // settle takes the key, and the second settles nothing.
    let revision = canwu.revision();
    for (offset, name) in (0_u64..).zip(["first", "second"]) {
        let request = create_account_command(
            &canwu,
            1 + offset,
            revision + offset,
            "fixture:create",
            name,
        );
        let at = canwu.time();
        canwu
            .enqueue_command(at, 0, request)
            .expect("command ingress");
    }
    settle(&mut canwu);
    let commands: Vec<_> = canwu
        .command_attempts()
        .iter()
        .map(|attempt| match attempt.outcome {
            CommandAttemptOutcome::Accepted { command_id } => command_id,
            CommandAttemptOutcome::Rejected { .. } => panic!("both commands are admitted"),
        })
        .collect();
    assert_eq!(
        settle_skips(&mut canwu),
        [(
            skip(
                command_packet(&canwu, commands[1]),
                ResourceIngressSkipReasonV1::OperationKeyTaken,
                Some("fixture:create"),
            ),
            person.clone(),
        )]
    );
    assert!(state(&canwu).accounts.contains_key(&account("first")));
    assert!(!state(&canwu).accounts.contains_key(&account("second")));

    // An allocation pass takes a key, and the four other ingress kinds then
    // name it, with the root and then the archive holding its outcome.
    let first = allocation(&canwu, "fixture:shared", 1);
    let at = canwu.time();
    enqueue_resource_allocation(&mut canwu, at, &holder(), &first).expect("allocation");
    assert!(settle_skips(&mut canwu).is_empty());
    let owner = digest_under(&canwu, &archive, "fixture:shared");
    for archived in [false, true] {
        if archived {
            let hot = state(&canwu);
            let prepared = hot
                .prepare_resource_archive(hot.terminal_archive_candidates.len(), &archive)
                .expect("prepare archive commit");
            enqueue_resource_archive(&mut canwu, &prepared, &archive).expect("archive commit");
            settle(&mut canwu);
            assert!(!state(&canwu).outcomes.contains_key(&key("fixture:shared")));
        }
        let before = state(&canwu);
        let other = allocation(&canwu, "fixture:shared", 2);
        let allocation_ingress = enqueue_resource_allocation(&mut canwu, at, &holder(), &other)
            .expect("allocation")
            .ingress_id;
        let adapter_ingress = queue_adapter(
            &mut canwu,
            serde_json::to_value(adapter_packet("fixture:shared")).expect("adapter payload"),
        );
        let batch_ingress = queue_output_batch(&mut canwu, &output_batch("fixture:shared"));
        // The allocation pass names its requester.
        assert_eq!(
            settle_skips(&mut canwu),
            [
                (
                    skip(
                        allocation_ingress,
                        ResourceIngressSkipReasonV1::OperationKeyTaken,
                        Some("fixture:shared"),
                    ),
                    person.clone(),
                ),
                (
                    skip(
                        adapter_ingress,
                        ResourceIngressSkipReasonV1::OperationKeyTaken,
                        Some("fixture:shared"),
                    ),
                    Vec::new(),
                ),
                (
                    skip(
                        batch_ingress,
                        ResourceIngressSkipReasonV1::OperationKeyTaken,
                        Some("fixture:shared"),
                    ),
                    Vec::new(),
                ),
            ],
            "archived: {archived}"
        );
        assert_eq!(state(&canwu), before);
        assert_eq!(digest_under(&canwu, &archive, "fixture:shared"), owner);
    }

    // A second single step under the acquisition id of an active lease.
    let lease = single_step(&canwu, "held", "left");
    enqueue_resource_completion_operation(&mut canwu, at, &lease).expect("lease");
    assert!(settle_skips(&mut canwu).is_empty());
    let other = single_step(&canwu, "held", "right");
    let ingress = enqueue_resource_completion_operation(&mut canwu, at, &other)
        .expect("lease")
        .ingress_id;
    let before = state(&canwu);
    assert_eq!(
        settle_skips(&mut canwu),
        [(
            skip(
                ingress,
                ResourceIngressSkipReasonV1::OperationKeyTaken,
                Some(other.operation_key().as_str()),
            ),
            Vec::new(),
        )]
    );
    assert_eq!(state(&canwu), before);
}

#[test]
fn a_packet_that_fails_its_settlement_checks_is_named_and_changes_nothing() {
    let archive = MemoryArchive::default();
    let mut canwu = runtime(&archive);
    let before = state(&canwu);
    let at = canwu.time();
    let mut expected = Vec::new();

    // A reserved key on adapter ingress, queued around
    // `resource_adapter_ingress`, and in a production output batch.
    let reserved = adapter_packet("resource:completion:acquire:fixture:taken");
    expected.push(skip(
        queue_adapter(
            &mut canwu,
            serde_json::to_value(&reserved).expect("adapter payload"),
        ),
        ResourceIngressSkipReasonV1::ReservedOperationKey,
        Some("resource:completion:acquire:fixture:taken"),
    ));
    expected.push(skip(
        queue_output_batch(
            &mut canwu,
            &output_batch("resource:completion:acquire:fixture:credit"),
        ),
        ResourceIngressSkipReasonV1::ReservedOperationKey,
        Some("resource:completion:acquire:fixture:credit"),
    ));
    // A payload that does not decode names no key.
    expected.push(skip(
        queue_adapter(&mut canwu, serde_json::json!({ "provider_plugin": 7 })),
        ResourceIngressSkipReasonV1::FailedSettlementChecks,
        None,
    ));
    // A resource command packet that no command produced.
    let forged = ResourceCommandV1 {
        subject: holder(),
        request: ResourceOperationRequestV1::CreateAccount(ResourceCreateAccountRequestV1 {
            operation_key: key("fixture:forged"),
            account: new_account("forged"),
        }),
    };
    expected.push(skip(
        canwu
            .enqueue_plugin_ingress(PluginIngressRequest::new(
                PLUGIN_NAME,
                RESOURCE_COMMAND_INGRESS,
                at,
                serde_json::json!({ "command": 1, "value": forged }),
            ))
            .expect("resource command packet queues")
            .ingress_id,
        ResourceIngressSkipReasonV1::FailedSettlementChecks,
        Some("fixture:forged"),
    ));
    // A production output batch that no production record authorizes.
    expected.push(skip(
        queue_output_batch(&mut canwu, &output_batch("fixture:credit")),
        ResourceIngressSkipReasonV1::FailedSettlementChecks,
        Some("fixture:credit"),
    ));
    // A completion step for an acquisition the root does not hold.
    let prepare = ResourceCompletionOperationV1::Prepare(PrepareCompletionCapacityV1 {
        acquisition: CompletionLeaseAcquisitionId::new("fixture:lease:missing").expect("id"),
        expected_acquisition_revision: ResourceRevision::INITIAL,
        grant: CompletionCapacityGrantId::new("fixture:grant:missing").expect("grant"),
        expected_grant_revision: ResourceRevision::INITIAL,
        current_boundary: 1,
        eligibility_envelope_digest: "5".repeat(64),
    });
    expected.push(skip(
        enqueue_resource_completion_operation(&mut canwu, at, &prepare)
            .expect("completion ingress")
            .ingress_id,
        ResourceIngressSkipReasonV1::FailedSettlementChecks,
        Some(prepare.operation_key().as_str()),
    ));

    let skipped = settle_skips(&mut canwu);
    assert_eq!(
        skipped
            .iter()
            .map(|(skip, _)| skip.clone())
            .collect::<Vec<_>>(),
        expected
    );
    assert!(skipped.iter().all(|(_, affected)| affected.is_empty()));
    assert_eq!(state(&canwu), before);

    // An allocation pass for a requester that is not an entity of the run is
    // refused when it is queued: its events would name a missing entity.
    let stranger = KnowledgeHolderRef::Person(PersonId::new(99));
    let request = allocation(&canwu, "fixture:allocate:stranger", 1);
    let error = enqueue_resource_allocation(&mut canwu, at, &stranger, &request)
        .expect_err("an unknown requester is refused");
    assert_eq!(error.code, canwu_api::ErrorCode::EntityNotFound);
}

fn resource_version(canwu: &Canwu) -> u64 {
    resource_state(canwu)
        .expect("resource query")
        .expect("resource state")
        .0
        .version
}

/// The resource events, settled or skipped, of the last boundary.
fn resource_events_of_last_boundary(canwu: &Canwu) -> usize {
    let boundary = canwu.boundaries().last().expect("a boundary");
    boundary
        .emissions
        .iter()
        .filter(|emission| emission.plugin == PLUGIN_NAME)
        .count()
}

#[test]
fn a_resend_already_recorded_is_not_new_work() {
    let archive = MemoryArchive::default();
    let mut canwu = runtime(&archive);
    let at = canwu.time();

    // A tracked command, and the same request again as a second command.
    // The second is admitted, because its outcome is its own, and its packet
    // settles nothing: the root is not written again and no event names it.
    for request_id in [1, 2] {
        let revision = canwu.revision();
        let request =
            create_account_command(&canwu, request_id, revision, "fixture:create:once", "once");
        canwu
            .enqueue_command(at, 0, request)
            .expect("command ingress");
        settle(&mut canwu);
        let before = resource_version(&canwu);
        settle(&mut canwu);
        if request_id == 1 {
            assert_eq!(resource_version(&canwu), before + 1);
        } else {
            assert_eq!(resource_version(&canwu), before);
            assert_eq!(resource_events_of_last_boundary(&canwu), 0);
        }
    }

    // A grant on the completion ingress starts an expiry clock. The same
    // grant sent again settles nothing: the root is not written again, and
    // the boundary schedules only the clock's own tick, not another.
    let ResourceCompletionOperationV1::SingleStepLease(lease) =
        single_step(&canwu, "four-step", "left")
    else {
        unreachable!("single_step builds a single-step lease")
    };
    let acquisition = lease.lease.id.clone();
    enqueue_resource_completion_operation(
        &mut canwu,
        at,
        &ResourceCompletionOperationV1::Acquire(lease.lease.clone()),
    )
    .expect("acquire");
    settle(&mut canwu);
    let current = state(&canwu);
    let grant = ResourceCompletionOperationV1::Grant(GrantCompletionCapacityV1 {
        grant_id: lease.grant_id.clone(),
        acquisition: acquisition.clone(),
        expected_acquisition_revision: current.completion_leases.acquisitions[&acquisition]
            .revision,
        owner_plugin: PLUGIN_NAME.to_owned(),
        target_versions: lease.target_versions.clone(),
        current_boundary: canwu
            .boundaries()
            .last()
            .map_or(1, |boundary| boundary.id.get() + 1),
    });
    enqueue_resource_completion_operation(&mut canwu, at, &grant).expect("grant");
    settle(&mut canwu);
    assert!(state(&canwu).outcomes.contains_key(&grant.operation_key()));
    let before = resource_version(&canwu);
    enqueue_resource_completion_operation(&mut canwu, at, &grant).expect("grant again");
    settle(&mut canwu);
    assert_eq!(resource_version(&canwu), before);
    let boundary = canwu.boundaries().last().expect("the resend boundary");
    assert_eq!(boundary.generated_ingress.len(), 1);
    assert_eq!(resource_events_of_last_boundary(&canwu), 0);
}
