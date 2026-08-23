mod support;

use canwu_api::{
    Canwu, CommandAuthority, CommandEnvelope, CommandRequest, CommandRequestId, EntityRef, Issuer,
    KnowledgeHolderRef, PluginIngressRequest, RoutingNodeRef, SimDuration, SimulationPlugin,
};
use canwu_correspondence::{
    CommunicationOpportunityRequest, CorrespondenceCapacityAdmission, CorrespondenceOperation,
    CorrespondenceOperationRecord, CorrespondencePlugin, CorrespondenceStatus,
    InitiateCorrespondenceRequest, KNOWLEDGE_INGRESS, OPPORTUNITY_INGRESS, PLUGIN_NAME,
    correspondence_command, correspondence_operation_ref, opportunity_ref,
};
use canwu_information::{InformationOperationId, InformationPlugin};
use support::{network_seed, scenario_with_prepared_dispatch};

fn main() {
    let local = run(false);
    let beijing = run(true);

    print_plan("Wuxi local delivery", &local);
    print_plan("Wuxi to Beijing", &beijing);
}

fn run(long_distance: bool) -> CorrespondenceOperation {
    let plugins: &[&dyn SimulationPlugin] = &[&InformationPlugin, &CorrespondencePlugin];
    let (scenario, sender, recipient, prepared_dispatch) = scenario_with_prepared_dispatch();
    let mut canwu = Canwu::new_with_plugins(1940, scenario, plugins).unwrap();
    let operation_key = if long_distance {
        "example-wuxi-to-beijing"
    } else {
        "example-wuxi-local"
    };
    let destination = if long_distance {
        RoutingNodeRef::new("beijing/delivery/recipient")
    } else {
        RoutingNodeRef::new("wuxi/delivery/recipient")
    };
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            PLUGIN_NAME,
            KNOWLEDGE_INGRESS,
            canwu.time(),
            serde_json::to_value(network_seed(
                KnowledgeHolderRef::Person(sender),
                recipient.clone(),
                destination,
                long_distance,
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
                candidates: vec![recipient.clone()],
                reason: "routine correspondence".to_owned(),
                probability_per_mille: 1_000,
                automatic: true,
            })
            .unwrap(),
        ))
        .unwrap();
    canwu.step_canonical().unwrap().unwrap();

    let request = InitiateCorrespondenceRequest {
        operation_key: operation_key.to_owned(),
        sender: EntityRef::Person(sender),
        recipient,
        carrier: KnowledgeHolderRef::Person(sender),
        channel_profile: "sealed-letter".to_owned(),
        origin: RoutingNodeRef::new("wuxi/hub"),
        due_at: canwu.time() + SimDuration::days(10),
        prepared_dispatch,
        delivery_attempt_operation: InformationOperationId::new(
            "example.correspondence",
            format!("{operation_key}-attempt"),
        ),
        routing_policy: canwu_api::RoutingPolicy::default(),
        capacity_admission: CorrespondenceCapacityAdmission::Unconstrained,
        execution_id: canwu_api::TransportExecutionId(if long_distance { 2 } else { 1 }),
        automatic_opportunity: Some(opportunity_ref(operation_key)),
    };
    canwu
        .enqueue_command(
            canwu.time(),
            0,
            CommandRequest::new(
                CommandRequestId::new(1),
                canwu.revision(),
                CommandEnvelope::new(
                    Issuer::System("correspondence-example".to_owned()),
                    correspondence_command(&request).unwrap(),
                )
                .at_time(canwu.time())
                .with_authority(CommandAuthority::no_responsible_actor(
                    "automatic communication opportunity",
                )),
            ),
        )
        .unwrap();

    for _ in 0..80 {
        if let Some(operation) = canwu
            .typed_domain_record(&correspondence_operation_ref(operation_key))
            .map(|record| {
                record
                    .decode_payload::<CorrespondenceOperationRecord>()
                    .unwrap()
            })
            && operation.status == CorrespondenceStatus::Settled
        {
            return operation;
        }
        canwu.step_canonical().unwrap().unwrap();
    }
    panic!("correspondence did not settle");
}

fn print_plan(label: &str, operation: &CorrespondenceOperation) {
    println!(
        "{label}: {} leg(s), arrival minute {}",
        operation.route_plan.legs.len(),
        operation.route_plan.estimated_arrival_at.as_minutes()
    );
    for leg in &operation.route_plan.legs {
        println!(
            "  {} -> {} via {:?}",
            leg.from.as_str(),
            leg.to.as_str(),
            leg.mode
        );
    }
}
