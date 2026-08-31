use crate::{
    ForceObservationRole, ForceReportId, ForceSupplyRuntimeRecord, ForceSupplyStateV1,
    ReferenceForceId, RequisitionSagaStage, ShortageAttributionV1, force_supply_runtime_reference,
    invalid,
};
use canwu_api::{
    Canwu, CanwuError, DomainRecordVersionRef, KnowledgeHolderRef, SimDuration, SimTime,
    canonical_hash,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const MAX_ARCHIVED_REPORT_RECORDS: usize = 16_384;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ForceSupplyReportV1 {
    pub id: ForceReportId,
    pub provider_state_revision: u64,
    pub holder: KnowledgeHolderRef,
    pub force: ReferenceForceId,
    pub role: ForceObservationRole,
    pub observed_at: SimTime,
    pub materialized_at: SimTime,
    pub confidence_per_mille: u16,
    pub source_versions: Vec<DomainRecordVersionRef>,
    pub observations: Vec<crate::ForceSupplyObservationV1>,
    pub shortage_attribution: Vec<ShortageAttributionV1>,
    pub requisition_stage: Option<RequisitionSagaStage>,
    pub latest_outcome_or_ack: Option<String>,
    pub recoverable_blocker: Option<String>,
    pub canonical_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ForceSupplyObservationWitnessV1 {
    pub provider_plugin: String,
    pub provider_version: String,
    pub provider_semantic_hash: String,
    pub provider_state_revision: u64,
    pub holder: KnowledgeHolderRef,
    pub force: ReferenceForceId,
    pub observed_at: SimTime,
    pub materialized_at: SimTime,
    pub confidence_per_mille: u16,
    pub source_versions: Vec<DomainRecordVersionRef>,
    pub report_digest: String,
    pub adapter_revision: String,
    pub canonical_digest: String,
}

pub fn force_supply_report(
    canwu: &Canwu,
    holder: &KnowledgeHolderRef,
    force: &ReferenceForceId,
) -> Result<ForceSupplyReportV1, CanwuError> {
    let record = canwu
        .typed_domain_record(&force_supply_runtime_reference())
        .ok_or_else(|| invalid("force-supply runtime is not configured"))?;
    let state = record.decode_payload::<ForceSupplyRuntimeRecord>()?;
    let archived = crate::load_package_archive_records::<
        crate::ForceArchiveKeyV1,
        crate::ForceArchivePayloadV1,
    >(
        crate::FORCE_ARCHIVE_DOMAIN,
        &state.archive_head,
        canwu,
        MAX_ARCHIVED_REPORT_RECORDS,
    )?
    .into_iter()
    .filter_map(|record| match record.payload {
        crate::ForceArchivePayloadV1::KnowledgePublication(publication) => Some(publication),
        _ => None,
    })
    .collect::<Vec<_>>();
    report_from_state_with_archive(&state, holder, force, canwu.time(), &archived)
}

pub fn force_supply_observation_witness(
    canwu: &Canwu,
    holder: &KnowledgeHolderRef,
    force: &ReferenceForceId,
) -> Result<ForceSupplyObservationWitnessV1, CanwuError> {
    let report = force_supply_report(canwu, holder, force)?;
    if report.source_versions.is_empty() {
        return Err(invalid(
            "force observation witness has no exact provider source versions",
        ));
    }
    let mut witness = ForceSupplyObservationWitnessV1 {
        provider_plugin: crate::PLUGIN_NAME.to_owned(),
        provider_version: env!("CARGO_PKG_VERSION").to_owned(),
        provider_semantic_hash: crate::FORCE_SUPPLY_SEMANTIC_HASH.to_owned(),
        provider_state_revision: report.provider_state_revision,
        holder: holder.clone(),
        force: force.clone(),
        observed_at: report.observed_at,
        materialized_at: report.materialized_at,
        confidence_per_mille: report.confidence_per_mille,
        source_versions: report.source_versions,
        report_digest: report.canonical_digest,
        adapter_revision: "canwu.force-supply.observation-adapter.v1".to_owned(),
        canonical_digest: String::new(),
    };
    witness.canonical_digest =
        canonical_hash("canwu.force-supply.observation-witness.v1", &witness)?;
    Ok(witness)
}

pub fn validate_force_supply_observation_witness(
    canwu: &Canwu,
    witness: &ForceSupplyObservationWitnessV1,
) -> Result<(), CanwuError> {
    let mut detached = witness.clone();
    let recorded = std::mem::take(&mut detached.canonical_digest);
    if witness.provider_plugin != crate::PLUGIN_NAME
        || witness.provider_version != env!("CARGO_PKG_VERSION")
        || witness.provider_semantic_hash != crate::FORCE_SUPPLY_SEMANTIC_HASH
        || witness.adapter_revision != "canwu.force-supply.observation-adapter.v1"
        || witness.source_versions.is_empty()
        || recorded != canonical_hash("canwu.force-supply.observation-witness.v1", &detached)?
    {
        return Err(invalid("force observation witness is forged"));
    }
    for source in &witness.source_versions {
        canwu
            .domain_record_version(source)
            .ok_or_else(|| invalid("force observation witness source body is unavailable"))?;
    }
    let exact = force_supply_observation_witness(canwu, &witness.holder, &witness.force)?;
    if &exact != witness {
        return Err(invalid(
            "force observation witness differs from the authoritative holder head",
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
pub fn report_from_state(
    state: &ForceSupplyStateV1,
    holder: &KnowledgeHolderRef,
    force: &ReferenceForceId,
    now: SimTime,
) -> Result<ForceSupplyReportV1, CanwuError> {
    report_from_state_with_archive(state, holder, force, now, &[])
}

#[allow(clippy::too_many_lines)]
fn report_from_state_with_archive(
    state: &ForceSupplyStateV1,
    holder: &KnowledgeHolderRef,
    force: &ReferenceForceId,
    now: SimTime,
    archived: &[crate::ForceKnowledgePublicationV1],
) -> Result<ForceSupplyReportV1, CanwuError> {
    let grant = state
        .observation_grants
        .values()
        .find(|grant| &grant.holder == holder && &grant.force == force)
        .ok_or_else(|| {
            CanwuError::new(
                canwu_api::ErrorCode::InvalidAuthority,
                "holder has no force-supply observation grant",
            )
        })?;
    let delay = i64::try_from(grant.observation_delay_minutes)
        .map_err(|_| invalid("force observation delay exceeds simulation time"))?;
    let observed_at = now
        .checked_add(SimDuration::minutes(-delay))
        .ok_or_else(|| invalid("force observation delay underflowed"))?;
    let mut visible_by_scope = BTreeMap::new();
    for publication in state
        .knowledge_publications
        .values()
        .chain(archived.iter())
        .filter(|publication| {
            &publication.holder == holder
                && &publication.force == force
                && publication.observed_at <= observed_at
                && publication.available_at <= now
        })
    {
        let scope = crate::observation_temporal_scope_key(
            &publication.holder,
            &publication.force,
            &publication.fact,
        );
        let replace = visible_by_scope.get(&scope).is_none_or(
            |current: &&crate::ForceKnowledgePublicationV1| {
                (current.observed_at, current.provider_revision, &current.id)
                    < (
                        publication.observed_at,
                        publication.provider_revision,
                        &publication.id,
                    )
            },
        );
        if replace {
            visible_by_scope.insert(scope, publication);
        }
    }
    let mut visible = visible_by_scope.into_values().collect::<Vec<_>>();
    visible.sort_by(|left, right| {
        (left.observed_at, left.provider_revision, &left.id).cmp(&(
            right.observed_at,
            right.provider_revision,
            &right.id,
        ))
    });
    let mut observation_heads = std::collections::BTreeMap::new();
    for publication in &visible {
        if let crate::ForceKnowledgeFactV1::SupplyObservation(observation) = &publication.fact {
            observation_heads.insert(observation.requirement.clone(), observation.clone());
        }
    }
    let mut observations: Vec<_> = observation_heads.into_values().collect();
    observations.sort_by(|left, right| left.requirement.cmp(&right.requirement));
    observations.truncate(crate::MAX_REPORT_FACTS);
    let mut shortage_attribution: Vec<_> = visible
        .iter()
        .filter_map(|publication| match &publication.fact {
            crate::ForceKnowledgeFactV1::ShortageAttribution(attribution) => {
                Some(attribution.clone())
            }
            _ => None,
        })
        .collect();
    shortage_attribution.sort_by(|left, right| {
        left.requirement
            .cmp(&right.requirement)
            .then_with(|| left.resource_outcome.id.cmp(&right.resource_outcome.id))
    });
    shortage_attribution.truncate(crate::MAX_REPORT_FACTS.saturating_sub(observations.len()));
    if grant.role == ForceObservationRole::RemoteCommander {
        for observation in &mut observations {
            let margin = observation
                .known_stock_high
                .saturating_sub(observation.known_stock_low)
                .max(1);
            observation.known_stock_low = observation.known_stock_low.saturating_sub(margin);
            observation.known_stock_high = observation.known_stock_high.saturating_add(margin);
        }
    }
    let progress = visible.iter().rev().find_map(|publication| {
        if let crate::ForceKnowledgeFactV1::RequisitionProgress {
            stage,
            latest_outcome_or_ack,
            recoverable_blocker,
        } = &publication.fact
        {
            Some((
                *stage,
                latest_outcome_or_ack.clone(),
                recoverable_blocker.clone(),
            ))
        } else {
            None
        }
    });
    let mut source_versions = visible
        .iter()
        .flat_map(|publication| publication.source_versions.iter().cloned())
        .collect::<Vec<_>>();
    source_versions.sort();
    source_versions.dedup();
    let id = ForceReportId::new(format!(
        "canwu.force-supply-reference:report:{}:{}",
        force,
        now.as_minutes()
    ))?;
    let mut report = ForceSupplyReportV1 {
        id,
        provider_state_revision: state.revision,
        holder: holder.clone(),
        force: force.clone(),
        role: grant.role,
        observed_at,
        materialized_at: now,
        confidence_per_mille: grant.confidence_per_mille,
        source_versions,
        observations,
        shortage_attribution,
        requisition_stage: progress.as_ref().map(|value| value.0),
        latest_outcome_or_ack: progress.as_ref().and_then(|value| value.1.clone()),
        recoverable_blocker: progress.and_then(|value| value.2),
        canonical_digest: String::new(),
    };
    report.canonical_digest = canonical_hash("canwu.force-supply.holder-report.v1", &report)?;
    Ok(report)
}

#[allow(clippy::too_many_lines)]
pub fn validate_force_supply_runtime(canwu: &Canwu) -> Result<(), CanwuError> {
    let Some(record) = canwu.typed_domain_record(&force_supply_runtime_reference()) else {
        return Ok(());
    };
    let state = record.decode_payload::<ForceSupplyRuntimeRecord>()?;
    state.validate()?;
    if state.provider_record_version != record.version || state.draft()?.payload != record.payload {
        return Err(invalid(
            "force-supply runtime root version or canonical encoding differs",
        ));
    }
    for intent in state.intents.values() {
        for target in &intent.completion_certificate.locked_target_versions {
            if let canwu_resource::CompletionLockedTargetV1::ExternalRecord { version } = target {
                canwu.domain_record_version(version).ok_or_else(|| {
                    invalid("force completion certificate exact target body is unavailable")
                })?;
            }
        }
        if let (Some(outcome), Some(source)) =
            (&intent.resource_outcome, &intent.resource_outcome_source)
        {
            let record = canwu.domain_record_version(source).ok_or_else(|| {
                invalid("retained resource outcome provider version is unavailable")
            })?;
            if record.reference != canwu_resource::resource_runtime_reference().into_untyped()
                || record.owner != "canwu-resource"
            {
                return Err(invalid("retained resource outcome provider is invalid"));
            }
            let resource = record.decode_payload::<canwu_resource::ResourceRuntimeRecord>()?;
            let exact = resource
                .outcomes
                .get(&outcome.operation_key)
                .map(canwu_resource::ResourceOperationOutcomeVersionV1::from)
                .ok_or_else(|| invalid("retained resource outcome is unavailable"))?;
            if &exact != outcome {
                return Err(invalid("retained resource outcome exact binding differs"));
            }
        }
    }
    for publication in state.knowledge_publications.values() {
        for source in &publication.source_versions {
            canwu.domain_record_version(source).ok_or_else(|| {
                invalid("force knowledge publication exact source body is unavailable")
            })?;
        }
    }
    for saga in state.sagas.values() {
        if let (Some(outcome), Some(source)) =
            (&saga.externality_outcome, &saga.externality_outcome_source)
        {
            let record = canwu.domain_record_version(source).ok_or_else(|| {
                invalid("retained economy externality provider version is unavailable")
            })?;
            let acquisition = &state.completion_leases.acquisitions[&state.intents[&saga.intent]
                .completion_certificate
                .acquisition];
            let expected_owner = acquisition
                .expected_participants
                .iter()
                .find(|owner| {
                    owner.as_str() != crate::PLUGIN_NAME
                        && owner.as_str() != canwu_resource::PLUGIN_NAME
                })
                .ok_or_else(|| invalid("retained externality provider is unavailable"))?;
            if record.owner != *expected_owner
                || record.reference
                    != crate::force_externality_outcome_reference(&outcome.id).into_untyped()
                || record.version != outcome.revision
            {
                return Err(invalid(
                    "retained economy externality provider identity differs",
                ));
            }
            let exact = record.decode_payload::<crate::ForceExternalityOutcomeProviderRecord>()?;
            if &exact != outcome {
                return Err(invalid(
                    "retained economy externality exact binding differs",
                ));
            }
        }
    }
    for receipt in state.terminal_receipts.values() {
        let record = canwu
            .domain_record_version(&receipt.resource_outcome_source)
            .ok_or_else(|| invalid("archived resource outcome provider version is unavailable"))?;
        if record.reference != canwu_resource::resource_runtime_reference().into_untyped()
            || record.owner != "canwu-resource"
        {
            return Err(invalid("archived resource outcome provider is invalid"));
        }
        let resource = record.decode_payload::<canwu_resource::ResourceRuntimeRecord>()?;
        let exact = resource
            .outcomes
            .get(&receipt.resource_outcome.operation_key)
            .map(canwu_resource::ResourceOperationOutcomeVersionV1::from)
            .ok_or_else(|| invalid("archived resource outcome is unavailable"))?;
        if exact != receipt.resource_outcome {
            return Err(invalid("archived resource outcome exact binding differs"));
        }
        if let (Some(outcome), Some(source)) = (
            &receipt.externality_outcome,
            &receipt.externality_outcome_source,
        ) {
            let record = canwu.domain_record_version(source).ok_or_else(|| {
                invalid("archived economy externality provider version is unavailable")
            })?;
            let acquisition =
                &state.completion_leases.acquisitions[&receipt.completion_certificate.acquisition];
            let expected_owner = acquisition
                .expected_participants
                .iter()
                .find(|owner| {
                    owner.as_str() != crate::PLUGIN_NAME
                        && owner.as_str() != canwu_resource::PLUGIN_NAME
                })
                .ok_or_else(|| invalid("archived externality provider is unavailable"))?;
            if record.owner != *expected_owner
                || record.reference
                    != crate::force_externality_outcome_reference(&outcome.id).into_untyped()
                || record.version != outcome.revision
                || record.decode_payload::<crate::ForceExternalityOutcomeProviderRecord>()?
                    != *outcome
            {
                return Err(invalid(
                    "archived economy externality exact binding differs",
                ));
            }
        }
    }
    Ok(())
}

/// Validates the hot force runtime together with every committed or pending
/// package archive object and readable store-side retention phase.
pub fn validate_force_supply_runtime_with_archive_store(
    canwu: &Canwu,
    store: &dyn crate::PackageArchiveStore,
) -> Result<(), CanwuError> {
    validate_force_supply_runtime(canwu)?;
    let Some(record) = canwu.typed_domain_record(&force_supply_runtime_reference()) else {
        return Ok(());
    };
    let state = record.decode_payload::<ForceSupplyRuntimeRecord>()?;
    crate::validate_package_archive_store::<crate::ForceArchiveKeyV1, crate::ForceArchivePayloadV1>(
        crate::FORCE_ARCHIVE_DOMAIN,
        &state.archive_head,
        &state.archive_retention_handles,
        &state.archive_maintenance_receipts,
        store,
    )
}

pub fn from_force_supply_snapshot_json(
    json: &str,
    plugins: &[&dyn canwu_api::SimulationPlugin],
) -> Result<Canwu, CanwuError> {
    let canwu = Canwu::from_snapshot_json_with_plugins(json, plugins)?;
    validate_force_supply_runtime(&canwu)?;
    Ok(canwu)
}

pub fn from_force_supply_checkpoint_journal(
    bundle: canwu_api::CheckpointJournal,
    plugins: &[&dyn canwu_api::SimulationPlugin],
) -> Result<Canwu, CanwuError> {
    let canwu = Canwu::from_checkpoint_journal_with_plugins(bundle, plugins)?;
    validate_force_supply_runtime(&canwu)?;
    Ok(canwu)
}

pub fn replay_force_supply_from_journal(
    plugins: &[&dyn canwu_api::SimulationPlugin],
    journal: &canwu_api::ReplayJournal,
) -> Result<Canwu, CanwuError> {
    let canwu = Canwu::replay_from_journal(plugins, journal)?;
    validate_force_supply_runtime(&canwu)?;
    Ok(canwu)
}
