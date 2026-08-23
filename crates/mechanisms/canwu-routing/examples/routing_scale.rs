use canwu_routing::{
    DepartureSlot, PlanningSnapshot, RoutePlan, RoutingConnection, RoutingConnectionRef,
    RoutingEndpoint, RoutingEndpointKind, RoutingNetwork, RoutingNodeRef, RoutingPolicy,
    RoutingRequest, TransferMode, TraversalModel, plan_route,
};
use canwu_time::{SimDuration, SimTime};
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    const NODE_COUNT: usize = 512;
    const ITERATIONS: usize = 10;

    let endpoints = (0..NODE_COUNT)
        .map(|index| RoutingEndpoint {
            id: RoutingNodeRef::new(format!("node/{index}")),
            kind: RoutingEndpointKind::Settlement,
        })
        .collect::<Vec<_>>();
    let mut connections = Vec::with_capacity((NODE_COUNT - 1) * 2);
    for index in 0..(NODE_COUNT - 1) {
        connections.push(RoutingConnection {
            id: RoutingConnectionRef::new(format!("rail/{index}")),
            from: RoutingNodeRef::new(format!("node/{index}")),
            to: RoutingNodeRef::new(format!("node/{}", index + 1)),
            mode: TransferMode::Rail,
            traversal: TraversalModel::Departures {
                slots: vec![DepartureSlot {
                    departure_at: SimTime::EPOCH,
                    duration: SimDuration::minutes(1),
                }],
            },
            available_from: None,
            available_until: None,
            risk_per_mille: 0,
            resource_cost: 1,
        });
        if index + 2 < NODE_COUNT {
            connections.push(RoutingConnection {
                id: RoutingConnectionRef::new(format!("express/{index}")),
                from: RoutingNodeRef::new(format!("node/{index}")),
                to: RoutingNodeRef::new(format!("node/{}", index + 2)),
                mode: TransferMode::Rail,
                traversal: TraversalModel::Fixed {
                    duration: SimDuration::minutes(3),
                },
                available_from: None,
                available_until: None,
                risk_per_mille: 0,
                resource_cost: 2,
            });
        }
    }
    let network = RoutingNetwork::new("benchmark.rail.v1", endpoints, connections)?;
    let snapshot = PlanningSnapshot {
        observer: "benchmark".to_owned(),
        observed_at: SimTime::EPOCH,
        valid_until: None,
        knowledge_cut: "benchmark-cut".to_owned(),
        topology_version: "benchmark.rail.v1".to_owned(),
        timetable_version: Some("benchmark-timetable.v1".to_owned()),
        network,
    };
    let mut policy = RoutingPolicy::default();
    policy.allowed_modes.insert(TransferMode::Rail);
    policy.max_transfers = NODE_COUNT;
    policy.max_expanded_nodes = NODE_COUNT * 8;
    let request = RoutingRequest {
        origin: RoutingNodeRef::new("node/0"),
        destination: RoutingNodeRef::new(format!("node/{}", NODE_COUNT - 1)),
        departure_at: SimTime::EPOCH,
        policy,
    };

    let started = Instant::now();
    let mut last: Option<RoutePlan> = None;
    for _ in 0..ITERATIONS {
        last = Some(plan_route(&snapshot, &request)?);
    }
    let elapsed = started.elapsed();
    let plan = last.expect("benchmark must produce a plan");
    println!(
        "nodes={} connections={} iterations={} elapsed_ms={} route_legs={} arrival_minutes={}",
        NODE_COUNT,
        snapshot.network.connections.len(),
        ITERATIONS,
        elapsed.as_secs_f64() * 1_000.0,
        plan.legs.len(),
        plan.estimated_arrival_at.as_minutes(),
    );
    Ok(())
}
