use crate::{
    EconomyObservationWitnessV1, MAX_ADAPTER_CALLS, MAX_OBSERVATION_FACTS, MAX_PRICE_FACTORS,
    MAX_TYPED_SOURCE_ADAPTERS, PriceEvidenceApplicabilityV1, REFERENCE_VERSION,
    economy_observation_witness,
};
use canwu_api::{Canwu, CanwuError, DomainRecordVersionRef, KnowledgeHolderRef, SimTime};
use canwu_force_supply_reference::{
    FORCE_SUPPLY_SEMANTIC_HASH, ReferenceForceId, force_supply_observation_witness,
    force_supply_report,
};
use canwu_production::{
    PRODUCTION_SEMANTIC_HASH, ProductionSiteId, production_observation_witness, production_report,
};
use canwu_resource::{
    RESOURCE_SEMANTIC_HASH, ResourceAccountId, ResourceDefinitionRevisionId,
    ResourceObservationAdapterRevisionId, ResourceQualityId, ResourceReportGrantId,
    ResourceScopeId, ResourceUnitRevisionId, resource_observation_witness,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Display, Formatter};

const RESOURCE_ADAPTER_REVISION: &str = "canwu.resource.observation-adapter.v1";
const PRODUCTION_ADAPTER_REVISION: &str = "canwu.production.observation-adapter.v1";
const FORCE_ADAPTER_REVISION: &str = "canwu.force-supply.observation-adapter.v1";
const ECONOMY_ADAPTER_REVISION: &str = "canwu.economy.observation-adapter.v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PriceEvidenceKind {
    ExecutedExchange,
    Quote,
    AdministeredPrice,
    ContractPrice,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PriceProjectionStatus {
    Observed,
    InferredPressure,
    ExplicitUnknown,
    NotApplicable,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceProjectionSourceV1 {
    pub grant: ResourceReportGrantId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionProjectionSourceV1 {
    pub site: ProductionSiteId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ForceProjectionSourceV1 {
    pub force: ReferenceForceId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectionScopeBindingV1 {
    pub holder: KnowledgeHolderRef,
    pub scope: ResourceScopeId,
    pub resource: ResourceProjectionSourceV1,
    pub production: Option<ProductionProjectionSourceV1>,
    pub force: Option<ForceProjectionSourceV1>,
    pub semantic_digest: String,
}

impl ProjectionScopeBindingV1 {
    pub fn seal(mut self) -> Result<Self, ProjectionError> {
        self.semantic_digest.clear();
        self.semantic_digest = digest("canwu.economy.projection-scope-binding.v1", &self)?;
        self.validate()?;
        Ok(self)
    }

    pub fn validate(&self) -> Result<(), ProjectionError> {
        let mut detached = self.clone();
        detached.semantic_digest.clear();
        if self.semantic_digest != digest("canwu.economy.projection-scope-binding.v1", &detached)? {
            return Err(ProjectionError::InvalidBinding(
                "projection source binding is empty, non-canonical, or forged".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectionProviderRegistryV1 {
    bindings: BTreeMap<(KnowledgeHolderRef, ResourceScopeId), ProjectionScopeBindingV1>,
}

impl ProjectionProviderRegistryV1 {
    pub fn new(
        bindings: impl IntoIterator<Item = ProjectionScopeBindingV1>,
    ) -> Result<Self, ProjectionError> {
        let mut registry = Self::default();
        for binding in bindings {
            binding.validate()?;
            let key = (binding.holder.clone(), binding.scope.clone());
            if registry.bindings.insert(key, binding).is_some() {
                return Err(ProjectionError::InvalidBinding(
                    "projection registry contains a duplicate holder/scope binding".to_owned(),
                ));
            }
        }
        Ok(registry)
    }

    #[must_use]
    pub fn binding_count(&self) -> usize {
        self.bindings.len()
    }

    #[must_use]
    pub fn project(
        &self,
        canwu: &Canwu,
        holder: &KnowledgeHolderRef,
        scope: &ResourceScopeId,
    ) -> ProjectionQueryResultV1 {
        match self.project_inner(canwu, holder, scope) {
            Ok(value) => ProjectionQueryResultV1::Available(value),
            Err(error) => ProjectionQueryResultV1::Unavailable(
                ProjectionUnavailableV1::from_error(holder, scope, canwu.time(), &error),
            ),
        }
    }

    #[allow(clippy::too_many_lines)]
    fn project_inner(
        &self,
        canwu: &Canwu,
        holder: &KnowledgeHolderRef,
        scope: &ResourceScopeId,
    ) -> Result<EconomyProjectionV1, ProjectionError> {
        let binding = self
            .bindings
            .get(&(holder.clone(), scope.clone()))
            .ok_or(ProjectionError::ScopeUnconfigured)?;
        binding.validate()?;
        let source_count = 2_usize
            .checked_add(usize::from(binding.production.is_some()))
            .and_then(|value| value.checked_add(usize::from(binding.force.is_some())))
            .ok_or(ProjectionError::ArithmeticOverflow)?;
        if source_count > MAX_TYPED_SOURCE_ADAPTERS || source_count > MAX_ADAPTER_CALLS {
            return Err(ProjectionError::BudgetExceeded("typed source adapters"));
        }

        let adapter_revision = ResourceObservationAdapterRevisionId::new(RESOURCE_ADAPTER_REVISION)
            .map_err(|error| ProjectionError::InvalidBinding(error.to_string()))?;
        let resource_witness = resource_observation_witness(
            canwu,
            holder,
            &binding.resource.grant,
            adapter_revision,
            canwu.time(),
        )
        .map_err(|error| provider_error("canwu-resource", error))?;
        if &resource_witness.holder != holder || &resource_witness.scope != scope {
            return Err(ProjectionError::HolderMismatch);
        }
        let economy_witness = economy_observation_witness(canwu, holder, scope)
            .map_err(|error| provider_error(crate::PLUGIN_NAME, error))?;
        validate_economy_witness(&economy_witness)?;
        if economy_witness.holder() != holder || economy_witness.scope() != scope {
            return Err(ProjectionError::HolderMismatch);
        }

        let access_by_source = latest_route_access(&economy_witness);
        let mut facts = Vec::new();
        for stock in &resource_witness.stock {
            let access = if &stock.scope == scope {
                StockAccessV1::Local
            } else {
                access_by_source
                    .get(&stock.scope)
                    .cloned()
                    .unwrap_or(StockAccessV1::RemoteUnobserved)
            };
            let (available_minimum, available_maximum) = unprotected_available(stock);
            facts.push(ObservationFactV1::Stock {
                account: stock.account.clone(),
                source_scope: stock.scope.clone(),
                access,
                available_minimum,
                available_maximum,
                protected: stock.protected,
            });
        }
        facts.extend(
            resource_witness
                .demands
                .iter()
                .map(|demand| ObservationFactV1::Demand {
                    requested: demand.requested,
                    fulfilled: demand.fulfilled,
                    remainder: demand.remainder,
                }),
        );
        facts.push(ObservationFactV1::Policy {
            rationed: economy_witness.head.rationed,
            requisitioned: economy_witness.head.requisitioned,
            reserve_release_allowed: economy_witness.head.reserve_release_allowed,
        });
        for price in &economy_witness.prices {
            facts.push(ObservationFactV1::Price {
                kind: price.kind,
                resource_revision: price.resource_revision.clone(),
                quality: price.quality.clone(),
                unit_revision: price.unit_revision.clone(),
                observed_scaled: price.observed_scaled,
                baseline_scaled: price.baseline_scaled,
                scale: price.scale,
                observed_at: price.observed_at,
                effective_from: price.effective_from,
                effective_until: price.effective_until,
                confidence_per_mille: price.confidence_per_mille,
                interpretation_rule_revision: price
                    .interpretation_rule_revision
                    .as_str()
                    .to_owned(),
                source_versions: price.source_versions.clone(),
            });
        }
        let mut witnesses = vec![TypedObservationWitnessV1::new(
            canwu_resource::PLUGIN_NAME,
            env!("CARGO_PKG_VERSION"),
            RESOURCE_SEMANTIC_HASH,
            resource_witness.adapter_provider_state_revision.get(),
            &resource_witness.digest,
            holder,
            scope,
            resource_witness.observed_at,
            resource_witness.materialized_at,
            resource_witness.confidence_per_mille,
            resource_witness.adapter_revision.as_str(),
            facts,
            resource_witness.source_versions,
        )?];
        witnesses.push(TypedObservationWitnessV1::from_economy(&economy_witness)?);

        if let Some(source) = &binding.production {
            let report = production_report(canwu, holder, &source.site)
                .map_err(|error| provider_error("canwu-production", error))?;
            let provider_witness = production_observation_witness(canwu, holder, &source.site)
                .map_err(|error| provider_error("canwu-production", error))?;
            if provider_witness.provider_plugin != canwu_production::PLUGIN_NAME
                || provider_witness.provider_semantic_hash != PRODUCTION_SEMANTIC_HASH
                || provider_witness.provider_state_revision != report.provider_state_revision
                || provider_witness.holder != *holder
                || provider_witness.scope != source.site
                || provider_witness.observed_at != report.observed_at
                || provider_witness.materialized_at != report.materialized_at
                || provider_witness.report_digest != report.canonical_digest
                || provider_witness.adapter_revision != PRODUCTION_ADAPTER_REVISION
            {
                return Err(ProjectionError::InvalidProviderWitness(
                    "production provider witness differs from its holder report and registered adapter"
                        .to_owned(),
                ));
            }
            let provider_state_revision = report.provider_state_revision;
            let observed_at = report.observed_at;
            let materialized_at = report.materialized_at;
            let facts =
                report
                    .facts
                    .into_iter()
                    .map(|fact| ObservationFactV1::Production {
                        state: fact.state,
                        expected_output_minimum: fact.quantity_low,
                        expected_output_maximum: fact.quantity_high,
                        blocked: false,
                    })
                    .chain(report.blockers.into_iter().map(|blocker| {
                        ObservationFactV1::Production {
                            state: format!("{blocker:?}"),
                            expected_output_minimum: 0,
                            expected_output_maximum: 0,
                            blocked: true,
                        }
                    }))
                    .collect();
            witnesses.push(TypedObservationWitnessV1::new(
                &provider_witness.provider_plugin,
                &provider_witness.provider_version,
                &provider_witness.provider_semantic_hash,
                provider_state_revision,
                &provider_witness.canonical_digest,
                holder,
                scope,
                observed_at,
                materialized_at,
                1_000,
                PRODUCTION_ADAPTER_REVISION,
                facts,
                provider_witness.source_versions,
            )?);
        }
        if let Some(source) = &binding.force {
            let report = force_supply_report(canwu, holder, &source.force)
                .map_err(|error| provider_error("canwu-force-supply-reference", error))?;
            let provider_witness =
                force_supply_observation_witness(canwu, holder, &source.force)
                    .map_err(|error| provider_error("canwu-force-supply-reference", error))?;
            if provider_witness.provider_plugin != canwu_force_supply_reference::PLUGIN_NAME
                || provider_witness.provider_semantic_hash != FORCE_SUPPLY_SEMANTIC_HASH
                || provider_witness.provider_state_revision != report.provider_state_revision
                || provider_witness.holder != *holder
                || provider_witness.force != source.force
                || provider_witness.observed_at != report.observed_at
                || provider_witness.materialized_at != report.materialized_at
                || provider_witness.confidence_per_mille != report.confidence_per_mille
                || provider_witness.report_digest != report.canonical_digest
                || provider_witness.adapter_revision != FORCE_ADAPTER_REVISION
            {
                return Err(ProjectionError::InvalidProviderWitness(
                    "force-supply provider witness differs from its holder report and registered adapter"
                        .to_owned(),
                ));
            }
            let provider_state_revision = report.provider_state_revision;
            let observed_at = report.observed_at;
            let materialized_at = report.materialized_at;
            let facts = report
                .observations
                .into_iter()
                .map(|observation| ObservationFactV1::ForceShortage {
                    demand_forecast: observation.demand_forecast,
                    known_stock_minimum: observation.known_stock_low,
                    known_stock_maximum: observation.known_stock_high,
                })
                .collect();
            witnesses.push(TypedObservationWitnessV1::new(
                &provider_witness.provider_plugin,
                &provider_witness.provider_version,
                &provider_witness.provider_semantic_hash,
                provider_state_revision,
                &provider_witness.canonical_digest,
                holder,
                scope,
                observed_at,
                materialized_at,
                provider_witness.confidence_per_mille,
                FORCE_ADAPTER_REVISION,
                facts,
                provider_witness.source_versions,
            )?);
        }
        project_registered_witnesses(
            holder,
            scope,
            canwu.time(),
            &witnesses,
            economy_witness.price_applicability,
            &economy_witness.profile_revision,
            &economy_witness.compiled_content_hash,
        )
    }
}

fn unprotected_available(stock: &canwu_resource::ResourceStockObservationV1) -> (u64, u64) {
    (
        stock
            .known_minimum
            .saturating_sub(stock.reserved)
            .saturating_sub(stock.protected),
        stock
            .known_maximum
            .saturating_sub(stock.reserved)
            .saturating_sub(stock.protected),
    )
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TypedObservationWitnessV1 {
    provider_plugin: String,
    provider_version: String,
    provider_semantic_hash: String,
    provider_state_version: u64,
    provider_head_digest: String,
    holder: KnowledgeHolderRef,
    scope: ResourceScopeId,
    observed_at: SimTime,
    materialized_at: SimTime,
    confidence_per_mille: u16,
    facts: Vec<ObservationFactV1>,
    source_versions: Vec<DomainRecordVersionRef>,
    adapter_revision: String,
    digest: String,
}

impl TypedObservationWitnessV1 {
    #[allow(clippy::too_many_arguments)]
    fn new(
        provider_plugin: &str,
        provider_version: &str,
        provider_semantic_hash: &str,
        provider_state_version: u64,
        provider_head_digest: &str,
        holder: &KnowledgeHolderRef,
        scope: &ResourceScopeId,
        observed_at: SimTime,
        materialized_at: SimTime,
        confidence_per_mille: u16,
        adapter_revision: &str,
        facts: Vec<ObservationFactV1>,
        mut source_versions: Vec<DomainRecordVersionRef>,
    ) -> Result<Self, ProjectionError> {
        source_versions.sort();
        source_versions.dedup();
        let mut witness = Self {
            provider_plugin: provider_plugin.to_owned(),
            provider_version: provider_version.to_owned(),
            provider_semantic_hash: provider_semantic_hash.to_owned(),
            provider_state_version,
            provider_head_digest: provider_head_digest.to_owned(),
            holder: holder.clone(),
            scope: scope.clone(),
            observed_at,
            materialized_at,
            confidence_per_mille,
            facts,
            source_versions,
            adapter_revision: adapter_revision.to_owned(),
            digest: String::new(),
        };
        witness.digest = digest("canwu.economy.typed-observation-witness.v1", &witness)?;
        validate_witness(&witness)?;
        Ok(witness)
    }

    fn from_economy(witness: &EconomyObservationWitnessV1) -> Result<Self, ProjectionError> {
        let mut facts = Vec::new();
        for route in &witness.routes {
            facts.push(ObservationFactV1::RouteAccess {
                source_scope: route.source_scope.clone(),
                reachable: route.reachable,
                delay_minutes: route.delay_minutes,
                confidence_per_mille: route.confidence_per_mille,
            });
        }
        Self::new(
            &witness.provider_plugin,
            &witness.provider_version,
            &witness.provider_semantic_hash,
            witness.provider_state_revision,
            &witness.canonical_digest,
            &witness.holder,
            &witness.scope,
            witness.observed_at,
            witness.materialized_at,
            witness.confidence_per_mille,
            ECONOMY_ADAPTER_REVISION,
            facts,
            witness.head.source_versions.clone(),
        )
    }

    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum StockAccessV1 {
    Local,
    RemoteReachable {
        delay_minutes: u64,
        confidence_per_mille: u16,
    },
    RemoteUnreachable {
        confidence_per_mille: u16,
    },
    RemoteUnobserved,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "fact", rename_all = "snake_case")]
enum ObservationFactV1 {
    Stock {
        account: ResourceAccountId,
        source_scope: ResourceScopeId,
        access: StockAccessV1,
        available_minimum: u64,
        available_maximum: u64,
        protected: u64,
    },
    Demand {
        requested: u64,
        fulfilled: u64,
        remainder: u64,
    },
    Buffer {
        target: u64,
        available: u64,
    },
    RouteAccess {
        source_scope: ResourceScopeId,
        reachable: bool,
        delay_minutes: u64,
        confidence_per_mille: u16,
    },
    Security {
        disruption_per_mille: u16,
    },
    Policy {
        rationed: bool,
        requisitioned: bool,
        reserve_release_allowed: bool,
    },
    Production {
        state: String,
        expected_output_minimum: u64,
        expected_output_maximum: u64,
        blocked: bool,
    },
    ForceShortage {
        demand_forecast: u64,
        known_stock_minimum: u64,
        known_stock_maximum: u64,
    },
    Price {
        kind: PriceEvidenceKind,
        resource_revision: ResourceDefinitionRevisionId,
        quality: ResourceQualityId,
        unit_revision: ResourceUnitRevisionId,
        observed_scaled: i64,
        baseline_scaled: i64,
        scale: u32,
        observed_at: SimTime,
        effective_from: SimTime,
        effective_until: SimTime,
        confidence_per_mille: u16,
        interpretation_rule_revision: String,
        source_versions: Vec<DomainRecordVersionRef>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LocalScarcityProjection {
    pub holder: KnowledgeHolderRef,
    pub scope: ResourceScopeId,
    pub observed_at: SimTime,
    pub materialized_at: SimTime,
    pub known_available_minimum: u64,
    pub known_available_maximum: u64,
    pub excluded_unreachable_minimum: u64,
    pub excluded_unreachable_maximum: u64,
    pub known_demand_remainder: u64,
    pub protected_or_buffered: u64,
    pub route_penalty_per_mille: u16,
    pub security_penalty_per_mille: u16,
    pub scarcity_per_mille: u16,
    pub stale: bool,
    pub causes: Vec<String>,
    pub witness_digests: Vec<String>,
    pub input_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PricePressureFactor {
    pub evidence_kind: PriceEvidenceKind,
    pub scope: ResourceScopeId,
    pub resource_revision: ResourceDefinitionRevisionId,
    pub quality: ResourceQualityId,
    pub unit_revision: ResourceUnitRevisionId,
    pub observed_scaled: i64,
    pub baseline_scaled: i64,
    pub scale: u32,
    pub change_per_mille: i32,
    pub observed_at: SimTime,
    pub materialized_at: SimTime,
    pub effective_from: SimTime,
    pub effective_until: SimTime,
    pub confidence_per_mille: u16,
    pub interpretation_rule_revision: String,
    pub source_versions: Vec<DomainRecordVersionRef>,
    pub witness_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PricePressureProjection {
    pub holder: KnowledgeHolderRef,
    pub scope: ResourceScopeId,
    pub observed_at: SimTime,
    pub materialized_at: SimTime,
    pub status: PriceProjectionStatus,
    pub pressure_per_mille: Option<i32>,
    pub factors: Vec<PricePressureFactor>,
    pub scarcity_input_digest: String,
    pub input_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EconomyProjectionV1 {
    pub scarcity: LocalScarcityProjection,
    pub price_pressure: PricePressureProjection,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[allow(clippy::large_enum_variant)]
#[serde(tag = "status", content = "value", rename_all = "snake_case")]
pub enum ProjectionQueryResultV1 {
    Available(EconomyProjectionV1),
    Unavailable(ProjectionUnavailableV1),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectionUnavailableV1 {
    pub holder: KnowledgeHolderRef,
    pub scope: ResourceScopeId,
    pub materialized_at: SimTime,
    pub blocker_code: String,
    pub provider: Option<String>,
    pub detail: String,
    pub digest: String,
}

impl ProjectionUnavailableV1 {
    fn from_error(
        holder: &KnowledgeHolderRef,
        scope: &ResourceScopeId,
        materialized_at: SimTime,
        error: &ProjectionError,
    ) -> Self {
        let (blocker_code, provider) = match error {
            ProjectionError::BudgetExceeded(_) => ("budget_exceeded", None),
            ProjectionError::HolderMismatch => ("holder_mismatch", None),
            ProjectionError::ScopeUnconfigured => ("scope_unconfigured", None),
            ProjectionError::ProviderUnavailable { provider, .. } => {
                ("provider_unavailable", Some(provider.clone()))
            }
            ProjectionError::InvalidProviderWitness(_) => ("invalid_provider_witness", None),
            ProjectionError::InvalidBinding(_) => ("invalid_binding", None),
            ProjectionError::ArithmeticOverflow => ("arithmetic_overflow", None),
        };
        let mut value = Self {
            holder: holder.clone(),
            scope: scope.clone(),
            materialized_at,
            blocker_code: blocker_code.to_owned(),
            provider,
            detail: error.to_string(),
            digest: String::new(),
        };
        value.digest = digest("canwu.economy.projection-unavailable.v1", &value)
            .unwrap_or_else(|_| "projection-unavailable-digest-error".to_owned());
        value
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProjectionError {
    BudgetExceeded(&'static str),
    HolderMismatch,
    ScopeUnconfigured,
    ProviderUnavailable { provider: String, reason: String },
    InvalidProviderWitness(String),
    InvalidBinding(String),
    ArithmeticOverflow,
}

impl Display for ProjectionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BudgetExceeded(name) => write!(formatter, "projection budget exceeded: {name}"),
            Self::HolderMismatch => formatter.write_str("typed witness belongs to another holder"),
            Self::ScopeUnconfigured => {
                formatter.write_str("projection scope has no compiled provider binding")
            }
            Self::ProviderUnavailable { provider, reason } => {
                write!(
                    formatter,
                    "projection provider {provider} is unavailable: {reason}"
                )
            }
            Self::InvalidProviderWitness(message) | Self::InvalidBinding(message) => {
                formatter.write_str(message)
            }
            Self::ArithmeticOverflow => formatter.write_str("projection arithmetic overflowed"),
        }
    }
}

impl std::error::Error for ProjectionError {}

#[allow(clippy::too_many_lines)]
fn project_registered_witnesses(
    holder: &KnowledgeHolderRef,
    scope: &ResourceScopeId,
    materialized_at: SimTime,
    witnesses: &[TypedObservationWitnessV1],
    price_applicability: PriceEvidenceApplicabilityV1,
    profile_revision: &str,
    compiled_content_hash: &str,
) -> Result<EconomyProjectionV1, ProjectionError> {
    if witnesses.len() > MAX_TYPED_SOURCE_ADAPTERS || witnesses.len() > MAX_ADAPTER_CALLS {
        return Err(ProjectionError::BudgetExceeded("typed source adapters"));
    }
    let fact_count = witnesses
        .iter()
        .try_fold(0_usize, |total, witness| {
            total.checked_add(witness.facts.len())
        })
        .ok_or(ProjectionError::ArithmeticOverflow)?;
    if fact_count > MAX_OBSERVATION_FACTS {
        return Err(ProjectionError::BudgetExceeded("observation facts"));
    }

    let mut sorted = witnesses.to_vec();
    sorted.sort_by(|left, right| {
        (
            left.provider_plugin.as_str(),
            left.adapter_revision.as_str(),
            left.provider_state_version,
            left.digest.as_str(),
        )
            .cmp(&(
                right.provider_plugin.as_str(),
                right.adapter_revision.as_str(),
                right.provider_state_version,
                right.digest.as_str(),
            ))
    });
    let mut available_minimum = 0_u64;
    let mut available_maximum = 0_u64;
    let mut excluded_unreachable_minimum = 0_u64;
    let mut excluded_unreachable_maximum = 0_u64;
    let mut demand_remainder = 0_u64;
    let mut protected_or_buffered = 0_u64;
    let mut route_penalty = 0_u16;
    let mut security_penalty = 0_u16;
    let mut observed_at = materialized_at;
    let mut stale = false;
    let mut causes = BTreeSet::new();
    let mut price_factors = Vec::new();
    let mut witness_digests = Vec::new();
    for witness in &sorted {
        validate_witness(witness)?;
        if &witness.holder != holder || &witness.scope != scope {
            return Err(ProjectionError::HolderMismatch);
        }
        observed_at = observed_at.min(witness.observed_at);
        stale |= witness.materialized_at < materialized_at || witness.observed_at < materialized_at;
        witness_digests.push(witness.digest.clone());
        for fact in &witness.facts {
            match fact {
                ObservationFactV1::Stock {
                    access,
                    available_minimum: minimum,
                    available_maximum: maximum,
                    protected,
                    ..
                } => {
                    protected_or_buffered = protected_or_buffered
                        .checked_add(*protected)
                        .ok_or(ProjectionError::ArithmeticOverflow)?;
                    match access {
                        StockAccessV1::Local | StockAccessV1::RemoteReachable { .. } => {
                            available_minimum = available_minimum
                                .checked_add(*minimum)
                                .ok_or(ProjectionError::ArithmeticOverflow)?;
                            available_maximum = available_maximum
                                .checked_add(*maximum)
                                .ok_or(ProjectionError::ArithmeticOverflow)?;
                        }
                        StockAccessV1::RemoteUnreachable { .. }
                        | StockAccessV1::RemoteUnobserved => {
                            excluded_unreachable_minimum = excluded_unreachable_minimum
                                .checked_add(*minimum)
                                .ok_or(ProjectionError::ArithmeticOverflow)?;
                            excluded_unreachable_maximum = excluded_unreachable_maximum
                                .checked_add(*maximum)
                                .ok_or(ProjectionError::ArithmeticOverflow)?;
                            route_penalty = route_penalty.max(300);
                            causes.insert("distant_stock_unreachable".to_owned());
                        }
                    }
                }
                ObservationFactV1::Demand { remainder, .. } => {
                    demand_remainder = demand_remainder
                        .checked_add(*remainder)
                        .ok_or(ProjectionError::ArithmeticOverflow)?;
                }
                ObservationFactV1::Buffer { target, available } => {
                    protected_or_buffered = protected_or_buffered
                        .checked_add(target.saturating_sub(*available))
                        .ok_or(ProjectionError::ArithmeticOverflow)?;
                    if available < target {
                        causes.insert("buffer_below_target".to_owned());
                    }
                }
                ObservationFactV1::RouteAccess {
                    reachable,
                    delay_minutes,
                    confidence_per_mille,
                    ..
                } => {
                    let penalty = if *reachable {
                        u16::try_from((*delay_minutes / 1_440).min(250)).unwrap_or(250)
                    } else {
                        300
                    };
                    route_penalty = route_penalty.max(penalty);
                    if !reachable {
                        causes.insert("route_unreachable".to_owned());
                    } else if *delay_minutes > 0 {
                        causes.insert("route_delay".to_owned());
                    }
                    if *confidence_per_mille < 500 {
                        causes.insert("route_evidence_uncertain".to_owned());
                    }
                }
                ObservationFactV1::Security {
                    disruption_per_mille,
                } => {
                    security_penalty = security_penalty.max((*disruption_per_mille).min(200));
                    if *disruption_per_mille > 0 {
                        causes.insert("security_disruption".to_owned());
                    }
                }
                ObservationFactV1::Policy {
                    rationed,
                    requisitioned,
                    reserve_release_allowed,
                } => {
                    if *rationed {
                        causes.insert("rationing".to_owned());
                    }
                    if *requisitioned {
                        causes.insert("requisition".to_owned());
                    }
                    if !reserve_release_allowed {
                        causes.insert("reserve_locked".to_owned());
                    }
                }
                ObservationFactV1::Production { blocked, .. } => {
                    if *blocked {
                        causes.insert("production_blocked".to_owned());
                    }
                }
                ObservationFactV1::ForceShortage {
                    demand_forecast,
                    known_stock_maximum,
                    ..
                } => {
                    if known_stock_maximum < demand_forecast {
                        causes.insert("force_shortage".to_owned());
                    }
                }
                ObservationFactV1::Price {
                    kind,
                    resource_revision,
                    quality,
                    unit_revision,
                    observed_scaled,
                    baseline_scaled,
                    scale,
                    observed_at: price_observed_at,
                    effective_from,
                    effective_until,
                    confidence_per_mille,
                    interpretation_rule_revision,
                    source_versions,
                } => {
                    if price_applicability != PriceEvidenceApplicabilityV1::Applicable
                        || materialized_at < *effective_from
                        || materialized_at >= *effective_until
                    {
                        continue;
                    }
                    if *baseline_scaled <= 0
                        || interpretation_rule_revision.trim().is_empty()
                        || source_versions.is_empty()
                    {
                        return Err(ProjectionError::InvalidProviderWitness(
                            "price evidence requires baseline, exact rule, and source versions"
                                .to_owned(),
                        ));
                    }
                    let delta = i128::from(*observed_scaled) - i128::from(*baseline_scaled);
                    let change = delta
                        .checked_mul(1_000)
                        .and_then(|value| value.checked_div(i128::from(*baseline_scaled)))
                        .ok_or(ProjectionError::ArithmeticOverflow)?;
                    price_factors.push(PricePressureFactor {
                        evidence_kind: *kind,
                        scope: scope.clone(),
                        resource_revision: resource_revision.clone(),
                        quality: quality.clone(),
                        unit_revision: unit_revision.clone(),
                        observed_scaled: *observed_scaled,
                        baseline_scaled: *baseline_scaled,
                        scale: *scale,
                        change_per_mille: i32::try_from(change.clamp(-10_000, 10_000))
                            .map_err(|_| ProjectionError::ArithmeticOverflow)?,
                        observed_at: *price_observed_at,
                        materialized_at,
                        effective_from: *effective_from,
                        effective_until: *effective_until,
                        confidence_per_mille: witness
                            .confidence_per_mille
                            .min(*confidence_per_mille),
                        interpretation_rule_revision: interpretation_rule_revision.clone(),
                        source_versions: source_versions.clone(),
                        witness_digest: witness.digest.clone(),
                    });
                }
            }
        }
    }
    if price_factors.len() > MAX_PRICE_FACTORS {
        return Err(ProjectionError::BudgetExceeded("price factors"));
    }
    let shortage = demand_remainder.saturating_sub(available_minimum);
    if shortage > 0 {
        causes.insert("known_demand_exceeds_known_supply".to_owned());
    }
    let material_pressure = if demand_remainder == 0 {
        0_u64
    } else {
        shortage
            .saturating_mul(1_000)
            .checked_div(demand_remainder)
            .unwrap_or_default()
    };
    let scarcity = material_pressure
        .saturating_add(u64::from(route_penalty))
        .saturating_add(u64::from(security_penalty))
        .min(2_000);
    let input_digest = digest(
        "canwu.economy.scarcity-input.v1",
        &(compiled_content_hash, profile_revision, &sorted),
    )?;
    let scarcity_projection = LocalScarcityProjection {
        holder: holder.clone(),
        scope: scope.clone(),
        observed_at,
        materialized_at,
        known_available_minimum: available_minimum,
        known_available_maximum: available_maximum,
        excluded_unreachable_minimum,
        excluded_unreachable_maximum,
        known_demand_remainder: demand_remainder,
        protected_or_buffered,
        route_penalty_per_mille: route_penalty,
        security_penalty_per_mille: security_penalty,
        scarcity_per_mille: u16::try_from(scarcity)
            .map_err(|_| ProjectionError::ArithmeticOverflow)?,
        stale,
        causes: causes.into_iter().collect(),
        witness_digests,
        input_digest: input_digest.clone(),
    };
    price_factors.sort_by(|left, right| {
        (
            left.evidence_kind,
            left.interpretation_rule_revision.as_str(),
            left.witness_digest.as_str(),
        )
            .cmp(&(
                right.evidence_kind,
                right.interpretation_rule_revision.as_str(),
                right.witness_digest.as_str(),
            ))
    });
    let (status, pressure_per_mille) =
        if price_applicability == PriceEvidenceApplicabilityV1::NotApplicable {
            (PriceProjectionStatus::NotApplicable, None)
        } else if price_factors.is_empty() {
            (PriceProjectionStatus::ExplicitUnknown, None)
        } else {
            let total = price_factors.iter().try_fold(0_i64, |total, factor| {
                total
                    .checked_add(i64::from(factor.change_per_mille))
                    .ok_or(ProjectionError::ArithmeticOverflow)
            })?;
            let mean = total
                / i64::try_from(price_factors.len())
                    .map_err(|_| ProjectionError::ArithmeticOverflow)?;
            (
                if price_factors.len() == 1 {
                    PriceProjectionStatus::Observed
                } else {
                    PriceProjectionStatus::InferredPressure
                },
                Some(i32::try_from(mean).map_err(|_| ProjectionError::ArithmeticOverflow)?),
            )
        };
    let price_input_digest = digest(
        "canwu.economy.price-pressure-input.v1",
        &(
            REFERENCE_VERSION,
            compiled_content_hash,
            profile_revision,
            &input_digest,
            &price_factors,
            price_applicability,
        ),
    )?;
    Ok(EconomyProjectionV1 {
        scarcity: scarcity_projection,
        price_pressure: PricePressureProjection {
            holder: holder.clone(),
            scope: scope.clone(),
            observed_at,
            materialized_at,
            status,
            pressure_per_mille,
            factors: price_factors,
            scarcity_input_digest: input_digest,
            input_digest: price_input_digest,
        },
    })
}

fn latest_route_access(
    witness: &EconomyObservationWitnessV1,
) -> BTreeMap<ResourceScopeId, StockAccessV1> {
    let mut access = BTreeMap::new();
    for route in &witness.routes {
        let candidate = if route.reachable {
            StockAccessV1::RemoteReachable {
                delay_minutes: route.delay_minutes,
                confidence_per_mille: route.confidence_per_mille,
            }
        } else {
            StockAccessV1::RemoteUnreachable {
                confidence_per_mille: route.confidence_per_mille,
            }
        };
        let replace = access.get(&route.source_scope).is_none_or(|prior| {
            matches!(
                (&candidate, prior),
                (
                    StockAccessV1::RemoteReachable { .. },
                    StockAccessV1::RemoteUnreachable { .. }
                )
            ) || matches!(
                (&candidate, prior),
                (
                    StockAccessV1::RemoteReachable { delay_minutes: candidate_delay, .. },
                    StockAccessV1::RemoteReachable { delay_minutes: prior_delay, .. }
                ) if candidate_delay < prior_delay
            )
        });
        if replace {
            access.insert(route.source_scope.clone(), candidate);
        }
    }
    access
}

fn validate_economy_witness(witness: &EconomyObservationWitnessV1) -> Result<(), ProjectionError> {
    let mut detached = witness.clone();
    detached.canonical_digest.clear();
    if witness.provider_plugin != crate::PLUGIN_NAME
        || witness.provider_semantic_hash != crate::ECONOMY_SEMANTIC_HASH
        || witness.provider_state_revision == 0
        || witness.confidence_per_mille > 1_000
        || witness.adapter_revision != ECONOMY_ADAPTER_REVISION
        || witness.canonical_digest != digest("canwu.economy.observation-witness.v1", &detached)?
    {
        return Err(ProjectionError::InvalidProviderWitness(
            "economy provider witness differs from the registered adapter".to_owned(),
        ));
    }
    Ok(())
}

fn validate_witness(witness: &TypedObservationWitnessV1) -> Result<(), ProjectionError> {
    let approved_adapter = matches!(
        (
            witness.provider_plugin.as_str(),
            witness.adapter_revision.as_str()
        ),
        ("canwu-resource", RESOURCE_ADAPTER_REVISION)
            | ("canwu-production", PRODUCTION_ADAPTER_REVISION)
            | ("canwu-force-supply-reference", FORCE_ADAPTER_REVISION)
            | (crate::PLUGIN_NAME, ECONOMY_ADAPTER_REVISION)
    );
    let mut detached = witness.clone();
    detached.digest.clear();
    if !approved_adapter
        || witness.provider_version.trim().is_empty()
        || witness.provider_semantic_hash.len() != 64
        || witness.provider_state_version == 0
        || witness.provider_head_digest.len() != 64
        || witness.confidence_per_mille > 1_000
        || witness.facts.len() > MAX_OBSERVATION_FACTS
        || witness.digest != digest("canwu.economy.typed-observation-witness.v1", &detached)?
    {
        return Err(ProjectionError::InvalidProviderWitness(
            "typed witness metadata, registered adapter, or digest differs".to_owned(),
        ));
    }
    Ok(())
}

fn provider_error(provider: &str, error: CanwuError) -> ProjectionError {
    ProjectionError::ProviderUnavailable {
        provider: provider.to_owned(),
        reason: error.message,
    }
}

fn digest<T: Serialize>(domain: &str, value: &T) -> Result<String, ProjectionError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| ProjectionError::InvalidProviderWitness(error.to_string()))?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain.as_bytes());
    hasher.update(&[0]);
    hasher.update(&bytes);
    Ok(hasher.finalize().to_hex().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use canwu_api::{DomainRecordKind, DomainRecordRef, DomainRecordVersionSource, PersonId};

    fn source_version() -> DomainRecordVersionRef {
        DomainRecordVersionRef {
            record: DomainRecordRef {
                kind: DomainRecordKind::new("canwu.test", "price-source"),
                id: "canwu.test:price-source:1".to_owned(),
            },
            version: 1,
            established_by: DomainRecordVersionSource::InitialScenario,
        }
    }

    #[test]
    fn protected_and_reserved_stock_never_satisfies_local_demand() {
        let stock = canwu_resource::ResourceStockObservationV1 {
            account: ResourceAccountId::new("economy:account:protected-grain").expect("account"),
            scope: ResourceScopeId::new("economy:scope:granary").expect("scope"),
            known_minimum: 20,
            known_maximum: 30,
            reserved: 2,
            protected: 10,
        };
        assert_eq!(unprotected_available(&stock), (8, 18));
    }

    #[test]
    fn substituted_provider_head_digest_invalidates_the_typed_witness() {
        let holder = KnowledgeHolderRef::Person(PersonId::new(9));
        let scope = ResourceScopeId::new("economy:scope:witness-integrity").expect("scope");
        let mut witness = TypedObservationWitnessV1::new(
            "canwu-resource",
            "0.10.0",
            &"a".repeat(64),
            7,
            &"b".repeat(64),
            &holder,
            &scope,
            SimTime::EPOCH,
            SimTime::EPOCH,
            900,
            RESOURCE_ADAPTER_REVISION,
            Vec::new(),
            vec![source_version()],
        )
        .expect("witness");
        witness.provider_head_digest = "c".repeat(64);
        assert!(validate_witness(&witness).is_err());
    }

    #[test]
    fn unreachable_stock_drives_scarcity_but_never_invents_a_price() {
        let holder = KnowledgeHolderRef::Person(PersonId::new(1));
        let scope = ResourceScopeId::new("economy:scope:test").expect("scope");
        let remote = ResourceScopeId::new("economy:scope:remote").expect("remote scope");
        let account = ResourceAccountId::new("economy:account:remote-grain").expect("account");
        let witness = TypedObservationWitnessV1::new(
            "canwu-resource",
            "0.10.0",
            &"a".repeat(64),
            1,
            &"d".repeat(64),
            &holder,
            &scope,
            SimTime::EPOCH,
            SimTime::EPOCH,
            800,
            RESOURCE_ADAPTER_REVISION,
            vec![
                ObservationFactV1::Stock {
                    account,
                    source_scope: remote,
                    access: StockAccessV1::RemoteUnobserved,
                    available_minimum: 20,
                    available_maximum: 30,
                    protected: 10,
                },
                ObservationFactV1::Demand {
                    requested: 100,
                    fulfilled: 0,
                    remainder: 100,
                },
            ],
            Vec::new(),
        )
        .expect("witness");
        let projection = project_registered_witnesses(
            &holder,
            &scope,
            SimTime::EPOCH,
            &[witness],
            PriceEvidenceApplicabilityV1::Applicable,
            "economy:profile:v1",
            &"b".repeat(64),
        )
        .expect("projection");
        assert!(projection.scarcity.scarcity_per_mille > 1_000);
        assert_eq!(projection.scarcity.known_available_minimum, 0);
        assert_eq!(projection.scarcity.excluded_unreachable_minimum, 20);
        assert_eq!(
            projection.price_pressure.status,
            PriceProjectionStatus::ExplicitUnknown
        );
        assert_eq!(projection.price_pressure.pressure_per_mille, None);
    }

    #[test]
    fn exact_effective_price_evidence_produces_observed_pressure() {
        let holder = KnowledgeHolderRef::Person(PersonId::new(2));
        let scope = ResourceScopeId::new("economy:scope:exchange").expect("scope");
        let at = SimTime::from_minutes(100);
        let witness = TypedObservationWitnessV1::new(
            crate::PLUGIN_NAME,
            "0.10.0",
            &"b".repeat(64),
            2,
            &"d".repeat(64),
            &holder,
            &scope,
            at,
            at,
            850,
            ECONOMY_ADAPTER_REVISION,
            vec![ObservationFactV1::Price {
                kind: PriceEvidenceKind::ExecutedExchange,
                resource_revision: ResourceDefinitionRevisionId::new("economy:resource:grain:v1")
                    .expect("resource"),
                quality: ResourceQualityId::new("economy:quality:grain:v1").expect("quality"),
                unit_revision: ResourceUnitRevisionId::new("economy:unit:grain:v1").expect("unit"),
                observed_scaled: 125,
                baseline_scaled: 100,
                scale: 2,
                observed_at: at,
                effective_from: SimTime::EPOCH,
                effective_until: SimTime::from_minutes(200),
                confidence_per_mille: 850,
                interpretation_rule_revision: "economy:price-rule:v1".to_owned(),
                source_versions: vec![source_version()],
            }],
            vec![source_version()],
        )
        .expect("witness");
        let projection = project_registered_witnesses(
            &holder,
            &scope,
            at,
            &[witness],
            PriceEvidenceApplicabilityV1::Applicable,
            "economy:profile:v1",
            &"c".repeat(64),
        )
        .expect("projection");
        assert_eq!(
            projection.price_pressure.status,
            PriceProjectionStatus::Observed
        );
        assert_eq!(projection.price_pressure.pressure_per_mille, Some(250));
    }
}
