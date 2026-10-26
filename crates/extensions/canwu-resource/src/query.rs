use crate::{
    AllocationLegStatus, DemandStatus, ResourceAccountId, ResourceAllocationLeg,
    ResourceAllocationLegId, ResourceAllocationLegVersionV1, ResourceConsumption,
    ResourceConsumptionId, ResourceConsumptionVersionV1, ResourceDemandId, ResourceError,
    ResourceFulfillmentId, ResourceFulfillmentVersionV1, ResourceOperationKey,
    ResourceOperationOutcome, ResourceOperationOutcomeId, ResourceOperationOutcomeVersionV1,
    ResourceReportGrantId, ResourceReportId, ResourceRevision, ResourceScopeId, ResourceState,
    ResourceTerminalRecordKeyV1, canonical_digest,
};
use canwu_api::{
    Canwu, CanwuError, DomainRecord, DomainRecordVersionRef, ErrorCode, EvidenceRef,
    KnowledgeHolderRef, PluginArchiveObjectProvider, ReplayJournal, SimTime, SimulationPlugin,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

struct ResourceRuntimeArchiveProvider {
    store: Arc<dyn crate::ResourceArchiveStore + Send + Sync>,
}

impl PluginArchiveObjectProvider for ResourceRuntimeArchiveProvider {
    fn load_plugin_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, CanwuError> {
        self.store
            .load_resource_archive_object(namespace, object_id)
            .map_err(resource_error)
    }
}

fn runtime_archive_provider(
    store: Arc<dyn crate::ResourceArchiveStore + Send + Sync>,
) -> Arc<dyn PluginArchiveObjectProvider + Send + Sync> {
    Arc::new(ResourceRuntimeArchiveProvider { store })
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceStockObservationV1 {
    pub account: ResourceAccountId,
    pub scope: ResourceScopeId,
    pub known_minimum: u64,
    pub known_maximum: u64,
    pub reserved: u64,
    pub protected: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceDemandObservationV1 {
    pub demand: ResourceDemandId,
    pub requested: u64,
    pub fulfilled: u64,
    pub remainder: u64,
    pub status: DemandStatus,
    pub rejection_reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceAllocationObservationV1 {
    pub allocation: ResourceAllocationLegId,
    pub exact: ResourceAllocationLegVersionV1,
    pub status: AllocationLegStatus,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceFulfillmentObservationV1 {
    pub fulfillment: ResourceFulfillmentId,
    pub consumed: u64,
    pub remainder: u64,
    pub rejection_reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceTransferObservationV1 {
    pub transfer: crate::ResourceTransferId,
    pub state: crate::ResourceTransferState,
    pub quantity: u64,
    pub escrow: u64,
    pub accepted: u64,
    pub lost: u64,
    pub returned: u64,
    pub external_outflow: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceConsumptionObservationV1 {
    pub consumption: ResourceConsumptionId,
    pub exact: ResourceConsumptionVersionV1,
    pub demand: ResourceDemandId,
    pub status: crate::ConsumptionStatus,
}

/// Holder-visible loss evidence: an account-level loss (`account`) or lost
/// transfer escrow (`transfer`), with its quantity and cause.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceLossObservationV1 {
    pub loss: crate::ResourceLossId,
    pub account: Option<ResourceAccountId>,
    pub transfer: Option<crate::ResourceTransferId>,
    pub quantity: u64,
    pub cause: EvidenceRef,
}

/// Persisted provider-owned holder observation. Reports are materialized from
/// this cut and never reconstructed from current authoritative balances.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceObservationHeadV1 {
    pub id: crate::ResourceObservationHeadId,
    pub revision: ResourceRevision,
    pub provider_plugin: String,
    pub provider_version: String,
    pub provider_semantic_hash: String,
    pub provider_source: DomainRecordVersionRef,
    pub holder: KnowledgeHolderRef,
    pub grant: ResourceReportGrantId,
    pub provider_state_revision: ResourceRevision,
    pub observed_at: SimTime,
    pub confidence_per_mille: u16,
    pub stock: Vec<ResourceStockObservationV1>,
    pub demands: Vec<ResourceDemandObservationV1>,
    pub allocations: Vec<ResourceAllocationObservationV1>,
    pub fulfillments: Vec<ResourceFulfillmentObservationV1>,
    #[serde(default)]
    pub transfers: Vec<ResourceTransferObservationV1>,
    #[serde(default)]
    pub consumptions: Vec<ResourceConsumptionObservationV1>,
    /// Loss details, gated like transfer and consumption details.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub losses: Vec<ResourceLossObservationV1>,
    pub source_versions: Vec<DomainRecordVersionRef>,
    pub semantic_digest: String,
}

impl ResourceObservationHeadV1 {
    pub fn seal(mut self) -> Result<Self, ResourceError> {
        self.source_versions.sort();
        self.source_versions.dedup();
        self.stock
            .sort_by(|left, right| left.account.cmp(&right.account));
        self.demands
            .sort_by(|left, right| left.demand.cmp(&right.demand));
        self.allocations
            .sort_by(|left, right| left.allocation.cmp(&right.allocation));
        self.fulfillments
            .sort_by(|left, right| left.fulfillment.cmp(&right.fulfillment));
        self.transfers
            .sort_by(|left, right| left.transfer.cmp(&right.transfer));
        self.consumptions
            .sort_by(|left, right| left.consumption.cmp(&right.consumption));
        self.losses
            .sort_by(|left, right| left.loss.cmp(&right.loss));
        self.semantic_digest.clear();
        self.semantic_digest = canonical_digest("canwu.resource.observation-head.v1", &self)?;
        Ok(self)
    }

    pub fn validate(&self) -> Result<(), ResourceError> {
        let mut sealed = self.clone();
        sealed.semantic_digest.clear();
        let mut canonical = sealed.clone();
        canonical.source_versions.sort();
        canonical.source_versions.dedup();
        canonical
            .stock
            .sort_by(|left, right| left.account.cmp(&right.account));
        canonical
            .demands
            .sort_by(|left, right| left.demand.cmp(&right.demand));
        canonical
            .allocations
            .sort_by(|left, right| left.allocation.cmp(&right.allocation));
        canonical
            .fulfillments
            .sort_by(|left, right| left.fulfillment.cmp(&right.fulfillment));
        canonical
            .transfers
            .sort_by(|left, right| left.transfer.cmp(&right.transfer));
        canonical
            .consumptions
            .sort_by(|left, right| left.consumption.cmp(&right.consumption));
        canonical
            .losses
            .sort_by(|left, right| left.loss.cmp(&right.loss));
        if self.confidence_per_mille > 1_000
            || canonical != sealed
            || self.semantic_digest
                != canonical_digest("canwu.resource.observation-head.v1", &sealed)?
        {
            return Err(ResourceError::InvalidDefinition(
                "resource observation head is non-canonical or forged".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Detached holder-bound resource report. It is not authoritative state and
/// cannot be used to mint or debit stock.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceReportDtoV1 {
    pub id: ResourceReportId,
    pub holder: KnowledgeHolderRef,
    pub grant: ResourceReportGrantId,
    pub provider_state_revision: ResourceRevision,
    pub observed_at: SimTime,
    pub materialized_at: SimTime,
    pub confidence_per_mille: u16,
    pub stale: bool,
    pub stock: Vec<ResourceStockObservationV1>,
    pub demands: Vec<ResourceDemandObservationV1>,
    pub allocations: Vec<ResourceAllocationObservationV1>,
    pub fulfillments: Vec<ResourceFulfillmentObservationV1>,
    pub transfers: Vec<ResourceTransferObservationV1>,
    pub consumptions: Vec<ResourceConsumptionObservationV1>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub losses: Vec<ResourceLossObservationV1>,
    pub sorting_evidence: Vec<String>,
    pub digest: String,
}

/// Bounded typed witness used by G5 source adapters. It is detached and
/// holder-bound, not a knowledge-ledger record.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceObservationWitnessV1 {
    pub provider_plugin: String,
    pub provider_version: String,
    pub provider_semantic_hash: String,
    pub provider_state_revision: ResourceRevision,
    pub adapter_provider_state_revision: ResourceRevision,
    pub head_revision: ResourceRevision,
    pub holder: KnowledgeHolderRef,
    pub scope: ResourceScopeId,
    pub observed_at: SimTime,
    pub materialized_at: SimTime,
    pub confidence_per_mille: u16,
    pub stock: Vec<ResourceStockObservationV1>,
    pub demands: Vec<ResourceDemandObservationV1>,
    pub allocations: Vec<ResourceAllocationObservationV1>,
    pub fulfillments: Vec<ResourceFulfillmentObservationV1>,
    pub transfers: Vec<ResourceTransferObservationV1>,
    pub consumptions: Vec<ResourceConsumptionObservationV1>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub losses: Vec<ResourceLossObservationV1>,
    pub source_versions: Vec<DomainRecordVersionRef>,
    pub adapter_revision: crate::ResourceObservationAdapterRevisionId,
    pub digest: String,
}

/// The whole resource root of a run: its live root record and the state
/// with every bucketed record attached ([`crate::ResourceRootReader::attach_all`]).
/// Reading it costs time in proportion to the hot terminal backlog and the
/// number of accounts; a host that reads a few named records reads them with
/// [`crate::ResourceRootReader::load_run`], and one that reads the whole root
/// again after a step keeps the reader ([`crate::ResourceRootReader::keep`])
/// and re-attaches it. Takes either runtime handle, [`canwu_api::Canwu`] or
/// [`canwu_api::CompactedCanwu`].
pub fn resource_state<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &H,
) -> Result<Option<(DomainRecord, ResourceState)>, CanwuError> {
    crate::whole_resource_root(canwu)
}

/// Answers `read` from the run's resource root with only what `attach`
/// names attached ([`crate::ResourceRootReader::load_run`]), or `None`
/// without a resource root. `read` must look only at the live root and the
/// records `attach` names, so that it answers as on the whole root. With the
/// `verify-targeted-reads` feature, which Canwu's own test runs enable and
/// applications leave off, it reads the whole root as well and panics on a
/// different answer.
fn targeted_read<H, T>(
    canwu: &H,
    attach: impl FnOnce(&mut crate::ResourceRootReader<'_, '_>) -> Result<(), CanwuError>,
    read: impl Fn(&ResourceState) -> T,
) -> Result<Option<T>, CanwuError>
where
    H: canwu_api::CanwuRun + ?Sized,
    T: PartialEq + std::fmt::Debug,
{
    let Some(mut reader) = crate::ResourceRootReader::load_run(canwu)? else {
        return Ok(None);
    };
    attach(&mut reader)?;
    let answer = read(reader.state());
    #[cfg(feature = "verify-targeted-reads")]
    if let Ok(Some((_, whole))) = resource_state(canwu) {
        assert_eq!(
            answer,
            read(&whole),
            "a resource host read answered differently on the named records than on the whole root"
        );
    }
    Ok(Some(answer))
}

/// The current allocation leg with this ID. It reads the live root and the
/// leg's bucket, not the whole root. Takes either runtime handle,
/// [`canwu_api::Canwu`] or [`canwu_api::CompactedCanwu`].
#[must_use]
pub fn resource_allocation_leg<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &H,
    id: &ResourceAllocationLegId,
) -> Option<ResourceAllocationLegVersionV1> {
    targeted_read(
        canwu,
        |reader| reader.attach([ResourceTerminalRecordKeyV1::AllocationLeg(id.clone())]),
        |state| {
            state
                .allocation_legs
                .get(id)
                .map(ResourceAllocationLegVersionV1::from)
        },
    )
    .ok()
    .flatten()
    .flatten()
}

/// An allocation leg at an exact state revision. It reads the live root and
/// the leg's bucket. Takes either runtime handle, [`canwu_api::Canwu`] or
/// [`canwu_api::CompactedCanwu`].
pub fn exact_resource_allocation_leg<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &H,
    exact: &ResourceAllocationLegVersionV1,
) -> Result<ResourceAllocationLeg, CanwuError> {
    targeted_read(
        canwu,
        |reader| reader.attach([ResourceTerminalRecordKeyV1::AllocationLeg(exact.id.clone())]),
        |state| {
            let leg = state
                .allocation_legs
                .get(&exact.id)
                .ok_or_else(|| unavailable("resource allocation leg is unavailable"))?;
            if leg.revision != exact.revision
                || leg.account != exact.account
                || leg.account_revision != exact.account_revision
                || leg.resource_revision != exact.resource_revision
                || leg.unit_revision != exact.unit_revision
                || leg.quantity != exact.quantity
                || leg.semantic_digest != exact.semantic_digest
            {
                return Err(invalid("resource allocation exact binding differs"));
            }
            Ok(leg.clone())
        },
    )?
    .ok_or_else(|| unavailable("resource runtime is absent"))?
}

/// The current consumption with this ID. It reads the live root and the
/// consumption's bucket. Takes either runtime handle, [`canwu_api::Canwu`]
/// or [`canwu_api::CompactedCanwu`].
#[must_use]
pub fn resource_consumption<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &H,
    id: &ResourceConsumptionId,
) -> Option<ResourceConsumptionVersionV1> {
    targeted_read(
        canwu,
        |reader| reader.attach([ResourceTerminalRecordKeyV1::Consumption(id.clone())]),
        |state| {
            state
                .consumptions
                .get(id)
                .map(ResourceConsumptionVersionV1::from)
        },
    )
    .ok()
    .flatten()
    .flatten()
}

/// A consumption at an exact state revision. It reads the live root and the
/// consumption's bucket. Takes either runtime handle, [`canwu_api::Canwu`]
/// or [`canwu_api::CompactedCanwu`].
pub fn exact_resource_consumption<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &H,
    exact: &ResourceConsumptionVersionV1,
) -> Result<ResourceConsumption, CanwuError> {
    targeted_read(
        canwu,
        |reader| reader.attach([ResourceTerminalRecordKeyV1::Consumption(exact.id.clone())]),
        |state| {
            let consumption = state
                .consumptions
                .get(&exact.id)
                .ok_or_else(|| unavailable("resource consumption is unavailable"))?;
            if consumption.revision != exact.revision
                || consumption.account != exact.account
                || consumption.allocation_leg != exact.allocation_leg
                || consumption.quantity != exact.quantity
                || consumption.consumer_evidence != exact.consumer_evidence
                || consumption.semantic_digest != exact.semantic_digest
            {
                return Err(invalid("resource consumption exact binding differs"));
            }
            Ok(consumption.clone())
        },
    )?
    .ok_or_else(|| unavailable("resource runtime is absent"))?
}

/// The hot outcome of an operation key. It reads the live root and the
/// outcome's bucket. Takes either runtime handle, [`canwu_api::Canwu`] or
/// [`canwu_api::CompactedCanwu`].
#[must_use]
pub fn resource_operation_outcome<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &H,
    operation_key: &ResourceOperationKey,
) -> Option<ResourceOperationOutcomeVersionV1> {
    targeted_read(
        canwu,
        |reader| reader.attach([ResourceTerminalRecordKeyV1::Outcome(operation_key.clone())]),
        |state| state.outcomes.get(operation_key).map(Into::into),
    )
    .ok()
    .flatten()
    .flatten()
}

/// The hot outcome with this operation ID. Takes either runtime handle, [`canwu_api::Canwu`] or [`canwu_api::CompactedCanwu`].
#[must_use]
pub fn resource_operation_outcome_by_id<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &H,
    id: &ResourceOperationOutcomeId,
) -> Option<ResourceOperationOutcomeVersionV1> {
    resource_state(canwu).ok().flatten().and_then(|(_, state)| {
        state
            .outcomes
            .values()
            .find(|outcome| &outcome.id == id)
            .map(Into::into)
    })
}

/// The completion certificate of a lease. It reads the live root, which
/// holds every unfinished lease. Takes either runtime handle,
/// [`canwu_api::Canwu`] or [`canwu_api::CompactedCanwu`].
///
/// A lease leaves the root when it finishes (completed, released, expired,
/// or aborted, with any report it reserved published), so this finds only an
/// unfinished lease: a finished one's receipts and outcomes are its record,
/// and its acquisition identity starts no other lease.
#[must_use]
pub fn resource_completion_certificate<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &H,
    acquisition: &crate::CompletionLeaseAcquisitionId,
) -> Option<crate::CompletionLeaseActivationCertificateV1> {
    targeted_read(
        canwu,
        |_| Ok(()),
        |state| state.completion_leases.certificate(acquisition).cloned(),
    )
    .ok()
    .flatten()
    .flatten()
}

/// A completion capacity grant. It reads the live root. Takes either runtime
/// handle, [`canwu_api::Canwu`] or [`canwu_api::CompactedCanwu`].
///
/// A lease leaves the root when it finishes (completed, released, expired,
/// or aborted, with any report it reserved published), so this finds only an
/// unfinished lease: a finished one's receipts and outcomes are its record,
/// and its acquisition identity starts no other lease.
#[must_use]
pub fn resource_completion_grant<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &H,
    grant: &crate::CompletionCapacityGrantId,
) -> Option<crate::CompletionCapacityGrantV1> {
    targeted_read(
        canwu,
        |_| Ok(()),
        |state| state.completion_leases.grants.get(grant).cloned(),
    )
    .ok()
    .flatten()
    .flatten()
}

/// The status of a completion lease acquisition. It reads the live root.
/// Takes either runtime handle, [`canwu_api::Canwu`] or
/// [`canwu_api::CompactedCanwu`].
///
/// A lease leaves the root when it finishes (completed, released, expired,
/// or aborted, with any report it reserved published), so this finds only an
/// unfinished lease: a finished one's receipts and outcomes are its record,
/// and its acquisition identity starts no other lease.
pub fn resource_completion_status<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &H,
    holder: &KnowledgeHolderRef,
    acquisition: &crate::CompletionLeaseAcquisitionId,
) -> Result<crate::CompletionLeaseStatusDtoV1, CanwuError> {
    targeted_read(
        canwu,
        |_| Ok(()),
        |state| {
            state
                .completion_leases
                .status_for(holder, acquisition)
                .map_err(resource_error)
        },
    )?
    .ok_or_else(|| unavailable("resource runtime is absent"))?
}

/// Holder-bound view of one access grant and its cap accounting, readable only
/// by the grant's grantor custodian or grantee.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceAccessGrantStatusDtoV1 {
    pub grant: crate::ResourceAccessGrantV1,
    pub revision: ResourceRevision,
    pub status: crate::ResourceAccessGrantStatusV1,
    pub reserved_quantity: u64,
    pub debited_quantity: u64,
    pub remaining_quantity: u64,
}

/// The status of an access grant. It reads the live root and the grant's
/// live bucket. Takes either runtime handle, [`canwu_api::Canwu`] or
/// [`canwu_api::CompactedCanwu`].
pub fn resource_access_grant_status<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &H,
    holder: &KnowledgeHolderRef,
    grant: &crate::ResourceAccessGrantId,
) -> Result<ResourceAccessGrantStatusDtoV1, CanwuError> {
    targeted_read(
        canwu,
        |reader| reader.attach_live([crate::ResourceLiveRecordKeyV1::AccessGrant(grant.clone())]),
        |state| {
            let record = state
                .access_grants
                .get(grant)
                .ok_or_else(|| unavailable("resource access grant is unavailable"))?;
            if &record.grant.grantor_custodian != holder && &record.grant.grantee != holder {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "resource access grant status is readable only by its grantor or grantee",
                ));
            }
            Ok(ResourceAccessGrantStatusDtoV1 {
                grant: record.grant.clone(),
                revision: record.revision,
                status: record.status,
                reserved_quantity: record.reserved_quantity,
                debited_quantity: record.debited_quantity,
                remaining_quantity: record.remaining(),
            })
        },
    )?
    .ok_or_else(|| unavailable("resource runtime is absent"))?
}

/// A completion certificate at an exact state revision. Takes either runtime handle, [`canwu_api::Canwu`] or [`canwu_api::CompactedCanwu`].
pub fn exact_resource_completion_certificate<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &H,
    exact: &crate::CompletionLeaseActivationCertificateV1,
) -> Result<crate::CompletionLeaseActivationCertificateV1, CanwuError> {
    let (_, state) =
        resource_state(canwu)?.ok_or_else(|| unavailable("resource runtime is absent"))?;
    state
        .completion_leases
        .validate(&state.run_budget)
        .map_err(resource_error)?;
    let persisted = state
        .completion_leases
        .certificate(&exact.acquisition)
        .ok_or_else(|| unavailable("resource completion certificate is unavailable"))?;
    if persisted != exact {
        return Err(invalid(
            "resource completion certificate differs from the persisted exact version",
        ));
    }
    Ok(persisted.clone())
}

/// An operation outcome at an exact state revision. It reads the live root
/// and the outcome's bucket. Takes either runtime handle,
/// [`canwu_api::Canwu`] or [`canwu_api::CompactedCanwu`].
pub fn exact_resource_operation_outcome<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &H,
    exact: &ResourceOperationOutcomeVersionV1,
) -> Result<ResourceOperationOutcome, CanwuError> {
    targeted_read(
        canwu,
        |reader| {
            reader.attach([ResourceTerminalRecordKeyV1::Outcome(
                exact.operation_key.clone(),
            )])
        },
        |state| {
            let outcome = state
                .outcomes
                .get(&exact.operation_key)
                .ok_or_else(|| unavailable("resource operation outcome is unavailable"))?;
            if outcome.id != exact.id
                || outcome.revision != exact.revision
                || outcome.status != exact.status
                || outcome.quantity != exact.quantity
                || outcome.remainder != exact.remainder
                || outcome.result_ref != exact.result_ref
                || outcome.semantic_digest != exact.semantic_digest
            {
                return Err(invalid("resource operation exact outcome binding differs"));
            }
            Ok(outcome.clone())
        },
    )?
    .ok_or_else(|| unavailable("resource runtime is absent"))?
}

/// The latest fulfillment of a demand. Takes either runtime handle, [`canwu_api::Canwu`] or [`canwu_api::CompactedCanwu`].
#[must_use]
pub fn latest_resource_fulfillment<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &H,
    demand: &ResourceDemandId,
) -> Option<ResourceFulfillmentVersionV1> {
    resource_state(canwu).ok().flatten().and_then(|(_, state)| {
        state
            .fulfillments
            .values()
            .rfind(|value| &value.demand == demand)
            .map(Into::into)
    })
}

/// A fulfillment at an exact state revision. It reads the live root and the
/// fulfillment's bucket. Takes either runtime handle, [`canwu_api::Canwu`]
/// or [`canwu_api::CompactedCanwu`].
pub fn exact_resource_fulfillment<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &H,
    exact: &ResourceFulfillmentVersionV1,
) -> Result<crate::ResourceFulfillment, CanwuError> {
    targeted_read(
        canwu,
        |reader| reader.attach([ResourceTerminalRecordKeyV1::Fulfillment(exact.id.clone())]),
        |state| {
            let fulfillment = state
                .fulfillments
                .get(&exact.id)
                .ok_or_else(|| unavailable("resource fulfillment is unavailable"))?;
            if fulfillment.revision != exact.revision
                || fulfillment.demand != exact.demand
                || fulfillment.consumed_quantity != exact.consumed_quantity
                || fulfillment.remainder != exact.remainder
                || fulfillment.status != exact.status
                || fulfillment.operation_key != exact.operation_key
                || fulfillment.semantic_digest != exact.semantic_digest
            {
                return Err(invalid("resource fulfillment exact binding differs"));
            }
            Ok(fulfillment.clone())
        },
    )?
    .ok_or_else(|| unavailable("resource runtime is absent"))?
}

/// The holder-relative resource report. Takes either runtime handle, [`canwu_api::Canwu`] or [`canwu_api::CompactedCanwu`].
pub fn resource_report<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &H,
    holder: &KnowledgeHolderRef,
    grant_id: &ResourceReportGrantId,
    materialized_at: SimTime,
) -> Result<ResourceReportDtoV1, CanwuError> {
    let (_, state) =
        resource_state(canwu)?.ok_or_else(|| unavailable("resource runtime is absent"))?;
    materialize_resource_report(&state, holder, grant_id, canwu.time(), materialized_at)
        .map_err(resource_error)
}

/// Materializes a detached G5 witness from the persisted provider-owned
/// observation head. This never reads current balances or backdates truth.
/// Takes either runtime handle, [`canwu_api::Canwu`] or [`canwu_api::CompactedCanwu`].
///
/// On a [`canwu_api::CompactedCanwu`], whether a cited superseded version that
/// no dependency declares still exists depends on the receipts earlier seals
/// kept; see [`canwu_api::CanwuRun::domain_record_version_evidence_exists`].
pub fn resource_observation_witness<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &H,
    holder: &KnowledgeHolderRef,
    grant_id: &ResourceReportGrantId,
    adapter_revision: crate::ResourceObservationAdapterRevisionId,
    materialized_at: SimTime,
) -> Result<ResourceObservationWitnessV1, CanwuError> {
    let (_, state) =
        resource_state(canwu)?.ok_or_else(|| unavailable("resource runtime is absent"))?;
    let grant = state
        .report_grants
        .get(grant_id)
        .ok_or_else(|| unavailable("resource observation grant is unavailable"))?;
    if &grant.holder != holder {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "resource observation grant belongs to another holder",
        ));
    }
    let head_id = state
        .observation_head_by_grant
        .get(grant_id)
        .ok_or_else(|| unavailable("resource observation head is unavailable"))?;
    let head = state
        .observation_heads
        .get(head_id)
        .ok_or_else(|| invalid("resource observation head index is broken"))?;
    head.validate().map_err(resource_error)?;
    if head.holder != *holder
        || head.grant != *grant_id
        || head.provider_plugin.is_empty()
        || head.provider_version.is_empty()
        || head.provider_semantic_hash.len() != 64
        || !head.source_versions.contains(&head.provider_source)
    {
        return Err(invalid(
            "resource observation provider, holder, grant, or source binding differs",
        ));
    }
    for source in &head.source_versions {
        if !canwu.domain_record_version_evidence_exists(source) {
            return Err(CanwuError::new(
                ErrorCode::EvidenceContentUnavailable,
                "resource observation exact source body is unavailable",
            ));
        }
    }
    let mut witness = ResourceObservationWitnessV1 {
        provider_plugin: head.provider_plugin.clone(),
        provider_version: head.provider_version.clone(),
        provider_semantic_hash: head.provider_semantic_hash.clone(),
        provider_state_revision: head.provider_state_revision,
        adapter_provider_state_revision: state.state_revision,
        head_revision: head.revision,
        holder: holder.clone(),
        scope: grant.scope.clone(),
        observed_at: head.observed_at,
        materialized_at,
        confidence_per_mille: head.confidence_per_mille,
        stock: head.stock.clone(),
        demands: head.demands.clone(),
        allocations: head.allocations.clone(),
        fulfillments: head.fulfillments.clone(),
        transfers: head.transfers.clone(),
        consumptions: head.consumptions.clone(),
        losses: head.losses.clone(),
        source_versions: head.source_versions.clone(),
        adapter_revision,
        digest: String::new(),
    };
    witness.digest = canonical_digest("canwu.resource.observation-witness.v1", &witness)
        .map_err(resource_error)?;
    Ok(witness)
}

pub fn materialize_resource_report(
    state: &ResourceState,
    holder: &KnowledgeHolderRef,
    grant_id: &ResourceReportGrantId,
    _observed_at: SimTime,
    materialized_at: SimTime,
) -> Result<ResourceReportDtoV1, ResourceError> {
    let grant = state.report_grants.get(grant_id).ok_or_else(|| {
        ResourceError::Authority("resource report grant is unavailable".to_owned())
    })?;
    if &grant.holder != holder {
        return Err(ResourceError::Authority(
            "resource report grant is bound to another holder".to_owned(),
        ));
    }
    if grant.accounts.len() + grant.demands.len() > crate::ResourceLimitsV1::MAX_QUERY_PAGE {
        return Err(ResourceError::LimitExceeded(
            "resource report grant exceeds the bounded query budget".to_owned(),
        ));
    }
    let head_id = state
        .observation_head_by_grant
        .get(grant_id)
        .ok_or_else(|| {
            ResourceError::NotFound("resource report has no persisted observation head".to_owned())
        })?;
    let head = state.observation_heads.get(head_id).ok_or_else(|| {
        ResourceError::InvalidDefinition("resource observation head index is broken".to_owned())
    })?;
    head.validate()?;
    if head.holder != *holder || head.grant != *grant_id {
        return Err(ResourceError::Authority(
            "resource observation head is bound to another holder or grant".to_owned(),
        ));
    }
    if head.stock.iter().any(|value| {
        !grant.accounts.contains(&value.account)
            || state
                .accounts
                .get(&value.account)
                .and_then(|account| state.definitions.get(&account.resource_revision))
                .is_none_or(|definition| definition.scope != value.scope)
    }) || head
        .demands
        .iter()
        .any(|value| !grant.demands.contains(&value.demand))
    {
        return Err(ResourceError::Authority(
            "resource observation head exceeds its holder grant".to_owned(),
        ));
    }
    state.validate_observation_head_authority(head, grant)?;
    let sorting_evidence = head
        .stock
        .iter()
        .map(|value| format!("account:{}", value.account.as_str()))
        .chain(
            head.demands
                .iter()
                .map(|value| format!("demand:{}", value.demand.as_str())),
        )
        .chain(
            head.transfers
                .iter()
                .map(|value| format!("transfer:{}", value.transfer.as_str())),
        )
        .chain(
            head.consumptions
                .iter()
                .map(|value| format!("consumption:{}", value.consumption.as_str())),
        )
        .chain(
            head.losses
                .iter()
                .map(|value| format!("loss:{}", value.loss.as_str())),
        )
        .collect();
    let id = ResourceReportId::new(format!(
        "resource:report:{}:{}",
        grant_id.as_str().replace(':', "-"),
        state.state_revision.get()
    ))?;
    let mut report = ResourceReportDtoV1 {
        id,
        holder: holder.clone(),
        grant: grant_id.clone(),
        provider_state_revision: head.provider_state_revision,
        observed_at: head.observed_at,
        materialized_at,
        confidence_per_mille: head.confidence_per_mille,
        stale: materialized_at > head.observed_at,
        stock: head.stock.clone(),
        demands: head.demands.clone(),
        allocations: head.allocations.clone(),
        fulfillments: head.fulfillments.clone(),
        transfers: head.transfers.clone(),
        consumptions: head.consumptions.clone(),
        losses: head.losses.clone(),
        sorting_evidence,
        digest: String::new(),
    };
    report.digest = canonical_digest("canwu.resource.report-dto.v1", &report)?;
    Ok(report)
}

pub fn validate_resource_report(report: &ResourceReportDtoV1) -> Result<(), ResourceError> {
    let mut detached = report.clone();
    detached.digest.clear();
    if report.confidence_per_mille > 1_000
        || report.digest != canonical_digest("canwu.resource.report-dto.v1", &detached)?
    {
        return Err(ResourceError::InvalidDefinition(
            "resource report metadata or digest is invalid".to_owned(),
        ));
    }
    Ok(())
}

/// Checks the resource runtime record and the exact evidence its
/// outcomes cite. Takes either runtime handle, [`canwu_api::Canwu`] or [`canwu_api::CompactedCanwu`].
///
/// On a [`canwu_api::CompactedCanwu`], whether a cited superseded version that
/// no dependency declares still exists depends on the receipts earlier seals
/// kept; see [`canwu_api::CanwuRun::domain_record_version_evidence_exists`].
pub fn validate_resource_runtime<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &H,
) -> Result<(), CanwuError> {
    let Some((record, state)) = resource_state(canwu)? else {
        return Ok(());
    };
    state.validate().map_err(resource_error)?;
    if record.version != state.state_revision.get() {
        return Err(invalid(
            "resource runtime domain-record version differs from its state revision",
        ));
    }
    if state.encoded_root_bytes().map_err(resource_error)? > state.limits.max_state_bytes {
        return Err(invalid("resource authoritative state exceeds its byte cap"));
    }
    for outcome in state.outcomes.values() {
        for evidence in &outcome.exact_evidence {
            if !canwu.domain_record_version_evidence_exists(evidence) {
                return Err(CanwuError::new(
                    ErrorCode::EvidenceUnavailable,
                    "resource operation exact evidence is unavailable",
                ));
            }
        }
    }
    Ok(())
}

/// Checks the resource runtime and authenticates its archive against
/// `store`. Takes either runtime handle, [`canwu_api::Canwu`] or [`canwu_api::CompactedCanwu`].
pub fn validate_resource_runtime_with_archive_store<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &H,
    store: &dyn crate::ResourceArchiveStore,
) -> Result<(), CanwuError> {
    validate_resource_runtime(canwu)?;
    let Some((_, state)) = resource_state(canwu)? else {
        return Ok(());
    };
    crate::validate_resource_archive_store(&state, store).map_err(resource_error)
}

pub fn from_resource_snapshot_json(
    json: &str,
    plugins: &[&dyn SimulationPlugin],
) -> Result<Canwu, CanwuError> {
    let canwu = Canwu::from_snapshot_json_with_plugins(json, plugins)?;
    validate_resource_runtime(&canwu)?;
    reject_archive_backed_restore_without_store(&canwu)?;
    Ok(canwu)
}

/// Restores a run from snapshot JSON, checks its resource state against
/// `store`, and attaches `store` as the run's plugin archive provider.
///
/// The run and its forks share `store`, and a fork may move to another
/// thread, so the store must be `Send + Sync`.
pub fn from_resource_snapshot_json_with_archive_store(
    json: &str,
    plugins: &[&dyn SimulationPlugin],
    store: Arc<dyn crate::ResourceArchiveStore + Send + Sync>,
) -> Result<Canwu, CanwuError> {
    let mut canwu = Canwu::from_snapshot_json_with_plugins(json, plugins)?;
    validate_resource_runtime_with_archive_store(&canwu, store.as_ref())?;
    canwu.set_plugin_archive_object_provider(runtime_archive_provider(store));
    Ok(canwu)
}

pub fn from_resource_checkpoint_journal(
    bundle: canwu_api::CheckpointJournal,
    plugins: &[&dyn SimulationPlugin],
) -> Result<Canwu, CanwuError> {
    let canwu = Canwu::from_checkpoint_journal_with_plugins(bundle, plugins)?;
    validate_resource_runtime(&canwu)?;
    reject_archive_backed_restore_without_store(&canwu)?;
    Ok(canwu)
}

/// Restores a run from a checkpoint journal, checks its resource state
/// against `store`, and attaches `store` as the run's plugin archive provider.
/// The store must be `Send + Sync`, as for
/// [`from_resource_snapshot_json_with_archive_store`].
pub fn from_resource_checkpoint_journal_with_archive_store(
    bundle: canwu_api::CheckpointJournal,
    plugins: &[&dyn SimulationPlugin],
    store: Arc<dyn crate::ResourceArchiveStore + Send + Sync>,
) -> Result<Canwu, CanwuError> {
    let mut canwu = Canwu::from_checkpoint_journal_with_plugins(bundle, plugins)?;
    validate_resource_runtime_with_archive_store(&canwu, store.as_ref())?;
    canwu.set_plugin_archive_object_provider(runtime_archive_provider(store));
    Ok(canwu)
}

pub fn replay_resource_from_journal(
    plugins: &[&dyn SimulationPlugin],
    journal: &ReplayJournal,
) -> Result<Canwu, CanwuError> {
    let canwu = Canwu::replay_from_journal(plugins, journal)?;
    validate_resource_runtime(&canwu)?;
    reject_archive_backed_restore_without_store(&canwu)?;
    Ok(canwu)
}

/// Replays a run with `store` attached as its plugin archive provider from
/// the first boundary, then checks its resource state against `store`. The
/// store must be `Send + Sync`, as for
/// [`from_resource_snapshot_json_with_archive_store`].
pub fn replay_resource_from_journal_with_archive_store(
    plugins: &[&dyn SimulationPlugin],
    journal: &ReplayJournal,
    store: Arc<dyn crate::ResourceArchiveStore + Send + Sync>,
) -> Result<Canwu, CanwuError> {
    let validation_store = Arc::clone(&store);
    let canwu = Canwu::replay_from_journal_with_archive_provider(
        plugins,
        journal,
        runtime_archive_provider(store),
    )?;
    validate_resource_runtime_with_archive_store(&canwu, validation_store.as_ref())?;
    Ok(canwu)
}

fn reject_archive_backed_restore_without_store(canwu: &Canwu) -> Result<(), CanwuError> {
    let Some((_, state)) = resource_state(canwu)? else {
        return Ok(());
    };
    if state.archive_head.directory_root.is_some()
        || state.archive_head.archived_record_count != 0
        || !state.archive_retention_handles.is_empty()
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidArchive,
            "resource restore requires an authenticated archive store for archive-backed state",
        ));
    }
    Ok(())
}

fn unavailable(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::DomainRecordNotFound, message)
}

fn invalid(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidDomainRecord, message)
}

#[allow(clippy::needless_pass_by_value)]
fn resource_error(error: ResourceError) -> CanwuError {
    invalid(error.to_string())
}
