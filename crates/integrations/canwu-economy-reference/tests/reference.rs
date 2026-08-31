use canwu_api::{Canwu, KnowledgeHolderRef, PersonId, Scenario, SimTime};
use canwu_economy_reference::{
    GrainDecision, LocalEconomyId, ProjectionProviderRegistryV1, ProjectionQueryResultV1,
    SyntheticGrainLoop, economy_reference_state,
};
use canwu_resource::ResourceScopeId;

#[test]
fn grain_loop_conserves_material_and_routes_force_supply() {
    let decisions = (0..14).map(|month| {
        if month == 4 {
            GrainDecision::RequisitionForForce
        } else if month % 3 == 0 {
            GrainDecision::ReliefFirst
        } else {
            GrainDecision::Balanced
        }
    });
    let summary = SyntheticGrainLoop::new()
        .expect("fixture")
        .run_fourteen_months(decisions)
        .expect("run");
    assert_eq!(summary.frames.len(), 14);
    assert!(summary.transport_executions > 0);
    assert!(summary.total_harvest > 0);
    assert_eq!(summary.closed_route_months, vec![1, 2]);
    assert_eq!(summary.rerouted_months, vec![3]);
    assert!(
        summary
            .frames
            .iter()
            .any(|frame| frame.decision == GrainDecision::RequisitionForForce)
    );
}

#[test]
fn unconfigured_projection_returns_a_stable_holder_bound_unavailable_dto() {
    let holder = KnowledgeHolderRef::Person(PersonId::new(1));
    let scope = ResourceScopeId::new("economy:scope:test").expect("scope");
    let canwu = Canwu::new(7, Scenario::new(SimTime::EPOCH, Vec::new())).expect("simulation");
    let registry = ProjectionProviderRegistryV1::new(Vec::new()).expect("registry");
    let ProjectionQueryResultV1::Unavailable(first) = registry.project(&canwu, &holder, &scope)
    else {
        panic!("unconfigured scope must fail closed");
    };
    let ProjectionQueryResultV1::Unavailable(second) = registry.project(&canwu, &holder, &scope)
    else {
        panic!("unconfigured scope must fail closed deterministically");
    };
    assert_eq!(first.holder, holder);
    assert_eq!(first.scope, scope);
    assert_eq!(first.blocker_code, "scope_unconfigured");
    assert_eq!(first, second);
}

#[test]
fn local_economy_scopes_are_unique_for_authoritative_externality_targeting() {
    let harness = SyntheticGrainLoop::new().expect("fixture");
    let (_, mut state) = economy_reference_state(harness.canwu())
        .expect("economy query")
        .expect("economy runtime");
    let mut duplicate = state
        .local_economies
        .values()
        .next()
        .expect("fixture economy")
        .clone();
    duplicate.id =
        LocalEconomyId::new("canwu.economy-reference:economy:duplicate-scope").expect("ID");
    state
        .local_economies
        .insert(duplicate.id.clone(), duplicate);

    let error = state
        .validate()
        .expect_err("one resource scope cannot ambiguously target two local economies");
    assert!(
        error
            .message
            .contains("resource scopes must be unique for exact externality targeting")
    );
}
