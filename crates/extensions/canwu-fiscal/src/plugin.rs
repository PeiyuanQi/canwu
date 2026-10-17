use crate::appointment::{
    admit_proposal_capacity, audit_binding, check_appointment, decided_institution,
    installed_basis, is_appointment_action, is_proposal,
};
use crate::archive::{
    FISCAL_ARCHIVE_INDEX_DIRECTORY_NAMESPACE, FiscalArchiveError, FiscalArchiveRetentionHandleV1,
    FiscalArchiveRetentionPhaseV1, FiscalArchiveStore, PreparedFiscalArchiveBatchV1,
    VerifiedFiscalArchiveCommitV1, reachable_fiscal_archive_objects,
};
use crate::model::{
    FISCAL_ACTING_BASIS_NOT_CURRENT, FISCAL_AUTHORITY_BASIS_NOT_CURRENT,
    FISCAL_INGRESS_MALFORMED_REJECTION, FISCAL_INGRESS_ORIGIN_REJECTION, FISCAL_OBSERVER_REMOVED,
    FISCAL_RECORD_RING_RESERVE_BYTES, FISCAL_REVISION_EXPIRED, FISCAL_REVISION_UNKNOWN,
    FiscalAction, FiscalActionBatch, FiscalActionEntry, FiscalActionIssuer, FiscalActionRequest,
    FiscalActionTarget, FiscalAppointmentDecision, FiscalAuthorityBinding, FiscalCatalogRecord,
    FiscalExecutionReceiptPacket, FiscalHistoricalContextPacket, FiscalIssuerRole,
    FiscalRecentOutcome, FiscalReportBudget, FiscalState, MAX_FISCAL_ACTION_JSON_BYTES,
    MAX_FISCAL_ACTION_TEXT_BYTES, MAX_FISCAL_BATCH_ENTRIES, MAX_FISCAL_BATCH_JSON_BYTES,
    MAX_FISCAL_EVIDENCE_PER_RECORD, MAX_FISCAL_IDENTIFIER_BYTES, MAX_FISCAL_LEDGER_RECORD_BYTES,
    MAX_FISCAL_RECEIPTS_PER_PACKET, MAX_FISCAL_RECORD_BYTES, MAX_FISCAL_REFERENCES_PER_DEFINITION,
    fiscal_catalog_reference, invalid, validate_identifier,
};
use crate::projection::committed_fiscal_catalog;
use crate::records::{
    FiscalArchiveRecord, FiscalJurisdictionRecord, FiscalLedgerRecord, FiscalObserverRecord,
    FiscalOfficeRecord, FiscalRecordKind, FiscalReportQueueRecord, FiscalRootRecord,
    OBSERVER_HOLDER_ROLE, ROOT_CATALOG_ROLE, audit_fiscal_record_set, fiscal_archive_reference,
};
use crate::report::{fiscal_report_knowledge_schema, materialize_reports};
use crate::settle::{
    AdmittedFiscalBatch, AdmittedFiscalEntry, CONTEXT_CHANGED_EVENT, FiscalExecutionReceiptBatch,
    FiscalPacket, RECEIPT_RECORDED_EVENT, json_len, named_entities, office_archive_reserve,
    settle_fiscal_packets,
};
use crate::working::{HostRecords, Working, active_record};
use canwu_api::{
    ArchiveReachabilityManifest, BoundaryContext, BoundaryPhase, BoundaryProposal,
    BoundarySystemContract, CanwuError, CanwuRun, CauseRef, Command, CommandContext,
    CommandIngress, CoreEntityKind, DecisionOrigin, DomainRecord, DomainRecordKind,
    DomainRecordMutationPolicy, DomainRecordSchema, DomainRecordType, DomainRecordVersionRef,
    DomainReferenceSchema, DomainReferenceTargetKind, EntityRef, ErrorCode, IngressClass,
    IngressId, IngressPayload, IngressReceipt, Issuer, KnowledgeHolderRef, KnowledgeRetention,
    KnowledgeWriteGrant, PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD, PayloadSchema, PersonId,
    PluginActionDescriptor, PluginArchiveObjectProvider, PluginArchiveRetention,
    PluginIngressDescriptor, PluginIngressPermit, PluginIngressRequest, PluginRegistrar, SimTime,
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
/// queues for the next boundary to settle: the admitted batch.
///
/// The type is kernel-internal: a host cannot enqueue it
/// (`enqueue_plugin_ingress` refuses it with `InvalidAuthority`), and the
/// command is the only path for an action. A packet is settled only when its
/// ingress record names, as its cause, the command the packet carries, which
/// only that command's admission produces; any other, which only a boundary
/// system of another plugin that declares this target could schedule, is
/// skipped with a [`FISCAL_INGRESS_REJECTED_EVENT`].
pub const FISCAL_ACTION_INGRESS: &str = "fiscal_action_v1";
/// A batch of up to [`MAX_FISCAL_RECEIPTS_PER_PACKET`] execution receipts,
/// queued by [`enqueue_execution_receipts`] and settled in order.
pub const FISCAL_EXECUTION_RECEIPTS_INGRESS: &str = "fiscal_execution_receipts_v1";
pub const FISCAL_HISTORICAL_CONTEXT_INGRESS: &str = "fiscal_historical_context_v1";
/// The internal packet a boundary schedules, due at once, when it leaves
/// report work queued for an observer who can receive knowledge, so that the
/// next boundary publishes it without waiting for a change of the fiscal
/// records. It carries nothing: the boundary that admits it finds the work
/// in the report queue.
pub const FISCAL_REPORT_DRAIN_INGRESS: &str = "fiscal_report_drain_v1";
/// The internal packet [`enqueue_fiscal_archive`] queues: one stored and
/// verified archive commit. Commits settle before every other fiscal packet
/// of their boundary.
pub const FISCAL_ARCHIVE_COMMIT_INGRESS: &str = "fiscal_archive_commit_v1";
/// The internal packet [`finalize_fiscal_archive_retention`] queues: the
/// acknowledgement that retires one settled commit's retention handle. It
/// settles after the commits and before every other fiscal packet.
pub const FISCAL_ARCHIVE_RETENTION_INGRESS: &str = "fiscal_archive_retention_v1";
/// Event that is the only record of a fiscal packet, or of one receipt of a
/// receipt batch, that settlement skips without changing anything: an
/// execution receipt or historical-context change that is refused, an
/// archive packet that does not decode or bind, or an action packet that is
/// malformed or was not produced by the admission of the command it names.
/// No fiscal packet fails its boundary. The summary names the ingress and
/// the packet (for a receipt, its receipt ID) and ends with the stable
/// reason: for a receipt one of [`crate::FISCAL_RECEIPT_REQUEST_UNKNOWN`] and
/// the receipt reasons after it, or
/// [`crate::FISCAL_RECEIPT_BOUNDARY_LIMIT`]; for a context change
/// [`crate::FISCAL_CONTEXT_OUTSIDE_PACK`]; and for any packet
/// [`crate::FISCAL_INGRESS_MALFORMED_REJECTION`],
/// [`crate::FISCAL_INGRESS_INVALID_REJECTION`],
/// [`crate::FISCAL_INGRESS_BUDGET_REJECTION`], or
/// [`crate::FISCAL_INGRESS_ORIGIN_REJECTION`].
pub const FISCAL_INGRESS_REJECTED_EVENT: &str = "canwu.fiscal.ingress_rejected.v1";
/// Event recorded for every settled action, applied or rejected: the durable
/// record of its outcome once the outcome leaves its target's ring. Its
/// summary is read with [`crate::FiscalSettledAction::from_summary`].
pub const FISCAL_ACTION_SETTLED_EVENT: &str = "canwu.fiscal.action_settled.v1";
/// Event recorded for every settled archive commit, applied or stale.
pub const FISCAL_ARCHIVE_MAINTENANCE_EVENT: &str = "canwu.fiscal.archive_maintenance.v1";

const PLUGIN_VERSION: &str = "0.5.0-experimental";
const SEMANTIC_HASH: &str = "e3ed464eaa0fe274cf92d07477c0525c7a3b0a62249a61ad11553f72891cdb72";

static ARCHIVE_COMMIT_PERMIT: OnceLock<PluginIngressPermit> = OnceLock::new();
static ARCHIVE_RETENTION_PERMIT: OnceLock<PluginIngressPermit> = OnceLock::new();

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct FiscalArchiveCommitPacket {
    commit: VerifiedFiscalArchiveCommitV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct FiscalArchiveRetentionPacket {
    handle_id: String,
}

/// The fiscal plugin.
///
/// # Report-system cadence
///
/// The report system (`materialize-fiscal-reports-v1`, phase 13) is
/// event-driven: it runs in every boundary that admits an event or ingress,
/// which includes every same-instant drain boundary, so a host whose
/// boundaries carry events publishes a report due at `t` in the first such
/// boundary at or after `t`. A host whose boundaries can pass without any
/// event sets [`Self::with_report_cadence`]: every boundary that carries
/// that cadence category and no event then wakes the report system when
/// report work is due, so a due report waits at most until the next such
/// boundary, and is published at its instant.
///
/// The configuration (the evidence and authority-basis kinds, and the
/// report cadence, budget share, and retention) is recorded in the plugin
/// descriptor that every snapshot persists: a run is restored and replayed
/// only with a plugin configured the same way, so changing it takes a new
/// run.
///
/// # Report budget and retention
///
/// The report system publishes at most its share of each boundary's
/// knowledge publication budget ([`FiscalReportBudget`], half of the
/// engine's by default, [`Self::with_report_budget`]); what does not fit
/// waits for the drain boundary at the same instant, which a boundary
/// schedules only when it did some of the work owed, so the drains at one
/// instant end even when the systems before the report system spend the
/// budget (the work then waits for the next run of the report system).
/// Report records are
/// retired from their holder when a newer record of the same line
/// supersedes them ([`KnowledgeRetention::RetireWhenSuperseded`], the
/// default), so held reports stay flat over a run; a host that reads a
/// closed cycle's figures after they leave the reports sets
/// [`Self::with_report_retention`] to [`KnowledgeRetention::Keep`].
#[derive(Clone, Debug)]
pub struct FiscalPlugin {
    external_evidence_kinds: Vec<DomainRecordKind>,
    authority_basis_kinds: Vec<DomainRecordKind>,
    report_cadence: Option<SystemCadence>,
    report_budget: FiscalReportBudget,
    report_retention: KnowledgeRetention,
}

impl Default for FiscalPlugin {
    fn default() -> Self {
        Self::new([])
    }
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
            report_cadence: None,
            report_budget: FiscalReportBudget::DEFAULT,
            report_retention: KnowledgeRetention::RetireWhenSuperseded,
        }
    }

    /// Sets fiscal's share of each boundary's knowledge publication budget:
    /// the most record bodies, holder entries, and payload bytes the report
    /// system publishes in one boundary, where the engine's remaining budget
    /// allows. A larger share publishes more of a burst in one boundary; a
    /// smaller one leaves more of the engine's budget to other plugins and
    /// defers the rest to drain boundaries at the same instant.
    ///
    /// The share is recorded in the plugin descriptor (in the report
    /// system's name), so a run restores and replays only with the same
    /// share, and changing it takes a new run. Registration fails with
    /// `InvalidPluginRegistration` for a share below
    /// [`FiscalReportBudget::MINIMUM`] or above the engine's per-boundary
    /// limits.
    #[must_use]
    pub fn with_report_budget(mut self, budget: FiscalReportBudget) -> Self {
        self.report_budget = budget;
        self
    }

    /// Fiscal's share of each boundary's publication budget.
    #[must_use]
    pub fn report_budget(&self) -> FiscalReportBudget {
        self.report_budget
    }

    /// Sets the retention of the report knowledge schema: under
    /// [`KnowledgeRetention::RetireWhenSuperseded`], the default, a report
    /// record leaves its holder in the boundary that publishes the record
    /// superseding it on its line, so a holder holds one current record per
    /// line and held knowledge stays flat over a run; a closed cycle's
    /// figures leave the holder once its emptied parts supersede them. Under
    /// [`KnowledgeRetention::Keep`] every report record a holder was given
    /// stays, readable through a full-history query, and held knowledge
    /// grows with every publication.
    ///
    /// The retention is part of the registered schema, so it is in the
    /// plugin descriptor: a run restores and replays only with the same
    /// retention, and changing it takes a new run.
    #[must_use]
    pub fn with_report_retention(mut self, retention: KnowledgeRetention) -> Self {
        self.report_retention = retention;
        self
    }

    /// The retention of the report knowledge schema.
    #[must_use]
    pub fn report_retention(&self) -> KnowledgeRetention {
        self.report_retention
    }

    /// Wakes the report system in every boundary that carries `cadence`
    /// and admits no event or ingress (in an event-bearing boundary it runs
    /// anyway), so that a report due at a time no event-bearing boundary
    /// follows soon is published at the next boundary of `cadence`. Canwu
    /// keeps no calendar: the host decides which boundaries carry the
    /// category.
    ///
    /// The waking system (`wake-fiscal-reports-on-cadence-v1`, phase 13)
    /// reads the root alone; when report work is due it schedules the
    /// internal [`FISCAL_REPORT_DRAIN_INGRESS`] packet, due at once, so the
    /// next boundary at the same instant, which `advance_canonical` and
    /// `step_canonical` settle, runs the report system. Parked entries alone
    /// wake nothing. (Two systems of one phase may not write the same
    /// records, so the report system itself cannot also be registered on the
    /// cadence.)
    ///
    /// The cadence is recorded in the plugin descriptor, so a run restores
    /// and replays only with the same one, and changing it takes a new run.
    /// Registration fails with `InvalidPluginRegistration` for
    /// `SystemCadence::EventDriven`, which the report system already is and
    /// which is not a category a host schedules.
    #[must_use]
    pub fn with_report_cadence(mut self, cadence: SystemCadence) -> Self {
        self.report_cadence = Some(cadence);
        self
    }

    /// The cadence category whose boundaries wake the report system, if any.
    #[must_use]
    pub fn report_cadence(&self) -> Option<SystemCadence> {
        self.report_cadence.clone()
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
    kind.namespace == FiscalCatalogRecord::NAMESPACE
}

fn canonical_kinds(kinds: impl IntoIterator<Item = DomainRecordKind>) -> Vec<DomainRecordKind> {
    let mut kinds: Vec<_> = kinds.into_iter().collect();
    kinds.sort();
    kinds.dedup();
    kinds
}

fn kind_key(kind: &DomainRecordKind) -> StateKey {
    StateKey::new(kind.namespace.clone(), kind.name.clone())
}

fn catalog_key() -> StateKey {
    kind_key(&DomainRecordKind::for_type::<FiscalCatalogRecord>())
}

/// The state keys of the fiscal record kinds, the catalog aside.
fn fiscal_record_keys() -> Vec<StateKey> {
    FiscalRecordKind::ALL
        .into_iter()
        .map(|kind| kind_key(&kind.kind()))
        .collect()
}

fn sorted(mut keys: Vec<StateKey>) -> Vec<StateKey> {
    keys.sort();
    keys.dedup();
    keys
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

    /// Checks the fiscal records a run is created or restored with: the
    /// catalog validates, the records assemble to a state that validates,
    /// every invariant across records holds, and the records are exactly
    /// that state's partition ([`audit_fiscal_record_set`]); each ledger,
    /// office, and jurisdiction fits its byte budget; and the kinds the state
    /// approves as execution evidence and cites as authority bases are ones
    /// the plugin was configured to read. It is `O(fiscal records)`.
    fn validate_activation(&self, records: &[DomainRecord]) -> Result<(), CanwuError> {
        let fiscal: Vec<&DomainRecord> = records
            .iter()
            .filter(|record| is_fiscal_record_kind(&record.reference.kind))
            .collect();
        if !fiscal
            .iter()
            .any(|record| record.reference.kind.matches_type::<FiscalRootRecord>())
        {
            for record in &fiscal {
                if record.reference.kind.matches_type::<FiscalCatalogRecord>() {
                    record.decode_payload::<FiscalCatalogRecord>()?.validate()?;
                } else {
                    return Err(invalid("fiscal records without a fiscal root"));
                }
            }
            return Ok(());
        }
        let state = audit_fiscal_record_set(fiscal.iter().copied())?;
        // Every record a settled run writes stays within its budget for
        // good: settlement holds a record's bytes, with an office's room for
        // archive commits, to its budget less the ring reserve, and what it
        // writes without that check (an outcome in the ring, the revision's
        // digits, a commit's totals into the room kept for them) stays
        // within the reserve. A run is refused when it is created with a
        // record that could not keep that promise: one whose bytes besides
        // its ring, with half the reserve, pass its budget.
        for record in &fiscal {
            let kind = &record.reference.kind;
            let (budget, room) = if kind.matches_type::<FiscalLedgerRecord>() {
                (MAX_FISCAL_LEDGER_RECORD_BYTES, 0)
            } else if kind.matches_type::<FiscalOfficeRecord>() {
                let office = state
                    .offices
                    .get(&record.reference.id)
                    .ok_or_else(|| invalid("a fiscal office record is not in its state"))?;
                (
                    MAX_FISCAL_RECORD_BYTES,
                    office_archive_reserve(&office.cycles)?,
                )
            } else if kind.matches_type::<FiscalJurisdictionRecord>() {
                (MAX_FISCAL_RECORD_BYTES, 0)
            } else {
                continue;
            };
            let ring = record
                .payload
                .get("outcomes")
                .map(json_len)
                .transpose()?
                .unwrap_or(0);
            if json_len(&record.payload)? - ring + room + FISCAL_RECORD_RING_RESERVE_BYTES / 2
                > budget
            {
                return Err(invalid(format!(
                    "fiscal record {} exceeds its byte budget",
                    record.reference
                )));
            }
        }
        // Settlement reads every evidence version a receipt cites, so a
        // kind the state approves but the plugin was not given would
        // refuse every receipt of it; the setup error is caught here.
        if state
            .root
            .execution_evidence_kinds
            .iter()
            .any(|kind| self.external_evidence_kinds.binary_search(kind).is_err())
        {
            return Err(invalid(
                "fiscal state approves an execution evidence kind not declared by FiscalPlugin::new",
            ));
        }
        if state
            .authority_bindings()
            .filter_map(|binding| binding.authority_basis.as_ref())
            .chain(
                state
                    .audit_bindings()
                    .map(|binding| &binding.authority_basis),
            )
            .any(|basis| !self.reads_as_authority_basis(&basis.record.kind))
        {
            return Err(invalid(
                "fiscal authority basis kind is not declared by FiscalPlugin::with_authority_basis_kinds",
            ));
        }
        Ok(())
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        register_fiscal_record_schemas(registrar)?;
        self.report_budget.validate()?;
        let report_schema = fiscal_report_knowledge_schema(self.report_retention);
        registrar.register_knowledge_schema(report_schema.clone())?;

        let mut fiscal_reads = vec![catalog_key()];
        fiscal_reads.extend(fiscal_record_keys());
        let mut action_reads = vec![StateKey::core_evidence()];
        action_reads.extend(fiscal_reads.iter().cloned());
        action_reads.extend(self.external_record_state_keys());
        registrar.register_command(
            PluginActionDescriptor {
                name: APPLY_FISCAL_ACTION_COMMAND.to_owned(),
                description: "Admit one batch of authority-bound fiscal procedure actions"
                    .to_owned(),
                payload_schema: PayloadSchema::Any,
                reads: sorted(action_reads),
                writes: Vec::new(),
            },
            admit_fiscal_action,
        )?;
        register_fiscal_ingress(registrar)?;
        registrar.register_archive_reachability_participant(fiscal_archive_reachability)?;

        let writable: Vec<StateKey> = FiscalRecordKind::ALL
            .into_iter()
            .filter(|kind| *kind != FiscalRecordKind::Index)
            .map(|kind| kind_key(&kind.kind()))
            .collect();
        let mut settle = BoundarySystemContract::new(
            "settle-fiscal-ingress-v1",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        let mut settle_reads = vec![StateKey::core_evidence(), StateKey::core_ingress()];
        settle_reads.extend(fiscal_reads.iter().cloned());
        settle_reads.extend(self.external_record_state_keys());
        settle.reads = sorted(settle_reads);
        settle.writes = sorted(writable.clone());
        settle.emits = vec![
            FISCAL_ACTION_SETTLED_EVENT.to_owned(),
            FISCAL_ARCHIVE_MAINTENANCE_EVENT.to_owned(),
            FISCAL_INGRESS_REJECTED_EVENT.to_owned(),
            RECEIPT_RECORDED_EVENT.to_owned(),
            CONTEXT_CHANGED_EVENT.to_owned(),
        ];
        settle.emits.sort();
        settle.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(settle, settle_fiscal_ingress)?;

        let mut reports = BoundarySystemContract::new(
            report_system_name(self.report_budget),
            BoundaryPhase::PerspectiveAndReportMaterialization,
            SystemCadence::EventDriven,
        );
        let mut report_reads = vec![
            StateKey::core_knowledge(),
            StateKey::core_person_availability(),
        ];
        report_reads.extend(fiscal_reads);
        reports.reads = sorted(report_reads);
        reports.writes = sorted(writable);
        reports.knowledge_writes = vec![KnowledgeWriteGrant {
            schema: report_schema.id,
            visibilities: vec![StateVisibility::SameBoundary],
        }];
        reports.visibility = StateVisibility::SameBoundary;
        let on_cadence = match self.report_cadence.clone() {
            None => None,
            Some(SystemCadence::EventDriven) => {
                return Err(CanwuError::new(
                    ErrorCode::InvalidPluginRegistration,
                    "the fiscal report system is event-driven already; give with_report_cadence a cadence a host schedules",
                ));
            }
            Some(cadence) => {
                // It writes nothing: two systems of one phase may not write
                // the same state, so it wakes the report system instead.
                let mut contract = BoundarySystemContract::new(
                    "wake-fiscal-reports-on-cadence-v1",
                    BoundaryPhase::PerspectiveAndReportMaterialization,
                    cadence,
                );
                contract.reads = vec![kind_key(&FiscalRecordKind::Root.kind())];
                contract.visibility = StateVisibility::SameBoundary;
                Some(contract)
            }
        };
        registrar.register_boundary_system(reports, materialize_fiscal_reports)?;
        match on_cadence {
            Some(contract) => registrar.register_boundary_system(contract, wake_fiscal_reports),
            None => Ok(()),
        }
    }
}

fn register_fiscal_record_schemas(registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
    let mut catalog_schema = DomainRecordSchema::for_record::<FiscalCatalogRecord>();
    catalog_schema.mutation_policy = DomainRecordMutationPolicy::CreateOnly;
    registrar.register_record_schema(catalog_schema)?;
    let mut root = DomainRecordSchema::for_record::<FiscalRootRecord>();
    root.references = vec![DomainReferenceSchema {
        role: ROOT_CATALOG_ROLE.to_owned(),
        targets: vec![DomainReferenceTargetKind::Domain(
            DomainRecordKind::for_type::<FiscalCatalogRecord>(),
        )],
        required: true,
        multiple: false,
        allow_retired: false,
    }];
    registrar.register_record_schema(root)?;
    registrar.register_record_schema(DomainRecordSchema::for_record::<FiscalArchiveRecord>())?;
    registrar.register_record_schema(DomainRecordSchema::for_record::<FiscalOfficeRecord>())?;
    let mut ledger = DomainRecordSchema::for_record::<FiscalLedgerRecord>();
    ledger.payload_schema = PayloadSchema::Object {
        properties: BTreeMap::from([(
            PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD.to_owned(),
            payload_required_evidence_continuation_property_v1(),
        )]),
        allow_additional: true,
    };
    registrar.register_record_schema(ledger)?;
    registrar
        .register_record_schema(DomainRecordSchema::for_record::<FiscalJurisdictionRecord>())?;
    // The engine checks these against its entity registry, so an observer
    // whose holder does not exist is refused when a run is created or
    // restored, and at every write.
    let mut observer = DomainRecordSchema::for_record::<FiscalObserverRecord>();
    observer.references = vec![DomainReferenceSchema {
        role: OBSERVER_HOLDER_ROLE.to_owned(),
        targets: vec![
            DomainReferenceTargetKind::Core(CoreEntityKind::Government),
            DomainReferenceTargetKind::Core(CoreEntityKind::Person),
        ],
        required: false,
        multiple: false,
        allow_retired: false,
    }];
    registrar.register_record_schema(observer)?;
    registrar.register_record_schema(DomainRecordSchema::for_record::<
        crate::records::FiscalDirectoryRecord,
    >())?;
    let mut index = DomainRecordSchema::for_record::<crate::records::FiscalIndexRecord>();
    index.mutation_policy = DomainRecordMutationPolicy::CreateOnly;
    registrar.register_record_schema(index)?;
    registrar.register_record_schema(DomainRecordSchema::for_record::<FiscalReportQueueRecord>())?;
    registrar.register_record_schema(DomainRecordSchema::for_record::<
        crate::records::FiscalReportScheduleRecord,
    >())
}

/// Registers the plugin's ingress types. Only the command's admission queues
/// an action, only the report system schedules a drain, and only the
/// archive functions queue a commit or an acknowledgement, so those are
/// internal and a host can author none of them; receipts and context changes
/// are host ingress.
fn register_fiscal_ingress(registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
    let _action_permit = registrar.register_internal_ingress(PluginIngressDescriptor {
        name: FISCAL_ACTION_INGRESS.to_owned(),
        description: "Settle one admitted batch of fiscal procedure actions".to_owned(),
        class: IngressClass::Decision,
        payload_schema: PayloadSchema::Any,
    })?;
    let _drain_permit = registrar.register_internal_ingress(PluginIngressDescriptor {
        name: FISCAL_REPORT_DRAIN_INGRESS.to_owned(),
        description: "Publish the fiscal report work an earlier boundary left queued".to_owned(),
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
            FISCAL_EXECUTION_RECEIPTS_INGRESS,
            "Record a batch of externally fulfilled fiscal execution evidence",
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

/// Admits one batch of fiscal actions. A rejection caused by the command
/// uses a code the engine records as a rejected command attempt instead of
/// failing the boundary.
///
/// The command is refused as a whole for what concerns the whole (its size,
/// its binding) and for any entry admission can already refuse; the message
/// of an entry's refusal begins with `entry N:` and the entry's reason, so a
/// host can drop or fix that entry and send the batch again. An entry is
/// refused when its target does not exist or belongs to another institution,
/// when its action ID has an outcome in its target's ring
/// (`IdempotencyConflict`), when it names a revision its target never had
/// ([`FISCAL_REVISION_UNKNOWN`]) or one at or below its target's outcome
/// floor ([`FISCAL_REVISION_EXPIRED`]), or when the batch names its action ID
/// twice for its target. Whether a revision is still current is checked only
/// at settlement, where a stale entry is recorded as a rejected outcome.
///
/// Admission decodes the binding's office and what an appointment or
/// observer entry changes; a scope entry reads its ledger's institution,
/// revision, ring, and the objects it cites without decoding the ledger.
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
    if json_len(payload)? > MAX_FISCAL_BATCH_JSON_BYTES {
        return Err(invalid_payload(
            "fiscal action batch exceeds its serialized byte limit",
        ));
    }
    let batch: FiscalActionBatch = decode(payload, "fiscal action batch")?;
    if batch.entries.is_empty() || batch.entries.len() > MAX_FISCAL_BATCH_ENTRIES {
        return Err(invalid_payload(
            "a fiscal action batch carries from one entry to the batch limit",
        ));
    }
    validate_identifier(&batch.authority_binding_id, "fiscal authority binding")
        .map_err(as_invalid_payload)?;
    if view
        .typed_domain_record(&fiscal_catalog_reference())?
        .is_none()
    {
        return Err(missing("fiscal catalog is not configured"));
    }
    let Some(mut working) = Working::new(view)? else {
        return Err(missing("fiscal state is not configured"));
    };
    if working
        .ensure_binding_office(&batch.authority_binding_id)?
        .is_none()
    {
        return Err(invalid_authority("fiscal authority binding is unavailable"));
    }
    let mut entries = Vec::with_capacity(batch.entries.len());
    let mut affected = BTreeSet::new();
    let mut named = BTreeSet::new();
    for (index, entry) in batch.entries.iter().enumerate() {
        let admitted = admit_entry(
            view,
            context,
            &mut working,
            &batch.authority_binding_id,
            entry,
        )
        .map_err(|error| {
            CanwuError::new(error.code, format!("entry {index}: {}", error.message))
        })?;
        if !named.insert((admitted.target.clone(), admitted.action_id.clone())) {
            return Err(invalid_payload(&format!(
                "entry {index}: the batch names fiscal action {} twice for its target",
                admitted.action_id
            )));
        }
        affected.extend(named_entities(&admitted.action));
        entries.push(admitted);
    }
    // The engine checks every affected entity of a queued item against its
    // entity registry, which also holds persons that have no record in the
    // world projection. Naming the persons and governments of a rebind here
    // therefore rejects the command as `EntityNotFound` when one of them does
    // not exist.
    if let Some(binding) = working
        .state
        .authority_binding(&batch.authority_binding_id)
        .map(|binding| binding.institution.clone())
        .or_else(|| {
            working
                .state
                .audit_binding(&batch.authority_binding_id)
                .map(|binding| binding.institution.clone())
        })
        .or_else(|| {
            working
                .state
                .appointment_policy(&batch.authority_binding_id)
                .map(|policy| policy.institution.clone())
        })
    {
        affected.insert(binding);
    }
    Ok(vec![SystemDirective::EnqueuePluginIngress {
        after: canwu_api::SimDuration::ZERO,
        packet_type: FISCAL_ACTION_INGRESS.to_owned(),
        priority: 0,
        affected: affected.into_iter().collect(),
        payload: serde_json::to_value(AdmittedFiscalBatch {
            authority_binding_id: batch.authority_binding_id,
            command: context.command_id,
            entries,
        })
        .map_err(encode_error)?,
    }])
}

/// Admits one entry of a batch: its bounds, its issuer's role, what it names,
/// its target, and its revision.
fn admit_entry(
    view: &SimulationView<'_>,
    context: &CommandContext,
    working: &mut Working<'_, SimulationView<'_>>,
    authority_binding_id: &str,
    entry: &FiscalActionEntry,
) -> Result<AdmittedFiscalEntry, CanwuError> {
    if json_len(entry)? > MAX_FISCAL_ACTION_JSON_BYTES {
        return Err(invalid_payload(
            "fiscal action entry exceeds its serialized byte limit",
        ));
    }
    validate_identifier(&entry.action_id, "fiscal action").map_err(as_invalid_payload)?;
    validate_action_bounds(&entry.action)?;
    let request = FiscalActionRequest::new(authority_binding_id, entry);
    load_admission_records(working, &request.action)?;
    let (issuer, institution) = admit_issuer(view, context, &working.state, &request)?;
    let institution = institution.clone();
    admit_action(view, working, &request, &issuer, &institution)?;
    let target = action_target(working, &request.action, &institution)?;
    admit_revision(working, &target, &request)?;
    Ok(AdmittedFiscalEntry {
        action_id: request.action_id,
        expected_revision: request.expected_revision,
        action: request.action,
        issuer,
        target,
    })
}

/// Loads, for admission, the offices and observers an appointment, binding,
/// or observer action reads: the offices the binding directory names for the
/// bindings and proposals it cites, the audited and appointing institutions'
/// offices, and the observer it changes. A scope action reads its ledger
/// without loading it.
fn load_admission_records<S: crate::working::RecordSource + ?Sized>(
    working: &mut Working<'_, S>,
    action: &FiscalAction,
) -> Result<(), CanwuError> {
    match action {
        FiscalAction::RebindAuthority {
            authority_binding_id,
            ..
        } => {
            working.ensure_binding_office(authority_binding_id)?;
        }
        FiscalAction::RebindAudit {
            audit_binding_id, ..
        } => {
            working.ensure_binding_office(audit_binding_id)?;
        }
        FiscalAction::NominateAuditor {
            nomination_id,
            audit_binding_id,
            institution,
            ..
        } => {
            working.ensure_proposal_office(nomination_id)?;
            working.ensure_binding_office(audit_binding_id)?;
            working.ensure_office(institution)?;
        }
        FiscalAction::PetitionAuditorRemoval {
            petition_id,
            audit_binding_id,
            ..
        } => {
            working.ensure_proposal_office(petition_id)?;
            working.ensure_binding_office(audit_binding_id)?;
        }
        FiscalAction::DecideAuditAppointment {
            audit_binding_id,
            decision,
        } => {
            working.ensure_binding_office(audit_binding_id)?;
            match decision {
                FiscalAppointmentDecision::AcceptNomination { nomination_id: id }
                | FiscalAppointmentDecision::AcceptPetition { petition_id: id }
                | FiscalAppointmentDecision::Dismiss { proposal_id: id } => {
                    working.ensure_proposal_office(id)?;
                }
                FiscalAppointmentDecision::Appoint { institution, .. } => {
                    working.ensure_office(institution)?;
                }
                FiscalAppointmentDecision::Remove => {}
            }
        }
        FiscalAction::RebindObserver {
            observer_binding_id,
            ..
        }
        | FiscalAction::RemoveObserver {
            observer_binding_id,
        } => {
            if validate_identifier(observer_binding_id, "observer binding").is_ok() {
                working.ensure_observer(observer_binding_id)?;
            }
        }
        FiscalAction::ChangeAdoption { .. }
        | FiscalAction::ApplyTransition { .. }
        | FiscalAction::OpenAssessment { .. }
        | FiscalAction::GrantRemission { .. }
        | FiscalAction::AuthorizeExecution { .. }
        | FiscalAction::RecordAudit { .. }
        | FiscalAction::CloseAccountingCycle { .. } => {}
    }
    if is_appointment_action(action) {
        let appointers: Vec<EntityRef> = working
            .state
            .offices
            .values()
            .filter_map(|office| office.appointment_policy.as_ref())
            .map(|policy| policy.appointer.clone())
            .collect();
        for appointer in appointers {
            working.ensure_office(&appointer)?;
        }
    }
    Ok(())
}

/// Refuses a request whose action ID has an outcome in its target's ring,
/// that names a revision its target never had, or one at or below the
/// target's outcome floor. None of the three consumes anything.
fn admit_revision(
    working: &Working<'_, SimulationView<'_>>,
    target: &FiscalActionTarget,
    request: &FiscalActionRequest,
) -> Result<(), CanwuError> {
    let head = working
        .target_head(target)?
        .ok_or_else(|| not_found("fiscal action target is unavailable"))?;
    if head.outcomes.get(&request.action_id).is_some() {
        return Err(CanwuError::new(
            ErrorCode::IdempotencyConflict,
            "fiscal action identity was already settled",
        ));
    }
    if request.expected_revision > head.revision {
        return Err(CanwuError::new(
            ErrorCode::ValueOutOfRange,
            FISCAL_REVISION_UNKNOWN,
        ));
    }
    if request.expected_revision <= head.outcomes.floor {
        return Err(CanwuError::new(
            ErrorCode::ValueOutOfRange,
            FISCAL_REVISION_EXPIRED,
        ));
    }
    Ok(())
}

/// The target of an action whose binding concerns `institution`: the scope
/// ledger of a scope action, the jurisdiction of a transition's target
/// scopes, the office of the institution a binding or proposal action
/// changes, or the observer binding an observer action changes.
fn action_target<S: crate::working::RecordSource + ?Sized>(
    working: &Working<'_, S>,
    action: &FiscalAction,
    institution: &EntityRef,
) -> Result<FiscalActionTarget, CanwuError> {
    let state = &working.state;
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
        FiscalAction::ApplyTransition {
            target_scope_bindings,
            ..
        } => {
            let mut regions = BTreeSet::new();
            for scope in target_scope_bindings.values() {
                let head = working
                    .ledger_head(scope)?
                    .ok_or_else(|| not_found("fiscal transition target scope is unavailable"))?;
                regions.insert(head.scope.jurisdiction_id);
            }
            let mut regions = regions.into_iter();
            match (regions.next(), regions.next()) {
                (Some(region), None) => FiscalActionTarget::Jurisdiction(region),
                _ => {
                    return Err(invalid_payload(
                        "fiscal transition targets must share one jurisdiction",
                    ));
                }
            }
        }
        FiscalAction::RebindAuthority { .. } | FiscalAction::RebindAudit { .. } => {
            FiscalActionTarget::Office(institution.clone())
        }
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
    if let Some(binding) = state.authority_binding(&request.authority_binding_id) {
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
    if let Some(policy) = state.appointment_policy(&request.authority_binding_id) {
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
    let Some(binding) = state.audit_binding(&request.authority_binding_id) else {
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
    working: &Working<'_, SimulationView<'_>>,
    request: &FiscalActionRequest,
    issuer: &FiscalActionIssuer,
    institution: &EntityRef,
) -> Result<(), CanwuError> {
    let state = &working.state;
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
                || state.authority_binding(audit_binding_id).is_some()
                || state.appointment_policy(audit_binding_id).is_some()
            {
                return Err(invalid_payload(
                    "a fiscal audit rebind names an auditor together with one exact authority basis, under an ID no authority binding uses and no appointment policy has",
                ));
            }
            match state.audit_binding(audit_binding_id) {
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
                .observer_binding(observer_binding_id)
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
            validate_action_scope_authority(working, action, institution)?;
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
pub(crate) fn holder_matches(actor: Option<PersonId>, holder: &KnowledgeHolderRef) -> bool {
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

/// Bounds, at admission, everything an action carries into the records: the
/// identifiers of what it creates, the references it names, its free text,
/// and its set sizes. The entry as a whole is bounded before.
#[allow(clippy::too_many_lines)]
fn validate_action_bounds(action: &FiscalAction) -> Result<(), CanwuError> {
    let created =
        |value: &str, label: &str| validate_identifier(value, label).map_err(as_invalid_payload);
    let reference = |value: &str| bounded(value, MAX_FISCAL_IDENTIFIER_BYTES, "reference");
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
            created(scope_binding_id, "scope binding")
        }
        FiscalAction::ApplyTransition {
            transition_id,
            target_scope_bindings,
        } => {
            reference(transition_id)?;
            if target_scope_bindings.is_empty()
                || target_scope_bindings.len() > MAX_FISCAL_REFERENCES_PER_DEFINITION
            {
                return Err(invalid_payload(
                    "fiscal transition binds no target rule, or more than a definition may reference",
                ));
            }
            target_scope_bindings
                .iter()
                .try_for_each(|(rule_id, scope_id)| {
                    reference(rule_id).and(created(scope_id, "scope binding"))
                })
        }
        FiscalAction::OpenAssessment {
            assessment_id,
            rule_id,
            scope_binding_id,
            accounting_cycle_id,
            unit,
            ..
        } => {
            created(assessment_id, "fiscal assessment")?;
            reference(rule_id)?;
            created(scope_binding_id, "scope binding")?;
            created(accounting_cycle_id, "fiscal accounting cycle")?;
            created(unit, "fiscal unit")
        }
        FiscalAction::GrantRemission {
            remission_id,
            scope_binding_id,
            assessment_id,
            reason,
            ..
        } => {
            created(remission_id, "fiscal remission")?;
            created(scope_binding_id, "scope binding")?;
            reference(assessment_id)?;
            text(reason, "remission reason")
        }
        FiscalAction::AuthorizeExecution {
            request_id,
            scope_binding_id,
            assessment_id,
            unit,
            purpose,
            ..
        } => {
            created(request_id, "fiscal execution request")?;
            created(scope_binding_id, "scope binding")?;
            reference(assessment_id)?;
            created(unit, "fiscal unit")?;
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
            created(scope_binding_id, "scope binding")?;
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
            created(scope_binding_id, "scope binding")?;
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
        } => created(observer_binding_id, "observer binding"),
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
            created(audit_binding_id, "fiscal audit binding")?;
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
                | FiscalAppointmentDecision::Dismiss { proposal_id: id } => {
                    created(id, "fiscal proposal")
                }
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
/// objects, reading the ledgers without decoding them.
fn validate_action_scope_authority(
    working: &Working<'_, SimulationView<'_>>,
    action: &FiscalAction,
    institution: &EntityRef,
) -> Result<(), CanwuError> {
    if let FiscalAction::ApplyTransition {
        target_scope_bindings,
        ..
    } = action
    {
        for scope_id in target_scope_bindings.values() {
            if working
                .ledger_head(scope_id)?
                .is_none_or(|head| head.scope.institution != *institution)
            {
                return Err(invalid_authority(
                    "fiscal transition authority must own every target scope",
                ));
            }
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
    let head = working
        .ledger_head(scope_id)?
        .ok_or_else(|| not_found("fiscal action scope is unavailable"))?;
    if head.scope.institution != *institution {
        return Err(invalid_authority(
            "fiscal authority binding does not own the action scope",
        ));
    }
    match cited {
        Some((id, false)) if !working.ledger_holds(scope_id, id, &["assessments"])? => {
            Err(not_found("fiscal action names an unknown assessment"))
        }
        Some((id, true))
            if !working.ledger_holds(
                scope_id,
                id,
                &["assessments", "execution_requests", "execution_receipts"],
            )? =>
        {
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
pub(crate) fn require_current_basis(
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
                    | FISCAL_EXECUTION_RECEIPTS_INGRESS
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
/// resolved for each entry. Any other action packet, which only a boundary
/// system of another plugin that declares this target could schedule, is
/// refused and changes nothing. The check reads the ingress record of the
/// boundary that admits it and no retained command evidence, so a sealed run
/// decides as its exact replay does.
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
                match decode::<AdmittedFiscalBatch>(packet.payload, "admitted fiscal batch") {
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
            FISCAL_EXECUTION_RECEIPTS_INGRESS => {
                match decode::<FiscalExecutionReceiptBatch>(
                    packet.payload,
                    "fiscal execution receipt batch",
                ) {
                    Ok(batch) if batch.receipts.len() <= MAX_FISCAL_RECEIPTS_PER_PACKET => {
                        FiscalPacket::Receipts(packet.ingress, batch.receipts)
                    }
                    _ => malformed(packet, FISCAL_EXECUTION_RECEIPTS_INGRESS),
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

/// Settles the boundary's fiscal ingress in settlement order. A boundary
/// that admitted nothing for this plugin proposes nothing and returns before
/// any fiscal record is read.
fn settle_fiscal_ingress(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let packets = admitted_fiscal_packets(view, context)?;
    if packets.is_empty() {
        return Ok(BoundaryProposal::default());
    }
    let Some(catalog) = committed_fiscal_catalog(view)? else {
        return Ok(BoundaryProposal::default());
    };
    let packets = decode_fiscal_packets(&packets);
    settle_fiscal_packets(view, context, &catalog, &packets)
}

/// The report system's name, which carries fiscal's share of the
/// publication budget when it is not the default, so that the share is in
/// the plugin descriptor and the handler, a plain function, reads it from
/// its own name.
fn report_system_name(budget: FiscalReportBudget) -> String {
    if budget == FiscalReportBudget::DEFAULT {
        REPORT_SYSTEM.to_owned()
    } else {
        format!(
            "{REPORT_SYSTEM}/share/{}/{}/{}",
            budget.bodies, budget.entries, budget.payload_bytes
        )
    }
}

/// The share a report system's name carries; see [`report_system_name`].
fn report_system_share(name: &str) -> Result<FiscalReportBudget, CanwuError> {
    let Some(share) = name.strip_prefix(REPORT_SYSTEM) else {
        return Err(invalid("the fiscal report system has an unknown name"));
    };
    if share.is_empty() {
        return Ok(FiscalReportBudget::DEFAULT);
    }
    let parts: Vec<&str> = share
        .strip_prefix("/share/")
        .map(|share| share.split('/').collect())
        .unwrap_or_default();
    let number = |index: usize| -> Result<usize, CanwuError> {
        parts
            .get(index)
            .and_then(|part| part.parse().ok())
            .ok_or_else(|| invalid("the fiscal report system's name carries no share"))
    };
    if parts.len() != 3 {
        return Err(invalid("the fiscal report system's name carries no share"));
    }
    Ok(FiscalReportBudget {
        bodies: number(0)?,
        entries: number(1)?,
        payload_bytes: number(2)?,
    })
}

const REPORT_SYSTEM: &str = "materialize-fiscal-reports-v1";

fn materialize_fiscal_reports(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let share = report_system_share(&context.system)?;
    materialize_reports(view, context, || committed_fiscal_catalog(view), share)
}

/// Wakes the report system in a boundary of the host's report cadence that
/// has no event or ingress, so the event-driven report system does not run
/// in it: when report work is due, it schedules the internal drain packet,
/// due at once, and the next boundary at the same instant runs the report
/// system. It reads the root alone. Parked entries alone wake nothing: the
/// rotation advances in the runs of the report system that happen anyway (a
/// revival is itself an event, and interval observers fall due regularly).
fn wake_fiscal_reports(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    if !context.admitted_events.is_empty() || !context.admitted_ingress.is_empty() {
        return Ok(BoundaryProposal::default());
    }
    let Some(root) = crate::working::active_record(view, &crate::fiscal_root_reference())? else {
        return Ok(BoundaryProposal::default());
    };
    let earliest_due: Option<canwu_api::SimTime> =
        crate::working::field(&root.payload, "earliest_due")?;
    if earliest_due.is_none_or(|due| due > context.at) || !view.can_schedule_ingress() {
        return Ok(BoundaryProposal::default());
    }
    Ok(BoundaryProposal {
        directives: vec![crate::report::drain_directive()],
        ..BoundaryProposal::default()
    })
}

/// The command that submits one batch of fiscal actions.
pub fn fiscal_action_command(batch: &FiscalActionBatch) -> Result<Command, serde_json::Error> {
    Ok(Command::Plugin {
        plugin: PLUGIN_NAME.to_owned(),
        command: APPLY_FISCAL_ACTION_COMMAND.to_owned(),
        payload: serde_json::to_value(batch)?,
    })
}

/// Queues a batch of up to [`MAX_FISCAL_RECEIPTS_PER_PACKET`] execution
/// receipts, settled in order by the boundary at `due_at`.
///
/// Each receipt must cite between one and
/// [`MAX_FISCAL_EVIDENCE_PER_RECORD`] evidence versions, each the current
/// version of its record now; the boundary refuses, with an event, a receipt
/// whose evidence is no longer current when it settles.
pub fn enqueue_execution_receipts<H: CanwuRun + ?Sized>(
    canwu: &mut H,
    due_at: SimTime,
    receipts: &[FiscalExecutionReceiptPacket],
) -> Result<IngressReceipt, CanwuError> {
    if receipts.is_empty() || receipts.len() > MAX_FISCAL_RECEIPTS_PER_PACKET {
        return Err(invalid(
            "a fiscal receipt batch carries from one receipt to the batch limit",
        ));
    }
    for packet in receipts {
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
    }
    let payload = serde_json::to_value(FiscalExecutionReceiptBatch {
        receipts: receipts.to_vec(),
    })
    .map_err(encode_error)?;
    canwu.enqueue_plugin_ingress(PluginIngressRequest::new(
        PLUGIN_NAME,
        FISCAL_EXECUTION_RECEIPTS_INGRESS,
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

/// The revision of one action target now, read without decoding its record;
/// `None` when the target does not exist. An entry names it as its
/// `expected_revision`. This is a host read of ground truth; an actor-facing
/// controller gets the revision through its option descriptor.
pub fn fiscal_target_revision<H: CanwuRun + ?Sized>(
    runtime: &H,
    target: &FiscalActionTarget,
) -> Result<Option<u64>, CanwuError> {
    let host = HostRecords(runtime);
    let Some(working) = Working::new(&host)? else {
        return Ok(None);
    };
    Ok(working.target_head(target)?.map(|head| head.revision))
}

/// The recent outcome of `action_id` on `target`, while the target's ring
/// keeps it; the outcome's `canwu.fiscal.action_settled.v1` event keeps it
/// for good.
pub fn fiscal_recent_outcome<H: CanwuRun + ?Sized>(
    runtime: &H,
    target: &FiscalActionTarget,
    action_id: &str,
) -> Result<Option<FiscalRecentOutcome>, CanwuError> {
    let host = HostRecords(runtime);
    let Some(working) = Working::new(&host)? else {
        return Ok(None);
    };
    Ok(working
        .target_head(target)?
        .and_then(|head| head.outcomes.get(action_id).cloned()))
}

/// The target an action issued under `authority_binding_id` would have, as
/// admission resolves it: the scope ledger of a scope action, the
/// jurisdiction of a transition's target scopes, the office of the
/// institution a binding or proposal action changes, or the observer binding
/// an observer action changes. A host reads the target's revision with
/// [`fiscal_target_revision`] to stamp an entry. It reads the binding
/// directory, the binding's office, and the records the action names.
pub fn fiscal_action_target<H: CanwuRun + ?Sized>(
    runtime: &H,
    authority_binding_id: &str,
    action: &FiscalAction,
) -> Result<FiscalActionTarget, CanwuError> {
    let host = HostRecords(runtime);
    let mut working =
        Working::new(&host)?.ok_or_else(|| missing("fiscal state is not configured"))?;
    working.ensure_binding_office(authority_binding_id)?;
    load_admission_records(&mut working, action)?;
    let institution = working
        .state
        .authority_binding(authority_binding_id)
        .map(|binding| binding.institution.clone())
        .or_else(|| {
            working
                .state
                .appointment_policy(authority_binding_id)
                .map(|policy| policy.institution.clone())
        })
        .or_else(|| {
            working
                .state
                .audit_binding(authority_binding_id)
                .map(|binding| binding.institution.clone())
        })
        .ok_or_else(|| invalid_authority("fiscal authority binding is unavailable"))?;
    action_target(&working, action, &institution)
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

/// The records of a run that an archive commit of up to `book_limit` books
/// is prepared from, as a partial state: the root, the archive head, the
/// ledgers that hold the first `book_limit` closed books in (scope, cycle)
/// order, and those ledgers' offices. Every other ledger is read only far
/// enough to see that its hot books are open, and no other record is read.
fn archive_candidates<H: CanwuRun + ?Sized>(
    runtime: &H,
    book_limit: usize,
) -> Result<FiscalState, FiscalArchiveError> {
    let host = HostRecords(runtime);
    let mut working =
        Working::new(&host)?.ok_or_else(|| missing("fiscal state is not configured"))?;
    working.ensure_archive()?;
    // Refused before any ledger is read; preparing checks it again.
    if working.state.archive.pending_handles.len() >= crate::MAX_FISCAL_ARCHIVE_PENDING_HANDLES {
        return Err(FiscalArchiveError::Backpressure);
    }
    let mut closed = BTreeMap::<&str, usize>::new();
    for record in runtime.domain_records() {
        if !record.reference.kind.matches_type::<FiscalLedgerRecord>()
            || record.lifecycle != canwu_api::DomainRecordLifecycle::Active
        {
            continue;
        }
        // An open book's status is the text `open`; a closed one's is an
        // object naming who closed it.
        let books = record
            .payload
            .get("books")
            .and_then(Value::as_object)
            .map_or(0, |books| {
                books
                    .values()
                    .filter(|book| book.get("status").and_then(Value::as_str) != Some("open"))
                    .count()
            });
        if books > 0 {
            closed.insert(record.reference.id.as_str(), books);
        }
    }
    let mut taken = 0_usize;
    for (scope, books) in closed {
        if taken >= book_limit {
            break;
        }
        if !working.ensure_ledger(scope)? {
            return Err(missing("a fiscal ledger is not active").into());
        }
        let office = working
            .state
            .ledgers
            .get(scope)
            .map(|ledger| ledger.office.clone())
            .ok_or_else(|| missing("a fiscal ledger is not loaded"))?;
        working.ensure_office_key(&office)?;
        taken += books;
    }
    Ok(working.state)
}

/// Prepares one archive commit of a run: up to `book_limit` closed books, in
/// (scope, cycle) order, together with the receipt of the commit at the head
/// and the institution totals of every cycle whose last hot books it takes.
/// It takes fewer books when those records would not fit
/// [`crate::MAX_FISCAL_ARCHIVE_COMMIT_RECORDS`].
///
/// It reads only the records the commit is prepared from: the root and the
/// archive head, every ledger far enough to see whether it holds a closed
/// book, and the ledgers of the books it may select with their offices, so
/// its cost follows the number of ledgers and the books selected, not the
/// size of the fiscal state.
///
/// The records are cut into blobs of
/// [`crate::MAX_FISCAL_ARCHIVE_RECORDS_PER_BLOB`], each with its membership
/// and temporal page, and the directory commits to the cumulative lookup
/// index extended by them. Extending the index reads the head's directory
/// and the lookup pages on the paths of the new keys from `store`; nothing
/// is written until [`PreparedFiscalArchiveBatchV1::store_and_verify`]. The
/// result is a function of the run's fiscal records, its sealed cut, and
/// the stored index alone.
///
/// A run that seals its evidence ([`CanwuRun::sealed_evidence_cursor`] is
/// `Some`) cannot archive a book that cites a version a boundary established
/// after that cut: the book's declaration of the version ends when it is
/// archived, so the version must already be in a sealed segment. Such a
/// book is refused with [`FiscalArchiveError::UnsealedCitations`]; seal,
/// then archive. The cut is the run's own, so a host cannot pass a wrong
/// one. A run that retains all its evidence (a `Canwu`, a never-sealed
/// `CompactedCanwu`, or one restored from a checkpoint and its journal until
/// its next seal) refuses none, and its archived books' citations reach a
/// sealed segment only at its next seal: seal before each continuation
/// checkpoint, or those versions are lost to the saved run.
///
/// It refuses, as [`FiscalArchiveError::Backpressure`], while the archive
/// head holds [`crate::MAX_FISCAL_ARCHIVE_PENDING_HANDLES`] retention
/// handles, of applied or stale commits, that the host has not
/// acknowledged, and with [`FiscalArchiveError::NothingToArchive`] when no
/// book is closed and no receipt waits.
pub fn prepare_fiscal_archive<H: CanwuRun + ?Sized>(
    runtime: &H,
    book_limit: usize,
    store: &dyn FiscalArchiveStore,
) -> Result<PreparedFiscalArchiveBatchV1, FiscalArchiveError> {
    if book_limit == 0 || book_limit > crate::MAX_FISCAL_ARCHIVE_COMMIT_RECORDS {
        return Err(FiscalArchiveError::Mismatch(
            "fiscal archive book limit is invalid".to_owned(),
        ));
    }
    archive_candidates(runtime, book_limit)?.prepare_archive_batch(
        book_limit,
        store,
        runtime.sealed_evidence_cursor().as_ref(),
    )
}

/// Stores, reads back, verifies, and queues one fiscal archive commit, due
/// now.
///
/// The batch must equal what [`prepare_fiscal_archive`] derives from the
/// run's current fiscal records, its sealed cut, and `store` for the same
/// number of books, lookup pages included: a batch prepared before a
/// boundary that changed a selected book, an affected office's cycle
/// bookkeeping, or the archive head is refused here. Like preparing, this
/// reads only the records the commit is prepared from. The boundary that
/// settles the commit reads no archive object: it applies the commit when
/// those are still as prepared, and otherwise records it as stale. Prepare
/// and enqueue in one gap between boundaries.
pub fn enqueue_fiscal_archive<H: CanwuRun + ?Sized>(
    runtime: &mut H,
    prepared: &PreparedFiscalArchiveBatchV1,
    store: &dyn FiscalArchiveStore,
) -> Result<FiscalArchiveIngressReceiptV1, FiscalArchiveError> {
    let canonical = prepare_fiscal_archive(&*runtime, prepared.selected_books.len().max(1), store)?;
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
        runtime.time(),
        serde_json::to_value(FiscalArchiveCommitPacket { commit }).map_err(encode_error)?,
    )
    .with_archive_retention(vec![PluginArchiveRetention {
        namespace: FISCAL_ARCHIVE_INDEX_DIRECTORY_NAMESPACE.to_owned(),
        object_id: directory_root.clone(),
    }]);
    match runtime.enqueue_permitted_plugin_ingress(request, permit) {
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
/// under it, from the archive head. It reads the archive head alone.
///
/// The handle must be in the head, settled. An acknowledgement that settles
/// after another one retired the handle is a no-op, so calling this twice
/// before the next boundary is harmless. A stale commit that settlement
/// refused because the head already held as many unacknowledged stale
/// commits as it may has no handle in the head: this returns an error for
/// it, and the host abandons that commit's stored copy (it stays in its
/// `DurableIngress` phase) itself.
pub fn finalize_fiscal_archive_retention<H: CanwuRun + ?Sized>(
    runtime: &mut H,
    store: &dyn FiscalArchiveStore,
    receipt: &FiscalArchiveIngressReceiptV1,
) -> Result<IngressReceipt, FiscalArchiveError> {
    let head = active_record(&HostRecords(&*runtime), &fiscal_archive_reference())?
        .ok_or_else(|| missing("the fiscal archive record is missing"))?
        .decode_payload::<FiscalArchiveRecord>()?;
    let handle: &FiscalArchiveRetentionHandleV1 = head
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
    Ok(runtime.enqueue_permitted_plugin_ingress(
        PluginIngressRequest::new(
            PLUGIN_NAME,
            FISCAL_ARCHIVE_RETENTION_INGRESS,
            runtime.time(),
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
    if let Some(record) = active_record(view, &fiscal_archive_reference())? {
        let archive = record.decode_payload::<FiscalArchiveRecord>()?;
        if let Some(root) = &archive.directory_root {
            roots.insert(root.clone());
            complete.insert(root.clone());
        }
        roots.extend(
            archive
                .pending_handles
                .values()
                .map(|handle| handle.target_directory_root.clone()),
        );
        bound = usize::try_from(archive.committed_batch_count)
            .unwrap_or(usize::MAX)
            .saturating_add(archive.pending_handles.len())
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

fn decode<T: serde::de::DeserializeOwned>(value: &Value, label: &str) -> Result<T, CanwuError> {
    T::deserialize(value).map_err(|error| {
        CanwuError::new(
            ErrorCode::InvalidPayload,
            format!("{label} could not be decoded: {error}"),
        )
    })
}

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn encode_error(error: serde_json::Error) -> CanwuError {
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

/// The action's issuer decided against a record that has since changed.
pub(crate) fn stale(message: &str) -> CanwuError {
    CanwuError::new(ErrorCode::DomainRecordVersionConflict, message)
}
