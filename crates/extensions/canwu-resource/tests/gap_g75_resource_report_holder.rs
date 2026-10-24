//! Downstream gap G-75: a completion lease reserves its holder's report, so a
//! holder that can never receive knowledge must be refused before the lease
//! exists, never when its report is published (a failed boundary). A report
//! grant to a holder of a kind that holds no knowledge is refused at install;
//! a completion that reserves reports for a holder the engine refuses now is
//! the operation's recorded rejection, and the boundary settles. Two
//! completions of one holder that reserved the same report grant, which has
//! no observation head, both give their report up in the same boundary.

use canwu_api::{
    BoundaryRequest, Canwu, DomainRecordRef, DomainRecordVersionRef, DomainRecordVersionSource,
    EntityRef, KnowledgeHolderRef, OrganizationId, PersonId, Scenario, SimTime,
};
use canwu_resource::*;
use std::collections::{BTreeMap, BTreeSet};

const NAMESPACE: &str = "fixture.completion";

fn person(value: u64) -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(PersonId::new(value))
}

fn report_grant(id: &str, holder: KnowledgeHolderRef) -> ResourceReportGrantV1 {
    ResourceReportGrantV1 {
        id: ResourceReportGrantId::new(id).expect("report grant ID"),
        holder,
        scope: ResourceScopeId::new("fixture:report-scope").expect("report scope"),
        accounts: BTreeSet::new(),
        demands: BTreeSet::new(),
        include_transfer_details: false,
        confidence_per_mille: 1_000,
        cadence_minutes: 60,
        delay_minutes: 0,
    }
}

fn recipe() -> CompletionCapacityRecipeV1 {
    CompletionCapacityRecipeV1 {
        receipts: MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE,
        mutations: 1,
        reports_per_holder: 1,
        holders: 1,
        bytes: 1_024,
    }
}

fn partition(authority: KnowledgeHolderRef, units: u64) -> CompletionCapacityPartitionV1 {
    CompletionCapacityPartitionV1 {
        authority,
        operation_namespace: NAMESPACE.to_owned(),
        guaranteed_units: units,
        reserved_pending_slots: 1,
        maximum_burst_units: units,
        request_token_capacity: 1,
        request_token_refill_minutes: 1,
        reacquire_cooldown_minutes: 1,
        root_acquisition_cap_per_sim_time: 1,
        guaranteed_max_wait_boundaries: 4,
    }
}

fn acquire(name: &str, holder: KnowledgeHolderRef) -> ResourceCompletionOperationV1 {
    ResourceCompletionOperationV1::Acquire(RequestCompletionLeaseV1 {
        id: CompletionLeaseAcquisitionId::new(format!("fixture:lease:{name}"))
            .expect("acquisition"),
        operation_key: ResourceOperationKey::new(format!("fixture:operation:{name}"))
            .expect("operation key"),
        holder,
        operation_namespace: NAMESPACE.to_owned(),
        eligibility_time: SimTime::EPOCH,
        eligibility_envelope: EligibilityEnvelopeV1::new(
            Vec::new(),
            BTreeMap::new(),
            BTreeSet::new(),
            Vec::new(),
            Vec::new(),
        )
        .expect("envelope"),
        recipe: recipe(),
        expected_participants: BTreeSet::from([PLUGIN_NAME.to_owned()]),
        policy_class: CompletionPolicyClassV1::Guaranteed,
    })
}

/// Takes one external participant of `holder` through grant, prepare,
/// consume, and completion on the state, as a coordinator would, so that it
/// waits for the report its recipe reserves.
fn completed_participant(state: &mut ResourceState, name: &str, holder: &KnowledgeHolderRef) {
    let acquisition =
        CompletionLeaseAcquisitionId::new(format!("fixture:participant:{name}")).expect("id");
    let operation_key = ResourceOperationKey::new(format!("fixture:participant-operation:{name}"))
        .expect("operation key");
    let source = DomainRecordVersionRef {
        record: DomainRecordRef::new("fixture.coordinator", "runtime", name),
        version: 1,
        established_by: DomainRecordVersionSource::InitialScenario,
    };
    let targets = vec![CompletionLockedTargetV1::ExternalRecord {
        version: source.clone(),
    }];
    let envelope_digest = "e".repeat(64);
    let step = |state: &mut ResourceState, operation| {
        let outcome = state
            .apply_operation(&ResourceOperationRequestV1::Completion(operation))
            .expect("participant step");
        assert_eq!(
            outcome.status,
            ResourceOperationStatus::Applied,
            "{:?}",
            outcome.rejection_reason
        );
    };
    step(
        state,
        ResourceCompletionOperationV1::GrantExternalParticipant(
            RequestExternalCompletionParticipantGrantV1 {
                coordinator_plugin: "fixture-coordinator".to_owned(),
                coordinator_source: source.clone(),
                coordinator_acquisition_revision: ResourceRevision::INITIAL,
                acquisition: acquisition.clone(),
                operation_key: operation_key.clone(),
                holder: holder.clone(),
                operation_namespace: NAMESPACE.to_owned(),
                eligibility_time: SimTime::EPOCH,
                eligibility_envelope_digest: envelope_digest.clone(),
                recipe: recipe(),
                policy_class: CompletionPolicyClassV1::Guaranteed,
                grant_id: CompletionCapacityGrantId::new(format!("fixture:grant:{name}"))
                    .expect("grant"),
                target_versions: targets.clone(),
                current_boundary: 1,
            },
        ),
    );
    let held = state.external_completion_participants.grants[&acquisition]
        .grant
        .clone();
    step(
        state,
        ResourceCompletionOperationV1::PrepareExternalParticipant(
            PrepareExternalCompletionParticipantGrantV1 {
                coordinator_source: source.clone(),
                acquisition: acquisition.clone(),
                expected_grant_revision: held.revision,
                current_boundary: 2,
                eligibility_envelope_digest: envelope_digest.clone(),
            },
        ),
    );
    let prepared = state.external_completion_participants.grants[&acquisition]
        .grant
        .clone();
    let mut certificate = CompletionLeaseActivationCertificateV1 {
        acquisition: acquisition.clone(),
        acquisition_revision: ResourceRevision::INITIAL,
        operation_key: operation_key.clone(),
        prepared_grants: vec![(prepared.id.clone(), prepared.revision)],
        locked_target_versions: targets,
        recipe_digest: recipe().digest().expect("recipe digest"),
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
    step(
        state,
        ResourceCompletionOperationV1::ConsumeExternalParticipant(
            ConsumeExternalCompletionParticipantGrantV1 {
                coordinator_source: source,
                certificate,
                at: SimTime::EPOCH,
            },
        ),
    );
    step(
        state,
        ResourceCompletionOperationV1::CompleteExternalParticipant(
            CompleteExternalCompletionParticipantGrantV1 {
                acquisition,
                operation_key,
            },
        ),
    );
}

#[test]
fn a_completion_report_holder_that_cannot_receive_knowledge_is_refused_before_the_lease() {
    let mut state = ResourceState::empty(ResourceLimitsV1::canonical()).expect("resource state");
    // An organization can never hold knowledge, so its report grant is
    // refused when it is installed.
    let organization = KnowledgeHolderRef::Entity(EntityRef::Organization(OrganizationId::new(1)));
    assert!(matches!(
        state.install_report_grant(report_grant("fixture:report:organization", organization)),
        Err(ResourceError::InvalidDefinition(_))
    ));
    // Person 1 is in the scenario; person 9 is not, so the engine would
    // refuse to publish to it.
    let units = recipe().canonical_units().expect("recipe units");
    state
        .install_run_budget(
            RunBudgetRevisionV1 {
                revision: ResourceRevision::INITIAL,
                total_completion_units: units * 6,
                shared_pending_slots: 0,
                partitions: vec![partition(person(1), units), partition(person(9), units)],
                semantic_digest: String::new(),
            }
            .seal()
            .expect("run budget"),
        )
        .expect("install run budget");
    for (id, holder) in [
        ("fixture:report:present", person(1)),
        ("fixture:report:absent", person(9)),
    ] {
        state
            .install_report_grant(report_grant(id, holder))
            .expect("report grant of a person");
    }
    // Two leases of person 1 completed, each waiting for the report it
    // reserved on person 1's one grant, which has no observation head.
    for name in ["first", "second"] {
        completed_participant(&mut state, name, &person(1));
    }
    let present_grant = ResourceReportGrantId::new("fixture:report:present").expect("grant ID");
    assert_eq!(state.completion_report_ready.len(), 2);
    assert!(
        state
            .completion_report_reservations
            .values()
            .all(|grants| grants.contains(&present_grant))
    );
    state.report_dirty_grants.clear();
    state.report_dirty_grants.insert(present_grant);
    let scenario = Scenario::new(SimTime::EPOCH, vec![EntityRef::Person(PersonId::new(1))])
        .with_domain_records(state.into_records().expect("resource records"));
    let mut canwu =
        Canwu::new_with_plugins(75, scenario, &[&ResourcePlugin::default()]).expect("runtime");
    let present = acquire("present", person(1));
    let absent = acquire("absent", person(9));
    for operation in [&present, &absent] {
        enqueue_resource_completion_operation(&mut canwu, SimTime::EPOCH, operation)
            .expect("completion ingress");
    }
    canwu
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("the boundary settles");
    let (_, state) = resource_state(&canwu)
        .expect("resource query")
        .expect("resource state");
    let outcome = |operation: &ResourceCompletionOperationV1| {
        state.outcomes[&ResourceOperationRequestV1::Completion(operation.clone()).operation_key()]
            .clone()
    };
    assert_eq!(outcome(&present).status, ResourceOperationStatus::Applied);
    let refused = outcome(&absent);
    assert_eq!(refused.status, ResourceOperationStatus::Rejected);
    assert_eq!(refused.rejection_code.as_deref(), Some("invalid_authority"));
    assert!(
        refused
            .rejection_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("cannot receive knowledge")),
        "{:?}",
        refused.rejection_reason
    );
    // The refused lease reserved nothing, and both completions gave their
    // reports up in the one boundary: no reservation or readiness of theirs
    // is left, and both participants are archive candidates.
    assert_eq!(state.completion_report_reservations.len(), 1);
    assert_eq!(state.completion_leases.acquisitions.len(), 1);
    assert!(state.completion_report_ready.is_empty());
    for name in ["first", "second"] {
        let acquisition =
            CompletionLeaseAcquisitionId::new(format!("fixture:participant:{name}")).expect("id");
        assert!(
            !state
                .completion_report_reservations
                .contains_key(&acquisition)
        );
        assert!(
            state.terminal_archive_candidates.values().any(|key| *key
                == ResourceTerminalRecordKeyV1::ExternalCompletionParticipant(acquisition.clone())),
            "participant {name} is an archive candidate"
        );
    }
}
