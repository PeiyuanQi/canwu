use crate::{
    AllocationLegStatus, DemandStatus, ResourceAccountId, ResourceAllocationLeg,
    ResourceAllocationLegId, ResourceAllocationLegVersionV1, ResourceConsumption,
    ResourceConsumptionId, ResourceConsumptionVersionV1, ResourceDemandId, ResourceError,
    ResourceFulfillmentId, ResourceFulfillmentVersionV1, ResourceOperationKey,
    ResourceOperationOutcome, ResourceOperationOutcomeId, ResourceOperationOutcomeVersionV1,
    ResourceReportGrantId, ResourceReportId, ResourceRevision, ResourceRuntimeRecord,
    ResourceScopeId, ResourceState, canonical_digest, resource_runtime_reference,
};
use canwu_api::{
    Canwu, CanwuError, DomainRecord, DomainRecordVersionRef, ErrorCode, KnowledgeHolderRef,
    PluginArchiveObjectProvider, ReplayJournal, SimTime, SimulationPlugin,
};
use serde::{Deserialize, Serialize};
use std::rc::Rc;

struct ResourceRuntimeArchiveProvider {
    store: Rc<dyn crate::ResourceArchiveStore>,
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
    store: Rc<dyn crate::ResourceArchiveStore>,
) -> Rc<dyn PluginArchiveObjectProvider> {
    Rc::new(ResourceRuntimeArchiveProvider { store })
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
    pub source_versions: Vec<DomainRecordVersionRef>,
    pub adapter_revision: crate::ResourceObservationAdapterRevisionId,
    pub digest: String,
}

pub fn resource_state(canwu: &Canwu) -> Result<Option<(DomainRecord, ResourceState)>, CanwuError> {
    let reference = resource_runtime_reference();
    let Some(record) = canwu.typed_domain_record(&reference).cloned() else {
        return Ok(None);
    };
    let state = record.decode_payload::<ResourceRuntimeRecord>()?;
    Ok(Some((record, state)))
}

#[must_use]
pub fn resource_allocation_leg(
    canwu: &Canwu,
    id: &ResourceAllocationLegId,
) -> Option<ResourceAllocationLegVersionV1> {
    resource_state(canwu).ok().flatten().and_then(|(_, state)| {
        state
            .allocation_legs
            .get(id)
            .map(ResourceAllocationLegVersionV1::from)
    })
}

pub fn exact_resource_allocation_leg(
    canwu: &Canwu,
    exact: &ResourceAllocationLegVersionV1,
) -> Result<ResourceAllocationLeg, CanwuError> {
    let (_, state) =
        resource_state(canwu)?.ok_or_else(|| unavailable("resource runtime is absent"))?;
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
}

#[must_use]
pub fn resource_consumption(
    canwu: &Canwu,
    id: &ResourceConsumptionId,
) -> Option<ResourceConsumptionVersionV1> {
    resource_state(canwu).ok().flatten().and_then(|(_, state)| {
        state
            .consumptions
            .get(id)
            .map(ResourceConsumptionVersionV1::from)
    })
}

pub fn exact_resource_consumption(
    canwu: &Canwu,
    exact: &ResourceConsumptionVersionV1,
) -> Result<ResourceConsumption, CanwuError> {
    let (_, state) =
        resource_state(canwu)?.ok_or_else(|| unavailable("resource runtime is absent"))?;
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
}

#[must_use]
pub fn resource_operation_outcome(
    canwu: &Canwu,
    operation_key: &ResourceOperationKey,
) -> Option<ResourceOperationOutcomeVersionV1> {
    resource_state(canwu)
        .ok()
        .flatten()
        .and_then(|(_, state)| state.outcomes.get(operation_key).map(Into::into))
}

#[must_use]
pub fn resource_operation_outcome_by_id(
    canwu: &Canwu,
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

#[must_use]
pub fn resource_completion_certificate(
    canwu: &Canwu,
    acquisition: &crate::CompletionLeaseAcquisitionId,
) -> Option<crate::CompletionLeaseActivationCertificateV1> {
    resource_state(canwu)
        .ok()
        .flatten()
        .and_then(|(_, state)| state.completion_leases.certificate(acquisition).cloned())
}

#[must_use]
pub fn resource_completion_grant(
    canwu: &Canwu,
    grant: &crate::CompletionCapacityGrantId,
) -> Option<crate::CompletionCapacityGrantV1> {
    resource_state(canwu)
        .ok()
        .flatten()
        .and_then(|(_, state)| state.completion_leases.grants.get(grant).cloned())
}

pub fn resource_completion_status(
    canwu: &Canwu,
    holder: &KnowledgeHolderRef,
    acquisition: &crate::CompletionLeaseAcquisitionId,
) -> Result<crate::CompletionLeaseStatusDtoV1, CanwuError> {
    let (_, state) =
        resource_state(canwu)?.ok_or_else(|| unavailable("resource runtime is absent"))?;
    state
        .completion_leases
        .status_for(holder, acquisition)
        .map_err(resource_error)
}

pub fn exact_resource_completion_certificate(
    canwu: &Canwu,
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

pub fn exact_resource_operation_outcome(
    canwu: &Canwu,
    exact: &ResourceOperationOutcomeVersionV1,
) -> Result<ResourceOperationOutcome, CanwuError> {
    let (_, state) =
        resource_state(canwu)?.ok_or_else(|| unavailable("resource runtime is absent"))?;
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
}

#[must_use]
pub fn latest_resource_fulfillment(
    canwu: &Canwu,
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

pub fn exact_resource_fulfillment(
    canwu: &Canwu,
    exact: &ResourceFulfillmentVersionV1,
) -> Result<crate::ResourceFulfillment, CanwuError> {
    let (_, state) =
        resource_state(canwu)?.ok_or_else(|| unavailable("resource runtime is absent"))?;
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
}

pub fn resource_report(
    canwu: &Canwu,
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
pub fn resource_observation_witness(
    canwu: &Canwu,
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

pub fn validate_resource_runtime(canwu: &Canwu) -> Result<(), CanwuError> {
    let Some((record, state)) = resource_state(canwu)? else {
        return Ok(());
    };
    state.validate().map_err(resource_error)?;
    if record.version != state.state_revision.get() {
        return Err(invalid(
            "resource runtime domain-record version differs from its state revision",
        ));
    }
    let encoded = serde_json::to_vec(&state).map_err(|error| invalid(error.to_string()))?;
    if encoded.len() > state.limits.max_state_bytes {
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

pub fn validate_resource_runtime_with_archive_store(
    canwu: &Canwu,
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

pub fn from_resource_snapshot_json_with_archive_store(
    json: &str,
    plugins: &[&dyn SimulationPlugin],
    store: Rc<dyn crate::ResourceArchiveStore>,
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

pub fn from_resource_checkpoint_journal_with_archive_store(
    bundle: canwu_api::CheckpointJournal,
    plugins: &[&dyn SimulationPlugin],
    store: Rc<dyn crate::ResourceArchiveStore>,
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

pub fn replay_resource_from_journal_with_archive_store(
    plugins: &[&dyn SimulationPlugin],
    journal: &ReplayJournal,
    store: Rc<dyn crate::ResourceArchiveStore>,
) -> Result<Canwu, CanwuError> {
    let validation_store = Rc::clone(&store);
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
