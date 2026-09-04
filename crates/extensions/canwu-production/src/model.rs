use crate::{PLUGIN_NAME, PLUGIN_NAMESPACE};
use canwu_api::{
    BoundaryId, CanwuError, CommandAttemptId, CommandRequestId, DecisionTicketId, DecisionTraceId,
    DomainRecord, DomainRecordClass, DomainRecordDraft, DomainRecordLifecycle, DomainRecordType,
    DomainRecordVersionRef, DomainValueKindClass, EntityRef, ErrorCode, EvidenceRef,
    KnowledgeHolderRef, PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD,
    PayloadRequiredEvidenceContinuationV1, RandomDrawAddress, RandomOperationTarget, RandomSample,
    RandomStreamKey, SimTime, TypedDomainRecordRef, canonical_hash,
};
use canwu_resource::{
    CompletionCapacityGrantV1, CompletionGrantStateV1, CompletionLeaseAcquisitionId,
    CompletionLeaseActivationCertificateV1, ResourceAccountId, ResourceAllocationLegVersionV1,
    ResourceConsumptionVersionV1, ResourceCreditRequestV1, ResourceCreditSourceV1,
    ResourceDefinitionRevisionId, ResourceOperationKey, ResourceOperationKind,
    ResourceOperationOutcome, ResourceOperationOutcomeVersionV1, ResourceOperationStatus,
    ResourceRevision, ResourceUnitRevisionId, RunBudgetRevisionV1,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

pub const PRODUCTION_RUNTIME_ID: &str = "canwu.production:runtime";
pub const MAX_PRODUCTION_IDENTIFIER_BYTES: usize = 160;
pub const MAX_REQUIREMENT_GROUPS: usize = 64;
pub const MAX_REQUIREMENT_ALTERNATIVES: usize = 16;
pub const MAX_EVIDENCE_BINDINGS: usize = 128;
pub const MAX_REPORT_FACTS: usize = 256;
pub const MAX_OBSERVATION_HEADS_PER_SCOPE: usize = 16;
pub const MAX_HOT_RECEIPTS: usize = 8_192;
pub const MAX_ARCHIVE_PREPARE_CANDIDATES: usize = 2_048;
pub const MAX_ARCHIVE_OBJECTS_PER_BATCH: usize = 1_024;
pub const MAX_ARCHIVE_BYTES_PER_BATCH: usize = 16 * 1024 * 1024;
pub const MAX_ARCHIVE_PAGE_ENTRIES: usize = 512;
pub const MAX_PENDING_RETENTION_HANDLES: usize = 64;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionLimitsV1 {
    pub max_process_revisions: usize,
    pub max_sites: usize,
    pub max_facilities: usize,
    pub max_work_orders: usize,
    pub max_executions: usize,
    pub max_capacity_allocations: usize,
    pub max_projects: usize,
    pub max_operation_outcomes: usize,
    pub max_observation_records: usize,
    pub max_observation_records_per_holder: usize,
    pub max_holders: usize,
    pub max_reports_per_boundary: usize,
    pub max_mutations_per_boundary: usize,
    pub max_incidents_per_boundary: usize,
    pub max_incident_receipts: usize,
    pub max_observation_heads: usize,
    pub max_observation_dirty: usize,
    pub max_archive_due: usize,
    pub max_pending_retention_handles: usize,
    pub max_serialized_bytes: usize,
}

impl ProductionLimitsV1 {
    #[must_use]
    pub const fn canonical() -> Self {
        Self {
            max_process_revisions: 2_048,
            max_sites: 2_048,
            max_facilities: 4_096,
            max_work_orders: 4_096,
            max_executions: 4_096,
            max_capacity_allocations: 4_096,
            max_projects: 4_096,
            max_operation_outcomes: 8_192,
            max_observation_records: 8_192,
            max_observation_records_per_holder: 1_024,
            max_holders: 1_024,
            max_reports_per_boundary: 64,
            max_mutations_per_boundary: 4_096,
            max_incidents_per_boundary: 512,
            max_incident_receipts: MAX_HOT_RECEIPTS,
            max_observation_heads: 8_192,
            max_observation_dirty: 4_096,
            max_archive_due: 4_096,
            max_pending_retention_handles: MAX_PENDING_RETENTION_HANDLES,
            max_serialized_bytes: 256 * 1024 * 1024,
        }
    }
}

macro_rules! production_id {
    ($name:ident) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, CanwuError> {
                let value = value.into();
                validate_identifier(&value, stringify!($name))?;
                Ok(Self(value))
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(de::Error::custom)
            }
        }
    };
}

production_id!(ProcessRevisionId);
production_id!(ProductionSiteId);
production_id!(FacilityAssetId);
production_id!(ProductionCapacityAllocationId);
production_id!(WorkOrderId);
production_id!(ProductionExecutionId);
production_id!(WorkInProgressId);
production_id!(FacilityProjectId);
production_id!(ProductionOperationOutcomeId);
production_id!(ProductionObservationId);
production_id!(ProductionObserverGrantId);
production_id!(ProductionIncidentId);
production_id!(ProductionArchiveReceiptId);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductionSiteForm {
    Household,
    SupplementaryHousehold,
    DistributedWorkshop,
    GovernmentWorkshop,
    ConcentratedPlant,
    MultiSiteEnterprise,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductionRequirementKind {
    Material,
    LaborCapability,
    Facility,
    ToolsMachines,
    Energy,
    TechnologyImplementation,
    Authorization,
    EnvironmentSeason,
    Security,
    Access,
    Maintenance,
    FinanceOrganization,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionRequirementAlternative {
    pub id: String,
    pub capability: String,
    pub minimum_quantity: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionRequirementGroup {
    pub id: String,
    pub kind: ProductionRequirementKind,
    pub any_of: Vec<ProductionRequirementAlternative>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceRequirement {
    pub resource: ResourceDefinitionRevisionId,
    pub unit: ResourceUnitRevisionId,
    pub quantity: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionOutputSpec {
    pub resource: ResourceDefinitionRevisionId,
    pub unit: ResourceUnitRevisionId,
    pub quantity: u64,
    pub quality_class: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CapacityRequirement {
    pub capability: String,
    pub quantity: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProcessRevision {
    pub id: ProcessRevisionId,
    pub label: String,
    pub semantic_digest: String,
    pub effective_from: SimTime,
    pub effective_until: Option<SimTime>,
    pub work_units: u64,
    pub requirements: Vec<ProductionRequirementGroup>,
    pub inputs: Vec<ResourceRequirement>,
    pub outputs: Vec<ProductionOutputSpec>,
    pub capacity: Vec<CapacityRequirement>,
    pub adoption_required: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionSite {
    pub id: ProductionSiteId,
    pub holder: KnowledgeHolderRef,
    pub place: EntityRef,
    pub form: ProductionSiteForm,
    pub active: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FacilityLifecycle {
    Planned,
    Authorized,
    Reserving,
    InProgress,
    Commissioning,
    Operational,
    Degraded,
    Damaged,
    Repairing,
    Retired,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FacilityAsset {
    pub id: FacilityAssetId,
    pub site: ProductionSiteId,
    pub generation: u64,
    pub lifecycle: FacilityLifecycle,
    pub condition_per_mille: u16,
    pub capacity: BTreeMap<String, u64>,
    pub maintenance_evidence: Vec<ProductionEvidenceBinding>,
    pub operational_stage_capacity_per_mille: u16,
    /// Per-evaluation probability, expressed in thousandths. Zero disables
    /// incident evaluation for this facility.
    pub incident_risk_per_mille: u16,
    /// Upper bound for a kernel-randomized condition loss when an incident is
    /// selected. The caller never supplies an incident draw or severity.
    pub incident_max_severity_per_mille: u16,
}

impl FacilityAsset {
    #[must_use]
    pub fn usable_capacity(&self, capability: &str) -> u64 {
        let base = self.capacity.get(capability).copied().unwrap_or_default();
        let availability = match self.lifecycle {
            FacilityLifecycle::Operational
            | FacilityLifecycle::Degraded
            | FacilityLifecycle::Damaged => self.condition_per_mille,
            FacilityLifecycle::InProgress | FacilityLifecycle::Commissioning => {
                self.operational_stage_capacity_per_mille
            }
            FacilityLifecycle::Repairing
            | FacilityLifecycle::Planned
            | FacilityLifecycle::Authorized
            | FacilityLifecycle::Reserving
            | FacilityLifecycle::Retired => 0,
        };
        base.saturating_mul(u64::from(availability)) / 1_000
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapacityAllocationState {
    Reserved,
    Consumed,
    Released,
    Expired,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionCapacityAllocation {
    pub id: ProductionCapacityAllocationId,
    pub facility: FacilityAssetId,
    pub facility_generation: u64,
    pub capability: String,
    pub start: SimTime,
    pub end: SimTime,
    pub quantity: u64,
    pub work_order: WorkOrderId,
    pub execution: ProductionExecutionId,
    pub operation_key: String,
    pub state: CapacityAllocationState,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkOrderLifecycle {
    Proposed,
    Authorized,
    Reserving,
    Ready,
    Running,
    CompletedPendingOutputSettlement,
    Settled,
    Cancelled,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkOrder {
    pub id: WorkOrderId,
    pub holder: KnowledgeHolderRef,
    pub process: ProcessRevisionId,
    pub site: ProductionSiteId,
    pub quantity: u64,
    pub lifecycle: WorkOrderLifecycle,
    pub execution: Option<ProductionExecutionId>,
    pub expected_revision: u64,
    pub created_at: SimTime,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionEvidenceBinding {
    pub kind: ProductionRequirementKind,
    pub capability: String,
    pub version: DomainRecordVersionRef,
    pub semantic_digest: String,
    pub available_quantity: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TechnologyEvidenceBinding {
    pub technique_revision: DomainRecordVersionRef,
    pub capability_qualification: Option<DomainRecordVersionRef>,
    pub implementation: Option<DomainRecordVersionRef>,
    pub adoption: Option<DomainRecordVersionRef>,
    pub semantic_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceInputBinding {
    pub allocation_leg: ResourceAllocationLegVersionV1,
    pub consumption: ResourceConsumptionVersionV1,
    pub consumption_outcome: ResourceOperationOutcomeVersionV1,
    pub quantity: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionResourceContinuationWitnessV1 {
    pub project: FacilityProjectId,
    pub resource_archive_directory_root: String,
    pub resource_archive_record_count: u64,
    pub input_bindings_digest: String,
    pub semantic_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionOutputSettlementRequest {
    pub operation_key: ResourceOperationKey,
    pub account: ResourceAccountId,
    pub expected_account_revision: ResourceRevision,
    pub resource: ResourceDefinitionRevisionId,
    pub unit: ResourceUnitRevisionId,
    pub quantity: u64,
}

impl ProductionOutputSettlementRequest {
    #[must_use]
    pub fn resource_credit_request(
        &self,
        source: DomainRecordVersionRef,
        completion_certificate: canwu_resource::CompletionLeaseActivationCertificateV1,
        at: SimTime,
    ) -> ResourceCreditRequestV1 {
        ResourceCreditRequestV1 {
            operation_key: self.operation_key.clone(),
            account: self.account.clone(),
            expected_account_revision: self.expected_account_revision,
            resource_revision: self.resource.clone(),
            unit_revision: self.unit.clone(),
            quantity: self.quantity,
            source: ResourceCreditSourceV1::Production(source),
            at,
            completion_certificate,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionOutputAcknowledgement {
    pub execution: ProductionExecutionId,
    pub production_source: DomainRecordVersionRef,
    pub outcomes: Vec<ResourceOperationOutcome>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionExecution {
    pub id: ProductionExecutionId,
    pub work_order: WorkOrderId,
    pub process: ProcessRevisionId,
    pub site: ProductionSiteId,
    pub facility: FacilityAssetId,
    pub allocations: Vec<ProductionCapacityAllocationId>,
    pub lifecycle: WorkOrderLifecycle,
    pub started_at: SimTime,
    pub completed_at: Option<SimTime>,
    pub evidence: Vec<ProductionEvidenceBinding>,
    pub technology: TechnologyEvidenceBinding,
    pub inputs: Vec<ResourceInputBinding>,
    pub output_requests: Vec<ProductionOutputSettlementRequest>,
    pub output_outcomes: Vec<ResourceOperationOutcome>,
    pub output_source: Option<DomainRecordVersionRef>,
    pub output_ack_digest: Option<String>,
    pub completion_certificate: canwu_resource::CompletionLeaseActivationCertificateV1,
    pub production_completion_grant: canwu_resource::CompletionCapacityGrantId,
    pub resource_completion_grant: canwu_resource::CompletionCapacityGrantId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkInProgress {
    pub id: WorkInProgressId,
    pub execution: ProductionExecutionId,
    pub completed_units: u64,
    pub total_units: u64,
    pub consumed_input_evidence: Vec<ResourceInputBinding>,
    pub recoverable_input_quantity: u64,
    pub non_recoverable_waste_quantity: u64,
    pub updated_at: SimTime,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FacilityProjectLifecycle {
    Planned,
    Authorized,
    Reserving,
    InProgress,
    Commissioning,
    CompletionPending,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FacilityProjectKind {
    Construction,
    Repair,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FacilityProject {
    pub id: FacilityProjectId,
    pub holder: KnowledgeHolderRef,
    pub site: ProductionSiteId,
    pub facility: FacilityAssetId,
    pub kind: FacilityProjectKind,
    pub process: ProcessRevisionId,
    pub lifecycle: FacilityProjectLifecycle,
    pub completed_units: u64,
    pub total_units: u64,
    pub base_generation: u64,
    pub resulting_generation: u64,
    pub evidence: Vec<ProductionEvidenceBinding>,
    pub technology: TechnologyEvidenceBinding,
    pub inputs: Vec<ResourceInputBinding>,
    pub operation_key: ResourceOperationKey,
    pub completion_certificate: CompletionLeaseActivationCertificateV1,
    pub production_completion_grant: canwu_resource::CompletionCapacityGrantId,
    pub resource_completion_grant: canwu_resource::CompletionCapacityGrantId,
    pub resulting_asset: Option<FacilityAsset>,
    pub created_at: SimTime,
    pub started_at: Option<SimTime>,
    pub completed_at: Option<SimTime>,
    pub result_evidence_digest: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductionOperationDisposition {
    Applied,
    Duplicate,
    Rejected,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionOperationOutcome {
    pub id: ProductionOperationOutcomeId,
    pub canonical_input_hash: String,
    pub command: ProductionCommandEnvelope,
    pub disposition: ProductionOperationDisposition,
    pub work_order: Option<WorkOrderId>,
    pub execution: Option<ProductionExecutionId>,
    #[serde(default)]
    pub project: Option<FacilityProjectId>,
    pub rejection_code: Option<String>,
    pub rejection_message: Option<String>,
    pub settled_at: SimTime,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionBlocker {
    pub requirement: ProductionRequirementGroup,
    pub kind: ProductionRequirementKind,
    pub required: Vec<ProductionRequirementAlternative>,
    pub available: Vec<ProductionEvidenceBinding>,
    pub shortage_by_alternative: BTreeMap<String, u64>,
    pub next_eligible_action: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductionObservationRole {
    Operator,
    LocalOwner,
    RemoteOwner,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionObserverGrant {
    pub id: ProductionObserverGrantId,
    pub holder: KnowledgeHolderRef,
    pub sites: BTreeSet<ProductionSiteId>,
    pub role: ProductionObservationRole,
    pub delay_minutes: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionObservationFact {
    pub subject: String,
    pub state: String,
    pub quantity_low: u64,
    pub quantity_high: u64,
    pub source_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionObservationReport {
    pub id: ProductionObservationId,
    pub holder: KnowledgeHolderRef,
    pub scope: ProductionSiteId,
    pub observed_at: SimTime,
    pub materialized_at: SimTime,
    pub provider_state_revision: u64,
    pub role: ProductionObservationRole,
    pub facts: Vec<ProductionObservationFact>,
    pub blockers: Vec<ProductionBlocker>,
    pub canonical_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ProductionObservationHeadKeyV1 {
    pub holder: KnowledgeHolderRef,
    pub scope: ProductionSiteId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionObservationHeadV1 {
    pub key: ProductionObservationHeadKeyV1,
    pub role: ProductionObservationRole,
    pub observed_at: SimTime,
    pub materialized_at: SimTime,
    pub provider_state_revision: u64,
    pub facts: Vec<ProductionObservationFact>,
    pub blockers: Vec<ProductionBlocker>,
    pub source_evidence: Vec<EvidenceRef>,
    pub canonical_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionObservationWitnessV1 {
    pub provider_plugin: String,
    pub provider_version: String,
    pub provider_semantic_hash: String,
    pub provider_state_revision: u64,
    pub holder: KnowledgeHolderRef,
    pub scope: ProductionSiteId,
    pub observed_at: SimTime,
    pub materialized_at: SimTime,
    pub report_digest: String,
    pub source_versions: Vec<DomainRecordVersionRef>,
    pub adapter_revision: String,
    pub canonical_digest: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DegradedFacilityChoice {
    ContinueDegraded,
    StopForRepair,
    DeferOrder,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionDecisionReceiptV1 {
    pub ticket_id: DecisionTicketId,
    pub ticket_version: u64,
    pub trace_id: DecisionTraceId,
    pub controller_id: String,
    pub selected_option: String,
    pub holder_facts_digest: String,
    pub command_request_id: Option<CommandRequestId>,
    pub command_attempt_id: Option<CommandAttemptId>,
    pub decided_at: SimTime,
    pub canonical_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DegradedFacilityDecisionContextV1 {
    pub holder: KnowledgeHolderRef,
    pub work_order: WorkOrderId,
    pub facility: FacilityAssetId,
    pub facility_generation: u64,
    pub expected_runtime_revision: u64,
    pub holder_facts_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionIncidentRandomEvidenceV1 {
    pub stream: RandomStreamKey,
    pub trigger: RandomSample,
    pub severity: Option<RandomSample>,
    pub operation_evidence: EvidenceRef,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionIncidentTransitionV1 {
    pub id: ProductionIncidentId,
    pub operation_key: String,
    pub facility: FacilityAssetId,
    pub expected_generation: u64,
    pub condition_before: u16,
    pub condition_after: u16,
    pub lifecycle_after: FacilityLifecycle,
    pub source_record_revision: u64,
    pub source_record_digest: String,
    pub random: ProductionIncidentRandomEvidenceV1,
    pub evaluated_at: SimTime,
    pub evaluation_boundary: BoundaryId,
    pub canonical_digest: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductionArchiveRetentionPhaseV1 {
    Prepared,
    Verified,
    DurableIngress,
    Committed,
    RejectedStale,
    Abandoned,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionArchiveRetentionHandleV1 {
    pub handle_id: String,
    pub source_root: String,
    pub target_directory_root: String,
    pub object_ids: BTreeMap<String, BTreeSet<String>>,
    pub phase: ProductionArchiveRetentionPhaseV1,
    pub semantic_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionArchiveMaintenanceReceiptV1 {
    pub id: ProductionArchiveReceiptId,
    pub source_root: String,
    pub directory_root: String,
    pub archived_executions: usize,
    #[serde(default)]
    pub archived_projects: usize,
    pub disposition: ProductionArchiveRetentionPhaseV1,
    pub canonical_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum ProductionTerminalArchiveKeyV1 {
    Execution(ProductionExecutionId),
    FacilityProject(FacilityProjectId),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionTerminalArchiveRecordV1 {
    pub key: ProductionTerminalArchiveKeyV1,
    pub work_order: WorkOrderId,
    pub process: ProcessRevisionId,
    pub site: ProductionSiteId,
    pub facility: FacilityAssetId,
    pub lifecycle: WorkOrderLifecycle,
    pub completed_units: u64,
    pub total_units: u64,
    pub recoverable_input_quantity: u64,
    pub non_recoverable_waste_quantity: u64,
    pub input_consumption_digests: Vec<String>,
    pub technology_digest: String,
    pub output_outcome_digests: Vec<String>,
    pub output_source: Option<DomainRecordVersionRef>,
    pub work_order_record: WorkOrder,
    pub execution_record: ProductionExecution,
    pub work_in_progress_record: WorkInProgress,
    pub operation_outcomes: Vec<ProductionOperationOutcome>,
    pub completion_acquisition: canwu_resource::CompletionLeaseAcquisitionV1,
    pub production_completion_grant: CompletionCapacityGrantV1,
    pub participant_grants: Vec<crate::ProductionCompletionParticipantGrantV1>,
    pub completion_receipts: Vec<canwu_resource::CompletionLeaseReceiptV1>,
    pub terminal_at: SimTime,
    pub canonical_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionFacilityProjectArchiveRecordV1 {
    pub key: ProductionTerminalArchiveKeyV1,
    pub project: FacilityProject,
    pub resulting_asset: FacilityAsset,
    #[serde(default)]
    pub operation_outcomes: Vec<ProductionOperationOutcome>,
    pub completion_acquisition: canwu_resource::CompletionLeaseAcquisitionV1,
    pub production_completion_grant: CompletionCapacityGrantV1,
    pub participant_grants: Vec<crate::ProductionCompletionParticipantGrantV1>,
    pub completion_receipts: Vec<canwu_resource::CompletionLeaseReceiptV1>,
    pub terminal_at: SimTime,
    pub canonical_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionArchiveBlobV1 {
    pub format_version: u32,
    pub expected_source_root: String,
    pub records: Vec<ProductionTerminalArchiveRecordV1>,
    #[serde(default)]
    pub project_records: Vec<ProductionFacilityProjectArchiveRecordV1>,
    pub content_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionArchiveMembershipV1 {
    pub key: ProductionTerminalArchiveKeyV1,
    pub blob_id: String,
    pub ordinal: u16,
    pub semantic_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionArchiveMembershipPageV1 {
    pub id: String,
    pub memberships: Vec<ProductionArchiveMembershipV1>,
    pub semantic_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionArchiveTemporalEntryV1 {
    pub terminal_at: SimTime,
    pub key: ProductionTerminalArchiveKeyV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionArchiveTemporalPageV1 {
    pub id: String,
    pub entries: Vec<ProductionArchiveTemporalEntryV1>,
    pub semantic_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionArchiveIndexDirectoryV1 {
    pub id: String,
    pub previous_root: Option<String>,
    pub blob_ids: Vec<String>,
    pub membership_pages: Vec<String>,
    pub temporal_pages: Vec<String>,
    pub archived_execution_count: u64,
    #[serde(default)]
    pub archived_project_count: u64,
    pub semantic_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PreparedProductionArchiveBatchV1 {
    pub expected_source_root: String,
    pub selected: Vec<ProductionExecutionId>,
    #[serde(default)]
    pub selected_projects: Vec<FacilityProjectId>,
    pub blob: ProductionArchiveBlobV1,
    pub membership_page: ProductionArchiveMembershipPageV1,
    pub temporal_page: ProductionArchiveTemporalPageV1,
    pub directory: ProductionArchiveIndexDirectoryV1,
    pub retention: ProductionArchiveRetentionHandleV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct VerifiedProductionArchiveCommitV1 {
    pub expected_source_root: String,
    pub selected: Vec<ProductionExecutionId>,
    #[serde(default)]
    pub selected_projects: Vec<FacilityProjectId>,
    pub directory_root: String,
    pub retention: ProductionArchiveRetentionHandleV1,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionArchiveHeadStateV1 {
    pub directory_root: Option<String>,
    pub archived_execution_count: u64,
    #[serde(default)]
    pub archived_project_count: u64,
    pub committed_batch_count: u64,
    pub pending_handles: BTreeMap<String, ProductionArchiveRetentionHandleV1>,
    pub maintenance_receipts:
        BTreeMap<ProductionArchiveReceiptId, ProductionArchiveMaintenanceReceiptV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum ProductionOperation {
    RequestCompletionLease {
        request: canwu_resource::RequestCompletionLeaseV1,
    },
    AbortCompletionLease {
        request: canwu_resource::AbortCompletionLeaseV1,
    },
    CreateWorkOrder {
        work_order: WorkOrder,
    },
    AuthorizeWorkOrder {
        work_order: WorkOrderId,
    },
    StartExecution {
        execution: ProductionExecution,
        allocations: Vec<ProductionCapacityAllocation>,
    },
    AdvanceExecution {
        execution: ProductionExecutionId,
        completed_units: u64,
    },
    CompleteExecution {
        execution: ProductionExecutionId,
    },
    CancelWorkOrder {
        work_order: WorkOrderId,
    },
    ResolveDegradedFacility {
        work_order: WorkOrderId,
        facility: FacilityAssetId,
        choice: DegradedFacilityChoice,
        decision_ticket: DecisionTicketId,
    },
    CreateFacilityProject {
        project: FacilityProject,
    },
    AuthorizeFacilityProject {
        project: FacilityProjectId,
    },
    AdvanceFacilityProject {
        project: FacilityProjectId,
        completed_units: u64,
    },
    AcceptFacilityCommissioning {
        project: FacilityProjectId,
    },
    RetireFacility {
        facility: FacilityAssetId,
        expected_generation: u64,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionCommandEnvelope {
    pub operation_id: ProductionOperationOutcomeId,
    pub holder: KnowledgeHolderRef,
    pub expected_runtime_revision: u64,
    pub operation: ProductionOperation,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionState {
    pub revision: u64,
    pub processes: BTreeMap<ProcessRevisionId, ProcessRevision>,
    pub sites: BTreeMap<ProductionSiteId, ProductionSite>,
    pub facilities: BTreeMap<FacilityAssetId, FacilityAsset>,
    pub capacity_allocations:
        BTreeMap<ProductionCapacityAllocationId, ProductionCapacityAllocation>,
    pub work_orders: BTreeMap<WorkOrderId, WorkOrder>,
    pub executions: BTreeMap<ProductionExecutionId, ProductionExecution>,
    pub work_in_progress: BTreeMap<WorkInProgressId, WorkInProgress>,
    pub facility_projects: BTreeMap<FacilityProjectId, FacilityProject>,
    #[serde(default)]
    pub resource_continuation_witnesses:
        BTreeMap<FacilityProjectId, ProductionResourceContinuationWitnessV1>,
    #[serde(default)]
    pub project_operation_outcome_reservations: BTreeMap<FacilityProjectId, u64>,
    pub operation_outcomes: BTreeMap<ProductionOperationOutcomeId, ProductionOperationOutcome>,
    pub observer_grants: BTreeMap<ProductionObserverGrantId, ProductionObserverGrant>,
    pub decision_receipts: BTreeMap<DecisionTicketId, ProductionDecisionReceiptV1>,
    pub production_run_budget: Option<RunBudgetRevisionV1>,
    pub production_completion_grants:
        BTreeMap<canwu_resource::CompletionCapacityGrantId, CompletionCapacityGrantV1>,
    pub production_completion_certificates:
        BTreeMap<CompletionLeaseAcquisitionId, CompletionLeaseActivationCertificateV1>,
    #[serde(default)]
    pub completion_acquisitions:
        BTreeMap<CompletionLeaseAcquisitionId, canwu_resource::CompletionLeaseAcquisitionV1>,
    #[serde(default)]
    pub completion_participant_grants: BTreeMap<
        CompletionLeaseAcquisitionId,
        BTreeMap<String, crate::ProductionCompletionParticipantGrantV1>,
    >,
    #[serde(default)]
    pub completion_admission_epochs: BTreeMap<String, crate::ProductionCompletionAdmissionEpochV1>,
    #[serde(default)]
    pub completion_target_locks: BTreeMap<
        String,
        (
            canwu_resource::CompletionLockedTargetV1,
            canwu_resource::CompletionCapacityGrantId,
        ),
    >,
    #[serde(default)]
    pub completion_receipts: BTreeMap<u64, canwu_resource::CompletionLeaseReceiptV1>,
    #[serde(default)]
    pub completion_expiry_due: BTreeMap<u64, BTreeSet<CompletionLeaseAcquisitionId>>,
    #[serde(default)]
    pub completion_reserved_units: u64,
    #[serde(default)]
    pub completion_next_sequence: u64,
    pub incident_due_index: BTreeSet<FacilityAssetId>,
    #[serde(default)]
    pub incident_cursor: Option<FacilityAssetId>,
    #[serde(default)]
    pub incident_round: u64,
    pub incident_receipts: BTreeMap<String, ProductionIncidentTransitionV1>,
    pub observation_heads: BTreeMap<String, Vec<ProductionObservationHeadV1>>,
    #[serde(default)]
    pub observation_rollover: BTreeMap<String, ProductionObservationHeadV1>,
    pub observation_dirty_index: BTreeSet<ProductionObservationHeadKeyV1>,
    pub observation_due_index: BTreeMap<SimTime, BTreeSet<ProductionObservationHeadKeyV1>>,
    pub archive_due_index: BTreeSet<ProductionExecutionId>,
    pub project_archive_due_index: BTreeSet<FacilityProjectId>,
    pub archive: ProductionArchiveHeadStateV1,
}

#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProductionRuntimeRecord;

impl DomainRecordType for ProductionRuntimeRecord {
    type Payload = ProductionState;
    type Class = DomainValueKindClass;

    const NAMESPACE: &'static str = PLUGIN_NAMESPACE;
    const NAME: &'static str = "runtime";
}

#[must_use]
pub fn production_runtime_reference() -> TypedDomainRecordRef<ProductionRuntimeRecord> {
    TypedDomainRecordRef::new(PRODUCTION_RUNTIME_ID)
}

impl ProductionState {
    pub fn into_initial_record(mut self) -> Result<DomainRecord, CanwuError> {
        self.rebuild_runtime_indexes()?;
        self.validate()?;
        let draft = self.draft()?;
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
        let mut draft = DomainRecordDraft::from_typed(production_runtime_reference(), self)?;
        let dependencies = self
            .executions
            .values()
            .filter(|execution| {
                matches!(
                    execution.lifecycle,
                    WorkOrderLifecycle::Running
                        | WorkOrderLifecycle::CompletedPendingOutputSettlement
                )
            })
            .flat_map(|execution| {
                execution
                    .evidence
                    .iter()
                    .map(|binding| binding.version.clone())
                    .chain(std::iter::once(
                        execution.technology.technique_revision.clone(),
                    ))
                    .chain(
                        execution
                            .technology
                            .capability_qualification
                            .iter()
                            .cloned(),
                    )
                    .chain(execution.technology.implementation.iter().cloned())
                    .chain(execution.technology.adoption.iter().cloned())
                    .chain(
                        execution
                            .inputs
                            .iter()
                            .map(|input| input.consumption.consumer_evidence.clone()),
                    )
            })
            .chain(
                self.facility_projects
                    .values()
                    .filter(|project| {
                        !matches!(
                            project.lifecycle,
                            FacilityProjectLifecycle::Completed
                                | FacilityProjectLifecycle::Cancelled
                                | FacilityProjectLifecycle::Failed
                        )
                    })
                    .flat_map(|project| {
                        project
                            .evidence
                            .iter()
                            .map(|binding| binding.version.clone())
                            .chain(std::iter::once(
                                project.technology.technique_revision.clone(),
                            ))
                            .chain(project.technology.capability_qualification.iter().cloned())
                            .chain(project.technology.implementation.iter().cloned())
                            .chain(project.technology.adoption.iter().cloned())
                            .chain(
                                project
                                    .inputs
                                    .iter()
                                    .map(|input| input.consumption.consumer_evidence.clone()),
                            )
                    }),
            )
            .map(EvidenceRef::DomainRecordVersion)
            .chain(
                self.incident_receipts
                    .values()
                    .map(|receipt| receipt.random.operation_evidence.clone()),
            )
            .chain(
                self.completion_participant_grants
                    .values()
                    .flat_map(std::collections::BTreeMap::values)
                    .map(|participant| {
                        EvidenceRef::DomainRecordVersion(participant.provider_source.clone())
                    }),
            )
            .chain(
                self.observation_heads
                    .values()
                    .flatten()
                    .flat_map(|head| head.source_evidence.iter().cloned()),
            )
            .chain(
                self.observation_rollover
                    .values()
                    .flat_map(|head| head.source_evidence.iter().cloned()),
            )
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let continuation = if dependencies.is_empty() {
            PayloadRequiredEvidenceContinuationV1::completed()
        } else {
            PayloadRequiredEvidenceContinuationV1::active(dependencies)
        };
        draft
            .payload
            .as_object_mut()
            .ok_or_else(|| invalid("production runtime payload is not an object"))?
            .insert(
                PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD.to_owned(),
                serde_json::to_value(continuation).map_err(|error| {
                    invalid(format!(
                        "production continuation could not be encoded: {error}"
                    ))
                })?,
            );
        Ok(draft)
    }

    pub fn validate(&self) -> Result<(), CanwuError> {
        let limits = ProductionLimitsV1::canonical();
        for (actual, maximum, label) in [
            (
                self.processes.len(),
                limits.max_process_revisions,
                "process revisions",
            ),
            (self.sites.len(), limits.max_sites, "production sites"),
            (self.facilities.len(), limits.max_facilities, "facilities"),
            (
                self.work_orders.len(),
                limits.max_work_orders,
                "work orders",
            ),
            (self.executions.len(), limits.max_executions, "executions"),
            (
                self.capacity_allocations.len(),
                limits.max_capacity_allocations,
                "capacity allocations",
            ),
            (
                self.facility_projects.len(),
                limits.max_projects,
                "facility projects",
            ),
            (
                self.operation_outcomes.len(),
                limits.max_operation_outcomes,
                "operation outcomes",
            ),
        ] {
            if actual > maximum {
                return Err(invalid(format!(
                    "production {label} exceed their canonical cap"
                )));
            }
        }
        if self.incident_due_index.len() > limits.max_incidents_per_boundary {
            return Err(invalid(
                "production incident due index exceeds its boundary cap",
            ));
        }
        let observation_head_count = self
            .observation_heads
            .values()
            .try_fold(0_usize, |total, heads| total.checked_add(heads.len()))
            .ok_or_else(|| invalid("production observation head count overflowed"))?;
        for (actual, maximum, label) in [
            (
                self.incident_receipts.len(),
                limits.max_incident_receipts,
                "incident receipts",
            ),
            (
                observation_head_count,
                limits.max_observation_heads,
                "observation heads",
            ),
            (
                self.observation_rollover.len(),
                MAX_HOT_RECEIPTS,
                "observation rollover receipts",
            ),
            (
                self.observation_dirty_index.len(),
                limits.max_observation_dirty,
                "observation dirty entries",
            ),
            (
                self.observation_due_index.values().map(BTreeSet::len).sum(),
                limits.max_observation_dirty,
                "observation due entries",
            ),
            (
                self.archive_due_index
                    .len()
                    .checked_add(self.project_archive_due_index.len())
                    .ok_or_else(|| invalid("production archive due count overflowed"))?,
                limits.max_archive_due,
                "archive due entries",
            ),
            (
                self.archive.pending_handles.len(),
                limits.max_pending_retention_handles,
                "pending archive handles",
            ),
            (
                self.archive.maintenance_receipts.len(),
                limits.max_operation_outcomes,
                "archive maintenance receipts",
            ),
            (
                self.production_completion_grants.len(),
                limits.max_executions,
                "production completion grants",
            ),
            (
                self.production_completion_certificates.len(),
                limits.max_executions,
                "production completion certificates",
            ),
        ] {
            if actual > maximum {
                return Err(invalid(format!(
                    "production {label} exceed their canonical cap"
                )));
            }
        }
        if self.observer_grants.len() > limits.max_holders {
            return Err(invalid(
                "production observer grants exceed their holder cap",
            ));
        }
        let mut observer_holders = BTreeSet::new();
        for (id, grant) in &self.observer_grants {
            if id != &grant.id
                || !observer_holders.insert(grant.holder.clone())
                || grant.sites.is_empty()
                || grant.sites.len() > MAX_REPORT_FACTS
                || grant
                    .sites
                    .iter()
                    .any(|site| !self.sites.contains_key(site))
            {
                return Err(invalid("production observer grant is invalid"));
            }
        }
        for (id, outcome) in &self.operation_outcomes {
            if id != &outcome.id
                || outcome.command.operation_id != outcome.id
                || outcome.canonical_input_hash
                    != canonical_hash("canwu.production.operation-input.v1", &outcome.command)?
                || (outcome.project.is_some()
                    && (outcome.work_order.is_some() || outcome.execution.is_some()))
                || match outcome.disposition {
                    ProductionOperationDisposition::Applied
                    | ProductionOperationDisposition::Duplicate => {
                        outcome.rejection_code.is_some() || outcome.rejection_message.is_some()
                    }
                    ProductionOperationDisposition::Rejected => {
                        outcome.rejection_code.is_none() || outcome.rejection_message.is_none()
                    }
                }
            {
                return Err(invalid("production operation outcome is invalid"));
            }
        }
        let reserved_project_outcomes = self
            .project_operation_outcome_reservations
            .values()
            .try_fold(0_usize, |total, reserved| {
                usize::try_from(*reserved)
                    .ok()
                    .and_then(|reserved| total.checked_add(reserved))
            })
            .ok_or_else(|| invalid("production project outcome reservation overflowed"))?;
        if self
            .operation_outcomes
            .len()
            .checked_add(reserved_project_outcomes)
            .is_none_or(|used| used > limits.max_operation_outcomes)
        {
            return Err(invalid(
                "production operation outcomes exceed their cap including project reservations",
            ));
        }
        if self.resource_continuation_witnesses.len() > limits.max_projects {
            return Err(invalid(
                "production resource continuation witnesses exceed their project bound",
            ));
        }
        for (project_id, witness) in &self.resource_continuation_witnesses {
            let project = self.facility_projects.get(project_id).ok_or_else(|| {
                invalid("production resource continuation witness lost its project")
            })?;
            let mut detached = witness.clone();
            let recorded = std::mem::take(&mut detached.semantic_digest);
            if &witness.project != project_id
                || matches!(
                    project.lifecycle,
                    FacilityProjectLifecycle::Completed
                        | FacilityProjectLifecycle::Cancelled
                        | FacilityProjectLifecycle::Failed
                )
                || witness.resource_archive_directory_root.len() != 64
                || witness.resource_archive_record_count == 0
                || witness.input_bindings_digest != resource_input_bindings_digest(&project.inputs)?
                || recorded
                    != canonical_hash(
                        "canwu.production.resource-continuation-witness.v1",
                        &detached,
                    )?
            {
                return Err(invalid(
                    "production resource continuation witness is forged or stale",
                ));
            }
        }
        self.validate_completion_capacity()?;
        self.validate_completion_coordinator()?;
        self.validate_runtime_indexes()?;
        self.validate_processes()?;
        self.validate_assets()?;
        self.validate_allocations()?;
        self.validate_work()?;
        self.validate_projects()?;
        let encoded = serde_json::to_vec(self)
            .map_err(|error| invalid(format!("production state could not be sized: {error}")))?;
        if encoded.len() > limits.max_serialized_bytes {
            return Err(invalid(
                "production authoritative state exceeds its byte cap",
            ));
        }
        Ok(())
    }

    fn validate_completion_capacity(&self) -> Result<(), CanwuError> {
        if self.production_completion_grants.is_empty()
            && self.production_completion_certificates.is_empty()
        {
            return Ok(());
        }
        let budget = self.production_run_budget.as_ref().ok_or_else(|| {
            invalid("production completion grants require an exact run-budget revision")
        })?;
        budget.validate().map_err(|error| {
            invalid(format!(
                "production completion run budget is invalid: {error}"
            ))
        })?;
        let mut reserved = 0_u64;
        for (id, grant) in &self.production_completion_grants {
            if id != &grant.id
                || grant.owner_plugin != PLUGIN_NAME
                || grant.run_budget_revision != budget.revision
                || grant.target_versions.is_empty()
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
                return Err(invalid("production completion capacity grant is invalid"));
            }
            if !matches!(
                grant.state,
                CompletionGrantStateV1::Completed
                    | CompletionGrantStateV1::Released
                    | CompletionGrantStateV1::Rejected
                    | CompletionGrantStateV1::Expired
            ) {
                reserved = reserved
                    .checked_add(grant.reserved_units)
                    .ok_or_else(|| invalid("production completion reserve overflowed"))?;
            }
        }
        if reserved > budget.total_completion_units {
            return Err(invalid(
                "production completion grants exceed the package-owned run budget",
            ));
        }
        for (acquisition, certificate) in &self.production_completion_certificates {
            let mut detached = certificate.clone();
            let recorded = std::mem::take(&mut detached.semantic_digest);
            if acquisition != &certificate.acquisition
                || recorded
                    != canonical_hash(
                        "canwu.resource.completion-activation-certificate.v1",
                        &detached,
                    )?
                || !certificate.prepared_grants.iter().any(|(id, revision)| {
                    self.production_completion_grants
                        .get(id)
                        .is_some_and(|grant| {
                            grant.acquisition == *acquisition
                                && (grant.revision == *revision
                                    || grant.revision.get() > revision.get())
                        })
                })
            {
                return Err(invalid(
                    "production completion certificate does not bind a local exact grant",
                ));
            }
        }
        Ok(())
    }

    fn validate_processes(&self) -> Result<(), CanwuError> {
        for (id, process) in &self.processes {
            if id != &process.id
                || process.semantic_digest.is_empty()
                || process.work_units == 0
                || process.outputs.is_empty()
                || process.outputs.iter().any(|output| output.quantity == 0)
                || process.outputs.len() > MAX_EVIDENCE_BINDINGS
                || process
                    .effective_until
                    .is_some_and(|until| until <= process.effective_from)
                || process.requirements.len() > MAX_REQUIREMENT_GROUPS
            {
                return Err(invalid(format!("process revision {id} is invalid")));
            }
            for group in &process.requirements {
                validate_identifier(&group.id, "requirement group")?;
                if group.any_of.is_empty() || group.any_of.len() > MAX_REQUIREMENT_ALTERNATIVES {
                    return Err(invalid(format!(
                        "process {id} has an invalid alternative group"
                    )));
                }
                for alternative in &group.any_of {
                    validate_identifier(&alternative.id, "requirement alternative")?;
                    canonical_text(&alternative.capability, "requirement capability")?;
                    if alternative.minimum_quantity == 0 {
                        return Err(invalid("production requirement quantity must be positive"));
                    }
                }
            }
            if process.inputs.iter().any(|value| value.quantity == 0)
                || process
                    .capacity
                    .iter()
                    .any(|value| value.quantity == 0 || value.capability.trim().is_empty())
            {
                return Err(invalid(format!(
                    "process revision {id} has zero requirements"
                )));
            }
        }
        Ok(())
    }

    fn validate_assets(&self) -> Result<(), CanwuError> {
        for (id, site) in &self.sites {
            if id != &site.id {
                return Err(invalid("production site map key does not match its ID"));
            }
        }
        for (id, facility) in &self.facilities {
            if id != &facility.id
                || !self.sites.contains_key(&facility.site)
                || facility.generation == 0
                || facility.condition_per_mille > 1_000
                || facility.operational_stage_capacity_per_mille > 1_000
                || facility.incident_risk_per_mille > 1_000
                || facility.incident_max_severity_per_mille > 1_000
                || (facility.incident_risk_per_mille > 0
                    && facility.incident_max_severity_per_mille == 0)
                || facility.capacity.values().any(|value| *value == 0)
            {
                return Err(invalid(format!("facility {id} is invalid")));
            }
            if matches!(facility.lifecycle, FacilityLifecycle::Retired)
                && facility.usable_capacity("any") > 0
            {
                return Err(invalid("retired facilities cannot expose usable capacity"));
            }
        }
        Ok(())
    }

    fn validate_allocations(&self) -> Result<(), CanwuError> {
        let mut partitions =
            BTreeMap::<(&FacilityAssetId, &str), Vec<&ProductionCapacityAllocation>>::new();
        for (id, allocation) in &self.capacity_allocations {
            let facility = self.facilities.get(&allocation.facility).ok_or_else(|| {
                invalid(format!(
                    "capacity allocation {id} names an unknown facility"
                ))
            })?;
            if id != &allocation.id
                || match allocation.state {
                    CapacityAllocationState::Reserved => {
                        allocation.facility_generation != facility.generation
                    }
                    CapacityAllocationState::Consumed => {
                        allocation.facility_generation > facility.generation
                    }
                    CapacityAllocationState::Released | CapacityAllocationState::Expired => false,
                }
                || allocation.start >= allocation.end
                || allocation.quantity == 0
                || allocation.operation_key.trim().is_empty()
                || !self.work_orders.contains_key(&allocation.work_order)
                || matches!(
                    allocation.state,
                    CapacityAllocationState::Reserved | CapacityAllocationState::Consumed
                ) && !self.executions.contains_key(&allocation.execution)
            {
                return Err(invalid(format!("capacity allocation {id} is invalid")));
            }
            if matches!(
                allocation.state,
                CapacityAllocationState::Reserved | CapacityAllocationState::Consumed
            ) {
                partitions
                    .entry((&allocation.facility, allocation.capability.as_str()))
                    .or_default()
                    .push(allocation);
            }
        }
        for ((facility_id, capability), mut allocations) in partitions {
            allocations.sort_by_key(|value| (&value.start, &value.end, &value.id));
            let facility = &self.facilities[facility_id];
            for point in allocations
                .iter()
                .flat_map(|value| [value.start, value.end])
            {
                let used = allocations
                    .iter()
                    .filter(|value| value.start <= point && point < value.end)
                    .try_fold(0_u64, |sum, value| sum.checked_add(value.quantity))
                    .ok_or_else(|| invalid("production capacity sum overflowed"))?;
                let has_unconsumed_reservation = allocations.iter().any(|value| {
                    value.start <= point
                        && point < value.end
                        && value.state == CapacityAllocationState::Reserved
                });
                let capacity_limit = if has_unconsumed_reservation {
                    facility.usable_capacity(capability)
                } else {
                    facility
                        .capacity
                        .get(capability)
                        .copied()
                        .unwrap_or_default()
                };
                if used > capacity_limit {
                    return Err(invalid(format!(
                        "capacity allocations overlap beyond usable capacity for {facility_id}:{capability}"
                    )));
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn validate_work(&self) -> Result<(), CanwuError> {
        let mut resource_legs = BTreeSet::new();
        for (id, order) in &self.work_orders {
            if id != &order.id
                || order.quantity == 0
                || !self.processes.contains_key(&order.process)
                || !self.sites.contains_key(&order.site)
            {
                return Err(invalid(format!("work order {id} is invalid")));
            }
            if let Some(execution) = &order.execution
                && !self.executions.contains_key(execution)
            {
                return Err(invalid(format!("work order {id} lost its execution")));
            }
        }
        for (id, execution) in &self.executions {
            let order = self
                .work_orders
                .get(&execution.work_order)
                .ok_or_else(|| invalid(format!("execution {id} lost its work order")))?;
            let process = self
                .processes
                .get(&execution.process)
                .ok_or_else(|| invalid(format!("execution {id} lost its process revision")))?;
            let allocations = execution
                .allocations
                .iter()
                .map(|allocation| {
                    self.capacity_allocations.get(allocation).ok_or_else(|| {
                        invalid(format!(
                            "execution {id} lost capacity allocation {allocation}"
                        ))
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            let output_operation_keys = execution
                .output_requests
                .iter()
                .map(|request| &request.operation_key)
                .collect::<BTreeSet<_>>();
            let output_accounts = execution
                .output_requests
                .iter()
                .map(|request| &request.account)
                .collect::<BTreeSet<_>>();
            if id != &execution.id
                || order.execution.as_ref() != Some(id)
                || order.lifecycle != execution.lifecycle
                || order.process != execution.process
                || order.site != execution.site
                || allocations.is_empty()
                || allocations
                    .iter()
                    .any(|allocation| allocation.execution != *id)
                || execution.evidence.len() > MAX_EVIDENCE_BINDINGS
                || execution.inputs.len() != process.inputs.len()
                || execution.output_requests.len() != process.outputs.len()
                || output_operation_keys.len() != execution.output_requests.len()
                || output_accounts.len() != execution.output_requests.len()
                || execution.output_source.as_ref().is_some_and(|source| {
                    source.record != production_runtime_reference().into_untyped()
                })
                || execution.output_requests.iter().zip(&process.outputs).any(
                    |(request, output)| {
                        request.resource != output.resource
                            || request.unit != output.unit
                            || request.quantity != output.quantity.saturating_mul(order.quantity)
                    },
                )
            {
                return Err(invalid(format!("execution {id} is inconsistent")));
            }
            validate_capacity_cover(process, order, execution, &allocations)?;
            validate_technology_binding(process, &execution.technology)?;
            validate_completion_certificate(execution, execution.started_at)?;
            let grant = self
                .production_completion_grants
                .get(&execution.production_completion_grant)
                .ok_or_else(|| {
                    invalid(format!(
                        "execution {id} lost its package-owned production completion grant"
                    ))
                })?;
            if self
                .production_completion_certificates
                .get(&execution.completion_certificate.acquisition)
                != Some(&execution.completion_certificate)
                || grant.acquisition != execution.completion_certificate.acquisition
                || grant.operation_key != execution.completion_certificate.operation_key
                || match execution.lifecycle {
                    WorkOrderLifecycle::Running
                    | WorkOrderLifecycle::CompletedPendingOutputSettlement => {
                        grant.state != CompletionGrantStateV1::Consumed
                    }
                    WorkOrderLifecycle::Settled
                    | WorkOrderLifecycle::Cancelled
                    | WorkOrderLifecycle::Failed => {
                        grant.state != CompletionGrantStateV1::Completed
                    }
                    _ => true,
                }
            {
                return Err(invalid(format!(
                    "execution {id} completion grant/certificate closure is invalid"
                )));
            }
            validate_requirements(process, &execution.evidence)?;
            for (input, required) in execution.inputs.iter().zip(&process.inputs) {
                if input.quantity == 0
                    || input.quantity != required.quantity.saturating_mul(order.quantity)
                    || input.allocation_leg.resource_revision != required.resource
                    || input.allocation_leg.unit_revision != required.unit
                    || input.allocation_leg.quantity < input.quantity
                    || input.consumption.allocation_leg != input.allocation_leg.id
                    || input.consumption.account != input.allocation_leg.account
                    || input.consumption.quantity != input.quantity
                    || input.consumption.semantic_digest.is_empty()
                    || !matches!(
                        input.consumption_outcome.status,
                        ResourceOperationStatus::Applied | ResourceOperationStatus::Duplicate
                    )
                    || input.consumption_outcome.quantity != input.quantity
                    || input.consumption_outcome.remainder != 0
                    || input.allocation_leg.semantic_digest.is_empty()
                    || input.consumption_outcome.semantic_digest.is_empty()
                    || !resource_legs.insert(input.allocation_leg.id.clone())
                {
                    return Err(invalid(
                        "production input does not exactly match its resource allocation and consumption outcome",
                    ));
                }
            }
            let wip_id = WorkInProgressId::new(format!("canwu.production:wip:{id}"))?;
            let wip = self
                .work_in_progress
                .get(&wip_id)
                .ok_or_else(|| invalid(format!("execution {id} lost its work in progress")))?;
            if wip.execution != *id
                || wip.completed_units > wip.total_units
                || wip.total_units != process.work_units.saturating_mul(order.quantity)
                || wip.consumed_input_evidence != execution.inputs
            {
                return Err(invalid(format!(
                    "execution {id} has invalid work in progress"
                )));
            }
            match execution.lifecycle {
                WorkOrderLifecycle::Running
                    if allocations.iter().all(|allocation| {
                        allocation.state == CapacityAllocationState::Consumed
                    }) && execution.output_outcomes.is_empty()
                        && execution.output_source.is_none()
                        && execution.output_ack_digest.is_none()
                        && execution.completed_at.is_none() => {}
                WorkOrderLifecycle::CompletedPendingOutputSettlement
                    if allocations.iter().all(|allocation| {
                        allocation.state == CapacityAllocationState::Consumed
                    }) && wip.completed_units == wip.total_units
                        && execution.completed_at.is_some()
                        && execution.output_outcomes.is_empty()
                        && execution.output_ack_digest.is_none() => {}
                WorkOrderLifecycle::Settled
                    if allocations.iter().all(|allocation| {
                        allocation.state == CapacityAllocationState::Released
                    }) && execution.output_outcomes.len() == execution.output_requests.len()
                        && !execution.output_outcomes.is_empty()
                        && execution.output_source.is_some()
                        && execution.output_ack_digest.is_some() => {}
                WorkOrderLifecycle::Cancelled | WorkOrderLifecycle::Failed
                    if allocations.iter().all(|allocation| {
                        matches!(
                            allocation.state,
                            CapacityAllocationState::Released | CapacityAllocationState::Expired
                        )
                    }) => {}
                _ => {
                    return Err(invalid(format!(
                        "execution {id} has an invalid lifecycle closure"
                    )));
                }
            }
        }
        Ok(())
    }

    fn validate_projects(&self) -> Result<(), CanwuError> {
        let mut active_generations = BTreeSet::new();
        for (id, project) in &self.facility_projects {
            let process = self
                .processes
                .get(&project.process)
                .ok_or_else(|| invalid(format!("facility project {id} lost its process")))?;
            let grant = self
                .production_completion_grants
                .get(&project.production_completion_grant)
                .ok_or_else(|| {
                    invalid(format!(
                        "facility project {id} lost its package-owned completion grant"
                    ))
                })?;
            let certificate = self
                .production_completion_certificates
                .get(&project.completion_certificate.acquisition);
            let resource_grant = self
                .completion_participant_grants
                .get(&project.completion_certificate.acquisition)
                .and_then(|participants| participants.get(canwu_resource::PLUGIN_NAME))
                .map(|participant| &participant.grant);
            let required_outcome_reservation =
                required_project_operation_outcome_reservation(project)?;
            if id != &project.id
                || !self.sites.contains_key(&project.site)
                || project.total_units != process.work_units
                || project.completed_units > project.total_units
                || project.base_generation == 0
                || project.resulting_generation != project.base_generation.saturating_add(1)
                || grant.acquisition != project.completion_certificate.acquisition
                || grant.operation_key != project.operation_key
                || certificate != Some(&project.completion_certificate)
                || resource_grant.is_none_or(|resource_grant| {
                    resource_grant.id != project.resource_completion_grant
                        || resource_grant.acquisition != project.completion_certificate.acquisition
                        || resource_grant.operation_key != project.operation_key
                        || resource_grant.state
                            != if project.lifecycle == FacilityProjectLifecycle::Completed {
                                CompletionGrantStateV1::Completed
                            } else {
                                CompletionGrantStateV1::Consumed
                            }
                })
                || if required_outcome_reservation == 0 {
                    self.project_operation_outcome_reservations.contains_key(id)
                } else {
                    self.project_operation_outcome_reservations.get(id)
                        != Some(&required_outcome_reservation)
                }
            {
                return Err(invalid(format!("facility project {id} is invalid")));
            }
            validate_project_completion_certificate(project)?;
            validate_requirements(process, &project.evidence)?;
            validate_technology_binding(process, &project.technology)?;
            validate_resource_inputs(process, &project.inputs, 1)?;
            let expected_result_digest = facility_project_result_digest(project)?;
            let terminal_evidence = matches!(
                project.lifecycle,
                FacilityProjectLifecycle::Commissioning
                    | FacilityProjectLifecycle::CompletionPending
                    | FacilityProjectLifecycle::Completed
            );
            let grant_state_is_valid = match project.lifecycle {
                FacilityProjectLifecycle::Planned | FacilityProjectLifecycle::Authorized => {
                    grant.state == CompletionGrantStateV1::Prepared
                }
                FacilityProjectLifecycle::Reserving
                | FacilityProjectLifecycle::InProgress
                | FacilityProjectLifecycle::Commissioning
                | FacilityProjectLifecycle::CompletionPending => {
                    grant.state == CompletionGrantStateV1::Consumed
                }
                FacilityProjectLifecycle::Completed
                | FacilityProjectLifecycle::Cancelled
                | FacilityProjectLifecycle::Failed => {
                    grant.state == CompletionGrantStateV1::Completed
                }
            };
            let result_is_valid = match &project.resulting_asset {
                Some(asset) => {
                    asset.id == project.facility
                        && asset.site == project.site
                        && asset.generation == project.resulting_generation
                        && asset.lifecycle == FacilityLifecycle::Operational
                        && asset.condition_per_mille == 1_000
                }
                None => false,
            };
            if project.created_at < process.effective_from
                || process
                    .effective_until
                    .is_some_and(|until| project.created_at >= until)
                || !grant_state_is_valid
                || project
                    .started_at
                    .is_some_and(|started| started < project.created_at)
                || project.completed_at.is_some_and(|completed| {
                    project.started_at.is_none_or(|started| completed < started)
                })
                || terminal_evidence
                    != (project.completed_units == project.total_units
                        && project.completed_at.is_some()
                        && result_is_valid
                        && project.result_evidence_digest.as_deref()
                            == Some(expected_result_digest.as_str()))
                || (project.lifecycle == FacilityProjectLifecycle::Completed
                    && self.facilities.get(&project.facility) != project.resulting_asset.as_ref())
                || (!terminal_evidence
                    && (project.resulting_asset.is_some()
                        || project.result_evidence_digest.is_some()))
            {
                return Err(invalid(format!(
                    "facility project {id} lacks exact time/result evidence"
                )));
            }
            if !matches!(
                project.lifecycle,
                FacilityProjectLifecycle::Completed
                    | FacilityProjectLifecycle::Cancelled
                    | FacilityProjectLifecycle::Failed
            ) && !active_generations.insert((project.facility.clone(), project.base_generation))
            {
                return Err(invalid(
                    "multiple active facility projects own the same facility generation",
                ));
            }
            if project.lifecycle == FacilityProjectLifecycle::Completed
                && self
                    .completion_acquisitions
                    .get(&project.completion_certificate.acquisition)
                    .is_none_or(|acquisition| {
                        acquisition.state
                            != canwu_resource::CompletionLeaseAcquisitionStateV1::Released
                    })
            {
                return Err(invalid(
                    "completed facility project has not closed every completion participant",
                ));
            }
        }
        if self
            .project_operation_outcome_reservations
            .keys()
            .any(|project_id| !self.facility_projects.contains_key(project_id))
        {
            return Err(invalid(
                "production project outcome reservation lost its facility project",
            ));
        }
        Ok(())
    }

    fn validate_runtime_indexes(&self) -> Result<(), CanwuError> {
        if self.incident_due_index.iter().any(|id| {
            self.facilities.get(id).is_none_or(|facility| {
                facility.incident_risk_per_mille == 0
                    || facility.condition_per_mille == 0
                    || !matches!(
                        facility.lifecycle,
                        FacilityLifecycle::Operational
                            | FacilityLifecycle::Degraded
                            | FacilityLifecycle::Damaged
                    )
            })
        }) {
            return Err(invalid(
                "production incident due index contains an ineligible facility",
            ));
        }
        for (key, receipt) in &self.incident_receipts {
            let mut detached = receipt.clone();
            let recorded = std::mem::take(&mut detached.canonical_digest);
            if key != &receipt.operation_key
                || recorded.is_empty()
                || recorded != canonical_hash("canwu.production.incident-transition.v1", &detached)?
                || receipt.random.trigger.stream != receipt.random.stream
                || receipt.random.trigger.upper_exclusive != 1_000
                || receipt.random.trigger.value >= 1_000
                || receipt.source_record_revision == 0
                || receipt.source_record_digest.is_empty()
            {
                return Err(invalid("production incident receipt is invalid"));
            }
        }
        for (storage_key, heads) in &self.observation_heads {
            let Some(key) = heads.first().map(|head| &head.key) else {
                return Err(invalid("production observation head chain is empty"));
            };
            if heads.is_empty()
                || heads.len() > MAX_OBSERVATION_HEADS_PER_SCOPE
                || !self.observation_key_is_granted(key)
                || storage_key != &production_observation_head_storage_key(key)?
                || heads.windows(2).any(|pair| {
                    pair[0].observed_at > pair[1].observed_at
                        || (pair[0].observed_at == pair[1].observed_at
                            && pair[0].provider_state_revision >= pair[1].provider_state_revision)
                })
            {
                return Err(invalid("production observation head chain is invalid"));
            }
            for head in heads {
                let mut detached = head.clone();
                let recorded = std::mem::take(&mut detached.canonical_digest);
                if head.key != *key
                    || head.facts.len() + head.blockers.len() > MAX_REPORT_FACTS
                    || head.source_evidence.is_empty()
                    || recorded
                        != canonical_hash("canwu.production.observation-head.v1", &detached)?
                {
                    return Err(invalid("production observation head is forged"));
                }
            }
        }
        for (digest, head) in &self.observation_rollover {
            let mut detached = head.clone();
            let recorded = std::mem::take(&mut detached.canonical_digest);
            if digest != &recorded
                || !self.observation_key_is_granted(&head.key)
                || recorded != canonical_hash("canwu.production.observation-head.v1", &detached)?
            {
                return Err(invalid("production observation rollover receipt is forged"));
            }
        }
        if self
            .observation_dirty_index
            .iter()
            .chain(self.observation_due_index.values().flatten())
            .any(|key| !self.observation_key_is_granted(key))
        {
            return Err(invalid(
                "production observation index contains an unauthorized key",
            ));
        }
        if self.archive_due_index.iter().any(|execution_id| {
            self.executions.get(execution_id).is_none_or(|execution| {
                !matches!(
                    execution.lifecycle,
                    WorkOrderLifecycle::Settled
                        | WorkOrderLifecycle::Cancelled
                        | WorkOrderLifecycle::Failed
                )
            })
        }) {
            return Err(invalid(
                "production archive due index contains a non-terminal execution",
            ));
        }
        if self.project_archive_due_index.iter().any(|project_id| {
            self.facility_projects
                .get(project_id)
                .is_none_or(|project| project.lifecycle != FacilityProjectLifecycle::Completed)
        }) {
            return Err(invalid(
                "production archive due index contains a non-terminal facility project",
            ));
        }
        for (id, receipt) in &self.archive.maintenance_receipts {
            let mut detached = receipt.clone();
            let recorded = std::mem::take(&mut detached.canonical_digest);
            if id != &receipt.id
                || recorded
                    != canonical_hash("canwu.production.archive-maintenance-receipt.v1", &detached)?
            {
                return Err(invalid("production archive maintenance receipt is invalid"));
            }
        }
        for handle in self.archive.pending_handles.values() {
            let mut detached = handle.clone();
            let recorded = std::mem::take(&mut detached.semantic_digest);
            if handle.handle_id.is_empty()
                || handle.source_root.is_empty()
                || handle.target_directory_root.is_empty()
                || handle.object_ids.is_empty()
                || recorded != canonical_hash("canwu.production.archive-retention.v1", &detached)?
            {
                return Err(invalid("production archive retention handle is invalid"));
            }
        }
        for (ticket_id, receipt) in &self.decision_receipts {
            let mut detached = receipt.clone();
            let recorded = std::mem::take(&mut detached.canonical_digest);
            if ticket_id != &receipt.ticket_id
                || recorded != canonical_hash("canwu.production.decision-receipt.v1", &detached)?
            {
                return Err(invalid("production decision receipt is invalid"));
            }
        }
        Ok(())
    }

    fn observation_key_is_granted(&self, key: &ProductionObservationHeadKeyV1) -> bool {
        self.observer_grants
            .values()
            .any(|grant| grant.holder == key.holder && grant.sites.contains(&key.scope))
    }

    #[must_use]
    pub fn blockers_for(
        &self,
        process: &ProcessRevision,
        evidence: &[ProductionEvidenceBinding],
    ) -> Vec<ProductionBlocker> {
        process
            .requirements
            .iter()
            .filter_map(|group| {
                let satisfied = group.any_of.iter().any(|alternative| {
                    evidence.iter().any(|binding| {
                        binding.kind == group.kind
                            && binding.capability == alternative.capability
                            && binding.available_quantity >= alternative.minimum_quantity
                    })
                });
                (!satisfied).then(|| ProductionBlocker {
                    requirement: group.clone(),
                    kind: group.kind,
                    required: group.any_of.clone(),
                    available: evidence
                        .iter()
                        .filter(|value| value.kind == group.kind)
                        .cloned()
                        .collect(),
                    shortage_by_alternative: group
                        .any_of
                        .iter()
                        .map(|alternative| {
                            let available = evidence
                                .iter()
                                .filter(|binding| {
                                    binding.kind == group.kind
                                        && binding.capability == alternative.capability
                                })
                                .map(|binding| binding.available_quantity)
                                .max()
                                .unwrap_or_default();
                            (
                                alternative.id.clone(),
                                alternative.minimum_quantity.saturating_sub(available),
                            )
                        })
                        .collect(),
                    next_eligible_action: next_action(group.kind).to_owned(),
                })
            })
            .collect()
    }

    pub fn apply_operation(
        &mut self,
        envelope: &ProductionCommandEnvelope,
        now: SimTime,
    ) -> Result<(), CanwuError> {
        let mut candidate = self.clone();
        candidate.apply_operation_mut(envelope, now)?;
        *self = candidate;
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn apply_operation_mut(
        &mut self,
        envelope: &ProductionCommandEnvelope,
        now: SimTime,
    ) -> Result<(), CanwuError> {
        if envelope.expected_runtime_revision != self.revision {
            return Err(CanwuError::new(
                ErrorCode::DomainRecordVersionConflict,
                "production command expected a stale runtime revision",
            ));
        }
        let affected_site = self.operation_site(&envelope.operation);
        let affected_facility = self.operation_facility(&envelope.operation);
        match &envelope.operation {
            ProductionOperation::RequestCompletionLease { request } => {
                self.ensure_new_lifecycle_capacity()?;
                if request.holder != envelope.holder {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidAuthority,
                        "production completion request holder is not the command holder",
                    ));
                }
                self.request_completion_acquisition(request.clone())?;
            }
            ProductionOperation::AbortCompletionLease { request } => {
                if request.holder != envelope.holder {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidAuthority,
                        "production completion abort holder is not the command holder",
                    ));
                }
                self.abort_completion_acquisition(request)?;
            }
            ProductionOperation::CreateWorkOrder { work_order } => {
                self.ensure_new_lifecycle_capacity()?;
                if work_order.holder != envelope.holder
                    || work_order.lifecycle != WorkOrderLifecycle::Proposed
                    || work_order.execution.is_some()
                    || work_order.created_at != now
                    || self.work_orders.contains_key(&work_order.id)
                {
                    return Err(invalid(
                        "new work order does not match its holder or initial state",
                    ));
                }
                let site = self
                    .sites
                    .get(&work_order.site)
                    .ok_or_else(|| invalid("work order site is unavailable"))?;
                if !site.active || site.holder != envelope.holder {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidAuthority,
                        "work order holder does not own the active site",
                    ));
                }
                let process = self
                    .processes
                    .get(&work_order.process)
                    .ok_or_else(|| invalid("work order process is unavailable"))?;
                validate_process_time(process, now)?;
                self.work_orders
                    .insert(work_order.id.clone(), work_order.clone());
            }
            ProductionOperation::AuthorizeWorkOrder { work_order } => {
                let order = self
                    .work_orders
                    .get_mut(work_order)
                    .ok_or_else(|| invalid("work order is unavailable"))?;
                ensure_holder(order, &envelope.holder)?;
                if order.lifecycle != WorkOrderLifecycle::Proposed {
                    return Err(invalid("only a proposed work order may be authorized"));
                }
                order.lifecycle = WorkOrderLifecycle::Authorized;
                order.expected_revision = order
                    .expected_revision
                    .checked_add(1)
                    .ok_or_else(|| invalid("work order revision overflowed"))?;
            }
            ProductionOperation::StartExecution {
                execution,
                allocations,
            } => {
                self.start_execution(&envelope.holder, execution, allocations, now)?;
            }
            ProductionOperation::AdvanceExecution {
                execution,
                completed_units,
            } => {
                let record = self
                    .executions
                    .get(execution)
                    .ok_or_else(|| invalid("execution is unavailable"))?;
                let order = &self.work_orders[&record.work_order];
                ensure_holder(order, &envelope.holder)?;
                if record.lifecycle != WorkOrderLifecycle::Running || *completed_units == 0 {
                    return Err(invalid(
                        "only a running execution may advance positive work",
                    ));
                }
                let wip_id = WorkInProgressId::new(format!("canwu.production:wip:{execution}"))?;
                let wip = self
                    .work_in_progress
                    .get_mut(&wip_id)
                    .ok_or_else(|| invalid("execution work in progress is unavailable"))?;
                wip.completed_units = wip
                    .completed_units
                    .checked_add(*completed_units)
                    .ok_or_else(|| invalid("work progress overflowed"))?;
                if wip.completed_units > wip.total_units {
                    return Err(invalid("work progress exceeds its total units"));
                }
                wip.updated_at = now;
            }
            ProductionOperation::CompleteExecution { execution } => {
                self.complete_execution(&envelope.holder, execution, now)?;
            }
            ProductionOperation::CancelWorkOrder { work_order } => {
                self.cancel_work_order(&envelope.holder, work_order, now)?;
            }
            ProductionOperation::ResolveDegradedFacility {
                work_order,
                facility,
                choice,
                decision_ticket: _,
            } => {
                self.resolve_degraded_choice(&envelope.holder, work_order, facility, *choice)?;
            }
            ProductionOperation::CreateFacilityProject { project } => {
                self.ensure_new_project_lifecycle_capacity(project)?;
                let process = self
                    .processes
                    .get(&project.process)
                    .ok_or_else(|| invalid("facility project process is unavailable"))?;
                let facility = self
                    .facilities
                    .get(&project.facility)
                    .ok_or_else(|| {
                        invalid(
                            "facility project requires an existing authoritative planned or repairing asset",
                        )
                    })?;
                if project.holder != envelope.holder
                    || project.lifecycle != FacilityProjectLifecycle::Planned
                    || project.created_at != now
                    || project.started_at.is_some()
                    || project.completed_at.is_some()
                    || project.result_evidence_digest.is_some()
                    || project.completed_units != 0
                    || project.total_units != process.work_units
                    || project.inputs.len() != process.inputs.len()
                    || project.resulting_asset.is_some()
                    || project.base_generation != facility.generation
                    || project.resulting_generation != facility.generation.saturating_add(1)
                    || project.site != facility.site
                    || match project.kind {
                        FacilityProjectKind::Construction => !matches!(
                            facility.lifecycle,
                            FacilityLifecycle::Planned | FacilityLifecycle::Authorized
                        ),
                        FacilityProjectKind::Repair => {
                            facility.lifecycle != FacilityLifecycle::Repairing
                        }
                    }
                    || self.facility_projects.contains_key(&project.id)
                    || self.facility_projects.values().any(|existing| {
                        existing.facility == project.facility
                            && existing.base_generation == project.base_generation
                            && !matches!(
                                existing.lifecycle,
                                FacilityProjectLifecycle::Completed
                                    | FacilityProjectLifecycle::Cancelled
                                    | FacilityProjectLifecycle::Failed
                            )
                    })
                    || self
                        .sites
                        .get(&project.site)
                        .is_none_or(|site| site.holder != envelope.holder)
                {
                    return Err(invalid(
                        "facility project does not match its holder or initial state",
                    ));
                }
                validate_process_time(process, now)?;
                validate_project_completion_certificate(project)?;
                let grant = self
                    .production_completion_grants
                    .get(&project.production_completion_grant)
                    .ok_or_else(|| invalid("facility project completion grant is unavailable"))?;
                let resource_grant = self
                    .completion_participant_grants
                    .get(&project.completion_certificate.acquisition)
                    .and_then(|participants| participants.get(canwu_resource::PLUGIN_NAME))
                    .map(|participant| &participant.grant)
                    .ok_or_else(|| {
                        invalid("facility project resource completion grant is unavailable")
                    })?;
                if self
                    .production_completion_certificates
                    .get(&project.completion_certificate.acquisition)
                    != Some(&project.completion_certificate)
                    || grant.acquisition != project.completion_certificate.acquisition
                    || grant.operation_key != project.operation_key
                    || grant.state != CompletionGrantStateV1::Prepared
                    || resource_grant.id != project.resource_completion_grant
                    || resource_grant.operation_key != project.operation_key
                    || resource_grant.state != CompletionGrantStateV1::Consumed
                {
                    return Err(invalid(
                        "facility project completion authority is not prepared and exact",
                    ));
                }
                validate_requirements(process, &project.evidence)?;
                validate_technology_binding(process, &project.technology)?;
                validate_resource_inputs(process, &project.inputs, 1)?;
                let reservation = required_project_operation_outcome_reservation(project)?;
                self.project_operation_outcome_reservations
                    .insert(project.id.clone(), reservation);
                self.facility_projects
                    .insert(project.id.clone(), project.clone());
            }
            ProductionOperation::AuthorizeFacilityProject { project } => {
                let reservation = {
                    let project = self
                        .facility_projects
                        .get_mut(project)
                        .ok_or_else(|| invalid("facility project is unavailable"))?;
                    if project.holder != envelope.holder {
                        return Err(CanwuError::new(
                            ErrorCode::InvalidAuthority,
                            "facility project holder is not authorized",
                        ));
                    }
                    if project.lifecycle != FacilityProjectLifecycle::Planned {
                        return Err(invalid("only a planned facility project may be authorized"));
                    }
                    if now != project.created_at {
                        return Err(invalid(
                            "facility project authorization must drain at its certified eligibility time",
                        ));
                    }
                    project.lifecycle = FacilityProjectLifecycle::Authorized;
                    required_project_operation_outcome_reservation(project)?
                };
                if self
                    .project_operation_outcome_reservations
                    .insert(project.clone(), reservation)
                    .is_none()
                {
                    return Err(invalid(
                        "facility project lost its operation outcome reservation",
                    ));
                }
            }
            ProductionOperation::AdvanceFacilityProject {
                project,
                completed_units,
            } => {
                let current = self
                    .facility_projects
                    .get(project)
                    .ok_or_else(|| invalid("facility project is unavailable"))?
                    .clone();
                if current.holder != envelope.holder
                    || *completed_units == 0
                    || !matches!(
                        current.lifecycle,
                        FacilityProjectLifecycle::Authorized | FacilityProjectLifecycle::InProgress
                    )
                {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidAuthority,
                        "facility project holder or lifecycle is not authorized",
                    ));
                }
                if current.lifecycle == FacilityProjectLifecycle::Authorized {
                    if now != current.created_at {
                        return Err(invalid(
                            "facility project activation must drain at its certified eligibility time",
                        ));
                    }
                    let (certificate, grant) = self.consume_local_completion_grant(
                        &current.completion_certificate.acquisition,
                        now,
                    )?;
                    if certificate != current.completion_certificate
                        || grant != current.production_completion_grant
                    {
                        return Err(invalid(
                            "facility project activation differs from its coordinator-owned certificate and grant",
                        ));
                    }
                }
                // A project admits and pins one exact process revision at
                // creation. Later expiry prevents new projects but cannot
                // strand already-acquired inputs, locks, or completion grants.
                let reservation = {
                    let project = self
                        .facility_projects
                        .get_mut(project)
                        .expect("facility project was checked");
                    project.completed_units = project
                        .completed_units
                        .checked_add(*completed_units)
                        .ok_or_else(|| invalid("facility project progress overflowed"))?;
                    if project.completed_units > project.total_units {
                        return Err(invalid("facility project progress exceeds its total"));
                    }
                    project.started_at.get_or_insert(now);
                    if project.completed_units == project.total_units {
                        project.completed_at = Some(now);
                        let base = self
                            .facilities
                            .get(&project.facility)
                            .ok_or_else(|| invalid("facility project source asset disappeared"))?;
                        if base.generation != project.base_generation
                            || match project.kind {
                                FacilityProjectKind::Construction => !matches!(
                                    base.lifecycle,
                                    FacilityLifecycle::Planned | FacilityLifecycle::Authorized
                                ),
                                FacilityProjectKind::Repair => {
                                    base.lifecycle != FacilityLifecycle::Repairing
                                }
                            }
                        {
                            return Err(invalid(
                                "facility project source asset changed before completion",
                            ));
                        }
                        let mut result = base.clone();
                        result.generation = project.resulting_generation;
                        result.lifecycle = FacilityLifecycle::Operational;
                        result.condition_per_mille = 1_000;
                        project.resulting_asset = Some(result);
                        project.lifecycle = FacilityProjectLifecycle::Commissioning;
                        project.result_evidence_digest =
                            Some(facility_project_result_digest(project)?);
                    } else {
                        project.lifecycle = FacilityProjectLifecycle::InProgress;
                    }
                    required_project_operation_outcome_reservation(project)?
                };
                if self
                    .project_operation_outcome_reservations
                    .insert(project.clone(), reservation)
                    .is_none()
                {
                    return Err(invalid(
                        "facility project lost its operation outcome reservation",
                    ));
                }
            }
            ProductionOperation::AcceptFacilityCommissioning { project } => {
                let project = self
                    .facility_projects
                    .get(project)
                    .ok_or_else(|| invalid("facility project is unavailable"))?
                    .clone();
                let expected_result_digest = facility_project_result_digest(&project)?;
                if project.holder != envelope.holder
                    || project.lifecycle != FacilityProjectLifecycle::Commissioning
                    || project.completed_units != project.total_units
                    || project.completed_at.is_none()
                    || project.resulting_asset.is_none()
                    || project.result_evidence_digest.as_deref()
                        != Some(expected_result_digest.as_str())
                {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidAuthority,
                        "facility commissioning is not ready for this holder",
                    ));
                }
                if self
                    .facilities
                    .get(&project.facility)
                    .is_none_or(|facility| {
                        facility.site != project.site
                            || facility.generation != project.base_generation
                    })
                {
                    return Err(invalid(
                        "facility project does not produce a newer site-local generation",
                    ));
                }
                self.facility_projects
                    .get_mut(&project.id)
                    .expect("facility project was checked")
                    .lifecycle = FacilityProjectLifecycle::CompletionPending;
                if self
                    .project_operation_outcome_reservations
                    .remove(&project.id)
                    != Some(1)
                {
                    return Err(invalid(
                        "facility commissioning lost its final operation outcome reservation",
                    ));
                }
            }
            ProductionOperation::RetireFacility {
                facility,
                expected_generation,
            } => {
                let facility = self
                    .facilities
                    .get_mut(facility)
                    .ok_or_else(|| invalid("retirement facility is unavailable"))?;
                let site = &self.sites[&facility.site];
                if site.holder != envelope.holder
                    || facility.generation != *expected_generation
                    || self.capacity_allocations.values().any(|allocation| {
                        allocation.facility == facility.id
                            && matches!(
                                allocation.state,
                                CapacityAllocationState::Reserved
                                    | CapacityAllocationState::Consumed
                            )
                    })
                    || self.facility_projects.values().any(|project| {
                        project.facility == facility.id
                            && project.base_generation == *expected_generation
                            && !matches!(
                                project.lifecycle,
                                FacilityProjectLifecycle::Completed
                                    | FacilityProjectLifecycle::Cancelled
                                    | FacilityProjectLifecycle::Failed
                            )
                    })
                {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidAuthority,
                        "facility retirement is unauthorized, stale, or still allocated",
                    ));
                }
                facility.lifecycle = FacilityLifecycle::Retired;
                facility.condition_per_mille = 0;
            }
        }
        if let Some(site) = affected_site {
            self.mark_site_observation_dirty(&site)?;
        }
        if let Some(facility) = affected_facility
            && self.facilities.get(&facility).is_some_and(|asset| {
                asset.incident_risk_per_mille > 0
                    && asset.condition_per_mille > 0
                    && matches!(
                        asset.lifecycle,
                        FacilityLifecycle::Operational
                            | FacilityLifecycle::Degraded
                            | FacilityLifecycle::Damaged
                    )
            })
            && (self.incident_due_index.len()
                < ProductionLimitsV1::canonical().max_incidents_per_boundary
                || self.incident_due_index.contains(&facility))
        {
            self.incident_due_index.insert(facility);
        }
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("production runtime revision overflowed"))?;
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn start_execution(
        &mut self,
        holder: &KnowledgeHolderRef,
        execution: &ProductionExecution,
        allocations: &[ProductionCapacityAllocation],
        now: SimTime,
    ) -> Result<(), CanwuError> {
        let order = self
            .work_orders
            .get(&execution.work_order)
            .ok_or_else(|| invalid("execution work order is unavailable"))?
            .clone();
        ensure_holder(&order, holder)?;
        if !matches!(
            order.lifecycle,
            WorkOrderLifecycle::Authorized | WorkOrderLifecycle::Ready
        ) || order.execution.is_some()
            || execution.lifecycle != WorkOrderLifecycle::Running
            || execution.started_at != now
            || self.executions.contains_key(&execution.id)
            || allocations.is_empty()
            || allocations.len() > MAX_REQUIREMENT_GROUPS
            || allocations
                .iter()
                .any(|allocation| self.capacity_allocations.contains_key(&allocation.id))
        {
            return Err(invalid(
                "execution cannot start from the supplied lifecycle state",
            ));
        }
        let process = self
            .processes
            .get(&order.process)
            .ok_or_else(|| invalid("execution process is unavailable"))?
            .clone();
        validate_process_time(&process, now)?;
        let allocation_ids = allocations
            .iter()
            .map(|allocation| allocation.id.clone())
            .collect::<Vec<_>>();
        let unique_ids = allocation_ids.iter().cloned().collect::<BTreeSet<_>>();
        if execution.process != order.process
            || execution.site != order.site
            || execution.work_order != order.id
            || execution.allocations != allocation_ids
            || unique_ids.len() != allocations.len()
            || allocations.iter().any(|allocation| {
                allocation.work_order != order.id
                    || allocation.execution != execution.id
                    || allocation.state != CapacityAllocationState::Reserved
            })
        {
            return Err(invalid(
                "execution, work order, and capacity allocation do not bind exactly",
            ));
        }
        validate_technology_binding(&process, &execution.technology)?;
        validate_completion_certificate(execution, now)?;
        self.consume_production_completion_capacity(execution, now)?;
        let blockers = self.blockers_for(&process, &execution.evidence);
        if !blockers.is_empty() {
            return Err(blocked(&blockers));
        }
        if execution.inputs.len() != process.inputs.len()
            || execution
                .inputs
                .iter()
                .zip(&process.inputs)
                .any(|(actual, required)| {
                    actual.quantity != required.quantity.saturating_mul(order.quantity)
                })
        {
            return Err(invalid(
                "execution inputs do not match the exact process quantities",
            ));
        }
        let facility = self
            .facilities
            .get(&execution.facility)
            .ok_or_else(|| invalid("execution facility is unavailable"))?;
        if execution.facility != facility.id
            || allocations.iter().any(|allocation| {
                allocation.facility != facility.id
                    || allocation.facility_generation != facility.generation
                    || allocation.start > now
                    || allocation.end <= now
            })
        {
            return Err(invalid(
                "execution facility allocation is stale or outside its interval",
            ));
        }
        let allocation_refs = allocations.iter().collect::<Vec<_>>();
        validate_capacity_cover(&process, &order, execution, &allocation_refs)?;
        for allocation in allocations {
            let mut allocation = allocation.clone();
            allocation.state = CapacityAllocationState::Consumed;
            self.capacity_allocations
                .insert(allocation.id.clone(), allocation);
        }
        self.executions
            .insert(execution.id.clone(), execution.clone());
        let total_units = process
            .work_units
            .checked_mul(order.quantity)
            .ok_or_else(|| invalid("work unit total overflowed"))?;
        let wip_id = WorkInProgressId::new(format!("canwu.production:wip:{}", execution.id))?;
        self.work_in_progress.insert(
            wip_id.clone(),
            WorkInProgress {
                id: wip_id,
                execution: execution.id.clone(),
                completed_units: 0,
                total_units,
                consumed_input_evidence: execution.inputs.clone(),
                recoverable_input_quantity: execution
                    .inputs
                    .iter()
                    .map(|value| value.quantity)
                    .sum(),
                non_recoverable_waste_quantity: 0,
                updated_at: now,
            },
        );
        let order = self
            .work_orders
            .get_mut(&order.id)
            .expect("work order was checked");
        order.lifecycle = WorkOrderLifecycle::Running;
        order.execution = Some(execution.id.clone());
        order.expected_revision += 1;
        self.validate_allocations()
    }

    fn complete_execution(
        &mut self,
        holder: &KnowledgeHolderRef,
        execution_id: &ProductionExecutionId,
        now: SimTime,
    ) -> Result<(), CanwuError> {
        let execution = self
            .executions
            .get(execution_id)
            .ok_or_else(|| invalid("execution is unavailable"))?;
        let order_id = execution.work_order.clone();
        ensure_holder(&self.work_orders[&order_id], holder)?;
        let wip_id = WorkInProgressId::new(format!("canwu.production:wip:{execution_id}"))?;
        let wip = self
            .work_in_progress
            .get(&wip_id)
            .ok_or_else(|| invalid("execution work in progress is unavailable"))?;
        if execution.lifecycle != WorkOrderLifecycle::Running
            || wip.completed_units != wip.total_units
        {
            return Err(invalid(
                "execution cannot complete before all work units are finished",
            ));
        }
        let execution = self
            .executions
            .get_mut(execution_id)
            .expect("execution was checked");
        execution.lifecycle = WorkOrderLifecycle::CompletedPendingOutputSettlement;
        execution.completed_at = Some(now);
        let order = self
            .work_orders
            .get_mut(&order_id)
            .expect("work order was checked");
        order.lifecycle = WorkOrderLifecycle::CompletedPendingOutputSettlement;
        order.expected_revision += 1;
        Ok(())
    }

    fn cancel_work_order(
        &mut self,
        holder: &KnowledgeHolderRef,
        work_order_id: &WorkOrderId,
        now: SimTime,
    ) -> Result<(), CanwuError> {
        let order = self
            .work_orders
            .get(work_order_id)
            .ok_or_else(|| invalid("work order is unavailable"))?
            .clone();
        ensure_holder(&order, holder)?;
        if matches!(
            order.lifecycle,
            WorkOrderLifecycle::Settled
                | WorkOrderLifecycle::Cancelled
                | WorkOrderLifecycle::Failed
                | WorkOrderLifecycle::CompletedPendingOutputSettlement
        ) {
            return Err(invalid(
                "terminal or output-pending work cannot be cancelled",
            ));
        }
        if let Some(execution_id) = &order.execution {
            let (allocation_ids, production_grant, acquisition) = self
                .executions
                .get(execution_id)
                .map(|execution| {
                    (
                        execution.allocations.clone(),
                        execution.production_completion_grant.clone(),
                        execution.completion_certificate.acquisition.clone(),
                    )
                })
                .ok_or_else(|| invalid("work order execution is unavailable"))?;
            let execution = self
                .executions
                .get_mut(execution_id)
                .expect("execution was checked above");
            execution.lifecycle = WorkOrderLifecycle::Cancelled;
            for allocation_id in allocation_ids {
                let allocation = self
                    .capacity_allocations
                    .get_mut(&allocation_id)
                    .ok_or_else(|| invalid("execution capacity allocation is unavailable"))?;
                allocation.state = CapacityAllocationState::Released;
            }
            let wip_id = WorkInProgressId::new(format!("canwu.production:wip:{execution_id}"))?;
            let wip = self
                .work_in_progress
                .get_mut(&wip_id)
                .ok_or_else(|| invalid("execution work in progress is unavailable"))?;
            let original = wip
                .consumed_input_evidence
                .iter()
                .try_fold(0_u64, |total, input| total.checked_add(input.quantity))
                .ok_or_else(|| invalid("cancellation input quantity overflowed"))?;
            let remaining_units = wip.total_units.saturating_sub(wip.completed_units);
            let recoverable = wip
                .consumed_input_evidence
                .iter()
                .try_fold(0_u64, |total, input| {
                    input
                        .quantity
                        .checked_mul(remaining_units)
                        .map(|scaled| scaled / wip.total_units)
                        .and_then(|quantity| total.checked_add(quantity))
                })
                .ok_or_else(|| invalid("cancellation recovery quantity overflowed"))?;
            let waste = original
                .checked_sub(recoverable)
                .ok_or_else(|| invalid("cancellation waste quantity underflowed"))?;
            wip.recoverable_input_quantity = recoverable;
            wip.non_recoverable_waste_quantity = waste;
            wip.updated_at = now;
            self.complete_production_completion_capacity(&production_grant, &acquisition)?;
            self.archive_due_index.insert(execution_id.clone());
        }
        let order = self
            .work_orders
            .get_mut(work_order_id)
            .expect("work order was checked");
        order.lifecycle = WorkOrderLifecycle::Cancelled;
        order.expected_revision += 1;
        Ok(())
    }

    pub(crate) fn ensure_operation_outcome_admission_capacity(
        &self,
        operation: &ProductionOperation,
    ) -> Result<(), CanwuError> {
        let limits = ProductionLimitsV1::canonical();
        let reserved = self
            .project_operation_outcome_reservations
            .values()
            .try_fold(0_usize, |total, reserved| {
                usize::try_from(*reserved)
                    .ok()
                    .and_then(|reserved| total.checked_add(reserved))
            })
            .ok_or_else(|| invalid("production project outcome reservation overflowed"))?;
        let additional = match operation {
            ProductionOperation::CreateFacilityProject { project } => {
                usize::try_from(required_project_operation_outcome_reservation(project)?)
                    .ok()
                    .and_then(|reserved| reserved.checked_add(1))
                    .ok_or_else(|| invalid("production project outcome reservation overflowed"))?
            }
            ProductionOperation::AuthorizeFacilityProject { project }
            | ProductionOperation::AdvanceFacilityProject { project, .. }
            | ProductionOperation::AcceptFacilityCommissioning { project }
                if self
                    .project_operation_outcome_reservations
                    .get(project)
                    .is_some_and(|reserved| *reserved > 0) =>
            {
                0
            }
            _ => 1,
        };
        if self
            .operation_outcomes
            .len()
            .checked_add(reserved)
            .and_then(|used| used.checked_add(additional))
            .is_none_or(|used| used > limits.max_operation_outcomes)
        {
            return Err(CanwuError::new(
                ErrorCode::ValueOutOfRange,
                "archive_backpressure: production operation outcome capacity is reserved for accepted facility projects",
            ));
        }
        Ok(())
    }

    pub(crate) fn project_operation_uses_reserved_outcome_at_capacity(
        &self,
        operation: &ProductionOperation,
    ) -> Result<bool, CanwuError> {
        let Some(project) = (match operation {
            ProductionOperation::AuthorizeFacilityProject { project }
            | ProductionOperation::AdvanceFacilityProject { project, .. }
            | ProductionOperation::AcceptFacilityCommissioning { project } => Some(project),
            _ => None,
        }) else {
            return Ok(false);
        };
        if self
            .project_operation_outcome_reservations
            .get(project)
            .is_none_or(|reserved| *reserved == 0)
        {
            return Ok(false);
        }
        let reserved = self
            .project_operation_outcome_reservations
            .values()
            .try_fold(0_usize, |total, reserved| {
                usize::try_from(*reserved)
                    .ok()
                    .and_then(|reserved| total.checked_add(reserved))
            })
            .ok_or_else(|| invalid("production project outcome reservation overflowed"))?;
        Ok(self
            .operation_outcomes
            .len()
            .checked_add(reserved)
            .is_some_and(|used| used >= ProductionLimitsV1::canonical().max_operation_outcomes))
    }

    fn ensure_new_project_lifecycle_capacity(
        &self,
        project: &FacilityProject,
    ) -> Result<(), CanwuError> {
        let future_outcomes =
            usize::try_from(required_project_operation_outcome_reservation(project)?)
                .map_err(|_| invalid("production project outcome reservation overflowed"))?;
        let required_outcomes = future_outcomes
            .checked_add(1)
            .ok_or_else(|| invalid("production project outcome reservation overflowed"))?;
        self.ensure_new_lifecycle_capacity_with_outcomes(required_outcomes)
    }

    fn ensure_new_lifecycle_capacity(&self) -> Result<(), CanwuError> {
        self.ensure_new_lifecycle_capacity_with_outcomes(1)
    }

    fn ensure_new_lifecycle_capacity_with_outcomes(
        &self,
        additional_outcomes: usize,
    ) -> Result<(), CanwuError> {
        let limits = ProductionLimitsV1::canonical();
        let active_completion = self
            .completion_acquisitions
            .values()
            .filter(|acquisition| {
                !matches!(
                    acquisition.state,
                    canwu_resource::CompletionLeaseAcquisitionStateV1::Released
                        | canwu_resource::CompletionLeaseAcquisitionStateV1::Expired
                )
            })
            .count();
        let project_outcome_reservations = self
            .project_operation_outcome_reservations
            .values()
            .try_fold(0_usize, |total, reserved| {
                usize::try_from(*reserved)
                    .ok()
                    .and_then(|reserved| total.checked_add(reserved))
            })
            .ok_or_else(|| invalid("production project outcome reservation overflowed"))?;
        let reserved_terminal = self
            .archive_due_index
            .len()
            .checked_add(self.project_archive_due_index.len())
            .and_then(|value| value.checked_add(self.archive.pending_handles.len()))
            .and_then(|value| value.checked_add(active_completion))
            .and_then(|value| value.checked_add(project_outcome_reservations))
            .and_then(|value| value.checked_add(self.operation_outcomes.len()))
            .and_then(|value| value.checked_add(additional_outcomes))
            .ok_or_else(|| invalid("production terminal capacity accounting overflowed"))?;
        if self
            .archive_due_index
            .len()
            .checked_add(self.project_archive_due_index.len())
            .is_none_or(|count| count >= limits.max_archive_due)
            || self.archive.pending_handles.len() >= limits.max_pending_retention_handles
            || reserved_terminal > limits.max_operation_outcomes
        {
            return Err(CanwuError::new(
                ErrorCode::ValueOutOfRange,
                "archive_backpressure: production terminal receipt/report capacity is unavailable",
            ));
        }
        Ok(())
    }

    fn resolve_degraded_choice(
        &mut self,
        holder: &KnowledgeHolderRef,
        work_order_id: &WorkOrderId,
        facility_id: &FacilityAssetId,
        choice: DegradedFacilityChoice,
    ) -> Result<(), CanwuError> {
        let order = self
            .work_orders
            .get(work_order_id)
            .ok_or_else(|| invalid("work order is unavailable"))?
            .clone();
        ensure_holder(&order, holder)?;
        let facility = self
            .facilities
            .get_mut(facility_id)
            .ok_or_else(|| invalid("facility is unavailable"))?;
        if !matches!(
            facility.lifecycle,
            FacilityLifecycle::Degraded | FacilityLifecycle::Damaged
        ) {
            return Err(invalid(
                "degraded-facility choice requires a degraded or damaged facility",
            ));
        }
        match choice {
            DegradedFacilityChoice::ContinueDegraded => {
                facility.condition_per_mille = facility.condition_per_mille.saturating_sub(50);
                if order.lifecycle == WorkOrderLifecycle::Authorized {
                    self.work_orders
                        .get_mut(work_order_id)
                        .expect("work order was checked")
                        .lifecycle = WorkOrderLifecycle::Ready;
                }
            }
            DegradedFacilityChoice::StopForRepair => {
                facility.lifecycle = FacilityLifecycle::Repairing;
                if let Some(execution) = &order.execution {
                    let allocation_ids = self
                        .executions
                        .get(execution)
                        .map(|value| value.allocations.clone())
                        .unwrap_or_default();
                    for allocation_id in allocation_ids {
                        if let Some(allocation) = self.capacity_allocations.get_mut(&allocation_id)
                        {
                            allocation.state = CapacityAllocationState::Released;
                        }
                    }
                }
            }
            DegradedFacilityChoice::DeferOrder => {
                self.work_orders
                    .get_mut(work_order_id)
                    .expect("work order was checked")
                    .lifecycle = WorkOrderLifecycle::Authorized;
            }
        }
        Ok(())
    }

    pub fn acknowledge_output(
        &mut self,
        acknowledgement: &ProductionOutputAcknowledgement,
        now: SimTime,
    ) -> Result<bool, CanwuError> {
        let mut candidate = self.clone();
        let changed = candidate.acknowledge_output_mut(acknowledgement, now)?;
        *self = candidate;
        Ok(changed)
    }

    fn acknowledge_output_mut(
        &mut self,
        acknowledgement: &ProductionOutputAcknowledgement,
        now: SimTime,
    ) -> Result<bool, CanwuError> {
        let execution = self
            .executions
            .get(&acknowledgement.execution)
            .ok_or_else(|| invalid("output acknowledgement execution is unavailable"))?
            .clone();
        if execution.lifecycle == WorkOrderLifecycle::Settled {
            if execution.output_outcomes == acknowledgement.outcomes
                && execution.output_source.as_ref() == Some(&acknowledgement.production_source)
            {
                return Ok(false);
            }
            return Err(CanwuError::new(
                ErrorCode::IdempotencyConflict,
                "settled production output received a different resource outcome",
            ));
        }
        if execution.lifecycle != WorkOrderLifecycle::CompletedPendingOutputSettlement
            || execution
                .output_source
                .as_ref()
                .is_some_and(|source| source != &acknowledgement.production_source)
            || acknowledgement.outcomes.len() != execution.output_requests.len()
            || acknowledgement
                .outcomes
                .iter()
                .zip(&execution.output_requests)
                .any(|(outcome, request)| {
                    outcome.validate().is_err()
                        || outcome.operation_key != request.operation_key
                        || !matches!(
                            outcome.status,
                            ResourceOperationStatus::Applied | ResourceOperationStatus::Duplicate
                        )
                        || outcome.kind != ResourceOperationKind::Credit
                        || outcome.quantity != request.quantity
                        || outcome.remainder != 0
                        || outcome.exact_evidence != vec![acknowledgement.production_source.clone()]
                        || outcome.semantic_digest.is_empty()
                })
            || !acknowledgement
                .production_source
                .record
                .kind
                .matches_type::<ProductionRuntimeRecord>()
        {
            return Err(invalid(
                "resource output outcome does not exactly settle the production request",
            ));
        }
        let execution_mut = self
            .executions
            .get_mut(&acknowledgement.execution)
            .expect("execution was checked");
        execution_mut.lifecycle = WorkOrderLifecycle::Settled;
        execution_mut
            .output_outcomes
            .clone_from(&acknowledgement.outcomes);
        execution_mut.output_source = Some(acknowledgement.production_source.clone());
        execution_mut.output_ack_digest = Some(canonical_hash(
            "canwu.production.output-ack.v1",
            acknowledgement,
        )?);
        self.complete_production_completion_capacity(
            &execution.production_completion_grant,
            &execution.completion_certificate.acquisition,
        )?;
        for allocation_id in &execution.allocations {
            let allocation = self
                .capacity_allocations
                .get_mut(allocation_id)
                .expect("allocation closure was validated");
            allocation.state = CapacityAllocationState::Released;
        }
        let order = self
            .work_orders
            .get_mut(&execution.work_order)
            .expect("work order closure was validated");
        order.lifecycle = WorkOrderLifecycle::Settled;
        order.expected_revision += 1;
        self.mark_site_observation_dirty(&execution.site)?;
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("production runtime revision overflowed"))?;
        let _ = now;
        self.validate()?;
        Ok(true)
    }

    /// Close the production-side mirror after the authoritative resource
    /// participant reports terminal completion. Canonical runs invoke this
    /// through the production completion ingress after resolving the exact
    /// resource runtime version and grant.
    pub fn finalize_execution_resource_completion(
        &mut self,
        acquisition: &CompletionLeaseAcquisitionId,
        provider_source: &DomainRecordVersionRef,
        resource_grant: &CompletionCapacityGrantV1,
    ) -> Result<ProductionExecutionId, CanwuError> {
        let execution_id = self
            .executions
            .values()
            .filter(|execution| {
                execution.completion_certificate.acquisition == *acquisition
                    && execution.lifecycle == WorkOrderLifecycle::Settled
                    && !self.archive_due_index.contains(&execution.id)
            })
            .map(|execution| execution.id.clone())
            .next()
            .ok_or_else(|| {
                invalid("resource completion acknowledgement has no settled production execution")
            })?;
        if self
            .executions
            .values()
            .filter(|execution| {
                execution.completion_certificate.acquisition == *acquisition
                    && execution.lifecycle == WorkOrderLifecycle::Settled
                    && !self.archive_due_index.contains(&execution.id)
            })
            .count()
            != 1
        {
            return Err(invalid(
                "resource completion acknowledgement ambiguously names production executions",
            ));
        }
        let execution = self.executions[&execution_id].clone();
        if resource_grant.id != execution.resource_completion_grant
            || resource_grant.acquisition != *acquisition
            || resource_grant.operation_key != execution.completion_certificate.operation_key
            || resource_grant.state != CompletionGrantStateV1::Completed
        {
            return Err(invalid(
                "completed resource participant grant differs from the settled production execution",
            ));
        }
        self.acknowledge_completed_participant(
            acquisition,
            canwu_resource::PLUGIN_NAME,
            provider_source,
            resource_grant,
        )?;
        self.archive_due_index.insert(execution_id.clone());
        Ok(execution_id)
    }

    pub(crate) fn finalize_facility_project_completion(
        &mut self,
        acquisition: &CompletionLeaseAcquisitionId,
        provider_source: &DomainRecordVersionRef,
        resource_grant: &CompletionCapacityGrantV1,
        now: SimTime,
    ) -> Result<FacilityProjectId, CanwuError> {
        if resource_grant.state != CompletionGrantStateV1::Completed
            || !provider_source
                .record
                .kind
                .matches_type::<canwu_resource::ResourceRuntimeRecord>()
        {
            return Err(invalid(
                "facility project completion requires an exact completed resource participant grant",
            ));
        }
        let project_id = self
            .facility_projects
            .values()
            .filter(|project| {
                project.completion_certificate.acquisition == *acquisition
                    && project.lifecycle == FacilityProjectLifecycle::CompletionPending
            })
            .map(|project| project.id.clone())
            .next()
            .ok_or_else(|| {
                invalid("resource completion acknowledgement has no pending facility project")
            })?;
        if self
            .facility_projects
            .values()
            .filter(|project| {
                project.completion_certificate.acquisition == *acquisition
                    && project.lifecycle == FacilityProjectLifecycle::CompletionPending
            })
            .count()
            != 1
        {
            return Err(invalid(
                "resource completion acknowledgement ambiguously names facility projects",
            ));
        }
        let project = self
            .facility_projects
            .get(&project_id)
            .expect("pending facility project was selected")
            .clone();
        if resource_grant.id != project.resource_completion_grant
            || resource_grant.acquisition != *acquisition
            || resource_grant.operation_key != project.operation_key
        {
            return Err(invalid(
                "completed resource participant grant differs from the pending facility project",
            ));
        }
        self.acknowledge_completed_participant(
            acquisition,
            canwu_resource::PLUGIN_NAME,
            provider_source,
            resource_grant,
        )?;
        let base = self.facilities.get(&project.facility).ok_or_else(|| {
            invalid("facility project source asset disappeared before settlement")
        })?;
        if base.site != project.site || base.generation != project.base_generation {
            return Err(invalid(
                "facility project source asset changed before resource completion acknowledgement",
            ));
        }
        let resulting_asset = project
            .resulting_asset
            .clone()
            .ok_or_else(|| invalid("facility project completion lost its authoritative result"))?;
        self.facilities
            .insert(project.facility.clone(), resulting_asset);
        let project_mut = self
            .facility_projects
            .get_mut(&project_id)
            .expect("pending facility project was selected");
        project_mut.lifecycle = FacilityProjectLifecycle::Completed;
        project_mut.completed_at = Some(now);
        project_mut.result_evidence_digest = Some(facility_project_result_digest(project_mut)?);
        self.complete_local_completion_grant(acquisition, &project.production_completion_grant)?;
        self.resource_continuation_witnesses.remove(&project_id);
        self.project_archive_due_index.insert(project_id.clone());
        self.mark_site_observation_dirty(&project.site)?;
        Ok(project_id)
    }

    fn consume_production_completion_capacity(
        &mut self,
        execution: &ProductionExecution,
        at: SimTime,
    ) -> Result<(), CanwuError> {
        validate_completion_certificate(execution, at)?;
        let (certificate, grant) =
            self.consume_local_completion_grant(&execution.completion_certificate.acquisition, at)?;
        if certificate != execution.completion_certificate
            || grant != execution.production_completion_grant
        {
            return Err(invalid(
                "execution completion fields differ from the coordinator-owned certificate and grant",
            ));
        }
        Ok(())
    }

    fn complete_production_completion_capacity(
        &mut self,
        grant_id: &canwu_resource::CompletionCapacityGrantId,
        acquisition: &CompletionLeaseAcquisitionId,
    ) -> Result<(), CanwuError> {
        self.complete_local_completion_grant(acquisition, grant_id)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub(crate) fn apply_incident_transition(
        &mut self,
        transition: ProductionIncidentTransitionV1,
    ) -> Result<(), CanwuError> {
        let mut detached = transition.clone();
        let recorded = std::mem::take(&mut detached.canonical_digest);
        if recorded.is_empty()
            || recorded != canonical_hash("canwu.production.incident-transition.v1", &detached)?
            || transition.condition_after >= transition.condition_before
            || transition.random.trigger.upper_exclusive != 1_000
            || transition.random.trigger.value >= 1_000
            || transition.random.stream != transition.random.trigger.stream
        {
            return Err(invalid(
                "production incident transition evidence is invalid",
            ));
        }
        let trigger_address = match &transition.random.trigger.address {
            RandomDrawAddress::OperationV1(address) => address,
            RandomDrawAddress::Sequential { .. } => {
                return Err(invalid(
                    "production incident trigger must use an operation-keyed draw",
                ));
            }
        };
        if trigger_address.producer_plugin != PLUGIN_NAME
            || trigger_address.operation_kind != "production_facility_incident"
            || trigger_address.application_operation_id != transition.operation_key
            || trigger_address.draw_slot != 0
            || trigger_address.target
                != RandomOperationTarget::CanonicalKey(transition.facility.to_string())
        {
            return Err(invalid(
                "production incident trigger address is not the exact facility operation",
            ));
        }
        let facility = self
            .facilities
            .get_mut(&transition.facility)
            .ok_or_else(|| invalid("incident facility disappeared"))?;
        let severity = transition
            .random
            .severity
            .as_ref()
            .ok_or_else(|| invalid("selected production incident lacks a severity draw"))?;
        let severity_address = match &severity.address {
            RandomDrawAddress::OperationV1(address) => address,
            RandomDrawAddress::Sequential { .. } => {
                return Err(invalid(
                    "production incident severity must use an operation-keyed draw",
                ));
            }
        };
        let expected_loss = u16::try_from(severity.value.saturating_add(1))
            .map_err(|_| invalid("production incident severity exceeds its integer bound"))?
            .min(transition.condition_before);
        if facility.generation != transition.expected_generation
            || facility.condition_per_mille != transition.condition_before
            || transition.random.trigger.value >= u64::from(facility.incident_risk_per_mille)
            || severity.stream != transition.random.stream
            || severity.upper_exclusive != u64::from(facility.incident_max_severity_per_mille)
            || severity.value >= severity.upper_exclusive
            || severity_address.producer_plugin != PLUGIN_NAME
            || severity_address.operation_kind != "production_facility_incident"
            || severity_address.application_operation_id != transition.operation_key
            || severity_address.draw_slot != 1
            || severity_address.target
                != RandomOperationTarget::CanonicalKey(transition.facility.to_string())
            || transition.condition_after
                != facility.condition_per_mille.saturating_sub(expected_loss)
        {
            return Err(invalid(
                "incident transition no longer matches its facility generation and condition",
            ));
        }
        if self
            .incident_receipts
            .insert(transition.operation_key.clone(), transition.clone())
            .is_some()
        {
            return Err(CanwuError::new(
                ErrorCode::IdempotencyConflict,
                "incident operation key was reused",
            ));
        }
        facility.condition_per_mille = transition.condition_after;
        facility.lifecycle = transition.lifecycle_after;
        let site = facility.site.clone();
        self.mark_site_observation_dirty(&site)?;
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("production runtime revision overflowed"))?;
        Ok(())
    }

    pub fn rebuild_runtime_indexes(&mut self) -> Result<(), CanwuError> {
        self.refresh_incident_due_index();
        self.observation_dirty_index.clear();
        for grant in self.observer_grants.values() {
            for scope in &grant.sites {
                self.observation_dirty_index
                    .insert(ProductionObservationHeadKeyV1 {
                        holder: grant.holder.clone(),
                        scope: scope.clone(),
                    });
            }
        }
        self.archive_due_index = self
            .executions
            .values()
            .filter(|execution| {
                matches!(
                    execution.lifecycle,
                    WorkOrderLifecycle::Settled
                        | WorkOrderLifecycle::Cancelled
                        | WorkOrderLifecycle::Failed
                )
            })
            .map(|execution| execution.id.clone())
            .collect();
        self.project_archive_due_index = self
            .facility_projects
            .values()
            .filter(|project| project.lifecycle == FacilityProjectLifecycle::Completed)
            .map(|project| project.id.clone())
            .collect();
        self.validate_runtime_indexes()
    }

    pub(crate) fn advance_incident_cursor(
        &mut self,
        evaluated: Option<FacilityAssetId>,
    ) -> Result<(), CanwuError> {
        if let Some(evaluated) = evaluated {
            self.incident_cursor = Some(evaluated);
            self.incident_round = self
                .incident_round
                .checked_add(1)
                .ok_or_else(|| invalid("production incident fairness round overflowed"))?;
            self.revision = self
                .revision
                .checked_add(1)
                .ok_or_else(|| invalid("production runtime revision overflowed"))?;
        }
        self.refresh_incident_due_index();
        Ok(())
    }

    fn refresh_incident_due_index(&mut self) {
        let eligible = self
            .facilities
            .values()
            .filter(|facility| {
                facility.incident_risk_per_mille > 0
                    && facility.condition_per_mille > 0
                    && matches!(
                        facility.lifecycle,
                        FacilityLifecycle::Operational
                            | FacilityLifecycle::Degraded
                            | FacilityLifecycle::Damaged
                    )
            })
            .map(|facility| facility.id.clone())
            .collect::<Vec<_>>();
        let maximum = ProductionLimitsV1::canonical().max_incidents_per_boundary;
        let mut selected = eligible
            .iter()
            .filter(|id| {
                self.incident_cursor
                    .as_ref()
                    .is_none_or(|cursor| *id > cursor)
            })
            .take(maximum)
            .cloned()
            .collect::<BTreeSet<_>>();
        if selected.is_empty() && !eligible.is_empty() {
            selected.extend(eligible.into_iter().take(maximum));
        }
        self.incident_due_index = selected;
    }

    pub(crate) fn mark_site_observation_dirty(
        &mut self,
        site: &ProductionSiteId,
    ) -> Result<(), CanwuError> {
        for grant in self
            .observer_grants
            .values()
            .filter(|grant| grant.sites.contains(site))
        {
            self.observation_dirty_index
                .insert(ProductionObservationHeadKeyV1 {
                    holder: grant.holder.clone(),
                    scope: site.clone(),
                });
        }
        if self.observation_dirty_index.len()
            > ProductionLimitsV1::canonical().max_observation_dirty
        {
            return Err(invalid(
                "production observation dirty index exceeds its canonical cap",
            ));
        }
        Ok(())
    }

    pub fn materialize_observation_head(
        &mut self,
        key: &ProductionObservationHeadKeyV1,
        now: SimTime,
        source_evidence: EvidenceRef,
    ) -> Result<ProductionObservationHeadV1, CanwuError> {
        let grant = self
            .observer_grants
            .values()
            .find(|grant| grant.holder == key.holder && grant.sites.contains(&key.scope))
            .cloned()
            .ok_or_else(|| invalid("production observation key lost its holder grant"))?;
        let (facts, blockers) = self.observation_facts(&grant, &key.scope)?;
        let mut head = ProductionObservationHeadV1 {
            key: key.clone(),
            role: grant.role,
            observed_at: now,
            materialized_at: now,
            provider_state_revision: self.revision,
            facts,
            blockers,
            source_evidence: vec![source_evidence],
            canonical_digest: String::new(),
        };
        head.canonical_digest = canonical_hash("canwu.production.observation-head.v1", &head)?;
        let storage_key = production_observation_head_storage_key(key)?;
        let heads = self.observation_heads.entry(storage_key).or_default();
        if heads.last().is_some_and(|existing| {
            existing.observed_at > now
                || (existing.observed_at == now
                    && existing.provider_state_revision >= self.revision)
        }) {
            return Err(invalid(
                "production observation heads must advance by observed cut or provider revision",
            ));
        }
        heads.push(head.clone());
        if heads.len() > MAX_OBSERVATION_HEADS_PER_SCOPE {
            if self.observation_rollover.len() >= MAX_HOT_RECEIPTS {
                return Err(CanwuError::new(
                    ErrorCode::ValueOutOfRange,
                    "archive_backpressure: production observation rollover is full",
                ));
            }
            let rolled = heads.remove(0);
            self.observation_rollover
                .insert(rolled.canonical_digest.clone(), rolled);
        }
        self.observation_dirty_index.remove(key);
        for keys in self.observation_due_index.values_mut() {
            keys.remove(key);
        }
        self.observation_due_index
            .retain(|_, keys| !keys.is_empty());
        let delay = i64::try_from(grant.delay_minutes)
            .map_err(|_| invalid("production observation delay exceeds simulation time"))?;
        let due = now
            .checked_add(canwu_api::SimDuration::minutes(delay))
            .ok_or_else(|| invalid("production observation delivery time overflowed"))?;
        self.observation_due_index
            .entry(due)
            .or_default()
            .insert(key.clone());
        Ok(head)
    }

    fn observation_facts(
        &self,
        grant: &ProductionObserverGrant,
        scope: &ProductionSiteId,
    ) -> Result<(Vec<ProductionObservationFact>, Vec<ProductionBlocker>), CanwuError> {
        let mut facts = Vec::new();
        for facility in self
            .facilities
            .values()
            .filter(|facility| &facility.site == scope)
        {
            let condition = u64::from(facility.condition_per_mille);
            facts.push(ProductionObservationFact {
                subject: facility.id.to_string(),
                state: format!("{:?}", facility.lifecycle).to_lowercase(),
                quantity_low: if grant.role == ProductionObservationRole::RemoteOwner {
                    condition.saturating_sub(100)
                } else {
                    condition
                },
                quantity_high: if grant.role == ProductionObservationRole::RemoteOwner {
                    condition.saturating_add(100).min(1_000)
                } else {
                    condition
                },
                source_digest: canonical_hash(
                    "canwu.production.facility-observation.v1",
                    facility,
                )?,
            });
        }
        for order in self
            .work_orders
            .values()
            .filter(|order| &order.site == scope)
        {
            let quantity = if grant.role == ProductionObservationRole::Operator
                || order.lifecycle == WorkOrderLifecycle::Settled
            {
                order.quantity
            } else {
                0
            };
            facts.push(ProductionObservationFact {
                subject: order.id.to_string(),
                state: format!("{:?}", order.lifecycle).to_lowercase(),
                quantity_low: quantity,
                quantity_high: quantity,
                source_digest: canonical_hash("canwu.production.order-observation.v1", order)?,
            });
        }
        facts.sort_by(|left, right| left.subject.cmp(&right.subject));
        facts.truncate(MAX_REPORT_FACTS);
        let blockers = if grant.role == ProductionObservationRole::Operator {
            self.work_orders
                .values()
                .filter(|order| &order.site == scope)
                .filter_map(|order| self.processes.get(&order.process))
                .flat_map(|process| self.blockers_for(process, &[]))
                .take(MAX_REPORT_FACTS.saturating_sub(facts.len()))
                .collect()
        } else {
            Vec::new()
        };
        Ok((facts, blockers))
    }

    fn operation_site(&self, operation: &ProductionOperation) -> Option<ProductionSiteId> {
        match operation {
            ProductionOperation::RequestCompletionLease { .. }
            | ProductionOperation::AbortCompletionLease { .. } => None,
            ProductionOperation::CreateWorkOrder { work_order } => Some(work_order.site.clone()),
            ProductionOperation::AuthorizeWorkOrder { work_order }
            | ProductionOperation::CancelWorkOrder { work_order }
            | ProductionOperation::ResolveDegradedFacility { work_order, .. } => self
                .work_orders
                .get(work_order)
                .map(|order| order.site.clone()),
            ProductionOperation::StartExecution { execution, .. } => Some(execution.site.clone()),
            ProductionOperation::AdvanceExecution { execution, .. }
            | ProductionOperation::CompleteExecution { execution } => self
                .executions
                .get(execution)
                .map(|execution| execution.site.clone()),
            ProductionOperation::CreateFacilityProject { project } => Some(project.site.clone()),
            ProductionOperation::AuthorizeFacilityProject { project }
            | ProductionOperation::AdvanceFacilityProject { project, .. }
            | ProductionOperation::AcceptFacilityCommissioning { project } => self
                .facility_projects
                .get(project)
                .map(|project| project.site.clone()),
            ProductionOperation::RetireFacility { facility, .. } => self
                .facilities
                .get(facility)
                .map(|facility| facility.site.clone()),
        }
    }

    fn operation_facility(&self, operation: &ProductionOperation) -> Option<FacilityAssetId> {
        match operation {
            ProductionOperation::StartExecution { execution, .. } => {
                Some(execution.facility.clone())
            }
            ProductionOperation::ResolveDegradedFacility { facility, .. }
            | ProductionOperation::RetireFacility { facility, .. } => Some(facility.clone()),
            ProductionOperation::CreateFacilityProject { project } => {
                Some(project.facility.clone())
            }
            ProductionOperation::AuthorizeFacilityProject { project }
            | ProductionOperation::AdvanceFacilityProject { project, .. }
            | ProductionOperation::AcceptFacilityCommissioning { project } => self
                .facility_projects
                .get(project)
                .map(|project| project.facility.clone()),
            ProductionOperation::RequestCompletionLease { .. }
            | ProductionOperation::AbortCompletionLease { .. }
            | ProductionOperation::CreateWorkOrder { .. }
            | ProductionOperation::AuthorizeWorkOrder { .. }
            | ProductionOperation::AdvanceExecution { .. }
            | ProductionOperation::CompleteExecution { .. }
            | ProductionOperation::CancelWorkOrder { .. } => None,
        }
    }
}

fn validate_capacity_cover(
    process: &ProcessRevision,
    order: &WorkOrder,
    execution: &ProductionExecution,
    allocations: &[&ProductionCapacityAllocation],
) -> Result<(), CanwuError> {
    if process.capacity.is_empty() || allocations.is_empty() {
        return Err(invalid(
            "production execution requires a non-empty exact capacity cover",
        ));
    }
    let mut required = BTreeMap::<String, u64>::new();
    for capacity in &process.capacity {
        let quantity = capacity
            .quantity
            .checked_mul(order.quantity)
            .ok_or_else(|| invalid("production capacity requirement overflowed"))?;
        let entry = required.entry(capacity.capability.clone()).or_default();
        *entry = entry
            .checked_add(quantity)
            .ok_or_else(|| invalid("production capacity requirement overflowed"))?;
    }
    let mut actual = BTreeMap::<String, u64>::new();
    for allocation in allocations {
        if allocation.execution != execution.id
            || allocation.work_order != order.id
            || allocation.facility != execution.facility
            || allocation.start > execution.started_at
            || allocation.end <= execution.started_at
            || !required.contains_key(&allocation.capability)
        {
            return Err(invalid(
                "production capacity allocation is outside the exact execution cover",
            ));
        }
        let entry = actual.entry(allocation.capability.clone()).or_default();
        *entry = entry
            .checked_add(allocation.quantity)
            .ok_or_else(|| invalid("production capacity allocation overflowed"))?;
    }
    if actual != required {
        return Err(invalid(
            "production capacity allocations do not exactly cover every process requirement",
        ));
    }
    Ok(())
}

fn validate_requirements(
    process: &ProcessRevision,
    evidence: &[ProductionEvidenceBinding],
) -> Result<(), CanwuError> {
    if evidence.len() > MAX_EVIDENCE_BINDINGS
        || evidence
            .iter()
            .any(|value| value.semantic_digest.is_empty())
    {
        return Err(invalid(
            "production evidence exceeds its bound or lacks a digest",
        ));
    }
    let empty_state = ProductionState::default();
    let blockers = empty_state.blockers_for(process, evidence);
    if blockers.is_empty() {
        Ok(())
    } else {
        Err(blocked(&blockers))
    }
}

fn validate_resource_inputs(
    process: &ProcessRevision,
    inputs: &[ResourceInputBinding],
    quantity: u64,
) -> Result<(), CanwuError> {
    if inputs.len() != process.inputs.len() {
        return Err(invalid(
            "production inputs do not cover every exact process material requirement",
        ));
    }
    let mut legs = BTreeSet::new();
    for (input, required) in inputs.iter().zip(&process.inputs) {
        let required_quantity = required
            .quantity
            .checked_mul(quantity)
            .ok_or_else(|| invalid("production input requirement overflowed"))?;
        if input.quantity == 0
            || input.quantity != required_quantity
            || input.allocation_leg.resource_revision != required.resource
            || input.allocation_leg.unit_revision != required.unit
            || input.allocation_leg.quantity < input.quantity
            || input.consumption.allocation_leg != input.allocation_leg.id
            || input.consumption.account != input.allocation_leg.account
            || input.consumption.quantity != input.quantity
            || input.consumption.semantic_digest.is_empty()
            || !matches!(
                input.consumption_outcome.status,
                ResourceOperationStatus::Applied | ResourceOperationStatus::Duplicate
            )
            || input.consumption_outcome.quantity != input.quantity
            || input.consumption_outcome.remainder != 0
            || input.allocation_leg.semantic_digest.is_empty()
            || input.consumption_outcome.semantic_digest.is_empty()
            || !legs.insert(input.allocation_leg.id.clone())
        {
            return Err(invalid(
                "production input does not exactly match its resource allocation and consumption outcome",
            ));
        }
    }
    Ok(())
}

pub(crate) fn facility_project_result_digest(
    project: &FacilityProject,
) -> Result<String, CanwuError> {
    let mut detached = project.clone();
    // Commissioning acceptance changes only the workflow lifecycle.  Bind the
    // evidence to the completed work and resulting asset so the same digest is
    // valid both before and after that authority transition.
    detached.lifecycle = FacilityProjectLifecycle::Commissioning;
    detached.result_evidence_digest = None;
    canonical_hash("canwu.production.facility-project-result.v1", &detached)
}

pub(crate) fn validate_project_completion_certificate(
    project: &FacilityProject,
) -> Result<(), CanwuError> {
    let certificate = &project.completion_certificate;
    let mut detached = certificate.clone();
    let recorded = std::mem::take(&mut detached.semantic_digest);
    if recorded.is_empty()
        || recorded
            != canonical_hash(
                "canwu.resource.completion-activation-certificate.v1",
                &detached,
            )?
        || certificate.operation_key != project.operation_key
        || certificate.eligibility_time != project.created_at
        || !certificate
            .prepared_grants
            .iter()
            .any(|(id, _)| id == &project.production_completion_grant)
        || !certificate
            .prepared_grants
            .iter()
            .any(|(id, _)| id == &project.resource_completion_grant)
    {
        return Err(invalid(
            "facility project lacks the exact activated production completion certificate",
        ));
    }
    Ok(())
}

fn required_project_operation_outcome_reservation(
    project: &FacilityProject,
) -> Result<u64, CanwuError> {
    let remaining_units = project
        .total_units
        .checked_sub(project.completed_units)
        .ok_or_else(|| invalid("facility project progress exceeds its total"))?;
    match project.lifecycle {
        FacilityProjectLifecycle::Planned => remaining_units
            .checked_add(2)
            .ok_or_else(|| invalid("facility project outcome reservation overflowed")),
        FacilityProjectLifecycle::Authorized
        | FacilityProjectLifecycle::Reserving
        | FacilityProjectLifecycle::InProgress => remaining_units
            .checked_add(1)
            .ok_or_else(|| invalid("facility project outcome reservation overflowed")),
        FacilityProjectLifecycle::Commissioning => Ok(1),
        FacilityProjectLifecycle::CompletionPending
        | FacilityProjectLifecycle::Completed
        | FacilityProjectLifecycle::Cancelled
        | FacilityProjectLifecycle::Failed => Ok(0),
    }
}

pub(crate) fn resource_input_bindings_digest(
    inputs: &[ResourceInputBinding],
) -> Result<String, CanwuError> {
    canonical_hash("canwu.production.resource-input-bindings.v1", inputs)
}

fn validate_technology_binding(
    process: &ProcessRevision,
    evidence: &TechnologyEvidenceBinding,
) -> Result<(), CanwuError> {
    if evidence.semantic_digest.is_empty()
        || !evidence
            .technique_revision
            .record
            .kind
            .matches_type::<canwu_technology::TechniqueRevision>()
        || evidence
            .capability_qualification
            .as_ref()
            .is_some_and(|value| {
                !value
                    .record
                    .kind
                    .matches_type::<canwu_technology::CapabilityQualification>()
            })
        || evidence.implementation.as_ref().is_some_and(|value| {
            !value
                .record
                .kind
                .matches_type::<canwu_technology::ImplementationRecord>()
        })
        || evidence.adoption.as_ref().is_some_and(|value| {
            !value
                .record
                .kind
                .matches_type::<canwu_technology::AdoptionRecord>()
        })
        || evidence.capability_qualification.is_none() && evidence.implementation.is_none()
        || process.adoption_required && evidence.adoption.is_none()
    {
        return Err(invalid(
            "technology evidence does not bind the exact required record classes",
        ));
    }
    Ok(())
}

pub(crate) fn validate_completion_certificate(
    execution: &ProductionExecution,
    eligibility_time: SimTime,
) -> Result<(), CanwuError> {
    let certificate = &execution.completion_certificate;
    let mut detached = certificate.clone();
    let recorded = std::mem::take(&mut detached.semantic_digest);
    if recorded.is_empty()
        || recorded
            != canonical_hash(
                "canwu.resource.completion-activation-certificate.v1",
                &detached,
            )?
        || execution.output_requests.is_empty()
        || &certificate.operation_key
            != execution
                .output_requests
                .first()
                .map(|request| &request.operation_key)
                .expect("non-empty output requests were checked")
        || certificate.eligibility_time != eligibility_time
        || execution.output_requests.iter().any(|request| {
            !certificate.locked_target_versions.contains(
                &canwu_resource::CompletionLockedTargetV1::Account {
                    id: request.account.clone(),
                    revision: request.expected_account_revision,
                },
            )
        })
        || !certificate
            .prepared_grants
            .iter()
            .any(|(id, _)| id == &execution.production_completion_grant)
        || !certificate
            .prepared_grants
            .iter()
            .any(|(id, _)| id == &execution.resource_completion_grant)
    {
        return Err(invalid(
            "execution lacks the exact activated production/resource completion certificate",
        ));
    }
    Ok(())
}

fn validate_process_time(process: &ProcessRevision, now: SimTime) -> Result<(), CanwuError> {
    if now < process.effective_from || process.effective_until.is_some_and(|until| now >= until) {
        Err(invalid(
            "process revision is not effective at the requested time",
        ))
    } else {
        Ok(())
    }
}

fn ensure_holder(order: &WorkOrder, holder: &KnowledgeHolderRef) -> Result<(), CanwuError> {
    if &order.holder == holder {
        Ok(())
    } else {
        Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "production holder is not authorized for this work order",
        ))
    }
}

fn next_action(kind: ProductionRequirementKind) -> &'static str {
    match kind {
        ProductionRequirementKind::Material => {
            "secure an exact resource allocation and consumption outcome"
        }
        ProductionRequirementKind::LaborCapability => "assign qualified labor",
        ProductionRequirementKind::Facility | ProductionRequirementKind::ToolsMachines => {
            "reserve usable facility capacity"
        }
        ProductionRequirementKind::Energy => "secure the required energy and quality",
        ProductionRequirementKind::TechnologyImplementation => {
            "install or qualify the exact technique revision"
        }
        ProductionRequirementKind::Authorization => "obtain production authority",
        ProductionRequirementKind::EnvironmentSeason => "wait for an eligible process interval",
        ProductionRequirementKind::Security => "restore acceptable site security",
        ProductionRequirementKind::Access => "restore site or route access",
        ProductionRequirementKind::Maintenance => "perform required maintenance",
        ProductionRequirementKind::FinanceOrganization => {
            "provide the required finance or organization evidence"
        }
    }
}

fn blocked(blockers: &[ProductionBlocker]) -> CanwuError {
    let groups = blockers
        .iter()
        .map(|value| value.requirement.id.as_str())
        .collect::<Vec<_>>()
        .join(",");
    CanwuError::new(
        ErrorCode::InvalidDomainRecord,
        format!("production requirements are blocked: {groups}"),
    )
}

pub(crate) fn production_observation_head_storage_key(
    key: &ProductionObservationHeadKeyV1,
) -> Result<String, CanwuError> {
    canonical_hash("canwu.production.observation-head-key.v1", key)
}

pub(crate) fn validate_identifier(value: &str, label: &str) -> Result<(), CanwuError> {
    if value.is_empty()
        || value.len() > MAX_PRODUCTION_IDENTIFIER_BYTES
        || !value.contains(':')
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidPayload,
            format!("{label} must be a validated namespaced identifier"),
        ));
    }
    Ok(())
}

fn canonical_text(value: &str, label: &str) -> Result<(), CanwuError> {
    if value.trim().is_empty() || value.len() > 256 {
        Err(invalid(format!("{label} is empty or over its byte bound")))
    } else {
        Ok(())
    }
}

pub(crate) fn invalid(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidDomainRecord, message)
}
