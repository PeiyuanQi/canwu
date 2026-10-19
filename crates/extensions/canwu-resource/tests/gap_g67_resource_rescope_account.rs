//! Downstream gap G-67: the host application moves an account to another
//! place scope.
//!
//! In a plugin-owned run, the host rescopes a store whose stock was carried
//! to another place, citing the carriage record. The account revision
//! advances, the outcome keeps the evidence, and the store's reservation and
//! demand carry over, so a transfer drawn on them is accepted locally at the
//! new place. A stale revision, an open transfer naming the account as its
//! source or destination, a completion lease on its current revision, and
//! evidence of a record kind the plugin does not read are recorded
//! rejections; a tracked command and an adapter packet cannot carry the
//! operation, a rescope another plugin schedules settles nothing, with a skip
//! event, and evidence that is not current is refused when the host queues
//! it. No boundary fails. A restored run and an exact replay agree.
//!
//! Evidence of the resource plugin's own record is refused like evidence of
//! a kind the plugin does not read, and a detached fixture keeps an account
//! another plugin's prepared participant grant locks in place until the
//! grant is released.
//!
//! A second fixture supersedes the cited evidence between the host queuing
//! the rescope and its settlement, and seals a fork of the run in between:
//! the sealed fork and the unsealed run both record the rescope as a
//! `version_conflict` rejection and keep equal checkpoint hashes, because the
//! check reads the current-version index and never retained evidence.

#![allow(
    clippy::needless_pass_by_value,
    clippy::similar_names,
    clippy::too_many_lines
)]

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundaryRequest,
    BoundarySystemContract, Canwu, CanwuError, CanwuRun, CommandAttemptOutcome, CommandEnvelope,
    CommandRequest, CommandRequestId, DomainRecord, DomainRecordClass, DomainRecordDraft,
    DomainRecordKind, DomainRecordLifecycle, DomainRecordMutation, DomainRecordRef,
    DomainRecordSchema, DomainRecordType, DomainRecordVersionRef, DomainRecordVersionSource,
    DomainValueKindClass, EntityRef, IngressClass, IngressPayload, Issuer, KnowledgeHolderRef,
    PayloadSchema, PersonId, PluginIngressDescriptor, PluginIngressRequest, PluginIngressTarget,
    PluginRegistrar, Scenario, SimDuration, SimTime, SimulationPlugin, SimulationView, StateKey,
    SystemCadence,
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

fn place(name: &str) -> ResourceScopeId {
    ResourceScopeId::new(format!("fixture:place:{name}")).expect("place")
}

fn acquisition(suffix: &str) -> CompletionLeaseAcquisitionId {
    CompletionLeaseAcquisitionId::new(format!("fixture:lease:{suffix}")).expect("acquisition")
}

/// An initial version of a provider record of kind `kind`.
fn record_version(kind: &str, id: &str) -> DomainRecordVersionRef {
    DomainRecordVersionRef {
        record: DomainRecordRef::new(EVIDENCE_NAMESPACE, kind, id),
        version: 1,
        established_by: DomainRecordVersionSource::InitialScenario,
    }
}

fn evidence(id: &str) -> DomainRecordVersionRef {
    record_version("evidence", id)
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

/// The carriage and handover records the resource plugin reads.
struct EvidenceRecord;

impl DomainRecordType for EvidenceRecord {
    type Payload = serde_json::Value;
    type Class = DomainValueKindClass;
    const NAMESPACE: &'static str = EVIDENCE_NAMESPACE;
    const NAME: &'static str = "evidence";
}

/// A record kind the resource plugin is not configured to read.
struct LedgerRecord;

impl DomainRecordType for LedgerRecord {
    type Payload = serde_json::Value;
    type Class = DomainValueKindClass;
    const NAMESPACE: &'static str = EVIDENCE_NAMESPACE;
    const NAME: &'static str = "ledger";
}

/// Moves an evidence record named by an admitted `bump` packet to its next
/// version, and schedules the rescope an admitted `relay` packet carries on
/// the resource plugin's rescope ingress, as a plugin that tried to move an
/// account would.
fn bump(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut directives = Vec::new();
    for id in &context.admitted_ingress {
        let Some(ingress) = view.ingress(*id)? else {
            continue;
        };
        let IngressPayload::Plugin {
            plugin,
            packet_type,
            payload,
            ..
        } = &ingress.payload
        else {
            continue;
        };
        if plugin == PROVIDER && packet_type == "relay" {
            directives.push(BoundaryDirective::SchedulePluginIngress {
                target_plugin: PLUGIN_NAME.to_owned(),
                after: SimDuration::ZERO,
                packet_type: RESOURCE_ACCOUNT_RESCOPE_INGRESS.to_owned(),
                priority: 0,
                payload: payload.clone(),
                affected: Vec::new(),
            });
            continue;
        }
        if plugin != PROVIDER || packet_type != "bump" {
            continue;
        }
        let reference = DomainRecordRef::new(
            EVIDENCE_NAMESPACE,
            "evidence",
            payload["record"].as_str().expect("record"),
        );
        let record = view
            .domain_record(&reference)?
            .expect("evidence record")
            .clone();
        directives.push(BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Update {
                record: DomainRecordDraft::new(
                    reference,
                    serde_json::json!({ "record": record.reference.id, "version": record.version + 1 }),
                ),
                expected_version: record.version,
            },
            summary: "The carriage record moves on".to_owned(),
        });
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
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
        "0000000000000000000000000000000000000000000000000000000000000067"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut evidence = DomainRecordSchema::for_record::<EvidenceRecord>();
        evidence.payload_schema = PayloadSchema::Any;
        let evidence_state = evidence.state_key();
        registrar.register_record_schema(evidence)?;
        let mut ledger = DomainRecordSchema::for_record::<LedgerRecord>();
        ledger.payload_schema = PayloadSchema::Any;
        registrar.register_record_schema(ledger)?;
        for (name, description) in [
            ("bump", "Move one evidence record to its next version"),
            (
                "relay",
                "Schedule one account rescope on the resource plugin",
            ),
        ] {
            registrar.register_ingress(PluginIngressDescriptor {
                name: name.to_owned(),
                description: description.to_owned(),
                class: IngressClass::Information,
                payload_schema: PayloadSchema::Any,
            })?;
        }
        let mut bumping = BoundarySystemContract::new(
            "bump-evidence",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        bumping.reads = vec![StateKey::core_ingress(), evidence_state.clone()];
        bumping.writes = vec![evidence_state];
        bumping.plugin_ingress_targets = vec![PluginIngressTarget {
            target_plugin: PLUGIN_NAME.to_owned(),
            packet_type: RESOURCE_ACCOUNT_RESCOPE_INGRESS.to_owned(),
        }];
        registrar.register_boundary_system(bumping, bump)
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

/// A granary of 50 at the depot with a reserved demand of 10 on it, and an
/// empty market account at the frontier.
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
    for (name, custodian, balance, scope) in [
        ("granary", KEEPER, 50, "depot"),
        ("market", MERCHANT, 0, "frontier"),
    ] {
        state
            .install_opening_account(ResourceAccount {
                id: account(name),
                revision: ResourceRevision::INITIAL,
                custodian: holder(custodian),
                resource_revision: resource.clone(),
                unit_revision: unit.clone(),
                balance,
                capacity: None,
                protected_floor_policy: None,
                closed: false,
                place_scope: Some(place(scope)),
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

fn runtime(seed: u64) -> Canwu {
    let scenario = Scenario::new(
        SimTime::EPOCH,
        vec![
            EntityRef::Person(PersonId::new(KEEPER)),
            EntityRef::Person(PersonId::new(MERCHANT)),
        ],
    )
    .with_domain_records(
        [
            evidence("carriage"),
            evidence("return-carriage"),
            evidence("handover"),
            evidence("sale"),
            record_version("ledger", "entry"),
        ]
        .iter()
        .map(provider_record)
        .chain(world().into_records().expect("resource records"))
        .collect(),
    );
    let resource_plugin =
        ResourcePlugin::new([DomainRecordKind::new(EVIDENCE_NAMESPACE, "evidence")]);
    Canwu::new_with_plugins(seed, scenario, &[&resource_plugin, &ProviderPlugin]).expect("runtime")
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

fn rescope(
    name: &str,
    account_id: &ResourceAccountId,
    expected_account_revision: ResourceRevision,
    scope: Option<&str>,
    cited: &DomainRecordVersionRef,
) -> ResourceRescopeAccountRequestV1 {
    ResourceRescopeAccountRequestV1 {
        operation_key: key(&format!("fixture:rescope:{name}")),
        account: account_id.clone(),
        expected_account_revision,
        place_scope: scope.map(place),
        evidence: cited.clone(),
    }
}

/// The host's rescope, settled in a boundary of its own. Returns its outcome.
fn host_rescope(
    canwu: &mut Canwu,
    at: SimTime,
    request: &ResourceRescopeAccountRequestV1,
) -> ResourceOperationOutcome {
    enqueue_resource_account_rescope(canwu, at, request).expect("rescope ingress");
    settle(canwu, at);
    state(canwu).outcomes[&request.operation_key].clone()
}

/// A rescope that is the recorded rejection `code` and leaves the account
/// as it was.
fn refused(canwu: &mut Canwu, at: SimTime, request: &ResourceRescopeAccountRequestV1, code: &str) {
    let before = state(canwu).accounts[&request.account].clone();
    let outcome = host_rescope(canwu, at, request);
    assert_eq!(outcome.status, ResourceOperationStatus::Rejected, "{code}");
    assert_eq!(outcome.kind, ResourceOperationKind::RescopeAccount);
    assert_eq!(
        outcome.rejection_code.as_deref(),
        Some(code),
        "{:?}",
        outcome.rejection_reason
    );
    assert_eq!(state(canwu).accounts[&request.account], before, "{code}");
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

fn applied_command(
    canwu: &mut Canwu,
    request_id: u64,
    actor: u64,
    at: SimTime,
    request: ResourceOperationRequestV1,
) {
    let operation_key = request.operation_key();
    let outcome = command(canwu, request_id, actor, at, request);
    assert!(
        matches!(outcome, CommandAttemptOutcome::Accepted { .. }),
        "{outcome:?}"
    );
    let outcome = &state(canwu).outcomes[&operation_key];
    assert_eq!(
        outcome.status,
        ResourceOperationStatus::Applied,
        "{:?}",
        outcome.rejection_reason
    );
}

/// Activates one single-step lease through the completion ingress.
fn lease(
    canwu: &mut Canwu,
    lease_holder: u64,
    suffix: &str,
    operation_key: &ResourceOperationKey,
    targets: Vec<CompletionLockedTargetV1>,
    at: SimTime,
) -> CompletionLeaseActivationCertificateV1 {
    let lease = SingleStepCompletionLeaseV1 {
        lease: RequestCompletionLeaseV1 {
            id: acquisition(suffix),
            operation_key: operation_key.clone(),
            holder: holder(lease_holder),
            operation_namespace: NAMESPACE.to_owned(),
            eligibility_time: at,
            eligibility_envelope: EligibilityEnvelopeV1::new(
                targets
                    .iter()
                    .filter_map(|target| match target {
                        CompletionLockedTargetV1::ExternalRecord { version } => {
                            Some(version.clone())
                        }
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

#[test]
fn the_host_moves_a_store_and_its_stock_is_accepted_locally_at_the_new_place() {
    let mut canwu = runtime(67);
    let carriage = evidence("carriage");
    let granary = account("granary");
    let market = account("market");
    let at = minute(1);
    let before = state(&canwu);
    let granary_revision = before.accounts[&granary].revision;

    // Only the host moves an account. A tracked command is refused at
    // admission, and adapter ingress refuses the request when it is queued.
    let request = rescope(
        "by-holder",
        &granary,
        granary_revision,
        Some("frontier"),
        &carriage,
    );
    let attempt = command(
        &mut canwu,
        1,
        KEEPER,
        at,
        ResourceOperationRequestV1::RescopeAccount(request.clone()),
    );
    assert!(
        matches!(attempt, CommandAttemptOutcome::Rejected { .. }),
        "{attempt:?}"
    );
    assert!(
        resource_adapter_ingress(
            at,
            &ResourceAdapterOperationV1 {
                provider_plugin: PROVIDER.to_owned(),
                provider_source: carriage.clone(),
                request: ResourceOperationRequestV1::RescopeAccount(request),
            },
        )
        .is_err()
    );
    // Evidence that is not its record's current version is refused when the
    // host queues it.
    let unknown = DomainRecordVersionRef {
        version: 2,
        ..carriage.clone()
    };
    assert!(
        enqueue_resource_account_rescope(
            &mut canwu,
            at,
            &rescope(
                "unknown",
                &granary,
                granary_revision,
                Some("frontier"),
                &unknown
            ),
        )
        .is_err()
    );
    // A rescope another plugin schedules settles nothing and records no
    // outcome: its boundary commits with a skip event.
    let relayed = rescope(
        "relayed",
        &granary,
        granary_revision,
        Some("frontier"),
        &carriage,
    );
    let events = canwu.events().len();
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            PROVIDER,
            "relay",
            at,
            serde_json::to_value(&relayed).expect("request"),
        ))
        .expect("relay ingress");
    settle(&mut canwu, at);
    settle(&mut canwu, at);
    let skips: Vec<_> = canwu.events()[events..]
        .iter()
        .filter_map(ResourceIngressSkipV1::from_event)
        .collect();
    assert_eq!(skips.len(), 1, "{skips:?}");
    assert_eq!(
        skips[0].reason,
        ResourceIngressSkipReasonV1::FailedSettlementChecks
    );
    assert_eq!(
        skips[0].operation_key.as_ref(),
        Some(&relayed.operation_key)
    );
    let unmoved = state(&canwu);
    assert!(!unmoved.outcomes.contains_key(&relayed.operation_key));
    assert_eq!(unmoved.accounts[&granary], before.accounts[&granary]);
    // A stale revision and evidence of a kind the plugin does not read are
    // recorded rejections.
    refused(
        &mut canwu,
        at,
        &rescope(
            "stale",
            &granary,
            granary_revision.next().expect("revision"),
            Some("frontier"),
            &carriage,
        ),
        "version_conflict",
    );
    refused(
        &mut canwu,
        at,
        &rescope(
            "unread",
            &granary,
            granary_revision,
            Some("frontier"),
            &record_version("ledger", "entry"),
        ),
        "invalid_authority",
    );
    // Nor does a record the lifecycle system reads for itself prove a move:
    // only the declared adapter evidence kinds do.
    let own = canwu
        .current_domain_record_version(&DomainRecordRef::new(
            PLUGIN_NAMESPACE,
            "runtime",
            "resource:runtime:v1",
        ))
        .expect("current-version index")
        .expect("the resource root");
    refused(
        &mut canwu,
        at,
        &rescope(
            "own-record",
            &granary,
            granary_revision,
            Some("frontier"),
            &own,
        ),
        "invalid_authority",
    );

    // The granary is moved to the frontier, where the market is.
    let moved = host_rescope(
        &mut canwu,
        at,
        &rescope(
            "moved",
            &granary,
            granary_revision,
            Some("frontier"),
            &carriage,
        ),
    );
    assert_eq!(moved.status, ResourceOperationStatus::Applied);
    assert_eq!(moved.kind, ResourceOperationKind::RescopeAccount);
    assert_eq!(moved.exact_evidence, vec![carriage.clone()]);
    let after = state(&canwu);
    let moved_account = &after.accounts[&granary];
    assert_eq!(moved_account.place_scope, Some(place("frontier")));
    assert_eq!(
        moved_account.revision,
        granary_revision.next().expect("revision")
    );
    assert_eq!(moved_account.balance, 50);
    // Its reservation and demand carry over.
    assert_eq!(after.reservations, before.reservations);
    assert_eq!(after.allocation_legs, before.allocation_legs);
    assert_eq!(after.demands, before.demands);
    assert_eq!(
        after
            .account_quantities(&granary)
            .expect("quantities")
            .reserved,
        10
    );
    after.validate().expect("root after a rescope");

    // The reserved stock leaves the granary for the market.
    let leg = after
        .allocation_legs
        .values()
        .next()
        .cloned()
        .expect("allocation leg");
    let start_key = key("fixture:start:haul");
    let certificate = lease(
        &mut canwu,
        KEEPER,
        "start",
        &start_key,
        vec![
            CompletionLockedTargetV1::Account {
                id: granary.clone(),
                revision: moved_account.revision,
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
        at,
    );
    let transfer = ResourceTransferId::new("fixture:transfer:haul").expect("transfer");
    applied_command(
        &mut canwu,
        2,
        KEEPER,
        at,
        ResourceOperationRequestV1::BeginTransfer(ResourceTransferStartRequestV1 {
            operation_key: start_key,
            transfer_id: transfer.clone(),
            allocation: (&leg).into(),
            expected_account_revision: moved_account.revision,
            destination: Some(market.clone()),
            at,
            completion_certificate: certificate,
        }),
    );
    // Neither account moves while the transfer is open.
    let open = state(&canwu);
    for (name, id) in [("source", &granary), ("destination", &market)] {
        refused(
            &mut canwu,
            at,
            &rescope(
                &format!("open-{name}"),
                id,
                open.accounts[id].revision,
                Some("depot"),
                &carriage,
            ),
            "invalid_lifecycle",
        );
    }
    // The market accepts it locally: both accounts are at the frontier now.
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
        at,
    );
    applied_command(
        &mut canwu,
        3,
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
    let accepted = state(&canwu);
    assert_eq!(
        accepted.transfers[&transfer].state,
        ResourceTransferState::Accepted
    );
    assert_eq!(accepted.accounts[&market].balance, 10);
    assert_eq!(accepted.accounts[&granary].balance, 40);

    // A lease on the granary's current revision keeps it in place until
    // the lease's operation completes.
    let at = minute(2);
    let sale = evidence("sale");
    let outflow_key = key("fixture:outflow:sale");
    let granary_revision = accepted.accounts[&granary].revision;
    let certificate = lease(
        &mut canwu,
        KEEPER,
        "sale",
        &outflow_key,
        vec![
            CompletionLockedTargetV1::Account {
                id: granary.clone(),
                revision: granary_revision,
            },
            CompletionLockedTargetV1::ExternalRecord {
                version: sale.clone(),
            },
        ],
        at,
    );
    let back = evidence("return-carriage");
    refused(
        &mut canwu,
        at,
        &rescope("leased", &granary, granary_revision, Some("depot"), &back),
        "invalid_lifecycle",
    );
    applied_command(
        &mut canwu,
        4,
        KEEPER,
        at,
        ResourceOperationRequestV1::ExternalOutflow(ResourceExternalOutflowRequestV1 {
            operation_key: outflow_key,
            account: granary.clone(),
            expected_account_revision: granary_revision,
            quantity: 5,
            allow_protected: false,
            authority_evidence: sale,
            at,
            completion_certificate: certificate,
        }),
    );
    let granary_revision = state(&canwu).accounts[&granary].revision;
    let returned = host_rescope(
        &mut canwu,
        at,
        &rescope("returned", &granary, granary_revision, Some("depot"), &back),
    );
    assert_eq!(returned.status, ResourceOperationStatus::Applied);
    let settled = state(&canwu);
    assert_eq!(settled.accounts[&granary].place_scope, Some(place("depot")));
    settled.validate().expect("root after the second move");

    // A restored run and an exact replay hold the same root.
    let resource_plugin =
        ResourcePlugin::new([DomainRecordKind::new(EVIDENCE_NAMESPACE, "evidence")]);
    let plugins: [&dyn SimulationPlugin; 2] = [&resource_plugin, &ProviderPlugin];
    let restored = from_resource_snapshot_json(&canwu.snapshot_json().expect("snapshot"), &plugins)
        .expect("checked restore");
    assert_eq!(state(&restored), settled);
    let replayed =
        replay_resource_from_journal(&plugins, &canwu.replay_journal()).expect("exact replay");
    assert_eq!(state(&replayed), settled);
}

#[test]
fn a_rescope_whose_evidence_was_superseded_is_refused_alike_sealed_and_unsealed() {
    let mut unsealed = runtime(68);
    let carriage = evidence("carriage");
    let granary = account("granary");
    let bump_carriage = |canwu: &mut Canwu, at: SimTime| {
        canwu
            .enqueue_plugin_ingress(PluginIngressRequest::new(
                PROVIDER,
                "bump",
                at,
                serde_json::json!({ "record": carriage.record.id }),
            ))
            .expect("bump ingress");
    };

    // The carriage record moves to a version a boundary establishes, and the
    // host queues the rescope citing it while it is current.
    bump_carriage(&mut unsealed, minute(1));
    settle(&mut unsealed, minute(1));
    let cited = unsealed
        .current_domain_record_version(&carriage.record)
        .expect("version read")
        .expect("carriage record");
    assert_eq!(cited.version, 2);
    let revision = state(&unsealed).accounts[&granary].revision;
    let request = rescope("superseded", &granary, revision, Some("frontier"), &cited);
    enqueue_resource_account_rescope(&mut unsealed, minute(3), &request)
        .expect("the evidence is current when the host queues the rescope");
    // The record moves on before the rescope settles.
    bump_carriage(&mut unsealed, minute(2));

    // A fork of the run seals what it can before each settlement.
    let mut sealed = unsealed.fork().into_compacted().expect("compact handle");
    sealed
        .seal_evidence()
        .expect("seal")
        .expect("the run seals");
    let at = minute(2);
    assert_eq!(
        sealed
            .settle_boundary(BoundaryRequest::at(at))
            .expect("sealed boundary"),
        unsealed
            .settle_boundary(BoundaryRequest::at(at))
            .expect("unsealed boundary")
    );
    sealed
        .seal_evidence()
        .expect("seal")
        .expect("the run seals again");
    let at = minute(3);
    assert_eq!(
        sealed
            .settle_boundary(BoundaryRequest::at(at))
            .expect("sealed boundary"),
        unsealed
            .settle_boundary(BoundaryRequest::at(at))
            .expect("unsealed boundary")
    );
    assert_eq!(sealed.checkpoint_hash(), unsealed.checkpoint_hash());
    for root in [state(&sealed), state(&unsealed)] {
        let outcome = &root.outcomes[&request.operation_key];
        assert_eq!(outcome.status, ResourceOperationStatus::Rejected);
        assert_eq!(outcome.rejection_code.as_deref(), Some("version_conflict"));
        assert_eq!(root.accounts[&granary].place_scope, Some(place("depot")));
    }
}

#[test]
fn an_account_another_plugin_locked_for_its_lease_is_not_rescoped_until_released() {
    let mut root = world();
    let granary = account("granary");
    let revision = root.accounts[&granary].revision;
    let coordinator_source = evidence("carriage");
    let participant = acquisition("coordinated");
    let recipe = CompletionCapacityRecipeV1 {
        receipts: MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE,
        mutations: 1,
        reports_per_holder: 0,
        holders: 0,
        bytes: 1_024,
    };
    // A coordinating plugin's lease holds a prepared participant grant on
    // the granary's current revision, and no lease of the resource plugin
    // does.
    let granted = root
        .apply_operation(&ResourceOperationRequestV1::Completion(
            ResourceCompletionOperationV1::GrantExternalParticipant(
                RequestExternalCompletionParticipantGrantV1 {
                    coordinator_plugin: PROVIDER.to_owned(),
                    coordinator_source: coordinator_source.clone(),
                    coordinator_acquisition_revision: ResourceRevision::INITIAL,
                    acquisition: participant.clone(),
                    operation_key: key("fixture:coordinated"),
                    holder: holder(KEEPER),
                    operation_namespace: NAMESPACE.to_owned(),
                    eligibility_time: SimTime::EPOCH,
                    eligibility_envelope_digest: "e".repeat(64),
                    recipe,
                    policy_class: CompletionPolicyClassV1::Guaranteed,
                    grant_id: CompletionCapacityGrantId::new("fixture:grant:coordinated")
                        .expect("grant"),
                    target_versions: vec![CompletionLockedTargetV1::Account {
                        id: granary.clone(),
                        revision,
                    }],
                    current_boundary: 1,
                },
            ),
        ))
        .expect("external participant grant");
    assert_eq!(
        granted.status,
        ResourceOperationStatus::Applied,
        "{:?}",
        granted.rejection_reason
    );
    // Preparing the grant locks its targets.
    let held = root.external_completion_participants.grants[&participant]
        .grant
        .revision;
    let prepared = root
        .apply_operation(&ResourceOperationRequestV1::Completion(
            ResourceCompletionOperationV1::PrepareExternalParticipant(
                PrepareExternalCompletionParticipantGrantV1 {
                    coordinator_source: coordinator_source.clone(),
                    acquisition: participant.clone(),
                    expected_grant_revision: held,
                    current_boundary: 2,
                    eligibility_envelope_digest: "e".repeat(64),
                },
            ),
        ))
        .expect("external participant prepare");
    assert_eq!(
        prepared.status,
        ResourceOperationStatus::Applied,
        "{:?}",
        prepared.rejection_reason
    );
    assert!(root.completion_leases.target_locks.is_empty());
    assert!(
        root.external_completion_participants
            .target_locks
            .contains_key(&CompletionLockedTargetV1::Account {
                id: granary.clone(),
                revision,
            })
    );
    let request = rescope(
        "coordinated",
        &granary,
        revision,
        Some("frontier"),
        &evidence("carriage"),
    );
    let refused = root
        .apply_operation(&ResourceOperationRequestV1::RescopeAccount(request))
        .expect("rescope");
    assert_eq!(refused.status, ResourceOperationStatus::Rejected);
    assert_eq!(refused.rejection_code.as_deref(), Some("invalid_lifecycle"));
    assert_eq!(root.accounts[&granary].place_scope, Some(place("depot")));

    // Once the coordinator releases its grant, the move settles.
    let grant_revision = root.external_completion_participants.grants[&participant]
        .grant
        .revision;
    let released = root
        .apply_operation(&ResourceOperationRequestV1::Completion(
            ResourceCompletionOperationV1::ReleaseExternalParticipant(
                ReleaseExternalCompletionParticipantGrantV1 {
                    coordinator_source,
                    acquisition: participant,
                    expected_grant_revision: grant_revision,
                    reason: "fixture: the coordinated work was given up".to_owned(),
                },
            ),
        ))
        .expect("release");
    assert_eq!(released.status, ResourceOperationStatus::Applied);
    let moved = root
        .apply_operation(&ResourceOperationRequestV1::RescopeAccount(rescope(
            "released",
            &granary,
            revision,
            Some("frontier"),
            &evidence("carriage"),
        )))
        .expect("rescope");
    assert_eq!(
        moved.status,
        ResourceOperationStatus::Applied,
        "{:?}",
        moved.rejection_reason
    );
    assert_eq!(root.accounts[&granary].place_scope, Some(place("frontier")));
    root.validate().expect("root after the move");
}
