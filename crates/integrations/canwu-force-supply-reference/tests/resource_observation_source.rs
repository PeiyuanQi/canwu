//! A force's supply observation that cites the resource provider is checked
//! against the stock of the accounts the force holder holds, which the
//! resource root keeps in live buckets beside its live root: an interval that
//! holds that stock is recorded, and one that excludes it is refused.

use canwu_api::{
    BoundaryRequest, Canwu, CommandEnvelope, CommandRequest, CommandRequestId, EntityRef, Issuer,
    KnowledgeHolderRef, PersonId, Scenario, SimTime, SimulationPlugin,
};
use canwu_economy_reference_content::{compile_content_pack, synthetic_grain_fixture};
use canwu_force_supply_reference::*;
use canwu_resource::{
    ResourceAccount, ResourceAccountId, ResourceDefinitionId, ResourceDefinitionRevision,
    ResourceLimitsV1, ResourcePlugin, ResourceQualityId, ResourceRevision, ResourceScopeId,
    ResourceState, ResourceUnitRevision, resource_runtime_reference,
};
use std::collections::BTreeSet;

const HOLDER: u64 = 1;

fn holder(id: u64) -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(PersonId::new(id))
}

fn force_state() -> (
    ForceSupplyStateV1,
    ReferenceForceId,
    ForceSupplyRequirementV1,
) {
    let content = compile_content_pack(&synthetic_grain_fixture()).expect("compiled fixture");
    let mut state = ForceSupplyStateV1::from_compiled_content(content).expect("force content");
    let profile = state
        .profiles
        .values()
        .find(|profile| {
            profile
                .requirements
                .iter()
                .any(|requirement| requirement.kind == SupplyResourceKind::Food)
        })
        .cloned()
        .expect("food profile");
    let requirement = profile
        .requirements
        .iter()
        .find(|requirement| requirement.kind == SupplyResourceKind::Food)
        .cloned()
        .expect("food requirement");
    let force_id =
        ReferenceForceId::new("canwu.force-supply-reference:force:observed").expect("ID");
    state.forces.insert(
        force_id.clone(),
        ReferenceForce {
            id: force_id.clone(),
            revision: 1,
            holder: holder(HOLDER),
            profile: profile.id.clone(),
            active: true,
            readiness_per_mille: 900,
            fatigue_per_mille: 100,
            cohesion_per_mille: 800,
            disease_per_mille: 0,
            desertion_per_mille: 0,
            supply_posture: "holding".to_owned(),
            due: profile
                .requirements
                .iter()
                .map(|requirement| {
                    (
                        requirement.id.clone(),
                        DueRequirementStateV1 {
                            requirement: requirement.id.clone(),
                            // Not due during the fixture.
                            next_due: SimTime::from_minutes(100_000),
                            persisted_remainder_minutes: 0,
                        },
                    )
                })
                .collect(),
            blocked_by_active_requisition: None,
        },
    );
    for requirement in &profile.requirements {
        state
            .due_index
            .entry(SimTime::from_minutes(100_000))
            .or_default()
            .insert((force_id.clone(), requirement.id.clone()));
    }
    let grant = ForceObserverGrantV1 {
        id: ForceObserverGrantId::new("canwu.force-supply-reference:grant:observed").expect("ID"),
        holder: holder(HOLDER),
        force: force_id.clone(),
        role: ForceObservationRole::Commander,
        observation_delay_minutes: 0,
        confidence_per_mille: 1_000,
    };
    state.observation_grants.insert(grant.id.clone(), grant);
    state.validate().expect("force state");
    (state, force_id, requirement)
}

/// A resource root with the requirement's resource and unit, two accounts of
/// the force holder (25 in all), and a larger account of another holder.
fn resource_state(requirement: &ForceSupplyRequirementV1) -> ResourceState {
    let mut state = ResourceState::empty(ResourceLimitsV1::canonical()).expect("resource state");
    state
        .install_unit(ResourceUnitRevision {
            id: requirement.unit_revision.clone(),
            revision: ResourceRevision::INITIAL,
            symbol: "u".to_owned(),
            scale_numerator: 1,
            scale_denominator: 1,
            semantic_digest: "0".repeat(64),
        })
        .expect("unit");
    state
        .install_definition(ResourceDefinitionRevision {
            id: requirement.resource_revision.clone(),
            resource: ResourceDefinitionId::new("fixture:resource:food").expect("definition"),
            revision: ResourceRevision::INITIAL,
            canonical_unit: requirement.unit_revision.clone(),
            quality: ResourceQualityId::new("fixture:quality:standard").expect("quality"),
            scope: ResourceScopeId::new("fixture:scope:camp").expect("scope"),
            effective_from: SimTime::EPOCH,
            effective_until: None,
            process_suitability: BTreeSet::new(),
            semantic_digest: "1".repeat(64),
        })
        .expect("definition");
    for (name, custodian, balance) in [
        ("first", HOLDER, 10),
        ("second", HOLDER, 15),
        ("other", 2, 1_000),
    ] {
        state
            .install_opening_account(ResourceAccount {
                id: ResourceAccountId::new(format!("fixture:account:{name}")).expect("account"),
                revision: ResourceRevision::INITIAL,
                custodian: holder(custodian),
                resource_revision: requirement.resource_revision.clone(),
                unit_revision: requirement.unit_revision.clone(),
                balance,
                capacity: None,
                protected_floor_policy: None,
                closed: false,
                place_scope: None,
            })
            .expect("account");
    }
    state
}

fn observe(
    canwu: &mut Canwu,
    request: u64,
    force: &ReferenceForceId,
    requirement: &ForceSupplyRequirementV1,
    (low, high): (u64, u64),
) -> ForceOperationOutcomeV1 {
    let runtime = canwu_api::typed_domain_record(&*canwu, &force_supply_runtime_reference())
        .expect("force runtime")
        .decode_payload::<ForceSupplyRuntimeRecord>()
        .expect("force state");
    let source = canwu
        .current_domain_record_version(&resource_runtime_reference().into_untyped())
        .expect("source query")
        .expect("resource root");
    let operation_id = ForceOperationId::new(format!(
        "canwu.force-supply-reference:operation:observe-{request}"
    ))
    .expect("ID");
    let command = force_supply_command(&ForceCommandEnvelopeV1 {
        operation_id: operation_id.clone(),
        holder: holder(HOLDER),
        expected_runtime_revision: runtime.revision,
        operation: ForceOperationV1::RecordSupplyObservation {
            force: force.clone(),
            observation: ForceSupplyObservationV1 {
                requirement: requirement.id.clone(),
                known_stock_low: low,
                known_stock_high: high,
                demand_forecast: 40,
                arrival_state: "holding".to_owned(),
                source: ForceSupplyObservationSourceV1::ResourceProvider,
                observed_at: SimTime::EPOCH,
                confidence_per_mille: 900,
                source_versions: vec![source],
            },
        },
    })
    .expect("command");
    canwu
        .enqueue_command(
            SimTime::EPOCH,
            0,
            CommandRequest::new(
                CommandRequestId::new(request),
                canwu.revision(),
                CommandEnvelope::new(Issuer::Actor(PersonId::new(HOLDER)), command)
                    .at_time(SimTime::EPOCH),
            ),
        )
        .expect("tracked command");
    for _ in 0..2 {
        canwu
            .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
            .expect("boundary");
    }
    canwu_api::typed_domain_record(&*canwu, &force_supply_runtime_reference())
        .expect("force runtime")
        .decode_payload::<ForceSupplyRuntimeRecord>()
        .expect("force state")
        .outcomes[&operation_id]
        .clone()
}

#[test]
fn a_resource_provider_observation_is_checked_against_the_holder_account_stock() {
    let (force, force_id, requirement) = force_state();
    let resource = resource_state(&requirement);
    let scenario = Scenario::new(
        SimTime::EPOCH,
        vec![
            EntityRef::Person(PersonId::new(HOLDER)),
            EntityRef::Person(PersonId::new(2)),
        ],
    )
    .with_domain_records(
        resource
            .into_records()
            .expect("resource root")
            .into_iter()
            .chain([force.into_initial_record().expect("force root")])
            .collect(),
    );
    let resource_plugin = ResourcePlugin::default();
    let plugins: [&dyn SimulationPlugin; 2] = [&resource_plugin, &ForceSupplyReferencePlugin];
    let mut canwu = Canwu::new_with_plugins(4, scenario, &plugins).expect("runtime");

    // The holder's two accounts hold 25; the other holder's account does not
    // count.
    let recorded = observe(&mut canwu, 1, &force_id, &requirement, (20, 30));
    assert!(recorded.applied, "{:?}", recorded.rejection_reason);
    let refused = observe(&mut canwu, 2, &force_id, &requirement, (0, 10));
    assert!(!refused.applied);
    assert!(
        refused
            .rejection_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("excludes its provider stock")),
        "{refused:?}"
    );
}
