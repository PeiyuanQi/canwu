//! Public-API fixture for downstream gaps G-73, G-79, and G-80 and engine
//! finding E-48 of the `canwu-movement` lifecycle plugin.
//!
//! G-80: a leg that has not departed is held by `DelayLeg` on its departure,
//! any number of times, without an itinerary revision; repeated holds keep
//! the first bound. G-73: a hold with `retime_later_legs` moves the later
//! legs' departures and booking windows by the part of the hold that
//! elapsed, so they keep their planned spacing; without it a later leg keeps
//! its planned time and departs at the held arrival. E-48: a due that a hold
//! replaces is withdrawn from the queue. G-79: a pool's custodian, or an
//! application incident, revokes a confirmed booking; the freed capacity can
//! be booked by another execution; a revoked booking fails its leg at
//! departure unless its owner replaced it, and a booking is replaced once.

#![allow(clippy::too_many_lines)]

use canwu_api::{
    Canwu, CanwuError, CapacityBooking, CapacityBookingId, CapacityBookingStatus,
    CommandAttemptOutcome, CommandEnvelope, CommandRequest, CommandRequestId, DomainRecord,
    DomainRecordClass, DomainRecordKind, DomainRecordLifecycle, DomainRecordRef,
    DomainRecordSchema, DomainRecordType, DomainRecordVersionRef, DomainRecordVersionSource,
    DomainValueKindClass, EntityRef, IngressPayload, Issuer, KnowledgeHistoryView,
    KnowledgeHolderRef, KnowledgeQuery, LegExecutionId, LegExecutionStatus,
    MAX_KNOWLEDGE_PAGE_SIZE, MovementInitiative, MovementOrder, MovementOrderId, MovementSubject,
    MovementSubjectRole, PayloadSchema, PersonId, PluginRegistrar, ROUTING_ALGORITHM_VERSION,
    RouteCost, RouteLeg, RoutePlan, RoutingConnectionRef, RoutingNodeRef, Scenario, SimDuration,
    SimTime, SimulationPlugin, TransferMode, TransportExecutionId, TransportExecutionState,
};
use canwu_movement::{
    MOVEMENT_LEG_DUE_INGRESS, MovementBookingRequestV1, MovementCommandV1, MovementErrorCode,
    MovementIncidentV1, MovementLegDueKind, MovementLegDueV1, MovementOperation,
    MovementOperationDisposition, MovementOperationOutcomeV1, MovementOperationScopeV1,
    MovementOrderRecordV1, MovementOrderRequestV1, MovementPlugin, MovementPoolOfferV1,
    MovementReportV1, MovementState, PLUGIN_NAME, movement_command, movement_incident_ingress,
    movement_report_knowledge_schema_id, movement_state,
};

const WORLD: &str = "test.world";
const WORLD_PLUGIN: &str = "test-world";

struct Incident;

impl DomainRecordType for Incident {
    type Payload = serde_json::Value;
    type Class = DomainValueKindClass;
    const NAMESPACE: &'static str = WORLD;
    const NAME: &'static str = "incident";
}

/// Owner of the incident record that application incidents cite.
struct WorldPlugin;

impl SimulationPlugin for WorldPlugin {
    fn name(&self) -> &'static str {
        WORLD_PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "00000000000000000000000000000000000000000000000000000000000000c3"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut schema = DomainRecordSchema::for_record::<Incident>();
        schema.payload_schema = PayloadSchema::Any;
        registrar.register_record_schema(schema)
    }
}

fn t(minutes: i64) -> SimTime {
    SimTime::from_minutes(minutes)
}

fn holder(person: u64) -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(PersonId::new(person))
}

fn scenario() -> Scenario {
    let people = (1..=7)
        .map(|person| EntityRef::Person(PersonId::new(person)))
        .collect();
    Scenario::new(t(0), people).with_domain_records(vec![DomainRecord {
        reference: DomainRecordRef::new(WORLD, "incident", "requisition-1"),
        owner: WORLD_PLUGIN.to_owned(),
        class: DomainRecordClass::Record,
        version: 1,
        lifecycle: DomainRecordLifecycle::Active,
        payload: serde_json::json!({ "id": "requisition-1" }),
        references: Vec::new(),
    }])
}

fn incident_evidence() -> DomainRecordVersionRef {
    DomainRecordVersionRef {
        record: DomainRecordRef::new(WORLD, "incident", "requisition-1"),
        version: 1,
        established_by: DomainRecordVersionSource::InitialScenario,
    }
}

fn plugins() -> (MovementPlugin, WorldPlugin) {
    (
        MovementPlugin::new([DomainRecordKind::for_type::<Incident>()]),
        WorldPlugin,
    )
}

/// A plan through `stops`: each leg takes `minutes` and waits `gap` minutes
/// before the next leg departs, the first departing at `start`.
fn plan(stops: &[&str], start: i64, minutes: i64, gap: i64) -> RoutePlan {
    let legs = stops
        .windows(2)
        .zip(0_i64..)
        .map(|(pair, index)| {
            let departure = start + index * (minutes + gap);
            RouteLeg {
                connection: RoutingConnectionRef::new(format!("{}-{}", pair[0], pair[1])),
                from: RoutingNodeRef::new(pair[0]),
                to: RoutingNodeRef::new(pair[1]),
                mode: TransferMode::Foot,
                planned_departure_at: t(departure),
                planned_arrival_at: t(departure + minutes),
            }
        })
        .collect::<Vec<_>>();
    let arrival = legs.last().unwrap().planned_arrival_at;
    RoutePlan {
        algorithm_version: ROUTING_ALGORITHM_VERSION.to_owned(),
        policy_version: "fixture.policy.v1".to_owned(),
        planning_snapshot_digest: "fixture-snapshot".to_owned(),
        edge_conditions_digest: None,
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

/// A self-directed journey of `person` along `route`, ordered at time 0.
fn order(id: u64, person: u64, route: RoutePlan) -> MovementOperation {
    MovementOperation::Order(MovementOrderRequestV1 {
        execution: TransportExecutionId(id),
        order: MovementOrder {
            id: MovementOrderId(id),
            subjects: vec![MovementSubject {
                entity: EntityRef::Person(PersonId::new(person)),
                role: MovementSubjectRole::MovablePrincipal,
                quantity: None,
                expected_custody: None,
            }],
            origin: route.origin.clone(),
            destination: route.destination.clone(),
            plan: route,
            initiative: MovementInitiative::SelfDirected,
            ordered_at: t(0),
            expected_position_revision: 1,
        },
        delivery_attempt: None,
        operator: None,
        remote_observers: Vec::new(),
        authority_basis: None,
    })
}

fn hold(execution: u64, leg: u64, until: i64, retime_later_legs: bool) -> MovementOperation {
    MovementOperation::DelayLeg {
        execution: TransportExecutionId(execution),
        leg: LegExecutionId(leg),
        until: t(until),
        reason: "reach closed".to_owned(),
        retime_later_legs,
    }
}

fn pool(id: &str, quantity: u64) -> MovementOperation {
    MovementOperation::OfferPool(MovementPoolOfferV1 {
        pool: id.to_owned(),
        resource: "boat".to_owned(),
        window_from: t(0),
        window_until: t(400),
        quantity,
    })
}

#[allow(clippy::too_many_arguments)]
fn booking(
    id: u64,
    execution: u64,
    leg: u64,
    pool: &str,
    window: (i64, i64),
    quantity: u64,
    replaces: Option<u64>,
) -> MovementOperation {
    MovementOperation::RequestBooking(MovementBookingRequestV1 {
        booking: CapacityBooking::new(
            CapacityBookingId(id),
            TransportExecutionId(execution),
            "boat".to_owned(),
            t(window.0),
            t(window.1),
            quantity,
            0,
        )
        .unwrap(),
        pool: pool.to_owned(),
        leg: LegExecutionId(leg),
        tie_break: format!("booking-{id}"),
        replaces: replaces.map(CapacityBookingId),
    })
}

fn revoke(pool: &str, booking: u64) -> MovementOperation {
    MovementOperation::RevokeBooking {
        pool: pool.to_owned(),
        booking: CapacityBookingId(booking),
        reason: "displaced_by_requisition".to_owned(),
    }
}

struct Host {
    canwu: Canwu,
    next_request: u64,
}

impl Host {
    fn new(seed: u64, plugins: &[&dyn SimulationPlugin]) -> Self {
        Self {
            canwu: Canwu::new_with_plugins(seed, scenario(), plugins).unwrap(),
            next_request: 0,
        }
    }

    /// Admits one tracked command and settles its boundary, which must not
    /// fail whatever the operation's outcome.
    fn accept(&mut self, person: u64, key: &str, operation: MovementOperation) {
        self.next_request += 1;
        let now = self.canwu.time();
        let request = CommandRequest::new(
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
            .at_time(now),
        );
        self.canwu.enqueue_command(now, 0, request).unwrap();
        self.canwu.advance_canonical(SimDuration::ZERO).unwrap();
        let outcome = &self.canwu.command_attempts().last().unwrap().outcome;
        assert!(
            matches!(outcome, CommandAttemptOutcome::Accepted { .. }),
            "{key}: {outcome:?}"
        );
    }

    fn incident(&mut self, key: &str, operation: MovementOperation) {
        let at = self.canwu.time();
        self.canwu
            .enqueue_plugin_ingress(
                movement_incident_ingress(
                    &MovementIncidentV1 {
                        operation_key: key.to_owned(),
                        evidence: incident_evidence(),
                        operation,
                    },
                    at,
                )
                .unwrap(),
            )
            .unwrap();
        self.canwu.advance_canonical(SimDuration::ZERO).unwrap();
    }

    fn run_until(&mut self, until: i64) {
        let duration = t(until).checked_sub(self.canwu.time()).unwrap();
        self.canwu.advance_canonical(duration).unwrap();
    }

    fn state(&self) -> MovementState {
        movement_state(&self.canwu).unwrap().unwrap().1
    }

    fn order(&self, execution: u64) -> MovementOrderRecordV1 {
        self.state()
            .order_for_execution(TransportExecutionId(execution))
            .unwrap()
            .clone()
    }

    fn outcome(&self, person: u64, key: &str) -> MovementOperationOutcomeV1 {
        self.state()
            .operation_outcome(&MovementOperationScopeV1::Holder(holder(person)), key)
            .unwrap()
            .unwrap()
            .clone()
    }

    fn legs(&self, execution: u64) -> Vec<(Option<SimTime>, Option<SimTime>)> {
        self.state().executions[&TransportExecutionId(execution)]
            .legs
            .iter()
            .map(|leg| (leg.actual_departure_at, leg.actual_arrival_at))
            .collect()
    }

    fn booking_status(&self, execution: u64, booking: u64) -> CapacityBookingStatus {
        self.state().executions[&TransportExecutionId(execution)]
            .bookings
            .iter()
            .find(|item| item.id == CapacityBookingId(booking))
            .unwrap()
            .status
    }

    /// The movement leg-due packets still queued for one execution.
    fn queued_dues(&self, execution: u64) -> Vec<MovementLegDueV1> {
        self.canwu
            .pending_ingress()
            .filter_map(|record| match &record.payload {
                IngressPayload::Plugin {
                    plugin,
                    packet_type,
                    payload,
                    ..
                } if plugin == PLUGIN_NAME && packet_type == MOVEMENT_LEG_DUE_INGRESS => {
                    serde_json::from_value::<MovementLegDueV1>(payload.clone()).ok()
                }
                _ => None,
            })
            .filter(|due| due.execution == TransportExecutionId(execution))
            .collect()
    }
}

fn applied(outcome: &MovementOperationOutcomeV1) {
    assert_eq!(
        outcome.disposition,
        MovementOperationDisposition::Applied,
        "{outcome:?}"
    );
}

fn rejected(outcome: &MovementOperationOutcomeV1) -> Option<MovementErrorCode> {
    assert_eq!(outcome.disposition, MovementOperationDisposition::Rejected);
    outcome.rejection_code
}

fn latest_report(canwu: &Canwu, person: u64, execution: u64) -> MovementReportV1 {
    reports(canwu, person, execution).pop().unwrap()
}

/// Every movement report `person` received about `execution`, in order.
fn reports(canwu: &Canwu, person: u64, execution: u64) -> Vec<MovementReportV1> {
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
        .map(|record| serde_json::from_value::<MovementReportV1>(record.payload).unwrap())
        .filter(|report| report.execution == TransportExecutionId(execution))
        .collect()
}

#[test]
fn gap_g73_g80_holds_retime_later_legs_without_a_revision() {
    let (movement, world) = plugins();
    let plugins: [&dyn SimulationPlugin; 2] = [&movement, &world];
    let mut host = Host::new(73, &plugins);
    host.accept(7, "pool", pool("pool.ferry", 1));
    // Two journeys a > b > c > d: ten-minute legs departing at 10, 30, and
    // 50, each waiting ten minutes at a stop.
    host.accept(
        5,
        "order-1",
        order(1, 5, plan(&["a", "b", "c", "d"], 10, 10, 10)),
    );
    host.accept(
        6,
        "order-2",
        order(2, 6, plan(&["a", "b", "c", "d"], 10, 10, 10)),
    );

    // G-80: the first leg has not departed; it is held at its origin nine
    // times, more holds than an execution has itinerary revisions, and no
    // revision is spent. Each hold withdraws the due it replaces (E-48).
    for until in 11..=19 {
        host.accept(5, &format!("hold-{until}"), hold(1, 1, until, true));
        applied(&host.outcome(5, &format!("hold-{until}")));
    }
    let order = host.order(1);
    let held = order.hold.as_ref().unwrap();
    assert_eq!(
        (held.kind, held.scheduled_at, held.until),
        (MovementLegDueKind::Departure, t(10), t(19))
    );
    assert_eq!(
        host.state().executions[&TransportExecutionId(1)]
            .revisions
            .len(),
        1
    );
    assert_eq!(
        host.queued_dues(1)
            .iter()
            .map(|due| (due.kind, due.due_at))
            .collect::<Vec<_>>(),
        vec![(MovementLegDueKind::Departure, t(19))]
    );
    // Repeated holds keep the first bound: the hold may shorten to the
    // planned departure but not before it.
    host.accept(5, "earlier", hold(1, 1, 9, true));
    assert_eq!(
        rejected(&host.outcome(5, "earlier")),
        Some(MovementErrorCode::InvalidOperation)
    );
    host.accept(5, "shorten", hold(1, 1, 15, true));
    applied(&host.outcome(5, "shorten"));
    assert_eq!(host.queued_dues(1).len(), 1);

    // The held leg departs at 15; its re-timing moves the later legs by the
    // five minutes it held.
    host.run_until(16);
    assert_eq!(host.legs(1)[0].0, Some(t(15)));
    assert_eq!(host.order(1).schedule_shift, SimDuration::minutes(5));
    assert_eq!(host.order(1).hold, None);

    // The second journey's first leg is held under way until 32 without
    // re-timing: its next leg keeps its planned time and departs at the held
    // arrival.
    host.accept(6, "hold-2", hold(2, 1, 32, false));
    applied(&host.outcome(6, "hold-2"));

    // The first journey arrives at 25 and waits its planned ten minutes: the
    // next departure is 35, not its planned 30 or the arrival instant.
    host.run_until(26);
    let due = host.order(1).pending_due.unwrap();
    assert_eq!(
        (due.kind, due.due_at),
        (MovementLegDueKind::Departure, t(35))
    );

    // G-73: the second leg, under way from 35, is held until 52 with
    // re-timing. A booking for the third leg must use its re-timed window:
    // the planned one (50 to 60) is refused, the window moved by the five
    // minutes already re-timed and the seven of the pending hold is admitted.
    host.run_until(36);
    host.accept(5, "hold-under-way", hold(1, 2, 52, true));
    applied(&host.outcome(5, "hold-under-way"));
    host.accept(
        5,
        "book-planned",
        booking(1, 1, 3, "pool.ferry", (50, 60), 1, None),
    );
    assert_eq!(
        rejected(&host.outcome(5, "book-planned")),
        Some(MovementErrorCode::InvalidOperation)
    );
    host.accept(
        5,
        "book-retimed",
        booking(2, 1, 3, "pool.ferry", (62, 72), 1, None),
    );
    applied(&host.outcome(5, "book-retimed"));
    assert_eq!(host.booking_status(1, 2), CapacityBookingStatus::Confirmed);
    assert_eq!(host.legs(2)[1].0, Some(t(32)));
    let saved = host.canwu.snapshot_json().unwrap();

    // Without re-timing the third leg of the second journey kept its
    // planned departure at 50; the journey settles at 60.
    host.run_until(61);
    assert_eq!(
        host.legs(2),
        vec![
            (Some(t(10)), Some(t(32))),
            (Some(t(32)), Some(t(42))),
            (Some(t(50)), Some(t(60))),
        ]
    );
    assert!(host.queued_dues(2).is_empty());

    // The held leg arrived at 52; the third leg departs ten minutes later at
    // 62 on its re-timed booking.
    host.run_until(73);
    assert_eq!(
        host.legs(1),
        vec![
            (Some(t(15)), Some(t(25))),
            (Some(t(35)), Some(t(52))),
            (Some(t(62)), Some(t(72))),
        ]
    );
    // The settled execution keeps no booking; the pool still counts the one
    // its third leg consumed.
    let state = host.state();
    let settled = &state.executions[&TransportExecutionId(1)];
    assert_eq!(settled.state, TransportExecutionState::Settled);
    assert!(settled.bookings.is_empty());
    assert_eq!(state.pools["pool.ferry"].consumed, 1);
    assert!(host.queued_dues(1).is_empty());
    // The owner's report keeps the shift after the holds have ended.
    let report = latest_report(&host.canwu, 5, 1);
    assert_eq!(
        report.detail.unwrap().schedule_shift,
        SimDuration::minutes(12)
    );

    // A run restored while the hold under way was pending, and exact replay,
    // agree.
    let mut restored = Canwu::from_snapshot_json_with_plugins(&saved, &plugins).unwrap();
    restored
        .advance_canonical(t(73).checked_sub(restored.time()).unwrap())
        .unwrap();
    assert_eq!(movement_state(&restored).unwrap().unwrap().1, host.state());
    assert_eq!(restored.checkpoint_hash(), host.canwu.checkpoint_hash());
    let replayed = Canwu::replay_from_journal(&plugins, &host.canwu.replay_journal()).unwrap();
    assert_eq!(replayed.checkpoint_hash(), host.canwu.checkpoint_hash());
}

#[test]
fn gap_g79_revoked_booking_is_replaced_by_its_owner() {
    let (movement, world) = plugins();
    let plugins: [&dyn SimulationPlugin; 2] = [&movement, &world];
    let mut host = Host::new(79, &plugins);
    host.accept(7, "pool", pool("pool.boats", 3));
    host.accept(7, "pool-upstream", pool("pool.boats.upstream", 2));
    // Two merchants' lots (persons 5 and 6) and a requisitioned crossing
    // (person 4), each one leg a > b from 10 to 20.
    for (execution, person) in [(1, 5), (2, 6), (3, 4)] {
        host.accept(
            person,
            &format!("order-{execution}"),
            order(execution, person, plan(&["a", "b"], 10, 10, 0)),
        );
    }
    host.accept(
        5,
        "book-1",
        booking(1, 1, 1, "pool.boats", (10, 20), 2, None),
    );
    host.accept(
        6,
        "book-2",
        booking(2, 2, 1, "pool.boats", (10, 20), 1, None),
    );
    assert_eq!(host.booking_status(1, 1), CapacityBookingStatus::Confirmed);
    assert_eq!(host.booking_status(2, 2), CapacityBookingStatus::Confirmed);

    // Refusals are recorded: a holder that is not the custodian, a booking
    // that is not on the named pool, and one that is unknown.
    host.accept(4, "not-custodian", revoke("pool.boats", 1));
    host.accept(7, "wrong-pool", revoke("pool.boats.upstream", 1));
    host.accept(7, "unknown", revoke("pool.boats", 9));
    for (person, key, code) in [
        (4, "not-custodian", MovementErrorCode::Unauthorized),
        (7, "wrong-pool", MovementErrorCode::NotFound),
        (7, "unknown", MovementErrorCode::NotFound),
    ] {
        assert_eq!(rejected(&host.outcome(person, key)), Some(code), "{key}");
    }

    // The custodian revokes the first lot and an application incident the
    // second; their capacity returns to the pool, and a revoked booking
    // cannot be revoked again.
    host.accept(7, "revoke-1", revoke("pool.boats", 1));
    applied(&host.outcome(7, "revoke-1"));
    host.incident("revoke-2", revoke("pool.boats", 2));
    host.accept(7, "revoke-again", revoke("pool.boats", 1));
    assert_eq!(
        rejected(&host.outcome(7, "revoke-again")),
        Some(MovementErrorCode::InvalidState)
    );
    let state = host.state();
    let revoked = &state.executions[&TransportExecutionId(1)].bookings[0];
    assert_eq!(revoked.status, CapacityBookingStatus::Revoked);
    assert_eq!(
        revoked
            .revocation
            .as_ref()
            .map(|revocation| (revocation.at, revocation.reason.as_str())),
        Some((t(0), "displaced_by_requisition"))
    );
    assert_eq!(host.booking_status(2, 2), CapacityBookingStatus::Revoked);
    assert_eq!(state.pools["pool.boats"].booked, 0);

    // The requisition takes the whole pool.
    host.accept(
        4,
        "requisition",
        booking(3, 3, 1, "pool.boats", (10, 20), 3, None),
    );
    assert_eq!(host.booking_status(3, 3), CapacityBookingStatus::Confirmed);

    // The first merchant re-requests its lot elsewhere in place of the
    // revoked booking; a second replacement of it is a conflict.
    host.accept(
        5,
        "rebook-1",
        booking(4, 1, 1, "pool.boats.upstream", (10, 20), 2, Some(1)),
    );
    applied(&host.outcome(5, "rebook-1"));
    host.accept(
        5,
        "rebook-1-again",
        booking(5, 1, 1, "pool.boats.upstream", (10, 20), 1, Some(1)),
    );
    assert_eq!(
        rejected(&host.outcome(5, "rebook-1-again")),
        Some(MovementErrorCode::Conflict)
    );
    assert_eq!(host.booking_status(1, 4), CapacityBookingStatus::Confirmed);

    // At 10 the replaced lot and the requisition depart on their bookings;
    // the lot whose revoked booking was not replaced fails its leg. The
    // replaced booking left its execution when it was replaced.
    host.run_until(11);
    let state = host.state();
    assert_eq!(
        state.executions[&TransportExecutionId(1)].legs[0].actual_departure_at,
        Some(t(10))
    );
    assert_eq!(host.booking_status(1, 4), CapacityBookingStatus::Consumed);
    assert_eq!(
        state.executions[&TransportExecutionId(1)]
            .bookings
            .iter()
            .map(|booking| booking.id)
            .collect::<Vec<_>>(),
        vec![CapacityBookingId(4)]
    );
    assert_eq!(host.booking_status(3, 3), CapacityBookingStatus::Consumed);
    let failed = &state.executions[&TransportExecutionId(2)];
    assert_eq!(failed.legs[0].status, LegExecutionStatus::Failed);
    assert_eq!(
        failed.legs[0].failure_reason.as_deref(),
        Some("capacity_unavailable")
    );
    assert_eq!(failed.state, TransportExecutionState::ReplanPending);

    // The owner's report showed why its first booking was revoked until the
    // booking was replaced.
    let revoked = reports(&host.canwu, 5, 1)
        .into_iter()
        .filter_map(|report| {
            report
                .detail?
                .bookings
                .into_iter()
                .find(|item| item.id == CapacityBookingId(1))?
                .revocation
                .map(|revocation| revocation.reason)
        })
        .collect::<Vec<_>>();
    assert_eq!(revoked, vec!["displaced_by_requisition".to_owned()]);

    let replayed = Canwu::replay_from_journal(&plugins, &host.canwu.replay_journal()).unwrap();
    assert_eq!(movement_state(&replayed).unwrap().unwrap().1, host.state());
    assert_eq!(replayed.checkpoint_hash(), host.canwu.checkpoint_hash());
}
