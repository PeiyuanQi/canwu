use crate::{
    ResourceAccountId, ResourceAllocationLegId, ResourceDemandId, ResourceError,
    ResourceOperationKey, ResourceRevision, ResourceTransferId, canonical_digest,
};
use canwu_api::{DomainRecordVersionRef, KnowledgeHolderRef, SimTime};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Display, Formatter};

pub const MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE: u16 = 16;
pub const MAX_COMPLETION_MUTATIONS_PER_LIFECYCLE: u16 = 32;
pub const MAX_COMPLETION_REPORTS_PER_HOLDER: u16 = 8;
pub const MAX_COMPLETION_BYTES_PER_LIFECYCLE: u32 = 256 * 1024;
pub const MAX_GRANTS_PER_ACQUISITION: usize = 8;
pub const MAX_PENDING_LEASE_ACQUISITIONS_PER_AUTHORITY: usize = 16;
pub const MAX_PENDING_LEASE_ACQUISITIONS_GLOBAL: usize = 1_024;
pub const MAX_RESERVED_PENDING_SLOTS_PER_AUTHORITY: u16 = 16;
pub const MAX_REQUEST_TOKENS_PER_AUTHORITY: u16 = 16;
pub const REQUEST_TOKEN_REFILL_INTERVAL_MINUTES: u64 = 1;
pub const MIN_REACQUIRE_COOLDOWN_MINUTES: u64 = 1;
pub const MAX_ROOT_ACQUISITIONS_PER_AUTHORITY_PER_SIM_TIME: u16 = 16;
pub const PREACTIVATION_LEASE_TTL_BOUNDARIES: u64 = 8;
pub const ACTIVATION_GUARD_BOUNDARIES: u64 = 2;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct CompletionLeaseAcquisitionId(String);

impl CompletionLeaseAcquisitionId {
    pub fn new(value: impl Into<String>) -> Result<Self, ResourceError> {
        let value = value.into();
        let valid = !value.is_empty()
            && value.len() <= 160
            && value.contains(':')
            && value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':' | b'/')
            });
        if !valid {
            return Err(ResourceError::InvalidIdentifier(
                "completion lease acquisition must be a namespaced identifier".to_owned(),
            ));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for CompletionLeaseAcquisitionId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for CompletionLeaseAcquisitionId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct CompletionCapacityGrantId(String);

impl CompletionCapacityGrantId {
    pub fn new(value: impl Into<String>) -> Result<Self, ResourceError> {
        let value = value.into();
        if value.is_empty() || value.len() > 160 || !value.contains(':') {
            return Err(ResourceError::InvalidIdentifier(
                "completion capacity grant must be a namespaced identifier".to_owned(),
            ));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for CompletionCapacityGrantId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for CompletionCapacityGrantId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionPolicyClassV1 {
    Guaranteed,
    SharedBurst,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompletionCapacityRecipeV1 {
    pub receipts: u16,
    pub mutations: u16,
    pub reports_per_holder: u16,
    pub holders: u16,
    pub bytes: u32,
}

impl CompletionCapacityRecipeV1 {
    pub fn validate(&self) -> Result<(), ResourceError> {
        if self.receipts == 0
            || self.receipts > MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE
            || self.mutations == 0
            || self.mutations > MAX_COMPLETION_MUTATIONS_PER_LIFECYCLE
            || self.reports_per_holder > MAX_COMPLETION_REPORTS_PER_HOLDER
            || self.bytes == 0
            || self.bytes > MAX_COMPLETION_BYTES_PER_LIFECYCLE
        {
            return Err(ResourceError::LimitExceeded(
                "completion recipe exceeds one or more lifecycle maxima".to_owned(),
            ));
        }
        Ok(())
    }

    /// Fixed canonical lease cost used by independent consumers.
    pub fn canonical_units(&self) -> Result<u64, ResourceError> {
        self.validate()?;
        let report_count = u64::from(self.reports_per_holder)
            .checked_mul(u64::from(self.holders))
            .ok_or(ResourceError::Overflow)?;
        u64::from(self.receipts)
            .checked_mul(1_024)
            .and_then(|value| value.checked_add(u64::from(self.mutations) * 256))
            .and_then(|value| value.checked_add(report_count * 512))
            .and_then(|value| value.checked_add(u64::from(self.bytes).div_ceil(1_024)))
            .ok_or(ResourceError::Overflow)
    }

    pub fn digest(&self) -> Result<String, ResourceError> {
        canonical_digest("canwu.resource.completion-recipe.v1", self)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EligibilityEnvelopeV1 {
    pub digest: String,
    pub exact_evidence: Vec<DomainRecordVersionRef>,
    pub demand_expiries: BTreeMap<String, SimTime>,
    pub protected_floor_revisions: BTreeSet<String>,
    pub capability_bindings: Vec<DomainRecordVersionRef>,
    pub route_evidence: Vec<DomainRecordVersionRef>,
}

impl EligibilityEnvelopeV1 {
    pub fn new(
        exact_evidence: Vec<DomainRecordVersionRef>,
        demand_expiries: BTreeMap<String, SimTime>,
        protected_floor_revisions: BTreeSet<String>,
        capability_bindings: Vec<DomainRecordVersionRef>,
        route_evidence: Vec<DomainRecordVersionRef>,
    ) -> Result<Self, ResourceError> {
        let mut value = Self {
            digest: String::new(),
            exact_evidence,
            demand_expiries,
            protected_floor_revisions,
            capability_bindings,
            route_evidence,
        };
        value.canonicalize();
        value.digest = value.computed_digest()?;
        Ok(value)
    }

    fn canonicalize(&mut self) {
        self.exact_evidence.sort();
        self.exact_evidence.dedup();
        self.capability_bindings.sort();
        self.capability_bindings.dedup();
        self.route_evidence.sort();
        self.route_evidence.dedup();
    }

    pub fn computed_digest(&self) -> Result<String, ResourceError> {
        let mut detached = self.clone();
        detached.digest.clear();
        canonical_digest("canwu.resource.eligibility-envelope.v1", &detached)
    }

    pub fn validate(&self) -> Result<(), ResourceError> {
        let mut canonical = self.clone();
        canonical.canonicalize();
        if canonical.exact_evidence != self.exact_evidence
            || canonical.capability_bindings != self.capability_bindings
            || canonical.route_evidence != self.route_evidence
            || self.digest != self.computed_digest()?
        {
            return Err(ResourceError::InvalidDefinition(
                "eligibility envelope is not canonical or has a forged digest".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompletionCapacityPartitionV1 {
    pub authority: KnowledgeHolderRef,
    pub operation_namespace: String,
    pub guaranteed_units: u64,
    pub reserved_pending_slots: u16,
    pub maximum_burst_units: u64,
    pub request_token_capacity: u16,
    pub request_token_refill_minutes: u64,
    pub reacquire_cooldown_minutes: u64,
    pub root_acquisition_cap_per_sim_time: u16,
    pub guaranteed_max_wait_boundaries: u64,
}

impl CompletionCapacityPartitionV1 {
    pub fn validate(&self) -> Result<(), ResourceError> {
        if self.operation_namespace.is_empty()
            || self.reserved_pending_slots == 0
            || self.reserved_pending_slots > MAX_RESERVED_PENDING_SLOTS_PER_AUTHORITY
            || self.request_token_capacity == 0
            || self.request_token_capacity > MAX_REQUEST_TOKENS_PER_AUTHORITY
            || self.request_token_refill_minutes < REQUEST_TOKEN_REFILL_INTERVAL_MINUTES
            || self.reacquire_cooldown_minutes < MIN_REACQUIRE_COOLDOWN_MINUTES
            || self.root_acquisition_cap_per_sim_time == 0
            || self.root_acquisition_cap_per_sim_time
                > MAX_ROOT_ACQUISITIONS_PER_AUTHORITY_PER_SIM_TIME
            || self.guaranteed_max_wait_boundaries == 0
        {
            return Err(ResourceError::LimitExceeded(
                "completion capacity partition violates a V1 bound".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RunBudgetRevisionV1 {
    pub revision: ResourceRevision,
    pub total_completion_units: u64,
    pub shared_pending_slots: u16,
    pub partitions: Vec<CompletionCapacityPartitionV1>,
    pub semantic_digest: String,
}

impl RunBudgetRevisionV1 {
    pub fn validate(&self) -> Result<(), ResourceError> {
        if self.total_completion_units == 0
            || usize::from(self.shared_pending_slots) > MAX_PENDING_LEASE_ACQUISITIONS_GLOBAL
        {
            return Err(ResourceError::LimitExceeded(
                "run budget has invalid global completion capacity".to_owned(),
            ));
        }
        let mut keys = BTreeSet::new();
        let mut reserved_slots = usize::from(self.shared_pending_slots);
        let mut guaranteed_units = 0_u64;
        for partition in &self.partitions {
            partition.validate()?;
            if !keys.insert((
                partition.authority.clone(),
                partition.operation_namespace.clone(),
            )) {
                return Err(ResourceError::InvalidDefinition(
                    "run budget contains a duplicate authority partition".to_owned(),
                ));
            }
            reserved_slots = reserved_slots
                .checked_add(usize::from(partition.reserved_pending_slots))
                .ok_or(ResourceError::Overflow)?;
            guaranteed_units = guaranteed_units
                .checked_add(partition.guaranteed_units)
                .ok_or(ResourceError::Overflow)?;
        }
        if reserved_slots > MAX_PENDING_LEASE_ACQUISITIONS_GLOBAL
            || guaranteed_units > self.total_completion_units
        {
            return Err(ResourceError::LimitExceeded(
                "run budget reserved slots or guaranteed units exceed the global pool".to_owned(),
            ));
        }
        let mut detached = self.clone();
        detached.semantic_digest.clear();
        let digest = canonical_digest("canwu.resource.run-budget.v1", &detached)?;
        if !self.semantic_digest.is_empty() && self.semantic_digest != digest {
            return Err(ResourceError::InvalidDefinition(
                "run budget semantic digest is invalid".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn seal(mut self) -> Result<Self, ResourceError> {
        self.semantic_digest.clear();
        self.validate()?;
        self.semantic_digest = canonical_digest("canwu.resource.run-budget.v1", &self)?;
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionLeaseAcquisitionStateV1 {
    Requested,
    PartiallyGranted,
    FullyGranted,
    Preparing,
    PreparedAll,
    Activated,
    Aborting,
    Released,
    Expired,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionGrantStateV1 {
    Held,
    Prepared,
    Consumed,
    Completed,
    Released,
    Rejected,
    Expired,
}

/// Exact resource or external record version locked by one completion grant.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CompletionLockedTargetV1 {
    Account {
        id: ResourceAccountId,
        revision: ResourceRevision,
    },
    AllocationLeg {
        id: ResourceAllocationLegId,
        revision: ResourceRevision,
    },
    Demand {
        id: ResourceDemandId,
        revision: ResourceRevision,
    },
    Transfer {
        id: ResourceTransferId,
        revision: ResourceRevision,
    },
    ExternalRecord {
        version: DomainRecordVersionRef,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompletionCapacityGrantV1 {
    pub id: CompletionCapacityGrantId,
    pub revision: ResourceRevision,
    pub acquisition: CompletionLeaseAcquisitionId,
    pub operation_key: ResourceOperationKey,
    pub owner_plugin: String,
    pub run_budget_revision: ResourceRevision,
    pub target_versions: Vec<CompletionLockedTargetV1>,
    pub recipe_digest: String,
    pub reserved_units: u64,
    pub expires_after_boundary: u64,
    pub activation_deadline_boundary: Option<u64>,
    pub state: CompletionGrantStateV1,
    pub rejection: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompletionLeaseAcquisitionV1 {
    pub id: CompletionLeaseAcquisitionId,
    pub revision: ResourceRevision,
    pub operation_key: ResourceOperationKey,
    pub holder: KnowledgeHolderRef,
    pub operation_namespace: String,
    pub eligibility_time: SimTime,
    pub eligibility_envelope: EligibilityEnvelopeV1,
    pub recipe: CompletionCapacityRecipeV1,
    pub recipe_digest: String,
    pub expected_participants: BTreeSet<String>,
    pub grants: BTreeMap<String, CompletionCapacityGrantId>,
    pub policy_class: CompletionPolicyClassV1,
    pub admitted_sequence: u64,
    pub fairness_round: u64,
    pub state: CompletionLeaseAcquisitionStateV1,
    pub blocker: Option<String>,
    pub refunded_units: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompletionLeaseActivationCertificateV1 {
    pub acquisition: CompletionLeaseAcquisitionId,
    pub acquisition_revision: ResourceRevision,
    pub operation_key: ResourceOperationKey,
    pub prepared_grants: Vec<(CompletionCapacityGrantId, ResourceRevision)>,
    pub locked_target_versions: Vec<CompletionLockedTargetV1>,
    pub recipe_digest: String,
    pub eligibility_time: SimTime,
    pub eligibility_envelope_digest: String,
    pub activation_boundary: u64,
    pub semantic_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RequestCompletionLeaseV1 {
    pub id: CompletionLeaseAcquisitionId,
    pub operation_key: ResourceOperationKey,
    pub holder: KnowledgeHolderRef,
    pub operation_namespace: String,
    pub eligibility_time: SimTime,
    pub eligibility_envelope: EligibilityEnvelopeV1,
    pub recipe: CompletionCapacityRecipeV1,
    pub expected_participants: BTreeSet<String>,
    pub policy_class: CompletionPolicyClassV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GrantCompletionCapacityV1 {
    pub grant_id: CompletionCapacityGrantId,
    pub acquisition: CompletionLeaseAcquisitionId,
    pub expected_acquisition_revision: ResourceRevision,
    pub owner_plugin: String,
    pub target_versions: Vec<CompletionLockedTargetV1>,
    pub current_boundary: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PrepareCompletionCapacityV1 {
    pub acquisition: CompletionLeaseAcquisitionId,
    pub expected_acquisition_revision: ResourceRevision,
    pub grant: CompletionCapacityGrantId,
    pub expected_grant_revision: ResourceRevision,
    pub current_boundary: u64,
    pub eligibility_envelope_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AbortCompletionLeaseV1 {
    pub acquisition: CompletionLeaseAcquisitionId,
    pub expected_revision: ResourceRevision,
    pub holder: KnowledgeHolderRef,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ActivateCompletionLeaseV1 {
    pub acquisition: CompletionLeaseAcquisitionId,
    pub expected_acquisition_revision: ResourceRevision,
    pub grant: CompletionCapacityGrantId,
    pub expected_grant_revision: ResourceRevision,
    pub at: SimTime,
    pub current_boundary: u64,
    pub eligibility_envelope_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExpireCompletionCapacityV1 {
    pub at: SimTime,
    pub current_boundary: u64,
    pub candidate_limit: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReleaseCompletionCapacityV1 {
    pub acquisition: CompletionLeaseAcquisitionId,
    pub expected_acquisition_revision: ResourceRevision,
    pub grant: CompletionCapacityGrantId,
    pub expected_grant_revision: ResourceRevision,
    pub reason: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionLeaseReceiptActionV1 {
    Requested,
    Granted,
    Prepared,
    Rejected,
    Activated,
    Aborted,
    Expired,
    Released,
    Consumed,
    Completed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompletionLeaseReceiptV1 {
    pub sequence: u64,
    pub operation_key: ResourceOperationKey,
    pub acquisition: CompletionLeaseAcquisitionId,
    pub grant: Option<CompletionCapacityGrantId>,
    pub action: CompletionLeaseReceiptActionV1,
    pub state: CompletionLeaseAcquisitionStateV1,
    pub reserved_units: u64,
    pub refunded_units: u64,
    pub reason: Option<String>,
    pub semantic_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AdmissionEpochV1 {
    pub at: SimTime,
    pub token_balance: u16,
    pub last_refill_minute: i64,
    pub next_eligible_minute: i64,
    pub root_acquisition_count: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompletionLeaseStatusDtoV1 {
    pub acquisition: CompletionLeaseAcquisitionId,
    pub operation_key: ResourceOperationKey,
    pub state: CompletionLeaseAcquisitionStateV1,
    pub grant_states: BTreeMap<String, CompletionGrantStateV1>,
    pub exact_grant_versions: BTreeMap<String, ResourceRevision>,
    pub eligibility_time: SimTime,
    pub expiry_boundaries: BTreeMap<String, u64>,
    pub activation_deadlines: BTreeMap<String, u64>,
    pub blocker: Option<String>,
    pub refunded_units: u64,
    pub next_eligible_action: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompletionLeaseBookV1 {
    pub acquisitions: BTreeMap<CompletionLeaseAcquisitionId, CompletionLeaseAcquisitionV1>,
    pub grants: BTreeMap<CompletionCapacityGrantId, CompletionCapacityGrantV1>,
    #[serde(with = "completion_epoch_map")]
    pub epochs: BTreeMap<(KnowledgeHolderRef, String), AdmissionEpochV1>,
    pub certificates:
        BTreeMap<CompletionLeaseAcquisitionId, CompletionLeaseActivationCertificateV1>,
    #[serde(with = "completion_target_lock_map")]
    pub target_locks: BTreeMap<CompletionLockedTargetV1, CompletionCapacityGrantId>,
    pub expiry_due: BTreeMap<u64, BTreeSet<CompletionCapacityGrantId>>,
    pub receipts: BTreeMap<u64, CompletionLeaseReceiptV1>,
    pub reserved_units: u64,
    pub next_sequence: u64,
}

/// A resource-owned participant grant for a completion acquisition coordinated
/// by another plugin. The coordinator remains the sole writer of the
/// acquisition and activation certificate; this record is the resource
/// plugin's authoritative capacity reservation and exact-target lock.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExternalCompletionParticipantGrantV1 {
    pub coordinator_plugin: String,
    pub coordinator_source: DomainRecordVersionRef,
    pub coordinator_acquisition_revision: ResourceRevision,
    pub holder: KnowledgeHolderRef,
    pub operation_namespace: String,
    pub eligibility_time: SimTime,
    pub eligibility_envelope_digest: String,
    pub recipe: CompletionCapacityRecipeV1,
    pub policy_class: CompletionPolicyClassV1,
    pub grant: CompletionCapacityGrantV1,
    pub certificate: Option<CompletionLeaseActivationCertificateV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RequestExternalCompletionParticipantGrantV1 {
    pub coordinator_plugin: String,
    pub coordinator_source: DomainRecordVersionRef,
    pub coordinator_acquisition_revision: ResourceRevision,
    pub acquisition: CompletionLeaseAcquisitionId,
    pub operation_key: ResourceOperationKey,
    pub holder: KnowledgeHolderRef,
    pub operation_namespace: String,
    pub eligibility_time: SimTime,
    pub eligibility_envelope_digest: String,
    pub recipe: CompletionCapacityRecipeV1,
    pub policy_class: CompletionPolicyClassV1,
    pub grant_id: CompletionCapacityGrantId,
    pub target_versions: Vec<CompletionLockedTargetV1>,
    pub current_boundary: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PrepareExternalCompletionParticipantGrantV1 {
    pub coordinator_source: DomainRecordVersionRef,
    pub acquisition: CompletionLeaseAcquisitionId,
    pub expected_grant_revision: ResourceRevision,
    pub current_boundary: u64,
    pub eligibility_envelope_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ConsumeExternalCompletionParticipantGrantV1 {
    pub coordinator_source: DomainRecordVersionRef,
    pub certificate: CompletionLeaseActivationCertificateV1,
    pub at: SimTime,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompleteExternalCompletionParticipantGrantV1 {
    pub acquisition: CompletionLeaseAcquisitionId,
    pub operation_key: ResourceOperationKey,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReleaseExternalCompletionParticipantGrantV1 {
    pub coordinator_source: DomainRecordVersionRef,
    pub acquisition: CompletionLeaseAcquisitionId,
    pub expected_grant_revision: ResourceRevision,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExpireExternalCompletionParticipantGrantsV1 {
    pub at: SimTime,
    pub current_boundary: u64,
    pub candidate_limit: usize,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExternalCompletionParticipantBookV1 {
    pub grants: BTreeMap<CompletionLeaseAcquisitionId, ExternalCompletionParticipantGrantV1>,
    #[serde(default)]
    pub terminal_grants:
        BTreeMap<CompletionLeaseAcquisitionId, ExternalCompletionParticipantGrantV1>,
    #[serde(with = "completion_target_lock_map")]
    pub target_locks: BTreeMap<CompletionLockedTargetV1, CompletionCapacityGrantId>,
    pub expiry_due: BTreeMap<u64, BTreeSet<CompletionLeaseAcquisitionId>>,
    pub reserved_units: u64,
}

impl ExternalCompletionParticipantBookV1 {
    #[must_use]
    pub fn participant(
        &self,
        acquisition: &CompletionLeaseAcquisitionId,
    ) -> Option<&ExternalCompletionParticipantGrantV1> {
        self.grants
            .get(acquisition)
            .or_else(|| self.terminal_grants.get(acquisition))
    }

    pub fn grant(
        &mut self,
        budget: &RunBudgetRevisionV1,
        request: RequestExternalCompletionParticipantGrantV1,
    ) -> Result<ExternalCompletionParticipantGrantV1, ResourceError> {
        budget.validate()?;
        request.recipe.validate()?;
        if request.coordinator_plugin.is_empty()
            || request.eligibility_envelope_digest.is_empty()
            || request.operation_namespace.is_empty()
            || request.target_versions.is_empty()
        {
            return Err(ResourceError::InvalidDefinition(
                "external completion participant request is incomplete".to_owned(),
            ));
        }
        let mut targets = request.target_versions.clone();
        targets.sort();
        targets.dedup();
        if targets != request.target_versions {
            return Err(ResourceError::InvalidDefinition(
                "external completion participant targets are not canonical".to_owned(),
            ));
        }
        let partition = budget
            .partitions
            .iter()
            .find(|partition| {
                partition.authority == request.holder
                    && partition.operation_namespace == request.operation_namespace
            })
            .ok_or_else(|| {
                ResourceError::Authority(
                    "external completion participant has no resource run-budget partition"
                        .to_owned(),
                )
            })?;
        let units = request.recipe.canonical_units()?;
        if (request.policy_class == CompletionPolicyClassV1::Guaranteed
            && units > partition.guaranteed_units)
            || (request.policy_class == CompletionPolicyClassV1::SharedBurst
                && units > partition.maximum_burst_units)
            || self
                .reserved_units
                .checked_add(units)
                .is_none_or(|total| total > budget.total_completion_units)
        {
            return Err(ResourceError::LimitExceeded(
                "external completion participant capacity is exhausted".to_owned(),
            ));
        }
        if let Some(existing) = self.grants.get(&request.acquisition) {
            if existing.coordinator_plugin == request.coordinator_plugin
                && existing.coordinator_source == request.coordinator_source
                && existing.coordinator_acquisition_revision
                    == request.coordinator_acquisition_revision
                && existing.holder == request.holder
                && existing.operation_namespace == request.operation_namespace
                && existing.eligibility_time == request.eligibility_time
                && existing.eligibility_envelope_digest == request.eligibility_envelope_digest
                && existing.recipe == request.recipe
                && existing.policy_class == request.policy_class
                && existing.grant.id == request.grant_id
                && existing.grant.operation_key == request.operation_key
                && existing.grant.target_versions == request.target_versions
            {
                return Ok(existing.clone());
            }
            return Err(ResourceError::IdempotencyConflict(
                "external completion acquisition was reused with changed participant data"
                    .to_owned(),
            ));
        }
        if self.terminal_grants.contains_key(&request.acquisition) {
            return Err(ResourceError::IdempotencyConflict(
                "external completion acquisition is already terminal".to_owned(),
            ));
        }
        if self.grants.len() >= MAX_PENDING_LEASE_ACQUISITIONS_GLOBAL
            || self
                .grants
                .values()
                .filter(|value| {
                    value.holder == request.holder
                        && matches!(
                            value.grant.state,
                            CompletionGrantStateV1::Held
                                | CompletionGrantStateV1::Prepared
                                | CompletionGrantStateV1::Consumed
                        )
                })
                .count()
                >= MAX_PENDING_LEASE_ACQUISITIONS_PER_AUTHORITY
        {
            return Err(ResourceError::LimitExceeded(
                "external completion participant pending capacity is exhausted".to_owned(),
            ));
        }
        let recipe_digest = request.recipe.digest()?;
        let grant = CompletionCapacityGrantV1 {
            id: request.grant_id,
            revision: ResourceRevision::INITIAL,
            acquisition: request.acquisition.clone(),
            operation_key: request.operation_key,
            owner_plugin: crate::PLUGIN_NAME.to_owned(),
            run_budget_revision: budget.revision,
            target_versions: request.target_versions,
            recipe_digest,
            reserved_units: units,
            expires_after_boundary: request
                .current_boundary
                .checked_add(PREACTIVATION_LEASE_TTL_BOUNDARIES)
                .ok_or(ResourceError::Overflow)?,
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
        self.reserved_units = self
            .reserved_units
            .checked_add(units)
            .ok_or(ResourceError::Overflow)?;
        self.expiry_due
            .entry(participant.grant.expires_after_boundary)
            .or_default()
            .insert(request.acquisition.clone());
        self.grants.insert(request.acquisition, participant.clone());
        Ok(participant)
    }

    pub fn prepare(
        &mut self,
        request: PrepareExternalCompletionParticipantGrantV1,
    ) -> Result<ExternalCompletionParticipantGrantV1, ResourceError> {
        let snapshot = self
            .grants
            .get(&request.acquisition)
            .cloned()
            .ok_or_else(|| {
                ResourceError::NotFound("external participant grant is unavailable".to_owned())
            })?;
        if snapshot.grant.revision != request.expected_grant_revision
            || snapshot.grant.state != CompletionGrantStateV1::Held
            || snapshot.eligibility_envelope_digest != request.eligibility_envelope_digest
            || request
                .current_boundary
                .checked_add(ACTIVATION_GUARD_BOUNDARIES)
                .is_none_or(|guard| guard > snapshot.grant.expires_after_boundary)
        {
            return Err(ResourceError::VersionConflict(
                "external participant prepare exact grant, envelope, or activation window differs"
                    .to_owned(),
            ));
        }
        if snapshot
            .grant
            .target_versions
            .iter()
            .any(|target| self.target_locks.contains_key(target))
        {
            return Err(ResourceError::VersionConflict(
                "external participant target is already completion-locked".to_owned(),
            ));
        }
        for target in &snapshot.grant.target_versions {
            self.target_locks
                .insert(target.clone(), snapshot.grant.id.clone());
        }
        let participant = self
            .grants
            .get_mut(&request.acquisition)
            .expect("external participant grant was checked");
        participant.coordinator_source = request.coordinator_source;
        participant.grant.state = CompletionGrantStateV1::Prepared;
        participant.grant.activation_deadline_boundary = Some(
            participant
                .grant
                .expires_after_boundary
                .saturating_sub(ACTIVATION_GUARD_BOUNDARIES - 1),
        );
        participant.grant.revision = participant.grant.revision.next()?;
        Ok(participant.clone())
    }

    pub(crate) fn reject_prepare(
        &mut self,
        request: &PrepareExternalCompletionParticipantGrantV1,
        reason: &str,
    ) -> Result<ExternalCompletionParticipantGrantV1, ResourceError> {
        let participant = self.grants.get_mut(&request.acquisition).ok_or_else(|| {
            ResourceError::NotFound("external participant grant is unavailable".to_owned())
        })?;
        if participant.grant.revision != request.expected_grant_revision
            || participant.grant.state != CompletionGrantStateV1::Held
        {
            return Err(ResourceError::VersionConflict(
                "external participant prepare rejection names a stale grant".to_owned(),
            ));
        }
        participant.coordinator_source = request.coordinator_source.clone();
        participant.grant.state = CompletionGrantStateV1::Rejected;
        participant.grant.rejection = Some(reason.to_owned());
        participant.grant.revision = participant.grant.revision.next()?;
        self.reserved_units = self
            .reserved_units
            .checked_sub(participant.grant.reserved_units)
            .ok_or(ResourceError::Conservation(
                "external participant rejected completion units underflowed".to_owned(),
            ))?;
        for acquisitions in self.expiry_due.values_mut() {
            acquisitions.remove(&request.acquisition);
        }
        self.expiry_due.retain(|_, values| !values.is_empty());
        Ok(participant.clone())
    }

    pub fn consume(
        &mut self,
        request: ConsumeExternalCompletionParticipantGrantV1,
    ) -> Result<ExternalCompletionParticipantGrantV1, ResourceError> {
        let mut detached = request.certificate.clone();
        let digest = std::mem::take(&mut detached.semantic_digest);
        if digest
            != canonical_digest(
                "canwu.resource.completion-activation-certificate.v1",
                &detached,
            )?
        {
            return Err(ResourceError::InvalidDefinition(
                "external participant certificate digest is invalid".to_owned(),
            ));
        }
        let participant = self
            .grants
            .get_mut(&request.certificate.acquisition)
            .ok_or_else(|| {
                ResourceError::NotFound("external participant grant is unavailable".to_owned())
            })?;
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
                self.target_locks.get(target) != Some(&participant.grant.id)
                    || !request.certificate.locked_target_versions.contains(target)
            })
        {
            return Err(ResourceError::VersionConflict(
                "external participant certificate does not bind the prepared resource grant"
                    .to_owned(),
            ));
        }
        participant.coordinator_source = request.coordinator_source;
        participant.certificate = Some(request.certificate);
        participant.grant.state = CompletionGrantStateV1::Consumed;
        participant.grant.revision = participant.grant.revision.next()?;
        for acquisitions in self.expiry_due.values_mut() {
            acquisitions.remove(&participant.grant.acquisition);
        }
        self.expiry_due.retain(|_, values| !values.is_empty());
        Ok(participant.clone())
    }

    pub fn complete(
        &mut self,
        request: &CompleteExternalCompletionParticipantGrantV1,
    ) -> Result<ExternalCompletionParticipantGrantV1, ResourceError> {
        if let Some(participant) = self.terminal_grants.get(&request.acquisition) {
            if participant.grant.operation_key == request.operation_key
                && participant.grant.state == CompletionGrantStateV1::Completed
            {
                return Ok(participant.clone());
            }
            return Err(ResourceError::IdempotencyConflict(
                "external participant completion names another terminal operation".to_owned(),
            ));
        }
        let participant = self.grants.get_mut(&request.acquisition).ok_or_else(|| {
            ResourceError::NotFound("external participant grant is unavailable".to_owned())
        })?;
        if participant.grant.operation_key != request.operation_key {
            return Err(ResourceError::IdempotencyConflict(
                "external participant completion names another operation".to_owned(),
            ));
        }
        if participant.grant.state != CompletionGrantStateV1::Consumed {
            return Err(ResourceError::InvalidLifecycle(
                "external participant completion requires a consumed grant".to_owned(),
            ));
        }
        participant.grant.state = CompletionGrantStateV1::Completed;
        participant.grant.revision = participant.grant.revision.next()?;
        self.reserved_units = self
            .reserved_units
            .checked_sub(participant.grant.reserved_units)
            .ok_or(ResourceError::Conservation(
                "external participant reserved completion units underflowed".to_owned(),
            ))?;
        for target in &participant.grant.target_versions {
            self.target_locks.remove(target);
        }
        let completed = participant.clone();
        self.grants.remove(&request.acquisition);
        self.terminal_grants
            .insert(request.acquisition.clone(), completed.clone());
        Ok(completed)
    }

    pub fn release(
        &mut self,
        request: ReleaseExternalCompletionParticipantGrantV1,
    ) -> Result<ExternalCompletionParticipantGrantV1, ResourceError> {
        let participant = self.grants.get_mut(&request.acquisition).ok_or_else(|| {
            ResourceError::NotFound("external participant grant is unavailable".to_owned())
        })?;
        if participant.grant.revision != request.expected_grant_revision {
            return Err(ResourceError::VersionConflict(
                "external participant release expected another grant revision".to_owned(),
            ));
        }
        if participant.grant.state == CompletionGrantStateV1::Consumed {
            return Err(ResourceError::InvalidLifecycle(
                "an activated external participant grant cannot be released".to_owned(),
            ));
        }
        if matches!(
            participant.grant.state,
            CompletionGrantStateV1::Released
                | CompletionGrantStateV1::Expired
                | CompletionGrantStateV1::Completed
        ) {
            return Ok(participant.clone());
        }
        participant.coordinator_source = request.coordinator_source;
        participant.grant.state = CompletionGrantStateV1::Released;
        participant.grant.rejection = Some(request.reason);
        participant.grant.revision = participant.grant.revision.next()?;
        self.reserved_units = self
            .reserved_units
            .checked_sub(participant.grant.reserved_units)
            .ok_or(ResourceError::Conservation(
                "external participant reserved completion units underflowed".to_owned(),
            ))?;
        for target in &participant.grant.target_versions {
            self.target_locks.remove(target);
        }
        for acquisitions in self.expiry_due.values_mut() {
            acquisitions.remove(&request.acquisition);
        }
        self.expiry_due.retain(|_, values| !values.is_empty());
        Ok(participant.clone())
    }

    pub fn expire(
        &mut self,
        request: &ExpireExternalCompletionParticipantGrantsV1,
    ) -> Result<Vec<CompletionLeaseAcquisitionId>, ResourceError> {
        if request.candidate_limit == 0
            || request.candidate_limit > MAX_PENDING_LEASE_ACQUISITIONS_GLOBAL
        {
            return Err(ResourceError::LimitExceeded(
                "external participant expiry candidate limit is invalid".to_owned(),
            ));
        }
        let candidates = self
            .expiry_due
            .range(..=request.current_boundary)
            .flat_map(|(_, values)| values.iter().cloned())
            .take(request.candidate_limit.saturating_add(1))
            .collect::<Vec<_>>();
        if candidates.len() > request.candidate_limit {
            return Err(ResourceError::LimitExceeded(
                "external participant expiry due-work budget was exceeded".to_owned(),
            ));
        }
        let mut expired = Vec::new();
        for acquisition in candidates {
            let participant = self.grants.get_mut(&acquisition).ok_or_else(|| {
                ResourceError::InvalidDefinition(
                    "external participant expiry index is orphaned".to_owned(),
                )
            })?;
            if request.at < participant.eligibility_time {
                return Err(ResourceError::VersionConflict(
                    "external participant expiry moved before eligibility time".to_owned(),
                ));
            }
            if !matches!(
                participant.grant.state,
                CompletionGrantStateV1::Held | CompletionGrantStateV1::Prepared
            ) {
                continue;
            }
            participant.grant.state = CompletionGrantStateV1::Expired;
            participant.grant.revision = participant.grant.revision.next()?;
            self.reserved_units = self
                .reserved_units
                .checked_sub(participant.grant.reserved_units)
                .ok_or(ResourceError::Conservation(
                    "external participant reserved completion units underflowed".to_owned(),
                ))?;
            for target in &participant.grant.target_versions {
                self.target_locks.remove(target);
            }
            expired.push(acquisition);
        }
        for values in self.expiry_due.values_mut() {
            values.retain(|acquisition| !expired.contains(acquisition));
        }
        self.expiry_due.retain(|_, values| !values.is_empty());
        Ok(expired)
    }

    pub fn validate(&self, budget: &RunBudgetRevisionV1) -> Result<(), ResourceError> {
        budget.validate()?;
        if self
            .terminal_grants
            .keys()
            .any(|acquisition| self.grants.contains_key(acquisition))
        {
            return Err(ResourceError::InvalidDefinition(
                "external completion participant is both active and terminal".to_owned(),
            ));
        }
        let mut reserved = 0_u64;
        for (acquisition, participant) in &self.grants {
            participant.recipe.validate()?;
            if acquisition != &participant.grant.acquisition
                || participant.grant.owner_plugin != crate::PLUGIN_NAME
                || participant.grant.run_budget_revision != budget.revision
                || participant.grant.recipe_digest != participant.recipe.digest()?
                || participant.eligibility_envelope_digest.is_empty()
            {
                return Err(ResourceError::InvalidDefinition(
                    "external completion participant grant closure is invalid".to_owned(),
                ));
            }
            if matches!(
                participant.grant.state,
                CompletionGrantStateV1::Held
                    | CompletionGrantStateV1::Prepared
                    | CompletionGrantStateV1::Consumed
            ) {
                reserved = reserved
                    .checked_add(participant.grant.reserved_units)
                    .ok_or(ResourceError::Overflow)?;
            }
            if participant.grant.state == CompletionGrantStateV1::Consumed
                && participant.certificate.is_none()
            {
                return Err(ResourceError::InvalidDefinition(
                    "consumed external participant grant lost its coordinator certificate"
                        .to_owned(),
                ));
            }
        }
        for (acquisition, participant) in &self.terminal_grants {
            participant.recipe.validate()?;
            if acquisition != &participant.grant.acquisition
                || participant.grant.owner_plugin != crate::PLUGIN_NAME
                || participant.grant.run_budget_revision != budget.revision
                || participant.grant.recipe_digest != participant.recipe.digest()?
                || participant.eligibility_envelope_digest.is_empty()
                || participant.grant.state != CompletionGrantStateV1::Completed
                || participant.certificate.is_none()
            {
                return Err(ResourceError::InvalidDefinition(
                    "terminal external completion participant grant is invalid".to_owned(),
                ));
            }
        }
        if reserved != self.reserved_units
            || self
                .reserved_units
                .checked_add(0)
                .is_none_or(|value| value > budget.total_completion_units)
        {
            return Err(ResourceError::Conservation(
                "external participant completion reserve does not reconcile".to_owned(),
            ));
        }
        for (target, grant_id) in &self.target_locks {
            let participant = self
                .grants
                .values()
                .find(|value| &value.grant.id == grant_id)
                .ok_or_else(|| {
                    ResourceError::InvalidDefinition(
                        "external participant target lock is orphaned".to_owned(),
                    )
                })?;
            if !participant.grant.target_versions.contains(target)
                || !matches!(
                    participant.grant.state,
                    CompletionGrantStateV1::Prepared | CompletionGrantStateV1::Consumed
                )
            {
                return Err(ResourceError::InvalidDefinition(
                    "external participant target lock is invalid".to_owned(),
                ));
            }
        }
        Ok(())
    }
}

impl Default for CompletionLeaseBookV1 {
    fn default() -> Self {
        Self {
            acquisitions: BTreeMap::new(),
            grants: BTreeMap::new(),
            epochs: BTreeMap::new(),
            certificates: BTreeMap::new(),
            target_locks: BTreeMap::new(),
            expiry_due: BTreeMap::new(),
            receipts: BTreeMap::new(),
            reserved_units: 0,
            next_sequence: 1,
        }
    }
}

impl CompletionLeaseBookV1 {
    #[allow(clippy::too_many_lines)]
    pub fn request_acquisition(
        &mut self,
        budget: &RunBudgetRevisionV1,
        request: RequestCompletionLeaseV1,
    ) -> Result<CompletionLeaseAcquisitionV1, ResourceError> {
        budget.validate()?;
        request.eligibility_envelope.validate()?;
        request.recipe.validate()?;
        if request.expected_participants.is_empty()
            || request.expected_participants.len() > MAX_GRANTS_PER_ACQUISITION
        {
            return Err(ResourceError::LimitExceeded(
                "completion lease participant count is invalid".to_owned(),
            ));
        }
        let recipe_digest = request.recipe.digest()?;
        if let Some(existing) = self.acquisitions.get(&request.id) {
            if existing.operation_key == request.operation_key
                && existing.holder == request.holder
                && existing.operation_namespace == request.operation_namespace
                && existing.eligibility_time == request.eligibility_time
                && existing.eligibility_envelope == request.eligibility_envelope
                && existing.recipe_digest == recipe_digest
                && existing.expected_participants == request.expected_participants
                && existing.policy_class == request.policy_class
            {
                return Ok(existing.clone());
            }
            return Err(ResourceError::IdempotencyConflict(
                "completion lease acquisition key was reused with a changed recipe or envelope"
                    .to_owned(),
            ));
        }
        if self
            .acquisitions
            .values()
            .any(|existing| existing.operation_key == request.operation_key)
        {
            return Err(ResourceError::IdempotencyConflict(
                "completion lease operation key was reused by another acquisition".to_owned(),
            ));
        }
        let partition = budget
            .partitions
            .iter()
            .find(|partition| {
                partition.authority == request.holder
                    && partition.operation_namespace == request.operation_namespace
            })
            .ok_or_else(|| {
                ResourceError::Authority(
                    "completion lease authority/namespace has no run-budget partition".to_owned(),
                )
            })?;
        let global_pending = self
            .acquisitions
            .values()
            .filter(|value| is_pending(value.state))
            .count();
        let authority_pending = self
            .acquisitions
            .values()
            .filter(|value| value.holder == request.holder && is_pending(value.state))
            .count();
        let guaranteed_pending = self
            .acquisitions
            .values()
            .filter(|value| {
                value.holder == request.holder
                    && value.policy_class == CompletionPolicyClassV1::Guaranteed
                    && is_pending(value.state)
            })
            .count();
        let shared_pending = self
            .acquisitions
            .values()
            .filter(|value| {
                value.policy_class == CompletionPolicyClassV1::SharedBurst
                    && is_pending(value.state)
            })
            .count();
        let configured_pending = usize::from(budget.shared_pending_slots)
            + budget
                .partitions
                .iter()
                .map(|value| usize::from(value.reserved_pending_slots))
                .sum::<usize>();
        if global_pending >= MAX_PENDING_LEASE_ACQUISITIONS_GLOBAL
            || global_pending >= configured_pending
            || authority_pending >= MAX_PENDING_LEASE_ACQUISITIONS_PER_AUTHORITY
            || (request.policy_class == CompletionPolicyClassV1::Guaranteed
                && guaranteed_pending >= usize::from(partition.reserved_pending_slots))
            || (request.policy_class == CompletionPolicyClassV1::SharedBurst
                && shared_pending >= usize::from(budget.shared_pending_slots))
        {
            return Err(ResourceError::LimitExceeded(
                "completion lease pending-admission capacity is exhausted".to_owned(),
            ));
        }
        let units = request.recipe.canonical_units()?;
        if request.policy_class == CompletionPolicyClassV1::Guaranteed
            && units > partition.guaranteed_units
        {
            return Err(ResourceError::LimitExceeded(
                "completion lease exceeds the authority guarantee".to_owned(),
            ));
        }
        if request.policy_class == CompletionPolicyClassV1::SharedBurst
            && units > partition.maximum_burst_units
        {
            return Err(ResourceError::LimitExceeded(
                "completion lease exceeds the authority burst maximum".to_owned(),
            ));
        }
        let epoch_key = (request.holder.clone(), request.operation_namespace.clone());
        let minute = request.eligibility_time.as_minutes();
        let epoch = self.epochs.entry(epoch_key).or_insert(AdmissionEpochV1 {
            at: request.eligibility_time,
            token_balance: partition.request_token_capacity,
            last_refill_minute: minute,
            next_eligible_minute: minute,
            root_acquisition_count: 0,
        });
        if request.eligibility_time < epoch.at {
            return Err(ResourceError::VersionConflict(
                "completion admission epoch cannot move backward in simulation time".to_owned(),
            ));
        }
        if request.eligibility_time > epoch.at {
            let elapsed = minute.saturating_sub(epoch.last_refill_minute);
            let refill = elapsed
                / i64::try_from(partition.request_token_refill_minutes)
                    .map_err(|_| ResourceError::Overflow)?;
            if refill > 0 {
                let refill = u16::try_from(refill).unwrap_or(u16::MAX);
                epoch.token_balance = epoch
                    .token_balance
                    .saturating_add(refill)
                    .min(partition.request_token_capacity);
                epoch.last_refill_minute = minute;
            }
            epoch.at = request.eligibility_time;
            epoch.root_acquisition_count = 0;
        }
        if minute < epoch.next_eligible_minute
            || epoch.token_balance == 0
            || epoch.root_acquisition_count >= partition.root_acquisition_cap_per_sim_time
        {
            return Err(ResourceError::LimitExceeded(
                "completion lease request token, cooldown, or same-time root cap is exhausted"
                    .to_owned(),
            ));
        }
        epoch.token_balance -= 1;
        epoch.root_acquisition_count += 1;
        epoch.next_eligible_minute = minute
            .checked_add(
                i64::try_from(partition.reacquire_cooldown_minutes)
                    .map_err(|_| ResourceError::Overflow)?,
            )
            .ok_or(ResourceError::Overflow)?;
        let admitted_sequence = self.next_sequence()?;
        let acquisition = CompletionLeaseAcquisitionV1 {
            id: request.id.clone(),
            revision: ResourceRevision::INITIAL,
            operation_key: request.operation_key,
            holder: request.holder,
            operation_namespace: request.operation_namespace,
            eligibility_time: request.eligibility_time,
            eligibility_envelope: request.eligibility_envelope,
            recipe: request.recipe,
            recipe_digest,
            expected_participants: request.expected_participants,
            grants: BTreeMap::new(),
            policy_class: request.policy_class,
            admitted_sequence,
            fairness_round: 0,
            state: CompletionLeaseAcquisitionStateV1::Requested,
            blocker: None,
            refunded_units: 0,
        };
        self.acquisitions
            .insert(acquisition.id.clone(), acquisition.clone());
        Ok(acquisition)
    }

    pub fn grant_capacity(
        &mut self,
        budget: &RunBudgetRevisionV1,
        mut request: GrantCompletionCapacityV1,
    ) -> Result<CompletionCapacityGrantV1, ResourceError> {
        budget.validate()?;
        request.target_versions.sort();
        request.target_versions.dedup();
        if request.target_versions.is_empty() {
            return Err(ResourceError::InvalidDefinition(
                "completion grant must lock at least one exact target".to_owned(),
            ));
        }
        let acquisition = self
            .acquisitions
            .get(&request.acquisition)
            .cloned()
            .ok_or_else(|| {
                ResourceError::NotFound("completion acquisition is unavailable".to_owned())
            })?;
        if acquisition.grants.is_empty()
            && deterministic_completion_fairness_order(self.acquisitions.values().cloned()).first()
                != Some(&request.acquisition)
        {
            return Err(ResourceError::InvalidLifecycle(
                "completion grant violates deterministic admission fairness order".to_owned(),
            ));
        }
        if acquisition.revision != request.expected_acquisition_revision
            || !acquisition
                .expected_participants
                .contains(&request.owner_plugin)
            || !matches!(
                acquisition.state,
                CompletionLeaseAcquisitionStateV1::Requested
                    | CompletionLeaseAcquisitionStateV1::PartiallyGranted
            )
        {
            return Err(ResourceError::VersionConflict(
                "completion acquisition is stale or not grantable".to_owned(),
            ));
        }
        let recipe_digest = acquisition.recipe.digest()?;
        let units = acquisition.recipe.canonical_units()?;
        if let Some(existing) = self.grants.get(&request.grant_id) {
            if existing.acquisition == request.acquisition
                && existing.owner_plugin == request.owner_plugin
                && existing.target_versions == request.target_versions
                && existing.recipe_digest == recipe_digest
            {
                return Ok(existing.clone());
            }
            return Err(ResourceError::IdempotencyConflict(
                "completion grant identity was reused with changed targets or recipe".to_owned(),
            ));
        }
        if acquisition.grants.contains_key(&request.owner_plugin)
            || self
                .reserved_units
                .checked_add(units)
                .is_none_or(|value| value > budget.total_completion_units)
        {
            return Err(ResourceError::LimitExceeded(
                "completion capacity is unavailable".to_owned(),
            ));
        }
        let grant = CompletionCapacityGrantV1 {
            id: request.grant_id.clone(),
            revision: ResourceRevision::INITIAL,
            acquisition: acquisition.id.clone(),
            operation_key: acquisition.operation_key,
            owner_plugin: request.owner_plugin.clone(),
            run_budget_revision: budget.revision,
            target_versions: request.target_versions,
            recipe_digest,
            reserved_units: units,
            expires_after_boundary: request
                .current_boundary
                .checked_add(PREACTIVATION_LEASE_TTL_BOUNDARIES)
                .ok_or(ResourceError::Overflow)?,
            activation_deadline_boundary: None,
            state: CompletionGrantStateV1::Held,
            rejection: None,
        };
        self.reserved_units = self
            .reserved_units
            .checked_add(units)
            .ok_or(ResourceError::Overflow)?;
        self.grants.insert(grant.id.clone(), grant.clone());
        self.expiry_due
            .entry(grant.expires_after_boundary)
            .or_default()
            .insert(grant.id.clone());
        let acquisition = self
            .acquisitions
            .get_mut(&request.acquisition)
            .ok_or_else(|| {
                ResourceError::InvalidDefinition(
                    "completion acquisition disappeared while granting capacity".to_owned(),
                )
            })?;
        acquisition
            .grants
            .insert(request.owner_plugin, grant.id.clone());
        acquisition.state = if acquisition.grants.len() == acquisition.expected_participants.len() {
            CompletionLeaseAcquisitionStateV1::FullyGranted
        } else {
            CompletionLeaseAcquisitionStateV1::PartiallyGranted
        };
        acquisition.revision = acquisition.revision.next()?;
        Ok(grant)
    }

    pub fn prepare_capacity(
        &mut self,
        request: PrepareCompletionCapacityV1,
    ) -> Result<CompletionCapacityGrantV1, ResourceError> {
        let PrepareCompletionCapacityV1 {
            acquisition: acquisition_id,
            expected_acquisition_revision,
            grant: grant_id,
            expected_grant_revision,
            current_boundary,
            eligibility_envelope_digest,
        } = request;
        let acquisition = self
            .acquisitions
            .get(&acquisition_id)
            .cloned()
            .ok_or_else(|| {
                ResourceError::NotFound("completion acquisition is unavailable".to_owned())
            })?;
        if acquisition.revision != expected_acquisition_revision
            || !matches!(
                acquisition.state,
                CompletionLeaseAcquisitionStateV1::FullyGranted
                    | CompletionLeaseAcquisitionStateV1::Preparing
            )
        {
            return Err(ResourceError::VersionConflict(
                "completion prepare acquisition is stale".to_owned(),
            ));
        }
        if acquisition.eligibility_envelope.digest != eligibility_envelope_digest {
            return self.reject_preparation(
                &acquisition_id,
                &grant_id,
                expected_grant_revision,
                "eligibility_envelope_mismatch",
            );
        }
        let grant_snapshot =
            self.grants.get(&grant_id).cloned().ok_or_else(|| {
                ResourceError::NotFound("completion grant is unavailable".to_owned())
            })?;
        if grant_snapshot.revision != expected_grant_revision
            || grant_snapshot.acquisition != acquisition_id
            || grant_snapshot.state != CompletionGrantStateV1::Held
        {
            return Err(ResourceError::VersionConflict(
                "completion prepare grant is stale or not held".to_owned(),
            ));
        }
        if grant_snapshot.target_versions.iter().any(|target| {
            self.target_locks
                .get(target)
                .is_some_and(|owner| owner != &grant_id)
        }) {
            return self.reject_preparation(
                &acquisition_id,
                &grant_id,
                expected_grant_revision,
                "lease_prepared_conflict",
            );
        }
        if grant_snapshot.expires_after_boundary
            < current_boundary
                .checked_add(ACTIVATION_GUARD_BOUNDARIES)
                .ok_or(ResourceError::Overflow)?
        {
            return self.reject_preparation(
                &acquisition_id,
                &grant_id,
                expected_grant_revision,
                "activation_guard_insufficient",
            );
        }
        let grant = self
            .grants
            .get_mut(&grant_id)
            .ok_or_else(|| ResourceError::NotFound("completion grant is unavailable".to_owned()))?;
        if grant.revision != expected_grant_revision
            || grant.acquisition != acquisition_id
            || grant.state != CompletionGrantStateV1::Held
        {
            return Err(ResourceError::VersionConflict(
                "completion prepare grant is stale or not held".to_owned(),
            ));
        }
        grant.state = CompletionGrantStateV1::Prepared;
        grant.activation_deadline_boundary = Some(grant.expires_after_boundary);
        grant.revision = grant.revision.next()?;
        let result = grant.clone();
        for target in &result.target_versions {
            self.target_locks.insert(target.clone(), result.id.clone());
        }
        let acquisition = self.acquisitions.get_mut(&acquisition_id).ok_or_else(|| {
            ResourceError::InvalidDefinition(
                "completion acquisition disappeared while preparing capacity".to_owned(),
            )
        })?;
        acquisition.state = CompletionLeaseAcquisitionStateV1::Preparing;
        if acquisition.grants.values().all(|id| {
            self.grants
                .get(id)
                .is_some_and(|grant| grant.state == CompletionGrantStateV1::Prepared)
        }) {
            acquisition.state = CompletionLeaseAcquisitionStateV1::PreparedAll;
        }
        acquisition.revision = acquisition.revision.next()?;
        Ok(result)
    }

    fn reject_preparation(
        &mut self,
        acquisition_id: &CompletionLeaseAcquisitionId,
        grant_id: &CompletionCapacityGrantId,
        expected_grant_revision: ResourceRevision,
        reason: &str,
    ) -> Result<CompletionCapacityGrantV1, ResourceError> {
        let grant = self
            .grants
            .get_mut(grant_id)
            .ok_or_else(|| ResourceError::NotFound("completion grant is unavailable".to_owned()))?;
        if grant.revision != expected_grant_revision
            || grant.acquisition != *acquisition_id
            || grant.state != CompletionGrantStateV1::Held
        {
            return Err(ResourceError::VersionConflict(
                "completion prepare rejection names a stale grant".to_owned(),
            ));
        }
        grant.state = CompletionGrantStateV1::Rejected;
        grant.rejection = Some(reason.to_owned());
        grant.revision = grant.revision.next()?;
        self.reserved_units = self
            .reserved_units
            .checked_sub(grant.reserved_units)
            .ok_or_else(|| {
                ResourceError::Conservation(
                    "completion rejected grant reserved units underflowed".to_owned(),
                )
            })?;
        let result = grant.clone();
        for grants in self.expiry_due.values_mut() {
            grants.remove(grant_id);
        }
        self.expiry_due.retain(|_, grants| !grants.is_empty());
        let acquisition = self.acquisitions.get_mut(acquisition_id).ok_or_else(|| {
            ResourceError::InvalidDefinition(
                "completion acquisition disappeared while rejecting prepare".to_owned(),
            )
        })?;
        acquisition.state = CompletionLeaseAcquisitionStateV1::Aborting;
        acquisition.blocker = Some(reason.to_owned());
        acquisition.refunded_units = acquisition
            .refunded_units
            .checked_add(result.reserved_units)
            .ok_or(ResourceError::Overflow)?;
        acquisition.revision = acquisition.revision.next()?;
        Ok(result)
    }

    pub fn cleanup_aborting(
        &mut self,
        candidate_limit: usize,
    ) -> Result<Vec<CompletionLeaseAcquisitionId>, ResourceError> {
        if candidate_limit == 0 || candidate_limit > MAX_PENDING_LEASE_ACQUISITIONS_GLOBAL {
            return Err(ResourceError::LimitExceeded(
                "completion cleanup candidate limit is invalid".to_owned(),
            ));
        }
        let candidates = self
            .acquisitions
            .values()
            .filter(|value| value.state == CompletionLeaseAcquisitionStateV1::Aborting)
            .map(|value| value.id.clone())
            .take(candidate_limit.saturating_add(1))
            .collect::<Vec<_>>();
        if candidates.len() > candidate_limit {
            return Err(ResourceError::LimitExceeded(
                "completion cleanup due-work budget was exceeded".to_owned(),
            ));
        }
        for acquisition_id in &candidates {
            let grant_ids = self.acquisitions[acquisition_id]
                .grants
                .values()
                .cloned()
                .collect::<Vec<_>>();
            for grant_id in grant_ids {
                self.release_grant(&grant_id)?;
            }
            let acquisition = self.acquisitions.get_mut(acquisition_id).ok_or_else(|| {
                ResourceError::InvalidDefinition(
                    "completion acquisition disappeared during autonomous cleanup".to_owned(),
                )
            })?;
            acquisition.state = CompletionLeaseAcquisitionStateV1::Released;
            acquisition.revision = acquisition.revision.next()?;
        }
        Ok(candidates)
    }

    pub(crate) fn reject_prepare_exact_mismatch(
        &mut self,
        request: &PrepareCompletionCapacityV1,
    ) -> Result<CompletionCapacityGrantV1, ResourceError> {
        self.reject_preparation(
            &request.acquisition,
            &request.grant,
            request.expected_grant_revision,
            "prepare_exact_target_or_envelope_mismatch",
        )
    }

    pub fn status_for(
        &self,
        holder: &KnowledgeHolderRef,
        acquisition_id: &CompletionLeaseAcquisitionId,
    ) -> Result<CompletionLeaseStatusDtoV1, ResourceError> {
        let acquisition = self.acquisitions.get(acquisition_id).ok_or_else(|| {
            ResourceError::NotFound("completion lease acquisition is unavailable".to_owned())
        })?;
        if &acquisition.holder != holder {
            return Err(ResourceError::Authority(
                "completion lease status is holder-bound".to_owned(),
            ));
        }
        let mut grant_states = BTreeMap::new();
        let mut exact_grant_versions = BTreeMap::new();
        let mut expiry_boundaries = BTreeMap::new();
        let mut activation_deadlines = BTreeMap::new();
        for (participant, id) in &acquisition.grants {
            let grant = self.grants.get(id).ok_or_else(|| {
                ResourceError::InvalidDefinition("acquisition lost a participant grant".to_owned())
            })?;
            grant_states.insert(participant.clone(), grant.state);
            exact_grant_versions.insert(participant.clone(), grant.revision);
            expiry_boundaries.insert(participant.clone(), grant.expires_after_boundary);
            if let Some(deadline) = grant.activation_deadline_boundary {
                activation_deadlines.insert(participant.clone(), deadline);
            }
        }
        Ok(CompletionLeaseStatusDtoV1 {
            acquisition: acquisition.id.clone(),
            operation_key: acquisition.operation_key.clone(),
            state: acquisition.state,
            grant_states,
            exact_grant_versions,
            eligibility_time: acquisition.eligibility_time,
            expiry_boundaries,
            activation_deadlines,
            blocker: acquisition.blocker.clone(),
            refunded_units: acquisition.refunded_units,
            next_eligible_action: next_action(acquisition.state).to_owned(),
        })
    }

    pub fn abort(
        &mut self,
        holder: &KnowledgeHolderRef,
        acquisition_id: &CompletionLeaseAcquisitionId,
        expected_revision: ResourceRevision,
    ) -> Result<&'static str, ResourceError> {
        let acquisition = self.acquisitions.get_mut(acquisition_id).ok_or_else(|| {
            ResourceError::NotFound("completion lease acquisition is unavailable".to_owned())
        })?;
        if &acquisition.holder != holder {
            return Err(ResourceError::Authority(
                "only the initiating holder may abort a resource lease".to_owned(),
            ));
        }
        if acquisition.state == CompletionLeaseAcquisitionStateV1::Activated {
            return Ok("already_activated");
        }
        if matches!(
            acquisition.state,
            CompletionLeaseAcquisitionStateV1::Released
                | CompletionLeaseAcquisitionStateV1::Expired
        ) {
            return Ok("already_released");
        }
        if acquisition.revision != expected_revision {
            return Err(ResourceError::VersionConflict(
                "completion lease acquisition revision is stale".to_owned(),
            ));
        }
        acquisition.state = CompletionLeaseAcquisitionStateV1::Aborting;
        acquisition.revision = acquisition.revision.next()?;
        let grant_ids: Vec<_> = acquisition.grants.values().cloned().collect();
        let _ = acquisition;
        for grant_id in grant_ids {
            self.release_grant(&grant_id)?;
        }
        let acquisition = self.acquisitions.get_mut(acquisition_id).ok_or_else(|| {
            ResourceError::InvalidDefinition(
                "completion acquisition disappeared while releasing capacity".to_owned(),
            )
        })?;
        acquisition.state = CompletionLeaseAcquisitionStateV1::Released;
        acquisition.revision = acquisition.revision.next()?;
        Ok("released")
    }

    pub fn activate_single_owner(
        &mut self,
        acquisition_id: &CompletionLeaseAcquisitionId,
        grant_id: &CompletionCapacityGrantId,
        boundary: u64,
    ) -> Result<CompletionLeaseActivationCertificateV1, ResourceError> {
        let acquisition = self.acquisitions.get(acquisition_id).ok_or_else(|| {
            ResourceError::NotFound("completion lease acquisition is unavailable".to_owned())
        })?;
        let grant = self.grants.get(grant_id).ok_or_else(|| {
            ResourceError::NotFound("completion capacity grant is unavailable".to_owned())
        })?;
        self.activate_capacity(ActivateCompletionLeaseV1 {
            acquisition: acquisition_id.clone(),
            expected_acquisition_revision: acquisition.revision,
            grant: grant_id.clone(),
            expected_grant_revision: grant.revision,
            at: acquisition.eligibility_time,
            current_boundary: boundary,
            eligibility_envelope_digest: acquisition.eligibility_envelope.digest.clone(),
        })
    }

    pub fn activate_capacity(
        &mut self,
        request: ActivateCompletionLeaseV1,
    ) -> Result<CompletionLeaseActivationCertificateV1, ResourceError> {
        let acquisition_snapshot = self
            .acquisitions
            .get(&request.acquisition)
            .cloned()
            .ok_or_else(|| {
                ResourceError::NotFound("completion lease acquisition is unavailable".to_owned())
            })?;
        let grant = self.grants.get(&request.grant).cloned().ok_or_else(|| {
            ResourceError::NotFound("completion capacity grant is unavailable".to_owned())
        })?;
        if acquisition_snapshot.revision != request.expected_acquisition_revision
            || acquisition_snapshot.state != CompletionLeaseAcquisitionStateV1::PreparedAll
            || acquisition_snapshot.eligibility_time != request.at
            || acquisition_snapshot.eligibility_envelope.digest
                != request.eligibility_envelope_digest
            || grant.revision != request.expected_grant_revision
            || grant.acquisition != request.acquisition
            || grant.state != CompletionGrantStateV1::Prepared
        {
            return Err(ResourceError::VersionConflict(
                "completion activation exact acquisition, grant, time, or envelope differs"
                    .to_owned(),
            ));
        }
        let mut prepared_grants = Vec::new();
        let mut locked_target_versions = Vec::new();
        let mut earliest_deadline = u64::MAX;
        for grant_id in acquisition_snapshot.grants.values() {
            let prepared = self.grants.get(grant_id).ok_or_else(|| {
                ResourceError::InvalidDefinition(
                    "completion activation lost a participant grant".to_owned(),
                )
            })?;
            if prepared.state != CompletionGrantStateV1::Prepared {
                return Err(ResourceError::InvalidLifecycle(
                    "completion activation requires every participant prepared".to_owned(),
                ));
            }
            let deadline = prepared.activation_deadline_boundary.ok_or_else(|| {
                ResourceError::InvalidLifecycle(
                    "prepared completion grant has no activation deadline".to_owned(),
                )
            })?;
            earliest_deadline = earliest_deadline.min(deadline);
            prepared_grants.push((prepared.id.clone(), prepared.revision));
            locked_target_versions.extend(prepared.target_versions.iter().cloned());
        }
        if request.current_boundary >= earliest_deadline {
            return Err(ResourceError::InvalidLifecycle(
                "activation must occur strictly before every prepared deadline".to_owned(),
            ));
        }
        prepared_grants.sort();
        locked_target_versions.sort();
        locked_target_versions.dedup();
        let acquisition = self
            .acquisitions
            .get_mut(&request.acquisition)
            .ok_or_else(|| {
                ResourceError::InvalidDefinition(
                    "completion acquisition disappeared while activating".to_owned(),
                )
            })?;
        acquisition.state = CompletionLeaseAcquisitionStateV1::Activated;
        acquisition.revision = acquisition.revision.next()?;
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
        certificate.semantic_digest = canonical_digest(
            "canwu.resource.completion-activation-certificate.v1",
            &certificate,
        )?;
        self.certificates
            .insert(certificate.acquisition.clone(), certificate.clone());
        for (grant_id, _) in &certificate.prepared_grants {
            for grants in self.expiry_due.values_mut() {
                grants.remove(grant_id);
            }
        }
        self.expiry_due.retain(|_, grants| !grants.is_empty());
        Ok(certificate)
    }

    pub fn consume_grant(
        &mut self,
        certificate: &CompletionLeaseActivationCertificateV1,
        grant_id: &CompletionCapacityGrantId,
    ) -> Result<(), ResourceError> {
        self.consume_authoritative_grant(
            certificate,
            grant_id,
            certificate.eligibility_time,
            &certificate.locked_target_versions,
        )
    }

    pub fn consume_authoritative_grant(
        &mut self,
        certificate: &CompletionLeaseActivationCertificateV1,
        grant_id: &CompletionCapacityGrantId,
        at: SimTime,
        required_targets: &[CompletionLockedTargetV1],
    ) -> Result<(), ResourceError> {
        let mut detached = certificate.clone();
        let recorded = std::mem::take(&mut detached.semantic_digest);
        if recorded
            != canonical_digest(
                "canwu.resource.completion-activation-certificate.v1",
                &detached,
            )?
        {
            return Err(ResourceError::InvalidDefinition(
                "completion activation certificate digest is invalid".to_owned(),
            ));
        }
        if self.certificates.get(&certificate.acquisition) != Some(certificate)
            || certificate.eligibility_time != at
        {
            return Err(ResourceError::VersionConflict(
                "completion certificate is not the authoritative persisted same-time version"
                    .to_owned(),
            ));
        }
        let acquisition = self
            .acquisitions
            .get(&certificate.acquisition)
            .ok_or_else(|| {
                ResourceError::NotFound("completion acquisition is unavailable".to_owned())
            })?;
        if acquisition.state != CompletionLeaseAcquisitionStateV1::Activated
            || acquisition.revision != certificate.acquisition_revision
            || acquisition.eligibility_envelope.digest != certificate.eligibility_envelope_digest
        {
            return Err(ResourceError::VersionConflict(
                "completion certificate no longer binds the authoritative acquisition".to_owned(),
            ));
        }
        let mut required_targets = required_targets.to_vec();
        required_targets.sort();
        required_targets.dedup();
        let grant = self
            .grants
            .get_mut(grant_id)
            .ok_or_else(|| ResourceError::NotFound("completion grant is unavailable".to_owned()))?;
        if certificate.acquisition != grant.acquisition
            || certificate.operation_key != grant.operation_key
            || !certificate
                .prepared_grants
                .contains(&(grant.id.clone(), grant.revision))
            || grant.state != CompletionGrantStateV1::Prepared
            || grant.target_versions != required_targets
            || grant
                .target_versions
                .iter()
                .any(|target| self.target_locks.get(target) != Some(grant_id))
        {
            return Err(ResourceError::VersionConflict(
                "completion certificate does not bind the exact prepared grant".to_owned(),
            ));
        }
        grant.state = CompletionGrantStateV1::Consumed;
        grant.revision = grant.revision.next()?;
        Ok(())
    }

    pub fn complete_grant(
        &mut self,
        acquisition_id: &CompletionLeaseAcquisitionId,
        grant_id: &CompletionCapacityGrantId,
    ) -> Result<(), ResourceError> {
        let grant = self
            .grants
            .get(grant_id)
            .ok_or_else(|| ResourceError::NotFound("completion grant is unavailable".to_owned()))?;
        if grant.acquisition != *acquisition_id
            || !matches!(
                grant.state,
                CompletionGrantStateV1::Consumed | CompletionGrantStateV1::Completed
            )
        {
            return Err(ResourceError::InvalidLifecycle(
                "completion release requires the exact consumed grant".to_owned(),
            ));
        }
        let participant_ids = self
            .acquisitions
            .get(acquisition_id)
            .ok_or_else(|| {
                ResourceError::InvalidDefinition("completion grant is orphaned".to_owned())
            })?
            .grants
            .values()
            .cloned()
            .collect::<Vec<_>>();
        if participant_ids.iter().any(|id| {
            self.grants.get(id).is_none_or(|participant| {
                !matches!(
                    participant.state,
                    CompletionGrantStateV1::Consumed | CompletionGrantStateV1::Completed
                )
            })
        }) {
            return Err(ResourceError::InvalidLifecycle(
                "completion cannot finish until every participant grant is consumed".to_owned(),
            ));
        }
        for participant_id in participant_ids {
            let participant = self
                .grants
                .get_mut(&participant_id)
                .expect("participant grant was checked");
            if participant.state == CompletionGrantStateV1::Completed {
                continue;
            }
            participant.state = CompletionGrantStateV1::Completed;
            participant.revision = participant.revision.next()?;
            self.reserved_units = self
                .reserved_units
                .checked_sub(participant.reserved_units)
                .ok_or_else(|| {
                    ResourceError::Conservation("completion reserved units underflowed".to_owned())
                })?;
            for target in &participant.target_versions {
                self.target_locks.remove(target);
            }
        }
        let acquisition = self.acquisitions.get_mut(acquisition_id).ok_or_else(|| {
            ResourceError::InvalidDefinition("completion grant is orphaned".to_owned())
        })?;
        acquisition.state = CompletionLeaseAcquisitionStateV1::Released;
        acquisition.revision = acquisition.revision.next()?;
        Ok(())
    }

    pub fn release_capacity(
        &mut self,
        request: &ReleaseCompletionCapacityV1,
    ) -> Result<(), ResourceError> {
        let acquisition = self.acquisitions.get(&request.acquisition).ok_or_else(|| {
            ResourceError::NotFound("completion acquisition is unavailable".to_owned())
        })?;
        let grant = self
            .grants
            .get(&request.grant)
            .ok_or_else(|| ResourceError::NotFound("completion grant is unavailable".to_owned()))?;
        if acquisition.revision != request.expected_acquisition_revision
            || grant.revision != request.expected_grant_revision
            || grant.acquisition != request.acquisition
        {
            return Err(ResourceError::VersionConflict(
                "completion release exact acquisition or grant differs".to_owned(),
            ));
        }
        self.release_grant(&request.grant)?;
        let acquisition = self
            .acquisitions
            .get_mut(&request.acquisition)
            .ok_or_else(|| {
                ResourceError::InvalidDefinition(
                    "completion acquisition disappeared while releasing".to_owned(),
                )
            })?;
        if acquisition.grants.values().all(|grant_id| {
            self.grants.get(grant_id).is_some_and(|grant| {
                matches!(
                    grant.state,
                    CompletionGrantStateV1::Released
                        | CompletionGrantStateV1::Expired
                        | CompletionGrantStateV1::Completed
                        | CompletionGrantStateV1::Rejected
                )
            })
        }) {
            acquisition.state = CompletionLeaseAcquisitionStateV1::Released;
            acquisition.revision = acquisition.revision.next()?;
        }
        Ok(())
    }

    pub fn expire_capacity(
        &mut self,
        request: &ExpireCompletionCapacityV1,
    ) -> Result<Vec<CompletionCapacityGrantId>, ResourceError> {
        if request.candidate_limit == 0
            || request.candidate_limit > MAX_PENDING_LEASE_ACQUISITIONS_GLOBAL
        {
            return Err(ResourceError::LimitExceeded(
                "completion expiry candidate limit is invalid".to_owned(),
            ));
        }
        let candidates: Vec<_> = self
            .expiry_due
            .range(..=request.current_boundary)
            .flat_map(|(_, grants)| grants.iter().cloned())
            .take(request.candidate_limit.saturating_add(1))
            .collect();
        if candidates.len() > request.candidate_limit {
            return Err(ResourceError::LimitExceeded(
                "completion expiry due-work budget was exceeded".to_owned(),
            ));
        }
        let mut expired = Vec::new();
        for grant_id in candidates {
            let Some(grant) = self.grants.get(&grant_id).cloned() else {
                return Err(ResourceError::InvalidDefinition(
                    "completion expiry index contains an unavailable grant".to_owned(),
                ));
            };
            if !matches!(
                grant.state,
                CompletionGrantStateV1::Held | CompletionGrantStateV1::Prepared
            ) {
                continue;
            }
            let acquisition = self.acquisitions.get(&grant.acquisition).ok_or_else(|| {
                ResourceError::InvalidDefinition("completion expiry grant is orphaned".to_owned())
            })?;
            if request.at < acquisition.eligibility_time {
                return Err(ResourceError::VersionConflict(
                    "completion expiry cannot move before eligibility time".to_owned(),
                ));
            }
            self.release_grant(&grant_id)?;
            let grant = self.grants.get_mut(&grant_id).ok_or_else(|| {
                ResourceError::InvalidDefinition(
                    "completion grant disappeared during expiry".to_owned(),
                )
            })?;
            grant.state = CompletionGrantStateV1::Expired;
            let acquisition = self
                .acquisitions
                .get_mut(&grant.acquisition)
                .ok_or_else(|| {
                    ResourceError::InvalidDefinition(
                        "completion acquisition disappeared during expiry".to_owned(),
                    )
                })?;
            acquisition.state = CompletionLeaseAcquisitionStateV1::Expired;
            acquisition.blocker = Some("preactivation_expired".to_owned());
            acquisition.revision = acquisition.revision.next()?;
            expired.push(grant_id);
        }
        for grants in self.expiry_due.values_mut() {
            grants.retain(|grant_id| {
                self.grants.get(grant_id).is_some_and(|grant| {
                    matches!(
                        grant.state,
                        CompletionGrantStateV1::Held | CompletionGrantStateV1::Prepared
                    )
                })
            });
        }
        self.expiry_due.retain(|_, grants| !grants.is_empty());
        Ok(expired)
    }

    #[must_use]
    pub fn certificate(
        &self,
        acquisition: &CompletionLeaseAcquisitionId,
    ) -> Option<&CompletionLeaseActivationCertificateV1> {
        self.certificates.get(acquisition)
    }

    pub fn record_receipt(
        &mut self,
        operation_key: ResourceOperationKey,
        acquisition: CompletionLeaseAcquisitionId,
        grant: Option<CompletionCapacityGrantId>,
        action: CompletionLeaseReceiptActionV1,
        reason: Option<String>,
    ) -> Result<CompletionLeaseReceiptV1, ResourceError> {
        let state = self
            .acquisitions
            .get(&acquisition)
            .map_or(CompletionLeaseAcquisitionStateV1::Released, |value| {
                value.state
            });
        let refunded_units = self
            .acquisitions
            .get(&acquisition)
            .map_or(0, |value| value.refunded_units);
        let reserved_units = grant
            .as_ref()
            .and_then(|id| self.grants.get(id))
            .map_or(0, |value| value.reserved_units);
        let sequence = self.next_sequence()?;
        let mut receipt = CompletionLeaseReceiptV1 {
            sequence,
            operation_key,
            acquisition,
            grant,
            action,
            state,
            reserved_units,
            refunded_units,
            reason,
            semantic_digest: String::new(),
        };
        receipt.semantic_digest =
            canonical_digest("canwu.resource.completion-lease-receipt.v1", &receipt)?;
        self.receipts.insert(sequence, receipt.clone());
        Ok(receipt)
    }

    fn release_grant(&mut self, grant_id: &CompletionCapacityGrantId) -> Result<(), ResourceError> {
        let grant = self.grants.get_mut(grant_id).ok_or_else(|| {
            ResourceError::NotFound("completion capacity grant is unavailable".to_owned())
        })?;
        if matches!(
            grant.state,
            CompletionGrantStateV1::Released
                | CompletionGrantStateV1::Expired
                | CompletionGrantStateV1::Completed
        ) {
            return Ok(());
        }
        if grant.state == CompletionGrantStateV1::Rejected {
            grant.state = CompletionGrantStateV1::Released;
            grant.revision = grant.revision.next()?;
            return Ok(());
        }
        if grant.state == CompletionGrantStateV1::Consumed {
            return Err(ResourceError::InvalidLifecycle(
                "an activated consumed grant cannot be released".to_owned(),
            ));
        }
        grant.state = CompletionGrantStateV1::Released;
        grant.revision = grant.revision.next()?;
        self.reserved_units = self
            .reserved_units
            .checked_sub(grant.reserved_units)
            .ok_or_else(|| {
                ResourceError::Conservation("completion reserved units underflowed".to_owned())
            })?;
        for target in &grant.target_versions {
            self.target_locks.remove(target);
        }
        let acquisition = self
            .acquisitions
            .get_mut(&grant.acquisition)
            .ok_or_else(|| ResourceError::InvalidDefinition("grant is orphaned".to_owned()))?;
        acquisition.refunded_units = acquisition
            .refunded_units
            .checked_add(grant.reserved_units)
            .ok_or(ResourceError::Overflow)?;
        Ok(())
    }

    fn next_sequence(&mut self) -> Result<u64, ResourceError> {
        let current = self.next_sequence;
        self.next_sequence = current.checked_add(1).ok_or(ResourceError::Overflow)?;
        Ok(current)
    }

    pub fn validate(&self, budget: &RunBudgetRevisionV1) -> Result<(), ResourceError> {
        budget.validate()?;
        let mut computed_reserved = 0_u64;
        for (id, grant) in &self.grants {
            if id != &grant.id
                || !self.acquisitions.contains_key(&grant.acquisition)
                || grant.run_budget_revision != budget.revision
            {
                return Err(ResourceError::InvalidDefinition(
                    "completion grant identity, acquisition, or budget revision is invalid"
                        .to_owned(),
                ));
            }
            if matches!(
                grant.state,
                CompletionGrantStateV1::Held
                    | CompletionGrantStateV1::Prepared
                    | CompletionGrantStateV1::Consumed
            ) {
                computed_reserved = computed_reserved
                    .checked_add(grant.reserved_units)
                    .ok_or(ResourceError::Overflow)?;
            }
        }
        if computed_reserved != self.reserved_units
            || computed_reserved > budget.total_completion_units
        {
            return Err(ResourceError::Conservation(
                "completion lease reserved totals do not reconcile".to_owned(),
            ));
        }
        for (id, acquisition) in &self.acquisitions {
            acquisition.eligibility_envelope.validate()?;
            acquisition.recipe.validate()?;
            if id != &acquisition.id
                || acquisition.recipe_digest != acquisition.recipe.digest()?
                || acquisition.expected_participants.len() > MAX_GRANTS_PER_ACQUISITION
                || acquisition
                    .grants
                    .keys()
                    .any(|participant| !acquisition.expected_participants.contains(participant))
            {
                return Err(ResourceError::InvalidDefinition(
                    "completion acquisition closure is invalid".to_owned(),
                ));
            }
        }
        for (target, grant_id) in &self.target_locks {
            let grant = self.grants.get(grant_id).ok_or_else(|| {
                ResourceError::InvalidDefinition("completion target lock is orphaned".to_owned())
            })?;
            if !grant.target_versions.contains(target)
                || !matches!(
                    grant.state,
                    CompletionGrantStateV1::Prepared | CompletionGrantStateV1::Consumed
                )
            {
                return Err(ResourceError::InvalidDefinition(
                    "completion target lock does not bind a prepared or consumed grant".to_owned(),
                ));
            }
        }
        for grant in self.grants.values().filter(|grant| {
            matches!(
                grant.state,
                CompletionGrantStateV1::Prepared | CompletionGrantStateV1::Consumed
            )
        }) {
            if grant
                .target_versions
                .iter()
                .any(|target| self.target_locks.get(target) != Some(&grant.id))
            {
                return Err(ResourceError::InvalidDefinition(
                    "prepared completion grant is missing an exact target lock".to_owned(),
                ));
            }
        }
        let indexed_due: BTreeSet<_> = self
            .expiry_due
            .values()
            .flat_map(|grants| grants.iter().cloned())
            .collect();
        let expected_due: BTreeSet<_> = self
            .grants
            .values()
            .filter(|grant| {
                matches!(
                    grant.state,
                    CompletionGrantStateV1::Held | CompletionGrantStateV1::Prepared
                ) && self
                    .acquisitions
                    .get(&grant.acquisition)
                    .is_some_and(|acquisition| {
                        acquisition.state != CompletionLeaseAcquisitionStateV1::Activated
                    })
            })
            .map(|grant| grant.id.clone())
            .collect();
        if indexed_due != expected_due {
            return Err(ResourceError::InvalidDefinition(
                "completion expiry due index differs from hot grant state".to_owned(),
            ));
        }
        for (acquisition_id, certificate) in &self.certificates {
            let mut detached = certificate.clone();
            let digest = std::mem::take(&mut detached.semantic_digest);
            if acquisition_id != &certificate.acquisition
                || digest
                    != canonical_digest(
                        "canwu.resource.completion-activation-certificate.v1",
                        &detached,
                    )?
                || !self.acquisitions.contains_key(acquisition_id)
            {
                return Err(ResourceError::InvalidDefinition(
                    "completion activation certificate is forged or orphaned".to_owned(),
                ));
            }
        }
        for receipt in self.receipts.values() {
            let mut detached = receipt.clone();
            let digest = std::mem::take(&mut detached.semantic_digest);
            if digest != canonical_digest("canwu.resource.completion-lease-receipt.v1", &detached)?
            {
                return Err(ResourceError::InvalidDefinition(
                    "completion lease receipt semantic digest is forged".to_owned(),
                ));
            }
        }
        Ok(())
    }
}

// JSON object keys cannot represent either the holder/namespace tuple or the
// typed target enum. Persist these maps as sorted entry arrays while retaining
// BTreeMap lookup semantics in memory. The explicit duplicate check also keeps
// restore validation from silently accepting a forged last-write-wins entry.
mod completion_epoch_map {
    use super::*;

    pub fn serialize<S>(
        value: &BTreeMap<(KnowledgeHolderRef, String), AdmissionEpochV1>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        value.iter().collect::<Vec<_>>().serialize(serializer)
    }

    pub fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<BTreeMap<(KnowledgeHolderRef, String), AdmissionEpochV1>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let entries =
            Vec::<((KnowledgeHolderRef, String), AdmissionEpochV1)>::deserialize(deserializer)?;
        let mut result = BTreeMap::new();
        for (key, value) in entries {
            if result.insert(key, value).is_some() {
                return Err(<D::Error as serde::de::Error>::custom(
                    "duplicate completion admission epoch",
                ));
            }
        }
        Ok(result)
    }
}

mod completion_target_lock_map {
    use super::*;

    pub fn serialize<S>(
        value: &BTreeMap<CompletionLockedTargetV1, CompletionCapacityGrantId>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        value.iter().collect::<Vec<_>>().serialize(serializer)
    }

    pub fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<BTreeMap<CompletionLockedTargetV1, CompletionCapacityGrantId>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let entries = Vec::<(CompletionLockedTargetV1, CompletionCapacityGrantId)>::deserialize(
            deserializer,
        )?;
        let mut result = BTreeMap::new();
        for (key, value) in entries {
            if result.insert(key, value).is_some() {
                return Err(<D::Error as serde::de::Error>::custom(
                    "duplicate completion target lock",
                ));
            }
        }
        Ok(result)
    }
}

/// Deterministic deficit-round-robin key. Holding release behavior constant,
/// a continuously eligible guaranteed request is always ordered before burst
/// work and cannot be displaced by another authority's churn.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct CompletionFairnessKeyV1 {
    pub policy_class: CompletionPolicyClassV1,
    pub round: u64,
    pub authority: KnowledgeHolderRef,
    pub admitted_sequence: u64,
    pub operation_key: ResourceOperationKey,
}

#[must_use]
pub fn deterministic_completion_fairness_order(
    acquisitions: impl IntoIterator<Item = CompletionLeaseAcquisitionV1>,
) -> Vec<CompletionLeaseAcquisitionId> {
    let mut values: Vec<_> = acquisitions
        .into_iter()
        .filter(|value| value.state == CompletionLeaseAcquisitionStateV1::Requested)
        .map(|value| {
            (
                CompletionFairnessKeyV1 {
                    policy_class: value.policy_class,
                    round: value.fairness_round,
                    authority: value.holder,
                    admitted_sequence: value.admitted_sequence,
                    operation_key: value.operation_key,
                },
                value.id,
            )
        })
        .collect();
    values.sort_by(|left, right| left.0.cmp(&right.0));
    values.into_iter().map(|(_, id)| id).collect()
}

const fn next_action(state: CompletionLeaseAcquisitionStateV1) -> &'static str {
    match state {
        CompletionLeaseAcquisitionStateV1::Requested
        | CompletionLeaseAcquisitionStateV1::PartiallyGranted => "await_grants",
        CompletionLeaseAcquisitionStateV1::FullyGranted => "prepare",
        CompletionLeaseAcquisitionStateV1::Preparing => "await_prepare",
        CompletionLeaseAcquisitionStateV1::PreparedAll => "activate",
        CompletionLeaseAcquisitionStateV1::Activated => "consume_or_complete",
        CompletionLeaseAcquisitionStateV1::Aborting => "release",
        CompletionLeaseAcquisitionStateV1::Released
        | CompletionLeaseAcquisitionStateV1::Expired => "none",
    }
}

const fn is_pending(state: CompletionLeaseAcquisitionStateV1) -> bool {
    matches!(
        state,
        CompletionLeaseAcquisitionStateV1::Requested
            | CompletionLeaseAcquisitionStateV1::PartiallyGranted
            | CompletionLeaseAcquisitionStateV1::FullyGranted
            | CompletionLeaseAcquisitionStateV1::Preparing
            | CompletionLeaseAcquisitionStateV1::PreparedAll
            | CompletionLeaseAcquisitionStateV1::Aborting
    )
}
