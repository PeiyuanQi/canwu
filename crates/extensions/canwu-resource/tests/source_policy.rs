//! Durable public contract for demand source selection and custody.
//! Quantities and minute offsets are test units, not historical estimates.
use canwu_api::{
    ArtifactManifest, Canwu, CommandAttemptOutcome, CommandAuthority, CommandEnvelope,
    CommandRequest, CommandRequestId, DecisionOrigin, EntityRef, ErrorCode, Issuer,
    KnowledgeHolderRef, PersonId, RunConfiguration, RunManifest, Scenario, SimTime,
};
use canwu_resource::*;
use std::collections::{BTreeMap, BTreeSet};

const REQUESTER: PersonId = PersonId::new(1);
const OTHER_CUSTODIAN: PersonId = PersonId::new(2);
const SEAT: &str = "fixture:seat:requester";
const CONTROLLER: &str = "fixture:controller:player";
const PERMISSION: &str = "fixture:permission:coordination-budget";
const FIRST: &str = "fixture:account:a-source";
const LAST: &str = "fixture:account:z-source";
const UNIT: &str = "fixture:unit:budget-credit:v1";
const RESOURCE: &str = "fixture:resource:budget-credit:v1";
const RELIEF: &str = "fixture:obligation:delivery:1";

fn holder() -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(REQUESTER)
}

fn state(simulation: &Canwu) -> ResourceState {
    resource_state(simulation)
        .expect("public resource query")
        .expect("resource installed")
        .1
}

fn scenario(requester_first: bool) -> Scenario {
    let mut resource = ResourceState::empty(ResourceLimitsV1::canonical()).expect("empty resource");
    resource
        .install_unit(ResourceUnitRevision {
            id: ResourceUnitRevisionId::new(UNIT).unwrap(),
            revision: ResourceRevision::INITIAL,
            symbol: "budget-credit".into(),
            scale_numerator: 1,
            scale_denominator: 1,
            semantic_digest: "0".repeat(64),
        })
        .unwrap();
    resource
        .install_definition(ResourceDefinitionRevision {
            id: ResourceDefinitionRevisionId::new(RESOURCE).unwrap(),
            resource: ResourceDefinitionId::new("fixture:resource:budget-credit").unwrap(),
            revision: ResourceRevision::INITIAL,
            canonical_unit: ResourceUnitRevisionId::new(UNIT).unwrap(),
            quality: ResourceQualityId::new("fixture:quality:budget-credit").unwrap(),
            scope: ResourceScopeId::new("fixture:scope:shared-store").unwrap(),
            effective_from: SimTime::EPOCH,
            effective_until: None,
            process_suitability: BTreeSet::new(),
            semantic_digest: "1".repeat(64),
        })
        .unwrap();
    resource
        .install_run_budget(
            RunBudgetRevisionV1 {
                revision: ResourceRevision::INITIAL,
                total_completion_units: 1_000_000,
                shared_pending_slots: 4,
                partitions: vec![CompletionCapacityPartitionV1 {
                    authority: holder(),
                    operation_namespace: "fixture.transfer".into(),
                    guaranteed_units: 400_000,
                    reserved_pending_slots: 4,
                    maximum_burst_units: 100_000,
                    request_token_capacity: 4,
                    request_token_refill_minutes: 1,
                    reacquire_cooldown_minutes: 1,
                    root_acquisition_cap_per_sim_time: 4,
                    guaranteed_max_wait_boundaries: 4,
                }],
                semantic_digest: String::new(),
            }
            .seal()
            .unwrap(),
        )
        .unwrap();
    for (id, actor) in [
        (
            FIRST,
            if requester_first {
                REQUESTER
            } else {
                OTHER_CUSTODIAN
            },
        ),
        (
            LAST,
            if requester_first {
                OTHER_CUSTODIAN
            } else {
                REQUESTER
            },
        ),
    ] {
        resource
            .install_opening_account(ResourceAccount {
                id: ResourceAccountId::new(id).unwrap(),
                revision: ResourceRevision::INITIAL,
                custodian: KnowledgeHolderRef::Person(actor),
                resource_revision: ResourceDefinitionRevisionId::new(RESOURCE).unwrap(),
                unit_revision: ResourceUnitRevisionId::new(UNIT).unwrap(),
                balance: 100,
                capacity: None,
                protected_floor_policy: None,
                closed: false,
            })
            .unwrap();
    }
    assert!(resource.demands.is_empty(), "no preseeded demands");
    Scenario::new(
        SimTime::from_minutes(31),
        vec![
            EntityRef::Person(REQUESTER),
            EntityRef::Person(OTHER_CUSTODIAN),
        ],
    )
    .with_domain_records(vec![resource.into_record().unwrap()])
}

fn new_simulation(plugin: &ResourcePlugin, requester_first: bool) -> Canwu {
    let scenario = scenario(requester_first);
    let configuration =
        RunConfiguration::play_as_character(SEAT, CONTROLLER, REQUESTER, PERMISSION);
    let manifest = RunManifest::declared(
        ArtifactManifest::for_scenario("fixture", "live-obligation-resources", "1", &scenario)
            .unwrap(),
        ArtifactManifest::for_run_configuration("fixture", "requester-seat", "1", &configuration)
            .unwrap(),
    );
    Canwu::new_with_run_configuration_and_plugins(49, scenario, manifest, configuration, &[plugin])
        .unwrap()
}

fn demand(id: &str, priority: i32, expiry: i64) -> ResourceDemand {
    ResourceDemand {
        id: ResourceDemandId::new(id).unwrap(),
        revision: ResourceRevision::INITIAL,
        requester: holder(),
        source_policy: ResourceDemandSourcePolicyV1::Pooled,
        resource_revision: ResourceDefinitionRevisionId::new(RESOURCE).unwrap(),
        unit_revision: ResourceUnitRevisionId::new(UNIT).unwrap(),
        requested: 60,
        fulfilled: 0,
        minimum_useful: 10,
        partial_fulfillment: PartialFulfillmentPolicy::AcceptPartial,
        alternative_group: None,
        due_at: SimTime::from_minutes(149),
        expires_at: SimTime::from_minutes(expiry),
        priority,
        tie_break: ResourceTieBreakKey::new(id).unwrap(),
        admitted_sequence: 0,
        protected_floor_policy: None,
        protection_override_class: None,
        status: DemandStatus::Open,
        rejection_reason: None,
    }
}

fn envelope(
    subject: KnowledgeHolderRef,
    request: ResourceOperationRequestV1,
    at: i64,
) -> CommandEnvelope {
    CommandEnvelope::new(
        Issuer::Human(CONTROLLER.into()),
        resource_command(&ResourceCommandV1 { subject, request }).unwrap(),
    )
    .with_authority(CommandAuthority {
        decision_origin: DecisionOrigin::Actor { actor: REQUESTER },
        seat_id: Some(SEAT.into()),
        permission_profile_id: Some(PERMISSION.into()),
        command_subject: Some(EntityRef::Person(REQUESTER)),
    })
    .at_time(SimTime::from_minutes(at))
}

fn submit_demand(simulation: &mut Canwu, request_id: u64, demand: ResourceDemand) {
    let demand_id = demand.id.clone();
    let request = ResourceOperationRequestV1::SubmitDemand(ResourceSubmitDemandRequestV1 {
        operation_key: ResourceOperationKey::new(format!("fixture:submit:{}", demand.id.as_str()))
            .unwrap(),
        demand,
    });
    let at = SimTime::from_minutes(137);
    simulation
        .enqueue_command(
            at,
            0,
            CommandRequest::new(
                CommandRequestId::new(request_id),
                simulation.revision(),
                envelope(holder(), request, at.as_minutes()),
            ),
        )
        .expect("enqueue human-bound demand after startup");
    simulation
        .step_canonical()
        .expect("settle demand admission")
        .expect("scheduled command");
    assert!(
        !state(simulation).demands.contains_key(&demand_id),
        "admission is not resource settlement"
    );
    {
        // ResourcePlugin command acceptance only records the command; its event-driven
        // lifecycle is a separate boundary. Drive that exact non-monthly boundary below.
        simulation
            .step_canonical()
            .expect("settle resource operation")
            .expect("resource boundary");
    }
    assert!(
        state(simulation).demands.contains_key(&demand_id),
        "missing demand; attempts={:?}",
        simulation.command_attempts()
    );
    assert_eq!(simulation.time(), at);
}

fn allocate(simulation: &mut Canwu) {
    let at = SimTime::from_minutes(149);
    enqueue_resource_allocation(
        simulation,
        at,
        &holder(),
        &ResourceAllocationRequestV1 {
            operation_key: ResourceOperationKey::new("fixture:allocate:137-orders").unwrap(),
            expected_state_revision: state(simulation).state_revision,
            at,
            candidate_limit: 2,
        },
    )
    .expect("public requester-scoped allocation");
    simulation
        .step_canonical()
        .unwrap()
        .expect("allocation is independently scheduled");
    assert_eq!(simulation.time(), at);
}

fn completion(simulation: &mut Canwu, operation: &ResourceCompletionOperationV1) {
    enqueue_resource_completion_operation(simulation, SimTime::from_minutes(163), operation)
        .unwrap();
    simulation
        .step_canonical()
        .unwrap()
        .expect("live completion boundary");
}

// All grants and the certificate are created by the installed ResourcePlugin after startup.
#[allow(clippy::too_many_lines)]
fn transfer_request(simulation: &mut Canwu) -> ResourceOperationRequestV1 {
    let leg = state(simulation)
        .allocation_legs
        .values()
        .next()
        .unwrap()
        .clone();
    let account_revision = state(simulation).accounts[&leg.account].revision;
    let acquisition = CompletionLeaseAcquisitionId::new("fixture:lease:relief-transfer").unwrap();
    let operation_key = ResourceOperationKey::new("fixture:transfer:relief").unwrap();
    let grant = CompletionCapacityGrantId::new("fixture:grant:relief-transfer").unwrap();
    let eligibility = EligibilityEnvelopeV1::new(
        Vec::new(),
        BTreeMap::new(),
        BTreeSet::new(),
        Vec::new(),
        Vec::new(),
    )
    .unwrap();
    completion(
        simulation,
        &ResourceCompletionOperationV1::Acquire(RequestCompletionLeaseV1 {
            id: acquisition.clone(),
            operation_key: operation_key.clone(),
            holder: holder(),
            operation_namespace: "fixture.transfer".into(),
            eligibility_time: SimTime::from_minutes(163),
            eligibility_envelope: eligibility.clone(),
            recipe: CompletionCapacityRecipeV1 {
                receipts: MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE,
                mutations: 2,
                reports_per_holder: 0,
                holders: 0,
                bytes: 2048,
            },
            expected_participants: BTreeSet::from([PLUGIN_NAME.to_owned()]),
            policy_class: CompletionPolicyClassV1::Guaranteed,
        }),
    );
    completion(
        simulation,
        &ResourceCompletionOperationV1::Grant(GrantCompletionCapacityV1 {
            grant_id: grant.clone(),
            acquisition: acquisition.clone(),
            expected_acquisition_revision: state(simulation).completion_leases.acquisitions
                [&acquisition]
                .revision,
            owner_plugin: PLUGIN_NAME.into(),
            current_boundary: 0,
            target_versions: vec![
                CompletionLockedTargetV1::Account {
                    id: leg.account.clone(),
                    revision: account_revision,
                },
                CompletionLockedTargetV1::AllocationLeg {
                    id: leg.id.clone(),
                    revision: leg.revision,
                },
                CompletionLockedTargetV1::Demand {
                    id: leg.demand.clone(),
                    revision: leg.demand_revision,
                },
            ],
        }),
    );
    let current = state(simulation);
    completion(
        simulation,
        &ResourceCompletionOperationV1::Prepare(PrepareCompletionCapacityV1 {
            acquisition: acquisition.clone(),
            grant: grant.clone(),
            current_boundary: 0,
            expected_acquisition_revision: current.completion_leases.acquisitions[&acquisition]
                .revision,
            expected_grant_revision: current.completion_leases.grants[&grant].revision,
            eligibility_envelope_digest: eligibility.digest.clone(),
        }),
    );
    let current = state(simulation);
    completion(
        simulation,
        &ResourceCompletionOperationV1::Activate(ActivateCompletionLeaseV1 {
            acquisition: acquisition.clone(),
            grant: grant.clone(),
            current_boundary: 0,
            expected_acquisition_revision: current.completion_leases.acquisitions[&acquisition]
                .revision,
            expected_grant_revision: current.completion_leases.grants[&grant].revision,
            at: SimTime::from_minutes(163),
            eligibility_envelope_digest: eligibility.digest,
        }),
    );
    ResourceOperationRequestV1::BeginTransfer(ResourceTransferStartRequestV1 {
        operation_key,
        transfer_id: ResourceTransferId::new("fixture:transfer:relief").unwrap(),
        allocation: (&leg).into(),
        expected_account_revision: account_revision,
        destination: None,
        at: SimTime::from_minutes(163),
        completion_certificate: state(simulation).completion_leases.certificates[&acquisition]
            .clone(),
    })
}

fn allocated(
    plugin: &ResourcePlugin,
    requester_first: bool,
    policy: &ResourceDemandSourcePolicyV1,
) -> Canwu {
    let mut simulation = new_simulation(plugin, requester_first);
    assert!(state(&simulation).demands.is_empty());
    let mut requested = demand(RELIEF, 20, 277);
    requested.source_policy = policy.clone();
    submit_demand(&mut simulation, 1, requested);
    let snapshot = simulation.snapshot_json().unwrap();
    let mut restored = Canwu::from_snapshot_json_with_plugins(&snapshot, &[plugin]).unwrap();
    allocate(&mut simulation);
    allocate(&mut restored);
    assert_eq!(
        simulation.snapshot_json().unwrap(),
        restored.snapshot_json().unwrap()
    );
    let resource = state(&simulation);
    resource.validate().unwrap();
    assert_eq!(resource.allocation_legs.len(), 1);
    let leg = resource.allocation_legs.values().next().unwrap();
    let expected_account = match &policy {
        ResourceDemandSourcePolicyV1::Pooled => FIRST,
        ResourceDemandSourcePolicyV1::ExactAccounts(accounts) => accounts.first().unwrap().as_str(),
    };
    assert_eq!(leg.account.as_str(), expected_account);
    assert_eq!(leg.quantity, 60);
    assert_eq!(
        resource.accounts[&leg.account].custodian,
        KnowledgeHolderRef::Person(
            if requester_first || matches!(policy, ResourceDemandSourcePolicyV1::ExactAccounts(_)) {
                REQUESTER
            } else {
                OTHER_CUSTODIAN
            }
        )
    );
    assert_eq!(
        resource
            .account_quantities(&ResourceAccountId::new(expected_account).unwrap())
            .unwrap()
            .reserved,
        60
    );
    assert_eq!(
        resource.accounts[&ResourceAccountId::new(if expected_account == FIRST {
            LAST
        } else {
            FIRST
        })
        .unwrap()]
            .balance,
        100
    );
    assert_eq!(
        resource.conservation,
        ConservationTotalsV1 {
            opening_balances: 200,
            ..ConservationTotalsV1::default()
        }
    );
    simulation
}

fn exact_replay(simulation: &Canwu, plugin: &ResourcePlugin) {
    let replay = Canwu::replay_from_journal(&[plugin], &simulation.replay_journal()).unwrap();
    assert_eq!(
        simulation.snapshot_json().unwrap(),
        replay.snapshot_json().unwrap()
    );
    assert_eq!(simulation.boundary_head_hash(), replay.boundary_head_hash());
}

#[test]
fn observed_foreign_reservation_is_not_authority_to_transfer_foreign_stock() {
    let plugin = ResourcePlugin::default();
    let mut simulation = allocated(&plugin, false, &ResourceDemandSourcePolicyV1::Pooled);
    let request = transfer_request(&mut simulation);
    let before = state(&simulation);
    simulation
        .enqueue_command(
            SimTime::from_minutes(163),
            0,
            CommandRequest::new(
                CommandRequestId::new(2),
                simulation.revision(),
                envelope(holder(), request, 163),
            ),
        )
        .unwrap();
    simulation.step_canonical().unwrap().unwrap();
    let attempts = simulation.command_attempts();
    let CommandAttemptOutcome::Rejected { error } = &attempts.last().unwrap().outcome else {
        panic!("foreign source admitted");
    };
    assert_eq!(error.code, ErrorCode::InvalidAuthority);
    assert_eq!(
        before,
        state(&simulation),
        "authority rejection changes no resources or lease"
    );
    assert!(state(&simulation).transfers.is_empty());
    exact_replay(&simulation, &plugin);
}

#[test]
fn own_source_ordering_allows_real_transfer_with_same_requester_authority() {
    let plugin = ResourcePlugin::default();
    let mut simulation = allocated(&plugin, true, &ResourceDemandSourcePolicyV1::Pooled);
    let request = transfer_request(&mut simulation);
    simulation
        .enqueue_command(
            SimTime::from_minutes(163),
            0,
            CommandRequest::new(
                CommandRequestId::new(2),
                simulation.revision(),
                envelope(holder(), request, 163),
            ),
        )
        .unwrap();
    let pending = simulation.snapshot_json().unwrap();
    let mut restored = Canwu::from_snapshot_json_with_plugins(&pending, &[&plugin]).unwrap();
    for sim in [&mut simulation, &mut restored] {
        sim.step_canonical().unwrap().unwrap();
        sim.step_canonical().unwrap().unwrap();
        let resource = state(sim);
        resource.validate().unwrap();
        let transfer =
            &resource.transfers[&ResourceTransferId::new("fixture:transfer:relief").unwrap()];
        assert_eq!(transfer.source.as_str(), FIRST);
        assert_eq!(transfer.escrow, 60);
        assert_eq!(transfer.state, ResourceTransferState::PendingDispatch);
        assert_eq!(
            resource.accounts[&ResourceAccountId::new(FIRST).unwrap()].balance,
            40
        );
        assert_eq!(
            resource.accounts[&ResourceAccountId::new(LAST).unwrap()].balance,
            100
        );
        assert_eq!(
            resource.accounts.values().map(|a| a.balance).sum::<u64>() + transfer.escrow,
            200
        );
    }
    assert_eq!(
        simulation.snapshot_json().unwrap(),
        restored.snapshot_json().unwrap()
    );
    exact_replay(&simulation, &plugin);
}

#[test]
fn exact_source_policy_selects_requester_owned_account_and_replays() {
    let plugin = ResourcePlugin::default();
    let exact =
        ResourceDemandSourcePolicyV1::ExactAccounts(vec![ResourceAccountId::new(LAST).unwrap()]);
    let mut simulation = allocated(&plugin, false, &exact);
    let request = transfer_request(&mut simulation);
    simulation
        .enqueue_command(
            SimTime::from_minutes(163),
            0,
            CommandRequest::new(
                CommandRequestId::new(2),
                simulation.revision(),
                envelope(holder(), request, 163),
            ),
        )
        .unwrap();
    simulation.step_canonical().unwrap().unwrap();
    simulation.step_canonical().unwrap().unwrap();
    let resource = state(&simulation);
    let transfer = resource.transfers.values().next().unwrap();
    assert_eq!(transfer.source.as_str(), LAST);
    assert_eq!(
        resource.accounts[&ResourceAccountId::new(FIRST).unwrap()].balance,
        100
    );
    assert_eq!(
        resource.accounts[&ResourceAccountId::new(LAST).unwrap()].balance,
        40
    );
    assert_eq!(resource.conservation.opening_balances, 200);
    exact_replay(&simulation, &plugin);
}

fn assert_rejected_command(
    simulation: &mut Canwu,
    request: ResourceOperationRequestV1,
    id: u64,
    at: i64,
) {
    let before = state(simulation);
    simulation
        .enqueue_command(
            SimTime::from_minutes(at),
            0,
            CommandRequest::new(
                CommandRequestId::new(id),
                simulation.revision(),
                envelope(holder(), request, at),
            ),
        )
        .unwrap();
    simulation.step_canonical().unwrap().unwrap();
    let attempts = simulation.command_attempts();
    let CommandAttemptOutcome::Rejected { error } = &attempts.last().unwrap().outcome else {
        panic!("invalid source command was admitted");
    };
    assert_eq!(error.code, ErrorCode::InvalidAuthority);
    assert_eq!(
        state(simulation),
        before,
        "rejection must not mutate resource state"
    );
}

#[test]
fn invalid_exact_sources_reject_without_mutation_and_allow_later_valid_demand() {
    let plugin = ResourcePlugin::default();
    let policies = vec![
        vec![ResourceAccountId::new(FIRST).unwrap()], // another custodian
        vec![],
        vec![ResourceAccountId::new(LAST).unwrap(); 2],
        vec![
            ResourceAccountId::new(LAST).unwrap(),
            ResourceAccountId::new(FIRST).unwrap(),
        ],
        (0..=MAX_DEMAND_SOURCE_ACCOUNTS)
            .map(|id| ResourceAccountId::new(format!("fixture:source:{id:04}")).unwrap())
            .collect(),
        vec![ResourceAccountId::new("fixture:missing").unwrap()],
    ];
    for accounts in policies {
        let mut simulation = new_simulation(&plugin, false);
        let mut requested = demand("fixture:invalid", 20, 277);
        requested.source_policy = ResourceDemandSourcePolicyV1::ExactAccounts(accounts);
        assert_rejected_command(
            &mut simulation,
            ResourceOperationRequestV1::SubmitDemand(ResourceSubmitDemandRequestV1 {
                operation_key: ResourceOperationKey::new("fixture:submit:invalid").unwrap(),
                demand: requested,
            }),
            91,
            137,
        );
        let mut valid = demand(RELIEF, 20, 277);
        valid.source_policy = ResourceDemandSourcePolicyV1::ExactAccounts(vec![
            ResourceAccountId::new(LAST).unwrap(),
        ]);
        submit_demand(&mut simulation, 92, valid);
        allocate(&mut simulation);
        assert_eq!(
            state(&simulation)
                .allocation_legs
                .values()
                .next()
                .unwrap()
                .account
                .as_str(),
            LAST
        );
        exact_replay(&simulation, &plugin);
    }
}

#[test]
fn exact_sources_bound_scarcity_minimum_and_partial_fulfillment() {
    let plugin = ResourcePlugin::default();
    for (minimum, partial, reserved, status) in [
        (
            10,
            PartialFulfillmentPolicy::AcceptPartial,
            100,
            DemandStatus::Open,
        ),
        (
            120,
            PartialFulfillmentPolicy::AcceptPartial,
            0,
            DemandStatus::RejectedMinimum,
        ),
        (
            10,
            PartialFulfillmentPolicy::RejectPartial,
            0,
            DemandStatus::Open,
        ),
    ] {
        let mut simulation = new_simulation(&plugin, false);
        let mut requested = demand(RELIEF, 20, 277);
        requested.requested = 120;
        requested.minimum_useful = minimum;
        requested.partial_fulfillment = partial;
        requested.source_policy = ResourceDemandSourcePolicyV1::ExactAccounts(vec![
            ResourceAccountId::new(LAST).unwrap(),
        ]);
        submit_demand(&mut simulation, 1, requested);
        allocate(&mut simulation);
        let resource = state(&simulation);
        assert_eq!(
            resource
                .account_quantities(&ResourceAccountId::new(LAST).unwrap())
                .unwrap()
                .reserved,
            reserved
        );
        assert_eq!(
            resource
                .account_quantities(&ResourceAccountId::new(FIRST).unwrap())
                .unwrap()
                .reserved,
            0
        );
        assert_eq!(
            resource.demands[&ResourceDemandId::new(RELIEF).unwrap()].status,
            status
        );
        assert_eq!(
            resource
                .accounts
                .values()
                .map(|account| account.balance)
                .sum::<u64>(),
            200
        );
        exact_replay(&simulation, &plugin);
    }
}

#[test]
fn policy_freezes_after_live_transfer_with_zero_fulfilled_and_replays_rejection() {
    let plugin = ResourcePlugin::default();
    let exact =
        ResourceDemandSourcePolicyV1::ExactAccounts(vec![ResourceAccountId::new(LAST).unwrap()]);
    let mut simulation = allocated(&plugin, false, &exact);
    let request = transfer_request(&mut simulation);
    simulation
        .enqueue_command(
            SimTime::from_minutes(163),
            0,
            CommandRequest::new(
                CommandRequestId::new(2),
                simulation.revision(),
                envelope(holder(), request, 163),
            ),
        )
        .unwrap();
    simulation.step_canonical().unwrap().unwrap();
    simulation.step_canonical().unwrap().unwrap();
    let before = state(&simulation);
    let demand_id = ResourceDemandId::new(RELIEF).unwrap();
    assert_eq!(before.demands[&demand_id].fulfilled, 0);
    assert!(
        before
            .reservations
            .values()
            .all(|reservation| reservation.status == ReservationStatus::Consumed)
    );
    assert!(!before.reservation_by_demand[&demand_id].is_empty());
    assert_eq!(before.transfers.values().next().unwrap().escrow, 60);
    let mut replacement = before.demands[&demand_id].clone();
    replacement.source_policy = ResourceDemandSourcePolicyV1::Pooled;
    let operation_key = ResourceOperationKey::new("fixture:amend:in-flight").unwrap();
    simulation
        .enqueue_command(
            SimTime::from_minutes(164),
            0,
            CommandRequest::new(
                CommandRequestId::new(3),
                simulation.revision(),
                envelope(
                    holder(),
                    ResourceOperationRequestV1::AmendDemand(ResourceAmendDemandRequestV1 {
                        operation_key: operation_key.clone(),
                        expected_demand_revision: replacement.revision,
                        replacement,
                    }),
                    164,
                ),
            ),
        )
        .unwrap();
    let snapshot = simulation.snapshot_json().unwrap();
    let mut restored = Canwu::from_snapshot_json_with_plugins(&snapshot, &[&plugin]).unwrap();
    for sim in [&mut simulation, &mut restored] {
        sim.step_canonical().unwrap().unwrap();
        sim.step_canonical().unwrap().unwrap();
        let after = state(sim);
        assert_eq!(
            after.outcomes[&operation_key].status,
            ResourceOperationStatus::Rejected
        );
        assert_eq!(
            after.outcomes[&operation_key].rejection_code.as_deref(),
            Some("invalid_lifecycle")
        );
        assert_eq!(after.demands, before.demands);
        assert_eq!(after.accounts, before.accounts);
        assert_eq!(after.reservations, before.reservations);
        assert_eq!(after.allocation_legs, before.allocation_legs);
        assert_eq!(after.transfers, before.transfers);
        assert_eq!(after.conservation, before.conservation);
    }
    assert_eq!(
        simulation.snapshot_json().unwrap(),
        restored.snapshot_json().unwrap()
    );
    exact_replay(&simulation, &plugin);
}

fn alter_policy(value: &mut serde_json::Value) -> usize {
    match value {
        serde_json::Value::Object(object) => {
            let mut count = 0;
            if let Some(policy) = object.get_mut("source_policy") {
                *policy = serde_json::json!({"exact_accounts": [FIRST]});
                count += 1;
            }
            count + object.values_mut().map(alter_policy).sum::<usize>()
        }
        serde_json::Value::Array(items) => items.iter_mut().map(alter_policy).sum(),
        _ => 0,
    }
}

#[test]
fn source_policy_restore_archive_and_tamper_validation_preserve_selection() {
    let plugin = ResourcePlugin::default();
    let exact =
        ResourceDemandSourcePolicyV1::ExactAccounts(vec![ResourceAccountId::new(LAST).unwrap()]);
    let mut simulation = allocated(&plugin, false, &exact);
    let demand_id = ResourceDemandId::new(RELIEF).unwrap();
    let snapshot = simulation.snapshot_json().unwrap();
    let restored = Canwu::from_snapshot_json_with_plugins(&snapshot, &[&plugin]).unwrap();
    assert_eq!(state(&restored).demands[&demand_id].source_policy, exact);
    assert_eq!(restored.snapshot_json().unwrap(), snapshot);

    // Direct DTO default is a compatibility convenience, not a snapshot migration.
    let mut dto = serde_json::to_value(&state(&simulation).demands[&demand_id]).unwrap();
    dto.as_object_mut().unwrap().remove("source_policy");
    assert_eq!(
        serde_json::from_value::<ResourceDemand>(dto)
            .unwrap()
            .source_policy,
        ResourceDemandSourcePolicyV1::Pooled
    );
    let mut tampered: serde_json::Value = serde_json::from_str(&snapshot).unwrap();
    assert!(alter_policy(&mut tampered) > 0);
    assert!(
        Canwu::from_snapshot_json_with_plugins(
            &serde_json::to_string(&tampered).unwrap(),
            &[&plugin]
        )
        .is_err()
    );

    // A valid owned list still cannot be rebound onto an existing foreign leg.
    let mut forged = state(&simulation);
    forged
        .accounts
        .get_mut(&ResourceAccountId::new(FIRST).unwrap())
        .unwrap()
        .custodian = holder();
    forged.demands.get_mut(&demand_id).unwrap().source_policy =
        ResourceDemandSourcePolicyV1::ExactAccounts(vec![ResourceAccountId::new(FIRST).unwrap()]);
    assert!(forged.validate().is_err());

    let before = state(&simulation);
    simulation
        .enqueue_command(
            SimTime::from_minutes(150),
            0,
            CommandRequest::new(
                CommandRequestId::new(2),
                simulation.revision(),
                envelope(
                    holder(),
                    ResourceOperationRequestV1::CancelDemand(ResourceCancelDemandRequestV1 {
                        operation_key: ResourceOperationKey::new("fixture:cancel:archive").unwrap(),
                        demand: demand_id.clone(),
                        expected_demand_revision: before.demands[&demand_id].revision,
                    }),
                    150,
                ),
            ),
        )
        .unwrap();
    simulation.step_canonical().unwrap().unwrap();
    simulation.step_canonical().unwrap().unwrap();
    let terminal = state(&simulation);
    let archive = terminal
        .prepare_resource_archive(terminal.terminal_archive_candidates.len())
        .unwrap();
    archive.validate().unwrap();
    let ordinal = archive.blob.records.iter().position(|record| matches!(&record.payload, ResourceTerminalArchivePayloadV1::Demand(demand) if demand.id == demand_id)).unwrap();
    let ResourceTerminalArchivePayloadV1::Demand(archived) = &archive.blob.records[ordinal].payload
    else {
        unreachable!()
    };
    assert_eq!(archived.source_policy, exact);
    let mut corrupted = archive.clone();
    let ResourceTerminalArchivePayloadV1::Demand(archived) =
        &mut corrupted.blob.records[ordinal].payload
    else {
        unreachable!()
    };
    archived.source_policy = ResourceDemandSourcePolicyV1::ExactAccounts(Vec::new());
    let error = corrupted.validate().unwrap_err();
    assert!(
        error
            .to_string()
            .contains("nonempty, bounded, sorted and unique")
    );
    exact_replay(&simulation, &plugin);
}

#[test]
fn exact_admission_rejects_closed_and_wrong_revision_accounts_and_foreign_amendment() {
    let plugin = ResourcePlugin::default();
    for mismatch in [0, 1, 2] {
        let mut simulation = new_simulation(&plugin, false);
        let mut requested = demand(RELIEF, 20, 277);
        requested.source_policy = ResourceDemandSourcePolicyV1::ExactAccounts(vec![
            ResourceAccountId::new(LAST).unwrap(),
        ]);
        // Genesis validation and runtime admission share this exact contract.
        let mut genesis = state(&simulation);
        match mismatch {
            0 => {
                genesis
                    .accounts
                    .get_mut(&ResourceAccountId::new(LAST).unwrap())
                    .unwrap()
                    .closed = true;
            }
            1 => {
                requested.resource_revision =
                    ResourceDefinitionRevisionId::new("fixture:wrong-resource").unwrap();
            }
            2 => {
                requested.unit_revision =
                    ResourceUnitRevisionId::new("fixture:wrong-unit").unwrap();
            }
            _ => unreachable!(),
        }
        assert!(genesis.install_demand(requested.clone()).is_err());
        if mismatch != 0 {
            assert_rejected_command(
                &mut simulation,
                ResourceOperationRequestV1::SubmitDemand(ResourceSubmitDemandRequestV1 {
                    operation_key: ResourceOperationKey::new("fixture:submit:wrong-revision")
                        .unwrap(),
                    demand: requested,
                }),
                1,
                137,
            );
        }
    }
    let mut simulation = new_simulation(&plugin, false);
    let mut requested = demand(RELIEF, 20, 277);
    requested.source_policy =
        ResourceDemandSourcePolicyV1::ExactAccounts(vec![ResourceAccountId::new(LAST).unwrap()]);
    submit_demand(&mut simulation, 1, requested);
    let mut replacement =
        state(&simulation).demands[&ResourceDemandId::new(RELIEF).unwrap()].clone();
    replacement.source_policy =
        ResourceDemandSourcePolicyV1::ExactAccounts(vec![ResourceAccountId::new(FIRST).unwrap()]);
    assert_rejected_command(
        &mut simulation,
        ResourceOperationRequestV1::AmendDemand(ResourceAmendDemandRequestV1 {
            operation_key: ResourceOperationKey::new("fixture:amend:foreign").unwrap(),
            expected_demand_revision: replacement.revision,
            replacement,
        }),
        2,
        138,
    );
    allocate(&mut simulation);
    assert_eq!(
        state(&simulation)
            .allocation_legs
            .values()
            .next()
            .unwrap()
            .account
            .as_str(),
        LAST
    );
    exact_replay(&simulation, &plugin);
}
