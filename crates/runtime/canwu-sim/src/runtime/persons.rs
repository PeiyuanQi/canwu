//! Core person availability and runtime person creation.
//!
//! Availability is a separate ordered core map keyed by [`PersonId`]. It is
//! deliberately not a field of the legacy [`Person`] projection, so the map
//! applies to every person entity and survives a later world-model move.

use super::{
    BoundaryPhase, BoundaryRecord, BoundarySystemContract, CanwuError, CommandAuthority,
    DecisionAttemptErrorCode, DecisionAuthority, DecisionControllerBinding, DecisionMutation,
    DecisionOrigin, DecisionState, DecisionTicket, DecisionTicketId, EntityRef, ErrorCode,
    EvidenceRef, GovernmentId, Issuer, Person, PersonId, PluginRegistry, SimTime, Simulation,
    SimulationSnapshot, StateKey, StateVisibility, TerritoryId, canonical_text, claim_counter,
    invalid_snapshot, invalid_snapshot_error, runtime_entity_exists,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Cancellation reason recorded on an open ticket whose person decision maker
/// became unavailable.
///
/// A successor holder continues the decision as a new ticket of its own. When
/// the successor's controller is bound to the same seat as the cancelled
/// ticket's controller, the new ticket may name the cancelled one as its
/// `parent_ticket`.
pub const DECISION_MAKER_UNAVAILABLE_REASON: &str = "decision_maker_unavailable";

/// Cancellation reason recorded on an open ticket whose assigned controller's
/// authority person became unavailable.
///
/// The authority person is the actor of [`DecisionAuthority::Actor`], or the
/// responsible actor of [`DecisionAuthority::Institution`] when one is named.
/// Council and no-responsible-actor authorities never trigger this reason.
/// The cancellation happens at the end of the boundary that makes the person
/// unavailable, after the boundary's random decisions are materialized, and
/// the IDs are recorded in
/// [`BoundaryPersonAvailabilityChange::cancelled_controller_tickets`]. A
/// ticket whose decision maker became unavailable in the same boundary is
/// cancelled with [`DECISION_MAKER_UNAVAILABLE_REASON`] instead.
///
/// A ticket cannot be reassigned to another controller, and decision ingress
/// refuses to open a ticket for a controller whose authority person is
/// unavailable ([`ErrorCode::IssuerUnavailable`]). To continue the decision,
/// open a successor ticket for a different, available controller with
/// `parent_ticket` naming the cancelled ticket.
pub const CONTROLLER_AUTHORITY_UNAVAILABLE_REASON: &str = "controller_authority_unavailable";

const MAX_PERSON_CORRELATION_BYTES: usize = 256;

/// Whether a person is alive. Absent availability means [`LifeState::Alive`].
#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum LifeState {
    #[default]
    Alive,
    Dead,
    Missing,
}

/// Whether a person is free to act. Absent availability means
/// [`CustodyState::Free`].
#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum CustodyState {
    #[default]
    Free,
    Detained,
    Hostage,
    Captive,
    Hiding,
    Exile,
}

/// Core life and custody state of one person.
///
/// A person is unavailable for command issuance and decision making when it
/// is not alive or is detained or captive. Hostage, hiding, and exile remain
/// admissible by default; applications may restrict them further.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PersonAvailability {
    pub life: LifeState,
    pub custody: CustodyState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custodian: Option<EntityRef>,
    pub since: SimTime,
}

impl PersonAvailability {
    #[must_use]
    pub const fn new(life: LifeState, custody: CustodyState, since: SimTime) -> Self {
        Self {
            life,
            custody,
            custodian: None,
            since,
        }
    }

    #[must_use]
    pub fn with_custodian(mut self, custodian: EntityRef) -> Self {
        self.custodian = Some(custodian);
        self
    }

    /// Returns whether the person may issue commands or make decisions.
    #[must_use]
    pub const fn is_available(&self) -> bool {
        matches!(self.life, LifeState::Alive)
            && !matches!(self.custody, CustodyState::Detained | CustodyState::Captive)
    }
}

/// Application-supplied content for a person created at a boundary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PersonDraft {
    pub name: String,
    pub government: GovernmentId,
    pub current_location: TerritoryId,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<String>,
    pub availability: PersonAvailability,
    /// Committed evidence that justifies the creation.
    pub provenance: EvidenceRef,
}

/// Committed availability change evidence in a boundary record.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BoundaryPersonAvailabilityChange {
    pub plugin: String,
    pub system: String,
    pub phase: BoundaryPhase,
    pub person: PersonId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<PersonAvailability>,
    pub availability: PersonAvailability,
    pub visibility: StateVisibility,
    pub summary: String,
    /// Open tickets closed in the same boundary because this person, their
    /// decision maker, became unavailable, in ticket-ID order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cancelled_tickets: Vec<DecisionTicketId>,
    /// Open tickets closed in the same boundary with
    /// [`CONTROLLER_AUTHORITY_UNAVAILABLE_REASON`] because this person, the
    /// authority person of their assigned controller, became unavailable, in
    /// ticket-ID order. A ticket closed for its decision maker by any change
    /// of the boundary is listed only in that change's `cancelled_tickets`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cancelled_controller_tickets: Vec<DecisionTicketId>,
}

/// Committed person-creation evidence in a boundary record.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BoundaryPersonCreation {
    pub plugin: String,
    pub system: String,
    pub correlation: String,
    pub person: Person,
    pub availability: PersonAvailability,
    pub provenance: EvidenceRef,
    pub summary: String,
}

/// Receipt entry binding a creation correlation to its engine-allocated ID.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CreatedPerson {
    pub plugin: String,
    pub system: String,
    pub correlation: String,
    pub person: PersonId,
}

impl From<&BoundaryPersonCreation> for CreatedPerson {
    fn from(value: &BoundaryPersonCreation) -> Self {
        Self {
            plugin: value.plugin.clone(),
            system: value.system.clone(),
            correlation: value.correlation.clone(),
            person: value.person.id,
        }
    }
}

pub(super) fn person_is_available(
    availability: &BTreeMap<PersonId, PersonAvailability>,
    person: PersonId,
) -> bool {
    availability
        .get(&person)
        .is_none_or(PersonAvailability::is_available)
}

/// Returns the plugin-owned subset of a boundary contract's writes after
/// checking the kernel-guarded core keys a boundary system may declare.
///
/// `canwu.core.person_availability` is writable in phases 7 and 10,
/// `canwu.core.people` (person creation) in phase 7, and
/// `canwu.core.transitions` (transition manifests) in phases 7, 10, and 12.
/// None is owned by a plugin, so several systems may declare them. Per-person
/// conflicts fail the boundary at settlement; a repeated pending lineage or an
/// exceeded pending bound fails it when the registration is admitted.
pub(super) fn plugin_owned_boundary_writes(
    contract: &BoundarySystemContract,
) -> Result<Vec<StateKey>, CanwuError> {
    let availability = StateKey::core_person_availability();
    let people = StateKey::core_people();
    let transitions = StateKey::core_transitions();
    let mut owned = Vec::with_capacity(contract.writes.len());
    for key in &contract.writes {
        let allowed = if *key == availability {
            matches!(
                contract.phase,
                BoundaryPhase::DomainDeltaProposal | BoundaryPhase::HistoricalCandidateEvaluation
            )
        } else if *key == people {
            contract.phase == BoundaryPhase::DomainDeltaProposal
        } else if *key == transitions {
            matches!(
                contract.phase,
                BoundaryPhase::DomainDeltaProposal
                    | BoundaryPhase::HistoricalCandidateEvaluation
                    | BoundaryPhase::StrategicAggregation
            )
        } else {
            owned.push(key.clone());
            continue;
        };
        if !allowed {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                format!(
                    "boundary system {} cannot write core state {}.{} in phase {:?}",
                    contract.name, key.namespace, key.name, contract.phase
                ),
            ));
        }
    }
    Ok(owned)
}

pub(super) fn validate_availability_value(
    person: PersonId,
    availability: &PersonAvailability,
    at: SimTime,
    entity_exists: &dyn Fn(&EntityRef) -> bool,
) -> Result<(), CanwuError> {
    if availability.since > at {
        return Err(CanwuError::new(
            ErrorCode::InvalidBoundary,
            format!("availability of person {person} cannot start after its boundary time"),
        ));
    }
    if let Some(custodian) = &availability.custodian {
        if availability.custody == CustodyState::Free || *custodian == EntityRef::Person(person) {
            return Err(CanwuError::new(
                ErrorCode::InvalidBoundary,
                format!("person {person} names an invalid custodian"),
            ));
        }
        if !entity_exists(custodian) {
            return Err(CanwuError::new(
                ErrorCode::EntityNotFound,
                format!("person {person} names missing custodian {custodian}"),
            )
            .with_entity(custodian.clone()));
        }
    }
    Ok(())
}

fn require_core_write(
    plugin: &str,
    contract: &BoundarySystemContract,
    key: &StateKey,
    phases: &[BoundaryPhase],
) -> Result<(), CanwuError> {
    if !contract.writes.contains(key) || !phases.contains(&contract.phase) {
        return Err(CanwuError::new(
            ErrorCode::UndeclaredStateWrite,
            format!(
                "boundary system {plugin}.{} did not declare core write {}.{}",
                contract.name, key.namespace, key.name
            ),
        ));
    }
    Ok(())
}

pub(super) fn validate_availability_directive(
    plugin: &str,
    contract: &BoundarySystemContract,
    now: SimTime,
    person: PersonId,
    availability: &PersonAvailability,
    summary: &str,
    entity_exists: &dyn Fn(&EntityRef) -> bool,
) -> Result<(), CanwuError> {
    require_core_write(
        plugin,
        contract,
        &StateKey::core_person_availability(),
        &[
            BoundaryPhase::DomainDeltaProposal,
            BoundaryPhase::HistoricalCandidateEvaluation,
        ],
    )?;
    if !canonical_text(summary) {
        return Err(CanwuError::new(
            ErrorCode::InvalidBoundary,
            "person availability summaries must be canonical text",
        ));
    }
    let entity = EntityRef::Person(person);
    if !entity_exists(&entity) {
        return Err(CanwuError::new(
            ErrorCode::EntityNotFound,
            format!(
                "boundary system {plugin}.{} set availability of missing person {person}",
                contract.name
            ),
        )
        .with_entity(entity));
    }
    validate_availability_value(person, availability, now, entity_exists)
}

pub(super) struct PersonDraftContext<'a> {
    pub(super) plugin: &'a str,
    pub(super) contract: &'a BoundarySystemContract,
    pub(super) now: SimTime,
    pub(super) government_exists: &'a dyn Fn(GovernmentId) -> bool,
    pub(super) territory_exists: &'a dyn Fn(TerritoryId) -> bool,
    pub(super) entity_exists: &'a dyn Fn(&EntityRef) -> bool,
}

pub(super) fn validate_person_draft(
    context: &PersonDraftContext<'_>,
    draft: &PersonDraft,
    correlation: &str,
    summary: &str,
) -> Result<(), CanwuError> {
    require_core_write(
        context.plugin,
        context.contract,
        &StateKey::core_people(),
        &[BoundaryPhase::DomainDeltaProposal],
    )?;
    if !canonical_text(summary)
        || !canonical_text(correlation)
        || correlation.len() > MAX_PERSON_CORRELATION_BYTES
        || !canonical_text(&draft.name)
        || draft.roles.iter().any(|role| !canonical_text(role))
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidBoundary,
            "person drafts require canonical name, roles, correlation, and summary",
        ));
    }
    if !(context.government_exists)(draft.government)
        || !(context.territory_exists)(draft.current_location)
    {
        return Err(CanwuError::new(
            ErrorCode::EntityNotFound,
            "person draft references a missing government or location",
        ));
    }
    // The ID is not allocated yet; validate against an impossible identity so
    // a draft cannot name itself as custodian.
    validate_availability_value(
        PersonId::new(0),
        &draft.availability,
        context.now,
        context.entity_exists,
    )
}

/// Boundary-wide staging guard for person directives.
#[derive(Default)]
pub(super) struct BoundaryPersonWrites {
    availability_writers: BTreeMap<PersonId, (String, String)>,
    correlations: BTreeSet<(String, String, String)>,
}

impl BoundaryPersonWrites {
    pub(super) fn stage(
        &mut self,
        plugin: &str,
        system: &str,
        directives: &[super::BoundaryDirective],
    ) -> Result<(), CanwuError> {
        for directive in directives {
            match directive {
                super::BoundaryDirective::SetPersonAvailability { person, .. } => {
                    if let Some((existing_plugin, existing_system)) = self
                        .availability_writers
                        .insert(*person, (plugin.to_owned(), system.to_owned()))
                    {
                        return Err(CanwuError::new(
                            ErrorCode::DuplicateBoundaryWriter,
                            format!(
                                "availability of person {person} is written by both {existing_plugin}.{existing_system} and {plugin}.{system} in one boundary"
                            ),
                        )
                        .with_entity(EntityRef::Person(*person)));
                    }
                }
                super::BoundaryDirective::CreatePerson { correlation, .. } => {
                    let key = (plugin.to_owned(), system.to_owned(), correlation.clone());
                    if !self.correlations.insert(key) {
                        return Err(CanwuError::new(
                            ErrorCode::InvalidBoundary,
                            format!(
                                "person creation correlation {correlation} is duplicated by {plugin}.{system} in one boundary"
                            ),
                        ));
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
}

fn command_persons(issuer: &Issuer, authority: &CommandAuthority) -> BTreeSet<PersonId> {
    let mut persons = BTreeSet::new();
    if let Issuer::Actor(actor) = issuer {
        persons.insert(*actor);
    }
    match &authority.decision_origin {
        DecisionOrigin::Actor { actor }
        | DecisionOrigin::Institution {
            responsible_actor: Some(actor),
            ..
        } => {
            persons.insert(*actor);
        }
        DecisionOrigin::Institution {
            responsible_actor: None,
            ..
        }
        | DecisionOrigin::Council { .. }
        | DecisionOrigin::NoResponsibleActor { .. } => {}
    }
    persons
}

/// Rejects a command whose issuing person is dead, missing, detained, or
/// captive. Non-person issuers and authorities are unaffected.
pub(super) fn validate_command_issuer_availability(
    availability: &BTreeMap<PersonId, PersonAvailability>,
    issuer: &Issuer,
    authority: &CommandAuthority,
) -> Result<(), CanwuError> {
    for person in command_persons(issuer, authority) {
        if !person_is_available(availability, person) {
            return Err(CanwuError::new(
                ErrorCode::IssuerUnavailable,
                format!("command issuer person {person} is dead, missing, detained, or captive"),
            )
            .with_entity(EntityRef::Person(person)));
        }
    }
    Ok(())
}

const fn authority_person(authority: &DecisionAuthority) -> Option<PersonId> {
    match authority {
        DecisionAuthority::Actor { actor } => Some(*actor),
        DecisionAuthority::Institution {
            responsible_actor, ..
        } => *responsible_actor,
        DecisionAuthority::Council { .. } | DecisionAuthority::NoResponsibleActor { .. } => None,
    }
}

fn decision_maker_message(person: PersonId) -> String {
    format!("decision maker person {person} is dead, missing, detained, or captive")
}

fn controller_issuer_message(controller: &str, person: PersonId) -> String {
    format!(
        "decision controller {controller} acts for person {person}, who is dead, missing, detained, or captive"
    )
}

fn controller_authority_availability_error(
    decisions: &DecisionState,
    controller_id: &str,
    availability: &BTreeMap<PersonId, PersonAvailability>,
) -> Option<(DecisionAttemptErrorCode, String)> {
    decisions
        .controller(controller_id)
        .and_then(|controller| authority_person(&controller.authority))
        .filter(|person| !person_is_available(availability, *person))
        .map(|person| {
            (
                DecisionAttemptErrorCode::IssuerUnavailable,
                controller_issuer_message(controller_id, person),
            )
        })
}

/// Deterministic availability admission rule for decision ingress, shared by
/// the runtime and snapshot reconstruction.
///
/// `Open` refuses an unavailable person decision maker first, then an
/// assigned controller whose authority person is unavailable, so a ticket is
/// never opened for a controller that could not resolve it. `Resolve` refuses
/// a controller whose authority person is unavailable.
pub(super) fn decision_mutation_availability_error(
    mutation: &DecisionMutation,
    decisions: &DecisionState,
    availability: &BTreeMap<PersonId, PersonAvailability>,
) -> Option<(DecisionAttemptErrorCode, String)> {
    match mutation {
        DecisionMutation::Open { ticket } => match ticket.decision_maker {
            EntityRef::Person(person) if !person_is_available(availability, person) => Some((
                DecisionAttemptErrorCode::DecisionMakerUnavailable,
                decision_maker_message(person),
            )),
            _ => controller_authority_availability_error(
                decisions,
                &ticket.assigned_controller,
                availability,
            ),
        },
        DecisionMutation::Resolve { controller_id, .. } => {
            controller_authority_availability_error(decisions, controller_id, availability)
        }
        DecisionMutation::RegisterController { .. }
        | DecisionMutation::ReplaceOptions { .. }
        | DecisionMutation::Cancel { .. } => None,
    }
}

/// Availability guard for resolving one ticket through its controller, used
/// by host decision preparation and by boundary random decision resolutions.
pub(super) fn validate_decision_preparation(
    availability: &BTreeMap<PersonId, PersonAvailability>,
    ticket: &DecisionTicket,
    controller: &DecisionControllerBinding,
) -> Result<(), CanwuError> {
    if let EntityRef::Person(person) = ticket.decision_maker
        && !person_is_available(availability, person)
    {
        return Err(CanwuError::new(
            ErrorCode::DecisionMakerUnavailable,
            decision_maker_message(person),
        )
        .with_entity(EntityRef::Person(person)));
    }
    if let Some(person) = authority_person(&controller.authority)
        && !person_is_available(availability, person)
    {
        return Err(CanwuError::new(
            ErrorCode::IssuerUnavailable,
            controller_issuer_message(&controller.id, person),
        )
        .with_entity(EntityRef::Person(person)));
    }
    Ok(())
}

/// Cancels the open tickets selected by `matches`, in ticket-ID order, with
/// `reason` and returns the cancelled IDs.
fn cancel_open_tickets(
    decisions: &mut DecisionState,
    matches: impl Fn(&DecisionState, &DecisionTicket) -> bool,
    reason: &str,
    at: SimTime,
) -> Result<Vec<DecisionTicketId>, CanwuError> {
    let state: &DecisionState = decisions;
    let open: Vec<_> = state
        .open_tickets()
        .filter(|ticket| matches(state, ticket))
        .map(|ticket| (ticket.id, ticket.version))
        .collect();
    for (ticket_id, expected_version) in &open {
        decisions
            .apply(
                DecisionMutation::Cancel {
                    ticket_id: *ticket_id,
                    expected_version: *expected_version,
                    reason: reason.to_owned(),
                },
                at,
                None,
            )
            .map_err(super::decision::decision_error)?;
    }
    Ok(open.into_iter().map(|(ticket_id, _)| ticket_id).collect())
}

/// End-of-boundary sweep shared by the runtime and snapshot reconstruction.
///
/// For every change that leaves its person unavailable, first cancels the
/// open tickets whose decision maker is that person
/// ([`DECISION_MAKER_UNAVAILABLE_REASON`]); then, for every such change,
/// cancels the remaining open tickets whose assigned controller's authority
/// person is that person ([`CONTROLLER_AUTHORITY_UNAVAILABLE_REASON`]). The
/// decision-maker pass runs for all changes first, so a ticket that qualifies
/// for both reasons carries the decision-maker reason. Both lists are
/// rewritten on every change, in ticket-ID order.
pub(super) fn sweep_unavailable_person_tickets(
    decisions: &mut DecisionState,
    changes: &mut [BoundaryPersonAvailabilityChange],
    at: SimTime,
) -> Result<(), CanwuError> {
    for change in changes.iter_mut() {
        let maker = EntityRef::Person(change.person);
        change.cancelled_tickets = if change.availability.is_available() {
            Vec::new()
        } else {
            cancel_open_tickets(
                decisions,
                |_, ticket| ticket.decision_maker == maker,
                DECISION_MAKER_UNAVAILABLE_REASON,
                at,
            )?
        };
    }
    for change in changes.iter_mut() {
        let person = change.person;
        change.cancelled_controller_tickets = if change.availability.is_available() {
            Vec::new()
        } else {
            cancel_open_tickets(
                decisions,
                |decisions, ticket| {
                    decisions
                        .controller(&ticket.assigned_controller)
                        .and_then(|controller| authority_person(&controller.authority))
                        == Some(person)
                },
                CONTROLLER_AUTHORITY_UNAVAILABLE_REASON,
                at,
            )?
        };
    }
    Ok(())
}

/// First runtime-allocated person ID: one past every initial person identity.
pub(super) fn first_runtime_person_id(
    initial_entities: &[EntityRef],
    initial_people: &[Person],
) -> Result<u64, CanwuError> {
    initial_entities
        .iter()
        .filter_map(|entity| match entity {
            EntityRef::Person(person) => Some(person.get()),
            _ => None,
        })
        .chain(initial_people.iter().map(|person| person.id.get()))
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::IdentifierExhausted,
                "person identifier space is exhausted",
            )
        })
}

impl Simulation {
    /// Returns committed availability for a person. `None` means no change
    /// has been committed: the person is alive and free.
    #[must_use]
    pub fn person_availability(&self, person: PersonId) -> Option<&PersonAvailability> {
        self.state.current.person_availability.get(&person)
    }

    /// Returns every committed person availability in person-ID order.
    pub fn person_availabilities(&self) -> impl Iterator<Item = (&PersonId, &PersonAvailability)> {
        self.state.current.person_availability.iter()
    }

    pub(super) fn validate_command_issuer(
        &self,
        issuer: &Issuer,
        authority: &CommandAuthority,
    ) -> Result<(), CanwuError> {
        validate_command_issuer_availability(
            &self.state.current.person_availability,
            issuer,
            authority,
        )
    }

    pub(super) fn apply_person_availability(
        &mut self,
        staged: (&str, &str, BoundaryPhase, StateVisibility),
        person: PersonId,
        availability: PersonAvailability,
        summary: String,
    ) -> Result<BoundaryPersonAvailabilityChange, CanwuError> {
        let (plugin, system, phase, visibility) = staged;
        let now = self.state.scheduler.now;
        let entity = EntityRef::Person(person);
        if !runtime_entity_exists(&self.state, &entity) {
            return Err(CanwuError::new(
                ErrorCode::EntityNotFound,
                format!("boundary stage {plugin}.{system} references unavailable person {person}"),
            )
            .with_entity(entity));
        }
        validate_availability_value(person, &availability, now, &|entity| {
            runtime_entity_exists(&self.state, entity)
        })?;
        self.invalidate_commitments(super::CommitmentDomains::WORLD);
        let previous = self
            .state
            .current
            .person_availability
            .insert(person, availability.clone());
        Ok(BoundaryPersonAvailabilityChange {
            plugin: plugin.to_owned(),
            system: system.to_owned(),
            phase,
            person,
            previous,
            availability,
            visibility,
            summary,
            cancelled_tickets: Vec::new(),
            cancelled_controller_tickets: Vec::new(),
        })
    }

    /// Closes, at the end of the boundary and after random-decision ingress is
    /// materialized, every open ticket whose person decision maker or whose
    /// assigned controller's authority person became unavailable in this
    /// boundary. The IDs are recorded on each change.
    pub(super) fn cancel_unavailable_person_tickets(
        &mut self,
        changes: &mut [BoundaryPersonAvailabilityChange],
    ) -> Result<(), CanwuError> {
        if changes
            .iter()
            .all(|change| change.availability.is_available())
        {
            return Ok(());
        }
        let mut decisions = self.state.current.decisions.clone();
        sweep_unavailable_person_tickets(&mut decisions, changes, self.state.scheduler.now)?;
        if changes.iter().any(|change| {
            !change.cancelled_tickets.is_empty() || !change.cancelled_controller_tickets.is_empty()
        }) {
            self.invalidate_commitments(super::CommitmentDomains::DECISIONS);
            self.state.current.decisions = decisions;
        }
        Ok(())
    }

    pub(super) fn apply_person_creation(
        &mut self,
        plugin: &str,
        system: &str,
        draft: PersonDraft,
        correlation: String,
        summary: String,
    ) -> Result<BoundaryPersonCreation, CanwuError> {
        let now = self.state.scheduler.now;
        if !self
            .state
            .current
            .governments
            .contains_key(&draft.government)
            || !self
                .state
                .current
                .territories
                .contains_key(&draft.current_location)
        {
            return Err(CanwuError::new(
                ErrorCode::EntityNotFound,
                "person draft references a missing government or location",
            ));
        }
        let next = if self.state.counters.next_person_id == 0 {
            let scenario = self
                .state
                .metadata
                .initial_scenario
                .as_ref()
                .ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::InvalidSnapshot,
                        "runtime person creation requires the bound initial scenario",
                    )
                })?;
            first_runtime_person_id(&scenario.entities, &scenario.world.people)?
        } else {
            self.state.counters.next_person_id
        };
        let (id, next_id) = claim_counter(next, "person ID")?;
        let id = PersonId::new(id);
        let entity = EntityRef::Person(id);
        if self.state.current.entities.contains(&entity)
            || self.state.current.people.contains_key(&id)
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidSnapshot,
                "runtime person allocation collided with an existing identity",
            ));
        }
        validate_availability_value(id, &draft.availability, now, &|entity| {
            runtime_entity_exists(&self.state, entity)
        })?;
        let person = Person {
            id,
            name: draft.name,
            government: draft.government,
            current_location: draft.current_location,
            roles: draft.roles,
            transit: None,
        };
        self.invalidate_commitments(super::CommitmentDomains::WORLD);
        self.state.counters.next_person_id = next_id;
        self.state.current.entities.insert(entity);
        self.state.current.people.insert(id, person.clone());
        self.state
            .current
            .person_availability
            .insert(id, draft.availability.clone());
        let creation = BoundaryPersonCreation {
            plugin: plugin.to_owned(),
            system: system.to_owned(),
            correlation,
            person,
            availability: draft.availability,
            provenance: draft.provenance,
            summary,
        };
        self.state
            .current
            .created_persons
            .push(CreatedPerson::from(&creation));
        Ok(creation)
    }
}

/// Person state reconstructed boundary by boundary during snapshot
/// validation, so decision-ingress reconstruction sees the same availability
/// and person identities that the runtime saw at admission.
pub(super) struct PersonReplayCut {
    pub(super) availability: BTreeMap<PersonId, PersonAvailability>,
    runtime_created: BTreeSet<PersonId>,
    created: BTreeSet<PersonId>,
}

impl PersonReplayCut {
    pub(super) fn new(snapshot: &SimulationSnapshot) -> Self {
        Self {
            availability: BTreeMap::new(),
            runtime_created: snapshot
                .boundaries
                .iter()
                .flat_map(|boundary| &boundary.created_persons)
                .map(|creation| creation.person.id)
                .collect(),
            created: BTreeSet::new(),
        }
    }

    /// Returns `None` for identities not created at runtime, whose existence
    /// is then decided by the persisted entity registry.
    pub(super) fn runtime_person_exists(&self, person: PersonId) -> Option<bool> {
        self.runtime_created
            .contains(&person)
            .then(|| self.created.contains(&person))
    }

    /// Advances through one committed boundary, re-deriving the same-boundary
    /// ticket cancellations on the reconstructed decision state.
    pub(super) fn apply_boundary(
        &mut self,
        record: &BoundaryRecord,
        decisions: &mut DecisionState,
    ) -> Result<(), CanwuError> {
        let mut expected = record.person_availability_changes.clone();
        sweep_unavailable_person_tickets(decisions, &mut expected, record.at)?;
        if expected != record.person_availability_changes {
            return invalid_snapshot(
                "person availability changes do not match their same-boundary ticket cancellations",
            );
        }
        for change in &record.person_availability_changes {
            self.availability
                .insert(change.person, change.availability.clone());
        }
        for creation in &record.created_persons {
            self.created.insert(creation.person.id);
            self.availability
                .insert(creation.person.id, creation.availability.clone());
        }
        Ok(())
    }
}

/// Checks the persisted entity registry: the initial scenario registry plus
/// exactly the persons created by committed boundaries.
pub(super) fn validate_snapshot_entity_registry(
    snapshot: &SimulationSnapshot,
) -> Result<(), CanwuError> {
    let Some(initial) = snapshot.initial_scenario.as_ref() else {
        return Ok(());
    };
    let mut expected_entities: BTreeSet<_> = initial.entities.iter().cloned().collect();
    let mut expected_people: BTreeSet<_> = initial
        .world
        .people
        .iter()
        .map(|person| person.id)
        .collect();
    for creation in snapshot
        .boundaries
        .iter()
        .flat_map(|boundary| &boundary.created_persons)
    {
        let person = creation.person.id;
        if !expected_entities.insert(EntityRef::Person(person)) || !expected_people.insert(person) {
            return invalid_snapshot("runtime-created person reuses an existing identity");
        }
    }
    if expected_entities.into_iter().collect::<Vec<_>>() != snapshot.entities
        || expected_people
            != snapshot
                .world
                .people
                .iter()
                .map(|person| person.id)
                .collect::<BTreeSet<_>>()
    {
        return invalid_snapshot(
            "snapshot entity registry does not match its manifest-bound initial scenario and committed person creations",
        );
    }
    Ok(())
}

fn snapshot_availability_is_valid(
    snapshot: &SimulationSnapshot,
    person: PersonId,
    availability: &PersonAvailability,
    at: SimTime,
) -> bool {
    validate_availability_value(person, availability, at, &|entity| {
        super::validation::snapshot_entity_identity_exists(snapshot, entity)
    })
    .is_ok()
}

fn snapshot_core_writer<'a>(
    plugins: &'a PluginRegistry,
    record: &BoundaryRecord,
    source: (&str, &str),
    key: &StateKey,
    phases: &[BoundaryPhase],
) -> Option<&'a BoundarySystemContract> {
    super::validation::snapshot_boundary_contract(plugins, source.0, source.1).filter(|contract| {
        contract.writes.contains(key)
            && phases.contains(&contract.phase)
            && super::boundary_system_due(
                contract,
                &record.cadences,
                super::boundary_has_event_ingress(record),
            )
    })
}

/// Validates committed availability and creation evidence and proves that it
/// reconstructs the persisted availability map and person counter.
pub(super) fn validate_snapshot_persons(
    snapshot: &SimulationSnapshot,
    plugins: &PluginRegistry,
) -> Result<(), CanwuError> {
    let Some(initial) = snapshot.initial_scenario.as_ref() else {
        return invalid_snapshot("person evidence requires the manifest-bound initial scenario");
    };
    let first_id = first_runtime_person_id(&initial.entities, &initial.world.people)
        .map_err(|error| invalid_snapshot_error(error.message))?;
    let mut next_id = first_id;
    let mut availability = BTreeMap::new();
    let availability_key = StateKey::core_person_availability();
    let people_key = StateKey::core_people();
    let evidence = super::validation::SnapshotValidationContext::new(snapshot);
    for record in &snapshot.boundaries {
        let mut touched = BTreeSet::new();
        for change in &record.person_availability_changes {
            let Some(contract) = snapshot_core_writer(
                plugins,
                record,
                (&change.plugin, &change.system),
                &availability_key,
                &[
                    BoundaryPhase::DomainDeltaProposal,
                    BoundaryPhase::HistoricalCandidateEvaluation,
                ],
            ) else {
                return invalid_snapshot("person availability change has no declared writer");
            };
            if change.phase != contract.phase
                || change.visibility != contract.visibility
                || !canonical_text(&change.summary)
                || !touched.insert(change.person)
                || !super::validation::snapshot_entity_exists(
                    snapshot,
                    &EntityRef::Person(change.person),
                )
                || change.previous.as_ref() != availability.get(&change.person)
                || !snapshot_availability_is_valid(
                    snapshot,
                    change.person,
                    &change.availability,
                    record.at,
                )
                || [
                    &change.cancelled_tickets,
                    &change.cancelled_controller_tickets,
                ]
                .into_iter()
                .any(|tickets| tickets.windows(2).any(|pair| pair[0] >= pair[1]))
            {
                return invalid_snapshot("person availability change evidence is inconsistent");
            }
            availability.insert(change.person, change.availability.clone());
        }
        let mut correlations = BTreeSet::new();
        for creation in &record.created_persons {
            let person = &creation.person;
            let valid_writer = snapshot_core_writer(
                plugins,
                record,
                (&creation.plugin, &creation.system),
                &people_key,
                &[BoundaryPhase::DomainDeltaProposal],
            )
            .is_some();
            if !valid_writer
                || person.id.get() != next_id
                || person.transit.is_some()
                || !canonical_text(&person.name)
                || person.roles.iter().any(|role| !canonical_text(role))
                || !canonical_text(&creation.summary)
                || !canonical_text(&creation.correlation)
                || creation.correlation.len() > MAX_PERSON_CORRELATION_BYTES
                || !correlations.insert((
                    creation.plugin.as_str(),
                    creation.system.as_str(),
                    creation.correlation.as_str(),
                ))
                || touched.contains(&person.id)
                || snapshot.world.person(person.id).is_none()
                || initial.world.government(person.government).is_none()
                || initial.world.territory(person.current_location).is_none()
                || !snapshot_availability_is_valid(
                    snapshot,
                    person.id,
                    &creation.availability,
                    record.at,
                )
                || super::validation::resolve_evidence_reference(&evidence, &creation.provenance)
                    != super::validation::EvidenceAvailability::Retained
                || availability
                    .insert(person.id, creation.availability.clone())
                    .is_some()
            {
                return invalid_snapshot("person creation evidence is inconsistent");
            }
            next_id = next_id.checked_add(1).ok_or_else(|| {
                invalid_snapshot_error("runtime person identifier space is exhausted")
            })?;
        }
    }
    let registry: Vec<_> = snapshot
        .boundaries
        .iter()
        .flat_map(|boundary| &boundary.created_persons)
        .map(CreatedPerson::from)
        .collect();
    if registry != snapshot.created_persons {
        return invalid_snapshot(
            "boundary person creations do not reconstruct the persisted created-person registry",
        );
    }
    let expected_counter = if next_id == first_id { 0 } else { next_id };
    if snapshot.next_person_id != expected_counter {
        return invalid_snapshot("runtime person counter does not follow committed creations");
    }
    if availability != snapshot.person_availability {
        return invalid_snapshot(
            "boundary availability changes do not reconstruct the persisted person availability",
        );
    }
    Ok(())
}
