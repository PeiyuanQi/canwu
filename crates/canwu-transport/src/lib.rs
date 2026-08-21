//! Domain-neutral transport execution records.
//!
//! The crate owns transport semantics, not the simulation scheduler or the
//! information lifecycle. Applications persist these values through their
//! domain-record schemas and drive transitions through canonical ingress.

#![allow(clippy::missing_errors_doc)]

use canwu_core::{DomainRecordVersionRef, EvidenceRef};
use canwu_routing::RoutePlan;
use canwu_time::SimTime;
use serde::{Deserialize, Serialize};

pub const TRANSPORT_SEMANTIC_VERSION: &str = "canwu-transport.v1";

#[must_use]
pub fn delivery_completion_operation_key(
    execution: TransportExecutionId,
    revision: ItineraryRevisionId,
    attempt_version: u64,
) -> String {
    format!(
        "transport/{}/revision/{}/delivery-completion/attempt-version/{}",
        execution.0, revision.0, attempt_version
    )
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct TransportExecutionId(pub u64);

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ItineraryRevisionId(pub u64);

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct LegExecutionId(pub u64);

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct HandoffId(pub u64);

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct CapacityBookingId(pub u64);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportExecutionState {
    Prepared,
    Planning,
    Booking,
    Ready,
    Executing,
    ReplanPending,
    ArrivalPending,
    Settled,
    Failed,
    Cancelled,
}

impl TransportExecutionState {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Settled | Self::Failed | Self::Cancelled)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ItineraryRevisionReason {
    Initial,
    Disaster { explanation: String },
    CapacityUnavailable { explanation: String },
    KnowledgeUpdate { explanation: String },
    Recovery { explanation: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ItineraryRevision {
    pub id: ItineraryRevisionId,
    pub predecessor: Option<ItineraryRevisionId>,
    pub plan: RoutePlan,
    pub planned_at: SimTime,
    pub valid_from: SimTime,
    pub reason: ItineraryRevisionReason,
    pub superseded_at: Option<SimTime>,
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LegExecutionStatus {
    Planned,
    Booked,
    Loaded,
    Departed,
    Arrived,
    Waiting,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LegExecution {
    pub id: LegExecutionId,
    pub itinerary_revision: ItineraryRevisionId,
    pub leg_index: usize,
    pub status: LegExecutionStatus,
    pub actual_departure_at: Option<SimTime>,
    pub actual_arrival_at: Option<SimTime>,
    pub failure_reason: Option<String>,
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Handoff {
    pub id: HandoffId,
    pub from_leg: LegExecutionId,
    pub to_leg: LegExecutionId,
    pub from_custodian: String,
    pub to_custodian: String,
    pub at: SimTime,
    pub location: String,
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DeliveryCompletionRequest {
    pub operation_key: String,
    pub execution: TransportExecutionId,
    pub itinerary_revision: ItineraryRevisionId,
    pub delivery_attempt: DomainRecordVersionRef,
    pub completed_at: SimTime,
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapacityBookingStatus {
    Requested,
    Confirmed,
    Consumed,
    Released,
    Expired,
    Cancelled,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CapacityBooking {
    pub id: CapacityBookingId,
    pub execution: TransportExecutionId,
    pub resource: String,
    pub valid_from: SimTime,
    pub valid_until: SimTime,
    pub quantity: u64,
    pub priority: i32,
    pub status: CapacityBookingStatus,
    pub allocation_evidence: Vec<EvidenceRef>,
}

impl CapacityBooking {
    pub fn new(
        id: CapacityBookingId,
        execution: TransportExecutionId,
        resource: String,
        valid_from: SimTime,
        valid_until: SimTime,
        quantity: u64,
        priority: i32,
    ) -> Result<Self, TransportError> {
        if valid_until < valid_from || quantity == 0 {
            return Err(TransportError::InvalidBooking(
                "capacity booking requires a positive quantity and non-inverted window".to_owned(),
            ));
        }
        Ok(Self {
            id,
            execution,
            resource,
            valid_from,
            valid_until,
            quantity,
            priority,
            status: CapacityBookingStatus::Requested,
            allocation_evidence: Vec::new(),
        })
    }

    pub fn transition(
        &mut self,
        status: CapacityBookingStatus,
        at: SimTime,
    ) -> Result<(), TransportError> {
        if at < self.valid_from {
            return Err(TransportError::InvalidBooking(
                "booking cannot transition before its validity window".to_owned(),
            ));
        }
        let allowed = matches!(
            (self.status, status),
            (
                CapacityBookingStatus::Requested,
                CapacityBookingStatus::Confirmed
                    | CapacityBookingStatus::Failed
                    | CapacityBookingStatus::Cancelled
            ) | (
                CapacityBookingStatus::Confirmed,
                CapacityBookingStatus::Consumed
                    | CapacityBookingStatus::Released
                    | CapacityBookingStatus::Cancelled
                    | CapacityBookingStatus::Expired
            ) | (
                CapacityBookingStatus::Consumed,
                CapacityBookingStatus::Released
            )
        );
        if !allowed {
            return Err(TransportError::InvalidBooking(
                "capacity booking transition is not allowed".to_owned(),
            ));
        }
        if at > self.valid_until && status == CapacityBookingStatus::Confirmed {
            return Err(TransportError::InvalidBooking(
                "capacity booking cannot be confirmed after its validity window".to_owned(),
            ));
        }
        if status == CapacityBookingStatus::Expired && at <= self.valid_until {
            return Err(TransportError::InvalidBooking(
                "capacity booking cannot expire before its validity window ends".to_owned(),
            ));
        }
        self.status = status;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SagaState {
    TransportIntent,
    WaitingForInformation,
    Executing,
    ArrivalPending,
    Settled,
    CompensationPending,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DeliverySaga {
    pub operation_key: String,
    pub delivery_attempt: DomainRecordVersionRef,
    pub state: SagaState,
    pub step: u32,
    pub expected_attempt_version: u64,
    pub last_error: Option<String>,
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TransportExecution {
    pub id: TransportExecutionId,
    pub delivery_attempt: Option<DomainRecordVersionRef>,
    pub state: TransportExecutionState,
    pub active_itinerary_revision: Option<ItineraryRevisionId>,
    pub current_leg_index: usize,
    pub estimated_arrival_at: Option<SimTime>,
    pub current_endpoint: Option<String>,
    pub revisions: Vec<ItineraryRevision>,
    pub legs: Vec<LegExecution>,
    pub handoffs: Vec<Handoff>,
    pub bookings: Vec<CapacityBooking>,
    pub saga: Option<DeliverySaga>,
}

impl TransportExecution {
    #[must_use]
    pub fn new(id: TransportExecutionId, delivery_attempt: Option<DomainRecordVersionRef>) -> Self {
        Self {
            id,
            delivery_attempt,
            state: TransportExecutionState::Prepared,
            active_itinerary_revision: None,
            current_leg_index: 0,
            estimated_arrival_at: None,
            current_endpoint: None,
            revisions: Vec::new(),
            legs: Vec::new(),
            handoffs: Vec::new(),
            bookings: Vec::new(),
            saga: None,
        }
    }

    pub fn install_initial_itinerary(
        &mut self,
        revision: ItineraryRevision,
    ) -> Result<(), TransportError> {
        if !self.revisions.is_empty() || revision.predecessor.is_some() {
            return Err(TransportError::InvalidRevision(
                "initial itinerary must be the first revision".to_owned(),
            ));
        }
        self.estimated_arrival_at = Some(revision.plan.estimated_arrival_at);
        self.active_itinerary_revision = Some(revision.id);
        self.current_endpoint = Some(revision.plan.origin.as_str().to_owned());
        self.legs = revision
            .plan
            .legs
            .iter()
            .enumerate()
            .map(|(index, _)| LegExecution {
                id: LegExecutionId(index as u64 + 1),
                itinerary_revision: revision.id,
                leg_index: index,
                status: LegExecutionStatus::Planned,
                actual_departure_at: None,
                actual_arrival_at: None,
                failure_reason: None,
                evidence: Vec::new(),
            })
            .collect();
        self.revisions.push(revision);
        self.state = TransportExecutionState::Planning;
        Ok(())
    }

    pub fn reroute(
        &mut self,
        revision: ItineraryRevision,
        at: SimTime,
    ) -> Result<(), TransportError> {
        let active = self
            .active_itinerary_revision
            .ok_or(TransportError::MissingItinerary)?;
        if revision.predecessor != Some(active) || revision.valid_from < at {
            return Err(TransportError::InvalidRevision("reroute must reference the active revision and start no earlier than the reroute time".to_owned()));
        }
        if let Some(previous) = self.revisions.iter_mut().find(|item| item.id == active) {
            previous.superseded_at = Some(at);
        }
        self.estimated_arrival_at = Some(revision.plan.estimated_arrival_at);
        self.active_itinerary_revision = Some(revision.id);
        self.current_leg_index = 0;
        let next_leg_id = self
            .legs
            .iter()
            .map(|leg| leg.id.0)
            .max()
            .unwrap_or_default()
            .checked_add(1)
            .ok_or(TransportError::Overflow)?;
        let mut legs = Vec::with_capacity(revision.plan.legs.len());
        for (index, _) in revision.plan.legs.iter().enumerate() {
            legs.push(LegExecution {
                id: LegExecutionId(
                    next_leg_id
                        .checked_add(index as u64)
                        .ok_or(TransportError::Overflow)?,
                ),
                itinerary_revision: revision.id,
                leg_index: index,
                status: LegExecutionStatus::Planned,
                actual_departure_at: None,
                actual_arrival_at: None,
                failure_reason: None,
                evidence: Vec::new(),
            });
        }
        self.legs = legs;
        self.revisions.push(revision);
        self.state = TransportExecutionState::Planning;
        Ok(())
    }

    pub fn begin_saga(
        &mut self,
        delivery_attempt: DomainRecordVersionRef,
        operation_key: String,
    ) -> Result<(), TransportError> {
        if self.saga.is_some() {
            return Err(TransportError::SagaAlreadyExists);
        }
        self.saga = Some(DeliverySaga {
            expected_attempt_version: delivery_attempt.version,
            operation_key,
            delivery_attempt,
            state: SagaState::TransportIntent,
            step: 0,
            last_error: None,
            evidence: Vec::new(),
        });
        self.state = TransportExecutionState::Executing;
        Ok(())
    }

    pub fn start_current_leg(&mut self, at: SimTime) -> Result<(), TransportError> {
        if self.state != TransportExecutionState::Ready
            && self.state != TransportExecutionState::Executing
            && self.state != TransportExecutionState::Planning
        {
            return Err(TransportError::InvalidState(
                "transport execution cannot start a leg in its current state".to_owned(),
            ));
        }
        let leg = self
            .legs
            .iter_mut()
            .find(|leg| leg.leg_index == self.current_leg_index)
            .ok_or(TransportError::MissingLeg)?;
        if !matches!(
            leg.status,
            LegExecutionStatus::Planned | LegExecutionStatus::Booked | LegExecutionStatus::Waiting
        ) {
            return Err(TransportError::InvalidState(
                "current leg is not startable".to_owned(),
            ));
        }
        leg.status = LegExecutionStatus::Departed;
        leg.actual_departure_at = Some(at);
        self.state = TransportExecutionState::Executing;
        Ok(())
    }

    pub fn complete_current_leg(
        &mut self,
        at: SimTime,
        endpoint: String,
    ) -> Result<bool, TransportError> {
        let final_leg = self.current_leg_index.saturating_add(1) >= self.legs.len();
        if final_leg && self.saga.is_none() {
            return Err(TransportError::MissingSaga);
        }
        let leg = self
            .legs
            .iter_mut()
            .find(|leg| leg.leg_index == self.current_leg_index)
            .ok_or(TransportError::MissingLeg)?;
        if leg.status != LegExecutionStatus::Departed {
            return Err(TransportError::InvalidState(
                "current leg must be departed before arrival".to_owned(),
            ));
        }
        if leg
            .actual_departure_at
            .is_some_and(|departure| at < departure)
        {
            return Err(TransportError::InvalidState(
                "arrival precedes departure".to_owned(),
            ));
        }
        leg.status = LegExecutionStatus::Arrived;
        leg.actual_arrival_at = Some(at);
        self.current_endpoint = Some(endpoint);
        self.current_leg_index = self.current_leg_index.saturating_add(1);
        if self.current_leg_index >= self.legs.len() {
            self.state = TransportExecutionState::ArrivalPending;
            self.mark_arrival_pending()?;
            Ok(true)
        } else {
            self.state = TransportExecutionState::Ready;
            Ok(false)
        }
    }

    pub fn fail_current_leg(&mut self, reason: String) -> Result<(), TransportError> {
        let leg = self
            .legs
            .iter_mut()
            .find(|leg| leg.leg_index == self.current_leg_index)
            .ok_or(TransportError::MissingLeg)?;
        leg.status = LegExecutionStatus::Failed;
        leg.failure_reason = Some(reason);
        self.state = TransportExecutionState::ReplanPending;
        Ok(())
    }

    pub fn mark_arrival_pending(&mut self) -> Result<(), TransportError> {
        let saga = self.saga.as_mut().ok_or(TransportError::MissingSaga)?;
        saga.step = saga.step.checked_add(1).ok_or(TransportError::Overflow)?;
        saga.state = SagaState::ArrivalPending;
        self.state = TransportExecutionState::ArrivalPending;
        Ok(())
    }

    pub fn record_handoff(&mut self, handoff: Handoff) -> Result<(), TransportError> {
        if handoff.from_leg == handoff.to_leg
            || handoff.from_custodian.trim().is_empty()
            || handoff.to_custodian.trim().is_empty()
            || handoff.location.trim().is_empty()
        {
            return Err(TransportError::InvalidHandoff(
                "handoff requires distinct legs, custodians, and location".to_owned(),
            ));
        }
        if self
            .handoffs
            .iter()
            .any(|existing| existing.id == handoff.id)
        {
            return Err(TransportError::InvalidHandoff(
                "handoff identity is already recorded".to_owned(),
            ));
        }
        let from_leg = self
            .legs
            .iter()
            .find(|leg| leg.id == handoff.from_leg)
            .ok_or(TransportError::MissingLeg)?;
        let to_leg = self
            .legs
            .iter()
            .find(|leg| leg.id == handoff.to_leg)
            .ok_or(TransportError::MissingLeg)?;
        if from_leg.status != LegExecutionStatus::Arrived
            || !matches!(
                to_leg.status,
                LegExecutionStatus::Planned
                    | LegExecutionStatus::Booked
                    | LegExecutionStatus::Waiting
            )
            || from_leg
                .actual_arrival_at
                .is_some_and(|arrived_at| handoff.at < arrived_at)
        {
            return Err(TransportError::InvalidHandoff(
                "handoff must follow an arrived leg and precede the next leg".to_owned(),
            ));
        }
        self.handoffs.push(handoff);
        Ok(())
    }

    pub fn completion_request(&self) -> Result<DeliveryCompletionRequest, TransportError> {
        let saga = self.saga.as_ref().ok_or(TransportError::MissingSaga)?;
        if self.state != TransportExecutionState::ArrivalPending
            || saga.state != SagaState::ArrivalPending
        {
            return Err(TransportError::InvalidState(
                "delivery completion requires an arrival-pending execution".to_owned(),
            ));
        }
        let revision = self
            .active_itinerary_revision
            .ok_or(TransportError::MissingItinerary)?;
        let completed_at = self
            .legs
            .last()
            .and_then(|leg| leg.actual_arrival_at)
            .ok_or(TransportError::MissingLeg)?;
        let attempt = self
            .delivery_attempt
            .clone()
            .ok_or(TransportError::MissingDeliveryAttempt)?;
        if attempt != saga.delivery_attempt {
            return Err(TransportError::InvalidState(
                "saga delivery attempt does not match execution".to_owned(),
            ));
        }
        Ok(DeliveryCompletionRequest {
            operation_key: saga.operation_key.clone(),
            execution: self.id,
            itinerary_revision: revision,
            delivery_attempt: attempt,
            completed_at,
            evidence: saga.evidence.clone(),
        })
    }

    pub fn reconcile_information(
        &mut self,
        success: bool,
        error: Option<String>,
    ) -> Result<(), TransportError> {
        let saga = self.saga.as_mut().ok_or(TransportError::MissingSaga)?;
        saga.step = saga.step.checked_add(1).ok_or(TransportError::Overflow)?;
        if success {
            saga.state = SagaState::Settled;
            saga.last_error = None;
            self.state = TransportExecutionState::Settled;
        } else {
            saga.state = SagaState::CompensationPending;
            saga.last_error = error;
            self.state = TransportExecutionState::Failed;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum TransportError {
    InvalidRevision(String),
    MissingItinerary,
    MissingSaga,
    SagaAlreadyExists,
    Overflow,
    MissingLeg,
    InvalidState(String),
    InvalidBooking(String),
    InvalidHandoff(String),
    MissingDeliveryAttempt,
}

#[cfg(test)]
mod tests {
    use super::*;
    use canwu_routing::{
        ROUTING_ALGORITHM_VERSION, RouteCost, RouteLeg, RoutingConnectionRef, RoutingNodeRef,
        TransferMode,
    };

    fn plan() -> RoutePlan {
        RoutePlan {
            algorithm_version: ROUTING_ALGORITHM_VERSION.to_owned(),
            policy_version: "policy.v1".to_owned(),
            planning_snapshot_digest: "snapshot".to_owned(),
            origin: RoutingNodeRef::new("a"),
            destination: RoutingNodeRef::new("b"),
            departure_at: canwu_time::SimTime::EPOCH,
            estimated_arrival_at: canwu_time::SimTime::from_minutes(10),
            cost: RouteCost {
                estimated_arrival_at: canwu_time::SimTime::from_minutes(10),
                risk_per_mille: 0,
                resource_cost: 0,
                transfers: 1,
            },
            legs: vec![RouteLeg {
                connection: RoutingConnectionRef::new("ab"),
                from: RoutingNodeRef::new("a"),
                to: RoutingNodeRef::new("b"),
                mode: TransferMode::Horse,
                planned_departure_at: canwu_time::SimTime::EPOCH,
                planned_arrival_at: canwu_time::SimTime::from_minutes(10),
            }],
            digest: "route".to_owned(),
        }
    }

    #[test]
    fn reroute_supersedes_without_creating_a_new_delivery_attempt() {
        let mut execution = TransportExecution::new(TransportExecutionId(1), None);
        execution
            .install_initial_itinerary(ItineraryRevision {
                id: ItineraryRevisionId(1),
                predecessor: None,
                plan: plan(),
                planned_at: canwu_time::SimTime::EPOCH,
                valid_from: canwu_time::SimTime::EPOCH,
                reason: ItineraryRevisionReason::Initial,
                superseded_at: None,
                evidence: Vec::new(),
            })
            .unwrap();
        let mut replacement = plan();
        replacement.destination = RoutingNodeRef::new("b");
        execution
            .reroute(
                ItineraryRevision {
                    id: ItineraryRevisionId(2),
                    predecessor: Some(ItineraryRevisionId(1)),
                    plan: replacement,
                    planned_at: canwu_time::SimTime::from_minutes(1),
                    valid_from: canwu_time::SimTime::from_minutes(1),
                    reason: ItineraryRevisionReason::Disaster {
                        explanation: "bridge closed".to_owned(),
                    },
                    superseded_at: None,
                    evidence: Vec::new(),
                },
                canwu_time::SimTime::from_minutes(1),
            )
            .unwrap();
        assert_eq!(execution.revisions.len(), 2);
        assert_eq!(
            execution.active_itinerary_revision,
            Some(ItineraryRevisionId(2))
        );
        assert_eq!(execution.state, TransportExecutionState::Planning);
    }

    #[test]
    fn capacity_booking_is_a_persisted_windowed_state_machine() {
        let mut booking = CapacityBooking::new(
            CapacityBookingId(1),
            TransportExecutionId(1),
            "relay-horse:wu-xi:01".to_owned(),
            canwu_time::SimTime::EPOCH,
            canwu_time::SimTime::from_minutes(60),
            1,
            10,
        )
        .unwrap();
        booking
            .transition(CapacityBookingStatus::Confirmed, canwu_time::SimTime::EPOCH)
            .unwrap();
        booking
            .transition(
                CapacityBookingStatus::Consumed,
                canwu_time::SimTime::from_minutes(10),
            )
            .unwrap();
        assert_eq!(booking.status, CapacityBookingStatus::Consumed);
    }

    #[test]
    fn completion_operation_key_is_stable_and_revision_scoped() {
        let first =
            delivery_completion_operation_key(TransportExecutionId(4), ItineraryRevisionId(2), 7);
        let second =
            delivery_completion_operation_key(TransportExecutionId(4), ItineraryRevisionId(3), 7);
        assert_ne!(first, second);
        assert_eq!(
            first,
            delivery_completion_operation_key(TransportExecutionId(4), ItineraryRevisionId(2), 7)
        );
    }

    #[test]
    fn completion_requires_saga_and_exposes_stable_bridge_request() {
        let attempt = DomainRecordVersionRef {
            record: canwu_core::DomainRecordRef::new(
                "fixture.information",
                "delivery_attempt",
                "delivery",
            ),
            version: 3,
            established_by: canwu_core::DomainRecordVersionSource::InitialScenario,
        };
        let mut execution = TransportExecution::new(TransportExecutionId(9), Some(attempt.clone()));
        let revision = ItineraryRevision {
            id: ItineraryRevisionId(1),
            predecessor: None,
            plan: plan(),
            planned_at: SimTime::EPOCH,
            valid_from: SimTime::EPOCH,
            reason: ItineraryRevisionReason::Initial,
            superseded_at: None,
            evidence: Vec::new(),
        };
        execution.install_initial_itinerary(revision).unwrap();
        assert_eq!(
            execution.complete_current_leg(SimTime::from_minutes(10), "b".to_owned()),
            Err(TransportError::MissingSaga)
        );
        execution
            .begin_saga(
                attempt,
                delivery_completion_operation_key(
                    TransportExecutionId(9),
                    ItineraryRevisionId(1),
                    3,
                ),
            )
            .unwrap();
        execution.start_current_leg(SimTime::EPOCH).unwrap();
        assert!(
            execution
                .complete_current_leg(SimTime::from_minutes(10), "b".to_owned())
                .unwrap()
        );
        let request = execution.completion_request().unwrap();
        assert_eq!(request.execution, TransportExecutionId(9));
        assert_eq!(request.itinerary_revision, ItineraryRevisionId(1));
        assert_eq!(request.delivery_attempt.version, 3);
    }

    #[test]
    fn handoff_requires_arrival_and_is_idempotency_safe() {
        let attempt = DomainRecordVersionRef {
            record: canwu_core::DomainRecordRef::new(
                "fixture.information",
                "delivery_attempt",
                "delivery",
            ),
            version: 1,
            established_by: canwu_core::DomainRecordVersionSource::InitialScenario,
        };
        let mut execution =
            TransportExecution::new(TransportExecutionId(10), Some(attempt.clone()));
        let mut route = plan();
        route.legs.push(RouteLeg {
            connection: RoutingConnectionRef::new("bc"),
            from: RoutingNodeRef::new("b"),
            to: RoutingNodeRef::new("c"),
            mode: TransferMode::Rail,
            planned_departure_at: SimTime::from_minutes(10),
            planned_arrival_at: SimTime::from_minutes(20),
        });
        route.destination = RoutingNodeRef::new("c");
        route.estimated_arrival_at = SimTime::from_minutes(20);
        let revision = ItineraryRevision {
            id: ItineraryRevisionId(1),
            predecessor: None,
            plan: route,
            planned_at: SimTime::EPOCH,
            valid_from: SimTime::EPOCH,
            reason: ItineraryRevisionReason::Initial,
            superseded_at: None,
            evidence: Vec::new(),
        };
        execution.install_initial_itinerary(revision).unwrap();
        execution.start_current_leg(SimTime::EPOCH).unwrap();
        execution
            .complete_current_leg(SimTime::from_minutes(10), "b".to_owned())
            .unwrap();
        let handoff = Handoff {
            id: HandoffId(1),
            from_leg: LegExecutionId(1),
            to_leg: LegExecutionId(2),
            from_custodian: "courier/wuxi".to_owned(),
            to_custodian: "rail/beijing".to_owned(),
            at: SimTime::from_minutes(10),
            location: "b".to_owned(),
            evidence: Vec::new(),
        };
        execution.record_handoff(handoff.clone()).unwrap();
        assert_eq!(
            execution.record_handoff(handoff),
            Err(TransportError::InvalidHandoff(
                "handoff identity is already recorded".to_owned()
            ))
        );
    }
}
