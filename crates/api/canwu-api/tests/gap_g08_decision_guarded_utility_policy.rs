//! Gap G-08: guarded utility policy. Ordered guards exclude or select before
//! weighted utility scoring; a near-equivalent utility tie is resolved by the
//! existing `ResolveDecisionRandomly` directive over only the tied candidates,
//! and every trace records its stage and fired guards.

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundaryRequest,
    BoundarySystemContract, Canwu, CanwuError, ControllerDecision, DecisionAuthority,
    DecisionContext, DecisionController, DecisionControllerBinding, DecisionError,
    DecisionErrorCode, DecisionEvaluation, DecisionIngressRequest, DecisionMutation,
    DecisionOption, DecisionOptionWeight, DecisionOutcome, DecisionPolicy, DecisionRandomEvidence,
    DecisionRequestId, DecisionRule, DecisionStage, DecisionTicket, DecisionTicketDraft,
    DecisionTicketId, DecisionTraceId, EntityRef, ErrorCode, EvidenceRef, GuardedUtilityPolicy,
    OrderedRulePolicy, PluginRegistrar, PolicyDecision, RandomDecisionResolution, RandomDrawId,
    RandomDrawOutcome, RandomOperationTarget, RandomStreamKey, RuleChoice, SimDuration, SimTime,
    SimulationPlugin, SimulationView, StateKey, SystemCadence, UtilityProfile,
    WeightedUtilityEvaluator,
};
use serde_json::json;
use std::collections::BTreeMap;

const CONTROLLER: &str = "guarded-controller";
const GUARD_TICKET: DecisionTicketId = DecisionTicketId::new(1);
const TIE_TICKET: DecisionTicketId = DecisionTicketId::new(2);
const UNOPTED_TICKET: DecisionTicketId = DecisionTicketId::new(3);

struct ExcludeOption;

impl DecisionRule for ExcludeOption {
    fn id(&self) -> &'static str {
        "exclude-gamma"
    }

    fn evaluate(&self, _ticket: &DecisionTicket) -> Result<RuleChoice, DecisionError> {
        Ok(RuleChoice::Exclude {
            option_id: "gamma".to_owned(),
            reason: "gamma is forbidden by standing policy".to_owned(),
        })
    }
}

struct SelectWhenUrgent(&'static str);

impl DecisionRule for SelectWhenUrgent {
    fn id(&self) -> &'static str {
        "urgent-select"
    }

    fn evaluate(&self, ticket: &DecisionTicket) -> Result<RuleChoice, DecisionError> {
        Ok(if ticket.context.payload["urgent"] == json!(true) {
            RuleChoice::Select(self.0.to_owned())
        } else {
            RuleChoice::NoMatch
        })
    }
}

fn policy_with(urgent_target: &'static str, margin: u64) -> GuardedUtilityPolicy {
    GuardedUtilityPolicy::new(
        "guarded-utility",
        "1",
        OrderedRulePolicy::new(
            "standing-guards",
            "1",
            vec![
                Box::new(ExcludeOption),
                Box::new(SelectWhenUrgent(urgent_target)),
            ],
        ),
        WeightedUtilityEvaluator::new(UtilityProfile {
            weights: BTreeMap::from([("merit".to_owned(), 1)]),
        }),
        margin,
        true,
    )
}

fn policy() -> GuardedUtilityPolicy {
    policy_with("delta", 1)
}

fn tie_stream() -> RandomStreamKey {
    RandomStreamKey::new("fixture-guarded-utility", "tie-break", 1)
}

fn decision_error(error: &DecisionError) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidDecision, error.to_string())
}

/// Resolves the open tie ticket with the existing random directive. With
/// `widen` in the ticket context it deliberately draws over every available
/// option instead of the pending candidates.
fn tie_break_system(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let Some(ticket) = view
        .decision_ticket(TIE_TICKET)?
        .filter(|ticket| ticket.is_open())
    else {
        return Ok(BoundaryProposal::default());
    };
    let controller = view
        .decision_controller(&ticket.assigned_controller)?
        .ok_or_else(|| CanwuError::new(ErrorCode::InvalidDecision, "missing controller"))?;
    let ControllerDecision::Pending(pending) =
        DecisionController::evaluate(ticket, controller, &policy())
            .map_err(|error| decision_error(&error))?
    else {
        return Ok(BoundaryProposal::default());
    };
    let DecisionOutcome::PendingRandom { candidates } = &pending.outcome else {
        return Ok(BoundaryProposal::default());
    };
    let option_weights = if ticket.context.payload["widen"] == json!(true) {
        ticket
            .options
            .iter()
            .filter(|option| option.is_available())
            .map(|option| DecisionOptionWeight::new(option.id.clone(), 1))
            .collect()
    } else {
        candidates.clone()
    };
    let calendar = context
        .admitted_ingress
        .first()
        .copied()
        .ok_or_else(|| CanwuError::new(ErrorCode::InvalidDecision, "missing calendar"))?;
    let sample = view.random_sample_for_operation(
        &tie_stream(),
        EvidenceRef::Ingress(calendar),
        "decision_tie_break",
        "fixture-tie-break",
        RandomOperationTarget::DecisionTicket {
            ticket_id: ticket.id,
            ticket_version: ticket.version,
        },
        0,
        option_weights.iter().map(|weight| weight.weight).sum(),
        "break a near-equivalent utility tie",
    )?;
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::ResolveDecisionRandomly {
            resolution: RandomDecisionResolution {
                priority: 0,
                decision_request_id: DecisionRequestId::new(50),
                command_request_id: None,
                ticket_id: ticket.id,
                expected_version: ticket.version,
                controller_id: ticket.assigned_controller.clone(),
                sample,
                option_weights,
                tie_break: Some(Box::new(pending)),
            },
        }],
        ..BoundaryProposal::default()
    })
}

struct TieBreakPlugin;

impl SimulationPlugin for TieBreakPlugin {
    fn name(&self) -> &'static str {
        "fixture-guarded-utility"
    }

    fn version(&self) -> &'static str {
        "1.0.0"
    }

    fn semantic_hash(&self) -> &'static str {
        "6d1f3b0c9a2e4f5a8b7c6d5e4f3a2b1c0d9e8f7a6b5c4d3e2f1a0b9c8d7e6f5a"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut contract = BoundarySystemContract::new(
            "resolve-utility-tie",
            BoundaryPhase::StrategicAggregation,
            SystemCadence::Daily,
        );
        contract.reads = vec![StateKey::core_decisions()];
        contract.random_streams = vec![tie_stream()];
        registrar.register_boundary_system(contract, tie_break_system)
    }
}

fn draft(id: DecisionTicketId, context: serde_json::Value) -> DecisionTicketDraft {
    let option = |id: &str, merit: i64| DecisionOption {
        utility_inputs: BTreeMap::from([("merit".to_owned(), merit)]),
        ..DecisionOption::new(id, id)
    };
    DecisionTicketDraft {
        id,
        definition: "fixture.course-of-action".to_owned(),
        decision_maker: EntityRef::Person(Canwu::demo_ids().commander),
        assigned_controller: CONTROLLER.to_owned(),
        summary: "Choose a course of action".to_owned(),
        context: DecisionContext::new("fixture.course-of-action.v1", context),
        // Gamma scores best but is excluded; alpha and beta lie within the
        // margin; delta is well behind.
        options: vec![
            option("alpha", 10),
            option("beta", 9),
            option("gamma", 50),
            option("delta", 3),
        ],
        deadline: None,
        parent_ticket: None,
    }
}

fn step(canwu: &mut Canwu) {
    canwu
        .step_canonical()
        .expect("boundary should settle")
        .expect("boundary receipt");
}

#[test]
#[allow(clippy::too_many_lines)]
fn gap_g08_decision_guarded_utility_policy() {
    let plugin = TieBreakPlugin;
    let mut canwu = Canwu::demo(808).expect("demo");
    canwu.register_plugin(&plugin).expect("plugin");
    let policy = policy();
    let identity = policy.identity().clone();
    assert!(identity.semantic_hash.is_some());
    let (now, revision) = (canwu.time(), canwu.revision());
    for (request_id, mutation) in [
        (
            1,
            DecisionMutation::RegisterController {
                controller: DecisionControllerBinding::new(
                    CONTROLLER,
                    identity.clone(),
                    DecisionAuthority::Actor {
                        actor: Canwu::demo_ids().commander,
                    },
                )
                .with_random_tie_break(),
            },
        ),
        (
            4,
            DecisionMutation::RegisterController {
                controller: DecisionControllerBinding::new(
                    "unopted-controller",
                    identity.clone(),
                    DecisionAuthority::Actor {
                        actor: Canwu::demo_ids().commander,
                    },
                ),
            },
        ),
        (
            2,
            DecisionMutation::Open {
                ticket: draft(GUARD_TICKET, json!({"urgent": true, "widen": false})),
            },
        ),
        (
            3,
            DecisionMutation::Open {
                ticket: draft(TIE_TICKET, json!({"urgent": false, "widen": false})),
            },
        ),
        (
            5,
            DecisionMutation::Open {
                ticket: DecisionTicketDraft {
                    assigned_controller: "unopted-controller".to_owned(),
                    ..draft(UNOPTED_TICKET, json!({"urgent": false}))
                },
            },
        ),
    ] {
        canwu
            .enqueue_decision(
                now,
                0,
                DecisionIngressRequest::new(DecisionRequestId::new(request_id), revision, mutation),
            )
            .expect("decision intake");
    }
    canwu
        .settle_boundary(BoundaryRequest::at(now))
        .expect("intake boundary");

    // Identity covers configuration: the same ID and version with another
    // margin no longer matches the registered controller.
    let drifted = canwu.prepare_decision(
        DecisionRequestId::new(90),
        None,
        GUARD_TICKET,
        &policy_with("delta", 2),
    );
    assert!(drifted.is_err(), "reconfigured policy must not match");

    // Guard order: gamma is excluded, then the urgent guard selects delta.
    let guard_ticket = canwu.decision_ticket(GUARD_TICKET).expect("guard ticket");
    let conflicting = policy_with("gamma", 1)
        .decide(guard_ticket)
        .expect_err("a later guard cannot select an excluded option");
    assert_eq!(conflicting.code, DecisionErrorCode::InvalidDecision);
    let DecisionEvaluation::Prepared(_) = canwu
        .drive_decision(
            now,
            0,
            DecisionRequestId::new(10),
            None,
            GUARD_TICKET,
            &policy,
        )
        .expect("guard decision")
    else {
        panic!("a selecting guard is authoritative");
    };
    step(&mut canwu);
    let guard_trace = canwu
        .decision_trace(DecisionTraceId::new(1))
        .expect("guard trace")
        .clone();
    assert_eq!(guard_trace.stage, Some(DecisionStage::Guard));
    assert_eq!(guard_trace.fired_guards, ["exclude-gamma", "urgent-select"]);
    assert_eq!(
        guard_trace.outcome,
        DecisionOutcome::Selected {
            option_id: "delta".to_owned()
        }
    );
    assert!(
        guard_trace
            .evaluations
            .iter()
            .any(|evaluation| evaluation.option_id == "gamma"
                && !evaluation.available
                && evaluation.blockers[0].contains("exclude-gamma"))
    );

    // Utility stage: gamma stays excluded; alpha and beta tie within the
    // margin, so the policy leaves a pending random tie-break and the host
    // cannot resolve it directly.
    let DecisionEvaluation::Pending(pending) = canwu
        .prepare_decision(DecisionRequestId::new(11), None, TIE_TICKET, &policy)
        .expect("tie evaluation")
    else {
        panic!("a near tie must stay pending for the random directive");
    };
    let tied = vec![
        DecisionOptionWeight::new("alpha", 1),
        DecisionOptionWeight::new("beta", 1),
    ];
    assert_eq!(
        pending.outcome,
        DecisionOutcome::PendingRandom {
            candidates: tied.clone()
        }
    );
    assert_eq!(pending.stage, Some(DecisionStage::Random));
    assert_eq!(pending.fired_guards, ["exclude-gamma"]);
    // A tie needs a binding that opted into random tie-breaks, and a host
    // cannot author the draw itself.
    assert!(
        canwu
            .prepare_decision(DecisionRequestId::new(12), None, UNOPTED_TICKET, &policy)
            .is_err()
    );
    let forged = canwu.enqueue_decision(
        now,
        0,
        DecisionIngressRequest::new(
            DecisionRequestId::new(13),
            canwu.revision(),
            DecisionMutation::Resolve {
                ticket_id: TIE_TICKET,
                expected_version: 1,
                controller_id: CONTROLLER.to_owned(),
                policy: identity.clone(),
                decision: PolicyDecision {
                    outcome: DecisionOutcome::Selected {
                        option_id: "alpha".to_owned(),
                    },
                    summary: "random tie-break selected alpha".to_owned(),
                    random: Some(DecisionRandomEvidence {
                        draw_id: RandomDrawId::new(1),
                        value: 0,
                        upper_exclusive: 2,
                        option_weights: tied.clone(),
                    }),
                    ..pending.clone()
                },
                command_request_id: None,
            },
        ),
    );
    assert_eq!(
        forged.expect_err("host-authored draws are rejected").code,
        ErrorCode::InvalidDecision
    );
    let scores = pending
        .evaluations
        .iter()
        .map(|evaluation| (evaluation.option_id.as_str(), evaluation.score))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        scores,
        BTreeMap::from([
            ("alpha", Some(10)),
            ("beta", Some(9)),
            ("delta", Some(3)),
            ("gamma", None)
        ])
    );

    // The boundary cannot widen the draw beyond the pending candidates.
    let mut widened = canwu.fork();
    let (widened_now, widened_revision) = (widened.time(), widened.revision());
    widened
        .enqueue_decision(
            widened_now,
            0,
            DecisionIngressRequest::new(
                DecisionRequestId::new(20),
                widened_revision,
                DecisionMutation::ReplaceOptions {
                    ticket_id: TIE_TICKET,
                    expected_version: 1,
                    context: DecisionContext::new(
                        "fixture.course-of-action.v1",
                        json!({"urgent": false, "widen": true}),
                    ),
                    options: draft(TIE_TICKET, json!({})).options,
                },
            ),
        )
        .expect("widening refresh");
    widened
        .settle_boundary(BoundaryRequest::at(widened_now))
        .expect("refresh boundary");
    let selection_at = SimTime::EPOCH + SimDuration::days(1);
    widened
        .schedule_calendar_boundary(selection_at, vec![SystemCadence::Daily])
        .expect("widened calendar");
    let error = widened
        .step_canonical()
        .expect_err("a widened tie-break draw must be rejected");
    assert_eq!(error.code, ErrorCode::InvalidDecision);
    assert!(
        error.message.contains("pending candidates"),
        "{}",
        error.message
    );

    // The existing random directive resolves the tie over alpha and beta only.
    canwu
        .schedule_calendar_boundary(selection_at, vec![SystemCadence::Daily])
        .expect("calendar");
    step(&mut canwu);
    step(&mut canwu);
    let trace = canwu
        .decision_trace(DecisionTraceId::new(2))
        .expect("tie-break trace");
    assert_eq!(trace.ticket_id, TIE_TICKET);
    assert_eq!(trace.policy, identity);
    assert_eq!(trace.stage, Some(DecisionStage::Random));
    assert_eq!(trace.fired_guards, ["exclude-gamma"]);
    assert_eq!(trace.evaluations, pending.evaluations);
    let random = trace.random.as_ref().expect("draw evidence");
    assert_eq!(random.option_weights, tied);
    assert_eq!(random.upper_exclusive, 2);
    let DecisionOutcome::Selected { option_id } = &trace.outcome else {
        panic!("the tie-break selects an option");
    };
    assert!(option_id == "alpha" || option_id == "beta");
    assert!(canwu.random_draws().iter().any(|draw| {
        draw.id == random.draw_id
            && matches!(
                &draw.outcome,
                Some(RandomDrawOutcome::DecisionSelection {
                    ticket_id,
                    option_id: drawn,
                    ..
                }) if *ticket_id == TIE_TICKET && drawn == option_id
            )
    }));

    // Stage and fired guards are persisted evidence: save/load and exact
    // replay reproduce the same state without rerunning the policy.
    let snapshot = canwu.snapshot();
    let restored = Canwu::from_snapshot_json_with_plugins(
        &serde_json::to_string(&snapshot).expect("snapshot json"),
        &[&plugin],
    )
    .expect("guarded decisions should reload");
    assert_eq!(restored.snapshot(), snapshot);
    let replayed = Canwu::replay_from_journal(&[&plugin], &canwu.replay_journal())
        .expect("guarded decisions should replay exactly");
    assert_eq!(replayed.snapshot(), snapshot);
}
