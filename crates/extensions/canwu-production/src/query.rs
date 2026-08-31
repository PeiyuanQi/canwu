use crate::model::{
    MAX_REPORT_FACTS, ProductionExecution, ProductionObservationHeadKeyV1,
    ProductionObservationHeadV1, ProductionObservationId, ProductionObservationReport,
    ProductionObservationWitnessV1, ProductionRuntimeRecord, ProductionState, WorkOrderLifecycle,
    invalid, production_runtime_reference,
};
use canwu_api::{
    Canwu, CanwuError, Command, DecisionAction, DecisionContext, DecisionOption,
    DecisionTicketDraft, DecisionTicketId, DomainRecordVersionRef, EntityRef, EvidenceRef,
    KnowledgeHolderRef, KnowledgeSubjectTarget, PluginArchiveObjectProvider, SimDuration, SimTime,
    canonical_hash,
};
use std::rc::Rc;

struct ProductionRuntimeArchiveProvider {
    production: Rc<dyn crate::ProductionArchiveStore>,
    resource: Rc<dyn canwu_resource::ResourceArchiveStore>,
}

impl PluginArchiveObjectProvider for ProductionRuntimeArchiveProvider {
    fn load_plugin_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, CanwuError> {
        if let Some(bytes) = self
            .production
            .load_production_archive_object(namespace, object_id)?
        {
            return Ok(Some(bytes));
        }
        self.resource
            .load_resource_archive_object(namespace, object_id)
            .map_err(|error| invalid(format!("resource archive provider failed: {error}")))
    }
}

fn runtime_archive_provider(
    production: Rc<dyn crate::ProductionArchiveStore>,
    resource: Rc<dyn canwu_resource::ResourceArchiveStore>,
) -> Rc<dyn PluginArchiveObjectProvider> {
    Rc::new(ProductionRuntimeArchiveProvider {
        production,
        resource,
    })
}

pub fn validate_production_runtime(canwu: &Canwu) -> Result<(), CanwuError> {
    validate_production_runtime_with_archive_providers(canwu, None, None)
}

pub fn validate_production_runtime_with_archives(
    canwu: &Canwu,
    production_archive: &dyn crate::ProductionArchiveStore,
    resource_archive: &dyn canwu_resource::ResourceArchiveStore,
) -> Result<(), CanwuError> {
    validate_production_runtime_with_archive_providers(
        canwu,
        Some(production_archive),
        Some(resource_archive),
    )
}

fn validate_production_runtime_with_archive_providers(
    canwu: &Canwu,
    production_archive: Option<&dyn crate::ProductionArchiveStore>,
    resource_archive: Option<&dyn canwu_resource::ResourceArchiveStore>,
) -> Result<(), CanwuError> {
    let Some(record) = canwu.typed_domain_record(&production_runtime_reference()) else {
        return Ok(());
    };
    let state = record.decode_payload::<ProductionRuntimeRecord>()?;
    state.validate()?;
    if state.draft()?.payload != record.payload {
        return Err(invalid(
            "production runtime root is not canonically encoded",
        ));
    }
    if state.archive.directory_root.is_some() || !state.archive.pending_handles.is_empty() {
        let store = production_archive.ok_or_else(|| {
            invalid(
                "production restore requires its archive provider to authenticate the complete directory and pending-retention closure",
            )
        })?;
        crate::validate_production_archive(store, &state)?;
    }
    let resource_state = if state.executions.is_empty() && state.facility_projects.is_empty() {
        None
    } else {
        Some(
            canwu_resource::resource_state(canwu)?
                .map(|(_, state)| state)
                .ok_or_else(|| {
                    invalid("production runtime requires its exact resource evidence provider")
                })?,
        )
    };
    for execution in state.executions.values() {
        validate_execution_restore(canwu, resource_state.as_ref(), resource_archive, execution)?;
    }
    for project in state.facility_projects.values() {
        validate_project_restore(canwu, resource_state.as_ref(), resource_archive, project)?;
    }
    for (project_id, witness) in &state.resource_continuation_witnesses {
        let resource = resource_state.as_ref().ok_or_else(|| {
            invalid("production continuation witness requires the resource runtime")
        })?;
        let store = resource_archive.ok_or_else(|| {
            invalid("production continuation witness requires its resource archive provider")
        })?;
        let project = state
            .facility_projects
            .get(project_id)
            .ok_or_else(|| invalid("production continuation witness lost its project"))?;
        canwu_resource::authenticate_reachable_resource_archive_directory(
            resource,
            store,
            &witness.resource_archive_directory_root,
            witness.resource_archive_record_count,
        )
        .map_err(|error| resource_archive_error(&error))?;
        for input in &project.inputs {
            validate_resource_input_restore(resource, Some(store), input)?;
        }
    }
    validate_production_decisions(canwu, &state)?;
    validate_production_incident_draws(canwu, &state)?;
    validate_production_knowledge(canwu, &state)?;
    Ok(())
}

fn validate_production_decisions(canwu: &Canwu, state: &ProductionState) -> Result<(), CanwuError> {
    for receipt in state.decision_receipts.values() {
        let ticket = canwu
            .decision_ticket(receipt.ticket_id)
            .ok_or_else(|| invalid("production decision receipt lost its persisted ticket"))?;
        let trace = canwu
            .decision_trace(receipt.trace_id)
            .ok_or_else(|| invalid("production decision receipt lost its persisted trace"))?;
        if ticket.assigned_controller != receipt.controller_id
            || ticket.version != receipt.ticket_version
            || !matches!(
                &ticket.state,
                canwu_api::DecisionTicketState::Resolved { option_id, trace_id }
                    if option_id == &receipt.selected_option && trace_id == &receipt.trace_id
            )
            || trace.ticket_id != receipt.ticket_id
            || trace.controller_id != receipt.controller_id
            || trace.decided_at != receipt.decided_at
            || !matches!(
                &trace.outcome,
                canwu_api::DecisionOutcome::Selected { option_id }
                    if option_id == &receipt.selected_option
            )
            || receipt.command_attempt_id.is_some_and(|attempt_id| {
                !canwu
                    .command_attempts()
                    .iter()
                    .any(|attempt| attempt.id == attempt_id)
            })
        {
            return Err(invalid(
                "production decision receipt does not match its ticket, trace, and command attempt",
            ));
        }
    }
    Ok(())
}

fn validate_production_incident_draws(
    canwu: &Canwu,
    state: &ProductionState,
) -> Result<(), CanwuError> {
    for receipt in state.incident_receipts.values() {
        let source = production_record_at_revision(canwu, receipt.source_record_revision)
            .ok_or_else(|| {
                invalid("production incident source runtime version is unavailable at restore")
            })?;
        if receipt.source_record_digest
            != canonical_hash("canwu.production.runtime-cut.v1", &source)?
        {
            return Err(invalid(
                "production incident source-runtime digest differs from its exact restored body",
            ));
        }
        for sample in std::iter::once(&receipt.random.trigger).chain(receipt.random.severity.iter())
        {
            let exact = canwu.random_draws().iter().any(|draw| {
                draw.at == receipt.evaluated_at
                    && draw.stream == sample.stream
                    && draw.address == sample.address
                    && draw.operation_evidence.as_ref() == Some(&receipt.random.operation_evidence)
                    && draw.upper_exclusive == sample.upper_exclusive
                    && draw.value == sample.value
            });
            if !exact {
                return Err(invalid(
                    "production incident random evidence differs from the authoritative draw journal",
                ));
            }
        }
    }
    Ok(())
}

fn production_record_at_revision(canwu: &Canwu, revision: u64) -> Option<canwu_api::DomainRecord> {
    let reference = production_runtime_reference().into_untyped();
    if revision == 1 {
        let version = canwu_api::DomainRecordVersionRef {
            record: reference.clone(),
            version: 1,
            established_by: canwu_api::DomainRecordVersionSource::InitialScenario,
        };
        if let Some(record) = canwu.domain_record_version(&version) {
            return Some(record);
        }
    }
    for boundary in canwu.boundaries().iter().rev() {
        for (change_index, change) in boundary.record_changes.iter().enumerate().rev() {
            if change.current.reference == reference && change.current.version == revision {
                let version = canwu_api::DomainRecordVersionRef {
                    record: reference.clone(),
                    version: revision,
                    established_by: canwu_api::DomainRecordVersionSource::BoundaryChange {
                        boundary: boundary.id,
                        change_index: u64::try_from(change_index).ok()?,
                    },
                };
                return canwu.domain_record_version(&version);
            }
        }
    }
    None
}

fn validate_execution_restore(
    canwu: &Canwu,
    resource_state: Option<&canwu_resource::ResourceState>,
    resource_archive: Option<&dyn canwu_resource::ResourceArchiveStore>,
    execution: &ProductionExecution,
) -> Result<(), CanwuError> {
    let resource_state =
        resource_state.ok_or_else(|| invalid("production resource state is unavailable"))?;
    resource_state
        .external_completion_participants
        .validate(&resource_state.run_budget)
        .map_err(|error| {
            invalid(format!(
                "production resource completion state is invalid at restore: {error}"
            ))
        })?;
    let participant = if let Some(participant) = resource_state
        .external_completion_participants
        .participant(&execution.completion_certificate.acquisition)
    {
        participant.clone()
    } else if execution.lifecycle == WorkOrderLifecycle::Settled {
        let archived = exact_resource_archive_record(
            resource_state,
            resource_archive,
            &canwu_resource::ResourceTerminalRecordKeyV1::ExternalCompletionParticipant(
                execution.completion_certificate.acquisition.clone(),
            ),
        )?;
        match archived.payload {
            canwu_resource::ResourceTerminalArchivePayloadV1::ExternalCompletionParticipant(
                participant,
            ) => participant,
            _ => {
                return Err(invalid(
                    "archived production execution resource completion grant body is unavailable",
                ));
            }
        }
    } else {
        return Err(invalid(
            "production execution resource completion grant is unavailable at restore",
        ));
    };
    let resource_grant = &participant.grant;
    if participant.certificate.as_ref() != Some(&execution.completion_certificate)
        || resource_grant.id != execution.resource_completion_grant
        || resource_grant.acquisition != execution.completion_certificate.acquisition
        || match execution.lifecycle {
            WorkOrderLifecycle::Settled => !matches!(
                resource_grant.state,
                canwu_resource::CompletionGrantStateV1::Consumed
                    | canwu_resource::CompletionGrantStateV1::Completed
            ),
            _ => resource_grant.state != canwu_resource::CompletionGrantStateV1::Consumed,
        }
    {
        return Err(invalid(
            "production execution resource completion authority differs at restore",
        ));
    }
    for evidence in &execution.evidence {
        let record = canwu
            .domain_record_version(&evidence.version)
            .ok_or_else(|| {
                invalid(format!(
                    "production evidence {:?} is unavailable at restore",
                    evidence.version
                ))
            })?;
        if evidence.semantic_digest
            != canonical_hash("canwu.production.provider-evidence.v1", &record)?
        {
            return Err(invalid(
                "production evidence digest differs from its exact restored body",
            ));
        }
    }
    let mut technology_records = Vec::new();
    for reference in [
        Some(&execution.technology.technique_revision),
        execution.technology.capability_qualification.as_ref(),
        execution.technology.implementation.as_ref(),
        execution.technology.adoption.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        technology_records.push(canwu.domain_record_version(reference).ok_or_else(|| {
            invalid(format!(
                "production technology evidence {reference:?} is unavailable at restore"
            ))
        })?);
    }
    if execution.technology.semantic_digest
        != canonical_hash(
            "canwu.production.technology-binding.v1",
            &technology_records,
        )?
    {
        return Err(invalid(
            "production technology digest differs from its exact restored bodies",
        ));
    }
    for input in &execution.inputs {
        validate_resource_input_restore(resource_state, resource_archive, input)?;
    }
    for (request, expected) in execution
        .output_requests
        .iter()
        .zip(&execution.output_outcomes)
    {
        validate_settled_output_restore(
            canwu,
            resource_state,
            resource_archive,
            execution,
            request,
            expected,
        )?;
    }
    Ok(())
}

fn validate_project_restore(
    canwu: &Canwu,
    resource_state: Option<&canwu_resource::ResourceState>,
    resource_archive: Option<&dyn canwu_resource::ResourceArchiveStore>,
    project: &crate::FacilityProject,
) -> Result<(), CanwuError> {
    let resource_state = resource_state
        .ok_or_else(|| invalid("production project resource state is unavailable"))?;
    resource_state
        .external_completion_participants
        .validate(&resource_state.run_budget)
        .map_err(|error| {
            invalid(format!(
                "production project resource completion state is invalid at restore: {error}"
            ))
        })?;
    let participant = if let Some(participant) = resource_state
        .external_completion_participants
        .participant(&project.completion_certificate.acquisition)
    {
        participant.clone()
    } else if project.lifecycle == crate::FacilityProjectLifecycle::Completed {
        let archived = exact_resource_archive_record(
            resource_state,
            resource_archive,
            &canwu_resource::ResourceTerminalRecordKeyV1::ExternalCompletionParticipant(
                project.completion_certificate.acquisition.clone(),
            ),
        )?;
        match archived.payload {
            canwu_resource::ResourceTerminalArchivePayloadV1::ExternalCompletionParticipant(
                participant,
            ) => participant,
            _ => {
                return Err(invalid(
                    "archived production project resource completion grant body is unavailable",
                ));
            }
        }
    } else {
        return Err(invalid(
            "production project resource completion grant is unavailable at restore",
        ));
    };
    if participant.certificate.as_ref() != Some(&project.completion_certificate)
        || participant.grant.id != project.resource_completion_grant
        || participant.grant.operation_key != project.operation_key
        || participant.grant.state
            != if project.lifecycle == crate::FacilityProjectLifecycle::Completed {
                canwu_resource::CompletionGrantStateV1::Completed
            } else {
                canwu_resource::CompletionGrantStateV1::Consumed
            }
    {
        return Err(invalid(
            "production project resource completion authority differs at restore",
        ));
    }
    for evidence in &project.evidence {
        let record = canwu
            .domain_record_version(&evidence.version)
            .ok_or_else(|| invalid("production project provider evidence is unavailable"))?;
        if evidence.semantic_digest
            != canonical_hash("canwu.production.provider-evidence.v1", &record)?
        {
            return Err(invalid(
                "production project evidence digest differs from its exact restored body",
            ));
        }
    }
    let mut technology_records = Vec::new();
    for reference in [
        Some(&project.technology.technique_revision),
        project.technology.capability_qualification.as_ref(),
        project.technology.implementation.as_ref(),
        project.technology.adoption.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        technology_records.push(
            canwu
                .domain_record_version(reference)
                .ok_or_else(|| invalid("production project technology evidence is unavailable"))?,
        );
    }
    if project.technology.semantic_digest
        != canonical_hash(
            "canwu.production.technology-binding.v1",
            &technology_records,
        )?
    {
        return Err(invalid(
            "production project technology digest differs from its exact restored bodies",
        ));
    }
    for input in &project.inputs {
        validate_resource_input_restore(resource_state, resource_archive, input)?;
    }
    Ok(())
}

fn validate_resource_input_restore(
    resource_state: &canwu_resource::ResourceState,
    resource_archive: Option<&dyn canwu_resource::ResourceArchiveStore>,
    input: &crate::ResourceInputBinding,
) -> Result<(), CanwuError> {
    let leg = resource_state
        .allocation_legs
        .get(&input.allocation_leg.id)
        .ok_or_else(|| invalid("production input allocation leg is unavailable at restore"))?;
    if canwu_resource::ResourceAllocationLegVersionV1::from(leg) != input.allocation_leg
        || leg.status != canwu_resource::AllocationLegStatus::Consumed
    {
        return Err(invalid(
            "production input allocation leg differs from its exact resource binding",
        ));
    }
    if let Some(consumption) = resource_state.consumptions.get(&input.consumption.id) {
        if canwu_resource::ResourceConsumptionVersionV1::from(consumption) != input.consumption {
            return Err(invalid(
                "production input consumption record does not match its exact binding",
            ));
        }
    } else {
        let archived = exact_resource_archive_record(
            resource_state,
            resource_archive,
            &canwu_resource::ResourceTerminalRecordKeyV1::Consumption(input.consumption.id.clone()),
        )?;
        let canwu_resource::ResourceTerminalArchivePayloadV1::Consumption(consumption) =
            &archived.payload
        else {
            return Err(invalid(
                "archived production input has the wrong typed consumption payload",
            ));
        };
        if canwu_resource::ResourceConsumptionVersionV1::from(consumption) != input.consumption
            || archived.operation_key != input.consumption_outcome.operation_key
            || archived.quantity != input.consumption.quantity
            || archived.remainder != 0
            || archived.exact_evidence != vec![input.consumption.consumer_evidence.clone()]
            || archived.semantic_digest != input.consumption.semantic_digest
        {
            return Err(invalid(
                "archived production input consumption differs from its exact binding",
            ));
        }
    }
    validate_resource_outcome_restore(
        resource_state,
        resource_archive,
        &input.consumption_outcome,
        input.quantity,
        &[],
    )
}

pub fn validate_production_resource_continuation(
    resource_state: &canwu_resource::ResourceState,
    resource_archive: &dyn canwu_resource::ResourceArchiveStore,
    input: &crate::ResourceInputBinding,
) -> Result<(), CanwuError> {
    validate_resource_input_restore(resource_state, Some(resource_archive), input)
}

fn validate_resource_outcome_restore(
    resource_state: &canwu_resource::ResourceState,
    resource_archive: Option<&dyn canwu_resource::ResourceArchiveStore>,
    exact: &canwu_resource::ResourceOperationOutcomeVersionV1,
    quantity: u64,
    exact_evidence: &[DomainRecordVersionRef],
) -> Result<(), CanwuError> {
    if let Some(outcome) = resource_state.outcomes.get(&exact.operation_key) {
        outcome.validate().map_err(|error| {
            invalid(format!(
                "resource operation outcome validation failed: {error}"
            ))
        })?;
        if canwu_resource::ResourceOperationOutcomeVersionV1::from(outcome) != *exact
            || outcome.quantity != quantity
            || (!exact_evidence.is_empty() && outcome.exact_evidence != exact_evidence)
        {
            return Err(invalid(
                "production input resource outcome does not match its exact binding",
            ));
        }
        return Ok(());
    }
    let archived = exact_resource_archive_record(
        resource_state,
        resource_archive,
        &canwu_resource::ResourceTerminalRecordKeyV1::Outcome(exact.operation_key.clone()),
    )?;
    let canwu_resource::ResourceTerminalArchivePayloadV1::Outcome(outcome) = &archived.payload
    else {
        return Err(invalid(
            "archived production resource outcome has the wrong typed payload",
        ));
    };
    outcome.validate().map_err(|error| {
        invalid(format!(
            "resource operation outcome validation failed: {error}"
        ))
    })?;
    if canwu_resource::ResourceOperationOutcomeVersionV1::from(outcome) != *exact
        || archived.operation_key != exact.operation_key
        || archived.quantity != quantity
        || archived.remainder != exact.remainder
        || archived.semantic_digest != exact.semantic_digest
        || (!exact_evidence.is_empty() && archived.exact_evidence != exact_evidence)
    {
        return Err(invalid(
            "archived production resource outcome differs from its exact binding",
        ));
    }
    Ok(())
}

fn exact_resource_archive_record(
    resource_state: &canwu_resource::ResourceState,
    resource_archive: Option<&dyn canwu_resource::ResourceArchiveStore>,
    key: &canwu_resource::ResourceTerminalRecordKeyV1,
) -> Result<canwu_resource::ResourceTerminalArchiveRecordV1, CanwuError> {
    let store = resource_archive.ok_or_else(|| {
        invalid(
            "production continuation requires the resource archive provider after evidence compaction",
        )
    })?;
    let mut next = resource_state.archive_head.directory_root.clone();
    let mut remaining = resource_state.archive_head.archived_record_count;
    let mut visited = std::collections::BTreeSet::new();
    let mut found = None;
    while let Some(root) = next {
        if !visited.insert(root.clone()) {
            return Err(invalid("resource archive directory chain is cyclic"));
        }
        let bytes = store
            .load_resource_archive_object(
                canwu_resource::RESOURCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE,
                &root,
            )
            .map_err(|error| resource_archive_error(&error))?
            .ok_or_else(|| invalid("resource archive directory is unavailable"))?;
        let directory: canwu_resource::ResourceArchiveIndexDirectoryV1 =
            serde_json::from_slice(&bytes).map_err(|error| {
                invalid(format!(
                    "resource archive directory could not decode: {error}"
                ))
            })?;
        canwu_resource::authenticate_resource_archive_directory(store, &directory)
            .map_err(|error| resource_archive_error(&error))?;
        if directory.id != root || directory.archived_record_count != remaining {
            return Err(invalid(
                "resource archive directory chain count differs from its authoritative head",
            ));
        }
        let mut batch_count = 0_u64;
        for blob_id in &directory.blob_ids {
            let bytes = store
                .load_resource_archive_object(
                    canwu_resource::RESOURCE_ARCHIVE_BLOB_NAMESPACE,
                    blob_id,
                )
                .map_err(|error| resource_archive_error(&error))?
                .ok_or_else(|| invalid("resource archive blob is unavailable"))?;
            let blob: canwu_resource::ResourceArchiveBlobV1 = serde_json::from_slice(&bytes)
                .map_err(|error| {
                    invalid(format!("resource archive blob could not decode: {error}"))
                })?;
            batch_count = batch_count
                .checked_add(
                    u64::try_from(blob.records.len())
                        .map_err(|_| invalid("resource archive record count overflowed"))?,
                )
                .ok_or_else(|| invalid("resource archive record count overflowed"))?;
            for record in blob.records {
                if &record.key == key && found.replace(record).is_some() {
                    return Err(invalid(
                        "resource archive repeats one exact continuation record",
                    ));
                }
            }
        }
        remaining = remaining
            .checked_sub(batch_count)
            .ok_or_else(|| invalid("resource archive record count underflowed"))?;
        next = directory.previous_root;
    }
    if remaining != 0 {
        return Err(invalid(
            "resource archive directory chain does not close its authoritative count",
        ));
    }
    found.ok_or_else(|| {
        invalid("exact production resource evidence is unavailable in hot or archive storage")
    })
}

fn resource_archive_error(error: &canwu_resource::ResourceError) -> CanwuError {
    invalid(format!("resource archive validation failed: {error}"))
}

fn validate_settled_output_restore(
    canwu: &Canwu,
    resource_state: &canwu_resource::ResourceState,
    resource_archive: Option<&dyn canwu_resource::ResourceArchiveStore>,
    execution: &ProductionExecution,
    request: &crate::ProductionOutputSettlementRequest,
    expected: &canwu_resource::ResourceOperationOutcome,
) -> Result<(), CanwuError> {
    let source = execution
        .output_source
        .as_ref()
        .ok_or_else(|| invalid("settled production output lost its exact production source"))?;
    validate_resource_outcome_restore(
        resource_state,
        resource_archive,
        &canwu_resource::ResourceOperationOutcomeVersionV1::from(expected),
        request.quantity,
        std::slice::from_ref(source),
    )?;
    let source_record = canwu.domain_record_version(source).ok_or_else(|| {
        invalid("settled production output exact source is unavailable at restore")
    })?;
    if !source_record
        .reference
        .kind
        .matches_type::<ProductionRuntimeRecord>()
    {
        return Err(invalid(
            "settled production output source has the wrong record kind",
        ));
    }
    let source_state = source_record.decode_payload::<ProductionRuntimeRecord>()?;
    source_state.validate()?;
    let source_execution = source_state
        .executions
        .get(&execution.id)
        .ok_or_else(|| invalid("settled production output source lost the execution"))?;
    if source_execution.lifecycle != WorkOrderLifecycle::CompletedPendingOutputSettlement
        || !source_execution.output_outcomes.is_empty()
        || source_execution.output_source.is_some()
        || !source_execution
            .output_requests
            .iter()
            .any(|source_request| source_request == request)
        || request.operation_key != expected.operation_key
    {
        return Err(invalid(
            "settled production output source is not the exact pending execution version",
        ));
    }
    Ok(())
}

fn validate_production_knowledge(canwu: &Canwu, state: &ProductionState) -> Result<(), CanwuError> {
    let limits = crate::ProductionLimitsV1::canonical();
    let mut total = 0_usize;
    let mut per_holder = std::collections::BTreeMap::<KnowledgeHolderRef, usize>::new();
    for (holder, records) in &canwu.knowledge().records {
        for record in records
            .values()
            .filter(|record| record.schema.kind.namespace == crate::PLUGIN_NAMESPACE)
        {
            total = total
                .checked_add(1)
                .ok_or_else(|| invalid("production knowledge count overflowed"))?;
            let holder_count = per_holder.entry(holder.clone()).or_default();
            *holder_count = holder_count
                .checked_add(1)
                .ok_or_else(|| invalid("production holder knowledge count overflowed"))?;
            if total > limits.max_observation_records
                || *holder_count > limits.max_observation_records_per_holder
            {
                return Err(invalid("production knowledge exceeds its canonical cap"));
            }
            if record.schema != crate::production_report_knowledge_schema_id()
                || record.holder != *holder
                || record.origin.method != "production_holder_report_v1"
                || record.origin.evidence.is_empty()
                || record.supersedes.len() > 1
                || !record.contradicts.is_empty()
            {
                return Err(invalid("production knowledge metadata is invalid"));
            }
            let report: ProductionObservationReport =
                serde_json::from_value(record.payload.clone()).map_err(|error| {
                    invalid(format!("production report could not be decoded: {error}"))
                })?;
            let grant = state
                .observer_grants
                .values()
                .find(|grant| &grant.holder == holder)
                .filter(|grant| grant.sites.contains(&report.scope))
                .ok_or_else(|| {
                    invalid("production report holder has no matching observation grant")
                })?;
            let site = state
                .sites
                .get(&report.scope)
                .ok_or_else(|| invalid("production report scope is unavailable"))?;
            let [subject] = record.subjects.as_slice() else {
                return Err(invalid("production report must have one site subject"));
            };
            if subject.role != "site"
                || subject.target != KnowledgeSubjectTarget::Entity(site.place.clone())
                || report.holder != *holder
                || report.role != grant.role
                || report.facts.len() + report.blockers.len() > MAX_REPORT_FACTS
                || record.as_of != Some(report.observed_at)
            {
                return Err(invalid(
                    "production report holder, scope, role, or bound is invalid",
                ));
            }
            let key = ProductionObservationHeadKeyV1 {
                holder: holder.clone(),
                scope: report.scope.clone(),
            };
            let storage_key = crate::model::production_observation_head_storage_key(&key)?;
            let observed_head = state
                .observation_heads
                .get(&storage_key)
                .and_then(|heads| {
                    heads.iter().find(|head| {
                        head.observed_at == report.observed_at
                            && head.role == report.role
                            && head.facts == report.facts
                            && head.blockers == report.blockers
                    })
                })
                .ok_or_else(|| {
                    invalid(
                        "production report is not derived from a retained persisted observation cut",
                    )
                })?;
            let source_versions = observed_head
                .source_evidence
                .iter()
                .filter_map(|evidence| match evidence {
                    EvidenceRef::DomainRecordVersion(version) => Some(version),
                    _ => None,
                })
                .collect::<Vec<_>>();
            if source_versions.is_empty() {
                return Err(invalid(
                    "production observation head has no exact provider source version",
                ));
            }
            for version in source_versions {
                let source = canwu.domain_record_version(version).ok_or_else(|| {
                    invalid("production observation provider source is unavailable at restore")
                })?;
                if !source
                    .reference
                    .kind
                    .matches_type::<ProductionRuntimeRecord>()
                {
                    return Err(invalid(
                        "production observation provider source has another record kind",
                    ));
                }
                let source_state = source.decode_payload::<ProductionRuntimeRecord>()?;
                if source_state.revision != observed_head.provider_state_revision {
                    return Err(invalid(
                        "production observation provider source revision differs from its head",
                    ));
                }
            }
            if report.observed_at > report.materialized_at
                || observed_head.materialized_at > report.materialized_at
            {
                return Err(invalid(
                    "production report predates its persisted observation materialization",
                ));
            }
            let mut detached = report;
            let recorded = std::mem::take(&mut detached.canonical_digest);
            if recorded != canonical_hash("canwu.production.holder-report.v1", &detached)? {
                return Err(invalid("production report canonical digest is forged"));
            }
        }
    }
    Ok(())
}

pub fn production_report(
    canwu: &Canwu,
    holder: &KnowledgeHolderRef,
    scope: &crate::ProductionSiteId,
) -> Result<ProductionObservationReport, CanwuError> {
    let record = canwu
        .typed_domain_record(&production_runtime_reference())
        .ok_or_else(|| invalid("production runtime is not configured"))?;
    let state = record.decode_payload::<ProductionRuntimeRecord>()?;
    production_report_from_state(&state, holder, scope, canwu.time())
}

pub fn production_observation_witness(
    canwu: &Canwu,
    holder: &KnowledgeHolderRef,
    scope: &crate::ProductionSiteId,
) -> Result<ProductionObservationWitnessV1, CanwuError> {
    let record = canwu
        .typed_domain_record(&production_runtime_reference())
        .ok_or_else(|| invalid("production runtime is not configured"))?;
    let state = record.decode_payload::<ProductionRuntimeRecord>()?;
    let head = eligible_observation_head(&state, holder, scope, canwu.time())?;
    let report = production_report_from_head(head, canwu.time())?;
    let mut source_versions = head
        .source_evidence
        .iter()
        .filter_map(|evidence| match evidence {
            EvidenceRef::DomainRecordVersion(version) => Some(version.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    source_versions.sort();
    source_versions.dedup();
    if source_versions.is_empty() {
        return Err(invalid(
            "production observation witness requires an exact provider source version",
        ));
    }
    let mut witness = ProductionObservationWitnessV1 {
        provider_plugin: crate::PLUGIN_NAME.to_owned(),
        provider_version: env!("CARGO_PKG_VERSION").to_owned(),
        provider_semantic_hash: "dc6dc9fda679601313939c880d83ae0f5679652691eb7c47a0c1aed5a2249553"
            .to_owned(),
        provider_state_revision: head.provider_state_revision,
        holder: holder.clone(),
        scope: scope.clone(),
        observed_at: report.observed_at,
        materialized_at: report.materialized_at,
        report_digest: report.canonical_digest,
        source_versions,
        adapter_revision: "canwu.production.observation-adapter.v1".to_owned(),
        canonical_digest: String::new(),
    };
    witness.canonical_digest = canonical_hash("canwu.production.observation-witness.v1", &witness)?;
    Ok(witness)
}

pub fn validate_production_observation_witness(
    canwu: &Canwu,
    witness: &ProductionObservationWitnessV1,
) -> Result<(), CanwuError> {
    let mut detached = witness.clone();
    let recorded = std::mem::take(&mut detached.canonical_digest);
    if witness.provider_plugin != crate::PLUGIN_NAME
        || witness.provider_version != env!("CARGO_PKG_VERSION")
        || witness.provider_semantic_hash != crate::PRODUCTION_SEMANTIC_HASH
        || witness.adapter_revision != "canwu.production.observation-adapter.v1"
        || witness.source_versions.is_empty()
        || recorded != canonical_hash("canwu.production.observation-witness.v1", &detached)?
    {
        return Err(invalid("production observation witness is forged"));
    }
    for version in &witness.source_versions {
        let source = canwu.domain_record_version(version).ok_or_else(|| {
            invalid("production observation witness provider source is unavailable")
        })?;
        if !source
            .reference
            .kind
            .matches_type::<ProductionRuntimeRecord>()
        {
            return Err(invalid(
                "production observation witness source has another record kind",
            ));
        }
        let state = source.decode_payload::<ProductionRuntimeRecord>()?;
        if state.revision != witness.provider_state_revision {
            return Err(invalid(
                "production observation witness source revision differs from its provider head",
            ));
        }
    }
    Ok(())
}

/// Builds the required playable degraded-facility decision ticket from the
/// holder's persisted observation cut. Each option contains the exact production
/// command that the Canwu decision engine will persist and execute if selected.
pub fn degraded_facility_decision_ticket(
    canwu: &Canwu,
    ticket_id: DecisionTicketId,
    assigned_controller: impl Into<String>,
    holder: &KnowledgeHolderRef,
    work_order: &crate::WorkOrderId,
    facility: &crate::FacilityAssetId,
    deadline: Option<SimTime>,
) -> Result<DecisionTicketDraft, CanwuError> {
    let record = canwu
        .typed_domain_record(&production_runtime_reference())
        .ok_or_else(|| invalid("production runtime is not configured"))?;
    let state = record.decode_payload::<ProductionRuntimeRecord>()?;
    let order = state
        .work_orders
        .get(work_order)
        .ok_or_else(|| invalid("degraded decision work order is unavailable"))?;
    let asset = state
        .facilities
        .get(facility)
        .ok_or_else(|| invalid("degraded decision facility is unavailable"))?;
    if &order.holder != holder
        || order.site != asset.site
        || !matches!(
            asset.lifecycle,
            crate::FacilityLifecycle::Degraded | crate::FacilityLifecycle::Damaged
        )
    {
        return Err(invalid(
            "degraded decision does not bind the holder's degraded site facility",
        ));
    }
    let head = eligible_observation_head(&state, holder, &order.site, canwu.time())?;
    let context = crate::DegradedFacilityDecisionContextV1 {
        holder: holder.clone(),
        work_order: work_order.clone(),
        facility: facility.clone(),
        facility_generation: asset.generation,
        expected_runtime_revision: state.revision,
        holder_facts_digest: head.canonical_digest.clone(),
    };
    let options = [
        (
            "continue_degraded",
            "Continue with degraded capacity",
            crate::DegradedFacilityChoice::ContinueDegraded,
        ),
        (
            "stop_for_repair",
            "Stop and repair the facility",
            crate::DegradedFacilityChoice::StopForRepair,
        ),
        (
            "defer_order",
            "Defer this work order",
            crate::DegradedFacilityChoice::DeferOrder,
        ),
    ]
    .into_iter()
    .map(|(option_id, label, choice)| {
        let envelope = crate::ProductionCommandEnvelope {
            operation_id: crate::ProductionOperationOutcomeId::new(format!(
                "canwu.production:degraded-choice:{}:{}",
                ticket_id.get(),
                option_id
            ))?,
            holder: holder.clone(),
            expected_runtime_revision: state.revision,
            operation: crate::ProductionOperation::ResolveDegradedFacility {
                work_order: work_order.clone(),
                facility: facility.clone(),
                choice,
                decision_ticket: ticket_id,
            },
        };
        let command = Command::Plugin {
            plugin: crate::PLUGIN_NAME.to_owned(),
            command: crate::PRODUCTION_COMMAND.to_owned(),
            payload: serde_json::to_value(envelope).map_err(|error| {
                invalid(format!(
                    "degraded decision command could not be encoded: {error}"
                ))
            })?,
        };
        Ok(DecisionOption {
            action: DecisionAction::Command {
                command: serde_json::to_value(command).map_err(|error| {
                    invalid(format!(
                        "degraded decision action could not be encoded: {error}"
                    ))
                })?,
            },
            metadata: serde_json::json!({ "holder_facts_digest": head.canonical_digest }),
            ..DecisionOption::new(option_id, label)
        })
    })
    .collect::<Result<Vec<_>, CanwuError>>()?;
    Ok(DecisionTicketDraft {
        id: ticket_id,
        definition: "canwu.production.degraded-facility-choice.v1".to_owned(),
        decision_maker: holder_entity(holder),
        assigned_controller: assigned_controller.into(),
        summary: format!("Choose how work order {work_order} responds to facility {facility}"),
        context: DecisionContext::new(
            "canwu.production.degraded-facility-choice.v1",
            serde_json::to_value(context).map_err(|error| {
                invalid(format!(
                    "degraded decision context could not be encoded: {error}"
                ))
            })?,
        ),
        options,
        deadline,
    })
}

pub fn production_report_from_state(
    state: &ProductionState,
    holder: &KnowledgeHolderRef,
    scope: &crate::ProductionSiteId,
    now: canwu_api::SimTime,
) -> Result<ProductionObservationReport, CanwuError> {
    let head = eligible_observation_head(state, holder, scope, now)?;
    production_report_from_head(head, now)
}

pub(crate) fn eligible_observation_head<'a>(
    state: &'a ProductionState,
    holder: &KnowledgeHolderRef,
    scope: &crate::ProductionSiteId,
    now: SimTime,
) -> Result<&'a ProductionObservationHeadV1, CanwuError> {
    let grant = state
        .observer_grants
        .values()
        .find(|grant| &grant.holder == holder && grant.sites.contains(scope))
        .ok_or_else(|| {
            canwu_api::CanwuError::new(
                canwu_api::ErrorCode::InvalidAuthority,
                "production holder has no observation grant for this site",
            )
        })?;
    let delay = i64::try_from(grant.delay_minutes)
        .map_err(|_| invalid("production observation delay exceeds simulation time"))?;
    let eligible_cut = now
        .checked_add(SimDuration::minutes(-delay))
        .ok_or_else(|| invalid("production observation delay underflowed"))?;
    let key = ProductionObservationHeadKeyV1 {
        holder: holder.clone(),
        scope: scope.clone(),
    };
    let storage_key = crate::model::production_observation_head_storage_key(&key)?;
    let hot = state.observation_heads.get(&storage_key).and_then(|heads| {
        heads
            .iter()
            .rev()
            .find(|head| head.observed_at <= eligible_cut)
    });
    let rolled = state
        .observation_rollover
        .values()
        .filter(|head| head.key == key && head.observed_at <= eligible_cut)
        .max_by_key(|head| (head.observed_at, head.provider_state_revision));
    match (hot, rolled) {
        (Some(left), Some(right)) => Ok(
            if (left.observed_at, left.provider_state_revision)
                >= (right.observed_at, right.provider_state_revision)
            {
                left
            } else {
                right
            },
        ),
        (Some(head), None) | (None, Some(head)) => Ok(head),
        (None, None) => Err(CanwuError::new(
            canwu_api::ErrorCode::DomainRecordNotFound,
            "no persisted production observation cut is yet eligible for this holder",
        )),
    }
}

pub(crate) fn production_report_from_head(
    head: &ProductionObservationHeadV1,
    now: SimTime,
) -> Result<ProductionObservationReport, CanwuError> {
    let id = ProductionObservationId::new(format!(
        "canwu.production:observation:{}:{}:{}:{}",
        holder_key(&head.key.holder),
        head.key.scope.as_str(),
        head.observed_at.as_minutes(),
        head.provider_state_revision,
    ))?;
    let mut report = ProductionObservationReport {
        id,
        holder: head.key.holder.clone(),
        scope: head.key.scope.clone(),
        observed_at: head.observed_at,
        materialized_at: now,
        provider_state_revision: head.provider_state_revision,
        role: head.role,
        facts: head.facts.clone(),
        blockers: head.blockers.clone(),
        canonical_digest: String::new(),
    };
    report.canonical_digest = canonical_hash("canwu.production.holder-report.v1", &report)?;
    Ok(report)
}

pub fn from_production_snapshot_json(
    json: &str,
    plugins: &[&dyn canwu_api::SimulationPlugin],
) -> Result<Canwu, CanwuError> {
    let canwu = Canwu::from_snapshot_json_with_plugins(json, plugins)?;
    validate_production_runtime(&canwu)?;
    Ok(canwu)
}

pub fn from_production_snapshot_json_with_archives(
    json: &str,
    plugins: &[&dyn canwu_api::SimulationPlugin],
    production_archive: Rc<dyn crate::ProductionArchiveStore>,
    resource_archive: Rc<dyn canwu_resource::ResourceArchiveStore>,
) -> Result<Canwu, CanwuError> {
    let mut canwu = Canwu::from_snapshot_json_with_plugins(json, plugins)?;
    validate_production_runtime_with_archives(
        &canwu,
        production_archive.as_ref(),
        resource_archive.as_ref(),
    )?;
    canwu.set_plugin_archive_object_provider(runtime_archive_provider(
        production_archive,
        resource_archive,
    ));
    Ok(canwu)
}

pub fn from_production_checkpoint_journal(
    bundle: canwu_api::CheckpointJournal,
    plugins: &[&dyn canwu_api::SimulationPlugin],
) -> Result<Canwu, CanwuError> {
    let canwu = Canwu::from_checkpoint_journal_with_plugins(bundle, plugins)?;
    validate_production_runtime(&canwu)?;
    Ok(canwu)
}

pub fn from_production_checkpoint_journal_with_archives(
    bundle: canwu_api::CheckpointJournal,
    plugins: &[&dyn canwu_api::SimulationPlugin],
    production_archive: Rc<dyn crate::ProductionArchiveStore>,
    resource_archive: Rc<dyn canwu_resource::ResourceArchiveStore>,
) -> Result<Canwu, CanwuError> {
    let mut canwu = Canwu::from_checkpoint_journal_with_plugins(bundle, plugins)?;
    validate_production_runtime_with_archives(
        &canwu,
        production_archive.as_ref(),
        resource_archive.as_ref(),
    )?;
    canwu.set_plugin_archive_object_provider(runtime_archive_provider(
        production_archive,
        resource_archive,
    ));
    Ok(canwu)
}

pub fn replay_production_from_journal(
    plugins: &[&dyn canwu_api::SimulationPlugin],
    journal: &canwu_api::ReplayJournal,
) -> Result<Canwu, CanwuError> {
    let canwu = Canwu::replay_from_journal(plugins, journal)?;
    validate_production_runtime(&canwu)?;
    Ok(canwu)
}

pub fn replay_production_from_journal_with_archives(
    plugins: &[&dyn canwu_api::SimulationPlugin],
    journal: &canwu_api::ReplayJournal,
    production_archive: Rc<dyn crate::ProductionArchiveStore>,
    resource_archive: Rc<dyn canwu_resource::ResourceArchiveStore>,
) -> Result<Canwu, CanwuError> {
    let production_for_validation = Rc::clone(&production_archive);
    let resource_for_validation = Rc::clone(&resource_archive);
    let archive_provider = runtime_archive_provider(production_archive, resource_archive);
    let canwu =
        Canwu::replay_from_journal_with_archive_provider(plugins, journal, archive_provider)?;
    validate_production_runtime_with_archives(
        &canwu,
        production_for_validation.as_ref(),
        resource_for_validation.as_ref(),
    )?;
    Ok(canwu)
}

fn holder_key(holder: &KnowledgeHolderRef) -> String {
    match holder {
        KnowledgeHolderRef::Person(person) => format!("person-{}", person.get()),
        KnowledgeHolderRef::Entity(entity) => format!("entity-{entity:?}"),
    }
    .replace([' ', '{', '}', '(', ')', ','], "-")
}

fn holder_entity(holder: &KnowledgeHolderRef) -> EntityRef {
    match holder {
        KnowledgeHolderRef::Person(person) => EntityRef::Person(*person),
        KnowledgeHolderRef::Entity(entity) => entity.clone(),
    }
}
