use canwu_api::{
    Canwu, CommandEnvelope, CommandRequest, CommandRequestId, EntityRef, Issuer, SimDuration,
};
use canwu_military::{
    CombatId, CombatStateRecord, ForceId, MilitaryCommand, MilitaryNodeId, OccupationId,
    OccupationStateRecord, OperationId, combat_reference, military_command, military_plugin,
    occupation_reference,
};
use canwu_reference_world::{ReferenceWorldPlugin, demo_scenario};

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

#[test]
fn altered_operation_key_input_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
    let (scenario, ids) = demo_scenario()?;
    let world = ReferenceWorldPlugin;
    let military = military_plugin();
    let mut canwu = Canwu::new_with_plugins(35, scenario, &[&world, &military])?;
    let force = ForceId::new("canwu.military:test:idempotency")?;
    let node = MilitaryNodeId::new(format!("canwu.military:node:{}", ids.western_territory))?;
    submit(
        &mut canwu,
        1,
        Issuer::Actor(ids.commander),
        MilitaryCommand::CreateForce {
            operation: canwu_military::MilitaryOperationKey::new("canwu.military:test:key")?,
            force: force.clone(),
            owner: EntityRef::Government(ids.government),
            location: node.clone(),
            authorized_strength: 100,
            initial_strength: None,
            branch: "infantry".to_owned(),
            commander: Some(ids.commander),
        },
    )?;
    canwu.enqueue_command(
        canwu.time(),
        1,
        CommandRequest::new(
            CommandRequestId::new(2),
            canwu.revision(),
            CommandEnvelope::new(
                Issuer::Actor(ids.commander),
                military_command(MilitaryCommand::CreateForce {
                    operation: canwu_military::MilitaryOperationKey::new(
                        "canwu.military:test:key",
                    )?,
                    force,
                    owner: EntityRef::Government(ids.government),
                    location: node,
                    authorized_strength: 999,
                    initial_strength: None,
                    branch: "cavalry".to_owned(),
                    commander: Some(ids.commander),
                })?,
            ),
        ),
    )?;
    let error = canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect_err("altered operation key input must be rejected");
    assert_eq!(error.code, canwu_api::ErrorCode::IdempotencyConflict);
    Ok(())
}

#[test]
fn complete_military_lifecycle_is_persisted_and_replayed() -> Result<(), Box<dyn std::error::Error>>
{
    let (scenario, ids) = demo_scenario()?;
    let world = ReferenceWorldPlugin;
    let military = military_plugin();
    let mut canwu = Canwu::new_with_plugins(35, scenario, &[&world, &military])?;
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
            opposing_force: Some(defender),
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

    let snapshot = canwu.snapshot_json()?;
    let restored = Canwu::from_snapshot_json_with_plugins(&snapshot, &[&world, &military])?;
    let replayed = Canwu::replay_from_journal(&[&world, &military], &canwu.replay_journal())?;
    assert_eq!(restored.checkpoint_hash(), canwu.checkpoint_hash());
    assert_eq!(replayed.checkpoint_hash(), canwu.checkpoint_hash());
    Ok(())
}
