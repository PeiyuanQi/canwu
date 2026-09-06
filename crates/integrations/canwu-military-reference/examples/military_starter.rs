use canwu_api::{
    Canwu, CommandEnvelope, CommandRequest, CommandRequestId, EntityRef, Issuer, SimDuration,
};
use canwu_military::{
    CombatId, CombatStateRecord, ForceId, IntegrationStage, MilitaryCommand, MilitaryNodeId,
    OccupationId, OccupationStateRecord, OperationId, combat_reference, military_command,
    military_plugin, occupation_reference,
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

#[allow(clippy::too_many_lines)]
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
            authorized_strength: 2_500,
            initial_strength: Some(2_000),
            branch: "levy_infantry".to_owned(),
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
            branch: "levy_infantry".to_owned(),
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
            tactic: "crossing_assault".to_owned(),
            opposing_force: None,
        },
    )?;
    submit(
        &mut canwu,
        4,
        Issuer::Actor(ids.observer),
        MilitaryCommand::CreateForce {
            operation: canwu_military::MilitaryOperationKey::new(
                "canwu.military:op:create-defender",
            )?,
            force: ForceId::new("canwu.military:force:defender")?,
            owner: EntityRef::Government(ids.government),
            location: MilitaryNodeId::new(format!(
                "canwu.military:node:{}",
                ids.eastern_territory
            ))?,
            authorized_strength: 100,
            initial_strength: None,
            branch: "levy_infantry".to_owned(),
            commander: Some(ids.observer),
        },
    )?;
    submit(
        &mut canwu,
        5,
        Issuer::Actor(ids.commander),
        MilitaryCommand::OrderMarch {
            operation: canwu_military::MilitaryOperationKey::new("canwu.military:op:march-front")?,
            force: force.clone(),
            operation_id: OperationId::new("canwu.military:operation:march-front")?,
            destination: MilitaryNodeId::new(format!(
                "canwu.military:node:{}",
                ids.eastern_territory
            ))?,
            objective: "secure the eastern route".to_owned(),
            tactic: "crossing_assault".to_owned(),
            opposing_force: Some(ForceId::new("canwu.military:force:defender")?),
            expected_force_revision: 2,
        },
    )?;
    canwu.advance_canonical(SimDuration::days(5))?;
    let combat = canwu
        .typed_domain_record(&combat_reference(&CombatId::new(
            "canwu.military:combat:canwu.military:operation:march-front",
        )?))
        .ok_or("combat record was not created")?
        .decode_payload::<CombatStateRecord>()?;
    assert!(
        combat.result.is_some(),
        "combat must reach a terminal result"
    );
    let occupation = canwu
        .typed_domain_record(&occupation_reference(&OccupationId::new(
            "canwu.military:occupation:canwu.military:operation:march-front",
        )?))
        .ok_or("victory occupation was not created")?
        .decode_payload::<OccupationStateRecord>()?;
    assert!(
        occupation.integration != IntegrationStage::Unadministered,
        "occupation must retain a military-control stage"
    );
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
