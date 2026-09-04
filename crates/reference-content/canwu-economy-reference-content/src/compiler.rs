use crate::{
    AuthoredRuleNature, AuthoredValueOrigin, BehavioralDefinitionV1,
    CompiledEconomyReferenceContentV1, CoverageCellId, CoverageCellV1, CoverageDeclarationV1,
    CoverageKeyV1, CoverageStatus, EconomyReferenceContentPackV1, MAX_CITATION_LOCATOR_BYTES,
    MAX_CITATIONS_PER_MODEL_CARD, MAX_COMPILED_PACK_BYTES, MAX_COVERAGE_CELLS, MAX_MODEL_CARDS,
    MAX_PROFILES, MAX_REFERENCES_PER_DEFINITION, ModelCardV1, ModelClassification,
    ProfileDisclosureV1, ReferenceProfileV1, encode_error, invalid,
};
use canwu_api::{CanwuError, canonical_hash};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Display;

/// Validate and compile one immutable economy reference-content pack.
///
/// Every manifest key must resolve to exactly one highest-priority declaration.
/// File order and geographic/time specificity have no implicit precedence.
pub fn compile_content_pack(
    pack: &EconomyReferenceContentPackV1,
) -> Result<CompiledEconomyReferenceContentV1, CanwuError> {
    validate_limits(pack)?;
    if pack.manifest.schema_version != crate::CONTENT_SCHEMA_VERSION
        || pack.manifest.license != "Apache-2.0"
    {
        return Err(invalid(
            "economy content requires the supported schema and Apache-2.0 license",
        ));
    }
    semver::Version::parse(&pack.manifest.pack_version)
        .map_err(|error| invalid(format!("economy content version is not SemVer: {error}")))?;
    if pack.manifest.required_coverage_keys.is_empty() {
        return Err(invalid(
            "economy content declares no required coverage keys",
        ));
    }
    if pack.manifest.required_coverage_keys.iter().any(|key| {
        key.historical_years
            .as_ref()
            .is_some_and(|window| window.start_year >= window.end_year_exclusive)
    }) {
        return Err(invalid(
            "economy content contains an empty historical coverage period",
        ));
    }

    let model_cards = unique_map(&pack.model_cards, |value| &value.id, "model card")?;
    let definitions = unique_map(&pack.definitions, |value| &value.id, "definition")?;
    let declarations = unique_map(&pack.coverage, |value| &value.id, "coverage declaration")?;
    let profiles = unique_map(&pack.profiles, |value| &value.id, "profile")?;

    for model_card in model_cards.values() {
        validate_model_card(model_card)?;
    }
    for definition in definitions.values() {
        validate_definition(definition, &model_cards)?;
    }
    validate_model_card_bindings(&model_cards, &definitions)?;
    for declaration in declarations.values() {
        validate_declaration(declaration, &definitions, &model_cards)?;
    }
    let coverage = materialize_coverage(
        &pack.manifest.required_coverage_keys,
        declarations.values(),
        &definitions,
        &model_cards,
    )?;
    for profile in profiles.values() {
        validate_profile(profile, &definitions, &model_cards)?;
    }

    let mut compiled = CompiledEconomyReferenceContentV1 {
        schema_version: pack.manifest.schema_version,
        pack_id: pack.manifest.pack_id.clone(),
        pack_version: pack.manifest.pack_version.clone(),
        content_hash: String::new(),
        model_cards,
        definitions,
        coverage,
        profiles,
    };
    compiled.content_hash = canonical_hash("canwu.economy.reference-content.v1", &compiled)?;
    compiled.validate()?;
    Ok(compiled)
}

pub(crate) fn validate_compiled_semantics(
    compiled: &CompiledEconomyReferenceContentV1,
) -> Result<(), CanwuError> {
    for card in compiled.model_cards.values() {
        validate_model_card(card)?;
    }
    for definition in compiled.definitions.values() {
        validate_definition(definition, &compiled.model_cards)?;
    }
    validate_model_card_bindings(&compiled.model_cards, &compiled.definitions)?;
    for profile in compiled.profiles.values() {
        validate_profile(profile, &compiled.definitions, &compiled.model_cards)?;
    }
    for (id, cell) in &compiled.coverage {
        if id != &cell.id {
            return Err(invalid(
                "compiled coverage map key differs from its cell ID",
            ));
        }
        require_refs(
            &cell.definition_ids,
            &compiled.definitions,
            "compiled coverage definition",
        )?;
        require_refs(
            &cell.model_card_ids,
            &compiled.model_cards,
            "compiled coverage model card",
        )?;
        if cell.definition_ids.iter().any(|definition_id| {
            let definition = &compiled.definitions[definition_id];
            definition.coverage_key != cell.key
                || !definition.model_card_ids.is_subset(&cell.model_card_ids)
        }) {
            return Err(invalid(
                "compiled coverage contains a definition outside its exact key or provenance",
            ));
        }
        let key_digest = canonical_hash("canwu.economy.coverage-key.v1", &cell.key)?;
        let expected_id = CoverageCellId::new(format!("canwu.economy:coverage-cell:{key_digest}"))?;
        let expected_resolution = canonical_hash(
            "canwu.economy.coverage-resolution.v1",
            &(
                &cell.key,
                cell.priority,
                &cell.declaration_id,
                cell.status,
                &cell.definition_ids,
            ),
        )?;
        if cell.id != expected_id || cell.resolution_digest != expected_resolution {
            return Err(invalid(
                "compiled coverage identity or resolution digest is forged",
            ));
        }
    }
    Ok(())
}

fn validate_limits(pack: &EconomyReferenceContentPackV1) -> Result<(), CanwuError> {
    let encoded = serde_json::to_vec(pack).map_err(encode_error)?;
    if encoded.len() > MAX_COMPILED_PACK_BYTES
        || pack.manifest.required_coverage_keys.len() > MAX_COVERAGE_CELLS
        || pack.coverage.len() > MAX_COVERAGE_CELLS
        || pack.model_cards.len() > MAX_MODEL_CARDS
        || pack.profiles.len() > MAX_PROFILES
        || pack.definitions.len() > MAX_PROFILES
    {
        return Err(invalid("economy content exceeds its bounded capacity"));
    }
    Ok(())
}

fn validate_model_card(card: &ModelCardV1) -> Result<(), CanwuError> {
    card.effective_period
        .validate("model-card effective period")?;
    if card.claim_scope.trim().is_empty()
        || card.forbidden_inferences.is_empty()
        || card.extraction_or_conversion_derivation.trim().is_empty()
        || card.semantic_hash.len() != 64
        || !card
            .semantic_hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || card.citations.len() > MAX_CITATIONS_PER_MODEL_CARD
        || card
            .citations
            .iter()
            .any(|citation| citation.locator.len() > MAX_CITATION_LOCATOR_BYTES)
        || card
            .forbidden_inferences
            .iter()
            .any(|value| value.trim().is_empty())
    {
        return Err(invalid(format!("model card {} is incomplete", card.id)));
    }
    if card
        .historical_years
        .as_ref()
        .is_some_and(|window| window.start_year >= window.end_year_exclusive)
    {
        return Err(invalid(format!(
            "model card {} has an empty historical-year window",
            card.id
        )));
    }
    let sourced = matches!(
        card.classification,
        ModelClassification::Archetype
            | ModelClassification::SourceCalibrated
            | ModelClassification::Disputed
    );
    if sourced
        && (card.citations.is_empty()
            || card.citations.iter().any(|citation| {
                citation.citation.trim().is_empty()
                    || citation.url.trim().is_empty()
                    || citation.locator.trim().is_empty()
            }))
    {
        return Err(invalid(format!(
            "sourced model card {} has no complete citation",
            card.id
        )));
    }
    if let Some(interval) = &card.uncertainty
        && (interval.low > interval.high || interval.unit.trim().is_empty())
    {
        return Err(invalid(format!(
            "model card {} has an invalid uncertainty interval",
            card.id
        )));
    }
    let mut detached = card.clone();
    let recorded = std::mem::take(&mut detached.semantic_hash);
    if recorded != canonical_hash("canwu.economy.model-card.v1", &detached)? {
        return Err(invalid(format!(
            "model card {} has a forged semantic hash",
            card.id
        )));
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn validate_definition(
    definition: &BehavioralDefinitionV1,
    cards: &BTreeMap<crate::ModelCardId, ModelCardV1>,
) -> Result<(), CanwuError> {
    if definition.coverage_key.mechanism != definition.mechanism
        || definition.model_card_ids.is_empty()
        || definition.model_card_ids.len() > MAX_REFERENCES_PER_DEFINITION
        || definition.numeric_fields.len() > MAX_REFERENCES_PER_DEFINITION
        || definition.causal_rules.len() > MAX_REFERENCES_PER_DEFINITION
        || definition.semantic_hash.len() != 64
        || !definition
            .semantic_hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(invalid(format!(
            "definition {} is incomplete",
            definition.id
        )));
    }
    require_refs(&definition.model_card_ids, cards, "definition model card")?;
    if let Some(window) = &definition.coverage_key.historical_years {
        for card_id in &definition.model_card_ids {
            let card_window = cards[card_id].historical_years.as_ref().ok_or_else(|| {
                invalid(format!(
                    "definition {} has historical coverage outside model card {}",
                    definition.id, card_id
                ))
            })?;
            if !card_window.contains_window(window) {
                return Err(invalid(format!(
                    "definition {} historical coverage exceeds model card {}",
                    definition.id, card_id
                )));
            }
        }
    }
    for field in &definition.numeric_fields {
        if field.field.trim().is_empty()
            || field.unit.trim().is_empty()
            || field.derivation.trim().is_empty()
            || !definition.model_card_ids.contains(&field.model_card_id)
        {
            return Err(invalid(format!(
                "numeric field in definition {} lacks an exact model card",
                definition.id
            )));
        }
        let card = &cards[&field.model_card_id];
        match field.origin {
            AuthoredValueOrigin::GameplayCalibration => {
                if card.classification != ModelClassification::Synthetic
                    || card.calibration_status != crate::CalibrationStatus::Uncalibrated
                {
                    return Err(invalid(format!(
                        "gameplay numeric field in definition {} is not bound to an uncalibrated synthetic card",
                        definition.id
                    )));
                }
            }
            AuthoredValueOrigin::SourceDerived => {
                if card.classification == ModelClassification::Synthetic
                    || card.calibration_status == crate::CalibrationStatus::Uncalibrated
                {
                    return Err(invalid(format!(
                        "source-derived numeric field in definition {} lacks a calibrated source card",
                        definition.id
                    )));
                }
            }
        }
    }
    for rule in &definition.causal_rules {
        if rule.rule.trim().is_empty() || !definition.model_card_ids.contains(&rule.model_card_id) {
            return Err(invalid(format!(
                "causal rule in definition {} lacks an exact model card",
                definition.id
            )));
        }
        let card = &cards[&rule.model_card_id];
        if !card.rule_revisions.contains(&rule.rule_revision) {
            return Err(invalid(format!(
                "causal rule {} in definition {} is not exactly bound by its model card",
                rule.rule_revision, definition.id
            )));
        }
        if rule.nature == AuthoredRuleNature::GameplayRule
            && card.classification != ModelClassification::Synthetic
        {
            return Err(invalid(format!(
                "gameplay rule {} in definition {} is not separated onto a synthetic card",
                rule.rule_revision, definition.id
            )));
        }
    }
    if let Some(capability) = &definition.resource_capability
        && (capability.definition_id != definition.id
            || capability.coverage_key != definition.coverage_key
            || capability.model_card_ids.is_empty()
            || !capability
                .model_card_ids
                .is_subset(&definition.model_card_ids))
    {
        return Err(invalid(format!(
            "resource capability in definition {} is not exactly bound",
            definition.id
        )));
    }
    if let Some(capability) = &definition.resource_capability {
        capability
            .effective_period
            .validate("resource-capability effective period")?;
        for card_id in &capability.model_card_ids {
            let card = &cards[card_id];
            if capability.effective_period.start < card.effective_period.start
                || capability.effective_period.end > card.effective_period.end
            {
                return Err(invalid(format!(
                    "resource capability in definition {} exceeds model card {} effective period",
                    definition.id, card_id
                )));
            }
        }
    }
    let mut detached = definition.clone();
    let recorded = std::mem::take(&mut detached.semantic_hash);
    if recorded != canonical_hash("canwu.economy.behavior-definition.v1", &detached)? {
        return Err(invalid(format!(
            "definition {} has a forged semantic hash",
            definition.id
        )));
    }
    Ok(())
}

fn validate_model_card_bindings(
    cards: &BTreeMap<crate::ModelCardId, ModelCardV1>,
    definitions: &BTreeMap<crate::DefinitionId, BehavioralDefinitionV1>,
) -> Result<(), CanwuError> {
    for card in cards.values() {
        let source_derived_values = definitions
            .values()
            .flat_map(|definition| definition.numeric_fields.iter())
            .filter(|field| {
                field.model_card_id == card.id && field.origin == AuthoredValueOrigin::SourceDerived
            })
            .count();
        if card.classification == ModelClassification::Archetype
            && card.calibration_status != crate::CalibrationStatus::Uncalibrated
            && source_derived_values == 0
        {
            return Err(invalid(format!(
                "archetype model card {} claims calibration without a derived numeric field",
                card.id
            )));
        }
        let referenced_rules: BTreeSet<_> = definitions
            .values()
            .flat_map(|definition| definition.causal_rules.iter())
            .filter(|rule| rule.model_card_id == card.id)
            .map(|rule| rule.rule_revision.clone())
            .collect();
        if referenced_rules != card.rule_revisions {
            return Err(invalid(format!(
                "model card {} rule revisions are not exactly bound to definitions",
                card.id
            )));
        }
    }
    Ok(())
}

fn validate_declaration(
    declaration: &CoverageDeclarationV1,
    definitions: &BTreeMap<crate::DefinitionId, BehavioralDefinitionV1>,
    cards: &BTreeMap<crate::ModelCardId, ModelCardV1>,
) -> Result<(), CanwuError> {
    if declaration.definition_ids.len() > MAX_REFERENCES_PER_DEFINITION
        || declaration.model_card_ids.len() > MAX_REFERENCES_PER_DEFINITION
    {
        return Err(invalid(format!(
            "coverage declaration {} exceeds its reference bound",
            declaration.id
        )));
    }
    require_refs(
        &declaration.definition_ids,
        definitions,
        "coverage definition",
    )?;
    require_refs(&declaration.model_card_ids, cards, "coverage model card")?;
    if declaration.status.authorizes_behavior()
        && (declaration.definition_ids.is_empty() || declaration.model_card_ids.is_empty())
    {
        return Err(invalid(format!(
            "coverage declaration {} promises behavior without provenance",
            declaration.id
        )));
    }
    if !declaration.status.authorizes_behavior()
        && (!declaration.definition_ids.is_empty() || !declaration.model_card_ids.is_empty())
    {
        return Err(invalid(format!(
            "coverage declaration {} carries behavior for a non-behavioral status",
            declaration.id
        )));
    }
    Ok(())
}

fn materialize_coverage<'a>(
    required: &BTreeSet<CoverageKeyV1>,
    declarations: impl Iterator<Item = &'a CoverageDeclarationV1>,
    definitions: &BTreeMap<crate::DefinitionId, BehavioralDefinitionV1>,
    cards: &BTreeMap<crate::ModelCardId, ModelCardV1>,
) -> Result<BTreeMap<CoverageCellId, CoverageCellV1>, CanwuError> {
    let declarations: Vec<_> = declarations.collect();
    let mut cells = BTreeMap::new();
    for key in required {
        let mut matches: Vec<_> = declarations
            .iter()
            .filter(|declaration| selector_matches(&declaration.selector, key))
            .copied()
            .collect();
        matches.sort_by_key(|declaration| declaration.priority);
        let Some(selected) = matches.last() else {
            return Err(invalid(format!("coverage is missing for {key:?}")));
        };
        if matches.len() > 1 && matches[matches.len() - 2].priority == selected.priority {
            return Err(invalid(format!(
                "coverage has an equal-priority conflict for {key:?}"
            )));
        }
        let cell_definition_ids: BTreeSet<_> = selected
            .definition_ids
            .iter()
            .filter(|id| definitions[*id].coverage_key == *key)
            .cloned()
            .collect();
        for id in &cell_definition_ids {
            let definition = &definitions[id];
            if !definition
                .model_card_ids
                .is_subset(&selected.model_card_ids)
            {
                return Err(invalid(format!(
                    "coverage declaration {} omits definition model-card provenance",
                    selected.id
                )));
            }
        }
        let status = selected.status;
        if status.authorizes_behavior() && cell_definition_ids.is_empty() {
            return Err(invalid(format!(
                "coverage declaration {} authorizes no definition for the exact key",
                selected.id
            )));
        }
        if status == CoverageStatus::Supported
            && selected
                .model_card_ids
                .iter()
                .any(|id| cards[id].classification == ModelClassification::Unknown)
        {
            return Err(invalid(
                "supported coverage cannot be proven by an unknown model card",
            ));
        }
        let key_digest = canonical_hash("canwu.economy.coverage-key.v1", key)?;
        let id = CoverageCellId::new(format!("canwu.economy:coverage-cell:{key_digest}"))?;
        let resolution_digest = canonical_hash(
            "canwu.economy.coverage-resolution.v1",
            &(
                key,
                selected.priority,
                &selected.id,
                status,
                &cell_definition_ids,
            ),
        )?;
        cells.insert(
            id.clone(),
            CoverageCellV1 {
                id,
                key: key.clone(),
                status,
                priority: selected.priority,
                definition_ids: cell_definition_ids,
                model_card_ids: selected.model_card_ids.clone(),
                declaration_id: selected.id.clone(),
                resolution_digest,
            },
        );
    }
    Ok(cells)
}

fn validate_profile(
    profile: &ReferenceProfileV1,
    definitions: &BTreeMap<crate::DefinitionId, BehavioralDefinitionV1>,
    cards: &BTreeMap<crate::ModelCardId, ModelCardV1>,
) -> Result<(), CanwuError> {
    if profile.label.trim().is_empty()
        || profile.design_note.trim().is_empty()
        || profile.definition_ids.is_empty()
    {
        return Err(invalid(format!("profile {} is incomplete", profile.id)));
    }
    require_refs(&profile.definition_ids, definitions, "profile definition")?;
    let disclosed: BTreeSet<_> = profile
        .disclosures
        .iter()
        .map(|value: &ProfileDisclosureV1| value.model_card_id.clone())
        .collect();
    let required_disclosures: BTreeSet<_> = profile
        .definition_ids
        .iter()
        .flat_map(|id| definitions[id].model_card_ids.iter())
        .filter(|id| {
            !matches!(
                cards[*id].classification,
                ModelClassification::SourceCalibrated
            )
        })
        .cloned()
        .collect();
    if profile.historically_named && !required_disclosures.is_subset(&disclosed) {
        return Err(invalid(format!(
            "historically named profile {} omits a synthetic/archetype/disputed/unknown disclosure",
            profile.id
        )));
    }
    for disclosure in &profile.disclosures {
        let card = cards
            .get(&disclosure.model_card_id)
            .ok_or_else(|| invalid("profile disclosure references an unknown model card"))?;
        if disclosure.field_or_rule.trim().is_empty()
            || disclosure.disclosure.trim().is_empty()
            || disclosure.classification != card.classification
        {
            return Err(invalid(format!(
                "profile {} has an invalid disclosure",
                profile.id
            )));
        }
    }
    if profile.claims_calibrated
        && profile.definition_ids.iter().any(|definition| {
            definitions[definition]
                .model_card_ids
                .iter()
                .any(|card| cards[card].classification != ModelClassification::SourceCalibrated)
        })
    {
        return Err(invalid(format!(
            "profile {} claims calibration while carrying non-calibrated behavior",
            profile.id
        )));
    }
    Ok(())
}

fn selector_matches(selector: &crate::CoverageSelectorV1, key: &CoverageKeyV1) -> bool {
    selected(&selector.periods, &key.period)
        && selected(&selector.regions, &key.region)
        && selected(&selector.mechanisms, &key.mechanism)
        && selected(&selector.resource_revisions, &key.resource_revision)
        && selected(&selector.quality_revisions, &key.quality_revision)
        && selected(&selector.unit_revisions, &key.unit_revision)
        && selected(
            &selector.process_or_organization_classes,
            &key.process_or_organization_class,
        )
}

fn selected<T: Ord>(selection: &BTreeSet<T>, value: &T) -> bool {
    selection.is_empty() || selection.contains(value)
}

fn unique_map<T, K, F>(values: &[T], key: F, label: &str) -> Result<BTreeMap<K, T>, CanwuError>
where
    T: Clone,
    K: Clone + Ord + Display,
    F: Fn(&T) -> &K,
{
    let mut result = BTreeMap::new();
    for value in values {
        let id = key(value).clone();
        if result.insert(id.clone(), value.clone()).is_some() {
            return Err(invalid(format!("duplicate {label} {id}")));
        }
    }
    Ok(result)
}

fn require_refs<K: Ord + Display, V>(
    references: &BTreeSet<K>,
    values: &BTreeMap<K, V>,
    label: &str,
) -> Result<(), CanwuError> {
    if let Some(missing) = references.iter().find(|id| !values.contains_key(*id)) {
        return Err(invalid(format!("{label} {missing} is unavailable")));
    }
    Ok(())
}
