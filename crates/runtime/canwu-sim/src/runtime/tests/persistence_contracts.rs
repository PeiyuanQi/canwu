use super::*;

#[test]
fn internal_runtime_partitions_preserve_flat_persistence_contracts() {
    let (scenario, ids) = demo_scenario();
    let mut simulation =
        Simulation::new(35, scenario.clone()).expect("the demo scenario should load");
    simulation
        .submit(move_order(&ids))
        .expect("the move should populate evidence and scheduled work");
    simulation
        .advance(SimDuration::days(1))
        .expect("the scheduled move should complete");

    let snapshot = simulation.snapshot();
    let snapshot_value = serde_json::to_value(&snapshot).expect("the snapshot should become JSON");
    let snapshot_object = snapshot_value
        .as_object()
        .expect("snapshot JSON should remain a flat object");
    for public_field in [
        "checkpoint_hash",
        "state_revision",
        "next_event_id",
        "admission_cursor_format_version",
        "events",
        "commands",
        "scheduled",
    ] {
        assert!(
            snapshot_object.contains_key(public_field),
            "snapshot should retain flat field {public_field}"
        );
    }
    for internal_owner in ["current", "metadata", "counters", "evidence", "scheduler"] {
        assert!(
            !snapshot_object.contains_key(internal_owner),
            "private owner {internal_owner} must not enter the snapshot wire shape"
        );
    }

    let json = serde_json::to_string(&snapshot).expect("the snapshot should serialize");
    let restored = Simulation::from_snapshot_json(&json).expect("the flat snapshot should restore");
    assert_eq!(restored.snapshot(), snapshot);

    let journal = restored.replay_journal();
    let journal_value =
        serde_json::to_value(&journal).expect("the replay journal should become JSON");
    let journal_object = journal_value
        .as_object()
        .expect("replay journal JSON should remain a flat object");
    for internal_owner in ["current", "metadata", "counters", "evidence", "scheduler"] {
        assert!(
            !journal_object.contains_key(internal_owner),
            "private owner {internal_owner} must not enter the journal wire shape"
        );
    }
    let replayed = Simulation::replay_from_journal(scenario, &[], &journal)
        .expect("the flat replay journal should remain exact");
    assert_eq!(replayed.snapshot(), snapshot);
}

#[test]
fn domain_commitments_migrate_replay_and_reject_each_tampered_root() {
    let (scenario, ids) = demo_scenario();
    let mut simulation =
        Simulation::new(97, scenario.clone()).expect("the commitment fixture should load");
    simulation
        .submit(move_order(&ids))
        .expect("the commitment fixture should accept its command");
    simulation
        .advance(SimDuration::days(1))
        .expect("the commitment fixture should execute scheduled work");
    let snapshot = simulation.snapshot();
    assert_eq!(
        snapshot.commitment_format_version,
        COMMITMENT_FORMAT_VERSION
    );
    assert!(commitment_roots_are_canonical(
        snapshot
            .commitment_roots
            .as_ref()
            .expect("current snapshots should persist domain roots")
    ));

    let expected_roots =
        snapshot_commitment_roots(&snapshot).expect("the canonical snapshot should produce roots");
    let mut reordered = snapshot.clone();
    reordered.world.people.reverse();
    reordered.world.governments.reverse();
    reordered.world.territories.reverse();
    reordered.world.routes.reverse();
    reordered.world.armies.reverse();
    reordered.events.reverse();
    reordered.commands.reverse();
    reordered.command_attempts.reverse();
    reordered.ingress.reverse();
    reordered.plugin_components.reverse();
    reordered.domain_records.reverse();
    reordered.plugin_descriptors.reverse();
    reordered.random_streams.reverse();
    reordered.random_draws.reverse();
    reordered.scheduled.reverse();
    assert_eq!(
        snapshot_commitment_roots(&reordered)
            .expect("collection insertion order should not affect roots"),
        expected_roots
    );

    for root_name in [
        "world",
        "knowledge",
        "plugin_components",
        "domain_records",
        "scheduler",
        "commands",
        "events",
        "ingress",
        "random",
        "boundary_chain",
        "identity",
        "control",
    ] {
        let mut forged = snapshot.clone();
        let mut roots_value = serde_json::to_value(
            forged
                .commitment_roots
                .as_ref()
                .expect("the fixture should persist roots"),
        )
        .expect("commitment roots should become JSON");
        roots_value
            .as_object_mut()
            .expect("commitment roots should be an object")
            .insert(root_name.to_owned(), Value::String("0".repeat(64)));
        forged.commitment_roots = Some(
            serde_json::from_value(roots_value)
                .expect("the forged commitment roots should deserialize"),
        );
        forged.checkpoint_hash = snapshot_checkpoint_hash(&forged)
            .expect("the forged roots should produce a coherent outer checkpoint");
        let error = Simulation::from_snapshot(forged)
            .err()
            .expect("every forged domain root must be rejected");
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);
        assert!(error.message.contains("commitment roots"));
    }

    let mut legacy_snapshot = snapshot.clone();
    downgrade_snapshot_commitments(&mut legacy_snapshot);
    legacy_snapshot.checkpoint_hash = snapshot_checkpoint_hash(&legacy_snapshot)
        .expect("the legacy fixture should reproduce checkpoint v3");
    let legacy_checkpoint = legacy_snapshot.checkpoint_hash.clone();
    let migrated = Simulation::from_snapshot(legacy_snapshot.clone())
        .expect("a verified legacy checkpoint should derive current roots");
    assert_eq!(
        migrated.snapshot().commitment_format_version,
        COMMITMENT_FORMAT_VERSION
    );
    assert_ne!(migrated.checkpoint_hash(), legacy_checkpoint);

    let mut tampered_legacy = legacy_snapshot;
    tampered_legacy.world.armies[0].morale += 1;
    let error = Simulation::from_snapshot(tampered_legacy)
        .err()
        .expect("migration must verify the old checkpoint before deriving roots");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    assert!(error.message.contains("pre-commitment state"));

    let mut legacy_journal = simulation.replay_journal();
    legacy_journal.commitment_format_version = 0;
    legacy_journal.checkpoint_hash = legacy_checkpoint;
    let replayed = Simulation::replay_from_journal(scenario, &[], &legacy_journal)
        .expect("legacy commitment journals should replay under checkpoint v3");
    assert_eq!(replayed.snapshot().commitment_format_version, 0);
    assert!(replayed.snapshot().commitment_roots.is_none());
    assert_eq!(replayed.checkpoint_hash(), legacy_journal.checkpoint_hash);
}

#[test]
fn boundary_state_commitments_are_incremental_versioned_and_legacy_replayable() {
    let (scenario, _) = demo_scenario();
    let configuration = RunConfiguration::read_only_observer();
    let manifest = RunManifest::declared(
        ArtifactManifest::for_scenario("canwu.test", "boundary-state-fixture", "1", &scenario)
            .expect("the boundary-state scenario should hash"),
        ArtifactManifest::for_run_configuration(
            "canwu.test",
            "boundary-state-run",
            "1",
            &configuration,
        )
        .expect("the boundary-state run configuration should hash"),
    );

    let mut current = Simulation::new_with_run_configuration(
        211,
        scenario.clone(),
        manifest.clone(),
        configuration.clone(),
    )
    .expect("the current boundary-state fixture should load");
    current
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("a current boundary should settle");
    let current_hash = current.boundaries()[0]
        .state_hash
        .as_deref()
        .expect("current boundaries should commit their state");
    let current_digest = current_hash
        .strip_prefix(BOUNDARY_STATE_HASH_V1_PREFIX)
        .expect("current boundaries should use the tagged commitment contract");
    assert!(is_canonical_hash(current_digest));
    let current_snapshot = current.snapshot();
    assert_eq!(
        current_snapshot.boundaries[0].state_hash.as_deref(),
        Some(
            snapshot_boundary_head_state_hash(&current_snapshot)
                .expect("the current boundary head should reproduce from persisted roots")
                .as_str()
        )
    );
    let current_restored = Simulation::from_snapshot(current_snapshot.clone())
        .expect("the current boundary commitment should load");
    assert_eq!(current_restored.snapshot(), current_snapshot);
    let current_replayed =
        Simulation::replay_from_journal(scenario.clone(), &[], &current.replay_journal())
            .expect("the current boundary commitment should replay exactly");
    assert_eq!(current_replayed.snapshot(), current_snapshot);
    let mut mislabeled_journal = current.replay_journal();
    mislabeled_journal.commitment_format_version = 0;
    let error = Simulation::replay_from_journal(scenario.clone(), &[], &mislabeled_journal)
        .err()
        .expect("a current boundary commitment cannot use a legacy journal contract");
    assert_eq!(error.code, ErrorCode::ReplayEnvironmentMismatch);

    let mut forged_state = current_snapshot.clone();
    forged_state.world.armies[0].morale += 1;
    refresh_snapshot_commitments_and_checkpoint(&mut forged_state);
    let error = Simulation::from_snapshot(forged_state)
        .err()
        .expect("coherently rehashed current state must still match its boundary head");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    assert!(error.message.contains("boundary-head state commitment"));

    let mut unsupported = current_snapshot;
    unsupported.boundaries[0].state_hash = Some(format!("v2:{}", "0".repeat(64)));
    rehash_tampered_snapshot(&mut unsupported);
    let error = Simulation::from_snapshot(unsupported)
        .err()
        .expect("unknown boundary state commitment tags must be rejected");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    assert!(error.message.contains("boundary state commitment"));

    let mut legacy =
        Simulation::new_with_run_configuration(223, scenario.clone(), manifest, configuration)
            .expect("the legacy boundary-state fixture should load");
    legacy
        .settle_boundary_with_state_hash_format(
            BoundaryRequest::at(SimTime::EPOCH),
            BoundaryStateHashFormat::LegacyV0,
        )
        .expect("a legacy boundary should remain reproducible");
    let legacy_hash = legacy.boundaries()[0]
        .state_hash
        .as_deref()
        .expect("legacy declared boundaries should commit their state");
    assert!(is_canonical_hash(legacy_hash));
    let legacy_snapshot = legacy.snapshot();
    let mut mixed = Simulation::from_snapshot(legacy_snapshot.clone())
        .expect("an existing legacy boundary commitment should still load");
    let legacy_replayed =
        Simulation::replay_from_journal(scenario.clone(), &[], &legacy.replay_journal())
            .expect("an existing legacy boundary commitment should replay exactly");
    assert_eq!(legacy_replayed.snapshot(), legacy_snapshot);

    mixed
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH + SimDuration::days(1)))
        .expect("continuation should append the current commitment contract");
    assert!(is_canonical_hash(
        mixed.boundaries()[0]
            .state_hash
            .as_deref()
            .expect("the legacy boundary should retain its state commitment")
    ));
    assert!(
        mixed.boundaries()[1]
            .state_hash
            .as_deref()
            .expect("the continued boundary should commit its state")
            .starts_with(BOUNDARY_STATE_HASH_V1_PREFIX)
    );
    let mixed_snapshot = mixed.snapshot();
    let mixed_restored = Simulation::from_snapshot(mixed_snapshot.clone())
        .expect("a mixed legacy/current boundary chain should load");
    assert_eq!(mixed_restored.snapshot(), mixed_snapshot);
    let mixed_replayed = Simulation::replay_from_journal(scenario, &[], &mixed.replay_journal())
        .expect("a mixed legacy/current boundary chain should replay exactly");
    assert_eq!(mixed_replayed.snapshot(), mixed_snapshot);
}

#[test]
fn cached_mutable_commitments_match_independent_snapshot_roots_after_each_mutation() {
    fn assert_exact(simulation: &Simulation) {
        let snapshot = simulation.snapshot();
        let expected = snapshot_commitment_roots(&snapshot)
            .expect("serialized state should independently reproduce every commitment root");
        assert_eq!(snapshot.commitment_roots.as_ref(), Some(&expected));
        let cache = simulation
            .state
            .metadata
            .commitment_cache
            .as_ref()
            .expect("current runtimes should maintain a private commitment cache");
        assert!(
            [
                &cache.world,
                &cache.knowledge,
                &cache.plugin_components,
                &cache.domain_records,
                &cache.scheduler,
                &cache.random_streams,
                &cache.identity,
            ]
            .into_iter()
            .all(Option::is_some),
            "every invalidated domain must be refreshed before a transaction commits"
        );
    }

    let (scenario, ids) = demo_scenario();
    let mut simulation = Simulation::new(101, scenario.clone()).expect("cache fixture should load");
    assert_exact(&simulation);
    simulation
        .register_plugin(&AuthorityPlugin)
        .expect("component plugin should register");
    assert_exact(&simulation);
    simulation
        .register_plugin(&PrimaryRandomPlugin)
        .expect("random plugin should register");
    assert_exact(&simulation);

    let before_rejection = simulation
        .snapshot()
        .commitment_roots
        .expect("current snapshots should have roots");
    let rejected = simulation
        .process_command(CommandRequest::new(
            CommandRequestId::new(1),
            simulation.revision() + 1,
            CommandEnvelope::new(
                Issuer::Debug,
                Command::DebugSetArmyMorale {
                    army: ids.army,
                    morale: 75,
                },
            ),
        ))
        .expect("stale input should become deterministic rejection evidence");
    assert!(matches!(rejected, CommandOutcome::Rejected { .. }));
    assert_exact(&simulation);
    let after_rejection = simulation
        .snapshot()
        .commitment_roots
        .expect("current snapshots should have roots");
    assert_eq!(before_rejection.world, after_rejection.world);
    assert_eq!(before_rejection.knowledge, after_rejection.knowledge);
    assert_eq!(
        before_rejection.plugin_components,
        after_rejection.plugin_components
    );
    assert_eq!(
        before_rejection.domain_records,
        after_rejection.domain_records
    );
    assert_eq!(before_rejection.scheduler, after_rejection.scheduler);
    assert_eq!(before_rejection.random, after_rejection.random);
    assert_eq!(before_rejection.identity, after_rejection.identity);
    assert_ne!(before_rejection.commands, after_rejection.commands);
    assert_ne!(before_rejection.control, after_rejection.control);

    simulation
        .process_command(CommandRequest::new(
            CommandRequestId::new(2),
            simulation.revision(),
            CommandEnvelope::new(
                Issuer::Actor(ids.commander),
                Command::Plugin {
                    plugin: "authority-test".to_owned(),
                    command: "set_stance".to_owned(),
                    payload: Value::Null,
                },
            ),
        ))
        .expect("component command should commit");
    assert_exact(&simulation);
    simulation
        .process_command(CommandRequest::new(
            CommandRequestId::new(3),
            simulation.revision(),
            move_order(&ids),
        ))
        .expect("movement command should commit");
    assert_exact(&simulation);
    simulation
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily))
        .expect("random boundary should commit");
    assert_exact(&simulation);
    simulation
        .advance(SimDuration::days(1))
        .expect("scheduled arrival should commit");
    assert_exact(&simulation);

    let mut records = Simulation::new(103, scenario).expect("record cache fixture should load");
    records
        .register_plugin(&RecordLifecyclePlugin)
        .expect("record plugin should register");
    assert_exact(&records);
    records
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily))
        .expect("record mutation boundary should commit");
    assert_exact(&records);
}

#[test]
fn rejection_transaction_restores_private_commitment_state_after_hash_failure() {
    let (scenario, ids) = demo_scenario();
    let mut simulation =
        Simulation::new(107, scenario).expect("rejection rollback fixture should load");
    let before = simulation.snapshot();
    simulation
        .state
        .metadata
        .commitment_cache
        .as_mut()
        .expect("current runtimes should maintain a commitment cache")
        .attempts
        .len = 2;

    let error = simulation
        .process_command(CommandRequest::new(
            CommandRequestId::new(1),
            0,
            CommandEnvelope::new(
                Issuer::Debug,
                Command::DebugSetArmyMorale {
                    army: ids.army,
                    morale: 101,
                },
            ),
        ))
        .expect_err("a fatal commitment-cache mismatch must abort the rejection transaction");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    assert_eq!(simulation.snapshot(), before);
    let restored_cache = simulation
        .state
        .metadata
        .commitment_cache
        .as_ref()
        .expect("rollback should restore the private cache");
    assert_eq!(restored_cache.attempts.len, 2);
    assert!(simulation.command_attempts().is_empty());
    assert_eq!(simulation.state.counters.next_command_attempt_id, 1);
    assert_eq!(simulation.revision(), 0);

    simulation.state.metadata.commitment_cache = None;
    simulation
        .refresh_checkpoint_hash()
        .expect("discarding the injected corrupt cache should rebuild it from evidence");
    let outcome = simulation
        .process_command(CommandRequest::new(
            CommandRequestId::new(1),
            0,
            CommandEnvelope::new(
                Issuer::Debug,
                Command::DebugSetArmyMorale {
                    army: ids.army,
                    morale: 101,
                },
            ),
        ))
        .expect("the repaired runtime should persist the same expected rejection");
    assert!(matches!(outcome, CommandOutcome::Rejected { .. }));
    let snapshot = simulation.snapshot();
    assert_eq!(
        snapshot.commitment_roots,
        Some(
            snapshot_commitment_roots(&snapshot)
                .expect("the repaired rejection should independently reproduce its roots")
        )
    );
}

#[test]
fn ingress_transaction_restores_queue_and_private_commitments_after_hash_failure() {
    let (scenario, _) = demo_scenario();
    let mut simulation =
        Simulation::new(108, scenario).expect("ingress rollback fixture should load");
    let before = simulation.snapshot();
    simulation
        .state
        .metadata
        .commitment_cache
        .as_mut()
        .expect("current runtimes should maintain a commitment cache")
        .ingress
        .len = 2;
    let cache_before = cache_fingerprint(&simulation);

    let error = simulation
        .schedule_calendar_boundary(SimTime::EPOCH, vec![SystemCadence::Daily])
        .expect_err("a fatal commitment-cache mismatch must abort ingress insertion");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    assert_eq!(simulation.snapshot(), before);
    assert_eq!(cache_fingerprint(&simulation), cache_before);
    let restored_cache = simulation
        .state
        .metadata
        .commitment_cache
        .as_ref()
        .expect("rollback should restore the private cache");
    assert_eq!(restored_cache.ingress.len, 2);
    assert!(simulation.ingress_log().is_empty());
    assert!(simulation.state.scheduler.pending_ingress.is_empty());
    assert_eq!(simulation.state.counters.next_ingress_id, 1);

    simulation.state.metadata.commitment_cache = None;
    simulation
        .refresh_checkpoint_hash()
        .expect("discarding the injected corrupt cache should rebuild it from evidence");
    let receipt = simulation
        .schedule_calendar_boundary(SimTime::EPOCH, vec![SystemCadence::Daily])
        .expect("the repaired runtime should queue the same calendar boundary");
    assert_eq!(receipt.ingress_id, IngressId::new(1));
    let snapshot = simulation.snapshot();
    assert_eq!(
        snapshot.commitment_roots,
        Some(
            snapshot_commitment_roots(&snapshot)
                .expect("the repaired ingress should independently reproduce its roots")
        )
    );
}

#[test]
fn command_transaction_restores_writable_domains_after_hash_failure() {
    let (scenario, ids) = demo_scenario();
    let mut simulation =
        Simulation::new(109, scenario).expect("command rollback fixture should load");
    let before = simulation.snapshot();
    simulation
        .state
        .metadata
        .commitment_cache
        .as_mut()
        .expect("current runtimes should maintain a commitment cache")
        .commands
        .len = 2;
    let request = || {
        CommandRequest::new(
            CommandRequestId::new(1),
            0,
            CommandEnvelope::new(
                Issuer::Debug,
                Command::DebugSetArmyMorale {
                    army: ids.army,
                    morale: 73,
                },
            ),
        )
    };

    let error = simulation
        .process_command(request())
        .expect_err("a fatal commitment-cache mismatch must abort command application");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    assert_eq!(simulation.snapshot(), before);
    let restored_cache = simulation
        .state
        .metadata
        .commitment_cache
        .as_ref()
        .expect("rollback should restore the private cache");
    assert_eq!(restored_cache.commands.len, 2);

    simulation.state.metadata.commitment_cache = None;
    simulation
        .refresh_checkpoint_hash()
        .expect("discarding the injected corrupt cache should rebuild it from evidence");
    let outcome = simulation
        .process_command(request())
        .expect("the repaired runtime should accept the same command");
    assert!(matches!(outcome, CommandOutcome::Accepted { .. }));
    let snapshot = simulation.snapshot();
    assert_eq!(
        snapshot.commitment_roots,
        Some(
            snapshot_commitment_roots(&snapshot)
                .expect("the repaired command should independently reproduce its roots")
        )
    );
}

#[test]
fn checkpoint_journals_are_incremental_contiguous_and_exact() {
    let (scenario, _) = demo_scenario();
    let plugins: &[&dyn SimulationPlugin] = &[&JournalCommandPlugin, &BoundaryRollbackPlugin];
    let mut simulation =
        Simulation::new(35, scenario.clone()).expect("checkpoint fixture should load");
    for plugin in plugins {
        simulation
            .register_plugin(*plugin)
            .expect("checkpoint fixture plugin should register");
    }
    simulation
        .enqueue_command(
            SimTime::EPOCH,
            0,
            CommandRequest::new(
                CommandRequestId::new(1),
                0,
                CommandEnvelope::new(
                    Issuer::Debug,
                    Command::Plugin {
                        plugin: "journal-command".to_owned(),
                        command: "noop".to_owned(),
                        payload: Value::Null,
                    },
                ),
            ),
        )
        .expect("checkpoint fixture command should queue");
    simulation
        .step_canonical()
        .expect("the first canonical boundary should settle")
        .expect("the queued command should produce a boundary");
    let first_cursor = simulation
        .evidence_cursor()
        .expect("the first journal cursor should be representable");
    let first_segment = simulation
        .journal_segment_since(EvidenceCursor::default())
        .expect("the first evidence segment should export");
    assert_eq!(first_segment.start, EvidenceCursor::default());
    assert_eq!(first_segment.end, first_cursor);

    simulation
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily))
        .expect("the random and ingress-producing boundary should settle");
    simulation
        .advance_canonical(SimDuration::hours(1))
        .expect("generated ingress should enter a later boundary");
    let checkpoint = simulation
        .checkpoint()
        .expect("current state should checkpoint without evidence cloning");
    assert_eq!(checkpoint.format_version, CHECKPOINT_JOURNAL_FORMAT_VERSION);
    assert!(checkpoint.state.events.is_empty());
    assert!(checkpoint.state.commands.is_empty());
    assert!(checkpoint.state.command_attempts.is_empty());
    assert!(checkpoint.state.ingress.is_empty());
    assert!(checkpoint.state.boundaries.is_empty());
    assert!(checkpoint.state.random_draws.is_empty());
    assert_eq!(
        checkpoint.journal_end,
        simulation
            .evidence_cursor()
            .expect("the final journal cursor should be representable")
    );
    let second_segment = simulation
        .journal_segment_since(first_cursor)
        .expect("only evidence after the first checkpoint should export");
    assert_eq!(second_segment.start, first_cursor);
    assert_eq!(second_segment.end, checkpoint.journal_end);
    assert!(!second_segment.events.is_empty());
    assert!(!second_segment.ingress.is_empty());
    assert!(!second_segment.boundaries.is_empty());
    assert!(!second_segment.random_draws.is_empty());

    let bundle = CheckpointJournal {
        checkpoint: checkpoint.clone(),
        segments: vec![first_segment.clone(), second_segment.clone()],
    };
    let restored = Simulation::from_checkpoint_journal_with_plugins(bundle, plugins)
        .expect("contiguous evidence segments should restore exact current state");
    assert_eq!(restored.snapshot(), simulation.snapshot());
    let replayed =
        Simulation::replay_from_journal(scenario.clone(), plugins, &restored.replay_journal())
            .expect("checkpoint-journal restoration should retain exact replay evidence");
    assert_eq!(replayed.snapshot(), simulation.snapshot());

    let json = simulation
        .checkpoint_journal_json()
        .expect("a portable checkpoint-journal bundle should serialize");
    let json_restored = Simulation::from_checkpoint_journal_json_with_plugins(&json, plugins)
        .expect("the portable checkpoint-journal bundle should restore");
    assert_eq!(json_restored.snapshot(), simulation.snapshot());
    assert!(
        serde_json::to_vec(&checkpoint)
            .expect("checkpoint should serialize")
            .len()
            < serde_json::to_vec(&simulation.snapshot())
                .expect("flat snapshot should serialize")
                .len(),
        "the current-state checkpoint must not duplicate accumulated evidence",
    );

    let error =
        Simulation::from_checkpoint_and_journal(checkpoint.clone(), vec![second_segment.clone()])
            .err()
            .expect("a journal gap must be rejected");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let error = Simulation::from_checkpoint_and_journal(
        checkpoint.clone(),
        vec![first_segment.clone(), first_segment.clone()],
    )
    .err()
    .expect("a duplicated journal segment must be rejected");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let mut inconsistent_end = second_segment.clone();
    inconsistent_end.end.event_count += 1;
    let error = Simulation::from_checkpoint_and_journal(
        checkpoint.clone(),
        vec![first_segment.clone(), inconsistent_end],
    )
    .err()
    .expect("a forged segment end must be rejected");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let mut tampered_segment = second_segment;
    tampered_segment.events[0].summary.push_str(" (tampered)");
    let error = Simulation::from_checkpoint_and_journal(
        checkpoint.clone(),
        vec![first_segment.clone(), tampered_segment],
    )
    .err()
    .expect("checkpoint roots must reject tampered archived evidence");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let mut duplicated_evidence = checkpoint.clone();
    duplicated_evidence
        .state
        .commands
        .push(first_segment.commands[0].clone());
    let error =
        Simulation::from_checkpoint_and_journal(duplicated_evidence, vec![first_segment.clone()])
            .err()
            .expect("checkpoint state must not duplicate archived evidence");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let mut unsupported = checkpoint.clone();
    unsupported.format_version += 1;
    let error = Simulation::from_checkpoint_and_journal(unsupported, vec![first_segment.clone()])
        .err()
        .expect("unknown checkpoint-journal formats must be rejected");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let mut future = checkpoint.journal_end;
    future.event_count += 1;
    let error = simulation
        .journal_segment_since(future)
        .expect_err("a future journal cursor must be rejected");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let empty = Simulation::new(37, scenario).expect("empty checkpoint fixture should load");
    let empty_bundle = empty
        .checkpoint_journal()
        .expect("an empty run should still checkpoint");
    assert!(empty_bundle.segments.is_empty());
    let empty_restored = Simulation::from_checkpoint_journal(empty_bundle)
        .expect("an empty journal prefix should restore without a synthetic segment");
    assert_eq!(empty_restored.snapshot(), empty.snapshot());
}

#[test]
fn compacted_live_journals_preserve_continuation_idempotency_and_exact_replay() {
    let (scenario, _) = demo_scenario();
    let plugins: &[&dyn SimulationPlugin] = &[&JournalCommandPlugin];
    let command = |request_id, revision| {
        CommandRequest::new(
            CommandRequestId::new(request_id),
            revision,
            CommandEnvelope::new(
                Issuer::Debug,
                Command::Plugin {
                    plugin: "journal-command".to_owned(),
                    command: "noop".to_owned(),
                    payload: Value::Null,
                },
            ),
        )
    };

    let mut simulation =
        Simulation::new(41, scenario.clone()).expect("compact fixture should load");
    simulation
        .register_plugin(&JournalCommandPlugin)
        .expect("compact fixture plugin should register");
    let first_request = command(1, 0);
    let first_ingress = simulation
        .enqueue_command(SimTime::EPOCH, 0, first_request.clone())
        .expect("the first compact fixture command should queue");
    simulation
        .step_canonical()
        .expect("the first compact fixture boundary should settle")
        .expect("queued work should produce a boundary");
    let first_hash = simulation.checkpoint_hash().to_owned();
    let first_cursor = simulation
        .evidence_cursor()
        .expect("the first compact cursor should be representable");

    let mut compact = simulation
        .into_compacted()
        .expect("the complete runtime should enter compact mode");
    let first_segment = compact
        .seal_evidence()
        .expect("the first live tail should seal")
        .expect("the first live tail should contain evidence");
    assert_eq!(first_segment.start, EvidenceCursor::default());
    assert_eq!(first_segment.end, first_cursor);
    assert_eq!(compact.checkpoint_hash(), first_hash);
    assert_eq!(
        compact
            .enqueue_command(SimTime::EPOCH, 0, first_request.clone())
            .expect("an archived ingress retry should remain idempotent"),
        first_ingress
    );

    let second_request = command(2, compact.revision());
    compact
        .enqueue_command(SimTime::EPOCH, 0, second_request)
        .expect("a new request should queue after sealing");
    compact
        .step_canonical()
        .expect("continuation after sealing should settle")
        .expect("the new request should produce a boundary");
    let second_segment = compact
        .seal_evidence()
        .expect("the continuation tail should seal")
        .expect("the continuation tail should contain evidence");
    assert_eq!(second_segment.start, first_cursor);
    assert_eq!(second_segment.end, compact.evidence_cursor().unwrap());
    assert!(
        compact
            .checkpoint()
            .expect("compacted current state should checkpoint")
            .state
            .events
            .is_empty()
    );

    compact
        .schedule_calendar_boundary(SimTime::EPOCH, vec![SystemCadence::Daily])
        .expect("calendar work should remain available after compaction");
    compact
        .step_canonical()
        .expect("calendar continuation should settle")
        .expect("scheduled calendar work should produce a boundary");
    let calendar_segment = compact
        .seal_evidence()
        .expect("calendar continuation evidence should seal")
        .expect("calendar continuation should produce a segment");
    assert_eq!(calendar_segment.start, second_segment.end);

    let segments = vec![
        first_segment.clone(),
        second_segment.clone(),
        calendar_segment.clone(),
    ];
    let snapshot = compact
        .snapshot_with_segments(segments.clone())
        .expect("the external archive should reconstruct a full snapshot");
    let restored = Simulation::from_snapshot_with_plugins(snapshot.clone(), plugins)
        .expect("the reconstructed snapshot should continue with exact plugins");
    assert_eq!(restored.snapshot(), snapshot);
    let replayed = Simulation::replay_from_journal(
        scenario.clone(),
        plugins,
        &compact
            .replay_journal_with_segments(segments.clone())
            .expect("the external archive should produce an exact replay journal"),
    )
    .expect("the compact archive should replay exactly");
    assert_eq!(replayed.snapshot(), snapshot);

    let mut tampered = segments;
    tampered[0].commands[0].envelope.expected_time = Some(SimTime::from_minutes(1));
    let error = compact
        .snapshot_with_segments(tampered)
        .expect_err("tampered sealed evidence must fail checkpoint validation");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let mut emitting =
        Simulation::new(42, scenario.clone()).expect("emitting compact fixture should load");
    emitting
        .register_plugin(&ArchiveEmissionPlugin)
        .expect("emitting compact fixture plugin should register");
    emitting
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily))
        .expect("the emitting boundary should settle");
    let mut emitting = emitting
        .into_compacted()
        .expect("the emitting runtime should enter compact mode");
    let error = emitting
        .seal_evidence()
        .expect_err("new boundary emissions remain pending admission");
    assert_eq!(error.code, ErrorCode::ArchiveNotReady);
    emitting
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("a later boundary should admit the emitted event");
    let first_emitting_segment = emitting
        .seal_evidence()
        .expect("admitted emitting evidence should seal")
        .expect("admitted emitting evidence should produce a segment");
    emitting
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily))
        .expect("an emitting runtime should continue after sealing");
    let error = emitting
        .seal_evidence()
        .expect_err("the new emission should retain the admission frontier");
    assert_eq!(error.code, ErrorCode::ArchiveNotReady);
    emitting
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("the next boundary should admit the second emission");
    let second_emitting_segment = emitting
        .seal_evidence()
        .expect("the second admitted tail should seal")
        .expect("the second admitted tail should produce a segment");
    assert_eq!(second_emitting_segment.start, first_emitting_segment.end);
    emitting
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("post-seal continuation should preserve the event cursor");

    let mut direct = Simulation::new(43, scenario).expect("direct compact fixture should load");
    direct
        .register_plugin(&JournalCommandPlugin)
        .expect("direct compact fixture plugin should register");
    let direct_request = command(11, 0);
    let direct_outcome = direct
        .process_command(direct_request.clone())
        .expect("the direct request should commit");
    let revision = direct.revision();
    let mut direct = direct
        .into_compacted()
        .expect("the direct runtime should enter compact mode");
    let error = direct
        .seal_evidence()
        .expect_err("unsettled command evidence should remain retained");
    assert_eq!(error.code, ErrorCode::ArchiveNotReady);
    assert_eq!(direct.revision(), revision);
    direct
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("the retained direct command should settle");
    direct
        .seal_evidence()
        .expect("settled direct evidence should seal")
        .expect("settled direct evidence should be returned");
    assert_eq!(
        direct
            .process_command(direct_request)
            .expect("an archived direct request retry should stay exact"),
        direct_outcome
    );
    assert_eq!(direct.revision(), revision + 1);
}

#[test]
fn two_phase_archive_sealing_is_immutable_atomic_idempotent_and_tamper_evident() {
    let (scenario, _) = demo_scenario();
    let command = |request_id, revision| {
        CommandRequest::new(
            CommandRequestId::new(request_id),
            revision,
            CommandEnvelope::new(
                Issuer::Debug,
                Command::Plugin {
                    plugin: "journal-command".to_owned(),
                    command: "noop".to_owned(),
                    payload: Value::Null,
                },
            ),
        )
    };
    let mut simulation = Simulation::new(141, scenario.clone()).unwrap();
    simulation.register_plugin(&JournalCommandPlugin).unwrap();
    let empty_wire = serde_json::to_value(simulation.checkpoint().unwrap()).unwrap();
    for field in [
        "archived_segment_manifest_root",
        "archived_receipt_root",
        "evidence_dependencies",
        "evidence_dependency_root",
        "keyed_draw_reservations",
        "keyed_reservation_root",
    ] {
        assert!(
            empty_wire.get(field).is_none(),
            "empty compact continuation must skip {field}"
        );
    }
    simulation
        .enqueue_command(SimTime::EPOCH, 0, command(1, 0))
        .unwrap();
    simulation.step_canonical().unwrap().unwrap();
    let mut compact = simulation.into_compacted().unwrap();
    let before = compact.checkpoint().unwrap();
    assert!(before.archived_segment_manifest_root.is_none());
    assert!(before.archived_receipt_root.is_none());
    assert!(!before.evidence_dependencies.is_empty());
    assert!(before.evidence_dependency_root.is_some());
    assert!(before.keyed_reservation_root.is_none());
    let prepared = compact.prepare_evidence_seal().unwrap().unwrap();
    assert_eq!(compact.checkpoint().unwrap(), before);
    assert_eq!(
        prepared.segment.archive.as_ref().unwrap().header.segment_id,
        prepared.token.segment_id
    );
    assert!(
        !prepared
            .segment
            .archive
            .as_ref()
            .unwrap()
            .entries
            .is_empty()
    );

    let archive = TestArchive::default();
    assert_eq!(
        archive.store_evidence_segment(&prepared.segment).unwrap(),
        ArchiveStoreOutcome::Stored
    );
    assert_eq!(
        archive.store_evidence_segment(&prepared.segment).unwrap(),
        ArchiveStoreOutcome::AlreadyPresent
    );
    let mut conflicting_segment = prepared.segment.clone();
    conflicting_segment.commands[0].accepted_at = SimTime::from_minutes(1);
    let error = archive
        .store_evidence_segment(&conflicting_segment)
        .expect_err("a content-addressed ID cannot be rebound to different bytes");
    assert_eq!(error.code, ErrorCode::InvalidArchive);
    compact
        .commit_evidence_seal(&prepared.token, &archive)
        .unwrap();
    let committed = compact.checkpoint().unwrap();
    assert_eq!(committed.archived_segment_headers.len(), 1);
    assert!(!committed.archived_evidence_receipts.is_empty());
    assert!(committed.archived_segment_manifest_root.is_some());
    assert!(committed.archived_receipt_root.is_some());
    assert!(committed.evidence_dependency_root.is_some());
    assert!(committed.keyed_reservation_root.is_none());
    assert!(
        committed
            .evidence_dependencies
            .iter()
            .all(|dependency| { dependency.requirement == EvidenceRequirement::IdentityOnly })
    );
    compact
        .commit_evidence_seal(&prepared.token, &archive)
        .expect("repeating the committed token must be idempotent");

    let mut corrupt_provider_segment = prepared.segment.clone();
    corrupt_provider_segment.commands[0].accepted_at = SimTime::from_minutes(1);
    let corrupt_provider = TestArchive::default();
    corrupt_provider
        .segments
        .borrow_mut()
        .insert(prepared.token.segment_id.clone(), corrupt_provider_segment);
    let error = compact
        .load_archived_evidence_segment(
            &committed.archived_evidence_receipts[0].evidence,
            &corrupt_provider,
        )
        .expect_err("provider bytes must reproduce the committed segment index");
    assert_eq!(error.code, ErrorCode::InvalidArchive);

    let mut tampered_checkpoint = committed.clone();
    tampered_checkpoint.archived_evidence_receipts[0]
        .item_commitment
        .replace_range(..1, "f");
    let Err(error) = Simulation::from_checkpoint_and_journal(
        tampered_checkpoint,
        vec![prepared.segment.clone()],
    ) else {
        panic!("a tampered compact receipt must not reconstruct");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let mut tampered_manifest_root = committed.clone();
    tampered_manifest_root.archived_segment_manifest_root = Some("0".repeat(64));
    let Err(error) = Simulation::from_checkpoint_and_journal(
        tampered_manifest_root,
        vec![prepared.segment.clone()],
    ) else {
        panic!("the archived-segment manifest root must be committed")
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let mut tampered_receipt_root = committed.clone();
    tampered_receipt_root.archived_receipt_root = Some("0".repeat(64));
    let Err(error) = Simulation::from_checkpoint_and_journal(
        tampered_receipt_root,
        vec![prepared.segment.clone()],
    ) else {
        panic!("the archived-receipt root must be committed")
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let mut missing_dependency_root = committed.clone();
    missing_dependency_root.evidence_dependency_root = None;
    let Err(error) = Simulation::from_checkpoint_and_journal(
        missing_dependency_root,
        vec![prepared.segment.clone()],
    ) else {
        panic!("a non-empty dependency vector cannot omit its root")
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let mut payload_dependency = committed.clone();
    payload_dependency.evidence_dependencies[0].requirement = EvidenceRequirement::PayloadRequired;
    payload_dependency.evidence_dependency_root = Some(
        canonical_hash(
            "canwu.evidence.dependencies.v1",
            &payload_dependency.evidence_dependencies,
        )
        .unwrap(),
    );
    let Err(error) =
        Simulation::from_checkpoint_and_journal(payload_dependency, vec![prepared.segment.clone()])
    else {
        panic!("payload requirements need an authoritative pending schema")
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let mut reordered_dependencies = committed.clone();
    reordered_dependencies.evidence_dependencies.reverse();
    reordered_dependencies.evidence_dependency_root = Some(
        canonical_hash(
            "canwu.evidence.dependencies.v1",
            &reordered_dependencies.evidence_dependencies,
        )
        .unwrap(),
    );
    let Err(error) = Simulation::from_checkpoint_and_journal(
        reordered_dependencies,
        vec![prepared.segment.clone()],
    ) else {
        panic!("dependency ordering is part of compact continuation")
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    compact
        .enqueue_command(SimTime::EPOCH, 0, command(2, compact.revision()))
        .unwrap();
    compact.step_canonical().unwrap().unwrap();
    let stale = compact.prepare_evidence_seal().unwrap().unwrap();
    compact
        .enqueue_command(SimTime::EPOCH, 0, command(3, compact.revision()))
        .unwrap();
    compact.step_canonical().unwrap().unwrap();
    let stale_archive = TestArchive::default();
    stale_archive
        .store_evidence_segment(&stale.segment)
        .unwrap();
    let error = compact
        .commit_evidence_seal(&stale.token, &stale_archive)
        .expect_err("a changed live cut must reject the prepared token");
    assert_eq!(error.code, ErrorCode::StaleSealToken);

    let fresh = compact.prepare_evidence_seal().unwrap().unwrap();
    let missing_archive = TestArchive::default();
    let before_missing = compact.checkpoint().unwrap();
    let error = compact
        .commit_evidence_seal(&fresh.token, &missing_archive)
        .expect_err("commit must read the stored segment back");
    assert_eq!(error.code, ErrorCode::ArchiveNotReady);
    assert_eq!(compact.checkpoint().unwrap(), before_missing);
}

#[test]
fn archived_segment_receipts_require_provider_for_payload_reads() {
    let (scenario, _) = demo_scenario();
    let mut simulation = Simulation::new(142, scenario).unwrap();
    simulation
        .register_plugin(&ArchivedEvidencePublicationPlugin)
        .unwrap();
    simulation
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily))
        .unwrap();
    simulation
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily))
        .unwrap();
    let mut segment = simulation
        .journal_segment_since(EvidenceCursor::default())
        .expect("the complete retained segment should materialize");
    let (archive_index, receipts) = persistence::evidence_archive_index(&segment)
        .expect("the segment should produce a verified evidence index");
    segment.archive = Some(archive_index);
    let boundary_evidence = EvidenceRef::Boundary(BoundaryId::new(1));
    let boundary_receipt = receipts
        .into_iter()
        .find(|receipt| receipt.evidence == boundary_evidence)
        .expect("the complete segment index should contain the boundary identity");

    let missing = TestArchive::default();
    let error = persistence::load_verified_archived_evidence_segment(&boundary_receipt, &missing)
        .expect_err("identity receipt must not fabricate archived payload bytes");
    assert_eq!(error.code, ErrorCode::EvidenceContentUnavailable);
    missing.store_evidence_segment(&segment).unwrap();
    assert_eq!(
        persistence::load_verified_archived_evidence_segment(&boundary_receipt, &missing).unwrap(),
        segment
    );
}

#[test]
fn repeated_keyed_seals_restore_sorted_reservations_and_exact_dependencies() {
    let (_, _, simulation, _) = run_keyed_fixture(&["zeta"]);
    let mut compact = simulation.into_compacted().unwrap();
    let first = compact
        .seal_evidence()
        .expect("the first keyed evidence tail should seal")
        .expect("the first keyed evidence tail should be non-empty");

    compact
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            KeyedRandomPlugin.name(),
            "operation",
            SimTime::EPOCH,
            serde_json::json!({ "operation": "alpha" }),
        ))
        .unwrap();
    compact
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("the second keyed operation should settle");
    let second = compact
        .seal_evidence()
        .expect("the second keyed evidence tail should seal")
        .expect("the second keyed evidence tail should be non-empty");
    assert_eq!(second.start, first.end);

    let checkpoint = compact.checkpoint().unwrap();
    assert_eq!(checkpoint.archived_segment_headers.len(), 2);
    assert_eq!(checkpoint.keyed_draw_reservations.len(), 2);
    assert!(checkpoint.keyed_reservation_root.is_some());
    assert!(checkpoint.evidence_dependency_root.is_some());
    assert!(
        checkpoint
            .keyed_draw_reservations
            .windows(2)
            .all(|reservations| {
                (&reservations[0].stream, &reservations[0].address)
                    < (&reservations[1].stream, &reservations[1].address)
            })
    );
    for reservation in &checkpoint.keyed_draw_reservations {
        assert!(checkpoint.evidence_dependencies.iter().any(|dependency| {
            dependency.reference == reservation.operation_evidence
                && dependency.requirement == EvidenceRequirement::IdentityOnly
        }));
        assert!(checkpoint.evidence_dependencies.iter().any(|dependency| {
            dependency.reference == reservation.draw_receipt.evidence
                && dependency.requirement == EvidenceRequirement::IdentityOnly
        }));
    }

    let restored = CompactedSimulation::from_checkpoint_and_journal_with_plugins(
        checkpoint.clone(),
        vec![first.clone(), second.clone()],
        &[&KeyedRandomPlugin],
    )
    .expect("repeated keyed segments should reconstruct exactly");
    let restored_snapshot = restored
        .snapshot_with_segments(Vec::new())
        .expect("repeated keyed segments should reconstruct a full snapshot");
    assert_eq!(restored_snapshot.random_draws.len(), 4);

    let alpha = restored_snapshot
        .random_draws
        .iter()
        .find(|draw| {
            matches!(
                &draw.address,
                RandomDrawAddress::OperationV1(address)
                    if address.application_operation_id == "alpha"
            )
        })
        .expect("the restored alpha reservation should have a draw");
    let RandomDrawAddress::OperationV1(alpha_address) = &alpha.address else {
        unreachable!("the selected draw is operation-keyed")
    };
    let keyed = random::retained_keyed_draws(&restored_snapshot.random_draws)
        .expect("restored draws should rebuild the keyed index");
    let available = restored_snapshot
        .random_streams
        .iter()
        .cloned()
        .map(|stream| (stream.key.clone(), stream))
        .collect::<BTreeMap<_, _>>();
    let mut retry_session = random::RandomSession::new(
        &available,
        std::slice::from_ref(&alpha.stream),
        restored_snapshot.root_seed,
        KeyedRandomPlugin.name(),
        &keyed,
    )
    .expect("restored keyed state should open a random session");
    assert_eq!(
        retry_session
            .range_for_operation(
                &alpha.stream,
                alpha
                    .operation_evidence
                    .clone()
                    .expect("operation draw should retain exact evidence"),
                &alpha_address.operation_kind,
                &alpha_address.application_operation_id,
                alpha_address.target.clone(),
                alpha_address.draw_slot,
                alpha.upper_exclusive,
                &alpha.purpose,
            )
            .expect("an exact retry should survive repeated seal and restore"),
        alpha.value
    );
    let conflict = retry_session
        .range_for_operation(
            &alpha.stream,
            EvidenceRef::Ingress(IngressId::new(99)),
            &alpha_address.operation_kind,
            &alpha_address.application_operation_id,
            alpha_address.target.clone(),
            alpha_address.draw_slot,
            alpha.upper_exclusive,
            &alpha.purpose,
        )
        .expect_err("the same restored entropy address with different evidence must conflict");
    assert_eq!(conflict.code, ErrorCode::RandomOperationConflict);
    assert!(retry_session.finish().draws.is_empty());

    let mut tampered = checkpoint;
    tampered.keyed_draw_reservations.swap(0, 1);
    tampered.keyed_reservation_root = Some(
        canonical_hash(
            "canwu.random.keyed-reservations.v1",
            &tampered.keyed_draw_reservations,
        )
        .unwrap(),
    );
    let Err(error) = Simulation::from_checkpoint_and_journal(tampered, vec![first, second]) else {
        panic!("reservation ordering is part of compact continuation")
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
}

#[test]
fn simulation_view_resolves_retained_commands_and_events_by_id() {
    let (scenario, ids) = demo_scenario();
    let mut simulation = Simulation::new(44, scenario).expect("lookup fixture should load");
    let debug_command = |morale| {
        CommandEnvelope::new(
            Issuer::Debug,
            Command::DebugSetArmyMorale {
                army: ids.army,
                morale,
            },
        )
    };
    let first = simulation
        .submit(debug_command(61))
        .expect("the first lookup command should commit");
    let second = simulation
        .submit(debug_command(62))
        .expect("the second lookup command should commit");
    let reads = [StateKey::core_commands(), StateKey::core_events()];
    let view = simulation.plugin_view("lookup", &reads);

    assert_eq!(
        view.command(first.command_id).unwrap().unwrap().id,
        first.command_id
    );
    assert_eq!(
        view.command(second.command_id).unwrap().unwrap().id,
        second.command_id
    );
    let first_event = *first
        .emitted_events
        .first()
        .expect("the first command should emit an event");
    let second_event = *second
        .emitted_events
        .first()
        .expect("the second command should emit an event");
    assert_eq!(view.event(first_event).unwrap().unwrap().id, first_event);
    assert_eq!(view.event(second_event).unwrap().unwrap().id, second_event);
    assert!(view.command(CommandId::new(0)).unwrap().is_none());
    assert!(view.event(EventId::new(0)).unwrap().is_none());
    assert!(view.command(CommandId::new(3)).unwrap().is_none());
    assert!(view.event(EventId::new(3)).unwrap().is_none());
}

#[test]
fn simulation_view_excludes_archived_ids_after_compaction() {
    let (scenario, ids) = demo_scenario();
    let mut simulation = Simulation::new(45, scenario).expect("archive lookup fixture should load");
    let debug_command = |morale| {
        CommandEnvelope::new(
            Issuer::Debug,
            Command::DebugSetArmyMorale {
                army: ids.army,
                morale,
            },
        )
    };
    let archived = simulation
        .submit(debug_command(63))
        .expect("the archived lookup command should commit");
    let retained = simulation
        .submit(debug_command(64))
        .expect("the retained lookup command should commit");
    let retained_command = simulation
        .state
        .evidence
        .commands
        .pop()
        .expect("the retained command should be in the live tail");
    let retained_event = simulation
        .state
        .evidence
        .events
        .pop()
        .expect("the retained event should be in the live tail");
    simulation.state.evidence.archived.command_count = 1;
    simulation.state.evidence.archived.event_count = 1;
    simulation.state.evidence.commands.clear();
    simulation.state.evidence.events.clear();
    simulation.state.evidence.commands.push(retained_command);
    simulation.state.evidence.events.push(retained_event);
    let reads = [StateKey::core_commands(), StateKey::core_events()];
    let view = simulation.plugin_view("lookup", &reads);

    assert!(view.command(archived.command_id).unwrap().is_none());
    assert!(view.event(archived.emitted_events[0]).unwrap().is_none());
    assert_eq!(
        view.command(retained.command_id).unwrap().unwrap().id,
        retained.command_id
    );
    assert_eq!(
        view.event(retained.emitted_events[0]).unwrap().unwrap().id,
        retained.emitted_events[0]
    );
    assert!(view.command(CommandId::new(3)).unwrap().is_none());
    assert!(view.event(EventId::new(3)).unwrap().is_none());
}

#[test]
fn runtime_and_snapshot_validation_contexts_share_cause_and_directive_rules() {
    let (scenario, _) = demo_scenario();
    let simulation = Simulation::new(46, scenario).expect("validation fixture should load");
    let snapshot = simulation.snapshot();
    let runtime_context = validation::RuntimeValidationContext::new(&simulation.state);
    let snapshot_context = validation::SnapshotValidationContext::new(&snapshot);
    let missing_cause = CauseRef::Event(EventId::new(1));

    assert!(validation::validate_cause_reference(&runtime_context, &missing_cause).is_err());
    assert!(validation::validate_cause_reference(&snapshot_context, &missing_cause).is_err());

    let directives = [SystemDirective::Emit {
        event_type: " marker ".to_owned(),
        summary: "invalid event type".to_owned(),
        affected: Vec::new(),
    }];
    let runtime_error = validation::validate_directives_with_context(
        &runtime_context,
        "fixture",
        &[],
        &BTreeMap::new(),
        &BTreeMap::new(),
        &directives,
    )
    .expect_err("runtime validation must reject a non-canonical directive");
    let snapshot_error = validation::validate_directives_with_context(
        &snapshot_context,
        "fixture",
        &[],
        &BTreeMap::new(),
        &BTreeMap::new(),
        &directives,
    )
    .expect_err("snapshot validation must reject a non-canonical directive");
    assert_eq!(runtime_error.code, ErrorCode::InvalidPayload);
    assert_eq!(snapshot_error.code, runtime_error.code);
    assert_eq!(snapshot_error.message, runtime_error.message);
}

#[test]
fn snapshot_round_trip_preserves_pending_work() {
    let (mut simulation, ids) = Simulation::demo(35).expect("demo should load");
    simulation
        .submit(move_order(&ids))
        .expect("order should validate");
    let json = simulation
        .snapshot_json()
        .expect("snapshot should serialize");
    let mut unsupported = simulation.snapshot();
    assert_eq!(unsupported.engine_version, ENGINE_VERSION);
    assert_eq!(unsupported.snapshot_format_version, SNAPSHOT_FORMAT_VERSION);
    unsupported.snapshot_format_version += 1;
    let Err(error) = Simulation::from_snapshot(unsupported) else {
        panic!("unknown snapshot formats must be rejected");
    };
    assert_eq!(error.code, ErrorCode::UnsupportedSnapshotVersion);

    let mut unmigrated_engine = simulation.snapshot();
    unmigrated_engine.engine_version = "0.4.0-other".to_owned();
    refresh_snapshot_commitments_and_checkpoint(&mut unmigrated_engine);
    let Err(error) = Simulation::from_snapshot(unmigrated_engine) else {
        panic!("current-format snapshots from another engine must require migration");
    };
    assert_eq!(error.code, ErrorCode::UnsupportedSnapshotVersion);

    let mut legacy_value =
        serde_json::to_value(simulation.snapshot()).expect("snapshot should convert to JSON value");
    let legacy_object = legacy_value
        .as_object_mut()
        .expect("snapshot JSON should be an object");
    legacy_object.insert("snapshot_format_version".to_owned(), Value::from(2));
    legacy_object.remove("run_manifest");
    legacy_object.remove("run_manifest_hash");
    legacy_object.remove("run_configuration");
    legacy_object.remove("checkpoint_hash");
    legacy_object.remove("commitment_format_version");
    legacy_object.remove("commitment_roots");
    legacy_object.remove("revision_format_version");
    legacy_object.remove("state_revision");
    legacy_object.remove("replay_revision_format_version");
    legacy_object.remove("admission_cursor_format_version");
    legacy_object.remove("admitted_attempt_count");
    legacy_object.remove("admitted_command_count");
    legacy_object.remove("admitted_event_count");
    legacy_object.remove("boundaries");
    legacy_object.remove("next_boundary_id");
    legacy_object.remove("root_seed");
    legacy_object.remove("random_streams");
    legacy_object.remove("random_draws");
    legacy_object.remove("next_random_draw_id");
    legacy_object.insert(
        "rng".to_owned(),
        serde_json::to_value(DeterministicRng::from_seed(35))
            .expect("legacy RNG fixture should serialize"),
    );
    let legacy_json =
        serde_json::to_string(&legacy_value).expect("legacy snapshot fixture should serialize");
    let Err(error) = Simulation::from_snapshot_json(&legacy_json) else {
        panic!("format 2 snapshots must first migrate through the 0.4 runtime");
    };
    assert_eq!(error.code, ErrorCode::UnsupportedSnapshotVersion);

    let mut restored = Simulation::from_snapshot_json(&json).expect("snapshot should restore");
    restored
        .advance(SimDuration::days(1))
        .expect("pending arrival should execute");
    assert_eq!(
        restored
            .world()
            .army(ids.army)
            .expect("army exists")
            .location,
        ids.eastern_territory
    );
    let mut changed_delivery = restored.snapshot();
    let mut changed_dispatch = None;
    for event in &mut changed_delivery.events {
        if event.kind.is_type("report_dispatched") {
            let changed_arrival = event
                .kind
                .decode_field::<SimTime>("arrives_at")
                .expect("report dispatch should have an arrival time")
                + SimDuration::minutes(1);
            event
                .kind
                .set_field("arrives_at", &changed_arrival)
                .expect("report dispatch should have an arrival time");
            changed_dispatch = Some((event.id, changed_arrival));
            break;
        }
    }
    let (dispatch_event, changed_arrival) =
        changed_dispatch.expect("arrival should dispatch an observer report");
    let scheduled = changed_delivery
        .scheduled
        .iter_mut()
        .find(|record| {
            matches!(
                record.action,
                ScheduledAction::KnowledgeReport {
                    dispatch_event: candidate,
                    ..
                } if candidate == dispatch_event
            )
        })
        .expect("the dispatched report should remain pending");
    scheduled.key.at = changed_arrival;
    refresh_snapshot_commitments_and_checkpoint(&mut changed_delivery);
    let Err(error) = Simulation::from_snapshot(changed_delivery) else {
        panic!("report timing must remain tied to its recorded random draw");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    assert!(error.message.contains("random draw"));

    let mut missing_draw = restored.snapshot();
    missing_draw.random_draws.clear();
    let core_stream = missing_draw
        .random_streams
        .iter_mut()
        .find(|state| state.key == random::core_report_delay_stream())
        .expect("the core report-delay stream should be persisted");
    core_stream.position = 0;
    core_stream.generator_state = core_stream.seed;
    missing_draw.next_random_draw_id = 1;
    refresh_snapshot_commitments_and_checkpoint(&mut missing_draw);
    let Err(error) = Simulation::from_snapshot(missing_draw) else {
        panic!("every report dispatch must retain its generating random draw");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    assert!(error.message.contains("core random draw"));

    let mut malformed_legacy =
        serde_json::to_value(restored.snapshot()).expect("snapshot should convert to JSON");
    let malformed_object = malformed_legacy
        .as_object_mut()
        .expect("snapshot JSON should be an object");
    malformed_object.insert("snapshot_format_version".to_owned(), Value::from(3));
    malformed_object.remove("run_manifest");
    malformed_object.remove("run_manifest_hash");
    malformed_object.remove("run_configuration");
    malformed_object.remove("checkpoint_hash");
    malformed_object.remove("commitment_format_version");
    malformed_object.remove("commitment_roots");
    malformed_object.remove("revision_format_version");
    malformed_object.remove("state_revision");
    malformed_object.remove("replay_revision_format_version");
    malformed_object.remove("admission_cursor_format_version");
    malformed_object.remove("admitted_attempt_count");
    malformed_object.remove("admitted_command_count");
    malformed_object.remove("admitted_event_count");
    malformed_object.remove("root_seed");
    malformed_object.remove("random_streams");
    malformed_object.remove("random_draws");
    malformed_object.remove("next_random_draw_id");
    malformed_object.insert(
        "rng".to_owned(),
        serde_json::to_value(DeterministicRng::from_seed(35))
            .expect("legacy RNG fixture should serialize"),
    );
    let malformed_dispatch = malformed_object
        .get_mut("events")
        .and_then(Value::as_array_mut)
        .and_then(|events| {
            events.iter_mut().find(|event| {
                event
                    .get("kind")
                    .and_then(|kind| kind.get("type"))
                    .and_then(Value::as_str)
                    == Some("report_dispatched")
            })
        })
        .expect("the legacy fixture should contain a report dispatch");
    malformed_dispatch["timestamp"] = Value::from(i64::MAX);
    malformed_dispatch["kind"]["arrives_at"] = Value::from(i64::MIN);
    let malformed_json = serde_json::to_string(&malformed_legacy)
        .expect("malformed legacy fixture should still serialize");
    let Err(error) = Simulation::from_snapshot_json(&malformed_json) else {
        panic!("legacy report-time overflow must return a structured error");
    };
    assert_eq!(error.code, ErrorCode::UnsupportedSnapshotVersion);

    let report_pending = restored
        .snapshot_json()
        .expect("pending reports should serialize");
    let mut report_restored = Simulation::from_snapshot_json(&report_pending)
        .expect("pending report evidence should restore");
    report_restored
        .advance(SimDuration::days(3))
        .expect("pending reports should be delivered");
    let delivered = report_restored
        .snapshot_json()
        .expect("delivered reports should serialize");
    Simulation::from_snapshot_json(&delivered)
        .expect("completed report evidence should restore without pending work");
}

#[test]
fn snapshot_rejects_extra_fields_on_compatibility_event_payloads() {
    let (mut simulation, ids) = Simulation::demo(35).expect("demo should load");
    simulation
        .submit(CommandEnvelope::new(
            Issuer::Debug,
            Command::DebugSetArmyMorale {
                army: ids.army,
                morale: 61,
            },
        ))
        .expect("debug command should emit a compatibility event");
    let mut snapshot = simulation.snapshot();
    let event = snapshot
        .events
        .iter_mut()
        .find(|event| event.kind.is_type("debug_field_changed"))
        .expect("debug event should exist");
    let mut kind = serde_json::to_value(&event.kind).expect("event should serialize");
    kind.as_object_mut()
        .expect("event kind should be an object")
        .insert("extra".to_owned(), Value::Bool(true));
    event.kind = serde_json::from_value(kind).expect("generic event should retain extra fields");
    refresh_snapshot_commitments_and_checkpoint(&mut snapshot);

    let error = Simulation::from_snapshot(snapshot)
        .err()
        .expect("typed compatibility payloads must reject extra fields");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    assert!(error.message.contains("event payload is not canonical"));
}

#[test]
fn pre_policy_format_four_journals_hydrate_compatibility_provenance() {
    let (scenario, ids) = demo_scenario();
    let mut simulation =
        Simulation::new(73, scenario.clone()).expect("compatibility run should load");
    simulation
        .submit(move_order(&ids))
        .expect("legacy command fixture should be accepted");
    let mut value =
        serde_json::to_value(simulation.replay_journal()).expect("journal should become JSON");
    let object = value
        .as_object_mut()
        .expect("replay journal JSON should be an object");
    object.remove("run_configuration");
    object.remove("command_attempts");
    let hydrated: ReplayJournal =
        serde_json::from_value(value).expect("pre-policy journal should deserialize");
    assert_eq!(
        hydrated.run_configuration,
        RunConfigurationSnapshot::CompatibilityV1
    );
    assert!(hydrated.command_attempts.is_empty());
    let replayed = Simulation::replay_from_journal(scenario.clone(), &[], &hydrated)
        .expect("pre-policy compatibility journal should replay exactly");
    assert_eq!(simulation.snapshot(), replayed.snapshot());

    let mut aliased = Simulation::new(74, scenario)
        .expect("compatibility run should load")
        .snapshot();
    aliased.run_configuration = Some(RunConfigurationSnapshot::ManifestOnlyV1);
    assert_eq!(
        snapshot_checkpoint_hash(&aliased)
            .expect("the provenance alias should remain checkpoint-neutral"),
        aliased.checkpoint_hash
    );
    let Err(error) = Simulation::from_snapshot(aliased) else {
        panic!("default run identity must have exactly one policy provenance");
    };
    assert_eq!(error.code, ErrorCode::InvalidRunManifest);
}

#[test]
fn pre_policy_format_four_custom_run_identity_remains_loadable() {
    let (scenario, _) = demo_scenario();
    let mut legacy = Simulation::new(73, scenario.clone())
        .expect("compatibility run should load")
        .snapshot();
    let scenario_manifest = ArtifactManifest::for_scenario("legacy", "scenario", "1", &scenario)
        .expect("scenario should hash");
    let run_configuration =
        ArtifactManifest::from_bytes("legacy", "custom-run-policy", "7", b"opaque-policy")
            .expect("legacy policy identity should hash");
    let run_manifest = RunManifest::declared(scenario_manifest, run_configuration);
    legacy.run_manifest_hash = manifest::hash(&run_manifest).expect("manifest should hash");
    legacy.run_manifest = Some(run_manifest);
    legacy.run_configuration = Some(RunConfigurationSnapshot::ManifestOnlyV1);
    refresh_snapshot_commitments_and_checkpoint(&mut legacy);
    let expected = legacy.clone();

    let mut value = serde_json::to_value(legacy).expect("snapshot should become JSON");
    value
        .as_object_mut()
        .expect("snapshot JSON should be an object")
        .remove("run_configuration");
    let json = serde_json::to_string(&value).expect("legacy snapshot should serialize");
    let restored = Simulation::from_snapshot_json(&json)
        .expect("custom pre-policy format-4 identity should hydrate explicitly");
    assert_eq!(
        restored.run_configuration(),
        &RunConfigurationSnapshot::ManifestOnlyV1
    );
    assert_eq!(restored.snapshot(), expected);
    let journal = restored.replay_journal();
    let mut journal_value =
        serde_json::to_value(&journal).expect("custom journal should become JSON");
    let journal_object = journal_value
        .as_object_mut()
        .expect("custom journal JSON should be an object");
    journal_object.remove("run_configuration");
    journal_object.remove("command_attempts");
    let hydrated_journal: ReplayJournal = serde_json::from_value(journal_value)
        .expect("custom pre-policy journal should deserialize");
    assert_eq!(
        hydrated_journal.run_configuration,
        RunConfigurationSnapshot::ManifestOnlyV1
    );
    let replayed = Simulation::replay_from_journal(scenario, &[], &hydrated_journal)
        .expect("manifest-only format-4 evidence should remain exactly replayable");
    assert_eq!(restored.snapshot(), replayed.snapshot());
}

#[test]
fn persistence_boundaries_reject_unloadable_or_noncanonical_state() {
    let (mut in_flight, in_flight_ids) = demo_scenario();
    in_flight.world.armies[0].transit = Some(TransitState {
        from: in_flight_ids.central_territory,
        to: in_flight_ids.eastern_territory,
        departed_at: in_flight.start_time,
        arrives_at: in_flight.start_time + SimDuration::days(1),
    });
    let Err(error) = Simulation::new(35, in_flight) else {
        panic!("initial transit without queue evidence must be rejected");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let (mut non_finite, _) = demo_scenario();
    non_finite.world.territories[0].position.x = f32::NAN;
    let Err(error) = Simulation::new(35, non_finite) else {
        panic!("non-finite map coordinates must be rejected");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let (mut simulation, ids) = Simulation::demo(35).expect("demo should load");
    simulation
        .submit(move_order(&ids))
        .expect("order should validate");
    let valid = simulation.snapshot();

    let mut past_schedule = valid.clone();
    past_schedule.scheduled[0].key.at = SimTime::from_minutes(past_schedule.now.as_minutes() - 1);
    let Err(error) = Simulation::from_snapshot(past_schedule) else {
        panic!("past scheduled work must be rejected");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let mut duplicate_arrival = valid.clone();
    let mut second_arrival = duplicate_arrival.scheduled[0].clone();
    second_arrival.key.sequence = duplicate_arrival.next_schedule_sequence;
    duplicate_arrival.next_schedule_sequence += 1;
    duplicate_arrival.scheduled.push(second_arrival);
    let Err(error) = Simulation::from_snapshot(duplicate_arrival) else {
        panic!("duplicate logical arrivals must be rejected");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let mut mismatched_arrival = valid.clone();
    mismatched_arrival.scheduled[0].key.at += SimDuration::minutes(1);
    let Err(error) = Simulation::from_snapshot(mismatched_arrival) else {
        panic!("arrival queue time must match transit and order evidence");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let mut stuck_transit = valid.clone();
    stuck_transit.scheduled.clear();
    let Err(error) = Simulation::from_snapshot(stuck_transit) else {
        panic!("an in-transit army must retain exactly one arrival action");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let mut reopened_registration = valid.clone();
    reopened_registration.plugin_registration_closed = false;
    let Err(error) = Simulation::from_snapshot(reopened_registration) else {
        panic!("executed snapshots must not reopen plugin registration");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let mut stale_counter = valid.clone();
    stale_counter.next_event_id = stale_counter
        .events
        .last()
        .expect("movement emitted an event")
        .id
        .get();
    let Err(error) = Simulation::from_snapshot(stale_counter) else {
        panic!("stale counters must be rejected");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let mut broken_reference = valid;
    broken_reference.world.armies[0].commander = PersonId::new(999);
    let Err(error) = Simulation::from_snapshot(broken_reference) else {
        panic!("broken entity references must be rejected");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let mut exhausted_counter = simulation.snapshot();
    exhausted_counter.next_command_id = u64::MAX;
    refresh_snapshot_commitments_and_checkpoint(&mut exhausted_counter);
    let mut restored =
        Simulation::from_snapshot(exhausted_counter).expect("the exhausted sentinel is valid");
    let before = restored.snapshot();
    let error = restored
        .submit(CommandEnvelope::new(
            Issuer::Debug,
            Command::DebugSetArmyMorale {
                army: ids.army,
                morale: 50,
            },
        ))
        .expect_err("counter exhaustion must be a structured failure");
    assert_eq!(error.code, ErrorCode::IdentifierExhausted);
    assert_eq!(before, restored.snapshot());
}
