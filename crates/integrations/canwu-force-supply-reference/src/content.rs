use crate::{
    ForceRequirementId, ForceSupplyCadenceV1, ForceSupplyProfileId, ForceSupplyProfileV1,
    ForceSupplyRequirementV1, RequisitionPolicyId, RequisitionPolicyV1, ShortageConsequenceRuleV1,
    SupplyResourceKind, invalid,
};
use canwu_api::{CanwuError, SimTime, canonical_hash};
use canwu_economy_reference_content::{
    AuthoredValueV1, BehavioralDefinitionV1, CompiledEconomyReferenceContentV1, CoverageCellV1,
    EconomyMechanism,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompiledForceSupplyConfigurationV1 {
    pub content_hash: String,
    pub profiles: BTreeMap<ForceSupplyProfileId, ForceSupplyProfileV1>,
    pub requisition_policies: BTreeMap<RequisitionPolicyId, RequisitionPolicyV1>,
}

pub fn compile_force_supply_configuration(
    content: &CompiledEconomyReferenceContentV1,
) -> Result<CompiledForceSupplyConfigurationV1, CanwuError> {
    content.validate()?;
    let mut profile_cells: BTreeMap<String, Vec<&CoverageCellV1>> = BTreeMap::new();
    let mut profiles = BTreeMap::new();
    let mut policies = BTreeMap::new();
    for cell in content.coverage.values() {
        match cell.key.mechanism {
            EconomyMechanism::ForceSupply => {
                profile_cells
                    .entry(cell.key.process_or_organization_class.to_string())
                    .or_default()
                    .push(cell);
            }
            EconomyMechanism::RequisitionExternality => {
                let policy = compile_policy(content, cell)?;
                if policies.insert(policy.id.clone(), policy).is_some() {
                    return Err(invalid("compiled requisition policy identity collided"));
                }
            }
            _ => {}
        }
    }
    for (organization_class, mut cells) in profile_cells {
        cells.sort_by(|left, right| left.key.cmp(&right.key));
        let profile = compile_profile(content, &organization_class, &cells)?;
        if profiles.insert(profile.id.clone(), profile).is_some() {
            return Err(invalid("compiled force profile identity collided"));
        }
    }
    Ok(CompiledForceSupplyConfigurationV1 {
        content_hash: content.content_hash.clone(),
        profiles,
        requisition_policies: policies,
    })
}

fn compile_profile(
    content: &CompiledEconomyReferenceContentV1,
    organization_class: &str,
    cells: &[&CoverageCellV1],
) -> Result<ForceSupplyProfileV1, CanwuError> {
    let mut requirements = Vec::with_capacity(cells.len());
    let mut requirement_coverage = BTreeMap::new();
    let mut requirement_resolution_digests = BTreeMap::new();
    let mut definition_ids = BTreeSet::new();
    let mut model_cards = BTreeSet::new();
    for cell in cells {
        let requirement = compile_requirement(content, cell)?;
        requirement_coverage.insert(requirement.id.clone(), cell.key.clone());
        requirement_resolution_digests
            .insert(requirement.id.clone(), cell.resolution_digest.clone());
        requirements.push(requirement);
        definition_ids.extend(cell.definition_ids.iter().cloned());
        model_cards.extend(cell.model_card_ids.iter().cloned());
    }
    requirements.sort_by(|left, right| left.id.cmp(&right.id));
    let first_requirement = requirements
        .first()
        .ok_or_else(|| invalid("compiled force profile has no requirements"))?;
    let coverage_key = requirement_coverage[&first_requirement.id].clone();
    let coverage_resolution_digest = requirement_resolution_digests[&first_requirement.id].clone();
    let mut profile = ForceSupplyProfileV1 {
        id: ForceSupplyProfileId::new(format!(
            "canwu.force-supply-reference:profile:{}",
            stable_suffix(organization_class)
        ))?,
        revision: 1,
        effective_from: SimTime::EPOCH,
        effective_until: None,
        organization_class: organization_class.to_owned(),
        requirements,
        requirement_coverage,
        requirement_resolution_digests,
        coverage_key,
        content_hash: content.content_hash.clone(),
        coverage_resolution_digest,
        definition_ids,
        model_cards,
        semantic_digest: String::new(),
    };
    profile.semantic_digest = canonical_hash("canwu.force-supply.profile.v1", &profile)?;
    Ok(profile)
}

fn compile_requirement(
    content: &CompiledEconomyReferenceContentV1,
    cell: &CoverageCellV1,
) -> Result<ForceSupplyRequirementV1, CanwuError> {
    let definition = exact_behavior_definition(content, cell)?;
    let values = exact_values(definition)?;
    require_exact_fields(
        &values,
        &[
            "quantity_per_due",
            "buffer_quantity",
            "cadence_minutes",
            "resource_kind_code",
            "shortage_tolerance_quantity",
            "readiness_delta_per_mille",
            "fatigue_delta_per_mille",
            "cohesion_delta_per_mille",
            "disease_delta_per_mille",
            "desertion_delta_per_mille",
            "nonlinear_or_threshold",
        ],
    )?;
    let rule = exact_rule(definition)?;
    let quantity_per_due = positive_u64(&values, "quantity_per_due")?;
    let buffer_quantity = nonnegative_u64(&values, "buffer_quantity")?;
    let cadence_minutes = positive_u64(&values, "cadence_minutes")?;
    let kind = supply_kind(value(&values, "resource_kind_code")?.value)?;
    let consequence = ShortageConsequenceRuleV1 {
        rule_revision: rule.rule_revision.clone(),
        tolerance_quantity: nonnegative_u64(&values, "shortage_tolerance_quantity")?,
        readiness_delta_per_mille: per_mille_delta(&values, "readiness_delta_per_mille")?,
        fatigue_delta_per_mille: per_mille_delta(&values, "fatigue_delta_per_mille")?,
        cohesion_delta_per_mille: per_mille_delta(&values, "cohesion_delta_per_mille")?,
        disease_delta_per_mille: per_mille_delta(&values, "disease_delta_per_mille")?,
        desertion_delta_per_mille: per_mille_delta(&values, "desertion_delta_per_mille")?,
        nonlinear_or_threshold: match value(&values, "nonlinear_or_threshold")?.value {
            0 => false,
            1 => true,
            _ => return Err(invalid("compiled force threshold flag is not boolean")),
        },
        model_card: rule.model_card_id.clone(),
    };
    Ok(ForceSupplyRequirementV1 {
        id: ForceRequirementId::new(format!(
            "canwu.force-supply-reference:requirement:{}",
            stable_suffix(definition.id.as_str())
        ))?,
        kind,
        resource_revision: cell.key.resource_revision.clone(),
        unit_revision: cell.key.unit_revision.clone(),
        quantity_per_due,
        buffer_quantity,
        cadence: ForceSupplyCadenceV1::FixedMinutes {
            interval_minutes: cadence_minutes,
        },
        consequence,
    })
}

fn compile_policy(
    content: &CompiledEconomyReferenceContentV1,
    cell: &CoverageCellV1,
) -> Result<RequisitionPolicyV1, CanwuError> {
    let definition = exact_behavior_definition(content, cell)?;
    let values = exact_values(definition)?;
    require_exact_fields(
        &values,
        &[
            "cooperation_cost_per_mille",
            "next_harvest_input_cost_per_mille",
        ],
    )?;
    let rule = exact_rule(definition)?;
    let applicability = definition
        .externality_applicability
        .ok_or_else(|| invalid("compiled requisition definition omits applicability"))?;
    let cooperation_cost = nonnegative_i16(&values, "cooperation_cost_per_mille")?;
    let harvest_cost = nonnegative_i16(&values, "next_harvest_input_cost_per_mille")?;
    let mut policy = RequisitionPolicyV1 {
        id: RequisitionPolicyId::new(format!(
            "canwu.force-supply-reference:policy:{}",
            stable_suffix(definition.id.as_str())
        ))?,
        revision: 1,
        applicability,
        cooperation_delta_per_mille: -cooperation_cost,
        harvest_input_delta_per_mille: -harvest_cost,
        rule_revision: rule.rule_revision.clone(),
        model_card: rule.model_card_id.clone(),
        coverage_key: cell.key.clone(),
        content_hash: content.content_hash.clone(),
        coverage_resolution_digest: cell.resolution_digest.clone(),
        definition_ids: cell.definition_ids.clone(),
        semantic_digest: String::new(),
    };
    policy.semantic_digest = canonical_hash("canwu.force-supply.requisition-policy.v1", &policy)?;
    Ok(policy)
}

fn exact_behavior_definition<'a>(
    content: &'a CompiledEconomyReferenceContentV1,
    cell: &CoverageCellV1,
) -> Result<&'a BehavioralDefinitionV1, CanwuError> {
    let mut definitions = cell
        .definition_ids
        .iter()
        .filter_map(|id| content.definitions.get(id))
        .filter(|definition| {
            !definition.numeric_fields.is_empty() || !definition.causal_rules.is_empty()
        });
    let definition = definitions
        .next()
        .ok_or_else(|| invalid("compiled force coverage has no executable definition"))?;
    if definitions.next().is_some() {
        return Err(invalid(
            "compiled force coverage has more than one executable definition",
        ));
    }
    Ok(definition)
}

fn exact_values(
    definition: &BehavioralDefinitionV1,
) -> Result<BTreeMap<&str, &AuthoredValueV1>, CanwuError> {
    let mut values = BTreeMap::new();
    for field in &definition.numeric_fields {
        if values.insert(field.field.as_str(), field).is_some() {
            return Err(invalid(
                "compiled force definition duplicates a numeric field",
            ));
        }
    }
    Ok(values)
}

fn exact_rule(
    definition: &BehavioralDefinitionV1,
) -> Result<&canwu_economy_reference_content::AuthoredRuleV1, CanwuError> {
    if definition.causal_rules.len() != 1 {
        return Err(invalid(
            "compiled force definition must bind exactly one executable rule revision",
        ));
    }
    Ok(&definition.causal_rules[0])
}

fn value<'a>(
    values: &'a BTreeMap<&str, &AuthoredValueV1>,
    name: &str,
) -> Result<&'a AuthoredValueV1, CanwuError> {
    values
        .get(name)
        .copied()
        .ok_or_else(|| invalid(format!("compiled force definition omits {name}")))
}

fn require_exact_fields(
    values: &BTreeMap<&str, &AuthoredValueV1>,
    expected: &[&str],
) -> Result<(), CanwuError> {
    if values.len() != expected.len() || expected.iter().any(|name| !values.contains_key(name)) {
        return Err(invalid(
            "compiled force definition numeric fields do not match the exact V1 schema",
        ));
    }
    Ok(())
}

fn positive_u64(values: &BTreeMap<&str, &AuthoredValueV1>, name: &str) -> Result<u64, CanwuError> {
    let result = nonnegative_u64(values, name)?;
    if result == 0 {
        return Err(invalid(format!(
            "compiled force field {name} must be positive"
        )));
    }
    Ok(result)
}

fn nonnegative_u64(
    values: &BTreeMap<&str, &AuthoredValueV1>,
    name: &str,
) -> Result<u64, CanwuError> {
    u64::try_from(value(values, name)?.value)
        .map_err(|_| invalid(format!("compiled force field {name} must be nonnegative")))
}

fn nonnegative_i16(
    values: &BTreeMap<&str, &AuthoredValueV1>,
    name: &str,
) -> Result<i16, CanwuError> {
    let result = i16::try_from(value(values, name)?.value)
        .map_err(|_| invalid(format!("compiled force field {name} exceeds i16")))?;
    if result < 0 {
        return Err(invalid(format!(
            "compiled force field {name} must be nonnegative"
        )));
    }
    Ok(result)
}

fn per_mille_delta(
    values: &BTreeMap<&str, &AuthoredValueV1>,
    name: &str,
) -> Result<i16, CanwuError> {
    let result = i16::try_from(value(values, name)?.value)
        .map_err(|_| invalid(format!("compiled force field {name} exceeds i16")))?;
    if !(-1_000..=1_000).contains(&result) {
        return Err(invalid(format!(
            "compiled force field {name} exceeds per-mille bounds"
        )));
    }
    Ok(result)
}

fn supply_kind(code: i64) -> Result<SupplyResourceKind, CanwuError> {
    match code {
        1 => Ok(SupplyResourceKind::Food),
        2 => Ok(SupplyResourceKind::Fodder),
        3 => Ok(SupplyResourceKind::PhysicalCurrency),
        4 => Ok(SupplyResourceKind::Ammunition),
        5 => Ok(SupplyResourceKind::Spares),
        6 => Ok(SupplyResourceKind::Fuel),
        7 => Ok(SupplyResourceKind::Other),
        _ => Err(invalid("compiled force resource kind code is unsupported")),
    }
}

fn stable_suffix(id: &str) -> String {
    id.rsplit(':').next().unwrap_or(id).replace(['/', ':'], "-")
}
