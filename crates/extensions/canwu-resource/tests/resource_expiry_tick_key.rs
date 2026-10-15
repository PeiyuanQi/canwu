//! An expiry tick settles its expiries under keys that name the instant and
//! the boundary, which a host application's completion operation can take
//! first.
//!
//! A host queues `Expire` for the next boundary, and a grant then falls due
//! at that boundary. The expiry tick the grant scheduled settles there and
//! finds the key taken: by the same expiry (a resend, which settles nothing)
//! or by an expiry of another candidate limit (skipped with an event naming
//! the key). Neither fails the boundary or writes over the host's outcome,
//! the whole root stays readable, and the tick, scheduled again, expires the
//! grant at the next boundary under that boundary's key.

use canwu_api::{
    BoundaryRequest, Canwu, EntityRef, KnowledgeHolderRef, PersonId, Scenario, SimEvent, SimTime,
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

#[test]
fn an_expiry_tick_whose_key_a_host_expiry_took_settles_nothing_and_expires_at_the_next_boundary() {
    for host_limit in [7, 1_024] {
        let plugin = ResourcePlugin::default();
        let scenario = Scenario::new(SimTime::EPOCH, vec![EntityRef::Person(PersonId::new(1))])
            .with_domain_records(world().into_records().expect("the root persists"));
        let mut canwu = Canwu::new_with_plugins(21, scenario, &[&plugin]).expect("runtime");
        while last_boundary(&canwu) < 8 {
            settle(&mut canwu);
        }
        let at = canwu.time();
        let next = last_boundary(&canwu) + 1;
        let taken = next + 1;

        // At the next boundary, the host's expiry for the one after it,
        // which finds nothing due and records its outcome under that
        // boundary's key, then a grant that falls due there.
        let host_expiry = ResourceCompletionOperationV1::Expire(ExpireCompletionCapacityV1 {
            at,
            current_boundary: taken,
            candidate_limit: host_limit,
        });
        enqueue_resource_completion_operation(&mut canwu, at, &host_expiry).expect("host expiry");
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
                current_boundary: taken - PREACTIVATION_LEASE_TTL_BOUNDARIES,
            }),
        )
        .expect("grant");
        assert!(settle(&mut canwu).is_empty());
        assert_eq!(last_boundary(&canwu), next);
        let host_outcome = state(&canwu).outcomes[&host_expiry.operation_key()].clone();
        assert!(
            state(&canwu)
                .completion_leases
                .expiry_due
                .range(..=taken)
                .any(|(_, grants)| !grants.is_empty()),
            "the grant falls due at the boundary whose key the host took"
        );

        // The grant scheduled an expiry tick, which settles at that boundary
        // and finds the key taken: a different expiry is skipped with an
        // event, the same expiry is a resend. Neither writes over the host's
        // outcome.
        let skipped = settle(&mut canwu);
        assert_eq!(last_boundary(&canwu), taken);
        if host_limit == 1_024 {
            assert!(skipped.is_empty(), "the same expiry is a resend");
        } else {
            assert_eq!(skipped.len(), 1, "{skipped:?}");
            assert_eq!(
                skipped[0].reason,
                ResourceIngressSkipReasonV1::OperationKeyTaken
            );
            assert_eq!(skipped[0].operation_key, Some(host_expiry.operation_key()));
        }
        let root = state(&canwu);
        assert_eq!(root.outcomes[&host_expiry.operation_key()], host_outcome);
        assert!(!root.completion_leases.expiry_due.is_empty());

        // The tick is scheduled again, and the next boundary's key is free:
        // the grant expires.
        assert!(canwu.step_canonical().expect("the tick settles").is_some());
        let root = state(&canwu);
        assert!(root.completion_leases.expiry_due.is_empty());
        assert_eq!(
            root.completion_leases.grants
                [&CompletionCapacityGrantId::new("fixture:grant:expiring").expect("grant")]
                .state,
            CompletionGrantStateV1::Expired
        );
    }
}
