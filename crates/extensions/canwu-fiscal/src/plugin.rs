use crate::appointment::{
    admit_proposal_capacity, apply_appointment, audit_binding, check_appointment,
    decided_institution, installed_basis, is_appointment_action, is_proposal,
};
use crate::archive::{
    FISCAL_ARCHIVE_INDEX_DIRECTORY_NAMESPACE, FiscalArchiveError, FiscalArchiveRetentionHandleV1,
    FiscalArchiveRetentionPhaseV1, FiscalArchiveSettlement, FiscalArchiveStore,
    PreparedFiscalArchiveBatchV1, VerifiedFiscalArchiveCommitV1, reachable_fiscal_archive_objects,
};
use crate::derive::{
    assemble_fiscal_reports, compute_aggregates, compute_transition_candidates, next_report_plan,
    observer_part_count, planned_reports, planned_withdrawals,
};
use crate::model::{
    CompiledFiscalCatalog, FISCAL_ACTING_BASIS_NOT_CURRENT, FISCAL_ARCHIVE_HEAD_RESERVE_BYTES,
    FISCAL_AUTHORITY_BASIS_NOT_CURRENT, FISCAL_BOUNDARY_ACTION_LIMIT, FISCAL_CONTEXT_OUTSIDE_PACK,
    FISCAL_CYCLE_ARCHIVED, FISCAL_CYCLE_CLOSED, FISCAL_HOT_BOOKS_EXHAUSTED,
    FISCAL_INGRESS_BUDGET_REJECTION, FISCAL_INGRESS_INVALID_REJECTION,
    FISCAL_INGRESS_MALFORMED_REJECTION, FISCAL_INGRESS_ORIGIN_REJECTION, FISCAL_ISSUER_NOT_BOUND,
    FISCAL_OBSERVER_REMOVED, FISCAL_RECEIPT_CLAIM_MISMATCH, FISCAL_RECEIPT_CYCLE_CLOSED,
    FISCAL_RECEIPT_EVIDENCE_COUNT, FISCAL_RECEIPT_EVIDENCE_KIND_UNAPPROVED,
    FISCAL_RECEIPT_EVIDENCE_NOT_CURRENT, FISCAL_RECEIPT_EVIDENCE_REUSED,
    FISCAL_RECEIPT_ID_CONFLICT, FISCAL_RECEIPT_OPERATION_REUSED, FISCAL_RECEIPT_QUANTITY_EXCEEDED,
    FISCAL_RECEIPT_REQUEST_UNKNOWN, FISCAL_REVISION_EXPIRED, FISCAL_REVISION_UNKNOWN,
    FISCAL_STATE_BUDGET_EXHAUSTED, FISCAL_STATE_CATALOG_ROLE, FISCAL_STATE_OBSERVER_ROLE,
    FiscalAction, FiscalActionDisposition, FiscalActionIssuer, FiscalActionRequest,
    FiscalActionTarget, FiscalAdoptionStage, FiscalAdoptionState, FiscalAppointmentDecision,
    FiscalAssessment, FiscalAuditFinding, FiscalAuthorityBinding, FiscalBookStatus,
    FiscalCatalogRecord, FiscalCycleBook, FiscalExecutionEvidence, FiscalExecutionReceipt,
    FiscalExecutionReceiptPacket, FiscalExecutionRequest, FiscalExternalOperationRef,
    FiscalHistoricalContext, FiscalHistoricalContextPacket, FiscalIssuerRole,
    FiscalObserverBinding, FiscalRebinding, FiscalReceiptDisposition, FiscalRecentOutcome,
    FiscalRemission, FiscalReport, FiscalReportPlan, FiscalReportRecord, FiscalScopeLedger,
    FiscalState, FiscalStateRecord, MAX_FISCAL_ACTION_JSON_BYTES, MAX_FISCAL_ACTION_TEXT_BYTES,
    MAX_FISCAL_ACTIONS_PER_BOUNDARY, MAX_FISCAL_EVIDENCE_PER_RECORD, MAX_FISCAL_EXECUTION_RECEIPTS,
    MAX_FISCAL_HOT_BOOKS_PER_LEDGER, MAX_FISCAL_IDENTIFIER_BYTES, MAX_FISCAL_OBSERVERS,
    MAX_FISCAL_REFERENCES_PER_DEFINITION, MAX_FISCAL_REPORT_PARTS, MAX_FISCAL_REPORT_PLAN_BYTES,
    MAX_FISCAL_STATE_JSON_BYTES, fiscal_catalog_reference, fiscal_state_reference, invalid,
    validate_identifier,
};
use crate::projection::{committed_fiscal_catalog, committed_fiscal_state};
use canwu_api::{
    ArchiveReachabilityManifest, BoundaryContext, BoundaryDirective, BoundaryPhase,
    BoundaryProposal, BoundarySystemContract, Canwu, CanwuError, CanwuViewer, CauseRef, Command,
    CommandContext, CommandIngress, CompactedCanwu, CoreEntityKind, DecisionOrigin, DomainRecord,
    DomainRecordKind, DomainRecordMutation, DomainRecordMutationPolicy, DomainRecordSchema,
    DomainRecordType, DomainRecordVersionRef, DomainRecordVersionSource, DomainReferenceSchema,
    DomainReferenceTargetKind, EntityRef, ErrorCode, EvidenceCursor, EvidenceRef, IngressClass,
    IngressId, IngressPayload, IngressReceipt, Issuer, KnowledgeHolderRef, KnowledgeOrigin,
    KnowledgeQuery, KnowledgeRecordDraft, KnowledgeRecordKind, KnowledgeSchemaId, KnowledgeSubject,
    KnowledgeSubjectSchema, KnowledgeSubjectTarget, KnowledgeSubjectTargetKind,
    KnowledgeWriteGrant, LifeState, MAX_KNOWLEDGE_PAGE_SIZE,
    PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD, PayloadSchema, PersonId, PluginActionDescriptor,
    PluginArchiveObjectProvider, PluginArchiveRetention, PluginIngressDescriptor,
    PluginIngressPermit, PluginIngressRequest, PluginKnowledgeSchema, PluginRegistrar, SimTime,
    SimulationPlugin, SimulationView, StateKey, StateVisibility, SystemCadence, SystemDirective,
    canonical_hash, payload_required_evidence_continuation_property_v1,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

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
/// The internal packet [`enqueue_fiscal_archive`] queues: one stored and
/// verified archive commit. Commits settle before every other fiscal packet
/// of their boundary.
pub const FISCAL_ARCHIVE_COMMIT_INGRESS: &str = "fiscal_archive_commit_v1";
/// The internal packet [`finalize_fiscal_archive_retention`] queues: the
/// acknowledgement that retires one settled commit's retention handle. It
/// settles after the commits and before every other fiscal packet.
pub const FISCAL_ARCHIVE_RETENTION_INGRESS: &str = "fiscal_archive_retention_v1";
/// Event that is the only record of a fiscal packet settlement skips without
/// changing anything: an execution receipt or historical-context change that
/// is refused, an archive packet that does not decode or bind, or an action
/// packet that is malformed or was not produced by the admission of the
/// command it names. No fiscal packet fails its boundary. The summary names
/// the ingress and the packet and ends with the stable reason: for a receipt
/// one of [`crate::FISCAL_RECEIPT_REQUEST_UNKNOWN`] and the receipt reasons
/// after it, for a context change [`crate::FISCAL_CONTEXT_OUTSIDE_PACK`], and
/// for any packet [`crate::FISCAL_INGRESS_MALFORMED_REJECTION`],
/// [`crate::FISCAL_INGRESS_INVALID_REJECTION`],
/// [`crate::FISCAL_INGRESS_BUDGET_REJECTION`], or
/// [`crate::FISCAL_INGRESS_ORIGIN_REJECTION`].
pub const FISCAL_INGRESS_REJECTED_EVENT: &str = "canwu.fiscal.ingress_rejected.v1";
/// Event recorded for every settled action, applied or rejected: the durable
/// record of its outcome once the outcome leaves its target's ring. Its
/// summary is read with [`FiscalSettledAction::from_summary`].
pub const FISCAL_ACTION_SETTLED_EVENT: &str = "canwu.fiscal.action_settled.v1";
/// Event recorded for every settled archive commit, applied or stale.
pub const FISCAL_ARCHIVE_MAINTENANCE_EVENT: &str = "canwu.fiscal.archive_maintenance.v1";

const PLUGIN_VERSION: &str = "0.2.0-experimental";
const SEMANTIC_HASH: &str = "5f0c2d8e41a7b39c6e1d07f4a28b5c93e6d41f0a7b2c58e9d13f6a04b7c2e851";
const FISCAL_REPORT_KNOWLEDGE: &str = "fiscal_report";
/// Version 1 was one record for an observer's whole report. Version 2 is a
/// summary record and part records, each on a report line of its own.
/// Version 3 numbers the lines from each binding's own slot, lets an
/// institution hold a report without a person, and marks a withdrawn
/// summary.
const FISCAL_REPORT_SCHEMA_VERSION: u32 = 3;
const FISCAL_REPORT_SCHEMA_HASH: &str =
    "b41e7d03c9a2f58e16d4b07a3c95e2f81a6d0c47e3b29f5a81c6d04e7b3a92f5";
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
// One observer's summary and parts are one publication, and the engine's
// limit on a batch's records cannot be reached by one.
const _: () =
    assert!(MAX_FISCAL_REPORT_PARTS < canwu_api::KnowledgeLimitsV1::CURRENT.records_per_batch);
const ACTION_DROPPED_EVENT: &str = "canwu.fiscal.action_dropped.v1";
const RECEIPT_RECORDED_EVENT: &str = "canwu.fiscal.execution_receipt_recorded.v1";
const CONTEXT_CHANGED_EVENT: &str = "canwu.fiscal.historical_context_changed.v1";
const STALE_ACTION: &str = "fiscal action was stale at its settlement boundary";
const STALE_OVERWRITE: &str = "fiscal action was stale at its settlement boundary: an earlier action of the boundary changed an adoption or binding it overwrites";

static ARCHIVE_COMMIT_PERMIT: OnceLock<PluginIngressPermit> = OnceLock::new();
static ARCHIVE_RETENTION_PERMIT: OnceLock<PluginIngressPermit> = OnceLock::new();

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct AdmittedFiscalAction {
    request: FiscalActionRequest,
    command: canwu_api::CommandId,
    /// Who issued the command and in which role, as admission resolved it.
    /// Settlement re-checks the role against the state it settles on.
    issuer: FiscalActionIssuer,
    /// The record whose ring keeps the action's outcome, as admission
    /// resolved it.
    target: FiscalActionTarget,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct FiscalArchiveCommitPacket {
    commit: VerifiedFiscalArchiveCommitV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct FiscalArchiveRetentionPacket {
    handle_id: String,
}

/// One admitted ingress item of this plugin, decoded in settlement order.
enum FiscalPacket {
    Action(Box<AdmittedFiscalAction>),
    Receipt(IngressId, FiscalExecutionReceiptPacket),
    HistoricalContext(IngressId, FiscalHistoricalContextPacket),
    ArchiveCommit(IngressId, Box<VerifiedFiscalArchiveCommitV1>),
    ArchiveRetention(String),
    /// A packet that settlement skips: its ingress, its packet type, and the
    /// reason.
    Refused(IngressId, &'static str, &'static str),
}

/// The adoptions and bindings that actions of one boundary have changed.
/// A later action of the same boundary may read them but not overwrite them.
#[derive(Default)]
struct BoundaryOverwrites {
    /// Keyed `scope/adoption`.
    adoptions: BTreeSet<String>,
    authority_bindings: BTreeSet<String>,
    audit_bindings: BTreeSet<String>,
    observer_bindings: BTreeSet<String>,
}

struct ValidatedExecutionEvidence {
    quantity: u64,
    disposition: FiscalReceiptDisposition,
    external_operations: BTreeSet<FiscalExternalOperationRef>,
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
        registrar.register_archive_reachability_participant(fiscal_archive_reachability)?;

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
            FISCAL_ACTION_SETTLED_EVENT.to_owned(),
            FISCAL_ARCHIVE_MAINTENANCE_EVENT.to_owned(),
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
        // whose observer binding names a holder who does not exist is
        // refused when a run is created or restored.
        DomainReferenceSchema {
            role: FISCAL_STATE_OBSERVER_ROLE.to_owned(),
            targets: vec![
                DomainReferenceTargetKind::Core(CoreEntityKind::Government),
                DomainReferenceTargetKind::Core(CoreEntityKind::Person),
            ],
            required: false,
            multiple: true,
            allow_retired: false,
        },
    ];
    registrar.register_record_schema(state_schema)
}

/// Registers the plugin's ingress types. Only the command's admission queues
/// an action, only the aggregate system schedules a drain, and only the
/// archive functions queue a commit or an acknowledgement, so those are
/// internal and a host can author none of them; receipts and context changes
/// are host ingress.
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
    let commit_permit = registrar.register_internal_ingress_with_archive_retention(
        PluginIngressDescriptor {
            name: FISCAL_ARCHIVE_COMMIT_INGRESS.to_owned(),
            description: "Commit one host-verified fiscal archive batch".to_owned(),
            class: IngressClass::ScheduledSystem,
            payload_schema: PayloadSchema::Any,
        },
        FISCAL_ARCHIVE_INDEX_DIRECTORY_NAMESPACE,
        vec!["/commit/directory_root".to_owned()],
    )?;
    keep_permit(&ARCHIVE_COMMIT_PERMIT, &commit_permit)?;
    let retention_permit = registrar.register_internal_ingress(PluginIngressDescriptor {
        name: FISCAL_ARCHIVE_RETENTION_INGRESS.to_owned(),
        description: "Acknowledge the finalized retention of one fiscal archive commit".to_owned(),
        class: IngressClass::Acknowledgement,
        payload_schema: PayloadSchema::Any,
    })?;
    keep_permit(&ARCHIVE_RETENTION_PERMIT, &retention_permit)?;
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

/// Keeps the permit of an internal ingress for the host functions that
/// enqueue it. Every registration of the plugin yields the same permit.
fn keep_permit(
    slot: &OnceLock<PluginIngressPermit>,
    permit: &PluginIngressPermit,
) -> Result<(), CanwuError> {
    if slot.set(permit.clone()).is_err() && slot.get() != Some(permit) {
        return Err(CanwuError::new(
            ErrorCode::InvalidPluginRegistration,
            "fiscal archive ingress permit changed across registrations",
        ));
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
/// boundary. Admission refuses, from the committed state alone, a request
/// whose action ID has an outcome in its target's ring (`IdempotencyConflict`),
/// one that names a procedure revision the state never had
/// ([`FISCAL_REVISION_UNKNOWN`]), and one whose revision is at or below its
/// target's outcome floor ([`FISCAL_REVISION_EXPIRED`]). Whether the revision
/// is still current is checked only at settlement, where a stale action is
/// recorded as a rejected outcome.
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
    let (issuer, institution) = admit_issuer(view, context, &state, &request)?;
    admit_action(view, &state, &request, &issuer, institution)?;
    let target = action_target(&state, &request.action, institution)?;
    admit_revision(&state, &target, &request)?;
    // The engine checks every affected entity of a queued item against its
    // entity registry, which also holds persons that have no record in the
    // world projection. Naming the persons and governments of a rebind here
    // therefore rejects the command as `EntityNotFound` when one of them does
    // not exist.
    let mut affected: Vec<_> = std::iter::once(institution.clone())
        .chain(named_entities(&request.action))
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
            target,
        })
        .map_err(encode_error)?,
    }])
}

/// Refuses a request whose action ID has an outcome in its target's ring,
/// that names a revision the state never had, or one at or below the
/// target's outcome floor. None of the three consumes anything.
fn admit_revision(
    state: &FiscalState,
    target: &FiscalActionTarget,
    request: &FiscalActionRequest,
) -> Result<(), CanwuError> {
    let ring = state
        .outcome_ring(target)
        .ok_or_else(|| not_found("fiscal action target is unavailable"))?;
    if ring.get(&request.action_id).is_some() {
        return Err(CanwuError::new(
            ErrorCode::IdempotencyConflict,
            "fiscal action identity was already settled",
        ));
    }
    if request.expected_procedure_revision > state.procedure_revision {
        return Err(CanwuError::new(
            ErrorCode::ValueOutOfRange,
            FISCAL_REVISION_UNKNOWN,
        ));
    }
    if request.expected_procedure_revision <= ring.floor {
        return Err(CanwuError::new(
            ErrorCode::ValueOutOfRange,
            FISCAL_REVISION_EXPIRED,
        ));
    }
    Ok(())
}

/// The target of an action whose binding concerns `institution`: the scope
/// ledger of a scope action, the office of the institution a binding,
/// proposal, or transition action changes, or the observer binding an
/// observer action changes.
fn action_target(
    state: &FiscalState,
    action: &FiscalAction,
    institution: &EntityRef,
) -> Result<FiscalActionTarget, CanwuError> {
    Ok(match action {
        FiscalAction::ChangeAdoption {
            scope_binding_id, ..
        }
        | FiscalAction::OpenAssessment {
            scope_binding_id, ..
        }
        | FiscalAction::GrantRemission {
            scope_binding_id, ..
        }
        | FiscalAction::AuthorizeExecution {
            scope_binding_id, ..
        }
        | FiscalAction::RecordAudit {
            scope_binding_id, ..
        }
        | FiscalAction::CloseAccountingCycle {
            scope_binding_id, ..
        } => FiscalActionTarget::Ledger(scope_binding_id.clone()),
        FiscalAction::ApplyTransition { .. }
        | FiscalAction::RebindAuthority { .. }
        | FiscalAction::RebindAudit { .. } => FiscalActionTarget::Office(institution.clone()),
        FiscalAction::NominateAuditor {
            institution: audited,
            ..
        } => FiscalActionTarget::Office(audited.clone()),
        FiscalAction::PetitionAuditorRemoval {
            audit_binding_id, ..
        } => {
            FiscalActionTarget::Office(audit_binding(state, audit_binding_id)?.institution.clone())
        }
        FiscalAction::DecideAuditAppointment {
            audit_binding_id,
            decision,
        } => FiscalActionTarget::Office(
            decided_institution(state, audit_binding_id, decision)?.clone(),
        ),
        FiscalAction::RebindObserver {
            observer_binding_id,
            ..
        }
        | FiscalAction::RemoveObserver {
            observer_binding_id,
        } => FiscalActionTarget::Observer(observer_binding_id.clone()),
    })
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
                    "a fiscal acting actor cannot rebind an authority, audit, or observer binding, nominate, petition, or decide an appointment",
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
/// the part an appointment action takes under its policy, or the observer
/// binding an observer action changes.
#[allow(clippy::too_many_lines)]
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
        FiscalAction::RebindObserver {
            observer_binding_id,
            ..
        }
        | FiscalAction::RemoveObserver {
            observer_binding_id,
        } => {
            let observer = state
                .observer_bindings
                .get(observer_binding_id)
                .ok_or_else(|| not_found("fiscal observer binding is unavailable"))?;
            if observer.controlled_by.as_ref() != Some(institution) {
                return Err(invalid_authority(
                    "a fiscal observer binding is re-pointed or removed only by the institution that controls it",
                ));
            }
            if !observer.is_active() {
                return Err(invalid_payload(FISCAL_OBSERVER_REMOVED));
            }
            if let FiscalAction::RebindObserver {
                actor,
                knowledge_holder,
                ..
            } = &request.action
                && !holder_matches(*actor, knowledge_holder)
            {
                return Err(invalid_payload(
                    "a fiscal observer holder is its actor's person or, without an actor, a government",
                ));
            }
            Ok(())
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

/// Whether `holder` is what an observer binding with `actor` may hold its
/// report as: that person, or, without an actor, a government.
fn holder_matches(actor: Option<PersonId>, holder: &KnowledgeHolderRef) -> bool {
    match (actor, holder) {
        (Some(actor), KnowledgeHolderRef::Person(person)) => actor == *person,
        (None, KnowledgeHolderRef::Entity(EntityRef::Government(_))) => true,
        _ => false,
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
#[allow(clippy::too_many_lines)]
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
            scope_binding_id,
            assessment_id,
            reason,
            ..
        } => {
            created(remission_id, "fiscal remission")?;
            reference(scope_binding_id)?;
            reference(assessment_id)?;
            text(reason, "remission reason")
        }
        FiscalAction::AuthorizeExecution {
            request_id,
            scope_binding_id,
            assessment_id,
            unit: requested_unit,
            purpose,
            ..
        } => {
            created(request_id, "fiscal execution request")?;
            reference(scope_binding_id)?;
            reference(assessment_id)?;
            unit(requested_unit)?;
            text(purpose, "execution purpose")
        }
        FiscalAction::RecordAudit {
            audit_id,
            scope_binding_id,
            target_id,
            finding,
            evidence,
            ..
        } => {
            created(audit_id, "fiscal audit")?;
            reference(scope_binding_id)?;
            reference(target_id)?;
            text(finding, "audit finding")?;
            if evidence.len() > MAX_FISCAL_EVIDENCE_PER_RECORD {
                return Err(invalid_payload(
                    "fiscal audit cites more evidence than one record may hold",
                ));
            }
            Ok(())
        }
        FiscalAction::CloseAccountingCycle {
            scope_binding_id,
            accounting_cycle_id,
        } => {
            reference(scope_binding_id)?;
            reference(accounting_cycle_id)
        }
        FiscalAction::RebindAuthority {
            authority_binding_id,
            ..
        } => reference(authority_binding_id),
        FiscalAction::RebindAudit {
            audit_binding_id, ..
        } => created(audit_binding_id, "fiscal audit binding"),
        FiscalAction::RebindObserver {
            observer_binding_id,
            ..
        }
        | FiscalAction::RemoveObserver {
            observer_binding_id,
        } => reference(observer_binding_id),
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

/// Checks that the issuer's institution owns every scope an action names,
/// and that the object a scope action cites is one of the scope's hot
/// objects.
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
                    .scope_binding(scope_id)
                    .is_none_or(|scope| &scope.institution != institution)
            })
        {
            return Err(invalid_authority(
                "fiscal transition authority must own every target scope",
            ));
        }
        return Ok(());
    }
    let (scope_id, cited) = match action {
        FiscalAction::ChangeAdoption {
            scope_binding_id, ..
        }
        | FiscalAction::OpenAssessment {
            scope_binding_id, ..
        }
        | FiscalAction::CloseAccountingCycle {
            scope_binding_id, ..
        } => (scope_binding_id, None),
        FiscalAction::GrantRemission {
            scope_binding_id,
            assessment_id,
            ..
        }
        | FiscalAction::AuthorizeExecution {
            scope_binding_id,
            assessment_id,
            ..
        } => (scope_binding_id, Some((assessment_id, false))),
        FiscalAction::RecordAudit {
            scope_binding_id,
            target_id,
            ..
        } => (scope_binding_id, Some((target_id, true))),
        // A transition was checked above, and the binding and observer
        // actions name a binding rather than a scope; admission checks them
        // separately.
        FiscalAction::ApplyTransition { .. }
        | FiscalAction::RebindAuthority { .. }
        | FiscalAction::RebindAudit { .. }
        | FiscalAction::NominateAuditor { .. }
        | FiscalAction::PetitionAuditorRemoval { .. }
        | FiscalAction::DecideAuditAppointment { .. }
        | FiscalAction::RebindObserver { .. }
        | FiscalAction::RemoveObserver { .. } => return Ok(()),
    };
    let ledger = state
        .ledger(scope_id)
        .ok_or_else(|| not_found("fiscal action scope is unavailable"))?;
    if ledger.scope.institution != *institution {
        return Err(invalid_authority(
            "fiscal authority binding does not own the action scope",
        ));
    }
    match cited {
        Some((id, false)) if ledger.book_of_assessment(id).is_none() => {
            Err(not_found("fiscal action names an unknown assessment"))
        }
        Some((id, true)) if ledger.book_of_audit_target(id).is_none() => {
            Err(not_found("fiscal audit target is unavailable"))
        }
        _ => Ok(()),
    }
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

/// One admitted ingress item addressed to this plugin, before it is decoded.
struct AdmittedPacket<'a> {
    ingress: IngressId,
    packet_type: &'a str,
    payload: &'a Value,
    cause: Option<&'a CauseRef>,
}

/// Lists the boundary's admitted ingress that this plugin settles, without
/// decoding any payload, in settlement order: archive commits, then archive
/// acknowledgements, then the rest, each group in admission order.
///
/// Every other fiscal packet can advance the procedure revision, which an
/// archive commit's source root binds, so a commit prepared from the state
/// the boundary started from applies even when an action, receipt, or
/// context change was admitted ahead of it.
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
                    | FISCAL_ARCHIVE_COMMIT_INGRESS
                    | FISCAL_ARCHIVE_RETENTION_INGRESS
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
    packets.sort_by_key(|packet| match packet.packet_type {
        FISCAL_ARCHIVE_COMMIT_INGRESS => 0,
        FISCAL_ARCHIVE_RETENTION_INGRESS => 1,
        _ => 2,
    });
    Ok(packets)
}

/// Decodes the boundary's packets. A packet whose payload does not decode,
/// or an action packet that its command did not produce, is kept as a
/// refusal: it changes nothing and does not fail the boundary.
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
            FISCAL_ARCHIVE_COMMIT_INGRESS => {
                match decode::<FiscalArchiveCommitPacket>(packet.payload, "fiscal archive commit") {
                    Ok(commit) => {
                        FiscalPacket::ArchiveCommit(packet.ingress, Box::new(commit.commit))
                    }
                    Err(_) => malformed(packet, FISCAL_ARCHIVE_COMMIT_INGRESS),
                }
            }
            FISCAL_ARCHIVE_RETENTION_INGRESS => {
                match decode::<FiscalArchiveRetentionPacket>(
                    packet.payload,
                    "fiscal archive retention acknowledgement",
                ) {
                    Ok(acknowledgement) => {
                        FiscalPacket::ArchiveRetention(acknowledgement.handle_id)
                    }
                    Err(_) => malformed(packet, FISCAL_ARCHIVE_RETENTION_INGRESS),
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
    /// The action IDs the boundary has settled, by target, so that a copy
    /// of an action is skipped even when an eviction took the first copy's
    /// outcome out of the ring within the boundary.
    settled: BTreeMap<FiscalActionTarget, BTreeSet<String>>,
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

/// Settles the boundary's fiscal ingress in settlement order.
///
/// Archive commits and acknowledgements settle first. Every action is then
/// compared with the procedure revision the state had when the boundary
/// began, so actions admitted against one revision settle together, each
/// applied to the state the previous one left. An action that fails its own
/// checks is recorded as a rejected outcome and does not affect the actions
/// after it.
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

/// Settles the packets on `state`, in settlement order.
///
/// With `measured`, the measure of `state`, the pass also holds every packet
/// to `budget`: an action whose result would not fit is rejected, and one
/// whose outcome would not fit is dropped; a receipt or a historical-context
/// change that would not fit changes nothing and is refused with an event.
/// Archive maintenance is never refused for the budget: the budget leaves
/// [`FISCAL_ARCHIVE_HEAD_RESERVE_BYTES`] for it, and the state is measured
/// again after it. Without `measured` nothing is measured; the caller
/// measures the result once and repeats the pass with measurement only when
/// it does not fit.
#[allow(clippy::too_many_lines)]
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
        settled: BTreeMap::new(),
        measured,
        budget,
        changed: false,
        events: Vec::new(),
    };
    let mut maintained = false;
    for packet in packets {
        if maintained
            && !matches!(
                packet,
                FiscalPacket::ArchiveCommit(..) | FiscalPacket::ArchiveRetention(_)
            )
        {
            maintained = false;
            if settlement.measured.is_some() {
                settlement.measured = Some(measure_writes(&settlement.state, catalog, context.at)?);
            }
        }
        match packet {
            FiscalPacket::ArchiveCommit(ingress_id, commit) => {
                let mut next = settlement.state.clone();
                match next.apply_fiscal_archive_commit(commit, context.at) {
                    Ok(FiscalArchiveSettlement::Duplicate { handle_id }) => {
                        settlement.events.push(SettledEvent {
                            event_type: FISCAL_ARCHIVE_MAINTENANCE_EVENT,
                            summary: format!(
                                "Settled fiscal archive commit for handle {handle_id} as stale duplicate: nothing recorded"
                            ),
                            affected: Vec::new(),
                        });
                    }
                    Ok(FiscalArchiveSettlement::Recorded(receipt)) => {
                        settlement.state = next;
                        settlement.changed = true;
                        maintained = true;
                        settlement.state.report_plan.settle_maintenance();
                        settlement.events.push(SettledEvent {
                            event_type: FISCAL_ARCHIVE_MAINTENANCE_EVENT,
                            summary: format!(
                                "Settled fiscal archive commit {} as {}: {} books, {} institution totals, {} receipts",
                                receipt.id,
                                if receipt.disposition == FiscalArchiveRetentionPhaseV1::Committed {
                                    "applied"
                                } else {
                                    "stale"
                                },
                                receipt.archived_books,
                                receipt.archived_totals,
                                receipt.archived_receipts
                            ),
                            affected: Vec::new(),
                        });
                    }
                    Err(error) => settlement.events.push(refused_ingress(
                        *ingress_id,
                        FISCAL_ARCHIVE_COMMIT_INGRESS,
                        &format!("{}: {FISCAL_INGRESS_INVALID_REJECTION}", error.message),
                        Vec::new(),
                    )),
                }
            }
            FiscalPacket::ArchiveRetention(handle_id) => {
                if settlement
                    .state
                    .acknowledge_fiscal_archive_retention(handle_id)
                {
                    settlement.changed = true;
                    maintained = true;
                    settlement.state.report_plan.settle_maintenance();
                }
            }
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
                            return Err(Refused::new(
                                FISCAL_CONTEXT_OUTSIDE_PACK,
                                "fiscal historical context lies outside the pack",
                            ));
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

/// Why a receipt or historical-context change was refused: its stable
/// reason, and a message that says more.
struct Refused {
    reason: &'static str,
    message: String,
}

impl Refused {
    fn new(reason: &'static str, message: impl Into<String>) -> Self {
        Self {
            reason,
            message: message.into(),
        }
    }
}

impl From<CanwuError> for Refused {
    fn from(error: CanwuError) -> Self {
        Self::new(FISCAL_INGRESS_INVALID_REJECTION, error.message)
    }
}

impl Settlement {
    /// Settles one admitted action: records its outcome, or drops the action
    /// when no outcome can be recorded for it. An action whose ID already has
    /// an outcome for its target, in the ring or settled earlier in this
    /// boundary, is skipped without one.
    fn settle_action(
        &mut self,
        view: &SimulationView<'_>,
        catalog: &CompiledFiscalCatalog,
        admitted: &AdmittedFiscalAction,
        at: SimTime,
    ) -> Result<(), CanwuError> {
        let request = &admitted.request;
        let target = &admitted.target;
        if self
            .state
            .recent_outcome(target, &request.action_id)
            .is_some()
            || self
                .settled
                .get(target)
                .is_some_and(|settled| settled.contains(&request.action_id))
        {
            return Ok(());
        }
        let verdict = if request.expected_procedure_revision == self.entry_revision {
            settlement_authority(view, &self.state, admitted)
                .and_then(|()| self.apply(catalog, admitted, at))
        } else {
            Err(stale(STALE_ACTION))
        };
        let disposition = if verdict.is_ok() {
            FiscalActionDisposition::Applied
        } else {
            FiscalActionDisposition::Rejected
        };
        // The outcome of an applied action was measured with its state. A
        // rejected one adds its ring entry to every record the boundary
        // writes, and the procedure revision it advances may gain a digit;
        // the report plan, which carries the revision too, is outside the
        // measure.
        if verdict.is_err() && self.measured.is_some() {
            let mut recorded = self.state.clone();
            record_outcome(&mut recorded, target, &request.action_id, disposition)?;
            let bytes = measure_writes(&recorded, catalog, at)?;
            if bytes > self.budget {
                self.events.push(dropped_action(
                    &self.state,
                    request,
                    "the fiscal state byte budget cannot hold its outcome",
                ));
                return Ok(());
            }
            self.measured = Some(bytes);
        }
        let affected = if verdict.is_ok() {
            rebound_entities(&self.state, request)
        } else {
            Vec::new()
        };
        record_outcome(&mut self.state, target, &request.action_id, disposition)?;
        self.settled
            .entry(target.clone())
            .or_default()
            .insert(request.action_id.clone());
        self.changed = true;
        let reason = match &verdict {
            Ok(()) => "applied".to_owned(),
            Err(error) => error.to_string(),
        };
        self.events.push(SettledEvent {
            event_type: FISCAL_ACTION_SETTLED_EVENT,
            summary: FiscalSettledAction {
                target: target.to_string(),
                action_id: request.action_id.clone(),
                disposition,
                reason,
            }
            .summary(),
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
    /// No such packet fails the boundary. One that is refused, for which
    /// `change` returns its reason and leaves the state as it was, changes
    /// nothing and is refused with an event whose summary ends with the
    /// reason. In a measured pass a change whose result would not fit the
    /// budget is not made either, and is refused with
    /// [`FISCAL_INGRESS_BUDGET_REJECTION`]. The refusal event is the packet's
    /// only record.
    fn settle_change(
        &mut self,
        catalog: &CompiledFiscalCatalog,
        at: SimTime,
        change: impl FnOnce(&mut FiscalState) -> Result<bool, Refused>,
        settled: (&'static str, &str),
        refused: Refusal,
    ) -> Result<(), CanwuError> {
        let text = |refusal: &Refused| format!("{}: {}", refusal.message, refusal.reason);
        if self.measured.is_none() {
            match change(&mut self.state) {
                Ok(true) => {}
                Ok(false) => return Ok(()),
                Err(refusal) => {
                    self.events.push(refused.event(&text(&refusal)));
                    return Ok(());
                }
            }
        } else {
            let mut next = self.state.clone();
            match change(&mut next) {
                Ok(true) => {}
                Ok(false) => return Ok(()),
                Err(refusal) => {
                    self.events.push(refused.event(&text(&refusal)));
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

/// Records one outcome in its target's ring, at the current procedure
/// revision, and advances the revision.
fn record_outcome(
    state: &mut FiscalState,
    target: &FiscalActionTarget,
    action_id: &str,
    disposition: FiscalActionDisposition,
) -> Result<(), CanwuError> {
    let settled_revision = state.procedure_revision;
    state.outcome_ring_mut(target)?.push(FiscalRecentOutcome {
        action_id: action_id.to_owned(),
        disposition,
        settled_revision,
    });
    increment_procedure_revision(state)
}

/// One settled action as its `canwu.fiscal.action_settled.v1` event records
/// it: the durable record of every outcome, applied or rejected.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FiscalSettledAction {
    /// The action's target, as [`FiscalActionTarget`] displays it.
    pub target: String,
    pub action_id: String,
    pub disposition: FiscalActionDisposition,
    /// `applied`, or why the action was rejected.
    pub reason: String,
}

impl FiscalSettledAction {
    fn summary(&self) -> String {
        format!(
            "Settled fiscal action {} on {} as {}: {}",
            self.action_id,
            self.target,
            match self.disposition {
                FiscalActionDisposition::Applied => "applied",
                FiscalActionDisposition::Rejected => "rejected",
            },
            self.reason
        )
    }

    /// Reads the summary of a `canwu.fiscal.action_settled.v1` event.
    #[must_use]
    pub fn from_summary(summary: &str) -> Option<Self> {
        let rest = summary.strip_prefix("Settled fiscal action ")?;
        let (action_id, rest) = rest.split_once(" on ")?;
        let (target, rest) = rest.split_once(" as ")?;
        let (disposition, reason) = rest.split_once(": ")?;
        Some(Self {
            target: target.to_owned(),
            action_id: action_id.to_owned(),
            disposition: match disposition {
                "applied" => FiscalActionDisposition::Applied,
                "rejected" => FiscalActionDisposition::Rejected,
                _ => return None,
            },
            reason: reason.to_owned(),
        })
    }

    /// Every settled action an event list records, in order.
    #[must_use]
    pub fn from_events(events: &[canwu_api::SimEvent]) -> Vec<Self> {
        events
            .iter()
            .filter(|event| {
                event.kind.plugin_identity() == Some((PLUGIN_NAME, FISCAL_ACTION_SETTLED_EVENT))
            })
            .filter_map(|event| Self::from_summary(&event.summary))
            .collect()
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
/// only in those fields and in the withdrawals the aggregate system moves
/// into the plan, so their sizes follow from `encoded` and the sizes of the
/// fields alone, and the largest of them is exact, however much the derived
/// fields grow or shrink. The report plan is left out because
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

/// The bytes, without the report plan, that settlement lets any record of
/// its boundary take: the state's byte budget less the largest report plan
/// and the reserve for archive maintenance.
fn settlement_budget() -> usize {
    #[cfg(test)]
    if let Some(bytes) = tests::SETTLEMENT_BUDGET.with(std::cell::Cell::get) {
        return bytes;
    }
    MAX_FISCAL_STATE_JSON_BYTES - MAX_FISCAL_REPORT_PLAN_BYTES - FISCAL_ARCHIVE_HEAD_RESERVE_BYTES
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

/// The institution of a receipt's scope, when an event may name it.
fn receipt_institution(
    state: &FiscalState,
    packet: &FiscalExecutionReceiptPacket,
) -> Vec<EntityRef> {
    state
        .scope_binding(&packet.scope_binding_id)
        .and_then(|scope| registered_entity(&scope.institution))
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

/// The persons and governments a rebind, nomination, direct appointment, or
/// observer re-pointing names.
fn named_entities(action: &FiscalAction) -> Vec<EntityRef> {
    match action {
        FiscalAction::RebindAuthority {
            authorized_actor,
            acting_actor,
            ..
        } => authorized_actor
            .iter()
            .chain(acting_actor)
            .copied()
            .map(EntityRef::Person)
            .collect(),
        FiscalAction::RebindAudit { auditor, .. } => {
            auditor.iter().copied().map(EntityRef::Person).collect()
        }
        FiscalAction::NominateAuditor { candidate, .. } => vec![EntityRef::Person(*candidate)],
        FiscalAction::DecideAuditAppointment {
            decision: FiscalAppointmentDecision::Appoint { auditor, .. },
            ..
        } => vec![EntityRef::Person(*auditor)],
        FiscalAction::RebindObserver {
            knowledge_holder, ..
        } => match knowledge_holder {
            KnowledgeHolderRef::Person(person) => vec![EntityRef::Person(*person)],
            KnowledgeHolderRef::Entity(entity) => vec![entity.clone()],
        },
        _ => Vec::new(),
    }
}

/// The institution, persons, and governments an applied rebind, appointment,
/// or observer action concerns, and nothing for any other action. The engine
/// checks the affected entities of an emitted event against its entity
/// registry when the boundary commits, so such an action can never commit a
/// binding to an entity that does not exist.
fn rebound_entities(state: &FiscalState, request: &FiscalActionRequest) -> Vec<EntityRef> {
    if !is_appointment_action(&request.action) {
        return Vec::new();
    }
    let mut affected: Vec<_> = state
        .authority_bindings
        .get(&request.authority_binding_id)
        .and_then(|binding| registered_entity(&binding.institution))
        .into_iter()
        .chain(named_entities(&request.action))
        .collect();
    affected.sort();
    affected.dedup();
    affected
}

/// Settles one execution receipt in the book of its request, or refuses it
/// with its reason. A receipt whose ID a hot receipt of the scope already has
/// with the same request and evidence is a resend and changes nothing.
fn settle_execution_receipt(
    view: &SimulationView<'_>,
    state: &mut FiscalState,
    packet: FiscalExecutionReceiptPacket,
    ingress_id: IngressId,
    at: SimTime,
    catalog: &CompiledFiscalCatalog,
) -> Result<bool, Refused> {
    let unknown = || {
        Refused::new(
            FISCAL_RECEIPT_REQUEST_UNKNOWN,
            "fiscal execution receipt names no hot execution request of its scope",
        )
    };
    let ledger = state.ledger(&packet.scope_binding_id).ok_or_else(unknown)?;
    let book = ledger
        .book_of_request(&packet.request_id)
        .ok_or_else(unknown)?;
    // A resent receipt cites the same exact versions, so it would derive the
    // same content. Settle it unchanged before its evidence is checked again,
    // since that evidence may no longer be current.
    if let Some(existing) = ledger
        .books
        .values()
        .find_map(|book| book.execution_receipts.get(&packet.receipt_id))
    {
        if existing.request_id == packet.request_id
            && existing.external_evidence == packet.external_evidence
        {
            return Ok(false);
        }
        return Err(Refused::new(
            FISCAL_RECEIPT_ID_CONFLICT,
            "fiscal receipt identity was reused with different content",
        ));
    }
    if !book.is_open() {
        return Err(Refused::new(
            FISCAL_RECEIPT_CYCLE_CLOSED,
            "fiscal execution receipt names a request of a closed book",
        ));
    }
    if state.execution_receipts().count() >= MAX_FISCAL_EXECUTION_RECEIPTS {
        return Err(Refused::new(
            FISCAL_INGRESS_BUDGET_REJECTION,
            "the fiscal state holds as many hot execution receipts as it may",
        ));
    }
    let evidence = validate_execution_evidence(view, state, ledger, book, &packet)?;
    let cycle = book.accounting_cycle_id.clone();
    let scope = packet.scope_binding_id.clone();
    let receipt_id = packet.receipt_id.clone();
    let ValidatedExecutionEvidence {
        quantity,
        disposition,
        external_operations,
    } = evidence;
    let receipt = FiscalExecutionReceipt {
        id: packet.receipt_id,
        request_id: packet.request_id,
        quantity,
        disposition,
        external_evidence: packet.external_evidence,
        external_operations,
        accepted_ingress: ingress_id,
        observed_at: at,
    };
    book_mut(state, &scope, &cycle)?
        .execution_receipts
        .insert(receipt_id.clone(), receipt);
    // A receipt the state does not accept leaves the state as it was, so
    // that settlement can refuse it without failing the boundary. A receipt
    // changes one book of one ledger, so that ledger is what can break.
    if let Err(error) = state.validate_scope(&scope, catalog) {
        book_mut(state, &scope, &cycle)?
            .execution_receipts
            .remove(&receipt_id);
        return Err(error.into());
    }
    increment_procedure_revision(state)?;
    Ok(true)
}

/// Checks a receipt's evidence against its request and its scope's hot
/// receipts, and derives what the receipt records.
#[allow(clippy::too_many_lines)]
fn validate_execution_evidence(
    view: &SimulationView<'_>,
    state: &FiscalState,
    ledger: &FiscalScopeLedger,
    book: &FiscalCycleBook,
    packet: &FiscalExecutionReceiptPacket,
) -> Result<ValidatedExecutionEvidence, Refused> {
    if packet.external_evidence.is_empty()
        || packet.external_evidence.len() > MAX_FISCAL_EVIDENCE_PER_RECORD
    {
        return Err(Refused::new(
            FISCAL_RECEIPT_EVIDENCE_COUNT,
            "fiscal execution receipt has an invalid external evidence count",
        ));
    }
    let request = book
        .execution_requests
        .get(&packet.request_id)
        .ok_or_else(|| {
            Refused::new(
                FISCAL_RECEIPT_REQUEST_UNKNOWN,
                "fiscal execution receipt names an unavailable execution request",
            )
        })?;
    let hot_receipts = || {
        ledger
            .books
            .values()
            .flat_map(|book| book.execution_receipts.values())
    };
    let consumed_operations: BTreeSet<_> = hot_receipts()
        .flat_map(|receipt| receipt.external_operations.iter())
        .collect();
    let consumed_evidence: BTreeSet<_> = hot_receipts()
        .flat_map(|receipt| receipt.external_evidence.iter())
        .collect();
    let mismatch = || {
        Refused::new(
            FISCAL_RECEIPT_CLAIM_MISMATCH,
            "typed fiscal execution evidence does not match its request or receipt",
        )
    };
    let mut evidenced_quantity = 0_u64;
    let mut disposition = None;
    let mut external_operations = BTreeSet::new();
    for evidence in &packet.external_evidence {
        if !state
            .execution_evidence_kinds
            .contains(&evidence.record.kind)
        {
            return Err(Refused::new(
                FISCAL_RECEIPT_EVIDENCE_KIND_UNAPPROVED,
                "fiscal execution receipt cites an unapproved external evidence kind",
            ));
        }
        if consumed_evidence.contains(evidence) {
            return Err(Refused::new(
                FISCAL_RECEIPT_EVIDENCE_REUSED,
                "one external execution result cannot settle multiple fiscal receipts",
            ));
        }
        // An earlier version's body and time can depend on what a seal kept,
        // so the receipt would settle differently in a compact run than in
        // its exact replay.
        let not_current = || {
            Refused::new(
                FISCAL_RECEIPT_EVIDENCE_NOT_CURRENT,
                "fiscal execution receipt must cite the current version of its external evidence",
            )
        };
        let (record, established_at) = view
            .replay_stable_domain_record_version(evidence)
            .map_err(|_| not_current())?
            .ok_or_else(not_current)?;
        let claim: FiscalExecutionEvidence =
            decode(&record.payload, "typed fiscal execution evidence").map_err(|_| mismatch())?;
        let operation = FiscalExternalOperationRef {
            evidence_kind: evidence.record.kind.clone(),
            external_operation_id: claim.external_operation_id.clone(),
        };
        if consumed_operations.contains(&operation) {
            return Err(Refused::new(
                FISCAL_RECEIPT_OPERATION_REUSED,
                "external fiscal operation was already settled by another receipt",
            ));
        }
        if !external_operations.insert(operation) {
            return Err(Refused::new(
                FISCAL_RECEIPT_OPERATION_REUSED,
                "external fiscal operation was repeated within one receipt",
            ));
        }
        if claim.id != evidence.record.id
            || claim.id.trim().is_empty()
            || claim.external_operation_id.trim().is_empty()
            || claim.scope_binding_id != packet.scope_binding_id
            || claim.request_id != request.id
            || claim.unit != request.unit
            || claim.payment_form != request.payment_form
            || claim.execution_kind != request.kind
            || claim.resource != request.resource
            || claim.source != request.source
            || claim.target != request.target
            || claim.disposition.counts_as_fulfillment() == (claim.quantity == 0)
            || disposition.is_some_and(|value| value != claim.disposition)
            || established_at < request.requested_at
        {
            return Err(mismatch());
        }
        disposition = Some(claim.disposition);
        evidenced_quantity = evidenced_quantity
            .checked_add(claim.quantity)
            .ok_or_else(|| {
                Refused::new(
                    FISCAL_RECEIPT_QUANTITY_EXCEEDED,
                    "fiscal execution evidence quantity overflowed",
                )
            })?;
    }
    let disposition = disposition.ok_or_else(mismatch)?;
    let fulfilled = book
        .execution_receipts
        .values()
        .filter(|receipt| {
            receipt.request_id == request.id && receipt.disposition.counts_as_fulfillment()
        })
        .try_fold(0_u64, |total, receipt| total.checked_add(receipt.quantity));
    let fulfilled = if disposition.counts_as_fulfillment() {
        fulfilled.and_then(|total| total.checked_add(evidenced_quantity))
    } else {
        fulfilled
    };
    if evidenced_quantity > request.quantity
        || fulfilled.is_none_or(|total| total > request.quantity)
    {
        return Err(Refused::new(
            FISCAL_RECEIPT_QUANTITY_EXCEEDED,
            "fiscal execution evidence exceeds its authorized quantity",
        ));
    }
    Ok(ValidatedExecutionEvidence {
        quantity: evidenced_quantity,
        disposition,
        external_operations,
    })
}

/// The adoptions of a state by `scope/adoption`.
fn adoption_keys(state: &FiscalState) -> BTreeMap<String, &FiscalAdoptionState> {
    state
        .ledgers
        .values()
        .flat_map(|ledger| {
            ledger
                .adoptions
                .values()
                .map(move |adoption| (format!("{}/{}", ledger.scope.id, adoption.id), adoption))
        })
        .collect()
}

/// The observer bindings of a state without their rings, which only
/// outcomes change.
fn observer_keys(state: &FiscalState) -> BTreeMap<String, FiscalObserverBinding> {
    state
        .observer_bindings
        .iter()
        .map(|(id, binding)| {
            let mut binding = binding.clone();
            binding.outcomes = crate::model::FiscalOutcomeRing::default();
            (id.clone(), binding)
        })
        .collect()
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
    let adoptions = changed_keys(&adoption_keys(state), &adoption_keys(&next));
    let authority_bindings = changed_keys(&state.authority_bindings, &next.authority_bindings);
    let audit_bindings = changed_keys(&state.audit_bindings, &next.audit_bindings);
    let observer_bindings = changed_keys(&observer_keys(state), &observer_keys(&next));
    if !adoptions.is_disjoint(&overwrites.adoptions)
        || !authority_bindings.is_disjoint(&overwrites.authority_bindings)
        || !audit_bindings.is_disjoint(&overwrites.audit_bindings)
        || !observer_bindings.is_disjoint(&overwrites.observer_bindings)
    {
        return Err(stale(STALE_OVERWRITE));
    }
    if let Some((measured, budget)) = measured {
        let mut settled = next.clone();
        record_outcome(
            &mut settled,
            &admitted.target,
            &admitted.request.action_id,
            FiscalActionDisposition::Applied,
        )?;
        let bytes = measure_writes(&settled, catalog, at)?;
        if bytes > budget {
            return Err(invalid(FISCAL_STATE_BUDGET_EXHAUSTED));
        }
        *measured = bytes;
    }
    overwrites.adoptions.extend(adoptions);
    overwrites.authority_bindings.extend(authority_bindings);
    overwrites.audit_bindings.extend(audit_bindings);
    overwrites.observer_bindings.extend(observer_bindings);
    *state = next;
    Ok(())
}

/// Keys whose entry was added, replaced, or removed between two maps.
fn changed_keys<T: PartialEq>(
    before: &BTreeMap<String, T>,
    after: &BTreeMap<String, T>,
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

/// Requires `id` to be free among the scope's hot objects of one kind.
fn reject_existing<'a, T: 'a>(
    mut maps: impl Iterator<Item = &'a BTreeMap<String, T>>,
    id: &str,
    label: &str,
) -> Result<(), CanwuError> {
    if maps.any(|values| values.contains_key(id)) {
        return Err(CanwuError::new(
            ErrorCode::IdempotencyConflict,
            format!("fiscal {label} identity already exists"),
        ));
    }
    Ok(())
}

/// The open book of a scope that holds `assessment_id`, by its cycle.
fn open_book_of_assessment(
    ledger: &FiscalScopeLedger,
    assessment_id: &str,
    label: &str,
) -> Result<String, CanwuError> {
    let book = ledger
        .book_of_assessment(assessment_id)
        .ok_or_else(|| invalid(format!("{label} names an unknown assessment")))?;
    if !book.is_open() {
        return Err(invalid(FISCAL_CYCLE_CLOSED));
    }
    Ok(book.accounting_cycle_id.clone())
}

fn book_mut<'a>(
    state: &'a mut FiscalState,
    scope: &str,
    cycle: &str,
) -> Result<&'a mut FiscalCycleBook, CanwuError> {
    state
        .ledger_mut(scope)?
        .books
        .get_mut(cycle)
        .ok_or_else(|| invalid("fiscal accounting cycle book is unavailable"))
}

#[allow(clippy::too_many_lines)]
fn apply_action(
    state: &mut FiscalState,
    catalog: &CompiledFiscalCatalog,
    request: &FiscalActionRequest,
    issuer: &FiscalActionIssuer,
    at: SimTime,
) -> Result<(), CanwuError> {
    let source = Some(request.action_id.clone());
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
            let year = state.historical_context.year;
            let ledger = state
                .ledger(scope_binding_id)
                .ok_or_else(|| invalid("fiscal adoption names an unknown scope"))?;
            let scope = &ledger.scope;
            if rule.mechanism != scope.mechanism
                || !rule.jurisdiction_ids.contains(&scope.jurisdiction_id)
            {
                return Err(invalid(
                    "fiscal adoption rule is incompatible with its scope",
                ));
            }
            let generation = if let Some(existing) = ledger.adoptions.get(adoption_id) {
                if existing.rule_id != *rule_id {
                    return Err(invalid(
                        "fiscal adoption identity cannot change its rule or scope",
                    ));
                }
                existing
                    .generation
                    .checked_add(1)
                    .ok_or_else(|| invalid("fiscal adoption generation overflowed"))?
            } else {
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
                if !rule.legal_window.contains(year) {
                    return Err(invalid("fiscal adoption is outside its legal window"));
                }
                1
            };
            state.ledger_mut(scope_binding_id)?.adoptions.insert(
                adoption_id.clone(),
                FiscalAdoptionState {
                    id: adoption_id.clone(),
                    rule_id: rule_id.clone(),
                    scope_binding_id: scope_binding_id.clone(),
                    stage: *stage,
                    generation,
                    changed_at: at,
                    source_action_id: source,
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
            let ledger = state
                .ledger(scope_binding_id)
                .ok_or_else(|| invalid("fiscal assessment names an unknown scope"))?;
            reject_existing(
                ledger.books.values().map(|book| &book.assessments),
                assessment_id,
                "assessment",
            )?;
            let institution = ledger.scope.institution.clone();
            match ledger.books.get(accounting_cycle_id) {
                Some(book) if !book.is_open() => return Err(invalid(FISCAL_CYCLE_CLOSED)),
                Some(_) => {}
                None => {
                    if ledger.books.len() >= MAX_FISCAL_HOT_BOOKS_PER_LEDGER {
                        return Err(CanwuError::new(
                            ErrorCode::ValueOutOfRange,
                            FISCAL_HOT_BOOKS_EXHAUSTED,
                        ));
                    }
                    let office_watermark = state
                        .office(&institution)
                        .and_then(|office| office.watermark.as_ref());
                    if ledger
                        .watermark
                        .iter()
                        .chain(office_watermark)
                        .any(|watermark| accounting_cycle_id <= watermark)
                    {
                        return Err(invalid(FISCAL_CYCLE_ARCHIVED));
                    }
                    state.ledger_mut(scope_binding_id)?.books.insert(
                        accounting_cycle_id.clone(),
                        FiscalCycleBook::open(accounting_cycle_id.clone()),
                    );
                    let entry = state
                        .office_mut(&institution)
                        .cycles
                        .entry(accounting_cycle_id.clone())
                        .or_default();
                    entry.hot_books = entry
                        .hot_books
                        .checked_add(1)
                        .ok_or_else(|| invalid("fiscal office book count overflowed"))?;
                }
            }
            book_mut(state, scope_binding_id, accounting_cycle_id)?
                .assessments
                .insert(
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
                        source_action_id: source,
                    },
                );
        }
        FiscalAction::GrantRemission {
            remission_id,
            scope_binding_id,
            assessment_id,
            quantity,
            reason,
        } => {
            let ledger = state
                .ledger(scope_binding_id)
                .ok_or_else(|| invalid("fiscal remission names an unknown scope"))?;
            reject_existing(
                ledger.books.values().map(|book| &book.remissions),
                remission_id,
                "remission",
            )?;
            let cycle = open_book_of_assessment(ledger, assessment_id, "fiscal remission")?;
            book_mut(state, scope_binding_id, &cycle)?
                .remissions
                .insert(
                    remission_id.clone(),
                    FiscalRemission {
                        id: remission_id.clone(),
                        assessment_id: assessment_id.clone(),
                        quantity: *quantity,
                        reason: reason.clone(),
                        granted_at: at,
                        source_action_id: source,
                    },
                );
        }
        FiscalAction::AuthorizeExecution {
            request_id,
            scope_binding_id,
            assessment_id,
            kind,
            quantity,
            unit,
            resource,
            source: execution_source,
            target,
            purpose,
        } => {
            let ledger = state
                .ledger(scope_binding_id)
                .ok_or_else(|| invalid("execution authorization names an unknown scope"))?;
            reject_existing(
                ledger.books.values().map(|book| &book.execution_requests),
                request_id,
                "execution request",
            )?;
            let cycle = open_book_of_assessment(ledger, assessment_id, "execution authorization")?;
            let payment_form = ledger.books[&cycle].assessments[assessment_id].payment_form;
            let institution = ledger.scope.institution.clone();
            book_mut(state, scope_binding_id, &cycle)?
                .execution_requests
                .insert(
                    request_id.clone(),
                    FiscalExecutionRequest {
                        id: request_id.clone(),
                        assessment_id: assessment_id.clone(),
                        institution,
                        kind: *kind,
                        quantity: *quantity,
                        unit: unit.clone(),
                        payment_form,
                        resource: *resource,
                        source: execution_source.clone(),
                        target: target.clone(),
                        purpose: purpose.clone(),
                        requested_at: at,
                        source_action_id: source,
                    },
                );
        }
        FiscalAction::RecordAudit {
            audit_id,
            scope_binding_id,
            target_id,
            severity,
            finding,
            evidence,
        } => {
            let ledger = state
                .ledger(scope_binding_id)
                .ok_or_else(|| invalid("fiscal audit names an unknown scope"))?;
            reject_existing(
                ledger.books.values().map(|book| &book.audits),
                audit_id,
                "audit",
            )?;
            let cycle = ledger
                .book_of_audit_target(target_id)
                .map(|book| book.accounting_cycle_id.clone())
                .ok_or_else(|| invalid("fiscal audit target is unavailable"))?;
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
            book_mut(state, scope_binding_id, &cycle)?.audits.insert(
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
                    source_action_id: source,
                },
            );
        }
        FiscalAction::CloseAccountingCycle {
            scope_binding_id,
            accounting_cycle_id,
        } => {
            let book = book_mut(state, scope_binding_id, accounting_cycle_id)?;
            if !book.is_open() {
                return Err(invalid(FISCAL_CYCLE_CLOSED));
            }
            book.status = FiscalBookStatus::Closed {
                closed_by: rebinding(request, issuer, at),
            };
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
        FiscalAction::RebindObserver {
            observer_binding_id,
            actor,
            knowledge_holder,
        } => {
            if !holder_matches(*actor, knowledge_holder) {
                return Err(invalid_payload(
                    "a fiscal observer holder is its actor's person or, without an actor, a government",
                ));
            }
            let observer = active_observer(state, observer_binding_id)?;
            let previous =
                std::mem::replace(&mut observer.knowledge_holder, knowledge_holder.clone());
            observer.actor = *actor;
            observer.rebound_by = Some(rebinding(request, issuer, at));
            if previous != *knowledge_holder {
                // The old holder gets a withdrawn summary, and the new one
                // the whole report, which it holds none of yet.
                let observer = observer.clone();
                let parts = observer_part_count(&observer, &state.aggregates);
                let withdrawals = state
                    .pending_withdrawals
                    .entry(observer_binding_id.clone())
                    .or_default();
                withdrawals.remove(knowledge_holder);
                withdrawals.insert(previous);
                state
                    .report_plan
                    .deferred
                    .insert(observer_binding_id.clone(), (0..parts).collect());
            }
        }
        FiscalAction::RemoveObserver {
            observer_binding_id,
        } => {
            let observer = active_observer(state, observer_binding_id)?;
            observer.removed = Some(rebinding(request, issuer, at));
            let holder = observer.knowledge_holder.clone();
            state
                .pending_withdrawals
                .entry(observer_binding_id.clone())
                .or_default()
                .insert(holder);
            state.report_plan.publish.remove(observer_binding_id);
            state.report_plan.deferred.remove(observer_binding_id);
        }
    }
    Ok(())
}

fn active_observer<'a>(
    state: &'a mut FiscalState,
    observer_binding_id: &str,
) -> Result<&'a mut FiscalObserverBinding, CanwuError> {
    state
        .observer_bindings
        .get_mut(observer_binding_id)
        .ok_or_else(|| invalid("fiscal observer binding is unavailable"))
        .and_then(|observer| {
            if observer.is_active() {
                Ok(observer)
            } else {
                Err(invalid(FISCAL_OBSERVER_REMOVED))
            }
        })
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
    target_scope_bindings: &BTreeMap<String, String>,
    action_id: &str,
    at: SimTime,
) -> Result<(), CanwuError> {
    let transition = catalog
        .transitions
        .get(transition_id)
        .ok_or_else(|| invalid("fiscal transition is unavailable"))?;
    if target_scope_bindings.keys().collect::<BTreeSet<_>>()
        != transition.to_rule_ids.iter().collect::<BTreeSet<_>>()
    {
        return Err(invalid(
            "fiscal transition must bind every target rule exactly once",
        ));
    }
    let mut target_jurisdiction = None;
    for (rule_id, scope_id) in target_scope_bindings {
        let rule = &catalog.rules[rule_id];
        let scope = state
            .scope_binding(scope_id)
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
    for ledger in state
        .ledgers
        .values_mut()
        .filter(|ledger| ledger.scope.jurisdiction_id == candidate.jurisdiction_id)
    {
        for adoption in ledger.adoptions.values_mut().filter(|adoption| {
            transition
                .supersedes_or_suspends
                .contains(&adoption.rule_id)
        }) {
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
        let ledger = state.ledger_mut(scope_id)?;
        let existing_id = ledger
            .adoptions
            .values()
            .find_map(|adoption| (adoption.rule_id == *rule_id).then(|| adoption.id.clone()));
        let adoption_id = existing_id
            .unwrap_or_else(|| format!("adopt.transition.{transition_id}.{scope_id}.{rule_id}"));
        let generation = ledger
            .adoptions
            .get(&adoption_id)
            .map_or(Ok(1), |adoption| {
                adoption
                    .generation
                    .checked_add(1)
                    .ok_or_else(|| invalid("fiscal adoption generation overflowed"))
            })?;
        ledger.adoptions.insert(
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
/// The withdrawals settlement left in the state move into the plan, so a
/// state a boundary has finished holds none. A plan that leaves parts owed
/// to an observer who can receive knowledge, because they did not fit the
/// boundary's limits, comes with an internal [`FISCAL_REPORT_DRAIN_INGRESS`]
/// packet due at once, so that the next boundary publishes them.
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
        let withdrawing = !state.pending_withdrawals.is_empty();
        // The report plan follows a change of the state. A state that no
        // boundary has written, with aggregates that are already right, is
        // left as it is, and its first report comes with its first change.
        let plan = if written_now || aggregates_changed || withdrawing {
            let unreachable = unreachable_observers(view, state)?;
            next_report_plan(state, &aggregates, &unreachable)
                .inspect(|plan| drain = owes_reachable_parts(plan, &unreachable))
        } else {
            None
        };
        if !aggregates_changed && plan.is_none() && !withdrawing {
            return Ok(None);
        }
        state.aggregates = aggregates;
        state.pending_withdrawals.clear();
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
/// Parts are owed when a boundary had more report records or publications
/// than it may publish, and that boundary scheduled a
/// [`FISCAL_REPORT_DRAIN_INGRESS`] packet so that the next boundary comes at
/// once; or when an observer's person was dead, until a boundary finds him
/// alive again. Either way they are found here, in any boundary that runs
/// the fiscal systems, without waiting for a change of the fiscal state.
///
/// The test reads the plan's owed observers and their holders from the
/// committed record's payload, without decoding the state, so a boundary in
/// which nothing is owed to an observer who can receive it costs what it did
/// before. Otherwise the state is decoded and a plan of the owed parts is
/// written, with a drain packet when some are still owed; the report system
/// then publishes it. Nothing else in the state changes, so the procedure
/// revision does not advance and no action becomes stale.
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
/// summary, to an observer whose holder can receive knowledge now. It reads
/// the plan's owed observer IDs and those observers' holders from the
/// payload and decodes nothing else; there are at most
/// [`MAX_FISCAL_OBSERVERS`].
fn owes_reachable_observer(view: &SimulationView<'_>, payload: &Value) -> Result<bool, CanwuError> {
    let Some(owed) = payload
        .get("report_plan")
        .and_then(|plan| plan.get("deferred"))
        .and_then(Value::as_object)
    else {
        return Ok(false);
    };
    for observer_id in owed.keys() {
        let holder = payload
            .get("observer_bindings")
            .and_then(|bindings| bindings.get(observer_id))
            .and_then(|binding| binding.get("knowledge_holder"))
            .and_then(|holder| serde_json::from_value::<KnowledgeHolderRef>(holder.clone()).ok())
            .ok_or_else(|| invalid("the fiscal report plan owes parts to an unknown observer"))?;
        if !holder_is_unreachable(view, &holder)? {
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

/// Whether the engine would refuse to publish knowledge to this holder.
///
/// It refuses a holder who does not exist or is dead. The state record
/// references every observer's holder, so the holder exists; what is left is
/// a person's life state. A government holder is always reachable. The
/// aggregate system and the report system both run after the last phase that
/// can change a life state, so they read the same answer the engine checks a
/// publication against.
fn holder_is_unreachable(
    view: &SimulationView<'_>,
    holder: &KnowledgeHolderRef,
) -> Result<bool, CanwuError> {
    match holder {
        KnowledgeHolderRef::Person(person) => Ok(view
            .person_availability(*person)?
            .is_some_and(|availability| availability.life == LifeState::Dead)),
        KnowledgeHolderRef::Entity(_) => Ok(false),
    }
}

/// The observer bindings no report can be published to now.
fn unreachable_observers(
    view: &SimulationView<'_>,
    state: &FiscalState,
) -> Result<BTreeSet<String>, CanwuError> {
    let mut unreachable = BTreeSet::new();
    for observer in state.observer_bindings.values() {
        if !observer.is_active() || holder_is_unreachable(view, &observer.knowledge_holder)? {
            unreachable.insert(observer.id.clone());
        }
    }
    Ok(unreachable)
}

/// The subjects of the records on one report line of one observer binding.
///
/// A holder can hold many lines: one for each summary and each part of every
/// observer binding it holds. `SimulationView::knowledge_heads` selects
/// records that carry every subject it is given, so each line needs a
/// subject no other line has. A subject is a role and a target, the target
/// has to exist, and the one record every report is about is the fiscal
/// state. The line is therefore told by the roles: the four hexadecimal
/// digits of its number, each a role of its own place, all on the fiscal
/// state record. A binding's lines are numbered from its line slot, which no
/// other binding has, and two lines differ in at least one place, so neither
/// line's subjects contain the other's.
fn report_line_subjects(slot: u16, index: u32) -> Vec<KnowledgeSubject> {
    let line = u32::from(slot) * REPORT_LINES_PER_OBSERVER + index;
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
/// record its line currently holds, and the withdrawn summaries it lists.
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
    let withdrawals = planned_withdrawals(&state, context.at);
    if reports.is_empty() && withdrawals.is_empty() {
        return Ok(BoundaryProposal::default());
    }
    // The version the facts were computed from: the last one this boundary
    // proposed.
    let Some(source_version) = view.current_domain_record_version(&state_reference)? else {
        return Ok(BoundaryProposal::default());
    };
    let kind = fiscal_report_knowledge_schema_id().kind;
    let mut directives = Vec::with_capacity(reports.len() + withdrawals.len());
    let mut withdrawn = BTreeMap::<&str, usize>::new();
    for (observer, holder, summary) in withdrawals {
        // Each withdrawal of a binding in one boundary goes to a holder of
        // its own, and is numbered among them.
        let index = withdrawn.entry(observer.id.as_str()).or_default();
        *index += 1;
        // A holder that can no longer receive knowledge keeps what it holds.
        if holder_is_unreachable(view, holder)? {
            continue;
        }
        let mut publication = ReportPublication {
            view,
            holder,
            kind: &kind,
            slot: observer.line_slot,
            as_of: context.at,
            confidence_per_mille: observer.confidence_per_mille,
            source_version: &source_version,
            records: Vec::with_capacity(1),
        };
        publication.push(0, &FiscalReportRecord::Summary(summary))?;
        directives.push(publication.directive(
            format!(
                "fiscal-report-withdrawal::{}::{index}::{}",
                observer.id, context.boundary_id
            ),
            "Withdraw one fiscal report from a holder",
        ));
    }
    for report in reports {
        // The plan lists no observer the engine would refuse, because the
        // aggregate system asked the same question of the same state. Asking
        // again keeps a refused publication from ever failing the boundary.
        if holder_is_unreachable(view, &report.observer.knowledge_holder)? {
            continue;
        }
        let mut publication = ReportPublication {
            view,
            holder: &report.observer.knowledge_holder,
            kind: &kind,
            slot: report.observer.line_slot,
            as_of: context.at,
            confidence_per_mille: report.observer.confidence_per_mille,
            source_version: &source_version,
            records: Vec::with_capacity(report.parts.len() + 1),
        };
        publication.push(0, &FiscalReportRecord::Summary(report.summary))?;
        for part in report.parts {
            publication.push(part.part.saturating_add(1), &FiscalReportRecord::Part(part))?;
        }
        directives.push(publication.directive(
            format!(
                "fiscal-report::{}::{}",
                report.observer.id, context.boundary_id
            ),
            "Publish the changed parts of one holder-relative fiscal report",
        ));
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

/// The records one boundary publishes to one holder on one binding's lines.
struct ReportPublication<'a, 'view> {
    view: &'a SimulationView<'view>,
    holder: &'a KnowledgeHolderRef,
    kind: &'a KnowledgeRecordKind,
    slot: u16,
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
                method: "fiscal_authority_report_v3".to_owned(),
                evidence: vec![EvidenceRef::DomainRecordVersion(
                    self.source_version.clone(),
                )],
            },
            supersedes,
            contradicts: Vec::new(),
        });
        Ok(())
    }

    /// The publication of the records, under `correlation`, which no other
    /// publication of the boundary has.
    fn directive(self, correlation: String, summary: &str) -> BoundaryDirective {
        BoundaryDirective::PublishKnowledge {
            holder: self.holder.clone(),
            visibility: StateVisibility::SameBoundary,
            producer_correlation: Some(correlation),
            records: self.records,
            summary: summary.to_owned(),
        }
    }
}

/// Reads the fiscal reports the viewer's holder currently holds, keyed by
/// observer binding ID, and puts each back together from its records.
///
/// A holder has one report for each observer binding it holds. Each is what
/// the binding lets the holder see and nothing else; see
/// [`assemble_fiscal_reports`] for how the records are assembled. A report
/// withdrawn from the holder is left out.
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

/// Reads the fiscal reports one holder currently holds, as a trusted host:
/// [`fiscal_reports`] for any holder, a government included.
pub fn fiscal_reports_of_holder(
    canwu: &Canwu,
    holder: &KnowledgeHolderRef,
) -> Result<BTreeMap<String, FiscalReport>, CanwuError> {
    let mut query = KnowledgeQuery {
        schemas: vec![fiscal_report_knowledge_schema_id()],
        limit: MAX_KNOWLEDGE_PAGE_SIZE,
        ..KnowledgeQuery::default()
    };
    let mut payloads = Vec::new();
    loop {
        let page = canwu.admin_query_knowledge(holder.clone(), &query)?;
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

/// Queues a fiscal execution receipt that cites exact, current external
/// evidence versions. Takes either runtime handle, [`canwu_api::Canwu`] or [`canwu_api::CompactedCanwu`].
pub fn enqueue_execution_receipt<H: canwu_api::CanwuRun + ?Sized>(
    canwu: &mut H,
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
    // rather than have the settling boundary refuse it.
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

/// A run a host archives fiscal books from: a full [`Canwu`] or a
/// [`CompactedCanwu`]. The fiscal archive functions read its fiscal state
/// and queue their internal packets on it.
pub trait FiscalArchiveRuntime {
    /// The committed fiscal state.
    fn fiscal_state(&self) -> Result<FiscalState, CanwuError>;
    /// The run's simulated time.
    fn fiscal_archive_time(&self) -> SimTime;
    /// Queues one internal fiscal packet.
    fn enqueue_fiscal_archive_ingress(
        &mut self,
        request: PluginIngressRequest,
        permit: &PluginIngressPermit,
    ) -> Result<IngressReceipt, CanwuError>;
}

fn decode_fiscal_state(record: Option<&DomainRecord>) -> Result<FiscalState, CanwuError> {
    record
        .ok_or_else(|| missing("fiscal state is not configured"))?
        .decode_payload::<FiscalStateRecord>()
}

impl FiscalArchiveRuntime for Canwu {
    fn fiscal_state(&self) -> Result<FiscalState, CanwuError> {
        decode_fiscal_state(self.typed_domain_record(&fiscal_state_reference()))
    }

    fn fiscal_archive_time(&self) -> SimTime {
        self.time()
    }

    fn enqueue_fiscal_archive_ingress(
        &mut self,
        request: PluginIngressRequest,
        permit: &PluginIngressPermit,
    ) -> Result<IngressReceipt, CanwuError> {
        self.enqueue_permitted_plugin_ingress(request, permit)
    }
}

impl FiscalArchiveRuntime for CompactedCanwu {
    fn fiscal_state(&self) -> Result<FiscalState, CanwuError> {
        decode_fiscal_state(self.typed_domain_record(&fiscal_state_reference()))
    }

    fn fiscal_archive_time(&self) -> SimTime {
        self.time()
    }

    fn enqueue_fiscal_archive_ingress(
        &mut self,
        request: PluginIngressRequest,
        permit: &PluginIngressPermit,
    ) -> Result<IngressReceipt, CanwuError> {
        self.enqueue_permitted_plugin_ingress(request, permit)
    }
}

/// What [`enqueue_fiscal_archive`] queued: the ingress, and the retention
/// handle and directory of the commit, which
/// [`finalize_fiscal_archive_retention`] takes once the commit settled.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FiscalArchiveIngressReceiptV1 {
    pub ingress: IngressReceipt,
    pub retention_handle_id: String,
    pub directory_root: String,
}

/// Stores, reads back, verifies, and queues one fiscal archive commit, due
/// now.
///
/// The batch must equal what [`FiscalState::prepare_fiscal_archive`] derives
/// from the run's current fiscal state, `store`, and `sealed_cut` for the
/// same number of books, lookup pages included: a batch prepared before a
/// boundary that changed the state is refused here. The boundary that
/// settles the commit reads no archive object: it applies the commit when
/// the state is still the one the batch was prepared from, and otherwise
/// records it as stale. Prepare and enqueue in one gap between boundaries.
pub fn enqueue_fiscal_archive<R: FiscalArchiveRuntime + ?Sized>(
    runtime: &mut R,
    prepared: &PreparedFiscalArchiveBatchV1,
    store: &dyn FiscalArchiveStore,
    sealed_cut: Option<&EvidenceCursor>,
) -> Result<FiscalArchiveIngressReceiptV1, FiscalArchiveError> {
    let state = runtime.fiscal_state()?;
    let canonical =
        state.prepare_fiscal_archive(prepared.selected_books.len().max(1), store, sealed_cut)?;
    if canonical != *prepared {
        return Err(FiscalArchiveError::Mismatch(
            "fiscal archive batch differs from the exact current archive candidates".to_owned(),
        ));
    }
    let mut commit = prepared.store_and_verify(store)?;
    let permit = ARCHIVE_COMMIT_PERMIT.get().ok_or_else(|| {
        CanwuError::new(
            ErrorCode::PluginNotActive,
            "the fiscal plugin must be registered before archive ingress",
        )
    })?;
    commit.retention.phase = FiscalArchiveRetentionPhaseV1::DurableIngress;
    commit.retention.semantic_digest.clear();
    commit.retention.semantic_digest =
        canonical_hash("canwu.fiscal.archive-retention.v1", &commit.retention)?;
    store.persist_fiscal_archive_retention(&commit.retention)?;
    let retention_handle_id = commit.retention.handle_id.clone();
    let directory_root = commit.directory_root.clone();
    let request = PluginIngressRequest::new(
        PLUGIN_NAME,
        FISCAL_ARCHIVE_COMMIT_INGRESS,
        runtime.fiscal_archive_time(),
        serde_json::to_value(FiscalArchiveCommitPacket { commit }).map_err(encode_error)?,
    )
    .with_archive_retention(vec![PluginArchiveRetention {
        namespace: FISCAL_ARCHIVE_INDEX_DIRECTORY_NAMESPACE.to_owned(),
        object_id: directory_root.clone(),
    }]);
    match runtime.enqueue_fiscal_archive_ingress(request, permit) {
        Ok(ingress) => Ok(FiscalArchiveIngressReceiptV1 {
            ingress,
            retention_handle_id,
            directory_root,
        }),
        Err(error) => {
            let _ = store.finalize_fiscal_archive_retention(
                &retention_handle_id,
                FiscalArchiveRetentionPhaseV1::Abandoned,
            );
            Err(error.into())
        }
    }
}

/// Finalizes the store-side retention of one settled commit and queues the
/// acknowledgement that retires its handle, and every stale receipt recorded
/// under it, from the state.
///
/// The handle must be in the state, settled. An acknowledgement that settles
/// after another one retired the handle is a no-op, so calling this twice
/// before the next boundary is harmless.
pub fn finalize_fiscal_archive_retention<R: FiscalArchiveRuntime + ?Sized>(
    runtime: &mut R,
    store: &dyn FiscalArchiveStore,
    receipt: &FiscalArchiveIngressReceiptV1,
) -> Result<IngressReceipt, FiscalArchiveError> {
    let state = runtime.fiscal_state()?;
    let handle: &FiscalArchiveRetentionHandleV1 = state
        .archive
        .pending_handles
        .get(&receipt.retention_handle_id)
        .ok_or_else(|| {
            FiscalArchiveError::Mismatch(
                "fiscal archive retention handle has not settled or was retired".to_owned(),
            )
        })?;
    if handle.target_directory_root != receipt.directory_root {
        return Err(FiscalArchiveError::Mismatch(
            "fiscal archive retention handle differs from the enqueue receipt".to_owned(),
        ));
    }
    store.finalize_fiscal_archive_retention(&handle.handle_id, handle.phase)?;
    let permit = ARCHIVE_RETENTION_PERMIT.get().ok_or_else(|| {
        CanwuError::new(
            ErrorCode::PluginNotActive,
            "the fiscal plugin must be registered before archive ingress",
        )
    })?;
    Ok(runtime.enqueue_fiscal_archive_ingress(
        PluginIngressRequest::new(
            PLUGIN_NAME,
            FISCAL_ARCHIVE_RETENTION_INGRESS,
            runtime.fiscal_archive_time(),
            serde_json::to_value(FiscalArchiveRetentionPacket {
                handle_id: handle.handle_id.clone(),
            })
            .map_err(encode_error)?,
        ),
        permit,
    )?)
}

/// The plugin archive provider seen as a fiscal archive store that only
/// reads.
struct PluginFiscalArchiveProvider<'a>(&'a dyn PluginArchiveObjectProvider);

impl FiscalArchiveStore for PluginFiscalArchiveProvider<'_> {
    fn store_fiscal_archive_object(
        &self,
        _namespace: &str,
        _object_id: &str,
        _bytes: &[u8],
    ) -> Result<(), FiscalArchiveError> {
        Err(FiscalArchiveError::Store(
            "the fiscal reachability provider is read-only".to_owned(),
        ))
    }

    fn load_fiscal_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, FiscalArchiveError> {
        Ok(self.0.load_plugin_archive_object(namespace, object_id)?)
    }

    fn persist_fiscal_archive_retention(
        &self,
        _handle: &FiscalArchiveRetentionHandleV1,
    ) -> Result<(), FiscalArchiveError> {
        Err(FiscalArchiveError::Store(
            "the fiscal reachability provider keeps no retention".to_owned(),
        ))
    }

    fn finalize_fiscal_archive_retention(
        &self,
        _handle_id: &str,
        _phase: FiscalArchiveRetentionPhaseV1,
    ) -> Result<(), FiscalArchiveError> {
        Err(FiscalArchiveError::Store(
            "the fiscal reachability provider keeps no retention".to_owned(),
        ))
    }
}

/// Marks every fiscal archive object a restore, a replay, a fork, or a
/// queued commit can still read: the commits reachable from the archive
/// head, from each unacknowledged retention handle, and from each commit the
/// kernel lists as queued, with their blobs, pages, and lookup pages.
fn fiscal_archive_reachability(
    view: &SimulationView<'_>,
    provider: &dyn PluginArchiveObjectProvider,
    manifest: &mut ArchiveReachabilityManifest,
) -> Result<(), CanwuError> {
    let provider = PluginFiscalArchiveProvider(provider);
    // Queued commits: the kernel marks the directory each queued archive
    // commit retains. Each may become the head, so its index must be whole.
    let queued = manifest
        .plugin_objects
        .get(FISCAL_ARCHIVE_INDEX_DIRECTORY_NAMESPACE)
        .cloned()
        .unwrap_or_default();
    let mut roots = queued.clone();
    let mut complete = queued;
    let mut bound = roots.len();
    if let Some((_, state)) = committed_fiscal_state(view)? {
        if let Some(root) = &state.archive.directory_root {
            roots.insert(root.clone());
            complete.insert(root.clone());
        }
        roots.extend(
            state
                .archive
                .pending_handles
                .values()
                .map(|handle| handle.target_directory_root.clone()),
        );
        bound = usize::try_from(state.archive.committed_batch_count)
            .unwrap_or(usize::MAX)
            .saturating_add(state.archive.pending_handles.len())
            .saturating_add(bound);
    }
    if roots.is_empty() {
        return Ok(());
    }
    let objects = reachable_fiscal_archive_objects(&provider, roots, &complete, bound)?;
    for (namespace, ids) in objects {
        for id in ids {
            manifest.insert_plugin_object(namespace, id);
        }
    }
    Ok(())
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
    use canwu_api::Canwu;

    thread_local! {
        /// The budget `settlement_budget` returns on this thread, so a test
        /// can reach the state's byte budget with a small state.
        pub(super) static SETTLEMENT_BUDGET: std::cell::Cell<Option<usize>> =
            const { std::cell::Cell::new(None) };
    }

    use crate::content::compile_fiscal_content;
    use crate::derive::recompute_derived;
    use crate::model::{
        FISCAL_RECEIPT_EVIDENCE_REUSED, FISCAL_RECENT_OUTCOMES, FiscalAssessmentBasis,
        FiscalAuditSeverity, FiscalCommutationPolicy, FiscalContentPack, FiscalContentSelection,
        FiscalCoverageDeclaration, FiscalCoverageSelector, FiscalCoverageStatus,
        FiscalExecutionKind, FiscalHistoricalMode, FiscalMechanism, FiscalOutcomeRing,
        FiscalPackManifest, FiscalPaymentForm, FiscalPeriodDefinition, FiscalProvenance,
        FiscalRegionDefinition, FiscalRuleDefinition, FiscalScopeBinding,
        FiscalTransitionDefinition, HistoricalConfidence, HistoricalYearWindow,
    };
    use canwu_api::{
        CommandEnvelope, CommandRequest, CommandRequestId, DomainRecordClass, DomainRecordDraft,
        DomainRecordLifecycle, DomainValueKindClass, GovernmentId, KnowledgeSnapshot,
        OrganizationId, ResourceId, Scenario, SimDuration, SimulationGranularity,
        TypedDomainRecordRef, WorldSnapshot,
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
            state.add_scope(FiscalScopeBinding {
                id: scope.clone(),
                institution: institution(),
                jurisdiction_id: "region.a".to_owned(),
                subject_scope: "registered_land".to_owned(),
                mechanism: FiscalMechanism::LandTax,
                authoritative_granularity: SimulationGranularity::Aggregate,
            });
            state
                .add_adoption(FiscalAdoptionState {
                    id: format!("adopt.{index}"),
                    rule_id: RULE.to_owned(),
                    scope_binding_id: scope,
                    stage: FiscalAdoptionStage::Implemented,
                    generation: 1,
                    changed_at: SimTime::EPOCH,
                    source_action_id: None,
                })
                .expect("adoption");
        }
        state
            .add_assessment(FiscalAssessment {
                id: AUTHORED.to_owned(),
                rule_id: RULE.to_owned(),
                scope_binding_id: scope_id(0),
                accounting_cycle_id: "cycle.authored".to_owned(),
                quantity: 500,
                unit: "grain".to_owned(),
                payment_form: FiscalPaymentForm::Grain,
                commutation_quote: None,
                created_at: SimTime::EPOCH,
                source_action_id: None,
            })
            .expect("authored assessment");
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
            entities: vec![
                EntityRef::Person(ISSUER),
                institution(),
                EntityRef::Government(GovernmentId::new(1)),
            ],
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
        let record = canwu_api::typed_domain_record(canwu, &fiscal_state_reference())
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

    /// Admits the actions against the procedure revision the run has less
    /// `behind`, and settles them, with settlement held to `budget`.
    fn submit(
        canwu: &mut Canwu,
        budget: Option<usize>,
        behind: u64,
        actions: impl IntoIterator<Item = (String, FiscalAction)>,
    ) {
        let expected_procedure_revision = committed(canwu).0.procedure_revision - behind;
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
                actor: Some(ISSUER),
                knowledge_holder: KnowledgeHolderRef::Person(ISSUER),
                visible_institutions: BTreeSet::from([institution()]),
                confidence_per_mille: 1_000,
                line_slot: 0,
                controlled_by: None,
                rebound_by: None,
                removed: None,
                outcomes: FiscalOutcomeRing::default(),
            },
        );
    }

    /// The settled outcome of `action_id`, as its event records it.
    fn settled(canwu: &Canwu, action_id: &str) -> Option<FiscalSettledAction> {
        FiscalSettledAction::from_events(canwu.events())
            .into_iter()
            .find(|settled| settled.action_id == action_id)
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
            let budget = committed(&canwu).1 + 2_200;
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
                            scope_binding_id: scope_id(0),
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
                    settled(&canwu, action_id).expect("settled").disposition,
                    FiscalActionDisposition::Applied
                );
            }
            let rejected = settled(&canwu, "does-not-fit").expect("settled");
            assert_eq!(rejected.disposition, FiscalActionDisposition::Rejected);
            assert!(rejected.reason.ends_with(FISCAL_STATE_BUDGET_EXHAUSTED));
            assert_eq!(
                state
                    .recent_outcome(&FiscalActionTarget::Ledger(scope_id(0)), "does-not-fit")
                    .map(|outcome| outcome.disposition),
                Some(FiscalActionDisposition::Rejected)
            );
            assert!(state.audits().next().is_none());
            assert_eq!(state.aggregates.len(), 3);
            assert!(bytes <= budget, "{bytes} bytes against {budget}");
        }
    }

    /// A rejected action's outcome is recorded when the state it leaves fits
    /// the byte budget exactly, and dropped when it is one byte over: with
    /// and without a report plan, which the budget leaves out, into an empty
    /// ring or a ring that holds an outcome, and when the procedure revision
    /// gains a digit.
    #[test]
    fn a_rejected_outcome_is_recorded_exactly_when_it_fits() {
        let filler = |state: &mut FiscalState| {
            let revision = state.procedure_revision;
            state
                .ledgers
                .get_mut(&scope_id(1))
                .expect("second scope")
                .outcomes
                .push(FiscalRecentOutcome {
                    action_id: "filler".to_owned(),
                    disposition: FiscalActionDisposition::Rejected,
                    settled_revision: revision - 1,
                });
        };
        for observed in [false, true] {
            for first in [false, true] {
                for revision in [2, 9, 99, 100] {
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
                        state
                            .recent_outcome(&FiscalActionTarget::Ledger(scope_id(1)), "stale")
                            .map(|outcome| outcome.disposition),
                        Some(FiscalActionDisposition::Rejected)
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
                            state
                                .recent_outcome(&FiscalActionTarget::Ledger(scope_id(1)), "stale")
                                .is_some(),
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

    /// A ring keeps the newest outcomes of its target; an outcome it evicts
    /// raises the floor to that outcome's settled revision.
    #[test]
    fn an_outcome_ring_keeps_the_newest_outcomes_and_raises_its_floor() {
        let mut ring = FiscalOutcomeRing::default();
        for revision in 1..=u64::try_from(FISCAL_RECENT_OUTCOMES + 3).expect("count") {
            ring.push(FiscalRecentOutcome {
                action_id: format!("action.{revision}"),
                disposition: FiscalActionDisposition::Applied,
                settled_revision: revision,
            });
        }
        assert_eq!(ring.recent.len(), FISCAL_RECENT_OUTCOMES);
        assert_eq!(ring.floor, 3);
        assert!(ring.get("action.3").is_none());
        assert!(ring.get("action.4").is_some());
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
                scope_binding_id: scope_id(0),
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
                state
                    .add_execution_request(
                        &scope_id(0),
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
                            source_action_id: None,
                        },
                    )
                    .expect("the collection request");
            },
        )
    }

    fn receipt(
        id: &str,
        results: &[(DomainRecord, DomainRecordVersionRef)],
    ) -> FiscalExecutionReceiptPacket {
        FiscalExecutionReceiptPacket {
            receipt_id: id.to_owned(),
            scope_binding_id: scope_id(0),
            request_id: REQUEST.to_owned(),
            external_evidence: results.iter().map(|(_, version)| version.clone()).collect(),
        }
    }

    fn hot_receipt(state: &FiscalState, id: &str) -> Option<FiscalExecutionReceipt> {
        state
            .execution_receipts()
            .find(|receipt| receipt.id == id)
            .cloned()
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
            assert!(hot_receipt(&state, "receipt.small").is_some());
            assert!(hot_receipt(&state, "receipt.large").is_none());
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
                hot_receipt(&committed(&canwu).0, "receipt.large")
                    .expect("the large receipt")
                    .quantity,
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
    /// nothing and is refused with an event that names it and ends with its
    /// stable reason; the valid packets of the same boundary settle, the
    /// boundary commits, later boundaries run, and restore and exact replay
    /// agree.
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
        assert!(hot_receipt(&state, "receipt.first").is_some());
        assert!(hot_receipt(&state, "receipt.second").is_none());
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
            FISCAL_CONTEXT_OUTSIDE_PACK
        ));
        assert!(refused("receipt.second)", FISCAL_RECEIPT_EVIDENCE_REUSED));

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
    /// revision, with a withdrawal for every action a boundary applies.
    #[test]
    fn the_largest_report_plan_fits_the_room_left_for_it() {
        let parts: BTreeSet<u32> =
            (0..u32::try_from(MAX_FISCAL_REPORT_PARTS).expect("parts")).collect();
        let id =
            |index: usize| format!("{index:02}{}", "o".repeat(MAX_FISCAL_IDENTIFIER_BYTES - 2));
        let observers: BTreeMap<String, BTreeSet<u32>> = (0..MAX_FISCAL_OBSERVERS)
            .map(|index| (id(index), parts.clone()))
            .collect();
        let withdraw: BTreeMap<String, BTreeSet<KnowledgeHolderRef>> = (0
            ..MAX_FISCAL_ACTIONS_PER_BOUNDARY)
            .map(|index| {
                (
                    id(index % MAX_FISCAL_OBSERVERS),
                    BTreeSet::from([KnowledgeHolderRef::Entity(EntityRef::Government(
                        GovernmentId::new(u64::MAX - u64::try_from(index).expect("index")),
                    ))]),
                )
            })
            .collect();
        let plan = FiscalReportPlan {
            revision: u64::MAX,
            publish: observers.clone(),
            deferred: observers,
            withdraw,
        };
        let bytes = plan_bytes(&plan).expect("plan bytes");
        assert!(
            bytes <= MAX_FISCAL_REPORT_PLAN_BYTES,
            "{bytes} bytes against {MAX_FISCAL_REPORT_PLAN_BYTES}"
        );
    }

    /// A settled event's summary reads back as the outcome it records.
    #[test]
    fn a_settled_event_summary_reads_back() {
        let settled = FiscalSettledAction {
            target: FiscalActionTarget::Ledger("scope.a".to_owned()).to_string(),
            action_id: "action.a".to_owned(),
            disposition: FiscalActionDisposition::Rejected,
            reason: "domain_record_version_conflict: stale: really".to_owned(),
        };
        assert_eq!(
            FiscalSettledAction::from_summary(&settled.summary()),
            Some(settled)
        );
        assert_eq!(
            FiscalSettledAction::from_summary("Settled something else"),
            None
        );
    }
}
