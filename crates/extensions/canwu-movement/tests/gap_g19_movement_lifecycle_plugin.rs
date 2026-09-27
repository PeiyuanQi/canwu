//! Public-API fixture for gap G-19: the `canwu-movement` lifecycle plugin.
//!
//! Orders settle their legs at due time through the lifecycle writer; bookings
//! requested in one boundary compete by priority, a booked leg consumes pool
//! capacity, and a leg whose booking failed fails deterministically. An
//! application-reported failure is followed by a reroute citing an external
//! condition and a seizure handoff; a delivery movement reconciles through
//! its delivery record. Authority is checked for subjects, seizures, and
//! reconciliation; operation keys are idempotent per source; reports are
//! holder-relative and published only when they change; closed executions
//! retire once their final reports are out; and the run survives save/load
//! mid-flight and exact replay.

#![allow(clippy::too_many_lines)]

use canwu_api::{
    ArmyId, Canwu, CanwuError, CapacityBooking, CapacityBookingId, CapacityBookingStatus,
    CommandAttemptOutcome, CommandEnvelope, CommandRequest, CommandRequestId, DomainRecord,
    DomainRecordClass, DomainRecordKind, DomainRecordLifecycle, DomainRecordRef,
    DomainRecordSchema, DomainRecordType, DomainRecordVersionRef, DomainRecordVersionSource,
    DomainReference, DomainReferenceSchema, DomainReferenceTarget, DomainReferenceTargetKind,
    DomainValueKindClass, EntityRef, ErrorCode, Handoff, HandoffId, HandoffKind, Issuer,
    ItineraryRevision, ItineraryRevisionId, ItineraryRevisionReason, KnowledgeHistoryView,
    KnowledgeHolderRef, KnowledgeQuery, LegExecutionId, LegExecutionStatus,
    MAX_KNOWLEDGE_PAGE_SIZE, MovementInitiative, MovementOrder, MovementOrderId, MovementSubject,
    MovementSubjectRole, PayloadSchema, PersonId, PluginRegistrar, ROUTING_ALGORITHM_VERSION,
    ReconciliationOutcome, RouteCost, RouteLeg, RoutePlan, RoutingConnectionRef, RoutingNodeRef,
    Scenario, SimDuration, SimTime, SimulationPlugin, TransferMode, TransportExecutionId,
    TransportExecutionState,
};
use canwu_movement::{
    MOVEMENT_GRANTEE_ROLE, MOVEMENT_SUBJECT_ROLE, MovementBookingRequestV1, MovementCommandV1,
    MovementErrorCode, MovementIncidentV1, MovementObserverRole, MovementOperation,
    MovementOperationDisposition, MovementOperationOutcomeV1, MovementOperationScopeV1,
    MovementOrderRequestV1, MovementPhaseV1, MovementPlugin, MovementPoolOfferV1,
    MovementRemoteObserverV1, MovementReportV1, MovementState, movement_command,
    movement_incident_ingress, movement_report_knowledge_schema_id, movement_state,
};

const WORLD: &str = "test.world";
const POOL: &str = "pool.ferry.a-b";

macro_rules! world_kind {
    ($marker:ident, $name:literal) => {
        struct $marker;
        impl DomainRecordType for $marker {
            type Payload = serde_json::Value;
            type Class = DomainValueKindClass;
            const NAMESPACE: &'static str = WORLD;
            const NAME: &'static str = $name;
        }
    };
}

world_kind!(Hazard, "hazard");
world_kind!(Incident, "incident");
world_kind!(Dispatch, "dispatch");
world_kind!(Warrant, "warrant");

/// Owner of the application records the movement plugin cites.
struct WorldPlugin;

impl SimulationPlugin for WorldPlugin {
    fn name(&self) -> &'static str {
        "test-world"
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "00000000000000000000000000000000000000000000000000000000000000c1"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut warrant = DomainRecordSchema::for_record::<Warrant>();
        warrant.references = [
            (MOVEMENT_GRANTEE_ROLE, false),
            (MOVEMENT_SUBJECT_ROLE, true),
        ]
        .into_iter()
        .map(|(role, multiple)| DomainReferenceSchema {
            role: role.to_owned(),
            targets: vec![DomainReferenceTargetKind::AnyEntity],
            required: false,
            multiple,
            allow_retired: false,
        })
        .collect();
        for mut schema in [
            DomainRecordSchema::for_record::<Hazard>(),
            DomainRecordSchema::for_record::<Incident>(),
            DomainRecordSchema::for_record::<Dispatch>(),
            warrant,
        ] {
            schema.payload_schema = PayloadSchema::Any;
            registrar.register_record_schema(schema)?;
        }
        Ok(())
    }
}

fn t(minutes: i64) -> SimTime {
    SimTime::from_minutes(minutes)
}

fn holder(person: u64) -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(PersonId::new(person))
}

fn world_record(name: &str, id: &str) -> DomainRecord {
    DomainRecord {
        reference: DomainRecordRef::new(WORLD, name, id),
        owner: "test-world".to_owned(),
        class: DomainRecordClass::Record,
        version: 1,
        lifecycle: DomainRecordLifecycle::Active,
        payload: serde_json::json!({ "id": id }),
        references: Vec::new(),
    }
}

fn version(name: &str, id: &str, version: u64) -> DomainRecordVersionRef {
    DomainRecordVersionRef {
        record: DomainRecordRef::new(WORLD, name, id),
        version,
        established_by: DomainRecordVersionSource::InitialScenario,
    }
}

/// Person 1's warrant to move persons 5, 6, and 7. The persons are listed
/// only as scenario entities, so restoring the initial record store must
/// accept them as well as live construction does.
fn warrant() -> DomainRecord {
    let person = |id| DomainReferenceTarget::Core(EntityRef::Person(PersonId::new(id)));
    let mut record = world_record("warrant", "warrant-1");
    record.references = [(MOVEMENT_GRANTEE_ROLE, 1), (MOVEMENT_SUBJECT_ROLE, 5)]
        .into_iter()
        .chain([(MOVEMENT_SUBJECT_ROLE, 6), (MOVEMENT_SUBJECT_ROLE, 7)])
        .map(|(role, id)| DomainReference {
            role: role.to_owned(),
            target: person(id),
        })
        .collect();
    record.references.sort();
    record
}

/// A plan through `stops`, ten minutes per leg from `start`.
fn plan(stops: &[&str], start: i64) -> RoutePlan {
    let legs = stops
        .windows(2)
        .zip(0_i64..)
        .map(|(pair, index)| RouteLeg {
            connection: RoutingConnectionRef::new(format!("{}-{}", pair[0], pair[1])),
            from: RoutingNodeRef::new(pair[0]),
            to: RoutingNodeRef::new(pair[1]),
            mode: TransferMode::Foot,
            planned_departure_at: t(start + index * 10),
            planned_arrival_at: t(start + index * 10 + 10),
        })
        .collect::<Vec<_>>();
    let arrival = legs.last().unwrap().planned_arrival_at;
    RoutePlan {
        algorithm_version: ROUTING_ALGORITHM_VERSION.to_owned(),
        policy_version: "fixture.policy.v1".to_owned(),
        planning_snapshot_digest: "fixture-snapshot".to_owned(),
        origin: RoutingNodeRef::new(stops[0]),
        destination: RoutingNodeRef::new(*stops.last().unwrap()),
        departure_at: t(start),
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

/// An order made at `ordered_at` moving `traveller` through `stops` from
/// `start`, authorized by the warrant record, operated by person 2, and
/// watched by person 3 thirty minutes late.
fn order(
    id: u64,
    traveller: u64,
    stops: &[&str],
    (ordered_at, start): (i64, i64),
    delivery_attempt: Option<DomainRecordVersionRef>,
) -> MovementOperation {
    let route = plan(stops, start);
    MovementOperation::Order(MovementOrderRequestV1 {
        execution: TransportExecutionId(id),
        order: MovementOrder {
            id: MovementOrderId(id),
            subjects: vec![MovementSubject {
                entity: EntityRef::Person(PersonId::new(traveller)),
                role: MovementSubjectRole::MovablePrincipal,
                quantity: None,
                expected_custody: None,
            }],
            origin: route.origin.clone(),
            destination: route.destination.clone(),
            plan: route,
            initiative: MovementInitiative::Commanded,
            ordered_at: t(ordered_at),
            expected_position_revision: 1,
        },
        delivery_attempt,
        operator: Some(holder(2)),
        remote_observers: vec![MovementRemoteObserverV1 {
            holder: holder(3),
            delay: SimDuration::minutes(30),
        }],
        authority_basis: Some(version("warrant", "warrant-1", 1)),
    })
}

fn booking(id: u64, execution: u64, priority: i32) -> MovementOperation {
    MovementOperation::RequestBooking(MovementBookingRequestV1 {
        booking: CapacityBooking::new(
            CapacityBookingId(id),
            TransportExecutionId(execution),
            "ferry_crossing".to_owned(),
            t(5),
            t(15),
            1,
            priority,
        )
        .unwrap(),
        pool: POOL.to_owned(),
        leg: LegExecutionId(1),
        tie_break: format!("execution-{execution}"),
    })
}

fn fail_leg(execution: u64, leg: u64) -> MovementOperation {
    MovementOperation::FailLeg {
        execution: TransportExecutionId(execution),
        leg: LegExecutionId(leg),
        reason: "ambushed".to_owned(),
    }
}

struct Host {
    canwu: Canwu,
    next_request: u64,
}

impl Host {
    fn request(&mut self, person: u64, key: &str, operation: MovementOperation) -> CommandRequest {
        self.next_request += 1;
        CommandRequest::new(
            CommandRequestId::new(self.next_request),
            self.canwu.revision(),
            CommandEnvelope::new(
                Issuer::Actor(PersonId::new(person)),
                movement_command(&MovementCommandV1 {
                    holder: holder(person),
                    operation_key: key.to_owned(),
                    operation,
                })
                .unwrap(),
            )
            .at_time(self.canwu.time()),
        )
    }

    /// Admits one tracked command and settles everything it makes due now.
    fn command(
        &mut self,
        person: u64,
        key: &str,
        operation: MovementOperation,
    ) -> CommandAttemptOutcome {
        let request = self.request(person, key, operation);
        let now = self.canwu.time();
        self.canwu.enqueue_command(now, 0, request).unwrap();
        self.canwu.advance_canonical(SimDuration::ZERO).unwrap();
        self.canwu
            .command_attempts()
            .last()
            .unwrap()
            .outcome
            .clone()
    }

    fn accept(&mut self, person: u64, key: &str, operation: MovementOperation) {
        let outcome = self.command(person, key, operation);
        assert!(
            matches!(outcome, CommandAttemptOutcome::Accepted { .. }),
            "{key}: {outcome:?}"
        );
    }

    /// Admits several commands in one boundary, so their operations settle
    /// together.
    fn accept_together(&mut self, commands: Vec<(u64, &str, MovementOperation)>) {
        let now = self.canwu.time();
        for (offset, (person, key, operation)) in (0_u64..).zip(commands) {
            let mut request = self.request(person, key, operation);
            request.expected_revision += offset;
            self.canwu.enqueue_command(now, 0, request).unwrap();
        }
        self.canwu.advance_canonical(SimDuration::ZERO).unwrap();
    }

    fn incident(
        &mut self,
        key: &str,
        evidence: DomainRecordVersionRef,
        operation: MovementOperation,
    ) {
        let request = movement_incident_ingress(
            &MovementIncidentV1 {
                operation_key: key.to_owned(),
                evidence,
                operation,
            },
            self.canwu.time(),
        )
        .unwrap();
        self.canwu.enqueue_plugin_ingress(request).unwrap();
        self.canwu.advance_canonical(SimDuration::ZERO).unwrap();
    }

    /// Settles every due boundary up to `until`, one at each due time.
    fn run_until(&mut self, until: i64) {
        let duration = t(until).checked_sub(self.canwu.time()).unwrap();
        self.canwu.advance_canonical(duration).unwrap();
    }

    fn state(&self) -> MovementState {
        movement_state(&self.canwu).unwrap().unwrap().1
    }

    fn command_outcome(&self, person: u64, key: &str) -> MovementOperationOutcomeV1 {
        self.state()
            .operation_outcome(&MovementOperationScopeV1::Holder(holder(person)), key)
            .unwrap()
            .unwrap()
            .clone()
    }

    fn incident_outcome(&self, kind: DomainRecordKind, key: &str) -> MovementOperationOutcomeV1 {
        self.state()
            .operation_outcome(&MovementOperationScopeV1::EvidenceKind(kind), key)
            .unwrap()
            .unwrap()
            .clone()
    }
}

fn rejected(outcome: &MovementOperationOutcomeV1) -> Option<MovementErrorCode> {
    assert_eq!(outcome.disposition, MovementOperationDisposition::Rejected);
    outcome.rejection_code
}

/// Every movement report a person holds, oldest first.
fn reports(canwu: &Canwu, person: u64) -> Vec<MovementReportV1> {
    canwu
        .viewer_for_actor(PersonId::new(person))
        .unwrap()
        .query_knowledge(&KnowledgeQuery {
            schemas: vec![movement_report_knowledge_schema_id()],
            view: KnowledgeHistoryView::FullHistory,
            limit: MAX_KNOWLEDGE_PAGE_SIZE,
            ..KnowledgeQuery::default()
        })
        .unwrap()
        .records
        .into_iter()
        .map(|record| serde_json::from_value(record.payload).unwrap())
        .collect()
}

fn latest(reports: &[MovementReportV1], execution: u64) -> &MovementReportV1 {
    reports
        .iter()
        .rev()
        .find(|report| report.execution == TransportExecutionId(execution))
        .unwrap()
}

#[test]
fn gap_g19_movement_lifecycle_plugin() {
    let people = (1..=7)
        .map(|person| EntityRef::Person(PersonId::new(person)))
        .collect();
    let scenario = Scenario::new(t(0), people).with_domain_records(vec![
        world_record("hazard", "flood-7"),
        world_record("incident", "ambush-1"),
        world_record("dispatch", "dispatch-1"),
        warrant(),
    ]);
    let movement = MovementPlugin::new([
        DomainRecordKind::for_type::<Hazard>(),
        DomainRecordKind::for_type::<Incident>(),
        DomainRecordKind::for_type::<Dispatch>(),
        DomainRecordKind::for_type::<Warrant>(),
    ]);
    let incident_kind = DomainRecordKind::for_type::<Incident>();
    let plugins: [&dyn SimulationPlugin; 2] = [&movement, &WorldPlugin];
    let mut host = Host {
        canwu: Canwu::new_with_plugins(19, scenario, &plugins).unwrap(),
        next_request: 0,
    };

    // Person 1 holds a one-crossing pool and a warrant to move persons 5 and
    // 6; person 2 operates both movements and person 3 watches from afar.
    host.accept(
        1,
        "pool",
        MovementOperation::OfferPool(MovementPoolOfferV1 {
            pool: POOL.to_owned(),
            resource: "ferry_crossing".to_owned(),
            window_from: t(0),
            window_until: t(200),
            quantity: 1,
        }),
    );
    host.accept(1, "order-1", order(1, 5, &["a", "b", "c"], (0, 5), None));
    host.accept(1, "order-2", order(2, 6, &["a", "b"], (0, 5), None));
    // Both bookings settle in one pass: priority, not admission order, wins,
    // and the crossing is confirmed before its window opens.
    host.accept_together(vec![
        (2, "book-2", booking(2, 2, 5)),
        (2, "book-1", booking(1, 1, 9)),
    ]);
    let state = host.state();
    assert_eq!(
        state.executions[&TransportExecutionId(1)].bookings[0].status,
        CapacityBookingStatus::Confirmed
    );
    assert_eq!(
        state.executions[&TransportExecutionId(2)].bookings[0].status,
        CapacityBookingStatus::Failed
    );
    assert_eq!(state.pools[POOL].booked, 1);
    let evidence = state.bookings[&CapacityBookingId(2)]
        .allocation
        .as_ref()
        .unwrap();
    assert!(evidence.digest_matches() && evidence.remaining_after == 0);

    // Operation keys are idempotent per holder: an exact retry changes
    // nothing and a conflicting reuse is refused at admission.
    let before = host.state();
    host.accept(1, "order-1", order(1, 5, &["a", "b", "c"], (0, 5), None));
    assert_eq!(host.state(), before);
    let CommandAttemptOutcome::Rejected { error } =
        host.command(1, "order-1", order(1, 5, &["a", "c"], (0, 5), None))
    else {
        panic!("a reused operation key with different input must be rejected");
    };
    assert_eq!(error.code, ErrorCode::IdempotencyConflict);

    // Moving someone else needs an authority basis that names the owner and
    // the subject: person 4 holds none, and person 1's warrant is not theirs.
    let mut unbacked = order(9, 7, &["a", "b"], (0, 5), None);
    if let MovementOperation::Order(request) = &mut unbacked {
        request.authority_basis = None;
    }
    host.accept(4, "unbacked", unbacked);
    host.accept(4, "borrowed", order(9, 7, &["a", "b"], (0, 5), None));
    for key in ["unbacked", "borrowed"] {
        assert_eq!(
            rejected(&host.command_outcome(4, key)),
            Some(MovementErrorCode::Unauthorized)
        );
    }

    // At the planned departure the booked leg consumes its capacity and the
    // leg whose booking failed fails instead of departing.
    host.run_until(5);
    let state = host.state();
    let one = &state.executions[&TransportExecutionId(1)];
    let two = &state.executions[&TransportExecutionId(2)];
    assert_eq!(one.legs[0].status, LegExecutionStatus::Departed);
    assert_eq!(one.bookings[0].status, CapacityBookingStatus::Consumed);
    assert_eq!(two.state, TransportExecutionState::ReplanPending);
    assert_eq!(
        two.legs[0].failure_reason.as_deref(),
        Some("capacity_unavailable")
    );
    assert_eq!(
        (state.pools[POOL].booked, state.pools[POOL].consumed),
        (0, 1)
    );

    // An unrelated person cannot act on a movement; its owner cancels it.
    host.accept(
        4,
        "intrude",
        MovementOperation::CompleteLeg {
            execution: TransportExecutionId(1),
            leg: LegExecutionId(1),
        },
    );
    assert_eq!(
        rejected(&host.command_outcome(4, "intrude")),
        Some(MovementErrorCode::Unauthorized)
    );
    host.accept(
        1,
        "cancel-2",
        MovementOperation::Cancel {
            execution: TransportExecutionId(2),
            reason: "no crossing".to_owned(),
        },
    );
    assert_eq!(
        host.state().executions[&TransportExecutionId(2)].state,
        TransportExecutionState::Cancelled
    );

    // Legs depart and arrive at their due times until the order settles.
    host.run_until(30);
    let state = host.state();
    let one = &state.executions[&TransportExecutionId(1)];
    assert_eq!(one.state, TransportExecutionState::Settled);
    let times = one
        .legs
        .iter()
        .map(|leg| (leg.actual_departure_at, leg.actual_arrival_at))
        .collect::<Vec<_>>();
    assert_eq!(
        times,
        vec![(Some(t(5)), Some(t(15))), (Some(t(15)), Some(t(25)))]
    );
    let closed = state.order_for_execution(TransportExecutionId(1)).unwrap();
    assert_eq!(
        (closed.closed_at, &closed.pending_due),
        (Some(t(25)), &None)
    );

    // A third movement is interrupted by an application-reported ambush.
    // Incidents must cite existing evidence of a declared kind and can only
    // fail, reroute, hand off, or reconcile.
    host.accept(1, "order-3", order(3, 7, &["a", "b", "c"], (30, 30), None));
    host.run_until(35);
    host.incident("forged", version("incident", "ambush-1", 2), fail_leg(3, 1));
    host.incident(
        "undeclared",
        DomainRecordVersionRef {
            record: DomainRecordRef::new("test.other", "note", "n-1"),
            version: 1,
            established_by: DomainRecordVersionSource::InitialScenario,
        },
        fail_leg(3, 1),
    );
    host.incident(
        "cancel",
        version("incident", "ambush-1", 1),
        MovementOperation::Cancel {
            execution: TransportExecutionId(3),
            reason: "ambushed".to_owned(),
        },
    );
    host.incident("ambush", version("incident", "ambush-1", 1), fail_leg(3, 1));
    assert_eq!(
        rejected(&host.incident_outcome(incident_kind.clone(), "forged")),
        Some(MovementErrorCode::Evidence)
    );
    assert_eq!(
        rejected(&host.incident_outcome(DomainRecordKind::new("test.other", "note"), "undeclared")),
        Some(MovementErrorCode::Evidence)
    );
    assert_eq!(
        rejected(&host.incident_outcome(incident_kind.clone(), "cancel")),
        Some(MovementErrorCode::Unauthorized)
    );
    assert_eq!(
        host.state().executions[&TransportExecutionId(3)].state,
        TransportExecutionState::ReplanPending
    );
    let interrupted = latest(&reports(&host.canwu, 1), 3).clone();
    assert_eq!(interrupted.phase, MovementPhaseV1::Interrupted);
    assert_eq!(
        interrupted.detail.as_ref().unwrap().failures[0].reason,
        "ambushed"
    );

    // The owner reroutes around a flooded ford; a seizure is an act by a
    // third party, so only the application's incident path may record it.
    host.run_until(40);
    let flood = ItineraryRevisionReason::ExternalCondition {
        record: DomainRecordRef::new(WORLD, "hazard", "flood-7"),
        version: 1,
        kind: "flood".to_owned(),
    };
    host.accept(
        1,
        "reroute-3",
        MovementOperation::Reroute {
            execution: TransportExecutionId(3),
            revision: ItineraryRevision {
                id: ItineraryRevisionId(2),
                predecessor: Some(ItineraryRevisionId(1)),
                plan: plan(&["a", "d", "c"], 41),
                planned_at: t(40),
                valid_from: t(41),
                reason: flood.clone(),
                superseded_at: None,
                evidence: Vec::new(),
            },
        },
    );
    let seizure = Handoff {
        id: HandoffId(1),
        from_leg: LegExecutionId(1),
        to_leg: LegExecutionId(3),
        from_custodian: "escort:2".to_owned(),
        to_custodian: "army:9".to_owned(),
        at: t(40),
        location: "a-b".to_owned(),
        evidence: Vec::new(),
        kind: HandoffKind::Seizure {
            by: EntityRef::Army(ArmyId::new(9)),
        },
    };
    let record_seizure = MovementOperation::RecordHandoff {
        execution: TransportExecutionId(3),
        handoff: seizure.clone(),
    };
    host.accept(1, "claimed-seizure", record_seizure.clone());
    assert_eq!(
        rejected(&host.command_outcome(1, "claimed-seizure")),
        Some(MovementErrorCode::Unauthorized)
    );
    host.incident(
        "seizure",
        version("incident", "ambush-1", 1),
        record_seizure,
    );

    // Pending leg due work and delayed-report wakes survive save/load.
    let mut restored =
        Canwu::from_snapshot_json_with_plugins(&host.canwu.snapshot_json().unwrap(), &plugins)
            .unwrap();
    host.run_until(65);
    restored
        .advance_canonical(t(65).checked_sub(restored.time()).unwrap())
        .unwrap();
    assert_eq!(movement_state(&restored).unwrap().unwrap().1, host.state());
    assert_eq!(restored.checkpoint_hash(), host.canwu.checkpoint_hash());
    let state = host.state();
    let three = &state.executions[&TransportExecutionId(3)];
    assert_eq!(three.state, TransportExecutionState::Settled);
    assert_eq!(three.revisions[1].reason, flood);
    assert_eq!(three.revisions[0].superseded_at, Some(t(40)));
    assert_eq!(three.handoffs, vec![seizure.clone()]);
    assert_eq!(three.legs[2].actual_departure_at, Some(t(41)));
    assert_eq!(three.legs[3].actual_arrival_at, Some(t(61)));

    // A movement that completes a delivery waits for the delivery's own
    // record to reconcile it; the owner cannot declare the outcome.
    host.run_until(66);
    host.accept(
        1,
        "order-4",
        order(
            4,
            6,
            &["a", "b"],
            (66, 66),
            Some(version("dispatch", "dispatch-1", 1)),
        ),
    );
    host.run_until(76);
    assert_eq!(
        host.state().executions[&TransportExecutionId(4)].state,
        TransportExecutionState::ArrivalPending
    );
    assert_eq!(
        latest(&reports(&host.canwu, 1), 4).phase,
        MovementPhaseV1::ArrivalPending
    );
    let reconcile = MovementOperation::Reconcile {
        execution: TransportExecutionId(4),
        outcome: ReconciliationOutcome::Success,
    };
    host.accept(1, "declare-delivered", reconcile.clone());
    assert_eq!(
        rejected(&host.command_outcome(1, "declare-delivered")),
        Some(MovementErrorCode::Unauthorized)
    );
    host.incident(
        "delivered-4",
        version("dispatch", "dispatch-1", 1),
        reconcile,
    );
    assert_eq!(
        host.state().executions[&TransportExecutionId(4)].state,
        TransportExecutionState::Settled
    );

    // Closed executions retire at the next settlement after every observer's
    // final report is out, keeping their identities reserved and their
    // consumed capacity counted; person 1's pool revision triggers it.
    host.run_until(120);
    host.accept(
        1,
        "pool-revision",
        MovementOperation::OfferPool(MovementPoolOfferV1 {
            pool: POOL.to_owned(),
            resource: "ferry_crossing".to_owned(),
            window_from: t(0),
            window_until: t(200),
            quantity: 2,
        }),
    );
    let state = host.state();
    state.validate().unwrap();
    assert!(state.executions.is_empty() && state.orders.is_empty());
    assert_eq!(
        state.retired_executions,
        (1..=4).map(TransportExecutionId).collect()
    );
    assert_eq!(state.retired_consumption[POOL], 1);
    assert_eq!(
        (state.pools[POOL].quantity, state.pools[POOL].consumed),
        (2, 1)
    );
    assert_eq!(
        host.command_outcome(1, "pool").disposition,
        MovementOperationDisposition::Applied
    );

    // Reports are holder-relative: the owner and operator see the current
    // detail, the remote observer sees only progress thirty minutes late, a
    // person without a grant sees nothing, and no report repeats unchanged.
    let owner = reports(&host.canwu, 1);
    let operator = reports(&host.canwu, 2);
    let remote = reports(&host.canwu, 3);
    for (view, role) in [
        (&owner, MovementObserverRole::Owner),
        (&operator, MovementObserverRole::Operator),
    ] {
        let report = latest(view, 3);
        assert_eq!(
            (report.role, report.phase),
            (role, MovementPhaseV1::Settled)
        );
        let detail = report.detail.as_ref().unwrap();
        assert_eq!(detail.handoffs, vec![seizure.clone()]);
        assert_eq!(detail.revisions[1].reason, flood);
        assert_eq!(report.observed_as_of, t(61));
    }
    assert!(remote.iter().all(
        |report| report.detail.is_none() && report.role == MovementObserverRole::DelayedRemote
    ));
    let remote_interrupted = remote
        .iter()
        .find(|report| {
            report.execution == TransportExecutionId(3)
                && report.phase == MovementPhaseV1::Interrupted
        })
        .unwrap();
    assert_eq!(remote_interrupted.observed_as_of, t(35));
    assert_eq!(remote_interrupted.legs[0].failed_at, Some(t(35)));
    assert_eq!(latest(&remote, 3).phase, MovementPhaseV1::Settled);
    assert_eq!(latest(&remote, 3).observed_as_of, t(61));
    assert_eq!(latest(&remote, 4).phase, MovementPhaseV1::Settled);
    assert!(reports(&host.canwu, 4).is_empty());
    for view in [&owner, &operator, &remote] {
        for execution in 1..=4 {
            let digests = view
                .iter()
                .filter(|report| report.execution == TransportExecutionId(execution))
                .map(|report| report.digest.as_str())
                .collect::<Vec<_>>();
            assert!(digests.windows(2).all(|pair| pair[0] != pair[1]));
        }
    }

    // Save/load and exact replay reproduce the movement runtime.
    let restored =
        Canwu::from_snapshot_json_with_plugins(&host.canwu.snapshot_json().unwrap(), &plugins)
            .unwrap();
    assert_eq!(movement_state(&restored).unwrap().unwrap().1, state);
    assert_eq!(restored.checkpoint_hash(), host.canwu.checkpoint_hash());
    let replayed = Canwu::replay_from_journal(&plugins, &host.canwu.replay_journal()).unwrap();
    assert_eq!(movement_state(&replayed).unwrap().unwrap().1, state);
    assert_eq!(replayed.checkpoint_hash(), host.canwu.checkpoint_hash());
}
