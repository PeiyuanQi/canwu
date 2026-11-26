use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundarySystemContract,
    CanwuError, DomainRecordDraft, DomainRecordMutation, DomainRecordMutationPolicy,
    DomainRecordSchema, DomainRecordType, DomainRecordVersionRef, DomainRecordVersionSource,
    DomainValueKindClass, ErrorCode, IngressClass, IngressPayload, PayloadSchema,
    PluginIngressDescriptor, PluginIngressRequest, PluginRegistrar, SimTime, SimulationPlugin,
    SimulationView, StateKey, StateVisibility, SystemCadence, TypedDomainRecordRef,
};
use canwu_fiscal::{
    FiscalExecutionEvidence, FiscalExecutionRequest, FiscalLedgerRecord, FiscalSettledRecord,
    fiscal_ledger_reference, fiscal_settled_reference,
};

const PLUGIN_NAME: &str = "canwu.ming-fiscal-reference";
const EXECUTION_RESULT_INGRESS: &str = "reference_fiscal_execution_result_v1";
/// Version 2 reads a settled request from its scope's settled bucket
/// (canwu-fiscal 0.7.0-experimental).
const PLUGIN_VERSION: &str = "2";
/// BLAKE3 of `canwu.ming-fiscal-reference plugin semantic hash 2: settled
/// executions are read from their bucket`.
const SEMANTIC_HASH: &str = "e5bde20a241606f8180aabb3000bbf382f647fb4ad2cc98b7f9370f7a68c2855";

pub struct ReferenceFiscalExecutionEvidenceRecord;

impl DomainRecordType for ReferenceFiscalExecutionEvidenceRecord {
    type Payload = FiscalExecutionEvidence;
    type Class = DomainValueKindClass;

    const NAMESPACE: &'static str = PLUGIN_NAME;
    const NAME: &'static str = "fiscal_execution_evidence";
}

pub type ReferenceFiscalExecutionEvidence = FiscalExecutionEvidence;

#[derive(Clone, Copy, Debug, Default)]
pub struct MingFiscalExecutionAdapterPlugin;

impl SimulationPlugin for MingFiscalExecutionAdapterPlugin {
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
        let mut schema = DomainRecordSchema::for_record::<ReferenceFiscalExecutionEvidenceRecord>();
        schema.mutation_policy = DomainRecordMutationPolicy::CreateOnly;
        schema.payload_schema = PayloadSchema::Any;
        let evidence_key = schema.state_key();
        registrar.register_record_schema(schema)?;
        registrar.register_ingress(PluginIngressDescriptor {
            name: EXECUTION_RESULT_INGRESS.to_owned(),
            description: "Record a typed result from the reference fiscal execution adapter"
                .to_owned(),
            class: IngressClass::Acknowledgement,
            payload_schema: PayloadSchema::Any,
        })?;
        let mut system = BoundarySystemContract::new(
            "record-reference-fiscal-execution-result-v1",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        system.reads = vec![
            StateKey::core_ingress(),
            StateKey::new(FiscalLedgerRecord::NAMESPACE, FiscalLedgerRecord::NAME),
            StateKey::new(FiscalSettledRecord::NAMESPACE, FiscalSettledRecord::NAME),
        ];
        system.writes = vec![evidence_key];
        system.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(system, record_execution_results)
    }
}

fn record_execution_results(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut directives = Vec::new();
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
        if plugin != PLUGIN_NAME || packet_type != EXECUTION_RESULT_INGRESS {
            continue;
        }
        let evidence: ReferenceFiscalExecutionEvidence = serde_json::from_value(payload.clone())
            .map_err(|error| {
                invalid(format!(
                    "reference fiscal execution evidence could not be decoded: {error}"
                ))
            })?;
        let request = execution_request(view, &evidence)?;
        validate_execution_result(request.as_ref(), &evidence)?;
        directives.push(BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Create {
                record: DomainRecordDraft::from_typed(
                    reference_execution_evidence_ref(&evidence.id),
                    &evidence,
                )?,
            },
            summary: "Record typed reference fiscal execution evidence".to_owned(),
        });
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

/// The request a result names: hot in its scope's ledger, or, once it
/// settled, in the settled bucket its ID names.
fn execution_request(
    view: &SimulationView<'_>,
    evidence: &ReferenceFiscalExecutionEvidence,
) -> Result<Option<FiscalExecutionRequest>, CanwuError> {
    let ledger = view
        .typed_domain_record(&fiscal_ledger_reference(&evidence.scope_binding_id))?
        .map(canwu_api::DomainRecord::decode_payload::<FiscalLedgerRecord>)
        .transpose()?;
    let hot = ledger
        .as_ref()
        .and_then(|ledger| ledger.book_of_request(&evidence.request_id))
        .and_then(|book| book.execution_requests.get(&evidence.request_id));
    if let Some(request) = hot {
        return Ok(Some(request.clone()));
    }
    if ledger.is_none() {
        return Ok(None);
    }
    let bucket = view
        .typed_domain_record(&fiscal_settled_reference(
            &evidence.scope_binding_id,
            &evidence.request_id,
        ))?
        .map(canwu_api::DomainRecord::decode_payload::<FiscalSettledRecord>)
        .transpose()?;
    Ok(bucket
        .and_then(|mut bucket| bucket.entries.remove(&evidence.request_id))
        .map(|entry| entry.request))
}

fn validate_execution_result(
    request: Option<&FiscalExecutionRequest>,
    evidence: &ReferenceFiscalExecutionEvidence,
) -> Result<(), CanwuError> {
    if evidence.id.trim().is_empty()
        || evidence.external_operation_id.trim().is_empty()
        || evidence.unit.trim().is_empty()
    {
        return Err(invalid(
            "reference fiscal execution evidence requires non-empty identities and unit",
        ));
    }
    let request = request.ok_or_else(|| {
        invalid("reference fiscal execution evidence names an unavailable request")
    })?;
    if evidence.unit != request.unit
        || evidence.payment_form != request.payment_form
        || evidence.execution_kind != request.kind
        || evidence.resource != request.resource
        || evidence.source != request.source
        || evidence.target != request.target
        || evidence.quantity > request.quantity
    {
        return Err(invalid(
            "reference fiscal execution evidence does not match its request",
        ));
    }
    if evidence.disposition.counts_as_fulfillment() == (evidence.quantity == 0) {
        return Err(invalid(
            "reference fiscal execution evidence has inconsistent quantity and disposition",
        ));
    }
    Ok(())
}

/// Queues a reference adapter execution result. Takes either runtime handle, [`canwu_api::Canwu`] or [`canwu_api::CompactedCanwu`].
///
/// The reference adapter records the evidence of one request of one scope:
/// a result with `parts` (aggregated evidence) is refused here.
pub fn enqueue_reference_execution_result<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &mut H,
    due_at: SimTime,
    evidence: &ReferenceFiscalExecutionEvidence,
) -> Result<canwu_api::IngressReceipt, CanwuError> {
    if !evidence.parts.is_empty() {
        return Err(invalid(
            "the reference adapter records per-scope execution results only",
        ));
    }
    let payload = serde_json::to_value(evidence)
        .map_err(|error| invalid(format!("execution evidence could not be encoded: {error}")))?;
    canwu.enqueue_plugin_ingress(PluginIngressRequest::new(
        PLUGIN_NAME,
        EXECUTION_RESULT_INGRESS,
        due_at,
        payload,
    ))
}

#[must_use]
pub fn reference_execution_evidence_ref(
    id: &str,
) -> TypedDomainRecordRef<ReferenceFiscalExecutionEvidenceRecord> {
    TypedDomainRecordRef::new(id)
}

/// The exact version of a reference execution evidence record, from the
/// current-version provenance index, so it answers alike before and after a
/// seal and on either runtime handle.
#[must_use]
pub fn reference_execution_evidence_version<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &H,
    id: &str,
) -> Option<DomainRecordVersionRef> {
    let reference = reference_execution_evidence_ref(id).into_untyped();
    canwu
        .current_domain_record_version(&reference)
        .ok()
        .flatten()
        .filter(|version| {
            matches!(
                version.established_by,
                DomainRecordVersionSource::BoundaryChange { .. }
            )
        })
}

fn invalid(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidPayload, message)
}
