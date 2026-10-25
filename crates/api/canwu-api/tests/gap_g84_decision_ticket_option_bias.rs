//! Downstream gap G-84: a ticket's drafter writes a per-option bias
//! (`DecisionOption::bias`) at `Open` and `ReplaceOptions`. One controller and
//! one policy then answer tickets whose biases differ: the utility stage adds
//! the ticket's bias to the policy's own and the trace records the sum apart
//! from the factor lines. The bias is committed ticket state: it survives a
//! strict reload and exact replay, and a snapshot that edits it is refused.

use canwu_api::{
    Canwu, DecisionAttemptOutcome, DecisionAuthority, DecisionContext, DecisionControllerBinding,
    DecisionEvaluation, DecisionIngressRequest, DecisionMutation, DecisionOption,
    DecisionOptionEvaluation, DecisionPolicy, DecisionRequestId, DecisionTicketDraft,
    DecisionTicketId, DecisionTicketState, EntityRef, ErrorCode, GuardedUtilityPolicy,
    OrderedRulePolicy, SimDuration, SimTime, UtilityProfile, WeightedUtilityEvaluator,
};
use serde_json::json;
use std::collections::BTreeMap;

const CONTROLLER: &str = "g84-doctrine";

/// One doctrine for every ticket: merit weighs 10, and `hold` carries the
/// policy's own standing +5.
fn policy() -> GuardedUtilityPolicy {
    GuardedUtilityPolicy::new(
        "g84-doctrine-utility",
        "1",
        OrderedRulePolicy::new("g84-doctrine-guards", "1", Vec::new()),
        WeightedUtilityEvaluator::new(UtilityProfile {
            weights: BTreeMap::from([("merit".to_owned(), 10)]),
        })
        .with_option_bias("hold", 5),
        0,
        false,
    )
}

/// `hold` (merit 2) and `proceed` (merit 3), each with the drafter's bias.
fn options(hold_bias: i64, proceed_bias: i64) -> Vec<DecisionOption> {
    let option = |id: &str, merit: i64, bias: i64| DecisionOption {
        utility_inputs: BTreeMap::from([("merit".to_owned(), merit)]),
        ..DecisionOption::new(id, id).with_bias(bias)
    };
    vec![
        option("hold", 2, hold_bias),
        option("proceed", 3, proceed_bias),
    ]
}

fn draft(id: u64, options: Vec<DecisionOption>) -> DecisionTicketDraft {
    DecisionTicketDraft {
        id: DecisionTicketId::new(id),
        definition: "fixture.party-answer".to_owned(),
        decision_maker: EntityRef::Person(Canwu::demo_ids().commander),
        assigned_controller: CONTROLLER.to_owned(),
        summary: format!("Fixture answer {id}"),
        context: DecisionContext::new("fixture.party-answer.v1", json!({ "ticket": id })),
        options,
        deadline: None,
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
}

fn accepted(canwu: &Canwu, request_id: u64) {
    let outcome = canwu
        .decision_attempt(DecisionRequestId::new(request_id))
        .map(|attempt| attempt.outcome.clone());
    assert!(
        matches!(outcome, Some(DecisionAttemptOutcome::Accepted { .. })),
        "request {request_id}: {outcome:?}"
    );
}

/// The selected option of a resolved ticket and its trace's evaluations by
/// option ID.
fn resolution(canwu: &Canwu, ticket: u64) -> (String, BTreeMap<String, DecisionOptionEvaluation>) {
    let Some(DecisionTicketState::Resolved {
        option_id,
        trace_id,
    }) = canwu
        .decision_ticket(DecisionTicketId::new(ticket))
        .map(|ticket| ticket.state.clone())
    else {
        panic!("ticket {ticket} should be resolved");
    };
    let trace = canwu.decision_trace(trace_id).expect("resolution trace");
    assert_eq!(trace.controller_id, CONTROLLER);
    let evaluations = trace
        .evaluations
        .iter()
        .map(|evaluation| (evaluation.option_id.clone(), evaluation.clone()))
        .collect();
    (option_id, evaluations)
}

#[test]
#[allow(clippy::too_many_lines)]
fn gap_g84_ticket_option_bias_shares_one_controller() {
    let mut canwu = Canwu::demo(984).expect("demo");
    let start = canwu.time();
    let policy = policy();
    settle_at(
        &mut canwu,
        start,
        vec![
            (
                1,
                DecisionMutation::RegisterController {
                    controller: DecisionControllerBinding::new(
                        CONTROLLER,
                        policy.identity().clone(),
                        DecisionAuthority::Actor {
                            actor: Canwu::demo_ids().commander,
                        },
                    ),
                },
            ),
            (
                2,
                DecisionMutation::Open {
                    ticket: draft(1, options(0, 0)),
                },
            ),
            (
                3,
                DecisionMutation::Open {
                    ticket: draft(2, options(10, 0)),
                },
            ),
            (
                4,
                DecisionMutation::Open {
                    ticket: draft(3, options(0, 0)),
                },
            ),
        ],
    );
    // The drafter re-biases an open ticket with its options: `proceed`
    // loses 10 on ticket 3.
    settle_at(
        &mut canwu,
        start + SimDuration::hours(1),
        vec![(
            5,
            DecisionMutation::ReplaceOptions {
                ticket_id: DecisionTicketId::new(3),
                expected_version: 1,
                context: DecisionContext::new("fixture.party-answer.v1", json!({ "ticket": 3 })),
                options: options(0, -10),
            },
        )],
    );
    for request_id in 1..=5 {
        accepted(&canwu, request_id);
    }
    assert_eq!(
        canwu
            .decision_ticket(DecisionTicketId::new(3))
            .and_then(|ticket| ticket.option("proceed"))
            .map(|option| option.bias),
        Some(-10)
    );

    // The one controller answers all three with the same policy.
    let at = canwu.time();
    for (request_id, ticket) in [(6, 1), (7, 2), (8, 3)] {
        let DecisionEvaluation::Prepared(_) = canwu
            .drive_decision(
                at,
                0,
                DecisionRequestId::new(request_id),
                None,
                DecisionTicketId::new(ticket),
                &policy,
            )
            .expect("the doctrine's decision should prepare")
        else {
            panic!("the utility policy resolves ticket {ticket}");
        };
    }
    canwu
        .step_canonical()
        .expect("resolution boundary")
        .expect("resolution receipt");
    for request_id in [6, 7, 8] {
        accepted(&canwu, request_id);
    }

    // Ticket 1: the doctrine's +5 alone leaves `proceed` ahead (30 to 25).
    let (selected, evaluations) = resolution(&canwu, 1);
    assert_eq!(selected, "proceed");
    assert_eq!(
        (evaluations["hold"].score, evaluations["hold"].bias),
        (Some(25), 5)
    );
    // Ticket 2: the drafter's +10 joins the doctrine's +5 in one bias line,
    // beside unchanged factor lines, and turns the answer (35 to 30).
    let (selected, biased) = resolution(&canwu, 2);
    assert_eq!(selected, "hold");
    let hold = &biased["hold"];
    assert_eq!((hold.score, hold.bias), (Some(35), 15));
    assert_eq!(hold.factors, evaluations["hold"].factors);
    let factors: i64 = hold.factors.iter().map(|line| line.contribution).sum();
    assert_eq!(hold.score, Some(factors + hold.bias));
    // Ticket 3: the replaced options' bias is the one scored (25 to 20).
    let (selected, rebiased) = resolution(&canwu, 3);
    assert_eq!(selected, "hold");
    assert_eq!(
        (rebiased["proceed"].score, rebiased["proceed"].bias),
        (Some(20), -10)
    );

    // The bias is committed ticket state: a strict reload gives back the
    // same snapshot, and one that edits it is refused.
    let snapshot = canwu.snapshot();
    let encoded = serde_json::to_value(&snapshot).expect("snapshot json");
    let restored =
        Canwu::from_snapshot_json(&encoded.to_string()).expect("the biased tickets should reload");
    assert_eq!(restored.snapshot(), snapshot);
    let mut tampered = encoded;
    let hold_option = tampered["decisions"]["tickets"]["2"]["options"]
        .as_array_mut()
        .and_then(|options| options.iter_mut().find(|option| option["id"] == "hold"))
        .expect("ticket 2 encodes its hold option");
    assert_eq!(hold_option["bias"], json!(10));
    hold_option["bias"] = json!(-10);
    let error = Canwu::from_snapshot_json(&tampered.to_string())
        .err()
        .expect("a snapshot whose ticket bias was edited must not load");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    assert!(
        error.message.contains("does not reconstruct"),
        "the admitted drafts must disagree with the edited ticket: {}",
        error.message
    );

    // Exact replay re-admits the biased drafts and gives back the same run.
    let replayed = Canwu::replay_from_journal(&[], &canwu.replay_journal())
        .expect("the biased tickets should replay");
    assert_eq!(replayed.snapshot(), snapshot);
}
