#![allow(clippy::unnecessary_wraps)]

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundarySystemContract,
    Canwu, CanwuError, DecisionAuthority, DecisionContext, DecisionControllerBinding,
    DecisionEvaluation, DecisionIngressRequest, DecisionMutation, DecisionOption,
    DecisionOptionWeight, DecisionPolicyIdentity, DecisionPolicyKind, DecisionRequestId,
    DecisionTicketDraft, DecisionTicketId, EntityRef, EvidenceRef, ExternalDecisionResponse,
    ExternalPolicy, LlmModelIdentity, QueuedLlmPolicy, RandomDecisionResolution,
    RandomOperationTarget, RandomStreamKey, SimDuration, SimTime, SimulationPlugin, SimulationView,
    StateKey, SystemCadence,
};
use serde_json::json;
use std::collections::BTreeMap;

const RANDOM_TICKET: DecisionTicketId = DecisionTicketId::new(1);

fn decision_stream() -> RandomStreamKey {
    RandomStreamKey::new("example-uncertainty", "decision-selection", 1)
}

fn random_resolution_system(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let ticket = view
        .decision_ticket(RANDOM_TICKET)?
        .expect("the example opens the ticket before the daily boundary");
    let calendar_ingress = context
        .admitted_ingress
        .first()
        .copied()
        .expect("the example runs this system from calendar ingress");
    let option_weights = vec![
        DecisionOptionWeight::new("fail", 25),
        DecisionOptionWeight::new("pass", 75),
    ];
    let sample = view.random_sample_for_operation(
        &decision_stream(),
        EvidenceRef::Ingress(calendar_ingress),
        "decision_selection",
        "law-proposal-42",
        RandomOperationTarget::DecisionTicket {
            ticket_id: ticket.id,
            ticket_version: ticket.version,
        },
        0,
        100,
        "select whether the law passes from configured weights",
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

struct UncertaintyPlugin;

impl SimulationPlugin for UncertaintyPlugin {
    fn name(&self) -> &'static str {
        "example-uncertainty"
    }

    fn version(&self) -> &'static str {
        "1.0.0"
    }

    fn semantic_hash(&self) -> &'static str {
        "95f702ca4cf602640094c022286fe741b47c9d7aa8573a719fce21e8df97b561"
    }

    fn register(&self, registrar: &mut canwu_api::PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut contract = BoundarySystemContract::new(
            "resolve-random-decision",
            BoundaryPhase::StrategicAggregation,
            SystemCadence::Daily,
        );
        contract.reads = vec![StateKey::core_decisions()];
        contract.random_streams = vec![decision_stream()];
        registrar.register_boundary_system(contract, random_resolution_system)
    }
}

fn enqueue_ticket(
    canwu: &mut Canwu,
    ticket_id: DecisionTicketId,
    request_offset: u64,
    controller: DecisionControllerBinding,
) -> Result<(), CanwuError> {
    let now = canwu.time();
    let controller_id = controller.id.clone();
    canwu.enqueue_decision(
        now,
        0,
        DecisionIngressRequest::new(
            DecisionRequestId::new(request_offset),
            canwu.revision(),
            DecisionMutation::RegisterController { controller },
        ),
    )?;
    canwu.enqueue_decision(
        now,
        0,
        DecisionIngressRequest::new(
            DecisionRequestId::new(request_offset + 1),
            canwu.revision(),
            DecisionMutation::Open {
                ticket: DecisionTicketDraft {
                    id: ticket_id,
                    definition: "example.law-passage".to_owned(),
                    decision_maker: EntityRef::Person(Canwu::demo_ids().commander),
                    assigned_controller: controller_id,
                    summary: "Will the proposed law pass?".to_owned(),
                    context: DecisionContext::new(
                        "example.law-passage.v1",
                        json!({
                            "supporting_seats": 72,
                            "opposing_seats": 28,
                            "public_pressure": "high"
                        }),
                    ),
                    options: vec![
                        DecisionOption::new("pass", "Pass the law"),
                        DecisionOption::new("fail", "Reject the law"),
                    ],
                    deadline: None,
                },
            },
        ),
    )?;
    canwu.settle_boundary(canwu_api::BoundaryRequest::at(now))?;
    Ok(())
}

fn random_branch() -> Result<(), Box<dyn std::error::Error>> {
    let mut canwu = Canwu::demo(202)?;
    canwu.register_plugin(&UncertaintyPlugin)?;
    enqueue_ticket(
        &mut canwu,
        RANDOM_TICKET,
        1,
        DecisionControllerBinding::new(
            "law-random-controller",
            DecisionPolicyIdentity::new(DecisionPolicyKind::Random, "weighted-random", "1"),
            DecisionAuthority::Actor {
                actor: Canwu::demo_ids().commander,
            },
        ),
    )?;
    let selection_at = SimTime::EPOCH + SimDuration::days(1);
    canwu.schedule_calendar_boundary(selection_at, vec![SystemCadence::Daily])?;
    canwu.step_canonical()?.expect("random source boundary");
    canwu
        .step_canonical()?
        .expect("generated decision resolution boundary");
    let trace = canwu
        .decision_trace(canwu_api::DecisionTraceId::new(1))
        .expect("random decision trace");
    println!("random_trace={}", serde_json::to_string(trace)?);
    Ok(())
}

fn llm_interface_branch() -> Result<(), Box<dyn std::error::Error>> {
    let mut canwu = Canwu::demo(202)?;
    let ticket_id = DecisionTicketId::new(2);
    enqueue_ticket(
        &mut canwu,
        ticket_id,
        10,
        DecisionControllerBinding::new(
            "law-llm-controller",
            DecisionPolicyIdentity::new(DecisionPolicyKind::Llm, "strict-law-selector", "1"),
            DecisionAuthority::Actor {
                actor: Canwu::demo_ids().commander,
            },
        ),
    )?;
    let mut policy = QueuedLlmPolicy::new(
        "strict-law-selector",
        "1",
        LlmModelIdentity {
            provider: "not-connected".to_owned(),
            model: "host-selected-model".to_owned(),
            prompt_contract: "return one existing option_id and no new action".to_owned(),
        },
    );
    let ticket = canwu.decision_ticket(ticket_id).expect("LLM ticket");
    let request = policy.external_request(ticket);
    println!("llm_request={}", serde_json::to_string(&request)?);

    // A real host sends `request` to its model and parses a strict structured
    // answer. This example supplies the same response object without making a
    // network call.
    policy.submit(
        ticket_id,
        ExternalDecisionResponse {
            ticket_version: request.ticket_version,
            option_id: "pass".to_owned(),
            provider: "not-connected".to_owned(),
            request_id: "example-response-1".to_owned(),
            metadata: BTreeMap::new(),
        },
    )?;
    let evaluation = canwu.drive_decision(
        canwu.time(),
        0,
        DecisionRequestId::new(12),
        None,
        ticket_id,
        &policy,
    )?;
    assert!(matches!(evaluation, DecisionEvaluation::Prepared(_)));
    canwu.step_canonical()?.expect("LLM decision boundary");
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    random_branch()?;
    llm_interface_branch()?;
    Ok(())
}
