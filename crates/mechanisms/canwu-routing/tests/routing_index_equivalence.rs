//! A compiled `RoutingIndex` plans exactly as `plan_route` on the same
//! snapshot, digests included, and keeps the route order of `Vec<RouteLeg>`
//! when routes tie on arrival, risk, resource cost, and transfers.

use canwu_routing::{
    PlanningSnapshot, RoutingAlgorithm, RoutingCache, RoutingConnection, RoutingConnectionRef,
    RoutingEndpoint, RoutingEndpointKind, RoutingError, RoutingIndex, RoutingNetwork,
    RoutingNodeRef, RoutingPolicy, RoutingRequest, TransferMode, TraversalModel, plan_route,
};
use canwu_time::{SimDuration, SimTime};

fn road(id: &str, from: &str, to: &str) -> RoutingConnection {
    RoutingConnection {
        id: RoutingConnectionRef::new(id),
        from: RoutingNodeRef::new(from),
        to: RoutingNodeRef::new(to),
        mode: TransferMode::Horse,
        traversal: TraversalModel::Fixed {
            duration: SimDuration::minutes(10),
        },
        available_from: None,
        available_until: None,
        risk_per_mille: 1,
        resource_cost: 1,
    }
}

/// Two three-leg routes from `a` to `d` that tie on every scalar. Route P
/// (`c1`, `c5`, `c6`) through `z1`, `z2` is first leg by leg from the origin;
/// route Q (`c2`, `c3`, `c4`) through `b1`, `b2` has the smaller last and
/// middle legs and its nodes sort first, so it is settled first and P must
/// displace it.
fn tied_snapshot() -> PlanningSnapshot {
    let network = RoutingNetwork::new(
        "tie.v1",
        ["a", "b1", "b2", "d", "z1", "z2"]
            .into_iter()
            .map(|id| RoutingEndpoint {
                id: RoutingNodeRef::new(id),
                kind: RoutingEndpointKind::RelayStation,
            })
            .collect(),
        vec![
            road("c1", "a", "z1"),
            road("c5", "z1", "z2"),
            road("c6", "z2", "d"),
            road("c2", "a", "b1"),
            road("c3", "b1", "b2"),
            road("c4", "b2", "d"),
        ],
    )
    .unwrap();
    PlanningSnapshot {
        observer: "courier".to_owned(),
        observed_at: SimTime::EPOCH,
        valid_until: Some(SimTime::from_minutes(1_000)),
        knowledge_cut: "cut:1".to_owned(),
        topology_version: "tie.v1".to_owned(),
        timetable_version: None,
        network,
    }
}

#[test]
fn routing_index_plans_as_plan_route_and_keeps_leg_order_on_ties() {
    let snapshot = tied_snapshot();
    let index = RoutingIndex::new(snapshot.clone()).unwrap();
    assert_eq!(index.digest(), snapshot.digest());
    for algorithm in [
        RoutingAlgorithm::FifoDijkstraV1,
        RoutingAlgorithm::BoundedLabelCorrectingV1,
    ] {
        let request = RoutingRequest {
            origin: RoutingNodeRef::new("a"),
            destination: RoutingNodeRef::new("d"),
            departure_at: SimTime::EPOCH,
            policy: RoutingPolicy {
                algorithm,
                ..RoutingPolicy::default()
            },
        };
        let cold = plan_route(&snapshot, &request).unwrap();
        assert_eq!(
            cold.legs
                .iter()
                .map(|leg| leg.connection.as_str())
                .collect::<Vec<_>>(),
            ["c1", "c5", "c6"],
            "{algorithm:?} must keep the route first leg by leg"
        );
        assert_eq!(index.plan_route(&request).unwrap(), cold);
        assert_eq!(
            RoutingCache::index_key(&index, None, &request),
            RoutingCache::key(&snapshot, &request)
        );

        // A new read cut of the same network: the plan records the new cut's
        // snapshot digest and is otherwise unchanged. Another network revision
        // is refused.
        for (topology, timetable) in [("tie.v2", None), ("tie.v1", Some("slots.v1"))] {
            assert!(matches!(
                index.reobserve(
                    topology,
                    timetable,
                    "courier",
                    SimTime::from_minutes(5),
                    None,
                    "cut:2"
                ),
                Err(RoutingError::InvalidSnapshot(_))
            ));
        }
        let later = index
            .reobserve(
                "tie.v1",
                None,
                "courier",
                SimTime::from_minutes(5),
                None,
                "cut:2",
            )
            .unwrap();
        let mut later_snapshot = snapshot.clone();
        later_snapshot.observed_at = SimTime::from_minutes(5);
        later_snapshot.valid_until = None;
        later_snapshot.knowledge_cut = "cut:2".to_owned();
        assert_eq!(later.digest(), later_snapshot.digest());
        assert_eq!(later.to_snapshot(), later_snapshot);
        let replanned = later.plan_route(&request).unwrap();
        assert_eq!(replanned, plan_route(&later_snapshot, &request).unwrap());
        assert_eq!(replanned.legs, cold.legs);
        assert_ne!(
            replanned.planning_snapshot_digest,
            cold.planning_snapshot_digest
        );
    }
}
