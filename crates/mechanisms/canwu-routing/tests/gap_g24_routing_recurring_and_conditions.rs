//! Downstream gap G-24: recurring departures stated as one period, sorted
//! offsets, and one duration, and time-windowed edge conditions applied by
//! departure instant without rebuilding the network.

use canwu_routing::{
    DepartureSlot, EdgeConditionEffect, EdgeConditionOverlay, EdgeConditionRef, EdgeConditionSet,
    MAX_RECURRING_OFFSETS, PlanningSnapshot, RoutingConnection, RoutingConnectionRef,
    RoutingEndpoint, RoutingEndpointKind, RoutingError, RoutingIndex, RoutingNetwork,
    RoutingNodeRef, RoutingPolicy, RoutingRequest, TransferMode, TraversalModel, plan_route,
};
use canwu_time::{SimDuration, SimTime};

fn minutes(value: i64) -> SimTime {
    SimTime::from_minutes(value)
}

fn recurring(period: i64, offsets: &[i64], duration: i64) -> TraversalModel {
    TraversalModel::Recurring {
        period: SimDuration::minutes(period),
        offsets: offsets.iter().copied().map(SimDuration::minutes).collect(),
        duration: SimDuration::minutes(duration),
    }
}

fn connection(id: &str, from: &str, to: &str, traversal: TraversalModel) -> RoutingConnection {
    RoutingConnection {
        id: RoutingConnectionRef::new(id),
        from: RoutingNodeRef::new(from),
        to: RoutingNodeRef::new(to),
        mode: TransferMode::RiverBoat,
        traversal,
        available_from: None,
        available_until: None,
        risk_per_mille: 10,
        resource_cost: 100,
    }
}

fn network(connections: Vec<RoutingConnection>) -> Result<RoutingNetwork, RoutingError> {
    RoutingNetwork::new(
        "river.v1",
        ["a", "b", "c"]
            .into_iter()
            .map(|id| RoutingEndpoint {
                id: RoutingNodeRef::new(id),
                kind: RoutingEndpointKind::Port,
            })
            .collect(),
        connections,
    )
}

fn snapshot(connections: Vec<RoutingConnection>) -> PlanningSnapshot {
    PlanningSnapshot {
        observer: "shipper".to_owned(),
        observed_at: SimTime::EPOCH,
        valid_until: None,
        knowledge_cut: "cut:1".to_owned(),
        topology_version: "river.v1".to_owned(),
        timetable_version: Some("ferries.v1".to_owned()),
        network: network(connections).unwrap(),
    }
}

fn request(origin: &str, destination: &str, at: i64) -> RoutingRequest {
    RoutingRequest {
        origin: RoutingNodeRef::new(origin),
        destination: RoutingNodeRef::new(destination),
        departure_at: minutes(at),
        policy: RoutingPolicy::default(),
    }
}

fn overlay(
    id: &str,
    connection: &str,
    from: i64,
    until: i64,
    effect: EdgeConditionEffect,
) -> EdgeConditionOverlay {
    EdgeConditionOverlay {
        id: EdgeConditionRef::new(id),
        connection: RoutingConnectionRef::new(connection),
        window_from: minutes(from),
        window_until: minutes(until),
        effect,
    }
}

fn adjusted(
    duration_per_mille: u32,
    resource_cost_per_mille: u32,
    risk_per_mille: Option<u32>,
) -> EdgeConditionEffect {
    EdgeConditionEffect::Adjusted {
        duration_per_mille,
        resource_cost_per_mille,
        risk_per_mille,
    }
}

/// A recurring pattern departs exactly when the equivalent list of absolute
/// slots does, for every query minute, before and after the epoch, at an
/// offset, between offsets, and after the last offset of a cycle; and a route
/// through it waits for the next cycle.
#[test]
fn gap_g24_recurring_departures_match_absolute_slots() {
    let (period, offsets, duration) = (360, [0, 45, 300], 30);
    let pattern = recurring(period, &offsets, duration);
    let slots = TraversalModel::Departures {
        slots: (-3..=4)
            .flat_map(|cycle| {
                offsets.iter().map(move |offset| DepartureSlot {
                    departure_at: minutes(cycle * period + offset),
                    duration: SimDuration::minutes(duration),
                })
            })
            .collect(),
    };
    for at in -2 * period..=3 * period {
        assert_eq!(
            pattern.next_departure(minutes(at)),
            slots.next_departure(minutes(at)),
            "query at minute {at}"
        );
    }

    // Arriving at minute 301, one minute after the last departure of the
    // cycle: the next ferry is the first of the following cycle.
    let mut road = connection(
        "a-b",
        "a",
        "b",
        TraversalModel::Fixed {
            duration: SimDuration::minutes(301),
        },
    );
    road.mode = TransferMode::Foot;
    let ferry_snapshot = snapshot(vec![road, connection("b-c", "b", "c", pattern)]);
    let plan = plan_route(&ferry_snapshot, &request("a", "c", 0)).unwrap();
    assert_eq!(plan.legs[1].planned_departure_at, minutes(360));
    assert_eq!(plan.estimated_arrival_at, minutes(390));

    // The season bounds the pattern: nothing departs before `available_from`
    // and nothing arrives after `available_until`.
    let mut seasonal = connection("a-c", "a", "c", recurring(1_440, &[480], 600));
    seasonal.available_from = Some(minutes(2 * 1_440));
    seasonal.available_until = Some(minutes(3 * 1_440));
    let snapshot_seasonal = snapshot(vec![seasonal]);
    let plan = plan_route(&snapshot_seasonal, &request("a", "c", 0)).unwrap();
    assert_eq!(plan.legs[0].planned_departure_at, minutes(2 * 1_440 + 480));
    assert_eq!(
        plan_route(&snapshot_seasonal, &request("a", "c", 2 * 1_440 + 481)),
        Err(RoutingError::NoKnownRoute)
    );
}

/// Every malformed pattern is refused when the network is built, and a
/// network assembled without `RoutingNetwork::new` is refused when it is
/// planned or compiled rather than dividing by a zero period.
#[test]
fn gap_g24_recurring_patterns_are_validated() {
    let too_many = (0..=i64::try_from(MAX_RECURRING_OFFSETS).unwrap()).collect::<Vec<_>>();
    let malformed = [
        recurring(0, &[0], 10),
        recurring(-60, &[0], 10),
        recurring(60, &[0], 0),
        recurring(60, &[], 10),
        recurring(60, &[60], 10),
        recurring(60, &[-1], 10),
        recurring(60, &[30, 10], 10),
        recurring(60, &[10, 10], 10),
        recurring(
            i64::try_from(MAX_RECURRING_OFFSETS).unwrap() + 10,
            &too_many,
            10,
        ),
    ];
    for traversal in malformed {
        let built = network(vec![connection("a-b", "a", "b", traversal.clone())]);
        assert!(
            matches!(built, Err(RoutingError::InvalidNetwork(_))),
            "{traversal:?} must be refused"
        );
        let mut literal = snapshot(Vec::new());
        literal
            .network
            .connections
            .push(connection("a-b", "a", "b", traversal));
        assert!(matches!(
            plan_route(&literal, &request("a", "b", 0)),
            Err(RoutingError::InvalidNetwork(_))
        ));
        assert!(matches!(
            RoutingIndex::new(literal),
            Err(RoutingError::InvalidNetwork(_))
        ));
    }
    let at_limit = (0..i64::try_from(MAX_RECURRING_OFFSETS).unwrap()).collect::<Vec<_>>();
    assert!(
        network(vec![connection(
            "a-b",
            "a",
            "b",
            recurring(i64::try_from(MAX_RECURRING_OFFSETS).unwrap(), &at_limit, 1),
        )])
        .is_ok()
    );
}

/// A closure makes a traveller wait for the first departure after it; an
/// adjustment scales the duration and resource cost (rounded up) and replaces
/// the risk of departures inside its window only; the plan records the set it
/// was made under; an overlay for a connection the network lacks does nothing.
#[test]
fn gap_g24_edge_conditions_apply_by_departure_instant() {
    let mut road = connection(
        "a-b",
        "a",
        "b",
        TraversalModel::Fixed {
            duration: SimDuration::minutes(61),
        },
    );
    road.mode = TransferMode::Foot;
    let ferry = connection("b-c", "b", "c", recurring(60, &[0], 10));
    let index = RoutingIndex::new(snapshot(vec![road, ferry])).unwrap();
    let conditions = EdgeConditionSet::new(vec![
        overlay("flood", "a-b", 0, 120, EdgeConditionEffect::Closed),
        overlay("mud", "a-b", 120, 240, adjusted(1_500, 2_501, Some(300))),
        overlay("ice-1", "b-c", 180, 300, EdgeConditionEffect::Closed),
        overlay("ice-2", "b-c", 200, 240, EdgeConditionEffect::Closed),
        overlay("elsewhere", "x-y", 0, 10_000, EdgeConditionEffect::Closed),
    ])
    .unwrap();

    // Waits out the flood, departs at 120 in the mud: 61 * 1.5 = 91.5, so 92
    // minutes; 100 * 2.501 = 250.1, so 251; risk replaced by 300. Arriving at
    // 212, the 240 ferry is iced over (a closure holding a shorter one) until 300.
    let plan = index
        .plan_route_with_conditions(&request("a", "c", 30), &conditions)
        .unwrap();
    assert_eq!(
        plan.legs
            .iter()
            .map(|leg| (leg.planned_departure_at, leg.planned_arrival_at))
            .collect::<Vec<_>>(),
        vec![(minutes(120), minutes(212)), (minutes(300), minutes(310))]
    );
    assert_eq!(plan.cost.resource_cost, 251 + 100);
    assert_eq!(plan.cost.risk_per_mille, 300 + 10);
    assert_eq!(
        plan.edge_conditions_digest.as_deref(),
        Some(conditions.digest())
    );
    // The overlay for a connection this network lacks had no effect and is
    // the one the index reports.
    assert_eq!(
        index
            .unmatched_conditions(&conditions)
            .iter()
            .map(|overlay| overlay.id.as_str())
            .collect::<Vec<_>>(),
        ["elsewhere"]
    );

    // A departure at the end of the adjustment window is unaffected.
    let after = index
        .plan_route_with_conditions(&request("a", "b", 240), &conditions)
        .unwrap();
    assert_eq!(after.estimated_arrival_at, minutes(301));
    assert_eq!(after.cost.risk_per_mille, 10);

    // No conditions, or an empty set, plans as the bare network does.
    let bare = index.plan_route(&request("a", "c", 30)).unwrap();
    assert_eq!(bare.estimated_arrival_at, minutes(130));
    assert_eq!(bare.edge_conditions_digest, None);
    assert_eq!(
        index
            .plan_route_with_conditions(&request("a", "c", 30), &EdgeConditionSet::default())
            .unwrap(),
        bare
    );
}

/// Malformed or ambiguous condition sets are refused when they are built.
#[test]
fn gap_g24_edge_condition_sets_are_validated() {
    let refused = [
        vec![
            overlay("one", "a-b", 0, 100, adjusted(2_000, 1_000, None)),
            overlay("two", "a-b", 99, 200, adjusted(500, 1_000, None)),
        ],
        vec![
            overlay("same", "a-b", 0, 10, EdgeConditionEffect::Closed),
            overlay("same", "b-c", 0, 10, EdgeConditionEffect::Closed),
        ],
        vec![overlay("empty", "a-b", 10, 10, EdgeConditionEffect::Closed)],
        vec![overlay("zero", "a-b", 0, 10, adjusted(0, 1_000, None))],
        vec![overlay(
            "risk",
            "a-b",
            0,
            10,
            adjusted(1_000, 1_000, Some(1_001)),
        )],
    ];
    for overlays in refused {
        assert!(
            matches!(
                EdgeConditionSet::new(overlays.clone()),
                Err(RoutingError::InvalidConditions(_))
            ),
            "{overlays:?} must be refused"
        );
    }
    // Closures may overlap each other and adjustments; adjacent adjustments
    // do not overlap.
    assert!(
        EdgeConditionSet::new(vec![
            overlay("one", "a-b", 0, 100, adjusted(2_000, 1_000, None)),
            overlay("two", "a-b", 100, 200, adjusted(500, 0, Some(0))),
            overlay("shut-1", "a-b", 50, 150, EdgeConditionEffect::Closed),
            overlay("shut-2", "a-b", 60, 70, EdgeConditionEffect::Closed),
        ])
        .is_ok()
    );
}
