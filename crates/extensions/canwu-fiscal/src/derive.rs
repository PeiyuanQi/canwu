use crate::model::{
    CompiledFiscalCatalog, FiscalAdoptionStage, FiscalHistoricalMode, FiscalProjection,
    FiscalReportFact, FiscalState, FiscalStrategicAggregate, FiscalTransitionCandidate, invalid,
};
use canwu_api::{CanwuError, SimTime};
use std::collections::BTreeMap;

pub fn recompute_derived(
    state: &mut FiscalState,
    catalog: &CompiledFiscalCatalog,
    at: SimTime,
) -> Result<(), CanwuError> {
    state.transition_candidates = compute_transition_candidates(state, catalog, at);
    state.aggregates = compute_aggregates(state, catalog)?;
    Ok(())
}

#[must_use]
pub fn compute_transition_candidates(
    state: &FiscalState,
    catalog: &CompiledFiscalCatalog,
    at: SimTime,
) -> BTreeMap<String, FiscalTransitionCandidate> {
    let mut candidates = BTreeMap::new();
    for transition in catalog.transitions.values() {
        let time_eligible = match state.historical_context.mode {
            FiscalHistoricalMode::Counterfactual => transition
                .eligibility_window
                .contains(state.historical_context.year),
            FiscalHistoricalMode::RecordedBaseline | FiscalHistoricalMode::ResearchReplay => {
                transition
                    .observed_window
                    .contains(state.historical_context.year)
            }
        };
        if !time_eligible {
            continue;
        }
        for jurisdiction_id in &transition.jurisdiction_ids {
            let scopes: Vec<_> = state
                .scope_bindings
                .values()
                .filter(|scope| scope.jurisdiction_id == *jurisdiction_id)
                .collect();
            if scopes.is_empty()
                || transition.to_rule_ids.iter().any(|rule_id| {
                    catalog.rules.get(rule_id).is_none_or(|rule| {
                        !scopes.iter().any(|scope| scope.mechanism == rule.mechanism)
                    })
                })
            {
                continue;
            }
            let source_ready = transition.from_rule_ids.is_empty()
                || state.adoptions.values().any(|adoption| {
                    state
                        .scope_bindings
                        .get(&adoption.scope_binding_id)
                        .is_some_and(|scope| scope.jurisdiction_id == *jurisdiction_id)
                        && transition.from_rule_ids.contains(&adoption.rule_id)
                        && adoption.stage != FiscalAdoptionStage::Repealed
                });
            let target_active = transition.to_rule_ids.iter().all(|target_rule| {
                state.adoptions.values().any(|adoption| {
                    adoption.rule_id == *target_rule
                        && adoption.stage.is_operational()
                        && state
                            .scope_bindings
                            .get(&adoption.scope_binding_id)
                            .is_some_and(|scope| scope.jurisdiction_id == *jurisdiction_id)
                })
            });
            let prerequisites_ready = transition.prerequisite_ids.iter().all(|required| {
                catalog
                    .transitions
                    .get(required)
                    .is_some_and(|prerequisite| {
                        prerequisite.to_rule_ids.iter().all(|target_rule| {
                            state.adoptions.values().any(|adoption| {
                                adoption.rule_id == *target_rule
                                    && adoption.stage.is_operational()
                                    && state
                                        .scope_bindings
                                        .get(&adoption.scope_binding_id)
                                        .is_some_and(|scope| {
                                            scope.jurisdiction_id == *jurisdiction_id
                                        })
                            })
                        })
                    })
            });
            if !source_ready || target_active || !prerequisites_ready {
                continue;
            }
            let id = format!("{}::{}", transition.id, jurisdiction_id);
            let historically_observed = transition
                .observed_window
                .contains(state.historical_context.year);
            let evaluated_at = state
                .transition_candidates
                .get(&id)
                .filter(|candidate| candidate.historically_observed == historically_observed)
                .map_or(at, |candidate| candidate.evaluated_at);
            candidates.insert(
                id.clone(),
                FiscalTransitionCandidate {
                    id,
                    transition_id: transition.id.clone(),
                    jurisdiction_id: jurisdiction_id.clone(),
                    historically_observed,
                    evaluated_at,
                },
            );
        }
    }
    candidates
}

pub fn compute_aggregates(
    state: &FiscalState,
    catalog: &CompiledFiscalCatalog,
) -> Result<BTreeMap<String, FiscalStrategicAggregate>, CanwuError> {
    let mut by_key = BTreeMap::new();
    let mut assessment_keys = BTreeMap::new();
    for assessment in state.assessments.values() {
        let scope = state
            .scope_bindings
            .get(&assessment.scope_binding_id)
            .ok_or_else(|| invalid("aggregate encountered an unknown fiscal scope"))?;
        let rule = catalog
            .rules
            .get(&assessment.rule_id)
            .ok_or_else(|| invalid("aggregate encountered an unknown fiscal rule"))?;
        let key = (
            scope.institution.clone(),
            rule.mechanism,
            scope.id.clone(),
            assessment.accounting_cycle_id.clone(),
            assessment.unit.clone(),
            assessment.payment_form,
        );
        assessment_keys.insert(assessment.id.as_str(), key.clone());
        let aggregate = by_key
            .entry(key)
            .or_insert_with(|| FiscalStrategicAggregate {
                institution: scope.institution.clone(),
                mechanism: rule.mechanism,
                scope_binding_id: scope.id.clone(),
                accounting_cycle_id: assessment.accounting_cycle_id.clone(),
                unit: assessment.unit.clone(),
                payment_form: assessment.payment_form,
                assessed: 0,
                remission_granted: 0,
                collected: 0,
                remitted: 0,
                disbursed: 0,
                reserved: 0,
                returned: 0,
                outstanding: 0,
            });
        aggregate.assessed = checked_sum(aggregate.assessed, assessment.quantity)?;
    }
    for remission in state.remissions.values() {
        let key = assessment_keys
            .get(remission.assessment_id.as_str())
            .ok_or_else(|| invalid("aggregate remission has no assessment"))?;
        let aggregate = by_key
            .get_mut(key)
            .ok_or_else(|| invalid("aggregate remission partition is unavailable"))?;
        aggregate.remission_granted = checked_sum(aggregate.remission_granted, remission.quantity)?;
    }
    let mut request_keys = BTreeMap::new();
    for request in state.execution_requests.values() {
        let key = assessment_keys
            .get(request.assessment_id.as_str())
            .ok_or_else(|| invalid("aggregate execution request has no assessment"))?;
        request_keys.insert(request.id.as_str(), (key.clone(), request.kind));
    }
    for receipt in state
        .execution_receipts
        .values()
        .filter(|receipt| receipt.disposition.counts_as_fulfillment())
    {
        let (key, kind) = request_keys
            .get(receipt.request_id.as_str())
            .ok_or_else(|| invalid("aggregate receipt has no execution request"))?;
        let aggregate = by_key
            .get_mut(key)
            .ok_or_else(|| invalid("aggregate receipt partition is unavailable"))?;
        let target = match kind {
            crate::model::FiscalExecutionKind::Collect => &mut aggregate.collected,
            crate::model::FiscalExecutionKind::Remit => &mut aggregate.remitted,
            crate::model::FiscalExecutionKind::Disburse => &mut aggregate.disbursed,
            crate::model::FiscalExecutionKind::Reserve => &mut aggregate.reserved,
            crate::model::FiscalExecutionKind::Return => &mut aggregate.returned,
        };
        *target = checked_sum(*target, receipt.quantity)?;
    }
    for aggregate in by_key.values_mut() {
        aggregate.outstanding = aggregate
            .assessed
            .checked_sub(aggregate.remission_granted)
            .and_then(|value| value.checked_sub(aggregate.collected))
            .ok_or_else(|| invalid("fiscal aggregate underflowed"))?;
    }
    Ok(by_key
        .into_values()
        .enumerate()
        .map(|(index, aggregate)| (format!("aggregate.{index:04}"), aggregate))
        .collect())
}

#[must_use]
pub fn compute_projections(state: &FiscalState, at: SimTime) -> BTreeMap<String, FiscalProjection> {
    state
        .observer_bindings
        .values()
        .map(|observer| {
            let facts = state
                .aggregates
                .values()
                .filter(|aggregate| {
                    observer
                        .visible_institutions
                        .contains(&aggregate.institution)
                })
                .enumerate()
                .map(|(index, aggregate)| {
                    let id = format!("fact.{index:04}");
                    (
                        id.clone(),
                        FiscalReportFact {
                            id,
                            institution: aggregate.institution.clone(),
                            mechanism: aggregate.mechanism,
                            scope_binding_id: aggregate.scope_binding_id.clone(),
                            accounting_cycle_id: aggregate.accounting_cycle_id.clone(),
                            unit: aggregate.unit.clone(),
                            payment_form: aggregate.payment_form,
                            assessed: estimate(aggregate.assessed, observer.confidence_per_mille),
                            collected: estimate(aggregate.collected, observer.confidence_per_mille),
                            outstanding: estimate(
                                aggregate.outstanding,
                                observer.confidence_per_mille,
                            ),
                        },
                    )
                })
                .collect();
            (
                observer.id.clone(),
                FiscalProjection {
                    actor: observer.actor,
                    as_of: at,
                    confidence_per_mille: observer.confidence_per_mille,
                    facts,
                },
            )
        })
        .collect()
}

fn estimate(value: u64, confidence_per_mille: u16) -> crate::model::FiscalAmountEstimate {
    let mut magnitude = 1_u64;
    while value / magnitude >= 10 && magnitude <= u64::MAX / 10 {
        magnitude *= 10;
    }
    let precision_divisor = match confidence_per_mille {
        900..=1_000 => 100,
        750..=899 => 10,
        500..=749 => 2,
        _ => 1,
    };
    let bucket_width = (magnitude / precision_divisor).max(2);
    let minimum = value / bucket_width * bucket_width;
    crate::model::FiscalAmountEstimate {
        minimum,
        maximum: minimum.saturating_add(bucket_width - 1),
    }
}

fn checked_sum(left: u64, right: u64) -> Result<u64, CanwuError> {
    left.checked_add(right)
        .ok_or_else(|| invalid("fiscal aggregate overflowed"))
}
