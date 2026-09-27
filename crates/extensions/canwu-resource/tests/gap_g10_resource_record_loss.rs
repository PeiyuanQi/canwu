//! Gap G-10: an account-level loss settles in place through the resource
//! lifecycle writer, is counted as admitted loss, respects the protected floor
//! unless explicitly overridden, rejects a stale account revision, reaches
//! holder reports as a loss observation, and survives save/load and replay.

#![allow(
    clippy::needless_pass_by_value,
    clippy::similar_names,
    clippy::too_many_lines
)]

use canwu_api::{
    BoundaryRequest, CanwuError, CommandEnvelope, CommandRequest, CommandRequestId, DomainRecord,
    DomainRecordClass, DomainRecordLifecycle, DomainRecordRef, DomainRecordSchema,
    DomainRecordType, DomainRecordVersionRef, DomainRecordVersionSource, DomainValueKindClass,
    EntityRef, EvidenceRef, Issuer, KnowledgeHolderRef, PayloadSchema, PersonId, PluginRegistrar,
    Scenario, SimDuration, SimTime, SimulationPlugin,
};
use canwu_resource::*;
use std::collections::{BTreeMap, BTreeSet};

const NAMESPACE: &str = "test.resource";

fn digest(value: char) -> String {
    value.to_string().repeat(64)
}

fn holder() -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(PersonId::new(1))
}

fn minute(value: i64) -> SimTime {
    SimTime::EPOCH + SimDuration::minutes(value)
}

fn key(value: &str) -> ResourceOperationKey {
    ResourceOperationKey::new(value).expect("operation key")
}

fn cause(id: &str) -> DomainRecordVersionRef {
    DomainRecordVersionRef {
        record: DomainRecordRef::new("test.provider", "evidence", id),
        version: 1,
        established_by: DomainRecordVersionSource::InitialScenario,
    }
}

fn cause_record(version: &DomainRecordVersionRef) -> DomainRecord {
    DomainRecord {
        reference: version.record.clone(),
        owner: "test-provider".to_owned(),
        class: DomainRecordClass::Record,
        version: version.version,
        lifecycle: DomainRecordLifecycle::Active,
        payload: serde_json::json!({ "cause": version.record.id }),
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

/// One account holding 100 units behind a protected floor of 60.
fn world() -> (ResourceState, ResourceAccountId) {
    let mut state = ResourceState::empty(ResourceLimitsV1::canonical()).expect("state");
    state
        .install_run_budget(
            RunBudgetRevisionV1 {
                revision: ResourceRevision::INITIAL,
                total_completion_units: 1_000_000,
                shared_pending_slots: 4,
                partitions: vec![CompletionCapacityPartitionV1 {
                    authority: holder(),
                    operation_namespace: NAMESPACE.to_owned(),
                    guaranteed_units: 400_000,
                    reserved_pending_slots: 4,
                    maximum_burst_units: 100_000,
                    request_token_capacity: 16,
                    request_token_refill_minutes: 1,
                    reacquire_cooldown_minutes: 1,
                    root_acquisition_cap_per_sim_time: 4,
                    guaranteed_max_wait_boundaries: 4,
                }],
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
            scope: ResourceScopeId::new("test:scope:store").expect("scope"),
            effective_from: SimTime::EPOCH,
            effective_until: None,
            process_suitability: BTreeSet::new(),
            semantic_digest: digest('1'),
        })
        .expect("definition");
    let floor = ProtectedFloorPolicyRevisionId::new("test:floor:reserve:v1").expect("floor");
    state
        .install_protected_floor_policy(ProtectedFloorPolicyRevision {
            id: floor.clone(),
            revision: ResourceRevision::INITIAL,
            floor: 60,
            override_classes: BTreeSet::new(),
            semantic_digest: digest('2'),
        })
        .expect("floor policy");
    let account = ResourceAccountId::new("test:account:store").expect("account");
    state
        .install_opening_account(ResourceAccount {
            id: account.clone(),
            revision: ResourceRevision::INITIAL,
            custodian: holder(),
            resource_revision: resource,
            unit_revision: unit,
            balance: 100,
            capacity: None,
            protected_floor_policy: Some(floor),
            closed: false,
            place_scope: None,
        })
        .expect("account");
    (state, account)
}

/// Acquires, grants, prepares, and activates one resource-owned completion
/// lease that locks exactly `targets` for `operation_key`.
fn certificate(
    state: &mut ResourceState,
    suffix: &str,
    operation_key: &ResourceOperationKey,
    targets: Vec<CompletionLockedTargetV1>,
    at: SimTime,
    boundary: u64,
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
                holder: holder(),
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
    let acquisition_revision = state.completion_leases.acquisitions[&acquisition].revision;
    applied(
        state,
        ResourceOperationRequestV1::Completion(ResourceCompletionOperationV1::Grant(
            GrantCompletionCapacityV1 {
                grant_id: grant.clone(),
                acquisition: acquisition.clone(),
                expected_acquisition_revision: acquisition_revision,
                owner_plugin: PLUGIN_NAME.to_owned(),
                target_versions: targets,
                current_boundary: boundary,
            },
        )),
    );
    let acquisition_revision = state.completion_leases.acquisitions[&acquisition].revision;
    let grant_revision = state.completion_leases.grants[&grant].revision;
    applied(
        state,
        ResourceOperationRequestV1::Completion(ResourceCompletionOperationV1::Prepare(
            PrepareCompletionCapacityV1 {
                acquisition: acquisition.clone(),
                expected_acquisition_revision: acquisition_revision,
                grant: grant.clone(),
                expected_grant_revision: grant_revision,
                current_boundary: boundary + 1,
                eligibility_envelope_digest: envelope.digest.clone(),
            },
        )),
    );
    let acquisition_revision = state.completion_leases.acquisitions[&acquisition].revision;
    let grant_revision = state.completion_leases.grants[&grant].revision;
    applied(
        state,
        ResourceOperationRequestV1::Completion(ResourceCompletionOperationV1::Activate(
            ActivateCompletionLeaseV1 {
                acquisition: acquisition.clone(),
                expected_acquisition_revision: acquisition_revision,
                grant,
                expected_grant_revision: grant_revision,
                at,
                current_boundary: boundary + 2,
                eligibility_envelope_digest: envelope.digest,
            },
        )),
    );
    state.completion_leases.certificates[&acquisition].clone()
}

fn loss_targets(
    account: &ResourceAccountId,
    revision: ResourceRevision,
    cause: &DomainRecordVersionRef,
) -> Vec<CompletionLockedTargetV1> {
    vec![
        CompletionLockedTargetV1::Account {
            id: account.clone(),
            revision,
        },
        CompletionLockedTargetV1::ExternalRecord {
            version: cause.clone(),
        },
    ]
}

#[allow(clippy::too_many_arguments)]
fn loss_request(
    operation_key: &ResourceOperationKey,
    loss_id: &str,
    account: &ResourceAccountId,
    expected_account_revision: ResourceRevision,
    quantity: u64,
    cause: &DomainRecordVersionRef,
    allow_protected: bool,
    at: SimTime,
    completion_certificate: CompletionLeaseActivationCertificateV1,
) -> ResourceOperationRequestV1 {
    ResourceOperationRequestV1::RecordLoss(ResourceAccountLossRequestV1 {
        operation_key: operation_key.clone(),
        loss_id: ResourceLossId::new(loss_id).expect("loss"),
        account: account.clone(),
        expected_account_revision,
        quantity,
        cause: EvidenceRef::DomainRecordVersion(cause.clone()),
        allow_protected,
        at,
        completion_certificate,
    })
}

/// Advances the account revision without changing its balance or floor.
fn touch_account(state: &mut ResourceState, account: &ResourceAccountId, operation: &str) {
    let expected_account_revision = state.accounts[account].revision;
    let policy = state.accounts[account].protected_floor_policy.clone();
    applied(
        state,
        ResourceOperationRequestV1::SetProtectedFloor(ResourceProtectedFloorRequestV1 {
            operation_key: key(operation),
            account: account.clone(),
            expected_account_revision,
            policy,
        }),
    );
}

#[test]
fn gap_g10_resource_record_loss() {
    let (mut state, account) = world();
    let opening = state.conservation;
    let causes: Vec<_> = (1..=4).map(|n| cause(&format!("loss-{n}"))).collect();

    // The protected floor holds: only 40 of 100 units are losable.
    let below_floor = key("test:loss:below-floor");
    let revision = state.accounts[&account].revision;
    let lease = certificate(
        &mut state,
        "below-floor",
        &below_floor,
        loss_targets(&account, revision, &causes[0]),
        minute(1),
        10,
    );
    let request = loss_request(
        &below_floor,
        "test:loss:below-floor",
        &account,
        revision,
        50,
        &causes[0],
        false,
        minute(1),
        lease,
    );
    let rejected = state.apply_operation(&request).expect("durable rejection");
    assert_eq!(rejected.kind, ResourceOperationKind::Loss);
    assert_eq!(rejected.status, ResourceOperationStatus::Rejected);
    assert_eq!(rejected.rejection_code.as_deref(), Some("protected_floor"));
    assert_eq!(state.apply_operation(&request).expect("replay"), rejected);
    assert_eq!(state.accounts[&account].balance, 100);
    assert_eq!(state.conservation, opening);
    assert!(state.losses.is_empty());

    // A loss bound to a stale account revision is rejected even with a
    // certificate for that revision.
    touch_account(&mut state, &account, "test:floor:touch-1");
    let stale = key("test:loss:stale");
    let stale_revision = state.accounts[&account].revision;
    let lease = certificate(
        &mut state,
        "stale",
        &stale,
        loss_targets(&account, stale_revision, &causes[1]),
        minute(2),
        20,
    );
    touch_account(&mut state, &account, "test:floor:touch-2");
    let rejected = state
        .apply_operation(&loss_request(
            &stale,
            "test:loss:stale",
            &account,
            stale_revision,
            10,
            &causes[1],
            false,
            minute(2),
            lease,
        ))
        .expect("durable stale rejection");
    assert_eq!(rejected.status, ResourceOperationStatus::Rejected);
    assert_eq!(rejected.rejection_code.as_deref(), Some("version_conflict"));
    assert_eq!(state.accounts[&account].balance, 100);
    assert_eq!(state.conservation, opening);

    // A loss inside available stock settles in place and conserves.
    let within = key("test:loss:within-available");
    let revision = state.accounts[&account].revision;
    let lease = certificate(
        &mut state,
        "within-available",
        &within,
        loss_targets(&account, revision, &causes[2]),
        minute(3),
        30,
    );
    let request = loss_request(
        &within,
        "test:loss:within-available",
        &account,
        revision,
        30,
        &causes[2],
        false,
        minute(3),
        lease,
    );
    let outcome = state.apply_operation(&request).expect("loss");
    let loss_id = ResourceLossId::new("test:loss:within-available").expect("loss");
    assert_eq!(outcome.status, ResourceOperationStatus::Applied);
    assert_eq!(outcome.quantity, 30);
    assert_eq!(
        outcome.result_ref,
        Some(ResourceRecordRefV1::Loss(loss_id.clone()))
    );
    assert_eq!(outcome.exact_evidence, vec![causes[2].clone()]);
    assert_eq!(state.apply_operation(&request).expect("replay"), outcome);
    let loss = &state.losses[&loss_id];
    assert_eq!(
        (loss.account.as_ref(), loss.transfer.as_ref(), loss.quantity),
        (Some(&account), None, 30)
    );
    assert_eq!(state.accounts[&account].balance, 70);
    assert_eq!(state.conservation.admitted_loss, 30);
    state.validate().expect("conserved state");

    // Holder reports carry the loss as observed evidence and reject a forged one.
    let mut observed = state.clone();
    let grant = ResourceReportGrantId::new("test:report-grant:store").expect("grant");
    observed
        .install_report_grant(ResourceReportGrantV1 {
            id: grant.clone(),
            holder: holder(),
            scope: ResourceScopeId::new("test:scope:store").expect("scope"),
            accounts: BTreeSet::from([account.clone()]),
            demands: BTreeSet::new(),
            include_transfer_details: true,
            confidence_per_mille: 1_000,
            cadence_minutes: 60,
            delay_minutes: 0,
        })
        .expect("report grant");
    let source = cause("observation");
    let head = ResourceObservationHeadV1 {
        id: ResourceObservationHeadId::new("test:observation:store").expect("head"),
        revision: ResourceRevision::INITIAL,
        provider_plugin: "test-provider".to_owned(),
        provider_version: "1".to_owned(),
        provider_semantic_hash: digest('a'),
        provider_source: source.clone(),
        holder: holder(),
        grant: grant.clone(),
        provider_state_revision: observed.state_revision,
        observed_at: minute(3),
        confidence_per_mille: 1_000,
        stock: Vec::new(),
        demands: Vec::new(),
        allocations: Vec::new(),
        fulfillments: Vec::new(),
        transfers: Vec::new(),
        consumptions: Vec::new(),
        losses: vec![ResourceLossObservationV1 {
            loss: loss_id.clone(),
            account: Some(account.clone()),
            transfer: None,
            quantity: 30,
            cause: EvidenceRef::DomainRecordVersion(causes[2].clone()),
        }],
        source_versions: vec![source],
        semantic_digest: String::new(),
    }
    .seal()
    .expect("head");
    let mut forged = head.clone();
    forged.losses[0].quantity = 29;
    let forged = forged.seal().expect("forged head");
    assert!(observed.record_observation_head(forged).is_err());
    observed
        .record_observation_head(head.clone())
        .expect("loss observation");
    let report = materialize_resource_report(&observed, &holder(), &grant, minute(3), minute(3))
        .expect("report");
    assert_eq!(report.losses, head.losses);

    // An explicit override may take stock below the protected floor. Settle it
    // through a tracked command, then prove save/load and exact replay.
    let override_key = key("test:loss:override-floor");
    let revision = state.accounts[&account].revision;
    let lease = certificate(
        &mut state,
        "override-floor",
        &override_key,
        loss_targets(&account, revision, &causes[3]),
        minute(4),
        40,
    );
    let command = ResourceCommandV1 {
        subject: holder(),
        request: loss_request(
            &override_key,
            "test:loss:override-floor",
            &account,
            revision,
            50,
            &causes[3],
            true,
            minute(4),
            lease,
        ),
    };
    let scenario = Scenario::new(minute(4), vec![EntityRef::Person(PersonId::new(1))])
        .with_domain_records(
            causes
                .iter()
                .map(cause_record)
                .chain([state.into_record().expect("resource record")])
                .collect(),
        );
    let resource_plugin = ResourcePlugin::default();
    let plugins: [&dyn SimulationPlugin; 2] = [&resource_plugin, &EvidencePlugin];
    let mut canwu =
        canwu_api::Canwu::new_with_plugins(10, scenario, &plugins).expect("resource runtime");
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
    for boundary in ["command admission", "loss settlement"] {
        canwu
            .settle_boundary(BoundaryRequest::at(minute(4)))
            .expect(boundary);
    }
    let (_, live) = resource_state(&canwu)
        .expect("query")
        .expect("resource state");
    let outcome = &live.outcomes[&override_key];
    assert_eq!(outcome.status, ResourceOperationStatus::Applied);
    assert_eq!(live.accounts[&account].balance, 20);
    assert_eq!(live.conservation.admitted_loss, 80);
    live.validate_conservation().expect("conservation");

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
}
