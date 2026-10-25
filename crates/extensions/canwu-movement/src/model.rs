use crate::{PLUGIN_NAME, PLUGIN_NAMESPACE};
use canwu_api::{
    CanwuError, CapacityBooking, CapacityBookingAllocationEvidenceV1, CapacityBookingId,
    CapacityBookingStatus, CommandId, DomainRecord, DomainRecordClass, DomainRecordDraft,
    DomainRecordKind, DomainRecordLifecycle, DomainRecordType, DomainRecordVersionRef,
    DomainValueKindClass, ErrorCode, Handoff, IngressId, ItineraryRevision, ItineraryRevisionId,
    KnowledgeHolderRef, LegExecution, LegExecutionId, LegExecutionStatus, MovementOrder,
    MovementOrderId, ReconciliationOutcome, RoutePlan, RoutingNodeRef, SimDuration, SimTime,
    TransportCapacityPoolV1, TransportError, TransportExecution, TransportExecutionId,
    TransportExecutionState, TypedDomainRecordRef,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Display, Formatter};

/// Stable identity of the single movement runtime record.
pub const MOVEMENT_RUNTIME_ID: &str = "canwu.movement:runtime";

/// Hard bounds on one movement runtime. Operations that would exceed them are
/// rejected as [`MovementErrorCode::LimitExceeded`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MovementLimitsV1 {
    pub text_bytes: usize,
    /// Encoded size of one command or incident payload.
    pub operation_bytes: usize,
    /// Every planned, booked, and pooled time lies within this many minutes
    /// of the epoch, so due times never overflow.
    pub time_bound_minutes: i64,
    pub subjects_per_order: usize,
    pub observers_per_order: usize,
    pub legs_per_execution: usize,
    pub revisions_per_execution: usize,
    pub handoffs_per_execution: usize,
    /// Bookings one execution keeps at once: those that ask for or hold
    /// capacity, those the leg under way consumed, and those lost for a leg
    /// that has not departed and not yet replaced. A closed booking leaves
    /// the execution ([`MovementState::validate`]), so it does not count, and
    /// an execution may book again any number of times over its life.
    pub bookings_per_execution: usize,
    /// Bookings the whole runtime keeps at once, across its executions.
    pub bookings_per_runtime: usize,
    pub report_holders_per_boundary: usize,
    pub reports_per_boundary: usize,
    pub orders_per_runtime: usize,
    /// Orders one owner may hold before they retire.
    pub orders_per_owner: usize,
    pub pools_per_runtime: usize,
    /// Longest delay a remote observer grant may carry; a closed execution
    /// retires at the latest this long after it closes.
    pub max_report_delay_minutes: i64,
    /// Hard bound on stored operation outcomes. Commands are refused at
    /// admission once `operation_outcomes_per_runtime -
    /// incident_outcome_headroom` are stored, so incidents keep room.
    pub operation_outcomes_per_runtime: usize,
    pub incident_outcome_headroom: usize,
    /// Commands one holder may have outcomes for at a time.
    pub operation_outcomes_per_holder: usize,
    /// Outcomes that no live execution needs are pruned after this long.
    pub outcome_retention_minutes: i64,
}

impl MovementLimitsV1 {
    pub const CURRENT: Self = Self {
        text_bytes: 256,
        operation_bytes: 8_192,
        time_bound_minutes: 1 << 52,
        subjects_per_order: 32,
        observers_per_order: 16,
        legs_per_execution: 64,
        revisions_per_execution: 8,
        handoffs_per_execution: 16,
        bookings_per_execution: 64,
        bookings_per_runtime: 16_384,
        report_holders_per_boundary: 64,
        reports_per_boundary: 1_024,
        orders_per_runtime: 1_024,
        orders_per_owner: 256,
        pools_per_runtime: 256,
        max_report_delay_minutes: 365 * 24 * 60,
        operation_outcomes_per_runtime: 16_384,
        incident_outcome_headroom: 1_024,
        operation_outcomes_per_holder: 1_024,
        outcome_retention_minutes: 7 * 24 * 60,
    };
}

/// Stable machine-readable reason carried by a rejected operation outcome.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MovementErrorCode {
    InvalidOperation,
    InvalidState,
    Unauthorized,
    NotFound,
    Conflict,
    LimitExceeded,
    Transport,
    Evidence,
}

impl MovementErrorCode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidOperation => "invalid_operation",
            Self::InvalidState => "invalid_state",
            Self::Unauthorized => "unauthorized",
            Self::NotFound => "not_found",
            Self::Conflict => "conflict",
            Self::LimitExceeded => "limit_exceeded",
            Self::Transport => "transport",
            Self::Evidence => "evidence",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MovementError {
    pub code: MovementErrorCode,
    pub message: String,
}

impl MovementError {
    pub fn new(code: MovementErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self::new(MovementErrorCode::InvalidOperation, message)
    }

    pub(crate) fn state(message: impl Into<String>) -> Self {
        Self::new(MovementErrorCode::InvalidState, message)
    }
}

impl Display for MovementError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for MovementError {}

impl From<TransportError> for MovementError {
    fn from(error: TransportError) -> Self {
        Self::new(MovementErrorCode::Transport, format!("{error:?}"))
    }
}

impl From<MovementError> for CanwuError {
    fn from(error: MovementError) -> Self {
        let code = match error.code {
            MovementErrorCode::InvalidOperation => ErrorCode::InvalidPayload,
            MovementErrorCode::InvalidState | MovementErrorCode::Transport => {
                ErrorCode::InvalidDomainRecord
            }
            MovementErrorCode::Unauthorized => ErrorCode::InvalidAuthority,
            MovementErrorCode::NotFound => ErrorCode::DomainRecordNotFound,
            MovementErrorCode::Conflict => ErrorCode::IdempotencyConflict,
            MovementErrorCode::LimitExceeded => ErrorCode::ValueOutOfRange,
            MovementErrorCode::Evidence => ErrorCode::EvidenceUnavailable,
        };
        Self::new(code, format!("canwu-movement: {}", error.message))
    }
}

/// One authority-bound movement operation submitted through
/// [`crate::MOVEMENT_COMMAND`].
///
/// `holder` must be the command issuer's own holder: the actor for an actor
/// issuer, or the command subject for a human, AI, or institution issuer.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MovementCommandV1 {
    pub holder: KnowledgeHolderRef,
    pub operation_key: String,
    pub operation: MovementOperation,
}

/// An application-reported leg failure, leg delay, reroute, handoff
/// (including a seizure), booking revocation, or delivery reconciliation
/// delivered as [`crate::MOVEMENT_INCIDENT_INGRESS`].
///
/// `evidence` must be the current version of a record whose kind was
/// declared to [`crate::MovementPlugin::new`] when the incident settles; a
/// superseded version is refused even while the run still retains it, so a
/// sealed run and its exact replay decide alike. The movement plugin never
/// draws the incident itself.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MovementIncidentV1 {
    pub operation_key: String,
    pub evidence: DomainRecordVersionRef,
    pub operation: MovementOperation,
}

/// A movement lifecycle operation. Leg operations name the expected current
/// leg so a stale request cannot act on a later leg.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum MovementOperation {
    /// Admit a movement order and create its transport execution.
    Order(MovementOrderRequestV1),
    /// Depart the current leg now instead of at its planned departure.
    StartLeg {
        execution: TransportExecutionId,
        leg: LegExecutionId,
    },
    /// Arrive the current departed leg now instead of at its due time.
    CompleteLeg {
        execution: TransportExecutionId,
        leg: LegExecutionId,
    },
    /// Record that the current leg failed; the execution waits for a reroute.
    FailLeg {
        execution: TransportExecutionId,
        leg: LegExecutionId,
        reason: String,
    },
    /// Hold the current leg until `until`: the arrival of a departed leg, or
    /// the departure of a leg that has not departed. The hold replaces that
    /// due and creates no itinerary revision. `until` is absolute and no
    /// earlier than the settling boundary or the due it holds (the arrival
    /// scheduled when the leg departed, or the departure pending when the
    /// leg was first held), so a later delay can lengthen, shorten, or lift
    /// a hold but never make the leg earlier. Only the latest hold is kept
    /// ([`MovementOrderRecordV1::hold`]).
    ///
    /// With `retime_later_legs`, the legs after the held one keep their
    /// planned spacing: when the hold ends, the part of it that elapsed is
    /// added to [`MovementOrderRecordV1::schedule_shift`], which moves their
    /// planned departures and booking windows. Without it they keep their
    /// planned times, and a leg after a held arrival departs no earlier than
    /// that arrival.
    DelayLeg {
        execution: TransportExecutionId,
        leg: LegExecutionId,
        until: SimTime,
        reason: String,
        retime_later_legs: bool,
    },
    /// Install a successor itinerary revision.
    Reroute {
        execution: TransportExecutionId,
        revision: ItineraryRevision,
    },
    /// Record a custody handoff into a leg of the active itinerary. A command
    /// may record only a planned handoff; a seizure is an incident.
    RecordHandoff {
        execution: TransportExecutionId,
        handoff: Handoff,
    },
    /// Request pool capacity for one unstarted leg, optionally in place of
    /// one of its bookings ([`MovementBookingRequestV1::replaces`]).
    RequestBooking(MovementBookingRequestV1),
    /// Revoke a confirmed booking on a pool, by the pool's custodian or an
    /// application incident. The booking becomes
    /// [`CapacityBookingStatus::Revoked`] with `reason` and its quantity
    /// returns to the pool. When its leg is due to depart, the leg fails with
    /// `capacity_unavailable` unless the execution's owner or operator has
    /// replaced the booking ([`MovementBookingRequestV1::replaces`]).
    RevokeBooking {
        pool: String,
        booking: CapacityBookingId,
        reason: String,
    },
    /// Cancel an execution whose subject is not travelling.
    Cancel {
        execution: TransportExecutionId,
        reason: String,
    },
    /// Reconcile an arrival-pending execution that carries a delivery attempt.
    /// Accepted only as an incident citing that delivery-attempt record.
    Reconcile {
        execution: TransportExecutionId,
        outcome: ReconciliationOutcome,
    },
    /// Offer or revise a capacity pool held by the acting holder.
    OfferPool(MovementPoolOfferV1),
}

impl MovementOperation {
    /// The execution an operation acts on, when it names one.
    #[must_use]
    pub const fn execution(&self) -> Option<TransportExecutionId> {
        match self {
            Self::Order(request) => Some(request.execution),
            Self::StartLeg { execution, .. }
            | Self::CompleteLeg { execution, .. }
            | Self::FailLeg { execution, .. }
            | Self::DelayLeg { execution, .. }
            | Self::Reroute { execution, .. }
            | Self::RecordHandoff { execution, .. }
            | Self::Cancel { execution, .. }
            | Self::Reconcile { execution, .. } => Some(*execution),
            Self::RequestBooking(request) => Some(request.booking.execution),
            Self::OfferPool(_) | Self::RevokeBooking { .. } => None,
        }
    }
}

/// Order admission. The acting holder becomes the order owner.
///
/// With a `delivery_attempt` (for example the application record whose
/// delivery the movement completes), arrival enters `ArrivalPending` and the
/// application reconciles it with a [`MovementOperation::Reconcile`]
/// incident; without one, arrival of the final leg settles the execution.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MovementOrderRequestV1 {
    pub execution: TransportExecutionId,
    pub order: MovementOrder,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_attempt: Option<DomainRecordVersionRef>,
    /// The person who carries out the movement, when different from the owner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator: Option<KnowledgeHolderRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remote_observers: Vec<MovementRemoteObserverV1>,
    /// Application record that authorizes the owner to move subjects other
    /// than itself: the current version of a record whose kind was declared
    /// to [`crate::MovementPlugin::new`] and whose references name the owner
    /// under [`crate::MOVEMENT_GRANTEE_ROLE`] and every other subject under
    /// [`crate::MOVEMENT_SUBJECT_ROLE`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority_basis: Option<DomainRecordVersionRef>,
}

/// A person who learns of the movement only after a positive delay.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MovementRemoteObserverV1 {
    pub holder: KnowledgeHolderRef,
    pub delay: SimDuration,
}

/// Capacity request for one leg of the active itinerary. The booking window
/// must lie inside the leg's planned window, moved by the holds that re-time
/// it (see [`MovementOperation::DelayLeg`]).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MovementBookingRequestV1 {
    pub booking: CapacityBooking,
    pub pool: String,
    pub leg: LegExecutionId,
    pub tie_break: String,
    /// A booking of the same leg that this one replaces, once. A requested
    /// one is cancelled and a confirmed one released when the replacement
    /// is admitted; a failed, expired, or revoked one keeps its status. The
    /// replaced booking then leaves the execution, so it no longer counts
    /// when the leg departs or against
    /// [`MovementLimitsV1::bookings_per_execution`], and a lost booking can
    /// be requested again without a reroute, as often as it is lost.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaces: Option<CapacityBookingId>,
}

/// Capacity offered by the acting holder, who becomes or must already be the
/// pool custodian.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MovementPoolOfferV1 {
    pub pool: String,
    pub resource: String,
    pub window_from: SimTime,
    pub window_until: SimTime,
    pub quantity: u64,
}

/// What an observer of one movement is entitled to see.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MovementObserverRole {
    /// The person carrying out the movement: current, detailed reports.
    Operator,
    /// The ordering holder: current, detailed reports.
    Owner,
    /// A distant person: progress only, as it stood `delay` earlier.
    DelayedRemote,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MovementObserverGrant {
    pub holder: KnowledgeHolderRef,
    pub role: MovementObserverRole,
    /// Zero for the operator and owner, positive for a delayed remote observer.
    pub delay: SimDuration,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MovementLegDueKind {
    Departure,
    Arrival,
}

/// Payload of [`crate::MOVEMENT_LEG_DUE_INGRESS`]. A due packet acts only when
/// it equals the order's persisted `pending_due`; any other packet is stale.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MovementLegDueV1 {
    pub execution: TransportExecutionId,
    pub revision: ItineraryRevisionId,
    pub leg: LegExecutionId,
    pub kind: MovementLegDueKind,
    pub due_at: SimTime,
}

/// Plugin bookkeeping for one admitted order and its execution.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MovementOrderRecordV1 {
    pub order: MovementOrder,
    pub execution: TransportExecutionId,
    pub owner: KnowledgeHolderRef,
    /// Report grants sorted by holder; only person holders receive reports.
    pub observers: Vec<MovementObserverGrant>,
    pub operation_key: String,
    pub admitted_at: SimTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_due: Option<MovementLegDueV1>,
    /// The latest hold on the current leg; its `until` is the pending due it
    /// holds. Cleared when the held departure or arrival settles, or the leg
    /// departs, arrives, or fails, or the execution is rerouted or closes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold: Option<MovementLegHoldV1>,
    /// How far behind their planned times the legs of the active revision
    /// that have not departed run, by the holds that re-timed them. A leg's
    /// scheduled departure after the previous leg arrives is no earlier than
    /// its planned departure plus this shift (an explicit `StartLeg` can
    /// still depart it earlier), and a booking requested for it must lie
    /// inside its planned window moved by the shift. Zero when no hold re-timed the
    /// revision; a reroute resets it.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub schedule_shift: SimDuration,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<SimTime>,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero(duration: &SimDuration) -> bool {
    *duration == SimDuration::ZERO
}

/// The current hold on the current leg, set by
/// [`MovementOperation::DelayLeg`]. Owner and operator reports show it; a
/// delayed remote observer learns of it only from the leg's departure or
/// arrival time.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MovementLegHoldV1 {
    pub leg: LegExecutionId,
    /// The due the hold moves: the departure of a leg that has not departed,
    /// or the arrival of the departed leg.
    pub kind: MovementLegDueKind,
    /// The due the hold replaced, which it never ends before: for an
    /// arrival, the leg's actual departure plus its planned duration; for a
    /// departure, the departure that was pending when the leg was first
    /// held.
    pub scheduled_at: SimTime,
    /// The held departure or arrival, the pending due; equal to
    /// `scheduled_at` once a hold is lifted.
    pub until: SimTime,
    pub reason: String,
    /// When the latest delay settled.
    pub held_at: SimTime,
    /// Whether the hold re-times the legs after the held one.
    pub retime_later_legs: bool,
}

impl MovementLegHoldV1 {
    /// How far the hold moves its due: `until` minus `scheduled_at`.
    #[must_use]
    pub fn extension(&self) -> SimDuration {
        self.until
            .checked_sub(self.scheduled_at)
            .filter(|extension| !extension.is_negative())
            .unwrap_or(SimDuration::ZERO)
    }
}

/// Plugin binding of one booking to its pool and leg.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MovementBookingBindingV1 {
    pub booking: CapacityBookingId,
    pub execution: TransportExecutionId,
    pub pool: String,
    pub leg: LegExecutionId,
    pub tie_break: String,
    pub admitted_sequence: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allocation: Option<CapacityBookingAllocationEvidenceV1>,
    /// The booking of the same leg this one replaced
    /// ([`MovementBookingRequestV1::replaces`]). The replaced booking has
    /// left the runtime; its identity stays reserved while this binding is
    /// kept, so no new booking can take it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaces: Option<CapacityBookingId>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MovementOperationSourceV1 {
    Command {
        command: CommandId,
        holder: KnowledgeHolderRef,
    },
    Incident {
        ingress: IngressId,
        evidence: DomainRecordVersionRef,
    },
}

impl MovementOperationSourceV1 {
    /// The namespace in which this source's operation keys are unique.
    #[must_use]
    pub fn scope(&self) -> MovementOperationScopeV1 {
        match self {
            Self::Command { holder, .. } => MovementOperationScopeV1::Holder(holder.clone()),
            Self::Incident { evidence, .. } => {
                MovementOperationScopeV1::EvidenceKind(evidence.record.kind.clone())
            }
        }
    }

    fn conflict_suffix(&self) -> String {
        match self {
            Self::Command { command, .. } => format!("#conflict:command-{}", command.get()),
            Self::Incident { ingress, .. } => format!("#conflict:ingress-{}", ingress.get()),
        }
    }
}

/// Operation keys are unique per holder for commands and per evidence record
/// kind for incidents, so no source can claim another source's keys.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MovementOperationScopeV1 {
    Holder(KnowledgeHolderRef),
    EvidenceKind(DomainRecordKind),
}

impl MovementOperationScopeV1 {
    /// The `operation_outcomes` key of one operation key in this scope.
    pub fn outcome_key(&self, operation_key: &str) -> Result<String, CanwuError> {
        let scope = serde_json::to_string(self).map_err(|error| {
            CanwuError::new(
                ErrorCode::InvalidPayload,
                format!("movement operation scope could not be encoded: {error}"),
            )
        })?;
        Ok(format!("{scope}|{operation_key}"))
    }
}

/// The `operation_outcomes` key of a rejected conflicting reuse of an
/// operation key: the scoped key suffixed with the conflicting source.
pub(crate) fn conflict_outcome_key(scoped: &str, source: &MovementOperationSourceV1) -> String {
    format!("{scoped}{}", source.conflict_suffix())
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MovementOperationDisposition {
    Applied,
    Rejected,
}

/// Durable, idempotent result of one operation key.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MovementOperationOutcomeV1 {
    pub operation_key: String,
    pub canonical_input_hash: String,
    pub source: MovementOperationSourceV1,
    pub disposition: MovementOperationDisposition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejection_code: Option<MovementErrorCode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejection_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<TransportExecutionId>,
    pub settled_at: SimTime,
}

/// The last report published to one holder about one execution.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MovementObservationHeadV1 {
    pub holder: KnowledgeHolderRef,
    pub execution: TransportExecutionId,
    pub role: MovementObserverRole,
    pub observed_as_of: SimTime,
    pub published_at: SimTime,
    pub report_digest: String,
}

/// Root state of the movement plugin, persisted as one domain record.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MovementState {
    pub revision: u64,
    pub executions: BTreeMap<TransportExecutionId, TransportExecution>,
    pub orders: BTreeMap<MovementOrderId, MovementOrderRecordV1>,
    pub pools: BTreeMap<String, TransportCapacityPoolV1>,
    pub bookings: BTreeMap<CapacityBookingId, MovementBookingBindingV1>,
    pub operation_outcomes: BTreeMap<String, MovementOperationOutcomeV1>,
    /// Sorted by `(holder, execution)`, one head per pair.
    pub observation_heads: Vec<MovementObservationHeadV1>,
    pub next_booking_sequence: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report_wake_at: Option<SimTime>,
    /// Identities of retired orders and executions, which are never reused.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub retired_orders: BTreeSet<MovementOrderId>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub retired_executions: BTreeSet<TransportExecutionId>,
    /// Pool capacity consumed by bookings that have left the runtime: closed
    /// when their leg arrived or failed, or retired with their execution.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub retired_consumption: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MovementRuntimeRecord;

impl DomainRecordType for MovementRuntimeRecord {
    type Payload = MovementState;
    type Class = DomainValueKindClass;

    const NAMESPACE: &'static str = PLUGIN_NAMESPACE;
    const NAME: &'static str = "runtime";
}

#[must_use]
pub fn movement_runtime_reference() -> TypedDomainRecordRef<MovementRuntimeRecord> {
    TypedDomainRecordRef::new(MOVEMENT_RUNTIME_ID)
}

impl MovementState {
    /// Encodes a validated initial runtime record, for example one that
    /// installs capacity pools before the run starts.
    pub fn into_initial_record(self) -> Result<DomainRecord, CanwuError> {
        let draft = self.draft()?;
        Ok(DomainRecord {
            reference: draft.reference,
            owner: PLUGIN_NAME.to_owned(),
            class: DomainRecordClass::Record,
            version: 1,
            lifecycle: DomainRecordLifecycle::Active,
            payload: draft.payload,
            references: draft.references,
        })
    }

    pub(crate) fn draft(&self) -> Result<DomainRecordDraft, CanwuError> {
        self.validate()?;
        DomainRecordDraft::from_typed(movement_runtime_reference(), self)
    }

    /// The outcome of one operation key in one scope.
    pub fn operation_outcome(
        &self,
        scope: &MovementOperationScopeV1,
        operation_key: &str,
    ) -> Result<Option<&MovementOperationOutcomeV1>, CanwuError> {
        Ok(self
            .operation_outcomes
            .get(&scope.outcome_key(operation_key)?))
    }

    /// The order record that created an execution.
    #[must_use]
    pub fn order_for_execution(
        &self,
        execution: TransportExecutionId,
    ) -> Option<&MovementOrderRecordV1> {
        self.orders
            .values()
            .find(|order| order.execution == execution)
    }

    /// The booking a binding binds.
    pub(crate) fn booking_of(
        &self,
        binding: &MovementBookingBindingV1,
    ) -> Option<&CapacityBooking> {
        self.executions
            .get(&binding.execution)?
            .bookings
            .iter()
            .find(|booking| booking.id == binding.booking)
    }

    pub(crate) fn order_id_for_execution(
        &self,
        execution: TransportExecutionId,
    ) -> Result<MovementOrderId, MovementError> {
        self.order_for_execution(execution)
            .map(|order| order.order.id)
            .ok_or_else(|| {
                MovementError::new(
                    MovementErrorCode::NotFound,
                    "movement execution is not known",
                )
            })
    }

    /// Validates every movement invariant: order and execution pairing, leg
    /// continuity, one active itinerary revision, bookings that start no
    /// earlier than their leg's planned departure and last no longer than
    /// the leg, revocations and replacements, an execution that keeps no
    /// closed booking (a released, cancelled, or replaced one, one consumed
    /// by a leg that is no longer under way, or one lost for a leg that can
    /// no longer depart), matching pool counters, pending due work and holds
    /// (a departed leg's arrival at its scheduled or held time, a held
    /// departure at its `until`), a non-negative schedule shift, terminal
    /// closure, one active movement per subject, and bounded collections.
    pub fn validate(&self) -> Result<(), MovementError> {
        let limits = MovementLimitsV1::CURRENT;
        if self.orders.len() > limits.orders_per_runtime
            || self.pools.len() > limits.pools_per_runtime
            || self.bookings.len() > limits.bookings_per_runtime
            || self.operation_outcomes.len() > limits.operation_outcomes_per_runtime
        {
            return Err(MovementError::new(
                MovementErrorCode::LimitExceeded,
                "movement runtime exceeds its collection limits",
            ));
        }
        let mut paired = BTreeSet::new();
        let mut active_subjects = BTreeSet::new();
        for (id, record) in &self.orders {
            if record.order.id != *id {
                return Err(MovementError::state(
                    "order record is stored under another id",
                ));
            }
            if !paired.insert(record.execution) {
                return Err(MovementError::state(
                    "two orders claim the same transport execution",
                ));
            }
            let execution = self
                .executions
                .get(&record.execution)
                .ok_or_else(|| MovementError::state("order names a missing execution"))?;
            validate_order_record(record, execution, &limits)?;
            validate_execution(record, execution, &limits)?;
            if !execution.state.is_terminal() {
                for subject in &record.order.subjects {
                    if !active_subjects.insert(subject.entity.clone()) {
                        return Err(MovementError::state(
                            "a subject is part of two active movements",
                        ));
                    }
                }
            }
        }
        if paired.len() != self.executions.len() {
            return Err(MovementError::state(
                "every transport execution needs exactly one order",
            ));
        }
        if self
            .orders
            .keys()
            .any(|id| self.retired_orders.contains(id))
            || self
                .executions
                .keys()
                .any(|id| self.retired_executions.contains(id))
            || self
                .retired_consumption
                .keys()
                .any(|pool| !self.pools.contains_key(pool))
        {
            return Err(MovementError::state(
                "retired identities cannot be live and retired consumption names a pool",
            ));
        }
        self.validate_bookings()?;
        self.validate_outcomes_and_heads()
    }

    fn validate_bookings(&self) -> Result<(), MovementError> {
        let mut held: BTreeMap<&str, (u64, u64)> = BTreeMap::new();
        let mut sequences = BTreeSet::new();
        for (id, pool) in &self.pools {
            pool.validate()?;
            if pool.id != *id {
                return Err(MovementError::state(
                    "capacity pool is stored under another id",
                ));
            }
            let retired = self
                .retired_consumption
                .get(id)
                .copied()
                .unwrap_or_default();
            held.insert(id.as_str(), (0, retired));
        }
        let mut bound = 0_usize;
        for (id, binding) in &self.bookings {
            if binding.booking != *id
                || binding.admitted_sequence >= self.next_booking_sequence
                || !sequences.insert(binding.admitted_sequence)
                || !is_label(&binding.tie_break, MovementLimitsV1::CURRENT.text_bytes)
            {
                return Err(MovementError::state("booking binding identity is invalid"));
            }
            let execution = self
                .executions
                .get(&binding.execution)
                .ok_or_else(|| MovementError::state("booking binds a missing execution"))?;
            let booking = execution
                .bookings
                .iter()
                .find(|booking| booking.id == *id)
                .ok_or_else(|| MovementError::state("booking binding has no booking"))?;
            let pool = self
                .pools
                .get(&binding.pool)
                .ok_or_else(|| MovementError::state("booking binds a missing pool"))?;
            let leg = execution
                .legs
                .iter()
                .find(|leg| leg.id == binding.leg)
                .ok_or_else(|| MovementError::state("booking binds a missing leg"))?;
            // A hold can move a leg's window later, never earlier, so a
            // booking starts no earlier than its leg's planned departure and
            // is no longer than the leg.
            let route_leg = route_leg(execution, leg.itinerary_revision, leg.leg_index)?;
            if booking.resource != pool.resource
                || booking.execution != binding.execution
                || booking.valid_from < route_leg.planned_departure_at
                || booking
                    .valid_until
                    .checked_sub(booking.valid_from)
                    .is_none_or(|length| {
                        length.is_negative() || length > planned_duration(route_leg)
                    })
                || !within_time_bound(booking.valid_until)
            {
                return Err(MovementError::state(
                    "a booking must use its pool resource inside its leg window",
                ));
            }
            if (booking.status == CapacityBookingStatus::Revoked) != booking.revocation.is_some()
                || booking.revocation.as_ref().is_some_and(|revocation| {
                    !is_label(&revocation.reason, MovementLimitsV1::CURRENT.text_bytes)
                })
            {
                return Err(MovementError::state(
                    "exactly a revoked booking records its bounded revocation reason",
                ));
            }
            if !execution_keeps_booking(execution, leg, booking.status) {
                return Err(MovementError::state(
                    "an execution keeps only the bookings that still bear on its legs",
                ));
            }
            match (booking.status, &binding.allocation) {
                (CapacityBookingStatus::Requested, None) => {}
                (CapacityBookingStatus::Requested, Some(_)) | (_, None)
                    if booking.status != CapacityBookingStatus::Cancelled =>
                {
                    return Err(MovementError::state(
                        "only unallocated bookings may lack allocation evidence",
                    ));
                }
                (_, Some(evidence))
                    if evidence.booking != *id
                        || evidence.pool != binding.pool
                        || evidence.execution != binding.execution
                        || evidence.requested != booking.quantity
                        || !evidence.digest_matches() =>
                {
                    return Err(MovementError::state(
                        "booking allocation evidence does not bind this booking",
                    ));
                }
                _ => {}
            }
            let counters = held
                .get_mut(binding.pool.as_str())
                .ok_or_else(|| MovementError::state("booking binds a missing pool"))?;
            match booking.status {
                CapacityBookingStatus::Confirmed => {
                    counters.0 = counters
                        .0
                        .checked_add(booking.quantity)
                        .ok_or_else(|| MovementError::state("pool counter overflow"))?;
                }
                CapacityBookingStatus::Consumed => {
                    counters.1 = counters
                        .1
                        .checked_add(booking.quantity)
                        .ok_or_else(|| MovementError::state("pool counter overflow"))?;
                }
                _ => {}
            }
            bound += 1;
        }
        let execution_bookings = self
            .executions
            .values()
            .map(|execution| execution.bookings.len())
            .sum::<usize>();
        if bound != execution_bookings {
            return Err(MovementError::state(
                "every execution booking needs exactly one binding",
            ));
        }
        // A replaced booking leaves the runtime when it is replaced, and no
        // booking is replaced twice, so a replacement names a booking the
        // runtime no longer keeps, and only one replacement names it.
        let mut replaced = BTreeSet::new();
        for binding in self.bookings.values() {
            let Some(original) = binding.replaces else {
                continue;
            };
            if self.bookings.contains_key(&original) || !replaced.insert(original) {
                return Err(MovementError::state(
                    "a replacement names a booking that has left the runtime, once",
                ));
            }
        }
        for (id, pool) in &self.pools {
            if held.get(id.as_str()) != Some(&(pool.booked, pool.consumed)) {
                return Err(MovementError::state(
                    "capacity pool counters disagree with its bookings",
                ));
            }
        }
        Ok(())
    }

    fn validate_outcomes_and_heads(&self) -> Result<(), MovementError> {
        let text = MovementLimitsV1::CURRENT.text_bytes;
        for (key, outcome) in &self.operation_outcomes {
            let scoped = outcome
                .source
                .scope()
                .outcome_key(&outcome.operation_key)
                .map_err(|error| MovementError::state(error.message))?;
            let conflict = conflict_outcome_key(&scoped, &outcome.source);
            let is_conflict = *key == conflict;
            if (*key != scoped && !is_conflict)
                || (is_conflict && outcome.rejection_code != Some(MovementErrorCode::Conflict))
                || !is_label(&outcome.operation_key, text)
                || (outcome.disposition == MovementOperationDisposition::Rejected)
                    != outcome.rejection_code.is_some()
                || outcome.rejection_code.is_some() != outcome.rejection_message.is_some()
            {
                return Err(MovementError::state("operation outcome is malformed"));
            }
        }
        if self.observation_heads.windows(2).any(|pair| {
            (&pair[0].holder, pair[0].execution) >= (&pair[1].holder, pair[1].execution)
        }) {
            return Err(MovementError::state(
                "observation heads must be sorted and unique",
            ));
        }
        let orders = self
            .orders
            .values()
            .map(|order| (order.execution, order))
            .collect::<BTreeMap<_, _>>();
        for head in &self.observation_heads {
            let granted = orders.get(&head.execution).is_some_and(|order| {
                order
                    .observers
                    .iter()
                    .any(|grant| grant.holder == head.holder && grant.role == head.role)
            });
            if !granted || head.observed_as_of > head.published_at {
                return Err(MovementError::state(
                    "observation head has no matching grant",
                ));
            }
        }
        Ok(())
    }
}

fn validate_order_record(
    record: &MovementOrderRecordV1,
    execution: &TransportExecution,
    limits: &MovementLimitsV1,
) -> Result<(), MovementError> {
    record
        .order
        .validate()
        .map_err(|error| MovementError::state(format!("{error:?}")))?;
    if record.order.subjects.len() > limits.subjects_per_order
        || record.observers.len() > limits.observers_per_order
        || !is_label(&record.operation_key, limits.text_bytes)
    {
        return Err(MovementError::new(
            MovementErrorCode::LimitExceeded,
            "order exceeds movement limits",
        ));
    }
    if record
        .observers
        .windows(2)
        .any(|pair| pair[0].holder >= pair[1].holder)
    {
        return Err(MovementError::state(
            "observer grants must be sorted and unique by holder",
        ));
    }
    let mut owners = 0_usize;
    let mut operators = 0_usize;
    for grant in &record.observers {
        if !matches!(grant.holder, KnowledgeHolderRef::Person(_)) {
            return Err(MovementError::state("report grants name person holders"));
        }
        match grant.role {
            MovementObserverRole::Owner => {
                owners += 1;
                if grant.holder != record.owner || grant.delay != SimDuration::ZERO {
                    return Err(MovementError::state(
                        "owner grant must be the owner, undelayed",
                    ));
                }
            }
            MovementObserverRole::Operator => {
                operators += 1;
                if grant.delay != SimDuration::ZERO || grant.holder == record.owner {
                    return Err(MovementError::state(
                        "operator grant must be undelayed and differ from the owner",
                    ));
                }
            }
            MovementObserverRole::DelayedRemote => {
                if grant.delay <= SimDuration::ZERO
                    || grant.delay.as_minutes() > limits.max_report_delay_minutes
                    || grant.holder == record.owner
                {
                    return Err(MovementError::state(
                        "remote grant needs a bounded positive delay and a holder other than the owner",
                    ));
                }
            }
        }
    }
    let person_owner = matches!(record.owner, KnowledgeHolderRef::Person(_));
    if owners != usize::from(person_owner) || operators > 1 {
        return Err(MovementError::state(
            "an order has one owner grant for a person owner and at most one operator",
        ));
    }
    let terminal = execution.state.is_terminal();
    if terminal != record.closed_at.is_some()
        || record
            .closed_at
            .is_some_and(|closed| closed < record.admitted_at)
    {
        return Err(MovementError::state(
            "only a terminal execution records its closing time",
        ));
    }
    Ok(())
}

fn validate_execution(
    record: &MovementOrderRecordV1,
    execution: &TransportExecution,
    limits: &MovementLimitsV1,
) -> Result<(), MovementError> {
    if execution.revisions.len() > limits.revisions_per_execution
        || execution.legs.len() > limits.legs_per_execution
        || execution.handoffs.len() > limits.handoffs_per_execution
        || execution.bookings.len() > limits.bookings_per_execution
    {
        return Err(MovementError::new(
            MovementErrorCode::LimitExceeded,
            "execution exceeds movement limits",
        ));
    }
    // One active itinerary revision, chained from the initial one.
    let active = execution
        .active_itinerary_revision
        .ok_or_else(|| MovementError::state("execution has no active itinerary"))?;
    let mut previous: Option<ItineraryRevisionId> = None;
    let mut ids = BTreeSet::new();
    for (position, revision) in execution.revisions.iter().enumerate() {
        let last = position + 1 == execution.revisions.len();
        if revision.predecessor != previous
            || !ids.insert(revision.id)
            || last != (revision.id == active)
            || last != revision.superseded_at.is_none()
        {
            return Err(MovementError::state(
                "itinerary revisions must chain to exactly one active revision",
            ));
        }
        validate_plan(
            &revision.plan,
            if position == 0 {
                Some(&record.order.origin)
            } else {
                None
            },
            &record.order.destination,
        )?;
        let legs = execution
            .legs
            .iter()
            .filter(|leg| leg.itinerary_revision == revision.id)
            .collect::<Vec<_>>();
        if legs.len() != revision.plan.legs.len()
            || legs
                .iter()
                .enumerate()
                .any(|(index, leg)| leg.leg_index != index)
        {
            return Err(MovementError::state(
                "each revision executes exactly its planned legs in order",
            ));
        }
        previous = Some(revision.id);
    }
    let mut leg_ids = BTreeSet::new();
    if execution.legs.iter().any(|leg| !leg_ids.insert(leg.id)) {
        return Err(MovementError::state("leg identities must be unique"));
    }
    // Leg continuity of the active revision around the current leg.
    let active_legs = execution
        .legs
        .iter()
        .filter(|leg| leg.itinerary_revision == active)
        .collect::<Vec<_>>();
    if execution.current_leg_index > active_legs.len() {
        return Err(MovementError::state("current leg is beyond the itinerary"));
    }
    for leg in &active_legs {
        let ordered_ok = match leg.leg_index.cmp(&execution.current_leg_index) {
            std::cmp::Ordering::Less => leg.status == LegExecutionStatus::Arrived,
            std::cmp::Ordering::Equal => true,
            std::cmp::Ordering::Greater => matches!(
                leg.status,
                LegExecutionStatus::Planned
                    | LegExecutionStatus::Booked
                    | LegExecutionStatus::Loaded
                    | LegExecutionStatus::Waiting
                    | LegExecutionStatus::Cancelled
            ),
        };
        if !ordered_ok {
            return Err(MovementError::state(
                "legs before the current leg arrived and later legs have not started",
            ));
        }
    }
    let departed = execution
        .legs
        .iter()
        .filter(|leg| leg.status == LegExecutionStatus::Departed)
        .collect::<Vec<_>>();
    match departed.as_slice() {
        [] => {}
        [leg]
            if leg.itinerary_revision == active
                && leg.leg_index == execution.current_leg_index
                && execution.state == TransportExecutionState::Executing => {}
        _ => {
            return Err(MovementError::state(
                "only the current leg of an executing movement may be departed",
            ));
        }
    }
    // Pending due work names the current leg in the matching status.
    if let Some(due) = &record.pending_due {
        let leg = active_legs
            .iter()
            .find(|leg| leg.id == due.leg)
            .filter(|leg| leg.leg_index == execution.current_leg_index)
            .ok_or_else(|| MovementError::state("pending due work names another leg"))?;
        let matches_status = match due.kind {
            MovementLegDueKind::Departure => matches!(
                leg.status,
                LegExecutionStatus::Planned
                    | LegExecutionStatus::Booked
                    | LegExecutionStatus::Waiting
            ),
            MovementLegDueKind::Arrival => leg.status == LegExecutionStatus::Departed,
        };
        if due.execution != record.execution || due.revision != active || !matches_status {
            return Err(MovementError::state(
                "pending due work does not match its leg",
            ));
        }
    } else if !departed.is_empty() {
        return Err(MovementError::state("a departed leg needs its arrival due"));
    }
    // A hold moves the pending due of the current leg to its `until`, never
    // before the due it replaced; a departed leg without a hold arrives at
    // its scheduled arrival.
    match (&record.hold, &record.pending_due) {
        (Some(hold), Some(due)) => {
            if hold.leg != due.leg
                || hold.kind != due.kind
                || due.due_at != hold.until
                || hold.until < hold.scheduled_at
                || hold.held_at > hold.until
                || !is_label(&hold.reason, limits.text_bytes)
            {
                return Err(MovementError::state(
                    "a hold names the leg and due it moves and never ends before the due it replaced",
                ));
            }
            if hold.kind == MovementLegDueKind::Arrival {
                let departed_at = departed
                    .first()
                    .and_then(|leg| leg.actual_departure_at)
                    .ok_or_else(|| MovementError::state("a held arrival needs a departed leg"))?;
                if hold.scheduled_at != scheduled_arrival(execution, due.leg)?
                    || hold.held_at < departed_at
                {
                    return Err(MovementError::state(
                        "a held arrival replaces the arrival scheduled at departure",
                    ));
                }
            }
        }
        (Some(_), None) => {
            return Err(MovementError::state(
                "a hold needs the pending due it moves",
            ));
        }
        (None, Some(due))
            if due.kind == MovementLegDueKind::Arrival
                && due.due_at != scheduled_arrival(execution, due.leg)? =>
        {
            return Err(MovementError::state(
                "a departed leg's arrival due is its scheduled or held arrival",
            ));
        }
        (None, _) => {}
    }
    if record.schedule_shift.is_negative()
        || record.schedule_shift.as_minutes() > limits.time_bound_minutes
    {
        return Err(MovementError::state(
            "a schedule shift is a bounded delay, never an advance",
        ));
    }
    // Terminal closure.
    if execution.state.is_terminal() {
        if record.pending_due.is_some()
            || !departed.is_empty()
            || execution.bookings.iter().any(|booking| {
                matches!(
                    booking.status,
                    CapacityBookingStatus::Requested | CapacityBookingStatus::Confirmed
                )
            })
        {
            return Err(MovementError::state(
                "a terminal execution holds no due work, departed leg, or live booking",
            ));
        }
        if execution.state == TransportExecutionState::Settled
            && execution.current_leg_index != active_legs.len()
        {
            return Err(MovementError::state(
                "a settled execution arrived on every active leg",
            ));
        }
    }
    if execution.state == TransportExecutionState::ArrivalPending
        && (execution.current_leg_index != active_legs.len() || execution.saga.is_none())
    {
        return Err(MovementError::state(
            "an arrival-pending execution arrived on every leg and carries a saga",
        ));
    }
    if execution.delivery_attempt.is_some() != execution.saga.is_some() {
        return Err(MovementError::state(
            "a movement carries a saga exactly when it carries a delivery attempt",
        ));
    }
    Ok(())
}

/// Checks that a plan chains its legs from its origin to its destination.
pub(crate) fn validate_plan(
    plan: &RoutePlan,
    origin: Option<&RoutingNodeRef>,
    destination: &RoutingNodeRef,
) -> Result<(), MovementError> {
    let (Some(first), Some(last)) = (plan.legs.first(), plan.legs.last()) else {
        return Err(MovementError::invalid(
            "a movement plan needs at least one leg",
        ));
    };
    if first.from != plan.origin
        || last.to != plan.destination
        || plan.destination != *destination
        || origin.is_some_and(|origin| plan.origin != *origin)
        || plan.legs.windows(2).any(|pair| pair[0].to != pair[1].from)
        || plan
            .legs
            .iter()
            .any(|leg| leg.planned_arrival_at < leg.planned_departure_at)
    {
        return Err(MovementError::invalid(
            "movement plan legs must chain from its origin to the order destination",
        ));
    }
    let times = [plan.departure_at, plan.estimated_arrival_at]
        .into_iter()
        .chain(
            plan.legs
                .iter()
                .flat_map(|leg| [leg.planned_departure_at, leg.planned_arrival_at]),
        );
    if times.into_iter().any(|time| !within_time_bound(time)) {
        return Err(MovementError::new(
            MovementErrorCode::LimitExceeded,
            "movement plan times exceed the supported time bound",
        ));
    }
    Ok(())
}

/// Whether a time lies within [`MovementLimitsV1::time_bound_minutes`] of the
/// epoch.
pub(crate) fn within_time_bound(time: SimTime) -> bool {
    time.as_minutes().unsigned_abs() <= MovementLimitsV1::CURRENT.time_bound_minutes.unsigned_abs()
}

pub(crate) fn route_leg(
    execution: &TransportExecution,
    revision: ItineraryRevisionId,
    index: usize,
) -> Result<&canwu_api::RouteLeg, MovementError> {
    execution
        .revisions
        .iter()
        .find(|item| item.id == revision)
        .and_then(|item| item.plan.legs.get(index))
        .ok_or_else(|| MovementError::state("leg has no planned route leg"))
}

/// How long a planned leg takes once it departs.
pub(crate) fn planned_duration(leg: &canwu_api::RouteLeg) -> SimDuration {
    leg.planned_arrival_at
        .checked_sub(leg.planned_departure_at)
        .unwrap_or(SimDuration::ZERO)
}

/// The arrival a departed leg was scheduled for when it departed: its actual
/// departure plus its planned duration.
pub(crate) fn scheduled_arrival(
    execution: &TransportExecution,
    leg: LegExecutionId,
) -> Result<SimTime, MovementError> {
    let leg = execution
        .legs
        .iter()
        .find(|item| item.id == leg)
        .ok_or_else(|| MovementError::state("leg disappeared"))?;
    let departed_at = leg
        .actual_departure_at
        .ok_or_else(|| MovementError::state("the leg has not departed"))?;
    departed_at
        .checked_add(planned_duration(route_leg(
            execution,
            leg.itinerary_revision,
            leg.leg_index,
        )?))
        .ok_or_else(|| MovementError::invalid("leg arrival time overflows"))
}

/// Whether an execution keeps a booking of `leg` in `status`: one that asks
/// for or holds capacity (`Requested`, `Confirmed`), one the leg under way
/// consumed, and one lost (failed, expired, or revoked) for a leg of the
/// active revision that has not departed, whose departure it fails unless it
/// is replaced. Any other booking is closed and leaves the execution
/// (downstream gap G-83): a released or cancelled one, one consumed by a leg
/// that arrived or failed, and one lost for a leg that failed, belongs to a
/// superseded revision, or whose execution closed. A replaced booking leaves
/// when it is replaced, whatever its status.
pub(crate) fn execution_keeps_booking(
    execution: &TransportExecution,
    leg: &LegExecution,
    status: CapacityBookingStatus,
) -> bool {
    match status {
        CapacityBookingStatus::Requested | CapacityBookingStatus::Confirmed => true,
        CapacityBookingStatus::Consumed => leg.status == LegExecutionStatus::Departed,
        CapacityBookingStatus::Failed
        | CapacityBookingStatus::Expired
        | CapacityBookingStatus::Revoked => {
            !execution.state.is_terminal()
                && execution.active_itinerary_revision == Some(leg.itinerary_revision)
                && matches!(
                    leg.status,
                    LegExecutionStatus::Planned
                        | LegExecutionStatus::Booked
                        | LegExecutionStatus::Loaded
                        | LegExecutionStatus::Waiting
                )
        }
        CapacityBookingStatus::Released | CapacityBookingStatus::Cancelled => false,
    }
}

pub(crate) fn is_label(value: &str, limit: usize) -> bool {
    !value.is_empty() && value.trim() == value && value.len() <= limit
}
