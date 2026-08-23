use super::*;

#[test]
fn deterministic_seed_and_event_order_survive_equal_runs() {
    let (scenario, ids) = demo_scenario();
    let mut first = Simulation::new(35, scenario.clone()).expect("demo should load");
    first
        .submit(move_order(&ids))
        .expect("order should validate");
    first
        .advance(SimDuration::days(4))
        .expect("time should advance");
    let second = Simulation::replay(35, scenario, first.command_log(), first.time())
        .expect("journal should replay");
    assert_eq!(first.snapshot(), second.snapshot());
}

#[test]
fn typed_ingress_is_idempotent_revision_guarded_and_replayable() {
    let (scenario, ids) = demo_scenario();
    let configuration = RunConfiguration::play_as_character(
        "seat.commander",
        "controller.human",
        ids.commander,
        "permission.military-command",
    );
    let manifest = manifest_for_configuration(&scenario, &configuration);
    let mut simulation = Simulation::new_with_run_configuration(
        35,
        scenario.clone(),
        manifest.clone(),
        configuration.clone(),
    )
    .expect("declared character run should load");
    let envelope = CommandEnvelope::new(
        Issuer::Human("controller.human".to_owned()),
        Command::OrderMovement {
            subject: EntityRef::Army(ids.army),
            destination: ids.eastern_territory,
            cargo: Vec::new(),
        },
    )
    .with_authority(character_authority(
        ids.commander,
        ids.army,
        "seat.commander",
        "permission.military-command",
    ))
    .at_time(SimTime::EPOCH);
    let request = CommandRequest::new(CommandRequestId::new(1), 0, envelope.clone());
    let accepted = simulation
        .process_command(request.clone())
        .expect("typed request should produce an outcome");
    let CommandOutcome::Accepted { receipt } = &accepted else {
        panic!("matching controller and seat should be accepted");
    };
    assert_eq!(receipt.attempt_id, Some(CommandAttemptId::new(1)));
    assert_eq!(receipt.command_id, CommandId::new(1));
    assert_eq!(receipt.request_id, Some(CommandRequestId::new(1)));
    assert_eq!(receipt.revision, 1);
    assert_eq!(simulation.revision(), 1);

    let after_accept = simulation.snapshot();
    assert_eq!(
        simulation
            .process_command(request)
            .expect("an exact retry should be served from idempotency evidence"),
        accepted
    );
    assert_eq!(simulation.snapshot(), after_accept);

    let mut collision_envelope = envelope.clone();
    collision_envelope.command = Command::OrderMovement {
        subject: EntityRef::Army(ids.army),
        destination: ids.western_territory,
        cargo: Vec::new(),
    };
    let collision = simulation
        .process_command(CommandRequest::new(
            CommandRequestId::new(1),
            1,
            collision_envelope,
        ))
        .expect("request-ID collision should be a structured outcome");
    let CommandOutcome::Rejected { rejection } = collision else {
        panic!("request-ID reuse with different input must be rejected");
    };
    assert_eq!(rejection.attempt_id, None);
    assert_eq!(rejection.retained_revision, 1);
    assert_eq!(rejection.error.code, ErrorCode::IdempotencyConflict);
    assert_eq!(simulation.snapshot(), after_accept);

    let stale_request = CommandRequest::new(CommandRequestId::new(2), 0, envelope);
    let stale = simulation
        .process_command(stale_request.clone())
        .expect("a stale request should remain structured evidence");
    let CommandOutcome::Rejected { rejection } = &stale else {
        panic!("stale revisions must be rejected");
    };
    assert_eq!(rejection.attempt_id, Some(CommandAttemptId::new(2)));
    assert_eq!(rejection.retained_revision, 2);
    assert_eq!(rejection.error.code, ErrorCode::SimulationRevisionConflict);
    assert_eq!(simulation.revision(), 2);
    assert_eq!(simulation.command_log().len(), 1);
    assert_eq!(simulation.command_attempts().len(), 2);

    let after_stale = simulation.snapshot();
    assert_eq!(
        simulation
            .process_command(stale_request)
            .expect("an exact rejected retry should be cached"),
        stale
    );
    assert_eq!(simulation.snapshot(), after_stale);
    let restored = Simulation::from_snapshot(after_stale.clone())
        .expect("typed ingress evidence should survive save/load");
    assert_eq!(restored.snapshot(), after_stale);

    let mut cyclic_cause = after_stale.clone();
    let event_id = cyclic_cause.events[0].id;
    cyclic_cause.events[0].cause = Some(CauseRef::Event(event_id));
    refresh_snapshot_commitments_and_checkpoint(&mut cyclic_cause);
    let Err(error) = Simulation::from_snapshot(cyclic_cause) else {
        panic!("event cause cycles must be rejected without unbounded traversal");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    assert!(error.message.contains("parent event"));

    let mut forged = after_stale;
    forged.command_attempts[0].envelope.issuer = Issuer::Human("controller.other".to_owned());
    forged.commands[0].envelope = forged.command_attempts[0].envelope.clone();
    refresh_snapshot_commitments_and_checkpoint(&mut forged);
    let Err(error) = Simulation::from_snapshot(forged) else {
        panic!("accepted attempts that violate recorded policy must not load");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    assert!(error.message.contains("ingress policy"));

    simulation
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("attempt evidence should enter the next boundary");
    let boundary = simulation
        .boundaries()
        .last()
        .expect("the boundary should be recorded");
    assert_eq!(
        boundary.admitted_attempts,
        vec![CommandAttemptId::new(1), CommandAttemptId::new(2)]
    );
    assert_eq!(boundary.admitted_commands, vec![CommandId::new(1)]);
    let journal = simulation.replay_journal();
    let replayed_fixture = Simulation::replay_with_run_configuration(
        35,
        scenario.clone(),
        manifest,
        configuration,
        &[],
        simulation.command_log(),
        simulation.command_attempts(),
        simulation.boundaries(),
        simulation.time(),
    )
    .expect("declared caller-supplied request journal should replay");
    assert_eq!(simulation.snapshot(), replayed_fixture.snapshot());
    let replayed = Simulation::replay_from_journal(scenario, &[], &journal)
        .expect("accepted and rejected request evidence should replay exactly");
    assert_eq!(simulation.snapshot(), replayed.snapshot());
}

#[test]
fn legacy_direct_and_tracked_request_ingress_cannot_mix() {
    let (scenario, ids) = demo_scenario();
    let mut legacy_first =
        Simulation::new(35, scenario.clone()).expect("compatibility run should load");
    legacy_first
        .submit(move_order(&ids))
        .expect("legacy direct command should be accepted");
    let after_legacy = legacy_first.snapshot();
    let error = legacy_first
        .process_command(CommandRequest::new(
            CommandRequestId::new(1),
            1,
            move_order(&ids),
        ))
        .expect_err("tracked requests cannot follow legacy-direct commands");
    assert_eq!(error.code, ErrorCode::MixedCommandIngress);
    assert_eq!(legacy_first.snapshot(), after_legacy);

    let mut tracked_first =
        Simulation::new(35, scenario.clone()).expect("compatibility run should load");
    let outcome = tracked_first
        .process_command(CommandRequest::new(
            CommandRequestId::new(1),
            0,
            CommandEnvelope::new(
                Issuer::Actor(ids.observer),
                Command::OrderMovement {
                    subject: EntityRef::Army(ids.army),
                    destination: ids.eastern_territory,
                    cargo: Vec::new(),
                },
            ),
        ))
        .expect("domain rejection should remain tracked evidence");
    assert!(matches!(outcome, CommandOutcome::Rejected { .. }));
    let after_tracked = tracked_first.snapshot();
    let error = tracked_first
        .submit(move_order(&ids))
        .expect_err("legacy direct commands cannot follow tracked attempts");
    assert_eq!(error.code, ErrorCode::MixedCommandIngress);
    assert_eq!(tracked_first.snapshot(), after_tracked);
    Simulation::from_snapshot(after_tracked.clone())
        .expect("a rejection-only tracked journal should remain loadable");

    let error = tracked_first
        .schedule_calendar_boundary(SimTime::EPOCH, vec![SystemCadence::Daily])
        .expect_err("canonical ingress cannot begin after a direct tracked attempt");
    assert_eq!(error.code, ErrorCode::MixedCommandIngress);
    assert_eq!(tracked_first.snapshot(), after_tracked);

    tracked_first
        .append_ingress(
            SimTime::EPOCH,
            IngressClass::ScheduledSystem,
            0,
            IngressPayload::Calendar {
                cadences: vec![SystemCadence::Daily],
            },
            Some(CauseRef::System("canwu.core.calendar".to_owned())),
            false,
        )
        .expect("the fixture should construct coherent mixed ingress evidence");
    let error = Simulation::from_snapshot(tracked_first.snapshot())
        .err()
        .expect("snapshot validation must reject mixed direct and canonical history");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let mut canonical_first =
        Simulation::new(35, scenario).expect("canonical compatibility run should load");
    canonical_first
        .schedule_calendar_boundary(SimTime::EPOCH, vec![SystemCadence::Daily])
        .expect("calendar ingress should establish the canonical family");
    let after_canonical = canonical_first.snapshot();
    let error = canonical_first
        .process_command(CommandRequest::new(
            CommandRequestId::new(2),
            0,
            move_order(&ids),
        ))
        .expect_err("direct tracked requests cannot bypass canonical ingress");
    assert_eq!(error.code, ErrorCode::MixedCommandIngress);
    assert_eq!(canonical_first.snapshot(), after_canonical);
}

#[test]
fn declared_runs_reject_untracked_legacy_command_history() {
    let (scenario, ids) = demo_scenario();
    let configuration = RunConfiguration::play_as_character(
        "seat.commander",
        "controller.human",
        ids.commander,
        "permission.military-command",
    );
    let manifest = manifest_for_configuration(&scenario, &configuration);
    let mut declared = Simulation::new_with_run_configuration(
        35,
        scenario.clone(),
        manifest.clone(),
        configuration.clone(),
    )
    .expect("declared run should load");
    let envelope = CommandEnvelope::new(
        Issuer::Human("controller.human".to_owned()),
        Command::OrderMovement {
            subject: EntityRef::Army(ids.army),
            destination: ids.eastern_territory,
            cargo: Vec::new(),
        },
    )
    .with_authority(character_authority(
        ids.commander,
        ids.army,
        "seat.commander",
        "permission.military-command",
    ))
    .at_time(SimTime::EPOCH);
    let before = declared.snapshot();
    let error = declared
        .submit(envelope)
        .expect_err("declared runs must not accept compatibility-only ingress");
    assert_eq!(error.code, ErrorCode::InvalidAuthority);
    assert_eq!(declared.snapshot(), before);

    let mut compatibility =
        Simulation::new(35, scenario.clone()).expect("compatibility run should load");
    compatibility
        .submit(move_order(&ids))
        .expect("legacy command fixture should be accepted");
    let mut forged = compatibility.snapshot();
    forged.run_manifest = before.run_manifest;
    forged.run_manifest_hash = before.run_manifest_hash;
    forged.run_configuration = before.run_configuration;
    refresh_snapshot_commitments_and_checkpoint(&mut forged);
    let Err(error) = Simulation::from_snapshot(forged) else {
        panic!("declared snapshots cannot smuggle untracked accepted commands");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    assert!(error.message.contains("tracked attempt evidence"));

    let Err(error) = Simulation::replay_with_run_configuration(
        35,
        scenario,
        manifest,
        configuration,
        &[],
        compatibility.command_log(),
        &[],
        &[],
        compatibility.time(),
    ) else {
        panic!("declared fixture replay cannot reinterpret legacy command input");
    };
    assert_eq!(error.code, ErrorCode::InvalidAuthority);
}

#[test]
fn declared_revision_and_time_guards_cover_boundaries_and_clock() {
    let (scenario, ids) = demo_scenario();
    let configuration = RunConfiguration::play_as_character(
        "seat.commander",
        "controller.human",
        ids.commander,
        "permission.military-command",
    );
    let manifest = manifest_for_configuration(&scenario, &configuration);
    let command_at = |time| {
        CommandEnvelope::new(
            Issuer::Human("controller.human".to_owned()),
            Command::OrderMovement {
                subject: EntityRef::Army(ids.army),
                destination: ids.eastern_territory,
                cargo: Vec::new(),
            },
        )
        .with_authority(character_authority(
            ids.commander,
            ids.army,
            "seat.commander",
            "permission.military-command",
        ))
        .at_time(time)
    };

    let mut after_boundary = Simulation::new_with_run_configuration(
        35,
        scenario.clone(),
        manifest.clone(),
        configuration.clone(),
    )
    .expect("declared run should load");
    after_boundary
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("boundary should publish");
    assert_eq!(after_boundary.revision(), 1);
    let stale = after_boundary
        .process_command(CommandRequest::new(
            CommandRequestId::new(1),
            0,
            command_at(SimTime::EPOCH),
        ))
        .expect("stale boundary revision should be retained as evidence");
    let CommandOutcome::Rejected { rejection } = stale else {
        panic!("a pre-boundary revision must be stale");
    };
    assert_eq!(rejection.error.code, ErrorCode::SimulationRevisionConflict);
    assert_eq!(rejection.retained_revision, 2);
    let accepted = after_boundary
        .process_command(CommandRequest::new(
            CommandRequestId::new(2),
            2,
            command_at(SimTime::EPOCH),
        ))
        .expect("current revision should be accepted");
    let CommandOutcome::Accepted { receipt } = accepted else {
        panic!("current revision and time should admit the command");
    };
    assert_eq!(receipt.revision, 3);
    assert_eq!(after_boundary.revision(), 3);
    let boundary_journal = after_boundary.replay_journal();
    let boundary_replay = Simulation::replay_from_journal(scenario.clone(), &[], &boundary_journal)
        .expect("boundary-relative revision evidence should replay exactly");
    assert_eq!(after_boundary.snapshot(), boundary_replay.snapshot());

    let mut after_clock =
        Simulation::new_with_run_configuration(35, scenario.clone(), manifest, configuration)
            .expect("declared run should load");
    after_clock
        .advance(SimDuration::hours(1))
        .expect("clock should advance");
    assert_eq!(after_clock.revision(), 0);
    let stale = after_clock
        .process_command(CommandRequest::new(
            CommandRequestId::new(1),
            0,
            command_at(SimTime::EPOCH),
        ))
        .expect("stale time should be retained as evidence");
    let CommandOutcome::Rejected { rejection } = stale else {
        panic!("a pre-advance simulation time must be stale");
    };
    assert_eq!(rejection.error.code, ErrorCode::SimulationTimeConflict);
    assert_eq!(rejection.retained_revision, 1);
    let accepted = after_clock
        .process_command(CommandRequest::new(
            CommandRequestId::new(2),
            1,
            command_at(after_clock.time()),
        ))
        .expect("current revision and time should be accepted");
    let CommandOutcome::Accepted { receipt } = accepted else {
        panic!("current clock guard should admit the command");
    };
    assert_eq!(receipt.revision, 2);
    let clock_journal = after_clock.replay_journal();
    let clock_replay = Simulation::replay_from_journal(scenario, &[], &clock_journal)
        .expect("clock-relative time evidence should replay exactly");
    assert_eq!(after_clock.snapshot(), clock_replay.snapshot());
}

#[test]
fn authoritative_revision_is_persisted_migrated_and_rollback_safe() {
    let (scenario, ids) = demo_scenario();
    let invalid_morale = |morale| {
        CommandEnvelope::new(
            Issuer::Debug,
            Command::DebugSetArmyMorale {
                army: ids.army,
                morale,
            },
        )
    };
    let first_request = CommandRequest::new(CommandRequestId::new(1), 0, invalid_morale(101));
    let mut simulation =
        Simulation::new(35, scenario.clone()).expect("compatibility run should load");

    let first = simulation
        .process_command(first_request.clone())
        .expect("expected rejection should persist");
    let CommandOutcome::Rejected { rejection } = &first else {
        panic!("invalid morale must be rejected");
    };
    assert_eq!(rejection.retained_revision, 1);
    assert_eq!(simulation.revision(), 1);

    let after_first = simulation.snapshot();
    assert_eq!(
        simulation
            .process_command(first_request)
            .expect("an exact retry should return the recorded outcome"),
        first
    );
    assert_eq!(simulation.snapshot(), after_first);

    let second = simulation
        .process_command(CommandRequest::new(
            CommandRequestId::new(2),
            1,
            invalid_morale(102),
        ))
        .expect("a second expected rejection should persist");
    let CommandOutcome::Rejected { rejection } = second else {
        panic!("invalid morale must be rejected");
    };
    assert_eq!(rejection.retained_revision, 2);
    assert_eq!(simulation.revision(), 2);

    let before_conflict = simulation.snapshot();
    let conflict = simulation
        .process_command(CommandRequest::new(
            CommandRequestId::new(2),
            2,
            invalid_morale(103),
        ))
        .expect("a request-ID collision should return a non-persisted rejection");
    let CommandOutcome::Rejected { rejection } = conflict else {
        panic!("a request-ID collision must not be accepted");
    };
    assert_eq!(rejection.attempt_id, None);
    assert_eq!(rejection.error.code, ErrorCode::IdempotencyConflict);
    assert_eq!(rejection.retained_revision, 2);
    assert_eq!(simulation.snapshot(), before_conflict);

    simulation
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("an empty boundary should publish");
    assert_eq!(simulation.revision(), 3);
    let first_boundary_snapshot = simulation.snapshot();
    let third = simulation
        .process_command(CommandRequest::new(
            CommandRequestId::new(3),
            3,
            invalid_morale(103),
        ))
        .expect("a post-boundary expected rejection should persist");
    let CommandOutcome::Rejected { rejection } = third else {
        panic!("invalid morale must be rejected");
    };
    assert_eq!(rejection.retained_revision, 4);
    simulation
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH + SimDuration::hours(1)))
        .expect("a second empty boundary should publish");
    assert_eq!(simulation.revision(), 5);
    let current_snapshot = simulation.snapshot();
    let restored = Simulation::from_snapshot(current_snapshot.clone())
        .expect("current revision evidence should survive load");
    assert_eq!(restored.revision(), 5);
    assert_eq!(restored.snapshot(), current_snapshot);
    let replayed =
        Simulation::replay_from_journal(scenario.clone(), &[], &simulation.replay_journal())
            .expect("current revision evidence should replay exactly");
    assert_eq!(replayed.snapshot(), current_snapshot);

    let mut inconsistent_revision = current_snapshot.clone();
    inconsistent_revision.state_revision = 6;
    inconsistent_revision.checkpoint_hash = snapshot_checkpoint_hash(&inconsistent_revision)
        .expect("the inconsistent revision fixture should remain coherently hashed");
    let error = Simulation::from_snapshot(inconsistent_revision)
        .err()
        .expect("a rehashed revision without evidence must not load");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    assert!(error.message.contains("state revision"));

    let mut legacy_first_boundary = first_boundary_snapshot;
    legacy_first_boundary.command_attempts[1].revision_before = 0;
    legacy_first_boundary.command_attempts[1].expected_revision = Some(0);
    legacy_first_boundary.revision_format_version = 0;
    legacy_first_boundary.state_revision = 0;
    legacy_first_boundary.replay_revision_format_version = 0;
    let legacy_first_boundary_state_hash = snapshot_state_hash(&legacy_first_boundary)
        .expect("legacy first-boundary state should hash canonically");

    let mut legacy_snapshot = current_snapshot.clone();
    legacy_snapshot.command_attempts[1].revision_before = 0;
    legacy_snapshot.command_attempts[1].expected_revision = Some(0);
    legacy_snapshot.command_attempts[2].revision_before = 1;
    legacy_snapshot.command_attempts[2].expected_revision = Some(u64::MAX);
    legacy_snapshot.command_attempts[2].outcome = CommandAttemptOutcome::Rejected {
        error: CanwuError::new(
            ErrorCode::SimulationRevisionConflict,
            format!(
                "command expected revision {}, but simulation is at revision 1",
                u64::MAX
            ),
        ),
    };
    legacy_snapshot.revision_format_version = 0;
    legacy_snapshot.state_revision = 0;
    legacy_snapshot.replay_revision_format_version = 0;
    legacy_snapshot.boundaries[0].state_hash = Some(legacy_first_boundary_state_hash);
    let legacy_state_hash = snapshot_state_hash(&legacy_snapshot)
        .expect("legacy revision state should hash canonically");
    legacy_snapshot
        .boundaries
        .last_mut()
        .expect("the migration fixture has a boundary head")
        .state_hash = Some(legacy_state_hash);
    migration::rehash_snapshot_boundaries(&mut legacy_snapshot)
        .expect("legacy boundary evidence should hash canonically");
    downgrade_snapshot_commitments(&mut legacy_snapshot);
    legacy_snapshot.checkpoint_hash = snapshot_checkpoint_hash(&legacy_snapshot)
        .expect("legacy checkpoint should bind its pre-migration state");
    let mut legacy_value =
        serde_json::to_value(legacy_snapshot).expect("legacy fixture should serialize");
    let legacy_object = legacy_value
        .as_object_mut()
        .expect("legacy snapshot JSON should be an object");
    legacy_object.remove("revision_format_version");
    legacy_object.remove("state_revision");
    legacy_object.remove("replay_revision_format_version");
    legacy_object.remove("admission_cursor_format_version");
    legacy_object.remove("admitted_attempt_count");
    legacy_object.remove("admitted_command_count");
    legacy_object.remove("admitted_event_count");
    let mut broken_chain = legacy_value.clone();
    broken_chain["boundaries"][0]["correlation_id"] = Value::from(999_u64);
    let error = Simulation::from_snapshot_json(
        &serde_json::to_string(&broken_chain).expect("tampered legacy fixture should encode"),
    )
    .err()
    .expect("migration must not launder a broken legacy boundary hash chain");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    assert!(error.message.contains("legacy boundary hash chain"));

    let migrated = Simulation::from_snapshot_json(
        &serde_json::to_string(&legacy_value).expect("legacy fixture should encode"),
    )
    .expect("legacy command revisions should migrate deterministically");
    assert_eq!(migrated.revision(), 5);
    assert_eq!(migrated.command_attempts()[1].revision_before, 1);
    assert_eq!(migrated.command_attempts()[2].revision_before, 3);
    assert_eq!(
        migrated.command_attempts()[2].expected_revision,
        Some(u64::MAX)
    );
    assert_eq!(migrated.snapshot().replay_revision_format_version, 0);
    let reloaded = Simulation::from_snapshot(migrated.snapshot())
        .expect("migration-only replay provenance should survive save and load");
    assert_eq!(reloaded.revision(), 5);

    let migrated_journal = reloaded.replay_journal();
    assert_eq!(migrated_journal.revision_format_version, 0);
    let error = Simulation::replay_from_journal(scenario, &[], &migrated_journal)
        .err()
        .expect("revision-migrated histories must not claim current exact replay");
    assert_eq!(error.code, ErrorCode::LegacyReplayUnavailable);

    let mut continued = reloaded;
    continued
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH + SimDuration::hours(2)))
        .expect("a revision-migrated snapshot should remain continuable");
    assert_eq!(continued.revision(), 6);
}

#[test]
fn legacy_revision_migration_rebases_admitted_and_pending_command_ingress() {
    let (scenario, ids) = demo_scenario();
    let invalid_request = |request_id, revision, morale| {
        CommandRequest::new(
            CommandRequestId::new(request_id),
            revision,
            CommandEnvelope::new(
                Issuer::Debug,
                Command::DebugSetArmyMorale {
                    army: ids.army,
                    morale,
                },
            ),
        )
    };
    let mut simulation =
        Simulation::new(47, scenario).expect("canonical migration run should load");
    simulation
        .enqueue_command(SimTime::EPOCH, 0, invalid_request(1, 0, 101))
        .expect("first invalid command should queue");
    simulation
        .step_canonical()
        .expect("first command boundary should settle")
        .expect("first command should create a boundary");
    assert_eq!(simulation.revision(), 2);
    let after_first_boundary = simulation.snapshot();

    let second_at = SimTime::EPOCH + SimDuration::hours(1);
    simulation
        .enqueue_command(second_at, 0, invalid_request(2, 2, 102))
        .expect("second invalid command should queue");
    simulation
        .advance_canonical(SimDuration::hours(1))
        .expect("second command boundary should settle");
    assert_eq!(simulation.revision(), 4);
    let after_second_boundary = simulation.snapshot();

    let pending_at = second_at + SimDuration::hours(1);
    simulation
        .enqueue_command(pending_at, 0, invalid_request(3, 4, 103))
        .expect("pending invalid command should queue");
    let current_snapshot = simulation.snapshot();

    let legacyize = |snapshot: &mut SimulationSnapshot| {
        snapshot.revision_format_version = 0;
        snapshot.state_revision = 0;
        snapshot.replay_revision_format_version = 0;
        for (index, attempt) in snapshot.command_attempts.iter_mut().enumerate() {
            let legacy_revision = u64::try_from(index).expect("fixture index should fit");
            attempt.revision_before = legacy_revision;
            attempt.expected_revision = Some(legacy_revision);
        }
        for (index, record) in snapshot.ingress.iter_mut().enumerate() {
            let IngressPayload::Command { request } = &mut record.payload else {
                continue;
            };
            request.expected_revision = u64::try_from(index).expect("fixture index should fit");
        }
    };

    let mut legacy_first = after_first_boundary;
    legacyize(&mut legacy_first);
    let first_state_hash =
        snapshot_state_hash(&legacy_first).expect("legacy first command boundary should hash");

    let mut legacy_second = after_second_boundary;
    legacyize(&mut legacy_second);
    legacy_second.boundaries[0].state_hash = Some(first_state_hash.clone());
    let second_state_hash =
        snapshot_state_hash(&legacy_second).expect("legacy second command boundary should hash");

    let mut legacy_snapshot = current_snapshot;
    legacyize(&mut legacy_snapshot);
    legacy_snapshot.boundaries[0].state_hash = Some(first_state_hash);
    legacy_snapshot.boundaries[1].state_hash = Some(second_state_hash);
    migration::rehash_snapshot_boundaries(&mut legacy_snapshot)
        .expect("legacy ingress boundary chain should hash");
    downgrade_snapshot_commitments(&mut legacy_snapshot);
    legacy_snapshot.checkpoint_hash =
        snapshot_checkpoint_hash(&legacy_snapshot).expect("legacy ingress checkpoint should hash");
    let mut legacy_value =
        serde_json::to_value(legacy_snapshot).expect("legacy ingress fixture should serialize");
    let legacy_object = legacy_value
        .as_object_mut()
        .expect("legacy ingress snapshot should be an object");
    legacy_object.remove("revision_format_version");
    legacy_object.remove("state_revision");
    legacy_object.remove("replay_revision_format_version");
    legacy_object.remove("admission_cursor_format_version");
    legacy_object.remove("admitted_attempt_count");
    legacy_object.remove("admitted_command_count");
    legacy_object.remove("admitted_event_count");

    let mut migrated = Simulation::from_snapshot_json(
        &serde_json::to_string(&legacy_value).expect("legacy ingress fixture should encode"),
    )
    .expect("admitted and pending command guards should migrate coherently");
    assert_eq!(migrated.revision(), 4);
    assert_eq!(
        migrated
            .command_attempts()
            .iter()
            .map(|attempt| (attempt.revision_before, attempt.expected_revision))
            .collect::<Vec<_>>(),
        vec![(0, Some(0)), (2, Some(2))]
    );
    assert_eq!(
        migrated
            .ingress_log()
            .iter()
            .filter_map(|record| match &record.payload {
                IngressPayload::Command { request } => Some(request.expected_revision),
                IngressPayload::Decision { .. }
                | IngressPayload::Plugin { .. }
                | IngressPayload::Calendar { .. } => None,
            })
            .collect::<Vec<_>>(),
        vec![0, 2, 4]
    );
    assert_eq!(migrated.snapshot().replay_revision_format_version, 0);

    migrated
        .step_canonical()
        .expect("migrated pending command should settle")
        .expect("pending command should create a boundary");
    assert_eq!(migrated.revision(), 6);
    assert_eq!(
        migrated
            .command_attempts()
            .last()
            .expect("pending command should create an attempt")
            .revision_before,
        4
    );
}

#[test]
fn admission_cursors_are_persisted_migrated_and_tamper_evident() {
    let (scenario, ids) = demo_scenario();
    let morale_request = |request_id, revision, morale| {
        CommandRequest::new(
            CommandRequestId::new(request_id),
            revision,
            CommandEnvelope::new(
                Issuer::Debug,
                Command::DebugSetArmyMorale {
                    army: ids.army,
                    morale,
                },
            ),
        )
    };
    let mut simulation = Simulation::new(59, scenario.clone()).expect("cursor fixture should load");
    assert!(matches!(
        simulation
            .process_command(morale_request(1, 0, 80))
            .expect("first command should be accepted"),
        CommandOutcome::Accepted { .. }
    ));
    assert!(matches!(
        simulation
            .process_command(morale_request(2, 1, 101))
            .expect("expected rejection should persist"),
        CommandOutcome::Rejected { .. }
    ));
    simulation
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("first cursor boundary should settle");
    let first_snapshot = simulation.snapshot();
    assert_eq!(first_snapshot.admitted_attempt_count, 2);
    assert_eq!(first_snapshot.admitted_command_count, 1);
    assert_eq!(first_snapshot.admitted_event_count, 1);

    assert!(matches!(
        simulation
            .process_command(morale_request(3, 3, 70))
            .expect("second command should be accepted"),
        CommandOutcome::Accepted { .. }
    ));
    simulation
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("second cursor boundary should settle");
    let current_snapshot = simulation.snapshot();
    assert_eq!(current_snapshot.admission_cursor_format_version, 1);
    assert_eq!(current_snapshot.admitted_attempt_count, 3);
    assert_eq!(current_snapshot.admitted_command_count, 2);
    assert_eq!(current_snapshot.admitted_event_count, 2);

    let restored = Simulation::from_snapshot(current_snapshot.clone())
        .expect("persisted admission cursors should load");
    assert_eq!(restored.snapshot(), current_snapshot);
    let replayed = Simulation::replay_from_journal(scenario, &[], &simulation.replay_journal())
        .expect("admission cursors should reproduce under exact replay");
    assert_eq!(replayed.snapshot(), current_snapshot);

    let mut legacy_value = serde_json::to_value(current_snapshot.clone())
        .expect("cursor migration fixture should serialize");
    let legacy_object = legacy_value
        .as_object_mut()
        .expect("cursor migration snapshot should be an object");
    legacy_object.remove("admission_cursor_format_version");
    legacy_object.remove("admitted_attempt_count");
    legacy_object.remove("admitted_command_count");
    legacy_object.remove("admitted_event_count");
    let migrated = Simulation::from_snapshot_json(
        &serde_json::to_string(&legacy_value).expect("cursor migration fixture should encode"),
    )
    .expect("legacy admission cursors should derive from boundary prefixes");
    assert_eq!(migrated.snapshot(), current_snapshot);

    let mut tampered_cursor = current_snapshot.clone();
    tampered_cursor.admitted_attempt_count -= 1;
    let error = Simulation::from_snapshot(tampered_cursor)
        .err()
        .expect("a cursor detached from boundary evidence must not load");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    assert!(error.message.contains("admission cursors"));

    let mut migrated_gap = current_snapshot;
    migrated_gap.boundaries[0].admitted_attempts.remove(0);
    migrated_gap.admission_cursor_format_version = 0;
    migrated_gap.admitted_attempt_count = 0;
    migrated_gap.admitted_command_count = 0;
    migrated_gap.admitted_event_count = 0;
    rehash_tampered_snapshot(&mut migrated_gap);
    let error = Simulation::from_snapshot(migrated_gap)
        .err()
        .expect("legacy cursor migration must reject a journal-prefix gap");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
}

#[test]
fn expected_domain_rejections_survive_load_and_exact_replay() {
    let (scenario, ids) = demo_scenario();
    let mut simulation =
        Simulation::new(35, scenario.clone()).expect("compatibility run should load");
    simulation
        .register_plugin(&AuthorityPlugin)
        .expect("payload-validation plugin should register");
    let requests = [
        (
            CommandRequestId::new(1),
            CommandEnvelope::new(
                Issuer::Actor(ids.commander),
                Command::OrderMovement {
                    subject: EntityRef::Army(ArmyId::new(999)),
                    destination: ids.eastern_territory,
                    cargo: Vec::new(),
                },
            ),
        ),
        (
            CommandRequestId::new(2),
            CommandEnvelope::new(
                Issuer::Debug,
                Command::DebugSetArmyMorale {
                    army: ids.army,
                    morale: 101,
                },
            ),
        ),
        (
            CommandRequestId::new(3),
            CommandEnvelope::new(
                Issuer::Actor(ids.commander),
                Command::Plugin {
                    plugin: "authority-test".to_owned(),
                    command: "set_stance".to_owned(),
                    payload: serde_json::json!({}),
                },
            ),
        ),
        (
            CommandRequestId::new(4),
            CommandEnvelope::new(
                Issuer::Actor(ids.commander),
                Command::Plugin {
                    plugin: "missing-plugin".to_owned(),
                    command: "missing-command".to_owned(),
                    payload: Value::Null,
                },
            ),
        ),
    ];
    let expected = [
        ErrorCode::ArmyNotFound,
        ErrorCode::ValueOutOfRange,
        ErrorCode::InvalidPayload,
        ErrorCode::PluginCommandNotFound,
    ];
    for ((request_id, envelope), expected_code) in requests.into_iter().zip(expected) {
        let revision_before = simulation.revision();
        let outcome = simulation
            .process_command(CommandRequest::new(request_id, revision_before, envelope))
            .expect("expected domain rejection should be a command outcome");
        let CommandOutcome::Rejected { rejection } = outcome else {
            panic!("invalid command fixture must be rejected");
        };
        assert_eq!(rejection.error.code, expected_code);
        assert_eq!(rejection.retained_revision, revision_before + 1);
    }
    assert!(simulation.command_log().is_empty());
    assert_eq!(simulation.command_attempts().len(), 4);
    assert_eq!(simulation.revision(), 4);

    let snapshot = simulation.snapshot();
    let restored = Simulation::from_snapshot(snapshot.clone())
        .expect("expected rejection evidence must not invalidate its own snapshot");
    assert_eq!(restored.snapshot(), snapshot);
    let journal = simulation.replay_journal();
    let replayed = Simulation::replay_from_journal(scenario, &[&AuthorityPlugin], &journal)
        .expect("expected rejection evidence should replay exactly");
    assert_eq!(simulation.snapshot(), replayed.snapshot());
}

#[test]
fn read_only_and_frozen_replay_ingress_are_not_interchangeable() {
    let (scenario, ids) = demo_scenario();
    let observer_configuration = RunConfiguration::read_only_observer();
    let observer_manifest = manifest_for_configuration(&scenario, &observer_configuration);
    let mut observer = Simulation::new_with_run_configuration(
        35,
        scenario.clone(),
        observer_manifest,
        observer_configuration,
    )
    .expect("read-only observer run should load");
    let before = observer.snapshot();
    let live_human = observer
        .process_command(CommandRequest::new(
            CommandRequestId::new(1),
            0,
            CommandEnvelope::new(
                Issuer::Human("controller.human".to_owned()),
                Command::OrderMovement {
                    subject: EntityRef::Army(ids.army),
                    destination: ids.eastern_territory,
                    cargo: Vec::new(),
                },
            )
            .with_authority(CommandAuthority::for_actor(ids.commander)),
        ))
        .expect("read-only rejection should be structured");
    let CommandOutcome::Rejected { rejection } = live_human else {
        panic!("read-only observer must reject a live human command");
    };
    assert_eq!(rejection.error.code, ErrorCode::InteractionReadOnly);
    assert_eq!(observer.world(), before.world);
    assert_eq!(observer.events(), before.events);
    assert_eq!(observer.command_log(), before.commands);
    assert_eq!(observer.random_draws(), before.random_draws);
    assert_eq!(observer.command_attempts().len(), 1);
    let observer_journal = observer.replay_journal();
    let observer_replay = Simulation::replay_from_journal(scenario.clone(), &[], &observer_journal)
        .expect("read-only rejection evidence should replay exactly");
    assert_eq!(observer.snapshot(), observer_replay.snapshot());

    let replay_configuration = RunConfiguration::replay_as_character(
        "seat.commander",
        "controller.recorded",
        ids.commander,
        "permission.military-command",
    );
    let replay_manifest = manifest_for_configuration(&scenario, &replay_configuration);
    let replay_envelope = CommandEnvelope::new(
        Issuer::Replay("controller.recorded".to_owned()),
        Command::OrderMovement {
            subject: EntityRef::Army(ids.army),
            destination: ids.eastern_territory,
            cargo: Vec::new(),
        },
    )
    .with_authority(character_authority(
        ids.commander,
        ids.army,
        "seat.commander",
        "permission.military-command",
    ))
    .at_time(SimTime::EPOCH);

    let mut live_replay = Simulation::new_with_run_configuration(
        35,
        scenario.clone(),
        replay_manifest.clone(),
        replay_configuration.clone(),
    )
    .expect("replay run should load");
    let outcome = live_replay
        .process_command(CommandRequest::new(
            CommandRequestId::new(1),
            0,
            replay_envelope.clone(),
        ))
        .expect("live replay forgery should be a structured rejection");
    let CommandOutcome::Rejected { rejection } = outcome else {
        panic!("a live caller cannot self-identify as frozen replay");
    };
    assert_eq!(rejection.error.code, ErrorCode::InvalidAuthority);
    assert!(live_replay.command_log().is_empty());

    let mut frozen_source = Simulation::new_with_run_configuration(
        35,
        scenario.clone(),
        replay_manifest.clone(),
        replay_configuration.clone(),
    )
    .expect("frozen replay source should load");
    let outcome = frozen_source
        .admit_command(
            Some(CommandRequestId::new(7)),
            Some(0),
            replay_envelope,
            CommandIngress::FrozenReplay,
            None,
            true,
        )
        .expect("the trusted replay path should consume frozen input");
    assert!(matches!(outcome, CommandOutcome::Accepted { .. }));
    let Err(error) = Simulation::replay_with_run_configuration(
        35,
        scenario.clone(),
        replay_manifest,
        replay_configuration,
        &[],
        frozen_source.command_log(),
        frozen_source.command_attempts(),
        frozen_source.boundaries(),
        frozen_source.time(),
    ) else {
        panic!("caller-supplied fixture replay cannot consume frozen ingress");
    };
    assert_eq!(error.code, ErrorCode::ReplayEnvironmentMismatch);
    let frozen_journal = frozen_source.replay_journal();
    let frozen_replay = Simulation::replay_from_journal(scenario, &[], &frozen_journal)
        .expect("frozen controller input should replay exactly");
    assert_eq!(frozen_source.snapshot(), frozen_replay.snapshot());

    let mut forged_live_ingress = frozen_source.snapshot();
    forged_live_ingress.command_attempts[0].ingress = CommandIngress::LiveRequest;
    refresh_snapshot_commitments_and_checkpoint(&mut forged_live_ingress);
    let Err(error) = Simulation::from_snapshot(forged_live_ingress) else {
        panic!("live ingress cannot be relabeled as an accepted replay command");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
}

#[test]
fn observation_and_trace_policy_are_causally_inert() {
    let (scenario, _) = demo_scenario();
    let public_configuration = RunConfiguration::read_only_observer();
    let mut research_configuration = public_configuration.clone();
    research_configuration.observation = ObservationPolicy::ResearchFull;
    research_configuration.trace = TracePolicy::FullResearch;
    let mut public = Simulation::new_with_run_configuration(
        35,
        scenario.clone(),
        manifest_for_configuration(&scenario, &public_configuration),
        public_configuration,
    )
    .expect("public observer run should load");
    let mut research = Simulation::new_with_run_configuration(
        35,
        scenario.clone(),
        manifest_for_configuration(&scenario, &research_configuration),
        research_configuration,
    )
    .expect("research observer run should load");

    assert_ne!(public.run_manifest_hash(), research.run_manifest_hash());
    assert_ne!(public.checkpoint_hash(), research.checkpoint_hash());
    assert_eq!(
        public
            .authoritative_state_hash()
            .expect("public state should hash"),
        research
            .authoritative_state_hash()
            .expect("research state should hash")
    );
    let public_receipt = public
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("public boundary should settle");
    let research_receipt = research
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("research boundary should settle");
    assert_eq!(public_receipt.boundary_hash, research_receipt.boundary_hash);
    assert_eq!(
        public.boundaries()[0].state_hash,
        research.boundaries()[0].state_hash
    );
    assert_eq!(public.world(), research.world());
    assert_eq!(public.random_draws(), research.random_draws());
    assert_eq!(
        public.snapshot().random_streams,
        research.snapshot().random_streams
    );
    assert_ne!(public.checkpoint_hash(), research.checkpoint_hash());
}

#[test]
fn invalid_command_does_not_mutate_any_serialized_state() {
    let (mut simulation, ids) = Simulation::demo(35).expect("demo should load");
    let before = simulation
        .snapshot_json()
        .expect("snapshot should serialize");
    let result = simulation.submit(CommandEnvelope::new(
        Issuer::Actor(ids.observer),
        Command::OrderMovement {
            subject: EntityRef::Army(ids.army),
            destination: ids.eastern_territory,
            cargo: Vec::new(),
        },
    ));
    assert_eq!(
        result.expect_err("observer cannot command army").code,
        ErrorCode::InvalidAuthority
    );
    assert_eq!(
        before,
        simulation
            .snapshot_json()
            .expect("snapshot should serialize")
    );
}

#[test]
fn movement_emits_events_and_executes_at_scheduled_time() {
    let (mut simulation, ids) = Simulation::demo(35).expect("demo should load");
    let receipt = simulation
        .submit(move_order(&ids))
        .expect("order should validate");
    assert_eq!(receipt.emitted_events.len(), 1);
    simulation
        .advance(SimDuration::hours(17))
        .expect("time should advance");
    assert_eq!(
        simulation
            .world()
            .army(ids.army)
            .expect("army exists")
            .location,
        ids.central_territory
    );
    let events = simulation
        .advance(SimDuration::hours(1))
        .expect("arrival should execute");
    assert!(
        events
            .iter()
            .any(|event| matches!(event.kind, EventKind::ArmyArrived { .. }))
    );
    assert_eq!(
        simulation
            .world()
            .army(ids.army)
            .expect("army exists")
            .location,
        ids.eastern_territory
    );
}

#[test]
fn legacy_move_army_wire_shape_is_not_a_current_command() {
    let value = serde_json::json!({
        "type": "move_army",
        "army": 1,
        "destination": 3,
    });
    assert!(serde_json::from_value::<Command>(value).is_err());
}

#[test]
fn person_self_move_carries_and_delivers_a_letter() {
    let (mut scenario, ids) = demo_scenario();
    scenario.world.letters.push(LetterCargo {
        id: LetterId::new(1),
        sender: ids.commander,
        recipient: ids.observer,
        body: "Meet at the western gate".to_owned(),
        status: LetterStatus::HeldByPerson,
        carrier: Some(ids.commander),
        location: None,
        delivered_at: None,
    });
    let mut simulation = Simulation::new(35, scenario).expect("letter scenario should load");
    let receipt = simulation
        .submit(CommandEnvelope::new(
            Issuer::Actor(ids.commander),
            Command::OrderMovement {
                subject: EntityRef::Person(ids.commander),
                destination: ids.western_territory,
                cargo: vec![LetterId::new(1)],
            },
        ))
        .expect("the person may move themself with a held letter");
    assert_eq!(receipt.emitted_events.len(), 1);
    assert_eq!(
        simulation
            .world()
            .person(ids.commander)
            .expect("person exists")
            .transit
            .as_ref()
            .expect("person should be in transit")
            .to,
        ids.western_territory
    );
    let events = simulation
        .advance(SimDuration::hours(12))
        .expect("person arrival should execute");
    assert!(
        events
            .iter()
            .any(|event| matches!(event.kind, EventKind::PersonArrived { .. }))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event.kind, EventKind::LetterDelivered { .. }))
    );
    let world = simulation.world();
    let letter = world.letter(LetterId::new(1)).expect("letter exists");
    assert_eq!(letter.status, LetterStatus::Delivered);
    assert_eq!(letter.location, Some(ids.western_territory));
    assert_eq!(letter.carrier, None);
    let restored = Simulation::from_snapshot(simulation.snapshot())
        .expect("person movement should survive snapshot validation");
    assert_eq!(restored.snapshot(), simulation.snapshot());
}

#[test]
fn snapshot_rejects_event_correlation_drift() {
    let (mut simulation, ids) = Simulation::demo(35).expect("demo should load");
    simulation
        .submit(move_order(&ids))
        .expect("movement command should commit");
    simulation
        .advance(SimDuration::hours(18))
        .expect("arrival work should execute");

    let mut tampered = simulation.snapshot();
    let child_index = tampered
        .events
        .iter()
        .position(|event| matches!(event.cause, Some(CauseRef::Event(_))))
        .expect("the movement timeline should contain a child event");
    tampered.events[child_index].correlation_id = tampered.events[child_index]
        .correlation_id
        .checked_add(1)
        .expect("the fixture correlation should remain representable");
    refresh_snapshot_commitments_and_checkpoint(&mut tampered);

    let Err(error) = Simulation::from_snapshot(tampered) else {
        panic!("an event child cannot silently change correlation chains");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    assert!(error.message.contains("correlation"));
}

#[test]
fn snapshot_rejects_correlation_reuse_across_command_roots() {
    let (scenario, ids) = demo_scenario();
    let mut simulation = Simulation::new(35, scenario).expect("demo should load");
    let debug_command = |morale| {
        CommandEnvelope::new(
            Issuer::Debug,
            Command::DebugSetArmyMorale {
                army: ids.army,
                morale,
            },
        )
    };
    simulation
        .submit(debug_command(61))
        .expect("the first command should commit");
    simulation
        .submit(debug_command(62))
        .expect("the second command should commit");

    let mut tampered = simulation.snapshot();
    tampered.events[1].correlation_id = tampered.events[0].correlation_id;
    refresh_snapshot_commitments_and_checkpoint(&mut tampered);

    let Err(error) = Simulation::from_snapshot(tampered) else {
        panic!("one correlation cannot identify two command roots");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    assert!(error.message.contains("unrelated causal roots"));
}

#[test]
fn snapshot_rejects_scheduled_plugin_correlation_drift() {
    let (mut simulation, ids) = Simulation::demo(35).expect("demo should load");
    simulation
        .register_plugin(&FailingPlugin)
        .expect("the scheduling fixture plugin should register");
    simulation
        .submit(move_order(&ids))
        .expect("the movement command should commit");
    simulation
        .submit(CommandEnvelope::new(
            Issuer::Debug,
            Command::DebugSetArmyMorale {
                army: ids.army,
                morale: 61,
            },
        ))
        .expect("a second command should provide another committed correlation");

    let order_event = simulation
        .events()
        .iter()
        .find(|event| matches!(event.kind, EventKind::MoveOrdered { .. }))
        .expect("the movement order event should exist");
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
                    summary: "A scheduled directive with forged provenance".to_owned(),
                }),
                allowed_writes: vec![StateKey::new("failure-fixture", "flag")],
                cause: CauseRef::Event(order_event.id),
                correlation_id: order_event
                    .correlation_id
                    .checked_add(1)
                    .expect("the fixture correlation should remain representable"),
            },
        )
        .expect("the future directive should be accepted into the scheduler");

    let mut tampered = simulation.snapshot();
    refresh_snapshot_commitments_and_checkpoint(&mut tampered);
    let Err(error) = Simulation::from_snapshot_with_plugins(tampered, &[&FailingPlugin]) else {
        panic!("a scheduled directive must retain its cause correlation");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    assert!(error.message.contains("event correlation"));
}
