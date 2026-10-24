//! Downstream gap G-81: an open ticket's deadline moves later
//! (`ExtendDeadline`) and its controller moves to another registered
//! controller (`Reassign`) under the same ticket ID. Both are checked at
//! admission, every expected refusal is a recorded rejection, an accepted one
//! advances the ticket version and moves its index entry, and the result
//! survives strict save/load and exact replay.

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundaryRequest,
    BoundarySystemContract, Canwu, CanwuError, CustodyState, DecisionAttemptErrorCode,
    DecisionAttemptOutcome, DecisionAuthority, DecisionContext, DecisionControllerBinding,
    DecisionEvaluation, DecisionIngressRequest, DecisionMutation, DecisionOption,
    DecisionRequestId, DecisionTicket, DecisionTicketDraft, DecisionTicketId, DecisionTicketState,
    EntityRef, LifeState, PersonAvailability, PluginRegistrar, PolicyDecision, SimDuration,
    SimTime, SimulationPlugin, SimulationView, StateKey, StateVisibility, SystemCadence,
    UtilityProfile, WeightedUtilityPolicy,
};
use serde_json::json;
use std::collections::BTreeMap;

const SEAT: &str = "g81-seat";
const DELEGATE: &str = "g81-delegate";
const HOLDER: &str = "g81-seat-holder";
const SUCCESSOR: &str = "g81-seat-successor";
const DEPUTY: &str = "g81-seat-deputy";

/// Captures the observer, the delegate controller's authority person, at the
/// first `Daily` boundary.
fn capture_observer(
    view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let observer = Canwu::demo_ids().observer;
    if view.person_availability(observer)?.is_some() {
        return Ok(BoundaryProposal::default());
    }
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::SetPersonAvailability {
            person: observer,
            availability: PersonAvailability::new(
                LifeState::Alive,
                CustodyState::Captive,
                view.time(),
            ),
            summary: "Raiders captured the delegate".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

struct CapturePlugin;

impl SimulationPlugin for CapturePlugin {
    fn name(&self) -> &'static str {
        "fixture-g81-capture"
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000000a81"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut contract = BoundarySystemContract::new(
            "capture",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        contract.reads = vec![StateKey::core_person_availability()];
        contract.writes = vec![StateKey::core_person_availability()];
        contract.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(contract, capture_observer)
    }
}

fn policy() -> WeightedUtilityPolicy {
    WeightedUtilityPolicy::new(
        "g81-utility",
        "1",
        UtilityProfile {
            weights: BTreeMap::from([("merit".to_owned(), 1)]),
        },
    )
}

fn draft(id: u64, decision_maker: EntityRef, deadline: Option<SimTime>) -> DecisionTicketDraft {
    DecisionTicketDraft {
        id: DecisionTicketId::new(id),
        definition: "fixture.held-ticket".to_owned(),
        decision_maker,
        assigned_controller: SEAT.to_owned(),
        summary: format!("Fixture decision {id}"),
        context: DecisionContext::new("fixture.held-ticket.v1", json!({ "ticket": id })),
        options: vec![DecisionOption {
            utility_inputs: BTreeMap::from([("merit".to_owned(), 1)]),
            ..DecisionOption::new("proceed", "Proceed")
        }],
        deadline,
        parent_ticket: None,
    }
}

/// Enqueues every mutation at `at` against the current revision and settles
/// that one boundary.
fn settle_at(canwu: &mut Canwu, at: SimTime, mutations: Vec<(u64, DecisionMutation)>) {
    let revision = canwu.revision();
    for (request_id, mutation) in mutations {
        canwu
            .enqueue_decision(
                at,
                0,
                DecisionIngressRequest::new(DecisionRequestId::new(request_id), revision, mutation),
            )
            .expect("decision ingress should queue");
    }
    canwu
        .step_canonical()
        .expect("decision boundary should settle")
        .expect("decision boundary receipt");
    assert_eq!(canwu.time(), at);
}

fn outcome(canwu: &Canwu, request_id: u64) -> DecisionAttemptOutcome {
    canwu
        .decision_attempt(DecisionRequestId::new(request_id))
        .map_or_else(
            || panic!("request {request_id} should have an attempt"),
            |attempt| attempt.outcome.clone(),
        )
}

fn rejection(canwu: &Canwu, request_id: u64) -> (DecisionAttemptErrorCode, String) {
    match outcome(canwu, request_id) {
        DecisionAttemptOutcome::Rejected { code, message } => (code, message),
        other @ DecisionAttemptOutcome::Accepted { .. } => {
            panic!("request {request_id} should be rejected, got {other:?}")
        }
    }
}

fn extend(ticket: u64, version: u64, deadline: SimTime) -> DecisionMutation {
    DecisionMutation::ExtendDeadline {
        ticket_id: DecisionTicketId::new(ticket),
        expected_version: version,
        deadline,
    }
}

fn reassign(ticket: u64, version: u64, controller: &str) -> DecisionMutation {
    DecisionMutation::Reassign {
        ticket_id: DecisionTicketId::new(ticket),
        expected_version: version,
        controller_id: controller.to_owned(),
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn gap_g81_decision_deadline_and_reassign() {
    let capture = CapturePlugin;
    let plugins: [&dyn SimulationPlugin; 1] = [&capture];
    let mut canwu = Canwu::demo(981).expect("demo");
    canwu.register_plugin(&capture).expect("capture plugin");
    let ids = Canwu::demo_ids();
    let maker = EntityRef::Person(ids.commander);
    let start = canwu.time();
    let hours = |count| start + SimDuration::hours(count);
    let register = |id: &str, actor| DecisionMutation::RegisterController {
        controller: DecisionControllerBinding::new(
            id,
            policy().identity.clone(),
            DecisionAuthority::Actor { actor },
        ),
    };
    let open = |ticket| DecisionMutation::Open { ticket };
    settle_at(
        &mut canwu,
        start,
        vec![
            (1, register(SEAT, ids.commander)),
            (2, register(DELEGATE, ids.observer)),
            (3, open(draft(1, maker.clone(), Some(hours(24))))),
            (4, open(draft(2, maker.clone(), None))),
            (5, open(draft(3, maker.clone(), Some(hours(24))))),
        ],
    );

    // Every refusal is a recorded rejection that leaves the ticket alone;
    // the accepted requests later in the same boundary see the version that
    // each accepted one before them advanced.
    settle_at(
        &mut canwu,
        hours(12),
        vec![
            (6, extend(1, 1, hours(24))),
            (7, extend(1, 2, hours(72))),
            (8, extend(2, 1, hours(72))),
            (9, reassign(1, 1, "g81-unknown")),
            (10, reassign(1, 1, SEAT)),
            (11, extend(1, 1, hours(72))),
            (12, reassign(1, 2, DELEGATE)),
            (13, extend(3, 1, hours(48))),
        ],
    );
    for (request_id, code, text) in [
        (6, DecisionAttemptErrorCode::InvalidDecision, "later"),
        (7, DecisionAttemptErrorCode::VersionConflict, "expected 2"),
        (8, DecisionAttemptErrorCode::InvalidDecision, "no deadline"),
        (9, DecisionAttemptErrorCode::InvalidController, "unknown"),
        (10, DecisionAttemptErrorCode::InvalidController, "already"),
    ] {
        let (observed, message) = rejection(&canwu, request_id);
        assert_eq!(observed, code, "request {request_id}: {message}");
        assert!(message.contains(text), "request {request_id}: {message}");
    }
    for request_id in [11, 12, 13] {
        assert_eq!(
            outcome(&canwu, request_id),
            DecisionAttemptOutcome::Accepted {
                trace_id: None,
                command_request_id: None,
            },
            "request {request_id} is journalled like any non-resolving mutation"
        );
    }
    let held = canwu
        .decision_ticket(DecisionTicketId::new(1))
        .expect("held ticket")
        .clone();
    assert_eq!(held.deadline, Some(hours(72)));
    assert_eq!(held.assigned_controller, DELEGATE);
    assert_eq!((held.version, held.updated_at), (3, hours(12)));
    assert!(held.is_open());
    assert_eq!(
        canwu
            .decision_ticket(DecisionTicketId::new(2))
            .map(|ticket| (ticket.version, ticket.deadline)),
        Some((1, None))
    );
    // A strict load rebuilds the open-ticket indexes from the tickets, and
    // comparing the decision states compares the live indexes with them, so
    // the old deadline and controller entries must be gone.
    reload_matches(&canwu, &plugins);

    // Past the old deadline both extended tickets stay open; past its new
    // one ticket 3 expires.
    canwu
        .settle_boundary(BoundaryRequest::at(hours(36)))
        .expect("a boundary past the old deadline");
    assert!(
        canwu
            .decision_ticket(DecisionTicketId::new(3))
            .is_some_and(DecisionTicket::is_open)
    );
    canwu
        .settle_boundary(BoundaryRequest::at(hours(49)))
        .expect("a boundary past the new deadline");
    assert_eq!(
        canwu
            .decision_ticket(DecisionTicketId::new(3))
            .map(|ticket| &ticket.state),
        Some(&DecisionTicketState::Expired)
    );
    assert!(
        canwu
            .decision_ticket(DecisionTicketId::new(1))
            .is_some_and(DecisionTicket::is_open)
    );

    // Only the new controller may resolve: the seat's own resolution is
    // refused, and the delegate's goes through the ordinary path.
    settle_at(
        &mut canwu,
        hours(50),
        vec![(
            14,
            DecisionMutation::Resolve {
                ticket_id: DecisionTicketId::new(1),
                expected_version: 3,
                controller_id: SEAT.to_owned(),
                policy: policy().identity.clone(),
                decision: PolicyDecision::selected("proceed", "the seat answers"),
                command_request_id: None,
            },
        )],
    );
    let (code, message) = rejection(&canwu, 14);
    assert_eq!(code, DecisionAttemptErrorCode::InvalidController);
    assert!(message.contains("not assigned"), "{message}");
    let DecisionEvaluation::Prepared(prepared) = canwu
        .drive_decision(
            canwu.time(),
            0,
            DecisionRequestId::new(15),
            None,
            DecisionTicketId::new(1),
            &policy(),
        )
        .expect("the delegate's decision should prepare")
    else {
        panic!("the utility policy resolves the ticket");
    };
    canwu
        .step_canonical()
        .expect("resolution boundary")
        .expect("resolution receipt");
    let DecisionAttemptOutcome::Accepted {
        trace_id: Some(trace_id),
        ..
    } = outcome(&canwu, prepared.request.request_id.get())
    else {
        panic!("the delegate's resolution should be accepted with a trace");
    };
    let trace = canwu.decision_trace(trace_id).expect("resolution trace");
    assert_eq!(
        (trace.controller_id.as_str(), trace.ticket_version),
        (DELEGATE, 3)
    );
    assert!(matches!(
        canwu
            .decision_ticket(DecisionTicketId::new(1))
            .map(|ticket| &ticket.state),
        Some(DecisionTicketState::Resolved { .. })
    ));

    // Reassignment keeps lineage, the rule a strict load re-checks: a seat
    // successor's ticket, linked to the cancelled ticket of another decision
    // maker by the seat alone, moves only to a controller of the same seat.
    let seat_controller = |id: &str, actor, seat: &str| DecisionMutation::RegisterController {
        controller: DecisionControllerBinding::new(
            id,
            policy().identity.clone(),
            DecisionAuthority::Actor { actor },
        )
        .with_seat(seat, "fixture.seat-profile"),
    };
    let on_controller = |mut ticket: DecisionTicketDraft, controller: &str| {
        controller.clone_into(&mut ticket.assigned_controller);
        ticket
    };
    settle_at(
        &mut canwu,
        hours(51),
        vec![
            (16, seat_controller(HOLDER, ids.commander, "seat.north")),
            (17, seat_controller(SUCCESSOR, ids.observer, "seat.north")),
            (18, seat_controller(DEPUTY, ids.commander, "seat.north")),
            (
                19,
                open(on_controller(draft(4, maker.clone(), None), HOLDER)),
            ),
        ],
    );
    let successor_ticket = DecisionTicketDraft {
        parent_ticket: Some(DecisionTicketId::new(4)),
        ..on_controller(draft(5, EntityRef::Person(ids.observer), None), SUCCESSOR)
    };
    settle_at(
        &mut canwu,
        hours(52),
        vec![
            (
                20,
                DecisionMutation::Cancel {
                    ticket_id: DecisionTicketId::new(4),
                    expected_version: 1,
                    reason: "the holder left the seat".to_owned(),
                },
            ),
            (21, open(successor_ticket)),
        ],
    );
    settle_at(
        &mut canwu,
        hours(53),
        vec![(22, reassign(5, 1, DELEGATE)), (23, reassign(5, 1, DEPUTY))],
    );
    let (code, message) = rejection(&canwu, 22);
    assert_eq!(code, DecisionAttemptErrorCode::InvalidDecision);
    assert!(message.contains("lineage"), "{message}");
    assert_eq!(
        canwu
            .decision_ticket(DecisionTicketId::new(5))
            .map(|ticket| (ticket.assigned_controller.as_str(), ticket.version)),
        Some((DEPUTY, 2))
    );

    // A controller whose authority person is unavailable cannot take a
    // ticket, as it could not open one.
    canwu
        .settle_boundary(BoundaryRequest::at(hours(54)).with_cadence(SystemCadence::Daily))
        .expect("the delegate is captured");
    settle_at(&mut canwu, hours(55), vec![(24, reassign(2, 1, DELEGATE))]);
    assert_eq!(
        rejection(&canwu, 24).0,
        DecisionAttemptErrorCode::IssuerUnavailable
    );
    assert_eq!(
        canwu
            .decision_ticket(DecisionTicketId::new(2))
            .map(|ticket| (ticket.assigned_controller.as_str(), ticket.version)),
        Some((SEAT, 1))
    );

    // The moved deadline and controller are persisted state: they reload,
    // and the decision ingress replays to the same state.
    reload_matches(&canwu, &plugins);
    let replayed = Canwu::replay_from_journal(&plugins, &canwu.replay_journal())
        .expect("the extended and reassigned tickets should replay");
    assert_eq!(replayed.snapshot(), canwu.snapshot());
}

/// A strict snapshot reload validates the decision state against its
/// admitted ingress and gives back the same snapshot.
fn reload_matches(canwu: &Canwu, plugins: &[&dyn SimulationPlugin]) {
    let snapshot = canwu.snapshot();
    let restored = Canwu::from_snapshot_json_with_plugins(
        &serde_json::to_string(&snapshot).expect("snapshot json"),
        plugins,
    )
    .expect("the extended and reassigned tickets should reload");
    assert_eq!(restored.snapshot(), snapshot);
}
