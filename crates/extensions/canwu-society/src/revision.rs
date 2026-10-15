//! Decision-provenanced runtime revision of policy pressure and of the
//! activity and reach of influence edges and organization nodes.
//!
//! An institution revises through the alignment that binds it to a target.
//! The [`PRESSURE_REVISION_COMMAND`] command checks the decision's authority
//! exactly as `set_institutional_policy` does and queues one internal
//! [`SOCIETY_PRESSURE_REVISION_INGRESS`] packet. The society plugin applies
//! queued packets in admission order in its next settlement, so the plugin
//! remains the only writer of `canwu.society:state`.
//!
//! The packet is admitted and queued at the boundary after the command, so a
//! revision takes effect at the first society settlement after that
//! admission, at least two boundaries after its command. An institutional
//! policy decision, which the command stores directly, takes effect one
//! boundary sooner.

use crate::ingress::SOCIETY_INGRESS_MALFORMED_REJECTION;
use crate::model::{InstitutionalAlignment, SocietyState, invalid};
use canwu_api::{CanwuError, CauseRef, CommandId, EntityRef, ErrorCode, IngressId, SimTime};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Society command carrying one [`PressureRevisionDecision`].
pub const PRESSURE_REVISION_COMMAND: &str = "revise_society_pressure";
/// Internal society ingress carrying one [`SocietyPressureRevisionV1`].
///
/// Hosts cannot enqueue it. Only the [`PRESSURE_REVISION_COMMAND`] handler
/// produces an admissible packet: the intake rejects a packet any other
/// source scheduled as [`PRESSURE_REVISION_ORIGIN_REJECTION`].
pub const SOCIETY_PRESSURE_REVISION_INGRESS: &str = "society_pressure_revision_v1";
/// Most rows one [`PressureRevisionDecision`] may revise.
pub const MAX_PRESSURE_REVISION_ROWS: usize = 256;
/// Event the society settlement emits for every applied pressure revision.
/// Its summary names the alignment, the decision version, the command, and
/// the ingress.
pub const SOCIETY_PRESSURE_REVISED_EVENT: &str = "society_pressure_revised_v1";
/// Event the society settlement emits for every rejected pressure revision.
/// Its summary names the rejection code and, when the packet could be
/// decoded, the alignment, the decision version, and the command.
pub const SOCIETY_PRESSURE_REVISION_REJECTED_EVENT: &str = "society_pressure_revision_rejected_v1";

/// Rejection code for a packet the revision command did not produce.
pub const PRESSURE_REVISION_ORIGIN_REJECTION: &str = "unauthorized_origin";
/// Rejection code for a decision whose alignment no longer exists, or is no
/// longer bound to the deciding institution, when the decision settles.
pub const PRESSURE_REVISION_AUTHORITY_REJECTION: &str = "authority_unavailable";
/// Rejection code for a decision whose `expected_version` is not the
/// alignment's applied revision version when the decision settles.
pub const PRESSURE_REVISION_STALE_REJECTION: &str = "stale_expected_version";
/// Rejection code for a decision whose version is not newer than the
/// alignment's applied revision version and is not that applied decision.
pub const PRESSURE_REVISION_SUPERSEDED_REJECTION: &str = "superseded_decision";
/// Rejection code for a decision that names a row which no longer exists or
/// lies outside the alignment's authority when the decision settles.
pub const PRESSURE_REVISION_SCOPE_REJECTION: &str = "row_out_of_scope";

/// New values for the pressure fields of one [`crate::PolicyPressure`] row.
///
/// The row's identity, target, and affected cohorts are structural and are
/// not revised. Every pressure field is stated, so a revision replaces the
/// row's pressure as a whole.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyPressureRevision {
    pub policy_id: String,
    pub support_per_mille: u16,
    pub legal_access_per_mille: u16,
    pub surveillance_per_mille: u16,
    pub censorship_per_mille: u16,
    pub coercion_per_mille: u16,
    pub material_penalty_per_mille: u16,
    pub disruption_per_mille: u16,
    pub migration_pressure_per_mille: u16,
}

/// New activity and reach of one [`crate::SocialInfluenceEdge`]. The edge's
/// source, target, channel, and trust are not revised.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InfluenceEdgeRevision {
    pub edge_id: String,
    pub active: bool,
    pub reach_per_mille: u16,
}

/// New activity and base reach of one [`crate::OrganizationNode`]. The
/// node's target and concealment are not revised.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OrganizationRevision {
    pub organization_id: String,
    pub active: bool,
    pub base_reach_per_mille: u16,
}

/// One versioned decision of the institution behind an alignment to revise
/// policy pressure it issued and the activity and reach of its target's
/// influence edges and organization nodes.
///
/// It is the payload of the [`PRESSURE_REVISION_COMMAND`] command. The
/// applied decisions of one alignment form a strictly increasing sequence of
/// `decision_version`, independent of the `set_institutional_policy`
/// sequence in [`InstitutionalAlignment::last_decision_version`]. The
/// society settles queued decisions in admission order: one applies when its
/// version is newer than the alignment's applied revision version and its
/// `expected_version` is that applied version; a resend of the applied
/// decision changes nothing; any other is rejected as a whole.
///
/// A decision changes nothing for a cohort outside its alignment's affected
/// cohorts (every cohort when that set is empty). It may revise:
///
/// * a [`crate::PolicyPressure`] row of the alignment's target whose `issuer`
///   is the alignment's institution and whose affected cohorts lie within the
///   alignment's; the applied decision version is written to the row's
///   `decision_version`. A row with no issuer is never revisable;
/// * a [`crate::SocialInfluenceEdge`] of the alignment's target whose target
///   cohort lies within the alignment's. An edge changes only the influence
///   signal of its target cohort, so its source may be any cohort,
///   organization, or institution;
/// * an [`crate::OrganizationNode`] of the alignment's target, only through
///   an alignment that covers every cohort: an organization's activity and
///   reach change signals, mobilization capacity, and concealment for every
///   cohort of its target. Through
///   [`crate::SocietyState::organization_relations`] they also change the
///   strength of related organizations, which may belong to other targets;
///   the rule bounds cohorts, not relations.
///
/// Rows are never created or removed, and transition rules are not revised.
/// A decision applies as a whole or not at all.
///
/// Two alignments of one institution on one target have separate version
/// sequences and may both revise one pressure row, so that row's
/// `decision_version` names the last decision applied to it and can be
/// lower than an earlier one's.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PressureRevisionDecision {
    /// The alignment whose institution decides.
    pub alignment_id: String,
    /// Version of this decision; positive.
    pub decision_version: u64,
    /// The alignment's applied revision version this decision was taken
    /// against: zero before the first, otherwise the `decision_version` of
    /// the decision it follows. A decision whose expectation is not the
    /// applied version when it settles is rejected as
    /// [`PRESSURE_REVISION_STALE_REJECTION`].
    pub expected_version: u64,
    /// Revised pressure rows, in ascending `policy_id` order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub policies: Vec<PolicyPressureRevision>,
    /// Revised influence edges, in ascending `edge_id` order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub influence_edges: Vec<InfluenceEdgeRevision>,
    /// Revised organization nodes, in ascending `organization_id` order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub organizations: Vec<OrganizationRevision>,
}

impl PressureRevisionDecision {
    /// Validates the decision's shape without reading society state.
    ///
    /// # Errors
    ///
    /// Returns `InvalidDecision` when the alignment ID is empty, the version
    /// is zero or not newer than the expected version, the decision revises
    /// no row or more than [`MAX_PRESSURE_REVISION_ROWS`], a row list is not
    /// in strictly ascending ID order, or a value exceeds 1,000 per mille.
    pub fn validate(&self) -> Result<(), CanwuError> {
        let rows = self.policies.len() + self.influence_edges.len() + self.organizations.len();
        let ordered = |ids: Vec<&str>| {
            ids.iter().all(|id| !id.is_empty()) && ids.windows(2).all(|pair| pair[0] < pair[1])
        };
        let bounded = self.policies.iter().all(|policy| {
            [
                policy.support_per_mille,
                policy.legal_access_per_mille,
                policy.surveillance_per_mille,
                policy.censorship_per_mille,
                policy.coercion_per_mille,
                policy.material_penalty_per_mille,
                policy.disruption_per_mille,
                policy.migration_pressure_per_mille,
            ]
            .into_iter()
            .all(|value| value <= 1_000)
        }) && self
            .influence_edges
            .iter()
            .all(|edge| edge.reach_per_mille <= 1_000)
            && self
                .organizations
                .iter()
                .all(|organization| organization.base_reach_per_mille <= 1_000);
        if self.alignment_id.is_empty()
            || self.decision_version == 0
            || self.expected_version >= self.decision_version
            || rows == 0
            || rows > MAX_PRESSURE_REVISION_ROWS
            || !ordered(
                self.policies
                    .iter()
                    .map(|row| row.policy_id.as_str())
                    .collect(),
            )
            || !ordered(
                self.influence_edges
                    .iter()
                    .map(|row| row.edge_id.as_str())
                    .collect(),
            )
            || !ordered(
                self.organizations
                    .iter()
                    .map(|row| row.organization_id.as_str())
                    .collect(),
            )
            || !bounded
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidDecision,
                "pressure revision must be versioned above its expected version, revise between \
                 1 and 256 rows in ascending ID order, and stay within permille bounds",
            ));
        }
        Ok(())
    }

    /// Canonical hash that identifies this exact decision. A resent decision
    /// is the applied one exactly when the digests match.
    ///
    /// # Errors
    ///
    /// Returns an error when the decision cannot be canonically encoded.
    pub fn digest(&self) -> Result<String, CanwuError> {
        canwu_api::canonical_hash("canwu.society.pressure-revision.v1", self)
    }
}

/// The packet the revision command queues for the society owner: the
/// decision with the provenance the command established at admission.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SocietyPressureRevisionV1 {
    pub decision: PressureRevisionDecision,
    /// The institution of the alignment when the command was admitted.
    pub issuer: EntityRef,
    /// The admitted command; the packet's ingress record names it as cause.
    pub command: CommandId,
}

/// Terminal outcome of one pressure revision decision.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PressureRevisionOutcome {
    pub decision_version: u64,
    /// [`PressureRevisionDecision::digest`] of the decision.
    pub digest: String,
    /// The deciding institution.
    pub issuer: EntityRef,
    /// The command that carried the decision.
    pub command: CommandId,
    /// The queued packet that settled.
    pub ingress: IngressId,
    pub settled_at: SimTime,
    /// Stable rejection code; present exactly on a rejected outcome.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejection: Option<String>,
}

/// The latest terminal outcomes of the pressure revisions decided through
/// one alignment. Earlier outcomes remain in the event journal as
/// [`SOCIETY_PRESSURE_REVISED_EVENT`] and
/// [`SOCIETY_PRESSURE_REVISION_REJECTED_EVENT`] evidence.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PressureRevisionOutcomes {
    /// The last applied decision. Its version is the alignment's applied
    /// revision version, which the next decision must expect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied: Option<PressureRevisionOutcome>,
    /// The rejected decision that settled last. A pressure revision refused
    /// for a full queue settles when the boundary that admits its packet
    /// commits, which can be before a decision of a lower version, queued
    /// earlier, is rejected at the next settlement; that rejection then
    /// replaces it here. Every rejection stays in the event journal as a
    /// [`SOCIETY_PRESSURE_REVISION_REJECTED_EVENT`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejected: Option<PressureRevisionOutcome>,
}

impl PressureRevisionOutcomes {
    /// The alignment's applied revision version; zero before the first.
    #[must_use]
    pub fn applied_version(&self) -> u64 {
        self.applied
            .as_ref()
            .map_or(0, |outcome| outcome.decision_version)
    }

    pub(crate) fn is_valid(&self) -> bool {
        let well_formed = |outcome: &PressureRevisionOutcome| {
            outcome.decision_version > 0
                && outcome.digest.len() == 64
                && outcome
                    .digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        };
        self.applied
            .as_ref()
            .is_none_or(|outcome| well_formed(outcome) && outcome.rejection.is_none())
            && self.rejected.as_ref().is_none_or(|outcome| {
                well_formed(outcome)
                    && outcome
                        .rejection
                        .as_ref()
                        .is_some_and(|reason| !reason.is_empty())
            })
            && (self.applied.is_some() || self.rejected.is_some())
    }
}

/// Why a decision cannot revise the rows it names.
pub(crate) enum RevisionScopeError {
    /// A named row does not exist.
    UnknownRow(String),
    /// A named row lies outside the alignment's authority.
    Unauthorized(String),
}

impl RevisionScopeError {
    pub(crate) fn into_error(self) -> CanwuError {
        match self {
            Self::UnknownRow(message) => CanwuError::new(ErrorCode::EntityNotFound, message),
            Self::Unauthorized(message) => CanwuError::new(ErrorCode::InvalidAuthority, message),
        }
    }
}

impl SocietyState {
    /// Checks that every row the decision names exists and lies within the
    /// authority of `alignment`.
    pub(crate) fn pressure_revision_scope(
        &self,
        decision: &PressureRevisionDecision,
        alignment: &InstitutionalAlignment,
    ) -> Result<(), RevisionScopeError> {
        for revision in &decision.policies {
            let policy = self.policies.get(&revision.policy_id).ok_or_else(|| {
                RevisionScopeError::UnknownRow(format!(
                    "unknown policy pressure {}",
                    revision.policy_id
                ))
            })?;
            let within_cohorts = alignment.affected_cohorts.is_empty()
                || (!policy.affected_cohorts.is_empty()
                    && policy
                        .affected_cohorts
                        .is_subset(&alignment.affected_cohorts));
            if policy.target_id != alignment.target_id
                || policy.issuer.as_ref() != Some(&alignment.institution)
                || !within_cohorts
            {
                return Err(RevisionScopeError::Unauthorized(format!(
                    "policy pressure {} was not issued by the institution of alignment {} for \
                     its target and cohorts",
                    policy.id, alignment.id
                )));
            }
        }
        for revision in &decision.influence_edges {
            let edge = self.influence_edges.get(&revision.edge_id).ok_or_else(|| {
                RevisionScopeError::UnknownRow(format!(
                    "unknown influence edge {}",
                    revision.edge_id
                ))
            })?;
            if edge.target_id != alignment.target_id
                || !(alignment.affected_cohorts.is_empty()
                    || alignment.affected_cohorts.contains(&edge.target_cohort_id))
            {
                return Err(RevisionScopeError::Unauthorized(format!(
                    "influence edge {} is outside the target and cohorts of alignment {}",
                    edge.id, alignment.id
                )));
            }
        }
        for revision in &decision.organizations {
            let organization = self
                .organizations
                .get(&revision.organization_id)
                .ok_or_else(|| {
                    RevisionScopeError::UnknownRow(format!(
                        "unknown organization {}",
                        revision.organization_id
                    ))
                })?;
            // An organization acts on every cohort of its target.
            if organization.target_id != alignment.target_id
                || !alignment.affected_cohorts.is_empty()
            {
                return Err(RevisionScopeError::Unauthorized(format!(
                    "organization {} is outside the target of alignment {}, or the alignment \
                     does not cover every cohort",
                    organization.id, alignment.id
                )));
            }
        }
        Ok(())
    }

    /// Atomically applies one pressure revision and returns whether any row
    /// changed.
    ///
    /// The decision must be well formed and every row it names must exist
    /// and lie within the authority of its alignment (see
    /// [`PressureRevisionDecision`]). Each revised policy row records the
    /// decision version. A changed coercion value or organization clears the
    /// materialized mobilization candidates, and a changed organization
    /// activity also clears the projections, for rematerialization (the
    /// society plugin keeps the projection of an observer whose estimate the
    /// change leaves as it was); aggregates do not depend on the revised
    /// rows and are kept.
    ///
    /// This applies the rows only. The decision's issuer, its place in the
    /// alignment's version sequence, and its idempotency are checked by the
    /// society plugin's command and settlement; a host that owns the state
    /// outside the plugin checks them itself.
    ///
    /// # Errors
    ///
    /// Returns `InvalidDecision` when the decision is malformed,
    /// `EntityNotFound` when it names an unknown row, and `InvalidAuthority`
    /// when it names an unknown alignment or a row outside the alignment's
    /// authority. On error the state is unchanged.
    pub fn apply_pressure_revision(
        &mut self,
        decision: &PressureRevisionDecision,
    ) -> Result<bool, CanwuError> {
        decision.validate()?;
        let alignment = self
            .institutional_alignments
            .get(&decision.alignment_id)
            .ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    format!(
                        "unknown pressure revision authority alignment {}",
                        decision.alignment_id
                    ),
                )
            })?;
        self.pressure_revision_scope(decision, alignment)
            .map_err(RevisionScopeError::into_error)?;

        let mut changed = false;
        let mut candidates_stale = false;
        let mut projections_stale = false;
        for revision in &decision.policies {
            let policy = self
                .policies
                .get_mut(&revision.policy_id)
                .ok_or_else(|| invalid("revised policy pressure disappeared"))?;
            let before = policy.clone();
            policy.support_per_mille = revision.support_per_mille;
            policy.legal_access_per_mille = revision.legal_access_per_mille;
            policy.surveillance_per_mille = revision.surveillance_per_mille;
            policy.censorship_per_mille = revision.censorship_per_mille;
            policy.coercion_per_mille = revision.coercion_per_mille;
            policy.material_penalty_per_mille = revision.material_penalty_per_mille;
            policy.disruption_per_mille = revision.disruption_per_mille;
            policy.migration_pressure_per_mille = revision.migration_pressure_per_mille;
            policy.decision_version = decision.decision_version;
            changed |= *policy != before;
            candidates_stale |= policy.coercion_per_mille != before.coercion_per_mille;
        }
        for revision in &decision.influence_edges {
            let edge = self
                .influence_edges
                .get_mut(&revision.edge_id)
                .ok_or_else(|| invalid("revised influence edge disappeared"))?;
            changed |=
                edge.active != revision.active || edge.reach_per_mille != revision.reach_per_mille;
            edge.active = revision.active;
            edge.reach_per_mille = revision.reach_per_mille;
        }
        for revision in &decision.organizations {
            let organization = self
                .organizations
                .get_mut(&revision.organization_id)
                .ok_or_else(|| invalid("revised organization disappeared"))?;
            let activity_changed = organization.active != revision.active;
            let reach_changed = organization.base_reach_per_mille != revision.base_reach_per_mille;
            organization.active = revision.active;
            organization.base_reach_per_mille = revision.base_reach_per_mille;
            changed |= activity_changed || reach_changed;
            candidates_stale |= activity_changed || reach_changed;
            projections_stale |= activity_changed;
        }
        // Mobilization candidates read policy coercion and organization
        // strength; projections read the concealment of active organizations.
        // Transition signals are computed at settlement and are not stored.
        if candidates_stale {
            self.mobilization_candidates.clear();
            self.last_mobilization_at = None;
        }
        if projections_stale {
            self.projections.clear();
            self.last_projection_at = None;
        }
        Ok(changed)
    }
}

/// Returns the admission rejection of a queued revision packet: it must be a
/// well-formed packet whose ingress record names the command it carries as
/// its cause, which only the revision command handler produces.
pub(crate) fn pressure_revision_admission_rejection(
    payload: &Value,
    cause: Option<&CauseRef>,
) -> Option<String> {
    let Ok(packet) = serde_json::from_value::<SocietyPressureRevisionV1>(payload.clone()) else {
        return Some(SOCIETY_INGRESS_MALFORMED_REJECTION.to_owned());
    };
    if !matches!(cause, Some(CauseRef::Command(command)) if *command == packet.command) {
        return Some(PRESSURE_REVISION_ORIGIN_REJECTION.to_owned());
    }
    packet
        .decision
        .validate()
        .is_err()
        .then(|| SOCIETY_INGRESS_MALFORMED_REJECTION.to_owned())
}
