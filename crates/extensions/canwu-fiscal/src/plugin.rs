use crate::derive::{compute_aggregates, compute_projections, compute_transition_candidates};
use crate::model::{
    CompiledFiscalCatalog, FiscalAction, FiscalActionDisposition, FiscalActionOutcome,
    FiscalActionRequest, FiscalAdoptionStage, FiscalAdoptionState, FiscalAssessment,
    FiscalAuditFinding, FiscalCatalogRecord, FiscalExecutionEvidence, FiscalExecutionReceipt,
    FiscalExecutionReceiptPacket, FiscalExecutionRequest, FiscalExternalOperationRef,
    FiscalHistoricalContextPacket, FiscalReceiptDisposition, FiscalRemission, FiscalState,
    FiscalStateRecord, MAX_FISCAL_ACTION_OUTCOMES, MAX_FISCAL_EVIDENCE_PER_RECORD,
    fiscal_state_reference, invalid, validate_identifier,
};
use crate::projection::{load_fiscal_catalog, load_fiscal_state};
use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundarySystemContract,
    Canwu, CanwuError, CauseRef, Command, CommandContext, CommandIngress, DecisionOrigin,
    DomainRecord, DomainRecordKind, DomainRecordMutation, DomainRecordMutationPolicy,
    DomainRecordSchema, DomainRecordType, DomainReferenceSchema, DomainReferenceTargetKind,
    ErrorCode, EvidenceRef, IngressClass, IngressPayload, Issuer, KnowledgeOrigin,
    KnowledgeRecordDraft, KnowledgeRecordId, KnowledgeRecordKind, KnowledgeSchemaId,
    KnowledgeSubject, KnowledgeSubjectSchema, KnowledgeSubjectTarget, KnowledgeSubjectTargetKind,
    KnowledgeWriteGrant, PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD, PayloadSchema,
    PluginActionDescriptor, PluginIngressDescriptor, PluginIngressRequest, PluginKnowledgeSchema,
    PluginRegistrar, SimTime, SimulationPlugin, SimulationView, StateKey, StateVisibility,
    SystemCadence, SystemDirective, payload_required_evidence_continuation_property_v1,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PLUGIN_NAME: &str = "canwu-fiscal";
pub const APPLY_FISCAL_ACTION_COMMAND: &str = "apply_fiscal_action_v1";
pub const FISCAL_ACTION_INGRESS: &str = "fiscal_action_v1";
pub const FISCAL_EXECUTION_RECEIPT_INGRESS: &str = "fiscal_execution_receipt_v1";
pub const FISCAL_HISTORICAL_CONTEXT_INGRESS: &str = "fiscal_historical_context_v1";

const PLUGIN_VERSION: &str = "0.1.0-experimental";
const SEMANTIC_HASH: &str = "4326696d6dd9771fd696d080747250b132e6e128edd601bd0eac99dc6a86384f";
const FISCAL_REPORT_KNOWLEDGE: &str = "fiscal_report";
const FISCAL_REPORT_SCHEMA_HASH: &str =
    "820036a60b05e071d4833590800432f6b0d2a1c0fa89b19e813ebe7131e1a14a";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct AdmittedFiscalAction {
    request: FiscalActionRequest,
    command: canwu_api::CommandId,
}

struct ValidatedExecutionEvidence {
    quantity: u64,
    disposition: FiscalReceiptDisposition,
    external_operations: std::collections::BTreeSet<FiscalExternalOperationRef>,
}

#[derive(Clone, Debug, Default)]
pub struct FiscalPlugin {
    external_evidence_kinds: Vec<DomainRecordKind>,
}

impl FiscalPlugin {
    #[must_use]
    pub fn new(external_evidence_kinds: impl IntoIterator<Item = DomainRecordKind>) -> Self {
        let mut external_evidence_kinds: Vec<_> = external_evidence_kinds.into_iter().collect();
        external_evidence_kinds.sort();
        external_evidence_kinds.dedup();
        Self {
            external_evidence_kinds,
        }
    }

    fn external_evidence_state_keys(&self) -> Vec<StateKey> {
        self.external_evidence_kinds
            .iter()
            .map(|kind| StateKey::new(kind.namespace.clone(), kind.name.clone()))
            .collect()
    }
}

impl SimulationPlugin for FiscalPlugin {
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
        register_fiscal_record_schemas(registrar)?;
        let report_schema = register_fiscal_report_schema(registrar)?;

        let mut action_reads = vec![StateKey::core_evidence(), catalog_key(), state_key()];
        action_reads.extend(self.external_evidence_state_keys());
        action_reads.sort();
        action_reads.dedup();
        registrar.register_command(
            PluginActionDescriptor {
                name: APPLY_FISCAL_ACTION_COMMAND.to_owned(),
                description: "Admit one authority-bound fiscal procedure action".to_owned(),
                payload_schema: PayloadSchema::Any,
                reads: action_reads,
                writes: Vec::new(),
            },
            admit_fiscal_action,
        )?;
        for (name, description, class) in [
            (
                FISCAL_ACTION_INGRESS,
                "Settle one admitted fiscal procedure action",
                IngressClass::Decision,
            ),
            (
                FISCAL_EXECUTION_RECEIPT_INGRESS,
                "Record externally fulfilled fiscal execution evidence",
                IngressClass::Acknowledgement,
            ),
            (
                FISCAL_HISTORICAL_CONTEXT_INGRESS,
                "Advance the explicit fiscal historical context",
                IngressClass::ScheduledSystem,
            ),
        ] {
            registrar.register_ingress(PluginIngressDescriptor {
                name: name.to_owned(),
                description: description.to_owned(),
                class,
                payload_schema: PayloadSchema::Any,
            })?;
        }

        let mut settle = BoundarySystemContract::new(
            "settle-fiscal-ingress-v1",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        settle.reads = vec![
            StateKey::core_commands(),
            StateKey::core_evidence(),
            StateKey::core_ingress(),
            catalog_key(),
            state_key(),
        ];
        settle.reads.extend(self.external_evidence_state_keys());
        settle.reads.sort();
        settle.reads.dedup();
        settle.writes = vec![state_key()];
        settle.emits = vec![
            "canwu.fiscal.action_settled.v1".to_owned(),
            "canwu.fiscal.execution_receipt_recorded.v1".to_owned(),
            "canwu.fiscal.historical_context_changed.v1".to_owned(),
        ];
        settle.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(settle, settle_fiscal_ingress)?;

        register_derived_system(
            registrar,
            "evaluate-fiscal-transition-candidates-v1",
            BoundaryPhase::HistoricalCandidateEvaluation,
            evaluate_candidates,
        )?;
        register_derived_system(
            registrar,
            "aggregate-fiscal-state-v1",
            BoundaryPhase::StrategicAggregation,
            aggregate_state,
        )?;
        let mut reports = BoundarySystemContract::new(
            "materialize-fiscal-reports-v1",
            BoundaryPhase::PerspectiveAndReportMaterialization,
            SystemCadence::EventDriven,
        );
        reports.reads = vec![StateKey::core_knowledge(), catalog_key(), state_key()];
        reports.knowledge_writes = vec![KnowledgeWriteGrant {
            schema: report_schema.id,
            visibilities: vec![StateVisibility::SameBoundary],
        }];
        reports.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(reports, materialize_reports)
    }
}

fn register_fiscal_record_schemas(registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
    let mut catalog_schema = DomainRecordSchema::for_record::<FiscalCatalogRecord>();
    catalog_schema.mutation_policy = DomainRecordMutationPolicy::CreateOnly;
    registrar.register_record_schema(catalog_schema)?;
    let mut state_schema = DomainRecordSchema::for_record::<FiscalStateRecord>();
    state_schema.payload_schema = PayloadSchema::Object {
        properties: std::collections::BTreeMap::from([(
            PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD.to_owned(),
            payload_required_evidence_continuation_property_v1(),
        )]),
        allow_additional: true,
    };
    state_schema.references = vec![DomainReferenceSchema {
        role: "catalog".to_owned(),
        targets: vec![DomainReferenceTargetKind::Domain(
            DomainRecordKind::for_type::<FiscalCatalogRecord>(),
        )],
        required: true,
        multiple: false,
        allow_retired: false,
    }];
    registrar.register_record_schema(state_schema)
}

fn register_fiscal_report_schema(
    registrar: &mut PluginRegistrar<'_>,
) -> Result<PluginKnowledgeSchema, CanwuError> {
    let schema = fiscal_report_knowledge_schema();
    registrar.register_knowledge_schema(schema.clone())?;
    Ok(schema)
}

fn register_derived_system(
    registrar: &mut PluginRegistrar<'_>,
    name: &str,
    phase: BoundaryPhase,
    handler: canwu_api::BoundarySystemHandler,
) -> Result<(), CanwuError> {
    let mut contract = BoundarySystemContract::new(name, phase, SystemCadence::EventDriven);
    contract.reads = vec![catalog_key(), state_key()];
    contract.writes = vec![state_key()];
    contract.visibility = StateVisibility::SameBoundary;
    registrar.register_boundary_system(contract, handler)
}

fn admit_fiscal_action(
    view: &SimulationView<'_>,
    context: &CommandContext,
    payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    if context.ingress == CommandIngress::LegacyDirect {
        return Err(CanwuError::new(
            ErrorCode::MixedCommandIngress,
            "fiscal actions require tracked command ingress",
        ));
    }
    let request: FiscalActionRequest = decode(payload, "fiscal action")?;
    validate_identifier(&request.action_id, "fiscal action")?;
    validate_identifier(&request.authority_binding_id, "fiscal authority binding")?;
    let Some((_, catalog)) = load_fiscal_catalog(view)? else {
        return Err(missing("fiscal catalog is not configured"));
    };
    let Some((_, state)) = load_fiscal_state(view, &catalog)? else {
        return Err(missing("fiscal state is not configured"));
    };
    if request.expected_procedure_revision != state.procedure_revision {
        return Err(CanwuError::new(
            ErrorCode::DomainRecordVersionConflict,
            "fiscal action expected a stale procedure revision",
        ));
    }
    if state.action_outcomes.contains_key(&request.action_id) {
        return Err(CanwuError::new(
            ErrorCode::IdempotencyConflict,
            "fiscal action identity was already settled",
        ));
    }
    ensure_action_outcome_capacity(&state, 1)?;
    let binding = state
        .authority_bindings
        .get(&request.authority_binding_id)
        .ok_or_else(|| invalid_authority("fiscal authority binding is unavailable"))?;
    validate_authority(context, binding)?;
    validate_action_scope_authority(&state, &request.action, &binding.institution)?;
    if let FiscalAction::OpenAssessment {
        commutation_quote: Some(reference),
        ..
    } = &request.action
        && !view.domain_record_version_evidence_exists(reference)?
    {
        return Err(invalid(
            "fiscal commutation requires an exact available quote record version",
        ));
    }
    Ok(vec![SystemDirective::EnqueuePluginIngress {
        after: canwu_api::SimDuration::ZERO,
        packet_type: FISCAL_ACTION_INGRESS.to_owned(),
        priority: 0,
        payload: serde_json::to_value(AdmittedFiscalAction {
            request,
            command: context.command_id,
        })
        .map_err(encode_error)?,
        affected: vec![binding.institution.clone()],
    }])
}

fn validate_action_scope_authority(
    state: &FiscalState,
    action: &FiscalAction,
    institution: &canwu_api::EntityRef,
) -> Result<(), CanwuError> {
    if let FiscalAction::ApplyTransition {
        target_scope_bindings,
        ..
    } = action
    {
        if target_scope_bindings.is_empty()
            || target_scope_bindings.values().any(|scope_id| {
                state
                    .scope_bindings
                    .get(scope_id)
                    .is_none_or(|scope| &scope.institution != institution)
            })
        {
            return Err(invalid_authority(
                "fiscal transition authority must own every target scope",
            ));
        }
        return Ok(());
    }
    let scope_id = match action {
        FiscalAction::ChangeAdoption {
            scope_binding_id, ..
        }
        | FiscalAction::OpenAssessment {
            scope_binding_id, ..
        } => scope_binding_id.as_str(),
        FiscalAction::GrantRemission { assessment_id, .. }
        | FiscalAction::AuthorizeExecution { assessment_id, .. } => state
            .assessments
            .get(assessment_id)
            .map(|assessment| assessment.scope_binding_id.as_str())
            .ok_or_else(|| invalid("fiscal action names an unknown assessment"))?,
        FiscalAction::RecordAudit { target_id, .. } => {
            if let Some(assessment) = state.assessments.get(target_id) {
                assessment.scope_binding_id.as_str()
            } else if let Some(request) = state.execution_requests.get(target_id) {
                state
                    .assessments
                    .get(&request.assessment_id)
                    .map(|assessment| assessment.scope_binding_id.as_str())
                    .ok_or_else(|| invalid("fiscal audit request lost its assessment"))?
            } else if let Some(receipt) = state.execution_receipts.get(target_id) {
                let request = state
                    .execution_requests
                    .get(&receipt.request_id)
                    .ok_or_else(|| invalid("fiscal audit receipt lost its request"))?;
                state
                    .assessments
                    .get(&request.assessment_id)
                    .map(|assessment| assessment.scope_binding_id.as_str())
                    .ok_or_else(|| invalid("fiscal audit receipt lost its assessment"))?
            } else {
                return Err(invalid("fiscal audit target is unavailable"));
            }
        }
        FiscalAction::ApplyTransition { .. } => unreachable!("handled above"),
    };
    let scope = state
        .scope_bindings
        .get(scope_id)
        .ok_or_else(|| invalid("fiscal action scope is unavailable"))?;
    if &scope.institution != institution {
        return Err(invalid_authority(
            "fiscal authority binding does not own the action scope",
        ));
    }
    Ok(())
}

fn validate_authority(
    context: &CommandContext,
    binding: &crate::FiscalAuthorityBinding,
) -> Result<(), CanwuError> {
    match (&context.issuer, context.decision_controller_id.as_deref()) {
        (Issuer::Actor(actor), None) if binding.authorized_actor == Some(*actor) => Ok(()),
        (Issuer::Human(issuer) | Issuer::Ai(issuer), Some(controller)) if issuer == controller => {
            if context.authority.command_subject.as_ref() != Some(&binding.institution) {
                return Err(invalid_authority(
                    "fiscal decision subject does not match the bound institution",
                ));
            }
            match (&context.authority.decision_origin, binding.authorized_actor) {
                (DecisionOrigin::Actor { actor }, Some(authorized)) if *actor == authorized => {
                    Ok(())
                }
                (
                    DecisionOrigin::Institution {
                        institution,
                        responsible_actor,
                    },
                    expected,
                ) if institution == &binding.institution && *responsible_actor == expected => {
                    Ok(())
                }
                _ => Err(invalid_authority(
                    "fiscal decision origin does not satisfy the authority binding",
                )),
            }
        }
        _ => Err(invalid_authority(
            "fiscal action requires its bound actor or validated decision controller",
        )),
    }
}

fn ensure_action_outcome_capacity(
    state: &FiscalState,
    additional_outcomes: usize,
) -> Result<(), CanwuError> {
    let projected = state
        .action_outcomes
        .len()
        .checked_add(additional_outcomes)
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::ValueOutOfRange,
                "fiscal action outcome capacity calculation overflowed",
            )
        })?;
    if projected > MAX_FISCAL_ACTION_OUTCOMES {
        return Err(CanwuError::new(
            ErrorCode::ValueOutOfRange,
            "fiscal action outcome capacity is insufficient",
        ));
    }
    Ok(())
}

fn preflight_action_outcome_batch(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    state: &FiscalState,
) -> Result<(), CanwuError> {
    let mut pending_action_ids = std::collections::BTreeSet::new();
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
        if plugin != PLUGIN_NAME || packet_type != FISCAL_ACTION_INGRESS {
            continue;
        }
        let admitted: AdmittedFiscalAction = decode(payload, "admitted fiscal action")?;
        validate_admitted_action(view, ingress.cause.as_ref(), &admitted)?;
        if !state
            .action_outcomes
            .contains_key(&admitted.request.action_id)
        {
            pending_action_ids.insert(admitted.request.action_id);
        }
    }
    ensure_action_outcome_capacity(state, pending_action_ids.len())
}

fn settle_fiscal_ingress(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let Some((_, catalog)) = load_fiscal_catalog(view)? else {
        return Ok(BoundaryProposal::default());
    };
    let Some((record, mut state)) = load_fiscal_state(view, &catalog)? else {
        return Ok(BoundaryProposal::default());
    };
    preflight_action_outcome_batch(view, context, &state)?;
    let mut changed = false;
    let mut events = Vec::new();
    let mut expected_version_consumed = false;
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
        match packet_type.as_str() {
            FISCAL_ACTION_INGRESS => {
                let admitted: AdmittedFiscalAction = decode(payload, "admitted fiscal action")?;
                validate_admitted_action(view, ingress.cause.as_ref(), &admitted)?;
                if settle_action(
                    &mut state,
                    &catalog,
                    admitted,
                    context.at,
                    expected_version_consumed,
                )? {
                    expected_version_consumed = true;
                    changed = true;
                    events.push((
                        "canwu.fiscal.action_settled.v1",
                        "Settled one fiscal procedure action",
                    ));
                }
            }
            FISCAL_EXECUTION_RECEIPT_INGRESS => {
                let packet: FiscalExecutionReceiptPacket =
                    decode(payload, "fiscal execution receipt")?;
                if settle_execution_receipt(
                    view,
                    &mut state,
                    packet,
                    *ingress_id,
                    context.at,
                    &catalog,
                )? {
                    expected_version_consumed = true;
                    changed = true;
                    events.push((
                        "canwu.fiscal.execution_receipt_recorded.v1",
                        "Recorded external fiscal execution evidence",
                    ));
                }
            }
            FISCAL_HISTORICAL_CONTEXT_INGRESS => {
                let packet: FiscalHistoricalContextPacket =
                    decode(payload, "fiscal historical context")?;
                if !catalog.historical_scope.contains(packet.year) {
                    return Err(invalid("fiscal historical context lies outside the pack"));
                }
                state.historical_context.year = packet.year;
                state.historical_context.mode = packet.mode;
                state.historical_context.updated_at = context.at;
                increment_procedure_revision(&mut state)?;
                expected_version_consumed = true;
                changed = true;
                events.push((
                    "canwu.fiscal.historical_context_changed.v1",
                    "Changed the explicit fiscal historical context",
                ));
            }
            _ => {}
        }
    }
    if !changed {
        return Ok(BoundaryProposal::default());
    }
    let mut proposal = update_state(&record, &state, &catalog, "Settled fiscal ingress")?;
    proposal
        .directives
        .extend(
            events
                .into_iter()
                .map(|(event_type, summary)| BoundaryDirective::Emit {
                    event_type: event_type.to_owned(),
                    summary: summary.to_owned(),
                    affected: Vec::new(),
                }),
        );
    Ok(proposal)
}

fn settle_execution_receipt(
    view: &SimulationView<'_>,
    state: &mut FiscalState,
    packet: FiscalExecutionReceiptPacket,
    ingress_id: canwu_api::IngressId,
    at: SimTime,
    catalog: &CompiledFiscalCatalog,
) -> Result<bool, CanwuError> {
    let evidence = validate_execution_evidence(view, state, &packet)?;
    let receipt_count = state.execution_receipts.len();
    apply_receipt(state, packet, evidence, ingress_id, at, catalog)?;
    let changed = state.execution_receipts.len() != receipt_count;
    if changed {
        increment_procedure_revision(state)?;
    }
    Ok(changed)
}

fn validate_admitted_action(
    view: &SimulationView<'_>,
    cause: Option<&CauseRef>,
    admitted: &AdmittedFiscalAction,
) -> Result<(), CanwuError> {
    let Some(CauseRef::Command(command_id)) = cause else {
        return Err(invalid_authority(
            "fiscal action ingress must be caused by an admitted command",
        ));
    };
    let command_record = view
        .command(*command_id)?
        .ok_or_else(|| invalid_authority("fiscal action command evidence is unavailable"))?;
    let command_matches = *command_id == admitted.command
        && matches!(
            &command_record.envelope.command,
            Command::Plugin {
                plugin,
                command,
                payload,
            } if plugin == PLUGIN_NAME
                && command == APPLY_FISCAL_ACTION_COMMAND
                && decode::<FiscalActionRequest>(payload, "fiscal action")
                    .is_ok_and(|request| request == admitted.request)
        );
    if !command_matches {
        return Err(invalid_authority(
            "fiscal action ingress does not match its authorized command",
        ));
    }
    Ok(())
}

fn validate_execution_evidence(
    view: &SimulationView<'_>,
    state: &FiscalState,
    packet: &FiscalExecutionReceiptPacket,
) -> Result<ValidatedExecutionEvidence, CanwuError> {
    if packet.external_evidence.is_empty()
        || packet.external_evidence.len() > MAX_FISCAL_EVIDENCE_PER_RECORD
    {
        return Err(invalid(
            "fiscal execution receipt has an invalid external evidence count",
        ));
    }
    let request = state
        .execution_requests
        .get(&packet.request_id)
        .ok_or_else(|| {
            invalid("fiscal execution receipt names an unavailable execution request")
        })?;
    let mut evidenced_quantity = 0_u64;
    let mut disposition = None;
    let consumed_external_operations: std::collections::BTreeSet<_> = state
        .execution_receipts
        .values()
        .filter(|receipt| receipt.id != packet.receipt_id)
        .flat_map(|receipt| receipt.external_operations.iter().cloned())
        .collect();
    let mut external_operations = std::collections::BTreeSet::new();
    for evidence in &packet.external_evidence {
        if !state
            .execution_evidence_kinds
            .contains(&evidence.record.kind)
        {
            return Err(invalid(
                "fiscal execution receipt cites an unapproved external evidence kind",
            ));
        }
        if !view.domain_record_version_evidence_exists(evidence)? {
            return Err(invalid(
                "fiscal execution receipt cites unavailable exact external evidence",
            ));
        }
        let record = view
            .domain_record_version(evidence)?
            .ok_or_else(|| invalid("fiscal execution evidence payload is unavailable"))?;
        let claim: FiscalExecutionEvidence =
            decode(&record.payload, "typed fiscal execution evidence")?;
        let operation = FiscalExternalOperationRef {
            evidence_kind: evidence.record.kind.clone(),
            external_operation_id: claim.external_operation_id.clone(),
        };
        if consumed_external_operations.contains(&operation) {
            return Err(invalid(
                "external fiscal operation was already settled by another receipt",
            ));
        }
        if !external_operations.insert(operation) {
            return Err(invalid(
                "external fiscal operation was repeated within one receipt",
            ));
        }
        if claim.id != evidence.record.id
            || claim.id.trim().is_empty()
            || claim.external_operation_id.trim().is_empty()
            || claim.request_id != request.id
            || claim.unit != request.unit
            || claim.payment_form != request.payment_form
            || claim.execution_kind != request.kind
            || claim.resource != request.resource
            || claim.source != request.source
            || claim.target != request.target
            || claim.disposition.counts_as_fulfillment() == (claim.quantity == 0)
            || disposition.is_some_and(|value| value != claim.disposition)
        {
            return Err(invalid(
                "typed fiscal execution evidence does not match its request or receipt",
            ));
        }
        disposition = Some(claim.disposition);
        evidenced_quantity = evidenced_quantity
            .checked_add(claim.quantity)
            .ok_or_else(|| invalid("fiscal execution evidence quantity overflowed"))?;
        let established_at = view
            .evidence_time(&canwu_api::EvidenceRef::DomainRecordVersion(
                evidence.clone(),
            ))?
            .ok_or_else(|| {
                invalid("fiscal execution receipt evidence has no establishment time")
            })?;
        if established_at < request.requested_at {
            return Err(invalid(
                "fiscal execution receipt evidence predates its execution request",
            ));
        }
    }
    if evidenced_quantity > request.quantity {
        return Err(invalid(
            "fiscal execution evidence exceeds its authorized quantity",
        ));
    }
    Ok(ValidatedExecutionEvidence {
        quantity: evidenced_quantity,
        disposition: disposition
            .ok_or_else(|| invalid("fiscal execution evidence has no disposition"))?,
        external_operations,
    })
}

fn settle_action(
    state: &mut FiscalState,
    catalog: &CompiledFiscalCatalog,
    admitted: AdmittedFiscalAction,
    at: SimTime,
    expected_version_consumed: bool,
) -> Result<bool, CanwuError> {
    if state
        .action_outcomes
        .contains_key(&admitted.request.action_id)
    {
        return Ok(false);
    }
    let mut candidate = None;
    let result = if expected_version_consumed
        || admitted.request.expected_procedure_revision != state.procedure_revision
    {
        Err(CanwuError::new(
            ErrorCode::DomainRecordVersionConflict,
            "fiscal action was stale at its settlement boundary",
        ))
    } else {
        let mut next = state.clone();
        let result = apply_action(&mut next, catalog, &admitted.request, at)
            .and_then(|()| next.validate(catalog));
        candidate = Some(next);
        result
    };
    let (disposition, reason) = match result {
        Ok(()) => {
            *state = candidate.ok_or_else(|| invalid("fiscal action candidate is unavailable"))?;
            (FiscalActionDisposition::Applied, "applied".to_owned())
        }
        Err(error) => (FiscalActionDisposition::Rejected, error.to_string()),
    };
    state.action_outcomes.insert(
        admitted.request.action_id.clone(),
        FiscalActionOutcome {
            action_id: admitted.request.action_id,
            disposition,
            reason,
            command: admitted.command,
            settled_at: at,
        },
    );
    increment_procedure_revision(state)?;
    Ok(true)
}

fn increment_procedure_revision(state: &mut FiscalState) -> Result<(), CanwuError> {
    state.procedure_revision = state
        .procedure_revision
        .checked_add(1)
        .ok_or_else(|| invalid("fiscal procedure revision overflowed"))?;
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn apply_action(
    state: &mut FiscalState,
    catalog: &CompiledFiscalCatalog,
    request: &FiscalActionRequest,
    at: SimTime,
) -> Result<(), CanwuError> {
    match &request.action {
        FiscalAction::ChangeAdoption {
            adoption_id,
            rule_id,
            scope_binding_id,
            stage,
        } => {
            let rule = catalog
                .rules
                .get(rule_id)
                .ok_or_else(|| invalid("fiscal adoption names an unknown rule"))?;
            let scope = state
                .scope_bindings
                .get(scope_binding_id)
                .ok_or_else(|| invalid("fiscal adoption names an unknown scope"))?;
            if rule.mechanism != scope.mechanism
                || !rule.jurisdiction_ids.contains(&scope.jurisdiction_id)
            {
                return Err(invalid(
                    "fiscal adoption rule is incompatible with its scope",
                ));
            }
            if !state.adoptions.contains_key(adoption_id) {
                if stage.is_operational()
                    && catalog
                        .transitions
                        .values()
                        .any(|transition| transition.to_rule_ids.contains(rule_id))
                {
                    return Err(invalid(
                        "operational transition targets require an atomic fiscal transition",
                    ));
                }
                if !rule.legal_window.contains(state.historical_context.year) {
                    return Err(invalid("fiscal adoption is outside its legal window"));
                }
            }
            let generation = if let Some(existing) = state.adoptions.get(adoption_id) {
                if existing.rule_id != *rule_id || existing.scope_binding_id != *scope_binding_id {
                    return Err(invalid(
                        "fiscal adoption identity cannot change its rule or scope",
                    ));
                }
                existing
                    .generation
                    .checked_add(1)
                    .ok_or_else(|| invalid("fiscal adoption generation overflowed"))?
            } else {
                1
            };
            state.adoptions.insert(
                adoption_id.clone(),
                FiscalAdoptionState {
                    id: adoption_id.clone(),
                    rule_id: rule_id.clone(),
                    scope_binding_id: scope_binding_id.clone(),
                    stage: *stage,
                    generation,
                    changed_at: at,
                    source_action_id: Some(request.action_id.clone()),
                },
            );
        }
        FiscalAction::ApplyTransition {
            transition_id,
            target_scope_bindings,
        } => apply_transition(
            state,
            catalog,
            transition_id,
            target_scope_bindings,
            &request.action_id,
            at,
        )?,
        FiscalAction::OpenAssessment {
            assessment_id,
            rule_id,
            scope_binding_id,
            accounting_cycle_id,
            quantity,
            unit,
            payment_form,
            commutation_quote,
        } => {
            reject_existing(&state.assessments, assessment_id, "assessment")?;
            let rule = catalog
                .rules
                .get(rule_id)
                .ok_or_else(|| invalid("fiscal assessment names an unknown rule"))?;
            if !rule.legal_window.contains(state.historical_context.year) {
                return Err(invalid(
                    "fiscal assessment rule is outside its legal window",
                ));
            }
            validate_identifier(accounting_cycle_id, "fiscal accounting cycle")?;
            state.assessments.insert(
                assessment_id.clone(),
                FiscalAssessment {
                    id: assessment_id.clone(),
                    rule_id: rule_id.clone(),
                    scope_binding_id: scope_binding_id.clone(),
                    accounting_cycle_id: accounting_cycle_id.clone(),
                    quantity: *quantity,
                    unit: unit.clone(),
                    payment_form: *payment_form,
                    commutation_quote: commutation_quote.clone(),
                    created_at: at,
                },
            );
        }
        FiscalAction::GrantRemission {
            remission_id,
            assessment_id,
            quantity,
            reason,
        } => {
            reject_existing(&state.remissions, remission_id, "remission")?;
            state.remissions.insert(
                remission_id.clone(),
                FiscalRemission {
                    id: remission_id.clone(),
                    assessment_id: assessment_id.clone(),
                    quantity: *quantity,
                    reason: reason.clone(),
                    granted_at: at,
                },
            );
        }
        FiscalAction::AuthorizeExecution {
            request_id,
            assessment_id,
            kind,
            quantity,
            unit,
            resource,
            source,
            target,
            purpose,
        } => {
            reject_existing(&state.execution_requests, request_id, "execution request")?;
            let assessment = state
                .assessments
                .get(assessment_id)
                .ok_or_else(|| invalid("execution authorization names an unknown assessment"))?;
            let institution = state.scope_bindings[&assessment.scope_binding_id]
                .institution
                .clone();
            state.execution_requests.insert(
                request_id.clone(),
                FiscalExecutionRequest {
                    id: request_id.clone(),
                    assessment_id: assessment_id.clone(),
                    institution,
                    kind: *kind,
                    quantity: *quantity,
                    unit: unit.clone(),
                    payment_form: assessment.payment_form,
                    resource: *resource,
                    source: source.clone(),
                    target: target.clone(),
                    purpose: purpose.clone(),
                    requested_at: at,
                },
            );
        }
        FiscalAction::RecordAudit {
            audit_id,
            target_id,
            severity,
            finding,
            evidence,
        } => {
            reject_existing(&state.audits, audit_id, "audit")?;
            state.audits.insert(
                audit_id.clone(),
                FiscalAuditFinding {
                    id: audit_id.clone(),
                    target_id: target_id.clone(),
                    severity: *severity,
                    finding: finding.clone(),
                    evidence: evidence.clone(),
                    recorded_at: at,
                },
            );
        }
    }
    Ok(())
}

fn apply_transition(
    state: &mut FiscalState,
    catalog: &CompiledFiscalCatalog,
    transition_id: &str,
    target_scope_bindings: &std::collections::BTreeMap<String, String>,
    action_id: &str,
    at: SimTime,
) -> Result<(), CanwuError> {
    let transition = catalog
        .transitions
        .get(transition_id)
        .ok_or_else(|| invalid("fiscal transition is unavailable"))?;
    if target_scope_bindings
        .keys()
        .collect::<std::collections::BTreeSet<_>>()
        != transition
            .to_rule_ids
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
    {
        return Err(invalid(
            "fiscal transition must bind every target rule exactly once",
        ));
    }
    let mut target_jurisdiction = None;
    for (rule_id, scope_id) in target_scope_bindings {
        let rule = &catalog.rules[rule_id];
        let scope = state
            .scope_bindings
            .get(scope_id)
            .ok_or_else(|| invalid("fiscal transition target scope is unavailable"))?;
        if scope.mechanism != rule.mechanism {
            return Err(invalid(
                "fiscal transition target is incompatible with its jurisdiction or mechanism",
            ));
        }
        if target_jurisdiction
            .replace(scope.jurisdiction_id.clone())
            .is_some_and(|jurisdiction| jurisdiction != scope.jurisdiction_id)
        {
            return Err(invalid(
                "fiscal transition targets must share one jurisdiction",
            ));
        }
    }
    let target_jurisdiction = target_jurisdiction
        .ok_or_else(|| invalid("fiscal transition has no target jurisdiction"))?;
    let candidate_id = format!("{transition_id}::{target_jurisdiction}");
    let candidates = compute_transition_candidates(state, catalog, at);
    let candidate = candidates
        .get(&candidate_id)
        .ok_or_else(|| invalid("fiscal transition is not currently eligible"))?;
    for adoption in state.adoptions.values_mut() {
        let in_jurisdiction = state
            .scope_bindings
            .get(&adoption.scope_binding_id)
            .is_some_and(|scope| scope.jurisdiction_id == candidate.jurisdiction_id);
        if in_jurisdiction
            && transition
                .supersedes_or_suspends
                .contains(&adoption.rule_id)
        {
            adoption.stage = FiscalAdoptionStage::Suspended;
            adoption.generation = adoption
                .generation
                .checked_add(1)
                .ok_or_else(|| invalid("fiscal adoption generation overflowed"))?;
            adoption.changed_at = at;
            adoption.source_action_id = Some(action_id.to_owned());
        }
    }
    for (rule_id, scope_id) in target_scope_bindings {
        let existing_id = state.adoptions.values().find_map(|adoption| {
            (adoption.rule_id == *rule_id && adoption.scope_binding_id == *scope_id)
                .then(|| adoption.id.clone())
        });
        let adoption_id = existing_id
            .unwrap_or_else(|| format!("adopt.transition.{transition_id}.{scope_id}.{rule_id}"));
        let generation = state
            .adoptions
            .get(&adoption_id)
            .map_or(Ok(1), |adoption| {
                adoption
                    .generation
                    .checked_add(1)
                    .ok_or_else(|| invalid("fiscal adoption generation overflowed"))
            })?;
        state.adoptions.insert(
            adoption_id.clone(),
            FiscalAdoptionState {
                id: adoption_id,
                rule_id: rule_id.clone(),
                scope_binding_id: scope_id.clone(),
                stage: FiscalAdoptionStage::Implemented,
                generation,
                changed_at: at,
                source_action_id: Some(action_id.to_owned()),
            },
        );
    }
    Ok(())
}

fn reject_existing<T>(
    values: &std::collections::BTreeMap<String, T>,
    id: &str,
    label: &str,
) -> Result<(), CanwuError> {
    if values.contains_key(id) {
        return Err(CanwuError::new(
            ErrorCode::IdempotencyConflict,
            format!("fiscal {label} identity already exists"),
        ));
    }
    Ok(())
}

fn apply_receipt(
    state: &mut FiscalState,
    packet: FiscalExecutionReceiptPacket,
    evidence: ValidatedExecutionEvidence,
    ingress: canwu_api::IngressId,
    at: SimTime,
    catalog: &CompiledFiscalCatalog,
) -> Result<(), CanwuError> {
    let ValidatedExecutionEvidence {
        quantity,
        disposition,
        external_operations,
    } = evidence;
    if let Some(existing) = state.execution_receipts.get(&packet.receipt_id) {
        if existing.request_id == packet.request_id
            && existing.quantity == quantity
            && existing.disposition == disposition
            && existing.external_evidence == packet.external_evidence
            && existing.external_operations == external_operations
        {
            return Ok(());
        }
        return Err(invalid(
            "fiscal receipt identity was reused with different content",
        ));
    }
    state.execution_receipts.insert(
        packet.receipt_id.clone(),
        FiscalExecutionReceipt {
            id: packet.receipt_id,
            request_id: packet.request_id,
            quantity,
            disposition,
            external_evidence: packet.external_evidence,
            external_operations,
            accepted_ingress: ingress,
            observed_at: at,
        },
    );
    state.validate(catalog)
}

fn evaluate_candidates(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    update_derived(view, |state, catalog| {
        let candidates = compute_transition_candidates(state, catalog, context.at);
        if state.transition_candidates == candidates {
            return Ok(None);
        }
        state.transition_candidates = candidates;
        Ok(Some("Evaluated fiscal transition candidates"))
    })
}

fn aggregate_state(
    view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    update_derived(view, |state, catalog| {
        let aggregates = compute_aggregates(state, catalog)?;
        if state.aggregates == aggregates {
            return Ok(None);
        }
        state.aggregates = aggregates;
        Ok(Some("Aggregated fiscal state"))
    })
}

fn materialize_reports(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let Some((_, catalog)) = load_fiscal_catalog(view)? else {
        return Ok(BoundaryProposal::default());
    };
    let Some((_, state)) = load_fiscal_state(view, &catalog)? else {
        return Ok(BoundaryProposal::default());
    };
    let Some(source_version) =
        view.proposed_domain_record_version(&fiscal_state_reference().into_untyped())?
    else {
        return Ok(BoundaryProposal::default());
    };
    let reports = compute_projections(&state, context.at);
    let mut directives = Vec::new();
    for (observer_id, report) in reports {
        let correlation_prefix = format!("fiscal-report::{observer_id}::");
        let supersedes = current_report_heads(view, &correlation_prefix)?;
        let holder = state.observer_bindings[&observer_id]
            .knowledge_holder
            .clone();
        directives.push(BoundaryDirective::PublishKnowledge {
            holder,
            visibility: StateVisibility::SameBoundary,
            producer_correlation: Some(format!("{correlation_prefix}{}", context.boundary_id)),
            records: vec![KnowledgeRecordDraft {
                schema: fiscal_report_knowledge_schema_id(),
                subjects: vec![KnowledgeSubject {
                    role: "fiscal_state".to_owned(),
                    target: KnowledgeSubjectTarget::DomainRecord(
                        fiscal_state_reference().into_untyped(),
                    ),
                }],
                payload: serde_json::to_value(report).map_err(encode_error)?,
                as_of: Some(context.at),
                confidence_per_mille: state.observer_bindings[&observer_id].confidence_per_mille,
                origin: KnowledgeOrigin {
                    method: "fiscal_authority_report_v1".to_owned(),
                    evidence: vec![EvidenceRef::DomainRecordVersion(source_version.clone())],
                },
                supersedes,
                contradicts: Vec::new(),
            }],
            summary: "Publish one holder-relative fiscal report".to_owned(),
        });
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

fn current_report_heads(
    view: &SimulationView<'_>,
    correlation_prefix: &str,
) -> Result<Vec<KnowledgeRecordId>, CanwuError> {
    let changes = view.knowledge_changes_by_correlation_prefix(PLUGIN_NAME, correlation_prefix)?;
    let mut candidates = std::collections::BTreeSet::new();
    let mut superseded = std::collections::BTreeSet::new();
    for record in changes.iter().flat_map(|change| &change.records) {
        if record.schema == fiscal_report_knowledge_schema_id() {
            candidates.insert(record.id);
            superseded.extend(record.supersedes.iter().copied());
        }
    }
    Ok(candidates.difference(&superseded).copied().collect())
}

#[must_use]
pub fn fiscal_report_knowledge_schema_id() -> KnowledgeSchemaId {
    KnowledgeSchemaId::new(
        KnowledgeRecordKind::new(PLUGIN_NAME, FISCAL_REPORT_KNOWLEDGE),
        1,
    )
}

fn fiscal_report_knowledge_schema() -> PluginKnowledgeSchema {
    PluginKnowledgeSchema {
        id: fiscal_report_knowledge_schema_id(),
        schema_hash: FISCAL_REPORT_SCHEMA_HASH.to_owned(),
        writable: true,
        payload_schema: PayloadSchema::Any,
        subjects: vec![KnowledgeSubjectSchema {
            role: "fiscal_state".to_owned(),
            targets: vec![KnowledgeSubjectTargetKind::Domain(
                DomainRecordKind::for_type::<FiscalStateRecord>(),
            )],
            required: true,
            multiple: false,
        }],
    }
}

fn update_derived(
    view: &SimulationView<'_>,
    change: impl FnOnce(
        &mut FiscalState,
        &CompiledFiscalCatalog,
    ) -> Result<Option<&'static str>, CanwuError>,
) -> Result<BoundaryProposal, CanwuError> {
    let Some((_, catalog)) = load_fiscal_catalog(view)? else {
        return Ok(BoundaryProposal::default());
    };
    let Some((record, mut state)) = load_fiscal_state(view, &catalog)? else {
        return Ok(BoundaryProposal::default());
    };
    let Some(summary) = change(&mut state, &catalog)? else {
        return Ok(BoundaryProposal::default());
    };
    update_state(&record, &state, &catalog, summary)
}

fn update_state(
    record: &DomainRecord,
    state: &FiscalState,
    catalog: &CompiledFiscalCatalog,
    summary: &str,
) -> Result<BoundaryProposal, CanwuError> {
    state.validate(catalog)?;
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Update {
                record: state.record_draft()?,
                expected_version: record.version,
            },
            summary: summary.to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

pub fn fiscal_action_command(request: &FiscalActionRequest) -> Result<Command, serde_json::Error> {
    Ok(Command::Plugin {
        plugin: PLUGIN_NAME.to_owned(),
        command: APPLY_FISCAL_ACTION_COMMAND.to_owned(),
        payload: serde_json::to_value(request)?,
    })
}

pub fn enqueue_execution_receipt(
    canwu: &mut Canwu,
    due_at: SimTime,
    packet: &FiscalExecutionReceiptPacket,
) -> Result<canwu_api::IngressReceipt, CanwuError> {
    if packet.external_evidence.is_empty()
        || packet.external_evidence.len() > MAX_FISCAL_EVIDENCE_PER_RECORD
    {
        return Err(invalid(
            "fiscal execution receipt has an invalid external evidence count",
        ));
    }
    if packet
        .external_evidence
        .iter()
        .any(|evidence| !canwu.domain_record_version_evidence_exists(evidence))
    {
        return Err(invalid(
            "fiscal execution receipt requires exact available external record versions",
        ));
    }
    let payload = serde_json::to_value(packet).map_err(encode_error)?;
    canwu.enqueue_plugin_ingress(PluginIngressRequest::new(
        PLUGIN_NAME,
        FISCAL_EXECUTION_RECEIPT_INGRESS,
        due_at,
        payload,
    ))
}

pub fn fiscal_historical_context_ingress(
    due_at: SimTime,
    packet: &FiscalHistoricalContextPacket,
) -> Result<PluginIngressRequest, CanwuError> {
    Ok(PluginIngressRequest::new(
        PLUGIN_NAME,
        FISCAL_HISTORICAL_CONTEXT_INGRESS,
        due_at,
        serde_json::to_value(packet).map_err(encode_error)?,
    ))
}

fn catalog_key() -> StateKey {
    StateKey::new(FiscalCatalogRecord::NAMESPACE, FiscalCatalogRecord::NAME)
}

fn state_key() -> StateKey {
    StateKey::new(FiscalStateRecord::NAMESPACE, FiscalStateRecord::NAME)
}

fn decode<T: serde::de::DeserializeOwned>(value: &Value, label: &str) -> Result<T, CanwuError> {
    serde_json::from_value(value.clone()).map_err(|error| {
        CanwuError::new(
            ErrorCode::InvalidPayload,
            format!("{label} could not be decoded: {error}"),
        )
    })
}

#[allow(clippy::needless_pass_by_value)]
fn encode_error(error: serde_json::Error) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidPayload, error.to_string())
}

fn missing(message: &str) -> CanwuError {
    CanwuError::new(ErrorCode::DomainRecordNotFound, message)
}

fn invalid_authority(message: &str) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidAuthority, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_outcome_capacity_rejects_before_an_unrecordable_action_is_admitted() {
        let mut state = FiscalState::new(
            1391,
            crate::model::FiscalHistoricalMode::RecordedBaseline,
            SimTime::EPOCH,
        );
        for index in 0..MAX_FISCAL_ACTION_OUTCOMES {
            let action_id = format!("capacity.action.{index}");
            state.action_outcomes.insert(
                action_id.clone(),
                FiscalActionOutcome {
                    action_id,
                    disposition: FiscalActionDisposition::Rejected,
                    reason: "capacity fixture".to_owned(),
                    command: canwu_api::CommandId::new(1),
                    settled_at: SimTime::EPOCH,
                },
            );
        }

        let error = ensure_action_outcome_capacity(&state, 1)
            .expect_err("a full outcome map must reject at command admission");
        assert_eq!(error.code, ErrorCode::ValueOutOfRange);
    }

    #[test]
    fn action_outcome_batch_rejects_two_actions_with_one_remaining_slot() {
        let mut state = FiscalState::new(
            1391,
            crate::model::FiscalHistoricalMode::RecordedBaseline,
            SimTime::EPOCH,
        );
        for index in 0..(MAX_FISCAL_ACTION_OUTCOMES - 1) {
            let action_id = format!("capacity.action.{index}");
            state.action_outcomes.insert(
                action_id.clone(),
                FiscalActionOutcome {
                    action_id,
                    disposition: FiscalActionDisposition::Rejected,
                    reason: "capacity fixture".to_owned(),
                    command: canwu_api::CommandId::new(1),
                    settled_at: SimTime::EPOCH,
                },
            );
        }

        ensure_action_outcome_capacity(&state, 1)
            .expect("one same-boundary action still fits the remaining slot");
        let error = ensure_action_outcome_capacity(&state, 2)
            .expect_err("two same-boundary actions must be rejected during batch preflight");
        assert_eq!(error.code, ErrorCode::ValueOutOfRange);
    }
}
