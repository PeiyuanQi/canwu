use crate::{
    AllocationLegStatus, CompletionCapacityGrantId, CompletionGrantStateV1,
    CompletionLeaseAcquisitionId, CompletionLeaseActivationCertificateV1,
    CompletionLeaseReceiptActionV1, CompletionLockedTargetV1, ConservationTotalsV1,
    ConsumptionStatus, DemandStatus, FulfillmentStatus, PLUGIN_NAME, PartialFulfillmentPolicy,
    ProtectedFloorPolicyRevision, ProtectedFloorPolicyRevisionId, ReservationStatus,
    ResourceAccount, ResourceAccountId, ResourceAllocationLeg, ResourceAllocationLegId,
    ResourceAllocationLegVersionV1, ResourceConsumption, ResourceConsumptionId,
    ResourceConsumptionVersionV1, ResourceDefinitionRevision, ResourceDefinitionRevisionId,
    ResourceDemand, ResourceDemandId, ResourceError, ResourceFulfillment, ResourceFulfillmentId,
    ResourceLimitsV1, ResourceLoss, ResourceLossId, ResourceOperationKey, ResourceOperationKind,
    ResourceOperationOutcome, ResourceOperationOutcomeId, ResourceOperationStatus,
    ResourceRecordRefV1, ResourceReportGrantV1, ResourceReservation, ResourceReservationId,
    ResourceRevision, ResourceState, ResourceTransfer, ResourceTransferId, ResourceTransferState,
    ResourceUnitRevision, ResourceUnitRevisionId, TransportExecutionLink, canonical_digest,
};
use canwu_api::{DomainRecordVersionRef, EvidenceRef, SimTime};
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

pub const RESOURCE_STATE_FORMAT_VERSION: u32 = 2;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceAllocationRequestV1 {
    pub operation_key: ResourceOperationKey,
    pub expected_state_revision: ResourceRevision,
    pub at: SimTime,
    pub candidate_limit: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceCreateAccountRequestV1 {
    pub operation_key: ResourceOperationKey,
    pub account: ResourceAccount,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceSubmitDemandRequestV1 {
    pub operation_key: ResourceOperationKey,
    pub demand: ResourceDemand,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceAmendDemandRequestV1 {
    pub operation_key: ResourceOperationKey,
    pub expected_demand_revision: ResourceRevision,
    pub replacement: ResourceDemand,
}

/// Exact input-allocation consumption request used by production and other
/// independent consumers.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceConsumptionRequestV1 {
    pub operation_key: ResourceOperationKey,
    pub consumption_id: ResourceConsumptionId,
    pub allocation: ResourceAllocationLegVersionV1,
    pub expected_account_revision: ResourceRevision,
    pub consumer_evidence: DomainRecordVersionRef,
    pub at: SimTime,
    pub completion_certificate: CompletionLeaseActivationCertificateV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceTransferStartRequestV1 {
    pub operation_key: ResourceOperationKey,
    pub transfer_id: ResourceTransferId,
    pub allocation: ResourceAllocationLegVersionV1,
    pub expected_account_revision: ResourceRevision,
    pub destination: Option<ResourceAccountId>,
    pub at: SimTime,
    pub completion_certificate: CompletionLeaseActivationCertificateV1,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferProgressV1 {
    InTransit,
    ArrivalPending,
    ReturnPending,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceTransferProgressRequestV1 {
    pub operation_key: ResourceOperationKey,
    pub transfer: ResourceTransferId,
    pub expected_transfer_revision: ResourceRevision,
    pub progress: TransferProgressV1,
    pub transport: TransportExecutionLink,
    pub transport_evidence: DomainRecordVersionRef,
}

/// Holder-authorized cancellation of an already-debited transfer. Cancellation
/// never destroys escrow: it deterministically moves the transfer onto the
/// reserved return path, which must later settle as returned or lost.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceTransferCancellationRequestV1 {
    pub operation_key: ResourceOperationKey,
    pub transfer: ResourceTransferId,
    pub expected_transfer_revision: ResourceRevision,
    pub at: SimTime,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "disposition", rename_all = "snake_case")]
pub enum ResourceTransferDispositionV1 {
    Accept {
        destination: ResourceAccountId,
        expected_destination_revision: ResourceRevision,
        acceptance: ResourceTransportAcceptanceV1,
    },
    Lose {
        loss_id: ResourceLossId,
        cause: EvidenceRef,
    },
    Return {
        expected_source_revision: ResourceRevision,
    },
    ExternalOutflow {
        authority_evidence: DomainRecordVersionRef,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceTransportAcceptanceV1 {
    pub evidence: DomainRecordVersionRef,
    pub execution: TransportExecutionLink,
    pub destination: ResourceAccountId,
    pub quantity: u64,
    pub accepted_at: SimTime,
    pub semantic_digest: String,
}

impl ResourceTransportAcceptanceV1 {
    pub fn seal(mut self) -> Result<Self, ResourceError> {
        self.semantic_digest.clear();
        self.semantic_digest = canonical_digest("canwu.resource.transport-acceptance.v1", &self)?;
        Ok(self)
    }

    pub fn validate(&self) -> Result<(), ResourceError> {
        let mut detached = self.clone();
        detached.semantic_digest.clear();
        if self.semantic_digest
            != canonical_digest("canwu.resource.transport-acceptance.v1", &detached)?
        {
            return Err(ResourceError::InvalidDefinition(
                "resource transport acceptance semantic digest is forged".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceTransferDispositionRequestV1 {
    pub operation_key: ResourceOperationKey,
    pub transfer: ResourceTransferId,
    pub expected_transfer_revision: ResourceRevision,
    pub at: SimTime,
    pub disposition: ResourceTransferDispositionV1,
    pub exact_transport_evidence: Option<DomainRecordVersionRef>,
    pub completion_certificate: CompletionLeaseActivationCertificateV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "source", content = "evidence", rename_all = "snake_case")]
pub enum ResourceCreditSourceV1 {
    Production(DomainRecordVersionRef),
    ExternalInflow(EvidenceRef),
}

/// Production-output settlement request. The source record body remains
/// payload-required until the returned exact outcome is acknowledged.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceCreditRequestV1 {
    pub operation_key: ResourceOperationKey,
    pub account: ResourceAccountId,
    pub expected_account_revision: ResourceRevision,
    pub resource_revision: ResourceDefinitionRevisionId,
    pub unit_revision: ResourceUnitRevisionId,
    pub quantity: u64,
    pub source: ResourceCreditSourceV1,
    pub at: SimTime,
    pub completion_certificate: CompletionLeaseActivationCertificateV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceExternalOutflowRequestV1 {
    pub operation_key: ResourceOperationKey,
    pub account: ResourceAccountId,
    pub expected_account_revision: ResourceRevision,
    pub quantity: u64,
    pub allow_protected: bool,
    pub authority_evidence: DomainRecordVersionRef,
    pub at: SimTime,
    pub completion_certificate: CompletionLeaseActivationCertificateV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceProtectedFloorRequestV1 {
    pub operation_key: ResourceOperationKey,
    pub account: ResourceAccountId,
    pub expected_account_revision: ResourceRevision,
    pub policy: Option<ProtectedFloorPolicyRevisionId>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceCancelDemandRequestV1 {
    pub operation_key: ResourceOperationKey,
    pub demand: ResourceDemandId,
    pub expected_demand_revision: ResourceRevision,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceObservationRequestV1 {
    pub operation_key: ResourceOperationKey,
    pub head: crate::ResourceObservationHeadV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "completion", content = "request", rename_all = "snake_case")]
pub enum ResourceCompletionOperationV1 {
    Acquire(crate::RequestCompletionLeaseV1),
    Grant(crate::GrantCompletionCapacityV1),
    Prepare(crate::PrepareCompletionCapacityV1),
    Activate(crate::ActivateCompletionLeaseV1),
    Abort(crate::AbortCompletionLeaseV1),
    Expire(crate::ExpireCompletionCapacityV1),
    Release(crate::ReleaseCompletionCapacityV1),
    GrantExternalParticipant(crate::RequestExternalCompletionParticipantGrantV1),
    PrepareExternalParticipant(crate::PrepareExternalCompletionParticipantGrantV1),
    ConsumeExternalParticipant(crate::ConsumeExternalCompletionParticipantGrantV1),
    CompleteExternalParticipant(crate::CompleteExternalCompletionParticipantGrantV1),
    ReleaseExternalParticipant(crate::ReleaseExternalCompletionParticipantGrantV1),
    ExpireExternalParticipants(crate::ExpireExternalCompletionParticipantGrantsV1),
}

impl ResourceCompletionOperationV1 {
    #[must_use]
    pub fn operation_key(&self) -> ResourceOperationKey {
        match self {
            Self::Acquire(value) => ResourceOperationKey::new(format!(
                "resource:completion:acquire:{}",
                value.id.as_str()
            ))
            .expect("completion ID always produces a valid namespaced operation key"),
            Self::Grant(value) => ResourceOperationKey::new(format!(
                "resource:completion:grant:{}",
                value.grant_id.as_str()
            ))
            .expect("grant ID always produces a valid namespaced operation key"),
            Self::Prepare(value) => ResourceOperationKey::new(format!(
                "resource:completion:prepare:{}:{}",
                value.acquisition.as_str(),
                value.grant.as_str()
            ))
            .expect("completion IDs always produce a valid namespaced operation key"),
            Self::Activate(value) => ResourceOperationKey::new(format!(
                "resource:completion:activate:{}",
                value.acquisition.as_str()
            ))
            .expect("completion ID always produces a valid namespaced operation key"),
            Self::Abort(value) => ResourceOperationKey::new(format!(
                "resource:completion:abort:{}",
                value.acquisition.as_str()
            ))
            .expect("completion ID always produces a valid namespaced operation key"),
            Self::Expire(value) => ResourceOperationKey::new(format!(
                "resource:completion:expire:{}:{}",
                value.at.as_minutes(),
                value.current_boundary
            ))
            .expect("completion expiry always produces a valid namespaced operation key"),
            Self::Release(value) => ResourceOperationKey::new(format!(
                "resource:completion:release:{}:{}",
                value.acquisition.as_str(),
                value.grant.as_str()
            ))
            .expect("completion IDs always produce a valid namespaced operation key"),
            Self::GrantExternalParticipant(value) => ResourceOperationKey::new(format!(
                "resource:completion:participant-grant:{}",
                value.acquisition.as_str()
            ))
            .expect("completion ID always produces a valid namespaced operation key"),
            Self::PrepareExternalParticipant(value) => ResourceOperationKey::new(format!(
                "resource:completion:participant-prepare:{}",
                value.acquisition.as_str()
            ))
            .expect("completion ID always produces a valid namespaced operation key"),
            Self::ConsumeExternalParticipant(value) => ResourceOperationKey::new(format!(
                "resource:completion:participant-consume:{}",
                value.certificate.acquisition.as_str()
            ))
            .expect("completion ID always produces a valid namespaced operation key"),
            Self::CompleteExternalParticipant(value) => ResourceOperationKey::new(format!(
                "resource:completion:participant-complete:{}",
                value.acquisition.as_str()
            ))
            .expect("completion ID always produces a valid namespaced operation key"),
            Self::ReleaseExternalParticipant(value) => ResourceOperationKey::new(format!(
                "resource:completion:participant-release:{}",
                value.acquisition.as_str()
            ))
            .expect("completion ID always produces a valid namespaced operation key"),
            Self::ExpireExternalParticipants(value) => ResourceOperationKey::new(format!(
                "resource:completion:participant-expire:{}:{}",
                value.at.as_minutes(),
                value.current_boundary
            ))
            .expect("completion expiry always produces a valid namespaced operation key"),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "operation", content = "request", rename_all = "snake_case")]
pub enum ResourceOperationRequestV1 {
    CreateAccount(ResourceCreateAccountRequestV1),
    SubmitDemand(ResourceSubmitDemandRequestV1),
    AmendDemand(ResourceAmendDemandRequestV1),
    Allocate(ResourceAllocationRequestV1),
    Consume(ResourceConsumptionRequestV1),
    BeginTransfer(ResourceTransferStartRequestV1),
    AdvanceTransfer(ResourceTransferProgressRequestV1),
    CancelTransfer(ResourceTransferCancellationRequestV1),
    CompleteTransfer(ResourceTransferDispositionRequestV1),
    Credit(ResourceCreditRequestV1),
    ExternalOutflow(ResourceExternalOutflowRequestV1),
    SetProtectedFloor(ResourceProtectedFloorRequestV1),
    CancelDemand(ResourceCancelDemandRequestV1),
    RecordObservation(ResourceObservationRequestV1),
    Completion(ResourceCompletionOperationV1),
}

impl ResourceOperationRequestV1 {
    #[must_use]
    pub fn operation_key(&self) -> ResourceOperationKey {
        match self {
            Self::CreateAccount(value) => value.operation_key.clone(),
            Self::SubmitDemand(value) => value.operation_key.clone(),
            Self::AmendDemand(value) => value.operation_key.clone(),
            Self::Allocate(value) => value.operation_key.clone(),
            Self::Consume(value) => value.operation_key.clone(),
            Self::BeginTransfer(value) => value.operation_key.clone(),
            Self::AdvanceTransfer(value) => value.operation_key.clone(),
            Self::CancelTransfer(value) => value.operation_key.clone(),
            Self::CompleteTransfer(value) => value.operation_key.clone(),
            Self::Credit(value) => value.operation_key.clone(),
            Self::ExternalOutflow(value) => value.operation_key.clone(),
            Self::SetProtectedFloor(value) => value.operation_key.clone(),
            Self::CancelDemand(value) => value.operation_key.clone(),
            Self::RecordObservation(value) => value.operation_key.clone(),
            Self::Completion(value) => value.operation_key(),
        }
    }

    #[must_use]
    pub const fn kind(&self) -> ResourceOperationKind {
        match self {
            Self::CreateAccount(_) => ResourceOperationKind::CreateAccount,
            Self::SubmitDemand(_) => ResourceOperationKind::SubmitDemand,
            Self::AmendDemand(_) => ResourceOperationKind::AmendDemand,
            Self::Allocate(_) => ResourceOperationKind::Allocate,
            Self::Consume(_) => ResourceOperationKind::Consume,
            Self::BeginTransfer(_) => ResourceOperationKind::BeginTransfer,
            Self::AdvanceTransfer(_) => ResourceOperationKind::AdvanceTransfer,
            Self::CancelTransfer(_) => ResourceOperationKind::CancelTransfer,
            Self::CompleteTransfer(_) => ResourceOperationKind::CompleteTransfer,
            Self::Credit(_) => ResourceOperationKind::Credit,
            Self::ExternalOutflow(_) => ResourceOperationKind::ExternalOutflow,
            Self::SetProtectedFloor(_) => ResourceOperationKind::SetProtectedFloor,
            Self::CancelDemand(_) => ResourceOperationKind::CancelDemand,
            Self::RecordObservation(_) => ResourceOperationKind::Observation,
            Self::Completion(_) => ResourceOperationKind::CompletionLease,
        }
    }
}

struct AppliedOperation {
    quantity: u64,
    remainder: u64,
    result_ref: Option<ResourceRecordRefV1>,
    exact_evidence: Vec<DomainRecordVersionRef>,
}

impl ResourceState {
    pub fn empty(limits: ResourceLimitsV1) -> Result<Self, ResourceError> {
        limits.validate()?;
        let run_budget = crate::RunBudgetRevisionV1 {
            revision: ResourceRevision::INITIAL,
            total_completion_units: 1,
            shared_pending_slots: 0,
            partitions: Vec::new(),
            semantic_digest: String::new(),
        }
        .seal()?;
        let mut archive_head = crate::ResourceArchiveHeadStateV1::default();
        archive_head.semantic_digest =
            canonical_digest("canwu.resource.archive-head.v1", &archive_head)?;
        Ok(Self {
            format_version: RESOURCE_STATE_FORMAT_VERSION,
            state_revision: ResourceRevision::INITIAL,
            continuation: canwu_api::PayloadRequiredEvidenceContinuationV1::completed(),
            limits,
            definitions: BTreeMap::new(),
            units: BTreeMap::new(),
            protected_floor_policies: BTreeMap::new(),
            accounts: BTreeMap::new(),
            demands: BTreeMap::new(),
            reservations: BTreeMap::new(),
            allocation_legs: BTreeMap::new(),
            transfers: BTreeMap::new(),
            consumptions: BTreeMap::new(),
            losses: BTreeMap::new(),
            fulfillments: BTreeMap::new(),
            outcomes: BTreeMap::new(),
            report_grants: BTreeMap::new(),
            observation_heads: BTreeMap::new(),
            observation_head_by_grant: BTreeMap::new(),
            report_dirty_grants: BTreeSet::new(),
            report_due_index: BTreeMap::new(),
            report_cursor: None,
            run_budget,
            completion_leases: crate::CompletionLeaseBookV1::default(),
            external_completion_participants: crate::ExternalCompletionParticipantBookV1::default(),
            completion_report_reservations: BTreeMap::new(),
            completion_report_ready: BTreeMap::new(),
            demand_due_index: BTreeMap::new(),
            demand_expiry_index: BTreeMap::new(),
            reservation_by_demand: BTreeMap::new(),
            dirty_demands: BTreeSet::new(),
            active_transfers: BTreeSet::new(),
            terminal_archive_candidates: BTreeMap::new(),
            archive_head,
            archive_retention_handles: BTreeMap::new(),
            archive_maintenance_receipts: BTreeMap::new(),
            conservation: ConservationTotalsV1::default(),
            next_admitted_sequence: 1,
        })
    }

    pub fn install_run_budget(
        &mut self,
        budget: crate::RunBudgetRevisionV1,
    ) -> Result<(), ResourceError> {
        budget.validate()?;
        if !self.completion_leases.acquisitions.is_empty()
            || !self.external_completion_participants.grants.is_empty()
        {
            return Err(ResourceError::InvalidLifecycle(
                "resource run budget cannot change after lease admission".to_owned(),
            ));
        }
        self.run_budget = budget;
        Ok(())
    }

    pub fn install_definition(
        &mut self,
        definition: ResourceDefinitionRevision,
    ) -> Result<(), ResourceError> {
        if self.definitions.len() >= self.limits.max_definitions
            || self.definitions.contains_key(&definition.id)
        {
            return Err(ResourceError::LimitExceeded(
                "resource definition capacity or identity is unavailable".to_owned(),
            ));
        }
        if definition
            .effective_until
            .is_some_and(|until| until <= definition.effective_from)
            || definition.semantic_digest.len() != 64
        {
            return Err(ResourceError::InvalidDefinition(
                "resource definition has an invalid interval or semantic digest".to_owned(),
            ));
        }
        self.definitions.insert(definition.id.clone(), definition);
        Ok(())
    }

    pub fn install_unit(&mut self, unit: ResourceUnitRevision) -> Result<(), ResourceError> {
        if self.units.len() >= self.limits.max_unit_revisions
            || self.units.contains_key(&unit.id)
            || unit.scale_numerator == 0
            || unit.scale_denominator == 0
            || unit.semantic_digest.len() != 64
        {
            return Err(ResourceError::InvalidDefinition(
                "resource unit is invalid or capacity is exhausted".to_owned(),
            ));
        }
        self.units.insert(unit.id.clone(), unit);
        Ok(())
    }

    pub fn install_protected_floor_policy(
        &mut self,
        policy: ProtectedFloorPolicyRevision,
    ) -> Result<(), ResourceError> {
        if policy.semantic_digest.len() != 64
            || self.protected_floor_policies.contains_key(&policy.id)
        {
            return Err(ResourceError::InvalidDefinition(
                "protected-floor policy is invalid or duplicated".to_owned(),
            ));
        }
        self.protected_floor_policies
            .insert(policy.id.clone(), policy);
        Ok(())
    }

    /// Installs scenario opening state. Runtime account creation must use zero.
    pub fn install_opening_account(
        &mut self,
        account: ResourceAccount,
    ) -> Result<(), ResourceError> {
        if self.accounts.len() >= self.limits.max_accounts
            || self.accounts.contains_key(&account.id)
        {
            return Err(ResourceError::LimitExceeded(
                "resource account capacity or identity is unavailable".to_owned(),
            ));
        }
        self.validate_account_contract(&account)?;
        self.conservation.opening_balances = self
            .conservation
            .opening_balances
            .checked_add(u128::from(account.balance))
            .ok_or(ResourceError::Overflow)?;
        self.accounts.insert(account.id.clone(), account);
        Ok(())
    }

    pub fn install_demand(&mut self, mut demand: ResourceDemand) -> Result<(), ResourceError> {
        let active_demands = self
            .demands
            .values()
            .filter(|candidate| {
                matches!(
                    candidate.status,
                    DemandStatus::Open | DemandStatus::PartiallyFulfilled
                )
            })
            .count();
        if active_demands >= self.limits.max_demands || self.demands.contains_key(&demand.id) {
            return Err(ResourceError::LimitExceeded(
                "resource demand capacity or identity is unavailable".to_owned(),
            ));
        }
        self.validate_demand_contract(&demand)?;
        if demand.status != DemandStatus::Open || demand.fulfilled != 0 {
            return Err(ResourceError::InvalidLifecycle(
                "installed resource demand must begin open and unfulfilled".to_owned(),
            ));
        }
        let indexed_due = self
            .demand_due_index
            .values()
            .map(BTreeSet::len)
            .sum::<usize>()
            .checked_add(
                self.demand_expiry_index
                    .values()
                    .map(BTreeSet::len)
                    .sum::<usize>(),
            )
            .and_then(|value| value.checked_add(2))
            .ok_or(ResourceError::Overflow)?;
        if indexed_due > self.limits.max_due_entries
            || self.dirty_demands.len() >= self.limits.max_dirty_demands
        {
            return Err(ResourceError::LimitExceeded(
                "resource demand due or dirty index capacity is exhausted".to_owned(),
            ));
        }
        demand.admitted_sequence = self.next_sequence()?;
        self.demand_due_index
            .entry(demand.due_at)
            .or_default()
            .insert(demand.id.clone());
        self.demand_expiry_index
            .entry(demand.expires_at)
            .or_default()
            .insert(demand.id.clone());
        self.dirty_demands.insert(demand.id.clone());
        self.demands.insert(demand.id.clone(), demand);
        Ok(())
    }

    pub fn install_report_grant(
        &mut self,
        grant: ResourceReportGrantV1,
    ) -> Result<(), ResourceError> {
        if grant.confidence_per_mille > 1_000
            || grant.cadence_minutes == 0
            || self.report_grants.contains_key(&grant.id)
            || self.report_grants.len() >= ResourceLimitsV1::MAX_HOLDERS
            || grant
                .accounts
                .iter()
                .any(|id| !self.accounts.contains_key(id))
        {
            return Err(ResourceError::InvalidDefinition(
                "resource report grant is invalid".to_owned(),
            ));
        }
        self.report_dirty_grants.insert(grant.id.clone());
        self.report_grants.insert(grant.id.clone(), grant);
        Ok(())
    }

    fn reserve_completion_reports(
        &mut self,
        acquisition: &CompletionLeaseAcquisitionId,
        holder: &canwu_api::KnowledgeHolderRef,
        recipe: &crate::CompletionCapacityRecipeV1,
    ) -> Result<(), ResourceError> {
        let requested = usize::from(recipe.reports_per_holder)
            .checked_mul(usize::from(recipe.holders))
            .ok_or(ResourceError::Overflow)?;
        if requested == 0 {
            return Ok(());
        }
        let grants = self
            .report_grants
            .values()
            .filter(|grant| &grant.holder == holder)
            .map(|grant| grant.id.clone())
            .take(requested)
            .collect::<BTreeSet<_>>();
        if grants.len() != requested {
            return Err(ResourceError::LimitExceeded(
                "completion recipe has no exact named holder report grants".to_owned(),
            ));
        }
        match self.completion_report_reservations.get(acquisition) {
            Some(existing) if existing == &grants => Ok(()),
            Some(_) => Err(ResourceError::IdempotencyConflict(
                "completion acquisition changed its named report reservation".to_owned(),
            )),
            None => {
                self.completion_report_reservations
                    .insert(acquisition.clone(), grants);
                Ok(())
            }
        }
    }

    pub(crate) fn mark_external_participant_archive_ready(
        &mut self,
        acquisition: &CompletionLeaseAcquisitionId,
    ) -> Result<(), ResourceError> {
        let key =
            crate::ResourceTerminalRecordKeyV1::ExternalCompletionParticipant(acquisition.clone());
        if self
            .completion_report_reservations
            .contains_key(acquisition)
            || !self
                .external_completion_participants
                .terminal_grants
                .contains_key(acquisition)
            || self
                .terminal_archive_candidates
                .values()
                .any(|candidate| candidate == &key)
        {
            return Ok(());
        }
        let terminal_sequence = self.next_sequence()?;
        self.terminal_archive_candidates
            .insert(terminal_sequence, key);
        Ok(())
    }

    fn mark_terminal_demand_closure(
        &mut self,
        demand_id: &ResourceDemandId,
    ) -> Result<(), ResourceError> {
        let demand = self.demands.get(demand_id).ok_or_else(|| {
            ResourceError::NotFound("terminal resource demand is unavailable".to_owned())
        })?;
        if matches!(
            demand.status,
            DemandStatus::Open | DemandStatus::PartiallyFulfilled
        ) {
            return Ok(());
        }
        let reservation_ids = self
            .reservation_by_demand
            .get(demand_id)
            .cloned()
            .unwrap_or_default();
        let mut keys =
            Vec::with_capacity(reservation_ids.len().saturating_mul(2).saturating_add(1));
        for reservation_id in reservation_ids {
            let reservation = self.reservations.get(&reservation_id).ok_or_else(|| {
                ResourceError::InvalidDefinition(
                    "terminal resource demand lost its reservation".to_owned(),
                )
            })?;
            let leg = self
                .allocation_legs
                .get(&reservation.allocation_leg)
                .ok_or_else(|| {
                    ResourceError::InvalidDefinition(
                        "terminal resource reservation lost its allocation leg".to_owned(),
                    )
                })?;
            if reservation.status == ReservationStatus::Active
                || leg.status == AllocationLegStatus::Reserved
            {
                return Ok(());
            }
            keys.push(crate::ResourceTerminalRecordKeyV1::AllocationLeg(
                leg.id.clone(),
            ));
            keys.push(crate::ResourceTerminalRecordKeyV1::Reservation(
                reservation.id.clone(),
            ));
        }
        keys.push(crate::ResourceTerminalRecordKeyV1::Demand(
            demand_id.clone(),
        ));
        keys.retain(|key| {
            !self
                .terminal_archive_candidates
                .values()
                .any(|candidate| candidate == key)
        });
        let projected = self
            .terminal_archive_candidates
            .len()
            .checked_add(keys.len())
            .ok_or(ResourceError::Overflow)?;
        if projected > self.limits.max_archive_candidates {
            return Err(ResourceError::LimitExceeded(
                "resource terminal demand closure requires archive progress".to_owned(),
            ));
        }
        for key in keys {
            let sequence = self.next_sequence()?;
            self.terminal_archive_candidates.insert(sequence, key);
        }
        Ok(())
    }

    pub fn record_observation_head(
        &mut self,
        mut head: crate::ResourceObservationHeadV1,
    ) -> Result<(), ResourceError> {
        head.validate()?;
        let grant = self.report_grants.get(&head.grant).ok_or_else(|| {
            ResourceError::NotFound("resource observation grant is unavailable".to_owned())
        })?;
        if head.holder != grant.holder
            || head.provider_plugin.is_empty()
            || head.provider_version.is_empty()
            || head.provider_semantic_hash.len() != 64
            || !head.source_versions.contains(&head.provider_source)
            || self.observation_heads.len() >= self.limits.max_observation_heads
                && !self.observation_heads.contains_key(&head.id)
        {
            return Err(ResourceError::InvalidDefinition(
                "resource observation head holder, source, revision, or capacity is invalid"
                    .to_owned(),
            ));
        }
        for stock in &head.stock {
            if !grant.accounts.contains(&stock.account) {
                return Err(ResourceError::Authority(
                    "resource observation stock exceeds its holder grant".to_owned(),
                ));
            }
            let account = self.accounts.get(&stock.account).ok_or_else(|| {
                ResourceError::NotFound("resource observation account is unavailable".to_owned())
            })?;
            let definition = self
                .definitions
                .get(&account.resource_revision)
                .ok_or_else(|| {
                    ResourceError::InvalidDefinition(
                        "resource observation account lost its exact definition".to_owned(),
                    )
                })?;
            if stock.scope != definition.scope {
                return Err(ResourceError::Authority(
                    "resource observation account scope differs from its authoritative exact definition"
                        .to_owned(),
                ));
            }
        }
        self.validate_observation_head_authority(&head, grant)?;
        if let Some(previous_id) = self.observation_head_by_grant.get(&head.grant) {
            let previous = self.observation_heads.get(previous_id).ok_or_else(|| {
                ResourceError::InvalidDefinition(
                    "resource observation grant index is broken".to_owned(),
                )
            })?;
            if head.id != previous.id
                || head.revision != previous.revision.next()?
                || head.observed_at < previous.observed_at
                || head.provider_state_revision < previous.provider_state_revision
            {
                return Err(ResourceError::VersionConflict(
                    "resource observation head does not advance its exact persisted cut".to_owned(),
                ));
            }
            self.observation_heads.remove(previous_id);
        } else if head.revision != ResourceRevision::INITIAL {
            return Err(ResourceError::VersionConflict(
                "initial resource observation head must use revision one".to_owned(),
            ));
        }
        head = head.seal()?;
        self.observation_head_by_grant
            .insert(head.grant.clone(), head.id.clone());
        for grants in self.report_due_index.values_mut() {
            grants.remove(&head.grant);
        }
        self.report_due_index.retain(|_, grants| !grants.is_empty());
        self.report_dirty_grants.insert(head.grant.clone());
        self.observation_heads.insert(head.id.clone(), head);
        Ok(())
    }

    pub(crate) fn validate_observation_head_authority(
        &self,
        head: &crate::ResourceObservationHeadV1,
        grant: &ResourceReportGrantV1,
    ) -> Result<(), ResourceError> {
        if head
            .demands
            .iter()
            .any(|value| !grant.demands.contains(&value.demand))
        {
            return Err(ResourceError::Authority(
                "resource observation demand exceeds its holder grant".to_owned(),
            ));
        }
        // Exact allocation, fulfillment, transfer, and consumption values are
        // checked when the provider records this cut.  Once the authoritative
        // resource revision advances, the sealed head intentionally remains a
        // historical observation and must not be compared to newer live state.
        if head.provider_state_revision != self.state_revision {
            return Ok(());
        }
        for observation in &head.allocations {
            let Some(leg) = self.allocation_legs.get(&observation.allocation) else {
                continue;
            };
            if observation.exact != ResourceAllocationLegVersionV1::from(leg)
                || observation.status != leg.status
                || !grant.accounts.contains(&leg.account)
                || !grant.demands.contains(&leg.demand)
            {
                return Err(ResourceError::Authority(
                    "resource observation allocation differs from authoritative holder ownership"
                        .to_owned(),
                ));
            }
        }
        for observation in &head.fulfillments {
            let Some(fulfillment) = self.fulfillments.get(&observation.fulfillment) else {
                continue;
            };
            let allocation_owned = fulfillment.allocation_legs.iter().all(|id| {
                self.allocation_legs.get(id).is_some_and(|leg| {
                    grant.accounts.contains(&leg.account) && grant.demands.contains(&leg.demand)
                })
            });
            if observation.consumed != fulfillment.consumed_quantity
                || observation.remainder != fulfillment.remainder
                || observation.rejection_reason != fulfillment.rejection_reason
                || !grant.demands.contains(&fulfillment.demand)
                || !allocation_owned
            {
                return Err(ResourceError::Authority(
                    "resource observation fulfillment differs from authoritative holder ownership"
                        .to_owned(),
                ));
            }
        }
        if !grant.include_transfer_details
            && (!head.transfers.is_empty() || !head.consumptions.is_empty())
        {
            return Err(ResourceError::Authority(
                "resource observation transfer details exceed its holder grant".to_owned(),
            ));
        }
        for observation in &head.transfers {
            let Some(transfer) = self.transfers.get(&observation.transfer) else {
                continue;
            };
            let owns_account = grant.accounts.contains(&transfer.source)
                || transfer
                    .destination
                    .as_ref()
                    .is_some_and(|account| grant.accounts.contains(account));
            if !owns_account
                || observation.state != transfer.state
                || observation.quantity != transfer.quantity
                || observation.escrow != transfer.escrow
                || observation.accepted != transfer.accepted
                || observation.lost != transfer.lost
                || observation.returned != transfer.returned
                || observation.external_outflow != transfer.external_outflow
            {
                return Err(ResourceError::Authority(
                    "resource observation transfer differs from authoritative holder custody"
                        .to_owned(),
                ));
            }
        }
        for observation in &head.consumptions {
            let Some(consumption) = self.consumptions.get(&observation.consumption) else {
                continue;
            };
            if observation.exact != ResourceConsumptionVersionV1::from(consumption)
                || observation.demand != consumption.demand
                || observation.status != consumption.status
                || !grant.accounts.contains(&consumption.account)
                || !grant.demands.contains(&consumption.demand)
            {
                return Err(ResourceError::Authority(
                    "resource observation consumption differs from authoritative holder work"
                        .to_owned(),
                ));
            }
        }
        Ok(())
    }

    pub fn account_quantities(
        &self,
        account_id: &ResourceAccountId,
    ) -> Result<crate::AccountQuantitiesV1, ResourceError> {
        let account = self
            .accounts
            .get(account_id)
            .ok_or_else(|| ResourceError::NotFound("resource account is unavailable".to_owned()))?;
        let reserved = self
            .reservations
            .values()
            .filter(|reservation| {
                &reservation.account == account_id
                    && reservation.status == ReservationStatus::Active
            })
            .try_fold(0_u64, |total, reservation| {
                total
                    .checked_add(reservation.quantity)
                    .ok_or(ResourceError::Overflow)
            })?;
        let protected = account
            .protected_floor_policy
            .as_ref()
            .and_then(|id| self.protected_floor_policies.get(id))
            .map_or(0, |policy| policy.floor.min(account.balance));
        let available = account
            .balance
            .saturating_sub(reserved)
            .saturating_sub(protected);
        Ok(crate::AccountQuantitiesV1 {
            authoritative_balance: account.balance,
            available,
            reserved,
            protected,
        })
    }

    pub fn apply_operation(
        &mut self,
        request: &ResourceOperationRequestV1,
    ) -> Result<ResourceOperationOutcome, ResourceError> {
        let request_digest = canonical_digest("canwu.resource.operation-request.v1", request)?;
        self.apply_operation_with_context(request, request_digest, None, None)
    }

    /// Apply every credit in one production output batch atomically. A
    /// rejected leg discards the detached candidate, so no account, outcome,
    /// conservation, or terminal-index mutation becomes authoritative.
    pub fn apply_production_output_batch(
        &mut self,
        requests: &[ResourceCreditRequestV1],
    ) -> Result<Vec<ResourceOperationOutcome>, ResourceError> {
        if requests.is_empty() || requests.len() > 64 {
            return Err(ResourceError::LimitExceeded(
                "production output batch must contain 1..=64 credits".to_owned(),
            ));
        }
        let certificate = &requests[0].completion_certificate;
        let mut keys = BTreeSet::new();
        let mut accounts = BTreeSet::new();
        if requests.iter().any(|request| {
            request.completion_certificate != *certificate
                || !keys.insert(request.operation_key.clone())
                || !accounts.insert(request.account.clone())
        }) {
            return Err(ResourceError::InvalidDefinition(
                "production output batch must use one certificate and unique operation/account legs"
                    .to_owned(),
            ));
        }
        let mut candidate = self.clone();
        let mut outcomes = Vec::with_capacity(requests.len());
        for request in requests {
            let outcome =
                candidate.apply_operation(&ResourceOperationRequestV1::Credit(request.clone()))?;
            if !matches!(
                outcome.status,
                ResourceOperationStatus::Applied | ResourceOperationStatus::Duplicate
            ) {
                return Err(ResourceError::InvalidLifecycle(
                    outcome
                        .rejection_reason
                        .unwrap_or_else(|| "production output batch leg was rejected".to_owned()),
                ));
            }
            outcomes.push(outcome);
        }
        *self = candidate;
        Ok(outcomes)
    }

    pub(crate) fn apply_prepare_with_external_revalidation(
        &mut self,
        request: &crate::PrepareCompletionCapacityV1,
        external_targets_current: bool,
    ) -> Result<ResourceOperationOutcome, ResourceError> {
        let operation = ResourceOperationRequestV1::Completion(
            ResourceCompletionOperationV1::Prepare(request.clone()),
        );
        let request_digest = canonical_digest("canwu.resource.operation-request.v1", &operation)?;
        self.apply_operation_with_context(
            &operation,
            request_digest,
            None,
            Some(external_targets_current),
        )
    }

    pub(crate) fn apply_authorized_allocation(
        &mut self,
        requester: &canwu_api::KnowledgeHolderRef,
        request: &ResourceAllocationRequestV1,
    ) -> Result<ResourceOperationOutcome, ResourceError> {
        let operation = ResourceOperationRequestV1::Allocate(request.clone());
        let request_digest = canonical_digest(
            "canwu.resource.authorized-allocation-request.v1",
            &(requester, request),
        )?;
        self.apply_operation_with_context(&operation, request_digest, Some(requester), None)
    }

    fn apply_operation_with_context(
        &mut self,
        request: &ResourceOperationRequestV1,
        request_digest: String,
        allocation_requester: Option<&canwu_api::KnowledgeHolderRef>,
        prepare_external_targets_current: Option<bool>,
    ) -> Result<ResourceOperationOutcome, ResourceError> {
        let operation_key = request.operation_key();
        if let Some(existing) = self.outcomes.get(&operation_key) {
            if existing.request_digest == request_digest {
                return Ok(existing.clone());
            }
            return Err(ResourceError::IdempotencyConflict(
                "resource operation key was reused with a different request".to_owned(),
            ));
        }
        let terminal_work_is_reserved = self.terminal_work_is_reserved(request);
        let newly_reserved_outcomes = match request {
            ResourceOperationRequestV1::Completion(ResourceCompletionOperationV1::Acquire(
                request,
            )) => usize::from(request.recipe.receipts),
            ResourceOperationRequestV1::Completion(
                ResourceCompletionOperationV1::GrantExternalParticipant(request),
            ) => usize::from(request.recipe.receipts),
            _ => 0,
        };
        let mut projected_outcomes = self
            .outcomes
            .len()
            .checked_add(1)
            .ok_or(ResourceError::Overflow)?;
        if !terminal_work_is_reserved {
            projected_outcomes = projected_outcomes
                .checked_add(self.reserved_hot_outcome_slots()?)
                .ok_or(ResourceError::Overflow)?;
        }
        projected_outcomes = projected_outcomes
            .checked_add(newly_reserved_outcomes)
            .ok_or(ResourceError::Overflow)?;
        if projected_outcomes > self.limits.max_operation_outcomes {
            return Err(ResourceError::LimitExceeded(
                "resource hot outcome capacity is exhausted or reserved for admitted terminal work"
                    .to_owned(),
            ));
        }
        let required_terminal_slots = match request {
            ResourceOperationRequestV1::Consume(_) => 7,
            ResourceOperationRequestV1::Credit(_)
            | ResourceOperationRequestV1::ExternalOutflow(_) => 2,
            ResourceOperationRequestV1::CompleteTransfer(
                ResourceTransferDispositionRequestV1 {
                    disposition:
                        ResourceTransferDispositionV1::Accept { .. }
                        | ResourceTransferDispositionV1::Lose { .. },
                    ..
                },
            ) => 8,
            ResourceOperationRequestV1::CompleteTransfer(_) => 4,
            ResourceOperationRequestV1::Completion(_) => 2,
            _ => 1,
        };
        let mut projected = self
            .terminal_archive_candidates
            .len()
            .checked_add(required_terminal_slots)
            .ok_or(ResourceError::Overflow)?;
        if terminal_work_is_reserved {
            projected = projected
                .checked_add(self.pending_external_participant_archive_slots()?)
                .ok_or(ResourceError::Overflow)?;
        } else {
            let newly_reserved = match request {
                ResourceOperationRequestV1::Completion(ResourceCompletionOperationV1::Acquire(
                    request,
                )) => usize::from(request.recipe.receipts),
                _ => 0,
            };
            projected = projected
                .checked_add(self.reserved_terminal_archive_slots()?)
                .and_then(|value| value.checked_add(newly_reserved))
                .ok_or(ResourceError::Overflow)?;
        }
        if projected > self.limits.max_archive_candidates {
            return Err(ResourceError::LimitExceeded(
                "resource terminal archive backpressure is active".to_owned(),
            ));
        }
        let before = self.clone();
        let applied = self.apply_inner(
            request,
            allocation_requester,
            prepare_external_targets_current,
        );
        let (
            status,
            quantity,
            remainder,
            result_ref,
            exact_evidence,
            rejection_code,
            rejection_reason,
        ) = match applied {
            Ok(applied) => (
                ResourceOperationStatus::Applied,
                applied.quantity,
                applied.remainder,
                applied.result_ref,
                applied.exact_evidence,
                None,
                None,
            ),
            Err(error) => {
                *self = before;
                (
                    ResourceOperationStatus::Rejected,
                    0,
                    request_remainder(self, request),
                    None,
                    Vec::new(),
                    Some(error.code().to_owned()),
                    Some(error.to_string()),
                )
            }
        };
        self.state_revision = self.state_revision.next()?;
        let id = outcome_id(&operation_key, &request_digest)?;
        let mut outcome = ResourceOperationOutcome {
            id,
            revision: ResourceRevision::INITIAL,
            operation_key,
            request_digest,
            kind: request.kind(),
            status,
            quantity,
            remainder,
            result_ref,
            rejection_code,
            rejection_reason,
            exact_evidence,
            semantic_digest: String::new(),
            sequence: self.next_sequence()?,
        };
        outcome.semantic_digest =
            canonical_digest("canwu.resource.operation-outcome.v1", &outcome)?;
        self.terminal_archive_candidates.insert(
            outcome.sequence,
            crate::ResourceTerminalRecordKeyV1::Outcome(outcome.operation_key.clone()),
        );
        self.outcomes
            .insert(outcome.operation_key.clone(), outcome.clone());
        if status == ResourceOperationStatus::Applied
            && !matches!(request, ResourceOperationRequestV1::RecordObservation(_))
        {
            self.report_dirty_grants
                .extend(self.report_grants.keys().cloned());
        }
        self.refresh_continuation();
        self.validate()?;
        Ok(outcome)
    }

    fn apply_inner(
        &mut self,
        request: &ResourceOperationRequestV1,
        allocation_requester: Option<&canwu_api::KnowledgeHolderRef>,
        prepare_external_targets_current: Option<bool>,
    ) -> Result<AppliedOperation, ResourceError> {
        match request {
            ResourceOperationRequestV1::CreateAccount(value) => self.create_account(value),
            ResourceOperationRequestV1::SubmitDemand(value) => self.submit_demand(value),
            ResourceOperationRequestV1::AmendDemand(value) => self.amend_demand(value),
            ResourceOperationRequestV1::Allocate(value) => {
                self.allocate(value, allocation_requester)
            }
            ResourceOperationRequestV1::Consume(value) => self.consume(value),
            ResourceOperationRequestV1::BeginTransfer(value) => self.begin_transfer(value),
            ResourceOperationRequestV1::AdvanceTransfer(value) => self.advance_transfer(value),
            ResourceOperationRequestV1::CancelTransfer(value) => self.cancel_transfer(value),
            ResourceOperationRequestV1::CompleteTransfer(value) => self.complete_transfer(value),
            ResourceOperationRequestV1::Credit(value) => self.credit(value),
            ResourceOperationRequestV1::ExternalOutflow(value) => self.external_outflow(value),
            ResourceOperationRequestV1::SetProtectedFloor(value) => self.set_protected_floor(value),
            ResourceOperationRequestV1::CancelDemand(value) => self.cancel_demand(value),
            ResourceOperationRequestV1::RecordObservation(value) => {
                self.record_observation_head(value.head.clone())?;
                Ok(AppliedOperation {
                    quantity: 0,
                    remainder: 0,
                    result_ref: None,
                    exact_evidence: value.head.source_versions.clone(),
                })
            }
            ResourceOperationRequestV1::Completion(value) => {
                self.apply_completion(value, prepare_external_targets_current)
            }
        }
    }

    fn apply_completion(
        &mut self,
        operation: &ResourceCompletionOperationV1,
        prepare_external_targets_current: Option<bool>,
    ) -> Result<AppliedOperation, ResourceError> {
        if matches!(
            operation,
            ResourceCompletionOperationV1::GrantExternalParticipant(_)
                | ResourceCompletionOperationV1::PrepareExternalParticipant(_)
                | ResourceCompletionOperationV1::ConsumeExternalParticipant(_)
                | ResourceCompletionOperationV1::CompleteExternalParticipant(_)
                | ResourceCompletionOperationV1::ReleaseExternalParticipant(_)
                | ResourceCompletionOperationV1::ExpireExternalParticipants(_)
        ) {
            return self.apply_external_completion_participant(operation);
        }
        let operation_key = operation.operation_key();
        let (acquisition, grant, action, reason, exact_evidence, result_ref) = match operation {
            ResourceCompletionOperationV1::Acquire(request) => {
                if request.recipe.receipts != crate::MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE {
                    return Err(ResourceError::LimitExceeded(
                        "resource completion acquisition must reserve the full bounded terminal receipt path"
                            .to_owned(),
                    ));
                }
                let exact_evidence = request.eligibility_envelope.exact_evidence.clone();
                let acquisition = self
                    .completion_leases
                    .request_acquisition(&self.run_budget, request.clone())?;
                self.reserve_completion_reports(&acquisition.id, &request.holder, &request.recipe)?;
                (
                    acquisition.id.clone(),
                    None,
                    CompletionLeaseReceiptActionV1::Requested,
                    None,
                    exact_evidence,
                    Some(ResourceRecordRefV1::Lease(acquisition.id)),
                )
            }
            ResourceCompletionOperationV1::Grant(request) => {
                self.validate_completion_locked_targets(&request.target_versions)?;
                let grant = self
                    .completion_leases
                    .grant_capacity(&self.run_budget, request.clone())?;
                (
                    grant.acquisition,
                    Some(grant.id),
                    CompletionLeaseReceiptActionV1::Granted,
                    None,
                    Vec::new(),
                    None,
                )
            }
            ResourceCompletionOperationV1::Prepare(request) => {
                let exact = self
                    .completion_leases
                    .acquisitions
                    .get(&request.acquisition)
                    .ok_or_else(|| {
                        ResourceError::NotFound(
                            "completion prepare acquisition is unavailable".to_owned(),
                        )
                    })?
                    .eligibility_envelope
                    .validate()
                    .and_then(|()| {
                        let targets = &self
                            .completion_leases
                            .grants
                            .get(&request.grant)
                            .ok_or_else(|| {
                                ResourceError::NotFound(
                                    "completion prepare grant is unavailable".to_owned(),
                                )
                            })?
                            .target_versions;
                        self.validate_completion_locked_targets(targets)
                    })
                    .and_then(|()| {
                        if prepare_external_targets_current == Some(false) {
                            Err(ResourceError::VersionConflict(
                                "completion prepare external target is stale".to_owned(),
                            ))
                        } else {
                            Ok(())
                        }
                    });
                let grant = if exact.is_ok() {
                    self.completion_leases.prepare_capacity(request.clone())?
                } else {
                    self.completion_leases
                        .reject_prepare_exact_mismatch(request)?
                };
                let (action, reason) = if grant.state == crate::CompletionGrantStateV1::Prepared {
                    (CompletionLeaseReceiptActionV1::Prepared, None)
                } else {
                    (
                        CompletionLeaseReceiptActionV1::Rejected,
                        grant.rejection.clone(),
                    )
                };
                (
                    grant.acquisition,
                    Some(grant.id),
                    action,
                    reason,
                    Vec::new(),
                    None,
                )
            }
            ResourceCompletionOperationV1::Activate(request) => {
                let certificate = self.completion_leases.activate_capacity(request.clone())?;
                (
                    certificate.acquisition.clone(),
                    Some(request.grant.clone()),
                    CompletionLeaseReceiptActionV1::Activated,
                    None,
                    Vec::new(),
                    Some(ResourceRecordRefV1::Lease(certificate.acquisition)),
                )
            }
            ResourceCompletionOperationV1::Abort(request) => {
                let reason = self
                    .completion_leases
                    .abort(
                        &request.holder,
                        &request.acquisition,
                        request.expected_revision,
                    )?
                    .to_owned();
                if reason != "already_activated" {
                    self.completion_report_reservations
                        .remove(&request.acquisition);
                    self.completion_report_ready.remove(&request.acquisition);
                }
                (
                    request.acquisition.clone(),
                    None,
                    CompletionLeaseReceiptActionV1::Aborted,
                    Some(reason),
                    Vec::new(),
                    Some(ResourceRecordRefV1::Lease(request.acquisition.clone())),
                )
            }
            ResourceCompletionOperationV1::Expire(request) => {
                let expired = self.completion_leases.expire_capacity(request)?;
                for grant_id in &expired {
                    if let Some(grant) = self.completion_leases.grants.get(grant_id) {
                        self.completion_report_reservations
                            .remove(&grant.acquisition);
                    }
                }
                let acquisition = expired
                    .first()
                    .and_then(|grant| self.completion_leases.grants.get(grant))
                    .map(|grant| grant.acquisition.clone())
                    .unwrap_or_else(|| {
                        CompletionLeaseAcquisitionId::new("resource:completion:none")
                            .expect("static completion identity is valid")
                    });
                (
                    acquisition,
                    expired.first().cloned(),
                    CompletionLeaseReceiptActionV1::Expired,
                    None,
                    Vec::new(),
                    None,
                )
            }
            ResourceCompletionOperationV1::Release(request) => {
                self.completion_leases.release_capacity(request)?;
                if self
                    .completion_leases
                    .acquisitions
                    .get(&request.acquisition)
                    .is_some_and(|value| {
                        value.state == crate::CompletionLeaseAcquisitionStateV1::Released
                    })
                {
                    self.completion_report_reservations
                        .remove(&request.acquisition);
                    self.completion_report_ready.remove(&request.acquisition);
                }
                (
                    request.acquisition.clone(),
                    Some(request.grant.clone()),
                    CompletionLeaseReceiptActionV1::Released,
                    Some(request.reason.clone()),
                    Vec::new(),
                    Some(ResourceRecordRefV1::Lease(request.acquisition.clone())),
                )
            }
            ResourceCompletionOperationV1::GrantExternalParticipant(_)
            | ResourceCompletionOperationV1::PrepareExternalParticipant(_)
            | ResourceCompletionOperationV1::ConsumeExternalParticipant(_)
            | ResourceCompletionOperationV1::CompleteExternalParticipant(_)
            | ResourceCompletionOperationV1::ReleaseExternalParticipant(_)
            | ResourceCompletionOperationV1::ExpireExternalParticipants(_) => {
                unreachable!("external participant operations return before local lease handling")
            }
        };
        let receipt = self.completion_leases.record_receipt(
            operation_key,
            acquisition.clone(),
            grant,
            action,
            reason,
        )?;
        let sequence = self.next_sequence()?;
        self.terminal_archive_candidates.insert(
            sequence,
            crate::ResourceTerminalRecordKeyV1::LeaseReceipt(receipt.sequence),
        );
        Ok(AppliedOperation {
            quantity: receipt.reserved_units,
            remainder: 0,
            result_ref,
            exact_evidence,
        })
    }

    fn apply_external_completion_participant(
        &mut self,
        operation: &ResourceCompletionOperationV1,
    ) -> Result<AppliedOperation, ResourceError> {
        match operation {
            ResourceCompletionOperationV1::GrantExternalParticipant(request) => {
                self.validate_completion_locked_targets(&request.target_versions)?;
                let participant = self
                    .external_completion_participants
                    .grant(&self.run_budget, request.clone())?;
                self.reserve_completion_reports(
                    &request.acquisition,
                    &request.holder,
                    &request.recipe,
                )?;
                Ok(AppliedOperation {
                    quantity: participant.grant.reserved_units,
                    remainder: 0,
                    result_ref: Some(ResourceRecordRefV1::Lease(participant.grant.acquisition)),
                    exact_evidence: vec![request.coordinator_source.clone()],
                })
            }
            ResourceCompletionOperationV1::PrepareExternalParticipant(request) => {
                let targets = self
                    .external_completion_participants
                    .grants
                    .get(&request.acquisition)
                    .ok_or_else(|| {
                        ResourceError::NotFound(
                            "external participant grant is unavailable".to_owned(),
                        )
                    })?
                    .grant
                    .target_versions
                    .clone();
                let participant = if self.validate_completion_locked_targets(&targets).is_ok() {
                    self.external_completion_participants
                        .prepare(request.clone())?
                } else {
                    let participant = self
                        .external_completion_participants
                        .reject_prepare(request, "prepare_exact_target_mismatch")?;
                    self.completion_report_reservations
                        .remove(&request.acquisition);
                    self.completion_report_ready.remove(&request.acquisition);
                    participant
                };
                Ok(AppliedOperation {
                    quantity: participant.grant.reserved_units,
                    remainder: 0,
                    result_ref: Some(ResourceRecordRefV1::Lease(participant.grant.acquisition)),
                    exact_evidence: vec![request.coordinator_source.clone()],
                })
            }
            ResourceCompletionOperationV1::ConsumeExternalParticipant(request) => {
                let participant = self
                    .external_completion_participants
                    .consume(request.clone())?;
                Ok(AppliedOperation {
                    quantity: participant.grant.reserved_units,
                    remainder: 0,
                    result_ref: Some(ResourceRecordRefV1::Lease(participant.grant.acquisition)),
                    exact_evidence: vec![request.coordinator_source.clone()],
                })
            }
            ResourceCompletionOperationV1::CompleteExternalParticipant(request) => {
                self.external_completion_participants.complete(request)?;
                if let Some(grants) = self
                    .completion_report_reservations
                    .get(&request.acquisition)
                    .cloned()
                {
                    self.completion_report_ready
                        .insert(request.acquisition.clone(), self.state_revision.next()?);
                    self.report_dirty_grants.extend(grants);
                } else {
                    self.mark_external_participant_archive_ready(&request.acquisition)?;
                }
                Ok(AppliedOperation {
                    quantity: 0,
                    remainder: 0,
                    result_ref: Some(ResourceRecordRefV1::Lease(request.acquisition.clone())),
                    exact_evidence: Vec::new(),
                })
            }
            ResourceCompletionOperationV1::ReleaseExternalParticipant(request) => {
                let participant = self
                    .external_completion_participants
                    .release(request.clone())?;
                self.completion_report_reservations
                    .remove(&request.acquisition);
                Ok(AppliedOperation {
                    quantity: participant.grant.reserved_units,
                    remainder: 0,
                    result_ref: Some(ResourceRecordRefV1::Lease(participant.grant.acquisition)),
                    exact_evidence: vec![request.coordinator_source.clone()],
                })
            }
            ResourceCompletionOperationV1::ExpireExternalParticipants(request) => {
                let expired = self.external_completion_participants.expire(request)?;
                for acquisition in &expired {
                    self.completion_report_reservations.remove(acquisition);
                    self.completion_report_ready.remove(acquisition);
                }
                Ok(AppliedOperation {
                    quantity: u64::try_from(expired.len()).map_err(|_| ResourceError::Overflow)?,
                    remainder: 0,
                    result_ref: expired.first().cloned().map(ResourceRecordRefV1::Lease),
                    exact_evidence: Vec::new(),
                })
            }
            ResourceCompletionOperationV1::Acquire(_)
            | ResourceCompletionOperationV1::Grant(_)
            | ResourceCompletionOperationV1::Prepare(_)
            | ResourceCompletionOperationV1::Activate(_)
            | ResourceCompletionOperationV1::Abort(_)
            | ResourceCompletionOperationV1::Expire(_)
            | ResourceCompletionOperationV1::Release(_) => unreachable!(
                "local completion operations are handled by the regular lease coordinator"
            ),
        }
    }

    fn create_account(
        &mut self,
        request: &ResourceCreateAccountRequestV1,
    ) -> Result<AppliedOperation, ResourceError> {
        if request.account.balance != 0 {
            return Err(ResourceError::Conservation(
                "runtime resource account creation must begin at zero".to_owned(),
            ));
        }
        self.install_opening_account(request.account.clone())?;
        self.conservation.opening_balances = self
            .conservation
            .opening_balances
            .checked_sub(u128::from(request.account.balance))
            .ok_or(ResourceError::Overflow)?;
        Ok(AppliedOperation {
            quantity: 0,
            remainder: 0,
            result_ref: None,
            exact_evidence: Vec::new(),
        })
    }

    fn submit_demand(
        &mut self,
        request: &ResourceSubmitDemandRequestV1,
    ) -> Result<AppliedOperation, ResourceError> {
        self.install_demand(request.demand.clone())?;
        Ok(AppliedOperation {
            quantity: 0,
            remainder: request.demand.requested,
            result_ref: None,
            exact_evidence: Vec::new(),
        })
    }

    fn amend_demand(
        &mut self,
        request: &ResourceAmendDemandRequestV1,
    ) -> Result<AppliedOperation, ResourceError> {
        let current = self
            .demands
            .get(&request.replacement.id)
            .cloned()
            .ok_or_else(|| ResourceError::NotFound("resource demand is unavailable".to_owned()))?;
        if current.revision != request.expected_demand_revision {
            return Err(ResourceError::VersionConflict(
                "resource demand amendment expected a stale revision".to_owned(),
            ));
        }
        if !matches!(
            current.status,
            DemandStatus::Open | DemandStatus::PartiallyFulfilled
        ) || request.replacement.requested < current.fulfilled
            || request.replacement.fulfilled != current.fulfilled
            || request.replacement.resource_revision != current.resource_revision
            || request.replacement.unit_revision != current.unit_revision
        {
            return Err(ResourceError::InvalidLifecycle(
                "resource demand amendment changes immutable or settled fields".to_owned(),
            ));
        }
        self.validate_demand_contract(&request.replacement)?;
        let mut replacement = request.replacement.clone();
        replacement.revision = current.revision.next()?;
        replacement.admitted_sequence = current.admitted_sequence;
        if let Some(ids) = self.demand_due_index.get_mut(&current.due_at) {
            ids.remove(&current.id);
        }
        if let Some(ids) = self.demand_expiry_index.get_mut(&current.expires_at) {
            ids.remove(&current.id);
        }
        self.demand_due_index
            .entry(replacement.due_at)
            .or_default()
            .insert(replacement.id.clone());
        self.demand_expiry_index
            .entry(replacement.expires_at)
            .or_default()
            .insert(replacement.id.clone());
        self.dirty_demands.insert(replacement.id.clone());
        self.demands
            .insert(replacement.id.clone(), replacement.clone());
        Ok(AppliedOperation {
            quantity: 0,
            remainder: replacement.remainder(),
            result_ref: None,
            exact_evidence: Vec::new(),
        })
    }

    #[allow(clippy::too_many_lines)]
    fn allocate(
        &mut self,
        request: &ResourceAllocationRequestV1,
        requester: Option<&canwu_api::KnowledgeHolderRef>,
    ) -> Result<AppliedOperation, ResourceError> {
        self.expect_state_revision(request.expected_state_revision)?;
        if request.candidate_limit == 0
            || request.candidate_limit > self.limits.max_allocation_candidates
        {
            return Err(ResourceError::LimitExceeded(
                "allocation candidate budget is invalid".to_owned(),
            ));
        }
        if requester.is_some_and(|requester| {
            !self
                .demands
                .values()
                .any(|demand| &demand.requester == requester)
        }) {
            return Err(ResourceError::Authority(
                "resource allocation requester owns no target demand".to_owned(),
            ));
        }
        self.expire_demands(request.at, request.candidate_limit, requester)?;
        let candidate_ids: BTreeSet<_> = self
            .demand_due_index
            .range(..=request.at)
            .flat_map(|(_, ids)| ids.iter().cloned())
            .chain(self.dirty_demands.iter().cloned())
            .filter(|id| {
                requester.is_none_or(|requester| {
                    self.demands
                        .get(id)
                        .is_some_and(|demand| &demand.requester == requester)
                })
            })
            .take(request.candidate_limit.saturating_add(1))
            .collect();
        if candidate_ids.len() > request.candidate_limit {
            return Err(ResourceError::LimitExceeded(
                "allocation due/dirty candidate budget was exceeded".to_owned(),
            ));
        }
        let mut candidates: Vec<_> = candidate_ids
            .iter()
            .filter_map(|id| self.demands.get(id))
            .filter(|demand| {
                matches!(
                    demand.status,
                    DemandStatus::Open | DemandStatus::PartiallyFulfilled
                ) && demand.due_at <= request.at
                    && request.at < demand.expires_at
                    && demand.remainder()
                        > self
                            .active_reserved_for_demand(&demand.id)
                            .unwrap_or(u64::MAX)
            })
            .map(|demand| {
                (
                    Reverse(demand.priority),
                    demand.due_at,
                    demand.tie_break.clone(),
                    demand.admitted_sequence,
                    demand.id.clone(),
                )
            })
            .collect();
        candidates.sort();
        let mut allocated = 0_u64;
        let mut last_leg = None;
        let mut active_allocation_count = self
            .reservations
            .values()
            .filter(|reservation| reservation.status == ReservationStatus::Active)
            .count();
        for (_, _, _, _, demand_id) in candidates {
            let demand = self.demands.get(&demand_id).cloned().ok_or_else(|| {
                ResourceError::NotFound("allocation demand disappeared".to_owned())
            })?;
            let reserved = self.active_reserved_for_demand(&demand.id)?;
            let needed = demand.remainder().saturating_sub(reserved);
            if needed == 0 {
                continue;
            }
            let mut accounts: Vec<_> = self
                .accounts
                .values()
                .filter(|account| {
                    !account.closed
                        && account.resource_revision == demand.resource_revision
                        && account.unit_revision == demand.unit_revision
                })
                .map(|account| account.id.clone())
                .collect();
            accounts.sort();
            let total_available = accounts.iter().try_fold(0_u64, |total, account| {
                total
                    .checked_add(self.available_for_demand(account, &demand)?)
                    .ok_or(ResourceError::Overflow)
            })?;
            let grantable = needed.min(total_available);
            let minimum_remaining = demand
                .minimum_useful
                .saturating_sub(demand.fulfilled + reserved);
            let partial_blocked = demand.partial_fulfillment
                == PartialFulfillmentPolicy::RejectPartial
                && grantable < needed;
            if grantable == 0 || grantable < minimum_remaining || partial_blocked {
                if reserved == 0 && demand.fulfilled == 0 && grantable < demand.minimum_useful {
                    let current = self.demands.get_mut(&demand.id).expect("candidate exists");
                    current.status = DemandStatus::RejectedMinimum;
                    current.rejection_reason = Some("minimum_useful_not_met".to_owned());
                    current.revision = current.revision.next()?;
                    if let Some(ids) = self.demand_due_index.get_mut(&demand.due_at) {
                        ids.remove(&demand.id);
                    }
                    if let Some(ids) = self.demand_expiry_index.get_mut(&demand.expires_at) {
                        ids.remove(&demand.id);
                    }
                    self.dirty_demands.remove(&demand.id);
                    self.mark_terminal_demand_closure(&demand.id)?;
                }
                continue;
            }
            let mut remaining = grantable;
            for account_id in accounts {
                if remaining == 0 {
                    break;
                }
                let available = self.available_for_demand(&account_id, &demand)?;
                let quantity = remaining.min(available);
                if quantity == 0 {
                    continue;
                }
                if active_allocation_count >= self.limits.max_demands {
                    return Err(ResourceError::LimitExceeded(
                        "resource active reservation/allocation capacity is exhausted".to_owned(),
                    ));
                }
                let sequence = self.next_sequence()?;
                let reservation_id =
                    ResourceReservationId::new(format!("resource:reservation:{sequence}"))?;
                let leg_id =
                    ResourceAllocationLegId::new(format!("resource:allocation:{sequence}"))?;
                let account_revision = self.accounts[&account_id].revision;
                let mut leg = ResourceAllocationLeg {
                    id: leg_id.clone(),
                    revision: ResourceRevision::INITIAL,
                    demand: demand.id.clone(),
                    demand_revision: demand.revision,
                    reservation: reservation_id.clone(),
                    account: account_id.clone(),
                    account_revision,
                    resource_revision: demand.resource_revision.clone(),
                    unit_revision: demand.unit_revision.clone(),
                    quantity,
                    status: AllocationLegStatus::Reserved,
                    priority: demand.priority,
                    due_at: demand.due_at,
                    tie_break: demand.tie_break.clone(),
                    admitted_sequence: demand.admitted_sequence,
                    operation_key: request.operation_key.clone(),
                    semantic_digest: String::new(),
                };
                leg.semantic_digest = canonical_digest("canwu.resource.allocation-leg.v1", &leg)?;
                let reservation = ResourceReservation {
                    id: reservation_id.clone(),
                    revision: ResourceRevision::INITIAL,
                    demand: demand.id.clone(),
                    account: account_id,
                    allocation_leg: leg_id.clone(),
                    quantity,
                    status: ReservationStatus::Active,
                    operation_key: request.operation_key.clone(),
                };
                self.reservation_by_demand
                    .entry(demand.id.clone())
                    .or_default()
                    .insert(reservation_id.clone());
                self.reservations.insert(reservation_id, reservation);
                self.allocation_legs.insert(leg_id.clone(), leg);
                active_allocation_count = active_allocation_count
                    .checked_add(1)
                    .ok_or(ResourceError::Overflow)?;
                last_leg = Some(leg_id);
                allocated = allocated
                    .checked_add(quantity)
                    .ok_or(ResourceError::Overflow)?;
                remaining -= quantity;
            }
        }
        for demand_id in candidate_ids {
            self.dirty_demands.remove(&demand_id);
        }
        Ok(AppliedOperation {
            quantity: allocated,
            remainder: self
                .demands
                .values()
                .filter(|demand| {
                    matches!(
                        demand.status,
                        DemandStatus::Open | DemandStatus::PartiallyFulfilled
                    ) && requester.is_none_or(|requester| &demand.requester == requester)
                })
                .try_fold(0_u64, |total, demand| {
                    total
                        .checked_add(demand.remainder())
                        .ok_or(ResourceError::Overflow)
                })?,
            result_ref: last_leg.map(ResourceRecordRefV1::AllocationLeg),
            exact_evidence: Vec::new(),
        })
    }

    fn consume(
        &mut self,
        request: &ResourceConsumptionRequestV1,
    ) -> Result<AppliedOperation, ResourceError> {
        let leg = self.exact_allocation(&request.allocation)?.clone();
        if self.consumptions.contains_key(&request.consumption_id) {
            return Err(ResourceError::IdempotencyConflict(
                "resource consumption identity already exists".to_owned(),
            ));
        }
        let resource_targets = vec![
            CompletionLockedTargetV1::Account {
                id: leg.account.clone(),
                revision: request.expected_account_revision,
            },
            CompletionLockedTargetV1::AllocationLeg {
                id: leg.id.clone(),
                revision: leg.revision,
            },
            CompletionLockedTargetV1::Demand {
                id: leg.demand.clone(),
                revision: leg.demand_revision,
            },
        ];
        let mut local_targets = resource_targets.clone();
        local_targets.push(CompletionLockedTargetV1::ExternalRecord {
            version: request.consumer_evidence.clone(),
        });
        let external_participant = self
            .external_completion_participants
            .grants
            .contains_key(&request.completion_certificate.acquisition);
        let (acquisition, grant) = if external_participant {
            self.validate_consumed_external_completion_certificate(
                &request.completion_certificate,
                request.at,
                &request.operation_key,
                &resource_targets,
            )?
        } else {
            self.consume_completion_certificate(
                &request.completion_certificate,
                request.at,
                &request.operation_key,
                &local_targets,
            )?
        };
        self.consume_leg(&leg, request.expected_account_revision)?;
        let terminal_sequence = self.next_sequence()?;
        let mut consumption = ResourceConsumption {
            id: request.consumption_id.clone(),
            revision: ResourceRevision::INITIAL,
            account: leg.account.clone(),
            allocation_leg: leg.id.clone(),
            demand: leg.demand.clone(),
            resource_revision: leg.resource_revision.clone(),
            unit_revision: leg.unit_revision.clone(),
            quantity: leg.quantity,
            consumer_evidence: request.consumer_evidence.clone(),
            completion_acquisition: acquisition.clone(),
            status: ConsumptionStatus::Settled,
            operation_key: request.operation_key.clone(),
            semantic_digest: String::new(),
            terminal_sequence,
        };
        consumption.semantic_digest =
            canonical_digest("canwu.resource.consumption.v1", &consumption)?;
        self.consumptions
            .insert(consumption.id.clone(), consumption.clone());
        self.terminal_archive_candidates.insert(
            consumption.terminal_sequence,
            crate::ResourceTerminalRecordKeyV1::Consumption(consumption.id.clone()),
        );
        self.conservation.admitted_consumption = self
            .conservation
            .admitted_consumption
            .checked_add(u128::from(leg.quantity))
            .ok_or(ResourceError::Overflow)?;
        let fulfillment = self.record_fulfillment(&leg, request.operation_key.clone())?;
        self.mark_terminal_demand_closure(&leg.demand)?;
        if external_participant {
            self.external_completion_participants.complete(
                &crate::CompleteExternalCompletionParticipantGrantV1 {
                    acquisition: acquisition.clone(),
                    operation_key: request.operation_key.clone(),
                },
            )?;
            if self
                .completion_report_reservations
                .contains_key(&acquisition)
            {
                self.completion_report_ready
                    .insert(acquisition, self.state_revision.next()?);
            }
        } else {
            self.completion_leases
                .complete_grant(&acquisition, &grant)?;
            self.record_completed_completion(request.operation_key.clone(), acquisition, grant)?;
        }
        Ok(AppliedOperation {
            quantity: leg.quantity,
            remainder: fulfillment.remainder,
            result_ref: Some(ResourceRecordRefV1::Consumption(consumption.id)),
            exact_evidence: vec![request.consumer_evidence.clone()],
        })
    }

    fn begin_transfer(
        &mut self,
        request: &ResourceTransferStartRequestV1,
    ) -> Result<AppliedOperation, ResourceError> {
        if self.transfers.len() >= self.limits.max_transfers
            || self.transfers.contains_key(&request.transfer_id)
        {
            return Err(ResourceError::LimitExceeded(
                "resource transfer capacity or identity is unavailable".to_owned(),
            ));
        }
        let leg = self.exact_allocation(&request.allocation)?.clone();
        if let Some(destination) = &request.destination {
            let account = self.accounts.get(destination).ok_or_else(|| {
                ResourceError::NotFound("transfer destination account is unavailable".to_owned())
            })?;
            if account.resource_revision != leg.resource_revision
                || account.unit_revision != leg.unit_revision
            {
                return Err(ResourceError::InvalidDefinition(
                    "transfer destination exact resource or unit revision differs".to_owned(),
                ));
            }
        }
        let targets = vec![
            CompletionLockedTargetV1::Account {
                id: leg.account.clone(),
                revision: request.expected_account_revision,
            },
            CompletionLockedTargetV1::AllocationLeg {
                id: leg.id.clone(),
                revision: leg.revision,
            },
            CompletionLockedTargetV1::Demand {
                id: leg.demand.clone(),
                revision: leg.demand_revision,
            },
        ];
        let (acquisition, _) = self.consume_completion_certificate(
            &request.completion_certificate,
            request.at,
            &request.operation_key,
            &targets,
        )?;
        self.consume_leg(&leg, request.expected_account_revision)?;
        let transfer = ResourceTransfer {
            id: request.transfer_id.clone(),
            revision: ResourceRevision::INITIAL,
            source: leg.account,
            destination: request.destination.clone(),
            allocation_leg: leg.id,
            resource_revision: leg.resource_revision,
            unit_revision: leg.unit_revision,
            quantity: leg.quantity,
            escrow: leg.quantity,
            accepted: 0,
            lost: 0,
            returned: 0,
            external_outflow: 0,
            state: ResourceTransferState::PendingDispatch,
            transport: None,
            exact_evidence: Vec::new(),
            completion_acquisition: acquisition,
            operation_key: request.operation_key.clone(),
            terminal_sequence: 0,
        };
        self.active_transfers.insert(transfer.id.clone());
        self.transfers.insert(transfer.id.clone(), transfer);
        Ok(AppliedOperation {
            quantity: leg.quantity,
            remainder: 0,
            result_ref: Some(ResourceRecordRefV1::Transfer(request.transfer_id.clone())),
            exact_evidence: Vec::new(),
        })
    }

    fn advance_transfer(
        &mut self,
        request: &ResourceTransferProgressRequestV1,
    ) -> Result<AppliedOperation, ResourceError> {
        let transfer = self.transfers.get_mut(&request.transfer).ok_or_else(|| {
            ResourceError::NotFound("resource transfer is unavailable".to_owned())
        })?;
        if transfer.revision != request.expected_transfer_revision || transfer.escrow == 0 {
            return Err(ResourceError::VersionConflict(
                "resource transfer exact revision is stale or terminal".to_owned(),
            ));
        }
        let next = match (transfer.state, request.progress) {
            (ResourceTransferState::PendingDispatch, TransferProgressV1::InTransit) => {
                ResourceTransferState::InTransit
            }
            (ResourceTransferState::InTransit, TransferProgressV1::ArrivalPending) => {
                ResourceTransferState::ArrivalPending
            }
            (
                ResourceTransferState::PendingDispatch
                | ResourceTransferState::InTransit
                | ResourceTransferState::ArrivalPending,
                TransferProgressV1::ReturnPending,
            ) => ResourceTransferState::ReturnPending,
            _ => {
                return Err(ResourceError::InvalidLifecycle(
                    "resource transfer progress is invalid".to_owned(),
                ));
            }
        };
        transfer.state = next;
        transfer.transport = Some(request.transport.clone());
        transfer
            .exact_evidence
            .push(request.transport_evidence.clone());
        transfer.exact_evidence.sort();
        transfer.exact_evidence.dedup();
        transfer.revision = transfer.revision.next()?;
        Ok(AppliedOperation {
            quantity: transfer.escrow,
            remainder: 0,
            result_ref: Some(ResourceRecordRefV1::Transfer(transfer.id.clone())),
            exact_evidence: vec![request.transport_evidence.clone()],
        })
    }

    fn cancel_transfer(
        &mut self,
        request: &ResourceTransferCancellationRequestV1,
    ) -> Result<AppliedOperation, ResourceError> {
        let transfer = self.transfers.get_mut(&request.transfer).ok_or_else(|| {
            ResourceError::NotFound("resource transfer is unavailable".to_owned())
        })?;
        if transfer.revision != request.expected_transfer_revision || transfer.escrow == 0 {
            return Err(ResourceError::VersionConflict(
                "resource transfer cancellation expected a stale or terminal revision".to_owned(),
            ));
        }
        if !matches!(
            transfer.state,
            ResourceTransferState::PendingDispatch
                | ResourceTransferState::InTransit
                | ResourceTransferState::ArrivalPending
        ) {
            return Err(ResourceError::InvalidLifecycle(
                "resource transfer cannot be cancelled from its current state".to_owned(),
            ));
        }
        transfer.state = ResourceTransferState::ReturnPending;
        transfer.revision = transfer.revision.next()?;
        Ok(AppliedOperation {
            quantity: transfer.escrow,
            remainder: 0,
            result_ref: Some(ResourceRecordRefV1::Transfer(transfer.id.clone())),
            exact_evidence: Vec::new(),
        })
    }

    #[allow(clippy::too_many_lines)]
    fn complete_transfer(
        &mut self,
        request: &ResourceTransferDispositionRequestV1,
    ) -> Result<AppliedOperation, ResourceError> {
        let snapshot = self
            .transfers
            .get(&request.transfer)
            .cloned()
            .ok_or_else(|| {
                ResourceError::NotFound("resource transfer is unavailable".to_owned())
            })?;
        if snapshot.revision != request.expected_transfer_revision || snapshot.escrow == 0 {
            return Err(ResourceError::VersionConflict(
                "resource transfer exact revision is stale or terminal".to_owned(),
            ));
        }
        let quantity = snapshot.escrow;
        let mut terminal_targets = vec![CompletionLockedTargetV1::Transfer {
            id: snapshot.id.clone(),
            revision: snapshot.revision,
        }];
        match &request.disposition {
            ResourceTransferDispositionV1::Accept {
                destination,
                expected_destination_revision,
                acceptance,
            } => {
                terminal_targets.push(CompletionLockedTargetV1::Account {
                    id: destination.clone(),
                    revision: *expected_destination_revision,
                });
                terminal_targets.push(CompletionLockedTargetV1::ExternalRecord {
                    version: acceptance.evidence.clone(),
                });
            }
            ResourceTransferDispositionV1::Return {
                expected_source_revision,
            } => terminal_targets.push(CompletionLockedTargetV1::Account {
                id: snapshot.source.clone(),
                revision: *expected_source_revision,
            }),
            ResourceTransferDispositionV1::Lose { cause, .. } => {
                if let EvidenceRef::DomainRecordVersion(version) = cause {
                    terminal_targets.push(CompletionLockedTargetV1::ExternalRecord {
                        version: version.clone(),
                    });
                }
            }
            ResourceTransferDispositionV1::ExternalOutflow { authority_evidence } => {
                terminal_targets.push(CompletionLockedTargetV1::ExternalRecord {
                    version: authority_evidence.clone(),
                });
            }
        }
        let (terminal_acquisition, terminal_grant) = self.consume_completion_certificate(
            &request.completion_certificate,
            request.at,
            &request.operation_key,
            &terminal_targets,
        )?;
        let original_grant =
            self.resource_grant_for_acquisition(&snapshot.completion_acquisition)?;
        let mut exact_evidence: Vec<DomainRecordVersionRef> = request
            .exact_transport_evidence
            .clone()
            .into_iter()
            .collect();
        let result_ref = match &request.disposition {
            ResourceTransferDispositionV1::Accept {
                destination,
                expected_destination_revision,
                acceptance,
            } => {
                acceptance.validate()?;
                if snapshot.state != ResourceTransferState::ArrivalPending {
                    return Err(ResourceError::InvalidLifecycle(
                        "destination acceptance requires ArrivalPending".to_owned(),
                    ));
                }
                if snapshot.destination.as_ref() != Some(destination) {
                    return Err(ResourceError::InvalidDefinition(
                        "acceptance destination differs from the transfer".to_owned(),
                    ));
                }
                if snapshot.transport.as_ref() != Some(&acceptance.execution)
                    || acceptance.destination != *destination
                    || acceptance.quantity != quantity
                    || acceptance.accepted_at != request.at
                    || request.exact_transport_evidence.as_ref() != Some(&acceptance.evidence)
                {
                    return Err(ResourceError::VersionConflict(
                        "destination credit requires the exact persisted transport acceptance"
                            .to_owned(),
                    ));
                }
                exact_evidence.push(acceptance.evidence.clone());
                let account = self.accounts.get_mut(destination).ok_or_else(|| {
                    ResourceError::NotFound("destination account is unavailable".to_owned())
                })?;
                if account.revision != *expected_destination_revision
                    || account.resource_revision != snapshot.resource_revision
                    || account.unit_revision != snapshot.unit_revision
                {
                    return Err(ResourceError::VersionConflict(
                        "destination account exact revisions do not match".to_owned(),
                    ));
                }
                credit_account(account, quantity)?;
                let leg = self
                    .allocation_legs
                    .get(&snapshot.allocation_leg)
                    .cloned()
                    .ok_or_else(|| {
                        ResourceError::InvalidDefinition(
                            "transfer lost its allocation leg".to_owned(),
                        )
                    })?;
                let fulfillment = self.record_fulfillment(&leg, request.operation_key.clone())?;
                self.mark_terminal_demand_closure(&leg.demand)?;
                let terminal_sequence = self.next_sequence()?;
                let transfer = self
                    .transfers
                    .get_mut(&request.transfer)
                    .expect("checked above");
                transfer.accepted = quantity;
                transfer.escrow = 0;
                transfer.state = ResourceTransferState::Accepted;
                transfer.revision = transfer.revision.next()?;
                transfer.exact_evidence = exact_evidence.clone();
                transfer.terminal_sequence = terminal_sequence;
                let transfer_id = transfer.id.clone();
                self.terminal_archive_candidates.insert(
                    terminal_sequence,
                    crate::ResourceTerminalRecordKeyV1::Transfer(transfer_id.clone()),
                );
                self.active_transfers.remove(&request.transfer);
                self.completion_leases
                    .complete_grant(&snapshot.completion_acquisition, &original_grant)?;
                self.completion_leases
                    .complete_grant(&terminal_acquisition, &terminal_grant)?;
                self.record_completed_completion(
                    snapshot.operation_key.clone(),
                    snapshot.completion_acquisition.clone(),
                    original_grant,
                )?;
                self.record_completed_completion(
                    request.operation_key.clone(),
                    terminal_acquisition,
                    terminal_grant,
                )?;
                return Ok(AppliedOperation {
                    quantity,
                    remainder: fulfillment.remainder,
                    result_ref: Some(ResourceRecordRefV1::Transfer(transfer_id)),
                    exact_evidence,
                });
            }
            ResourceTransferDispositionV1::Lose { loss_id, cause } => {
                if self.losses.contains_key(loss_id) {
                    return Err(ResourceError::IdempotencyConflict(
                        "resource loss identity already exists".to_owned(),
                    ));
                }
                self.conservation.admitted_loss = self
                    .conservation
                    .admitted_loss
                    .checked_add(u128::from(quantity))
                    .ok_or(ResourceError::Overflow)?;
                let loss = ResourceLoss {
                    id: loss_id.clone(),
                    revision: ResourceRevision::INITIAL,
                    account: None,
                    transfer: Some(snapshot.id.clone()),
                    resource_revision: snapshot.resource_revision.clone(),
                    unit_revision: snapshot.unit_revision.clone(),
                    quantity,
                    cause: cause.clone(),
                    operation_key: request.operation_key.clone(),
                    terminal_sequence: self.next_sequence()?,
                };
                self.losses.insert(loss.id.clone(), loss);
                self.terminal_archive_candidates.insert(
                    self.losses[loss_id].terminal_sequence,
                    crate::ResourceTerminalRecordKeyV1::Loss(loss_id.clone()),
                );
                let transfer = self
                    .transfers
                    .get_mut(&request.transfer)
                    .expect("checked above");
                transfer.lost = quantity;
                transfer.escrow = 0;
                transfer.state = ResourceTransferState::Lost;
                transfer.revision = transfer.revision.next()?;
                ResourceRecordRefV1::Loss(loss_id.clone())
            }
            ResourceTransferDispositionV1::Return {
                expected_source_revision,
            } => {
                if snapshot.state != ResourceTransferState::ReturnPending {
                    return Err(ResourceError::InvalidLifecycle(
                        "return credit requires ReturnPending".to_owned(),
                    ));
                }
                let source = self.accounts.get_mut(&snapshot.source).ok_or_else(|| {
                    ResourceError::NotFound("transfer source account is unavailable".to_owned())
                })?;
                if source.revision != *expected_source_revision {
                    return Err(ResourceError::VersionConflict(
                        "return credit expected a stale source account revision".to_owned(),
                    ));
                }
                credit_account(source, quantity)?;
                let transfer = self
                    .transfers
                    .get_mut(&request.transfer)
                    .expect("checked above");
                transfer.returned = quantity;
                transfer.escrow = 0;
                transfer.state = ResourceTransferState::Returned;
                transfer.revision = transfer.revision.next()?;
                ResourceRecordRefV1::Transfer(transfer.id.clone())
            }
            ResourceTransferDispositionV1::ExternalOutflow { authority_evidence } => {
                exact_evidence.push(authority_evidence.clone());
                self.conservation.external_outflow = self
                    .conservation
                    .external_outflow
                    .checked_add(u128::from(quantity))
                    .ok_or(ResourceError::Overflow)?;
                let transfer = self
                    .transfers
                    .get_mut(&request.transfer)
                    .expect("checked above");
                transfer.external_outflow = quantity;
                transfer.escrow = 0;
                transfer.state = ResourceTransferState::ExternalOutflowSettled;
                transfer.revision = transfer.revision.next()?;
                ResourceRecordRefV1::Transfer(transfer.id.clone())
            }
        };
        let terminal_sequence = self.next_sequence()?;
        let transfer = self
            .transfers
            .get_mut(&request.transfer)
            .expect("checked above");
        transfer.exact_evidence = exact_evidence.clone();
        transfer.terminal_sequence = terminal_sequence;
        self.terminal_archive_candidates.insert(
            terminal_sequence,
            crate::ResourceTerminalRecordKeyV1::Transfer(request.transfer.clone()),
        );
        self.active_transfers.remove(&request.transfer);
        self.completion_leases
            .complete_grant(&snapshot.completion_acquisition, &original_grant)?;
        self.completion_leases
            .complete_grant(&terminal_acquisition, &terminal_grant)?;
        self.record_completed_completion(
            snapshot.operation_key.clone(),
            snapshot.completion_acquisition,
            original_grant,
        )?;
        self.record_completed_completion(
            request.operation_key.clone(),
            terminal_acquisition,
            terminal_grant,
        )?;
        Ok(AppliedOperation {
            quantity,
            remainder: 0,
            result_ref: Some(result_ref),
            exact_evidence,
        })
    }

    fn credit(
        &mut self,
        request: &ResourceCreditRequestV1,
    ) -> Result<AppliedOperation, ResourceError> {
        if request.quantity == 0 {
            return Err(ResourceError::InvalidDefinition(
                "resource credit quantity must be positive".to_owned(),
            ));
        }
        let mut targets = vec![CompletionLockedTargetV1::Account {
            id: request.account.clone(),
            revision: request.expected_account_revision,
        }];
        match &request.source {
            ResourceCreditSourceV1::Production(version) => {
                targets.push(CompletionLockedTargetV1::ExternalRecord {
                    version: version.clone(),
                });
            }
            ResourceCreditSourceV1::ExternalInflow(EvidenceRef::DomainRecordVersion(version)) => {
                targets.push(CompletionLockedTargetV1::ExternalRecord {
                    version: version.clone(),
                });
            }
            ResourceCreditSourceV1::ExternalInflow(_) => {}
        }
        let external_participant = matches!(request.source, ResourceCreditSourceV1::Production(_))
            && self
                .external_completion_participants
                .grants
                .contains_key(&request.completion_certificate.acquisition);
        let (acquisition, grant) = if external_participant {
            self.validate_consumed_external_completion_certificate(
                &request.completion_certificate,
                request.at,
                &request.operation_key,
                &targets,
            )?
        } else {
            self.consume_completion_certificate(
                &request.completion_certificate,
                request.at,
                &request.operation_key,
                &targets,
            )?
        };
        let account = self.accounts.get_mut(&request.account).ok_or_else(|| {
            ResourceError::NotFound("resource credit account is unavailable".to_owned())
        })?;
        if account.revision != request.expected_account_revision
            || account.resource_revision != request.resource_revision
            || account.unit_revision != request.unit_revision
        {
            return Err(ResourceError::VersionConflict(
                "resource credit exact account/resource/unit revision differs".to_owned(),
            ));
        }
        credit_account(account, request.quantity)?;
        let exact_evidence = match &request.source {
            ResourceCreditSourceV1::Production(evidence) => {
                self.conservation.admitted_production = self
                    .conservation
                    .admitted_production
                    .checked_add(u128::from(request.quantity))
                    .ok_or(ResourceError::Overflow)?;
                vec![evidence.clone()]
            }
            ResourceCreditSourceV1::ExternalInflow(_) => {
                self.conservation.external_inflow = self
                    .conservation
                    .external_inflow
                    .checked_add(u128::from(request.quantity))
                    .ok_or(ResourceError::Overflow)?;
                Vec::new()
            }
        };
        if !external_participant {
            self.completion_leases
                .complete_grant(&acquisition, &grant)?;
            self.record_completed_completion(request.operation_key.clone(), acquisition, grant)?;
        }
        Ok(AppliedOperation {
            quantity: request.quantity,
            remainder: 0,
            result_ref: None,
            exact_evidence,
        })
    }

    fn external_outflow(
        &mut self,
        request: &ResourceExternalOutflowRequestV1,
    ) -> Result<AppliedOperation, ResourceError> {
        let targets = vec![
            CompletionLockedTargetV1::Account {
                id: request.account.clone(),
                revision: request.expected_account_revision,
            },
            CompletionLockedTargetV1::ExternalRecord {
                version: request.authority_evidence.clone(),
            },
        ];
        let (acquisition, grant) = self.consume_completion_certificate(
            &request.completion_certificate,
            request.at,
            &request.operation_key,
            &targets,
        )?;
        let quantities = self.account_quantities(&request.account)?;
        let account = self.accounts.get_mut(&request.account).ok_or_else(|| {
            ResourceError::NotFound("external outflow account is unavailable".to_owned())
        })?;
        if account.revision != request.expected_account_revision {
            return Err(ResourceError::VersionConflict(
                "external outflow expected a stale account revision".to_owned(),
            ));
        }
        let spendable = if request.allow_protected {
            account.balance.saturating_sub(quantities.reserved)
        } else {
            quantities.available
        };
        if request.quantity == 0 || request.quantity > spendable {
            return Err(ResourceError::ProtectedFloor(
                "external outflow would consume reserved or protected stock".to_owned(),
            ));
        }
        debit_account(account, request.quantity)?;
        self.conservation.external_outflow = self
            .conservation
            .external_outflow
            .checked_add(u128::from(request.quantity))
            .ok_or(ResourceError::Overflow)?;
        self.completion_leases
            .complete_grant(&acquisition, &grant)?;
        self.record_completed_completion(request.operation_key.clone(), acquisition, grant)?;
        Ok(AppliedOperation {
            quantity: request.quantity,
            remainder: 0,
            result_ref: None,
            exact_evidence: vec![request.authority_evidence.clone()],
        })
    }

    fn set_protected_floor(
        &mut self,
        request: &ResourceProtectedFloorRequestV1,
    ) -> Result<AppliedOperation, ResourceError> {
        if let Some(policy) = &request.policy
            && !self.protected_floor_policies.contains_key(policy)
        {
            return Err(ResourceError::NotFound(
                "protected-floor policy revision is unavailable".to_owned(),
            ));
        }
        let account = self.accounts.get_mut(&request.account).ok_or_else(|| {
            ResourceError::NotFound("protected-floor account is unavailable".to_owned())
        })?;
        if account.revision != request.expected_account_revision {
            return Err(ResourceError::VersionConflict(
                "protected-floor request expected a stale account revision".to_owned(),
            ));
        }
        account.protected_floor_policy.clone_from(&request.policy);
        account.revision = account.revision.next()?;
        Ok(AppliedOperation {
            quantity: self.account_quantities(&request.account)?.protected,
            remainder: 0,
            result_ref: None,
            exact_evidence: Vec::new(),
        })
    }

    fn cancel_demand(
        &mut self,
        request: &ResourceCancelDemandRequestV1,
    ) -> Result<AppliedOperation, ResourceError> {
        let demand = self
            .demands
            .get_mut(&request.demand)
            .ok_or_else(|| ResourceError::NotFound("resource demand is unavailable".to_owned()))?;
        if demand.revision != request.expected_demand_revision {
            return Err(ResourceError::VersionConflict(
                "resource demand cancellation expected a stale revision".to_owned(),
            ));
        }
        if matches!(
            demand.status,
            DemandStatus::Fulfilled | DemandStatus::Cancelled | DemandStatus::Expired
        ) {
            return Err(ResourceError::InvalidLifecycle(
                "resource demand is already terminal".to_owned(),
            ));
        }
        demand.status = DemandStatus::Cancelled;
        demand.revision = demand.revision.next()?;
        let remainder = demand.remainder();
        let due_at = demand.due_at;
        let expires_at = demand.expires_at;
        let reservation_ids = self
            .reservation_by_demand
            .get(&request.demand)
            .cloned()
            .unwrap_or_default();
        for reservation_id in reservation_ids {
            let reservation = self.reservations.get_mut(&reservation_id).ok_or_else(|| {
                ResourceError::InvalidDefinition(
                    "resource demand reservation index is broken".to_owned(),
                )
            })?;
            if reservation.status != ReservationStatus::Active {
                continue;
            }
            reservation.status = ReservationStatus::Released;
            reservation.revision = reservation.revision.next()?;
            if let Some(leg) = self.allocation_legs.get_mut(&reservation.allocation_leg) {
                leg.status = AllocationLegStatus::Released;
                leg.revision = leg.revision.next()?;
                leg.semantic_digest.clear();
                leg.semantic_digest = canonical_digest("canwu.resource.allocation-leg.v1", leg)?;
            }
        }
        if let Some(ids) = self.demand_due_index.get_mut(&due_at) {
            ids.remove(&request.demand);
        }
        if let Some(ids) = self.demand_expiry_index.get_mut(&expires_at) {
            ids.remove(&request.demand);
        }
        self.demand_due_index.retain(|_, ids| !ids.is_empty());
        self.demand_expiry_index.retain(|_, ids| !ids.is_empty());
        self.dirty_demands.remove(&request.demand);
        self.mark_terminal_demand_closure(&request.demand)?;
        Ok(AppliedOperation {
            quantity: 0,
            remainder,
            result_ref: None,
            exact_evidence: Vec::new(),
        })
    }

    fn consume_leg(
        &mut self,
        leg: &ResourceAllocationLeg,
        expected_account_revision: ResourceRevision,
    ) -> Result<(), ResourceError> {
        if leg.status != AllocationLegStatus::Reserved || leg.quantity == 0 {
            return Err(ResourceError::InvalidLifecycle(
                "resource allocation leg is not consumable".to_owned(),
            ));
        }
        let reservation = self.reservations.get_mut(&leg.reservation).ok_or_else(|| {
            ResourceError::InvalidDefinition("allocation leg lost its reservation".to_owned())
        })?;
        if reservation.status != ReservationStatus::Active
            || reservation.account != leg.account
            || reservation.quantity != leg.quantity
        {
            return Err(ResourceError::InvalidLifecycle(
                "allocation reservation is unavailable or differs".to_owned(),
            ));
        }
        let account = self.accounts.get_mut(&leg.account).ok_or_else(|| {
            ResourceError::NotFound("allocation account is unavailable".to_owned())
        })?;
        if account.revision != expected_account_revision
            || account.resource_revision != leg.resource_revision
            || account.unit_revision != leg.unit_revision
        {
            return Err(ResourceError::VersionConflict(
                "allocation consumer expected stale account/resource/unit revisions".to_owned(),
            ));
        }
        debit_account(account, leg.quantity)?;
        reservation.status = ReservationStatus::Consumed;
        reservation.revision = reservation.revision.next()?;
        let current_leg = self.allocation_legs.get_mut(&leg.id).expect("leg exists");
        current_leg.status = AllocationLegStatus::Consumed;
        current_leg.revision = current_leg.revision.next()?;
        current_leg.semantic_digest.clear();
        current_leg.semantic_digest =
            canonical_digest("canwu.resource.allocation-leg.v1", current_leg)?;
        Ok(())
    }

    fn record_fulfillment(
        &mut self,
        leg: &ResourceAllocationLeg,
        operation_key: ResourceOperationKey,
    ) -> Result<ResourceFulfillment, ResourceError> {
        let (demand_id, remainder, due_at, expires_at) = {
            let demand = self.demands.get_mut(&leg.demand).ok_or_else(|| {
                ResourceError::InvalidDefinition("allocation leg lost its demand".to_owned())
            })?;
            demand.fulfilled = demand
                .fulfilled
                .checked_add(leg.quantity)
                .ok_or(ResourceError::Overflow)?;
            if demand.fulfilled > demand.requested {
                return Err(ResourceError::Conservation(
                    "resource demand fulfillment exceeded its requested quantity".to_owned(),
                ));
            }
            demand.status = if demand.fulfilled == demand.requested {
                DemandStatus::Fulfilled
            } else {
                DemandStatus::PartiallyFulfilled
            };
            demand.revision = demand.revision.next()?;
            (
                demand.id.clone(),
                demand.remainder(),
                demand.due_at,
                demand.expires_at,
            )
        };
        if remainder == 0 {
            if let Some(ids) = self.demand_due_index.get_mut(&due_at) {
                ids.remove(&demand_id);
            }
            if let Some(ids) = self.demand_expiry_index.get_mut(&expires_at) {
                ids.remove(&demand_id);
            }
            self.demand_due_index.retain(|_, ids| !ids.is_empty());
            self.demand_expiry_index.retain(|_, ids| !ids.is_empty());
            self.dirty_demands.remove(&demand_id);
        } else {
            self.dirty_demands.insert(demand_id.clone());
        }
        let sequence = self.next_sequence()?;
        let id = ResourceFulfillmentId::new(format!("resource:fulfillment:{sequence}"))?;
        let terminal_sequence = self.next_sequence()?;
        let mut fulfillment = ResourceFulfillment {
            id: id.clone(),
            revision: ResourceRevision::INITIAL,
            demand: demand_id,
            allocation_legs: vec![leg.id.clone()],
            consumed_quantity: leg.quantity,
            remainder,
            status: if remainder == 0 {
                FulfillmentStatus::Complete
            } else {
                FulfillmentStatus::Partial
            },
            rejection_reason: None,
            operation_key,
            semantic_digest: String::new(),
            terminal_sequence,
        };
        fulfillment.semantic_digest =
            canonical_digest("canwu.resource.fulfillment.v1", &fulfillment)?;
        self.fulfillments.insert(id, fulfillment.clone());
        self.terminal_archive_candidates.insert(
            fulfillment.terminal_sequence,
            crate::ResourceTerminalRecordKeyV1::Fulfillment(fulfillment.id.clone()),
        );
        Ok(fulfillment)
    }

    fn exact_allocation(
        &self,
        exact: &ResourceAllocationLegVersionV1,
    ) -> Result<&ResourceAllocationLeg, ResourceError> {
        let leg = self.allocation_legs.get(&exact.id).ok_or_else(|| {
            ResourceError::NotFound("resource allocation leg is unavailable".to_owned())
        })?;
        if leg.revision != exact.revision
            || leg.account != exact.account
            || leg.account_revision != exact.account_revision
            || leg.resource_revision != exact.resource_revision
            || leg.unit_revision != exact.unit_revision
            || leg.quantity != exact.quantity
            || leg.semantic_digest != exact.semantic_digest
        {
            return Err(ResourceError::VersionConflict(
                "resource allocation exact version or semantic digest differs".to_owned(),
            ));
        }
        Ok(leg)
    }

    fn expect_state_revision(&self, revision: ResourceRevision) -> Result<(), ResourceError> {
        if self.state_revision != revision {
            return Err(ResourceError::VersionConflict(
                "resource operation expected a stale state revision".to_owned(),
            ));
        }
        Ok(())
    }

    fn reserved_terminal_archive_slots(&self) -> Result<usize, ResourceError> {
        let local = self
            .completion_leases
            .acquisitions
            .values()
            .filter(|acquisition| {
                !matches!(
                    acquisition.state,
                    crate::CompletionLeaseAcquisitionStateV1::Released
                        | crate::CompletionLeaseAcquisitionStateV1::Expired
                )
            })
            .try_fold(0_usize, |total, acquisition| {
                total
                    .checked_add(usize::from(acquisition.recipe.receipts))
                    .ok_or(ResourceError::Overflow)
            })?;
        local
            .checked_add(self.pending_external_participant_archive_slots()?)
            .ok_or(ResourceError::Overflow)
    }

    fn pending_external_participant_archive_slots(&self) -> Result<usize, ResourceError> {
        self.completion_report_ready
            .keys()
            .filter(|acquisition| {
                self.external_completion_participants
                    .terminal_grants
                    .contains_key(*acquisition)
            })
            .try_fold(0_usize, |total, _| {
                total.checked_add(1).ok_or(ResourceError::Overflow)
            })
    }

    fn reserved_hot_outcome_slots(&self) -> Result<usize, ResourceError> {
        let local = self
            .completion_leases
            .acquisitions
            .values()
            .filter(|acquisition| {
                !matches!(
                    acquisition.state,
                    crate::CompletionLeaseAcquisitionStateV1::Released
                        | crate::CompletionLeaseAcquisitionStateV1::Expired
                )
            })
            .try_fold(0_usize, |total, acquisition| {
                total
                    .checked_add(usize::from(acquisition.recipe.receipts))
                    .ok_or(ResourceError::Overflow)
            })?;
        self.external_completion_participants
            .grants
            .values()
            .filter(|participant| {
                !matches!(
                    participant.grant.state,
                    CompletionGrantStateV1::Completed
                        | CompletionGrantStateV1::Released
                        | CompletionGrantStateV1::Rejected
                        | CompletionGrantStateV1::Expired
                )
            })
            .try_fold(local, |total, participant| {
                total
                    .checked_add(usize::from(participant.recipe.receipts))
                    .ok_or(ResourceError::Overflow)
            })
    }

    pub(crate) fn reserved_knowledge_report_slots(&self) -> Result<usize, ResourceError> {
        self.completion_report_reservations
            .values()
            .try_fold(0_usize, |total, grants| {
                total
                    .checked_add(grants.len())
                    .ok_or(ResourceError::Overflow)
            })
    }

    fn terminal_work_is_reserved(&self, request: &ResourceOperationRequestV1) -> bool {
        match request {
            ResourceOperationRequestV1::Consume(request) => {
                self.completion_leases
                    .certificate(&request.completion_certificate.acquisition)
                    == Some(&request.completion_certificate)
            }
            ResourceOperationRequestV1::BeginTransfer(request) => {
                self.completion_leases
                    .certificate(&request.completion_certificate.acquisition)
                    == Some(&request.completion_certificate)
            }
            ResourceOperationRequestV1::Credit(request) => {
                self.completion_leases
                    .certificate(&request.completion_certificate.acquisition)
                    == Some(&request.completion_certificate)
                    || self
                        .external_completion_participants
                        .participant(&request.completion_certificate.acquisition)
                        .is_some_and(|participant| {
                            participant.certificate.as_ref()
                                == Some(&request.completion_certificate)
                                && participant.grant.state == CompletionGrantStateV1::Consumed
                        })
            }
            ResourceOperationRequestV1::ExternalOutflow(request) => {
                self.completion_leases
                    .certificate(&request.completion_certificate.acquisition)
                    == Some(&request.completion_certificate)
            }
            ResourceOperationRequestV1::AdvanceTransfer(request) => self
                .transfers
                .get(&request.transfer)
                .is_some_and(|transfer| transfer.escrow > 0),
            ResourceOperationRequestV1::CancelTransfer(request) => self
                .transfers
                .get(&request.transfer)
                .is_some_and(|transfer| transfer.escrow > 0),
            ResourceOperationRequestV1::CompleteTransfer(request) => {
                self.transfers
                    .get(&request.transfer)
                    .is_some_and(|transfer| transfer.escrow > 0)
                    && self
                        .completion_leases
                        .certificate(&request.completion_certificate.acquisition)
                        == Some(&request.completion_certificate)
            }
            ResourceOperationRequestV1::Completion(ResourceCompletionOperationV1::Acquire(_)) => {
                false
            }
            ResourceOperationRequestV1::Completion(ResourceCompletionOperationV1::Grant(
                request,
            )) => self
                .completion_leases
                .acquisitions
                .contains_key(&request.acquisition),
            ResourceOperationRequestV1::Completion(ResourceCompletionOperationV1::Prepare(
                request,
            )) => self
                .completion_leases
                .acquisitions
                .contains_key(&request.acquisition),
            ResourceOperationRequestV1::Completion(ResourceCompletionOperationV1::Activate(
                request,
            )) => self
                .completion_leases
                .acquisitions
                .contains_key(&request.acquisition),
            ResourceOperationRequestV1::Completion(ResourceCompletionOperationV1::Abort(
                request,
            )) => self
                .completion_leases
                .acquisitions
                .contains_key(&request.acquisition),
            ResourceOperationRequestV1::Completion(ResourceCompletionOperationV1::Expire(
                request,
            )) => self
                .completion_leases
                .expiry_due
                .range(..=request.current_boundary)
                .any(|(_, grants)| !grants.is_empty()),
            ResourceOperationRequestV1::Completion(ResourceCompletionOperationV1::Release(
                request,
            )) => self
                .completion_leases
                .acquisitions
                .contains_key(&request.acquisition),
            ResourceOperationRequestV1::Completion(
                ResourceCompletionOperationV1::GrantExternalParticipant(request),
            ) => self
                .external_completion_participants
                .grants
                .contains_key(&request.acquisition),
            ResourceOperationRequestV1::Completion(
                ResourceCompletionOperationV1::PrepareExternalParticipant(request),
            ) => self
                .external_completion_participants
                .grants
                .contains_key(&request.acquisition),
            ResourceOperationRequestV1::Completion(
                ResourceCompletionOperationV1::ConsumeExternalParticipant(request),
            ) => self
                .external_completion_participants
                .grants
                .contains_key(&request.certificate.acquisition),
            ResourceOperationRequestV1::Completion(
                ResourceCompletionOperationV1::CompleteExternalParticipant(request),
            ) => self
                .external_completion_participants
                .grants
                .contains_key(&request.acquisition),
            ResourceOperationRequestV1::Completion(
                ResourceCompletionOperationV1::ReleaseExternalParticipant(request),
            ) => self
                .external_completion_participants
                .grants
                .contains_key(&request.acquisition),
            ResourceOperationRequestV1::Completion(
                ResourceCompletionOperationV1::ExpireExternalParticipants(request),
            ) => self
                .external_completion_participants
                .expiry_due
                .range(..=request.current_boundary)
                .any(|(_, grants)| !grants.is_empty()),
            ResourceOperationRequestV1::CreateAccount(_)
            | ResourceOperationRequestV1::SubmitDemand(_)
            | ResourceOperationRequestV1::AmendDemand(_)
            | ResourceOperationRequestV1::Allocate(_)
            | ResourceOperationRequestV1::SetProtectedFloor(_)
            | ResourceOperationRequestV1::CancelDemand(_)
            | ResourceOperationRequestV1::RecordObservation(_) => false,
        }
    }

    fn resource_grant_for_acquisition(
        &self,
        acquisition: &CompletionLeaseAcquisitionId,
    ) -> Result<CompletionCapacityGrantId, ResourceError> {
        let acquisition = self
            .completion_leases
            .acquisitions
            .get(acquisition)
            .ok_or_else(|| {
                ResourceError::NotFound("completion acquisition is unavailable".to_owned())
            })?;
        let grant = acquisition.grants.get(PLUGIN_NAME).ok_or_else(|| {
            ResourceError::NotFound(
                "completion acquisition has no authoritative resource grant".to_owned(),
            )
        })?;
        Ok(grant.clone())
    }

    fn record_completed_completion(
        &mut self,
        operation_key: ResourceOperationKey,
        acquisition: CompletionLeaseAcquisitionId,
        grant: CompletionCapacityGrantId,
    ) -> Result<(), ResourceError> {
        let receipt = self.completion_leases.record_receipt(
            operation_key,
            acquisition.clone(),
            Some(grant),
            CompletionLeaseReceiptActionV1::Completed,
            None,
        )?;
        let sequence = self.next_sequence()?;
        self.terminal_archive_candidates.insert(
            sequence,
            crate::ResourceTerminalRecordKeyV1::LeaseReceipt(receipt.sequence),
        );
        if let Some(grants) = self
            .completion_report_reservations
            .get(&acquisition)
            .cloned()
        {
            self.completion_report_ready
                .insert(acquisition, self.state_revision.next()?);
            self.report_dirty_grants.extend(grants);
        }
        Ok(())
    }

    fn validate_completion_locked_targets(
        &self,
        targets: &[CompletionLockedTargetV1],
    ) -> Result<(), ResourceError> {
        if targets.is_empty() {
            return Err(ResourceError::InvalidDefinition(
                "completion grant has no exact locked targets".to_owned(),
            ));
        }
        for target in targets {
            let current = match target {
                CompletionLockedTargetV1::Account { id, revision } => self
                    .accounts
                    .get(id)
                    .is_some_and(|value| value.revision == *revision),
                CompletionLockedTargetV1::AllocationLeg { id, revision } => self
                    .allocation_legs
                    .get(id)
                    .is_some_and(|value| value.revision == *revision),
                CompletionLockedTargetV1::Demand { id, revision } => self
                    .demands
                    .get(id)
                    .is_some_and(|value| value.revision == *revision),
                CompletionLockedTargetV1::Transfer { id, revision } => self
                    .transfers
                    .get(id)
                    .is_some_and(|value| value.revision == *revision),
                CompletionLockedTargetV1::ExternalRecord { .. } => true,
            };
            if !current {
                return Err(ResourceError::VersionConflict(
                    "completion grant names a stale or unavailable exact target".to_owned(),
                ));
            }
        }
        Ok(())
    }

    fn consume_completion_certificate(
        &mut self,
        certificate: &CompletionLeaseActivationCertificateV1,
        at: SimTime,
        operation_key: &ResourceOperationKey,
        required_targets: &[CompletionLockedTargetV1],
    ) -> Result<(CompletionLeaseAcquisitionId, CompletionCapacityGrantId), ResourceError> {
        if &certificate.operation_key != operation_key {
            return Err(ResourceError::Authority(
                "completion certificate is bound to another resource operation".to_owned(),
            ));
        }
        let grant = self.resource_grant_for_acquisition(&certificate.acquisition)?;
        let acquisition = self
            .completion_leases
            .acquisitions
            .get(&certificate.acquisition)
            .cloned()
            .ok_or_else(|| {
                ResourceError::NotFound("completion acquisition is unavailable".to_owned())
            })?;
        let mut covered_targets = Vec::new();
        let mut participant_grants = Vec::new();
        for grant_id in acquisition.grants.values() {
            let participant = self
                .completion_leases
                .grants
                .get(grant_id)
                .cloned()
                .ok_or_else(|| {
                    ResourceError::InvalidDefinition(
                        "completion acquisition lost a participant grant".to_owned(),
                    )
                })?;
            covered_targets.extend(participant.target_versions.iter().cloned());
            participant_grants.push(participant);
        }
        covered_targets.sort();
        covered_targets.dedup();
        let mut required = required_targets.to_vec();
        required.sort();
        required.dedup();
        if covered_targets != required {
            return Err(ResourceError::Authority(
                "completion participant grants do not exactly cover the irreversible operation targets"
                    .to_owned(),
            ));
        }
        participant_grants.sort_by(|left, right| left.owner_plugin.cmp(&right.owner_plugin));
        for participant in participant_grants {
            self.completion_leases.consume_authoritative_grant(
                certificate,
                &participant.id,
                at,
                &participant.target_versions,
            )?;
        }
        Ok((certificate.acquisition.clone(), grant))
    }

    fn validate_consumed_external_completion_certificate(
        &self,
        certificate: &CompletionLeaseActivationCertificateV1,
        at: SimTime,
        operation_key: &ResourceOperationKey,
        required_targets: &[CompletionLockedTargetV1],
    ) -> Result<(CompletionLeaseAcquisitionId, CompletionCapacityGrantId), ResourceError> {
        let participant = self
            .external_completion_participants
            .grants
            .get(&certificate.acquisition)
            .ok_or_else(|| {
                ResourceError::NotFound(
                    "external completion participant grant is unavailable".to_owned(),
                )
            })?;
        if participant.certificate.as_ref() != Some(certificate)
            || participant.grant.operation_key != certificate.operation_key
            || participant.eligibility_time != at
            || participant.grant.state != CompletionGrantStateV1::Consumed
            || required_targets.iter().any(|target| {
                !participant.grant.target_versions.contains(target)
                    || self
                        .external_completion_participants
                        .target_locks
                        .get(target)
                        != Some(&participant.grant.id)
                    || !certificate.locked_target_versions.contains(target)
            })
        {
            return Err(ResourceError::VersionConflict(
                "external completion certificate does not authorize this exact resource credit"
                    .to_owned(),
            ));
        }
        let _ = operation_key;
        Ok((
            certificate.acquisition.clone(),
            participant.grant.id.clone(),
        ))
    }

    fn active_reserved_for_demand(
        &self,
        demand_id: &ResourceDemandId,
    ) -> Result<u64, ResourceError> {
        self.reservations
            .values()
            .filter(|reservation| {
                &reservation.demand == demand_id && reservation.status == ReservationStatus::Active
            })
            .try_fold(0_u64, |total, reservation| {
                total
                    .checked_add(reservation.quantity)
                    .ok_or(ResourceError::Overflow)
            })
    }

    fn available_for_demand(
        &self,
        account_id: &ResourceAccountId,
        demand: &ResourceDemand,
    ) -> Result<u64, ResourceError> {
        let quantities = self.account_quantities(account_id)?;
        let account = &self.accounts[account_id];
        if demand.protected_floor_policy != account.protected_floor_policy {
            return Err(ResourceError::VersionConflict(
                "resource demand protected-floor policy revision differs from its account"
                    .to_owned(),
            ));
        }
        let may_override = account
            .protected_floor_policy
            .as_ref()
            .zip(demand.protection_override_class.as_ref())
            .and_then(|(policy_id, class)| {
                self.protected_floor_policies
                    .get(policy_id)
                    .map(|policy| policy.override_classes.contains(class))
            })
            .unwrap_or(false);
        if may_override {
            Ok(account.balance.saturating_sub(quantities.reserved))
        } else {
            Ok(quantities.available)
        }
    }

    fn expire_demands(
        &mut self,
        at: SimTime,
        candidate_limit: usize,
        requester: Option<&canwu_api::KnowledgeHolderRef>,
    ) -> Result<(), ResourceError> {
        let expired: Vec<_> = self
            .demand_expiry_index
            .range(..=at)
            .flat_map(|(_, ids)| ids.iter().cloned())
            .filter(|id| {
                requester.is_none_or(|requester| {
                    self.demands
                        .get(id)
                        .is_some_and(|demand| &demand.requester == requester)
                })
            })
            .take(candidate_limit.saturating_add(1))
            .collect();
        if expired.len() > candidate_limit {
            return Err(ResourceError::LimitExceeded(
                "resource demand expiry due-work budget was exceeded".to_owned(),
            ));
        }
        for demand_id in expired {
            let snapshot = self.demands.get(&demand_id).cloned().ok_or_else(|| {
                ResourceError::InvalidDefinition(
                    "resource demand expiry index contains an unavailable demand".to_owned(),
                )
            })?;
            if !matches!(
                snapshot.status,
                DemandStatus::Open | DemandStatus::PartiallyFulfilled
            ) {
                continue;
            }
            let demand = self.demands.get_mut(&demand_id).expect("selected above");
            demand.status = DemandStatus::Expired;
            demand.revision = demand.revision.next()?;
            let reservation_ids = self
                .reservation_by_demand
                .get(&demand_id)
                .cloned()
                .unwrap_or_default();
            for reservation_id in reservation_ids {
                let reservation = self.reservations.get_mut(&reservation_id).ok_or_else(|| {
                    ResourceError::InvalidDefinition(
                        "resource demand reservation index contains an unavailable reservation"
                            .to_owned(),
                    )
                })?;
                if reservation.status != ReservationStatus::Active {
                    continue;
                }
                reservation.status = ReservationStatus::Expired;
                reservation.revision = reservation.revision.next()?;
                if let Some(leg) = self.allocation_legs.get_mut(&reservation.allocation_leg) {
                    leg.status = AllocationLegStatus::Expired;
                    leg.revision = leg.revision.next()?;
                    leg.semantic_digest.clear();
                    leg.semantic_digest =
                        canonical_digest("canwu.resource.allocation-leg.v1", leg)?;
                }
            }
            if let Some(ids) = self.demand_due_index.get_mut(&snapshot.due_at) {
                ids.remove(&demand_id);
            }
            if let Some(ids) = self.demand_expiry_index.get_mut(&snapshot.expires_at) {
                ids.remove(&demand_id);
            }
            self.dirty_demands.remove(&demand_id);
            self.mark_terminal_demand_closure(&demand_id)?;
        }
        self.demand_due_index.retain(|_, ids| !ids.is_empty());
        self.demand_expiry_index.retain(|_, ids| !ids.is_empty());
        Ok(())
    }

    fn validate_account_contract(&self, account: &ResourceAccount) -> Result<(), ResourceError> {
        let definition = self
            .definitions
            .get(&account.resource_revision)
            .ok_or_else(|| {
                ResourceError::NotFound(
                    "resource account definition revision is unavailable".to_owned(),
                )
            })?;
        if !self.units.contains_key(&account.unit_revision)
            || definition.canonical_unit != account.unit_revision
            || account
                .capacity
                .is_some_and(|capacity| account.balance > capacity)
            || account
                .protected_floor_policy
                .as_ref()
                .is_some_and(|policy| !self.protected_floor_policies.contains_key(policy))
        {
            return Err(ResourceError::InvalidDefinition(
                "resource account exact unit, capacity, or protected-floor binding is invalid"
                    .to_owned(),
            ));
        }
        Ok(())
    }

    fn validate_demand_contract(&self, demand: &ResourceDemand) -> Result<(), ResourceError> {
        if demand.requested == 0
            || demand.minimum_useful == 0
            || demand.minimum_useful > demand.requested
            || demand.due_at >= demand.expires_at
            || !self.definitions.contains_key(&demand.resource_revision)
            || !self.units.contains_key(&demand.unit_revision)
            || demand
                .protected_floor_policy
                .as_ref()
                .is_some_and(|policy| !self.protected_floor_policies.contains_key(policy))
        {
            return Err(ResourceError::InvalidDefinition(
                "resource demand quantity, interval, or exact revision binding is invalid"
                    .to_owned(),
            ));
        }
        Ok(())
    }

    fn next_sequence(&mut self) -> Result<u64, ResourceError> {
        let current = self.next_admitted_sequence;
        self.next_admitted_sequence = current.checked_add(1).ok_or(ResourceError::Overflow)?;
        Ok(current)
    }

    #[allow(clippy::too_many_lines)]
    pub fn validate(&self) -> Result<(), ResourceError> {
        if self.format_version != RESOURCE_STATE_FORMAT_VERSION {
            return Err(ResourceError::InvalidDefinition(
                "resource runtime format version is unsupported".to_owned(),
            ));
        }
        self.limits.validate()?;
        let active_demands = self
            .demands
            .values()
            .filter(|demand| {
                matches!(
                    demand.status,
                    DemandStatus::Open | DemandStatus::PartiallyFulfilled
                )
            })
            .count();
        let active_reservations = self
            .reservations
            .values()
            .filter(|reservation| reservation.status == ReservationStatus::Active)
            .count();
        let active_allocation_legs = self
            .allocation_legs
            .values()
            .filter(|leg| leg.status == AllocationLegStatus::Reserved)
            .count();
        if self.definitions.len() > self.limits.max_definitions
            || self.units.len() > self.limits.max_unit_revisions
            || self.accounts.len() > self.limits.max_accounts
            || active_demands > self.limits.max_demands
            || active_reservations > self.limits.max_demands
            || active_allocation_legs > self.limits.max_demands
            || self.active_transfers.len() > self.limits.max_transfers
            || self.outcomes.len() > self.limits.max_operation_outcomes
            || self.observation_heads.len() > self.limits.max_observation_heads
            || self.dirty_demands.len() > self.limits.max_dirty_demands
            || self.terminal_archive_candidates.len() > self.limits.max_archive_candidates
            || self.archive_maintenance_receipts.len()
                > self.limits.max_archive_maintenance_receipts
        {
            return Err(ResourceError::LimitExceeded(
                "resource authoritative records exceed configured limits".to_owned(),
            ));
        }
        for (id, account) in &self.accounts {
            if id != &account.id {
                return Err(ResourceError::InvalidDefinition(
                    "resource account map key differs from its identity".to_owned(),
                ));
            }
            self.validate_account_contract(account)?;
            let quantities = self.account_quantities(id)?;
            if quantities.reserved > account.balance {
                return Err(ResourceError::Conservation(
                    "resource reservations exceed the authoritative account balance".to_owned(),
                ));
            }
        }
        for (id, demand) in &self.demands {
            if id != &demand.id || demand.fulfilled > demand.requested {
                return Err(ResourceError::InvalidDefinition(
                    "resource demand identity or fulfillment is invalid".to_owned(),
                ));
            }
            self.validate_demand_contract(demand)?;
        }
        let expected_due: BTreeSet<_> = self
            .demands
            .values()
            .filter(|demand| {
                matches!(
                    demand.status,
                    DemandStatus::Open | DemandStatus::PartiallyFulfilled
                )
            })
            .map(|demand| demand.id.clone())
            .collect();
        let indexed_due: BTreeSet<_> = self
            .demand_due_index
            .values()
            .flat_map(|ids| ids.iter().cloned())
            .collect();
        let indexed_expiry: BTreeSet<_> = self
            .demand_expiry_index
            .values()
            .flat_map(|ids| ids.iter().cloned())
            .collect();
        let due_entry_count = self
            .demand_due_index
            .values()
            .map(BTreeSet::len)
            .sum::<usize>()
            .checked_add(
                self.demand_expiry_index
                    .values()
                    .map(BTreeSet::len)
                    .sum::<usize>(),
            )
            .ok_or(ResourceError::Overflow)?;
        if indexed_due != expected_due
            || indexed_expiry != expected_due
            || due_entry_count > self.limits.max_due_entries
            || self.demand_due_index.iter().any(|(at, ids)| {
                ids.iter().any(|id| {
                    self.demands
                        .get(id)
                        .is_none_or(|demand| demand.due_at != *at)
                })
            })
            || self.demand_expiry_index.iter().any(|(at, ids)| {
                ids.iter().any(|id| {
                    self.demands
                        .get(id)
                        .is_none_or(|demand| demand.expires_at != *at)
                })
            })
            || self
                .dirty_demands
                .iter()
                .any(|id| !expected_due.contains(id))
        {
            return Err(ResourceError::InvalidDefinition(
                "resource bounded demand due/expiry/dirty indexes differ from hot state".to_owned(),
            ));
        }
        let mut active_leg_reservations = BTreeSet::new();
        for (id, reservation) in &self.reservations {
            let leg = self.allocation_legs.get(&reservation.allocation_leg);
            if id != &reservation.id
                || leg.is_none() && reservation.status == ReservationStatus::Active
                || leg.is_some_and(|leg| {
                    leg.reservation != *id
                        || leg.account != reservation.account
                        || leg.demand != reservation.demand
                        || leg.quantity != reservation.quantity
                        || (reservation.status == ReservationStatus::Active
                            && leg.status != AllocationLegStatus::Reserved)
                })
            {
                return Err(ResourceError::InvalidDefinition(
                    "resource reservation/allocation closure is invalid".to_owned(),
                ));
            }
            if reservation.status == ReservationStatus::Active
                && !active_leg_reservations.insert(
                    leg.expect("active reservation leg was checked above")
                        .id
                        .clone(),
                )
            {
                return Err(ResourceError::Conservation(
                    "resource allocation leg is reserved more than once".to_owned(),
                ));
            }
        }
        let expected_reservation_index: BTreeMap<_, BTreeSet<_>> =
            self.reservations
                .values()
                .fold(BTreeMap::new(), |mut index, reservation| {
                    index
                        .entry(reservation.demand.clone())
                        .or_insert_with(BTreeSet::new)
                        .insert(reservation.id.clone());
                    index
                });
        if self.reservation_by_demand != expected_reservation_index {
            return Err(ResourceError::InvalidDefinition(
                "resource reservation-by-demand index differs from hot state".to_owned(),
            ));
        }
        let expected_active_transfers: BTreeSet<_> = self
            .transfers
            .values()
            .filter(|transfer| transfer.escrow > 0)
            .map(|transfer| transfer.id.clone())
            .collect();
        if self.active_transfers != expected_active_transfers {
            return Err(ResourceError::InvalidDefinition(
                "resource active-transfer index differs from escrow state".to_owned(),
            ));
        }
        for (id, transfer) in &self.transfers {
            if id != &transfer.id
                || u128::from(transfer.quantity)
                    != u128::from(transfer.escrow)
                        + u128::from(transfer.accepted)
                        + u128::from(transfer.lost)
                        + u128::from(transfer.returned)
                        + u128::from(transfer.external_outflow)
                || (transfer.state == ResourceTransferState::Cancelled && transfer.escrow != 0)
                || (matches!(
                    transfer.state,
                    ResourceTransferState::Accepted
                        | ResourceTransferState::Lost
                        | ResourceTransferState::ExternalOutflowSettled
                        | ResourceTransferState::Returned
                        | ResourceTransferState::Cancelled
                ) && transfer.escrow != 0)
            {
                return Err(ResourceError::Conservation(
                    "resource transfer escrow partition is invalid".to_owned(),
                ));
            }
        }
        for (operation_key, outcome) in &self.outcomes {
            if operation_key != &outcome.operation_key {
                return Err(ResourceError::InvalidDefinition(
                    "resource operation outcome map key differs from its identity".to_owned(),
                ));
            }
            outcome.validate()?;
        }
        for consumption in self.consumptions.values() {
            let mut detached = consumption.clone();
            let digest = std::mem::take(&mut detached.semantic_digest);
            if digest != canonical_digest("canwu.resource.consumption.v1", &detached)? {
                return Err(ResourceError::InvalidDefinition(
                    "resource consumption semantic digest is forged".to_owned(),
                ));
            }
        }
        for fulfillment in self.fulfillments.values() {
            let mut detached = fulfillment.clone();
            let digest = std::mem::take(&mut detached.semantic_digest);
            if digest != canonical_digest("canwu.resource.fulfillment.v1", &detached)? {
                return Err(ResourceError::InvalidDefinition(
                    "resource fulfillment semantic digest is forged".to_owned(),
                ));
            }
        }
        if self.report_grants.len() > ResourceLimitsV1::MAX_HOLDERS
            || self.observation_head_by_grant.len() != self.observation_heads.len()
            || self
                .report_dirty_grants
                .iter()
                .chain(self.report_due_index.values().flatten())
                .any(|grant| !self.report_grants.contains_key(grant))
            || self
                .report_cursor
                .as_ref()
                .is_some_and(|grant| !self.report_grants.contains_key(grant))
            || self
                .report_due_index
                .values()
                .map(BTreeSet::len)
                .sum::<usize>()
                > ResourceLimitsV1::MAX_HOLDERS
        {
            return Err(ResourceError::LimitExceeded(
                "resource holder grant or observation-head capacity is invalid".to_owned(),
            ));
        }
        for grant in self.report_grants.values() {
            if grant.confidence_per_mille > 1_000 || grant.cadence_minutes == 0 {
                return Err(ResourceError::InvalidDefinition(
                    "resource report grant cadence or confidence is invalid".to_owned(),
                ));
            }
        }
        for (acquisition, grants) in &self.completion_report_reservations {
            let holder = self
                .completion_leases
                .acquisitions
                .get(acquisition)
                .map(|value| &value.holder)
                .or_else(|| {
                    self.external_completion_participants
                        .participant(acquisition)
                        .map(|value| &value.holder)
                })
                .ok_or_else(|| {
                    ResourceError::InvalidDefinition(
                        "completion report reservation is orphaned".to_owned(),
                    )
                })?;
            if grants.is_empty()
                || grants.iter().any(|grant_id| {
                    self.report_grants
                        .get(grant_id)
                        .is_none_or(|grant| &grant.holder != holder)
                })
            {
                return Err(ResourceError::InvalidDefinition(
                    "completion report reservation is not bound to its exact holder grant"
                        .to_owned(),
                ));
            }
        }
        if self.completion_report_ready.keys().any(|acquisition| {
            !self
                .completion_report_reservations
                .contains_key(acquisition)
        }) {
            return Err(ResourceError::InvalidDefinition(
                "completion report readiness is orphaned".to_owned(),
            ));
        }
        for (grant_id, head_id) in &self.observation_head_by_grant {
            let grant = self.report_grants.get(grant_id).ok_or_else(|| {
                ResourceError::InvalidDefinition(
                    "resource observation head references an unavailable holder grant".to_owned(),
                )
            })?;
            let head = self.observation_heads.get(head_id).ok_or_else(|| {
                ResourceError::InvalidDefinition(
                    "resource observation head index is broken".to_owned(),
                )
            })?;
            head.validate()?;
            if head.grant != *grant_id
                || head.holder != grant.holder
                || !head.source_versions.contains(&head.provider_source)
                || head
                    .stock
                    .iter()
                    .any(|value| !grant.accounts.contains(&value.account))
                || head.stock.iter().any(|value| {
                    self.accounts
                        .get(&value.account)
                        .and_then(|account| self.definitions.get(&account.resource_revision))
                        .is_none_or(|definition| definition.scope != value.scope)
                })
                || head
                    .demands
                    .iter()
                    .any(|value| !grant.demands.contains(&value.demand))
            {
                return Err(ResourceError::Authority(
                    "resource observation head exceeds or differs from its holder grant".to_owned(),
                ));
            }
            self.validate_observation_head_authority(head, grant)?;
        }
        self.run_budget.validate()?;
        self.completion_leases.validate(&self.run_budget)?;
        self.external_completion_participants
            .validate(&self.run_budget)?;
        let mut archive_head = self.archive_head.clone();
        let archive_digest = std::mem::take(&mut archive_head.semantic_digest);
        if archive_digest != canonical_digest("canwu.resource.archive-head.v1", &archive_head)? {
            return Err(ResourceError::InvalidDefinition(
                "resource archive head semantic digest is forged".to_owned(),
            ));
        }
        for (sequence, key) in &self.terminal_archive_candidates {
            let terminal = self.terminal_archive_record(key, *sequence)?;
            if terminal.terminal_sequence != *sequence {
                return Err(ResourceError::InvalidDefinition(
                    "resource terminal archive index sequence differs".to_owned(),
                ));
            }
        }
        for (sequence, receipt) in &self.archive_maintenance_receipts {
            let mut detached = receipt.clone();
            let digest = std::mem::take(&mut detached.semantic_digest);
            if sequence != &receipt.sequence
                || digest
                    != canonical_digest("canwu.resource.archive-maintenance-receipt.v1", &detached)?
            {
                return Err(ResourceError::InvalidDefinition(
                    "resource archive maintenance receipt is forged".to_owned(),
                ));
            }
        }
        for (id, handle) in &self.archive_retention_handles {
            let mut detached = handle.clone();
            let digest = std::mem::take(&mut detached.semantic_digest);
            if id != &handle.id
                || digest != canonical_digest("canwu.resource.archive-retention.v1", &detached)?
            {
                return Err(ResourceError::InvalidDefinition(
                    "resource archive retention handle is forged".to_owned(),
                ));
            }
        }
        if self.continuation != self.computed_continuation() {
            return Err(ResourceError::InvalidDefinition(
                "resource payload-required evidence continuation differs from active work"
                    .to_owned(),
            ));
        }
        let encoded = serde_json::to_vec(self).map_err(|error| {
            ResourceError::InvalidDefinition(format!(
                "resource authoritative state cannot be encoded: {error}"
            ))
        })?;
        if encoded.len() > self.limits.max_state_bytes {
            return Err(ResourceError::LimitExceeded(
                "resource authoritative state byte cap is exhausted".to_owned(),
            ));
        }
        self.validate_conservation()
    }

    pub fn validate_conservation(&self) -> Result<(), ResourceError> {
        let balances = self.accounts.values().try_fold(0_u128, |total, account| {
            total
                .checked_add(u128::from(account.balance))
                .ok_or(ResourceError::Overflow)
        })?;
        let escrow = self
            .transfers
            .values()
            .try_fold(0_u128, |total, transfer| {
                total
                    .checked_add(u128::from(transfer.escrow))
                    .ok_or(ResourceError::Overflow)
            })?;
        let closing = balances
            .checked_add(escrow)
            .ok_or(ResourceError::Overflow)?;
        let sources = self
            .conservation
            .opening_balances
            .checked_add(self.conservation.opening_active_escrow)
            .and_then(|value| value.checked_add(self.conservation.admitted_production))
            .and_then(|value| value.checked_add(self.conservation.external_inflow))
            .ok_or(ResourceError::Overflow)?;
        let sinks = self
            .conservation
            .admitted_consumption
            .checked_add(self.conservation.admitted_loss)
            .and_then(|value| value.checked_add(self.conservation.external_outflow))
            .ok_or(ResourceError::Overflow)?;
        if closing.checked_add(sinks).ok_or(ResourceError::Overflow)? != sources {
            return Err(ResourceError::Conservation(
                "closing balances plus active escrow do not reconcile with admitted sources and sinks"
                    .to_owned(),
            ));
        }
        Ok(())
    }
}

fn credit_account(account: &mut ResourceAccount, quantity: u64) -> Result<(), ResourceError> {
    let balance = account
        .balance
        .checked_add(quantity)
        .ok_or(ResourceError::Overflow)?;
    if account.capacity.is_some_and(|capacity| balance > capacity) {
        return Err(ResourceError::Capacity(
            "resource account capacity would be exceeded".to_owned(),
        ));
    }
    account.balance = balance;
    account.revision = account.revision.next()?;
    Ok(())
}

fn debit_account(account: &mut ResourceAccount, quantity: u64) -> Result<(), ResourceError> {
    if quantity == 0 || quantity > account.balance {
        return Err(ResourceError::Conservation(
            "resource account debit exceeds its authoritative balance".to_owned(),
        ));
    }
    account.balance -= quantity;
    account.revision = account.revision.next()?;
    Ok(())
}

fn outcome_id(
    operation_key: &ResourceOperationKey,
    request_digest: &str,
) -> Result<ResourceOperationOutcomeId, ResourceError> {
    crate::canonical_operation_outcome_id(operation_key, request_digest)
}

fn request_remainder(state: &ResourceState, request: &ResourceOperationRequestV1) -> u64 {
    match request {
        ResourceOperationRequestV1::SubmitDemand(value) => value.demand.requested,
        ResourceOperationRequestV1::AmendDemand(value) => value.replacement.remainder(),
        ResourceOperationRequestV1::Consume(value) => value.allocation.quantity,
        ResourceOperationRequestV1::BeginTransfer(value) => value.allocation.quantity,
        ResourceOperationRequestV1::Credit(value) => value.quantity,
        ResourceOperationRequestV1::ExternalOutflow(value) => value.quantity,
        ResourceOperationRequestV1::CancelDemand(value) => state
            .demands
            .get(&value.demand)
            .map_or(0, ResourceDemand::remainder),
        ResourceOperationRequestV1::CreateAccount(_)
        | ResourceOperationRequestV1::Allocate(_)
        | ResourceOperationRequestV1::AdvanceTransfer(_)
        | ResourceOperationRequestV1::CancelTransfer(_)
        | ResourceOperationRequestV1::CompleteTransfer(_)
        | ResourceOperationRequestV1::SetProtectedFloor(_)
        | ResourceOperationRequestV1::RecordObservation(_)
        | ResourceOperationRequestV1::Completion(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use canwu_api::{
        DomainRecordRef, DomainRecordVersionRef, DomainRecordVersionSource, KnowledgeHolderRef,
        PersonId, SimTime,
    };

    use super::*;

    #[test]
    fn externally_revalidated_prepare_is_a_stable_rejection() {
        let holder = KnowledgeHolderRef::Person(PersonId::new(1));
        let mut state = ResourceState::empty(ResourceLimitsV1::canonical()).expect("state");
        state
            .install_run_budget(
                crate::RunBudgetRevisionV1 {
                    revision: ResourceRevision::INITIAL,
                    total_completion_units: 100_000,
                    shared_pending_slots: 1,
                    partitions: vec![crate::CompletionCapacityPartitionV1 {
                        authority: holder.clone(),
                        operation_namespace: "test.resource".to_owned(),
                        guaranteed_units: 100_000,
                        reserved_pending_slots: 1,
                        maximum_burst_units: 0,
                        request_token_capacity: 1,
                        request_token_refill_minutes: 1,
                        reacquire_cooldown_minutes: 1,
                        root_acquisition_cap_per_sim_time: 1,
                        guaranteed_max_wait_boundaries: 1,
                    }],
                    semantic_digest: String::new(),
                }
                .seal()
                .expect("budget"),
            )
            .expect("install budget");
        let acquisition =
            crate::CompletionLeaseAcquisitionId::new("test:lease:external-prepare-revalidation")
                .expect("acquisition");
        let grant =
            crate::CompletionCapacityGrantId::new("test:grant:external-prepare-revalidation")
                .expect("grant");
        let target = DomainRecordVersionRef {
            record: DomainRecordRef::new("test.provider", "record", "target"),
            version: 1,
            established_by: DomainRecordVersionSource::InitialScenario,
        };
        let envelope = crate::EligibilityEnvelopeV1::new(
            vec![target.clone()],
            BTreeMap::new(),
            BTreeSet::new(),
            Vec::new(),
            Vec::new(),
        )
        .expect("envelope");
        state
            .apply_operation(&ResourceOperationRequestV1::Completion(
                ResourceCompletionOperationV1::Acquire(crate::RequestCompletionLeaseV1 {
                    id: acquisition.clone(),
                    operation_key: ResourceOperationKey::new(
                        "test:operation:external-prepare-revalidation",
                    )
                    .expect("operation"),
                    holder,
                    operation_namespace: "test.resource".to_owned(),
                    eligibility_time: SimTime::EPOCH,
                    eligibility_envelope: envelope.clone(),
                    recipe: crate::CompletionCapacityRecipeV1 {
                        receipts: crate::MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE,
                        mutations: 1,
                        reports_per_holder: 0,
                        holders: 0,
                        bytes: 1_024,
                    },
                    expected_participants: BTreeSet::from([crate::PLUGIN_NAME.to_owned()]),
                    policy_class: crate::CompletionPolicyClassV1::Guaranteed,
                }),
            ))
            .expect("acquire");
        state
            .apply_operation(&ResourceOperationRequestV1::Completion(
                ResourceCompletionOperationV1::Grant(crate::GrantCompletionCapacityV1 {
                    grant_id: grant.clone(),
                    acquisition: acquisition.clone(),
                    expected_acquisition_revision: state.completion_leases.acquisitions
                        [&acquisition]
                        .revision,
                    owner_plugin: crate::PLUGIN_NAME.to_owned(),
                    target_versions: vec![crate::CompletionLockedTargetV1::ExternalRecord {
                        version: target,
                    }],
                    current_boundary: 1,
                }),
            ))
            .expect("grant");
        let outcome = state
            .apply_prepare_with_external_revalidation(
                &crate::PrepareCompletionCapacityV1 {
                    acquisition: acquisition.clone(),
                    expected_acquisition_revision: state.completion_leases.acquisitions
                        [&acquisition]
                        .revision,
                    grant: grant.clone(),
                    expected_grant_revision: state.completion_leases.grants[&grant].revision,
                    current_boundary: 2,
                    eligibility_envelope_digest: envelope.digest,
                },
                false,
            )
            .expect("stable rejection");
        assert_eq!(outcome.status, ResourceOperationStatus::Applied);
        assert_eq!(
            state.completion_leases.grants[&grant].state,
            crate::CompletionGrantStateV1::Rejected
        );
        assert_eq!(
            state.completion_leases.acquisitions[&acquisition].state,
            crate::CompletionLeaseAcquisitionStateV1::Aborting
        );
    }
}
