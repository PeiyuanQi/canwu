//! What the fiscal state derives: each jurisdiction's transition candidates,
//! and what each observer is told.

use crate::model::{
    CompiledFiscalCatalog, EMPTY_PART_ENTRY, FISCAL_REPORT_ROWS_PER_PART, FiscalAmountEstimate,
    FiscalCycleBook, FiscalCycleTotal, FiscalHistoricalContext, FiscalHistoricalMode,
    FiscalJurisdiction, FiscalMechanism, FiscalObserver, FiscalProjection, FiscalReport,
    FiscalReportFact, FiscalReportGranularity, FiscalReportPart, FiscalReportRecord,
    FiscalReportRow, FiscalReportRowFact, FiscalReportSummary, FiscalScopeBinding, FiscalState,
    FiscalTransitionCandidate, invalid, sum_report_totals,
};
use canwu_api::{CanwuError, EntityRef, SimTime, canonical_byte_hash};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// The transition candidates of one jurisdiction: each transition of the
/// catalog that names the region, is eligible at the context's year in its
/// mode, has a target rule for every mechanism the region's scopes lack
/// none of, has its source rules adopted and not repealed in the region (or
/// none), has not all its target rules operational there yet, and has every
/// prerequisite transition's target rules operational there.
///
/// It reads only the jurisdiction's counts, the catalog, and the context. A
/// candidate keeps the time it was first evaluated while whether it was
/// historically observed stays the same; a new one takes `at`.
#[must_use]
pub fn jurisdiction_candidates(
    jurisdiction: &FiscalJurisdiction,
    catalog: &CompiledFiscalCatalog,
    context: &FiscalHistoricalContext,
    at: SimTime,
) -> BTreeMap<String, FiscalTransitionCandidate> {
    let region = &jurisdiction.region_id;
    let operational = |rule: &String| {
        jurisdiction
            .adoption_counts
            .get(rule)
            .is_some_and(|count| count.operational > 0)
    };
    let mut candidates = BTreeMap::new();
    if jurisdiction.scope_count == 0 {
        return candidates;
    }
    for transition in catalog
        .transitions
        .values()
        .filter(|transition| transition.jurisdiction_ids.contains(region))
    {
        let time_eligible = match context.mode {
            FiscalHistoricalMode::Counterfactual => {
                transition.eligibility_window.contains(context.year)
            }
            FiscalHistoricalMode::RecordedBaseline | FiscalHistoricalMode::ResearchReplay => {
                transition.observed_window.contains(context.year)
            }
        };
        if !time_eligible
            || transition.to_rule_ids.iter().any(|rule_id| {
                catalog
                    .rules
                    .get(rule_id)
                    .is_none_or(|rule| !jurisdiction.mechanisms.contains_key(&rule.mechanism))
            })
        {
            continue;
        }
        let source_ready = transition.from_rule_ids.is_empty()
            || transition.from_rule_ids.iter().any(|rule| {
                jurisdiction
                    .adoption_counts
                    .get(rule)
                    .is_some_and(|count| count.live > 0)
            });
        let target_active = transition.to_rule_ids.iter().all(operational);
        let prerequisites_ready = transition.prerequisite_ids.iter().all(|required| {
            catalog
                .transitions
                .get(required)
                .is_some_and(|prerequisite| prerequisite.to_rule_ids.iter().all(operational))
        });
        if !source_ready || target_active || !prerequisites_ready {
            continue;
        }
        let id = format!("{}::{region}", transition.id);
        let historically_observed = transition.observed_window.contains(context.year);
        let evaluated_at = jurisdiction
            .transition_candidates
            .get(&id)
            .filter(|candidate| candidate.historically_observed == historically_observed)
            .map_or(at, |candidate| candidate.evaluated_at);
        candidates.insert(
            id.clone(),
            FiscalTransitionCandidate {
                id,
                transition_id: transition.id.clone(),
                jurisdiction_id: region.clone(),
                historically_observed,
                evaluated_at,
            },
        );
    }
    candidates
}

/// The regions whose candidates a change of the historical context from
/// `before` to `after` can change: those of every transition whose observed
/// or eligibility window holds exactly one of the two years, or of every
/// transition when the mode changes.
pub(crate) fn context_regions(
    catalog: &CompiledFiscalCatalog,
    before: &FiscalHistoricalContext,
    after: &FiscalHistoricalContext,
) -> BTreeSet<String> {
    catalog
        .transitions
        .values()
        .filter(|transition| {
            before.mode != after.mode
                || transition.observed_window.contains(before.year)
                    != transition.observed_window.contains(after.year)
                || transition.eligibility_window.contains(before.year)
                    != transition.eligibility_window.contains(after.year)
        })
        .flat_map(|transition| transition.jurisdiction_ids.iter().cloned())
        .collect()
}

/// The row of one scope's book in a report at `confidence_per_mille`, or
/// `None` when the book has no assessment yet.
pub(crate) fn report_row(
    row: u32,
    scope: &FiscalScopeBinding,
    book: &FiscalCycleBook,
    confidence_per_mille: u16,
) -> Result<Option<FiscalReportRow>, CanwuError> {
    Ok(report_row_of_totals(
        row,
        scope,
        &book.totals(scope.mechanism)?,
        confidence_per_mille,
    ))
}

/// The row of a scope whose book of the cycle has these totals.
pub(crate) fn report_row_of_totals(
    row: u32,
    scope: &FiscalScopeBinding,
    totals: &[FiscalCycleTotal],
    confidence_per_mille: u16,
) -> Option<FiscalReportRow> {
    rendered_row(
        row,
        &scope.institution,
        scope.mechanism,
        &scope.id,
        totals,
        confidence_per_mille,
    )
}

/// The row of an institution and mechanism whose books of the cycle sum to
/// these totals: an institution row, with no scope.
pub(crate) fn institution_row_of_totals(
    row: u32,
    institution: &EntityRef,
    mechanism: FiscalMechanism,
    totals: &[FiscalCycleTotal],
    confidence_per_mille: u16,
) -> Option<FiscalReportRow> {
    rendered_row(
        row,
        institution,
        mechanism,
        "",
        totals,
        confidence_per_mille,
    )
}

/// The row of `institution` and `mechanism` whose books of the cycle have
/// these totals: a scope row when `scope_binding_id` names the scope, an
/// institution row when it is empty. `None` when there is no total.
pub(crate) fn rendered_row(
    row: u32,
    institution: &EntityRef,
    mechanism: FiscalMechanism,
    scope_binding_id: &str,
    totals: &[FiscalCycleTotal],
    confidence_per_mille: u16,
) -> Option<FiscalReportRow> {
    let facts: Vec<_> = totals
        .iter()
        .map(|total| FiscalReportRowFact {
            unit: total.unit.clone(),
            payment_form: total.payment_form,
            assessed: estimate(total.assessed, confidence_per_mille),
            collected: estimate(total.collected, confidence_per_mille),
            outstanding: estimate(total.outstanding, confidence_per_mille),
        })
        .collect();
    (!facts.is_empty()).then(|| FiscalReportRow {
        row,
        institution: institution.clone(),
        mechanism,
        scope_binding_id: scope_binding_id.to_owned(),
        facts,
    })
}

/// The entry an observer record keeps for a part with these rows: sixteen
/// hexadecimal digits of the digest of the rows and two of their fact count,
/// or [`EMPTY_PART_ENTRY`] for no rows.
///
/// The digest is over a compact binary encoding of the rendered rows, every
/// field in order with each text length-delimited, built into `buffer`: the
/// rows are encoded once, not as JSON, so that comparing a part costs a
/// fraction of publishing it. It only decides whether to republish, so it is
/// never stored outside the observer record.
pub(crate) fn part_entry(rows: &[FiscalReportRow], buffer: &mut Vec<u8>) -> String {
    if rows.is_empty() {
        return EMPTY_PART_ENTRY.to_owned();
    }
    buffer.clear();
    let text = |buffer: &mut Vec<u8>, value: &[u8]| {
        buffer.extend_from_slice(&u64::try_from(value.len()).unwrap_or(u64::MAX).to_le_bytes());
        buffer.extend_from_slice(value);
    };
    for row in rows {
        buffer.extend_from_slice(&row.row.to_le_bytes());
        text(buffer, row.institution.to_string().as_bytes());
        buffer.push(row.mechanism as u8);
        text(buffer, row.scope_binding_id.as_bytes());
        buffer.extend_from_slice(
            &u32::try_from(row.facts.len())
                .unwrap_or(u32::MAX)
                .to_le_bytes(),
        );
        for fact in &row.facts {
            text(buffer, fact.unit.as_bytes());
            buffer.push(fact.payment_form as u8);
            for amount in [&fact.assessed, &fact.collected, &fact.outstanding] {
                buffer.extend_from_slice(&amount.minimum.to_le_bytes());
                buffer.extend_from_slice(&amount.maximum.to_le_bytes());
            }
        }
    }
    let digest = canonical_byte_hash("canwu.fiscal.report-part.v2", buffer);
    let mut digest = digest[..16].to_owned();
    if digest == EMPTY_PART_ENTRY[..16] {
        digest.replace_range(15..16, "1");
    }
    let facts: usize = rows.iter().map(|row| row.facts.len()).sum();
    format!("{digest}{:02x}", facts.min(0xff))
}

/// The part that holds row `row`.
#[must_use]
pub(crate) fn part_of_row(row: u32) -> u32 {
    row / u32::try_from(FISCAL_REPORT_ROWS_PER_PART).unwrap_or(u32::MAX)
}

/// How many parts a cycle of a report of `rows` rows has.
#[must_use]
pub(crate) fn parts_of(rows: u32) -> u32 {
    rows.div_ceil(u32::try_from(FISCAL_REPORT_ROWS_PER_PART).unwrap_or(u32::MAX))
}

/// What one observer of a whole `state` is told as of `at`: the facts of
/// every hot book of a visible institution's scope in each cycle the
/// observer's report shows, or, at
/// [`crate::FiscalReportGranularity::Institution`], those facts summed per
/// institution and mechanism, labelled as [`FiscalReportFact::label`] says.
///
/// This is what a complete report assembles to at the end of a boundary
/// that published it, with `state` the fiscal records at the end of that
/// boundary (see [`assemble_fiscal_reports`]). An `OnChange` report is
/// published in every boundary that changes what it shows; an `Interval`
/// report only at its due times, so between them it shows the records as of
/// its `as_of` boundary. `None` for an unknown or removed binding.
pub fn compute_projection(
    state: &FiscalState,
    observer_binding_id: &str,
    at: SimTime,
) -> Result<Option<FiscalProjection>, CanwuError> {
    let Some(observer) = state
        .observers
        .get(observer_binding_id)
        .filter(|observer| observer.binding.is_active())
    else {
        return Ok(None);
    };
    let mut facts = BTreeMap::new();
    for cycle in observer.report.cycles.keys() {
        for row in observer_rows(state, observer, cycle)? {
            push_facts(&mut facts, cycle, row);
        }
    }
    Ok(Some(FiscalProjection {
        actor: observer.binding.actor,
        as_of: at,
        confidence_per_mille: observer.binding.confidence_per_mille,
        facts,
    }))
}

/// Every row an observer of a whole state has in one cycle, in row order.
fn observer_rows(
    state: &FiscalState,
    observer: &FiscalObserver,
    cycle: &str,
) -> Result<Vec<FiscalReportRow>, CanwuError> {
    let confidence = observer.binding.confidence_per_mille;
    let mut rows = Vec::new();
    for block in &observer.rows {
        let ledgers = state
            .ledgers
            .values()
            .filter(|ledger| ledger.office == block.office);
        match observer.binding.granularity {
            FiscalReportGranularity::Scope => {
                for ledger in ledgers {
                    let Some(book) = ledger.books.get(cycle) else {
                        continue;
                    };
                    if let Some(row) = report_row(
                        block.first_row.saturating_add(ledger.rank),
                        &ledger.scope,
                        book,
                        confidence,
                    )? {
                        rows.push(row);
                    }
                }
            }
            FiscalReportGranularity::Institution => {
                let mut sums = BTreeMap::<FiscalMechanism, Vec<FiscalCycleTotal>>::new();
                for ledger in ledgers {
                    if let Some(book) = ledger.books.get(cycle) {
                        sum_report_totals(
                            sums.entry(ledger.scope.mechanism).or_default(),
                            &book.totals(ledger.scope.mechanism)?,
                        );
                    }
                }
                for (index, mechanism) in block.mechanisms.iter().enumerate() {
                    let row = block
                        .first_row
                        .saturating_add(u32::try_from(index).unwrap_or(u32::MAX));
                    if let Some(row) = sums.get(mechanism).and_then(|totals| {
                        institution_row_of_totals(
                            row,
                            &block.institution,
                            *mechanism,
                            totals,
                            confidence,
                        )
                    }) {
                        rows.push(row);
                    }
                }
            }
        }
    }
    rows.sort_by_key(|row| row.row);
    Ok(rows)
}

fn push_facts(facts: &mut BTreeMap<String, FiscalReportFact>, cycle: &str, row: FiscalReportRow) {
    for (index, fact) in row.facts.into_iter().enumerate() {
        let id = FiscalReportFact::label(cycle, row.row, index);
        facts.insert(
            id.clone(),
            FiscalReportFact {
                id,
                institution: row.institution.clone(),
                mechanism: row.mechanism,
                scope_binding_id: row.scope_binding_id.clone(),
                accounting_cycle_id: cycle.to_owned(),
                unit: fact.unit,
                payment_form: fact.payment_form,
                assessed: fact.assessed,
                collected: fact.collected,
                outstanding: fact.outstanding,
            },
        );
    }
}

/// Reassembles fiscal reports from the payloads of the fiscal report records
/// one holder currently holds, keyed by observer binding ID.
///
/// Pass every current record of [`crate::fiscal_report_knowledge_schema_id`]
/// the holder has, and no superseded one; [`crate::fiscal_reports`] does that
/// for a viewer. A report is its summary and the parts of the cycles the
/// summary lists, on the slot it names for each; a part of another cycle or
/// slot is left over from an earlier cycle and not part of the report. Each
/// fact is labelled as [`FiscalReportFact::label`] says, so a complete report
/// equals what [`compute_projection`] derives from the fiscal state it was
/// published from. In an incomplete report the facts of a pending part are
/// the older ones the holder has, or missing when it has none;
/// [`FiscalReport::pending`] names the pending parts. A report whose summary
/// is withdrawn is left out with its parts.
///
/// # Errors
///
/// Returns an error when a payload is not a fiscal report record, when two
/// records fill one summary or part, when a part has no summary, or when the
/// parts of a complete report do not hold the facts its summary counts.
pub fn assemble_fiscal_reports<'a>(
    payloads: impl IntoIterator<Item = &'a Value>,
) -> Result<BTreeMap<String, FiscalReport>, CanwuError> {
    let mut summaries = BTreeMap::<String, FiscalReportSummary>::new();
    let mut parts = BTreeMap::<String, BTreeMap<(u16, u32), FiscalReportPart>>::new();
    for payload in payloads {
        let duplicate = match FiscalReportRecord::deserialize(payload)
            .map_err(|error| invalid(format!("fiscal report record is invalid: {error}")))?
        {
            FiscalReportRecord::Summary(summary) => summaries
                .insert(summary.observer_id.clone(), summary)
                .is_some(),
            FiscalReportRecord::Part(part) => parts
                .entry(part.observer_id.clone())
                .or_default()
                .insert((part.slot, part.part), part)
                .is_some(),
        };
        if duplicate {
            return Err(invalid(
                "two current fiscal report records fill the same summary or part",
            ));
        }
    }
    let mut reports = BTreeMap::new();
    for (observer_id, summary) in summaries {
        let held = parts.remove(&observer_id).unwrap_or_default();
        if summary.withdrawn {
            continue;
        }
        let mut facts = BTreeMap::new();
        for part in held.into_values() {
            if summary.cycles.get(&part.accounting_cycle_id) != Some(&part.slot) {
                continue;
            }
            for row in part.rows {
                push_facts(&mut facts, &part.accounting_cycle_id, row);
            }
        }
        if summary.pending.is_empty()
            && u32::try_from(facts.len()).unwrap_or(u32::MAX) != summary.fact_count
        {
            return Err(invalid(
                "fiscal report parts do not hold the facts their summary counts",
            ));
        }
        reports.insert(
            observer_id,
            FiscalReport {
                projection: FiscalProjection {
                    actor: summary.actor,
                    as_of: summary.as_of,
                    confidence_per_mille: summary.confidence_per_mille,
                    facts,
                },
                pending: summary.pending,
                omitted_cycles: summary.omitted_cycles,
            },
        );
    }
    if !parts.is_empty() {
        return Err(invalid("a fiscal report part has no summary"));
    }
    Ok(reports)
}

pub(crate) fn estimate(value: u64, confidence_per_mille: u16) -> FiscalAmountEstimate {
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
    FiscalAmountEstimate {
        minimum,
        maximum: minimum.saturating_add(bucket_width - 1),
    }
}
