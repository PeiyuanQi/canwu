//! Deterministic routing over a versioned, time-dependent transfer network.
//!
//! This crate is deliberately pure: it does not read simulation state, mutate
//! capacity, draw randomness, schedule work, or resolve information recipients.
//!
//! A [`PlanningSnapshot`] is what an observer knows of the network at a read
//! cut. [`plan_route`] answers one request from a snapshot; a host that asks
//! many questions of the same snapshot compiles it once into a
//! [`RoutingIndex`] and plans against that, so a query no longer rebuilds the
//! adjacency index or hashes the network. [`RoutingIndex::reobserve`] moves an
//! index to a new read cut without recompiling an unchanged network.
//!
//! Timetables are part of the network: [`TraversalModel::Recurring`] states a
//! periodic service in integer simulated time (one period, sorted offsets, one
//! duration) instead of one absolute slot per departure. Time-windowed
//! conditions that close an edge or change its cost travel beside the network
//! in an [`EdgeConditionSet`] and are applied by
//! [`RoutingIndex::plan_route_with_conditions`] without rebuilding the
//! network.

#![allow(clippy::missing_errors_doc)]

use canwu_time::{SimDuration, SimTime};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, VecDeque};
use std::fmt;
use std::rc::Rc;
use std::sync::Arc;

/// The routing algorithm and digest scheme of the plans this crate produces.
/// `v2` (0.18.0) composes the planning-snapshot digest from the network digest
/// and adds the edge-condition digest to the plan digest.
pub const ROUTING_ALGORITHM_VERSION: &str = "canwu-routing.v2";

/// The most offsets one [`TraversalModel::Recurring`] departure pattern may
/// hold. A denser pattern over a long period is a shorter period.
pub const MAX_RECURRING_OFFSETS: usize = 4_096;

/// The most overlays one [`EdgeConditionSet`] may hold.
pub const MAX_EDGE_CONDITIONS: usize = 65_536;

/// The most overlays one [`EdgeConditionSet`] may hold for one connection.
pub const MAX_EDGE_CONDITIONS_PER_CONNECTION: usize = 1_024;

/// The largest per-mille multiplier an [`EdgeConditionEffect::Adjusted`]
/// overlay may apply (one thousand times the base value).
pub const MAX_CONDITION_PER_MILLE: u32 = 1_000_000;

const PER_MILLE: u32 = 1_000;
const SNAPSHOT_DIGEST_DOMAIN: &str = "canwu-routing.planning-snapshot.v2";
const NETWORK_DIGEST_DOMAIN: &str = "canwu-routing.network.v2";
const EDGE_CONDITIONS_DIGEST_DOMAIN: &str = "canwu-routing.edge-conditions.v1";

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

/// The stable id of one [`EdgeConditionOverlay`] inside an
/// [`EdgeConditionSet`].
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct EdgeConditionRef(String);

impl EdgeConditionRef {
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

/// How long a connection takes and when it can be entered.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TraversalModel {
    /// Depart at once; the traversal takes `duration`.
    Fixed { duration: SimDuration },
    /// Depart only at the listed absolute instants (strictly increasing).
    Departures { slots: Vec<DepartureSlot> },
    /// Depart at once; the duration is that of the latest sample starting at
    /// or before the departure instant.
    Piecewise { samples: Vec<DurationSample> },
    /// Depart at `EPOCH + k * period + offset` for every integer `k` (negative
    /// `k` included) and every listed offset; every departure takes
    /// `duration`. `period` and `duration` are positive, the offsets are
    /// strictly increasing, at least one and at most
    /// [`MAX_RECURRING_OFFSETS`], and each lies in `[0, period)`. A pattern
    /// anchored elsewhere folds its anchor into the offsets
    /// (`(anchor - EPOCH) mod period`); the connection's `available_from` and
    /// `available_until` bound the season the pattern runs.
    Recurring {
        period: SimDuration,
        offsets: Vec<SimDuration>,
        duration: SimDuration,
    },
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
            Self::Recurring {
                period,
                offsets,
                duration,
            } => {
                if period.as_minutes() <= 0 || duration.as_minutes() <= 0 {
                    return Err(RoutingError::InvalidNetwork(
                        "recurring period and duration must be positive".to_owned(),
                    ));
                }
                if offsets.is_empty() || offsets.len() > MAX_RECURRING_OFFSETS {
                    return Err(RoutingError::InvalidNetwork(format!(
                        "recurring departures need 1 to {MAX_RECURRING_OFFSETS} offsets"
                    )));
                }
                if offsets.windows(2).any(|pair| pair[0] >= pair[1])
                    || offsets
                        .iter()
                        .any(|offset| offset.is_negative() || offset >= period)
                {
                    return Err(RoutingError::InvalidNetwork(
                        "recurring offsets must be strictly increasing and inside the period"
                            .to_owned(),
                    ));
                }
                Ok(())
            }
        }
    }

    /// The earliest departure at or after `at` and the duration of that
    /// traversal, ignoring the connection's availability window and any edge
    /// condition. `None` when no departure follows `at` or the departure is
    /// not representable.
    ///
    /// The lookup is a binary search: O(log slots) for `Departures`,
    /// O(log samples) for `Piecewise`, and O(log offsets) for `Recurring` at
    /// any `at`, computed exactly in integer minutes. For a model that
    /// [`RoutingNetwork::new`] would refuse the answer is unspecified, but the
    /// call never panics.
    #[must_use]
    pub fn next_departure(&self, at: SimTime) -> Option<(SimTime, SimDuration)> {
        match self {
            Self::Fixed { duration } => Some((at, *duration)),
            Self::Departures { slots } => {
                let index = slots.partition_point(|slot| slot.departure_at < at);
                slots
                    .get(index)
                    .map(|slot| (slot.departure_at, slot.duration))
            }
            Self::Piecewise { samples } => {
                let index = samples.partition_point(|sample| sample.from <= at);
                let sample = samples.get(index.checked_sub(1)?)?;
                Some((at, sample.duration))
            }
            Self::Recurring {
                period,
                offsets,
                duration,
            } => {
                let period_minutes = i128::from(period.as_minutes());
                let first = offsets.first()?;
                if period_minutes <= 0 {
                    return None;
                }
                let at_minutes = i128::from(at.as_minutes());
                let cycle_start = at_minutes - at_minutes.rem_euclid(period_minutes);
                let phase = at_minutes - cycle_start;
                let index =
                    offsets.partition_point(|offset| i128::from(offset.as_minutes()) < phase);
                let departure = match offsets.get(index) {
                    Some(offset) => cycle_start + i128::from(offset.as_minutes()),
                    None => cycle_start + period_minutes + i128::from(first.as_minutes()),
                };
                let departure = SimTime::from_minutes(i64::try_from(departure).ok()?);
                Some((departure, *duration))
            }
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
    fn validate(&self, endpoints_known: bool) -> Result<(), RoutingError> {
        if !endpoints_known || self.from == self.to {
            return Err(RoutingError::InvalidNetwork(format!(
                "connection {} has invalid endpoints",
                self.id.0
            )));
        }
        if self.risk_per_mille > PER_MILLE {
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

    /// The traversal entered at or after `at`: the availability window, then
    /// the earliest departure outside every closure, then the adjustment in
    /// force at that departure instant.
    fn traverse_after(
        &self,
        at: SimTime,
        conditions: Option<&ConnectionConditions>,
    ) -> Option<EdgeTraversal> {
        let at = self.available_from.map_or(at, |start| at.max(start));
        if self.available_until.is_some_and(|end| at > end) {
            return None;
        }
        let closures = conditions.map_or(&[][..], |conditions| &conditions.closures);
        let (departure_at, duration) = open_departure(&self.traversal, at, closures)?;
        let (duration, risk_per_mille, resource_cost) =
            match conditions.and_then(|conditions| conditions.adjustment_at(departure_at)) {
                Some(adjustment) => (
                    scale_duration(duration, adjustment.duration_per_mille)?,
                    adjustment.risk_per_mille.unwrap_or(self.risk_per_mille),
                    scale_cost(self.resource_cost, adjustment.resource_cost_per_mille)?,
                ),
                None => (duration, self.risk_per_mille, self.resource_cost),
            };
        let arrival_at = departure_at.checked_add(duration)?;
        if self.available_until.is_some_and(|end| arrival_at > end) {
            return None;
        }
        Some(EdgeTraversal {
            departure_at,
            arrival_at,
            risk_per_mille,
            resource_cost,
        })
    }
}

struct EdgeTraversal {
    departure_at: SimTime,
    arrival_at: SimTime,
    risk_per_mille: u32,
    resource_cost: u64,
}

/// The earliest departure of `model` at or after `at` that is not inside a
/// closure. `closures` are sorted, disjoint, and non-adjacent half-open
/// windows, so each jump passes one closure and the loop ends after at most
/// `closures.len() + 1` lookups.
fn open_departure(
    model: &TraversalModel,
    mut at: SimTime,
    closures: &[(SimTime, SimTime)],
) -> Option<(SimTime, SimDuration)> {
    loop {
        let (departure_at, duration) = model.next_departure(at)?;
        let index = closures.partition_point(|(from, _)| *from <= departure_at);
        match index.checked_sub(1).map(|index| closures[index]) {
            Some((_, until)) if departure_at < until => at = until,
            _ => return Some((departure_at, duration)),
        }
    }
}

fn ceil_per_mille(value: u128, per_mille: u32) -> u128 {
    (value * u128::from(per_mille)).div_ceil(u128::from(PER_MILLE))
}

fn scale_duration(duration: SimDuration, per_mille: u32) -> Option<SimDuration> {
    let minutes = u128::try_from(duration.as_minutes()).ok()?;
    let scaled = i64::try_from(ceil_per_mille(minutes, per_mille)).ok()?;
    Some(SimDuration::minutes(scaled))
}

fn scale_cost(cost: u64, per_mille: u32) -> Option<u64> {
    u64::try_from(ceil_per_mille(u128::from(cost), per_mille)).ok()
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
        let network = Self {
            version: version.into(),
            endpoints,
            connections,
        };
        Topology::compile(&network)?;
        Ok(network)
    }

    /// The content digest of the network: its version, endpoints, and
    /// connections in their stored order.
    #[must_use]
    pub fn digest(&self) -> String {
        canonical_digest(&(NETWORK_DIGEST_DOMAIN, self))
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
        validate_cut(self.observed_at, self.valid_until)?;
        if self.topology_version != self.network.version {
            return Err(RoutingError::InvalidSnapshot(
                "topology version does not match network version".to_owned(),
            ));
        }
        Ok(())
    }

    /// The snapshot digest: the read cut and versions composed with
    /// [`RoutingNetwork::digest`]. Equal to [`RoutingIndex::digest`] of an
    /// index compiled from this snapshot.
    #[must_use]
    pub fn digest(&self) -> String {
        snapshot_digest(
            &self.observer,
            self.observed_at,
            self.valid_until,
            &self.knowledge_cut,
            &self.topology_version,
            self.timetable_version.as_deref(),
            &self.network.digest(),
        )
    }
}

fn validate_cut(observed_at: SimTime, valid_until: Option<SimTime>) -> Result<(), RoutingError> {
    if valid_until.is_some_and(|until| until < observed_at) {
        return Err(RoutingError::InvalidSnapshot(
            "planning snapshot expires before it is observed".to_owned(),
        ));
    }
    Ok(())
}

fn snapshot_digest(
    observer: &str,
    observed_at: SimTime,
    valid_until: Option<SimTime>,
    knowledge_cut: &str,
    topology_version: &str,
    timetable_version: Option<&str>,
    network_digest: &str,
) -> String {
    canonical_digest(&(
        SNAPSHOT_DIGEST_DOMAIN,
        observer,
        observed_at,
        valid_until,
        knowledge_cut,
        topology_version,
        timetable_version,
        network_digest,
    ))
}

/// What an [`EdgeConditionOverlay`] does to departures inside its window.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EdgeConditionEffect {
    /// No departure inside the window: a traveller waits for the first
    /// departure at or after the window's end (or of any later closure).
    Closed,
    /// A departure inside the window takes `duration_per_mille / 1000` of
    /// the base duration and costs `resource_cost_per_mille / 1000` of the
    /// base resource cost, both rounded up to a whole unit, and carries
    /// `risk_per_mille` instead of the connection's risk when it is set.
    /// `duration_per_mille` is 1 to [`MAX_CONDITION_PER_MILLE`],
    /// `resource_cost_per_mille` 0 to [`MAX_CONDITION_PER_MILLE`], and
    /// `risk_per_mille` at most 1000.
    Adjusted {
        duration_per_mille: u32,
        resource_cost_per_mille: u32,
        risk_per_mille: Option<u32>,
    },
}

/// A time-windowed change to one connection, applied by departure instant:
/// a departure at `t` with `window_from <= t < window_until` takes the
/// effect.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EdgeConditionOverlay {
    pub id: EdgeConditionRef,
    pub connection: RoutingConnectionRef,
    pub window_from: SimTime,
    pub window_until: SimTime,
    pub effect: EdgeConditionEffect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct EdgeAdjustment {
    from: SimTime,
    until: SimTime,
    duration_per_mille: u32,
    resource_cost_per_mille: u32,
    risk_per_mille: Option<u32>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct ConnectionConditions {
    /// Merged closure windows: sorted, disjoint, and non-adjacent.
    closures: Vec<(SimTime, SimTime)>,
    /// Adjustment windows sorted by start; they never overlap.
    adjustments: Vec<EdgeAdjustment>,
}

impl ConnectionConditions {
    /// Merges one connection's closures and orders its adjustments, refusing
    /// overlapping adjustments.
    fn build(
        connection: &RoutingConnectionRef,
        mut entries: Vec<&EdgeConditionOverlay>,
    ) -> Result<Self, RoutingError> {
        if entries.len() > MAX_EDGE_CONDITIONS_PER_CONNECTION {
            return Err(RoutingError::InvalidConditions(format!(
                "connection {} has more than {MAX_EDGE_CONDITIONS_PER_CONNECTION} edge conditions",
                connection.0
            )));
        }
        entries.sort_by(|left, right| {
            (left.window_from, &left.id).cmp(&(right.window_from, &right.id))
        });
        let mut conditions = Self::default();
        let mut last_adjustment: Option<&EdgeConditionOverlay> = None;
        for entry in entries {
            match entry.effect {
                EdgeConditionEffect::Closed => match conditions.closures.last_mut() {
                    Some((_, until)) if entry.window_from <= *until => {
                        *until = (*until).max(entry.window_until);
                    }
                    _ => conditions
                        .closures
                        .push((entry.window_from, entry.window_until)),
                },
                EdgeConditionEffect::Adjusted {
                    duration_per_mille,
                    resource_cost_per_mille,
                    risk_per_mille,
                } => {
                    if let Some(previous) = last_adjustment
                        && entry.window_from < previous.window_until
                    {
                        return Err(RoutingError::InvalidConditions(format!(
                            "edge conditions {} and {} overlap on connection {}",
                            previous.id.0, entry.id.0, connection.0
                        )));
                    }
                    last_adjustment = Some(entry);
                    conditions.adjustments.push(EdgeAdjustment {
                        from: entry.window_from,
                        until: entry.window_until,
                        duration_per_mille,
                        resource_cost_per_mille,
                        risk_per_mille,
                    });
                }
            }
        }
        Ok(conditions)
    }

    fn adjustment_at(&self, at: SimTime) -> Option<&EdgeAdjustment> {
        let index = self
            .adjustments
            .partition_point(|adjustment| adjustment.from <= at);
        let adjustment = self.adjustments.get(index.checked_sub(1)?)?;
        (at < adjustment.until).then_some(adjustment)
    }
}

/// A validated set of time-windowed edge conditions, indexed by connection.
///
/// Built once (O(n log n) in the overlays) and applied by
/// [`RoutingIndex::plan_route_with_conditions`] with one ordered lookup per
/// relaxed edge, so a condition change never rebuilds the network. Closures
/// may overlap anything; two `Adjusted` windows on one connection must not
/// overlap. An overlay naming a connection the queried network does not hold
/// has no effect, so one set can serve several observers' networks.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EdgeConditionSet {
    overlays: Vec<EdgeConditionOverlay>,
    by_connection: BTreeMap<RoutingConnectionRef, ConnectionConditions>,
    digest: String,
}

impl Default for EdgeConditionSet {
    fn default() -> Self {
        Self {
            overlays: Vec::new(),
            by_connection: BTreeMap::new(),
            digest: edge_conditions_digest(&[]),
        }
    }
}

impl EdgeConditionSet {
    pub fn new(mut overlays: Vec<EdgeConditionOverlay>) -> Result<Self, RoutingError> {
        if overlays.len() > MAX_EDGE_CONDITIONS {
            return Err(RoutingError::InvalidConditions(format!(
                "at most {MAX_EDGE_CONDITIONS} edge conditions"
            )));
        }
        overlays.sort_by(|left, right| left.id.cmp(&right.id));
        if let Some(pair) = overlays.windows(2).find(|pair| pair[0].id == pair[1].id) {
            return Err(RoutingError::InvalidConditions(format!(
                "duplicate edge condition {}",
                pair[0].id.0
            )));
        }
        let mut grouped = BTreeMap::<RoutingConnectionRef, Vec<&EdgeConditionOverlay>>::new();
        for overlay in &overlays {
            overlay.validate()?;
            grouped
                .entry(overlay.connection.clone())
                .or_default()
                .push(overlay);
        }
        let by_connection = grouped
            .into_iter()
            .map(|(connection, entries)| {
                let conditions = ConnectionConditions::build(&connection, entries)?;
                Ok((connection, conditions))
            })
            .collect::<Result<BTreeMap<_, _>, RoutingError>>()?;
        let digest = edge_conditions_digest(&overlays);
        Ok(Self {
            overlays,
            by_connection,
            digest,
        })
    }

    /// The overlays, sorted by id.
    #[must_use]
    pub fn overlays(&self) -> &[EdgeConditionOverlay] {
        &self.overlays
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.overlays.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.overlays.len()
    }

    /// The content digest of the overlays, recorded by every plan made under
    /// a non-empty set as [`RoutePlan::edge_conditions_digest`].
    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }

    fn applied_digest(&self) -> Option<String> {
        (!self.is_empty()).then(|| self.digest.clone())
    }

    fn for_connection(&self, connection: &RoutingConnectionRef) -> Option<&ConnectionConditions> {
        self.by_connection.get(connection)
    }
}

impl EdgeConditionOverlay {
    fn validate(&self) -> Result<(), RoutingError> {
        if self.window_from >= self.window_until {
            return Err(RoutingError::InvalidConditions(format!(
                "edge condition {} has an empty or inverted window",
                self.id.0
            )));
        }
        if let EdgeConditionEffect::Adjusted {
            duration_per_mille,
            resource_cost_per_mille,
            risk_per_mille,
        } = self.effect
            && (!(1..=MAX_CONDITION_PER_MILLE).contains(&duration_per_mille)
                || resource_cost_per_mille > MAX_CONDITION_PER_MILLE
                || risk_per_mille.is_some_and(|risk| risk > PER_MILLE))
        {
            return Err(RoutingError::InvalidConditions(format!(
                "edge condition {} has an out-of-range adjustment",
                self.id.0
            )));
        }
        Ok(())
    }
}

fn edge_conditions_digest(overlays: &[EdgeConditionOverlay]) -> String {
    canonical_digest(&(EDGE_CONDITIONS_DIGEST_DOMAIN, overlays))
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
    /// The [`EdgeConditionSet::digest`] the plan was made under; `None` when
    /// no condition set, or an empty one, was applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge_conditions_digest: Option<String>,
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
    InvalidConditions(String),
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
            Self::InvalidConditions(message) => {
                write!(formatter, "invalid edge conditions: {message}")
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

/// The compiled adjacency of a network: dense node ranks (the endpoints in
/// id order), each node's outgoing connections in stored order, and each
/// connection's rank in id order, which orders route legs exactly as
/// [`RouteLeg`]'s `Ord` does because connection ids are unique.
#[derive(Clone, Debug)]
struct Topology {
    /// Endpoint positions in id order; a node's rank is its index here.
    node_order: Vec<usize>,
    /// `first_edge[rank]..first_edge[rank + 1]` indexes `edges`.
    first_edge: Vec<usize>,
    /// Connection positions grouped by source rank, in stored order.
    edges: Vec<usize>,
    /// Per connection position: the rank of its target node.
    targets: Vec<usize>,
    /// Per connection position: its rank among connection ids.
    connection_ranks: Vec<usize>,
    /// Connection positions in id order (the inverse of `connection_ranks`).
    connections_by_id: Vec<usize>,
}

impl Topology {
    fn compile(network: &RoutingNetwork) -> Result<Self, RoutingError> {
        let endpoints = &network.endpoints;
        let mut node_order = (0..endpoints.len()).collect::<Vec<_>>();
        node_order.sort_by(|left, right| endpoints[*left].id.cmp(&endpoints[*right].id));
        if node_order
            .windows(2)
            .any(|pair| endpoints[pair[0]].id == endpoints[pair[1]].id)
        {
            return Err(RoutingError::InvalidNetwork(
                "duplicate endpoint".to_owned(),
            ));
        }
        let rank_of = |node: &RoutingNodeRef| {
            node_order
                .binary_search_by(|position| endpoints[*position].id.cmp(node))
                .ok()
        };
        let connections = &network.connections;
        let mut by_id = (0..connections.len()).collect::<Vec<_>>();
        by_id.sort_by(|left, right| connections[*left].id.cmp(&connections[*right].id));
        if by_id
            .windows(2)
            .any(|pair| connections[pair[0]].id == connections[pair[1]].id)
        {
            return Err(RoutingError::InvalidNetwork(
                "duplicate connection".to_owned(),
            ));
        }
        let mut connection_ranks = vec![0; connections.len()];
        for (rank, position) in by_id.iter().enumerate() {
            connection_ranks[*position] = rank;
        }
        let mut sources = Vec::with_capacity(connections.len());
        let mut targets = Vec::with_capacity(connections.len());
        let mut first_edge = vec![0; endpoints.len() + 1];
        for connection in connections {
            let ranks = rank_of(&connection.from).zip(rank_of(&connection.to));
            connection.validate(ranks.is_some())?;
            let Some((source, target)) = ranks else {
                continue;
            };
            sources.push(source);
            targets.push(target);
            first_edge[source + 1] += 1;
        }
        for rank in 0..endpoints.len() {
            first_edge[rank + 1] += first_edge[rank];
        }
        let mut cursor = first_edge.clone();
        let mut edges = vec![0; connections.len()];
        for (position, source) in sources.into_iter().enumerate() {
            edges[cursor[source]] = position;
            cursor[source] += 1;
        }
        Ok(Self {
            node_order,
            first_edge,
            edges,
            targets,
            connection_ranks,
            connections_by_id: by_id,
        })
    }

    fn rank(&self, network: &RoutingNetwork, node: &RoutingNodeRef) -> Option<usize> {
        self.node_order
            .binary_search_by(|position| network.endpoints[*position].id.cmp(node))
            .ok()
    }

    fn has_connection(&self, network: &RoutingNetwork, id: &RoutingConnectionRef) -> bool {
        self.connections_by_id
            .binary_search_by(|position| network.connections[*position].id.cmp(id))
            .is_ok()
    }

    fn outgoing(&self, rank: usize) -> &[usize] {
        &self.edges[self.first_edge[rank]..self.first_edge[rank + 1]]
    }
}

#[derive(Debug)]
struct CompiledNetwork {
    network: RoutingNetwork,
    digest: String,
    topology: Topology,
}

/// A planning snapshot compiled for repeated queries.
///
/// Compiling validates the snapshot and its network, hashes the network once,
/// and builds a dense adjacency index; every query then costs only the
/// search. The compiled network is shared (`Arc`), so cloning an index or
/// moving it to a new read cut with [`Self::reobserve`] is cheap, and an index
/// may be sent to and shared between threads. A host keeps one index per
/// network revision (its `topology_version` and `timetable_version`) and
/// compiles a new one when the network an observer knows changes.
///
/// Plans from an index are identical to [`plan_route`] on the same snapshot,
/// digests included.
#[derive(Clone, Debug)]
pub struct RoutingIndex {
    observer: String,
    observed_at: SimTime,
    valid_until: Option<SimTime>,
    knowledge_cut: String,
    timetable_version: Option<String>,
    compiled: Arc<CompiledNetwork>,
    digest: String,
}

impl RoutingIndex {
    pub fn new(snapshot: PlanningSnapshot) -> Result<Self, RoutingError> {
        snapshot.validate()?;
        let topology = Topology::compile(&snapshot.network)?;
        let network_digest = snapshot.network.digest();
        let compiled = Arc::new(CompiledNetwork {
            network: snapshot.network,
            digest: network_digest,
            topology,
        });
        Self::at_cut(
            &compiled,
            snapshot.observer,
            snapshot.observed_at,
            snapshot.valid_until,
            snapshot.knowledge_cut,
            snapshot.timetable_version,
        )
    }

    /// The same compiled network at another read cut, without recompiling.
    ///
    /// The caller names the network revision the observer knows at the new
    /// cut; a `topology_version` or `timetable_version` other than this
    /// index's is refused as `InvalidSnapshot`, and the caller compiles a new
    /// index. The versions are the caller's statement: equal versions are
    /// trusted to name the same network content.
    pub fn reobserve(
        &self,
        topology_version: &str,
        timetable_version: Option<&str>,
        observer: impl Into<String>,
        observed_at: SimTime,
        valid_until: Option<SimTime>,
        knowledge_cut: impl Into<String>,
    ) -> Result<Self, RoutingError> {
        if topology_version != self.compiled.network.version
            || timetable_version != self.timetable_version.as_deref()
        {
            return Err(RoutingError::InvalidSnapshot(
                "the network revision differs from the index's; compile a new index".to_owned(),
            ));
        }
        Self::at_cut(
            &self.compiled,
            observer.into(),
            observed_at,
            valid_until,
            knowledge_cut.into(),
            self.timetable_version.clone(),
        )
    }

    fn at_cut(
        compiled: &Arc<CompiledNetwork>,
        observer: String,
        observed_at: SimTime,
        valid_until: Option<SimTime>,
        knowledge_cut: String,
        timetable_version: Option<String>,
    ) -> Result<Self, RoutingError> {
        validate_cut(observed_at, valid_until)?;
        let digest = snapshot_digest(
            &observer,
            observed_at,
            valid_until,
            &knowledge_cut,
            &compiled.network.version,
            timetable_version.as_deref(),
            &compiled.digest,
        );
        Ok(Self {
            observer,
            observed_at,
            valid_until,
            knowledge_cut,
            timetable_version,
            compiled: Arc::clone(compiled),
            digest,
        })
    }

    /// The planning-snapshot digest every plan from this index records.
    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }

    #[must_use]
    pub fn network_digest(&self) -> &str {
        &self.compiled.digest
    }

    #[must_use]
    pub fn network(&self) -> &RoutingNetwork {
        &self.compiled.network
    }

    #[must_use]
    pub fn observer(&self) -> &str {
        &self.observer
    }

    #[must_use]
    pub fn observed_at(&self) -> SimTime {
        self.observed_at
    }

    #[must_use]
    pub fn valid_until(&self) -> Option<SimTime> {
        self.valid_until
    }

    #[must_use]
    pub fn knowledge_cut(&self) -> &str {
        &self.knowledge_cut
    }

    #[must_use]
    pub fn topology_version(&self) -> &str {
        &self.compiled.network.version
    }

    #[must_use]
    pub fn timetable_version(&self) -> Option<&str> {
        self.timetable_version.as_deref()
    }

    /// The equivalent planning snapshot (clones the network).
    #[must_use]
    pub fn to_snapshot(&self) -> PlanningSnapshot {
        PlanningSnapshot {
            observer: self.observer.clone(),
            observed_at: self.observed_at,
            valid_until: self.valid_until,
            knowledge_cut: self.knowledge_cut.clone(),
            topology_version: self.compiled.network.version.clone(),
            timetable_version: self.timetable_version.clone(),
            network: self.compiled.network.clone(),
        }
    }

    pub fn plan_route(&self, request: &RoutingRequest) -> Result<RoutePlan, RoutingError> {
        self.plan(request, None)
    }

    /// Plans under time-windowed edge conditions; the plan records the set's
    /// digest unless the set is empty.
    pub fn plan_route_with_conditions(
        &self,
        request: &RoutingRequest,
        conditions: &EdgeConditionSet,
    ) -> Result<RoutePlan, RoutingError> {
        self.plan(request, Some(conditions))
    }

    /// The overlays of `conditions` that name a connection this index's
    /// network does not hold, in id order. They have no effect on its plans;
    /// a host checks a set against the networks it serves with this (a
    /// misspelled connection id shows up here). O(n log connections).
    #[must_use]
    pub fn unmatched_conditions<'a>(
        &self,
        conditions: &'a EdgeConditionSet,
    ) -> Vec<&'a EdgeConditionOverlay> {
        conditions
            .overlays()
            .iter()
            .filter(|overlay| {
                !self
                    .compiled
                    .topology
                    .has_connection(&self.compiled.network, &overlay.connection)
            })
            .collect()
    }

    fn plan(
        &self,
        request: &RoutingRequest,
        conditions: Option<&EdgeConditionSet>,
    ) -> Result<RoutePlan, RoutingError> {
        let search = Search {
            network: &self.compiled.network,
            topology: &self.compiled.topology,
            conditions: conditions.filter(|conditions| !conditions.is_empty()),
        };
        let label = search.solve(request, self.valid_until)?;
        Ok(finish_plan(
            request,
            self.digest.clone(),
            search.conditions.and_then(EdgeConditionSet::applied_digest),
            &label,
            &search,
        ))
    }
}

/// Plans one request from a snapshot. Validates and compiles the snapshot and
/// hashes its network on every call; compile a [`RoutingIndex`] to answer
/// several requests from one snapshot.
pub fn plan_route(
    snapshot: &PlanningSnapshot,
    request: &RoutingRequest,
) -> Result<RoutePlan, RoutingError> {
    snapshot.validate()?;
    let topology = Topology::compile(&snapshot.network)?;
    let search = Search {
        network: &snapshot.network,
        topology: &topology,
        conditions: None,
    };
    let label = search.solve(request, snapshot.valid_until)?;
    Ok(finish_plan(
        request,
        snapshot.digest(),
        None,
        &label,
        &search,
    ))
}

/// One step of a route under construction. Steps form a persistent list from
/// the newest leg back to the origin, so extending a label allocates one
/// step instead of copying the route.
#[derive(Debug)]
struct Step {
    connection: usize,
    connection_rank: usize,
    departure_at: SimTime,
    arrival_at: SimTime,
    depth: usize,
    parent: Path,
}

type Path = Option<Rc<Step>>;

impl Step {
    fn key(&self) -> (usize, SimTime, SimTime) {
        (self.connection_rank, self.departure_at, self.arrival_at)
    }
}

fn depth(path: &Path) -> usize {
    path.as_ref().map_or(0, |step| step.depth)
}

/// Compares two routes leg by leg from the origin, as `Vec<RouteLeg>`'s
/// `Ord` does.
fn compare_paths(left: &Path, right: &Path) -> Ordering {
    if depth(left) != depth(right) {
        return collect_keys(left).cmp(&collect_keys(right));
    }
    // Walk both routes back towards the origin until they share a step; the
    // difference nearest the origin decides.
    let (mut left, mut right) = (left.as_deref(), right.as_deref());
    let mut decided = Ordering::Equal;
    while let (Some(left_step), Some(right_step)) = (left, right) {
        if std::ptr::eq(left_step, right_step) {
            break;
        }
        let ordering = left_step.key().cmp(&right_step.key());
        if ordering != Ordering::Equal {
            decided = ordering;
        }
        left = left_step.parent.as_deref();
        right = right_step.parent.as_deref();
    }
    decided
}

fn collect_keys(path: &Path) -> Vec<(usize, SimTime, SimTime)> {
    let mut keys = Vec::with_capacity(depth(path));
    let mut step = path.as_deref();
    while let Some(current) = step {
        keys.push(current.key());
        step = current.parent.as_deref();
    }
    keys.reverse();
    keys
}

#[derive(Clone, Debug)]
struct Label {
    arrival_at: SimTime,
    risk_per_mille: u64,
    resource_cost: u64,
    transfers: usize,
    path: Path,
}

impl Label {
    fn start(departure_at: SimTime) -> Self {
        Self {
            arrival_at: departure_at,
            risk_per_mille: 0,
            resource_cost: 0,
            transfers: 0,
            path: None,
        }
    }

    fn scalar_key(&self) -> (SimTime, u64, u64, usize) {
        (
            self.arrival_at,
            self.risk_per_mille,
            self.resource_cost,
            self.transfers,
        )
    }

    /// The label order: arrival, risk, resource cost, transfers, then the
    /// legs from the origin.
    fn cmp_key(&self, other: &Self) -> Ordering {
        self.scalar_key()
            .cmp(&other.scalar_key())
            .then_with(|| compare_paths(&self.path, &other.path))
    }

    /// The same label: equal scalars and the same route object. Labels only
    /// ever improve strictly, so this matches full equality.
    fn is(&self, other: &Self) -> bool {
        self.scalar_key() == other.scalar_key()
            && match (&self.path, &other.path) {
                (Some(left), Some(right)) => Rc::ptr_eq(left, right),
                (None, None) => true,
                _ => false,
            }
    }
}

struct QueueEntry {
    node: usize,
    label: Label,
}

impl QueueEntry {
    fn cmp_key(&self, other: &Self) -> Ordering {
        self.label
            .scalar_key()
            .cmp(&other.label.scalar_key())
            .then_with(|| self.node.cmp(&other.node))
            .then_with(|| compare_paths(&self.label.path, &other.label.path))
    }
}

impl PartialEq for QueueEntry {
    fn eq(&self, other: &Self) -> bool {
        self.cmp_key(other) == Ordering::Equal
    }
}

impl Eq for QueueEntry {}

impl Ord for QueueEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        other.cmp_key(self)
    }
}

impl PartialOrd for QueueEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

struct Search<'a> {
    network: &'a RoutingNetwork,
    topology: &'a Topology,
    conditions: Option<&'a EdgeConditionSet>,
}

impl Search<'_> {
    fn solve(
        &self,
        request: &RoutingRequest,
        valid_until: Option<SimTime>,
    ) -> Result<Label, RoutingError> {
        let origin = self
            .topology
            .rank(self.network, &request.origin)
            .ok_or(RoutingError::UnknownOrigin)?;
        let destination = self
            .topology
            .rank(self.network, &request.destination)
            .ok_or(RoutingError::UnknownDestination)?;
        if valid_until.is_some_and(|until| request.departure_at > until) {
            return Err(RoutingError::SearchHorizonExceeded);
        }
        if origin == destination {
            return Ok(Label::start(request.departure_at));
        }
        match request.policy.algorithm {
            RoutingAlgorithm::FifoDijkstraV1 => self.solve_dijkstra(request, origin, destination),
            RoutingAlgorithm::BoundedLabelCorrectingV1 => {
                self.solve_label_correcting(request, origin, destination)
            }
        }
    }

    fn extend(&self, label: &Label, connection: usize, policy: &RoutingPolicy) -> Option<Label> {
        let edge = &self.network.connections[connection];
        if !policy.allows(edge.mode) || label.transfers >= policy.max_transfers {
            return None;
        }
        let conditions = self
            .conditions
            .and_then(|conditions| conditions.for_connection(&edge.id));
        let traversal = edge.traverse_after(label.arrival_at, conditions)?;
        let risk_per_mille = label
            .risk_per_mille
            .checked_add(u64::from(traversal.risk_per_mille))?;
        let resource_cost = label.resource_cost.checked_add(traversal.resource_cost)?;
        let transfers = label.transfers + 1;
        if risk_per_mille > policy.max_risk_per_mille
            || policy
                .max_arrival_at
                .is_some_and(|limit| traversal.arrival_at > limit)
        {
            return None;
        }
        Some(Label {
            arrival_at: traversal.arrival_at,
            risk_per_mille,
            resource_cost,
            transfers,
            path: Some(Rc::new(Step {
                connection,
                connection_rank: self.topology.connection_ranks[connection],
                departure_at: traversal.departure_at,
                arrival_at: traversal.arrival_at,
                depth: transfers,
                parent: label.path.clone(),
            })),
        })
    }

    fn solve_dijkstra(
        &self,
        request: &RoutingRequest,
        origin: usize,
        destination: usize,
    ) -> Result<Label, RoutingError> {
        let mut labels = vec![None::<Label>; self.topology.node_order.len()];
        let start = Label::start(request.departure_at);
        labels[origin] = Some(start.clone());
        let mut queue = BinaryHeap::from([QueueEntry {
            node: origin,
            label: start,
        }]);
        let mut expanded = 0;
        while let Some(entry) = queue.pop() {
            expanded += 1;
            if expanded > request.policy.max_expanded_nodes {
                return Err(RoutingError::ExpansionBudgetExceeded);
            }
            let Some(current) = labels[entry.node].clone() else {
                continue;
            };
            if !current.is(&entry.label) {
                // The label may have been improved since this queue entry was added.
                continue;
            }
            if entry.node == destination {
                return Ok(current);
            }
            for &connection in self.topology.outgoing(entry.node) {
                let Some(candidate) = self.extend(&current, connection, &request.policy) else {
                    continue;
                };
                let target = self.topology.targets[connection];
                let replace = labels[target]
                    .as_ref()
                    .is_none_or(|existing| candidate.cmp_key(existing) == Ordering::Less);
                if replace {
                    queue.push(QueueEntry {
                        node: target,
                        label: candidate.clone(),
                    });
                    labels[target] = Some(candidate);
                }
            }
        }
        Err(RoutingError::NoKnownRoute)
    }

    /// Expands every label in first-in, first-out order within the budget and
    /// keeps the best label reaching the destination. No duplicate check is
    /// needed: a candidate is its parent's route plus one leg, each label is
    /// expanded once, and connection ids are unique (`Topology::compile`
    /// refuses a duplicate when the route is planned), so two candidates never
    /// hold the same sequence of legs.
    fn solve_label_correcting(
        &self,
        request: &RoutingRequest,
        origin: usize,
        destination: usize,
    ) -> Result<Label, RoutingError> {
        let mut queue = VecDeque::from([(origin, Label::start(request.departure_at))]);
        let mut best: Option<Label> = None;
        let mut expanded = 0;
        while let Some((node, current)) = queue.pop_front() {
            expanded += 1;
            if expanded > request.policy.max_expanded_nodes {
                return Err(RoutingError::ExpansionBudgetExceeded);
            }
            for &connection in self.topology.outgoing(node) {
                let Some(candidate) = self.extend(&current, connection, &request.policy) else {
                    continue;
                };
                let target = self.topology.targets[connection];
                if target == destination
                    && best
                        .as_ref()
                        .is_none_or(|best| candidate.cmp_key(best) == Ordering::Less)
                {
                    best = Some(candidate.clone());
                }
                queue.push_back((target, candidate));
            }
        }
        best.ok_or(RoutingError::NoKnownRoute)
    }

    fn legs(&self, path: &Path) -> Vec<RouteLeg> {
        let mut legs = Vec::with_capacity(depth(path));
        let mut step = path.as_deref();
        while let Some(current) = step {
            let connection = &self.network.connections[current.connection];
            legs.push(RouteLeg {
                connection: connection.id.clone(),
                from: connection.from.clone(),
                to: connection.to.clone(),
                mode: connection.mode,
                planned_departure_at: current.departure_at,
                planned_arrival_at: current.arrival_at,
            });
            step = current.parent.as_deref();
        }
        legs.reverse();
        legs
    }
}

fn finish_plan(
    request: &RoutingRequest,
    planning_snapshot_digest: String,
    edge_conditions_digest: Option<String>,
    label: &Label,
    search: &Search<'_>,
) -> RoutePlan {
    let mut plan = RoutePlan {
        algorithm_version: ROUTING_ALGORITHM_VERSION.to_owned(),
        policy_version: request.policy.version.clone(),
        planning_snapshot_digest,
        edge_conditions_digest,
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
        legs: search.legs(&label.path),
        digest: String::new(),
    };
    plan.digest = canonical_digest(&plan_without_digest(&plan));
    plan
}

type PlanDigestInput<'a> = (
    &'a str,
    &'a str,
    &'a str,
    Option<&'a str>,
    &'a RoutingNodeRef,
    &'a RoutingNodeRef,
    SimTime,
    SimTime,
    &'a RouteCost,
    &'a Vec<RouteLeg>,
);

fn plan_without_digest(plan: &RoutePlan) -> PlanDigestInput<'_> {
    (
        &plan.algorithm_version,
        &plan.policy_version,
        &plan.planning_snapshot_digest,
        plan.edge_conditions_digest.as_deref(),
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
    /// The cache key of a request against a snapshot. Hashes the network;
    /// prefer [`Self::index_key`] when the snapshot is compiled.
    #[must_use]
    pub fn key(snapshot: &PlanningSnapshot, request: &RoutingRequest) -> String {
        cache_key(&snapshot.digest(), None, request)
    }

    /// The cache key of a request against a compiled index and an optional
    /// condition set, without hashing the network. Equal to [`Self::key`] for
    /// the same snapshot when no condition set, or an empty one, is given.
    #[must_use]
    pub fn index_key(
        index: &RoutingIndex,
        conditions: Option<&EdgeConditionSet>,
        request: &RoutingRequest,
    ) -> String {
        cache_key(
            index.digest(),
            conditions
                .filter(|conditions| !conditions.is_empty())
                .map(EdgeConditionSet::digest),
            request,
        )
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

fn cache_key(
    planning_snapshot_digest: &str,
    edge_conditions_digest: Option<&str>,
    request: &RoutingRequest,
) -> String {
    canonical_digest(&(planning_snapshot_digest, edge_conditions_digest, request))
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
