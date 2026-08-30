use super::*;

const TICKET_ID: DecisionTicketId = DecisionTicketId::new(1);

fn random_decision_stream() -> RandomStreamKey {
    RandomStreamKey::new("fixture-random-decision", "decision-selection", 1)
}

fn resolve_ticket_randomly(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let ticket = view.decision_ticket(TICKET_ID)?.ok_or_else(|| {
        CanwuError::new(
            ErrorCode::InvalidDecision,
            "random decision fixture ticket is missing",
        )
    })?;
    let evidence = context.admitted_ingress.first().copied().ok_or_else(|| {
        CanwuError::new(
            ErrorCode::EvidenceUnavailable,
            "random decision fixture requires admitted calendar evidence",
        )
    })?;
    let option_weights = vec![
        DecisionOptionWeight::new("fail", 1),
        DecisionOptionWeight::new("pass", 3),
    ];
    let sample = view.random_sample_for_operation(
        &random_decision_stream(),
        EvidenceRef::Ingress(evidence),
        "decision_selection",
        "fixture-law-vote-1",
        RandomOperationTarget::DecisionTicket {
            ticket_id: ticket.id,
            ticket_version: ticket.version,
        },
        0,
        4,
        "select one available decision option by configured weight",
    )?;
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::ResolveDecisionRandomly {
            resolution: RandomDecisionResolution {
                priority: 0,
                decision_request_id: DecisionRequestId::new(3),
                command_request_id: None,
                ticket_id: ticket.id,
                expected_version: ticket.version,
                controller_id: ticket.assigned_controller.clone(),
                sample,
                option_weights,
            },
        }],
        ..BoundaryProposal::default()
    })
}

struct RandomDecisionPlugin;

impl SimulationPlugin for RandomDecisionPlugin {
    fn name(&self) -> &'static str {
        "fixture-random-decision"
    }

    fn version(&self) -> &'static str {
        "1.0.0"
    }

    fn semantic_hash(&self) -> &'static str {
        "c1ed435888aeebc0fe3dc476fc7ea2b2343f728bf41ef752e6f4f0a93bf79735"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut contract = BoundarySystemContract::new(
            "resolve",
            BoundaryPhase::StrategicAggregation,
            SystemCadence::Daily,
        );
        contract.reads = vec![StateKey::core_decisions()];
        contract.random_streams = vec![random_decision_stream()];
        registrar.register_boundary_system(contract, resolve_ticket_randomly)
    }
}

fn rewrite_rejected_random_weights(
    snapshot: &mut SimulationSnapshot,
    rewrite: impl FnOnce(&str, &mut Vec<DecisionOptionWeight>),
) {
    let request_commitment = {
        let request = snapshot
            .ingress
            .iter_mut()
            .find_map(|record| match &mut record.payload {
                IngressPayload::Decision { request }
                    if request.request_id == DecisionRequestId::new(3) =>
                {
                    Some(request.as_mut())
                }
                _ => None,
            })
            .expect("generated random decision ingress should remain in evidence");
        let DecisionMutation::Resolve { decision, .. } = &mut request.mutation else {
            panic!("generated random decision ingress should resolve its ticket");
        };
        let DecisionOutcome::Selected { option_id } = &decision.outcome else {
            panic!("generated random decision should select an option");
        };
        let option_id = option_id.clone();
        let weights = &mut decision
            .random
            .as_mut()
            .expect("generated decision should retain random evidence")
            .option_weights;
        rewrite(&option_id, weights);
        canonical_hash(DECISION_REQUEST_COMMITMENT_DOMAIN, request)
            .expect("tampered request should still hash")
    };
    let mut decisions =
        serde_json::to_value(&snapshot.decisions).expect("decision state should serialize");
    let attempts = decisions
        .get_mut("attempts")
        .and_then(|attempts| attempts.get_mut("entries"))
        .and_then(serde_json::Value::as_object_mut)
        .expect("decision attempts should use the persistent log wire shape");
    let attempt = attempts
        .values_mut()
        .find(|attempt| attempt.get("request_id") == Some(&serde_json::json!(3)))
        .expect("rejected random decision attempt should be persisted");
    attempt["request_commitment"] = serde_json::Value::String(request_commitment);
    snapshot.decisions = serde_json::from_value(decisions)
        .expect("tampered decision state should remain structurally decodable");
    rehash_tampered_snapshot(snapshot);
}

#[test]
fn random_policy_resolution_is_generated_replayable_and_tamper_evident() {
    let (scenario, ids) = demo_scenario();
    let plugin = RandomDecisionPlugin;
    let mut simulation = Simulation::new(202, scenario).expect("demo should load");
    simulation
        .register_plugin(&plugin)
        .expect("random decision plugin should register");
    let controller = DecisionControllerBinding::new(
        "fixture-random-controller",
        DecisionPolicyIdentity::new(DecisionPolicyKind::Random, "weighted-random", "1"),
        DecisionAuthority::Actor {
            actor: ids.commander,
        },
    );
    simulation
        .enqueue_decision(
            SimTime::EPOCH,
            0,
            DecisionIngressRequest::new(
                DecisionRequestId::new(1),
                0,
                DecisionMutation::RegisterController { controller },
            ),
        )
        .expect("controller should queue");
    simulation
        .enqueue_decision(
            SimTime::EPOCH,
            0,
            DecisionIngressRequest::new(
                DecisionRequestId::new(2),
                0,
                DecisionMutation::Open {
                    ticket: DecisionTicketDraft {
                        id: TICKET_ID,
                        definition: "fixture.law-vote".to_owned(),
                        decision_maker: EntityRef::Person(ids.commander),
                        assigned_controller: "fixture-random-controller".to_owned(),
                        summary: "Decide whether the law passes".to_owned(),
                        context: DecisionContext::new(
                            "fixture.law-vote.v1",
                            serde_json::json!({"support": 75, "opposition": 25}),
                        ),
                        options: vec![
                            DecisionOption::new("pass", "Pass"),
                            DecisionOption::new("fail", "Fail"),
                        ],
                        deadline: None,
                    },
                },
            ),
        )
        .expect("ticket should queue");
    simulation
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("decision intake should settle");

    let selection_at = SimTime::EPOCH + SimDuration::days(1);
    simulation
        .schedule_calendar_boundary(selection_at, vec![SystemCadence::Daily])
        .expect("calendar boundary should queue");
    let source = simulation
        .step_canonical()
        .expect("source boundary should run")
        .expect("source boundary receipt");
    assert_eq!(source.random_draws.len(), 1);
    assert_eq!(source.generated_ingress.len(), 1);
    assert!(
        simulation
            .decision_ticket(TICKET_ID)
            .is_some_and(DecisionTicket::is_open)
    );

    let mut rejected = simulation.fork();
    rejected
        .enqueue_command(
            selection_at,
            -1,
            CommandRequest::new(
                CommandRequestId::new(1),
                rejected.revision(),
                move_order(&ids).at_time(selection_at),
            ),
        )
        .expect("interfering command ingress should queue");
    for _ in 0..2 {
        if rejected
            .decision_attempt(DecisionRequestId::new(3))
            .is_some()
        {
            break;
        }
        rejected
            .step_canonical()
            .expect("interfering boundary should settle")
            .expect("interfering boundary receipt");
    }
    let rejected_outcome = rejected
        .decision_attempt(DecisionRequestId::new(3))
        .map(|attempt| &attempt.outcome);
    assert!(
        matches!(
            rejected_outcome,
            Some(DecisionAttemptOutcome::Rejected {
                code: DecisionAttemptErrorCode::SimulationRevisionConflict,
                ..
            })
        ),
        "unexpected generated random decision outcome: {rejected_outcome:?}"
    );
    let mut tampered_rejection = rejected.snapshot();
    rewrite_rejected_random_weights(&mut tampered_rejection, |option_id, weights| {
        for weight in weights {
            weight.weight = u64::from(weight.option_id != option_id) * 4;
        }
    });
    let Err(error) = Simulation::from_snapshot_with_plugins(tampered_rejection, &[&plugin]) else {
        panic!("remapped rejected random weights must be rejected");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    let mut incomplete_rejection = rejected.snapshot();
    rewrite_rejected_random_weights(&mut incomplete_rejection, |option_id, weights| {
        weights.retain(|weight| weight.option_id == option_id);
        weights[0].weight = 4;
    });
    let Err(error) = Simulation::from_snapshot_with_plugins(incomplete_rejection, &[&plugin])
    else {
        panic!("incomplete rejected random weights must be rejected");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    simulation
        .step_canonical()
        .expect("generated decision boundary should run")
        .expect("generated decision boundary receipt");
    let trace = simulation
        .decision_trace(DecisionTraceId::new(1))
        .expect("random decision trace should exist");
    let random = trace.random.as_ref().expect("trace should cite its draw");
    assert_eq!(random.draw_id, source.random_draws[0]);
    assert_eq!(random.option_weights[0].option_id, "fail");
    let draw = simulation
        .random_draws()
        .iter()
        .find(|draw| draw.id == random.draw_id)
        .expect("cited draw should be retained");
    assert!(matches!(
        &draw.outcome,
        Some(RandomDrawOutcome::DecisionSelection {
            ticket_id,
            ticket_version: 1,
            option_id,
        }) if *ticket_id == TICKET_ID
            && matches!(&trace.outcome, DecisionOutcome::Selected { option_id: selected } if selected == option_id)
    ));

    let restored = Simulation::from_snapshot_with_plugins(simulation.snapshot(), &[&plugin])
        .expect("random decision evidence should restore");
    assert_eq!(restored.snapshot(), simulation.snapshot());
    let replayed = Simulation::replay_from_journal(&[&plugin], &simulation.replay_journal())
        .expect("random decision evidence should replay exactly");
    assert_eq!(replayed.snapshot(), simulation.snapshot());

    let mut tampered = simulation.snapshot();
    let draw = tampered
        .random_draws
        .first_mut()
        .expect("tamper fixture should contain random draw evidence");
    draw.value = (draw.value + 1) % draw.upper_exclusive;
    let Err(error) = Simulation::from_snapshot_with_plugins(tampered, &[&plugin]) else {
        panic!("tampered random decision evidence must be rejected");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
}
