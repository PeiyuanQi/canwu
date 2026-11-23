//! Downstream gap G-111: the host application changes an open account's
//! capacity.
//!
//! In a plugin-owned run, the host raises a granary's capacity, citing the
//! record of the works. The account revision advances, the outcome keeps the
//! evidence, and the granary's reservation carries over. A credit up to the
//! new capacity then lands, and one past it is a recorded
//! `capacity_exceeded` rejection. A tracked command cannot carry the
//! operation. A restored run and an exact replay agree.
//!
//! A second run refuses a capacity below the balance as a recorded
//! rejection, and refuses a lowering that would leave no room for the escrow
//! of an open transfer naming the account, as source or destination. A
//! lowering to exactly the balance plus that escrow is admitted, and the
//! transfer's acceptance then fills the destination to its new capacity.

#![allow(
    clippy::needless_pass_by_value,
    clippy::similar_names,
    clippy::too_many_lines
)]

use canwu_api::{
    BoundaryRequest, Canwu, CanwuError, CanwuRun, CommandAttemptOutcome, CommandEnvelope,
    CommandRequest, CommandRequestId, DomainRecord, DomainRecordClass, DomainRecordKind,
    DomainRecordLifecycle, DomainRecordRef, DomainRecordSchema, DomainRecordType,
    DomainRecordVersionRef, DomainRecordVersionSource, DomainValueKindClass, EntityRef, Issuer,
    KnowledgeHolderRef, PayloadSchema, PersonId, PluginRegistrar, Scenario, SimDuration, SimTime,
    SimulationPlugin,
};
use canwu_resource::*;
use std::collections::{BTreeMap, BTreeSet};

const NAMESPACE: &str = "fixture.resource";
const EVIDENCE_NAMESPACE: &str = "fixture.provider";
const PROVIDER: &str = "fixture-provider";
const KEEPER: u64 = 1;
const MERCHANT: u64 = 2;

fn holder(value: u64) -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(PersonId::new(value))
}

fn minute(value: i64) -> SimTime {
    SimTime::EPOCH + SimDuration::minutes(value)
}

fn key(value: &str) -> ResourceOperationKey {
    ResourceOperationKey::new(value).expect("operation key")
}

fn account(name: &str) -> ResourceAccountId {
    ResourceAccountId::new(format!("fixture:account:{name}")).expect("account")
}

fn acquisition(suffix: &str) -> CompletionLeaseAcquisitionId {
    CompletionLeaseAcquisitionId::new(format!("fixture:lease:{suffix}")).expect("acquisition")
}

/// An initial version of a provider evidence record.
fn evidence(id: &str) -> DomainRecordVersionRef {
    DomainRecordVersionRef {
        record: DomainRecordRef::new(EVIDENCE_NAMESPACE, "evidence", id),
        version: 1,
        established_by: DomainRecordVersionSource::InitialScenario,
    }
}

fn provider_record(version: &DomainRecordVersionRef) -> DomainRecord {
    DomainRecord {
        reference: version.record.clone(),
        owner: PROVIDER.to_owned(),
        class: DomainRecordClass::Record,
        version: version.version,
        lifecycle: DomainRecordLifecycle::Active,
        payload: serde_json::json!({ "record": version.record.id }),
        references: Vec::new(),
    }
}

/// The works, remittance, and handover records the resource plugin reads.
struct EvidenceRecord;

impl DomainRecordType for EvidenceRecord {
    type Payload = serde_json::Value;
    type Class = DomainValueKindClass;
    const NAMESPACE: &'static str = EVIDENCE_NAMESPACE;
    const NAME: &'static str = "evidence";
}

struct ProviderPlugin;

impl SimulationPlugin for ProviderPlugin {
    fn name(&self) -> &'static str {
        PROVIDER
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000000111"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut schema = DomainRecordSchema::for_record::<EvidenceRecord>();
        schema.payload_schema = PayloadSchema::Any;
        registrar.register_record_schema(schema)
    }
}

fn partition(authority: KnowledgeHolderRef) -> CompletionCapacityPartitionV1 {
    CompletionCapacityPartitionV1 {
        authority,
        operation_namespace: NAMESPACE.to_owned(),
        guaranteed_units: 400_000,
        reserved_pending_slots: 8,
        maximum_burst_units: 100_000,
        request_token_capacity: 16,
        request_token_refill_minutes: 1,
        reacquire_cooldown_minutes: 1,
        root_acquisition_cap_per_sim_time: 4,
        guaranteed_max_wait_boundaries: 4,
    }
}

/// A granary of 50 with room for 60 and a reserved demand of 10 on it, and
/// an empty market account with room for 40, both at the depot.
fn world() -> ResourceState {
    let mut state = ResourceState::empty(ResourceLimitsV1::canonical()).expect("state");
    state
        .install_run_budget(
            RunBudgetRevisionV1 {
                revision: ResourceRevision::INITIAL,
                total_completion_units: 2_000_000,
                shared_pending_slots: 4,
                partitions: vec![partition(holder(KEEPER)), partition(holder(MERCHANT))],
                semantic_digest: String::new(),
            }
            .seal()
            .expect("budget"),
        )
        .expect("install budget");
    let unit = ResourceUnitRevisionId::new("fixture:unit:measure:v1").expect("unit");
    state
        .install_unit(ResourceUnitRevision {
            id: unit.clone(),
            revision: ResourceRevision::INITIAL,
            symbol: "u".to_owned(),
            scale_numerator: 1,
            scale_denominator: 1,
            semantic_digest: "0".repeat(64),
        })
        .expect("unit");
    let resource = ResourceDefinitionRevisionId::new("fixture:resource:grain:v1").expect("id");
    state
        .install_definition(ResourceDefinitionRevision {
            id: resource.clone(),
            resource: ResourceDefinitionId::new("fixture:resource:grain").expect("definition"),
            revision: ResourceRevision::INITIAL,
            canonical_unit: unit.clone(),
            quality: ResourceQualityId::new("fixture:quality:standard").expect("quality"),
            scope: ResourceScopeId::new("fixture:scope:market").expect("scope"),
            effective_from: SimTime::EPOCH,
            effective_until: None,
            process_suitability: BTreeSet::new(),
            semantic_digest: "1".repeat(64),
        })
        .expect("definition");
    for (name, custodian, balance, capacity) in
        [("granary", KEEPER, 50, 60), ("market", MERCHANT, 0, 40)]
    {
        state
            .install_opening_account(ResourceAccount {
                id: account(name),
                revision: ResourceRevision::INITIAL,
                custodian: holder(custodian),
                resource_revision: resource.clone(),
                unit_revision: unit.clone(),
                balance,
                capacity: Some(capacity),
                protected_floor_policy: None,
                closed: false,
                place_scope: Some(ResourceScopeId::new("fixture:place:depot").expect("place")),
            })
            .expect("account");
    }
    state
        .install_demand(ResourceDemand {
            id: ResourceDemandId::new("fixture:demand:haul").expect("demand"),
            revision: ResourceRevision::INITIAL,
            requester: holder(KEEPER),
            source_policy: ResourceDemandSourcePolicyV1::ExactAccounts(vec![account("granary")]),
            resource_revision: resource,
            unit_revision: unit,
            requested: 10,
            fulfilled: 0,
            minimum_useful: 10,
            partial_fulfillment: PartialFulfillmentPolicy::RejectPartial,
            alternative_group: None,
            due_at: SimTime::EPOCH,
            expires_at: SimTime::EPOCH + SimDuration::days(10),
            priority: 1,
            tie_break: ResourceTieBreakKey::new("fixture:tie:haul").expect("tie"),
            admitted_sequence: 0,
            protected_floor_policy: None,
            protection_override_class: None,
            status: DemandStatus::Open,
            rejection_reason: None,
        })
        .expect("demand");
    let revision = state.state_revision;
    state
        .apply_operation(&ResourceOperationRequestV1::Allocate(
            ResourceAllocationRequestV1 {
                operation_key: key("fixture:allocate:haul"),
                expected_state_revision: revision,
                at: SimTime::EPOCH,
                candidate_limit: 8,
            },
        ))
        .expect("allocation");
    state
}

fn plugin() -> ResourcePlugin {
    ResourcePlugin::new([DomainRecordKind::new(EVIDENCE_NAMESPACE, "evidence")])
}

fn runtime(seed: u64, resource_plugin: &ResourcePlugin) -> Canwu {
    let scenario = Scenario::new(
        SimTime::EPOCH,
        vec![
            EntityRef::Person(PersonId::new(KEEPER)),
            EntityRef::Person(PersonId::new(MERCHANT)),
        ],
    )
    .with_domain_records(
        ["raise", "full", "over", "shrink", "handover"]
            .into_iter()
            .map(|id| provider_record(&evidence(id)))
            .chain(world().into_records().expect("resource records"))
            .collect(),
    );
    Canwu::new_with_plugins(seed, scenario, &[resource_plugin, &ProviderPlugin]).expect("runtime")
}

fn state<H: CanwuRun + ?Sized>(canwu: &H) -> ResourceState {
    resource_state(canwu)
        .expect("resource query")
        .expect("resource state")
        .1
}

fn settle(canwu: &mut Canwu, at: SimTime) {
    canwu
        .settle_boundary(BoundaryRequest::at(at))
        .expect("a resource boundary never fails in this fixture");
}

fn next_boundary(canwu: &Canwu) -> u64 {
    canwu
        .boundaries()
        .last()
        .map_or(1, |boundary| boundary.id.get().saturating_add(1))
}

/// A capacity change of `account_id` at its current revision.
fn set_capacity(
    canwu: &Canwu,
    name: &str,
    account_id: &ResourceAccountId,
    capacity_after: Option<u64>,
    cited: &DomainRecordVersionRef,
) -> ResourceSetAccountCapacityRequestV1 {
    ResourceSetAccountCapacityRequestV1 {
        operation_key: key(&format!("fixture:capacity:{name}")),
        account: account_id.clone(),
        expected_account_revision: state(canwu).accounts[account_id].revision,
        capacity_after,
        evidence: cited.clone(),
    }
}

/// The host's capacity change, settled in a boundary of its own. Returns its
/// outcome.
fn host_capacity(
    canwu: &mut Canwu,
    at: SimTime,
    request: &ResourceSetAccountCapacityRequestV1,
) -> ResourceOperationOutcome {
    enqueue_resource_account_capacity(canwu, at, request).expect("capacity ingress");
    settle(canwu, at);
    state(canwu).outcomes[&request.operation_key].clone()
}

/// A capacity change that is the recorded rejection `capacity_exceeded` and
/// leaves the account as it was.
fn refused(canwu: &mut Canwu, at: SimTime, request: &ResourceSetAccountCapacityRequestV1) {
    let before = state(canwu).accounts[&request.account].clone();
    let outcome = host_capacity(canwu, at, request);
    assert_eq!(outcome.status, ResourceOperationStatus::Rejected);
    assert_eq!(outcome.kind, ResourceOperationKind::SetAccountCapacity);
    assert_eq!(
        outcome.rejection_code.as_deref(),
        Some("capacity_exceeded"),
        "{:?}",
        outcome.rejection_reason
    );
    assert_eq!(state(canwu).accounts[&request.account], before);
}

/// An applied capacity change that keeps its evidence and advances the
/// account revision.
fn applied(canwu: &mut Canwu, at: SimTime, request: &ResourceSetAccountCapacityRequestV1) {
    let outcome = host_capacity(canwu, at, request);
    assert_eq!(
        outcome.status,
        ResourceOperationStatus::Applied,
        "{:?}",
        outcome.rejection_reason
    );
    assert_eq!(outcome.kind, ResourceOperationKind::SetAccountCapacity);
    assert_eq!(outcome.exact_evidence, vec![request.evidence.clone()]);
    let after = &state(canwu).accounts[&request.account];
    assert_eq!(after.capacity, request.capacity_after);
    assert_eq!(
        after.revision,
        request.expected_account_revision.next().expect("revision")
    );
}

/// Submits one tracked command from `actor` and settles it when admitted.
fn command(
    canwu: &mut Canwu,
    request_id: u64,
    actor: u64,
    at: SimTime,
    request: ResourceOperationRequestV1,
) -> CommandAttemptOutcome {
    canwu
        .enqueue_command(
            at,
            0,
            CommandRequest::new(
                CommandRequestId::new(request_id),
                canwu.revision(),
                CommandEnvelope::new(
                    Issuer::Actor(PersonId::new(actor)),
                    resource_command(&ResourceCommandV1 {
                        subject: holder(actor),
                        request,
                    })
                    .expect("command"),
                )
                .at_time(at),
            ),
        )
        .expect("tracked command ingress");
    settle(canwu, at);
    let outcome = canwu
        .command_attempts()
        .last()
        .expect("command attempt")
        .outcome
        .clone();
    if matches!(outcome, CommandAttemptOutcome::Accepted { .. }) {
        settle(canwu, at);
    }
    outcome
}

/// The settled outcome of a tracked command that was admitted.
fn settled_command(
    canwu: &mut Canwu,
    request_id: u64,
    actor: u64,
    at: SimTime,
    request: ResourceOperationRequestV1,
) -> ResourceOperationOutcome {
    let operation_key = request.operation_key();
    let attempt = command(canwu, request_id, actor, at, request);
    assert!(
        matches!(attempt, CommandAttemptOutcome::Accepted { .. }),
        "{attempt:?}"
    );
    state(canwu).outcomes[&operation_key].clone()
}

/// Activates one single-step lease through the completion ingress, bound to
/// `binding` when one is given.
fn lease(
    canwu: &mut Canwu,
    lease_holder: u64,
    suffix: &str,
    operation_key: &ResourceOperationKey,
    targets: Vec<CompletionLockedTargetV1>,
    binding: Option<CompletionOperationBindingV1>,
    at: SimTime,
) -> CompletionLeaseActivationCertificateV1 {
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
    let lease = SingleStepCompletionLeaseV1 {
        lease: RequestCompletionLeaseV1 {
            id: acquisition(suffix),
            operation_key: operation_key.clone(),
            holder: holder(lease_holder),
            operation_namespace: NAMESPACE.to_owned(),
            eligibility_time: at,
            eligibility_envelope: match binding {
                Some(binding) => envelope.bound_to(binding).expect("bound envelope"),
                None => envelope,
            },
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
        grant_id: CompletionCapacityGrantId::new(format!("fixture:grant:{suffix}")).expect("grant"),
        target_versions: targets,
        current_boundary: next_boundary(canwu),
    };
    enqueue_resource_completion_operation(
        canwu,
        at,
        &ResourceCompletionOperationV1::SingleStepLease(lease),
    )
    .expect("completion ingress");
    settle(canwu, at);
    state(canwu).completion_leases.certificates[&acquisition(suffix)].clone()
}

/// The keeper books an external inflow of `quantity` to the granary under a
/// lease bound to exactly that credit. Returns its outcome.
fn inflow(
    canwu: &mut Canwu,
    request_id: u64,
    name: &str,
    quantity: u64,
    at: SimTime,
) -> ResourceOperationOutcome {
    let granary = account("granary");
    let remittance = evidence(name);
    let operation_key = key(&format!("fixture:inflow:{name}"));
    let current = state(canwu).accounts[&granary].clone();
    let certificate = lease(
        canwu,
        KEEPER,
        name,
        &operation_key,
        vec![
            CompletionLockedTargetV1::Account {
                id: granary.clone(),
                revision: current.revision,
            },
            CompletionLockedTargetV1::ExternalRecord {
                version: remittance.clone(),
            },
        ],
        Some(CompletionOperationBindingV1::new(
            ResourceOperationKind::Credit,
            quantity,
        )),
        at,
    );
    settled_command(
        canwu,
        request_id,
        KEEPER,
        at,
        ResourceOperationRequestV1::Credit(ResourceCreditRequestV1 {
            operation_key,
            account: granary,
            expected_account_revision: current.revision,
            resource_revision: current.resource_revision,
            unit_revision: current.unit_revision,
            quantity,
            source: ResourceCreditSourceV1::ExternalInflow(remittance),
            at,
            completion_certificate: certificate,
        }),
    )
}

#[test]
fn a_raised_store_takes_a_credit_up_to_its_new_capacity() {
    let resource_plugin = plugin();
    let mut canwu = runtime(111, &resource_plugin);
    let granary = account("granary");
    let raise = evidence("raise");
    let at = minute(1);

    // Only the host changes a capacity: a tracked command is refused at
    // admission and records nothing.
    let by_keeper = set_capacity(&canwu, "by-keeper", &granary, Some(100), &raise);
    let attempt = command(
        &mut canwu,
        1,
        KEEPER,
        at,
        ResourceOperationRequestV1::SetAccountCapacity(by_keeper.clone()),
    );
    assert!(
        matches!(attempt, CommandAttemptOutcome::Rejected { .. }),
        "{attempt:?}"
    );
    assert!(
        !state(&canwu)
            .outcomes
            .contains_key(&by_keeper.operation_key)
    );

    // The works complete and the host raises the granary from 60 to 100. The
    // stock and its reservation stay as they were.
    let before = state(&canwu)
        .account_quantities(&granary)
        .expect("quantities");
    let request = set_capacity(&canwu, "raise", &granary, Some(100), &raise);
    applied(&mut canwu, at, &request);
    let raised = state(&canwu);
    assert_eq!(
        raised.account_quantities(&granary).expect("quantities"),
        before
    );
    assert_eq!(before.authoritative_balance, 50);
    assert_eq!(before.reserved, 10);

    // A credit of 50 fills it to the new capacity, past the old one; one
    // more is the recorded rejection `capacity_exceeded`.
    let full = inflow(&mut canwu, 2, "full", 50, at);
    assert_eq!(
        full.status,
        ResourceOperationStatus::Applied,
        "{:?}",
        full.rejection_reason
    );
    assert_eq!(state(&canwu).accounts[&granary].balance, 100);
    let over = inflow(&mut canwu, 3, "over", 1, minute(2));
    assert_eq!(over.status, ResourceOperationStatus::Rejected);
    assert_eq!(over.rejection_code.as_deref(), Some("capacity_exceeded"));
    let settled = state(&canwu);
    assert_eq!(settled.accounts[&granary].balance, 100);
    settled.validate().expect("root after the raise");
    settled.validate_conservation().expect("conserved");
    validate_resource_runtime(&canwu).expect("the kept evidence exists");

    // A restored run and an exact replay hold the same root.
    let plugins: [&dyn SimulationPlugin; 2] = [&resource_plugin, &ProviderPlugin];
    let restored = from_resource_snapshot_json(&canwu.snapshot_json().expect("snapshot"), &plugins)
        .expect("checked restore");
    assert_eq!(state(&restored), settled);
    let replayed =
        replay_resource_from_journal(&plugins, &canwu.replay_journal()).expect("exact replay");
    assert_eq!(state(&replayed), settled);
}

#[test]
fn a_capacity_below_the_stock_or_its_stock_in_flight_is_refused() {
    let resource_plugin = plugin();
    let mut canwu = runtime(112, &resource_plugin);
    let granary = account("granary");
    let market = account("market");
    let shrink = evidence("shrink");
    let at = minute(1);

    // A capacity below the balance of 50 is a recorded rejection.
    let request = set_capacity(&canwu, "below", &granary, Some(49), &shrink);
    refused(&mut canwu, at, &request);

    // The reserved 10 leaves the granary for the market.
    let opened = state(&canwu);
    let leg = opened
        .allocation_legs
        .values()
        .next()
        .cloned()
        .expect("allocation leg");
    let granary_revision = opened.accounts[&granary].revision;
    let start_key = key("fixture:start:haul");
    let certificate = lease(
        &mut canwu,
        KEEPER,
        "start",
        &start_key,
        vec![
            CompletionLockedTargetV1::Account {
                id: granary.clone(),
                revision: granary_revision,
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
        None,
        at,
    );
    let transfer = ResourceTransferId::new("fixture:transfer:haul").expect("transfer");
    let started = settled_command(
        &mut canwu,
        1,
        KEEPER,
        at,
        ResourceOperationRequestV1::BeginTransfer(ResourceTransferStartRequestV1 {
            operation_key: start_key,
            transfer_id: transfer.clone(),
            allocation: (&leg).into(),
            expected_account_revision: granary_revision,
            destination: Some(market.clone()),
            at,
            completion_certificate: certificate,
        }),
    );
    assert_eq!(
        started.status,
        ResourceOperationStatus::Applied,
        "{:?}",
        started.rejection_reason
    );
    let open = state(&canwu);
    assert_eq!(open.accounts[&granary].balance, 40);
    assert_eq!(open.transfers[&transfer].escrow, 10);

    // While the 10 is in flight, neither end is lowered below its balance
    // plus that escrow, though both capacities hold the balance alone: the
    // market may still accept it and the granary may take it back.
    let request = set_capacity(&canwu, "market-short", &market, Some(5), &shrink);
    refused(&mut canwu, at, &request);
    let request = set_capacity(&canwu, "granary-short", &granary, Some(45), &shrink);
    refused(&mut canwu, at, &request);
    // A lowering to exactly the balance plus the escrow is admitted.
    let request = set_capacity(&canwu, "market-exact", &market, Some(10), &shrink);
    applied(&mut canwu, at, &request);
    let request = set_capacity(&canwu, "granary-exact", &granary, Some(50), &shrink);
    applied(&mut canwu, at, &request);

    // The market accepts the transfer, which fills it to its new capacity.
    let open = state(&canwu);
    let transfer_revision = open.transfers[&transfer].revision;
    let market_revision = open.accounts[&market].revision;
    let accept_key = key("fixture:accept:haul");
    let handover = evidence("handover");
    let certificate = lease(
        &mut canwu,
        MERCHANT,
        "accept",
        &accept_key,
        vec![
            CompletionLockedTargetV1::Transfer {
                id: transfer.clone(),
                revision: transfer_revision,
            },
            CompletionLockedTargetV1::Account {
                id: market.clone(),
                revision: market_revision,
            },
            CompletionLockedTargetV1::ExternalRecord {
                version: handover.clone(),
            },
        ],
        None,
        at,
    );
    let accepted = settled_command(
        &mut canwu,
        2,
        MERCHANT,
        at,
        ResourceOperationRequestV1::CompleteTransfer(ResourceTransferDispositionRequestV1 {
            operation_key: accept_key,
            transfer: transfer.clone(),
            expected_transfer_revision: transfer_revision,
            at,
            disposition: ResourceTransferDispositionV1::AcceptLocal {
                destination: market.clone(),
                expected_destination_revision: market_revision,
                handover_evidence: handover,
            },
            exact_transport_evidence: None,
            completion_certificate: certificate,
        }),
    );
    assert_eq!(
        accepted.status,
        ResourceOperationStatus::Applied,
        "{:?}",
        accepted.rejection_reason
    );
    let settled = state(&canwu);
    assert_eq!(
        settled.transfers[&transfer].state,
        ResourceTransferState::Accepted
    );
    assert_eq!(settled.accounts[&market].balance, 10);
    assert_eq!(settled.accounts[&market].capacity, Some(10));
    settled.validate().expect("root after the lowerings");
}
