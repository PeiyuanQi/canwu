use canwu_api::{
    Canwu, Command, CommandRequestId, DecisionAction, DecisionAttemptErrorCode,
    DecisionAttemptOutcome, DecisionAuthority, DecisionContext, DecisionControllerBinding,
    DecisionEvaluation, DecisionIngressRequest, DecisionMutation, DecisionOption, DecisionOutcome,
    DecisionPolicyIdentity, DecisionPolicyKind, DecisionRequestId, DecisionTicketDraft,
    DecisionTicketId, EntityRef, ErrorCode, Scenario, SimDuration, UtilityProfile,
    WeightedUtilityPolicy,
};
use serde_json::json;
use std::collections::BTreeMap;

#[test]
#[allow(clippy::too_many_lines)]
fn utility_decision_is_persisted_and_exactly_replayed_without_rerunning_policy() {
    let mut canwu = Canwu::demo(1918).expect("demo");
    let initial = canwu.snapshot();
    let scenario = Scenario {
        start_time: initial.initial_time,
        world: initial.world,
        knowledge: initial.knowledge,
        domain_records: Vec::new(),
    };
    let ids = Canwu::demo_ids();
    let controller = DecisionControllerBinding::new(
        "warlord-b-ai",
        DecisionPolicyIdentity::new(DecisionPolicyKind::Utility, "aid-utility", "1"),
        DecisionAuthority::Actor {
            actor: ids.commander,
        },
    )
    .with_command_subject(EntityRef::Army(ids.army));
    let revision = canwu.revision();
    let now = canwu.time();
    canwu
        .enqueue_decision(
            now,
            0,
            DecisionIngressRequest::new(
                DecisionRequestId::new(1),
                revision,
                DecisionMutation::RegisterController { controller },
            ),
        )
        .expect("controller ingress");
    let open_request = DecisionIngressRequest::new(
        DecisionRequestId::new(2),
        revision,
        DecisionMutation::Open {
            ticket: DecisionTicketDraft {
                id: DecisionTicketId::new(1),
                definition: "beiyang.request-military-aid".to_owned(),
                decision_maker: EntityRef::Person(ids.commander),
                assigned_controller: "warlord-b-ai".to_owned(),
                summary: "Neighboring warlord requests immediate military aid".to_owned(),
                context: DecisionContext::new(
                    "beiyang.aid-request.v1",
                    json!({"common_enemy": true}),
                ),
                options: vec![DecisionOption {
                    action: DecisionAction::None,
                    utility_inputs: BTreeMap::from([("alliance".to_owned(), -10)]),
                    ..DecisionOption::new("decline", "Decline aid")
                }],
                deadline: None,
            },
        },
    );
    let open_receipt = canwu
        .enqueue_decision(now, 0, open_request.clone())
        .expect("ticket ingress");
    canwu
        .step_canonical()
        .expect("intake")
        .expect("intake boundary");
    assert_eq!(
        canwu
            .enqueue_decision(now, 0, open_request.clone())
            .expect("exact decision retry"),
        open_receipt
    );
    canwu
        .enqueue_decision(
            canwu.time(),
            0,
            DecisionIngressRequest::new(
                DecisionRequestId::new(3),
                canwu.revision(),
                DecisionMutation::ReplaceOptions {
                    ticket_id: DecisionTicketId::new(1),
                    expected_version: 1,
                    context: DecisionContext::new(
                        "beiyang.aid-request.v1",
                        json!({"common_enemy": true, "route_confirmed": true}),
                    ),
                    options: vec![
                        DecisionOption {
                            action: DecisionAction::None,
                            utility_inputs: BTreeMap::from([("alliance".to_owned(), -10)]),
                            ..DecisionOption::new("decline", "Decline aid")
                        },
                        DecisionOption {
                            action: DecisionAction::Command {
                                command: serde_json::to_value(Command::MoveArmy {
                                    army: ids.army,
                                    destination: ids.eastern_territory,
                                })
                                .expect("command value"),
                            },
                            utility_inputs: BTreeMap::from([("alliance".to_owned(), 50)]),
                            ..DecisionOption::new("send-aid", "Send aid")
                        },
                    ],
                },
            ),
        )
        .expect("dynamic option refresh");
    canwu
        .step_canonical()
        .expect("option refresh")
        .expect("option refresh boundary");

    let policy = WeightedUtilityPolicy::new(
        "aid-utility",
        "1",
        UtilityProfile {
            weights: BTreeMap::from([("alliance".to_owned(), 2)]),
        },
    );
    let future_due = canwu.time() + SimDuration::days(1);
    let future = canwu
        .prepare_decision_at(
            future_due,
            DecisionRequestId::new(99),
            Some(CommandRequestId::new(99)),
            DecisionTicketId::new(1),
            &policy,
        )
        .expect("prepare future decision");
    let DecisionEvaluation::Prepared(future) = future else {
        panic!("utility policy should prepare a future resolution");
    };
    assert_eq!(
        future
            .request
            .command
            .as_ref()
            .and_then(|request| request.envelope.expected_time),
        Some(future_due)
    );
    canwu
        .enqueue_decision(future_due, 0, future.request.clone())
        .expect("enqueue future decision");
    let evaluation = canwu
        .drive_decision(
            canwu.time(),
            0,
            DecisionRequestId::new(4),
            Some(CommandRequestId::new(1)),
            DecisionTicketId::new(1),
            &policy,
        )
        .expect("drive decision");
    assert!(matches!(evaluation, DecisionEvaluation::Prepared(_)));
    canwu
        .step_canonical()
        .expect("resolution")
        .expect("resolution boundary");

    while canwu.decision_attempts().len() < 5 {
        canwu
            .step_canonical()
            .expect("advance to future decision")
            .expect("scheduled boundary");
    }
    assert!(matches!(
        canwu
            .decision_attempts()
            .last()
            .map(|attempt| &attempt.outcome),
        Some(DecisionAttemptOutcome::Rejected {
            code: DecisionAttemptErrorCode::SimulationRevisionConflict,
            ..
        })
    ));

    let trace = canwu.decision_traces().last().expect("trace");
    assert_eq!(
        trace.outcome,
        DecisionOutcome::Selected {
            option_id: "send-aid".to_owned()
        }
    );
    assert_eq!(trace.evaluations.len(), 2);
    let snapshot = canwu.snapshot();
    let restored =
        Canwu::from_snapshot_json(&serde_json::to_string(&snapshot).expect("snapshot json"))
            .expect("snapshot restore");
    assert_eq!(restored.snapshot(), snapshot);

    let journal = canwu.replay_journal();
    let replayed = Canwu::replay_from_journal(scenario, &[], &journal).expect("exact replay");
    assert_eq!(replayed.snapshot(), snapshot);

    let mut compact = canwu.into_compacted().expect("compact decision runtime");
    let segment = compact
        .seal_evidence()
        .expect("seal decision evidence")
        .expect("decision evidence segment");
    assert_eq!(
        compact
            .enqueue_command(
                future_due,
                0,
                *future.request.command.clone().expect("future command"),
            )
            .expect_err("sealed rejected decision keeps its nested command ID reserved")
            .code,
        ErrorCode::IdempotencyConflict
    );
    assert_eq!(
        compact
            .enqueue_decision(now, 0, open_request)
            .expect("archived decision retry"),
        open_receipt
    );
    assert_eq!(
        compact
            .snapshot_with_segments(vec![segment])
            .expect("reconstruct decision snapshot"),
        snapshot
    );
}

#[test]
fn conflicting_decision_mutations_are_persisted_rejections_without_poisoning_the_queue() {
    let mut canwu = Canwu::demo(1918).expect("demo");
    let ids = Canwu::demo_ids();
    let now = canwu.time();
    let controller = DecisionControllerBinding::new(
        "warlord-b-ai",
        DecisionPolicyIdentity::new(DecisionPolicyKind::Utility, "aid-utility", "1"),
        DecisionAuthority::Actor {
            actor: ids.commander,
        },
    );
    canwu
        .enqueue_decision(
            now,
            0,
            DecisionIngressRequest::new(
                DecisionRequestId::new(1),
                canwu.revision(),
                DecisionMutation::RegisterController { controller },
            ),
        )
        .expect("controller");
    canwu
        .enqueue_decision(
            now,
            0,
            DecisionIngressRequest::new(
                DecisionRequestId::new(2),
                canwu.revision(),
                DecisionMutation::Open {
                    ticket: DecisionTicketDraft {
                        id: DecisionTicketId::new(1),
                        definition: "beiyang.request-military-aid".to_owned(),
                        decision_maker: EntityRef::Person(ids.commander),
                        assigned_controller: "warlord-b-ai".to_owned(),
                        summary: "Aid request".to_owned(),
                        context: DecisionContext::new("beiyang.aid-request.v1", json!({})),
                        options: vec![DecisionOption::new("decline", "Decline")],
                        deadline: None,
                    },
                },
            ),
        )
        .expect("ticket");
    canwu.step_canonical().expect("open").expect("boundary");

    for request_id in [3, 4] {
        canwu
            .enqueue_decision(
                canwu.time(),
                0,
                DecisionIngressRequest::new(
                    DecisionRequestId::new(request_id),
                    canwu.revision(),
                    DecisionMutation::ReplaceOptions {
                        ticket_id: DecisionTicketId::new(1),
                        expected_version: 1,
                        context: DecisionContext::new(
                            "beiyang.aid-request.v1",
                            json!({"request": request_id}),
                        ),
                        options: vec![DecisionOption::new("decline", "Decline")],
                    },
                ),
            )
            .expect("conflicting option replacement queues");
    }
    canwu
        .step_canonical()
        .expect("conflict boundary")
        .expect("boundary must make progress");
    assert_eq!(
        canwu
            .decision_ticket(DecisionTicketId::new(1))
            .unwrap()
            .version,
        2
    );
    assert!(matches!(
        canwu
            .decision_attempts()
            .last()
            .map(|attempt| &attempt.outcome),
        Some(DecisionAttemptOutcome::Rejected {
            code: DecisionAttemptErrorCode::VersionConflict,
            ..
        })
    ));
    let snapshot = canwu.snapshot();
    let restored = Canwu::from_snapshot_json(
        &serde_json::to_string(&snapshot).expect("serialize rejected decision"),
    )
    .expect("restore rejected decision");
    assert_eq!(restored.snapshot(), snapshot);
}

#[test]
#[allow(clippy::too_many_lines)]
fn decision_and_command_request_ids_fail_closed_before_persistence() {
    let mut canwu = Canwu::demo(1918).expect("demo");
    let zero = canwu
        .enqueue_decision(
            canwu.time(),
            0,
            DecisionIngressRequest::new(
                DecisionRequestId::new(0),
                canwu.revision(),
                DecisionMutation::Cancel {
                    ticket_id: DecisionTicketId::new(1),
                    expected_version: 1,
                    reason: "invalid fixture".to_owned(),
                },
            ),
        )
        .expect_err("zero decision request ID");
    assert_eq!(zero.code, ErrorCode::InvalidDecision);

    let ids = Canwu::demo_ids();
    let controller = DecisionControllerBinding::new(
        "warlord-b-ai",
        DecisionPolicyIdentity::new(DecisionPolicyKind::Utility, "aid-utility", "1"),
        DecisionAuthority::Actor {
            actor: ids.commander,
        },
    )
    .with_command_subject(EntityRef::Army(ids.army));
    for mutation in [
        DecisionMutation::RegisterController { controller },
        DecisionMutation::Open {
            ticket: DecisionTicketDraft {
                id: DecisionTicketId::new(1),
                definition: "beiyang.request-military-aid".to_owned(),
                decision_maker: EntityRef::Person(ids.commander),
                assigned_controller: "warlord-b-ai".to_owned(),
                summary: "Aid request".to_owned(),
                context: DecisionContext::new("beiyang.aid-request.v1", json!({})),
                options: vec![DecisionOption {
                    action: DecisionAction::Command {
                        command: serde_json::to_value(Command::MoveArmy {
                            army: ids.army,
                            destination: ids.eastern_territory,
                        })
                        .expect("command"),
                    },
                    utility_inputs: BTreeMap::from([("alliance".to_owned(), 1)]),
                    ..DecisionOption::new("send-aid", "Send aid")
                }],
                deadline: None,
            },
        },
    ] {
        let request_id = DecisionRequestId::new(canwu.ingress_log().len() as u64 + 1);
        canwu
            .enqueue_decision(
                canwu.time(),
                0,
                DecisionIngressRequest::new(request_id, canwu.revision(), mutation),
            )
            .expect("setup decision");
    }
    canwu.step_canonical().expect("setup").expect("boundary");
    let policy = WeightedUtilityPolicy::new(
        "aid-utility",
        "1",
        UtilityProfile {
            weights: BTreeMap::from([("alliance".to_owned(), 1)]),
        },
    );
    let due_at = canwu.time() + SimDuration::days(1);
    let DecisionEvaluation::Prepared(first) = canwu
        .prepare_decision_at(
            due_at,
            DecisionRequestId::new(3),
            Some(CommandRequestId::new(10)),
            DecisionTicketId::new(1),
            &policy,
        )
        .expect("prepare command-first collision")
    else {
        panic!("prepared");
    };
    canwu
        .enqueue_command(
            due_at,
            0,
            *first.request.command.clone().expect("nested command"),
        )
        .expect("reserve command ID");
    assert_eq!(
        canwu
            .enqueue_decision(due_at, 0, first.request)
            .expect_err("decision must not reuse queued command ID")
            .code,
        ErrorCode::IdempotencyConflict
    );

    let DecisionEvaluation::Prepared(second) = canwu
        .prepare_decision_at(
            due_at,
            DecisionRequestId::new(4),
            Some(CommandRequestId::new(11)),
            DecisionTicketId::new(1),
            &policy,
        )
        .expect("prepare decision-first collision")
    else {
        panic!("prepared");
    };
    let nested = *second.request.command.clone().expect("nested command");
    canwu
        .enqueue_decision(due_at, 0, second.request)
        .expect("reserve nested command ID");
    assert_eq!(
        canwu
            .enqueue_command(due_at, 0, nested)
            .expect_err("command must not reuse decision command ID")
            .code,
        ErrorCode::IdempotencyConflict
    );
    Canwu::from_snapshot_json(
        &serde_json::to_string(&canwu.snapshot()).expect("serialize pending ingress"),
    )
    .expect("pending ingress remains loadable");
}
