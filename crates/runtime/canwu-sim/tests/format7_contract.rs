use canwu_core::SimulationGranularity;
use canwu_sim::{ErrorCode, Simulation};
use serde_json::Value;

#[test]
fn format7_snapshot_is_strict_and_self_describing() {
    let (simulation, _) = Simulation::demo(17).expect("demo should initialize");
    let mut wire: Value = serde_json::from_str(
        &simulation
            .snapshot_json()
            .expect("format 7 snapshot should serialize"),
    )
    .expect("snapshot JSON should be an object");

    wire["snapshot_format_version"] = Value::from(6_u64);
    let Err(error) = Simulation::from_snapshot_json(&wire.to_string()) else {
        panic!("old formats reject");
    };
    assert_eq!(error.code, ErrorCode::UnsupportedSnapshotVersion);

    let mut wire: Value = serde_json::from_str(
        &simulation
            .snapshot_json()
            .expect("format 7 snapshot should serialize"),
    )
    .expect("snapshot JSON should be an object");
    wire["world"]["unexpected"] = Value::Bool(true);
    let Err(error) = Simulation::from_snapshot_json(&wire.to_string()) else {
        panic!("unknown fields reject");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
}

#[test]
fn exact_replay_uses_the_journal_initial_scenario_and_preserves_outbox_identity() {
    let (simulation, _) = Simulation::demo(23).expect("demo should initialize");
    let snapshot = simulation.snapshot();
    assert_eq!(snapshot.root_seed, 23);
    assert_ne!(snapshot.root_seed, snapshot.authority_root_seed);
    assert!(snapshot.initial_scenario.is_some());

    let journal = simulation.replay_journal();
    assert_eq!(
        journal.initial_scenario,
        snapshot.initial_scenario.clone().expect("scenario")
    );
    let replayed = Simulation::replay_from_journal(&[], &journal).expect("journal should replay");
    assert_eq!(replayed.snapshot(), snapshot);
    assert_eq!(
        replayed
            .outbox_entries()
            .expect("outbox should be readable"),
        simulation
            .outbox_entries()
            .expect("outbox should be readable")
    );

    let mut tampered_snapshot = snapshot;
    tampered_snapshot.authority_root_seed ^= 1;
    let Err(error) = Simulation::from_snapshot(tampered_snapshot) else {
        panic!("authority roots must remain bound to the run identity");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let mut tampered_journal = journal;
    tampered_journal.authority_root_seed ^= 1;
    let Err(error) = Simulation::replay_from_journal(&[], &tampered_journal) else {
        panic!("exact replay must reject an authority-root substitution");
    };
    assert_eq!(error.code, ErrorCode::ReplayEnvironmentMismatch);

    let mut journal_wire =
        serde_json::to_value(simulation.replay_journal()).expect("replay journal should serialize");
    journal_wire["initial_scenario"]["unexpected"] = Value::Bool(true);
    let Err(error) = Simulation::replay_from_journal_json(&[], &journal_wire.to_string()) else {
        panic!("replay journal JSON must reject unknown nested fields");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
}

#[test]
fn simulation_granularity_is_domain_neutral_and_stably_encoded() {
    let values = [
        (SimulationGranularity::Aggregate, "aggregate"),
        (SimulationGranularity::Group, "group"),
        (SimulationGranularity::Actor, "actor"),
    ];
    for (value, encoded) in values {
        assert_eq!(value.as_str(), encoded);
        assert_eq!(
            serde_json::to_string(&value).expect("granularity JSON"),
            format!("\"{encoded}\"")
        );
        assert_eq!(
            serde_json::from_str::<SimulationGranularity>(&format!("\"{encoded}\""))
                .expect("granularity decode"),
            value
        );
    }
}
