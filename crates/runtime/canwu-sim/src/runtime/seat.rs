//! Seat rebinding: moving the run's seat to another existing person while the
//! run continues.
//!
//! A run configuration names its seat binding once, when the run starts, and
//! the run manifest binds that configuration, so it never changes. A seat
//! rebinding is canonical ingress that a boundary admits: from the start of
//! that boundary the rebound binding is the one in force, and the human-only
//! gates, the actor-bound viewer, and the seat's open tickets follow it. The
//! binding in force is persisted beside the configuration, and the
//! checkpoint hash binds it.

use super::persons::{AvailabilityLookup, LifeState, authority_person, cancel_open_tickets};
use super::{
    CanwuError, DecisionState, ErrorCode, IngressClass, IngressId, IngressPayload, IngressReceipt,
    InteractionPolicy, RunConfiguration, RunConfigurationSnapshot, SeatBinding, Simulation,
    canonical_text,
};
use canwu_core::{DecisionTicketId, EntityRef, PersonId};
use canwu_time::SimTime;
use serde::{Deserialize, Serialize};

/// Cancellation reason of an open ticket that a seat rebinding closed: its
/// assigned controller belonged to the seat and acted for the person the
/// seat no longer binds.
pub const SEAT_REBOUND_REASON: &str = "seat_rebound";

/// A request to bind the run's seat to another existing person from the
/// next boundary at the current instant.
///
/// The host queues it with [`Simulation::rebind_seat`]. The boundary that
/// admits it decides it before any other ingress it admits, so commands
/// admitted in the same boundary already meet the new binding. The seat's
/// identity and institution never change; the controller and permission
/// profile change only when the request names new ones, as a successor whose
/// decision controller is registered under another ID needs.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SeatRebindingRequest {
    /// The seat of the run's binding; a run has one seat.
    pub seat_id: String,
    /// The existing, living person the seat binds from its boundary. Custody
    /// does not matter: a detained or captive person may hold the seat, and
    /// the command gates refuse their commands until they are free.
    pub actor: PersonId,
    /// The controller ID of the new binding; `None` keeps the current one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controller_id: Option<String>,
    /// The permission profile of the new binding; `None` keeps the current
    /// one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_profile_id: Option<String>,
    /// The run revision the host read; the admitting boundary refuses the
    /// request when its revision differs.
    pub expected_revision: u64,
    /// The current simulation time; the request is due at this instant.
    pub expected_time: SimTime,
}

impl SeatRebindingRequest {
    /// A rebinding of `seat_id` to `actor` that keeps the controller and
    /// permission profile.
    #[must_use]
    pub fn new(
        seat_id: impl Into<String>,
        actor: PersonId,
        expected_revision: u64,
        expected_time: SimTime,
    ) -> Self {
        Self {
            seat_id: seat_id.into(),
            actor,
            controller_id: None,
            permission_profile_id: None,
            expected_revision,
            expected_time,
        }
    }

    /// Names the controller of the new binding.
    #[must_use]
    pub fn with_controller(mut self, controller_id: impl Into<String>) -> Self {
        self.controller_id = Some(controller_id.into());
        self
    }

    /// Names the permission profile of the new binding.
    #[must_use]
    pub fn with_permission_profile(mut self, permission_profile_id: impl Into<String>) -> Self {
        self.permission_profile_id = Some(permission_profile_id.into());
        self
    }
}

/// One seat rebinding a boundary admitted, recorded in admission order.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BoundarySeatRebinding {
    /// The admitted seat-rebinding ingress record.
    pub ingress: IngressId,
    pub seat_id: String,
    pub outcome: SeatRebindingOutcome,
}

/// What admitting a seat rebinding did.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SeatRebindingOutcome {
    /// The seat binds `binding` from this boundary. `cancelled_tickets` are
    /// the open tickets, in ticket-ID order, whose assigned controller
    /// belonged to the seat and acted for the person the seat no longer
    /// binds; each was cancelled with [`SEAT_REBOUND_REASON`].
    Applied {
        previous: Box<SeatBinding>,
        binding: Box<SeatBinding>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        cancelled_tickets: Vec<DecisionTicketId>,
    },
    /// The rebinding was refused and changed nothing: an unknown person
    /// ([`ErrorCode::ActorNotFound`]), a dead or missing one
    /// ([`ErrorCode::IssuerUnavailable`]), a stale revision
    /// ([`ErrorCode::SimulationRevisionConflict`]), or a binding already in
    /// force ([`ErrorCode::InvalidPayload`]).
    Rejected { error: CanwuError },
}

/// The binding a rebinding would put in force over `current`.
fn rebound_binding(current: &SeatBinding, request: &SeatRebindingRequest) -> SeatBinding {
    SeatBinding {
        seat_id: current.seat_id.clone(),
        controller_id: request
            .controller_id
            .clone()
            .unwrap_or_else(|| current.controller_id.clone()),
        actor: Some(request.actor),
        institution: current.institution.clone(),
        permission_profile_id: request
            .permission_profile_id
            .clone()
            .unwrap_or_else(|| current.permission_profile_id.clone()),
    }
}

/// `configuration` with `binding` in place of its own seat binding.
pub(super) fn configuration_with_binding(
    configuration: &RunConfiguration,
    binding: &SeatBinding,
) -> RunConfiguration {
    RunConfiguration {
        seat_binding: Some(binding.clone()),
        ..configuration.clone()
    }
}

/// The checks a seat rebinding request passes before it is queued, and that
/// every persisted seat-rebinding ingress record passes on load: a declared
/// run that accepts authored ingress, a seat binding with the request's seat,
/// canonical controller and permission-profile texts, and a configuration
/// that stays valid with the rebound binding.
pub(super) fn validate_seat_rebinding_shape(
    run_configuration: &RunConfigurationSnapshot,
    request: &SeatRebindingRequest,
) -> Result<(), CanwuError> {
    let Some(configuration) = run_configuration.declared() else {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "seat rebinding requires a declared run configuration",
        ));
    };
    if configuration.interaction == InteractionPolicy::ReadOnly {
        return Err(CanwuError::new(
            ErrorCode::InteractionReadOnly,
            "the run interaction policy rejects a seat rebinding",
        ));
    }
    let Some(binding) = configuration.seat_binding.as_ref() else {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "seat rebinding requires a run with a seat binding",
        ));
    };
    if request.seat_id != binding.seat_id {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            format!(
                "seat rebinding names seat {}, but the run binds seat {}",
                request.seat_id, binding.seat_id
            ),
        ));
    }
    if binding.actor.is_none() {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            format!(
                "seat {} binds an institution without a person, so it cannot be rebound",
                binding.seat_id
            ),
        ));
    }
    if request
        .controller_id
        .as_deref()
        .is_some_and(|value| !canonical_text(value))
        || request
            .permission_profile_id
            .as_deref()
            .is_some_and(|value| !canonical_text(value))
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidPayload,
            "seat rebinding controller and permission-profile IDs must be canonical",
        ));
    }
    configuration_with_binding(configuration, &rebound_binding(binding, request))
        .validate()
        .map_err(|error| {
            CanwuError::new(
                ErrorCode::InvalidPayload,
                format!(
                    "seat rebinding would leave an invalid run configuration: {}",
                    error.message
                ),
            )
        })
}

/// What admitting one seat rebinding does, decided from the state committed
/// at the start of its boundary: the binding in force, the revision, the
/// person registry, and person availability. Shared by settlement and
/// snapshot reconstruction. `Err` is the recorded rejection.
pub(super) fn admit_seat_rebinding<A: AvailabilityLookup + ?Sized>(
    current: Option<&SeatBinding>,
    request: &SeatRebindingRequest,
    revision: u64,
    person_exists: &dyn Fn(PersonId) -> bool,
    availability: &A,
) -> Result<SeatBinding, CanwuError> {
    let Some(current) = current.filter(|binding| binding.seat_id == request.seat_id) else {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            format!("seat {} is not bound in this run", request.seat_id),
        ));
    };
    if request.expected_revision != revision {
        return Err(CanwuError::new(
            ErrorCode::SimulationRevisionConflict,
            format!(
                "seat rebinding expected revision {}, current revision is {revision}",
                request.expected_revision
            ),
        ));
    }
    let person = EntityRef::Person(request.actor);
    if !person_exists(request.actor) {
        return Err(CanwuError::new(
            ErrorCode::ActorNotFound,
            format!("seat rebinding names unknown person {}", request.actor),
        )
        .with_entity(person));
    }
    let alive = availability
        .availability_of(request.actor)
        .is_none_or(|state| state.life == LifeState::Alive);
    if !alive {
        return Err(CanwuError::new(
            ErrorCode::IssuerUnavailable,
            format!(
                "seat rebinding names person {}, who is dead or missing",
                request.actor
            ),
        )
        .with_entity(person));
    }
    let binding = rebound_binding(current, request);
    if binding == *current {
        return Err(CanwuError::new(
            ErrorCode::InvalidPayload,
            format!(
                "seat {} already binds this person, controller, and permission profile",
                current.seat_id
            ),
        ));
    }
    Ok(binding)
}

/// Cancels, in ticket-ID order, the open tickets whose assigned controller
/// belongs to the seat and acts for the person `previous` bound, when the
/// rebinding moves the seat to another person.
pub(super) fn cancel_rebound_seat_tickets(
    decisions: &mut DecisionState,
    previous: &SeatBinding,
    binding: &SeatBinding,
    at: SimTime,
) -> Result<Vec<DecisionTicketId>, CanwuError> {
    let Some(person) = previous
        .actor
        .filter(|person| binding.actor != Some(*person))
    else {
        return Ok(Vec::new());
    };
    cancel_open_tickets(
        decisions,
        |decisions, ticket| {
            decisions
                .controller(&ticket.assigned_controller)
                .is_some_and(|controller| {
                    controller.seat_id.as_deref() == Some(previous.seat_id.as_str())
                        && authority_person(&controller.authority) == Some(person)
                })
        },
        SEAT_REBOUND_REASON,
        at,
    )
}

/// The persisted rebound binding: `None` while the configuration's own
/// binding is in force.
pub(super) fn normalized_rebound_seat(
    run_configuration: &RunConfigurationSnapshot,
    binding: SeatBinding,
) -> Option<SeatBinding> {
    let configured = run_configuration
        .declared()
        .and_then(|configuration| configuration.seat_binding.as_ref());
    (configured != Some(&binding)).then_some(binding)
}

/// Checks a persisted rebound binding without history: a declared run that
/// accepts authored ingress and binds a person to its seat, a binding of the
/// configured seat and institution that differs from the configured binding,
/// binds an existing person, and keeps the configuration valid.
pub(super) fn validate_rebound_seat_shape(
    run_configuration: Option<&RunConfigurationSnapshot>,
    rebound: &SeatBinding,
    person_exists: &dyn Fn(PersonId) -> bool,
) -> Result<(), CanwuError> {
    let invalid = || {
        Err(CanwuError::new(
            ErrorCode::InvalidSnapshot,
            "the persisted rebound seat binding is not a valid rebinding of the run's seat",
        ))
    };
    let Some(configuration) = run_configuration.and_then(RunConfigurationSnapshot::declared) else {
        return invalid();
    };
    let Some(configured) = configuration.seat_binding.as_ref() else {
        return invalid();
    };
    let (Some(actor), Some(_)) = (rebound.actor, configured.actor) else {
        return invalid();
    };
    if configuration.interaction == InteractionPolicy::ReadOnly
        || rebound == configured
        || rebound.seat_id != configured.seat_id
        || rebound.institution != configured.institution
        || !canonical_text(&rebound.controller_id)
        || !canonical_text(&rebound.permission_profile_id)
        || !person_exists(actor)
        || configuration_with_binding(configuration, rebound)
            .validate()
            .is_err()
    {
        return invalid();
    }
    Ok(())
}

/// The seat binding in force over a full snapshot's boundaries, rebuilt
/// boundary by boundary from the configuration's binding and the applied
/// rebindings.
pub(super) struct SeatHistory {
    configured: Option<SeatBinding>,
    /// Each applied rebinding as the number of boundaries after which it is
    /// in force (its boundary's index plus one) and the binding, in order.
    changes: Vec<(usize, SeatBinding)>,
    /// Whether the history was rebuilt from boundary evidence. A full
    /// snapshot without ingress or boundaries has none to rebuild: it is at
    /// genesis or holds direct command history, and neither ever rebinds.
    derived: bool,
}

impl SeatHistory {
    pub(super) fn new(snapshot: &super::SimulationSnapshot, derived: bool) -> Self {
        Self {
            configured: snapshot
                .run_configuration
                .as_ref()
                .and_then(RunConfigurationSnapshot::declared)
                .and_then(|configuration| configuration.seat_binding.clone()),
            changes: Vec::new(),
            derived,
        }
    }

    /// The binding in force after the first `boundaries` boundaries.
    pub(super) fn binding_after(&self, boundaries: usize) -> Option<&SeatBinding> {
        self.changes
            .iter()
            .take_while(|(after, _)| *after <= boundaries)
            .last()
            .map(|(_, binding)| binding)
            .or(self.configured.as_ref())
    }

    /// The binding in force now, after every boundary rebuilt so far.
    pub(super) fn current(&self) -> Option<&SeatBinding> {
        self.changes
            .last()
            .map(|(_, binding)| binding)
            .or(self.configured.as_ref())
    }

    pub(super) fn record(&mut self, after_boundaries: usize, binding: SeatBinding) {
        self.changes.push((after_boundaries, binding));
    }

    /// Checks the snapshot's persisted rebound binding against the history:
    /// a rebuilt history must end at it, and a full snapshot without ingress
    /// or boundaries never admitted a rebinding, so it holds none.
    pub(super) fn check_persisted(
        &self,
        snapshot: &super::SimulationSnapshot,
    ) -> Result<(), CanwuError> {
        let consistent = if self.derived {
            let expected = self.current().cloned().and_then(|binding| {
                snapshot
                    .run_configuration
                    .as_ref()
                    .and_then(|configuration| normalized_rebound_seat(configuration, binding))
            });
            expected == snapshot.rebound_seat
        } else {
            snapshot.rebound_seat.is_none()
        };
        if consistent {
            Ok(())
        } else {
            Err(CanwuError::new(
                ErrorCode::InvalidSnapshot,
                "the persisted rebound seat binding does not follow the run's seat rebindings",
            ))
        }
    }
}

impl Simulation {
    /// The seat binding in force: the binding of the last applied seat
    /// rebinding, or else the run configuration's own binding. `None` for a
    /// run without a declared seat binding.
    #[must_use]
    pub fn seat_binding(&self) -> Option<&SeatBinding> {
        self.state.metadata.rebound_seat.as_deref().or_else(|| {
            self.state
                .metadata
                .run_configuration
                .declared()
                .and_then(|configuration| configuration.seat_binding.as_ref())
        })
    }

    /// Queues a rebinding of the run's seat to another existing person at the
    /// current instant.
    ///
    /// The next boundary admits it before any other ingress it admits and
    /// records the outcome in [`super::BoundaryRecord::seat_rebindings`] and
    /// [`super::BoundaryReceipt::seat_rebindings`]. An applied rebinding puts
    /// the new binding in force from that boundary: human commands must then
    /// match it, the actor-bound viewer follows it, and the seat's open
    /// tickets that acted for the previous person are cancelled with
    /// [`SEAT_REBOUND_REASON`]. A rebinding to an unknown, dead, or missing
    /// person, at a stale revision, or to the binding already in force is a
    /// recorded rejection that changes nothing else; a detained or captive
    /// person may hold the seat.
    ///
    /// The call itself fails, queuing nothing, before the runtime is ready;
    /// after direct command history ([`ErrorCode::MixedCommandIngress`]); in a
    /// read-only run ([`ErrorCode::InteractionReadOnly`]); in a run without a
    /// declared seat binding, for another seat, or for a seat bound to an
    /// institution without a person ([`ErrorCode::InvalidAuthority`]); for
    /// non-canonical controller or permission-profile IDs
    /// ([`ErrorCode::InvalidPayload`]); while another rebinding of the seat is
    /// queued ([`ErrorCode::IdempotencyConflict`]);
    /// and for an expected time or revision that is not the current one
    /// ([`ErrorCode::SimulationTimeConflict`],
    /// [`ErrorCode::SimulationRevisionConflict`]). Repeating a queued request
    /// returns its receipt.
    pub fn rebind_seat(
        &mut self,
        request: SeatRebindingRequest,
    ) -> Result<IngressReceipt, CanwuError> {
        self.ensure_runtime_ready()?;
        self.ensure_canonical_ingress_can_start()?;
        validate_seat_rebinding_shape(&self.state.metadata.run_configuration, &request)?;
        for key in &self.state.scheduler.pending_ingress {
            let Some(record) = self.state.evidence.ingress_record(key.id) else {
                continue;
            };
            let IngressPayload::SeatRebinding { request: queued } = &record.payload else {
                continue;
            };
            if queued.as_ref() == &request {
                return Ok(IngressReceipt {
                    ingress_id: record.id,
                    issued_at: record.issued_at,
                    due_at: record.due_at,
                });
            }
            return Err(CanwuError::new(
                ErrorCode::IdempotencyConflict,
                format!(
                    "a rebinding of seat {} is already queued as ingress {}",
                    queued.seat_id, record.id
                ),
            ));
        }
        if request.expected_time != self.state.scheduler.now {
            return Err(CanwuError::new(
                ErrorCode::SimulationTimeConflict,
                format!(
                    "seat rebinding expected time {}, but simulation is at {}",
                    request.expected_time, self.state.scheduler.now
                ),
            ));
        }
        if request.expected_revision != self.revision() {
            return Err(CanwuError::new(
                ErrorCode::SimulationRevisionConflict,
                format!(
                    "seat rebinding expected revision {}, current revision is {}",
                    request.expected_revision,
                    self.revision()
                ),
            ));
        }
        self.append_ingress(
            request.expected_time,
            IngressClass::Command,
            0,
            IngressPayload::SeatRebinding {
                request: Box::new(request),
            },
            None,
            false,
        )
    }

    /// Decides, in admission order and before any other admitted ingress,
    /// the seat rebindings a boundary admits.
    pub(super) fn apply_seat_rebindings(
        &mut self,
        admitted: &[IngressId],
    ) -> Result<Vec<BoundarySeatRebinding>, CanwuError> {
        let mut evidence = Vec::new();
        for ingress_id in admitted {
            let Some(record) = self.state.evidence.ingress_record(*ingress_id) else {
                return Err(CanwuError::new(
                    ErrorCode::InvalidSnapshot,
                    "pending ingress references an unknown record",
                ));
            };
            let IngressPayload::SeatRebinding { request } = &record.payload else {
                continue;
            };
            let request = request.as_ref().clone();
            let current = self.seat_binding().cloned();
            let decided = admit_seat_rebinding(
                current.as_ref(),
                &request,
                self.revision(),
                &|person| {
                    self.state
                        .current
                        .entities
                        .contains(&EntityRef::Person(person))
                },
                &self.state.current.person_availability,
            );
            let outcome = match (decided, current) {
                (Ok(binding), Some(previous)) => {
                    let mut decisions = self.state.current.decisions.clone();
                    let cancelled_tickets = cancel_rebound_seat_tickets(
                        &mut decisions,
                        &previous,
                        &binding,
                        self.state.scheduler.now,
                    )?;
                    if !cancelled_tickets.is_empty() {
                        self.invalidate_commitments(super::CommitmentDomains::DECISIONS);
                        self.state.current.decisions = decisions;
                    }
                    self.state.metadata.rebound_seat = normalized_rebound_seat(
                        &self.state.metadata.run_configuration,
                        binding.clone(),
                    )
                    .map(std::sync::Arc::new);
                    SeatRebindingOutcome::Applied {
                        previous: Box::new(previous),
                        binding: Box::new(binding),
                        cancelled_tickets,
                    }
                }
                (Err(error), _) => SeatRebindingOutcome::Rejected { error },
                (Ok(_), None) => {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidSnapshot,
                        "an applied seat rebinding requires a bound seat",
                    ));
                }
            };
            evidence.push(BoundarySeatRebinding {
                ingress: *ingress_id,
                seat_id: request.seat_id,
                outcome,
            });
        }
        Ok(evidence)
    }
}
