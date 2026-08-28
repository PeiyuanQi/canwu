use crate::model::{
    CULTURE_PLAN_HASH_DOMAIN, CULTURE_PLAN_VERSION, ChannelKey, CohortKey, CompiledChannel,
    CompiledCohort, CompiledCulturePlan, CompiledEffect, CompiledInstitutionBinding,
    CompiledTarget, CompiledTransition, CultureDefinition, EffectKey, InstitutionKey, TargetKey,
    TransitionKey, validate_definition,
};
use canwu_api::{CanwuError, SimTime, canonical_hash};
use std::collections::BTreeMap;

/// Compiles a validated content definition into the deterministic runtime plan.
///
/// The compiler sorts all authored collections by their stable source IDs,
/// assigns dense numeric keys, and builds reverse indexes used by dirty-set
/// settlement. Strings remain at the authoring boundary only.
#[allow(clippy::cast_possible_truncation, clippy::too_many_lines)]
pub fn compile_culture(definition: &CultureDefinition) -> Result<CompiledCulturePlan, CanwuError> {
    validate_definition(definition)?;
    let mut canonical_definition = definition.clone();
    canonical_definition
        .targets
        .sort_by(|left, right| left.id.cmp(&right.id));
    canonical_definition
        .cohorts
        .sort_by(|left, right| left.id.cmp(&right.id));
    canonical_definition
        .channels
        .sort_by(|left, right| left.id.cmp(&right.id));
    canonical_definition
        .transitions
        .sort_by(|left, right| left.id.cmp(&right.id));
    canonical_definition
        .effects
        .sort_by(|left, right| left.id.cmp(&right.id));
    canonical_definition
        .institutions
        .sort_by(|left, right| left.id.cmp(&right.id));
    let content_hash = canonical_hash(CULTURE_PLAN_HASH_DOMAIN, &canonical_definition)?;

    let mut target_by_id = BTreeMap::new();
    let mut targets = definition.targets.clone();
    targets.sort_by(|left, right| left.id.cmp(&right.id));
    for (index, target) in targets.iter().enumerate() {
        target_by_id.insert(target.id.clone(), TargetKey::from_raw(index as u32));
    }
    let compiled_targets = targets
        .into_iter()
        .enumerate()
        .map(|(index, target)| CompiledTarget {
            key: TargetKey::from_raw(index as u32),
            parent: target
                .parent
                .as_ref()
                .and_then(|parent| target_by_id.get(parent).copied()),
            source_id: target.id,
            neutral_profile: target.neutral_profile,
            metadata: target.metadata,
        })
        .collect::<Vec<_>>();

    let mut cohort_by_id = BTreeMap::new();
    let mut cohorts = definition.cohorts.clone();
    cohorts.sort_by(|left, right| left.id.cmp(&right.id));
    for (index, cohort) in cohorts.iter().enumerate() {
        cohort_by_id.insert(cohort.id.clone(), CohortKey::from_raw(index as u32));
    }
    let compiled_cohorts = cohorts
        .into_iter()
        .enumerate()
        .map(|(index, cohort)| CompiledCohort {
            key: CohortKey::from_raw(index as u32),
            source_id: cohort.id,
            territory: cohort.territory,
            headcount: cohort.headcount,
            classification: cohort.classification,
        })
        .collect::<Vec<_>>();

    let mut authored_channels = definition.channels.clone();
    authored_channels.sort_by(|left, right| left.id.cmp(&right.id));
    let channels = authored_channels
        .into_iter()
        .enumerate()
        .map(|(index, channel)| {
            Ok(CompiledChannel {
                key: ChannelKey::from_raw(index as u32),
                source_id: channel.id,
                source_cohort: channel
                    .source_cohort_id
                    .as_ref()
                    .and_then(|id| cohort_by_id.get(id).copied()),
                target_cohort: cohort_by_id[&channel.target_cohort_id],
                target: target_by_id[&channel.target_id],
                reach_per_mille: channel.reach_per_mille,
                trust_per_mille: channel.trust_per_mille,
                interpretation_fidelity_per_mille: channel.interpretation_fidelity_per_mille,
                delay_boundaries: channel.delay_boundaries,
                capacity: channel.capacity,
                active: channel.active,
            })
        })
        .collect::<Result<Vec<_>, CanwuError>>()?;

    let mut authored_transitions = definition.transitions.clone();
    authored_transitions.sort_by(|left, right| left.id.cmp(&right.id));
    let mut transitions_by_target = BTreeMap::<TargetKey, Vec<TransitionKey>>::new();
    let transitions = authored_transitions
        .into_iter()
        .enumerate()
        .map(|(index, transition)| {
            let key = TransitionKey::from_raw(index as u32);
            let target = target_by_id[&transition.target_id];
            transitions_by_target.entry(target).or_default().push(key);
            let mut affected_cohorts = transition
                .affected_cohorts
                .iter()
                .filter_map(|id| cohort_by_id.get(id).copied())
                .collect::<Vec<_>>();
            affected_cohorts.sort();
            CompiledTransition {
                key,
                source_id: transition.id,
                target,
                affected_cohorts,
                from: transition.from,
                to: transition.to,
                base_rate_per_million: transition.base_rate_per_million,
                weights: transition.weights,
            }
        })
        .collect::<Vec<_>>();

    let mut authored_effects = definition.effects.clone();
    authored_effects.sort_by(|left, right| left.id.cmp(&right.id));
    let mut effects_by_target = BTreeMap::<TargetKey, Vec<EffectKey>>::new();
    let effects = authored_effects
        .into_iter()
        .enumerate()
        .map(|(index, effect)| {
            let key = EffectKey::from_raw(index as u32);
            let target = target_by_id[&effect.target_id];
            effects_by_target.entry(target).or_default().push(key);
            CompiledEffect {
                key,
                source_id: effect.id,
                target,
                signal_kind: effect.signal_kind,
                scope: effect.scope.into_iter().collect(),
                cadence_boundaries: effect.cadence_boundaries,
                persistence: effect.persistence,
                requires_evidence: effect.requires_evidence,
            }
        })
        .collect::<Vec<_>>();

    let mut authored_institutions = definition.institutions.clone();
    authored_institutions.sort_by(|left, right| left.id.cmp(&right.id));
    let mut institutions_by_target = BTreeMap::<TargetKey, Vec<InstitutionKey>>::new();
    let institutions = authored_institutions
        .into_iter()
        .enumerate()
        .map(|(index, institution)| {
            let key = InstitutionKey::from_raw(index as u32);
            let target = target_by_id[&institution.target_id];
            institutions_by_target.entry(target).or_default().push(key);
            let mut affected_cohorts = institution
                .affected_cohorts
                .iter()
                .filter_map(|id| cohort_by_id.get(id).copied())
                .collect::<Vec<_>>();
            affected_cohorts.sort();
            CompiledInstitutionBinding {
                key,
                source_id: institution.id,
                target,
                institution: institution.institution,
                affected_cohorts,
            }
        })
        .collect::<Vec<_>>();

    let authored_bytes = serde_json::to_vec(&canonical_definition)
        .map_err(|error| {
            CanwuError::new(
                canwu_api::ErrorCode::InvalidDomainRecord,
                format!("culture definition cannot be measured: {error}"),
            )
        })?
        .len();
    let estimated_memory = estimate_memory_bytes(
        compiled_targets.len(),
        compiled_cohorts.len(),
        channels.len(),
        transitions.len(),
        effects.len(),
        institutions.len(),
        authored_bytes,
    );
    if estimated_memory > definition.budgets.max_memory_bytes {
        return Err(CanwuError::new(
            canwu_api::ErrorCode::InvalidDomainRecord,
            format!(
                "compiled culture plan exceeds memory budget: {estimated_memory} > {}",
                definition.budgets.max_memory_bytes
            ),
        ));
    }

    let plan = CompiledCulturePlan {
        plan_version: CULTURE_PLAN_VERSION,
        definition_id: definition.id.clone(),
        content_hash,
        budgets: definition.budgets,
        retirement: definition.retirement,
        targets: compiled_targets,
        cohorts: compiled_cohorts,
        channels,
        transitions,
        effects,
        institutions,
        target_by_id,
        cohort_by_id,
        transitions_by_target,
        effects_by_target,
        institutions_by_target,
    };
    crate::CultureState::from_plan_at(&plan, SimTime::EPOCH).validate_against_plan(&plan)?;
    Ok(plan)
}

fn estimate_memory_bytes(
    targets: usize,
    cohorts: usize,
    channels: usize,
    transitions: usize,
    effects: usize,
    institutions: usize,
    authored_bytes: usize,
) -> usize {
    targets
        .saturating_mul(192)
        .saturating_add(cohorts.saturating_mul(192))
        .saturating_add(channels.saturating_mul(160))
        .saturating_add(transitions.saturating_mul(192))
        .saturating_add(effects.saturating_mul(160))
        .saturating_add(institutions.saturating_mul(128))
        // Account conservatively for retained string/map payloads and their
        // serialized authoring representation in content caches.
        .saturating_add(authored_bytes.saturating_mul(2))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CultureCohortDefinition, TransitionSpec};
    use canwu_api::TerritoryId;

    fn definition() -> CultureDefinition {
        CultureDefinition::builder("rights")
            .target("equality")
            .target("dignity")
            .cohort(CultureCohortDefinition::new(
                "town",
                TerritoryId::new(1),
                100,
            ))
            .transition(TransitionSpec::awareness_from_influence(
                "aware", "equality", 100_000,
            ))
            .build()
            .expect("valid definition")
    }

    #[test]
    fn plan_ids_and_hash_are_invariant_to_authoring_order() {
        let first = compile_culture(&definition()).expect("compile");
        let mut second_definition = definition();
        second_definition.targets.reverse();
        second_definition.transitions.reverse();
        let second = compile_culture(&second_definition).expect("compile");
        assert_eq!(first, second);
        assert_eq!(first.target_by_id["dignity"].get(), 0);
        assert_eq!(first.target_by_id["equality"].get(), 1);
    }

    #[test]
    fn memory_budget_rejects_before_runtime_use() {
        let mut definition = definition();
        definition.budgets.max_memory_bytes = 1;
        let error = compile_culture(&definition).expect_err("budget must reject");
        assert_eq!(error.code, canwu_api::ErrorCode::InvalidDomainRecord);
    }

    #[test]
    fn canonical_hash_uses_engine_hash_contract() {
        let definition = definition();
        let plan = compile_culture(&definition).expect("compile");
        assert_eq!(plan.content_hash.len(), 64);
    }
}
