//! Gap G-12: a transfer between two accounts that declare the same place
//! scope settles by local acceptance without a transport execution. A
//! different or undeclared scope, or a transfer that already has a transport
//! link, is rejected. The declared scope persists through save/load and replay.

#![allow(
    clippy::needless_pass_by_value,
    clippy::similar_names,
    clippy::too_many_lines
)]

use canwu_api::{
    BoundaryRequest, CanwuError, CommandEnvelope, CommandOutcome, CommandRequest, CommandRequestId,
    DomainRecord, DomainRecordClass, DomainRecordLifecycle, DomainRecordRef, DomainRecordSchema,
    DomainRecordType, DomainRecordVersionRef, DomainRecordVersionSource, DomainValueKindClass,
    EntityRef, ErrorCode, Issuer, KnowledgeHolderRef, PayloadSchema, PersonId, PluginRegistrar,
    Scenario, SimDuration, SimTime, SimulationPlugin,
};
use canwu_resource::*;
use std::collections::{BTreeMap, BTreeSet};

const NAMESPACE: &str = "test.resource";

fn digest(value: char) -> String {
    value.to_string().repeat(64)
}

fn holder(value: u64) -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(PersonId::new(value))
}

fn minute(value: i64) -> SimTime {
    SimTime::EPOCH + SimDuration::minutes(value)
}

fn key(value: &str) -> ResourceOperationKey {
    ResourceOperationKey::new(value).expect("operation key")
}

fn scope(value: &str) -> ResourceScopeId {
    ResourceScopeId::new(format!("test:place:{value}")).expect("place scope")
}

fn evidence(id: &str) -> DomainRecordVersionRef {
    DomainRecordVersionRef {
        record: DomainRecordRef::new("test.provider", "evidence", id),
        version: 1,
        established_by: DomainRecordVersionSource::InitialScenario,
    }
}

fn evidence_record(version: &DomainRecordVersionRef) -> DomainRecord {
    DomainRecord {
        reference: version.record.clone(),
        owner: "test-provider".to_owned(),
        class: DomainRecordClass::Record,
        version: version.version,
        lifecycle: DomainRecordLifecycle::Active,
        payload: serde_json::json!({ "evidence": version.record.id }),
        references: Vec::new(),
    }
}

/// Owner of the application evidence records cited by the fixture.
struct EvidenceRecord;

impl DomainRecordType for EvidenceRecord {
    type Payload = serde_json::Value;
    type Class = DomainValueKindClass;
    const NAMESPACE: &'static str = "test.provider";
    const NAME: &'static str = "evidence";
}

struct EvidencePlugin;

impl SimulationPlugin for EvidencePlugin {
    fn name(&self) -> &'static str {
        "test-provider"
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "00000000000000000000000000000000000000000000000000000000000000e1"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut schema = DomainRecordSchema::for_record::<EvidenceRecord>();
        schema.payload_schema = PayloadSchema::Any;
        registrar.register_record_schema(schema)
    }
}

fn applied(state: &mut ResourceState, request: ResourceOperationRequestV1) {
    let outcome = state.apply_operation(&request).expect("operation");
    assert_eq!(
        outcome.status,
        ResourceOperationStatus::Applied,
        "{:?}",
        outcome.rejection_reason
    );
}

fn partition(authority: KnowledgeHolderRef) -> CompletionCapacityPartitionV1 {
    CompletionCapacityPartitionV1 {
        authority,
        operation_namespace: NAMESPACE.to_owned(),
        guaranteed_units: 400_000,
        reserved_pending_slots: 4,
        maximum_burst_units: 100_000,
        request_token_capacity: 16,
        request_token_refill_minutes: 1,
        reacquire_cooldown_minutes: 1,
        root_acquisition_cap_per_sim_time: 4,
        guaranteed_max_wait_boundaries: 4,
    }
}

/// Activates one completion lease held by `lease_holder` for `operation_key`.
fn certificate(
    state: &mut ResourceState,
    lease_holder: u64,
    suffix: &str,
    operation_key: &ResourceOperationKey,
    targets: Vec<CompletionLockedTargetV1>,
    at: SimTime,
) -> CompletionLeaseActivationCertificateV1 {
    let acquisition =
        CompletionLeaseAcquisitionId::new(format!("test:lease:{suffix}")).expect("acquisition");
    let envelope = EligibilityEnvelopeV1::new(
        targets
            .iter()
            .filter_map(|target| match target {
                CompletionLockedTargetV1::ExternalRecord { version } => Some(version.clone()),
                _ => None,
            })
            .collect(),
        BTreeMap::new(),
        BTreeSet::new(),
        Vec::new(),
        Vec::new(),
    )
    .expect("envelope");
    applied(
        state,
        ResourceOperationRequestV1::Completion(ResourceCompletionOperationV1::Acquire(
            RequestCompletionLeaseV1 {
                id: acquisition.clone(),
                operation_key: operation_key.clone(),
                holder: holder(lease_holder),
                operation_namespace: NAMESPACE.to_owned(),
                eligibility_time: at,
                eligibility_envelope: envelope.clone(),
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
        )),
    );
    let grant = CompletionCapacityGrantId::new(format!("test:grant:{suffix}")).expect("grant");
    let revision = state.completion_leases.acquisitions[&acquisition].revision;
    applied(
        state,
        ResourceOperationRequestV1::Completion(ResourceCompletionOperationV1::Grant(
            GrantCompletionCapacityV1 {
                grant_id: grant.clone(),
                acquisition: acquisition.clone(),
                expected_acquisition_revision: revision,
                owner_plugin: PLUGIN_NAME.to_owned(),
                target_versions: targets,
                current_boundary: 10,
            },
        )),
    );
    for activate in [false, true] {
        let acquisition_revision = state.completion_leases.acquisitions[&acquisition].revision;
        let grant_revision = state.completion_leases.grants[&grant].revision;
        let operation = if activate {
            ResourceCompletionOperationV1::Activate(ActivateCompletionLeaseV1 {
                acquisition: acquisition.clone(),
                expected_acquisition_revision: acquisition_revision,
                grant: grant.clone(),
                expected_grant_revision: grant_revision,
                at,
                current_boundary: 12,
                eligibility_envelope_digest: envelope.digest.clone(),
            })
        } else {
            ResourceCompletionOperationV1::Prepare(PrepareCompletionCapacityV1 {
                acquisition: acquisition.clone(),
                expected_acquisition_revision: acquisition_revision,
                grant: grant.clone(),
                expected_grant_revision: grant_revision,
                current_boundary: 11,
                eligibility_envelope_digest: envelope.digest.clone(),
            })
        };
        applied(state, ResourceOperationRequestV1::Completion(operation));
    }
    state.completion_leases.certificates[&acquisition].clone()
}

struct World {
    state: ResourceState,
    source: ResourceAccountId,
    resource: ResourceDefinitionRevisionId,
    unit: ResourceUnitRevisionId,
}

fn world() -> World {
    let mut state = ResourceState::empty(ResourceLimitsV1::canonical()).expect("state");
    state
        .install_run_budget(
            RunBudgetRevisionV1 {
                revision: ResourceRevision::INITIAL,
                total_completion_units: 1_000_000,
                shared_pending_slots: 4,
                partitions: vec![partition(holder(1)), partition(holder(2))],
                semantic_digest: String::new(),
            }
            .seal()
            .expect("budget"),
        )
        .expect("install budget");
    let unit = ResourceUnitRevisionId::new("test:unit:measure:v1").expect("unit");
    state
        .install_unit(ResourceUnitRevision {
            id: unit.clone(),
            revision: ResourceRevision::INITIAL,
            symbol: "u".to_owned(),
            scale_numerator: 1,
            scale_denominator: 1,
            semantic_digest: digest('0'),
        })
        .expect("unit");
    let resource = ResourceDefinitionRevisionId::new("test:resource:stock:v1").expect("resource");
    state
        .install_definition(ResourceDefinitionRevision {
            id: resource.clone(),
            resource: ResourceDefinitionId::new("test:resource:stock").expect("definition"),
            revision: ResourceRevision::INITIAL,
            canonical_unit: unit.clone(),
            quality: ResourceQualityId::new("test:quality:standard").expect("quality"),
            scope: ResourceScopeId::new("test:scope:market").expect("scope"),
            effective_from: SimTime::EPOCH,
            effective_until: None,
            process_suitability: BTreeSet::new(),
            semantic_digest: digest('1'),
        })
        .expect("definition");
    let source = ResourceAccountId::new("test:account:source").expect("source");
    let mut value = World {
        state,
        source: source.clone(),
        resource,
        unit,
    };
    open_account(&mut value, &source, 1, 100, Some(scope("depot")));
    value
}

fn open_account(
    world: &mut World,
    id: &ResourceAccountId,
    custodian: u64,
    balance: u64,
    place_scope: Option<ResourceScopeId>,
) {
    world
        .state
        .install_opening_account(ResourceAccount {
            id: id.clone(),
            revision: ResourceRevision::INITIAL,
            custodian: holder(custodian),
            resource_revision: world.resource.clone(),
            unit_revision: world.unit.clone(),
            balance,
            capacity: None,
            protected_floor_policy: None,
            closed: false,
            place_scope,
        })
        .expect("account");
}

/// Opens a destination account in `place_scope` and a 10-unit demand on the
/// source account for it.
fn destination(world: &mut World, name: &str, place_scope: Option<ResourceScopeId>) {
    let destination = ResourceAccountId::new(format!("test:account:{name}")).expect("account");
    open_account(world, &destination, 2, 0, place_scope);
    let demand = ResourceDemandId::new(format!("test:demand:{name}")).expect("demand");
    world
        .state
        .install_demand(ResourceDemand {
            id: demand.clone(),
            revision: ResourceRevision::INITIAL,
            requester: holder(1),
            source_policy: ResourceDemandSourcePolicyV1::ExactAccounts(vec![world.source.clone()]),
            resource_revision: world.resource.clone(),
            unit_revision: world.unit.clone(),
            requested: 10,
            fulfilled: 0,
            minimum_useful: 10,
            partial_fulfillment: PartialFulfillmentPolicy::RejectPartial,
            alternative_group: None,
            due_at: SimTime::EPOCH,
            expires_at: SimTime::EPOCH + SimDuration::days(10),
            priority: 1,
            tie_break: ResourceTieBreakKey::new(format!("test:tie:{name}")).expect("tie"),
            admitted_sequence: 0,
            protected_floor_policy: None,
            protection_override_class: None,
            status: DemandStatus::Open,
            rejection_reason: None,
        })
        .expect("demand");
}

/// Starts the untransported transfer that consumes the named demand's leg.
fn start_transfer(
    world: &mut World,
    name: &str,
    at: SimTime,
) -> (ResourceTransferId, ResourceAccountId) {
    let destination = ResourceAccountId::new(format!("test:account:{name}")).expect("account");
    let demand = ResourceDemandId::new(format!("test:demand:{name}")).expect("demand");
    let leg = world
        .state
        .allocation_legs
        .values()
        .find(|leg| leg.demand == demand)
        .cloned()
        .expect("allocation");
    let account_revision = world.state.accounts[&world.source].revision;
    let start_key = key(&format!("test:start:{name}"));
    let lease = certificate(
        &mut world.state,
        1,
        &format!("start-{name}"),
        &start_key,
        vec![
            CompletionLockedTargetV1::Account {
                id: world.source.clone(),
                revision: account_revision,
            },
            CompletionLockedTargetV1::AllocationLeg {
                id: leg.id.clone(),
                revision: leg.revision,
            },
            CompletionLockedTargetV1::Demand {
                id: demand,
                revision: leg.demand_revision,
            },
        ],
        at,
    );
    let transfer = ResourceTransferId::new(format!("test:transfer:{name}")).expect("transfer");
    applied(
        &mut world.state,
        ResourceOperationRequestV1::BeginTransfer(ResourceTransferStartRequestV1 {
            operation_key: start_key,
            transfer_id: transfer.clone(),
            allocation: (&leg).into(),
            expected_account_revision: account_revision,
            destination: Some(destination.clone()),
            at,
            completion_certificate: lease,
        }),
    );
    (transfer, destination)
}

/// Builds a local-acceptance disposition with its terminal lease.
fn accept_local(
    world: &mut World,
    name: &str,
    transfer: &ResourceTransferId,
    destination: &ResourceAccountId,
    handover: &DomainRecordVersionRef,
    at: SimTime,
) -> ResourceOperationRequestV1 {
    let accept_key = key(&format!("test:accept-local:{name}"));
    let transfer_revision = world.state.transfers[transfer].revision;
    let destination_revision = world.state.accounts[destination].revision;
    let lease = certificate(
        &mut world.state,
        2,
        &format!("accept-{name}"),
        &accept_key,
        vec![
            CompletionLockedTargetV1::Transfer {
                id: transfer.clone(),
                revision: transfer_revision,
            },
            CompletionLockedTargetV1::Account {
                id: destination.clone(),
                revision: destination_revision,
            },
            CompletionLockedTargetV1::ExternalRecord {
                version: handover.clone(),
            },
        ],
        at,
    );
    ResourceOperationRequestV1::CompleteTransfer(ResourceTransferDispositionRequestV1 {
        operation_key: accept_key,
        transfer: transfer.clone(),
        expected_transfer_revision: transfer_revision,
        at,
        disposition: ResourceTransferDispositionV1::AcceptLocal {
            destination: destination.clone(),
            expected_destination_revision: destination_revision,
            handover_evidence: handover.clone(),
        },
        exact_transport_evidence: None,
        completion_certificate: lease,
    })
}

#[test]
fn gap_g12_resource_accept_local() {
    let mut world = world();
    let handovers: Vec<_> = ["far", "undeclared", "moving", "local"]
        .into_iter()
        .map(evidence)
        .collect();
    let transport = evidence("transport");
    destination(&mut world, "far", Some(scope("frontier")));
    destination(&mut world, "undeclared", None);
    destination(&mut world, "moving", Some(scope("depot")));
    destination(&mut world, "local", Some(scope("depot")));
    let expected_state_revision = world.state.state_revision;
    applied(
        &mut world.state,
        ResourceOperationRequestV1::Allocate(ResourceAllocationRequestV1 {
            operation_key: key("test:allocate:all"),
            expected_state_revision,
            at: SimTime::EPOCH,
            candidate_limit: 8,
        }),
    );
    let (far, far_account) = start_transfer(&mut world, "far", minute(1));
    let (undeclared, undeclared_account) = start_transfer(&mut world, "undeclared", minute(2));
    let (moving, moving_account) = start_transfer(&mut world, "moving", minute(3));
    let (local, local_account) = start_transfer(&mut world, "local", minute(4));
    let moving_revision = world.state.transfers[&moving].revision;
    applied(
        &mut world.state,
        ResourceOperationRequestV1::AdvanceTransfer(ResourceTransferProgressRequestV1 {
            operation_key: key("test:advance:moving"),
            transfer: moving.clone(),
            expected_transfer_revision: moving_revision,
            progress: TransferProgressV1::InTransit,
            transport: TransportExecutionLink {
                execution: canwu_api::TransportExecutionId(1),
                itinerary_revision: canwu_api::ItineraryRevisionId(1),
                leg_execution: None,
                handoff: None,
                capacity_booking: None,
            },
            transport_evidence: transport.clone(),
        }),
    );

    // Different, undeclared, or transported: rejected without moving stock.
    for (name, transfer, destination, handover, at, code) in [
        (
            "far",
            &far,
            &far_account,
            &handovers[0],
            minute(1),
            "invalid_definition",
        ),
        (
            "undeclared",
            &undeclared,
            &undeclared_account,
            &handovers[1],
            minute(2),
            "invalid_definition",
        ),
        (
            "moving",
            &moving,
            &moving_account,
            &handovers[2],
            minute(3),
            "invalid_lifecycle",
        ),
    ] {
        let request = accept_local(&mut world, name, transfer, destination, handover, at);
        let outcome = world
            .state
            .apply_operation(&request)
            .expect("durable rejection");
        assert_eq!(outcome.status, ResourceOperationStatus::Rejected, "{name}");
        assert_eq!(outcome.rejection_code.as_deref(), Some(code), "{name}");
        assert_eq!(world.state.accounts[destination].balance, 0);
        assert_eq!(world.state.transfers[transfer].escrow, 10);
    }

    // Shared scope and no transport: the destination custodian accepts locally
    // through a tracked command, and the result survives save/load and replay.
    let request = accept_local(
        &mut world,
        "local",
        &local,
        &local_account,
        &handovers[3],
        minute(4),
    );
    let command = ResourceCommandV1 {
        subject: holder(2),
        request,
    };
    let scenario = Scenario::new(
        minute(4),
        vec![
            EntityRef::Person(PersonId::new(1)),
            EntityRef::Person(PersonId::new(2)),
        ],
    )
    .with_domain_records(
        handovers
            .iter()
            .chain([&transport])
            .map(evidence_record)
            .chain([world.state.into_record().expect("resource record")])
            .collect(),
    );
    let resource_plugin = ResourcePlugin::default();
    let plugins: [&dyn SimulationPlugin; 2] = [&resource_plugin, &EvidencePlugin];
    let mut canwu =
        canwu_api::Canwu::new_with_plugins(12, scenario, &plugins).expect("resource runtime");
    canwu
        .enqueue_command(
            minute(4),
            0,
            CommandRequest::new(
                CommandRequestId::new(1),
                canwu.revision(),
                CommandEnvelope::new(
                    Issuer::Actor(PersonId::new(2)),
                    resource_command(&command).expect("command"),
                ),
            ),
        )
        .expect("tracked command ingress");
    for boundary in ["command admission", "acceptance settlement"] {
        canwu
            .settle_boundary(BoundaryRequest::at(minute(4)))
            .expect(boundary);
    }
    let (_, live) = resource_state(&canwu)
        .expect("query")
        .expect("resource state");
    let outcome = &live.outcomes[&key("test:accept-local:local")];
    assert_eq!(outcome.status, ResourceOperationStatus::Applied);
    assert_eq!(outcome.exact_evidence, vec![handovers[3].clone()]);
    let transfer = &live.transfers[&local];
    assert_eq!(transfer.state, ResourceTransferState::Accepted);
    assert_eq!((transfer.accepted, transfer.escrow), (10, 0));
    assert!(transfer.transport.is_none());
    assert_eq!(live.accounts[&local_account].balance, 10);
    assert_eq!(
        live.accounts[&local_account].place_scope,
        Some(scope("depot"))
    );
    live.validate().expect("accepted state");

    let restored = from_resource_snapshot_json(&canwu.snapshot_json().expect("snapshot"), &plugins)
        .expect("strict restore");
    assert_eq!(
        resource_state(&restored).expect("query").expect("state").1,
        live
    );
    let replayed =
        replay_resource_from_journal(&plugins, &canwu.replay_journal()).expect("exact replay");
    assert_eq!(
        resource_state(&replayed).expect("query").expect("state").1,
        live
    );

    // The place scope is host-declared: a tracked command cannot set it.
    let mut account = live.accounts[&local_account].clone();
    account.id = ResourceAccountId::new("test:account:self-declared").expect("account");
    account.revision = ResourceRevision::INITIAL;
    account.balance = 0;
    let command = ResourceCommandV1 {
        subject: holder(2),
        request: ResourceOperationRequestV1::CreateAccount(ResourceCreateAccountRequestV1 {
            operation_key: key("test:create:self-declared"),
            account,
        }),
    };
    let scenario = Scenario::new(minute(4), vec![EntityRef::Person(PersonId::new(2))])
        .with_domain_records(
            handovers
                .iter()
                .chain([&transport])
                .map(evidence_record)
                .chain([live.clone().into_record().expect("resource record")])
                .collect(),
        );
    let mut direct =
        canwu_api::Canwu::new_with_plugins(13, scenario, &plugins).expect("direct runtime");
    let outcome = direct
        .process_command(CommandRequest::new(
            CommandRequestId::new(1),
            direct.revision(),
            CommandEnvelope::new(
                Issuer::Actor(PersonId::new(2)),
                resource_command(&command).expect("command"),
            )
            .at_time(minute(4)),
        ))
        .expect("durable admission rejection");
    let CommandOutcome::Rejected { rejection } = outcome else {
        panic!("a tracked command declared a place scope: {outcome:?}");
    };
    assert_eq!(rejection.error.code, ErrorCode::InvalidAuthority);
}
