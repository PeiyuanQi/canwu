use canwu_core::{DomainRecordKind, DomainRecordRef, EntityRef, TerritoryId};
use canwu_sim::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundaryRequest,
    BoundarySystemContract, CanwuError, DomainRecordClass, DomainRecordDraft, DomainRecordMutation,
    DomainRecordMutationPolicy, DomainRecordSchema, ENGINE_VERSION, ErrorCode,
    KnowledgeHolderPolicy, PluginRegistrar, RandomDrawAddress, RandomStreamKey, ReplayJournal,
    SNAPSHOT_FORMAT_VERSION, Simulation, SimulationPlugin, SimulationView, StateKey,
    StateVisibility, SystemCadence,
};
use canwu_time::{SimDuration, SimTime};
use serde_json::{Value, json};

const EMPTY: &str = include_str!("fixtures/format4/v4-empty-snapshot.json");
const EMPTY_BAD: &str = include_str!("fixtures/format4/v4-empty-snapshot-tampered-checkpoint.json");
const PLUGIN: &str = include_str!("fixtures/format4/v4-plugin-random-snapshot.json");
const PLUGIN_BAD: &str =
    include_str!("fixtures/format4/v4-plugin-random-snapshot-tampered-draw.json");
const REPLAY: &str = include_str!("fixtures/format4/v4-plugin-random-replay-journal.json");
const REPLAY_BAD: &str =
    include_str!("fixtures/format4/v4-plugin-random-replay-journal-tampered-boundary.json");
const CHECKPOINT: &str = include_str!("fixtures/format4/v4-plugin-random-checkpoint-journal.json");
const CHECKPOINT_BAD: &str =
    include_str!("fixtures/format4/v4-plugin-random-checkpoint-journal-tampered-cursor.json");
const COMPACT: &str = include_str!("fixtures/format4/v4-plugin-random-compact-continuation.json");
const COMPACT_BAD: &str =
    include_str!("fixtures/format4/v4-plugin-random-compact-continuation-tampered-segment.json");
const METADATA: &str = include_str!("fixtures/format4/v4-golden-metadata.json");

struct FixturePlugin;

fn fixture_kind() -> DomainRecordKind {
    DomainRecordKind::new("fixture.migration", "dispatch")
}

fn fixture_stream() -> RandomStreamKey {
    RandomStreamKey::new("fixture-migration", "boundary-roll", 1)
}

fn apply_fixture(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let roll = view.random_range(&fixture_stream(), 10_000, "fixture boundary roll")?;
    let mut directives = vec![BoundaryDirective::SetComponent {
        state: StateKey::new("fixture-migration", "roll"),
        entity: EntityRef::Territory(TerritoryId::new(1)),
        component: "value".to_owned(),
        value: Value::from(roll),
        summary: format!("Fixture roll {roll}"),
    }];
    if context.boundary_id.get() == 1 {
        directives.push(BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Create {
                record: DomainRecordDraft::new(
                    DomainRecordRef {
                        kind: fixture_kind(),
                        id: "dispatch-1".to_owned(),
                    },
                    json!({ "status": "sealed" }),
                ),
            },
            summary: "Create migration dispatch fixture".to_owned(),
        });
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

impl SimulationPlugin for FixturePlugin {
    fn name(&self) -> &'static str {
        "fixture-migration"
    }

    fn version(&self) -> &'static str {
        "1.0.0"
    }

    fn semantic_hash(&self) -> &'static str {
        "58d3da9e7c142956f655c9a7b98d25e6b586729a5f01bf4f80db359cf4f31245"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let schema = DomainRecordSchema::new(fixture_kind(), DomainRecordClass::Record);
        let record_state = schema.state_key();
        registrar.register_record_schema(schema)?;
        let mut contract = BoundarySystemContract::new(
            "produce-fixture",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        contract.writes = vec![record_state, StateKey::new("fixture-migration", "roll")];
        contract.random_streams = vec![fixture_stream()];
        contract.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(contract, apply_fixture)
    }
}

fn normalized_fixture_hash(body: &str) -> String {
    let normalized = body.replace("\r\n", "\n");
    blake3::hash(normalized.as_bytes()).to_hex().to_string()
}

fn assert_invalid_snapshot(body: &str, plugins: &[&dyn SimulationPlugin]) {
    let result = if plugins.is_empty() {
        Simulation::from_snapshot_json(body)
    } else {
        Simulation::from_snapshot_json_with_plugins(body, plugins)
    };
    let Err(error) = result else {
        panic!("tampered format-4 snapshot must fail closed");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
}

#[test]
fn checked_in_fixtures_are_exact_0_4_canonical_artifacts() {
    let metadata: Value = serde_json::from_str(METADATA).expect("metadata should parse");
    assert_eq!(metadata["generator"]["engine_version"], "0.4.0");
    assert_eq!(metadata["generator"]["snapshot_format_version"], 4);
    assert_eq!(
        metadata["generator"]["source_commit"],
        "570efa54eb5f6817c38e80bc0fe67060e89170a4"
    );

    for (name, body, key) in [
        ("v4-empty-snapshot.json", EMPTY, "empty_snapshot"),
        (
            "v4-plugin-random-snapshot.json",
            PLUGIN,
            "plugin_random_snapshot",
        ),
        (
            "v4-plugin-random-replay-journal.json",
            REPLAY,
            "replay_journal",
        ),
        (
            "v4-plugin-random-checkpoint-journal.json",
            CHECKPOINT,
            "checkpoint_journal",
        ),
        (
            "v4-plugin-random-compact-continuation.json",
            COMPACT,
            "compact_continuation",
        ),
    ] {
        assert_eq!(metadata["fixtures"][key]["file"], name);
        assert_eq!(
            metadata["fixtures"][key]["canonical_json_blake3"],
            normalized_fixture_hash(body)
        );
    }

    let plugin: Value = serde_json::from_str(PLUGIN).expect("plugin snapshot should parse");
    let recorded = &metadata["fixtures"]["plugin_random_snapshot"];
    assert_eq!(recorded["checkpoint_hash"], plugin["checkpoint_hash"]);
    assert_eq!(recorded["commitment_roots"], plugin["commitment_roots"]);
    assert_eq!(recorded["boundary_hash"], plugin["boundaries"][0]["hash"]);
    assert_eq!(
        recorded["boundary_state_hash"],
        plugin["boundaries"][0]["state_hash"]
    );
    assert_eq!(recorded["random_draw_count"], 1);
    assert_eq!(recorded["domain_record_count"], 1);
    assert!(
        plugin["boundaries"][0]["state_hash"]
            .as_str()
            .is_some_and(|value| value.starts_with("v1:"))
    );
    assert!(plugin["random_draws"][0].get("position").is_some());
    assert!(plugin["random_draws"][0].get("address").is_none());
    assert_eq!(
        metadata["fixtures"]["compact_continuation"]["segment_count"],
        2
    );
}

#[test]
fn snapshots_migrate_to_format_5_and_continue_deterministically() {
    let empty = Simulation::from_snapshot_json(EMPTY).expect("empty V4 should migrate");
    assert_eq!(empty.snapshot().engine_version, ENGINE_VERSION);
    assert_eq!(
        empty.snapshot().snapshot_format_version,
        SNAPSHOT_FORMAT_VERSION
    );

    let first = Simulation::from_snapshot_json_with_plugins(PLUGIN, &[&FixturePlugin])
        .expect("plugin V4 should migrate");
    let second = Simulation::from_snapshot_json_with_plugins(PLUGIN, &[&FixturePlugin])
        .expect("the same V4 should migrate twice");
    assert_eq!(
        first.snapshot_json().unwrap(),
        second.snapshot_json().unwrap()
    );
    let migrated_snapshot = first.snapshot();
    let migrated_descriptor = migrated_snapshot
        .plugin_descriptors
        .first()
        .expect("the migrated plugin descriptor should remain present");
    let migrated_schema = migrated_descriptor
        .record_schemas
        .first()
        .expect("the migrated record schema should remain present");
    assert_eq!(
        migrated_schema.holder_policy,
        KnowledgeHolderPolicy::Disallowed
    );
    assert_eq!(
        migrated_schema.mutation_policy,
        DomainRecordMutationPolicy::Versioned
    );
    assert!(migrated_descriptor.knowledge_schemas.is_empty());
    assert_eq!(first.domain_records().count(), 1);
    assert_eq!(first.random_draws().len(), 1);
    assert!(matches!(
        first.random_draws()[0].address,
        RandomDrawAddress::Sequential { position: 0 }
    ));

    let mut continued = first;
    continued
        .settle_boundary(
            BoundaryRequest::at(SimTime::EPOCH + SimDuration::days(2))
                .with_cadence(SystemCadence::Daily),
        )
        .expect("migrated format-5 state should continue");
    assert_eq!(continued.random_draws().len(), 2);
    assert!(matches!(
        continued.random_draws()[1].address,
        RandomDrawAddress::Sequential { position: 1 }
    ));
}

#[test]
fn legacy_replay_is_validated_but_exact_0_4_history_remains_historical() {
    let journal: ReplayJournal =
        serde_json::from_str(REPLAY).expect("valid V4 replay should parse");
    assert_eq!(journal.engine_version, "0.4.0");
    assert_eq!(journal.snapshot_format_version, 4);
    assert_eq!(journal.revision_format_version, 0);
    let (scenario, _) = canwu_sim::demo_scenario();
    let Err(error) = Simulation::replay_from_journal(scenario, &[&FixturePlugin], &journal) else {
        panic!("the 0.5 runtime must not claim exact 0.4 intermediate replay");
    };
    assert_eq!(error.code, ErrorCode::LegacyReplayUnavailable);
    assert!(serde_json::from_str::<ReplayJournal>(REPLAY_BAD).is_err());
}

#[test]
fn checkpoint_journals_migrate_and_compact_history_continues() {
    let checkpoint =
        Simulation::from_checkpoint_journal_json_with_plugins(CHECKPOINT, &[&FixturePlugin])
            .expect("V4 checkpoint journal should migrate");
    assert_eq!(checkpoint.random_draws().len(), 1);

    let mut compact =
        Simulation::from_checkpoint_journal_json_with_plugins(COMPACT, &[&FixturePlugin])
            .expect("V4 compact continuation should migrate");
    assert_eq!(compact.random_draws().len(), 2);
    compact
        .settle_boundary(
            BoundaryRequest::at(SimTime::EPOCH + SimDuration::days(3))
                .with_cadence(SystemCadence::Daily),
        )
        .expect("migrated compact history should continue");
    assert_eq!(compact.random_draws().len(), 3);
}

#[test]
fn every_tamper_companion_and_format_5_smuggling_attempt_fails_closed() {
    assert_invalid_snapshot(EMPTY_BAD, &[]);
    assert_invalid_snapshot(PLUGIN_BAD, &[&FixturePlugin]);

    let checkpoint =
        Simulation::from_checkpoint_journal_json_with_plugins(CHECKPOINT_BAD, &[&FixturePlugin]);
    assert!(checkpoint.is_err());
    let compact =
        Simulation::from_checkpoint_journal_json_with_plugins(COMPACT_BAD, &[&FixturePlugin]);
    assert!(compact.is_err());

    let mut smuggled: Value = serde_json::from_str(PLUGIN).expect("fixture should parse");
    smuggled["random_draws"][0]["address"] =
        json!({ "type": "sequential", "value": { "position": 0 } });
    assert_invalid_snapshot(
        &serde_json::to_string(&smuggled).expect("smuggled fixture should serialize"),
        &[&FixturePlugin],
    );

    let mut nested_unknown: Value = serde_json::from_str(EMPTY).expect("fixture should parse");
    nested_unknown["world"]["format_5_only"] = Value::Bool(true);
    assert_invalid_snapshot(
        &serde_json::to_string(&nested_unknown).expect("unknown field should serialize"),
        &[],
    );

    let mut schema_smuggled: Value = serde_json::from_str(PLUGIN).expect("fixture should parse");
    schema_smuggled["plugin_descriptors"][0]["record_schemas"][0]["holder_policy"] =
        Value::String("disallowed".to_owned());
    assert_invalid_snapshot(
        &serde_json::to_string(&schema_smuggled).expect("smuggled fixture should serialize"),
        &[&FixturePlugin],
    );

    for format_5_kind in [
        json!({"type": "person_move_ordered", "person": 1, "from": 1, "to": 2, "arrival_at": 1}),
        json!({"type": "person_arrived", "person": 1, "territory": 1}),
        json!({"type": "letter_delivered", "letter": 1, "carrier": 1, "recipient": 2, "territory": 1}),
        json!({"type": "knowledge_published", "holder": {"type": "person", "id": 1}, "record_count": 1}),
    ] {
        let mut event_smuggled: Value = serde_json::from_str(PLUGIN).expect("fixture should parse");
        event_smuggled["events"][0]["kind"] = format_5_kind;
        assert_invalid_snapshot(
            &serde_json::to_string(&event_smuggled)
                .expect("format-5 event smuggling fixture should serialize"),
            &[&FixturePlugin],
        );
    }
}
