//! A completion expiry names the boundary that settles it.
//!
//! A host application cannot know which boundary will admit a packet it
//! queues, so `enqueue_resource_completion_operation` refuses an `Expire`
//! and an `ExpireExternalParticipants` from it, whatever boundary they name:
//! the expiry tick, which names no boundary, is the host's way to expire
//! grants. An expiry another plugin schedules is checked when it settles: one
//! that names a later boundary than the one settling it would expire grants
//! early, so it settles nothing, with an event, and the grant expires on time
//! through the tick.

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundaryRequest,
    BoundarySystemContract, Canwu, CanwuError, EntityRef, IngressClass, IngressPayload,
    KnowledgeHolderRef, PayloadSchema, PersonId, PluginIngressDescriptor, PluginIngressRequest,
    PluginIngressTarget, PluginRegistrar, Scenario, SimDuration, SimEvent, SimTime,
    SimulationPlugin, SimulationView, StateKey, StateVisibility, SystemCadence,
};
use canwu_resource::*;
use std::collections::{BTreeMap, BTreeSet};

const NAMESPACE: &str = "fixture.resource";
const CUSTODIAN: u64 = 1;

fn holder() -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(PersonId::new(CUSTODIAN))
}

fn key(value: &str) -> ResourceOperationKey {
    ResourceOperationKey::new(value).expect("operation key")
}

fn account() -> ResourceAccountId {
    ResourceAccountId::new("fixture:account:opening").expect("account")
}

/// A root with one custodian, a completion-capacity partition for it, one
/// account, and a requested completion acquisition.
fn world() -> ResourceState {
    let unit = ResourceUnitRevisionId::new("fixture:unit:measure:v1").expect("unit");
    let resource =
        ResourceDefinitionRevisionId::new("fixture:resource:stock:v1").expect("resource");
    let mut state = ResourceState::empty(ResourceLimitsV1::canonical()).expect("state");
    state
        .install_run_budget(
            RunBudgetRevisionV1 {
                revision: ResourceRevision::INITIAL,
                total_completion_units: 2_000_000,
                shared_pending_slots: 4,
                partitions: vec![CompletionCapacityPartitionV1 {
                    authority: holder(),
                    operation_namespace: NAMESPACE.to_owned(),
                    guaranteed_units: 400_000,
                    reserved_pending_slots: 8,
                    maximum_burst_units: 100_000,
                    request_token_capacity: 16,
                    request_token_refill_minutes: 1,
                    reacquire_cooldown_minutes: 1,
                    root_acquisition_cap_per_sim_time: 8,
                    guaranteed_max_wait_boundaries: 4,
                }],
                semantic_digest: String::new(),
            }
            .seal()
            .expect("budget"),
        )
        .expect("install budget");
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
    state
        .install_definition(ResourceDefinitionRevision {
            id: resource.clone(),
            resource: ResourceDefinitionId::new("fixture:resource:stock").expect("definition"),
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
            id: account(),
            revision: ResourceRevision::INITIAL,
            custodian: holder(),
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
        .apply_operation(&ResourceOperationRequestV1::Completion(
            ResourceCompletionOperationV1::Acquire(RequestCompletionLeaseV1 {
                id: acquisition(),
                operation_key: key("fixture:operation:expiring"),
                holder: holder(),
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
                recipe: CompletionCapacityRecipeV1 {
                    receipts: MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE,
                    mutations: 2,
                    reports_per_holder: 0,
                    holders: 0,
                    bytes: 1_024,
                },
                expected_participants: BTreeSet::from([PLUGIN_NAME.to_owned()]),
                policy_class: CompletionPolicyClassV1::Guaranteed,
            }),
        ))
        .expect("acquire");
    state
}

fn acquisition() -> CompletionLeaseAcquisitionId {
    CompletionLeaseAcquisitionId::new("fixture:lease:expiring").expect("acquisition")
}

fn state(canwu: &Canwu) -> ResourceState {
    resource_state(canwu)
        .expect("the whole root reads")
        .expect("resource state")
        .1
}

fn settle(canwu: &mut Canwu) -> Vec<ResourceIngressSkipV1> {
    let events = canwu.events().len();
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("a resource boundary never fails in this fixture");
    skips(&canwu.events()[events..])
}

fn skips(events: &[SimEvent]) -> Vec<ResourceIngressSkipV1> {
    events
        .iter()
        .filter_map(ResourceIngressSkipV1::from_event)
        .collect()
}

fn last_boundary(canwu: &Canwu) -> u64 {
    canwu
        .boundaries()
        .last()
        .map_or(0, |boundary| boundary.id.get())
}

const EXPIRY_PLUGIN: &str = "fixture-expiry";
const EXPIRY_INGRESS: &str = "fixture_expiry_v1";

/// Schedules, for each of its packets, one resource completion expiry that
/// names the boundary the packet gives.
struct ExpiryPlugin;

impl SimulationPlugin for ExpiryPlugin {
    fn name(&self) -> &'static str {
        EXPIRY_PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "00000000000000000000000000000000000000000000000000000000000000f7"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_ingress(PluginIngressDescriptor {
            name: EXPIRY_INGRESS.to_owned(),
            description: "Schedule one resource completion expiry".to_owned(),
            class: IngressClass::Information,
            payload_schema: PayloadSchema::Any,
        })?;
        let mut contract = BoundarySystemContract::new(
            "schedule-fixture-expiry-v1",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        contract.reads = vec![StateKey::core_ingress()];
        contract.plugin_ingress_targets = vec![PluginIngressTarget {
            target_plugin: PLUGIN_NAME.to_owned(),
            packet_type: RESOURCE_COMPLETION_INGRESS.to_owned(),
        }];
        contract.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(contract, schedule_expiry)
    }
}

fn schedule_expiry(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut directives = Vec::new();
    for ingress in &context.admitted_ingress {
        let Some(IngressPayload::Plugin {
            plugin,
            packet_type,
            payload,
            ..
        }) = view.ingress(*ingress)?.map(|ingress| &ingress.payload)
        else {
            continue;
        };
        if plugin != EXPIRY_PLUGIN || packet_type != EXPIRY_INGRESS {
            continue;
        }
        let expiry = ResourceCompletionOperationV1::Expire(ExpireCompletionCapacityV1 {
            at: context.at,
            current_boundary: payload["current_boundary"].as_u64().expect("boundary"),
            candidate_limit: 1_024,
        });
        directives.push(BoundaryDirective::SchedulePluginIngress {
            target_plugin: PLUGIN_NAME.to_owned(),
            after: SimDuration::ZERO,
            packet_type: RESOURCE_COMPLETION_INGRESS.to_owned(),
            priority: 0,
            payload: serde_json::to_value(&expiry).expect("expiry"),
            affected: Vec::new(),
        });
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

fn grant_state(canwu: &Canwu) -> CompletionGrantStateV1 {
    state(canwu).completion_leases.grants
        [&CompletionCapacityGrantId::new("fixture:grant:expiring").expect("grant")]
        .state
}

#[test]
fn a_completion_expiry_names_its_settling_boundary_or_settles_nothing() {
    let resource = ResourcePlugin::default();
    let plugins: [&dyn SimulationPlugin; 2] = [&resource, &ExpiryPlugin];
    let scenario = Scenario::new(SimTime::EPOCH, vec![EntityRef::Person(PersonId::new(1))])
        .with_domain_records(world().into_records().expect("the root persists"));
    let mut canwu = Canwu::new_with_plugins(21, scenario, &plugins).expect("runtime");
    while last_boundary(&canwu) < 8 {
        settle(&mut canwu);
    }
    let at = canwu.time();
    let next = last_boundary(&canwu) + 1;
    let due = next + 2;

    // The host's expiry is refused whatever boundary it names: the host
    // cannot know which boundary admits it.
    for current_boundary in [next, due] {
        let refused = enqueue_resource_completion_operation(
            &mut canwu,
            at,
            &ResourceCompletionOperationV1::Expire(ExpireCompletionCapacityV1 {
                at,
                current_boundary,
                candidate_limit: 1_024,
            }),
        )
        .expect_err("a host expiry is refused at enqueue");
        assert!(
            refused
                .message
                .contains("enqueue_resource_completion_expiry_tick"),
            "{refused:?}"
        );
    }

    // A grant that falls due at `due`, and a plugin expiry, scheduled at
    // `next`, that settles at the boundary after it but names `due`.
    let revision = state(&canwu).completion_leases.acquisitions[&acquisition()].revision;
    enqueue_resource_completion_operation(
        &mut canwu,
        at,
        &ResourceCompletionOperationV1::Grant(GrantCompletionCapacityV1 {
            grant_id: CompletionCapacityGrantId::new("fixture:grant:expiring").expect("grant"),
            acquisition: acquisition(),
            expected_acquisition_revision: revision,
            owner_plugin: PLUGIN_NAME.to_owned(),
            target_versions: vec![CompletionLockedTargetV1::Account {
                id: account(),
                revision: ResourceRevision::INITIAL,
            }],
            current_boundary: due - PREACTIVATION_LEASE_TTL_BOUNDARIES,
        }),
    )
    .expect("grant");
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            EXPIRY_PLUGIN,
            EXPIRY_INGRESS,
            at,
            serde_json::json!({ "current_boundary": due }),
        ))
        .expect("fixture expiry");
    assert!(settle(&mut canwu).is_empty());
    assert_eq!(last_boundary(&canwu), next);
    assert_eq!(grant_state(&canwu), CompletionGrantStateV1::Held);

    // The plugin's expiry names a later boundary than the one settling it:
    // it settles nothing, with an event, and expires nothing early.
    let skipped = settle(&mut canwu);
    assert_eq!(last_boundary(&canwu), next + 1);
    assert_eq!(skipped.len(), 1, "{skipped:?}");
    assert_eq!(
        skipped[0].reason,
        ResourceIngressSkipReasonV1::FailedSettlementChecks
    );
    assert_eq!(grant_state(&canwu), CompletionGrantStateV1::Held);
    let skipped_key = format!("resource:completion:expire:{}:{due}", at.as_minutes());
    assert!(
        !state(&canwu)
            .outcomes
            .keys()
            .any(|key| key.as_str() == skipped_key),
        "the skipped expiry took no key"
    );

    // The grant's own expiry tick expires it on time, under its boundary's
    // key, and the expired lease leaves the root.
    while last_boundary(&canwu) < due {
        settle(&mut canwu);
    }
    let root = state(&canwu);
    let expiry = &root.outcomes[&ResourceOperationKey::new(skipped_key).expect("key")];
    assert_eq!(expiry.status, ResourceOperationStatus::Applied);
    assert!(root.completion_leases.expiry_due.is_empty());
    assert!(
        !root
            .completion_leases
            .acquisitions
            .contains_key(&acquisition())
    );
    assert!(root.completion_leases.grants.is_empty());
}
