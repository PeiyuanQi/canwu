use crate::model::{
    AssentBand, DispositionBucket, DispositionDistribution, DispositionProfile, InfluenceSource,
    MobilizationBand, MobilizationCandidate, OrganizationalTieBand, PracticeBand, ProjectionEntry,
    PublicAlignmentBand, SocietyAggregate, SocietyProjection, SocietyState, TransitionRemainder,
    TransitionRule, VisibilityBand, distribution_id, invalid, remainder_id,
};
use canwu_api::{CanwuError, SimTime};
use std::collections::{BTreeMap, BTreeSet};

const RATE_DENOMINATOR: u64 = 1_000_000;

#[derive(Clone, Copy, Debug, Default)]
struct TransitionSignals {
    influence: u16,
    institutional_support: u16,
    institutional_enforcement: u16,
    policy_support: u16,
    policy_coercion: u16,
    policy_disruption: u16,
}

#[derive(Clone, Copy, Debug, Default)]
struct SignalTotal {
    total: u64,
    count: u64,
}

impl SignalTotal {
    fn add(&mut self, value: u16) {
        self.total = self.total.saturating_add(u64::from(value));
        self.count = self.count.saturating_add(1);
    }

    fn combine(self, other: Self) -> Self {
        Self {
            total: self.total.saturating_add(other.total),
            count: self.count.saturating_add(other.count),
        }
    }

    fn average(self) -> u16 {
        self.total
            .checked_div(self.count)
            .and_then(|value| u16::try_from(value).ok())
            .unwrap_or(0)
    }
}

#[derive(Clone, Debug, Default)]
struct ScopedSignalIndex {
    global: BTreeMap<String, SignalTotal>,
    scoped: BTreeMap<(String, String), SignalTotal>,
}

impl ScopedSignalIndex {
    fn add(&mut self, target_id: &str, cohorts: &BTreeSet<String>, value: u16) {
        if cohorts.is_empty() {
            self.global
                .entry(target_id.to_owned())
                .or_default()
                .add(value);
            return;
        }
        for cohort_id in cohorts {
            self.scoped
                .entry((cohort_id.clone(), target_id.to_owned()))
                .or_default()
                .add(value);
        }
    }

    fn average(&self, cohort_id: &str, target_id: &str) -> u16 {
        self.global
            .get(target_id)
            .copied()
            .unwrap_or_default()
            .combine(
                self.scoped
                    .get(&(cohort_id.to_owned(), target_id.to_owned()))
                    .copied()
                    .unwrap_or_default(),
            )
            .average()
    }
}

#[derive(Clone, Debug, Default)]
struct TransitionSignalIndex {
    influence: BTreeMap<(String, String), u16>,
    institutional_support: ScopedSignalIndex,
    institutional_enforcement: ScopedSignalIndex,
    policy_support: ScopedSignalIndex,
    policy_coercion: ScopedSignalIndex,
    policy_disruption: ScopedSignalIndex,
}

struct ProjectionAggregateIndex<'a> {
    aggregates: &'a BTreeMap<String, SocietyAggregate>,
    by_cohort: BTreeMap<String, BTreeSet<String>>,
    by_target: BTreeMap<String, BTreeSet<String>>,
}

impl<'a> ProjectionAggregateIndex<'a> {
    fn build(aggregates: &'a BTreeMap<String, SocietyAggregate>) -> Self {
        let mut index = Self {
            aggregates,
            by_cohort: BTreeMap::new(),
            by_target: BTreeMap::new(),
        };
        for (id, aggregate) in aggregates {
            index
                .by_cohort
                .entry(aggregate.cohort_id.clone())
                .or_default()
                .insert(id.clone());
            index
                .by_target
                .entry(aggregate.target_id.clone())
                .or_default()
                .insert(id.clone());
        }
        index
    }

    fn candidate_ids(&self, observer: &crate::ObserverProfile) -> BTreeSet<String> {
        let cohort_ids = selected_ids(&self.by_cohort, &observer.cohorts);
        let target_ids = selected_ids(&self.by_target, &observer.targets);
        match (cohort_ids, target_ids) {
            (None, None) => self.aggregates.keys().cloned().collect(),
            (Some(ids), None) | (None, Some(ids)) => ids,
            (Some(cohorts), Some(targets)) => cohorts.intersection(&targets).cloned().collect(),
        }
    }
}

fn selected_ids(
    index: &BTreeMap<String, BTreeSet<String>>,
    selected: &BTreeSet<String>,
) -> Option<BTreeSet<String>> {
    if selected.is_empty() {
        return None;
    }
    Some(
        selected
            .iter()
            .filter_map(|key| index.get(key))
            .flat_map(BTreeSet::iter)
            .cloned()
            .collect(),
    )
}

impl TransitionSignalIndex {
    fn build(state: &SocietyState, organization_strengths: &BTreeMap<String, u16>) -> Self {
        let mut index = Self::default();
        for edge in state.influence_edges.values().filter(|edge| edge.active) {
            let source_strength = match &edge.source {
                InfluenceSource::Cohort(source) => cohort_support(state, source, &edge.target_id),
                InfluenceSource::Organization(source) => state
                    .organizations
                    .get(source)
                    .filter(|organization| organization.active)
                    .and_then(|_| organization_strengths.get(source))
                    .copied()
                    .unwrap_or(0),
                InfluenceSource::Institution(source) => state
                    .institutional_alignments
                    .get(source)
                    .filter(|alignment| alignment.target_id == edge.target_id)
                    .map_or(0, |alignment| alignment.support_per_mille),
            };
            let contribution = u32::from(source_strength)
                * u32::from(edge.reach_per_mille)
                * u32::from(edge.trust_per_mille)
                / 1_000_000;
            let entry = index
                .influence
                .entry((edge.target_cohort_id.clone(), edge.target_id.clone()))
                .or_default();
            *entry = entry
                .saturating_add(u16::try_from(contribution).unwrap_or(1_000))
                .min(1_000);
        }
        for alignment in state.institutional_alignments.values() {
            index.institutional_support.add(
                &alignment.target_id,
                &alignment.affected_cohorts,
                average_u16(
                    alignment.support_per_mille,
                    alignment.access_grant_per_mille,
                ),
            );
            index.institutional_enforcement.add(
                &alignment.target_id,
                &alignment.affected_cohorts,
                alignment.enforcement_per_mille,
            );
        }
        for policy in state.policies.values() {
            index.policy_support.add(
                &policy.target_id,
                &policy.affected_cohorts,
                average_u16(policy.support_per_mille, policy.legal_access_per_mille),
            );
            index.policy_coercion.add(
                &policy.target_id,
                &policy.affected_cohorts,
                policy.coercion_per_mille,
            );
            index.policy_disruption.add(
                &policy.target_id,
                &policy.affected_cohorts,
                average_three(
                    policy.censorship_per_mille,
                    policy.disruption_per_mille,
                    policy.material_penalty_per_mille,
                ),
            );
        }
        index
    }

    fn signals(&self, cohort_id: &str, target_id: &str) -> TransitionSignals {
        TransitionSignals {
            influence: self
                .influence
                .get(&(cohort_id.to_owned(), target_id.to_owned()))
                .copied()
                .unwrap_or(0),
            institutional_support: self.institutional_support.average(cohort_id, target_id),
            institutional_enforcement: self.institutional_enforcement.average(cohort_id, target_id),
            policy_support: self.policy_support.average(cohort_id, target_id),
            policy_coercion: self.policy_coercion.average(cohort_id, target_id),
            policy_disruption: self.policy_disruption.average(cohort_id, target_id),
        }
    }
}

/// Settles every configured transition once at `at` in canonical key order.
///
/// # Errors
///
/// Returns an error when the state is invalid or arithmetic cannot be
/// represented without violating population conservation.
pub fn settle_transitions(state: &mut SocietyState, at: SimTime) -> Result<bool, CanwuError> {
    state.canonicalize()?;
    state.validate()?;
    if state.last_transition_at.is_some_and(|last| at <= last) {
        return Ok(false);
    }

    let organization_strengths = organization_strengths(state);
    let signal_index = TransitionSignalIndex::build(state, &organization_strengths);
    let rules: Vec<_> = state.transition_rules.values().cloned().collect();
    let all_cohorts: Vec<_> = state.cohorts.keys().cloned().collect();
    let mut changed = false;

    for rule in rules {
        let cohort_ids: Vec<_> = if rule.affected_cohorts.is_empty() {
            all_cohorts.clone()
        } else {
            rule.affected_cohorts.iter().cloned().collect()
        };
        for cohort_id in cohort_ids {
            materialize_distribution(state, &cohort_id, &rule.target_id)?;
            let signals = signal_index.signals(&cohort_id, &rule.target_id);
            let rate = effective_rate(&rule, signals);
            let distribution_key = distribution_id(&cohort_id, &rule.target_id);
            let source_count = state
                .distributions
                .get(&distribution_key)
                .and_then(|distribution| {
                    distribution
                        .buckets
                        .iter()
                        .find(|bucket| bucket.profile == rule.from)
                })
                .map_or(0, |bucket| bucket.headcount);
            if source_count == 0 {
                continue;
            }

            let remainder_key = remainder_id(&rule.id, &cohort_id);
            let previous_remainder = state
                .remainders
                .get(&remainder_key)
                .map_or(0, |remainder| remainder.remainder);
            let numerator = u128::from(source_count)
                .checked_mul(u128::from(rate))
                .and_then(|value| value.checked_add(u128::from(previous_remainder)))
                .ok_or_else(|| invalid("transition numerator overflowed"))?;
            let transfer = u64::try_from(numerator / u128::from(RATE_DENOMINATOR))
                .map_err(|_| invalid("transition quantity exceeded u64"))?
                .min(source_count);
            let next_remainder = u64::try_from(numerator % u128::from(RATE_DENOMINATOR))
                .map_err(|_| invalid("transition remainder exceeded u64"))?;

            if transfer > 0 {
                let distribution = state
                    .distributions
                    .get_mut(&distribution_key)
                    .ok_or_else(|| invalid("materialized distribution disappeared"))?;
                transfer_between_profiles(distribution, rule.from, rule.to, transfer)?;
                changed = true;
            }
            if next_remainder != previous_remainder {
                state.remainders.insert(
                    remainder_key.clone(),
                    TransitionRemainder {
                        id: remainder_key,
                        rule_id: rule.id.clone(),
                        cohort_id: cohort_id.clone(),
                        remainder: next_remainder,
                    },
                );
                changed = true;
            }
        }
    }

    state.last_transition_at = Some(at);
    state.aggregates.clear();
    state.mobilization_candidates.clear();
    state.projections.clear();
    state.last_aggregation_at = None;
    state.last_mobilization_at = None;
    state.last_projection_at = None;
    state.canonicalize()?;
    state.validate()?;
    Ok(changed || state.last_transition_at == Some(at))
}

#[must_use]
pub fn compute_aggregates(state: &SocietyState) -> BTreeMap<String, SocietyAggregate> {
    state
        .distributions
        .iter()
        .map(|(id, distribution)| {
            let mut aggregate = SocietyAggregate {
                cohort_id: distribution.cohort_id.clone(),
                target_id: distribution.target_id.clone(),
                ..SocietyAggregate::default()
            };
            for bucket in &distribution.buckets {
                aggregate.headcount = aggregate.headcount.saturating_add(bucket.headcount);
                if bucket.profile.awareness == crate::AwarenessBand::Aware {
                    aggregate.aware = aggregate.aware.saturating_add(bucket.headcount);
                }
                if bucket.profile.assent >= AssentBand::Sympathetic {
                    aggregate.assenting = aggregate.assenting.saturating_add(bucket.headcount);
                }
                if bucket.profile.practice > PracticeBand::None {
                    aggregate.practicing = aggregate.practicing.saturating_add(bucket.headcount);
                }
                if bucket.profile.public_alignment >= PublicAlignmentBand::Conforming {
                    aggregate.publicly_aligned =
                        aggregate.publicly_aligned.saturating_add(bucket.headcount);
                }
                if bucket.profile.organizational_tie > OrganizationalTieBand::None {
                    aggregate.organizationally_tied = aggregate
                        .organizationally_tied
                        .saturating_add(bucket.headcount);
                }
                if bucket.profile.mobilization == MobilizationBand::Active {
                    aggregate.mobilized = aggregate.mobilized.saturating_add(bucket.headcount);
                }
                if bucket.profile.visibility != VisibilityBand::Public {
                    aggregate.hidden = aggregate.hidden.saturating_add(bucket.headcount);
                }
            }
            (id.clone(), aggregate)
        })
        .collect()
}

pub(crate) fn compute_mobilization_candidates(
    state: &SocietyState,
    at: SimTime,
) -> BTreeMap<String, MobilizationCandidate> {
    let aggregates = compute_aggregates(state);
    let strengths = organization_strengths(state);
    let mut organization_capacity = BTreeMap::<String, u16>::new();
    for organization in state
        .organizations
        .values()
        .filter(|organization| organization.active)
    {
        let strength = strengths.get(&organization.id).copied().unwrap_or(0);
        let entry = organization_capacity
            .entry(organization.target_id.clone())
            .or_default();
        *entry = (*entry).max(strength);
    }
    let mut coercion = ScopedSignalIndex::default();
    for policy in state.policies.values() {
        coercion.add(
            &policy.target_id,
            &policy.affected_cohorts,
            policy.coercion_per_mille,
        );
    }
    aggregates
        .into_iter()
        .filter_map(|(id, aggregate)| {
            if aggregate.mobilized == 0 {
                return None;
            }
            let organization_capacity = organization_capacity
                .get(&aggregate.target_id)
                .copied()
                .unwrap_or(0);
            let coercion = coercion.average(&aggregate.cohort_id, &aggregate.target_id);
            Some((
                id.clone(),
                MobilizationCandidate {
                    id,
                    cohort_id: aggregate.cohort_id,
                    target_id: aggregate.target_id,
                    mobilized_headcount: aggregate.mobilized,
                    organization_capacity_per_mille: organization_capacity,
                    coercion_per_mille: coercion,
                    observed_at: at,
                },
            ))
        })
        .collect()
}

pub(crate) fn compute_projections(
    state: &SocietyState,
    at: SimTime,
) -> BTreeMap<String, SocietyProjection> {
    let aggregates = compute_aggregates(state);
    let aggregate_index = ProjectionAggregateIndex::build(&aggregates);
    let mut concealment = BTreeMap::<String, SignalTotal>::new();
    for organization in state
        .organizations
        .values()
        .filter(|organization| organization.active)
    {
        concealment
            .entry(organization.target_id.clone())
            .or_default()
            .add(organization.concealment_per_mille);
    }
    state
        .observer_profiles
        .iter()
        .map(|(actor_key, observer)| {
            let mut projection = SocietyProjection {
                actor: observer.actor,
                observed_at: at,
                entries: BTreeMap::new(),
            };
            for id in aggregate_index.candidate_ids(observer) {
                let aggregate = &aggregates[&id];
                let concealment = concealment
                    .get(&aggregate.target_id)
                    .copied()
                    .unwrap_or_default()
                    .average();
                let private_detection = mul_per_mille(
                    observer.private_detection_per_mille,
                    1_000_u16.saturating_sub(concealment),
                );
                projection.entries.insert(
                    id,
                    ProjectionEntry {
                        cohort_id: aggregate.cohort_id.clone(),
                        target_id: aggregate.target_id.clone(),
                        estimated_publicly_aligned: estimate_count(
                            aggregate.publicly_aligned,
                            aggregate.headcount,
                            observer.public_detection_per_mille,
                            observer.false_positive_per_mille,
                        ),
                        estimated_organizationally_tied: estimate_count(
                            aggregate.organizationally_tied,
                            aggregate.headcount,
                            private_detection,
                            observer.false_positive_per_mille,
                        ),
                        estimated_mobilized: estimate_count(
                            aggregate.mobilized,
                            aggregate.headcount,
                            private_detection,
                            observer.false_positive_per_mille,
                        ),
                        confidence_per_mille: observer.confidence_per_mille,
                    },
                );
            }
            (actor_key.clone(), projection)
        })
        .collect()
}

fn materialize_distribution(
    state: &mut SocietyState,
    cohort_id: &str,
    target_id: &str,
) -> Result<(), CanwuError> {
    let id = distribution_id(cohort_id, target_id);
    if state.distributions.contains_key(&id) {
        return Ok(());
    }
    let cohort = state
        .cohorts
        .get(cohort_id)
        .ok_or_else(|| invalid(format!("unknown cohort {cohort_id}")))?;
    let target = state
        .targets
        .get(target_id)
        .ok_or_else(|| invalid(format!("unknown target {target_id}")))?;
    state.distributions.insert(
        id.clone(),
        DispositionDistribution {
            id,
            cohort_id: cohort_id.to_owned(),
            target_id: target_id.to_owned(),
            buckets: vec![DispositionBucket {
                profile: target.neutral_profile,
                headcount: cohort.headcount,
            }],
        },
    );
    Ok(())
}

fn transfer_between_profiles(
    distribution: &mut DispositionDistribution,
    from: DispositionProfile,
    to: DispositionProfile,
    quantity: u64,
) -> Result<(), CanwuError> {
    if from == to || quantity == 0 {
        return Ok(());
    }
    let mut buckets: BTreeMap<_, _> = distribution
        .buckets
        .iter()
        .map(|bucket| (bucket.profile, bucket.headcount))
        .collect();
    let source = buckets.get_mut(&from).ok_or_else(|| {
        invalid(format!(
            "distribution {} lacks source profile",
            distribution.id
        ))
    })?;
    *source = source
        .checked_sub(quantity)
        .ok_or_else(|| invalid("transition exceeds source profile population"))?;
    let target = buckets.entry(to).or_default();
    *target = target
        .checked_add(quantity)
        .ok_or_else(|| invalid("transition target profile overflowed"))?;
    distribution.buckets = buckets
        .into_iter()
        .filter(|(_, headcount)| *headcount > 0)
        .map(|(profile, headcount)| DispositionBucket { profile, headcount })
        .collect();
    Ok(())
}

fn effective_rate(rule: &TransitionRule, signals: TransitionSignals) -> u64 {
    let mut rate = i64::from(rule.base_rate_per_million);
    for (signal, weight) in [
        (signals.influence, rule.weights.influence),
        (
            signals.institutional_support,
            rule.weights.institutional_support,
        ),
        (
            signals.institutional_enforcement,
            rule.weights.institutional_enforcement,
        ),
        (signals.policy_support, rule.weights.policy_support),
        (signals.policy_coercion, rule.weights.policy_coercion),
        (signals.policy_disruption, rule.weights.policy_disruption),
    ] {
        rate += i64::from(signal) * i64::from(weight) / 1_000;
    }
    u64::try_from(rate.clamp(0, 1_000_000)).unwrap_or_default()
}

fn cohort_support(state: &SocietyState, cohort_id: &str, target_id: &str) -> u16 {
    let id = distribution_id(cohort_id, target_id);
    let Some(distribution) = state.distributions.get(&id) else {
        return 0;
    };
    let Some(cohort) = state.cohorts.get(cohort_id) else {
        return 0;
    };
    let supporters = distribution
        .buckets
        .iter()
        .filter(|bucket| {
            bucket.profile.assent >= AssentBand::Sympathetic
                || bucket.profile.practice == PracticeBand::Regular
                || bucket.profile.organizational_tie >= OrganizationalTieBand::Member
        })
        .fold(0_u64, |total, bucket| {
            total.saturating_add(bucket.headcount)
        });
    ratio_per_mille(supporters, cohort.headcount)
}

fn organization_strengths(state: &SocietyState) -> BTreeMap<String, u16> {
    let base_strengths: BTreeMap<_, _> = state
        .organizations
        .iter()
        .map(|(id, organization)| {
            (
                id.clone(),
                if organization.active {
                    organization.base_reach_per_mille
                } else {
                    0
                },
            )
        })
        .collect();
    let mut strengths = base_strengths.clone();
    for _ in 0..state.topology_passes {
        let previous = strengths.clone();
        let mut next = base_strengths.clone();
        for relation in state.organization_relations.values() {
            let endpoints_active = state
                .organizations
                .get(&relation.source_organization_id)
                .is_some_and(|organization| organization.active)
                && state
                    .organizations
                    .get(&relation.target_organization_id)
                    .is_some_and(|organization| organization.active);
            if !endpoints_active {
                continue;
            }
            let source = previous
                .get(&relation.source_organization_id)
                .copied()
                .unwrap_or(0);
            let contribution = mul_per_mille(source, relation.strength_per_mille);
            let entry = next
                .entry(relation.target_organization_id.clone())
                .or_default();
            *entry = entry.saturating_add(contribution).min(1_000);
        }
        if next == strengths {
            break;
        }
        strengths = next;
    }
    strengths
}

fn estimate_count(true_count: u64, total: u64, detection: u16, false_positive: u16) -> u64 {
    let missed = total.saturating_sub(true_count);
    let numerator = u128::from(true_count) * u128::from(detection)
        + u128::from(missed) * u128::from(false_positive);
    u64::try_from(numerator / 1_000)
        .unwrap_or(u64::MAX)
        .min(total)
}

fn ratio_per_mille(numerator: u64, denominator: u64) -> u16 {
    if denominator == 0 {
        return 0;
    }
    let value = u128::from(numerator) * 1_000 / u128::from(denominator);
    u16::try_from(value.min(1_000)).unwrap_or(1_000)
}

fn mul_per_mille(left: u16, right: u16) -> u16 {
    u16::try_from(u32::from(left) * u32::from(right) / 1_000).unwrap_or(1_000)
}

fn average_u16(left: u16, right: u16) -> u16 {
    u16::try_from(u32::midpoint(u32::from(left), u32::from(right))).unwrap_or(1_000)
}

fn average_three(first: u16, second: u16, third: u16) -> u16 {
    u16::try_from((u32::from(first) + u32::from(second) + u32::from(third)) / 3).unwrap_or(1_000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AffiliationTarget, InstitutionalAlignment, OrganizationNode, PolicyPressure,
        SocialInfluenceEdge, SocietyCohort,
    };
    use canwu_api::{EntityRef, GovernmentId, TerritoryId};

    #[test]
    fn active_signal_index_grows_with_sparse_inputs_not_cross_products() {
        const ACTIVE_PAIRS: usize = 128;
        let mut state = SocietyState::default();
        for index in 0..ACTIVE_PAIRS {
            let cohort_id = format!("cohort-{index:03}");
            let target_id = format!("target-{index:03}");
            let organization_id = format!("organization-{index:03}");
            let edge_id = format!("edge-{index:03}");
            let alignment_id = format!("alignment-{index:03}");
            let policy_id = format!("policy-{index:03}");
            let affected_cohorts = BTreeSet::from([cohort_id.clone()]);
            state.cohorts.insert(
                cohort_id.clone(),
                SocietyCohort {
                    id: cohort_id.clone(),
                    territory: TerritoryId::new(1),
                    headcount: 1,
                    classification: BTreeMap::new(),
                },
            );
            state.targets.insert(
                target_id.clone(),
                AffiliationTarget {
                    id: target_id.clone(),
                    parent: None,
                    neutral_profile: DispositionProfile::neutral(),
                    metadata: BTreeMap::new(),
                },
            );
            state.organizations.insert(
                organization_id.clone(),
                OrganizationNode {
                    id: organization_id.clone(),
                    target_id: target_id.clone(),
                    base_reach_per_mille: 1_000,
                    concealment_per_mille: 0,
                    active: true,
                },
            );
            state.influence_edges.insert(
                edge_id.clone(),
                SocialInfluenceEdge {
                    id: edge_id,
                    source: InfluenceSource::Organization(organization_id),
                    target_cohort_id: cohort_id.clone(),
                    target_id: target_id.clone(),
                    channel: "sparse".to_owned(),
                    reach_per_mille: 1_000,
                    trust_per_mille: 1_000,
                    active: true,
                },
            );
            state.institutional_alignments.insert(
                alignment_id.clone(),
                InstitutionalAlignment {
                    id: alignment_id,
                    institution: EntityRef::Government(GovernmentId::new(1)),
                    target_id: target_id.clone(),
                    affected_cohorts: affected_cohorts.clone(),
                    support_per_mille: 100,
                    enforcement_per_mille: 100,
                    access_grant_per_mille: 100,
                    authorized_actor: None,
                    last_decision_version: 0,
                },
            );
            state.policies.insert(
                policy_id.clone(),
                PolicyPressure {
                    id: policy_id,
                    target_id,
                    affected_cohorts,
                    support_per_mille: 100,
                    legal_access_per_mille: 100,
                    surveillance_per_mille: 0,
                    censorship_per_mille: 100,
                    coercion_per_mille: 100,
                    material_penalty_per_mille: 100,
                    disruption_per_mille: 100,
                    migration_pressure_per_mille: 0,
                },
            );
        }

        let strengths = organization_strengths(&state);
        let index = TransitionSignalIndex::build(&state, &strengths);
        assert_eq!(index.influence.len(), ACTIVE_PAIRS);
        assert_eq!(index.institutional_support.scoped.len(), ACTIVE_PAIRS);
        assert_eq!(index.institutional_enforcement.scoped.len(), ACTIVE_PAIRS);
        assert_eq!(index.policy_support.scoped.len(), ACTIVE_PAIRS);
        assert_eq!(index.policy_coercion.scoped.len(), ACTIVE_PAIRS);
        assert_eq!(index.policy_disruption.scoped.len(), ACTIVE_PAIRS);
        assert!(index.institutional_support.global.is_empty());
        assert!(index.policy_support.global.is_empty());
    }

    #[test]
    fn narrow_observer_projection_candidates_grow_with_visible_pairs() {
        const PAIRS: usize = 128;
        let aggregates: BTreeMap<_, _> = (0..PAIRS)
            .map(|index| {
                let cohort_id = format!("cohort-{index:03}");
                let target_id = format!("target-{index:03}");
                (
                    distribution_id(&cohort_id, &target_id),
                    SocietyAggregate {
                        cohort_id,
                        target_id,
                        headcount: 1,
                        ..SocietyAggregate::default()
                    },
                )
            })
            .collect();
        let index = ProjectionAggregateIndex::build(&aggregates);
        let ids = canwu_api::Canwu::demo_ids();
        let total_candidates: usize = (0..PAIRS)
            .map(|pair| crate::ObserverProfile {
                actor: ids.observer,
                cohorts: BTreeSet::from([format!("cohort-{pair:03}")]),
                targets: BTreeSet::from([format!("target-{pair:03}")]),
                public_detection_per_mille: 0,
                private_detection_per_mille: 0,
                false_positive_per_mille: 0,
                confidence_per_mille: 0,
            })
            .map(|observer| index.candidate_ids(&observer).len())
            .sum();
        assert_eq!(total_candidates, PAIRS);
    }
}
