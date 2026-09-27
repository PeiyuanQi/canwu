//! Gap G-09: decision ticket lineage. A follow-up ticket names a terminal
//! parent of the same decision maker, or, for a seat succession, a parent
//! whose controller is bound to the same seat; the parent is validated at
//! admission, persisted on the ticket and its trace, and survives save/load
//! and replay.

use canwu_api::{
    Canwu, DecisionAttemptErrorCode, DecisionAttemptOutcome, DecisionAuthority, DecisionContext,
    DecisionControllerBinding, DecisionEvaluation, DecisionHistoryKey, DecisionHistoryLocation,
    DecisionIngressRequest, DecisionMutation, DecisionOption, DecisionRequestId,
    DecisionTicketDraft, DecisionTicketId, DecisionTicketState, EntityRef, UtilityProfile,
    WeightedUtilityPolicy,
};
use serde_json::json;
use std::collections::BTreeMap;

const CONTROLLER: &str = "lineage-controller";
const SEAT_HOLDER: &str = "north-seat-holder";
const SEAT_SUCCESSOR: &str = "north-seat-successor";
const OTHER_SEAT: &str = "south-seat-holder";

fn policy() -> WeightedUtilityPolicy {
    WeightedUtilityPolicy::new(
        "lineage-utility",
        "1",
        UtilityProfile {
            weights: BTreeMap::from([("merit".to_owned(), 1)]),
        },
    )
}

fn draft(id: u64, decision_maker: EntityRef, parent: Option<u64>) -> DecisionTicketDraft {
    seat_draft(CONTROLLER, id, decision_maker, parent)
}

fn seat_draft(
    controller: &str,
    id: u64,
    decision_maker: EntityRef,
    parent: Option<u64>,
) -> DecisionTicketDraft {
    DecisionTicketDraft {
        id: DecisionTicketId::new(id),
        definition: "fixture.follow-up".to_owned(),
        decision_maker,
        assigned_controller: controller.to_owned(),
        summary: format!("Fixture decision {id}"),
        context: DecisionContext::new("fixture.follow-up.v1", json!({ "ticket": id })),
        options: vec![DecisionOption {
            utility_inputs: BTreeMap::from([("merit".to_owned(), 1)]),
            ..DecisionOption::new("proceed", "Proceed")
        }],
        deadline: None,
        parent_ticket: parent.map(DecisionTicketId::new),
    }
}

/// Enqueues every mutation at the current revision and settles one boundary.
fn settle(canwu: &mut Canwu, mutations: Vec<(u64, DecisionMutation)>) {
    let (now, revision) = (canwu.time(), canwu.revision());
    for (request_id, mutation) in mutations {
        canwu
            .enqueue_decision(
                now,
                0,
                DecisionIngressRequest::new(DecisionRequestId::new(request_id), revision, mutation),
            )
            .expect("decision ingress should queue");
    }
    canwu
        .step_canonical()
        .expect("decision boundary should settle")
        .expect("decision boundary receipt");
}

fn rejection(canwu: &Canwu, request_id: u64) -> (DecisionAttemptErrorCode, String) {
    match canwu
        .decision_attempt(DecisionRequestId::new(request_id))
        .map(|attempt| &attempt.outcome)
    {
        Some(DecisionAttemptOutcome::Rejected { code, message }) => (*code, message.clone()),
        other => panic!("request {request_id} should be rejected, got {other:?}"),
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn gap_g09_decision_ticket_lineage() {
    let mut canwu = Canwu::demo(912).expect("demo");
    let ids = Canwu::demo_ids();
    let holder = EntityRef::Person(ids.commander);
    let other_holder = EntityRef::Person(ids.observer);
    let open = |ticket| DecisionMutation::Open { ticket };
    settle(
        &mut canwu,
        vec![
            (
                1,
                DecisionMutation::RegisterController {
                    controller: DecisionControllerBinding::new(
                        CONTROLLER,
                        policy().identity.clone(),
                        DecisionAuthority::Actor {
                            actor: ids.commander,
                        },
                    ),
                },
            ),
            (2, open(draft(1, holder.clone(), None))),
            (3, open(draft(2, other_holder.clone(), None))),
            // The parent exists but is still open.
            (4, open(draft(3, holder.clone(), Some(1)))),
        ],
    );
    let (code, message) = rejection(&canwu, 4);
    assert_eq!(code, DecisionAttemptErrorCode::InvalidDecision);
    assert!(message.contains("still open"), "{message}");
    assert!(canwu.decision_ticket(DecisionTicketId::new(3)).is_none());

    let cancel = |ticket_id: u64| DecisionMutation::Cancel {
        ticket_id: DecisionTicketId::new(ticket_id),
        expected_version: 1,
        reason: "superseded by a follow-up".to_owned(),
    };
    settle(&mut canwu, vec![(5, cancel(1)), (6, cancel(2))]);
    settle(
        &mut canwu,
        vec![
            // A closed parent that belongs to another decision maker.
            (7, open(draft(4, holder.clone(), Some(2)))),
            // A parent that does not exist.
            (8, open(draft(5, holder.clone(), Some(99)))),
            // A closed parent of the same decision maker.
            (9, open(draft(6, holder.clone(), Some(1)))),
        ],
    );
    let (code, message) = rejection(&canwu, 7);
    assert_eq!(code, DecisionAttemptErrorCode::InvalidDecision);
    assert!(message.contains("different decision maker"), "{message}");
    assert_eq!(
        rejection(&canwu, 8).0,
        DecisionAttemptErrorCode::TicketNotFound
    );
    assert!(canwu.decision_ticket(DecisionTicketId::new(4)).is_none());
    assert!(canwu.decision_ticket(DecisionTicketId::new(5)).is_none());
    let child = canwu
        .decision_ticket(DecisionTicketId::new(6))
        .expect("valid lineage should admit the follow-up ticket");
    assert_eq!(child.parent_ticket, Some(DecisionTicketId::new(1)));
    assert!(matches!(
        canwu
            .decision_ticket(DecisionTicketId::new(1))
            .map(|parent| &parent.state),
        Some(DecisionTicketState::Cancelled { .. })
    ));

    // The resolution trace carries the lineage into decision history.
    let evaluation = canwu
        .drive_decision(
            canwu.time(),
            0,
            DecisionRequestId::new(10),
            None,
            DecisionTicketId::new(6),
            &policy(),
        )
        .expect("follow-up decision should prepare");
    let DecisionEvaluation::Prepared(prepared) = evaluation else {
        panic!("utility policy should resolve the follow-up ticket");
    };
    canwu
        .step_canonical()
        .expect("resolution boundary")
        .expect("resolution receipt");
    let Some(DecisionAttemptOutcome::Accepted {
        trace_id: Some(trace_id),
        ..
    }) = canwu
        .decision_attempt(prepared.request.request_id)
        .map(|attempt| attempt.outcome.clone())
    else {
        panic!("follow-up resolution should be accepted with a trace");
    };
    let trace = canwu.decision_trace(trace_id).expect("follow-up trace");
    assert_eq!(trace.parent_ticket, Some(DecisionTicketId::new(1)));
    assert_eq!(
        canwu.decision_history_location(&DecisionHistoryKey::Ticket(DecisionTicketId::new(1))),
        DecisionHistoryLocation::Hot
    );

    // Seat succession: the seat's first holder cannot continue, so a
    // successor holder, a different decision maker, registers a new
    // controller bound to the same seat and follows the cancelled ticket.
    let seat_controller = |id: &str, actor, seat: &str| DecisionMutation::RegisterController {
        controller: DecisionControllerBinding::new(
            id,
            policy().identity.clone(),
            DecisionAuthority::Actor { actor },
        )
        .with_seat(seat, "fixture.seat-profile"),
    };
    settle(
        &mut canwu,
        vec![
            (
                11,
                seat_controller(SEAT_HOLDER, ids.commander, "seat.north"),
            ),
            (
                12,
                seat_controller(SEAT_SUCCESSOR, ids.observer, "seat.north"),
            ),
            (13, seat_controller(OTHER_SEAT, ids.observer, "seat.south")),
            (14, open(seat_draft(SEAT_HOLDER, 7, holder.clone(), None))),
        ],
    );
    settle(&mut canwu, vec![(15, cancel(7))]);
    settle(
        &mut canwu,
        vec![
            // The successor holder of the same seat.
            (
                16,
                open(seat_draft(SEAT_SUCCESSOR, 8, other_holder.clone(), Some(7))),
            ),
            // The same successor under a controller of another seat.
            (
                17,
                open(seat_draft(OTHER_SEAT, 9, other_holder.clone(), Some(7))),
            ),
            // A controller without a seat never continues another maker's
            // ticket.
            (18, open(draft(10, other_holder.clone(), Some(7)))),
        ],
    );
    let successor = canwu
        .decision_ticket(DecisionTicketId::new(8))
        .expect("the same seat should admit the successor's follow-up ticket");
    assert_eq!(successor.parent_ticket, Some(DecisionTicketId::new(7)));
    assert_eq!(successor.decision_maker, other_holder);
    for (request_id, ticket_id) in [(17, 9), (18, 10)] {
        let (code, message) = rejection(&canwu, request_id);
        assert_eq!(code, DecisionAttemptErrorCode::InvalidDecision);
        assert!(message.contains("different decision maker"), "{message}");
        assert!(
            canwu
                .decision_ticket(DecisionTicketId::new(ticket_id))
                .is_none()
        );
    }

    // Lineage is persisted state: old-default tickets keep their shape, and
    // the chain survives strict save/load and exact replay.
    let parent_json = serde_json::to_value(
        canwu
            .decision_ticket(DecisionTicketId::new(1))
            .expect("parent"),
    )
    .expect("parent json");
    assert!(parent_json.get("parent_ticket").is_none());
    let snapshot = canwu.snapshot();
    let restored =
        Canwu::from_snapshot_json(&serde_json::to_string(&snapshot).expect("snapshot json"))
            .expect("lineage should reload");
    assert_eq!(restored.snapshot(), snapshot);
    assert_eq!(
        restored
            .decision_ticket(DecisionTicketId::new(6))
            .and_then(|ticket| ticket.parent_ticket),
        Some(DecisionTicketId::new(1))
    );
    let replayed =
        Canwu::replay_from_journal(&[], &canwu.replay_journal()).expect("lineage should replay");
    assert_eq!(replayed.snapshot(), snapshot);
}
