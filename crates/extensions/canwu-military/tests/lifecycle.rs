use canwu_api::{
    Canwu, CanwuError, CommandAttemptOutcome, CommandEnvelope, CommandRequest, CommandRequestId,
    EntityRef, ErrorCode, GovernmentId, Issuer, PersonId, PluginIngressRequest, Scenario,
    SimDuration, SimTime,
};
use canwu_military::{
    CombatId, CombatStateRecord, ForceId, ForceState, ForceStateRecord, ForceStatus,
    IntegrationStage, MAX_RECORDS, MILITARY_COMMAND_INGRESS, MILITARY_REJECTION_EVENT,
    MilitaryCommand, MilitaryCommandEnvelope, MilitaryLedger, MilitaryLedgerRecord, MilitaryNodeId,
    MilitaryOperationKey, MilitaryRecordMeta, OccupationId, OccupationStateRecord, OperationId,
    OperationPhase, OperationStateRecord, OutcomeDisposition, PLUGIN_NAME, ProviderDisposition,
    ProviderOutcome, ProviderOutcomeId, SubunitId, SubunitState, SubunitStatus, combat_reference,
    enqueue_provider_outcome, force_reference, input_digest, ledger_reference, military_command,
    military_plugin, occupation_reference, operation_reference, record_from,
};
use std::collections::BTreeMap;

#[derive(Clone, Copy)]
struct DemoIds {
    commander: PersonId,
    observer: PersonId,
    government: GovernmentId,
    western_territory: u64,
    eastern_territory: u64,
}

fn demo_scenario() -> (Scenario, DemoIds) {
    let ids = DemoIds {
        commander: PersonId::new(1),
        observer: PersonId::new(2),
        government: GovernmentId::new(1),
        western_territory: 1,
        eastern_territory: 3,
    };
    let entities = vec![
        EntityRef::Person(ids.commander),
        EntityRef::Person(ids.observer),
        EntityRef::Government(ids.government),
        EntityRef::Territory(canwu_api::TerritoryId::new(ids.western_territory)),
        EntityRef::Territory(canwu_api::TerritoryId::new(2)),
        EntityRef::Territory(canwu_api::TerritoryId::new(ids.eastern_territory)),
    ];
    (Scenario::new(SimTime::EPOCH, entities), ids)
}

fn submit(
    canwu: &mut Canwu,
    sequence: i32,
    issuer: Issuer,
    command: MilitaryCommand,
) -> Result<(), Box<dyn std::error::Error>> {
    canwu.enqueue_command(
        canwu.time(),
        sequence,
        CommandRequest::new(
            CommandRequestId::new(u64::try_from(sequence)?),
            canwu.revision(),
            CommandEnvelope::new(issuer, military_command(command)?),
        ),
    )?;
    canwu.advance_canonical(SimDuration::minutes(1))?;
    Ok(())
}

fn key(name: &str) -> Result<MilitaryOperationKey, CanwuError> {
    MilitaryOperationKey::new(format!("canwu.military:test:{name}"))
}

fn create_force(
    key_name: &str,
    force: &ForceId,
    commander: PersonId,
    location: &MilitaryNodeId,
) -> Result<MilitaryCommand, CanwuError> {
    Ok(MilitaryCommand::CreateForce {
        operation: key(key_name)?,
        force: force.clone(),
        owner: EntityRef::Government(GovernmentId::new(1)),
        location: location.clone(),
        authorized_strength: 500,
        initial_strength: None,
        branch: "infantry".to_owned(),
        commander: Some(commander),
    })
}

fn last_rejection(canwu: &Canwu) -> Option<ErrorCode> {
    match &canwu.command_attempts().last()?.outcome {
        CommandAttemptOutcome::Rejected { error } => Some(error.code.clone()),
        CommandAttemptOutcome::Accepted { .. } => None,
    }
}

fn ledger(canwu: &Canwu) -> Result<MilitaryLedger, Box<dyn std::error::Error>> {
    Ok(canwu
        .typed_domain_record(&ledger_reference())
        .ok_or("military ledger is missing")?
        .decode_payload::<MilitaryLedgerRecord>()?)
}

fn force_state(canwu: &Canwu, force: &ForceId) -> Result<ForceState, Box<dyn std::error::Error>> {
    Ok(canwu
        .typed_domain_record(&force_reference(force))
        .ok_or("force is missing")?
        .decode_payload::<ForceStateRecord>()?)
}

fn provider_outcome(at: SimTime, id: &str, operation: &str) -> Result<ProviderOutcome, CanwuError> {
    Ok(ProviderOutcome {
        meta: MilitaryRecordMeta::new(1, at, &())?,
        id: ProviderOutcomeId::new(format!("canwu.military:test:{id}"))?,
        operation: key(operation)?,
        provider_plugin: "canwu-law".to_owned(),
        provider_record: format!("canwu.law:record:{id}"),
        provider_version: 1,
        disposition: ProviderDisposition::Accepted,
        quantity: 1,
        digest: format!("digest:{id}"),
    })
}

#[test]
#[allow(clippy::too_many_lines)]
fn rejected_military_input_is_recorded_without_blocking_the_queue()
-> Result<(), Box<dyn std::error::Error>> {
    let (scenario, ids) = demo_scenario();
    let military = military_plugin();
    let mut canwu = Canwu::new_with_plugins(35, scenario, &[&military])?;
    let force = ForceId::new("canwu.military:test:garrison")?;
    let west = MilitaryNodeId::new(format!("canwu.military:node:{}", ids.western_territory))?;
    let occupation = OccupationId::new("canwu.military:test:occupation")?;
    submit(
        &mut canwu,
        1,
        Issuer::Actor(ids.commander),
        create_force("create", &force, ids.commander, &west)?,
    )?;

    // Reusing a settled key with different input is a recorded admission rejection.
    let mut altered = create_force("create", &force, ids.commander, &west)?;
    if let MilitaryCommand::CreateForce {
        authorized_strength,
        ..
    } = &mut altered
    {
        *authorized_strength = 999;
    }
    submit(&mut canwu, 2, Issuer::Actor(ids.commander), altered)?;
    assert_eq!(last_rejection(&canwu), Some(ErrorCode::IdempotencyConflict));

    // A stale force revision is rejected in phase 7 and recorded under its key.
    submit(
        &mut canwu,
        3,
        Issuer::Actor(ids.commander),
        MilitaryCommand::TrainAndEquip {
            operation: key("train-stale")?,
            force: force.clone(),
            expected_force_revision: 99,
            training_delta: 100,
            equipment_delta: 100,
        },
    )?;
    let stale = ledger(&canwu)?
        .outcomes
        .get(&key("train-stale")?)
        .cloned()
        .ok_or("stale command outcome is missing")?;
    assert_eq!(stale.disposition, OutcomeDisposition::Rejected);
    assert!(stale.message.contains("domain_record_version_conflict"));

    submit(
        &mut canwu,
        4,
        Issuer::Actor(ids.commander),
        MilitaryCommand::EstablishOccupation {
            operation: key("occupy")?,
            occupation: occupation.clone(),
            force: force.clone(),
            node: west,
            expected_force_revision: 1,
        },
    )?;
    for (sequence, name) in [(5, "admin-1"), (6, "admin-2")] {
        submit(
            &mut canwu,
            sequence,
            Issuer::Actor(ids.commander),
            MilitaryCommand::MilitaryAdministrationAction {
                operation: key(name)?,
                occupation: occupation.clone(),
                action: "appoint-magistrate".to_owned(),
                provider_plugin: "canwu-law".to_owned(),
                expected_provider_version: 1,
            },
        )?;
    }

    // A stray acknowledgement and two valid ones settle in the same pass.
    let at = canwu.time();
    for (id, operation) in [
        ("stray", "unknown"),
        ("ack-1", "admin-1"),
        ("ack-2", "admin-2"),
    ] {
        enqueue_provider_outcome(&mut canwu, at, provider_outcome(at, id, operation)?)?;
    }
    canwu.advance_canonical(SimDuration::minutes(1))?;
    let settled = ledger(&canwu)?;
    assert!(settled.pending.is_empty());
    assert!(settled.outcomes.contains_key(&key("admin-1")?));
    assert!(settled.outcomes.contains_key(&key("admin-2")?));
    let integration = canwu
        .typed_domain_record(&occupation_reference(&occupation))
        .ok_or("occupation is missing")?
        .decode_payload::<OccupationStateRecord>()?
        .integration;
    assert_eq!(integration, IntegrationStage::LegalRecognition);
    let rejections: Vec<&str> = canwu
        .events()
        .iter()
        .filter(|event| {
            event.kind.plugin_identity() == Some((PLUGIN_NAME, MILITARY_REJECTION_EVENT))
        })
        .map(|event| event.summary.as_str())
        .collect();
    assert_eq!(rejections.len(), 2);
    assert!(rejections[0].contains("train-stale"));
    assert!(rejections[1].contains("provider acknowledgement"));

    // Nothing stayed queued: a later valid command applies.
    submit(
        &mut canwu,
        7,
        Issuer::Actor(ids.commander),
        MilitaryCommand::TrainAndEquip {
            operation: key("train")?,
            force: force.clone(),
            expected_force_revision: 1,
            training_delta: 100,
            equipment_delta: 100,
        },
    )?;
    assert_eq!(force_state(&canwu, &force)?.training_per_mille, 100);

    let snapshot = canwu.snapshot_json()?;
    let restored = Canwu::from_snapshot_json_with_plugins(&snapshot, &[&military])?;
    let replayed = Canwu::replay_from_journal(&[&military], &canwu.replay_journal())?;
    assert_eq!(restored.checkpoint_hash(), canwu.checkpoint_hash());
    assert_eq!(replayed.checkpoint_hash(), canwu.checkpoint_hash());
    Ok(())
}

#[test]
fn special_operations_settle_on_success_and_failure() -> Result<(), Box<dyn std::error::Error>> {
    let military = military_plugin();
    let force = ForceId::new("canwu.military:test:raiders")?;
    let (mut completed, mut failed) = (0, 0);
    for run in 0..8 {
        let (scenario, ids) = demo_scenario();
        let mut canwu = Canwu::new_with_plugins(35, scenario, &[&military])?;
        let west = MilitaryNodeId::new(format!("canwu.military:node:{}", ids.western_territory))?;
        let east = MilitaryNodeId::new(format!("canwu.military:node:{}", ids.eastern_territory))?;
        submit(
            &mut canwu,
            1,
            Issuer::Actor(ids.commander),
            create_force("create", &force, ids.commander, &west)?,
        )?;
        let operation_id = OperationId::new(format!("canwu.military:test:raid-{run}"))?;
        submit(
            &mut canwu,
            2,
            Issuer::Actor(ids.commander),
            MilitaryCommand::ExecuteSpecialOperation {
                operation: key(&format!("raid-{run}"))?,
                operation_id: operation_id.clone(),
                force: force.clone(),
                objective: "seize the ford".to_owned(),
                target: east.clone(),
            },
        )?;
        canwu.advance_canonical(SimDuration::days(2))?;
        let operation = canwu
            .typed_domain_record(&operation_reference(&operation_id))
            .ok_or("operation is missing")?
            .decode_payload::<OperationStateRecord>()?;
        let state = force_state(&canwu, &force)?;
        assert_eq!(state.active_operation, None);
        match operation.phase {
            OperationPhase::Completed => {
                completed += 1;
                assert_eq!((state.location, state.status), (west, ForceStatus::Ready));
                let replayed = Canwu::replay_from_journal(&[&military], &canwu.replay_journal())?;
                assert_eq!(replayed.checkpoint_hash(), canwu.checkpoint_hash());
            }
            OperationPhase::Failed => {
                failed += 1;
                assert_eq!((state.location, state.status), (east, ForceStatus::Routing));
            }
            phase => return Err(format!("special operation ended in {phase:?}").into()),
        }
    }
    assert!(
        completed > 0 && failed > 0,
        "{completed} completed, {failed} failed"
    );
    Ok(())
}

#[test]
#[allow(clippy::too_many_lines)]
fn military_authority_is_checked_at_admission() -> Result<(), Box<dyn std::error::Error>> {
    let (scenario, ids) = demo_scenario();
    let military = military_plugin();
    let mut canwu = Canwu::new_with_plugins(35, scenario, &[&military])?;
    let force = ForceId::new("canwu.military:test:guard")?;
    let west = MilitaryNodeId::new(format!("canwu.military:node:{}", ids.western_territory))?;
    let occupation = OccupationId::new("canwu.military:test:occupation")?;
    submit(
        &mut canwu,
        1,
        Issuer::Actor(ids.commander),
        create_force("create", &force, ids.commander, &west)?,
    )?;

    let other = ForceId::new("canwu.military:test:other")?;
    submit(
        &mut canwu,
        2,
        Issuer::Actor(ids.observer),
        create_force("create-other", &other, ids.commander, &west)?,
    )?;
    assert_eq!(last_rejection(&canwu), Some(ErrorCode::InvalidAuthority));
    assert!(
        canwu
            .typed_domain_record(&force_reference(&other))
            .is_none()
    );

    submit(
        &mut canwu,
        3,
        Issuer::Actor(ids.observer),
        MilitaryCommand::TrainAndEquip {
            operation: key("train")?,
            force: force.clone(),
            expected_force_revision: 1,
            training_delta: 10,
            equipment_delta: 10,
        },
    )?;
    assert_eq!(last_rejection(&canwu), Some(ErrorCode::InvalidAuthority));

    submit(
        &mut canwu,
        4,
        Issuer::Actor(ids.commander),
        MilitaryCommand::AssignCommander {
            operation: key("assign-missing")?,
            force: force.clone(),
            commander: PersonId::new(99),
            expected_force_revision: 1,
        },
    )?;
    assert_eq!(last_rejection(&canwu), Some(ErrorCode::EntityNotFound));

    submit(
        &mut canwu,
        5,
        Issuer::Actor(ids.commander),
        MilitaryCommand::EstablishOccupation {
            operation: key("occupy")?,
            occupation: occupation.clone(),
            force: force.clone(),
            node: west,
            expected_force_revision: 1,
        },
    )?;
    assert_eq!(last_rejection(&canwu), None);
    submit(
        &mut canwu,
        6,
        Issuer::Actor(ids.observer),
        MilitaryCommand::SetOccupationPolicy {
            operation: key("policy")?,
            occupation: occupation.clone(),
            policy_revision: 1,
            security_per_mille: 0,
            collaboration_per_mille: 0,
            extraction_burden_per_mille: 1_000,
        },
    )?;
    assert_eq!(last_rejection(&canwu), Some(ErrorCode::InvalidAuthority));
    submit(
        &mut canwu,
        7,
        Issuer::Actor(ids.commander),
        MilitaryCommand::AdvanceTick {
            operation: None,
            occupation: Some(occupation),
            operation_key: key("tick")?,
        },
    )?;
    assert_eq!(last_rejection(&canwu), Some(ErrorCode::InvalidAuthority));

    // Hosts cannot enqueue the internal packet that carries admitted commands.
    let forged = MilitaryCommand::AssignCommander {
        operation: key("forged")?,
        force: force.clone(),
        commander: ids.observer,
        expected_force_revision: 1,
    };
    let payload = serde_json::json!({
        "envelope": MilitaryCommandEnvelope {
            input_digest: input_digest(&forged)?,
            command: forged,
        }
    });
    let at = canwu.time();
    let refused = canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            PLUGIN_NAME,
            MILITARY_COMMAND_INGRESS,
            at,
            payload,
        ))
        .expect_err("hosts must not author admitted military commands");
    assert_eq!(refused.code, ErrorCode::InvalidAuthority);
    assert_eq!(force_state(&canwu, &force)?.commander, Some(ids.commander));
    Ok(())
}

#[test]
#[allow(clippy::too_many_lines)]
fn complete_military_lifecycle_is_persisted_and_replayed() -> Result<(), Box<dyn std::error::Error>>
{
    let (scenario, ids) = demo_scenario();
    let military = military_plugin();
    let mut canwu = Canwu::new_with_plugins(35, scenario, &[&military])?;
    let attacker = ForceId::new("canwu.military:test:attacker")?;
    let defender = ForceId::new("canwu.military:test:defender")?;
    let west = MilitaryNodeId::new(format!("canwu.military:node:{}", ids.western_territory))?;
    let east = MilitaryNodeId::new(format!("canwu.military:node:{}", ids.eastern_territory))?;

    submit(
        &mut canwu,
        1,
        Issuer::Actor(ids.commander),
        MilitaryCommand::CreateForce {
            operation: canwu_military::MilitaryOperationKey::new("canwu.military:test:create-a")?,
            force: attacker.clone(),
            owner: EntityRef::Government(ids.government),
            location: west.clone(),
            authorized_strength: 2_500,
            initial_strength: Some(2_000),
            branch: "infantry".to_owned(),
            commander: Some(ids.commander),
        },
    )?;
    submit(
        &mut canwu,
        2,
        Issuer::Actor(ids.commander),
        MilitaryCommand::Recruit {
            operation: canwu_military::MilitaryOperationKey::new("canwu.military:test:recruit-a")?,
            force: attacker.clone(),
            subunit: canwu_military::SubunitId::new("canwu.military:test:reserve")?,
            branch: "infantry".to_owned(),
            quantity: 400,
            expected_force_revision: 1,
            society_operation: None,
        },
    )?;
    submit(
        &mut canwu,
        3,
        Issuer::Actor(ids.observer),
        MilitaryCommand::CreateForce {
            operation: canwu_military::MilitaryOperationKey::new("canwu.military:test:create-d")?,
            force: defender.clone(),
            owner: EntityRef::Government(ids.government),
            location: east.clone(),
            authorized_strength: 100,
            initial_strength: None,
            branch: "infantry".to_owned(),
            commander: Some(ids.observer),
        },
    )?;
    submit(
        &mut canwu,
        4,
        Issuer::Actor(ids.commander),
        MilitaryCommand::OrderMarch {
            operation: canwu_military::MilitaryOperationKey::new("canwu.military:test:march")?,
            force: attacker.clone(),
            operation_id: OperationId::new("canwu.military:test:operation")?,
            destination: east,
            objective: "secure route".to_owned(),
            tactic: "screen-and-advance".to_owned(),
            opposing_force: Some(defender.clone()),
            expected_force_revision: 2,
        },
    )?;
    canwu.advance_canonical(SimDuration::days(5))?;

    let combat = canwu
        .typed_domain_record(&combat_reference(&CombatId::new(
            "canwu.military:combat:canwu.military:test:operation",
        )?))
        .ok_or("combat was not persisted")?
        .decode_payload::<CombatStateRecord>()?;
    assert!(combat.result.is_some());
    let occupation = canwu
        .typed_domain_record(&occupation_reference(&OccupationId::new(
            "canwu.military:occupation:canwu.military:test:operation",
        )?))
        .ok_or("occupation was not persisted")?
        .decode_payload::<OccupationStateRecord>()?;
    assert!(occupation.military_control_per_mille > 0);

    // Combat losses keep subunits reconciled, so both forces stay commandable.
    force_state(&canwu, &defender)?.validate()?;
    let attacker_state = force_state(&canwu, &attacker)?;
    attacker_state.validate()?;
    submit(
        &mut canwu,
        5,
        Issuer::Actor(ids.commander),
        MilitaryCommand::TrainAndEquip {
            operation: key("drill-after-battle")?,
            force: attacker.clone(),
            expected_force_revision: attacker_state.meta.revision,
            training_delta: 10,
            equipment_delta: 10,
        },
    )?;
    let drill = ledger(&canwu)?
        .outcomes
        .get(&key("drill-after-battle")?)
        .map(|outcome| outcome.disposition);
    assert_eq!(drill, Some(OutcomeDisposition::Accepted));

    let snapshot = canwu.snapshot_json()?;
    let restored = Canwu::from_snapshot_json_with_plugins(&snapshot, &[&military])?;
    let replayed = Canwu::replay_from_journal(&[&military], &canwu.replay_journal())?;
    assert_eq!(restored.checkpoint_hash(), canwu.checkpoint_hash());
    assert_eq!(replayed.checkpoint_hash(), canwu.checkpoint_hash());
    Ok(())
}

#[test]
fn many_forces_stay_within_record_and_report_limits() -> Result<(), Box<dyn std::error::Error>> {
    let (scenario, ids) = demo_scenario();
    let west = MilitaryNodeId::new(format!("canwu.military:node:{}", ids.western_territory))?;
    let mut records = Vec::new();
    for index in 0..MAX_RECORDS - 1 {
        let id = ForceId::new(format!("canwu.military:test:bulk-{index:05}"))?;
        let unit = SubunitState {
            id: SubunitId::new(format!("canwu.military:test:bulk-{index:05}:initial"))?,
            branch: "infantry".to_owned(),
            strength: 1,
            training_per_mille: 0,
            equipment_per_mille: 0,
            fatigue_per_mille: 0,
            status: SubunitStatus::Active,
        };
        let state = ForceState {
            meta: MilitaryRecordMeta::new(1, SimTime::EPOCH, &())?,
            id: id.clone(),
            owner: EntityRef::Government(ids.government),
            formation_parent: None,
            location: west.clone(),
            // More commanded forces than the kernel's per-system batch limit.
            commander: (index < 100).then_some(ids.commander),
            subunits: BTreeMap::from([(unit.id.clone(), unit)]),
            authorized_strength: 1,
            actual_strength: 1,
            training_per_mille: 0,
            equipment_per_mille: 0,
            fatigue_per_mille: 0,
            supply_per_mille: 1_000,
            morale_per_mille: 500,
            discipline_per_mille: 500,
            cohesion_per_mille: 500,
            loyalty_per_mille: 500,
            casualties: 0,
            missing: 0,
            prisoners: 0,
            deserters: 0,
            replacements_pending: 0,
            transport_capacity: 0,
            active_operation: None,
            active_order: None,
            prepared_ambush: None,
            status: ForceStatus::Ready,
        };
        records.push(record_from(force_reference(&id), &state, SimTime::EPOCH)?);
    }
    let military = military_plugin();
    let mut canwu =
        Canwu::new_with_plugins(35, scenario.with_domain_records(records), &[&military])?;

    // The last free slot is filled; the next force is a recorded rejection.
    let last = ForceId::new("canwu.military:test:last")?;
    submit(
        &mut canwu,
        1,
        Issuer::Actor(ids.commander),
        create_force("create-last", &last, ids.commander, &west)?,
    )?;
    assert!(canwu.typed_domain_record(&force_reference(&last)).is_some());
    let over = ForceId::new("canwu.military:test:over")?;
    submit(
        &mut canwu,
        2,
        Issuer::Actor(ids.commander),
        create_force("create-over", &over, ids.commander, &west)?,
    )?;
    assert!(canwu.typed_domain_record(&force_reference(&over)).is_none());
    let outcome = ledger(&canwu)?
        .outcomes
        .get(&key("create-over")?)
        .cloned()
        .ok_or("over-limit outcome is missing")?;
    assert_eq!(outcome.disposition, OutcomeDisposition::Rejected);
    assert!(outcome.message.contains("MAX_RECORDS"));
    Ok(())
}
