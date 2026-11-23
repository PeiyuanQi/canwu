//! Gap G-98: a decision ticket that a person in custody decides.
//!
//! A captor's question to his captive is an engine ticket marked at `Open`
//! with the custody it is addressed in. The mark exempts it from every
//! availability gate while that custody holds, and the end-of-boundary sweep
//! cancels it with its own reason when the captive dies, is freed, or passes
//! to another custodian. An ordinary ticket for the same person keeps the
//! G-02 rules.

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundaryRequest,
    BoundarySystemContract, BoundarySystemHandler, CUSTODY_ENDED_REASON,
    CUSTODY_TRANSFERRED_REASON, Canwu, CanwuError, CompactedCanwu, ContinuationCheckpoint,
    CustodyState, DECISION_MAKER_DEAD_REASON, DECISION_MAKER_UNAVAILABLE_REASON,
    DecisionAttemptErrorCode, DecisionAttemptOutcome, DecisionAuthority, DecisionContext,
    DecisionControllerBinding, DecisionCustodyAddress, DecisionEvaluation, DecisionIngressRequest,
    DecisionMutation, DecisionOption, DecisionPolicyIdentity, DecisionPolicyKind,
    DecisionRequestId, DecisionTicketDraft, DecisionTicketId, DecisionTicketState, EntityRef,
    ErrorCode, LifeState, PersonAvailability, PersonId, PluginRegistrar, SimDuration, SimTime,
    SimulationPlugin, SimulationView, StateKey, StateVisibility, SystemCadence, UtilityProfile,
    WeightedUtilityPolicy,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const POLICY: &str = "court-utility";
const CAPTIVE: &str = "captive";
const COURT: &str = "court";
/// The custody-addressed ticket kept open from the second boundary on.
const SUMMONS: u64 = 4;

fn captor() -> EntityRef {
    EntityRef::Person(Canwu::demo_ids().commander)
}

fn other_captor() -> EntityRef {
    EntityRef::Army(Canwu::demo_ids().army)
}

fn held(custody: CustodyState, by: EntityRef, at: SimTime) -> PersonAvailability {
    PersonAvailability::new(LifeState::Alive, custody, at).with_custodian(by)
}

/// Sets the observer's availability for the day of the boundary: captured by
/// the commander on day 1, detained by him on day 2 (the same custody), and
/// `day3` on day 3. A value equal to the committed one is not written again.
fn script(
    view: &SimulationView<'_>,
    day3: impl Fn(SimTime) -> PersonAvailability,
) -> Result<BoundaryProposal, CanwuError> {
    let observer = Canwu::demo_ids().observer;
    let at = view.time();
    let target = match at.day() {
        1 => held(CustodyState::Captive, captor(), at),
        2 => held(CustodyState::Detained, captor(), at),
        3 => day3(at),
        _ => return Ok(BoundaryProposal::default()),
    };
    let current = view.person_availability(observer)?;
    if current.is_some_and(|value| {
        (value.life, value.custody, &value.custodian)
            == (target.life, target.custody, &target.custodian)
    }) {
        return Ok(BoundaryProposal::default());
    }
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::SetPersonAvailability {
            person: observer,
            availability: target,
            summary: "The observer's custody changed".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

fn dies(view: &SimulationView<'_>, _: &BoundaryContext) -> Result<BoundaryProposal, CanwuError> {
    script(view, |at| {
        PersonAvailability::new(LifeState::Dead, CustodyState::Free, at)
    })
}

fn freed(view: &SimulationView<'_>, _: &BoundaryContext) -> Result<BoundaryProposal, CanwuError> {
    script(view, |at| {
        PersonAvailability::new(LifeState::Alive, CustodyState::Free, at)
    })
}

fn transferred(
    view: &SimulationView<'_>,
    _: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    script(view, |at| held(CustodyState::Captive, other_captor(), at))
}

/// Holds the observer's day-3 fate.
struct Custody {
    name: &'static str,
    hash: &'static str,
    handler: BoundarySystemHandler,
}

impl SimulationPlugin for Custody {
    fn name(&self) -> &'static str {
        self.name
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        self.hash
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut contract = BoundarySystemContract::new(
            "custody",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        contract.reads = vec![StateKey::core_person_availability()];
        contract.writes = vec![StateKey::core_person_availability()];
        contract.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(contract, self.handler)
    }
}

const DIES: Custody = Custody {
    name: "fixture-custody-dies",
    hash: "0000000000000000000000000000000000000000000000000000000000000981",
    handler: dies,
};
const FREED: Custody = Custody {
    name: "fixture-custody-freed",
    hash: "0000000000000000000000000000000000000000000000000000000000000982",
    handler: freed,
};
const TRANSFERRED: Custody = Custody {
    name: "fixture-custody-transferred",
    hash: "0000000000000000000000000000000000000000000000000000000000000983",
    handler: transferred,
};

fn policy() -> WeightedUtilityPolicy {
    WeightedUtilityPolicy::new(
        POLICY,
        "1",
        UtilityProfile {
            weights: BTreeMap::new(),
        },
    )
}

fn register(id: &str, actor: PersonId) -> DecisionMutation {
    DecisionMutation::RegisterController {
        controller: DecisionControllerBinding::new(
            id,
            DecisionPolicyIdentity::new(DecisionPolicyKind::Utility, POLICY, "1"),
            DecisionAuthority::Actor { actor },
        ),
    }
}

/// A question to the observer answered through `controller`, addressed in
/// the custody of `custodian` when one is given.
fn question(id: u64, controller: &str, custodian: Option<EntityRef>) -> DecisionMutation {
    DecisionMutation::Open {
        ticket: DecisionTicketDraft {
            id: DecisionTicketId::new(id),
            definition: "fixture.summons".to_owned(),
            decision_maker: EntityRef::Person(Canwu::demo_ids().observer),
            assigned_controller: controller.to_owned(),
            summary: "The captor summons the observer to submit".to_owned(),
            context: DecisionContext::new("fixture.summons.v1", json!({})),
            options: vec![
                DecisionOption::new("refuse", "Refuse"),
                DecisionOption::new("submit", "Submit"),
            ],
            deadline: None,
            parent_ticket: None,
            custody_addressed: custodian.map(DecisionCustodyAddress::new),
        },
    }
}

/// Enqueues decision requests at the current time and revision, then admits
/// them in one canonical boundary.
fn admit(canwu: &mut Canwu, requests: Vec<(u64, DecisionMutation)>) {
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

fn rejection(canwu: &Canwu, request: u64) -> Option<DecisionAttemptErrorCode> {
    match canwu
        .decision_attempt(DecisionRequestId::new(request))
        .map(|attempt| &attempt.outcome)
    {
        Some(DecisionAttemptOutcome::Rejected { code, .. }) => Some(*code),
        _ => None,
    }
}

fn state(canwu: &Canwu, id: u64) -> Option<DecisionTicketState> {
    canwu
        .decision_ticket(DecisionTicketId::new(id))
        .map(|ticket| ticket.state.clone())
}

fn cancelled(reason: &str) -> DecisionTicketState {
    DecisionTicketState::Cancelled {
        reason: reason.to_owned(),
    }
}

/// Strict reload and exact replay re-derive every admission and sweep.
fn assert_reloads(canwu: &Canwu, plugin: &Custody) {
    let snapshot = canwu.snapshot();
    let json = serde_json::to_string(&snapshot).expect("snapshot json");
    let restored = Canwu::from_snapshot_json_with_plugins(&json, &[plugin]).expect("restore");
    assert_eq!(restored.snapshot(), snapshot);
    assert_eq!(restored.checkpoint_hash(), canwu.checkpoint_hash());
    let replayed =
        Canwu::replay_from_journal(&[plugin], &canwu.replay_journal()).expect("exact replay");
    assert_eq!(replayed.snapshot(), snapshot);
}

/// Replaces the custodian of every persisted custody address.
fn retarget_custody(value: &mut Value, custodian: &Value) -> usize {
    match value {
        Value::Object(map) => {
            let mut count = 0;
            for (key, entry) in map.iter_mut() {
                if key == "custody_addressed" {
                    entry["custodian"] = custodian.clone();
                    count += 1;
                } else {
                    count += retarget_custody(entry, custodian);
                }
            }
            count
        }
        Value::Array(items) => items
            .iter_mut()
            .map(|item| retarget_custody(item, custodian))
            .sum(),
        _ => 0,
    }
}

/// Captures the observer, opens and answers custody-addressed tickets, and
/// keeps ticket [`SUMMONS`] open through a second change of the same
/// custody. Cases (a) and (b) are checked on the way.
fn captured(plugin: &Custody) -> Canwu {
    let ids = Canwu::demo_ids();
    let mut canwu = Canwu::demo(9898).expect("demo");
    canwu.register_plugin(plugin).expect("custody plugin");
    // The captive answers through his own controller; the court acts for the
    // commander. Ticket 9 is an ordinary question put while he is free.
    admit(
        &mut canwu,
        vec![
            (1, register(CAPTIVE, ids.observer)),
            (2, register(COURT, ids.commander)),
            (3, question(9, COURT, None)),
        ],
    );

    // Day 1: the commander takes the observer captive. The ordinary ticket is
    // cancelled as before.
    canwu.settle_boundary(daily(1)).expect("capture boundary");
    assert_eq!(
        state(&canwu, 9),
        Some(cancelled(DECISION_MAKER_UNAVAILABLE_REASON))
    );

    // (a) A custody-addressed ticket opens for the captive, through the
    // captive's own controller. (b) An ordinary ticket for him is refused, and
    // a custody address that names someone who does not hold him is invalid,
    // as is one that offers a command the captive could never issue.
    let mut commanding = question(7, CAPTIVE, Some(captor()));
    if let DecisionMutation::Open { ticket } = &mut commanding {
        ticket.options[1] = DecisionOption::new("submit", "Submit")
            .with_command(json!({ "fixture": "surrender the city" }));
    }
    admit(
        &mut canwu,
        vec![
            (4, question(1, CAPTIVE, Some(captor()))),
            (5, question(2, COURT, None)),
            (6, question(3, CAPTIVE, Some(other_captor()))),
            (7, question(SUMMONS, CAPTIVE, Some(captor()))),
            (9, commanding),
        ],
    );
    assert_eq!(state(&canwu, 1), Some(DecisionTicketState::Open));
    assert_eq!(
        canwu
            .decision_ticket(DecisionTicketId::new(1))
            .and_then(|ticket| ticket.custody_addressed.clone()),
        Some(DecisionCustodyAddress::new(captor()))
    );
    assert_eq!(
        rejection(&canwu, 5),
        Some(DecisionAttemptErrorCode::DecisionMakerUnavailable)
    );
    assert!(canwu.decision_ticket(DecisionTicketId::new(2)).is_none());
    assert_eq!(
        rejection(&canwu, 6),
        Some(DecisionAttemptErrorCode::InvalidDecision)
    );
    assert!(canwu.decision_ticket(DecisionTicketId::new(3)).is_none());
    assert_eq!(
        rejection(&canwu, 9),
        Some(DecisionAttemptErrorCode::InvalidDecision)
    );
    assert!(canwu.decision_ticket(DecisionTicketId::new(7)).is_none());

    // (a) His controller's answer prepares and is admitted.
    let DecisionEvaluation::Prepared(prepared) = canwu
        .drive_decision(
            canwu.time(),
            0,
            DecisionRequestId::new(8),
            None,
            DecisionTicketId::new(1),
            &policy(),
        )
        .expect("the captive's answer should prepare")
    else {
        panic!("the utility policy answers the summons");
    };
    canwu
        .step_canonical()
        .expect("answer boundary")
        .expect("answer receipt");
    assert!(matches!(
        canwu
            .decision_attempt(prepared.request.request_id)
            .map(|attempt| &attempt.outcome),
        Some(DecisionAttemptOutcome::Accepted { .. })
    ));
    assert!(matches!(
        state(&canwu, 1),
        Some(DecisionTicketState::Resolved { .. })
    ));

    // Day 2: he stays in the commander's hands, now detained. Neither sweep
    // pass touches the open summons, although he is still unavailable and the
    // authority person of its controller.
    canwu.settle_boundary(daily(2)).expect("detention boundary");
    let change = canwu
        .boundaries()
        .last()
        .and_then(|boundary| boundary.person_availability_changes.first())
        .cloned()
        .expect("detention evidence");
    assert_eq!(change.availability.custody, CustodyState::Detained);
    assert!(change.cancelled_tickets.is_empty());
    assert!(change.cancelled_controller_tickets.is_empty());
    assert!(change.cancelled_custody_tickets.is_empty());
    assert_eq!(state(&canwu, SUMMONS), Some(DecisionTicketState::Open));
    canwu
}

/// Settles day 3 and returns the change and the summons's final state.
fn end_custody(canwu: &mut Canwu) -> (Vec<DecisionTicketId>, Option<DecisionTicketState>) {
    canwu.settle_boundary(daily(3)).expect("custody boundary");
    let change = canwu
        .boundaries()
        .last()
        .and_then(|boundary| boundary.person_availability_changes.first())
        .cloned()
        .expect("custody evidence");
    assert!(change.cancelled_tickets.is_empty());
    assert!(change.cancelled_controller_tickets.is_empty());
    (change.cancelled_custody_tickets, state(canwu, SUMMONS))
}

#[test]
fn custody_addressed_ticket_opens_answers_and_restores() {
    let mut canwu = captured(&FREED);

    // (d) A continuation saved with the summons open restores it with the
    // same state hash, mark included, and continues alike.
    let (checkpoint, _) = canwu
        .continuation_checkpoint()
        .expect("continuation checkpoint");
    let mut continued =
        CompactedCanwu::from_continuation_checkpoint_with_plugins(checkpoint, &[&FREED])
            .expect("the custody run should continue");
    assert_eq!(
        continued
            .authoritative_state_hash()
            .expect("continued hash"),
        canwu.authoritative_state_hash().expect("hash")
    );
    assert_eq!(
        continued
            .decision_ticket(DecisionTicketId::new(SUMMONS))
            .and_then(|ticket| ticket.custody_addressed.clone()),
        Some(DecisionCustodyAddress::new(captor()))
    );
    assert_reloads(&canwu, &FREED);

    // The mark is in the state hash: a continuation whose decision state
    // alone names another custodian, its continuation hash recomputed, does
    // not reproduce the checkpoint hash and is refused.
    let (checkpoint, _) = canwu
        .continuation_checkpoint()
        .expect("continuation checkpoint");
    let mut value = serde_json::to_value(&checkpoint).expect("checkpoint value");
    let other = serde_json::to_value(other_captor()).expect("custodian value");
    assert!(retarget_custody(&mut value["state"]["decisions"], &other) > 0);
    let mut tampered: ContinuationCheckpoint =
        serde_json::from_value(value).expect("the tampered checkpoint decodes");
    tampered.continuation_hash = tampered
        .compute_continuation_hash()
        .expect("continuation hash");
    let Err(error) = CompactedCanwu::from_continuation_checkpoint_with_plugins(tampered, &[&FREED])
    else {
        panic!("a retargeted custody address must not restore");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot, "{}", error.message);

    // (c2) Freed, the summons is cancelled with its own reason, in the
    // original and the continued run alike.
    let (tickets, summons) = end_custody(&mut canwu);
    assert_eq!(tickets, vec![DecisionTicketId::new(SUMMONS)]);
    assert_eq!(summons, Some(cancelled(CUSTODY_ENDED_REASON)));
    continued
        .settle_boundary(daily(3))
        .expect("the continued run settles alike");
    assert_eq!(
        continued
            .authoritative_state_hash()
            .expect("continued hash"),
        canwu.authoritative_state_hash().expect("hash")
    );
    assert_reloads(&canwu, &FREED);
}

#[test]
fn custody_addressed_ticket_ends_on_death() {
    // (c1) Death cancels it with a reason distinct from the G-02 one.
    let mut canwu = captured(&DIES);
    let (tickets, summons) = end_custody(&mut canwu);
    assert_eq!(tickets, vec![DecisionTicketId::new(SUMMONS)]);
    assert_eq!(summons, Some(cancelled(DECISION_MAKER_DEAD_REASON)));
    assert_reloads(&canwu, &DIES);
}

#[test]
fn custody_addressed_ticket_ends_on_transfer() {
    // (c2) Passing to another custodian cancels it, and the new custodian
    // opens his own, while the old address is now refused.
    let mut canwu = captured(&TRANSFERRED);
    let (tickets, summons) = end_custody(&mut canwu);
    assert_eq!(tickets, vec![DecisionTicketId::new(SUMMONS)]);
    assert_eq!(summons, Some(cancelled(CUSTODY_TRANSFERRED_REASON)));
    admit(
        &mut canwu,
        vec![
            (20, question(5, CAPTIVE, Some(other_captor()))),
            (21, question(6, CAPTIVE, Some(captor()))),
        ],
    );
    assert_eq!(state(&canwu, 5), Some(DecisionTicketState::Open));
    assert_eq!(
        rejection(&canwu, 21),
        Some(DecisionAttemptErrorCode::InvalidDecision)
    );
    assert_reloads(&canwu, &TRANSFERRED);
}
