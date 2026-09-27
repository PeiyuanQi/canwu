//! Public-API fixture for gap G-07: a planning snapshot built from one
//! holder's knowledge ledger, with a digest of the knowledge records read.

use canwu_api::{
    Canwu, CanwuError, Command, CommandContext, CommandEnvelope, CommandRequest, CommandRequestId,
    EntityRef, ErrorCode, Issuer, KnowledgeHolderRef, PayloadSchema, PersonId, PlanningSnapshot,
    PluginActionDescriptor, PluginIngressRequest, PluginRegistrar, RoutingConnection,
    RoutingConnectionRef, RoutingEndpoint, RoutingEndpointKind, RoutingNodeRef, RoutingPolicy,
    RoutingRequest, Scenario, SimDuration, SimulationPlugin, SimulationView, StateKey,
    SystemDirective, TransferMode, TraversalModel, plan_route,
};
use canwu_correspondence::{
    CorrespondencePlugin, KNOWLEDGE_INGRESS, KnowledgeReadCutDigest, KnownAddress,
    KnownRoutingConnection, KnownRoutingEndpoint, NetworkKnowledgeSeed, PLUGIN_NAME,
    planning_knowledge_query, planning_snapshot_from_holder_knowledge,
    planning_snapshot_from_knowledge_result,
};
use canwu_information::InformationPlugin;
use serde_json::{Value, json};

const PROBE_PLUGIN: &str = "fixture-holder-planning";
const PROBE_COMMAND: &str = "probe_holder_planning";

fn probe_state() -> StateKey {
    StateKey::new("fixture", "holder_planning")
}

/// Builds the issuing actor's snapshot in a command whose only declared read
/// is the core knowledge ledger, so any read of route or world truth would
/// fail. The holder comes from the admitted issuer, never from the payload.
fn probe(
    view: &SimulationView<'_>,
    context: &CommandContext,
    _payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    let Issuer::Actor(holder) = context.issuer else {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "only an actor may probe its own planning knowledge",
        ));
    };
    let (snapshot, digest) = planning_snapshot_from_holder_knowledge(
        view,
        &KnowledgeHolderRef::Person(holder),
        view.time(),
    )?;
    Ok(vec![SystemDirective::SetComponent {
        state: probe_state(),
        entity: EntityRef::Person(holder),
        component: "planning".to_owned(),
        value: json!({ "snapshot": snapshot, "digest": digest }),
        summary: "Record the holder-relative planning snapshot".to_owned(),
    }])
}

struct PlanningProbePlugin;

impl SimulationPlugin for PlanningProbePlugin {
    fn name(&self) -> &'static str {
        PROBE_PLUGIN
    }

    fn version(&self) -> &'static str {
        "1.0.0"
    }

    fn semantic_hash(&self) -> &'static str {
        "0707070707070707070707070707070707070707070707070707070707070707"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_command(
            PluginActionDescriptor {
                name: PROBE_COMMAND.to_owned(),
                description: "Build a holder-relative planning snapshot".to_owned(),
                payload_schema: PayloadSchema::Any,
                reads: vec![StateKey::core_knowledge()],
                writes: vec![probe_state()],
            },
            probe,
        )
    }
}

fn seed(
    seed_key: &str,
    holder: PersonId,
    endpoints: &[&str],
    connections: &[(&str, &str, &str, i64)],
) -> NetworkKnowledgeSeed {
    NetworkKnowledgeSeed {
        seed_key: seed_key.to_owned(),
        holder: KnowledgeHolderRef::Person(holder),
        endpoints: endpoints
            .iter()
            .map(|id| KnownRoutingEndpoint {
                network_version: "fixture.v1".to_owned(),
                endpoint: RoutingEndpoint {
                    id: RoutingNodeRef::new(*id),
                    kind: RoutingEndpointKind::RelayStation,
                },
            })
            .collect(),
        connections: connections
            .iter()
            .map(|(id, from, to, hours)| KnownRoutingConnection {
                network_version: "fixture.v1".to_owned(),
                connection: RoutingConnection {
                    id: RoutingConnectionRef::new(*id),
                    from: RoutingNodeRef::new(*from),
                    to: RoutingNodeRef::new(*to),
                    mode: TransferMode::Horse,
                    traversal: TraversalModel::Fixed {
                        duration: SimDuration::hours(*hours),
                    },
                    available_from: None,
                    available_until: None,
                    risk_per_mille: 0,
                    resource_cost: 1,
                },
            })
            .collect(),
        addresses: Vec::new(),
    }
}

fn install(canwu: &mut Canwu, seeds: Vec<NetworkKnowledgeSeed>) {
    for seed in seeds {
        canwu
            .enqueue_plugin_ingress(PluginIngressRequest::new(
                PLUGIN_NAME,
                KNOWLEDGE_INGRESS,
                canwu.time(),
                serde_json::to_value(seed).unwrap(),
            ))
            .unwrap();
    }
    canwu.step_canonical().unwrap().unwrap();
}

fn probe_holder(
    canwu: &mut Canwu,
    request: u64,
    holder: PersonId,
) -> (PlanningSnapshot, KnowledgeReadCutDigest) {
    canwu
        .enqueue_command(
            canwu.time(),
            0,
            CommandRequest::new(
                CommandRequestId::new(request),
                canwu.revision(),
                CommandEnvelope::new(
                    Issuer::Actor(holder),
                    Command::Plugin {
                        plugin: PROBE_PLUGIN.to_owned(),
                        command: PROBE_COMMAND.to_owned(),
                        payload: json!({}),
                    },
                ),
            ),
        )
        .unwrap();
    canwu.step_canonical().unwrap().unwrap();
    let value = canwu
        .snapshot()
        .plugin_components
        .iter()
        .find(|record| record.state == probe_state() && record.entity == EntityRef::Person(holder))
        .map(|record| record.value.clone())
        .unwrap();
    (
        serde_json::from_value(value["snapshot"].clone()).unwrap(),
        serde_json::from_value(value["digest"].clone()).unwrap(),
    )
}

fn connection_ids(snapshot: &PlanningSnapshot) -> Vec<&str> {
    snapshot
        .network
        .connections
        .iter()
        .map(|connection| connection.id.as_str())
        .collect()
}

#[test]
fn gap_g07_correspondence_holder_planning_snapshot() {
    let demo = Canwu::demo(1).unwrap();
    let ids = Canwu::demo_ids();
    let initial = demo.snapshot();
    let scenario = Scenario {
        start_time: initial.initial_time,
        entities: initial.entities,
        world: initial.world,
        knowledge: initial.knowledge,
        domain_records: Vec::new(),
    };
    let mut canwu = Canwu::new_with_plugins(
        707,
        scenario,
        &[
            &InformationPlugin,
            &CorrespondencePlugin,
            &PlanningProbePlugin,
        ],
    )
    .unwrap();
    let (holder, surveyor) = (ids.commander, ids.observer);
    // The full network, including the direct a-c road, is published to another
    // holder and stands in for route truth; the demo world also carries core
    // routes. The planning holder knows only a-b and b-c.
    install(
        &mut canwu,
        vec![
            seed(
                "surveyor-network",
                surveyor,
                &["a", "b", "c"],
                &[
                    ("a-b", "a", "b", 4),
                    ("a-c", "a", "c", 3),
                    ("b-c", "b", "c", 4),
                ],
            ),
            seed(
                "holder-network",
                holder,
                &["a", "b", "c"],
                &[("a-b", "a", "b", 4), ("b-c", "b", "c", 4)],
            ),
        ],
    );

    let (snapshot, digest) = probe_holder(&mut canwu, 1, holder);
    assert_eq!(connection_ids(&snapshot), ["a-b", "b-c"]);
    let (full, full_digest) = probe_holder(&mut canwu, 2, surveyor);
    assert_eq!(connection_ids(&full), ["a-b", "a-c", "b-c"]);
    assert_ne!(full_digest, digest);
    let route = plan_route(
        &snapshot,
        &RoutingRequest {
            origin: RoutingNodeRef::new("a"),
            destination: RoutingNodeRef::new("c"),
            departure_at: canwu.time(),
            policy: RoutingPolicy::default(),
        },
    )
    .unwrap();
    assert_eq!(route.legs.len(), 2);

    // The pure builder over a host query of the same holder shares the admission
    // rule and yields the same network and digest.
    let hosted = canwu
        .admin_query_knowledge(
            KnowledgeHolderRef::Person(holder),
            &planning_knowledge_query(),
        )
        .unwrap();
    let (hosted_snapshot, hosted_digest) =
        planning_snapshot_from_knowledge_result(&hosted, canwu.time()).unwrap();
    assert_eq!(hosted_snapshot.network, snapshot.network);
    assert_eq!(hosted_digest, digest);

    // A new holder record outside the network (an address) moves the read cut
    // but not the digest.
    let mut address = seed("holder-address", holder, &[], &[]);
    address.addresses.push(KnownAddress {
        network_version: "fixture.v1".to_owned(),
        recipient: KnowledgeHolderRef::Person(surveyor),
        destination: RoutingNodeRef::new("c"),
    });
    install(&mut canwu, vec![address]);
    let (unchanged, unchanged_digest) = probe_holder(&mut canwu, 3, holder);
    assert_eq!(unchanged.network, snapshot.network);
    assert_ne!(unchanged.knowledge_cut, snapshot.knowledge_cut);
    assert_eq!(unchanged_digest, digest);

    // A newer record for an admitted connection replaces the fact the planner
    // reads and therefore changes the digest.
    install(
        &mut canwu,
        vec![seed(
            "holder-road-update",
            holder,
            &[],
            &[("b-c", "b", "c", 9)],
        )],
    );
    let (updated, updated_digest) = probe_holder(&mut canwu, 4, holder);
    assert_eq!(connection_ids(&updated), ["a-b", "b-c"]);
    assert_ne!(updated.network, snapshot.network);
    assert_ne!(updated_digest, digest);
}
