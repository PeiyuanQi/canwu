//! Gap G-02: person availability is core state with admission consequences.

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundaryRequest,
    BoundarySystemContract, CONTROLLER_AUTHORITY_UNAVAILABLE_REASON, Canwu, CanwuError, Command,
    CommandAttemptOutcome, CommandEnvelope, CommandRequest, CommandRequestId, CustodyState,
    DECISION_MAKER_UNAVAILABLE_REASON, DecisionAttemptErrorCode, DecisionAttemptOutcome,
    DecisionAuthority, DecisionContext, DecisionControllerBinding, DecisionIngressRequest,
    DecisionMutation, DecisionOption, DecisionPolicyIdentity, DecisionPolicyKind,
    DecisionRequestId, DecisionTicketDraft, DecisionTicketId, DecisionTicketState, EntityRef,
    ErrorCode, Issuer, LifeState, PersonAvailability, PersonId, PluginRegistrar, SimDuration,
    SimTime, SimulationPlugin, SimulationView, StateKey, StateVisibility, SystemCadence,
    UtilityProfile, WeightedUtilityPolicy,
};
use serde_json::json;
use std::collections::BTreeMap;

fn kill_commander(
    view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let commander = Canwu::demo_ids().commander;
    if view.person_availability(commander)?.is_some() {
        return Ok(BoundaryProposal::default());
    }
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::SetPersonAvailability {
            person: commander,
            availability: PersonAvailability::new(LifeState::Dead, CustodyState::Free, view.time()),
            summary: "The commander died of fever".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

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
            summary: "Raiders captured the observer".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

fn availability_writer(name: &str) -> BoundarySystemContract {
    let mut contract = BoundarySystemContract::new(
        name,
        BoundaryPhase::DomainDeltaProposal,
        SystemCadence::Daily,
    );
    contract.reads = vec![StateKey::core_person_availability()];
    contract.writes = vec![StateKey::core_person_availability()];
    contract.visibility = StateVisibility::SameBoundary;
    contract
}

struct FatePlugin;

impl SimulationPlugin for FatePlugin {
    fn name(&self) -> &'static str {
        "fixture-fate"
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000000201"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_boundary_system(availability_writer("fate"), kill_commander)
    }
}

/// Captures the observer, who acts as a controller's authority person.
struct CapturePlugin;

impl SimulationPlugin for CapturePlugin {
    fn name(&self) -> &'static str {
        "fixture-capture"
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000000204"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_boundary_system(availability_writer("capture"), capture_observer)
    }
}

/// A second declared writer of the same person in the same boundary.
struct RivalPlugin;

impl SimulationPlugin for RivalPlugin {
    fn name(&self) -> &'static str {
        "fixture-rival"
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000000202"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_boundary_system(availability_writer("assassin"), kill_commander)
    }
}

/// Proposes the same directive without declaring the core write.
struct UndeclaredPlugin;

impl SimulationPlugin for UndeclaredPlugin {
    fn name(&self) -> &'static str {
        "fixture-undeclared"
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000000203"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut contract = BoundarySystemContract::new(
            "rumor",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        contract.reads = vec![StateKey::core_person_availability()];
        registrar.register_boundary_system(contract, kill_commander)
    }
}

fn ticket_for_commander(id: u64) -> DecisionTicketDraft {
    DecisionTicketDraft {
        id: DecisionTicketId::new(id),
        definition: "fixture.garrison-order".to_owned(),
        decision_maker: EntityRef::Person(Canwu::demo_ids().commander),
        assigned_controller: "court".to_owned(),
        summary: "The commander must choose a garrison posture".to_owned(),
        context: DecisionContext::new("fixture.garrison.v1", json!({})),
        options: vec![DecisionOption::new("hold", "Hold the line")],
        deadline: None,
        parent_ticket: None,
    }
}

fn register(id: &str, actor: PersonId) -> DecisionMutation {
    DecisionMutation::RegisterController {
        controller: DecisionControllerBinding::new(
            id,
            DecisionPolicyIdentity::new(DecisionPolicyKind::Utility, "court-utility", "1"),
            DecisionAuthority::Actor { actor },
        ),
    }
}

const fn open(ticket: DecisionTicketDraft) -> DecisionMutation {
    DecisionMutation::Open { ticket }
}

/// Enqueues decision requests at the current time and revision, then admits
/// them in one canonical boundary.
fn admit_decisions(canwu: &mut Canwu, requests: Vec<(u64, DecisionMutation)>) {
    let (now, revision) = (canwu.time(), canwu.revision());
    for (request, mutation) in requests {
        canwu
            .enqueue_decision(
                now,
                0,
                DecisionIngressRequest::new(DecisionRequestId::new(request), revision, mutation),
            )
            .expect("decision ingress");
    }
    canwu
        .step_canonical()
        .expect("decision intake")
        .expect("boundary");
}

fn daily(days: i64) -> BoundaryRequest {
    BoundaryRequest::at(SimTime::EPOCH + SimDuration::days(days)).with_cadence(SystemCadence::Daily)
}

#[test]
#[allow(clippy::too_many_lines)]
fn gap_g02_sim_person_availability() {
    let ids = Canwu::demo_ids();
    let fate = FatePlugin;
    let mut canwu = Canwu::demo(1212).expect("demo");
    canwu.register_plugin(&fate).expect("fate plugin");
    let start = canwu.time();
    assert_eq!(start, SimTime::EPOCH);

    // An open ticket whose decision maker is the commander.
    let controller = DecisionControllerBinding::new(
        "court",
        DecisionPolicyIdentity::new(DecisionPolicyKind::Utility, "court-utility", "1"),
        DecisionAuthority::Actor {
            actor: ids.observer,
        },
    );
    let revision = canwu.revision();
    canwu
        .enqueue_decision(
            start,
            0,
            DecisionIngressRequest::new(
                DecisionRequestId::new(1),
                revision,
                DecisionMutation::RegisterController { controller },
            ),
        )
        .expect("controller ingress");
    canwu
        .enqueue_decision(
            start,
            0,
            DecisionIngressRequest::new(
                DecisionRequestId::new(2),
                revision,
                DecisionMutation::Open {
                    ticket: ticket_for_commander(1),
                },
            ),
        )
        .expect("ticket ingress");
    canwu.step_canonical().expect("intake").expect("boundary");
    assert!(
        canwu
            .decision_ticket(DecisionTicketId::new(1))
            .expect("ticket")
            .is_open()
    );

    // A declared phase-7 writer makes the commander dead; the open ticket is
    // cancelled in the same boundary and the boundary records the trace.
    canwu
        .settle_boundary(daily(1))
        .expect("availability boundary");
    let change = canwu
        .boundaries()
        .last()
        .and_then(|boundary| boundary.person_availability_changes.first())
        .cloned()
        .expect("availability evidence");
    assert_eq!(
        (
            change.plugin.as_str(),
            change.system.as_str(),
            change.person
        ),
        ("fixture-fate", "fate", ids.commander)
    );
    assert_eq!(change.previous, None);
    assert_eq!(change.cancelled_tickets, vec![DecisionTicketId::new(1)]);
    assert_eq!(
        canwu
            .person_availability(ids.commander)
            .map(|value| value.life),
        Some(LifeState::Dead)
    );
    let ticket = canwu
        .decision_ticket(DecisionTicketId::new(1))
        .expect("ticket");
    assert_eq!(
        ticket.state,
        DecisionTicketState::Cancelled {
            reason: DECISION_MAKER_UNAVAILABLE_REASON.to_owned()
        }
    );
    let policy = WeightedUtilityPolicy::new(
        "court-utility",
        "1",
        UtilityProfile {
            weights: BTreeMap::new(),
        },
    );
    assert_eq!(
        canwu
            .prepare_decision(
                DecisionRequestId::new(9),
                None,
                DecisionTicketId::new(1),
                &policy
            )
            .expect_err("dead decision maker")
            .code,
        ErrorCode::DecisionMakerUnavailable
    );

    // Decision ingress cannot open a new ticket for the dead commander.
    let now = canwu.time();
    canwu
        .enqueue_decision(
            now,
            0,
            DecisionIngressRequest::new(
                DecisionRequestId::new(3),
                canwu.revision(),
                DecisionMutation::Open {
                    ticket: ticket_for_commander(2),
                },
            ),
        )
        .expect("refused ticket is still admissible ingress");
    canwu.step_canonical().expect("decision").expect("boundary");
    assert!(matches!(
        canwu
            .decision_attempt(DecisionRequestId::new(3))
            .map(|attempt| &attempt.outcome),
        Some(DecisionAttemptOutcome::Rejected {
            code: DecisionAttemptErrorCode::DecisionMakerUnavailable,
            ..
        })
    ));
    assert!(canwu.decision_ticket(DecisionTicketId::new(2)).is_none());

    // A command issued by the dead commander is rejected with a stable code
    // and the rejection is persisted as a command attempt.
    canwu
        .enqueue_command(
            now,
            0,
            CommandRequest::new(
                CommandRequestId::new(1),
                canwu.revision(),
                CommandEnvelope::new(
                    Issuer::Actor(ids.commander),
                    Command::OrderMovement {
                        subject: EntityRef::Army(ids.army),
                        destination: ids.eastern_territory,
                        cargo: Vec::new(),
                    },
                )
                .at_time(now),
            ),
        )
        .expect("command ingress");
    canwu.step_canonical().expect("command").expect("boundary");
    let attempt = canwu.command_attempts().last().expect("command attempt");
    let CommandAttemptOutcome::Rejected { error } = &attempt.outcome else {
        panic!("a dead issuer's command must be rejected");
    };
    assert_eq!(error.code, ErrorCode::IssuerUnavailable);
    assert!(canwu.commands().is_empty());

    // Save/load and exact replay reproduce the availability, the cancelled
    // ticket, and both admission rejections.
    let snapshot = canwu.snapshot();
    let json = serde_json::to_string(&snapshot).expect("snapshot json");
    let restored = Canwu::from_snapshot_json_with_plugins(&json, &[&fate]).expect("restore");
    assert_eq!(restored.snapshot(), snapshot);
    assert_eq!(restored.checkpoint_hash(), canwu.checkpoint_hash());
    let replayed =
        Canwu::replay_from_journal(&[&fate], &canwu.replay_journal()).expect("exact replay");
    assert_eq!(replayed.snapshot(), snapshot);
    assert_eq!(
        replayed.person_availability(ids.commander),
        canwu.person_availability(ids.commander)
    );

    // The availability map is committed state: dropping it fails closed.
    let mut tampered = snapshot;
    tampered.person_availability.clear();
    assert!(
        Canwu::from_snapshot_json(&serde_json::to_string(&tampered).expect("tampered json"))
            .is_err()
    );

    // An undeclared writer fails the whole boundary.
    let mut undeclared = Canwu::demo(1212).expect("demo");
    undeclared
        .register_plugin(&UndeclaredPlugin)
        .expect("undeclared plugin");
    let revision = undeclared.revision();
    let error = undeclared
        .settle_boundary(daily(1))
        .expect_err("undeclared availability writer");
    assert_eq!(error.code, ErrorCode::UndeclaredStateWrite);
    assert!(undeclared.boundaries().is_empty());
    assert_eq!(undeclared.revision(), revision);
    assert!(undeclared.person_availability(ids.commander).is_none());

    // Two declared writers of the same person in one boundary fail closed.
    let mut contested = Canwu::demo(1212).expect("demo");
    contested.register_plugin(&FatePlugin).expect("fate plugin");
    contested
        .register_plugin(&RivalPlugin)
        .expect("rival plugin");
    let error = contested
        .settle_boundary(daily(1))
        .expect_err("duplicate availability writer");
    assert_eq!(error.code, ErrorCode::DuplicateBoundaryWriter);
    assert!(contested.boundaries().is_empty());
    assert!(contested.person_availability(ids.commander).is_none());

    controller_authority_capture_closes_tickets_and_allows_a_successor();
}

/// The decision maker stays available while the assigned controller's
/// authority person is captured. Without the sweep the ticket could never be
/// resolved: there is no reassignment mutation and every resolution through
/// the captured controller is refused.
fn controller_authority_capture_closes_tickets_and_allows_a_successor() {
    let ids = Canwu::demo_ids();
    let capture = CapturePlugin;
    let mut canwu = Canwu::demo(1212).expect("demo");
    canwu.register_plugin(&capture).expect("capture plugin");
    let observer_ticket = DecisionTicketDraft {
        decision_maker: EntityRef::Person(ids.observer),
        ..ticket_for_commander(2)
    };
    admit_decisions(
        &mut canwu,
        vec![
            (11, register("court", ids.observer)),
            (12, register("regent", ids.commander)),
            (13, open(ticket_for_commander(1))),
            (14, open(observer_ticket)),
        ],
    );

    // Capturing the court's authority person closes the commander's ticket
    // for its controller; the observer's own ticket keeps the decision-maker
    // reason although it qualifies for both.
    canwu.settle_boundary(daily(1)).expect("capture boundary");
    let change = canwu
        .boundaries()
        .last()
        .and_then(|boundary| boundary.person_availability_changes.first())
        .cloned()
        .expect("availability evidence");
    assert_eq!(change.person, ids.observer);
    assert_eq!(change.cancelled_tickets, vec![DecisionTicketId::new(2)]);
    assert_eq!(
        change.cancelled_controller_tickets,
        vec![DecisionTicketId::new(1)]
    );
    let state = |id: u64| {
        canwu
            .decision_ticket(DecisionTicketId::new(id))
            .map(|ticket| ticket.state.clone())
    };
    assert_eq!(
        state(1),
        Some(DecisionTicketState::Cancelled {
            reason: CONTROLLER_AUTHORITY_UNAVAILABLE_REASON.to_owned()
        })
    );
    assert_eq!(
        state(2),
        Some(DecisionTicketState::Cancelled {
            reason: DECISION_MAKER_UNAVAILABLE_REASON.to_owned()
        })
    );

    // The captured controller cannot open a ticket, while a successor for an
    // available controller records the cancelled ticket as its parent.
    let successor = DecisionTicketDraft {
        assigned_controller: "regent".to_owned(),
        parent_ticket: Some(DecisionTicketId::new(1)),
        ..ticket_for_commander(4)
    };
    admit_decisions(
        &mut canwu,
        vec![(15, open(ticket_for_commander(3))), (16, open(successor))],
    );
    assert!(matches!(
        canwu
            .decision_attempt(DecisionRequestId::new(15))
            .map(|attempt| &attempt.outcome),
        Some(DecisionAttemptOutcome::Rejected {
            code: DecisionAttemptErrorCode::IssuerUnavailable,
            ..
        })
    ));
    assert!(canwu.decision_ticket(DecisionTicketId::new(3)).is_none());
    let successor = canwu
        .decision_ticket(DecisionTicketId::new(4))
        .expect("successor ticket");
    assert!(successor.is_open());
    assert_eq!(successor.parent_ticket, Some(DecisionTicketId::new(1)));

    // Save/load and exact replay re-derive both cancellation reasons.
    let snapshot = canwu.snapshot();
    let json = serde_json::to_string(&snapshot).expect("snapshot json");
    let restored = Canwu::from_snapshot_json_with_plugins(&json, &[&capture]).expect("restore");
    assert_eq!(restored.snapshot(), snapshot);
    assert_eq!(restored.checkpoint_hash(), canwu.checkpoint_hash());
    let replayed =
        Canwu::replay_from_journal(&[&capture], &canwu.replay_journal()).expect("exact replay");
    assert_eq!(replayed.snapshot(), snapshot);
}
