use crate::model::AddressResolution;
use canwu_api::{
    CanwuError, ErrorCode, HolderKnowledgeRecordId, KnowledgeHistoryView, KnowledgeHolderRef,
    KnowledgeQuery, KnowledgeQueryResult, KnowledgeRecordKind, KnowledgeRecordView,
    KnowledgeSchemaId, MAX_KNOWLEDGE_PAGE_SIZE, PayloadSchema, PlanningSnapshot,
    PluginKnowledgeSchema, RoutingConnection, RoutingConnectionRef, RoutingEndpoint,
    RoutingNetwork, RoutingNodeRef, SimTime, SimulationView, canonical_hash,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const ENDPOINT_KNOWLEDGE_SCHEMA: &str = "routing_endpoint";
pub const CONNECTION_KNOWLEDGE_SCHEMA: &str = "routing_connection";
pub const ADDRESS_KNOWLEDGE_SCHEMA: &str = "address";
/// Knowledge schema name of the holder-relative
/// [`crate::CorrespondenceAttemptReport`].
pub const ATTEMPT_REPORT_KNOWLEDGE_SCHEMA: &str = "attempt_report";
const KNOWLEDGE_NAMESPACE: &str = "canwu.correspondence";
const KNOWLEDGE_CUT_HASH_DOMAIN: &str = "canwu.correspondence.knowledge-cut.v1";
const TOPOLOGY_HASH_DOMAIN: &str = "canwu.correspondence.topology.v1";
const PLANNING_READ_SET_HASH_DOMAIN: &str = "canwu.correspondence.planning-read-set.v1";

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
            ATTEMPT_REPORT_KNOWLEDGE_SCHEMA,
            "82bb41a4bff5a48c97e8c07ceded1ceb1f3dd1848eeb5dd6a12b93d5cd48da2b",
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

/// Read-set digest of the holder knowledge a planning snapshot was built from:
/// the canonical hash of the holder plus the versioned schema and
/// holder-relative ID of every endpoint and connection record admitted into
/// the network at the read cut.
///
/// Knowledge records are immutable, so a record ID pins its content. The
/// digest stays equal while the admitted records are unchanged, even when
/// unrelated holder records move the read cut itself, and changes when a newer
/// record replaces an admitted fact. It is evidence for what the planner read;
/// it is not a commitment over the holder ledger.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct KnowledgeReadCutDigest(pub String);

/// Builds a holder-relative planning snapshot inside a plugin system or
/// command handler.
///
/// Reads only the holder's current knowledge heads through
/// [`planning_knowledge_query`]; the caller must declare the
/// `canwu.core.knowledge` read, and no route or other world state is read. The
/// network admits exactly the endpoints and connections the holder's ledger
/// asserts at the read cut the view derives, taking the latest fact per
/// endpoint or connection ID; the snapshot's `knowledge_cut` commits to that
/// cut. Recipient addresses are not required. A known connection whose
/// endpoint the holder does not know fails the build rather than being
/// dropped.
///
/// The view read is omniscient system access, not actor authorization: derive
/// `holder` from the admitted authority (for example the command issuer), not
/// from an unvalidated payload.
pub fn planning_snapshot_from_holder_knowledge(
    view: &SimulationView<'_>,
    holder: &KnowledgeHolderRef,
    observed_at: SimTime,
) -> Result<(PlanningSnapshot, KnowledgeReadCutDigest), CanwuError> {
    let result = view.knowledge_records(holder.clone(), &planning_knowledge_query())?;
    planning_snapshot_from_knowledge_result(&result, observed_at)
}

/// Pure form of [`planning_snapshot_from_holder_knowledge`] over a query
/// result the caller already holds, such as a restricted viewer's
/// `query_knowledge(&planning_knowledge_query())`.
///
/// Records outside this crate's planning knowledge schemas are ignored, and
/// address records never enter the network. A paginated result is rejected
/// because a partial page cannot prove the holder's full network.
pub fn planning_snapshot_from_knowledge_result(
    result: &KnowledgeQueryResult,
    observed_at: SimTime,
) -> Result<(PlanningSnapshot, KnowledgeReadCutDigest), CanwuError> {
    if result.next.is_some() {
        return Err(CanwuError::new(
            ErrorCode::KnowledgeLimitExceeded,
            "planning knowledge exceeds the bounded current-head query",
        ));
    }
    let invalid = |message: String| CanwuError::new(ErrorCode::InvalidKnowledgeRecord, message);
    let facts = PlanningFacts::collect(result).map_err(invalid)?;
    let snapshot = facts.snapshot(result, observed_at).map_err(invalid)?;
    let digest = facts.read_set_digest(&result.holder)?;
    Ok((snapshot, digest))
}

/// The lifecycle's planning read: the carrier's current planning-knowledge
/// heads through the same bounded query and admission rule as
/// [`planning_snapshot_from_holder_knowledge`], plus the recipient address
/// resolved from that same read. A host that repeats the public builder for
/// the carrier at the same cut therefore sees the network the plugin planned
/// over.
pub(crate) fn carrier_planning_snapshot(
    view: &SimulationView<'_>,
    carrier: &KnowledgeHolderRef,
    recipient: &KnowledgeHolderRef,
    observed_at: SimTime,
) -> Result<(PlanningSnapshot, AddressResolution), CanwuError> {
    let result = view.knowledge_records(carrier.clone(), &planning_knowledge_query())?;
    build_planning_snapshot(&result, recipient, observed_at)
        .map_err(|message| CanwuError::new(ErrorCode::InvalidDomainRecord, message))
}

pub(crate) fn build_planning_snapshot(
    result: &KnowledgeQueryResult,
    recipient: &KnowledgeHolderRef,
    observed_at: SimTime,
) -> Result<(PlanningSnapshot, AddressResolution), String> {
    if result.next.is_some() {
        return Err("planning knowledge exceeds the bounded current-head query".to_owned());
    }
    let facts = PlanningFacts::collect(result)?;
    let address = facts
        .addresses
        .get(recipient)
        .ok_or_else(|| "carrier knowledge must contain one current recipient address".to_owned())?;
    let snapshot = facts.snapshot(result, observed_at)?;
    Ok((
        snapshot,
        AddressResolution {
            recipient: recipient.clone(),
            destination: address.payload.destination.clone(),
            resolved_at: observed_at,
            read_cut: result.read_cut.clone(),
            source_record: address.record,
        },
    ))
}

/// The latest holder fact per endpoint, connection, and recipient key. Shared
/// by the plugin and the public builders so their admission rule cannot drift.
struct PlanningFacts {
    endpoints: BTreeMap<RoutingNodeRef, HolderFact<KnownRoutingEndpoint>>,
    connections: BTreeMap<RoutingConnectionRef, HolderFact<KnownRoutingConnection>>,
    addresses: BTreeMap<KnowledgeHolderRef, HolderFact<KnownAddress>>,
}

struct HolderFact<V> {
    learned_at: SimTime,
    record: HolderKnowledgeRecordId,
    payload: V,
}

#[derive(Serialize)]
struct PlanningReadSetMaterial<'a> {
    holder: &'a KnowledgeHolderRef,
    records: Vec<(KnowledgeSchemaId, HolderKnowledgeRecordId)>,
}

impl PlanningFacts {
    fn collect(result: &KnowledgeQueryResult) -> Result<Self, String> {
        let endpoint_schema = schema_id(ENDPOINT_KNOWLEDGE_SCHEMA);
        let connection_schema = schema_id(CONNECTION_KNOWLEDGE_SCHEMA);
        let address_schema = schema_id(ADDRESS_KNOWLEDGE_SCHEMA);
        let mut facts = Self {
            endpoints: BTreeMap::new(),
            connections: BTreeMap::new(),
            addresses: BTreeMap::new(),
        };
        for record in &result.records {
            if record.schema == endpoint_schema {
                let payload: KnownRoutingEndpoint = serde_json::from_value(record.payload.clone())
                    .map_err(|error| format!("routing endpoint knowledge is invalid: {error}"))?;
                let key = payload.endpoint.id.clone();
                replace_latest(&mut facts.endpoints, key, record, payload);
            } else if record.schema == connection_schema {
                let payload: KnownRoutingConnection =
                    serde_json::from_value(record.payload.clone()).map_err(|error| {
                        format!("routing connection knowledge is invalid: {error}")
                    })?;
                let key = payload.connection.id.clone();
                replace_latest(&mut facts.connections, key, record, payload);
            } else if record.schema == address_schema {
                let payload: KnownAddress = serde_json::from_value(record.payload.clone())
                    .map_err(|error| format!("address knowledge is invalid: {error}"))?;
                let key = payload.recipient.clone();
                replace_latest(&mut facts.addresses, key, record, payload);
            }
        }
        Ok(facts)
    }

    fn snapshot(
        &self,
        result: &KnowledgeQueryResult,
        observed_at: SimTime,
    ) -> Result<PlanningSnapshot, String> {
        let endpoints = self
            .endpoints
            .values()
            .map(|fact| fact.payload.endpoint.clone())
            .collect::<Vec<_>>();
        let connections = self
            .connections
            .values()
            .map(|fact| fact.payload.connection.clone())
            .collect::<Vec<_>>();
        let topology_version = canonical_hash(
            TOPOLOGY_HASH_DOMAIN,
            &(
                self.endpoints
                    .values()
                    .map(|fact| &fact.payload)
                    .collect::<Vec<_>>(),
                self.connections
                    .values()
                    .map(|fact| &fact.payload)
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
        Ok(snapshot)
    }

    fn read_set_digest(
        &self,
        holder: &KnowledgeHolderRef,
    ) -> Result<KnowledgeReadCutDigest, CanwuError> {
        let endpoint_schema = schema_id(ENDPOINT_KNOWLEDGE_SCHEMA);
        let connection_schema = schema_id(CONNECTION_KNOWLEDGE_SCHEMA);
        let mut records = self
            .endpoints
            .values()
            .map(|fact| (endpoint_schema.clone(), fact.record))
            .chain(
                self.connections
                    .values()
                    .map(|fact| (connection_schema.clone(), fact.record)),
            )
            .collect::<Vec<_>>();
        records.sort();
        canonical_hash(
            PLANNING_READ_SET_HASH_DOMAIN,
            &PlanningReadSetMaterial { holder, records },
        )
        .map(KnowledgeReadCutDigest)
    }
}

fn replace_latest<K, V>(
    facts: &mut BTreeMap<K, HolderFact<V>>,
    key: K,
    record: &KnowledgeRecordView,
    payload: V,
) where
    K: Ord,
{
    let replace = facts
        .get(&key)
        .is_none_or(|prior| (record.learned_at, record.id) > (prior.learned_at, prior.record));
    if replace {
        facts.insert(
            key,
            HolderFact {
                learned_at: record.learned_at,
                record: record.id,
                payload,
            },
        );
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
