use canwu_api::{Canwu, CommandRequest, CommandRequestId, EntityRef, Issuer, SimDuration};
use canwu_reference_world::{
    MovementCommand, ReferenceWorldPlugin, demo_scenario, order_movement,
    snapshot as reference_snapshot,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (scenario, ids) = demo_scenario()?;
    let plugin = ReferenceWorldPlugin;
    let mut canwu = Canwu::new_with_plugins(35, scenario, &[&plugin])?;

    let envelope = order_movement(
        Issuer::Actor(ids.commander),
        &MovementCommand {
            subject: EntityRef::Army(ids.army),
            destination: ids.eastern_territory,
            cargo: Vec::new(),
        },
    )?
    .at_time(canwu.time());
    canwu.enqueue_command(
        canwu.time(),
        0,
        CommandRequest::new(CommandRequestId::new(1), canwu.revision(), envelope),
    )?;
    canwu.advance_canonical(SimDuration::hours(19))?;

    let saved = canwu.snapshot_json()?;
    let loaded = Canwu::from_snapshot_json_with_plugins(&saved, &[&plugin])?;
    let fork = loaded.fork();
    let journal = canwu.replay_journal();
    let replayed = Canwu::replay_from_journal(&[&plugin], &journal)?;

    assert_eq!(loaded.checkpoint_hash(), canwu.checkpoint_hash());
    assert_eq!(fork.checkpoint_hash(), canwu.checkpoint_hash());
    assert_eq!(replayed.checkpoint_hash(), canwu.checkpoint_hash());
    println!(
        "army_location={} checkpoint={}",
        reference_snapshot(&replayed)?
            .army(ids.army)
            .expect("demo army exists")
            .location,
        replayed.checkpoint_hash()
    );
    Ok(())
}
