//! Holder-relative movement reports.
//!
//! A report is a projection of one execution as it stood at the observer's
//! cut: the current time for the operator and owner, and the current time
//! minus the grant delay for a delayed remote observer. Only the operator and
//! owner see the execution detail (state, revision reasons, handoffs,
//! bookings, and failure reasons); a remote observer sees coarse progress.

use crate::PLUGIN_NAMESPACE;
use crate::model::{
    MovementError, MovementErrorCode, MovementObserverGrant, MovementObserverRole,
    MovementOrderRecordV1, MovementState, route_leg,
};
use canwu_api::{
    CapacityBooking, Handoff, ItineraryRevisionId, ItineraryRevisionReason, KnowledgeHolderRef,
    KnowledgeRecordKind, KnowledgeSchemaId, LegExecutionId, LegExecutionStatus, MovementOrderId,
    MovementSubject, RoutingNodeRef, SimTime, TransportExecution, TransportExecutionId,
    TransportExecutionState, canonical_hash,
};
use serde::{Deserialize, Serialize};

pub const MOVEMENT_REPORT_KNOWLEDGE: &str = "movement_report";
const REPORT_DIGEST_DOMAIN: &str = "canwu.movement.report.v1";

#[must_use]
pub fn movement_report_knowledge_schema_id() -> KnowledgeSchemaId {
    KnowledgeSchemaId::new(
        KnowledgeRecordKind::new(PLUGIN_NAMESPACE, MOVEMENT_REPORT_KNOWLEDGE),
        1,
    )
}

/// Coarse progress of a movement as an observer knows it.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MovementPhaseV1 {
    /// Not travelling: before departure or between legs.
    Waiting,
    InTransit,
    /// The current leg failed and a new itinerary is awaited.
    Interrupted,
    ArrivalPending,
    Settled,
    Failed,
    Cancelled,
}

/// One leg as it stood at the observer's cut.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MovementLegObservationV1 {
    pub leg: LegExecutionId,
    pub revision: ItineraryRevisionId,
    pub leg_index: usize,
    pub from: RoutingNodeRef,
    pub to: RoutingNodeRef,
    pub status: LegExecutionStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub departed_at: Option<SimTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrived_at: Option<SimTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed_at: Option<SimTime>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MovementRevisionObservationV1 {
    pub revision: ItineraryRevisionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predecessor: Option<ItineraryRevisionId>,
    pub reason: ItineraryRevisionReason,
    pub installed_at: SimTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_at: Option<SimTime>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MovementLegFailureV1 {
    pub leg: LegExecutionId,
    pub reason: String,
}

/// Execution detail visible only to the operator and owner.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MovementReportDetailV1 {
    pub state: TransportExecutionState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_revision: Option<ItineraryRevisionId>,
    pub revisions: Vec<MovementRevisionObservationV1>,
    pub failures: Vec<MovementLegFailureV1>,
    pub handoffs: Vec<Handoff>,
    pub bookings: Vec<CapacityBooking>,
}

/// Payload of one `canwu.movement.movement_report` knowledge record.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MovementReportV1 {
    pub execution: TransportExecutionId,
    pub order: MovementOrderId,
    pub holder: KnowledgeHolderRef,
    pub role: MovementObserverRole,
    pub observed_as_of: SimTime,
    pub subjects: Vec<MovementSubject>,
    pub origin: RoutingNodeRef,
    pub destination: RoutingNodeRef,
    pub phase: MovementPhaseV1,
    pub last_known_endpoint: String,
    pub legs: Vec<MovementLegObservationV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<MovementReportDetailV1>,
    /// Digest over every other field.
    pub digest: String,
}

/// The report `holder` may see about `order` at `at`, or `None` when the
/// holder has no grant or its cut precedes the order.
pub fn movement_report(
    state: &MovementState,
    order: MovementOrderId,
    holder: &KnowledgeHolderRef,
    at: SimTime,
) -> Result<Option<MovementReportV1>, MovementError> {
    let Some(record) = state.orders.get(&order) else {
        return Ok(None);
    };
    let Some(grant) = record
        .observers
        .iter()
        .find(|grant| grant.holder == *holder)
    else {
        return Ok(None);
    };
    project(state, record, grant, at)
}

pub(crate) fn project(
    state: &MovementState,
    record: &MovementOrderRecordV1,
    grant: &MovementObserverGrant,
    at: SimTime,
) -> Result<Option<MovementReportV1>, MovementError> {
    let Some(cut) = observer_cut(at, grant) else {
        return Ok(None);
    };
    if cut < record.admitted_at {
        return Ok(None);
    }
    let execution = state
        .executions
        .get(&record.execution)
        .ok_or_else(|| MovementError::state("order names a missing execution"))?;
    let closed = record.closed_at.filter(|closed| *closed <= cut);
    let mut legs = Vec::new();
    let mut visible_revision = None;
    for revision in &execution.revisions {
        if installed_at(record, execution, revision.id)? > cut {
            continue;
        }
        visible_revision = Some(revision.id);
        for leg in execution
            .legs
            .iter()
            .filter(|leg| leg.itinerary_revision == revision.id)
        {
            let planned = route_leg(execution, revision.id, leg.leg_index)?;
            let seen = |time: Option<SimTime>| time.filter(|time| *time <= cut);
            let (departed_at, arrived_at, failed_at) = (
                seen(leg.actual_departure_at),
                seen(leg.actual_arrival_at),
                seen(leg.failed_at),
            );
            let status = if arrived_at.is_some() {
                LegExecutionStatus::Arrived
            } else if failed_at.is_some() {
                LegExecutionStatus::Failed
            } else if departed_at.is_some() {
                LegExecutionStatus::Departed
            } else if closed.is_some() && leg.status == LegExecutionStatus::Cancelled {
                LegExecutionStatus::Cancelled
            } else {
                LegExecutionStatus::Planned
            };
            legs.push(MovementLegObservationV1 {
                leg: leg.id,
                revision: revision.id,
                leg_index: leg.leg_index,
                from: planned.from.clone(),
                to: planned.to.clone(),
                status,
                departed_at,
                arrived_at,
                failed_at,
            });
        }
    }
    let phase = if closed.is_some() {
        match execution.state {
            TransportExecutionState::Settled => MovementPhaseV1::Settled,
            TransportExecutionState::Cancelled => MovementPhaseV1::Cancelled,
            _ => MovementPhaseV1::Failed,
        }
    } else {
        let current = legs
            .iter()
            .filter(|leg| Some(leg.revision) == visible_revision)
            .collect::<Vec<_>>();
        if current
            .iter()
            .any(|leg| leg.status == LegExecutionStatus::Departed)
        {
            MovementPhaseV1::InTransit
        } else if current
            .iter()
            .any(|leg| leg.status == LegExecutionStatus::Failed)
        {
            MovementPhaseV1::Interrupted
        } else if !current.is_empty()
            && current
                .iter()
                .all(|leg| leg.status == LegExecutionStatus::Arrived)
        {
            MovementPhaseV1::ArrivalPending
        } else {
            MovementPhaseV1::Waiting
        }
    };
    let last_known_endpoint = legs
        .iter()
        .filter_map(|leg| leg.arrived_at.map(|time| (time, leg.leg, &leg.to)))
        .max()
        .map_or_else(
            || record.order.origin.as_str().to_owned(),
            |(_, _, to)| to.as_str().to_owned(),
        );
    let detail = match grant.role {
        MovementObserverRole::DelayedRemote => None,
        MovementObserverRole::Operator | MovementObserverRole::Owner => {
            Some(detail(record, execution)?)
        }
    };
    let mut report = MovementReportV1 {
        execution: record.execution,
        order: record.order.id,
        holder: grant.holder.clone(),
        role: grant.role,
        // The latest fact the report reflects, never the publication time,
        // so an unchanged report is not republished and a delayed observer
        // learns nothing from when it was published.
        observed_as_of: event_times(
            state,
            record,
            grant.role != MovementObserverRole::DelayedRemote,
        )?
        .into_iter()
        .filter(|time| *time <= cut)
        .max()
        .unwrap_or(record.admitted_at),
        subjects: record.order.subjects.clone(),
        origin: record.order.origin.clone(),
        destination: record.order.destination.clone(),
        phase,
        last_known_endpoint,
        legs,
        detail,
        digest: String::new(),
    };
    report.digest = canonical_hash(REPORT_DIGEST_DOMAIN, &report)
        .map_err(|error| MovementError::state(error.message))?;
    Ok(Some(report))
}

fn detail(
    record: &MovementOrderRecordV1,
    execution: &TransportExecution,
) -> Result<MovementReportDetailV1, MovementError> {
    let mut revisions = Vec::new();
    for revision in &execution.revisions {
        revisions.push(MovementRevisionObservationV1 {
            revision: revision.id,
            predecessor: revision.predecessor,
            reason: revision.reason.clone(),
            installed_at: installed_at(record, execution, revision.id)?,
            superseded_at: revision.superseded_at,
        });
    }
    Ok(MovementReportDetailV1 {
        state: execution.state,
        active_revision: execution.active_itinerary_revision,
        revisions,
        failures: execution
            .legs
            .iter()
            .filter_map(|leg| {
                leg.failure_reason
                    .as_ref()
                    .map(|reason| MovementLegFailureV1 {
                        leg: leg.id,
                        reason: reason.clone(),
                    })
            })
            .collect(),
        handoffs: execution.handoffs.clone(),
        bookings: execution.bookings.clone(),
    })
}

/// When a revision took effect: order admission for the initial revision and
/// the supersession time of its predecessor for a reroute.
fn installed_at(
    record: &MovementOrderRecordV1,
    execution: &TransportExecution,
    revision: ItineraryRevisionId,
) -> Result<SimTime, MovementError> {
    let item = execution
        .revisions
        .iter()
        .find(|item| item.id == revision)
        .ok_or_else(|| MovementError::state("revision disappeared"))?;
    let Some(predecessor) = item.predecessor else {
        return Ok(record.admitted_at);
    };
    execution
        .revisions
        .iter()
        .find(|item| item.id == predecessor)
        .and_then(|item| item.superseded_at)
        .ok_or_else(|| {
            MovementError::new(
                MovementErrorCode::InvalidState,
                "a rerouted revision's predecessor has no supersession time",
            )
        })
}

pub(crate) fn observer_cut(at: SimTime, grant: &MovementObserverGrant) -> Option<SimTime> {
    at.as_minutes()
        .checked_sub(grant.delay.as_minutes())
        .map(SimTime::from_minutes)
}

/// Every time at which a fact in the reports about `record` changed;
/// handoff and allocation times count only for detailed reports.
pub(crate) fn event_times(
    state: &MovementState,
    record: &MovementOrderRecordV1,
    detailed: bool,
) -> Result<Vec<SimTime>, MovementError> {
    let execution = state
        .executions
        .get(&record.execution)
        .ok_or_else(|| MovementError::state("order names a missing execution"))?;
    let mut times = vec![record.admitted_at];
    times.extend(record.closed_at);
    for revision in &execution.revisions {
        times.push(installed_at(record, execution, revision.id)?);
    }
    for leg in &execution.legs {
        times.extend(leg.actual_departure_at);
        times.extend(leg.actual_arrival_at);
        times.extend(leg.failed_at);
    }
    if detailed {
        times.extend(execution.handoffs.iter().map(|handoff| handoff.at));
        times.extend(
            state
                .bookings
                .values()
                .filter(|binding| binding.execution == record.execution)
                .filter_map(|binding| {
                    binding
                        .allocation
                        .as_ref()
                        .map(|evidence| evidence.allocated_at)
                }),
        );
    }
    times.sort_unstable();
    times.dedup();
    Ok(times)
}
