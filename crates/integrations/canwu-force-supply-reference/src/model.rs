use crate::{PLUGIN_NAME, PLUGIN_NAMESPACE};
use canwu_api::{
    CanwuError, DomainRecord, DomainRecordClass, DomainRecordDraft, DomainRecordLifecycle,
    DomainRecordType, DomainRecordVersionRef, DomainValueKindClass, ErrorCode, KnowledgeHolderRef,
    SimTime, TypedDomainRecordRef, canonical_hash,
};
use canwu_economy_reference_content::{
    CompiledEconomyReferenceContentV1, CoverageKeyV1, DefinitionId, ExternalityApplicability,
    ModelCardId, RuleRevisionId,
};
use canwu_resource::{
    AbortCompletionLeaseV1, ActivateCompletionLeaseV1, CompletionCapacityPartitionV1,
    CompletionGrantStateV1, CompletionLeaseAcquisitionId, CompletionLeaseAcquisitionStateV1,
    CompletionLeaseAcquisitionV1, CompletionLeaseActivationCertificateV1, CompletionLeaseBookV1,
    CompletionLeaseReceiptActionV1, CompletionLeaseStatusDtoV1, CompletionLockedTargetV1,
    ExpireCompletionCapacityV1, ExternalCompletionParticipantGrantV1, GrantCompletionCapacityV1,
    PrepareCompletionCapacityV1, ReleaseCompletionCapacityV1, RequestCompletionLeaseV1,
    ResourceAccountId, ResourceAllocationLegVersionV1, ResourceConsumptionId,
    ResourceConsumptionRequestV1, ResourceDefinitionRevisionId, ResourceError,
    ResourceOperationKey, ResourceOperationOutcomeId, ResourceOperationOutcomeVersionV1,
    ResourceOperationStatus, ResourceRecordRefV1, ResourceRevision, ResourceUnitRevisionId,
    RunBudgetRevisionV1,
};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Display, Formatter};

pub const FORCE_RUNTIME_ID: &str = "canwu.force-supply-reference:runtime:v1";
pub const ECONOMY_EXTERNALITY_PROVIDER: &str = "canwu-economy-reference";
pub const MAX_IDENTIFIER_BYTES: usize = 192;
pub const MAX_REQUIREMENTS_PER_PROFILE: usize = 64;
pub const MAX_REPORT_FACTS: usize = 256;
pub const MAX_DUE_CANDIDATES_PER_TICK: usize = 2_048;
pub const MAX_TEMPORAL_HEADS_PER_SCOPE: usize = 512;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ForceSupplyReferenceLimitsV1 {
    pub max_forces: usize,
    pub max_profiles: usize,
    pub max_active_intents: usize,
    pub max_active_sagas: usize,
    pub max_consequences: usize,
    pub max_externality_outcomes: usize,
    pub max_terminal_receipts: usize,
    pub max_observer_grants: usize,
    pub max_reports_per_holder: usize,
    pub max_state_bytes: usize,
}

impl ForceSupplyReferenceLimitsV1 {
    pub const DEFAULT_FORCES: usize = 256 * 8;
    pub const HARD_FORCES: usize = 1_024 * 64;
    pub const DEFAULT_ACTIVE: usize = 1_024 * 8;
    pub const HARD_ACTIVE: usize = 4_096 * 64;
    pub const MAX_CONSUMPTION_INTENTS: usize = 2_048;
    pub const MAX_OUTCOMES: usize = 2_048;
    pub const MAX_KNOWLEDGE_RECORDS: usize = 4_096;
    pub const MAX_PER_HOLDER: usize = 512;
    pub const MAX_STATE_BYTES: usize = 64 * 1024 * 1024;

    #[must_use]
    pub const fn canonical() -> Self {
        Self {
            max_forces: Self::DEFAULT_FORCES,
            max_profiles: 2_048,
            max_active_intents: Self::MAX_CONSUMPTION_INTENTS,
            max_active_sagas: Self::DEFAULT_ACTIVE,
            max_consequences: Self::MAX_OUTCOMES,
            max_externality_outcomes: Self::MAX_OUTCOMES,
            max_terminal_receipts: Self::MAX_CONSUMPTION_INTENTS * 4,
            max_observer_grants: Self::MAX_KNOWLEDGE_RECORDS,
            max_reports_per_holder: Self::MAX_PER_HOLDER,
            max_state_bytes: Self::MAX_STATE_BYTES,
        }
    }

    pub fn validate(self) -> Result<(), CanwuError> {
        if self.max_forces == 0
            || self.max_forces > Self::HARD_FORCES
            || self.max_profiles == 0
            || self.max_profiles > 2_048
            || self.max_active_intents == 0
            || self.max_active_intents > Self::HARD_ACTIVE
            || self.max_active_sagas == 0
            || self.max_active_sagas > Self::HARD_ACTIVE
            || self.max_consequences == 0
            || self.max_consequences > Self::MAX_OUTCOMES
            || self.max_externality_outcomes == 0
            || self.max_externality_outcomes > Self::MAX_OUTCOMES
            || self.max_terminal_receipts == 0
            || self.max_terminal_receipts > self.max_active_intents.saturating_mul(4)
            || self.max_observer_grants == 0
            || self.max_observer_grants > Self::MAX_KNOWLEDGE_RECORDS
            || self.max_reports_per_holder == 0
            || self.max_reports_per_holder > Self::MAX_PER_HOLDER
            || self.max_state_bytes == 0
            || self.max_state_bytes > Self::MAX_STATE_BYTES
        {
            return Err(invalid("force-supply limits exceed the V1 hard maxima"));
        }
        Ok(())
    }
}

impl Default for ForceSupplyReferenceLimitsV1 {
    fn default() -> Self {
        Self::canonical()
    }
}

fn validate_identifier(value: &str, label: &str) -> Result<(), CanwuError> {
    if value.is_empty()
        || value.len() > MAX_IDENTIFIER_BYTES
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

typed_id!(ReferenceForceId, "force");
typed_id!(ForceSupplyProfileId, "force-supply profile");
typed_id!(ForceRequirementId, "force requirement");
typed_id!(ForceConsumptionIntentId, "force consumption intent");
typed_id!(ForceConsequenceId, "force consequence");
typed_id!(ForceExternalityIntentId, "force externality intent");
typed_id!(RequisitionSagaId, "requisition saga");
typed_id!(ForceObserverGrantId, "force observer grant");
typed_id!(ForceOperationId, "force operation");
typed_id!(ForceReportId, "force report");
typed_id!(ForceKnowledgePublicationId, "force knowledge publication");
typed_id!(ExternalityOutcomeId, "economy externality outcome");
typed_id!(RequisitionPolicyId, "requisition policy");

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SupplyResourceKind {
    Food,
    Fodder,
    PhysicalCurrency,
    Ammunition,
    Spares,
    Fuel,
    Other,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "cadence", rename_all = "snake_case")]
pub enum ForceSupplyCadenceV1 {
    FixedMinutes { interval_minutes: u64 },
    EventDriven,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ShortageConsequenceRuleV1 {
    pub rule_revision: RuleRevisionId,
    pub tolerance_quantity: u64,
    pub readiness_delta_per_mille: i16,
    pub fatigue_delta_per_mille: i16,
    pub cohesion_delta_per_mille: i16,
    pub disease_delta_per_mille: i16,
    pub desertion_delta_per_mille: i16,
    pub nonlinear_or_threshold: bool,
    pub model_card: ModelCardId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ForceSupplyRequirementV1 {
    pub id: ForceRequirementId,
    pub kind: SupplyResourceKind,
    pub resource_revision: ResourceDefinitionRevisionId,
    pub unit_revision: ResourceUnitRevisionId,
    pub quantity_per_due: u64,
    pub buffer_quantity: u64,
    pub cadence: ForceSupplyCadenceV1,
    pub consequence: ShortageConsequenceRuleV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ForceAcceptedTransferEvidenceV1 {
    pub transfer: canwu_resource::ResourceTransferId,
    pub transfer_revision: ResourceRevision,
    pub destination: ResourceAccountId,
    pub accepted_quantity: u64,
    pub transport: canwu_resource::TransportExecutionLink,
    pub acceptance_source: DomainRecordVersionRef,
    pub semantic_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ForceStockCustodyBindingV1 {
    pub destination_account: ResourceAccountId,
    pub destination_custodian: KnowledgeHolderRef,
    pub accepted_transfer: Option<ForceAcceptedTransferEvidenceV1>,
    pub semantic_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ForceSupplyProfileV1 {
    pub id: ForceSupplyProfileId,
    pub revision: u64,
    pub effective_from: SimTime,
    pub effective_until: Option<SimTime>,
    pub organization_class: String,
    pub requirements: Vec<ForceSupplyRequirementV1>,
    #[serde(default)]
    pub requirement_coverage: BTreeMap<ForceRequirementId, CoverageKeyV1>,
    #[serde(default)]
    pub requirement_resolution_digests: BTreeMap<ForceRequirementId, String>,
    pub coverage_key: CoverageKeyV1,
    pub content_hash: String,
    pub coverage_resolution_digest: String,
    pub definition_ids: BTreeSet<DefinitionId>,
    pub model_cards: BTreeSet<ModelCardId>,
    pub semantic_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RequisitionPolicyV1 {
    pub id: RequisitionPolicyId,
    pub revision: u64,
    pub applicability: ExternalityApplicability,
    pub cooperation_delta_per_mille: i16,
    pub harvest_input_delta_per_mille: i16,
    pub rule_revision: RuleRevisionId,
    pub model_card: ModelCardId,
    pub coverage_key: CoverageKeyV1,
    pub content_hash: String,
    pub coverage_resolution_digest: String,
    pub definition_ids: BTreeSet<DefinitionId>,
    pub semantic_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DueRequirementStateV1 {
    pub requirement: ForceRequirementId,
    pub next_due: SimTime,
    pub persisted_remainder_minutes: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReferenceForce {
    pub id: ReferenceForceId,
    pub revision: u64,
    pub holder: KnowledgeHolderRef,
    pub profile: ForceSupplyProfileId,
    pub active: bool,
    pub readiness_per_mille: u16,
    pub fatigue_per_mille: u16,
    pub cohesion_per_mille: u16,
    pub disease_per_mille: u16,
    pub desertion_per_mille: u16,
    pub supply_posture: String,
    pub due: BTreeMap<ForceRequirementId, DueRequirementStateV1>,
    pub blocked_by_active_requisition: Option<RequisitionSagaId>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ForceConsumptionIntentStatus {
    PendingResourceConsumption,
    ConsequenceCommitted,
    Settled,
    Rejected,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ForceConsumptionIntent {
    pub id: ForceConsumptionIntentId,
    pub revision: u64,
    pub force: ReferenceForceId,
    pub expected_force_runtime_revision: u64,
    pub expected_force_revision: u64,
    pub requirement: ForceRequirementId,
    pub scheduled_due: SimTime,
    pub due_at: SimTime,
    pub due_count: u16,
    pub requested_quantity: u64,
    pub allocation: ResourceAllocationLegVersionV1,
    pub stock_custody: ForceStockCustodyBindingV1,
    pub resource_operation_key: ResourceOperationKey,
    pub consumption_id: ResourceConsumptionId,
    pub requisition_policy: Option<RequisitionPolicyId>,
    pub completion_certificate: CompletionLeaseActivationCertificateV1,
    pub status: ForceConsumptionIntentStatus,
    pub resource_outcome: Option<ResourceOperationOutcomeVersionV1>,
    pub resource_outcome_source: Option<DomainRecordVersionRef>,
    pub consequence: Option<ForceConsequenceId>,
    pub semantic_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ShortageAttributionV1 {
    pub requirement: ForceRequirementId,
    pub kind: SupplyResourceKind,
    pub requested: u64,
    pub consumed: u64,
    pub shortage: u64,
    pub resource_outcome: ResourceOperationOutcomeVersionV1,
    pub consumption: canwu_resource::ResourceConsumptionVersionV1,
    pub fulfillment: canwu_resource::ResourceFulfillmentVersionV1,
    pub stock_custody: ForceStockCustodyBindingV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ForceConsequenceRecord {
    pub id: ForceConsequenceId,
    pub force: ReferenceForceId,
    pub force_revision_before: u64,
    pub force_revision_after: u64,
    pub intent: ForceConsumptionIntentId,
    pub attribution: ShortageAttributionV1,
    pub readiness_delta_per_mille: i16,
    pub fatigue_delta_per_mille: i16,
    pub cohesion_delta_per_mille: i16,
    pub disease_delta_per_mille: i16,
    pub desertion_delta_per_mille: i16,
    pub committed_at: SimTime,
    pub semantic_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ForceExternalityIntent {
    pub id: ForceExternalityIntentId,
    pub saga: RequisitionSagaId,
    pub operation_key: ResourceOperationKey,
    pub force_consequence: ForceConsequenceId,
    pub resource_outcome: ResourceOperationOutcomeVersionV1,
    pub expected_economy_target: DomainRecordVersionRef,
    pub cooperation_delta_per_mille: i16,
    pub harvest_input_delta_per_mille: i16,
    pub quantity: u64,
    pub policy: RequisitionPolicyId,
    pub semantic_digest: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalityOutcomeDisposition {
    Applied,
    Rejected,
    NotApplicable,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EconomyExternalityOutcomeVersionV1 {
    pub id: ExternalityOutcomeId,
    pub revision: u64,
    pub intent: ForceExternalityIntentId,
    pub disposition: ExternalityOutcomeDisposition,
    pub expected_target: DomainRecordVersionRef,
    pub resulting_target_revision: Option<u64>,
    pub blocker: Option<String>,
    pub semantic_digest: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RequisitionSagaStage {
    PendingResourceConsumption,
    ForceConsequenceCommitted,
    ExternalityPending,
    ExternalityApplied,
    ExternalityRejected,
    Settled,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RequisitionSagaV1 {
    pub id: RequisitionSagaId,
    pub force: ReferenceForceId,
    pub intent: ForceConsumptionIntentId,
    pub stage: RequisitionSagaStage,
    pub consequence: Option<ForceConsequenceId>,
    pub externality_intent: Option<ForceExternalityIntentId>,
    pub externality_outcome: Option<EconomyExternalityOutcomeVersionV1>,
    pub externality_outcome_source: Option<DomainRecordVersionRef>,
    pub recoverable_blocker: Option<String>,
    pub final_ack_digest: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ForceTerminalReceiptV1 {
    pub sequence: u64,
    pub intent: ForceConsumptionIntentId,
    pub saga: Option<RequisitionSagaId>,
    pub resource_outcome: ResourceOperationOutcomeVersionV1,
    pub resource_outcome_source: DomainRecordVersionRef,
    pub externality_outcome: Option<EconomyExternalityOutcomeVersionV1>,
    pub externality_outcome_source: Option<DomainRecordVersionRef>,
    pub consequence: ForceConsequenceRecord,
    pub completion_certificate: CompletionLeaseActivationCertificateV1,
    pub final_ack_digest: Option<String>,
    pub terminal_at: SimTime,
    pub semantic_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ForceTerminalContinuationV1 {
    pub through_sequence: u64,
    pub compacted_receipts: u64,
    pub chain_digest: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ForceObservationRole {
    Commander,
    WarehouseCustodian,
    RemoteCommander,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ForceSupplyObservationSourceV1 {
    ResourceProvider,
    TransportProvider,
    ForceConsequence,
    EconomyExternality,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ForceObserverGrantV1 {
    pub id: ForceObserverGrantId,
    pub holder: KnowledgeHolderRef,
    pub force: ReferenceForceId,
    pub role: ForceObservationRole,
    pub observation_delay_minutes: u64,
    pub confidence_per_mille: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ForceSupplyObservationV1 {
    pub requirement: ForceRequirementId,
    pub known_stock_low: u64,
    pub known_stock_high: u64,
    pub demand_forecast: u64,
    pub arrival_state: String,
    pub source: ForceSupplyObservationSourceV1,
    pub observed_at: SimTime,
    pub confidence_per_mille: u16,
    pub source_versions: Vec<DomainRecordVersionRef>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ForceObservationTemporalKeyV1 {
    pub observed_at: SimTime,
    pub provider_revision: u64,
    pub publication: ForceKnowledgePublicationId,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ForceObservationScopeV1 {
    pub holder: KnowledgeHolderRef,
    pub force: ReferenceForceId,
    pub fact_key: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "fact", content = "payload", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum ForceKnowledgeFactV1 {
    SupplyObservation(ForceSupplyObservationV1),
    ShortageAttribution(ShortageAttributionV1),
    RequisitionProgress {
        stage: RequisitionSagaStage,
        latest_outcome_or_ack: Option<String>,
        recoverable_blocker: Option<String>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ForceKnowledgePublicationV1 {
    pub id: ForceKnowledgePublicationId,
    pub grant: ForceObserverGrantId,
    pub holder: KnowledgeHolderRef,
    pub force: ReferenceForceId,
    pub observed_at: SimTime,
    pub available_at: SimTime,
    pub provider_revision: u64,
    pub source_versions: Vec<DomainRecordVersionRef>,
    pub fact: ForceKnowledgeFactV1,
    pub semantic_digest: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ForceSupplyDecisionChoiceV1 {
    WaitForSupply,
    AdvanceImmediately,
    RequisitionLocally,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ForceSupplyDecisionTicketV1 {
    pub holder: KnowledgeHolderRef,
    pub force: ReferenceForceId,
    pub force_revision: u64,
    pub options: Vec<ForceSupplyDecisionChoiceV1>,
    pub holder_facts_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ForceDecisionSelectionV1 {
    pub ticket: canwu_api::DecisionTicketId,
    pub option_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ForceOperationOutcomeV1 {
    pub id: ForceOperationId,
    pub input_digest: String,
    pub applied: bool,
    pub rejection_code: Option<String>,
    pub rejection_reason: Option<String>,
    pub settled_at: SimTime,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "operation", content = "payload", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum ForceOperationV1 {
    RegisterForce {
        force: ReferenceForce,
    },
    SelectSupplyPosture {
        force: ReferenceForceId,
        posture: String,
        decision: ForceDecisionSelectionV1,
    },
    GrantObservation {
        grant: ForceObserverGrantV1,
    },
    RecordSupplyObservation {
        force: ReferenceForceId,
        observation: ForceSupplyObservationV1,
    },
    SubmitConsumptionIntent {
        intent: ForceConsumptionIntent,
    },
    Completion {
        operation: ForceCompletionOperationV1,
    },
    FinalizeRequisition {
        saga: RequisitionSagaId,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "operation", content = "payload", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum ForceCompletionOperationV1 {
    Acquire(RequestCompletionLeaseV1),
    Grant(GrantCompletionCapacityV1),
    Prepare(PrepareCompletionCapacityV1),
    AcknowledgeExternalParticipant {
        owner_source: DomainRecordVersionRef,
        participant: ExternalCompletionParticipantGrantV1,
    },
    Activate(ActivateCompletionLeaseV1),
    Abort(AbortCompletionLeaseV1),
    Release(ReleaseCompletionCapacityV1),
    Expire(ExpireCompletionCapacityV1),
    ConsumeParticipant {
        acquisition: CompletionLeaseAcquisitionId,
        owner_plugin: String,
    },
    Complete {
        acquisition: CompletionLeaseAcquisitionId,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ForceCommandEnvelopeV1 {
    pub operation_id: ForceOperationId,
    pub holder: KnowledgeHolderRef,
    pub expected_runtime_revision: u64,
    pub operation: ForceOperationV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceOutcomePacketV1 {
    pub intent: ForceConsumptionIntentId,
    pub authoritative_resource_state: DomainRecordVersionRef,
    pub outcome_id: ResourceOperationOutcomeId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ForceResourceSettlementEvidenceV1 {
    pub provider_state: DomainRecordVersionRef,
    pub outcome: ResourceOperationOutcomeVersionV1,
    pub consumption: canwu_resource::ResourceConsumptionVersionV1,
    pub fulfillment: canwu_resource::ResourceFulfillmentVersionV1,
    pub destination_account_revision: ResourceRevision,
    pub destination_custodian: KnowledgeHolderRef,
    pub accepted_transfer: Option<ForceAcceptedTransferEvidenceV1>,
    pub semantic_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExternalityOutcomePacketV1 {
    pub saga: RequisitionSagaId,
    pub authoritative_outcome: DomainRecordVersionRef,
    pub authoritative_participant: DomainRecordVersionRef,
}

#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EconomyExternalityOutcomeProviderRecord;

impl DomainRecordType for EconomyExternalityOutcomeProviderRecord {
    type Payload = EconomyExternalityOutcomeVersionV1;
    type Class = DomainValueKindClass;
    const NAMESPACE: &'static str = "canwu.economy-reference";
    const NAME: &'static str = "force-externality-outcome";
}

#[must_use]
pub fn economy_externality_outcome_reference(
    id: &ExternalityOutcomeId,
) -> TypedDomainRecordRef<EconomyExternalityOutcomeProviderRecord> {
    TypedDomainRecordRef::new(format!(
        "canwu.economy-reference:force-externality-outcome:{id}"
    ))
}

#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ForceExternalityOutcomeProviderRecord;

impl DomainRecordType for ForceExternalityOutcomeProviderRecord {
    type Payload = EconomyExternalityOutcomeVersionV1;
    type Class = DomainValueKindClass;
    const NAMESPACE: &'static str = "canwu.force-supply-reference";
    const NAME: &'static str = "externality-outcome";
}

#[must_use]
pub fn force_externality_outcome_reference(
    id: &ExternalityOutcomeId,
) -> TypedDomainRecordRef<ForceExternalityOutcomeProviderRecord> {
    TypedDomainRecordRef::new(format!(
        "canwu.force-supply-reference:externality-outcome:{id}"
    ))
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ForceExternalityCompletionParticipantProviderV1 {
    pub provider_plugin: String,
    pub participant: ExternalCompletionParticipantGrantV1,
    pub semantic_digest: String,
}

impl ForceExternalityCompletionParticipantProviderV1 {
    pub fn seal(mut self) -> Result<Self, CanwuError> {
        self.semantic_digest.clear();
        self.semantic_digest = canonical_hash(
            "canwu.force-supply.externality-completion-participant.v1",
            &self,
        )?;
        Ok(self)
    }
}

#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ForceExternalityCompletionParticipantProviderRecord;

impl DomainRecordType for ForceExternalityCompletionParticipantProviderRecord {
    type Payload = ForceExternalityCompletionParticipantProviderV1;
    type Class = DomainValueKindClass;
    const NAMESPACE: &'static str = "canwu.force-supply-reference";
    const NAME: &'static str = "externality-completion-participant";
}

#[must_use]
pub fn force_externality_completion_participant_reference(
    acquisition: &CompletionLeaseAcquisitionId,
) -> TypedDomainRecordRef<ForceExternalityCompletionParticipantProviderRecord> {
    TypedDomainRecordRef::new(format!(
        "canwu.force-supply-reference:externality-completion-participant:{acquisition}"
    ))
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(from = "ForceSupplyStateStorageV1", into = "ForceSupplyStateStorageV1")]
pub struct ForceSupplyStateV1 {
    pub format_version: u32,
    pub revision: u64,
    pub provider_record_version: u64,
    pub limits: ForceSupplyReferenceLimitsV1,
    pub compiled_content: Option<CompiledEconomyReferenceContentV1>,
    pub profiles: BTreeMap<ForceSupplyProfileId, ForceSupplyProfileV1>,
    pub requisition_policies: BTreeMap<RequisitionPolicyId, RequisitionPolicyV1>,
    pub forces: BTreeMap<ReferenceForceId, ReferenceForce>,
    #[serde(default)]
    pub due_index: BTreeMap<SimTime, BTreeSet<(ReferenceForceId, ForceRequirementId)>>,
    pub intents: BTreeMap<ForceConsumptionIntentId, ForceConsumptionIntent>,
    pub consequences: BTreeMap<ForceConsequenceId, ForceConsequenceRecord>,
    pub externality_intents: BTreeMap<ForceExternalityIntentId, ForceExternalityIntent>,
    pub sagas: BTreeMap<RequisitionSagaId, RequisitionSagaV1>,
    pub observation_grants: BTreeMap<ForceObserverGrantId, ForceObserverGrantV1>,
    pub observations:
        BTreeMap<ReferenceForceId, BTreeMap<ForceRequirementId, ForceSupplyObservationV1>>,
    pub knowledge_publications: BTreeMap<ForceKnowledgePublicationId, ForceKnowledgePublicationV1>,
    pub observation_temporal_index: BTreeMap<
        ForceObservationScopeV1,
        BTreeMap<ForceObservationTemporalKeyV1, ForceKnowledgePublicationId>,
    >,
    pub completion_run_budget: RunBudgetRevisionV1,
    pub completion_leases: CompletionLeaseBookV1,
    #[serde(default)]
    pub completion_participant_grants: BTreeMap<
        CompletionLeaseAcquisitionId,
        BTreeMap<String, ExternalCompletionParticipantGrantV1>,
    >,
    pub next_terminal_sequence: u64,
    pub terminal_receipts: BTreeMap<u64, ForceTerminalReceiptV1>,
    pub terminal_continuation: Option<ForceTerminalContinuationV1>,
    pub archive_head: crate::ForceArchiveHeadStateV1,
    pub archive_retention_handles: BTreeMap<String, crate::ForceArchiveRetentionHandleV1>,
    pub archive_maintenance_receipts: BTreeMap<u64, crate::ForceArchiveMaintenanceReceiptV1>,
    pub outcomes: BTreeMap<ForceOperationId, ForceOperationOutcomeV1>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ForceSupplyStateStorageV1 {
    format_version: u32,
    revision: u64,
    provider_record_version: u64,
    limits: ForceSupplyReferenceLimitsV1,
    compiled_content: Option<CompiledEconomyReferenceContentV1>,
    profiles: BTreeMap<ForceSupplyProfileId, ForceSupplyProfileV1>,
    requisition_policies: BTreeMap<RequisitionPolicyId, RequisitionPolicyV1>,
    forces: BTreeMap<ReferenceForceId, ReferenceForce>,
    intents: BTreeMap<ForceConsumptionIntentId, ForceConsumptionIntent>,
    consequences: BTreeMap<ForceConsequenceId, ForceConsequenceRecord>,
    externality_intents: BTreeMap<ForceExternalityIntentId, ForceExternalityIntent>,
    sagas: BTreeMap<RequisitionSagaId, RequisitionSagaV1>,
    observation_grants: BTreeMap<ForceObserverGrantId, ForceObserverGrantV1>,
    observations:
        BTreeMap<ReferenceForceId, BTreeMap<ForceRequirementId, ForceSupplyObservationV1>>,
    knowledge_publications: BTreeMap<ForceKnowledgePublicationId, ForceKnowledgePublicationV1>,
    completion_run_budget: RunBudgetRevisionV1,
    completion_leases: CompletionLeaseBookV1,
    completion_participant_grants: BTreeMap<
        CompletionLeaseAcquisitionId,
        BTreeMap<String, ExternalCompletionParticipantGrantV1>,
    >,
    next_terminal_sequence: u64,
    terminal_receipts: BTreeMap<u64, ForceTerminalReceiptV1>,
    terminal_continuation: Option<ForceTerminalContinuationV1>,
    archive_head: crate::ForceArchiveHeadStateV1,
    archive_retention_handles: BTreeMap<String, crate::ForceArchiveRetentionHandleV1>,
    archive_maintenance_receipts: BTreeMap<u64, crate::ForceArchiveMaintenanceReceiptV1>,
    outcomes: BTreeMap<ForceOperationId, ForceOperationOutcomeV1>,
}

impl From<ForceSupplyStateV1> for ForceSupplyStateStorageV1 {
    fn from(state: ForceSupplyStateV1) -> Self {
        Self {
            format_version: state.format_version,
            revision: state.revision,
            provider_record_version: state.provider_record_version,
            limits: state.limits,
            compiled_content: state.compiled_content,
            profiles: state.profiles,
            requisition_policies: state.requisition_policies,
            forces: state.forces,
            intents: state.intents,
            consequences: state.consequences,
            externality_intents: state.externality_intents,
            sagas: state.sagas,
            observation_grants: state.observation_grants,
            observations: state.observations,
            knowledge_publications: state.knowledge_publications,
            completion_run_budget: state.completion_run_budget,
            completion_leases: state.completion_leases,
            completion_participant_grants: state.completion_participant_grants,
            next_terminal_sequence: state.next_terminal_sequence,
            terminal_receipts: state.terminal_receipts,
            terminal_continuation: state.terminal_continuation,
            archive_head: state.archive_head,
            archive_retention_handles: state.archive_retention_handles,
            archive_maintenance_receipts: state.archive_maintenance_receipts,
            outcomes: state.outcomes,
        }
    }
}

impl From<ForceSupplyStateStorageV1> for ForceSupplyStateV1 {
    fn from(state: ForceSupplyStateStorageV1) -> Self {
        let mut restored = Self {
            format_version: state.format_version,
            revision: state.revision,
            provider_record_version: state.provider_record_version,
            limits: state.limits,
            compiled_content: state.compiled_content,
            profiles: state.profiles,
            requisition_policies: state.requisition_policies,
            forces: state.forces,
            due_index: BTreeMap::new(),
            intents: state.intents,
            consequences: state.consequences,
            externality_intents: state.externality_intents,
            sagas: state.sagas,
            observation_grants: state.observation_grants,
            observations: state.observations,
            knowledge_publications: state.knowledge_publications,
            observation_temporal_index: BTreeMap::new(),
            completion_run_budget: state.completion_run_budget,
            completion_leases: state.completion_leases,
            completion_participant_grants: state.completion_participant_grants,
            next_terminal_sequence: state.next_terminal_sequence,
            terminal_receipts: state.terminal_receipts,
            terminal_continuation: state.terminal_continuation,
            archive_head: state.archive_head,
            archive_retention_handles: state.archive_retention_handles,
            archive_maintenance_receipts: state.archive_maintenance_receipts,
            outcomes: state.outcomes,
        };
        restored.rebuild_derived_indexes();
        restored
    }
}

impl Default for ForceSupplyStateV1 {
    fn default() -> Self {
        Self {
            format_version: 1,
            revision: 1,
            provider_record_version: 1,
            limits: ForceSupplyReferenceLimitsV1::canonical(),
            compiled_content: None,
            profiles: BTreeMap::new(),
            requisition_policies: BTreeMap::new(),
            forces: BTreeMap::new(),
            due_index: BTreeMap::new(),
            intents: BTreeMap::new(),
            consequences: BTreeMap::new(),
            externality_intents: BTreeMap::new(),
            sagas: BTreeMap::new(),
            observation_grants: BTreeMap::new(),
            observations: BTreeMap::new(),
            knowledge_publications: BTreeMap::new(),
            observation_temporal_index: BTreeMap::new(),
            completion_run_budget: default_completion_run_budget(),
            completion_leases: CompletionLeaseBookV1::default(),
            completion_participant_grants: BTreeMap::new(),
            next_terminal_sequence: 1,
            terminal_receipts: BTreeMap::new(),
            terminal_continuation: None,
            archive_head: crate::sealed_archive_head(
                crate::FORCE_ARCHIVE_DOMAIN,
                crate::ForceArchiveHeadStateV1::default(),
            )
            .expect("static empty force archive head must seal"),
            archive_retention_handles: BTreeMap::new(),
            archive_maintenance_receipts: BTreeMap::new(),
            outcomes: BTreeMap::new(),
        }
    }
}

fn default_completion_run_budget() -> RunBudgetRevisionV1 {
    RunBudgetRevisionV1 {
        revision: ResourceRevision::INITIAL,
        total_completion_units: 4_000_000,
        shared_pending_slots: 64,
        partitions: Vec::new(),
        semantic_digest: String::new(),
    }
    .seal()
    .expect("static force completion budget must be valid")
}

#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ForceSupplyRuntimeRecord;

impl DomainRecordType for ForceSupplyRuntimeRecord {
    type Payload = ForceSupplyStateV1;
    type Class = DomainValueKindClass;
    const NAMESPACE: &'static str = PLUGIN_NAMESPACE;
    const NAME: &'static str = "runtime";
}

#[must_use]
pub fn force_supply_runtime_reference() -> TypedDomainRecordRef<ForceSupplyRuntimeRecord> {
    TypedDomainRecordRef::new(FORCE_RUNTIME_ID)
}

impl ForceSupplyStateV1 {
    pub fn rebuild_derived_indexes(&mut self) {
        self.due_index.clear();
        for force in self.forces.values() {
            for due in force.due.values() {
                self.due_index
                    .entry(due.next_due)
                    .or_default()
                    .insert((force.id.clone(), due.requirement.clone()));
            }
        }
        self.observation_temporal_index.clear();
        for publication in self.knowledge_publications.values() {
            self.observation_temporal_index
                .entry(observation_temporal_scope_key(
                    &publication.holder,
                    &publication.force,
                    &publication.fact,
                ))
                .or_default()
                .insert(
                    ForceObservationTemporalKeyV1 {
                        observed_at: publication.observed_at,
                        provider_revision: publication.provider_revision,
                        publication: publication.id.clone(),
                    },
                    publication.id.clone(),
                );
        }
    }

    pub fn configure_completion_authority(
        &mut self,
        holder: KnowledgeHolderRef,
    ) -> Result<(), CanwuError> {
        let operation_namespace = "canwu.force-supply-reference:requisition".to_owned();
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
        if !self.completion_leases.acquisitions.is_empty()
            || !self.completion_leases.grants.is_empty()
        {
            return Err(invalid(
                "force completion budget cannot change after lease admission",
            ));
        }
        self.completion_run_budget
            .partitions
            .push(CompletionCapacityPartitionV1 {
                authority: holder,
                operation_namespace,
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
        self.validate()
    }

    #[allow(clippy::too_many_lines)]
    pub fn acknowledge_external_participant(
        &mut self,
        holder: &KnowledgeHolderRef,
        participant: ExternalCompletionParticipantGrantV1,
    ) -> Result<(), CanwuError> {
        let grant = &participant.grant;
        require_completion_holder(self, holder, &grant.acquisition)?;
        let acquisition = self
            .completion_leases
            .acquisitions
            .get(&grant.acquisition)
            .cloned()
            .ok_or_else(|| invalid("force completion acquisition is unavailable"))?;
        if !acquisition
            .expected_participants
            .contains(&grant.owner_plugin)
            || grant.owner_plugin == PLUGIN_NAME
            || participant.coordinator_plugin != PLUGIN_NAME
            || participant.holder != acquisition.holder
            || participant.operation_namespace != acquisition.operation_namespace
            || participant.eligibility_time != acquisition.eligibility_time
            || participant.eligibility_envelope_digest != acquisition.eligibility_envelope.digest
            || participant.recipe != acquisition.recipe
            || participant.policy_class != acquisition.policy_class
            || participant.coordinator_acquisition_revision > acquisition.revision
            || grant.operation_key != acquisition.operation_key
            || grant.recipe_digest != acquisition.recipe_digest
            || !matches!(
                grant.state,
                CompletionGrantStateV1::Held
                    | CompletionGrantStateV1::Prepared
                    | CompletionGrantStateV1::Consumed
                    | CompletionGrantStateV1::Completed
                    | CompletionGrantStateV1::Released
                    | CompletionGrantStateV1::Rejected
                    | CompletionGrantStateV1::Expired
            )
        {
            return Err(invalid(
                "force completion participant acknowledgement is stale or forged",
            ));
        }
        let by_owner = self
            .completion_participant_grants
            .entry(grant.acquisition.clone())
            .or_default();
        if let Some(existing) = by_owner.get(&grant.owner_plugin) {
            if existing == &participant {
                return Ok(());
            }
            let existing_grant = &existing.grant;
            let stable = existing.coordinator_plugin == participant.coordinator_plugin
                && existing.holder == participant.holder
                && existing.operation_namespace == participant.operation_namespace
                && existing.eligibility_time == participant.eligibility_time
                && existing.eligibility_envelope_digest == participant.eligibility_envelope_digest
                && existing.recipe == participant.recipe
                && existing.policy_class == participant.policy_class
                && existing_grant.id == grant.id
                && existing_grant.acquisition == grant.acquisition
                && existing_grant.operation_key == grant.operation_key
                && existing_grant.owner_plugin == grant.owner_plugin
                && existing_grant.run_budget_revision == grant.run_budget_revision
                && existing_grant.target_versions == grant.target_versions
                && existing_grant.recipe_digest == grant.recipe_digest
                && existing_grant.reserved_units == grant.reserved_units
                && existing_grant.expires_after_boundary == grant.expires_after_boundary;
            let next_revision = existing_grant.revision.next().map_err(resource_error)?;
            let monotonic = (grant.revision == next_revision
                && matches!(
                    (existing_grant.state, grant.state),
                    (
                        CompletionGrantStateV1::Held,
                        CompletionGrantStateV1::Prepared
                            | CompletionGrantStateV1::Released
                            | CompletionGrantStateV1::Rejected
                            | CompletionGrantStateV1::Expired
                    ) | (
                        CompletionGrantStateV1::Prepared,
                        CompletionGrantStateV1::Consumed
                            | CompletionGrantStateV1::Released
                            | CompletionGrantStateV1::Expired
                    ) | (
                        CompletionGrantStateV1::Consumed,
                        CompletionGrantStateV1::Completed
                    )
                ))
                || (existing_grant.state == CompletionGrantStateV1::Prepared
                    && grant.state == CompletionGrantStateV1::Completed
                    && participant.certificate.is_some()
                    && grant.revision == next_revision.next().map_err(resource_error)?);
            if !stable || !monotonic {
                return Err(invalid(
                    "force completion participant acknowledgement conflicts with its exact owner grant",
                ));
            }
        } else if grant.state != CompletionGrantStateV1::Held
            || grant.revision != ResourceRevision::INITIAL
        {
            return Err(invalid(
                "force completion participant must first acknowledge its exact held owner grant",
            ));
        }
        by_owner.insert(grant.owner_plugin.clone(), participant);
        self.refresh_completion_acquisition_state(&acquisition.id, true)?;
        Ok(())
    }

    fn refresh_completion_acquisition_state(
        &mut self,
        acquisition_id: &CompletionLeaseAcquisitionId,
        advance_revision: bool,
    ) -> Result<(), CanwuError> {
        let snapshot = self
            .completion_leases
            .acquisitions
            .get(acquisition_id)
            .cloned()
            .ok_or_else(|| invalid("force completion acquisition is unavailable"))?;
        if matches!(
            snapshot.state,
            CompletionLeaseAcquisitionStateV1::Activated
                | CompletionLeaseAcquisitionStateV1::Aborting
                | CompletionLeaseAcquisitionStateV1::Released
                | CompletionLeaseAcquisitionStateV1::Expired
        ) {
            return Ok(());
        }
        let local = snapshot
            .grants
            .iter()
            .map(|(owner, id)| {
                self.completion_leases
                    .grants
                    .get(id)
                    .map(|grant| (owner.clone(), grant.state))
                    .ok_or_else(|| invalid("force completion local grant is orphaned"))
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        let external = self
            .completion_participant_grants
            .get(acquisition_id)
            .map(|values| {
                values
                    .iter()
                    .map(|(owner, participant)| (owner.clone(), participant.grant.state))
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default();
        let observed = local
            .keys()
            .chain(external.keys())
            .cloned()
            .collect::<BTreeSet<_>>();
        let state = if observed == snapshot.expected_participants {
            let mut states = local.values().chain(external.values()).copied();
            if states.clone().any(|state| {
                matches!(
                    state,
                    CompletionGrantStateV1::Released
                        | CompletionGrantStateV1::Rejected
                        | CompletionGrantStateV1::Expired
                )
            }) {
                CompletionLeaseAcquisitionStateV1::Aborting
            } else if states
                .clone()
                .all(|state| state == CompletionGrantStateV1::Prepared)
            {
                CompletionLeaseAcquisitionStateV1::PreparedAll
            } else if states.any(|state| state == CompletionGrantStateV1::Prepared) {
                CompletionLeaseAcquisitionStateV1::Preparing
            } else {
                CompletionLeaseAcquisitionStateV1::FullyGranted
            }
        } else if observed.is_empty() {
            CompletionLeaseAcquisitionStateV1::Requested
        } else {
            CompletionLeaseAcquisitionStateV1::PartiallyGranted
        };
        let acquisition = self
            .completion_leases
            .acquisitions
            .get_mut(acquisition_id)
            .expect("force completion acquisition was checked");
        acquisition.state = state;
        if advance_revision {
            acquisition.revision = acquisition.revision.next().map_err(resource_error)?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn activate_completion_with_external(
        &mut self,
        request: &ActivateCompletionLeaseV1,
    ) -> Result<CompletionLeaseActivationCertificateV1, CanwuError> {
        let acquisition = self
            .completion_leases
            .acquisitions
            .get(&request.acquisition)
            .cloned()
            .ok_or_else(|| invalid("force completion acquisition is unavailable"))?;
        let cited_grant = self
            .completion_leases
            .grants
            .get(&request.grant)
            .ok_or_else(|| invalid("force completion activation grant is unavailable"))?;
        if acquisition.revision != request.expected_acquisition_revision
            || acquisition.state != CompletionLeaseAcquisitionStateV1::PreparedAll
            || acquisition.eligibility_time != request.at
            || acquisition.eligibility_envelope.digest != request.eligibility_envelope_digest
            || cited_grant.revision != request.expected_grant_revision
            || cited_grant.acquisition != request.acquisition
            || cited_grant.state != CompletionGrantStateV1::Prepared
        {
            return Err(invalid(
                "force completion activation exact acquisition, grant, time, or envelope differs",
            ));
        }
        let external = self
            .completion_participant_grants
            .get(&request.acquisition)
            .cloned()
            .unwrap_or_default();
        let observed = acquisition
            .grants
            .keys()
            .chain(external.keys())
            .cloned()
            .collect::<BTreeSet<_>>();
        if observed != acquisition.expected_participants {
            return Err(invalid(
                "force completion activation is missing an owner participant",
            ));
        }
        let mut grants = acquisition
            .grants
            .values()
            .map(|id| {
                self.completion_leases
                    .grants
                    .get(id)
                    .cloned()
                    .ok_or_else(|| invalid("force completion activation lost a local grant"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        grants.extend(external.values().map(|value| value.grant.clone()));
        if grants
            .iter()
            .any(|grant| grant.state != CompletionGrantStateV1::Prepared)
        {
            return Err(invalid(
                "force completion activation requires every owner grant prepared",
            ));
        }
        let earliest_deadline = grants
            .iter()
            .map(|grant| {
                grant.activation_deadline_boundary.ok_or_else(|| {
                    invalid("force completion prepared grant has no activation deadline")
                })
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .min()
            .ok_or_else(|| invalid("force completion activation has no participants"))?;
        if request.current_boundary >= earliest_deadline {
            return Err(invalid(
                "force completion activation must precede every owner deadline",
            ));
        }
        let mut prepared_grants = grants
            .iter()
            .map(|grant| (grant.id.clone(), grant.revision))
            .collect::<Vec<_>>();
        prepared_grants.sort();
        let mut locked_target_versions = grants
            .iter()
            .flat_map(|grant| grant.target_versions.iter().cloned())
            .collect::<Vec<_>>();
        locked_target_versions.sort();
        locked_target_versions.dedup();
        let acquisition = self
            .completion_leases
            .acquisitions
            .get_mut(&request.acquisition)
            .expect("force completion acquisition was checked");
        acquisition.state = CompletionLeaseAcquisitionStateV1::Activated;
        acquisition.revision = acquisition.revision.next().map_err(resource_error)?;
        let mut certificate = CompletionLeaseActivationCertificateV1 {
            acquisition: acquisition.id.clone(),
            acquisition_revision: acquisition.revision,
            operation_key: acquisition.operation_key.clone(),
            prepared_grants,
            locked_target_versions,
            recipe_digest: acquisition.recipe_digest.clone(),
            eligibility_time: acquisition.eligibility_time,
            eligibility_envelope_digest: acquisition.eligibility_envelope.digest.clone(),
            activation_boundary: request.current_boundary,
            semantic_digest: String::new(),
        };
        certificate.semantic_digest = canwu_resource::canonical_digest(
            "canwu.resource.completion-activation-certificate.v1",
            &certificate,
        )
        .map_err(resource_error)?;
        self.completion_leases
            .certificates
            .insert(certificate.acquisition.clone(), certificate.clone());
        for grants in self.completion_leases.expiry_due.values_mut() {
            grants.retain(|grant_id| !acquisition.grants.values().any(|id| id == grant_id));
        }
        self.completion_leases
            .expiry_due
            .retain(|_, grants| !grants.is_empty());
        Ok(certificate)
    }

    pub fn from_compiled_content(
        compiled_content: CompiledEconomyReferenceContentV1,
    ) -> Result<Self, CanwuError> {
        let configuration = crate::compile_force_supply_configuration(&compiled_content)?;
        let mut state = Self {
            compiled_content: Some(compiled_content),
            profiles: configuration.profiles,
            requisition_policies: configuration.requisition_policies,
            ..Self::default()
        };
        state.revision = 1;
        state.validate()?;
        Ok(state)
    }

    pub fn into_initial_record(self) -> Result<DomainRecord, CanwuError> {
        self.validate()?;
        if self.provider_record_version != 1 {
            return Err(invalid(
                "initial force runtime must bind provider record version one",
            ));
        }
        let draft = DomainRecordDraft::from_typed(force_supply_runtime_reference(), &self)?;
        Ok(DomainRecord {
            reference: draft.reference,
            owner: PLUGIN_NAME.to_owned(),
            class: DomainRecordClass::Record,
            version: 1,
            lifecycle: DomainRecordLifecycle::Active,
            payload: draft.payload,
            references: draft.references,
        })
    }

    pub(crate) fn draft(&self) -> Result<DomainRecordDraft, CanwuError> {
        self.validate()?;
        DomainRecordDraft::from_typed(force_supply_runtime_reference(), self)
    }

    #[allow(clippy::too_many_lines)]
    pub fn validate(&self) -> Result<(), CanwuError> {
        self.limits.validate()?;
        self.completion_run_budget
            .validate()
            .map_err(resource_error)?;
        self.completion_leases
            .validate(&self.completion_run_budget)
            .map_err(resource_error)?;
        for (acquisition_id, participants) in &self.completion_participant_grants {
            let acquisition = self
                .completion_leases
                .acquisitions
                .get(acquisition_id)
                .ok_or_else(|| invalid("force external completion participant is orphaned"))?;
            for (owner, participant) in participants {
                let grant = &participant.grant;
                let certificate = self.completion_leases.certificate(acquisition_id);
                if owner != &grant.owner_plugin
                    || owner == PLUGIN_NAME
                    || !acquisition.expected_participants.contains(owner)
                    || acquisition.grants.contains_key(owner)
                    || participant.coordinator_plugin != PLUGIN_NAME
                    || participant.holder != acquisition.holder
                    || participant.operation_namespace != acquisition.operation_namespace
                    || participant.eligibility_time != acquisition.eligibility_time
                    || participant.eligibility_envelope_digest
                        != acquisition.eligibility_envelope.digest
                    || participant.recipe != acquisition.recipe
                    || participant.policy_class != acquisition.policy_class
                    || participant.coordinator_acquisition_revision > acquisition.revision
                    || grant.acquisition != *acquisition_id
                    || grant.operation_key != acquisition.operation_key
                    || grant.recipe_digest != acquisition.recipe_digest
                    || (matches!(
                        grant.state,
                        CompletionGrantStateV1::Consumed | CompletionGrantStateV1::Completed
                    ) && participant.certificate.as_ref() != certificate)
                {
                    return Err(invalid(
                        "force external completion participant closure is invalid",
                    ));
                }
            }
        }
        for acquisition in self.completion_leases.acquisitions.values() {
            if acquisition.grants.keys().any(|owner| owner != PLUGIN_NAME) {
                return Err(invalid(
                    "force completion coordinator cannot shadow another owner's reservation",
                ));
            }
            let participants = self
                .completion_participant_grants
                .get(&acquisition.id)
                .map(|values| values.keys().cloned().collect::<BTreeSet<_>>())
                .unwrap_or_default();
            let observed = acquisition
                .grants
                .keys()
                .chain(participants.iter())
                .cloned()
                .collect::<BTreeSet<_>>();
            if !matches!(
                acquisition.state,
                CompletionLeaseAcquisitionStateV1::Requested
                    | CompletionLeaseAcquisitionStateV1::PartiallyGranted
                    | CompletionLeaseAcquisitionStateV1::Aborting
            ) && observed != acquisition.expected_participants
            {
                return Err(invalid(
                    "force completion acquisition is missing an exact owner participant",
                ));
            }
        }
        let encoded = serde_json::to_vec(self).map_err(encode_error)?;
        if self.format_version != 1
            || self.revision == 0
            || self.provider_record_version == 0
            || encoded.len() > self.limits.max_state_bytes
            || self.profiles.len() > self.limits.max_profiles
            || self.forces.len() > self.limits.max_forces
            || self.intents.len() > self.limits.max_active_intents
            || self.sagas.len() > self.limits.max_active_sagas
            || self.consequences.len() > self.limits.max_consequences
            || self.outcomes.len() > self.limits.max_externality_outcomes
            || self.observation_grants.len() > self.limits.max_observer_grants
            || self.knowledge_publications.len() > self.limits.max_observer_grants
            || self.terminal_receipts.len() > self.limits.max_terminal_receipts
            || self.next_terminal_sequence == 0
        {
            return Err(invalid(
                "force-supply state exceeds its schema or bounded limits",
            ));
        }
        match &self.compiled_content {
            Some(content) => {
                let configuration = crate::compile_force_supply_configuration(content)?;
                if self.profiles != configuration.profiles
                    || self.requisition_policies != configuration.requisition_policies
                {
                    return Err(invalid(
                        "force profiles or requisition policies differ from exact compiled economy coverage",
                    ));
                }
            }
            None if !self.profiles.is_empty() || !self.requisition_policies.is_empty() => {
                return Err(invalid(
                    "force profiles and policies require exact compiled economy content",
                ));
            }
            None => {}
        }
        for profile in self.profiles.values() {
            validate_profile(profile)?;
        }
        for policy in self.requisition_policies.values() {
            validate_policy(policy)?;
        }
        let mut expected_due_index =
            BTreeMap::<SimTime, BTreeSet<(ReferenceForceId, ForceRequirementId)>>::new();
        for force in self.forces.values() {
            let profile = self
                .profiles
                .get(&force.profile)
                .ok_or_else(|| invalid("force profile is unavailable"))?;
            if force.revision == 0
                || force.readiness_per_mille > 1_000
                || force.fatigue_per_mille > 1_000
                || force.cohesion_per_mille > 1_000
                || force.disease_per_mille > 1_000
                || force.desertion_per_mille > 1_000
            {
                return Err(invalid("force readiness or consequence state is invalid"));
            }
            let requirements: BTreeSet<_> =
                profile.requirements.iter().map(|value| &value.id).collect();
            if force.due.keys().any(|id| !requirements.contains(id)) {
                return Err(invalid(
                    "force due state references a requirement outside its profile",
                ));
            }
            if force.active && force.due.len() != profile.requirements.len() {
                return Err(invalid(
                    "active force does not carry due state for every compiled requirement",
                ));
            }
            for (requirement_id, due) in &force.due {
                let requirement = profile
                    .requirements
                    .iter()
                    .find(|value| &value.id == requirement_id)
                    .ok_or_else(|| invalid("force due state requirement is unavailable"))?;
                if due.requirement != *requirement_id
                    || cadence_interval(requirement)
                        .is_some_and(|interval| due.persisted_remainder_minutes >= interval)
                {
                    return Err(invalid("force due state is non-canonical"));
                }
                expected_due_index
                    .entry(due.next_due)
                    .or_default()
                    .insert((force.id.clone(), requirement_id.clone()));
            }
        }
        if self.due_index != expected_due_index {
            return Err(invalid(
                "force due index differs from canonical force requirement state",
            ));
        }
        for (intent_id, intent) in &self.intents {
            let force = self
                .forces
                .get(&intent.force)
                .ok_or_else(|| invalid("force intent references a missing force"))?;
            let profile = &self.profiles[&force.profile];
            let requirement = profile
                .requirements
                .iter()
                .find(|value| value.id == intent.requirement)
                .ok_or_else(|| invalid("force intent requirement is unavailable"))?;
            let mut detached_intent = intent.clone();
            let recorded_intent_digest = std::mem::take(&mut detached_intent.semantic_digest);
            let mut detached_certificate = intent.completion_certificate.clone();
            let recorded_certificate_digest =
                std::mem::take(&mut detached_certificate.semantic_digest);
            let authoritative_certificate = self
                .completion_leases
                .certificate(&intent.completion_certificate.acquisition);
            let acquisition = self
                .completion_leases
                .acquisitions
                .get(&intent.completion_certificate.acquisition)
                .ok_or_else(|| invalid("force intent completion acquisition is unavailable"))?;
            if intent_id != &intent.id
                || intent.revision == 0
                || intent.due_count == 0
                || intent.expected_force_runtime_revision == 0
                || intent.requested_quantity
                    != requirement
                        .quantity_per_due
                        .checked_mul(u64::from(intent.due_count))
                        .ok_or_else(|| invalid("force intent quantity overflowed"))?
                || intent.allocation.resource_revision != requirement.resource_revision
                || intent.allocation.unit_revision != requirement.unit_revision
                || intent.allocation.quantity > intent.requested_quantity
                || recorded_intent_digest
                    != canonical_hash("canwu.force-supply.intent.v1", &detached_intent)?
                || intent.resource_outcome.is_some() != intent.resource_outcome_source.is_some()
                || intent.completion_certificate.operation_key != intent.resource_operation_key
                || intent.due_at < intent.scheduled_due
                || intent.completion_certificate.eligibility_time != intent.due_at
                || recorded_certificate_digest
                    != canwu_resource::canonical_digest(
                        "canwu.resource.completion-activation-certificate.v1",
                        &detached_certificate,
                    )
                    .map_err(resource_error)?
                || authoritative_certificate != Some(&intent.completion_certificate)
                || acquisition.operation_key != intent.resource_operation_key
                || acquisition.holder != force.holder
                || acquisition.operation_namespace != "canwu.force-supply-reference:requisition"
                || acquisition.eligibility_time != intent.due_at
                || acquisition.recipe.reports_per_holder == 0
                || acquisition.recipe.holders == 0
            {
                return Err(invalid("force consumption intent is not exactly bound"));
            }
            validate_stock_custody_binding(
                &intent.stock_custody,
                &intent.allocation,
                &force.holder,
            )?;
            validate_completion_participants(self, intent, force, acquisition)?;
            let targets = &intent.completion_certificate.locked_target_versions;
            if !targets.iter().any(|value| {
                matches!(value, CompletionLockedTargetV1::Account { id, revision }
                    if id == &intent.allocation.account
                        && revision == &intent.allocation.account_revision)
            })
                || !targets.iter().any(|value| {
                    matches!(value, CompletionLockedTargetV1::AllocationLeg { id, revision }
                        if id == &intent.allocation.id && revision == &intent.allocation.revision)
                })
                || !targets.iter().any(|value| {
                    matches!(value, CompletionLockedTargetV1::ExternalRecord { version }
                        if version.record == force_supply_runtime_reference().into_untyped()
                            && version.version != 0)
                })
                || intent.requisition_policy.as_ref().is_some_and(|policy| {
                    self.requisition_policies.get(policy).is_some_and(|policy| {
                        policy.applicability == ExternalityApplicability::Required
                            && !targets
                                .iter()
                                .any(|value| {
                                    matches!(value, CompletionLockedTargetV1::ExternalRecord { version }
                                        if version.record.kind.namespace == "canwu.economy-reference")
                                })
                    })
                })
            {
                return Err(invalid(
                    "force intent completion certificate omits a required participant target",
                ));
            }
            if intent.requisition_policy.as_ref().is_some_and(|policy| {
                self.requisition_policies[policy].applicability
                    == ExternalityApplicability::Required
            }) {
                exact_economy_target(intent)?;
            }
            match intent.status {
                ForceConsumptionIntentStatus::PendingResourceConsumption => {
                    if intent.resource_outcome.is_some()
                        || intent.resource_outcome_source.is_some()
                        || intent.consequence.is_some()
                    {
                        return Err(invalid(
                            "pending force intent already carries terminal consequence evidence",
                        ));
                    }
                }
                ForceConsumptionIntentStatus::ConsequenceCommitted => {
                    if intent.resource_outcome.is_none()
                        || intent.resource_outcome_source.is_none()
                        || intent.consequence.is_none()
                        || intent.requisition_policy.is_none()
                    {
                        return Err(invalid(
                            "committed force intent lacks its exact outcome closure",
                        ));
                    }
                }
                ForceConsumptionIntentStatus::Settled | ForceConsumptionIntentStatus::Rejected => {
                    return Err(invalid(
                        "terminal force intent must be represented by a terminal receipt",
                    ));
                }
            }
        }
        for (consequence_id, consequence) in &self.consequences {
            let intent = self
                .intents
                .get(&consequence.intent)
                .ok_or_else(|| invalid("force consequence is orphaned from its intent"))?;
            let mut detached = consequence.clone();
            let recorded = std::mem::take(&mut detached.semantic_digest);
            let outcome_digest = &consequence.attribution.resource_outcome.semantic_digest;
            if consequence_id != &consequence.id
                || consequence.force != intent.force
                || intent.consequence.as_ref() != Some(consequence_id)
                || intent.resource_outcome.as_ref()
                    != Some(&consequence.attribution.resource_outcome)
                || consequence.attribution.requirement != intent.requirement
                || consequence.attribution.requested != intent.requested_quantity
                || consequence.attribution.consumed
                    != consequence.attribution.resource_outcome.quantity
                || consequence.attribution.consumption.id != intent.consumption_id
                || consequence.attribution.consumption.account
                    != intent.stock_custody.destination_account
                || consequence.attribution.consumption.allocation_leg != intent.allocation.id
                || consequence.attribution.fulfillment.operation_key
                    != intent.resource_operation_key
                || consequence.attribution.fulfillment.consumed_quantity
                    != consequence.attribution.consumed
                || consequence.attribution.stock_custody != intent.stock_custody
                || consequence.attribution.shortage
                    != consequence
                        .attribution
                        .requested
                        .saturating_sub(consequence.attribution.consumed)
                || consequence.force_revision_after
                    != consequence.force_revision_before.saturating_add(1)
                || outcome_digest.len() != 64
                || recorded != canonical_hash("canwu.force-supply.consequence.v1", &detached)?
            {
                return Err(invalid("force consequence closure is forged"));
            }
        }
        for (saga_id, saga) in &self.sagas {
            let intent = self
                .intents
                .get(&saga.intent)
                .ok_or_else(|| invalid("requisition saga intent is unavailable"))?;
            let expected_saga_id =
                RequisitionSagaId::new(format!("canwu.force-supply-reference:saga:{}", intent.id))?;
            if saga_id != &saga.id
                || saga.id != expected_saga_id
                || saga.force != intent.force
                || intent.requisition_policy.is_none()
                || self.forces[&saga.force]
                    .blocked_by_active_requisition
                    .as_ref()
                    != Some(saga_id)
            {
                return Err(invalid(
                    "requisition saga is not bound to a requisition intent",
                ));
            }
            validate_saga_closure(self, saga, intent)?;
        }
        for (externality_id, externality) in &self.externality_intents {
            let saga = self
                .sagas
                .get(&externality.saga)
                .ok_or_else(|| invalid("force externality intent is orphaned from its saga"))?;
            let intent = self
                .intents
                .get(&saga.intent)
                .ok_or_else(|| invalid("force externality intent lost its force intent"))?;
            let consequence = self
                .consequences
                .get(&externality.force_consequence)
                .ok_or_else(|| invalid("force externality intent lost its consequence"))?;
            let policy_id = intent
                .requisition_policy
                .as_ref()
                .ok_or_else(|| invalid("force externality intent lacks a policy"))?;
            let policy = &self.requisition_policies[policy_id];
            let mut detached = externality.clone();
            let recorded = std::mem::take(&mut detached.semantic_digest);
            if externality_id != &externality.id
                || saga.externality_intent.as_ref() != Some(externality_id)
                || externality.operation_key != intent.resource_operation_key
                || externality.force_consequence != consequence.id
                || externality.resource_outcome != consequence.attribution.resource_outcome
                || externality.expected_economy_target != *exact_economy_target(intent)?
                || externality.cooperation_delta_per_mille != policy.cooperation_delta_per_mille
                || externality.harvest_input_delta_per_mille != policy.harvest_input_delta_per_mille
                || externality.quantity != consequence.attribution.consumed
                || externality.policy != *policy_id
                || recorded
                    != canonical_hash("canwu.force-supply.externality-intent.v1", &detached)?
            {
                return Err(invalid(
                    "force externality intent differs from its exact policy/consequence/resource outcome",
                ));
            }
        }
        for publication in self.knowledge_publications.values() {
            let grant = self
                .observation_grants
                .get(&publication.grant)
                .ok_or_else(|| invalid("force knowledge publication grant is unavailable"))?;
            let mut detached = publication.clone();
            let recorded = std::mem::take(&mut detached.semantic_digest);
            let mut source_versions = publication.source_versions.clone();
            source_versions.sort();
            source_versions.dedup();
            if publication.provider_revision == 0
                || publication.available_at < publication.observed_at
                || publication.source_versions.is_empty()
                || source_versions != publication.source_versions
                || grant.holder != publication.holder
                || grant.force != publication.force
                || recorded
                    != canonical_hash("canwu.force-supply.knowledge-publication.v1", &detached)?
            {
                return Err(invalid(
                    "force knowledge publication is forged or not bound to its grant",
                ));
            }
        }
        let mut expected_temporal = BTreeMap::<
            ForceObservationScopeV1,
            BTreeMap<ForceObservationTemporalKeyV1, ForceKnowledgePublicationId>,
        >::new();
        for publication in self.knowledge_publications.values() {
            expected_temporal
                .entry(observation_temporal_scope_key(
                    &publication.holder,
                    &publication.force,
                    &publication.fact,
                ))
                .or_default()
                .insert(
                    ForceObservationTemporalKeyV1 {
                        observed_at: publication.observed_at,
                        provider_revision: publication.provider_revision,
                        publication: publication.id.clone(),
                    },
                    publication.id.clone(),
                );
        }
        if self.observation_temporal_index != expected_temporal
            || self
                .observation_temporal_index
                .values()
                .any(|heads| heads.len() > MAX_TEMPORAL_HEADS_PER_SCOPE)
        {
            return Err(invalid(
                "force observation temporal index is stale, forged, or over budget",
            ));
        }
        for (sequence, receipt) in &self.terminal_receipts {
            let mut detached = receipt.clone();
            let recorded = std::mem::take(&mut detached.semantic_digest);
            let authoritative_certificate = self
                .completion_leases
                .certificate(&receipt.completion_certificate.acquisition);
            let acquisition = self
                .completion_leases
                .acquisitions
                .get(&receipt.completion_certificate.acquisition)
                .ok_or_else(|| invalid("force terminal receipt completion lease is unavailable"))?;
            let mut detached_consequence = receipt.consequence.clone();
            let consequence_digest = std::mem::take(&mut detached_consequence.semantic_digest);
            if sequence != &receipt.sequence
                || receipt.sequence >= self.next_terminal_sequence
                || receipt.externality_outcome.is_some()
                    != receipt.externality_outcome_source.is_some()
                || receipt.resource_outcome != receipt.consequence.attribution.resource_outcome
                || receipt.intent != receipt.consequence.intent
                || consequence_digest
                    != canonical_hash("canwu.force-supply.consequence.v1", &detached_consequence)?
                || authoritative_certificate != Some(&receipt.completion_certificate)
                || acquisition.state != canwu_resource::CompletionLeaseAcquisitionStateV1::Released
                || recorded != canonical_hash("canwu.force-supply.terminal-receipt.v1", &detached)?
            {
                return Err(invalid("force terminal receipt is forged"));
            }
        }
        if let Some(continuation) = &self.terminal_continuation
            && (continuation.through_sequence >= self.next_terminal_sequence
                || continuation.compacted_receipts == 0
                || continuation.chain_digest.len() != 64)
        {
            return Err(invalid("force terminal continuation is invalid"));
        }
        let mut archive_head = self.archive_head.clone();
        let head_digest = std::mem::take(&mut archive_head.semantic_digest);
        if head_digest
            != canonical_hash(
                "canwu.force-supply-reference.archive-head.v1",
                &archive_head,
            )?
            || self.archive_retention_handles.len() > 64
            || self.archive_maintenance_receipts.len() > 8_192
        {
            return Err(invalid(
                "force archive head or hot maintenance bounds are invalid",
            ));
        }
        for (id, handle) in &self.archive_retention_handles {
            let mut detached = handle.clone();
            let recorded = std::mem::take(&mut detached.semantic_digest);
            if id != &handle.id
                || recorded
                    != canonical_hash(
                        "canwu.force-supply-reference.archive-retention.v1",
                        &detached,
                    )?
            {
                return Err(invalid("force archive retention handle is forged"));
            }
        }
        for (sequence, receipt) in &self.archive_maintenance_receipts {
            let mut detached = receipt.clone();
            let recorded = std::mem::take(&mut detached.semantic_digest);
            if sequence != &receipt.sequence
                || recorded
                    != canonical_hash(
                        "canwu.force-supply-reference.archive-maintenance-receipt.v1",
                        &detached,
                    )?
            {
                return Err(invalid("force archive maintenance receipt is forged"));
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    pub fn apply_operation(
        &mut self,
        envelope: &ForceCommandEnvelopeV1,
        now: SimTime,
    ) -> Result<(), CanwuError> {
        let requirement_scoped_intent = matches!(
            &envelope.operation,
            ForceOperationV1::SubmitConsumptionIntent { intent }
                if intent.expected_force_runtime_revision == envelope.expected_runtime_revision
                    && envelope.expected_runtime_revision <= self.revision
        );
        if envelope.expected_runtime_revision != self.revision && !requirement_scoped_intent {
            return Err(CanwuError::new(
                ErrorCode::DomainRecordVersionConflict,
                "force-supply runtime revision is stale",
            ));
        }
        let completion_only = matches!(envelope.operation, ForceOperationV1::Completion { .. });
        match &envelope.operation {
            ForceOperationV1::RegisterForce { force } => {
                if force.holder != envelope.holder
                    || !self.profiles.contains_key(&force.profile)
                    || force.revision == 0
                {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidAuthority,
                        "force registration holder or profile is invalid",
                    ));
                }
                let profile = &self.profiles[&force.profile];
                if force.due.len() != profile.requirements.len()
                    || profile.requirements.iter().any(|requirement| {
                        force.due.get(&requirement.id).is_none_or(|due| {
                            due.requirement != requirement.id
                                || due.persisted_remainder_minutes
                                    >= cadence_interval(requirement).unwrap_or(u64::MAX)
                        })
                    })
                {
                    return Err(invalid(
                        "force registration must provide one canonical due state per requirement",
                    ));
                }
                if self
                    .forces
                    .insert(force.id.clone(), force.clone())
                    .is_some()
                {
                    return Err(CanwuError::new(
                        ErrorCode::DuplicateDomainRecord,
                        "force already exists",
                    ));
                }
                for due in force.due.values() {
                    self.due_index
                        .entry(due.next_due)
                        .or_default()
                        .insert((force.id.clone(), due.requirement.clone()));
                }
            }
            ForceOperationV1::SelectSupplyPosture {
                force,
                posture,
                decision,
            } => {
                require_force_holder(self, force, &envelope.holder)?;
                if posture.is_empty()
                    || posture.len() > MAX_IDENTIFIER_BYTES
                    || decision.option_id != *posture
                {
                    return Err(invalid("force supply posture is invalid"));
                }
                let force = self
                    .forces
                    .get_mut(force)
                    .ok_or_else(|| invalid("force supply posture target is unavailable"))?;
                force.supply_posture.clone_from(posture);
                force.revision = force
                    .revision
                    .checked_add(1)
                    .ok_or_else(|| invalid("force revision overflowed"))?;
            }
            ForceOperationV1::GrantObservation { grant } => {
                if grant.confidence_per_mille > 1_000
                    || self
                        .forces
                        .get(&grant.force)
                        .is_none_or(|force| force.holder != envelope.holder)
                {
                    return Err(invalid("force observation grant is invalid"));
                }
                if self
                    .observation_grants
                    .insert(grant.id.clone(), grant.clone())
                    .is_some()
                {
                    return Err(CanwuError::new(
                        ErrorCode::DuplicateDomainRecord,
                        "force observation grant already exists",
                    ));
                }
            }
            ForceOperationV1::RecordSupplyObservation { force, observation } => {
                require_force_holder(self, force, &envelope.holder)?;
                let mut source_versions = observation.source_versions.clone();
                source_versions.sort();
                source_versions.dedup();
                if observation.known_stock_low > observation.known_stock_high
                    || observation.confidence_per_mille > 1_000
                    || observation.observed_at > now
                    || observation.source_versions.is_empty()
                    || source_versions != observation.source_versions
                {
                    return Err(invalid("force supply observation interval is invalid"));
                }
                self.observations
                    .entry(force.clone())
                    .or_default()
                    .insert(observation.requirement.clone(), observation.clone());
                self.publish_recorded_observation(
                    &envelope.holder,
                    force,
                    observation.clone(),
                    now,
                )?;
            }
            ForceOperationV1::SubmitConsumptionIntent { intent } => {
                self.submit_intent(&envelope.holder, intent, now)?;
            }
            ForceOperationV1::Completion { operation } => {
                self.apply_completion_operation(&envelope.holder, operation)?;
            }
            ForceOperationV1::FinalizeRequisition { saga } => {
                self.finalize_saga(&envelope.holder, saga, now)?;
            }
        }
        if !completion_only {
            self.revision = self
                .revision
                .checked_add(1)
                .ok_or_else(|| invalid("force-supply revision overflowed"))?;
        }
        self.validate()
    }

    #[allow(clippy::too_many_lines)]
    fn apply_completion_operation(
        &mut self,
        holder: &KnowledgeHolderRef,
        operation: &ForceCompletionOperationV1,
    ) -> Result<(), CanwuError> {
        match operation {
            ForceCompletionOperationV1::Acquire(request) => {
                if &request.holder != holder
                    || request.operation_namespace != "canwu.force-supply-reference:requisition"
                {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidAuthority,
                        "force completion acquisition is not holder/namespace authorized",
                    ));
                }
                let active_completion = self
                    .completion_leases
                    .acquisitions
                    .values()
                    .filter(|value| {
                        !matches!(
                            value.state,
                            canwu_resource::CompletionLeaseAcquisitionStateV1::Released
                                | canwu_resource::CompletionLeaseAcquisitionStateV1::Expired
                        )
                    })
                    .count();
                if self
                    .terminal_receipts
                    .len()
                    .checked_add(self.intents.len())
                    .and_then(|value| value.checked_add(active_completion))
                    .is_none_or(|reserved| reserved >= self.limits.max_terminal_receipts)
                {
                    return Err(invalid(
                        "archive_backpressure: force terminal report capacity is unavailable",
                    ));
                }
                let acquisition = self
                    .completion_leases
                    .request_acquisition(&self.completion_run_budget, request.clone())
                    .map_err(resource_error)?;
                self.completion_leases
                    .record_receipt(
                        acquisition.operation_key,
                        acquisition.id,
                        None,
                        CompletionLeaseReceiptActionV1::Requested,
                        None,
                    )
                    .map_err(resource_error)?;
            }
            ForceCompletionOperationV1::Grant(request) => {
                let acquisition = self
                    .completion_leases
                    .acquisitions
                    .get(&request.acquisition)
                    .ok_or_else(|| invalid("force completion acquisition is unavailable"))?;
                if &acquisition.holder != holder {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidAuthority,
                        "force completion grant is not holder-authorized",
                    ));
                }
                let grant = self
                    .completion_leases
                    .grant_capacity(&self.completion_run_budget, request.clone())
                    .map_err(resource_error)?;
                self.completion_leases
                    .record_receipt(
                        grant.operation_key,
                        grant.acquisition,
                        Some(grant.id),
                        CompletionLeaseReceiptActionV1::Granted,
                        None,
                    )
                    .map_err(resource_error)?;
            }
            ForceCompletionOperationV1::Prepare(request) => {
                require_completion_holder(self, holder, &request.acquisition)?;
                let grant = self
                    .completion_leases
                    .prepare_capacity(request.clone())
                    .map_err(resource_error)?;
                self.refresh_completion_acquisition_state(&request.acquisition, false)?;
                let (action, reason) = if grant.state == CompletionGrantStateV1::Prepared {
                    (CompletionLeaseReceiptActionV1::Prepared, None)
                } else {
                    (
                        CompletionLeaseReceiptActionV1::Rejected,
                        grant.rejection.clone(),
                    )
                };
                self.completion_leases
                    .record_receipt(
                        grant.operation_key,
                        grant.acquisition,
                        Some(grant.id),
                        action,
                        reason,
                    )
                    .map_err(resource_error)?;
            }
            ForceCompletionOperationV1::AcknowledgeExternalParticipant {
                owner_source: _,
                participant,
            } => {
                self.acknowledge_external_participant(holder, participant.clone())?;
            }
            ForceCompletionOperationV1::Activate(request) => {
                require_completion_holder(self, holder, &request.acquisition)?;
                let certificate = self.activate_completion_with_external(request)?;
                self.completion_leases
                    .record_receipt(
                        certificate.operation_key,
                        certificate.acquisition,
                        Some(request.grant.clone()),
                        CompletionLeaseReceiptActionV1::Activated,
                        None,
                    )
                    .map_err(resource_error)?;
            }
            ForceCompletionOperationV1::Abort(request) => {
                if &request.holder != holder {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidAuthority,
                        "force completion abort holder differs",
                    ));
                }
                let acquisition = self
                    .completion_leases
                    .acquisitions
                    .get(&request.acquisition)
                    .cloned()
                    .ok_or_else(|| invalid("force completion acquisition is unavailable"))?;
                let result = self
                    .completion_leases
                    .abort(holder, &request.acquisition, request.expected_revision)
                    .map_err(resource_error)?;
                self.completion_leases
                    .record_receipt(
                        acquisition.operation_key,
                        acquisition.id,
                        None,
                        CompletionLeaseReceiptActionV1::Aborted,
                        Some(result.to_owned()),
                    )
                    .map_err(resource_error)?;
            }
            ForceCompletionOperationV1::Release(request) => {
                require_completion_holder(self, holder, &request.acquisition)?;
                let operation_key = self.completion_leases.acquisitions[&request.acquisition]
                    .operation_key
                    .clone();
                self.completion_leases
                    .release_capacity(request)
                    .map_err(resource_error)?;
                self.completion_leases
                    .record_receipt(
                        operation_key,
                        request.acquisition.clone(),
                        Some(request.grant.clone()),
                        CompletionLeaseReceiptActionV1::Released,
                        Some(request.reason.clone()),
                    )
                    .map_err(resource_error)?;
            }
            ForceCompletionOperationV1::Expire(request) => {
                let expired = self
                    .completion_leases
                    .expire_capacity(request)
                    .map_err(resource_error)?;
                for grant in expired {
                    let snapshot = self.completion_leases.grants[&grant].clone();
                    self.completion_leases
                        .record_receipt(
                            snapshot.operation_key,
                            snapshot.acquisition,
                            Some(snapshot.id),
                            CompletionLeaseReceiptActionV1::Expired,
                            Some("preactivation_expired".to_owned()),
                        )
                        .map_err(resource_error)?;
                }
            }
            ForceCompletionOperationV1::ConsumeParticipant {
                acquisition,
                owner_plugin,
            } => {
                require_completion_holder(self, holder, acquisition)?;
                self.consume_completion_participant(acquisition, owner_plugin)?;
            }
            ForceCompletionOperationV1::Complete { acquisition } => {
                require_completion_holder(self, holder, acquisition)?;
                self.complete_completion_acquisition(acquisition)?;
            }
        }
        Ok(())
    }

    pub fn completion_status(
        &self,
        holder: &KnowledgeHolderRef,
        acquisition: &CompletionLeaseAcquisitionId,
    ) -> Result<CompletionLeaseStatusDtoV1, CanwuError> {
        let mut status = self
            .completion_leases
            .status_for(holder, acquisition)
            .map_err(resource_error)?;
        if let Some(participants) = self.completion_participant_grants.get(acquisition) {
            for (owner, participant) in participants {
                status
                    .grant_states
                    .insert(owner.clone(), participant.grant.state);
                status
                    .exact_grant_versions
                    .insert(owner.clone(), participant.grant.revision);
                status
                    .expiry_boundaries
                    .insert(owner.clone(), participant.grant.expires_after_boundary);
                if let Some(deadline) = participant.grant.activation_deadline_boundary {
                    status.activation_deadlines.insert(owner.clone(), deadline);
                }
            }
        }
        Ok(status)
    }

    fn consume_completion_participant(
        &mut self,
        acquisition: &CompletionLeaseAcquisitionId,
        owner_plugin: &str,
    ) -> Result<(), CanwuError> {
        let certificate = self
            .completion_leases
            .certificate(acquisition)
            .cloned()
            .ok_or_else(|| invalid("force completion certificate is unavailable"))?;
        let acquisition_state = self
            .completion_leases
            .acquisitions
            .get(acquisition)
            .cloned()
            .ok_or_else(|| invalid("force completion acquisition is unavailable"))?;
        if owner_plugin != PLUGIN_NAME {
            return Err(invalid(
                "force coordinator cannot consume capacity owned by another plugin",
            ));
        }
        let grant_id = acquisition_state
            .grants
            .get(owner_plugin)
            .cloned()
            .ok_or_else(|| invalid("force completion participant grant is unavailable"))?;
        let targets = self.completion_leases.grants[&grant_id]
            .target_versions
            .clone();
        if self.completion_leases.grants[&grant_id].state == CompletionGrantStateV1::Consumed {
            return Ok(());
        }
        self.completion_leases
            .consume_authoritative_grant(
                &certificate,
                &grant_id,
                certificate.eligibility_time,
                &targets,
            )
            .map_err(resource_error)?;
        self.completion_leases
            .record_receipt(
                certificate.operation_key,
                certificate.acquisition,
                Some(grant_id),
                CompletionLeaseReceiptActionV1::Consumed,
                None,
            )
            .map_err(resource_error)?;
        Ok(())
    }

    fn complete_completion_acquisition(
        &mut self,
        acquisition: &CompletionLeaseAcquisitionId,
    ) -> Result<(), CanwuError> {
        let snapshot = self
            .completion_leases
            .acquisitions
            .get(acquisition)
            .cloned()
            .ok_or_else(|| invalid("force completion acquisition is unavailable"))?;
        let external = self
            .completion_participant_grants
            .get(acquisition)
            .cloned()
            .unwrap_or_default();
        if snapshot
            .expected_participants
            .iter()
            .filter(|owner| owner.as_str() != PLUGIN_NAME)
            .any(|owner| {
                external.get(owner).is_none_or(|participant| {
                    participant.grant.state != CompletionGrantStateV1::Completed
                })
            })
        {
            return Err(invalid(
                "force completion cannot close before every external owner completed",
            ));
        }
        let first_grant = snapshot
            .grants
            .values()
            .next()
            .cloned()
            .ok_or_else(|| invalid("force completion acquisition has no grants"))?;
        self.completion_leases
            .complete_grant(acquisition, &first_grant)
            .map_err(resource_error)?;
        self.completion_leases
            .record_receipt(
                snapshot.operation_key,
                snapshot.id,
                Some(first_grant),
                CompletionLeaseReceiptActionV1::Completed,
                None,
            )
            .map_err(resource_error)?;
        Ok(())
    }

    fn publish_recorded_observation(
        &mut self,
        holder: &KnowledgeHolderRef,
        force: &ReferenceForceId,
        observation: ForceSupplyObservationV1,
        available_at: SimTime,
    ) -> Result<(), CanwuError> {
        let grant = self
            .observation_grants
            .values()
            .find(|grant| &grant.holder == holder && &grant.force == force)
            .cloned()
            .ok_or_else(|| invalid("recorded observation has no exact holder grant"))?;
        self.insert_publication(
            &grant,
            observation.observed_at,
            available_at,
            observation.source_versions.clone(),
            ForceKnowledgeFactV1::SupplyObservation(observation),
        )
    }

    #[allow(clippy::needless_pass_by_value)]
    fn publish_force_fact(
        &mut self,
        force: &ReferenceForceId,
        observed_at: SimTime,
        source_versions: Vec<DomainRecordVersionRef>,
        fact: ForceKnowledgeFactV1,
    ) -> Result<(), CanwuError> {
        let grants: Vec<_> = self
            .observation_grants
            .values()
            .filter(|grant| &grant.force == force)
            .cloned()
            .collect();
        for grant in grants {
            let delay = i64::try_from(grant.observation_delay_minutes)
                .map_err(|_| invalid("force observation delay exceeds simulation time"))?;
            let available_at = observed_at
                .checked_add(canwu_api::SimDuration::minutes(delay))
                .ok_or_else(|| invalid("force knowledge publication time overflowed"))?;
            self.insert_publication(
                &grant,
                observed_at,
                available_at,
                source_versions.clone(),
                fact.clone(),
            )?;
        }
        Ok(())
    }

    fn insert_publication(
        &mut self,
        grant: &ForceObserverGrantV1,
        observed_at: SimTime,
        available_at: SimTime,
        mut source_versions: Vec<DomainRecordVersionRef>,
        fact: ForceKnowledgeFactV1,
    ) -> Result<(), CanwuError> {
        if self.knowledge_publications.len() >= self.limits.max_observer_grants {
            return Err(invalid("force knowledge publication capacity is exhausted"));
        }
        let provider_revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("force-supply revision overflowed"))?;
        let digest = canonical_hash(
            "canwu.force-supply.knowledge-publication-id.v1",
            &(
                &grant.id,
                observed_at,
                available_at,
                provider_revision,
                &fact,
            ),
        )?;
        let id = ForceKnowledgePublicationId::new(format!(
            "canwu.force-supply-reference:knowledge:{digest}"
        ))?;
        source_versions.sort();
        source_versions.dedup();
        if source_versions.is_empty() {
            return Err(invalid(
                "force knowledge publication requires exact source versions",
            ));
        }
        let mut publication = ForceKnowledgePublicationV1 {
            id: id.clone(),
            grant: grant.id.clone(),
            holder: grant.holder.clone(),
            force: grant.force.clone(),
            observed_at,
            available_at,
            provider_revision,
            source_versions,
            fact,
            semantic_digest: String::new(),
        };
        publication.semantic_digest =
            canonical_hash("canwu.force-supply.knowledge-publication.v1", &publication)?;
        let temporal_scope = observation_temporal_scope_key(
            &publication.holder,
            &publication.force,
            &publication.fact,
        );
        let temporal_key = ForceObservationTemporalKeyV1 {
            observed_at,
            provider_revision,
            publication: id.clone(),
        };
        let temporal = self
            .observation_temporal_index
            .entry(temporal_scope)
            .or_default();
        if temporal.len() >= MAX_TEMPORAL_HEADS_PER_SCOPE {
            return Err(invalid(
                "archive_backpressure: force observation temporal history is full",
            ));
        }
        temporal.insert(temporal_key, id.clone());
        self.knowledge_publications.insert(id, publication);
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn submit_intent(
        &mut self,
        holder: &KnowledgeHolderRef,
        intent: &ForceConsumptionIntent,
        now: SimTime,
    ) -> Result<(), CanwuError> {
        let force = self
            .forces
            .get(&intent.force)
            .cloned()
            .ok_or_else(|| invalid("force intent references an unavailable force"))?;
        if &force.holder != holder
            || !force.active
            || intent.expected_force_runtime_revision > self.revision
            || force.revision < intent.expected_force_revision
            || force.blocked_by_active_requisition.is_some()
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidAuthority,
                "force cannot admit this consumption intent",
            ));
        }
        let profile = &self.profiles[&force.profile];
        if now < profile.effective_from
            || profile.effective_until.is_some_and(|until| now >= until)
            || intent.due_at != now
        {
            return Err(invalid(
                "force profile or requirement is not due at admission",
            ));
        }
        let due = force
            .due
            .get(&intent.requirement)
            .ok_or_else(|| invalid("force requirement has no persisted due state"))?;
        let requirement = profile
            .requirements
            .iter()
            .find(|requirement| requirement.id == intent.requirement)
            .ok_or_else(|| invalid("force requirement is unavailable"))?;
        let (scheduled_due, due_count, _) = derive_due(requirement, due, now)?;
        if intent.scheduled_due != scheduled_due
            || intent.completion_certificate.eligibility_time != now
        {
            return Err(invalid(
                "force intent does not cite the exact scheduled due",
            ));
        }
        if self.intents.contains_key(&intent.id)
            || self
                .intents
                .values()
                .any(|value| value.resource_operation_key == intent.resource_operation_key)
            || self.terminal_receipts.values().any(|receipt| {
                receipt.intent == intent.id
                    || receipt.resource_outcome.operation_key == intent.resource_operation_key
            })
        {
            return Err(CanwuError::new(
                ErrorCode::IdempotencyConflict,
                "force intent identity or resource operation key was reused",
            ));
        }
        if self.intents.values().any(|active| {
            active.force == intent.force
                && active.requirement == intent.requirement
                && matches!(
                    active.status,
                    ForceConsumptionIntentStatus::PendingResourceConsumption
                        | ForceConsumptionIntentStatus::ConsequenceCommitted
                )
        }) {
            return Err(CanwuError::new(
                ErrorCode::DomainRecordVersionConflict,
                "force requirement already has an active consumption lifecycle",
            ));
        }
        if self
            .terminal_receipts
            .len()
            .checked_add(self.intents.len())
            .is_none_or(|reserved| reserved >= self.limits.max_terminal_receipts)
        {
            return Err(invalid(
                "archive_backpressure: force terminal report capacity is unavailable",
            ));
        }
        if intent.due_count != 0 || intent.requested_quantity != 0 {
            return Err(invalid(
                "force due count and requested quantity are package-derived fields",
            ));
        }
        let mut sealed = intent.clone();
        sealed.due_count = due_count;
        sealed.requested_quantity = requirement
            .quantity_per_due
            .checked_mul(u64::from(due_count))
            .ok_or_else(|| invalid("force due quantity overflowed"))?;
        sealed.semantic_digest = String::new();
        sealed.semantic_digest = canonical_hash("canwu.force-supply.intent.v1", &sealed)?;
        if intent.status != ForceConsumptionIntentStatus::PendingResourceConsumption {
            return Err(invalid("force intent is not canonically sealed"));
        }
        validate_stock_custody_binding(&sealed.stock_custody, &sealed.allocation, &force.holder)?;
        if self
            .completion_leases
            .certificate(&intent.completion_certificate.acquisition)
            != Some(&intent.completion_certificate)
        {
            return Err(invalid(
                "force intent certificate was not generated by the force coordinator",
            ));
        }
        self.consume_completion_participant(
            &intent.completion_certificate.acquisition,
            PLUGIN_NAME,
        )?;
        validate_completion_participants(
            self,
            &sealed,
            &force,
            self.completion_leases
                .acquisitions
                .get(&intent.completion_certificate.acquisition)
                .ok_or_else(|| invalid("force completion acquisition is unavailable"))?,
        )?;
        if let Some(policy) = &sealed.requisition_policy {
            if !self.requisition_policies.contains_key(policy) {
                return Err(invalid("requisition policy is unavailable"));
            }
            let saga_id =
                RequisitionSagaId::new(format!("canwu.force-supply-reference:saga:{}", sealed.id))?;
            self.sagas.insert(
                saga_id.clone(),
                RequisitionSagaV1 {
                    id: saga_id.clone(),
                    force: sealed.force.clone(),
                    intent: sealed.id.clone(),
                    stage: RequisitionSagaStage::PendingResourceConsumption,
                    consequence: None,
                    externality_intent: None,
                    externality_outcome: None,
                    externality_outcome_source: None,
                    recoverable_blocker: None,
                    final_ack_digest: None,
                },
            );
            self.forces
                .get_mut(&sealed.force)
                .expect("force checked")
                .blocked_by_active_requisition = Some(saga_id);
        }
        self.intents.insert(sealed.id.clone(), sealed);
        Ok(())
    }

    #[allow(clippy::missing_panics_doc, clippy::too_many_lines)]
    pub fn acknowledge_resource_outcome(
        &mut self,
        packet: &ResourceOutcomePacketV1,
        settlement: &ForceResourceSettlementEvidenceV1,
        now: SimTime,
    ) -> Result<(), CanwuError> {
        let authoritative_outcome = &settlement.outcome;
        validate_resource_settlement(settlement)?;
        if packet.authoritative_resource_state != settlement.provider_state {
            return Err(invalid(
                "resource settlement packet does not cite the exact provider state",
            ));
        }
        let Some(intent) = self.intents.get(&packet.intent).cloned() else {
            let receipt = self
                .terminal_receipts
                .values()
                .find(|receipt| receipt.intent == packet.intent)
                .ok_or_else(|| invalid("resource outcome intent is unavailable"))?;
            return if receipt.resource_outcome == *authoritative_outcome
                && receipt.resource_outcome_source == packet.authoritative_resource_state
                && receipt.resource_outcome.id == packet.outcome_id
            {
                Ok(())
            } else {
                Err(CanwuError::new(
                    ErrorCode::IdempotencyConflict,
                    "archived force intent received a different resource outcome",
                ))
            };
        };
        if intent.status != ForceConsumptionIntentStatus::PendingResourceConsumption {
            return if intent.resource_outcome.as_ref() == Some(authoritative_outcome) {
                Ok(())
            } else {
                Err(CanwuError::new(
                    ErrorCode::IdempotencyConflict,
                    "force intent received a different resource outcome",
                ))
            };
        }
        if authoritative_outcome.id != packet.outcome_id
            || authoritative_outcome.operation_key != intent.resource_operation_key
            || !matches!(
                authoritative_outcome.status,
                ResourceOperationStatus::Applied | ResourceOperationStatus::Duplicate
            )
            || authoritative_outcome.quantity > intent.requested_quantity
            || authoritative_outcome.semantic_digest.is_empty()
            || authoritative_outcome.result_ref
                != Some(ResourceRecordRefV1::Consumption(
                    intent.consumption_id.clone(),
                ))
            || settlement.consumption.id != intent.consumption_id
            || settlement.consumption.account != intent.stock_custody.destination_account
            || settlement.consumption.allocation_leg != intent.allocation.id
            || settlement.consumption.quantity != authoritative_outcome.quantity
            || settlement.fulfillment.operation_key != intent.resource_operation_key
            || settlement.fulfillment.consumed_quantity != authoritative_outcome.quantity
            || settlement.fulfillment.remainder != authoritative_outcome.remainder
            || settlement.destination_custodian != intent.stock_custody.destination_custodian
            || settlement.destination_account_revision <= intent.allocation.account_revision
            || settlement.accepted_transfer != intent.stock_custody.accepted_transfer
        {
            return Err(invalid(
                "resource outcome/consumption/fulfillment does not exactly settle authorized local stock",
            ));
        }
        let force_before = self.forces[&intent.force].clone();
        let profile = &self.profiles[&force_before.profile];
        let requirement = profile
            .requirements
            .iter()
            .find(|value| value.id == intent.requirement)
            .ok_or_else(|| invalid("force intent requirement disappeared"))?;
        if force_before
            .due
            .get(&requirement.id)
            .is_none_or(|due| due.next_due != intent.scheduled_due)
        {
            return Err(CanwuError::new(
                ErrorCode::DomainRecordVersionConflict,
                "force requirement cadence changed before its resource consequence",
            ));
        }
        let shortage = intent
            .requested_quantity
            .saturating_sub(authoritative_outcome.quantity);
        let factor = i16::from(shortage > requirement.consequence.tolerance_quantity);
        let consequence_id = ForceConsequenceId::new(format!(
            "canwu.force-supply-reference:consequence:{}",
            intent.id
        ))?;
        let mut consequence = ForceConsequenceRecord {
            id: consequence_id.clone(),
            force: intent.force.clone(),
            force_revision_before: force_before.revision,
            force_revision_after: force_before
                .revision
                .checked_add(1)
                .ok_or_else(|| invalid("force revision overflowed"))?,
            intent: intent.id.clone(),
            attribution: ShortageAttributionV1 {
                requirement: requirement.id.clone(),
                kind: requirement.kind,
                requested: intent.requested_quantity,
                consumed: authoritative_outcome.quantity,
                shortage,
                resource_outcome: authoritative_outcome.clone(),
                consumption: settlement.consumption.clone(),
                fulfillment: settlement.fulfillment.clone(),
                stock_custody: intent.stock_custody.clone(),
            },
            readiness_delta_per_mille: requirement.consequence.readiness_delta_per_mille * factor,
            fatigue_delta_per_mille: requirement.consequence.fatigue_delta_per_mille * factor,
            cohesion_delta_per_mille: requirement.consequence.cohesion_delta_per_mille * factor,
            disease_delta_per_mille: requirement.consequence.disease_delta_per_mille * factor,
            desertion_delta_per_mille: requirement.consequence.desertion_delta_per_mille * factor,
            committed_at: now,
            semantic_digest: String::new(),
        };
        consequence.semantic_digest =
            canonical_hash("canwu.force-supply.consequence.v1", &consequence)?;
        let force = self.forces.get_mut(&intent.force).expect("force checked");
        force.readiness_per_mille = apply_delta(
            force.readiness_per_mille,
            consequence.readiness_delta_per_mille,
        );
        force.fatigue_per_mille =
            apply_delta(force.fatigue_per_mille, consequence.fatigue_delta_per_mille);
        force.cohesion_per_mille = apply_delta(
            force.cohesion_per_mille,
            consequence.cohesion_delta_per_mille,
        );
        force.disease_per_mille =
            apply_delta(force.disease_per_mille, consequence.disease_delta_per_mille);
        force.desertion_per_mille = apply_delta(
            force.desertion_per_mille,
            consequence.desertion_delta_per_mille,
        );
        force.revision = consequence.force_revision_after;
        let old_due = force
            .due
            .get(&requirement.id)
            .ok_or_else(|| invalid("force due state disappeared"))?
            .next_due;
        advance_due(force, requirement, intent.due_count, now)?;
        if let Some(ids) = self.due_index.get_mut(&old_due) {
            ids.remove(&(intent.force.clone(), requirement.id.clone()));
        }
        self.due_index.retain(|_, ids| !ids.is_empty());
        let next_due = force.due[&requirement.id].next_due;
        self.due_index
            .entry(next_due)
            .or_default()
            .insert((intent.force.clone(), requirement.id.clone()));
        self.consequences
            .insert(consequence_id.clone(), consequence);
        let intent_mut = self.intents.get_mut(&intent.id).expect("intent checked");
        intent_mut.resource_outcome = Some(authoritative_outcome.clone());
        intent_mut.resource_outcome_source = Some(packet.authoritative_resource_state.clone());
        intent_mut.consequence = Some(consequence_id.clone());
        if let Some(policy_id) = &intent.requisition_policy {
            let saga_id = force
                .blocked_by_active_requisition
                .clone()
                .ok_or_else(|| invalid("requisition force lost its active saga lock"))?;
            let policy = &self.requisition_policies[policy_id];
            let saga = self.sagas.get_mut(&saga_id).expect("saga checked");
            saga.consequence = Some(consequence_id.clone());
            saga.stage = RequisitionSagaStage::ForceConsequenceCommitted;
            match policy.applicability {
                ExternalityApplicability::Required => {
                    let externality_id = ForceExternalityIntentId::new(format!(
                        "canwu.force-supply-reference:externality:{}",
                        intent.id
                    ))?;
                    let mut externality = ForceExternalityIntent {
                        id: externality_id.clone(),
                        saga: saga_id,
                        operation_key: intent.resource_operation_key.clone(),
                        force_consequence: consequence_id.clone(),
                        resource_outcome: authoritative_outcome.clone(),
                        expected_economy_target: exact_economy_target(&intent)?.clone(),
                        cooperation_delta_per_mille: policy.cooperation_delta_per_mille,
                        harvest_input_delta_per_mille: policy.harvest_input_delta_per_mille,
                        quantity: authoritative_outcome.quantity,
                        policy: policy.id.clone(),
                        semantic_digest: String::new(),
                    };
                    externality.semantic_digest =
                        canonical_hash("canwu.force-supply.externality-intent.v1", &externality)?;
                    self.externality_intents
                        .insert(externality_id.clone(), externality);
                    saga.externality_intent = Some(externality_id);
                    saga.stage = RequisitionSagaStage::ExternalityPending;
                }
                ExternalityApplicability::ExternalityNotApplicable => {
                    saga.stage = RequisitionSagaStage::ExternalityRejected;
                    saga.recoverable_blocker = Some("externality_not_applicable".to_owned());
                }
                ExternalityApplicability::ExplicitUnknown => {
                    return Err(invalid(
                        "explicit-unknown requisition externality cannot authorize behavior",
                    ));
                }
            }
            intent_mut.status = ForceConsumptionIntentStatus::ConsequenceCommitted;
        } else {
            intent_mut.status = ForceConsumptionIntentStatus::Settled;
        }
        reseal_intent(intent_mut)?;
        self.publish_force_fact(
            &intent.force,
            now,
            vec![packet.authoritative_resource_state.clone()],
            ForceKnowledgeFactV1::ShortageAttribution(
                self.consequences[&consequence_id].attribution.clone(),
            ),
        )?;
        let requisition_progress = self.forces[&intent.force]
            .blocked_by_active_requisition
            .as_ref()
            .map(|saga_id| {
                let saga = &self.sagas[saga_id];
                (saga.stage, saga.recoverable_blocker.clone())
            });
        if let Some((stage, recoverable_blocker)) = requisition_progress {
            self.publish_force_fact(
                &intent.force,
                now,
                vec![packet.authoritative_resource_state.clone()],
                ForceKnowledgeFactV1::RequisitionProgress {
                    stage,
                    latest_outcome_or_ack: Some(
                        self.consequences[&consequence_id].semantic_digest.clone(),
                    ),
                    recoverable_blocker,
                },
            )?;
        }
        if intent.requisition_policy.is_none() {
            self.archive_terminal_intent(&intent.id, None, now)?;
        }
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("force-supply revision overflowed"))?;
        self.validate()
    }

    #[allow(clippy::missing_panics_doc, clippy::too_many_lines)]
    pub fn acknowledge_externality_outcome(
        &mut self,
        packet: &ExternalityOutcomePacketV1,
        authoritative_outcome: &EconomyExternalityOutcomeVersionV1,
        now: SimTime,
    ) -> Result<(), CanwuError> {
        let mut detached_outcome = authoritative_outcome.clone();
        let recorded_outcome_digest = std::mem::take(&mut detached_outcome.semantic_digest);
        if recorded_outcome_digest
            != canonical_hash(
                "canwu.force-supply.economy-externality-outcome.v1",
                &detached_outcome,
            )?
        {
            return Err(invalid("economy externality outcome digest is forged"));
        }
        if let Some(receipt) = self
            .terminal_receipts
            .values()
            .find(|receipt| receipt.saga.as_ref() == Some(&packet.saga))
        {
            return if receipt.externality_outcome.as_ref() == Some(authoritative_outcome)
                && receipt.externality_outcome_source.as_ref()
                    == Some(&packet.authoritative_outcome)
            {
                Ok(())
            } else {
                Err(CanwuError::new(
                    ErrorCode::IdempotencyConflict,
                    "archived requisition saga received a different externality outcome",
                ))
            };
        }
        let saga_snapshot = self
            .sagas
            .get(&packet.saga)
            .cloned()
            .ok_or_else(|| invalid("externality outcome saga is unavailable"))?;
        if matches!(
            saga_snapshot.stage,
            RequisitionSagaStage::ExternalityApplied
                | RequisitionSagaStage::ExternalityRejected
                | RequisitionSagaStage::Settled
        ) {
            return if saga_snapshot.externality_outcome.as_ref() == Some(authoritative_outcome) {
                Ok(())
            } else {
                Err(CanwuError::new(
                    ErrorCode::IdempotencyConflict,
                    "requisition saga received a different externality outcome",
                ))
            };
        }
        if saga_snapshot.stage != RequisitionSagaStage::ExternalityPending
            || saga_snapshot.externality_intent.as_ref() != Some(&authoritative_outcome.intent)
            || authoritative_outcome.semantic_digest.is_empty()
        {
            return Err(invalid(
                "economy externality outcome does not exactly match the pending saga",
            ));
        }
        let externality_intent = self
            .externality_intents
            .get(&authoritative_outcome.intent)
            .ok_or_else(|| invalid("pending externality intent is unavailable"))?;
        if authoritative_outcome.expected_target != externality_intent.expected_economy_target {
            return Err(invalid(
                "economy externality outcome targets a different exact economy record version",
            ));
        }
        let saga = self
            .sagas
            .get_mut(&packet.saga)
            .expect("externality saga was checked");
        saga.stage = match authoritative_outcome.disposition {
            ExternalityOutcomeDisposition::Applied => RequisitionSagaStage::ExternalityApplied,
            ExternalityOutcomeDisposition::Rejected
            | ExternalityOutcomeDisposition::NotApplicable => {
                RequisitionSagaStage::ExternalityRejected
            }
        };
        saga.recoverable_blocker
            .clone_from(&authoritative_outcome.blocker);
        saga.externality_outcome = Some(authoritative_outcome.clone());
        saga.externality_outcome_source = Some(packet.authoritative_outcome.clone());
        let force = saga.force.clone();
        let stage = saga.stage;
        let blocker = saga.recoverable_blocker.clone();
        self.publish_force_fact(
            &force,
            now,
            vec![packet.authoritative_outcome.clone()],
            ForceKnowledgeFactV1::RequisitionProgress {
                stage,
                latest_outcome_or_ack: Some(authoritative_outcome.semantic_digest.clone()),
                recoverable_blocker: blocker,
            },
        )?;
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("force-supply revision overflowed"))?;
        self.validate()
    }

    fn finalize_saga(
        &mut self,
        holder: &KnowledgeHolderRef,
        saga_id: &RequisitionSagaId,
        now: SimTime,
    ) -> Result<(), CanwuError> {
        let Some(saga) = self.sagas.get(saga_id).cloned() else {
            let receipt = self
                .terminal_receipts
                .values()
                .find(|receipt| receipt.saga.as_ref() == Some(saga_id))
                .ok_or_else(|| invalid("requisition saga is unavailable"))?;
            require_force_holder(self, &receipt.consequence.force, holder)?;
            return Ok(());
        };
        require_force_holder(self, &saga.force, holder)?;
        if saga.stage == RequisitionSagaStage::Settled {
            return Ok(());
        }
        if !matches!(
            saga.stage,
            RequisitionSagaStage::ExternalityApplied | RequisitionSagaStage::ExternalityRejected
        ) {
            return Err(invalid(
                "requisition saga cannot settle before an externality terminal outcome",
            ));
        }
        let ack = canonical_hash("canwu.force-supply.requisition-ack.v1", &saga)?;
        let saga_mut = self.sagas.get_mut(saga_id).expect("saga checked");
        saga_mut.stage = RequisitionSagaStage::Settled;
        saga_mut.final_ack_digest = Some(ack.clone());
        self.forces
            .get_mut(&saga.force)
            .expect("force checked")
            .blocked_by_active_requisition = None;
        let intent = self.intents.get_mut(&saga.intent).expect("intent checked");
        intent.status = ForceConsumptionIntentStatus::Settled;
        reseal_intent(intent)?;
        self.publish_force_fact(
            &saga.force,
            now,
            saga.externality_outcome_source
                .iter()
                .cloned()
                .chain(
                    self.intents[&saga.intent]
                        .resource_outcome_source
                        .iter()
                        .cloned(),
                )
                .collect(),
            ForceKnowledgeFactV1::RequisitionProgress {
                stage: RequisitionSagaStage::Settled,
                latest_outcome_or_ack: Some(ack),
                recoverable_blocker: saga.recoverable_blocker,
            },
        )?;
        self.archive_terminal_intent(&saga.intent, Some(saga_id), now)?;
        Ok(())
    }

    fn archive_terminal_intent(
        &mut self,
        intent_id: &ForceConsumptionIntentId,
        saga_id: Option<&RequisitionSagaId>,
        terminal_at: SimTime,
    ) -> Result<(), CanwuError> {
        let intent = self
            .intents
            .get(intent_id)
            .cloned()
            .ok_or_else(|| invalid("terminal force intent is unavailable"))?;
        let outcome = intent
            .resource_outcome
            .clone()
            .ok_or_else(|| invalid("terminal force intent lacks its resource outcome"))?;
        let outcome_source = intent
            .resource_outcome_source
            .clone()
            .ok_or_else(|| invalid("terminal force intent lacks its provider version"))?;
        let consequence_id = intent
            .consequence
            .clone()
            .ok_or_else(|| invalid("terminal force intent lacks its consequence"))?;
        let consequence = self
            .consequences
            .get(&consequence_id)
            .cloned()
            .ok_or_else(|| invalid("terminal force consequence is unavailable"))?;
        let saga = saga_id
            .map(|id| {
                self.sagas
                    .get(id)
                    .cloned()
                    .ok_or_else(|| invalid("terminal requisition saga is unavailable"))
            })
            .transpose()?;
        let completion_certificate = intent.completion_certificate.clone();
        self.complete_completion_acquisition(&completion_certificate.acquisition)?;
        let sequence = self.next_terminal_sequence;
        self.next_terminal_sequence = self
            .next_terminal_sequence
            .checked_add(1)
            .ok_or_else(|| invalid("force terminal sequence overflowed"))?;
        let mut receipt = ForceTerminalReceiptV1 {
            sequence,
            intent: intent.id.clone(),
            saga: saga.as_ref().map(|value| value.id.clone()),
            resource_outcome: outcome,
            resource_outcome_source: outcome_source,
            externality_outcome: saga
                .as_ref()
                .and_then(|value| value.externality_outcome.clone()),
            externality_outcome_source: saga
                .as_ref()
                .and_then(|value| value.externality_outcome_source.clone()),
            consequence,
            completion_certificate,
            final_ack_digest: saga
                .as_ref()
                .and_then(|value| value.final_ack_digest.clone()),
            terminal_at,
            semantic_digest: String::new(),
        };
        receipt.semantic_digest =
            canonical_hash("canwu.force-supply.terminal-receipt.v1", &receipt)?;
        self.terminal_receipts.insert(sequence, receipt);
        self.intents.remove(intent_id);
        self.consequences.remove(&consequence_id);
        if let Some(saga) = saga {
            if let Some(externality) = saga.externality_intent {
                self.externality_intents.remove(&externality);
            }
            self.sagas.remove(&saga.id);
        }
        Ok(())
    }

    pub fn force_archive_source_root(&self) -> Result<String, CanwuError> {
        canonical_hash(
            "canwu.force-supply-reference.archive-source-root.v1",
            &(
                self.revision,
                self.next_terminal_sequence,
                &self.terminal_receipts,
                &self.completion_leases,
                &self.completion_participant_grants,
                &self.knowledge_publications,
                &self.outcomes,
                &self.archive_head,
            ),
        )
    }

    fn terminal_lifecycle_archive(
        &self,
        receipt: &ForceTerminalReceiptV1,
    ) -> Result<crate::ForceTerminalLifecycleArchiveV1, CanwuError> {
        let acquisition_id = &receipt.completion_certificate.acquisition;
        let acquisition = self
            .completion_leases
            .acquisitions
            .get(acquisition_id)
            .cloned()
            .ok_or_else(|| invalid("force terminal archive lost its completion acquisition"))?;
        if acquisition.state != CompletionLeaseAcquisitionStateV1::Released {
            return Err(invalid(
                "force terminal archive acquisition is not fully released",
            ));
        }
        let local_grants = acquisition
            .grants
            .values()
            .map(|grant_id| {
                self.completion_leases
                    .grants
                    .get(grant_id)
                    .cloned()
                    .map(|grant| (grant_id.clone(), grant))
                    .ok_or_else(|| invalid("force terminal archive lost a local grant"))
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        let certificate = self
            .completion_leases
            .certificates
            .get(acquisition_id)
            .cloned()
            .ok_or_else(|| invalid("force terminal archive lost its certificate"))?;
        let external_participants = self
            .completion_participant_grants
            .get(acquisition_id)
            .cloned()
            .unwrap_or_default();
        let lease_receipts = self
            .completion_leases
            .receipts
            .values()
            .filter(|value| &value.acquisition == acquisition_id)
            .cloned()
            .collect::<Vec<_>>();
        let mut archived = crate::ForceTerminalLifecycleArchiveV1 {
            receipt: receipt.clone(),
            acquisition,
            local_grants,
            certificate,
            external_participants,
            lease_receipts,
            semantic_digest: String::new(),
        };
        archived.semantic_digest = canonical_hash(
            "canwu.force-supply-reference.terminal-lifecycle-archive.v1",
            &archived,
        )?;
        Ok(archived)
    }

    pub fn prepare_force_archive(
        &self,
        candidate_limit: usize,
    ) -> Result<crate::PreparedForceArchiveBatchV1, CanwuError> {
        if candidate_limit == 0 || candidate_limit > crate::MAX_PACKAGE_ARCHIVE_PAGE_ENTRIES {
            return Err(invalid("force archive candidate budget is invalid"));
        }
        let mut records = Vec::new();
        for receipt in self.terminal_receipts.values() {
            let lifecycle_records = 2 + usize::from(receipt.saga.is_some());
            if records.len().saturating_add(lifecycle_records) > candidate_limit {
                break;
            }
            let lifecycle = self.terminal_lifecycle_archive(receipt)?;
            let lifecycle_digest = lifecycle.semantic_digest.clone();
            let payload = crate::ForceArchivePayloadV1::TerminalLifecycle(lifecycle);
            records.push(crate::PackageArchiveRecordV1 {
                key: crate::ForceArchiveKeyV1::TerminalIntent(receipt.intent.clone()),
                terminal_sequence: receipt.sequence,
                semantic_digest: canonical_hash(
                    "canwu.force-supply-reference.archive-record-payload.v1",
                    &payload,
                )?,
                payload,
            });
            if let Some(saga) = &receipt.saga {
                let payload = crate::ForceArchivePayloadV1::TerminalSagaAlias {
                    intent: receipt.intent.clone(),
                    lifecycle_digest: lifecycle_digest.clone(),
                };
                records.push(crate::PackageArchiveRecordV1 {
                    key: crate::ForceArchiveKeyV1::TerminalSaga(saga.clone()),
                    terminal_sequence: receipt.sequence,
                    semantic_digest: canonical_hash(
                        "canwu.force-supply-reference.archive-record-payload.v1",
                        &payload,
                    )?,
                    payload,
                });
            }
            let payload = crate::ForceArchivePayloadV1::TerminalOperationAlias {
                intent: receipt.intent.clone(),
                lifecycle_digest: lifecycle_digest.clone(),
            };
            records.push(crate::PackageArchiveRecordV1 {
                key: crate::ForceArchiveKeyV1::TerminalOperation(
                    receipt.resource_outcome.operation_key.clone(),
                ),
                terminal_sequence: receipt.sequence,
                semantic_digest: canonical_hash(
                    "canwu.force-supply-reference.archive-record-payload.v1",
                    &payload,
                )?,
                payload,
            });
        }
        for outcome in self.outcomes.values() {
            if records.len() >= candidate_limit {
                break;
            }
            let payload = crate::ForceArchivePayloadV1::OperationOutcome(outcome.clone());
            records.push(crate::PackageArchiveRecordV1 {
                key: crate::ForceArchiveKeyV1::OperationOutcome(outcome.id.clone()),
                terminal_sequence: u64::try_from(outcome.settled_at.as_minutes()).unwrap_or(0),
                semantic_digest: canonical_hash(
                    "canwu.force-supply-reference.archive-record-payload.v1",
                    &payload,
                )?,
                payload,
            });
        }
        let hot_heads = self
            .observation_temporal_index
            .values()
            .filter_map(|history| history.values().next_back())
            .cloned()
            .collect::<BTreeSet<_>>();
        for publication in self.knowledge_publications.values() {
            if records.len() >= candidate_limit {
                break;
            }
            if hot_heads.contains(&publication.id) {
                continue;
            }
            let payload = crate::ForceArchivePayloadV1::KnowledgePublication(publication.clone());
            records.push(crate::PackageArchiveRecordV1 {
                key: crate::ForceArchiveKeyV1::KnowledgePublication(publication.id.clone()),
                terminal_sequence: u64::try_from(publication.observed_at.as_minutes())
                    .unwrap_or_default(),
                semantic_digest: canonical_hash(
                    "canwu.force-supply-reference.archive-record-payload.v1",
                    &payload,
                )?,
                payload,
            });
        }
        crate::prepare_package_archive(
            crate::FORCE_ARCHIVE_DOMAIN,
            self.force_archive_source_root()?,
            &self.archive_head,
            records,
        )
    }

    #[allow(clippy::too_many_lines)]
    pub(crate) fn apply_force_archive_commit(
        &mut self,
        commit: &crate::VerifiedForceArchiveCommitV1,
    ) -> Result<crate::ForceArchiveMaintenanceReceiptV1, CanwuError> {
        if commit.retention.phase != crate::PackageArchiveRetentionPhaseV1::Verified
            || commit.retention.directory_root != commit.directory_root
            || commit.retention.expected_source_root != commit.expected_source_root
            || usize::try_from(commit.archived_records).ok() != Some(commit.selected.len())
        {
            return Err(invalid("force archive commit is forged"));
        }
        let source_matches = self.force_archive_source_root()? == commit.expected_source_root;
        let disposition = if source_matches {
            let selected = commit.selected.iter().cloned().collect::<BTreeSet<_>>();
            for key in &commit.selected {
                match key {
                    crate::ForceArchiveKeyV1::TerminalOperation(operation_key)
                        if !self.terminal_receipts.values().any(|receipt| {
                            &receipt.resource_outcome.operation_key == operation_key
                                && selected.contains(&crate::ForceArchiveKeyV1::TerminalIntent(
                                    receipt.intent.clone(),
                                ))
                        }) =>
                    {
                        return Err(invalid(
                            "force archive operation alias lacks its terminal lifecycle",
                        ));
                    }
                    crate::ForceArchiveKeyV1::TerminalSaga(saga)
                        if !self.terminal_receipts.values().any(|receipt| {
                            receipt.saga.as_ref() == Some(saga)
                                && selected.contains(&crate::ForceArchiveKeyV1::TerminalIntent(
                                    receipt.intent.clone(),
                                ))
                        }) =>
                    {
                        return Err(invalid(
                            "force archive saga alias lacks its terminal lifecycle",
                        ));
                    }
                    _ => {}
                }
            }
            for key in &commit.selected {
                match key {
                    crate::ForceArchiveKeyV1::TerminalIntent(intent_id) => {
                        let receipt = self
                            .terminal_receipts
                            .values()
                            .find(|receipt| &receipt.intent == intent_id)
                            .cloned()
                            .ok_or_else(|| {
                                invalid("force archive selected lifecycle disappeared")
                            })?;
                        let alias = crate::ForceArchiveKeyV1::TerminalOperation(
                            receipt.resource_outcome.operation_key.clone(),
                        );
                        if !selected.contains(&alias) {
                            return Err(invalid(
                                "force archive lifecycle lacks its operation-key alias",
                            ));
                        }
                        self.retire_terminal_lifecycle(&receipt)?;
                    }
                    crate::ForceArchiveKeyV1::TerminalOperation(_)
                    | crate::ForceArchiveKeyV1::TerminalSaga(_) => {}
                    crate::ForceArchiveKeyV1::OperationOutcome(operation_id) => {
                        self.outcomes.remove(operation_id).ok_or_else(|| {
                            invalid("force archive selected operation outcome disappeared")
                        })?;
                    }
                    crate::ForceArchiveKeyV1::KnowledgePublication(publication_id) => {
                        self.knowledge_publications
                            .remove(publication_id)
                            .ok_or_else(|| {
                                invalid("force archive selected knowledge publication disappeared")
                            })?;
                    }
                }
            }
            self.rebuild_derived_indexes();
            self.archive_head = crate::sealed_archive_head(
                crate::FORCE_ARCHIVE_DOMAIN,
                crate::ForceArchiveHeadStateV1 {
                    revision: self
                        .archive_head
                        .revision
                        .checked_add(1)
                        .ok_or_else(|| invalid("force archive revision overflow"))?,
                    directory_root: Some(commit.directory_root.clone()),
                    archived_record_count: self
                        .archive_head
                        .archived_record_count
                        .checked_add(u64::from(commit.archived_records))
                        .ok_or_else(|| invalid("force archive count overflow"))?,
                    semantic_digest: String::new(),
                },
            )?;
            crate::PackageArchiveMaintenanceDispositionV1::Applied
        } else {
            crate::PackageArchiveMaintenanceDispositionV1::RejectedStale
        };
        let sequence = self.next_terminal_sequence;
        self.next_terminal_sequence = sequence
            .checked_add(1)
            .ok_or_else(|| invalid("force archive receipt sequence overflow"))?;
        let mut receipt = crate::ForceArchiveMaintenanceReceiptV1 {
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
            "canwu.force-supply-reference.archive-maintenance-receipt.v1",
            &receipt,
        )?;
        let mut durable = commit.retention.clone();
        durable.phase = crate::PackageArchiveRetentionPhaseV1::DurableIngress;
        durable.semantic_digest.clear();
        durable.semantic_digest = canonical_hash(
            "canwu.force-supply-reference.archive-retention.v1",
            &durable,
        )?;
        self.archive_retention_handles
            .insert(durable.id.clone(), durable);
        self.archive_maintenance_receipts
            .insert(sequence, receipt.clone());
        Ok(receipt)
    }

    fn retire_terminal_lifecycle(
        &mut self,
        receipt: &ForceTerminalReceiptV1,
    ) -> Result<(), CanwuError> {
        let acquisition_id = &receipt.completion_certificate.acquisition;
        let acquisition = self
            .completion_leases
            .acquisitions
            .get(acquisition_id)
            .cloned()
            .ok_or_else(|| invalid("force terminal acquisition disappeared during archive"))?;
        if acquisition.state != CompletionLeaseAcquisitionStateV1::Released {
            return Err(invalid(
                "force terminal acquisition was not released before archive",
            ));
        }
        let grant_ids = acquisition
            .grants
            .values()
            .cloned()
            .collect::<BTreeSet<_>>();
        for grant_id in &grant_ids {
            self.completion_leases
                .grants
                .remove(grant_id)
                .ok_or_else(|| invalid("force terminal local grant disappeared during archive"))?;
        }
        self.completion_leases
            .target_locks
            .retain(|_, grant_id| !grant_ids.contains(grant_id));
        for due in self.completion_leases.expiry_due.values_mut() {
            due.retain(|grant_id| !grant_ids.contains(grant_id));
        }
        self.completion_leases
            .expiry_due
            .retain(|_, grants| !grants.is_empty());
        self.completion_leases
            .certificates
            .remove(acquisition_id)
            .ok_or_else(|| invalid("force terminal certificate disappeared during archive"))?;
        self.completion_leases.acquisitions.remove(acquisition_id);
        self.completion_leases
            .receipts
            .retain(|_, value| &value.acquisition != acquisition_id);
        self.completion_participant_grants.remove(acquisition_id);
        self.terminal_receipts
            .remove(&receipt.sequence)
            .ok_or_else(|| invalid("force terminal receipt disappeared during archive"))?;
        Ok(())
    }

    pub fn decision_ticket(
        &self,
        holder: &KnowledgeHolderRef,
        force_id: &ReferenceForceId,
    ) -> Result<ForceSupplyDecisionTicketV1, CanwuError> {
        let force = self
            .forces
            .get(force_id)
            .ok_or_else(|| invalid("force is unavailable"))?;
        if &force.holder != holder {
            return Err(CanwuError::new(
                ErrorCode::InvalidAuthority,
                "force ticket holder is unauthorized",
            ));
        }
        Ok(ForceSupplyDecisionTicketV1 {
            holder: holder.clone(),
            force: force.id.clone(),
            force_revision: force.revision,
            options: vec![
                ForceSupplyDecisionChoiceV1::WaitForSupply,
                ForceSupplyDecisionChoiceV1::AdvanceImmediately,
                ForceSupplyDecisionChoiceV1::RequisitionLocally,
            ],
            holder_facts_digest: canonical_hash("canwu.force-supply.ticket.v1", &(holder, force))?,
        })
    }

    #[allow(clippy::type_complexity)]
    pub fn due_requirements(
        &self,
        now: SimTime,
        candidate_limit: usize,
    ) -> Result<Vec<(ReferenceForceId, ForceRequirementId, SimTime, u16, u64)>, CanwuError> {
        if candidate_limit == 0 || candidate_limit > MAX_DUE_CANDIDATES_PER_TICK {
            return Err(CanwuError::new(
                ErrorCode::QueryBudgetExceeded,
                "force due-work candidate limit is invalid",
            ));
        }
        let candidates = self
            .due_index
            .range(..=now)
            .flat_map(|(at, values)| {
                values
                    .iter()
                    .map(move |(force, requirement)| (force.clone(), requirement.clone(), *at))
            })
            .take(candidate_limit.saturating_add(1))
            .collect::<Vec<_>>();
        if candidates.len() > candidate_limit {
            return Err(CanwuError::new(
                ErrorCode::QueryBudgetExceeded,
                "force due-work budget was exceeded",
            ));
        }
        candidates
            .into_iter()
            .map(|(force_id, requirement_id, scheduled)| {
                let force = self
                    .forces
                    .get(&force_id)
                    .ok_or_else(|| invalid("force due index references a missing force"))?;
                let profile = &self.profiles[&force.profile];
                let requirement = profile
                    .requirements
                    .iter()
                    .find(|value| value.id == requirement_id)
                    .ok_or_else(|| invalid("force due index references a missing requirement"))?;
                let due = force
                    .due
                    .get(&requirement_id)
                    .ok_or_else(|| invalid("force due state is unavailable"))?;
                let (exact_scheduled, due_count, _) = derive_due(requirement, due, now)?;
                if exact_scheduled != scheduled {
                    return Err(invalid("force due index scheduled time is stale"));
                }
                let requested_quantity = requirement
                    .quantity_per_due
                    .checked_mul(u64::from(due_count))
                    .ok_or_else(|| invalid("force due quantity overflowed"))?;
                Ok((
                    force_id,
                    requirement_id,
                    exact_scheduled,
                    due_count,
                    requested_quantity,
                ))
            })
            .collect()
    }
}

#[must_use]
pub fn resource_consumption_request(
    intent: &ForceConsumptionIntent,
    consumer_evidence: DomainRecordVersionRef,
) -> ResourceConsumptionRequestV1 {
    ResourceConsumptionRequestV1 {
        operation_key: intent.resource_operation_key.clone(),
        consumption_id: intent.consumption_id.clone(),
        allocation: intent.allocation.clone(),
        expected_account_revision: intent.allocation.account_revision,
        consumer_evidence,
        at: intent.due_at,
        completion_certificate: intent.completion_certificate.clone(),
    }
}

fn validate_profile(profile: &ForceSupplyProfileV1) -> Result<(), CanwuError> {
    if profile.revision == 0
        || profile.organization_class.trim().is_empty()
        || profile.requirements.is_empty()
        || profile.requirements.len() > MAX_REQUIREMENTS_PER_PROFILE
        || profile.content_hash.len() != 64
        || profile.coverage_resolution_digest.len() != 64
        || profile.definition_ids.is_empty()
        || profile.model_cards.is_empty()
        || profile.semantic_digest.len() != 64
        || profile
            .effective_until
            .is_some_and(|until| until <= profile.effective_from)
    {
        return Err(invalid("force-supply profile is incomplete"));
    }
    let mut resources = BTreeSet::new();
    for requirement in &profile.requirements {
        if requirement.quantity_per_due == 0
            || !resources.insert(requirement.id.clone())
            || requirement.consequence.model_card.as_str().is_empty()
            || !profile
                .model_cards
                .contains(&requirement.consequence.model_card)
            || matches!(
                requirement.cadence,
                ForceSupplyCadenceV1::FixedMinutes {
                    interval_minutes: 0
                }
            )
        {
            return Err(invalid("force-supply requirement is invalid or duplicated"));
        }
    }
    let requirement_ids: BTreeSet<_> = profile
        .requirements
        .iter()
        .map(|requirement| requirement.id.clone())
        .collect();
    if profile
        .requirement_coverage
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>()
        != requirement_ids
        || profile
            .requirement_resolution_digests
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>()
            != requirement_ids
        || profile
            .requirement_resolution_digests
            .values()
            .any(|digest| digest.len() != 64)
    {
        return Err(invalid(
            "force-supply profile does not bind exact coverage for every requirement",
        ));
    }
    let first_requirement = profile
        .requirements
        .first()
        .ok_or_else(|| invalid("force-supply profile has no requirements"))?;
    if profile.requirement_coverage[&first_requirement.id] != profile.coverage_key
        || profile.requirement_resolution_digests[&first_requirement.id]
            != profile.coverage_resolution_digest
    {
        return Err(invalid(
            "force-supply profile compatibility provenance is not its canonical first requirement",
        ));
    }
    let mut detached = profile.clone();
    let recorded = std::mem::take(&mut detached.semantic_digest);
    if recorded != canonical_hash("canwu.force-supply.profile.v1", &detached)? {
        return Err(invalid("force-supply profile semantic digest is forged"));
    }
    Ok(())
}

fn validate_policy(policy: &RequisitionPolicyV1) -> Result<(), CanwuError> {
    if policy.revision == 0
        || policy.content_hash.len() != 64
        || policy.coverage_resolution_digest.len() != 64
        || policy.definition_ids.is_empty()
        || policy.semantic_digest.len() != 64
        || policy.applicability == ExternalityApplicability::ExplicitUnknown
    {
        return Err(invalid(
            "requisition policy cannot be unsealed or explicitly unknown",
        ));
    }
    let mut detached = policy.clone();
    let recorded = std::mem::take(&mut detached.semantic_digest);
    if recorded != canonical_hash("canwu.force-supply.requisition-policy.v1", &detached)? {
        return Err(invalid("requisition policy semantic digest is forged"));
    }
    Ok(())
}

fn validate_saga_closure(
    state: &ForceSupplyStateV1,
    saga: &RequisitionSagaV1,
    intent: &ForceConsumptionIntent,
) -> Result<(), CanwuError> {
    if saga.externality_outcome.is_some() != saga.externality_outcome_source.is_some() {
        return Err(invalid(
            "requisition saga externality outcome lacks its exact provider version",
        ));
    }
    match saga.stage {
        RequisitionSagaStage::PendingResourceConsumption => {
            if saga.consequence.is_some()
                || saga.externality_intent.is_some()
                || saga.externality_outcome.is_some()
                || saga.final_ack_digest.is_some()
            {
                return Err(invalid(
                    "pending requisition saga already carries later-stage evidence",
                ));
            }
        }
        RequisitionSagaStage::ForceConsequenceCommitted => {
            if saga.consequence.is_none()
                || saga.externality_intent.is_some()
                || saga.externality_outcome.is_some()
                || saga.final_ack_digest.is_some()
            {
                return Err(invalid("force-consequence stage lacks a consequence"));
            }
        }
        RequisitionSagaStage::ExternalityPending => {
            if saga.consequence.is_none()
                || saga.externality_intent.is_none()
                || saga.externality_outcome.is_some()
                || saga.final_ack_digest.is_some()
            {
                return Err(invalid(
                    "externality-pending stage lacks exact intent closure",
                ));
            }
        }
        RequisitionSagaStage::ExternalityApplied | RequisitionSagaStage::ExternalityRejected => {
            if saga.consequence.is_none()
                || saga.externality_intent.is_none()
                || saga.externality_outcome.is_none()
                || saga.final_ack_digest.is_some()
            {
                return Err(invalid(
                    "terminal externality stage lacks exact outcome closure",
                ));
            }
        }
        RequisitionSagaStage::Settled => {
            if saga.consequence.is_none()
                || saga.final_ack_digest.as_ref().is_none_or(String::is_empty)
            {
                return Err(invalid(
                    "settled requisition lacks its final force acknowledgement",
                ));
            }
        }
    }
    if let Some(consequence) = &saga.consequence
        && (intent.consequence.as_ref() != Some(consequence)
            || !state.consequences.contains_key(consequence))
    {
        return Err(invalid(
            "requisition saga consequence differs from its exact force intent",
        ));
    }
    if let Some(externality) = &saga.externality_intent
        && state
            .externality_intents
            .get(externality)
            .is_none_or(|value| value.saga != saga.id)
    {
        return Err(invalid(
            "requisition saga externality intent is orphaned or rewrapped",
        ));
    }
    if let Some(outcome) = &saga.externality_outcome {
        let externality = saga
            .externality_intent
            .as_ref()
            .ok_or_else(|| invalid("requisition externality outcome lacks its intent"))?;
        let exact_intent = &state.externality_intents[externality];
        let mut detached = outcome.clone();
        let recorded = std::mem::take(&mut detached.semantic_digest);
        if outcome.intent != *externality
            || outcome.expected_target != exact_intent.expected_economy_target
            || recorded
                != canonical_hash(
                    "canwu.force-supply.economy-externality-outcome.v1",
                    &detached,
                )?
        {
            return Err(invalid(
                "requisition saga externality outcome is forged or targets another intent",
            ));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn validate_completion_participants(
    state: &ForceSupplyStateV1,
    intent: &ForceConsumptionIntent,
    force: &ReferenceForce,
    acquisition: &CompletionLeaseAcquisitionV1,
) -> Result<(), CanwuError> {
    let policy_requires_economy = intent.requisition_policy.as_ref().is_some_and(|policy| {
        state.requisition_policies[policy].applicability == ExternalityApplicability::Required
    });
    let expected_core = BTreeSet::from([
        PLUGIN_NAME.to_owned(),
        canwu_resource::PLUGIN_NAME.to_owned(),
    ]);
    let external_owners = acquisition
        .expected_participants
        .difference(&expected_core)
        .cloned()
        .collect::<Vec<_>>();
    if (policy_requires_economy && external_owners.len() != 1)
        || (!policy_requires_economy && !external_owners.is_empty())
        || !expected_core.is_subset(&acquisition.expected_participants)
        || acquisition.state != canwu_resource::CompletionLeaseAcquisitionStateV1::Activated
    {
        return Err(invalid(
            "force completion acquisition has a forged participant set or lifecycle",
        ));
    }
    let force_version = intent
        .completion_certificate
        .locked_target_versions
        .iter()
        .find_map(|target| match target {
            CompletionLockedTargetV1::ExternalRecord { version }
                if version.record == force_supply_runtime_reference().into_untyped()
                    && version.version != 0 =>
            {
                Some(version.clone())
            }
            _ => None,
        })
        .ok_or_else(|| invalid("force completion target version is unavailable"))?;
    let force_grant = participant_grant(state, acquisition, PLUGIN_NAME)?;
    if force_grant.target_versions
        != vec![CompletionLockedTargetV1::ExternalRecord {
            version: force_version,
        }]
        || force_grant.state != CompletionGrantStateV1::Consumed
    {
        return Err(invalid(
            "force completion grant does not exactly lock the force runtime",
        ));
    }
    let resource_grant = participant_grant(state, acquisition, canwu_resource::PLUGIN_NAME)?;
    let mut exact_resource_targets = vec![
        CompletionLockedTargetV1::Account {
            id: intent.allocation.account.clone(),
            revision: intent.allocation.account_revision,
        },
        CompletionLockedTargetV1::AllocationLeg {
            id: intent.allocation.id.clone(),
            revision: intent.allocation.revision,
        },
    ];
    exact_resource_targets.sort();
    let expected_resource_state = if intent.resource_outcome.is_some() {
        CompletionGrantStateV1::Completed
    } else {
        CompletionGrantStateV1::Prepared
    };
    let mut actual_core_targets = resource_grant
        .target_versions
        .iter()
        .filter(|target| !matches!(target, CompletionLockedTargetV1::Demand { .. }))
        .cloned()
        .collect::<Vec<_>>();
    actual_core_targets.sort();
    let demand_lock_count = resource_grant
        .target_versions
        .iter()
        .filter(|target| matches!(target, CompletionLockedTargetV1::Demand { .. }))
        .count();
    if actual_core_targets != exact_resource_targets
        || demand_lock_count > 1
        || resource_grant.state != expected_resource_state
    {
        return Err(invalid(format!(
            "resource participant grant does not exactly lock the allocation/account: actual={:?}, expected={:?}, demand_locks={demand_lock_count}, state={:?}, expected_state={:?}",
            resource_grant.target_versions,
            exact_resource_targets,
            resource_grant.state,
            expected_resource_state,
        )));
    }
    if policy_requires_economy {
        let external_owner = external_owners
            .first()
            .ok_or_else(|| invalid("force externality completion provider is unavailable"))?;
        let economy_grant = participant_grant(state, acquisition, external_owner)?;
        let expected_economy_state = if state
            .sagas
            .values()
            .find(|saga| saga.intent == intent.id)
            .is_some_and(|saga| saga.externality_outcome.is_some())
        {
            CompletionGrantStateV1::Completed
        } else {
            CompletionGrantStateV1::Prepared
        };
        if economy_grant.target_versions
            != vec![CompletionLockedTargetV1::ExternalRecord {
                version: exact_economy_target(intent)?.clone(),
            }]
            || economy_grant.state != expected_economy_state
        {
            return Err(invalid(
                "economy participant grant does not exactly lock the externality target",
            ));
        }
    }
    if force.id != intent.force {
        return Err(invalid("force completion grant names another force"));
    }
    Ok(())
}

fn participant_grant<'a>(
    state: &'a ForceSupplyStateV1,
    acquisition: &CompletionLeaseAcquisitionV1,
    owner: &str,
) -> Result<&'a canwu_resource::CompletionCapacityGrantV1, CanwuError> {
    if let Some(grant_id) = acquisition.grants.get(owner) {
        return state
            .completion_leases
            .grants
            .get(grant_id)
            .ok_or_else(|| invalid(format!("force completion {owner} grant is orphaned")));
    }
    state
        .completion_participant_grants
        .get(&acquisition.id)
        .and_then(|participants| participants.get(owner))
        .map(|participant| &participant.grant)
        .ok_or_else(|| invalid(format!("force completion acquisition lacks {owner} grant")))
}

fn require_force_holder(
    state: &ForceSupplyStateV1,
    force: &ReferenceForceId,
    holder: &KnowledgeHolderRef,
) -> Result<(), CanwuError> {
    if state
        .forces
        .get(force)
        .is_some_and(|value| &value.holder == holder)
    {
        Ok(())
    } else {
        Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "holder is not authorized for this force",
        ))
    }
}

fn require_completion_holder(
    state: &ForceSupplyStateV1,
    holder: &KnowledgeHolderRef,
    acquisition: &CompletionLeaseAcquisitionId,
) -> Result<(), CanwuError> {
    if state
        .completion_leases
        .acquisitions
        .get(acquisition)
        .is_some_and(|value| &value.holder == holder)
    {
        Ok(())
    } else {
        Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "force completion lease is not holder-authorized",
        ))
    }
}

fn reseal_intent(intent: &mut ForceConsumptionIntent) -> Result<(), CanwuError> {
    intent.semantic_digest.clear();
    intent.semantic_digest = canonical_hash("canwu.force-supply.intent.v1", intent)?;
    Ok(())
}

fn advance_due(
    force: &mut ReferenceForce,
    requirement: &ForceSupplyRequirementV1,
    due_count: u16,
    serviced_at: SimTime,
) -> Result<(), CanwuError> {
    let due = force
        .due
        .get_mut(&requirement.id)
        .ok_or_else(|| invalid("force due state disappeared"))?;
    if let ForceSupplyCadenceV1::FixedMinutes { interval_minutes } = requirement.cadence {
        let minutes = interval_minutes
            .checked_mul(u64::from(due_count))
            .ok_or_else(|| invalid("force cadence overflowed"))?;
        let minutes =
            i64::try_from(minutes).map_err(|_| invalid("force cadence exceeds simulation time"))?;
        let scheduled_due = due.next_due;
        let elapsed = serviced_at
            .as_minutes()
            .checked_sub(scheduled_due.as_minutes())
            .ok_or_else(|| invalid("force cadence service precedes its scheduled due"))?;
        due.persisted_remainder_minutes = u64::try_from(elapsed)
            .map_err(|_| invalid("force cadence elapsed time is negative"))?
            % interval_minutes;
        due.next_due = scheduled_due
            .checked_add(canwu_api::SimDuration::minutes(minutes))
            .ok_or_else(|| invalid("force next due time overflowed"))?;
        if due.next_due <= serviced_at {
            return Err(invalid(
                "force cadence due count failed to advance beyond the service cut",
            ));
        }
    }
    Ok(())
}

fn cadence_interval(requirement: &ForceSupplyRequirementV1) -> Option<u64> {
    match requirement.cadence {
        ForceSupplyCadenceV1::FixedMinutes { interval_minutes } => Some(interval_minutes),
        ForceSupplyCadenceV1::EventDriven => None,
    }
}

fn derive_due(
    requirement: &ForceSupplyRequirementV1,
    due: &DueRequirementStateV1,
    now: SimTime,
) -> Result<(SimTime, u16, u64), CanwuError> {
    if due.requirement != requirement.id || now < due.next_due {
        return Err(invalid(
            "force requirement is not due at the current actor cut",
        ));
    }
    match requirement.cadence {
        ForceSupplyCadenceV1::FixedMinutes { interval_minutes } => {
            if interval_minutes == 0 {
                return Err(invalid("force cadence interval is zero"));
            }
            let elapsed = now
                .as_minutes()
                .checked_sub(due.next_due.as_minutes())
                .ok_or_else(|| invalid("force cadence elapsed time underflowed"))?;
            let elapsed = u64::try_from(elapsed)
                .map_err(|_| invalid("force cadence elapsed time is negative"))?;
            let due_count = elapsed
                .checked_div(interval_minutes)
                .and_then(|count| count.checked_add(1))
                .ok_or_else(|| invalid("force due count overflowed"))?;
            let due_count = u16::try_from(due_count)
                .map_err(|_| invalid("force due count exceeds the bounded catch-up window"))?;
            Ok((due.next_due, due_count, elapsed % interval_minutes))
        }
        ForceSupplyCadenceV1::EventDriven => Ok((due.next_due, 1, 0)),
    }
}

fn validate_stock_custody_binding(
    binding: &ForceStockCustodyBindingV1,
    allocation: &ResourceAllocationLegVersionV1,
    force_holder: &KnowledgeHolderRef,
) -> Result<(), CanwuError> {
    let mut detached = binding.clone();
    let recorded = std::mem::take(&mut detached.semantic_digest);
    if binding.destination_account != allocation.account {
        return Err(invalid(
            "force stock custody names a different account than its allocation",
        ));
    }
    if &binding.destination_custodian != force_holder {
        return Err(invalid(format!(
            "force stock custody is held by {:?}, not the force authority {:?}",
            binding.destination_custodian, force_holder
        )));
    }
    if recorded != canonical_hash("canwu.force-supply.stock-custody.v1", &detached)? {
        return Err(invalid("force stock custody binding digest is forged"));
    }
    if let Some(transfer) = &binding.accepted_transfer {
        let mut detached = transfer.clone();
        let recorded = std::mem::take(&mut detached.semantic_digest);
        if transfer.destination != binding.destination_account
            || transfer.accepted_quantity == 0
            || recorded != canonical_hash("canwu.force-supply.accepted-transfer.v1", &detached)?
        {
            return Err(invalid(
                "force accepted-transfer evidence is incomplete or forged",
            ));
        }
    }
    Ok(())
}

fn validate_resource_settlement(
    settlement: &ForceResourceSettlementEvidenceV1,
) -> Result<(), CanwuError> {
    let mut detached = settlement.clone();
    let recorded = std::mem::take(&mut detached.semantic_digest);
    if settlement.outcome.semantic_digest.is_empty()
        || settlement.consumption.semantic_digest.is_empty()
        || settlement.fulfillment.semantic_digest.is_empty()
        || settlement.destination_account_revision == ResourceRevision::INITIAL
        || recorded != canonical_hash("canwu.force-supply.resource-settlement.v1", &detached)?
    {
        return Err(invalid(
            "force resource settlement evidence is incomplete or forged",
        ));
    }
    Ok(())
}

pub(crate) fn observation_temporal_scope_key(
    holder: &KnowledgeHolderRef,
    force: &ReferenceForceId,
    fact: &ForceKnowledgeFactV1,
) -> ForceObservationScopeV1 {
    let suffix = match fact {
        ForceKnowledgeFactV1::SupplyObservation(observation) => {
            format!("supply:{}", observation.requirement)
        }
        ForceKnowledgeFactV1::ShortageAttribution(attribution) => {
            format!("shortage:{}", attribution.requirement)
        }
        ForceKnowledgeFactV1::RequisitionProgress { .. } => "requisition".to_owned(),
    };
    ForceObservationScopeV1 {
        holder: holder.clone(),
        force: force.clone(),
        fact_key: suffix,
    }
}

fn exact_economy_target(
    intent: &ForceConsumptionIntent,
) -> Result<&DomainRecordVersionRef, CanwuError> {
    let mut targets = intent
        .completion_certificate
        .locked_target_versions
        .iter()
        .filter_map(|target| match target {
            CompletionLockedTargetV1::ExternalRecord { version }
                if version.record.kind.namespace == "canwu.economy-reference"
                    && version.record.kind.name == "runtime" =>
            {
                Some(version)
            }
            _ => None,
        });
    let target = targets
        .next()
        .ok_or_else(|| invalid("requisition intent lacks its exact economy target version"))?;
    if targets.next().is_some() {
        return Err(invalid(
            "requisition intent carries ambiguous economy target versions",
        ));
    }
    Ok(target)
}

fn apply_delta(value: u16, delta: i16) -> u16 {
    let next = i32::from(value) + i32::from(delta);
    u16::try_from(next.clamp(0, 1_000)).expect("clamped value")
}

pub(crate) fn invalid(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidDomainRecord, message)
}

#[allow(clippy::needless_pass_by_value)]
fn resource_error(error: ResourceError) -> CanwuError {
    let code = match error {
        ResourceError::Authority(_) => ErrorCode::InvalidAuthority,
        ResourceError::NotFound(_) => ErrorCode::DomainRecordNotFound,
        ResourceError::VersionConflict(_) => ErrorCode::DomainRecordVersionConflict,
        ResourceError::IdempotencyConflict(_) => ErrorCode::IdempotencyConflict,
        _ => ErrorCode::InvalidDomainRecord,
    };
    CanwuError::new(code, error.to_string())
}
#[allow(clippy::needless_pass_by_value)]
pub(crate) fn encode_error(error: serde_json::Error) -> CanwuError {
    invalid(format!(
        "force-supply payload could not be encoded: {error}"
    ))
}
