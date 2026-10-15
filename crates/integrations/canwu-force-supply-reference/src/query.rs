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
use std::rc::Rc;

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

/// The holder-relative force-supply report, read with the attached
/// archive provider. Takes either runtime handle.
pub fn force_supply_report<H: canwu_api::CanwuRun + canwu_api::PluginArchiveObjectProvider>(
    canwu: &H,
    holder: &KnowledgeHolderRef,
    force: &ReferenceForceId,
) -> Result<ForceSupplyReportV1, CanwuError> {
    let record = canwu_api::typed_domain_record(canwu, &force_supply_runtime_reference())
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

/// Which resource outcomes a validation of the force runtime compares.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ResourceOutcomes {
    /// The ones the resource root holds hot. One it does not hold is
    /// unavailable only when the resource root has no archive to hold it.
    Hot,
    /// Hot ones, and archived ones through the archive provider attached to
    /// the runtime.
    HotAndArchived,
}

/// Checks that the resource root of `canwu` holds `expected`, the exact
/// outcome a force record cites. An outcome is written once: the resource
/// root holds it while it is hot, and the resource archive once a commit has
/// archived it, so the cited version's body need not be read for it.
fn check_resource_outcome(
    canwu: &Canwu,
    resource: Option<&canwu_resource::ResourceState>,
    expected: &canwu_resource::ResourceOperationOutcomeVersionV1,
    outcomes: ResourceOutcomes,
    unavailable: &str,
    differs: &str,
) -> Result<(), CanwuError> {
    let Some(resource) = resource else {
        return Err(invalid(unavailable));
    };
    let exact = match resource.outcomes.get(&expected.operation_key) {
        Some(outcome) => outcome.clone(),
        None if resource.archive_head.directory_root.is_none() => {
            return Err(invalid(unavailable));
        }
        None if outcomes == ResourceOutcomes::Hot => return Ok(()),
        None => canwu_resource::archived_resource_operation_outcome(
            resource,
            &HostResourceArchive(canwu),
            &expected.operation_key,
        )
        .map_err(|error| invalid(format!("resource archive lookup failed: {error}")))?
        .ok_or_else(|| invalid(unavailable))?,
    };
    if canwu_resource::ResourceOperationOutcomeVersionV1::from(&exact) != *expected {
        return Err(invalid(differs));
    }
    Ok(())
}

/// The resource archive as the provider attached to a runtime serves it.
struct HostResourceArchive<'a>(&'a Canwu);

impl canwu_resource::ResourceArchiveStore for HostResourceArchive<'_> {
    fn store_resource_archive_object(
        &self,
        _namespace: &str,
        _object_id: &str,
        _bytes: &[u8],
    ) -> Result<(), canwu_resource::ResourceError> {
        Err(canwu_resource::ResourceError::InvalidLifecycle(
            "the attached resource archive is read here only".to_owned(),
        ))
    }

    fn load_resource_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, canwu_resource::ResourceError> {
        self.0
            .plugin_archive_object(namespace, object_id)
            .map_err(|error| canwu_resource::ResourceError::InvalidDefinition(error.to_string()))
    }

    fn persist_resource_archive_retention(
        &self,
        _handle: &canwu_resource::ResourceArchiveRetentionHandleV1,
    ) -> Result<(), canwu_resource::ResourceError> {
        Err(canwu_resource::ResourceError::InvalidLifecycle(
            "the attached resource archive is read here only".to_owned(),
        ))
    }

    fn finalize_resource_archive_retention(
        &self,
        _handle_id: &str,
        _phase: canwu_resource::ResourceArchiveRetentionPhaseV1,
    ) -> Result<(), canwu_resource::ResourceError> {
        Err(canwu_resource::ResourceError::InvalidLifecycle(
            "the attached resource archive is read here only".to_owned(),
        ))
    }
}

/// Validates the hot force runtime and the exact versions it cites, without
/// reading any archive.
///
/// A resource outcome it cites is compared only while the resource root
/// holds it hot; one that is not hot is refused only when the resource root
/// has no archive. An archived one is compared by
/// [`validate_force_supply_runtime_with_archive_store`] and by the restore
/// helpers, which read it through the runtime's archive provider.
pub fn validate_force_supply_runtime(canwu: &Canwu) -> Result<(), CanwuError> {
    validate_runtime(canwu, ResourceOutcomes::Hot)
}

#[allow(clippy::too_many_lines)]
fn validate_runtime(canwu: &Canwu, outcomes: ResourceOutcomes) -> Result<(), CanwuError> {
    let Some(record) = canwu_api::typed_domain_record(canwu, &force_supply_runtime_reference())
    else {
        return Ok(());
    };
    let state = record.decode_payload::<ForceSupplyRuntimeRecord>()?;
    state.validate()?;
    if state.provider_record_version != record.version || state.draft()?.payload != record.payload {
        return Err(invalid(
            "force-supply runtime root version or canonical encoding differs",
        ));
    }
    let resource = canwu_resource::resource_state(canwu)?.map(|(_, state)| state);
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
            check_resource_outcome(
                canwu,
                resource.as_ref(),
                outcome,
                outcomes,
                "retained resource outcome is unavailable",
                "retained resource outcome exact binding differs",
            )?;
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
        check_resource_outcome(
            canwu,
            resource.as_ref(),
            &receipt.resource_outcome,
            outcomes,
            "archived resource outcome is unavailable",
            "archived resource outcome exact binding differs",
        )?;
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
/// package archive object and readable store-side retention phase, and every
/// archived resource outcome it cites, which it reads through the archive
/// provider attached to `canwu`.
pub fn validate_force_supply_runtime_with_archive_store(
    canwu: &Canwu,
    store: &dyn crate::PackageArchiveStore,
) -> Result<(), CanwuError> {
    validate_runtime(canwu, ResourceOutcomes::HotAndArchived)?;
    let Some(record) = canwu_api::typed_domain_record(canwu, &force_supply_runtime_reference())
    else {
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

/// Restores a runtime from a snapshot, attaches `archive_provider`, the
/// archive the runtime's plugins read archived records from, and validates
/// the force runtime, every resource outcome it cites included, hot or
/// archived.
pub fn from_force_supply_snapshot_json(
    json: &str,
    plugins: &[&dyn canwu_api::SimulationPlugin],
    archive_provider: Rc<dyn canwu_api::PluginArchiveObjectProvider>,
) -> Result<Canwu, CanwuError> {
    let mut canwu = Canwu::from_snapshot_json_with_plugins(json, plugins)?;
    canwu.set_plugin_archive_object_provider(archive_provider);
    validate_runtime(&canwu, ResourceOutcomes::HotAndArchived)?;
    Ok(canwu)
}

/// Restores a runtime from a checkpoint journal, attaches `archive_provider`,
/// and validates the force runtime as [`from_force_supply_snapshot_json`]
/// does.
pub fn from_force_supply_checkpoint_journal(
    bundle: canwu_api::CheckpointJournal,
    plugins: &[&dyn canwu_api::SimulationPlugin],
    archive_provider: Rc<dyn canwu_api::PluginArchiveObjectProvider>,
) -> Result<Canwu, CanwuError> {
    let mut canwu = Canwu::from_checkpoint_journal_with_plugins(bundle, plugins)?;
    canwu.set_plugin_archive_object_provider(archive_provider);
    validate_runtime(&canwu, ResourceOutcomes::HotAndArchived)?;
    Ok(canwu)
}

/// Replays a journal with `archive_provider` attached and validates the
/// force runtime as [`from_force_supply_snapshot_json`] does.
pub fn replay_force_supply_from_journal(
    plugins: &[&dyn canwu_api::SimulationPlugin],
    journal: &canwu_api::ReplayJournal,
    archive_provider: Rc<dyn canwu_api::PluginArchiveObjectProvider>,
) -> Result<Canwu, CanwuError> {
    let canwu =
        Canwu::replay_from_journal_with_archive_provider(plugins, journal, archive_provider)?;
    validate_runtime(&canwu, ResourceOutcomes::HotAndArchived)?;
    Ok(canwu)
}

#[cfg(test)]
mod tests {
    use super::{ResourceOutcomes, check_resource_outcome};
    use canwu_api::{BoundaryRequest, Canwu, CanwuError, EntityRef, PersonId, Scenario, SimTime};
    use canwu_resource::{
        ResourceArchiveRetentionHandleV1, ResourceArchiveRetentionPhaseV1, ResourceArchiveStore,
        ResourceCancelDemandRequestV1, ResourceDemandId, ResourceError, ResourceLimitsV1,
        ResourceOperationKey, ResourceOperationOutcomeVersionV1, ResourceOperationRequestV1,
        ResourcePlugin, ResourceRevision, ResourceState, enqueue_resource_archive, resource_state,
    };
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::rc::Rc;

    /// Stored archive objects by namespace and object id.
    type ArchiveObjects = BTreeMap<(String, String), Vec<u8>>;

    #[derive(Clone, Default)]
    struct MemoryArchive {
        objects: Rc<RefCell<ArchiveObjects>>,
    }

    impl ResourceArchiveStore for MemoryArchive {
        fn store_resource_archive_object(
            &self,
            namespace: &str,
            object_id: &str,
            bytes: &[u8],
        ) -> Result<(), ResourceError> {
            self.objects
                .borrow_mut()
                .insert((namespace.to_owned(), object_id.to_owned()), bytes.to_vec());
            Ok(())
        }

        fn load_resource_archive_object(
            &self,
            namespace: &str,
            object_id: &str,
        ) -> Result<Option<Vec<u8>>, ResourceError> {
            Ok(self
                .objects
                .borrow()
                .get(&(namespace.to_owned(), object_id.to_owned()))
                .cloned())
        }

        fn persist_resource_archive_retention(
            &self,
            _handle: &ResourceArchiveRetentionHandleV1,
        ) -> Result<(), ResourceError> {
            Ok(())
        }

        fn finalize_resource_archive_retention(
            &self,
            _handle_id: &str,
            _phase: ResourceArchiveRetentionPhaseV1,
        ) -> Result<(), ResourceError> {
            Ok(())
        }
    }

    impl canwu_api::PluginArchiveObjectProvider for MemoryArchive {
        fn load_plugin_archive_object(
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
    }

    fn key(value: &str) -> ResourceOperationKey {
        ResourceOperationKey::new(value).expect("operation key")
    }

    /// A runtime whose resource root has archived the outcome of one durable
    /// rejection, and the exact version of that outcome a record would cite.
    fn archived_outcome() -> (Canwu, ResourceOperationOutcomeVersionV1) {
        let mut state = ResourceState::empty(ResourceLimitsV1::canonical()).expect("state");
        let request = ResourceOperationRequestV1::CancelDemand(ResourceCancelDemandRequestV1 {
            operation_key: key("fixture:operation:cancel-missing"),
            demand: ResourceDemandId::new("fixture:demand:missing").expect("demand"),
            expected_demand_revision: ResourceRevision::INITIAL,
        });
        state.apply_operation(&request).expect("durable rejection");
        let cited =
            ResourceOperationOutcomeVersionV1::from(&state.outcomes[&request.operation_key()]);
        let archive = MemoryArchive::default();
        let plugin = ResourcePlugin::default();
        let scenario = Scenario::new(SimTime::EPOCH, vec![EntityRef::Person(PersonId::new(1))])
            .with_domain_records(state.into_records().expect("the root persists"));
        let mut canwu = Canwu::new_with_plugins(1, scenario, &[&plugin]).expect("runtime");
        canwu.set_plugin_archive_object_provider(Rc::new(archive.clone()));
        let (_, root) = resource_state(&canwu)
            .expect("resource root")
            .expect("resource runtime");
        let prepared = root
            .prepare_resource_archive(1, &archive)
            .expect("archive batch");
        enqueue_resource_archive(&mut canwu, &prepared, &archive).expect("archive commit");
        canwu
            .settle_boundary(BoundaryRequest::at(canwu.time()))
            .expect("the commit applies");
        (canwu, cited)
    }

    #[test]
    fn an_archived_citation_is_compared_with_the_archived_outcome() {
        let (canwu, cited) = archived_outcome();
        let (_, resource) = resource_state(&canwu)
            .expect("resource root")
            .expect("resource runtime");
        assert!(!resource.outcomes.contains_key(&cited.operation_key));
        let check = |cited: &ResourceOperationOutcomeVersionV1, outcomes| {
            check_resource_outcome(
                &canwu,
                Some(&resource),
                cited,
                outcomes,
                "unavailable",
                "differs",
            )
        };
        check(&cited, ResourceOutcomes::HotAndArchived).expect("the archived outcome is cited");

        let mut forged = cited.clone();
        forged.quantity += 1;
        let error = check(&forged, ResourceOutcomes::HotAndArchived).expect_err("forged");
        assert_eq!(error.message, "differs");
        let mut unsettled = cited.clone();
        unsettled.operation_key = key("fixture:operation:never-settled");
        let error = check(&unsettled, ResourceOutcomes::HotAndArchived).expect_err("unsettled");
        assert_eq!(error.message, "unavailable");

        // Validation that reads no archive leaves an archived citation to
        // the restore helpers.
        check(&forged, ResourceOutcomes::Hot).expect("not compared without the archive");
    }
}
