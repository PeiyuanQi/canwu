use canwu_core::{
    KnowledgeHolderRef, KnowledgeRecordId, KnowledgeRecordKind, KnowledgeSchemaId, PersonId,
};
use canwu_knowledge::{KnowledgeOrigin, KnowledgeRecord, KnowledgeSnapshot};
use canwu_sim::{ErrorCode, Scenario, Simulation};
use canwu_time::SimTime;
use canwu_world::WorldSnapshot;
use serde_json::json;
use std::collections::BTreeMap;

fn empty_scenario() -> Scenario {
    Scenario {
        start_time: SimTime::EPOCH,
        world: WorldSnapshot::default(),
        knowledge: KnowledgeSnapshot::default(),
        domain_records: vec![],
    }
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
