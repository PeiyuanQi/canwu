use canwu_core::{
    EntityRef, KnowledgeHolderRef, KnowledgeRecordId, KnowledgeRecordKind, KnowledgeSchemaId,
    PersonId,
};
use canwu_knowledge::{KnowledgeOrigin, KnowledgeRecord};
use canwu_sim::{ErrorCode, Scenario, Simulation, demo_scenario};
use canwu_time::SimTime;
use serde_json::json;
use std::collections::BTreeMap;

fn empty_scenario() -> Scenario {
    Scenario::new(SimTime::EPOCH, Vec::new())
}

#[test]
fn scenario_generic_ledger_is_kernel_issued_only() {
    let simulation = Simulation::new(7, empty_scenario())
        .expect("an empty generic ledger should preserve baseline admission");
    let snapshot: serde_json::Value = serde_json::from_str(
        &simulation
            .snapshot_json()
            .expect("baseline snapshot should serialize"),
    )
    .expect("baseline snapshot JSON should parse");
    assert!(snapshot["knowledge"].get("records").is_none());
    assert!(snapshot.get("next_knowledge_record_id").is_none());

    let holder = KnowledgeHolderRef::Person(PersonId::new(1));
    let record = KnowledgeRecord {
        id: KnowledgeRecordId::new(1),
        holder: holder.clone(),
        schema: KnowledgeSchemaId::new(
            KnowledgeRecordKind::new("fixture.knowledge", "preselected"),
            1,
        ),
        subjects: vec![],
        payload: json!(null),
        as_of: None,
        learned_at: SimTime::EPOCH,
        confidence_per_mille: 1_000,
        origin: KnowledgeOrigin {
            method: "scenario".to_owned(),
            evidence: vec![],
        },
        supersedes: vec![],
        contradicts: vec![],
    };
    let mut scenario = empty_scenario();
    scenario.knowledge.records = BTreeMap::from([(
        holder,
        BTreeMap::from([(KnowledgeRecordId::new(1), record)]),
    )]);

    let error = Simulation::new(7, scenario)
        .err()
        .expect("scenario authors cannot preselect generic IDs, times, or origins");
    assert_eq!(error.code, ErrorCode::InvalidKnowledgeRecord);
}

#[test]
fn compatibility_scenarios_derive_and_require_the_complete_entity_registry() {
    let (mut omitted, _) = demo_scenario();
    let expected = omitted.entities.clone();
    omitted.entities.clear();
    let restored = Simulation::new(35, omitted)
        .expect("omitted compatibility registry should derive from the world");
    assert_eq!(restored.entities().cloned().collect::<Vec<_>>(), expected);

    let (mut partial, _) = demo_scenario();
    partial.entities.pop();
    let error = Simulation::new(35, partial)
        .err()
        .expect("partial compatibility registry must be rejected");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
}

#[test]
fn generic_entity_registry_is_bound_to_snapshot_commitments() {
    let entity = EntityRef::Person(PersonId::new(1));
    let simulation = Simulation::new(7, Scenario::new(SimTime::EPOCH, vec![entity]))
        .expect("generic entity scenario should initialize");
    let mut snapshot = simulation.snapshot();
    snapshot.entities.push(EntityRef::Person(PersonId::new(2)));

    let error = Simulation::from_snapshot(snapshot)
        .err()
        .expect("entity-registry tampering must invalidate the snapshot");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
}
