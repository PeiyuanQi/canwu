use crate::appointment::{
    admit_proposal_capacity, apply_appointment, check_appointment, installed_basis,
    is_appointment_action, is_proposal,
};
use crate::derive::{
    assemble_fiscal_reports, compute_aggregates, compute_transition_candidates, next_report_plan,
    planned_reports,
};
use crate::model::{
    CompiledFiscalCatalog, FISCAL_ACTING_BASIS_NOT_CURRENT, FISCAL_AUTHORITY_BASIS_NOT_CURRENT,
    FISCAL_BOUNDARY_ACTION_LIMIT, FISCAL_INGRESS_BUDGET_REJECTION,
    FISCAL_INGRESS_INVALID_REJECTION, FISCAL_INGRESS_MALFORMED_REJECTION,
    FISCAL_INGRESS_ORIGIN_REJECTION, FISCAL_ISSUER_NOT_BOUND, FISCAL_STATE_BUDGET_EXHAUSTED,
    FISCAL_STATE_CATALOG_ROLE, FISCAL_STATE_OBSERVER_ROLE, FiscalAction, FiscalActionDisposition,
    FiscalActionIssuer, FiscalActionOutcome, FiscalActionRequest, FiscalAdoptionStage,
    FiscalAdoptionState, FiscalAppointmentDecision, FiscalAssessment, FiscalAuditFinding,
    FiscalAuthorityBinding, FiscalCatalogRecord, FiscalExecutionEvidence, FiscalExecutionReceipt,
    FiscalExecutionReceiptPacket, FiscalExecutionRequest, FiscalExternalOperationRef,
    FiscalHistoricalContext, FiscalHistoricalContextPacket, FiscalIssuerRole,
    FiscalObserverBinding, FiscalRebinding, FiscalReceiptDisposition, FiscalRemission,
    FiscalReport, FiscalReportPlan, FiscalReportRecord, FiscalState, FiscalStateRecord,
    MAX_FISCAL_ACTION_JSON_BYTES, MAX_FISCAL_ACTION_OUTCOMES, MAX_FISCAL_ACTION_TEXT_BYTES,
    MAX_FISCAL_ACTIONS_PER_BOUNDARY, MAX_FISCAL_EVIDENCE_PER_RECORD, MAX_FISCAL_IDENTIFIER_BYTES,
    MAX_FISCAL_OBSERVERS, MAX_FISCAL_REFERENCES_PER_DEFINITION, MAX_FISCAL_REPORT_PARTS,
    MAX_FISCAL_REPORT_PLAN_BYTES, MAX_FISCAL_STATE_JSON_BYTES, fiscal_catalog_reference,
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
/// The internal packet that the admission of an [`APPLY_FISCAL_ACTION_COMMAND`]
/// queues for the next boundary to settle.
///
/// The type is kernel-internal: a host cannot enqueue it
/// (`enqueue_plugin_ingress` refuses it with `InvalidAuthority`), and the
/// command is the only path for an action. A packet is settled only when its
/// ingress record names, as its cause, the command the packet carries, which
/// only that command's admission produces; any other, which only a boundary
/// system of another plugin that declares this target could schedule, is
/// skipped with a [`FISCAL_INGRESS_REJECTED_EVENT`].
pub const FISCAL_ACTION_INGRESS: &str = "fiscal_action_v1";
pub const FISCAL_EXECUTION_RECEIPT_INGRESS: &str = "fiscal_execution_receipt_v1";
pub const FISCAL_HISTORICAL_CONTEXT_INGRESS: &str = "fiscal_historical_context_v1";
/// The internal packet a boundary schedules, due at once, when it leaves
/// report parts owed to an observer who can receive knowledge, so that the
/// next boundary publishes them without waiting for a change of the fiscal
/// state. It carries nothing: the boundary that admits it finds the owed
/// parts in the committed report plan.
pub const FISCAL_REPORT_DRAIN_INGRESS: &str = "fiscal_report_drain_v1";
/// Event that is the only record of a fiscal packet settlement skips without
/// changing anything: an execution receipt or historical-context change that
/// does not decode, is not valid, or does not fit the state's byte budget,
/// or an action packet that is malformed or was not produced by the admission
/// of the command it names. No fiscal packet fails its boundary. The summary
/// names the ingress and the packet and ends with the reason:
/// [`crate::FISCAL_INGRESS_MALFORMED_REJECTION`],
/// [`crate::FISCAL_INGRESS_INVALID_REJECTION`] (after the message that says
/// why), [`crate::FISCAL_INGRESS_BUDGET_REJECTION`], or
/// [`crate::FISCAL_INGRESS_ORIGIN_REJECTION`].
pub const FISCAL_INGRESS_REJECTED_EVENT: &str = "canwu.fiscal.ingress_rejected.v1";

const PLUGIN_VERSION: &str = "0.1.0-experimental";
const SEMANTIC_HASH: &str = "c3632fe8c85dd22d312da6b55ac3f8d8e1d567595676e7efad7c05a7257616cb";
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
const RECEIPT_RECORDED_EVENT: &str = "canwu.fiscal.execution_receipt_recorded.v1";
const CONTEXT_CHANGED_EVENT: &str = "canwu.fiscal.historical_context_changed.v1";
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
    HistoricalContext(IngressId, FiscalHistoricalContextPacket),
    /// A packet that settlement skips: its ingress, its packet type, and the
    /// reason.
    Refused(IngressId, &'static str, &'static str),
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
    /// The plugin that reads `external_evidence_kinds` as execution evidence.
    /// Every kind a fiscal state approves in `execution_evidence_kinds` must
    /// be one of them: activation refuses a state that approves another, which
    /// settlement could not read.
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
            // Settlement reads every evidence version a receipt cites, so a
            // kind the state approves but the plugin was not given would
            // refuse every receipt of it; the setup error is caught here.
            if state
                .execution_evidence_kinds
                .iter()
                .any(|kind| self.external_evidence_kinds.binary_search(kind).is_err())
            {
                return Err(invalid(
                    "fiscal state approves an execution evidence kind not declared by FiscalPlugin::new",
                ));
            }
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
                // No boundary has written an authored state yet, and the
                // first one that runs the fiscal systems writes its derived
                // fields without a settlement to measure them. Measure them
                // here, at the widest time that boundary could have, so that
                // such a state is refused where it enters the run instead of
                // failing that boundary.
                if record.version == 1
                    && measure_writes(&state, &catalog, SimTime::from_minutes(i64::MIN))?
                        > settlement_budget()
                {
                    return Err(invalid(
                        "the authored fiscal state with its derived fields exceeds the fiscal state byte budget",
                    ));
                }
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
        register_fiscal_ingress(registrar)?;

        let mut settle = BoundarySystemContract::new(
            "settle-fiscal-ingress-v1",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        settle.reads = vec![
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
            FISCAL_INGRESS_REJECTED_EVENT.to_owned(),
            RECEIPT_RECORDED_EVENT.to_owned(),
            CONTEXT_CHANGED_EVENT.to_owned(),
        ];
        settle.emits.sort();
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

/// Registers the plugin's ingress types. Only the command's admission queues an
/// action, and only the aggregate system schedules a drain, so both are
/// internal and a host can author neither; receipts and context changes are
/// host ingress. Plugin systems schedule their own internal packets with
/// boundary directives, so the registration permits are not kept.
fn register_fiscal_ingress(registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
    let _action_permit = registrar.register_internal_ingress(PluginIngressDescriptor {
        name: FISCAL_ACTION_INGRESS.to_owned(),
        description: "Settle one admitted fiscal procedure action".to_owned(),
        class: IngressClass::Decision,
        payload_schema: PayloadSchema::Any,
    })?;
    let _drain_permit = registrar.register_internal_ingress(PluginIngressDescriptor {
        name: FISCAL_REPORT_DRAIN_INGRESS.to_owned(),
        description: "Publish the fiscal report parts an earlier boundary left owed".to_owned(),
        class: IngressClass::ScheduledSystem,
        payload_schema: PayloadSchema::Object {
            properties: BTreeMap::new(),
            allow_additional: false,
        },
    })?;
    for (name, description, class) in [
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
    Ok(())
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
    admit_action(view, &state, &request, &issuer, institution)?;
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
    let appointment = is_appointment_action(&request.action);
    if let Some(binding) = state.authority_bindings.get(&request.authority_binding_id) {
        let (role, actor) = authority_role(context, binding, appointment)?;
        let authority_basis = if role == FiscalIssuerRole::Acting {
            if appointment {
                return Err(invalid_authority(
                    "a fiscal acting actor cannot rebind an authority or audit binding, nominate, petition, or decide an appointment",
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
    if let Some(policy) = state
        .audit_appointment_policies
        .get(&request.authority_binding_id)
    {
        // A person the policy lists acts under the policy itself; whether he
        // is listed for this proposal is checked with the rest of it.
        if !is_proposal(&request.action) {
            return Err(invalid_authority(
                "an appointment policy admits only the nominations and removal petitions of the persons it lists",
            ));
        }
        let actor = issuing_actor(context, &policy.institution).ok_or_else(|| {
            invalid_authority(
                "fiscal action requires its bound actor or validated decision controller",
            )
        })?;
        return Ok((
            FiscalActionIssuer {
                binding_id: policy.id.clone(),
                role: FiscalIssuerRole::ListedActor,
                actor: Some(actor),
                authority_basis: None,
            },
            &policy.institution,
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
    if issuing_actor(context, &binding.institution) != Some(binding.auditor) {
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
/// institution must own, the binding and authority basis a rebind installs,
/// or the part an appointment action takes under its policy.
fn admit_action(
    view: &SimulationView<'_>,
    state: &FiscalState,
    request: &FiscalActionRequest,
    issuer: &FiscalActionIssuer,
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
                || state
                    .audit_appointment_policies
                    .contains_key(audit_binding_id)
            {
                return Err(invalid_payload(
                    "a fiscal audit rebind names an auditor together with one exact authority basis, under an ID no authority binding uses and no appointment policy has",
                ));
            }
            match state.audit_bindings.get(audit_binding_id) {
                Some(existing) if existing.institution != *institution => {
                    return Err(invalid_authority(
                        "a fiscal audit binding is rebound only from its audited institution",
                    ));
                }
                None if auditor.is_none() => {
                    return Err(not_found("fiscal audit binding is unavailable"));
                }
                _ => {}
            }
            check_appointment(state, request, issuer)?;
            admit_new_basis(view, authority_basis.as_ref())
        }
        FiscalAction::NominateAuditor { .. }
        | FiscalAction::PetitionAuditorRemoval { .. }
        | FiscalAction::DecideAuditAppointment { .. } => {
            check_appointment(state, request, issuer)?;
            admit_proposal_capacity(state, request, issuer)?;
            admit_new_basis(view, installed_basis(state, &request.action))
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
        FiscalAction::NominateAuditor { .. }
        | FiscalAction::PetitionAuditorRemoval { .. }
        | FiscalAction::DecideAuditAppointment { .. } => validate_appointment_bounds(action),
    }
}

/// Bounds what a nomination, petition, or appointer's decision carries: the
/// IDs it creates or cites, and a petition's grounds, of which it needs one.
fn validate_appointment_bounds(action: &FiscalAction) -> Result<(), CanwuError> {
    let created =
        |value: &str, label: &str| validate_identifier(value, label).map_err(as_invalid_payload);
    let reference = |value: &str| bounded(value, MAX_FISCAL_IDENTIFIER_BYTES, "reference");
    match action {
        FiscalAction::NominateAuditor {
            nomination_id,
            audit_binding_id,
            ..
        } => {
            created(nomination_id, "fiscal nomination")?;
            created(audit_binding_id, "fiscal audit binding")
        }
        FiscalAction::PetitionAuditorRemoval {
            petition_id,
            audit_binding_id,
            grounds,
        } => {
            created(petition_id, "fiscal removal petition")?;
            reference(audit_binding_id)?;
            if grounds.is_empty() || grounds.len() > MAX_FISCAL_EVIDENCE_PER_RECORD {
                return Err(invalid_payload(
                    "a fiscal removal petition cites at least one ground and no more evidence than one record may hold",
                ));
            }
            Ok(())
        }
        FiscalAction::DecideAuditAppointment {
            audit_binding_id,
            decision,
        } => {
            created(audit_binding_id, "fiscal audit binding")?;
            match decision {
                FiscalAppointmentDecision::AcceptNomination { nomination_id: id }
                | FiscalAppointmentDecision::AcceptPetition { petition_id: id }
                | FiscalAppointmentDecision::Dismiss { proposal_id: id } => reference(id),
                FiscalAppointmentDecision::Appoint { .. } | FiscalAppointmentDecision::Remove => {
                    Ok(())
                }
            }
        }
        _ => Ok(()),
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
        // A transition was checked above, and a rebind or appointment action
        // names a binding rather than a scope; admission checks it separately.
        FiscalAction::ApplyTransition { .. }
        | FiscalAction::RebindAuthority { .. }
        | FiscalAction::RebindAudit { .. }
        | FiscalAction::NominateAuditor { .. }
        | FiscalAction::PetitionAuditorRemoval { .. }
        | FiscalAction::DecideAuditAppointment { .. } => return Ok(()),
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
/// the acting role, which never admits a rebind or appointment action. Only
/// for such an action (`appointment`), and only with no responsible actor at
/// all, does the institution deciding as a body hold the institution role, so
/// that it can replace an authorized actor who can no longer act, or nominate,
/// petition, or decide as a body. Any other responsible actor holds no role.
fn authority_role(
    context: &CommandContext,
    binding: &FiscalAuthorityBinding,
    appointment: bool,
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
                } else if appointment && responsible_actor.is_none() {
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

/// The person a command comes from, for an auditor or a person an
/// appointment policy lists: its actor issuer, or the actor or responsible
/// actor of a validated decision whose subject is `subject`, the audited
/// institution.
fn issuing_actor(context: &CommandContext, subject: &EntityRef) -> Option<PersonId> {
    match (&context.issuer, context.decision_controller_id.as_deref()) {
        (Issuer::Actor(actor), None) => Some(*actor),
        (Issuer::Human(issuer) | Issuer::Ai(issuer), Some(controller))
            if issuer == controller
                && context.authority.command_subject.as_ref() == Some(subject) =>
        {
            match &context.authority.decision_origin {
                DecisionOrigin::Actor { actor } => Some(*actor),
                DecisionOrigin::Institution {
                    responsible_actor, ..
                } => *responsible_actor,
                _ => None,
            }
        }
        _ => None,
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
/// or auditor may no longer be current. An appointment action is checked
/// against its policy again, and what it cites must still be pending. The
/// basis a rebind, nomination, or appointment installs must also still be
/// current.
fn settlement_authority(
    view: &SimulationView<'_>,
    state: &FiscalState,
    admitted: &AdmittedFiscalAction,
) -> Result<(), CanwuError> {
    let issuer = &admitted.issuer;
    let not_bound = || invalid_authority(FISCAL_ISSUER_NOT_BOUND);
    let action = &admitted.request.action;
    let appointment = is_appointment_action(action);
    // The roles admit different actions, so an item that pairs a role with an
    // action admission would never have paired is not what admission queued.
    let role_admits_action = match issuer.role {
        FiscalIssuerRole::Principal => true,
        FiscalIssuerRole::Acting => !appointment,
        FiscalIssuerRole::Institution => appointment,
        FiscalIssuerRole::Auditor => matches!(action, FiscalAction::RecordAudit { .. }),
        FiscalIssuerRole::ListedActor => is_proposal(action),
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
    } else if issuer.role == FiscalIssuerRole::ListedActor {
        // Policies do not change, so whether the person is listed is
        // checked below with the rest of the appointment.
        if issuer.actor.is_none()
            || !state
                .audit_appointment_policies
                .contains_key(&issuer.binding_id)
        {
            return Err(not_bound());
        }
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
            | FiscalIssuerRole::Auditor
            | FiscalIssuerRole::ListedActor => {}
        }
    }
    check_appointment(state, &admitted.request, issuer)?;
    match installed_basis(state, action) {
        Some(basis) => require_current_basis(view, basis, FISCAL_AUTHORITY_BASIS_NOT_CURRENT),
        None => Ok(()),
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

/// Decodes the admitted packets, before any of them changes the state.
///
/// An action packet is settled only when its ingress record names, as its
/// cause, the command the packet carries. The action ingress is internal, so
/// a host cannot enqueue a packet, and a command's handler can queue ingress
/// only for its own plugin: the admission of an `apply_fiscal_action_v1`
/// command is the one producer of a fiscal packet whose cause is a command,
/// and it records that command in the packet, with the issuer and role it
/// resolved. Any other action packet, which only a boundary system of another
/// plugin that declares this target could schedule, is refused and changes
/// nothing. The check reads the ingress record of the boundary that admits it
/// and no retained command evidence, so a sealed run decides as its exact
/// replay does.
/// Decodes the boundary's packets. A packet whose payload does not decode,
/// or an action packet that its command did not produce, is kept as a
/// refusal: it changes nothing and does not fail the boundary.
fn decode_fiscal_packets(packets: &[AdmittedPacket<'_>]) -> Vec<FiscalPacket> {
    let malformed = |packet: &AdmittedPacket<'_>, packet_type| {
        FiscalPacket::Refused(
            packet.ingress,
            packet_type,
            FISCAL_INGRESS_MALFORMED_REJECTION,
        )
    };
    packets
        .iter()
        .map(|packet| match packet.packet_type {
            FISCAL_ACTION_INGRESS => {
                match decode::<AdmittedFiscalAction>(packet.payload, "admitted fiscal action") {
                    Err(_) => malformed(packet, FISCAL_ACTION_INGRESS),
                    Ok(admitted) if packet.cause != Some(&CauseRef::Command(admitted.command)) => {
                        FiscalPacket::Refused(
                            packet.ingress,
                            FISCAL_ACTION_INGRESS,
                            FISCAL_INGRESS_ORIGIN_REJECTION,
                        )
                    }
                    Ok(admitted) => FiscalPacket::Action(Box::new(admitted)),
                }
            }
            FISCAL_EXECUTION_RECEIPT_INGRESS => {
                match decode(packet.payload, "fiscal execution receipt") {
                    Ok(receipt) => FiscalPacket::Receipt(packet.ingress, receipt),
                    Err(_) => malformed(packet, FISCAL_EXECUTION_RECEIPT_INGRESS),
                }
            }
            _ => match decode(packet.payload, "fiscal historical context") {
                Ok(context) => FiscalPacket::HistoricalContext(packet.ingress, context),
                Err(_) => malformed(packet, FISCAL_HISTORICAL_CONTEXT_INGRESS),
            },
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
    /// When this pass holds every packet to `budget`: the measure of the
    /// state as it stands (see [`measure_writes`]).
    measured: Option<usize>,
    /// The bytes, without the report plan, that any record the boundary
    /// writes may take.
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
///
/// No packet fails the boundary for the state's byte budget or for what the
/// derived systems of the boundary add. The pass is measured once, as the
/// largest record the boundary will write; when that does not fit, the
/// packets are settled again from the committed state with every one of them
/// measured, and the one that does not fit is refused on its own.
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
    let packets = decode_fiscal_packets(&packets);
    let budget = settlement_budget();
    let mut settlement = settle_packets(view, context, &catalog, state, &packets, None, budget)?;
    let mut draft = None;
    if settlement.changed {
        let (unmeasured_draft, encoded) = settlement.state.encode_record()?;
        if largest_write(&settlement.state, &catalog, context.at, encoded)? <= budget {
            draft = Some(unmeasured_draft);
        } else {
            // Some record of this boundary would not fit. Settle the same
            // packets again from the committed state, this time measuring
            // every one, so that the one that does not fit is refused on its
            // own instead of failing the boundary for all of them.
            let Some((_, state)) = committed_state()? else {
                return Ok(BoundaryProposal::default());
            };
            let measured = measure_writes(&state, &catalog, context.at)?;
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
/// With `measured`, the measure of `state`, the pass also holds every packet
/// to `budget`: an action whose result would not fit is rejected, and one
/// whose outcome would not fit is dropped; a receipt or a historical-context
/// change that would not fit changes nothing and is refused with an event.
/// Without it nothing is measured; the caller measures the result once and
/// repeats the pass with measurement only when it does not fit.
fn settle_packets(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    catalog: &CompiledFiscalCatalog,
    state: FiscalState,
    packets: &[FiscalPacket],
    measured: Option<usize>,
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
                let refused = Refusal {
                    ingress: *ingress_id,
                    packet: format!("{FISCAL_EXECUTION_RECEIPT_INGRESS} {}", packet.receipt_id),
                    affected: receipt_institution(&settlement.state, packet),
                };
                settlement.settle_change(
                    catalog,
                    context.at,
                    |state| {
                        settle_execution_receipt(
                            view,
                            state,
                            packet.clone(),
                            *ingress_id,
                            context.at,
                            catalog,
                        )
                    },
                    (
                        RECEIPT_RECORDED_EVENT,
                        "Recorded external fiscal execution evidence",
                    ),
                    refused,
                )?;
            }
            FiscalPacket::HistoricalContext(ingress_id, packet) => {
                let refused = Refusal {
                    ingress: *ingress_id,
                    packet: FISCAL_HISTORICAL_CONTEXT_INGRESS.to_owned(),
                    affected: Vec::new(),
                };
                settlement.settle_change(
                    catalog,
                    context.at,
                    |state| {
                        if !catalog.historical_scope.contains(packet.year) {
                            return Err(invalid("fiscal historical context lies outside the pack"));
                        }
                        // A change the state does not accept, such as a new
                        // year at the start of the run, leaves it as it was.
                        let previous = (state.historical_context.clone(), state.procedure_revision);
                        increment_procedure_revision(state)?;
                        state.historical_context = FiscalHistoricalContext {
                            year: packet.year,
                            mode: packet.mode,
                            updated_at: context.at,
                        };
                        state.validate(catalog).inspect_err(|_| {
                            (state.historical_context, state.procedure_revision) = previous;
                        })?;
                        Ok(true)
                    },
                    (
                        CONTEXT_CHANGED_EVENT,
                        "Changed the explicit fiscal historical context",
                    ),
                    refused,
                )?;
            }
            FiscalPacket::Refused(ingress_id, packet_type, reason) => {
                settlement.events.push(refused_ingress(
                    *ingress_id,
                    packet_type,
                    reason,
                    Vec::new(),
                ));
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
        // rejected one adds its entry to every record the boundary writes,
        // and the procedure revision it advances may gain a digit; the report
        // plan, which carries the revision too, is outside the measure.
        if verdict.is_err()
            && let Some(measured) = self.measured.as_mut()
        {
            let grown = measured
                .saturating_add(outcome_entry_bytes(
                    &outcome,
                    self.state.action_outcomes.is_empty(),
                )?)
                .saturating_add(revision_growth(self.state.procedure_revision));
            if grown > self.budget {
                self.events.push(dropped_action(
                    &self.state,
                    request,
                    "the fiscal state byte budget cannot hold its outcome",
                ));
                return Ok(());
            }
            *measured = grown;
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

    /// Settles one receipt or historical-context change, which `change`
    /// applies to a state, reporting whether it changed anything. `settled`
    /// is the type and summary of the event that records the change.
    ///
    /// No such packet fails the boundary. One whose evidence or content is
    /// invalid, for which `change` returns an error and leaves the state as
    /// it was, changes nothing and is refused with
    /// [`FISCAL_INGRESS_INVALID_REJECTION`] after the error's message. In a
    /// measured pass a change whose result would not fit the budget is not
    /// made either, and is refused with [`FISCAL_INGRESS_BUDGET_REJECTION`].
    /// The refusal event is the packet's only record.
    fn settle_change(
        &mut self,
        catalog: &CompiledFiscalCatalog,
        at: SimTime,
        change: impl FnOnce(&mut FiscalState) -> Result<bool, CanwuError>,
        settled: (&'static str, &str),
        refused: Refusal,
    ) -> Result<(), CanwuError> {
        let invalid =
            |error: &CanwuError| format!("{}: {FISCAL_INGRESS_INVALID_REJECTION}", error.message);
        if self.measured.is_none() {
            match change(&mut self.state) {
                Ok(true) => {}
                Ok(false) => return Ok(()),
                Err(error) => {
                    self.events.push(refused.event(&invalid(&error)));
                    return Ok(());
                }
            }
        } else {
            let mut next = self.state.clone();
            match change(&mut next) {
                Ok(true) => {}
                Ok(false) => return Ok(()),
                Err(error) => {
                    self.events.push(refused.event(&invalid(&error)));
                    return Ok(());
                }
            }
            let bytes = measure_writes(&next, catalog, at)?;
            if bytes > self.budget {
                self.events
                    .push(refused.event(FISCAL_INGRESS_BUDGET_REJECTION));
                return Ok(());
            }
            self.state = next;
            self.measured = Some(bytes);
        }
        self.changed = true;
        self.events.push(SettledEvent {
            event_type: settled.0,
            summary: settled.1.to_owned(),
            affected: Vec::new(),
        });
        Ok(())
    }
}

/// The measure of a state that settlement leaves: the bytes, without the
/// report plan, of the largest record its boundary writes. See
/// [`largest_write`].
fn measure_writes(
    state: &FiscalState,
    catalog: &CompiledFiscalCatalog,
    at: SimTime,
) -> Result<usize, CanwuError> {
    largest_write(state, catalog, at, state.encode_record()?.1)
}

/// The bytes, without the report plan, of the largest record a boundary
/// writes when its settlement leaves the state as `state`, whose own record
/// takes `encoded` bytes.
///
/// Settlement writes `state`. The candidate system then writes it again with
/// the transition candidates derived from it, and the aggregate system with
/// the aggregates derived from it and a report plan. The three records differ
/// only in those fields, so their sizes follow from `encoded` and the sizes of
/// the fields alone, and the largest of them is exact, however much the
/// derived fields grow or shrink. The report plan is left out because
/// [`MAX_FISCAL_REPORT_PLAN_BYTES`] bounds every plan and the budget this is
/// held to leaves that much free.
fn largest_write(
    state: &FiscalState,
    catalog: &CompiledFiscalCatalog,
    at: SimTime,
    encoded: usize,
) -> Result<usize, CanwuError> {
    let overflow = || invalid("fiscal state size measure overflowed");
    let settlement_write = encoded
        .checked_sub(plan_bytes(&state.report_plan)?)
        .ok_or_else(overflow)?;
    let derived_candidates = json_bytes(&compute_transition_candidates(state, catalog, at))?;
    let derived_aggregates = json_bytes(&compute_aggregates(state, catalog)?)?;
    let candidate_write = settlement_write
        .checked_sub(json_bytes(&state.transition_candidates)?)
        .and_then(|bytes| bytes.checked_add(derived_candidates))
        .ok_or_else(overflow)?;
    let aggregate_write = candidate_write
        .checked_sub(json_bytes(&state.aggregates)?)
        .and_then(|bytes| bytes.checked_add(derived_aggregates))
        .ok_or_else(overflow)?;
    Ok(settlement_write.max(candidate_write).max(aggregate_write))
}

/// The bytes a report plan adds to the serialized state: the comma that
/// separates it from the other fields, its key, a colon, and its value;
/// nothing while it is unused and so omitted.
fn plan_bytes(plan: &FiscalReportPlan) -> Result<usize, CanwuError> {
    if plan.is_unused() {
        return Ok(0);
    }
    Ok(r#","report_plan":"#.len() + json_bytes(plan)?)
}

fn json_bytes(value: &impl Serialize) -> Result<usize, CanwuError> {
    Ok(serde_json::to_vec(value).map_err(encode_error)?.len())
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

/// The bytes, without the report plan, that settlement lets any record of
/// its boundary take: the state's byte budget less the largest report plan.
fn settlement_budget() -> usize {
    #[cfg(test)]
    if let Some(bytes) = tests::SETTLEMENT_BUDGET.with(std::cell::Cell::get) {
        return bytes;
    }
    MAX_FISCAL_STATE_JSON_BYTES - MAX_FISCAL_REPORT_PLAN_BYTES
}

/// What names a receipt or historical-context packet in the event that
/// refuses it.
struct Refusal {
    ingress: IngressId,
    /// The packet type, and for a receipt its receipt ID.
    packet: String,
    affected: Vec<EntityRef>,
}

impl Refusal {
    fn event(self, reason: &str) -> SettledEvent {
        refused_ingress(self.ingress, &self.packet, reason, self.affected)
    }
}

/// Reports a packet that settlement skips without changing anything, by its
/// ingress, a label naming the packet, and the reason the summary ends with.
fn refused_ingress(
    ingress: IngressId,
    packet: &str,
    reason: &str,
    affected: Vec<EntityRef>,
) -> SettledEvent {
    SettledEvent {
        event_type: FISCAL_INGRESS_REJECTED_EVENT,
        summary: format!(
            "Rejected fiscal ingress {} ({packet}): {reason}",
            ingress.get()
        ),
        affected,
    }
}

/// The institution a receipt's execution request was authorized for, when
/// an event may name it.
fn receipt_institution(
    state: &FiscalState,
    packet: &FiscalExecutionReceiptPacket,
) -> Vec<EntityRef> {
    state
        .execution_requests
        .get(&packet.request_id)
        .and_then(|request| registered_entity(&request.institution))
        .into_iter()
        .collect()
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

/// The persons a rebind, nomination, or direct appointment names.
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
        FiscalAction::NominateAuditor { candidate, .. } => vec![*candidate],
        FiscalAction::DecideAuditAppointment {
            decision: FiscalAppointmentDecision::Appoint { auditor, .. },
            ..
        } => vec![*auditor],
        _ => Vec::new(),
    }
}

/// The institution and persons an applied rebind or appointment action
/// concerns, and nothing for any other action. The engine checks the affected
/// entities of an emitted event against its entity registry when the boundary
/// commits, so a rebind or appointment can never commit a binding to a person
/// who does not exist.
fn rebound_entities(state: &FiscalState, request: &FiscalActionRequest) -> Vec<EntityRef> {
    let persons = named_persons(&request.action);
    if !is_appointment_action(&request.action) {
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
    measured: Option<(&mut usize, usize)>,
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
        let bytes = measure_writes(&settled, catalog, at)?;
        if bytes > budget {
            return Err(invalid(FISCAL_STATE_BUDGET_EXHAUSTED));
        }
        *measured = bytes;
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
            // A finding entered under an audit binding keeps how its auditor
            // was appointed, as the binding holds it now.
            let (appointed_by, nomination) = match issuer.role {
                FiscalIssuerRole::Auditor => state
                    .audit_bindings
                    .get(&issuer.binding_id)
                    .map(|binding| (binding.rebound_by.clone(), binding.nomination.clone()))
                    .unwrap_or_default(),
                _ => (None, None),
            };
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
                    appointed_by,
                    nomination,
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
        FiscalAction::RebindAudit { .. }
        | FiscalAction::NominateAuditor { .. }
        | FiscalAction::PetitionAuditorRemoval { .. }
        | FiscalAction::DecideAuditAppointment { .. } => {
            apply_appointment(state, request, issuer, at)?;
        }
    }
    Ok(())
}

pub(crate) fn rebinding(
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
    let receipt_id = packet.receipt_id.clone();
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
    // A receipt the state does not accept, for example one that takes its
    // request past the authorized quantity, leaves the state as it was, so
    // that settlement can refuse it without failing the boundary.
    state.validate(catalog).inspect_err(|_| {
        state.execution_receipts.remove(&receipt_id);
    })
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

/// Derives the aggregates and plans the reports of a state this boundary
/// changed, or of a state still at its scenario version; in any other
/// boundary, publishes the report parts an earlier boundary left owed (see
/// [`publish_owed_reports`]).
///
/// A plan that leaves parts owed to an observer who can receive knowledge,
/// because they did not fit the boundary's record limit, comes with an
/// internal [`FISCAL_REPORT_DRAIN_INGRESS`] packet due at once, so that the
/// next boundary publishes them.
fn aggregate_state(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    if state_writer(view, context)? == StateWriter::EarlierBoundary {
        return publish_owed_reports(view);
    }
    let mut drain = false;
    let mut proposal = update_derived(view, context, |state, catalog, written_now| {
        let aggregates = compute_aggregates(state, catalog)?;
        let aggregates_changed = state.aggregates != aggregates;
        // The report plan follows a change of the state. A state that no
        // boundary has written, with aggregates that are already right, is
        // left as it is, and its first report comes with its first change.
        let plan = if written_now || aggregates_changed {
            let unreachable = unreachable_observers(view, state)?;
            next_report_plan(state, &aggregates, &unreachable)
                .inspect(|plan| drain = owes_reachable_parts(plan, &unreachable))
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
    })?;
    if drain && view.can_schedule_ingress() {
        proposal.directives.push(drain_directive());
    }
    Ok(proposal)
}

/// Publishes the report parts that the committed plan owes an observer who
/// can receive knowledge now, in a boundary that has not written the fiscal
/// state.
///
/// Parts are owed when a boundary had more report records to publish than
/// [`crate::MAX_FISCAL_REPORT_RECORDS_PER_BOUNDARY`], and that boundary scheduled a
/// [`FISCAL_REPORT_DRAIN_INGRESS`] packet so that the next boundary comes at
/// once; or when an observer's actor was dead, until a boundary finds the
/// actor alive again. Either way they are found here, in any boundary that
/// runs the fiscal systems, without waiting for a change of the fiscal state.
///
/// The test reads the plan's owed observers and their actors from the
/// committed record's payload, without decoding the state, so a boundary in
/// which nothing is owed to an observer who can receive it costs what it did
/// before. Otherwise the state is decoded and a plan of the owed parts is
/// written, at most [`crate::MAX_FISCAL_REPORT_RECORDS_PER_BOUNDARY`] records of
/// them, with a drain packet when some are still owed; the report system then
/// publishes it. Nothing else in the state changes, so the procedure revision
/// does not advance and no action becomes stale.
fn publish_owed_reports(view: &SimulationView<'_>) -> Result<BoundaryProposal, CanwuError> {
    let Some(record) = view.typed_domain_record(&fiscal_state_reference())? else {
        return Ok(BoundaryProposal::default());
    };
    if !owes_reachable_observer(view, &record.payload)? {
        return Ok(BoundaryProposal::default());
    }
    let Some(catalog) = committed_fiscal_catalog(view)? else {
        return Ok(BoundaryProposal::default());
    };
    let mut state = record.decode_payload::<FiscalStateRecord>()?;
    let unreachable = unreachable_observers(view, &state)?;
    let Some(plan) = next_report_plan(&state, &state.aggregates, &unreachable) else {
        return Ok(BoundaryProposal::default());
    };
    if plan == state.report_plan {
        return Ok(BoundaryProposal::default());
    }
    let drain = owes_reachable_parts(&plan, &unreachable);
    state.report_plan = plan;
    state.validate(&catalog)?;
    let (draft, bytes) = state.encode_record()?;
    // Every state a settlement wrote leaves room for any plan, and so does an
    // authored one, whose first derivation activation measured. This guard
    // only keeps an oversized plan from failing the boundary; its owed parts
    // would then wait for the next change of the fiscal state.
    if bytes > MAX_FISCAL_STATE_JSON_BYTES {
        return Ok(BoundaryProposal::default());
    }
    let mut directives = vec![BoundaryDirective::MutateRecord {
        mutation: DomainRecordMutation::Update {
            record: draft,
            expected_version: record.version,
        },
        summary: "Planned owed fiscal report parts".to_owned(),
    }];
    if drain && view.can_schedule_ingress() {
        directives.push(drain_directive());
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

/// Whether the report plan in a committed state payload owes parts, or a
/// summary, to an observer whose actor can receive knowledge now. It reads
/// the plan's owed observer IDs and those observers' actors from the payload
/// and decodes nothing else; there are at most [`MAX_FISCAL_OBSERVERS`].
fn owes_reachable_observer(view: &SimulationView<'_>, payload: &Value) -> Result<bool, CanwuError> {
    let Some(owed) = payload
        .get("report_plan")
        .and_then(|plan| plan.get("deferred"))
        .and_then(Value::as_object)
    else {
        return Ok(false);
    };
    for observer_id in owed.keys() {
        let actor = payload
            .get("observer_bindings")
            .and_then(|bindings| bindings.get(observer_id))
            .and_then(|binding| binding.get("actor"))
            .and_then(|actor| serde_json::from_value::<PersonId>(actor.clone()).ok())
            .ok_or_else(|| invalid("the fiscal report plan owes parts to an unknown observer"))?;
        if !actor_is_unreachable(view, actor)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether a plan leaves parts, or a summary, owed to an observer who can
/// receive knowledge now.
fn owes_reachable_parts(plan: &FiscalReportPlan, unreachable: &BTreeSet<String>) -> bool {
    plan.deferred
        .keys()
        .any(|observer_id| !unreachable.contains(observer_id))
}

/// The internal packet, due at once, that makes the next boundary come and
/// publish the report parts this one left owed.
fn drain_directive() -> BoundaryDirective {
    BoundaryDirective::ScheduleIngress {
        after: canwu_api::SimDuration::ZERO,
        packet_type: FISCAL_REPORT_DRAIN_INGRESS.to_owned(),
        priority: 0,
        payload: Value::Object(serde_json::Map::new()),
        affected: Vec::new(),
    }
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
    actor_is_unreachable(view, observer.actor)
}

fn actor_is_unreachable(view: &SimulationView<'_>, actor: PersonId) -> Result<bool, CanwuError> {
    Ok(view
        .person_availability(actor)?
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
pub(crate) fn not_found(message: &str) -> CanwuError {
    CanwuError::new(ErrorCode::EntityNotFound, message)
}

fn as_invalid_payload(mut error: CanwuError) -> CanwuError {
    error.code = ErrorCode::InvalidPayload;
    error
}

pub(crate) fn invalid_authority(message: &str) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidAuthority, message)
}

pub(crate) fn invalid_payload(message: &str) -> CanwuError {
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
        FiscalCoverageStatus, FiscalExecutionKind, FiscalHistoricalMode, FiscalMechanism,
        FiscalPackManifest, FiscalPaymentForm, FiscalPeriodDefinition, FiscalProvenance,
        FiscalRegionDefinition, FiscalRuleDefinition, FiscalScopeBinding,
        FiscalTransitionDefinition, HistoricalConfidence, HistoricalYearWindow,
    };
    use canwu_api::{
        CommandEnvelope, CommandRequest, CommandRequestId, DomainRecordClass, DomainRecordDraft,
        DomainRecordLifecycle, DomainValueKindClass, KnowledgeSnapshot, OrganizationId, ResourceId,
        Scenario, SimDuration, SimulationGranularity, TypedDomainRecordRef, WorldSnapshot,
    };

    const ISSUER: PersonId = PersonId::new(1);
    const AUTHORITY: &str = "authority.a";
    const RULE: &str = "rule.land";
    const AUTHORED: &str = "authored.assessment";
    /// A land rule that transitions of `catalog_with_transitions` target.
    const REFORMED_RULE: &str = "rule.land.reformed";
    const EVIDENCE_PLUGIN: &str = "fixture-evidence";
    /// An execution request on the authored assessment.
    const REQUEST: &str = "request.collect";

    fn institution() -> EntityRef {
        EntityRef::Organization(OrganizationId::new(1))
    }

    fn catalog() -> CompiledFiscalCatalog {
        catalog_with_transitions(0)
    }

    /// The fixture catalog with `count` transitions to a second land rule,
    /// each eligible in the region from year 160, which no state adopts.
    fn catalog_with_transitions(count: usize) -> CompiledFiscalCatalog {
        let window = HistoricalYearWindow {
            start: 100,
            end: 200,
        };
        let provenance = BTreeSet::from(["source.fixture".to_owned()]);
        let rule = |id: &str| FiscalRuleDefinition {
            id: id.to_owned(),
            revision: 1,
            label: "Land levy".to_owned(),
            mechanism: FiscalMechanism::LandTax,
            legal_window: window.clone(),
            jurisdiction_ids: BTreeSet::from(["region.a".to_owned()]),
            subject_scope: "registered_land".to_owned(),
            assessment_basis: FiscalAssessmentBasis::RegisteredLand,
            payment_forms: BTreeSet::from([FiscalPaymentForm::Grain]),
            commutation: FiscalCommutationPolicy::Disabled,
            earmark_ids: BTreeSet::new(),
            provenance_ids: provenance.clone(),
            confidence: HistoricalConfidence::Medium,
        };
        let mut rules = vec![rule(RULE)];
        let mut transitions = Vec::new();
        if count > 0 {
            rules.push(rule(REFORMED_RULE));
            for index in 0..count {
                transitions.push(FiscalTransitionDefinition {
                    id: format!("transition.reform.{index:02}"),
                    label: "Reform".to_owned(),
                    from_rule_ids: BTreeSet::new(),
                    to_rule_ids: BTreeSet::from([REFORMED_RULE.to_owned()]),
                    observed_window: HistoricalYearWindow {
                        start: 160,
                        end: 200,
                    },
                    eligibility_window: HistoricalYearWindow {
                        start: 160,
                        end: 200,
                    },
                    jurisdiction_ids: BTreeSet::from(["region.a".to_owned()]),
                    supersedes_or_suspends: BTreeSet::new(),
                    prerequisite_ids: BTreeSet::new(),
                    provenance_ids: provenance.clone(),
                    confidence: HistoricalConfidence::Medium,
                });
            }
        }
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
            rules,
            transitions,
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
        run_with(
            &catalog(),
            &[&FiscalPlugin::default()],
            Vec::new(),
            configure,
        )
    }

    /// The run of [`small_run`] on `catalog`, with `plugins` and the scenario's
    /// other `records`.
    fn run_with(
        catalog: &CompiledFiscalCatalog,
        plugins: &[&dyn SimulationPlugin],
        records: Vec<DomainRecord>,
        configure: impl Fn(&mut FiscalState),
    ) -> Canwu {
        Canwu::new_with_plugins(
            7,
            small_scenario(catalog, records, configure, true),
            plugins,
        )
        .expect("small run")
    }

    /// The scenario of the small run, with the derived fields of its state
    /// computed when `derived`, and left empty otherwise.
    fn small_scenario(
        catalog: &CompiledFiscalCatalog,
        records: Vec<DomainRecord>,
        configure: impl Fn(&mut FiscalState),
        derived: bool,
    ) -> Scenario {
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
        if derived {
            recompute_derived(&mut state, catalog, SimTime::EPOCH).expect("derived fiscal fields");
        }
        let mut domain_records = vec![
            catalog.clone().into_record().expect("catalog record"),
            state.into_record(catalog).expect("fiscal state record"),
        ];
        domain_records.extend(records);
        Scenario {
            start_time: SimTime::EPOCH,
            entities: vec![EntityRef::Person(ISSUER), institution()],
            world: WorldSnapshot::default(),
            knowledge: KnowledgeSnapshot::default(),
            domain_records,
        }
    }

    /// An authored state whose derived fields, which the first boundary that
    /// runs the fiscal systems writes and no settlement measures, would take
    /// it past the budget is refused when the run is created. The same state
    /// with room for them is accepted.
    #[test]
    fn an_authored_state_whose_derived_fields_do_not_fit_is_refused_at_creation() {
        let catalog = catalog();
        let scenario = || small_scenario(&catalog, Vec::new(), |_| {}, false);
        let authored = scenario()
            .domain_records
            .into_iter()
            .find(|record| record.reference == fiscal_state_reference().into_untyped())
            .expect("the authored state");
        let authored_bytes = serde_json::to_vec(&authored.payload)
            .expect("authored payload")
            .len();
        let state = authored
            .decode_payload::<FiscalStateRecord>()
            .expect("authored state");
        let needed = measure_writes(&state, &catalog, SimTime::from_minutes(i64::MIN))
            .expect("the measure of its first write");
        // The aggregates the first boundary derives are what does not fit.
        assert!(authored_bytes < needed - 1);
        let plugins: [&dyn SimulationPlugin; 1] = [&FiscalPlugin::default()];
        SETTLEMENT_BUDGET.set(Some(needed - 1));
        let refused = Canwu::new_with_plugins(7, scenario(), &plugins);
        SETTLEMENT_BUDGET.set(Some(needed));
        let accepted = Canwu::new_with_plugins(7, scenario(), &plugins);
        SETTLEMENT_BUDGET.set(None);
        let error = refused
            .err()
            .expect("the oversized authored state is refused");
        assert_eq!(error.code, ErrorCode::InvalidDomainRecord);
        assert!(
            error
                .message
                .contains("exceeds the fiscal state byte budget")
        );
        assert!(accepted.is_ok());
    }

    /// The committed state and the bytes of its record without the report
    /// plan, which is what settlement holds to the budget: every committed
    /// state's derived fields are current, so it is also its measure.
    fn committed(canwu: &Canwu) -> (FiscalState, usize) {
        let record = canwu
            .typed_domain_record(&fiscal_state_reference())
            .expect("fiscal state record");
        let state = record
            .decode_payload::<FiscalStateRecord>()
            .expect("fiscal state");
        let bytes = serde_json::to_vec(&record.payload)
            .expect("fiscal state payload")
            .len()
            - plan_bytes(&state.report_plan).expect("plan bytes");
        (state, bytes)
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
    /// budget is reached here on a small state: settlement finds that a
    /// record of the boundary would not fit, settles the packets a second
    /// time, and measures every action.
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
    /// and without a report plan, which the budget leaves out, as the first
    /// outcome or a later one, and when the procedure revision gains a digit.
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

    /// An application record holding one external execution result.
    struct ExecutionResult;

    impl DomainRecordType for ExecutionResult {
        type Payload = FiscalExecutionEvidence;
        type Class = DomainValueKindClass;

        const NAMESPACE: &'static str = "fixture.execution";
        const NAME: &'static str = "result";
    }

    /// Owns the execution results.
    struct EvidencePlugin;

    impl SimulationPlugin for EvidencePlugin {
        fn name(&self) -> &'static str {
            EVIDENCE_PLUGIN
        }

        fn version(&self) -> &'static str {
            "1"
        }

        fn semantic_hash(&self) -> &'static str {
            "00000000000000000000000000000000000000000000000000000000000000e1"
        }

        fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
            registrar.register_record_schema(DomainRecordSchema::for_record::<ExecutionResult>())
        }
    }

    /// One result of `quantity` collected for [`REQUEST`], as the record the
    /// scenario installs and the exact version a receipt cites.
    fn execution_result(id: &str, quantity: u64) -> (DomainRecord, DomainRecordVersionRef) {
        let draft = DomainRecordDraft::from_typed(
            TypedDomainRecordRef::<ExecutionResult>::new(id),
            &FiscalExecutionEvidence {
                id: id.to_owned(),
                external_operation_id: format!("operation.{id}"),
                request_id: REQUEST.to_owned(),
                quantity,
                unit: "grain".to_owned(),
                payment_form: FiscalPaymentForm::Grain,
                execution_kind: FiscalExecutionKind::Collect,
                resource: ResourceId::new(1),
                source: EntityRef::Person(ISSUER),
                target: institution(),
                disposition: FiscalReceiptDisposition::Fulfilled,
            },
        )
        .expect("execution result draft");
        let version = DomainRecordVersionRef {
            record: draft.reference.clone(),
            version: 1,
            established_by: DomainRecordVersionSource::InitialScenario,
        };
        let record = DomainRecord {
            reference: draft.reference,
            owner: EVIDENCE_PLUGIN.to_owned(),
            class: DomainRecordClass::Record,
            version: 1,
            lifecycle: DomainRecordLifecycle::Active,
            payload: draft.payload,
            references: draft.references,
        };
        (record, version)
    }

    /// The small run, after `configure`, with a collection request on the
    /// authored assessment and every result `results` names installed as
    /// approved evidence for it.
    fn receipt_run(
        configure: fn(&mut FiscalState),
        results: &[(DomainRecord, DomainRecordVersionRef)],
    ) -> Canwu {
        let kind = DomainRecordKind::for_type::<ExecutionResult>();
        let fiscal = FiscalPlugin::new([kind.clone()]);
        run_with(
            &catalog(),
            &[&fiscal, &EvidencePlugin],
            results.iter().map(|(record, _)| record.clone()).collect(),
            |state| {
                configure(state);
                state.execution_evidence_kinds.insert(kind.clone());
                state.execution_requests.insert(
                    REQUEST.to_owned(),
                    FiscalExecutionRequest {
                        id: REQUEST.to_owned(),
                        assessment_id: AUTHORED.to_owned(),
                        institution: institution(),
                        kind: FiscalExecutionKind::Collect,
                        quantity: 400,
                        unit: "grain".to_owned(),
                        payment_form: FiscalPaymentForm::Grain,
                        resource: ResourceId::new(1),
                        source: EntityRef::Person(ISSUER),
                        target: institution(),
                        purpose: "fixture collection".to_owned(),
                        requested_at: SimTime::EPOCH,
                    },
                );
            },
        )
    }

    fn receipt(
        id: &str,
        results: &[(DomainRecord, DomainRecordVersionRef)],
    ) -> FiscalExecutionReceiptPacket {
        FiscalExecutionReceiptPacket {
            receipt_id: id.to_owned(),
            request_id: REQUEST.to_owned(),
            external_evidence: results.iter().map(|(_, version)| version.clone()).collect(),
        }
    }

    /// Settles what is due, with settlement held to `budget`.
    fn settle_within(canwu: &mut Canwu, budget: Option<usize>) {
        SETTLEMENT_BUDGET.set(budget);
        let settled = canwu.advance_canonical(SimDuration::minutes(1));
        SETTLEMENT_BUDGET.set(None);
        settled.expect("fiscal boundaries");
    }

    fn refusals(canwu: &Canwu) -> Vec<String> {
        canwu
            .events()
            .iter()
            .filter(|event| {
                event.kind.plugin_identity() == Some((PLUGIN_NAME, FISCAL_INGRESS_REJECTED_EVENT))
            })
            .map(|event| event.summary.clone())
            .collect()
    }

    /// An execution receipt whose result would not fit the byte budget
    /// changes nothing: the state keeps neither the receipt nor its evidence,
    /// and one event names the receipt and the reason. A smaller receipt of
    /// the same boundary settles, and the refused one settles once there is
    /// room, because none of its evidence versions or operations was consumed.
    #[test]
    fn a_receipt_past_the_byte_budget_changes_nothing_and_is_reported() {
        let large: Vec<_> = (0..16)
            .map(|index| execution_result(&format!("result.large.{index:02}"), 10))
            .collect();
        let small = vec![execution_result("result.small", 10)];
        let results: Vec<_> = large.iter().chain(&small).cloned().collect();
        for configure in [(|_| {}) as fn(&mut FiscalState), observer] {
            // What the state grows to with the small receipt alone.
            let mut unbounded = receipt_run(configure, &results);
            let now = unbounded.time();
            enqueue_execution_receipt(&mut unbounded, now, &receipt("receipt.small", &small))
                .expect("small receipt");
            settle_within(&mut unbounded, None);
            let needed = committed(&unbounded).1;

            let mut canwu = receipt_run(configure, &results);
            let now = canwu.time();
            for packet in [
                receipt("receipt.large", &large),
                receipt("receipt.small", &small),
            ] {
                enqueue_execution_receipt(&mut canwu, now, &packet).expect("receipt");
            }
            settle_within(&mut canwu, Some(needed));
            let (state, bytes) = committed(&canwu);
            assert!(state.execution_receipts.contains_key("receipt.small"));
            assert!(!state.execution_receipts.contains_key("receipt.large"));
            assert_eq!(bytes, needed);
            let refusals = refusals(&canwu);
            assert_eq!(refusals.len(), 1);
            assert!(refusals[0].contains("receipt.large"), "{}", refusals[0]);
            assert!(refusals[0].ends_with(FISCAL_INGRESS_BUDGET_REJECTION));

            let now = canwu.time();
            enqueue_execution_receipt(&mut canwu, now, &receipt("receipt.large", &large))
                .expect("the refused receipt again");
            settle_within(&mut canwu, None);
            assert_eq!(
                committed(&canwu).0.execution_receipts["receipt.large"].quantity,
                160
            );
        }
    }

    /// A historical-context change whose own bytes fit, but whose derived
    /// transition candidates would take the record the candidate system
    /// writes after settlement past the budget, changes nothing and is
    /// reported. With room the same change applies, and the measure that
    /// settlement held to the budget is exactly the largest record its
    /// boundary wrote.
    #[test]
    fn a_context_change_whose_derived_fields_do_not_fit_changes_nothing() {
        let catalog = catalog_with_transitions(16);
        let mut canwu = run_with(&catalog, &[&FiscalPlugin::default()], Vec::new(), |_| {});
        // A context change at the start of the run is not valid; this one comes a
        // day later.
        canwu
            .advance_canonical(SimDuration::days(1))
            .expect("an idle day");
        let (before, bytes) = committed(&canwu);
        assert!(before.transition_candidates.is_empty());
        let context = FiscalHistoricalContextPacket {
            year: 170,
            mode: FiscalHistoricalMode::Counterfactual,
        };
        canwu
            .enqueue_plugin_ingress(
                fiscal_historical_context_ingress(canwu.time(), &context).expect("context"),
            )
            .expect("context ingress");
        settle_within(&mut canwu, Some(bytes + 64));
        assert_eq!(committed(&canwu), (before.clone(), bytes));
        let refusals = refusals(&canwu);
        assert_eq!(refusals.len(), 1);
        assert!(refusals[0].contains(FISCAL_HISTORICAL_CONTEXT_INGRESS));
        assert!(refusals[0].ends_with(FISCAL_INGRESS_BUDGET_REJECTION));

        let at = canwu.time();
        let mut next = before;
        next.historical_context.year = context.year;
        next.historical_context.updated_at = at;
        increment_procedure_revision(&mut next).expect("revision");
        let measured = measure_writes(&next, &catalog, at).expect("measure");
        canwu
            .enqueue_plugin_ingress(
                fiscal_historical_context_ingress(at, &context).expect("context"),
            )
            .expect("context ingress");
        settle_within(&mut canwu, None);
        let (state, bytes) = committed(&canwu);
        assert_eq!(state.historical_context.year, 170);
        assert_eq!(state.transition_candidates.len(), 16);
        assert_eq!(bytes, measured);
    }

    /// Receipts and historical-context changes are public ingress, so a host
    /// can send one that settlement cannot accept: a payload that does not
    /// decode, a year outside the pack, or a second receipt citing evidence
    /// the first receipt of the boundary already settled. Each changes
    /// nothing and is refused with an event that names it and ends with the
    /// reason; the valid packets of the same boundary settle, the boundary
    /// commits, later boundaries run, and restore and exact replay agree.
    #[test]
    fn an_invalid_receipt_or_context_is_refused_without_failing_the_boundary() {
        let results = vec![execution_result("result.shared", 10)];
        let fiscal = FiscalPlugin::new([DomainRecordKind::for_type::<ExecutionResult>()]);
        let mut canwu = receipt_run(|_| {}, &results);
        // A context change at the start of the run is not valid either; this
        // one comes a day later.
        canwu
            .advance_canonical(SimDuration::days(1))
            .expect("an idle day");
        let now = canwu.time();
        let context = |year| {
            fiscal_historical_context_ingress(
                now,
                &FiscalHistoricalContextPacket {
                    year,
                    mode: FiscalHistoricalMode::Counterfactual,
                },
            )
            .expect("context ingress request")
        };
        canwu
            .enqueue_plugin_ingress(PluginIngressRequest::new(
                PLUGIN_NAME,
                FISCAL_EXECUTION_RECEIPT_INGRESS,
                now,
                serde_json::json!({ "receipt_id": "typo" }),
            ))
            .expect("a malformed receipt is accepted as public ingress");
        canwu
            .enqueue_plugin_ingress(context(999))
            .expect("a year outside the pack is accepted as public ingress");
        for id in ["receipt.first", "receipt.second"] {
            enqueue_execution_receipt(&mut canwu, now, &receipt(id, &results))
                .expect("both receipts are current when they are sent");
        }
        canwu
            .enqueue_plugin_ingress(context(170))
            .expect("a valid context change");
        let revision = committed(&canwu).0.procedure_revision;
        settle_within(&mut canwu, None);

        let (state, _) = committed(&canwu);
        assert!(state.execution_receipts.contains_key("receipt.first"));
        assert!(!state.execution_receipts.contains_key("receipt.second"));
        assert_eq!(state.historical_context.year, 170);
        // The receipt and the context change advanced the revision; the
        // refused packets did not.
        assert_eq!(state.procedure_revision, revision + 2);
        let summaries = refusals(&canwu);
        assert_eq!(summaries.len(), 3, "{summaries:?}");
        let refused = |packet: &str, reason: &str| {
            summaries
                .iter()
                .any(|summary| summary.contains(packet) && summary.ends_with(reason))
        };
        assert!(refused(
            &format!("({FISCAL_EXECUTION_RECEIPT_INGRESS})"),
            FISCAL_INGRESS_MALFORMED_REJECTION
        ));
        assert!(refused(
            "lies outside the pack",
            FISCAL_INGRESS_INVALID_REJECTION
        ));
        assert!(refused(
            "receipt.second): external fiscal operation was already settled",
            FISCAL_INGRESS_INVALID_REJECTION
        ));

        // A later boundary refuses the same receipt again rather than failing.
        let now = canwu.time();
        enqueue_execution_receipt(&mut canwu, now, &receipt("receipt.second", &results))
            .expect("the duplicate again");
        settle_within(&mut canwu, None);
        assert_eq!(refusals(&canwu).len(), 4);

        let plugins: [&dyn SimulationPlugin; 2] = [&fiscal, &EvidencePlugin];
        let snapshot = canwu.snapshot_json().expect("snapshot");
        let restored =
            Canwu::from_snapshot_json_with_plugins(&snapshot, &plugins).expect("restored run");
        assert_eq!(
            restored.snapshot_json().expect("restored snapshot"),
            snapshot
        );
        let replayed =
            Canwu::replay_from_journal(&plugins, &canwu.replay_journal()).expect("replayed run");
        assert_eq!(
            replayed.snapshot_json().expect("replayed snapshot"),
            snapshot
        );
    }

    /// No report plan a valid state can hold takes more than the room
    /// settlement leaves for it: every observer binding, under the longest
    /// binding ID, listing every part to publish and to defer, at the largest
    /// revision.
    #[test]
    fn the_largest_report_plan_fits_the_room_left_for_it() {
        let parts: BTreeSet<u32> =
            (0..u32::try_from(MAX_FISCAL_REPORT_PARTS).expect("parts")).collect();
        let observers: BTreeMap<String, BTreeSet<u32>> = (0..MAX_FISCAL_OBSERVERS)
            .map(|index| {
                let id = format!("{index:02}{}", "o".repeat(MAX_FISCAL_IDENTIFIER_BYTES - 2));
                (id, parts.clone())
            })
            .collect();
        let plan = FiscalReportPlan {
            revision: u64::MAX,
            publish: observers.clone(),
            deferred: observers,
        };
        let bytes = plan_bytes(&plan).expect("plan bytes");
        assert!(
            bytes <= MAX_FISCAL_REPORT_PLAN_BYTES,
            "{bytes} bytes against {MAX_FISCAL_REPORT_PLAN_BYTES}"
        );
    }
}
