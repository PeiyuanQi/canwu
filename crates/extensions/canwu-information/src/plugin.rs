use crate::lifecycle::{
    GenericInformationPublicationDraft, InformationLifecycle, LifecycleRequest,
    validate_delegation_claim, validate_delegation_grant,
};
use crate::model::{
    DelegationAuthorityGrant, DelegationClaimV1, DelegationEvidenceSelector, DeliveryAttemptStatus,
    InformationLimitsV1, InterpretationAuthority, ReleaseStatus,
};
use crate::operation::{
    InformationAdmissionRef, InformationContinuation, InformationOperationEnvelope,
    InformationOperationId, InformationOperationPayload, InformationOperationStatus,
    InformationRetryDisposition, classify_operation_retry, derive_operation_record_ref,
    validate_operation_envelope, validate_operation_transition,
};
use crate::query::InformationRecordSet;
use crate::schema::{
    Access, Audience, AuthorityAssignment, Content, DeliveryAttempt, Dispatch,
    InformationOperationRecord, Interpretation, Release, Representation,
    information_knowledge_schemas, information_record_schemas,
};
use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundarySystemContract,
    CanwuError, CauseRef, Command, CommandIngress, DomainRecord, DomainRecordDraft,
    DomainRecordKind, DomainRecordMutation, DomainRecordType, DomainRecordVersionRef,
    DomainReference, DomainReferenceTarget, EntityRef, ErrorCode, EvidenceRef, IngressClass,
    IngressPayload, KnowledgeHolderRef, KnowledgeOrigin, KnowledgeRecordDraft, KnowledgeRecordKind,
    KnowledgeSchemaId, KnowledgeSubject, KnowledgeSubjectTarget, KnowledgeWriteGrant,
    PayloadProperty, PayloadSchema, PayloadValueType, PluginActionDescriptor,
    PluginIngressDescriptor, PluginRegistrar, SimDuration, SimulationPlugin, SimulationView,
    StateKey, StateVisibility, SystemCadence, SystemDirective, TypedDomainRecordRef,
    canonical_hash,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const PLUGIN_NAME: &str = "canwu-information";
pub const PLUGIN_NAMESPACE: &str = "canwu.information";
pub const INFORMATION_COMMAND: &str = "apply_information_operation_v1";
pub const INFORMATION_INGRESS: &str = "information_operation_v1";
pub const INFORMATION_FINALIZATION_INGRESS: &str = "information_operation_finalize_v1";
pub const DELEGATED_AUTHORITY_GRANT: &str = "interpret_for_holder";
pub const INSTITUTIONAL_AUTHORITY_GRANT: &str = "interpret_as_assigned_role";
pub const AUTHORITY_COMMAND_PRODUCER: &str = "canwu-authority";
pub const AUTHORITY_COMMAND_TYPE: &str = "delegate_interpretation_v1";

const PLUGIN_VERSION: &str = "0.2.0-experimental";
const SEMANTIC_HASH: &str = "8b20a4c41417220c920b8d0312a6011c4cf7ec98566bad61e82bbc5fada30bc8";
const AUTHORITY_GRANTS_HASH: &str =
    "1f956d1dbee04d6cf7a076f778bb058e47a0a6155cd778c4163f78ccbbfe4b5c";
const INPUT_HASH_DOMAIN: &str = "canwu.information.operation-input.v1";
const PHASE7_SYSTEM: &str = "information_lifecycle_v1";
const PHASE13_SYSTEM: &str = "information_publication_v1";
const MAX_RECORD_QUERY: usize = 10_000;
const MAX_PUBLICATION_BATCHES: usize = 64;
const ORIGIN_METHOD: &str = "information_lifecycle_record_v1";
const AUTHORITY_CLAIM_HASH_DOMAIN: &str = "canwu.information.delegation-claim.v1";
const AUTHORITY_GRANTS_HASH_DOMAIN: &str = "canwu.information.authority-grants.v1";

#[derive(Clone, Copy, Debug, Default)]
pub struct InformationPlugin;

#[must_use]
pub fn information_authority_grants() -> Vec<DelegationAuthorityGrant> {
    vec![
        DelegationAuthorityGrant {
            code: INSTITUTIONAL_AUTHORITY_GRANT.to_owned(),
            selector: DelegationEvidenceSelector::DomainRecord {
                owner_plugin: PLUGIN_NAME.to_owned(),
                kind: DomainRecordKind::for_type::<AuthorityAssignment>(),
            },
            claim_path: vec!["claim".to_owned()],
        },
        DelegationAuthorityGrant {
            code: DELEGATED_AUTHORITY_GRANT.to_owned(),
            selector: DelegationEvidenceSelector::Command {
                producer_plugin: AUTHORITY_COMMAND_PRODUCER.to_owned(),
                command_type: AUTHORITY_COMMAND_TYPE.to_owned(),
            },
            claim_path: vec!["claim".to_owned()],
        },
    ]
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct FinalizationRequest {
    id: InformationOperationId,
    canonical_input_hash: String,
}

impl InformationPlugin {
    #[must_use]
    pub const fn limits(&self) -> InformationLimitsV1 {
        InformationLimitsV1::canonical()
    }

    #[must_use]
    pub fn command_descriptor(&self) -> PluginActionDescriptor {
        PluginActionDescriptor {
            name: INFORMATION_COMMAND.to_owned(),
            description: "Submit one versioned neutral information lifecycle operation".to_owned(),
            payload_schema: operation_payload_schema(),
            reads: vec![record_state::<InformationOperationRecord>()],
            writes: Vec::new(),
        }
    }

    #[must_use]
    pub fn ingress_descriptor(&self) -> PluginIngressDescriptor {
        PluginIngressDescriptor {
            name: INFORMATION_INGRESS.to_owned(),
            description: "Admit one resolved neutral information lifecycle operation".to_owned(),
            class: IngressClass::Information,
            payload_schema: operation_payload_schema(),
        }
    }

    #[must_use]
    pub const fn command_handler(&self) -> canwu_api::PluginCommandHandler {
        apply_operation_command
    }

    pub fn validate_transport_payload(&self, payload: &Value) -> Result<(), CanwuError> {
        decode_envelope(payload).map(|_| ())
    }
}

impl SimulationPlugin for InformationPlugin {
    fn name(&self) -> &'static str {
        PLUGIN_NAME
    }
    fn version(&self) -> &'static str {
        PLUGIN_VERSION
    }
    fn semantic_hash(&self) -> &'static str {
        SEMANTIC_HASH
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let grants = information_authority_grants();
        for grant in &grants {
            validate_delegation_grant(grant).map_err(invalid_record)?;
        }
        if grants.windows(2).any(|pair| pair[0].code >= pair[1].code) {
            return Err(invalid_record(
                "information authority grants must be sorted and unique",
            ));
        }
        let grants_hash = canonical_hash(AUTHORITY_GRANTS_HASH_DOMAIN, &grants)?;
        if grants_hash != AUTHORITY_GRANTS_HASH {
            return Err(invalid_record(
                "information authority grants do not match the semantic descriptor identity",
            ));
        }
        let record_schemas = information_record_schemas();
        for schema in &record_schemas {
            registrar.register_record_schema(schema.clone())?;
        }
        let knowledge_schemas = information_knowledge_schemas();
        for schema in &knowledge_schemas {
            registrar.register_knowledge_schema(schema.clone())?;
        }
        registrar.register_command(self.command_descriptor(), apply_operation_command)?;
        registrar.register_ingress(self.ingress_descriptor())?;
        registrar.register_ingress(PluginIngressDescriptor {
            name: INFORMATION_FINALIZATION_INGRESS.to_owned(),
            description: "Finalize persisted neutral information publications".to_owned(),
            class: IngressClass::Acknowledgement,
            payload_schema: finalization_payload_schema(),
        })?;

        let mut phase7 = BoundarySystemContract::new(
            PHASE7_SYSTEM,
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        phase7.reads = record_schemas
            .iter()
            .map(canwu_api::DomainRecordSchema::state_key)
            .collect();
        phase7.reads.extend([
            StateKey::core_commands(),
            StateKey::core_ingress(),
            StateKey::core_knowledge(),
        ]);
        phase7.reads.sort();
        phase7.reads.dedup();
        phase7.writes = record_schemas
            .iter()
            .map(canwu_api::DomainRecordSchema::state_key)
            .collect();
        phase7.writes.sort();
        phase7.writes.dedup();
        phase7.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(phase7, apply_lifecycle_boundary)?;

        let mut phase13 = BoundarySystemContract::new(
            PHASE13_SYSTEM,
            BoundaryPhase::PerspectiveAndReportMaterialization,
            SystemCadence::EventDriven,
        );
        phase13.reads = record_schemas
            .iter()
            .map(canwu_api::DomainRecordSchema::state_key)
            .collect();
        phase13.reads.sort();
        phase13.reads.dedup();
        phase13.writes = vec![record_state::<InformationOperationRecord>()];
        phase13.knowledge_writes = knowledge_schemas
            .into_iter()
            .map(|schema| KnowledgeWriteGrant {
                schema: schema.id,
                visibilities: vec![StateVisibility::SameBoundary],
            })
            .collect();
        phase13.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(phase13, publish_lifecycle_boundary)
    }
}

fn apply_operation_command(
    view: &SimulationView<'_>,
    context: &canwu_api::CommandContext,
    payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    if context.ingress == CommandIngress::LegacyDirect {
        return Err(CanwuError::new(
            ErrorCode::MixedCommandIngress,
            "information operations require tracked command ingress",
        ));
    }
    let envelope = decode_envelope(payload)?;
    let input_hash = canonical_hash(INPUT_HASH_DOMAIN, &envelope)?;
    let reference = derive_operation_record_ref(&envelope.id);
    let existing = view.typed_domain_record(&reference)?;
    let existing_payload = existing
        .map(canwu_api::DomainRecord::decode_payload::<InformationOperationRecord>)
        .transpose()?;
    match classify_operation_retry(existing_payload.as_ref(), &envelope.id, &input_hash) {
        Ok(InformationRetryDisposition::ExactRetry) => Ok(Vec::new()),
        Ok(InformationRetryDisposition::New) => Ok(vec![SystemDirective::EnqueuePluginIngress {
            after: SimDuration::ZERO,
            packet_type: INFORMATION_INGRESS.to_owned(),
            priority: 0,
            payload: serde_json::to_value(envelope).map_err(|error| encode_error(&error))?,
            affected: Vec::new(),
        }]),
        Err(message) => Err(CanwuError::new(ErrorCode::IdempotencyConflict, message)),
    }
}

#[allow(clippy::single_match_else)]
fn apply_lifecycle_boundary(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let records = load_information_records(view)?;
    let mut operations = BTreeMap::<
        InformationOperationId,
        (
            String,
            InformationOperationEnvelope,
            InformationAdmissionRef,
        ),
    >::new();
    let mut finalizations = BTreeMap::<InformationOperationId, FinalizationRequest>::new();
    for ingress_id in &context.admitted_ingress {
        let Some(ingress) = view.ingress(*ingress_id)? else {
            continue;
        };
        let IngressPayload::Plugin {
            plugin,
            packet_type,
            payload,
            ..
        } = &ingress.payload
        else {
            continue;
        };
        if plugin != PLUGIN_NAME {
            continue;
        }
        if packet_type == INFORMATION_INGRESS {
            let envelope = decode_envelope(payload)?;
            let hash = canonical_hash(INPUT_HASH_DOMAIN, &envelope)?;
            let admission = match &ingress.cause {
                Some(CauseRef::Command(command)) => InformationAdmissionRef::Command(*command),
                _ => InformationAdmissionRef::Ingress(*ingress_id),
            };
            if let Some((prior_hash, _, _)) = operations.get(&envelope.id) {
                if prior_hash != &hash {
                    return Err(CanwuError::new(
                        ErrorCode::IdempotencyConflict,
                        "one boundary admitted conflicting information operation inputs",
                    ));
                }
            } else {
                operations.insert(envelope.id.clone(), (hash, envelope, admission));
            }
        } else if packet_type == INFORMATION_FINALIZATION_INGRESS {
            let request: FinalizationRequest =
                serde_json::from_value(payload.clone()).map_err(|error| {
                    CanwuError::new(
                        ErrorCode::InvalidPayload,
                        format!("information finalization payload could not be decoded: {error}"),
                    )
                })?;
            if let Some(prior) = finalizations.get(&request.id) {
                if prior != &request {
                    return Err(CanwuError::new(
                        ErrorCode::IdempotencyConflict,
                        "one boundary admitted conflicting information finalizations",
                    ));
                }
            } else {
                finalizations.insert(request.id.clone(), request);
            }
        }
    }

    let mut directives = Vec::new();
    for request in finalizations.values() {
        finalize_operation(view, context, request, &mut directives)?;
    }
    for (hash, envelope, admission) in operations.values() {
        let operation_ref = derive_operation_record_ref(&envelope.id);
        let existing = view.typed_domain_record(&operation_ref)?;
        let Some(existing) = existing else {
            let operation = InformationOperationPayload {
                id: envelope.id.clone(),
                operation_version: envelope.operation_version,
                operation_kind: envelope.operation_kind.clone(),
                canonical_input_hash: hash.clone(),
                output_slots: envelope.output_slots.clone(),
                status: InformationOperationStatus::Accepted,
                admitted_at: context.at,
                accepted_cause: *admission,
                authority_claim_hash: None,
                domain_result_refs: Vec::new(),
                domain_result_evidence: Vec::new(),
                publication_result_ids: Vec::new(),
                continuation: None,
                completed_at: None,
                rejection_code: None,
            };
            directives.push(mutate(
                operation_create(&operation, &[])?,
                "Accept neutral information operation",
            ));
            directives.push(schedule_operation_progress(envelope)?);
            continue;
        };
        let previous = existing.decode_payload::<InformationOperationRecord>()?;
        match classify_operation_retry(Some(&previous), &envelope.id, hash) {
            Ok(InformationRetryDisposition::ExactRetry) => {}
            Ok(InformationRetryDisposition::New) => {
                unreachable!("existing operation cannot classify as new")
            }
            Err(message) => {
                return Err(CanwuError::new(ErrorCode::IdempotencyConflict, message));
            }
        }
        if previous.status.is_terminal()
            || matches!(
                previous.status,
                InformationOperationStatus::AwaitingPublication
                    | InformationOperationStatus::AwaitingFinalization
            )
        {
            continue;
        }
        if previous.status == InformationOperationStatus::Accepted {
            let mut proposed = previous.clone();
            proposed.status = InformationOperationStatus::ApplyingDomainChanges;
            validate_operation_transition(&previous, &proposed).map_err(invalid_record)?;
            directives.push(mutate(
                operation_update(existing, &proposed)?,
                "Begin neutral information domain changes",
            ));
            directives.push(schedule_operation_progress(envelope)?);
            continue;
        }
        if previous.status != InformationOperationStatus::ApplyingDomainChanges {
            return Err(invalid_record(
                "information operation ingress found an invalid progress state",
            ));
        }
        let Ok(authority_claim_hash) = validate_runtime_interpretation_authority(view, envelope)
        else {
            let mut proposed = previous.clone();
            proposed.status = InformationOperationStatus::Rejected;
            proposed.rejection_code = Some("invalid_authority".to_owned());
            validate_operation_transition(&previous, &proposed).map_err(invalid_record)?;
            directives.push(mutate(
                operation_update(existing, &proposed)?,
                "Persist rejected neutral information authority",
            ));
            continue;
        };
        match InformationLifecycle::plan(
            &records,
            &envelope.operation.request,
            InformationLimitsV1::canonical(),
        ) {
            Ok(plan) => {
                let mut result_refs: Vec<_> = plan
                    .mutations
                    .iter()
                    .map(|mutation| mutation.target().clone())
                    .collect();
                result_refs.sort();
                result_refs.dedup();
                let publication_count = u64::try_from(plan.publications.len()).map_err(|_| {
                    invalid_record("information publication count is not representable")
                })?;
                let chunk_size = u32::try_from(
                    publication_count.min(MAX_PUBLICATION_BATCHES as u64),
                )
                .map_err(|_| {
                    invalid_record("information publication chunk size is not representable")
                })?;
                let status = if publication_count == 0 && !result_refs.is_empty() {
                    InformationOperationStatus::AwaitingFinalization
                } else if publication_count == 0 {
                    InformationOperationStatus::Completed
                } else {
                    InformationOperationStatus::AwaitingPublication
                };
                let mut proposed = previous.clone();
                proposed.status = status;
                proposed.authority_claim_hash = authority_claim_hash;
                proposed.domain_result_refs.clone_from(&result_refs);
                proposed.continuation =
                    (publication_count > 0).then_some(InformationContinuation {
                        cursor: 0,
                        remaining: publication_count,
                        chunk_size,
                    });
                proposed.completed_at =
                    (publication_count == 0 && result_refs.is_empty()).then_some(context.at);
                validate_operation_transition(&previous, &proposed).map_err(invalid_record)?;
                for mutation in plan.mutations {
                    directives.push(mutate(
                        mutation,
                        "Apply neutral information lifecycle mutation",
                    ));
                }
                directives.push(mutate(
                    operation_update(existing, &proposed)?,
                    "Commit neutral information operation results",
                ));
            }
            Err(_) => {
                let mut proposed = previous.clone();
                proposed.status = InformationOperationStatus::Rejected;
                proposed.rejection_code = Some("invalid_lifecycle".to_owned());
                validate_operation_transition(&previous, &proposed).map_err(invalid_record)?;
                directives.push(mutate(
                    operation_update(existing, &proposed)?,
                    "Persist rejected neutral information operation",
                ));
            }
        }
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

fn schedule_operation_progress(
    envelope: &InformationOperationEnvelope,
) -> Result<BoundaryDirective, CanwuError> {
    Ok(BoundaryDirective::ScheduleIngress {
        after: SimDuration::ZERO,
        packet_type: INFORMATION_INGRESS.to_owned(),
        priority: 0,
        payload: serde_json::to_value(envelope).map_err(|error| encode_error(&error))?,
        affected: Vec::new(),
    })
}

fn finalize_operation(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    request: &FinalizationRequest,
    directives: &mut Vec<BoundaryDirective>,
) -> Result<(), CanwuError> {
    let reference = derive_operation_record_ref(&request.id);
    let record = view
        .typed_domain_record(&reference)?
        .ok_or_else(|| invalid_record("information finalization names a missing operation"))?;
    let previous = record.decode_payload::<InformationOperationRecord>()?;
    classify_operation_retry(Some(&previous), &request.id, &request.canonical_input_hash)
        .map_err(|message| CanwuError::new(ErrorCode::IdempotencyConflict, message))?;
    if previous.status == InformationOperationStatus::Completed {
        return Ok(());
    }
    if previous.status != InformationOperationStatus::AwaitingFinalization {
        return Err(invalid_record(
            "information finalization requires awaiting-finalization state",
        ));
    }
    let correlation_prefix = publication_correlation_prefix(&previous);
    let changes = view.knowledge_changes_by_correlation_prefix(PLUGIN_NAME, &correlation_prefix)?;
    if changes.is_empty() {
        return Err(invalid_record(
            "information finalization found no committed publication batch",
        ));
    }
    let mut proposed = previous.clone();
    proposed.publication_result_ids.extend(
        changes
            .into_iter()
            .flat_map(|change| change.records.into_iter().map(|record| record.id)),
    );
    proposed.publication_result_ids.sort();
    proposed.publication_result_ids.dedup();
    if proposed.continuation.is_some() {
        proposed.status = InformationOperationStatus::AwaitingPublication;
    } else {
        proposed.status = InformationOperationStatus::Completed;
        proposed.completed_at = Some(context.at);
    }
    validate_operation_transition(&previous, &proposed).map_err(invalid_record)?;
    directives.push(mutate(
        operation_update(record, &proposed)?,
        "Finalize neutral information publication",
    ));
    Ok(())
}

fn publish_lifecycle_boundary(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let kind = DomainRecordKind::for_type::<InformationOperationRecord>();
    let operations = all_domain_records_of_kind(view, &kind)?;
    let mut directives = Vec::new();
    for record in operations {
        let previous = record.decode_payload::<InformationOperationRecord>()?;
        if previous.status == InformationOperationStatus::AwaitingFinalization
            && previous.continuation.is_none()
            && previous.domain_result_evidence.is_empty()
        {
            let result_evidence = resolve_result_evidence(view, &previous)?;
            let mut proposed = previous.clone();
            proposed.status = InformationOperationStatus::Completed;
            proposed.domain_result_evidence = result_evidence.values().cloned().collect();
            proposed.completed_at = Some(context.at);
            validate_operation_transition(&previous, &proposed).map_err(invalid_record)?;
            directives.push(mutate(
                operation_update(&record, &proposed)?,
                "Finalize neutral information domain-result evidence",
            ));
            continue;
        }
        if previous.status != InformationOperationStatus::AwaitingPublication {
            continue;
        }
        if view
            .proposed_domain_record_version(&record.reference)?
            .is_none()
        {
            continue;
        }
        let mut publications = reconstruct_publications(view, &previous)?;
        publications.sort();
        let continuation = previous
            .continuation
            .as_ref()
            .ok_or_else(|| invalid_record("awaiting-publication operation lacks continuation"))?;
        let total = u64::try_from(publications.len())
            .map_err(|_| invalid_record("information publication count is not representable"))?;
        if continuation.cursor.checked_add(continuation.remaining) != Some(total) {
            return Err(invalid_record(
                "information publication continuation does not match closed publication set",
            ));
        }
        let start = usize::try_from(continuation.cursor)
            .map_err(|_| invalid_record("information publication cursor is not representable"))?;
        let take = usize::try_from(u64::from(continuation.chunk_size).min(continuation.remaining))
            .map_err(|_| invalid_record("information publication chunk is not representable"))?;
        let end = start
            .checked_add(take)
            .ok_or_else(|| invalid_record("information publication chunk overflow"))?;
        let slice = publications.get(start..end).ok_or_else(|| {
            invalid_record("information publication continuation is out of bounds")
        })?;
        let operation_evidence = view
            .proposed_domain_record_version(&record.reference)?
            .ok_or_else(|| {
                invalid_record("information operation proposal evidence is unavailable")
            })?;
        let result_evidence = resolve_result_evidence(view, &previous)?;
        let mut batches = BTreeMap::<KnowledgeHolderRef, Vec<KnowledgeRecordDraft>>::new();
        for publication in slice {
            let (holder, draft) =
                knowledge_draft(view, publication, &result_evidence, &operation_evidence)?;
            batches.entry(holder).or_default().push(draft);
        }
        if batches.len() > MAX_PUBLICATION_BATCHES {
            return Err(invalid_record(
                "information publication chunk exceeds holder-batch limit",
            ));
        }
        let correlation_prefix = publication_correlation_prefix(&previous);
        for (batch_index, (holder, records)) in batches.into_iter().enumerate() {
            let batch_index = u64::try_from(batch_index)
                .map_err(|_| invalid_record("information batch index is not representable"))?;
            let absolute_batch_index = continuation
                .cursor
                .checked_add(batch_index)
                .ok_or_else(|| invalid_record("information batch index overflow"))?;
            let correlation = format!("{correlation_prefix}{absolute_batch_index:020}");
            directives.push(BoundaryDirective::PublishKnowledge {
                holder,
                visibility: StateVisibility::SameBoundary,
                producer_correlation: Some(correlation.clone()),
                records,
                summary: "Publish neutral information lifecycle record".to_owned(),
            });
        }
        let remaining = total
            - u64::try_from(end)
                .map_err(|_| invalid_record("information publication end is not representable"))?;
        let next_chunk_size = u32::try_from(remaining.min(MAX_PUBLICATION_BATCHES as u64))
            .map_err(|_| {
                invalid_record("information publication chunk size is not representable")
            })?;
        let mut proposed = previous.clone();
        proposed.status = InformationOperationStatus::AwaitingFinalization;
        proposed.domain_result_evidence = result_evidence.values().cloned().collect();
        proposed.continuation = (remaining > 0).then_some(InformationContinuation {
            cursor: end as u64,
            remaining,
            chunk_size: next_chunk_size,
        });
        validate_operation_transition(&previous, &proposed).map_err(invalid_record)?;
        directives.push(mutate(
            operation_update(&record, &proposed)?,
            "Record neutral information publication proposal",
        ));
        directives.push(BoundaryDirective::ScheduleIngress {
            after: SimDuration::ZERO,
            packet_type: INFORMATION_FINALIZATION_INGRESS.to_owned(),
            priority: 0,
            payload: serde_json::to_value(FinalizationRequest {
                id: previous.id.clone(),
                canonical_input_hash: previous.canonical_input_hash.clone(),
            })
            .map_err(|error| encode_error(&error))?,
            affected: Vec::new(),
        });
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

fn publication_correlation_prefix(operation: &InformationOperationPayload) -> String {
    format!("information-operation:{}:", operation.canonical_input_hash)
}

fn reconstruct_publications(
    view: &SimulationView<'_>,
    operation: &InformationOperationPayload,
) -> Result<Vec<GenericInformationPublicationDraft>, CanwuError> {
    let result = single_result(operation)?;
    match operation.operation_kind.as_str() {
        "transition_delivery_attempt" => {
            let attempt_ref = typed::<DeliveryAttempt>(result.clone())?;
            let attempt = required_record(view, attempt_ref.as_untyped())?;
            let payload = attempt.decode_payload::<DeliveryAttempt>()?;
            if payload.status != DeliveryAttemptStatus::Delivered {
                return Ok(Vec::new());
            }
            let holder = single_holder(attempt, "recipient")?;
            let dispatch_ref = single_typed::<Dispatch>(attempt, "dispatch")?;
            let dispatch = required_record(view, dispatch_ref.as_untyped())?;
            let representation = single_typed::<Representation>(dispatch, "representation")?;
            let representation_record = required_record(view, representation.as_untyped())?;
            Ok(vec![
                GenericInformationPublicationDraft::RepresentationAvailable {
                    holder,
                    representation,
                    delivery_attempt: attempt_ref,
                    record_version: representation_record.version,
                },
            ])
        }
        "record_access" => {
            let access = typed::<Access>(result.clone())?;
            let record = required_record(view, access.as_untyped())?;
            Ok(vec![GenericInformationPublicationDraft::AccessRecorded {
                holder: single_holder(record, "holder")?,
                access,
                record_version: record.version,
            }])
        }
        "record_interpretation" => {
            let interpretation = typed::<Interpretation>(result.clone())?;
            let record = required_record(view, interpretation.as_untyped())?;
            Ok(vec![
                GenericInformationPublicationDraft::InterpretationRecorded {
                    holder: single_holder(record, "performed_for")?,
                    interpretation,
                    record_version: record.version,
                },
            ])
        }
        "transition_release" => {
            let release = typed::<Release>(result.clone())?;
            let record = required_record(view, release.as_untyped())?;
            if record.decode_payload::<Release>()?.status != ReleaseStatus::Active {
                return Ok(Vec::new());
            }
            let audience = single_typed::<Audience>(record, "audience")?;
            let audience_record = required_record(view, audience.as_untyped())?;
            let mut holders = role_targets(audience_record, "member")
                .into_iter()
                .map(holder_from_target)
                .collect::<Vec<_>>();
            holders.sort();
            holders.dedup();
            Ok(holders
                .into_iter()
                .map(
                    |holder| GenericInformationPublicationDraft::ReleaseAvailable {
                        holder,
                        release: release.clone(),
                        record_version: record.version,
                    },
                )
                .collect())
        }
        _ => Ok(Vec::new()),
    }
}

fn knowledge_draft(
    view: &SimulationView<'_>,
    publication: &GenericInformationPublicationDraft,
    result_evidence: &BTreeMap<canwu_api::DomainRecordRef, DomainRecordVersionRef>,
    operation_evidence: &DomainRecordVersionRef,
) -> Result<(KnowledgeHolderRef, KnowledgeRecordDraft), CanwuError> {
    let (holder, schema_name, subjects, primary, record_version) = match publication {
        GenericInformationPublicationDraft::RepresentationAvailable {
            holder,
            representation,
            delivery_attempt,
            record_version,
        } => {
            let representation_record = required_record(view, representation.as_untyped())?;
            let content = single_typed::<Content>(representation_record, "content")?;
            (
                holder.clone(),
                "representation_available",
                vec![
                    subject("representation", representation.as_untyped()),
                    subject("content", content.as_untyped()),
                ],
                delivery_attempt.as_untyped(),
                *record_version,
            )
        }
        GenericInformationPublicationDraft::AccessRecorded {
            holder,
            access,
            record_version,
        } => {
            let access_record = required_record(view, access.as_untyped())?;
            let representation = single_typed::<Representation>(access_record, "representation")?;
            (
                holder.clone(),
                "access_recorded",
                vec![
                    subject("access", access.as_untyped()),
                    subject("representation", representation.as_untyped()),
                ],
                access.as_untyped(),
                *record_version,
            )
        }
        GenericInformationPublicationDraft::InterpretationRecorded {
            holder,
            interpretation,
            record_version,
        } => (
            holder.clone(),
            "interpretation_recorded",
            vec![subject("interpretation", interpretation.as_untyped())],
            interpretation.as_untyped(),
            *record_version,
        ),
        GenericInformationPublicationDraft::ReleaseAvailable {
            holder,
            release,
            record_version,
        } => {
            let release_record = required_record(view, release.as_untyped())?;
            let representation = single_typed::<Representation>(release_record, "representation")?;
            (
                holder.clone(),
                "release_available",
                vec![
                    subject("release", release.as_untyped()),
                    subject("representation", representation.as_untyped()),
                ],
                release.as_untyped(),
                *record_version,
            )
        }
    };
    let mut subjects = subjects;
    subjects.sort();
    subjects.dedup();
    let result = result_evidence.get(primary).ok_or_else(|| {
        invalid_record("information publication lacks exact domain-result evidence")
    })?;
    let mut evidence = vec![
        EvidenceRef::DomainRecordVersion(result.clone()),
        EvidenceRef::DomainRecordVersion(operation_evidence.clone()),
    ];
    evidence.sort();
    evidence.dedup();
    Ok((
        holder,
        KnowledgeRecordDraft {
            schema: knowledge_schema(schema_name),
            subjects,
            payload: json!({"record_version": record_version}),
            as_of: None,
            confidence_per_mille: 1_000,
            origin: KnowledgeOrigin {
                method: ORIGIN_METHOD.to_owned(),
                evidence,
            },
            supersedes: Vec::new(),
            contradicts: Vec::new(),
        },
    ))
}

fn resolve_result_evidence(
    view: &SimulationView<'_>,
    operation: &InformationOperationPayload,
) -> Result<BTreeMap<canwu_api::DomainRecordRef, DomainRecordVersionRef>, CanwuError> {
    if !operation.domain_result_evidence.is_empty() {
        let map: BTreeMap<_, _> = operation
            .domain_result_evidence
            .iter()
            .cloned()
            .map(|evidence| (evidence.record.clone(), evidence))
            .collect();
        let expected: BTreeSet<_> = operation.domain_result_refs.iter().cloned().collect();
        if map.keys().cloned().collect::<BTreeSet<_>>() != expected {
            return Err(invalid_record(
                "stored domain-result evidence does not match operation results",
            ));
        }
        return Ok(map);
    }
    operation
        .domain_result_refs
        .iter()
        .map(|reference| {
            view.proposed_domain_record_version(reference)?
                .map(|evidence| (reference.clone(), evidence))
                .ok_or_else(|| {
                    invalid_record("information domain-result proposal evidence is unavailable")
                })
        })
        .collect()
}

fn load_information_records(view: &SimulationView<'_>) -> Result<InformationRecordSet, CanwuError> {
    let mut records = Vec::new();
    for schema in information_record_schemas() {
        if schema.kind.matches_type::<InformationOperationRecord>() {
            continue;
        }
        records.extend(all_domain_records_of_kind(view, &schema.kind)?);
    }
    InformationRecordSet::from_records(records).map_err(invalid_record)
}

fn all_domain_records_of_kind(
    view: &SimulationView<'_>,
    kind: &DomainRecordKind,
) -> Result<Vec<DomainRecord>, CanwuError> {
    let mut records = Vec::new();
    let mut cursor = None;
    loop {
        let page = view.domain_records_of_kind_after(kind, cursor.as_ref(), MAX_RECORD_QUERY)?;
        let page_is_complete = page.len() < MAX_RECORD_QUERY;
        cursor = page.last().map(|record| record.reference.clone());
        records.extend(page);
        if page_is_complete {
            return Ok(records);
        }
    }
}

fn operation_create(
    payload: &InformationOperationPayload,
    results: &[canwu_api::DomainRecordRef],
) -> Result<DomainRecordMutation, CanwuError> {
    let mut draft =
        DomainRecordDraft::from_typed(derive_operation_record_ref(&payload.id), payload)?;
    draft.references = results
        .iter()
        .cloned()
        .map(|target| DomainReference {
            role: "domain_result".to_owned(),
            target: DomainReferenceTarget::Domain(target),
        })
        .collect();
    Ok(DomainRecordMutation::Create { record: draft })
}

fn operation_update(
    previous: &DomainRecord,
    payload: &InformationOperationPayload,
) -> Result<DomainRecordMutation, CanwuError> {
    let reference = typed::<InformationOperationRecord>(previous.reference.clone())?;
    let mut draft = DomainRecordDraft::from_typed(reference, payload)?;
    draft.references.clone_from(&previous.references);
    Ok(DomainRecordMutation::Update {
        record: draft,
        expected_version: previous.version,
    })
}

fn mutate(mutation: DomainRecordMutation, summary: &str) -> BoundaryDirective {
    BoundaryDirective::MutateRecord {
        mutation,
        summary: summary.to_owned(),
    }
}

fn single_result(
    operation: &InformationOperationPayload,
) -> Result<&canwu_api::DomainRecordRef, CanwuError> {
    match operation.domain_result_refs.as_slice() {
        [result] => Ok(result),
        _ => Err(invalid_record(
            "publishing information operation requires exactly one domain result",
        )),
    }
}
fn required_record<'a>(
    view: &'a SimulationView<'_>,
    reference: &canwu_api::DomainRecordRef,
) -> Result<&'a DomainRecord, CanwuError> {
    view.domain_record(reference)?.ok_or_else(|| {
        invalid_record(format!(
            "required information record {reference} is unavailable"
        ))
    })
}
fn typed<T: DomainRecordType>(
    reference: canwu_api::DomainRecordRef,
) -> Result<TypedDomainRecordRef<T>, CanwuError> {
    TypedDomainRecordRef::from_untyped(reference)
        .map_err(|_| invalid_record("information record reference has the wrong kind"))
}
fn single_typed<T: DomainRecordType>(
    record: &DomainRecord,
    role: &str,
) -> Result<TypedDomainRecordRef<T>, CanwuError> {
    let values = role_targets(record, role);
    match values.as_slice() {
        [DomainReferenceTarget::Domain(reference)] => typed(reference.clone()),
        _ => Err(invalid_record(format!(
            "information record requires exactly one {role} domain reference"
        ))),
    }
}
fn single_holder(record: &DomainRecord, role: &str) -> Result<KnowledgeHolderRef, CanwuError> {
    let values = role_targets(record, role);
    match values.as_slice() {
        [target] => Ok(holder_from_target(target.clone())),
        _ => Err(invalid_record(format!(
            "information record requires exactly one {role} holder reference"
        ))),
    }
}
fn role_targets(record: &DomainRecord, role: &str) -> Vec<DomainReferenceTarget> {
    record
        .references
        .iter()
        .filter(|reference| reference.role == role)
        .map(|reference| reference.target.clone())
        .collect()
}
fn holder_from_target(target: DomainReferenceTarget) -> KnowledgeHolderRef {
    match target {
        DomainReferenceTarget::Core(EntityRef::Person(person)) => {
            KnowledgeHolderRef::Person(person)
        }
        DomainReferenceTarget::Core(entity) => KnowledgeHolderRef::Entity(entity),
        DomainReferenceTarget::Domain(reference) => {
            KnowledgeHolderRef::Entity(EntityRef::Domain(reference))
        }
    }
}
fn subject(role: &str, reference: &canwu_api::DomainRecordRef) -> KnowledgeSubject {
    KnowledgeSubject {
        role: role.to_owned(),
        target: KnowledgeSubjectTarget::DomainRecord(reference.clone()),
    }
}
fn knowledge_schema(name: &str) -> KnowledgeSchemaId {
    KnowledgeSchemaId::new(KnowledgeRecordKind::new(PLUGIN_NAMESPACE, name), 1)
}
fn record_state<T: DomainRecordType>() -> StateKey {
    canwu_api::DomainRecordSchema::for_type::<T>().state_key()
}
fn validate_runtime_interpretation_authority(
    view: &SimulationView<'_>,
    envelope: &InformationOperationEnvelope,
) -> Result<Option<String>, CanwuError> {
    let LifecycleRequest::RecordInterpretation {
        binding,
        payload,
        authority,
    } = &envelope.operation.request
    else {
        return Ok(None);
    };
    let performed_by = single_entity_target(&binding.references, "performed_by")?;
    let performed_for = single_holder_from_references(&binding.references, "performed_for")?;
    if matches!(authority, InterpretationAuthority::HolderSelf) {
        if holder_from_entity(&performed_by) != performed_for {
            return Err(invalid_record(
                "self interpretation authority requires performer and holder equality",
            ));
        }
        return Ok(None);
    }
    let (grant_code, evidence) = match authority {
        InterpretationAuthority::InstitutionalRole {
            assignment,
            authority_grant,
        } => (
            authority_grant.as_str(),
            EvidenceRef::DomainRecordVersion(assignment.clone()),
        ),
        InterpretationAuthority::Delegated {
            evidence,
            authority_grant,
        } => (authority_grant.as_str(), evidence.clone()),
        InterpretationAuthority::HolderSelf => unreachable!("holder-self returned above"),
    };
    let grants = information_authority_grants();
    let grant = grants
        .iter()
        .find(|candidate| candidate.code == grant_code)
        .ok_or_else(|| invalid_record("interpretation names an unknown authority grant"))?;
    if matches!(authority, InterpretationAuthority::InstitutionalRole { .. })
        != matches!(
            grant.selector,
            DelegationEvidenceSelector::DomainRecord { .. }
        )
    {
        return Err(invalid_record(
            "interpretation authority variant does not match its grant selector",
        ));
    }
    let persisted_payload = authority_evidence_payload(view, &evidence, &grant.selector)?;
    let claim_value = extract_claim_value(&persisted_payload, &grant.claim_path)?;
    let claim: DelegationClaimV1 = serde_json::from_value(claim_value).map_err(|error| {
        invalid_record(format!(
            "interpretation authority claim could not be decoded: {error}"
        ))
    })?;
    validate_delegation_claim(
        &claim,
        &performed_by,
        &performed_for,
        &payload.capability,
        payload.interpreted_at,
    )
    .map_err(invalid_record)?;
    canonical_hash(AUTHORITY_CLAIM_HASH_DOMAIN, &claim)
        .map(Some)
        .map_err(|error| invalid_record(error.to_string()))
}

fn authority_evidence_payload(
    view: &SimulationView<'_>,
    evidence: &EvidenceRef,
    selector: &DelegationEvidenceSelector,
) -> Result<Value, CanwuError> {
    match (selector, evidence) {
        (
            DelegationEvidenceSelector::Command {
                producer_plugin,
                command_type,
            },
            EvidenceRef::Command(id),
        ) => {
            let record = view
                .command(*id)?
                .ok_or_else(|| invalid_record("delegation command evidence is unavailable"))?;
            match &record.envelope.command {
                Command::Plugin {
                    plugin,
                    command,
                    payload,
                } if plugin == producer_plugin && command == command_type => Ok(payload.clone()),
                _ => Err(invalid_record(
                    "delegation command producer or type does not match its grant",
                )),
            }
        }
        (
            DelegationEvidenceSelector::Ingress {
                producer_plugin,
                packet_type,
            },
            EvidenceRef::Ingress(id),
        ) => {
            let record = view
                .ingress(*id)?
                .ok_or_else(|| invalid_record("delegation ingress evidence is unavailable"))?;
            match &record.payload {
                IngressPayload::Plugin {
                    plugin,
                    packet_type: actual_type,
                    payload,
                    ..
                } if plugin == producer_plugin && actual_type == packet_type => Ok(payload.clone()),
                _ => Err(invalid_record(
                    "delegation ingress producer or type does not match its grant",
                )),
            }
        }
        (
            DelegationEvidenceSelector::DomainRecord { owner_plugin, kind },
            EvidenceRef::DomainRecordVersion(version),
        ) => exact_domain_record_payload(view, version, owner_plugin, kind),
        _ => Err(invalid_record(
            "delegation evidence kind does not match its authority grant",
        )),
    }
}

fn exact_domain_record_payload(
    view: &SimulationView<'_>,
    version: &DomainRecordVersionRef,
    owner_plugin: &str,
    kind: &DomainRecordKind,
) -> Result<Value, CanwuError> {
    if &version.record.kind != kind || version.version == 0 {
        return Err(invalid_record(
            "delegation record evidence has the wrong kind or version",
        ));
    }
    if !view.domain_record_version_evidence_exists(version)? {
        return Err(invalid_record(
            "delegation record evidence does not name an exact available version",
        ));
    }
    let proposed_version = view.proposed_domain_record_version(&version.record)?;
    let record = if let Some(actual) = proposed_version {
        if actual != *version {
            return Err(invalid_record(
                "delegation record evidence does not name the exact proposed version",
            ));
        }
        view.proposed_domain_record(&version.record)?
    } else {
        view.domain_record(&version.record)?
    }
    .ok_or_else(|| invalid_record("delegation record evidence is unavailable"))?;
    if record.owner != owner_plugin
        || record.reference.kind != *kind
        || record.version != version.version
    {
        return Err(invalid_record(
            "delegation record owner, kind, or version does not match its grant",
        ));
    }
    Ok(record.payload.clone())
}

fn extract_claim_value(payload: &Value, path: &[String]) -> Result<Value, CanwuError> {
    let mut current = payload;
    for key in path {
        current = current
            .as_object()
            .and_then(|object| object.get(key))
            .ok_or_else(|| invalid_record("delegation claim path is missing or not an object"))?;
    }
    Ok(current.clone())
}

fn single_entity_target(
    references: &[DomainReference],
    role: &str,
) -> Result<EntityRef, CanwuError> {
    let values = references
        .iter()
        .filter(|reference| reference.role == role)
        .map(|reference| reference.target.clone())
        .collect::<Vec<_>>();
    match values.as_slice() {
        [DomainReferenceTarget::Core(entity)] => Ok(entity.clone()),
        [DomainReferenceTarget::Domain(reference)] => Ok(EntityRef::Domain(reference.clone())),
        _ => Err(invalid_record(format!(
            "interpretation requires exactly one {role} entity reference"
        ))),
    }
}

fn single_holder_from_references(
    references: &[DomainReference],
    role: &str,
) -> Result<KnowledgeHolderRef, CanwuError> {
    let values = references
        .iter()
        .filter(|reference| reference.role == role)
        .map(|reference| reference.target.clone())
        .collect::<Vec<_>>();
    match values.as_slice() {
        [target] => Ok(holder_from_target(target.clone())),
        _ => Err(invalid_record(format!(
            "interpretation requires exactly one {role} holder reference"
        ))),
    }
}

fn holder_from_entity(entity: &EntityRef) -> KnowledgeHolderRef {
    match entity {
        EntityRef::Person(person) => KnowledgeHolderRef::Person(*person),
        entity => KnowledgeHolderRef::Entity(entity.clone()),
    }
}

fn decode_envelope(payload: &Value) -> Result<InformationOperationEnvelope, CanwuError> {
    let envelope: InformationOperationEnvelope =
        serde_json::from_value(payload.clone()).map_err(|error| {
            CanwuError::new(
                ErrorCode::InvalidPayload,
                format!("information operation payload could not be decoded: {error}"),
            )
        })?;
    validate_operation_envelope(&envelope, InformationLimitsV1::canonical())
        .map_err(|message| CanwuError::new(ErrorCode::InvalidPayload, message))?;
    Ok(envelope)
}
fn invalid_record(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidDomainRecord, message)
}
fn encode_error(error: &serde_json::Error) -> CanwuError {
    CanwuError::new(
        ErrorCode::InvalidPayload,
        format!("information runtime payload could not be encoded: {error}"),
    )
}

fn operation_payload_schema() -> PayloadSchema {
    PayloadSchema::Object {
        properties: BTreeMap::from([
            ("id".to_owned(), required(PayloadValueType::Object)),
            ("lineage".to_owned(), optional(PayloadValueType::Array)),
            ("operation".to_owned(), required(PayloadValueType::Object)),
            (
                "operation_kind".to_owned(),
                required(PayloadValueType::String),
            ),
            (
                "operation_version".to_owned(),
                required(PayloadValueType::Integer),
            ),
            ("output_slots".to_owned(), required(PayloadValueType::Array)),
        ]),
        allow_additional: false,
    }
}
fn finalization_payload_schema() -> PayloadSchema {
    PayloadSchema::Object {
        properties: BTreeMap::from([
            ("id".to_owned(), required(PayloadValueType::Object)),
            (
                "canonical_input_hash".to_owned(),
                required(PayloadValueType::String),
            ),
        ]),
        allow_additional: false,
    }
}
const fn required(value_type: PayloadValueType) -> PayloadProperty {
    PayloadProperty {
        value_type,
        required: true,
    }
}
const fn optional(value_type: PayloadValueType) -> PayloadProperty {
    PayloadProperty {
        value_type,
        required: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_installs_authoritative_runtime_contracts() {
        let mut registry = canwu_api::PluginRegistry::default();
        let mut types = canwu_api::SchemaRegistry::default();
        registry
            .register(&InformationPlugin, &mut types)
            .expect("information plugin should register");
        let descriptor = registry.descriptors().next().expect("descriptor");
        assert_eq!(descriptor.record_schemas.len(), 12);
        assert_eq!(descriptor.knowledge_schemas.len(), 4);
        assert_eq!(descriptor.commands.len(), 1);
        assert_eq!(descriptor.ingress.len(), 2);
        assert_eq!(descriptor.boundary_systems.len(), 2);
        assert!(
            descriptor
                .boundary_systems
                .iter()
                .any(|system| system.phase == BoundaryPhase::DomainDeltaProposal)
        );
        assert!(
            descriptor
                .boundary_systems
                .iter()
                .any(|system| system.phase == BoundaryPhase::PerspectiveAndReportMaterialization)
        );
    }

    #[test]
    fn semantic_identity_and_transport_names_are_fixed() {
        let plugin = InformationPlugin;
        assert_eq!(plugin.name(), PLUGIN_NAME);
        assert_eq!(plugin.semantic_hash().len(), 64);
        assert_eq!(plugin.command_descriptor().name, INFORMATION_COMMAND);
        assert_eq!(plugin.ingress_descriptor().name, INFORMATION_INGRESS);
        assert_eq!(
            canonical_hash(
                AUTHORITY_GRANTS_HASH_DOMAIN,
                &information_authority_grants()
            )
            .expect("authority grants should hash"),
            AUTHORITY_GRANTS_HASH
        );
        assert_ne!(plugin.semantic_hash(), AUTHORITY_GRANTS_HASH);
    }
}
