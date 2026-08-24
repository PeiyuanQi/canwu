#![allow(clippy::too_many_lines)]

#[path = "../examples/support/mod.rs"]
mod support;

use canwu_api::{
    Canwu, CommandAuthority, CommandEnvelope, CommandRequest, CommandRequestId, DecisionAuthority,
    DecisionControllerBinding, DecisionEvaluation, DecisionIngressRequest, DecisionMutation,
    DecisionPolicyIdentity, DecisionPolicyKind, DecisionRequestId, DecisionTicketId,
    DomainRecordKind, EntityRef, ErrorCode, Issuer, KnowledgeHolderRef, PluginIngressRequest,
    RoutingConnection, RoutingConnectionRef, RoutingNodeRef, SimDuration, SimulationPlugin,
    TransferMode, TransportExecutionId, TraversalModel, TypedDomainRecordRef, UtilityProfile,
    WeightedUtilityPolicy,
};
use canwu_correspondence::{
    CommunicationOpportunityRecord, CommunicationOpportunityRequest,
    CommunicationOpportunityStatus, CorrespondenceCapacityAdmission, CorrespondenceIncidentKind,
    CorrespondenceIncidentRequest, CorrespondenceOperationRecord, CorrespondencePlugin,
    CorrespondenceRecoveryAction, CorrespondenceStatus, InitiateCorrespondenceRequest,
    KNOWLEDGE_INGRESS, KnownRoutingConnection, OPPORTUNITY_INGRESS, PLUGIN_NAME, ProgressAction,
    ProgressRequest, ResolveCorrespondenceRequest, correspondence_command,
    correspondence_decision_ticket, correspondence_operation_ref,
    correspondence_recovery_decision_ticket, opportunity_ref,
};
use canwu_information::{
    Access, DeliveryAttempt, DeliveryAttemptStatus, Dispatch, DispatchStatus,
    InformationOperationRecord, InformationOperationStatus, InformationPlugin,
    derive_operation_record_ref,
};
use std::collections::BTreeMap;
use support::{network_seed, scenario_with_prepared_dispatch};

#[derive(Clone, Copy)]
enum IncidentCase {
    None,
    Disaster,
    DisasterAfterHandoff,
    Interception,
}

#[test]
fn local_and_long_distance_correspondence_share_one_delivery_contract() {
    let local = run_delivery_case(false, IncidentCase::None, SimDuration::days(10));
    assert_eq!(local.route_plan.legs.len(), 1);
    assert_eq!(local.route_plan.legs[0].mode, TransferMode::Horse);
    assert_eq!(local.status, CorrespondenceStatus::Settled);

    let long = run_delivery_case(true, IncidentCase::None, SimDuration::days(10));
    assert_eq!(long.route_plan.legs.len(), 2);
    assert_eq!(long.route_plan.legs[0].mode, TransferMode::Rail);
    assert_eq!(long.route_plan.legs[1].mode, TransferMode::Horse);
    assert_eq!(long.execution.handoffs.len(), 1);
    assert_eq!(long.execution.handoffs[0].location, "beijing/station");
    assert_eq!(long.status, CorrespondenceStatus::Settled);
}

#[test]
fn disaster_reroutes_the_same_delivery_attempt_and_replays_exactly() {
    let operation = run_delivery_case(true, IncidentCase::Disaster, SimDuration::days(10));
    assert_eq!(operation.status, CorrespondenceStatus::Settled);
    assert_eq!(operation.execution.revisions.len(), 2);
    assert_eq!(
        operation
            .execution
            .delivery_attempt
            .as_ref()
            .unwrap()
            .version,
        3
    );
    assert_eq!(operation.incidents.len(), 1);
    assert!(operation.incidents["flood-1"].triggered);
    assert_eq!(operation.planning_history.len(), 2);
}

#[test]
fn disaster_after_handoff_preserves_prior_leg_and_handoff_evidence() {
    let operation = run_delivery_case(
        true,
        IncidentCase::DisasterAfterHandoff,
        SimDuration::days(10),
    );
    assert_eq!(operation.status, CorrespondenceStatus::Settled);
    assert_eq!(operation.execution.revisions.len(), 2);
    assert_eq!(operation.planning_history.len(), 2);
    assert!(operation.execution.legs.iter().any(|leg| {
        leg.status == canwu_api::LegExecutionStatus::Failed
            && leg.itinerary_revision == canwu_api::ItineraryRevisionId(1)
    }));
    let failed_leg = operation
        .execution
        .legs
        .iter()
        .find(|leg| {
            leg.status == canwu_api::LegExecutionStatus::Failed
                && leg.itinerary_revision == canwu_api::ItineraryRevisionId(1)
        })
        .unwrap();
    assert!(failed_leg.failed_at.is_some());
    assert!(operation.execution.handoffs.iter().any(|handoff| {
        handoff.from_leg == failed_leg.id
            && operation.execution.legs.iter().any(|leg| {
                leg.id == handoff.to_leg
                    && leg.itinerary_revision == canwu_api::ItineraryRevisionId(2)
            })
    }));
    for handoff in &operation.execution.handoffs {
        assert!(
            operation
                .execution
                .legs
                .iter()
                .any(|leg| leg.id == handoff.from_leg)
        );
        assert!(
            operation
                .execution
                .legs
                .iter()
                .any(|leg| leg.id == handoff.to_leg)
        );
    }
    assert!(operation.execution.handoffs.iter().any(|handoff| {
        handoff.from_leg
            == operation
                .execution
                .legs
                .iter()
                .find(|leg| {
                    leg.itinerary_revision == canwu_api::ItineraryRevisionId(1)
                        && leg.status == canwu_api::LegExecutionStatus::Failed
                })
                .map(|leg| leg.id)
                .unwrap()
    }));
}

#[test]
fn knowledge_update_explicitly_wakes_a_waiting_route_on_the_same_attempt() {
    let plugins: &[&dyn SimulationPlugin] = &[&InformationPlugin, &CorrespondencePlugin];
    let (mut canwu, sender, request) = start_delivery_case(true, SimDuration::days(10));
    step_until_status(
        &mut canwu,
        &request.operation_key,
        CorrespondenceStatus::Scheduled,
    );
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            PLUGIN_NAME,
            canwu_correspondence::INCIDENT_INGRESS,
            canwu.time(),
            serde_json::to_value(CorrespondenceIncidentRequest {
                operation_key: request.operation_key.clone(),
                incident_key: "all-known-rail-blocked".to_owned(),
                probability_per_mille: 1_000,
                kind: CorrespondenceIncidentKind::Disaster {
                    blocked_connections: vec![
                        RoutingConnectionRef::new("wuxi-beijing-direct"),
                        RoutingConnectionRef::new("wuxi-nanjing"),
                    ],
                    explanation: "known rail departures are blocked".to_owned(),
                },
            })
            .unwrap(),
        ))
        .unwrap();
    step_until_status(
        &mut canwu,
        &request.operation_key,
        CorrespondenceStatus::WaitingForRoute,
    );

    let mut update = network_seed(
        KnowledgeHolderRef::Person(sender),
        request.recipient.clone(),
        RoutingNodeRef::new("beijing/delivery/recipient"),
        true,
    );
    update.seed_key = "carrier-network-recovery".to_owned();
    update.connections.push(KnownRoutingConnection {
        network_version: "jiangnan-jingjin.recovery.v1".to_owned(),
        connection: RoutingConnection {
            id: RoutingConnectionRef::new("wuxi-beijing-recovery-road"),
            from: RoutingNodeRef::new("wuxi/hub"),
            to: RoutingNodeRef::new("beijing/station"),
            mode: TransferMode::Horse,
            traversal: TraversalModel::Fixed {
                duration: SimDuration::days(3),
            },
            available_from: None,
            available_until: None,
            risk_per_mille: 0,
            resource_cost: 1,
        },
    });
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            PLUGIN_NAME,
            KNOWLEDGE_INGRESS,
            canwu.time(),
            serde_json::to_value(update).unwrap(),
        ))
        .unwrap();
    canwu.step_canonical().unwrap().unwrap();

    let recovery = ResolveCorrespondenceRequest {
        operation_key: request.operation_key.clone(),
        action: CorrespondenceRecoveryAction::ReplanCurrentAttempt,
    };
    open_and_resolve_recovery_decision(&mut canwu, sender, &recovery);
    step_until_recovery_count(&mut canwu, &request.operation_key, 1);
    step_until_terminal(&mut canwu, &request.operation_key);
    let settled = load_operation(&canwu, &request.operation_key);
    assert_eq!(settled.status, CorrespondenceStatus::Settled);
    assert_eq!(settled.current_attempt_number, 1);
    assert_eq!(settled.planning_history.len(), 2);
    assert_ne!(
        settled.planning_history[0].read_cut,
        settled.planning_history[1].read_cut
    );

    let restored =
        Canwu::from_snapshot_json_with_plugins(&canwu.snapshot_json().unwrap(), plugins).unwrap();
    assert_eq!(restored.snapshot(), canwu.snapshot());
    let replayed = Canwu::replay_from_journal(plugins, &canwu.replay_journal()).unwrap();
    assert_eq!(replayed.snapshot(), canwu.snapshot());
}

#[test]
fn interception_records_access_without_stopping_delivery() {
    let operation = run_delivery_case(true, IncidentCase::Interception, SimDuration::days(10));
    assert_eq!(operation.status, CorrespondenceStatus::Settled);
    let incident = &operation.incidents["interception-1"];
    assert!(incident.triggered);
    assert!(incident.information_operation.is_some());
}

#[test]
fn late_arrival_fails_without_rewriting_the_deadline() {
    let (mut canwu, _, request) = start_delivery_case(true, SimDuration::days(1));
    step_until_terminal(&mut canwu, &request.operation_key);
    let operation = load_operation(&canwu, &request.operation_key);
    assert_eq!(operation.status, CorrespondenceStatus::DeadlineMissed);
    assert!(operation.route_plan.estimated_arrival_at > operation.intent.due_at);
    let attempt_evidence = operation.execution.delivery_attempt.as_ref().unwrap();
    let attempt_ref =
        TypedDomainRecordRef::<DeliveryAttempt>::from_untyped(attempt_evidence.record.clone())
            .unwrap();
    let attempt_record = canwu.typed_domain_record(&attempt_ref).unwrap();
    assert_eq!(attempt_record.version, attempt_evidence.version);
    let attempt = attempt_record.decode_payload::<DeliveryAttempt>().unwrap();
    assert_eq!(attempt.status, DeliveryAttemptStatus::Failed);
    assert_eq!(attempt.due_at, request.due_at);
}

#[test]
fn failed_delivery_retries_with_a_successor_attempt_and_active_dispatch() {
    let plugins: &[&dyn SimulationPlugin] = &[&InformationPlugin, &CorrespondencePlugin];
    let (mut canwu, sender, request) = start_delivery_case(true, SimDuration::days(1));
    step_until_terminal(&mut canwu, &request.operation_key);
    let failed = load_operation(&canwu, &request.operation_key);
    assert_eq!(failed.status, CorrespondenceStatus::DeadlineMissed);
    assert_eq!(failed.dispatch.version, 2);

    let recovery = ResolveCorrespondenceRequest {
        operation_key: request.operation_key.clone(),
        action: CorrespondenceRecoveryAction::RetryDelivery {
            due_at: canwu.time() + SimDuration::days(10),
            delivery_attempt_operation: canwu_information::InformationOperationId::new(
                "fixture.correspondence",
                "beijing-attempt-retry-2",
            ),
            execution_id: TransportExecutionId(22),
        },
    };
    open_and_resolve_recovery_decision(&mut canwu, sender, &recovery);
    step_until_recovery_count(&mut canwu, &request.operation_key, 1);
    step_until_terminal(&mut canwu, &request.operation_key);
    let settled = load_operation(&canwu, &request.operation_key);
    assert_eq!(settled.status, CorrespondenceStatus::Settled);
    assert_eq!(settled.current_attempt_number, 2);
    assert_eq!(settled.recovery_history.len(), 1);
    assert_eq!(settled.planning_history.len(), 2);
    assert_eq!(settled.execution.id, TransportExecutionId(22));
    assert_eq!(settled.dispatch.version, 3);

    let restored =
        Canwu::from_snapshot_json_with_plugins(&canwu.snapshot_json().unwrap(), plugins).unwrap();
    assert_eq!(restored.snapshot(), canwu.snapshot());
    let replayed = Canwu::replay_from_journal(plugins, &canwu.replay_journal()).unwrap();
    assert_eq!(replayed.snapshot(), canwu.snapshot());
}

#[test]
fn failed_delivery_keeps_dispatch_active_until_explicit_finalization() {
    let (mut canwu, sender, request) = start_delivery_case(true, SimDuration::days(1));
    step_until_terminal(&mut canwu, &request.operation_key);
    let failed = load_operation(&canwu, &request.operation_key);
    assert_eq!(failed.dispatch.version, 2);

    let recovery = ResolveCorrespondenceRequest {
        operation_key: request.operation_key.clone(),
        action: CorrespondenceRecoveryAction::FinalizeDispatch,
    };
    open_and_resolve_recovery_decision(&mut canwu, sender, &recovery);
    step_until_recovery_count(&mut canwu, &request.operation_key, 1);
    step_until_terminal(&mut canwu, &request.operation_key);
    let finalized = load_operation(&canwu, &request.operation_key);
    assert_eq!(finalized.status, CorrespondenceStatus::DeadlineMissed);
    assert_eq!(finalized.dispatch.version, 3);
}

#[test]
fn external_progress_cannot_bypass_planned_travel_time() {
    let (mut canwu, _, request) = start_delivery_case(false, SimDuration::days(1));
    step_until_status(
        &mut canwu,
        &request.operation_key,
        CorrespondenceStatus::Scheduled,
    );
    let operation = load_operation(&canwu, &request.operation_key);
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            PLUGIN_NAME,
            "progress_correspondence_v1",
            canwu.time(),
            serde_json::to_value(ProgressRequest {
                operation_key: request.operation_key,
                sequence: operation.next_sequence,
                action: ProgressAction::CompleteLeg,
            })
            .unwrap(),
        ))
        .unwrap();
    assert_eq!(
        canwu.step_canonical().unwrap_err().code,
        ErrorCode::InvalidAuthority
    );
}

#[test]
fn sender_cannot_read_an_unrelated_carriers_private_route_knowledge() {
    let plugins: &[&dyn SimulationPlugin] = &[&InformationPlugin, &CorrespondencePlugin];
    let (scenario, sender, recipient, prepared_dispatch) = scenario_with_prepared_dispatch();
    let mut canwu = Canwu::new_with_plugins(1940, scenario, plugins).unwrap();
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            PLUGIN_NAME,
            KNOWLEDGE_INGRESS,
            canwu.time(),
            serde_json::to_value(network_seed(
                recipient.clone(),
                recipient.clone(),
                RoutingNodeRef::new("wuxi/delivery/recipient"),
                false,
            ))
            .unwrap(),
        ))
        .unwrap();
    canwu.step_canonical().unwrap().unwrap();
    let request = InitiateCorrespondenceRequest {
        operation_key: "unauthorized-carrier".to_owned(),
        sender: EntityRef::Person(sender),
        recipient: recipient.clone(),
        carrier: recipient,
        channel_profile: "sealed-letter".to_owned(),
        origin: RoutingNodeRef::new("wuxi/hub"),
        due_at: canwu.time() + SimDuration::days(1),
        prepared_dispatch,
        delivery_attempt_operation: canwu_information::InformationOperationId::new(
            "fixture.correspondence",
            "unauthorized-carrier-attempt",
        ),
        routing_policy: canwu_api::RoutingPolicy::default(),
        capacity_admission: CorrespondenceCapacityAdmission::Unconstrained,
        execution_id: TransportExecutionId(31),
        automatic_opportunity: None,
    };
    open_and_resolve_send_decision(&mut canwu, sender, &request);
    assert!(
        canwu
            .typed_domain_record(&correspondence_operation_ref(&request.operation_key))
            .is_none()
    );
}

#[test]
fn automatic_opportunity_selects_and_consumes_one_recipient_deterministically() {
    let plugins: &[&dyn SimulationPlugin] = &[&InformationPlugin, &CorrespondencePlugin];
    let (scenario, sender, recipient, prepared_dispatch) = scenario_with_prepared_dispatch();
    let mut canwu = Canwu::new_with_plugins(1940, scenario, plugins).unwrap();
    let operation_key = "automatic-wuxi-letter";
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            PLUGIN_NAME,
            KNOWLEDGE_INGRESS,
            canwu.time(),
            serde_json::to_value(network_seed(
                KnowledgeHolderRef::Person(sender),
                recipient.clone(),
                RoutingNodeRef::new("wuxi/delivery/recipient"),
                false,
            ))
            .unwrap(),
        ))
        .unwrap();
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            PLUGIN_NAME,
            OPPORTUNITY_INGRESS,
            canwu.time(),
            serde_json::to_value(CommunicationOpportunityRequest {
                operation_key: operation_key.to_owned(),
                sender: EntityRef::Person(sender),
                candidates: vec![recipient.clone(), KnowledgeHolderRef::Person(sender)],
                reason: "routine correspondence".to_owned(),
                probability_per_mille: 1_000,
                automatic: true,
            })
            .unwrap(),
        ))
        .unwrap();
    canwu.step_canonical().unwrap().unwrap();
    let opportunity_reference = opportunity_ref(operation_key);
    let selected = canwu
        .typed_domain_record(&opportunity_reference)
        .unwrap()
        .decode_payload::<CommunicationOpportunityRecord>()
        .unwrap();
    assert_eq!(
        selected.status,
        CommunicationOpportunityStatus::SelectedAutomatic
    );
    assert_eq!(selected.selected_recipient.as_ref(), Some(&recipient));

    let request = InitiateCorrespondenceRequest {
        operation_key: operation_key.to_owned(),
        sender: EntityRef::Person(sender),
        recipient,
        carrier: KnowledgeHolderRef::Person(sender),
        channel_profile: "sealed-letter".to_owned(),
        origin: RoutingNodeRef::new("wuxi/hub"),
        due_at: canwu.time() + SimDuration::days(1),
        prepared_dispatch,
        delivery_attempt_operation: canwu_information::InformationOperationId::new(
            "fixture.correspondence",
            "automatic-attempt",
        ),
        routing_policy: canwu_api::RoutingPolicy::default(),
        capacity_admission: CorrespondenceCapacityAdmission::Unconstrained,
        execution_id: canwu_api::TransportExecutionId(10),
        automatic_opportunity: Some(opportunity_reference.clone()),
    };
    canwu
        .enqueue_command(
            canwu.time(),
            0,
            CommandRequest::new(
                CommandRequestId::new(1),
                canwu.revision(),
                CommandEnvelope::new(
                    Issuer::System("correspondence-scheduler".to_owned()),
                    correspondence_command(&request).unwrap(),
                )
                .at_time(canwu.time())
                .with_authority(CommandAuthority::no_responsible_actor(
                    "automatic communication opportunity",
                )),
            ),
        )
        .unwrap();
    step_until_terminal(&mut canwu, operation_key);
    assert_eq!(
        canwu
            .typed_domain_record(&opportunity_reference)
            .unwrap()
            .decode_payload::<CommunicationOpportunityRecord>()
            .unwrap()
            .status,
        CommunicationOpportunityStatus::Consumed
    );
    assert_eq!(
        load_operation(&canwu, operation_key).status,
        CorrespondenceStatus::Settled
    );

    let restored =
        Canwu::from_snapshot_json_with_plugins(&canwu.snapshot_json().unwrap(), plugins).unwrap();
    assert_eq!(restored.snapshot(), canwu.snapshot());
    let replayed = Canwu::replay_from_journal(plugins, &canwu.replay_journal()).unwrap();
    assert_eq!(replayed.snapshot(), canwu.snapshot());
}

fn run_delivery_case(
    long_distance: bool,
    incident: IncidentCase,
    due_after: SimDuration,
) -> canwu_correspondence::CorrespondenceOperation {
    let plugins: &[&dyn SimulationPlugin] = &[&InformationPlugin, &CorrespondencePlugin];
    let (mut canwu, sender, request) = start_delivery_case(long_distance, due_after);

    if matches!(incident, IncidentCase::DisasterAfterHandoff) {
        step_until_handoff(&mut canwu, &request.operation_key);
    } else if !matches!(incident, IncidentCase::None) {
        step_until_status(
            &mut canwu,
            &request.operation_key,
            CorrespondenceStatus::Scheduled,
        );
    }
    if !matches!(incident, IncidentCase::None) {
        let (incident_key, kind) = match incident {
            IncidentCase::Disaster => (
                "flood-1",
                CorrespondenceIncidentKind::Disaster {
                    blocked_connections: vec![RoutingConnectionRef::new("wuxi-beijing-direct")],
                    explanation: "flooded rail bridge".to_owned(),
                },
            ),
            IncidentCase::DisasterAfterHandoff => (
                "final-mile-blockage",
                CorrespondenceIncidentKind::Disaster {
                    blocked_connections: vec![RoutingConnectionRef::new("beijing-final-mile")],
                    explanation: "blocked final-mile road".to_owned(),
                },
            ),
            IncidentCase::Interception => (
                "interception-1",
                CorrespondenceIncidentKind::Interception {
                    intercepted_by: KnowledgeHolderRef::Person(sender),
                    extent_per_mille: 700,
                },
            ),
            IncidentCase::None => unreachable!(),
        };
        canwu
            .enqueue_plugin_ingress(PluginIngressRequest::new(
                PLUGIN_NAME,
                canwu_correspondence::INCIDENT_INGRESS,
                if matches!(incident, IncidentCase::DisasterAfterHandoff) {
                    canwu.time() + SimDuration::minutes(1)
                } else {
                    canwu.time()
                },
                serde_json::to_value(CorrespondenceIncidentRequest {
                    operation_key: request.operation_key.clone(),
                    incident_key: incident_key.to_owned(),
                    probability_per_mille: 1_000,
                    kind,
                })
                .unwrap(),
            ))
            .unwrap();
    }
    step_until_terminal(&mut canwu, &request.operation_key);
    let operation = load_operation(&canwu, &request.operation_key);
    let dispatch =
        TypedDomainRecordRef::<Dispatch>::from_untyped(operation.dispatch.record.clone()).unwrap();
    let dispatch_status = canwu
        .typed_domain_record(&dispatch)
        .unwrap()
        .decode_payload::<Dispatch>()
        .unwrap()
        .status;
    if operation.status == CorrespondenceStatus::Settled {
        assert_eq!(operation.dispatch.version, 3);
        assert_eq!(dispatch_status, DispatchStatus::Completed);
    } else {
        assert_eq!(operation.dispatch.version, 2);
        assert_eq!(dispatch_status, DispatchStatus::Active);
    }
    if let IncidentCase::Interception = incident {
        let information_operation = operation.incidents["interception-1"]
            .information_operation
            .as_ref()
            .unwrap();
        let information = canwu
            .typed_domain_record(&derive_operation_record_ref(information_operation))
            .unwrap()
            .decode_payload::<InformationOperationRecord>()
            .unwrap();
        assert_eq!(information.status, InformationOperationStatus::Completed);
        assert!(
            information
                .domain_result_evidence
                .iter()
                .any(|evidence| { evidence.record.kind == DomainRecordKind::for_type::<Access>() })
        );
    }

    let snapshot_json = canwu.snapshot_json().unwrap();
    let restored = Canwu::from_snapshot_json_with_plugins(&snapshot_json, plugins).unwrap();
    assert_eq!(restored.snapshot(), canwu.snapshot());
    let replayed = Canwu::replay_from_journal(plugins, &canwu.replay_journal()).unwrap();
    assert_eq!(replayed.snapshot(), canwu.snapshot());
    operation
}

fn start_delivery_case(
    long_distance: bool,
    due_after: SimDuration,
) -> (Canwu, canwu_api::PersonId, InitiateCorrespondenceRequest) {
    let plugins: &[&dyn SimulationPlugin] = &[&InformationPlugin, &CorrespondencePlugin];
    let (scenario, sender, recipient, prepared_dispatch) = scenario_with_prepared_dispatch();
    let mut canwu = Canwu::new_with_plugins(1940, scenario, plugins).unwrap();
    let carrier = KnowledgeHolderRef::Person(sender);
    let destination = if long_distance {
        RoutingNodeRef::new("beijing/delivery/recipient")
    } else {
        RoutingNodeRef::new("wuxi/delivery/recipient")
    };
    let seed = network_seed(
        carrier.clone(),
        recipient.clone(),
        destination,
        long_distance,
    );
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            PLUGIN_NAME,
            KNOWLEDGE_INGRESS,
            canwu.time(),
            serde_json::to_value(seed).unwrap(),
        ))
        .unwrap();
    canwu.step_canonical().unwrap().unwrap();

    let request = InitiateCorrespondenceRequest {
        operation_key: if long_distance {
            "wuxi-to-beijing-letter".to_owned()
        } else {
            "wuxi-local-letter".to_owned()
        },
        sender: EntityRef::Person(sender),
        recipient: recipient.clone(),
        carrier,
        channel_profile: "sealed-letter".to_owned(),
        origin: RoutingNodeRef::new("wuxi/hub"),
        due_at: canwu.time() + due_after,
        prepared_dispatch,
        delivery_attempt_operation: canwu_information::InformationOperationId::new(
            "fixture.correspondence",
            if long_distance {
                "beijing-attempt"
            } else {
                "local-attempt"
            },
        ),
        routing_policy: canwu_api::RoutingPolicy::default(),
        capacity_admission: CorrespondenceCapacityAdmission::Unconstrained,
        execution_id: canwu_api::TransportExecutionId(if long_distance { 2 } else { 1 }),
        automatic_opportunity: None,
    };
    open_and_resolve_send_decision(&mut canwu, sender, &request);
    (canwu, sender, request)
}

fn open_and_resolve_send_decision(
    canwu: &mut Canwu,
    sender: canwu_api::PersonId,
    request: &InitiateCorrespondenceRequest,
) {
    let controller = DecisionControllerBinding::new(
        "sender-policy",
        DecisionPolicyIdentity::new(DecisionPolicyKind::Utility, "send-policy", "1"),
        DecisionAuthority::Actor { actor: sender },
    );
    let mut ticket = correspondence_decision_ticket(
        DecisionTicketId::new(1),
        EntityRef::Person(sender),
        "sender-policy",
        "Decide whether to send the prepared correspondence",
        Some(request.due_at),
        request,
    )
    .unwrap();
    ticket.options.iter_mut().for_each(|option| {
        option
            .utility_inputs
            .insert("send".to_owned(), if option.id == "send" { 100 } else { 0 });
    });
    let now = canwu.time();
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
        .unwrap();
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
                canwu.time(),
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
}

fn open_and_resolve_recovery_decision(
    canwu: &mut Canwu,
    sender: canwu_api::PersonId,
    request: &ResolveCorrespondenceRequest,
) {
    let mut ticket = correspondence_recovery_decision_ticket(
        DecisionTicketId::new(2),
        EntityRef::Person(sender),
        "sender-policy",
        "Resolve failed correspondence",
        None,
        request,
    )
    .unwrap();
    ticket.options.iter_mut().for_each(|option| {
        option.utility_inputs.insert(
            "send".to_owned(),
            if option.id == "apply_recovery" {
                100
            } else {
                0
            },
        );
    });
    canwu
        .enqueue_decision(
            canwu.time(),
            0,
            DecisionIngressRequest::new(
                DecisionRequestId::new(4),
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
                canwu.time(),
                0,
                DecisionRequestId::new(5),
                Some(CommandRequestId::new(2)),
                DecisionTicketId::new(2),
                &policy,
            )
            .unwrap(),
        DecisionEvaluation::Prepared(_)
    ));
    canwu.step_canonical().unwrap().unwrap();
}

fn step_until_status(canwu: &mut Canwu, key: &str, expected: CorrespondenceStatus) {
    for _ in 0..40 {
        if canwu
            .typed_domain_record(&correspondence_operation_ref(key))
            .map(|record| {
                record
                    .decode_payload::<CorrespondenceOperationRecord>()
                    .unwrap()
                    .status
            })
            == Some(expected)
        {
            return;
        }
        canwu.step_canonical().unwrap().unwrap();
    }
    panic!("correspondence did not reach {expected:?}");
}

fn step_until_handoff(canwu: &mut Canwu, key: &str) {
    for _ in 0..60 {
        if let Some(operation) = canwu
            .typed_domain_record(&correspondence_operation_ref(key))
            .map(|record| {
                record
                    .decode_payload::<CorrespondenceOperationRecord>()
                    .unwrap()
            })
            && operation.status == CorrespondenceStatus::Scheduled
            && !operation.execution.handoffs.is_empty()
        {
            return;
        }
        canwu.step_canonical().unwrap().unwrap();
    }
    panic!("correspondence did not reach a post-handoff boundary");
}

fn step_until_recovery_count(canwu: &mut Canwu, key: &str, expected: usize) {
    for _ in 0..40 {
        if canwu
            .typed_domain_record(&correspondence_operation_ref(key))
            .map(|record| {
                record
                    .decode_payload::<CorrespondenceOperationRecord>()
                    .unwrap()
                    .recovery_history
                    .len()
            })
            == Some(expected)
        {
            return;
        }
        canwu.step_canonical().unwrap().unwrap();
    }
    panic!("correspondence did not apply recovery action");
}

fn step_until_terminal(canwu: &mut Canwu, key: &str) {
    for _ in 0..80 {
        if let Some(operation) = canwu
            .typed_domain_record(&correspondence_operation_ref(key))
            .map(|record| {
                record
                    .decode_payload::<CorrespondenceOperationRecord>()
                    .unwrap()
            })
            && matches!(
                operation.status,
                CorrespondenceStatus::Settled
                    | CorrespondenceStatus::DeadlineMissed
                    | CorrespondenceStatus::Failed
                    | CorrespondenceStatus::CompensationPending
            )
        {
            return;
        }
        canwu.step_canonical().unwrap().unwrap();
    }
    panic!("correspondence did not terminate");
}

fn load_operation(canwu: &Canwu, key: &str) -> canwu_correspondence::CorrespondenceOperation {
    canwu
        .typed_domain_record(&correspondence_operation_ref(key))
        .unwrap()
        .decode_payload::<CorrespondenceOperationRecord>()
        .unwrap()
}
