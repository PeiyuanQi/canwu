//! Public-API fixture for downstream gap G-83 of the `canwu-movement`
//! lifecycle plugin.
//!
//! An execution keeps only the bookings that still bear on its legs: those
//! that ask for or hold capacity, those the leg under way consumed, and those
//! lost for a leg that has not departed until they are replaced. A replaced
//! booking leaves at once, and a consumed one when its leg arrives, its
//! capacity still counted by its pool. So a leg can book more lots than the
//! 0.20.0 lifetime bound of 16, and re-book them in place any number of
//! times, while `bookings_per_execution` bounds what the execution keeps at
//! once.

#![allow(clippy::too_many_lines)]

use canwu_api::{
    Canwu, CapacityBooking, CapacityBookingId, CapacityBookingStatus, CommandAttemptOutcome,
    CommandEnvelope, CommandRequest, CommandRequestId, EntityRef, Issuer, KnowledgeHolderRef,
    LegExecutionId, MovementInitiative, MovementOrder, MovementOrderId, MovementSubject,
    MovementSubjectRole, PersonId, ROUTING_ALGORITHM_VERSION, RouteCost, RouteLeg, RoutePlan,
    RoutingConnectionRef, RoutingNodeRef, Scenario, SimDuration, SimTime, SimulationPlugin,
    TransferMode, TransportExecutionId, TransportExecutionState,
};
use canwu_movement::{
    MovementBookingRequestV1, MovementCommandV1, MovementErrorCode, MovementLimitsV1,
    MovementOperation, MovementOperationDisposition, MovementOperationScopeV1,
    MovementOrderRequestV1, MovementPlugin, MovementPoolOfferV1, MovementState, movement_command,
    movement_state,
};

const POOL: &str = "pool.boats";
const OWNER: u64 = 5;
const CUSTODIAN: u64 = 7;
const LOTS: u64 = 20;

fn t(minutes: i64) -> SimTime {
    SimTime::from_minutes(minutes)
}

fn holder(person: u64) -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(PersonId::new(person))
}

/// A journey a > b > c whose legs leave at 10 and 30 and take ten minutes.
fn journey() -> MovementOperation {
    let legs = [("a", "b", 10), ("b", "c", 30)]
        .into_iter()
        .map(|(from, to, departure)| RouteLeg {
            connection: RoutingConnectionRef::new(format!("{from}-{to}")),
            from: RoutingNodeRef::new(from),
            to: RoutingNodeRef::new(to),
            mode: TransferMode::Foot,
            planned_departure_at: t(departure),
            planned_arrival_at: t(departure + 10),
        })
        .collect::<Vec<_>>();
    let plan = RoutePlan {
        algorithm_version: ROUTING_ALGORITHM_VERSION.to_owned(),
        policy_version: "fixture.policy.v1".to_owned(),
        planning_snapshot_digest: "fixture-snapshot".to_owned(),
        edge_conditions_digest: None,
        origin: RoutingNodeRef::new("a"),
        destination: RoutingNodeRef::new("c"),
        departure_at: t(10),
        estimated_arrival_at: t(40),
        cost: RouteCost {
            estimated_arrival_at: t(40),
            risk_per_mille: 0,
            resource_cost: 0,
            transfers: 0,
        },
        legs,
        digest: "a>b>c".to_owned(),
    };
    MovementOperation::Order(MovementOrderRequestV1 {
        execution: TransportExecutionId(1),
        order: MovementOrder {
            id: MovementOrderId(1),
            subjects: vec![MovementSubject {
                entity: EntityRef::Person(PersonId::new(OWNER)),
                role: MovementSubjectRole::MovablePrincipal,
                quantity: None,
                expected_custody: None,
            }],
            origin: plan.origin.clone(),
            destination: plan.destination.clone(),
            plan,
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

/// One lot of the pool for `leg` (1 leaves at 10, 2 at 30).
fn lot(id: u64, leg: u64, replaces: Option<u64>) -> MovementOperation {
    let departure = if leg == 1 { 10 } else { 30 };
    MovementOperation::RequestBooking(MovementBookingRequestV1 {
        booking: CapacityBooking::new(
            CapacityBookingId(id),
            TransportExecutionId(1),
            "boat".to_owned(),
            t(departure),
            t(departure + 10),
            1,
            0,
        )
        .unwrap(),
        pool: POOL.to_owned(),
        leg: LegExecutionId(leg),
        tie_break: format!("lot-{id}"),
        replaces: replaces.map(CapacityBookingId),
    })
}

struct Host {
    canwu: Canwu,
    next_request: u64,
}

impl Host {
    /// Admits several commands in one boundary, so their operations settle
    /// together; each must be admitted, and the boundary must stand whatever
    /// the operations' outcomes.
    fn accept(&mut self, commands: Vec<(u64, String, MovementOperation)>) {
        let now = self.canwu.time();
        let revision = self.canwu.revision();
        let first = self.canwu.command_attempts().len();
        let count = commands.len();
        for (offset, (person, key, operation)) in (0_u64..).zip(commands) {
            self.next_request += 1;
            let request = CommandRequest::new(
                CommandRequestId::new(self.next_request),
                revision + offset,
                CommandEnvelope::new(
                    Issuer::Actor(PersonId::new(person)),
                    movement_command(&MovementCommandV1 {
                        holder: holder(person),
                        operation_key: key,
                        operation,
                    })
                    .unwrap(),
                )
                .at_time(now),
            );
            self.canwu.enqueue_command(now, 0, request).unwrap();
        }
        self.canwu.advance_canonical(SimDuration::ZERO).unwrap();
        let attempts = &self.canwu.command_attempts()[first..];
        assert_eq!(attempts.len(), count);
        assert!(
            attempts
                .iter()
                .all(|attempt| matches!(attempt.outcome, CommandAttemptOutcome::Accepted { .. }))
        );
    }

    fn run_until(&mut self, until: i64) {
        let duration = t(until).checked_sub(self.canwu.time()).unwrap();
        self.canwu.advance_canonical(duration).unwrap();
    }

    fn state(&self) -> MovementState {
        movement_state(&self.canwu).unwrap().unwrap().1
    }

    /// The rejection code of one operation, `None` when it was applied.
    fn rejection(&self, person: u64, key: &str) -> Option<MovementErrorCode> {
        let state = self.state();
        let outcome = state
            .operation_outcome(&MovementOperationScopeV1::Holder(holder(person)), key)
            .unwrap()
            .unwrap();
        assert_eq!(
            outcome.disposition == MovementOperationDisposition::Applied,
            outcome.rejection_code.is_none()
        );
        outcome.rejection_code
    }

    /// The bookings the execution keeps, with their status.
    fn kept(&self) -> Vec<(u64, CapacityBookingStatus)> {
        self.state().executions[&TransportExecutionId(1)]
            .bookings
            .iter()
            .map(|booking| (booking.id.0, booking.status))
            .collect()
    }
}

fn keyed(
    person: u64,
    key: String,
    operation: MovementOperation,
) -> (u64, String, MovementOperation) {
    (person, key, operation)
}

#[test]
fn gap_g83_closed_and_replaced_bookings_leave_their_execution() {
    let movement = MovementPlugin::new([]);
    let plugins: [&dyn SimulationPlugin; 1] = [&movement];
    let people = [OWNER, CUSTODIAN]
        .into_iter()
        .map(|person| EntityRef::Person(PersonId::new(person)))
        .collect();
    let mut host = Host {
        canwu: Canwu::new_with_plugins(83, Scenario::new(t(0), people), &plugins).unwrap(),
        next_request: 0,
    };
    let bound = MovementLimitsV1::CURRENT.bookings_per_execution;
    host.accept(vec![
        keyed(
            CUSTODIAN,
            "pool".to_owned(),
            MovementOperation::OfferPool(MovementPoolOfferV1 {
                pool: POOL.to_owned(),
                resource: "boat".to_owned(),
                window_from: t(0),
                window_until: t(400),
                quantity: 200,
            }),
        ),
        keyed(OWNER, "order".to_owned(), journey()),
    ]);

    // The first leg books twenty lots at once, more than an execution could
    // book in its whole life in 0.20.0.
    host.accept(
        (1..=LOTS)
            .map(|id| keyed(OWNER, format!("lot-{id}"), lot(id, 1, None)))
            .collect(),
    );
    assert_eq!(
        host.kept(),
        (1..=LOTS)
            .map(|id| (id, CapacityBookingStatus::Confirmed))
            .collect::<Vec<_>>()
    );

    // Three times the custodian takes every lot back and the owner books
    // each again in place of the one it lost: eighty bookings over the leg's
    // life, but the execution keeps only the twenty that bear on it.
    for round in 1..=3_u64 {
        let lost = (round - 1) * LOTS + 1..=round * LOTS;
        host.accept(
            lost.clone()
                .map(|id| {
                    keyed(
                        CUSTODIAN,
                        format!("revoke-{id}"),
                        MovementOperation::RevokeBooking {
                            pool: POOL.to_owned(),
                            booking: CapacityBookingId(id),
                            reason: "requisitioned".to_owned(),
                        },
                    )
                })
                .collect(),
        );
        assert!(
            host.kept()
                .iter()
                .all(|(_, status)| *status == CapacityBookingStatus::Revoked)
        );
        host.accept(
            lost.map(|id| {
                keyed(
                    OWNER,
                    format!("lot-{}", id + LOTS),
                    lot(id + LOTS, 1, Some(id)),
                )
            })
            .collect(),
        );
        let state = host.state();
        assert_eq!(
            host.kept(),
            (round * LOTS + 1..=(round + 1) * LOTS)
                .map(|id| (id, CapacityBookingStatus::Confirmed))
                .collect::<Vec<_>>()
        );
        assert_eq!(u64::try_from(state.bookings.len()).unwrap(), LOTS);
        assert_eq!(state.pools[POOL].booked, LOTS);
        assert!(
            state
                .bookings
                .values()
                .all(|binding| binding.replaces.map(|id| id.0 + LOTS) == Some(binding.booking.0))
        );
        if round == 1 {
            // A replaced booking has left, but its identity stays taken while
            // its replacement is kept: replacing it again is a conflict, and
            // so is a new booking under its identity.
            host.accept(vec![
                keyed(OWNER, "again".to_owned(), lot(99, 1, Some(1))),
                keyed(OWNER, "reuse".to_owned(), lot(1, 1, None)),
            ]);
            assert_eq!(
                host.rejection(OWNER, "again"),
                Some(MovementErrorCode::Conflict)
            );
            assert_eq!(
                host.rejection(OWNER, "reuse"),
                Some(MovementErrorCode::Conflict)
            );
        }
    }

    // The second leg's lots fill the bound; one more is refused, recorded,
    // and the boundary stands.
    let fill = u64::try_from(bound).unwrap() - LOTS;
    host.accept(
        (101..101 + fill)
            .map(|id| keyed(OWNER, format!("lot-{id}"), lot(id, 2, None)))
            .collect(),
    );
    let over = 101 + fill;
    host.accept(vec![keyed(OWNER, "over".to_owned(), lot(over, 2, None))]);
    assert_eq!(
        host.rejection(OWNER, "over"),
        Some(MovementErrorCode::LimitExceeded)
    );
    assert_eq!(host.kept().len(), bound);

    // Under way, the first leg's consumed lots are kept with the second leg's
    // confirmed ones; when it arrives they leave, their capacity still
    // counted by the pool, and the refused lot now fits.
    host.run_until(11);
    let state = host.state();
    assert_eq!(
        state.executions[&TransportExecutionId(1)].bookings.len(),
        bound
    );
    assert_eq!(
        (state.pools[POOL].booked, state.pools[POOL].consumed),
        (fill, LOTS)
    );
    host.run_until(21);
    let state = host.state();
    assert_eq!(host.kept().len(), bound - usize::try_from(LOTS).unwrap());
    assert_eq!(state.retired_consumption[POOL], LOTS);
    assert_eq!(state.pools[POOL].consumed, LOTS);
    host.accept(vec![keyed(
        OWNER,
        "over-later".to_owned(),
        lot(over, 2, None),
    )]);
    assert_eq!(host.rejection(OWNER, "over-later"), None);
    let saved = host.canwu.snapshot_json().unwrap();

    // The settled execution keeps no booking, and the pool counts every lot
    // the journey consumed.
    host.run_until(41);
    let state = host.state();
    state.validate().unwrap();
    let execution = &state.executions[&TransportExecutionId(1)];
    assert_eq!(execution.state, TransportExecutionState::Settled);
    assert!(execution.bookings.is_empty() && state.bookings.is_empty());
    assert_eq!(
        (state.pools[POOL].booked, state.pools[POOL].consumed),
        (0, LOTS + fill + 1)
    );
    assert_eq!(state.retired_consumption[POOL], LOTS + fill + 1);

    // A run restored after the first leg arrived, and exact replay, agree.
    let mut restored = Canwu::from_snapshot_json_with_plugins(&saved, &plugins).unwrap();
    restored
        .advance_canonical(t(41).checked_sub(restored.time()).unwrap())
        .unwrap();
    assert_eq!(movement_state(&restored).unwrap().unwrap().1, state);
    assert_eq!(restored.checkpoint_hash(), host.canwu.checkpoint_hash());
    let replayed = Canwu::replay_from_journal(&plugins, &host.canwu.replay_journal()).unwrap();
    assert_eq!(replayed.checkpoint_hash(), host.canwu.checkpoint_hash());
}
