use crate::model::{
    CompiledFiscalCatalog, FISCAL_REPORT_FACTS_PER_PART, FiscalAdoptionStage, FiscalHistoricalMode,
    FiscalMechanism, FiscalObserverBinding, FiscalPaymentForm, FiscalProjection, FiscalReport,
    FiscalReportFact, FiscalReportPart, FiscalReportPlan, FiscalReportRecord, FiscalReportSummary,
    FiscalState, FiscalStrategicAggregate, FiscalTransitionCandidate,
    MAX_FISCAL_REPORT_RECORDS_PER_BOUNDARY, invalid,
};
use canwu_api::{CanwuError, EntityRef, SimTime};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// Brings the transition candidates and the aggregates of `state` up to date.
///
/// The report plan is not touched: it records what changed between two
/// derivations of a running state, and only the aggregate system writes it.
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

/// What identifies one aggregate: its institution, mechanism, scope binding,
/// accounting cycle, unit, and payment form.
type AggregateKey = (
    EntityRef,
    FiscalMechanism,
    String,
    String,
    String,
    FiscalPaymentForm,
);

fn aggregate_key(aggregate: &FiscalStrategicAggregate) -> AggregateKey {
    (
        aggregate.institution.clone(),
        aggregate.mechanism,
        aggregate.scope_binding_id.clone(),
        aggregate.accounting_cycle_id.clone(),
        aggregate.unit.clone(),
        aggregate.payment_form,
    )
}

/// Derives the aggregates of `state`, keyed by aggregate ID.
///
/// An aggregate keeps the ID it has in `state.aggregates`, and a new
/// aggregate takes the next ID, so an ID names the same aggregate for the
/// life of a run and the IDs list the aggregates in the order they first
/// appeared. Aggregates that appear together are numbered in key order, which
/// is the order of all of them when `state.aggregates` is empty.
///
/// A state may come with aggregates. If they are numbered from zero without a
/// gap and each is still an aggregate of the state, they keep their IDs and
/// the ones that are missing are appended, so a state that comes with all of
/// its aggregates keeps its numbering. Any other numbering is replaced by one
/// in key order.
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
    Ok(with_stable_ids(&state.aggregates, by_key))
}

fn aggregate_id(index: usize) -> String {
    format!("aggregate.{index:04}")
}

/// Keys the aggregates by ID, keeping the IDs `previous` gave.
///
/// `previous` is kept when it numbers its aggregates from zero without a gap
/// and each of them is still an aggregate. Assessments are never removed, so
/// that holds for every state a boundary has derived, and for an authored
/// state whose aggregates were derived the same way. Anything else is
/// numbered afresh in key order.
fn with_stable_ids(
    previous: &BTreeMap<String, FiscalStrategicAggregate>,
    by_key: BTreeMap<AggregateKey, FiscalStrategicAggregate>,
) -> BTreeMap<String, FiscalStrategicAggregate> {
    let mut kept = BTreeMap::new();
    let numbered = previous.iter().enumerate().all(|(index, (id, aggregate))| {
        let key = aggregate_key(aggregate);
        *id == aggregate_id(index) && by_key.contains_key(&key) && kept.insert(key, index).is_none()
    });
    if !numbered {
        kept.clear();
    }
    let mut next = kept.len();
    by_key
        .into_iter()
        .map(|(key, aggregate)| {
            let index = kept.get(&key).copied().unwrap_or_else(|| {
                next += 1;
                next - 1
            });
            (aggregate_id(index), aggregate)
        })
        .collect()
}

/// What each observer of `state` is told as of `at`, keyed by observer binding
/// ID. The facts follow the aggregates in ID order and are labelled
/// `fact.NNNN` by their place among the facts the observer can see, so a
/// fact keeps its label while later aggregates are added.
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
                        report_fact(id, aggregate, observer.confidence_per_mille),
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

/// One observer's facts in the order reports publish them, without labels.
///
/// That order is aggregate ID order. IDs are given in the order aggregates
/// first appear and are never given again, so a new aggregate adds a fact at
/// the end, and the parts cut from this order keep their members while the
/// last part fills.
fn observer_facts(
    observer: &FiscalObserverBinding,
    aggregates: &BTreeMap<String, FiscalStrategicAggregate>,
) -> Vec<FiscalReportFact> {
    aggregates
        .values()
        .filter(|aggregate| {
            observer
                .visible_institutions
                .contains(&aggregate.institution)
        })
        .map(|aggregate| report_fact(String::new(), aggregate, observer.confidence_per_mille))
        .collect()
}

fn report_fact(
    id: String,
    aggregate: &FiscalStrategicAggregate,
    confidence_per_mille: u16,
) -> FiscalReportFact {
    FiscalReportFact {
        id,
        institution: aggregate.institution.clone(),
        mechanism: aggregate.mechanism,
        scope_binding_id: aggregate.scope_binding_id.clone(),
        accounting_cycle_id: aggregate.accounting_cycle_id.clone(),
        unit: aggregate.unit.clone(),
        payment_form: aggregate.payment_form,
        assessed: estimate(aggregate.assessed, confidence_per_mille),
        collected: estimate(aggregate.collected, confidence_per_mille),
        outstanding: estimate(aggregate.outstanding, confidence_per_mille),
    }
}

fn count(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// Decides which report parts the boundary that derives `aggregates`
/// publishes, or `None` when the plan the state holds needs no change.
///
/// `state` still holds the aggregates and the plan of the previous
/// derivation. A part is planned when its facts differ between the two sets
/// of aggregates, when the previous plan deferred it, or, for the first plan
/// of a state, whenever it has facts. Each observer with a planned part also
/// gets its summary. At most [`MAX_FISCAL_REPORT_RECORDS_PER_BOUNDARY`]
/// records are planned: the parts an earlier plan deferred come first, the
/// rest are deferred, and a later plan takes them up.
///
/// `unreachable` lists the observer bindings whose actor cannot receive
/// knowledge now. Nothing is planned for them and they use none of the
/// boundary's records: what changed in their reports is deferred, and stays
/// deferred until a plan finds them reachable again.
pub(crate) fn next_report_plan(
    state: &FiscalState,
    aggregates: &BTreeMap<String, FiscalStrategicAggregate>,
    unreachable: &BTreeSet<String>,
) -> Option<FiscalReportPlan> {
    // Observer bindings never change, so a state without any has no report.
    if state.observer_bindings.is_empty() {
        return None;
    }
    let previous = &state.report_plan;
    let reported = previous.revision != 0;
    if reported && previous.deferred.is_empty() && state.aggregates == *aggregates {
        return None;
    }
    // IDs are stable, so the institutions whose reports can have changed are
    // those of the aggregates that differ under one ID.
    let touched: BTreeSet<&EntityRef> = aggregates
        .iter()
        .filter(|(id, aggregate)| state.aggregates.get(*id) != Some(*aggregate))
        .map(|(_, aggregate)| &aggregate.institution)
        .chain(
            state
                .aggregates
                .iter()
                .filter(|(id, _)| !aggregates.contains_key(*id))
                .map(|(_, aggregate)| &aggregate.institution),
        )
        .collect();
    let mut publish = BTreeMap::<String, BTreeSet<u32>>::new();
    let mut deferred = BTreeMap::<String, BTreeSet<u32>>::new();
    let mut carried = Vec::new();
    let mut changed = Vec::new();
    for observer in state.observer_bindings.values() {
        let owed = previous.deferred.get(&observer.id);
        if reported
            && owed.is_none()
            && !touched
                .iter()
                .any(|institution| observer.visible_institutions.contains(*institution))
        {
            continue;
        }
        let facts = observer_facts(observer, aggregates);
        let parts: Vec<_> = facts.chunks(FISCAL_REPORT_FACTS_PER_PART).collect();
        let differing: BTreeSet<u32> = if reported {
            let published_facts = observer_facts(observer, &state.aggregates);
            let published: Vec<_> = published_facts
                .chunks(FISCAL_REPORT_FACTS_PER_PART)
                .collect();
            (0..parts.len().max(published.len()))
                .filter(|index| parts.get(*index) != published.get(*index))
                .map(count)
                .collect()
        } else {
            (0..parts.len()).map(count).collect()
        };
        if reported && owed.is_none() && differing.is_empty() {
            continue;
        }
        if unreachable.contains(&observer.id) {
            // Kept for when the observer can receive knowledge again. The
            // entry stands even without a part, for the summary it is owed.
            let held = deferred.entry(observer.id.clone()).or_default();
            held.extend(owed.into_iter().flatten().copied());
            held.extend(differing);
            continue;
        }
        publish.insert(observer.id.clone(), BTreeSet::new());
        carried.extend(
            owed.into_iter()
                .flatten()
                .map(|part| (observer.id.as_str(), *part)),
        );
        changed.extend(
            differing
                .into_iter()
                .filter(|part| owed.is_none_or(|owed| !owed.contains(part)))
                .map(|part| (observer.id.as_str(), part)),
        );
    }
    if publish.is_empty() && deferred == previous.deferred {
        return None;
    }
    // Every listed observer gets its summary; the parts share what is left.
    let mut budget = MAX_FISCAL_REPORT_RECORDS_PER_BOUNDARY.saturating_sub(publish.len());
    for (observer_id, part) in carried.into_iter().chain(changed) {
        let planned = if budget == 0 {
            deferred.entry(observer_id.to_owned()).or_default()
        } else {
            budget -= 1;
            publish.entry(observer_id.to_owned()).or_default()
        };
        planned.insert(part);
    }
    Some(FiscalReportPlan {
        revision: state.procedure_revision,
        publish,
        deferred,
    })
}

/// The summary and the parts the report system publishes for one observer.
pub(crate) struct PlannedReport<'a> {
    pub(crate) observer: &'a FiscalObserverBinding,
    /// The observer binding's place among the state's observer bindings.
    pub(crate) slot: u32,
    pub(crate) summary: FiscalReportSummary,
    pub(crate) parts: Vec<FiscalReportPart>,
}

/// What the report plan of `state` has the boundary at `at` publish. Nothing
/// is planned unless the plan was computed at the state's own procedure
/// revision: a plan of an earlier revision has already been carried out.
pub(crate) fn planned_reports(state: &FiscalState, at: SimTime) -> Vec<PlannedReport<'_>> {
    let plan = &state.report_plan;
    if plan.revision != state.procedure_revision || plan.publish.is_empty() {
        return Vec::new();
    }
    state
        .observer_bindings
        .values()
        .enumerate()
        .filter_map(|(slot, observer)| {
            let planned = plan.publish.get(&observer.id)?;
            let facts = observer_facts(observer, &state.aggregates);
            let summary = FiscalReportSummary {
                observer_id: observer.id.clone(),
                actor: observer.actor,
                as_of: at,
                confidence_per_mille: observer.confidence_per_mille,
                fact_count: count(facts.len()),
                part_count: count(facts.len().div_ceil(FISCAL_REPORT_FACTS_PER_PART)),
                pending_parts: plan.deferred.get(&observer.id).cloned().unwrap_or_default(),
            };
            let parts = planned
                .iter()
                .map(|part| FiscalReportPart {
                    observer_id: observer.id.clone(),
                    part: *part,
                    // A part past the report's last one is published empty,
                    // which withdraws it.
                    facts: usize::try_from(*part)
                        .ok()
                        .and_then(|index| facts.chunks(FISCAL_REPORT_FACTS_PER_PART).nth(index))
                        .map(<[FiscalReportFact]>::to_vec)
                        .unwrap_or_default(),
                })
                .collect();
            Some(PlannedReport {
                observer,
                slot: count(slot),
                summary,
                parts,
            })
        })
        .collect()
}

/// Reassembles fiscal reports from the payloads of the fiscal report records
/// one holder currently holds, keyed by observer binding ID.
///
/// Pass every current record of [`crate::fiscal_report_knowledge_schema_id`]
/// the holder has, and no superseded one; [`crate::fiscal_reports`] does that
/// for a viewer. The parts are joined in order and each fact is labelled
/// `fact.NNNN` by its place in the whole report, so a complete report equals
/// what [`compute_projections`] derives from the fiscal state it was
/// published from. In an incomplete report the facts of a pending part are
/// the older ones the holder has, or missing when it has none, and the other
/// facts keep their labels; [`FiscalReport::pending_parts`] names the pending
/// parts.
///
/// # Errors
///
/// Returns an error when a payload is not a fiscal report record, when two
/// records fill one summary or part, when a part has no summary, or when the
/// parts of a complete report do not hold the facts its summary counts.
pub fn assemble_fiscal_reports<'a>(
    payloads: impl IntoIterator<Item = &'a Value>,
) -> Result<BTreeMap<String, FiscalReport>, CanwuError> {
    let mut summaries = BTreeMap::new();
    let mut parts = BTreeMap::<String, BTreeMap<u32, Vec<FiscalReportFact>>>::new();
    for payload in payloads {
        let duplicate = match FiscalReportRecord::deserialize(payload)
            .map_err(|error| invalid(format!("fiscal report record is invalid: {error}")))?
        {
            FiscalReportRecord::Summary(summary) => summaries
                .insert(summary.observer_id.clone(), summary)
                .is_some(),
            FiscalReportRecord::Part(part) => parts
                .entry(part.observer_id)
                .or_default()
                .insert(part.part, part.facts)
                .is_some(),
        };
        if duplicate {
            return Err(invalid(
                "two current fiscal report records fill the same summary or part",
            ));
        }
    }
    let reports = summaries
        .into_iter()
        .map(|(observer_id, summary)| {
            let facts: BTreeMap<_, _> = parts
                .remove(&observer_id)
                .unwrap_or_default()
                .into_iter()
                .flat_map(|(part, facts)| {
                    let first = usize::try_from(part)
                        .unwrap_or(usize::MAX)
                        .saturating_mul(FISCAL_REPORT_FACTS_PER_PART);
                    facts
                        .into_iter()
                        .enumerate()
                        .map(move |(offset, mut fact)| {
                            fact.id = format!("fact.{:04}", first.saturating_add(offset));
                            (fact.id.clone(), fact)
                        })
                })
                .collect();
            if summary.pending_parts.is_empty() && count(facts.len()) != summary.fact_count {
                return Err(invalid(
                    "fiscal report parts do not hold the facts their summary counts",
                ));
            }
            Ok((
                observer_id,
                FiscalReport {
                    projection: FiscalProjection {
                        actor: summary.actor,
                        as_of: summary.as_of,
                        confidence_per_mille: summary.confidence_per_mille,
                        facts,
                    },
                    pending_parts: summary.pending_parts,
                },
            ))
        })
        .collect::<Result<BTreeMap<_, _>, CanwuError>>()?;
    if !parts.is_empty() {
        return Err(invalid("a fiscal report part has no summary"));
    }
    Ok(reports)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        FiscalAmountEstimate, FiscalAssessment, MAX_FISCAL_IDENTIFIER_BYTES, MAX_FISCAL_OBSERVERS,
        MAX_FISCAL_REPORT_PARTS,
    };
    use canwu_api::{
        DomainRecordRef, KnowledgeHolderRef, KnowledgeLimitsV1, OrganizationId, PersonId,
        SimDuration,
    };

    const SCOPE: &str = "scope.a";

    fn institution(id: u64) -> EntityRef {
        EntityRef::Organization(OrganizationId::new(id))
    }

    fn observer(id: &str, institution: EntityRef) -> FiscalObserverBinding {
        let actor = PersonId::new(1);
        FiscalObserverBinding {
            id: id.to_owned(),
            actor,
            knowledge_holder: KnowledgeHolderRef::Person(actor),
            visible_institutions: BTreeSet::from([institution]),
            confidence_per_mille: 1_000,
        }
    }

    /// A state with `observers` and no assessment yet, at procedure revision
    /// 7.
    fn observed_state(observers: impl IntoIterator<Item = FiscalObserverBinding>) -> FiscalState {
        let mut state = FiscalState::new(
            150,
            crate::model::FiscalHistoricalMode::Counterfactual,
            SimTime::EPOCH,
        );
        state.procedure_revision = 7;
        state.observer_bindings = observers
            .into_iter()
            .map(|observer| (observer.id.clone(), observer))
            .collect();
        state
    }

    /// Adds the assessment of one accounting cycle and returns the aggregate
    /// that reports it.
    fn assess(
        state: &mut FiscalState,
        institution: EntityRef,
        cycle: &str,
        assessed: u64,
    ) -> FiscalStrategicAggregate {
        let id = format!("assessment.{cycle}");
        state.assessments.insert(
            id.clone(),
            FiscalAssessment {
                id,
                rule_id: "rule.a".to_owned(),
                scope_binding_id: SCOPE.to_owned(),
                accounting_cycle_id: cycle.to_owned(),
                quantity: assessed,
                unit: "grain".to_owned(),
                payment_form: FiscalPaymentForm::Grain,
                commutation_quote: None,
                created_at: SimTime::EPOCH,
            },
        );
        FiscalStrategicAggregate {
            institution,
            mechanism: FiscalMechanism::LandTax,
            scope_binding_id: SCOPE.to_owned(),
            accounting_cycle_id: cycle.to_owned(),
            unit: "grain".to_owned(),
            payment_form: FiscalPaymentForm::Grain,
            assessed,
            remission_granted: 0,
            collected: 0,
            remitted: 0,
            disbursed: 0,
            reserved: 0,
            returned: 0,
            outstanding: assessed,
        }
    }

    /// Derives `aggregates` as the aggregate system would, with the IDs the
    /// state's aggregates already have, and returns the plan it wrote, if any.
    fn derive(
        state: &mut FiscalState,
        aggregates: impl IntoIterator<Item = FiscalStrategicAggregate>,
    ) -> Option<FiscalReportPlan> {
        derive_without(state, aggregates, &[])
    }

    /// Derives as [`derive`] does while the listed observers cannot receive
    /// knowledge.
    fn derive_without(
        state: &mut FiscalState,
        aggregates: impl IntoIterator<Item = FiscalStrategicAggregate>,
        unreachable: &[&str],
    ) -> Option<FiscalReportPlan> {
        let unreachable = unreachable.iter().map(|id| (*id).to_owned()).collect();
        let by_key = aggregates
            .into_iter()
            .map(|aggregate| (aggregate_key(&aggregate), aggregate))
            .collect();
        let aggregates = with_stable_ids(&state.aggregates, by_key);
        let plan = next_report_plan(state, &aggregates, &unreachable);
        state.aggregates = aggregates;
        if let Some(plan) = &plan {
            state.report_plan = plan.clone();
        }
        plan
    }

    fn cycles(aggregates: &BTreeMap<String, FiscalStrategicAggregate>) -> Vec<&str> {
        aggregates
            .values()
            .map(|aggregate| aggregate.accounting_cycle_id.as_str())
            .collect()
    }

    fn parts(numbers: impl IntoIterator<Item = u32>) -> BTreeSet<u32> {
        numbers.into_iter().collect()
    }

    #[test]
    fn an_aggregate_keeps_its_id_and_a_new_one_takes_the_next() {
        let mut state = observed_state(Vec::new());
        let mut aggregates = vec![
            assess(&mut state, institution(1), "cycle.b", 100),
            assess(&mut state, institution(1), "cycle.d", 100),
        ];
        // Aggregates that appear together are numbered in key order.
        derive(&mut state, aggregates.iter().rev().cloned());
        assert_eq!(cycles(&state.aggregates), ["cycle.b", "cycle.d"]);

        // Later ones take the next IDs, wherever their keys sort, and a
        // changed aggregate keeps its ID.
        aggregates.push(assess(&mut state, institution(1), "cycle.c", 100));
        aggregates.push(assess(&mut state, institution(1), "cycle.a", 100));
        aggregates[0].collected = 50;
        derive(&mut state, aggregates.clone());
        assert_eq!(
            cycles(&state.aggregates),
            ["cycle.b", "cycle.d", "cycle.a", "cycle.c"]
        );
        assert_eq!(state.aggregates["aggregate.0000"].collected, 50);
        assert_eq!(
            state.aggregates.keys().collect::<Vec<_>>(),
            [
                "aggregate.0000",
                "aggregate.0001",
                "aggregate.0002",
                "aggregate.0003"
            ]
        );
        let stable = state.aggregates.clone();
        derive(&mut state, aggregates.clone());
        assert_eq!(state.aggregates, stable);

        // A numbering with a gap, or one that holds an aggregate the state no
        // longer derives, is not one this function gave: all are numbered
        // afresh in key order.
        let sorted = ["cycle.a", "cycle.b", "cycle.c", "cycle.d"];
        let mut gapped = state.clone();
        let moved = gapped
            .aggregates
            .remove("aggregate.0003")
            .expect("the last aggregate");
        gapped.aggregates.insert("aggregate.0007".to_owned(), moved);
        derive(&mut gapped, aggregates.clone());
        assert_eq!(cycles(&gapped.aggregates), sorted);
        let mut outdated = state.clone();
        outdated.aggregates.insert(
            "aggregate.0004".to_owned(),
            assess(
                &mut observed_state(Vec::new()),
                institution(1),
                "cycle.z",
                1,
            ),
        );
        derive(&mut outdated, aggregates);
        assert_eq!(cycles(&outdated.aggregates), sorted);
    }

    #[test]
    fn a_full_part_of_the_longest_facts_fits_one_knowledge_record() {
        // Every text at its longest and made of a character JSON escapes to
        // six bytes, except the scope binding ID, whose characters are plain.
        let escaped = |bytes: usize| "\u{1}".repeat(bytes);
        let third = MAX_FISCAL_IDENTIFIER_BYTES / 3;
        let amount = FiscalAmountEstimate {
            minimum: u64::MAX,
            maximum: u64::MAX,
        };
        let part = |fact: FiscalReportFact| {
            let record = FiscalReportRecord::Part(FiscalReportPart {
                observer_id: "o".repeat(MAX_FISCAL_IDENTIFIER_BYTES),
                part: u32::MAX,
                facts: vec![fact; FISCAL_REPORT_FACTS_PER_PART],
            });
            serde_json::to_vec(&record).expect("part payload").len()
        };
        let longest = part(FiscalReportFact {
            id: String::new(),
            institution: EntityRef::Domain(DomainRecordRef::new(
                escaped(third),
                escaped(third),
                escaped(MAX_FISCAL_IDENTIFIER_BYTES - 2 * third),
            )),
            mechanism: FiscalMechanism::TreasuryAndRemittance,
            scope_binding_id: "s".repeat(MAX_FISCAL_IDENTIFIER_BYTES),
            accounting_cycle_id: escaped(MAX_FISCAL_IDENTIFIER_BYTES),
            unit: escaped(MAX_FISCAL_IDENTIFIER_BYTES),
            payment_form: FiscalPaymentForm::SaltCertificate,
            assessed: amount.clone(),
            collected: amount.clone(),
            outstanding: amount,
        });
        let limit = KnowledgeLimitsV1::CURRENT.payload_bytes_per_record;
        assert!(longest <= limit, "{longest} bytes");

        let ordinary = part(FiscalReportFact {
            id: String::new(),
            institution: institution(1),
            mechanism: FiscalMechanism::LandTax,
            scope_binding_id: "scope.district-a.land".to_owned(),
            accounting_cycle_id: "cycle.0001".to_owned(),
            unit: "grain_equivalent".to_owned(),
            payment_form: FiscalPaymentForm::Grain,
            assessed: estimate(123_456, 850),
            collected: estimate(23_456, 850),
            outstanding: estimate(100_000, 850),
        });
        assert!(ordinary < limit / 3, "{ordinary} bytes");
    }

    #[test]
    fn a_plan_lists_the_parts_whose_facts_changed() {
        let mut state = observed_state([
            observer("observer.a", institution(1)),
            observer("observer.b", institution(2)),
        ]);
        let mut aggregates: Vec<_> = (0..20)
            .map(|cycle| {
                assess(
                    &mut state,
                    institution(1),
                    &format!("cycle.{cycle:02}"),
                    100,
                )
            })
            .collect();

        // The first plan lists every part, and a summary for an observer
        // that has no fact yet.
        let first = derive(&mut state, aggregates.clone()).expect("the first plan");
        assert_eq!(first.revision, 7);
        assert_eq!(
            first.publish,
            BTreeMap::from([
                ("observer.a".to_owned(), parts([0, 1])),
                ("observer.b".to_owned(), parts([])),
            ])
        );
        assert!(first.deferred.is_empty());
        assert_eq!(planned_reports(&state, SimTime::EPOCH).len(), 2);

        // Deriving the same aggregates again plans nothing, and a plan of an
        // earlier revision is not carried out twice.
        assert_eq!(derive(&mut state, aggregates.clone()), None);
        state.procedure_revision = 8;
        assert!(planned_reports(&state, SimTime::EPOCH).is_empty());

        // A new aggregate joins the last part, wherever its key sorts.
        aggregates.push(assess(&mut state, institution(1), "cycle.00a", 100));
        let appended = derive(&mut state, aggregates.clone()).expect("a plan");
        assert_eq!(
            appended.publish,
            BTreeMap::from([("observer.a".to_owned(), parts([1]))])
        );
        let planned = planned_reports(&state, SimTime::EPOCH);
        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].summary.fact_count, 21);
        assert_eq!(planned[0].summary.part_count, 2);
        assert_eq!(planned[0].parts.len(), 1);
        assert_eq!(planned[0].parts[0].facts.len(), 5);
        assert_eq!(
            planned[0].parts[0].facts[4].accounting_cycle_id,
            "cycle.00a"
        );

        // A change the observer's confidence cannot tell apart plans
        // nothing; one it can tell plans the part that holds the fact.
        state.procedure_revision = 9;
        aggregates[3].collected = 1;
        assert_eq!(derive(&mut state, aggregates.clone()), None);
        aggregates[3].collected = 50;
        let changed = derive(&mut state, aggregates.clone()).expect("a plan");
        assert_eq!(
            changed.publish,
            BTreeMap::from([("observer.a".to_owned(), parts([0]))])
        );

        // A change the other observer sees reaches only that observer.
        aggregates.push(assess(&mut state, institution(2), "cycle.b", 100));
        let other = derive(&mut state, aggregates).expect("a plan");
        assert_eq!(
            other.publish,
            BTreeMap::from([("observer.b".to_owned(), parts([0]))])
        );

        // A state without observers never has a plan.
        let mut unobserved = observed_state(Vec::new());
        let aggregate = assess(&mut unobserved, institution(1), "cycle.00", 100);
        assert_eq!(derive(&mut unobserved, [aggregate]), None);
    }

    #[test]
    fn a_plan_defers_what_one_boundary_cannot_publish_and_takes_it_up_first() {
        const PARTS: usize = 17;
        let observer_id = |index: usize| format!("observer.{index:02}");
        let mut state = observed_state(
            (0..MAX_FISCAL_OBSERVERS).map(|index| observer(&observer_id(index), institution(1))),
        );
        let mut aggregates: Vec<_> = (0..PARTS * FISCAL_REPORT_FACTS_PER_PART)
            .map(|cycle| {
                assess(
                    &mut state,
                    institution(1),
                    &format!("cycle.{cycle:03}"),
                    100,
                )
            })
            .collect();
        let planned = |plan: &BTreeMap<String, BTreeSet<u32>>| -> usize {
            plan.values().map(BTreeSet::len).sum()
        };

        // Every observer gets its summary; the parts share what is left of
        // the boundary's records, in observer order.
        let wanted = MAX_FISCAL_OBSERVERS * PARTS;
        let first = derive(&mut state, aggregates.clone()).expect("the first plan");
        assert_eq!(first.publish.len(), MAX_FISCAL_OBSERVERS);
        assert_eq!(
            first.publish.len() + planned(&first.publish),
            MAX_FISCAL_REPORT_RECORDS_PER_BOUNDARY
        );
        let deferred = wanted + MAX_FISCAL_OBSERVERS - MAX_FISCAL_REPORT_RECORDS_PER_BOUNDARY;
        assert_eq!(planned(&first.deferred), deferred);
        assert_eq!(first.publish[&observer_id(0)].len(), PARTS);
        let last = observer_id(MAX_FISCAL_OBSERVERS - 1);
        assert_eq!(first.deferred[&last].len(), PARTS);
        let report = planned_reports(&state, SimTime::EPOCH)
            .pop()
            .expect("the last observer's summary");
        assert_eq!(report.summary.pending_parts.len(), PARTS);
        assert!(report.parts.is_empty());

        // Every part changes again. The deferred parts are planned before any
        // of them, so no report waits behind the others twice.
        state.procedure_revision = 8;
        for aggregate in &mut aggregates {
            aggregate.collected = 50;
        }
        let second = derive(&mut state, aggregates.clone()).expect("the second plan");
        assert_eq!(second.publish[&last].len(), PARTS);
        assert_eq!(
            second.publish.len() + planned(&second.publish),
            MAX_FISCAL_REPORT_RECORDS_PER_BOUNDARY
        );
        assert_eq!(planned(&second.deferred), deferred);
        assert!(!second.deferred.contains_key(&last));

        // With nothing else changing, the rest follows and the plan empties.
        state.procedure_revision = 9;
        let third = derive(&mut state, aggregates.clone()).expect("the third plan");
        assert_eq!(planned(&third.publish), deferred);
        assert!(third.deferred.is_empty());
        state.procedure_revision = 10;
        assert_eq!(derive(&mut state, aggregates), None);

        // No report can need a part number the report lines do not have.
        const { assert!(PARTS <= MAX_FISCAL_REPORT_PARTS) };
    }

    #[test]
    fn a_plan_holds_what_an_unreachable_observer_is_owed() {
        const PARTS: usize = 17;
        let mut state = observed_state([
            observer("observer.a", institution(1)),
            observer("observer.b", institution(1)),
            observer("observer.c", institution(2)),
        ]);
        let mut aggregates: Vec<_> = (0..20)
            .map(|cycle| {
                assess(
                    &mut state,
                    institution(1),
                    &format!("cycle.{cycle:02}"),
                    100,
                )
            })
            .collect();
        let owed = |parts: &[u32]| parts.iter().copied().collect::<BTreeSet<_>>();

        // In a first plan an unreachable observer is listed as owed its
        // whole report: its parts, or with no fact its summary alone.
        let unreachable = ["observer.b", "observer.c"];
        let first = derive_without(&mut state, aggregates.clone(), &unreachable).expect("a plan");
        assert_eq!(
            first.publish,
            BTreeMap::from([("observer.a".to_owned(), owed(&[0, 1]))])
        );
        assert_eq!(
            first.deferred,
            BTreeMap::from([
                ("observer.b".to_owned(), owed(&[0, 1])),
                ("observer.c".to_owned(), owed(&[])),
            ])
        );
        assert_eq!(planned_reports(&state, SimTime::EPOCH).len(), 1);

        // What it is owed is a set of parts, so it does not grow as its
        // report keeps changing, and a derivation that changes nothing writes
        // no plan.
        state.procedure_revision = 8;
        aggregates[3].collected = 50;
        let second = derive_without(&mut state, aggregates.clone(), &unreachable).expect("a plan");
        assert_eq!(
            second.publish,
            BTreeMap::from([("observer.a".to_owned(), owed(&[0]))])
        );
        assert_eq!(second.deferred, first.deferred);
        state.procedure_revision = 9;
        assert_eq!(
            derive_without(&mut state, aggregates.clone(), &unreachable),
            None
        );

        // Reachable again, each gets what it is owed, and no more is owed.
        let third = derive(&mut state, aggregates.clone()).expect("a plan");
        assert_eq!(third.revision, 9);
        assert_eq!(
            third.publish,
            BTreeMap::from([
                ("observer.b".to_owned(), owed(&[0, 1])),
                ("observer.c".to_owned(), owed(&[])),
            ])
        );
        assert!(third.deferred.is_empty());
        let reports = planned_reports(&state, SimTime::EPOCH);
        assert_eq!(reports.len(), 2);
        assert!(
            reports
                .iter()
                .all(|report| report.summary.pending_parts.is_empty())
        );

        // Unreachable observers use none of a boundary's records: every part
        // of every reachable observer is planned, however much the others
        // are owed.
        let observer_id = |index: usize| format!("observer.{index:02}");
        let mut crowded = observed_state(
            (0..MAX_FISCAL_OBSERVERS).map(|index| observer(&observer_id(index), institution(1))),
        );
        let aggregates: Vec<_> = (0..PARTS * FISCAL_REPORT_FACTS_PER_PART)
            .map(|cycle| {
                assess(
                    &mut crowded,
                    institution(1),
                    &format!("cycle.{cycle:03}"),
                    100,
                )
            })
            .collect();
        let unreachable: Vec<_> = (0..MAX_FISCAL_OBSERVERS / 2).map(observer_id).collect();
        let unreachable: Vec<_> = unreachable.iter().map(String::as_str).collect();
        let plan = derive_without(&mut crowded, aggregates, &unreachable).expect("a plan");
        assert_eq!(plan.publish.len(), MAX_FISCAL_OBSERVERS / 2);
        assert!(plan.publish.values().all(|parts| parts.len() == PARTS));
        assert_eq!(plan.deferred.len(), MAX_FISCAL_OBSERVERS / 2);
        assert!(
            plan.deferred
                .iter()
                .all(|(id, parts)| unreachable.contains(&id.as_str()) && parts.len() == PARTS)
        );
    }

    #[test]
    fn an_assembled_report_is_what_the_state_derives() {
        let mut state = observed_state([
            observer("observer.a", institution(1)),
            observer("observer.b", institution(2)),
        ]);
        // The odd cycles appear first and the even ones later, so the order
        // of the reports is not the order of the aggregates' keys.
        let mut aggregates = Vec::new();
        for parity in [1, 0] {
            aggregates.extend(
                (0..40_u64)
                    .filter(|cycle| cycle % 2 == parity)
                    .map(|cycle| {
                        assess(
                            &mut state,
                            institution(1 + cycle / 20),
                            &format!("cycle.{cycle:02}"),
                            100 + cycle,
                        )
                    }),
            );
            state.procedure_revision += 1;
            derive(&mut state, aggregates.clone()).expect("a plan");
        }
        // Publish every part, as the first plan of a state does.
        state.report_plan.revision = 0;
        derive(&mut state, aggregates).expect("a plan of every part");
        let at = SimTime::EPOCH + SimDuration::minutes(500);
        let records: Vec<Value> = planned_reports(&state, at)
            .into_iter()
            .flat_map(|report| {
                std::iter::once(FiscalReportRecord::Summary(report.summary))
                    .chain(report.parts.into_iter().map(FiscalReportRecord::Part))
            })
            .map(|record| serde_json::to_value(record).expect("record payload"))
            .collect();
        // Two summaries, and two parts for each observer's 20 facts.
        assert_eq!(records.len(), 6);

        let reports = assemble_fiscal_reports(&records).expect("assembled reports");
        let derived = compute_projections(&state, at);
        assert_eq!(reports.len(), derived.len());
        for (observer_id, report) in &reports {
            assert!(report.is_complete());
            assert_eq!(report.projection, derived[observer_id]);
        }
        let reported: Vec<_> = reports["observer.a"]
            .projection
            .facts
            .values()
            .map(|fact| fact.accounting_cycle_id.as_str())
            .collect();
        assert_eq!(reported[..2], ["cycle.01", "cycle.03"]);
        assert_eq!(reported[10..12], ["cycle.00", "cycle.02"]);

        // Records that do not make up whole reports are refused.
        let is_part = |record: &&Value| record["type"] == "part";
        let without_one_part: Vec<_> = records
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != 1)
            .map(|(_, record)| record)
            .collect();
        assert!(assemble_fiscal_reports(without_one_part).is_err());
        assert!(assemble_fiscal_reports(records.iter().filter(is_part)).is_err());
        assert!(assemble_fiscal_reports(records.iter().chain([&records[1]])).is_err());
        assert!(assemble_fiscal_reports([&serde_json::json!({"type": "other"})]).is_err());

        // A report whose summary counts pending parts may lack facts.
        let mut pending = records.clone();
        pending[0]["pending_parts"] = serde_json::json!([0]);
        pending.remove(1);
        let reports = assemble_fiscal_reports(&pending).expect("an incomplete report");
        assert!(!reports["observer.a"].is_complete());
        assert!(reports["observer.a"].is_stale("fact.0003"));
        assert!(!reports["observer.a"].is_stale("fact.0016"));
        assert!(!reports["observer.b"].is_stale("fact.0003"));
        assert_eq!(
            reports["observer.a"].projection.facts.len(),
            20 - FISCAL_REPORT_FACTS_PER_PART
        );
    }
}
