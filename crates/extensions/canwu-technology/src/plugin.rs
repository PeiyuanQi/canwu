use crate::model::{
    ProgramStatus, TechnologyCommandEnvelope, TechnologyExecutionIntent,
    TechnologyExecutionIntentPayload, TechnologyIntentRequest, TechnologyIntentState,
    TechnologyLimitsV1, TechnologyOperation, TechnologyOperationPayload, TechnologyOperationStatus,
    TechnologyRecordChange, TechnologyRecordPayload, TechnologyResultEnvelope,
    attach_payload_continuation,
};
use crate::query::{TechnologyEvidenceAccess, TechnologyRecordSet};
use crate::schema::{
    ADOPTION_KNOWLEDGE, ATTEMPT_KNOWLEDGE, CAPABILITY_KNOWLEDGE, CLAIM_KNOWLEDGE,
    IMPLEMENTATION_KNOWLEDGE, technology_knowledge_schemas, technology_record_schemas,
};
use crate::{PLUGIN_NAME, PLUGIN_NAMESPACE};
use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundarySystemContract,
    CanwuError, CauseRef, Command, CommandContext, CommandIngress, DomainRecord, DomainRecordClass,
    DomainRecordDraft, DomainRecordLifecycle, DomainRecordMutation, DomainReference,
    DomainReferenceTarget, ErrorCode, EvidenceRef, IngressClass, IngressPayload, Issuer,
    KnowledgeHolderRef, KnowledgeOrigin, KnowledgeRecordDraft, KnowledgeRecordKind,
    KnowledgeSchemaId, KnowledgeSubject, KnowledgeSubjectTarget, KnowledgeWriteGrant,
    PayloadProperty, PayloadSchema, PayloadValueType, PluginActionDescriptor,
    PluginIngressDescriptor, PluginRegistrar, SimDuration, SimulationPlugin, SimulationView,
    StateKey, StateVisibility, SystemCadence, SystemDirective, TypedDomainRecordRef,
    canonical_hash,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const TECHNOLOGY_COMMAND: &str = "apply_technology_operation_v1";
pub(crate) const TECHNOLOGY_COMMAND_INGRESS: &str = "technology_command_v1";
pub const TECHNOLOGY_RESULT_INGRESS: &str = "technology_result_v1";
const PLUGIN_VERSION: &str = env!("CARGO_PKG_VERSION");
const SEMANTIC_HASH: &str = "ab7e52dd84e24e862e5c1f14f8048db2b473d108c7958a333b72d8e852a03080";
pub(crate) const INPUT_HASH_DOMAIN: &str = "canwu.technology.operation-input.v1";
pub(crate) const CONFLICT_HASH_DOMAIN: &str = "canwu.technology.operation-conflict.v1";
pub(crate) const APPLY_SYSTEM: &str = "technology_operation_apply_v1";
pub(crate) const FINALIZE_SYSTEM: &str = "technology_intent_finalize_v1";
const PUBLISH_SYSTEM: &str = "technology_knowledge_publish_v1";
pub(crate) const CAPACITY_REJECTION_EVENT: &str = "technology_operation_rejected_capacity_v1";
const KNOWLEDGE_CAPACITY_REJECTION_EVENT: &str = "technology_knowledge_rejected_capacity_v1";

#[derive(Clone, Copy, Debug, Default)]
pub struct TechnologyPlugin;

#[must_use]
pub fn technology_command_descriptor() -> PluginActionDescriptor {
    PluginActionDescriptor {
        name: TECHNOLOGY_COMMAND.to_owned(),
        description: "Submit one authority-checked technology operation".to_owned(),
        payload_schema: command_payload_schema(),
        reads: vec![StateKey::new(PLUGIN_NAMESPACE, "operation")],
        writes: Vec::new(),
    }
}

#[must_use]
pub fn technology_result_ingress_descriptor() -> PluginIngressDescriptor {
    PluginIngressDescriptor {
        name: TECHNOLOGY_RESULT_INGRESS.to_owned(),
        description: "Admit one resolved provider result or passive observation".to_owned(),
        class: IngressClass::Acknowledgement,
        payload_schema: result_payload_schema(),
    }
}

impl SimulationPlugin for TechnologyPlugin {
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
        let record_schemas = technology_record_schemas();
        let knowledge_schemas = technology_knowledge_schemas();
        for schema in &record_schemas {
            registrar.register_record_schema(schema.clone())?;
        }
        for schema in &knowledge_schemas {
            registrar.register_knowledge_schema(schema.clone())?;
        }
        registrar.register_command(technology_command_descriptor(), apply_technology_command)?;
        registrar.register_ingress(PluginIngressDescriptor {
            name: TECHNOLOGY_COMMAND_INGRESS.to_owned(),
            description: "Apply an admitted authority-checked technology command".to_owned(),
            class: IngressClass::ScheduledSystem,
            payload_schema: command_payload_schema(),
        })?;
        registrar.register_ingress(technology_result_ingress_descriptor())?;

        let mut apply = BoundarySystemContract::new(
            APPLY_SYSTEM,
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        apply.reads = record_schemas
            .iter()
            .map(canwu_api::DomainRecordSchema::state_key)
            .collect();
        apply.reads.push(StateKey::core_ingress());
        apply.reads.extend([
            StateKey::core_commands(),
            StateKey::core_events(),
            StateKey::core_evidence(),
        ]);
        apply.reads.sort();
        apply.reads.dedup();
        apply.writes = record_schemas
            .iter()
            .map(canwu_api::DomainRecordSchema::state_key)
            .collect();
        apply.emits = vec![CAPACITY_REJECTION_EVENT.to_owned()];
        apply.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(apply, apply_operations)?;

        let mut finalize = BoundarySystemContract::new(
            FINALIZE_SYSTEM,
            BoundaryPhase::StrategicAggregation,
            SystemCadence::EventDriven,
        );
        finalize.reads = record_schemas
            .iter()
            .map(canwu_api::DomainRecordSchema::state_key)
            .collect();
        finalize.reads.push(StateKey::core_ingress());
        finalize.reads.extend([
            StateKey::core_commands(),
            StateKey::core_events(),
            StateKey::core_evidence(),
        ]);
        finalize.reads.sort();
        finalize.reads.dedup();
        finalize.writes = record_schemas
            .iter()
            .filter(|schema| schema.kind.matches_type::<TechnologyExecutionIntent>())
            .map(canwu_api::DomainRecordSchema::state_key)
            .collect();
        finalize.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(finalize, finalize_intents)?;

        let mut publish = BoundarySystemContract::new(
            PUBLISH_SYSTEM,
            BoundaryPhase::PerspectiveAndReportMaterialization,
            SystemCadence::EventDriven,
        );
        publish.reads = record_schemas
            .iter()
            .map(canwu_api::DomainRecordSchema::state_key)
            .collect();
        publish.reads.push(StateKey::core_ingress());
        publish.reads.push(StateKey::core_knowledge());
        publish.reads.push(StateKey::core_commands());
        publish.reads.sort();
        publish.reads.dedup();
        publish.knowledge_writes = knowledge_schemas
            .into_iter()
            .map(|schema| KnowledgeWriteGrant {
                schema: schema.id,
                visibilities: vec![StateVisibility::SameBoundary],
            })
            .collect();
        publish.emits = vec![KNOWLEDGE_CAPACITY_REJECTION_EVENT.to_owned()];
        publish.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(publish, publish_knowledge)
    }
}

fn apply_technology_command(
    view: &SimulationView<'_>,
    context: &CommandContext,
    payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    if context.ingress == CommandIngress::LegacyDirect {
        return Err(CanwuError::new(
            ErrorCode::MixedCommandIngress,
            "technology operations require tracked command ingress",
        ));
    }
    let envelope: TechnologyCommandEnvelope = decode(payload, "technology command")?;
    validate_identifier(&envelope.id, "technology operation")?;
    let value = change_value(&envelope.change);
    if value.authority_subject() != Some(&envelope.subject) {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "technology command subject does not own the requested action or the record is result-only",
        ));
    }
    require_subject_authority(context, &envelope.subject)?;
    let input_hash = canonical_hash(INPUT_HASH_DOMAIN, &envelope)?;
    if let Some(existing) = view.typed_domain_record(&operation_ref(&envelope.id))? {
        let existing = existing.decode_payload::<TechnologyOperation>()?;
        if existing
            .canonical_input_hashes
            .binary_search(&input_hash)
            .is_ok()
        {
            return Ok(Vec::new());
        }
        return Err(CanwuError::new(
            ErrorCode::IdempotencyConflict,
            "technology operation ID was reused with different input",
        ));
    }
    let affected = affected_entities(value);
    Ok(vec![SystemDirective::EnqueuePluginIngress {
        after: SimDuration::ZERO,
        packet_type: TECHNOLOGY_COMMAND_INGRESS.to_owned(),
        priority: 0,
        payload: serde_json::to_value(&envelope).map_err(|error| encoding_error(&error))?,
        affected,
    }])
}

#[allow(clippy::too_many_lines)]
fn apply_operations(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let operations = admitted_operations(view, context)?;
    if operations.is_empty() {
        return Ok(BoundaryProposal::default());
    }
    let mut state = TechnologyRecordSet::load(view)?;
    let mut directives = Vec::new();
    let mut new_operations = BTreeMap::new();
    for (id, operation) in operations {
        let existing = view.typed_domain_record(&operation_ref(&operation.id))?;
        if let Some(existing) = existing {
            let existing = existing.decode_payload::<TechnologyOperation>()?;
            if !operation
                .input_hashes
                .iter()
                .all(|hash| existing.canonical_input_hashes.binary_search(hash).is_ok())
            {
                return Err(CanwuError::new(
                    ErrorCode::IdempotencyConflict,
                    "technology operation ID was reused with different input",
                ));
            }
            continue;
        }
        new_operations.insert(id, operation);
    }
    let maximum_mutations = new_operations
        .values()
        .try_fold(0usize, |total, operation| {
            total.checked_add(2 + usize::from(operation.execution_intent().is_some()))
        });
    let limits = TechnologyLimitsV1::canonical();
    let capacity_exhausted = state
        .records
        .len()
        .checked_add(new_operations.len())
        .is_none_or(|count| count > limits.max_total_records);
    if capacity_exhausted
        || maximum_mutations.is_none_or(|maximum| maximum > limits.max_mutations_per_boundary)
    {
        let reason = if capacity_exhausted {
            "technology record capacity cannot retain terminal operation outcomes"
        } else {
            "technology boundary mutation budget is exhausted"
        };
        return Ok(capacity_rejection_proposal(new_operations, reason));
    }
    for reduced in reduce_new_operations(view, &mut state, new_operations, context.at)? {
        if let Some(mutation) = reduced.mutation {
            directives.push(BoundaryDirective::MutateRecord {
                mutation,
                summary: "Apply authority-checked technology record change".to_owned(),
            });
        }
        directives.push(BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Create {
                record: operation_draft(&reduced.outcome)?,
            },
            summary: "Record terminal technology operation outcome".to_owned(),
        });
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

fn capacity_rejection_proposal(
    operations: BTreeMap<String, AdmittedOperation>,
    reason: &str,
) -> BoundaryProposal {
    BoundaryProposal {
        directives: operations
            .into_values()
            .map(|operation| BoundaryDirective::Emit {
                event_type: CAPACITY_REJECTION_EVENT.to_owned(),
                summary: format!("Reject technology operation {}: {reason}", operation.id),
                affected: affected_entities(change_value(&operation.change)),
            })
            .collect(),
        ..BoundaryProposal::default()
    }
}

fn finalize_intents(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let operations = admitted_operations(view, context)?;
    let contested = contested_intents(&operations);
    let command_targets = command_intent_targets(&operations);
    let mut directives = Vec::new();
    for operation in operations.into_values() {
        if operation.conflicted() {
            continue;
        }
        let OperationOrigin::Result {
            execution_intent: Some(intent_ref),
            ingress,
            ..
        } = &operation.origin
        else {
            continue;
        };
        if contested.contains(intent_ref) || command_targets.contains(&intent_ref.record) {
            continue;
        }
        let outcome_ref = operation_ref(&operation.id).into_untyped();
        let Some(outcome) = view.typed_domain_record(&operation_ref(&operation.id))? else {
            continue;
        };
        let outcome = outcome.decode_payload::<TechnologyOperation>()?;
        if outcome.status != TechnologyOperationStatus::Applied
            || outcome.execution_intent.as_ref() != Some(intent_ref)
        {
            continue;
        }
        let result_ref = outcome
            .result
            .as_ref()
            .ok_or_else(|| invalid("applied provider operation has no result"))?;
        let operation_version = view
            .proposed_domain_record_version(&outcome_ref)?
            .ok_or_else(|| invalid("provider operation exact version is unavailable"))?;
        let result_version = view
            .proposed_domain_record_version(result_ref)?
            .ok_or_else(|| invalid("provider result exact version is unavailable"))?;
        let current = view
            .domain_record_version(intent_ref)?
            .ok_or_else(|| invalid("provider intent exact version is unavailable"))?;
        let mut intent = current.decode_payload::<TechnologyExecutionIntent>()?;
        if intent.state != TechnologyIntentState::Pending {
            continue;
        }
        intent.state = TechnologyIntentState::Consumed {
            ingress: ingress.clone(),
            operation: operation_version,
            result: result_version,
        };
        let id = intent_ref.record.id.clone();
        let draft = TechnologyRecordPayload::ExecutionIntent(intent).draft(id)?;
        directives.push(BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Update {
                record: draft,
                expected_version: intent_ref.version,
            },
            summary: "Atomically consume the exact technology execution intent".to_owned(),
        });
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

fn publish_knowledge(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let operations = admitted_operations(view, context)?;
    let mut publications = Vec::new();
    for operation in operations.into_values() {
        let target = change_value(&operation.change).reference(change_id(&operation.change));
        let Some(outcome) = view.typed_domain_record(&operation_ref(&operation.id))? else {
            continue;
        };
        let outcome = outcome.decode_payload::<TechnologyOperation>()?;
        if outcome.status != TechnologyOperationStatus::Applied
            || outcome.result.as_ref() != Some(&target)
        {
            continue;
        }
        if let Some(intent_ref) = &outcome.execution_intent {
            let Some(intent) =
                view.typed_domain_record(&canwu_api::TypedDomainRecordRef::<
                    TechnologyExecutionIntent,
                >::new(intent_ref.record.id.clone()))?
            else {
                continue;
            };
            let intent = intent.decode_payload::<TechnologyExecutionIntent>()?;
            if !matches!(
                intent.state,
                TechnologyIntentState::Consumed { ref result, .. }
                    if result.record == target
            ) {
                continue;
            }
        }
        let Some(version) = view.proposed_domain_record_version(&target)? else {
            continue;
        };
        let Some((holder, schema)) = publication(change_value(&operation.change)) else {
            continue;
        };
        publications.push((
            holder.clone(),
            knowledge_draft(
                schema,
                &version,
                change_value(&operation.change),
                context.at,
            )?,
        ));
    }
    let existing = view.knowledge_record_count_in_namespace(PLUGIN_NAMESPACE)?;
    let limits = TechnologyLimitsV1::canonical();
    let retained_capacity = limits.max_knowledge_records.saturating_sub(existing);
    let publication_capacity = retained_capacity.min(limits.max_publications_per_boundary);
    let directives = publications
        .into_iter()
        .enumerate()
        .map(|(index, (holder, record))| {
            if index < publication_capacity {
                BoundaryDirective::PublishKnowledge {
                    holder,
                    visibility: StateVisibility::SameBoundary,
                    producer_correlation: None,
                    records: vec![record],
                    summary: "Publish holder-relative technology evidence".to_owned(),
                }
            } else {
                BoundaryDirective::Emit {
                    event_type: KNOWLEDGE_CAPACITY_REJECTION_EVENT.to_owned(),
                    summary: if index >= limits.max_publications_per_boundary {
                        "Reject technology knowledge publication: boundary publication budget is exhausted"
                            .to_owned()
                    } else {
                        "Reject technology knowledge publication: retained knowledge capacity is exhausted"
                            .to_owned()
                    },
                    affected: affected_holder_entities(&holder),
                }
            }
        })
        .collect();
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

#[derive(Clone)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum OperationOrigin {
    Command,
    Result {
        provider: String,
        execution_intent: Option<canwu_api::DomainRecordVersionRef>,
        ingress: EvidenceRef,
    },
}

pub(crate) struct AdmittedOperation {
    pub(crate) id: String,
    pub(crate) input_hashes: BTreeSet<String>,
    pub(crate) causes: BTreeMap<String, EvidenceRef>,
    pub(crate) change: TechnologyRecordChange,
    pub(crate) origin: OperationOrigin,
}

impl AdmittedOperation {
    fn conflicted(&self) -> bool {
        self.input_hashes.len() > 1
    }

    fn summary_hash(&self) -> Result<String, CanwuError> {
        if self.input_hashes.len() == 1 {
            return Ok(self.input_hashes.first().cloned().expect("one input hash"));
        }
        canonical_hash(
            CONFLICT_HASH_DOMAIN,
            &self.input_hashes.iter().collect::<Vec<_>>(),
        )
    }

    fn provider(&self) -> Option<&str> {
        match &self.origin {
            OperationOrigin::Command => None,
            OperationOrigin::Result { provider, .. } => Some(provider),
        }
    }

    pub(crate) fn execution_intent(&self) -> Option<&canwu_api::DomainRecordVersionRef> {
        match &self.origin {
            OperationOrigin::Command => None,
            OperationOrigin::Result {
                execution_intent, ..
            } => execution_intent.as_ref(),
        }
    }
}

pub(crate) struct ReducedOperation {
    pub(crate) outcome: TechnologyOperationPayload,
    pub(crate) mutation: Option<DomainRecordMutation>,
    pub(crate) candidate: Option<DomainRecord>,
    pub(crate) previous: Option<DomainRecord>,
}

pub(crate) fn reduce_new_operations(
    access: &(impl TechnologyEvidenceAccess + ?Sized),
    state: &mut TechnologyRecordSet,
    operations: BTreeMap<String, AdmittedOperation>,
    at: canwu_api::SimTime,
) -> Result<Vec<ReducedOperation>, CanwuError> {
    let reserved_outcomes = operations.len();
    ensure_total_capacity(state.records.len(), reserved_outcomes)?;
    let contested_intents = contested_intents(&operations);
    let command_intent_targets = command_intent_targets(&operations);
    let mut reduced = Vec::with_capacity(operations.len());
    for operation in operations.into_values() {
        let result = if operation.conflicted() {
            Err(CanwuError::new(
                ErrorCode::IdempotencyConflict,
                "technology operation ID was reused with different input in one boundary",
            ))
        } else if operation.execution_intent().is_some_and(|intent| {
            contested_intents.contains(intent) || command_intent_targets.contains(&intent.record)
        }) {
            Err(invalid(
                "technology execution intent has competing work in one boundary",
            ))
        } else {
            prepare_change(access, state, &operation.change, &operation.origin, at).and_then(
                |(candidate, mutation)| {
                    state.hydrate(access, change_value(&operation.change).exact_versions())?;
                    let reference = candidate.reference.clone();
                    let previous = state.records.insert(reference.clone(), candidate.clone());
                    if let Err(error) =
                        ensure_total_capacity(state.records.len(), reserved_outcomes)
                            .and_then(|()| state.validate(at))
                            .and_then(|()| state.validate_temporal_evidence(access))
                    {
                        if let Some(previous) = &previous {
                            state.records.insert(reference, previous.clone());
                        } else {
                            state.records.remove(&reference);
                        }
                        return Err(error);
                    }
                    Ok((candidate, mutation, previous))
                },
            )
        };
        let (status, rejection_code, result_ref, candidate, mutation, previous) = match result {
            Ok((candidate, mutation, previous)) => (
                TechnologyOperationStatus::Applied,
                None,
                Some(mutation.target().clone()),
                Some(candidate),
                Some(mutation),
                previous,
            ),
            Err(error) if is_expected_domain_rejection(&error) => (
                TechnologyOperationStatus::Rejected,
                Some(error.code.as_str().to_owned()),
                None,
                None,
                None,
                None,
            ),
            Err(error) => return Err(error),
        };
        let mut causes = operation.causes.values().cloned().collect::<Vec<_>>();
        causes.sort();
        causes.dedup();
        let conflicted = operation.conflicted();
        let canonical_input_hash = operation.summary_hash()?;
        let provider = operation.provider().map(str::to_owned);
        let execution_intent = operation.execution_intent().cloned();
        reduced.push(ReducedOperation {
            outcome: TechnologyOperationPayload {
                id: operation.id,
                canonical_input_hash,
                canonical_input_hashes: operation.input_hashes.into_iter().collect(),
                causes,
                provider: if conflicted { None } else { provider },
                execution_intent: if conflicted { None } else { execution_intent },
                status,
                result: result_ref,
                rejection_code,
            },
            mutation,
            candidate,
            previous,
        });
    }
    Ok(reduced)
}

pub(crate) fn operation_draft(
    outcome: &TechnologyOperationPayload,
) -> Result<DomainRecordDraft, CanwuError> {
    let mut draft = DomainRecordDraft::from_typed(operation_ref(&outcome.id), outcome)?;
    attach_payload_continuation(&mut draft.payload, outcome.execution_intent.iter().cloned())?;
    if let Some(result) = &outcome.result {
        draft.references.push(DomainReference {
            role: "domain".to_owned(),
            target: DomainReferenceTarget::Domain(result.clone()),
        });
    }
    Ok(draft)
}

fn ensure_total_capacity(current: usize, reserved_outcomes: usize) -> Result<(), CanwuError> {
    if current
        .checked_add(reserved_outcomes)
        .is_none_or(|total| total > TechnologyLimitsV1::canonical().max_total_records)
    {
        return Err(invalid(
            "technology records plus terminal outcomes exceed the shared total cap",
        ));
    }
    Ok(())
}

fn admitted_operations(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BTreeMap<String, AdmittedOperation>, CanwuError> {
    let mut operations = BTreeMap::<String, AdmittedOperation>::new();
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
        let (id, change, origin, cause, input_hash) = if packet_type == TECHNOLOGY_COMMAND_INGRESS {
            let envelope: TechnologyCommandEnvelope =
                decode(payload, "technology command ingress")?;
            let cause = match ingress.cause {
                Some(CauseRef::Command(command)) => EvidenceRef::Command(command),
                _ => return Err(invalid("technology command ingress lacks command evidence")),
            };
            let command_id = match &cause {
                EvidenceRef::Command(command) => *command,
                _ => unreachable!("technology command cause was checked above"),
            };
            let command = view.command(command_id)?.ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "technology command evidence is unavailable",
                )
            })?;
            if !matches!(
                &command.envelope.command,
                Command::Plugin { plugin, command, payload }
                    if plugin == PLUGIN_NAME
                        && command == TECHNOLOGY_COMMAND
                        && decode::<TechnologyCommandEnvelope>(payload, "technology command evidence")
                            .is_ok_and(|value| value == envelope)
            ) {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "technology command ingress does not match its authorized command",
                ));
            }
            let hash = canonical_hash(INPUT_HASH_DOMAIN, &envelope)?;
            (
                envelope.id,
                envelope.change,
                OperationOrigin::Command,
                cause,
                hash,
            )
        } else if packet_type == TECHNOLOGY_RESULT_INGRESS {
            let envelope: TechnologyResultEnvelope = decode(payload, "technology result ingress")?;
            validate_identifier(&envelope.provider, "technology provider")?;
            let cause = EvidenceRef::Ingress(*ingress_id);
            let hash = canonical_hash(INPUT_HASH_DOMAIN, &envelope)?;
            (
                envelope.id,
                envelope.change,
                OperationOrigin::Result {
                    provider: envelope.provider,
                    execution_intent: envelope.execution_intent,
                    ingress: cause.clone(),
                },
                cause,
                hash,
            )
        } else {
            continue;
        };
        validate_identifier(&id, "technology operation")?;
        if let Some(prior) = operations.get_mut(&id) {
            prior.input_hashes.insert(input_hash.clone());
            let cause_entry = prior.causes.entry(input_hash).or_insert(cause.clone());
            if cause < *cause_entry {
                *cause_entry = cause;
            }
            continue;
        }
        let input_hashes = BTreeSet::from([input_hash.clone()]);
        let causes = BTreeMap::from([(input_hash, cause)]);
        operations.insert(
            id.clone(),
            AdmittedOperation {
                id,
                input_hashes,
                causes,
                change,
                origin,
            },
        );
    }
    Ok(operations)
}

fn contested_intents(
    operations: &BTreeMap<String, AdmittedOperation>,
) -> BTreeSet<canwu_api::DomainRecordVersionRef> {
    let mut first = BTreeSet::new();
    let mut contested = BTreeSet::new();
    for operation in operations.values() {
        if let Some(intent) = operation.execution_intent()
            && !first.insert(intent.clone())
        {
            contested.insert(intent.clone());
        }
    }
    contested
}

fn command_intent_targets(
    operations: &BTreeMap<String, AdmittedOperation>,
) -> BTreeSet<canwu_api::DomainRecordRef> {
    operations
        .values()
        .filter(|operation| matches!(operation.origin, OperationOrigin::Command))
        .filter_map(|operation| match &operation.change {
            TechnologyRecordChange::Update {
                id,
                value: TechnologyRecordPayload::ExecutionIntent(_),
                ..
            } => Some(
                canwu_api::TypedDomainRecordRef::<TechnologyExecutionIntent>::new(id)
                    .into_untyped(),
            ),
            _ => None,
        })
        .collect()
}

fn validate_command_intent_change(
    state: &TechnologyRecordSet,
    change: &TechnologyRecordChange,
    intent: &TechnologyExecutionIntentPayload,
) -> Result<(), CanwuError> {
    validate_identifier(&intent.provider, "technology provider")?;
    if intent
        .expires_at
        .is_some_and(|until| until < intent.not_before)
    {
        return Err(invalid(
            "technology execution intent has an invalid time window",
        ));
    }
    validate_intent_request(&intent.request)?;
    match change {
        TechnologyRecordChange::Create { .. } => {
            if intent.state != TechnologyIntentState::Pending {
                return Err(invalid("new technology execution intent must be pending"));
            }
        }
        TechnologyRecordChange::Update { id, .. } => {
            let reference = canwu_api::TypedDomainRecordRef::<TechnologyExecutionIntent>::new(id)
                .into_untyped();
            let previous = state
                .records
                .get(&reference)
                .ok_or_else(|| invalid("technology execution intent update target is unavailable"))?
                .decode_payload::<TechnologyExecutionIntent>()?;
            let mut expected = previous.clone();
            expected.state = TechnologyIntentState::Cancelled;
            if previous.state != TechnologyIntentState::Pending || *intent != expected {
                return Err(invalid(
                    "ordinary commands may only cancel an unchanged pending technology intent",
                ));
            }
        }
    }
    Ok(())
}

fn validate_intent_request(request: &TechnologyIntentRequest) -> Result<(), CanwuError> {
    let result_id = match request {
        TechnologyIntentRequest::Experiment {
            result_id,
            operation,
            ..
        } => {
            validate_identifier(operation, "technology operation")?;
            result_id
        }
        TechnologyIntentRequest::Production { result_id, .. }
        | TechnologyIntentRequest::Invention { result_id, .. } => result_id,
    };
    validate_identifier(result_id, "technology result")
}

#[allow(clippy::too_many_lines)]
fn validate_result_intent(
    access: &(impl TechnologyEvidenceAccess + ?Sized),
    state: &TechnologyRecordSet,
    change: &TechnologyRecordChange,
    provider: &str,
    intent_ref: Option<&canwu_api::DomainRecordVersionRef>,
    at: canwu_api::SimTime,
) -> Result<(), CanwuError> {
    let value = change_value(change);
    if matches!(value, TechnologyRecordPayload::AttemptObservation(_)) {
        if intent_ref.is_some() {
            return Err(invalid(
                "passive observation must not consume an execution intent",
            ));
        }
        return Ok(());
    }
    let intent_ref = intent_ref.ok_or_else(|| {
        invalid("provider experiment, production, and invention results require an exact intent")
    })?;
    let current = state
        .records
        .get(&intent_ref.record)
        .ok_or_else(|| invalid("technology execution intent is unavailable"))?;
    if current.version != intent_ref.version {
        return Err(invalid("technology execution intent is no longer current"));
    }
    let intent = access
        .technology_domain_record_version(intent_ref)?
        .ok_or_else(|| invalid("exact technology execution intent is unavailable"))?
        .decode_payload::<TechnologyExecutionIntent>()?;
    if intent.provider != provider
        || intent.state != TechnologyIntentState::Pending
        || at < intent.not_before
        || intent.expires_at.is_some_and(|until| at > until)
    {
        return Err(invalid(
            "technology result provider, intent state, or time window does not match authorization",
        ));
    }
    let program_record = access
        .technology_domain_record_version(&intent.program)?
        .ok_or_else(|| invalid("technology intent program version is unavailable"))?;
    let current_program = state
        .records
        .get(&intent.program.record)
        .ok_or_else(|| invalid("technology intent program is unavailable"))?;
    let program = program_record.decode_payload::<crate::model::TechnicalProgram>()?;
    if current_program.version != intent.program.version
        || program.status != ProgramStatus::Active
        || program.sponsor != intent.authorized_by
        || (!program.requirements.is_empty()
            && !program
                .requirements
                .iter()
                .any(|requirement| requirement.provider == provider))
    {
        return Err(invalid(
            "technology result does not target the current authorized active program",
        ));
    }
    let result_id = change_id(change);
    let matches = match (&intent.request, value) {
        (
            TechnologyIntentRequest::Experiment {
                result_id: expected_id,
                revision,
                operation,
                site,
                operator,
                required_assets,
            },
            TechnologyRecordPayload::ExperimentAttempt(result),
        ) => {
            result.execution_intent == *intent_ref
                && result.program == intent.program
                && result.revision == *revision
                && result.operation == *operation
                && result.site == *site
                && operator
                    .as_ref()
                    .is_none_or(|value| value == &result.operator)
                && result.started_at >= intent.not_before
                && intent
                    .expires_at
                    .is_none_or(|expires_at| result.ended_at <= expires_at)
                && same_exact_set(required_assets, &result.assets)
                && result_id == expected_id
        }
        (
            TechnologyIntentRequest::Production {
                result_id: expected_id,
                revision,
                application,
                site,
                operator,
                required_assets,
            },
            TechnologyRecordPayload::ProductionRun(result),
        ) => {
            result.execution_intent == *intent_ref
                && result.revision == *revision
                && result.application == *application
                && result.site == *site
                && operator
                    .as_ref()
                    .is_none_or(|value| value == &result.operator)
                && result.started_at >= intent.not_before
                && intent
                    .expires_at
                    .is_none_or(|expires_at| result.ended_at <= expires_at)
                && same_exact_set(required_assets, &result.assets)
                && result_id == expected_id
        }
        (
            TechnologyIntentRequest::Invention {
                result_id: expected_id,
                spec,
                parent,
                site,
            },
            TechnologyRecordPayload::TechniqueRevision(result),
        ) => {
            result.execution_intent.as_ref() == Some(intent_ref)
                && result.produced_by.as_ref() == Some(&intent.program)
                && result.spec == *spec
                && program.site == *site
                && parent.as_ref().is_none_or(|expected| {
                    result.parents.iter().any(|value| &value.parent == expected)
                })
                && result_id == expected_id
        }
        _ => false,
    };
    if !matches {
        return Err(invalid(
            "technology result does not match the exact authorized intent",
        ));
    }
    Ok(())
}

fn same_exact_set(
    left: &[canwu_api::DomainRecordVersionRef],
    right: &[canwu_api::DomainRecordVersionRef],
) -> bool {
    left.iter().collect::<BTreeSet<_>>() == right.iter().collect::<BTreeSet<_>>()
}

#[allow(clippy::too_many_lines)]
fn prepare_change(
    access: &(impl TechnologyEvidenceAccess + ?Sized),
    state: &TechnologyRecordSet,
    change: &TechnologyRecordChange,
    origin: &OperationOrigin,
    at: canwu_api::SimTime,
) -> Result<(DomainRecord, DomainRecordMutation), CanwuError> {
    let value = change_value(change);
    for reference in value.exact_versions() {
        if !access.technology_domain_record_version_exists(&reference)? {
            return Err(invalid(format!(
                "exact technology evidence {reference:?} is unavailable"
            )));
        }
    }
    for reference in value.evidence_refs() {
        if !access.technology_evidence_exists(&reference)? {
            return Err(invalid(format!(
                "technology evidence {reference:?} is unavailable"
            )));
        }
    }
    match (origin, value) {
        (
            OperationOrigin::Result { .. },
            TechnologyRecordPayload::ExperimentAttempt(_)
            | TechnologyRecordPayload::AttemptObservation(_)
            | TechnologyRecordPayload::ProductionRun(_)
            | TechnologyRecordPayload::TechniqueRevision(_),
        ) => {}
        (
            OperationOrigin::Command,
            TechnologyRecordPayload::ExperimentAttempt(_)
            | TechnologyRecordPayload::AttemptObservation(_)
            | TechnologyRecordPayload::ProductionRun(_)
            | TechnologyRecordPayload::TechniqueRevision(_),
        ) => {
            return Err(invalid(
                "provider results cannot be created by a deliberate command",
            ));
        }
        (OperationOrigin::Result { .. }, _) => {
            return Err(invalid(
                "result ingress cannot create deliberate technology state",
            ));
        }
        (OperationOrigin::Command, TechnologyRecordPayload::ExecutionIntent(intent)) => {
            validate_command_intent_change(state, change, intent)?;
        }
        (OperationOrigin::Command, _) => {}
    }
    if let OperationOrigin::Result {
        provider,
        execution_intent,
        ..
    } = origin
    {
        validate_result_intent(
            access,
            state,
            change,
            provider,
            execution_intent.as_ref(),
            at,
        )?;
    }
    if matches!(origin, OperationOrigin::Command) {
        match (change, value) {
            (
                TechnologyRecordChange::Create { .. } | TechnologyRecordChange::Update { .. },
                TechnologyRecordPayload::Implementation(implementation),
            ) if implementation.active => {
                state.validate_current_implementation_dependencies(implementation)?;
            }
            (
                TechnologyRecordChange::Create { .. },
                TechnologyRecordPayload::Implementation(implementation),
            ) => {
                state.validate_current_implementation_dependencies(implementation)?;
            }
            (
                TechnologyRecordChange::Create { .. } | TechnologyRecordChange::Update { .. },
                TechnologyRecordPayload::Adoption(adoption),
            ) => {
                state.validate_current_adoption_dependencies(adoption)?;
            }
            _ => {}
        }
    }
    if let TechnologyRecordPayload::TechniqueRevision(revision) = value
        && revision.produced_by.is_none()
    {
        return Err(invalid(
            "result-created technique revision requires an invention program",
        ));
    }
    let id = change_id(change);
    validate_identifier(id, "technology record")?;
    let draft = value.draft(id)?;
    let (version, mutation) = match change {
        TechnologyRecordChange::Create { .. } => {
            if let TechnologyRecordPayload::Transmission(transmission) = value {
                state.validate_current_transmission_source(transmission)?;
            }
            if state.records.contains_key(&draft.reference) {
                return Err(CanwuError::new(
                    ErrorCode::DuplicateDomainRecord,
                    format!("technology record {} already exists", draft.reference),
                ));
            }
            (
                1,
                DomainRecordMutation::Create {
                    record: draft.clone(),
                },
            )
        }
        TechnologyRecordChange::Update {
            expected_version, ..
        } => {
            if matches!(
                value,
                TechnologyRecordPayload::ExperimentAttempt(_)
                    | TechnologyRecordPayload::AttemptObservation(_)
                    | TechnologyRecordPayload::TechnicalClaim(_)
                    | TechnologyRecordPayload::ClaimAssessment(_)
                    | TechnologyRecordPayload::ProductionRun(_)
                    | TechnologyRecordPayload::TechniqueRevision(_)
            ) {
                return Err(invalid("immutable technology evidence cannot be updated"));
            }
            let existing = state.records.get(&draft.reference).ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::DomainRecordNotFound,
                    format!("technology record {} was not found", draft.reference),
                )
            })?;
            let existing_payload = crate::query::decode_runtime_payload(existing)?
                .ok_or_else(|| invalid("runtime update targets a non-runtime technology record"))?;
            if let (
                TechnologyRecordPayload::Transmission(previous),
                TechnologyRecordPayload::Transmission(next),
            ) = (&existing_payload, value)
            {
                if !previous.active && next.active {
                    return Err(invalid(
                        "a closed transmission opportunity cannot be reopened; create a new opportunity from a current source capability",
                    ));
                }
                let mut previous = previous.clone();
                previous.active = next.active;
                if previous != *next {
                    return Err(invalid(
                        "transmission updates may change only the active flag",
                    ));
                }
            }
            if existing_payload.authority_subject() != value.authority_subject() {
                return Err(CanwuError::new(
                    ErrorCode::InvalidAuthority,
                    "technology record ownership cannot change through a normal update",
                ));
            }
            if existing.version != *expected_version {
                return Err(CanwuError::new(
                    ErrorCode::DomainRecordVersionConflict,
                    "technology record expected version is stale",
                ));
            }
            (
                expected_version
                    .checked_add(1)
                    .ok_or_else(|| invalid("record version overflow"))?,
                DomainRecordMutation::Update {
                    record: draft.clone(),
                    expected_version: *expected_version,
                },
            )
        }
    };
    Ok((
        DomainRecord {
            reference: draft.reference,
            owner: PLUGIN_NAME.to_owned(),
            class: DomainRecordClass::Record,
            version,
            lifecycle: DomainRecordLifecycle::Active,
            payload: draft.payload,
            references: draft.references,
        },
        mutation,
    ))
}

fn publication(value: &TechnologyRecordPayload) -> Option<(&KnowledgeHolderRef, &'static str)> {
    match value {
        TechnologyRecordPayload::TechnicalClaim(value) => {
            Some((&value.asserted_by, CLAIM_KNOWLEDGE))
        }
        TechnologyRecordPayload::AttemptObservation(value) => {
            Some((&value.observer, ATTEMPT_KNOWLEDGE))
        }
        TechnologyRecordPayload::Capability(value) => Some((&value.holder, CAPABILITY_KNOWLEDGE)),
        TechnologyRecordPayload::Implementation(value) => {
            Some((&value.owner, IMPLEMENTATION_KNOWLEDGE))
        }
        TechnologyRecordPayload::Adoption(value) => Some((&value.adopter, ADOPTION_KNOWLEDGE)),
        _ => None,
    }
}

fn knowledge_draft(
    schema: &str,
    version: &canwu_api::DomainRecordVersionRef,
    value: &TechnologyRecordPayload,
    at: canwu_api::SimTime,
) -> Result<KnowledgeRecordDraft, CanwuError> {
    Ok(KnowledgeRecordDraft {
        schema: KnowledgeSchemaId::new(KnowledgeRecordKind::new(PLUGIN_NAMESPACE, schema), 1),
        subjects: vec![KnowledgeSubject {
            role: "record".to_owned(),
            target: KnowledgeSubjectTarget::DomainRecord(version.record.clone()),
        }],
        payload: json!({
            "record_version": version.version,
            "record": serde_json::to_value(value).map_err(|error| encoding_error(&error))?,
        }),
        as_of: Some(at),
        confidence_per_mille: 1_000,
        origin: KnowledgeOrigin {
            method: "technology_record_evidence_v1".to_owned(),
            evidence: vec![EvidenceRef::DomainRecordVersion(version.clone())],
        },
        supersedes: Vec::new(),
        contradicts: Vec::new(),
    })
}

fn require_subject_authority(
    context: &CommandContext,
    subject: &KnowledgeHolderRef,
) -> Result<(), CanwuError> {
    let authorized = match subject {
        KnowledgeHolderRef::Person(person) => context.issuer == Issuer::Actor(*person),
        KnowledgeHolderRef::Entity(entity) => {
            context.authority.command_subject.as_ref() == Some(entity)
                && context.decision_controller_id.is_some()
        }
    };
    if authorized {
        Ok(())
    } else {
        Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "technology command issuer is not authorized for its subject",
        ))
    }
}

fn operation_ref(id: &str) -> TypedDomainRecordRef<TechnologyOperation> {
    TypedDomainRecordRef::new(id)
}

fn change_id(change: &TechnologyRecordChange) -> &str {
    match change {
        TechnologyRecordChange::Create { id, .. } | TechnologyRecordChange::Update { id, .. } => id,
    }
}

fn change_value(change: &TechnologyRecordChange) -> &TechnologyRecordPayload {
    match change {
        TechnologyRecordChange::Create { value, .. }
        | TechnologyRecordChange::Update { value, .. } => value,
    }
}

fn affected_entities(value: &TechnologyRecordPayload) -> Vec<canwu_api::EntityRef> {
    match value.authority_subject() {
        None => Vec::new(),
        Some(holder) => affected_holder_entities(holder),
    }
}

fn affected_holder_entities(holder: &KnowledgeHolderRef) -> Vec<canwu_api::EntityRef> {
    match holder {
        KnowledgeHolderRef::Person(person) => vec![canwu_api::EntityRef::Person(*person)],
        KnowledgeHolderRef::Entity(entity) => vec![entity.clone()],
    }
}

fn is_expected_domain_rejection(error: &CanwuError) -> bool {
    matches!(
        error.code,
        ErrorCode::InvalidAuthority
            | ErrorCode::InvalidDomainRecord
            | ErrorCode::DomainRecordNotFound
            | ErrorCode::DomainRecordVersionConflict
            | ErrorCode::DuplicateDomainRecord
            | ErrorCode::IdempotencyConflict
    )
}

fn command_payload_schema() -> PayloadSchema {
    object_schema(&[
        ("change", PayloadValueType::Object),
        ("id", PayloadValueType::String),
        ("subject", PayloadValueType::Object),
    ])
}

fn result_payload_schema() -> PayloadSchema {
    let PayloadSchema::Object {
        mut properties,
        allow_additional,
    } = object_schema(&[
        ("change", PayloadValueType::Object),
        ("id", PayloadValueType::String),
        ("provider", PayloadValueType::String),
    ])
    else {
        unreachable!("object_schema always returns an object")
    };
    properties.insert(
        "execution_intent".to_owned(),
        PayloadProperty {
            value_type: PayloadValueType::Object,
            required: false,
        },
    );
    PayloadSchema::Object {
        properties,
        allow_additional,
    }
}

fn object_schema(fields: &[(&str, PayloadValueType)]) -> PayloadSchema {
    PayloadSchema::Object {
        properties: fields
            .iter()
            .map(|(name, value_type)| {
                (
                    (*name).to_owned(),
                    PayloadProperty {
                        value_type: value_type.clone(),
                        required: true,
                    },
                )
            })
            .collect(),
        allow_additional: false,
    }
}

fn decode<T: serde::de::DeserializeOwned>(value: &Value, label: &str) -> Result<T, CanwuError> {
    serde_json::from_value(value.clone()).map_err(|error| {
        CanwuError::new(
            ErrorCode::InvalidPayload,
            format!("{label} could not be decoded: {error}"),
        )
    })
}

fn validate_identifier(value: &str, label: &str) -> Result<(), CanwuError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidPayload,
            format!("{label} identity is not canonical"),
        ));
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidDomainRecord, message)
}

fn encoding_error(error: &serde_json::Error) -> CanwuError {
    CanwuError::new(
        ErrorCode::InvalidPayload,
        format!("technology payload could not be encoded: {error}"),
    )
}

trait ErrorCodeText {
    fn as_str(&self) -> &'static str;
}

impl ErrorCodeText for ErrorCode {
    fn as_str(&self) -> &'static str {
        match self {
            ErrorCode::InvalidDomainRecord => "invalid_domain_record",
            ErrorCode::DomainRecordNotFound => "domain_record_not_found",
            ErrorCode::DomainRecordVersionConflict => "domain_record_version_conflict",
            ErrorCode::DuplicateDomainRecord => "duplicate_domain_record",
            ErrorCode::IdempotencyConflict => "idempotency_conflict",
            _ => "technology_operation_rejected",
        }
    }
}
