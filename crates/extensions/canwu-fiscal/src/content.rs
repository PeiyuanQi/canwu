use crate::model::{
    CompiledFiscalCatalog, FISCAL_CONTENT_SCHEMA_VERSION, FiscalContentPack,
    FiscalContentSelection, FiscalCoverageCell, FiscalCoverageDeclaration, FiscalCoverageStatus,
    FiscalInstitutionDefinition, FiscalMechanism, FiscalPeriodDefinition, FiscalProvenance,
    FiscalRegionDefinition, FiscalRuleDefinition, FiscalTransitionDefinition,
    MAX_FISCAL_CATALOG_COVERAGE_CELLS, MAX_FISCAL_CATALOG_DEFINITIONS,
    MAX_FISCAL_CATALOG_JSON_BYTES, MAX_FISCAL_CATALOG_PERIODS, MAX_FISCAL_CATALOG_REGIONS,
    MAX_FISCAL_REFERENCES_PER_DEFINITION, invalid, validate_identifier,
};
use canwu_api::{CanwuError, canonical_hash};
use std::collections::{BTreeMap, BTreeSet};

/// Validates and compiles an authored fiscal pack into one immutable run catalog.
///
/// Empty region and mechanism selections mean "all declared values". Coverage
/// is resolved by explicit numeric priority. Equal-priority overlaps fail
/// closed, so file order never changes the selected historical interpretation.
#[allow(clippy::too_many_lines)]
pub fn compile_fiscal_content(
    pack: &FiscalContentPack,
    mut selection: FiscalContentSelection,
) -> Result<CompiledFiscalCatalog, CanwuError> {
    let validated = validate_pack(pack)?;
    if !pack
        .manifest
        .historical_scope
        .contains(selection.historical_year)
    {
        return Err(invalid(
            "fiscal selection lies outside the pack's historical scope",
        ));
    }
    if selection.region_ids.is_empty() {
        selection.region_ids = validated.regions.keys().cloned().collect();
    }
    if selection.mechanisms.is_empty() {
        selection.mechanisms = pack.manifest.mechanisms.iter().copied().collect();
    }
    reject_unknown_selection(&selection, &validated.regions, &pack.manifest.mechanisms)?;

    let selected_period_ids: BTreeSet<_> = validated
        .periods
        .values()
        .filter(|period| period.window.contains(selection.historical_year))
        .map(|period| period.id.clone())
        .collect();
    if selected_period_ids.is_empty() {
        return Err(invalid(format!(
            "no fiscal period covers historical year {}",
            selection.historical_year
        )));
    }

    let selected_coverage: BTreeMap<_, _> = validated
        .coverage
        .iter()
        .filter(|(_, cell)| {
            selection.region_ids.contains(&cell.region_id)
                && selection.mechanisms.contains(&cell.mechanism)
        })
        .map(|(id, cell)| (id.clone(), cell.clone()))
        .collect();
    let selected_definition_ids: BTreeSet<_> = selected_coverage
        .values()
        .flat_map(|cell| cell.definition_ids.iter().cloned())
        .collect();

    let rules: BTreeMap<_, _> = validated
        .rules
        .iter()
        .filter(|(_, rule)| {
            selection.mechanisms.contains(&rule.mechanism)
                && (selected_definition_ids.contains(rule.id.as_str())
                    || rule
                        .jurisdiction_ids
                        .iter()
                        .any(|region| selection.region_ids.contains(region)))
        })
        .map(|(id, value)| {
            let mut value = value.clone();
            value
                .jurisdiction_ids
                .retain(|region| selection.region_ids.contains(region));
            (id.clone(), value)
        })
        .collect();
    let rule_ids: BTreeSet<_> = rules.keys().cloned().collect();
    let mut transitions: BTreeMap<_, _> = validated
        .transitions
        .iter()
        .filter(|(_, transition)| {
            (selected_definition_ids.contains(transition.id.as_str())
                || transition
                    .jurisdiction_ids
                    .iter()
                    .any(|region| selection.region_ids.contains(region)))
                && transition
                    .from_rule_ids
                    .iter()
                    .chain(&transition.to_rule_ids)
                    .chain(&transition.supersedes_or_suspends)
                    .all(|rule| rule_ids.contains(rule))
        })
        .map(|(id, value)| {
            let mut value = value.clone();
            value
                .jurisdiction_ids
                .retain(|region| selection.region_ids.contains(region));
            (id.clone(), value)
        })
        .collect();
    loop {
        let available: BTreeSet<_> = transitions.keys().cloned().collect();
        let before = transitions.len();
        transitions.retain(|_, transition| {
            transition
                .prerequisite_ids
                .iter()
                .all(|required| available.contains(required))
        });
        if transitions.len() == before {
            break;
        }
    }
    let institutions: BTreeMap<_, _> = validated
        .institutions
        .iter()
        .filter(|(_, institution)| {
            selected_definition_ids.contains(institution.id.as_str())
                || institution
                    .region_ids
                    .iter()
                    .any(|region| selection.region_ids.contains(region))
        })
        .map(|(id, value)| {
            let mut value = value.clone();
            value
                .region_ids
                .retain(|region| selection.region_ids.contains(region));
            (id.clone(), value)
        })
        .collect();
    let regions: BTreeMap<_, _> = validated
        .regions
        .iter()
        .filter(|(id, _)| selection.region_ids.contains(*id))
        .map(|(id, value)| (id.clone(), value.clone()))
        .collect();
    let definition_ids: BTreeSet<_> = rules
        .keys()
        .chain(institutions.keys())
        .chain(transitions.keys())
        .cloned()
        .collect();
    let coverage: BTreeMap<_, _> = selected_coverage
        .into_iter()
        .map(|(id, mut cell)| {
            cell.definition_ids.retain(|definition| {
                definition_ids.contains(definition)
                    && (institutions.contains_key(definition)
                        || rules
                            .get(definition)
                            .is_some_and(|rule| rule.mechanism == cell.mechanism)
                        || transitions.get(definition).is_some_and(|transition| {
                            transition
                                .from_rule_ids
                                .iter()
                                .chain(&transition.to_rule_ids)
                                .any(|rule_id| {
                                    rules
                                        .get(rule_id)
                                        .is_some_and(|rule| rule.mechanism == cell.mechanism)
                                })
                        }))
            });
            (id, cell)
        })
        .collect();
    let catalog = CompiledFiscalCatalog {
        schema_version: FISCAL_CONTENT_SCHEMA_VERSION,
        pack_id: pack.manifest.pack_id.clone(),
        pack_version: pack.manifest.pack_version.clone(),
        content_hash: canonical_hash("canwu.fiscal.content-pack.v1", pack)?,
        historical_year: selection.historical_year,
        historical_scope: pack.manifest.historical_scope.clone(),
        selected_period_ids,
        periods: validated.periods,
        regions,
        institutions,
        rules,
        transitions,
        coverage,
        provenance: validated.provenance,
    };
    catalog.validate()?;
    Ok(catalog)
}

struct ValidatedPack {
    periods: BTreeMap<String, FiscalPeriodDefinition>,
    regions: BTreeMap<String, FiscalRegionDefinition>,
    institutions: BTreeMap<String, FiscalInstitutionDefinition>,
    rules: BTreeMap<String, FiscalRuleDefinition>,
    transitions: BTreeMap<String, FiscalTransitionDefinition>,
    provenance: BTreeMap<String, FiscalProvenance>,
    coverage: BTreeMap<String, FiscalCoverageCell>,
}

#[allow(clippy::too_many_lines)]
fn validate_pack(pack: &FiscalContentPack) -> Result<ValidatedPack, CanwuError> {
    let encoded_len = serde_json::to_vec(pack)
        .map_err(|error| invalid(format!("fiscal content pack could not be sized: {error}")))?
        .len();
    if encoded_len > MAX_FISCAL_CATALOG_JSON_BYTES {
        return Err(invalid(
            "fiscal content pack exceeds its serialized byte budget",
        ));
    }
    if pack.manifest.schema_version != FISCAL_CONTENT_SCHEMA_VERSION
        || pack.manifest.license != "Apache-2.0"
    {
        return Err(invalid(
            "fiscal pack requires the supported schema and Apache-2.0 content license",
        ));
    }
    validate_identifier(&pack.manifest.pack_id, "pack")?;
    semver::Version::parse(&pack.manifest.pack_version)
        .map_err(|error| invalid(format!("fiscal pack version is not SemVer: {error}")))?;
    pack.manifest
        .historical_scope
        .validate_for_content("pack historical scope")?;
    if pack.periods.len() > MAX_FISCAL_CATALOG_PERIODS
        || pack.regions.len() > MAX_FISCAL_CATALOG_REGIONS
        || pack.institutions.len() > MAX_FISCAL_CATALOG_DEFINITIONS
        || pack.rules.len() > MAX_FISCAL_CATALOG_DEFINITIONS
        || pack.transitions.len() > MAX_FISCAL_CATALOG_DEFINITIONS
        || pack.provenance.len() > MAX_FISCAL_CATALOG_DEFINITIONS
        || pack.coverage.len() > MAX_FISCAL_CATALOG_COVERAGE_CELLS
    {
        return Err(invalid("fiscal content pack exceeds its bounded capacity"));
    }
    if pack.manifest.period_ids.len() > MAX_FISCAL_CATALOG_PERIODS
        || pack.manifest.region_ids.len() > MAX_FISCAL_CATALOG_REGIONS
        || pack.institutions.iter().any(|value| {
            value.region_ids.len() > MAX_FISCAL_REFERENCES_PER_DEFINITION
                || value.provenance_ids.len() > MAX_FISCAL_REFERENCES_PER_DEFINITION
        })
        || pack.rules.iter().any(|value| {
            value.jurisdiction_ids.len() > MAX_FISCAL_REFERENCES_PER_DEFINITION
                || value.earmark_ids.len() > MAX_FISCAL_REFERENCES_PER_DEFINITION
                || value.provenance_ids.len() > MAX_FISCAL_REFERENCES_PER_DEFINITION
        })
        || pack.transitions.iter().any(|value| {
            value.from_rule_ids.len() > MAX_FISCAL_REFERENCES_PER_DEFINITION
                || value.to_rule_ids.len() > MAX_FISCAL_REFERENCES_PER_DEFINITION
                || value.jurisdiction_ids.len() > MAX_FISCAL_REFERENCES_PER_DEFINITION
                || value.supersedes_or_suspends.len() > MAX_FISCAL_REFERENCES_PER_DEFINITION
                || value.prerequisite_ids.len() > MAX_FISCAL_REFERENCES_PER_DEFINITION
                || value.provenance_ids.len() > MAX_FISCAL_REFERENCES_PER_DEFINITION
        })
        || pack.coverage.iter().any(|value| {
            value.selector.period_ids.len() > MAX_FISCAL_REFERENCES_PER_DEFINITION
                || value.selector.region_ids.len() > MAX_FISCAL_REFERENCES_PER_DEFINITION
                || value.definition_ids.len() > MAX_FISCAL_REFERENCES_PER_DEFINITION
                || value.provenance_ids.len() > MAX_FISCAL_REFERENCES_PER_DEFINITION
        })
        || pack
            .provenance
            .iter()
            .any(|value| value.forbidden_inferences.len() > MAX_FISCAL_REFERENCES_PER_DEFINITION)
    {
        return Err(invalid(
            "fiscal content pack nested collection exceeds its bounded capacity",
        ));
    }
    let periods = unique_map(&pack.periods, |value| &value.id, "period")?;
    let regions = unique_map(&pack.regions, |value| &value.id, "region")?;
    let institutions = unique_map(&pack.institutions, |value| &value.id, "institution")?;
    let rules = unique_map(&pack.rules, |value| &value.id, "rule")?;
    let transitions = unique_map(&pack.transitions, |value| &value.id, "transition")?;
    let provenance = unique_map(&pack.provenance, |value| &value.id, "provenance")?;
    let declarations = unique_map(&pack.coverage, |value| &value.id, "coverage declaration")?;
    for source in provenance.values() {
        if source.citation.trim().is_empty()
            || source.url.trim().is_empty()
            || source.claim_scope.trim().is_empty()
            || source.forbidden_inferences.is_empty()
            || source
                .forbidden_inferences
                .iter()
                .any(|inference| inference.trim().is_empty())
        {
            return Err(invalid(format!(
                "fiscal provenance {} is incomplete",
                source.id
            )));
        }
    }
    if periods.keys().cloned().collect::<Vec<_>>()
        != sorted_unique(&pack.manifest.period_ids, "manifest period")?
        || regions.keys().cloned().collect::<Vec<_>>()
            != sorted_unique(&pack.manifest.region_ids, "manifest region")?
    {
        return Err(invalid(
            "manifest dimensions do not exactly match their fiscal definitions",
        ));
    }
    let manifest_mechanisms: BTreeSet<_> = pack.manifest.mechanisms.iter().copied().collect();
    if manifest_mechanisms.len() != pack.manifest.mechanisms.len() || manifest_mechanisms.is_empty()
    {
        return Err(invalid("manifest mechanisms must be unique and non-empty"));
    }
    for period in periods.values() {
        period.window.validate_for_content("fiscal period window")?;
        if period.window.start < pack.manifest.historical_scope.start
            || period.window.end > pack.manifest.historical_scope.end
        {
            return Err(invalid(format!(
                "period {} lies outside the pack scope",
                period.id
            )));
        }
    }
    for institution in institutions.values() {
        if institution.provenance_ids.is_empty() {
            return Err(invalid(format!(
                "institution {} has no provenance",
                institution.id
            )));
        }
        require_refs(&institution.region_ids, &regions, "institution region")?;
        require_refs(
            &institution.provenance_ids,
            &provenance,
            "institution provenance",
        )?;
    }
    for rule in rules.values() {
        rule.legal_window
            .validate_for_content("fiscal rule legal window")?;
        if rule.revision == 0
            || rule.payment_forms.is_empty()
            || rule.provenance_ids.is_empty()
            || !manifest_mechanisms.contains(&rule.mechanism)
            || rule.legal_window.start < pack.manifest.historical_scope.start
            || rule.legal_window.end > pack.manifest.historical_scope.end
        {
            return Err(invalid(format!("fiscal rule {} is incomplete", rule.id)));
        }
        require_refs(&rule.jurisdiction_ids, &regions, "rule jurisdiction")?;
        require_refs(&rule.provenance_ids, &provenance, "rule provenance")?;
    }
    for transition in transitions.values() {
        transition
            .observed_window
            .validate_for_content("transition observed window")?;
        transition
            .eligibility_window
            .validate_for_content("transition eligibility window")?;
        if transition.provenance_ids.is_empty()
            || transition.observed_window.start < pack.manifest.historical_scope.start
            || transition.observed_window.end > pack.manifest.historical_scope.end
            || transition.eligibility_window.start < pack.manifest.historical_scope.start
            || transition.eligibility_window.end > pack.manifest.historical_scope.end
        {
            return Err(invalid(format!(
                "fiscal transition {} is unproven or outside the pack scope",
                transition.id
            )));
        }
        require_refs(&transition.from_rule_ids, &rules, "transition source rule")?;
        require_refs(&transition.to_rule_ids, &rules, "transition target rule")?;
        require_refs(
            &transition.supersedes_or_suspends,
            &rules,
            "transition suspended rule",
        )?;
        require_refs(
            &transition.prerequisite_ids,
            &transitions,
            "transition prerequisite",
        )?;
        require_refs(
            &transition.jurisdiction_ids,
            &regions,
            "transition jurisdiction",
        )?;
        require_refs(
            &transition.provenance_ids,
            &provenance,
            "transition provenance",
        )?;
    }
    reject_transition_cycles(&transitions)?;
    for declaration in declarations.values() {
        validate_selector(
            declaration,
            &periods,
            &regions,
            &manifest_mechanisms,
            &rules,
            &institutions,
            &transitions,
            &provenance,
        )?;
    }
    let coverage = materialize_coverage(
        &periods,
        &regions,
        &manifest_mechanisms,
        declarations.values(),
    )?;
    Ok(ValidatedPack {
        periods,
        regions,
        institutions,
        rules,
        transitions,
        provenance,
        coverage,
    })
}

#[allow(clippy::too_many_arguments)]
fn validate_selector(
    declaration: &FiscalCoverageDeclaration,
    periods: &BTreeMap<String, FiscalPeriodDefinition>,
    regions: &BTreeMap<String, FiscalRegionDefinition>,
    mechanisms: &BTreeSet<FiscalMechanism>,
    rules: &BTreeMap<String, FiscalRuleDefinition>,
    institutions: &BTreeMap<String, FiscalInstitutionDefinition>,
    transitions: &BTreeMap<String, FiscalTransitionDefinition>,
    provenance: &BTreeMap<String, FiscalProvenance>,
) -> Result<(), CanwuError> {
    require_refs(&declaration.selector.period_ids, periods, "coverage period")?;
    require_refs(&declaration.selector.region_ids, regions, "coverage region")?;
    if declaration
        .selector
        .mechanisms
        .iter()
        .any(|mechanism| !mechanisms.contains(mechanism))
    {
        return Err(invalid(format!(
            "coverage declaration {} uses an undeclared mechanism",
            declaration.id
        )));
    }
    for definition in &declaration.definition_ids {
        if !rules.contains_key(definition)
            && !institutions.contains_key(definition)
            && !transitions.contains_key(definition)
        {
            return Err(invalid(format!(
                "coverage declaration {} references unknown definition {definition}",
                declaration.id
            )));
        }
    }
    require_refs(
        &declaration.provenance_ids,
        provenance,
        "coverage provenance",
    )?;
    if matches!(
        declaration.status,
        FiscalCoverageStatus::Supported | FiscalCoverageStatus::ArchetypeFallback
    ) && declaration.provenance_ids.is_empty()
    {
        return Err(invalid(format!(
            "coverage declaration {} has no provenance",
            declaration.id
        )));
    }
    if matches!(
        declaration.status,
        FiscalCoverageStatus::Supported | FiscalCoverageStatus::ArchetypeFallback
    ) && declaration.definition_ids.is_empty()
    {
        return Err(invalid(format!(
            "coverage declaration {} promises content without a definition",
            declaration.id
        )));
    }
    if matches!(
        declaration.status,
        FiscalCoverageStatus::ExplicitUnknown | FiscalCoverageStatus::NotApplicable
    ) && !declaration.definition_ids.is_empty()
    {
        return Err(invalid(format!(
            "coverage declaration {} carries definitions for a non-behavioral status",
            declaration.id
        )));
    }
    Ok(())
}

fn reject_transition_cycles(
    transitions: &BTreeMap<String, FiscalTransitionDefinition>,
) -> Result<(), CanwuError> {
    let mut remaining: BTreeMap<_, _> = transitions
        .iter()
        .map(|(id, transition)| (id.clone(), transition.prerequisite_ids.clone()))
        .collect();
    let mut resolved = BTreeSet::new();
    while !remaining.is_empty() {
        let ready: Vec<_> = remaining
            .iter()
            .filter(|(_, prerequisites)| prerequisites.is_subset(&resolved))
            .map(|(id, _)| id.clone())
            .collect();
        if ready.is_empty() {
            return Err(invalid("fiscal transition prerequisites contain a cycle"));
        }
        for id in ready {
            remaining.remove(&id);
            resolved.insert(id);
        }
    }
    Ok(())
}

fn materialize_coverage<'a>(
    periods: &BTreeMap<String, FiscalPeriodDefinition>,
    regions: &BTreeMap<String, FiscalRegionDefinition>,
    mechanisms: &BTreeSet<FiscalMechanism>,
    declarations: impl Iterator<Item = &'a FiscalCoverageDeclaration>,
) -> Result<BTreeMap<String, FiscalCoverageCell>, CanwuError> {
    ensure_coverage_cell_capacity(periods.len(), regions.len(), mechanisms.len())?;
    let declarations: Vec<_> = declarations.collect();
    let mut cells = BTreeMap::new();
    for period in periods.keys() {
        for region in regions.keys() {
            for mechanism in mechanisms {
                if cells.len() >= MAX_FISCAL_CATALOG_COVERAGE_CELLS {
                    return Err(invalid(
                        "fiscal coverage exceeds its materialized cell capacity",
                    ));
                }
                let mut matches: Vec<_> = declarations
                    .iter()
                    .filter(|declaration| {
                        selector_matches(&declaration.selector.period_ids, period)
                            && selector_matches(&declaration.selector.region_ids, region)
                            && (declaration.selector.mechanisms.is_empty()
                                || declaration.selector.mechanisms.contains(mechanism))
                    })
                    .collect();
                matches.sort_by_key(|declaration| declaration.priority);
                let Some(selected) = matches.last() else {
                    return Err(invalid(format!(
                        "coverage is missing for {period}/{region}/{mechanism:?}"
                    )));
                };
                if matches.len() > 1 && matches[matches.len() - 2].priority == selected.priority {
                    return Err(invalid(format!(
                        "coverage has an equal-priority conflict for {period}/{region}/{mechanism:?}"
                    )));
                }
                let id = coverage_id(period, region, *mechanism);
                cells.insert(
                    id.clone(),
                    FiscalCoverageCell {
                        id,
                        period_id: period.clone(),
                        region_id: region.clone(),
                        mechanism: *mechanism,
                        status: selected.status,
                        definition_ids: selected.definition_ids.clone(),
                        provenance_ids: selected.provenance_ids.clone(),
                        declaration_id: selected.id.clone(),
                    },
                );
            }
        }
    }
    Ok(cells)
}

fn ensure_coverage_cell_capacity(
    period_count: usize,
    region_count: usize,
    mechanism_count: usize,
) -> Result<usize, CanwuError> {
    let cell_count = period_count
        .checked_mul(region_count)
        .and_then(|count| count.checked_mul(mechanism_count))
        .ok_or_else(|| invalid("fiscal coverage cell count overflowed"))?;
    if cell_count > MAX_FISCAL_CATALOG_COVERAGE_CELLS {
        return Err(invalid(
            "fiscal coverage exceeds its materialized cell capacity",
        ));
    }
    Ok(cell_count)
}

fn coverage_id(period: &str, region: &str, mechanism: FiscalMechanism) -> String {
    format!("{period}::{region}::{mechanism:?}").to_ascii_lowercase()
}

fn selector_matches(values: &BTreeSet<String>, value: &str) -> bool {
    values.is_empty() || values.contains(value)
}

fn reject_unknown_selection(
    selection: &FiscalContentSelection,
    regions: &BTreeMap<String, FiscalRegionDefinition>,
    mechanisms: &[FiscalMechanism],
) -> Result<(), CanwuError> {
    if let Some(region) = selection
        .region_ids
        .iter()
        .find(|region| !regions.contains_key(*region))
    {
        return Err(invalid(format!(
            "selected fiscal region {region} is unknown"
        )));
    }
    let mechanisms: BTreeSet<_> = mechanisms.iter().copied().collect();
    if selection
        .mechanisms
        .iter()
        .any(|mechanism| !mechanisms.contains(mechanism))
    {
        return Err(invalid(
            "selected fiscal mechanism is not declared by the pack",
        ));
    }
    Ok(())
}

fn unique_map<T: Clone>(
    values: &[T],
    id: impl Fn(&T) -> &String,
    label: &str,
) -> Result<BTreeMap<String, T>, CanwuError> {
    let mut result = BTreeMap::new();
    for value in values {
        validate_identifier(id(value), label)?;
        if result.insert(id(value).clone(), value.clone()).is_some() {
            return Err(invalid(format!("duplicate {label} {}", id(value))));
        }
    }
    Ok(result)
}

fn sorted_unique(values: &[String], label: &str) -> Result<Vec<String>, CanwuError> {
    let mut result = values.to_vec();
    for value in &result {
        validate_identifier(value, label)?;
    }
    result.sort();
    result.dedup();
    if result.len() != values.len() {
        return Err(invalid(format!("{label} values are not unique")));
    }
    Ok(result)
}

fn require_refs<T>(
    refs: &BTreeSet<String>,
    values: &BTreeMap<String, T>,
    label: &str,
) -> Result<(), CanwuError> {
    if let Some(missing) = refs
        .iter()
        .find(|reference| !values.contains_key(*reference))
    {
        return Err(invalid(format!("{label} {missing} is unavailable")));
    }
    Ok(())
}

trait ContentWindowValidation {
    fn validate_for_content(&self, label: &str) -> Result<(), CanwuError>;
}

impl ContentWindowValidation for crate::model::HistoricalYearWindow {
    fn validate_for_content(&self, label: &str) -> Result<(), CanwuError> {
        if self.start > self.end {
            return Err(invalid(format!("{label} starts after it ends")));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_product_is_rejected_when_each_axis_is_individually_legal() {
        let periods = 257;
        let regions = 256;
        let mechanisms = BTreeSet::from([FiscalMechanism::LandTax]);
        assert!(periods <= MAX_FISCAL_CATALOG_PERIODS);
        assert!(regions <= MAX_FISCAL_CATALOG_REGIONS);
        assert_eq!(mechanisms.len(), 1);
        assert_eq!(periods * regions * mechanisms.len(), 65_792);

        let error = ensure_coverage_cell_capacity(periods, regions, mechanisms.len())
            .expect_err("the Cartesian product must fail before coverage materialization");
        assert_eq!(error.code, canwu_api::ErrorCode::InvalidDomainRecord);
    }
}
