//! Seat rebinding: the run's seat moves to another existing person through
//! canonical ingress, its outcome is boundary evidence, and saves, exact
//! replay, and continuation restores keep the binding in force.

#![allow(clippy::too_many_lines, clippy::unnecessary_wraps)]

use canwu_api::{
    ArtifactManifest, BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal,
    BoundaryReceipt, BoundaryRequest, BoundarySystemContract, Canwu, CanwuError, Command,
    CommandAttemptOutcome, CommandAuthority, CommandContext, CommandEnvelope, CommandRequest,
    CommandRequestId, CompactedCanwu, CustodyState, DecisionAuthority, DecisionContext,
    DecisionControllerBinding, DecisionIngressRequest, DecisionMutation, DecisionOption,
    DecisionOrigin, DecisionPolicyIdentity, DecisionPolicyKind, DecisionRequestId,
    DecisionTicketDraft, DecisionTicketId, DecisionTicketState, EntityRef, ErrorCode, Issuer,
    LifeState, PayloadSchema, PersonAvailability, PersonId, PluginActionDescriptor,
    PluginRegistrar, RunConfiguration, RunManifest, SEAT_REBOUND_REASON, Scenario, SeatBinding,
    SeatRebindingOutcome, SeatRebindingRequest, SimDuration, SimTime, SimulationPlugin,
    SimulationView, StateKey, StateVisibility, SystemCadence, SystemDirective,
};
use serde_json::{Value, json};

const PLUGIN: &str = "fixture-seat";
const SEAT: &str = "seat.realm";
const PROFILE: &str = "profile.ruler";
const HOLDER_CONTROLLER: &str = "controller.holder";
const HEIR_CONTROLLER: &str = "controller.heir";

const fn holder() -> PersonId {
    PersonId::new(1)
}

const fn heir() -> PersonId {
    PersonId::new(2)
}

/// Dies at the first daily boundary.
const fn cadet() -> PersonId {
    PersonId::new(3)
}

/// Never part of the run.
const fn stranger() -> PersonId {
    PersonId::new(99)
}

fn decree(
    _view: &SimulationView<'_>,
    _context: &CommandContext,
    _payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    Ok(Vec::new())
}

fn fate(
    view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    if view.person_availability(cadet())?.is_some() {
        return Ok(BoundaryProposal::default());
    }
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::SetPersonAvailability {
            person: cadet(),
            availability: PersonAvailability::new(LifeState::Dead, CustodyState::Free, view.time()),
            summary: "The cadet died".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

/// Fails every boundary it runs in; the run settles monthly boundaries only
/// to show a failed boundary's rollback.
fn collapse(
    _view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    Err(CanwuError::new(
        ErrorCode::InvalidPayload,
        "the monthly reckoning fails",
    ))
}

struct SeatFixture;

impl SimulationPlugin for SeatFixture {
    fn name(&self) -> &'static str {
        PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000005ea7"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_command(
            PluginActionDescriptor {
                name: "decree".to_owned(),
                description: "A decree issued from the seat".to_owned(),
                payload_schema: PayloadSchema::Any,
                reads: Vec::new(),
                writes: Vec::new(),
            },
            decree,
        )?;
        let mut contract = BoundarySystemContract::new(
            "fate",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        contract.reads = vec![StateKey::core_person_availability()];
        contract.writes = vec![StateKey::core_person_availability()];
        contract.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(contract, fate)?;
        registrar.register_boundary_system(
            BoundarySystemContract::new(
                "collapse",
                BoundaryPhase::DomainDeltaProposal,
                SystemCadence::Monthly,
            ),
            collapse,
        )
    }
}

fn new_run(plugin: &SeatFixture) -> Canwu {
    let scenario = Scenario::new(
        SimTime::EPOCH,
        vec![
            EntityRef::Person(holder()),
            EntityRef::Person(heir()),
            EntityRef::Person(cadet()),
        ],
    );
    let configuration =
        RunConfiguration::play_as_character(SEAT, HOLDER_CONTROLLER, holder(), PROFILE);
    let manifest = RunManifest::declared(
        ArtifactManifest::for_scenario("fixture", "realm", "1", &scenario).expect("scenario"),
        ArtifactManifest::for_run_configuration("fixture", "realm-seat", "1", &configuration)
            .expect("configuration"),
    );
    Canwu::new_with_run_configuration_and_plugins(97, scenario, manifest, configuration, &[plugin])
        .expect("declared run")
}

fn binding(controller: &str, actor: PersonId) -> SeatBinding {
    SeatBinding {
        seat_id: SEAT.to_owned(),
        controller_id: controller.to_owned(),
        actor: Some(actor),
        institution: None,
        permission_profile_id: PROFILE.to_owned(),
    }
}

/// A human decree through `controller` for `actor`, at the current instant.
fn decree_request(
    request: u64,
    revision: u64,
    at: SimTime,
    controller: &str,
    actor: PersonId,
) -> CommandRequest {
    CommandRequest::new(
        CommandRequestId::new(request),
        revision,
        CommandEnvelope::new(
            Issuer::Human(controller.to_owned()),
            Command::Plugin {
                plugin: PLUGIN.to_owned(),
                command: "decree".to_owned(),
                payload: json!({}),
            },
        )
        .with_authority(CommandAuthority {
            decision_origin: DecisionOrigin::Actor { actor },
            seat_id: Some(SEAT.to_owned()),
            permission_profile_id: Some(PROFILE.to_owned()),
            command_subject: None,
        })
        .at_time(at),
    )
}

fn attempt_error(run: &Canwu, request: u64) -> Option<ErrorCode> {
    let attempt = run
        .command_attempts()
        .iter()
        .find(|attempt| attempt.request_id == Some(CommandRequestId::new(request)))
        .expect("command attempt");
    match &attempt.outcome {
        CommandAttemptOutcome::Accepted { .. } => None,
        CommandAttemptOutcome::Rejected { error } => Some(error.code.clone()),
    }
}

fn controller(id: &str, actor: PersonId) -> DecisionControllerBinding {
    DecisionControllerBinding::new(
        id,
        DecisionPolicyIdentity::new(DecisionPolicyKind::Utility, "realm-utility", "1"),
        DecisionAuthority::Actor { actor },
    )
}

fn ticket(id: u64, assigned_controller: &str) -> DecisionTicketDraft {
    DecisionTicketDraft {
        id: DecisionTicketId::new(id),
        definition: "fixture.edict".to_owned(),
        decision_maker: EntityRef::Person(holder()),
        assigned_controller: assigned_controller.to_owned(),
        summary: "The ruler must choose an edict".to_owned(),
        context: DecisionContext::new("fixture.edict.v1", json!({})),
        options: vec![DecisionOption::new("proclaim", "Proclaim it")],
        deadline: None,
        parent_ticket: None,
    }
}

/// Queues a rebinding at the current instant and revision and settles the
/// boundary that decides it.
fn rebind(run: &mut Canwu, request: SeatRebindingRequest) -> BoundaryReceipt {
    run.rebind_seat(request).expect("rebinding queues");
    run.step_canonical()
        .expect("boundary")
        .expect("boundary settles")
}

fn rejection(receipt: &BoundaryReceipt) -> ErrorCode {
    match &receipt.seat_rebindings[..] {
        [entry] => match &entry.outcome {
            SeatRebindingOutcome::Rejected { error } => error.code.clone(),
            SeatRebindingOutcome::Applied { .. } => panic!("the rebinding must be refused"),
        },
        other => panic!("one decided rebinding expected, got {other:?}"),
    }
}

#[test]
fn seat_rebinding_moves_the_seat_through_canonical_ingress() {
    let plugin = SeatFixture;
    let mut run = new_run(&plugin);
    let start = run.time();

    // The holder's seat controller has an open ticket, and so has a court
    // controller outside the seat that acts for the same person.
    let revision = run.revision();
    let decisions = [
        DecisionMutation::RegisterController {
            controller: controller(HOLDER_CONTROLLER, holder()).with_seat(SEAT, PROFILE),
        },
        DecisionMutation::RegisterController {
            controller: controller(HEIR_CONTROLLER, heir()).with_seat(SEAT, PROFILE),
        },
        DecisionMutation::RegisterController {
            controller: controller("court", holder()),
        },
        DecisionMutation::Open {
            ticket: ticket(1, HOLDER_CONTROLLER),
        },
        DecisionMutation::Open {
            ticket: ticket(2, "court"),
        },
    ];
    for (index, mutation) in (1..).zip(decisions) {
        run.enqueue_decision(
            start,
            0,
            DecisionIngressRequest::new(DecisionRequestId::new(index), revision, mutation),
        )
        .expect("decision ingress");
    }
    run.step_canonical()
        .expect("decision intake")
        .expect("boundary");
    assert_eq!(
        run.seat_binding(),
        Some(&binding(HOLDER_CONTROLLER, holder()))
    );

    // The cadet dies at the first daily boundary.
    let day_one = start + SimDuration::days(1);
    run.settle_boundary(BoundaryRequest::at(day_one).with_cadence(SystemCadence::Daily))
        .expect("daily boundary");

    // The call refuses another seat and a stale instant, queuing nothing.
    let other_seat = SeatRebindingRequest::new("seat.other", heir(), run.revision(), day_one);
    assert_eq!(
        run.rebind_seat(other_seat).expect_err("another seat").code,
        ErrorCode::InvalidAuthority
    );
    let stale = SeatRebindingRequest::new(SEAT, heir(), run.revision(), start);
    assert_eq!(
        run.rebind_seat(stale).expect_err("stale instant").code,
        ErrorCode::SimulationTimeConflict
    );

    // An unknown or a dead person is a recorded rejection that leaves the
    // binding, the seat's ticket, and the holder's command authority as they
    // were.
    for (person, code) in [
        (stranger(), ErrorCode::ActorNotFound),
        (cadet(), ErrorCode::IssuerUnavailable),
    ] {
        let revision = run.revision();
        let receipt = rebind(
            &mut run,
            SeatRebindingRequest::new(SEAT, person, revision, day_one),
        );
        assert_eq!(rejection(&receipt), code);
        assert_eq!(
            run.seat_binding(),
            Some(&binding(HOLDER_CONTROLLER, holder()))
        );
        assert!(
            run.decision_ticket(DecisionTicketId::new(1))
                .expect("ticket")
                .is_open()
        );
    }
    run.enqueue_command(
        day_one,
        0,
        decree_request(1, run.revision(), day_one, HOLDER_CONTROLLER, holder()),
    )
    .expect("holder decree queues");
    run.step_canonical().expect("decree").expect("boundary");
    assert_eq!(attempt_error(&run, 1), None);

    // Two decrees queue ahead of the rebinding, at a higher priority: the
    // heir's, then the holder's. Each attempt takes one revision, so the
    // later decree expects the next one.
    let revision = run.revision();
    run.enqueue_command(
        day_one,
        10,
        decree_request(3, revision, day_one, HEIR_CONTROLLER, heir()),
    )
    .expect("heir decree queues");
    run.enqueue_command(
        day_one,
        9,
        decree_request(2, revision + 1, day_one, HOLDER_CONTROLLER, holder()),
    )
    .expect("holder decree queues");

    // A pending rebinding answers its own retry and refuses a different one.
    let to_heir =
        SeatRebindingRequest::new(SEAT, heir(), revision, day_one).with_controller(HEIR_CONTROLLER);
    let receipt = run.rebind_seat(to_heir.clone()).expect("rebinding queues");
    assert_eq!(run.rebind_seat(to_heir.clone()).expect("retry"), receipt);
    let rival = SeatRebindingRequest::new(SEAT, cadet(), revision, day_one);
    assert_eq!(
        run.rebind_seat(rival)
            .expect_err("second pending rebinding")
            .code,
        ErrorCode::IdempotencyConflict
    );

    // A boundary that applies the rebinding and then fails leaves the
    // binding, the revision, the checkpoint, and the seat's ticket as they
    // were, and everything it admitted still queued.
    let checkpoint_hash = run.checkpoint_hash().to_owned();
    let boundaries = run.boundaries().len();
    run.settle_boundary(BoundaryRequest::at(day_one).with_cadence(SystemCadence::Monthly))
        .expect_err("the monthly boundary fails");
    assert_eq!(
        run.seat_binding(),
        Some(&binding(HOLDER_CONTROLLER, holder()))
    );
    assert_eq!(run.revision(), revision);
    assert_eq!(run.checkpoint_hash(), checkpoint_hash);
    assert_eq!(run.boundaries().len(), boundaries);
    assert_eq!(run.pending_ingress().count(), 3);
    assert!(
        run.decision_ticket(DecisionTicketId::new(1))
            .expect("ticket")
            .is_open()
    );

    // The boundary that applies the rebinding decides it before the decrees
    // queued ahead of it: the heir's decree is admitted and the holder's is
    // refused by the seat gate. The seat's open ticket for the holder is
    // cancelled, the court's is not.
    let settled = run.step_canonical().expect("rebinding").expect("boundary");
    let heir_binding = binding(HEIR_CONTROLLER, heir());
    let [entry] = &settled.seat_rebindings[..] else {
        panic!("one decided rebinding expected");
    };
    assert_eq!(entry.ingress, receipt.ingress_id);
    assert_eq!(
        entry.outcome,
        SeatRebindingOutcome::Applied {
            previous: Box::new(binding(HOLDER_CONTROLLER, holder())),
            binding: Box::new(heir_binding.clone()),
            cancelled_tickets: vec![DecisionTicketId::new(1)],
        }
    );
    assert_eq!(attempt_error(&run, 2), Some(ErrorCode::InvalidAuthority));
    assert_eq!(attempt_error(&run, 3), None);
    assert_eq!(run.seat_binding(), Some(&heir_binding));
    assert_eq!(
        run.decision_ticket(DecisionTicketId::new(1))
            .map(|ticket| ticket.state.clone()),
        Some(DecisionTicketState::Cancelled {
            reason: SEAT_REBOUND_REASON.to_owned()
        })
    );
    assert!(
        run.decision_ticket(DecisionTicketId::new(2))
            .expect("ticket")
            .is_open()
    );
    assert_eq!(
        run.run_configuration()
            .declared()
            .and_then(|c| c.seat_binding.clone()),
        Some(binding(HOLDER_CONTROLLER, holder()))
    );
    assert!(run.viewer().is_ok());
    assert!(run.viewer_for_actor(heir()).is_ok());
    assert_eq!(
        run.viewer_for_actor(holder()).err().map(|error| error.code),
        Some(ErrorCode::InvalidAuthority)
    );

    // The journal shows the rebinding at its boundary; a full save and an
    // exact replay of the journal reproduce it.
    let boundary = run.boundaries().last().expect("boundary");
    assert_eq!(boundary.seat_rebindings, settled.seat_rebindings);
    assert!(boundary.admitted_ingress.contains(&receipt.ingress_id));
    let snapshot = run.snapshot();
    assert_eq!(snapshot.rebound_seat.as_ref(), Some(&heir_binding));
    let json = serde_json::to_string(&snapshot).expect("snapshot json");
    let restored = Canwu::from_snapshot_json_with_plugins(&json, &[&plugin]).expect("restore");
    assert_eq!(restored.snapshot(), snapshot);
    assert_eq!(restored.seat_binding(), Some(&heir_binding));
    let replayed = Canwu::replay_from_journal(&[&plugin], &run.replay_journal()).expect("replay");
    assert_eq!(replayed.snapshot(), snapshot);
    assert_eq!(replayed.checkpoint_hash(), run.checkpoint_hash());

    // A save whose rebound binding disagrees with its rebinding history is
    // refused.
    let mut tampered = snapshot.clone();
    tampered.rebound_seat = Some(binding(HOLDER_CONTROLLER, heir()));
    let tampered = serde_json::to_string(&tampered).expect("tampered json");
    assert!(Canwu::from_snapshot_json_with_plugins(&tampered, &[&plugin]).is_err());

    // A continuation saved after the rebinding restores with the heir bound
    // and the same hashes, and its seat gate keeps following the heir. The
    // checkpoint hash binds the rebound binding: a changed one is refused
    // even when the continuation hash is recomputed over it.
    let (checkpoint, hash) = run.continuation_checkpoint().expect("continuation");
    let mut forged = checkpoint.clone();
    forged.state.rebound_seat = Some(binding(HOLDER_CONTROLLER, heir()));
    forged.continuation_hash = forged
        .compute_continuation_hash()
        .expect("continuation hash");
    assert_eq!(
        CompactedCanwu::from_continuation_checkpoint_with_plugins(forged, &[&plugin])
            .map(|_| ())
            .expect_err("forged binding")
            .code,
        ErrorCode::InvalidSnapshot
    );
    let mut continued =
        CompactedCanwu::from_continuation_checkpoint_verified(checkpoint, &hash, &[&plugin])
            .expect("continuation restore");
    assert_eq!(continued.seat_binding(), Some(&heir_binding));
    assert_eq!(continued.checkpoint_hash(), run.checkpoint_hash());
    assert_eq!(
        continued.authoritative_state_hash().expect("state hash"),
        run.authoritative_state_hash().expect("state hash")
    );
    let revision = continued.revision();
    continued
        .enqueue_command(
            day_one,
            0,
            decree_request(4, revision, day_one, HOLDER_CONTROLLER, holder()),
        )
        .expect("holder decree queues");
    continued
        .enqueue_command(
            day_one,
            0,
            decree_request(5, revision + 1, day_one, HEIR_CONTROLLER, heir()),
        )
        .expect("heir decree queues");
    continued
        .step_canonical()
        .expect("decrees")
        .expect("boundary");
    let outcomes: Vec<_> = continued
        .command_attempts()
        .iter()
        .filter_map(|attempt| {
            let request = attempt.request_id?.get();
            (request >= 4).then(|| match &attempt.outcome {
                CommandAttemptOutcome::Accepted { .. } => (request, None),
                CommandAttemptOutcome::Rejected { error } => (request, Some(error.code.clone())),
            })
        })
        .collect();
    assert_eq!(
        outcomes,
        vec![(4, Some(ErrorCode::InvalidAuthority)), (5, None)]
    );
}
