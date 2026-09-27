//! Public-API fixtures for gaps G-21 to G-23: persons-group movement subjects,
//! seizure handoffs, and external-condition itinerary revisions.

use canwu_core::{ArmyId, DomainRecordRef, EntityRef, PersonId};
use canwu_routing::{
    ROUTING_ALGORITHM_VERSION, RouteCost, RouteLeg, RoutePlan, RoutingConnectionRef,
    RoutingNodeRef, TransferMode,
};
use canwu_time::SimTime;
use canwu_transport::{
    Handoff, HandoffId, HandoffKind, ItineraryRevision, ItineraryRevisionId,
    ItineraryRevisionReason, LegExecutionId, LegExecutionStatus, MovementInitiative, MovementOrder,
    MovementOrderError, MovementOrderId, MovementSubject, MovementSubjectRole, TransportError,
    TransportExecution, TransportExecutionId, TransportExecutionState,
};
use serde_json::json;

/// A plan through `stops`, ten minutes per leg from the epoch.
fn plan(stops: &[&str]) -> RoutePlan {
    let legs = stops
        .windows(2)
        .zip(0_i64..)
        .map(|(pair, index)| RouteLeg {
            connection: RoutingConnectionRef::new(format!("{}-{}", pair[0], pair[1])),
            from: RoutingNodeRef::new(pair[0]),
            to: RoutingNodeRef::new(pair[1]),
            mode: TransferMode::Foot,
            planned_departure_at: SimTime::from_minutes(index * 10),
            planned_arrival_at: SimTime::from_minutes(index * 10 + 10),
        })
        .collect::<Vec<_>>();
    let arrival = legs.last().unwrap().planned_arrival_at;
    RoutePlan {
        algorithm_version: ROUTING_ALGORITHM_VERSION.to_owned(),
        policy_version: "fixture.policy.v1".to_owned(),
        planning_snapshot_digest: "fixture-snapshot".to_owned(),
        origin: RoutingNodeRef::new(stops[0]),
        destination: RoutingNodeRef::new(*stops.last().unwrap()),
        departure_at: SimTime::EPOCH,
        estimated_arrival_at: arrival,
        cost: RouteCost {
            estimated_arrival_at: arrival,
            risk_per_mille: 0,
            resource_cost: 0,
            transfers: 0,
        },
        legs,
        digest: stops.join(">"),
    }
}

fn revision(
    id: u64,
    predecessor: Option<u64>,
    route: RoutePlan,
    at: SimTime,
    reason: ItineraryRevisionReason,
) -> ItineraryRevision {
    ItineraryRevision {
        id: ItineraryRevisionId(id),
        predecessor: predecessor.map(ItineraryRevisionId),
        plan: route,
        planned_at: at,
        valid_from: at,
        reason,
        superseded_at: None,
        evidence: Vec::new(),
    }
}

/// An execution whose first leg departed and then failed at minute five.
fn execution_with_failed_first_leg(stops: &[&str]) -> TransportExecution {
    let mut execution = TransportExecution::new(TransportExecutionId(1), None);
    execution
        .install_initial_itinerary(revision(
            1,
            None,
            plan(stops),
            SimTime::EPOCH,
            ItineraryRevisionReason::Initial,
        ))
        .unwrap();
    execution.start_current_leg(SimTime::EPOCH).unwrap();
    execution
        .fail_current_leg("interrupted".to_owned(), SimTime::from_minutes(5))
        .unwrap();
    execution
}

fn round_trip<T: serde::Serialize + serde::de::DeserializeOwned>(value: &T) -> T {
    serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
}

#[test]
fn gap_g21_transport_persons_group() {
    let group = EntityRef::Domain(DomainRecordRef::new("fixture.society", "cohort", "levy-3"));
    let mut order = MovementOrder {
        id: MovementOrderId(1),
        subjects: vec![
            MovementSubject {
                entity: group.clone(),
                role: MovementSubjectRole::PersonsGroup,
                quantity: Some(240),
                expected_custody: None,
            },
            MovementSubject {
                entity: EntityRef::Person(PersonId::new(7)),
                role: MovementSubjectRole::MovablePrincipal,
                quantity: None,
                expected_custody: None,
            },
        ],
        origin: RoutingNodeRef::new("a"),
        destination: RoutingNodeRef::new("b"),
        plan: plan(&["a", "b"]),
        initiative: MovementInitiative::Commanded,
        ordered_at: SimTime::EPOCH,
        expected_position_revision: 1,
    };
    order.validate().unwrap();

    // The head count is part of the persisted manifest and round-trips exactly.
    let encoded = serde_json::to_value(&order.subjects[0]).unwrap();
    assert_eq!(encoded["role"], json!("persons_group"));
    assert_eq!(encoded["quantity"], json!(240));
    assert_eq!(round_trip(&order), order);

    // Like cargo, a persons group needs a positive quantity, and a single
    // identity role still cannot carry one.
    for (index, role, quantity) in [
        (0, MovementSubjectRole::PersonsGroup, None),
        (0, MovementSubjectRole::PersonsGroup, Some(0)),
        (1, MovementSubjectRole::MovablePrincipal, Some(240)),
    ] {
        let mut invalid = order.clone();
        invalid.subjects[index].role = role;
        invalid.subjects[index].quantity = quantity;
        assert!(matches!(
            invalid.validate(),
            Err(MovementOrderError::Invalid(_))
        ));
    }
    order.subjects[0].role = MovementSubjectRole::Cargo;
    order.validate().unwrap();
}

#[test]
fn gap_g22_transport_seizure_handoff() {
    let mut execution = execution_with_failed_first_leg(&["a", "b", "c"]);
    let seizure = Handoff {
        id: HandoffId(1),
        from_leg: LegExecutionId(1),
        to_leg: LegExecutionId(2),
        from_custodian: "carrier/escort".to_owned(),
        to_custodian: "army:3".to_owned(),
        at: SimTime::from_minutes(5),
        location: "a-b".to_owned(),
        evidence: Vec::new(),
        kind: HandoffKind::Seizure {
            by: EntityRef::Army(ArmyId::new(3)),
        },
    };

    // A seizure obeys the same leg and time rules as a planned handoff.
    let mut early = seizure.clone();
    early.at = SimTime::from_minutes(4);
    assert!(matches!(
        execution.record_handoff(early),
        Err(TransportError::InvalidHandoff(_))
    ));
    let mut anonymous = seizure.clone();
    anonymous.kind = HandoffKind::Seizure {
        by: EntityRef::Domain(DomainRecordRef::new("fixture.world", "band", " ")),
    };
    assert!(matches!(
        execution.record_handoff(anonymous),
        Err(TransportError::InvalidHandoff(_))
    ));
    assert!(execution.handoffs.is_empty());
    execution.record_handoff(seizure.clone()).unwrap();

    let encoded = serde_json::to_value(&execution.handoffs[0]).unwrap();
    assert_eq!(
        encoded["kind"],
        json!({"seizure": {"by": {"type": "army", "id": 3}}})
    );
    let restored = round_trip(&execution);
    assert_eq!(restored, execution);
    assert_eq!(restored.handoffs[0].kind, seizure.kind);

    // A planned handoff keeps the pre-seizure JSON shape, and handoff JSON
    // written before the field existed loads as planned.
    let mut planned = seizure;
    planned.kind = HandoffKind::Planned;
    let encoded = serde_json::to_value(&planned).unwrap();
    assert!(encoded.get("kind").is_none());
    let legacy = json!({
        "id": 9,
        "from_leg": 1,
        "to_leg": 2,
        "from_custodian": "carrier/escort",
        "to_custodian": "carrier/relay",
        "at": SimTime::from_minutes(5),
        "location": "b",
        "evidence": []
    });
    let loaded: Handoff = serde_json::from_value(legacy).unwrap();
    assert_eq!(loaded.kind, HandoffKind::Planned);
    assert_eq!(loaded.id, HandoffId(9));
}

#[test]
fn gap_g23_transport_external_condition() {
    let mut execution = execution_with_failed_first_leg(&["a", "b"]);
    assert_eq!(execution.state, TransportExecutionState::ReplanPending);
    let hazard = DomainRecordRef::new("fixture.world", "hazard", "flood-7");
    let reroute_at = SimTime::from_minutes(6);
    let cited = ItineraryRevisionReason::ExternalCondition {
        record: hazard.clone(),
        version: 3,
        kind: "flood".to_owned(),
    };

    // The citation must name a well-formed record, a positive version, and a kind.
    for invalid in [
        ItineraryRevisionReason::ExternalCondition {
            record: hazard.clone(),
            version: 0,
            kind: "flood".to_owned(),
        },
        ItineraryRevisionReason::ExternalCondition {
            record: hazard.clone(),
            version: 3,
            kind: " ".to_owned(),
        },
        ItineraryRevisionReason::ExternalCondition {
            record: DomainRecordRef::new("fixture.world", "hazard", ""),
            version: 3,
            kind: "flood".to_owned(),
        },
    ] {
        let rejected = execution.reroute(
            revision(
                2,
                Some(1),
                plan(&["a", "c", "b"]),
                reroute_at,
                invalid.clone(),
            ),
            reroute_at,
        );
        assert!(matches!(rejected, Err(TransportError::InvalidRevision(_))));
        let mut fresh = TransportExecution::new(TransportExecutionId(2), None);
        let rejected = fresh.install_initial_itinerary(revision(
            1,
            None,
            plan(&["a", "b"]),
            SimTime::EPOCH,
            invalid,
        ));
        assert!(matches!(rejected, Err(TransportError::InvalidRevision(_))));
        assert!(fresh.revisions.is_empty() && fresh.legs.is_empty());
    }
    assert_eq!(execution.revisions.len(), 1);
    assert_eq!(execution.revisions[0].superseded_at, None);

    execution
        .reroute(
            revision(
                2,
                Some(1),
                plan(&["a", "c", "b"]),
                reroute_at,
                cited.clone(),
            ),
            reroute_at,
        )
        .unwrap();
    assert_eq!(
        execution.active_itinerary_revision,
        Some(ItineraryRevisionId(2))
    );
    assert_eq!(execution.revisions[0].superseded_at, Some(reroute_at));
    assert_eq!(execution.revisions[1].reason, cited);
    assert_eq!(execution.legs[0].status, LegExecutionStatus::Failed);

    let encoded = serde_json::to_value(&execution.revisions[1]).unwrap();
    assert_eq!(
        encoded["reason"],
        json!({"external_condition": {
            "record": {"kind": {"namespace": "fixture.world", "name": "hazard"}, "id": "flood-7"},
            "version": 3,
            "kind": "flood"
        }})
    );
    assert_eq!(round_trip(&execution), execution);
    // Existing reasons keep their persisted shape.
    let legacy: ItineraryRevisionReason =
        serde_json::from_value(json!({"disaster": {"explanation": "bridge closed"}})).unwrap();
    assert_eq!(
        legacy,
        ItineraryRevisionReason::Disaster {
            explanation: "bridge closed".to_owned()
        }
    );
}
