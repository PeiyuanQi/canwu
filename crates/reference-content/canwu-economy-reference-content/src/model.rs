use crate::{CONTENT_SCHEMA_VERSION, MAX_COMPILED_PACK_BYTES};
use canwu_api::{CanwuError, ErrorCode, SimTime};
use canwu_production::ProcessRevisionId;
use canwu_resource::{ResourceDefinitionRevisionId, ResourceQualityId, ResourceUnitRevisionId};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Display, Formatter};

fn validate_identifier(value: &str, label: &str) -> Result<(), CanwuError> {
    if value.is_empty()
        || value.len() > 192
        || !value.contains(':')
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':' | b'/')
        })
    {
        return Err(invalid(format!(
            "{label} must be a 1-192 byte namespaced identifier"
        )));
    }
    Ok(())
}

macro_rules! typed_id {
    ($name:ident, $label:literal) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, CanwuError> {
                let value = value.into();
                validate_identifier(&value, $label)?;
                Ok(Self(value))
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Display for $name {
            fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                Self::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
            }
        }
    };
}

typed_id!(ContentPackId, "content pack");
typed_id!(PeriodId, "period");
typed_id!(RegionId, "region");
typed_id!(OrganizationClassId, "process or organization class");
typed_id!(ModelCardId, "model card");
typed_id!(DefinitionId, "definition");
typed_id!(CoverageDeclarationId, "coverage declaration");
typed_id!(CoverageCellId, "coverage cell");
typed_id!(ProfileId, "profile");
typed_id!(CitationId, "citation");
typed_id!(RuleRevisionId, "rule revision");
typed_id!(ResourceCapabilityRevisionId, "resource capability revision");

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelClassification {
    Synthetic,
    Archetype,
    SourceCalibrated,
    Disputed,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageStatus {
    Supported,
    ArchetypeFallback,
    ExplicitUnknown,
    NotApplicable,
}

impl CoverageStatus {
    #[must_use]
    pub const fn authorizes_behavior(self) -> bool {
        matches!(self, Self::Supported | Self::ArchetypeFallback)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EconomyMechanism {
    ResourceCapability,
    SeasonalHarvest,
    WorkshopProduction,
    IndustrialProduction,
    ForceSupply,
    RequisitionExternality,
    LocalScarcity,
    PricePressure,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceCapabilityStage {
    Potential,
    ObservedSurveyed,
    Proven,
    TechnicallyExtractable,
    OperatingSite,
    RouteAccessible,
    DeliveredAccepted,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalityApplicability {
    Required,
    ExternalityNotApplicable,
    ExplicitUnknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CalibrationStatus {
    Calibrated,
    PartiallyCalibrated,
    Uncalibrated,
    Disputed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthoredValueOrigin {
    SourceDerived,
    GameplayCalibration,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthoredRuleNature {
    FactualConstraint,
    GameplayRule,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfidenceLevel {
    Low,
    Medium,
    High,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EffectivePeriodV1 {
    pub start: SimTime,
    pub end: SimTime,
}

impl EffectivePeriodV1 {
    #[must_use]
    pub const fn contains(&self, time: SimTime) -> bool {
        time.as_minutes() >= self.start.as_minutes() && time.as_minutes() < self.end.as_minutes()
    }

    pub(crate) fn validate(&self, label: &str) -> Result<(), CanwuError> {
        if self.start >= self.end {
            return Err(invalid(format!(
                "{label} must be a non-empty half-open interval"
            )));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CitationV1 {
    pub id: CitationId,
    pub citation: String,
    pub url: String,
    pub locator: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct UncertaintyIntervalV1 {
    pub low: i64,
    pub high: i64,
    pub unit: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ModelCardV1 {
    pub id: ModelCardId,
    pub classification: ModelClassification,
    pub citations: Vec<CitationV1>,
    pub claim_scope: String,
    pub forbidden_inferences: Vec<String>,
    pub competing_interpretations: Vec<String>,
    pub geographic_scope: BTreeSet<RegionId>,
    pub historical_years: Option<HistoricalYearWindowV1>,
    pub effective_period: EffectivePeriodV1,
    pub resource_revisions: BTreeSet<ResourceDefinitionRevisionId>,
    pub unit_revisions: BTreeSet<ResourceUnitRevisionId>,
    pub quality_revisions: BTreeSet<ResourceQualityId>,
    pub process_revisions: BTreeSet<ProcessRevisionId>,
    pub rule_revisions: BTreeSet<RuleRevisionId>,
    pub extraction_or_conversion_derivation: String,
    pub uncertainty: Option<UncertaintyIntervalV1>,
    pub confidence: ConfidenceLevel,
    pub calibration_status: CalibrationStatus,
    pub semantic_hash: String,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct HistoricalYearWindowV1 {
    pub start_year: i32,
    pub end_year_exclusive: i32,
}

impl HistoricalYearWindowV1 {
    #[must_use]
    pub const fn contains(&self, year: i32) -> bool {
        year >= self.start_year && year < self.end_year_exclusive
    }

    #[must_use]
    pub const fn contains_window(&self, other: &Self) -> bool {
        self.start_year <= other.start_year && self.end_year_exclusive >= other.end_year_exclusive
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct CoverageKeyV1 {
    pub period: PeriodId,
    pub historical_years: Option<HistoricalYearWindowV1>,
    pub region: RegionId,
    pub mechanism: EconomyMechanism,
    pub resource_revision: ResourceDefinitionRevisionId,
    pub quality_revision: ResourceQualityId,
    pub unit_revision: ResourceUnitRevisionId,
    pub process_or_organization_class: OrganizationClassId,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct CoverageSelectorV1 {
    pub periods: BTreeSet<PeriodId>,
    pub regions: BTreeSet<RegionId>,
    pub mechanisms: BTreeSet<EconomyMechanism>,
    pub resource_revisions: BTreeSet<ResourceDefinitionRevisionId>,
    pub quality_revisions: BTreeSet<ResourceQualityId>,
    pub unit_revisions: BTreeSet<ResourceUnitRevisionId>,
    pub process_or_organization_classes: BTreeSet<OrganizationClassId>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CoverageDeclarationV1 {
    pub id: CoverageDeclarationId,
    pub priority: u16,
    pub selector: CoverageSelectorV1,
    pub status: CoverageStatus,
    pub definition_ids: BTreeSet<DefinitionId>,
    pub model_card_ids: BTreeSet<ModelCardId>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CoverageCellV1 {
    pub id: CoverageCellId,
    pub key: CoverageKeyV1,
    pub status: CoverageStatus,
    pub priority: u16,
    pub definition_ids: BTreeSet<DefinitionId>,
    pub model_card_ids: BTreeSet<ModelCardId>,
    pub declaration_id: CoverageDeclarationId,
    pub resolution_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AuthoredValueV1 {
    pub field: String,
    pub value: i64,
    pub unit: String,
    pub origin: AuthoredValueOrigin,
    pub derivation: String,
    pub model_card_id: ModelCardId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AuthoredRuleV1 {
    pub rule_revision: RuleRevisionId,
    pub rule: String,
    pub nature: AuthoredRuleNature,
    pub model_card_id: ModelCardId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceCapabilityRevision {
    pub id: ResourceCapabilityRevisionId,
    pub definition_id: DefinitionId,
    pub coverage_key: CoverageKeyV1,
    pub stage: ResourceCapabilityStage,
    pub effective_period: EffectivePeriodV1,
    pub surveyed_or_operating_site: Option<String>,
    pub suitable_process_revisions: BTreeSet<ProcessRevisionId>,
    pub route_access_evidence_class: Option<String>,
    pub model_card_ids: BTreeSet<ModelCardId>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BehavioralDefinitionV1 {
    pub id: DefinitionId,
    pub mechanism: EconomyMechanism,
    pub coverage_key: CoverageKeyV1,
    pub numeric_fields: Vec<AuthoredValueV1>,
    pub causal_rules: Vec<AuthoredRuleV1>,
    pub resource_capability: Option<ResourceCapabilityRevision>,
    pub externality_applicability: Option<ExternalityApplicability>,
    pub model_card_ids: BTreeSet<ModelCardId>,
    pub semantic_hash: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProfileDisclosureV1 {
    pub field_or_rule: String,
    pub classification: ModelClassification,
    pub model_card_id: ModelCardId,
    pub disclosure: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReferenceProfileV1 {
    pub id: ProfileId,
    pub label: String,
    pub historically_named: bool,
    pub claims_calibrated: bool,
    pub definition_ids: BTreeSet<DefinitionId>,
    pub disclosures: Vec<ProfileDisclosureV1>,
    pub design_note: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ContentManifestV1 {
    pub schema_version: u32,
    pub pack_id: ContentPackId,
    pub pack_version: String,
    pub license: String,
    pub required_coverage_keys: BTreeSet<CoverageKeyV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EconomyReferenceContentPackV1 {
    pub manifest: ContentManifestV1,
    pub model_cards: Vec<ModelCardV1>,
    pub definitions: Vec<BehavioralDefinitionV1>,
    pub coverage: Vec<CoverageDeclarationV1>,
    pub profiles: Vec<ReferenceProfileV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompiledEconomyReferenceContentV1 {
    pub schema_version: u32,
    pub pack_id: ContentPackId,
    pub pack_version: String,
    pub content_hash: String,
    pub model_cards: BTreeMap<ModelCardId, ModelCardV1>,
    pub definitions: BTreeMap<DefinitionId, BehavioralDefinitionV1>,
    pub coverage: BTreeMap<CoverageCellId, CoverageCellV1>,
    pub profiles: BTreeMap<ProfileId, ReferenceProfileV1>,
}

impl CompiledEconomyReferenceContentV1 {
    pub fn validate(&self) -> Result<(), CanwuError> {
        if self.schema_version != CONTENT_SCHEMA_VERSION
            || self.content_hash.len() != 64
            || !self
                .content_hash
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(invalid(
                "compiled economy content has an invalid schema or digest",
            ));
        }
        let bytes = serde_json::to_vec(self).map_err(encode_error)?;
        if bytes.len() > MAX_COMPILED_PACK_BYTES {
            return Err(invalid("compiled economy content exceeds its byte budget"));
        }
        let mut detached = self.clone();
        let recorded = std::mem::take(&mut detached.content_hash);
        if recorded != canwu_api::canonical_hash("canwu.economy.reference-content.v1", &detached)? {
            return Err(invalid(
                "compiled economy content has a forged content hash",
            ));
        }
        crate::compiler::validate_compiled_semantics(self)?;
        for cell in self.coverage.values() {
            if !cell.status.authorizes_behavior()
                && (!cell.definition_ids.is_empty() || !cell.model_card_ids.is_empty())
            {
                return Err(invalid("non-behavioral compiled coverage carries behavior"));
            }
            if cell.status.authorizes_behavior()
                && (cell.definition_ids.is_empty() || cell.model_card_ids.is_empty())
            {
                return Err(invalid(
                    "behavioral compiled coverage lacks definitions or provenance",
                ));
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn coverage_cell(&self, key: &CoverageKeyV1) -> Option<&CoverageCellV1> {
        self.coverage.values().find(|cell| &cell.key == key)
    }

    pub fn behavior_for(
        &self,
        key: &CoverageKeyV1,
    ) -> Result<Vec<&BehavioralDefinitionV1>, CanwuError> {
        let cell = self
            .coverage
            .values()
            .find(|cell| &cell.key == key)
            .ok_or_else(|| invalid("required economy coverage key is absent"))?;
        if !cell.status.authorizes_behavior() {
            return Err(invalid(format!(
                "coverage status {:?} is non-behavioral",
                cell.status
            )));
        }
        cell.definition_ids
            .iter()
            .map(|id| {
                self.definitions
                    .get(id)
                    .ok_or_else(|| invalid("compiled coverage references a missing definition"))
            })
            .collect()
    }
}

pub(crate) fn invalid(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidDomainRecord, message)
}

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn encode_error(error: serde_json::Error) -> CanwuError {
    invalid(format!(
        "economy reference content could not be encoded: {error}"
    ))
}
