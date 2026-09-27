//! Public-API fixture for gap G-20: capacity pools and deterministic booking
//! allocation.

#![allow(clippy::too_many_lines)]

use canwu_core::{KnowledgeHolderRef, PersonId};
use canwu_time::SimTime;
use canwu_transport::{
    BookingAllocationV1, CapacityAllocationFailureV1, CapacityBooking, CapacityBookingId,
    CapacityBookingRequestV1, CapacityBookingStatus, TransportCapacityPoolV1, TransportError,
    TransportExecutionId, allocate_capacity_bookings, capacity_booking_allocation_operation_key,
};
use serde_json::json;

fn t(minutes: i64) -> SimTime {
    SimTime::from_minutes(minutes)
}

struct Spec {
    id: u64,
    priority: i32,
    valid_from: i64,
    valid_until: i64,
    quantity: u64,
    tie_break: &'static str,
    admitted_sequence: u64,
}

fn request(spec: &Spec) -> CapacityBookingRequestV1 {
    CapacityBookingRequestV1 {
        booking: CapacityBooking::new(
            CapacityBookingId(spec.id),
            TransportExecutionId(100 + spec.id),
            "ferry_crossing".to_owned(),
            t(spec.valid_from),
            t(spec.valid_until),
            spec.quantity,
            spec.priority,
        )
        .unwrap(),
        tie_break: spec.tie_break.to_owned(),
        admitted_sequence: spec.admitted_sequence,
    }
}

fn pool() -> TransportCapacityPoolV1 {
    TransportCapacityPoolV1::new(
        "pool.ferry.a-b".to_owned(),
        "ferry_crossing".to_owned(),
        KnowledgeHolderRef::Person(PersonId::new(1)),
        t(0),
        t(100),
        7,
    )
    .unwrap()
}

fn summary(allocations: &[BookingAllocationV1]) -> Vec<(u64, CapacityBookingStatus, u64, u64)> {
    allocations
        .iter()
        .map(|allocation| {
            (
                allocation.booking.0,
                allocation.status,
                allocation.quantity,
                allocation.evidence.remaining_after,
            )
        })
        .collect()
}

#[test]
fn gap_g20_transport_capacity_pool_allocation() {
    use CapacityBookingStatus::{Confirmed, Failed};
    let specs = [
        // Same priority, window, and tie-break as 4 and 8, but admitted later.
        Spec {
            id: 2,
            priority: 5,
            valid_from: 10,
            valid_until: 20,
            quantity: 1,
            tie_break: "a",
            admitted_sequence: 1,
        },
        // Loses to tie-break "a".
        Spec {
            id: 1,
            priority: 5,
            valid_from: 10,
            valid_until: 20,
            quantity: 2,
            tie_break: "b",
            admitted_sequence: 0,
        },
        // Highest priority is served first even though its window opens last.
        Spec {
            id: 3,
            priority: 9,
            valid_from: 30,
            valid_until: 40,
            quantity: 3,
            tie_break: "z",
            admitted_sequence: 2,
        },
        // Ties with 8 on every key except the booking identity.
        Spec {
            id: 4,
            priority: 5,
            valid_from: 10,
            valid_until: 20,
            quantity: 1,
            tie_break: "a",
            admitted_sequence: 0,
        },
        // The earlier window wins among equal priorities.
        Spec {
            id: 5,
            priority: 5,
            valid_from: 5,
            valid_until: 15,
            quantity: 2,
            tie_break: "z",
            admitted_sequence: 9,
        },
        // Extends past the pool window.
        Spec {
            id: 6,
            priority: 7,
            valid_from: 90,
            valid_until: 120,
            quantity: 1,
            tie_break: "a",
            admitted_sequence: 3,
        },
        // Its window ended before the allocation time.
        Spec {
            id: 7,
            priority: 1,
            valid_from: 0,
            valid_until: 3,
            quantity: 1,
            tie_break: "a",
            admitted_sequence: 4,
        },
        // Over capacity when its turn comes; a later, smaller request still fits.
        Spec {
            id: 8,
            priority: 5,
            valid_from: 10,
            valid_until: 20,
            quantity: 2,
            tie_break: "a",
            admitted_sequence: 0,
        },
    ];
    let requests = specs.iter().map(request).collect::<Vec<_>>();
    let pool = pool();
    let at = t(5);
    let allocations = allocate_capacity_bookings(&pool, &requests, at).unwrap();
    assert_eq!(
        summary(&allocations),
        vec![
            (3, Confirmed, 3, 4),
            (6, Failed, 0, 4),
            (5, Confirmed, 2, 2),
            (4, Confirmed, 1, 1),
            (8, Failed, 0, 1),
            (2, Confirmed, 1, 0),
            (1, Failed, 0, 0),
            (7, Failed, 0, 0),
        ]
    );
    let failures = allocations
        .iter()
        .map(|allocation| allocation.evidence.failure)
        .collect::<Vec<_>>();
    assert_eq!(
        failures,
        vec![
            None,
            Some(CapacityAllocationFailureV1::OutsidePoolWindow),
            None,
            None,
            Some(CapacityAllocationFailureV1::InsufficientCapacity),
            None,
            Some(CapacityAllocationFailureV1::InsufficientCapacity),
            Some(CapacityAllocationFailureV1::WindowElapsed),
        ]
    );

    // The result depends only on the requests, never on their input order.
    for rotation in 1..requests.len() {
        let mut permuted = requests.clone();
        permuted.rotate_left(rotation);
        permuted.swap(0, rotation % requests.len());
        assert_eq!(
            allocate_capacity_bookings(&pool, &permuted, at).unwrap(),
            allocations
        );
    }
    let mut reversed = requests.clone();
    reversed.reverse();
    assert_eq!(
        allocate_capacity_bookings(&pool, &reversed, at).unwrap(),
        allocations
    );

    // Evidence binds the pool revision, carries a stable operation key, and
    // has a stable, tamper-evident digest.
    let first = &allocations[0].evidence;
    assert_eq!(
        first.operation_key,
        capacity_booking_allocation_operation_key("pool.ferry.a-b", 1, CapacityBookingId(3))
    );
    assert_eq!(
        first.operation_key,
        "transport/pool/pool.ferry.a-b/revision/1/booking/3/allocation"
    );
    assert_eq!(
        first.semantic_digest,
        "8ede51fcdf563729b0dc0337fda47a58daa80ffbf4b4c0180bec9704270f57bd"
    );
    assert!(
        allocations
            .iter()
            .all(|allocation| allocation.evidence.digest_matches())
    );
    let encoded = serde_json::to_value(first).unwrap();
    assert!(encoded.get("failure").is_none());
    assert_eq!(
        serde_json::to_value(&allocations[1].evidence).unwrap()["failure"],
        json!("outside_pool_window")
    );
    let restored: Vec<BookingAllocationV1> =
        serde_json::from_str(&serde_json::to_string(&allocations).unwrap()).unwrap();
    assert_eq!(restored, allocations);

    // Committing the pass books the confirmed quantity and advances the pool
    // revision; the same pass cannot be committed twice, and tampered or
    // duplicated evidence is refused without changing the pool.
    let mut committed = pool.clone();
    let mut tampered = allocations.clone();
    tampered[0].quantity = 2;
    tampered[0].evidence.quantity = 2;
    assert!(matches!(
        committed.apply_allocations(&tampered),
        Err(TransportError::InvalidCapacityPool(_))
    ));
    assert_eq!(committed, pool);
    let doubled = [allocations[0].clone(), allocations[0].clone()];
    assert!(committed.apply_allocations(&doubled).is_err());
    assert_eq!(committed, pool);
    committed.apply_allocations(&allocations).unwrap();
    assert_eq!(
        (committed.booked, committed.consumed, committed.revision),
        (7, 0, 2)
    );
    assert_eq!(committed.available(), 0);
    assert!(committed.apply_allocations(&allocations).is_err());
    let restored_pool: TransportCapacityPoolV1 =
        serde_json::from_str(&serde_json::to_string(&committed).unwrap()).unwrap();
    assert_eq!(restored_pool, committed);

    // A full pool fails every further request deterministically.
    let late = request(&Spec {
        id: 9,
        priority: 100,
        valid_from: 10,
        valid_until: 20,
        quantity: 1,
        tie_break: "a",
        admitted_sequence: 5,
    });
    let over = allocate_capacity_bookings(&committed, std::slice::from_ref(&late), at).unwrap();
    assert_eq!(summary(&over), vec![(9, Failed, 0, 0)]);
    assert_eq!(
        over[0].evidence.failure,
        Some(CapacityAllocationFailureV1::InsufficientCapacity)
    );
    assert_eq!(over[0].evidence.pool_revision, 2);

    // Malformed passes are rejected rather than partially allocated.
    let mut duplicate = requests.clone();
    duplicate.push(requests[0].clone());
    let mut not_requested = requests.clone();
    not_requested[0].booking.status = Confirmed;
    let mut foreign = requests.clone();
    foreign[0].booking.resource = "cart_load".to_owned();
    for invalid in [duplicate, not_requested, foreign] {
        assert!(matches!(
            allocate_capacity_bookings(&pool, &invalid, at),
            Err(TransportError::InvalidCapacityPool(_))
        ));
    }

    // Booking transitions carry the pool counters. A booking may be confirmed
    // before its window opens but consumed only inside its window.
    let mut booking = requests[2].booking.clone();
    booking.transition(Confirmed, at).unwrap();
    for outside in [t(29), t(41)] {
        assert!(matches!(
            booking.transition(CapacityBookingStatus::Consumed, outside),
            Err(TransportError::InvalidBooking(_))
        ));
    }
    booking
        .transition(CapacityBookingStatus::Consumed, t(30))
        .unwrap();
    committed
        .apply_booking_transition(3, Confirmed, CapacityBookingStatus::Consumed)
        .unwrap();
    assert_eq!(
        (committed.booked, committed.consumed, committed.revision),
        (4, 3, 3)
    );
    committed
        .apply_booking_transition(2, Confirmed, CapacityBookingStatus::Expired)
        .unwrap();
    committed
        .apply_booking_transition(
            3,
            CapacityBookingStatus::Consumed,
            CapacityBookingStatus::Released,
        )
        .unwrap();
    assert_eq!(
        (committed.booked, committed.consumed, committed.available()),
        (2, 0, 5)
    );
    assert!(
        committed
            .apply_booking_transition(1, CapacityBookingStatus::Requested, Confirmed)
            .is_err()
    );
    assert!(committed.revise(t(0), t(100), 1).is_err());
    committed.revise(t(0), t(200), 9).unwrap();
    assert_eq!(
        (
            committed.quantity,
            committed.available(),
            committed.revision
        ),
        (9, 7, 6)
    );
}
