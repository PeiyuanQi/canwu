//! Downstream gap G-82: a boundary system lists the tickets its boundary's
//! admitted `Open`s created, and the open tickets of a decision maker or of a
//! controller, through `SimulationView` behind `canwu.core.decisions`; the
//! host reads the same lists on `Canwu` and `CompactedCanwu`. The lists are
//! the same in a live run, a strict reload, an exact replay, and a run
//! continued from a continuation checkpoint.

#![allow(clippy::too_many_lines, clippy::unnecessary_wraps)]

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundaryRequest,
    BoundarySystemContract, Canwu, CanwuError, CompactedCanwu, DecisionAttemptErrorCode,
    DecisionAttemptOutcome, DecisionAuthority, DecisionContext, DecisionControllerBinding,
    DecisionIngressRequest, DecisionMutation, DecisionOption, DecisionPolicyIdentity,
    DecisionPolicyKind, DecisionRequestId, DecisionTicket, DecisionTicketDraft, DecisionTicketId,
    EntityRef, PersonId, PluginRegistrar, Scenario, SimDuration, SimTime, SimulationPlugin,
    SimulationView, StateKey, SystemCadence,
};
use serde_json::json;

const PLUGIN: &str = "fixture-desk";
const DESK_A: &str = "desk-a";
const DESK_B: &str = "desk-b";

const fn maker_a() -> PersonId {
    PersonId::new(1)
}

const fn maker_b() -> PersonId {
    PersonId::new(2)
}

fn ids<'a>(tickets: impl Iterator<Item = &'a DecisionTicket>) -> Vec<u64> {
    tickets.map(|ticket| ticket.id.get()).collect()
}

fn joined(ids: &[u64]) -> String {
    ids.iter().map(u64::to_string).collect::<Vec<_>>().join(",")
}

/// Emits what the boundary opened, and the open tickets of the first opened
/// ticket's decision maker and controller, when the boundary opened any.
fn list_opened(
    view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let opened = view.decision_tickets_opened()?.collect::<Vec<_>>();
    let Some(first) = opened.first() else {
        return Ok(BoundaryProposal::default());
    };
    let maker = ids(view.open_decision_tickets_of(&first.decision_maker)?);
    let desk = ids(view.open_decision_tickets_of_controller(&first.assigned_controller)?);
    let mut proposal = BoundaryProposal::default();
    proposal.directives.push(BoundaryDirective::Emit {
        event_type: "opened".to_owned(),
        summary: format!(
            "opened={} maker={} controller={}",
            joined(&ids(opened.into_iter())),
            joined(&maker),
            joined(&desk)
        ),
        affected: Vec::new(),
    });
    Ok(proposal)
}

struct DeskPlugin;

impl SimulationPlugin for DeskPlugin {
    fn name(&self) -> &'static str {
        PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000000a82"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut contract = BoundarySystemContract::new(
            "list-opened",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        contract.reads = vec![StateKey::core_decisions()];
        contract.emits = vec!["opened".to_owned()];
        registrar.register_boundary_system(contract, list_opened)
    }
}

fn identity() -> DecisionPolicyIdentity {
    DecisionPolicyIdentity::new(DecisionPolicyKind::Human, "fixture-desk-human", "1")
}

fn open(id: u64, maker: PersonId, desk: &str, deadline: Option<SimTime>) -> DecisionMutation {
    DecisionMutation::Open {
        ticket: DecisionTicketDraft {
            id: DecisionTicketId::new(id),
            definition: "fixture.desk".to_owned(),
            decision_maker: EntityRef::Person(maker),
            assigned_controller: desk.to_owned(),
            summary: format!("Fixture desk ticket {id}"),
            context: DecisionContext::new("fixture.desk.v1", json!({ "ticket": id })),
            options: vec![DecisionOption::new("seal", "Seal")],
            deadline,
            parent_ticket: None,
        },
    }
}

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
}

fn summaries(canwu: &Canwu) -> Vec<String> {
    canwu
        .events()
        .iter()
        .filter(|event| event.kind.field("event_type") == Some(json!("opened")))
        .map(|event| event.summary.clone())
        .collect()
}

/// The host's three reads: what the latest boundary opened, each maker's open
/// tickets, and each desk's.
type HostReads = (Vec<u64>, [Vec<u64>; 2], [Vec<u64>; 2]);

fn host_reads(canwu: &Canwu) -> HostReads {
    (
        ids(canwu.decision_tickets_opened()),
        [maker_a(), maker_b()]
            .map(|maker| ids(canwu.open_decision_tickets_of(&EntityRef::Person(maker)))),
        [DESK_A, DESK_B].map(|desk| ids(canwu.open_decision_tickets_of_controller(desk))),
    )
}

fn compacted_reads(canwu: &CompactedCanwu) -> HostReads {
    (
        ids(canwu.decision_tickets_opened()),
        [maker_a(), maker_b()]
            .map(|maker| ids(canwu.open_decision_tickets_of(&EntityRef::Person(maker)))),
        [DESK_A, DESK_B].map(|desk| ids(canwu.open_decision_tickets_of_controller(desk))),
    )
}

#[test]
fn gap_g82_decision_ticket_reads() {
    let plugin = DeskPlugin;
    let plugins: [&dyn SimulationPlugin; 1] = [&plugin];
    let scenario = Scenario::new(
        SimTime::EPOCH,
        vec![EntityRef::Person(maker_a()), EntityRef::Person(maker_b())],
    );
    let mut canwu = Canwu::new_with_plugins(82, scenario, &plugins).expect("desk run");
    let minutes = |count| SimTime::EPOCH + SimDuration::minutes(count);
    let register = |id: &str, actor| DecisionMutation::RegisterController {
        controller: DecisionControllerBinding::new(
            id,
            identity(),
            DecisionAuthority::Actor { actor },
        ),
    };

    // Admission order, not ticket-ID order.
    settle_at(
        &mut canwu,
        minutes(0),
        vec![
            (1, register(DESK_A, maker_a())),
            (2, register(DESK_B, maker_b())),
            (3, open(3, maker_a(), DESK_B, None)),
            (4, open(1, maker_a(), DESK_A, None)),
            (5, open(2, maker_b(), DESK_A, None)),
        ],
    );
    assert_eq!(
        summaries(&canwu),
        vec!["opened=3,1,2 maker=1,3 controller=3"]
    );
    assert_eq!(
        host_reads(&canwu),
        (vec![3, 1, 2], [vec![1, 3], vec![2]], [vec![1, 2], vec![3]])
    );

    // A cancellation and a reassignment move tickets out of and between the
    // indexes; a refused `Open` lists nothing.
    settle_at(
        &mut canwu,
        minutes(60),
        vec![
            (
                6,
                DecisionMutation::Cancel {
                    ticket_id: DecisionTicketId::new(1),
                    expected_version: 1,
                    reason: "withdrawn".to_owned(),
                },
            ),
            (
                7,
                DecisionMutation::Reassign {
                    ticket_id: DecisionTicketId::new(2),
                    expected_version: 1,
                    controller_id: DESK_B.to_owned(),
                },
            ),
            (8, open(4, maker_b(), DESK_A, Some(minutes(90)))),
            (9, open(3, maker_b(), DESK_A, None)),
        ],
    );
    assert!(matches!(
        canwu
            .decision_attempt(DecisionRequestId::new(9))
            .map(|attempt| &attempt.outcome),
        Some(DecisionAttemptOutcome::Rejected {
            code: DecisionAttemptErrorCode::DuplicateTicket,
            ..
        })
    ));
    assert_eq!(summaries(&canwu)[1..], ["opened=4 maker=2,4 controller=4"]);
    let after_second = host_reads(&canwu);
    assert_eq!(
        after_second,
        (vec![4], [vec![3], vec![2, 4]], [vec![4], vec![2, 3]])
    );

    // The admission batch is persisted decision state: a strict reload
    // reconstructs it from the admitted ingress, exact replay settles to the
    // same lists, and a continuation checkpoint, which carries no evidence,
    // restores it through the decision commitment.
    let snapshot = canwu.snapshot();
    let reloaded = Canwu::from_snapshot_json_with_plugins(
        &serde_json::to_string(&snapshot).expect("snapshot json"),
        &plugins,
    )
    .expect("the desk run should reload");
    assert_eq!(host_reads(&reloaded), after_second);
    let replayed = Canwu::replay_from_journal(&plugins, &canwu.replay_journal())
        .expect("the desk run should replay");
    assert_eq!(replayed.snapshot(), snapshot);
    let (checkpoint, _) = canwu
        .continuation_checkpoint()
        .expect("continuation checkpoint");
    let mut continued =
        CompactedCanwu::from_continuation_checkpoint_with_plugins(checkpoint, &plugins)
            .expect("the desk run should continue");
    assert_eq!(compacted_reads(&continued), after_second);

    // A boundary that opens nothing starts an empty batch, and a deadline
    // that passes takes its ticket out of the indexes.
    canwu
        .settle_boundary(BoundaryRequest::at(minutes(120)))
        .expect("a boundary that opens nothing");
    continued
        .settle_boundary(BoundaryRequest::at(minutes(120)))
        .expect("the continued run settles alike");
    let quiet = (Vec::new(), [vec![3], vec![2]], [Vec::new(), vec![2, 3]]);
    assert_eq!(host_reads(&canwu), quiet);
    assert_eq!(compacted_reads(&continued), quiet);
    assert_eq!(summaries(&canwu).len(), 2);
}
