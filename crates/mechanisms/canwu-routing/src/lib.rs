//! Deterministic routing over a versioned, time-dependent transfer network.
//!
//! This crate is deliberately pure: it does not read simulation state, mutate
//! capacity, draw randomness, schedule work, or resolve information recipients.

#![allow(clippy::missing_errors_doc)]

use canwu_time::{SimDuration, SimTime};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, VecDeque};
use std::fmt;

pub const ROUTING_ALGORITHM_VERSION: &str = "canwu-routing.v1";

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct RoutingNodeRef(String);

impl RoutingNodeRef {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct RoutingConnectionRef(String);

impl RoutingConnectionRef {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutingEndpointKind {
    Settlement,
    RelayStation,
    RailwayStation,
    Port,
    Airport,
    TelegraphOffice,
    DeliveryDistrict,
    MilitaryPosition,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferMode {
    Foot,
    Horse,
    RoadVehicle,
    RiverBoat,
    Sea,
    Rail,
    Air,
    Signal,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RoutingEndpoint {
    pub id: RoutingNodeRef,
    pub kind: RoutingEndpointKind,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DepartureSlot {
    pub departure_at: SimTime,
    pub duration: SimDuration,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DurationSample {
    pub from: SimTime,
    pub duration: SimDuration,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TraversalModel {
    Fixed { duration: SimDuration },
    Departures { slots: Vec<DepartureSlot> },
    Piecewise { samples: Vec<DurationSample> },
}

impl TraversalModel {
    fn validate(&self) -> Result<(), RoutingError> {
        match self {
            Self::Fixed { duration } if duration.is_negative() => Err(
                RoutingError::InvalidNetwork("negative traversal duration".to_owned()),
            ),
            Self::Fixed { .. } => Ok(()),
            Self::Departures { slots } => {
                if slots
                    .windows(2)
                    .any(|pair| pair[0].departure_at >= pair[1].departure_at)
                    || slots.iter().any(|slot| slot.duration.is_negative())
                {
                    return Err(RoutingError::InvalidNetwork(
                        "departure slots must be sorted and non-negative".to_owned(),
                    ));
                }
                Ok(())
            }
            Self::Piecewise { samples } => {
                if samples.windows(2).any(|pair| pair[0].from >= pair[1].from)
                    || samples.iter().any(|sample| sample.duration.is_negative())
                {
                    return Err(RoutingError::InvalidNetwork(
                        "duration samples must be sorted and non-negative".to_owned(),
                    ));
                }
                Ok(())
            }
        }
    }

    fn traverse_after(&self, at: SimTime) -> Option<(SimTime, SimTime)> {
        match self {
            Self::Fixed { duration } => Some((at, at.checked_add(*duration)?)),
            Self::Departures { slots } => slots.iter().find_map(|slot| {
                (slot.departure_at >= at).then(|| {
                    Some((
                        slot.departure_at,
                        slot.departure_at.checked_add(slot.duration)?,
                    ))
                })?
            }),
            Self::Piecewise { samples } => samples
                .iter()
                .rev()
                .find(|sample| sample.from <= at)
                .and_then(|sample| Some((at, at.checked_add(sample.duration)?))),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RoutingConnection {
    pub id: RoutingConnectionRef,
    pub from: RoutingNodeRef,
    pub to: RoutingNodeRef,
    pub mode: TransferMode,
    pub traversal: TraversalModel,
    pub available_from: Option<SimTime>,
    pub available_until: Option<SimTime>,
    pub risk_per_mille: u32,
    pub resource_cost: u64,
}

impl RoutingConnection {
    fn validate(&self, endpoints: &BTreeSet<RoutingNodeRef>) -> Result<(), RoutingError> {
        if !endpoints.contains(&self.from) || !endpoints.contains(&self.to) || self.from == self.to
        {
            return Err(RoutingError::InvalidNetwork(format!(
                "connection {} has invalid endpoints",
                self.id.0
            )));
        }
        if self.risk_per_mille > 1_000 {
            return Err(RoutingError::InvalidNetwork(format!(
                "connection {} risk exceeds 1000 per mille",
                self.id.0
            )));
        }
        if let (Some(start), Some(end)) = (self.available_from, self.available_until)
            && start > end
        {
            return Err(RoutingError::InvalidNetwork(format!(
                "connection {} availability is inverted",
                self.id.0
            )));
        }
        self.traversal.validate()
    }

    fn traverse_after(&self, at: SimTime) -> Option<(SimTime, SimTime)> {
        let at = self.available_from.map_or(at, |start| at.max(start));
        if self.available_until.is_some_and(|end| at > end) {
            return None;
        }
        let result = self.traversal.traverse_after(at)?;
        self.available_until
            .is_none_or(|end| result.1 <= end)
            .then_some(result)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RoutingNetwork {
    pub version: String,
    pub endpoints: Vec<RoutingEndpoint>,
    pub connections: Vec<RoutingConnection>,
}

impl RoutingNetwork {
    pub fn new(
        version: impl Into<String>,
        mut endpoints: Vec<RoutingEndpoint>,
        mut connections: Vec<RoutingConnection>,
    ) -> Result<Self, RoutingError> {
        endpoints.sort_by(|left, right| left.id.cmp(&right.id));
        connections.sort_by(|left, right| left.id.cmp(&right.id));
        let endpoint_ids = endpoints
            .iter()
            .map(|endpoint| endpoint.id.clone())
            .collect::<Vec<_>>();
        if endpoint_ids.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(RoutingError::InvalidNetwork(
                "duplicate endpoint".to_owned(),
            ));
        }
        let endpoint_set = endpoint_ids.iter().cloned().collect::<BTreeSet<_>>();
        if connections.windows(2).any(|pair| pair[0].id == pair[1].id) {
            return Err(RoutingError::InvalidNetwork(
                "duplicate connection".to_owned(),
            ));
        }
        for connection in &connections {
            connection.validate(&endpoint_set)?;
        }
        Ok(Self {
            version: version.into(),
            endpoints,
            connections,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PlanningSnapshot {
    pub observer: String,
    pub observed_at: SimTime,
    pub valid_until: Option<SimTime>,
    pub knowledge_cut: String,
    pub topology_version: String,
    pub timetable_version: Option<String>,
    pub network: RoutingNetwork,
}

impl PlanningSnapshot {
    pub fn validate(&self) -> Result<(), RoutingError> {
        if self
            .valid_until
            .is_some_and(|until| until < self.observed_at)
        {
            return Err(RoutingError::InvalidSnapshot(
                "planning snapshot expires before it is observed".to_owned(),
            ));
        }
        if self.topology_version != self.network.version {
            return Err(RoutingError::InvalidSnapshot(
                "topology version does not match network version".to_owned(),
            ));
        }
        Ok(())
    }

    #[must_use]
    pub fn digest(&self) -> String {
        canonical_digest(self)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutingAlgorithm {
    FifoDijkstraV1,
    BoundedLabelCorrectingV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RoutingPolicy {
    pub version: String,
    pub algorithm: RoutingAlgorithm,
    pub allowed_modes: BTreeSet<TransferMode>,
    pub max_arrival_at: Option<SimTime>,
    pub max_expanded_nodes: usize,
    pub max_transfers: usize,
    pub max_risk_per_mille: u64,
}

impl Default for RoutingPolicy {
    fn default() -> Self {
        Self {
            version: "canwu-routing.policy.v1".to_owned(),
            algorithm: RoutingAlgorithm::FifoDijkstraV1,
            allowed_modes: BTreeSet::new(),
            max_arrival_at: None,
            max_expanded_nodes: 10_000,
            max_transfers: 64,
            max_risk_per_mille: 1_000,
        }
    }
}

impl RoutingPolicy {
    fn allows(&self, mode: TransferMode) -> bool {
        self.allowed_modes.is_empty() || self.allowed_modes.contains(&mode)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RoutingRequest {
    pub origin: RoutingNodeRef,
    pub destination: RoutingNodeRef,
    pub departure_at: SimTime,
    pub policy: RoutingPolicy,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct RouteLeg {
    pub connection: RoutingConnectionRef,
    pub from: RoutingNodeRef,
    pub to: RoutingNodeRef,
    pub mode: TransferMode,
    pub planned_departure_at: SimTime,
    pub planned_arrival_at: SimTime,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct RouteCost {
    pub estimated_arrival_at: SimTime,
    pub risk_per_mille: u64,
    pub resource_cost: u64,
    pub transfers: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RoutePlan {
    pub algorithm_version: String,
    pub policy_version: String,
    pub planning_snapshot_digest: String,
    pub origin: RoutingNodeRef,
    pub destination: RoutingNodeRef,
    pub departure_at: SimTime,
    pub estimated_arrival_at: SimTime,
    pub cost: RouteCost,
    pub legs: Vec<RouteLeg>,
    pub digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum RoutingError {
    InvalidNetwork(String),
    InvalidSnapshot(String),
    UnknownOrigin,
    UnknownDestination,
    NoKnownRoute,
    SearchHorizonExceeded,
    ExpansionBudgetExceeded,
    RequirementsUnsatisfied,
    ArithmeticOverflow,
}

impl fmt::Display for RoutingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidNetwork(message) => {
                write!(formatter, "invalid routing network: {message}")
            }
            Self::InvalidSnapshot(message) => {
                write!(formatter, "invalid planning snapshot: {message}")
            }
            Self::UnknownOrigin => {
                formatter.write_str("routing origin is not present in the network")
            }
            Self::UnknownDestination => {
                formatter.write_str("routing destination is not present in the network")
            }
            Self::NoKnownRoute => formatter.write_str("no route satisfies the routing policy"),
            Self::SearchHorizonExceeded => {
                formatter.write_str("route departure is outside the planning snapshot horizon")
            }
            Self::ExpansionBudgetExceeded => {
                formatter.write_str("routing expansion budget was exceeded")
            }
            Self::RequirementsUnsatisfied => {
                formatter.write_str("routing requirements are not satisfied")
            }
            Self::ArithmeticOverflow => formatter.write_str("routing arithmetic overflowed"),
        }
    }
}

impl std::error::Error for RoutingError {}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Label {
    node: RoutingNodeRef,
    arrival_at: SimTime,
    risk_per_mille: u64,
    resource_cost: u64,
    transfers: usize,
    legs: Vec<RouteLeg>,
}

impl Label {
    fn key(&self) -> (&SimTime, u64, u64, usize, &Vec<RouteLeg>) {
        (
            &self.arrival_at,
            self.risk_per_mille,
            self.resource_cost,
            self.transfers,
            &self.legs,
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct QueueEntry {
    node: RoutingNodeRef,
    arrival_at: SimTime,
    risk_per_mille: u64,
    resource_cost: u64,
    transfers: usize,
    legs: Vec<RouteLeg>,
}

impl QueueEntry {
    fn from_label(label: &Label) -> Self {
        Self {
            node: label.node.clone(),
            arrival_at: label.arrival_at,
            risk_per_mille: label.risk_per_mille,
            resource_cost: label.resource_cost,
            transfers: label.transfers,
            legs: label.legs.clone(),
        }
    }

    fn key(&self) -> (&SimTime, u64, u64, usize, &RoutingNodeRef, &Vec<RouteLeg>) {
        (
            &self.arrival_at,
            self.risk_per_mille,
            self.resource_cost,
            self.transfers,
            &self.node,
            &self.legs,
        )
    }
}

impl Ord for QueueEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        other.key().cmp(&self.key())
    }
}

impl PartialOrd for QueueEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn better(left: &Label, right: &Label) -> bool {
    left.key() < right.key()
}

fn adjacency(network: &RoutingNetwork) -> BTreeMap<RoutingNodeRef, Vec<&RoutingConnection>> {
    let mut index = BTreeMap::new();
    for connection in &network.connections {
        index
            .entry(connection.from.clone())
            .or_insert_with(Vec::new)
            .push(connection);
    }
    index
}

fn outgoing<'a>(
    index: &'a BTreeMap<RoutingNodeRef, Vec<&'a RoutingConnection>>,
    node: &RoutingNodeRef,
) -> impl Iterator<Item = &'a RoutingConnection> {
    index.get(node).into_iter().flatten().copied()
}

fn extend(label: &Label, connection: &RoutingConnection, policy: &RoutingPolicy) -> Option<Label> {
    if !policy.allows(connection.mode) || label.transfers >= policy.max_transfers {
        return None;
    }
    let (departure_at, arrival_at) = connection.traverse_after(label.arrival_at)?;
    let risk_per_mille = label
        .risk_per_mille
        .checked_add(u64::from(connection.risk_per_mille))?;
    let resource_cost = label.resource_cost.checked_add(connection.resource_cost)?;
    let transfers = label.transfers + 1;
    if risk_per_mille > policy.max_risk_per_mille
        || policy
            .max_arrival_at
            .is_some_and(|limit| arrival_at > limit)
    {
        return None;
    }
    let mut legs = label.legs.clone();
    legs.push(RouteLeg {
        connection: connection.id.clone(),
        from: connection.from.clone(),
        to: connection.to.clone(),
        mode: connection.mode,
        planned_departure_at: departure_at,
        planned_arrival_at: arrival_at,
    });
    Some(Label {
        node: connection.to.clone(),
        arrival_at,
        risk_per_mille,
        resource_cost,
        transfers,
        legs,
    })
}

fn solve_dijkstra(
    request: &RoutingRequest,
    index: &BTreeMap<RoutingNodeRef, Vec<&RoutingConnection>>,
) -> Result<Label, RoutingError> {
    let mut labels = BTreeMap::<RoutingNodeRef, Label>::new();
    let start = Label {
        node: request.origin.clone(),
        arrival_at: request.departure_at,
        risk_per_mille: 0,
        resource_cost: 0,
        transfers: 0,
        legs: Vec::new(),
    };
    labels.insert(request.origin.clone(), start.clone());
    let mut queue = BinaryHeap::from([QueueEntry::from_label(&start)]);
    let mut expanded = 0;
    while let Some(entry) = queue.pop() {
        expanded += 1;
        if expanded > request.policy.max_expanded_nodes {
            return Err(RoutingError::ExpansionBudgetExceeded);
        }
        let Some(current) = labels.get(&entry.node).cloned() else {
            continue;
        };
        if current.arrival_at != entry.arrival_at
            || current.risk_per_mille != entry.risk_per_mille
            || current.resource_cost != entry.resource_cost
            || current.transfers != entry.transfers
            || current.legs != entry.legs
        {
            // The label may have been improved since this queue entry was added.
            continue;
        }
        if entry.node == request.destination {
            return Ok(current);
        }
        for connection in outgoing(index, &entry.node) {
            let Some(candidate) = extend(&current, connection, &request.policy) else {
                continue;
            };
            let replace = labels
                .get(&candidate.node)
                .is_none_or(|existing| better(&candidate, existing));
            if replace {
                queue.push(QueueEntry::from_label(&candidate));
                labels.insert(candidate.node.clone(), candidate);
            }
        }
    }
    Err(RoutingError::NoKnownRoute)
}

fn solve_label_correcting(
    request: &RoutingRequest,
    index: &BTreeMap<RoutingNodeRef, Vec<&RoutingConnection>>,
) -> Result<Label, RoutingError> {
    let mut labels = BTreeMap::<RoutingNodeRef, Vec<Label>>::new();
    let start = Label {
        node: request.origin.clone(),
        arrival_at: request.departure_at,
        risk_per_mille: 0,
        resource_cost: 0,
        transfers: 0,
        legs: Vec::new(),
    };
    labels.insert(request.origin.clone(), vec![start.clone()]);
    let mut queue = VecDeque::from([start]);
    let mut expanded = 0;
    while let Some(current) = queue.pop_front() {
        expanded += 1;
        if expanded > request.policy.max_expanded_nodes {
            return Err(RoutingError::ExpansionBudgetExceeded);
        }
        for connection in outgoing(index, &current.node) {
            let Some(candidate) = extend(&current, connection, &request.policy) else {
                continue;
            };
            let node_labels = labels.entry(candidate.node.clone()).or_default();
            if node_labels.iter().any(|existing| existing == &candidate) {
                continue;
            }
            node_labels.push(candidate.clone());
            queue.push_back(candidate);
        }
    }
    labels
        .remove(&request.destination)
        .and_then(|candidates| {
            candidates
                .into_iter()
                .min_by(|left, right| left.key().cmp(&right.key()))
        })
        .ok_or(RoutingError::NoKnownRoute)
}

pub fn plan_route(
    snapshot: &PlanningSnapshot,
    request: &RoutingRequest,
) -> Result<RoutePlan, RoutingError> {
    snapshot.validate()?;
    if !snapshot
        .network
        .endpoints
        .iter()
        .any(|endpoint| endpoint.id == request.origin)
    {
        return Err(RoutingError::UnknownOrigin);
    }
    if !snapshot
        .network
        .endpoints
        .iter()
        .any(|endpoint| endpoint.id == request.destination)
    {
        return Err(RoutingError::UnknownDestination);
    }
    if snapshot
        .valid_until
        .is_some_and(|until| request.departure_at > until)
    {
        return Err(RoutingError::SearchHorizonExceeded);
    }
    if request.origin == request.destination {
        let mut plan = RoutePlan {
            algorithm_version: ROUTING_ALGORITHM_VERSION.to_owned(),
            policy_version: request.policy.version.clone(),
            planning_snapshot_digest: snapshot.digest(),
            origin: request.origin.clone(),
            destination: request.destination.clone(),
            departure_at: request.departure_at,
            estimated_arrival_at: request.departure_at,
            cost: RouteCost {
                estimated_arrival_at: request.departure_at,
                ..RouteCost::default()
            },
            legs: Vec::new(),
            digest: String::new(),
        };
        plan.digest = canonical_digest(&plan_without_digest(&plan));
        return Ok(plan);
    }
    let index = adjacency(&snapshot.network);
    let label = match request.policy.algorithm {
        RoutingAlgorithm::FifoDijkstraV1 => solve_dijkstra(request, &index)?,
        RoutingAlgorithm::BoundedLabelCorrectingV1 => solve_label_correcting(request, &index)?,
    };
    let mut plan = RoutePlan {
        algorithm_version: ROUTING_ALGORITHM_VERSION.to_owned(),
        policy_version: request.policy.version.clone(),
        planning_snapshot_digest: snapshot.digest(),
        origin: request.origin.clone(),
        destination: request.destination.clone(),
        departure_at: request.departure_at,
        estimated_arrival_at: label.arrival_at,
        cost: RouteCost {
            estimated_arrival_at: label.arrival_at,
            risk_per_mille: label.risk_per_mille,
            resource_cost: label.resource_cost,
            transfers: label.transfers,
        },
        legs: label.legs,
        digest: String::new(),
    };
    plan.digest = canonical_digest(&plan_without_digest(&plan));
    Ok(plan)
}

fn plan_without_digest(
    plan: &RoutePlan,
) -> (
    &str,
    &str,
    &str,
    &RoutingNodeRef,
    &RoutingNodeRef,
    SimTime,
    SimTime,
    &RouteCost,
    &Vec<RouteLeg>,
) {
    (
        &plan.algorithm_version,
        &plan.policy_version,
        &plan.planning_snapshot_digest,
        &plan.origin,
        &plan.destination,
        plan.departure_at,
        plan.estimated_arrival_at,
        &plan.cost,
        &plan.legs,
    )
}

fn canonical_digest<T: Serialize>(value: &T) -> String {
    let bytes = serde_json::to_vec(value).expect("routing types must be serializable");
    blake3::hash(&bytes).to_hex().to_string()
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct RoutingCache {
    entries: BTreeMap<String, RoutePlan>,
}

impl RoutingCache {
    #[must_use]
    pub fn key(snapshot: &PlanningSnapshot, request: &RoutingRequest) -> String {
        canonical_digest(&(snapshot.digest(), request))
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<&RoutePlan> {
        self.entries.get(key)
    }

    pub fn insert(&mut self, key: String, plan: RoutePlan) {
        self.entries.insert(key, plan);
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint(id: &str) -> RoutingEndpoint {
        RoutingEndpoint {
            id: RoutingNodeRef::new(id),
            kind: RoutingEndpointKind::Settlement,
        }
    }

    fn connection(id: &str, from: &str, to: &str, minutes: i64) -> RoutingConnection {
        RoutingConnection {
            id: RoutingConnectionRef::new(id),
            from: RoutingNodeRef::new(from),
            to: RoutingNodeRef::new(to),
            mode: TransferMode::Horse,
            traversal: TraversalModel::Fixed {
                duration: SimDuration::minutes(minutes),
            },
            available_from: None,
            available_until: None,
            risk_per_mille: 0,
            resource_cost: 0,
        }
    }

    fn snapshot(connections: Vec<RoutingConnection>) -> PlanningSnapshot {
        snapshot_with_endpoints(connections, ["a", "b", "c"])
    }

    fn snapshot_with_endpoints<const N: usize>(
        connections: Vec<RoutingConnection>,
        endpoint_ids: [&str; N],
    ) -> PlanningSnapshot {
        let network = RoutingNetwork::new(
            "roads.v1",
            endpoint_ids.into_iter().map(endpoint).collect(),
            connections,
        )
        .unwrap();
        PlanningSnapshot {
            observer: "courier".to_owned(),
            observed_at: SimTime::EPOCH,
            valid_until: None,
            knowledge_cut: "knowledge:1".to_owned(),
            topology_version: "roads.v1".to_owned(),
            timetable_version: None,
            network,
        }
    }

    #[test]
    fn chooses_deterministic_earliest_arrival_across_multiple_legs() {
        let snapshot = snapshot(vec![
            connection("a-c", "a", "c", 90),
            connection("a-b", "a", "b", 30),
            connection("b-c", "b", "c", 30),
        ]);
        let plan = plan_route(
            &snapshot,
            &RoutingRequest {
                origin: RoutingNodeRef::new("a"),
                destination: RoutingNodeRef::new("c"),
                departure_at: SimTime::EPOCH,
                policy: RoutingPolicy::default(),
            },
        )
        .unwrap();
        assert_eq!(plan.estimated_arrival_at, SimTime::from_minutes(60));
        assert_eq!(
            plan.legs
                .iter()
                .map(|leg| leg.connection.clone())
                .collect::<Vec<_>>(),
            vec![
                RoutingConnectionRef::new("a-b"),
                RoutingConnectionRef::new("b-c")
            ]
        );
    }

    #[test]
    fn scheduled_connections_wait_for_the_next_departure() {
        let mut rail = connection("rail", "a", "c", 1);
        rail.mode = TransferMode::Rail;
        rail.traversal = TraversalModel::Departures {
            slots: vec![DepartureSlot {
                departure_at: SimTime::from_minutes(60),
                duration: SimDuration::minutes(20),
            }],
        };
        let snapshot = snapshot(vec![rail]);
        let plan = plan_route(
            &snapshot,
            &RoutingRequest {
                origin: RoutingNodeRef::new("a"),
                destination: RoutingNodeRef::new("c"),
                departure_at: SimTime::from_minutes(10),
                policy: RoutingPolicy::default(),
            },
        )
        .unwrap();
        assert_eq!(plan.estimated_arrival_at, SimTime::from_minutes(80));
    }

    #[test]
    fn label_correcting_handles_piecewise_duration_changes() {
        let mut edge = connection("edge", "a", "c", 10);
        edge.traversal = TraversalModel::Piecewise {
            samples: vec![
                DurationSample {
                    from: SimTime::EPOCH,
                    duration: SimDuration::minutes(100),
                },
                DurationSample {
                    from: SimTime::from_minutes(10),
                    duration: SimDuration::minutes(5),
                },
            ],
        };
        let snapshot = snapshot(vec![edge]);
        let policy = RoutingPolicy {
            algorithm: RoutingAlgorithm::BoundedLabelCorrectingV1,
            ..RoutingPolicy::default()
        };
        let plan = plan_route(
            &snapshot,
            &RoutingRequest {
                origin: RoutingNodeRef::new("a"),
                destination: RoutingNodeRef::new("c"),
                departure_at: SimTime::from_minutes(10),
                policy,
            },
        )
        .unwrap();
        assert_eq!(plan.estimated_arrival_at, SimTime::from_minutes(15));
    }

    #[test]
    fn label_correcting_keeps_a_later_label_for_a_faster_non_fifo_departure() {
        let mut final_leg = connection("b-d", "b", "d", 100);
        final_leg.traversal = TraversalModel::Piecewise {
            samples: vec![
                DurationSample {
                    from: SimTime::EPOCH,
                    duration: SimDuration::minutes(100),
                },
                DurationSample {
                    from: SimTime::from_minutes(10),
                    duration: SimDuration::minutes(1),
                },
            ],
        };
        let snapshot = snapshot_with_endpoints(
            vec![
                connection("a-b", "a", "b", 5),
                connection("a-c", "a", "c", 10),
                connection("c-b", "c", "b", 0),
                final_leg,
            ],
            ["a", "b", "c", "d"],
        );
        let policy = RoutingPolicy {
            algorithm: RoutingAlgorithm::BoundedLabelCorrectingV1,
            ..RoutingPolicy::default()
        };
        let plan = plan_route(
            &snapshot,
            &RoutingRequest {
                origin: RoutingNodeRef::new("a"),
                destination: RoutingNodeRef::new("d"),
                departure_at: SimTime::EPOCH,
                policy,
            },
        )
        .unwrap();
        assert_eq!(plan.estimated_arrival_at, SimTime::from_minutes(11));
        assert_eq!(plan.legs[1].connection, RoutingConnectionRef::new("c-b"));
    }

    #[test]
    fn cache_key_changes_with_snapshot_or_policy() {
        let snapshot = snapshot(vec![connection("a-c", "a", "c", 10)]);
        let request = RoutingRequest {
            origin: RoutingNodeRef::new("a"),
            destination: RoutingNodeRef::new("c"),
            departure_at: SimTime::EPOCH,
            policy: RoutingPolicy::default(),
        };
        let first = RoutingCache::key(&snapshot, &request);
        let mut changed = request.clone();
        changed.policy.version = "policy.v2".to_owned();
        assert_ne!(first, RoutingCache::key(&snapshot, &changed));
    }
}
