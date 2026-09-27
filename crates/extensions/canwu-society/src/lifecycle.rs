//! Target-scoped structural lifecycle deltas applied by the society owner.
//!
//! A lifecycle provider (for example `canwu-culture`) decides *when* an
//! affiliation target becomes dormant, retired, or active again. The society
//! plugin remains the only writer of `canwu.society:state`, so the provider
//! hands it a [`SocietyLifecycleDeltaV1`]: either directly to
//! [`SocietyState::apply_lifecycle_delta`] in a host-driven flow, or as the
//! internal [`SOCIETY_LIFECYCLE_DELTA_INGRESS`] packet that the society plugin
//! applies in its own phase-7 settlement.

use crate::model::{InstitutionalAlignment, SocietyState, TransitionRule, invalid};
use canwu_api::CanwuError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Internal society ingress carrying one [`SocietyLifecycleDeltaV1`].
///
/// Hosts cannot enqueue it; a boundary system that declares it as a
/// cross-plugin ingress target schedules it.
pub const SOCIETY_LIFECYCLE_DELTA_INGRESS: &str = "society_lifecycle_delta_v1";

/// The provider-owned transition rules and institutional alignments of one
/// affiliation target.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SocietyTargetBindings {
    pub target_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<TransitionRule>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alignments: Vec<InstitutionalAlignment>,
}

/// One atomic, target-scoped structural change to society state.
///
/// * `installs` restore the bindings of reactivated targets. A rule must be
///   absent or identical; an alignment must be absent or bound to the same
///   institution, target, and cohorts (its current values are kept).
/// * `deactivations` remove the listed rules of dormant or retired targets.
/// * `releases` name retired targets (each must also be deactivated): their
///   listed alignments and every target-scoped distribution, aggregate,
///   mobilization candidate, and projection entry are released, but only when
///   no other rule, alignment, active influence edge, active organization, or
///   policy still depends on the target and the released alignments carry no
///   live values.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SocietyLifecycleDeltaV1 {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub installs: Vec<SocietyTargetBindings>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deactivations: Vec<SocietyTargetBindings>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub releases: BTreeSet<String>,
}

impl SocietyLifecycleDeltaV1 {
    /// Returns whether the delta changes nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.installs.is_empty() && self.deactivations.is_empty() && self.releases.is_empty()
    }

    /// Validates the delta's internal shape without reading society state.
    ///
    /// # Errors
    ///
    /// Returns `InvalidDomainRecord` when a target repeats, a binding names a
    /// different target, a target is both installed and deactivated, or a
    /// released target is not deactivated.
    pub fn validate(&self) -> Result<(), CanwuError> {
        let mut installed = BTreeSet::new();
        for set in &self.installs {
            if !installed.insert(set.target_id.as_str()) {
                return Err(invalid(format!(
                    "lifecycle delta installs target {} twice",
                    set.target_id
                )));
            }
            set.validate()?;
        }
        let mut deactivated = BTreeSet::new();
        for set in &self.deactivations {
            if !deactivated.insert(set.target_id.as_str())
                || installed.contains(set.target_id.as_str())
            {
                return Err(invalid(format!(
                    "lifecycle delta deactivates target {} twice or also installs it",
                    set.target_id
                )));
            }
            set.validate()?;
        }
        if let Some(target) = self
            .releases
            .iter()
            .find(|target| !deactivated.contains(target.as_str()))
        {
            return Err(invalid(format!(
                "lifecycle delta releases target {target} without deactivating it"
            )));
        }
        Ok(())
    }
}

impl SocietyTargetBindings {
    fn validate(&self) -> Result<(), CanwuError> {
        if self
            .rules
            .iter()
            .any(|rule| rule.target_id != self.target_id)
            || self
                .alignments
                .iter()
                .any(|alignment| alignment.target_id != self.target_id)
        {
            return Err(invalid(format!(
                "lifecycle delta bindings for target {} name another target",
                self.target_id
            )));
        }
        Ok(())
    }
}

impl SocietyState {
    /// Atomically applies one lifecycle delta.
    ///
    /// On error the state is unchanged.
    ///
    /// # Errors
    ///
    /// Returns `InvalidDomainRecord` when the delta is malformed, an install
    /// conflicts with an existing rule or alignment, a live society
    /// dependency blocks a release, or the result violates a society
    /// invariant.
    pub fn apply_lifecycle_delta(
        &mut self,
        delta: &SocietyLifecycleDeltaV1,
    ) -> Result<(), CanwuError> {
        delta.validate()?;
        let mut draft = self.clone();
        apply_lifecycle_delta_draft(&mut draft, delta)?;
        *self = draft;
        Ok(())
    }
}

#[allow(clippy::too_many_lines)]
fn apply_lifecycle_delta_draft(
    draft: &mut SocietyState,
    delta: &SocietyLifecycleDeltaV1,
) -> Result<(), CanwuError> {
    for set in &delta.installs {
        for rule in &set.rules {
            if let Some(existing) = draft.transition_rules.get(&rule.id) {
                if existing != rule {
                    return Err(invalid(format!(
                        "society transition {} conflicts with the lifecycle delta",
                        rule.id
                    )));
                }
            } else {
                draft.transition_rules.insert(rule.id.clone(), rule.clone());
            }
        }
        for alignment in &set.alignments {
            if let Some(existing) = draft.institutional_alignments.get(&alignment.id) {
                if existing.institution != alignment.institution
                    || existing.target_id != alignment.target_id
                    || existing.affected_cohorts != alignment.affected_cohorts
                {
                    return Err(invalid(format!(
                        "society alignment {} conflicts with the lifecycle delta",
                        alignment.id
                    )));
                }
            } else {
                draft
                    .institutional_alignments
                    .insert(alignment.id.clone(), alignment.clone());
            }
        }
    }

    let inactive_rules = delta
        .deactivations
        .iter()
        .flat_map(|set| set.rules.iter().map(|rule| rule.id.as_str()))
        .collect::<BTreeSet<_>>();
    let released_targets = delta
        .releases
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let released = delta
        .deactivations
        .iter()
        .filter(|set| released_targets.contains(set.target_id.as_str()));
    let mut released_rules = BTreeSet::new();
    let mut released_alignments = BTreeSet::new();
    for set in released {
        released_rules.extend(set.rules.iter().map(|rule| rule.id.as_str()));
        released_alignments.extend(set.alignments.iter().map(|alignment| alignment.id.as_str()));
    }
    if !released_targets.is_empty() {
        let external_rule = draft.transition_rules.iter().any(|(id, rule)| {
            released_targets.contains(rule.target_id.as_str())
                && !released_rules.contains(id.as_str())
        });
        let external_alignment = draft
            .institutional_alignments
            .iter()
            .any(|(id, alignment)| {
                released_targets.contains(alignment.target_id.as_str())
                    && !released_alignments.contains(id.as_str())
            });
        let live_released_alignment =
            draft
                .institutional_alignments
                .iter()
                .any(|(id, alignment)| {
                    released_targets.contains(alignment.target_id.as_str())
                        && released_alignments.contains(id.as_str())
                        && (alignment.support_per_mille > 0
                            || alignment.enforcement_per_mille > 0
                            || alignment.access_grant_per_mille > 0
                            || alignment.authorized_actor.is_some())
                });
        let live_influence = draft
            .influence_edges
            .values()
            .any(|edge| released_targets.contains(edge.target_id.as_str()) && edge.active);
        let live_organization = draft.organizations.values().any(|organization| {
            released_targets.contains(organization.target_id.as_str()) && organization.active
        });
        let live_policy = draft
            .policies
            .values()
            .any(|policy| released_targets.contains(policy.target_id.as_str()));
        if external_rule
            || external_alignment
            || live_released_alignment
            || live_influence
            || live_organization
            || live_policy
        {
            return Err(invalid(
                "live society dependency blocks the release of a retired target",
            ));
        }
    }

    draft
        .transition_rules
        .retain(|rule_id, _| !inactive_rules.contains(rule_id.as_str()));
    let rules = &draft.transition_rules;
    draft
        .remainders
        .retain(|_, remainder| rules.contains_key(&remainder.rule_id));
    if !released_targets.is_empty() {
        draft
            .institutional_alignments
            .retain(|id, _| !released_alignments.contains(id.as_str()));
        draft
            .distributions
            .retain(|_, value| !released_targets.contains(value.target_id.as_str()));
        draft
            .aggregates
            .retain(|_, value| !released_targets.contains(value.target_id.as_str()));
        draft
            .mobilization_candidates
            .retain(|_, value| !released_targets.contains(value.target_id.as_str()));
        for projection in draft.projections.values_mut() {
            projection
                .entries
                .retain(|_, value| !released_targets.contains(value.target_id.as_str()));
        }
    }
    draft.canonicalize()?;
    draft.validate()
}
