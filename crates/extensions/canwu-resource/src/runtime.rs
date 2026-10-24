use crate::{
    AllocationLegStatus, CompletionCapacityGrantId, CompletionGrantStateV1,
    CompletionLeaseAcquisitionId, CompletionLeaseActivationCertificateV1,
    CompletionLeaseReceiptActionV1, CompletionLockedTargetV1, ConservationTotalsV1,
    ConsumptionStatus, DemandStatus, FulfillmentStatus, PLUGIN_NAME, PartialFulfillmentPolicy,
    ProtectedFloorPolicyRevision, ProtectedFloorPolicyRevisionId, ReservationStatus,
    ResourceAccount, ResourceAccountId, ResourceAllocationLeg, ResourceAllocationLegId,
    ResourceAllocationLegVersionV1, ResourceConsumption, ResourceConsumptionId,
    ResourceConsumptionVersionV1, ResourceDefinitionRevision, ResourceDefinitionRevisionId,
    ResourceDemand, ResourceDemandId, ResourceDemandSourcePolicyV1, ResourceError,
    ResourceFulfillment, ResourceFulfillmentId, ResourceLimitsV1, ResourceLoss, ResourceLossId,
    ResourceOperationKey, ResourceOperationKind, ResourceOperationOutcome,
    ResourceOperationOutcomeId, ResourceOperationStatus, ResourceRecordRefV1,
    ResourceReportGrantV1, ResourceReservation, ResourceReservationId, ResourceRevision,
    ResourceState, ResourceTransfer, ResourceTransferId, ResourceTransferState,
    ResourceUnitRevision, ResourceUnitRevisionId, TransportExecutionLink, canonical_digest,
};
use canwu_api::{DomainRecordVersionRef, EvidenceRef, SimTime};
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

/// Format of the resource root ([`ResourceState`]). 6 since 0.20.0: the live
/// root declares identity-only evidence dependencies
/// (`canwu_identity_evidence_dependencies`), and so does each bucket.
pub const RESOURCE_STATE_FORMAT_VERSION: u32 = 6;

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

/// One item of a [`ResourceConsumptionBatchRequestV1`]: one exact allocation
/// leg to consume, with the demand it belongs to.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceConsumptionBatchItemV1 {
    pub consumption_id: ResourceConsumptionId,
    pub allocation: ResourceAllocationLegVersionV1,
    /// The demand the leg was allocated to, at the revision the leg records.
    pub demand: ResourceDemandId,
    pub demand_revision: ResourceRevision,
    pub expected_account_revision: ResourceRevision,
}

/// Several exact allocation-leg consumptions under one completion lease, with
/// one outcome per item.
///
/// The lease certifies the batch as one operation: its certificate binds
/// `operation_key` and locks exactly [`Self::locked_targets`], every item's
/// account, allocation leg, and demand and the one consumer evidence. The
/// batch settles item by item. An item that fails its checks (a stale leg,
/// account, or demand revision, a leg that is no longer reserved, a
/// consumption identity that exists, a granted leg whose grant is not
/// current) is refused alone: its outcome, under
/// [`Self::item_operation_key`], records the refusal and the root keeps its
/// leg reserved. Every other item consumes its leg and records an applied
/// outcome. Several items may draw on one account: an item that expects the
/// revision the batch found draws on the account as the batch's earlier
/// items left it. The batch outcome under `operation_key` totals the
/// consumed quantity (`quantity`) and the quantity of the refused items
/// (`remainder`), and the lease completes once, with one receipt, whatever
/// the items did: a batch whose every item is refused is applied with
/// quantity zero, and its lease finishes and leaves the root.
///
/// A refusal of the batch itself (an empty batch, more items than
/// [`MAX_RESOURCE_CONSUMPTION_BATCH_ITEMS`] or than
/// `max_batch_items_per_boundary`, a certificate that does not bind exactly
/// these targets, an item key already recorded) is one rejected batch
/// outcome; it records no item outcome and leaves the lease active. A batch
/// key under a reserved prefix is refused by the ingress call, and skipped
/// with `reserved_operation_key` if it reaches settlement. Room is measured
/// on the batch's whole result: a batch whose records do not fit the root
/// records nothing and is skipped, and its lease stays active
/// ([`crate::ResourceState::consumption_batch_can_fit`] tells a provider
/// what can ever fit).
///
/// A batch enters through the adapter ingress of its provider, which
/// authorizes every item with a typed consumption intent whose operation key
/// is the item's key.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceConsumptionBatchRequestV1 {
    pub operation_key: ResourceOperationKey,
    pub items: Vec<ResourceConsumptionBatchItemV1>,
    pub consumer_evidence: DomainRecordVersionRef,
    pub at: SimTime,
    pub completion_certificate: CompletionLeaseActivationCertificateV1,
}

/// Most items one consumption batch carries.
pub const MAX_RESOURCE_CONSUMPTION_BATCH_ITEMS: usize = 256;

/// Hot terminal records one consumption batch item leaves when it consumes
/// the one leg of its demand: its outcome, consumption, and fulfillment, and
/// the demand it finishes with that leg and its reservation. A leg that
/// finishes a demand of several legs also leaves the demand's other legs and
/// reservations; a refused item leaves its outcome only.
pub const RESOURCE_CONSUMPTION_BATCH_ITEM_RECORDS: usize = 6;

/// Hot terminal records a consumption batch leaves beside its items: the
/// batch outcome and its lease's completion receipt.
pub const RESOURCE_CONSUMPTION_BATCH_RECORDS: usize = 2;

/// Prefix of the operation keys of consumption batch items. An operation key
/// under it is reserved for the item outcomes of a batch: any request that
/// names one is refused.
pub const RESOURCE_CONSUMPTION_BATCH_ITEM_KEY_PREFIX: &str = "resource:consume-batch:";

/// The operation key of item `index` of the consumption batch whose key is
/// `batch`: what [`ResourceConsumptionBatchRequestV1::item_operation_key`]
/// returns, for a provider that authorizes the items before the batch is
/// built.
#[must_use]
pub fn resource_consumption_batch_item_key(
    batch: &ResourceOperationKey,
    index: usize,
) -> ResourceOperationKey {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"canwu.resource.consume-batch-item.v1");
    hasher.update(&[0]);
    hasher.update(batch.as_str().as_bytes());
    let digest = hasher.finalize().to_hex();
    ResourceOperationKey::new(format!(
        "{RESOURCE_CONSUMPTION_BATCH_ITEM_KEY_PREFIX}{}:{index}",
        &digest[..32]
    ))
    .expect("a digest and an index always form a valid operation key")
}

impl ResourceConsumptionBatchRequestV1 {
    /// The operation key of item `index`:
    /// `resource:consume-batch:{digest}:{index}`, where the digest names the
    /// batch's operation key. It is reserved, so only this batch records an
    /// outcome under it, and a batch key is used once, so item keys are
    /// unique for the run.
    #[must_use]
    pub fn item_operation_key(&self, index: usize) -> ResourceOperationKey {
        resource_consumption_batch_item_key(&self.operation_key, index)
    }

    /// The exact targets the batch's completion lease locks: every item's
    /// account at its expected revision, allocation leg, and demand, and the
    /// consumer evidence, sorted and without duplicates.
    #[must_use]
    pub fn locked_targets(&self) -> Vec<CompletionLockedTargetV1> {
        let mut targets: Vec<_> = self
            .items
            .iter()
            .flat_map(|item| {
                [
                    CompletionLockedTargetV1::Account {
                        id: item.allocation.account.clone(),
                        revision: item.expected_account_revision,
                    },
                    CompletionLockedTargetV1::AllocationLeg {
                        id: item.allocation.id.clone(),
                        revision: item.allocation.revision,
                    },
                    CompletionLockedTargetV1::Demand {
                        id: item.demand.clone(),
                        revision: item.demand_revision,
                    },
                ]
            })
            .chain(std::iter::once(CompletionLockedTargetV1::ExternalRecord {
                version: self.consumer_evidence.clone(),
            }))
            .collect();
        targets.sort();
        targets.dedup();
        targets
    }
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

/// Atomic start of two transfer legs. Both legs are admitted together or the
/// whole exchange is rejected with one outcome, keyed by `operation_key`, that
/// cites both transfer IDs.
///
/// Each leg is an ordinary [`ResourceTransferStartRequestV1`] with its own
/// completion certificate, and that certificate must be a completion lease
/// held by the custodian of the leg's source account. A leg's `operation_key`
/// must equal the key that [`ResourceExchangeTermsV1::leg_operation_keys`]
/// derives from the exchange terms, so each custodian's lease consents to the
/// whole exchange (both transfers, allocations, and destinations) rather than
/// to its own leg alone. The leg key becomes the transfer's operation
/// identity; only the exchange key receives an operation outcome. Because each
/// leg keeps its own lease, later terminal dispositions of the two transfers
/// remain independent.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceExchangeStartRequestV1 {
    pub operation_key: ResourceOperationKey,
    pub leg_a: ResourceTransferStartRequestV1,
    pub leg_b: ResourceTransferStartRequestV1,
}

impl ResourceExchangeStartRequestV1 {
    /// The consent-bearing terms of this exchange.
    #[must_use]
    pub fn terms(&self) -> ResourceExchangeTermsV1 {
        ResourceExchangeTermsV1 {
            operation_key: self.operation_key.clone(),
            leg_a: ResourceExchangeLegTermsV1::from(&self.leg_a),
            leg_b: ResourceExchangeLegTermsV1::from(&self.leg_b),
        }
    }
}

/// What one exchange leg moves: the transfer identity, the exact allocation
/// it debits, and its destination. Timing and the expected source revision
/// are bound by the leg's own completion lease instead.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceExchangeLegTermsV1 {
    pub transfer_id: ResourceTransferId,
    pub allocation: ResourceAllocationLegVersionV1,
    pub destination: Option<ResourceAccountId>,
}

impl From<&ResourceTransferStartRequestV1> for ResourceExchangeLegTermsV1 {
    fn from(value: &ResourceTransferStartRequestV1) -> Self {
        Self {
            transfer_id: value.transfer_id.clone(),
            allocation: value.allocation.clone(),
            destination: value.destination.clone(),
        }
    }
}

/// Terms both parties agree to before either acquires its leg's completion
/// lease.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceExchangeTermsV1 {
    pub operation_key: ResourceOperationKey,
    pub leg_a: ResourceExchangeLegTermsV1,
    pub leg_b: ResourceExchangeLegTermsV1,
}

impl ResourceExchangeTermsV1 {
    /// Canonical digest of the terms.
    pub fn digest(&self) -> Result<String, ResourceError> {
        canonical_digest("canwu.resource.exchange-terms.v1", self)
    }

    /// Operation keys that `leg_a` and `leg_b`, and their completion leases,
    /// must use: `resource:exchange-leg:{a|b}:{terms digest}`.
    pub fn leg_operation_keys(&self) -> Result<[ResourceOperationKey; 2], ResourceError> {
        let digest = self.digest()?;
        Ok([
            ResourceOperationKey::new(format!("resource:exchange-leg:a:{digest}"))?,
            ResourceOperationKey::new(format!("resource:exchange-leg:b:{digest}"))?,
        ])
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferProgressV1 {
    InTransit,
    ArrivalPending,
    ReturnPending,
}

/// Advances a transfer along its transport execution and records the link
/// in the transfer's ordered link history
/// ([`crate::ResourceTransfer::link_history`]).
///
/// A transfer advances from `PendingDispatch` to `InTransit` (its
/// dispatch), from `InTransit` to `InTransit` (an intermediate leg or
/// handoff), from `InTransit` to `ArrivalPending`, and from `PendingDispatch`,
/// `InTransit`, or `ArrivalPending` to `ReturnPending`. An intermediate
/// advance names the same execution as the current link and either a new
/// leg, not lower than the current one, or a handoff the history does not
/// record yet; any other is a recorded `invalid_lifecycle` rejection. The
/// history holds at most [`crate::MAX_RESOURCE_TRANSFER_LINKS`] entries, two
/// of them kept for the arrival and the return, so an intermediate advance
/// past fourteen in-transit links is a recorded `limit_exceeded` rejection,
/// while an arrival or a return is always admitted.
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
    /// Same-place acceptance without a transport execution. Valid only while
    /// the transfer is `PendingDispatch` with no transport link and when the
    /// source and destination accounts declare the same
    /// [`crate::ResourceAccount::place_scope`]. The exact handover record is
    /// locked by the terminal completion certificate and kept as evidence.
    AcceptLocal {
        destination: ResourceAccountId,
        expected_destination_revision: ResourceRevision,
        handover_evidence: DomainRecordVersionRef,
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

/// Which terminal disposition a [`ResourceTransferDispositionV1`] is, without
/// its data: what a lease binding names
/// ([`crate::CompletionOperationBindingV1::disposition`]).
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceTransferDispositionKindV1 {
    Accept,
    AcceptLocal,
    Lose,
    Return,
    ExternalOutflow,
}

impl ResourceTransferDispositionV1 {
    #[must_use]
    pub const fn kind(&self) -> ResourceTransferDispositionKindV1 {
        match self {
            Self::Accept { .. } => ResourceTransferDispositionKindV1::Accept,
            Self::AcceptLocal { .. } => ResourceTransferDispositionKindV1::AcceptLocal,
            Self::Lose { .. } => ResourceTransferDispositionKindV1::Lose,
            Self::Return { .. } => ResourceTransferDispositionKindV1::Return,
            Self::ExternalOutflow { .. } => ResourceTransferDispositionKindV1::ExternalOutflow,
        }
    }
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

/// Where credited stock comes from. The applied credit's outcome keeps the
/// cited version as its exact evidence, on every path.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "source", content = "evidence", rename_all = "snake_case")]
pub enum ResourceCreditSourceV1 {
    /// Output of a production execution. It settles only through the
    /// production output batch ingress.
    Production(DomainRecordVersionRef),
    /// Value from outside the modelled accounts, the counterpart of
    /// [`ResourceExternalOutflowRequestV1`], proven by an exact record
    /// version. A tracked command from the account's custodian may carry it,
    /// under a completion lease (the single step the host application
    /// queues) that locks the account and this version; canonical adapter
    /// ingress may carry it with this version as its provider source.
    ExternalInflow(DomainRecordVersionRef),
}

/// Credit of one account: production output, or an external inflow. The
/// production source record body remains payload-required until the
/// returned exact outcome is acknowledged.
///
/// When the certificate belongs to an external completion participant, the
/// participant locked the coordinator's runtime record at the version current
/// when the lease was granted. A `Production` source must be that record at
/// the locked version or later (the version proving the completion); the
/// production output batch ingress further requires it to equal the source
/// the coordinator pinned on the execution.
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

/// Account-level loss settled in place, without a transfer. The debit
/// respects reservations always and the account's protected floor unless
/// `allow_protected` is set. The settled [`crate::ResourceLoss`] carries
/// `account: Some(..)`, `transfer: None`, and counts as admitted loss in
/// [`crate::ConservationTotalsV1`].
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceAccountLossRequestV1 {
    pub operation_key: ResourceOperationKey,
    pub loss_id: ResourceLossId,
    pub account: ResourceAccountId,
    pub expected_account_revision: ResourceRevision,
    pub quantity: u64,
    pub cause: EvidenceRef,
    pub allow_protected: bool,
    pub at: SimTime,
    pub completion_certificate: CompletionLeaseActivationCertificateV1,
}

/// Loss of part of an open transfer's escrow.
///
/// The transfer keeps its state and the rest of its escrow: `quantity`
/// moves from `escrow` to `lost`, the transfer revision advances, and a
/// [`crate::ResourceLoss`] with `transfer: Some(..)` counts as admitted loss
/// in [`crate::ConservationTotalsV1`]. It is allowed in every state that is
/// not terminal. `quantity` must be above zero and below the escrow: a loss
/// of the whole escrow ends the transfer, and settles as `CompleteTransfer`
/// with the `Lose` disposition. The demand of the transfer's allocation is
/// credited only with what is accepted, so a loss reopens that much of it.
///
/// The completion certificate locks the transfer at
/// `expected_transfer_revision` and, for a domain-record cause, that record
/// version. The loss completes this certificate's lease, never the lease the
/// transfer was started under. A tracked command must come from the
/// transfer's controller, the source custodian or, for a transfer started
/// under an access grant, its grantee; canonical adapter ingress may cite the
/// cause record as its provider source. Because the transfer revision
/// advances, an advance or cancellation that expects the earlier revision
/// settles as a `version_conflict` rejection. A lease that locks the
/// transfer's current revision for a terminal disposition keeps the loss's
/// own lease from locking it, so the host application releases that lease
/// first and takes a new one for the revision the loss leaves.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceTransferLossRequestV1 {
    pub operation_key: ResourceOperationKey,
    pub loss_id: ResourceLossId,
    pub transfer: ResourceTransferId,
    pub expected_transfer_revision: ResourceRevision,
    pub quantity: u64,
    pub cause: EvidenceRef,
    pub at: SimTime,
    pub completion_certificate: CompletionLeaseActivationCertificateV1,
}

/// The host application moves an account to another place scope, or to
/// none: the account's stock was carried elsewhere.
///
/// Only the host application sends it, through
/// [`crate::enqueue_resource_account_rescope`]; a tracked command, an adapter
/// packet, or a packet another plugin schedules cannot carry it. It needs no
/// completion lease, leaves one `RescopeAccount` outcome that keeps
/// `evidence` as its exact evidence, and advances the account revision.
/// `evidence` is the record of the move (the carriage that took the stock to
/// its new place, for example) and must be its record's current version when
/// the operation settles, read from the current-version index rather than
/// from retained evidence, so a sealed run and its exact replay decide alike;
/// its record kind must be one of the adapter evidence kinds the resource
/// plugin was configured with, not one the plugin reads for itself.
///
/// It is refused, as a recorded rejection, while the account is the source
/// or destination of a transfer that is not terminal, or while a completion
/// lease locks the account's current revision. Reservations, demands, and
/// access grants that name the account carry over unchanged. A new scope
/// equal to the current one is applied like any other, with its evidence.
/// [`ResourceState::apply_operation`] has no simulation view, so a detached
/// root does not read the evidence; the resource plugin does.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceRescopeAccountRequestV1 {
    pub operation_key: ResourceOperationKey,
    pub account: ResourceAccountId,
    pub expected_account_revision: ResourceRevision,
    pub place_scope: Option<crate::ResourceScopeId>,
    pub evidence: DomainRecordVersionRef,
}

/// Points one account at a protected-floor policy revision, or at none.
///
/// A tracked command must come from the account's custodian; an access grant
/// gives its grantee no authority over the grantor's floors. The operation is
/// token-only: it needs no completion lease, leaves one operation outcome,
/// and advances the account revision.
///
/// `policy` names an authored revision in the root by its id, or `None` to
/// clear the floor. To set a floor of any quantity, the custodian sends the
/// body of a content-addressed revision in `install` and names its id in
/// `policy`: the operation installs the revision if the root does not hold
/// it and points the account at it, together or not at all. A
/// content-addressed revision is always named with its body. Its id alone is
/// refused whether or not the root holds it, because that depends on what
/// other accounts and demands refer to.
///
/// Every applied request then retires the content-addressed revisions that no
/// account and no demand in the root refers to, so a floor that changes often
/// does not accumulate revisions. A retired revision is installed again by
/// sending its body. Retirement removes the body from the root: an archived
/// demand keeps the identity of the floor it was admitted under and nothing
/// else.
///
/// A refusal of the request is its recorded rejection, with one exception it
/// shares with every resource operation: a request whose result does not fit
/// the outcome and archive budgets of the root, or would pass its state byte
/// cap, records nothing. The resource plugin skips its packet with an event,
/// and [`ResourceState::apply_operation`] returns
/// [`crate::ResourceError::LimitExceeded`].
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceProtectedFloorRequestV1 {
    pub operation_key: ResourceOperationKey,
    pub account: ResourceAccountId,
    pub expected_account_revision: ResourceRevision,
    pub policy: Option<ProtectedFloorPolicyRevisionId>,
    /// Body of the revision `policy` names, to install it with this request.
    /// It must be content-addressed
    /// ([`ProtectedFloorPolicyRevision::content_addressed`]). Omitted from
    /// the canonical encoding while `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install: Option<ProtectedFloorPolicyRevision>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceCancelDemandRequestV1 {
    pub operation_key: ResourceOperationKey,
    pub demand: ResourceDemandId,
    pub expected_demand_revision: ResourceRevision,
}

/// Issues one delegated access grant. A tracked command must come from the
/// grant's `grantor_custodian`, and the grant's exact authority evidence must
/// be an available record version.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceIssueAccessGrantRequestV1 {
    pub operation_key: ResourceOperationKey,
    pub grant: crate::ResourceAccessGrantV1,
}

/// Revokes an access grant. A tracked command must come from the grantor
/// custodian. Like a demand source-policy amendment, revocation is refused
/// once any reservation has drawn on the grant.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceRevokeAccessGrantRequestV1 {
    pub operation_key: ResourceOperationKey,
    pub grant_id: crate::ResourceAccessGrantId,
    pub expected_grant_revision: ResourceRevision,
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
    /// Acquires, grants, prepares, and activates a lease whose only
    /// participant is the resource plugin, leaving one outcome and one
    /// receipt. Its operation key is
    /// `resource:completion:lease:{acquisition}`.
    SingleStepLease(crate::SingleStepCompletionLeaseV1),
}

/// Prefix of every completion operation's key. An operation key under it is
/// reserved for [`ResourceCompletionOperationV1`]: any other request that
/// names one is refused.
///
/// A holder can therefore take a completion key only with a completion
/// operation it may send as a tracked command: an `Acquire`, and an `Abort`
/// of a lease it holds. No holder can take the key of a single step, a
/// grant, a prepare, an activation, a release, an expiry, or an external
/// participant step. A packet whose key a different request holds settles
/// nothing on any ingress, is named by a
/// [`crate::RESOURCE_INGRESS_SKIPPED_EVENT`], and does not fail its
/// boundary.
pub const RESOURCE_COMPLETION_OPERATION_KEY_PREFIX: &str = "resource:completion:";

/// Credits one production output batch may carry. A batch with more settles
/// nothing, so a producer bounds its outputs by it.
pub const MAX_PRODUCTION_OUTPUT_BATCH_CREDITS: usize = 64;

impl ResourceCompletionOperationV1 {
    /// The key this operation derives from its identities, which is valid
    /// only when [`Self::has_canonical_operation_key`].
    fn derived_operation_key(&self) -> String {
        let prefix = RESOURCE_COMPLETION_OPERATION_KEY_PREFIX;
        match self {
            Self::Acquire(value) => format!("{prefix}acquire:{}", value.id.as_str()),
            Self::Grant(value) => format!("{prefix}grant:{}", value.grant_id.as_str()),
            Self::Prepare(value) => format!(
                "{prefix}prepare:{}:{}",
                value.acquisition.as_str(),
                value.grant.as_str()
            ),
            Self::Activate(value) => {
                format!("{prefix}activate:{}", value.acquisition.as_str())
            }
            Self::Abort(value) => format!("{prefix}abort:{}", value.acquisition.as_str()),
            Self::Expire(value) => format!(
                "{prefix}expire:{}:{}",
                value.at.as_minutes(),
                value.current_boundary
            ),
            Self::Release(value) => format!(
                "{prefix}release:{}:{}",
                value.acquisition.as_str(),
                value.grant.as_str()
            ),
            Self::GrantExternalParticipant(value) => {
                format!("{prefix}participant-grant:{}", value.acquisition.as_str())
            }
            Self::PrepareExternalParticipant(value) => {
                format!("{prefix}participant-prepare:{}", value.acquisition.as_str())
            }
            Self::ConsumeExternalParticipant(value) => format!(
                "{prefix}participant-consume:{}",
                value.certificate.acquisition.as_str()
            ),
            Self::CompleteExternalParticipant(value) => {
                format!(
                    "{prefix}participant-complete:{}",
                    value.acquisition.as_str()
                )
            }
            Self::ReleaseExternalParticipant(value) => {
                format!("{prefix}participant-release:{}", value.acquisition.as_str())
            }
            Self::ExpireExternalParticipants(value) => format!(
                "{prefix}participant-expire:{}:{}",
                value.at.as_minutes(),
                value.current_boundary
            ),
            Self::SingleStepLease(value) => format!("{prefix}lease:{}", value.lease.id.as_str()),
        }
    }

    /// Whether the identities this operation names form a valid operation
    /// key: `resource:completion:{step}:{identities}` within the 160-byte
    /// bound of [`ResourceOperationKey`]. An acquisition or grant identity
    /// may be valid by itself and still too long for the key of a step.
    ///
    /// A single step must also be releasable: its key names only the
    /// acquisition, but the `Release` that may have to give the lease up
    /// names the acquisition and the grant, so both identities must form
    /// that key too.
    #[must_use]
    pub fn has_canonical_operation_key(&self) -> bool {
        let releasable = match self {
            Self::SingleStepLease(value) => ResourceOperationKey::new(format!(
                "{RESOURCE_COMPLETION_OPERATION_KEY_PREFIX}release:{}:{}",
                value.lease.id.as_str(),
                value.grant_id.as_str()
            ))
            .is_ok(),
            _ => true,
        };
        releasable && ResourceOperationKey::new(self.derived_operation_key()).is_ok()
    }

    /// The operation key of this completion operation.
    ///
    /// It is derived from the identities the operation names. When they do
    /// not form a valid key, the key is `resource:completion:invalid:{digest}`
    /// over what would have been derived. Such an operation is refused, as is
    /// every operation for which [`Self::has_canonical_operation_key`] is
    /// false, so the refusal is recorded like any other and nothing panics.
    #[must_use]
    pub fn operation_key(&self) -> ResourceOperationKey {
        let derived = self.derived_operation_key();
        ResourceOperationKey::new(derived.clone()).unwrap_or_else(|_| {
            let digest = blake3::hash(derived.as_bytes()).to_hex();
            ResourceOperationKey::new(format!(
                "{RESOURCE_COMPLETION_OPERATION_KEY_PREFIX}invalid:{digest}"
            ))
            .expect("a digest always forms a valid operation key")
        })
    }

    /// For an operation that starts a lease, the key the other way of
    /// starting one takes for the same acquisition identity: an `Acquire`'s
    /// single-step key `resource:completion:lease:{id}`, a single step's
    /// `resource:completion:acquire:{id}`. An identity is used once for the
    /// run: a lease leaves the root when it finishes, so the outcome under
    /// either key, hot or archived, is what refuses a second lease of that
    /// identity. A key the identity cannot form was never taken.
    #[must_use]
    pub(crate) fn sibling_lease_operation_key(&self) -> Option<ResourceOperationKey> {
        let prefix = RESOURCE_COMPLETION_OPERATION_KEY_PREFIX;
        match self {
            Self::Acquire(value) => {
                ResourceOperationKey::new(format!("{prefix}lease:{}", value.id.as_str())).ok()
            }
            Self::SingleStepLease(value) => {
                ResourceOperationKey::new(format!("{prefix}acquire:{}", value.lease.id.as_str()))
                    .ok()
            }
            _ => None,
        }
    }
}

/// The refusal of a redemption that a lease's binding does not admit.
fn lease_bound_elsewhere() -> ResourceError {
    ResourceError::Authority(
        "completion lease is bound to another operation kind, disposition, or quantity".to_owned(),
    )
}

/// One completion lease a request redeems
/// ([`ResourceOperationRequestV1::lease_redemptions`]): its certificate, and
/// the kind, the transfer disposition, and the quantity of the operation
/// that redeems it, the quantity `None` when only the settlement knows it.
pub(crate) struct LeaseRedemption<'a> {
    pub(crate) certificate: &'a CompletionLeaseActivationCertificateV1,
    pub(crate) kind: ResourceOperationKind,
    pub(crate) disposition: Option<ResourceTransferDispositionKindV1>,
    pub(crate) quantity: Option<u64>,
}

/// The refusal of a lease start whose acquisition identity an earlier lease
/// took ([`ResourceCompletionOperationV1::sibling_lease_operation_key`]): an
/// applied outcome under the sibling key. A refused lease start took
/// nothing.
pub(crate) fn lease_identity_taken() -> ResourceError {
    ResourceError::IdempotencyConflict(
        "completion lease acquisition identity was already used by another lease".to_owned(),
    )
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
    RecordLoss(ResourceAccountLossRequestV1),
    BeginExchange(ResourceExchangeStartRequestV1),
    IssueAccessGrant(ResourceIssueAccessGrantRequestV1),
    RevokeAccessGrant(ResourceRevokeAccessGrantRequestV1),
    ConsumeBatch(ResourceConsumptionBatchRequestV1),
    /// Loss of part of an open transfer's escrow; see
    /// [`ResourceTransferLossRequestV1`].
    RecordTransferLoss(ResourceTransferLossRequestV1),
    /// The host application's change of an account's place scope; see
    /// [`ResourceRescopeAccountRequestV1`].
    RescopeAccount(ResourceRescopeAccountRequestV1),
}

impl ResourceOperationRequestV1 {
    #[must_use]
    pub fn operation_key(&self) -> ResourceOperationKey {
        match self {
            Self::RecordLoss(value) => value.operation_key.clone(),
            Self::RecordTransferLoss(value) => value.operation_key.clone(),
            Self::RescopeAccount(value) => value.operation_key.clone(),
            Self::BeginExchange(value) => value.operation_key.clone(),
            Self::IssueAccessGrant(value) => value.operation_key.clone(),
            Self::RevokeAccessGrant(value) => value.operation_key.clone(),
            Self::ConsumeBatch(value) => value.operation_key.clone(),
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
            Self::RecordLoss(_) | Self::RecordTransferLoss(_) => ResourceOperationKind::Loss,
            Self::RescopeAccount(_) => ResourceOperationKind::RescopeAccount,
            Self::BeginExchange(_) => ResourceOperationKind::BeginExchange,
            Self::IssueAccessGrant(_) => ResourceOperationKind::IssueAccessGrant,
            Self::RevokeAccessGrant(_) => ResourceOperationKind::RevokeAccessGrant,
            Self::ConsumeBatch(_) => ResourceOperationKind::ConsumeBatch,
        }
    }

    /// The completion certificates this request redeems: one for each
    /// irreversible operation, two for an exchange, none for any other
    /// request.
    #[must_use]
    pub fn completion_certificates(&self) -> Vec<&CompletionLeaseActivationCertificateV1> {
        self.lease_redemptions()
            .into_iter()
            .map(|redemption| redemption.certificate)
            .collect()
    }

    /// Each completion lease this request redeems, with what the request
    /// alone says of the operation that redeems it
    /// ([`crate::CompletionOperationBindingV1::admits`]).
    pub(crate) fn lease_redemptions(&self) -> Vec<LeaseRedemption<'_>> {
        let redemption = |certificate, quantity| LeaseRedemption {
            certificate,
            kind: self.kind(),
            disposition: None,
            quantity,
        };
        match self {
            Self::Consume(value) => vec![redemption(
                &value.completion_certificate,
                Some(value.allocation.quantity),
            )],
            Self::ConsumeBatch(value) => vec![redemption(
                &value.completion_certificate,
                value.items.iter().try_fold(0_u64, |total, item| {
                    total.checked_add(item.allocation.quantity)
                }),
            )],
            Self::BeginTransfer(value) => vec![redemption(
                &value.completion_certificate,
                Some(value.allocation.quantity),
            )],
            // The escrow a disposition settles is the one it finds.
            Self::CompleteTransfer(value) => vec![LeaseRedemption {
                disposition: Some(value.disposition.kind()),
                ..redemption(&value.completion_certificate, None)
            }],
            Self::Credit(value) => {
                vec![redemption(
                    &value.completion_certificate,
                    Some(value.quantity),
                )]
            }
            Self::ExternalOutflow(value) => {
                vec![redemption(
                    &value.completion_certificate,
                    Some(value.quantity),
                )]
            }
            Self::RecordLoss(value) => {
                vec![redemption(
                    &value.completion_certificate,
                    Some(value.quantity),
                )]
            }
            Self::RecordTransferLoss(value) => {
                vec![redemption(
                    &value.completion_certificate,
                    Some(value.quantity),
                )]
            }
            Self::BeginExchange(value) => vec![
                redemption(
                    &value.leg_a.completion_certificate,
                    Some(value.leg_a.allocation.quantity),
                ),
                redemption(
                    &value.leg_b.completion_certificate,
                    Some(value.leg_b.allocation.quantity),
                ),
            ],
            Self::CreateAccount(_)
            | Self::SubmitDemand(_)
            | Self::AmendDemand(_)
            | Self::Allocate(_)
            | Self::AdvanceTransfer(_)
            | Self::CancelTransfer(_)
            | Self::SetProtectedFloor(_)
            | Self::CancelDemand(_)
            | Self::RecordObservation(_)
            | Self::Completion(_)
            | Self::IssueAccessGrant(_)
            | Self::RevokeAccessGrant(_)
            | Self::RescopeAccount(_) => Vec::new(),
        }
    }

    /// The loss identity this request would record: an account loss, a
    /// partial transfer loss, or a transfer's `Lose` disposition.
    #[must_use]
    pub fn loss_identity(&self) -> Option<&ResourceLossId> {
        match self {
            Self::RecordLoss(value) => Some(&value.loss_id),
            Self::RecordTransferLoss(value) => Some(&value.loss_id),
            Self::CompleteTransfer(value) => match &value.disposition {
                ResourceTransferDispositionV1::Lose { loss_id, .. } => Some(loss_id),
                _ => None,
            },
            _ => None,
        }
    }

    /// The digest an outcome records as `request_digest` for this request
    /// when it settles as a tracked command, an adapter packet, or a
    /// completion operation. An operation key belongs to the first request
    /// that settles under it: a host application compares this digest with
    /// the outcome's to tell whether its request is the one that settled.
    pub fn request_digest(&self) -> Result<String, ResourceError> {
        canonical_digest("canwu.resource.operation-request.v1", self)
    }

    /// Whether this request names an operation key it may not use: a key
    /// under [`RESOURCE_COMPLETION_OPERATION_KEY_PREFIX`] on a request that is
    /// not a completion operation, or under
    /// [`RESOURCE_CONSUMPTION_BATCH_ITEM_KEY_PREFIX`] on any request. Such a
    /// request is refused wherever it enters and never settles.
    #[must_use]
    pub fn names_reserved_operation_key(&self) -> bool {
        let key = self.operation_key();
        (!matches!(self, Self::Completion(_))
            && key
                .as_str()
                .starts_with(RESOURCE_COMPLETION_OPERATION_KEY_PREFIX))
            || is_batch_item_key(key.as_str())
    }

    /// Transfer identities an outcome of this request must cite. Only an
    /// exchange names transfers; its outcome cites them whether applied or
    /// rejected.
    #[must_use]
    pub fn cited_transfers(&self) -> Vec<ResourceTransferId> {
        match self {
            Self::BeginExchange(value) => vec![
                value.leg_a.transfer_id.clone(),
                value.leg_b.transfer_id.clone(),
            ],
            _ => Vec::new(),
        }
    }
}

/// Whether `key` lies under [`RESOURCE_CONSUMPTION_BATCH_ITEM_KEY_PREFIX`].
pub(crate) fn is_batch_item_key(key: &str) -> bool {
    key.starts_with(RESOURCE_CONSUMPTION_BATCH_ITEM_KEY_PREFIX)
}

/// Whether a free-form operation key, such as a production credit's, lies
/// under a prefix that only completion operations and consumption batch
/// items may use.
pub(crate) fn is_reserved_operation_key(key: &str) -> bool {
    key.starts_with(RESOURCE_COMPLETION_OPERATION_KEY_PREFIX) || is_batch_item_key(key)
}

/// What one consumption settles under: its consumer evidence, its lease,
/// its operation key, and whether it is a batch item, whose caller measures
/// room on the batch's whole result.
#[derive(Clone, Copy)]
struct ConsumptionSettlement<'a> {
    consumer_evidence: &'a DomainRecordVersionRef,
    acquisition: &'a CompletionLeaseAcquisitionId,
    operation_key: &'a ResourceOperationKey,
    in_batch: bool,
}

/// How one consumption batch item settled.
enum BatchItemOutcome {
    Applied {
        quantity: u64,
        remainder: u64,
        consumption: ResourceConsumptionId,
    },
    Rejected(ResourceError),
}

struct AppliedOperation {
    quantity: u64,
    remainder: u64,
    result_ref: Option<ResourceRecordRefV1>,
    exact_evidence: Vec<DomainRecordVersionRef>,
}

/// Why an operation settled nothing: what it would leave does not fit the
/// root.
///
/// Room is measured on the result, not predicted from the kind of
/// operation. A refused operation needs room for its own outcome only; an
/// applied one for every terminal record it adds and every reservation it
/// makes. Work that is not certified by an admitted lease must also leave
/// the room that admitted leases reserve.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RoomShortage {
    /// The hot terminal records leave no room, and an archive commit of
    /// every hot candidate would.
    ArchiveBacklog,
    /// Even an empty backlog would leave no room: what remains is reserved
    /// by admitted completion leases, or the budget is smaller than the
    /// operation. An archive commit does not help.
    ///
    /// `permanent` when no archive commit and no lease release can make room
    /// for this work: it would not fit even with an empty backlog and every
    /// reservation free. Otherwise reservations hold the room, and they free:
    /// leases complete, are released, or expire, and the slots of finished
    /// participants whose reports wait become ordinary hot records once the
    /// reports publish, which an archive commit then clears. That holds for
    /// work an admitted lease certifies too, which must fit beside those
    /// waiting slots.
    ReservedCapacity { permanent: bool },
    /// The result would pass another limit of the root, such as its state
    /// byte cap.
    RootLimit,
}

impl RoomShortage {
    /// The error a detached caller of [`ResourceState::apply_operation`]
    /// receives, which has no packet to skip.
    fn error(self) -> ResourceError {
        ResourceError::LimitExceeded(
            match self {
                Self::ArchiveBacklog => "resource terminal archive backpressure is active",
                Self::ReservedCapacity { .. } => {
                    "resource hot capacity is reserved for admitted terminal work"
                }
                Self::RootLimit => "resource operation result exceeds a limit of the root",
            }
            .to_owned(),
        )
    }
}

/// What settling one operation did to the root.
pub(crate) enum OperationSettlement {
    /// The outcome under the operation's key: recorded now, applied or
    /// rejected, or recorded before for the same request.
    Settled(ResourceOperationOutcome),
    /// Nothing changed and nothing was recorded, because the result does not
    /// fit. The operation key stays free.
    NoRoom(RoomShortage),
}

impl OperationSettlement {
    fn into_outcome(self) -> Result<ResourceOperationOutcome, ResourceError> {
        match self {
            Self::Settled(outcome) => Ok(outcome),
            Self::NoRoom(shortage) => Err(shortage.error()),
        }
    }
}

/// What settling one production output batch did to the root.
pub(crate) enum OutputBatchSettlement {
    /// Every credit applied; their outcomes in batch order.
    Settled(Vec<ResourceOperationOutcome>),
    /// A credit did not fit, so nothing changed.
    NoRoom(RoomShortage),
    /// A credit was refused, so nothing changed: its operation key and the
    /// reason.
    Refused(ResourceOperationKey, String),
}

/// The scalar position of a root before it records a rejection, enough to
/// take that rejection back without a copy of the whole root.
struct RecordedRejection {
    operation_key: ResourceOperationKey,
    sequence: u64,
    state_revision: ResourceRevision,
    next_admitted_sequence: u64,
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
            identity_evidence: canwu_api::IdentityEvidenceDependenciesV1::new(Vec::new()),
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
            access_grants: BTreeMap::new(),
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
            hot_terminal: crate::ResourceHotTerminalLedgerV1::default(),
            live: crate::ResourceLiveLedgerV1::default(),
            custodian_accounts: BTreeMap::new(),
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
        if self.definition_count() >= self.limits.max_definitions
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
        if self.unit_count() >= self.limits.max_unit_revisions
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

    /// Installs one protected-floor policy revision while a scenario is
    /// built. The root holds at most
    /// [`crate::MAX_PROTECTED_FLOOR_POLICY_REVISIONS`]. At runtime a custodian
    /// installs a content-addressed revision with
    /// [`ResourceProtectedFloorRequestV1::install`] instead.
    ///
    /// An authored revision stays in the root. A content-addressed one does
    /// not stay for having been installed here: the first applied
    /// `SetProtectedFloor` retires it unless an account or a demand refers to
    /// it. An identity under
    /// [`crate::PROTECTED_FLOOR_POLICY_CONTENT_ID_PREFIX`] must be the one its
    /// content derives.
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
        if policy
            .id
            .as_str()
            .starts_with(crate::PROTECTED_FLOOR_POLICY_CONTENT_ID_PREFIX)
            && !policy.is_content_addressed()
        {
            return Err(ResourceError::InvalidDefinition(
                "a protected-floor policy identity under the content prefix must be derived from its content"
                    .to_owned(),
            ));
        }
        if self.protected_floor_policies.len() >= crate::MAX_PROTECTED_FLOOR_POLICY_REVISIONS {
            return Err(ResourceError::LimitExceeded(
                "protected-floor policy revision capacity is exhausted".to_owned(),
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
        if self.account_count() >= self.limits.max_accounts
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
        // The resource plugin attaches the custodian's index before it
        // settles an account's creation.
        self.custodian_accounts
            .entry(account.custodian.clone())
            .or_default()
            .insert(account.id.clone());
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
        self.validate_demand_admission(&demand, false)?;
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

    /// Installs a holder report grant, refusing one whose holder is of a
    /// kind that can never hold knowledge (a core organization, resource,
    /// route, or territory, or a person as an entity): its reports could
    /// never be published. Whether a person or a domain entity can hold
    /// knowledge depends on the run, so a grant to one that cannot is
    /// installed and published nothing; a completion that would reserve its
    /// report is refused when it settles.
    pub fn install_report_grant(
        &mut self,
        grant: ResourceReportGrantV1,
    ) -> Result<(), ResourceError> {
        if !crate::holder_kind_can_hold_knowledge(&grant.holder) {
            return Err(ResourceError::InvalidDefinition(
                "resource report grant holder is of a kind that can hold no knowledge".to_owned(),
            ));
        }
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

    /// Settles the report a completion reserved on `grant`: it was
    /// published, or it is given up because the grant has no observation
    /// head to report from or its holder cannot receive knowledge. When it
    /// was the completion's last reserved report, the completed participant
    /// becomes an archive candidate.
    pub(crate) fn release_completion_report(
        &mut self,
        acquisition: &CompletionLeaseAcquisitionId,
        grant: &crate::ResourceReportGrantId,
    ) -> Result<(), ResourceError> {
        if let Some(grants) = self.completion_report_reservations.get_mut(acquisition) {
            grants.remove(grant);
        }
        if self
            .completion_report_reservations
            .get(acquisition)
            .is_some_and(BTreeSet::is_empty)
        {
            self.completion_report_reservations.remove(acquisition);
            self.completion_report_ready.remove(acquisition);
            self.mark_external_participant_archive_ready(acquisition)?;
        }
        Ok(())
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
            || self.hot_candidate_attached(&key)
        {
            return Ok(());
        }
        let terminal_sequence = self.next_sequence()?;
        self.add_hot_candidate(terminal_sequence, key)?;
        // Its coordinator source moves from the live root's declaration to
        // its bucket's.
        self.refresh_continuation();
        Ok(())
    }

    /// The allocation legs open transfers draw on.
    fn open_transfer_legs(&self) -> BTreeSet<&ResourceAllocationLegId> {
        self.active_transfers
            .iter()
            .filter_map(|id| self.transfers.get(id))
            .map(|transfer| &transfer.allocation_leg)
            .collect()
    }

    /// Whether the closure of the ended demand `demand_id` (the demand, its
    /// reservations, and its legs) is held back from the archive: one of its
    /// reservations is still active, one of its legs is still reserved, or
    /// an open transfer draws on one of its legs (`open_transfer_legs`),
    /// whose terminal disposition reads the leg and marks the closure then.
    /// [`Self::mark_terminal_demand_closure`] and validation read this one
    /// rule.
    fn demand_closure_held(
        &self,
        demand_id: &ResourceDemandId,
        open_transfer_legs: &BTreeSet<&ResourceAllocationLegId>,
    ) -> Result<bool, ResourceError> {
        for reservation_id in self
            .reservation_by_demand
            .get(demand_id)
            .into_iter()
            .flatten()
        {
            let reservation = self.reservations.get(reservation_id).ok_or_else(|| {
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
                || open_transfer_legs.contains(&leg.id)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn mark_terminal_demand_closure(
        &mut self,
        demand_id: &ResourceDemandId,
        room_is_measured_after: bool,
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
        if self.demand_closure_held(demand_id, &self.open_transfer_legs())? {
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
        // The closure's records are read above, so a hot terminal one among
        // them is attached.
        keys.retain(|key| !self.hot_candidate_attached(key));
        let projected = self
            .hot_terminal_records()
            .checked_add(keys.len())
            .ok_or(ResourceError::Overflow)?;
        // A consumption batch and a terminal transfer disposition measure
        // room on their whole result instead ([`Self::room_shortage`]), so a
        // closure they cannot hold is a skip that records nothing, not a
        // refusal that takes their key.
        if !room_is_measured_after && projected > self.limits.max_archive_candidates {
            return Err(ResourceError::LimitExceeded(
                "resource terminal demand closure requires archive progress".to_owned(),
            ));
        }
        for key in keys {
            let sequence = self.next_sequence()?;
            self.add_hot_candidate(sequence, key)?;
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
            && (!head.transfers.is_empty()
                || !head.consumptions.is_empty()
                || !head.losses.is_empty())
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
        for observation in &head.losses {
            let Some(loss) = self.losses.get(&observation.loss) else {
                continue;
            };
            let owns_account = loss
                .account
                .as_ref()
                .is_some_and(|account| grant.accounts.contains(account))
                || loss
                    .transfer
                    .as_ref()
                    .and_then(|transfer| self.transfers.get(transfer))
                    .is_some_and(|transfer| {
                        grant.accounts.contains(&transfer.source)
                            || transfer
                                .destination
                                .as_ref()
                                .is_some_and(|account| grant.accounts.contains(account))
                    });
            if !owns_account
                || observation.account != loss.account
                || observation.transfer != loss.transfer
                || observation.quantity != loss.quantity
                || observation.cause != loss.cause
            {
                return Err(ResourceError::Authority(
                    "resource observation loss differs from authoritative holder custody"
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
        Ok(self.quantities_with_reserved(account, reserved))
    }

    /// What each account has reserved by its active reservations, in one pass
    /// over the reservations.
    fn reserved_by_account(&self) -> Result<BTreeMap<ResourceAccountId, u64>, ResourceError> {
        let mut reserved = BTreeMap::<ResourceAccountId, u64>::new();
        for reservation in self
            .reservations
            .values()
            .filter(|reservation| reservation.status == ReservationStatus::Active)
        {
            let total = reserved.entry(reservation.account.clone()).or_default();
            *total = total
                .checked_add(reservation.quantity)
                .ok_or(ResourceError::Overflow)?;
        }
        Ok(reserved)
    }

    /// The quantities of `account` given what its active reservations hold.
    fn quantities_with_reserved(
        &self,
        account: &ResourceAccount,
        reserved: u64,
    ) -> crate::AccountQuantitiesV1 {
        let protected = account
            .protected_floor_policy
            .as_ref()
            .and_then(|id| self.protected_floor_policies.get(id))
            .map_or(0, |policy| policy.floor.min(account.balance));
        let available = account
            .balance
            .saturating_sub(reserved)
            .saturating_sub(protected);
        crate::AccountQuantitiesV1 {
            authoritative_balance: account.balance,
            available,
            reserved,
            protected,
        }
    }

    /// Applies one operation to this root and records its outcome.
    ///
    /// A domain refusal is the operation's recorded rejection. Errors are
    /// what a detached caller must handle instead: a key under
    /// [`RESOURCE_COMPLETION_OPERATION_KEY_PREFIX`] on a request that is not
    /// a completion operation, a key whose outcome another request recorded,
    /// and a result that does not fit the root
    /// ([`ResourceError::LimitExceeded`], with nothing changed).
    ///
    /// Room is measured on the result. A refused operation needs room for
    /// its own outcome only, and an applied one for the terminal records it
    /// adds and the reservations it makes; work that an admitted lease does
    /// not certify must also leave the room admitted leases reserve. The
    /// resource plugin skips a packet whose result does not fit instead of
    /// failing its boundary.
    pub fn apply_operation(
        &mut self,
        request: &ResourceOperationRequestV1,
    ) -> Result<ResourceOperationOutcome, ResourceError> {
        self.settle_operation(request)?.into_outcome()
    }

    /// Settles an operation the plugin applies without a settlement clock of
    /// its own, as [`Self::apply_operation`] does, but reports a result that
    /// does not fit as [`OperationSettlement::NoRoom`].
    pub(crate) fn settle_operation(
        &mut self,
        request: &ResourceOperationRequestV1,
    ) -> Result<OperationSettlement, ResourceError> {
        let request_digest = canonical_digest("canwu.resource.operation-request.v1", request)?;
        self.settle_operation_with_context(request, request_digest, None, None, None)
    }

    /// Settles an operation that the plugin settles in a boundary, at
    /// `settled_at` when it is time-bound (detached callers of
    /// [`Self::apply_operation`] have no separate settlement clock, so their
    /// request times stand in for it), with `external_refusal`, what the
    /// plugin found against the simulation view, as its recorded rejection.
    /// A lease start (an archived outcome that took its identity, or a report
    /// holder that cannot receive knowledge), an external participant grant
    /// (a report holder that cannot receive knowledge), an account rescope
    /// (evidence that is not current or not of an adapter evidence kind), an
    /// observation head (a source that is not current or not of a kind the
    /// plugin reads), and an operation that redeems a completion lease (a
    /// loss identity an archived loss took, or a lease a tracked command may
    /// not redeem) read such a refusal.
    pub(crate) fn settle_operation_refused_by(
        &mut self,
        request: &ResourceOperationRequestV1,
        settled_at: Option<SimTime>,
        external_refusal: Option<&ResourceError>,
    ) -> Result<OperationSettlement, ResourceError> {
        let request_digest = canonical_digest("canwu.resource.operation-request.v1", request)?;
        self.settle_operation_with_context(
            request,
            request_digest,
            None,
            external_refusal,
            settled_at,
        )
    }

    /// Apply every credit in one production output batch atomically. A
    /// rejected leg discards the detached candidate, so no account, outcome,
    /// conservation, or terminal-index mutation becomes authoritative. A leg
    /// whose result does not fit the root is a [`ResourceError::LimitExceeded`]
    /// error, again with nothing changed.
    pub fn apply_production_output_batch(
        &mut self,
        requests: &[ResourceCreditRequestV1],
    ) -> Result<Vec<ResourceOperationOutcome>, ResourceError> {
        match self.settle_production_output_batch(requests)? {
            OutputBatchSettlement::Settled(outcomes) => Ok(outcomes),
            OutputBatchSettlement::NoRoom(shortage) => Err(shortage.error()),
            OutputBatchSettlement::Refused(_, reason) => {
                Err(ResourceError::InvalidLifecycle(reason))
            }
        }
    }

    /// Whether a production output batch of `credits` credits can ever
    /// settle in this root: it carries at most
    /// [`MAX_PRODUCTION_OUTPUT_BATCH_CREDITS`] credits, and with an empty
    /// archive backlog and every reservation free its outcomes, and the
    /// receipt of the archive commit that emptied the backlog, fit the
    /// root's limits. A batch that cannot is skipped as permanently short of
    /// room, so a producer refuses such an output set before it starts.
    #[must_use]
    pub fn production_output_batch_can_fit(&self, credits: usize) -> bool {
        (1..=MAX_PRODUCTION_OUTPUT_BATCH_CREDITS).contains(&credits)
            && credits.saturating_add(1) <= self.limits.max_archive_candidates
            && credits <= self.limits.max_operation_outcomes
    }

    /// Whether a hot outcome of this root shows that the lease start
    /// `operation` would reuse an acquisition identity another lease took
    /// ([`ResourceCompletionOperationV1::sibling_lease_operation_key`]).
    fn lease_identity_taken_in_root(&self, operation: &ResourceCompletionOperationV1) -> bool {
        operation.sibling_lease_operation_key().is_some_and(|key| {
            self.outcomes
                .get(&key)
                .is_some_and(|outcome| outcome.status == ResourceOperationStatus::Applied)
        })
    }

    /// Whether a consumption batch of `items` items, each consuming the one
    /// leg of its demand, can ever settle in this root: it carries at most
    /// [`MAX_RESOURCE_CONSUMPTION_BATCH_ITEMS`] items and at most
    /// `max_batch_items_per_boundary`, and with an empty archive backlog and
    /// every reservation free its records
    /// ([`RESOURCE_CONSUMPTION_BATCH_ITEM_RECORDS`] an item and
    /// [`RESOURCE_CONSUMPTION_BATCH_RECORDS`]), its outcomes, and the receipt
    /// of the archive commit that emptied the backlog fit the root's limits.
    ///
    /// A provider checks it before it takes the batch's lease. A batch
    /// larger than either bound is refused whole when it settles; one whose
    /// records do not fit is skipped and records nothing, so its lease stays
    /// active until it is released or expires.
    #[must_use]
    pub fn consumption_batch_can_fit(&self, items: usize) -> bool {
        (1..=MAX_RESOURCE_CONSUMPTION_BATCH_ITEMS).contains(&items)
            && items <= self.limits.max_batch_items_per_boundary
            && items
                .saturating_mul(RESOURCE_CONSUMPTION_BATCH_ITEM_RECORDS)
                .saturating_add(RESOURCE_CONSUMPTION_BATCH_RECORDS)
                .saturating_add(1)
                <= self.limits.max_archive_candidates
            && items.saturating_add(1) <= self.limits.max_operation_outcomes
    }

    /// Settles one production output batch on a detached candidate, so the
    /// root changes only when every credit applies.
    pub(crate) fn settle_production_output_batch(
        &mut self,
        requests: &[ResourceCreditRequestV1],
    ) -> Result<OutputBatchSettlement, ResourceError> {
        if requests.is_empty() || requests.len() > MAX_PRODUCTION_OUTPUT_BATCH_CREDITS {
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
        // Room is measured once, for the whole batch against the root before
        // it: a batch settles every credit or none, so whether an archive
        // commit makes room is a question about all of them.
        let reserved_work = self
            .terminal_work_is_reserved(&ResourceOperationRequestV1::Credit(requests[0].clone()));
        let hot_candidates = self.hot_terminal_records();
        let hot_outcomes = self.hot_operation_outcomes();
        let mut candidate = self.clone();
        let mut outcomes = Vec::with_capacity(requests.len());
        for request in requests {
            let credit = ResourceOperationRequestV1::Credit(request.clone());
            let request_digest = canonical_digest("canwu.resource.operation-request.v1", &credit)?;
            let outcome = match candidate.settle_operation_measured(
                &credit,
                request_digest,
                None,
                None,
                None,
                false,
            )? {
                OperationSettlement::Settled(outcome) => outcome,
                OperationSettlement::NoRoom(shortage) => {
                    return Ok(OutputBatchSettlement::NoRoom(shortage));
                }
            };
            if !matches!(
                outcome.status,
                ResourceOperationStatus::Applied | ResourceOperationStatus::Duplicate
            ) {
                return Ok(OutputBatchSettlement::Refused(
                    request.operation_key.clone(),
                    outcome
                        .rejection_reason
                        .unwrap_or_else(|| "production output batch leg was rejected".to_owned()),
                ));
            }
            outcomes.push(outcome);
        }
        if let Some(shortage) =
            candidate.room_shortage(reserved_work, hot_candidates, hot_outcomes, 0)?
        {
            return Ok(OutputBatchSettlement::NoRoom(shortage));
        }
        match candidate.validate_within(self.limits.operation_state_bytes()) {
            Ok(()) => {}
            Err(ResourceError::LimitExceeded(_)) => {
                return Ok(OutputBatchSettlement::NoRoom(RoomShortage::RootLimit));
            }
            Err(error) => return Err(error),
        }
        *self = candidate;
        Ok(OutputBatchSettlement::Settled(outcomes))
    }

    pub(crate) fn settle_prepare_with_external_revalidation(
        &mut self,
        request: &crate::PrepareCompletionCapacityV1,
        external_targets_current: bool,
    ) -> Result<OperationSettlement, ResourceError> {
        let operation = ResourceOperationRequestV1::Completion(
            ResourceCompletionOperationV1::Prepare(request.clone()),
        );
        let request_digest = canonical_digest("canwu.resource.operation-request.v1", &operation)?;
        let refusal = (!external_targets_current).then(|| {
            ResourceError::VersionConflict("completion prepare external target is stale".to_owned())
        });
        self.settle_operation_with_context(&operation, request_digest, None, refusal.as_ref(), None)
    }

    /// Settles one single-step lease that the plugin settles in a boundary.
    /// `external_refusal` is what the plugin found against the simulation
    /// view, which the detached state cannot read: a cited external record
    /// version that is not current, an eligibility time that is not the
    /// boundary time, or no room for the reserved reports. A refusal settles
    /// as the operation's recorded rejection, like one the state finds, and
    /// needs room for that outcome only.
    pub(crate) fn settle_single_step_lease_with_external_validation(
        &mut self,
        request: &crate::SingleStepCompletionLeaseV1,
        external_refusal: Option<&ResourceError>,
    ) -> Result<OperationSettlement, ResourceError> {
        let operation = ResourceOperationRequestV1::Completion(
            ResourceCompletionOperationV1::SingleStepLease(request.clone()),
        );
        let request_digest = canonical_digest("canwu.resource.operation-request.v1", &operation)?;
        self.settle_operation_with_context(&operation, request_digest, None, external_refusal, None)
    }

    /// Settles one requester-scoped allocation that the plugin settles in a
    /// boundary at `settled_at`.
    pub(crate) fn settle_authorized_allocation(
        &mut self,
        requester: &canwu_api::KnowledgeHolderRef,
        request: &ResourceAllocationRequestV1,
        settled_at: SimTime,
    ) -> Result<OperationSettlement, ResourceError> {
        let operation = ResourceOperationRequestV1::Allocate(request.clone());
        let request_digest = canonical_digest(
            "canwu.resource.authorized-allocation-request.v1",
            &(requester, request),
        )?;
        self.settle_operation_with_context(
            &operation,
            request_digest,
            Some(requester),
            None,
            Some(settled_at),
        )
    }

    /// `external_refusal` carries what the plugin established against the
    /// simulation view for a `Prepare` or a `SingleStepLease`; see
    /// [`Self::settle_prepare_with_external_revalidation`] and
    /// [`Self::settle_single_step_lease_with_external_validation`].
    ///
    /// The operation is applied first and its room measured on the result
    /// ([`Self::room_shortage`]). A result that does not fit, or that passes
    /// another limit of the root, leaves the root as it was and returns
    /// [`OperationSettlement::NoRoom`]; nothing is recorded and the key stays
    /// free for the same request later.
    fn settle_operation_with_context(
        &mut self,
        request: &ResourceOperationRequestV1,
        request_digest: String,
        allocation_requester: Option<&canwu_api::KnowledgeHolderRef>,
        external_refusal: Option<&ResourceError>,
        settled_at: Option<SimTime>,
    ) -> Result<OperationSettlement, ResourceError> {
        self.settle_operation_measured(
            request,
            request_digest,
            allocation_requester,
            external_refusal,
            settled_at,
            true,
        )
    }

    /// [`Self::settle_operation_with_context`], which measures room and
    /// validates the root when `measure` is set. A caller that settles several
    /// operations as one unit, a production output batch, clears it, and
    /// measures and validates the unit itself.
    fn settle_operation_measured(
        &mut self,
        request: &ResourceOperationRequestV1,
        request_digest: String,
        allocation_requester: Option<&canwu_api::KnowledgeHolderRef>,
        external_refusal: Option<&ResourceError>,
        settled_at: Option<SimTime>,
        measure: bool,
    ) -> Result<OperationSettlement, ResourceError> {
        // Not recorded: an outcome under the key would itself take it from
        // the completion operation that derives it.
        if request.names_reserved_operation_key() {
            return Err(ResourceError::Authority(
                "operation keys under `resource:completion:` are reserved for completion operations"
                    .to_owned(),
            ));
        }
        let operation_key = request.operation_key();
        if let Some(existing) = self.outcomes.get(&operation_key) {
            if existing.request_digest == request_digest {
                return Ok(OperationSettlement::Settled(existing.clone()));
            }
            return Err(ResourceError::IdempotencyConflict(
                "resource operation key was reused with a different request".to_owned(),
            ));
        }
        let terminal_work_is_reserved = self.terminal_work_is_reserved(request);
        let hot_candidates = self.hot_terminal_records();
        let hot_outcomes = self.hot_operation_outcomes();
        // The root as it was. An applied operation keeps it to take the
        // whole operation back; a rejection restores it at once and is taken
        // back from its recorded position instead.
        let mut before = Some(self.clone());
        let applied = self.apply_inner(
            request,
            &request_digest,
            allocation_requester,
            external_refusal,
            settled_at,
        );
        if applied.is_err() {
            *self = before
                .take()
                .expect("the root before the operation is kept");
        } else {
            self.retire_finished_leases();
        }
        if measure
            && let Some(shortage) =
                self.room_shortage(terminal_work_is_reserved, hot_candidates, hot_outcomes, 1)?
        {
            if let Some(before) = before {
                *self = before;
            }
            return Ok(OperationSettlement::NoRoom(shortage));
        }
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
            Err(error) => (
                ResourceOperationStatus::Rejected,
                0,
                request_remainder(self, request),
                None,
                Vec::new(),
                Some(error.code().to_owned()),
                Some(error.to_string()),
            ),
        };
        let state_revision = self.state_revision;
        let next_admitted_sequence = self.next_admitted_sequence;
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
            cited_transfers: request.cited_transfers(),
            rejection_code,
            rejection_reason,
            exact_evidence,
            semantic_digest: String::new(),
            sequence: self.next_sequence()?,
        };
        outcome.semantic_digest =
            canonical_digest("canwu.resource.operation-outcome.v1", &outcome)?;
        self.add_hot_candidate(
            outcome.sequence,
            crate::ResourceTerminalRecordKeyV1::Outcome(outcome.operation_key.clone()),
        )?;
        self.outcomes
            .insert(outcome.operation_key.clone(), outcome.clone());
        if status == ResourceOperationStatus::Applied
            && !matches!(request, ResourceOperationRequestV1::RecordObservation(_))
        {
            self.report_dirty_grants
                .extend(self.report_grants.keys().cloned());
        }
        self.refresh_continuation();
        if !measure {
            return Ok(OperationSettlement::Settled(outcome));
        }
        // An operation leaves the maintenance reserve of the byte cap free.
        match self.validate_within(self.limits.operation_state_bytes()) {
            Ok(()) => Ok(OperationSettlement::Settled(outcome)),
            // The counted budgets were measured above, so this is another
            // limit, in practice the state byte cap.
            Err(ResourceError::LimitExceeded(_)) => {
                match before {
                    Some(before) => *self = before,
                    None => self.take_back_rejection(RecordedRejection {
                        operation_key: outcome.operation_key,
                        sequence: outcome.sequence,
                        state_revision,
                        next_admitted_sequence,
                    })?,
                }
                Ok(OperationSettlement::NoRoom(RoomShortage::RootLimit))
            }
            Err(error) => Err(error),
        }
    }

    /// Takes back a rejection recorded on a root that was otherwise left as
    /// it was: its outcome, its archive candidate, and the counters
    /// recording it advanced. A rejection changes nothing else.
    fn take_back_rejection(&mut self, recorded: RecordedRejection) -> Result<(), ResourceError> {
        self.drop_hot_candidate(&crate::ResourceTerminalRecordKeyV1::Outcome(
            recorded.operation_key.clone(),
        ))?;
        self.outcomes.remove(&recorded.operation_key);
        self.terminal_archive_candidates.remove(&recorded.sequence);
        self.state_revision = recorded.state_revision;
        self.next_admitted_sequence = recorded.next_admitted_sequence;
        self.refresh_continuation();
        Ok(())
    }

    /// Whether this root, about to record `pending_records` more outcomes,
    /// lacks room for what the work left in it. `hot_candidates` and
    /// `hot_outcomes` are the counts before the work.
    ///
    /// Each outcome takes one archive candidate slot and one hot outcome slot.
    /// Work certified by an admitted lease (`reserved_work`) draws on the
    /// room that lease reserved, so it must fit only beside the reports that
    /// wait to become archive candidates. Any other work must also leave
    /// every reservation of admitted leases and participants free.
    ///
    /// A shortage is [`RoomShortage::ArchiveBacklog`] when an archive commit
    /// of every hot candidate would make room: such a commit archives every
    /// hot outcome and leaves its own maintenance receipt as the one
    /// candidate. Otherwise it is [`RoomShortage::ReservedCapacity`], which is
    /// permanent only for work that would not fit even with an empty backlog
    /// and every reservation free.
    fn room_shortage(
        &self,
        reserved_work: bool,
        hot_candidates: usize,
        hot_outcomes: usize,
        pending_records: usize,
    ) -> Result<Option<RoomShortage>, ResourceError> {
        let candidates = self
            .hot_terminal_records()
            .checked_add(pending_records)
            .ok_or(ResourceError::Overflow)?;
        let outcomes = self
            .hot_operation_outcomes()
            .checked_add(pending_records)
            .ok_or(ResourceError::Overflow)?;
        let (reserved_candidates, reserved_outcomes) = if reserved_work {
            (self.pending_external_participant_archive_slots()?, 0)
        } else {
            (
                self.reserved_terminal_archive_slots()?,
                self.reserved_hot_outcome_slots()?,
            )
        };
        let fits = |candidates: usize, outcomes: usize| -> Result<bool, ResourceError> {
            Ok(candidates
                .checked_add(reserved_candidates)
                .ok_or(ResourceError::Overflow)?
                <= self.limits.max_archive_candidates
                && outcomes
                    .checked_add(reserved_outcomes)
                    .ok_or(ResourceError::Overflow)?
                    <= self.limits.max_operation_outcomes)
        };
        if fits(candidates, outcomes)? {
            return Ok(None);
        }
        let added_candidates = candidates.saturating_sub(hot_candidates);
        let added_outcomes = outcomes.saturating_sub(hot_outcomes);
        let archive_makes_room =
            hot_candidates > 0 && fits(added_candidates.saturating_add(1), added_outcomes)?;
        if archive_makes_room {
            return Ok(Some(RoomShortage::ArchiveBacklog));
        }
        // With an empty backlog and every reservation free, the work would
        // hold its own records and, after an archive commit, that commit's
        // receipt.
        let floor = usize::from(hot_candidates > 0);
        let fits_alone = added_candidates.saturating_add(floor)
            <= self.limits.max_archive_candidates
            && added_outcomes <= self.limits.max_operation_outcomes;
        Ok(Some(RoomShortage::ReservedCapacity {
            permanent: !fits_alone,
        }))
    }

    fn apply_inner(
        &mut self,
        request: &ResourceOperationRequestV1,
        request_digest: &str,
        allocation_requester: Option<&canwu_api::KnowledgeHolderRef>,
        external_refusal: Option<&ResourceError>,
        settled_at: Option<SimTime>,
    ) -> Result<AppliedOperation, ResourceError> {
        // What the plugin found about an operation that redeems a completion
        // lease and that the root alone cannot tell (a loss identity an
        // archived loss took, or a lease a tracked command may not redeem)
        // refuses it before its own checks. A completion operation and a
        // rescope read their refusal themselves.
        if let Some(refusal) = external_refusal
            && !request.completion_certificates().is_empty()
        {
            return Err(refusal.clone());
        }
        match request {
            ResourceOperationRequestV1::ConsumeBatch(value) => {
                self.consume_batch(value, request_digest, settled_at)
            }
            ResourceOperationRequestV1::CreateAccount(value) => self.create_account(value),
            ResourceOperationRequestV1::SubmitDemand(value) => self.submit_demand(value),
            ResourceOperationRequestV1::AmendDemand(value) => self.amend_demand(value),
            ResourceOperationRequestV1::Allocate(value) => {
                self.allocate(value, allocation_requester, settled_at)
            }
            ResourceOperationRequestV1::Consume(value) => self.consume(value, settled_at),
            ResourceOperationRequestV1::BeginTransfer(value) => {
                self.begin_transfer(value, settled_at)
            }
            ResourceOperationRequestV1::AdvanceTransfer(value) => self.advance_transfer(value),
            ResourceOperationRequestV1::CancelTransfer(value) => self.cancel_transfer(value),
            ResourceOperationRequestV1::CompleteTransfer(value) => self.complete_transfer(value),
            ResourceOperationRequestV1::Credit(value) => self.credit(value),
            ResourceOperationRequestV1::ExternalOutflow(value) => self.external_outflow(value),
            ResourceOperationRequestV1::SetProtectedFloor(value) => self.set_protected_floor(value),
            ResourceOperationRequestV1::CancelDemand(value) => self.cancel_demand(value),
            ResourceOperationRequestV1::RecordObservation(value) => {
                // A source the plugin found not current.
                if let Some(refusal) = external_refusal {
                    return Err(refusal.clone());
                }
                self.record_observation_head(value.head.clone())?;
                Ok(AppliedOperation {
                    quantity: 0,
                    remainder: 0,
                    result_ref: None,
                    exact_evidence: value.head.source_versions.clone(),
                })
            }
            ResourceOperationRequestV1::Completion(value) => {
                self.apply_completion(value, external_refusal)
            }
            ResourceOperationRequestV1::RecordLoss(value) => self.record_account_loss(value),
            ResourceOperationRequestV1::BeginExchange(value) => {
                self.begin_exchange(value, settled_at)
            }
            ResourceOperationRequestV1::IssueAccessGrant(value) => self.issue_access_grant(value),
            ResourceOperationRequestV1::RevokeAccessGrant(value) => self.revoke_access_grant(value),
            ResourceOperationRequestV1::RecordTransferLoss(value) => {
                self.record_transfer_loss(value)
            }
            ResourceOperationRequestV1::RescopeAccount(value) => {
                self.rescope_account(value, external_refusal)
            }
        }
    }

    fn apply_completion(
        &mut self,
        operation: &ResourceCompletionOperationV1,
        external_refusal: Option<&ResourceError>,
    ) -> Result<AppliedOperation, ResourceError> {
        if !operation.has_canonical_operation_key() {
            return Err(ResourceError::InvalidIdentifier(
                "completion operation identities do not form a valid operation key".to_owned(),
            ));
        }
        if matches!(
            operation,
            ResourceCompletionOperationV1::GrantExternalParticipant(_)
                | ResourceCompletionOperationV1::PrepareExternalParticipant(_)
                | ResourceCompletionOperationV1::ConsumeExternalParticipant(_)
                | ResourceCompletionOperationV1::CompleteExternalParticipant(_)
                | ResourceCompletionOperationV1::ReleaseExternalParticipant(_)
                | ResourceCompletionOperationV1::ExpireExternalParticipants(_)
        ) {
            // The plugin refuses a participant grant whose report holder
            // cannot receive knowledge.
            if let (ResourceCompletionOperationV1::GrantExternalParticipant(_), Some(refusal)) =
                (operation, external_refusal)
            {
                return Err(refusal.clone());
            }
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
                // A resend is answered by the outcome under this key. An
                // acquisition the root already holds was therefore created
                // by another operation, a single step, and is never resumed:
                // the lease book would hand back a matching acquisition
                // whatever its state.
                if self
                    .completion_leases
                    .acquisitions
                    .contains_key(&request.id)
                {
                    return Err(ResourceError::IdempotencyConflict(
                        "completion lease acquisition already exists".to_owned(),
                    ));
                }
                // A finished lease of this identity has left the root; the
                // outcome of its single step still names it. The plugin
                // finds an archived one (`external_refusal`).
                if let Some(refusal) = external_refusal {
                    return Err(refusal.clone());
                }
                if self.lease_identity_taken_in_root(operation) {
                    return Err(lease_identity_taken());
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
                    .and_then(|()| external_refusal.map_or(Ok(()), |refusal| Err(refusal.clone())));
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
            ResourceCompletionOperationV1::SingleStepLease(request) => {
                if let Some(refusal) = external_refusal {
                    return Err(refusal.clone());
                }
                if self.lease_identity_taken_in_root(operation) {
                    return Err(lease_identity_taken());
                }
                let certificate = self.lease_single_step(request)?;
                (
                    certificate.acquisition.clone(),
                    Some(request.grant_id.clone()),
                    CompletionLeaseReceiptActionV1::Activated,
                    None,
                    request.lease.eligibility_envelope.exact_evidence.clone(),
                    Some(ResourceRecordRefV1::Lease(certificate.acquisition)),
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
        self.add_hot_candidate(
            sequence,
            crate::ResourceTerminalRecordKeyV1::LeaseReceipt(receipt.sequence),
        )?;
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
            | ResourceCompletionOperationV1::Release(_)
            | ResourceCompletionOperationV1::SingleStepLease(_) => unreachable!(
                "local completion operations are handled by the regular lease coordinator"
            ),
        }
    }

    /// Runs the acquire, grant, prepare, and activate steps of one lease
    /// whose only participant is the resource plugin, each with the checks it
    /// has as a separate operation. Any error leaves the caller to restore
    /// the state it held before, so the lease exists whole or not at all.
    fn lease_single_step(
        &mut self,
        request: &crate::SingleStepCompletionLeaseV1,
    ) -> Result<CompletionLeaseActivationCertificateV1, ResourceError> {
        let lease = &request.lease;
        if lease.expected_participants.len() != 1
            || !lease.expected_participants.contains(PLUGIN_NAME)
        {
            return Err(ResourceError::InvalidLifecycle(
                "a single-step completion lease has the resource plugin as its only participant; a lease with another participant uses the acquire, grant, prepare, and activate operations"
                    .to_owned(),
            ));
        }
        if lease.recipe.receipts != crate::MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE {
            return Err(ResourceError::LimitExceeded(
                "resource completion acquisition must reserve the full bounded terminal receipt path"
                    .to_owned(),
            ));
        }
        // The four-step path resumes an acquisition or a grant that already
        // exists. A single step never does: it creates both or is refused.
        if self.completion_leases.acquisitions.contains_key(&lease.id)
            || self
                .completion_leases
                .grants
                .contains_key(&request.grant_id)
        {
            return Err(ResourceError::IdempotencyConflict(
                "single-step completion lease names an acquisition or grant that already exists"
                    .to_owned(),
            ));
        }
        // Acquire.
        let acquisition = self
            .completion_leases
            .request_acquisition(&self.run_budget, lease.clone())?;
        self.reserve_completion_reports(&acquisition.id, &lease.holder, &lease.recipe)?;
        // Grant.
        self.validate_completion_locked_targets(&request.target_versions)?;
        let grant = self.completion_leases.grant_capacity(
            &self.run_budget,
            crate::GrantCompletionCapacityV1 {
                grant_id: request.grant_id.clone(),
                acquisition: acquisition.id.clone(),
                expected_acquisition_revision: acquisition.revision,
                owner_plugin: PLUGIN_NAME.to_owned(),
                target_versions: request.target_versions.clone(),
                current_boundary: request.current_boundary,
            },
        )?;
        // Prepare. The targets were validated above against this same state.
        let granted_revision = self.completion_leases.acquisitions[&acquisition.id].revision;
        let prepared =
            self.completion_leases
                .prepare_capacity(crate::PrepareCompletionCapacityV1 {
                    acquisition: acquisition.id.clone(),
                    expected_acquisition_revision: granted_revision,
                    grant: grant.id.clone(),
                    expected_grant_revision: grant.revision,
                    current_boundary: request.current_boundary,
                    eligibility_envelope_digest: lease.eligibility_envelope.digest.clone(),
                })?;
        if prepared.state != CompletionGrantStateV1::Prepared {
            return Err(ResourceError::VersionConflict(format!(
                "single-step completion lease could not prepare its grant: {}",
                prepared.rejection.as_deref().unwrap_or("prepare_rejected")
            )));
        }
        // Activate.
        self.completion_leases.activate_single_owner(
            &acquisition.id,
            &prepared.id,
            request.current_boundary,
        )
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
        // Lifecycle status, its rejection reason, and the requester belong to
        // the lifecycle writer and authority, not to the amendment; a changed
        // value would desynchronize the due, expiry, and dirty indexes.
        if !matches!(
            current.status,
            DemandStatus::Open | DemandStatus::PartiallyFulfilled
        ) || request.replacement.status != current.status
            || request.replacement.rejection_reason != current.rejection_reason
            || request.replacement.requester != current.requester
            || request.replacement.requested < current.fulfilled
            || request.replacement.fulfilled != current.fulfilled
            || request.replacement.resource_revision != current.resource_revision
            || request.replacement.unit_revision != current.unit_revision
        {
            return Err(ResourceError::InvalidLifecycle(
                "resource demand amendment changes immutable or settled fields".to_owned(),
            ));
        }
        // What is reserved for the demand and what open transfers carry
        // toward it will still be fulfilled or given back; the demand may not
        // ask for less than that and what it holds.
        let drawn = self
            .active_reserved_for_demand(&current.id)?
            .checked_add(
                self.in_flight_by_demand()?
                    .get(&current.id)
                    .copied()
                    .unwrap_or(0),
            )
            .and_then(|drawn| drawn.checked_add(current.fulfilled))
            .ok_or(ResourceError::Overflow)?;
        if request.replacement.requested < drawn {
            return Err(ResourceError::InvalidLifecycle(
                "resource demand amendment asks for less than is fulfilled, reserved, and in flight"
                    .to_owned(),
            ));
        }
        self.validate_demand_contract(&request.replacement)?;
        // Reservations remain indexed, including consumed legs, until the demand is
        // terminal and its closure is archived. Terminal demands cannot be amended.
        let drawn = current.fulfilled > 0
            || self
                .reservation_by_demand
                .get(&current.id)
                .is_some_and(|ids| !ids.is_empty());
        self.validate_demand_admission(&request.replacement, drawn)?;
        if request.replacement.source_policy != current.source_policy && drawn {
            return Err(ResourceError::InvalidLifecycle(
                "resource demand source policy cannot change after reservation or fulfillment"
                    .to_owned(),
            ));
        }
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
        settled_at: Option<SimTime>,
    ) -> Result<AppliedOperation, ResourceError> {
        self.expect_state_revision(request.expected_state_revision)?;
        if request.candidate_limit == 0
            || request.candidate_limit > self.limits.max_allocation_candidates
        {
            return Err(ResourceError::LimitExceeded(
                "allocation candidate budget is invalid".to_owned(),
            ));
        }
        // Hot terminal demands count too; the ledger knows their requesters
        // whether or not they are attached.
        if requester.is_some_and(|requester| {
            !self.hot_terminal.demand_requesters.contains_key(requester)
                && !self
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
        let in_flight = self.in_flight_by_demand()?;
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
                            .ok()
                            .and_then(|reserved| {
                                reserved
                                    .checked_add(in_flight.get(&demand.id).copied().unwrap_or(0))
                            })
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
        // One pass over the reservations for the whole pass, kept current as
        // the pass reserves.
        let mut reserved_by_account = self.reserved_by_account()?;
        for (_, _, _, _, demand_id) in candidates {
            let demand = self.demands.get(&demand_id).cloned().ok_or_else(|| {
                ResourceError::NotFound("allocation demand disappeared".to_owned())
            })?;
            self.validate_demand_sources(&demand)?;
            // What open transfers carry toward the demand is drawn already:
            // their acceptance fulfils it, and only what a loss or a return
            // takes from them is needed again.
            let drawn = self
                .active_reserved_for_demand(&demand.id)?
                .checked_add(in_flight.get(&demand.id).copied().unwrap_or(0))
                .ok_or(ResourceError::Overflow)?;
            let needed = demand.remainder().saturating_sub(drawn);
            if needed == 0 {
                continue;
            }
            // A pooled demand draws on every account of its resource; the
            // resource plugin attaches them all before such a pass.
            let mut accounts: Vec<_> = match &demand.source_policy {
                ResourceDemandSourcePolicyV1::Pooled => self
                    .accounts
                    .values()
                    .filter(|account| {
                        !account.closed
                            && account.resource_revision == demand.resource_revision
                            && account.unit_revision == demand.unit_revision
                    })
                    .map(|account| account.id.clone())
                    .collect(),
                ResourceDemandSourcePolicyV1::ExactAccounts(accounts)
                | ResourceDemandSourcePolicyV1::Granted { accounts, .. } => accounts.clone(),
            };
            accounts.sort();
            let mut total_available = accounts.iter().try_fold(0_u64, |total, account| {
                total
                    .checked_add(self.available_for_source(
                        account,
                        &demand,
                        reserved_by_account.get(account).copied().unwrap_or(0),
                    )?)
                    .ok_or(ResourceError::Overflow)
            })?;
            // A granted demand is bounded by its grant before scarcity
            // arbitration: an inactive grant, or one not current at both the
            // requested allocation time and the settling boundary, supplies
            // nothing, and no allocation exceeds the remaining cap. There is
            // no fallback.
            let grant_limit = demand.source_policy.access_grant().map(|grant_id| {
                self.access_grants
                    .get(grant_id)
                    .filter(|record| {
                        record.status == crate::ResourceAccessGrantStatusV1::Active
                            && record.grant.is_current_at(request.at)
                            && settled_at.is_none_or(|at| record.grant.is_current_at(at))
                    })
                    .map_or(0, crate::ResourceAccessGrantRecordV1::remaining)
            });
            if let Some(limit) = grant_limit {
                total_available = total_available.min(limit);
            }
            let grantable = needed.min(total_available);
            let minimum_remaining = demand
                .minimum_useful
                .saturating_sub(demand.fulfilled.saturating_add(drawn));
            let partial_blocked = demand.partial_fulfillment
                == PartialFulfillmentPolicy::RejectPartial
                && grantable < needed;
            if grantable == 0 || grantable < minimum_remaining || partial_blocked {
                if drawn == 0 && demand.fulfilled == 0 && grantable < demand.minimum_useful {
                    let current = self.demands.get_mut(&demand.id).expect("candidate exists");
                    current.status = DemandStatus::RejectedMinimum;
                    current.rejection_reason = Some(if grant_limit == Some(0) {
                        "access_grant_unavailable".to_owned()
                    } else {
                        "minimum_useful_not_met".to_owned()
                    });
                    current.revision = current.revision.next()?;
                    if let Some(ids) = self.demand_due_index.get_mut(&demand.due_at) {
                        ids.remove(&demand.id);
                    }
                    if let Some(ids) = self.demand_expiry_index.get_mut(&demand.expires_at) {
                        ids.remove(&demand.id);
                    }
                    self.dirty_demands.remove(&demand.id);
                    self.mark_terminal_demand_closure(&demand.id, false)?;
                }
                continue;
            }
            let mut remaining = grantable;
            for account_id in accounts {
                if remaining == 0 {
                    break;
                }
                let available = self.available_for_source(
                    &account_id,
                    &demand,
                    reserved_by_account.get(&account_id).copied().unwrap_or(0),
                )?;
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
                let reserved = reserved_by_account
                    .entry(reservation.account.clone())
                    .or_default();
                *reserved = reserved
                    .checked_add(quantity)
                    .ok_or(ResourceError::Overflow)?;
                self.reservations.insert(reservation_id, reservation);
                self.allocation_legs.insert(leg_id.clone(), leg);
                if let Some(grant_id) = demand.source_policy.access_grant() {
                    self.reserve_access_grant(grant_id, quantity)?;
                }
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
        settled_at: Option<SimTime>,
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
        self.granted_debit_authority(
            &leg,
            request.at,
            settled_at,
            &request.completion_certificate,
        )?;
        let external_participant = self
            .external_completion_participants
            .grants
            .contains_key(&request.completion_certificate.acquisition);
        let (acquisition, grant) = if external_participant {
            self.validate_consumed_external_completion_certificate(
                &request.completion_certificate,
                request.at,
                false,
                &request.operation_key,
                &resource_targets,
            )?
        } else {
            self.consume_completion_certificate(
                &request.completion_certificate,
                request.at,
                &request.operation_key,
                &local_targets,
                crate::CompletionOperationBindingV1::new(
                    ResourceOperationKind::Consume,
                    leg.quantity,
                ),
            )?
        };
        let (consumption, fulfillment) = self.settle_consumption(
            &leg,
            &request.consumption_id,
            request.expected_account_revision,
            &ConsumptionSettlement {
                consumer_evidence: &request.consumer_evidence,
                acquisition: &acquisition,
                operation_key: &request.operation_key,
                in_batch: false,
            },
        )?;
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

    /// Debits one exact allocation leg and records its consumption and the
    /// fulfillment of its demand, which the consumption names, and marks a
    /// demand that this finishes as terminal. The caller has checked the
    /// leg and the certificate.
    fn settle_consumption(
        &mut self,
        leg: &ResourceAllocationLeg,
        consumption_id: &ResourceConsumptionId,
        expected_account_revision: ResourceRevision,
        settlement: &ConsumptionSettlement<'_>,
    ) -> Result<(ResourceConsumption, crate::ResourceFulfillment), ResourceError> {
        let ConsumptionSettlement {
            consumer_evidence,
            acquisition,
            operation_key,
            in_batch,
        } = *settlement;
        self.consume_leg(leg, expected_account_revision)?;
        let terminal_sequence = self.next_sequence()?;
        self.conservation.admitted_consumption = self
            .conservation
            .admitted_consumption
            .checked_add(u128::from(leg.quantity))
            .ok_or(ResourceError::Overflow)?;
        let fulfillment = self.record_fulfillment(leg, leg.quantity, operation_key.clone())?;
        let mut consumption = ResourceConsumption {
            id: consumption_id.clone(),
            revision: ResourceRevision::INITIAL,
            account: leg.account.clone(),
            allocation_leg: leg.id.clone(),
            demand: leg.demand.clone(),
            resource_revision: leg.resource_revision.clone(),
            unit_revision: leg.unit_revision.clone(),
            quantity: leg.quantity,
            consumer_evidence: consumer_evidence.clone(),
            completion_acquisition: acquisition.clone(),
            status: ConsumptionStatus::Settled,
            operation_key: operation_key.clone(),
            fulfillment: fulfillment.id.clone(),
            semantic_digest: String::new(),
            terminal_sequence,
        };
        consumption.semantic_digest =
            canonical_digest("canwu.resource.consumption.v1", &consumption)?;
        self.consumptions
            .insert(consumption.id.clone(), consumption.clone());
        self.add_hot_candidate(
            consumption.terminal_sequence,
            crate::ResourceTerminalRecordKeyV1::Consumption(consumption.id.clone()),
        )?;
        self.mark_terminal_demand_closure(&leg.demand, in_batch)?;
        Ok((consumption, fulfillment))
    }

    /// Settles a consumption batch: the certificate once for the whole batch,
    /// then each item, refused alone or applied, then the lease's completion.
    /// An error refuses the batch as a whole; the caller restores the root.
    /// The terminal closures of finished demands are not checked against the
    /// archive budget as they are added: the caller measures room on the
    /// whole result, so a batch that does not fit is a skip.
    fn consume_batch(
        &mut self,
        request: &ResourceConsumptionBatchRequestV1,
        request_digest: &str,
        settled_at: Option<SimTime>,
    ) -> Result<AppliedOperation, ResourceError> {
        if request.items.is_empty() || request.items.len() > MAX_RESOURCE_CONSUMPTION_BATCH_ITEMS {
            return Err(ResourceError::LimitExceeded(format!(
                "a resource consumption batch carries 1 to {MAX_RESOURCE_CONSUMPTION_BATCH_ITEMS} items"
            )));
        }
        // A batch larger than a boundary settles could never settle.
        if request.items.len() > self.limits.max_batch_items_per_boundary {
            return Err(ResourceError::LimitExceeded(
                "a resource consumption batch carries more items than one boundary settles"
                    .to_owned(),
            ));
        }
        if settled_at.is_some_and(|at| at != request.at) {
            return Err(ResourceError::Authority(
                "a resource consumption batch settles at its certified time".to_owned(),
            ));
        }
        let certificate = &request.completion_certificate;
        if self
            .external_completion_participants
            .participant(&certificate.acquisition)
            .is_some()
        {
            return Err(ResourceError::InvalidLifecycle(
                "a resource consumption batch runs under a local completion lease".to_owned(),
            ));
        }
        let batch_quantity = request.items.iter().try_fold(0_u64, |total, item| {
            total
                .checked_add(item.allocation.quantity)
                .ok_or(ResourceError::Overflow)
        })?;
        let (acquisition, grant) = self.consume_completion_certificate(
            certificate,
            request.at,
            &request.operation_key,
            &request.locked_targets(),
            crate::CompletionOperationBindingV1::new(
                ResourceOperationKind::ConsumeBatch,
                batch_quantity,
            ),
        )?;
        let mut consumed = 0_u64;
        let mut refused = 0_u64;
        // The account revisions this batch's own debits produced, by account:
        // the revision the batch found (which its certificate locks) and the
        // one its last debit left. An item that expects the revision the
        // batch found draws on the account as the batch left it, so one batch
        // consumes several legs of one account.
        let mut debited =
            BTreeMap::<ResourceAccountId, (ResourceRevision, ResourceRevision)>::new();
        for (index, item) in request.items.iter().enumerate() {
            let key = request.item_operation_key(index);
            if self.outcomes.contains_key(&key) {
                return Err(ResourceError::IdempotencyConflict(
                    "a resource consumption batch item key is already recorded".to_owned(),
                ));
            }
            let expected_account_revision = match debited.get(&item.allocation.account) {
                Some((found, current)) if *found == item.expected_account_revision => *current,
                _ => item.expected_account_revision,
            };
            // Only the checks refuse an item. Once they pass, settling it
            // fails for nothing an item can cause, so an error there refuses
            // the batch, and the caller restores the root.
            let outcome =
                match self.check_batch_item(item, expected_account_revision, request, settled_at) {
                    Err(error) => {
                        refused = refused
                            .checked_add(item.allocation.quantity)
                            .ok_or(ResourceError::Overflow)?;
                        BatchItemOutcome::Rejected(error)
                    }
                    Ok(leg) => {
                        let (consumption, fulfillment) = self.settle_consumption(
                            &leg,
                            &item.consumption_id,
                            expected_account_revision,
                            &ConsumptionSettlement {
                                consumer_evidence: &request.consumer_evidence,
                                acquisition: &acquisition,
                                operation_key: &key,
                                in_batch: true,
                            },
                        )?;
                        let revision = self
                            .accounts
                            .get(&leg.account)
                            .ok_or_else(|| {
                                ResourceError::InvalidDefinition(
                                    "a consumed batch item lost its account".to_owned(),
                                )
                            })?
                            .revision;
                        debited
                            .entry(leg.account.clone())
                            .and_modify(|(_, current)| *current = revision)
                            .or_insert((item.expected_account_revision, revision));
                        consumed = consumed
                            .checked_add(leg.quantity)
                            .ok_or(ResourceError::Overflow)?;
                        BatchItemOutcome::Applied {
                            quantity: leg.quantity,
                            remainder: fulfillment.remainder,
                            consumption: consumption.id,
                        }
                    }
                };
            self.record_batch_item_outcome(
                key,
                request_digest,
                &request.consumer_evidence,
                item.allocation.quantity,
                outcome,
            )?;
        }
        self.completion_leases
            .complete_grant(&acquisition, &grant)?;
        self.record_completed_completion(request.operation_key.clone(), acquisition, grant)?;
        Ok(AppliedOperation {
            quantity: consumed,
            remainder: refused,
            result_ref: None,
            exact_evidence: vec![request.consumer_evidence.clone()],
        })
    }

    /// The checks one batch item must pass before its leg is debited; their
    /// refusal is the item's recorded rejection. They are the checks a
    /// single consumption makes of its leg, account, demand, and grant.
    fn check_batch_item(
        &self,
        item: &ResourceConsumptionBatchItemV1,
        expected_account_revision: ResourceRevision,
        request: &ResourceConsumptionBatchRequestV1,
        settled_at: Option<SimTime>,
    ) -> Result<ResourceAllocationLeg, ResourceError> {
        let leg = self.exact_allocation(&item.allocation)?.clone();
        if leg.demand != item.demand || leg.demand_revision != item.demand_revision {
            return Err(ResourceError::VersionConflict(
                "resource consumption batch item names another demand revision".to_owned(),
            ));
        }
        if self.consumptions.contains_key(&item.consumption_id) {
            return Err(ResourceError::IdempotencyConflict(
                "resource consumption identity already exists".to_owned(),
            ));
        }
        if leg.status != AllocationLegStatus::Reserved || leg.quantity == 0 {
            return Err(ResourceError::InvalidLifecycle(
                "resource allocation leg is not consumable".to_owned(),
            ));
        }
        let reservation = self.reservations.get(&leg.reservation).ok_or_else(|| {
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
        let account = self.accounts.get(&leg.account).ok_or_else(|| {
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
        if account.balance < leg.quantity {
            return Err(ResourceError::Conservation(
                "resource account balance is below its reserved allocation".to_owned(),
            ));
        }
        let demand = self.demands.get(&leg.demand).ok_or_else(|| {
            ResourceError::InvalidDefinition("allocation leg lost its demand".to_owned())
        })?;
        if demand
            .fulfilled
            .checked_add(leg.quantity)
            .is_none_or(|fulfilled| fulfilled > demand.requested)
        {
            return Err(ResourceError::Conservation(
                "resource demand fulfillment would exceed its requested quantity".to_owned(),
            ));
        }
        self.granted_debit_authority(
            &leg,
            request.at,
            settled_at,
            &request.completion_certificate,
        )?;
        Ok(leg)
    }

    /// Records the outcome of one batch item under its reserved key, as a
    /// consumption outcome of the batch's request.
    fn record_batch_item_outcome(
        &mut self,
        operation_key: ResourceOperationKey,
        request_digest: &str,
        consumer_evidence: &DomainRecordVersionRef,
        allocated: u64,
        outcome: BatchItemOutcome,
    ) -> Result<(), ResourceError> {
        let (status, quantity, remainder, result_ref, exact_evidence, rejection) = match outcome {
            BatchItemOutcome::Applied {
                quantity,
                remainder,
                consumption,
            } => (
                ResourceOperationStatus::Applied,
                quantity,
                remainder,
                Some(ResourceRecordRefV1::Consumption(consumption)),
                vec![consumer_evidence.clone()],
                None,
            ),
            BatchItemOutcome::Rejected(error) => (
                ResourceOperationStatus::Rejected,
                0,
                allocated,
                None,
                Vec::new(),
                Some(error),
            ),
        };
        let id = outcome_id(&operation_key, request_digest)?;
        let mut outcome = ResourceOperationOutcome {
            id,
            revision: ResourceRevision::INITIAL,
            operation_key,
            request_digest: request_digest.to_owned(),
            kind: ResourceOperationKind::Consume,
            status,
            quantity,
            remainder,
            result_ref,
            cited_transfers: Vec::new(),
            rejection_code: rejection.as_ref().map(|error| error.code().to_owned()),
            rejection_reason: rejection.as_ref().map(ToString::to_string),
            exact_evidence,
            semantic_digest: String::new(),
            sequence: self.next_sequence()?,
        };
        outcome.semantic_digest =
            canonical_digest("canwu.resource.operation-outcome.v1", &outcome)?;
        self.add_hot_candidate(
            outcome.sequence,
            crate::ResourceTerminalRecordKeyV1::Outcome(outcome.operation_key.clone()),
        )?;
        self.outcomes.insert(outcome.operation_key.clone(), outcome);
        Ok(())
    }

    /// Takes every completion lease that has finished out of the root: an
    /// acquisition released or expired whose named report reservation, if
    /// any, has published, with its grants and certificate. Their receipts
    /// stay hot terminal records until an archive commit takes them, and the
    /// operation outcomes under their keys answer a resend; nothing else
    /// reads a finished lease.
    pub(crate) fn retire_finished_leases(&mut self) {
        let finished: Vec<_> = self
            .completion_leases
            .acquisitions
            .values()
            .filter(|acquisition| {
                matches!(
                    acquisition.state,
                    crate::CompletionLeaseAcquisitionStateV1::Released
                        | crate::CompletionLeaseAcquisitionStateV1::Expired
                ) && !self
                    .completion_report_reservations
                    .contains_key(&acquisition.id)
                    && !self.completion_report_ready.contains_key(&acquisition.id)
            })
            .map(|acquisition| acquisition.id.clone())
            .collect();
        if finished.is_empty() {
            return;
        }
        let leases = &mut self.completion_leases;
        for id in &finished {
            leases.acquisitions.remove(id);
            leases.certificates.remove(id);
        }
        let retired: BTreeSet<_> = finished.into_iter().collect();
        let grants: Vec<_> = leases
            .grants
            .values()
            .filter(|grant| retired.contains(&grant.acquisition))
            .map(|grant| grant.id.clone())
            .collect();
        for grant_id in &grants {
            if let Some(grant) = leases.grants.remove(grant_id) {
                for target in grant.target_versions {
                    if leases.target_locks.get(&target) == Some(grant_id) {
                        leases.target_locks.remove(&target);
                    }
                }
            }
        }
        let grants: BTreeSet<_> = grants.into_iter().collect();
        for due in leases.expiry_due.values_mut() {
            due.retain(|grant| !grants.contains(grant));
        }
        leases.expiry_due.retain(|_, due| !due.is_empty());
        // The retired grants and certificates no longer declare the
        // versions they locked.
        self.refresh_continuation();
    }

    fn begin_transfer(
        &mut self,
        request: &ResourceTransferStartRequestV1,
        settled_at: Option<SimTime>,
    ) -> Result<AppliedOperation, ResourceError> {
        if self.hot_transfer_count()? >= self.limits.max_transfers
            || self.transfers.contains_key(&request.transfer_id)
        {
            return Err(ResourceError::LimitExceeded(
                "resource transfer capacity or identity is unavailable".to_owned(),
            ));
        }
        let (leg, targets) = self.transfer_start_targets(request)?;
        let access_grant = self.granted_debit_authority(
            &leg,
            request.at,
            settled_at,
            &request.completion_certificate,
        )?;
        let (acquisition, _) = self.consume_completion_certificate(
            &request.completion_certificate,
            request.at,
            &request.operation_key,
            &targets,
            crate::CompletionOperationBindingV1::new(
                ResourceOperationKind::BeginTransfer,
                leg.quantity,
            ),
        )?;
        self.settle_transfer_start(request, leg.clone(), acquisition, access_grant)?;
        Ok(AppliedOperation {
            quantity: leg.quantity,
            remainder: 0,
            result_ref: Some(ResourceRecordRefV1::Transfer(request.transfer_id.clone())),
            exact_evidence: Vec::new(),
        })
    }

    /// Starts both legs of an exchange on this (already detached) state. Any
    /// error rejects the whole exchange: the caller restores the pre-operation
    /// state, so neither leg debits, consumes a lease, or creates a transfer.
    fn begin_exchange(
        &mut self,
        request: &ResourceExchangeStartRequestV1,
        settled_at: Option<SimTime>,
    ) -> Result<AppliedOperation, ResourceError> {
        let legs = [&request.leg_a, &request.leg_b];
        let leg_keys = request.terms().leg_operation_keys()?;
        if legs
            .iter()
            .zip(&leg_keys)
            .any(|(leg, key)| &leg.operation_key != key)
        {
            return Err(ResourceError::Authority(
                "resource exchange leg keys must be derived from the exchange terms".to_owned(),
            ));
        }
        if leg_keys.iter().any(|key| self.outcomes.contains_key(key)) {
            return Err(ResourceError::IdempotencyConflict(
                "resource exchange leg key already has an operation outcome".to_owned(),
            ));
        }
        if request.leg_a.transfer_id == request.leg_b.transfer_id
            || request.leg_a.allocation.id == request.leg_b.allocation.id
            || request.leg_a.completion_certificate.acquisition
                == request.leg_b.completion_certificate.acquisition
        {
            return Err(ResourceError::InvalidDefinition(
                "resource exchange legs must use distinct transfers, allocations, and completion leases"
                    .to_owned(),
            ));
        }
        let projected = self
            .hot_transfer_count()?
            .checked_add(legs.len())
            .ok_or(ResourceError::Overflow)?;
        if projected > self.limits.max_transfers
            || legs
                .iter()
                .any(|leg| self.transfers.contains_key(&leg.transfer_id))
        {
            return Err(ResourceError::LimitExceeded(
                "resource transfer capacity or identity is unavailable".to_owned(),
            ));
        }
        let mut prepared = Vec::with_capacity(legs.len());
        for leg in legs {
            let (allocation, targets) = self.transfer_start_targets(leg)?;
            let source = self.accounts.get(&allocation.account).ok_or_else(|| {
                ResourceError::NotFound("exchange source account is unavailable".to_owned())
            })?;
            let lease_holder = self
                .completion_leases
                .acquisitions
                .get(&leg.completion_certificate.acquisition)
                .map(|acquisition| &acquisition.holder);
            // A granted leg is consented to by its grantee under the grant;
            // any other leg by its source custodian.
            let access_grant = self.granted_debit_authority(
                &allocation,
                leg.at,
                settled_at,
                &leg.completion_certificate,
            )?;
            if access_grant.is_none() && lease_holder != Some(&source.custodian) {
                return Err(ResourceError::Authority(
                    "each exchange leg requires a completion lease held by its source custodian"
                        .to_owned(),
                ));
            }
            prepared.push((leg, allocation, targets, access_grant));
        }
        for (leg, allocation, targets, access_grant) in prepared {
            let (acquisition, _) = self.consume_completion_certificate(
                &leg.completion_certificate,
                leg.at,
                &leg.operation_key,
                &targets,
                crate::CompletionOperationBindingV1::new(
                    ResourceOperationKind::BeginExchange,
                    allocation.quantity,
                ),
            )?;
            self.settle_transfer_start(leg, allocation, acquisition, access_grant)?;
        }
        Ok(AppliedOperation {
            quantity: 0,
            remainder: 0,
            result_ref: None,
            exact_evidence: Vec::new(),
        })
    }

    /// Validates one transfer start against current state and returns its
    /// exact allocation leg and the completion targets it must lock.
    fn transfer_start_targets(
        &self,
        request: &ResourceTransferStartRequestV1,
    ) -> Result<(ResourceAllocationLeg, Vec<CompletionLockedTargetV1>), ResourceError> {
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
        Ok((leg, targets))
    }

    /// Debits the exact allocation into escrow and creates the transfer after
    /// its completion certificate was consumed.
    fn settle_transfer_start(
        &mut self,
        request: &ResourceTransferStartRequestV1,
        leg: ResourceAllocationLeg,
        acquisition: CompletionLeaseAcquisitionId,
        access_grant: Option<crate::ResourceAccessGrantId>,
    ) -> Result<(), ResourceError> {
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
            link_history: Vec::new(),
            completion_acquisition: acquisition,
            operation_key: request.operation_key.clone(),
            terminal_sequence: 0,
            access_grant,
        };
        self.active_transfers.insert(transfer.id.clone());
        self.transfers.insert(transfer.id.clone(), transfer);
        Ok(())
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
            (ResourceTransferState::InTransit, TransferProgressV1::InTransit) => {
                check_intermediate_link(transfer, &request.transport)?;
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
        // An arrival and a return always fit: an intermediate advance leaves
        // room for both, and each happens at most once.
        transfer.state = next;
        transfer.transport = Some(request.transport.clone());
        // The link history is the open transfer's only copy of its transport
        // evidence; `exact_evidence` is written when the transfer ends.
        transfer.link_history.push(crate::ResourceTransferLinkV1 {
            progress: request.progress,
            transport: request.transport.clone(),
            evidence: request.transport_evidence.clone(),
        });
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
            ResourceTransferDispositionV1::AcceptLocal {
                destination,
                expected_destination_revision,
                handover_evidence,
            } => {
                terminal_targets.push(CompletionLockedTargetV1::Account {
                    id: destination.clone(),
                    revision: *expected_destination_revision,
                });
                terminal_targets.push(CompletionLockedTargetV1::ExternalRecord {
                    version: handover_evidence.clone(),
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
            crate::CompletionOperationBindingV1::new(
                ResourceOperationKind::CompleteTransfer,
                quantity,
            )
            .with_disposition(request.disposition.kind()),
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
                return self.settle_transfer_acceptance(
                    request,
                    &snapshot,
                    destination,
                    *expected_destination_revision,
                    exact_evidence,
                    (original_grant, terminal_acquisition, terminal_grant),
                );
            }
            ResourceTransferDispositionV1::AcceptLocal {
                destination,
                expected_destination_revision,
                handover_evidence,
            } => {
                if snapshot.state != ResourceTransferState::PendingDispatch
                    || snapshot.transport.is_some()
                    || request.exact_transport_evidence.is_some()
                {
                    return Err(ResourceError::InvalidLifecycle(
                        "local acceptance requires a pending transfer without a transport execution"
                            .to_owned(),
                    ));
                }
                if snapshot.destination.as_ref() != Some(destination) {
                    return Err(ResourceError::InvalidDefinition(
                        "acceptance destination differs from the transfer".to_owned(),
                    ));
                }
                let source_scope = self
                    .accounts
                    .get(&snapshot.source)
                    .and_then(|account| account.place_scope.as_ref());
                let destination_scope = self
                    .accounts
                    .get(destination)
                    .and_then(|account| account.place_scope.as_ref());
                if source_scope.is_none() || source_scope != destination_scope {
                    return Err(ResourceError::InvalidDefinition(
                        "local acceptance requires source and destination accounts to declare the same place scope"
                            .to_owned(),
                    ));
                }
                exact_evidence.push(handover_evidence.clone());
                return self.settle_transfer_acceptance(
                    request,
                    &snapshot,
                    destination,
                    *expected_destination_revision,
                    exact_evidence,
                    (original_grant, terminal_acquisition, terminal_grant),
                );
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
                self.add_hot_candidate(
                    self.losses[loss_id].terminal_sequence,
                    crate::ResourceTerminalRecordKeyV1::Loss(loss_id.clone()),
                )?;
                let transfer = self
                    .transfers
                    .get_mut(&request.transfer)
                    .expect("checked above");
                // Partial losses may have taken part of the escrow before:
                // this loses the rest.
                transfer.lost = transfer
                    .lost
                    .checked_add(quantity)
                    .ok_or(ResourceError::Overflow)?;
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
        self.add_hot_candidate(
            terminal_sequence,
            crate::ResourceTerminalRecordKeyV1::Transfer(request.transfer.clone()),
        )?;
        self.active_transfers.remove(&request.transfer);
        // A demand that ended while this transfer was in flight held its
        // closure back for it ([`Self::mark_terminal_demand_closure`]); its
        // leg is in the live root until then.
        if let Some(demand) = self
            .allocation_legs
            .get(&snapshot.allocation_leg)
            .map(|leg| leg.demand.clone())
            && self.demands.contains_key(&demand)
        {
            self.mark_terminal_demand_closure(&demand, true)?;
        }
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

    /// Credits the destination with the whole escrow, records it as the
    /// fulfillment of the allocation's demand, and closes the transfer and
    /// both completion leases. Shared by transport-backed and same-place
    /// acceptance after their own checks. The demand is credited with what is
    /// accepted, not with the allocation: escrow that partial losses took
    /// stays in the demand's remainder.
    fn settle_transfer_acceptance(
        &mut self,
        request: &ResourceTransferDispositionRequestV1,
        snapshot: &ResourceTransfer,
        destination: &ResourceAccountId,
        expected_destination_revision: ResourceRevision,
        exact_evidence: Vec<DomainRecordVersionRef>,
        (original_grant, terminal_acquisition, terminal_grant): (
            CompletionCapacityGrantId,
            CompletionLeaseAcquisitionId,
            CompletionCapacityGrantId,
        ),
    ) -> Result<AppliedOperation, ResourceError> {
        let quantity = snapshot.escrow;
        let account = self.accounts.get_mut(destination).ok_or_else(|| {
            ResourceError::NotFound("destination account is unavailable".to_owned())
        })?;
        if account.revision != expected_destination_revision
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
                ResourceError::InvalidDefinition("transfer lost its allocation leg".to_owned())
            })?;
        let fulfillment = self.record_fulfillment(&leg, quantity, request.operation_key.clone())?;
        // The transfer is no longer in flight, so its leg no longer holds
        // back its demand's closure.
        self.active_transfers.remove(&request.transfer);
        self.mark_terminal_demand_closure(&leg.demand, true)?;
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
        self.add_hot_candidate(
            terminal_sequence,
            crate::ResourceTerminalRecordKeyV1::Transfer(transfer_id.clone()),
        )?;
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
        Ok(AppliedOperation {
            quantity,
            remainder: fulfillment.remainder,
            result_ref: Some(ResourceRecordRefV1::Transfer(transfer_id)),
            exact_evidence,
        })
    }

    /// Settles an account-level loss in place. The certificate locks the
    /// exact account revision and, when the cause is a domain record, that
    /// record version.
    fn record_account_loss(
        &mut self,
        request: &ResourceAccountLossRequestV1,
    ) -> Result<AppliedOperation, ResourceError> {
        if self.losses.contains_key(&request.loss_id) {
            return Err(ResourceError::IdempotencyConflict(
                "resource loss identity already exists".to_owned(),
            ));
        }
        let mut targets = vec![CompletionLockedTargetV1::Account {
            id: request.account.clone(),
            revision: request.expected_account_revision,
        }];
        let exact_evidence = match &request.cause {
            EvidenceRef::DomainRecordVersion(version) => {
                targets.push(CompletionLockedTargetV1::ExternalRecord {
                    version: version.clone(),
                });
                vec![version.clone()]
            }
            _ => Vec::new(),
        };
        let (acquisition, grant) = self.consume_completion_certificate(
            &request.completion_certificate,
            request.at,
            &request.operation_key,
            &targets,
            crate::CompletionOperationBindingV1::new(ResourceOperationKind::Loss, request.quantity),
        )?;
        let quantities = self.account_quantities(&request.account)?;
        let account = self.accounts.get_mut(&request.account).ok_or_else(|| {
            ResourceError::NotFound("resource loss account is unavailable".to_owned())
        })?;
        if account.revision != request.expected_account_revision {
            return Err(ResourceError::VersionConflict(
                "resource loss expected a stale account revision".to_owned(),
            ));
        }
        let losable = if request.allow_protected {
            account.balance.saturating_sub(quantities.reserved)
        } else {
            quantities.available
        };
        if request.quantity == 0 || request.quantity > losable {
            return Err(ResourceError::ProtectedFloor(
                "resource loss would remove reserved or protected stock".to_owned(),
            ));
        }
        debit_account(account, request.quantity)?;
        let resource_revision = account.resource_revision.clone();
        let unit_revision = account.unit_revision.clone();
        self.conservation.admitted_loss = self
            .conservation
            .admitted_loss
            .checked_add(u128::from(request.quantity))
            .ok_or(ResourceError::Overflow)?;
        let loss = ResourceLoss {
            id: request.loss_id.clone(),
            revision: ResourceRevision::INITIAL,
            account: Some(request.account.clone()),
            transfer: None,
            resource_revision,
            unit_revision,
            quantity: request.quantity,
            cause: request.cause.clone(),
            operation_key: request.operation_key.clone(),
            terminal_sequence: self.next_sequence()?,
        };
        self.add_hot_candidate(
            loss.terminal_sequence,
            crate::ResourceTerminalRecordKeyV1::Loss(loss.id.clone()),
        )?;
        self.losses.insert(loss.id.clone(), loss);
        self.completion_leases
            .complete_grant(&acquisition, &grant)?;
        self.record_completed_completion(request.operation_key.clone(), acquisition, grant)?;
        Ok(AppliedOperation {
            quantity: request.quantity,
            remainder: 0,
            result_ref: Some(ResourceRecordRefV1::Loss(request.loss_id.clone())),
            exact_evidence,
        })
    }

    /// Settles the loss of part of an open transfer's escrow. The transfer
    /// keeps its state and the rest of its escrow, and its revision advances.
    /// The certificate locks the exact transfer revision and, when the cause
    /// is a domain record, that record version; the loss completes that
    /// lease, never the one the transfer was started under, which its
    /// terminal disposition completes.
    fn record_transfer_loss(
        &mut self,
        request: &ResourceTransferLossRequestV1,
    ) -> Result<AppliedOperation, ResourceError> {
        if self.losses.contains_key(&request.loss_id) {
            return Err(ResourceError::IdempotencyConflict(
                "resource loss identity already exists".to_owned(),
            ));
        }
        let transfer = self.transfers.get(&request.transfer).ok_or_else(|| {
            ResourceError::NotFound("resource transfer is unavailable".to_owned())
        })?;
        if transfer.revision != request.expected_transfer_revision || transfer.escrow == 0 {
            return Err(ResourceError::VersionConflict(
                "resource transfer exact revision is stale or terminal".to_owned(),
            ));
        }
        if request.quantity == 0 {
            return Err(ResourceError::InvalidDefinition(
                "a partial resource transfer loss must lose a positive quantity".to_owned(),
            ));
        }
        if request.quantity >= transfer.escrow {
            return Err(ResourceError::InvalidLifecycle(
                "a partial resource transfer loss leaves part of the escrow; a loss of the whole escrow completes the transfer with the Lose disposition"
                    .to_owned(),
            ));
        }
        let mut targets = vec![CompletionLockedTargetV1::Transfer {
            id: transfer.id.clone(),
            revision: transfer.revision,
        }];
        let exact_evidence = match &request.cause {
            EvidenceRef::DomainRecordVersion(version) => {
                targets.push(CompletionLockedTargetV1::ExternalRecord {
                    version: version.clone(),
                });
                vec![version.clone()]
            }
            _ => Vec::new(),
        };
        let (acquisition, grant) = self.consume_completion_certificate(
            &request.completion_certificate,
            request.at,
            &request.operation_key,
            &targets,
            crate::CompletionOperationBindingV1::new(ResourceOperationKind::Loss, request.quantity),
        )?;
        let transfer = self
            .transfers
            .get_mut(&request.transfer)
            .expect("checked above");
        transfer.escrow -= request.quantity;
        transfer.lost = transfer
            .lost
            .checked_add(request.quantity)
            .ok_or(ResourceError::Overflow)?;
        transfer.revision = transfer.revision.next()?;
        let remainder = transfer.escrow;
        let resource_revision = transfer.resource_revision.clone();
        let unit_revision = transfer.unit_revision.clone();
        self.conservation.admitted_loss = self
            .conservation
            .admitted_loss
            .checked_add(u128::from(request.quantity))
            .ok_or(ResourceError::Overflow)?;
        let loss = ResourceLoss {
            id: request.loss_id.clone(),
            revision: ResourceRevision::INITIAL,
            account: None,
            transfer: Some(request.transfer.clone()),
            resource_revision,
            unit_revision,
            quantity: request.quantity,
            cause: request.cause.clone(),
            operation_key: request.operation_key.clone(),
            terminal_sequence: self.next_sequence()?,
        };
        self.add_hot_candidate(
            loss.terminal_sequence,
            crate::ResourceTerminalRecordKeyV1::Loss(loss.id.clone()),
        )?;
        self.losses.insert(loss.id.clone(), loss);
        self.completion_leases
            .complete_grant(&acquisition, &grant)?;
        self.record_completed_completion(request.operation_key.clone(), acquisition, grant)?;
        Ok(AppliedOperation {
            quantity: request.quantity,
            remainder,
            result_ref: Some(ResourceRecordRefV1::Loss(request.loss_id.clone())),
            exact_evidence,
        })
    }

    /// Moves an account to another place scope, or to none: the host
    /// application's operation. `external_refusal` is what the plugin found
    /// against the simulation view about the evidence (a version that is not
    /// its record's current version, or not of an adapter evidence kind),
    /// checked after the root's own refusals. A detached caller has no view
    /// and passes none.
    fn rescope_account(
        &mut self,
        request: &ResourceRescopeAccountRequestV1,
        external_refusal: Option<&ResourceError>,
    ) -> Result<AppliedOperation, ResourceError> {
        let account = self
            .accounts
            .get(&request.account)
            .ok_or_else(|| ResourceError::NotFound("resource account is unavailable".to_owned()))?;
        if account.revision != request.expected_account_revision {
            return Err(ResourceError::VersionConflict(
                "resource account rescope expected a stale account revision".to_owned(),
            ));
        }
        // Every transfer that is not terminal holds escrow, and the live
        // root holds every such transfer.
        if self
            .active_transfers
            .iter()
            .filter_map(|id| self.transfers.get(id))
            .any(|transfer| {
                transfer.source == request.account
                    || transfer.destination.as_ref() == Some(&request.account)
            })
        {
            return Err(ResourceError::InvalidLifecycle(
                "a resource account is not rescoped while a transfer that is not terminal names it"
                    .to_owned(),
            ));
        }
        let current = CompletionLockedTargetV1::Account {
            id: request.account.clone(),
            revision: account.revision,
        };
        if self.completion_leases.target_locks.contains_key(&current)
            || self
                .external_completion_participants
                .target_locks
                .contains_key(&current)
        {
            return Err(ResourceError::InvalidLifecycle(
                "a resource account is not rescoped while a completion lease locks its current revision"
                    .to_owned(),
            ));
        }
        if let Some(refusal) = external_refusal {
            return Err(refusal.clone());
        }
        let account = self
            .accounts
            .get_mut(&request.account)
            .expect("checked above");
        account.place_scope.clone_from(&request.place_scope);
        account.revision = account.revision.next()?;
        Ok(AppliedOperation {
            quantity: 0,
            remainder: 0,
            result_ref: None,
            exact_evidence: vec![request.evidence.clone()],
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
            ResourceCreditSourceV1::Production(version)
            | ResourceCreditSourceV1::ExternalInflow(version) => {
                targets.push(CompletionLockedTargetV1::ExternalRecord {
                    version: version.clone(),
                });
            }
        }
        let external_participant = matches!(request.source, ResourceCreditSourceV1::Production(_))
            && self
                .external_completion_participants
                .grants
                .contains_key(&request.completion_certificate.acquisition);
        let (acquisition, grant) = if external_participant {
            // The coordinator's lease locked its runtime record before the
            // execution existed. The credit cites a later exact version of the
            // same record that proves the completion, so the lock is the lower
            // bound of the cited source rather than the source itself.
            if let ResourceCreditSourceV1::Production(source) = &request.source {
                let locked = self.external_locked_production_source(
                    &request.completion_certificate.acquisition,
                    source,
                )?;
                targets.retain(|target| {
                    !matches!(
                        target,
                        CompletionLockedTargetV1::ExternalRecord { version } if version == source
                    )
                });
                targets.push(CompletionLockedTargetV1::ExternalRecord { version: locked });
            }
            self.validate_consumed_external_completion_certificate(
                &request.completion_certificate,
                request.at,
                true,
                &request.operation_key,
                &targets,
            )?
        } else {
            self.consume_completion_certificate(
                &request.completion_certificate,
                request.at,
                &request.operation_key,
                &targets,
                crate::CompletionOperationBindingV1::new(
                    ResourceOperationKind::Credit,
                    request.quantity,
                ),
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
            // The outcome keeps the inflow's evidence, as it keeps a
            // production source and an outflow's authority.
            ResourceCreditSourceV1::ExternalInflow(evidence) => {
                self.conservation.external_inflow = self
                    .conservation
                    .external_inflow
                    .checked_add(u128::from(request.quantity))
                    .ok_or(ResourceError::Overflow)?;
                vec![evidence.clone()]
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
            crate::CompletionOperationBindingV1::new(
                ResourceOperationKind::ExternalOutflow,
                request.quantity,
            ),
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

    /// Points the account at `request.policy`, first installing
    /// `request.install` when the root does not hold it, then retires the
    /// content-addressed revisions nothing refers to any more. Any error
    /// leaves the caller to restore the state it held before.
    fn set_protected_floor(
        &mut self,
        request: &ResourceProtectedFloorRequestV1,
    ) -> Result<AppliedOperation, ResourceError> {
        if let Some(install) = &request.install {
            if request.policy.as_ref() != Some(&install.id) {
                return Err(ResourceError::InvalidDefinition(
                    "protected-floor request must point the account at the revision it installs"
                        .to_owned(),
                ));
            }
            if !install.is_content_addressed() {
                return Err(ResourceError::InvalidDefinition(
                    "a protected-floor policy revision installed at runtime must be content-addressed"
                        .to_owned(),
                ));
            }
            // The identity commits to the content, so a revision the root
            // already holds under it is this one. The comparison guards a
            // root assembled by hand.
            match self.protected_floor_policies.get(&install.id) {
                Some(existing) if existing == install => {}
                Some(_) => {
                    return Err(ResourceError::IdempotencyConflict(
                        "protected-floor policy revision identity is held with other content"
                            .to_owned(),
                    ));
                }
                None => {
                    self.protected_floor_policies
                        .insert(install.id.clone(), install.clone());
                }
            }
        } else if let Some(policy) = &request.policy {
            // Whether the root holds a content-addressed revision depends on
            // what other accounts and demands refer to. Naming one by id
            // alone would let a custodian test that, so it needs its body.
            if policy
                .as_str()
                .starts_with(crate::PROTECTED_FLOOR_POLICY_CONTENT_ID_PREFIX)
            {
                return Err(ResourceError::InvalidDefinition(
                    "a content-addressed protected-floor policy revision is named with its body"
                        .to_owned(),
                ));
            }
            if !self.protected_floor_policies.contains_key(policy) {
                return Err(ResourceError::NotFound(
                    "protected-floor policy revision is unavailable".to_owned(),
                ));
            }
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
        self.retire_unreferenced_protected_floor_policies();
        // Checked after retirement, so replacing a floor at the bound frees
        // the revision it replaces before the new one is counted.
        if self.protected_floor_policies.len() > crate::MAX_PROTECTED_FLOOR_POLICY_REVISIONS {
            return Err(ResourceError::LimitExceeded(
                "protected-floor policy revision capacity is exhausted".to_owned(),
            ));
        }
        Ok(AppliedOperation {
            quantity: self.account_quantities(&request.account)?.protected,
            remainder: 0,
            result_ref: None,
            exact_evidence: Vec::new(),
        })
    }

    /// Removes every content-addressed protected-floor policy revision that
    /// no account and no demand in the root refers to.
    ///
    /// Only a content-addressed revision is retired: its identity commits to
    /// its content, so an archived record that names it stays exact and the
    /// same body installs it again. An authored revision stays, referenced or
    /// not. A demand keeps the revision it cites until the demand leaves the
    /// root, so a revision an archived demand released is retired by the next
    /// floor change.
    fn retire_unreferenced_protected_floor_policies(&mut self) {
        // Identities are ordered, so the content-addressed ones are one
        // contiguous range and authored revisions are never visited.
        let prefix = crate::PROTECTED_FLOOR_POLICY_CONTENT_ID_PREFIX;
        let Ok(first) = ProtectedFloorPolicyRevisionId::new(prefix) else {
            return;
        };
        let candidates: Vec<_> = self
            .protected_floor_policies
            .range(first..)
            .map(|(id, _)| id)
            .take_while(|id| id.as_str().starts_with(prefix))
            .cloned()
            .collect();
        if candidates.is_empty() {
            return;
        }
        // Hot terminal demands and accounts that are not attached refer to
        // policies too; their ledgers know which.
        let referenced: BTreeSet<_> = self
            .accounts
            .values()
            .filter_map(|account| account.protected_floor_policy.as_ref())
            .chain(self.live.account_floor_policies.keys())
            .chain(
                self.demands
                    .values()
                    .filter_map(|demand| demand.protected_floor_policy.as_ref()),
            )
            .chain(self.hot_terminal.demand_floor_policies.keys())
            .collect();
        let retired: Vec<_> = candidates
            .into_iter()
            .filter(|id| {
                !referenced.contains(id)
                    && self
                        .protected_floor_policies
                        .get(id)
                        .is_some_and(ProtectedFloorPolicyRevision::is_content_addressed)
            })
            .collect();
        for id in retired {
            self.protected_floor_policies.remove(&id);
        }
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
        let access_grant = demand.source_policy.access_grant().cloned();
        let mut released = 0_u64;
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
            released = released
                .checked_add(reservation.quantity)
                .ok_or(ResourceError::Overflow)?;
            if let Some(leg) = self.allocation_legs.get_mut(&reservation.allocation_leg) {
                leg.status = AllocationLegStatus::Released;
                leg.revision = leg.revision.next()?;
                leg.semantic_digest.clear();
                leg.semantic_digest = canonical_digest("canwu.resource.allocation-leg.v1", leg)?;
            }
        }
        if let Some(grant_id) = &access_grant
            && released > 0
        {
            self.release_access_grant(grant_id, released)?;
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
        self.mark_terminal_demand_closure(&request.demand, false)?;
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
        if let Some(grant_id) = self
            .demands
            .get(&leg.demand)
            .and_then(|demand| demand.source_policy.access_grant())
            .cloned()
        {
            self.debit_access_grant(&grant_id, leg.quantity)?;
        }
        Ok(())
    }

    /// Records `quantity` of the allocation `leg` as fulfilled on its demand:
    /// the leg's whole quantity for a consumption, what was accepted for a
    /// transfer.
    ///
    /// Only an open demand (`Open` or `PartiallyFulfilled`) changes status
    /// and its due, expiry, and dirty indexes. A demand cancelled, expired, or
    /// rejected for its minimum while a transfer drawn on it was in flight
    /// counts what that transfer delivers in `fulfilled` and keeps its
    /// terminal status.
    fn record_fulfillment(
        &mut self,
        leg: &ResourceAllocationLeg,
        quantity: u64,
        operation_key: ResourceOperationKey,
    ) -> Result<ResourceFulfillment, ResourceError> {
        let (demand_id, remainder, due_at, expires_at, open) = {
            let demand = self.demands.get_mut(&leg.demand).ok_or_else(|| {
                ResourceError::InvalidDefinition("allocation leg lost its demand".to_owned())
            })?;
            demand.fulfilled = demand
                .fulfilled
                .checked_add(quantity)
                .ok_or(ResourceError::Overflow)?;
            if demand.fulfilled > demand.requested {
                return Err(ResourceError::Conservation(
                    "resource demand fulfillment exceeded its requested quantity".to_owned(),
                ));
            }
            let open = matches!(
                demand.status,
                DemandStatus::Open | DemandStatus::PartiallyFulfilled
            );
            if open {
                demand.status = if demand.fulfilled == demand.requested {
                    DemandStatus::Fulfilled
                } else {
                    DemandStatus::PartiallyFulfilled
                };
            }
            demand.revision = demand.revision.next()?;
            (
                demand.id.clone(),
                demand.remainder(),
                demand.due_at,
                demand.expires_at,
                open,
            )
        };
        // A terminal demand left its indexes when it became terminal.
        if open && remainder == 0 {
            if let Some(ids) = self.demand_due_index.get_mut(&due_at) {
                ids.remove(&demand_id);
            }
            if let Some(ids) = self.demand_expiry_index.get_mut(&expires_at) {
                ids.remove(&demand_id);
            }
            self.demand_due_index.retain(|_, ids| !ids.is_empty());
            self.demand_expiry_index.retain(|_, ids| !ids.is_empty());
            self.dirty_demands.remove(&demand_id);
        } else if open {
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
            consumed_quantity: quantity,
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
        self.add_hot_candidate(
            fulfillment.terminal_sequence,
            crate::ResourceTerminalRecordKeyV1::Fulfillment(fulfillment.id.clone()),
        )?;
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
            // A batch leaves more terminal records than its lease reserves,
            // so it must leave the reservations of every other lease free;
            // its own lease has finished by the time room is measured.
            ResourceOperationRequestV1::ConsumeBatch(_) => false,
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
            ResourceOperationRequestV1::RecordLoss(request) => {
                self.completion_leases
                    .certificate(&request.completion_certificate.acquisition)
                    == Some(&request.completion_certificate)
            }
            ResourceOperationRequestV1::RecordTransferLoss(request) => {
                self.completion_leases
                    .certificate(&request.completion_certificate.acquisition)
                    == Some(&request.completion_certificate)
            }
            ResourceOperationRequestV1::BeginExchange(request) => {
                [&request.leg_a, &request.leg_b].iter().all(|leg| {
                    self.completion_leases
                        .certificate(&leg.completion_certificate.acquisition)
                        == Some(&leg.completion_certificate)
                })
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
            ResourceOperationRequestV1::Completion(
                ResourceCompletionOperationV1::Acquire(_)
                | ResourceCompletionOperationV1::SingleStepLease(_),
            ) => false,
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
            | ResourceOperationRequestV1::RecordObservation(_)
            | ResourceOperationRequestV1::IssueAccessGrant(_)
            | ResourceOperationRequestV1::RevokeAccessGrant(_)
            | ResourceOperationRequestV1::RescopeAccount(_) => false,
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
        self.add_hot_candidate(
            sequence,
            crate::ResourceTerminalRecordKeyV1::LeaseReceipt(receipt.sequence),
        )?;
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

    /// Consumes the lease `certificate` certifies for the operation
    /// `redeemed`, the kind, quantity, and (for a terminal transfer
    /// disposition) disposition of the operation that redeems it, which a
    /// lease bound to an operation must admit
    /// ([`crate::CompletionOperationBindingV1::admits`]).
    fn consume_completion_certificate(
        &mut self,
        certificate: &CompletionLeaseActivationCertificateV1,
        at: SimTime,
        operation_key: &ResourceOperationKey,
        required_targets: &[CompletionLockedTargetV1],
        redeemed: crate::CompletionOperationBindingV1,
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
        // The binding is read from the acquisition the root holds, whose
        // envelope digest the certificate must carry.
        if acquisition
            .eligibility_envelope
            .operation_binding
            .is_some_and(|bound| {
                !bound.admits(redeemed.kind, redeemed.disposition, Some(redeemed.quantity))
            })
        {
            return Err(lease_bound_elsewhere());
        }
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

    /// Validates an operation authorized by an external participant grant
    /// that its coordinator already consumed at the lease eligibility time.
    /// An input consumption is part of that consumption and must happen at
    /// exactly that time; a terminal output credit settles the consumed grant
    /// later, at or after it (`settles_after_eligibility`).
    fn validate_consumed_external_completion_certificate(
        &self,
        certificate: &CompletionLeaseActivationCertificateV1,
        at: SimTime,
        settles_after_eligibility: bool,
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
            || (if settles_after_eligibility {
                at < participant.eligibility_time
            } else {
                at != participant.eligibility_time
            })
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

    fn issue_access_grant(
        &mut self,
        request: &ResourceIssueAccessGrantRequestV1,
    ) -> Result<AppliedOperation, ResourceError> {
        let grant = &request.grant;
        if self.access_grants.contains_key(&grant.grant_id) {
            return Err(ResourceError::IdempotencyConflict(
                "resource access grant identity already exists".to_owned(),
            ));
        }
        if self.access_grant_count() >= crate::MAX_RESOURCE_ACCESS_GRANTS {
            return Err(ResourceError::LimitExceeded(
                "resource access grant capacity is exhausted".to_owned(),
            ));
        }
        self.validate_access_grant_contract(grant)?;
        self.access_grants.insert(
            grant.grant_id.clone(),
            crate::ResourceAccessGrantRecordV1 {
                grant: grant.clone(),
                revision: ResourceRevision::INITIAL,
                status: crate::ResourceAccessGrantStatusV1::Active,
                reserved_quantity: 0,
                debited_quantity: 0,
            },
        );
        Ok(AppliedOperation {
            quantity: grant.cap_quantity,
            remainder: grant.cap_quantity,
            result_ref: None,
            exact_evidence: vec![grant.authority_evidence.clone()],
        })
    }

    fn revoke_access_grant(
        &mut self,
        request: &ResourceRevokeAccessGrantRequestV1,
    ) -> Result<AppliedOperation, ResourceError> {
        let record = self
            .access_grants
            .get_mut(&request.grant_id)
            .ok_or_else(|| {
                ResourceError::NotFound("resource access grant is unavailable".to_owned())
            })?;
        if record.revision != request.expected_grant_revision {
            return Err(ResourceError::VersionConflict(
                "resource access grant revocation expected a stale revision".to_owned(),
            ));
        }
        if record.status != crate::ResourceAccessGrantStatusV1::Active || record.is_drawn() {
            return Err(ResourceError::InvalidLifecycle(
                "resource access grant is revoked or fixed by a reservation".to_owned(),
            ));
        }
        record.status = crate::ResourceAccessGrantStatusV1::Revoked;
        record.revision = record.revision.next()?;
        Ok(AppliedOperation {
            quantity: 0,
            remainder: record.remaining(),
            result_ref: None,
            exact_evidence: Vec::new(),
        })
    }

    fn validate_access_grant_contract(
        &self,
        grant: &crate::ResourceAccessGrantV1,
    ) -> Result<(), ResourceError> {
        grant.validate_shape()?;
        let definition = self
            .definitions
            .get(&grant.resource_revision)
            .ok_or_else(|| {
                ResourceError::NotFound(
                    "resource access grant definition revision is unavailable".to_owned(),
                )
            })?;
        if definition.canonical_unit != grant.unit_revision
            || !self.units.contains_key(&grant.unit_revision)
        {
            return Err(ResourceError::InvalidDefinition(
                "resource access grant unit is not its resource's exact canonical unit".to_owned(),
            ));
        }
        Ok(())
    }

    fn access_grant_mut(
        &mut self,
        grant_id: &crate::ResourceAccessGrantId,
    ) -> Result<&mut crate::ResourceAccessGrantRecordV1, ResourceError> {
        self.access_grants.get_mut(grant_id).ok_or_else(|| {
            ResourceError::InvalidDefinition("resource demand lost its access grant".to_owned())
        })
    }

    /// Charges a new reservation to its grant's cap.
    fn reserve_access_grant(
        &mut self,
        grant_id: &crate::ResourceAccessGrantId,
        quantity: u64,
    ) -> Result<(), ResourceError> {
        let record = self.access_grant_mut(grant_id)?;
        if quantity > record.remaining() {
            return Err(ResourceError::Conservation(
                "resource allocation would exceed its access grant's cap".to_owned(),
            ));
        }
        record.reserved_quantity = record
            .reserved_quantity
            .checked_add(quantity)
            .ok_or(ResourceError::Overflow)?;
        record.revision = record.revision.next()?;
        Ok(())
    }

    /// Returns a released or expired reservation's quantity to its grant.
    fn release_access_grant(
        &mut self,
        grant_id: &crate::ResourceAccessGrantId,
        quantity: u64,
    ) -> Result<(), ResourceError> {
        let record = self.access_grant_mut(grant_id)?;
        record.reserved_quantity =
            record
                .reserved_quantity
                .checked_sub(quantity)
                .ok_or_else(|| {
                    ResourceError::Conservation(
                        "resource access grant reservation accounting underflowed".to_owned(),
                    )
                })?;
        record.revision = record.revision.next()?;
        Ok(())
    }

    /// Moves a debited reservation's quantity from reserved to debited, so
    /// the grant charges each unit exactly once.
    fn debit_access_grant(
        &mut self,
        grant_id: &crate::ResourceAccessGrantId,
        quantity: u64,
    ) -> Result<(), ResourceError> {
        let record = self.access_grant_mut(grant_id)?;
        record.reserved_quantity =
            record
                .reserved_quantity
                .checked_sub(quantity)
                .ok_or_else(|| {
                    ResourceError::Conservation(
                        "resource access grant debit exceeds its reservations".to_owned(),
                    )
                })?;
        record.debited_quantity = record
            .debited_quantity
            .checked_add(quantity)
            .ok_or(ResourceError::Overflow)?;
        record.revision = record.revision.next()?;
        Ok(())
    }

    /// Authority for debiting an allocation that was reserved under an access
    /// grant. The grantor's consent is the grant; the debit itself must run
    /// under the grantee's own completion lease while the grant is current.
    /// A lease held by anyone else, including the grantor custodian, cannot
    /// debit a granted allocation. Returns the grant, or `None` for an
    /// allocation that no grant backs.
    ///
    /// `at` is the debit's certified lease time and `settled_at` the boundary
    /// time that settles it (`None` for a detached caller, whose request time
    /// stands in). An activated lease does not expire and demand expiry is
    /// lazy, so the grant must be current when the debit settles, and a
    /// granted debit settles only at its certified time, as adapter ingress
    /// already requires of every irreversible operation.
    fn granted_debit_authority(
        &self,
        leg: &ResourceAllocationLeg,
        at: SimTime,
        settled_at: Option<SimTime>,
        certificate: &CompletionLeaseActivationCertificateV1,
    ) -> Result<Option<crate::ResourceAccessGrantId>, ResourceError> {
        let Some(grant_id) = self
            .demands
            .get(&leg.demand)
            .and_then(|demand| demand.source_policy.access_grant())
        else {
            return Ok(None);
        };
        let record = self.access_grants.get(grant_id).ok_or_else(|| {
            ResourceError::InvalidDefinition("resource demand lost its access grant".to_owned())
        })?;
        let settled_at = settled_at.unwrap_or(at);
        if record.status != crate::ResourceAccessGrantStatusV1::Active
            || !record.grant.is_current_at(at)
            || !record.grant.is_current_at(settled_at)
        {
            return Err(ResourceError::Authority(
                "resource access grant is not current at the debit time".to_owned(),
            ));
        }
        if settled_at != at {
            return Err(ResourceError::Authority(
                "a granted allocation is debited only at its certified lease time".to_owned(),
            ));
        }
        let lease_holder = self
            .completion_leases
            .acquisitions
            .get(&certificate.acquisition)
            .map(|acquisition| &acquisition.holder)
            .or_else(|| {
                self.external_completion_participants
                    .grants
                    .get(&certificate.acquisition)
                    .map(|participant| &participant.holder)
            });
        if lease_holder != Some(&record.grant.grantee) {
            return Err(ResourceError::Authority(
                "a granted allocation is debited only under the grantee's completion lease"
                    .to_owned(),
            ));
        }
        Ok(Some(grant_id.clone()))
    }

    /// Holder whose tracked command may debit an allocation leg: the grantee
    /// for a leg reserved under an access grant, otherwise the source custodian.
    pub(crate) fn allocation_debit_holder(
        &self,
        allocation: &ResourceAllocationLegVersionV1,
    ) -> Option<canwu_api::KnowledgeHolderRef> {
        let granted = self
            .allocation_legs
            .get(&allocation.id)
            .and_then(|leg| self.demands.get(&leg.demand))
            .and_then(|demand| {
                demand
                    .source_policy
                    .access_grant()
                    .map(|_| demand.requester.clone())
            });
        granted.or_else(|| {
            self.accounts
                .get(&allocation.account)
                .map(|account| account.custodian.clone())
        })
    }

    /// Holder controlling a transfer's cancellation, return, and loss: the
    /// grantee of the access grant it was started under, otherwise the source
    /// custodian.
    pub(crate) fn transfer_controller(
        &self,
        transfer: &ResourceTransfer,
    ) -> Option<canwu_api::KnowledgeHolderRef> {
        match &transfer.access_grant {
            Some(grant_id) => self
                .access_grants
                .get(grant_id)
                .map(|record| record.grant.grantee.clone()),
            None => self
                .accounts
                .get(&transfer.source)
                .map(|account| account.custodian.clone()),
        }
    }

    /// The refusal of a tracked command that redeems a lease of another
    /// plugin: one whose expected participants name a plugin other than the
    /// resource plugin, or in which such a plugin holds a grant. Such a lease
    /// authorizes that plugin's operation, which reaches the root through its
    /// adapter ingress, never a holder's command. Checking the expected
    /// participants, not only the grants, refuses a command sent while the
    /// other plugin's grant is still on its way.
    ///
    /// The plugin refuses such a command at admission, and skips it at
    /// settlement if the lease appeared in between; either way the command
    /// takes no operation key, so the other plugin's operation still settles
    /// under it. `None` when the request redeems no such lease the root holds.
    pub(crate) fn tracked_foreign_lease_refusal(
        &self,
        request: &ResourceOperationRequestV1,
    ) -> Option<ResourceError> {
        request.lease_redemptions().into_iter().find_map(|redemption| {
            let acquisition = self
                .completion_leases
                .acquisitions
                .get(&redemption.certificate.acquisition)?;
            let foreign = acquisition
                .expected_participants
                .iter()
                .any(|participant| participant != crate::PLUGIN_NAME)
                || acquisition.grants.values().any(|grant| {
                    self.completion_leases
                        .grants
                        .get(grant)
                        .is_some_and(|grant| grant.owner_plugin != crate::PLUGIN_NAME)
                });
            foreign.then(|| {
                ResourceError::Authority(
                    "a tracked command redeems only a completion lease whose only participant is the resource plugin"
                        .to_owned(),
                )
            })
        })
    }

    /// The refusal of a tracked command that redeems a completion lease it
    /// may not redeem, or of a tracked `Acquire` that writes a binding:
    ///
    /// - a lease bound to another operation, as far as the request alone
    ///   shows it (kind, disposition, and the quantity when the request fixes
    ///   it; the redemption checks the rest);
    /// - for an external inflow `Credit`, a lease that is not bound to an
    ///   operation ([`crate::CompletionOperationBindingV1`]); and
    /// - an `Acquire` whose envelope carries a binding: only the issuer of a
    ///   lease, the host application or a plugin that coordinates it, binds
    ///   it.
    ///
    /// The plugin refuses a redemption at admission, so it takes no key, and
    /// again at settlement, as a recorded rejection, if the lease changed in
    /// between; the `Acquire` is its recorded rejection. A lease of another
    /// plugin is [`Self::tracked_foreign_lease_refusal`]. `None` when the
    /// request redeems no lease the root holds; the operation then refuses
    /// itself.
    pub(crate) fn tracked_redemption_refusal(
        &self,
        request: &ResourceOperationRequestV1,
    ) -> Option<ResourceError> {
        if let ResourceOperationRequestV1::Completion(ResourceCompletionOperationV1::Acquire(
            acquisition,
        )) = request
            && acquisition.eligibility_envelope.operation_binding.is_some()
        {
            return Some(ResourceError::Authority(
                "a holder's completion lease acquisition carries no operation binding; the lease's issuer binds it"
                    .to_owned(),
            ));
        }
        for redemption in request.lease_redemptions() {
            let Some(acquisition) = self
                .completion_leases
                .acquisitions
                .get(&redemption.certificate.acquisition)
            else {
                continue;
            };
            match acquisition.eligibility_envelope.operation_binding {
                Some(binding)
                    if !binding.admits(
                        redemption.kind,
                        redemption.disposition,
                        redemption.quantity,
                    ) =>
                {
                    return Some(lease_bound_elsewhere());
                }
                None if matches!(
                    request,
                    ResourceOperationRequestV1::Credit(ResourceCreditRequestV1 {
                        source: ResourceCreditSourceV1::ExternalInflow(_),
                        ..
                    })
                ) =>
                {
                    return Some(ResourceError::Authority(
                        "a tracked external inflow credit requires a completion lease bound to exactly that credit"
                            .to_owned(),
                    ));
                }
                _ => {}
            }
        }
        None
    }

    /// Returns the one coordinator record version that an external
    /// participant grant locked for `source.record`, provided `source` is that
    /// version or a later version of the same record.
    ///
    /// Invariant: a production output credit is authorized by the consumed
    /// resource participant grant of the execution's completion lease. That
    /// lease locks the coordinator runtime at the version current when it was
    /// granted, which predates the execution. Every later coordinator
    /// transition, including the completion itself, advances the record, so
    /// the credit must cite a version that is no older than the locked one
    /// (the coordinator plugin additionally proves that the cited body holds
    /// the completed execution with the same certificate). Citing an older
    /// version, another record, or an unlocked record is rejected.
    fn external_locked_production_source(
        &self,
        acquisition: &CompletionLeaseAcquisitionId,
        source: &DomainRecordVersionRef,
    ) -> Result<DomainRecordVersionRef, ResourceError> {
        let participant = self
            .external_completion_participants
            .grants
            .get(acquisition)
            .ok_or_else(|| {
                ResourceError::NotFound(
                    "external completion participant grant is unavailable".to_owned(),
                )
            })?;
        let mut locked =
            participant
                .grant
                .target_versions
                .iter()
                .filter_map(|target| match target {
                    CompletionLockedTargetV1::ExternalRecord { version }
                        if version.record == source.record =>
                    {
                        Some(version)
                    }
                    _ => None,
                });
        match (locked.next(), locked.next()) {
            (Some(version), None) if version.version <= source.version => Ok(version.clone()),
            _ => Err(ResourceError::VersionConflict(
                "production credit source is not the locked coordinator record at or after its locked version"
                    .to_owned(),
            )),
        }
    }

    /// What the active reservations of a demand hold. A live demand's
    /// reservations are all in the root, indexed by demand.
    /// The escrow of the open transfers drawn on each demand: what an
    /// allocation already moved toward the demand, which its acceptance
    /// fulfils and a loss or a return gives back. A returning transfer
    /// (`ReturnPending`) is never accepted, so its escrow does not count.
    ///
    /// The live root holds every open transfer and, while a demand is open,
    /// every allocation leg drawn on it, so the totals of open demands are
    /// whole in a partial state. A leg that is not attached belongs to a
    /// terminal demand, which no allocation pass reads, and is passed over.
    fn in_flight_by_demand(&self) -> Result<BTreeMap<ResourceDemandId, u64>, ResourceError> {
        let mut in_flight = BTreeMap::<ResourceDemandId, u64>::new();
        for transfer in self
            .active_transfers
            .iter()
            .filter_map(|id| self.transfers.get(id))
            .filter(|transfer| transfer.state != ResourceTransferState::ReturnPending)
        {
            let Some(leg) = self.allocation_legs.get(&transfer.allocation_leg) else {
                continue;
            };
            let total = in_flight.entry(leg.demand.clone()).or_default();
            *total = total
                .checked_add(transfer.escrow)
                .ok_or(ResourceError::Overflow)?;
        }
        Ok(in_flight)
    }

    fn active_reserved_for_demand(
        &self,
        demand_id: &ResourceDemandId,
    ) -> Result<u64, ResourceError> {
        self.reservation_by_demand
            .get(demand_id)
            .into_iter()
            .flatten()
            .filter_map(|id| self.reservations.get(id))
            .filter(|reservation| reservation.status == ReservationStatus::Active)
            .try_fold(0_u64, |total, reservation| {
                total
                    .checked_add(reservation.quantity)
                    .ok_or(ResourceError::Overflow)
            })
    }

    /// Supply one source account offers a demand. For a granted demand the
    /// account belongs to another custodian, so a protected-floor policy the
    /// grantor changed after admission withholds that account's supply
    /// instead of failing the grantee's whole allocation pass.
    fn available_for_source(
        &self,
        account_id: &ResourceAccountId,
        demand: &ResourceDemand,
        reserved: u64,
    ) -> Result<u64, ResourceError> {
        if demand.source_policy.access_grant().is_some()
            && self.accounts.get(account_id).is_some_and(|account| {
                account.protected_floor_policy != demand.protected_floor_policy
            })
        {
            return Ok(0);
        }
        self.available_for_demand(account_id, demand, reserved)
    }

    fn available_for_demand(
        &self,
        account_id: &ResourceAccountId,
        demand: &ResourceDemand,
        reserved: u64,
    ) -> Result<u64, ResourceError> {
        let account = self
            .accounts
            .get(account_id)
            .ok_or_else(|| ResourceError::NotFound("resource account is unavailable".to_owned()))?;
        let quantities = self.quantities_with_reserved(account, reserved);
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
            let mut released = 0_u64;
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
                released = released
                    .checked_add(reservation.quantity)
                    .ok_or(ResourceError::Overflow)?;
                if let Some(leg) = self.allocation_legs.get_mut(&reservation.allocation_leg) {
                    leg.status = AllocationLegStatus::Expired;
                    leg.revision = leg.revision.next()?;
                    leg.semantic_digest.clear();
                    leg.semantic_digest =
                        canonical_digest("canwu.resource.allocation-leg.v1", leg)?;
                }
            }
            if let Some(grant_id) = snapshot.source_policy.access_grant()
                && released > 0
            {
                self.release_access_grant(grant_id, released)?;
            }
            if let Some(ids) = self.demand_due_index.get_mut(&snapshot.due_at) {
                ids.remove(&demand_id);
            }
            if let Some(ids) = self.demand_expiry_index.get_mut(&snapshot.expires_at) {
                ids.remove(&demand_id);
            }
            self.dirty_demands.remove(&demand_id);
            self.mark_terminal_demand_closure(&demand_id, false)?;
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

    pub(crate) fn validate_demand_sources(
        &self,
        demand: &ResourceDemand,
    ) -> Result<(), ResourceError> {
        self.validate_demand_sources_within(demand, false)
    }

    /// [`Self::validate_demand_sources`]; with `partial`, a source account or
    /// grant that is not attached to a partial state is not checked.
    fn validate_demand_sources_within(
        &self,
        demand: &ResourceDemand,
        partial: bool,
    ) -> Result<(), ResourceError> {
        demand.source_policy.validate_shape()?;
        let skip_accounts = partial && self.live.accounts > 0;
        if let ResourceDemandSourcePolicyV1::ExactAccounts(accounts) = &demand.source_policy {
            for id in accounts {
                let Some(account) = self.accounts.get(id) else {
                    if skip_accounts {
                        continue;
                    }
                    return Err(ResourceError::NotFound(
                        "resource demand source account is unavailable".to_owned(),
                    ));
                };
                if account.custodian != demand.requester {
                    return Err(ResourceError::Authority(
                        "resource demand source account is not custodied by its requester"
                            .to_owned(),
                    ));
                }
                if account.closed
                    || account.resource_revision != demand.resource_revision
                    || account.unit_revision != demand.unit_revision
                {
                    return Err(ResourceError::InvalidDefinition(
                        "resource demand source account resource or unit differs".to_owned(),
                    ));
                }
            }
        }
        if let ResourceDemandSourcePolicyV1::Granted { grant_id, accounts } = &demand.source_policy
        {
            let Some(record) = self.access_grants.get(grant_id) else {
                if partial && self.live.access_grants > 0 {
                    return Ok(());
                }
                return Err(ResourceError::NotFound(
                    "resource demand access grant is unavailable".to_owned(),
                ));
            };
            let grant = &record.grant;
            if grant.grantee != demand.requester {
                return Err(ResourceError::Authority(
                    "resource demand access grant was not issued to its requester".to_owned(),
                ));
            }
            if grant.resource_revision != demand.resource_revision
                || grant.unit_revision != demand.unit_revision
                || demand.due_at < grant.valid_from
                || demand.expires_at > grant.valid_until
            {
                return Err(ResourceError::InvalidDefinition(
                    "resource demand resource, unit, or window exceeds its access grant".to_owned(),
                ));
            }
            for id in accounts {
                let Some(account) = self.accounts.get(id) else {
                    if skip_accounts {
                        continue;
                    }
                    return Err(ResourceError::NotFound(
                        "resource demand source account is unavailable".to_owned(),
                    ));
                };
                if account.custodian != grant.grantor_custodian {
                    return Err(ResourceError::Authority(
                        "resource demand granted account is not custodied by the grantor"
                            .to_owned(),
                    ));
                }
                if account.closed
                    || account.resource_revision != grant.resource_revision
                    || account.unit_revision != grant.unit_revision
                {
                    return Err(ResourceError::InvalidDefinition(
                        "resource demand granted account resource or unit differs".to_owned(),
                    ));
                }
            }
        }
        Ok(())
    }

    /// Admission rules for a new or amended demand beyond its static source
    /// contract: a `Granted` demand needs an active grant, and a demand that
    /// has not yet drawn on its grant may not request more than the grant's
    /// remaining cap.
    pub(crate) fn validate_demand_admission(
        &self,
        demand: &ResourceDemand,
        drawn: bool,
    ) -> Result<(), ResourceError> {
        self.validate_demand_sources(demand)?;
        let Some(grant_id) = demand.source_policy.access_grant() else {
            return Ok(());
        };
        let record = &self.access_grants[grant_id];
        if record.status != crate::ResourceAccessGrantStatusV1::Active {
            return Err(ResourceError::InvalidLifecycle(
                "resource demand access grant is revoked".to_owned(),
            ));
        }
        if !drawn && demand.requested > record.remaining() {
            return Err(ResourceError::Capacity(
                "resource demand exceeds its access grant's remaining cap".to_owned(),
            ));
        }
        Ok(())
    }

    fn validate_demand_contract(&self, demand: &ResourceDemand) -> Result<(), ResourceError> {
        self.validate_demand_contract_within(demand, false)
    }

    /// [`Self::validate_demand_contract`]; with `partial`, a source account,
    /// grant, definition, or unit that is not attached to a partial state is
    /// not checked.
    fn validate_demand_contract_within(
        &self,
        demand: &ResourceDemand,
        partial: bool,
    ) -> Result<(), ResourceError> {
        self.validate_demand_sources_within(demand, partial)?;
        let definition_known = self.definitions.contains_key(&demand.resource_revision)
            || (partial && self.live.definitions > 0);
        let unit_known =
            self.units.contains_key(&demand.unit_revision) || (partial && self.live.units > 0);
        if demand.requested == 0
            || demand.minimum_useful == 0
            || demand.minimum_useful > demand.requested
            || demand.due_at >= demand.expires_at
            || !definition_known
            || !unit_known
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

    /// Transfers of the root, live and hot terminal, attached or not: what
    /// `max_transfers` counts when a transfer starts.
    fn hot_transfer_count(&self) -> Result<usize, ResourceError> {
        let attached_terminal = self
            .terminal_archive_candidates
            .values()
            .filter(|key| matches!(key, crate::ResourceTerminalRecordKeyV1::Transfer(_)))
            .count();
        self.transfers
            .len()
            .checked_sub(attached_terminal)
            .and_then(|live| {
                usize::try_from(self.hot_terminal.transfers)
                    .ok()
                    .and_then(|hot| live.checked_add(hot))
            })
            .ok_or(ResourceError::Overflow)
    }

    fn next_sequence(&mut self) -> Result<u64, ResourceError> {
        let current = self.next_admitted_sequence;
        self.next_admitted_sequence = current.checked_add(1).ok_or(ResourceError::Overflow)?;
        Ok(current)
    }

    pub fn validate(&self) -> Result<(), ResourceError> {
        self.validate_within(self.limits.max_state_bytes)
    }

    /// [`Self::validate`] with `max_state_bytes` as the byte cap of the
    /// encoded root.
    #[allow(clippy::too_many_lines)]
    fn validate_within(&self, max_state_bytes: usize) -> Result<(), ResourceError> {
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
        if self.definition_count() > self.limits.max_definitions
            || self.unit_count() > self.limits.max_unit_revisions
            || self.protected_floor_policies.len() > crate::MAX_PROTECTED_FLOOR_POLICY_REVISIONS
            || self.account_count() > self.limits.max_accounts
            || active_demands > self.limits.max_demands
            || active_reservations > self.limits.max_demands
            || active_allocation_legs > self.limits.max_demands
            || self.active_transfers.len() > self.limits.max_transfers
            || self.observation_heads.len() > self.limits.max_observation_heads
            || self.dirty_demands.len() > self.limits.max_dirty_demands
        {
            return Err(ResourceError::LimitExceeded(
                "resource authoritative records exceed configured limits".to_owned(),
            ));
        }

        // A partial state validates its live root and its attached records.
        // A reference from a live-root record to a bucketed record that is
        // not attached is not checked here: that record is unchanged since it
        // was last written, and a whole state checks every reference
        // (activation, restore, `resource_state`, and the debug-build
        // equivalence check).
        let reserved_by_account = self.reserved_by_account()?;
        for (id, account) in &self.accounts {
            if id != &account.id {
                return Err(ResourceError::InvalidDefinition(
                    "resource account map key differs from its identity".to_owned(),
                ));
            }
            self.validate_account_contract(account)?;
            if reserved_by_account.get(id).copied().unwrap_or(0) > account.balance {
                return Err(ResourceError::Conservation(
                    "resource reservations exceed the authoritative account balance".to_owned(),
                ));
            }
        }
        // The custodian index lists exactly each custodian's accounts, as far
        // as both are attached.
        for (id, account) in &self.accounts {
            let indexed = match self.custodian_accounts.get(&account.custodian) {
                Some(accounts) => accounts.contains(id),
                None => self.live.custodians > 0,
            };
            if !indexed {
                return Err(ResourceError::InvalidDefinition(
                    "resource account is missing from its custodian's index".to_owned(),
                ));
            }
        }
        for (holder, accounts) in &self.custodian_accounts {
            if accounts.is_empty()
                || accounts.iter().any(|id| match self.accounts.get(id) {
                    Some(account) => &account.custodian != holder,
                    None => self.live.accounts == 0,
                })
            {
                return Err(ResourceError::InvalidDefinition(
                    "resource custodian index differs from its accounts".to_owned(),
                ));
            }
        }
        for (id, demand) in &self.demands {
            if id != &demand.id || demand.fulfilled > demand.requested {
                return Err(ResourceError::InvalidDefinition(
                    "resource demand identity or fulfillment is invalid".to_owned(),
                ));
            }
            self.validate_demand_contract_within(demand, true)?;
        }
        // An ended demand stays out of the archive candidates only while its
        // closure is held back ([`Self::demand_closure_held`], the rule
        // [`Self::mark_terminal_demand_closure`] reads); any other is a
        // closure that leaked.
        let candidate_demands: BTreeSet<_> = self
            .terminal_archive_candidates
            .values()
            .filter_map(|key| match key {
                crate::ResourceTerminalRecordKeyV1::Demand(id) => Some(id),
                _ => None,
            })
            .collect();
        let open_transfer_legs = self.open_transfer_legs();
        for demand in self.demands.values() {
            if !matches!(
                demand.status,
                DemandStatus::Open | DemandStatus::PartiallyFulfilled
            ) && !candidate_demands.contains(&demand.id)
                && !self.demand_closure_held(&demand.id, &open_transfer_legs)?
            {
                return Err(ResourceError::InvalidDefinition(
                    "an ended resource demand is neither an archive candidate nor held back by a reservation or a transfer in flight"
                        .to_owned(),
                ));
            }
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
            let demand = self.demands.get(&reservation.demand);
            if (reservation.status == ReservationStatus::Active && demand.is_none())
                || demand.is_some_and(|demand| !demand.source_policy.permits(&reservation.account))
            {
                return Err(ResourceError::InvalidDefinition(
                    "resource reservation violates demand source policy or lost its active demand"
                        .to_owned(),
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
        self.validate_access_grants()?;
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
            if !transfer_link_history_is_valid(transfer) {
                return Err(ResourceError::InvalidDefinition(
                    "resource transfer link history is not one its advances could record"
                        .to_owned(),
                ));
            }
        }
        for (id, loss) in &self.losses {
            if id != &loss.id
                || loss.quantity == 0
                || loss.account.is_some() == loss.transfer.is_some()
            {
                return Err(ResourceError::InvalidDefinition(
                    "resource loss identity, quantity, or account/transfer target is invalid"
                        .to_owned(),
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
            // The fulfillment a consumption names is the one it recorded: a
            // reader attaches it by that name. A fulfillment is recorded
            // after its consumption and archived after it, so a whole root
            // that holds the consumption holds the fulfillment too.
            match self.fulfillments.get(&consumption.fulfillment) {
                Some(fulfillment) => {
                    if !consumption_names_its_fulfillment(consumption, fulfillment) {
                        return Err(ResourceError::InvalidDefinition(
                            "resource consumption names a fulfillment of another consumption"
                                .to_owned(),
                        ));
                    }
                }
                None if self.hot_terminal_records_attached() => {
                    return Err(ResourceError::InvalidDefinition(
                        "resource consumption names a fulfillment the root does not hold"
                            .to_owned(),
                    ));
                }
                None => {}
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
            if !crate::holder_kind_can_hold_knowledge(&grant.holder) {
                return Err(ResourceError::InvalidDefinition(
                    "resource report grant holder is of a kind that can hold no knowledge"
                        .to_owned(),
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
                    match self.accounts.get(&value.account) {
                        Some(account) => self
                            .definitions
                            .get(&account.resource_revision)
                            .is_none_or(|definition| definition.scope != value.scope),
                        // Checked when the head was recorded, with the
                        // account attached.
                        None => self.live.accounts == 0,
                    }
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
        self.validate_archive_maintenance_candidacy()?;
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
        if self.identity_evidence != self.computed_identity_evidence() {
            return Err(ResourceError::InvalidDefinition(
                "resource identity-only evidence dependencies differ from the live root".to_owned(),
            ));
        }
        // The hot terminal budgets, and the ledger against the attached
        // records.
        self.validate_hot_terminal_ledger()?;
        if self.encoded_root_bytes()? > max_state_bytes {
            return Err(ResourceError::LimitExceeded(
                "resource authoritative state byte cap is exhausted".to_owned(),
            ));
        }
        self.validate_conservation()
    }

    /// Access grant identity, shape, and cap accounting: every grant's
    /// reserved quantity equals the active reservations of the demands drawn
    /// under it, its debited quantity covers at least the hot transfers and
    /// consumptions debited under it, and reserved plus debited never exceeds
    /// its cap. While a transfer's allocation leg and demand are hot, the
    /// transfer names exactly the grant that demand drew on.
    fn validate_access_grants(&self) -> Result<(), ResourceError> {
        if self.access_grant_count() > crate::MAX_RESOURCE_ACCESS_GRANTS {
            return Err(ResourceError::LimitExceeded(
                "resource access grants exceed their bound".to_owned(),
            ));
        }
        let mut reserved = BTreeMap::<&crate::ResourceAccessGrantId, u64>::new();
        for reservation in self
            .reservations
            .values()
            .filter(|reservation| reservation.status == ReservationStatus::Active)
        {
            if let Some(grant_id) = self
                .demands
                .get(&reservation.demand)
                .and_then(|demand| demand.source_policy.access_grant())
            {
                let total = reserved.entry(grant_id).or_default();
                *total = total
                    .checked_add(reservation.quantity)
                    .ok_or(ResourceError::Overflow)?;
            }
        }
        // Debits never return to a grant, so the hot debit records are a
        // lower bound on its debited quantity; archived ones are not counted.
        let mut debited = BTreeMap::<&crate::ResourceAccessGrantId, u64>::new();
        for transfer in self.transfers.values() {
            let demand_grant = self
                .allocation_legs
                .get(&transfer.allocation_leg)
                .and_then(|leg| self.demands.get(&leg.demand))
                .map(|demand| demand.source_policy.access_grant());
            if demand_grant.is_some_and(|expected| expected != transfer.access_grant.as_ref()) {
                return Err(ResourceError::InvalidDefinition(
                    "resource transfer access grant differs from the grant its demand drew on"
                        .to_owned(),
                ));
            }
            if let Some(grant_id) = &transfer.access_grant {
                if !self.access_grants.contains_key(grant_id) && self.live.access_grants == 0 {
                    return Err(ResourceError::InvalidDefinition(
                        "resource transfer names an unavailable access grant".to_owned(),
                    ));
                }
                let total = debited.entry(grant_id).or_default();
                *total = total
                    .checked_add(transfer.quantity)
                    .ok_or(ResourceError::Overflow)?;
            }
        }
        for consumption in self.consumptions.values() {
            if let Some(grant_id) = self
                .demands
                .get(&consumption.demand)
                .and_then(|demand| demand.source_policy.access_grant())
            {
                let total = debited.entry(grant_id).or_default();
                *total = total
                    .checked_add(consumption.quantity)
                    .ok_or(ResourceError::Overflow)?;
            }
        }
        for (id, record) in &self.access_grants {
            if id != &record.grant.grant_id {
                return Err(ResourceError::InvalidDefinition(
                    "resource access grant map key differs from its identity".to_owned(),
                ));
            }
            self.validate_access_grant_contract(&record.grant)?;
            let charged = record
                .reserved_quantity
                .checked_add(record.debited_quantity)
                .ok_or(ResourceError::Overflow)?;
            if charged > record.grant.cap_quantity
                || record.reserved_quantity != reserved.get(id).copied().unwrap_or(0)
                || record.debited_quantity < debited.get(id).copied().unwrap_or(0)
                || (record.status == crate::ResourceAccessGrantStatusV1::Revoked
                    && record.is_drawn())
            {
                return Err(ResourceError::Conservation(
                    "resource access grant cap accounting does not reconcile".to_owned(),
                ));
            }
        }
        Ok(())
    }

    pub fn validate_conservation(&self) -> Result<(), ResourceError> {
        // Accounts that are not attached count through the live ledger.
        let balances = self.total_account_balance()?;
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

/// Whether `next` may follow `current`, the link of an in-transit transfer
/// whose recorded links are `history`, as an intermediate link: the same
/// execution, a leg not lower than the current one (a link without a leg is
/// lower than one with a leg), and either a new leg or a handoff `history`
/// does not record. Leg order beyond that is the transport's.
fn is_intermediate_link(
    current: &TransportExecutionLink,
    history: &[crate::ResourceTransferLinkV1],
    next: &TransportExecutionLink,
) -> bool {
    let lower_leg = match (current.leg_execution, next.leg_execution) {
        (Some(current), Some(next)) => next < current,
        (Some(_), None) => true,
        (None, _) => false,
    };
    let new_leg = next.leg_execution.is_some() && next.leg_execution != current.leg_execution;
    let new_handoff = next.handoff.is_some_and(|handoff| {
        !history
            .iter()
            .any(|entry| entry.transport.handoff == Some(handoff))
    });
    next.execution == current.execution && !lower_leg && (new_leg || new_handoff)
}

/// Admits an intermediate advance of an in-transit transfer to `link`, which
/// must be an intermediate link ([`is_intermediate_link`]) and leave room in
/// the history for an arrival and a return. Each refusal is the advance's
/// recorded rejection.
fn check_intermediate_link(
    transfer: &ResourceTransfer,
    link: &TransportExecutionLink,
) -> Result<(), ResourceError> {
    let intermediate = transfer
        .transport
        .as_ref()
        .is_some_and(|current| is_intermediate_link(current, &transfer.link_history, link));
    if !intermediate {
        return Err(ResourceError::InvalidLifecycle(
            "an intermediate resource transfer advance names the execution of the current link with a new leg, not lower than the current one, or a handoff not recorded before"
                .to_owned(),
        ));
    }
    let in_transit = transfer
        .link_history
        .iter()
        .filter(|entry| entry.progress == TransferProgressV1::InTransit)
        .count();
    if in_transit >= crate::MAX_RESOURCE_TRANSFER_IN_TRANSIT_LINKS {
        return Err(ResourceError::LimitExceeded(format!(
            "a resource transfer records at most {} in-transit links; its arrival and return stay admitted",
            crate::MAX_RESOURCE_TRANSFER_IN_TRANSIT_LINKS
        )));
    }
    Ok(())
}

/// Whether a transfer's link history is one its advances could have
/// recorded: at most [`crate::MAX_RESOURCE_TRANSFER_LINKS`] entries and
/// [`crate::MAX_RESOURCE_TRANSFER_IN_TRANSIT_LINKS`] in-transit ones, each a
/// transition `advance_transfer` admits from the state the entries before it
/// left, intermediate links included, with `transport` its last link. An
/// open transfer is in the state its history leaves, or returning after a
/// cancellation, which records no link, and keeps no `exact_evidence` beside
/// its history. An accepted transfer was accepted at its destination after
/// an arrival, or locally without a link.
pub(crate) fn transfer_link_history_is_valid(transfer: &ResourceTransfer) -> bool {
    let history = &transfer.link_history;
    if history.len() > crate::MAX_RESOURCE_TRANSFER_LINKS
        || transfer.transport.as_ref() != history.last().map(|entry| &entry.transport)
    {
        return false;
    }
    let mut state = ResourceTransferState::PendingDispatch;
    let mut in_transit = 0_usize;
    for (index, entry) in history.iter().enumerate() {
        state = match (state, entry.progress) {
            (ResourceTransferState::PendingDispatch, TransferProgressV1::InTransit) => {
                ResourceTransferState::InTransit
            }
            (ResourceTransferState::InTransit, TransferProgressV1::InTransit)
                if is_intermediate_link(
                    &history[index - 1].transport,
                    &history[..index],
                    &entry.transport,
                ) =>
            {
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
            _ => return false,
        };
        if entry.progress == TransferProgressV1::InTransit {
            in_transit += 1;
        }
    }
    let ends_in_place = if transfer.escrow > 0 {
        transfer.exact_evidence.is_empty()
            && (transfer.state == state
                || (transfer.state == ResourceTransferState::ReturnPending
                    && state != ResourceTransferState::ReturnPending))
    } else {
        // A return, a loss, and an outflow end a transfer in any open state
        // (a return after a cancellation, which records no link).
        transfer.state != ResourceTransferState::Accepted
            || history.is_empty()
            || state == ResourceTransferState::ArrivalPending
    };
    in_transit <= crate::MAX_RESOURCE_TRANSFER_IN_TRANSIT_LINKS && ends_in_place
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

/// Whether `fulfillment` is the one `consumption` recorded: of its demand,
/// under its operation key, and fulfilled by its leg.
pub(crate) fn consumption_names_its_fulfillment(
    consumption: &ResourceConsumption,
    fulfillment: &crate::ResourceFulfillment,
) -> bool {
    fulfillment.id == consumption.fulfillment
        && fulfillment.demand == consumption.demand
        && fulfillment.operation_key == consumption.operation_key
        && fulfillment
            .allocation_legs
            .contains(&consumption.allocation_leg)
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
        // A batch outcome's remainder is the quantity its refused items did
        // not consume: a batch refused whole, every item's.
        ResourceOperationRequestV1::ConsumeBatch(value) => {
            value.items.iter().fold(0_u64, |total, item| {
                total.saturating_add(item.allocation.quantity)
            })
        }
        ResourceOperationRequestV1::BeginTransfer(value) => value.allocation.quantity,
        ResourceOperationRequestV1::Credit(value) => value.quantity,
        ResourceOperationRequestV1::ExternalOutflow(value) => value.quantity,
        ResourceOperationRequestV1::RecordLoss(value) => value.quantity,
        ResourceOperationRequestV1::RecordTransferLoss(value) => value.quantity,
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
        | ResourceOperationRequestV1::Completion(_)
        | ResourceOperationRequestV1::BeginExchange(_)
        | ResourceOperationRequestV1::RevokeAccessGrant(_)
        | ResourceOperationRequestV1::RescopeAccount(_) => 0,
        ResourceOperationRequestV1::IssueAccessGrant(value) => value.grant.cap_quantity,
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
            .settle_prepare_with_external_revalidation(
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
            .and_then(super::OperationSettlement::into_outcome)
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
