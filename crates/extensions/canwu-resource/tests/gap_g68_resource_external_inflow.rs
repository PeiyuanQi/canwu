//! Downstream gap G-68 and engine finding E-45: value from outside the
//! modelled accounts is credited, and the credit keeps its evidence.
//!
//! In a plugin-owned run, the host application takes a single-step lease,
//! bound to an inflow of forty, for a custodian, which then books that
//! external inflow to its own account by
//! tracked command, the counterpart of an external outflow. The outcome keeps
//! the inflow's evidence and the inflow counts in the conservation totals. A
//! production credit, an inflow to another custodian's account, and a zero
//! inflow the lease does not admit are refused at admission; no boundary
//! fails. A provider's adapter credit, under a lease in which it holds its own
//! participant grant, keeps its evidence too. A restored run and an exact
//! replay agree.
//!
//! A second run shows that a tracked inflow redeems only a lease bound to
//! exactly that credit: under an unbound outflow lease, a lease bound to an
//! outflow, a lease in which the provider holds its own grant, and a lease
//! bound to another quantity it is refused at admission, credits nothing,
//! and leaves the key free for the operation the lease was issued for, which
//! then settles under it. A front-run sent before the provider's grant
//! settles is refused at admission, or skipped when it settles, and takes no
//! key either. A holder's own acquisition of a bound lease is a recorded
//! rejection. A third, detached fixture refuses a root whose lease
//! envelope was rewritten after its certificate was issued.

#![allow(
    clippy::needless_pass_by_value,
    clippy::similar_names,
    clippy::too_many_lines
)]

use canwu_api::{
    BoundaryRequest, Canwu, CanwuError, CommandAttemptOutcome, CommandEnvelope, CommandRequest,
    CommandRequestId, DomainRecord, DomainRecordClass, DomainRecordKind, DomainRecordLifecycle,
    DomainRecordRef, DomainRecordSchema, DomainRecordType, DomainRecordVersionRef,
    DomainRecordVersionSource, DomainValueKindClass, EntityRef, Issuer, KnowledgeHolderRef,
    PayloadSchema, PersonId, PluginRegistrar, Scenario, SimDuration, SimTime, SimulationPlugin,
};
use canwu_resource::*;
use std::collections::{BTreeMap, BTreeSet};

const NAMESPACE: &str = "fixture.resource";
const EVIDENCE_NAMESPACE: &str = "fixture.provider";
const PROVIDER: &str = "fixture-provider";
const TREASURER: u64 = 1;
const OTHER: u64 = 2;

fn holder(value: u64) -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(PersonId::new(value))
}

fn minute(value: i64) -> SimTime {
    SimTime::EPOCH + SimDuration::minutes(value)
}

fn key(value: &str) -> ResourceOperationKey {
    ResourceOperationKey::new(value).expect("operation key")
}

fn treasury() -> ResourceAccountId {
    ResourceAccountId::new("fixture:account:treasury").expect("account")
}

fn acquisition(suffix: &str) -> CompletionLeaseAcquisitionId {
    CompletionLeaseAcquisitionId::new(format!("fixture:lease:{suffix}")).expect("acquisition")
}

fn grant(suffix: &str) -> CompletionCapacityGrantId {
    CompletionCapacityGrantId::new(format!("fixture:grant:{suffix}")).expect("grant")
}

fn evidence(id: &str) -> DomainRecordVersionRef {
    DomainRecordVersionRef {
        record: DomainRecordRef::new(EVIDENCE_NAMESPACE, "evidence", id),
        version: 1,
        established_by: DomainRecordVersionSource::InitialScenario,
    }
}

fn evidence_record(version: &DomainRecordVersionRef) -> DomainRecord {
    DomainRecord {
        reference: version.record.clone(),
        owner: PROVIDER.to_owned(),
        class: DomainRecordClass::Record,
        version: version.version,
        lifecycle: DomainRecordLifecycle::Active,
        payload: serde_json::json!({ "remittance": version.record.id }),
        references: Vec::new(),
    }
}

/// The provider's remittance records, which prove an inflow.
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
        "0000000000000000000000000000000000000000000000000000000000000068"
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

/// A treasury of 10 held by the treasurer.
fn world() -> ResourceState {
    let mut state = ResourceState::empty(ResourceLimitsV1::canonical()).expect("state");
    state
        .install_run_budget(
            RunBudgetRevisionV1 {
                revision: ResourceRevision::INITIAL,
                total_completion_units: 2_000_000,
                shared_pending_slots: 4,
                partitions: vec![partition(holder(TREASURER)), partition(holder(OTHER))],
                semantic_digest: String::new(),
            }
            .seal()
            .expect("budget"),
        )
        .expect("install budget");
    let unit = ResourceUnitRevisionId::new("fixture:unit:coin:v1").expect("unit");
    state
        .install_unit(ResourceUnitRevision {
            id: unit.clone(),
            revision: ResourceRevision::INITIAL,
            symbol: "c".to_owned(),
            scale_numerator: 1,
            scale_denominator: 1,
            semantic_digest: "0".repeat(64),
        })
        .expect("unit");
    let resource = ResourceDefinitionRevisionId::new("fixture:resource:silver:v1").expect("id");
    state
        .install_definition(ResourceDefinitionRevision {
            id: resource.clone(),
            resource: ResourceDefinitionId::new("fixture:resource:silver").expect("definition"),
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
    state
        .install_opening_account(ResourceAccount {
            id: treasury(),
            revision: ResourceRevision::INITIAL,
            custodian: holder(TREASURER),
            resource_revision: resource,
            unit_revision: unit,
            balance: 10,
            capacity: None,
            protected_floor_policy: None,
            closed: false,
            place_scope: None,
        })
        .expect("account");
    state
}

fn state(canwu: &Canwu) -> ResourceState {
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

/// A lease for the treasurer, bound to `binding` when the host binds it.
fn lease_request(
    participants: &[&str],
    suffix: &str,
    operation_key: &ResourceOperationKey,
    remittance: &DomainRecordVersionRef,
    at: SimTime,
    binding: Option<CompletionOperationBindingV1>,
) -> RequestCompletionLeaseV1 {
    let envelope = EligibilityEnvelopeV1::new(
        vec![remittance.clone()],
        BTreeMap::new(),
        BTreeSet::new(),
        Vec::new(),
        Vec::new(),
    )
    .expect("envelope");
    RequestCompletionLeaseV1 {
        id: acquisition(suffix),
        operation_key: operation_key.clone(),
        holder: holder(TREASURER),
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
        expected_participants: participants
            .iter()
            .map(|participant| (*participant).to_owned())
            .collect(),
        policy_class: CompletionPolicyClassV1::Guaranteed,
    }
}

fn inflow(
    name: &str,
    canwu: &Canwu,
    quantity: u64,
    remittance: &DomainRecordVersionRef,
    at: SimTime,
    completion_certificate: CompletionLeaseActivationCertificateV1,
) -> ResourceCreditRequestV1 {
    let account = &state(canwu).accounts[&treasury()];
    ResourceCreditRequestV1 {
        operation_key: key(&format!("fixture:inflow:{name}")),
        account: treasury(),
        expected_account_revision: account.revision,
        resource_revision: account.resource_revision.clone(),
        unit_revision: account.unit_revision.clone(),
        quantity,
        source: ResourceCreditSourceV1::ExternalInflow(remittance.clone()),
        at,
        completion_certificate,
    }
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

fn completion(canwu: &mut Canwu, at: SimTime, operation: ResourceCompletionOperationV1) {
    enqueue_resource_completion_operation(canwu, at, &operation).expect("completion ingress");
    settle(canwu, at);
}

/// The host takes a single-step lease for the treasurer under `suffix`,
/// locking the treasury and `remittance`, bound to `binding` when it binds
/// one, and returns its certificate.
fn treasurer_lease(
    canwu: &mut Canwu,
    suffix: &str,
    operation_key: &ResourceOperationKey,
    remittance: &DomainRecordVersionRef,
    at: SimTime,
    binding: Option<CompletionOperationBindingV1>,
) -> CompletionLeaseActivationCertificateV1 {
    let revision = state(canwu).accounts[&treasury()].revision;
    let current_boundary = next_boundary(canwu);
    completion(
        canwu,
        at,
        ResourceCompletionOperationV1::SingleStepLease(SingleStepCompletionLeaseV1 {
            lease: lease_request(
                &[PLUGIN_NAME],
                suffix,
                operation_key,
                remittance,
                at,
                binding,
            ),
            grant_id: grant(suffix),
            target_versions: vec![
                CompletionLockedTargetV1::Account {
                    id: treasury(),
                    revision,
                },
                CompletionLockedTargetV1::ExternalRecord {
                    version: remittance.clone(),
                },
            ],
            current_boundary,
        }),
    );
    state(canwu).completion_leases.certificates[&acquisition(suffix)].clone()
}

/// The four steps of an unbound lease for the treasurer in which the
/// provider holds its own participant grant on `remittance` and the resource
/// plugin the grant on the treasury, the resource grant's identity being
/// `suffix`. Returns its certificate.
fn provider_lease(
    canwu: &mut Canwu,
    suffix: &str,
    operation_key: &ResourceOperationKey,
    remittance: &DomainRecordVersionRef,
    at: SimTime,
) -> CompletionLeaseActivationCertificateV1 {
    let lease = provider_acquisition(suffix, operation_key, remittance, at);
    completion(canwu, at, ResourceCompletionOperationV1::Acquire(lease));
    finish_provider_lease(canwu, suffix, remittance, at)
}

/// The host's acquisition of an unbound lease for the treasurer in which the
/// provider is a participant.
fn provider_acquisition(
    suffix: &str,
    operation_key: &ResourceOperationKey,
    remittance: &DomainRecordVersionRef,
    at: SimTime,
) -> RequestCompletionLeaseV1 {
    lease_request(
        &[PLUGIN_NAME, PROVIDER],
        suffix,
        operation_key,
        remittance,
        at,
        None,
    )
}

/// The grant, prepare, and activate steps of [`provider_lease`] once its
/// acquisition has settled. Returns its certificate.
fn finish_provider_lease(
    canwu: &mut Canwu,
    suffix: &str,
    remittance: &DomainRecordVersionRef,
    at: SimTime,
) -> CompletionLeaseActivationCertificateV1 {
    let revision = state(canwu).accounts[&treasury()].revision;
    let digest = state(canwu).completion_leases.acquisitions[&acquisition(suffix)]
        .eligibility_envelope
        .digest
        .clone();
    let provider_grant = format!("{suffix}-provider");
    for (grant_suffix, owner, target) in [
        (
            suffix,
            PLUGIN_NAME,
            CompletionLockedTargetV1::Account {
                id: treasury(),
                revision,
            },
        ),
        (
            provider_grant.as_str(),
            PROVIDER,
            CompletionLockedTargetV1::ExternalRecord {
                version: remittance.clone(),
            },
        ),
    ] {
        let current = state(canwu);
        let current_boundary = next_boundary(canwu);
        completion(
            canwu,
            at,
            ResourceCompletionOperationV1::Grant(GrantCompletionCapacityV1 {
                grant_id: grant(grant_suffix),
                acquisition: acquisition(suffix),
                expected_acquisition_revision: current.completion_leases.acquisitions
                    [&acquisition(suffix)]
                    .revision,
                owner_plugin: owner.to_owned(),
                target_versions: vec![target],
                current_boundary,
            }),
        );
    }
    for grant_suffix in [suffix, provider_grant.as_str()] {
        let current = state(canwu);
        let current_boundary = next_boundary(canwu);
        completion(
            canwu,
            at,
            ResourceCompletionOperationV1::Prepare(PrepareCompletionCapacityV1 {
                acquisition: acquisition(suffix),
                expected_acquisition_revision: current.completion_leases.acquisitions
                    [&acquisition(suffix)]
                    .revision,
                grant: grant(grant_suffix),
                expected_grant_revision: current.completion_leases.grants[&grant(grant_suffix)]
                    .revision,
                current_boundary,
                eligibility_envelope_digest: digest.clone(),
            }),
        );
    }
    let current = state(canwu);
    let current_boundary = next_boundary(canwu);
    completion(
        canwu,
        at,
        ResourceCompletionOperationV1::Activate(ActivateCompletionLeaseV1 {
            acquisition: acquisition(suffix),
            expected_acquisition_revision: current.completion_leases.acquisitions
                [&acquisition(suffix)]
                .revision,
            grant: grant(suffix),
            expected_grant_revision: current.completion_leases.grants[&grant(suffix)].revision,
            at,
            current_boundary,
            eligibility_envelope_digest: digest,
        }),
    );
    state(canwu).completion_leases.certificates[&acquisition(suffix)].clone()
}

#[test]
fn a_custodian_books_an_external_inflow_and_its_outcome_keeps_the_evidence() {
    let remittances = [evidence("first"), evidence("second")];
    let resource_plugin =
        ResourcePlugin::new([DomainRecordKind::new(EVIDENCE_NAMESPACE, "evidence")]);
    let plugins: [&dyn SimulationPlugin; 2] = [&resource_plugin, &ProviderPlugin];
    let scenario = Scenario::new(
        SimTime::EPOCH,
        vec![
            EntityRef::Person(PersonId::new(TREASURER)),
            EntityRef::Person(PersonId::new(OTHER)),
        ],
    )
    .with_domain_records(
        remittances
            .iter()
            .map(evidence_record)
            .chain(world().into_records().expect("resource records"))
            .collect(),
    );
    let mut canwu = Canwu::new_with_plugins(68, scenario, &plugins).expect("runtime");

    // The host takes a single-step lease for the treasurer, locking the
    // treasury and the remittance, bound to an inflow credit of forty.
    let at = minute(1);
    let first = &remittances[0];
    let first_key = key("fixture:inflow:first");
    let certificate = treasurer_lease(
        &mut canwu,
        "first",
        &first_key,
        first,
        at,
        Some(CompletionOperationBindingV1::new(
            ResourceOperationKind::Credit,
            40,
        )),
    );

    // A production credit stays a batch credit, and only the custodian books
    // an inflow to its account: both are refused at admission.
    let mut production = inflow("production", &canwu, 5, first, at, certificate.clone());
    production.source = ResourceCreditSourceV1::Production(first.clone());
    for (request_id, actor, request) in [
        (1, TREASURER, production),
        (
            2,
            OTHER,
            inflow("stranger", &canwu, 5, first, at, certificate.clone()),
        ),
    ] {
        let attempt = command(
            &mut canwu,
            request_id,
            actor,
            at,
            ResourceOperationRequestV1::Credit(request),
        );
        assert!(
            matches!(attempt, CommandAttemptOutcome::Rejected { .. }),
            "{attempt:?}"
        );
    }
    // A zero inflow is refused at admission: the lease admits forty only.
    let zero = ResourceOperationRequestV1::Credit(inflow(
        "zero",
        &canwu,
        0,
        first,
        at,
        certificate.clone(),
    ));
    refused_at_admission(&mut canwu, 3, at, zero, "bound to another operation");

    // The treasurer books forty under the lease.
    let request =
        ResourceOperationRequestV1::Credit(inflow("first", &canwu, 40, first, at, certificate));
    command(&mut canwu, 4, TREASURER, at, request.clone());
    let booked = state(&canwu);
    let outcome = &booked.outcomes[&first_key];
    assert_eq!(
        outcome.status,
        ResourceOperationStatus::Applied,
        "{:?}",
        outcome.rejection_reason
    );
    assert_eq!(outcome.kind, ResourceOperationKind::Credit);
    assert_eq!(outcome.quantity, 40);
    assert_eq!(outcome.exact_evidence, vec![first.clone()]);
    assert_eq!(booked.accounts[&treasury()].balance, 50);
    assert_eq!(booked.conservation.external_inflow, 40);
    assert!(
        !booked
            .completion_leases
            .acquisitions
            .contains_key(&acquisition("first"))
    );
    booked.validate_conservation().expect("conserved");
    // A resend settles nothing more.
    command(&mut canwu, 5, TREASURER, at, request);
    assert_eq!(state(&canwu).accounts[&treasury()].balance, 50);

    // A provider credits an inflow through its adapter ingress, under a
    // lease in which it holds its own participant grant: the outcome keeps
    // the evidence on that path too.
    let at = minute(3);
    let second = &remittances[1];
    let second_key = key("fixture:inflow:second");
    let certificate = provider_lease(&mut canwu, "second", &second_key, second, at);
    let packet = ResourceAdapterOperationV1 {
        provider_plugin: PROVIDER.to_owned(),
        provider_source: second.clone(),
        request: ResourceOperationRequestV1::Credit(inflow(
            "second",
            &canwu,
            25,
            second,
            at,
            certificate,
        )),
    };
    enqueue_resource_adapter_operation(&mut canwu, at, &packet).expect("adapter ingress");
    settle(&mut canwu, at);
    let credited = state(&canwu);
    let outcome = &credited.outcomes[&second_key];
    assert_eq!(
        outcome.status,
        ResourceOperationStatus::Applied,
        "{:?}",
        outcome.rejection_reason
    );
    assert_eq!(outcome.exact_evidence, vec![second.clone()]);
    assert_eq!(credited.accounts[&treasury()].balance, 75);
    assert_eq!(credited.conservation.external_inflow, 65);
    credited.validate().expect("root after both inflows");
    validate_resource_runtime(&canwu).expect("the kept evidence exists");

    // A restored run and an exact replay hold the same root.
    let restored = from_resource_snapshot_json(&canwu.snapshot_json().expect("snapshot"), &plugins)
        .expect("checked restore");
    assert_eq!(state(&restored), credited);
    let replayed =
        replay_resource_from_journal(&plugins, &canwu.replay_journal()).expect("exact replay");
    assert_eq!(state(&replayed), credited);
}

/// The recorded outcome of a tracked command the treasurer sends.
fn treasurer_command(
    canwu: &mut Canwu,
    request_id: u64,
    at: SimTime,
    request: ResourceOperationRequestV1,
) -> ResourceOperationOutcome {
    let operation_key = request.operation_key();
    let attempt = command(canwu, request_id, TREASURER, at, request);
    assert!(
        matches!(attempt, CommandAttemptOutcome::Accepted { .. }),
        "{attempt:?}"
    );
    state(canwu).outcomes[&operation_key].clone()
}

/// A tracked command of the treasurer refused at admission for `reason`: it
/// settles nothing and leaves its operation key free.
fn refused_at_admission(
    canwu: &mut Canwu,
    request_id: u64,
    at: SimTime,
    request: ResourceOperationRequestV1,
    reason: &str,
) {
    let operation_key = request.operation_key();
    let before = state(canwu);
    let attempt = command(canwu, request_id, TREASURER, at, request);
    let CommandAttemptOutcome::Rejected { error } = attempt else {
        panic!("{reason}: admitted: {attempt:?}");
    };
    assert!(error.message.contains(reason), "{error:?}");
    let after = state(canwu);
    assert!(!after.outcomes.contains_key(&operation_key), "{reason}");
    assert_eq!(after.accounts, before.accounts, "{reason}");
}

fn applied(outcome: &ResourceOperationOutcome) {
    assert_eq!(
        outcome.status,
        ResourceOperationStatus::Applied,
        "{:?}",
        outcome.rejection_reason
    );
}

/// The treasurer's tracked outflow of `quantity` under `certificate`.
fn outflow(
    canwu: &Canwu,
    operation_key: &ResourceOperationKey,
    quantity: u64,
    authority: &DomainRecordVersionRef,
    at: SimTime,
    completion_certificate: CompletionLeaseActivationCertificateV1,
) -> ResourceOperationRequestV1 {
    ResourceOperationRequestV1::ExternalOutflow(ResourceExternalOutflowRequestV1 {
        operation_key: operation_key.clone(),
        account: treasury(),
        expected_account_revision: state(canwu).accounts[&treasury()].revision,
        quantity,
        allow_protected: false,
        authority_evidence: authority.clone(),
        at,
        completion_certificate,
    })
}

#[test]
fn a_tracked_inflow_redeems_only_a_lease_bound_to_exactly_that_credit() {
    let remittances = ["mint", "kind", "provider", "over", "self", "timed", "raced"].map(evidence);
    let resource_plugin =
        ResourcePlugin::new([DomainRecordKind::new(EVIDENCE_NAMESPACE, "evidence")]);
    let plugins: [&dyn SimulationPlugin; 2] = [&resource_plugin, &ProviderPlugin];
    let scenario = Scenario::new(
        SimTime::EPOCH,
        vec![
            EntityRef::Person(PersonId::new(TREASURER)),
            EntityRef::Person(PersonId::new(OTHER)),
        ],
    )
    .with_domain_records(
        remittances
            .iter()
            .map(evidence_record)
            .chain(world().into_records().expect("resource records"))
            .collect(),
    );
    let mut canwu = Canwu::new_with_plugins(168, scenario, &plugins).expect("runtime");
    let outflow_of = |quantity| {
        CompletionOperationBindingV1::new(ResourceOperationKind::ExternalOutflow, quantity)
    };

    // The host leases an outflow of five without binding it. The treasurer's
    // inflow of a million under its key is refused at admission, since an
    // inflow needs a lease bound to exactly that credit, and the key stays
    // free: the outflow the lease was issued for settles under it.
    let at = minute(1);
    let mint = &remittances[0];
    let mint_key = key("fixture:inflow:mint");
    let certificate = treasurer_lease(&mut canwu, "mint", &mint_key, mint, at, None);
    let request = inflow("mint", &canwu, 1_000_000, mint, at, certificate.clone());
    refused_at_admission(
        &mut canwu,
        1,
        at,
        ResourceOperationRequestV1::Credit(request),
        "bound to exactly that credit",
    );
    let request = outflow(&canwu, &mint_key, 5, mint, at, certificate);
    applied(&treasurer_command(&mut canwu, 2, at, request));

    // Bound to an outflow of five, a lease is not redeemed for an inflow;
    // the outflow settles under its key.
    let at = minute(2);
    let kind = &remittances[1];
    let kind_key = key("fixture:inflow:kind");
    let certificate = treasurer_lease(&mut canwu, "kind", &kind_key, kind, at, Some(outflow_of(5)));
    let request = inflow("kind", &canwu, 1_000_000, kind, at, certificate.clone());
    refused_at_admission(
        &mut canwu,
        3,
        at,
        ResourceOperationRequestV1::Credit(request),
        "bound to another operation",
    );
    let request = outflow(&canwu, &kind_key, 5, kind, at, certificate);
    applied(&treasurer_command(&mut canwu, 4, at, request));
    assert_eq!(state(&canwu).accounts[&treasury()].balance, 0);

    // A lease in which the provider holds its own participant grant, meant
    // for the provider's adapter credit, is not redeemed by a tracked
    // command at all, and the provider's credit of 25 then settles under its
    // key.
    let at = minute(3);
    let provider = &remittances[2];
    let provider_key = key("fixture:inflow:provider");
    let certificate = provider_lease(&mut canwu, "provider", &provider_key, provider, at);
    let request = inflow(
        "provider",
        &canwu,
        2_000_000,
        provider,
        at,
        certificate.clone(),
    );
    refused_at_admission(
        &mut canwu,
        5,
        at,
        ResourceOperationRequestV1::Credit(request),
        "only participant is the resource plugin",
    );
    let packet = ResourceAdapterOperationV1 {
        provider_plugin: PROVIDER.to_owned(),
        provider_source: provider.clone(),
        request: ResourceOperationRequestV1::Credit(inflow(
            "provider",
            &canwu,
            25,
            provider,
            at,
            certificate,
        )),
    };
    enqueue_resource_adapter_operation(&mut canwu, at, &packet).expect("adapter ingress");
    settle(&mut canwu, at);
    applied(&state(&canwu).outcomes[&provider_key]);

    // A lease bound to an inflow of forty is not redeemed for forty-one;
    // forty is booked under its key.
    let at = minute(4);
    let over = &remittances[3];
    let over_key = key("fixture:inflow:over");
    let certificate = treasurer_lease(
        &mut canwu,
        "over",
        &over_key,
        over,
        at,
        Some(CompletionOperationBindingV1::new(
            ResourceOperationKind::Credit,
            40,
        )),
    );
    let request = inflow("over", &canwu, 41, over, at, certificate.clone());
    refused_at_admission(
        &mut canwu,
        6,
        at,
        ResourceOperationRequestV1::Credit(request),
        "bound to another operation",
    );
    let request = inflow("over", &canwu, 40, over, at, certificate);
    let outcome = treasurer_command(
        &mut canwu,
        7,
        at,
        ResourceOperationRequestV1::Credit(request),
    );
    applied(&outcome);
    assert_eq!(outcome.exact_evidence, vec![over.clone()]);

    // A holder does not write a binding: the treasurer's own acquisition of
    // a lease bound to an inflow of three million is a recorded rejection,
    // and no lease is acquired for the host to grant.
    let at = minute(5);
    let own = &remittances[4];
    let own_key = key("fixture:inflow:self");
    let acquire = ResourceCompletionOperationV1::Acquire(lease_request(
        &[PLUGIN_NAME],
        "self",
        &own_key,
        own,
        at,
        Some(CompletionOperationBindingV1::new(
            ResourceOperationKind::Credit,
            3_000_000,
        )),
    ));
    let outcome = treasurer_command(
        &mut canwu,
        8,
        at,
        ResourceOperationRequestV1::Completion(acquire),
    );
    assert_eq!(outcome.status, ResourceOperationStatus::Rejected);
    assert_eq!(outcome.rejection_code.as_deref(), Some("invalid_authority"));
    assert!(
        outcome
            .rejection_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("issuer binds it")),
        "{:?}",
        outcome.rejection_reason
    );
    assert!(
        !state(&canwu)
            .completion_leases
            .acquisitions
            .contains_key(&acquisition("self"))
    );

    // A front-run timed before the provider's grant settles. Sent once the
    // host's acquisition, which names the provider as a participant, has
    // settled, it is refused at admission; sent with that acquisition, it is
    // admitted and then skipped when it settles. Either way it takes no key,
    // and the provider's credit of 25 settles under it.
    for (index, (suffix, admitted)) in [("timed", false), ("raced", true)].into_iter().enumerate() {
        let at = minute(6 + i64::try_from(index).expect("minute"));
        let remittance = &remittances[5 + index];
        let timed_key = key(&format!("fixture:inflow:{suffix}"));
        let lease = provider_acquisition(suffix, &timed_key, remittance, at);
        let forged = CompletionLeaseActivationCertificateV1 {
            acquisition: acquisition(suffix),
            acquisition_revision: ResourceRevision::INITIAL,
            operation_key: timed_key.clone(),
            prepared_grants: Vec::new(),
            locked_target_versions: Vec::new(),
            recipe_digest: String::new(),
            eligibility_time: at,
            eligibility_envelope_digest: lease.eligibility_envelope.digest.clone(),
            activation_boundary: 0,
            semantic_digest: String::new(),
        };
        let front_run = ResourceOperationRequestV1::Credit(inflow(
            suffix, &canwu, 2_000_000, remittance, at, forged,
        ));
        if admitted {
            enqueue_resource_completion_operation(
                &mut canwu,
                at,
                &ResourceCompletionOperationV1::Acquire(lease),
            )
            .expect("completion ingress");
            let events = canwu.events().len();
            let attempt = command(
                &mut canwu,
                10 + u64::try_from(index).expect("id"),
                TREASURER,
                at,
                front_run,
            );
            assert!(
                matches!(attempt, CommandAttemptOutcome::Accepted { .. }),
                "{attempt:?}"
            );
            let skips: Vec<_> = canwu.events()[events..]
                .iter()
                .filter_map(ResourceIngressSkipV1::from_event)
                .collect();
            assert_eq!(skips.len(), 1, "{skips:?}");
            assert_eq!(
                skips[0].reason,
                ResourceIngressSkipReasonV1::FailedSettlementChecks
            );
            assert_eq!(skips[0].operation_key.as_ref(), Some(&timed_key));
            assert!(!state(&canwu).outcomes.contains_key(&timed_key));
        } else {
            completion(
                &mut canwu,
                at,
                ResourceCompletionOperationV1::Acquire(lease),
            );
            refused_at_admission(
                &mut canwu,
                10 + u64::try_from(index).expect("id"),
                at,
                front_run,
                "only participant is the resource plugin",
            );
        }
        let certificate = finish_provider_lease(&mut canwu, suffix, remittance, at);
        let packet = ResourceAdapterOperationV1 {
            provider_plugin: PROVIDER.to_owned(),
            provider_source: remittance.clone(),
            request: ResourceOperationRequestV1::Credit(inflow(
                suffix,
                &canwu,
                25,
                remittance,
                at,
                certificate,
            )),
        };
        enqueue_resource_adapter_operation(&mut canwu, at, &packet).expect("adapter ingress");
        settle(&mut canwu, at);
        applied(&state(&canwu).outcomes[&timed_key]);
    }

    let settled = state(&canwu);
    assert_eq!(settled.accounts[&treasury()].balance, 115);
    assert_eq!(settled.conservation.external_inflow, 115);
    assert_eq!(settled.conservation.external_outflow, 10);
    settled.validate_conservation().expect("conserved");

    // A restored run and an exact replay hold the same root.
    let restored = from_resource_snapshot_json(&canwu.snapshot_json().expect("snapshot"), &plugins)
        .expect("checked restore");
    assert_eq!(state(&restored), settled);
    let replayed =
        replay_resource_from_journal(&plugins, &canwu.replay_journal()).expect("exact replay");
    assert_eq!(state(&replayed), settled);
}

#[test]
fn a_root_whose_certificate_carries_another_envelope_than_its_lease_is_refused() {
    // The host's single step for an inflow of forty, in a detached root.
    let mut root = world();
    let remittance = evidence("first");
    let operation_key = key("fixture:inflow:first");
    let revision = root.accounts[&treasury()].revision;
    let outcome = root
        .apply_operation(&ResourceOperationRequestV1::Completion(
            ResourceCompletionOperationV1::SingleStepLease(SingleStepCompletionLeaseV1 {
                lease: lease_request(
                    &[PLUGIN_NAME],
                    "first",
                    &operation_key,
                    &remittance,
                    minute(1),
                    Some(CompletionOperationBindingV1::new(
                        ResourceOperationKind::Credit,
                        40,
                    )),
                ),
                grant_id: grant("first"),
                target_versions: vec![
                    CompletionLockedTargetV1::Account {
                        id: treasury(),
                        revision,
                    },
                    CompletionLockedTargetV1::ExternalRecord {
                        version: remittance.clone(),
                    },
                ],
                current_boundary: 1,
            }),
        ))
        .expect("single step");
    applied(&outcome);
    let records = root.into_records().expect("resource records");
    let live = records
        .iter()
        .position(|record| record.reference.id == "resource:runtime:v1")
        .expect("live root record");
    let restore = |records: &[DomainRecord]| {
        ResourceState::from_records(
            &records[live],
            records
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != live)
                .map(|(_, record)| record),
        )
    };
    restore(&records).expect("the untouched root restores");

    // The acquisition's envelope is rewritten to a valid one bound to a
    // million: the certificate still carries the digest of the envelope it
    // was activated on, so the root is refused.
    let rebound = EligibilityEnvelopeV1::new(
        vec![remittance],
        BTreeMap::new(),
        BTreeSet::new(),
        Vec::new(),
        Vec::new(),
    )
    .expect("envelope")
    .bound_to(CompletionOperationBindingV1::new(
        ResourceOperationKind::Credit,
        1_000_000,
    ))
    .expect("bound envelope");
    let mut forged = records.clone();
    let envelope = forged[live]
        .payload
        .pointer_mut(&format!(
            "/completion_leases/acquisitions/{}/eligibility_envelope",
            acquisition("first")
                .as_str()
                .replace('~', "~0")
                .replace('/', "~1")
        ))
        .expect("the acquisition's envelope");
    *envelope = serde_json::to_value(&rebound).expect("envelope JSON");
    let error = restore(&forged).expect_err("a rebound lease");
    assert!(
        error.message.contains("completion activation certificate"),
        "{error:?}"
    );
}
