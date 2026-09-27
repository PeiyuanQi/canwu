//! Public-API fixture for gap G-26: a seized carrier terminates the delivery
//! attempt with a terminal seizure handoff, and the carrier holder receives a
//! report that the attempt ended by seizure.

#![allow(clippy::too_many_lines)]

#[path = "../examples/support/mod.rs"]
mod support;

use canwu_api::{
    Canwu, CommandAttemptOutcome, CommandAuthority, CommandEnvelope, CommandRequest,
    CommandRequestId, DecisionAuthority, DecisionControllerBinding, DecisionEvaluation,
    DecisionIngressRequest, DecisionMutation, DecisionOrigin, DecisionPolicyIdentity,
    DecisionPolicyKind, DecisionRequestId, DecisionTicketId, DomainRecordRef, EntityRef, ErrorCode,
    Handoff, HandoffId, HandoffKind, Issuer, KnowledgeHolderRef, KnowledgeQuery,
    LegExecutionStatus, PluginIngressRequest, RoutingNodeRef, RoutingPolicy, SimDuration,
    SimulationPlugin, TransportExecutionId, TransportExecutionState, TypedDomainRecordRef,
    UtilityProfile, WeightedUtilityPolicy,
};
use canwu_correspondence::{
    ATTEMPT_REPORT_KNOWLEDGE_SCHEMA, CARRY_CORRESPONDENCE_CAPABILITY, CarrierDelegationRequest,
    CorrespondenceAttemptOutcome, CorrespondenceAttemptReport, CorrespondenceCapacityAdmission,
    CorrespondenceIncidentKind, CorrespondenceIncidentRequest, CorrespondenceOperation,
    CorrespondenceOperationRecord, CorrespondencePlugin, CorrespondenceStatus, INCIDENT_INGRESS,
    InitiateCorrespondenceRequest, KNOWLEDGE_INGRESS, PLUGIN_NAME, carrier_delegation_command,
    correspondence_decision_ticket, correspondence_knowledge_schemas, correspondence_operation_ref,
};
use canwu_information::{
    DelegationClaimV1, DeliveryAttempt, DeliveryAttemptStatus, Dispatch, DispatchStatus,
    InformationOperationId, InformationPlugin,
};
use std::collections::BTreeMap;
use support::{network_seed, scenario_with_prepared_dispatch};

const KEY: &str = "escorted-letter";

fn escort_entity() -> EntityRef {
    EntityRef::Army(Canwu::demo_ids().army)
}

fn load(canwu: &Canwu) -> CorrespondenceOperation {
    canwu
        .typed_domain_record(&correspondence_operation_ref(KEY))
        .unwrap()
        .decode_payload::<CorrespondenceOperationRecord>()
        .unwrap()
}

fn step_until(canwu: &mut Canwu, done: impl Fn(&CorrespondenceOperation) -> bool) {
    for _ in 0..80 {
        if canwu
            .typed_domain_record(&correspondence_operation_ref(KEY))
            .is_some()
            && done(&load(canwu))
        {
            return;
        }
        canwu.step_canonical().unwrap().unwrap();
    }
    panic!("correspondence did not reach the expected state");
}

fn seizure(custody_handoff: u64, seized_by: &EntityRef) -> CorrespondenceIncidentRequest {
    CorrespondenceIncidentRequest {
        operation_key: KEY.to_owned(),
        incident_key: format!("seizure-{custody_handoff}"),
        probability_per_mille: 1_000,
        kind: CorrespondenceIncidentKind::CarrierSeized {
            seized_by: seized_by.clone(),
            custody_handoff: HandoffId(custody_handoff),
        },
    }
}

fn enqueue_incident(canwu: &mut Canwu, request: &CorrespondenceIncidentRequest) {
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            PLUGIN_NAME,
            INCIDENT_INGRESS,
            canwu.time() + SimDuration::minutes(1),
            serde_json::to_value(request).unwrap(),
        ))
        .unwrap();
}

fn reports(canwu: &Canwu, holder: &KnowledgeHolderRef) -> Vec<CorrespondenceAttemptReport> {
    let schema = correspondence_knowledge_schemas()
        .into_iter()
        .find(|schema| schema.id.kind.name == ATTEMPT_REPORT_KNOWLEDGE_SCHEMA)
        .unwrap()
        .id;
    canwu
        .admin_query_knowledge(
            holder.clone(),
            &KnowledgeQuery {
                schemas: vec![schema],
                ..KnowledgeQuery::default()
            },
        )
        .unwrap()
        .records
        .into_iter()
        .map(|record| serde_json::from_value(record.payload).unwrap())
        .collect()
}

/// Starts a long-distance letter carried by an army escort under the
/// sender's delegation, and runs it until the escort has handed the letter
/// from the railway to the final-mile road.
fn start() -> (
    Canwu,
    KnowledgeHolderRef,
    KnowledgeHolderRef,
    KnowledgeHolderRef,
) {
    let plugins: &[&dyn SimulationPlugin] = &[&InformationPlugin, &CorrespondencePlugin];
    let (scenario, sender, recipient, prepared_dispatch) = scenario_with_prepared_dispatch();
    let mut canwu = Canwu::new_with_plugins(2026, scenario, plugins).unwrap();
    let escort = KnowledgeHolderRef::Entity(EntityRef::Army(Canwu::demo_ids().army));
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            PLUGIN_NAME,
            KNOWLEDGE_INGRESS,
            canwu.time(),
            serde_json::to_value(network_seed(
                escort.clone(),
                recipient.clone(),
                RoutingNodeRef::new("beijing/delivery/recipient"),
                true,
            ))
            .unwrap(),
        ))
        .unwrap();
    let now = canwu.time();
    canwu
        .enqueue_decision(
            now,
            0,
            DecisionIngressRequest::new(
                DecisionRequestId::new(1),
                canwu.revision(),
                DecisionMutation::RegisterController {
                    controller: DecisionControllerBinding::new(
                        "sender-policy",
                        DecisionPolicyIdentity::new(
                            DecisionPolicyKind::Utility,
                            "send-policy",
                            "1",
                        ),
                        DecisionAuthority::Actor { actor: sender },
                    ),
                },
            ),
        )
        .unwrap();
    canwu.step_canonical().unwrap().unwrap();
    // The escort accepts carrying for the sender under its own institution
    // authority.
    canwu
        .enqueue_command(
            now,
            0,
            CommandRequest::new(
                CommandRequestId::new(100),
                canwu.revision(),
                CommandEnvelope::new(
                    Issuer::Institution("escort-command".to_owned()),
                    carrier_delegation_command(&CarrierDelegationRequest {
                        claim: DelegationClaimV1 {
                            format_version: 1,
                            performed_by: EntityRef::Army(Canwu::demo_ids().army),
                            performed_for: KnowledgeHolderRef::Person(sender),
                            capabilities: vec![CARRY_CORRESPONDENCE_CAPABILITY.to_owned()],
                            not_before: None,
                            expires_at: None,
                        },
                    })
                    .unwrap(),
                )
                .at_time(now)
                .with_authority(CommandAuthority {
                    decision_origin: DecisionOrigin::Institution {
                        institution: EntityRef::Army(Canwu::demo_ids().army),
                        responsible_actor: None,
                    },
                    seat_id: None,
                    permission_profile_id: None,
                    command_subject: None,
                }),
            ),
        )
        .unwrap();
    canwu.step_canonical().unwrap().unwrap();
    let CommandAttemptOutcome::Accepted {
        command_id: delegation,
    } = canwu.command_attempts().last().unwrap().outcome
    else {
        panic!("the escort delegation must be accepted");
    };
    let request = InitiateCorrespondenceRequest {
        operation_key: KEY.to_owned(),
        sender: EntityRef::Person(sender),
        recipient: recipient.clone(),
        carrier: escort.clone(),
        channel_profile: "sealed-letter".to_owned(),
        origin: RoutingNodeRef::new("wuxi/hub"),
        due_at: now + SimDuration::days(10),
        prepared_dispatch,
        delivery_attempt_operation: InformationOperationId::new(
            "fixture.correspondence",
            "escorted-attempt",
        ),
        routing_policy: RoutingPolicy::default(),
        capacity_admission: CorrespondenceCapacityAdmission::Unconstrained,
        execution_id: TransportExecutionId(26),
        automatic_opportunity: None,
        carrier_delegation: Some(delegation),
    };
    let mut ticket = correspondence_decision_ticket(
        DecisionTicketId::new(1),
        EntityRef::Person(sender),
        "sender-policy",
        "Send the letter with the escort",
        Some(request.due_at),
        &request,
    )
    .unwrap();
    for option in &mut ticket.options {
        option
            .utility_inputs
            .insert("send".to_owned(), if option.id == "send" { 100 } else { 0 });
    }
    canwu
        .enqueue_decision(
            now,
            0,
            DecisionIngressRequest::new(
                DecisionRequestId::new(2),
                canwu.revision(),
                DecisionMutation::Open { ticket },
            ),
        )
        .unwrap();
    canwu.step_canonical().unwrap().unwrap();
    let policy = WeightedUtilityPolicy::new(
        "send-policy",
        "1",
        UtilityProfile {
            weights: BTreeMap::from([("send".to_owned(), 1)]),
        },
    );
    assert!(matches!(
        canwu
            .drive_decision(
                now,
                0,
                DecisionRequestId::new(3),
                Some(CommandRequestId::new(1)),
                DecisionTicketId::new(1),
                &policy,
            )
            .unwrap(),
        DecisionEvaluation::Prepared(_)
    ));
    canwu.step_canonical().unwrap().unwrap();
    assert!(matches!(
        canwu.command_attempts().last().unwrap().outcome,
        CommandAttemptOutcome::Accepted { .. }
    ));
    step_until(&mut canwu, |operation| {
        operation.status == CorrespondenceStatus::Scheduled
            && !operation.execution.handoffs.is_empty()
    });
    (canwu, KnowledgeHolderRef::Person(sender), recipient, escort)
}

#[test]
fn gap_g26_correspondence_carrier_seized() {
    let plugins: &[&dyn SimulationPlugin] = &[&InformationPlugin, &CorrespondencePlugin];
    let (mut canwu, sender, recipient, escort) = start();
    let band = EntityRef::Domain(DomainRecordRef::new("fixture.world", "band", "river-band"));
    assert!(load(&canwu).execution.handoffs[0].kind.is_planned());

    // A seizure citing the planned railway handoff is rejected without
    // stalling the run: it is kept as suppressed evidence, and the attempt
    // continues on the final-mile road.
    enqueue_incident(&mut canwu, &seizure(1, &band));
    step_until(&mut canwu, |operation| {
        operation.incidents.contains_key("seizure-1")
    });
    let operation = load(&canwu);
    let mismatched = &operation.incidents["seizure-1"];
    assert!(!mismatched.triggered);
    assert!(
        mismatched
            .suppressed_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("already recorded"))
    );
    assert_eq!(operation.status, CorrespondenceStatus::InTransit);
    assert_eq!(operation.execution.handoffs.len(), 1);

    // A seizure that names the escort itself as the seizing party is
    // malformed and rejected outright.
    let mut malformed = canwu.fork();
    enqueue_incident(&mut malformed, &seizure(2, &escort_entity()));
    let error = (0..5)
        .find_map(|_| malformed.step_canonical().err())
        .expect("a self-seizure must be rejected");
    assert_eq!(error.code, ErrorCode::InvalidDomainRecord);

    // The seizure on the final-mile road terminates the attempt.
    enqueue_incident(&mut canwu, &seizure(2, &band));
    step_until(&mut canwu, |operation| operation.status.is_terminal());
    let operation = load(&canwu);
    assert_eq!(operation.status, CorrespondenceStatus::Failed);
    let incident = &operation.incidents["seizure-2"];
    assert!(incident.triggered);
    let seized: &Handoff = &operation.execution.handoffs[1];
    assert_eq!(seized.id, HandoffId(2));
    assert_eq!(seized.kind, HandoffKind::Seizure { by: band.clone() });
    assert_eq!(seized.from_leg, seized.to_leg);
    let leg = operation
        .execution
        .legs
        .iter()
        .find(|leg| leg.id == seized.from_leg)
        .unwrap();
    assert_eq!(leg.status, LegExecutionStatus::Failed);
    assert_eq!(leg.failed_at, Some(incident.at));
    assert_eq!(operation.execution.state, TransportExecutionState::Failed);
    let attempt = operation.execution.delivery_attempt.as_ref().unwrap();
    let attempt = canwu
        .typed_domain_record(
            &TypedDomainRecordRef::<DeliveryAttempt>::from_untyped(attempt.record.clone()).unwrap(),
        )
        .unwrap()
        .decode_payload::<DeliveryAttempt>()
        .unwrap();
    assert_eq!(attempt.status, DeliveryAttemptStatus::Failed);
    assert_eq!(attempt.completed_at, Some(incident.at));
    let dispatch = canwu
        .typed_domain_record(
            &TypedDomainRecordRef::<Dispatch>::from_untyped(operation.dispatch.record.clone())
                .unwrap(),
        )
        .unwrap()
        .decode_payload::<Dispatch>()
        .unwrap();
    assert_eq!(dispatch.status, DispatchStatus::Active);

    // The escort that was seized learns that the attempt ended by seizure;
    // the delegating sender and the recipient are not told by the engine.
    canwu.step_canonical().unwrap().unwrap();
    assert_eq!(
        reports(&canwu, &escort),
        [CorrespondenceAttemptReport {
            operation_key: KEY.to_owned(),
            attempt_number: 1,
            ended_at: incident.at,
            outcome: CorrespondenceAttemptOutcome::CarrierSeized {
                custody_handoff: HandoffId(2),
            },
        }]
    );
    assert!(reports(&canwu, &sender).is_empty());
    assert!(reports(&canwu, &recipient).is_empty());

    // The host cannot author a report of its own.
    assert_eq!(
        canwu
            .enqueue_plugin_ingress(PluginIngressRequest::new(
                PLUGIN_NAME,
                "correspondence_attempt_report_v1",
                canwu.time(),
                serde_json::json!({}),
            ))
            .unwrap_err()
            .code,
        ErrorCode::InvalidAuthority
    );

    let restored =
        Canwu::from_snapshot_json_with_plugins(&canwu.snapshot_json().unwrap(), plugins).unwrap();
    assert_eq!(restored.snapshot(), canwu.snapshot());
    let replayed = Canwu::replay_from_journal(plugins, &canwu.replay_journal()).unwrap();
    assert_eq!(replayed.snapshot(), canwu.snapshot());
}
