use crate::model::{CompiledCulturePlan, CultureDefinition};
use canwu_api::{CanwuError, ErrorCode, SimTime};
use canwu_society::{
    AffiliationTarget, InstitutionalAlignment, SocietyCohort, SocietyState, TransitionRule,
    distribution_id,
};
use std::collections::BTreeSet;

fn culture_binding_id(plan: &CompiledCulturePlan, source_id: &str) -> String {
    format!(
        "culture:v1:{}:{}:{}:{}",
        plan.definition_id.len(),
        plan.definition_id,
        source_id.len(),
        source_id
    )
}

#[derive(Default)]
struct CompiledBindingIds {
    rules: BTreeSet<String>,
    alignments: BTreeSet<String>,
}

fn compiled_binding_ids_for_targets<'a>(
    plan: &CompiledCulturePlan,
    target_ids: impl IntoIterator<Item = &'a str>,
) -> Result<CompiledBindingIds, CanwuError> {
    let mut ids = CompiledBindingIds::default();
    for target_id in target_ids {
        let target = plan
            .target_by_id
            .get(target_id)
            .ok_or_else(|| invalid(format!("unknown compiled culture target {target_id}")))?;
        for transition_key in plan.transitions_by_target.get(target).into_iter().flatten() {
            let transition = plan
                .transitions
                .get(transition_key.get() as usize)
                .ok_or_else(|| invalid("compiled culture transition key is invalid"))?;
            ids.rules
                .insert(culture_binding_id(plan, &transition.source_id));
        }
        for institution_key in plan
            .institutions_by_target
            .get(target)
            .into_iter()
            .flatten()
        {
            let institution = plan
                .institutions
                .get(institution_key.get() as usize)
                .ok_or_else(|| invalid("compiled culture institution key is invalid"))?;
            ids.alignments
                .insert(culture_binding_id(plan, &institution.source_id));
        }
    }
    Ok(ids)
}

/// Installs the compiled culture structure into the generic society state.
///
/// The adapter only adds reusable society mechanics. It never creates a dense
/// cohort/target cross-product and never writes legal or other downstream
/// domain state.
#[allow(clippy::too_many_lines)]
pub fn install_into_society(
    plan: &CompiledCulturePlan,
    state: &mut SocietyState,
) -> Result<(), CanwuError> {
    let mut draft = state.clone();
    install_into_society_draft(plan, &mut draft)?;
    *state = draft;
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn install_into_society_draft(
    plan: &CompiledCulturePlan,
    state: &mut SocietyState,
) -> Result<(), CanwuError> {
    for target in &plan.targets {
        if let Some(existing) = state.targets.get(&target.source_id) {
            if existing.neutral_profile != target.neutral_profile
                || existing.metadata != target.metadata
                || existing.parent
                    != target.parent.as_ref().map(|parent| {
                        plan.targets
                            .get(parent.get() as usize)
                            .map(|candidate| candidate.source_id.clone())
                            .unwrap_or_default()
                    })
            {
                return Err(invalid(format!(
                    "society target {} conflicts with compiled culture target",
                    target.source_id
                )));
            }
            continue;
        }
        state.targets.insert(
            target.source_id.clone(),
            AffiliationTarget {
                id: target.source_id.clone(),
                parent: target
                    .parent
                    .as_ref()
                    .map(|parent| plan.targets[parent.get() as usize].source_id.clone()),
                neutral_profile: target.neutral_profile,
                metadata: target.metadata.clone(),
            },
        );
    }

    for cohort in &plan.cohorts {
        if let Some(existing) = state.cohorts.get(&cohort.source_id) {
            if existing.territory != cohort.territory
                || existing.headcount != cohort.headcount
                || existing.classification != cohort.classification
            {
                return Err(invalid(format!(
                    "society cohort {} conflicts with compiled culture cohort",
                    cohort.source_id
                )));
            }
            continue;
        }
        state.cohorts.insert(
            cohort.source_id.clone(),
            SocietyCohort {
                id: cohort.source_id.clone(),
                territory: cohort.territory,
                headcount: cohort.headcount,
                classification: cohort.classification.clone(),
            },
        );
    }

    for transition in &plan.transitions {
        let target_id = &plan.targets[transition.target.get() as usize].source_id;
        let rule_id = culture_binding_id(plan, &transition.source_id);
        let rule = TransitionRule {
            id: rule_id.clone(),
            target_id: target_id.clone(),
            affected_cohorts: transition
                .affected_cohorts
                .iter()
                .map(|cohort| plan.cohorts[cohort.get() as usize].source_id.clone())
                .collect::<BTreeSet<_>>(),
            from: transition.from,
            to: transition.to,
            base_rate_per_million: transition.base_rate_per_million,
            weights: transition.weights,
        };
        if let Some(existing) = state.transition_rules.get(&rule_id) {
            if existing != &rule {
                return Err(invalid(format!(
                    "society transition {rule_id} conflicts with compiled culture transition"
                )));
            }
        } else {
            state.transition_rules.insert(rule_id, rule);
        }
    }

    for institution in &plan.institutions {
        let target_id = &plan.targets[institution.target.get() as usize].source_id;
        let alignment_id = culture_binding_id(plan, &institution.source_id);
        let affected_cohorts = institution
            .affected_cohorts
            .iter()
            .map(|cohort| plan.cohorts[cohort.get() as usize].source_id.clone())
            .collect::<BTreeSet<_>>();
        let alignment = InstitutionalAlignment {
            id: alignment_id.clone(),
            institution: institution.institution.clone(),
            target_id: target_id.clone(),
            affected_cohorts,
            support_per_mille: 0,
            enforcement_per_mille: 0,
            access_grant_per_mille: 0,
            authorized_actor: None,
            last_decision_version: 0,
        };
        if let Some(existing) = state.institutional_alignments.get(&alignment_id) {
            if existing.institution != alignment.institution
                || existing.target_id != alignment.target_id
                || existing.affected_cohorts != alignment.affected_cohorts
            {
                return Err(invalid(format!(
                    "society alignment {alignment_id} conflicts with compiled culture binding"
                )));
            }
        } else {
            state
                .institutional_alignments
                .insert(alignment_id, alignment);
        }
    }

    state.canonicalize()?;
    state.validate()
}

/// Atomically applies culture lifecycle eligibility to generic society state.
///
/// Dormant targets stop running transition rules. Retired targets additionally
/// release target-scoped dynamic and derived society data; the affiliation
/// target catalog remains so historical references and tombstones stay valid.
/// Reactivating a target and calling this function reinstalls its compiled
/// rules, with distributions materialized lazily by `canwu-society`.
#[allow(clippy::too_many_lines)]
pub fn synchronize_society_lifecycle(
    plan: &CompiledCulturePlan,
    runtime: &crate::CultureRuntime,
    state: &mut SocietyState,
) -> Result<(), CanwuError> {
    let mut draft = state.clone();
    synchronize_society_lifecycle_draft(plan, runtime, &mut draft)?;
    *state = draft;
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn synchronize_society_lifecycle_draft(
    plan: &CompiledCulturePlan,
    runtime: &crate::CultureRuntime,
    draft: &mut SocietyState,
) -> Result<(), CanwuError> {
    if runtime.plan_hash() != plan.content_hash {
        return Err(invalid(
            "culture runtime and compiled plan hashes do not match",
        ));
    }
    install_active_bindings_draft(plan, runtime, draft)?;

    let inactive_targets = runtime
        .state()
        .targets()
        .values()
        .filter(|target| target.state != crate::CultureLifecycle::Active)
        .map(|target| target.target_id.as_str())
        .collect::<BTreeSet<_>>();
    let retired_targets = runtime
        .state()
        .targets()
        .values()
        .filter(|target| target.state == crate::CultureLifecycle::Retired)
        .map(|target| target.target_id.as_str())
        .collect::<BTreeSet<_>>();
    let inactive_bindings =
        compiled_binding_ids_for_targets(plan, inactive_targets.iter().copied())?;
    let retired_bindings = compiled_binding_ids_for_targets(plan, retired_targets.iter().copied())?;
    let external_rule = draft.transition_rules.iter().any(|(id, rule)| {
        retired_targets.contains(rule.target_id.as_str()) && !retired_bindings.rules.contains(id)
    });
    let external_alignment = draft
        .institutional_alignments
        .iter()
        .any(|(id, alignment)| {
            retired_targets.contains(alignment.target_id.as_str())
                && !retired_bindings.alignments.contains(id)
        });
    let live_culture_alignment = draft
        .institutional_alignments
        .iter()
        .any(|(id, alignment)| {
            retired_targets.contains(alignment.target_id.as_str())
                && retired_bindings.alignments.contains(id)
                && (alignment.support_per_mille > 0
                    || alignment.enforcement_per_mille > 0
                    || alignment.access_grant_per_mille > 0
                    || alignment.authorized_actor.is_some())
        });
    let live_influence = draft
        .influence_edges
        .values()
        .any(|edge| retired_targets.contains(edge.target_id.as_str()) && edge.active);
    let live_organization = draft.organizations.values().any(|organization| {
        retired_targets.contains(organization.target_id.as_str()) && organization.active
    });
    let live_policy = draft
        .policies
        .values()
        .any(|policy| retired_targets.contains(policy.target_id.as_str()));
    if external_rule
        || external_alignment
        || live_culture_alignment
        || live_influence
        || live_organization
        || live_policy
    {
        return Err(invalid(
            "live society dependency blocks culture target retirement",
        ));
    }

    draft
        .transition_rules
        .retain(|rule_id, _| !inactive_bindings.rules.contains(rule_id));
    draft
        .remainders
        .retain(|_, remainder| draft.transition_rules.contains_key(&remainder.rule_id));

    draft
        .institutional_alignments
        .retain(|id, _| !retired_bindings.alignments.contains(id));
    draft
        .distributions
        .retain(|_, distribution| !retired_targets.contains(distribution.target_id.as_str()));
    draft
        .aggregates
        .retain(|_, aggregate| !retired_targets.contains(aggregate.target_id.as_str()));
    draft
        .mobilization_candidates
        .retain(|_, candidate| !retired_targets.contains(candidate.target_id.as_str()));
    for projection in draft.projections.values_mut() {
        projection
            .entries
            .retain(|_, entry| !retired_targets.contains(entry.target_id.as_str()));
    }

    draft.canonicalize()?;
    draft.validate()?;
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn install_active_bindings_draft(
    plan: &CompiledCulturePlan,
    runtime: &crate::CultureRuntime,
    state: &mut SocietyState,
) -> Result<(), CanwuError> {
    for target_id in runtime.active_target_ids() {
        install_target_bindings_draft(plan, target_id, state)?;
    }
    Ok(())
}

fn install_target_bindings_draft(
    plan: &CompiledCulturePlan,
    target_id: &str,
    state: &mut SocietyState,
) -> Result<(), CanwuError> {
    let target = plan
        .target_by_id
        .get(target_id)
        .ok_or_else(|| invalid(format!("unknown compiled culture target {target_id}")))?;
    for transition_key in plan.transitions_by_target.get(target).into_iter().flatten() {
        let transition = plan
            .transitions
            .get(transition_key.get() as usize)
            .ok_or_else(|| invalid("compiled culture transition key is invalid"))?;
        let rule_id = culture_binding_id(plan, &transition.source_id);
        let rule = TransitionRule {
            id: rule_id.clone(),
            target_id: target_id.to_owned(),
            affected_cohorts: transition
                .affected_cohorts
                .iter()
                .filter_map(|cohort| plan.cohorts.get(cohort.get() as usize))
                .map(|cohort| cohort.source_id.clone())
                .collect(),
            from: transition.from,
            to: transition.to,
            base_rate_per_million: transition.base_rate_per_million,
            weights: transition.weights,
        };
        if let Some(existing) = state.transition_rules.get(&rule_id) {
            if existing != &rule {
                return Err(invalid(format!(
                    "society transition {rule_id} conflicts with compiled culture transition"
                )));
            }
        } else {
            state.transition_rules.insert(rule_id, rule);
        }
    }
    for institution_key in plan
        .institutions_by_target
        .get(target)
        .into_iter()
        .flatten()
    {
        let institution = plan
            .institutions
            .get(institution_key.get() as usize)
            .ok_or_else(|| invalid("compiled culture institution key is invalid"))?;
        let alignment_id = culture_binding_id(plan, &institution.source_id);
        let affected_cohorts = institution
            .affected_cohorts
            .iter()
            .filter_map(|cohort| plan.cohorts.get(cohort.get() as usize))
            .map(|cohort| cohort.source_id.clone())
            .collect::<BTreeSet<_>>();
        if let Some(existing) = state.institutional_alignments.get(&alignment_id) {
            if existing.institution != institution.institution
                || existing.target_id != target_id
                || existing.affected_cohorts != affected_cohorts
            {
                return Err(invalid(format!(
                    "society alignment {alignment_id} conflicts with compiled culture binding"
                )));
            }
        } else {
            state.institutional_alignments.insert(
                alignment_id.clone(),
                InstitutionalAlignment {
                    id: alignment_id,
                    institution: institution.institution.clone(),
                    target_id: target_id.to_owned(),
                    affected_cohorts,
                    support_per_mille: 0,
                    enforcement_per_mille: 0,
                    access_grant_per_mille: 0,
                    authorized_actor: None,
                    last_decision_version: 0,
                },
            );
        }
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn apply_society_lifecycle_delta_draft(
    plan: &CompiledCulturePlan,
    transitions: &[crate::LifecycleTransition],
    draft: &mut SocietyState,
) -> Result<(), CanwuError> {
    let mut inactive_targets = BTreeSet::new();
    let mut retired_targets = BTreeSet::new();
    let mut reactivated_targets = BTreeSet::new();
    for transition in transitions {
        match transition.kind {
            crate::LifecycleTransitionKind::BecameDormant => {
                inactive_targets.insert(transition.target_id.as_str());
            }
            crate::LifecycleTransitionKind::Retired => {
                inactive_targets.insert(transition.target_id.as_str());
                retired_targets.insert(transition.target_id.as_str());
            }
            crate::LifecycleTransitionKind::Reactivated => {
                reactivated_targets.insert(transition.target_id.as_str());
            }
        }
    }
    for target_id in reactivated_targets {
        install_target_bindings_draft(plan, target_id, draft)?;
    }

    let inactive_bindings =
        compiled_binding_ids_for_targets(plan, inactive_targets.iter().copied())?;
    let retired_bindings = compiled_binding_ids_for_targets(plan, retired_targets.iter().copied())?;
    if !retired_targets.is_empty() {
        let external_rule = draft.transition_rules.iter().any(|(id, rule)| {
            retired_targets.contains(rule.target_id.as_str())
                && !retired_bindings.rules.contains(id)
        });
        let external_alignment = draft
            .institutional_alignments
            .iter()
            .any(|(id, alignment)| {
                retired_targets.contains(alignment.target_id.as_str())
                    && !retired_bindings.alignments.contains(id)
            });
        let live_culture_alignment =
            draft
                .institutional_alignments
                .iter()
                .any(|(id, alignment)| {
                    retired_targets.contains(alignment.target_id.as_str())
                        && retired_bindings.alignments.contains(id)
                        && (alignment.support_per_mille > 0
                            || alignment.enforcement_per_mille > 0
                            || alignment.access_grant_per_mille > 0
                            || alignment.authorized_actor.is_some())
                });
        let live_influence = draft
            .influence_edges
            .values()
            .any(|edge| retired_targets.contains(edge.target_id.as_str()) && edge.active);
        let live_organization = draft.organizations.values().any(|organization| {
            retired_targets.contains(organization.target_id.as_str()) && organization.active
        });
        let live_policy = draft
            .policies
            .values()
            .any(|policy| retired_targets.contains(policy.target_id.as_str()));
        if external_rule
            || external_alignment
            || live_culture_alignment
            || live_influence
            || live_organization
            || live_policy
        {
            return Err(invalid(
                "live society dependency blocks culture target retirement",
            ));
        }
    }

    draft
        .transition_rules
        .retain(|rule_id, _| !inactive_bindings.rules.contains(rule_id));
    draft
        .remainders
        .retain(|_, remainder| draft.transition_rules.contains_key(&remainder.rule_id));
    if !retired_targets.is_empty() {
        draft
            .institutional_alignments
            .retain(|id, _| !retired_bindings.alignments.contains(id));
        draft
            .distributions
            .retain(|_, value| !retired_targets.contains(value.target_id.as_str()));
        draft
            .aggregates
            .retain(|_, value| !retired_targets.contains(value.target_id.as_str()));
        draft
            .mobilization_candidates
            .retain(|_, value| !retired_targets.contains(value.target_id.as_str()));
        for projection in draft.projections.values_mut() {
            projection
                .entries
                .retain(|_, value| !retired_targets.contains(value.target_id.as_str()));
        }
    }
    draft.canonicalize()?;
    draft.validate()
}

/// Atomically settles culture lifecycle state and synchronizes society state.
///
/// This is the preferred host boundary helper. If lifecycle settlement or a
/// live society dependency rejects retirement, neither caller-owned state is
/// changed. The returned transitions and resulting culture record still need
/// to be persisted by the host's authoritative boundary transaction.
pub fn settle_culture_society_boundary(
    plan: &CompiledCulturePlan,
    runtime: &mut crate::CultureRuntime,
    society: &mut SocietyState,
    at: SimTime,
    observations: &std::collections::BTreeMap<String, crate::LifecycleObservation>,
) -> Result<Vec<crate::LifecycleTransition>, CanwuError> {
    if runtime.plan_hash() != plan.content_hash {
        return Err(invalid(
            "culture runtime and compiled plan hashes do not match",
        ));
    }
    let delta = runtime.prepare_boundary(at, observations)?;
    let transitions = delta.transitions().to_vec();
    if transitions.is_empty() {
        runtime.apply_boundary_delta(delta);
        return Ok(transitions);
    }
    let mut society_draft = society.clone();
    apply_society_lifecycle_delta_draft(plan, delta.transitions(), &mut society_draft)?;
    runtime.apply_boundary_delta(delta);
    *society = society_draft;
    Ok(transitions)
}

/// Installs a definition after compiling it. This is the ergonomic entry point
/// for content loaders that do not need to retain the intermediate definition.
pub fn install_definition_into_society(
    definition: &CultureDefinition,
    state: &mut SocietyState,
) -> Result<CompiledCulturePlan, CanwuError> {
    let plan = crate::compile_culture(definition)?;
    install_into_society(&plan, state)?;
    Ok(plan)
}

/// Returns the canonical society distribution identity for a compiled pair.
pub fn society_distribution_id(
    plan: &CompiledCulturePlan,
    cohort: crate::CohortKey,
    target: crate::TargetKey,
) -> Result<String, CanwuError> {
    let cohort = plan
        .cohorts
        .get(cohort.get() as usize)
        .ok_or_else(|| invalid("unknown compiled culture cohort key"))?;
    let target = plan
        .targets
        .get(target.get() as usize)
        .ok_or_else(|| invalid("unknown compiled culture target key"))?;
    Ok(distribution_id(&cohort.source_id, &target.source_id))
}

fn invalid(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidDomainRecord, message)
}
