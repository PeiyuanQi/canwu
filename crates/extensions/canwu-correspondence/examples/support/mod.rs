use canwu_api::{
    Canwu, DomainRecord, DomainRecordClass, DomainRecordLifecycle, DomainRecordType,
    DomainRecordVersionRef, DomainRecordVersionSource, DomainReference, DomainReferenceTarget,
    EntityRef, KnowledgeHolderRef, RoutingConnection, RoutingConnectionRef, RoutingEndpoint,
    RoutingEndpointKind, RoutingNodeRef, Scenario, SimDuration, SimTime, TransferMode,
    TraversalModel, TypedDomainRecordRef,
};
use canwu_correspondence::{
    KnownAddress, KnownRoutingConnection, KnownRoutingEndpoint, NetworkKnowledgeSeed,
};
use canwu_information::{
    Channel, ChannelCapability, ChannelPayload, Content, ContentPayload, ContentRelation, Dispatch,
    DispatchPayload, DispatchStatus, DispatchTarget, InformationBody, Representation,
    RepresentationPayload,
};
use serde::Serialize;
use serde_json::json;

const NETWORK_VERSION: &str = "jiangnan-jingjin.v1";

pub fn network_seed(
    holder: KnowledgeHolderRef,
    recipient: KnowledgeHolderRef,
    destination: RoutingNodeRef,
    long_distance: bool,
) -> NetworkKnowledgeSeed {
    let departure = SimTime::EPOCH + SimDuration::hours(1);
    let mut endpoints = vec![endpoint("wuxi/hub", RoutingEndpointKind::RelayStation)];
    let mut connections = Vec::new();
    if long_distance {
        endpoints.extend([
            endpoint("nanjing/station", RoutingEndpointKind::RailwayStation),
            endpoint("beijing/station", RoutingEndpointKind::RailwayStation),
            endpoint(
                "beijing/delivery/recipient",
                RoutingEndpointKind::DeliveryDistrict,
            ),
        ]);
        connections.extend([
            connection_departure(
                "wuxi-beijing-direct",
                "wuxi/hub",
                "beijing/station",
                TransferMode::Rail,
                departure,
                SimDuration::days(2),
            ),
            connection_departure(
                "wuxi-nanjing",
                "wuxi/hub",
                "nanjing/station",
                TransferMode::Rail,
                departure,
                SimDuration::days(1),
            ),
            connection(
                "nanjing-beijing",
                "nanjing/station",
                "beijing/station",
                TransferMode::Rail,
                SimDuration::days(2),
            ),
            connection(
                "beijing-final-mile",
                "beijing/station",
                "beijing/delivery/recipient",
                TransferMode::Horse,
                SimDuration::hours(2),
            ),
            connection(
                "beijing-final-mile-alternate",
                "beijing/station",
                "beijing/delivery/recipient",
                TransferMode::Horse,
                SimDuration::hours(3),
            ),
        ]);
    } else {
        endpoints.push(endpoint(
            "wuxi/delivery/recipient",
            RoutingEndpointKind::DeliveryDistrict,
        ));
        connections.push(connection(
            "wuxi-final-mile",
            "wuxi/hub",
            "wuxi/delivery/recipient",
            TransferMode::Horse,
            SimDuration::minutes(30),
        ));
    }
    NetworkKnowledgeSeed {
        seed_key: if long_distance {
            "carrier-network-long".to_owned()
        } else {
            "carrier-network-local".to_owned()
        },
        holder,
        endpoints,
        connections,
        addresses: vec![KnownAddress {
            network_version: NETWORK_VERSION.to_owned(),
            recipient,
            destination,
        }],
    }
}

pub fn scenario_with_prepared_dispatch() -> (
    Scenario,
    canwu_api::PersonId,
    KnowledgeHolderRef,
    DomainRecordVersionRef,
) {
    let demo = Canwu::demo(1).unwrap();
    let ids = Canwu::demo_ids();
    let snapshot = demo.snapshot();
    let recipient = KnowledgeHolderRef::Person(ids.observer);
    let channel = TypedDomainRecordRef::<Channel>::new("sealed-letter");
    let content = TypedDomainRecordRef::<Content>::new("letter-content");
    let representation = TypedDomainRecordRef::<Representation>::new("letter-representation");
    let dispatch = TypedDomainRecordRef::<Dispatch>::new("prepared-dispatch");
    let mut records = vec![
        initial_record(
            &channel,
            &ChannelPayload {
                profile: "sealed-letter".to_owned(),
                capabilities: vec![
                    ChannelCapability::PersistentInstance,
                    ChannelCapability::AddressedDelivery,
                ],
            },
            Vec::new(),
        ),
        initial_record(
            &content,
            &ContentPayload {
                content_type: "letter".to_owned(),
                body: InformationBody::InlineJson {
                    value: json!({"message": "report to Beijing"}),
                },
                created_at: snapshot.initial_time,
                derivation: None,
            },
            vec![core_reference("creator", EntityRef::Person(ids.commander))],
        ),
        initial_record(
            &representation,
            &RepresentationPayload {
                format: "written-letter".to_owned(),
                created_at: snapshot.initial_time,
                operation: "author".to_owned(),
                content_relation: ContentRelation::SameContent,
                sources: Vec::new(),
                claimed_source: None,
                interpretation_capability: None,
            },
            vec![DomainReference::from_typed("content", content)],
        ),
        initial_record(
            &dispatch,
            &DispatchPayload {
                status: DispatchStatus::Prepared,
                target: DispatchTarget::Addressed(vec![recipient.clone()]),
                prepared_at: snapshot.initial_time,
                dispatched_at: None,
                completed_at: None,
            },
            vec![
                DomainReference::from_typed("channel", channel),
                core_reference("intended_recipient", EntityRef::Person(ids.observer)),
                DomainReference::from_typed("representation", representation),
                core_reference("sender", EntityRef::Person(ids.commander)),
            ],
        ),
    ];
    records.sort_by(|left, right| left.reference.cmp(&right.reference));
    (
        Scenario {
            start_time: snapshot.initial_time,
            entities: snapshot.entities,
            world: snapshot.world,
            knowledge: snapshot.knowledge,
            domain_records: records,
        },
        ids.commander,
        recipient,
        DomainRecordVersionRef {
            record: dispatch.into_untyped(),
            version: 1,
            established_by: DomainRecordVersionSource::InitialScenario,
        },
    )
}

fn endpoint(id: &str, kind: RoutingEndpointKind) -> KnownRoutingEndpoint {
    KnownRoutingEndpoint {
        network_version: NETWORK_VERSION.to_owned(),
        endpoint: RoutingEndpoint {
            id: RoutingNodeRef::new(id),
            kind,
        },
    }
}

fn connection(
    id: &str,
    from: &str,
    to: &str,
    mode: TransferMode,
    duration: SimDuration,
) -> KnownRoutingConnection {
    KnownRoutingConnection {
        network_version: NETWORK_VERSION.to_owned(),
        connection: RoutingConnection {
            id: RoutingConnectionRef::new(id),
            from: RoutingNodeRef::new(from),
            to: RoutingNodeRef::new(to),
            mode,
            traversal: TraversalModel::Fixed { duration },
            available_from: None,
            available_until: None,
            risk_per_mille: 0,
            resource_cost: 1,
        },
    }
}

fn connection_departure(
    id: &str,
    from: &str,
    to: &str,
    mode: TransferMode,
    departure_at: SimTime,
    duration: SimDuration,
) -> KnownRoutingConnection {
    let mut known = connection(id, from, to, mode, duration);
    known.connection.traversal = TraversalModel::Departures {
        slots: [0, 5, 10]
            .into_iter()
            .map(|days| canwu_api::DepartureSlot {
                departure_at: departure_at + SimDuration::days(days),
                duration,
            })
            .collect(),
    };
    known
}

fn initial_record<T: DomainRecordType>(
    reference: &TypedDomainRecordRef<T>,
    payload: &T::Payload,
    mut references: Vec<DomainReference>,
) -> DomainRecord
where
    T::Payload: Serialize,
{
    references.sort();
    DomainRecord {
        reference: reference.as_untyped().clone(),
        owner: canwu_information::PLUGIN_NAME.to_owned(),
        class: DomainRecordClass::Record,
        version: 1,
        lifecycle: DomainRecordLifecycle::Active,
        payload: serde_json::to_value(payload).unwrap(),
        references,
    }
}

fn core_reference(role: &str, entity: EntityRef) -> DomainReference {
    DomainReference {
        role: role.to_owned(),
        target: DomainReferenceTarget::Core(entity),
    }
}
