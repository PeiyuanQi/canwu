use crate::{MAX_OBSERVATION_FACTS, MAX_PRICE_FACTORS, PLUGIN_NAME, PLUGIN_NAMESPACE};
use canwu_api::{
    CanwuError, DomainRecord, DomainRecordClass, DomainRecordDraft, DomainRecordLifecycle,
    DomainRecordType, DomainRecordVersionRef, DomainValueKindClass, ErrorCode, KnowledgeHolderRef,
    SimTime, TypedDomainRecordRef, canonical_hash,
};
use canwu_economy_reference_content::CompiledEconomyReferenceContentV1;
use canwu_force_supply_reference::{
    EconomyExternalityOutcomeVersionV1, ExternalityOutcomeId, ForceOperationId,
};
use canwu_resource::{
    CompleteExternalCompletionParticipantGrantV1, CompletionCapacityGrantId,
    CompletionCapacityGrantV1, CompletionCapacityPartitionV1, CompletionGrantStateV1,
    CompletionLeaseAcquisitionId, CompletionLockedTargetV1,
    ConsumeExternalCompletionParticipantGrantV1, ExpireExternalCompletionParticipantGrantsV1,
    ExternalCompletionParticipantGrantV1, PrepareExternalCompletionParticipantGrantV1,
    ReleaseExternalCompletionParticipantGrantV1, RequestExternalCompletionParticipantGrantV1,
    ResourceDefinitionRevisionId, ResourceDemandId, ResourceError,
    ResourceOperationOutcomeVersionV1, ResourceQualityId, ResourceRevision, ResourceScopeId,
    ResourceTransferId, ResourceUnitRevisionId, RunBudgetRevisionV1,
};
use canwu_transport::TransportExecution;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const ECONOMY_ARCHIVE_BLOB_NAMESPACE: &str = "canwu.economy-reference.archive.blob";
pub const ECONOMY_ARCHIVE_MEMBERSHIP_PAGE_NAMESPACE: &str =
    "canwu.economy-reference.archive.membership-page";
pub const ECONOMY_ARCHIVE_TEMPORAL_PAGE_NAMESPACE: &str =
    "canwu.economy-reference.archive.temporal-page";
pub const ECONOMY_ARCHIVE_INDEX_DIRECTORY_NAMESPACE: &str =
    "canwu.economy-reference.archive.index-directory";
pub const ECONOMY_ARCHIVE_DOMAIN: canwu_force_supply_reference::PackageArchiveDomainV1 =
    canwu_force_supply_reference::PackageArchiveDomainV1 {
        digest_prefix: "canwu.economy-reference",
        blob_namespace: ECONOMY_ARCHIVE_BLOB_NAMESPACE,
        membership_namespace: ECONOMY_ARCHIVE_MEMBERSHIP_PAGE_NAMESPACE,
        temporal_namespace: ECONOMY_ARCHIVE_TEMPORAL_PAGE_NAMESPACE,
        directory_namespace: ECONOMY_ARCHIVE_INDEX_DIRECTORY_NAMESPACE,
    };
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", content = "key", rename_all = "snake_case")]
pub enum EconomyArchiveKeyV1 {
    MonthlyFrame(LocalEconomyId, u16),
    ObservationHead {
        holder_scope: String,
        observed_at: SimTime,
        digest: String,
    },
    RouteObservation(EconomyRouteObservationId),
    PriceObservation(EconomyPriceObservationId),
    DeliveryAttempt(EconomyDeliveryAttemptId),
    ExternalityOutcome(ExternalityOutcomeId),
    OperationOutcome(EconomyOperationId),
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "payload", content = "value", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum EconomyArchivePayloadV1 {
    MonthlyFrame(MonthlyEconomyFrameV1),
    ObservationHead(EconomyObservationHeadV1),
    RouteObservation(EconomyRouteObservationV1),
    PriceObservation(EconomyPriceObservationV1),
    DeliveryAttempt(EconomyDeliveryAttemptV1),
    ExternalityOutcome(EconomyExternalityOutcomeVersionV1),
    OperationOutcome(EconomyOperationOutcomeV1),
}

pub type EconomyArchiveBlobV1 = canwu_force_supply_reference::PackageArchiveBlobV1<
    EconomyArchiveKeyV1,
    EconomyArchivePayloadV1,
>;
pub type EconomyArchiveHeadStateV1 = canwu_force_supply_reference::PackageArchiveHeadStateV1;
pub type EconomyArchiveRetentionHandleV1 =
    canwu_force_supply_reference::PackageArchiveRetentionHandleV1;
pub type EconomyArchiveMaintenanceReceiptV1 =
    canwu_force_supply_reference::PackageArchiveMaintenanceReceiptV1;
pub type PreparedEconomyArchiveBatchV1 =
    canwu_force_supply_reference::PreparedPackageArchiveBatchV1<
        EconomyArchiveKeyV1,
        EconomyArchivePayloadV1,
    >;
pub type VerifiedEconomyArchiveCommitV1 =
    canwu_force_supply_reference::VerifiedPackageArchiveCommitV1<EconomyArchiveKeyV1>;
use std::fmt::{Display, Formatter};

pub const ECONOMY_RUNTIME_ID: &str = "canwu.economy-reference:runtime:v1";
pub const ECONOMY_FORMAT_VERSION: u32 = 1;
pub const MAX_ECONOMY_IDENTIFIER_BYTES: usize = 192;

fn invalid(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidDomainRecord, message)
}

#[allow(clippy::needless_pass_by_value)]
fn resource_error(error: ResourceError) -> CanwuError {
    invalid(error.to_string())
}

fn validate_identifier(value: &str, label: &str) -> Result<(), CanwuError> {
    if value.is_empty()
        || value.len() > MAX_ECONOMY_IDENTIFIER_BYTES
        || !value.contains(':')
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':' | b'/')
        })
    {
        return Err(invalid(format!(
            "{label} is not a validated namespaced identifier"
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

typed_id!(LocalEconomyId, "local economy");
typed_id!(EconomyProfileId, "economy profile");
typed_id!(EconomyOperationId, "economy operation");
typed_id!(EconomyObservationGrantId, "economy observation grant");
typed_id!(EconomyRouteObservationId, "economy route observation");
typed_id!(EconomyPriceObservationId, "economy price observation");
typed_id!(EconomyDeliveryAttemptId, "economy delivery attempt");
typed_id!(EconomyRuleRevisionId, "economy rule revision");
typed_id!(
    EconomyRouteProviderRecordId,
    "economy route provider record"
);
typed_id!(
    EconomyPriceProviderRecordId,
    "economy price provider record"
);

#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EconomyRouteProviderRecord;

impl DomainRecordType for EconomyRouteProviderRecord {
    type Payload = EconomyRouteProviderPayloadV1;
    type Class = DomainValueKindClass;
    const NAMESPACE: &'static str = PLUGIN_NAMESPACE;
    const NAME: &'static str = "route-observation-provider";
}

#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EconomyPriceProviderRecord;

impl DomainRecordType for EconomyPriceProviderRecord {
    type Payload = EconomyPriceProviderPayloadV1;
    type Class = DomainValueKindClass;
    const NAMESPACE: &'static str = PLUGIN_NAMESPACE;
    const NAME: &'static str = "price-observation-provider";
}

#[must_use]
pub fn economy_route_provider_reference(
    id: &EconomyRouteProviderRecordId,
) -> TypedDomainRecordRef<EconomyRouteProviderRecord> {
    TypedDomainRecordRef::new(id.as_str())
}

#[must_use]
pub fn economy_price_provider_reference(
    id: &EconomyPriceProviderRecordId,
) -> TypedDomainRecordRef<EconomyPriceProviderRecord> {
    TypedDomainRecordRef::new(id.as_str())
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EconomyReferenceLimitsV1 {
    pub max_local_economies: usize,
    pub max_profiles: usize,
    pub max_frames_per_economy: usize,
    pub max_observation_heads_per_scope: usize,
    pub max_route_observations: usize,
    pub max_price_observations: usize,
    pub max_delivery_attempts: usize,
    pub max_operation_outcomes: usize,
    pub max_observation_grants: usize,
    pub max_state_bytes: usize,
}

impl EconomyReferenceLimitsV1 {
    pub const MAX_LOCAL_ECONOMIES: usize = 4_096;
    pub const MAX_PROFILES: usize = 1_024;
    pub const MAX_FRAMES_PER_ECONOMY: usize = 1_024;
    pub const MAX_OBSERVATION_HEADS_PER_SCOPE: usize = 2_048;
    pub const MAX_ROUTE_OBSERVATIONS: usize = 16_384;
    pub const MAX_PRICE_OBSERVATIONS: usize = 16_384;
    pub const MAX_DELIVERY_ATTEMPTS: usize = 16_384;
    pub const MAX_OPERATION_OUTCOMES: usize = 32_768;
    pub const MAX_OBSERVATION_GRANTS: usize = 8_192;
    pub const MAX_STATE_BYTES: usize = 64 * 1024 * 1024;

    #[must_use]
    pub const fn canonical() -> Self {
        Self {
            max_local_economies: 256,
            max_profiles: 128,
            max_frames_per_economy: 256,
            max_observation_heads_per_scope: 512,
            max_route_observations: 4_096,
            max_price_observations: 4_096,
            max_delivery_attempts: 4_096,
            max_operation_outcomes: 8_192,
            max_observation_grants: 2_048,
            max_state_bytes: 32 * 1024 * 1024,
        }
    }

    pub fn validate(self) -> Result<(), CanwuError> {
        if self.max_local_economies == 0
            || self.max_local_economies > Self::MAX_LOCAL_ECONOMIES
            || self.max_profiles == 0
            || self.max_profiles > Self::MAX_PROFILES
            || self.max_frames_per_economy < 14
            || self.max_frames_per_economy > Self::MAX_FRAMES_PER_ECONOMY
            || self.max_observation_heads_per_scope == 0
            || self.max_observation_heads_per_scope > Self::MAX_OBSERVATION_HEADS_PER_SCOPE
            || self.max_route_observations == 0
            || self.max_route_observations > Self::MAX_ROUTE_OBSERVATIONS
            || self.max_price_observations == 0
            || self.max_price_observations > Self::MAX_PRICE_OBSERVATIONS
            || self.max_delivery_attempts == 0
            || self.max_delivery_attempts > Self::MAX_DELIVERY_ATTEMPTS
            || self.max_operation_outcomes == 0
            || self.max_operation_outcomes > Self::MAX_OPERATION_OUTCOMES
            || self.max_observation_grants == 0
            || self.max_observation_grants > Self::MAX_OBSERVATION_GRANTS
            || self.max_state_bytes == 0
            || self.max_state_bytes > Self::MAX_STATE_BYTES
        {
            return Err(invalid(
                "economy-reference limits exceed the V1 hard maxima",
            ));
        }
        Ok(())
    }
}

impl Default for EconomyReferenceLimitsV1 {
    fn default() -> Self {
        Self::canonical()
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GrainDecision {
    ReliefFirst,
    ForceFirst,
    Balanced,
    RequisitionForForce,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PriceEvidenceApplicabilityV1 {
    NotApplicable,
    Applicable,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryDispositionV1 {
    Pending,
    Accepted,
    Lost,
    Returned,
    CancelledBeforeDebit,
    ExternalOutflow,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PopulationConsumptionProfileV1 {
    pub monthly_need: u64,
    pub shortage_wellbeing_cost_per_unit: u16,
    pub relief_wellbeing_gain_per_unit: u16,
    pub rule_revision: EconomyRuleRevisionId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SeasonalHarvestProfileV1 {
    pub harvest_month: u16,
    pub base_output: u64,
    pub seed_floor: u64,
    pub minimum_environment_per_mille: u16,
    pub rule_revision: EconomyRuleRevisionId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReliefPolicyV1 {
    pub monthly_target: u64,
    pub rule_revision: EconomyRuleRevisionId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RequisitionPolicyV1 {
    pub cooperation_cost_per_use: u16,
    pub next_harvest_penalty_per_mille: u16,
    pub rule_revision: EconomyRuleRevisionId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EconomyProfileV1 {
    pub id: EconomyProfileId,
    pub revision: u64,
    pub synthetic: bool,
    pub compiled_content_hash: String,
    pub definition_ids: BTreeSet<String>,
    pub model_card_ids: BTreeSet<String>,
    pub consumption: PopulationConsumptionProfileV1,
    pub harvest: SeasonalHarvestProfileV1,
    pub relief: ReliefPolicyV1,
    pub requisition: RequisitionPolicyV1,
    pub price_applicability: PriceEvidenceApplicabilityV1,
    pub interpretation_rules: BTreeSet<EconomyRuleRevisionId>,
    pub semantic_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LocalEconomyV1 {
    pub id: LocalEconomyId,
    pub revision: u64,
    pub manager: KnowledgeHolderRef,
    pub scope: ResourceScopeId,
    pub profile: EconomyProfileId,
    pub month: u16,
    pub population_wellbeing_per_mille: u16,
    pub cooperation_per_mille: u16,
    pub pending_harvest_penalty_per_mille: u16,
    pub latest_decision: GrainDecision,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MonthlyEconomyEvidenceV1 {
    pub civilian_demand: ResourceDemandId,
    pub relief_demand: ResourceDemandId,
    pub force_demand: ResourceDemandId,
    pub harvest_credit: Option<ResourceOperationOutcomeVersionV1>,
    pub force_operation: Option<ForceOperationId>,
    pub source_versions: Vec<DomainRecordVersionRef>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MonthlyEconomyFrameV1 {
    pub economy: LocalEconomyId,
    pub month: u16,
    pub at: SimTime,
    pub decision: GrainDecision,
    pub civilian_requested: u64,
    pub civilian_fulfilled: u64,
    pub civilian_remainder: u64,
    pub relief_requested: u64,
    pub relief_fulfilled: u64,
    pub relief_remainder: u64,
    pub force_requested: u64,
    pub force_fulfilled: u64,
    pub force_remainder: u64,
    pub harvest_output: u64,
    pub population_wellbeing_per_mille: u16,
    pub cooperation_per_mille: u16,
    pub evidence: MonthlyEconomyEvidenceV1,
    pub semantic_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EconomyObservationGrantV1 {
    pub id: EconomyObservationGrantId,
    pub holder: KnowledgeHolderRef,
    pub scopes: BTreeSet<ResourceScopeId>,
    pub delay_minutes: u64,
    pub confidence_per_mille: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct EconomyObservationHeadV1 {
    pub economy: LocalEconomyId,
    pub scope: ResourceScopeId,
    pub observed_at: SimTime,
    pub population_wellbeing_per_mille: u16,
    pub cooperation_per_mille: u16,
    pub relief_open: bool,
    pub rationed: bool,
    pub requisitioned: bool,
    pub reserve_release_allowed: bool,
    pub source_versions: Vec<DomainRecordVersionRef>,
    pub semantic_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct EconomyObservationTemporalKeyV1 {
    pub observed_at: SimTime,
    pub canonical_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct EconomyFactTemporalKeyV1 {
    pub observed_at: SimTime,
    pub observation_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EconomyRouteObservationV1 {
    pub id: EconomyRouteObservationId,
    pub route_key: String,
    pub holder: KnowledgeHolderRef,
    pub target_scope: ResourceScopeId,
    pub source_scope: ResourceScopeId,
    pub observed_at: SimTime,
    pub reachable: bool,
    pub delay_minutes: u64,
    pub confidence_per_mille: u16,
    pub provider_source: DomainRecordVersionRef,
    pub source_versions: Vec<DomainRecordVersionRef>,
    pub semantic_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EconomyRouteProviderPayloadV1 {
    pub id: EconomyRouteProviderRecordId,
    pub provider_plugin: String,
    pub route_key: String,
    pub holder: KnowledgeHolderRef,
    pub target_scope: ResourceScopeId,
    pub source_scope: ResourceScopeId,
    pub observed_at: SimTime,
    pub reachable: bool,
    pub delay_minutes: u64,
    pub confidence_per_mille: u16,
    pub source_versions: Vec<DomainRecordVersionRef>,
    pub semantic_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EconomyPriceObservationV1 {
    pub id: EconomyPriceObservationId,
    pub holder: KnowledgeHolderRef,
    pub scope: ResourceScopeId,
    pub observed_at: SimTime,
    pub kind: crate::PriceEvidenceKind,
    pub resource_revision: ResourceDefinitionRevisionId,
    pub quality: ResourceQualityId,
    pub unit_revision: ResourceUnitRevisionId,
    pub observed_scaled: i64,
    pub baseline_scaled: i64,
    pub scale: u32,
    pub effective_from: SimTime,
    pub effective_until: SimTime,
    pub interpretation_rule_revision: EconomyRuleRevisionId,
    pub confidence_per_mille: u16,
    pub provider_source: DomainRecordVersionRef,
    pub source_versions: Vec<DomainRecordVersionRef>,
    pub semantic_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EconomyPriceProviderPayloadV1 {
    pub id: EconomyPriceProviderRecordId,
    pub provider_plugin: String,
    pub holder: KnowledgeHolderRef,
    pub scope: ResourceScopeId,
    pub observed_at: SimTime,
    pub kind: crate::PriceEvidenceKind,
    pub resource_revision: ResourceDefinitionRevisionId,
    pub quality: ResourceQualityId,
    pub unit_revision: ResourceUnitRevisionId,
    pub observed_scaled: i64,
    pub baseline_scaled: i64,
    pub scale: u32,
    pub effective_from: SimTime,
    pub effective_until: SimTime,
    pub interpretation_rule_revision: EconomyRuleRevisionId,
    pub confidence_per_mille: u16,
    pub source_versions: Vec<DomainRecordVersionRef>,
    pub semantic_digest: String,
}

impl EconomyRouteProviderPayloadV1 {
    pub fn seal(mut self) -> Result<Self, CanwuError> {
        self.semantic_digest.clear();
        self.semantic_digest = canonical_hash("canwu.economy.route-provider-payload.v1", &self)?;
        Ok(self)
    }
}

impl EconomyPriceProviderPayloadV1 {
    pub fn seal(mut self) -> Result<Self, CanwuError> {
        self.semantic_digest.clear();
        self.semantic_digest = canonical_hash("canwu.economy.price-provider-payload.v1", &self)?;
        Ok(self)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct EconomyDeliveryAttemptV1 {
    pub id: EconomyDeliveryAttemptId,
    pub economy: LocalEconomyId,
    pub resource_transfer: ResourceTransferId,
    pub source_scope: ResourceScopeId,
    pub target_scope: ResourceScopeId,
    pub disposition: DeliveryDispositionV1,
    pub execution: TransportExecution,
    pub recorded_at: SimTime,
    pub source_versions: Vec<DomainRecordVersionRef>,
    pub semantic_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EconomyOperationOutcomeV1 {
    pub id: EconomyOperationId,
    pub input_digest: String,
    pub applied: bool,
    pub rejection_code: Option<String>,
    pub rejection_reason: Option<String>,
    pub settled_at: SimTime,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(
    try_from = "EconomyReferenceStateStorageV1",
    into = "EconomyReferenceStateStorageV1"
)]
pub struct EconomyReferenceStateV1 {
    pub format_version: u32,
    pub revision: u64,
    pub limits: EconomyReferenceLimitsV1,
    pub compiled_content: Option<CompiledEconomyReferenceContentV1>,
    pub profiles: BTreeMap<EconomyProfileId, EconomyProfileV1>,
    pub local_economies: BTreeMap<LocalEconomyId, LocalEconomyV1>,
    #[serde(default)]
    pub resilience_postures: BTreeMap<LocalEconomyId, String>,
    #[serde(default)]
    pub resource_consumption_intents: BTreeMap<
        canwu_resource::ResourceConsumptionIntentId,
        canwu_resource::ResourceConsumptionIntentV1,
    >,
    pub completion_run_budget: RunBudgetRevisionV1,
    pub completion_participants:
        BTreeMap<CompletionLeaseAcquisitionId, ExternalCompletionParticipantGrantV1>,
    pub completion_target_locks: BTreeMap<CompletionLockedTargetV1, CompletionCapacityGrantId>,
    pub completion_expiry_due: BTreeMap<u64, BTreeSet<CompletionLeaseAcquisitionId>>,
    pub completion_reserved_units: u64,
    pub frames: BTreeMap<LocalEconomyId, Vec<MonthlyEconomyFrameV1>>,
    pub observation_grants: BTreeMap<EconomyObservationGrantId, EconomyObservationGrantV1>,
    pub observation_grant_by_holder_scope: BTreeMap<String, EconomyObservationGrantId>,
    pub observation_heads: BTreeMap<ResourceScopeId, Vec<EconomyObservationHeadV1>>,
    pub observation_head_by_holder_scope: BTreeMap<String, EconomyObservationHeadV1>,
    pub observation_temporal_by_holder_scope:
        BTreeMap<String, BTreeMap<EconomyObservationTemporalKeyV1, EconomyObservationHeadV1>>,
    pub route_heads_by_holder_scope: BTreeMap<String, BTreeMap<String, EconomyRouteObservationId>>,
    pub price_heads_by_holder_scope: BTreeMap<String, BTreeMap<String, EconomyPriceObservationId>>,
    pub route_temporal_by_holder_scope: BTreeMap<
        String,
        BTreeMap<String, BTreeMap<EconomyFactTemporalKeyV1, EconomyRouteObservationId>>,
    >,
    pub price_temporal_by_holder_scope: BTreeMap<
        String,
        BTreeMap<String, BTreeMap<EconomyFactTemporalKeyV1, EconomyPriceObservationId>>,
    >,
    pub route_observations: BTreeMap<EconomyRouteObservationId, EconomyRouteObservationV1>,
    pub price_observations: BTreeMap<EconomyPriceObservationId, EconomyPriceObservationV1>,
    pub delivery_attempts: BTreeMap<EconomyDeliveryAttemptId, EconomyDeliveryAttemptV1>,
    #[serde(default)]
    pub externality_outcomes: BTreeMap<ExternalityOutcomeId, EconomyExternalityOutcomeVersionV1>,
    pub archive_head: EconomyArchiveHeadStateV1,
    pub archive_retention_handles: BTreeMap<String, EconomyArchiveRetentionHandleV1>,
    pub archive_maintenance_receipts: BTreeMap<u64, EconomyArchiveMaintenanceReceiptV1>,
    pub outcomes: BTreeMap<EconomyOperationId, EconomyOperationOutcomeV1>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct EconomyReferenceStateStorageV1 {
    format_version: u32,
    revision: u64,
    limits: EconomyReferenceLimitsV1,
    compiled_content: Option<CompiledEconomyReferenceContentV1>,
    profiles: BTreeMap<EconomyProfileId, EconomyProfileV1>,
    local_economies: BTreeMap<LocalEconomyId, LocalEconomyV1>,
    resilience_postures: BTreeMap<LocalEconomyId, String>,
    #[serde(default)]
    resource_consumption_intents: BTreeMap<
        canwu_resource::ResourceConsumptionIntentId,
        canwu_resource::ResourceConsumptionIntentV1,
    >,
    completion_run_budget: RunBudgetRevisionV1,
    completion_participants:
        BTreeMap<CompletionLeaseAcquisitionId, ExternalCompletionParticipantGrantV1>,
    completion_reserved_units: u64,
    frames: BTreeMap<LocalEconomyId, Vec<MonthlyEconomyFrameV1>>,
    observation_grants: BTreeMap<EconomyObservationGrantId, EconomyObservationGrantV1>,
    observation_heads: BTreeMap<ResourceScopeId, Vec<EconomyObservationHeadV1>>,
    route_observations: BTreeMap<EconomyRouteObservationId, EconomyRouteObservationV1>,
    price_observations: BTreeMap<EconomyPriceObservationId, EconomyPriceObservationV1>,
    delivery_attempts: BTreeMap<EconomyDeliveryAttemptId, EconomyDeliveryAttemptV1>,
    #[serde(default)]
    externality_outcomes: BTreeMap<ExternalityOutcomeId, EconomyExternalityOutcomeVersionV1>,
    archive_head: EconomyArchiveHeadStateV1,
    archive_retention_handles: BTreeMap<String, EconomyArchiveRetentionHandleV1>,
    archive_maintenance_receipts: BTreeMap<u64, EconomyArchiveMaintenanceReceiptV1>,
    outcomes: BTreeMap<EconomyOperationId, EconomyOperationOutcomeV1>,
}

impl From<EconomyReferenceStateV1> for EconomyReferenceStateStorageV1 {
    fn from(state: EconomyReferenceStateV1) -> Self {
        Self {
            format_version: state.format_version,
            revision: state.revision,
            limits: state.limits,
            compiled_content: state.compiled_content,
            profiles: state.profiles,
            local_economies: state.local_economies,
            resilience_postures: state.resilience_postures,
            resource_consumption_intents: state.resource_consumption_intents,
            completion_run_budget: state.completion_run_budget,
            completion_participants: state.completion_participants,
            completion_reserved_units: state.completion_reserved_units,
            frames: state.frames,
            observation_grants: state.observation_grants,
            observation_heads: state.observation_heads,
            route_observations: state.route_observations,
            price_observations: state.price_observations,
            delivery_attempts: state.delivery_attempts,
            externality_outcomes: state.externality_outcomes,
            archive_head: state.archive_head,
            archive_retention_handles: state.archive_retention_handles,
            archive_maintenance_receipts: state.archive_maintenance_receipts,
            outcomes: state.outcomes,
        }
    }
}

impl TryFrom<EconomyReferenceStateStorageV1> for EconomyReferenceStateV1 {
    type Error = CanwuError;

    fn try_from(state: EconomyReferenceStateStorageV1) -> Result<Self, Self::Error> {
        let mut restored = Self {
            format_version: state.format_version,
            revision: state.revision,
            limits: state.limits,
            compiled_content: state.compiled_content,
            profiles: state.profiles,
            local_economies: state.local_economies,
            resilience_postures: state.resilience_postures,
            resource_consumption_intents: state.resource_consumption_intents,
            completion_run_budget: state.completion_run_budget,
            completion_participants: state.completion_participants,
            completion_target_locks: BTreeMap::new(),
            completion_expiry_due: BTreeMap::new(),
            completion_reserved_units: state.completion_reserved_units,
            frames: state.frames,
            observation_grants: state.observation_grants,
            observation_grant_by_holder_scope: BTreeMap::new(),
            observation_heads: state.observation_heads,
            observation_head_by_holder_scope: BTreeMap::new(),
            observation_temporal_by_holder_scope: BTreeMap::new(),
            route_heads_by_holder_scope: BTreeMap::new(),
            price_heads_by_holder_scope: BTreeMap::new(),
            route_temporal_by_holder_scope: BTreeMap::new(),
            price_temporal_by_holder_scope: BTreeMap::new(),
            route_observations: state.route_observations,
            price_observations: state.price_observations,
            delivery_attempts: state.delivery_attempts,
            externality_outcomes: state.externality_outcomes,
            archive_head: state.archive_head,
            archive_retention_handles: state.archive_retention_handles,
            archive_maintenance_receipts: state.archive_maintenance_receipts,
            outcomes: state.outcomes,
        };
        restored.rebuild_derived_indexes()?;
        Ok(restored)
    }
}

impl Default for EconomyReferenceStateV1 {
    fn default() -> Self {
        Self {
            format_version: ECONOMY_FORMAT_VERSION,
            revision: 1,
            limits: EconomyReferenceLimitsV1::canonical(),
            compiled_content: None,
            profiles: BTreeMap::new(),
            local_economies: BTreeMap::new(),
            resilience_postures: BTreeMap::new(),
            resource_consumption_intents: BTreeMap::new(),
            completion_run_budget: default_completion_run_budget(),
            completion_participants: BTreeMap::new(),
            completion_target_locks: BTreeMap::new(),
            completion_expiry_due: BTreeMap::new(),
            completion_reserved_units: 0,
            frames: BTreeMap::new(),
            observation_grants: BTreeMap::new(),
            observation_grant_by_holder_scope: BTreeMap::new(),
            observation_heads: BTreeMap::new(),
            observation_head_by_holder_scope: BTreeMap::new(),
            observation_temporal_by_holder_scope: BTreeMap::new(),
            route_heads_by_holder_scope: BTreeMap::new(),
            price_heads_by_holder_scope: BTreeMap::new(),
            route_temporal_by_holder_scope: BTreeMap::new(),
            price_temporal_by_holder_scope: BTreeMap::new(),
            route_observations: BTreeMap::new(),
            price_observations: BTreeMap::new(),
            delivery_attempts: BTreeMap::new(),
            externality_outcomes: BTreeMap::new(),
            archive_head: canwu_force_supply_reference::sealed_archive_head(
                ECONOMY_ARCHIVE_DOMAIN,
                EconomyArchiveHeadStateV1::default(),
            )
            .expect("static empty economy archive head must seal"),
            archive_retention_handles: BTreeMap::new(),
            archive_maintenance_receipts: BTreeMap::new(),
            outcomes: BTreeMap::new(),
        }
    }
}

fn default_completion_run_budget() -> RunBudgetRevisionV1 {
    RunBudgetRevisionV1 {
        revision: ResourceRevision::INITIAL,
        total_completion_units: 2_000_000,
        shared_pending_slots: 32,
        partitions: Vec::new(),
        semantic_digest: String::new(),
    }
    .seal()
    .expect("static economy completion budget must be valid")
}

#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EconomyReferenceRuntimeRecord;

impl DomainRecordType for EconomyReferenceRuntimeRecord {
    type Payload = EconomyReferenceStateV1;
    type Class = DomainValueKindClass;

    const NAMESPACE: &'static str = PLUGIN_NAMESPACE;
    const NAME: &'static str = "runtime";
}

#[must_use]
pub fn economy_reference_runtime_reference() -> TypedDomainRecordRef<EconomyReferenceRuntimeRecord>
{
    TypedDomainRecordRef::new(ECONOMY_RUNTIME_ID)
}

impl EconomyProfileV1 {
    pub fn seal(mut self) -> Result<Self, CanwuError> {
        self.semantic_digest.clear();
        self.semantic_digest = canonical_hash("canwu.economy.profile.v1", &self)?;
        self.validate()?;
        Ok(self)
    }

    pub fn validate(&self) -> Result<(), CanwuError> {
        let mut detached = self.clone();
        detached.semantic_digest.clear();
        if self.revision == 0
            || !self.synthetic
            || self.compiled_content_hash.len() != 64
            || self.definition_ids.is_empty()
            || self.model_card_ids.is_empty()
            || self.consumption.monthly_need == 0
            || self.harvest.harvest_month == 0
            || self.harvest.harvest_month > 12
            || self.harvest.base_output == 0
            || self.harvest.minimum_environment_per_mille > 1_000
            || self.interpretation_rules.is_empty()
            || self.semantic_digest != canonical_hash("canwu.economy.profile.v1", &detached)?
        {
            return Err(invalid(
                "economy profile is not a sealed synthetic V1 reference profile",
            ));
        }
        Ok(())
    }
}

impl EconomyReferenceStateV1 {
    #[allow(clippy::too_many_lines)]
    pub fn rebuild_derived_indexes(&mut self) -> Result<(), CanwuError> {
        self.completion_target_locks.clear();
        self.completion_expiry_due.clear();
        for participant in self.completion_participants.values() {
            if matches!(
                participant.grant.state,
                CompletionGrantStateV1::Prepared | CompletionGrantStateV1::Consumed
            ) {
                for target in &participant.grant.target_versions {
                    if self
                        .completion_target_locks
                        .insert(target.clone(), participant.grant.id.clone())
                        .is_some()
                    {
                        return Err(invalid(
                            "economy completion restore found a duplicate target lock",
                        ));
                    }
                }
            }
            if matches!(
                participant.grant.state,
                CompletionGrantStateV1::Held | CompletionGrantStateV1::Prepared
            ) {
                self.completion_expiry_due
                    .entry(participant.grant.expires_after_boundary)
                    .or_default()
                    .insert(participant.grant.acquisition.clone());
            }
        }

        self.observation_grant_by_holder_scope.clear();
        for grant in self.observation_grants.values() {
            for scope in &grant.scopes {
                let key = holder_scope_index_key(&grant.holder, scope)?;
                if self
                    .observation_grant_by_holder_scope
                    .insert(key, grant.id.clone())
                    .is_some()
                {
                    return Err(invalid(
                        "economy restore found duplicate holder/scope observation grants",
                    ));
                }
            }
        }

        self.observation_head_by_holder_scope.clear();
        self.observation_temporal_by_holder_scope.clear();
        for heads in self.observation_heads.values() {
            for head in heads {
                let local = self
                    .local_economies
                    .get(&head.economy)
                    .ok_or_else(|| invalid("economy observation head lost its local economy"))?;
                let holder_scope = holder_scope_index_key(&local.manager, &head.scope)?;
                self.observation_temporal_by_holder_scope
                    .entry(holder_scope.clone())
                    .or_default()
                    .insert(
                        EconomyObservationTemporalKeyV1 {
                            observed_at: head.observed_at,
                            canonical_digest: head.semantic_digest.clone(),
                        },
                        head.clone(),
                    );
                self.observation_head_by_holder_scope
                    .insert(holder_scope, head.clone());
            }
        }

        self.route_heads_by_holder_scope.clear();
        self.route_temporal_by_holder_scope.clear();
        for observation in self.route_observations.values() {
            let holder_scope =
                holder_scope_index_key(&observation.holder, &observation.target_scope)?;
            let fact = route_head_index_key(&observation.route_key, &observation.source_scope);
            self.route_temporal_by_holder_scope
                .entry(holder_scope.clone())
                .or_default()
                .entry(fact.clone())
                .or_default()
                .insert(
                    EconomyFactTemporalKeyV1 {
                        observed_at: observation.observed_at,
                        observation_id: observation.id.to_string(),
                    },
                    observation.id.clone(),
                );
            let heads = self
                .route_heads_by_holder_scope
                .entry(holder_scope)
                .or_default();
            let replace = heads.get(&fact).is_none_or(|current| {
                let current = &self.route_observations[current];
                (current.observed_at, &current.id) < (observation.observed_at, &observation.id)
            });
            if replace {
                heads.insert(fact, observation.id.clone());
            }
        }

        self.price_heads_by_holder_scope.clear();
        self.price_temporal_by_holder_scope.clear();
        for observation in self.price_observations.values() {
            let holder_scope = holder_scope_index_key(&observation.holder, &observation.scope)?;
            let fact =
                price_head_index_key(observation.kind, &observation.interpretation_rule_revision);
            self.price_temporal_by_holder_scope
                .entry(holder_scope.clone())
                .or_default()
                .entry(fact.clone())
                .or_default()
                .insert(
                    EconomyFactTemporalKeyV1 {
                        observed_at: observation.observed_at,
                        observation_id: observation.id.to_string(),
                    },
                    observation.id.clone(),
                );
            let heads = self
                .price_heads_by_holder_scope
                .entry(holder_scope)
                .or_default();
            let replace = heads.get(&fact).is_none_or(|current| {
                let current = &self.price_observations[current];
                (current.observed_at, &current.id) < (observation.observed_at, &observation.id)
            });
            if replace {
                heads.insert(fact, observation.id.clone());
            }
        }
        Ok(())
    }

    pub fn configure_completion_authority(
        &mut self,
        holder: KnowledgeHolderRef,
        operation_namespace: &str,
    ) -> Result<(), CanwuError> {
        if self
            .completion_run_budget
            .partitions
            .iter()
            .any(|partition| {
                partition.authority == holder
                    && partition.operation_namespace == operation_namespace
            })
        {
            return Ok(());
        }
        if !self.completion_participants.is_empty() {
            return Err(invalid(
                "economy completion budget cannot change after participant admission",
            ));
        }
        self.completion_run_budget
            .partitions
            .push(CompletionCapacityPartitionV1 {
                authority: holder,
                operation_namespace: operation_namespace.to_owned(),
                guaranteed_units: 256_000,
                reserved_pending_slots: 8,
                maximum_burst_units: 256_000,
                request_token_capacity: 8,
                request_token_refill_minutes: 1,
                reacquire_cooldown_minutes: 1,
                root_acquisition_cap_per_sim_time: 8,
                guaranteed_max_wait_boundaries: 8,
            });
        self.completion_run_budget
            .partitions
            .sort_by(|left, right| {
                (&left.authority, &left.operation_namespace)
                    .cmp(&(&right.authority, &right.operation_namespace))
            });
        self.completion_run_budget.semantic_digest.clear();
        self.completion_run_budget = self
            .completion_run_budget
            .clone()
            .seal()
            .map_err(resource_error)?;
        Ok(())
    }

    pub fn grant_completion_participant(
        &mut self,
        request: RequestExternalCompletionParticipantGrantV1,
    ) -> Result<ExternalCompletionParticipantGrantV1, CanwuError> {
        request.recipe.validate().map_err(resource_error)?;
        let units = request.recipe.canonical_units().map_err(resource_error)?;
        let mut targets = request.target_versions.clone();
        targets.sort();
        targets.dedup();
        let partition = self
            .completion_run_budget
            .partitions
            .iter()
            .find(|partition| {
                partition.authority == request.holder
                    && partition.operation_namespace == request.operation_namespace
            })
            .ok_or_else(|| invalid("economy completion authority is not partitioned"))?;
        if request.coordinator_plugin != canwu_force_supply_reference::PLUGIN_NAME
            || request.eligibility_envelope_digest.is_empty()
            || targets != request.target_versions
            || request.target_versions.is_empty()
            || units > partition.guaranteed_units
            || self
                .completion_reserved_units
                .checked_add(units)
                .is_none_or(|value| value > self.completion_run_budget.total_completion_units)
        {
            return Err(invalid(
                "economy completion participant request is invalid or over budget",
            ));
        }
        if let Some(existing) = self.completion_participants.get(&request.acquisition) {
            if existing.coordinator_source == request.coordinator_source
                && existing.grant.operation_key == request.operation_key
                && existing.grant.id == request.grant_id
                && existing.grant.target_versions == request.target_versions
                && existing.eligibility_envelope_digest == request.eligibility_envelope_digest
            {
                return Ok(existing.clone());
            }
            return Err(invalid(
                "economy completion participant acquisition is already bound differently",
            ));
        }
        let grant = CompletionCapacityGrantV1 {
            id: request.grant_id,
            revision: ResourceRevision::INITIAL,
            acquisition: request.acquisition.clone(),
            operation_key: request.operation_key,
            owner_plugin: PLUGIN_NAME.to_owned(),
            run_budget_revision: self.completion_run_budget.revision,
            target_versions: request.target_versions,
            recipe_digest: request.recipe.digest().map_err(resource_error)?,
            reserved_units: units,
            expires_after_boundary: request
                .current_boundary
                .checked_add(canwu_resource::PREACTIVATION_LEASE_TTL_BOUNDARIES)
                .ok_or_else(|| invalid("economy completion expiry overflowed"))?,
            activation_deadline_boundary: None,
            state: CompletionGrantStateV1::Held,
            rejection: None,
        };
        let participant = ExternalCompletionParticipantGrantV1 {
            coordinator_plugin: request.coordinator_plugin,
            coordinator_source: request.coordinator_source,
            coordinator_acquisition_revision: request.coordinator_acquisition_revision,
            holder: request.holder,
            operation_namespace: request.operation_namespace,
            eligibility_time: request.eligibility_time,
            eligibility_envelope_digest: request.eligibility_envelope_digest,
            recipe: request.recipe,
            policy_class: request.policy_class,
            grant,
            certificate: None,
        };
        self.completion_reserved_units = self
            .completion_reserved_units
            .checked_add(units)
            .ok_or_else(|| invalid("economy completion reserve overflowed"))?;
        self.completion_expiry_due
            .entry(participant.grant.expires_after_boundary)
            .or_default()
            .insert(request.acquisition.clone());
        self.completion_participants
            .insert(request.acquisition, participant.clone());
        Ok(participant)
    }

    pub fn prepare_completion_participant(
        &mut self,
        request: PrepareExternalCompletionParticipantGrantV1,
    ) -> Result<ExternalCompletionParticipantGrantV1, CanwuError> {
        let snapshot = self
            .completion_participants
            .get(&request.acquisition)
            .cloned()
            .ok_or_else(|| invalid("economy completion participant is unavailable"))?;
        if snapshot.grant.revision != request.expected_grant_revision
            || snapshot.grant.state != CompletionGrantStateV1::Held
            || snapshot.eligibility_envelope_digest != request.eligibility_envelope_digest
            || request
                .current_boundary
                .checked_add(canwu_resource::ACTIVATION_GUARD_BOUNDARIES)
                .is_none_or(|guard| guard > snapshot.grant.expires_after_boundary)
            || snapshot
                .grant
                .target_versions
                .iter()
                .any(|target| self.completion_target_locks.contains_key(target))
        {
            return Err(invalid(
                "economy completion prepare exact grant, envelope, window, or lock differs",
            ));
        }
        for target in &snapshot.grant.target_versions {
            self.completion_target_locks
                .insert(target.clone(), snapshot.grant.id.clone());
        }
        let participant = self
            .completion_participants
            .get_mut(&request.acquisition)
            .ok_or_else(|| invalid("economy completion participant disappeared during prepare"))?;
        participant.coordinator_source = request.coordinator_source;
        participant.grant.state = CompletionGrantStateV1::Prepared;
        participant.grant.activation_deadline_boundary = Some(
            participant
                .grant
                .expires_after_boundary
                .saturating_sub(canwu_resource::ACTIVATION_GUARD_BOUNDARIES - 1),
        );
        participant.grant.revision = participant.grant.revision.next().map_err(resource_error)?;
        Ok(participant.clone())
    }

    pub fn consume_completion_participant(
        &mut self,
        request: ConsumeExternalCompletionParticipantGrantV1,
    ) -> Result<ExternalCompletionParticipantGrantV1, CanwuError> {
        let mut detached = request.certificate.clone();
        let digest = std::mem::take(&mut detached.semantic_digest);
        if digest
            != canonical_hash(
                "canwu.resource.completion-activation-certificate.v1",
                &detached,
            )?
        {
            return Err(invalid(
                "economy completion participant certificate digest is invalid",
            ));
        }
        let participant = self
            .completion_participants
            .get_mut(&request.certificate.acquisition)
            .ok_or_else(|| invalid("economy completion participant is unavailable"))?;
        if participant.grant.state != CompletionGrantStateV1::Prepared
            || participant.grant.operation_key != request.certificate.operation_key
            || participant.eligibility_time != request.at
            || participant.eligibility_time != request.certificate.eligibility_time
            || participant.eligibility_envelope_digest
                != request.certificate.eligibility_envelope_digest
            || !request
                .certificate
                .prepared_grants
                .contains(&(participant.grant.id.clone(), participant.grant.revision))
            || participant.grant.target_versions.iter().any(|target| {
                self.completion_target_locks.get(target) != Some(&participant.grant.id)
                    || !request.certificate.locked_target_versions.contains(target)
            })
        {
            return Err(invalid(
                "economy completion participant certificate does not bind its prepared grant",
            ));
        }
        participant.coordinator_source = request.coordinator_source;
        participant.certificate = Some(request.certificate);
        participant.grant.state = CompletionGrantStateV1::Consumed;
        participant.grant.revision = participant.grant.revision.next().map_err(resource_error)?;
        for values in self.completion_expiry_due.values_mut() {
            values.remove(&participant.grant.acquisition);
        }
        self.completion_expiry_due
            .retain(|_, values| !values.is_empty());
        Ok(participant.clone())
    }

    pub fn complete_completion_participant(
        &mut self,
        request: &CompleteExternalCompletionParticipantGrantV1,
    ) -> Result<(), CanwuError> {
        let participant = self
            .completion_participants
            .get_mut(&request.acquisition)
            .ok_or_else(|| invalid("economy completion participant is unavailable"))?;
        if participant.grant.operation_key != request.operation_key {
            return Err(invalid(
                "economy completion participant names another operation",
            ));
        }
        if participant.grant.state == CompletionGrantStateV1::Completed {
            return Ok(());
        }
        if participant.grant.state != CompletionGrantStateV1::Consumed {
            return Err(invalid(
                "economy completion completion requires a consumed grant",
            ));
        }
        participant.grant.state = CompletionGrantStateV1::Completed;
        participant.grant.revision = participant.grant.revision.next().map_err(resource_error)?;
        self.completion_reserved_units = self
            .completion_reserved_units
            .checked_sub(participant.grant.reserved_units)
            .ok_or_else(|| invalid("economy completion reserve underflowed"))?;
        for target in &participant.grant.target_versions {
            self.completion_target_locks.remove(target);
        }
        Ok(())
    }

    pub fn release_completion_participant(
        &mut self,
        request: ReleaseExternalCompletionParticipantGrantV1,
    ) -> Result<(), CanwuError> {
        let participant = self
            .completion_participants
            .get_mut(&request.acquisition)
            .ok_or_else(|| invalid("economy completion participant is unavailable"))?;
        if participant.grant.revision != request.expected_grant_revision
            || participant.grant.state == CompletionGrantStateV1::Consumed
        {
            return Err(invalid(
                "economy completion release is stale or already activated",
            ));
        }
        if matches!(
            participant.grant.state,
            CompletionGrantStateV1::Released
                | CompletionGrantStateV1::Expired
                | CompletionGrantStateV1::Completed
        ) {
            return Ok(());
        }
        participant.coordinator_source = request.coordinator_source;
        participant.grant.state = CompletionGrantStateV1::Released;
        participant.grant.rejection = Some(request.reason);
        participant.grant.revision = participant.grant.revision.next().map_err(resource_error)?;
        self.completion_reserved_units = self
            .completion_reserved_units
            .checked_sub(participant.grant.reserved_units)
            .ok_or_else(|| invalid("economy completion reserve underflowed"))?;
        for target in &participant.grant.target_versions {
            self.completion_target_locks.remove(target);
        }
        for values in self.completion_expiry_due.values_mut() {
            values.remove(&request.acquisition);
        }
        self.completion_expiry_due
            .retain(|_, values| !values.is_empty());
        Ok(())
    }

    pub fn expire_completion_participants(
        &mut self,
        request: &ExpireExternalCompletionParticipantGrantsV1,
    ) -> Result<(), CanwuError> {
        let candidates = self
            .completion_expiry_due
            .range(..=request.current_boundary)
            .flat_map(|(_, values)| values.iter().cloned())
            .take(request.candidate_limit.saturating_add(1))
            .collect::<Vec<_>>();
        if request.candidate_limit == 0
            || candidates.len() > request.candidate_limit
            || request.candidate_limit > canwu_resource::MAX_PENDING_LEASE_ACQUISITIONS_GLOBAL
        {
            return Err(invalid(
                "economy completion expiry candidate budget is invalid",
            ));
        }
        for acquisition in &candidates {
            let participant = self
                .completion_participants
                .get_mut(acquisition)
                .ok_or_else(|| invalid("economy completion expiry index is orphaned"))?;
            if request.at < participant.eligibility_time {
                return Err(invalid(
                    "economy completion expiry precedes eligibility time",
                ));
            }
            if matches!(
                participant.grant.state,
                CompletionGrantStateV1::Held | CompletionGrantStateV1::Prepared
            ) {
                participant.grant.state = CompletionGrantStateV1::Expired;
                participant.grant.revision =
                    participant.grant.revision.next().map_err(resource_error)?;
                self.completion_reserved_units = self
                    .completion_reserved_units
                    .checked_sub(participant.grant.reserved_units)
                    .ok_or_else(|| invalid("economy completion reserve underflowed"))?;
                for target in &participant.grant.target_versions {
                    self.completion_target_locks.remove(target);
                }
            }
        }
        for values in self.completion_expiry_due.values_mut() {
            values.retain(|acquisition| !candidates.contains(acquisition));
        }
        self.completion_expiry_due
            .retain(|_, values| !values.is_empty());
        Ok(())
    }

    pub fn with_compiled_content(
        mut self,
        content: CompiledEconomyReferenceContentV1,
    ) -> Result<Self, CanwuError> {
        content.validate()?;
        self.compiled_content = Some(content);
        self.validate()?;
        Ok(self)
    }

    pub fn into_initial_record(self) -> Result<DomainRecord, CanwuError> {
        self.validate()?;
        let draft = DomainRecordDraft::from_typed(economy_reference_runtime_reference(), &self)?;
        Ok(DomainRecord {
            reference: draft.reference,
            owner: PLUGIN_NAME.to_owned(),
            class: DomainRecordClass::Record,
            version: self.revision,
            lifecycle: DomainRecordLifecycle::Active,
            payload: draft.payload,
            references: draft.references,
        })
    }

    pub(crate) fn draft(&self) -> Result<DomainRecordDraft, CanwuError> {
        self.validate()?;
        DomainRecordDraft::from_typed(economy_reference_runtime_reference(), self)
    }

    #[allow(clippy::too_many_lines)]
    pub fn validate(&self) -> Result<(), CanwuError> {
        self.limits.validate()?;
        self.completion_run_budget
            .validate()
            .map_err(resource_error)?;
        let reserved = self
            .completion_participants
            .values()
            .filter(|participant| {
                matches!(
                    participant.grant.state,
                    CompletionGrantStateV1::Held
                        | CompletionGrantStateV1::Prepared
                        | CompletionGrantStateV1::Consumed
                )
            })
            .try_fold(0_u64, |total, participant| {
                total
                    .checked_add(participant.grant.reserved_units)
                    .ok_or_else(|| invalid("economy completion reserve overflowed"))
            })?;
        if reserved != self.completion_reserved_units
            || reserved > self.completion_run_budget.total_completion_units
            || self
                .completion_participants
                .iter()
                .any(|(acquisition, participant)| {
                    acquisition != &participant.grant.acquisition
                        || participant.grant.owner_plugin != PLUGIN_NAME
                        || participant.grant.run_budget_revision
                            != self.completion_run_budget.revision
                        || participant
                            .recipe
                            .digest()
                            .map_or(true, |digest| digest != participant.grant.recipe_digest)
                        || participant.eligibility_envelope_digest.is_empty()
                        || (participant.grant.state == CompletionGrantStateV1::Consumed
                            && participant.certificate.is_none())
                })
        {
            return Err(invalid(
                "economy completion participant closure or reserve is invalid",
            ));
        }
        for (target, grant) in &self.completion_target_locks {
            let participant = self
                .completion_participants
                .values()
                .find(|participant| &participant.grant.id == grant)
                .ok_or_else(|| invalid("economy completion target lock is orphaned"))?;
            if !participant.grant.target_versions.contains(target)
                || !matches!(
                    participant.grant.state,
                    CompletionGrantStateV1::Prepared | CompletionGrantStateV1::Consumed
                )
            {
                return Err(invalid("economy completion target lock is invalid"));
            }
        }
        let indexed_expiry = self
            .completion_expiry_due
            .values()
            .flat_map(|values| values.iter().cloned())
            .collect::<BTreeSet<_>>();
        let expected_expiry = self
            .completion_participants
            .values()
            .filter(|participant| {
                matches!(
                    participant.grant.state,
                    CompletionGrantStateV1::Held | CompletionGrantStateV1::Prepared
                )
            })
            .map(|participant| participant.grant.acquisition.clone())
            .collect::<BTreeSet<_>>();
        if indexed_expiry != expected_expiry {
            return Err(invalid(
                "economy completion expiry index differs from participant state",
            ));
        }
        let encoded = serde_json::to_vec(self).map_err(|error| invalid(error.to_string()))?;
        if self.format_version != ECONOMY_FORMAT_VERSION
            || self.revision == 0
            || encoded.len() > self.limits.max_state_bytes
            || self.profiles.len() > self.limits.max_profiles
            || self.local_economies.len() > self.limits.max_local_economies
            || self.route_observations.len() > self.limits.max_route_observations
            || self.price_observations.len() > self.limits.max_price_observations
            || self.delivery_attempts.len() > self.limits.max_delivery_attempts
            || self.outcomes.len() > self.limits.max_operation_outcomes
            || self.observation_grants.len() > self.limits.max_observation_grants
        {
            return Err(invalid(
                "economy-reference state exceeds its V1 bounded limits",
            ));
        }
        let mut archive_head = self.archive_head.clone();
        let recorded_head = std::mem::take(&mut archive_head.semantic_digest);
        if recorded_head
            != canonical_hash("canwu.economy-reference.archive-head.v1", &archive_head)?
            || self.archive_retention_handles.len() > 64
            || self.archive_maintenance_receipts.len() > 8_192
        {
            return Err(invalid(
                "economy archive head or hot maintenance bounds are invalid",
            ));
        }
        for (id, handle) in &self.archive_retention_handles {
            let mut detached = handle.clone();
            let recorded = std::mem::take(&mut detached.semantic_digest);
            if id != &handle.id
                || recorded
                    != canonical_hash("canwu.economy-reference.archive-retention.v1", &detached)?
            {
                return Err(invalid("economy archive retention handle is forged"));
            }
        }
        for (sequence, receipt) in &self.archive_maintenance_receipts {
            let mut detached = receipt.clone();
            let recorded = std::mem::take(&mut detached.semantic_digest);
            if sequence != &receipt.sequence
                || recorded
                    != canonical_hash(
                        "canwu.economy-reference.archive-maintenance-receipt.v1",
                        &detached,
                    )?
            {
                return Err(invalid("economy archive maintenance receipt is forged"));
            }
        }
        for profile in self.profiles.values() {
            profile.validate()?;
            let content = self.compiled_content.as_ref().ok_or_else(|| {
                invalid("economy profiles require exact compiled reference content")
            })?;
            if profile.compiled_content_hash != content.content_hash
                || profile.definition_ids.iter().any(|id| {
                    !content
                        .definitions
                        .keys()
                        .any(|candidate| candidate.as_str() == id)
                })
                || profile.model_card_ids.iter().any(|id| {
                    !content
                        .model_cards
                        .keys()
                        .any(|candidate| candidate.as_str() == id)
                })
                || profile.interpretation_rules.iter().any(|rule| {
                    !content.model_cards.values().any(|card| {
                        card.rule_revisions
                            .iter()
                            .any(|candidate| candidate.as_str() == rule.as_str())
                    })
                })
            {
                return Err(invalid(
                    "economy profile differs from its exact compiled content bindings",
                ));
            }
        }
        let mut economy_scopes = BTreeSet::new();
        for economy in self.local_economies.values() {
            if economy.revision == 0
                || economy.population_wellbeing_per_mille > 1_000
                || economy.cooperation_per_mille > 1_000
                || economy.pending_harvest_penalty_per_mille > 1_000
                || !self.profiles.contains_key(&economy.profile)
            {
                return Err(invalid(
                    "local economy references invalid profile or bounded state",
                ));
            }
            if !economy_scopes.insert(economy.scope.clone()) {
                return Err(invalid(
                    "local economy resource scopes must be unique for exact externality targeting",
                ));
            }
            if self
                .frames
                .get(&economy.id)
                .is_some_and(|frames| frames.len() > self.limits.max_frames_per_economy)
            {
                return Err(invalid("local economy frame history exceeds its hot cap"));
            }
        }
        if self.resilience_postures.iter().any(|(economy, posture)| {
            !self.local_economies.contains_key(economy) || posture.is_empty() || posture.len() > 192
        }) {
            return Err(invalid(
                "economy resilience posture is empty, oversized, or orphaned",
            ));
        }
        if self.resource_consumption_intents.len() > self.limits.max_operation_outcomes {
            return Err(invalid(
                "economy resource consumption intent hot cap was exceeded",
            ));
        }
        for (id, intent) in &self.resource_consumption_intents {
            intent
                .validate()
                .map_err(|error| invalid(error.to_string()))?;
            if id != &intent.id
                || intent.provider_plugin != crate::PLUGIN_NAME
                || intent.status != canwu_resource::ResourceConsumptionIntentStatusV1::Authorized
            {
                return Err(invalid(
                    "economy resource consumption intent is forged or terminal",
                ));
            }
        }
        for heads in self.observation_heads.values() {
            if heads.len() > self.limits.max_observation_heads_per_scope
                || heads
                    .windows(2)
                    .any(|pair| pair[0].observed_at > pair[1].observed_at)
            {
                return Err(invalid(
                    "economy observation heads exceed their cap or are not ordered",
                ));
            }
        }
        for (key, grant_id) in &self.observation_grant_by_holder_scope {
            let grant = self
                .observation_grants
                .get(grant_id)
                .ok_or_else(|| invalid("economy holder/scope grant index is orphaned"))?;
            if !grant.scopes.iter().any(|scope| {
                holder_scope_index_key(&grant.holder, scope).is_ok_and(|expected| &expected == key)
            }) {
                return Err(invalid("economy holder/scope grant index is forged"));
            }
        }
        for grant in self.observation_grants.values() {
            for scope in &grant.scopes {
                if self
                    .observation_grant_by_holder_scope
                    .get(&holder_scope_index_key(&grant.holder, scope)?)
                    != Some(&grant.id)
                {
                    return Err(invalid(
                        "economy observation grant lacks a persistent holder/scope index",
                    ));
                }
            }
        }
        for (key, head) in &self.observation_head_by_holder_scope {
            let grant_id = self
                .observation_grant_by_holder_scope
                .get(key)
                .ok_or_else(|| invalid("economy holder/scope observation head lost its grant"))?;
            let grant = self
                .observation_grants
                .get(grant_id)
                .ok_or_else(|| invalid("economy holder/scope observation grant is orphaned"))?;
            if !grant.scopes.contains(&head.scope)
                || holder_scope_index_key(&grant.holder, &head.scope)? != *key
                || self
                    .observation_heads
                    .get(&head.scope)
                    .is_none_or(|heads| !heads.contains(head))
            {
                return Err(invalid("economy holder/scope observation head is forged"));
            }
        }
        for (holder_scope, observations) in &self.observation_temporal_by_holder_scope {
            if observations.len() > self.limits.max_observation_heads_per_scope {
                return Err(invalid(
                    "economy temporal observation index exceeds its hot cap",
                ));
            }
            for (temporal, head) in observations {
                if temporal.observed_at != head.observed_at
                    || temporal.canonical_digest != head.semantic_digest
                    || self
                        .observation_grant_by_holder_scope
                        .get(holder_scope)
                        .and_then(|grant| self.observation_grants.get(grant))
                        .is_none_or(|grant| !grant.scopes.contains(&head.scope))
                {
                    return Err(invalid("economy temporal observation index is forged"));
                }
            }
        }
        for (route_id, route) in &self.route_observations {
            validate_route_observation(route)?;
            let holder_scope = holder_scope_index_key(&route.holder, &route.target_scope)?;
            let fact_key = route_head_index_key(&route.route_key, &route.source_scope);
            let temporal_key = EconomyFactTemporalKeyV1 {
                observed_at: route.observed_at,
                observation_id: route.id.to_string(),
            };
            if self
                .route_temporal_by_holder_scope
                .get(&holder_scope)
                .and_then(|facts| facts.get(&fact_key))
                .and_then(|history| history.get(&temporal_key))
                != Some(route_id)
            {
                return Err(invalid(
                    "economy route observation lacks its exact temporal index",
                ));
            }
            if self
                .route_heads_by_holder_scope
                .get(&holder_scope_index_key(&route.holder, &route.target_scope)?)
                .and_then(|heads| {
                    heads.get(&route_head_index_key(&route.route_key, &route.source_scope))
                })
                == Some(route_id)
            {
                continue;
            }
            let newer_exists = self.route_observations.values().any(|candidate| {
                candidate.holder == route.holder
                    && candidate.target_scope == route.target_scope
                    && candidate.route_key == route.route_key
                    && candidate.source_scope == route.source_scope
                    && (candidate.observed_at, &candidate.id) > (route.observed_at, &route.id)
            });
            if !newer_exists {
                return Err(invalid(format!(
                    "economy route head index is missing its latest value: {route_id} scope={} target={}",
                    route.source_scope, route.target_scope
                )));
            }
        }
        for (price_id, price) in &self.price_observations {
            validate_price_observation(price, self)?;
            let holder_scope = holder_scope_index_key(&price.holder, &price.scope)?;
            let fact_key = price_head_index_key(price.kind, &price.interpretation_rule_revision);
            let temporal_key = EconomyFactTemporalKeyV1 {
                observed_at: price.observed_at,
                observation_id: price.id.to_string(),
            };
            if self
                .price_temporal_by_holder_scope
                .get(&holder_scope)
                .and_then(|facts| facts.get(&fact_key))
                .and_then(|history| history.get(&temporal_key))
                != Some(price_id)
            {
                return Err(invalid(
                    "economy price observation lacks its exact temporal index",
                ));
            }
            if self
                .price_heads_by_holder_scope
                .get(&holder_scope_index_key(&price.holder, &price.scope)?)
                .and_then(|heads| {
                    heads.get(&price_head_index_key(
                        price.kind,
                        &price.interpretation_rule_revision,
                    ))
                })
                == Some(price_id)
            {
                continue;
            }
            let newer_exists = self.price_observations.values().any(|candidate| {
                candidate.holder == price.holder
                    && candidate.scope == price.scope
                    && candidate.kind == price.kind
                    && candidate.interpretation_rule_revision == price.interpretation_rule_revision
                    && (candidate.observed_at, &candidate.id) > (price.observed_at, &price.id)
            });
            if !newer_exists {
                return Err(invalid(
                    "economy price head index is missing its latest value",
                ));
            }
        }
        for attempt in self.delivery_attempts.values() {
            validate_delivery_attempt(attempt)?;
        }
        if self.externality_outcomes.len() > self.limits.max_operation_outcomes {
            return Err(invalid("economy externality outcome hot cap was exceeded"));
        }
        for (id, outcome) in &self.externality_outcomes {
            let mut detached = outcome.clone();
            let recorded = std::mem::take(&mut detached.semantic_digest);
            if id != &outcome.id
                || outcome.revision == 0
                || recorded
                    != canonical_hash(
                        "canwu.force-supply.economy-externality-outcome.v1",
                        &detached,
                    )?
            {
                return Err(invalid("economy externality outcome history is forged"));
            }
        }
        Ok(())
    }

    pub(crate) fn push_observation_head(
        &mut self,
        holder: &KnowledgeHolderRef,
        head: EconomyObservationHeadV1,
    ) -> Result<(), CanwuError> {
        let values = self
            .observation_heads
            .entry(head.scope.clone())
            .or_default();
        if values.len() >= self.limits.max_observation_heads_per_scope {
            return Err(CanwuError::new(
                ErrorCode::QueryBudgetExceeded,
                "economy observation hot cap requires archive progress",
            ));
        }
        if values
            .last()
            .is_some_and(|latest| latest.observed_at > head.observed_at)
        {
            return Err(invalid("economy observation head time moved backwards"));
        }
        values.push(head.clone());
        let holder_scope = holder_scope_index_key(holder, &head.scope)?;
        let temporal = self
            .observation_temporal_by_holder_scope
            .entry(holder_scope.clone())
            .or_default();
        if temporal.len() >= self.limits.max_observation_heads_per_scope {
            return Err(CanwuError::new(
                ErrorCode::QueryBudgetExceeded,
                "economy temporal observation hot cap requires archive progress",
            ));
        }
        temporal.insert(
            EconomyObservationTemporalKeyV1 {
                observed_at: head.observed_at,
                canonical_digest: head.semantic_digest.clone(),
            },
            head.clone(),
        );
        self.observation_head_by_holder_scope
            .insert(holder_scope, head);
        Ok(())
    }

    pub(crate) fn index_route_observation(
        &mut self,
        observation: &EconomyRouteObservationV1,
    ) -> Result<(), CanwuError> {
        let holder_scope = holder_scope_index_key(&observation.holder, &observation.target_scope)?;
        let fact_key = route_head_index_key(&observation.route_key, &observation.source_scope);
        let history = self
            .route_temporal_by_holder_scope
            .entry(holder_scope)
            .or_default()
            .entry(fact_key)
            .or_default();
        if history.len() >= MAX_OBSERVATION_FACTS {
            return Err(CanwuError::new(
                ErrorCode::QueryBudgetExceeded,
                "economy route temporal hot cap requires archive progress",
            ));
        }
        history.insert(
            EconomyFactTemporalKeyV1 {
                observed_at: observation.observed_at,
                observation_id: observation.id.to_string(),
            },
            observation.id.clone(),
        );
        Ok(())
    }

    pub(crate) fn index_price_observation(
        &mut self,
        observation: &EconomyPriceObservationV1,
    ) -> Result<(), CanwuError> {
        let holder_scope = holder_scope_index_key(&observation.holder, &observation.scope)?;
        let fact_key =
            price_head_index_key(observation.kind, &observation.interpretation_rule_revision);
        let history = self
            .price_temporal_by_holder_scope
            .entry(holder_scope)
            .or_default()
            .entry(fact_key)
            .or_default();
        if history.len() >= MAX_PRICE_FACTORS {
            return Err(CanwuError::new(
                ErrorCode::QueryBudgetExceeded,
                "economy price temporal hot cap requires archive progress",
            ));
        }
        history.insert(
            EconomyFactTemporalKeyV1 {
                observed_at: observation.observed_at,
                observation_id: observation.id.to_string(),
            },
            observation.id.clone(),
        );
        Ok(())
    }

    pub fn economy_archive_source_root(&self) -> Result<String, CanwuError> {
        canonical_hash(
            "canwu.economy-reference.archive-source-root.v1",
            &(
                self.revision,
                &self.resource_consumption_intents,
                &self.frames,
                &self.observation_heads,
                &self.route_observations,
                &self.price_observations,
                &self.delivery_attempts,
                &self.externality_outcomes,
                &self.outcomes,
                &self.archive_head,
            ),
        )
    }

    #[allow(clippy::too_many_lines)]
    pub fn prepare_economy_archive(
        &self,
        candidate_limit: usize,
    ) -> Result<PreparedEconomyArchiveBatchV1, CanwuError> {
        if candidate_limit == 0
            || candidate_limit > canwu_force_supply_reference::MAX_PACKAGE_ARCHIVE_PAGE_ENTRIES
        {
            return Err(invalid("economy archive candidate budget is invalid"));
        }
        let mut records = Vec::new();
        let mut push_record = |key: EconomyArchiveKeyV1,
                               terminal_sequence: u64,
                               payload: EconomyArchivePayloadV1|
         -> Result<(), CanwuError> {
            if records.len() < candidate_limit {
                records.push(canwu_force_supply_reference::PackageArchiveRecordV1 {
                    key,
                    terminal_sequence,
                    semantic_digest: canonical_hash(
                        "canwu.economy-reference.archive-record-payload.v1",
                        &payload,
                    )?,
                    payload,
                });
            }
            Ok(())
        };
        for (economy, frames) in &self.frames {
            for frame in frames {
                push_record(
                    EconomyArchiveKeyV1::MonthlyFrame(economy.clone(), frame.month),
                    archive_sequence(frame.at),
                    EconomyArchivePayloadV1::MonthlyFrame(frame.clone()),
                )?;
            }
        }
        for heads in self.observation_heads.values() {
            for head in heads.iter().take(heads.len().saturating_sub(1)) {
                let manager = &self
                    .local_economies
                    .get(&head.economy)
                    .ok_or_else(|| invalid("economy archive observation lost its economy"))?
                    .manager;
                let holder_scope = holder_scope_index_key(manager, &head.scope)?;
                push_record(
                    EconomyArchiveKeyV1::ObservationHead {
                        holder_scope,
                        observed_at: head.observed_at,
                        digest: head.semantic_digest.clone(),
                    },
                    archive_sequence(head.observed_at),
                    EconomyArchivePayloadV1::ObservationHead(head.clone()),
                )?;
            }
        }
        let route_heads = self
            .route_heads_by_holder_scope
            .values()
            .flat_map(BTreeMap::values)
            .cloned()
            .collect::<BTreeSet<_>>();
        for observation in self.route_observations.values() {
            if !route_heads.contains(&observation.id) {
                push_record(
                    EconomyArchiveKeyV1::RouteObservation(observation.id.clone()),
                    archive_sequence(observation.observed_at),
                    EconomyArchivePayloadV1::RouteObservation(observation.clone()),
                )?;
            }
        }
        let price_heads = self
            .price_heads_by_holder_scope
            .values()
            .flat_map(BTreeMap::values)
            .cloned()
            .collect::<BTreeSet<_>>();
        for observation in self.price_observations.values() {
            if !price_heads.contains(&observation.id) {
                push_record(
                    EconomyArchiveKeyV1::PriceObservation(observation.id.clone()),
                    archive_sequence(observation.observed_at),
                    EconomyArchivePayloadV1::PriceObservation(observation.clone()),
                )?;
            }
        }
        for attempt in self
            .delivery_attempts
            .values()
            .filter(|attempt| attempt.disposition != crate::DeliveryDispositionV1::Pending)
        {
            push_record(
                EconomyArchiveKeyV1::DeliveryAttempt(attempt.id.clone()),
                archive_sequence(attempt.recorded_at),
                EconomyArchivePayloadV1::DeliveryAttempt(attempt.clone()),
            )?;
        }
        for outcome in self.externality_outcomes.values() {
            push_record(
                EconomyArchiveKeyV1::ExternalityOutcome(outcome.id.clone()),
                outcome.revision,
                EconomyArchivePayloadV1::ExternalityOutcome(outcome.clone()),
            )?;
        }
        for outcome in self.outcomes.values() {
            push_record(
                EconomyArchiveKeyV1::OperationOutcome(outcome.id.clone()),
                archive_sequence(outcome.settled_at),
                EconomyArchivePayloadV1::OperationOutcome(outcome.clone()),
            )?;
        }
        canwu_force_supply_reference::prepare_package_archive(
            ECONOMY_ARCHIVE_DOMAIN,
            self.economy_archive_source_root()?,
            &self.archive_head,
            records,
        )
    }

    #[allow(clippy::too_many_lines)]
    pub(crate) fn apply_economy_archive_commit(
        &mut self,
        commit: &VerifiedEconomyArchiveCommitV1,
    ) -> Result<EconomyArchiveMaintenanceReceiptV1, CanwuError> {
        if commit.retention.phase
            != canwu_force_supply_reference::PackageArchiveRetentionPhaseV1::Verified
            || commit.retention.directory_root != commit.directory_root
            || commit.retention.expected_source_root != commit.expected_source_root
            || usize::try_from(commit.archived_records).ok() != Some(commit.selected.len())
        {
            return Err(invalid("economy archive commit is forged"));
        }
        let source_matches = self.economy_archive_source_root()? == commit.expected_source_root;
        let disposition = if source_matches {
            for key in &commit.selected {
                match key {
                    EconomyArchiveKeyV1::MonthlyFrame(economy, month) => {
                        let frames = self
                            .frames
                            .get_mut(economy)
                            .ok_or_else(|| invalid("economy archive source disappeared"))?;
                        let position = frames
                            .iter()
                            .position(|frame| frame.month == *month)
                            .ok_or_else(|| invalid("economy archive frame disappeared"))?;
                        frames.remove(position);
                    }
                    EconomyArchiveKeyV1::ObservationHead {
                        holder_scope,
                        observed_at,
                        digest,
                    } => {
                        let head = self
                            .observation_heads
                            .values_mut()
                            .flat_map(|heads| heads.iter_mut())
                            .find(|head| {
                                head.observed_at == *observed_at
                                    && head.semantic_digest == *digest
                                    && self
                                        .local_economies
                                        .get(&head.economy)
                                        .and_then(|economy| {
                                            holder_scope_index_key(&economy.manager, &head.scope)
                                                .ok()
                                        })
                                        .as_ref()
                                        == Some(holder_scope)
                            })
                            .cloned()
                            .ok_or_else(|| invalid("economy archive observation disappeared"))?;
                        self.observation_heads
                            .get_mut(&head.scope)
                            .expect("observation scope was found")
                            .retain(|candidate| candidate != &head);
                    }
                    EconomyArchiveKeyV1::RouteObservation(id) => {
                        self.route_observations.remove(id).ok_or_else(|| {
                            invalid("economy archive route observation disappeared")
                        })?;
                    }
                    EconomyArchiveKeyV1::PriceObservation(id) => {
                        self.price_observations.remove(id).ok_or_else(|| {
                            invalid("economy archive price observation disappeared")
                        })?;
                    }
                    EconomyArchiveKeyV1::DeliveryAttempt(id) => {
                        self.delivery_attempts.remove(id).ok_or_else(|| {
                            invalid("economy archive delivery attempt disappeared")
                        })?;
                    }
                    EconomyArchiveKeyV1::ExternalityOutcome(id) => {
                        self.externality_outcomes.remove(id).ok_or_else(|| {
                            invalid("economy archive externality outcome disappeared")
                        })?;
                    }
                    EconomyArchiveKeyV1::OperationOutcome(id) => {
                        self.outcomes.remove(id).ok_or_else(|| {
                            invalid("economy archive operation outcome disappeared")
                        })?;
                    }
                }
            }
            self.frames.retain(|_, frames| !frames.is_empty());
            self.observation_heads.retain(|_, heads| !heads.is_empty());
            self.rebuild_derived_indexes()?;
            self.archive_head = canwu_force_supply_reference::sealed_archive_head(
                ECONOMY_ARCHIVE_DOMAIN,
                EconomyArchiveHeadStateV1 {
                    revision: self
                        .archive_head
                        .revision
                        .checked_add(1)
                        .ok_or_else(|| invalid("economy archive revision overflow"))?,
                    directory_root: Some(commit.directory_root.clone()),
                    archived_record_count: self
                        .archive_head
                        .archived_record_count
                        .checked_add(u64::from(commit.archived_records))
                        .ok_or_else(|| invalid("economy archive count overflow"))?,
                    semantic_digest: String::new(),
                },
            )?;
            canwu_force_supply_reference::PackageArchiveMaintenanceDispositionV1::Applied
        } else {
            canwu_force_supply_reference::PackageArchiveMaintenanceDispositionV1::RejectedStale
        };
        let sequence = self
            .archive_maintenance_receipts
            .keys()
            .next_back()
            .copied()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| invalid("economy archive receipt overflow"))?;
        let mut receipt = EconomyArchiveMaintenanceReceiptV1 {
            sequence,
            retention_handle_id: commit.retention.id.clone(),
            expected_source_root: commit.expected_source_root.clone(),
            directory_root: commit.directory_root.clone(),
            disposition,
            archived_records: if source_matches {
                commit.archived_records
            } else {
                0
            },
            semantic_digest: String::new(),
        };
        receipt.semantic_digest = canonical_hash(
            "canwu.economy-reference.archive-maintenance-receipt.v1",
            &receipt,
        )?;
        let mut durable = commit.retention.clone();
        durable.phase =
            canwu_force_supply_reference::PackageArchiveRetentionPhaseV1::DurableIngress;
        durable.semantic_digest.clear();
        durable.semantic_digest =
            canonical_hash("canwu.economy-reference.archive-retention.v1", &durable)?;
        self.archive_retention_handles
            .insert(durable.id.clone(), durable);
        self.archive_maintenance_receipts
            .insert(sequence, receipt.clone());
        Ok(receipt)
    }
}

fn archive_sequence(at: SimTime) -> u64 {
    u64::try_from(at.as_minutes()).unwrap_or_default()
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod archive_tests {
    use super::*;
    use canwu_api::DomainRecordVersionSource;
    use canwu_force_supply_reference::{
        PackageArchiveRetentionHandleV1, PackageArchiveRetentionPhaseV1, PackageArchiveStore,
    };
    use std::cell::RefCell;

    #[derive(Default)]
    struct Store {
        objects: RefCell<BTreeMap<(String, String), Vec<u8>>>,
        handles: RefCell<BTreeMap<String, PackageArchiveRetentionHandleV1>>,
    }

    impl PackageArchiveStore for Store {
        fn store_package_archive_object(
            &self,
            namespace: &str,
            object_id: &str,
            bytes: &[u8],
        ) -> Result<(), CanwuError> {
            self.objects
                .borrow_mut()
                .insert((namespace.to_owned(), object_id.to_owned()), bytes.to_vec());
            Ok(())
        }

        fn load_package_archive_object(
            &self,
            namespace: &str,
            object_id: &str,
        ) -> Result<Option<Vec<u8>>, CanwuError> {
            Ok(self
                .objects
                .borrow()
                .get(&(namespace.to_owned(), object_id.to_owned()))
                .cloned())
        }

        fn persist_package_archive_retention(
            &self,
            handle: &PackageArchiveRetentionHandleV1,
        ) -> Result<(), CanwuError> {
            self.handles
                .borrow_mut()
                .insert(handle.id.clone(), handle.clone());
            Ok(())
        }

        fn load_package_archive_retention(
            &self,
            handle_id: &str,
        ) -> Result<Option<PackageArchiveRetentionHandleV1>, CanwuError> {
            Ok(self.handles.borrow().get(handle_id).cloned())
        }

        fn finalize_package_archive_retention(
            &self,
            handle: &PackageArchiveRetentionHandleV1,
        ) -> Result<(), CanwuError> {
            self.persist_package_archive_retention(handle)
        }
    }

    #[test]
    fn archive_retires_externality_and_operation_outcome_hot_history() {
        let mut state = EconomyReferenceStateV1::default();
        let externality_id =
            ExternalityOutcomeId::new("canwu.economy-reference:externality-outcome:archive-test")
                .expect("externality id");
        let mut externality = EconomyExternalityOutcomeVersionV1 {
            id: externality_id.clone(),
            revision: 1,
            intent: canwu_force_supply_reference::ForceExternalityIntentId::new(
                "canwu.force-supply-reference:externality-intent:archive-test",
            )
            .expect("intent id"),
            disposition: canwu_force_supply_reference::ExternalityOutcomeDisposition::Applied,
            expected_target: DomainRecordVersionRef {
                record: economy_reference_runtime_reference().into_untyped(),
                version: 1,
                established_by: DomainRecordVersionSource::InitialScenario,
            },
            resulting_target_revision: Some(2),
            blocker: None,
            semantic_digest: String::new(),
        };
        externality.semantic_digest = canonical_hash(
            "canwu.force-supply.economy-externality-outcome.v1",
            &externality,
        )
        .expect("externality digest");
        state
            .externality_outcomes
            .insert(externality_id.clone(), externality);
        let operation_id =
            EconomyOperationId::new("canwu.economy-reference:operation:archive-test")
                .expect("operation id");
        state.outcomes.insert(
            operation_id.clone(),
            EconomyOperationOutcomeV1 {
                id: operation_id.clone(),
                input_digest: "a".repeat(64),
                applied: true,
                rejection_code: None,
                rejection_reason: None,
                settled_at: SimTime::EPOCH,
            },
        );
        state.validate().expect("valid hot history");
        let prepared = state.prepare_economy_archive(8).expect("prepare archive");
        assert!(
            prepared
                .selected
                .contains(&EconomyArchiveKeyV1::ExternalityOutcome(
                    externality_id.clone()
                ))
        );
        assert!(
            prepared
                .selected
                .contains(&EconomyArchiveKeyV1::OperationOutcome(operation_id.clone()))
        );
        let commit = prepared
            .store_and_verify(ECONOMY_ARCHIVE_DOMAIN, &Store::default())
            .expect("verify archive");
        state
            .apply_economy_archive_commit(&commit)
            .expect("commit archive");
        assert!(!state.externality_outcomes.contains_key(&externality_id));
        assert!(!state.outcomes.contains_key(&operation_id));
        assert_eq!(state.archive_head.archived_record_count, 2);
        assert_eq!(
            commit.retention.phase,
            PackageArchiveRetentionPhaseV1::Verified
        );
    }
}

pub(crate) fn holder_scope_index_key(
    holder: &KnowledgeHolderRef,
    scope: &ResourceScopeId,
) -> Result<String, CanwuError> {
    canonical_hash("canwu.economy.holder-scope-index.v1", &(holder, scope))
}

pub(crate) fn price_head_index_key(
    kind: crate::PriceEvidenceKind,
    rule: &EconomyRuleRevisionId,
) -> String {
    format!("{kind:?}:{}", rule.as_str())
}

pub(crate) fn route_head_index_key(route_key: &str, scope: &ResourceScopeId) -> String {
    format!("{route_key}:{}", scope.as_str())
}

pub(crate) fn validate_route_observation(
    observation: &EconomyRouteObservationV1,
) -> Result<(), CanwuError> {
    let mut detached = observation.clone();
    detached.semantic_digest.clear();
    if observation.route_key.trim().is_empty()
        || observation.confidence_per_mille > 1_000
        || observation.target_scope == observation.source_scope
        || observation.semantic_digest
            != canonical_hash("canwu.economy.route-observation.v1", &detached)?
    {
        return Err(invalid(
            "economy route observation is not canonically sealed",
        ));
    }
    Ok(())
}

pub(crate) fn validate_price_observation(
    observation: &EconomyPriceObservationV1,
    state: &EconomyReferenceStateV1,
) -> Result<(), CanwuError> {
    let mut detached = observation.clone();
    detached.semantic_digest.clear();
    let applicable_rule = state
        .local_economies
        .values()
        .filter(|economy| economy.scope == observation.scope)
        .filter_map(|economy| state.profiles.get(&economy.profile))
        .any(|profile| {
            profile.price_applicability == PriceEvidenceApplicabilityV1::Applicable
                && profile
                    .interpretation_rules
                    .contains(&observation.interpretation_rule_revision)
        });
    if observation.baseline_scaled <= 0
        || observation.scale == 0
        || observation.effective_until <= observation.effective_from
        || observation.observed_at < observation.effective_from
        || observation.observed_at >= observation.effective_until
        || observation.confidence_per_mille > 1_000
        || observation.source_versions.is_empty()
        || !applicable_rule
        || observation.semantic_digest
            != canonical_hash("canwu.economy.price-observation.v1", &detached)?
    {
        return Err(invalid(
            "economy price observation is inapplicable or not canonically sealed",
        ));
    }
    Ok(())
}

pub(crate) fn validate_delivery_attempt(
    attempt: &EconomyDeliveryAttemptV1,
) -> Result<(), CanwuError> {
    let mut detached = attempt.clone();
    detached.semantic_digest.clear();
    if attempt.source_scope == attempt.target_scope
        || attempt.execution.revisions.is_empty()
        || attempt.semantic_digest
            != canonical_hash("canwu.economy.delivery-attempt.v1", &detached)?
    {
        return Err(invalid(
            "economy delivery attempt lacks an itinerary or exact canonical seal",
        ));
    }
    Ok(())
}
