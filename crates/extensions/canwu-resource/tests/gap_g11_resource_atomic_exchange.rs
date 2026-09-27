//! Gap G-11: two transfer legs start atomically. One invalid leg rejects
//! both under one outcome that cites both transfer IDs; a valid exchange
//! admits both legs, each authorized by its own source custodian's lease, and
//! the two transfers settle independently afterwards.

#![allow(
    clippy::needless_pass_by_value,
    clippy::similar_names,
    clippy::too_many_lines
)]

use canwu_api::{
    BoundaryRequest, CommandEnvelope, CommandRequest, CommandRequestId, EntityRef, Issuer,
    KnowledgeHolderRef, PersonId, Scenario, SimDuration, SimTime,
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

struct Side {
    source: ResourceAccountId,
    destination: ResourceAccountId,
    demand: ResourceDemandId,
}

fn install_resource(state: &mut ResourceState, name: &str) -> ResourceDefinitionRevisionId {
    let unit = ResourceUnitRevisionId::new(format!("test:unit:{name}:v1")).expect("unit");
    state
        .install_unit(ResourceUnitRevision {
            id: unit.clone(),
            revision: ResourceRevision::INITIAL,
            symbol: name.to_owned(),
            scale_numerator: 1,
            scale_denominator: 1,
            semantic_digest: digest('0'),
        })
        .expect("unit");
    let resource =
        ResourceDefinitionRevisionId::new(format!("test:resource:{name}:v1")).expect("resource");
    state
        .install_definition(ResourceDefinitionRevision {
            id: resource.clone(),
            resource: ResourceDefinitionId::new(format!("test:resource:{name}"))
                .expect("definition"),
            revision: ResourceRevision::INITIAL,
            canonical_unit: unit,
            quality: ResourceQualityId::new("test:quality:standard").expect("quality"),
            scope: ResourceScopeId::new("test:scope:market").expect("scope"),
            effective_from: SimTime::EPOCH,
            effective_until: None,
            process_suitability: BTreeSet::new(),
            semantic_digest: digest('1'),
        })
        .expect("definition");
    resource
}

/// Party `payer` sends `quantity` of `name` from its own account to `payee`.
fn side(
    state: &mut ResourceState,
    name: &str,
    payer: u64,
    payee: u64,
    opening: u64,
    quantity: u64,
) -> Side {
    let resource = install_resource(state, name);
    let unit = state.definitions[&resource].canonical_unit.clone();
    let source = ResourceAccountId::new(format!("test:account:{name}:{payer}")).expect("source");
    let destination =
        ResourceAccountId::new(format!("test:account:{name}:{payee}")).expect("destination");
    for (id, custodian, balance) in [
        (source.clone(), payer, opening),
        (destination.clone(), payee, 0),
    ] {
        state
            .install_opening_account(ResourceAccount {
                id,
                revision: ResourceRevision::INITIAL,
                custodian: holder(custodian),
                resource_revision: resource.clone(),
                unit_revision: unit.clone(),
                balance,
                capacity: None,
                protected_floor_policy: None,
                closed: false,
                place_scope: None,
            })
            .expect("account");
    }
    let demand = ResourceDemandId::new(format!("test:demand:{name}")).expect("demand");
    state
        .install_demand(ResourceDemand {
            id: demand.clone(),
            revision: ResourceRevision::INITIAL,
            requester: holder(payer),
            source_policy: ResourceDemandSourcePolicyV1::ExactAccounts(vec![source.clone()]),
            resource_revision: resource,
            unit_revision: unit,
            requested: quantity,
            fulfilled: 0,
            minimum_useful: quantity,
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
    Side {
        source,
        destination,
        demand,
    }
}

fn allocation(state: &ResourceState, demand: &ResourceDemandId) -> ResourceAllocationLegVersionV1 {
    state
        .allocation_legs
        .values()
        .find(|leg| &leg.demand == demand)
        .map(Into::into)
        .expect("allocation leg")
}

fn leg_targets(
    state: &ResourceState,
    leg: &ResourceAllocationLegVersionV1,
) -> Vec<CompletionLockedTargetV1> {
    let exact = &state.allocation_legs[&leg.id];
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
            id: exact.demand.clone(),
            revision: exact.demand_revision,
        },
    ]
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
        Vec::new(),
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

fn leg_terms(
    transfer: &str,
    allocation: &ResourceAllocationLegVersionV1,
    destination: &ResourceAccountId,
) -> ResourceExchangeLegTermsV1 {
    ResourceExchangeLegTermsV1 {
        transfer_id: ResourceTransferId::new(transfer).expect("transfer"),
        allocation: allocation.clone(),
        destination: Some(destination.clone()),
    }
}

/// Binds agreed leg terms to their derived key, the current source revision,
/// the lease time, and the lease itself.
fn leg(
    terms: &ResourceExchangeLegTermsV1,
    operation_key: &ResourceOperationKey,
    state: &ResourceState,
    at: SimTime,
    completion_certificate: CompletionLeaseActivationCertificateV1,
) -> ResourceTransferStartRequestV1 {
    ResourceTransferStartRequestV1 {
        operation_key: operation_key.clone(),
        transfer_id: terms.transfer_id.clone(),
        allocation: terms.allocation.clone(),
        expected_account_revision: state.accounts[&terms.allocation.account].revision,
        destination: terms.destination.clone(),
        at,
        completion_certificate,
    }
}

/// Activates both leg leases for `terms`, held by `holders` at `times`.
fn exchange(
    state: &mut ResourceState,
    terms: &ResourceExchangeTermsV1,
    holders: [u64; 2],
    times: [SimTime; 2],
) -> ResourceExchangeStartRequestV1 {
    let keys = terms.leg_operation_keys().expect("derived leg keys");
    let mut legs = Vec::new();
    for (index, leg_terms) in [&terms.leg_a, &terms.leg_b].into_iter().enumerate() {
        let targets = leg_targets(state, &leg_terms.allocation);
        let lease = certificate(
            state,
            holders[index],
            &format!("{}-{index}", terms.operation_key.as_str().replace(':', "-")),
            &keys[index],
            targets,
            times[index],
        );
        legs.push(leg(leg_terms, &keys[index], state, times[index], lease));
    }
    let leg_b = legs.pop().expect("leg b");
    let leg_a = legs.pop().expect("leg a");
    ResourceExchangeStartRequestV1 {
        operation_key: terms.operation_key.clone(),
        leg_a,
        leg_b,
    }
}

/// Releases the unused leases of an exchange that never started.
fn release(state: &mut ResourceState, request: &ResourceExchangeStartRequestV1) {
    for leg in [&request.leg_a, &request.leg_b] {
        let lease = &leg.completion_certificate;
        let (grant, expected_grant_revision) = lease.prepared_grants[0].clone();
        let expected_acquisition_revision =
            state.completion_leases.acquisitions[&lease.acquisition].revision;
        applied(
            state,
            ResourceOperationRequestV1::Completion(ResourceCompletionOperationV1::Release(
                ReleaseCompletionCapacityV1 {
                    acquisition: lease.acquisition.clone(),
                    expected_acquisition_revision,
                    grant,
                    expected_grant_revision,
                    reason: "exchange did not start".to_owned(),
                },
            )),
        );
    }
}

/// Asserts one durable rejection that cites both transfers and changes no
/// balance, allocation, lease, or transfer.
fn assert_rejected(
    state: &mut ResourceState,
    exchange: &ResourceExchangeStartRequestV1,
    code: &str,
) {
    let before = state.clone();
    let request = ResourceOperationRequestV1::BeginExchange(exchange.clone());
    let outcome = state.apply_operation(&request).expect("durable rejection");
    assert_eq!(outcome.kind, ResourceOperationKind::BeginExchange);
    assert_eq!(outcome.status, ResourceOperationStatus::Rejected);
    assert_eq!(outcome.rejection_code.as_deref(), Some(code));
    assert_eq!(
        outcome.cited_transfers,
        vec![
            exchange.leg_a.transfer_id.clone(),
            exchange.leg_b.transfer_id.clone()
        ]
    );
    assert_eq!(state.apply_operation(&request).expect("replay"), outcome);
    assert!(state.transfers.is_empty());
    assert_eq!(state.accounts, before.accounts);
    assert_eq!(state.allocation_legs, before.allocation_legs);
    assert_eq!(state.conservation, before.conservation);
    assert_eq!(
        state.completion_leases.grants,
        before.completion_leases.grants
    );
    for leg in [&exchange.leg_a, &exchange.leg_b] {
        assert!(!state.outcomes.contains_key(&leg.operation_key));
    }
}

#[test]
fn gap_g11_resource_atomic_exchange() {
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
    // Party 1 pays 30 of one resource; party 2 pays 20 of another.
    let first = side(&mut state, "first", 1, 2, 100, 30);
    let second = side(&mut state, "second", 2, 1, 50, 20);
    let expected_state_revision = state.state_revision;
    applied(
        &mut state,
        ResourceOperationRequestV1::Allocate(ResourceAllocationRequestV1 {
            operation_key: key("test:allocate:exchange"),
            expected_state_revision,
            at: SimTime::EPOCH,
            candidate_limit: 8,
        }),
    );
    let allocation_a = allocation(&state, &first.demand);
    let allocation_b = allocation(&state, &second.demand);
    let terms_for = |operation_key: &str| ResourceExchangeTermsV1 {
        operation_key: key(operation_key),
        leg_a: leg_terms("test:transfer:first", &allocation_a, &first.destination),
        leg_b: leg_terms("test:transfer:second", &allocation_b, &second.destination),
    };

    // The counterparty's leg needs a lease held by the counterparty itself.
    let unconsented = exchange(
        &mut state,
        &terms_for("test:exchange:unconsented"),
        [1, 1],
        [minute(1), minute(2)],
    );
    assert_rejected(&mut state, &unconsented, "invalid_authority");
    release(&mut state, &unconsented);

    // Leases consent to one set of terms: redirecting a leg, here party 1
    // paying itself, cannot reuse them.
    let attempt = exchange(
        &mut state,
        &terms_for("test:exchange:first-attempt"),
        [1, 2],
        [minute(3), minute(1)],
    );
    let mut redirected = attempt.clone();
    redirected.operation_key = key("test:exchange:redirected");
    redirected.leg_a.destination = Some(first.source.clone());
    assert_rejected(&mut state, &redirected, "invalid_authority");

    // One invalid leg rejects both, even after the other leg settled in the
    // detached candidate: leg b names a time its lease was not issued for.
    let mut late = attempt.clone();
    late.leg_b.at = minute(9);
    assert_rejected(&mut state, &late, "version_conflict");
    release(&mut state, &attempt);

    // A valid exchange admits both legs through one tracked command and replays.
    let terms = terms_for("test:exchange:valid");
    let valid = exchange(&mut state, &terms, [1, 2], [minute(4), minute(2)]);
    assert_eq!(valid.terms(), terms);
    let [key_a, _] = terms.leg_operation_keys().expect("derived leg keys");
    let exchange_key = valid.operation_key.clone();
    let transfer_a = valid.leg_a.transfer_id.clone();
    let transfer_b = valid.leg_b.transfer_id.clone();
    let command = ResourceCommandV1 {
        subject: holder(1),
        request: ResourceOperationRequestV1::BeginExchange(valid),
    };
    let scenario = Scenario::new(
        minute(4),
        vec![
            EntityRef::Person(PersonId::new(1)),
            EntityRef::Person(PersonId::new(2)),
        ],
    )
    .with_domain_records(vec![state.into_record().expect("resource record")]);
    let plugin = ResourcePlugin::default();
    let mut canwu =
        canwu_api::Canwu::new_with_plugins(11, scenario, &[&plugin]).expect("resource runtime");
    canwu
        .enqueue_command(
            minute(4),
            0,
            CommandRequest::new(
                CommandRequestId::new(1),
                canwu.revision(),
                CommandEnvelope::new(
                    Issuer::Actor(PersonId::new(1)),
                    resource_command(&command).expect("command"),
                ),
            ),
        )
        .expect("tracked command ingress");
    for boundary in ["command admission", "exchange settlement"] {
        canwu
            .settle_boundary(BoundaryRequest::at(minute(4)))
            .expect(boundary);
    }
    let (_, live) = resource_state(&canwu)
        .expect("query")
        .expect("resource state");
    let outcome = &live.outcomes[&exchange_key];
    assert_eq!(outcome.status, ResourceOperationStatus::Applied);
    assert_eq!(
        outcome.cited_transfers,
        vec![transfer_a.clone(), transfer_b.clone()]
    );
    assert_eq!(live.transfers[&transfer_a].escrow, 30);
    assert_eq!(live.transfers[&transfer_b].escrow, 20);
    assert_eq!(live.transfers[&transfer_a].operation_key, key_a);
    assert_eq!(live.accounts[&first.source].balance, 70);
    assert_eq!(live.accounts[&second.source].balance, 30);
    assert_ne!(
        live.transfers[&transfer_a].completion_acquisition,
        live.transfers[&transfer_b].completion_acquisition
    );
    live.validate().expect("exchange state");

    let restored =
        from_resource_snapshot_json(&canwu.snapshot_json().expect("snapshot"), &[&plugin])
            .expect("strict restore");
    assert_eq!(
        resource_state(&restored).expect("query").expect("state").1,
        live
    );
    let replayed =
        replay_resource_from_journal(&[&plugin], &canwu.replay_journal()).expect("exact replay");
    assert_eq!(
        resource_state(&replayed).expect("query").expect("state").1,
        live
    );

    // Terminal dispositions stay independent: cancelling one leg leaves the
    // other in flight with its own lease.
    let mut settled = live.clone();
    let expected_transfer_revision = settled.transfers[&transfer_a].revision;
    applied(
        &mut settled,
        ResourceOperationRequestV1::CancelTransfer(ResourceTransferCancellationRequestV1 {
            operation_key: key("test:exchange:cancel-first"),
            transfer: transfer_a.clone(),
            expected_transfer_revision,
            at: minute(5),
        }),
    );
    assert_eq!(
        settled.transfers[&transfer_a].state,
        ResourceTransferState::ReturnPending
    );
    assert_eq!(settled.transfers[&transfer_b], live.transfers[&transfer_b]);
}
