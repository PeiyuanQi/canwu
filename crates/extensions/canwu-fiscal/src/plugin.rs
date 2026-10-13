use crate::derive::{
    assemble_fiscal_reports, compute_aggregates, compute_transition_candidates, next_report_plan,
    planned_reports,
};
use crate::model::{
    CompiledFiscalCatalog, FISCAL_ACTING_BASIS_NOT_CURRENT, FISCAL_AUTHORITY_BASIS_NOT_CURRENT,
    FISCAL_BOUNDARY_ACTION_LIMIT, FISCAL_ISSUER_NOT_BOUND, FISCAL_STATE_BUDGET_EXHAUSTED,
    FISCAL_STATE_CATALOG_ROLE, FISCAL_STATE_OBSERVER_ROLE, FiscalAction, FiscalActionDisposition,
    FiscalActionIssuer, FiscalActionOutcome, FiscalActionRequest, FiscalAdoptionStage,
    FiscalAdoptionState, FiscalAssessment, FiscalAuditBinding, FiscalAuditFinding,
    FiscalAuthorityBinding, FiscalCatalogRecord, FiscalExecutionEvidence, FiscalExecutionReceipt,
    FiscalExecutionReceiptPacket, FiscalExecutionRequest, FiscalExternalOperationRef,
    FiscalHistoricalContextPacket, FiscalIssuerRole, FiscalObserverBinding, FiscalRebinding,
    FiscalReceiptDisposition, FiscalRemission, FiscalReport, FiscalReportPlan, FiscalReportRecord,
    FiscalState, FiscalStateRecord, MAX_FISCAL_ACTION_JSON_BYTES, MAX_FISCAL_ACTION_OUTCOMES,
    MAX_FISCAL_ACTION_TEXT_BYTES, MAX_FISCAL_ACTIONS_PER_BOUNDARY, MAX_FISCAL_EVIDENCE_PER_RECORD,
    MAX_FISCAL_IDENTIFIER_BYTES, MAX_FISCAL_OBSERVERS, MAX_FISCAL_REFERENCES_PER_DEFINITION,
    MAX_FISCAL_REPORT_PARTS, MAX_FISCAL_STATE_JSON_BYTES, fiscal_catalog_reference,
    fiscal_state_reference, invalid, validate_identifier,
};
use crate::projection::{committed_fiscal_catalog, committed_fiscal_state};
use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundarySystemContract,
    Canwu, CanwuError, CanwuViewer, CauseRef, Command, CommandContext, CommandIngress,
    CoreEntityKind, DecisionOrigin, DomainRecord, DomainRecordKind, DomainRecordMutation,
    DomainRecordMutationPolicy, DomainRecordSchema, DomainRecordType, DomainRecordVersionRef,
    DomainRecordVersionSource, DomainReferenceSchema, DomainReferenceTargetKind, EntityRef,
    ErrorCode, EvidenceRef, IngressClass, IngressId, IngressPayload, Issuer, KnowledgeHolderRef,
    KnowledgeOrigin, KnowledgeQuery, KnowledgeRecordDraft, KnowledgeRecordKind, KnowledgeSchemaId,
    KnowledgeSubject, KnowledgeSubjectSchema, KnowledgeSubjectTarget, KnowledgeSubjectTargetKind,
    KnowledgeWriteGrant, LifeState, MAX_KNOWLEDGE_PAGE_SIZE,
    PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD, PayloadSchema, PersonId, PluginActionDescriptor,
    PluginIngressDescriptor, PluginIngressRequest, PluginKnowledgeSchema, PluginRegistrar, SimTime,
    SimulationPlugin, SimulationView, StateKey, StateVisibility, SystemCadence, SystemDirective,
    payload_required_evidence_continuation_property_v1,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const PLUGIN_NAME: &str = "canwu-fiscal";
pub const APPLY_FISCAL_ACTION_COMMAND: &str = "apply_fiscal_action_v1";
pub const FISCAL_ACTION_INGRESS: &str = "fiscal_action_v1";
pub const FISCAL_EXECUTION_RECEIPT_INGRESS: &str = "fiscal_execution_receipt_v1";
pub const FISCAL_HISTORICAL_CONTEXT_INGRESS: &str = "fiscal_historical_context_v1";

const PLUGIN_VERSION: &str = "0.1.0-experimental";
const SEMANTIC_HASH: &str = "293b7993cb315b08602ddeeaa4a44fd5caf8507d3aa8ccde2bea8b41a508a1dc";
const FISCAL_REPORT_KNOWLEDGE: &str = "fiscal_report";
/// Version 1 was one record for an observer's whole report. Version 2 is a
/// summary record and part records, each on a report line of its own.
const FISCAL_REPORT_SCHEMA_VERSION: u32 = 2;
const FISCAL_REPORT_SCHEMA_HASH: &str =
    "7924ae5b8ff1860a018585e09e0410481858408b31495c45da266c6c92df1ab2";
/// The subject role every fiscal report record carries.
const REPORT_STATE_ROLE: &str = "fiscal_state";
/// Report lines one observer binding has: line 0 for its summary, then one
/// for each part.
const REPORT_LINES_PER_OBSERVER: u32 = 1_024;
/// Hexadecimal places of a report line number.
const REPORT_LINE_PLACES: u32 = 4;
const _: () = assert!(MAX_FISCAL_REPORT_PARTS < REPORT_LINES_PER_OBSERVER as usize);
const _: () = assert!(
    MAX_FISCAL_OBSERVERS * REPORT_LINES_PER_OBSERVER as usize <= 1 << (4 * REPORT_LINE_PLACES)
);
// One observer's summary and parts are one publication, and each observer has
// one, so neither engine limit on a system's publications can be reached.
const _: () =
    assert!(MAX_FISCAL_REPORT_PARTS < canwu_api::KnowledgeLimitsV1::CURRENT.records_per_batch);
const _: () = assert!(
    MAX_FISCAL_OBSERVERS <= canwu_api::KnowledgeLimitsV1::CURRENT.batches_per_system_boundary
);
const ACTION_SETTLED_EVENT: &str = "canwu.fiscal.action_settled.v1";
const ACTION_DROPPED_EVENT: &str = "canwu.fiscal.action_dropped.v1";
/// A settlement that leaves the state record within this many bytes of its
/// budget is settled again with every action measured. The margin covers what
/// the derived systems of the same boundary add after settlement.
const STATE_BUDGET_MARGIN_BYTES: usize = 1_024 * 1_024;
/// Bytes of the state's byte budget that settlement leaves free.
///
/// Settlement measures the state as the derived systems of the same boundary
/// will leave it, taking every observer to be able to receive knowledge. For
/// an observer who cannot, the report plan lists the same parts under another
/// key, which moves at most one observer ID for each observer binding. The
/// reserve is more than that, so what the aggregate system writes after a
/// measured settlement always fits the budget.
const REPORT_PLAN_RESERVE_BYTES: usize = 16 * 1_024;
const _: () = assert!(
    MAX_FISCAL_OBSERVERS * (MAX_FISCAL_IDENTIFIER_BYTES + 16) + 64 <= REPORT_PLAN_RESERVE_BYTES
);
const STALE_ACTION: &str = "fiscal action was stale at its settlement boundary";
const STALE_OVERWRITE: &str = "fiscal action was stale at its settlement boundary: an earlier action of the boundary changed an adoption or binding it overwrites";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct AdmittedFiscalAction {
    request: FiscalActionRequest,
    command: canwu_api::CommandId,
    /// Who issued the command and in which role, as admission resolved it.
    /// Settlement re-checks the role against the state it settles on.
    issuer: FiscalActionIssuer,
}

/// One admitted ingress item of this plugin, decoded in admission order.
enum FiscalPacket {
    Action(Box<AdmittedFiscalAction>),
    Receipt(IngressId, FiscalExecutionReceiptPacket),
    HistoricalContext(FiscalHistoricalContextPacket),
}

/// The adoptions and bindings that actions of one boundary have changed.
/// A later action of the same boundary may read them but not overwrite them.
#[derive(Default)]
struct BoundaryOverwrites {
    adoptions: BTreeSet<String>,
    authority_bindings: BTreeSet<String>,
    audit_bindings: BTreeSet<String>,
}

struct ValidatedExecutionEvidence {
    quantity: u64,
    disposition: FiscalReceiptDisposition,
    external_operations: std::collections::BTreeSet<FiscalExternalOperationRef>,
}

#[derive(Clone, Debug, Default)]
pub struct FiscalPlugin {
    external_evidence_kinds: Vec<DomainRecordKind>,
    authority_basis_kinds: Vec<DomainRecordKind>,
}

impl FiscalPlugin {
    #[must_use]
    pub fn new(external_evidence_kinds: impl IntoIterator<Item = DomainRecordKind>) -> Self {
        Self {
            external_evidence_kinds: canonical_kinds(external_evidence_kinds),
            authority_basis_kinds: Vec::new(),
        }
    }

    /// Declares the record kinds that an authority binding may cite as its
    /// acting actor's `authority_basis` and an audit binding as its auditor's.
    /// The plugin reads these kinds to check that a basis is still the current
    /// version of its record. A basis must be of a kind declared here or
    /// passed to [`Self::new`] as an external evidence kind: activation
    /// rejects a binding that cites any other kind, and admission rejects a
    /// `RebindAuthority` or `RebindAudit` action that would install one.
    #[must_use]
    pub fn with_authority_basis_kinds(
        mut self,
        authority_basis_kinds: impl IntoIterator<Item = DomainRecordKind>,
    ) -> Self {
        self.authority_basis_kinds = canonical_kinds(authority_basis_kinds);
        self
    }

    /// Whether a binding may cite a record of `kind` as its authority basis:
    /// an external kind this plugin was configured to read. Command admission
    /// can only observe that a kind is readable, so activation accepts exactly
    /// the kinds admission would let a rebind install, and a run that settled
    /// a rebind can always be restored.
    fn reads_as_authority_basis(&self, kind: &DomainRecordKind) -> bool {
        !is_fiscal_record_kind(kind)
            && (self.authority_basis_kinds.binary_search(kind).is_ok()
                || self.external_evidence_kinds.binary_search(kind).is_ok())
    }

    fn external_record_state_keys(&self) -> Vec<StateKey> {
        self.external_evidence_kinds
            .iter()
            .chain(&self.authority_basis_kinds)
            .map(|kind| StateKey::new(kind.namespace.clone(), kind.name.clone()))
            .collect()
    }
}

/// One of the plugin's own record kinds, which it reads but which can never
/// stand for an application's grant of authority.
fn is_fiscal_record_kind(kind: &DomainRecordKind) -> bool {
    kind.namespace == FiscalStateRecord::NAMESPACE
}

fn canonical_kinds(kinds: impl IntoIterator<Item = DomainRecordKind>) -> Vec<DomainRecordKind> {
    let mut kinds: Vec<_> = kinds.into_iter().collect();
    kinds.sort();
    kinds.dedup();
    kinds
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

    fn validate_activation(&self, records: &[DomainRecord]) -> Result<(), CanwuError> {
        for record in records
            .iter()
            .filter(|record| record.reference.kind.matches_type::<FiscalStateRecord>())
        {
            let state = record.decode_payload::<FiscalStateRecord>()?;
            if state
                .authority_bindings
                .values()
                .filter_map(|binding| binding.authority_basis.as_ref())
                .chain(
                    state
                        .audit_bindings
                        .values()
                        .map(|binding| &binding.authority_basis),
                )
                .any(|basis| !self.reads_as_authority_basis(&basis.record.kind))
            {
                return Err(invalid(
                    "fiscal authority basis kind is not declared by FiscalPlugin::with_authority_basis_kinds",
                ));
            }
            // The boundary systems skip the fiscal records on a boundary that
            // has nothing for them, so a record that enters a run is validated
            // here, where it enters, as every later write is before it commits.
            if record.reference == fiscal_state_reference().into_untyped()
                && let Some(catalog) = records
                    .iter()
                    .find(|record| record.reference == fiscal_catalog_reference().into_untyped())
            {
                let catalog = catalog.decode_payload::<FiscalCatalogRecord>()?;
                catalog.validate()?;
                state.validate(&catalog)?;
                state.validate_record_binding(record)?;
            }
        }
        Ok(())
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        register_fiscal_record_schemas(registrar)?;
        let report_schema = register_fiscal_report_schema(registrar)?;

        let mut action_reads = vec![StateKey::core_evidence(), catalog_key(), state_key()];
        action_reads.extend(self.external_record_state_keys());
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
        settle.reads.extend(self.external_record_state_keys());
        settle.reads.sort();
        settle.reads.dedup();
        settle.writes = vec![state_key()];
        settle.emits = vec![
            ACTION_DROPPED_EVENT.to_owned(),
            ACTION_SETTLED_EVENT.to_owned(),
            "canwu.fiscal.execution_receipt_recorded.v1".to_owned(),
            "canwu.fiscal.historical_context_changed.v1".to_owned(),
        ];
        settle.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(settle, settle_fiscal_ingress)?;

        register_derived_system(
            registrar,
            "evaluate-fiscal-transition-candidates-v1",
            BoundaryPhase::HistoricalCandidateEvaluation,
            &[],
            evaluate_candidates,
        )?;
        // The aggregate system plans the reports, and plans none for an
        // observer the engine would refuse to publish to.
        register_derived_system(
            registrar,
            "aggregate-fiscal-state-v1",
            BoundaryPhase::StrategicAggregation,
            &[StateKey::core_person_availability()],
            aggregate_state,
        )?;
        let mut reports = BoundarySystemContract::new(
            "materialize-fiscal-reports-v1",
            BoundaryPhase::PerspectiveAndReportMaterialization,
            SystemCadence::EventDriven,
        );
        reports.reads = vec![
            StateKey::core_knowledge(),
            StateKey::core_person_availability(),
            catalog_key(),
            state_key(),
        ];
        reports.reads.sort();
        reports.reads.dedup();
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
    state_schema.references = vec![
        DomainReferenceSchema {
            role: FISCAL_STATE_CATALOG_ROLE.to_owned(),
            targets: vec![DomainReferenceTargetKind::Domain(
                DomainRecordKind::for_type::<FiscalCatalogRecord>(),
            )],
            required: true,
            multiple: false,
            allow_retired: false,
        },
        // The engine checks these against its entity registry, so a state
        // whose observer binding names a person who does not exist is refused
        // when a run is created or restored.
        DomainReferenceSchema {
            role: FISCAL_STATE_OBSERVER_ROLE.to_owned(),
            targets: vec![DomainReferenceTargetKind::Core(CoreEntityKind::Person)],
            required: false,
            multiple: true,
            allow_retired: false,
        },
    ];
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
    core_reads: &[StateKey],
    handler: canwu_api::BoundarySystemHandler,
) -> Result<(), CanwuError> {
    let mut contract = BoundarySystemContract::new(name, phase, SystemCadence::EventDriven);
    contract.reads = vec![catalog_key(), state_key()];
    contract.reads.extend_from_slice(core_reads);
    contract.reads.sort();
    contract.reads.dedup();
    contract.writes = vec![state_key()];
    contract.visibility = StateVisibility::SameBoundary;
    registrar.register_boundary_system(contract, handler)
}

/// Admits one fiscal action. A rejection caused by the command uses a code the
/// engine records as a rejected command attempt instead of failing the
/// boundary. `expected_procedure_revision` is checked only at settlement,
/// where a stale action is recorded as a rejected outcome.
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
    if serde_json::to_vec(payload).map_err(encode_error)?.len() > MAX_FISCAL_ACTION_JSON_BYTES {
        return Err(invalid_payload(
            "fiscal action request exceeds its serialized byte limit",
        ));
    }
    let request: FiscalActionRequest = decode(payload, "fiscal action")?;
    validate_identifier(&request.action_id, "fiscal action").map_err(as_invalid_payload)?;
    validate_identifier(&request.authority_binding_id, "fiscal authority binding")
        .map_err(as_invalid_payload)?;
    validate_action_bounds(&request.action)?;
    if view
        .typed_domain_record(&fiscal_catalog_reference())?
        .is_none()
    {
        return Err(missing("fiscal catalog is not configured"));
    }
    let Some((_, state)) = committed_fiscal_state(view)? else {
        return Err(missing("fiscal state is not configured"));
    };
    if state.action_outcomes.contains_key(&request.action_id) {
        return Err(CanwuError::new(
            ErrorCode::IdempotencyConflict,
            "fiscal action identity was already settled",
        ));
    }
    ensure_action_outcome_slot(&state)?;
    let (issuer, institution) = admit_issuer(view, context, &state, &request)?;
    admit_action(view, &state, &request, institution)?;
    // The engine checks every affected entity of a queued item against its
    // entity registry, which also holds persons that have no record in the
    // world projection. Naming the persons of a rebind here therefore rejects
    // the command as `EntityNotFound` when one of them does not exist.
    let mut affected: Vec<_> = std::iter::once(institution.clone())
        .chain(
            named_persons(&request.action)
                .into_iter()
                .map(EntityRef::Person),
        )
        .collect();
    affected.sort();
    affected.dedup();
    Ok(vec![SystemDirective::EnqueuePluginIngress {
        after: canwu_api::SimDuration::ZERO,
        packet_type: FISCAL_ACTION_INGRESS.to_owned(),
        priority: 0,
        affected,
        payload: serde_json::to_value(AdmittedFiscalAction {
            request,
            command: context.command_id,
            issuer,
        })
        .map_err(encode_error)?,
    }])
}

/// Resolves the binding a request names and the role its issuer holds under
/// it, together with the institution the action concerns.
fn admit_issuer<'a>(
    view: &SimulationView<'_>,
    context: &CommandContext,
    state: &'a FiscalState,
    request: &FiscalActionRequest,
) -> Result<(FiscalActionIssuer, &'a EntityRef), CanwuError> {
    let rebind = matches!(
        request.action,
        FiscalAction::RebindAuthority { .. } | FiscalAction::RebindAudit { .. }
    );
    if let Some(binding) = state.authority_bindings.get(&request.authority_binding_id) {
        let (role, actor) = authority_role(context, binding, rebind)?;
        let authority_basis = if role == FiscalIssuerRole::Acting {
            if rebind {
                return Err(invalid_authority(
                    "a fiscal acting actor cannot rebind an authority or audit binding",
                ));
            }
            let basis = binding
                .authority_basis
                .as_ref()
                .ok_or_else(|| invalid_authority(FISCAL_ACTING_BASIS_NOT_CURRENT))?;
            require_current_basis(view, basis, FISCAL_ACTING_BASIS_NOT_CURRENT)?;
            Some(basis.clone())
        } else {
            None
        };
        return Ok((
            FiscalActionIssuer {
                binding_id: binding.id.clone(),
                role,
                actor,
                authority_basis,
            },
            &binding.institution,
        ));
    }
    let Some(binding) = state.audit_bindings.get(&request.authority_binding_id) else {
        return Err(invalid_authority("fiscal authority binding is unavailable"));
    };
    if !matches!(request.action, FiscalAction::RecordAudit { .. }) {
        return Err(invalid_authority(
            "a fiscal audit binding admits only audit findings",
        ));
    }
    if !issuer_is_auditor(context, binding) {
        return Err(invalid_authority(
            "fiscal audit finding requires its bound auditor or a validated decision of that auditor",
        ));
    }
    require_current_basis(
        view,
        &binding.authority_basis,
        FISCAL_AUTHORITY_BASIS_NOT_CURRENT,
    )?;
    Ok((
        FiscalActionIssuer {
            binding_id: binding.id.clone(),
            role: FiscalIssuerRole::Auditor,
            actor: Some(binding.auditor),
            authority_basis: Some(binding.authority_basis.clone()),
        },
        &binding.institution,
    ))
}

/// Checks, at admission, what a request names beyond its issuer: the scope its
/// institution must own, or the binding and authority basis a rebind installs.
fn admit_action(
    view: &SimulationView<'_>,
    state: &FiscalState,
    request: &FiscalActionRequest,
    institution: &EntityRef,
) -> Result<(), CanwuError> {
    match &request.action {
        FiscalAction::RebindAuthority {
            authority_binding_id,
            authorized_actor,
            acting_actor,
            authority_basis,
        } => {
            if *authority_binding_id != request.authority_binding_id {
                return Err(invalid_authority(
                    "a fiscal authority rebind must be issued under the binding it replaces",
                ));
            }
            if acting_actor.is_some() != authority_basis.is_some()
                || acting_actor.is_some() && acting_actor == authorized_actor
            {
                return Err(invalid_payload(
                    "fiscal acting actors require one exact authority basis and differ from the authorized actor",
                ));
            }
            admit_new_basis(view, authority_basis.as_ref())
        }
        FiscalAction::RebindAudit {
            audit_binding_id,
            auditor,
            authority_basis,
        } => {
            if auditor.is_some() != authority_basis.is_some()
                || state.authority_bindings.contains_key(audit_binding_id)
            {
                return Err(invalid_payload(
                    "a fiscal audit rebind names an auditor together with one exact authority basis, under an ID no authority binding uses",
                ));
            }
            match state.audit_bindings.get(audit_binding_id) {
                Some(existing) if existing.institution != *institution => Err(invalid_authority(
                    "a fiscal audit binding is rebound only from its audited institution",
                )),
                None if auditor.is_none() => Err(not_found("fiscal audit binding is unavailable")),
                _ => admit_new_basis(view, authority_basis.as_ref()),
            }
        }
        action => {
            validate_action_scope_authority(state, action, institution)?;
            if let FiscalAction::OpenAssessment {
                commutation_quote: Some(reference),
                ..
            } = action
            {
                // A quote of a kind the plugin was not configured to read is as
                // unavailable as one that does not exist. Only a version whose answer
                // no seal can change qualifies, so a sealed run admits exactly what
                // its replay admits.
                match view.replay_stable_domain_record_version(reference) {
                    Ok(Some(_)) => {}
                    Ok(None) => {
                        return Err(not_found(
                            "fiscal commutation quote must be the current or initial-scenario version of its record",
                        ));
                    }
                    Err(error) if error.code == ErrorCode::UndeclaredStateRead => {
                        return Err(not_found(
                            "fiscal commutation quote kind is not readable by FiscalPlugin",
                        ));
                    }
                    Err(error) => return Err(error),
                }
            }
            Ok(())
        }
    }
}

/// Admits the authority basis a rebind would install: the current version of
/// a record of an external kind the plugin was configured to read.
fn admit_new_basis(
    view: &SimulationView<'_>,
    basis: Option<&DomainRecordVersionRef>,
) -> Result<(), CanwuError> {
    let Some(basis) = basis else {
        return Ok(());
    };
    let undeclared = || not_found("fiscal authority basis kind is not declared to FiscalPlugin");
    if is_fiscal_record_kind(&basis.record.kind) {
        return Err(undeclared());
    }
    match require_current_basis(view, basis, FISCAL_AUTHORITY_BASIS_NOT_CURRENT) {
        Err(error) if error.code == ErrorCode::UndeclaredStateRead => Err(undeclared()),
        result => result,
    }
}

/// Bounds, at admission, everything an action carries into the state: the
/// identifiers of what it creates, the references it names, its free text,
/// and its set sizes. The request as a whole is bounded before it is decoded.
fn validate_action_bounds(action: &FiscalAction) -> Result<(), CanwuError> {
    let created =
        |value: &str, label: &str| validate_identifier(value, label).map_err(as_invalid_payload);
    let reference = |value: &str| bounded(value, MAX_FISCAL_IDENTIFIER_BYTES, "reference");
    let unit = |value: &str| bounded(value, MAX_FISCAL_IDENTIFIER_BYTES, "unit");
    let text = |value: &str, label: &str| bounded(value, MAX_FISCAL_ACTION_TEXT_BYTES, label);
    match action {
        FiscalAction::ChangeAdoption {
            adoption_id,
            rule_id,
            scope_binding_id,
            ..
        } => {
            created(adoption_id, "fiscal adoption")?;
            reference(rule_id)?;
            reference(scope_binding_id)
        }
        FiscalAction::ApplyTransition {
            transition_id,
            target_scope_bindings,
        } => {
            reference(transition_id)?;
            if target_scope_bindings.len() > MAX_FISCAL_REFERENCES_PER_DEFINITION {
                return Err(invalid_payload(
                    "fiscal transition binds more target rules than a definition may reference",
                ));
            }
            target_scope_bindings
                .iter()
                .try_for_each(|(rule_id, scope_id)| reference(rule_id).and(reference(scope_id)))
        }
        FiscalAction::OpenAssessment {
            assessment_id,
            rule_id,
            scope_binding_id,
            accounting_cycle_id,
            unit: assessed_unit,
            ..
        } => {
            created(assessment_id, "fiscal assessment")?;
            reference(rule_id)?;
            reference(scope_binding_id)?;
            created(accounting_cycle_id, "fiscal accounting cycle")?;
            unit(assessed_unit)
        }
        FiscalAction::GrantRemission {
            remission_id,
            assessment_id,
            reason,
            ..
        } => {
            created(remission_id, "fiscal remission")?;
            reference(assessment_id)?;
            text(reason, "remission reason")
        }
        FiscalAction::AuthorizeExecution {
            request_id,
            assessment_id,
            unit: requested_unit,
            purpose,
            ..
        } => {
            created(request_id, "fiscal execution request")?;
            reference(assessment_id)?;
            unit(requested_unit)?;
            text(purpose, "execution purpose")
        }
        FiscalAction::RecordAudit {
            audit_id,
            target_id,
            finding,
            evidence,
            ..
        } => {
            created(audit_id, "fiscal audit")?;
            reference(target_id)?;
            text(finding, "audit finding")?;
            if evidence.len() > MAX_FISCAL_EVIDENCE_PER_RECORD {
                return Err(invalid_payload(
                    "fiscal audit cites more evidence than one record may hold",
                ));
            }
            Ok(())
        }
        FiscalAction::RebindAuthority {
            authority_binding_id,
            ..
        } => reference(authority_binding_id),
        FiscalAction::RebindAudit {
            audit_binding_id, ..
        } => created(audit_binding_id, "fiscal audit binding"),
    }
}

fn bounded(value: &str, limit: usize, label: &str) -> Result<(), CanwuError> {
    if value.len() > limit {
        return Err(invalid_payload(&format!(
            "fiscal action {label} is longer than {limit} bytes"
        )));
    }
    Ok(())
}

fn validate_action_scope_authority(
    state: &FiscalState,
    action: &FiscalAction,
    institution: &EntityRef,
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
            .ok_or_else(|| not_found("fiscal action names an unknown assessment"))?,
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
                return Err(not_found("fiscal audit target is unavailable"));
            }
        }
        // A transition was checked above, and a rebind names a binding rather
        // than a scope; admission checks it separately.
        FiscalAction::ApplyTransition { .. }
        | FiscalAction::RebindAuthority { .. }
        | FiscalAction::RebindAudit { .. } => return Ok(()),
    };
    let scope = state
        .scope_bindings
        .get(scope_id)
        .ok_or_else(|| not_found("fiscal action scope is unavailable"))?;
    if &scope.institution != institution {
        return Err(invalid_authority(
            "fiscal authority binding does not own the action scope",
        ));
    }
    Ok(())
}

fn actor_role(binding: &FiscalAuthorityBinding, actor: PersonId) -> Option<FiscalIssuerRole> {
    if binding.authorized_actor == Some(actor) {
        Some(FiscalIssuerRole::Principal)
    } else if binding.acting_actor == Some(actor) {
        Some(FiscalIssuerRole::Acting)
    } else {
        None
    }
}

/// Resolves the role the command issuer holds under an authority binding and
/// the person that role names.
///
/// An institution-origin decision is resolved by its responsible actor: the
/// binding's authorized actor holds the principal role and its acting actor
/// the acting role, which never admits a rebind. Only for a rebind, and only
/// with no responsible actor at all, does the institution deciding as a body
/// hold the institution role, so that it can replace an authorized actor who
/// can no longer act. Any other responsible actor holds no role.
fn authority_role(
    context: &CommandContext,
    binding: &FiscalAuthorityBinding,
    rebind: bool,
) -> Result<(FiscalIssuerRole, Option<PersonId>), CanwuError> {
    match (&context.issuer, context.decision_controller_id.as_deref()) {
        (Issuer::Actor(actor), None) => actor_role(binding, *actor)
            .map(|role| (role, Some(*actor)))
            .ok_or_else(|| {
                invalid_authority(
                    "fiscal action requires its bound actor or validated decision controller",
                )
            }),
        (Issuer::Human(issuer) | Issuer::Ai(issuer), Some(controller)) if issuer == controller => {
            if context.authority.command_subject.as_ref() != Some(&binding.institution) {
                return Err(invalid_authority(
                    "fiscal decision subject does not match the bound institution",
                ));
            }
            match &context.authority.decision_origin {
                DecisionOrigin::Actor { actor } => {
                    actor_role(binding, *actor).map(|role| (role, Some(*actor)))
                }
                DecisionOrigin::Institution {
                    institution,
                    responsible_actor,
                } if institution == &binding.institution => if *responsible_actor
                    == binding.authorized_actor
                {
                    Some(FiscalIssuerRole::Principal)
                } else if responsible_actor.is_some() && *responsible_actor == binding.acting_actor
                {
                    Some(FiscalIssuerRole::Acting)
                } else if rebind && responsible_actor.is_none() {
                    Some(FiscalIssuerRole::Institution)
                } else {
                    None
                }
                .map(|role| (role, *responsible_actor)),
                _ => None,
            }
            .ok_or_else(|| {
                invalid_authority("fiscal decision origin does not satisfy the authority binding")
            })
        }
        _ => Err(invalid_authority(
            "fiscal action requires its bound actor or validated decision controller",
        )),
    }
}

/// Whether the command comes from an audit binding's auditor: directly, or as
/// a validated decision on the audited institution that the auditor made or
/// is the responsible actor of.
fn issuer_is_auditor(context: &CommandContext, binding: &FiscalAuditBinding) -> bool {
    match (&context.issuer, context.decision_controller_id.as_deref()) {
        (Issuer::Actor(actor), None) => *actor == binding.auditor,
        (Issuer::Human(issuer) | Issuer::Ai(issuer), Some(controller)) if issuer == controller => {
            context.authority.command_subject.as_ref() == Some(&binding.institution)
                && match &context.authority.decision_origin {
                    DecisionOrigin::Actor { actor } => *actor == binding.auditor,
                    DecisionOrigin::Institution {
                        responsible_actor, ..
                    } => *responsible_actor == Some(binding.auditor),
                    _ => false,
                }
        }
        _ => false,
    }
}

/// Requires `basis` to be the current version of its record, or rejects with
/// `reason`. The answer comes from the current-version provenance index, which
/// a seal cannot change, so a sealed run decides exactly as its replay does.
fn require_current_basis(
    view: &SimulationView<'_>,
    basis: &DomainRecordVersionRef,
    reason: &str,
) -> Result<(), CanwuError> {
    if view.current_domain_record_version(&basis.record)?.as_ref() == Some(basis) {
        Ok(())
    } else {
        Err(invalid_authority(reason))
    }
}

/// Re-checks, against the state an action settles on, that its issuer still
/// holds the role admission resolved. A rebind settled since then may have
/// replaced the binding's actors, and the authority basis of an acting actor
/// or auditor may no longer be current. The basis a rebind installs must also
/// still be current.
fn settlement_authority(
    view: &SimulationView<'_>,
    state: &FiscalState,
    admitted: &AdmittedFiscalAction,
) -> Result<(), CanwuError> {
    let issuer = &admitted.issuer;
    let not_bound = || invalid_authority(FISCAL_ISSUER_NOT_BOUND);
    let action = &admitted.request.action;
    let rebind = matches!(
        action,
        FiscalAction::RebindAuthority { .. } | FiscalAction::RebindAudit { .. }
    );
    // The roles admit different actions, so an item that pairs a role with an
    // action admission would never have paired is not what admission queued.
    let role_admits_action = match issuer.role {
        FiscalIssuerRole::Principal => true,
        FiscalIssuerRole::Acting => !rebind,
        FiscalIssuerRole::Institution => rebind,
        FiscalIssuerRole::Auditor => matches!(action, FiscalAction::RecordAudit { .. }),
    };
    if !role_admits_action || issuer.binding_id != admitted.request.authority_binding_id {
        return Err(not_bound());
    }
    if issuer.role == FiscalIssuerRole::Auditor {
        let binding = state
            .audit_bindings
            .get(&issuer.binding_id)
            .filter(|binding| {
                issuer.actor == Some(binding.auditor)
                    && issuer.authority_basis.as_ref() == Some(&binding.authority_basis)
            })
            .ok_or_else(not_bound)?;
        require_current_basis(
            view,
            &binding.authority_basis,
            FISCAL_AUTHORITY_BASIS_NOT_CURRENT,
        )?;
    } else {
        let binding = state
            .authority_bindings
            .get(&issuer.binding_id)
            .ok_or_else(not_bound)?;
        match issuer.role {
            FiscalIssuerRole::Principal if binding.authorized_actor != issuer.actor => {
                return Err(not_bound());
            }
            FiscalIssuerRole::Acting => {
                let basis = issuer
                    .authority_basis
                    .as_ref()
                    .filter(|basis| binding.authority_basis.as_ref() == Some(*basis))
                    .ok_or_else(|| invalid_authority(FISCAL_ACTING_BASIS_NOT_CURRENT))?;
                if issuer.actor.is_none() || binding.acting_actor != issuer.actor {
                    return Err(not_bound());
                }
                require_current_basis(view, basis, FISCAL_ACTING_BASIS_NOT_CURRENT)?;
            }
            // The institution role names no person. On the running binding
            // the same decision holds it, or the principal role once the
            // binding names no authorized actor; either admits a rebind.
            FiscalIssuerRole::Institution if issuer.actor.is_some() => {
                return Err(not_bound());
            }
            FiscalIssuerRole::Principal
            | FiscalIssuerRole::Institution
            | FiscalIssuerRole::Auditor => {}
        }
    }
    match action {
        FiscalAction::RebindAuthority {
            authority_basis: Some(basis),
            ..
        }
        | FiscalAction::RebindAudit {
            authority_basis: Some(basis),
            ..
        } => require_current_basis(view, basis, FISCAL_AUTHORITY_BASIS_NOT_CURRENT),
        _ => Ok(()),
    }
}

/// Rejects a command once the state holds every outcome it may ever hold, so
/// no further action can be recorded.
fn ensure_action_outcome_slot(state: &FiscalState) -> Result<(), CanwuError> {
    if state.action_outcomes.len() >= MAX_FISCAL_ACTION_OUTCOMES {
        return Err(CanwuError::new(
            ErrorCode::ValueOutOfRange,
            "fiscal action outcome capacity is insufficient",
        ));
    }
    Ok(())
}

/// One admitted ingress item addressed to this plugin, before it is decoded.
struct AdmittedPacket<'a> {
    ingress: IngressId,
    packet_type: &'a str,
    payload: &'a Value,
    cause: Option<&'a CauseRef>,
}

/// Lists the boundary's admitted ingress that this plugin settles, in
/// admission order, without decoding any payload.
fn admitted_fiscal_packets<'a>(
    view: &'a SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<Vec<AdmittedPacket<'a>>, CanwuError> {
    let mut packets = Vec::new();
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
        if plugin == PLUGIN_NAME
            && matches!(
                packet_type.as_str(),
                FISCAL_ACTION_INGRESS
                    | FISCAL_EXECUTION_RECEIPT_INGRESS
                    | FISCAL_HISTORICAL_CONTEXT_INGRESS
            )
        {
            packets.push(AdmittedPacket {
                ingress: *ingress_id,
                packet_type,
                payload,
                cause: ingress.cause.as_ref(),
            });
        }
    }
    Ok(packets)
}

/// Decodes the admitted packets and checks every action against the command
/// that caused it, before any of them changes the state.
fn decode_fiscal_packets(
    view: &SimulationView<'_>,
    packets: &[AdmittedPacket<'_>],
) -> Result<Vec<FiscalPacket>, CanwuError> {
    packets
        .iter()
        .map(|packet| {
            Ok(match packet.packet_type {
                FISCAL_ACTION_INGRESS => {
                    let admitted: AdmittedFiscalAction =
                        decode(packet.payload, "admitted fiscal action")?;
                    validate_admitted_action(view, packet.cause, &admitted)?;
                    FiscalPacket::Action(Box::new(admitted))
                }
                FISCAL_EXECUTION_RECEIPT_INGRESS => FiscalPacket::Receipt(
                    packet.ingress,
                    decode(packet.payload, "fiscal execution receipt")?,
                ),
                _ => FiscalPacket::HistoricalContext(decode(
                    packet.payload,
                    "fiscal historical context",
                )?),
            })
        })
        .collect()
}

/// One event a settlement emits, with the entities it concerns.
struct SettledEvent {
    event_type: &'static str,
    summary: String,
    affected: Vec<EntityRef>,
}

/// One pass over a boundary's packets: the state it runs on and what it has
/// produced so far.
struct Settlement {
    state: FiscalState,
    /// The procedure revision the state had when the boundary began.
    entry_revision: u64,
    /// Actions that reached the copy and validation of the state.
    applied_actions: usize,
    overwrites: BoundaryOverwrites,
    /// The size the state record will have, when this pass holds every
    /// action to `budget`.
    measured: Option<MeasuredState>,
    /// The serialized size settlement may leave the state record at.
    budget: usize,
    /// Whether any packet changed the state.
    changed: bool,
    events: Vec<SettledEvent>,
}

/// Settles the boundary's fiscal ingress in admission order.
///
/// Every action is compared with the procedure revision the state had when the
/// boundary began, so actions admitted against one revision settle together,
/// each applied to the state the previous one left. An action that fails its
/// own checks is recorded as a rejected outcome and does not affect the
/// actions after it.
fn settle_fiscal_ingress(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    // A boundary that admitted nothing for this plugin proposes nothing, so it
    // returns before the fiscal records are decoded.
    let packets = admitted_fiscal_packets(view, context)?;
    if packets.is_empty() {
        return Ok(BoundaryProposal::default());
    }
    let Some(catalog) = committed_fiscal_catalog(view)? else {
        return Ok(BoundaryProposal::default());
    };
    // A report plan means something only in a state a boundary has written.
    // One that came with a scenario is dropped here, so the aggregate system
    // plans every report whole.
    let authored = state_writer(view, context)? == StateWriter::Scenario;
    let committed_state = || -> Result<Option<(u64, FiscalState)>, CanwuError> {
        Ok(committed_fiscal_state(view)?.map(|(version, mut state)| {
            if authored {
                state.report_plan = FiscalReportPlan::default();
            }
            (version, state)
        }))
    };
    let Some((version, state)) = committed_state()? else {
        return Ok(BoundaryProposal::default());
    };
    let packets = decode_fiscal_packets(view, &packets)?;
    let budget = settlement_budget();
    let mut settlement = settle_packets(view, context, &catalog, state, &packets, None, budget)?;
    let mut draft = None;
    if settlement.changed {
        let (unmeasured_draft, bytes) = settlement.state.encode_record()?;
        if bytes.saturating_add(STATE_BUDGET_MARGIN_BYTES) <= budget {
            draft = Some(unmeasured_draft);
        } else {
            // The state is close to its byte budget. Settle the same packets
            // again from the committed state, this time measuring every
            // action, so that the one that does not fit is rejected on its
            // own instead of failing the boundary for all of them.
            let Some((_, state)) = committed_state()? else {
                return Ok(BoundaryProposal::default());
            };
            let measured = measured_state(&state, &catalog, context.at)?;
            settlement = settle_packets(
                view,
                context,
                &catalog,
                state,
                &packets,
                Some(measured),
                budget,
            )?;
            if settlement.changed {
                draft = Some(settlement.state.record_draft()?);
            }
        }
    }
    let mut directives = Vec::with_capacity(settlement.events.len() + 1);
    if let Some(record) = draft {
        settlement.state.validate(&catalog)?;
        directives.push(BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Update {
                record,
                expected_version: version,
            },
            summary: "Settled fiscal ingress".to_owned(),
        });
    }
    directives.extend(
        settlement
            .events
            .into_iter()
            .map(|event| BoundaryDirective::Emit {
                event_type: event.event_type.to_owned(),
                summary: event.summary,
                affected: event.affected,
            }),
    );
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

/// Settles the packets on `state`, in admission order.
///
/// With `measured`, the size the state record will have, the pass also holds
/// every action to `budget`: an action whose result would not fit is rejected,
/// and one whose outcome would not fit is dropped. Without it nothing is
/// measured; the caller measures the result once and repeats the pass with
/// measurement only near the budget.
fn settle_packets(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    catalog: &CompiledFiscalCatalog,
    state: FiscalState,
    packets: &[FiscalPacket],
    measured: Option<MeasuredState>,
    budget: usize,
) -> Result<Settlement, CanwuError> {
    let mut settlement = Settlement {
        entry_revision: state.procedure_revision,
        state,
        applied_actions: 0,
        overwrites: BoundaryOverwrites::default(),
        measured,
        budget,
        changed: false,
        events: Vec::new(),
    };
    for packet in packets {
        match packet {
            FiscalPacket::Action(admitted) => {
                settlement.settle_action(view, catalog, admitted, context.at)?;
            }
            FiscalPacket::Receipt(ingress_id, packet) => {
                if settle_execution_receipt(
                    view,
                    &mut settlement.state,
                    packet.clone(),
                    *ingress_id,
                    context.at,
                    catalog,
                )? {
                    settlement.record_change(
                        catalog,
                        context.at,
                        "canwu.fiscal.execution_receipt_recorded.v1",
                        "Recorded external fiscal execution evidence",
                    )?;
                }
            }
            FiscalPacket::HistoricalContext(packet) => {
                if !catalog.historical_scope.contains(packet.year) {
                    return Err(invalid("fiscal historical context lies outside the pack"));
                }
                settlement.state.historical_context.year = packet.year;
                settlement.state.historical_context.mode = packet.mode;
                settlement.state.historical_context.updated_at = context.at;
                increment_procedure_revision(&mut settlement.state)?;
                settlement.record_change(
                    catalog,
                    context.at,
                    "canwu.fiscal.historical_context_changed.v1",
                    "Changed the explicit fiscal historical context",
                )?;
            }
        }
    }
    Ok(settlement)
}

impl Settlement {
    /// Settles one admitted action: records its outcome, or drops the action
    /// when no outcome can be recorded for it.
    fn settle_action(
        &mut self,
        view: &SimulationView<'_>,
        catalog: &CompiledFiscalCatalog,
        admitted: &AdmittedFiscalAction,
        at: SimTime,
    ) -> Result<(), CanwuError> {
        let request = &admitted.request;
        if self.state.action_outcomes.contains_key(&request.action_id) {
            return Ok(());
        }
        if self.state.action_outcomes.len() >= MAX_FISCAL_ACTION_OUTCOMES {
            self.events.push(dropped_action(
                &self.state,
                request,
                "no outcome slot is left",
            ));
            return Ok(());
        }
        let verdict = if request.expected_procedure_revision == self.entry_revision {
            settlement_authority(view, &self.state, admitted)
                .and_then(|()| self.apply(catalog, admitted, at))
        } else {
            Err(stale(STALE_ACTION))
        };
        let outcome = action_outcome(admitted, &verdict, at);
        // The outcome of an applied action was measured with its state. A
        // rejected one adds its entry, and the procedure revision it advances
        // may gain a digit: in the state, and in the report plan if the
        // aggregate system will stamp one with it.
        if verdict.is_err()
            && let Some(measured) = self.measured.as_mut()
        {
            let revisions = 1 + usize::from(measured.plan_stamped);
            let grown = measured
                .bytes
                .saturating_add(outcome_entry_bytes(
                    &outcome,
                    self.state.action_outcomes.is_empty(),
                )?)
                .saturating_add(revisions * revision_growth(self.state.procedure_revision));
            if grown > self.budget {
                self.events.push(dropped_action(
                    &self.state,
                    request,
                    "the fiscal state byte budget cannot hold its outcome",
                ));
                return Ok(());
            }
            measured.bytes = grown;
        }
        let affected = if verdict.is_ok() {
            rebound_entities(&self.state, request)
        } else {
            Vec::new()
        };
        self.state
            .action_outcomes
            .insert(request.action_id.clone(), outcome);
        increment_procedure_revision(&mut self.state)?;
        self.changed = true;
        self.events.push(SettledEvent {
            event_type: ACTION_SETTLED_EVENT,
            summary: "Settled one fiscal procedure action".to_owned(),
            affected,
        });
        Ok(())
    }

    /// Applies a current, authorized action, unless the boundary has already
    /// applied as many as it may. Only an action that gets this far, to the
    /// copy and validation of the state, uses the boundary's limit.
    fn apply(
        &mut self,
        catalog: &CompiledFiscalCatalog,
        admitted: &AdmittedFiscalAction,
        at: SimTime,
    ) -> Result<(), CanwuError> {
        if self.applied_actions >= MAX_FISCAL_ACTIONS_PER_BOUNDARY {
            return Err(CanwuError::new(
                ErrorCode::ValueOutOfRange,
                FISCAL_BOUNDARY_ACTION_LIMIT,
            ));
        }
        self.applied_actions += 1;
        let budget = self.budget;
        apply_settled_action(
            &mut self.state,
            catalog,
            admitted,
            at,
            &mut self.overwrites,
            self.measured.as_mut().map(|measured| (measured, budget)),
        )
    }

    /// Records a change that a receipt or a historical-context change made.
    fn record_change(
        &mut self,
        catalog: &CompiledFiscalCatalog,
        at: SimTime,
        event_type: &'static str,
        summary: &str,
    ) -> Result<(), CanwuError> {
        self.changed = true;
        self.events.push(SettledEvent {
            event_type,
            summary: summary.to_owned(),
            affected: Vec::new(),
        });
        if let Some(measured) = self.measured.as_mut() {
            *measured = measured_state(&self.state, catalog, at)?;
        }
        Ok(())
    }
}

/// The size the state record will have once the derived systems of the same
/// boundary have refreshed it.
#[derive(Clone, Copy)]
struct MeasuredState {
    /// The serialized size of the record's payload.
    bytes: usize,
    /// Whether the aggregate system will write a report plan, which it stamps
    /// with the procedure revision the boundary ends at.
    plan_stamped: bool,
}

fn measured_state(
    state: &FiscalState,
    catalog: &CompiledFiscalCatalog,
    at: SimTime,
) -> Result<MeasuredState, CanwuError> {
    let mut derived = state.clone();
    derived.transition_candidates = compute_transition_candidates(&derived, catalog, at);
    let aggregates = compute_aggregates(&derived, catalog)?;
    // Whether an observer can receive knowledge is not read here; see
    // `REPORT_PLAN_RESERVE_BYTES`.
    let plan = next_report_plan(&derived, &aggregates, &BTreeSet::new());
    let plan_stamped = plan.is_some();
    if let Some(plan) = plan {
        derived.report_plan = plan;
    }
    derived.aggregates = aggregates;
    Ok(MeasuredState {
        bytes: derived.encode_record()?.1,
        plan_stamped,
    })
}

/// The terminal outcome of one action, as its verdict decides it.
fn action_outcome(
    admitted: &AdmittedFiscalAction,
    verdict: &Result<(), CanwuError>,
    at: SimTime,
) -> FiscalActionOutcome {
    let (disposition, reason) = match verdict {
        Ok(()) => (FiscalActionDisposition::Applied, "applied".to_owned()),
        Err(error) => (FiscalActionDisposition::Rejected, error.to_string()),
    };
    FiscalActionOutcome {
        action_id: admitted.request.action_id.clone(),
        disposition,
        reason,
        command: admitted.command,
        settled_at: at,
    }
}

/// The bytes one outcome adds to the serialized state: its map key in
/// quotes, a colon, its body, and a comma unless it is the map's `first`
/// entry. An action ID has no character JSON escapes.
fn outcome_entry_bytes(outcome: &FiscalActionOutcome, first: bool) -> Result<usize, CanwuError> {
    Ok(serde_json::to_vec(outcome).map_err(encode_error)?.len()
        + outcome.action_id.len()
        + 3
        + usize::from(!first))
}

/// The digits the procedure revision gains when it advances by one.
fn revision_growth(revision: u64) -> usize {
    let digits = |value: u64| value.checked_ilog10().map_or(1, |log| log + 1);
    usize::from(digits(revision.saturating_add(1)) > digits(revision))
}

/// The serialized size settlement may leave the state record at: the state's
/// byte budget less what is kept for the report plan.
fn settlement_budget() -> usize {
    #[cfg(test)]
    if let Some(bytes) = tests::SETTLEMENT_BUDGET.with(std::cell::Cell::get) {
        return bytes;
    }
    MAX_FISCAL_STATE_JSON_BYTES - REPORT_PLAN_RESERVE_BYTES
}

/// Reports an action that settlement could give no outcome: it changes
/// nothing, keeps its action ID, and this event is its only record. The
/// event names the institution the action was issued for.
fn dropped_action(state: &FiscalState, request: &FiscalActionRequest, cause: &str) -> SettledEvent {
    let institution = state
        .authority_bindings
        .get(&request.authority_binding_id)
        .map(|binding| &binding.institution)
        .or_else(|| {
            state
                .audit_bindings
                .get(&request.authority_binding_id)
                .map(|binding| &binding.institution)
        });
    SettledEvent {
        event_type: ACTION_DROPPED_EVENT,
        summary: format!(
            "Dropped fiscal action {} without an outcome: {cause}",
            request.action_id
        ),
        affected: institution
            .and_then(registered_entity)
            .into_iter()
            .collect(),
    }
}

/// An entity an event may name: one of the engine's entity registry, which
/// never loses an identity. The engine fails a boundary whose event names an
/// entity that does not exist, and the plugin cannot tell whether a domain
/// record that was an institution when a command was admitted still exists
/// when its action settles, so a domain-record institution is left out.
fn registered_entity(entity: &EntityRef) -> Option<EntityRef> {
    (!matches!(entity, EntityRef::Domain(_))).then(|| entity.clone())
}

/// The persons a rebind names.
fn named_persons(action: &FiscalAction) -> Vec<PersonId> {
    match action {
        FiscalAction::RebindAuthority {
            authorized_actor,
            acting_actor,
            ..
        } => authorized_actor
            .iter()
            .chain(acting_actor)
            .copied()
            .collect(),
        FiscalAction::RebindAudit { auditor, .. } => auditor.iter().copied().collect(),
        _ => Vec::new(),
    }
}

/// The institution and persons an applied rebind concerns, and nothing for
/// any other action. The engine checks the affected entities of an emitted
/// event against its entity registry when the boundary commits, so a rebind
/// can never commit a binding to a person who does not exist.
fn rebound_entities(state: &FiscalState, request: &FiscalActionRequest) -> Vec<EntityRef> {
    let persons = named_persons(&request.action);
    if !matches!(
        request.action,
        FiscalAction::RebindAuthority { .. } | FiscalAction::RebindAudit { .. }
    ) {
        return Vec::new();
    }
    let mut affected: Vec<_> = state
        .authority_bindings
        .get(&request.authority_binding_id)
        .and_then(|binding| registered_entity(&binding.institution))
        .into_iter()
        .chain(persons.into_iter().map(EntityRef::Person))
        .collect();
    affected.sort();
    affected.dedup();
    affected
}

fn settle_execution_receipt(
    view: &SimulationView<'_>,
    state: &mut FiscalState,
    packet: FiscalExecutionReceiptPacket,
    ingress_id: canwu_api::IngressId,
    at: SimTime,
    catalog: &CompiledFiscalCatalog,
) -> Result<bool, CanwuError> {
    // A resent receipt cites the same exact versions, so it would derive the
    // same content. Settle it unchanged before its evidence is checked again,
    // since that evidence may no longer be current.
    if state
        .execution_receipts
        .get(&packet.receipt_id)
        .is_some_and(|existing| {
            existing.request_id == packet.request_id
                && existing.external_evidence == packet.external_evidence
        })
    {
        return Ok(false);
    }
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
        // An earlier version's body and time can depend on what a seal kept,
        // so the receipt would settle differently in a compact run than in
        // its exact replay.
        let (record, established_at) = view
            .replay_stable_domain_record_version(evidence)?
            .ok_or_else(|| {
                invalid(
                    "fiscal execution receipt must cite the current version of its external evidence",
                )
            })?;
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

/// Applies one current, authorized action to a copy of the state and keeps the
/// copy only if the result validates and overwrites nothing that an earlier
/// action of this boundary changed. The issuer of the later action decided
/// against the state before that change, so letting it win would be a lost
/// update; reading what an earlier action created or changed is allowed.
///
/// With `measured`, the measure of the state and the budget it is held to,
/// the result must also fit that budget together with the action's outcome
/// and the derived fields that follow from it; the measure of the result then
/// replaces the caller's.
fn apply_settled_action(
    state: &mut FiscalState,
    catalog: &CompiledFiscalCatalog,
    admitted: &AdmittedFiscalAction,
    at: SimTime,
    overwrites: &mut BoundaryOverwrites,
    measured: Option<(&mut MeasuredState, usize)>,
) -> Result<(), CanwuError> {
    let mut next = state.clone();
    apply_action(&mut next, catalog, &admitted.request, &admitted.issuer, at)?;
    next.validate(catalog)?;
    let adoptions = changed_keys(&state.adoptions, &next.adoptions);
    let authority_bindings = changed_keys(&state.authority_bindings, &next.authority_bindings);
    let audit_bindings = changed_keys(&state.audit_bindings, &next.audit_bindings);
    if !adoptions.is_disjoint(&overwrites.adoptions)
        || !authority_bindings.is_disjoint(&overwrites.authority_bindings)
        || !audit_bindings.is_disjoint(&overwrites.audit_bindings)
    {
        return Err(stale(STALE_OVERWRITE));
    }
    if let Some((measured, budget)) = measured {
        let mut settled = next.clone();
        settled.action_outcomes.insert(
            admitted.request.action_id.clone(),
            action_outcome(admitted, &Ok(()), at),
        );
        increment_procedure_revision(&mut settled)?;
        let settled = measured_state(&settled, catalog, at)?;
        if settled.bytes > budget {
            return Err(invalid(FISCAL_STATE_BUDGET_EXHAUSTED));
        }
        *measured = settled;
    }
    overwrites.adoptions.extend(adoptions);
    overwrites.authority_bindings.extend(authority_bindings);
    overwrites.audit_bindings.extend(audit_bindings);
    *state = next;
    Ok(())
}

/// Keys whose entry was added, replaced, or removed between two maps.
fn changed_keys<T: PartialEq>(
    before: &std::collections::BTreeMap<String, T>,
    after: &std::collections::BTreeMap<String, T>,
) -> BTreeSet<String> {
    after
        .iter()
        .filter(|(key, value)| before.get(*key) != Some(*value))
        .map(|(key, _)| key)
        .chain(before.keys().filter(|key| !after.contains_key(*key)))
        .cloned()
        .collect()
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
    issuer: &FiscalActionIssuer,
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
                    entered_by: Some(issuer.clone()),
                },
            );
        }
        FiscalAction::RebindAuthority {
            authority_binding_id,
            authorized_actor,
            acting_actor,
            authority_basis,
        } => {
            let binding = state
                .authority_bindings
                .get_mut(authority_binding_id)
                .filter(|_| *authority_binding_id == request.authority_binding_id)
                .ok_or_else(|| {
                    invalid_authority(
                        "a fiscal authority rebind must be issued under the binding it replaces",
                    )
                })?;
            binding.authorized_actor = *authorized_actor;
            binding.acting_actor = *acting_actor;
            binding.authority_basis.clone_from(authority_basis);
            binding.rebound_by = Some(rebinding(request, issuer, at));
        }
        FiscalAction::RebindAudit {
            audit_binding_id,
            auditor,
            authority_basis,
        } => {
            let institution = state
                .authority_bindings
                .get(&request.authority_binding_id)
                .map(|binding| binding.institution.clone())
                .ok_or_else(|| invalid_authority("fiscal authority binding is unavailable"))?;
            if state
                .audit_bindings
                .get(audit_binding_id)
                .is_some_and(|existing| existing.institution != institution)
            {
                return Err(invalid_authority(
                    "a fiscal audit binding is rebound only from its audited institution",
                ));
            }
            match (auditor, authority_basis) {
                (Some(auditor), Some(authority_basis)) => {
                    state.audit_bindings.insert(
                        audit_binding_id.clone(),
                        FiscalAuditBinding {
                            id: audit_binding_id.clone(),
                            institution,
                            auditor: *auditor,
                            authority_basis: authority_basis.clone(),
                            rebound_by: Some(rebinding(request, issuer, at)),
                        },
                    );
                }
                (None, None) => {
                    state
                        .audit_bindings
                        .remove(audit_binding_id)
                        .ok_or_else(|| not_found("fiscal audit binding is unavailable"))?;
                }
                _ => {
                    return Err(invalid(
                        "a fiscal audit rebind names an auditor together with one exact authority basis",
                    ));
                }
            }
        }
    }
    Ok(())
}

fn rebinding(
    request: &FiscalActionRequest,
    issuer: &FiscalActionIssuer,
    at: SimTime,
) -> FiscalRebinding {
    FiscalRebinding {
        action_id: request.action_id.clone(),
        issuer: issuer.clone(),
        rebound_at: at,
    }
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
    update_derived(view, context, |state, catalog, _| {
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
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    update_derived(view, context, |state, catalog, written_now| {
        let aggregates = compute_aggregates(state, catalog)?;
        let aggregates_changed = state.aggregates != aggregates;
        // The report plan follows a change of the state. A state that no
        // boundary has written, with aggregates that are already right, is
        // left as it is, and its first report comes with its first change.
        let plan = if written_now || aggregates_changed {
            next_report_plan(state, &aggregates, &unreachable_observers(view, state)?)
        } else {
            None
        };
        if !aggregates_changed && plan.is_none() {
            return Ok(None);
        }
        state.aggregates = aggregates;
        if let Some(plan) = plan {
            state.report_plan = plan;
        }
        Ok(Some("Aggregated fiscal state"))
    })
}

/// Whether the engine would refuse to publish knowledge to this observer.
///
/// It refuses a holder who does not exist or is dead. The state record
/// references every observer's person, so the person exists; what is left is
/// the person's life state. The aggregate system and the report system both
/// run after the last phase that can change it, so they read the same answer
/// the engine checks a publication against.
fn observer_is_unreachable(
    view: &SimulationView<'_>,
    observer: &FiscalObserverBinding,
) -> Result<bool, CanwuError> {
    Ok(view
        .person_availability(observer.actor)?
        .is_some_and(|availability| availability.life == LifeState::Dead))
}

/// The observer bindings no report can be published to now.
fn unreachable_observers(
    view: &SimulationView<'_>,
    state: &FiscalState,
) -> Result<BTreeSet<String>, CanwuError> {
    let mut unreachable = BTreeSet::new();
    for observer in state.observer_bindings.values() {
        if observer_is_unreachable(view, observer)? {
            unreachable.insert(observer.id.clone());
        }
    }
    Ok(unreachable)
}

/// The subjects of the records on one report line of one observer binding.
///
/// A holder can hold many lines: one for each summary and each part of every
/// observer binding that names its actor. `SimulationView::knowledge_heads`
/// selects records that carry every subject it is given, so each line needs a
/// subject no other line has. A subject is a role and a target, the target
/// has to exist, and the one record every report is about is the fiscal
/// state. The line is therefore told by the roles: the four hexadecimal
/// digits of its number, each a role of its own place, all on the fiscal
/// state record. Two lines differ in at least one place, so neither's
/// subjects contain the other's.
fn report_line_subjects(slot: u32, index: u32) -> Vec<KnowledgeSubject> {
    let line = slot * REPORT_LINES_PER_OBSERVER + index;
    let target = KnowledgeSubjectTarget::DomainRecord(fiscal_state_reference().into_untyped());
    let mut subjects: Vec<_> = (0..REPORT_LINE_PLACES)
        .map(|place| KnowledgeSubject {
            role: report_line_role(place, (line >> (4 * place)) & 0xf),
            target: target.clone(),
        })
        .collect();
    subjects.push(KnowledgeSubject {
        role: REPORT_STATE_ROLE.to_owned(),
        target,
    });
    subjects.sort();
    subjects
}

fn report_line_role(place: u32, digit: u32) -> String {
    format!("report_line_{place}_{digit:x}")
}

/// Publishes the report parts the state's plan lists, each superseding the
/// record its line currently holds.
///
/// The record to supersede is read from the committed knowledge ledger with
/// `SimulationView::knowledge_heads`, never from retained evidence, so a run
/// that sealed its evidence supersedes exactly what its replay does.
fn materialize_reports(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    // A report is published only for a state this boundary changed, so that
    // is established before the fiscal records are decoded.
    let state_reference = fiscal_state_reference().into_untyped();
    if view
        .proposed_domain_record_version(&state_reference)?
        .is_none()
    {
        return Ok(BoundaryProposal::default());
    }
    if view
        .typed_domain_record(&fiscal_catalog_reference())?
        .is_none()
    {
        return Ok(BoundaryProposal::default());
    }
    let Some((_, state)) = committed_fiscal_state(view)? else {
        return Ok(BoundaryProposal::default());
    };
    let reports = planned_reports(&state, context.at);
    if reports.is_empty() {
        return Ok(BoundaryProposal::default());
    }
    // The version the facts were computed from: the last one this boundary
    // proposed.
    let Some(source_version) = view.current_domain_record_version(&state_reference)? else {
        return Ok(BoundaryProposal::default());
    };
    let kind = fiscal_report_knowledge_schema_id().kind;
    let mut directives = Vec::with_capacity(reports.len());
    for report in reports {
        // The plan lists no observer the engine would refuse, because the
        // aggregate system asked the same question of the same state. Asking
        // again keeps a refused publication from ever failing the boundary.
        if observer_is_unreachable(view, report.observer)? {
            continue;
        }
        let holder = &report.observer.knowledge_holder;
        let mut publication = ReportPublication {
            view,
            holder,
            kind: &kind,
            slot: report.slot,
            as_of: context.at,
            confidence_per_mille: report.observer.confidence_per_mille,
            source_version: &source_version,
            records: Vec::with_capacity(report.parts.len() + 1),
        };
        publication.push(0, &FiscalReportRecord::Summary(report.summary))?;
        for part in report.parts {
            publication.push(part.part.saturating_add(1), &FiscalReportRecord::Part(part))?;
        }
        directives.push(BoundaryDirective::PublishKnowledge {
            holder: holder.clone(),
            visibility: StateVisibility::SameBoundary,
            producer_correlation: Some(format!(
                "fiscal-report::{}::{}",
                report.observer.id, context.boundary_id
            )),
            records: publication.records,
            summary: "Publish the changed parts of one holder-relative fiscal report".to_owned(),
        });
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

/// The records one boundary publishes to one observer.
struct ReportPublication<'a, 'view> {
    view: &'a SimulationView<'view>,
    holder: &'a KnowledgeHolderRef,
    kind: &'a KnowledgeRecordKind,
    slot: u32,
    as_of: SimTime,
    confidence_per_mille: u16,
    source_version: &'a DomainRecordVersionRef,
    records: Vec<KnowledgeRecordDraft>,
}

impl ReportPublication<'_, '_> {
    /// Adds the next record of one report line. It supersedes what the line
    /// holds now: normally one record, or none for the line's first.
    fn push(&mut self, index: u32, record: &FiscalReportRecord) -> Result<(), CanwuError> {
        let subjects = report_line_subjects(self.slot, index);
        let supersedes = self
            .view
            .knowledge_heads(self.holder, self.kind, &subjects)?
            .records;
        self.records.push(KnowledgeRecordDraft {
            schema: fiscal_report_knowledge_schema_id(),
            subjects,
            payload: serde_json::to_value(record).map_err(encode_error)?,
            as_of: Some(self.as_of),
            confidence_per_mille: self.confidence_per_mille,
            origin: KnowledgeOrigin {
                method: "fiscal_authority_report_v2".to_owned(),
                evidence: vec![EvidenceRef::DomainRecordVersion(
                    self.source_version.clone(),
                )],
            },
            supersedes,
            contradicts: Vec::new(),
        });
        Ok(())
    }
}

/// Reads the fiscal reports the viewer's holder currently holds, keyed by
/// observer binding ID, and puts each back together from its records.
///
/// A holder has one report for each observer binding that names its actor.
/// Each is what the binding lets the actor see and nothing else; see
/// [`assemble_fiscal_reports`] for how the records are assembled.
pub fn fiscal_reports(
    viewer: &CanwuViewer<'_>,
) -> Result<BTreeMap<String, FiscalReport>, CanwuError> {
    let mut query = KnowledgeQuery {
        schemas: vec![fiscal_report_knowledge_schema_id()],
        limit: MAX_KNOWLEDGE_PAGE_SIZE,
        ..KnowledgeQuery::default()
    };
    let mut payloads = Vec::new();
    loop {
        let page = viewer.query_knowledge(&query)?;
        payloads.extend(page.records.into_iter().map(|record| record.payload));
        let Some(cursor) = page.next else {
            break;
        };
        query.after = Some(cursor);
    }
    assemble_fiscal_reports(&payloads)
}

/// The knowledge schema of fiscal report records, at its one writable
/// version.
///
/// A record of this schema is one [`FiscalReportRecord`]: the summary or one
/// part of one observer's report. Use [`fiscal_reports`] to read whole
/// reports.
#[must_use]
pub fn fiscal_report_knowledge_schema_id() -> KnowledgeSchemaId {
    KnowledgeSchemaId::new(
        KnowledgeRecordKind::new(PLUGIN_NAME, FISCAL_REPORT_KNOWLEDGE),
        FISCAL_REPORT_SCHEMA_VERSION,
    )
}

fn fiscal_report_knowledge_schema() -> PluginKnowledgeSchema {
    PluginKnowledgeSchema {
        id: fiscal_report_knowledge_schema_id(),
        schema_hash: FISCAL_REPORT_SCHEMA_HASH.to_owned(),
        writable: true,
        payload_schema: PayloadSchema::Any,
        subjects: report_subject_schemas(),
    }
}

/// The fiscal state every report record is about, and one role for each
/// digit of each place of a report line number.
fn report_subject_schemas() -> Vec<KnowledgeSubjectSchema> {
    let on_fiscal_state = |role: String, required: bool| KnowledgeSubjectSchema {
        role,
        targets: vec![KnowledgeSubjectTargetKind::Domain(
            DomainRecordKind::for_type::<FiscalStateRecord>(),
        )],
        required,
        multiple: false,
    };
    let mut subjects: Vec<_> = (0..REPORT_LINE_PLACES)
        .flat_map(|place| (0..16).map(move |digit| report_line_role(place, digit)))
        .map(|role| on_fiscal_state(role, false))
        .collect();
    subjects.push(on_fiscal_state(REPORT_STATE_ROLE.to_owned(), true));
    subjects.sort_by(|left, right| left.role.cmp(&right.role));
    subjects
}

/// Which boundary established the state's current version, as far as the
/// derived systems need to know.
///
/// Transition candidates, aggregates, and the report plan follow from the
/// rest of the state and from the immutable catalog, and both derived systems
/// run in every boundary that writes the state. A state whose current version
/// an earlier boundary established was therefore left consistent by that
/// boundary, and a derived system would propose nothing for it. A state this
/// boundary changed is recomputed, and so is one still at its
/// initial-scenario version, whose authored derived fields no boundary has
/// checked yet. The answer comes from the current-version provenance index,
/// which a seal cannot change.
#[derive(Clone, Copy, Eq, PartialEq)]
enum StateWriter {
    EarlierBoundary,
    ThisBoundary,
    Scenario,
}

fn state_writer(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<StateWriter, CanwuError> {
    Ok(
        match view.current_domain_record_version(&fiscal_state_reference().into_untyped())? {
            Some(DomainRecordVersionRef {
                established_by: DomainRecordVersionSource::BoundaryChange { boundary, .. },
                ..
            }) => {
                if boundary == context.boundary_id {
                    StateWriter::ThisBoundary
                } else {
                    StateWriter::EarlierBoundary
                }
            }
            _ => StateWriter::Scenario,
        },
    )
}

/// Runs one derived system: `change` gets the state, the catalog, and whether
/// this boundary has already written the state, and returns the summary of
/// its change, or `None` to propose nothing.
fn update_derived(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    change: impl FnOnce(
        &mut FiscalState,
        &CompiledFiscalCatalog,
        bool,
    ) -> Result<Option<&'static str>, CanwuError>,
) -> Result<BoundaryProposal, CanwuError> {
    let writer = state_writer(view, context)?;
    if writer == StateWriter::EarlierBoundary {
        return Ok(BoundaryProposal::default());
    }
    let Some(catalog) = committed_fiscal_catalog(view)? else {
        return Ok(BoundaryProposal::default());
    };
    let Some((version, mut state)) = committed_fiscal_state(view)? else {
        return Ok(BoundaryProposal::default());
    };
    if writer == StateWriter::Scenario {
        // A report plan means something only in a state a boundary has
        // written, so one that came with a scenario is dropped.
        state.report_plan = FiscalReportPlan::default();
    }
    let Some(summary) = change(&mut state, &catalog, writer == StateWriter::ThisBoundary)? else {
        return Ok(BoundaryProposal::default());
    };
    update_state(version, &state, &catalog, summary)
}

/// Proposes the next version of the fiscal state. Every write is validated
/// here, which is what lets the handlers decode a committed state without
/// validating it again.
fn update_state(
    expected_version: u64,
    state: &FiscalState,
    catalog: &CompiledFiscalCatalog,
    summary: &str,
) -> Result<BoundaryProposal, CanwuError> {
    state.validate(catalog)?;
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Update {
                record: state.record_draft()?,
                expected_version,
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
    // Settlement needs the current version, so reject an older one here
    // rather than fail the settling boundary.
    for evidence in &packet.external_evidence {
        if canwu
            .current_domain_record_version(&evidence.record)?
            .as_ref()
            != Some(evidence)
        {
            return Err(invalid(
                "fiscal execution receipt requires exact available external record versions that are current",
            ));
        }
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

/// A command names a fiscal object or record version that does not exist.
fn not_found(message: &str) -> CanwuError {
    CanwuError::new(ErrorCode::EntityNotFound, message)
}

fn as_invalid_payload(mut error: CanwuError) -> CanwuError {
    error.code = ErrorCode::InvalidPayload;
    error
}

fn invalid_authority(message: &str) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidAuthority, message)
}

fn invalid_payload(message: &str) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidPayload, message)
}

/// The action's issuer decided against a state that has since changed.
fn stale(message: &str) -> CanwuError {
    CanwuError::new(ErrorCode::DomainRecordVersionConflict, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    thread_local! {
        /// The budget `settlement_budget` returns on this thread, so a test
        /// can reach the state's byte budget with a small state.
        pub(super) static SETTLEMENT_BUDGET: std::cell::Cell<Option<usize>> =
            const { std::cell::Cell::new(None) };
    }

    use crate::content::compile_fiscal_content;
    use crate::derive::recompute_derived;
    use crate::model::{
        FiscalAssessmentBasis, FiscalAuditSeverity, FiscalCommutationPolicy, FiscalContentPack,
        FiscalContentSelection, FiscalCoverageDeclaration, FiscalCoverageSelector,
        FiscalCoverageStatus, FiscalHistoricalMode, FiscalMechanism, FiscalPackManifest,
        FiscalPaymentForm, FiscalPeriodDefinition, FiscalProvenance, FiscalRegionDefinition,
        FiscalRuleDefinition, FiscalScopeBinding, HistoricalConfidence, HistoricalYearWindow,
    };
    use canwu_api::{
        CommandEnvelope, CommandRequest, CommandRequestId, KnowledgeSnapshot, OrganizationId,
        Scenario, SimDuration, SimulationGranularity, WorldSnapshot,
    };

    const ISSUER: PersonId = PersonId::new(1);
    const AUTHORITY: &str = "authority.a";
    const RULE: &str = "rule.land";
    const AUTHORED: &str = "authored.assessment";

    fn institution() -> EntityRef {
        EntityRef::Organization(OrganizationId::new(1))
    }

    fn catalog() -> CompiledFiscalCatalog {
        let window = HistoricalYearWindow {
            start: 100,
            end: 200,
        };
        let provenance = BTreeSet::from(["source.fixture".to_owned()]);
        let pack = FiscalContentPack {
            manifest: FiscalPackManifest {
                pack_id: "fixture.budget".to_owned(),
                pack_version: "1.0.0".to_owned(),
                schema_version: 1,
                license: "Apache-2.0".to_owned(),
                title: "Budget fixture".to_owned(),
                historical_scope: window.clone(),
                period_ids: vec!["period.a".to_owned()],
                region_ids: vec!["region.a".to_owned()],
                mechanisms: vec![FiscalMechanism::LandTax],
            },
            periods: vec![FiscalPeriodDefinition {
                id: "period.a".to_owned(),
                label: "Fixture period".to_owned(),
                window: window.clone(),
                branch: None,
            }],
            regions: vec![FiscalRegionDefinition {
                id: "region.a".to_owned(),
                label: "Fixture region".to_owned(),
            }],
            institutions: Vec::new(),
            rules: vec![FiscalRuleDefinition {
                id: RULE.to_owned(),
                revision: 1,
                label: "Land levy".to_owned(),
                mechanism: FiscalMechanism::LandTax,
                legal_window: window,
                jurisdiction_ids: BTreeSet::from(["region.a".to_owned()]),
                subject_scope: "registered_land".to_owned(),
                assessment_basis: FiscalAssessmentBasis::RegisteredLand,
                payment_forms: BTreeSet::from([FiscalPaymentForm::Grain]),
                commutation: FiscalCommutationPolicy::Disabled,
                earmark_ids: BTreeSet::new(),
                provenance_ids: provenance.clone(),
                confidence: HistoricalConfidence::Medium,
            }],
            transitions: Vec::new(),
            coverage: vec![FiscalCoverageDeclaration {
                id: "coverage.a".to_owned(),
                priority: 1,
                selector: FiscalCoverageSelector {
                    period_ids: BTreeSet::new(),
                    region_ids: BTreeSet::new(),
                    mechanisms: BTreeSet::new(),
                },
                status: FiscalCoverageStatus::Supported,
                definition_ids: BTreeSet::from([RULE.to_owned()]),
                provenance_ids: provenance,
            }],
            provenance: vec![FiscalProvenance {
                id: "source.fixture".to_owned(),
                citation: "Synthetic fixture".to_owned(),
                url: "https://example.invalid/fixture".to_owned(),
                claim_scope: "test content only".to_owned(),
                confidence: HistoricalConfidence::Medium,
                forbidden_inferences: vec!["no historical claim".to_owned()],
            }],
        };
        compile_fiscal_content(
            &pack,
            FiscalContentSelection {
                historical_year: 150,
                ..FiscalContentSelection::default()
            },
        )
        .expect("fixture fiscal catalog")
    }

    fn scope_id(index: usize) -> String {
        format!("scope.{index}")
    }

    /// A run with one institution that owns two scopes and has assessed the
    /// first, after `configure` and with consistent derived fields.
    fn small_run(configure: impl Fn(&mut FiscalState)) -> Canwu {
        let catalog = catalog();
        let mut state = FiscalState::new(150, FiscalHistoricalMode::Counterfactual, SimTime::EPOCH);
        state.authority_bindings.insert(
            AUTHORITY.to_owned(),
            FiscalAuthorityBinding {
                id: AUTHORITY.to_owned(),
                institution: institution(),
                authorized_actor: Some(ISSUER),
                acting_actor: None,
                authority_basis: None,
                rebound_by: None,
            },
        );
        for index in 0..2 {
            let scope = scope_id(index);
            state.scope_bindings.insert(
                scope.clone(),
                FiscalScopeBinding {
                    id: scope.clone(),
                    institution: institution(),
                    jurisdiction_id: "region.a".to_owned(),
                    subject_scope: "registered_land".to_owned(),
                    mechanism: FiscalMechanism::LandTax,
                    authoritative_granularity: SimulationGranularity::Aggregate,
                },
            );
            let adoption = format!("adopt.{index}");
            state.adoptions.insert(
                adoption.clone(),
                FiscalAdoptionState {
                    id: adoption,
                    rule_id: RULE.to_owned(),
                    scope_binding_id: scope,
                    stage: FiscalAdoptionStage::Implemented,
                    generation: 1,
                    changed_at: SimTime::EPOCH,
                    source_action_id: None,
                },
            );
        }
        state.assessments.insert(
            AUTHORED.to_owned(),
            FiscalAssessment {
                id: AUTHORED.to_owned(),
                rule_id: RULE.to_owned(),
                scope_binding_id: scope_id(0),
                accounting_cycle_id: "cycle.authored".to_owned(),
                quantity: 500,
                unit: "grain".to_owned(),
                payment_form: FiscalPaymentForm::Grain,
                commutation_quote: None,
                created_at: SimTime::EPOCH,
            },
        );
        configure(&mut state);
        recompute_derived(&mut state, &catalog, SimTime::EPOCH).expect("derived fiscal fields");
        let scenario = Scenario {
            start_time: SimTime::EPOCH,
            entities: vec![EntityRef::Person(ISSUER), institution()],
            world: WorldSnapshot::default(),
            knowledge: KnowledgeSnapshot::default(),
            domain_records: vec![
                catalog.clone().into_record().expect("catalog record"),
                state.into_record(&catalog).expect("fiscal state record"),
            ],
        };
        Canwu::new_with_plugins(7, scenario, &[&FiscalPlugin::default()]).expect("small run")
    }

    fn committed(canwu: &Canwu) -> (FiscalState, usize) {
        let record = canwu
            .typed_domain_record(&fiscal_state_reference())
            .expect("fiscal state record");
        let bytes = serde_json::to_vec(&record.payload)
            .expect("fiscal state payload")
            .len();
        (
            record
                .decode_payload::<FiscalStateRecord>()
                .expect("fiscal state"),
            bytes,
        )
    }

    fn assess(action_id: &str, scope: usize) -> (String, FiscalAction) {
        (
            action_id.to_owned(),
            FiscalAction::OpenAssessment {
                assessment_id: format!("{action_id}.assessment"),
                rule_id: RULE.to_owned(),
                scope_binding_id: scope_id(scope),
                accounting_cycle_id: "cycle.1".to_owned(),
                quantity: 100,
                unit: "grain".to_owned(),
                payment_form: FiscalPaymentForm::Grain,
                commutation_quote: None,
            },
        )
    }

    /// Admits the actions against the procedure revision the run has plus
    /// `ahead`, and settles them, with settlement held to `budget`.
    fn submit(
        canwu: &mut Canwu,
        budget: Option<usize>,
        ahead: u64,
        actions: impl IntoIterator<Item = (String, FiscalAction)>,
    ) {
        let expected_procedure_revision = committed(canwu).0.procedure_revision + ahead;
        let revision = canwu.revision();
        for (offset, (action_id, action)) in (0_u64..).zip(actions) {
            let request = FiscalActionRequest {
                action_id,
                authority_binding_id: AUTHORITY.to_owned(),
                expected_procedure_revision,
                action,
            };
            canwu
                .enqueue_command(
                    canwu.time(),
                    0,
                    CommandRequest::new(
                        CommandRequestId::new(revision + offset + 1),
                        revision + offset,
                        CommandEnvelope::new(
                            Issuer::Actor(ISSUER),
                            fiscal_action_command(&request).expect("fiscal command"),
                        )
                        .at_time(canwu.time()),
                    ),
                )
                .expect("tracked fiscal command");
        }
        SETTLEMENT_BUDGET.set(budget);
        let settled = canwu.advance_canonical(SimDuration::minutes(1));
        SETTLEMENT_BUDGET.set(None);
        settled.expect("fiscal boundaries");
    }

    fn observer(state: &mut FiscalState) {
        state.observer_bindings.insert(
            "observer.a".to_owned(),
            FiscalObserverBinding {
                id: "observer.a".to_owned(),
                actor: ISSUER,
                knowledge_holder: KnowledgeHolderRef::Person(ISSUER),
                visible_institutions: BTreeSet::from([institution()]),
                confidence_per_mille: 1_000,
            },
        );
    }

    /// An action whose result would not fit the byte budget is rejected as
    /// its own outcome, and the actions before and after it settle. The
    /// budget is reached here on a small state: settlement finds the state
    /// within its margin of the budget, settles the packets a second time,
    /// and measures every action.
    #[test]
    fn an_action_past_the_byte_budget_is_rejected_alone() {
        for configure in [(|_| {}) as fn(&mut FiscalState), observer] {
            let mut canwu = small_run(configure);
            // Room for two assessments, a rejected outcome, and a first
            // report plan; not for an assessment and the largest finding.
            let budget = committed(&canwu).1 + 2_600;
            submit(
                &mut canwu,
                Some(budget),
                0,
                [
                    assess("fits.first", 0),
                    (
                        "does-not-fit".to_owned(),
                        FiscalAction::RecordAudit {
                            audit_id: "does-not-fit.finding".to_owned(),
                            target_id: AUTHORED.to_owned(),
                            severity: FiscalAuditSeverity::Concern,
                            finding: "x".repeat(MAX_FISCAL_ACTION_TEXT_BYTES),
                            evidence: BTreeSet::new(),
                        },
                    ),
                    assess("fits.last", 1),
                ],
            );
            let (state, bytes) = committed(&canwu);
            for action_id in ["fits.first", "fits.last"] {
                assert_eq!(
                    state.action_outcomes[action_id].disposition,
                    FiscalActionDisposition::Applied
                );
            }
            let rejected = &state.action_outcomes["does-not-fit"];
            assert_eq!(rejected.disposition, FiscalActionDisposition::Rejected);
            assert!(rejected.reason.ends_with(FISCAL_STATE_BUDGET_EXHAUSTED));
            assert!(state.audits.is_empty());
            assert_eq!(state.aggregates.len(), 3);
            assert!(bytes <= budget, "{bytes} bytes against {budget}");
        }
    }

    /// A rejected action's outcome is recorded when the state it leaves fits
    /// the byte budget exactly, and dropped when it is one byte over: with
    /// and without a report plan to stamp, as the first outcome or a later
    /// one, and when the procedure revision gains a digit.
    #[test]
    fn a_rejected_outcome_is_recorded_exactly_when_it_fits() {
        let filler = |state: &mut FiscalState| {
            state.action_outcomes.insert(
                "filler".to_owned(),
                FiscalActionOutcome {
                    action_id: "filler".to_owned(),
                    disposition: FiscalActionDisposition::Rejected,
                    reason: "filler".to_owned(),
                    command: canwu_api::CommandId::new(1),
                    settled_at: SimTime::EPOCH,
                },
            );
        };
        for observed in [false, true] {
            for first in [false, true] {
                for revision in [1, 9, 99, 100] {
                    let run = || {
                        small_run(|state| {
                            state.procedure_revision = revision;
                            if observed {
                                observer(state);
                            }
                            if !first {
                                filler(state);
                            }
                        })
                    };
                    let outdated = || [assess("stale", 1)];
                    // What the state grows to when the outcome is recorded.
                    let mut unbounded = run();
                    submit(&mut unbounded, None, 1, outdated());
                    let (state, needed) = committed(&unbounded);
                    assert_eq!(
                        state.action_outcomes["stale"].disposition,
                        FiscalActionDisposition::Rejected
                    );
                    assert_eq!(observed, !state.report_plan.is_unused());

                    for (budget, recorded) in [(needed, true), (needed - 1, false)] {
                        let case = format!(
                            "observed {observed}, first {first}, revision {revision}, \
                             budget {budget} of {needed}"
                        );
                        let mut canwu = run();
                        let before = committed(&canwu);
                        submit(&mut canwu, Some(budget), 1, outdated());
                        let (state, bytes) = committed(&canwu);
                        assert_eq!(
                            state.action_outcomes.contains_key("stale"),
                            recorded,
                            "{case}"
                        );
                        assert!(bytes <= budget, "{case}: {bytes} bytes");
                        let dropped: Vec<_> = canwu
                            .events()
                            .iter()
                            .filter(|event| {
                                event.kind.plugin_identity()
                                    == Some((PLUGIN_NAME, ACTION_DROPPED_EVENT))
                            })
                            .collect();
                        if recorded {
                            assert_eq!(bytes, needed, "{case}");
                            assert!(dropped.is_empty(), "{case}");
                        } else {
                            // Nothing of the action is left but its event,
                            // which names the institution it was issued for.
                            assert_eq!((state, bytes), before, "{case}");
                            assert_eq!(dropped.len(), 1, "{case}");
                            assert_eq!(dropped[0].affected_entities, [institution()], "{case}");
                        }
                    }
                }
            }
        }
    }

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

        let error = ensure_action_outcome_slot(&state)
            .expect_err("a full outcome map must reject at command admission");
        assert_eq!(error.code, ErrorCode::ValueOutOfRange);
    }
}
