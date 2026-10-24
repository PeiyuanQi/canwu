//! Public-API fixtures for downstream gap G-78 and engine finding E-41: a
//! seizure report to a carrier who died before the seizure closed is
//! withheld with an event, a knowledge seed for a holder that accepts no
//! knowledge or with a malformed connection is a recorded refusal, and no
//! boundary fails.

#![allow(clippy::too_many_lines)]

#[path = "../examples/support/mod.rs"]
mod support;

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundarySystemContract,
    Canwu, CanwuError, CommandAuthority, CommandEnvelope, CommandRequest, CommandRequestId,
    CustodyState, EntityRef, HandoffId, IngressClass, IngressPayload, Issuer, KnowledgeHolderRef,
    KnowledgeQuery, LifeState, PayloadSchema, PersonAvailability, PersonId,
    PluginIngressDescriptor, PluginIngressRequest, PluginRegistrar, RoutingConnection,
    RoutingConnectionRef, RoutingNodeRef, RoutingPolicy, SimDuration, SimulationPlugin,
    SimulationView, StateKey, StateVisibility, SystemCadence, TransferMode, TransportExecutionId,
    TraversalModel,
};
use canwu_correspondence::{
    ATTEMPT_REPORT_KNOWLEDGE_SCHEMA, ATTEMPT_REPORT_WITHHELD_EVENT, CONNECTION_KNOWLEDGE_SCHEMA,
    CommunicationOpportunityRequest, CorrespondenceCapacityAdmission, CorrespondenceIncidentKind,
    CorrespondenceIncidentRequest, CorrespondenceOperation, CorrespondenceOperationRecord,
    CorrespondencePlugin, CorrespondenceStatus, INCIDENT_INGRESS, InitiateCorrespondenceRequest,
    KNOWLEDGE_INGRESS, KnowledgeSeedReceipt, KnowledgeSeedRecord, KnowledgeSeedRefusal,
    KnownRoutingConnection, NetworkKnowledgeSeed, OPPORTUNITY_INGRESS, PLUGIN_NAME,
    correspondence_command, correspondence_knowledge_schemas, correspondence_operation_ref,
    knowledge_seed_ref, opportunity_ref,
};
use canwu_information::{InformationOperationId, InformationPlugin};
use serde_json::json;
use support::{network_seed, scenario_with_prepared_dispatch};

const KEY: &str = "seized-letter";
const LIFE_PLUGIN: &str = "fixture-life";

/// Marks a person dead for every `die` packet it admits.
struct LifePlugin;

impl SimulationPlugin for LifePlugin {
    fn name(&self) -> &'static str {
        LIFE_PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000000c78"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_ingress(PluginIngressDescriptor {
            name: "die".to_owned(),
            description: "A person dies".to_owned(),
            class: IngressClass::Information,
            payload_schema: PayloadSchema::Any,
        })?;
        let mut contract = BoundarySystemContract::new(
            "record-deaths",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        contract.reads = vec![StateKey::core_ingress()];
        contract.writes = vec![StateKey::core_person_availability()];
        contract.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(contract, record_deaths)
    }
}

fn record_deaths(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut directives = Vec::new();
    for ingress_id in &context.admitted_ingress {
        if let Some(ingress) = view.ingress(*ingress_id)?
            && let IngressPayload::Plugin {
                plugin, payload, ..
            } = &ingress.payload
            && plugin == LIFE_PLUGIN
        {
            directives.push(BoundaryDirective::SetPersonAvailability {
                person: PersonId::new(payload["person"].as_u64().expect("person")),
                availability: PersonAvailability::new(
                    LifeState::Dead,
                    CustodyState::Free,
                    view.time(),
                ),
                summary: "A person dies".to_owned(),
            });
        }
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

fn plugins() -> [&'static dyn SimulationPlugin; 3] {
    [&InformationPlugin, &CorrespondencePlugin, &LifePlugin]
}

fn load(canwu: &Canwu) -> Option<CorrespondenceOperation> {
    canwu
        .typed_domain_record(&correspondence_operation_ref(KEY))
        .map(|record| {
            record
                .decode_payload::<CorrespondenceOperationRecord>()
                .unwrap()
        })
}

fn step_until(canwu: &mut Canwu, done: impl Fn(&Canwu) -> bool) {
    for _ in 0..60 {
        if done(canwu) {
            return;
        }
        canwu.step_canonical().unwrap().unwrap();
    }
    panic!("the run did not reach the expected state");
}

fn enqueue(canwu: &mut Canwu, plugin: &str, packet: &str, payload: serde_json::Value) {
    let at = canwu.time();
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(plugin, packet, at, payload))
        .unwrap();
}

fn receipt(canwu: &Canwu, key: &str) -> (KnowledgeSeedReceipt, usize) {
    let record = canwu.typed_domain_record(&knowledge_seed_ref(key)).unwrap();
    (
        record.decode_payload::<KnowledgeSeedRecord>().unwrap(),
        record.references.len(),
    )
}

fn knowledge_count(canwu: &Canwu, holder: &KnowledgeHolderRef, schema: &str) -> usize {
    let schema = correspondence_knowledge_schemas()
        .into_iter()
        .find(|candidate| candidate.id.kind.name == schema)
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
        .len()
}

fn assert_restores_and_replays(canwu: &Canwu) {
    let plugins = plugins();
    let restored =
        Canwu::from_snapshot_json_with_plugins(&canwu.snapshot_json().unwrap(), &plugins).unwrap();
    assert_eq!(restored.snapshot(), canwu.snapshot());
    let replayed = Canwu::replay_from_journal(&plugins, &canwu.replay_journal()).unwrap();
    assert_eq!(replayed.snapshot(), canwu.snapshot());
}

/// A sender who is its own carrier is seized on the road and dies before the
/// seizure closes his attempt: the attempt still fails, no report reaches his
/// ledger, an event records the withheld report, and a later seed for him is
/// refused on its receipt. No boundary fails.
#[test]
fn gap_g78_correspondence_report_to_a_dead_carrier_is_withheld() {
    let plugins = plugins();
    let (scenario, sender, recipient, prepared_dispatch) = scenario_with_prepared_dispatch();
    let mut canwu = Canwu::new_with_plugins(78, scenario, &plugins).unwrap();
    let carrier = KnowledgeHolderRef::Person(sender);
    enqueue(
        &mut canwu,
        PLUGIN_NAME,
        KNOWLEDGE_INGRESS,
        serde_json::to_value(network_seed(
            carrier.clone(),
            recipient.clone(),
            RoutingNodeRef::new("wuxi/delivery/recipient"),
            false,
        ))
        .unwrap(),
    );
    enqueue(
        &mut canwu,
        PLUGIN_NAME,
        OPPORTUNITY_INGRESS,
        serde_json::to_value(CommunicationOpportunityRequest {
            operation_key: KEY.to_owned(),
            sender: EntityRef::Person(sender),
            candidates: vec![recipient.clone()],
            reason: "routine correspondence".to_owned(),
            probability_per_mille: 1_000,
            automatic: true,
        })
        .unwrap(),
    );
    canwu.step_canonical().unwrap().unwrap();
    let request = InitiateCorrespondenceRequest {
        operation_key: KEY.to_owned(),
        sender: EntityRef::Person(sender),
        recipient,
        carrier: carrier.clone(),
        channel_profile: "sealed-letter".to_owned(),
        origin: RoutingNodeRef::new("wuxi/hub"),
        due_at: canwu.time() + SimDuration::days(1),
        prepared_dispatch,
        delivery_attempt_operation: InformationOperationId::new(
            "fixture.correspondence",
            "seized-attempt",
        ),
        routing_policy: RoutingPolicy::default(),
        capacity_admission: CorrespondenceCapacityAdmission::Unconstrained,
        execution_id: TransportExecutionId(78),
        automatic_opportunity: Some(opportunity_ref(KEY)),
        carrier_delegation: None,
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
    step_until(&mut canwu, |canwu| {
        load(canwu).is_some_and(|operation| operation.status == CorrespondenceStatus::InTransit)
    });

    // The carrier is seized on the road and dies in the same boundary.
    let at = canwu.time() + SimDuration::minutes(1);
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            PLUGIN_NAME,
            INCIDENT_INGRESS,
            at,
            serde_json::to_value(CorrespondenceIncidentRequest {
                operation_key: KEY.to_owned(),
                incident_key: "seizure".to_owned(),
                probability_per_mille: 1_000,
                kind: CorrespondenceIncidentKind::CarrierSeized {
                    seized_by: EntityRef::Army(Canwu::demo_ids().army),
                    custody_handoff: HandoffId(1),
                },
            })
            .unwrap(),
        ))
        .unwrap();
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            LIFE_PLUGIN,
            "die",
            at,
            json!({"person": sender.get()}),
        ))
        .unwrap();
    let withheld = |canwu: &Canwu| {
        canwu
            .events()
            .iter()
            .filter(|event| {
                event.kind.plugin_identity() == Some((PLUGIN_NAME, ATTEMPT_REPORT_WITHHELD_EVENT))
            })
            .count()
    };
    step_until(&mut canwu, |canwu| withheld(canwu) == 1);
    let operation = load(&canwu).unwrap();
    assert_eq!(operation.status, CorrespondenceStatus::Failed);
    assert!(operation.incidents["seizure"].triggered);
    assert_eq!(
        knowledge_count(&canwu, &carrier, ATTEMPT_REPORT_KNOWLEDGE_SCHEMA),
        0
    );

    // A seed for the dead carrier publishes nothing; its receipt records
    // the refusal and names no holder.
    let mut seed = network_seed(
        carrier.clone(),
        carrier.clone(),
        RoutingNodeRef::new("wuxi/hub"),
        false,
    );
    seed.seed_key = "dead-carrier-network".to_owned();
    let before = knowledge_count(&canwu, &carrier, CONNECTION_KNOWLEDGE_SCHEMA);
    enqueue(
        &mut canwu,
        PLUGIN_NAME,
        KNOWLEDGE_INGRESS,
        serde_json::to_value(&seed).unwrap(),
    );
    step_until(&mut canwu, |canwu| {
        canwu
            .typed_domain_record(&knowledge_seed_ref("dead-carrier-network"))
            .is_some()
    });
    let (refused, references) = receipt(&canwu, "dead-carrier-network");
    assert_eq!(
        refused.refusal,
        Some(KnowledgeSeedRefusal::HolderUnavailable)
    );
    assert_eq!(references, 0);
    assert_eq!(
        knowledge_count(&canwu, &carrier, CONNECTION_KNOWLEDGE_SCHEMA),
        before
    );
    assert_eq!(withheld(&canwu), 1);
    assert_restores_and_replays(&canwu);
}

/// A seed stating a connection no planning network admits (a recurring
/// departure outside its period) is refused on its receipt and publishes
/// nothing, while a valid seed for the same holder in the same boundary is
/// published. No boundary fails.
#[test]
fn e41_correspondence_malformed_connection_seed_is_refused() {
    let plugins = plugins();
    let (scenario, sender, recipient, _) = scenario_with_prepared_dispatch();
    let mut canwu = Canwu::new_with_plugins(41, scenario, &plugins).unwrap();
    let holder = KnowledgeHolderRef::Person(sender);
    let valid = network_seed(
        holder.clone(),
        recipient.clone(),
        RoutingNodeRef::new("wuxi/delivery/recipient"),
        false,
    );
    let mut malformed = network_seed(
        holder.clone(),
        recipient,
        RoutingNodeRef::new("wuxi/delivery/recipient"),
        false,
    );
    malformed.seed_key = "malformed-network".to_owned();
    malformed.connections.push(KnownRoutingConnection {
        network_version: "malformed".to_owned(),
        connection: RoutingConnection {
            id: RoutingConnectionRef::new("ferry"),
            from: RoutingNodeRef::new("wuxi/hub"),
            to: RoutingNodeRef::new("wuxi/delivery/recipient"),
            mode: TransferMode::Horse,
            traversal: TraversalModel::Recurring {
                period: SimDuration::hours(1),
                offsets: vec![SimDuration::hours(2)],
                duration: SimDuration::minutes(30),
            },
            available_from: None,
            available_until: None,
            risk_per_mille: 0,
            resource_cost: 1,
        },
    });
    for seed in [&malformed, &valid] {
        enqueue(
            &mut canwu,
            PLUGIN_NAME,
            KNOWLEDGE_INGRESS,
            serde_json::to_value::<&NetworkKnowledgeSeed>(seed).unwrap(),
        );
    }
    canwu.step_canonical().unwrap().unwrap();

    let (refused, references) = receipt(&canwu, "malformed-network");
    assert!(matches!(
        refused.refusal,
        Some(KnowledgeSeedRefusal::InvalidConnection { ref connection, .. })
            if connection.as_str() == "ferry"
    ));
    assert_eq!(references, 1);
    let (published, _) = receipt(&canwu, &valid.seed_key);
    assert_eq!(published.refusal, None);
    assert_eq!(
        knowledge_count(&canwu, &holder, CONNECTION_KNOWLEDGE_SCHEMA),
        valid.connections.len()
    );

    // An exact resend of the refused seed changes nothing.
    enqueue(
        &mut canwu,
        PLUGIN_NAME,
        KNOWLEDGE_INGRESS,
        serde_json::to_value(&malformed).unwrap(),
    );
    canwu.step_canonical().unwrap().unwrap();
    assert_eq!(receipt(&canwu, "malformed-network").0, refused);
    assert_eq!(
        knowledge_count(&canwu, &holder, CONNECTION_KNOWLEDGE_SCHEMA),
        valid.connections.len()
    );
    assert_restores_and_replays(&canwu);
}
