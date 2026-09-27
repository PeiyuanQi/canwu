//! Gap G-05: a grantor custodian delegates bounded access to its stock. A
//! granted demand needs a current grant issued to its requester, draws only on
//! the listed grantor accounts, and never falls back to the pool. Allocation
//! and debit charge the grant's cap exactly once, releases return it, and a
//! grant is fixed at its first reservation. The grantee's own completion lease
//! debits a granted allocation while the grant is current. Everything persists
//! through save/load and exact replay.

#![allow(
    clippy::needless_pass_by_value,
    clippy::similar_names,
    clippy::too_many_lines
)]

use canwu_api::{
    BoundaryRequest, Canwu, CanwuError, CommandAttemptOutcome, CommandEnvelope, CommandRequest,
    CommandRequestId, DomainRecord, DomainRecordClass, DomainRecordLifecycle, DomainRecordRef,
    DomainRecordSchema, DomainRecordType, DomainRecordVersionRef, DomainRecordVersionSource,
    DomainValueKindClass, EntityRef, ErrorCode, Issuer, KnowledgeHolderRef, PayloadSchema,
    PersonId, PluginRegistrar, Scenario, SimDuration, SimTime, SimulationPlugin,
};
use canwu_resource::*;
use std::collections::{BTreeMap, BTreeSet};

const NAMESPACE: &str = "test.resource";
const GRANTOR: u64 = 1;
const GRANTEE: u64 = 2;
const OUTSIDER: u64 = 3;

fn holder(value: u64) -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(PersonId::new(value))
}

fn start() -> SimTime {
    SimTime::EPOCH + SimDuration::minutes(10)
}

fn key(value: &str) -> ResourceOperationKey {
    ResourceOperationKey::new(value).expect("operation key")
}

fn account(value: &str) -> ResourceAccountId {
    ResourceAccountId::new(format!("test:account:{value}")).expect("account")
}

fn grant_id(value: &str) -> ResourceAccessGrantId {
    ResourceAccessGrantId::new(format!("test:grant:{value}")).expect("grant")
}

fn resource() -> ResourceDefinitionRevisionId {
    ResourceDefinitionRevisionId::new("test:resource:grain:v1").expect("resource")
}

fn unit() -> ResourceUnitRevisionId {
    ResourceUnitRevisionId::new("test:unit:grain:v1").expect("unit")
}

/// The application record that justifies the grant, such as an accepted
/// requisition.
fn acceptance(version: u64) -> DomainRecordVersionRef {
    DomainRecordVersionRef {
        record: DomainRecordRef::new("test.provider", "evidence", "acceptance"),
        version,
        established_by: DomainRecordVersionSource::InitialScenario,
    }
}

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
        "00000000000000000000000000000000000000000000000000000000000000e5"
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
        reserved_pending_slots: 4,
        maximum_burst_units: 100_000,
        request_token_capacity: 16,
        request_token_refill_minutes: 1,
        reacquire_cooldown_minutes: 1,
        root_acquisition_cap_per_sim_time: 4,
        guaranteed_max_wait_boundaries: 4,
    }
}

fn scenario() -> Scenario {
    let mut state = ResourceState::empty(ResourceLimitsV1::canonical()).expect("state");
    state
        .install_run_budget(
            RunBudgetRevisionV1 {
                revision: ResourceRevision::INITIAL,
                total_completion_units: 1_200_000,
                shared_pending_slots: 4,
                partitions: vec![
                    partition(holder(GRANTOR)),
                    partition(holder(GRANTEE)),
                    partition(holder(OUTSIDER)),
                ],
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
            symbol: "grain".to_owned(),
            scale_numerator: 1,
            scale_denominator: 1,
            semantic_digest: "0".repeat(64),
        })
        .expect("unit");
    state
        .install_definition(ResourceDefinitionRevision {
            id: resource(),
            resource: ResourceDefinitionId::new("test:resource:grain").expect("definition"),
            revision: ResourceRevision::INITIAL,
            canonical_unit: unit(),
            quality: ResourceQualityId::new("test:quality:standard").expect("quality"),
            scope: ResourceScopeId::new("test:scope:district").expect("scope"),
            effective_from: SimTime::EPOCH,
            effective_until: None,
            process_suitability: BTreeSet::new(),
            semantic_digest: "1".repeat(64),
        })
        .expect("definition");
    for (id, custodian, balance) in [
        ("east", GRANTOR, 60),
        ("west", GRANTOR, 60),
        ("store", GRANTEE, 0),
        ("outsider", OUTSIDER, 500),
    ] {
        state
            .install_opening_account(ResourceAccount {
                id: account(id),
                revision: ResourceRevision::INITIAL,
                custodian: holder(custodian),
                resource_revision: resource(),
                unit_revision: unit(),
                balance,
                capacity: None,
                protected_floor_policy: None,
                closed: false,
                place_scope: None,
            })
            .expect("account");
    }
    let evidence = acceptance(1);
    Scenario::new(
        start(),
        [GRANTOR, GRANTEE, OUTSIDER]
            .map(|value| EntityRef::Person(PersonId::new(value)))
            .to_vec(),
    )
    .with_domain_records(vec![
        state.into_record().expect("resource record"),
        DomainRecord {
            reference: evidence.record.clone(),
            owner: "test-provider".to_owned(),
            class: DomainRecordClass::Record,
            version: 1,
            lifecycle: DomainRecordLifecycle::Active,
            payload: serde_json::json!({ "accepted": "requisition" }),
            references: Vec::new(),
        },
    ])
}

fn state(canwu: &Canwu) -> ResourceState {
    resource_state(canwu).expect("query").expect("state").1
}

fn settle(canwu: &mut Canwu, at: SimTime) {
    canwu
        .settle_boundary(BoundaryRequest::at(at))
        .expect("resource boundary");
}

fn next_boundary(canwu: &Canwu) -> u64 {
    canwu
        .boundaries()
        .last()
        .map_or(1, |boundary| boundary.id.get().saturating_add(1))
}

/// Submits one tracked command from `actor` about itself and, when admitted,
/// settles it. Returns the admission outcome.
fn command(
    canwu: &mut Canwu,
    id: u64,
    actor: u64,
    at: SimTime,
    request: ResourceOperationRequestV1,
) -> CommandAttemptOutcome {
    canwu
        .enqueue_command(
            at,
            0,
            CommandRequest::new(
                CommandRequestId::new(id),
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

/// Asserts an admission rejection, for the stated cause, that leaves resource
/// state unchanged.
fn rejected(
    canwu: &mut Canwu,
    id: u64,
    actor: u64,
    request: ResourceOperationRequestV1,
    cause: &str,
) {
    let before = state(canwu);
    let at = canwu.time();
    let outcome = command(canwu, id, actor, at, request);
    let CommandAttemptOutcome::Rejected { error } = outcome else {
        panic!("command {id} was admitted");
    };
    assert_eq!(error.code, ErrorCode::InvalidAuthority, "command {id}");
    assert!(
        error.message.contains(cause),
        "command {id}: {}",
        error.message
    );
    assert_eq!(state(canwu), before, "command {id} changed resource state");
}

/// Asserts a durable settlement rejection for the stated cause.
fn assert_rejected_outcome(outcome: &ResourceOperationOutcome, code: &str, cause: &str) {
    assert_eq!(outcome.status, ResourceOperationStatus::Rejected);
    assert_eq!(outcome.rejection_code.as_deref(), Some(code));
    assert!(
        outcome
            .rejection_reason
            .as_deref()
            .is_some_and(|reason| reason.contains(cause)),
        "{:?}",
        outcome.rejection_reason
    );
}

/// Settles one admitted command and returns its durable outcome.
fn settled(
    canwu: &mut Canwu,
    id: u64,
    actor: u64,
    at: SimTime,
    request: ResourceOperationRequestV1,
) -> ResourceOperationOutcome {
    let operation_key = request.operation_key();
    let outcome = command(canwu, id, actor, at, request);
    assert!(
        matches!(outcome, CommandAttemptOutcome::Accepted { .. }),
        "command {id} was not admitted: {outcome:?}"
    );
    state(canwu).outcomes[&operation_key].clone()
}

fn grant(
    id: &str,
    cap_quantity: u64,
    authority_evidence: DomainRecordVersionRef,
) -> ResourceAccessGrantV1 {
    ResourceAccessGrantV1 {
        grant_id: grant_id(id),
        grantor_custodian: holder(GRANTOR),
        grantee: holder(GRANTEE),
        resource_revision: resource(),
        unit_revision: unit(),
        cap_quantity,
        valid_from: start(),
        valid_until: start() + SimDuration::days(5),
        authority_evidence,
    }
}

fn issue(grant: ResourceAccessGrantV1) -> ResourceOperationRequestV1 {
    ResourceOperationRequestV1::IssueAccessGrant(ResourceIssueAccessGrantRequestV1 {
        operation_key: key(&format!(
            "test:issue:{}",
            grant.grant_id.as_str().replace(':', "-")
        )),
        grant,
    })
}

fn demand(
    id: &str,
    requester: u64,
    policy: ResourceDemandSourcePolicyV1,
    requested: u64,
    minimum_useful: u64,
    priority: i32,
) -> ResourceDemand {
    ResourceDemand {
        id: ResourceDemandId::new(format!("test:demand:{id}")).expect("demand"),
        revision: ResourceRevision::INITIAL,
        requester: holder(requester),
        source_policy: policy,
        resource_revision: resource(),
        unit_revision: unit(),
        requested,
        fulfilled: 0,
        minimum_useful,
        partial_fulfillment: PartialFulfillmentPolicy::AcceptPartial,
        alternative_group: None,
        due_at: start(),
        expires_at: start() + SimDuration::days(2),
        priority,
        tie_break: ResourceTieBreakKey::new(format!("test:tie:{id}")).expect("tie"),
        admitted_sequence: 0,
        protected_floor_policy: None,
        protection_override_class: None,
        status: DemandStatus::Open,
        rejection_reason: None,
    }
}

fn granted(grant: &str, accounts: &[&str]) -> ResourceDemandSourcePolicyV1 {
    ResourceDemandSourcePolicyV1::Granted {
        grant_id: grant_id(grant),
        accounts: accounts.iter().map(|value| account(value)).collect(),
    }
}

fn submit(demand: ResourceDemand) -> ResourceOperationRequestV1 {
    ResourceOperationRequestV1::SubmitDemand(ResourceSubmitDemandRequestV1 {
        operation_key: key(&format!(
            "test:submit:{}",
            demand.id.as_str().replace(':', "-")
        )),
        demand,
    })
}

fn completion(canwu: &mut Canwu, at: SimTime, operation: ResourceCompletionOperationV1) {
    enqueue_resource_completion_operation(canwu, at, &operation).expect("completion ingress");
    settle(canwu, at);
}

/// Activates a live completion lease held by `lease_holder` that locks one
/// allocation leg's exact account, leg, and demand revisions.
fn lease(
    canwu: &mut Canwu,
    lease_holder: u64,
    suffix: &str,
    operation_key: &ResourceOperationKey,
    leg: &ResourceAllocationLeg,
    at: SimTime,
) -> CompletionLeaseActivationCertificateV1 {
    let acquisition =
        CompletionLeaseAcquisitionId::new(format!("test:lease:{suffix}")).expect("acquisition");
    let lease_grant =
        CompletionCapacityGrantId::new(format!("test:lease-grant:{suffix}")).expect("grant");
    let envelope = EligibilityEnvelopeV1::new(
        Vec::new(),
        BTreeMap::new(),
        BTreeSet::new(),
        Vec::new(),
        Vec::new(),
    )
    .expect("envelope");
    completion(
        canwu,
        at,
        ResourceCompletionOperationV1::Acquire(RequestCompletionLeaseV1 {
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
                bytes: 2_048,
            },
            expected_participants: BTreeSet::from([PLUGIN_NAME.to_owned()]),
            policy_class: CompletionPolicyClassV1::Guaranteed,
        }),
    );
    let current = state(canwu);
    let current_boundary = next_boundary(canwu);
    completion(
        canwu,
        at,
        ResourceCompletionOperationV1::Grant(GrantCompletionCapacityV1 {
            grant_id: lease_grant.clone(),
            acquisition: acquisition.clone(),
            expected_acquisition_revision: current.completion_leases.acquisitions[&acquisition]
                .revision,
            owner_plugin: PLUGIN_NAME.to_owned(),
            target_versions: vec![
                CompletionLockedTargetV1::Account {
                    id: leg.account.clone(),
                    revision: current.accounts[&leg.account].revision,
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
            current_boundary,
        }),
    );
    for activate in [false, true] {
        let current = state(canwu);
        let expected_acquisition_revision =
            current.completion_leases.acquisitions[&acquisition].revision;
        let expected_grant_revision = current.completion_leases.grants[&lease_grant].revision;
        let current_boundary = next_boundary(canwu);
        let operation = if activate {
            ResourceCompletionOperationV1::Activate(ActivateCompletionLeaseV1 {
                acquisition: acquisition.clone(),
                expected_acquisition_revision,
                grant: lease_grant.clone(),
                expected_grant_revision,
                at,
                current_boundary,
                eligibility_envelope_digest: envelope.digest.clone(),
            })
        } else {
            ResourceCompletionOperationV1::Prepare(PrepareCompletionCapacityV1 {
                acquisition: acquisition.clone(),
                expected_acquisition_revision,
                grant: lease_grant.clone(),
                expected_grant_revision,
                current_boundary,
                eligibility_envelope_digest: envelope.digest.clone(),
            })
        };
        completion(canwu, at, operation);
    }
    // Settle the lease's pending expiry tick before time moves on.
    settle(canwu, at);
    state(canwu).completion_leases.certificates[&acquisition].clone()
}

/// Releases an activated lease that no debit used.
fn release(canwu: &mut Canwu, acquisition: &CompletionLeaseAcquisitionId) {
    let current = state(canwu);
    let certificate = &current.completion_leases.certificates[acquisition];
    let (grant, expected_grant_revision) = certificate.prepared_grants[0].clone();
    let at = canwu.time();
    completion(
        canwu,
        at,
        ResourceCompletionOperationV1::Release(ReleaseCompletionCapacityV1 {
            acquisition: acquisition.clone(),
            expected_acquisition_revision: current.completion_leases.acquisitions[acquisition]
                .revision,
            grant,
            expected_grant_revision,
            reason: "the lease holder may not debit a granted allocation".to_owned(),
        }),
    );
}

fn begin_transfer(
    canwu: &Canwu,
    transfer: &str,
    leg: &ResourceAllocationLeg,
    operation_key: ResourceOperationKey,
    at: SimTime,
    completion_certificate: CompletionLeaseActivationCertificateV1,
) -> ResourceOperationRequestV1 {
    ResourceOperationRequestV1::BeginTransfer(ResourceTransferStartRequestV1 {
        operation_key,
        transfer_id: ResourceTransferId::new(format!("test:transfer:{transfer}")).expect("id"),
        allocation: leg.into(),
        expected_account_revision: state(canwu).accounts[&leg.account].revision,
        destination: Some(account("store")),
        at,
        completion_certificate,
    })
}

fn leg_of(state: &ResourceState, demand: &str, account_id: &str) -> ResourceAllocationLeg {
    let demand = ResourceDemandId::new(format!("test:demand:{demand}")).expect("demand");
    state
        .allocation_legs
        .values()
        .find(|leg| leg.demand == demand && leg.account == account(account_id))
        .cloned()
        .expect("allocation leg")
}

/// `cap = remaining + reserved + debited` for the grantee's holder-bound view.
fn assert_cap(canwu: &Canwu, id: &str, reserved: u64, debited: u64) {
    let status =
        resource_access_grant_status(canwu, &holder(GRANTEE), &grant_id(id)).expect("status");
    assert_eq!(
        (status.reserved_quantity, status.debited_quantity),
        (reserved, debited),
        "grant {id}"
    );
    assert_eq!(
        status.remaining_quantity + status.reserved_quantity + status.debited_quantity,
        status.grant.cap_quantity
    );
}

#[test]
fn gap_g05_resource_access_grant() {
    let resource_plugin = ResourcePlugin::default();
    let plugins: [&dyn SimulationPlugin; 2] = [&resource_plugin, &EvidencePlugin];
    let mut canwu = Canwu::new_with_plugins(5, scenario(), &plugins).expect("resource runtime");

    // Only the grantor custodian issues, and only with exact available evidence.
    rejected(
        &mut canwu,
        1,
        GRANTEE,
        issue(grant("main", 80, acceptance(1))),
        "does not control the actual target holder",
    );
    rejected(
        &mut canwu,
        2,
        GRANTOR,
        issue(grant("main", 80, acceptance(2))),
        "authority evidence is not an available exact record version",
    );
    let issued = settled(
        &mut canwu,
        3,
        GRANTOR,
        start(),
        issue(grant("main", 80, acceptance(1))),
    );
    assert_eq!(issued.status, ResourceOperationStatus::Applied);
    assert_eq!(issued.kind, ResourceOperationKind::IssueAccessGrant);
    assert_eq!(issued.exact_evidence, vec![acceptance(1)]);
    settled(
        &mut canwu,
        4,
        GRANTOR,
        start(),
        issue(grant("spare", 10, acceptance(1))),
    );
    assert_cap(&canwu, "main", 0, 0);
    let foreign = resource_access_grant_status(&canwu, &holder(OUTSIDER), &grant_id("main"))
        .expect_err("the outsider reads no grant");
    assert_eq!(foreign.code, ErrorCode::InvalidAuthority);

    // A granted demand needs the grantee as requester, grantor-custodied
    // accounts, the grant's window, and room under its cap.
    let mut late = demand("late", GRANTEE, granted("main", &["east"]), 10, 10, 1);
    late.expires_at = start() + SimDuration::days(6);
    for (id, actor, cause, request) in [
        (
            5,
            OUTSIDER,
            "was not issued to its requester",
            demand("foreign", OUTSIDER, granted("main", &["east"]), 10, 10, 1),
        ),
        (
            6,
            GRANTEE,
            "not custodied by the grantor",
            demand(
                "custody",
                GRANTEE,
                granted("main", &["east", "outsider"]),
                10,
                10,
                1,
            ),
        ),
        (
            7,
            GRANTEE,
            "remaining cap",
            demand(
                "excess",
                GRANTEE,
                granted("main", &["east", "west"]),
                90,
                10,
                1,
            ),
        ),
        (8, GRANTEE, "window exceeds its access grant", late),
        (
            9,
            GRANTEE,
            "access grant is unavailable",
            demand("unknown", GRANTEE, granted("missing", &["east"]), 10, 10, 1),
        ),
    ] {
        rejected(&mut canwu, id, actor, submit(request), cause);
    }

    // Three granted demands: one draws on two grantor accounts, one wants more
    // than the cap has left, and one names a grant revoked before any draw.
    for (id, request) in [
        (
            10,
            demand(
                "requisition",
                GRANTEE,
                granted("main", &["east", "west"]),
                70,
                70,
                3,
            ),
        ),
        (
            11,
            demand("forage", GRANTEE, granted("main", &["west"]), 20, 5, 2),
        ),
        (
            12,
            demand("levy", GRANTEE, granted("spare", &["east"]), 5, 5, 1),
        ),
    ] {
        let outcome = settled(&mut canwu, id, GRANTEE, start(), submit(request));
        assert_eq!(outcome.status, ResourceOperationStatus::Applied);
    }
    let revision = state(&canwu).access_grants[&grant_id("spare")].revision;
    let revoked = settled(
        &mut canwu,
        13,
        GRANTOR,
        start(),
        ResourceOperationRequestV1::RevokeAccessGrant(ResourceRevokeAccessGrantRequestV1 {
            operation_key: key("test:revoke:spare"),
            grant_id: grant_id("spare"),
            expected_grant_revision: revision,
        }),
    );
    assert_eq!(revoked.status, ResourceOperationStatus::Applied);
    assert_eq!(
        state(&canwu).access_grants[&grant_id("spare")].status,
        ResourceAccessGrantStatusV1::Revoked
    );

    let expected_state_revision = state(&canwu).state_revision;
    enqueue_resource_allocation(
        &mut canwu,
        start(),
        &holder(GRANTEE),
        &ResourceAllocationRequestV1 {
            operation_key: key("test:allocate:grantee"),
            expected_state_revision,
            at: start(),
            candidate_limit: 8,
        },
    )
    .expect("allocation ingress");
    settle(&mut canwu, start());
    let allocated = state(&canwu);
    // The requisition takes 60 + 10 from the grantor's accounts; the forage
    // demand is cut to the 10 the cap has left although west still holds 50;
    // the revoked grant supplies nothing. Nothing falls back to the outsider.
    assert_eq!(leg_of(&allocated, "requisition", "east").quantity, 60);
    assert_eq!(leg_of(&allocated, "requisition", "west").quantity, 10);
    assert_eq!(leg_of(&allocated, "forage", "west").quantity, 10);
    let levy = &allocated.demands[&ResourceDemandId::new("test:demand:levy").expect("demand")];
    assert_eq!(levy.status, DemandStatus::RejectedMinimum);
    assert_eq!(
        levy.rejection_reason.as_deref(),
        Some("access_grant_unavailable")
    );
    assert!(
        allocated
            .allocation_legs
            .values()
            .all(|leg| leg.account != account("outsider") && leg.account != account("store"))
    );
    assert_eq!(
        allocated
            .account_quantities(&account("outsider"))
            .expect("outsider")
            .reserved,
        0
    );
    assert_cap(&canwu, "main", 80, 0);

    // The grant is fixed at its first reservation.
    let revision = allocated.access_grants[&grant_id("main")].revision;
    let fixed = settled(
        &mut canwu,
        14,
        GRANTOR,
        start(),
        ResourceOperationRequestV1::RevokeAccessGrant(ResourceRevokeAccessGrantRequestV1 {
            operation_key: key("test:revoke:main"),
            grant_id: grant_id("main"),
            expected_grant_revision: revision,
        }),
    );
    assert_rejected_outcome(&fixed, "invalid_lifecycle", "fixed by a reservation");

    // Only the grantee debits a granted allocation, under its own lease: the
    // grantor custodian can neither command the debit nor lend its lease.
    let east = leg_of(&allocated, "requisition", "east");
    let forage = leg_of(&allocated, "forage", "west");
    let grantor_key = key("test:transfer:grantor-lease");
    let grantor_lease = lease(
        &mut canwu,
        GRANTOR,
        "grantor",
        &grantor_key,
        &forage,
        start(),
    );
    let request = begin_transfer(
        &canwu,
        "grantor",
        &east,
        key("test:transfer:grantor"),
        start(),
        grantor_lease.clone(),
    );
    rejected(
        &mut canwu,
        15,
        GRANTOR,
        request,
        "does not control the actual target holder",
    );
    let grantor_lease_id = grantor_lease.acquisition.clone();
    let request = begin_transfer(
        &canwu,
        "grantor-lease",
        &forage,
        grantor_key,
        start(),
        grantor_lease,
    );
    let outcome = settled(&mut canwu, 16, GRANTEE, start(), request);
    assert_rejected_outcome(&outcome, "invalid_authority", "grantee's completion lease");
    release(&mut canwu, &grantor_lease_id);

    let requisition_key = key("test:transfer:requisition");
    let grantee_lease = lease(
        &mut canwu,
        GRANTEE,
        "grantee",
        &requisition_key,
        &east,
        start(),
    );
    let request = begin_transfer(
        &canwu,
        "requisition",
        &east,
        requisition_key.clone(),
        start(),
        grantee_lease,
    );
    let outcome = settled(&mut canwu, 17, GRANTEE, start(), request);
    assert_eq!(outcome.status, ResourceOperationStatus::Applied);
    let transfer = &state(&canwu).transfers
        [&ResourceTransferId::new("test:transfer:requisition").expect("id")];
    assert_eq!(transfer.escrow, 60);
    assert_eq!(transfer.access_grant, Some(grant_id("main")));
    // The debit moved 60 from reserved to debited: charged exactly once.
    assert_cap(&canwu, "main", 20, 60);

    // A granted debit settles only at its certified lease time. In an
    // exchange, the grantee's granted leg certified a minute before its
    // counterparty's rejects the whole exchange, citing both transfers.
    let counter = demand(
        "counter",
        OUTSIDER,
        ResourceDemandSourcePolicyV1::ExactAccounts(vec![account("outsider")]),
        30,
        30,
        1,
    );
    let outcome = settled(&mut canwu, 30, OUTSIDER, start(), submit(counter));
    assert_eq!(outcome.status, ResourceOperationStatus::Applied);
    let expected_state_revision = state(&canwu).state_revision;
    enqueue_resource_allocation(
        &mut canwu,
        start(),
        &holder(OUTSIDER),
        &ResourceAllocationRequestV1 {
            operation_key: key("test:allocate:outsider"),
            expected_state_revision,
            at: start(),
            candidate_limit: 8,
        },
    )
    .expect("counterparty allocation ingress");
    settle(&mut canwu, start());
    let counter_leg = leg_of(&state(&canwu), "counter", "outsider");
    let transfer_id = |value: &str| ResourceTransferId::new(value).expect("transfer");
    let terms = ResourceExchangeTermsV1 {
        operation_key: key("test:exchange:granted"),
        leg_a: ResourceExchangeLegTermsV1 {
            transfer_id: transfer_id("test:transfer:exchange-granted"),
            allocation: (&forage).into(),
            destination: Some(account("outsider")),
        },
        leg_b: ResourceExchangeLegTermsV1 {
            transfer_id: transfer_id("test:transfer:exchange-counter"),
            allocation: (&counter_leg).into(),
            destination: Some(account("store")),
        },
    };
    let [key_a, key_b] = terms.leg_operation_keys().expect("leg keys");
    let early = start() + SimDuration::minutes(1);
    let exchange_at = start() + SimDuration::minutes(2);
    let lease_a = lease(&mut canwu, GRANTEE, "exchange-a", &key_a, &forage, early);
    let lease_b = lease(
        &mut canwu,
        OUTSIDER,
        "exchange-b",
        &key_b,
        &counter_leg,
        exchange_at,
    );
    let current = state(&canwu);
    let exchange = ResourceExchangeStartRequestV1 {
        operation_key: terms.operation_key.clone(),
        leg_a: ResourceTransferStartRequestV1 {
            operation_key: key_a,
            transfer_id: terms.leg_a.transfer_id.clone(),
            allocation: terms.leg_a.allocation.clone(),
            expected_account_revision: current.accounts[&forage.account].revision,
            destination: terms.leg_a.destination.clone(),
            at: early,
            completion_certificate: lease_a.clone(),
        },
        leg_b: ResourceTransferStartRequestV1 {
            operation_key: key_b,
            transfer_id: terms.leg_b.transfer_id.clone(),
            allocation: terms.leg_b.allocation.clone(),
            expected_account_revision: current.accounts[&counter_leg.account].revision,
            destination: terms.leg_b.destination.clone(),
            at: exchange_at,
            completion_certificate: lease_b.clone(),
        },
    };
    let outcome = settled(
        &mut canwu,
        31,
        GRANTEE,
        exchange_at,
        ResourceOperationRequestV1::BeginExchange(exchange),
    );
    assert_rejected_outcome(&outcome, "invalid_authority", "certified lease time");
    assert_eq!(
        outcome.cited_transfers,
        vec![terms.leg_a.transfer_id, terms.leg_b.transfer_id]
    );
    assert_eq!(state(&canwu).transfers.len(), 1);
    assert_cap(&canwu, "main", 20, 60);
    release(&mut canwu, &lease_a.acquisition);
    release(&mut canwu, &lease_b.acquisition);

    // A plain granted transfer certified at one minute and settled at the
    // next, both inside the window, is rejected the same way.
    let certified = start() + SimDuration::minutes(3);
    let early_key = key("test:transfer:early");
    let early_lease = lease(&mut canwu, GRANTEE, "early", &early_key, &forage, certified);
    let request = begin_transfer(
        &canwu,
        "early",
        &forage,
        early_key,
        certified,
        early_lease.clone(),
    );
    let outcome = settled(
        &mut canwu,
        32,
        GRANTEE,
        certified + SimDuration::minutes(1),
        request,
    );
    assert_rejected_outcome(&outcome, "invalid_authority", "certified lease time");
    assert_cap(&canwu, "main", 20, 60);
    release(&mut canwu, &early_lease.acquisition);

    // Once the grant window closes, even the grantee's own lease cannot
    // debit what the grant reserved: an activated lease does not expire, so a
    // debit certified inside the window but settled after it is rejected.
    let after = start() + SimDuration::days(6);
    let certified = start() + SimDuration::minutes(5);
    let expired_key = key("test:transfer:expired");
    let expired_lease = lease(
        &mut canwu,
        GRANTEE,
        "expired",
        &expired_key,
        &forage,
        certified,
    );
    let request = begin_transfer(
        &canwu,
        "expired",
        &forage,
        expired_key,
        certified,
        expired_lease,
    );
    let outcome = settled(&mut canwu, 18, GRANTEE, after, request);
    assert_rejected_outcome(
        &outcome,
        "invalid_authority",
        "not current at the debit time",
    );
    assert_cap(&canwu, "main", 20, 60);

    // Cancelling the forage demand returns its reservation to the cap.
    let forage_demand = ResourceDemandId::new("test:demand:forage").expect("demand");
    let revision = state(&canwu).demands[&forage_demand].revision;
    let cancelled = settled(
        &mut canwu,
        19,
        GRANTEE,
        after,
        ResourceOperationRequestV1::CancelDemand(ResourceCancelDemandRequestV1 {
            operation_key: key("test:cancel:forage"),
            demand: forage_demand,
            expected_demand_revision: revision,
        }),
    );
    assert_eq!(cancelled.status, ResourceOperationStatus::Applied);
    assert_cap(&canwu, "main", 10, 60);

    // An allocation pass that names an in-window time but settles after the
    // window reserves nothing under the grant.
    let gleaning = demand("gleaning", GRANTEE, granted("main", &["west"]), 5, 5, 1);
    let outcome = settled(&mut canwu, 33, GRANTEE, after, submit(gleaning));
    assert_eq!(outcome.status, ResourceOperationStatus::Applied);
    let expected_state_revision = state(&canwu).state_revision;
    enqueue_resource_allocation(
        &mut canwu,
        after,
        &holder(GRANTEE),
        &ResourceAllocationRequestV1 {
            operation_key: key("test:allocate:stale"),
            expected_state_revision,
            at: start() + SimDuration::days(1),
            candidate_limit: 8,
        },
    )
    .expect("stale allocation ingress");
    settle(&mut canwu, after);
    let gleaning =
        &state(&canwu).demands[&ResourceDemandId::new("test:demand:gleaning").expect("id")];
    assert_eq!(gleaning.status, DemandStatus::RejectedMinimum);
    assert_eq!(
        gleaning.rejection_reason.as_deref(),
        Some("access_grant_unavailable")
    );
    assert_cap(&canwu, "main", 10, 60);

    // A host allocation pass after the demand window expires the requisition
    // demand and returns its remaining reservation; the debit stays charged.
    let expected_state_revision = state(&canwu).state_revision;
    enqueue_resource_allocation(
        &mut canwu,
        after,
        &holder(GRANTEE),
        &ResourceAllocationRequestV1 {
            operation_key: key("test:allocate:expiry"),
            expected_state_revision,
            at: after,
            candidate_limit: 8,
        },
    )
    .expect("expiry allocation ingress");
    settle(&mut canwu, after);
    assert_eq!(
        state(&canwu).demands[&ResourceDemandId::new("test:demand:requisition").expect("id")]
            .status,
        DemandStatus::Expired
    );
    assert_cap(&canwu, "main", 0, 60);

    // The grantee, not the grantor custodian, controls the granted transfer.
    let requisition = ResourceTransferId::new("test:transfer:requisition").expect("id");
    let cancel = |operation_key: &str, canwu: &Canwu| {
        ResourceOperationRequestV1::CancelTransfer(ResourceTransferCancellationRequestV1 {
            operation_key: key(operation_key),
            transfer: requisition.clone(),
            expected_transfer_revision: state(canwu).transfers[&requisition].revision,
            at: after,
        })
    };
    let request = cancel("test:cancel-transfer:grantor", &canwu);
    rejected(
        &mut canwu,
        20,
        GRANTOR,
        request,
        "does not control the actual target holder",
    );
    let request = cancel("test:cancel-transfer:grantee", &canwu);
    let outcome = settled(&mut canwu, 21, GRANTEE, after, request);
    assert_eq!(outcome.status, ResourceOperationStatus::Applied);
    assert_eq!(
        state(&canwu).transfers[&requisition].state,
        ResourceTransferState::ReturnPending
    );

    // Physical conservation is untouched by the grant accounting: the 60 in
    // escrow is still conserved outside account balances.
    let live = state(&canwu);
    live.validate().expect("granted state");
    // Validation rejects forged grant accounting and a transfer rebound away
    // from the grant its demand drew on.
    let forgeries: [fn(&mut ResourceState); 3] = [
        |forged| {
            forged
                .access_grants
                .get_mut(&grant_id("main"))
                .expect("grant")
                .debited_quantity = 0;
        },
        |forged| {
            forged
                .transfers
                .values_mut()
                .for_each(|transfer| transfer.access_grant = None);
        },
        |forged| {
            forged
                .transfers
                .values_mut()
                .for_each(|transfer| transfer.access_grant = Some(grant_id("spare")));
        },
    ];
    for forge in forgeries {
        let mut forged = live.clone();
        forge(&mut forged);
        forged.validate().expect_err("forged grant state");
    }
    assert_eq!(live.transfers[&requisition].escrow, 60);
    assert_eq!(live.accounts[&account("east")].balance, 0);
    assert_eq!(live.accounts[&account("west")].balance, 60);
    assert_eq!(live.accounts[&account("outsider")].balance, 500);
    assert_eq!(live.conservation.opening_balances, 620);

    // Save/load and exact replay reproduce the grants and their accounting.
    let snapshot = canwu.snapshot_json().expect("snapshot");
    let restored = from_resource_snapshot_json(&snapshot, &plugins).expect("strict restore");
    assert_eq!(state(&restored), live);
    assert_eq!(
        restored.snapshot_json().expect("restored snapshot"),
        snapshot
    );
    let replayed =
        replay_resource_from_journal(&plugins, &canwu.replay_journal()).expect("exact replay");
    assert_eq!(state(&replayed), live);
    assert_eq!(
        replayed.snapshot_json().expect("replayed snapshot"),
        snapshot
    );
}
