//! Downstream gap G-17: several consumptions settle under one completion
//! lease, with one outcome per item.
//!
//! A settlement office requests one demand per household account. A provider
//! publishes a typed consumption intent for every item of its settlement
//! batch, and one completion lease locks every item's account, allocation
//! leg, and demand. One household's demand is cancelled after the lease was
//! taken: the batch refuses that item alone, applies the others, totals them
//! in the batch outcome, and completes the lease once, which then leaves the
//! root. A resend is answered by the batch outcome, the item keys are
//! reserved, and a restored run and an exact replay agree.
//!
//! The second fixture bounds the batch items one boundary settles: a batch
//! larger than the bound is a recorded rejection that leaves its lease
//! active and takes nothing of the budget, and a batch that fits alone but
//! not beside another is deferred with `boundary_budget` to the next boundary
//! at the same instant, which settles it when the host advances time.
//!
//! The other fixtures settle a full batch under the canonical limits, skip a
//! batch whose records do not fit the archive budget without recording
//! anything, and consume several legs of one account in one batch.

#![allow(clippy::too_many_lines)]

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundaryRequest,
    BoundarySystemContract, Canwu, CanwuError, CommandAttemptOutcome, CommandEnvelope,
    CommandRequest, CommandRequestId, DomainRecord, DomainRecordClass, DomainRecordDraft,
    DomainRecordKind, DomainRecordLifecycle, DomainRecordMutation, DomainRecordRef,
    DomainRecordSchema, DomainRecordType, DomainRecordVersionRef, DomainValueKindClass, EntityRef,
    Issuer, KnowledgeHolderRef, PayloadSchema, PersonId, PluginRegistrar, Scenario, SimDuration,
    SimTime, SimulationPlugin, SimulationView, StateVisibility, SystemCadence,
};
use canwu_resource::*;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

const PROVIDER: &str = "fixture-settlement";
const NAMESPACE: &str = "fixture.settlement.lease";
/// A settlement office: custodian of its household stores, requester of
/// their demands, and holder of their lease. One holder takes at most one
/// lease per simulated minute, so batches that settle in one boundary belong
/// to different offices.
fn office(id: u64) -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(PersonId::new(id))
}

fn key(value: &str) -> ResourceOperationKey {
    ResourceOperationKey::new(value).expect("operation key")
}

fn account(household: usize) -> ResourceAccountId {
    ResourceAccountId::new(format!("fixture:account:household:{household}")).expect("account")
}

fn demand(household: usize) -> ResourceDemandId {
    ResourceDemandId::new(format!("fixture:demand:household:{household}")).expect("demand")
}

fn batch_key(label: &str) -> ResourceOperationKey {
    key(&format!("fixture:batch:{label}"))
}

/// The provider's settlement record of one batch.
struct SettlementRecord;

impl DomainRecordType for SettlementRecord {
    type Payload = Value;
    type Class = DomainValueKindClass;
    const NAMESPACE: &'static str = "fixture.settlement";
    const NAME: &'static str = "plan";
}

fn settlement(label: &str) -> DomainRecordRef {
    DomainRecordRef::new(
        "fixture.settlement",
        "plan",
        format!("fixture:plan:{label}"),
    )
}

/// Publishes, once, a typed consumption intent for every item of each
/// settlement record's batch, under the item's reserved key. Once the batch
/// has settled, it records the fulfillment of each consumption as a reader
/// that names only the batch outcome and the consumptions finds it.
struct ProviderPlugin;

impl SimulationPlugin for ProviderPlugin {
    fn name(&self) -> &'static str {
        PROVIDER
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "00000000000000000000000000000000000000000000000000000000000000c7"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut schema = DomainRecordSchema::for_record::<SettlementRecord>();
        schema.payload_schema = PayloadSchema::Any;
        let own = schema.state_key();
        registrar.register_record_schema(schema)?;
        let mut contract = BoundarySystemContract::new(
            "publish-settlement-intents",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        contract.reads = vec![
            own.clone(),
            DomainRecordSchema::for_record::<ResourceRuntimeRecord>().state_key(),
            resource_hot_terminal_state_key(),
        ];
        contract.writes = vec![own];
        contract.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(contract, publish_intents)
    }
}

fn publish_intents(
    view: &SimulationView<'_>,
    _: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut directives = Vec::new();
    for record in
        view.domain_records_of_kind(&DomainRecordKind::for_type::<SettlementRecord>(), 16)?
    {
        let batch = key(record.payload["batch"].as_str().expect("batch"));
        let households: Vec<usize> = record.payload["households"]
            .as_array()
            .expect("households")
            .iter()
            .map(|value| usize::try_from(value.as_u64().expect("household")).expect("index"))
            .collect();
        if record.payload["published"] == true {
            if record.payload.get("fulfillments").is_none()
                && let Some(audit) = audit_fulfillments(view, &batch, &households)?
            {
                let mut payload = record.payload.clone();
                payload["fulfillments"] = audit;
                directives.push(BoundaryDirective::MutateRecord {
                    mutation: DomainRecordMutation::Update {
                        record: DomainRecordDraft::new(record.reference.clone(), payload),
                        expected_version: record.version,
                    },
                    summary: "Record the settled batch's fulfillments".to_owned(),
                });
            }
            continue;
        }
        let mut reader = ResourceRootReader::load(view)?.expect("resource root");
        reader.attach_live(
            households
                .iter()
                .map(|household| ResourceLiveRecordKeyV1::Account(account(*household))),
        )?;
        let resource = reader.into_state();
        let mut intents = BTreeMap::new();
        for (index, household) in households.iter().enumerate() {
            let leg = resource
                .allocation_legs
                .values()
                .find(|leg| leg.demand == demand(*household))
                .expect("allocated leg");
            let intent = ResourceConsumptionIntentV1 {
                id: ResourceConsumptionIntentId::new(format!("fixture:intent:{household}"))
                    .expect("intent"),
                provider_plugin: PROVIDER.to_owned(),
                demand: leg.demand.clone(),
                demand_revision: leg.demand_revision,
                allocation: leg.into(),
                account: leg.account.clone(),
                expected_account_revision: resource.accounts[&leg.account].revision,
                consumption_id: ResourceConsumptionId::new(format!(
                    "fixture:consumption:{household}"
                ))
                .expect("consumption"),
                operation_key: resource_consumption_batch_item_key(&batch, index),
                quantity: leg.quantity,
                status: ResourceConsumptionIntentStatusV1::Authorized,
                semantic_digest: String::new(),
            }
            .seal()
            .expect("intent");
            intents.insert(intent.id.to_string(), intent);
        }
        let mut payload = record.payload.clone();
        payload["published"] = json!(true);
        payload["resource_consumption_intents"] = serde_json::to_value(intents).expect("intents");
        directives.push(BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Update {
                record: DomainRecordDraft::new(record.reference.clone(), payload),
                expected_version: record.version,
            },
            summary: "Publish the settlement batch's consumption intents".to_owned(),
        });
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

/// The demand of each settled consumption's fulfillment, as a reader that
/// names the batch outcome and the consumptions, and no fulfillment, finds
/// it: the consumption names its fulfillment, and the reader attaches what
/// an attached record names. `None` until the batch has settled.
fn audit_fulfillments(
    view: &SimulationView<'_>,
    batch: &ResourceOperationKey,
    households: &[usize],
) -> Result<Option<Value>, CanwuError> {
    let consumptions: Vec<_> = households
        .iter()
        .map(|household| {
            ResourceConsumptionId::new(format!("fixture:consumption:{household}"))
                .expect("consumption")
        })
        .collect();
    let (_, resource) = resource_root_in_view(
        view,
        std::iter::once(ResourceTerminalRecordKeyV1::Outcome(batch.clone())).chain(
            consumptions
                .iter()
                .cloned()
                .map(ResourceTerminalRecordKeyV1::Consumption),
        ),
    )?
    .expect("resource root");
    if !resource.outcomes.contains_key(batch) {
        return Ok(None);
    }
    let found: BTreeMap<String, Value> = consumptions
        .iter()
        .filter_map(|id| resource.consumptions.get(id))
        .map(|consumption| {
            (
                consumption.id.to_string(),
                resource
                    .fulfillments
                    .get(&consumption.fulfillment)
                    .map_or(Value::Null, |fulfillment| json!(fulfillment.demand)),
            )
        })
        .collect();
    Ok(Some(json!(found)))
}

/// A root with one account and one demand per household, each of the office
/// `offices[household]`, and a lease partition for every office. With
/// `shared_account`, every demand draws on the first household's account.
fn world(offices: &[u64], limits: ResourceLimitsV1, shared_account: bool) -> ResourceState {
    let mut state = ResourceState::empty(limits).expect("state");
    state
        .install_run_budget(
            RunBudgetRevisionV1 {
                revision: ResourceRevision::INITIAL,
                total_completion_units: 2_000_000,
                shared_pending_slots: 4,
                partitions: offices
                    .iter()
                    .copied()
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .map(|id| CompletionCapacityPartitionV1 {
                        authority: office(id),
                        operation_namespace: NAMESPACE.to_owned(),
                        guaranteed_units: 400_000,
                        reserved_pending_slots: 8,
                        maximum_burst_units: 100_000,
                        request_token_capacity: 16,
                        request_token_refill_minutes: 1,
                        reacquire_cooldown_minutes: 1,
                        root_acquisition_cap_per_sim_time: 8,
                        guaranteed_max_wait_boundaries: 4,
                    })
                    .collect(),
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
            scope: ResourceScopeId::new("fixture:scope:county").expect("scope"),
            effective_from: SimTime::EPOCH,
            effective_until: None,
            process_suitability: BTreeSet::new(),
            semantic_digest: "1".repeat(64),
        })
        .expect("definition");
    for (household, id) in offices.iter().copied().enumerate() {
        state
            .install_opening_account(ResourceAccount {
                id: account(household),
                revision: ResourceRevision::INITIAL,
                custodian: office(id),
                resource_revision: resource.clone(),
                unit_revision: unit.clone(),
                balance: 100,
                capacity: None,
                protected_floor_policy: None,
                closed: false,
                place_scope: None,
            })
            .expect("account");
        state
            .install_demand(ResourceDemand {
                id: demand(household),
                revision: ResourceRevision::INITIAL,
                requester: office(id),
                source_policy: ResourceDemandSourcePolicyV1::ExactAccounts(vec![account(
                    if shared_account { 0 } else { household },
                )]),
                resource_revision: resource.clone(),
                unit_revision: unit.clone(),
                requested: 10,
                fulfilled: 0,
                minimum_useful: 10,
                partial_fulfillment: PartialFulfillmentPolicy::RejectPartial,
                alternative_group: None,
                due_at: SimTime::EPOCH,
                expires_at: SimTime::EPOCH + SimDuration::days(30),
                priority: 0,
                tie_break: ResourceTieBreakKey::new(format!("fixture:tie:{household}"))
                    .expect("tie"),
                admitted_sequence: 0,
                protected_floor_policy: None,
                protection_override_class: None,
                status: DemandStatus::Open,
                rejection_reason: None,
            })
            .expect("demand");
    }
    state
}

fn plugins() -> (ResourcePlugin, ProviderPlugin) {
    (
        ResourcePlugin::new([DomainRecordKind::for_type::<SettlementRecord>()]),
        ProviderPlugin,
    )
}

struct Run {
    canwu: Canwu,
    commands: u64,
}

fn state(canwu: &Canwu) -> ResourceState {
    resource_state(canwu)
        .expect("resource query")
        .expect("resource state")
        .1
}

fn skips(canwu: &Canwu, from: usize) -> Vec<ResourceIngressSkipV1> {
    canwu.events()[from..]
        .iter()
        .filter_map(ResourceIngressSkipV1::from_event)
        .collect()
}

impl Run {
    /// A run with `households` household accounts and allocated demands,
    /// and one settlement record per batch, whose intents the provider has
    /// published.
    fn new(offices: &[u64], batch_budget: usize, batches: &[(&str, &[usize])]) -> Self {
        let mut limits = ResourceLimitsV1::canonical();
        limits.max_batch_items_per_boundary = batch_budget;
        Self::build(offices, limits, false, batches)
    }

    /// [`Self::new`] with the root's limits and account sharing given.
    fn build(
        offices: &[u64],
        limits: ResourceLimitsV1,
        shared_account: bool,
        batches: &[(&str, &[usize])],
    ) -> Self {
        let (resource, provider) = plugins();
        let plugins: [&dyn SimulationPlugin; 2] = [&resource, &provider];
        let records = world(offices, limits, shared_account)
            .into_records()
            .expect("resource root")
            .into_iter()
            .chain(batches.iter().map(|(label, members)| DomainRecord {
                reference: settlement(label),
                owner: PROVIDER.to_owned(),
                class: DomainRecordClass::Record,
                version: 1,
                lifecycle: DomainRecordLifecycle::Active,
                payload: json!({
                    "published": false,
                    "batch": batch_key(label),
                    "households": members,
                }),
                references: Vec::new(),
            }))
            .collect();
        let ids: BTreeSet<u64> = offices.iter().copied().collect();
        let scenario = Scenario::new(
            SimTime::EPOCH,
            ids.iter()
                .map(|id| EntityRef::Person(PersonId::new(*id)))
                .collect(),
        )
        .with_domain_records(records);
        let canwu = Canwu::new_with_plugins(17, scenario, &plugins).expect("runtime");
        let mut run = Self { canwu, commands: 0 };
        for id in ids {
            let root = state(&run.canwu);
            enqueue_resource_allocation(
                &mut run.canwu,
                SimTime::EPOCH,
                &office(id),
                &ResourceAllocationRequestV1 {
                    operation_key: key(&format!("fixture:allocate:{id}")),
                    expected_state_revision: root.state_revision,
                    at: SimTime::EPOCH,
                    candidate_limit: offices.len(),
                },
            )
            .expect("allocation");
            run.settle();
        }
        run.canwu
            .settle_boundary(BoundaryRequest {
                at: SimTime::EPOCH,
                cadences: vec![SystemCadence::Daily],
            })
            .expect("the provider publishes its intents");
        run
    }

    fn settle(&mut self) {
        self.canwu
            .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
            .expect("a resource boundary never fails in this fixture");
    }

    fn next_boundary(&self) -> u64 {
        self.canwu
            .boundaries()
            .last()
            .map_or(1, |boundary| boundary.id.get().saturating_add(1))
    }

    fn source(&self, label: &str) -> DomainRecordVersionRef {
        self.canwu
            .current_domain_record_version(&settlement(label))
            .expect("source query")
            .expect("published settlement")
    }

    fn completion(&mut self, operation: &ResourceCompletionOperationV1) {
        enqueue_resource_completion_operation(&mut self.canwu, SimTime::EPOCH, operation)
            .expect("completion ingress");
        self.settle();
    }

    /// The batch of `households`, without its certificate yet.
    fn batch(&self, label: &str, households: &[usize]) -> ResourceConsumptionBatchRequestV1 {
        let root = state(&self.canwu);
        let items = households
            .iter()
            .map(|household| {
                let leg = root
                    .allocation_legs
                    .values()
                    .find(|leg| leg.demand == demand(*household))
                    .expect("allocated leg");
                ResourceConsumptionBatchItemV1 {
                    consumption_id: ResourceConsumptionId::new(format!(
                        "fixture:consumption:{household}"
                    ))
                    .expect("consumption"),
                    allocation: leg.into(),
                    demand: leg.demand.clone(),
                    demand_revision: leg.demand_revision,
                    expected_account_revision: root.accounts[&leg.account].revision,
                }
            })
            .collect();
        ResourceConsumptionBatchRequestV1 {
            operation_key: batch_key(label),
            items,
            consumer_evidence: self.source(label),
            at: SimTime::EPOCH,
            completion_certificate: CompletionLeaseActivationCertificateV1 {
                acquisition: acquisition(label),
                acquisition_revision: ResourceRevision::INITIAL,
                operation_key: batch_key(label),
                prepared_grants: Vec::new(),
                locked_target_versions: Vec::new(),
                recipe_digest: String::new(),
                eligibility_time: SimTime::EPOCH,
                eligibility_envelope_digest: String::new(),
                activation_boundary: 0,
                semantic_digest: String::new(),
            },
        }
    }

    /// Takes the one lease of `batch`, held by `holder`, the office of its
    /// households, with a resource grant over every item's targets and a
    /// provider grant over the settlement record, and returns the batch with
    /// its certificate.
    fn lease(
        &mut self,
        label: &str,
        holder: u64,
        mut batch: ResourceConsumptionBatchRequestV1,
    ) -> ResourceConsumptionBatchRequestV1 {
        let source = batch.consumer_evidence.clone();
        let id = acquisition(label);
        let envelope = EligibilityEnvelopeV1::new(
            vec![source.clone()],
            BTreeMap::new(),
            BTreeSet::new(),
            Vec::new(),
            Vec::new(),
        )
        .expect("envelope");
        self.completion(&ResourceCompletionOperationV1::Acquire(
            RequestCompletionLeaseV1 {
                id: id.clone(),
                operation_key: batch.operation_key.clone(),
                holder: office(holder),
                operation_namespace: NAMESPACE.to_owned(),
                eligibility_time: SimTime::EPOCH,
                eligibility_envelope: envelope.clone(),
                recipe: CompletionCapacityRecipeV1 {
                    receipts: MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE,
                    mutations: 2,
                    reports_per_holder: 0,
                    holders: 0,
                    bytes: 4_096,
                },
                expected_participants: BTreeSet::from([
                    PLUGIN_NAME.to_owned(),
                    PROVIDER.to_owned(),
                ]),
                policy_class: CompletionPolicyClassV1::Guaranteed,
            },
        ));
        let resource_targets: Vec<_> = batch
            .locked_targets()
            .into_iter()
            .filter(|target| !matches!(target, CompletionLockedTargetV1::ExternalRecord { .. }))
            .collect();
        let resource_grant = grant(label, "resource");
        let provider_grant = grant(label, "provider");
        for (grant, owner, targets) in [
            (resource_grant.clone(), PLUGIN_NAME, resource_targets),
            (
                provider_grant.clone(),
                PROVIDER,
                vec![CompletionLockedTargetV1::ExternalRecord {
                    version: source.clone(),
                }],
            ),
        ] {
            let root = state(&self.canwu);
            let current_boundary = self.next_boundary();
            self.completion(&ResourceCompletionOperationV1::Grant(
                GrantCompletionCapacityV1 {
                    grant_id: grant,
                    acquisition: id.clone(),
                    expected_acquisition_revision: root.completion_leases.acquisitions[&id]
                        .revision,
                    owner_plugin: owner.to_owned(),
                    target_versions: targets,
                    current_boundary,
                },
            ));
        }
        for grant in [&resource_grant, &provider_grant] {
            let root = state(&self.canwu);
            let current_boundary = self.next_boundary();
            self.completion(&ResourceCompletionOperationV1::Prepare(
                PrepareCompletionCapacityV1 {
                    acquisition: id.clone(),
                    expected_acquisition_revision: root.completion_leases.acquisitions[&id]
                        .revision,
                    grant: grant.clone(),
                    expected_grant_revision: root.completion_leases.grants[grant].revision,
                    current_boundary,
                    eligibility_envelope_digest: envelope.digest.clone(),
                },
            ));
        }
        let root = state(&self.canwu);
        let current_boundary = self.next_boundary();
        self.completion(&ResourceCompletionOperationV1::Activate(
            ActivateCompletionLeaseV1 {
                acquisition: id.clone(),
                expected_acquisition_revision: root.completion_leases.acquisitions[&id].revision,
                grant: resource_grant,
                expected_grant_revision: root.completion_leases.grants[&grant(label, "resource")]
                    .revision,
                at: SimTime::EPOCH,
                current_boundary,
                eligibility_envelope_digest: envelope.digest,
            },
        ));
        batch.completion_certificate = state(&self.canwu)
            .completion_leases
            .certificate(&id)
            .cloned()
            .expect("the lease is active");
        batch
    }

    /// Queues `batch` through the provider's adapter ingress.
    fn send(&mut self, label: &str, batch: &ResourceConsumptionBatchRequestV1) {
        let provider_source = self.source(label);
        enqueue_resource_adapter_operation(
            &mut self.canwu,
            SimTime::EPOCH,
            &ResourceAdapterOperationV1 {
                provider_plugin: PROVIDER.to_owned(),
                provider_source,
                request: ResourceOperationRequestV1::ConsumeBatch(batch.clone()),
            },
        )
        .expect("adapter ingress");
    }

    /// Cancels one household's demand by a tracked command of its office.
    fn cancel(&mut self, holder: u64, household: usize) {
        self.commands += 1;
        let revision = state(&self.canwu).demands[&demand(household)].revision;
        self.canwu
            .enqueue_command(
                SimTime::EPOCH,
                0,
                CommandRequest::new(
                    CommandRequestId::new(self.commands),
                    self.canwu.revision(),
                    CommandEnvelope::new(
                        Issuer::Actor(PersonId::new(holder)),
                        resource_command(&ResourceCommandV1 {
                            subject: office(holder),
                            request: ResourceOperationRequestV1::CancelDemand(
                                ResourceCancelDemandRequestV1 {
                                    operation_key: key(&format!("fixture:cancel:{household}")),
                                    demand: demand(household),
                                    expected_demand_revision: revision,
                                },
                            ),
                        })
                        .expect("command"),
                    )
                    .at_time(SimTime::EPOCH),
                ),
            )
            .expect("tracked command");
        self.settle();
        assert!(matches!(
            self.canwu
                .command_attempts()
                .last()
                .expect("command attempt")
                .outcome,
            CommandAttemptOutcome::Accepted { .. }
        ));
        self.settle();
    }
}

fn acquisition(label: &str) -> CompletionLeaseAcquisitionId {
    CompletionLeaseAcquisitionId::new(format!("fixture:lease:{label}")).expect("acquisition")
}

fn grant(label: &str, owner: &str) -> CompletionCapacityGrantId {
    CompletionCapacityGrantId::new(format!("fixture:grant:{label}:{owner}")).expect("grant")
}

#[test]
fn a_batch_settles_each_item_under_one_lease_and_refuses_a_stale_item_alone() {
    let mut run = Run::new(&[1, 1, 1], 2_048, &[("month", &[0, 1, 2])]);
    let batch = run.batch("month", &[0, 1, 2]);
    let batch = run.lease("month", 1, batch);
    // The second household's demand is cancelled after the lease locked it.
    run.cancel(1, 1);
    let before = state(&run.canwu);
    let events = run.canwu.events().len();
    run.send("month", &batch);
    run.settle();
    assert!(skips(&run.canwu, events).is_empty());

    let root = state(&run.canwu);
    let request = ResourceOperationRequestV1::ConsumeBatch(batch.clone());
    let digest = request.request_digest().expect("digest");
    let outcome = &root.outcomes[&batch.operation_key];
    assert_eq!(outcome.kind, ResourceOperationKind::ConsumeBatch);
    assert_eq!(outcome.status, ResourceOperationStatus::Applied);
    assert_eq!(outcome.request_digest, digest);
    // The refused item's leg of 10 is the remainder.
    assert_eq!((outcome.quantity, outcome.remainder), (20, 10));
    for (index, household) in [0, 1, 2].into_iter().enumerate() {
        let item = &root.outcomes[&batch.item_operation_key(index)];
        assert_eq!(item.request_digest, digest);
        assert_eq!(item.kind, ResourceOperationKind::Consume);
        if household == 1 {
            assert_eq!(item.status, ResourceOperationStatus::Rejected);
            assert_eq!(item.rejection_code.as_deref(), Some("version_conflict"));
            assert_eq!(root.accounts[&account(household)].balance, 100);
        } else {
            assert_eq!(item.status, ResourceOperationStatus::Applied);
            let consumption =
                ResourceConsumptionId::new(format!("fixture:consumption:{household}"))
                    .expect("consumption");
            assert_eq!(
                item.result_ref,
                Some(ResourceRecordRefV1::Consumption(consumption.clone()))
            );
            assert_eq!(root.accounts[&account(household)].balance, 90);
            // The consumption names the fulfillment it recorded.
            let consumption = &root.consumptions[&consumption];
            let fulfillment = &root.fulfillments[&consumption.fulfillment];
            assert_eq!(fulfillment.demand, demand(household));
            assert_eq!(
                fulfillment.allocation_legs,
                vec![consumption.allocation_leg.clone()]
            );
        }
    }
    assert_eq!(
        root.conservation.admitted_consumption,
        before.conservation.admitted_consumption + 20
    );
    // One lease completed once, with one receipt, and left the root.
    let completed: Vec<_> = root
        .completion_leases
        .receipts
        .values()
        .filter(|receipt| receipt.action == CompletionLeaseReceiptActionV1::Completed)
        .collect();
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].operation_key, batch.operation_key);
    assert!(
        !root
            .completion_leases
            .acquisitions
            .contains_key(&acquisition("month"))
    );
    assert!(root.completion_leases.target_locks.is_empty());
    root.validate().expect("the root stays whole and valid");

    // A resend is answered by the batch outcome and settles nothing.
    let events = run.canwu.events().len();
    run.send("month", &batch);
    run.settle();
    assert!(skips(&run.canwu, events).is_empty());
    assert_eq!(state(&run.canwu), root);

    // The item keys are reserved for the batch.
    let item_key = ResourceOperationRequestV1::CancelDemand(ResourceCancelDemandRequestV1 {
        operation_key: batch.item_operation_key(0),
        demand: demand(0),
        expected_demand_revision: ResourceRevision::INITIAL,
    });
    assert!(item_key.names_reserved_operation_key());
    assert!(
        resource_adapter_ingress(
            SimTime::EPOCH,
            &ResourceAdapterOperationV1 {
                provider_plugin: PROVIDER.to_owned(),
                provider_source: run.source("month"),
                request: item_key,
            },
        )
        .is_err()
    );

    // A restored run and an exact replay agree.
    let (resource, provider) = plugins();
    let plugins: [&dyn SimulationPlugin; 2] = [&resource, &provider];
    let snapshot = run.canwu.snapshot_json().expect("snapshot");
    let restored = from_resource_snapshot_json(&snapshot, &plugins).expect("restore");
    assert_eq!(state(&restored), root);
    let replayed =
        replay_resource_from_journal(&plugins, &run.canwu.replay_journal()).expect("replay");
    assert_eq!(replayed.snapshot_json().expect("replay snapshot"), snapshot);

    // A boundary system that names the batch outcome and the consumptions
    // reads each applied consumption's fulfillment through the reader,
    // without naming or searching for it.
    run.canwu
        .settle_boundary(BoundaryRequest {
            at: SimTime::EPOCH,
            cadences: vec![SystemCadence::Daily],
        })
        .expect("the provider audits its batch");
    let audited = run
        .canwu
        .domain_record(&settlement("month"))
        .expect("settlement record")
        .payload["fulfillments"]
        .clone();
    assert_eq!(
        audited,
        json!({
            "fixture:consumption:0": demand(0),
            "fixture:consumption:2": demand(2),
        })
    );
}

#[test]
fn a_boundary_settles_at_most_its_budget_of_batch_items() {
    // A budget of three items. A batch of four is refused whole, keeps its
    // lease, and takes nothing of the budget, so the first batch of two
    // beside it settles; the second batch of two does not fit beside the
    // first and is deferred to the next boundary at the same instant.
    let mut run = Run::new(
        &[1, 1, 1, 1, 2, 2, 3, 3],
        3,
        &[
            ("large", &[0, 1, 2, 3]),
            ("first", &[4, 5]),
            ("second", &[6, 7]),
        ],
    );
    let large = run.batch("large", &[0, 1, 2, 3]);
    let large = run.lease("large", 1, large);
    let first = run.batch("first", &[4, 5]);
    let first = run.lease("first", 2, first);
    let second = run.batch("second", &[6, 7]);
    let second = run.lease("second", 3, second);
    let at = run.canwu.time();
    let events = run.canwu.events().len();
    run.send("large", &large);
    run.send("first", &first);
    run.send("second", &second);
    run.settle();
    let root = state(&run.canwu);
    let refused = &root.outcomes[&large.operation_key];
    assert_eq!(refused.status, ResourceOperationStatus::Rejected);
    assert_eq!(refused.rejection_code.as_deref(), Some("limit_exceeded"));
    // A batch refused whole leaves every item's quantity as its remainder.
    assert_eq!(refused.remainder, 40);
    assert!((0..4).all(|index| !root.outcomes.contains_key(&large.item_operation_key(index))));
    assert_eq!(
        root.completion_leases.acquisitions[&acquisition("large")].state,
        CompletionLeaseAcquisitionStateV1::Activated
    );
    assert_eq!(
        root.outcomes[&first.operation_key].status,
        ResourceOperationStatus::Applied
    );
    assert!(!root.outcomes.contains_key(&second.operation_key));
    let skipped = skips(&run.canwu, events);
    assert_eq!(skipped.len(), 1, "{skipped:?}");
    assert_eq!(
        skipped[0].reason,
        ResourceIngressSkipReasonV1::BoundaryBudget
    );
    assert_eq!(skipped[0].operation_key, Some(second.operation_key.clone()));
    assert_eq!(skipped[0].deferred_until, Some(at));

    // The host advances time: the deferred batch settles first, at the
    // instant its certificate binds, and nothing is skipped again.
    let events = run.canwu.events().len();
    let boundaries = run.canwu.boundaries().len();
    run.canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect("time advances past the deferred batch");
    assert!(skips(&run.canwu, events).is_empty());
    assert_eq!(run.canwu.boundaries()[boundaries].at, at);
    assert_eq!(run.canwu.time(), at + SimDuration::minutes(1));
    let root = state(&run.canwu);
    assert_eq!(
        root.outcomes[&second.operation_key].status,
        ResourceOperationStatus::Applied
    );
    assert_eq!(root.consumptions.len(), 4);
    assert!(
        !root
            .completion_leases
            .acquisitions
            .contains_key(&acquisition("second"))
    );
}

#[test]
fn a_full_batch_settles_under_the_canonical_limits() {
    // One office requests all of its households' demands and settles them
    // in one batch of the canonical per-boundary bound.
    let full = ResourceLimitsV1::CANONICAL_BATCH_ITEMS_PER_BOUNDARY;
    let households: Vec<usize> = (0..full).collect();
    let mut run = Run::build(
        &vec![1; full],
        ResourceLimitsV1::canonical(),
        false,
        &[("month", &households)],
    );
    let root = state(&run.canwu);
    assert!(root.consumption_batch_can_fit(full));
    assert!(!root.consumption_batch_can_fit(full + 1));
    let hot_before = root.hot_terminal_records();
    let batch = run.batch("month", &households);
    let batch = run.lease("month", 1, batch);
    let hot_leased = state(&run.canwu).hot_terminal_records();
    let events = run.canwu.events().len();
    run.send("month", &batch);
    run.settle();
    assert!(skips(&run.canwu, events).is_empty());
    let root = state(&run.canwu);
    let outcome = &root.outcomes[&batch.operation_key];
    assert_eq!(outcome.status, ResourceOperationStatus::Applied);
    assert_eq!((outcome.quantity, outcome.remainder), (10 * full as u64, 0));
    assert_eq!(root.consumptions.len(), full);
    // Each item leaves its outcome, consumption, fulfillment, and its
    // finished demand's closure; the batch its outcome and one receipt.
    assert_eq!(
        root.hot_terminal_records() - hot_leased,
        full * RESOURCE_CONSUMPTION_BATCH_ITEM_RECORDS + RESOURCE_CONSUMPTION_BATCH_RECORDS
    );
    assert!(root.hot_terminal_records() - hot_before <= root.limits.max_archive_candidates);
}

#[test]
fn a_batch_whose_records_do_not_fit_is_skipped_and_records_nothing() {
    // An archive budget of 64 candidates holds the records of ten items
    // beside the batch's own and a commit receipt, not of twelve.
    let mut limits = ResourceLimitsV1::canonical();
    limits.max_archive_candidates = 64;
    let households: Vec<usize> = (0..12).collect();
    let mut run = Run::build(&[1; 12], limits, false, &[("month", &households)]);
    let root = state(&run.canwu);
    assert!(root.consumption_batch_can_fit(10));
    assert!(!root.consumption_batch_can_fit(12));
    let batch = run.batch("month", &households);
    let batch = run.lease("month", 1, batch);
    let before = state(&run.canwu);
    let events = run.canwu.events().len();
    run.send("month", &batch);
    run.settle();
    let skipped = skips(&run.canwu, events);
    assert_eq!(skipped.len(), 1, "{skipped:?}");
    assert_eq!(
        skipped[0].reason,
        ResourceIngressSkipReasonV1::ReservedCapacity
    );
    assert_eq!(skipped[0].operation_key, Some(batch.operation_key.clone()));
    // Nothing is recorded: no outcome takes the batch key or an item key,
    // no leg is consumed, and the lease stays active for its holder to
    // release.
    let root = state(&run.canwu);
    assert_eq!(root, before);
    assert!(!root.outcomes.contains_key(&batch.operation_key));
    assert!((0..12).all(|index| !root.outcomes.contains_key(&batch.item_operation_key(index))));
    assert_eq!(
        root.completion_leases.acquisitions[&acquisition("month")].state,
        CompletionLeaseAcquisitionStateV1::Activated
    );
}

#[test]
fn a_batch_consumes_several_legs_of_one_account() {
    // A settlement office draws three households' demands from one granary
    // account: every item expects the revision the batch finds, and each
    // draws on the account as the items before it left it.
    let mut run = Run::build(
        &[1, 1, 1],
        ResourceLimitsV1::canonical(),
        true,
        &[("month", &[0, 1, 2])],
    );
    let batch = run.batch("month", &[0, 1, 2]);
    assert!(
        batch
            .items
            .iter()
            .all(|item| item.allocation.account == account(0)
                && item.expected_account_revision == batch.items[0].expected_account_revision)
    );
    let batch = run.lease("month", 1, batch);
    let before = state(&run.canwu).accounts[&account(0)].clone();
    run.send("month", &batch);
    run.settle();
    let root = state(&run.canwu);
    let outcome = &root.outcomes[&batch.operation_key];
    assert_eq!(outcome.status, ResourceOperationStatus::Applied);
    assert_eq!((outcome.quantity, outcome.remainder), (30, 0));
    for index in 0..3 {
        assert_eq!(
            root.outcomes[&batch.item_operation_key(index)].status,
            ResourceOperationStatus::Applied
        );
    }
    let after = &root.accounts[&account(0)];
    assert_eq!(after.balance, before.balance - 30);
    assert_eq!(
        after.revision,
        before
            .revision
            .next()
            .unwrap()
            .next()
            .unwrap()
            .next()
            .unwrap()
    );
    root.validate().expect("the root stays whole and valid");
}

/// The resource records of a run: its live root and its buckets.
fn resource_records(canwu: &Canwu) -> (DomainRecord, Vec<DomainRecord>) {
    let live_ref = resource_runtime_reference().into_untyped();
    let records: Vec<_> = canwu
        .domain_records()
        .filter(|record| record.owner == PLUGIN_NAME)
        .cloned()
        .collect();
    let live = records
        .iter()
        .find(|record| record.reference == live_ref)
        .cloned()
        .expect("live root");
    let buckets = records
        .into_iter()
        .filter(|record| record.reference != live_ref)
        .collect();
    (live, buckets)
}

/// The records with the hot consumption `id` naming `fulfillment`, its
/// digest recomputed and the ledger's byte count kept exact, so only the
/// link between the two records is forged.
fn forge_fulfillment(
    (mut live, mut buckets): (DomainRecord, Vec<DomainRecord>),
    id: &ResourceConsumptionId,
    fulfillment: &ResourceFulfillmentId,
) -> (DomainRecord, Vec<DomainRecord>) {
    let mut delta = 0_i64;
    let mut found = false;
    for bucket in &mut buckets {
        let Some(entries) = bucket
            .payload
            .get_mut("entries")
            .and_then(Value::as_array_mut)
        else {
            continue;
        };
        for entry in entries {
            if entry["key"]["kind"] != "consumption" || entry["key"]["id"] != id.as_str() {
                continue;
            }
            let before = serde_json::to_vec(entry).expect("entry").len();
            let mut consumption: ResourceConsumption =
                serde_json::from_value(entry["payload"]["value"].clone()).expect("consumption");
            consumption.fulfillment = fulfillment.clone();
            let mut detached = consumption.clone();
            detached.semantic_digest = String::new();
            consumption.semantic_digest =
                canonical_digest("canwu.resource.consumption.v1", &detached).expect("digest");
            entry["payload"]["value"] = serde_json::to_value(&consumption).expect("value");
            let after = serde_json::to_vec(entry).expect("entry").len();
            delta = i64::try_from(after).expect("length") - i64::try_from(before).expect("length");
            found = true;
        }
    }
    assert!(found, "the consumption is a hot terminal record");
    let bytes = live.payload["hot_terminal"]["bytes"]
        .as_i64()
        .expect("ledger bytes");
    live.payload["hot_terminal"]["bytes"] = json!(bytes + delta);
    (live, buckets)
}

#[test]
fn a_consumption_naming_another_fulfillment_is_refused_when_the_root_is_restored() {
    let mut run = Run::new(&[1, 1], 2_048, &[("month", &[0, 1])]);
    let batch = run.batch("month", &[0, 1]);
    let batch = run.lease("month", 1, batch);
    run.send("month", &batch);
    run.settle();
    let root = state(&run.canwu);
    let first = root.consumptions[&batch.items[0].consumption_id].clone();
    let second = root.consumptions[&batch.items[1].consumption_id].clone();
    let records = resource_records(&run.canwu);
    ResourceState::from_records(&records.0, &records.1).expect("the untouched root restores");
    // A consumption naming the other household's fulfillment, and one naming
    // a fulfillment the root does not hold.
    let missing = ResourceFulfillmentId::new("resource:fulfillment:missing").expect("id");
    for fulfillment in [&second.fulfillment, &missing] {
        let (live, buckets) = forge_fulfillment(records.clone(), &first.id, fulfillment);
        let error = ResourceState::from_records(&live, &buckets)
            .expect_err("a consumption must name the fulfillment it recorded");
        assert!(error.message.contains("fulfillment"), "{error:?}");
    }
}
