use canwu_core::{DecisionRequestId, DecisionTicketId, EntityRef, PersonId};
use canwu_decision::{
    DecisionAction, DecisionAttemptOutcome, DecisionAttemptRecord, DecisionContext,
    DecisionErrorCode, DecisionOption, DecisionPolicy, DecisionState, DecisionTicket,
    DecisionTicketState, ExternalDecisionResponse, ExternalPolicy, HumanDecisionResponse,
    LlmModelIdentity, QueuedExternalPolicy, QueuedHumanPolicy, QueuedLlmPolicy,
};
use canwu_time::SimTime;
use serde_json::json;
use std::collections::BTreeMap;

fn ticket(version: u64) -> DecisionTicket {
    DecisionTicket {
        id: DecisionTicketId::new(7),
        definition: "diplomacy.answer-aid-request".to_owned(),
        decision_maker: EntityRef::Person(PersonId::new(3)),
        assigned_controller: "neighbor-controller".to_owned(),
        summary: "Choose a response to the neighboring warlord".to_owned(),
        context: DecisionContext::new("diplomacy.aid-request.v1", json!({"known": true})),
        options: vec![DecisionOption {
            description: "Send a field army".to_owned(),
            action: DecisionAction::Command {
                command: json!({"private_command": "must-not-cross-policy-boundary"}),
            },
            metadata: json!({"public_hint": "supports ally"}),
            ..DecisionOption::new("send-aid", "Send aid")
        }],
        opened_at: SimTime::EPOCH,
        updated_at: SimTime::EPOCH,
        deadline: None,
        version,
        state: DecisionTicketState::Open,
    }
}

#[test]
fn decision_attempt_index_rebuilds_from_the_persisted_journal() {
    let request_id = DecisionRequestId::new(41);
    let attempt = DecisionAttemptRecord {
        request_id,
        request_commitment: "a".repeat(64),
        at: SimTime::EPOCH,
        revision_before: 7,
        expected_revision: 7,
        outcome: DecisionAttemptOutcome::Accepted {
            trace_id: None,
            command_request_id: None,
        },
    };
    let mut state = DecisionState::default();
    state
        .append_attempt(attempt.clone())
        .expect("unique attempt should append");
    let encoded = serde_json::to_string(&state).expect("decision state should serialize");
    assert!(!encoded.contains("attempts_by_request"));

    let restored: DecisionState =
        serde_json::from_str(&encoded).expect("decision state should deserialize");
    assert_eq!(restored.attempt(request_id), Some(&attempt));
    restored.validate().expect("rebuilt index should validate");
}

#[test]
fn external_request_exposes_options_but_not_authoritative_actions() {
    let policy = QueuedExternalPolicy::new("diplomacy-service", "1");
    let request = policy.external_request(&ticket(2));
    let serialized = serde_json::to_string(&request).expect("external decision request");

    assert_eq!(request.ticket_version, 2);
    assert_eq!(request.options[0].id, "send-aid");
    assert!(serialized.contains("public_hint"));
    assert!(!serialized.contains("private_command"));
    assert!(!serialized.contains("must-not-cross-policy-boundary"));
}

#[test]
fn asynchronous_policies_reject_stale_ticket_versions() {
    let current = ticket(2);

    let mut human = QueuedHumanPolicy::new("human-seat", "1");
    human.submit(
        current.id,
        HumanDecisionResponse {
            ticket_version: 1,
            option_id: "send-aid".to_owned(),
            operator_id: "operator-9".to_owned(),
        },
    );
    assert_eq!(
        human
            .decide(&current)
            .expect_err("stale human response")
            .code,
        DecisionErrorCode::VersionConflict
    );

    let stale_external = ExternalDecisionResponse {
        ticket_version: 1,
        option_id: "send-aid".to_owned(),
        provider: "policy-service".to_owned(),
        request_id: "request-1".to_owned(),
        metadata: BTreeMap::new(),
    };
    let mut external = QueuedExternalPolicy::new("external-service", "1");
    external.submit(current.id, stale_external.clone());
    assert_eq!(
        external
            .decide(&current)
            .expect_err("stale external response")
            .code,
        DecisionErrorCode::VersionConflict
    );

    let mut llm = QueuedLlmPolicy::new(
        "llm-adapter",
        "1",
        LlmModelIdentity {
            provider: "policy-service".to_owned(),
            model: "decision-model".to_owned(),
            prompt_contract: "select-existing-option.v1".to_owned(),
        },
    );
    llm.submit(current.id, stale_external);
    assert_eq!(
        llm.decide(&current).expect_err("stale LLM response").code,
        DecisionErrorCode::VersionConflict
    );

    llm.submit(
        current.id,
        ExternalDecisionResponse {
            ticket_version: current.version,
            option_id: "send-aid".to_owned(),
            provider: "policy-service".to_owned(),
            request_id: "request-2".to_owned(),
            metadata: BTreeMap::new(),
        },
    );
    let decision = llm.decide(&current).expect("current LLM response");
    let evidence = decision.external.expect("LLM trace evidence");
    assert_eq!(evidence.model.as_deref(), Some("decision-model"));
    assert_eq!(
        evidence.prompt_contract.as_deref(),
        Some("select-existing-option.v1")
    );
}
