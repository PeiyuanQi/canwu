use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundarySystemContract,
    CanwuError, DomainRecordDraft, DomainRecordMutation, DomainRecordMutationPolicy,
    DomainRecordSchema, DomainRecordType, DomainRecordVersionRef, DomainRecordVersionSource,
    DomainValueKindClass, ErrorCode, IngressClass, IngressPayload, PayloadSchema,
    PluginIngressDescriptor, PluginIngressRequest, PluginRegistrar, SimTime, SimulationPlugin,
    SimulationView, StateKey, StateVisibility, SystemCadence, TypedDomainRecordRef,
};
use canwu_fiscal::{FiscalExecutionEvidence, FiscalLedgerRecord, fiscal_ledger_reference};

const PLUGIN_NAME: &str = "canwu.ming-fiscal-reference";
const EXECUTION_RESULT_INGRESS: &str = "reference_fiscal_execution_result_v1";
const PLUGIN_VERSION: &str = "1";
const SEMANTIC_HASH: &str = "67cba604d87167b0a91e681de3817a353577a531f7080ae403c936eb7dc00862";

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
        let ledger = view
            .typed_domain_record(&fiscal_ledger_reference(&evidence.scope_binding_id))?
            .map(canwu_api::DomainRecord::decode_payload::<FiscalLedgerRecord>)
            .transpose()?;
        validate_execution_result(ledger.as_ref(), &evidence)?;
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

fn validate_execution_result(
    ledger: Option<&canwu_fiscal::FiscalScopeLedger>,
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
    let request = ledger
        .and_then(|ledger| ledger.book_of_request(&evidence.request_id))
        .and_then(|book| book.execution_requests.get(&evidence.request_id))
        .ok_or_else(|| {
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
pub fn enqueue_reference_execution_result<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &mut H,
    due_at: SimTime,
    evidence: &ReferenceFiscalExecutionEvidence,
) -> Result<canwu_api::IngressReceipt, CanwuError> {
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
