use canwu_api::{
    Canwu, Command, CommandRequestId, DecisionAction, DecisionAuthority, DecisionContext,
    DecisionControllerBinding, DecisionEvaluation, DecisionIngressRequest, DecisionMutation,
    DecisionOption, DecisionPolicyIdentity, DecisionPolicyKind, DecisionRequestId,
    DecisionTicketDraft, DecisionTicketId, DemoIds, EntityRef, Scenario, SimDuration,
    UtilityProfile, WeightedUtilityPolicy,
};
use serde_json::json;
use std::collections::BTreeMap;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut canwu = Canwu::demo(1918)?;
    let replay_scenario = replay_scenario(&canwu);
    let ids = Canwu::demo_ids();

    open_aid_request(&mut canwu, ids)?;
    refresh_aid_options(&mut canwu, ids)?;
    resolve_aid_request(&mut canwu)?;

    let trace = canwu
        .decision_traces()
        .last()
        .expect("utility decision trace");
    println!("{}", serde_json::to_string_pretty(trace)?);

    verify_persistence_and_replay(&canwu, replay_scenario)?;
    println!("snapshot_restore=ok exact_replay=ok");
    Ok(())
}

fn replay_scenario(canwu: &Canwu) -> Scenario {
    let initial = canwu.snapshot();
    Scenario {
        start_time: initial.initial_time,
        world: initial.world,
        knowledge: initial.knowledge,
        domain_records: Vec::new(),
    }
}

fn open_aid_request(canwu: &mut Canwu, ids: DemoIds) -> Result<(), Box<dyn std::error::Error>> {
    let now = canwu.time();
    let controller = DecisionControllerBinding::new(
        "warlord-b-ai",
        DecisionPolicyIdentity::new(DecisionPolicyKind::Utility, "aid-utility", "1"),
        DecisionAuthority::Actor {
            actor: ids.commander,
        },
    )
    .with_command_subject(EntityRef::Army(ids.army));

    canwu.enqueue_decision(
        now,
        0,
        DecisionIngressRequest::new(
            DecisionRequestId::new(1),
            canwu.revision(),
            DecisionMutation::RegisterController { controller },
        ),
    )?;
    canwu.enqueue_decision(
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
                    summary: "Neighboring warlord requests immediate military aid".to_owned(),
                    context: DecisionContext::new(
                        "beiyang.aid-request.v1",
                        json!({
                            "requester": "warlord-a",
                            "battle": "ongoing-front",
                            "common_enemy": true
                        }),
                    ),
                    options: vec![DecisionOption {
                        action: DecisionAction::None,
                        utility_inputs: BTreeMap::from([
                            ("home_defense".to_owned(), 80),
                            ("alliance".to_owned(), -40),
                        ]),
                        ..DecisionOption::new("decline", "Decline aid")
                    }],
                    deadline: Some(now + SimDuration::days(2)),
                },
            },
        ),
    )?;
    canwu.step_canonical()?.expect("decision intake boundary");
    Ok(())
}

fn refresh_aid_options(canwu: &mut Canwu, ids: DemoIds) -> Result<(), Box<dyn std::error::Error>> {
    canwu.enqueue_decision(
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
                    json!({
                        "requester": "warlord-a",
                        "battle": "ongoing-front",
                        "common_enemy": true,
                        "route_confirmed": true
                    }),
                ),
                options: vec![
                    DecisionOption {
                        action: DecisionAction::None,
                        utility_inputs: BTreeMap::from([
                            ("home_defense".to_owned(), 80),
                            ("alliance".to_owned(), -40),
                        ]),
                        ..DecisionOption::new("decline", "Decline aid")
                    },
                    DecisionOption {
                        action: DecisionAction::Command {
                            command: serde_json::to_value(Command::OrderMovement {
                                subject: EntityRef::Army(ids.army),
                                destination: ids.eastern_territory,
                                cargo: Vec::new(),
                            })?,
                        },
                        utility_inputs: BTreeMap::from([
                            ("home_defense".to_owned(), -20),
                            ("alliance".to_owned(), 90),
                        ]),
                        ..DecisionOption::new("send-aid", "Send the neighboring army")
                    },
                ],
            },
        ),
    )?;
    canwu
        .step_canonical()?
        .expect("dynamic option refresh boundary");
    Ok(())
}

fn resolve_aid_request(canwu: &mut Canwu) -> Result<(), Box<dyn std::error::Error>> {
    let policy = WeightedUtilityPolicy::new(
        "aid-utility",
        "1",
        UtilityProfile {
            weights: BTreeMap::from([("alliance".to_owned(), 3), ("home_defense".to_owned(), 1)]),
        },
    );
    let evaluation = canwu.drive_decision(
        canwu.time(),
        0,
        DecisionRequestId::new(4),
        Some(CommandRequestId::new(1)),
        DecisionTicketId::new(1),
        &policy,
    )?;
    assert!(matches!(evaluation, DecisionEvaluation::Prepared(_)));
    canwu
        .step_canonical()?
        .expect("decision resolution boundary");
    Ok(())
}

fn verify_persistence_and_replay(
    canwu: &Canwu,
    replay_scenario: Scenario,
) -> Result<(), Box<dyn std::error::Error>> {
    let snapshot = canwu.snapshot();
    let restored = Canwu::from_snapshot_json(&serde_json::to_string(&snapshot)?)?;
    assert_eq!(restored.snapshot(), snapshot);
    let replayed = Canwu::replay_from_journal(replay_scenario, &[], &canwu.replay_journal())?;
    assert_eq!(replayed.snapshot(), snapshot);
    Ok(())
}
