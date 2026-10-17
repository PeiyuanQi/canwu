//! Routing scale evidence: a 512-node rail chain (the 0.5 baseline) and a
//! 1,296-node, 5,040-connection grid with a river crossed by ferries, planned
//! cold through `plan_route` and through a compiled `RoutingIndex`, with the
//! ferries stated as absolute departures and as a recurring pattern, and under
//! edge conditions.
//!
//! Run with `cargo run --release -p canwu-routing --example routing_scale`.

use canwu_routing::{
    DepartureSlot, EdgeConditionEffect, EdgeConditionOverlay, EdgeConditionRef, EdgeConditionSet,
    PlanningSnapshot, RoutePlan, RoutingCache, RoutingConnection, RoutingConnectionRef,
    RoutingEndpoint, RoutingEndpointKind, RoutingIndex, RoutingNetwork, RoutingNodeRef,
    RoutingPolicy, RoutingRequest, TransferMode, TraversalModel, plan_route,
};
use canwu_time::{SimDuration, SimTime};
use std::hint::black_box;
use std::time::{Duration, Instant};

const SIDE: usize = 36;
const RIVER_ROW: usize = 17;
const FERRY_PERIOD: i64 = 360;
const FERRY_OFFSET: i64 = 120;
const FERRY_DURATION: i64 = 45;
const HORIZON_DAYS: i64 = 60;
const ROUNDS: u32 = 5;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    chain()?;
    grid()?;
    Ok(())
}

fn per_call_ms(elapsed: Duration, calls: usize) -> f64 {
    elapsed.as_secs_f64() * 1_000.0 / f64::from(u32::try_from(calls).unwrap_or(u32::MAX))
}

fn chain() -> Result<(), Box<dyn std::error::Error>> {
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
    let policy = RoutingPolicy {
        allowed_modes: [TransferMode::Rail].into_iter().collect(),
        max_transfers: NODE_COUNT,
        max_expanded_nodes: NODE_COUNT * 8,
        ..RoutingPolicy::default()
    };
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
        "chain: nodes={} connections={} iterations={} elapsed_ms={:.3} route_legs={} arrival_minutes={}",
        NODE_COUNT,
        snapshot.network.connections.len(),
        ITERATIONS,
        elapsed.as_secs_f64() * 1_000.0,
        plan.legs.len(),
        plan.estimated_arrival_at.as_minutes(),
    );
    Ok(())
}

fn node(row: usize, column: usize) -> RoutingNodeRef {
    RoutingNodeRef::new(format!("n/{row:02}/{column:02}"))
}

fn road_minutes(seed: usize) -> i64 {
    let mixed = u64::try_from(seed)
        .unwrap_or(0)
        .wrapping_mul(6_364_136_223_846_793_005)
        .rotate_left(17);
    30 + i64::try_from(mixed % 90).unwrap_or(0)
}

fn ferry(recurring: bool) -> TraversalModel {
    if recurring {
        TraversalModel::Recurring {
            period: SimDuration::minutes(FERRY_PERIOD),
            offsets: vec![SimDuration::minutes(FERRY_OFFSET)],
            duration: SimDuration::minutes(FERRY_DURATION),
        }
    } else {
        TraversalModel::Departures {
            slots: (0..(HORIZON_DAYS * SimTime::MINUTES_PER_DAY / FERRY_PERIOD))
                .map(|cycle| DepartureSlot {
                    departure_at: SimTime::from_minutes(FERRY_OFFSET + cycle * FERRY_PERIOD),
                    duration: SimDuration::minutes(FERRY_DURATION),
                })
                .collect(),
        }
    }
}

fn grid_snapshot(recurring: bool) -> Result<PlanningSnapshot, Box<dyn std::error::Error>> {
    let mut endpoints = Vec::new();
    let mut connections = Vec::new();
    let mut seed = 0;
    for row in 0..SIDE {
        for column in 0..SIDE {
            endpoints.push(RoutingEndpoint {
                id: node(row, column),
                kind: RoutingEndpointKind::Settlement,
            });
            let mut neighbours = Vec::new();
            if column + 1 < SIDE {
                neighbours.push((row, column + 1));
            }
            if column > 0 {
                neighbours.push((row, column - 1));
            }
            if row + 1 < SIDE {
                neighbours.push((row + 1, column));
            }
            if row > 0 {
                neighbours.push((row - 1, column));
            }
            for (to_row, to_column) in neighbours {
                seed += 1;
                let crossing = row.min(to_row) == RIVER_ROW && row != to_row;
                connections.push(RoutingConnection {
                    id: RoutingConnectionRef::new(format!(
                        "e/{row:02}/{column:02}/{to_row:02}/{to_column:02}"
                    )),
                    from: node(row, column),
                    to: node(to_row, to_column),
                    mode: if crossing {
                        TransferMode::RiverBoat
                    } else {
                        TransferMode::Foot
                    },
                    traversal: if crossing {
                        ferry(recurring)
                    } else {
                        TraversalModel::Fixed {
                            duration: SimDuration::minutes(road_minutes(seed)),
                        }
                    },
                    available_from: None,
                    available_until: None,
                    risk_per_mille: u32::try_from(seed % 7)?,
                    resource_cost: u64::try_from(seed % 5)?,
                });
            }
        }
    }
    let network = RoutingNetwork::new("grid.v1", endpoints, connections)?;
    Ok(PlanningSnapshot {
        observer: "benchmark".to_owned(),
        observed_at: SimTime::EPOCH,
        valid_until: Some(SimTime::from_minutes(
            HORIZON_DAYS * SimTime::MINUTES_PER_DAY,
        )),
        knowledge_cut: "benchmark-cut".to_owned(),
        topology_version: "grid.v1".to_owned(),
        timetable_version: Some(
            if recurring {
                "ferries.recurring"
            } else {
                "ferries.slots"
            }
            .to_owned(),
        ),
        network,
    })
}

fn grid_requests() -> Vec<RoutingRequest> {
    let policy = RoutingPolicy {
        max_transfers: 1_024,
        max_expanded_nodes: 100_000,
        max_risk_per_mille: u64::MAX,
        ..RoutingPolicy::default()
    };
    [
        ((0, 0), (35, 35)),
        ((0, 35), (35, 0)),
        ((35, 35), (0, 0)),
        ((35, 0), (0, 35)),
        ((0, 17), (35, 17)),
        ((10, 3), (25, 30)),
        ((30, 5), (2, 20)),
        ((17, 0), (18, 35)),
    ]
    .into_iter()
    .map(
        |((from_row, from_column), (to_row, to_column))| RoutingRequest {
            origin: node(from_row, from_column),
            destination: node(to_row, to_column),
            departure_at: SimTime::EPOCH,
            policy: policy.clone(),
        },
    )
    .collect()
}

fn time_plans(
    requests: &[RoutingRequest],
    mut plan: impl FnMut(&RoutingRequest) -> Result<RoutePlan, canwu_routing::RoutingError>,
) -> Result<(f64, Vec<RoutePlan>), Box<dyn std::error::Error>> {
    let mut plans = Vec::new();
    let started = Instant::now();
    for round in 0..ROUNDS {
        for request in requests {
            let result = black_box(plan(request)?);
            if round == 0 {
                plans.push(result);
            }
        }
    }
    let calls = requests.len() * usize::try_from(ROUNDS)?;
    Ok((per_call_ms(started.elapsed(), calls), plans))
}

fn time_once<T>(mut work: impl FnMut() -> T) -> (f64, T) {
    let started = Instant::now();
    let mut result = work();
    for _ in 1..ROUNDS {
        result = black_box(work());
    }
    (
        per_call_ms(started.elapsed(), usize::try_from(ROUNDS).unwrap_or(1)),
        result,
    )
}

fn grid() -> Result<(), Box<dyn std::error::Error>> {
    let requests = grid_requests();
    let slots = grid_snapshot(false)?;
    let recurring = grid_snapshot(true)?;
    for (label, snapshot) in [("slots", &slots), ("recurring", &recurring)] {
        let bytes = serde_json::to_vec(snapshot)?.len();
        let (digest_ms, _) = time_once(|| snapshot.digest());
        let (key_ms, _) = time_once(|| RoutingCache::key(snapshot, &requests[0]));
        let (cold_ms, cold) = time_plans(&requests, |request| plan_route(snapshot, request))?;
        let (compile_ms, index) = time_once(|| RoutingIndex::new(snapshot.clone()));
        let index = index?;
        let (indexed_ms, indexed) = time_plans(&requests, |request| index.plan_route(request))?;
        let (index_key_ms, _) = time_once(|| RoutingCache::index_key(&index, None, &requests[0]));
        let (reobserve_ms, _) = time_once(|| {
            index.reobserve(
                "grid.v1",
                index.timetable_version(),
                "benchmark",
                SimTime::from_minutes(1),
                None,
                "cut-2",
            )
        });
        assert_eq!(cold, indexed, "an index must plan exactly as plan_route");
        println!(
            "grid/{label}: nodes={} connections={} snapshot_json_bytes={bytes} digest_ms={digest_ms:.3} \
             cache_key_ms={key_ms:.3} cold_plan_ms={cold_ms:.3} compile_ms={compile_ms:.3} \
             indexed_plan_ms={indexed_ms:.3} index_key_ms={index_key_ms:.4} reobserve_ms={reobserve_ms:.4}",
            snapshot.network.endpoints.len(),
            snapshot.network.connections.len(),
        );
        for plan in &indexed {
            println!(
                "  legs={} arrival_minutes={}",
                plan.legs.len(),
                plan.estimated_arrival_at.as_minutes()
            );
        }
    }

    let index = RoutingIndex::new(recurring)?;
    let mut overlays = Vec::new();
    for (position, connection) in index.network().connections.iter().enumerate() {
        let effect = if connection.mode == TransferMode::RiverBoat {
            Some((EdgeConditionEffect::Closed, 2))
        } else if position % 10 == 0 {
            Some((
                EdgeConditionEffect::Adjusted {
                    duration_per_mille: 1_500,
                    resource_cost_per_mille: 2_000,
                    risk_per_mille: Some(100),
                },
                10,
            ))
        } else {
            None
        };
        if let Some((effect, days)) = effect {
            overlays.push(EdgeConditionOverlay {
                id: EdgeConditionRef::new(format!("condition/{position}")),
                connection: connection.id.clone(),
                window_from: SimTime::EPOCH,
                window_until: SimTime::from_minutes(days * SimTime::MINUTES_PER_DAY),
                effect,
            });
        }
    }
    let count = overlays.len();
    let (build_ms, conditions) = time_once(|| EdgeConditionSet::new(overlays.clone()));
    let conditions = conditions?;
    let (conditioned_ms, conditioned) = time_plans(&requests, |request| {
        index.plan_route_with_conditions(request, &conditions)
    })?;
    println!(
        "grid/recurring+conditions: overlays={count} build_ms={build_ms:.3} conditioned_plan_ms={conditioned_ms:.3}"
    );
    for plan in &conditioned {
        println!(
            "  legs={} arrival_minutes={}",
            plan.legs.len(),
            plan.estimated_arrival_at.as_minutes()
        );
    }
    Ok(())
}
