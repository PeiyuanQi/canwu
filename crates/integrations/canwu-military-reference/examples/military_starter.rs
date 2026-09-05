use canwu_api::{
    Canwu, CommandEnvelope, CommandRequest, CommandRequestId, EntityRef, Issuer, SimDuration,
};
use canwu_military::{
    ForceId, MilitaryCommand, MilitaryNodeId, OccupationId, OperationId, military_command,
    military_plugin,
};
use canwu_military_reference::{demo_military_scenario, ruleset_profiles};
use canwu_reference_world::ReferenceWorldPlugin;

fn submit(
    canwu: &mut Canwu,
    sequence: u64,
    issuer: Issuer,
    command: MilitaryCommand,
) -> Result<(), Box<dyn std::error::Error>> {
    let envelope = CommandEnvelope::new(issuer, military_command(command)?);
    canwu.enqueue_command(
        canwu.time(),
        i32::try_from(sequence)?,
        CommandRequest::new(CommandRequestId::new(sequence), canwu.revision(), envelope),
    )?;
    canwu.advance_canonical(SimDuration::minutes(1))?;
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (scenario, ids) = demo_military_scenario()?;
    let (reference_world, military) = (ReferenceWorldPlugin, military_plugin());
    let mut canwu = Canwu::new_with_plugins(35, scenario, &[&reference_world, &military])?;
    let force = ForceId::new("canwu.military:force:field-1")?;
    let node = MilitaryNodeId::new(format!("canwu.military:node:{}", ids.central_territory))?;

    submit(
        &mut canwu,
        1,
        Issuer::Actor(ids.commander),
        MilitaryCommand::CreateForce {
            operation: canwu_military::MilitaryOperationKey::new(
                "canwu.military:op:create-field-1",
            )?,
            force: force.clone(),
            owner: EntityRef::Government(ids.government),
            location: node.clone(),
            authorized_strength: 2_000,
            branch: "infantry".to_owned(),
            commander: Some(ids.commander),
        },
    )?;
    submit(
        &mut canwu,
        2,
        Issuer::Actor(ids.commander),
        MilitaryCommand::Recruit {
            operation: canwu_military::MilitaryOperationKey::new(
                "canwu.military:op:recruit-field-1",
            )?,
            force: force.clone(),
            subunit: canwu_military::SubunitId::new("canwu.military:subunit:reserve")?,
            branch: "infantry".to_owned(),
            quantity: 400,
            expected_force_revision: 1,
            society_operation: Some("canwu.society:transfer:field-1".to_owned()),
        },
    )?;
    submit(
        &mut canwu,
        3,
        Issuer::Actor(ids.commander),
        MilitaryCommand::PlanOperation {
            operation: canwu_military::MilitaryOperationKey::new("canwu.military:op:plan-front")?,
            operation_id: OperationId::new("canwu.military:operation:front")?,
            owner: EntityRef::Government(ids.government),
            objective: "secure the eastern route".to_owned(),
            force: force.clone(),
            from: node.clone(),
            destination: MilitaryNodeId::new(format!(
                "canwu.military:node:{}",
                ids.eastern_territory
            ))?,
            tactic: "screen-and-advance".to_owned(),
        },
    )?;
    submit(
        &mut canwu,
        4,
        Issuer::Actor(ids.commander),
        MilitaryCommand::EstablishOccupation {
            operation: canwu_military::MilitaryOperationKey::new("canwu.military:op:occupy-east")?,
            occupation: OccupationId::new("canwu.military:occupation:east")?,
            force: force.clone(),
            node: MilitaryNodeId::new(format!("canwu.military:node:{}", ids.eastern_territory))?,
            expected_force_revision: 2,
        },
    )?;

    let snapshot = canwu.snapshot_json()?;
    let restored =
        Canwu::from_snapshot_json_with_plugins(&snapshot, &[&reference_world, &military])?;
    let replayed =
        Canwu::replay_from_journal(&[&reference_world, &military], &canwu.replay_journal())?;
    assert_eq!(restored.checkpoint_hash(), canwu.checkpoint_hash());
    assert_eq!(replayed.checkpoint_hash(), canwu.checkpoint_hash());
    let (riverine, industrial) = ruleset_profiles();
    println!(
        "military_gameplan=complete riverine={} industrial={} checkpoint={}",
        riverine.profile,
        industrial.profile,
        canwu.checkpoint_hash()
    );
    Ok(())
}
