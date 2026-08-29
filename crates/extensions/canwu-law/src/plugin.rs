use crate::LegalRuntime;
use crate::{LAW_SEMANTIC_HASH, LegalRuntimeRecord, PLUGIN_NAME, legal_runtime_reference};
use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, Canwu, CanwuError,
    CauseRef, Command, CommandContext, DecisionOrigin, DecisionRequestId, DecisionTicketId,
    DomainRecord, DomainRecordMutation, DomainRecordRef, DomainRecordSchema, EntityRef, ErrorCode,
    EvidenceRef, IDENTITY_EVIDENCE_DEPENDENCIES_FIELD, IdentityEvidenceDependenciesV1,
    IngressClass, IngressPayload, KnowledgeHolderRef, PayloadProperty, PayloadSchema,
    PayloadValueType, PluginActionDescriptor, PluginIngressDescriptor, PluginRegistrar,
    SimulationPlugin, SimulationView, StateKey, StateVisibility, SystemCadence, SystemDirective,
    canonical_hash, identity_evidence_dependencies_property_v1,
};
use serde_json::Value;
use std::collections::BTreeMap;

pub const LAW_COMMAND: &str = "submit_pending_intent";
pub const LAW_RUNTIME_STATE: &str = "runtime";
pub const LAW_INTENT_INGRESS: &str = "pending_legal_intent";
pub const LAW_ACTOR_CONTEXT_INGRESS: &str = "legal_actor_context";
pub const LAW_OUTBOX_ACK_INGRESS: &str = "legal_outbox_enqueued";
pub const LAW_OUTBOX_PREPARE_INGRESS: &str = "prepare_legal_outbox_enqueue";
pub const LAW_MUTATION_INGRESS: &str = "legal_mutation";
pub const LAW_WAKE_INGRESS: &str = "legal_due_work";
const LAW_ADMISSION_SYSTEM: &str = "admit_legal_ingress";

#[derive(Clone, Debug, Eq, PartialEq)]
struct OutboxAcknowledgementAdmission {
    expected_revision: u64,
    ingress_id: canwu_api::IngressId,
    controller_request_id: Option<u64>,
    create_request_id: u64,
    ticket_id: u64,
    draft_hash: String,
    outcome_commitment: String,
}

impl OutboxAcknowledgementAdmission {
    fn semantically_matches(&self, other: &Self) -> bool {
        self.expected_revision == other.expected_revision
            && self.controller_request_id == other.controller_request_id
            && self.create_request_id == other.create_request_id
            && self.ticket_id == other.ticket_id
            && self.draft_hash == other.draft_hash
            && self.outcome_commitment == other.outcome_commitment
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct LawPlugin;

impl SimulationPlugin for LawPlugin {
    fn name(&self) -> &'static str {
        PLUGIN_NAME
    }
    fn version(&self) -> &'static str {
        "0.7.0"
    }
    fn semantic_hash(&self) -> &'static str {
        LAW_SEMANTIC_HASH
    }

    fn validate_activation(&self, records: &[DomainRecord]) -> Result<(), CanwuError> {
        validate_law_activation_records(records)
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let schemas = law_record_schemas();
        for schema in &schemas {
            registrar.register_record_schema(schema.clone())?;
        }
        registrar.register_ingress(PluginIngressDescriptor {
            name: LAW_ACTOR_CONTEXT_INGRESS.to_owned(),
            description: "Derive one legal seat context from holder-relative knowledge".to_owned(),
            class: IngressClass::Information,
            payload_schema: PayloadSchema::Any,
        })?;
        registrar.register_ingress(PluginIngressDescriptor {
            name: LAW_MUTATION_INGRESS.to_owned(),
            description: "Apply one plan-bound legal mutation at an atomic boundary".to_owned(),
            class: IngressClass::ScheduledSystem,
            payload_schema: PayloadSchema::Any,
        })?;
        registrar.register_ingress(PluginIngressDescriptor {
            name: LAW_WAKE_INGRESS.to_owned(),
            description: "Advance indexed legal deadline or effective-time work".to_owned(),
            class: IngressClass::ScheduledSystem,
            payload_schema: PayloadSchema::Any,
        })?;
        registrar.register_ingress(PluginIngressDescriptor {
            name: LAW_INTENT_INGRESS.to_owned(),
            description: "Admit one authority-checked legal decision intent".to_owned(),
            class: IngressClass::ScheduledSystem,
            payload_schema: PayloadSchema::Any,
        })?;
        registrar.register_ingress(PluginIngressDescriptor {
            name: LAW_OUTBOX_ACK_INGRESS.to_owned(),
            description: "Persist successful host enqueue of one legal outbox item".to_owned(),
            class: IngressClass::Acknowledgement,
            payload_schema: PayloadSchema::Any,
        })?;
        registrar.register_ingress(PluginIngressDescriptor {
            name: LAW_OUTBOX_PREPARE_INGRESS.to_owned(),
            description: "Persist the revision for a later legal decision enqueue".to_owned(),
            class: IngressClass::Information,
            payload_schema: PayloadSchema::Any,
        })?;
        registrar.register_command(law_command_descriptor(), submit_pending_intent)?;
        let runtime_state = DomainRecordSchema::for_record::<LegalRuntimeRecord>().state_key();
        let mut admission = canwu_api::BoundarySystemContract::new(
            LAW_ADMISSION_SYSTEM,
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        admission.reads = vec![
            runtime_state.clone(),
            StateKey::core_commands(),
            StateKey::core_ingress(),
            StateKey::core_knowledge(),
            StateKey::core_decisions(),
            StateKey::core_domain_records(),
            StateKey::core_evidence(),
        ];
        admission.writes = vec![runtime_state];
        admission.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(admission, admit_legal_ingress)
    }
}

#[must_use]
pub fn law_command_descriptor() -> PluginActionDescriptor {
    PluginActionDescriptor {
        name: LAW_COMMAND.to_owned(),
        description: "Append one controller-bound pending legal intent".to_owned(),
        payload_schema: PayloadSchema::Object {
            properties: BTreeMap::from([(
                "intent".to_owned(),
                PayloadProperty {
                    value_type: PayloadValueType::Object,
                    required: true,
                },
            )]),
            allow_additional: false,
        },
        reads: Vec::new(),
        writes: Vec::new(),
    }
}

/// Queue one legal mutation through Canwu's canonical ingress.
pub fn enqueue_legal_mutation(
    canwu: &mut Canwu,
    mutation: &crate::LegalMutation,
) -> Result<canwu_api::IngressReceipt, CanwuError> {
    enqueue_legal_mutation_at(canwu, canwu.time(), mutation)
}

/// Queue one legal mutation for a declared future boundary.
pub fn enqueue_legal_mutation_at(
    canwu: &mut Canwu,
    at: canwu_api::SimTime,
    mutation: &crate::LegalMutation,
) -> Result<canwu_api::IngressReceipt, CanwuError> {
    canwu.enqueue_plugin_ingress(canwu_api::PluginIngressRequest::new(
        PLUGIN_NAME,
        LAW_MUTATION_INGRESS,
        at,
        serde_json::json!({ "mutation": mutation }),
    ))
}

fn submit_pending_intent(
    _view: &SimulationView<'_>,
    context: &CommandContext,
    payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    if context.decision_controller_id.is_none() || context.attempt_id.is_none() {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "legal intents require a validated decision controller and attempt",
        ));
    }
    let intent_value = payload.get("intent").cloned().ok_or_else(|| {
        CanwuError::new(
            ErrorCode::InvalidPayload,
            "legal intent command requires intent",
        )
    })?;
    let mut intent =
        serde_json::from_value::<crate::PendingLegalIntent>(intent_value).map_err(|error| {
            CanwuError::new(
                ErrorCode::InvalidPayload,
                format!("invalid pending legal intent: {error}"),
            )
        })?;
    if context.authority.seat_id.as_deref() != Some(intent.seat.as_str())
        || context.authority.permission_profile_id.is_none()
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "pending legal intent seat or permission does not match command authority",
        ));
    }
    let controller = holder_for_origin(&context.authority.decision_origin)?;
    if controller != intent.controller {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "pending legal intent holder does not match the decision origin",
        ));
    }
    intent.command = EvidenceRef::Command(context.command_id);
    intent.attempt = context.attempt_id.map(EvidenceRef::CommandAttempt);
    intent.request_id = context.request_id.map(canwu_api::CommandRequestId::get);
    intent.admitted_at = context.simulation_time;

    let value = serde_json::to_value(intent).map_err(|error| {
        CanwuError::new(
            ErrorCode::InvalidPayload,
            format!("pending legal intent cannot be encoded: {error}"),
        )
    })?;
    Ok(vec![SystemDirective::EnqueuePluginIngress {
        after: canwu_api::SimDuration::ZERO,
        packet_type: LAW_INTENT_INGRESS.to_owned(),
        priority: 0,
        payload: value,
        affected: Vec::new(),
    }])
}

fn admit_legal_ingress(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut intents = Vec::new();
    let mut acknowledgements = BTreeMap::<u64, OutboxAcknowledgementAdmission>::new();
    let mut preparations = BTreeMap::<u64, u64>::new();
    let mut actor_context_queries = Vec::new();
    let mut mutations = Vec::new();
    let mut wake_requested = Vec::new();
    let mut runtime = None;
    let mut collected_mutations = 0;
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
        if ![
            LAW_INTENT_INGRESS,
            LAW_ACTOR_CONTEXT_INGRESS,
            LAW_MUTATION_INGRESS,
            LAW_WAKE_INGRESS,
            LAW_OUTBOX_PREPARE_INGRESS,
            LAW_OUTBOX_ACK_INGRESS,
        ]
        .contains(&packet_type.as_str())
        {
            continue;
        }
        if runtime.is_none() {
            let record = view
                .typed_domain_record(&legal_runtime_reference())?
                .ok_or_else(|| {
                    CanwuError::new(ErrorCode::InvalidDomainRecord, "legal runtime is missing")
                })?;
            runtime = Some(decode_legal_runtime_record(record)?);
        }
        let mutation_budget = runtime
            .as_ref()
            .expect("known legal ingress loaded the runtime")
            .budgets
            .max_mutations_per_boundary;
        if packet_type == LAW_INTENT_INGRESS {
            let intent = serde_json::from_value::<crate::PendingLegalIntent>(payload.clone())
                .map_err(|error| {
                    CanwuError::new(
                        ErrorCode::InvalidPayload,
                        format!("invalid admitted legal intent: {error}"),
                    )
                })?;
            verify_intent_command(view, ingress.cause.as_ref(), &intent)?;
            reserve_collected_mutation(&mut collected_mutations, mutation_budget)?;
            intents.push(intent);
        } else if packet_type == LAW_ACTOR_CONTEXT_INGRESS {
            let requirement = serde_json::from_value::<crate::LegalActorContextRequirement>(
                payload.get("requirement").cloned().ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::InvalidPayload,
                        "legal actor context requires a seat requirement",
                    )
                })?,
            )
            .map_err(|error| {
                CanwuError::new(
                    ErrorCode::InvalidPayload,
                    format!("invalid legal actor context requirement: {error}"),
                )
            })?;
            let query = serde_json::from_value::<canwu_api::KnowledgeQuery>(
                payload.get("query").cloned().ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::InvalidPayload,
                        "legal actor context requires a knowledge query",
                    )
                })?,
            )
            .map_err(|error| {
                CanwuError::new(
                    ErrorCode::InvalidPayload,
                    format!("invalid legal actor knowledge query: {error}"),
                )
            })?;
            reserve_collected_mutation(&mut collected_mutations, mutation_budget)?;
            actor_context_queries.push((requirement, query));
        } else if packet_type == LAW_MUTATION_INGRESS {
            let mutation = serde_json::from_value::<crate::LegalMutation>(
                payload.get("mutation").cloned().ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::InvalidPayload,
                        "legal mutation ingress requires a mutation",
                    )
                })?,
            )
            .map_err(|error| {
                CanwuError::new(
                    ErrorCode::InvalidPayload,
                    format!("invalid legal mutation: {error}"),
                )
            })?;
            reserve_collected_mutation(&mut collected_mutations, mutation_budget)?;
            mutations.push((ingress.id, mutation));
        } else if packet_type == LAW_WAKE_INGRESS {
            let due_at = payload
                .get("due_at")
                .and_then(Value::as_i64)
                .map(canwu_api::SimTime::from_minutes)
                .ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::InvalidPayload,
                        "legal wake ingress requires due_at",
                    )
                })?;
            if due_at > context.at {
                return Err(CanwuError::new(
                    ErrorCode::InvalidPayload,
                    "legal wake cannot be admitted before its due time",
                ));
            }
            reserve_collected_mutation(&mut collected_mutations, mutation_budget)?;
            wake_requested.push(due_at);
        } else if packet_type == LAW_OUTBOX_PREPARE_INGRESS {
            let sequence = payload_u64(payload, "sequence", "legal outbox preparation")?;
            let expected_revision =
                payload_u64(payload, "expected_revision", "legal outbox preparation")?;
            match preparations.entry(sequence) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    reserve_collected_mutation(&mut collected_mutations, mutation_budget)?;
                    entry.insert(expected_revision);
                }
                std::collections::btree_map::Entry::Occupied(entry) => {
                    if *entry.get() != expected_revision {
                        return Err(CanwuError::new(
                            ErrorCode::IdempotencyConflict,
                            "conflicting legal outbox preparations share one sequence",
                        ));
                    }
                }
            }
        } else if packet_type == LAW_OUTBOX_ACK_INGRESS {
            let (sequence, expected_revision) =
                sequence_and_revision(payload, "legal outbox acknowledgement")?;
            let candidate = OutboxAcknowledgementAdmission {
                expected_revision,
                ingress_id: ingress.id,
                controller_request_id: payload_optional_u64(
                    payload,
                    "controller_request_id",
                    "legal outbox acknowledgement",
                )?,
                create_request_id: payload_u64(
                    payload,
                    "create_request_id",
                    "legal outbox acknowledgement",
                )?,
                ticket_id: payload_u64(payload, "ticket_id", "legal outbox acknowledgement")?,
                draft_hash: payload
                    .get("draft_hash")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        CanwuError::new(
                            ErrorCode::InvalidPayload,
                            "legal outbox acknowledgement requires a draft hash",
                        )
                    })?
                    .to_owned(),
                outcome_commitment: payload
                    .get("outcome_commitment")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        CanwuError::new(
                            ErrorCode::InvalidPayload,
                            "legal outbox acknowledgement requires an outcome commitment",
                        )
                    })?
                    .to_owned(),
            };
            match acknowledgements.entry(sequence) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    reserve_collected_mutation(&mut collected_mutations, mutation_budget)?;
                    entry.insert(candidate);
                }
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    if !entry.get().semantically_matches(&candidate) {
                        return Err(CanwuError::new(
                            ErrorCode::IdempotencyConflict,
                            "conflicting legal outbox acknowledgements share one sequence",
                        ));
                    }
                    if candidate.ingress_id < entry.get().ingress_id {
                        entry.get_mut().ingress_id = candidate.ingress_id;
                    }
                }
            }
        }
    }
    if intents.is_empty()
        && acknowledgements.is_empty()
        && preparations.is_empty()
        && actor_context_queries.is_empty()
        && mutations.is_empty()
        && wake_requested.is_empty()
    {
        return Ok(BoundaryProposal::default());
    }
    intents.sort_by(|left, right| left.id.cmp(&right.id));
    actor_context_queries.sort_by(|left, right| {
        (&left.0.procedure, left.0.stage, left.0.round, &left.0.seat).cmp(&(
            &right.0.procedure,
            right.0.stage,
            right.0.round,
            &right.0.seat,
        ))
    });
    let reference = legal_runtime_reference();
    let record = view.typed_domain_record(&reference)?.ok_or_else(|| {
        CanwuError::new(ErrorCode::InvalidDomainRecord, "legal runtime is missing")
    })?;
    let mut runtime = runtime.expect("non-empty legal ingress loaded the runtime");
    let plan = runtime.plan.clone();
    runtime.validate_live_plan_binding(&plan)?;
    wake_requested.sort();
    wake_requested.dedup();
    for due_at in wake_requested {
        runtime.consume_wake(due_at)?;
    }
    let mut signals = Vec::new();
    for (ingress_id, mutation) in mutations {
        verify_legal_mutation(view, &plan, &mutation)?;
        match mutation {
            crate::LegalMutation::SubmitProposal { proposal } => {
                runtime.submit_proposal_within_boundary(&plan, proposal)?;
            }
            crate::LegalMutation::AdmitNonProceduralSource { proposal } => {
                let admitted_signal_kinds = verified_required_signal_kinds(view, &plan, &proposal)?;
                runtime.admit_non_procedural_source_within_boundary(
                    &plan,
                    proposal,
                    &admitted_signal_kinds,
                    context.at,
                )?;
            }
            crate::LegalMutation::Signal { signal } => {
                let proposal = runtime.proposals.get(&signal.proposal_id).ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::InvalidDomainRecord,
                        "legal signal references an unknown proposal",
                    )
                })?;
                verify_proposal_inputs(view, proposal)?;
                let active_procedure = proposal.active_procedure.clone();
                if let Some(procedure) = active_procedure {
                    runtime.dirty_procedures.insert(procedure);
                }
                signals.push(signal);
            }
            crate::LegalMutation::RetireCulturalTarget { target, reason } => runtime
                .retire_cultural_target_from_ingress(
                    &plan,
                    &target,
                    context.at,
                    reason,
                    EvidenceRef::Ingress(ingress_id),
                )?,
            crate::LegalMutation::RecordCase { case } => runtime.record_case(&plan, case)?,
            crate::LegalMutation::RecordFinding { finding } => {
                runtime.record_finding(&plan, finding)?;
            }
            crate::LegalMutation::RecordRuling { ruling } => {
                runtime.record_ruling(&plan, ruling)?;
            }
            crate::LegalMutation::RecordConflict { conflict } => {
                runtime.record_conflict(&plan, conflict)?;
            }
            crate::LegalMutation::RecordPublicity { publicity } => {
                runtime.record_publicity_at(&plan, publicity, context.at)?;
            }
            crate::LegalMutation::RecordSuccession { succession } => {
                runtime.record_succession_for_plan(&plan, succession)?;
            }
            crate::LegalMutation::AdmitCapacity { allocation } => {
                runtime.admit_capacity_allocation(&plan, allocation)?;
            }
        }
    }
    for (requirement, query) in actor_context_queries {
        if query.after.is_some()
            || query.limit == 0
            || usize::try_from(query.limit)
                .ok()
                .is_none_or(|limit| limit > runtime.budgets.max_evidence_per_record)
        {
            return Err(CanwuError::new(
                ErrorCode::KnowledgeLimitExceeded,
                "legal actor context requires one bounded, unpaginated knowledge query",
            ));
        }
        let result = view.knowledge_records(requirement.holder.clone(), &query)?;
        if result.next.is_some() || result.holder != requirement.holder {
            return Err(CanwuError::new(
                ErrorCode::KnowledgeLimitExceeded,
                "legal actor context query must fit in one holder-bound page",
            ));
        }
        let actor_context = crate::runtime::actor_context_from_query_result(&result)?;
        runtime.stage_actor_context(&requirement, actor_context)?;
    }
    for (sequence, expected_revision) in preparations {
        if runtime
            .outbox
            .get(&sequence)
            .and_then(|item| item.enqueue_expected_revision)
            .is_some_and(|existing| existing != expected_revision)
        {
            verify_outbox_can_reprepare(view, &runtime, sequence)?;
        }
        runtime.stage_outbox_expected_revision(sequence, expected_revision)?;
    }
    for (sequence, acknowledgement) in acknowledgements {
        verify_outbox_enqueue(
            view,
            &runtime,
            sequence,
            acknowledgement.expected_revision,
            acknowledgement.controller_request_id,
            acknowledgement.create_request_id,
            acknowledgement.ticket_id,
            &acknowledgement.draft_hash,
            &acknowledgement.outcome_commitment,
        )?;
        runtime.mark_outbox_enqueued(
            sequence,
            acknowledgement.expected_revision,
            EvidenceRef::Ingress(acknowledgement.ingress_id),
            acknowledgement.outcome_commitment,
        )?;
    }
    for intent in intents {
        runtime.queue_authorized_pending_intent(intent)?;
    }
    verify_pending_adoption_guards(view, &runtime)?;
    runtime.settle_boundary(&plan, context.at, &signals)?;
    let mut directives = Vec::new();
    if let Some(due_at) = runtime.next_due_time()
        && !runtime.scheduled_wakes.contains(&due_at)
    {
        let after = due_at.checked_sub(context.at).ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidBoundary,
                "legal due time precedes this boundary",
            )
        })?;
        runtime.mark_wake_scheduled(due_at)?;
        directives.push(BoundaryDirective::ScheduleIngress {
            after,
            packet_type: LAW_WAKE_INGRESS.to_owned(),
            priority: 0,
            payload: serde_json::json!({ "due_at": due_at.as_minutes() }),
            affected: Vec::new(),
        });
    }
    let mut draft = runtime.to_record_draft()?;
    debug_assert_eq!(draft.reference, reference.into_untyped());
    draft.references.clone_from(&record.references);
    directives.insert(
        0,
        BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Update {
                record: draft,
                expected_version: record.version,
            },
            summary: "Persist admitted and settled legal state".to_owned(),
        },
    );
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

fn reserve_collected_mutation(count: &mut usize, budget: usize) -> Result<(), CanwuError> {
    if *count >= budget {
        return Err(CanwuError::new(
            ErrorCode::ValueOutOfRange,
            "legal ingress mutation budget exhausted",
        ));
    }
    *count += 1;
    Ok(())
}

fn payload_u64(payload: &Value, field: &str, context: &str) -> Result<u64, CanwuError> {
    payload.get(field).and_then(Value::as_u64).ok_or_else(|| {
        CanwuError::new(
            ErrorCode::InvalidPayload,
            format!("{context} requires {field}"),
        )
    })
}

fn payload_optional_u64(
    payload: &Value,
    field: &str,
    context: &str,
) -> Result<Option<u64>, CanwuError> {
    match payload.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_u64().map(Some).ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidPayload,
                format!("{context} requires {field} to be an unsigned integer or null"),
            )
        }),
    }
}

fn verify_evidence(
    view: &SimulationView<'_>,
    evidence: impl IntoIterator<Item = EvidenceRef>,
) -> Result<(), CanwuError> {
    for reference in evidence {
        if !view.evidence_exists(&reference)? {
            return Err(CanwuError::new(
                ErrorCode::InvalidAuthority,
                "legal mutation references unavailable evidence",
            ));
        }
    }
    Ok(())
}

fn verify_expected_versions(
    view: &SimulationView<'_>,
    versions: &[canwu_api::DomainRecordVersionRef],
) -> Result<(), CanwuError> {
    for version in versions {
        if !view.domain_record_version_is_current(version)? {
            return Err(CanwuError::new(
                ErrorCode::InvalidAuthority,
                "legal proposal host record compare-and-set failed",
            ));
        }
    }
    Ok(())
}

fn verify_proposal_inputs(
    view: &SimulationView<'_>,
    proposal: &crate::LegalProposal,
) -> Result<(), CanwuError> {
    verify_expected_versions(view, &proposal.expected_versions)?;
    verify_evidence(view, proposal.evidence.iter().cloned())?;
    if proposal
        .cultural_dependencies
        .iter()
        .any(|dependency| !proposal.evidence.contains(&dependency.evidence))
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "legal culture dependency lacks admitted proposal evidence",
        ));
    }
    Ok(())
}

fn verify_legal_mutation(
    view: &SimulationView<'_>,
    plan: &crate::CompiledLawPlan,
    mutation: &crate::LegalMutation,
) -> Result<(), CanwuError> {
    match mutation {
        crate::LegalMutation::SubmitProposal { proposal }
        | crate::LegalMutation::AdmitNonProceduralSource { proposal } => {
            verify_proposal_inputs(view, proposal)
        }
        crate::LegalMutation::Signal { signal } => {
            verify_evidence(view, signal.evidence.iter().cloned())?;
            verify_signal_provenance(view, plan, &signal.kind, &signal.evidence)
        }
        crate::LegalMutation::RetireCulturalTarget { .. }
        | crate::LegalMutation::RecordConflict { .. } => Ok(()),
        crate::LegalMutation::RecordCase { case } => {
            verify_evidence(view, case.allegations.iter().cloned())
        }
        crate::LegalMutation::RecordFinding { finding } => {
            verify_evidence(view, finding.evidence.iter().cloned())
        }
        crate::LegalMutation::RecordRuling { ruling } => {
            verify_evidence(view, ruling.evidence.iter().cloned())
        }
        crate::LegalMutation::RecordPublicity { publicity } => {
            verify_evidence(view, publicity.evidence.iter().cloned())?;
            verify_publicity_provenance(view, plan, publicity)
        }
        crate::LegalMutation::RecordSuccession { succession } => {
            verify_evidence(view, succession.evidence.iter().cloned())
        }
        crate::LegalMutation::AdmitCapacity { allocation } => {
            verify_evidence(view, [allocation.evidence.clone()])
        }
    }
}

fn verified_required_signal_kinds(
    view: &SimulationView<'_>,
    plan: &crate::CompiledLawPlan,
    proposal: &crate::LegalProposal,
) -> Result<Vec<String>, CanwuError> {
    let profile = plan
        .source_profile_by_id
        .get(&proposal.source_profile)
        .and_then(|key| plan.source_profiles.get(key.get() as usize))
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidDomainRecord,
                "non-procedural source profile is missing",
            )
        })?;
    for kind in &profile.required_signal_kinds {
        verify_signal_provenance(view, plan, kind, &proposal.evidence)?;
    }
    Ok(profile.required_signal_kinds.clone())
}

fn verify_signal_provenance(
    view: &SimulationView<'_>,
    plan: &crate::CompiledLawPlan,
    kind: &str,
    evidence: &[EvidenceRef],
) -> Result<(), CanwuError> {
    let provider = plan.signal_provider_by_kind.get(kind).ok_or_else(|| {
        CanwuError::new(
            ErrorCode::InvalidAuthority,
            format!("legal signal kind {kind} has no compiled provider"),
        )
    })?;
    for evidence in evidence {
        if let EvidenceRef::Ingress(id) = evidence
            && view.plugin_ingress_matches(*id, &provider.plugin, &provider.packet_type)?
        {
            return Ok(());
        }
    }
    Err(CanwuError::new(
        ErrorCode::InvalidAuthority,
        format!(
            "legal signal kind {kind} lacks ingress from provider {}:{}",
            provider.plugin, provider.packet_type
        ),
    ))
}

fn verify_publicity_provenance(
    view: &SimulationView<'_>,
    plan: &crate::CompiledLawPlan,
    publicity: &crate::LegalPublicityEvent,
) -> Result<(), CanwuError> {
    let provider = plan
        .signal_provider_by_kind
        .get(&publicity.signal_kind)
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidAuthority,
                format!(
                    "legal publicity signal kind {} has no compiled provider",
                    publicity.signal_kind
                ),
            )
        })?;
    let expected_payload = serde_json::json!({
        "proposal": publicity.proposal.id,
        "at": publicity.at.as_minutes(),
        "signal_kind": publicity.signal_kind,
        "medium": publicity.medium,
        "scope": publicity.scope,
    });
    for evidence in &publicity.evidence {
        if let EvidenceRef::Ingress(id) = evidence
            && view.plugin_ingress_payload_matches(
                *id,
                &provider.plugin,
                &provider.packet_type,
                publicity.at,
                &expected_payload,
            )?
        {
            return Ok(());
        }
    }
    Err(CanwuError::new(
        ErrorCode::InvalidAuthority,
        format!(
            "legal publicity {} lacks an exact retained provider payload from {}:{}",
            publicity.id, provider.plugin, provider.packet_type
        ),
    ))
}

fn verify_pending_adoption_guards(
    view: &SimulationView<'_>,
    runtime: &LegalRuntime,
) -> Result<(), CanwuError> {
    for procedure_id in &runtime.dirty_procedures {
        let Some(proposal) = runtime
            .procedures
            .get(procedure_id)
            .and_then(|procedure| runtime.proposals.get(&procedure.proposal.id))
        else {
            continue;
        };
        verify_proposal_inputs(view, proposal)?;
        if proposal.cultural_dependencies.iter().any(|dependency| {
            runtime
                .retired_cultural_targets
                .contains(&dependency.target)
        }) {
            return Err(CanwuError::new(
                ErrorCode::InvalidAuthority,
                "legal adoption depends on a retired culture generation",
            ));
        }
    }
    Ok(())
}

fn verify_outbox_can_reprepare(
    view: &SimulationView<'_>,
    runtime: &LegalRuntime,
    sequence: u64,
) -> Result<(), CanwuError> {
    let item = runtime.outbox.get(&sequence).ok_or_else(|| {
        CanwuError::new(
            ErrorCode::InvalidDomainRecord,
            "legal outbox preparation references an unknown sequence",
        )
    })?;
    let controller_request_id = item.refresh_request_id.ok_or_else(|| {
        CanwuError::new(
            ErrorCode::InvalidDomainRecord,
            "legal outbox controller request ID is missing",
        )
    })?;
    if view
        .decision_attempt(DecisionRequestId::new(controller_request_id))?
        .is_some()
        || view
            .decision_attempt(DecisionRequestId::new(item.create_request_id))?
            .is_some()
        || view
            .decision_ticket(DecisionTicketId::new(item.ticket_id))?
            .is_some()
    {
        return Err(CanwuError::new(
            ErrorCode::IdempotencyConflict,
            "legal outbox cannot reprepare after a core decision outcome exists",
        ));
    }
    if view
        .decision_controller(&item.decision_controller_id)?
        .is_some_and(
            |controller| match crate::runtime::expected_decision_controller(item) {
                Ok(expected) => controller != &expected,
                Err(_) => true,
            },
        )
    {
        return Err(CanwuError::new(
            ErrorCode::IdempotencyConflict,
            "legal outbox controller binding conflicts with the persisted draft",
        ));
    }
    Ok(())
}

fn sequence_and_revision(payload: &Value, context: &str) -> Result<(u64, u64), CanwuError> {
    Ok((
        payload_u64(payload, "sequence", context)?,
        payload_u64(payload, "expected_revision", context)?,
    ))
}

#[allow(clippy::too_many_arguments)]
fn verify_outbox_enqueue(
    view: &SimulationView<'_>,
    runtime: &LegalRuntime,
    sequence: u64,
    expected_revision: u64,
    controller_request_id_from_payload: Option<u64>,
    create_request_id: u64,
    ticket_id: u64,
    draft_hash: &str,
    outcome_commitment: &str,
) -> Result<(), CanwuError> {
    let item = runtime.outbox.get(&sequence).ok_or_else(|| {
        CanwuError::new(
            ErrorCode::InvalidDomainRecord,
            "legal outbox acknowledgement references an unknown sequence",
        )
    })?;
    let controller_request_id = item.refresh_request_id.ok_or_else(|| {
        CanwuError::new(
            ErrorCode::InvalidAuthority,
            "legal outbox controller request ID is missing",
        )
    })?;
    let controller_attempt =
        view.decision_attempt(DecisionRequestId::new(controller_request_id))?;
    let open_attempt = view
        .decision_attempt(DecisionRequestId::new(create_request_id))?
        .ok_or_else(|| CanwuError::new(ErrorCode::InvalidAuthority, "ticket request missing"))?;
    let controller = view
        .decision_controller(&item.decision_controller_id)?
        .ok_or_else(|| CanwuError::new(ErrorCode::InvalidAuthority, "controller missing"))?;
    let ticket = view
        .decision_ticket(DecisionTicketId::new(ticket_id))?
        .ok_or_else(|| CanwuError::new(ErrorCode::InvalidAuthority, "ticket missing"))?;
    crate::runtime::verify_accepted_outbox_state(
        item,
        expected_revision,
        controller_attempt,
        open_attempt,
        controller,
        ticket,
    )?;
    let expected_outcome_commitment = crate::runtime::outbox_outcome_commitment(
        controller_attempt,
        open_attempt,
        controller,
        ticket,
    )?;
    let exact_hash = canonical_hash("canwu.law.decision-draft.v1", &item.draft)?;
    if item.enqueue_expected_revision != Some(expected_revision)
        || controller_attempt.map(|attempt| attempt.request_id.get())
            != controller_request_id_from_payload
        || item.create_request_id != create_request_id
        || item.ticket_id != ticket_id
        || exact_hash != draft_hash
        || outcome_commitment != expected_outcome_commitment
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "legal outbox acknowledgement does not match the persisted enqueue request",
        ));
    }
    Ok(())
}

fn verify_intent_command(
    view: &SimulationView<'_>,
    cause: Option<&CauseRef>,
    intent: &crate::PendingLegalIntent,
) -> Result<(), CanwuError> {
    let Some(CauseRef::Command(command_id)) = cause else {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "legal intent ingress lacks command evidence",
        ));
    };
    let command = view.command(*command_id)?.ok_or_else(|| {
        CanwuError::new(
            ErrorCode::InvalidAuthority,
            "legal intent command evidence is unavailable",
        )
    })?;
    let Command::Plugin {
        plugin,
        command: command_name,
        payload,
    } = &command.envelope.command
    else {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "legal intent evidence is not a plugin command",
        ));
    };
    let mut authored = payload
        .get("intent")
        .cloned()
        .ok_or_else(|| CanwuError::new(ErrorCode::InvalidPayload, "legal intent is missing"))
        .and_then(|value| {
            serde_json::from_value::<crate::PendingLegalIntent>(value).map_err(|error| {
                CanwuError::new(
                    ErrorCode::InvalidPayload,
                    format!("invalid command-evidence legal intent: {error}"),
                )
            })
        })?;
    authored.command.clone_from(&intent.command);
    authored.attempt.clone_from(&intent.attempt);
    authored.request_id = intent.request_id;
    authored.admitted_at = intent.admitted_at;
    if plugin != PLUGIN_NAME || command_name != LAW_COMMAND || &authored != intent {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "legal intent ingress does not match its authorized command",
        ));
    }
    Ok(())
}

fn holder_for_origin(origin: &DecisionOrigin) -> Result<KnowledgeHolderRef, CanwuError> {
    match origin {
        DecisionOrigin::Actor { actor } => Ok(KnowledgeHolderRef::Person(*actor)),
        DecisionOrigin::Institution { institution, .. } => {
            Ok(KnowledgeHolderRef::Entity(institution.clone()))
        }
        DecisionOrigin::Council { council_id } => {
            Ok(KnowledgeHolderRef::Entity(EntityRef::Domain(
                DomainRecordRef::new(crate::PLUGIN_NAMESPACE, "council", council_id),
            )))
        }
        DecisionOrigin::NoResponsibleActor { .. } => Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "legal intent requires a responsible controller",
        )),
    }
}

#[must_use]
pub fn law_record_schemas() -> Vec<DomainRecordSchema> {
    let mut runtime = DomainRecordSchema::for_record::<LegalRuntimeRecord>();
    runtime.payload_schema = PayloadSchema::Object {
        properties: BTreeMap::from([(
            IDENTITY_EVIDENCE_DEPENDENCIES_FIELD.to_owned(),
            identity_evidence_dependencies_property_v1(),
        )]),
        allow_additional: true,
    };
    vec![runtime]
}

fn decode_legal_runtime_record(record: &DomainRecord) -> Result<LegalRuntime, CanwuError> {
    let runtime = record.decode_payload::<LegalRuntimeRecord>()?;
    let declaration = record
        .payload
        .get(IDENTITY_EVIDENCE_DEPENDENCIES_FIELD)
        .cloned()
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidDomainRecord,
                "law runtime is missing its identity-evidence declaration",
            )
        })?;
    let declaration = serde_json::from_value::<IdentityEvidenceDependenciesV1>(declaration)
        .map_err(|error| {
            CanwuError::new(
                ErrorCode::InvalidDomainRecord,
                format!("law identity-evidence declaration is invalid: {error}"),
            )
        })?;
    if declaration != runtime.identity_evidence_dependencies() {
        return Err(CanwuError::new(
            ErrorCode::InvalidDomainRecord,
            "law identity-evidence declaration does not match the live ledger",
        ));
    }
    Ok(runtime)
}

fn encoded_runtime_payload_len(record: &DomainRecord) -> Result<usize, CanwuError> {
    serde_json::to_vec(&record.payload)
        .map(|encoded| encoded.len())
        .map_err(|error| CanwuError::new(ErrorCode::InvalidDomainRecord, error.to_string()))
}

/// Load the plugin-owned runtime record and fail closed on a plan mismatch.
pub fn load_legal_runtime(
    canwu: &Canwu,
    plan: &crate::CompiledLawPlan,
) -> Result<Option<LegalRuntime>, CanwuError> {
    validate_law_records(canwu)?;
    let Some(record) = canwu.typed_domain_record(&legal_runtime_reference()) else {
        return Ok(None);
    };
    let encoded_bytes = encoded_runtime_payload_len(record)?;
    if encoded_bytes > plan.budgets.max_state_bytes || encoded_bytes > plan.budgets.max_memory_bytes
    {
        return Err(CanwuError::new(
            ErrorCode::ValueOutOfRange,
            "law runtime serialized-state budget exceeded before decode",
        ));
    }
    let runtime = decode_legal_runtime_record(record)?;
    runtime.validate_against_plan(plan)?;
    Ok(Some(runtime))
}

/// Decode every law-owned record before exposing the plan-bound runtime.
pub fn load_law_state_for_plan(
    canwu: &Canwu,
    plan: &crate::CompiledLawPlan,
) -> Result<Option<LegalRuntime>, CanwuError> {
    load_legal_runtime(canwu, plan)
}

fn validate_law_records(canwu: &Canwu) -> Result<(), CanwuError> {
    let expected_runtime = legal_runtime_reference().into_untyped();
    for record in canwu
        .domain_records()
        .filter(|record| record.reference.kind.namespace == crate::PLUGIN_NAMESPACE)
    {
        match record.reference.kind.name.as_str() {
            LAW_RUNTIME_STATE => {
                if record.reference != expected_runtime {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidDomainRecord,
                        "law runtime must use the canonical aggregate record identity",
                    ));
                }
                // Decode is intentionally deferred until the plan-bound loader
                // can reject an oversized payload before materializing it.
            }
            unknown => {
                return Err(CanwuError::new(
                    ErrorCode::InvalidDomainRecord,
                    format!("unrecognized law-owned record kind {unknown}"),
                ));
            }
        }
    }
    Ok(())
}

fn validate_law_activation_records(records: &[DomainRecord]) -> Result<(), CanwuError> {
    let expected_runtime = legal_runtime_reference().into_untyped();
    let mut runtime_seen = false;
    for record in records
        .iter()
        .filter(|record| record.reference.kind.namespace == crate::PLUGIN_NAMESPACE)
    {
        match record.reference.kind.name.as_str() {
            LAW_RUNTIME_STATE => {
                if runtime_seen || record.reference != expected_runtime {
                    return Err(CanwuError::new(
                        ErrorCode::InvalidDomainRecord,
                        "law runtime must be one canonical aggregate record",
                    ));
                }
                runtime_seen = true;
                if encoded_runtime_payload_len(record)? > crate::MAX_LEGAL_STATE_BYTES {
                    return Err(CanwuError::new(
                        ErrorCode::ValueOutOfRange,
                        "law runtime exceeds the absolute activation payload ceiling",
                    ));
                }
                let runtime = decode_legal_runtime_record(record)?;
                runtime.validate_against_plan(&runtime.plan)?;
            }
            unknown => {
                return Err(CanwuError::new(
                    ErrorCode::InvalidDomainRecord,
                    format!("unrecognized law-owned record kind {unknown}"),
                ));
            }
        }
    }
    Ok(())
}
