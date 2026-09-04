#![allow(clippy::manual_let_else, clippy::match_wildcard_for_single_variants)]
#![allow(clippy::similar_names, clippy::too_many_lines)]

use canwu_api::{
    BoundaryRequest, CommandEnvelope, CommandOutcome, CommandRequest, CommandRequestId,
    DomainRecord, DomainRecordClass, DomainRecordKind, DomainRecordLifecycle, DomainRecordRef,
    DomainRecordVersionRef, DomainRecordVersionSource, EntityRef, ErrorCode, EvidenceRef, Issuer,
    KnowledgeHolderRef, PersonId, Scenario, SimDuration, SimTime,
};
use canwu_resource::*;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

fn digest(value: char) -> String {
    value.to_string().repeat(64)
}

fn canonical_test_digest<T: serde::Serialize>(domain: &str, value: &T) -> String {
    let bytes = serde_json::to_vec(value).expect("canonical test encoding");
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain.as_bytes());
    hasher.update(&[0]);
    hasher.update(&bytes);
    hasher.finalize().to_hex().to_string()
}

fn holder(value: u64) -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(PersonId::new(value))
}

fn external_version(id: &str) -> DomainRecordVersionRef {
    DomainRecordVersionRef {
        record: DomainRecordRef::new("test.provider", "execution", id),
        version: 1,
        established_by: DomainRecordVersionSource::InitialScenario,
    }
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
    account: ResourceAccountId,
    destination: ResourceAccountId,
    resource: ResourceDefinitionRevisionId,
    unit: ResourceUnitRevisionId,
    floor: ProtectedFloorPolicyRevisionId,
    scope: ResourceScopeId,
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
        account,
        destination,
        resource,
        unit,
        floor: floor_id,
        scope,
    }
}

fn complete_external_participant(
    state: &mut ResourceState,
    suffix: &str,
) -> CompletionLeaseAcquisitionId {
    let acquisition =
        CompletionLeaseAcquisitionId::new(format!("test:external-completion:{suffix}"))
            .expect("acquisition");
    let operation_key = ResourceOperationKey::new(format!("test:external-operation:{suffix}"))
        .expect("operation key");
    let coordinator_source = external_version(&format!("external-completion-{suffix}"));
    let target = CompletionLockedTargetV1::ExternalRecord {
        version: coordinator_source.clone(),
    };
    let recipe = CompletionCapacityRecipeV1 {
        receipts: MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE,
        mutations: 1,
        reports_per_holder: 0,
        holders: 0,
        bytes: 1_024,
    };
    let envelope_digest = digest('e');
    state
        .apply_operation(&ResourceOperationRequestV1::Completion(
            ResourceCompletionOperationV1::GrantExternalParticipant(
                RequestExternalCompletionParticipantGrantV1 {
                    coordinator_plugin: "test.provider".to_owned(),
                    coordinator_source: coordinator_source.clone(),
                    coordinator_acquisition_revision: ResourceRevision::INITIAL,
                    acquisition: acquisition.clone(),
                    operation_key: operation_key.clone(),
                    holder: holder(1),
                    operation_namespace: "test.resource".to_owned(),
                    eligibility_time: SimTime::EPOCH,
                    eligibility_envelope_digest: envelope_digest.clone(),
                    recipe: recipe.clone(),
                    policy_class: CompletionPolicyClassV1::Guaranteed,
                    grant_id: CompletionCapacityGrantId::new(format!(
                        "test:external-grant:{suffix}"
                    ))
                    .expect("grant"),
                    target_versions: vec![target.clone()],
                    current_boundary: 1,
                },
            ),
        ))
        .expect("grant external participant");
    let held = state.external_completion_participants.grants[&acquisition]
        .grant
        .clone();
    state
        .apply_operation(&ResourceOperationRequestV1::Completion(
            ResourceCompletionOperationV1::PrepareExternalParticipant(
                PrepareExternalCompletionParticipantGrantV1 {
                    coordinator_source: coordinator_source.clone(),
                    acquisition: acquisition.clone(),
                    expected_grant_revision: held.revision,
                    current_boundary: 2,
                    eligibility_envelope_digest: envelope_digest.clone(),
                },
            ),
        ))
        .expect("prepare external participant");
    let prepared = state.external_completion_participants.grants[&acquisition]
        .grant
        .clone();
    let mut certificate = CompletionLeaseActivationCertificateV1 {
        acquisition: acquisition.clone(),
        acquisition_revision: ResourceRevision::INITIAL,
        operation_key: operation_key.clone(),
        prepared_grants: vec![(prepared.id.clone(), prepared.revision)],
        locked_target_versions: vec![target],
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
    state
        .apply_operation(&ResourceOperationRequestV1::Completion(
            ResourceCompletionOperationV1::ConsumeExternalParticipant(
                ConsumeExternalCompletionParticipantGrantV1 {
                    coordinator_source,
                    certificate,
                    at: SimTime::EPOCH,
                },
            ),
        ))
        .expect("consume external participant");
    state
        .apply_operation(&ResourceOperationRequestV1::Completion(
            ResourceCompletionOperationV1::CompleteExternalParticipant(
                CompleteExternalCompletionParticipantGrantV1 {
                    acquisition: acquisition.clone(),
                    operation_key,
                },
            ),
        ))
        .expect("complete external participant");
    acquisition
}

fn demand(
    id: &str,
    requested: u64,
    minimum: u64,
    partial: PartialFulfillmentPolicy,
    fixture: &Fixture,
    override_class: Option<&str>,
) -> ResourceDemand {
    ResourceDemand {
        id: ResourceDemandId::new(id).expect("demand"),
        revision: ResourceRevision::INITIAL,
        requester: holder(1),
        resource_revision: fixture.resource.clone(),
        unit_revision: fixture.unit.clone(),
        requested,
        fulfilled: 0,
        minimum_useful: minimum,
        partial_fulfillment: partial,
        alternative_group: None,
        due_at: SimTime::EPOCH,
        expires_at: SimTime::EPOCH + SimDuration::days(10),
        priority: 10,
        tie_break: ResourceTieBreakKey::new(format!("test:tie:{id}")).expect("tie"),
        admitted_sequence: 0,
        protected_floor_policy: Some(fixture.floor.clone()),
        protection_override_class: override_class.map(str::to_owned),
        status: DemandStatus::Open,
        rejection_reason: None,
    }
}

fn allocate(state: &mut ResourceState, key: &str) -> ResourceOperationOutcome {
    let revision = state.state_revision;
    state
        .apply_operation(&ResourceOperationRequestV1::Allocate(
            ResourceAllocationRequestV1 {
                operation_key: ResourceOperationKey::new(key).expect("key"),
                expected_state_revision: revision,
                at: SimTime::EPOCH,
                candidate_limit: 32,
            },
        ))
        .expect("allocate")
}

fn only_leg(state: &ResourceState) -> ResourceAllocationLegVersionV1 {
    state
        .allocation_legs
        .values()
        .next()
        .map(Into::into)
        .expect("allocation leg")
}

fn activate_certificate(
    state: &mut ResourceState,
    suffix: &str,
    operation_key: ResourceOperationKey,
    targets: Vec<CompletionLockedTargetV1>,
    at: SimTime,
    boundary: u64,
) -> CompletionLeaseActivationCertificateV1 {
    let acquisition =
        CompletionLeaseAcquisitionId::new(format!("test:lease:{suffix}")).expect("acquisition");
    let acquire = RequestCompletionLeaseV1 {
        id: acquisition.clone(),
        operation_key,
        holder: holder(1),
        operation_namespace: "test.resource".to_owned(),
        eligibility_time: at,
        eligibility_envelope: EligibilityEnvelopeV1::new(
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
    };
    let outcome = state
        .apply_operation(&ResourceOperationRequestV1::Completion(
            ResourceCompletionOperationV1::Acquire(acquire),
        ))
        .expect("acquire");
    assert_eq!(outcome.status, ResourceOperationStatus::Applied);
    let grant_id =
        CompletionCapacityGrantId::new(format!("test:grant:{suffix}")).expect("grant id");
    let acquisition_revision = state.completion_leases.acquisitions[&acquisition].revision;
    assert_eq!(
        state
            .apply_operation(&ResourceOperationRequestV1::Completion(
                ResourceCompletionOperationV1::Grant(GrantCompletionCapacityV1 {
                    grant_id: grant_id.clone(),
                    acquisition: acquisition.clone(),
                    expected_acquisition_revision: acquisition_revision,
                    owner_plugin: PLUGIN_NAME.to_owned(),
                    target_versions: targets,
                    current_boundary: boundary,
                }),
            ))
            .expect("grant")
            .status,
        ResourceOperationStatus::Applied
    );
    let envelope_digest = state.completion_leases.acquisitions[&acquisition]
        .eligibility_envelope
        .digest
        .clone();
    assert_eq!(
        state
            .apply_operation(&ResourceOperationRequestV1::Completion(
                ResourceCompletionOperationV1::Prepare(PrepareCompletionCapacityV1 {
                    acquisition: acquisition.clone(),
                    expected_acquisition_revision: state.completion_leases.acquisitions
                        [&acquisition]
                        .revision,
                    grant: grant_id.clone(),
                    expected_grant_revision: state.completion_leases.grants[&grant_id].revision,
                    current_boundary: boundary + 1,
                    eligibility_envelope_digest: envelope_digest.clone(),
                }),
            ))
            .expect("prepare")
            .status,
        ResourceOperationStatus::Applied
    );
    assert_eq!(
        state
            .apply_operation(&ResourceOperationRequestV1::Completion(
                ResourceCompletionOperationV1::Activate(ActivateCompletionLeaseV1 {
                    acquisition: acquisition.clone(),
                    expected_acquisition_revision: state.completion_leases.acquisitions
                        [&acquisition]
                        .revision,
                    grant: grant_id.clone(),
                    expected_grant_revision: state.completion_leases.grants[&grant_id].revision,
                    at,
                    current_boundary: boundary + 2,
                    eligibility_envelope_digest: envelope_digest,
                }),
            ))
            .expect("activate")
            .status,
        ResourceOperationStatus::Applied
    );
    state.completion_leases.certificates[&acquisition].clone()
}

#[test]
fn conservation_partial_minimum_and_protected_floor_are_enforced() {
    let mut partial = fixture(100, 60);
    partial
        .state
        .install_demand(demand(
            "test:demand:partial",
            100,
            30,
            PartialFulfillmentPolicy::AcceptPartial,
            &partial,
            None,
        ))
        .expect("demand");
    assert_eq!(
        allocate(&mut partial.state, "test:allocate:partial").quantity,
        40
    );
    let quantities = partial
        .state
        .account_quantities(&partial.account)
        .expect("quantities");
    assert_eq!(
        (
            quantities.available,
            quantities.reserved,
            quantities.protected
        ),
        (0, 40, 60)
    );
    // The acquisition reserves its full bounded terminal path. Once activated,
    // unrelated new work sees archive backpressure, while the already-leased
    // debit below can still consume the reserved receipt slots.
    partial.state.limits.max_archive_candidates = 19;
    let allocation = only_leg(&partial.state);
    let leg = partial.state.allocation_legs[&allocation.id].clone();
    let consumer = external_version("partial-consumer");
    let key = ResourceOperationKey::new("test:consume:partial").expect("key");
    let certificate = activate_certificate(
        &mut partial.state,
        "consume-partial",
        key.clone(),
        vec![
            CompletionLockedTargetV1::Account {
                id: partial.account.clone(),
                revision: ResourceRevision::INITIAL,
            },
            CompletionLockedTargetV1::AllocationLeg {
                id: allocation.id.clone(),
                revision: allocation.revision,
            },
            CompletionLockedTargetV1::Demand {
                id: leg.demand,
                revision: leg.demand_revision,
            },
            CompletionLockedTargetV1::ExternalRecord {
                version: consumer.clone(),
            },
        ],
        SimTime::EPOCH,
        10,
    );
    assert!(partial.state.continuation.active);
    assert!(
        partial
            .state
            .continuation
            .dependencies
            .contains(&EvidenceRef::DomainRecordVersion(consumer.clone()))
    );
    assert!(matches!(
        partial
            .state
            .apply_operation(&ResourceOperationRequestV1::CancelDemand(
                ResourceCancelDemandRequestV1 {
                    operation_key: ResourceOperationKey::new("test:blocked:new-work").expect("key"),
                    demand: ResourceDemandId::new("test:missing:blocked").expect("demand"),
                    expected_demand_revision: ResourceRevision::INITIAL,
                },
            )),
        Err(ResourceError::LimitExceeded(_))
    ));
    let outcome = partial
        .state
        .apply_operation(&ResourceOperationRequestV1::Consume(
            ResourceConsumptionRequestV1 {
                operation_key: key,
                consumption_id: ResourceConsumptionId::new("test:consumption:partial")
                    .expect("consumption"),
                allocation,
                expected_account_revision: ResourceRevision::INITIAL,
                consumer_evidence: consumer.clone(),
                at: SimTime::EPOCH,
                completion_certificate: certificate,
            },
        ))
        .expect("consume");
    assert_eq!((outcome.quantity, outcome.remainder), (40, 60));
    assert!(!partial.state.continuation.active);
    assert_eq!(
        partial
            .state
            .consumptions
            .values()
            .next()
            .expect("terminal consumption receipt")
            .consumer_evidence,
        consumer
    );
    assert_eq!(partial.state.accounts[&partial.account].balance, 60);
    partial.state.validate().expect("valid state");

    let mut minimum = fixture(30, 0);
    minimum
        .state
        .install_demand(demand(
            "test:demand:minimum",
            50,
            40,
            PartialFulfillmentPolicy::AcceptPartial,
            &minimum,
            None,
        ))
        .expect("minimum demand");
    assert_eq!(
        allocate(&mut minimum.state, "test:allocate:minimum").quantity,
        0
    );
    assert_eq!(
        minimum
            .state
            .demands
            .values()
            .next()
            .expect("demand")
            .status,
        DemandStatus::RejectedMinimum
    );
    minimum.state.validate().expect("minimum state");

    let mut emergency = fixture(100, 60);
    emergency
        .state
        .install_demand(demand(
            "test:demand:emergency",
            80,
            50,
            PartialFulfillmentPolicy::AcceptPartial,
            &emergency,
            Some("emergency"),
        ))
        .expect("emergency demand");
    assert_eq!(
        allocate(&mut emergency.state, "test:allocate:emergency").quantity,
        80
    );
}

fn transfer_fixture(suffix: &str) -> (Fixture, ResourceAllocationLegVersionV1) {
    let mut value = fixture(100, 0);
    value
        .state
        .install_demand(demand(
            &format!("test:demand:{suffix}"),
            30,
            30,
            PartialFulfillmentPolicy::RejectPartial,
            &value,
            None,
        ))
        .expect("transfer demand");
    allocate(&mut value.state, &format!("test:allocate:{suffix}"));
    let leg = only_leg(&value.state);
    (value, leg)
}

fn start_transfer(
    value: &mut Fixture,
    leg: ResourceAllocationLegVersionV1,
    suffix: &str,
    destination: Option<ResourceAccountId>,
) -> ResourceTransferId {
    let key = ResourceOperationKey::new(format!("test:transfer-start:{suffix}")).expect("key");
    let exact = value.state.allocation_legs[&leg.id].clone();
    let certificate = activate_certificate(
        &mut value.state,
        &format!("transfer-start-{suffix}"),
        key.clone(),
        vec![
            CompletionLockedTargetV1::Account {
                id: value.account.clone(),
                revision: ResourceRevision::INITIAL,
            },
            CompletionLockedTargetV1::AllocationLeg {
                id: leg.id.clone(),
                revision: leg.revision,
            },
            CompletionLockedTargetV1::Demand {
                id: exact.demand,
                revision: exact.demand_revision,
            },
        ],
        SimTime::EPOCH,
        20,
    );
    let id = ResourceTransferId::new(format!("test:transfer:{suffix}")).expect("transfer");
    assert_eq!(
        value
            .state
            .apply_operation(&ResourceOperationRequestV1::BeginTransfer(
                ResourceTransferStartRequestV1 {
                    operation_key: key,
                    transfer_id: id.clone(),
                    allocation: leg,
                    expected_account_revision: ResourceRevision::INITIAL,
                    destination,
                    at: SimTime::EPOCH,
                    completion_certificate: certificate,
                },
            ))
            .expect("start transfer")
            .status,
        ResourceOperationStatus::Applied
    );
    id
}

fn advance_transfer(
    value: &mut Fixture,
    transfer: &ResourceTransferId,
    suffix: &str,
    progress: TransferProgressV1,
) {
    let revision = value.state.transfers[transfer].revision;
    let evidence = external_version(&format!("transport-{suffix}"));
    let outcome = value
        .state
        .apply_operation(&ResourceOperationRequestV1::AdvanceTransfer(
            ResourceTransferProgressRequestV1 {
                operation_key: ResourceOperationKey::new(format!("test:advance:{suffix}"))
                    .expect("key"),
                transfer: transfer.clone(),
                expected_transfer_revision: revision,
                progress,
                transport: TransportExecutionLink {
                    execution: canwu_api::TransportExecutionId(1),
                    itinerary_revision: canwu_api::ItineraryRevisionId(1),
                    leg_execution: None,
                    handoff: None,
                    capacity_booking: None,
                },
                transport_evidence: evidence.clone(),
            },
        ))
        .expect("advance");
    assert_eq!(outcome.exact_evidence, vec![evidence]);
    assert!(value.state.continuation.active);
    assert!(
        value
            .state
            .continuation
            .dependencies
            .contains(&EvidenceRef::DomainRecordVersion(
                outcome.exact_evidence[0].clone(),
            ))
    );
}

fn terminal_certificate(
    value: &mut Fixture,
    transfer: &ResourceTransferId,
    suffix: &str,
    key: &ResourceOperationKey,
    mut targets: Vec<CompletionLockedTargetV1>,
) -> CompletionLeaseActivationCertificateV1 {
    targets.push(CompletionLockedTargetV1::Transfer {
        id: transfer.clone(),
        revision: value.state.transfers[transfer].revision,
    });
    activate_certificate(
        &mut value.state,
        suffix,
        key.clone(),
        targets,
        SimTime::EPOCH + SimDuration::minutes(1),
        40,
    )
}

#[test]
fn transfer_loss_return_duplicate_and_exact_acceptance_preserve_escrow() {
    let (mut lost, leg) = transfer_fixture("lost");
    let transfer = start_transfer(&mut lost, leg, "lost", None);
    advance_transfer(&mut lost, &transfer, "lost", TransferProgressV1::InTransit);
    let key = ResourceOperationKey::new("test:transfer-loss").expect("key");
    let cause = external_version("loss-cause");
    let certificate = terminal_certificate(
        &mut lost,
        &transfer,
        "loss-terminal",
        &key,
        vec![CompletionLockedTargetV1::ExternalRecord {
            version: cause.clone(),
        }],
    );
    let request =
        ResourceOperationRequestV1::CompleteTransfer(ResourceTransferDispositionRequestV1 {
            operation_key: key,
            transfer: transfer.clone(),
            expected_transfer_revision: lost.state.transfers[&transfer].revision,
            at: SimTime::EPOCH + SimDuration::minutes(1),
            disposition: ResourceTransferDispositionV1::Lose {
                loss_id: ResourceLossId::new("test:loss:transfer").expect("loss"),
                cause: EvidenceRef::DomainRecordVersion(cause),
            },
            exact_transport_evidence: None,
            completion_certificate: certificate,
        });
    let first = lost.state.apply_operation(&request).expect("loss");
    assert_eq!(
        first,
        lost.state.apply_operation(&request).expect("duplicate")
    );
    assert_eq!(lost.state.accounts[&lost.account].balance, 70);
    assert_eq!(lost.state.transfers[&transfer].escrow, 0);
    lost.state.validate().expect("loss state");

    let (mut returned, leg) = transfer_fixture("returned");
    let transfer = start_transfer(&mut returned, leg, "returned", None);
    advance_transfer(
        &mut returned,
        &transfer,
        "return",
        TransferProgressV1::ReturnPending,
    );
    let key = ResourceOperationKey::new("test:transfer-return").expect("key");
    let expected_source_revision = returned.state.accounts[&returned.account].revision;
    let source_account = returned.account.clone();
    let certificate = terminal_certificate(
        &mut returned,
        &transfer,
        "return-terminal",
        &key,
        vec![CompletionLockedTargetV1::Account {
            id: source_account,
            revision: expected_source_revision,
        }],
    );
    let transfer_revision = returned.state.transfers[&transfer].revision;
    returned
        .state
        .apply_operation(&ResourceOperationRequestV1::CompleteTransfer(
            ResourceTransferDispositionRequestV1 {
                operation_key: key,
                transfer: transfer.clone(),
                expected_transfer_revision: transfer_revision,
                at: SimTime::EPOCH + SimDuration::minutes(1),
                disposition: ResourceTransferDispositionV1::Return {
                    expected_source_revision,
                },
                exact_transport_evidence: None,
                completion_certificate: certificate,
            },
        ))
        .expect("return");
    assert_eq!(returned.state.accounts[&returned.account].balance, 100);
    returned.state.validate().expect("return state");

    let (mut accepted, leg) = transfer_fixture("accept");
    let destination = accepted.destination.clone();
    let transfer = start_transfer(&mut accepted, leg, "accept", Some(destination.clone()));
    advance_transfer(
        &mut accepted,
        &transfer,
        "accept-1",
        TransferProgressV1::InTransit,
    );
    advance_transfer(
        &mut accepted,
        &transfer,
        "accept-2",
        TransferProgressV1::ArrivalPending,
    );
    let evidence = external_version("acceptance");
    let key = ResourceOperationKey::new("test:transfer-accept-forged").expect("key");
    let certificate = terminal_certificate(
        &mut accepted,
        &transfer,
        "accept-terminal",
        &key,
        vec![
            CompletionLockedTargetV1::Account {
                id: destination.clone(),
                revision: ResourceRevision::INITIAL,
            },
            CompletionLockedTargetV1::ExternalRecord {
                version: evidence.clone(),
            },
        ],
    );
    let acceptance = ResourceTransportAcceptanceV1 {
        evidence,
        execution: accepted.state.transfers[&transfer]
            .transport
            .clone()
            .expect("transport"),
        destination: destination.clone(),
        quantity: 30,
        accepted_at: SimTime::EPOCH + SimDuration::minutes(1),
        semantic_digest: String::new(),
    }
    .seal()
    .expect("acceptance");
    let transfer_revision = accepted.state.transfers[&transfer].revision;
    let rejected = accepted
        .state
        .apply_operation(&ResourceOperationRequestV1::CompleteTransfer(
            ResourceTransferDispositionRequestV1 {
                operation_key: key,
                transfer,
                expected_transfer_revision: transfer_revision,
                at: SimTime::EPOCH + SimDuration::minutes(1),
                disposition: ResourceTransferDispositionV1::Accept {
                    destination: destination.clone(),
                    expected_destination_revision: ResourceRevision::INITIAL,
                    acceptance,
                },
                exact_transport_evidence: None,
                completion_certificate: certificate,
            },
        ))
        .expect("durable rejection");
    assert_eq!(rejected.status, ResourceOperationStatus::Rejected);
    assert_eq!(accepted.state.accounts[&destination].balance, 0);
}

#[test]
fn transfer_cancellation_preserves_escrow_until_the_reserved_return_settles() {
    let (mut value, leg) = transfer_fixture("cancelled");
    let transfer = start_transfer(&mut value, leg, "cancelled", None);
    advance_transfer(
        &mut value,
        &transfer,
        "cancelled-in-transit",
        TransferProgressV1::InTransit,
    );
    let cancellation =
        ResourceOperationRequestV1::CancelTransfer(ResourceTransferCancellationRequestV1 {
            operation_key: ResourceOperationKey::new("test:transfer-cancel").expect("key"),
            transfer: transfer.clone(),
            expected_transfer_revision: value.state.transfers[&transfer].revision,
            at: SimTime::EPOCH,
        });
    let first = value
        .state
        .apply_operation(&cancellation)
        .expect("cancel transfer");
    assert_eq!(
        first,
        value
            .state
            .apply_operation(&cancellation)
            .expect("duplicate cancellation")
    );
    assert_eq!(
        value.state.transfers[&transfer].state,
        ResourceTransferState::ReturnPending
    );
    assert_eq!(value.state.transfers[&transfer].escrow, 30);
    assert_eq!(value.state.accounts[&value.account].balance, 70);

    let key = ResourceOperationKey::new("test:cancelled-transfer-return").expect("key");
    let source_revision = value.state.accounts[&value.account].revision;
    let account = value.account.clone();
    let certificate = terminal_certificate(
        &mut value,
        &transfer,
        "cancelled-transfer-return",
        &key,
        vec![CompletionLockedTargetV1::Account {
            id: account,
            revision: source_revision,
        }],
    );
    value
        .state
        .apply_operation(&ResourceOperationRequestV1::CompleteTransfer(
            ResourceTransferDispositionRequestV1 {
                operation_key: key,
                transfer: transfer.clone(),
                expected_transfer_revision: value.state.transfers[&transfer].revision,
                at: SimTime::EPOCH + SimDuration::minutes(1),
                disposition: ResourceTransferDispositionV1::Return {
                    expected_source_revision: source_revision,
                },
                exact_transport_evidence: None,
                completion_certificate: certificate,
            },
        ))
        .expect("settle cancelled return");
    assert_eq!(value.state.accounts[&value.account].balance, 100);
    assert_eq!(value.state.transfers[&transfer].escrow, 0);
    assert_eq!(
        value.state.transfers[&transfer].state,
        ResourceTransferState::Returned
    );
    value.state.validate().expect("cancelled transfer state");
}

#[test]
fn admitted_transfer_terminal_work_keeps_hot_outcome_capacity_reserved() {
    let (mut value, leg) = transfer_fixture("outcome-reserve");
    value.state.limits.max_operation_outcomes = 40;
    let transfer = start_transfer(&mut value, leg, "outcome-reserve", None);
    advance_transfer(
        &mut value,
        &transfer,
        "outcome-reserve-return",
        TransferProgressV1::ReturnPending,
    );
    let key = ResourceOperationKey::new("test:outcome-reserve-return").expect("key");
    let source_revision = value.state.accounts[&value.account].revision;
    let account = value.account.clone();
    let certificate = terminal_certificate(
        &mut value,
        &transfer,
        "outcome-reserve-terminal",
        &key,
        vec![CompletionLockedTargetV1::Account {
            id: account,
            revision: source_revision,
        }],
    );
    let blocked = value
        .state
        .apply_operation(&ResourceOperationRequestV1::CreateAccount(
            ResourceCreateAccountRequestV1 {
                operation_key: ResourceOperationKey::new("test:outcome-reserve-unrelated")
                    .expect("key"),
                account: ResourceAccount {
                    id: ResourceAccountId::new("test:account:unrelated").expect("account"),
                    revision: ResourceRevision::INITIAL,
                    custodian: holder(1),
                    resource_revision: value.resource.clone(),
                    unit_revision: value.unit.clone(),
                    balance: 0,
                    capacity: Some(1),
                    protected_floor_policy: None,
                    closed: false,
                },
            },
        ));
    assert!(matches!(blocked, Err(ResourceError::LimitExceeded(_))));

    value
        .state
        .apply_operation(&ResourceOperationRequestV1::CompleteTransfer(
            ResourceTransferDispositionRequestV1 {
                operation_key: key,
                transfer: transfer.clone(),
                expected_transfer_revision: value.state.transfers[&transfer].revision,
                at: SimTime::EPOCH + SimDuration::minutes(1),
                disposition: ResourceTransferDispositionV1::Return {
                    expected_source_revision: source_revision,
                },
                exact_transport_evidence: None,
                completion_certificate: certificate,
            },
        ))
        .expect("reserved terminal outcome must settle");
    assert_eq!(value.state.accounts[&value.account].balance, 100);
}

#[test]
fn named_terminal_report_reservation_publishes_the_last_real_hot_slot() {
    let mut value = fixture(10, 0);
    let report_source = external_version("full-cap-report-source");
    let observation_revision = value.state.state_revision;
    for ordinal in 0..ResourceLimitsV1::MAX_HOLDERS {
        let grant = ResourceReportGrantId::new(format!("test:report-grant:full-cap:{ordinal:04}"))
            .expect("report grant");
        value
            .state
            .install_report_grant(ResourceReportGrantV1 {
                id: grant.clone(),
                holder: holder(1),
                scope: value.scope.clone(),
                accounts: BTreeSet::new(),
                demands: BTreeSet::new(),
                include_transfer_details: false,
                confidence_per_mille: 1_000,
                cadence_minutes: 1,
                delay_minutes: 0,
            })
            .expect("install report grant");
        value
            .state
            .record_observation_head(
                ResourceObservationHeadV1 {
                    id: ResourceObservationHeadId::new(format!(
                        "test:observation:full-cap:{ordinal:04}"
                    ))
                    .expect("observation"),
                    revision: ResourceRevision::INITIAL,
                    provider_plugin: "test-provider".to_owned(),
                    provider_version: "1".to_owned(),
                    provider_semantic_hash: digest('f'),
                    provider_source: report_source.clone(),
                    holder: holder(1),
                    grant,
                    provider_state_revision: observation_revision,
                    observed_at: SimTime::EPOCH,
                    confidence_per_mille: 1_000,
                    stock: Vec::new(),
                    demands: Vec::new(),
                    allocations: Vec::new(),
                    fulfillments: Vec::new(),
                    transfers: Vec::new(),
                    consumptions: Vec::new(),
                    source_versions: vec![report_source.clone()],
                    semantic_digest: String::new(),
                }
                .seal()
                .expect("seal observation"),
            )
            .expect("record observation");
    }
    let acquisition =
        CompletionLeaseAcquisitionId::new("test:lease:full-cap-terminal-report").expect("lease");
    let operation_key =
        ResourceOperationKey::new("test:operation:full-cap-terminal-report").expect("operation");
    let completion_grant =
        CompletionCapacityGrantId::new("test:grant:full-cap-terminal-report").expect("grant");
    let envelope = EligibilityEnvelopeV1::new(
        vec![report_source.clone()],
        BTreeMap::new(),
        BTreeSet::new(),
        Vec::new(),
        Vec::new(),
    )
    .expect("envelope");
    let recipe = CompletionCapacityRecipeV1 {
        receipts: MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE,
        mutations: 1,
        reports_per_holder: 1,
        holders: 1,
        bytes: 1_024,
    };
    value
        .state
        .apply_operation(&ResourceOperationRequestV1::Completion(
            ResourceCompletionOperationV1::GrantExternalParticipant(
                RequestExternalCompletionParticipantGrantV1 {
                    coordinator_plugin: "test-provider".to_owned(),
                    coordinator_source: report_source.clone(),
                    coordinator_acquisition_revision: ResourceRevision::INITIAL,
                    acquisition: acquisition.clone(),
                    operation_key: operation_key.clone(),
                    holder: holder(1),
                    operation_namespace: "test.resource".to_owned(),
                    eligibility_time: SimTime::EPOCH,
                    eligibility_envelope_digest: envelope.digest.clone(),
                    recipe: recipe.clone(),
                    policy_class: CompletionPolicyClassV1::Guaranteed,
                    grant_id: completion_grant.clone(),
                    target_versions: vec![CompletionLockedTargetV1::Account {
                        id: value.account.clone(),
                        revision: ResourceRevision::INITIAL,
                    }],
                    current_boundary: 1,
                },
            ),
        ))
        .expect("reserve named report");
    let reserved_report = value.state.completion_report_reservations[&acquisition]
        .iter()
        .next()
        .expect("named report reservation")
        .clone();
    value
        .state
        .apply_operation(&ResourceOperationRequestV1::Completion(
            ResourceCompletionOperationV1::PrepareExternalParticipant(
                PrepareExternalCompletionParticipantGrantV1 {
                    coordinator_source: report_source.clone(),
                    acquisition: acquisition.clone(),
                    expected_grant_revision: value.state.external_completion_participants.grants
                        [&acquisition]
                        .grant
                        .revision,
                    current_boundary: 2,
                    eligibility_envelope_digest: envelope.digest.clone(),
                },
            ),
        ))
        .expect("prepare terminal participant");
    let prepared = value.state.external_completion_participants.grants[&acquisition]
        .grant
        .clone();
    let mut certificate = CompletionLeaseActivationCertificateV1 {
        acquisition: acquisition.clone(),
        acquisition_revision: ResourceRevision::INITIAL,
        operation_key: operation_key.clone(),
        prepared_grants: vec![(prepared.id.clone(), prepared.revision)],
        locked_target_versions: prepared.target_versions.clone(),
        recipe_digest: recipe.digest().expect("recipe digest"),
        eligibility_time: SimTime::EPOCH,
        eligibility_envelope_digest: envelope.digest,
        activation_boundary: 3,
        semantic_digest: String::new(),
    };
    certificate.semantic_digest = canonical_test_digest(
        "canwu.resource.completion-activation-certificate.v1",
        &certificate,
    );
    value
        .state
        .apply_operation(&ResourceOperationRequestV1::Completion(
            ResourceCompletionOperationV1::ConsumeExternalParticipant(
                ConsumeExternalCompletionParticipantGrantV1 {
                    coordinator_source: report_source.clone(),
                    certificate,
                    at: SimTime::EPOCH,
                },
            ),
        ))
        .expect("consume terminal participant");
    value
        .state
        .apply_operation(&ResourceOperationRequestV1::Completion(
            ResourceCompletionOperationV1::CompleteExternalParticipant(
                CompleteExternalCompletionParticipantGrantV1 {
                    acquisition: acquisition.clone(),
                    operation_key,
                },
            ),
        ))
        .expect("complete terminal participant");
    let head_id = value.state.observation_head_by_grant[&reserved_report].clone();
    let mut terminal_head = value.state.observation_heads[&head_id].clone();
    terminal_head.revision = terminal_head.revision.next().expect("head revision");
    terminal_head.provider_state_revision = value.state.state_revision;
    terminal_head.semantic_digest.clear();
    value
        .state
        .record_observation_head(terminal_head.seal().expect("terminal head"))
        .expect("record terminal observation");
    value.state.report_dirty_grants.remove(&reserved_report);
    value
        .state
        .report_due_index
        .entry(32)
        .or_default()
        .insert(reserved_report.clone());
    value.state.validate().expect("full-cap source state");

    let provider = DomainRecord {
        reference: report_source.record,
        owner: "test-provider".to_owned(),
        class: DomainRecordClass::Record,
        version: 1,
        lifecycle: DomainRecordLifecycle::Active,
        payload: serde_json::json!({"report": "full-cap-source"}),
        references: Vec::new(),
    };
    let scenario = Scenario::new(SimTime::EPOCH, vec![EntityRef::Person(PersonId::new(1))])
        .with_domain_records(vec![provider, value.state.into_record().expect("record")]);
    let mut canwu = canwu_api::Canwu::new_with_plugins(41, scenario, &[&ResourcePlugin::default()])
        .expect("resource runtime");
    enqueue_resource_completion_operation(
        &mut canwu,
        SimTime::EPOCH,
        &ResourceCompletionOperationV1::Expire(ExpireCompletionCapacityV1 {
            at: SimTime::EPOCH,
            current_boundary: 0,
            candidate_limit: 1,
        }),
    )
    .expect("initial report wake boundary");
    for minute in 0..32 {
        canwu
            .settle_boundary(BoundaryRequest::at(
                SimTime::EPOCH + SimDuration::minutes(minute),
            ))
            .expect("fill ordinary report capacity");
    }
    assert_eq!(
        canwu
            .knowledge()
            .for_holder(&holder(1))
            .expect("ordinary reports")
            .len(),
        8_191
    );
    canwu
        .settle_boundary(BoundaryRequest::at(
            SimTime::EPOCH + SimDuration::minutes(32),
        ))
        .expect("publish mandatory terminal report");
    let reports = canwu
        .knowledge()
        .for_holder(&holder(1))
        .expect("full report ledger");
    assert_eq!(reports.len(), 8_192);
    assert!(reports.values().any(|record| {
        serde_json::from_value::<ResourceReportDtoV1>(record.payload.clone())
            .is_ok_and(|report| report.grant == reserved_report)
    }));
}

#[test]
fn holder_reports_use_persisted_authoritative_scope_and_reject_relabeling() {
    let mut value = fixture(75, 10);
    let grant_id = ResourceReportGrantId::new("test:report-grant:custodian").expect("grant");
    value
        .state
        .install_report_grant(ResourceReportGrantV1 {
            id: grant_id.clone(),
            holder: holder(1),
            scope: value.scope.clone(),
            accounts: BTreeSet::from([value.account.clone()]),
            demands: BTreeSet::new(),
            include_transfer_details: false,
            confidence_per_mille: 900,
            cadence_minutes: 60,
            delay_minutes: 0,
        })
        .expect("grant");
    let source = external_version("observation");
    let head = ResourceObservationHeadV1 {
        id: ResourceObservationHeadId::new("test:observation:holder").expect("head"),
        revision: ResourceRevision::INITIAL,
        provider_plugin: "test-provider".to_owned(),
        provider_version: "1.0.0".to_owned(),
        provider_semantic_hash: digest('a'),
        provider_source: source.clone(),
        holder: holder(1),
        grant: grant_id.clone(),
        provider_state_revision: ResourceRevision::new(7).expect("revision"),
        observed_at: SimTime::EPOCH,
        confidence_per_mille: 800,
        stock: vec![ResourceStockObservationV1 {
            account: value.account.clone(),
            scope: value.scope.clone(),
            known_minimum: 20,
            known_maximum: 30,
            reserved: 2,
            protected: 10,
        }],
        demands: Vec::new(),
        allocations: Vec::new(),
        fulfillments: Vec::new(),
        transfers: Vec::new(),
        consumptions: Vec::new(),
        source_versions: vec![source.clone()],
        semantic_digest: String::new(),
    }
    .seal()
    .expect("seal head");
    value
        .state
        .record_observation_head(head.clone())
        .expect("record head");
    let report = materialize_resource_report(
        &value.state,
        &holder(1),
        &grant_id,
        SimTime::EPOCH,
        SimTime::EPOCH + SimDuration::days(1),
    )
    .expect("report");
    assert_eq!(report.stock[0].known_maximum, 30);
    assert_eq!(report.stock[0].scope, value.scope);
    assert!(report.stale);
    assert!(
        materialize_resource_report(
            &value.state,
            &holder(2),
            &grant_id,
            SimTime::EPOCH,
            SimTime::EPOCH,
        )
        .is_err()
    );
    let mut forged = head;
    forged.revision = ResourceRevision::new(2).expect("revision");
    forged.stock[0].scope = ResourceScopeId::new("test:scope:distant").expect("scope");
    forged.semantic_digest.clear();
    forged = forged.seal().expect("forge digest");
    assert!(value.state.record_observation_head(forged).is_err());

    let provider = DomainRecord {
        reference: source.record.clone(),
        owner: "test-provider".to_owned(),
        class: DomainRecordClass::Record,
        version: 1,
        lifecycle: DomainRecordLifecycle::Active,
        payload: serde_json::json!({"observation": "persisted"}),
        references: Vec::new(),
    };
    let scenario = Scenario::new(
        SimTime::EPOCH,
        vec![
            EntityRef::Person(PersonId::new(1)),
            EntityRef::Person(PersonId::new(2)),
        ],
    )
    .with_domain_records(vec![
        provider,
        value.state.into_record().expect("resource record"),
    ]);
    let canwu = canwu_api::Canwu::new_with_plugins(3, scenario, &[&ResourcePlugin::default()])
        .expect("runtime");
    let witness = resource_observation_witness(
        &canwu,
        &holder(1),
        &grant_id,
        ResourceObservationAdapterRevisionId::new("test:adapter:v1").expect("adapter"),
        SimTime::EPOCH,
    )
    .expect("witness");
    assert_eq!(witness.source_versions, vec![source]);
    assert_eq!(witness.stock[0].known_minimum, 20);
}

#[test]
fn report_observations_bind_authoritative_allocation_and_fulfillment_ownership() {
    let mut value = fixture(75, 0);
    let demand_id = ResourceDemandId::new("test:demand:report-owned").expect("demand");
    value
        .state
        .install_demand(demand(
            demand_id.as_str(),
            30,
            1,
            PartialFulfillmentPolicy::AcceptPartial,
            &value,
            None,
        ))
        .expect("demand");
    allocate(&mut value.state, "test:allocate:report-owned");
    let leg = value
        .state
        .allocation_legs
        .values()
        .next()
        .cloned()
        .expect("allocation");
    let fulfillment_id =
        ResourceFulfillmentId::new("test:fulfillment:report-owned").expect("fulfillment");
    let mut fulfillment = ResourceFulfillment {
        id: fulfillment_id.clone(),
        revision: ResourceRevision::INITIAL,
        demand: demand_id.clone(),
        allocation_legs: vec![leg.id.clone()],
        consumed_quantity: 10,
        remainder: 20,
        status: FulfillmentStatus::Partial,
        rejection_reason: None,
        operation_key: ResourceOperationKey::new("test:fulfillment:report-owned").expect("key"),
        semantic_digest: String::new(),
        terminal_sequence: 777,
    };
    fulfillment.semantic_digest =
        canonical_test_digest("canwu.resource.fulfillment.v1", &fulfillment);
    value
        .state
        .fulfillments
        .insert(fulfillment_id.clone(), fulfillment);
    let grant_id = ResourceReportGrantId::new("test:report-grant:owned-work").expect("grant");
    value
        .state
        .install_report_grant(ResourceReportGrantV1 {
            id: grant_id.clone(),
            holder: holder(1),
            scope: value.scope.clone(),
            accounts: BTreeSet::from([value.account.clone()]),
            demands: BTreeSet::from([demand_id]),
            include_transfer_details: false,
            confidence_per_mille: 1_000,
            cadence_minutes: 60,
            delay_minutes: 0,
        })
        .expect("grant");
    let source = external_version("report-owned");
    let head = ResourceObservationHeadV1 {
        id: ResourceObservationHeadId::new("test:observation:owned-work").expect("head"),
        revision: ResourceRevision::INITIAL,
        provider_plugin: "test-provider".to_owned(),
        provider_version: "1".to_owned(),
        provider_semantic_hash: digest('d'),
        provider_source: source.clone(),
        holder: holder(1),
        grant: grant_id,
        provider_state_revision: value.state.state_revision,
        observed_at: SimTime::EPOCH,
        confidence_per_mille: 1_000,
        stock: Vec::new(),
        demands: Vec::new(),
        allocations: vec![ResourceAllocationObservationV1 {
            allocation: leg.id.clone(),
            exact: (&leg).into(),
            status: leg.status,
        }],
        fulfillments: vec![ResourceFulfillmentObservationV1 {
            fulfillment: fulfillment_id,
            consumed: 10,
            remainder: 20,
            rejection_reason: None,
        }],
        transfers: Vec::new(),
        consumptions: Vec::new(),
        source_versions: vec![source],
        semantic_digest: String::new(),
    }
    .seal()
    .expect("head");
    value
        .state
        .record_observation_head(head.clone())
        .expect("owned observation");
    let mut forged = head;
    forged.revision = ResourceRevision::new(2).expect("revision");
    forged.allocations[0].exact.quantity += 1;
    forged.fulfillments[0].consumed += 1;
    forged.semantic_digest.clear();
    forged = forged.seal().expect("re-seal forged observation");
    assert!(value.state.record_observation_head(forged).is_err());
}

#[test]
fn delayed_report_wakes_and_publishes_on_an_otherwise_quiet_boundary() {
    let mut value = fixture(20, 0);
    let grant_id = ResourceReportGrantId::new("test:report-grant:delayed").expect("grant");
    value
        .state
        .install_report_grant(ResourceReportGrantV1 {
            id: grant_id.clone(),
            holder: holder(1),
            scope: value.scope.clone(),
            accounts: BTreeSet::from([value.account.clone()]),
            demands: BTreeSet::new(),
            include_transfer_details: false,
            confidence_per_mille: 1_000,
            cadence_minutes: 60,
            delay_minutes: 5,
        })
        .expect("grant");
    let source = external_version("delayed-report-source");
    value
        .state
        .record_observation_head(
            ResourceObservationHeadV1 {
                id: ResourceObservationHeadId::new("test:observation:delayed").expect("head"),
                revision: ResourceRevision::INITIAL,
                provider_plugin: "test-provider".to_owned(),
                provider_version: "1".to_owned(),
                provider_semantic_hash: digest('e'),
                provider_source: source.clone(),
                holder: holder(1),
                grant: grant_id,
                provider_state_revision: value.state.state_revision,
                observed_at: SimTime::EPOCH,
                confidence_per_mille: 1_000,
                stock: vec![ResourceStockObservationV1 {
                    account: value.account.clone(),
                    scope: value.scope.clone(),
                    known_minimum: 20,
                    known_maximum: 20,
                    reserved: 0,
                    protected: 0,
                }],
                demands: Vec::new(),
                allocations: Vec::new(),
                fulfillments: Vec::new(),
                transfers: Vec::new(),
                consumptions: Vec::new(),
                source_versions: vec![source.clone()],
                semantic_digest: String::new(),
            }
            .seal()
            .expect("head"),
        )
        .expect("observation");
    let provider = DomainRecord {
        reference: source.record,
        owner: "test-provider".to_owned(),
        class: DomainRecordClass::Record,
        version: 1,
        lifecycle: DomainRecordLifecycle::Active,
        payload: serde_json::json!({"report": "source"}),
        references: Vec::new(),
    };
    let scenario = Scenario::new(SimTime::EPOCH, vec![EntityRef::Person(PersonId::new(1))])
        .with_domain_records(vec![provider, value.state.into_record().expect("record")]);
    let plugin = ResourcePlugin::default();
    let mut canwu =
        canwu_api::Canwu::new_with_plugins(29, scenario, &[&plugin]).expect("resource runtime");
    let acquisition = lease_request(
        1,
        "test:lease:delayed-report-trigger",
        SimTime::EPOCH,
        CompletionPolicyClassV1::Guaranteed,
    );
    let acquisition = RequestCompletionLeaseV1 {
        recipe: CompletionCapacityRecipeV1 {
            receipts: MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE,
            mutations: 1,
            reports_per_holder: 0,
            holders: 0,
            bytes: 1_024,
        },
        ..acquisition
    };
    enqueue_resource_completion_operation(
        &mut canwu,
        SimTime::EPOCH,
        &ResourceCompletionOperationV1::Acquire(acquisition),
    )
    .expect("trigger ingress");
    canwu
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("schedule delayed report");
    assert!(canwu.knowledge().for_holder(&holder(1)).is_none());
    canwu
        .settle_boundary(BoundaryRequest::at(
            SimTime::EPOCH + SimDuration::minutes(5),
        ))
        .expect("quiet delayed-report boundary");
    assert!(
        canwu
            .knowledge()
            .for_holder(&holder(1))
            .is_some_and(|records| !records.is_empty())
    );
}

#[test]
fn prepare_revalidates_exact_local_targets_and_persists_stable_rejection() {
    let mut value = fixture(10, 0);
    let request = RequestCompletionLeaseV1 {
        recipe: CompletionCapacityRecipeV1 {
            receipts: MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE,
            mutations: 1,
            reports_per_holder: 0,
            holders: 0,
            bytes: 1_024,
        },
        expected_participants: BTreeSet::from([
            PLUGIN_NAME.to_owned(),
            "test.other-participant".to_owned(),
        ]),
        ..lease_request(
            1,
            "test:lease:prepare-revalidation",
            SimTime::EPOCH,
            CompletionPolicyClassV1::Guaranteed,
        )
    };
    let acquisition = request.id.clone();
    value
        .state
        .apply_operation(&ResourceOperationRequestV1::Completion(
            ResourceCompletionOperationV1::Acquire(request),
        ))
        .expect("acquire");
    let grant_id =
        CompletionCapacityGrantId::new("test:grant:prepare-revalidation").expect("grant");
    let other_grant =
        CompletionCapacityGrantId::new("test:grant:prepare-revalidation-other").expect("grant");
    value
        .state
        .apply_operation(&ResourceOperationRequestV1::Completion(
            ResourceCompletionOperationV1::Grant(GrantCompletionCapacityV1 {
                grant_id: other_grant.clone(),
                acquisition: acquisition.clone(),
                expected_acquisition_revision: value.state.completion_leases.acquisitions
                    [&acquisition]
                    .revision,
                owner_plugin: "test.other-participant".to_owned(),
                target_versions: vec![CompletionLockedTargetV1::Account {
                    id: value.account.clone(),
                    revision: ResourceRevision::INITIAL,
                }],
                current_boundary: 1,
            }),
        ))
        .expect("second participant grant");
    value
        .state
        .apply_operation(&ResourceOperationRequestV1::Completion(
            ResourceCompletionOperationV1::Grant(GrantCompletionCapacityV1 {
                grant_id: grant_id.clone(),
                acquisition: acquisition.clone(),
                expected_acquisition_revision: value.state.completion_leases.acquisitions
                    [&acquisition]
                    .revision,
                owner_plugin: PLUGIN_NAME.to_owned(),
                target_versions: vec![CompletionLockedTargetV1::Account {
                    id: value.account.clone(),
                    revision: ResourceRevision::INITIAL,
                }],
                current_boundary: 1,
            }),
        ))
        .expect("grant");
    value
        .state
        .accounts
        .get_mut(&value.account)
        .expect("account")
        .revision = ResourceRevision::new(2).expect("revision");
    let envelope_digest = value.state.completion_leases.acquisitions[&acquisition]
        .eligibility_envelope
        .digest
        .clone();
    let outcome = value
        .state
        .apply_operation(&ResourceOperationRequestV1::Completion(
            ResourceCompletionOperationV1::Prepare(PrepareCompletionCapacityV1 {
                acquisition: acquisition.clone(),
                expected_acquisition_revision: value.state.completion_leases.acquisitions
                    [&acquisition]
                    .revision,
                grant: grant_id.clone(),
                expected_grant_revision: value.state.completion_leases.grants[&grant_id].revision,
                current_boundary: 2,
                eligibility_envelope_digest: envelope_digest,
            }),
        ))
        .expect("stable prepare result");
    assert_eq!(outcome.status, ResourceOperationStatus::Applied);
    assert_eq!(
        value.state.completion_leases.grants[&grant_id].state,
        CompletionGrantStateV1::Rejected
    );
    assert_eq!(
        value.state.completion_leases.acquisitions[&acquisition].state,
        CompletionLeaseAcquisitionStateV1::Aborting
    );
    let encoded = serde_json::to_vec(&value.state).expect("restart state encoding");
    let mut restarted: ResourceState =
        serde_json::from_slice(&encoded).expect("restart state decoding");
    let cleaned = restarted
        .completion_leases
        .cleanup_aborting(16)
        .expect("zero-delay autonomous cleanup after restart");
    assert_eq!(cleaned, vec![acquisition.clone()]);
    assert_eq!(
        restarted.completion_leases.acquisitions[&acquisition].state,
        CompletionLeaseAcquisitionStateV1::Released
    );
    assert_eq!(
        restarted.completion_leases.grants[&grant_id].state,
        CompletionGrantStateV1::Released
    );
    assert_eq!(
        restarted.completion_leases.grants[&other_grant].state,
        CompletionGrantStateV1::Released
    );
    assert_eq!(restarted.completion_leases.reserved_units, 0);
}

fn lease_request(
    authority: u64,
    id: &str,
    at: SimTime,
    class: CompletionPolicyClassV1,
) -> RequestCompletionLeaseV1 {
    RequestCompletionLeaseV1 {
        id: CompletionLeaseAcquisitionId::new(id).expect("lease"),
        operation_key: ResourceOperationKey::new(format!("test:operation:{id}")).expect("key"),
        holder: holder(authority),
        operation_namespace: "test.resource".to_owned(),
        eligibility_time: at,
        eligibility_envelope: EligibilityEnvelopeV1::new(
            Vec::new(),
            BTreeMap::new(),
            BTreeSet::new(),
            Vec::new(),
            Vec::new(),
        )
        .expect("envelope"),
        recipe: CompletionCapacityRecipeV1 {
            receipts: 1,
            mutations: 1,
            reports_per_holder: 0,
            holders: 0,
            bytes: 1_024,
        },
        expected_participants: BTreeSet::from([PLUGIN_NAME.to_owned()]),
        policy_class: class,
    }
}

#[test]
fn leases_enforce_abort_activation_fairness_and_same_time_cutoff() {
    let budget = budget();
    let mut book = CompletionLeaseBookV1::default();
    let guaranteed = book
        .request_acquisition(
            &budget,
            lease_request(
                1,
                "test:lease:guaranteed",
                SimTime::EPOCH,
                CompletionPolicyClassV1::Guaranteed,
            ),
        )
        .expect("guaranteed");
    assert!(
        book.request_acquisition(
            &budget,
            lease_request(
                1,
                "test:lease:same-time",
                SimTime::EPOCH,
                CompletionPolicyClassV1::Guaranteed,
            ),
        )
        .is_err()
    );
    let burst = book
        .request_acquisition(
            &budget,
            lease_request(
                2,
                "test:lease:burst",
                SimTime::EPOCH,
                CompletionPolicyClassV1::SharedBurst,
            ),
        )
        .expect("burst");
    assert_eq!(
        deterministic_completion_fairness_order([burst.clone(), guaranteed.clone()])[0],
        guaranteed.id
    );
    assert!(
        book.grant_capacity(
            &budget,
            GrantCompletionCapacityV1 {
                grant_id: CompletionCapacityGrantId::new("test:grant:burst").expect("grant"),
                acquisition: burst.id,
                expected_acquisition_revision: burst.revision,
                owner_plugin: PLUGIN_NAME.to_owned(),
                target_versions: vec![CompletionLockedTargetV1::ExternalRecord {
                    version: external_version("burst"),
                }],
                current_boundary: 1,
            },
        )
        .is_err()
    );
    let grant = book
        .grant_capacity(
            &budget,
            GrantCompletionCapacityV1 {
                grant_id: CompletionCapacityGrantId::new("test:grant:guaranteed").expect("grant"),
                acquisition: guaranteed.id.clone(),
                expected_acquisition_revision: guaranteed.revision,
                owner_plugin: PLUGIN_NAME.to_owned(),
                target_versions: vec![CompletionLockedTargetV1::ExternalRecord {
                    version: external_version("guaranteed"),
                }],
                current_boundary: 1,
            },
        )
        .expect("grant");
    let revision = book.acquisitions[&guaranteed.id].revision;
    assert_eq!(
        book.abort(&holder(1), &guaranteed.id, revision)
            .expect("abort"),
        "released"
    );
    assert!(book.status_for(&holder(2), &guaranteed.id).is_err());
    assert_eq!(
        book.grants[&grant.id].state,
        CompletionGrantStateV1::Released
    );

    let mut activation = CompletionLeaseBookV1::default();
    let request = activation
        .request_acquisition(
            &budget,
            lease_request(
                1,
                "test:lease:activate",
                SimTime::EPOCH,
                CompletionPolicyClassV1::Guaranteed,
            ),
        )
        .expect("request");
    let target = CompletionLockedTargetV1::ExternalRecord {
        version: external_version("activation"),
    };
    let grant = activation
        .grant_capacity(
            &budget,
            GrantCompletionCapacityV1 {
                grant_id: CompletionCapacityGrantId::new("test:grant:activate").expect("grant"),
                acquisition: request.id.clone(),
                expected_acquisition_revision: request.revision,
                owner_plugin: PLUGIN_NAME.to_owned(),
                target_versions: vec![target.clone()],
                current_boundary: 1,
            },
        )
        .expect("grant");
    let prepared = activation
        .prepare_capacity(PrepareCompletionCapacityV1 {
            acquisition: request.id.clone(),
            expected_acquisition_revision: activation.acquisitions[&request.id].revision,
            grant: grant.id.clone(),
            expected_grant_revision: grant.revision,
            current_boundary: 2,
            eligibility_envelope_digest: activation.acquisitions[&request.id]
                .eligibility_envelope
                .digest
                .clone(),
        })
        .expect("prepare");
    let certificate = activation
        .activate_single_owner(&request.id, &prepared.id, 3)
        .expect("activate");
    let restored: CompletionLeaseBookV1 = serde_json::from_str(
        &serde_json::to_string(&activation).expect("serialize active lease book"),
    )
    .expect("deserialize active lease book");
    restored
        .validate(&budget)
        .expect("restored active lease book");
    assert_eq!(
        activation
            .abort(
                &holder(1),
                &request.id,
                activation.acquisitions[&request.id].revision,
            )
            .expect("stable abort"),
        "already_activated"
    );
    activation
        .consume_authoritative_grant(&certificate, &grant.id, SimTime::EPOCH, &[target])
        .expect("consume");
    activation
        .complete_grant(&request.id, &grant.id)
        .expect("complete");
    activation.validate(&budget).expect("lease closure");
}

type ArchiveObjectMap = BTreeMap<(String, String), Vec<u8>>;

#[derive(Clone, Default)]
struct MemoryArchiveStore {
    objects: Rc<RefCell<ArchiveObjectMap>>,
    retention: Rc<RefCell<BTreeMap<String, ResourceArchiveRetentionHandleV1>>>,
}

impl ResourceArchiveStore for MemoryArchiveStore {
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
        handle: &ResourceArchiveRetentionHandleV1,
    ) -> Result<(), ResourceError> {
        self.retention
            .borrow_mut()
            .insert(handle.id.clone(), handle.clone());
        Ok(())
    }

    fn finalize_resource_archive_retention(
        &self,
        handle_id: &str,
        phase: ResourceArchiveRetentionPhaseV1,
    ) -> Result<(), ResourceError> {
        let mut retention = self.retention.borrow_mut();
        let handle = retention
            .get_mut(handle_id)
            .ok_or_else(|| ResourceError::NotFound("retention handle unavailable".to_owned()))?;
        handle.phase = phase;
        Ok(())
    }
}

impl canwu_api::PluginArchiveObjectProvider for MemoryArchiveStore {
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

fn rehash_resource_archive_wrappers(prepared: &mut PreparedResourceArchiveBatchV1) {
    prepared.blob.content_id.clear();
    prepared.blob.content_id =
        canonical_test_digest("canwu.resource.archive-blob.v1", &prepared.blob);
    prepared.membership_page.memberships = prepared
        .blob
        .records
        .iter()
        .enumerate()
        .map(|(ordinal, record)| ResourceArchiveMembershipV1 {
            key: record.key.clone(),
            blob_id: prepared.blob.content_id.clone(),
            ordinal: u16::try_from(ordinal).expect("archive ordinal"),
            terminal_sequence: record.terminal_sequence,
            semantic_digest: record.semantic_digest.clone(),
        })
        .collect();
    prepared.membership_page.id.clear();
    prepared.membership_page.semantic_digest.clear();
    prepared.membership_page.semantic_digest = canonical_test_digest(
        "canwu.resource.archive-membership-page.v1",
        &prepared.membership_page,
    );
    prepared.membership_page.id = prepared.membership_page.semantic_digest.clone();
    prepared.temporal_page.entries = prepared
        .blob
        .records
        .iter()
        .map(|record| ResourceArchiveTemporalEntryV1 {
            terminal_sequence: record.terminal_sequence,
            key: record.key.clone(),
        })
        .collect();
    prepared
        .temporal_page
        .entries
        .sort_by_key(|entry| entry.terminal_sequence);
    prepared.temporal_page.id.clear();
    prepared.temporal_page.semantic_digest.clear();
    prepared.temporal_page.semantic_digest = canonical_test_digest(
        "canwu.resource.archive-temporal-page.v1",
        &prepared.temporal_page,
    );
    prepared.temporal_page.id = prepared.temporal_page.semantic_digest.clone();
    prepared.directory.membership_pages = vec![prepared.membership_page.id.clone()];
    prepared.directory.temporal_pages = vec![prepared.temporal_page.id.clone()];
    prepared.directory.blob_ids = vec![prepared.blob.content_id.clone()];
    prepared.directory.id.clear();
    prepared.directory.semantic_digest.clear();
    prepared.directory.semantic_digest =
        canonical_test_digest("canwu.resource.archive-directory.v1", &prepared.directory);
    prepared.directory.id = prepared.directory.semantic_digest.clone();
    prepared.retention.id = canonical_test_digest(
        "canwu.resource.archive-retention-id.v1",
        &(&prepared.expected_source_root, &prepared.directory.id),
    );
    prepared.retention.directory_root = prepared.directory.id.clone();
    prepared.retention.object_ids = BTreeMap::from([
        (
            RESOURCE_ARCHIVE_BLOB_NAMESPACE.to_owned(),
            BTreeSet::from([prepared.blob.content_id.clone()]),
        ),
        (
            RESOURCE_ARCHIVE_MEMBERSHIP_PAGE_NAMESPACE.to_owned(),
            BTreeSet::from([prepared.membership_page.id.clone()]),
        ),
        (
            RESOURCE_ARCHIVE_TEMPORAL_PAGE_NAMESPACE.to_owned(),
            BTreeSet::from([prepared.temporal_page.id.clone()]),
        ),
        (
            RESOURCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE.to_owned(),
            BTreeSet::from([prepared.directory.id.clone()]),
        ),
    ]);
    prepared.retention.semantic_digest.clear();
    prepared.retention.semantic_digest =
        canonical_test_digest("canwu.resource.archive-retention.v1", &prepared.retention);
}

fn rejected_operation(state: &mut ResourceState, suffix: &str) {
    let outcome = state
        .apply_operation(&ResourceOperationRequestV1::CancelDemand(
            ResourceCancelDemandRequestV1 {
                operation_key: ResourceOperationKey::new(format!("test:reject:{suffix}"))
                    .expect("key"),
                demand: ResourceDemandId::new(format!("test:missing:{suffix}")).expect("demand"),
                expected_demand_revision: ResourceRevision::INITIAL,
            },
        ))
        .expect("durable rejection");
    assert_eq!(outcome.status, ResourceOperationStatus::Rejected);
}

#[test]
fn bounded_archive_backpressure_restart_and_forgery_are_explicit() {
    let mut limits = ResourceLimitsV1::canonical();
    limits.max_archive_candidates = 2;
    let mut state = ResourceState::empty(limits).expect("state");
    rejected_operation(&mut state, "one");
    rejected_operation(&mut state, "two");
    assert!(matches!(
        state.apply_operation(&ResourceOperationRequestV1::CancelDemand(
            ResourceCancelDemandRequestV1 {
                operation_key: ResourceOperationKey::new("test:reject:three").expect("key"),
                demand: ResourceDemandId::new("test:missing:three").expect("demand"),
                expected_demand_revision: ResourceRevision::INITIAL,
            },
        )),
        Err(ResourceError::LimitExceeded(_))
    ));
    let prepared = state.prepare_resource_archive(1).expect("prepared archive");
    let store = MemoryArchiveStore::default();
    let verified = prepared
        .store_and_verify(&store)
        .expect("verified archive commit");
    let mut forged_commit = verified;
    forged_commit.archived_records = 2;
    assert!(forged_commit.validate().is_err());
    let mut forged = prepared.clone();
    forged.directory.id = digest('f');
    assert!(
        forged
            .store_and_verify(&MemoryArchiveStore::default())
            .is_err()
    );

    let mut forged_membership = prepared.membership_page.clone();
    forged_membership.memberships[0].ordinal = 1;
    forged_membership.id.clear();
    forged_membership.semantic_digest.clear();
    forged_membership.semantic_digest = canonical_test_digest(
        "canwu.resource.archive-membership-page.v1",
        &forged_membership,
    );
    forged_membership.id = forged_membership.semantic_digest.clone();
    store
        .store_resource_archive_object(
            RESOURCE_ARCHIVE_MEMBERSHIP_PAGE_NAMESPACE,
            &forged_membership.id,
            &serde_json::to_vec(&forged_membership).expect("forged membership encoding"),
        )
        .expect("store internally hashed forged membership");
    let mut forged_directory = prepared.directory.clone();
    forged_directory.membership_pages = vec![forged_membership.id];
    forged_directory.id.clear();
    forged_directory.semantic_digest.clear();
    forged_directory.semantic_digest =
        canonical_test_digest("canwu.resource.archive-directory.v1", &forged_directory);
    forged_directory.id = forged_directory.semantic_digest.clone();
    assert!(authenticate_resource_archive_directory(&store, &forged_directory).is_err());

    let plugin = ResourcePlugin::default();
    let scenario = Scenario::new(SimTime::EPOCH, Vec::new())
        .with_domain_records(vec![state.into_record().expect("record")]);
    let mut canwu = canwu_api::Canwu::new_with_plugins(11, scenario, &[&plugin]).expect("runtime");
    let mut forged_live_batch = prepared.clone();
    forged_live_batch.blob.records[0].quantity = 999;
    assert!(enqueue_resource_archive(&mut canwu, &forged_live_batch, &store).is_err());
    let receipt = enqueue_resource_archive(&mut canwu, &prepared, &store).expect("enqueue archive");
    canwu
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("archive boundary");
    let snapshot = canwu.snapshot_json().expect("snapshot");
    let checkpoint = canwu.checkpoint_journal().expect("checkpoint journal");
    let journal = canwu.replay_journal();
    assert!(from_resource_snapshot_json(&snapshot, &[&plugin]).is_err());
    assert!(from_resource_checkpoint_journal(checkpoint.clone(), &[&plugin]).is_err());
    assert!(replay_resource_from_journal(&[&plugin], &journal).is_err());
    from_resource_checkpoint_journal_with_archive_store(
        checkpoint.clone(),
        &[&plugin],
        Rc::new(store.clone()),
    )
    .expect("checkpoint restore authenticates archive provider");
    replay_resource_from_journal_with_archive_store(&[&plugin], &journal, Rc::new(store.clone()))
        .expect("journal replay authenticates archive provider");
    let mut restarted = from_resource_snapshot_json_with_archive_store(
        &snapshot,
        &[&plugin],
        Rc::new(store.clone()),
    )
    .expect("provider-verified restart");
    finalize_resource_archive_retention(&mut restarted, &store, &receipt)
        .expect("finalize after restart");
    restarted
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("ack boundary");
    let (_, restored) = resource_state(&restarted)
        .expect("query")
        .expect("resource state");
    assert_eq!(restored.archive_head.archived_record_count, 1);
    assert!(
        !restored
            .archive_retention_handles
            .contains_key(&receipt.retention_handle_id)
    );
    assert!(
        restored
            .terminal_archive_candidates
            .values()
            .any(|candidate| {
                matches!(
                    candidate,
                    ResourceTerminalRecordKeyV1::ArchiveMaintenanceReceipt(_)
                )
            })
    );
    let next_archive = restored
        .prepare_resource_archive(2)
        .expect("maintenance receipt can move cold");
    assert!(next_archive.selected.iter().any(|candidate| {
        matches!(
            candidate,
            ResourceTerminalRecordKeyV1::ArchiveMaintenanceReceipt(_)
        )
    }));

    store.objects.borrow_mut().insert(
        (
            RESOURCE_ARCHIVE_BLOB_NAMESPACE.to_owned(),
            prepared.blob.content_id.clone(),
        ),
        b"{}".to_vec(),
    );
    assert!(
        from_resource_snapshot_json_with_archive_store(
            &snapshot,
            &[&plugin],
            Rc::new(store.clone()),
        )
        .is_err(),
        "restore must reject a provider object that no longer authenticates"
    );
    assert!(
        from_resource_checkpoint_journal_with_archive_store(
            checkpoint,
            &[&plugin],
            Rc::new(store.clone()),
        )
        .is_err()
    );
    assert!(
        replay_resource_from_journal_with_archive_store(
            &[&plugin],
            &journal,
            Rc::new(store.clone()),
        )
        .is_err()
    );
}

#[test]
fn rehashed_terminal_summaries_cannot_override_typed_archive_payloads() {
    let mut value = fixture(100, 0);
    let demand_id = ResourceDemandId::new("test:demand:typed-archive-forgery").expect("demand");
    value
        .state
        .install_demand(demand(
            demand_id.as_str(),
            30,
            30,
            PartialFulfillmentPolicy::RejectPartial,
            &value,
            None,
        ))
        .expect("install demand");
    allocate(&mut value.state, "test:allocate:typed-archive-forgery");
    let demand_revision = value.state.demands[&demand_id].revision;
    value
        .state
        .apply_operation(&ResourceOperationRequestV1::CancelDemand(
            ResourceCancelDemandRequestV1 {
                operation_key: ResourceOperationKey::new("test:cancel:typed-archive-forgery")
                    .expect("operation key"),
                demand: demand_id,
                expected_demand_revision: demand_revision,
            },
        ))
        .expect("cancel demand");
    let prepared = value
        .state
        .prepare_resource_archive(value.state.terminal_archive_candidates.len())
        .expect("prepare typed archive records");
    assert!(
        prepared
            .blob
            .records
            .iter()
            .any(|record| matches!(record.payload, ResourceTerminalArchivePayloadV1::Demand(_)))
    );
    assert!(prepared.blob.records.iter().any(|record| matches!(
        record.payload,
        ResourceTerminalArchivePayloadV1::Reservation(_)
    )));
    assert!(prepared.blob.records.iter().any(|record| matches!(
        record.payload,
        ResourceTerminalArchivePayloadV1::AllocationLeg(_)
    )));
    assert!(
        prepared
            .blob
            .records
            .iter()
            .any(|record| matches!(record.payload, ResourceTerminalArchivePayloadV1::Outcome(_)))
    );

    for ordinal in 0..prepared.blob.records.len() {
        let mut forged = prepared.clone();
        forged.blob.records[ordinal].quantity = forged.blob.records[ordinal]
            .quantity
            .checked_add(1)
            .expect("forged quantity");
        rehash_resource_archive_wrappers(&mut forged);
        assert!(
            forged
                .store_and_verify(&MemoryArchiveStore::default())
                .is_err(),
            "internally re-hashed summary at ordinal {ordinal} must still match its typed payload"
        );
    }
}

#[test]
fn archived_operation_identity_controls_exact_retry_after_target_retirement() {
    let mut value = fixture(100, 0);
    let demand_id = ResourceDemandId::new("test:demand:cold-idempotency").expect("demand");
    value
        .state
        .install_demand(demand(
            demand_id.as_str(),
            10,
            10,
            PartialFulfillmentPolicy::RejectPartial,
            &value,
            None,
        ))
        .expect("install demand");
    let request = ResourceOperationRequestV1::CancelDemand(ResourceCancelDemandRequestV1 {
        operation_key: ResourceOperationKey::new("test:cancel:cold-idempotency")
            .expect("operation key"),
        demand: demand_id.clone(),
        expected_demand_revision: value.state.demands[&demand_id].revision,
    });
    let expected_outcome = value
        .state
        .apply_operation(&request)
        .expect("cancel demand");
    let prepared = value
        .state
        .prepare_resource_archive(value.state.terminal_archive_candidates.len())
        .expect("prepare cold identity archive");
    let store = Rc::new(MemoryArchiveStore::default());
    let plugin = ResourcePlugin::default();
    let scenario = Scenario::new(SimTime::EPOCH, vec![EntityRef::Person(PersonId::new(1))])
        .with_domain_records(vec![value.state.into_record().expect("resource record")]);
    let mut canwu =
        canwu_api::Canwu::new_with_plugins(41, scenario, &[&plugin]).expect("archive runtime");
    enqueue_resource_archive(&mut canwu, &prepared, store.as_ref()).expect("enqueue archive");
    canwu
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("archive boundary");
    let (_, archived_state) = resource_state(&canwu)
        .expect("resource query")
        .expect("resource state");
    assert!(!archived_state.demands.contains_key(&demand_id));
    assert_eq!(
        archived_resource_operation_outcome(
            &archived_state,
            store.as_ref(),
            &request.operation_key(),
        )
        .expect("authenticated cold lookup"),
        Some(expected_outcome)
    );
    let restart_scenario = Scenario::new(SimTime::EPOCH, vec![EntityRef::Person(PersonId::new(1))])
        .with_domain_records(vec![archived_state.into_record().expect("archived record")]);
    let mut restarted =
        canwu_api::Canwu::new_with_plugins(42, restart_scenario, &[&plugin]).expect("restart host");
    restarted.set_plugin_archive_object_provider(store);

    let command = ResourceCommandV1 {
        subject: holder(1),
        request: request.clone(),
    };
    assert!(matches!(
        restarted
            .process_command(CommandRequest::new(
                CommandRequestId::new(1),
                restarted.revision(),
                CommandEnvelope::new(
                    Issuer::Actor(PersonId::new(1)),
                    resource_command(&command).expect("command"),
                )
                .at_time(SimTime::EPOCH),
            ))
            .expect("exact cold retry"),
        CommandOutcome::Accepted { .. }
    ));

    let mut conflicting = command;
    let ResourceOperationRequestV1::CancelDemand(cancel) = &mut conflicting.request else {
        unreachable!("test request is cancellation")
    };
    cancel.expected_demand_revision = cancel
        .expected_demand_revision
        .next()
        .expect("conflicting revision");
    let conflict = restarted
        .process_command(CommandRequest::new(
            CommandRequestId::new(2),
            restarted.revision(),
            CommandEnvelope::new(
                Issuer::Actor(PersonId::new(1)),
                resource_command(&conflicting).expect("command"),
            )
            .at_time(SimTime::EPOCH),
        ))
        .expect("cold conflict is durable command evidence");
    let CommandOutcome::Rejected { rejection } = conflict else {
        panic!("conflicting cold retry was admitted");
    };
    assert_eq!(rejection.error.code, ErrorCode::IdempotencyConflict);
}

#[test]
fn terminal_demand_allocation_closure_retires_and_releases_active_capacity() {
    let mut value = fixture(100, 0);
    let demand_id = ResourceDemandId::new("test:demand:retire-closure").expect("demand");
    value
        .state
        .install_demand(demand(
            demand_id.as_str(),
            30,
            30,
            PartialFulfillmentPolicy::RejectPartial,
            &value,
            None,
        ))
        .expect("install demand");
    allocate(&mut value.state, "test:allocate:retire-closure");
    let allocation = only_leg(&value.state);
    let reservation = value.state.allocation_legs[&allocation.id]
        .reservation
        .clone();
    let demand_revision = value.state.demands[&demand_id].revision;
    value
        .state
        .apply_operation(&ResourceOperationRequestV1::CancelDemand(
            ResourceCancelDemandRequestV1 {
                operation_key: ResourceOperationKey::new("test:cancel:retire-closure")
                    .expect("operation key"),
                demand: demand_id.clone(),
                expected_demand_revision: demand_revision,
            },
        ))
        .expect("cancel allocated demand");
    value.state.limits.max_demands = 1;
    value
        .state
        .validate()
        .expect("terminal history does not consume active demand capacity");
    let replacement_id =
        ResourceDemandId::new("test:demand:capacity-replacement").expect("replacement demand");
    value
        .state
        .install_demand(demand(
            replacement_id.as_str(),
            10,
            10,
            PartialFulfillmentPolicy::RejectPartial,
            &value,
            None,
        ))
        .expect("active capacity was released");
    let prepared = value
        .state
        .prepare_resource_archive(value.state.terminal_archive_candidates.len())
        .expect("prepare terminal closure archive");
    for key in [
        ResourceTerminalRecordKeyV1::AllocationLeg(allocation.id.clone()),
        ResourceTerminalRecordKeyV1::Reservation(reservation.clone()),
        ResourceTerminalRecordKeyV1::Demand(demand_id.clone()),
    ] {
        assert!(prepared.selected.contains(&key));
    }
    let store = MemoryArchiveStore::default();
    let plugin = ResourcePlugin::default();
    let scenario = Scenario::new(SimTime::EPOCH, Vec::new())
        .with_domain_records(vec![value.state.into_record().expect("resource record")]);
    let mut canwu =
        canwu_api::Canwu::new_with_plugins(43, scenario, &[&plugin]).expect("archive runtime");
    enqueue_resource_archive(&mut canwu, &prepared, &store).expect("enqueue archive");
    canwu
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("archive boundary");
    let (_, archived) = resource_state(&canwu)
        .expect("resource query")
        .expect("resource state");
    assert!(!archived.demands.contains_key(&demand_id));
    assert!(!archived.reservations.contains_key(&reservation));
    assert!(!archived.allocation_legs.contains_key(&allocation.id));
    assert!(archived.demands.contains_key(&replacement_id));
    archived
        .validate()
        .expect("bounded archived resource state");
}

#[test]
fn tracked_commands_cannot_cross_holder_targets() {
    let (mut value, leg) = transfer_fixture("authority");
    let demand_id = value.state.allocation_legs[&leg.id].demand.clone();
    let destination = value.destination.clone();
    let transfer = start_transfer(&mut value, leg, "authority", Some(destination.clone()));
    let resource = value.resource.clone();
    let unit = value.unit.clone();
    let floor = value.floor.clone();
    let account = value.account.clone();
    let account_revision = value.state.accounts[&account].revision;
    let demand_revision = value.state.demands[&demand_id].revision;
    let transfer_revision = value.state.transfers[&transfer].revision;
    let certificate = value
        .state
        .completion_leases
        .certificates
        .values()
        .next()
        .cloned()
        .expect("retained completion certificate");
    let scenario = Scenario::new(
        SimTime::EPOCH,
        vec![
            EntityRef::Person(PersonId::new(1)),
            EntityRef::Person(PersonId::new(2)),
        ],
    )
    .with_domain_records(vec![value.state.into_record().expect("record")]);
    let mut canwu = canwu_api::Canwu::new_with_plugins(13, scenario, &[&ResourcePlugin::default()])
        .expect("runtime");

    let commands = [
        ResourceCommandV1 {
            subject: holder(1),
            request: ResourceOperationRequestV1::CreateAccount(ResourceCreateAccountRequestV1 {
                operation_key: ResourceOperationKey::new("test:create:cross-holder").expect("key"),
                account: ResourceAccount {
                    id: ResourceAccountId::new("test:account:cross-holder").expect("account"),
                    revision: ResourceRevision::INITIAL,
                    custodian: holder(2),
                    resource_revision: resource,
                    unit_revision: unit,
                    balance: 0,
                    capacity: None,
                    protected_floor_policy: None,
                    closed: false,
                },
            }),
        },
        ResourceCommandV1 {
            subject: holder(2),
            request: ResourceOperationRequestV1::SetProtectedFloor(
                ResourceProtectedFloorRequestV1 {
                    operation_key: ResourceOperationKey::new("test:floor:cross-holder")
                        .expect("key"),
                    account: account.clone(),
                    expected_account_revision: account_revision,
                    policy: Some(floor),
                },
            ),
        },
        ResourceCommandV1 {
            subject: holder(1),
            request: ResourceOperationRequestV1::CompleteTransfer(
                ResourceTransferDispositionRequestV1 {
                    operation_key: ResourceOperationKey::new("test:transfer:accept-command-bypass")
                        .expect("key"),
                    transfer: transfer.clone(),
                    expected_transfer_revision: transfer_revision,
                    at: SimTime::EPOCH,
                    disposition: ResourceTransferDispositionV1::Accept {
                        destination,
                        expected_destination_revision: ResourceRevision::INITIAL,
                        acceptance: ResourceTransportAcceptanceV1 {
                            evidence: external_version("accept-command-bypass"),
                            execution: TransportExecutionLink {
                                execution: canwu_api::TransportExecutionId(7),
                                itinerary_revision: canwu_api::ItineraryRevisionId(7),
                                leg_execution: None,
                                handoff: None,
                                capacity_booking: None,
                            },
                            destination: value.destination.clone(),
                            quantity: 30,
                            accepted_at: SimTime::EPOCH,
                            semantic_digest: String::new(),
                        }
                        .seal()
                        .expect("acceptance"),
                    },
                    exact_transport_evidence: Some(external_version("accept-command-bypass")),
                    completion_certificate: certificate.clone(),
                },
            ),
        },
        ResourceCommandV1 {
            subject: holder(2),
            request: ResourceOperationRequestV1::CancelDemand(ResourceCancelDemandRequestV1 {
                operation_key: ResourceOperationKey::new("test:demand:cross-holder").expect("key"),
                demand: demand_id,
                expected_demand_revision: demand_revision,
            }),
        },
        ResourceCommandV1 {
            subject: holder(2),
            request: ResourceOperationRequestV1::CompleteTransfer(
                ResourceTransferDispositionRequestV1 {
                    operation_key: ResourceOperationKey::new("test:transfer:cross-holder")
                        .expect("key"),
                    transfer,
                    expected_transfer_revision: transfer_revision,
                    at: SimTime::EPOCH,
                    disposition: ResourceTransferDispositionV1::Lose {
                        loss_id: ResourceLossId::new("test:loss:cross-holder").expect("loss"),
                        cause: EvidenceRef::DomainRecordVersion(external_version(
                            "cross-holder-loss",
                        )),
                    },
                    exact_transport_evidence: None,
                    completion_certificate: certificate,
                },
            ),
        },
    ];
    for (offset, command) in commands.into_iter().enumerate() {
        let outcome = canwu
            .process_command(CommandRequest::new(
                CommandRequestId::new(u64::try_from(offset + 1).expect("request id")),
                canwu.revision(),
                CommandEnvelope::new(
                    Issuer::Actor(match command.subject {
                        KnowledgeHolderRef::Person(actor) => actor,
                        _ => unreachable!("test commands are person-bound"),
                    }),
                    resource_command(&command).expect("command"),
                )
                .at_time(SimTime::EPOCH),
            ))
            .expect("tracked authority rejection is durable evidence");
        let CommandOutcome::Rejected { rejection } = outcome else {
            panic!("cross-holder resource command was admitted");
        };
        assert_eq!(rejection.error.code, ErrorCode::InvalidAuthority);
    }
}

#[test]
fn canonical_allocation_ingress_is_requester_bound_and_revision_exact() {
    let mut value = fixture(100, 0);
    value
        .state
        .install_demand(demand(
            "test:demand:authorized-one",
            30,
            10,
            PartialFulfillmentPolicy::AcceptPartial,
            &value,
            None,
        ))
        .expect("first demand");
    let mut other = demand(
        "test:demand:authorized-two",
        30,
        10,
        PartialFulfillmentPolicy::AcceptPartial,
        &value,
        None,
    );
    other.requester = holder(2);
    value
        .state
        .install_demand(other.clone())
        .expect("other demand");
    let expected_state_revision = value.state.state_revision;
    let scenario = Scenario::new(
        SimTime::EPOCH,
        vec![
            EntityRef::Person(PersonId::new(1)),
            EntityRef::Person(PersonId::new(2)),
        ],
    )
    .with_domain_records(vec![value.state.into_record().expect("record")]);
    let mut canwu = canwu_api::Canwu::new_with_plugins(19, scenario, &[&ResourcePlugin::default()])
        .expect("runtime");
    let request = ResourceAllocationRequestV1 {
        operation_key: ResourceOperationKey::new("test:allocation:authorized-one").expect("key"),
        expected_state_revision,
        at: SimTime::EPOCH,
        candidate_limit: 8,
    };
    enqueue_resource_allocation(&mut canwu, SimTime::EPOCH, &holder(1), &request)
        .expect("allocation ingress");
    canwu
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("allocation boundary");
    let (_, state) = resource_state(&canwu)
        .expect("query")
        .expect("resource state");
    assert!(
        state
            .reservations
            .values()
            .any(|reservation| reservation.demand.as_str() == "test:demand:authorized-one")
    );
    assert!(
        !state
            .reservations
            .values()
            .any(|reservation| reservation.demand == other.id)
    );

    let stale = ResourceAllocationRequestV1 {
        operation_key: ResourceOperationKey::new("test:allocation:stale").expect("key"),
        expected_state_revision,
        at: SimTime::EPOCH,
        candidate_limit: 8,
    };
    enqueue_resource_allocation(&mut canwu, SimTime::EPOCH, &holder(1), &stale)
        .expect("stale allocation ingress");
    canwu
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("stale allocation rejection boundary");
    let outcome =
        resource_operation_outcome(&canwu, &stale.operation_key).expect("durable stale outcome");
    assert_eq!(outcome.status, ResourceOperationStatus::Rejected);
}

#[test]
fn canonical_completion_ingress_persists_holder_status_and_abort() {
    let value = fixture(10, 0);
    let request = RequestCompletionLeaseV1 {
        recipe: CompletionCapacityRecipeV1 {
            receipts: MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE,
            mutations: 2,
            reports_per_holder: 0,
            holders: 0,
            bytes: 1_024,
        },
        ..lease_request(
            1,
            "test:lease:canonical-ingress",
            SimTime::EPOCH,
            CompletionPolicyClassV1::Guaranteed,
        )
    };
    let acquisition = request.id.clone();
    let scenario = Scenario::new(SimTime::EPOCH, vec![EntityRef::Person(PersonId::new(1))])
        .with_domain_records(vec![value.state.into_record().expect("record")]);
    let plugin = ResourcePlugin::default();
    let mut canwu =
        canwu_api::Canwu::new_with_plugins(23, scenario, &[&plugin]).expect("resource runtime");
    enqueue_resource_completion_operation(
        &mut canwu,
        SimTime::EPOCH,
        &ResourceCompletionOperationV1::Acquire(request),
    )
    .expect("acquire ingress");
    canwu
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("acquire boundary");
    assert_eq!(
        resource_completion_status(&canwu, &holder(1), &acquisition)
            .expect("holder status")
            .state,
        CompletionLeaseAcquisitionStateV1::Requested
    );
    assert!(resource_completion_status(&canwu, &holder(2), &acquisition).is_err());
    let (_, state) = resource_state(&canwu)
        .expect("query")
        .expect("resource state");
    let expected_revision = state.completion_leases.acquisitions[&acquisition].revision;
    enqueue_resource_completion_operation(
        &mut canwu,
        SimTime::EPOCH,
        &ResourceCompletionOperationV1::Abort(AbortCompletionLeaseV1 {
            acquisition: acquisition.clone(),
            expected_revision,
            holder: holder(1),
        }),
    )
    .expect("abort ingress");
    canwu
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("abort boundary");
    assert_eq!(
        resource_completion_status(&canwu, &holder(1), &acquisition)
            .expect("released status")
            .state,
        CompletionLeaseAcquisitionStateV1::Released
    );
    let snapshot = canwu.snapshot_json().expect("snapshot");
    from_resource_snapshot_json(&snapshot, &[&plugin]).expect("completion restart");
}

#[test]
fn completion_grant_expires_without_coordinator_follow_up() {
    let value = fixture(10, 0);
    let request = RequestCompletionLeaseV1 {
        recipe: CompletionCapacityRecipeV1 {
            receipts: MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE,
            mutations: 2,
            reports_per_holder: 0,
            holders: 0,
            bytes: 1_024,
        },
        ..lease_request(
            1,
            "test:lease:autonomous-expiry",
            SimTime::EPOCH,
            CompletionPolicyClassV1::Guaranteed,
        )
    };
    let acquisition = request.id.clone();
    let account = value.account.clone();
    let scenario = Scenario::new(SimTime::EPOCH, vec![EntityRef::Person(PersonId::new(1))])
        .with_domain_records(vec![value.state.into_record().expect("record")]);
    let plugin = ResourcePlugin::default();
    let mut canwu =
        canwu_api::Canwu::new_with_plugins(31, scenario, &[&plugin]).expect("resource runtime");
    enqueue_resource_completion_operation(
        &mut canwu,
        SimTime::EPOCH,
        &ResourceCompletionOperationV1::Acquire(request),
    )
    .expect("acquire ingress");
    canwu
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("acquire boundary");
    let (_, state) = resource_state(&canwu).expect("query").expect("state");
    let grant = CompletionCapacityGrantId::new("test:grant:autonomous-expiry").expect("grant");
    enqueue_resource_completion_operation(
        &mut canwu,
        SimTime::EPOCH,
        &ResourceCompletionOperationV1::Grant(GrantCompletionCapacityV1 {
            grant_id: grant,
            acquisition: acquisition.clone(),
            expected_acquisition_revision: state.completion_leases.acquisitions[&acquisition]
                .revision,
            owner_plugin: PLUGIN_NAME.to_owned(),
            target_versions: vec![CompletionLockedTargetV1::Account {
                id: account,
                revision: ResourceRevision::INITIAL,
            }],
            current_boundary: 2,
        }),
    )
    .expect("grant ingress");
    canwu
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("grant boundary");
    for _ in 0..10 {
        canwu
            .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
            .expect("autonomous expiry tick");
    }
    let (_, state) = resource_state(&canwu).expect("query").expect("state");
    assert_eq!(
        state.completion_leases.acquisitions[&acquisition].state,
        CompletionLeaseAcquisitionStateV1::Expired
    );
    assert_eq!(state.completion_leases.reserved_units, 0);
    assert!(state.completion_leases.target_locks.is_empty());
}

#[test]
fn external_coordinator_certificate_consumes_once_and_completes_resource_participant() {
    let mut value = fixture(100, 0);
    let demand_id = ResourceDemandId::new("test:demand:external-force").expect("demand");
    value
        .state
        .install_demand(demand(
            demand_id.as_str(),
            25,
            1,
            PartialFulfillmentPolicy::AcceptPartial,
            &value,
            None,
        ))
        .expect("demand");
    allocate(&mut value.state, "test:allocate:external-force");
    let leg = only_leg(&value.state);
    let acquisition =
        CompletionLeaseAcquisitionId::new("test:lease:external-force").expect("acquisition");
    let grant_id =
        CompletionCapacityGrantId::new("test:grant:external-force-resource").expect("grant");
    let coordinator_source = external_version("test:force-runtime");
    let operation_key =
        ResourceOperationKey::new("test:consume:external-force").expect("operation");
    let resource_targets = vec![
        CompletionLockedTargetV1::Account {
            id: leg.account.clone(),
            revision: leg.account_revision,
        },
        CompletionLockedTargetV1::AllocationLeg {
            id: leg.id.clone(),
            revision: leg.revision,
        },
        CompletionLockedTargetV1::Demand {
            id: demand_id.clone(),
            revision: value.state.demands[&demand_id].revision,
        },
    ];
    let recipe = CompletionCapacityRecipeV1 {
        receipts: MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE,
        mutations: 4,
        reports_per_holder: 0,
        holders: 0,
        bytes: 2_048,
    };
    let envelope = EligibilityEnvelopeV1::new(
        vec![coordinator_source.clone()],
        BTreeMap::new(),
        BTreeSet::new(),
        Vec::new(),
        Vec::new(),
    )
    .expect("envelope");
    let grant = value
        .state
        .apply_operation(&ResourceOperationRequestV1::Completion(
            ResourceCompletionOperationV1::GrantExternalParticipant(
                RequestExternalCompletionParticipantGrantV1 {
                    coordinator_plugin: "test.provider".to_owned(),
                    coordinator_source: coordinator_source.clone(),
                    coordinator_acquisition_revision: ResourceRevision::INITIAL,
                    acquisition: acquisition.clone(),
                    operation_key: operation_key.clone(),
                    holder: holder(1),
                    operation_namespace: "test.resource".to_owned(),
                    eligibility_time: SimTime::EPOCH,
                    eligibility_envelope_digest: envelope.digest.clone(),
                    recipe: recipe.clone(),
                    policy_class: CompletionPolicyClassV1::Guaranteed,
                    grant_id: grant_id.clone(),
                    target_versions: resource_targets.clone(),
                    current_boundary: 1,
                },
            ),
        ))
        .expect("external grant");
    assert_eq!(grant.status, ResourceOperationStatus::Applied);
    value
        .state
        .apply_operation(&ResourceOperationRequestV1::Completion(
            ResourceCompletionOperationV1::PrepareExternalParticipant(
                PrepareExternalCompletionParticipantGrantV1 {
                    coordinator_source: coordinator_source.clone(),
                    acquisition: acquisition.clone(),
                    expected_grant_revision: value.state.external_completion_participants.grants
                        [&acquisition]
                        .grant
                        .revision,
                    current_boundary: 2,
                    eligibility_envelope_digest: envelope.digest.clone(),
                },
            ),
        ))
        .expect("external prepare");
    let prepared = value.state.external_completion_participants.grants[&acquisition]
        .grant
        .clone();
    let force_grant =
        CompletionCapacityGrantId::new("test:grant:external-force-coordinator").expect("grant");
    let mut certificate = CompletionLeaseActivationCertificateV1 {
        acquisition: acquisition.clone(),
        acquisition_revision: ResourceRevision::new(4).expect("revision"),
        operation_key: operation_key.clone(),
        recipe_digest: recipe.digest().expect("recipe digest"),
        eligibility_time: SimTime::EPOCH,
        eligibility_envelope_digest: envelope.digest,
        prepared_grants: vec![
            (force_grant, ResourceRevision::INITIAL),
            (prepared.id.clone(), prepared.revision),
        ],
        locked_target_versions: std::iter::once(CompletionLockedTargetV1::ExternalRecord {
            version: coordinator_source.clone(),
        })
        .chain(resource_targets.iter().cloned())
        .collect(),
        activation_boundary: 3,
        semantic_digest: String::new(),
    };
    certificate.semantic_digest = canonical_digest(
        "canwu.resource.completion-activation-certificate.v1",
        &certificate,
    )
    .expect("certificate digest");
    value
        .state
        .apply_operation(&ResourceOperationRequestV1::Completion(
            ResourceCompletionOperationV1::ConsumeExternalParticipant(
                ConsumeExternalCompletionParticipantGrantV1 {
                    coordinator_source: coordinator_source.clone(),
                    certificate: certificate.clone(),
                    at: SimTime::EPOCH,
                },
            ),
        ))
        .expect("external consume grant");
    let request = ResourceOperationRequestV1::Consume(ResourceConsumptionRequestV1 {
        operation_key: operation_key.clone(),
        consumption_id: ResourceConsumptionId::new("test:consumption:external-force")
            .expect("consumption"),
        allocation: leg,
        expected_account_revision: ResourceRevision::INITIAL,
        consumer_evidence: coordinator_source,
        at: SimTime::EPOCH,
        completion_certificate: certificate,
    });
    let applied = value.state.apply_operation(&request).expect("consume");
    assert_eq!(applied.status, ResourceOperationStatus::Applied);
    let duplicate = value.state.apply_operation(&request).expect("duplicate");
    assert_eq!(duplicate.id, applied.id);
    assert_eq!(duplicate.semantic_digest, applied.semantic_digest);
    assert_eq!(value.state.accounts[&value.account].balance, 75);
    assert_eq!(
        value
            .state
            .external_completion_participants
            .participant(&acquisition)
            .expect("completed participant")
            .grant
            .state,
        CompletionGrantStateV1::Completed
    );
}

#[test]
fn external_participant_prepare_revalidates_resource_owned_targets_as_stable_rejection() {
    let mut value = fixture(10, 0);
    let acquisition =
        CompletionLeaseAcquisitionId::new("test:lease:external-stale-target").expect("lease");
    let coordinator_source = external_version("external-stale-target");
    let envelope = EligibilityEnvelopeV1::new(
        vec![coordinator_source.clone()],
        BTreeMap::new(),
        BTreeSet::new(),
        Vec::new(),
        Vec::new(),
    )
    .expect("envelope");
    value
        .state
        .apply_operation(&ResourceOperationRequestV1::Completion(
            ResourceCompletionOperationV1::GrantExternalParticipant(
                RequestExternalCompletionParticipantGrantV1 {
                    coordinator_plugin: "test.provider".to_owned(),
                    coordinator_source: coordinator_source.clone(),
                    coordinator_acquisition_revision: ResourceRevision::INITIAL,
                    acquisition: acquisition.clone(),
                    operation_key: ResourceOperationKey::new(
                        "test:operation:external-stale-target",
                    )
                    .expect("operation"),
                    holder: holder(1),
                    operation_namespace: "test.resource".to_owned(),
                    eligibility_time: SimTime::EPOCH,
                    eligibility_envelope_digest: envelope.digest.clone(),
                    recipe: CompletionCapacityRecipeV1 {
                        receipts: MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE,
                        mutations: 1,
                        reports_per_holder: 0,
                        holders: 0,
                        bytes: 1_024,
                    },
                    policy_class: CompletionPolicyClassV1::Guaranteed,
                    grant_id: CompletionCapacityGrantId::new("test:grant:external-stale-target")
                        .expect("grant"),
                    target_versions: vec![CompletionLockedTargetV1::Account {
                        id: value.account.clone(),
                        revision: ResourceRevision::INITIAL,
                    }],
                    current_boundary: 1,
                },
            ),
        ))
        .expect("grant external participant");
    value
        .state
        .accounts
        .get_mut(&value.account)
        .expect("account")
        .revision = ResourceRevision::new(2).expect("revision");
    let expected_grant_revision = value.state.external_completion_participants.grants[&acquisition]
        .grant
        .revision;
    let outcome = value
        .state
        .apply_operation(&ResourceOperationRequestV1::Completion(
            ResourceCompletionOperationV1::PrepareExternalParticipant(
                PrepareExternalCompletionParticipantGrantV1 {
                    coordinator_source,
                    acquisition: acquisition.clone(),
                    expected_grant_revision,
                    current_boundary: 2,
                    eligibility_envelope_digest: envelope.digest,
                },
            ),
        ))
        .expect("stale target is a durable participant rejection");
    assert_eq!(outcome.status, ResourceOperationStatus::Applied);
    assert_eq!(
        value.state.external_completion_participants.grants[&acquisition]
            .grant
            .state,
        CompletionGrantStateV1::Rejected
    );
    assert_eq!(
        value.state.external_completion_participants.reserved_units,
        0
    );
}

#[test]
fn completed_external_participants_reuse_pending_admission_capacity() {
    let budget = budget();
    let recipe = CompletionCapacityRecipeV1 {
        receipts: MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE,
        mutations: 1,
        reports_per_holder: 0,
        holders: 0,
        bytes: 1_024,
    };
    let target = CompletionLockedTargetV1::ExternalRecord {
        version: external_version("reusable-participant-target"),
    };
    let coordinator_source = external_version("reusable-participant-coordinator");
    let envelope_digest = digest('a');
    let mut book = ExternalCompletionParticipantBookV1::default();

    for index in 0..=MAX_PENDING_LEASE_ACQUISITIONS_GLOBAL {
        let acquisition =
            CompletionLeaseAcquisitionId::new(format!("test:reusable-participant:{index:05}"))
                .expect("acquisition");
        let operation_key =
            ResourceOperationKey::new(format!("test:reusable-operation:{index:05}"))
                .expect("operation key");
        let grant = book
            .grant(
                &budget,
                RequestExternalCompletionParticipantGrantV1 {
                    coordinator_plugin: "test.provider".to_owned(),
                    coordinator_source: coordinator_source.clone(),
                    coordinator_acquisition_revision: ResourceRevision::INITIAL,
                    acquisition: acquisition.clone(),
                    operation_key: operation_key.clone(),
                    holder: holder(1),
                    operation_namespace: "test.resource".to_owned(),
                    eligibility_time: SimTime::EPOCH,
                    eligibility_envelope_digest: envelope_digest.clone(),
                    recipe: recipe.clone(),
                    policy_class: CompletionPolicyClassV1::Guaranteed,
                    grant_id: CompletionCapacityGrantId::new(format!(
                        "test:reusable-grant:{index:05}"
                    ))
                    .expect("grant ID"),
                    target_versions: vec![target.clone()],
                    current_boundary: 1,
                },
            )
            .expect("completed participants must not consume pending admission capacity");
        let prepared = book
            .prepare(PrepareExternalCompletionParticipantGrantV1 {
                coordinator_source: coordinator_source.clone(),
                acquisition: acquisition.clone(),
                expected_grant_revision: grant.grant.revision,
                current_boundary: 2,
                eligibility_envelope_digest: envelope_digest.clone(),
            })
            .expect("prepare participant");
        let mut certificate = CompletionLeaseActivationCertificateV1 {
            acquisition: acquisition.clone(),
            acquisition_revision: ResourceRevision::INITIAL,
            operation_key: operation_key.clone(),
            prepared_grants: vec![(prepared.grant.id.clone(), prepared.grant.revision)],
            locked_target_versions: vec![target.clone()],
            recipe_digest: recipe.digest().expect("recipe digest"),
            eligibility_time: SimTime::EPOCH,
            eligibility_envelope_digest: envelope_digest.clone(),
            activation_boundary: 3,
            semantic_digest: String::new(),
        };
        certificate.semantic_digest = canonical_digest(
            "canwu.resource.completion-activation-certificate.v1",
            &certificate,
        )
        .expect("certificate digest");
        book.consume(ConsumeExternalCompletionParticipantGrantV1 {
            coordinator_source: coordinator_source.clone(),
            certificate,
            at: SimTime::EPOCH,
        })
        .expect("consume participant");
        book.complete(&CompleteExternalCompletionParticipantGrantV1 {
            acquisition,
            operation_key,
        })
        .expect("complete participant");
        assert!(book.grants.is_empty());
        assert_eq!(book.reserved_units, 0);
    }

    assert_eq!(
        book.terminal_grants.len(),
        MAX_PENDING_LEASE_ACQUISITIONS_GLOBAL + 1
    );
    book.validate(&budget)
        .expect("terminal participant history remains valid outside the pending cap");
}

#[test]
fn completed_external_participant_archives_and_restores_authoritatively() {
    let mut value = fixture(10, 0);
    let acquisition = complete_external_participant(&mut value.state, "archive");
    assert!(
        !value
            .state
            .external_completion_participants
            .grants
            .contains_key(&acquisition)
    );
    assert!(
        value
            .state
            .external_completion_participants
            .terminal_grants
            .contains_key(&acquisition)
    );
    assert!(value.state.terminal_archive_candidates.values().any(|key| {
        key == &ResourceTerminalRecordKeyV1::ExternalCompletionParticipant(acquisition.clone())
    }));
    value.state.validate().expect("completed participant state");

    let prepared = value
        .state
        .prepare_resource_archive(value.state.terminal_archive_candidates.len())
        .expect("archive completed participant");
    let archived_participant = prepared
        .blob
        .records
        .iter()
        .find(|record| {
            record.key
                == ResourceTerminalRecordKeyV1::ExternalCompletionParticipant(acquisition.clone())
        })
        .and_then(|record| match &record.payload {
            ResourceTerminalArchivePayloadV1::ExternalCompletionParticipant(participant) => {
                Some(participant)
            }
            _ => None,
        })
        .expect("archive carries the exact completed participant");
    assert_eq!(
        archived_participant.grant.state,
        CompletionGrantStateV1::Completed
    );

    let store = MemoryArchiveStore::default();
    let plugin = ResourcePlugin::default();
    let scenario = Scenario::new(SimTime::EPOCH, Vec::new())
        .with_domain_records(vec![value.state.into_record().expect("resource record")]);
    let mut canwu =
        canwu_api::Canwu::new_with_plugins(34, scenario, &[&plugin]).expect("archive runtime");
    enqueue_resource_archive(&mut canwu, &prepared, &store).expect("enqueue participant archive");
    canwu
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("participant archive boundary");
    let (_, archived_state) = resource_state(&canwu)
        .expect("resource query")
        .expect("archived resource state");
    assert!(
        archived_state
            .external_completion_participants
            .participant(&acquisition)
            .is_none()
    );
    validate_resource_archive_store(&archived_state, &store)
        .expect("completed participant archive authenticates");

    let snapshot = canwu.snapshot_json().expect("participant archive snapshot");
    let restored = from_resource_snapshot_json_with_archive_store(
        &snapshot,
        &[&plugin],
        Rc::new(store.clone()),
    )
    .expect("participant archive restore");
    let (_, restored_state) = resource_state(&restored)
        .expect("restored resource query")
        .expect("restored resource state");
    assert!(
        restored_state
            .external_completion_participants
            .participant(&acquisition)
            .is_none()
    );
    validate_resource_archive_store(&restored_state, &store)
        .expect("restored participant archive authenticates");
}

#[test]
fn canonical_external_participant_ingress_requires_exact_coordinator_payload_and_replays() {
    let value = fixture(100, 0);
    let acquisition =
        CompletionLeaseAcquisitionId::new("test:lease:canonical-provider").expect("acquisition");
    let operation_key =
        ResourceOperationKey::new("test:operation:canonical-provider").expect("operation");
    let provider_source = external_version("canonical-coordinator");
    let recipe = CompletionCapacityRecipeV1 {
        receipts: MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE,
        mutations: 4,
        reports_per_holder: 0,
        holders: 0,
        bytes: 2_048,
    };
    let envelope = EligibilityEnvelopeV1::new(
        vec![provider_source.clone()],
        BTreeMap::new(),
        BTreeSet::new(),
        Vec::new(),
        Vec::new(),
    )
    .expect("envelope");
    let authoritative = CompletionLeaseAcquisitionV1 {
        id: acquisition.clone(),
        revision: ResourceRevision::INITIAL,
        operation_key: operation_key.clone(),
        holder: holder(1),
        operation_namespace: "test.resource".to_owned(),
        eligibility_time: SimTime::EPOCH,
        eligibility_envelope: envelope.clone(),
        recipe: recipe.clone(),
        recipe_digest: recipe.digest().expect("recipe digest"),
        expected_participants: BTreeSet::from(["test.provider".to_owned(), PLUGIN_NAME.to_owned()]),
        grants: BTreeMap::new(),
        policy_class: CompletionPolicyClassV1::Guaranteed,
        admitted_sequence: 1,
        fairness_round: 0,
        state: CompletionLeaseAcquisitionStateV1::Requested,
        blocker: None,
        refunded_units: 0,
    };
    let provider = DomainRecord {
        reference: provider_source.record.clone(),
        owner: "test.provider".to_owned(),
        class: DomainRecordClass::Record,
        version: provider_source.version,
        lifecycle: DomainRecordLifecycle::Active,
        payload: serde_json::json!({
            "completion_acquisitions": {
                acquisition.as_str(): authoritative,
            },
        }),
        references: Vec::new(),
    };
    let request = RequestExternalCompletionParticipantGrantV1 {
        coordinator_plugin: "test.provider".to_owned(),
        coordinator_source: provider_source.clone(),
        coordinator_acquisition_revision: ResourceRevision::INITIAL,
        acquisition: acquisition.clone(),
        operation_key: operation_key.clone(),
        holder: holder(1),
        operation_namespace: "test.resource".to_owned(),
        eligibility_time: SimTime::EPOCH,
        eligibility_envelope_digest: envelope.digest,
        recipe,
        policy_class: CompletionPolicyClassV1::Guaranteed,
        grant_id: CompletionCapacityGrantId::new("test:grant:canonical-provider-resource")
            .expect("grant"),
        target_versions: vec![CompletionLockedTargetV1::ExternalRecord {
            version: provider_source,
        }],
        current_boundary: 1,
    };
    let resource_record = value.state.into_record().expect("resource record");
    let scenario = Scenario::new(SimTime::EPOCH, Vec::new())
        .with_domain_records(vec![resource_record.clone(), provider.clone()]);
    let plugin = ResourcePlugin::new([DomainRecordKind::new("test.provider", "execution")]);
    let mut canwu =
        canwu_api::Canwu::new_with_plugins(31, scenario, &[&plugin]).expect("canonical runtime");
    enqueue_resource_completion_operation(
        &mut canwu,
        SimTime::EPOCH,
        &ResourceCompletionOperationV1::GrantExternalParticipant(request.clone()),
    )
    .expect("canonical external grant ingress");
    canwu
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("canonical external grant boundary");
    let (_, state) = resource_state(&canwu)
        .expect("resource query")
        .expect("resource state");
    assert_eq!(
        state.external_completion_participants.grants[&acquisition]
            .grant
            .state,
        CompletionGrantStateV1::Held
    );
    let restored_scenario = Scenario::new(SimTime::EPOCH, Vec::new()).with_domain_records(vec![
        state
            .clone()
            .into_record()
            .expect("restored resource record"),
    ]);
    let restarted =
        canwu_api::Canwu::new_with_plugins(33, restored_scenario, &[&ResourcePlugin::default()])
            .expect("restart");
    assert!(
        resource_state(&restarted)
            .expect("restarted query")
            .expect("restarted resource state")
            .1
            .external_completion_participants
            .grants
            .contains_key(&acquisition)
    );

    let forged_scenario = Scenario::new(SimTime::EPOCH, Vec::new())
        .with_domain_records(vec![resource_record, provider]);
    let mut forged = canwu_api::Canwu::new_with_plugins(32, forged_scenario, &[&plugin])
        .expect("forged runtime");
    let mut forged_request = request;
    forged_request.holder = holder(2);
    enqueue_resource_completion_operation(
        &mut forged,
        SimTime::EPOCH,
        &ResourceCompletionOperationV1::GrantExternalParticipant(forged_request),
    )
    .expect("forged external grant ingress queues as untrusted input");
    let error = forged
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect_err("forged provider packet must fail before resource mutation");
    assert_eq!(error.code, ErrorCode::InvalidAuthority);
}

#[test]
fn forged_restore_candidate_is_rejected() {
    let mut state = ResourceState::empty(ResourceLimitsV1::canonical()).expect("state");
    rejected_operation(&mut state, "restore");
    let mut record = state.into_record().expect("record");
    record.payload["outcomes"]["test:reject:restore"]["quantity"] = serde_json::json!(1);
    let scenario = Scenario::new(SimTime::EPOCH, Vec::new()).with_domain_records(vec![record]);
    let result = canwu_api::Canwu::new_with_plugins(17, scenario, &[&ResourcePlugin::default()]);
    let error = match result {
        Ok(_) => panic!("forged resource state unexpectedly activated"),
        Err(error) => error,
    };
    assert_eq!(error.code, ErrorCode::InvalidDomainRecord);
}
