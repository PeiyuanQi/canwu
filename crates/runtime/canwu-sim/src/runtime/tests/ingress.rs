use super::*;

#[test]
fn canonical_ingress_orders_commands_packets_and_calendar_work() {
    let (scenario, ids) = demo_scenario();
    let plugin = CanonicalIngressPlugin;
    let mut simulation =
        Simulation::new(41, scenario.clone()).expect("ingress fixture should load");
    simulation
        .register_plugin(&plugin)
        .expect("canonical ingress plugin should register");
    let due_at = SimTime::EPOCH
        .checked_add(SimDuration::hours(1))
        .expect("fixture due time should be representable");

    let information = simulation
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            "canonical-ingress",
            "report",
            due_at,
            serde_json::json!({ "label": "field report" }),
        ))
        .expect("information should queue");
    let low_priority = simulation
        .enqueue_plugin_ingress(
            PluginIngressRequest::new(
                "canonical-ingress",
                "dispatch",
                due_at,
                serde_json::json!({ "label": "routine dispatch" }),
            )
            .with_priority(-10),
        )
        .expect("low-priority communication should queue");
    let acknowledgement = simulation
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            "canonical-ingress",
            "ack",
            due_at,
            serde_json::json!({ "label": "received" }),
        ))
        .expect("acknowledgement should queue");
    let high_priority = simulation
        .enqueue_plugin_ingress(
            PluginIngressRequest::new(
                "canonical-ingress",
                "dispatch",
                due_at,
                serde_json::json!({ "label": "urgent dispatch" }),
            )
            .with_priority(10),
        )
        .expect("high-priority communication should queue");
    let calendar = simulation
        .schedule_calendar_boundary(due_at, vec![SystemCadence::Daily])
        .expect("daily calendar work should queue");
    let command_request = CommandRequest::new(
        CommandRequestId::new(77),
        0,
        move_order(&ids).at_time(due_at),
    );
    let command = simulation
        .enqueue_command(due_at, 0, command_request.clone())
        .expect("command should queue");
    let future_at = due_at
        .checked_add(SimDuration::hours(1))
        .expect("future ingress time should be representable");
    let future = simulation
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            "canonical-ingress",
            "report",
            future_at,
            serde_json::json!({ "label": "future report" }),
        ))
        .expect("future information should queue without becoming visible early");
    assert_eq!(
        simulation
            .enqueue_command(due_at, 0, command_request.clone())
            .expect("an exact queued retry should be idempotent"),
        command
    );
    assert_eq!(simulation.ingress_log().len(), 7);
    let collision = simulation
        .enqueue_command(due_at, 1, command_request)
        .expect_err("a queued request-ID collision must fail closed");
    assert_eq!(collision.code, ErrorCode::IdempotencyConflict);
    let mixed_before = simulation.snapshot();
    let mixed = simulation
        .submit(move_order(&ids))
        .expect_err("legacy commands cannot bypass queued tracked ingress");
    assert_eq!(mixed.code, ErrorCode::MixedCommandIngress);
    assert_eq!(simulation.snapshot(), mixed_before);

    let before_legacy_advance = simulation.snapshot();
    let error = simulation
        .advance(SimDuration::hours(1))
        .expect_err("legacy advancement cannot skip canonical ingress");
    assert_eq!(error.code, ErrorCode::InvalidBoundary);
    assert_eq!(simulation.snapshot(), before_legacy_advance);

    let before_skipped_boundary = simulation.snapshot();
    let error = simulation
        .settle_boundary(BoundaryRequest::at(future_at))
        .expect_err("manual settlement cannot skip an earlier ingress due time");
    assert_eq!(error.code, ErrorCode::InvalidBoundary);
    assert_eq!(simulation.snapshot(), before_skipped_boundary);

    let pending_json = simulation
        .snapshot_json()
        .expect("pending ingress should serialize");
    let mut restored = Simulation::from_snapshot_json_with_plugins(&pending_json, &[&plugin])
        .expect("pending ingress should restore with its plugin contract");
    assert_eq!(simulation.snapshot(), restored.snapshot());

    let receipts = simulation
        .advance_canonical(SimDuration::hours(1))
        .expect("canonical advancement should settle every due input");
    let restored_receipts = restored
        .advance_canonical(SimDuration::hours(1))
        .expect("restored canonical ingress should settle identically");
    assert_eq!(receipts, restored_receipts);
    assert_eq!(simulation.snapshot(), restored.snapshot());
    assert_eq!(receipts.len(), 1);

    let boundary = simulation
        .boundaries()
        .last()
        .expect("canonical advancement should publish a boundary");
    let mut altered_boundary = boundary.clone();
    altered_boundary.admitted_ingress.pop();
    assert_ne!(
        compute_boundary_hash(boundary).expect("boundary evidence should hash"),
        compute_boundary_hash(&altered_boundary).expect("altered boundary evidence should hash"),
        "canonical admission evidence must be committed by the boundary chain",
    );
    assert_eq!(boundary.cadences, vec![SystemCadence::Daily]);
    assert_eq!(
        boundary.admitted_ingress,
        vec![
            command.ingress_id,
            high_priority.ingress_id,
            low_priority.ingress_id,
            acknowledgement.ingress_id,
            information.ingress_id,
            calendar.ingress_id,
        ]
    );
    assert_eq!(boundary.admitted_attempts, vec![CommandAttemptId::new(1)]);
    assert_eq!(boundary.admitted_commands, vec![CommandId::new(1)]);
    assert!(!boundary.admitted_ingress.contains(&future.ingress_id));
    let snapshot = simulation.snapshot();
    assert!(snapshot.plugin_components.iter().any(|component| {
        component.state == StateKey::new("ingress-fixture", "received")
            && component.value
                == serde_json::json!([
                    "communication:dispatch:10",
                    "communication:dispatch:-10",
                    "acknowledgement:ack:0",
                    "information:report:0"
                ])
    }));
    assert!(snapshot.plugin_components.iter().any(|component| {
        component.state == StateKey::new("ingress-fixture", "calendar")
            && component.value == Value::Bool(true)
    }));

    let post_boundary_due = future_at
        .checked_add(SimDuration::hours(1))
        .expect("post-boundary ingress time should be representable");
    simulation
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            "canonical-ingress",
            "report",
            post_boundary_due,
            serde_json::json!({ "label": "post-boundary report" }),
        ))
        .expect("future ingress may be authored after a completed boundary");
    let post_boundary_snapshot = simulation.snapshot();
    let post_boundary_restored =
        Simulation::from_snapshot_with_plugins(post_boundary_snapshot.clone(), &[&plugin])
            .expect("post-boundary pending ingress must not invalidate its own snapshot");
    assert_eq!(post_boundary_restored.snapshot(), post_boundary_snapshot);

    let late_before = simulation.snapshot();
    let late = simulation
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            "canonical-ingress",
            "report",
            SimTime::EPOCH,
            serde_json::json!({ "label": "late report" }),
        ))
        .expect_err("late ingress cannot rewrite an already committed boundary");
    assert_eq!(late.code, ErrorCode::LateIngress);
    assert_eq!(simulation.snapshot(), late_before);

    let journal = simulation.replay_journal();
    let replayed = Simulation::replay_from_journal_with_scenario(scenario, &[&plugin], &journal)
        .expect("canonical ingress should replay in its recorded environment");
    assert_eq!(simulation.snapshot(), replayed.snapshot());

    let mut reordered = simulation.snapshot();
    reordered.boundaries[0].admitted_ingress.swap(0, 1);
    rehash_tampered_snapshot(&mut reordered);
    let error = Simulation::from_snapshot_with_plugins(reordered, &[&plugin])
        .err()
        .expect("a rehashed noncanonical ingress order must not load");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let mut predating_issue_cut = simulation.snapshot();
    let last = predating_issue_cut
        .ingress
        .last_mut()
        .expect("the fixture retains post-boundary ingress");
    last.issued_at = SimTime::EPOCH;
    rehash_tampered_snapshot(&mut predating_issue_cut);
    let error = Simulation::from_snapshot_with_plugins(predating_issue_cut, &[&plugin])
        .err()
        .expect("ingress cannot predate its declared boundary issue cut");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let mut skipped_due_time = simulation.snapshot();
    let information_record = skipped_due_time
        .ingress
        .iter_mut()
        .find(|record| record.id == information.ingress_id)
        .expect("the information ingress should remain in the journal");
    information_record.due_at = SimTime::EPOCH;
    rehash_tampered_snapshot(&mut skipped_due_time);
    let error = Simulation::from_snapshot_with_plugins(skipped_due_time, &[&plugin])
        .err()
        .expect("a boundary cannot be forged past an earlier due ingress time");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
}

#[test]
fn command_ingress_precedes_equal_time_internal_scheduled_work() {
    let (scenario, ids) = demo_scenario();
    let mut simulation = Simulation::new(47, scenario).expect("ordering fixture should load");
    simulation
        .enqueue_command(
            SimTime::EPOCH,
            0,
            CommandRequest::new(CommandRequestId::new(1), 0, move_order(&ids)),
        )
        .expect("the initial movement should queue");
    simulation
        .step_canonical()
        .expect("the movement boundary should settle")
        .expect("the queued movement supplies due work");
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

    simulation
        .step_canonical()
        .expect("the equal-time boundary should settle")
        .expect("the arrival and command are both due");
    let attempt = simulation
        .command_attempts()
        .last()
        .expect("the queued command should leave attempt evidence");
    assert!(matches!(
        &attempt.outcome,
        CommandAttemptOutcome::Rejected { error }
            if error.code == ErrorCode::InvalidAuthority
                && error.message.contains("already moving")
    ));
    assert_eq!(
        simulation
            .world()
            .army(ids.army)
            .expect("the army should remain present")
            .location,
        ids.eastern_territory,
        "the scheduled arrival executes after the command-class admission decision",
    );
}

#[test]
fn exact_replay_cannot_advance_past_unadmitted_due_ingress() {
    let (scenario, _) = demo_scenario();
    let mut simulation = Simulation::new(49, scenario.clone()).expect("replay fixture should load");
    let due_at = SimTime::EPOCH
        .checked_add(SimDuration::hours(1))
        .expect("due time should be representable");
    simulation
        .schedule_calendar_boundary(due_at, vec![SystemCadence::Daily])
        .expect("calendar ingress should queue");
    let forged_final = due_at
        .checked_add(SimDuration::hours(1))
        .expect("forged final time should be representable");
    let mut forged_snapshot = simulation.snapshot();
    forged_snapshot.now = forged_final;
    refresh_snapshot_commitments_and_checkpoint(&mut forged_snapshot);
    let mut journal = simulation.replay_journal();
    journal.final_time = forged_final;
    journal.checkpoint_hash = forged_snapshot.checkpoint_hash;

    let error = Simulation::replay_from_journal_with_scenario(scenario, &[], &journal)
        .err()
        .expect("replay must not cross unadmitted due ingress");
    assert_eq!(error.code, ErrorCode::InvalidBoundary);
}

#[test]
fn snapshot_ingress_reconstructs_ordered_command_and_calendar_effects() {
    let (scenario, ids) = demo_scenario();
    let mut commands = Simulation::new(51, scenario.clone()).expect("command fixture should load");
    for (request_id, revision, priority, morale) in [(1, 0, 10, 80), (2, 1, 0, 90)] {
        let envelope = CommandEnvelope::new(
            Issuer::Debug,
            Command::DebugSetArmyMorale {
                army: ids.army,
                morale,
            },
        )
        .at_time(SimTime::EPOCH);
        commands
            .enqueue_command(
                SimTime::EPOCH,
                priority,
                CommandRequest::new(CommandRequestId::new(request_id), revision, envelope),
            )
            .expect("ordered command should queue");
    }
    commands
        .step_canonical()
        .expect("command boundary should settle")
        .expect("commands supply due work");
    commands
        .schedule_calendar_boundary(
            SimTime::EPOCH
                .checked_add(SimDuration::hours(1))
                .expect("future time should be representable"),
            vec![SystemCadence::Daily],
        )
        .expect("future ingress keeps the snapshot beyond its boundary head");
    let mut reordered_commands = commands.snapshot();
    reordered_commands.ingress[0].priority = 0;
    reordered_commands.ingress[1].priority = 10;
    reordered_commands.boundaries[0].admitted_ingress.swap(0, 1);
    rehash_tampered_snapshot(&mut reordered_commands);
    let error = Simulation::from_snapshot(reordered_commands)
        .err()
        .expect("queue order cannot be detached from command-attempt order");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let mut relabeled_attempt = commands.snapshot();
    relabeled_attempt.command_attempts[0].ingress = CommandIngress::FrozenReplay;
    rehash_tampered_snapshot(&mut relabeled_attempt);
    let error = Simulation::from_snapshot(relabeled_attempt)
        .err()
        .expect("queued command attempts must retain live-request provenance");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let mut calendar = Simulation::new(53, scenario).expect("calendar fixture should load");
    calendar
        .schedule_calendar_boundary(SimTime::EPOCH, vec![SystemCadence::Daily])
        .expect("calendar work should queue");
    calendar
        .step_canonical()
        .expect("calendar boundary should settle")
        .expect("calendar work supplies a boundary");
    calendar
        .schedule_calendar_boundary(
            SimTime::EPOCH
                .checked_add(SimDuration::hours(1))
                .expect("future time should be representable"),
            vec![SystemCadence::Daily],
        )
        .expect("future calendar work keeps the snapshot beyond its boundary head");
    let mut omitted_calendar = calendar.snapshot();
    omitted_calendar.boundaries[0].cadences.clear();
    rehash_tampered_snapshot(&mut omitted_calendar);
    let error = Simulation::from_snapshot(omitted_calendar)
        .err()
        .expect("admitted calendar work must appear in boundary cadence evidence");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
}

#[test]
fn generated_ingress_delay_must_be_representable() {
    let (mut scenario, _) = demo_scenario();
    let earliest = SimTime::from_minutes(i64::MIN);
    scenario.start_time = earliest;
    for actor in scenario.knowledge.actors.values_mut() {
        for army in actor.armies.values_mut() {
            army.observed_at = earliest;
            army.learned_at = earliest;
        }
    }
    let plugin = GeneratedIngressPlugin;
    let mut simulation =
        Simulation::new(55, scenario).expect("extreme-time ingress fixture should load");
    simulation
        .register_plugin(&plugin)
        .expect("generated ingress plugin should register");
    simulation
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            "generated-ingress",
            "dispatch",
            earliest,
            serde_json::json!({ "label": "dispatch" }),
        ))
        .expect("extreme-time dispatch should queue");
    simulation
        .step_canonical()
        .expect("extreme-time boundary should settle")
        .expect("dispatch supplies due work");
    let mut overflow = simulation.snapshot();
    overflow.ingress[1].due_at = SimTime::from_minutes(i64::MAX);
    rehash_tampered_snapshot(&mut overflow);
    let error = Simulation::from_snapshot_with_plugins(overflow, &[&plugin])
        .err()
        .expect("generated delay must fit the simulation duration domain");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
}

#[test]
fn boundary_generated_zero_delay_ingress_waits_for_the_next_same_time_boundary() {
    let (scenario, _) = demo_scenario();
    let plugin = GeneratedIngressPlugin;
    let mut simulation =
        Simulation::new(43, scenario.clone()).expect("generated ingress fixture should load");
    simulation
        .register_plugin(&plugin)
        .expect("generated ingress plugin should register");
    let dispatch = simulation
        .enqueue_plugin_ingress(
            PluginIngressRequest::new(
                "generated-ingress",
                "dispatch",
                SimTime::EPOCH,
                serde_json::json!({ "label": "dispatch" }),
            )
            .with_entity(EntityRef::Person(PersonId::new(1))),
        )
        .expect("dispatch should queue");

    let first = simulation
        .step_canonical()
        .expect("the first canonical boundary should settle")
        .expect("the dispatch supplies due work");
    assert_eq!(first.settled_at, SimTime::EPOCH);
    assert_eq!(first.generated_ingress, vec![IngressId::new(2)]);
    assert_eq!(simulation.boundaries().len(), 1);
    assert_eq!(
        simulation.boundaries()[0].admitted_ingress,
        vec![dispatch.ingress_id]
    );
    assert_eq!(
        simulation.boundaries()[0]
            .generated_ingress
            .iter()
            .map(|generation| generation.ingress)
            .collect::<Vec<_>>(),
        vec![IngressId::new(2)],
    );
    let generation = &simulation.boundaries()[0].generated_ingress[0];
    assert_eq!(generation.plugin, "generated-ingress");
    assert_eq!(generation.system, "relay-ingress");
    assert_eq!(generation.phase, BoundaryPhase::DomainDeltaProposal);
    assert_eq!(generation.visibility, StateVisibility::SameBoundary);
    let mut altered_boundary = simulation.boundaries()[0].clone();
    altered_boundary.generated_ingress.clear();
    assert_ne!(
        compute_boundary_hash(&simulation.boundaries()[0])
            .expect("generated ingress evidence should hash"),
        compute_boundary_hash(&altered_boundary).expect("altered generation evidence should hash"),
        "generated ingress evidence must be committed by the boundary chain",
    );
    assert!(
        !simulation
            .snapshot()
            .plugin_components
            .iter()
            .any(|component| {
                component.state == StateKey::new("generated-ingress-fixture", "received")
            })
    );

    let pending = simulation.snapshot();
    let mut restored = Simulation::from_snapshot_with_plugins(pending.clone(), &[&plugin])
        .expect("a pending generated acknowledgement should restore");
    assert_eq!(restored.snapshot(), pending);

    let second = simulation
        .step_canonical()
        .expect("the generated acknowledgement boundary should settle")
        .expect("the acknowledgement remains due at the same simulation time");
    let restored_second = restored
        .step_canonical()
        .expect("the restored acknowledgement boundary should settle")
        .expect("the restored acknowledgement remains due");
    assert_eq!(second, restored_second);
    assert_eq!(second.settled_at, SimTime::EPOCH);
    assert!(second.generated_ingress.is_empty());
    assert_eq!(simulation.boundaries().len(), 2);
    assert_eq!(
        simulation.boundaries()[1].admitted_ingress,
        vec![IngressId::new(2)]
    );
    assert!(
        simulation
            .snapshot()
            .plugin_components
            .iter()
            .any(|component| {
                component.state == StateKey::new("generated-ingress-fixture", "received")
                    && component.value == Value::Bool(true)
            })
    );
    assert_eq!(simulation.snapshot(), restored.snapshot());

    let journal = simulation.replay_journal();
    let replayed = Simulation::replay_from_journal_with_scenario(scenario, &[&plugin], &journal)
        .expect("boundary-generated ingress should replay from its producing system");
    assert_eq!(simulation.snapshot(), replayed.snapshot());

    let mut missing_generation_evidence = simulation.snapshot();
    missing_generation_evidence.boundaries[0]
        .generated_ingress
        .clear();
    rehash_tampered_snapshot(&mut missing_generation_evidence);
    let error = Simulation::from_snapshot_with_plugins(missing_generation_evidence, &[&plugin])
        .err()
        .expect("boundary-caused ingress without producer evidence must not load");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let mut false_producer = simulation.snapshot();
    false_producer.boundaries[0].generated_ingress[0].phase = BoundaryPhase::StrategicAggregation;
    rehash_tampered_snapshot(&mut false_producer);
    let error = Simulation::from_snapshot_with_plugins(false_producer, &[&plugin])
        .err()
        .expect("generated ingress must retain exact producer-stage provenance");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
}

#[test]
fn cross_plugin_ingress_requires_a_declared_target_and_waits_for_next_admission() {
    let (scenario, _) = demo_scenario();
    let producer = CrossPluginProducer;
    let consumer = CrossPluginConsumer;
    let mut simulation = Simulation::new(59, scenario.clone())
        .expect("cross-plugin ingress fixture should initialize");
    simulation
        .register_plugin(&producer)
        .expect("producer should register");
    simulation
        .register_plugin(&consumer)
        .expect("consumer should register");
    let start = simulation
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            "cross-producer",
            "start",
            SimTime::EPOCH,
            serde_json::json!({ "label": "start" }),
        ))
        .expect("producer ingress should queue");

    let first = simulation
        .step_canonical()
        .expect("producer boundary should settle")
        .expect("producer ingress is due");
    assert_eq!(
        simulation.boundaries()[0].admitted_ingress,
        vec![start.ingress_id]
    );
    assert_eq!(first.generated_ingress, vec![IngressId::new(2)]);
    assert!(
        simulation
            .snapshot()
            .plugin_components
            .iter()
            .all(|component| component.state != StateKey::new("cross-consumer", "received")),
        "zero-delay cross-plugin ingress must not recurse into the producing boundary",
    );

    let second = simulation
        .step_canonical()
        .expect("consumer boundary should settle")
        .expect("generated consumer ingress remains due");
    assert_eq!(second.settled_at, SimTime::EPOCH);
    assert_eq!(
        simulation.boundaries()[1].admitted_ingress,
        vec![IngressId::new(2)]
    );
    assert!(
        simulation
            .snapshot()
            .plugin_components
            .iter()
            .any(|component| {
                component.state == StateKey::new("cross-consumer", "received")
                    && component.value == Value::Bool(true)
            })
    );

    let restored =
        Simulation::from_snapshot_with_plugins(simulation.snapshot(), &[&producer, &consumer])
            .expect("cross-plugin ingress provenance should survive snapshot restoration");
    assert_eq!(simulation.snapshot(), restored.snapshot());

    let replayed = Simulation::replay_from_journal_with_scenario(
        scenario,
        &[&producer, &consumer],
        &simulation.replay_journal(),
    )
    .expect("cross-plugin ingress should replay exactly");
    assert_eq!(simulation.snapshot(), replayed.snapshot());
}
