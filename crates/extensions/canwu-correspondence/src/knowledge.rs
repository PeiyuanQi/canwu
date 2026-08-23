use crate::model::AddressResolution;
use canwu_api::{
    KnowledgeHistoryView, KnowledgeHolderRef, KnowledgeQuery, KnowledgeQueryResult,
    KnowledgeRecordKind, KnowledgeSchemaId, MAX_KNOWLEDGE_PAGE_SIZE, PayloadSchema,
    PlanningSnapshot, PluginKnowledgeSchema, RoutingConnection, RoutingEndpoint, RoutingNetwork,
    SimTime, canonical_hash,
};
use serde::{Deserialize, Serialize};

pub const ENDPOINT_KNOWLEDGE_SCHEMA: &str = "routing_endpoint";
pub const CONNECTION_KNOWLEDGE_SCHEMA: &str = "routing_connection";
pub const ADDRESS_KNOWLEDGE_SCHEMA: &str = "address";
const KNOWLEDGE_NAMESPACE: &str = "canwu.correspondence";
const KNOWLEDGE_CUT_HASH_DOMAIN: &str = "canwu.correspondence.knowledge-cut.v1";
const TOPOLOGY_HASH_DOMAIN: &str = "canwu.correspondence.topology.v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KnownRoutingEndpoint {
    pub network_version: String,
    pub endpoint: RoutingEndpoint,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KnownRoutingConnection {
    pub network_version: String,
    pub connection: RoutingConnection,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KnownAddress {
    pub network_version: String,
    pub recipient: KnowledgeHolderRef,
    pub destination: canwu_api::RoutingNodeRef,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NetworkKnowledgeSeed {
    pub seed_key: String,
    pub holder: KnowledgeHolderRef,
    pub endpoints: Vec<KnownRoutingEndpoint>,
    pub connections: Vec<KnownRoutingConnection>,
    pub addresses: Vec<KnownAddress>,
}

#[must_use]
pub fn planning_knowledge_query() -> KnowledgeQuery {
    KnowledgeQuery {
        schemas: vec![
            schema_id(ADDRESS_KNOWLEDGE_SCHEMA),
            schema_id(CONNECTION_KNOWLEDGE_SCHEMA),
            schema_id(ENDPOINT_KNOWLEDGE_SCHEMA),
        ],
        view: KnowledgeHistoryView::CurrentHeads,
        limit: MAX_KNOWLEDGE_PAGE_SIZE,
        ..KnowledgeQuery::default()
    }
}

#[must_use]
pub fn correspondence_knowledge_schemas() -> Vec<PluginKnowledgeSchema> {
    [
        (
            ADDRESS_KNOWLEDGE_SCHEMA,
            "5a4ad47b7a51305f90582a09956a99f4f9ff194acf605e1f27bfed79c5992901",
        ),
        (
            CONNECTION_KNOWLEDGE_SCHEMA,
            "2f10c55b8504d952b97d947c53fd8abfceae919e4c2d0991f702b175d576639f",
        ),
        (
            ENDPOINT_KNOWLEDGE_SCHEMA,
            "fda7cd3aead8bd294844a478299109973b8a9304f39e5e1056909cc46819b3a7",
        ),
    ]
    .into_iter()
    .map(|(name, hash)| PluginKnowledgeSchema {
        id: schema_id(name),
        schema_hash: hash.to_owned(),
        writable: true,
        payload_schema: PayloadSchema::Any,
        subjects: Vec::new(),
    })
    .collect()
}

pub(crate) fn build_planning_snapshot(
    result: &KnowledgeQueryResult,
    recipient: &KnowledgeHolderRef,
    observed_at: SimTime,
) -> Result<(PlanningSnapshot, AddressResolution), String> {
    if result.next.is_some() {
        return Err("planning knowledge exceeds the bounded current-head query".to_owned());
    }
    let mut endpoint_facts = std::collections::BTreeMap::new();
    let mut connection_facts = std::collections::BTreeMap::new();
    let mut address_facts = std::collections::BTreeMap::new();
    for record in &result.records {
        let name = record.schema.kind.name.as_str();
        match name {
            ENDPOINT_KNOWLEDGE_SCHEMA => {
                let payload: KnownRoutingEndpoint = serde_json::from_value(record.payload.clone())
                    .map_err(|error| format!("routing endpoint knowledge is invalid: {error}"))?;
                let key = payload.endpoint.id.clone();
                replace_latest(
                    &mut endpoint_facts,
                    key,
                    record.learned_at,
                    record.id,
                    payload,
                );
            }
            CONNECTION_KNOWLEDGE_SCHEMA => {
                let payload: KnownRoutingConnection =
                    serde_json::from_value(record.payload.clone()).map_err(|error| {
                        format!("routing connection knowledge is invalid: {error}")
                    })?;
                let key = payload.connection.id.clone();
                replace_latest(
                    &mut connection_facts,
                    key,
                    record.learned_at,
                    record.id,
                    payload,
                );
            }
            ADDRESS_KNOWLEDGE_SCHEMA => {
                let payload: KnownAddress = serde_json::from_value(record.payload.clone())
                    .map_err(|error| format!("address knowledge is invalid: {error}"))?;
                replace_latest(
                    &mut address_facts,
                    payload.recipient.clone(),
                    record.learned_at,
                    record.id,
                    payload,
                );
            }
            _ => {}
        }
    }
    let (_, source_record, address) = address_facts
        .get(recipient)
        .ok_or_else(|| "carrier knowledge must contain one current recipient address".to_owned())?;
    let endpoints = endpoint_facts
        .values()
        .map(|(_, _, payload)| payload.endpoint.clone())
        .collect::<Vec<_>>();
    let connections = connection_facts
        .values()
        .map(|(_, _, payload)| payload.connection.clone())
        .collect::<Vec<_>>();
    let topology_version = canonical_hash(
        TOPOLOGY_HASH_DOMAIN,
        &(
            endpoint_facts
                .values()
                .map(|(_, _, payload)| payload)
                .collect::<Vec<_>>(),
            connection_facts
                .values()
                .map(|(_, _, payload)| payload)
                .collect::<Vec<_>>(),
        ),
    )
    .map_err(|error| error.to_string())?;
    let network = RoutingNetwork::new(topology_version.clone(), endpoints, connections)
        .map_err(|error| error.to_string())?;
    let knowledge_cut = canonical_hash(KNOWLEDGE_CUT_HASH_DOMAIN, &result.read_cut)
        .map_err(|error| error.to_string())?;
    let snapshot = PlanningSnapshot {
        observer: serde_json::to_string(&result.holder).map_err(|error| error.to_string())?,
        observed_at,
        valid_until: None,
        knowledge_cut,
        topology_version,
        timetable_version: None,
        network,
    };
    snapshot.validate().map_err(|error| error.to_string())?;
    Ok((
        snapshot,
        AddressResolution {
            recipient: recipient.clone(),
            destination: address.destination.clone(),
            resolved_at: observed_at,
            read_cut: result.read_cut.clone(),
            source_record: *source_record,
        },
    ))
}

fn replace_latest<K, V>(
    facts: &mut std::collections::BTreeMap<K, (SimTime, canwu_api::HolderKnowledgeRecordId, V)>,
    key: K,
    learned_at: SimTime,
    record: canwu_api::HolderKnowledgeRecordId,
    payload: V,
) where
    K: Ord,
{
    let replace = facts.get(&key).is_none_or(|(prior_at, prior_record, _)| {
        (learned_at, record) > (*prior_at, *prior_record)
    });
    if replace {
        facts.insert(key, (learned_at, record, payload));
    }
}

pub(crate) fn schema_id(name: &str) -> KnowledgeSchemaId {
    KnowledgeSchemaId::new(KnowledgeRecordKind::new(KNOWLEDGE_NAMESPACE, name), 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use canwu_api::{
        HolderKnowledgeRecordId, KnowledgeReadCut, KnowledgeRecordView, PersonId,
        RoutingConnectionRef, RoutingEndpointKind, RoutingNodeRef, SimDuration, TransferMode,
        TraversalModel,
    };

    #[test]
    fn latest_holder_fact_wins_by_local_record_id_at_the_same_time() {
        let holder = KnowledgeHolderRef::Person(PersonId::new(1));
        let recipient = KnowledgeHolderRef::Person(PersonId::new(2));
        let records = vec![
            view(
                &holder,
                ENDPOINT_KNOWLEDGE_SCHEMA,
                1,
                &KnownRoutingEndpoint {
                    network_version: "old".to_owned(),
                    endpoint: RoutingEndpoint {
                        id: RoutingNodeRef::new("origin"),
                        kind: RoutingEndpointKind::Settlement,
                    },
                },
            ),
            view(
                &holder,
                ENDPOINT_KNOWLEDGE_SCHEMA,
                2,
                &KnownRoutingEndpoint {
                    network_version: "new".to_owned(),
                    endpoint: RoutingEndpoint {
                        id: RoutingNodeRef::new("origin"),
                        kind: RoutingEndpointKind::TelegraphOffice,
                    },
                },
            ),
            view(
                &holder,
                ENDPOINT_KNOWLEDGE_SCHEMA,
                3,
                &KnownRoutingEndpoint {
                    network_version: "new".to_owned(),
                    endpoint: RoutingEndpoint {
                        id: RoutingNodeRef::new("destination"),
                        kind: RoutingEndpointKind::TelegraphOffice,
                    },
                },
            ),
            view(
                &holder,
                CONNECTION_KNOWLEDGE_SCHEMA,
                4,
                &connection("old", SimDuration::days(2)),
            ),
            view(
                &holder,
                CONNECTION_KNOWLEDGE_SCHEMA,
                5,
                &connection("new", SimDuration::hours(1)),
            ),
            view(
                &holder,
                ADDRESS_KNOWLEDGE_SCHEMA,
                6,
                &KnownAddress {
                    network_version: "old".to_owned(),
                    recipient: recipient.clone(),
                    destination: RoutingNodeRef::new("unknown"),
                },
            ),
            view(
                &holder,
                ADDRESS_KNOWLEDGE_SCHEMA,
                7,
                &KnownAddress {
                    network_version: "new".to_owned(),
                    recipient: recipient.clone(),
                    destination: RoutingNodeRef::new("destination"),
                },
            ),
        ];
        let result = KnowledgeQueryResult {
            holder,
            read_cut: KnowledgeReadCut {
                boundary: None,
                holder_projection_root: "projection".to_owned(),
                holder_overlay_root: None,
            },
            records,
            next: None,
        };

        let (snapshot, address) =
            build_planning_snapshot(&result, &recipient, SimTime::EPOCH).unwrap();
        assert_eq!(address.destination.as_str(), "destination");
        assert_eq!(address.source_record, HolderKnowledgeRecordId::new(7));
        assert_eq!(
            snapshot
                .network
                .endpoints
                .iter()
                .find(|endpoint| endpoint.id.as_str() == "origin")
                .unwrap()
                .kind,
            RoutingEndpointKind::TelegraphOffice
        );
        assert!(matches!(
            snapshot.network.connections[0].traversal,
            TraversalModel::Fixed { duration } if duration == SimDuration::hours(1)
        ));
    }

    fn connection(version: &str, duration: SimDuration) -> KnownRoutingConnection {
        KnownRoutingConnection {
            network_version: version.to_owned(),
            connection: RoutingConnection {
                id: RoutingConnectionRef::new("line"),
                from: RoutingNodeRef::new("origin"),
                to: RoutingNodeRef::new("destination"),
                mode: TransferMode::Signal,
                traversal: TraversalModel::Fixed { duration },
                available_from: None,
                available_until: None,
                risk_per_mille: 0,
                resource_cost: 1,
            },
        }
    }

    fn view(
        holder: &KnowledgeHolderRef,
        schema: &str,
        id: u64,
        payload: &impl Serialize,
    ) -> KnowledgeRecordView {
        KnowledgeRecordView {
            id: HolderKnowledgeRecordId::new(id),
            holder: holder.clone(),
            schema: schema_id(schema),
            subjects: Vec::new(),
            payload: serde_json::to_value(payload).unwrap(),
            as_of: None,
            learned_at: SimTime::EPOCH,
            confidence_per_mille: 1_000,
            supersedes: Vec::new(),
            contradicts: Vec::new(),
        }
    }
}
