//! Public integration contract for an installed, non-reference resource consumer.
#![allow(clippy::too_many_lines)]
use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundaryRequest,
    BoundarySystemContract, Canwu, CanwuError, DomainRecord, DomainRecordClass, DomainRecordDraft,
    DomainRecordKind, DomainRecordLifecycle, DomainRecordMutation, DomainRecordSchema,
    DomainRecordType, DomainRecordVersionRef, DomainValueKindClass, EntityRef, ErrorCode,
    KnowledgeHolderRef, PayloadSchema, PersonId, PluginRegistrar, Scenario, SimDuration, SimTime,
    SimulationPlugin, SimulationView, StateVisibility, SystemCadence, TypedDomainRecordRef,
};
use canwu_resource::*;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

const CONSUMER: &str = "fixture-consumer";
fn digest(value: char) -> String {
    value.to_string().repeat(64)
}
fn holder(value: u64) -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(PersonId::new(value))
}
fn budget() -> RunBudgetRevisionV1 {
    RunBudgetRevisionV1 {
        revision: ResourceRevision::INITIAL,
        total_completion_units: 1_000_000,
        shared_pending_slots: 4,
        partitions: vec![1, 2]
            .into_iter()
            .map(|id| CompletionCapacityPartitionV1 {
                authority: holder(id),
                operation_namespace: "test.resource".to_owned(),
                guaranteed_units: 400_000,
                reserved_pending_slots: 4,
                maximum_burst_units: 100_000,
                request_token_capacity: 4,
                request_token_refill_minutes: 1,
                reacquire_cooldown_minutes: 1,
                root_acquisition_cap_per_sim_time: 4,
                guaranteed_max_wait_boundaries: 4,
            })
            .collect(),
        semantic_digest: String::new(),
    }
    .seal()
    .expect("budget")
}

struct Fixture {
    state: ResourceState,

    resource: ResourceDefinitionRevisionId,
    unit: ResourceUnitRevisionId,
    floor: ProtectedFloorPolicyRevisionId,
}

fn fixture(balance: u64, floor: u64) -> Fixture {
    let mut state = ResourceState::empty(ResourceLimitsV1::canonical()).expect("empty state");
    state.install_run_budget(budget()).expect("budget");
    let unit = ResourceUnitRevisionId::new("test:unit:kg:v1").expect("unit");
    state
        .install_unit(ResourceUnitRevision {
            id: unit.clone(),
            revision: ResourceRevision::INITIAL,
            symbol: "kg".to_owned(),
            scale_numerator: 1,
            scale_denominator: 1,
            semantic_digest: digest('0'),
        })
        .expect("unit");
    let resource = ResourceDefinitionRevisionId::new("test:resource:grain:v1").expect("resource");
    let scope = ResourceScopeId::new("test:scope:local").expect("scope");
    state
        .install_definition(ResourceDefinitionRevision {
            id: resource.clone(),
            resource: ResourceDefinitionId::new("test:resource:grain").expect("definition"),
            revision: ResourceRevision::INITIAL,
            canonical_unit: unit.clone(),
            quality: ResourceQualityId::new("test:quality:merchantable").expect("quality"),
            scope: scope.clone(),
            effective_from: SimTime::EPOCH,
            effective_until: None,
            process_suitability: BTreeSet::new(),
            semantic_digest: digest('1'),
        })
        .expect("definition");
    let floor_id = ProtectedFloorPolicyRevisionId::new("test:floor:seed:v1").expect("floor");
    state
        .install_protected_floor_policy(ProtectedFloorPolicyRevision {
            id: floor_id.clone(),
            revision: ResourceRevision::INITIAL,
            floor,
            override_classes: BTreeSet::from(["emergency".to_owned()]),
            semantic_digest: digest('2'),
        })
        .expect("floor policy");
    let account = ResourceAccountId::new("test:account:source").expect("account");
    let destination = ResourceAccountId::new("test:account:destination").expect("destination");
    for (id, custodian, opening) in [
        (account.clone(), holder(1), balance),
        (destination.clone(), holder(2), 0),
    ] {
        state
            .install_opening_account(ResourceAccount {
                id,
                revision: ResourceRevision::INITIAL,
                custodian,
                resource_revision: resource.clone(),
                unit_revision: unit.clone(),
                balance: opening,
                capacity: Some(1_000),
                protected_floor_policy: Some(floor_id.clone()),
                closed: false,
            })
            .expect("account");
    }
    Fixture {
        state,

        resource,
        unit,
        floor: floor_id,
    }
}

struct ConsumerRecord;
impl DomainRecordType for ConsumerRecord {
    type Payload = Value;
    type Class = DomainValueKindClass;
    const NAMESPACE: &'static str = "fixture.consumer";
    const NAME: &'static str = "authorization";
}
fn consumer_ref() -> TypedDomainRecordRef<ConsumerRecord> {
    TypedDomainRecordRef::new("consumer")
}
struct ConsumerPlugin;
impl SimulationPlugin for ConsumerPlugin {
    fn name(&self) -> &'static str {
        CONSUMER
    }
    fn version(&self) -> &'static str {
        "1"
    }
    fn semantic_hash(&self) -> &'static str {
        "00000000000000000000000000000000000000000000000000000000000000c1"
    }
    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut schema = DomainRecordSchema::for_record::<ConsumerRecord>();
        schema.payload_schema = PayloadSchema::Any;
        let own = schema.state_key();
        registrar.register_record_schema(schema)?;
        let mut contract = BoundarySystemContract::new(
            "publish-consumption-intent",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        contract.reads = vec![
            own.clone(),
            DomainRecordSchema::for_record::<ResourceRuntimeRecord>().state_key(),
        ];
        contract.writes = vec![own];
        contract.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(contract, publish_intent)
    }
}

// The fixture provider owns its authorization record and publishes through a
// canonical boundary mutation. Fault modes model a buggy provider; ResourcePlugin
// must still reject invalid typed authorization without changing conserved state.
fn publish_intent(
    view: &SimulationView<'_>,
    _: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let own = view
        .domain_record(&consumer_ref().into_untyped())?
        .expect("consumer record");
    if own.payload["published"] == true {
        return Ok(BoundaryProposal::default());
    }
    let resource = view
        .domain_record(&resource_runtime_reference().into_untyped())?
        .expect("resource record")
        .decode_payload::<ResourceRuntimeRecord>()?;
    let leg = resource
        .allocation_legs
        .values()
        .next()
        .expect("allocated demand");
    let mut intent = ResourceConsumptionIntentV1 {
        id: ResourceConsumptionIntentId::new("fixture:intent").expect("id"),
        provider_plugin: CONSUMER.to_owned(),
        demand: leg.demand.clone(),
        demand_revision: leg.demand_revision,
        allocation: leg.into(),
        account: leg.account.clone(),
        expected_account_revision: resource.accounts[&leg.account].revision,
        consumption_id: ResourceConsumptionId::new("fixture:consumption").expect("id"),
        operation_key: ResourceOperationKey::new("fixture:consume").expect("id"),
        quantity: leg.quantity,
        status: ResourceConsumptionIntentStatusV1::Authorized,
        semantic_digest: String::new(),
    }
    .seal()
    .expect("valid provider intent");
    let fault = own.payload["fault"].as_str().expect("fixture fault");
    match fault {
        "retired" => {
            intent.status = ResourceConsumptionIntentStatusV1::Retired;
            intent = intent.seal().expect("retired intent");
        }
        "digest" => intent.semantic_digest = digest('f'),
        "wrong-demand" => {
            intent.demand = ResourceDemandId::new("fixture:other-demand").expect("id");
            intent = intent.seal().expect("different demand");
        }
        _ => {}
    }
    let mut intents = BTreeMap::from([(intent.id.to_string(), intent.clone())]);
    if fault == "ambiguous" || fault == "oversized" {
        let count = if fault == "ambiguous" {
            2
        } else {
            resource.limits.max_operation_outcomes + 1
        };
        for index in 1..count {
            let mut other = intent.clone();
            other.id =
                ResourceConsumptionIntentId::new(format!("fixture:intent:{index}")).expect("id");
            other = other.seal().expect("second intent");
            intents.insert(other.id.to_string(), other);
        }
    }
    let payload = if fault == "missing" {
        json!({"published": true, "pending_consumption": intent})
    } else {
        json!({"published": true, "resource_consumption_intents": intents})
    };
    let record = DomainRecordDraft::from_typed(consumer_ref(), &payload)?;
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Update {
                record,
                expected_version: own.version,
            },
            summary: "Consumer publishes its exact resource authorization".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

fn read_resources(simulation: &Canwu) -> ResourceState {
    resource_state(simulation)
        .expect("resource query")
        .expect("resources")
        .1
}
fn settle(simulation: &mut Canwu) {
    simulation
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("canonical boundary");
}
fn completion(simulation: &mut Canwu, operation: &ResourceCompletionOperationV1) {
    enqueue_resource_completion_operation(simulation, SimTime::EPOCH, operation)
        .expect("lease ingress");
    settle(simulation);
}
fn resource_plugin() -> ResourcePlugin {
    ResourcePlugin::new([DomainRecordKind::for_type::<ConsumerRecord>()])
}

fn prepared(fault: &str) -> (Canwu, ResourceAdapterOperationV1, DomainRecordVersionRef) {
    let mut initial = fixture(100, 0);
    // Genesis supplies opening inventory and a due demand, not authorization or a lease.
    initial.state.limits.max_operation_outcomes = 32;
    initial
        .state
        .install_demand(ResourceDemand {
            id: ResourceDemandId::new("fixture:demand").expect("id"),
            revision: ResourceRevision::INITIAL,
            requester: holder(1),
            source_policy: ResourceDemandSourcePolicyV1::default(),
            resource_revision: initial.resource.clone(),
            unit_revision: initial.unit.clone(),
            requested: 10,
            fulfilled: 0,
            minimum_useful: 10,
            partial_fulfillment: PartialFulfillmentPolicy::RejectPartial,
            alternative_group: None,
            due_at: SimTime::EPOCH,
            expires_at: SimTime::EPOCH + SimDuration::days(10),
            priority: 10,
            tie_break: ResourceTieBreakKey::new("fixture:tie").expect("id"),
            admitted_sequence: 0,
            protected_floor_policy: Some(initial.floor.clone()),
            protection_override_class: None,
            status: DemandStatus::Open,
            rejection_reason: None,
        })
        .expect("demand");
    let scenario = Scenario::new(
        SimTime::EPOCH,
        vec![
            EntityRef::Person(PersonId::new(1)),
            EntityRef::Person(PersonId::new(2)),
        ],
    )
    .with_domain_records(vec![
        initial.state.into_record().expect("genesis resources"),
        DomainRecord {
            reference: consumer_ref().into_untyped(),
            owner: CONSUMER.to_owned(),
            class: DomainRecordClass::Record,
            version: 1,
            lifecycle: DomainRecordLifecycle::Active,
            payload: json!({"published": false, "fault": fault}),
            references: Vec::new(),
        },
    ]);
    let mut simulation =
        Canwu::new_with_plugins(31, scenario, &[&resource_plugin(), &ConsumerPlugin])
            .expect("installed consumer runtime");
    let original_source = simulation
        .current_domain_record_version(&consumer_ref().into_untyped())
        .expect("source query")
        .expect("initial source");
    let before = read_resources(&simulation);
    enqueue_resource_allocation(
        &mut simulation,
        SimTime::EPOCH,
        &holder(1),
        &ResourceAllocationRequestV1 {
            operation_key: ResourceOperationKey::new("fixture:allocate").expect("id"),
            expected_state_revision: before.state_revision,
            at: SimTime::EPOCH,
            candidate_limit: 32,
        },
    )
    .expect("allocation ingress");
    settle(&mut simulation);
    simulation
        .settle_boundary(BoundaryRequest {
            at: SimTime::EPOCH,
            cadences: vec![SystemCadence::Daily],
        })
        .expect("provider publishes authorization");
    let source = simulation
        .current_domain_record_version(&consumer_ref().into_untyped())
        .expect("source query")
        .expect("published source");
    assert!(source.version > original_source.version);
    let state = read_resources(&simulation);
    let leg = state
        .allocation_legs
        .values()
        .next()
        .expect("live allocation")
        .clone();
    let operation_key = ResourceOperationKey::new("fixture:consume").expect("id");
    let acquisition = CompletionLeaseAcquisitionId::new("fixture:lease").expect("id");
    let envelope = EligibilityEnvelopeV1::new(
        vec![source.clone()],
        BTreeMap::new(),
        BTreeSet::new(),
        Vec::new(),
        Vec::new(),
    )
    .expect("envelope");
    completion(
        &mut simulation,
        &ResourceCompletionOperationV1::Acquire(RequestCompletionLeaseV1 {
            id: acquisition.clone(),
            operation_key: operation_key.clone(),
            holder: holder(1),
            operation_namespace: "test.resource".to_owned(),
            eligibility_time: SimTime::EPOCH,
            eligibility_envelope: envelope.clone(),
            recipe: CompletionCapacityRecipeV1 {
                receipts: MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE,
                mutations: 2,
                reports_per_holder: 0,
                holders: 0,
                bytes: 2048,
            },
            expected_participants: BTreeSet::from([PLUGIN_NAME.to_owned(), CONSUMER.to_owned()]),
            policy_class: CompletionPolicyClassV1::Guaranteed,
        }),
    );
    let resource_targets = vec![
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
    ];
    let resource_grant = CompletionCapacityGrantId::new("fixture:grant:resource").expect("id");
    let provider_grant = CompletionCapacityGrantId::new("fixture:grant:provider").expect("id");
    for (grant, owner, targets) in [
        (resource_grant.clone(), PLUGIN_NAME, resource_targets),
        (
            provider_grant.clone(),
            CONSUMER,
            vec![CompletionLockedTargetV1::ExternalRecord {
                version: source.clone(),
            }],
        ),
    ] {
        let state = read_resources(&simulation);
        completion(
            &mut simulation,
            &ResourceCompletionOperationV1::Grant(GrantCompletionCapacityV1 {
                grant_id: grant,
                acquisition: acquisition.clone(),
                expected_acquisition_revision: state.completion_leases.acquisitions[&acquisition]
                    .revision,
                owner_plugin: owner.to_owned(),
                target_versions: targets,
                current_boundary: 0,
            }),
        );
    }
    let state = read_resources(&simulation);
    let mut expected_acquisition_revision =
        state.completion_leases.acquisitions[&acquisition].revision;
    // Prepare both participants in one cut, then activate before lease expiry.
    for grant in [&resource_grant, &provider_grant] {
        enqueue_resource_completion_operation(
            &mut simulation,
            SimTime::EPOCH,
            &ResourceCompletionOperationV1::Prepare(PrepareCompletionCapacityV1 {
                acquisition: acquisition.clone(),
                expected_acquisition_revision,
                grant: grant.clone(),
                expected_grant_revision: state.completion_leases.grants[grant].revision,
                current_boundary: 0,
                eligibility_envelope_digest: envelope.digest.clone(),
            }),
        )
        .expect("prepare participant ingress");
        expected_acquisition_revision = expected_acquisition_revision.next().expect("revision");
    }
    settle(&mut simulation);
    let state = read_resources(&simulation);
    completion(
        &mut simulation,
        &ResourceCompletionOperationV1::Activate(ActivateCompletionLeaseV1 {
            acquisition: acquisition.clone(),
            expected_acquisition_revision: state.completion_leases.acquisitions[&acquisition]
                .revision,
            grant: resource_grant.clone(),
            expected_grant_revision: state.completion_leases.grants[&resource_grant].revision,
            at: SimTime::EPOCH,
            current_boundary: 0,
            eligibility_envelope_digest: envelope.digest,
        }),
    );
    let state = read_resources(&simulation);
    let certificate = state
        .completion_leases
        .certificates
        .get(&acquisition)
        .unwrap_or_else(|| panic!("no certificate: {:?}", state.completion_leases))
        .clone();
    let packet = ResourceAdapterOperationV1 {
        provider_plugin: CONSUMER.to_owned(),
        provider_source: source.clone(),
        request: ResourceOperationRequestV1::Consume(ResourceConsumptionRequestV1 {
            operation_key,
            consumption_id: ResourceConsumptionId::new("fixture:consumption").expect("id"),
            allocation: (&leg).into(),
            expected_account_revision: state.accounts[&leg.account].revision,
            consumer_evidence: source,
            at: SimTime::EPOCH,
            completion_certificate: certificate,
        }),
    };
    (simulation, packet, original_source)
}

fn consume(simulation: &mut Canwu, packet: &ResourceAdapterOperationV1) {
    enqueue_resource_adapter_operation(simulation, SimTime::EPOCH, packet)
        .expect("consumer ingress");
    settle(simulation);
}

#[test]
fn installed_consumer_settles_once_and_survives_restore_and_exact_replay() {
    let (mut simulation, packet, _) = prepared("valid");
    let resource = resource_plugin();
    let plugins: [&dyn SimulationPlugin; 2] = [&resource, &ConsumerPlugin];
    let snapshot = simulation.snapshot_json().expect("prepared snapshot");
    let mut restored =
        from_resource_snapshot_json(&snapshot, &plugins).expect("restore prepared lease");
    consume(&mut simulation, &packet);
    consume(&mut restored, &packet);
    let after = read_resources(&simulation);
    let account = ResourceAccountId::new("test:account:source").expect("id");
    assert_eq!(after.accounts[&account].balance, 90);
    assert_eq!(after.consumptions.len(), 1);
    assert_eq!(after.conservation.admitted_consumption, 10);
    after.validate().expect("conservation and lease invariants");
    assert_eq!(after, read_resources(&restored));
    assert_eq!(
        simulation.boundary_head_hash(),
        restored.boundary_head_hash()
    );
    let replayed =
        replay_resource_from_journal(&plugins, &simulation.replay_journal()).expect("exact replay");
    assert_eq!(
        simulation.snapshot_json().expect("snapshot"),
        replayed.snapshot_json().expect("replay snapshot")
    );
    let terminal = from_resource_snapshot_json(
        &simulation.snapshot_json().expect("terminal snapshot"),
        &plugins,
    )
    .expect("terminal restore");
    assert_eq!(after, read_resources(&terminal));
    assert!(
        from_resource_snapshot_json(&snapshot, &[&ResourcePlugin::default(), &ConsumerPlugin])
            .is_err()
    );
    assert!(from_resource_snapshot_json(&snapshot, &[&resource]).is_err());
    // Canonical adapter retries revalidate authority; a completed lease is rejected.
    // Idempotency here means no second debit, not a successful duplicate receipt.
    enqueue_resource_adapter_operation(&mut simulation, SimTime::EPOCH, &packet)
        .expect("duplicate ingress");
    let duplicate = simulation
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect_err("completed lease rejects retry");
    assert_eq!(duplicate.code, ErrorCode::InvalidAuthority);
    assert_eq!(after, read_resources(&simulation));
}
#[test]
fn malformed_authorizations_and_forged_packets_roll_back_without_spending_lease() {
    for (fault, expected) in [
        ("retired", ErrorCode::InvalidAuthority),
        ("wrong-demand", ErrorCode::InvalidAuthority),
        ("ambiguous", ErrorCode::InvalidAuthority),
        ("digest", ErrorCode::InvalidDomainRecord),
        ("missing", ErrorCode::InvalidDomainRecord),
        ("oversized", ErrorCode::ValueOutOfRange),
    ] {
        let (mut simulation, packet, _) = prepared(fault);
        let before = read_resources(&simulation);
        enqueue_resource_adapter_operation(&mut simulation, SimTime::EPOCH, &packet)
            .expect("untrusted packet ingress");
        let error = simulation
            .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
            .expect_err(fault);
        assert_eq!(error.code, expected, "{fault}: {error:?}");
        assert_eq!(
            before,
            read_resources(&simulation),
            "{fault} spent resources or lease capacity"
        );
    }
    for fault in [
        "operation",
        "quantity",
        "account-revision",
        "consumption-id",
        "provider",
        "stale-source",
        "time",
    ] {
        let (mut simulation, mut packet, old_source) = prepared("valid");
        let ResourceOperationRequestV1::Consume(request) = &mut packet.request else {
            unreachable!()
        };
        match fault {
            "operation" => {
                request.operation_key =
                    ResourceOperationKey::new("fixture:forged-operation").expect("id");
            }
            "quantity" => request.allocation.quantity += 1,
            "account-revision" => {
                request.expected_account_revision =
                    request.expected_account_revision.next().expect("revision");
            }
            "consumption-id" => {
                request.consumption_id =
                    ResourceConsumptionId::new("fixture:forged-consumption").expect("id");
            }
            "provider" => packet.provider_plugin = "fixture-impostor".to_owned(),
            "stale-source" => {
                packet.provider_source = old_source.clone();
                request.consumer_evidence = old_source;
            }
            "time" => request.at = SimTime::EPOCH + SimDuration::minutes(1),
            _ => unreachable!(),
        }
        let before = read_resources(&simulation);
        enqueue_resource_adapter_operation(&mut simulation, SimTime::EPOCH, &packet)
            .expect("untrusted packet ingress");
        simulation
            .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
            .expect_err(fault);
        assert_eq!(
            before,
            read_resources(&simulation),
            "{fault} spent resources or lease capacity"
        );
    }
}
