//! Deterministic movement transitions. Every change to a transport execution
//! goes through a `canwu-transport` transition method; this module adds only
//! authority, bookkeeping, capacity-pool accounting, and due scheduling.

use crate::model::{
    MovementBookingBindingV1, MovementBookingRequestV1, MovementError, MovementErrorCode,
    MovementLegDueKind, MovementLegDueV1, MovementLimitsV1, MovementObserverGrant,
    MovementObserverRole, MovementOperation, MovementOrderRecordV1, MovementOrderRequestV1,
    MovementPoolOfferV1, MovementState, is_label, route_leg, validate_plan, within_time_bound,
};
use canwu_api::{
    BoundaryId, CanwuError, CapacityBookingRequestV1, CapacityBookingStatus, DomainRecord,
    DomainRecordRef, DomainRecordVersionRef, DomainReferenceTarget, EntityRef, EvidenceRef,
    Handoff, ItineraryRevision, ItineraryRevisionId, ItineraryRevisionReason, KnowledgeHolderRef,
    LegExecutionId, LegExecutionStatus, MovementInitiative, MovementSubjectRole, PersonId,
    ReconciliationOutcome, SagaState, SimDuration, SimTime, TransportCapacityPoolV1,
    TransportExecution, TransportExecutionId, TransportExecutionState, allocate_capacity_bookings,
    delivery_completion_operation_key,
};

/// Read access the lifecycle needs from the settlement view.
pub(crate) trait EvidenceReader {
    fn version_exists(&self, version: &DomainRecordVersionRef) -> Result<bool, CanwuError>;
    /// The record at `version` when that version is current.
    fn current_record(
        &self,
        version: &DomainRecordVersionRef,
    ) -> Result<Option<DomainRecord>, CanwuError>;
    fn condition_exists(&self, record: &DomainRecordRef, version: u64) -> Result<bool, CanwuError>;
}

/// Reference role naming the holder an authority basis record authorizes.
pub const MOVEMENT_GRANTEE_ROLE: &str = "movement_grantee";
/// Reference role naming one subject an authority basis record covers.
pub const MOVEMENT_SUBJECT_ROLE: &str = "movement_subject";

/// Who submitted an operation.
pub(crate) enum OperationActor<'a> {
    /// A tracked command by `holder`.
    Holder(&'a KnowledgeHolderRef),
    /// An application-reported incident citing `evidence`.
    Incident(&'a DomainRecordVersionRef),
}

fn evidence_error(error: &CanwuError) -> MovementError {
    MovementError::new(
        MovementErrorCode::Evidence,
        format!(
            "cited evidence could not be read; its record kind must be declared to the movement plugin ({})",
            error.message
        ),
    )
}

fn unauthorized(message: &str) -> MovementError {
    MovementError::new(MovementErrorCode::Unauthorized, message)
}

fn later(left: SimTime, right: SimTime) -> SimTime {
    left.max(right)
}

pub(crate) fn holder_entity(holder: &KnowledgeHolderRef) -> EntityRef {
    match holder {
        KnowledgeHolderRef::Person(person) => EntityRef::Person(*person),
        KnowledgeHolderRef::Entity(entity) => entity.clone(),
    }
}

impl MovementState {
    /// Applies one admitted operation. The caller applies it to a candidate
    /// copy and discards the copy when it returns an error.
    pub(crate) fn apply_operation(
        &mut self,
        operation: &MovementOperation,
        operation_key: &str,
        actor: &OperationActor<'_>,
        at: SimTime,
        reader: &dyn EvidenceReader,
        scheduled: &mut Vec<MovementLegDueV1>,
    ) -> Result<(), MovementError> {
        if let OperationActor::Incident(evidence) = actor {
            if !matches!(
                operation,
                MovementOperation::FailLeg { .. }
                    | MovementOperation::Reroute { .. }
                    | MovementOperation::RecordHandoff { .. }
                    | MovementOperation::Reconcile { .. }
            ) {
                return Err(unauthorized(
                    "an incident may only fail a leg, reroute, record a handoff, or reconcile a delivery",
                ));
            }
            if !reader
                .version_exists(evidence)
                .map_err(|error| evidence_error(&error))?
            {
                return Err(MovementError::new(
                    MovementErrorCode::Evidence,
                    "incident evidence version does not exist",
                ));
            }
        }
        match operation {
            MovementOperation::Order(request) => {
                let OperationActor::Holder(holder) = actor else {
                    return Err(unauthorized("orders require a command holder"));
                };
                self.order(request, operation_key, holder, at, reader, scheduled)
            }
            MovementOperation::OfferPool(offer) => {
                let OperationActor::Holder(holder) = actor else {
                    return Err(unauthorized("pools require a command holder"));
                };
                self.offer_pool(offer, holder)
            }
            MovementOperation::StartLeg { execution, leg } => {
                self.authorize(*execution, actor)?;
                self.depart(*execution, Some(*leg), true, at, scheduled)
            }
            MovementOperation::CompleteLeg { execution, leg } => {
                self.authorize(*execution, actor)?;
                self.arrive(*execution, Some(*leg), at, scheduled)
            }
            MovementOperation::FailLeg {
                execution,
                leg,
                reason,
            } => {
                self.authorize(*execution, actor)?;
                self.fail_leg(*execution, *leg, reason, at)
            }
            MovementOperation::Reroute {
                execution,
                revision,
            } => {
                self.authorize(*execution, actor)?;
                self.reroute(*execution, revision, at, reader, scheduled)
            }
            MovementOperation::RecordHandoff { execution, handoff } => {
                self.authorize(*execution, actor)?;
                // A seizure asserts an act by someone outside the itinerary,
                // so only the application's incident path may record one.
                if matches!(actor, OperationActor::Holder(_)) && !handoff.kind.is_planned() {
                    return Err(unauthorized(
                        "a seizure handoff is reported by an application incident",
                    ));
                }
                self.record_handoff(*execution, handoff, at)
            }
            MovementOperation::RequestBooking(request) => {
                self.authorize(request.booking.execution, actor)?;
                self.request_booking(request)
            }
            MovementOperation::Cancel { execution, reason } => {
                self.authorize(*execution, actor)?;
                if !is_label(reason, MovementLimitsV1::CURRENT.text_bytes) {
                    return Err(MovementError::invalid(
                        "cancellation needs a bounded reason",
                    ));
                }
                self.execution_mut(*execution)?.cancel()?;
                self.close(*execution, at, CapacityBookingStatus::Cancelled)
            }
            MovementOperation::Reconcile { execution, outcome } => {
                // The delivery's own lifecycle attests the outcome: the
                // incident must cite the delivery attempt record.
                let OperationActor::Incident(evidence) = actor else {
                    return Err(unauthorized(
                        "a delivery is reconciled by an incident citing its attempt record",
                    ));
                };
                let attempt = self
                    .executions
                    .get(execution)
                    .and_then(|execution| execution.delivery_attempt.as_ref())
                    .ok_or_else(|| MovementError::state("the execution carries no delivery"))?;
                if evidence.record != attempt.record || evidence.version < attempt.version {
                    return Err(unauthorized(
                        "a reconciliation must cite the delivery attempt record",
                    ));
                }
                self.reconcile(*execution, outcome, at)
            }
        }
    }

    /// Owners and operators act on an execution; incidents carry application
    /// authority.
    fn authorize(
        &self,
        execution: TransportExecutionId,
        actor: &OperationActor<'_>,
    ) -> Result<(), MovementError> {
        let order = self.order_for_execution(execution).ok_or_else(|| {
            MovementError::new(
                MovementErrorCode::NotFound,
                "movement execution is not known",
            )
        })?;
        match actor {
            OperationActor::Incident(_) => Ok(()),
            OperationActor::Holder(holder) => {
                let operator = order.observers.iter().any(|grant| {
                    grant.role == MovementObserverRole::Operator && grant.holder == **holder
                });
                if order.owner == **holder || operator {
                    Ok(())
                } else {
                    Err(unauthorized(
                        "only the movement owner or operator may act on its execution",
                    ))
                }
            }
        }
    }

    fn execution_mut(
        &mut self,
        execution: TransportExecutionId,
    ) -> Result<&mut TransportExecution, MovementError> {
        self.executions.get_mut(&execution).ok_or_else(|| {
            MovementError::new(
                MovementErrorCode::NotFound,
                "movement execution is not known",
            )
        })
    }

    fn order_mut(
        &mut self,
        execution: TransportExecutionId,
    ) -> Result<&mut MovementOrderRecordV1, MovementError> {
        let id = self.order_id_for_execution(execution)?;
        self.orders
            .get_mut(&id)
            .ok_or_else(|| MovementError::state("order record disappeared"))
    }

    fn schedule(
        &mut self,
        due: MovementLegDueV1,
        scheduled: &mut Vec<MovementLegDueV1>,
    ) -> Result<(), MovementError> {
        if !within_time_bound(due.due_at) {
            return Err(MovementError::new(
                MovementErrorCode::LimitExceeded,
                "movement due time exceeds the supported time bound",
            ));
        }
        self.order_mut(due.execution)?.pending_due = Some(due.clone());
        scheduled.push(due);
        Ok(())
    }

    fn order(
        &mut self,
        request: &MovementOrderRequestV1,
        operation_key: &str,
        holder: &KnowledgeHolderRef,
        at: SimTime,
        reader: &dyn EvidenceReader,
        scheduled: &mut Vec<MovementLegDueV1>,
    ) -> Result<(), MovementError> {
        let limits = MovementLimitsV1::CURRENT;
        let order = &request.order;
        order
            .validate()
            .map_err(|error| MovementError::invalid(format!("{error:?}")))?;
        if order.subjects.len() > limits.subjects_per_order
            || order.plan.legs.len() > limits.legs_per_execution
            || request.remote_observers.len() + 2 > limits.observers_per_order
        {
            return Err(MovementError::new(
                MovementErrorCode::LimitExceeded,
                "order exceeds movement limits",
            ));
        }
        validate_plan(&order.plan, Some(&order.origin), &order.destination)?;
        if order.ordered_at > at || request.execution.0 == 0 {
            return Err(MovementError::invalid(
                "an order needs a nonzero execution id and cannot be ordered in the future",
            ));
        }
        if self.orders.len() >= limits.orders_per_runtime
            || self
                .orders
                .values()
                .filter(|other| other.owner == *holder)
                .count()
                >= limits.orders_per_owner
        {
            return Err(MovementError::new(
                MovementErrorCode::LimitExceeded,
                "movement runtime has no room for another order from this owner",
            ));
        }
        if self.orders.contains_key(&order.id)
            || self.executions.contains_key(&request.execution)
            || self.retired_orders.contains(&order.id)
            || self.retired_executions.contains(&request.execution)
        {
            return Err(MovementError::new(
                MovementErrorCode::Conflict,
                "movement order or execution identity is already used",
            ));
        }
        Self::authorize_subjects(request, holder, reader)?;
        for other in self.orders.values() {
            let active = self
                .executions
                .get(&other.execution)
                .is_some_and(|execution| !execution.state.is_terminal());
            if active
                && other.order.subjects.iter().any(|subject| {
                    order
                        .subjects
                        .iter()
                        .any(|candidate| candidate.entity == subject.entity)
                })
            {
                return Err(MovementError::new(
                    MovementErrorCode::Conflict,
                    "a subject already has an active movement",
                ));
            }
        }
        if let Some(attempt) = &request.delivery_attempt
            && !reader
                .version_exists(attempt)
                .map_err(|error| evidence_error(&error))?
        {
            return Err(MovementError::new(
                MovementErrorCode::Evidence,
                "delivery attempt version does not exist",
            ));
        }
        let observers = observer_grants(holder, request)?;
        let mut execution =
            TransportExecution::new(request.execution, request.delivery_attempt.clone());
        let revision = ItineraryRevisionId(1);
        execution.install_initial_itinerary(ItineraryRevision {
            id: revision,
            predecessor: None,
            plan: order.plan.clone(),
            planned_at: at,
            valid_from: at,
            reason: ItineraryRevisionReason::Initial,
            superseded_at: None,
            evidence: Vec::new(),
        })?;
        if let Some(attempt) = &request.delivery_attempt {
            let key =
                delivery_completion_operation_key(request.execution, revision, attempt.version);
            execution.begin_saga(attempt.clone(), key)?;
        }
        let first = execution
            .legs
            .first()
            .map(|leg| leg.id)
            .ok_or_else(|| MovementError::invalid("a movement plan needs at least one leg"))?;
        let departure = later(at, order.plan.legs[0].planned_departure_at);
        self.executions.insert(request.execution, execution);
        self.orders.insert(
            order.id,
            MovementOrderRecordV1 {
                order: order.clone(),
                execution: request.execution,
                owner: holder.clone(),
                observers,
                operation_key: operation_key.to_owned(),
                admitted_at: at,
                pending_due: None,
                closed_at: None,
            },
        );
        self.schedule(
            MovementLegDueV1 {
                execution: request.execution,
                revision,
                leg: first,
                kind: MovementLegDueKind::Departure,
                due_at: departure,
            },
            scheduled,
        )
    }

    /// The owner may move itself; any other subject needs an authority basis:
    /// the current version of a declared application record whose references
    /// name the owner under [`MOVEMENT_GRANTEE_ROLE`] and every other subject
    /// under [`MOVEMENT_SUBJECT_ROLE`]. The initiative must agree: a
    /// self-directed order moves its ordering person, and an order that moves
    /// only that person is self-directed.
    fn authorize_subjects(
        request: &MovementOrderRequestV1,
        holder: &KnowledgeHolderRef,
        reader: &dyn EvidenceReader,
    ) -> Result<(), MovementError> {
        let order = &request.order;
        let own = holder_entity(holder);
        let moves_self_as_principal = matches!(holder, KnowledgeHolderRef::Person(_))
            && order.subjects.iter().any(|subject| {
                subject.entity == own && subject.role == MovementSubjectRole::MovablePrincipal
            });
        if order.initiative == MovementInitiative::SelfDirected && !moves_self_as_principal {
            return Err(unauthorized(
                "a self-directed movement must move its ordering person",
            ));
        }
        if moves_self_as_principal
            && order.subjects.len() == 1
            && order.initiative != MovementInitiative::SelfDirected
        {
            return Err(unauthorized(
                "a movement of only its ordering person is self-directed",
            ));
        }
        let others = order
            .subjects
            .iter()
            .filter(|subject| subject.entity != own)
            .collect::<Vec<_>>();
        if others.is_empty() {
            return Ok(());
        }
        let Some(basis) = &request.authority_basis else {
            return Err(unauthorized(
                "moving another holder's subjects requires an authority basis",
            ));
        };
        let record = reader
            .current_record(basis)
            .map_err(|error| evidence_error(&error))?
            .ok_or_else(|| {
                unauthorized("the order's authority basis is not a current record version")
            })?;
        let names = |role: &str, entity: &EntityRef| {
            record.references.iter().any(|reference| {
                reference.role == role
                    && match &reference.target {
                        DomainReferenceTarget::Core(target) => target == entity,
                        DomainReferenceTarget::Domain(target) => {
                            *entity == EntityRef::Domain(target.clone())
                        }
                    }
            })
        };
        if !names(MOVEMENT_GRANTEE_ROLE, &own)
            || others
                .iter()
                .any(|subject| !names(MOVEMENT_SUBJECT_ROLE, &subject.entity))
        {
            return Err(unauthorized(
                "the authority basis does not grant this owner every moved subject",
            ));
        }
        Ok(())
    }

    /// Retires every closed execution whose latest observer delay has passed
    /// and whose observers have all received their final report (or can no
    /// longer receive one), and every closed execution unconditionally once
    /// [`MovementLimitsV1::max_report_delay_minutes`] has passed since it
    /// closed. Retirement keeps the identities reserved and the consumed pool
    /// capacity counted. Returns whether anything was retired.
    pub(crate) fn retire_closed(
        &mut self,
        at: SimTime,
        is_dead: &dyn Fn(PersonId) -> bool,
    ) -> Result<bool, MovementError> {
        let deadline = SimDuration::minutes(MovementLimitsV1::CURRENT.max_report_delay_minutes);
        let mut due = Vec::new();
        for order in self.orders.values() {
            let Some(closed) = order.closed_at else {
                continue;
            };
            let latest_delay = order
                .observers
                .iter()
                .map(|grant| grant.delay)
                .max()
                .unwrap_or(SimDuration::ZERO);
            if closed
                .checked_add(latest_delay)
                .is_none_or(|reports_due| at <= reports_due)
            {
                continue;
            }
            let expired = closed.checked_add(deadline).is_none_or(|limit| at > limit);
            let reported = order.observers.iter().all(|grant| {
                matches!(grant.holder, KnowledgeHolderRef::Person(person) if is_dead(person))
                    || self
                        .observation_heads
                        .binary_search_by(|head| {
                            (&head.holder, head.execution).cmp(&(&grant.holder, order.execution))
                        })
                        .is_ok_and(|index| self.observation_heads[index].observed_as_of >= closed)
            });
            if expired || reported {
                due.push((order.order.id, order.execution));
            }
        }
        let retired = !due.is_empty();
        for (order_id, execution_id) in due {
            self.retire(order_id, execution_id)?;
        }
        Ok(retired)
    }

    fn retire(
        &mut self,
        order_id: canwu_api::MovementOrderId,
        execution_id: TransportExecutionId,
    ) -> Result<(), MovementError> {
        let execution = self
            .executions
            .remove(&execution_id)
            .ok_or_else(|| MovementError::state("execution disappeared"))?;
        self.orders.remove(&order_id);
        for booking in &execution.bookings {
            let binding = self
                .bookings
                .remove(&booking.id)
                .ok_or_else(|| MovementError::state("booking binding disappeared"))?;
            if booking.status == CapacityBookingStatus::Consumed {
                let retired = self.retired_consumption.entry(binding.pool).or_default();
                *retired = retired
                    .checked_add(booking.quantity)
                    .ok_or_else(|| MovementError::state("retired consumption overflow"))?;
            }
        }
        self.observation_heads
            .retain(|head| head.execution != execution_id);
        self.operation_outcomes
            .retain(|_, outcome| outcome.execution != Some(execution_id));
        self.retired_orders.insert(order_id);
        self.retired_executions.insert(execution_id);
        Ok(())
    }

    /// Prunes outcomes that no live execution needs once they are older than
    /// the outcome retention period. Returns whether anything was pruned.
    pub(crate) fn prune_outcomes(&mut self, at: SimTime) -> bool {
        let retention = SimDuration::minutes(MovementLimitsV1::CURRENT.outcome_retention_minutes);
        let before = self.operation_outcomes.len();
        let executions = &self.executions;
        self.operation_outcomes.retain(|_, outcome| {
            outcome
                .execution
                .is_some_and(|execution| executions.contains_key(&execution))
                || outcome
                    .settled_at
                    .checked_add(retention)
                    .is_none_or(|expires| at <= expires)
        });
        before != self.operation_outcomes.len()
    }

    fn offer_pool(
        &mut self,
        offer: &MovementPoolOfferV1,
        holder: &KnowledgeHolderRef,
    ) -> Result<(), MovementError> {
        let text = MovementLimitsV1::CURRENT.text_bytes;
        if !is_label(&offer.pool, text) || !is_label(&offer.resource, text) {
            return Err(MovementError::invalid(
                "pool identity and resource must be bounded labels",
            ));
        }
        if !within_time_bound(offer.window_from) || !within_time_bound(offer.window_until) {
            return Err(MovementError::new(
                MovementErrorCode::LimitExceeded,
                "pool window exceeds the supported time bound",
            ));
        }
        if let Some(pool) = self.pools.get(&offer.pool) {
            if pool.custodian != *holder {
                return Err(unauthorized("only the pool custodian may revise its offer"));
            }
            if pool.resource != offer.resource {
                return Err(MovementError::invalid("a pool keeps its resource"));
            }
            let outside = self.bookings.values().any(|binding| {
                binding.pool == offer.pool
                    && self
                        .executions
                        .get(&binding.execution)
                        .and_then(|execution| {
                            execution
                                .bookings
                                .iter()
                                .find(|booking| booking.id == binding.booking)
                        })
                        .is_some_and(|booking| {
                            matches!(
                                booking.status,
                                CapacityBookingStatus::Confirmed | CapacityBookingStatus::Consumed
                            ) && (booking.valid_from < offer.window_from
                                || booking.valid_until > offer.window_until)
                        })
            });
            if outside {
                return Err(MovementError::invalid(
                    "a pool window cannot exclude capacity it has already granted",
                ));
            }
            let pool = self
                .pools
                .get_mut(&offer.pool)
                .ok_or_else(|| MovementError::state("pool disappeared"))?;
            pool.revise(offer.window_from, offer.window_until, offer.quantity)?;
        } else {
            if self.pools.len() >= MovementLimitsV1::CURRENT.pools_per_runtime {
                return Err(MovementError::new(
                    MovementErrorCode::LimitExceeded,
                    "movement runtime has no room for another capacity pool",
                ));
            }
            let pool = TransportCapacityPoolV1::new(
                offer.pool.clone(),
                offer.resource.clone(),
                holder.clone(),
                offer.window_from,
                offer.window_until,
                offer.quantity,
            )?;
            self.pools.insert(offer.pool.clone(), pool);
        }
        Ok(())
    }

    /// Departs the current leg: explicitly (a `StartLeg` operation) or at its
    /// departure due time.
    pub(crate) fn depart(
        &mut self,
        execution_id: TransportExecutionId,
        expected_leg: Option<LegExecutionId>,
        explicit: bool,
        at: SimTime,
        scheduled: &mut Vec<MovementLegDueV1>,
    ) -> Result<(), MovementError> {
        let execution = self
            .executions
            .get(&execution_id)
            .ok_or_else(|| MovementError::new(MovementErrorCode::NotFound, "unknown execution"))?;
        if execution.state.is_terminal()
            || matches!(
                execution.state,
                TransportExecutionState::ArrivalPending | TransportExecutionState::ReplanPending
            )
        {
            return Err(MovementError::state(
                "the execution cannot depart a leg now",
            ));
        }
        let (leg_id, revision, index) = current_leg(execution)?;
        if expected_leg.is_some_and(|expected| expected != leg_id) {
            return Err(MovementError::state("the named leg is not the current leg"));
        }
        let planned = route_leg(execution, revision, index)?;
        let duration = planned
            .planned_arrival_at
            .checked_sub(planned.planned_departure_at)
            .unwrap_or(SimDuration::ZERO);
        let mut waiting_until = None;
        let mut unavailable = false;
        let mut consume = Vec::new();
        for binding in self
            .bookings
            .values()
            .filter(|binding| binding.execution == execution_id && binding.leg == leg_id)
        {
            let booking = execution
                .bookings
                .iter()
                .find(|booking| booking.id == binding.booking)
                .ok_or_else(|| MovementError::state("booking binding has no booking"))?;
            match booking.status {
                CapacityBookingStatus::Requested => {
                    return Err(MovementError::state(
                        "the leg has a capacity request that is not yet allocated",
                    ));
                }
                CapacityBookingStatus::Failed | CapacityBookingStatus::Expired => {
                    unavailable = true;
                }
                CapacityBookingStatus::Confirmed if booking.valid_from > at => {
                    waiting_until =
                        Some(waiting_until.map_or(booking.valid_from, |time: SimTime| {
                            time.max(booking.valid_from)
                        }));
                }
                CapacityBookingStatus::Confirmed => {
                    consume.push((booking.id, binding.pool.clone(), booking.quantity));
                }
                _ => {}
            }
        }
        if unavailable {
            if explicit {
                return Err(MovementError::state(
                    "the leg's booked capacity is unavailable",
                ));
            }
            return self.fail_leg(execution_id, leg_id, "capacity_unavailable", at);
        }
        if let Some(opens) = waiting_until {
            if explicit {
                return Err(MovementError::state(
                    "the leg's booked capacity window has not opened",
                ));
            }
            return self.schedule(
                MovementLegDueV1 {
                    execution: execution_id,
                    revision,
                    leg: leg_id,
                    kind: MovementLegDueKind::Departure,
                    due_at: opens,
                },
                scheduled,
            );
        }
        self.execution_mut(execution_id)?.start_current_leg(at)?;
        for (booking, pool, quantity) in consume {
            self.transition_booking(
                execution_id,
                booking,
                &pool,
                quantity,
                CapacityBookingStatus::Consumed,
                at,
            )?;
        }
        let arrival = at
            .checked_add(duration)
            .ok_or_else(|| MovementError::invalid("leg arrival time overflows"))?;
        self.schedule(
            MovementLegDueV1 {
                execution: execution_id,
                revision,
                leg: leg_id,
                kind: MovementLegDueKind::Arrival,
                due_at: arrival,
            },
            scheduled,
        )
    }

    /// Arrives the current departed leg: explicitly (a `CompleteLeg`
    /// operation) or at its arrival due time.
    pub(crate) fn arrive(
        &mut self,
        execution_id: TransportExecutionId,
        expected_leg: Option<LegExecutionId>,
        at: SimTime,
        scheduled: &mut Vec<MovementLegDueV1>,
    ) -> Result<(), MovementError> {
        let execution = self
            .executions
            .get(&execution_id)
            .ok_or_else(|| MovementError::new(MovementErrorCode::NotFound, "unknown execution"))?;
        let (leg_id, revision, index) = current_leg(execution)?;
        if expected_leg.is_some_and(|expected| expected != leg_id) {
            return Err(MovementError::state("the named leg is not the current leg"));
        }
        let leg_count = execution
            .legs
            .iter()
            .filter(|leg| leg.itinerary_revision == revision)
            .count();
        let endpoint = route_leg(execution, revision, index)?
            .to
            .as_str()
            .to_owned();
        let settles = index + 1 == leg_count && execution.delivery_attempt.is_none();
        if settles {
            self.execution_mut(execution_id)?
                .settle_arrival(at, endpoint)?;
            return self.close(execution_id, at, CapacityBookingStatus::Released);
        }
        let arrived_everywhere = self
            .execution_mut(execution_id)?
            .complete_current_leg(at, endpoint)?;
        self.order_mut(execution_id)?.pending_due = None;
        if arrived_everywhere {
            return Ok(());
        }
        let execution = self
            .executions
            .get(&execution_id)
            .ok_or_else(|| MovementError::state("execution disappeared"))?;
        let (next, revision, index) = current_leg(execution)?;
        let departure = later(
            at,
            route_leg(execution, revision, index)?.planned_departure_at,
        );
        self.schedule(
            MovementLegDueV1 {
                execution: execution_id,
                revision,
                leg: next,
                kind: MovementLegDueKind::Departure,
                due_at: departure,
            },
            scheduled,
        )
    }

    fn fail_leg(
        &mut self,
        execution_id: TransportExecutionId,
        leg: LegExecutionId,
        reason: &str,
        at: SimTime,
    ) -> Result<(), MovementError> {
        if !is_label(reason, MovementLimitsV1::CURRENT.text_bytes) {
            return Err(MovementError::invalid(
                "a leg failure needs a bounded reason",
            ));
        }
        let execution = self
            .executions
            .get(&execution_id)
            .ok_or_else(|| MovementError::new(MovementErrorCode::NotFound, "unknown execution"))?;
        if execution.state.is_terminal()
            || matches!(
                execution.state,
                TransportExecutionState::ArrivalPending | TransportExecutionState::ReplanPending
            )
        {
            return Err(MovementError::state(
                "the execution has no leg that can fail",
            ));
        }
        let (current, _, _) = current_leg(execution)?;
        let status = execution
            .legs
            .iter()
            .find(|item| item.id == current)
            .map(|item| item.status)
            .ok_or_else(|| MovementError::state("current leg disappeared"))?;
        if current != leg
            || !matches!(
                status,
                LegExecutionStatus::Planned
                    | LegExecutionStatus::Booked
                    | LegExecutionStatus::Loaded
                    | LegExecutionStatus::Waiting
                    | LegExecutionStatus::Departed
            )
        {
            return Err(MovementError::state(
                "the named leg is not the current open leg",
            ));
        }
        self.execution_mut(execution_id)?
            .fail_current_leg(reason.to_owned(), at)?;
        self.release_leg_bookings(execution_id, |binding| binding.leg == leg, at)?;
        self.order_mut(execution_id)?.pending_due = None;
        Ok(())
    }

    fn reroute(
        &mut self,
        execution_id: TransportExecutionId,
        revision: &ItineraryRevision,
        at: SimTime,
        reader: &dyn EvidenceReader,
        scheduled: &mut Vec<MovementLegDueV1>,
    ) -> Result<(), MovementError> {
        let limits = MovementLimitsV1::CURRENT;
        let destination = self
            .order_for_execution(execution_id)
            .map(|order| order.order.destination.clone())
            .ok_or_else(|| MovementError::new(MovementErrorCode::NotFound, "unknown execution"))?;
        let execution = self
            .executions
            .get(&execution_id)
            .ok_or_else(|| MovementError::new(MovementErrorCode::NotFound, "unknown execution"))?;
        if execution.state.is_terminal()
            || execution.state == TransportExecutionState::ArrivalPending
        {
            return Err(MovementError::state("the execution cannot be rerouted"));
        }
        let active = execution
            .active_itinerary_revision
            .ok_or_else(|| MovementError::state("execution has no active itinerary"))?;
        let active_legs = execution
            .legs
            .iter()
            .filter(|leg| leg.itinerary_revision == active);
        // A leg that failed after departing leaves the subject somewhere on
        // the way, so its replacement may start anywhere; otherwise the
        // replacement starts where the movement stands.
        let mut failed_on_the_way = false;
        for leg in active_legs {
            match leg.status {
                LegExecutionStatus::Departed => {
                    return Err(MovementError::state(
                        "a departed leg must arrive or fail before a reroute",
                    ));
                }
                LegExecutionStatus::Failed if leg.actual_departure_at.is_some() => {
                    failed_on_the_way = true;
                }
                _ => {}
            }
        }
        if execution.revisions.len() >= limits.revisions_per_execution
            || execution.legs.len() + revision.plan.legs.len() > limits.legs_per_execution
        {
            return Err(MovementError::new(
                MovementErrorCode::LimitExceeded,
                "reroute exceeds movement limits",
            ));
        }
        validate_plan(&revision.plan, None, &destination)?;
        if !failed_on_the_way
            && execution.current_endpoint.as_deref() != Some(revision.plan.origin.as_str())
        {
            return Err(MovementError::invalid(
                "a reroute without a failed leg starts where the movement stands",
            ));
        }
        if revision.planned_at > at
            || revision.superseded_at.is_some()
            || !within_time_bound(revision.valid_from)
        {
            return Err(MovementError::invalid(
                "a reroute is planned no later than its admission and is not superseded",
            ));
        }
        if let ItineraryRevisionReason::ExternalCondition {
            record, version, ..
        } = &revision.reason
            && !reader
                .condition_exists(record, *version)
                .map_err(|error| evidence_error(&error))?
        {
            return Err(MovementError::new(
                MovementErrorCode::Evidence,
                "the cited external condition version does not exist",
            ));
        }
        self.execution_mut(execution_id)?
            .reroute(revision.clone(), at)?;
        self.release_leg_bookings(execution_id, |binding| binding.leg_revision == active, at)?;
        let execution = self
            .executions
            .get(&execution_id)
            .ok_or_else(|| MovementError::state("execution disappeared"))?;
        let (first, installed, index) = current_leg(execution)?;
        let departure = later(
            later(at, revision.valid_from),
            route_leg(execution, installed, index)?.planned_departure_at,
        );
        self.schedule(
            MovementLegDueV1 {
                execution: execution_id,
                revision: installed,
                leg: first,
                kind: MovementLegDueKind::Departure,
                due_at: departure,
            },
            scheduled,
        )
    }

    fn record_handoff(
        &mut self,
        execution_id: TransportExecutionId,
        handoff: &Handoff,
        at: SimTime,
    ) -> Result<(), MovementError> {
        let limits = MovementLimitsV1::CURRENT;
        let execution = self.execution_mut(execution_id)?;
        if execution.state.is_terminal() {
            return Err(MovementError::state(
                "a terminal execution records no handoff",
            ));
        }
        let active = execution.active_itinerary_revision;
        if handoff.at > at
            || !execution
                .legs
                .iter()
                .any(|leg| leg.id == handoff.to_leg && Some(leg.itinerary_revision) == active)
        {
            return Err(MovementError::state(
                "a handoff is recorded no later than now into a leg of the active itinerary",
            ));
        }
        if execution.handoffs.len() >= limits.handoffs_per_execution
            || [
                &handoff.from_custodian,
                &handoff.to_custodian,
                &handoff.location,
            ]
            .iter()
            .any(|label| label.len() > limits.text_bytes)
        {
            return Err(MovementError::new(
                MovementErrorCode::LimitExceeded,
                "handoff exceeds movement limits",
            ));
        }
        execution.record_handoff(handoff.clone())?;
        Ok(())
    }

    fn request_booking(&mut self, request: &MovementBookingRequestV1) -> Result<(), MovementError> {
        let limits = MovementLimitsV1::CURRENT;
        let booking = &request.booking;
        if !is_label(&request.tie_break, limits.text_bytes) {
            return Err(MovementError::invalid(
                "a booking needs a bounded tie-break key",
            ));
        }
        let pool = self.pools.get(&request.pool).ok_or_else(|| {
            MovementError::new(MovementErrorCode::NotFound, "unknown capacity pool")
        })?;
        if pool.resource != booking.resource {
            return Err(MovementError::invalid(
                "a booking must request its pool resource",
            ));
        }
        if self.bookings.contains_key(&booking.id) {
            return Err(MovementError::new(
                MovementErrorCode::Conflict,
                "capacity booking identity is already used",
            ));
        }
        let execution = self
            .executions
            .get(&booking.execution)
            .ok_or_else(|| MovementError::new(MovementErrorCode::NotFound, "unknown execution"))?;
        if execution.state == TransportExecutionState::ArrivalPending
            || execution.bookings.len() >= limits.bookings_per_execution
        {
            return Err(MovementError::state(
                "the execution cannot book more capacity",
            ));
        }
        let active = execution
            .active_itinerary_revision
            .ok_or_else(|| MovementError::state("execution has no active itinerary"))?;
        let leg = execution
            .legs
            .iter()
            .find(|leg| leg.id == request.leg && leg.itinerary_revision == active)
            .filter(|leg| {
                matches!(
                    leg.status,
                    LegExecutionStatus::Planned
                        | LegExecutionStatus::Booked
                        | LegExecutionStatus::Waiting
                )
            })
            .ok_or_else(|| {
                MovementError::state(
                    "capacity is booked for an unstarted leg of the active itinerary",
                )
            })?;
        let planned = route_leg(execution, active, leg.leg_index)?;
        if booking.valid_from < planned.planned_departure_at
            || booking.valid_until > planned.planned_arrival_at
        {
            return Err(MovementError::invalid(
                "a booking window must lie inside its leg's planned window",
            ));
        }
        self.execution_mut(booking.execution)?
            .request_booking(booking.clone())?;
        let sequence = self.next_booking_sequence;
        self.next_booking_sequence = sequence
            .checked_add(1)
            .ok_or_else(|| MovementError::state("booking sequence overflow"))?;
        self.bookings.insert(
            booking.id,
            MovementBookingBindingV1 {
                booking: booking.id,
                execution: booking.execution,
                pool: request.pool.clone(),
                leg: request.leg,
                tie_break: request.tie_break.clone(),
                admitted_sequence: sequence,
                allocation: None,
            },
        );
        Ok(())
    }

    fn reconcile(
        &mut self,
        execution_id: TransportExecutionId,
        outcome: &ReconciliationOutcome,
        at: SimTime,
    ) -> Result<(), MovementError> {
        let execution = self.execution_mut(execution_id)?;
        if execution.state != TransportExecutionState::ArrivalPending
            || execution
                .saga
                .as_ref()
                .is_none_or(|saga| saga.state != SagaState::ArrivalPending)
        {
            return Err(MovementError::state(
                "only an arrival-pending delivery can be reconciled",
            ));
        }
        if let ReconciliationOutcome::Failure { error } = outcome
            && !is_label(error, MovementLimitsV1::CURRENT.text_bytes)
        {
            return Err(MovementError::invalid(
                "a reconciliation failure needs a bounded reason",
            ));
        }
        execution.reconcile_information(outcome.clone())?;
        self.close(execution_id, at, CapacityBookingStatus::Released)
    }

    /// Closes a terminal execution: cancels unallocated requests, returns
    /// confirmed capacity, and clears due work.
    fn close(
        &mut self,
        execution_id: TransportExecutionId,
        at: SimTime,
        confirmed_to: CapacityBookingStatus,
    ) -> Result<(), MovementError> {
        self.release_leg_bookings_with(execution_id, |_| true, at, confirmed_to)?;
        let order = self.order_mut(execution_id)?;
        order.pending_due = None;
        order.closed_at = Some(at);
        Ok(())
    }

    fn release_leg_bookings(
        &mut self,
        execution_id: TransportExecutionId,
        select: impl Fn(&BindingView) -> bool,
        at: SimTime,
    ) -> Result<(), MovementError> {
        self.release_leg_bookings_with(execution_id, select, at, CapacityBookingStatus::Released)
    }

    fn release_leg_bookings_with(
        &mut self,
        execution_id: TransportExecutionId,
        select: impl Fn(&BindingView) -> bool,
        at: SimTime,
        confirmed_to: CapacityBookingStatus,
    ) -> Result<(), MovementError> {
        let execution = self
            .executions
            .get(&execution_id)
            .ok_or_else(|| MovementError::state("execution disappeared"))?;
        let mut changes = Vec::new();
        for binding in self
            .bookings
            .values()
            .filter(|binding| binding.execution == execution_id)
        {
            let leg_revision = execution
                .legs
                .iter()
                .find(|leg| leg.id == binding.leg)
                .map(|leg| leg.itinerary_revision)
                .ok_or_else(|| MovementError::state("booking binds a missing leg"))?;
            if !select(&BindingView {
                leg: binding.leg,
                leg_revision,
            }) {
                continue;
            }
            let booking = execution
                .bookings
                .iter()
                .find(|booking| booking.id == binding.booking)
                .ok_or_else(|| MovementError::state("booking binding has no booking"))?;
            let target = match booking.status {
                CapacityBookingStatus::Requested => CapacityBookingStatus::Cancelled,
                CapacityBookingStatus::Confirmed => confirmed_to,
                _ => continue,
            };
            changes.push((booking.id, binding.pool.clone(), booking.quantity, target));
        }
        for (booking, pool, quantity, target) in changes {
            self.transition_booking(execution_id, booking, &pool, quantity, target, at)?;
        }
        Ok(())
    }

    fn transition_booking(
        &mut self,
        execution_id: TransportExecutionId,
        booking_id: canwu_api::CapacityBookingId,
        pool_id: &str,
        quantity: u64,
        to: CapacityBookingStatus,
        at: SimTime,
    ) -> Result<(), MovementError> {
        let execution = self.execution_mut(execution_id)?;
        let booking = execution
            .bookings
            .iter_mut()
            .find(|booking| booking.id == booking_id)
            .ok_or_else(|| MovementError::state("booking disappeared"))?;
        let from = booking.status;
        booking.transition(to, at)?;
        self.pools
            .get_mut(pool_id)
            .ok_or_else(|| MovementError::state("booking pool disappeared"))?
            .apply_booking_transition(quantity, from, to)?;
        Ok(())
    }

    /// Expires confirmed bookings whose window has ended, then allocates every
    /// requested booking of every pool in one deterministic pass per pool.
    pub(crate) fn allocate_pools(
        &mut self,
        at: SimTime,
        boundary: BoundaryId,
    ) -> Result<bool, MovementError> {
        let mut changed = false;
        let mut expired = Vec::new();
        for binding in self.bookings.values() {
            let booking = self
                .executions
                .get(&binding.execution)
                .and_then(|execution| {
                    execution
                        .bookings
                        .iter()
                        .find(|booking| booking.id == binding.booking)
                })
                .ok_or_else(|| MovementError::state("booking binding has no booking"))?;
            if booking.status == CapacityBookingStatus::Confirmed && at > booking.valid_until {
                expired.push((
                    binding.execution,
                    booking.id,
                    binding.pool.clone(),
                    booking.quantity,
                ));
            }
        }
        for (execution, booking, pool, quantity) in expired {
            self.transition_booking(
                execution,
                booking,
                &pool,
                quantity,
                CapacityBookingStatus::Expired,
                at,
            )?;
            changed = true;
        }
        let pool_ids = self.pools.keys().cloned().collect::<Vec<_>>();
        for pool_id in pool_ids {
            let mut requests = Vec::new();
            for binding in self
                .bookings
                .values()
                .filter(|binding| binding.pool == pool_id)
            {
                let booking = self
                    .executions
                    .get(&binding.execution)
                    .and_then(|execution| {
                        execution
                            .bookings
                            .iter()
                            .find(|booking| booking.id == binding.booking)
                    })
                    .ok_or_else(|| MovementError::state("booking binding has no booking"))?;
                if booking.status == CapacityBookingStatus::Requested {
                    requests.push(CapacityBookingRequestV1 {
                        booking: booking.clone(),
                        tie_break: binding.tie_break.clone(),
                        admitted_sequence: binding.admitted_sequence,
                    });
                }
            }
            if requests.is_empty() {
                continue;
            }
            let pool = self
                .pools
                .get_mut(&pool_id)
                .ok_or_else(|| MovementError::state("pool disappeared"))?;
            let allocations = allocate_capacity_bookings(pool, &requests, at)?;
            pool.apply_allocations(&allocations)?;
            for allocation in allocations {
                let binding = self
                    .bookings
                    .get_mut(&allocation.booking)
                    .ok_or_else(|| MovementError::state("booking binding disappeared"))?;
                let execution = self
                    .executions
                    .get_mut(&binding.execution)
                    .ok_or_else(|| MovementError::state("execution disappeared"))?;
                let booking = execution
                    .bookings
                    .iter_mut()
                    .find(|booking| booking.id == allocation.booking)
                    .ok_or_else(|| MovementError::state("booking disappeared"))?;
                booking.transition(allocation.status, at)?;
                booking
                    .allocation_evidence
                    .push(EvidenceRef::Boundary(boundary));
                binding.allocation = Some(allocation.evidence);
            }
            changed = true;
        }
        Ok(changed)
    }

    /// Applies one leg-due packet when it is still the order's pending due
    /// work, and returns whether the state changed.
    ///
    /// Settling due work never fails the boundary. When the departure or
    /// arrival cannot be applied, the current leg fails with
    /// `due_settlement_failed`; when even that is impossible, the state is
    /// left unchanged and the owner or operator may complete, fail, or cancel
    /// the leg explicitly.
    pub(crate) fn apply_due(
        &mut self,
        due: &MovementLegDueV1,
        at: SimTime,
        scheduled: &mut Vec<MovementLegDueV1>,
    ) -> bool {
        let Some(order) = self.order_for_execution(due.execution) else {
            return false;
        };
        if order.pending_due.as_ref() != Some(due) || due.due_at > at {
            return false;
        }
        let mut candidate = self.clone();
        let mut local = Vec::new();
        if candidate
            .settle_due(due, at, &mut local)
            .and_then(|()| candidate.validate())
            .is_ok()
        {
            *self = candidate;
            scheduled.extend(local);
            return true;
        }
        let mut fallback = self.clone();
        let failed = fallback
            .order_mut(due.execution)
            .map(|order| order.pending_due = None)
            .and_then(|()| fallback.fail_leg(due.execution, due.leg, "due_settlement_failed", at))
            .and_then(|()| fallback.validate());
        if failed.is_ok() {
            *self = fallback;
            return true;
        }
        false
    }

    fn settle_due(
        &mut self,
        due: &MovementLegDueV1,
        at: SimTime,
        scheduled: &mut Vec<MovementLegDueV1>,
    ) -> Result<(), MovementError> {
        self.order_mut(due.execution)?.pending_due = None;
        match due.kind {
            MovementLegDueKind::Departure => {
                self.depart(due.execution, Some(due.leg), false, at, scheduled)
            }
            MovementLegDueKind::Arrival => self.arrive(due.execution, Some(due.leg), at, scheduled),
        }
    }
}

/// The booking facts a release filter selects on.
pub(crate) struct BindingView {
    pub(crate) leg: LegExecutionId,
    pub(crate) leg_revision: ItineraryRevisionId,
}

fn current_leg(
    execution: &TransportExecution,
) -> Result<(LegExecutionId, ItineraryRevisionId, usize), MovementError> {
    let revision = execution
        .active_itinerary_revision
        .ok_or_else(|| MovementError::state("execution has no active itinerary"))?;
    execution
        .legs
        .iter()
        .find(|leg| {
            leg.itinerary_revision == revision && leg.leg_index == execution.current_leg_index
        })
        .map(|leg| (leg.id, revision, leg.leg_index))
        .ok_or_else(|| MovementError::state("execution has no current leg"))
}

fn observer_grants(
    owner: &KnowledgeHolderRef,
    request: &MovementOrderRequestV1,
) -> Result<Vec<MovementObserverGrant>, MovementError> {
    let mut grants = Vec::new();
    if matches!(owner, KnowledgeHolderRef::Person(_)) {
        grants.push(MovementObserverGrant {
            holder: owner.clone(),
            role: MovementObserverRole::Owner,
            delay: SimDuration::ZERO,
        });
    }
    if let Some(operator) = &request.operator
        && operator != owner
    {
        grants.push(MovementObserverGrant {
            holder: operator.clone(),
            role: MovementObserverRole::Operator,
            delay: SimDuration::ZERO,
        });
    }
    for remote in &request.remote_observers {
        if remote.delay <= SimDuration::ZERO
            || remote.delay.as_minutes() > MovementLimitsV1::CURRENT.max_report_delay_minutes
        {
            return Err(MovementError::invalid(
                "a remote observer needs a positive delay within the report delay limit",
            ));
        }
        grants.push(MovementObserverGrant {
            holder: remote.holder.clone(),
            role: MovementObserverRole::DelayedRemote,
            delay: remote.delay,
        });
    }
    if grants
        .iter()
        .any(|grant| !matches!(grant.holder, KnowledgeHolderRef::Person(_)))
    {
        return Err(MovementError::invalid(
            "operators and remote observers are person holders",
        ));
    }
    grants.sort_by(|left, right| left.holder.cmp(&right.holder));
    if grants
        .windows(2)
        .any(|pair| pair[0].holder == pair[1].holder)
    {
        return Err(MovementError::invalid("an observer holder is named twice"));
    }
    Ok(grants)
}
