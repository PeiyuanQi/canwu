use super::*;

#[test]
fn plugin_command_receives_issuer_and_namespaces_state() {
    let (mut simulation, ids) = Simulation::demo(35).expect("demo should load");
    simulation
        .register_plugin(&AuthorityPlugin)
        .expect("plugin should register");

    let before = simulation
        .snapshot_json()
        .expect("snapshot should serialize");
    let rejected = simulation.submit(CommandEnvelope::new(
        Issuer::Actor(ids.observer),
        Command::Plugin {
            plugin: "authority-test".to_owned(),
            command: "set_stance".to_owned(),
            payload: Value::Null,
        },
    ));
    assert_eq!(
        rejected.expect_err("wrong actor must be rejected").code,
        ErrorCode::InvalidAuthority
    );
    assert_eq!(
        before,
        simulation
            .snapshot_json()
            .expect("snapshot should serialize")
    );

    let invalid_payload = simulation.submit(CommandEnvelope::new(
        Issuer::Actor(ids.commander),
        Command::Plugin {
            plugin: "authority-test".to_owned(),
            command: "set_stance".to_owned(),
            payload: serde_json::json!({}),
        },
    ));
    assert_eq!(
        invalid_payload
            .expect_err("payloads must match their declared schema")
            .code,
        ErrorCode::InvalidPayload
    );
    assert_eq!(
        before,
        simulation
            .snapshot_json()
            .expect("payload rejection must not mutate the simulation")
    );

    simulation
        .submit(CommandEnvelope::new(
            Issuer::Actor(ids.commander),
            Command::Plugin {
                plugin: "authority-test".to_owned(),
                command: "set_stance".to_owned(),
                payload: Value::Null,
            },
        ))
        .expect("authorized actor should be accepted");
    let snapshot = simulation.snapshot();
    assert_eq!(snapshot.plugin_components.len(), 1);
    assert_eq!(snapshot.plugin_components[0].plugin, "authority-test");
    assert_eq!(
        snapshot.plugin_components[0].state,
        StateKey::new("military", "stance")
    );
    assert_eq!(snapshot.plugin_components[0].component, "stance");
    assert_eq!(
        simulation
            .register_plugin(&MarkerPlugin {
                name: "late-plugin",
                writes: Vec::new(),
            })
            .expect_err("new plugins cannot appear after execution begins")
            .code,
        ErrorCode::PluginRegistrationClosed
    );
}

#[test]
fn synchronous_reactor_depth_is_bounded_and_rolls_back() {
    let (mut simulation, ids) = Simulation::demo(35).expect("demo should load");
    simulation
        .register_plugin(&RecursivePlugin)
        .expect("recursive compatibility plugin should register");
    let before = simulation
        .snapshot_json()
        .expect("snapshot should serialize before the rejected cascade");

    let error = simulation
        .submit(move_order(&ids))
        .expect_err("recursive immediate reactions must be bounded");
    assert_eq!(error.code, ErrorCode::SynchronousReactionLimit);
    assert!(error.message.contains("maximum nested depth"));
    assert_eq!(
        before,
        simulation
            .snapshot_json()
            .expect("bounded cascade must roll back the entire transaction")
    );
}

#[test]
fn plugin_registration_is_atomic_and_rejects_duplicate_state_owners() {
    let (mut simulation, _) = Simulation::demo(35).expect("demo should load");
    simulation
        .register_plugin(&MarkerPlugin {
            name: "first-owner",
            writes: vec![StateKey::new("shared-domain", "balance")],
        })
        .expect("first owner should register");
    let before = simulation
        .snapshot_json()
        .expect("snapshot should serialize");
    let error = simulation
        .register_plugin(&MarkerPlugin {
            name: "second-owner",
            writes: vec![StateKey::new("shared-domain", "balance")],
        })
        .expect_err("a second owner must be rejected");
    assert_eq!(error.code, ErrorCode::DuplicateStateOwner);
    assert_eq!(
        before,
        simulation
            .snapshot_json()
            .expect("failed registration must not change state or manifests")
    );
    simulation
        .register_plugin(&GhostPlugin)
        .expect("a caught registrar error may not poison the candidate registry");
    simulation
        .register_plugin(&MarkerPlugin {
            name: "fresh-owner",
            writes: vec![StateKey::new("fresh-domain", "value")],
        })
        .expect("the failed multi-key claim must leave no ghost owner");
    simulation
        .register_plugin(&BoundaryGhostPlugin)
        .expect("a caught boundary registrar error may not poison the candidate registry");
    simulation
        .register_plugin(&MarkerPlugin {
            name: "boundary-ghost-owner",
            writes: vec![StateKey::new("boundary-ghost", "value")],
        })
        .expect("a later boundary-writer failure must leave no ghost owner");
}

#[test]
fn phased_boundary_allocates_deterministically_and_respects_visibility() {
    let (scenario, _) = demo_scenario();
    let mut first = Simulation::new(35, scenario.clone()).expect("demo should load");
    first
        .register_plugin(&JournalCommandPlugin)
        .expect("journal command plugin should register");
    first
        .register_plugin(&GrainSupplyPlugin)
        .expect("supply plugin should register");
    first
        .register_plugin(&HighClaimPlugin)
        .expect("high claim should register");
    first
        .register_plugin(&LowClaimPlugin)
        .expect("low claim should register");
    first
        .register_plugin(&VisibilityValidatorPlugin)
        .expect("validator should register");

    let mut second = Simulation::new(35, scenario.clone()).expect("demo should load");
    second
        .register_plugin(&VisibilityValidatorPlugin)
        .expect("validator should register");
    second
        .register_plugin(&LowClaimPlugin)
        .expect("low claim should register");
    second
        .register_plugin(&HighClaimPlugin)
        .expect("high claim should register");
    second
        .register_plugin(&GrainSupplyPlugin)
        .expect("supply plugin should register");
    second
        .register_plugin(&JournalCommandPlugin)
        .expect("journal command plugin should register");

    for simulation in [&mut first, &mut second] {
        for _ in 0..2 {
            simulation
                .submit(CommandEnvelope::new(
                    Issuer::Debug,
                    Command::Plugin {
                        plugin: "journal-command".to_owned(),
                        command: "noop".to_owned(),
                        payload: Value::Null,
                    },
                ))
                .expect("journal fixture command should be accepted");
        }
    }

    let request = BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily);
    let first_receipt = first
        .settle_boundary(request.clone())
        .expect("daily boundary should settle");
    let second_receipt = second
        .settle_boundary(request.clone())
        .expect("registration order must not change settlement");
    assert_eq!(first_receipt, second_receipt);
    assert_eq!(first.snapshot(), second.snapshot());
    let first_followup = first
        .settle_boundary(request.clone())
        .expect("a same-time follow-up boundary should settle");
    let second_followup = second
        .settle_boundary(request)
        .expect("the follow-up boundary must remain registration-order independent");
    assert_eq!(first_followup, second_followup);
    assert_eq!(first.snapshot(), second.snapshot());

    let allocations: BTreeMap<_, _> = first_receipt
        .allocations
        .iter()
        .map(|allocation| {
            (
                allocation.reservation.plugin.as_str(),
                (allocation.granted, allocation.disposition),
            )
        })
        .collect();
    assert_eq!(
        allocations.get("high-claim"),
        Some(&(7, ReservationDisposition::Fulfilled))
    );
    assert_eq!(
        allocations.get("low-claim"),
        Some(&(3, ReservationDisposition::Partial))
    );
    let components: BTreeMap<_, _> = first
        .snapshot()
        .plugin_components
        .into_iter()
        .map(|record| (record.component, record.value))
        .collect();
    assert_eq!(components.get("high").and_then(Value::as_u64), Some(7));
    assert_eq!(components.get("low").and_then(Value::as_u64), Some(3));

    let json = first
        .snapshot_json()
        .expect("settled boundary should serialize");
    let restored = Simulation::from_snapshot_json_with_plugins(
        &json,
        &[
            &GrainSupplyPlugin,
            &HighClaimPlugin,
            &LowClaimPlugin,
            &JournalCommandPlugin,
            &VisibilityValidatorPlugin,
        ],
    )
    .expect("settled boundary should rehydrate");
    assert_eq!(first.snapshot(), restored.snapshot());

    let plugins: &[&dyn SimulationPlugin] = &[
        &GrainSupplyPlugin,
        &HighClaimPlugin,
        &LowClaimPlugin,
        &JournalCommandPlugin,
        &VisibilityValidatorPlugin,
    ];
    let replayed = Simulation::replay_with_boundaries(
        35,
        scenario,
        plugins,
        first.command_log(),
        first.boundaries(),
        first.time(),
    )
    .expect("boundary journal should replay exactly");
    assert_eq!(first.snapshot(), replayed.snapshot());

    let mut corrupted_allocation = first.snapshot();
    corrupted_allocation.boundaries[0].allocations[0].granted += 1;
    let error = Simulation::from_snapshot_with_plugins(corrupted_allocation, plugins)
        .err()
        .expect("tampered allocation evidence must not load");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let mut corrupted_provenance = first.snapshot();
    corrupted_provenance.boundaries[0].emissions[0].system = "request".to_owned();
    let error = Simulation::from_snapshot_with_plugins(corrupted_provenance, plugins)
        .err()
        .expect("tampered boundary source provenance must not load");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let mut corrupted_command_cut = first.snapshot();
    corrupted_command_cut.boundaries[0].admitted_commands = vec![CommandId::new(2)];
    let error = Simulation::from_snapshot_with_plugins(corrupted_command_cut, plugins)
        .err()
        .expect("boundary admission must be a global command-journal prefix");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let mut corrupted_event_cut = first.snapshot();
    let later_event = corrupted_event_cut.boundaries[1].emissions[0].event;
    corrupted_event_cut.boundaries[0].admitted_events = vec![later_event];
    let error = Simulation::from_snapshot_with_plugins(corrupted_event_cut, plugins)
        .err()
        .expect("an earlier boundary cannot admit a later boundary event");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let mut corrupted_boundary_counter = first.snapshot();
    corrupted_boundary_counter.next_boundary_id += 1;
    let error = Simulation::from_snapshot_with_plugins(corrupted_boundary_counter, plugins)
        .err()
        .expect("the next boundary counter must not skip an identifier");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let (mut causal_cut, ids) = Simulation::demo(35).expect("demo should load");
    causal_cut
        .submit(move_order(&ids))
        .expect("movement should emit command-caused evidence");
    causal_cut
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("an evidence-only boundary should settle");
    assert!(causal_cut.boundaries()[0].emissions.is_empty());

    let mut omitted_same_time_event = causal_cut.snapshot();
    omitted_same_time_event.boundaries[0]
        .admitted_events
        .clear();
    let error = Simulation::from_snapshot(omitted_same_time_event)
        .err()
        .expect("a no-emission boundary cannot omit already caused same-time evidence");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let mut due_at_boundary = causal_cut.snapshot();
    due_at_boundary.scheduled[0].key.at = due_at_boundary.now;
    due_at_boundary.boundaries[0].state_hash = Some(
        snapshot_state_hash(&due_at_boundary)
            .expect("the structurally corrupted state should hash"),
    );
    due_at_boundary.boundaries[0].hash = compute_boundary_hash(&due_at_boundary.boundaries[0])
        .expect("the structurally corrupted boundary should hash");
    refresh_snapshot_commitments_and_checkpoint(&mut due_at_boundary);
    let error = Simulation::from_snapshot(due_at_boundary)
        .err()
        .expect("completed boundaries cannot retain due ingress");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    assert!(error.message.contains("future-dated"));
}

#[test]
fn domain_record_lifecycle_is_atomic_replayable_and_tamper_evident() {
    let (scenario, _) = demo_scenario();
    let mut record_free = Simulation::new(87, scenario.clone())
        .expect("record-free compatibility fixture should load");
    let record_free_snapshot = record_free.snapshot();
    let mut record_free_restored = Simulation::from_snapshot(record_free_snapshot)
        .expect("a pristine Format 6 snapshot should restore");
    record_free
        .register_plugin(&RecordLifecyclePlugin)
        .expect("the original pristine runtime should accept record schemas");
    record_free_restored
        .register_plugin(&RecordLifecyclePlugin)
        .expect("a restored pristine runtime must retain record-schema capability");
    assert_eq!(record_free.snapshot(), record_free_restored.snapshot());
    let mut initial_scenario = scenario.clone();
    initial_scenario.domain_records = vec![
        initial_record(
            "fixture-record-lifecycle",
            DomainRecordClass::Entity,
            office_draft("office-a", "Primary Office"),
        ),
        initial_record(
            "fixture-record-lifecycle",
            DomainRecordClass::Entity,
            office_draft("office-b", "Successor Office"),
        ),
        initial_record(
            "fixture-record-lifecycle",
            DomainRecordClass::Record,
            obligation_draft("office-a", "open"),
        ),
    ];
    let error = Simulation::new(88, initial_scenario.clone())
        .err()
        .expect("initial domain records must not create a half-configured runtime");
    assert_eq!(error.code, ErrorCode::PluginNotActive);
    let initial = Simulation::new_with_plugins(88, initial_scenario, &[&RecordLifecyclePlugin])
        .expect("plugin-aware construction should validate initial domain records");
    let initial_json = initial
        .snapshot_json()
        .expect("configured initial record state should serialize");
    let initial_restored =
        Simulation::from_snapshot_json_with_plugins(&initial_json, &[&RecordLifecyclePlugin])
            .expect("configured initial record state should reload immediately");
    assert_eq!(initial.snapshot(), initial_restored.snapshot());

    let mut simulation = Simulation::new(89, scenario.clone()).expect("record fixture should load");
    simulation
        .register_plugin(&RecordLifecyclePlugin)
        .expect("record lifecycle plugin should register");
    let request = BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily);
    let created = simulation
        .settle_boundary(request.clone())
        .expect("record creation boundary should settle");
    let retired = simulation
        .settle_boundary(request.clone())
        .expect("record retirement boundary should settle");
    let succession = simulation
        .settle_boundary(request.clone())
        .expect("a later successor should retire without invalidating its predecessor");
    let deleted = simulation
        .settle_boundary(request)
        .expect("atomic reference transfer and deletion should settle");
    assert_eq!(created.record_change_count, 3);
    assert_eq!(retired.record_change_count, 2);
    assert_eq!(succession.record_change_count, 1);
    assert_eq!(deleted.record_change_count, 2);
    assert_eq!(
        created.change_count
            + retired.change_count
            + succession.change_count
            + deleted.change_count,
        1
    );

    let original = simulation
        .domain_record(&office_reference("office-a"))
        .expect("deleted office tombstone should remain addressable");
    assert!(original.is_deleted());
    assert_eq!(original.version, 3);
    let obligation = simulation
        .domain_record(&obligation_reference())
        .expect("transferred obligation should remain present");
    assert_eq!(obligation.version, 2);
    assert!(obligation.references.iter().any(|reference| {
        reference.target == DomainReferenceTarget::Domain(office_reference("office-c"))
    }));
    assert!(matches!(
        &simulation
            .domain_record(&office_reference("office-b"))
            .expect("the intermediate successor should remain addressable")
            .lifecycle,
        DomainRecordLifecycle::Retired {
            successor: Some(successor),
            ..
        } if successor == &office_reference("office-c")
    ));
    assert!(simulation.boundaries().iter().all(|boundary| {
        boundary.record_changes.len()
            == boundary
                .emissions
                .iter()
                .filter(|emission| {
                    matches!(emission.kind, BoundaryEmissionKind::RecordChange { .. })
                })
                .count()
    }));

    let before_stale_update = simulation
        .snapshot_json()
        .expect("pre-conflict record state should serialize");
    let conflict = simulation
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily))
        .expect_err("stale record versions must reject before commit");
    assert_eq!(conflict.code, ErrorCode::DomainRecordVersionConflict);
    assert_eq!(
        before_stale_update,
        simulation
            .snapshot_json()
            .expect("version conflicts must roll back the complete boundary")
    );
    let quiet = simulation
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Monthly))
        .expect("an unrelated cadence should publish an empty later boundary");
    assert_eq!(quiet.change_count + quiet.record_change_count, 0);

    let json = simulation
        .snapshot_json()
        .expect("domain-record snapshot should serialize");
    let restored = Simulation::from_snapshot_json_with_plugins(&json, &[&RecordLifecyclePlugin])
        .expect("domain-record evidence should restore with exact plugin code");
    assert_eq!(simulation.snapshot(), restored.snapshot());

    let plugins: &[&dyn SimulationPlugin] = &[&RecordLifecyclePlugin];
    let replayed = Simulation::replay_with_boundaries(
        89,
        scenario,
        plugins,
        simulation.command_log(),
        simulation.boundaries(),
        simulation.time(),
    )
    .expect("domain-record boundary evidence should replay exactly");
    assert_eq!(simulation.snapshot(), replayed.snapshot());

    let mut cross_system_creation = simulation.snapshot();
    let observer_event = cross_system_creation.boundaries[0]
        .emissions
        .iter()
        .find_map(|emission| {
            (emission.system == "observer"
                && matches!(emission.kind, BoundaryEmissionKind::Explicit))
            .then_some(emission.event)
        })
        .expect("the independent observer should emit boundary evidence");
    cross_system_creation
        .events
        .iter_mut()
        .find(|event| event.id == observer_event)
        .expect("the observer event should exist")
        .affected_entities = vec![EntityRef::Domain(office_reference("office-b"))];
    let final_state_hash = snapshot_state_hash(&cross_system_creation)
        .expect("the cross-system creation forgery should have coherent final state");
    cross_system_creation
        .boundaries
        .last_mut()
        .expect("the fixture should have a boundary head")
        .state_hash = Some(final_state_hash);
    rehash_tampered_snapshot(&mut cross_system_creation);
    let error = Simulation::from_snapshot_with_plugins(cross_system_creation, plugins)
        .err()
        .expect("one proposal cannot consume another system's same-stage creation");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let mut precreation_reference = simulation.snapshot();
    let marker_change = precreation_reference.boundaries[0]
        .changes
        .first_mut()
        .expect("the first boundary should persist its marker change");
    marker_change.entity = EntityRef::Domain(office_reference("office-c"));
    let marker_event = precreation_reference.boundaries[0]
        .emissions
        .iter()
        .find_map(|emission| {
            matches!(
                emission.kind,
                BoundaryEmissionKind::Change { change_index: 0 }
            )
            .then_some(emission.event)
        })
        .expect("the marker change should have causal event evidence");
    precreation_reference
        .events
        .iter_mut()
        .find(|event| event.id == marker_event)
        .expect("the marker change event should exist")
        .affected_entities = vec![EntityRef::Domain(office_reference("office-c"))];
    precreation_reference
        .plugin_components
        .iter_mut()
        .find(|record| record.component == "status")
        .expect("the persisted marker component should exist")
        .entity = EntityRef::Domain(office_reference("office-c"));
    precreation_reference
        .plugin_components
        .sort_by_key(|record| {
            component_key(
                &record.plugin,
                &record.state,
                &record.entity,
                &record.component,
            )
        });
    let final_state_hash = snapshot_state_hash(&precreation_reference)
        .expect("the pre-creation forgery should have coherent final state");
    precreation_reference
        .boundaries
        .last_mut()
        .expect("the fixture should have a boundary head")
        .state_hash = Some(final_state_hash);
    rehash_tampered_snapshot(&mut precreation_reference);
    let error = Simulation::from_snapshot_with_plugins(precreation_reference, plugins)
        .err()
        .expect("earlier evidence cannot reference an entity created by a later boundary");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let mut post_deletion_reference = simulation.snapshot();
    let (last_boundary_id, last_boundary_at, last_boundary_correlation) = {
        let last_boundary = post_deletion_reference
            .boundaries
            .last_mut()
            .expect("the fixture should have an empty later boundary");
        last_boundary.cadences = vec![SystemCadence::Daily];
        (
            last_boundary.id,
            last_boundary.at,
            last_boundary.correlation_id,
        )
    };
    let event_id = EventId::new(post_deletion_reference.next_event_id);
    post_deletion_reference.next_event_id = post_deletion_reference
        .next_event_id
        .checked_add(1)
        .expect("the tamper fixture should have event ID capacity");
    post_deletion_reference.events.push(SimEvent {
        id: event_id,
        timestamp: last_boundary_at,
        kind: EventKind::plugin("fixture-record-lifecycle", "record_probe"),
        affected_entities: vec![EntityRef::Domain(office_reference("office-a"))],
        summary: "Forge evidence after the office was deleted".to_owned(),
        cause: Some(CauseRef::Boundary(last_boundary_id)),
        correlation_id: last_boundary_correlation,
    });
    post_deletion_reference
        .boundaries
        .last_mut()
        .expect("the fixture should retain its boundary head")
        .emissions
        .push(BoundaryEmission {
            plugin: "fixture-record-lifecycle".to_owned(),
            system: "lifecycle".to_owned(),
            event: event_id,
            kind: BoundaryEmissionKind::Explicit,
        });
    let final_state_hash = snapshot_state_hash(&post_deletion_reference)
        .expect("the post-deletion forgery should have coherent final state");
    post_deletion_reference
        .boundaries
        .last_mut()
        .expect("the fixture should retain its boundary head")
        .state_hash = Some(final_state_hash);
    rehash_tampered_snapshot(&mut post_deletion_reference);
    let error = Simulation::from_snapshot_with_plugins(post_deletion_reference, plugins)
        .err()
        .expect("later evidence cannot reference a deleted domain entity");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let mut corrupted = simulation.snapshot();
    corrupted.boundaries[1].record_changes[0].system = "forged-system".to_owned();
    rehash_tampered_snapshot(&mut corrupted);
    let error = Simulation::from_snapshot_with_plugins(corrupted, plugins)
        .err()
        .expect("forged domain-record provenance must not load");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let mut shifted_to_genesis = simulation.snapshot();
    let forged_initial_records = shifted_to_genesis.domain_records.clone();
    shifted_to_genesis
        .initial_scenario
        .as_mut()
        .expect("new snapshots retain their manifest-bound initial scenario")
        .domain_records
        .clone_from(&forged_initial_records);
    shifted_to_genesis.boundaries.clear();
    shifted_to_genesis.events.clear();
    shifted_to_genesis.plugin_registration_closed = false;
    shifted_to_genesis.next_event_id = 1;
    shifted_to_genesis.next_boundary_id = 1;
    shifted_to_genesis.next_correlation_id = 1;
    refresh_snapshot_commitments_and_checkpoint(&mut shifted_to_genesis);
    let error = Simulation::from_snapshot_with_plugins(shifted_to_genesis, plugins)
        .err()
        .expect("record creations cannot be relabeled as manifest-bound genesis state");
    assert_eq!(error.code, ErrorCode::InvalidRunManifest);

    let mut stripped_feature = simulation.snapshot();
    stripped_feature.initial_scenario = None;
    stripped_feature.domain_records.clear();
    stripped_feature.boundaries.clear();
    stripped_feature.events.clear();
    stripped_feature.plugin_registration_closed = false;
    stripped_feature.next_event_id = 1;
    stripped_feature.next_boundary_id = 1;
    stripped_feature.next_correlation_id = 1;
    refresh_snapshot_commitments_and_checkpoint(&mut stripped_feature);
    let error = Simulation::from_snapshot_with_plugins(stripped_feature, plugins)
        .err()
        .expect("record schemas cannot downgrade to an unbound old-v4 snapshot shape");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
}

#[test]
fn domain_record_delete_rejects_live_references_and_rolls_back() {
    let (scenario, _) = demo_scenario();
    let mut simulation = Simulation::new(90, scenario).expect("record fixture should load");
    simulation
        .register_plugin(&RecordDeleteOnlyPlugin)
        .expect("invalid-delete fixture should register");
    let request = BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily);
    simulation
        .settle_boundary(request.clone())
        .expect("record creation should settle");
    simulation
        .settle_boundary(request.clone())
        .expect("record retirement should settle");
    let before = simulation
        .snapshot_json()
        .expect("pre-failure state should serialize");
    let error = simulation
        .settle_boundary(request)
        .expect_err("a referenced record cannot be deleted");
    assert_eq!(error.code, ErrorCode::DomainRecordReferenced);
    assert_eq!(
        before,
        simulation
            .snapshot_json()
            .expect("failed deletion must restore every persisted field")
    );
}

#[test]
fn domain_record_successor_cycles_are_rejected_in_genesis_and_atomic_bundles() {
    let (scenario, _) = demo_scenario();
    let mut cyclic_genesis = scenario.clone();
    let mut first = initial_record(
        "fixture-record-cycle",
        DomainRecordClass::Entity,
        office_draft("office-a", "First Office"),
    );
    first.lifecycle = DomainRecordLifecycle::Retired {
        at: SimTime::EPOCH,
        successor: Some(office_reference("office-b")),
    };
    let mut second = initial_record(
        "fixture-record-cycle",
        DomainRecordClass::Entity,
        office_draft("office-b", "Second Office"),
    );
    second.lifecycle = DomainRecordLifecycle::Retired {
        at: SimTime::EPOCH,
        successor: Some(office_reference("office-a")),
    };
    cyclic_genesis.domain_records = vec![first, second];
    let error = Simulation::new_with_plugins(91, cyclic_genesis, &[&RecordCyclePlugin])
        .err()
        .expect("cyclic successor state must not enter a new run");
    assert_eq!(error.code, ErrorCode::InvalidDomainRecord);

    let mut simulation = Simulation::new(92, scenario).expect("cycle fixture should load");
    simulation
        .register_plugin(&RecordCyclePlugin)
        .expect("cycle fixture plugin should register");
    let request = BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily);
    simulation
        .settle_boundary(request.clone())
        .expect("cycle fixture records should be created");
    let before = simulation
        .snapshot_json()
        .expect("pre-cycle state should serialize");
    let error = simulation
        .settle_boundary(request)
        .expect_err("mutual successor retirement must reject atomically");
    assert_eq!(error.code, ErrorCode::InvalidDomainRecord);
    assert_eq!(
        before,
        simulation
            .snapshot_json()
            .expect("failed successor cycles must roll back the whole boundary")
    );
}

#[test]
fn domain_record_snapshot_cannot_delete_the_bound_seat_institution() {
    let (scenario, _) = demo_scenario();
    let mut initial_scenario = scenario;
    initial_scenario.domain_records = vec![initial_record(
        "fixture-record-seat-deletion",
        DomainRecordClass::Entity,
        office_draft("office-a", "Bound Office"),
    )];
    let mut simulation =
        Simulation::new_with_plugins(93, initial_scenario.clone(), &[&RecordSeatDeletionPlugin])
            .expect("unbound seat-deletion fixture should load");
    let request = BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily);
    simulation
        .settle_boundary(request.clone())
        .expect("the fixture office should retire");
    simulation
        .settle_boundary(request)
        .expect("an unbound retired office may be deleted");

    let configuration = RunConfiguration {
        format_version: RUN_CONFIGURATION_FORMAT_VERSION,
        purpose: RunPurpose::Play,
        controller: ControllerPolicy::HumanRoleBound,
        seat: SeatPolicy::InstitutionBound,
        observation: ObservationPolicy::ActorBound,
        interaction: InteractionPolicy::EraInternalCommands,
        trace: TracePolicy::Causal,
        seat_binding: Some(SeatBinding {
            seat_id: "seat.bound-office".to_owned(),
            controller_id: "controller.human".to_owned(),
            actor: None,
            institution: Some(EntityRef::Domain(office_reference("office-a"))),
            permission_profile_id: "permission.institution".to_owned(),
        }),
        declared_interventions: Vec::new(),
        diagnostic_commands_enabled: false,
        require_idempotency_keys: true,
    };
    let mut forged = simulation.snapshot();
    let run_manifest = manifest_for_configuration(&initial_scenario, &configuration);
    forged.run_manifest_hash =
        manifest::hash(&run_manifest).expect("forged manifest should hash canonically");
    forged.run_manifest = Some(run_manifest);
    forged.run_configuration = Some(RunConfigurationSnapshot::Declared(configuration));
    let (_, authority_manifest_hash) = authoritative_run_identity(
        forged.run_manifest.as_ref().expect("forged manifest"),
        &forged.run_manifest_hash,
        forged
            .run_configuration
            .as_ref()
            .expect("forged configuration"),
    )
    .expect("forged authority identity should hash");
    forged.authority_root_seed =
        fresh_authority_root_seed(forged.root_seed, &authority_manifest_hash)
            .expect("forged authority root should derive");
    let final_state_hash =
        snapshot_state_hash(&forged).expect("the forged institution-bound final state should hash");
    forged
        .boundaries
        .last_mut()
        .expect("the forged fixture should have a boundary head")
        .state_hash = Some(final_state_hash);
    rehash_tampered_snapshot(&mut forged);
    let error = Simulation::from_snapshot_with_plugins(forged, &[&RecordSeatDeletionPlugin])
        .err()
        .expect("a snapshot cannot delete the institution bound to its active seat");
    assert_eq!(error.code, ErrorCode::InvalidRunConfiguration);
}

#[test]
fn failed_phased_boundary_restores_every_writable_domain_and_retries_exactly() {
    let (scenario, ids) = demo_scenario();
    let record_plugin = RecordLifecyclePlugin;
    let rollback_plugin = BoundaryRollbackPlugin;
    let random_plugin = PrimaryRandomPlugin;
    let mut simulation = Simulation::new(35, scenario).expect("rollback fixture should load");
    simulation
        .register_plugin(&record_plugin)
        .expect("record fixture should register");
    simulation
        .register_plugin(&rollback_plugin)
        .expect("rollback fixture should register");
    simulation
        .register_plugin(&random_plugin)
        .expect("random fixture should register");
    simulation
        .enqueue_command(
            SimTime::EPOCH,
            0,
            CommandRequest::new(
                CommandRequestId::new(1),
                simulation.revision(),
                move_order(&ids),
            ),
        )
        .expect("the initial movement should queue");
    simulation
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily))
        .expect("the initial boundary should create records and schedule the arrival");
    let arrival_at = SimTime::EPOCH
        .checked_add(SimDuration::hours(18))
        .expect("arrival time should be representable");
    let return_order = CommandEnvelope::new(
        Issuer::Actor(ids.commander),
        Command::OrderMovement {
            subject: EntityRef::Army(ids.army),
            destination: ids.western_territory,
            cargo: Vec::new(),
        },
    )
    .at_time(arrival_at);
    simulation
        .enqueue_command(
            arrival_at,
            0,
            CommandRequest::new(
                CommandRequestId::new(2),
                simulation.revision(),
                return_order,
            ),
        )
        .expect("the equal-time return order should queue");

    let baseline = simulation.snapshot();
    let mut control = Simulation::from_snapshot_with_plugins(
        baseline.clone(),
        &[&record_plugin, &rollback_plugin, &random_plugin],
    )
    .expect("the pending rollback fixture should reload exactly");
    let next_random_draw_id = simulation.state.counters.next_random_draw_id;
    simulation.state.counters.next_random_draw_id = u64::MAX - 1;
    let cache_before = cache_fingerprint(&simulation);
    let before = simulation
        .snapshot_json()
        .expect("snapshot should serialize");
    let error = simulation
        .settle_boundary(BoundaryRequest::at(arrival_at).with_cadence(SystemCadence::Daily))
        .expect_err("random-draw identifier exhaustion must abort the whole boundary");
    assert_eq!(error.code, ErrorCode::IdentifierExhausted);
    assert_eq!(
        before,
        simulation
            .snapshot_json()
            .expect("failed settlement must restore every serialized field")
    );
    assert_eq!(cache_fingerprint(&simulation), cache_before);

    simulation.state.counters.next_random_draw_id = next_random_draw_id;
    assert_eq!(simulation.snapshot(), baseline);
    let retry = simulation
        .settle_boundary(BoundaryRequest::at(arrival_at).with_cadence(SystemCadence::Daily))
        .expect("the repaired boundary should settle");
    let control_receipt = control
        .settle_boundary(BoundaryRequest::at(arrival_at).with_cadence(SystemCadence::Daily))
        .expect("the control boundary should settle");
    assert_eq!(retry, control_receipt);
    assert_eq!(simulation.snapshot(), control.snapshot());
    assert!(retry.change_count > 0);
    assert!(retry.record_change_count > 0);
    assert!(!retry.generated_ingress.is_empty());
    assert!(!retry.random_draws.is_empty());
}

#[test]
fn scoped_random_streams_are_isolated_recorded_hashed_and_replayable() {
    let (scenario, _) = demo_scenario();
    let mut primary_only = Simulation::new(73, scenario.clone()).expect("demo should load");
    primary_only
        .register_plugin(&PrimaryRandomPlugin)
        .expect("primary random plugin should register");

    let mut with_noise = Simulation::new(73, scenario.clone()).expect("demo should load");
    with_noise
        .register_plugin(&NoiseRandomPlugin)
        .expect("noise random plugin should register");
    with_noise
        .register_plugin(&PrimaryRandomPlugin)
        .expect("primary random plugin should register");

    let request = BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily);
    let primary_receipt = primary_only
        .settle_boundary(request.clone())
        .expect("primary boundary should settle");
    with_noise
        .settle_boundary(request)
        .expect("noise boundary should settle");

    let primary_draw = primary_only
        .random_draws()
        .first()
        .expect("the primary system should record its draw");
    let isolated_draw = with_noise
        .random_draws()
        .iter()
        .find(|draw| draw.stream == primary_random_stream())
        .expect("the primary stream should remain present with unrelated noise");
    assert_eq!(primary_draw.value, isolated_draw.value);
    assert_eq!(primary_draw.address, isolated_draw.address);
    assert_eq!(primary_draw.id, primary_receipt.random_draws[0]);
    assert_eq!(
        primary_draw.cause,
        CauseRef::Boundary(primary_receipt.boundary_id)
    );
    assert!(matches!(
        &primary_draw.producer,
        RandomDrawProducer::BoundarySystem {
            boundary,
            plugin,
            system,
        } if *boundary == primary_receipt.boundary_id
            && plugin == "random-primary"
            && system == "roll"
    ));

    let first_hash = primary_receipt.boundary_hash;
    let second_receipt = primary_only
        .settle_boundary(
            BoundaryRequest::at(SimTime::EPOCH + SimDuration::days(1))
                .with_cadence(SystemCadence::Daily),
        )
        .expect("second primary boundary should settle");
    let second_boundary = primary_only
        .boundaries()
        .last()
        .expect("second boundary should be recorded");
    assert_eq!(second_boundary.previous_hash, first_hash);
    assert_eq!(second_boundary.hash, second_receipt.boundary_hash);
    assert!(second_boundary.state_hash.is_some());
    assert_eq!(
        primary_only.boundary_head_hash(),
        Some(second_receipt.boundary_hash.as_str())
    );

    let restored =
        Simulation::from_snapshot_with_plugins(primary_only.snapshot(), &[&PrimaryRandomPlugin])
            .expect("scoped random evidence should survive snapshot restoration");
    assert_eq!(primary_only.snapshot(), restored.snapshot());

    let mut changed_state = primary_only.snapshot();
    changed_state.world.armies[0].morale += 1;
    let Err(error) = Simulation::from_snapshot_with_plugins(changed_state, &[&PrimaryRandomPlugin])
    else {
        panic!("persisted state cannot change while retaining its checkpoint commitment");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    assert!(error.message.contains("commitment roots"));

    let mut missing_state_commitment = primary_only.snapshot();
    missing_state_commitment.boundaries[0].state_hash = None;
    missing_state_commitment.boundaries[0].hash =
        compute_boundary_hash(&missing_state_commitment.boundaries[0])
            .expect("the malformed legacy-style boundary should hash");
    let first_hash = missing_state_commitment.boundaries[0].hash.clone();
    missing_state_commitment.boundaries[1].previous_hash = first_hash;
    missing_state_commitment.boundaries[1].hash =
        compute_boundary_hash(&missing_state_commitment.boundaries[1])
            .expect("the dependent boundary should rehash");
    refresh_snapshot_commitments_and_checkpoint(&mut missing_state_commitment);
    let Err(error) =
        Simulation::from_snapshot_with_plugins(missing_state_commitment, &[&PrimaryRandomPlugin])
    else {
        panic!("current declared runs require every boundary state commitment");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let replayed = Simulation::replay_with_boundaries(
        73,
        scenario,
        &[&PrimaryRandomPlugin],
        primary_only.command_log(),
        primary_only.boundaries(),
        primary_only.time(),
    )
    .expect("scoped draws and boundary hashes should replay exactly");
    assert_eq!(primary_only.snapshot(), replayed.snapshot());

    let mut unsupported_draw = primary_only.snapshot();
    unsupported_draw.random_draws[0].address =
        RandomDrawAddress::OperationV1(RandomOperationAddressV1 {
            producer_plugin: "random-primary".to_owned(),
            operation_kind: "fixture".to_owned(),
            application_operation_id: "operation-1".to_owned(),
            target: RandomOperationTarget::CanonicalKey("target-1".to_owned()),
            draw_slot: 0,
        });
    unsupported_draw.random_draws[0].operation_evidence = Some(canwu_core::EvidenceRef::Boundary(
        primary_receipt.boundary_id,
    ));
    refresh_snapshot_commitments_and_checkpoint(&mut unsupported_draw);
    let Err(error) =
        Simulation::from_snapshot_with_plugins(unsupported_draw, &[&PrimaryRandomPlugin])
    else {
        panic!("forged operation-addressed draws must fail closed");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let mut corrupted_draw = primary_only.snapshot();
    corrupted_draw.random_draws[0].value =
        (corrupted_draw.random_draws[0].value + 1) % corrupted_draw.random_draws[0].upper_exclusive;
    let Err(error) =
        Simulation::from_snapshot_with_plugins(corrupted_draw, &[&PrimaryRandomPlugin])
    else {
        panic!("tampered random evidence must not load");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let mut corrupted_hash = primary_only.snapshot();
    corrupted_hash.boundaries[0].hash.replace_range(..1, "f");
    let Err(error) =
        Simulation::from_snapshot_with_plugins(corrupted_hash, &[&PrimaryRandomPlugin])
    else {
        panic!("tampered boundary hashes must not load");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
}

#[test]
fn run_and_plugin_manifests_bind_continuation_and_replay() {
    let (scenario, _) = demo_scenario();
    let scenario_manifest =
        ArtifactManifest::for_scenario("fixture", "reference-scenario", "1", &scenario)
            .expect("scenario identity should hash");
    let run_configuration = RunConfiguration::read_only_observer();
    let run_configuration_manifest =
        ArtifactManifest::for_run_configuration("fixture", "run-policy", "1", &run_configuration)
            .expect("run configuration should hash");
    let mut run_manifest = RunManifest::declared(scenario_manifest, run_configuration_manifest);
    let RunManifest::Declared {
        rules,
        content,
        localization_contracts,
        sources,
        ..
    } = &mut run_manifest;
    rules.extend([
        ArtifactManifest::from_bytes("fixture", "zeta-rules", "1", b"zeta")
            .expect("rule identity should hash"),
        ArtifactManifest::from_bytes("fixture", "alpha-rules", "1", b"alpha")
            .expect("rule identity should hash"),
    ]);
    content.push(
        ArtifactManifest::from_bytes("fixture", "historical-content", "1", b"content")
            .expect("content identity should hash"),
    );
    localization_contracts.push(
        ArtifactManifest::from_bytes("fixture", "localization-contract", "1", b"keys-v1")
            .expect("localization identity should hash"),
    );
    sources.push(
        ArtifactManifest::from_bytes("fixture", "source-ledger", "1", b"sources")
            .expect("source identity should hash"),
    );

    let mut simulation = Simulation::new_with_run_configuration(
        91,
        scenario.clone(),
        run_manifest,
        run_configuration.clone(),
    )
    .expect("declared run identity should be admitted");
    let RunManifest::Declared { rules, .. } = simulation.run_manifest();
    assert_eq!(rules[0].name, "alpha-rules");
    assert_eq!(rules[1].name, "zeta-rules");
    assert!(is_canonical_hash(simulation.run_manifest_hash()));
    simulation
        .register_plugin(&PrimaryRandomPlugin)
        .expect("versioned plugin should register");
    simulation
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily))
        .expect("manifest-bound boundary should settle");

    let exact_manifest = simulation.run_manifest().clone();
    let snapshot = simulation.snapshot();
    let restored =
        Simulation::from_snapshot_with_plugins(snapshot.clone(), &[&PrimaryRandomPlugin])
            .expect("the exact executable manifest should restore");
    assert_eq!(simulation.snapshot(), restored.snapshot());

    let Err(error) =
        Simulation::from_snapshot_with_plugins(snapshot.clone(), &[&ChangedPrimaryRandomPlugin])
    else {
        panic!("changed executable semantics must not rehydrate an exact descriptor");
    };
    assert_eq!(error.code, ErrorCode::PluginManifestMismatch);

    let mut changed_scenario = scenario.clone();
    changed_scenario.world.armies[0].strength += 1;
    let Err(error) = Simulation::new_with_run_configuration(
        91,
        changed_scenario,
        exact_manifest.clone(),
        run_configuration.clone(),
    ) else {
        panic!("a scenario must match its declared semantic identity");
    };
    assert_eq!(error.code, ErrorCode::InvalidRunManifest);

    let mut corrupted_manifest_hash = snapshot.clone();
    let replacement = if corrupted_manifest_hash.run_manifest_hash.starts_with('f') {
        "e"
    } else {
        "f"
    };
    corrupted_manifest_hash
        .run_manifest_hash
        .replace_range(..1, replacement);
    let Err(error) = Simulation::from_snapshot(corrupted_manifest_hash) else {
        panic!("a tampered run manifest hash must not load");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let replayed = Simulation::replay_with_run_configuration(
        91,
        scenario.clone(),
        exact_manifest.clone(),
        run_configuration.clone(),
        &[&PrimaryRandomPlugin],
        simulation.command_log(),
        simulation.command_attempts(),
        simulation.boundaries(),
        simulation.time(),
    )
    .expect("the exact run and plugin environment should replay");
    assert_eq!(simulation.snapshot(), replayed.snapshot());

    let mut changed_environment = exact_manifest;
    let RunManifest::Declared { content, .. } = &mut changed_environment;
    content[0].semantic_hash =
        "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".to_owned();
    let Err(error) = Simulation::replay_with_run_configuration(
        91,
        scenario,
        changed_environment,
        run_configuration,
        &[&PrimaryRandomPlugin],
        simulation.command_log(),
        simulation.command_attempts(),
        simulation.boundaries(),
        simulation.time(),
    ) else {
        panic!("replay under changed content identity must fail");
    };
    assert_eq!(error.code, ErrorCode::ReplayMismatch);
}

#[test]
fn plugin_reads_and_writes_are_limited_to_declared_owned_state() {
    let (mut simulation, ids) = Simulation::demo(35).expect("demo should load");
    simulation
        .register_plugin(&SecretPlugin)
        .expect("secret owner should register");
    simulation
        .register_plugin(&UndeclaredAccessPlugin)
        .expect("access fixture should register");
    simulation
        .submit(CommandEnvelope::new(
            Issuer::Actor(ids.commander),
            Command::Plugin {
                plugin: "secret-owner".to_owned(),
                command: "seed".to_owned(),
                payload: Value::Null,
            },
        ))
        .expect("the owner should write its declared state");
    let before = simulation
        .snapshot_json()
        .expect("snapshot should serialize");

    for (command, expected) in [
        ("missing", ErrorCode::EntityNotFound),
        ("read", ErrorCode::UndeclaredStateRead),
        ("write", ErrorCode::UndeclaredStateWrite),
    ] {
        let error = simulation
            .submit(CommandEnvelope::new(
                Issuer::Actor(ids.commander),
                Command::Plugin {
                    plugin: "undeclared-access".to_owned(),
                    command: command.to_owned(),
                    payload: Value::Null,
                },
            ))
            .expect_err("undeclared state access must fail");
        assert_eq!(error.code, expected);
        assert_eq!(
            before,
            simulation
                .snapshot_json()
                .expect("rejected access must leave no serialized change")
        );
    }
}

#[test]
fn typed_component_keys_isolate_adversarial_plugin_and_state_names() {
    let (mut simulation, ids) = Simulation::demo(35).expect("demo should load");
    simulation
        .register_plugin(&CollisionPluginA)
        .expect("first collision fixture should register");
    simulation
        .register_plugin(&CollisionPluginB)
        .expect("second collision fixture should register");
    for (plugin, expected) in [("a", "first"), ("a/person:1/b", "second")] {
        simulation
            .submit(CommandEnvelope::new(
                Issuer::Actor(ids.commander),
                Command::Plugin {
                    plugin: plugin.to_owned(),
                    command: "write".to_owned(),
                    payload: Value::Null,
                },
            ))
            .expect("adversarial key should remain isolated");
        assert!(
            simulation
                .snapshot()
                .plugin_components
                .iter()
                .any(|record| {
                    record.plugin == plugin && record.value == Value::String(expected.to_owned())
                })
        );
    }
    assert_eq!(simulation.snapshot().plugin_components.len(), 2);
}

#[test]
fn plugin_event_order_does_not_depend_on_registration_order() {
    let (scenario, ids) = demo_scenario();
    let mut first = Simulation::new(35, scenario.clone()).expect("demo should load");
    first
        .register_plugin(&MarkerPlugin {
            name: "zeta",
            writes: Vec::new(),
        })
        .expect("zeta should register");
    first
        .register_plugin(&MarkerPlugin {
            name: "alpha",
            writes: Vec::new(),
        })
        .expect("alpha should register");

    let mut second = Simulation::new(35, scenario).expect("demo should load");
    second
        .register_plugin(&MarkerPlugin {
            name: "alpha",
            writes: Vec::new(),
        })
        .expect("alpha should register");
    second
        .register_plugin(&MarkerPlugin {
            name: "zeta",
            writes: Vec::new(),
        })
        .expect("zeta should register");

    first
        .submit(move_order(&ids))
        .expect("first order should validate");
    second
        .submit(move_order(&ids))
        .expect("second order should validate");
    assert_eq!(first.snapshot(), second.snapshot());
    let marker_plugins: Vec<_> = first
        .events()
        .iter()
        .filter_map(|event| event.kind.plugin_identity().map(|(plugin, _)| plugin))
        .collect();
    assert_eq!(marker_plugins, vec!["alpha", "zeta"]);
}

#[test]
fn failed_command_application_rolls_back_every_serialized_change() {
    let (mut simulation, ids) = Simulation::demo(35).expect("demo should load");
    simulation
        .register_plugin(&FailingPlugin)
        .expect("plugin should register");
    let before = simulation
        .snapshot_json()
        .expect("snapshot should serialize");
    let error = simulation
        .submit(CommandEnvelope::new(
            Issuer::Actor(ids.commander),
            Command::Plugin {
                plugin: "failing-test".to_owned(),
                command: "mutate".to_owned(),
                payload: serde_json::json!({ "scheduled": false }),
            },
        ))
        .expect_err("the injected failure should reject the command");
    assert_eq!(error.code, ErrorCode::InvalidDuration);
    assert_eq!(
        before,
        simulation
            .snapshot_json()
            .expect("failed command must leave no mutation, event, or consumed ID")
    );

    let panic_error = simulation
        .submit(CommandEnvelope::new(
            Issuer::Actor(ids.commander),
            Command::Plugin {
                plugin: "failing-test".to_owned(),
                command: "panic".to_owned(),
                payload: Value::Null,
            },
        ))
        .expect_err("plugin panics must cross the boundary as structured errors");
    assert_eq!(panic_error.code, ErrorCode::PluginPanicked);
    assert_eq!(
        before,
        simulation
            .snapshot_json()
            .expect("a panicking plugin must leave no serialized change")
    );

    let (mut ceiling_scenario, ceiling_ids) = demo_scenario();
    ceiling_scenario.start_time = SimTime::from_minutes(i64::MAX - 60);
    let mut ceiling = Simulation::new(35, ceiling_scenario)
        .expect("a scenario near the time ceiling should load");
    let ceiling_before = ceiling
        .snapshot_json()
        .expect("the ceiling fixture should serialize");
    let movement_error = ceiling
        .submit(move_order(&ceiling_ids))
        .expect_err("movement whose arrival overflows simulation time must fail");
    assert_eq!(movement_error.code, ErrorCode::InvalidDuration);
    let advance_error = ceiling
        .advance(SimDuration::hours(2))
        .expect_err("advancing beyond the time domain must fail");
    assert_eq!(advance_error.code, ErrorCode::InvalidDuration);
    assert_eq!(
        ceiling_before,
        ceiling
            .snapshot_json()
            .expect("time overflow must leave the simulation unchanged")
    );

    ceiling
        .register_plugin(&FailingPlugin)
        .expect("registration should remain open after rejected execution");
    let scheduled_before = ceiling
        .snapshot_json()
        .expect("the registered ceiling fixture should serialize");
    let schedule_error = ceiling
        .submit(CommandEnvelope::new(
            Issuer::Actor(ceiling_ids.commander),
            Command::Plugin {
                plugin: "failing-test".to_owned(),
                command: "mutate".to_owned(),
                payload: serde_json::json!({ "scheduled": true }),
            },
        ))
        .expect_err("plugin work whose target overflows simulation time must fail");
    assert_eq!(schedule_error.code, ErrorCode::InvalidDuration);
    assert_eq!(
        scheduled_before,
        ceiling
            .snapshot_json()
            .expect("rejected plugin scheduling must not mutate the simulation")
    );
}

#[test]
fn failed_scheduled_batch_restores_every_writable_domain() {
    let (mut simulation, ids) = Simulation::demo(35).expect("demo should load");
    simulation
        .register_plugin(&FailingPlugin)
        .expect("plugin should register");
    simulation
        .submit(move_order(&ids))
        .expect("the arrival should schedule");
    let arrival_at = SimTime::EPOCH
        .checked_add(SimDuration::hours(18))
        .expect("arrival time should be representable");
    simulation
        .schedule_at(
            arrival_at,
            ScheduledAction::PluginDirective {
                plugin: "failing-test".to_owned(),
                directive: Box::new(SystemDirective::SetComponent {
                    state: StateKey::new("failure-fixture", "flag"),
                    entity: EntityRef::Army(ids.army),
                    component: "flag".to_owned(),
                    value: Value::Bool(true),
                    summary: "Set a flag after the arrival mutates state".to_owned(),
                }),
                allowed_writes: vec![StateKey::new("failure-fixture", "flag")],
                cause: CauseRef::System("scheduled-rollback-fixture".to_owned()),
                correlation_id: 0,
            },
        )
        .expect("the failing action should share the arrival timestamp");
    let cache_before = cache_fingerprint(&simulation);
    let before_boundary = simulation
        .snapshot_json()
        .expect("snapshot should serialize");
    let error = simulation
        .advance(SimDuration::hours(18))
        .expect_err("the scheduled batch should fail after the arrival");
    assert_eq!(error.code, ErrorCode::InvalidDuration);
    assert_eq!(
        before_boundary,
        simulation
            .snapshot_json()
            .expect("failed boundary must restore its clock, queue, state, events, and IDs")
    );
    assert_eq!(cache_fingerprint(&simulation), cache_before);
}

#[test]
fn failed_clock_only_advance_restores_time_and_commitments() {
    let (mut simulation, _) = Simulation::demo(35).expect("demo should load");
    simulation
        .state
        .metadata
        .commitment_cache
        .as_mut()
        .expect("current runtimes should maintain a commitment cache")
        .events
        .len = 2;
    let cache_before = cache_fingerprint(&simulation);
    let before = simulation
        .snapshot_json()
        .expect("snapshot should serialize");
    let error = simulation
        .advance(SimDuration::hours(1))
        .expect_err("the corrupt cache must abort clock-only advancement");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    assert_eq!(
        before,
        simulation
            .snapshot_json()
            .expect("failed clock advancement must restore serialized state")
    );
    assert_eq!(cache_fingerprint(&simulation), cache_before);
}

#[test]
fn snapshot_continuation_requires_exact_plugin_rehydration() {
    let (mut simulation, ids) = Simulation::demo(35).expect("demo should load");
    let plugin = AuthorityPlugin;
    simulation
        .register_plugin(&plugin)
        .expect("plugin should register");
    simulation
        .submit(CommandEnvelope::new(
            Issuer::Actor(ids.commander),
            Command::Plugin {
                plugin: "authority-test".to_owned(),
                command: "set_stance".to_owned(),
                payload: Value::Null,
            },
        ))
        .expect("plugin command should succeed");
    let json = simulation
        .snapshot_json()
        .expect("snapshot should serialize");

    let mut restored = Simulation::from_snapshot_json(&json).expect("snapshot should load");
    assert_eq!(
        restored
            .advance(SimDuration::ZERO)
            .expect_err("continuation without handlers must be blocked")
            .code,
        ErrorCode::PluginNotActive
    );
    let mismatch = MarkerPlugin {
        name: "authority-test",
        writes: Vec::new(),
    };
    assert_eq!(
        restored
            .register_plugin(&mismatch)
            .expect_err("a different executable manifest must be rejected")
            .code,
        ErrorCode::PluginManifestMismatch
    );
    restored
        .register_plugin(&plugin)
        .expect("the exact plugin manifest should rehydrate");
    restored
        .advance(SimDuration::ZERO)
        .expect("rehydrated snapshot should continue");
    assert_eq!(simulation.snapshot(), restored.snapshot());
}

#[test]
fn command_only_replay_journal_binds_the_recorded_plugin_environment() {
    let (scenario, ids) = demo_scenario();
    let mut registration_closed_only =
        Simulation::new(35, scenario.clone()).expect("demo should load");
    registration_closed_only
        .advance(SimDuration::ZERO)
        .expect("zero advance should close authoritative registration");
    let closure_journal = registration_closed_only.replay_journal();
    let closure_replay =
        Simulation::replay_from_journal_with_scenario(scenario.clone(), &[], &closure_journal)
            .expect("exact replay should reproduce registration closure without other work");
    assert_eq!(
        registration_closed_only.snapshot(),
        closure_replay.snapshot()
    );

    let plugin = AuthorityPlugin;
    let mut simulation = Simulation::new(35, scenario.clone()).expect("demo should load");
    simulation
        .register_plugin(&plugin)
        .expect("plugin should register");
    simulation
        .submit(CommandEnvelope::new(
            Issuer::Actor(ids.commander),
            Command::Plugin {
                plugin: "authority-test".to_owned(),
                command: "set_stance".to_owned(),
                payload: Value::Null,
            },
        ))
        .expect("plugin command should succeed");

    let replay_without_plugins = Simulation::replay(
        35,
        scenario.clone(),
        simulation.command_log(),
        simulation.time(),
    );
    let Err(error) = replay_without_plugins else {
        panic!("plugin replay without executable handlers must fail");
    };
    assert_eq!(error.code, ErrorCode::PluginCommandNotFound);
    let journal = simulation.replay_journal();
    let exact = Simulation::replay_from_journal_with_scenario(
        scenario.clone(),
        &[&AuthorityPlugin],
        &journal,
    )
    .expect("the exact command-only environment should replay");
    assert_eq!(simulation.snapshot(), exact.snapshot());

    let Err(error) = Simulation::replay_from_journal_with_scenario(
        scenario.clone(),
        &[&ChangedAuthorityPlugin],
        &journal,
    ) else {
        panic!("changed handler semantics must fail before command-only replay");
    };
    assert_eq!(error.code, ErrorCode::ReplayEnvironmentMismatch);

    let replayed = Simulation::replay_with_plugins(
        35,
        scenario,
        &[&plugin],
        simulation.command_log(),
        simulation.time(),
    )
    .expect("plugin-aware replay should succeed");
    assert_eq!(simulation.snapshot(), replayed.snapshot());
}
