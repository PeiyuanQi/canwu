use crate::{
    CompletionLeaseAcquisitionId, CompletionLeaseBookV1, PLUGIN_NAMESPACE,
    ResourceArchiveHeadStateV1, ResourceArchiveMaintenanceReceiptV1,
    ResourceArchiveRetentionHandleV1, ResourceTerminalRecordKeyV1, RunBudgetRevisionV1,
};
use canwu_api::{
    CapacityBookingId, DomainRecord, DomainRecordClass, DomainRecordDraft, DomainRecordLifecycle,
    DomainRecordType, DomainRecordVersionRef, DomainValueKindClass, EntityRef, EvidenceRef,
    HandoffId, ItineraryRevisionId, KnowledgeHolderRef, LegExecutionId,
    PayloadRequiredEvidenceContinuationV1, SimTime, TransportExecutionId,
};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Display, Formatter};

/// Canonical resource plugin limits. A scenario may choose lower values.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceLimitsV1 {
    pub max_definitions: usize,
    pub max_unit_revisions: usize,
    pub max_accounts: usize,
    pub max_demands: usize,
    pub max_transfers: usize,
    pub max_allocation_candidates: usize,
    pub max_mutations_per_boundary: usize,
    pub max_operation_outcomes: usize,
    pub max_reports_per_boundary: usize,
    pub max_observation_heads: usize,
    pub max_archive_candidates: usize,
    pub max_archive_maintenance_receipts: usize,
    pub max_dirty_demands: usize,
    pub max_due_entries: usize,
    pub max_state_bytes: usize,
}

impl ResourceLimitsV1 {
    pub const DEFAULT_ACTIVE_PER_SHARD: usize = 1_024;
    pub const HARD_ACTIVE_PER_SHARD: usize = 4_096;
    pub const MAX_SHARDS: usize = 64;
    pub const DEFAULT_SHARDS: usize = 8;
    pub const MAX_HOLDERS: usize = 1_024;
    pub const MAX_QUERY_PAGE: usize = 256;
    pub const MAX_HOT_RECEIPTS: usize = 8_192;
    pub const MAX_HOT_RECORD_BYTES: usize = 65_536;
    pub const MAX_AUTHORITATIVE_STATE_BYTES: usize = 256 * 1024 * 1024;

    #[must_use]
    pub const fn canonical() -> Self {
        Self {
            max_definitions: 1_024,
            max_unit_revisions: 1_024,
            max_accounts: Self::DEFAULT_ACTIVE_PER_SHARD * Self::DEFAULT_SHARDS,
            max_demands: Self::DEFAULT_ACTIVE_PER_SHARD * Self::DEFAULT_SHARDS,
            max_transfers: Self::DEFAULT_ACTIVE_PER_SHARD * Self::DEFAULT_SHARDS,
            max_allocation_candidates: 2_048,
            max_mutations_per_boundary: 4_096,
            max_operation_outcomes: Self::MAX_HOT_RECEIPTS,
            max_reports_per_boundary: 256,
            max_observation_heads: Self::MAX_HOLDERS * 4,
            max_archive_candidates: 256,
            max_archive_maintenance_receipts: 1_024,
            max_dirty_demands: 2_048,
            max_due_entries: Self::HARD_ACTIVE_PER_SHARD * Self::MAX_SHARDS,
            max_state_bytes: Self::MAX_AUTHORITATIVE_STATE_BYTES,
        }
    }

    pub fn validate(self) -> Result<(), ResourceError> {
        if self.max_definitions == 0
            || self.max_definitions > 1_024
            || self.max_unit_revisions == 0
            || self.max_unit_revisions > 1_024
            || self.max_accounts == 0
            || self.max_accounts > Self::HARD_ACTIVE_PER_SHARD * Self::MAX_SHARDS
            || self.max_demands == 0
            || self.max_demands > Self::HARD_ACTIVE_PER_SHARD * Self::MAX_SHARDS
            || self.max_transfers == 0
            || self.max_transfers > Self::HARD_ACTIVE_PER_SHARD * Self::MAX_SHARDS
            || self.max_allocation_candidates == 0
            || self.max_allocation_candidates > 2_048
            || self.max_mutations_per_boundary == 0
            || self.max_mutations_per_boundary > 4_096
            || self.max_operation_outcomes == 0
            || self.max_operation_outcomes > Self::MAX_HOT_RECEIPTS
            || self.max_reports_per_boundary == 0
            || self.max_reports_per_boundary > 256
            || self.max_observation_heads == 0
            || self.max_observation_heads > Self::MAX_HOLDERS * 4
            || self.max_archive_candidates == 0
            || self.max_archive_candidates > 256
            || self.max_archive_maintenance_receipts == 0
            || self.max_archive_maintenance_receipts > 1_024
            || self.max_dirty_demands == 0
            || self.max_dirty_demands > 2_048
            || self.max_due_entries == 0
            || self.max_due_entries > Self::HARD_ACTIVE_PER_SHARD * Self::MAX_SHARDS
            || self.max_state_bytes == 0
            || self.max_state_bytes > Self::MAX_AUTHORITATIVE_STATE_BYTES
        {
            return Err(ResourceError::LimitExceeded(
                "resource limits exceed the V1 hard maxima".to_owned(),
            ));
        }
        Ok(())
    }
}

impl Default for ResourceLimitsV1 {
    fn default() -> Self {
        Self::canonical()
    }
}

fn validate_namespaced(value: &str, label: &str) -> Result<(), ResourceError> {
    let valid_len = !value.is_empty() && value.len() <= 160;
    let valid_chars = value.bytes().all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':' | b'/')
    });
    let namespaced = value.contains(':');
    if !valid_len || !valid_chars || !namespaced {
        return Err(ResourceError::InvalidIdentifier(format!(
            "{label} must be a 1-160 byte namespaced identifier"
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
            pub fn new(value: impl Into<String>) -> Result<Self, ResourceError> {
                let value = value.into();
                validate_namespaced(&value, $label)?;
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
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

typed_id!(ResourceDefinitionId, "resource definition");
typed_id!(ResourceDefinitionRevisionId, "resource definition revision");
typed_id!(ResourceUnitRevisionId, "resource unit revision");
typed_id!(
    ProtectedFloorPolicyRevisionId,
    "protected-floor policy revision"
);
typed_id!(ResourceAccountId, "resource account");
typed_id!(ResourceDemandId, "resource demand");
typed_id!(ResourceReservationId, "resource reservation");
typed_id!(ResourceAllocationLegId, "resource allocation leg");
typed_id!(ResourceTransferId, "resource transfer");
typed_id!(ResourceConsumptionId, "resource consumption");
typed_id!(ResourceConsumptionIntentId, "resource consumption intent");
typed_id!(ResourceLossId, "resource loss");
typed_id!(ResourceFulfillmentId, "resource fulfillment");
typed_id!(ResourceOperationOutcomeId, "resource operation outcome");
typed_id!(ResourceOperationKey, "resource operation key");
typed_id!(ResourceQualityId, "resource quality");
typed_id!(ResourceScopeId, "resource scope");
typed_id!(ResourceTieBreakKey, "resource tie-break key");
typed_id!(ResourceAlternativeGroupId, "resource alternative group");
typed_id!(ResourceReportGrantId, "resource report grant");
typed_id!(ResourceReportId, "resource report");
typed_id!(ResourceObservationHeadId, "resource observation head");
typed_id!(
    ResourceObservationAdapterRevisionId,
    "resource observation adapter revision"
);

/// Non-zero revision of a resource-owned dynamic record.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ResourceRevision(u64);

impl ResourceRevision {
    pub const INITIAL: Self = Self(1);

    pub fn new(value: u64) -> Result<Self, ResourceError> {
        if value == 0 {
            return Err(ResourceError::InvalidRevision);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    pub fn next(self) -> Result<Self, ResourceError> {
        Self::new(self.0.checked_add(1).ok_or(ResourceError::Overflow)?)
    }
}

impl<'de> Deserialize<'de> for ResourceRevision {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(u64::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// Immutable resource definition revision.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceDefinitionRevision {
    pub id: ResourceDefinitionRevisionId,
    pub resource: ResourceDefinitionId,
    pub revision: ResourceRevision,
    pub canonical_unit: ResourceUnitRevisionId,
    pub quality: ResourceQualityId,
    pub scope: ResourceScopeId,
    pub effective_from: SimTime,
    pub effective_until: Option<SimTime>,
    pub process_suitability: BTreeSet<String>,
    pub semantic_digest: String,
}

/// Immutable unit definition. Conversions require a separate explicit
/// revision; the resource ledger never converts implicitly.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceUnitRevision {
    pub id: ResourceUnitRevisionId,
    pub revision: ResourceRevision,
    pub symbol: String,
    pub scale_numerator: u64,
    pub scale_denominator: u64,
    pub semantic_digest: String,
}

/// Immutable policy that protects a floor in one account.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProtectedFloorPolicyRevision {
    pub id: ProtectedFloorPolicyRevisionId,
    pub revision: ResourceRevision,
    pub floor: u64,
    pub override_classes: BTreeSet<String>,
    pub semantic_digest: String,
}

/// One authoritative balance. Available, reserved, and protected quantities
/// are derived through [`ResourceState::account_quantities`].
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceAccount {
    pub id: ResourceAccountId,
    pub revision: ResourceRevision,
    pub custodian: KnowledgeHolderRef,
    pub resource_revision: ResourceDefinitionRevisionId,
    pub unit_revision: ResourceUnitRevisionId,
    pub balance: u64,
    pub capacity: Option<u64>,
    pub protected_floor_policy: Option<ProtectedFloorPolicyRevisionId>,
    pub closed: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PartialFulfillmentPolicy {
    RejectPartial,
    AcceptPartial,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DemandStatus {
    Open,
    PartiallyFulfilled,
    Fulfilled,
    Cancelled,
    Expired,
    RejectedMinimum,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceDemand {
    pub id: ResourceDemandId,
    pub revision: ResourceRevision,
    pub requester: KnowledgeHolderRef,
    pub resource_revision: ResourceDefinitionRevisionId,
    pub unit_revision: ResourceUnitRevisionId,
    pub requested: u64,
    pub fulfilled: u64,
    pub minimum_useful: u64,
    pub partial_fulfillment: PartialFulfillmentPolicy,
    pub alternative_group: Option<ResourceAlternativeGroupId>,
    pub due_at: SimTime,
    pub expires_at: SimTime,
    pub priority: i32,
    pub tie_break: ResourceTieBreakKey,
    pub admitted_sequence: u64,
    pub protected_floor_policy: Option<ProtectedFloorPolicyRevisionId>,
    pub protection_override_class: Option<String>,
    pub status: DemandStatus,
    pub rejection_reason: Option<String>,
}

impl ResourceDemand {
    #[must_use]
    pub fn remainder(&self) -> u64 {
        self.requested.saturating_sub(self.fulfilled)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReservationStatus {
    Active,
    Consumed,
    Released,
    Expired,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceReservation {
    pub id: ResourceReservationId,
    pub revision: ResourceRevision,
    pub demand: ResourceDemandId,
    pub account: ResourceAccountId,
    pub allocation_leg: ResourceAllocationLegId,
    pub quantity: u64,
    pub status: ReservationStatus,
    pub operation_key: ResourceOperationKey,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AllocationLegStatus {
    Reserved,
    Consumed,
    Released,
    Expired,
}

/// Exact allocation result consumed by production, force supply, or another
/// independent resource consumer.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceAllocationLeg {
    pub id: ResourceAllocationLegId,
    pub revision: ResourceRevision,
    pub demand: ResourceDemandId,
    pub demand_revision: ResourceRevision,
    pub reservation: ResourceReservationId,
    pub account: ResourceAccountId,
    pub account_revision: ResourceRevision,
    pub resource_revision: ResourceDefinitionRevisionId,
    pub unit_revision: ResourceUnitRevisionId,
    pub quantity: u64,
    pub status: AllocationLegStatus,
    pub priority: i32,
    pub due_at: SimTime,
    pub tie_break: ResourceTieBreakKey,
    pub admitted_sequence: u64,
    pub operation_key: ResourceOperationKey,
    pub semantic_digest: String,
}

/// Compact exact reference suitable for cross-extension packets.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceAllocationLegVersionV1 {
    pub id: ResourceAllocationLegId,
    pub revision: ResourceRevision,
    pub account: ResourceAccountId,
    pub account_revision: ResourceRevision,
    pub resource_revision: ResourceDefinitionRevisionId,
    pub unit_revision: ResourceUnitRevisionId,
    pub quantity: u64,
    pub semantic_digest: String,
}

impl From<&ResourceAllocationLeg> for ResourceAllocationLegVersionV1 {
    fn from(value: &ResourceAllocationLeg) -> Self {
        Self {
            id: value.id.clone(),
            revision: value.revision,
            account: value.account.clone(),
            account_revision: value.account_revision,
            resource_revision: value.resource_revision.clone(),
            unit_revision: value.unit_revision.clone(),
            quantity: value.quantity,
            semantic_digest: value.semantic_digest.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceConsumptionIntentStatusV1 {
    Authorized,
    Retired,
}

/// Provider-owned authorization for one exact resource consumption. This
/// value lives inside the provider's exact domain payload; the resource
/// adapter accepts no inferred policy, string prefix, or equal-size demand.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceConsumptionIntentV1 {
    pub id: ResourceConsumptionIntentId,
    pub provider_plugin: String,
    pub demand: ResourceDemandId,
    pub demand_revision: ResourceRevision,
    pub allocation: ResourceAllocationLegVersionV1,
    pub account: ResourceAccountId,
    pub expected_account_revision: ResourceRevision,
    pub consumption_id: ResourceConsumptionId,
    pub operation_key: ResourceOperationKey,
    pub quantity: u64,
    pub status: ResourceConsumptionIntentStatusV1,
    pub semantic_digest: String,
}

impl ResourceConsumptionIntentV1 {
    pub fn seal(mut self) -> Result<Self, ResourceError> {
        self.semantic_digest.clear();
        self.semantic_digest =
            crate::canonical_digest("canwu.resource.consumption-intent.v1", &self)?;
        self.validate()?;
        Ok(self)
    }

    pub fn validate(&self) -> Result<(), ResourceError> {
        let mut detached = self.clone();
        let recorded = std::mem::take(&mut detached.semantic_digest);
        if self.provider_plugin.is_empty()
            || self.quantity == 0
            || self.quantity != self.allocation.quantity
            || self.account != self.allocation.account
            || recorded
                != crate::canonical_digest("canwu.resource.consumption-intent.v1", &detached)?
        {
            return Err(ResourceError::InvalidDefinition(
                "resource consumption intent is empty, inconsistent, or forged".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Transport execution is evidence and custody execution, not a second
/// material balance.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TransportExecutionLink {
    pub execution: TransportExecutionId,
    pub itinerary_revision: ItineraryRevisionId,
    pub leg_execution: Option<LegExecutionId>,
    pub handoff: Option<HandoffId>,
    pub capacity_booking: Option<CapacityBookingId>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceTransferState {
    PendingDispatch,
    InTransit,
    ArrivalPending,
    ReturnPending,
    Accepted,
    Lost,
    ExternalOutflowSettled,
    Cancelled,
    Returned,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceTransfer {
    pub id: ResourceTransferId,
    pub revision: ResourceRevision,
    pub source: ResourceAccountId,
    pub destination: Option<ResourceAccountId>,
    pub allocation_leg: ResourceAllocationLegId,
    pub resource_revision: ResourceDefinitionRevisionId,
    pub unit_revision: ResourceUnitRevisionId,
    pub quantity: u64,
    pub escrow: u64,
    pub accepted: u64,
    pub lost: u64,
    pub returned: u64,
    pub external_outflow: u64,
    pub state: ResourceTransferState,
    pub transport: Option<TransportExecutionLink>,
    pub exact_evidence: Vec<DomainRecordVersionRef>,
    pub completion_acquisition: CompletionLeaseAcquisitionId,
    pub operation_key: ResourceOperationKey,
    pub terminal_sequence: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsumptionStatus {
    Settled,
    Reversed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceConsumption {
    pub id: ResourceConsumptionId,
    pub revision: ResourceRevision,
    pub account: ResourceAccountId,
    pub allocation_leg: ResourceAllocationLegId,
    pub demand: ResourceDemandId,
    pub resource_revision: ResourceDefinitionRevisionId,
    pub unit_revision: ResourceUnitRevisionId,
    pub quantity: u64,
    pub consumer_evidence: DomainRecordVersionRef,
    pub completion_acquisition: CompletionLeaseAcquisitionId,
    pub status: ConsumptionStatus,
    pub operation_key: ResourceOperationKey,
    pub semantic_digest: String,
    pub terminal_sequence: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceConsumptionVersionV1 {
    pub id: ResourceConsumptionId,
    pub revision: ResourceRevision,
    pub account: ResourceAccountId,
    pub allocation_leg: ResourceAllocationLegId,
    pub quantity: u64,
    pub consumer_evidence: DomainRecordVersionRef,
    pub semantic_digest: String,
}

impl From<&ResourceConsumption> for ResourceConsumptionVersionV1 {
    fn from(value: &ResourceConsumption) -> Self {
        Self {
            id: value.id.clone(),
            revision: value.revision,
            account: value.account.clone(),
            allocation_leg: value.allocation_leg.clone(),
            quantity: value.quantity,
            consumer_evidence: value.consumer_evidence.clone(),
            semantic_digest: value.semantic_digest.clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceLoss {
    pub id: ResourceLossId,
    pub revision: ResourceRevision,
    pub account: Option<ResourceAccountId>,
    pub transfer: Option<ResourceTransferId>,
    pub resource_revision: ResourceDefinitionRevisionId,
    pub unit_revision: ResourceUnitRevisionId,
    pub quantity: u64,
    pub cause: EvidenceRef,
    pub operation_key: ResourceOperationKey,
    pub terminal_sequence: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FulfillmentStatus {
    Partial,
    Complete,
    RejectedMinimum,
    Cancelled,
    Expired,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceFulfillment {
    pub id: ResourceFulfillmentId,
    pub revision: ResourceRevision,
    pub demand: ResourceDemandId,
    pub allocation_legs: Vec<ResourceAllocationLegId>,
    pub consumed_quantity: u64,
    pub remainder: u64,
    pub status: FulfillmentStatus,
    pub rejection_reason: Option<String>,
    pub operation_key: ResourceOperationKey,
    pub semantic_digest: String,
    pub terminal_sequence: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceFulfillmentVersionV1 {
    pub id: ResourceFulfillmentId,
    pub revision: ResourceRevision,
    pub demand: ResourceDemandId,
    pub consumed_quantity: u64,
    pub remainder: u64,
    pub status: FulfillmentStatus,
    pub operation_key: ResourceOperationKey,
    pub semantic_digest: String,
}

impl From<&ResourceFulfillment> for ResourceFulfillmentVersionV1 {
    fn from(value: &ResourceFulfillment) -> Self {
        Self {
            id: value.id.clone(),
            revision: value.revision,
            demand: value.demand.clone(),
            consumed_quantity: value.consumed_quantity,
            remainder: value.remainder,
            status: value.status,
            operation_key: value.operation_key.clone(),
            semantic_digest: value.semantic_digest.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceOperationStatus {
    Applied,
    Rejected,
    Duplicate,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceOperationKind {
    CreateAccount,
    SubmitDemand,
    AmendDemand,
    Allocate,
    Consume,
    BeginTransfer,
    AdvanceTransfer,
    CancelTransfer,
    CompleteTransfer,
    Credit,
    ExternalOutflow,
    SetProtectedFloor,
    CancelDemand,
    Observation,
    CompletionLease,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceOperationOutcome {
    pub id: ResourceOperationOutcomeId,
    pub revision: ResourceRevision,
    pub operation_key: ResourceOperationKey,
    pub request_digest: String,
    pub kind: ResourceOperationKind,
    pub status: ResourceOperationStatus,
    pub quantity: u64,
    pub remainder: u64,
    pub result_ref: Option<ResourceRecordRefV1>,
    pub rejection_code: Option<String>,
    pub rejection_reason: Option<String>,
    pub exact_evidence: Vec<DomainRecordVersionRef>,
    pub semantic_digest: String,
    pub sequence: u64,
}

impl ResourceOperationOutcome {
    /// Validate the self-contained identity and terminal semantics of an
    /// operation outcome. The request body is intentionally not retained in
    /// the compact outcome, so the request digest and deterministic outcome
    /// id are the binding that survives archive compaction.
    pub fn validate(&self) -> Result<(), ResourceError> {
        let expected_id =
            canonical_operation_outcome_id(&self.operation_key, &self.request_digest)?;
        let mut detached = self.clone();
        let semantic_digest = std::mem::take(&mut detached.semantic_digest);
        if self.id != expected_id
            || self.revision != ResourceRevision::INITIAL
            || self.sequence == 0
            || semantic_digest.len() != 64
            || semantic_digest
                != crate::canonical_digest("canwu.resource.operation-outcome.v1", &detached)?
            || !is_canonical_hex(&self.request_digest)
        {
            return Err(ResourceError::InvalidDefinition(
                "resource operation outcome identity or digest is invalid".to_owned(),
            ));
        }

        match self.status {
            ResourceOperationStatus::Applied => {
                if self.rejection_code.is_some() || self.rejection_reason.is_some() {
                    return Err(ResourceError::InvalidDefinition(
                        "applied resource outcome carries rejection details".to_owned(),
                    ));
                }
            }
            ResourceOperationStatus::Rejected => {
                if self.quantity != 0
                    || self.result_ref.is_some()
                    || !self.exact_evidence.is_empty()
                    || self.rejection_code.as_deref().is_none_or(str::is_empty)
                    || self.rejection_reason.as_deref().is_none_or(str::is_empty)
                {
                    return Err(ResourceError::InvalidDefinition(
                        "rejected resource outcome is not a deterministic zero-result closure"
                            .to_owned(),
                    ));
                }
            }
            ResourceOperationStatus::Duplicate => {
                return Err(ResourceError::InvalidDefinition(
                    "duplicate resource outcomes are not persisted terminal records".to_owned(),
                ));
            }
        }

        if self.status == ResourceOperationStatus::Applied
            && !result_ref_matches_kind(
                self.kind,
                self.operation_key.as_str(),
                self.result_ref.as_ref(),
            )
        {
            return Err(ResourceError::InvalidDefinition(
                "resource operation outcome result reference does not match its operation kind"
                    .to_owned(),
            ));
        }

        let evidence_count = self.exact_evidence.len();
        let evidence_shape_ok = self.status != ResourceOperationStatus::Applied
            || match self.kind {
                ResourceOperationKind::Consume | ResourceOperationKind::ExternalOutflow => {
                    evidence_count == 1
                }
                ResourceOperationKind::AdvanceTransfer => evidence_count == 1,
                ResourceOperationKind::Credit => evidence_count <= 1,
                ResourceOperationKind::CompletionLease => {
                    completion_evidence_shape(self.operation_key.as_str(), evidence_count)
                }
                _ => true,
            };
        if !evidence_shape_ok {
            return Err(ResourceError::InvalidDefinition(
                "resource operation outcome evidence is not canonical for its operation kind"
                    .to_owned(),
            ));
        }
        Ok(())
    }
}

pub(crate) fn canonical_operation_outcome_id(
    operation_key: &ResourceOperationKey,
    request_digest: &str,
) -> Result<ResourceOperationOutcomeId, ResourceError> {
    if !is_canonical_hex(request_digest) {
        return Err(ResourceError::InvalidDefinition(
            "resource operation request digest is not canonical hexadecimal".to_owned(),
        ));
    }
    ResourceOperationOutcomeId::new(format!(
        "resource:outcome:{}:{}",
        operation_key.as_str().replace(':', "-"),
        &request_digest[..16]
    ))
}

fn is_canonical_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn result_ref_matches_kind(
    kind: ResourceOperationKind,
    operation_key: &str,
    result_ref: Option<&ResourceRecordRefV1>,
) -> bool {
    match kind {
        ResourceOperationKind::Allocate => {
            result_ref.is_none_or(|value| matches!(value, ResourceRecordRefV1::AllocationLeg(_)))
        }
        ResourceOperationKind::Consume => {
            matches!(result_ref, Some(ResourceRecordRefV1::Consumption(_)))
        }
        ResourceOperationKind::BeginTransfer
        | ResourceOperationKind::AdvanceTransfer
        | ResourceOperationKind::CancelTransfer => {
            matches!(result_ref, Some(ResourceRecordRefV1::Transfer(_)))
        }
        ResourceOperationKind::CompleteTransfer => matches!(
            result_ref,
            Some(ResourceRecordRefV1::Transfer(_) | ResourceRecordRefV1::Loss(_))
        ),
        ResourceOperationKind::CompletionLease => {
            let Some(suffix) = operation_key.strip_prefix("resource:completion:") else {
                return false;
            };
            let operation = suffix.split(':').next().unwrap_or_default();
            match operation {
                "grant" | "prepare" | "expire" => result_ref.is_none(),
                "participant-expire" => {
                    result_ref.is_none_or(|value| matches!(value, ResourceRecordRefV1::Lease(_)))
                }
                "acquire"
                | "activate"
                | "abort"
                | "release"
                | "participant-grant"
                | "participant-prepare"
                | "participant-consume"
                | "participant-complete"
                | "participant-release" => {
                    matches!(result_ref, Some(ResourceRecordRefV1::Lease(_)))
                }
                _ => false,
            }
        }
        ResourceOperationKind::CreateAccount
        | ResourceOperationKind::SubmitDemand
        | ResourceOperationKind::AmendDemand
        | ResourceOperationKind::SetProtectedFloor
        | ResourceOperationKind::CancelDemand
        | ResourceOperationKind::Observation => result_ref.is_none(),
        ResourceOperationKind::Credit | ResourceOperationKind::ExternalOutflow => {
            result_ref.is_none()
        }
    }
}

fn completion_evidence_shape(operation_key: &str, evidence_count: usize) -> bool {
    let Some(suffix) = operation_key.strip_prefix("resource:completion:") else {
        return false;
    };
    let operation = suffix.split(':').next().unwrap_or_default();
    match operation {
        "acquire" => true,
        "participant-grant"
        | "participant-prepare"
        | "participant-consume"
        | "participant-release" => evidence_count == 1,
        "activate"
        | "abort"
        | "grant"
        | "prepare"
        | "expire"
        | "release"
        | "participant-complete"
        | "participant-expire" => evidence_count == 0,
        _ => false,
    }
}

/// Exact acknowledgement packet used by independent consumers.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceOperationOutcomeVersionV1 {
    pub id: ResourceOperationOutcomeId,
    pub revision: ResourceRevision,
    pub operation_key: ResourceOperationKey,
    pub status: ResourceOperationStatus,
    pub quantity: u64,
    pub remainder: u64,
    pub result_ref: Option<ResourceRecordRefV1>,
    pub semantic_digest: String,
}

impl From<&ResourceOperationOutcome> for ResourceOperationOutcomeVersionV1 {
    fn from(value: &ResourceOperationOutcome) -> Self {
        Self {
            id: value.id.clone(),
            revision: value.revision,
            operation_key: value.operation_key.clone(),
            status: value.status,
            quantity: value.quantity,
            remainder: value.remainder,
            result_ref: value.result_ref.clone(),
            semantic_digest: value.semantic_digest.clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum ResourceRecordRefV1 {
    AllocationLeg(ResourceAllocationLegId),
    Consumption(ResourceConsumptionId),
    Transfer(ResourceTransferId),
    Fulfillment(ResourceFulfillmentId),
    Loss(ResourceLossId),
    Outcome(ResourceOperationOutcomeId),
    Lease(CompletionLeaseAcquisitionId),
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ConservationTotalsV1 {
    pub opening_balances: u128,
    pub opening_active_escrow: u128,
    pub admitted_production: u128,
    pub external_inflow: u128,
    pub admitted_consumption: u128,
    pub admitted_loss: u128,
    pub external_outflow: u128,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AccountQuantitiesV1 {
    pub authoritative_balance: u64,
    pub available: u64,
    pub reserved: u64,
    pub protected: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceReportGrantV1 {
    pub id: ResourceReportGrantId,
    pub holder: KnowledgeHolderRef,
    pub scope: ResourceScopeId,
    pub accounts: BTreeSet<ResourceAccountId>,
    pub demands: BTreeSet<ResourceDemandId>,
    pub include_transfer_details: bool,
    pub confidence_per_mille: u16,
    pub cadence_minutes: u64,
    pub delay_minutes: u64,
}

/// Canonical authoritative resource root stored as one versioned plugin-owned
/// domain record. Nested records retain their own exact revisions.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceState {
    pub format_version: u32,
    pub state_revision: ResourceRevision,
    #[serde(rename = "canwu_payload_required_evidence_continuation")]
    pub continuation: PayloadRequiredEvidenceContinuationV1,
    pub limits: ResourceLimitsV1,
    pub definitions: BTreeMap<ResourceDefinitionRevisionId, ResourceDefinitionRevision>,
    pub units: BTreeMap<ResourceUnitRevisionId, ResourceUnitRevision>,
    pub protected_floor_policies:
        BTreeMap<ProtectedFloorPolicyRevisionId, ProtectedFloorPolicyRevision>,
    pub accounts: BTreeMap<ResourceAccountId, ResourceAccount>,
    pub demands: BTreeMap<ResourceDemandId, ResourceDemand>,
    pub reservations: BTreeMap<ResourceReservationId, ResourceReservation>,
    pub allocation_legs: BTreeMap<ResourceAllocationLegId, ResourceAllocationLeg>,
    pub transfers: BTreeMap<ResourceTransferId, ResourceTransfer>,
    pub consumptions: BTreeMap<ResourceConsumptionId, ResourceConsumption>,
    pub losses: BTreeMap<ResourceLossId, ResourceLoss>,
    pub fulfillments: BTreeMap<ResourceFulfillmentId, ResourceFulfillment>,
    pub outcomes: BTreeMap<ResourceOperationKey, ResourceOperationOutcome>,
    pub report_grants: BTreeMap<ResourceReportGrantId, ResourceReportGrantV1>,
    pub observation_heads: BTreeMap<ResourceObservationHeadId, crate::ResourceObservationHeadV1>,
    pub observation_head_by_grant: BTreeMap<ResourceReportGrantId, ResourceObservationHeadId>,
    #[serde(default)]
    pub report_dirty_grants: BTreeSet<ResourceReportGrantId>,
    #[serde(default)]
    pub report_due_index: BTreeMap<i64, BTreeSet<ResourceReportGrantId>>,
    #[serde(default)]
    pub report_cursor: Option<ResourceReportGrantId>,
    pub run_budget: RunBudgetRevisionV1,
    pub completion_leases: CompletionLeaseBookV1,
    pub external_completion_participants: crate::ExternalCompletionParticipantBookV1,
    /// Named mandatory report reservations retained until the exact holder
    /// grant has published its terminal observation.
    #[serde(default)]
    pub completion_report_reservations:
        BTreeMap<CompletionLeaseAcquisitionId, BTreeSet<ResourceReportGrantId>>,
    #[serde(default)]
    pub completion_report_ready: BTreeMap<CompletionLeaseAcquisitionId, ResourceRevision>,
    pub demand_due_index: BTreeMap<SimTime, BTreeSet<ResourceDemandId>>,
    pub demand_expiry_index: BTreeMap<SimTime, BTreeSet<ResourceDemandId>>,
    pub reservation_by_demand: BTreeMap<ResourceDemandId, BTreeSet<ResourceReservationId>>,
    pub dirty_demands: BTreeSet<ResourceDemandId>,
    pub active_transfers: BTreeSet<ResourceTransferId>,
    pub terminal_archive_candidates: BTreeMap<u64, ResourceTerminalRecordKeyV1>,
    pub archive_head: ResourceArchiveHeadStateV1,
    pub archive_retention_handles: BTreeMap<String, ResourceArchiveRetentionHandleV1>,
    pub archive_maintenance_receipts: BTreeMap<u64, ResourceArchiveMaintenanceReceiptV1>,
    pub conservation: ConservationTotalsV1,
    pub next_admitted_sequence: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ResourceRuntimeRecord;

impl DomainRecordType for ResourceRuntimeRecord {
    type Payload = ResourceState;
    type Class = DomainValueKindClass;

    const NAMESPACE: &'static str = PLUGIN_NAMESPACE;
    const NAME: &'static str = "runtime";
}

#[must_use]
pub fn resource_runtime_reference() -> canwu_api::TypedDomainRecordRef<ResourceRuntimeRecord> {
    canwu_api::TypedDomainRecordRef::new("resource:runtime:v1")
}

impl ResourceState {
    pub fn into_record(mut self) -> Result<DomainRecord, canwu_api::CanwuError> {
        self.refresh_continuation();
        self.validate().map_err(resource_canwu_error)?;
        let draft = self.record_draft()?;
        Ok(DomainRecord {
            reference: draft.reference,
            owner: crate::PLUGIN_NAME.to_owned(),
            class: DomainRecordClass::Record,
            version: self.state_revision.get(),
            lifecycle: DomainRecordLifecycle::Active,
            payload: draft.payload,
            references: draft.references,
        })
    }

    pub(crate) fn record_draft(&self) -> Result<DomainRecordDraft, canwu_api::CanwuError> {
        let mut persisted = self.clone();
        persisted.refresh_continuation();
        serde_json::to_value(&persisted.report_dirty_grants).map_err(|error| {
            canwu_api::CanwuError::new(
                canwu_api::ErrorCode::InvalidDomainRecord,
                format!("resource report dirty index is not serializable: {error}"),
            )
        })?;
        serde_json::to_value(&persisted.report_due_index).map_err(|error| {
            canwu_api::CanwuError::new(
                canwu_api::ErrorCode::InvalidDomainRecord,
                format!("resource report due index is not serializable: {error}"),
            )
        })?;
        serde_json::to_value(&persisted.report_cursor).map_err(|error| {
            canwu_api::CanwuError::new(
                canwu_api::ErrorCode::InvalidDomainRecord,
                format!("resource report cursor is not serializable: {error}"),
            )
        })?;
        DomainRecordDraft::from_typed(resource_runtime_reference(), &persisted)
    }

    pub(crate) fn refresh_continuation(&mut self) {
        self.continuation = self.computed_continuation();
    }

    #[must_use]
    pub fn computed_continuation(&self) -> PayloadRequiredEvidenceContinuationV1 {
        let dependencies = self
            .transfers
            .values()
            .filter(|transfer| {
                !matches!(
                    transfer.state,
                    ResourceTransferState::Accepted
                        | ResourceTransferState::Lost
                        | ResourceTransferState::ExternalOutflowSettled
                        | ResourceTransferState::Cancelled
                        | ResourceTransferState::Returned
                )
            })
            .flat_map(|transfer| transfer.exact_evidence.iter().cloned())
            .chain(
                self.completion_leases
                    .acquisitions
                    .values()
                    .filter(|acquisition| {
                        !matches!(
                            acquisition.state,
                            crate::CompletionLeaseAcquisitionStateV1::Released
                                | crate::CompletionLeaseAcquisitionStateV1::Expired
                        )
                    })
                    .flat_map(|acquisition| {
                        acquisition
                            .eligibility_envelope
                            .exact_evidence
                            .iter()
                            .chain(&acquisition.eligibility_envelope.capability_bindings)
                            .chain(&acquisition.eligibility_envelope.route_evidence)
                            .cloned()
                    }),
            )
            .map(EvidenceRef::DomainRecordVersion)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        if dependencies.is_empty() {
            PayloadRequiredEvidenceContinuationV1::completed()
        } else {
            PayloadRequiredEvidenceContinuationV1::active(dependencies)
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResourceError {
    InvalidIdentifier(String),
    InvalidRevision,
    InvalidDefinition(String),
    NotFound(String),
    VersionConflict(String),
    Authority(String),
    Conservation(String),
    Capacity(String),
    LimitExceeded(String),
    MinimumUseful(String),
    ProtectedFloor(String),
    InvalidLifecycle(String),
    IdempotencyConflict(String),
    Overflow,
}

impl ResourceError {
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidIdentifier(_) => "invalid_identifier",
            Self::InvalidRevision => "invalid_revision",
            Self::InvalidDefinition(_) => "invalid_definition",
            Self::NotFound(_) => "not_found",
            Self::VersionConflict(_) => "version_conflict",
            Self::Authority(_) => "invalid_authority",
            Self::Conservation(_) => "conservation_violation",
            Self::Capacity(_) => "capacity_exceeded",
            Self::LimitExceeded(_) => "limit_exceeded",
            Self::MinimumUseful(_) => "minimum_useful_not_met",
            Self::ProtectedFloor(_) => "protected_floor",
            Self::InvalidLifecycle(_) => "invalid_lifecycle",
            Self::IdempotencyConflict(_) => "idempotency_conflict",
            Self::Overflow => "numeric_overflow",
        }
    }
}

impl Display for ResourceError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidRevision => formatter.write_str("resource revision must be non-zero"),
            Self::Overflow => formatter.write_str("resource arithmetic overflowed"),
            Self::InvalidIdentifier(message)
            | Self::InvalidDefinition(message)
            | Self::NotFound(message)
            | Self::VersionConflict(message)
            | Self::Authority(message)
            | Self::Conservation(message)
            | Self::Capacity(message)
            | Self::LimitExceeded(message)
            | Self::MinimumUseful(message)
            | Self::ProtectedFloor(message)
            | Self::InvalidLifecycle(message)
            | Self::IdempotencyConflict(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for ResourceError {}

pub fn canonical_digest<T: Serialize>(domain: &str, value: &T) -> Result<String, ResourceError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| ResourceError::InvalidDefinition(error.to_string()))?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain.as_bytes());
    hasher.update(&[0]);
    hasher.update(&bytes);
    Ok(hasher.finalize().to_hex().to_string())
}

pub(crate) fn holder_entity(holder: &KnowledgeHolderRef) -> EntityRef {
    match holder {
        KnowledgeHolderRef::Person(person) => EntityRef::Person(*person),
        KnowledgeHolderRef::Entity(entity) => entity.clone(),
    }
}

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn resource_canwu_error(error: ResourceError) -> canwu_api::CanwuError {
    canwu_api::CanwuError::new(canwu_api::ErrorCode::InvalidDomainRecord, error.to_string())
}
