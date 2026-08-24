use super::*;

const SAME_BOUNDARY_VERSION_PLUGIN: &str = "fixture-same-boundary-version";

fn same_boundary_record_ref() -> DomainRecordRef {
    DomainRecordRef {
        kind: DomainRecordKind::new("fixture.same-boundary", "record"),
        id: "primary".to_owned(),
    }
}

fn propose_same_boundary_update(
    view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let reference = same_boundary_record_ref();
    let current = view
        .domain_record(&reference)?
        .ok_or_else(|| CanwuError::new(ErrorCode::InvalidDomainRecord, "fixture record missing"))?;
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Update {
                record: DomainRecordDraft::new(reference, serde_json::json!({"value": 2})),
                expected_version: current.version,
            },
            summary: "Propose a same-boundary replacement version".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

fn verify_prior_same_boundary_version(
    view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let prior = DomainRecordVersionRef {
        record: same_boundary_record_ref(),
        version: 1,
        established_by: DomainRecordVersionSource::InitialScenario,
    };
    if !view.domain_record_version_evidence_exists(&prior)?
        || !view.evidence_exists(&EvidenceRef::DomainRecordVersion(prior))?
    {
        return Err(CanwuError::new(
            ErrorCode::EvidenceUnavailable,
            "an earlier same-boundary proposal hid valid prior-version evidence",
        ));
    }
    Ok(BoundaryProposal::default())
}

struct SameBoundaryVersionPlugin;

impl SimulationPlugin for SameBoundaryVersionPlugin {
    fn name(&self) -> &'static str {
        SAME_BOUNDARY_VERSION_PLUGIN
    }

    fn version(&self) -> &'static str {
        "test-v1"
    }

    fn semantic_hash(&self) -> &'static str {
        "7100000000000000000000000000000000000000000000000000000000000000"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut schema =
            DomainRecordSchema::new(same_boundary_record_ref().kind, DomainRecordClass::Record);
        schema.mutation_policy = DomainRecordMutationPolicy::Versioned;
        let state = schema.state_key();
        registrar.register_record_schema(schema)?;

        let mut update = BoundarySystemContract::new(
            "a-propose-update",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        update.reads = vec![state.clone()];
        update.writes = vec![state.clone()];
        update.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(update, propose_same_boundary_update)?;

        let mut verify = BoundarySystemContract::new(
            "z-verify-prior-version",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        verify.reads = vec![state];
        registrar.register_boundary_system(verify, verify_prior_same_boundary_version)
    }
}

#[test]
fn same_boundary_proposal_does_not_hide_valid_prior_version_evidence() {
    let (mut scenario, _) = demo_scenario();
    scenario.domain_records.push(DomainRecord {
        reference: same_boundary_record_ref(),
        owner: SAME_BOUNDARY_VERSION_PLUGIN.to_owned(),
        class: DomainRecordClass::Record,
        version: 1,
        lifecycle: DomainRecordLifecycle::Active,
        payload: serde_json::json!({"value": 1}),
        references: Vec::new(),
    });
    let mut simulation = Simulation::new_with_plugins(811, scenario, &[&SameBoundaryVersionPlugin])
        .expect("same-boundary fixture should initialize");
    simulation
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily))
        .expect("both old and proposed versions should remain valid evidence");
    assert_eq!(
        simulation
            .domain_record(&same_boundary_record_ref())
            .expect("updated record")
            .version,
        2
    );
}

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
    let replayed = Simulation::replay_from_journal_with_scenario(scenario, &[], &journal)
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
    forged.authority_root_seed = before.authority_root_seed;
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
    let boundary_replay =
        Simulation::replay_from_journal_with_scenario(scenario.clone(), &[], &boundary_journal)
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
    let clock_replay = Simulation::replay_from_journal_with_scenario(scenario, &[], &clock_journal)
        .expect("clock-relative time evidence should replay exactly");
    assert_eq!(after_clock.snapshot(), clock_replay.snapshot());
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
    let replayed =
        Simulation::replay_from_journal_with_scenario(scenario, &[&AuthorityPlugin], &journal)
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
    let observer_replay =
        Simulation::replay_from_journal_with_scenario(scenario.clone(), &[], &observer_journal)
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
    let frozen_replay =
        Simulation::replay_from_journal_with_scenario(scenario, &[], &frozen_journal)
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
            .any(|event| event.kind.is_type("army_arrived"))
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
    scenario
        .entities
        .push(EntityRef::Resource(ResourceId::new(1)));
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
            .any(|event| event.kind.is_type("person_arrived"))
    );
    assert!(
        events
            .iter()
            .any(|event| event.kind.is_type("letter_delivered"))
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
        .find(|event| event.kind.is_type("move_ordered"))
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

#[test]
fn domain_record_candidates_stay_sparse_among_unrelated_kinds() {
    let target = DomainRecordKind::new("test.technology", "attempt");
    let unrelated = DomainRecordKind::new("test.world", "unrelated");
    let mut records = BTreeMap::new();
    for index in 0..100_000 {
        insert_query_fixture(&mut records, unrelated.clone(), format!("{index:06}"));
    }
    for id in ["000", "001", "002"] {
        insert_query_fixture(&mut records, target.clone(), id.to_owned());
    }

    let first = domain_record_candidates(&records, &target, None, 2);
    assert_eq!(first.len(), 2);
    assert_eq!(
        first
            .keys()
            .map(|value| value.id.as_str())
            .collect::<Vec<_>>(),
        ["000", "001"]
    );
    let cursor = first.last_key_value().expect("page should not be empty").0;
    let second = domain_record_candidates(&records, &target, Some(cursor), 2);
    assert_eq!(second.len(), 1);
    assert_eq!(
        second.first_key_value().expect("tail should exist").0.id,
        "002"
    );
}

#[test]
fn domain_record_pages_reject_a_stale_revision() {
    let (scenario, _) = demo_scenario();
    let mut simulation = Simulation::new(113, scenario).expect("demo should load");
    let kind = DomainRecordKind::new("test.technology", "attempt");
    let first = simulation
        .domain_record_page(&kind, None, 16, None)
        .expect("first page should bind a revision");
    simulation
        .settle_boundary(BoundaryRequest::at(simulation.time()))
        .expect("empty boundary should settle");
    let error = simulation
        .domain_record_page(&kind, None, 16, Some(first.revision))
        .expect_err("a page from an older authoritative revision must fail");
    assert_eq!(error.code, ErrorCode::SimulationRevisionConflict);
}

#[test]
fn domain_record_pages_validate_limits_cursors_and_empty_tails() {
    let (scenario, _) = demo_scenario();
    let simulation = Simulation::new(119, scenario).expect("demo should load");
    let kind = DomainRecordKind::new("test.technology", "attempt");
    let wrong_kind = DomainRecordKind::new("test.world", "unrelated");
    let wrong_cursor = DomainRecordRef::new(&wrong_kind.namespace, &wrong_kind.name, "cursor");

    let error = simulation
        .domain_record_page(&kind, None, 0, None)
        .expect_err("zero-sized pages must be rejected");
    assert_eq!(error.code, ErrorCode::ValueOutOfRange);
    let error = simulation
        .domain_record_page(&kind, Some(&wrong_cursor), 16, None)
        .expect_err("a cursor from another kind must be rejected");
    assert_eq!(error.code, ErrorCode::InvalidPayload);

    let after_tail = DomainRecordRef::new(&kind.namespace, &kind.name, "zzzz");
    let page = simulation
        .domain_record_page(&kind, Some(&after_tail), 16, None)
        .expect("a cursor beyond the final record should return an empty page");
    assert!(page.records.is_empty());
    assert!(page.next.is_none());
    assert_eq!(page.revision, simulation.revision());
}

#[test]
fn generic_evidence_identity_query_distinguishes_missing_ids() {
    let (scenario, _) = demo_scenario();
    let mut simulation = Simulation::new(127, scenario).expect("demo should load");
    simulation
        .settle_boundary(BoundaryRequest::at(simulation.time()))
        .expect("boundary should settle");
    let boundary = simulation
        .boundaries()
        .last()
        .expect("boundary evidence should exist")
        .id;
    assert!(simulation.evidence_exists(&EvidenceRef::Boundary(boundary)));
    assert_eq!(
        simulation.evidence_time(&EvidenceRef::Boundary(boundary)),
        Some(simulation.time())
    );
    assert!(
        !simulation.evidence_exists(&EvidenceRef::Boundary(BoundaryId::new(boundary.get() + 1)))
    );
    assert_eq!(
        simulation.evidence_time(&EvidenceRef::Boundary(BoundaryId::new(boundary.get() + 1))),
        None
    );
}

fn insert_query_fixture(
    records: &mut BTreeMap<DomainRecordRef, DomainRecord>,
    kind: DomainRecordKind,
    id: String,
) {
    let reference = DomainRecordRef { kind, id };
    records.insert(
        reference.clone(),
        DomainRecord {
            reference,
            owner: "test".to_owned(),
            class: DomainRecordClass::Record,
            version: 1,
            lifecycle: DomainRecordLifecycle::Active,
            payload: Value::Null,
            references: Vec::new(),
        },
    );
}
