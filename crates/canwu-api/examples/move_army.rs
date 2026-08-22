use canwu_api::{Canwu, EntityRef, ObserveRequest, SemanticAction, SimDuration};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut canwu = Canwu::demo(35)?;
    let ids = Canwu::demo_ids();

    canwu.act(
        ids.commander,
        SemanticAction::MoveEntity {
            subject: EntityRef::Army(ids.army),
            destination: ids.eastern_territory,
            cargo: Vec::new(),
        },
    )?;
    let arrival_events = canwu.advance(SimDuration::days(1))?;

    let commander = canwu.observe(ids.commander, &ObserveRequest::default())?;
    let observer = canwu.observe(ids.observer, &ObserveRequest::default())?;
    println!("time: {}", canwu.time());
    println!("arrival events: {}", arrival_events.len());
    println!(
        "commander knows location: {:?}",
        commander.known_armies[0].known_location
    );
    println!(
        "observer still believes location: {:?}",
        observer.known_armies[0].known_location
    );

    canwu.advance(SimDuration::days(3))?;
    let observer = canwu.observe(ids.observer, &ObserveRequest::default())?;
    println!(
        "observer after report: {:?}",
        observer.known_armies[0].known_location
    );
    Ok(())
}
