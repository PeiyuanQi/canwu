use super::*;

#[test]
fn knowledge_publication_is_phase_scoped_atomic_persisted_and_replayable() {
    let (scenario, _) = demo_scenario();
    let plugin = KnowledgePublicationPlugin;
    let mut simulation = Simulation::new(53, scenario.clone())
        .expect("knowledge publication fixture should initialize");
    simulation
        .register_plugin(&plugin)
        .expect("knowledge publication plugin should register");
    let receipt = simulation
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily))
        .expect("phase-scoped publications should settle");
    assert_eq!(receipt.knowledge_batch_count, 3);
    assert_eq!(receipt.knowledge_record_count, 3);
    assert_eq!(receipt.emitted_events.len(), 3);
    let holder = KnowledgeHolderRef::Person(PersonId::new(1));
    assert_eq!(
        simulation
            .knowledge()
            .for_holder(&holder)
            .map_or(0, HolderLedger::len),
        3
    );
    assert_eq!(simulation.boundaries()[0].knowledge_changes.len(), 3);

    let snapshot = simulation.snapshot();
    let restored = Simulation::from_snapshot_with_plugins(snapshot.clone(), &[&plugin])
        .expect("published knowledge should restore with exact schemas");
    assert_eq!(restored.snapshot(), snapshot);

    let replayed = Simulation::replay_from_journal_with_scenario(
        scenario,
        &[&plugin],
        &simulation.replay_journal(),
    )
    .expect("published knowledge should replay exactly");
    assert_eq!(replayed.snapshot(), snapshot);

    let mut missing_record = snapshot;
    missing_record
        .knowledge
        .records
        .remove_body(KnowledgeRecordId::new(2))
        .expect("the record exists");
    rehash_tampered_snapshot(&mut missing_record);
    let error = Simulation::from_snapshot_with_plugins(missing_record, &[&plugin])
        .err()
        .expect("boundary evidence must reconstruct the exact holder ledger");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
}

#[test]
fn phase13_publication_resolves_exact_current_boundary_record_evidence() {
    let (scenario, _) = demo_scenario();
    let plugin = PendingEvidencePublicationPlugin;
    let mut simulation =
        Simulation::new(59, scenario).expect("pending evidence fixture should initialize");
    simulation
        .register_plugin(&plugin)
        .expect("pending evidence plugin should register");
    simulation
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily))
        .expect("phase 13 should resolve the exact phase-7 record version");

    let snapshot = simulation.snapshot();
    Simulation::from_snapshot_with_plugins(snapshot.clone(), &[&plugin])
        .expect("exact current-boundary evidence should restore");

    let mut wrong_index = snapshot;
    let retarget = |body: &mut KnowledgeRecordBody| {
        let EvidenceRef::DomainRecordVersion(version) = &mut body.origin.evidence[0] else {
            panic!("fixture origin should contain a record version");
        };
        version.established_by = DomainRecordVersionSource::BoundaryChange {
            boundary: BoundaryId::new(1),
            change_index: 1,
        };
    };
    let id = wrong_index.boundaries[0].knowledge_changes[0].records[0].id;
    assert!(tamper_knowledge_body(&mut wrong_index, id, retarget));
    retarget(std::sync::Arc::make_mut(
        &mut wrong_index.boundaries[0].knowledge_changes[0].records[0],
    ));
    rehash_tampered_snapshot(&mut wrong_index);
    let error = Simulation::from_snapshot_with_plugins(wrong_index, &[&plugin])
        .err()
        .expect("wrong record-change evidence index must fail");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
}

#[test]
fn knowledge_snapshot_tamper_matrix_covers_ledger_evidence_and_batch_metadata() {
    let (scenario, _) = demo_scenario();
    let plugin = KnowledgePublicationPlugin;
    let mut simulation =
        Simulation::new(61, scenario).expect("knowledge tamper fixture should initialize");
    simulation
        .register_plugin(&plugin)
        .expect("knowledge publication plugin should register");
    simulation
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily))
        .expect("knowledge tamper fixture should publish");
    let snapshot = simulation.snapshot();
    let holder = KnowledgeHolderRef::Person(PersonId::new(1));
    let reject = |label: &str, mut tampered: SimulationSnapshot| {
        if let Err(error) = try_rehash_tampered_snapshot(&mut tampered) {
            assert_eq!(error.code, ErrorCode::InvalidSnapshot, "{label}");
            return;
        }
        let error = Simulation::from_snapshot_with_plugins(tampered, &[&plugin])
            .err()
            .unwrap_or_else(|| panic!("{label} tamper unexpectedly loaded"));
        assert_eq!(error.code, ErrorCode::InvalidSnapshot, "{label}");
    };

    let first = KnowledgeRecordId::new(1);
    // A body edit the wire refuses cannot be loaded; one it accepts must be
    // refused by validation.
    let mutate_first_record =
        |snapshot: &mut SimulationSnapshot, mutate: &dyn Fn(&mut KnowledgeRecordBody)| {
            tamper_knowledge_body(snapshot, first, mutate)
        };

    // The first entry moved to another holder.
    let mut record_holder = snapshot.clone();
    assert!(tamper_knowledge_ledger(&mut record_holder, |wire| {
        let holders = wire["holders"].as_array_mut().expect("holders");
        let entry = holders[0]["entries"]
            .as_array_mut()
            .expect("entries")
            .remove(0);
        holders.push(serde_json::json!({
            "holder": KnowledgeHolderRef::Person(PersonId::new(2)),
            "next_local": 2,
            "entries": [entry],
        }));
    }));
    reject("record holder", record_holder);

    let mut schema_version = snapshot.clone();
    mutate_first_record(&mut schema_version, &|record| record.schema.version += 1);
    reject("schema version", schema_version);

    let mut schema_hash = snapshot.clone();
    schema_hash.plugin_descriptors[0].knowledge_schemas[0]
        .schema_hash
        .replace_range(..1, "f");
    reject("schema semantic hash", schema_hash);

    let mut payload = snapshot.clone();
    mutate_first_record(&mut payload, &|record| {
        record.payload = serde_json::json!({ "value": 999 });
    });
    reject("record payload", payload);

    let mut subject = snapshot.clone();
    mutate_first_record(&mut subject, &|record| {
        record.subjects.push(KnowledgeSubject {
            role: "unexpected".to_owned(),
            target: KnowledgeSubjectTarget::Event(EventId::new(1)),
        });
    });
    reject("record subject", subject);

    let mut confidence = snapshot.clone();
    mutate_first_record(&mut confidence, &|record| {
        record.confidence_per_mille = 1_001;
    });
    reject("record confidence", confidence);

    let mut learned_at = snapshot.clone();
    mutate_first_record(&mut learned_at, &|record| {
        record.learned_at = SimTime::from_minutes(1);
    });
    reject("record learned time", learned_at);

    let mut forward_relation = snapshot.clone();
    assert!(tamper_knowledge_entry(
        &mut forward_relation,
        &holder,
        first,
        |entry| entry.supersedes = vec![KnowledgeRecordId::new(3)],
    ));
    reject("forward relation", forward_relation);

    // The fixture publishes record 1 in phase 4 for the same boundary, record
    // 2 in phase 4 for the next boundary, and record 3 in phase 13. Each
    // relation below is written to the ledger and to its boundary evidence,
    // and every commitment over them is recomputed, so only the relation rule
    // can refuse the snapshot: a load admits exactly the targets that were
    // visible at the publisher's cut.
    let relate = |record: u64, target: u64| {
        let mut tampered = snapshot.clone();
        let record = KnowledgeRecordId::new(record);
        let target = vec![KnowledgeRecordId::new(target)];
        assert!(tamper_knowledge_entry(
            &mut tampered,
            &holder,
            record,
            |entry| entry.supersedes.clone_from(&target),
        ));
        published_entry_mut(&mut tampered.boundaries[0], record).supersedes = target;
        refresh_snapshot_commitments_and_checkpoint(&mut tampered);
        tampered.boundaries[0].state_hash = Some(
            snapshot_boundary_head_state_hash(&tampered)
                .expect("tampered boundary head should have a state commitment"),
        );
        rehash_tampered_snapshot(&mut tampered);
        Simulation::from_snapshot_with_plugins(tampered, &[&plugin])
    };
    relate(3, 1).expect("phase 13 may relate to a same-boundary phase-4 record");
    for (label, record, target) in [
        ("a record deferred to the next boundary", 3, 2),
        ("a record of the publisher's own phase", 2, 1),
        ("a later record", 1, 3),
    ] {
        let error = relate(record, target)
            .err()
            .unwrap_or_else(|| panic!("a relation to {label} unexpectedly loaded"));
        assert_eq!(error.code, ErrorCode::InvalidSnapshot, "{label}");
        assert!(
            error.message.contains("publisher's cut"),
            "{label}: {}",
            error.message
        );
    }

    let mut evidence = snapshot.clone();
    mutate_first_record(&mut evidence, &|record| {
        record.origin.evidence = vec![EvidenceRef::Event(EventId::new(999))];
    });
    reject("origin evidence", evidence);

    let mut producer = snapshot.clone();
    producer.boundaries[0].knowledge_changes[0].plugin = "foreign-plugin".to_owned();
    reject("batch producer", producer);

    let mut phase = snapshot.clone();
    phase.boundaries[0].knowledge_changes[0].phase = BoundaryPhase::DomainDeltaProposal;
    reject("batch phase", phase);

    let mut visibility = snapshot.clone();
    visibility.boundaries[0]
        .knowledge_changes
        .iter_mut()
        .find(|change| change.phase == BoundaryPhase::PerspectiveAndReportMaterialization)
        .expect("fixture phase-13 publication should exist")
        .visibility = StateVisibility::NextBoundary;
    reject("batch visibility", visibility);

    let mut batch_order = snapshot.clone();
    batch_order.boundaries[0].knowledge_changes.swap(0, 1);
    reject("batch order", batch_order);

    let mut first_id = snapshot.clone();
    std::sync::Arc::make_mut(&mut first_id.boundaries[0].knowledge_changes[0].records[0]).id =
        KnowledgeRecordId::new(99);
    reject("batch first record ID", first_id);

    let mut local_id = snapshot.clone();
    published_entry_mut(&mut local_id.boundaries[0], first).local = HolderKnowledgeRecordId::new(7);
    reject("published local ID", local_id);

    let mut event_holder = snapshot.clone();
    let event = event_holder
        .events
        .iter_mut()
        .find(|event| event.kind.is_type("knowledge_published"))
        .expect("fixture publication event should exist");
    event
        .kind
        .set_field(
            "holders",
            &vec![KnowledgeHolderRef::Person(PersonId::new(2))],
        )
        .expect("publication event should have holders");
    reject("event holder", event_holder);

    let mut emission_index = snapshot.clone();
    let emission = emission_index.boundaries[0]
        .emissions
        .iter_mut()
        .find(|emission| matches!(emission.kind, BoundaryEmissionKind::KnowledgeChange { .. }))
        .expect("fixture knowledge emission should exist");
    emission.kind = BoundaryEmissionKind::KnowledgeChange { change_index: 99 };
    reject("knowledge emission index", emission_index);

    let mut counter_backward = snapshot.clone();
    counter_backward.next_knowledge_record_id = 3;
    reject("next record counter backward", counter_backward);

    let mut counter_forward = snapshot;
    counter_forward.next_knowledge_record_id = 99;
    reject("next record counter forward", counter_forward);
}

fn cross_holder_kind() -> KnowledgeRecordKind {
    KnowledgeRecordKind::new("fixture.cross_holder", "notice")
}

fn cross_holder_schema() -> KnowledgeSchemaId {
    KnowledgeSchemaId::new(cross_holder_kind(), 1)
}

/// Boundary 1 publishes to person 1 and boundary 2 to person 2. Every later
/// boundary publishes to person 2 a record that supersedes person 1's current
/// record, a target in another holder's committed ledger.
fn publish_across_holders(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let first = KnowledgeHolderRef::Person(PersonId::new(1));
    let second = KnowledgeHolderRef::Person(PersonId::new(2));
    let (holder, supersedes) = match context.boundary_id.get() {
        1 => (first, Vec::new()),
        2 => (second, Vec::new()),
        _ => (
            second,
            view.knowledge_heads(&first, &cross_holder_kind(), &[])?
                .records,
        ),
    };
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::PublishKnowledge {
            visibility: StateVisibility::SameBoundary,
            producer_correlation: None,
            supersession: crate::KnowledgeSupersession::Explicit,
            groups: vec![crate::KnowledgePublicationGroup {
                holders: vec![holder],
                records: vec![KnowledgeRecordDraft {
                    schema: cross_holder_schema(),
                    subjects: Vec::new(),
                    payload: serde_json::json!({ "boundary": context.boundary_id.get() }),
                    as_of: None,
                    confidence_per_mille: 1_000,
                    origin: KnowledgeOrigin {
                        method: "fixture".to_owned(),
                        evidence: Vec::new(),
                    },
                    supersedes,
                    contradicts: Vec::new(),
                }],
            }],
            summary: "Publish across holders".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

struct CrossHolderPlugin;

impl SimulationPlugin for CrossHolderPlugin {
    fn name(&self) -> &'static str {
        "fixture-knowledge-cross-holder"
    }

    test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000c18");

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_knowledge_schema(PluginKnowledgeSchema {
            id: cross_holder_schema(),
            schema_hash: "0000000000000000000000000000000000000000000000000000000000000c19"
                .to_owned(),
            writable: true,
            payload_schema: PayloadSchema::Any,
            subjects: Vec::new(),
            retention: crate::KnowledgeRetention::Keep,
        })?;
        let mut contract = BoundarySystemContract::new(
            "publish-across-holders",
            BoundaryPhase::PerceptionAndAttentionRefresh,
            SystemCadence::Daily,
        );
        contract.reads = vec![StateKey::core_knowledge()];
        contract.knowledge_writes = vec![KnowledgeWriteGrant {
            schema: cross_holder_schema(),
            visibilities: vec![StateVisibility::SameBoundary],
        }];
        registrar.register_boundary_system(contract, publish_across_holders)
    }
}

/// A record may relate only to records of its own holder: a target in
/// another holder's committed ledger is refused by settlement and by a load,
/// even though the target exists, is committed, and has the same kind.
#[test]
fn a_relation_cannot_name_another_holders_record() {
    let (scenario, _) = demo_scenario();
    let plugin = CrossHolderPlugin;
    let mut simulation =
        Simulation::new(73, scenario).expect("the cross-holder fixture should initialize");
    simulation
        .register_plugin(&plugin)
        .expect("the cross-holder plugin should register");
    let daily = |day: i64| {
        BoundaryRequest::at(SimTime::EPOCH + SimDuration::days(day))
            .with_cadence(SystemCadence::Daily)
    };
    for day in 0..2 {
        simulation
            .settle_boundary(daily(day))
            .expect("each holder's own publication should settle");
    }
    let snapshot = simulation.snapshot();

    let error = simulation
        .settle_boundary(daily(2))
        .expect_err("a record must not supersede another holder's record");
    assert_eq!(error.code, ErrorCode::KnowledgeRecordNotFound);
    assert_eq!(simulation.snapshot(), snapshot);

    // The same relation written into the boundary evidence, with every
    // commitment over it recomputed, is refused by a load: the ledger a load
    // rebuilds from evidence cannot resolve it in person 2's ledger. (A
    // stored ledger with the relation does not even decode.)
    let first = KnowledgeRecordId::new(1);
    let mut tampered = snapshot;
    assert!(
        tampered
            .knowledge
            .records
            .get(&KnowledgeHolderRef::Person(PersonId::new(2)))
            .is_some_and(|ledger| ledger.contains_key(&KnowledgeRecordId::new(2)))
    );
    assert!(
        tampered
            .knowledge
            .records
            .get(&KnowledgeHolderRef::Person(PersonId::new(1)))
            .is_some_and(|ledger| ledger.contains_key(&first))
    );
    assert!(!tamper_knowledge_entry(
        &mut tampered.clone(),
        &KnowledgeHolderRef::Person(PersonId::new(2)),
        KnowledgeRecordId::new(2),
        |entry| entry.supersedes = vec![first],
    ));
    tampered.boundaries[1].knowledge_changes[0].holders[0].entries[0].supersedes = vec![first];
    refresh_snapshot_commitments_and_checkpoint(&mut tampered);
    tampered.boundaries[1].state_hash = Some(
        snapshot_boundary_head_state_hash(&tampered)
            .expect("the tampered boundary head should have a state commitment"),
    );
    rehash_tampered_snapshot(&mut tampered);
    let error = Simulation::from_snapshot_with_plugins(tampered, &[&plugin])
        .err()
        .expect("a load must not accept a relation to another holder's record");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
}

#[test]
fn operation_keyed_randomness_is_order_independent_idempotent_and_replayable() {
    let (baseline, baseline_sequential, baseline_run, baseline_scenario) =
        run_keyed_fixture(&["alpha", "beta"]);
    let (reordered, reordered_sequential, reordered_run, reordered_scenario) =
        run_keyed_fixture(&["noise", "beta", "alpha"]);
    assert_eq!(baseline["alpha"], reordered["alpha"]);
    assert_eq!(baseline["beta"], reordered["beta"]);
    assert_eq!(baseline_sequential, reordered_sequential);
    assert_eq!(baseline_run.random_draws().len(), 3);
    assert_eq!(reordered_run.random_draws().len(), 4);
    assert_eq!(
        baseline_run
            .state
            .current
            .random_streams
            .get(&keyed_fixture_stream())
            .expect("keyed stream exists")
            .position,
        1
    );

    Simulation::from_snapshot_with_plugins(baseline_run.snapshot(), &[&KeyedRandomPlugin])
        .expect("operation-keyed draws should restore");
    let replayed = Simulation::replay_from_journal_with_scenario(
        baseline_scenario,
        &[&KeyedRandomPlugin],
        &baseline_run.replay_journal(),
    )
    .expect("operation-keyed draws should replay exactly");
    assert_eq!(replayed.snapshot(), baseline_run.snapshot());
    let replayed_reordered = Simulation::replay_from_journal_with_scenario(
        reordered_scenario,
        &[&KeyedRandomPlugin],
        &reordered_run.replay_journal(),
    )
    .expect("reordered operation-keyed draws should replay exactly");
    assert_eq!(replayed_reordered.snapshot(), reordered_run.snapshot());
}

#[test]
fn later_boundary_failure_rolls_back_staged_keyed_draws_and_indexes() {
    let (scenario, _) = demo_scenario();
    let plugin = KeyedRollbackPlugin;
    let mut simulation =
        Simulation::new(89, scenario).expect("keyed rollback fixture should initialize");
    simulation
        .register_plugin(&plugin)
        .expect("keyed rollback plugin should register");
    simulation
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            plugin.name(),
            "operation",
            SimTime::EPOCH,
            serde_json::json!({ "operation": "rollback-operation" }),
        ))
        .expect("rollback probe should enqueue");
    let before = simulation.snapshot();
    let error = simulation
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect_err("the undeclared later publication must reject the boundary");
    assert_eq!(error.code, ErrorCode::UndeclaredKnowledgeWrite);
    assert_eq!(simulation.snapshot(), before);
}

#[test]
fn operation_keyed_snapshot_tamper_matrix_is_recomputed_fail_closed() {
    let (_, _, simulation, _) = run_keyed_fixture(&["alpha", "beta"]);
    let snapshot = simulation.snapshot();
    let plugin = KeyedRandomPlugin;

    let reject = |label: &str, mut tampered: SimulationSnapshot| {
        rehash_tampered_snapshot(&mut tampered);
        let error = Simulation::from_snapshot_with_plugins(tampered, &[&plugin])
            .err()
            .unwrap_or_else(|| panic!("{label} tamper unexpectedly loaded"));
        assert_eq!(error.code, ErrorCode::InvalidSnapshot, "{label}");
    };
    let keyed_index = snapshot
        .random_draws
        .iter()
        .position(|draw| matches!(&draw.address, RandomDrawAddress::OperationV1(_)))
        .expect("fixture should contain a keyed draw");

    let mut address = snapshot.clone();
    let RandomDrawAddress::OperationV1(operation) = &mut address.random_draws[keyed_index].address
    else {
        unreachable!("selected draw is operation-keyed")
    };
    operation.application_operation_id.push_str("-tampered");
    reject("operation address", address);

    let mut evidence = snapshot.clone();
    evidence.random_draws[keyed_index].operation_evidence =
        Some(EvidenceRef::Ingress(IngressId::new(2)));
    reject("operation evidence", evidence);

    let mut bound = snapshot.clone();
    bound.random_draws[keyed_index].upper_exclusive += 1;
    reject("operation bound", bound);

    let mut purpose = snapshot.clone();
    purpose.random_draws[keyed_index]
        .purpose
        .push_str(" tampered");
    reject("operation purpose", purpose);

    let mut value = snapshot;
    value.random_draws[keyed_index].value = (value.random_draws[keyed_index].value + 1)
        % value.random_draws[keyed_index].upper_exclusive;
    reject("operation value", value);
}

#[test]
fn read_only_runs_reject_live_plugin_ingress_without_mutation() {
    let (scenario, _) = demo_scenario();
    let configuration = RunConfiguration::read_only_observer();
    let manifest = manifest_for_configuration(&scenario, &configuration);
    let plugin = CanonicalIngressPlugin;
    let mut simulation =
        Simulation::new_with_run_configuration(47, scenario, manifest, configuration)
            .expect("read-only ingress fixture should load");
    simulation
        .register_plugin(&plugin)
        .expect("read-only ingress plugin should register");
    let before = simulation.snapshot();
    let error = simulation
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            "canonical-ingress",
            "report",
            SimTime::EPOCH,
            serde_json::json!({ "label": "unauthorized live report" }),
        ))
        .expect_err("read-only runs cannot accept newly authored plugin ingress");
    assert_eq!(error.code, ErrorCode::InteractionReadOnly);
    assert_eq!(simulation.snapshot(), before);

    simulation
        .append_ingress(
            SimTime::EPOCH,
            IngressClass::Information,
            0,
            IngressPayload::Plugin {
                plugin: "canonical-ingress".to_owned(),
                packet_type: "report".to_owned(),
                payload: serde_json::json!({ "label": "forged live report" }),
                affected_entities: Vec::new(),
                archive_retention: Vec::new(),
            },
            None,
            false,
        )
        .expect("the fixture should construct coherent but unauthorized evidence");
    let error = Simulation::from_snapshot_with_plugins(simulation.snapshot(), &[&plugin])
        .err()
        .expect("snapshot validation must reject impossible read-only live ingress");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
}

/// Evidence citing an initial-scenario record version resolves through the
/// scenario's index of record positions, not a scan of the scenario: each
/// resolution reads exactly one scenario record, the one the index names,
/// in a scenario of 65,536 records as in one of 256 (the cited records are
/// the last a scan reaches). A wrong version reads its record and is
/// refused; version zero and an unknown record are refused without reading
/// any.
#[test]
fn initial_scenario_evidence_resolves_without_scanning_the_scenario() {
    use super::super::state::{INITIAL_RECORDS_READ, InitialRecordIndex};
    use super::super::validation::initial_domain_record_matches;

    let scenario_of = |records: usize| -> Scenario {
        let mut scenario = Scenario::new(SimTime::EPOCH, Vec::new());
        scenario.domain_records = (0..records)
            .map(|index| DomainRecord {
                reference: DomainRecordRef::new(
                    "fixture.initial",
                    "record",
                    format!("r{index:06}"),
                ),
                owner: "fixture".to_owned(),
                class: DomainRecordClass::Record,
                version: 1 + u64::try_from(index % 3).unwrap(),
                lifecycle: DomainRecordLifecycle::Active,
                payload: Value::Null,
                references: Vec::new(),
            })
            .collect();
        scenario
    };
    let version_ref = |record: &DomainRecord, version: u64| DomainRecordVersionRef {
        record: record.reference.clone(),
        version,
        established_by: DomainRecordVersionSource::InitialScenario,
    };
    for records in [256, 65_536] {
        let scenario = scenario_of(records);
        let index = InitialRecordIndex::of(Some(&scenario));
        let last = scenario.domain_records.last().expect("records");
        let reads = |reference: &DomainRecordVersionRef, matches: bool| -> usize {
            INITIAL_RECORDS_READ.set(0);
            assert_eq!(
                initial_domain_record_matches(&index, Some(&scenario), reference),
                matches,
                "{reference:?}"
            );
            INITIAL_RECORDS_READ.get()
        };
        for record in &scenario.domain_records[records - 64..] {
            assert_eq!(reads(&version_ref(record, record.version), true), 1);
        }
        assert_eq!(reads(&version_ref(last, last.version + 1), false), 1);
        assert_eq!(reads(&version_ref(last, 0), false), 0);
        let mut unknown = version_ref(last, last.version);
        unknown.record.id.push_str("-unknown");
        assert_eq!(reads(&unknown, false), 0);
    }
}
