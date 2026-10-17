use crate::PLUGIN_NAME;
use canwu_api::{
    CanwuError, DomainRecord, DomainRecordClass, DomainRecordDraft, DomainRecordKind,
    DomainRecordLifecycle, DomainRecordType, DomainRecordVersionRef, DomainRecordVersionSource,
    DomainValueKindClass, EntityRef, ErrorCode, EvidenceRef, IngressId, KnowledgeHolderRef,
    KnowledgeLimitsV1, PersonId, ResourceId, SimDuration, SimTime, SimulationGranularity,
    TypedDomainRecordRef,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const FISCAL_CONTENT_SCHEMA_VERSION: u32 = 1;
/// Version of the fiscal records. Version 5 partitions the state into
/// records: one root, one office per institution, one ledger per scope
/// binding, one jurisdiction per catalog region in use, one observer per
/// observer binding, binding-directory buckets, static index pages, and
/// report-queue pages (see [`crate::FiscalRecordKind`]). Each ledger, office,
/// jurisdiction, and observer has a revision of its own. Version 6 adds an
/// observer binding's granularity and cadence, an office's mechanisms and its
/// observers' granularity and cadence, the report schedule record, and the
/// root's earliest due time and parked flag in place of its list of queue
/// pages.
pub const FISCAL_RUNTIME_SCHEMA_VERSION: u32 = 6;
/// Scope bindings a run may have, one ledger each.
pub const MAX_FISCAL_SCOPE_BINDINGS: usize = 1_048_576;
/// Authority bindings one institution's office holds.
pub const MAX_FISCAL_AUTHORITY_BINDINGS_PER_OFFICE: usize = 256;
/// Audit bindings on one institution, held in its office. Audit binding IDs
/// are one namespace across all institutions, with authority binding and
/// appointment policy IDs (see [`crate::FiscalDirectoryBucket`]).
pub const MAX_FISCAL_AUDIT_BINDINGS_PER_OFFICE: usize = 256;
/// Observer bindings a run may have, one record each.
pub const MAX_FISCAL_OBSERVERS: usize = 65_536;
/// Institutions one observer binding may see.
pub const MAX_FISCAL_VISIBLE_INSTITUTIONS: usize = 4_096;
/// Observer bindings that may see one institution: its office lists them.
pub const MAX_FISCAL_OBSERVERS_PER_INSTITUTION: usize = 4_096;
/// Report rows one observer binding has: one per scope of the institutions
/// it sees. A part is [`FISCAL_REPORT_ROWS_PER_PART`] rows, so a cycle of a
/// report has at most [`MAX_FISCAL_REPORT_PARTS_PER_CYCLE`] parts.
pub const MAX_FISCAL_OBSERVER_ROWS: usize = 16_384;
/// Partition keys (unit, payment form) one (institution, mechanism, cycle)
/// may use, and so assessments one book may hold: an assessment that would
/// add another is rejected with [`FISCAL_PARTITION_KEYS_EXHAUSTED`]. A row of
/// a report holds a book's facts, one per partition.
pub const MAX_FISCAL_PARTITION_KEYS: usize = 4;
/// Remissions one book holds.
pub const MAX_FISCAL_REMISSIONS_PER_BOOK: usize = 256;
/// Execution requests one book holds.
pub const MAX_FISCAL_EXECUTION_REQUESTS_PER_BOOK: usize = 256;
/// Execution receipts one book holds.
pub const MAX_FISCAL_EXECUTION_RECEIPTS_PER_BOOK: usize = 1_024;
/// Audit findings one book holds.
pub const MAX_FISCAL_AUDITS_PER_BOOK: usize = 256;
/// Adoptions one scope ledger holds.
pub const MAX_FISCAL_ADOPTIONS_PER_LEDGER: usize = 64;
/// Accounting cycle books, open or closed, one scope ledger holds at once. A
/// further book is refused with [`FISCAL_HOT_BOOKS_EXHAUSTED`] until a closed
/// one is archived.
pub const MAX_FISCAL_HOT_BOOKS_PER_LEDGER: usize = 64;
/// Accounting cycles of which one institution's scopes may hold hot books at
/// once; a book of a further cycle is refused with
/// [`FISCAL_HOT_BOOKS_EXHAUSTED`].
pub const MAX_FISCAL_HOT_CYCLES_PER_OFFICE: usize = 64;
/// Recent outcomes one action target keeps in its [`FiscalOutcomeRing`].
/// Every outcome, applied or rejected, is also recorded for good in its
/// `canwu.fiscal.action_settled.v1` event.
pub const FISCAL_RECENT_OUTCOMES: usize = 8;
/// Fiscal actions one boundary applies. Only an action that is current and
/// authorized counts; a stale or unauthorized action is rejected without
/// using the limit. A further such action in the same boundary is recorded
/// as a `Rejected` outcome whose reason ends with
/// [`FISCAL_BOUNDARY_ACTION_LIMIT`].
pub const MAX_FISCAL_ACTIONS_PER_BOUNDARY: usize = 65_536;
/// Fiscal actions one boundary applies to one target record, which bounds
/// the repeated validation of a record. A further one is rejected with
/// [`FISCAL_RECORD_ACTION_LIMIT`].
pub const MAX_FISCAL_ACTIONS_PER_RECORD_PER_BOUNDARY: usize = 64;
/// Scope ledgers one boundary writes, each counted once however many actions
/// and receipts touch it. An action past it is rejected with
/// [`FISCAL_BOUNDARY_ACTION_LIMIT`], and a receipt refused with
/// [`FISCAL_RECEIPT_BOUNDARY_LIMIT`].
pub const MAX_FISCAL_LEDGER_WRITES_PER_BOUNDARY: usize = 65_536;
/// Entries one fiscal action batch may carry.
pub const MAX_FISCAL_BATCH_ENTRIES: usize = 4_096;
/// Execution receipts one receipt packet may carry.
pub const MAX_FISCAL_RECEIPTS_PER_PACKET: usize = 4_096;
/// Longest identifier, in bytes, that a fiscal record or action may carry.
/// The same length bounds every text of a report row: a scope, a unit, an
/// accounting cycle, and the namespace, name, and ID of a scope's
/// domain-record institution together.
pub const MAX_FISCAL_IDENTIFIER_BYTES: usize = 160;
/// Longest free text, in bytes, that a fiscal action may carry: an audit
/// `finding`, a remission `reason`, or an execution `purpose`. Admission
/// refuses a longer one as `InvalidPayload`.
pub const MAX_FISCAL_ACTION_TEXT_BYTES: usize = 2_048;
/// Largest serialized fiscal action entry, in bytes, that admission
/// accepts. It bounds every field no narrower limit names, such as entity
/// references and evidence references.
pub const MAX_FISCAL_ACTION_JSON_BYTES: usize = 32 * 1_024;
/// Largest serialized fiscal action batch, in bytes.
pub const MAX_FISCAL_BATCH_JSON_BYTES: usize = 4 * 1_024 * 1_024;
/// Rows one part of a fiscal report holds: one cycle of 16 consecutive rows.
pub const FISCAL_REPORT_ROWS_PER_PART: usize = 16;
/// Parts one cycle of an observer's report can have.
pub const MAX_FISCAL_REPORT_PARTS_PER_CYCLE: usize =
    MAX_FISCAL_OBSERVER_ROWS / FISCAL_REPORT_ROWS_PER_PART;
/// Hot accounting cycles one observer's report shows at once, each in a slot
/// of its own. A cycle past them is left out of the report until a slot is
/// free, and the summary counts it in
/// [`FiscalReportSummary::omitted_cycles`]. It also bounds the summary, which
/// names every cycle the report shows, within one knowledge record.
pub const MAX_FISCAL_REPORT_CYCLE_SLOTS: usize = 64;
/// Fiscal's share of a boundary's knowledge publication budget: the most
/// record bodies, holder entries, and payload bytes the report system
/// publishes in one boundary.
///
/// The report system publishes the least of this share and what the
/// engine's budget has left at its cut
/// (`SimulationView::knowledge_publication_budget`), which every publishing
/// plugin spends in dispatch order; each report record is one body and one
/// entry, charged its payload's compact JSON bytes. What does not fit stays
/// queued, and the boundary schedules an internal `fiscal_report_drain_v1`
/// packet, due at once, so the next boundary at the same instant publishes
/// it. Set it with `FiscalPlugin::with_report_budget`; the default is half
/// of `KnowledgeLimitsV1::CURRENT`. It is recorded in the plugin descriptor,
/// so a run restores and replays only with the same share.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalReportBudget {
    /// Record bodies one boundary publishes.
    pub bodies: usize,
    /// Holder entries one boundary adds.
    pub entries: usize,
    /// Payload bytes of the bodies one boundary publishes.
    pub payload_bytes: usize,
}

impl FiscalReportBudget {
    /// Half of the engine's per-boundary budget: 16,384 bodies, 16,384
    /// entries, and 32 MiB of payload.
    pub const DEFAULT: Self = Self {
        bodies: KnowledgeLimitsV1::CURRENT.records_per_boundary / 2,
        entries: KnowledgeLimitsV1::CURRENT.holder_entries_per_boundary / 2,
        payload_bytes: KnowledgeLimitsV1::CURRENT.payload_bytes_per_boundary / 2,
    };

    /// The smallest share: two records of the largest payload, a summary and
    /// a part, so that every boundary with its whole share left publishes at
    /// least one part of what is owed.
    pub const MINIMUM: Self = Self {
        bodies: 2,
        entries: 2,
        payload_bytes: 2 * KnowledgeLimitsV1::CURRENT.payload_bytes_per_record,
    };

    /// Checks that the share lies between [`Self::MINIMUM`] and the engine's
    /// per-boundary limits.
    pub(crate) fn validate(&self) -> Result<(), CanwuError> {
        let limits = KnowledgeLimitsV1::CURRENT;
        if (Self::MINIMUM.bodies..=limits.records_per_boundary).contains(&self.bodies)
            && (Self::MINIMUM.entries..=limits.holder_entries_per_boundary).contains(&self.entries)
            && (Self::MINIMUM.payload_bytes..=limits.payload_bytes_per_boundary)
                .contains(&self.payload_bytes)
        {
            Ok(())
        } else {
            Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                "a fiscal report budget lies between FiscalReportBudget::MINIMUM and the engine's per-boundary knowledge limits",
            ))
        }
    }
}

impl Default for FiscalReportBudget {
    fn default() -> Self {
        Self::DEFAULT
    }
}
/// Parked report-queue entries one run of the report system checks again:
/// it re-reads the holder of each of the next parked entries after the
/// root's `parked_cursor`, wrapping round, so a revived holder is
/// published to within `ceil(parked / 16)` runs, at a fixed cost per run.
pub const FISCAL_PARKED_CHECKS_PER_BOUNDARY: usize = 16;
/// Report-queue pages: the report work owed to an observer is kept on the
/// page its binding ID's digest names, so a boundary writes only the pages
/// of the observers it queues or publishes.
pub const FISCAL_REPORT_QUEUE_PAGES: usize = 4_096;
/// Cycles one observer's queued report work names part by part; past them
/// the whole report is owed.
pub const FISCAL_REPORT_QUEUE_CYCLES: usize = 16;
/// Parts of one cycle one observer's queued report work names; past them
/// every part of the cycle is owed.
pub const FISCAL_REPORT_QUEUE_PARTS: usize = 64;
/// Binding-directory buckets: the office of a binding, appointment policy,
/// or auditor proposal ID is kept in the bucket its digest names, so a
/// request that names only an ID finds its office with one read.
pub const FISCAL_DIRECTORY_BUCKETS: usize = 4_096;
/// Scope IDs one index page lists: an office's scopes in rank order, or a
/// region's scopes in ID order. Index pages are written with the scenario
/// and never change.
pub const FISCAL_INDEX_PAGE_SCOPES: usize = 1_024;
pub const MAX_FISCAL_EVIDENCE_KINDS: usize = 32;
pub const MAX_FISCAL_EVIDENCE_PER_RECORD: usize = 32;
/// Parties one [`FiscalAppointmentPolicy`] lists as nominators, and as
/// petitioners, each.
pub const MAX_FISCAL_APPOINTMENT_PARTIES: usize = 32;
/// Nominations and removal petitions that one party of one
/// [`FiscalAppointmentPolicy`] may have pending at once, over all the
/// policy's bindings. Admission refuses a further one as `ValueOutOfRange`,
/// unless it replaces the party's pending proposal of the same kind on the
/// same binding.
pub const MAX_FISCAL_PENDING_PROPOSALS_PER_PARTY: usize = 4;
/// Resolved nominations and petitions kept for one policy, as the record of
/// who proposed, on what, and who decided. A resolution past it drops the
/// policy's oldest resolved proposals (by the time they were resolved, then
/// by ID); a binding and a finding keep their nominator in
/// [`FiscalProposalRef`] either way.
pub const MAX_FISCAL_RESOLVED_PROPOSALS_PER_POLICY: usize = 64;
/// Serialized byte budget of a scope ledger record. An action whose result
/// would not fit, less [`FISCAL_RECORD_RING_RESERVE_BYTES`], is rejected as
/// its own outcome with [`FISCAL_RECORD_BUDGET_EXHAUSTED`]; a receipt that
/// would not fit is refused with [`FISCAL_INGRESS_BUDGET_REJECTION`]. A
/// ledger with one hot annual book takes about 2.5 KiB, so the budget is
/// reached only by years of books left unarchived, and
/// [`MAX_FISCAL_HOT_BOOKS_PER_LEDGER`] refuses a new book before that.
pub const MAX_FISCAL_LEDGER_RECORD_BYTES: usize = 256 * 1_024;
/// Serialized byte budget of an office or jurisdiction record, held as for a
/// ledger. The caps on what an office holds (bindings, proposals, observers,
/// hot cycles) keep it under about half of this; a jurisdiction grows with
/// the transitions its catalog makes eligible in its region.
pub const MAX_FISCAL_RECORD_BYTES: usize = 1_024 * 1_024;
/// Room every record keeps for its outcome ring and the digits of its
/// revision: an action is applied only while its record fits its budget
/// with this much to spare, so recording any outcome always fits.
pub const FISCAL_RECORD_RING_RESERVE_BYTES: usize = 4 * 1_024;
pub const MAX_FISCAL_CATALOG_PERIODS: usize = 512;
pub const MAX_FISCAL_CATALOG_REGIONS: usize = 65_536;
pub const MAX_FISCAL_CATALOG_DEFINITIONS: usize = 16_384;
pub const MAX_FISCAL_CATALOG_COVERAGE_CELLS: usize = 1_048_576;
pub const MAX_FISCAL_REFERENCES_PER_DEFINITION: usize = 1_024;
pub const MAX_FISCAL_CATALOG_JSON_BYTES: usize = 64 * 1_024 * 1_024;
pub(crate) const ROOT_ID: &str = "primary";

pub struct FiscalCatalogRecord;

impl DomainRecordType for FiscalCatalogRecord {
    type Payload = CompiledFiscalCatalog;
    type Class = DomainValueKindClass;

    const NAMESPACE: &'static str = "canwu.fiscal";
    const NAME: &'static str = "catalog";
}

#[must_use]
pub fn fiscal_catalog_reference() -> TypedDomainRecordRef<FiscalCatalogRecord> {
    TypedDomainRecordRef::new(ROOT_ID)
}

#[must_use]
pub fn initial_catalog_version() -> DomainRecordVersionRef {
    DomainRecordVersionRef {
        record: fiscal_catalog_reference().into_untyped(),
        version: 1,
        established_by: DomainRecordVersionSource::InitialScenario,
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FiscalMechanism {
    LandTax,
    LaborService,
    GrainTribute,
    MilitaryFarm,
    SaltMonopoly,
    CommercialTax,
    MiscellaneousLevy,
    TreasuryAndRemittance,
    MilitarySurcharge,
    MerchantCredit,
    ReliefReserve,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FiscalCoverageStatus {
    Supported,
    ArchetypeFallback,
    ExplicitUnknown,
    NotApplicable,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoricalConfidence {
    Low,
    Medium,
    High,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FiscalPaymentForm {
    Grain,
    Silver,
    Labor,
    SaltCertificate,
    Coin,
    Goods,
    Credit,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FiscalAssessmentBasis {
    RegisteredHousehold,
    RegisteredLand,
    TaxGrainQuota,
    LaborService,
    MilitaryFarmOutput,
    SaltCertificate,
    CommercialFlow,
    EmergencySurcharge,
    NegotiatedCredit,
    TreasuryAllocation,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FiscalCommutationPolicy {
    Disabled,
    Allowed,
    Required,
    RegionalPractice,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FiscalAdoptionStage {
    Promulgated,
    Communicated,
    Accepted,
    Implemented,
    Audited,
    Entrenched,
    Suspended,
    Repealed,
}

impl FiscalAdoptionStage {
    #[must_use]
    pub const fn is_operational(self) -> bool {
        matches!(self, Self::Implemented | Self::Audited | Self::Entrenched)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FiscalHistoricalMode {
    RecordedBaseline,
    Counterfactual,
    ResearchReplay,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FiscalExecutionKind {
    Collect,
    Remit,
    Disburse,
    Reserve,
    Return,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FiscalReceiptDisposition {
    Fulfilled,
    Partial,
    Rejected,
    Excused,
}

impl FiscalReceiptDisposition {
    #[must_use]
    pub const fn counts_as_fulfillment(self) -> bool {
        matches!(self, Self::Fulfilled | Self::Partial)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FiscalActionDisposition {
    Applied,
    Rejected,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FiscalAuditSeverity {
    Notice,
    Concern,
    Material,
    Critical,
}

/// The binding role an issuer held when a fiscal action was admitted.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FiscalIssuerRole {
    /// The authority binding's `authorized_actor`.
    Principal,
    /// The authority binding's `acting_actor`, under its `authority_basis`.
    Acting,
    /// A decision controller deciding as the binding's institution as a body:
    /// its decision origin is that institution with no responsible actor.
    /// Only the rebind and appointment actions admit this role (see
    /// [`FiscalAction::NominateAuditor`]), and only while the binding names
    /// an authorized actor; on a binding with none, the same controller holds
    /// the principal role.
    Institution,
    /// The `auditor` of a [`FiscalAuditBinding`], under its `authority_basis`.
    Auditor,
    /// A person a [`FiscalAppointmentPolicy`] lists by name as a nominator or
    /// petitioner, acting under that policy: the issuer's `binding_id` is the
    /// policy's ID. Only a nomination or removal petition admits this role.
    ListedActor,
}

/// Who issued a settled fiscal action, under which binding and role.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalActionIssuer {
    /// The authority binding or audit binding the issuer acted under, or the
    /// appointment policy a [`FiscalIssuerRole::ListedActor`] acted under.
    pub binding_id: String,
    pub role: FiscalIssuerRole,
    /// The person who issued the action. `None` when an institution decided
    /// with no responsible actor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<PersonId>,
    /// The exact authority basis an acting actor or auditor was admitted
    /// under; `None` for the other roles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority_basis: Option<DomainRecordVersionRef>,
}

/// A batch of fiscal actions issued under one binding: the payload of one
/// `apply_fiscal_action_v1` command.
///
/// The issuer and its role are resolved once, at admission, for the binding.
/// Each entry is an ordinary action: its own target record, staleness check,
/// validation, outcome, and event. Entries settle in order; an entry settled
/// after a rebind that took the issuer's role away is rejected with
/// [`FISCAL_ISSUER_NOT_BOUND`]. A batch may mix entries on different scopes
/// of one institution and any action kinds the role admits.
///
/// Admission refuses the whole command for what concerns the whole (its
/// size, the binding, the issuer) and for an entry it can already refuse (an
/// unknown target, a target of another institution, an expired or unknown
/// revision, a known action ID, an ID named twice, or bounds): the refusal's
/// message begins with `entry N:` and the entry's reason, so a host can fix
/// or drop that entry and send the batch again.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FiscalActionBatch {
    /// The binding the issuer acts under: an authority binding of the
    /// institution the actions concern, or, for [`FiscalAction::RecordAudit`]
    /// only, an audit binding on that institution. For the appointment actions
    /// it is an authority binding of a party the governing
    /// [`FiscalAppointmentPolicy`] lists, or, for a person the policy lists by
    /// name, the policy's own ID.
    pub authority_binding_id: String,
    /// From 1 to [`MAX_FISCAL_BATCH_ENTRIES`] entries.
    pub entries: Vec<FiscalActionEntry>,
}

impl FiscalActionBatch {
    /// A batch of one entry: the 0.15.0 single action.
    #[must_use]
    pub fn single(authority_binding_id: impl Into<String>, entry: FiscalActionEntry) -> Self {
        Self {
            authority_binding_id: authority_binding_id.into(),
            entries: vec![entry],
        }
    }
}

/// One action of a [`FiscalActionBatch`].
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FiscalActionEntry {
    /// The action's ID, unique among the recent outcomes of its target (see
    /// [`FiscalActionTarget`]).
    pub action_id: String,
    /// The revision of the action's target record the issuer decided against
    /// (see [`crate::fiscal_target_revision`]). The action is stale, and
    /// settles as a `Rejected` outcome, unless this equals the target's
    /// revision at the start of the boundary that settles it. Entries admitted
    /// against the same revision of one target settle together, in admission
    /// order, each validated against the record the previous one left. A
    /// later entry is also stale when it would overwrite an adoption, an
    /// authority or audit binding, or an observer binding that an earlier
    /// action of the same boundary already changed.
    ///
    /// Only an outcome or a receipt on the target advances its revision, so
    /// an action is stale only when its own target changed since its issuer
    /// looked; activity on other scopes, offices, jurisdictions, observers,
    /// archive maintenance, and historical-context changes leave it current.
    ///
    /// Admission refuses, without consuming anything, a revision above the
    /// target's current one ([`FISCAL_REVISION_UNKNOWN`]) and one at or below
    /// the target's outcome floor ([`FISCAL_REVISION_EXPIRED`]): such a
    /// request can only be a re-send of an action whose outcome has left the
    /// target's [`FiscalOutcomeRing`].
    pub expected_revision: u64,
    pub action: FiscalAction,
}

/// One entry of a batch together with the binding it was issued under: what
/// the settlement and appointment rules read.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct FiscalActionRequest {
    pub(crate) action_id: String,
    pub(crate) authority_binding_id: String,
    pub(crate) expected_revision: u64,
    pub(crate) action: FiscalAction,
}

impl FiscalActionRequest {
    pub(crate) fn new(authority_binding_id: &str, entry: &FiscalActionEntry) -> Self {
        Self {
            action_id: entry.action_id.clone(),
            authority_binding_id: authority_binding_id.to_owned(),
            expected_revision: entry.expected_revision,
            action: entry.action.clone(),
        }
    }
}

/// The record an action decides on and keeps its outcome in: its target.
///
/// Every action has one target record, whose revision it names and whose
/// [`FiscalOutcomeRing`] keeps its recent outcome. Action IDs are per
/// target: two targets may use one action ID, and a host that wants its IDs
/// unique across targets provides that itself.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "type", content = "id", rename_all = "snake_case")]
pub enum FiscalActionTarget {
    /// The scope ledger of one scope binding: the target of the actions on a
    /// scope's adoptions and books.
    Ledger(String),
    /// The office of one institution: the target of the actions on its
    /// authority and audit bindings and its auditor proposals.
    Office(EntityRef),
    /// The jurisdiction of one catalog region: the target of the transitions
    /// applied in it.
    Jurisdiction(String),
    /// One observer binding: the target of re-pointing and removing it.
    Observer(String),
}

impl std::fmt::Display for FiscalActionTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ledger(scope) => write!(formatter, "ledger:{scope}"),
            Self::Office(institution) => {
                write!(formatter, "office:{}", fiscal_office_key(institution))
            }
            Self::Jurisdiction(region) => write!(formatter, "jurisdiction:{region}"),
            Self::Observer(observer) => write!(formatter, "observer:{observer}"),
        }
    }
}

/// The key of an institution's [`FiscalOffice`] in [`FiscalState::offices`]
/// and the identity of its office record: `office.` and the first 32
/// hexadecimal digits of a domain-separated BLAKE3 digest of the
/// institution's reference. The office holds its institution, and validation
/// requires the key to match it, so two institutions whose digests agree in
/// those digits are an invalid state, never a merged office.
#[must_use]
pub fn fiscal_office_key(institution: &EntityRef) -> String {
    let digest = canwu_api::canonical_hash("canwu.fiscal.office-key.v1", institution)
        .unwrap_or_else(|_| "0".repeat(64));
    format!("office.{}", &digest[..32])
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FiscalAction {
    ChangeAdoption {
        adoption_id: String,
        rule_id: String,
        scope_binding_id: String,
        stage: FiscalAdoptionStage,
    },
    ApplyTransition {
        transition_id: String,
        target_scope_bindings: BTreeMap<String, String>,
    },
    /// Opens an assessment in the book of `accounting_cycle_id` in the scope's
    /// ledger. The first assessment of a cycle creates its book, which the
    /// cycle may have only while it lies above both watermarks of the scope
    /// (the ledger's and its office's, see [`FiscalScopeLedger::watermark`]
    /// and [`FiscalOffice::watermark`]); otherwise the action is rejected with
    /// [`FISCAL_CYCLE_ARCHIVED`]. A closed book takes no assessment
    /// ([`FISCAL_CYCLE_CLOSED`]).
    OpenAssessment {
        assessment_id: String,
        rule_id: String,
        scope_binding_id: String,
        accounting_cycle_id: String,
        quantity: u64,
        unit: String,
        payment_form: FiscalPaymentForm,
        commutation_quote: Option<DomainRecordVersionRef>,
    },
    /// Remits part of an assessment of the scope, in an open book.
    GrantRemission {
        remission_id: String,
        scope_binding_id: String,
        assessment_id: String,
        quantity: u64,
        reason: String,
    },
    /// Authorizes an execution against an assessment of the scope, in an
    /// open book.
    AuthorizeExecution {
        request_id: String,
        scope_binding_id: String,
        assessment_id: String,
        kind: FiscalExecutionKind,
        quantity: u64,
        unit: String,
        resource: ResourceId,
        source: EntityRef,
        target: EntityRef,
        purpose: String,
    },
    /// Records an audit finding on an assessment, execution request, or
    /// execution receipt of the scope. It is admitted under an authority
    /// binding of the institution that owns the scope, or under an audit
    /// binding on that institution; the settled finding names its issuer in
    /// [`FiscalAuditFinding::entered_by`]. A closed book still takes findings
    /// until it is archived.
    RecordAudit {
        audit_id: String,
        scope_binding_id: String,
        target_id: String,
        severity: FiscalAuditSeverity,
        finding: String,
        evidence: BTreeSet<EvidenceRef>,
    },
    /// Closes the open book of one accounting cycle of one scope. A closed
    /// book takes no assessment, remission, execution authorization, or
    /// receipt, and is a candidate for the fiscal archive.
    CloseAccountingCycle {
        scope_binding_id: String,
        accounting_cycle_id: String,
    },
    /// Re-points one observer binding to another actor or holder: with
    /// `actor` the holder must be that person, and without one it must be a
    /// government. The new holder gets the whole report at the next
    /// publication; the old one gets one last summary marked withdrawn, when
    /// it can still receive knowledge, and keeps what it was told.
    ///
    /// The issuer must hold the principal or institution role under an
    /// authority binding of the observer binding's
    /// [`FiscalObserverBinding::controlled_by`] institution. A binding with no
    /// controlling institution changes only with the scenario. Every person
    /// and government named must exist. What the binding may see is the
    /// scenario's and does not change.
    RebindObserver {
        observer_binding_id: String,
        actor: Option<PersonId>,
        knowledge_holder: KnowledgeHolderRef,
    },
    /// Removes one observer binding, under the same authority as
    /// [`Self::RebindObserver`]: its holder gets one last summary marked
    /// withdrawn, and nothing is published to the binding again. The binding
    /// stays in the state, marked removed, so its identity and report lines
    /// are never used again.
    RemoveObserver { observer_binding_id: String },
    /// Replaces the actors of one authority binding in a single commit: the
    /// previous acting actor and authority basis stop being admitted, and the
    /// new ones start, at the boundary that settles the action.
    ///
    /// `authority_binding_id` must be the binding the request is issued
    /// under. The issuer must be that binding's authorized actor, or a
    /// decision controller deciding as the binding's institution with no
    /// responsible actor ([`FiscalIssuerRole::Institution`]). An acting actor
    /// cannot rebind, directly or as the responsible actor of an institution
    /// decision, and neither can any other person.
    ///
    /// The institution role is the power to appoint: such a controller can
    /// name any actors, or none, and on a binding with no authorized actor it
    /// then acts as principal. A host registers a controller of that shape
    /// only for the body that appoints the binding's actors.
    ///
    /// Every person the action names must exist. `acting_actor` and
    /// `authority_basis` are set together, and the basis must be the current
    /// version of an application record whose kind the plugin was configured
    /// to read (see [`crate::FiscalPlugin::with_authority_basis_kinds`]), when
    /// the command is admitted and again at the start of the boundary that
    /// settles the action. A basis whose record advances inside the settling
    /// boundary is still installed; the acting actor is then rejected at the
    /// next admission. The binding names the action and its issuer in
    /// [`FiscalAuthorityBinding::rebound_by`].
    RebindAuthority {
        authority_binding_id: String,
        authorized_actor: Option<PersonId>,
        acting_actor: Option<PersonId>,
        authority_basis: Option<DomainRecordVersionRef>,
    },
    /// Installs, replaces, or removes one audit binding on the issuer's own
    /// institution, where that institution appoints its own auditors. With
    /// `auditor` and `authority_basis` both set it installs the binding or
    /// replaces its auditor and basis; with both `None` it removes the
    /// binding.
    ///
    /// The issuer must hold the principal or institution role under an
    /// authority binding of the audited institution, the auditor must exist,
    /// and the basis must be current as for [`Self::RebindAuthority`]. An
    /// audit binding of another institution cannot be changed. The
    /// [`FiscalAppointmentPolicy`] of the audited institution must name that
    /// institution as its own appointer, and must let the appointer appoint
    /// without a nomination (to install), remove without a petition (to
    /// remove), or both (to replace the seated auditor by another): the
    /// action is then the appointer's decision
    /// ([`Self::DecideAuditAppointment`] with
    /// [`FiscalAppointmentDecision::Appoint`] or
    /// [`FiscalAppointmentDecision::Remove`]) and has the same effects.
    /// Anywhere else it is refused, and the policy's appointer decides.
    RebindAudit {
        audit_binding_id: String,
        auditor: Option<PersonId>,
        authority_basis: Option<DomainRecordVersionRef>,
    },
    /// Nominates `candidate`, under `authority_basis`, as the auditor of the
    /// audit binding `audit_binding_id` on `institution`, which need not
    /// exist yet. The nomination stays pending, as a
    /// [`FiscalAuditProposal`] with ID `nomination_id`, until the appointer
    /// decides on it or it is superseded, or the same party's next
    /// nomination for the binding replaces it. A party has at most
    /// [`MAX_FISCAL_PENDING_PROPOSALS_PER_PARTY`] proposals pending.
    ///
    /// The issuer must be a nominator of the institution's
    /// [`FiscalAppointmentPolicy`]: an institution it lists, acting under one
    /// of its authority bindings as its principal or as a body, or a person
    /// it lists, acting under the policy's ID. An acting actor never
    /// nominates. The candidate must exist, the basis must be current as for
    /// [`Self::RebindAuthority`], and an existing binding of that ID must be
    /// on `institution`. All of this is checked when the command is admitted
    /// and again when the action settles.
    NominateAuditor {
        nomination_id: String,
        audit_binding_id: String,
        institution: EntityRef,
        candidate: PersonId,
        authority_basis: DomainRecordVersionRef,
    },
    /// Petitions the appointer to remove the auditor of the existing audit
    /// binding `audit_binding_id`, on `grounds` (at least one evidence
    /// reference). The petition names the auditor the binding has when it
    /// settles and stays pending, as a [`FiscalAuditProposal`] with ID
    /// `petition_id`, until the appointer decides on it or it is superseded,
    /// or the same party's next petition on the binding replaces it.
    ///
    /// The issuer must be a petitioner of the audited institution's
    /// [`FiscalAppointmentPolicy`], as for a nominator in
    /// [`Self::NominateAuditor`].
    PetitionAuditorRemoval {
        petition_id: String,
        audit_binding_id: String,
        grounds: BTreeSet<EvidenceRef>,
    },
    /// The appointer's decision on the audit binding `audit_binding_id`.
    ///
    /// The issuer must hold the principal or institution role under an
    /// authority binding of the appointer that the audited institution's
    /// [`FiscalAppointmentPolicy`] names; an acting actor never decides. The
    /// binding records the decision in `rebound_by` and the nomination it
    /// accepted, if any, in [`FiscalAuditBinding::nomination`]. See
    /// [`FiscalAppointmentDecision`] for the decisions and for what each
    /// resolves.
    DecideAuditAppointment {
        audit_binding_id: String,
        decision: FiscalAppointmentDecision,
    },
}

/// What an appointer decides about one audit binding.
///
/// An appointment (of a nominee or not) fills a vacant binding, or appoints
/// its seated auditor again, as the policy's appointment rule allows.
/// Replacing a seated auditor by another is also a removal: the policy must
/// let the appointer remove without a petition
/// ([`FISCAL_PETITION_REQUIRED`] otherwise), and an auditor a petition is
/// pending against is otherwise removed by accepting it first. An
/// appointment resolves every other nomination pending on the binding as
/// superseded, and the petitions pending on it only when it replaces the
/// auditor they name. A removal (on a petition or not) removes the binding
/// and resolves every other petition pending on it as superseded;
/// nominations stay pending, since they propose someone for the vacancy.
/// Only the audited institution's proposals are superseded: a binding ID
/// free on one institution can be nominated for on another. A cited
/// nomination or petition must be pending on the decided binding
/// ([`FISCAL_PROPOSAL_NOT_PENDING`] otherwise), and it is resolved as
/// accepted, or as dismissed.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum FiscalAppointmentDecision {
    /// Appoints the candidate of the pending nomination `nomination_id`,
    /// under the basis it named, which must still be current.
    AcceptNomination { nomination_id: String },
    /// Appoints `auditor` under `authority_basis` without a nomination, which
    /// the policy must allow ([`FISCAL_NOMINATION_REQUIRED`] otherwise).
    /// `institution` is the audited institution: the policy's, and an
    /// existing binding's.
    Appoint {
        institution: EntityRef,
        auditor: PersonId,
        authority_basis: DomainRecordVersionRef,
    },
    /// Removes the auditor the pending petition `petition_id` names, who
    /// still holds the binding.
    AcceptPetition { petition_id: String },
    /// Removes the binding's auditor without a petition, which the policy
    /// must allow ([`FISCAL_PETITION_REQUIRED`] otherwise).
    Remove,
    /// Dismisses the pending nomination or petition `proposal_id` and
    /// changes nothing else.
    Dismiss { proposal_id: String },
}

/// External evidence that settles one execution request of one scope.
///
/// Settlement records the receipt in the book that holds the request, or
/// refuses it with one `canwu.fiscal.ingress_rejected.v1` event whose summary
/// ends with the stable reason (see [`FISCAL_RECEIPT_REQUEST_UNKNOWN`] and the
/// constants after it); a refused receipt changes nothing and consumes
/// nothing, and the packets after it settle.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FiscalExecutionReceiptPacket {
    pub receipt_id: String,
    pub scope_binding_id: String,
    pub request_id: String,
    pub external_evidence: BTreeSet<DomainRecordVersionRef>,
}

/// What one external evidence record claims: the scope and request it
/// settles, and what was executed. A receipt that cites it settles only that
/// request of that scope.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalExecutionEvidence {
    pub id: String,
    pub external_operation_id: String,
    pub scope_binding_id: String,
    pub request_id: String,
    pub quantity: u64,
    pub unit: String,
    pub payment_form: FiscalPaymentForm,
    pub execution_kind: FiscalExecutionKind,
    pub resource: ResourceId,
    pub source: EntityRef,
    pub target: EntityRef,
    pub disposition: FiscalReceiptDisposition,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct FiscalExternalOperationRef {
    pub evidence_kind: DomainRecordKind,
    pub external_operation_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalHistoricalContextPacket {
    pub year: i32,
    pub mode: FiscalHistoricalMode,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct HistoricalYearWindow {
    pub start: i32,
    pub end: i32,
}

impl HistoricalYearWindow {
    #[must_use]
    pub const fn contains(&self, year: i32) -> bool {
        self.start <= year && year <= self.end
    }

    fn validate(&self, label: &str) -> Result<(), CanwuError> {
        if self.start > self.end {
            return Err(invalid(format!(
                "{label} starts after it ends: {} > {}",
                self.start, self.end
            )));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalPackManifest {
    pub pack_id: String,
    pub pack_version: String,
    pub schema_version: u32,
    pub license: String,
    pub title: String,
    pub historical_scope: HistoricalYearWindow,
    pub period_ids: Vec<String>,
    pub region_ids: Vec<String>,
    pub mechanisms: Vec<FiscalMechanism>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalPeriodDefinition {
    pub id: String,
    pub label: String,
    pub window: HistoricalYearWindow,
    #[serde(default)]
    pub branch: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalRegionDefinition {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalInstitutionDefinition {
    pub id: String,
    pub label: String,
    pub institution_kind: String,
    pub region_ids: BTreeSet<String>,
    pub provenance_ids: BTreeSet<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalRuleDefinition {
    pub id: String,
    pub revision: u32,
    pub label: String,
    pub mechanism: FiscalMechanism,
    pub legal_window: HistoricalYearWindow,
    pub jurisdiction_ids: BTreeSet<String>,
    pub subject_scope: String,
    pub assessment_basis: FiscalAssessmentBasis,
    pub payment_forms: BTreeSet<FiscalPaymentForm>,
    pub commutation: FiscalCommutationPolicy,
    pub earmark_ids: BTreeSet<String>,
    pub provenance_ids: BTreeSet<String>,
    pub confidence: HistoricalConfidence,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalTransitionDefinition {
    pub id: String,
    pub label: String,
    pub from_rule_ids: BTreeSet<String>,
    pub to_rule_ids: BTreeSet<String>,
    pub observed_window: HistoricalYearWindow,
    pub eligibility_window: HistoricalYearWindow,
    pub jurisdiction_ids: BTreeSet<String>,
    pub supersedes_or_suspends: BTreeSet<String>,
    pub prerequisite_ids: BTreeSet<String>,
    pub provenance_ids: BTreeSet<String>,
    pub confidence: HistoricalConfidence,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalCoverageSelector {
    #[serde(default)]
    pub period_ids: BTreeSet<String>,
    #[serde(default)]
    pub region_ids: BTreeSet<String>,
    #[serde(default)]
    pub mechanisms: BTreeSet<FiscalMechanism>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalCoverageDeclaration {
    pub id: String,
    pub priority: u16,
    pub selector: FiscalCoverageSelector,
    pub status: FiscalCoverageStatus,
    pub definition_ids: BTreeSet<String>,
    pub provenance_ids: BTreeSet<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalCoverageCell {
    pub id: String,
    pub period_id: String,
    pub region_id: String,
    pub mechanism: FiscalMechanism,
    pub status: FiscalCoverageStatus,
    pub definition_ids: BTreeSet<String>,
    pub provenance_ids: BTreeSet<String>,
    pub declaration_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalProvenance {
    pub id: String,
    pub citation: String,
    pub url: String,
    pub claim_scope: String,
    pub confidence: HistoricalConfidence,
    pub forbidden_inferences: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalContentPack {
    pub manifest: FiscalPackManifest,
    pub periods: Vec<FiscalPeriodDefinition>,
    pub regions: Vec<FiscalRegionDefinition>,
    pub institutions: Vec<FiscalInstitutionDefinition>,
    pub rules: Vec<FiscalRuleDefinition>,
    pub transitions: Vec<FiscalTransitionDefinition>,
    pub coverage: Vec<FiscalCoverageDeclaration>,
    pub provenance: Vec<FiscalProvenance>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalContentSelection {
    pub historical_year: i32,
    pub region_ids: BTreeSet<String>,
    pub mechanisms: BTreeSet<FiscalMechanism>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompiledFiscalCatalog {
    pub schema_version: u32,
    pub pack_id: String,
    pub pack_version: String,
    pub content_hash: String,
    pub historical_year: i32,
    pub historical_scope: HistoricalYearWindow,
    pub selected_period_ids: BTreeSet<String>,
    pub periods: BTreeMap<String, FiscalPeriodDefinition>,
    pub regions: BTreeMap<String, FiscalRegionDefinition>,
    pub institutions: BTreeMap<String, FiscalInstitutionDefinition>,
    pub rules: BTreeMap<String, FiscalRuleDefinition>,
    pub transitions: BTreeMap<String, FiscalTransitionDefinition>,
    pub coverage: BTreeMap<String, FiscalCoverageCell>,
    pub provenance: BTreeMap<String, FiscalProvenance>,
}

impl CompiledFiscalCatalog {
    #[allow(clippy::too_many_lines)]
    pub fn validate(&self) -> Result<(), CanwuError> {
        if self.schema_version != FISCAL_CONTENT_SCHEMA_VERSION {
            return Err(invalid("unsupported compiled fiscal catalog schema"));
        }
        validate_identifier(&self.pack_id, "pack")?;
        if self.content_hash.len() != 64
            || !self
                .content_hash
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(invalid("fiscal content hash is not a 64-character digest"));
        }
        if self.selected_period_ids.is_empty() || self.regions.is_empty() {
            return Err(invalid("compiled fiscal catalog selection is empty"));
        }
        if self.periods.len() > MAX_FISCAL_CATALOG_PERIODS
            || self.regions.len() > MAX_FISCAL_CATALOG_REGIONS
            || self.institutions.len() > MAX_FISCAL_CATALOG_DEFINITIONS
            || self.rules.len() > MAX_FISCAL_CATALOG_DEFINITIONS
            || self.transitions.len() > MAX_FISCAL_CATALOG_DEFINITIONS
            || self.provenance.len() > MAX_FISCAL_CATALOG_DEFINITIONS
            || self.coverage.len() > MAX_FISCAL_CATALOG_COVERAGE_CELLS
        {
            return Err(invalid(
                "compiled fiscal catalog exceeds its bounded capacity",
            ));
        }
        if self.selected_period_ids.len() > MAX_FISCAL_CATALOG_PERIODS
            || self.institutions.values().any(|value| {
                value.region_ids.len() > MAX_FISCAL_REFERENCES_PER_DEFINITION
                    || value.provenance_ids.len() > MAX_FISCAL_REFERENCES_PER_DEFINITION
            })
            || self.rules.values().any(|value| {
                value.jurisdiction_ids.len() > MAX_FISCAL_REFERENCES_PER_DEFINITION
                    || value.earmark_ids.len() > MAX_FISCAL_REFERENCES_PER_DEFINITION
                    || value.provenance_ids.len() > MAX_FISCAL_REFERENCES_PER_DEFINITION
            })
            || self.transitions.values().any(|value| {
                value.from_rule_ids.len() > MAX_FISCAL_REFERENCES_PER_DEFINITION
                    || value.to_rule_ids.len() > MAX_FISCAL_REFERENCES_PER_DEFINITION
                    || value.jurisdiction_ids.len() > MAX_FISCAL_REFERENCES_PER_DEFINITION
                    || value.supersedes_or_suspends.len() > MAX_FISCAL_REFERENCES_PER_DEFINITION
                    || value.prerequisite_ids.len() > MAX_FISCAL_REFERENCES_PER_DEFINITION
                    || value.provenance_ids.len() > MAX_FISCAL_REFERENCES_PER_DEFINITION
            })
            || self.coverage.values().any(|value| {
                value.definition_ids.len() > MAX_FISCAL_REFERENCES_PER_DEFINITION
                    || value.provenance_ids.len() > MAX_FISCAL_REFERENCES_PER_DEFINITION
            })
            || self.provenance.values().any(|value| {
                value.forbidden_inferences.len() > MAX_FISCAL_REFERENCES_PER_DEFINITION
            })
        {
            return Err(invalid(
                "compiled fiscal catalog nested collection exceeds its bounded capacity",
            ));
        }
        let transition_scope_count = self.transitions.values().fold(0_usize, |count, value| {
            count.saturating_add(value.jurisdiction_ids.len())
        });
        let encoded_len = serde_json::to_vec(self)
            .map_err(|error| invalid(format!("fiscal catalog could not be sized: {error}")))?
            .len();
        if transition_scope_count > MAX_FISCAL_CATALOG_COVERAGE_CELLS
            || encoded_len > MAX_FISCAL_CATALOG_JSON_BYTES
        {
            return Err(invalid(
                "compiled fiscal catalog exceeds its transition or byte budget",
            ));
        }
        self.historical_scope
            .validate("compiled fiscal catalog historical scope")?;
        if !self.historical_scope.contains(self.historical_year) {
            return Err(invalid(
                "initial fiscal year lies outside the catalog scope",
            ));
        }
        validate_map(&self.periods, |value| &value.id, "period")?;
        validate_refs(&self.selected_period_ids, &self.periods, "selected period")?;
        if self
            .selected_period_ids
            .iter()
            .any(|id| !self.periods[id].window.contains(self.historical_year))
        {
            return Err(invalid(
                "selected fiscal period does not cover the initial year",
            ));
        }
        validate_map(&self.regions, |value| &value.id, "region")?;
        validate_map(&self.institutions, |value| &value.id, "institution")?;
        validate_map(&self.rules, |value| &value.id, "rule")?;
        validate_map(&self.transitions, |value| &value.id, "transition")?;
        validate_map(&self.coverage, |value| &value.id, "coverage cell")?;
        validate_map(&self.provenance, |value| &value.id, "provenance")?;
        for rule in self.rules.values() {
            rule.legal_window.validate("fiscal rule legal window")?;
            if rule.revision == 0 || rule.payment_forms.is_empty() {
                return Err(invalid(format!(
                    "fiscal rule {} requires a revision and payment form",
                    rule.id
                )));
            }
            validate_refs(&rule.jurisdiction_ids, &self.regions, "rule region")?;
            validate_refs(&rule.provenance_ids, &self.provenance, "rule provenance")?;
        }
        for transition in self.transitions.values() {
            transition
                .observed_window
                .validate("transition observed window")?;
            transition
                .eligibility_window
                .validate("transition eligibility window")?;
            validate_refs(
                &transition.from_rule_ids,
                &self.rules,
                "transition source rule",
            )?;
            validate_refs(
                &transition.to_rule_ids,
                &self.rules,
                "transition target rule",
            )?;
            validate_refs(
                &transition.supersedes_or_suspends,
                &self.rules,
                "transition superseded rule",
            )?;
            validate_refs(
                &transition.prerequisite_ids,
                &self.transitions,
                "transition prerequisite",
            )?;
            validate_refs(
                &transition.provenance_ids,
                &self.provenance,
                "transition provenance",
            )?;
        }
        for cell in self.coverage.values() {
            if !self.periods.contains_key(&cell.period_id)
                || !self.regions.contains_key(&cell.region_id)
            {
                return Err(invalid(format!(
                    "coverage cell {} escaped its compiled selection",
                    cell.id
                )));
            }
            for definition in &cell.definition_ids {
                if !self.rules.contains_key(definition)
                    && !self.institutions.contains_key(definition)
                    && !self.transitions.contains_key(definition)
                {
                    return Err(invalid(format!(
                        "coverage cell {} references unknown definition {definition}",
                        cell.id
                    )));
                }
            }
            let permits_comparative_definition =
                cell.status == FiscalCoverageStatus::ArchetypeFallback;
            if matches!(
                cell.status,
                FiscalCoverageStatus::Supported | FiscalCoverageStatus::ArchetypeFallback
            ) && !cell.definition_ids.iter().any(|definition| {
                self.rules.get(definition).is_some_and(|rule| {
                    rule.mechanism == cell.mechanism
                        && (permits_comparative_definition
                            || rule.jurisdiction_ids.contains(&cell.region_id))
                }) || self.transitions.get(definition).is_some_and(|transition| {
                    (permits_comparative_definition
                        || transition.jurisdiction_ids.contains(&cell.region_id))
                        && transition
                            .from_rule_ids
                            .iter()
                            .chain(&transition.to_rule_ids)
                            .any(|rule_id| {
                                self.rules
                                    .get(rule_id)
                                    .is_some_and(|rule| rule.mechanism == cell.mechanism)
                            })
                })
            }) {
                return Err(invalid(format!(
                    "coverage cell {} has no mechanism-specific definition",
                    cell.id
                )));
            }
            validate_refs(
                &cell.provenance_ids,
                &self.provenance,
                "coverage provenance",
            )?;
        }
        Ok(())
    }

    #[must_use]
    pub fn active_period_ids(&self, year: i32) -> BTreeSet<String> {
        self.periods
            .values()
            .filter(|period| period.window.contains(year))
            .map(|period| period.id.clone())
            .collect()
    }

    pub fn into_record(self) -> Result<DomainRecord, CanwuError> {
        self.validate()?;
        let draft = DomainRecordDraft::from_typed(fiscal_catalog_reference(), &self)?;
        Ok(DomainRecord {
            reference: draft.reference,
            owner: PLUGIN_NAME.to_owned(),
            class: DomainRecordClass::Record,
            version: 1,
            lifecycle: DomainRecordLifecycle::Active,
            payload: draft.payload,
            references: Vec::new(),
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalHistoricalContext {
    pub year: i32,
    pub mode: FiscalHistoricalMode,
    pub updated_at: SimTime,
}

/// Stable rejection message used when an acting actor's authority basis is
/// missing or no longer the current version of its record.
pub const FISCAL_ACTING_BASIS_NOT_CURRENT: &str =
    "fiscal acting actor authority basis is not the current record version";

/// Stable rejection message used when the authority basis a rebind action
/// installs, or the basis of an audit binding, is not the current version of
/// its record.
pub const FISCAL_AUTHORITY_BASIS_NOT_CURRENT: &str =
    "fiscal authority basis is not the current record version";

/// Stable rejection message used when a rebind that settled after an action
/// was admitted took its issuer's role on the binding away.
pub const FISCAL_ISSUER_NOT_BOUND: &str =
    "fiscal action issuer no longer holds the binding it was admitted under";

/// Stable rejection message used when no [`FiscalAppointmentPolicy`]
/// governs the auditors of the institution an appointment action concerns.
/// Without one, nobody appoints or removes them at runtime.
pub const FISCAL_APPOINTMENT_POLICY_MISSING: &str =
    "no appointment policy governs the auditors of this institution";

/// Stable rejection message used when the issuer of an appointer's decision,
/// or of a `RebindAudit`, does not act for the appointer the policy names.
pub const FISCAL_ISSUER_NOT_APPOINTER: &str =
    "the issuer does not act for the appointer of this institution's auditors";

/// Stable rejection message used when the issuer of a nomination or removal
/// petition is not a party the policy lists for it.
pub const FISCAL_ISSUER_NOT_LISTED: &str =
    "the appointment policy does not list the issuer for this proposal";

/// Stable rejection message used when the appointer appoints without a
/// nomination under a policy that requires one.
pub const FISCAL_NOMINATION_REQUIRED: &str =
    "the appointment policy requires a nomination before an appointment";

/// Stable rejection message used when the appointer removes an auditor
/// without a petition under a policy that requires one.
pub const FISCAL_PETITION_REQUIRED: &str =
    "the appointment policy requires a removal petition before a removal";

/// Stable rejection message used when a decision cites a nomination or
/// petition that is not pending on the audit binding it decides.
pub const FISCAL_PROPOSAL_NOT_PENDING: &str =
    "the cited nomination or petition is not pending on this audit binding";

/// Stable rejection message used for an action that is current and
/// authorized in a boundary that already applied
/// [`MAX_FISCAL_ACTIONS_PER_BOUNDARY`] actions, or already wrote
/// [`MAX_FISCAL_LEDGER_WRITES_PER_BOUNDARY`] ledgers and would write another.
pub const FISCAL_BOUNDARY_ACTION_LIMIT: &str =
    "fiscal action exceeds the per-boundary action limit";

/// Stable rejection message used for an action that is current and
/// authorized in a boundary that already applied
/// [`MAX_FISCAL_ACTIONS_PER_RECORD_PER_BOUNDARY`] actions to its target.
pub const FISCAL_RECORD_ACTION_LIMIT: &str =
    "fiscal action exceeds the per-record action limit of its boundary";

/// Stable rejection message used for an action whose result would not fit
/// the byte budget of a record it writes (see
/// [`MAX_FISCAL_LEDGER_RECORD_BYTES`] and [`MAX_FISCAL_RECORD_BYTES`]), less
/// [`FISCAL_RECORD_RING_RESERVE_BYTES`].
pub const FISCAL_RECORD_BUDGET_EXHAUSTED: &str =
    "fiscal action would exceed the byte budget of a fiscal record";

/// Stable rejection message used for an assessment that would give one
/// institution's books of one cycle and mechanism a partition key (unit and
/// payment form) past [`MAX_FISCAL_PARTITION_KEYS`].
pub const FISCAL_PARTITION_KEYS_EXHAUSTED: &str =
    "fiscal assessment would use more partition keys than an institution's cycle may";

/// Reason that ends the summary of a `canwu.fiscal.ingress_rejected.v1`
/// event for an execution receipt or a historical-context change whose
/// result would not fit the byte budget of a record it writes. The packet
/// changes nothing: a receipt consumes neither its evidence versions nor its
/// external operations, and no revision advances.
pub const FISCAL_INGRESS_BUDGET_REJECTION: &str = "state_budget_exhausted";

/// Reason that ends the summary of a `canwu.fiscal.ingress_rejected.v1`
/// event for an execution receipt that would write a ledger past
/// [`MAX_FISCAL_LEDGER_WRITES_PER_BOUNDARY`]. It changes nothing; a host sends
/// it again.
pub const FISCAL_RECEIPT_BOUNDARY_LIMIT: &str = "boundary_limit";

/// Reason that ends the summary of a `canwu.fiscal.ingress_rejected.v1`
/// event for a fiscal packet whose payload does not decode: an execution
/// receipt or historical-context change a host sent malformed, or an action
/// packet, which only a boundary system of another plugin that declares the
/// action ingress as a target can produce. The packet changes nothing.
pub const FISCAL_INGRESS_MALFORMED_REJECTION: &str = "malformed_payload";

/// Reason that ends the summary of a `canwu.fiscal.ingress_rejected.v1`
/// event for an execution receipt or historical-context change that
/// decodes and passes the checks that have a reason of their own (see
/// [`FISCAL_RECEIPT_REQUEST_UNKNOWN`] and the constants after it, and
/// [`FISCAL_CONTEXT_OUTSIDE_PACK`]) but whose result still does not validate,
/// after the message that says why. The packet changes nothing: a receipt
/// consumes neither its evidence versions nor its external operations, and
/// no revision advances.
pub const FISCAL_INGRESS_INVALID_REJECTION: &str = "invalid_packet";

/// Reason that ends the summary of a `canwu.fiscal.ingress_rejected.v1`
/// event for a fiscal action packet whose ingress record does not name, as
/// its cause, the command the packet carries. Only the admission of an
/// `apply_fiscal_action_v1` command produces a packet with that cause, so
/// such a packet was scheduled by a boundary system of another plugin; it
/// changes nothing and its action ID is not consumed.
pub const FISCAL_INGRESS_ORIGIN_REJECTION: &str = "unauthorized_origin";

/// Reason a receipt is refused when its scope holds no hot execution request
/// with its request ID: the scope or request does not exist, or the request's
/// book was archived.
pub const FISCAL_RECEIPT_REQUEST_UNKNOWN: &str = "request_unknown";
/// Reason a receipt is refused when its request's book is closed.
pub const FISCAL_RECEIPT_CYCLE_CLOSED: &str = "cycle_closed";
/// Reason a receipt is refused when it cites no evidence, or more than
/// [`MAX_FISCAL_EVIDENCE_PER_RECORD`] versions.
pub const FISCAL_RECEIPT_EVIDENCE_COUNT: &str = "evidence_count_invalid";
/// Reason a receipt is refused when it cites a record kind the state does not
/// approve as execution evidence.
pub const FISCAL_RECEIPT_EVIDENCE_KIND_UNAPPROVED: &str = "evidence_kind_unapproved";
/// Reason a receipt is refused when an evidence version it cites is not the
/// current version of its record.
pub const FISCAL_RECEIPT_EVIDENCE_NOT_CURRENT: &str = "evidence_not_current";
/// Reason a receipt is refused when an evidence claim does not match the
/// receipt's scope and request, or the claims of one receipt differ in their
/// disposition, or a claim predates its request.
pub const FISCAL_RECEIPT_CLAIM_MISMATCH: &str = "claim_mismatch";
/// Reason a receipt is refused when it would fulfil more than its request
/// authorizes.
pub const FISCAL_RECEIPT_QUANTITY_EXCEEDED: &str = "quantity_exceeded";
/// Reason a receipt is refused when an external operation it cites already
/// settled a receipt of its scope's hot books, or repeats within it.
pub const FISCAL_RECEIPT_OPERATION_REUSED: &str = "operation_reused";
/// Reason a receipt is refused when an evidence version it cites already
/// settled a receipt of its scope's hot books.
pub const FISCAL_RECEIPT_EVIDENCE_REUSED: &str = "evidence_reused";
/// Reason a receipt is refused when a hot receipt of its scope has its ID
/// with other content.
pub const FISCAL_RECEIPT_ID_CONFLICT: &str = "receipt_id_conflict";
/// Reason a historical-context change is refused when its year lies outside
/// the catalog's historical scope.
pub const FISCAL_CONTEXT_OUTSIDE_PACK: &str = "context_outside_pack";

/// Stable rejection message used at admission for an entry whose
/// `expected_revision` is above its target record's current revision: it
/// was never a revision of the target.
pub const FISCAL_REVISION_UNKNOWN: &str = "fiscal action names a revision its target never had";
/// Stable rejection message used at admission for an entry whose
/// `expected_revision` is at or below its target's outcome floor: a re-send
/// of an action whose outcome left the target's ring.
pub const FISCAL_REVISION_EXPIRED: &str =
    "fiscal action names a revision at or below its target's outcome floor";
/// Stable rejection message used for an assessment, remission, or execution
/// authorization on a closed book.
pub const FISCAL_CYCLE_CLOSED: &str = "fiscal accounting cycle book is closed";
/// Stable rejection message used for a new book whose accounting cycle is at
/// or below its scope's watermark or its office's.
pub const FISCAL_CYCLE_ARCHIVED: &str =
    "fiscal accounting cycle is at or below an archived cycle of its scope or institution";
/// Stable rejection message used for a new book in a ledger that holds
/// [`MAX_FISCAL_HOT_BOOKS_PER_LEDGER`] books.
pub const FISCAL_HOT_BOOKS_EXHAUSTED: &str =
    "fiscal scope ledger holds as many accounting cycle books as it may";
/// Stable rejection message used for an action on a removed observer
/// binding.
pub const FISCAL_OBSERVER_REMOVED: &str = "fiscal observer binding was removed";

/// The settled action that last replaced a binding's actors, and its issuer.
///
/// Appointments use the same record for the action that made or resolved a
/// nomination or petition ([`FiscalAuditProposal`]); `rebound_at` is then
/// the time that action settled.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalRebinding {
    /// The rebind action; its settled event ([`crate::FiscalSettledAction`])
    /// records its outcome.
    pub action_id: String,
    pub issuer: FiscalActionIssuer,
    pub rebound_at: SimTime,
}

/// Actors who may issue fiscal procedure actions for one institution.
///
/// `authorized_actor` is the standing principal. `acting_actor` is an optional
/// second actor who is admitted only while the exact `authority_basis` record
/// version is still the current version of that record; once the basis record
/// advances or retires, the acting actor is rejected with
/// [`FISCAL_ACTING_BASIS_NOT_CURRENT`] while the principal remains admitted.
/// [`FiscalAction::RebindAuthority`] replaces the three fields together.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalAuthorityBinding {
    pub id: String,
    pub institution: EntityRef,
    pub authorized_actor: Option<PersonId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acting_actor: Option<PersonId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority_basis: Option<DomainRecordVersionRef>,
    /// Who last rebound this binding, by which action. `None` while the
    /// binding still holds the actors it was installed with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rebound_by: Option<FiscalRebinding>,
}

/// One auditor from outside an institution who may record audit findings on
/// that institution's scopes.
///
/// The auditor is admitted for [`FiscalAction::RecordAudit`] only, on targets
/// whose scope the audited `institution` owns, and only while the exact
/// `authority_basis` record version is the current version of its record;
/// otherwise the action is rejected with
/// [`FISCAL_AUTHORITY_BASIS_NOT_CURRENT`]. An audit binding grants no other
/// fiscal action and is never consulted when an authority binding is checked.
/// It is installed with the state or by the appointer of the audited
/// institution's [`FiscalAppointmentPolicy`], and its ID must differ from
/// every authority binding ID and appointment policy ID. Audit binding IDs are
/// one namespace across all institutions.
///
/// Who installs and removes the binding at runtime is the policy's to say: an
/// appointer, which may need a nomination or a removal petition first. The
/// binding is independent of the audited institution exactly as far as the
/// policy makes it: an institution that is its own appointer installs,
/// replaces, and removes its own auditors, and can remove one before a
/// finding admitted under it settles, so its bindings establish attribution
/// only; an institution with no policy has only the bindings installed with
/// the state. The basis only has to be the current version of some
/// application record the plugin reads; what that record says about the
/// auditor is the application's to decide. A basis named here, in
/// [`FiscalAuditFinding::entered_by`], in a nomination, or in `rebound_by` is
/// not declared as required evidence: no decision reads it again, and after
/// `seal_evidence` a superseded version may no longer be provable.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalAuditBinding {
    pub id: String,
    /// The audited institution.
    pub institution: EntityRef,
    pub auditor: PersonId,
    pub authority_basis: DomainRecordVersionRef,
    /// Who decided to install or last replace this binding at runtime, by
    /// which action: the appointer's decision. `None` for a binding installed
    /// with the state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rebound_by: Option<FiscalRebinding>,
    /// The nomination that decision accepted, and its nominator. `None` for
    /// an auditor appointed without a nomination or installed with the state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nomination: Option<FiscalProposalRef>,
}

/// Who appoints and removes the auditors of one institution, and who may ask
/// for an appointment or a removal.
///
/// A policy is declared with the state, at most one per audited
/// `institution`, and no action changes it. The `appointer` is an institution:
/// it decides under one of its authority bindings, as that binding's
/// principal or as a body ([`FiscalIssuerRole::Principal`] or
/// [`FiscalIssuerRole::Institution`]), never through an acting actor. The
/// audited institution is its own appointer only where the policy names it
/// so; otherwise it cannot install, replace, or remove its own auditors.
///
/// Nominators propose a candidate ([`FiscalAction::NominateAuditor`]) and
/// petitioners ask for an auditor's removal on grounds
/// ([`FiscalAction::PetitionAuditorRemoval`]). Whether the appointer may act
/// without either is the policy's to say: `removes_without_petition`
/// governs every removal of a seated auditor, including his replacement by
/// another. Each proposal is recorded with its party in
/// [`FiscalState::audit_proposals`]; a party has at most
/// [`MAX_FISCAL_PENDING_PROPOSALS_PER_PARTY`] pending, and the policy keeps
/// its latest [`MAX_FISCAL_RESOLVED_PROPOSALS_PER_POLICY`] resolved ones.
///
/// The shape names no binding kind: the same policy could govern who
/// appoints and dismisses the actors of an authority binding.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalAppointmentPolicy {
    pub id: String,
    /// The institution whose audit bindings the policy governs.
    pub institution: EntityRef,
    /// The institution whose decision appoints and removes them.
    pub appointer: EntityRef,
    /// Who may nominate a candidate.
    pub nominators: BTreeSet<FiscalAppointmentParty>,
    /// Who may petition for an auditor's removal.
    pub petitioners: BTreeSet<FiscalAppointmentParty>,
    /// Whether the appointer may appoint without a nomination.
    pub appoints_without_nomination: bool,
    /// Whether the appointer may remove without a petition.
    pub removes_without_petition: bool,
}

/// A nominator or petitioner of a [`FiscalAppointmentPolicy`].
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FiscalAppointmentParty {
    /// An institution, acting under one of its authority bindings as that
    /// binding's principal or as a body.
    Institution { institution: EntityRef },
    /// A person, acting under the policy itself
    /// ([`FiscalIssuerRole::ListedActor`]): directly, or as the actor or
    /// responsible actor of a validated decision whose subject is the
    /// audited institution.
    Actor { actor: PersonId },
}

/// One nomination or removal petition for one audit binding.
///
/// A proposal is pending until it is resolved, once: the appointer accepts
/// or dismisses it, or a decision on its binding supersedes it as
/// [`FiscalAppointmentDecision`] says. A new proposal of the same kind by the
/// same party on the same binding replaces the party's pending one, which is
/// dropped. A resolved proposal stays, as the record of who proposed, on
/// what, and who decided, among the latest
/// [`MAX_FISCAL_RESOLVED_PROPOSALS_PER_POLICY`] of its policy.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalAuditProposal {
    pub id: String,
    pub audit_binding_id: String,
    /// The audited institution.
    pub institution: EntityRef,
    pub kind: FiscalAuditProposalKind,
    /// The listed party that made the proposal.
    pub party: FiscalAppointmentParty,
    /// The action that made the proposal, and its issuer.
    pub proposed_by: FiscalRebinding,
    /// How the proposal was resolved; `None` while it is pending.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<FiscalProposalResolution>,
}

impl FiscalAuditProposal {
    /// Whether the proposal still waits for a decision.
    #[must_use]
    pub fn is_pending(&self) -> bool {
        self.resolution.is_none()
    }
}

/// What a [`FiscalAuditProposal`] proposes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FiscalAuditProposalKind {
    /// `candidate`, under `authority_basis`, as the binding's auditor.
    Nomination {
        candidate: PersonId,
        authority_basis: DomainRecordVersionRef,
    },
    /// The removal of `auditor`, who held the binding when the petition
    /// settled, on `grounds`.
    RemovalPetition {
        auditor: PersonId,
        grounds: BTreeSet<EvidenceRef>,
    },
}

/// How a proposal was resolved, and by which settled action.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalProposalResolution {
    pub outcome: FiscalProposalOutcome,
    /// The appointer's decision that accepted, dismissed, or superseded the
    /// proposal, and its issuer.
    pub resolved_by: FiscalRebinding,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FiscalProposalOutcome {
    /// The appointer appointed the nominee, or removed the petitioned auditor.
    Accepted,
    /// The appointer dismissed it.
    Dismissed,
    /// A decision on the binding made it moot.
    Superseded,
}

/// A nomination or petition as a binding or finding cites it: its ID in
/// [`FiscalState::audit_proposals`], and who made it.
///
/// The record is kept only among the latest resolved proposals of its
/// policy, and once it is dropped or pruned its ID can be used again, so a
/// binding's or finding's `proposal_id` may later name a different
/// proposal. `proposed_by`, which the citing record carries itself, stays
/// authoritative for who nominated.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalProposalRef {
    pub proposal_id: String,
    pub proposed_by: FiscalActionIssuer,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalScopeBinding {
    pub id: String,
    pub institution: EntityRef,
    pub jurisdiction_id: String,
    pub subject_scope: String,
    pub mechanism: FiscalMechanism,
    pub authoritative_granularity: SimulationGranularity,
}

/// One holder who is told about some institutions, at some confidence.
///
/// Observer bindings come with the state. With an `actor` the holder is that
/// person; without one it is a government, which holds the report as an
/// institution. The holder must exist: the observer record references the
/// person or government, and the engine refuses a record that references an
/// entity its registry does not hold. While a person holder is dead, nothing
/// is published to the binding and what changes in its report waits in the
/// report queue.
///
/// The binding's report lines are named on its own observer record, so
/// re-pointing or removing one binding never moves another's lines. A binding
/// with `controlled_by` can be re-pointed ([`FiscalAction::RebindObserver`])
/// and removed ([`FiscalAction::RemoveObserver`]) by that institution; what it
/// may see, its granularity, and its cadence are the scenario's and do not
/// change. Set the granularity and cadence before
/// [`FiscalState::add_observer`], which queues the binding's first report at
/// its first due time.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalObserverBinding {
    pub id: String,
    pub actor: Option<PersonId>,
    pub knowledge_holder: KnowledgeHolderRef,
    /// At most [`MAX_FISCAL_VISIBLE_INSTITUTIONS`], each with an office, and
    /// at most [`MAX_FISCAL_OBSERVER_ROWS`] rows among them.
    pub visible_institutions: BTreeSet<EntityRef>,
    pub confidence_per_mille: u16,
    /// What one row of the report is: a scope (the default) or an
    /// institution and mechanism.
    #[serde(default, skip_serializing_if = "is_default")]
    pub granularity: FiscalReportGranularity,
    /// When the report is due: in the boundary that changes what the
    /// observer sees (the default), or at aligned interval times.
    #[serde(default, skip_serializing_if = "is_default")]
    pub cadence: FiscalReportCadence,
    /// The institution whose authority bindings may re-point or remove this
    /// binding; `None` when only the scenario changes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controlled_by: Option<EntityRef>,
    /// The action that last re-pointed the binding, and its issuer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rebound_by: Option<FiscalRebinding>,
    /// The action that removed the binding, and its issuer. A removed
    /// binding is never published to again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub removed: Option<FiscalRebinding>,
}

impl FiscalObserverBinding {
    /// Whether the binding is still published to.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.removed.is_none()
    }

    /// The cadence the binding is scheduled by: its own while it is active;
    /// a removed binding has no due times, so the withdrawal of its report
    /// is owed at once.
    #[must_use]
    pub fn scheduled_cadence(&self) -> FiscalReportCadence {
        if self.is_active() {
            self.cadence
        } else {
            FiscalReportCadence::OnChange
        }
    }
}

/// Whether a value is its type's default, for fields left out when default.
#[allow(clippy::trivially_copy_pass_by_ref)]
pub(crate) fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

/// What one row of an observer's report is.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FiscalReportGranularity {
    /// One row per scope of the visible institutions, the facts of its book.
    #[default]
    Scope,
    /// One row per visible institution and mechanism of its scopes, the
    /// facts summed over the institution's hot books of the cycle of that
    /// mechanism, per unit and payment form, each amount saturating at
    /// `u64::MAX`.
    Institution,
}

/// When an observer's report is due.
///
/// Due times are simulated times. An `Interval` report is published by the
/// first boundary at or after each due time that runs the report system,
/// with what changed since its last publication and its summary (at a due
/// time at which nothing changed, its summary alone, once the report has
/// been published at least once), and never between due times: its `as_of`
/// is the boundary that published it, and the report shows the fiscal
/// records as they were then. Calendar cadences
/// (lunar months, fiscal years) are not expressible, since `every` is a fixed
/// duration: a calendar host uses `OnChange`, an interval that approximates
/// its calendar, or rebinds.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FiscalReportCadence {
    /// Due in the boundary that changes something the observer sees.
    #[default]
    OnChange,
    /// Due at `offset + k * every` for every `k >= 0`, aligned to
    /// `SimTime::EPOCH`; `every` is positive and `offset` below it, both in
    /// whole minutes ([`SimDuration`] counts minutes).
    Interval {
        every: SimDuration,
        offset: SimDuration,
    },
}

impl FiscalReportCadence {
    /// `Interval { every, offset }`, with `offset` the `index`-th of `stride`
    /// offsets spread evenly over `every`:
    /// `offset = (index * every / stride) % every`, in whole minutes. With
    /// `every` thirty days and `stride` thirty, observers `0, 1, 2, ...` are
    /// due one day apart, and every thirtieth observer on the same day.
    #[must_use]
    pub fn staggered(every: SimDuration, index: u64, stride: u64) -> Self {
        let minutes = every.as_minutes();
        let offset = if minutes > 0 {
            let spread = i128::from(index) * i128::from(minutes) / i128::from(stride.max(1));
            i64::try_from(spread % i128::from(minutes)).unwrap_or(0)
        } else {
            0
        };
        Self::Interval {
            every,
            offset: SimDuration::minutes(offset),
        }
    }

    /// Whether the cadence is an interval.
    #[must_use]
    pub const fn is_interval(self) -> bool {
        matches!(self, Self::Interval { .. })
    }

    /// The first due time at or after `at`: `at` itself for `OnChange`, the
    /// first aligned time for `Interval`; `None` past the range of
    /// simulated time or for an invalid interval.
    #[must_use]
    pub fn due_at_or_after(self, at: SimTime) -> Option<SimTime> {
        match self {
            Self::OnChange => Some(at),
            Self::Interval { every, offset } => {
                let (every, offset) = (every.as_minutes(), offset.as_minutes());
                if every <= 0 || !(0..every).contains(&offset) {
                    return None;
                }
                let at = at.as_minutes();
                if at <= offset {
                    return Some(SimTime::from_minutes(offset));
                }
                let steps = at.checked_sub(offset)?.checked_add(every - 1)? / every;
                offset
                    .checked_add(steps.checked_mul(every)?)
                    .map(SimTime::from_minutes)
            }
        }
    }

    /// The first due time strictly after `at`.
    #[must_use]
    pub fn due_after(self, at: SimTime) -> Option<SimTime> {
        self.due_at_or_after(SimTime::from_minutes(at.as_minutes().checked_add(1)?))
    }

    /// Whether `at` is one of the cadence's due times; every time is one for
    /// `OnChange`.
    #[must_use]
    pub fn is_due_time(self, at: SimTime) -> bool {
        self.due_at_or_after(at) == Some(at)
    }

    /// Checks an interval: `every` positive, `0 <= offset < every`.
    pub(crate) fn validate(self) -> Result<(), CanwuError> {
        match self {
            Self::OnChange => Ok(()),
            Self::Interval { every, offset } => {
                if every.as_minutes() > 0 && (0..every.as_minutes()).contains(&offset.as_minutes())
                {
                    Ok(())
                } else {
                    Err(invalid(
                        "a fiscal report interval requires a positive period and an offset below it",
                    ))
                }
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalAdoptionState {
    pub id: String,
    pub rule_id: String,
    pub scope_binding_id: String,
    pub stage: FiscalAdoptionStage,
    pub generation: u64,
    pub changed_at: SimTime,
    pub source_action_id: Option<String>,
}

/// One assessment of one accounting partition (scope, cycle, unit, payment
/// form), held in the book of its cycle in its scope's ledger.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalAssessment {
    pub id: String,
    pub rule_id: String,
    pub scope_binding_id: String,
    pub accounting_cycle_id: String,
    pub quantity: u64,
    pub unit: String,
    pub payment_form: FiscalPaymentForm,
    pub commutation_quote: Option<DomainRecordVersionRef>,
    pub created_at: SimTime,
    /// The action that opened the assessment; `None` for one authored with
    /// the state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_action_id: Option<String>,
}

/// A remission of part of one assessment, in the assessment's book.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalRemission {
    pub id: String,
    pub assessment_id: String,
    pub quantity: u64,
    pub reason: String,
    pub granted_at: SimTime,
    /// The action that granted the remission; `None` for one authored with
    /// the state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_action_id: Option<String>,
}

/// An execution authorized against one assessment, in the assessment's book.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalExecutionRequest {
    pub id: String,
    pub assessment_id: String,
    pub institution: EntityRef,
    pub kind: FiscalExecutionKind,
    pub quantity: u64,
    pub unit: String,
    pub payment_form: FiscalPaymentForm,
    pub resource: ResourceId,
    pub source: EntityRef,
    pub target: EntityRef,
    pub purpose: String,
    pub requested_at: SimTime,
    /// The action that authorized the execution; `None` for one authored
    /// with the state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_action_id: Option<String>,
}

/// External evidence of one execution, in its request's book.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalExecutionReceipt {
    pub id: String,
    pub request_id: String,
    pub quantity: u64,
    pub disposition: FiscalReceiptDisposition,
    pub external_evidence: BTreeSet<DomainRecordVersionRef>,
    pub external_operations: BTreeSet<FiscalExternalOperationRef>,
    pub accepted_ingress: IngressId,
    pub observed_at: SimTime,
}

/// An audit finding on an assessment, execution request, or receipt, in the
/// target's book.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalAuditFinding {
    pub id: String,
    pub target_id: String,
    pub severity: FiscalAuditSeverity,
    pub finding: String,
    pub evidence: BTreeSet<EvidenceRef>,
    pub recorded_at: SimTime,
    /// Who entered the finding: the auditor of an audit binding, or an actor
    /// of the audited institution's own authority binding. `None` only for a
    /// finding authored with the state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entered_by: Option<FiscalActionIssuer>,
    /// For a finding entered under an audit binding, the decision that
    /// appointed its auditor, as the binding held it when the finding
    /// settled. `None` otherwise, and for a binding installed with the state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appointed_by: Option<FiscalRebinding>,
    /// The nomination that appointment accepted, and its nominator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nomination: Option<FiscalProposalRef>,
    /// The action that recorded the finding; `None` for one authored with
    /// the state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_action_id: Option<String>,
}

/// One recent outcome of one action target: what [`FiscalOutcomeRing`]
/// keeps. The reason is not kept; it is in the outcome's
/// `canwu.fiscal.action_settled.v1` event.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalRecentOutcome {
    pub action_id: String,
    pub disposition: FiscalActionDisposition,
    /// The target's revision when the outcome was recorded, before the
    /// outcome advanced it.
    pub settled_revision: u64,
}

/// The newest outcomes of one action target, and its outcome floor.
///
/// Every outcome, applied or rejected, is recorded here and advances its
/// target's revision. The ring keeps the newest [`FISCAL_RECENT_OUTCOMES`];
/// an outcome it evicts raises `floor` to that outcome's settled revision.
/// Admission refuses a request whose action ID is in the ring
/// (`IdempotencyConflict`), and one whose expected revision is at or below
/// the floor ([`FISCAL_REVISION_EXPIRED`]): every outcome was admitted with
/// an expected revision no greater than its settled revision, so a re-send
/// of an evicted outcome's request is refused, while a fresh request names
/// the current revision, which is above every recorded outcome's.
/// Settlement skips a copy of an action whose ID has an outcome in the ring
/// or among the outcomes the boundary has settled for the same target.
/// Together they apply an action at most once, without keeping every
/// outcome for the life of the run.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalOutcomeRing {
    /// Oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent: Vec<FiscalRecentOutcome>,
    /// The settled revision of the newest outcome the ring evicted; zero
    /// while none was.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub floor: u64,
}

impl FiscalOutcomeRing {
    /// Whether no outcome was ever recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.recent.is_empty() && self.floor == 0
    }

    /// The recent outcome of `action_id`, while the ring keeps it.
    #[must_use]
    pub fn get(&self, action_id: &str) -> Option<&FiscalRecentOutcome> {
        self.recent
            .iter()
            .find(|outcome| outcome.action_id == action_id)
    }

    /// Records an outcome, evicting the oldest past
    /// [`FISCAL_RECENT_OUTCOMES`].
    pub(crate) fn push(&mut self, outcome: FiscalRecentOutcome) {
        self.recent.push(outcome);
        while self.recent.len() > FISCAL_RECENT_OUTCOMES {
            let evicted = self.recent.remove(0);
            self.floor = evicted.settled_revision;
        }
    }

    /// Checks the ring of a target at `revision`, which is at least 1.
    pub(crate) fn validate(&self, revision: u64) -> Result<(), CanwuError> {
        let mut ids = BTreeSet::new();
        if self.recent.len() > FISCAL_RECENT_OUTCOMES
            || self.floor >= revision
            || self.recent.iter().any(|outcome| {
                validate_identifier(&outcome.action_id, "fiscal action").is_err()
                    || !ids.insert(&outcome.action_id)
                    || outcome.settled_revision >= revision
            })
            || self
                .recent
                .windows(2)
                .any(|pair| pair[0].settled_revision >= pair[1].settled_revision)
            || self
                .recent
                .first()
                .is_some_and(|oldest| oldest.settled_revision <= self.floor)
        {
            return Err(invalid(
                "a fiscal outcome ring is unbounded, unordered, repeats an action, or lies past its record's revision",
            ));
        }
        Ok(())
    }
}

/// Whether a ring or floor holds nothing yet; serde helper.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero(value: &u64) -> bool {
    *value == 0
}

/// Whether an accounting cycle book takes new entries.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FiscalBookStatus {
    Open,
    /// Closed by [`FiscalAction::CloseAccountingCycle`]; the book takes only
    /// audit findings until it is archived.
    Closed {
        closed_by: FiscalRebinding,
    },
}

/// What one scope holds for one accounting cycle while the cycle is hot.
///
/// Object IDs are unique among the hot objects of their kind in one scope,
/// so a reference to one names its scope; an ID may be used again in a later
/// book once the earlier one is archived. The assessments of a book are of
/// distinct (unit, payment form) partitions, at most
/// [`MAX_FISCAL_PARTITION_KEYS`]. The book's totals per partition
/// ([`Self::totals`]) are what a report row shows of it, and are archived
/// with it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalCycleBook {
    pub accounting_cycle_id: String,
    pub status: FiscalBookStatus,
    /// Advances on every change of the book. An archive commit binds the
    /// revision of each book it takes, so one prepared before a change of a
    /// selected book settles stale.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub revision: u64,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub assessments: BTreeMap<String, FiscalAssessment>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub remissions: BTreeMap<String, FiscalRemission>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub execution_requests: BTreeMap<String, FiscalExecutionRequest>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub execution_receipts: BTreeMap<String, FiscalExecutionReceipt>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub audits: BTreeMap<String, FiscalAuditFinding>,
}

impl FiscalCycleBook {
    /// An open book with nothing in it.
    #[must_use]
    pub fn open(accounting_cycle_id: impl Into<String>) -> Self {
        Self {
            accounting_cycle_id: accounting_cycle_id.into(),
            status: FiscalBookStatus::Open,
            revision: 0,
            assessments: BTreeMap::new(),
            remissions: BTreeMap::new(),
            execution_requests: BTreeMap::new(),
            execution_receipts: BTreeMap::new(),
            audits: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn is_open(&self) -> bool {
        self.status == FiscalBookStatus::Open
    }

    /// Advances the book's revision after a change.
    pub(crate) fn touch(&mut self) -> Result<(), CanwuError> {
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("fiscal book revision overflowed"))?;
        Ok(())
    }

    /// The partition keys of the book's assessments, in partition order.
    #[must_use]
    pub fn partition_keys(&self) -> BTreeSet<FiscalPartitionKey> {
        self.assessments
            .values()
            .map(|assessment| FiscalPartitionKey {
                unit: assessment.unit.clone(),
                payment_form: assessment.payment_form,
            })
            .collect()
    }

    /// The scope-level totals of the book, one per (unit, payment form)
    /// partition, in partition order, under the scope's `mechanism`.
    pub fn totals(&self, mechanism: FiscalMechanism) -> Result<Vec<FiscalCycleTotal>, CanwuError> {
        cycle_totals(
            mechanism,
            self.assessments.values().map(|assessment| {
                (
                    assessment.id.as_str(),
                    assessment.unit.as_str(),
                    assessment.payment_form,
                    assessment.quantity,
                )
            }),
            self.remissions
                .values()
                .map(|remission| (remission.assessment_id.as_str(), remission.quantity)),
            self.execution_requests.values().map(|request| {
                (
                    request.id.as_str(),
                    request.assessment_id.as_str(),
                    request.kind,
                )
            }),
            self.execution_receipts.values().map(|receipt| {
                (
                    receipt.request_id.as_str(),
                    receipt.disposition,
                    receipt.quantity,
                )
            }),
        )
    }

    /// The evidence versions the book's hot objects cite: its assessments'
    /// commutation quotes and its receipts' evidence.
    pub(crate) fn citations(&self) -> impl Iterator<Item = &DomainRecordVersionRef> {
        self.assessments
            .values()
            .filter_map(|assessment| assessment.commutation_quote.as_ref())
            .chain(
                self.execution_receipts
                    .values()
                    .flat_map(|receipt| receipt.external_evidence.iter()),
            )
    }
}

/// One accounting partition of a scope and cycle: what one assessment
/// assesses, and one fact of a report row.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct FiscalPartitionKey {
    pub unit: String,
    pub payment_form: FiscalPaymentForm,
}

/// One cycle's totals of one partition: of one scope's book, or summed over
/// an institution's books of the cycle per (mechanism, unit, payment form).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalCycleTotal {
    pub mechanism: FiscalMechanism,
    pub unit: String,
    pub payment_form: FiscalPaymentForm,
    pub assessed: u64,
    pub remission_granted: u64,
    pub collected: u64,
    pub remitted: u64,
    pub disbursed: u64,
    pub reserved: u64,
    pub returned: u64,
    pub outstanding: u64,
}

impl FiscalCycleTotal {
    fn empty(mechanism: FiscalMechanism, unit: &str, payment_form: FiscalPaymentForm) -> Self {
        Self {
            mechanism,
            unit: unit.to_owned(),
            payment_form,
            assessed: 0,
            remission_granted: 0,
            collected: 0,
            remitted: 0,
            disbursed: 0,
            reserved: 0,
            returned: 0,
            outstanding: 0,
        }
    }

    /// The partition the total is of.
    fn key(&self) -> (FiscalMechanism, &str, FiscalPaymentForm) {
        (self.mechanism, &self.unit, self.payment_form)
    }

    fn add(&mut self, other: &Self) -> Result<(), CanwuError> {
        self.assessed = sum(self.assessed, other.assessed)?;
        self.remission_granted = sum(self.remission_granted, other.remission_granted)?;
        self.collected = sum(self.collected, other.collected)?;
        self.remitted = sum(self.remitted, other.remitted)?;
        self.disbursed = sum(self.disbursed, other.disbursed)?;
        self.reserved = sum(self.reserved, other.reserved)?;
        self.returned = sum(self.returned, other.returned)?;
        self.outstanding = sum(self.outstanding, other.outstanding)?;
        Ok(())
    }

    fn add_saturating(&mut self, other: &Self) {
        self.assessed = self.assessed.saturating_add(other.assessed);
        self.remission_granted = self
            .remission_granted
            .saturating_add(other.remission_granted);
        self.collected = self.collected.saturating_add(other.collected);
        self.remitted = self.remitted.saturating_add(other.remitted);
        self.disbursed = self.disbursed.saturating_add(other.disbursed);
        self.reserved = self.reserved.saturating_add(other.reserved);
        self.returned = self.returned.saturating_add(other.returned);
        self.outstanding = self.outstanding.saturating_add(other.outstanding);
    }
}

/// Adds `totals` into `into`, partition by partition, keeping `into` in
/// partition order.
pub(crate) fn merge_cycle_totals(
    into: &mut Vec<FiscalCycleTotal>,
    totals: &[FiscalCycleTotal],
) -> Result<(), CanwuError> {
    for total in totals {
        match into.binary_search_by(|existing| existing.key().cmp(&total.key())) {
            Ok(index) => into[index].add(total)?,
            Err(index) => into.insert(index, total.clone()),
        }
    }
    Ok(())
}

/// Adds `totals` into `into` as [`merge_cycle_totals`] does, each amount
/// saturating at `u64::MAX`: a report's sum over many books, which no
/// settlement holds to a bound, so it must never fail a boundary. The report
/// system and [`crate::compute_projection`] both sum this way, so a report
/// still equals its projection.
pub(crate) fn sum_report_totals(into: &mut Vec<FiscalCycleTotal>, totals: &[FiscalCycleTotal]) {
    for total in totals {
        match into.binary_search_by(|existing| existing.key().cmp(&total.key())) {
            Ok(index) => into[index].add_saturating(total),
            Err(index) => into.insert(index, total.clone()),
        }
    }
}

fn sum(left: u64, right: u64) -> Result<u64, CanwuError> {
    left.checked_add(right)
        .ok_or_else(|| invalid("fiscal cycle total overflowed"))
}

/// The totals of one book, one per (unit, payment form) partition, in
/// partition order, from the fields of its objects that they depend on:
/// each assessment's ID, unit, payment form, and quantity; each remission's
/// assessment and quantity; each execution request's ID, assessment, and
/// kind; and each receipt's request, disposition, and quantity. Both
/// [`FiscalCycleBook::totals`] and the report system's read of a book in its
/// record ([`BookFigures`]) compute them here.
pub(crate) fn cycle_totals<'a>(
    mechanism: FiscalMechanism,
    assessments: impl IntoIterator<Item = (&'a str, &'a str, FiscalPaymentForm, u64)>,
    remissions: impl IntoIterator<Item = (&'a str, u64)>,
    requests: impl IntoIterator<Item = (&'a str, &'a str, FiscalExecutionKind)>,
    receipts: impl IntoIterator<Item = (&'a str, FiscalReceiptDisposition, u64)>,
) -> Result<Vec<FiscalCycleTotal>, CanwuError> {
    let mut totals = BTreeMap::<(&str, FiscalPaymentForm), FiscalCycleTotal>::new();
    let mut partitions = BTreeMap::new();
    for (id, unit, payment_form, quantity) in assessments {
        let key = (unit, payment_form);
        partitions.insert(id, key);
        let total = totals
            .entry(key)
            .or_insert_with(|| FiscalCycleTotal::empty(mechanism, unit, payment_form));
        total.assessed = sum(total.assessed, quantity)?;
    }
    for (assessment_id, quantity) in remissions {
        let key = partitions
            .get(assessment_id)
            .ok_or_else(|| invalid("fiscal remission has no assessment in its book"))?;
        let total = totals
            .get_mut(key)
            .ok_or_else(|| invalid("fiscal remission partition is unavailable"))?;
        total.remission_granted = sum(total.remission_granted, quantity)?;
    }
    let mut request_keys = BTreeMap::new();
    for (id, assessment_id, kind) in requests {
        let key = partitions
            .get(assessment_id)
            .ok_or_else(|| invalid("fiscal execution request has no assessment in its book"))?;
        request_keys.insert(id, (*key, kind));
    }
    for (request_id, _, quantity) in receipts
        .into_iter()
        .filter(|(_, disposition, _)| disposition.counts_as_fulfillment())
    {
        let (key, kind) = request_keys
            .get(request_id)
            .ok_or_else(|| invalid("fiscal receipt has no execution request in its book"))?;
        let total = totals
            .get_mut(key)
            .ok_or_else(|| invalid("fiscal receipt partition is unavailable"))?;
        let target = match kind {
            FiscalExecutionKind::Collect => &mut total.collected,
            FiscalExecutionKind::Remit => &mut total.remitted,
            FiscalExecutionKind::Disburse => &mut total.disbursed,
            FiscalExecutionKind::Reserve => &mut total.reserved,
            FiscalExecutionKind::Return => &mut total.returned,
        };
        *target = sum(*target, quantity)?;
    }
    totals
        .into_values()
        .map(|mut total| {
            total.outstanding = total
                .assessed
                .checked_sub(total.remission_granted)
                .and_then(|value| value.checked_sub(total.collected))
                .ok_or_else(|| invalid("fiscal cycle total underflowed"))?;
            Ok(total)
        })
        .collect()
}

/// What a book's totals depend on, read from the book's encoded form in its
/// ledger record and borrowing its text, so that the report system sums a
/// book without decoding it: every other field of the book and of its
/// objects is skipped.
#[derive(Deserialize)]
pub(crate) struct BookFigures<'a> {
    #[serde(default, borrow)]
    assessments: BTreeMap<&'a str, AssessmentFigures<'a>>,
    #[serde(default, borrow)]
    remissions: BTreeMap<&'a str, RemissionFigures<'a>>,
    #[serde(default, borrow)]
    execution_requests: BTreeMap<&'a str, RequestFigures<'a>>,
    #[serde(default, borrow)]
    execution_receipts: BTreeMap<&'a str, ReceiptFigures<'a>>,
}

#[derive(Deserialize)]
struct AssessmentFigures<'a> {
    id: &'a str,
    unit: &'a str,
    payment_form: FiscalPaymentForm,
    quantity: u64,
}

#[derive(Deserialize)]
struct RemissionFigures<'a> {
    assessment_id: &'a str,
    quantity: u64,
}

#[derive(Deserialize)]
struct RequestFigures<'a> {
    id: &'a str,
    assessment_id: &'a str,
    kind: FiscalExecutionKind,
}

#[derive(Deserialize)]
struct ReceiptFigures<'a> {
    request_id: &'a str,
    disposition: FiscalReceiptDisposition,
    quantity: u64,
}

impl BookFigures<'_> {
    /// The book's totals, as [`FiscalCycleBook::totals`] gives them.
    pub(crate) fn totals(
        &self,
        mechanism: FiscalMechanism,
    ) -> Result<Vec<FiscalCycleTotal>, CanwuError> {
        cycle_totals(
            mechanism,
            self.assessments.values().map(|assessment| {
                (
                    assessment.id,
                    assessment.unit,
                    assessment.payment_form,
                    assessment.quantity,
                )
            }),
            self.remissions
                .values()
                .map(|remission| (remission.assessment_id, remission.quantity)),
            self.execution_requests
                .values()
                .map(|request| (request.id, request.assessment_id, request.kind)),
            self.execution_receipts
                .values()
                .map(|receipt| (receipt.request_id, receipt.disposition, receipt.quantity)),
        )
    }
}

/// One scope binding with everything the state holds for it: its adoptions,
/// the books of its hot accounting cycles, its watermark, its revision, and
/// the recent outcomes of the actions on it. The payload of its
/// `canwu.fiscal:ledger` record.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalScopeLedger {
    pub scope: FiscalScopeBinding,
    /// The key of the office of the scope's institution; derived.
    #[serde(default)]
    pub office: String,
    /// The scope's place among its office's scopes, in scope ID order, which
    /// is its row among an observer's rows of that office; derived.
    #[serde(default)]
    pub rank: u32,
    /// Advances by one with every outcome and every receipt on the scope.
    /// An action names the revision it decided against.
    pub revision: u64,
    /// Adoptions of rules in this scope, keyed by adoption ID; at most
    /// [`MAX_FISCAL_ADOPTIONS_PER_LEDGER`].
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub adoptions: BTreeMap<String, FiscalAdoptionState>,
    /// Hot books, open or closed, keyed by accounting cycle; at most
    /// [`MAX_FISCAL_HOT_BOOKS_PER_LEDGER`].
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub books: BTreeMap<String, FiscalCycleBook>,
    /// The greatest accounting cycle, in byte order, of the scope's archived
    /// books. A new book must be above it, and above the watermark of the
    /// scope's office, so an accounting partition is assessed at most once
    /// for the life of a run without reading the archive. Hot books may lie
    /// below it, since books archive in any order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watermark: Option<String>,
    /// The recent outcomes of the actions on this scope.
    #[serde(default, skip_serializing_if = "FiscalOutcomeRing::is_empty")]
    pub outcomes: FiscalOutcomeRing,
}

impl FiscalScopeLedger {
    /// Checks the ledger on its own: its scope, revision, ring, adoptions,
    /// books, and every per-assessment invariant of its books. This is what
    /// every write of its record checks.
    pub fn validate(&self, catalog: &CompiledFiscalCatalog) -> Result<(), CanwuError> {
        validate_ledger(self, catalog)
    }

    /// A ledger with nothing but its scope binding.
    #[must_use]
    pub fn new(scope: FiscalScopeBinding) -> Self {
        Self {
            office: fiscal_office_key(&scope.institution),
            scope,
            rank: 0,
            revision: 1,
            adoptions: BTreeMap::new(),
            books: BTreeMap::new(),
            watermark: None,
            outcomes: FiscalOutcomeRing::default(),
        }
    }

    /// The book that holds the assessment `assessment_id`.
    #[must_use]
    pub fn book_of_assessment(&self, assessment_id: &str) -> Option<&FiscalCycleBook> {
        self.books
            .values()
            .find(|book| book.assessments.contains_key(assessment_id))
    }

    /// The book that holds the execution request `request_id`.
    #[must_use]
    pub fn book_of_request(&self, request_id: &str) -> Option<&FiscalCycleBook> {
        self.books
            .values()
            .find(|book| book.execution_requests.contains_key(request_id))
    }

    /// The book that holds the assessment, execution request, or receipt
    /// `target_id`, looked up in that order.
    #[must_use]
    pub fn book_of_audit_target(&self, target_id: &str) -> Option<&FiscalCycleBook> {
        self.book_of_assessment(target_id)
            .or_else(|| self.book_of_request(target_id))
            .or_else(|| {
                self.books
                    .values()
                    .find(|book| book.execution_receipts.contains_key(target_id))
            })
    }
}

/// One institution: its authority bindings, the audit bindings on it, the
/// policy that governs its auditors and their proposals, the cycle
/// bookkeeping of its scopes' books, its revision, and the recent outcomes
/// of the actions on it. The payload of its `canwu.fiscal:office` record,
/// keyed by [`fiscal_office_key`].
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalOffice {
    pub institution: EntityRef,
    /// Advances by one with every outcome on the office.
    pub revision: u64,
    /// The recent outcomes of the actions targeting the office.
    #[serde(default, skip_serializing_if = "FiscalOutcomeRing::is_empty")]
    pub outcomes: FiscalOutcomeRing,
    /// The institution's authority bindings, keyed by binding ID; at most
    /// [`MAX_FISCAL_AUTHORITY_BINDINGS_PER_OFFICE`].
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub authority_bindings: BTreeMap<String, FiscalAuthorityBinding>,
    /// Auditors from outside the institution, keyed by audit binding ID; at
    /// most [`MAX_FISCAL_AUDIT_BINDINGS_PER_OFFICE`].
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub audit_bindings: BTreeMap<String, FiscalAuditBinding>,
    /// Who appoints and removes the institution's auditors; declared with
    /// the state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appointment_policy: Option<FiscalAppointmentPolicy>,
    /// The nominations and petitions concerning the institution's auditors,
    /// pending and the latest resolved, keyed by proposal ID.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub audit_proposals: BTreeMap<String, FiscalAuditProposal>,
    /// Each accounting cycle of which some scope of the institution holds a
    /// hot book: how many do, the partition keys their books use, how many
    /// were archived, and the totals of those archived. A cycle leaves when
    /// its last hot book is archived, by the archive commit that also
    /// archives the institution's totals of it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub cycles: BTreeMap<String, FiscalOfficeCycle>,
    /// The greatest accounting cycle, in byte order, whose institution totals
    /// the archive holds. No scope of the institution makes a new book at or
    /// below it, so those totals are archived once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watermark: Option<String>,
    /// How many scope bindings the institution has; derived.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub scope_count: u32,
    /// The mechanisms of the institution's scope bindings; derived. An
    /// observer at [`FiscalReportGranularity::Institution`] has one row for
    /// each, in this order.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub mechanisms: BTreeSet<FiscalMechanism>,
    /// The observer bindings that see the institution, each with the first
    /// row of the institution in its report, its granularity, and whether its
    /// cadence is an interval; derived. Settlement queues the report work a
    /// change of a scope's book owes from this alone, without reading an
    /// observer: the scope's row is that first row plus the scope's rank, or,
    /// at institution granularity, plus the place of the scope's mechanism
    /// among `mechanisms`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub observers: BTreeMap<String, FiscalOfficeObserver>,
}

/// One observer of an institution, as the institution's office lists it:
/// the first row of the institution in the observer's report, the binding's
/// granularity, and whether its cadence is an interval, the least settlement
/// needs to queue its work; derived from the binding, which keeps the full
/// cadence. An office can list 4,096 observers, so each entry stays small.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalOfficeObserver {
    pub first_row: u32,
    #[serde(default, skip_serializing_if = "is_default")]
    pub granularity: FiscalReportGranularity,
    /// Whether the binding's cadence is [`FiscalReportCadence::Interval`].
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub interval: bool,
}

impl FiscalOfficeObserver {
    /// The row of the observer's report that a book of the office's scope of
    /// rank `rank` and mechanism `mechanism` is summed into, given the
    /// office's `mechanisms`.
    #[must_use]
    pub fn row(
        &self,
        rank: u32,
        mechanism: FiscalMechanism,
        mechanisms: &BTreeSet<FiscalMechanism>,
    ) -> u32 {
        let offset = match self.granularity {
            FiscalReportGranularity::Scope => rank,
            FiscalReportGranularity::Institution => u32::try_from(
                mechanisms
                    .iter()
                    .take_while(|held| **held != mechanism)
                    .count(),
            )
            .unwrap_or(u32::MAX),
        };
        self.first_row.saturating_add(offset)
    }
}

impl FiscalOffice {
    #[must_use]
    pub fn new(institution: EntityRef) -> Self {
        Self {
            institution,
            revision: 1,
            outcomes: FiscalOutcomeRing::default(),
            authority_bindings: BTreeMap::new(),
            audit_bindings: BTreeMap::new(),
            appointment_policy: None,
            audit_proposals: BTreeMap::new(),
            cycles: BTreeMap::new(),
            watermark: None,
            scope_count: 0,
            mechanisms: BTreeSet::new(),
            observers: BTreeMap::new(),
        }
    }
}

/// One hot accounting cycle of one institution.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalOfficeCycle {
    /// Scopes of the institution that hold a hot book of the cycle; above
    /// zero.
    pub hot_books: u32,
    /// The partition keys the cycle's books use, per mechanism, at most
    /// [`MAX_FISCAL_PARTITION_KEYS`] each. Keys stay until the cycle leaves.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub partition_keys: BTreeMap<FiscalMechanism, BTreeSet<FiscalPartitionKey>>,
    /// Books of the cycle already archived.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub archived_books: u32,
    /// The totals of the archived books, per (mechanism, unit, payment
    /// form), in that order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub archived_totals: Vec<FiscalCycleTotal>,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

/// One catalog region in use: its scopes' mechanisms, its adoption counts,
/// and the transition candidates they derive, its revision, and the recent
/// outcomes of the transitions applied in it. The payload of its
/// `canwu.fiscal:jurisdiction` record.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalJurisdiction {
    pub region_id: String,
    /// Advances by one with every outcome on the jurisdiction.
    pub revision: u64,
    #[serde(default, skip_serializing_if = "FiscalOutcomeRing::is_empty")]
    pub outcomes: FiscalOutcomeRing,
    /// How many scope bindings name the region; derived.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub scope_count: u32,
    /// The mechanisms of those scopes, and how many scopes have each;
    /// derived.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub mechanisms: BTreeMap<FiscalMechanism, u32>,
    /// For each rule adopted in the region, how many of its adoptions are not
    /// repealed and how many are operational; derived.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub adoption_counts: BTreeMap<String, FiscalAdoptionCount>,
    /// The transitions the catalog, the historical context, and the adoption
    /// counts make eligible in the region; derived.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub transition_candidates: BTreeMap<String, FiscalTransitionCandidate>,
}

impl FiscalJurisdiction {
    #[must_use]
    pub fn new(region_id: impl Into<String>) -> Self {
        Self {
            region_id: region_id.into(),
            revision: 1,
            outcomes: FiscalOutcomeRing::default(),
            scope_count: 0,
            mechanisms: BTreeMap::new(),
            adoption_counts: BTreeMap::new(),
            transition_candidates: BTreeMap::new(),
        }
    }
}

/// How many adoptions of one rule a region has that are not repealed, and
/// how many are operational.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalAdoptionCount {
    pub live: u32,
    pub operational: u32,
}

/// One observer binding with its revision, the recent outcomes of the
/// actions on it, the layout of its report, and what its report has
/// published. The payload of its `canwu.fiscal:observer` record, whose
/// identity is the binding ID; the binding's report lines are named on that
/// record.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalObserver {
    pub binding: FiscalObserverBinding,
    /// Advances by one with every outcome on the binding.
    pub revision: u64,
    #[serde(default, skip_serializing_if = "FiscalOutcomeRing::is_empty")]
    pub outcomes: FiscalOutcomeRing,
    /// The report's rows: for each visible institution, in canonical order,
    /// its office and the row of its first scope; derived.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rows: Vec<FiscalObserverRows>,
    /// What the report has published.
    #[serde(default, skip_serializing_if = "FiscalObserverReport::is_unused")]
    pub report: FiscalObserverReport,
}

impl FiscalObserver {
    /// An observer with nothing published yet.
    #[must_use]
    pub fn new(binding: FiscalObserverBinding) -> Self {
        Self {
            binding,
            revision: 1,
            outcomes: FiscalOutcomeRing::default(),
            rows: Vec::new(),
            report: FiscalObserverReport::default(),
        }
    }

    /// The row of `rank`-th scope of the office `office`, when the observer
    /// sees that office at [`FiscalReportGranularity::Scope`].
    #[must_use]
    pub fn row_of(&self, office: &str, rank: u32) -> Option<u32> {
        if self.binding.granularity != FiscalReportGranularity::Scope {
            return None;
        }
        self.rows
            .iter()
            .find(|rows| rows.office == office && rank < rows.scopes)
            .and_then(|rows| rows.first_row.checked_add(rank))
    }

    /// How many rows the report has.
    #[must_use]
    pub fn row_count(&self) -> u32 {
        self.rows.last().map_or(0, |rows| {
            rows.first_row
                .saturating_add(rows.span(self.binding.granularity))
        })
    }
}

/// The rows of one visible institution in an observer's report.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalObserverRows {
    pub institution: EntityRef,
    /// The institution's office key.
    pub office: String,
    /// The institution's first row.
    pub first_row: u32,
    /// How many scopes the institution has: its rows at
    /// [`FiscalReportGranularity::Scope`].
    pub scopes: u32,
    /// At [`FiscalReportGranularity::Institution`], the mechanisms of the
    /// institution's scopes, one row each, in this order; empty at `Scope`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mechanisms: Vec<FiscalMechanism>,
}

impl FiscalObserverRows {
    /// How many rows the institution takes at `granularity`.
    #[must_use]
    pub fn span(&self, granularity: FiscalReportGranularity) -> u32 {
        match granularity {
            FiscalReportGranularity::Scope => self.scopes,
            FiscalReportGranularity::Institution => {
                u32::try_from(self.mechanisms.len()).unwrap_or(u32::MAX)
            }
        }
    }
}

/// What one observer's report has published: the time it last changed, the
/// slot of each hot cycle it shows with the digest and fact count of each of
/// the cycle's parts, the cycles it had no slot for, and the holders owed a
/// withdrawn summary.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalObserverReport {
    /// When the report was last published; `None` while nothing was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub as_of: Option<SimTime>,
    /// Each cycle the report shows, by accounting cycle.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub cycles: BTreeMap<String, FiscalReportCycleSlot>,
    /// Hot cycles the report has no slot for (see
    /// [`MAX_FISCAL_REPORT_CYCLE_SLOTS`]).
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub omitted_cycles: BTreeSet<String>,
    /// Holders the binding was re-pointed away from, or removed from, still
    /// owed a withdrawn summary.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub withdraw: BTreeSet<KnowledgeHolderRef>,
}

impl FiscalObserverReport {
    /// Whether nothing was published yet and nothing is owed.
    #[must_use]
    pub fn is_unused(&self) -> bool {
        *self == Self::default()
    }

    /// The facts the report holds once every part is current.
    #[must_use]
    pub fn fact_count(&self) -> u32 {
        self.cycles
            .values()
            .map(FiscalReportCycleSlot::fact_count)
            .fold(0, u32::saturating_add)
    }
}

/// Hexadecimal digits one part takes in [`FiscalReportCycleSlot::parts`]:
/// sixteen of digest and two of fact count.
pub(crate) const PART_ENTRY_DIGITS: usize = 18;

/// The entry of a part with no facts.
pub(crate) const EMPTY_PART_ENTRY: &str = "000000000000000000";

/// The slot one cycle of a report takes, and what was published of each of
/// its parts: for each part in order, sixteen hexadecimal digits of the
/// digest of its rows and two of its fact count, all zeros for a part with
/// no facts, with the empty parts at the end left out. The digest is the
/// first 64 bits of a BLAKE3 digest; a change of a part is missed only when
/// the digests of its old and new rows agree.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalReportCycleSlot {
    pub slot: u16,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub parts: String,
}

impl FiscalReportCycleSlot {
    /// An empty slot.
    #[must_use]
    pub fn new(slot: u16) -> Self {
        Self {
            slot,
            parts: String::new(),
        }
    }

    /// The published entry of part `part`: its digest and fact count.
    #[must_use]
    pub fn entry(&self, part: u32) -> &str {
        usize::try_from(part)
            .ok()
            .and_then(|part| part.checked_mul(PART_ENTRY_DIGITS))
            .and_then(|start| self.parts.get(start..start + PART_ENTRY_DIGITS))
            .unwrap_or(EMPTY_PART_ENTRY)
    }

    /// How many parts the stored entries span, the empty ones between them
    /// included.
    #[must_use]
    pub fn part_count(&self) -> u32 {
        u32::try_from(self.parts.len() / PART_ENTRY_DIGITS).unwrap_or(u32::MAX)
    }

    /// The parts that have facts.
    pub fn parts_with_facts(&self) -> impl Iterator<Item = u32> + '_ {
        self.parts
            .as_bytes()
            .chunks(PART_ENTRY_DIGITS)
            .enumerate()
            .filter(|(_, entry)| *entry != EMPTY_PART_ENTRY.as_bytes())
            .filter_map(|(part, _)| u32::try_from(part).ok())
    }

    /// The facts the cycle's parts hold.
    #[must_use]
    pub fn fact_count(&self) -> u32 {
        self.parts
            .as_bytes()
            .chunks(PART_ENTRY_DIGITS)
            .filter_map(|entry| {
                entry
                    .get(16..)
                    .and_then(|count| std::str::from_utf8(count).ok())
            })
            .filter_map(|count| u32::from_str_radix(count, 16).ok())
            .fold(0, u32::saturating_add)
    }

    /// Stores the entry of part `part`.
    pub(crate) fn set_entry(&mut self, part: u32, entry: &str) {
        let start = usize::try_from(part)
            .unwrap_or(usize::MAX)
            .saturating_mul(PART_ENTRY_DIGITS);
        if start >= self.parts.len() {
            if entry == EMPTY_PART_ENTRY {
                return;
            }
            while self.parts.len() < start {
                self.parts.push_str(EMPTY_PART_ENTRY);
            }
            self.parts.push_str(entry);
        } else {
            self.parts
                .replace_range(start..start + PART_ENTRY_DIGITS, entry);
        }
        while self.parts.ends_with(EMPTY_PART_ENTRY) {
            self.parts.truncate(self.parts.len() - PART_ENTRY_DIGITS);
        }
    }

    /// Whether the cycle has no part with facts.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }
}

/// The report work one observer is owed: which parts of which cycles
/// changed, whether its whole report is owed, and when it is due.
///
/// The report system publishes an entry in the first boundary at or after
/// `due_at` that runs it, within the boundary's publication limits. For an
/// [`FiscalReportCadence::OnChange`] observer, a boundary that changes what
/// the observer sees queues an entry due at its own time, and the entry
/// leaves the queue once published. An active
/// [`FiscalReportCadence::Interval`] observer always has exactly one entry,
/// due at one of its due times: [`FiscalState::add_observer`] queues it at
/// the first, settlement merges what it owes into it without moving
/// `due_at`, and the report system, once it has published what the entry
/// owed, queues it again at the next due time after its boundary, whether or
/// not anything was published. No future-dated packet is scheduled for it.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalReportQueueEntry {
    /// The time the entry is due: the boundary time of the change that
    /// queued it for an `OnChange` observer, a due time of an `Interval`
    /// observer's cadence. Work merged into an entry keeps the earlier time,
    /// and an `Interval` entry's time does not move.
    pub due_at: SimTime,
    /// The parts whose rows changed, by accounting cycle; an empty set
    /// stands for every part of the cycle. At most
    /// [`FISCAL_REPORT_QUEUE_CYCLES`] cycles of at most
    /// [`FISCAL_REPORT_QUEUE_PARTS`] parts each: past them a cycle is owed
    /// whole, and past the cycles the report is.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub cycles: BTreeMap<String, BTreeSet<u32>>,
    /// Whether every part of every cycle the report shows is owed, and
    /// published whether it changed or not: the holder may hold none of
    /// them, because the report was never published to it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub whole: bool,
    /// Whether the parts `cycles` names are published whether they changed
    /// or not: they are what is left of a whole report that did not fit one
    /// boundary.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub republish: bool,
    /// Whether the holder could not receive knowledge when the entry was
    /// last due: the observer is parked. A parked entry is not due; the
    /// report system checks the holders of parked entries in rotation,
    /// [`FISCAL_PARKED_CHECKS_PER_BOUNDARY`] in each run, and unparks an
    /// entry whose holder can receive knowledge again, which it then
    /// publishes as owed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub parked: bool,
}

/// The state-wide part of the fiscal state: the historical context, the
/// approved execution-evidence kinds, and when report work is next due. The
/// payload of the `canwu.fiscal:root` record. The archive head and the report
/// schedule are records of their own (`canwu.fiscal:archive`,
/// `canwu.fiscal:report-schedule`), so that the root stays small: a boundary
/// that owes no report reads the root alone.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalRoot {
    pub schema_version: u32,
    pub catalog_version: DomainRecordVersionRef,
    pub historical_context: FiscalHistoricalContext,
    /// Advances with every historical-context change.
    pub context_revision: u64,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub execution_evidence_kinds: BTreeSet<DomainRecordKind>,
    /// The earliest `due_at` of a queued entry that is not parked: the least
    /// of the report schedule's `page_due`; derived.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub earliest_due: Option<SimTime>,
    /// Whether some queued entry is parked, so that the report system checks
    /// parked holders in rotation; derived.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub parked: bool,
    /// The observer binding of the parked entry the rotation checked last;
    /// the next run continues after it, in order of (report-queue page,
    /// observer binding ID), wrapping round. `None` while no entry is parked,
    /// and after a run that checked every parked entry. It is in the root,
    /// which every run reads anyway, so that moving it rewrites a few
    /// hundred bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parked_cursor: Option<String>,
}

/// Where the report queue's work is: each report-queue page's earliest due
/// time and the pages that hold parked entries. The payload of the
/// `canwu.fiscal:report-schedule` record, identity `primary`.
///
/// A boundary reads the root's `earliest_due` first, then this record, and
/// decodes only the pages whose earliest due time has come, so the cost of
/// a boundary that publishes is the pages it publishes from, not every page
/// in use. `page_due` and `parked_pages` are derived from the pages and
/// checked by [`crate::audit_fiscal_records`], with the root's
/// `earliest_due` and `parked`.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalReportSchedule {
    /// Each report-queue page with an entry that is not parked, and the
    /// earliest `due_at` among those entries; derived. At most
    /// [`FISCAL_REPORT_QUEUE_PAGES`] entries.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub page_due: BTreeMap<String, SimTime>,
    /// Each report-queue page with a parked entry, and how many it holds;
    /// derived.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub parked_pages: BTreeMap<String, u32>,
}

impl FiscalReportSchedule {
    /// Whether the schedule names nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Sets what the schedule keeps for one page from the page's entries.
    pub(crate) fn update_page<'a>(
        &mut self,
        page: &str,
        entries: impl IntoIterator<Item = &'a FiscalReportQueueEntry>,
    ) {
        let mut due: Option<SimTime> = None;
        let mut parked = 0_u32;
        for entry in entries {
            if entry.parked {
                parked = parked.saturating_add(1);
            } else {
                due = Some(due.map_or(entry.due_at, |due| due.min(entry.due_at)));
            }
        }
        match due {
            Some(due) => {
                self.page_due.insert(page.to_owned(), due);
            }
            None => {
                self.page_due.remove(page);
            }
        }
        if parked == 0 {
            self.parked_pages.remove(page);
        } else {
            self.parked_pages.insert(page.to_owned(), parked);
        }
    }

    /// The root's `earliest_due` and `parked` this schedule derives.
    #[must_use]
    pub(crate) fn root_fields(&self) -> (Option<SimTime>, bool) {
        (
            self.page_due.values().min().copied(),
            !self.parked_pages.is_empty(),
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalTransitionCandidate {
    pub id: String,
    pub transition_id: String,
    pub jurisdiction_id: String,
    pub historically_observed: bool,
    pub evaluated_at: SimTime,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalAmountEstimate {
    pub minimum: u64,
    pub maximum: u64,
}

/// One fact of a report: one accounting partition of one scope's book, as an
/// observer is told it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalReportFact {
    /// The fact's label in a [`FiscalProjection`]: the accounting cycle, the
    /// row of the scope among the observer's rows, and the fact's place in
    /// the row, joined by `:`. A fact keeps its label while its book is hot.
    pub id: String,
    pub institution: EntityRef,
    pub mechanism: FiscalMechanism,
    /// The scope of a fact of a scope row; empty for an institution row.
    pub scope_binding_id: String,
    pub accounting_cycle_id: String,
    pub unit: String,
    pub payment_form: FiscalPaymentForm,
    pub assessed: FiscalAmountEstimate,
    pub collected: FiscalAmountEstimate,
    pub outstanding: FiscalAmountEstimate,
}

impl FiscalReportFact {
    /// The label of the `index`-th fact of row `row` in `cycle`.
    #[must_use]
    pub fn label(cycle: &str, row: u32, index: usize) -> String {
        format!("{cycle}:{row}:{index}")
    }

    /// The cycle and part of a fact's label.
    #[must_use]
    pub fn part_of(label: &str) -> Option<(&str, u32)> {
        let mut pieces = label.rsplitn(3, ':');
        let _index = pieces.next()?;
        let row: u32 = pieces.next()?.parse().ok()?;
        let cycle = pieces.next()?;
        Some((
            cycle,
            row / u32::try_from(FISCAL_REPORT_ROWS_PER_PART).unwrap_or(u32::MAX),
        ))
    }
}

/// Everything one observer is told: what [`crate::compute_projection`]
/// derives from the fiscal state, and what
/// [`crate::assemble_fiscal_reports`] puts back together from the report
/// records the observer's holder holds.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalProjection {
    /// The observer binding's actor; `None` for a report an institution
    /// holds.
    pub actor: Option<PersonId>,
    pub as_of: SimTime,
    pub confidence_per_mille: u16,
    pub facts: BTreeMap<String, FiscalReportFact>,
}

/// The payload of one fiscal report knowledge record (schema version 5).
///
/// An observer's report is one summary and, for each hot cycle it shows,
/// parts of [`FISCAL_REPORT_ROWS_PER_PART`] rows each, every one its own line
/// of records on the observer's record: a record supersedes the previous
/// record of the same line and no other. Read a report with
/// [`crate::fiscal_reports`] or [`crate::assemble_fiscal_reports`] rather
/// than from single records.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FiscalReportRecord {
    Summary(FiscalReportSummary),
    Part(FiscalReportPart),
}

/// The head of one observer's report. It is republished whenever one of the
/// report's parts is, and, for an [`FiscalReportCadence::Interval`] report,
/// alone at every due time at which nothing changed, once the report has
/// been published at least once, so that `as_of` advances and silence never
/// means "no report". For an `OnChange` report `as_of` is the time the
/// report last changed; for an `Interval` report, the boundary of its last
/// due time.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalReportSummary {
    /// The observer binding the report belongs to.
    pub observer_id: String,
    pub actor: Option<PersonId>,
    pub as_of: SimTime,
    pub confidence_per_mille: u16,
    /// The binding's granularity: what a row of the report is.
    #[serde(default, skip_serializing_if = "is_default")]
    pub granularity: FiscalReportGranularity,
    /// The binding's cadence: when the report is published.
    #[serde(default, skip_serializing_if = "is_default")]
    pub cadence: FiscalReportCadence,
    /// Each hot cycle the report shows, and the slot its parts are in. A part
    /// of another slot, or of a cycle not listed, is not part of the report.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub cycles: BTreeMap<String, u16>,
    /// Facts the report holds once every part is current.
    pub fact_count: u32,
    /// The parts, by cycle, whose rows changed but which no boundary has
    /// published yet. The records the holder holds for them are older, or
    /// missing. Empty for a complete report. Each cycle's parts are written
    /// as a bitmap in hexadecimal, bit `p % 8` of byte `p / 8` for part `p`,
    /// so a summary stays within one knowledge record.
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        with = "part_bitmaps"
    )]
    pub pending: BTreeMap<String, BTreeSet<u32>>,
    /// Hot cycles the report has no slot for and does not show (see
    /// [`MAX_FISCAL_REPORT_CYCLE_SLOTS`]).
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub omitted_cycles: u32,
    /// Whether the binding was re-pointed away from this holder, or removed:
    /// the holder keeps what it was told, and [`crate::fiscal_reports`] and
    /// [`crate::assemble_fiscal_reports`] omit the report.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub withdrawn: bool,
}

/// Each cycle's parts as the hexadecimal bitmap a summary's `pending` holds.
pub(crate) fn part_bitmap_entries(
    parts: &BTreeMap<String, BTreeSet<u32>>,
) -> BTreeMap<String, String> {
    part_bitmaps::entries(parts)
}

/// The parts of each cycle as a hexadecimal bitmap.
mod part_bitmaps {
    use super::MAX_FISCAL_REPORT_PARTS_PER_CYCLE;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::collections::{BTreeMap, BTreeSet};

    #[allow(clippy::trivially_copy_pass_by_ref)]
    pub(super) fn serialize<S: Serializer>(
        parts: &BTreeMap<String, BTreeSet<u32>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        entries(parts).serialize(serializer)
    }

    /// Each cycle's parts as its hexadecimal bitmap.
    pub(crate) fn entries(parts: &BTreeMap<String, BTreeSet<u32>>) -> BTreeMap<String, String> {
        parts
            .iter()
            .map(|(cycle, parts)| {
                let mut bytes = Vec::new();
                for part in parts {
                    let index = usize::try_from(*part / 8).unwrap_or(usize::MAX);
                    if bytes.len() <= index {
                        bytes.resize(index + 1, 0_u8);
                    }
                    bytes[index] |= 1 << (part % 8);
                }
                let hex = bytes.iter().fold(String::new(), |mut hex, byte| {
                    use std::fmt::Write;
                    let _ = write!(hex, "{byte:02x}");
                    hex
                });
                (cycle.clone(), hex)
            })
            .collect()
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<String, BTreeSet<u32>>, D::Error> {
        let encoded = BTreeMap::<String, String>::deserialize(deserializer)?;
        let mut parts = BTreeMap::new();
        for (cycle, hex) in encoded {
            if hex.len() % 2 != 0 || hex.len() > MAX_FISCAL_REPORT_PARTS_PER_CYCLE / 4 {
                return Err(serde::de::Error::custom(
                    "a fiscal part bitmap is malformed",
                ));
            }
            let mut set = BTreeSet::new();
            for (index, pair) in hex.as_bytes().chunks(2).enumerate() {
                let byte = std::str::from_utf8(pair)
                    .ok()
                    .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                    .ok_or_else(|| serde::de::Error::custom("a fiscal part bitmap is malformed"))?;
                for bit in 0..8_u32 {
                    if byte & (1 << bit) != 0 {
                        set.insert(u32::try_from(index).unwrap_or(u32::MAX) * 8 + bit);
                    }
                }
            }
            parts.insert(cycle, set);
        }
        Ok(parts)
    }
}

/// [`FISCAL_REPORT_ROWS_PER_PART`] consecutive rows of one cycle of one
/// observer's report. A row lists the facts of one scope's book of the
/// cycle, or, at [`FiscalReportGranularity::Institution`], of one
/// institution's books of one mechanism summed; a row with no book of the
/// cycle is left out, and a part with no row withdraws what its line held.
///
/// A part of scope rows cites, as evidence, the ledger versions its rows
/// were read from. A part of institution rows, summed over up to thousands
/// of books, cannot cite them all within the relations a knowledge record
/// may carry: it cites the current version of each office its rows belong
/// to and counts in `books_summed` the books it summed, which is weaker
/// provenance than a scope part's.
///
/// A part is encoded compactly: the institutions of its rows once each, in
/// an `institutions` table in the order its rows first name them, and each
/// row as `[row, institution, mechanism, scope_binding_id, facts]`, with
/// `institution` an index into the table and `scope_binding_id` empty for
/// an institution row, and each fact as `[unit, payment_form, assessed
/// minimum, assessed maximum, collected minimum, collected maximum,
/// outstanding minimum, outstanding maximum]`, arrays rather than objects,
/// so that a part's payload stays small. A [`FiscalReportRow`] on its own
/// keeps its object form.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "FiscalReportPartWire", into = "FiscalReportPartWire")]
pub struct FiscalReportPart {
    pub observer_id: String,
    pub accounting_cycle_id: String,
    pub slot: u16,
    /// The part's number within its cycle, counted from zero: it holds rows
    /// `part * 16` to `part * 16 + 15`.
    pub part: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rows: Vec<FiscalReportRow>,
    /// For a part of institution rows, the hot books with an assessment its
    /// rows were summed from when it was published; `None` for scope rows.
    /// A change of the count alone, below the observer's precision, does
    /// not republish the part.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub books_summed: Option<u32>,
}

/// The encoded form of a [`FiscalReportPart`], as its documentation describes.
#[derive(Clone, Deserialize, Serialize)]
struct FiscalReportPartWire {
    observer_id: String,
    accounting_cycle_id: String,
    slot: u16,
    part: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    institutions: Vec<EntityRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    rows: Vec<FiscalReportRowWire>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    books_summed: Option<u32>,
}

/// One encoded row: the row, the index of its institution in the part's
/// table, its mechanism, its scope (empty for an institution row), and its
/// facts.
#[derive(Clone, Deserialize, Serialize)]
struct FiscalReportRowWire(
    u32,
    u32,
    FiscalMechanism,
    String,
    Vec<FiscalReportRowFactWire>,
);

/// One encoded fact: its unit and payment form, and the minimum and maximum
/// of what was assessed, collected, and is outstanding.
#[derive(Clone, Deserialize, Serialize)]
struct FiscalReportRowFactWire(String, FiscalPaymentForm, u64, u64, u64, u64, u64, u64);

impl From<FiscalReportPart> for FiscalReportPartWire {
    fn from(part: FiscalReportPart) -> Self {
        let mut institutions: Vec<EntityRef> = Vec::new();
        let rows = part
            .rows
            .into_iter()
            .map(|row| {
                let index = institutions
                    .iter()
                    .position(|institution| *institution == row.institution)
                    .unwrap_or_else(|| {
                        institutions.push(row.institution);
                        institutions.len() - 1
                    });
                FiscalReportRowWire(
                    row.row,
                    u32::try_from(index).unwrap_or(u32::MAX),
                    row.mechanism,
                    row.scope_binding_id,
                    row.facts
                        .into_iter()
                        .map(|fact| {
                            FiscalReportRowFactWire(
                                fact.unit,
                                fact.payment_form,
                                fact.assessed.minimum,
                                fact.assessed.maximum,
                                fact.collected.minimum,
                                fact.collected.maximum,
                                fact.outstanding.minimum,
                                fact.outstanding.maximum,
                            )
                        })
                        .collect(),
                )
            })
            .collect();
        Self {
            observer_id: part.observer_id,
            accounting_cycle_id: part.accounting_cycle_id,
            slot: part.slot,
            part: part.part,
            institutions,
            rows,
            books_summed: part.books_summed,
        }
    }
}

impl TryFrom<FiscalReportPartWire> for FiscalReportPart {
    type Error = String;

    fn try_from(wire: FiscalReportPartWire) -> Result<Self, String> {
        let rows = wire
            .rows
            .into_iter()
            .map(
                |FiscalReportRowWire(row, institution, mechanism, scope, facts)| {
                    let institution = usize::try_from(institution)
                        .ok()
                        .and_then(|index| wire.institutions.get(index))
                        .cloned()
                        .ok_or_else(|| {
                            "a fiscal report row names an institution its part does not list"
                                .to_owned()
                        })?;
                    Ok(FiscalReportRow {
                        row,
                        institution,
                        mechanism,
                        scope_binding_id: scope,
                        facts: facts
                            .into_iter()
                            .map(
                                |FiscalReportRowFactWire(
                                    unit,
                                    payment_form,
                                    a0,
                                    a1,
                                    c0,
                                    c1,
                                    o0,
                                    o1,
                                )| {
                                    FiscalReportRowFact {
                                        unit,
                                        payment_form,
                                        assessed: FiscalAmountEstimate {
                                            minimum: a0,
                                            maximum: a1,
                                        },
                                        collected: FiscalAmountEstimate {
                                            minimum: c0,
                                            maximum: c1,
                                        },
                                        outstanding: FiscalAmountEstimate {
                                            minimum: o0,
                                            maximum: o1,
                                        },
                                    }
                                },
                            )
                            .collect(),
                    })
                },
            )
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Self {
            observer_id: wire.observer_id,
            accounting_cycle_id: wire.accounting_cycle_id,
            slot: wire.slot,
            part: wire.part,
            rows,
            books_summed: wire.books_summed,
        })
    }
}

/// One scope's facts of one cycle, or one institution's of one mechanism.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalReportRow {
    /// The row among the observer's rows.
    pub row: u32,
    pub institution: EntityRef,
    pub mechanism: FiscalMechanism,
    /// The scope of a scope row; empty, and left out, for an institution
    /// row.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub scope_binding_id: String,
    /// One per partition of the scope's book, in partition order; at most
    /// [`MAX_FISCAL_PARTITION_KEYS`].
    pub facts: Vec<FiscalReportRowFact>,
}

/// One partition's estimates in a [`FiscalReportRow`].
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalReportRowFact {
    pub unit: String,
    pub payment_form: FiscalPaymentForm,
    pub assessed: FiscalAmountEstimate,
    pub collected: FiscalAmountEstimate,
    pub outstanding: FiscalAmountEstimate,
}

/// One observer's report, assembled from the records its holder currently
/// holds.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalReport {
    /// The report's facts. For a complete report this equals what
    /// [`crate::compute_projection`] derives, at `as_of`, from the fiscal
    /// records the report was published from.
    pub projection: FiscalProjection,
    /// The parts, by cycle, that are not current; see
    /// [`FiscalReportSummary::pending`].
    pub pending: BTreeMap<String, BTreeSet<u32>>,
    /// Hot cycles the report does not show.
    pub omitted_cycles: u32,
}

impl FiscalReport {
    /// Whether every part of the report is current.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.pending.is_empty()
    }

    /// Whether the fact with this label belongs to a part that is not
    /// current, so that it may be older than the report's `as_of`.
    #[must_use]
    pub fn is_stale(&self, fact_id: &str) -> bool {
        FiscalReportFact::part_of(fact_id).is_some_and(|(cycle, part)| {
            self.pending
                .get(cycle)
                .is_some_and(|parts| parts.contains(&part))
        })
    }
}

/// The fiscal state of a run: the union of its fiscal records, as one value
/// to author, assemble, and audit.
///
/// A run keeps it as records (see [`crate::FiscalRecordKind`]):
/// [`FiscalState::into_records`] partitions an authored state, and
/// [`FiscalState::from_records`] assembles one, which costs a pass over every
/// fiscal record and is for hosts, tools, and tests. A boundary decodes and
/// writes only the records its packets touch.
///
/// Each scope's adoptions and the books of its hot accounting cycles are in
/// its [`FiscalScopeLedger`]; each institution's bindings, auditor policy and
/// proposals, and cycle bookkeeping in its [`FiscalOffice`]; each region's
/// adoption counts and transition candidates in its [`FiscalJurisdiction`];
/// each observer binding with its report bookkeeping in its
/// [`FiscalObserver`]. Closed books leave through the fiscal archive (see
/// [`crate::prepare_fiscal_archive`]). Nothing grows for the life of a
/// run: every action target keeps its recent outcomes in a
/// [`FiscalOutcomeRing`], and the archive keeps every archived book and
/// institution total for good.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalState {
    pub root: FiscalRoot,
    /// The fiscal archive head: what every applied commit has archived, the
    /// maintenance receipts, and the retention handles not yet acknowledged.
    /// The payload of the `canwu.fiscal:archive` record.
    #[serde(default, skip_serializing_if = "crate::FiscalArchiveHeadV1::is_unused")]
    pub archive: crate::FiscalArchiveHeadV1,
    /// Keyed by [`fiscal_office_key`].
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub offices: BTreeMap<String, FiscalOffice>,
    /// Keyed by scope binding ID.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ledgers: BTreeMap<String, FiscalScopeLedger>,
    /// Keyed by catalog region ID; one for each region a scope names.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub jurisdictions: BTreeMap<String, FiscalJurisdiction>,
    /// Keyed by observer binding ID.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub observers: BTreeMap<String, FiscalObserver>,
    /// The report work each observer is owed, keyed by observer binding ID.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub report_queue: BTreeMap<String, FiscalReportQueueEntry>,
    /// Where the report queue's work is, and the parked rotation. The
    /// payload of the `canwu.fiscal:report-schedule` record.
    #[serde(default, skip_serializing_if = "FiscalReportSchedule::is_empty")]
    pub report_schedule: FiscalReportSchedule,
}

impl FiscalState {
    #[must_use]
    pub fn new(year: i32, mode: FiscalHistoricalMode, at: SimTime) -> Self {
        Self {
            root: FiscalRoot {
                schema_version: FISCAL_RUNTIME_SCHEMA_VERSION,
                catalog_version: initial_catalog_version(),
                historical_context: FiscalHistoricalContext {
                    year,
                    mode,
                    updated_at: at,
                },
                context_revision: 1,
                execution_evidence_kinds: BTreeSet::new(),
                earliest_due: None,
                parked: false,
                parked_cursor: None,
            },
            archive: crate::FiscalArchiveHeadV1::default(),
            offices: BTreeMap::new(),
            ledgers: BTreeMap::new(),
            jurisdictions: BTreeMap::new(),
            observers: BTreeMap::new(),
            report_queue: BTreeMap::new(),
            report_schedule: FiscalReportSchedule::default(),
        }
    }

    /// The historical context of the run.
    #[must_use]
    pub fn historical_context(&self) -> &FiscalHistoricalContext {
        &self.root.historical_context
    }

    /// Adds a scope binding with an empty ledger, or replaces the binding of
    /// an existing one, making its office and jurisdiction when it has none.
    pub fn add_scope(&mut self, binding: FiscalScopeBinding) {
        self.office_mut(&binding.institution);
        self.jurisdictions
            .entry(binding.jurisdiction_id.clone())
            .or_insert_with(|| FiscalJurisdiction::new(binding.jurisdiction_id.clone()));
        match self.ledgers.get_mut(&binding.id) {
            Some(ledger) => {
                ledger.office = fiscal_office_key(&binding.institution);
                ledger.scope = binding;
            }
            None => {
                self.ledgers
                    .insert(binding.id.clone(), FiscalScopeLedger::new(binding));
            }
        }
    }

    /// Adds an adoption to the ledger of its scope.
    pub fn add_adoption(&mut self, adoption: FiscalAdoptionState) -> Result<(), CanwuError> {
        self.ledger_mut(&adoption.scope_binding_id)?
            .adoptions
            .insert(adoption.id.clone(), adoption);
        Ok(())
    }

    /// Adds an assessment to the book of its cycle in its scope's ledger,
    /// opening the book, and counting it in its office, when there is none.
    pub fn add_assessment(&mut self, assessment: FiscalAssessment) -> Result<(), CanwuError> {
        let ledger = self.ledger_mut(&assessment.scope_binding_id)?;
        let opened = !ledger.books.contains_key(&assessment.accounting_cycle_id);
        let book = ledger
            .books
            .entry(assessment.accounting_cycle_id.clone())
            .or_insert_with(|| FiscalCycleBook::open(assessment.accounting_cycle_id.clone()));
        book.assessments.insert(assessment.id.clone(), assessment);
        book.touch()?;
        if opened {
            self.recompute_offices();
        } else {
            self.recompute_partition_keys();
        }
        Ok(())
    }

    /// Adds a remission to the book of its assessment in `scope_binding_id`.
    pub fn add_remission(
        &mut self,
        scope_binding_id: &str,
        remission: FiscalRemission,
    ) -> Result<(), CanwuError> {
        let book = self.book_with_mut(scope_binding_id, |book| {
            book.assessments.contains_key(&remission.assessment_id)
        })?;
        book.remissions.insert(remission.id.clone(), remission);
        book.touch()
    }

    /// Adds an execution request to the book of its assessment in
    /// `scope_binding_id`.
    pub fn add_execution_request(
        &mut self,
        scope_binding_id: &str,
        request: FiscalExecutionRequest,
    ) -> Result<(), CanwuError> {
        let book = self.book_with_mut(scope_binding_id, |book| {
            book.assessments.contains_key(&request.assessment_id)
        })?;
        book.execution_requests.insert(request.id.clone(), request);
        book.touch()
    }

    /// Adds an execution receipt to the book of its request in
    /// `scope_binding_id`.
    pub fn add_execution_receipt(
        &mut self,
        scope_binding_id: &str,
        receipt: FiscalExecutionReceipt,
    ) -> Result<(), CanwuError> {
        let book = self.book_with_mut(scope_binding_id, |book| {
            book.execution_requests.contains_key(&receipt.request_id)
        })?;
        book.execution_receipts.insert(receipt.id.clone(), receipt);
        book.touch()
    }

    /// Adds an audit finding to the book of its target in
    /// `scope_binding_id`.
    pub fn add_audit(
        &mut self,
        scope_binding_id: &str,
        audit: FiscalAuditFinding,
    ) -> Result<(), CanwuError> {
        let cycle = self
            .ledger(scope_binding_id)
            .and_then(|ledger| ledger.book_of_audit_target(&audit.target_id))
            .map(|book| book.accounting_cycle_id.clone())
            .ok_or_else(|| invalid("fiscal audit target is not in its scope's books"))?;
        let book = self
            .ledger_mut(scope_binding_id)?
            .books
            .get_mut(&cycle)
            .ok_or_else(|| invalid("fiscal audit book is unavailable"))?;
        book.audits.insert(audit.id.clone(), audit);
        book.touch()
    }

    /// Adds an authority binding to the office of its institution.
    pub fn add_authority_binding(&mut self, binding: FiscalAuthorityBinding) {
        self.office_mut(&binding.institution)
            .authority_bindings
            .insert(binding.id.clone(), binding);
    }

    /// Adds an audit binding to the office of its audited institution.
    pub fn add_audit_binding(&mut self, binding: FiscalAuditBinding) {
        self.office_mut(&binding.institution)
            .audit_bindings
            .insert(binding.id.clone(), binding);
    }

    /// Sets the appointment policy of its institution's office.
    pub fn add_appointment_policy(&mut self, policy: FiscalAppointmentPolicy) {
        self.office_mut(&policy.appointer);
        let institution = policy.institution.clone();
        self.office_mut(&institution).appointment_policy = Some(policy);
    }

    /// Adds an auditor proposal to the office of its audited institution.
    pub fn add_audit_proposal(&mut self, proposal: FiscalAuditProposal) {
        self.office_mut(&proposal.institution)
            .audit_proposals
            .insert(proposal.id.clone(), proposal);
    }

    /// Adds an observer binding, or replaces one, with nothing published yet:
    /// its whole report is owed from its first due time, the start of the
    /// run for an `OnChange` binding and, for an `Interval` binding, the
    /// first aligned time at or after the time the state was authored for
    /// (its historical context's `updated_at`). Set `updated_at` to the
    /// run's start before adding interval bindings: with an earlier time,
    /// every first due time that falls before the start is past due, so all
    /// of those bindings are published by the first boundary, and their
    /// stagger takes effect only from their second due time.
    pub fn add_observer(&mut self, binding: FiscalObserverBinding) {
        for institution in binding
            .visible_institutions
            .iter()
            .chain(binding.controlled_by.iter())
            .cloned()
            .collect::<Vec<_>>()
        {
            self.office_mut(&institution);
        }
        let due_at = match binding.cadence {
            FiscalReportCadence::OnChange => SimTime::EPOCH,
            cadence @ FiscalReportCadence::Interval { .. } => cadence
                .due_at_or_after(self.root.historical_context.updated_at)
                .unwrap_or(SimTime::EPOCH),
        };
        self.report_queue.insert(
            binding.id.clone(),
            FiscalReportQueueEntry {
                due_at,
                whole: true,
                ..FiscalReportQueueEntry::default()
            },
        );
        self.observers
            .insert(binding.id.clone(), FiscalObserver::new(binding));
    }

    /// Removes an observer binding from an authored state, with its queued
    /// report work.
    pub fn remove_observer(&mut self, observer_binding_id: &str) {
        self.observers.remove(observer_binding_id);
        self.report_queue.remove(observer_binding_id);
    }

    fn book_with_mut(
        &mut self,
        scope_binding_id: &str,
        holds: impl Fn(&FiscalCycleBook) -> bool,
    ) -> Result<&mut FiscalCycleBook, CanwuError> {
        self.ledger_mut(scope_binding_id)?
            .books
            .values_mut()
            .find(|book| holds(book))
            .ok_or_else(|| invalid("fiscal object names nothing its scope's books hold"))
    }

    /// The ledger of one scope binding.
    #[must_use]
    pub fn ledger(&self, scope_binding_id: &str) -> Option<&FiscalScopeLedger> {
        self.ledgers.get(scope_binding_id)
    }

    pub(crate) fn ledger_mut(
        &mut self,
        scope_binding_id: &str,
    ) -> Result<&mut FiscalScopeLedger, CanwuError> {
        self.ledgers
            .get_mut(scope_binding_id)
            .ok_or_else(|| invalid(format!("fiscal scope {scope_binding_id} is unavailable")))
    }

    /// The binding of one scope.
    #[must_use]
    pub fn scope_binding(&self, scope_binding_id: &str) -> Option<&FiscalScopeBinding> {
        self.ledger(scope_binding_id).map(|ledger| &ledger.scope)
    }

    /// Every scope binding, in ID order.
    pub fn scope_bindings(&self) -> impl Iterator<Item = &FiscalScopeBinding> {
        self.ledgers.values().map(|ledger| &ledger.scope)
    }

    /// Every adoption, by scope and then adoption ID.
    pub fn adoptions(&self) -> impl Iterator<Item = &FiscalAdoptionState> {
        self.ledgers
            .values()
            .flat_map(|ledger| ledger.adoptions.values())
    }

    /// Every hot book with its ledger, by scope and then cycle.
    pub fn books(&self) -> impl Iterator<Item = (&FiscalScopeLedger, &FiscalCycleBook)> {
        self.ledgers
            .values()
            .flat_map(|ledger| ledger.books.values().map(move |book| (ledger, book)))
    }

    /// Every hot assessment.
    pub fn assessments(&self) -> impl Iterator<Item = &FiscalAssessment> {
        self.books().flat_map(|(_, book)| book.assessments.values())
    }

    /// Every hot remission.
    pub fn remissions(&self) -> impl Iterator<Item = &FiscalRemission> {
        self.books().flat_map(|(_, book)| book.remissions.values())
    }

    /// Every hot execution request.
    pub fn execution_requests(&self) -> impl Iterator<Item = &FiscalExecutionRequest> {
        self.books()
            .flat_map(|(_, book)| book.execution_requests.values())
    }

    /// Every hot execution receipt.
    pub fn execution_receipts(&self) -> impl Iterator<Item = &FiscalExecutionReceipt> {
        self.books()
            .flat_map(|(_, book)| book.execution_receipts.values())
    }

    /// Every hot audit finding.
    pub fn audits(&self) -> impl Iterator<Item = &FiscalAuditFinding> {
        self.books().flat_map(|(_, book)| book.audits.values())
    }

    /// The office of one institution, when it has one.
    #[must_use]
    pub fn office(&self, institution: &EntityRef) -> Option<&FiscalOffice> {
        self.offices.get(&fiscal_office_key(institution))
    }

    /// The office of one institution, made when it has none.
    pub fn office_mut(&mut self, institution: &EntityRef) -> &mut FiscalOffice {
        self.offices
            .entry(fiscal_office_key(institution))
            .or_insert_with(|| FiscalOffice::new(institution.clone()))
    }

    /// The jurisdiction of one region.
    #[must_use]
    pub fn jurisdiction(&self, region_id: &str) -> Option<&FiscalJurisdiction> {
        self.jurisdictions.get(region_id)
    }

    /// One observer binding.
    #[must_use]
    pub fn observer_binding(&self, observer_binding_id: &str) -> Option<&FiscalObserverBinding> {
        self.observers
            .get(observer_binding_id)
            .map(|observer| &observer.binding)
    }

    /// Every observer binding, in ID order.
    pub fn observer_bindings(&self) -> impl Iterator<Item = &FiscalObserverBinding> {
        self.observers.values().map(|observer| &observer.binding)
    }

    /// The authority binding `binding_id`, to change it.
    pub fn authority_binding_mut(
        &mut self,
        binding_id: &str,
    ) -> Option<&mut FiscalAuthorityBinding> {
        self.offices
            .values_mut()
            .find_map(|office| office.authority_bindings.get_mut(binding_id))
    }

    /// The authority binding `binding_id`, in whichever office holds it.
    #[must_use]
    pub fn authority_binding(&self, binding_id: &str) -> Option<&FiscalAuthorityBinding> {
        self.offices
            .values()
            .find_map(|office| office.authority_bindings.get(binding_id))
    }

    /// Every authority binding, by office and then ID.
    pub fn authority_bindings(&self) -> impl Iterator<Item = &FiscalAuthorityBinding> {
        self.offices
            .values()
            .flat_map(|office| office.authority_bindings.values())
    }

    /// The audit binding `binding_id`, in the office of its audited
    /// institution.
    #[must_use]
    pub fn audit_binding(&self, binding_id: &str) -> Option<&FiscalAuditBinding> {
        self.offices
            .values()
            .find_map(|office| office.audit_bindings.get(binding_id))
    }

    /// Every audit binding, by office and then ID.
    pub fn audit_bindings(&self) -> impl Iterator<Item = &FiscalAuditBinding> {
        self.offices
            .values()
            .flat_map(|office| office.audit_bindings.values())
    }

    /// The appointment policy `policy_id`.
    #[must_use]
    pub fn appointment_policy(&self, policy_id: &str) -> Option<&FiscalAppointmentPolicy> {
        self.appointment_policies()
            .find(|policy| policy.id == policy_id)
    }

    /// The appointment policy that governs the auditors of `institution`.
    #[must_use]
    pub fn appointment_policy_of(
        &self,
        institution: &EntityRef,
    ) -> Option<&FiscalAppointmentPolicy> {
        self.office(institution)
            .and_then(|office| office.appointment_policy.as_ref())
    }

    /// Every appointment policy, by office.
    pub fn appointment_policies(&self) -> impl Iterator<Item = &FiscalAppointmentPolicy> {
        self.offices
            .values()
            .filter_map(|office| office.appointment_policy.as_ref())
    }

    /// The auditor proposal `proposal_id`, in the office of its audited
    /// institution.
    #[must_use]
    pub fn audit_proposal(&self, proposal_id: &str) -> Option<&FiscalAuditProposal> {
        self.offices
            .values()
            .find_map(|office| office.audit_proposals.get(proposal_id))
    }

    /// Every auditor proposal, by office and then ID.
    pub fn audit_proposals(&self) -> impl Iterator<Item = &FiscalAuditProposal> {
        self.offices
            .values()
            .flat_map(|office| office.audit_proposals.values())
    }

    /// The revision of one action target, when it exists.
    #[must_use]
    pub fn target_revision(&self, target: &FiscalActionTarget) -> Option<u64> {
        match target {
            FiscalActionTarget::Ledger(scope) => self.ledger(scope).map(|ledger| ledger.revision),
            FiscalActionTarget::Office(institution) => {
                self.office(institution).map(|office| office.revision)
            }
            FiscalActionTarget::Jurisdiction(region) => self
                .jurisdiction(region)
                .map(|jurisdiction| jurisdiction.revision),
            FiscalActionTarget::Observer(observer) => self
                .observers
                .get(observer)
                .map(|observer| observer.revision),
        }
    }

    /// The outcome ring of one action target.
    #[must_use]
    pub fn outcome_ring(&self, target: &FiscalActionTarget) -> Option<&FiscalOutcomeRing> {
        match target {
            FiscalActionTarget::Ledger(scope) => self.ledger(scope).map(|ledger| &ledger.outcomes),
            FiscalActionTarget::Office(institution) => {
                self.office(institution).map(|office| &office.outcomes)
            }
            FiscalActionTarget::Jurisdiction(region) => self
                .jurisdiction(region)
                .map(|jurisdiction| &jurisdiction.outcomes),
            FiscalActionTarget::Observer(observer) => self
                .observers
                .get(observer)
                .map(|observer| &observer.outcomes),
        }
    }

    /// The recent outcome of `action_id` on `target`, while its ring keeps
    /// it; the outcome's event keeps it for good.
    #[must_use]
    pub fn recent_outcome(
        &self,
        target: &FiscalActionTarget,
        action_id: &str,
    ) -> Option<&FiscalRecentOutcome> {
        self.outcome_ring(target)?.get(action_id)
    }

    /// The ring and the revision of one action target, to record an outcome
    /// in.
    pub(crate) fn target_mut(
        &mut self,
        target: &FiscalActionTarget,
    ) -> Result<(&mut FiscalOutcomeRing, &mut u64), CanwuError> {
        match target {
            FiscalActionTarget::Ledger(scope) => {
                let ledger = self.ledger_mut(scope)?;
                Ok((&mut ledger.outcomes, &mut ledger.revision))
            }
            FiscalActionTarget::Office(institution) => {
                let office = self
                    .offices
                    .get_mut(&fiscal_office_key(institution))
                    .ok_or_else(|| invalid("fiscal office is unavailable"))?;
                Ok((&mut office.outcomes, &mut office.revision))
            }
            FiscalActionTarget::Jurisdiction(region) => {
                let jurisdiction = self
                    .jurisdictions
                    .get_mut(region)
                    .ok_or_else(|| invalid("fiscal jurisdiction is unavailable"))?;
                Ok((&mut jurisdiction.outcomes, &mut jurisdiction.revision))
            }
            FiscalActionTarget::Observer(observer) => {
                let observer = self
                    .observers
                    .get_mut(observer)
                    .ok_or_else(|| invalid("fiscal observer binding is unavailable"))?;
                Ok((&mut observer.outcomes, &mut observer.revision))
            }
        }
    }

    /// Records one outcome on its target at the target's revision, and
    /// advances the revision.
    pub(crate) fn record_outcome(
        &mut self,
        target: &FiscalActionTarget,
        action_id: &str,
        disposition: FiscalActionDisposition,
    ) -> Result<(), CanwuError> {
        let (ring, revision) = self.target_mut(target)?;
        ring.push(FiscalRecentOutcome {
            action_id: action_id.to_owned(),
            disposition,
            settled_revision: *revision,
        });
        *revision = revision
            .checked_add(1)
            .ok_or_else(|| invalid("fiscal record revision overflowed"))?;
        Ok(())
    }

    /// Sets each office's count of hot books per cycle, and the partition
    /// keys of its cycles, from the ledgers, making the offices that need
    /// one. What an office keeps of archived books (their totals and its
    /// watermark) is left as it is.
    pub fn recompute_offices(&mut self) {
        let mut counts = BTreeMap::<String, (EntityRef, BTreeMap<String, u32>)>::new();
        for ledger in self
            .ledgers
            .values()
            .filter(|ledger| !ledger.books.is_empty())
        {
            let institution = &ledger.scope.institution;
            let (_, cycles) = counts
                .entry(fiscal_office_key(institution))
                .or_insert_with(|| (institution.clone(), BTreeMap::new()));
            for cycle in ledger.books.keys() {
                *cycles.entry(cycle.clone()).or_default() += 1;
            }
        }
        for entry in self
            .offices
            .values_mut()
            .flat_map(|office| office.cycles.values_mut())
        {
            entry.hot_books = 0;
        }
        for (key, (institution, cycles)) in counts {
            let office = self
                .offices
                .entry(key)
                .or_insert_with(|| FiscalOffice::new(institution));
            for (cycle, hot_books) in cycles {
                office.cycles.entry(cycle).or_default().hot_books = hot_books;
            }
        }
        for office in self.offices.values_mut() {
            office.cycles.retain(|_, entry| entry.hot_books > 0);
        }
        self.recompute_partition_keys();
    }

    /// Adds to each office cycle the partition keys its hot books use.
    fn recompute_partition_keys(&mut self) {
        let mut keys =
            BTreeMap::<(String, String), Vec<(FiscalMechanism, FiscalPartitionKey)>>::new();
        for ledger in self.ledgers.values() {
            for (cycle, book) in &ledger.books {
                keys.entry((ledger.office.clone(), cycle.clone()))
                    .or_default()
                    .extend(
                        book.partition_keys()
                            .into_iter()
                            .map(|key| (ledger.scope.mechanism, key)),
                    );
            }
        }
        for ((office, cycle), used) in keys {
            if let Some(entry) = self
                .offices
                .get_mut(&office)
                .and_then(|office| office.cycles.get_mut(&cycle))
            {
                for (mechanism, key) in used {
                    entry
                        .partition_keys
                        .entry(mechanism)
                        .or_default()
                        .insert(key);
                }
            }
        }
    }

    /// Checks the whole state: every record on its own (the root, each
    /// office, ledger, jurisdiction, and observer, as settlement checks every
    /// record it writes), and every invariant across records, derived fields
    /// included.
    #[allow(clippy::too_many_lines)]
    pub fn validate(&self, catalog: &CompiledFiscalCatalog) -> Result<(), CanwuError> {
        validate_root(&self.root, catalog)?;
        self.archive.validate()?;
        if self.ledgers.len() > MAX_FISCAL_SCOPE_BINDINGS
            || self.observers.len() > MAX_FISCAL_OBSERVERS
        {
            return Err(invalid("fiscal runtime state exceeds its bounded capacity"));
        }
        // Binding and policy IDs are one namespace across offices, and
        // proposal IDs another: the binding directory resolves each to one
        // office.
        let mut bindings = BTreeSet::new();
        let mut proposals = BTreeSet::new();
        for (key, office) in &self.offices {
            if *key != fiscal_office_key(&office.institution) {
                return Err(invalid("a fiscal office is not keyed by its institution"));
            }
            validate_office(office)?;
            for id in office_binding_ids(office) {
                if !bindings.insert(id) {
                    return Err(invalid(format!(
                        "fiscal binding or policy ID {id} is used twice"
                    )));
                }
            }
            for id in office.audit_proposals.keys() {
                if !proposals.insert(id) {
                    return Err(invalid(format!("fiscal proposal ID {id} is used twice")));
                }
            }
        }
        for binding in self.audit_bindings() {
            if self.authority_binding(&binding.id).is_some() {
                return Err(invalid(
                    "fiscal audit bindings require an ID no authority binding or appointment policy uses",
                ));
            }
        }
        for proposal in self.audit_proposals() {
            if self.authority_binding(&proposal.audit_binding_id).is_some()
                || self
                    .appointment_policy(&proposal.audit_binding_id)
                    .is_some()
            {
                return Err(invalid(format!(
                    "audit proposal {} names an authority binding or policy ID",
                    proposal.id
                )));
            }
            if let FiscalAuditProposalKind::RemovalPetition { auditor, .. } = &proposal.kind
                && proposal.is_pending()
                && self
                    .audit_binding(&proposal.audit_binding_id)
                    .is_none_or(|binding| {
                        binding.institution != proposal.institution || binding.auditor != *auditor
                    })
            {
                return Err(invalid(format!(
                    "audit proposal {} is no longer apt while pending",
                    proposal.id
                )));
            }
        }
        for (key, ledger) in &self.ledgers {
            validate_identifier(key, "scope binding")?;
            if *key != ledger.scope.id {
                return Err(invalid("scope binding map key does not match its value"));
            }
            validate_ledger(ledger, catalog)?;
            if !self.offices.contains_key(&ledger.office)
                || !self
                    .jurisdictions
                    .contains_key(&ledger.scope.jurisdiction_id)
            {
                return Err(invalid(format!(
                    "fiscal scope {key} has no office or jurisdiction"
                )));
            }
        }
        for (key, jurisdiction) in &self.jurisdictions {
            if *key != jurisdiction.region_id {
                return Err(invalid(
                    "fiscal jurisdiction map key does not match its value",
                ));
            }
            validate_jurisdiction(jurisdiction, catalog)?;
        }
        for (key, observer) in &self.observers {
            if *key != observer.binding.id {
                return Err(invalid("observer binding map key does not match its value"));
            }
            validate_observer(observer)?;
            if observer
                .binding
                .visible_institutions
                .iter()
                .chain(observer.binding.controlled_by.iter())
                .any(|institution| self.office(institution).is_none())
            {
                return Err(invalid(format!(
                    "observer binding {key} names an institution without an office"
                )));
            }
        }
        for (observer_id, entry) in &self.report_queue {
            if !self.observers.contains_key(observer_id) {
                return Err(invalid(format!(
                    "the fiscal report queue names an unknown observer binding: {observer_id}"
                )));
            }
            validate_queue_entry(entry)?;
        }
        // An active interval binding always has its one entry, due at one of
        // its due times: the report system finds its next due time there,
        // and nowhere else.
        for (observer_id, observer) in &self.observers {
            let cadence = observer.binding.scheduled_cadence();
            let entry = self.report_queue.get(observer_id);
            if cadence.is_interval()
                && !entry.is_some_and(|entry| cadence.is_due_time(entry.due_at))
            {
                return Err(invalid(format!(
                    "interval observer binding {observer_id} has no report-queue entry at one of its due times"
                )));
            }
            // A removed binding is owed its withdrawal and nothing past it:
            // an entry of one that owes no withdrawal and names no work was
            // queued again after its removal.
            if !observer.binding.is_active()
                && observer.report.withdraw.is_empty()
                && entry.is_some_and(|entry| !entry.whole && entry.cycles.is_empty())
            {
                return Err(invalid(format!(
                    "removed observer binding {observer_id} is queued with nothing owed past its removal"
                )));
            }
        }
        validate_report_schedule(&self.report_schedule)?;
        self.validate_derived(catalog)?;
        if self
            .authority_bindings()
            .filter_map(|binding| binding.authority_basis.as_ref())
            .chain(
                self.audit_bindings()
                    .map(|binding| &binding.authority_basis),
            )
            .chain(
                self.audit_proposals()
                    .filter_map(|proposal| match &proposal.kind {
                        FiscalAuditProposalKind::Nomination {
                            authority_basis, ..
                        } => Some(authority_basis),
                        FiscalAuditProposalKind::RemovalPetition { .. } => None,
                    }),
            )
            .any(|basis| basis.record.kind.namespace == FiscalCatalogRecord::NAMESPACE)
        {
            return Err(invalid(
                "a fiscal authority basis must be an application record, not a fiscal record",
            ));
        }
        Ok(())
    }

    /// Checks that every derived field is what the rest of the state derives:
    /// the offices' scope counts, observer lists, and cycle bookkeeping, the
    /// ledgers' offices and ranks, the jurisdictions' counts and candidates
    /// (their evaluation times aside), the observers' rows, and the root's
    /// queue pages.
    fn validate_derived(&self, catalog: &CompiledFiscalCatalog) -> Result<(), CanwuError> {
        let mut derived = self.clone();
        derived.recompute_derived_fields(catalog, None);
        let unequal =
            |what: &str| invalid(format!("fiscal {what} differ from what the state derives"));
        for (key, office) in &self.offices {
            let expected = derived.offices.get(key).ok_or_else(|| unequal("offices"))?;
            if office.scope_count != expected.scope_count
                || office.mechanisms != expected.mechanisms
                || office.observers != expected.observers
                || office.cycles.keys().collect::<Vec<_>>()
                    != expected.cycles.keys().collect::<Vec<_>>()
                || office.cycles.iter().any(|(cycle, entry)| {
                    let expected = &expected.cycles[cycle];
                    entry.hot_books != expected.hot_books
                        || expected.partition_keys.iter().any(|(mechanism, keys)| {
                            entry
                                .partition_keys
                                .get(mechanism)
                                .is_none_or(|held| !held.is_superset(keys))
                        })
                })
            {
                return Err(unequal("office cycle bookkeeping or observer lists"));
            }
        }
        if derived.offices.len() != self.offices.len() {
            return Err(unequal("offices"));
        }
        for (key, ledger) in &self.ledgers {
            let expected = &derived.ledgers[key];
            if ledger.office != expected.office || ledger.rank != expected.rank {
                return Err(unequal("ledger offices or ranks"));
            }
        }
        if derived.jurisdictions.len() != self.jurisdictions.len() {
            return Err(unequal("jurisdictions"));
        }
        for (key, jurisdiction) in &self.jurisdictions {
            let expected = &derived.jurisdictions[key];
            let candidates = |jurisdiction: &FiscalJurisdiction| {
                jurisdiction
                    .transition_candidates
                    .values()
                    .map(|candidate| {
                        (
                            candidate.id.clone(),
                            candidate.transition_id.clone(),
                            candidate.historically_observed,
                        )
                    })
                    .collect::<Vec<_>>()
            };
            if jurisdiction.scope_count != expected.scope_count
                || jurisdiction.mechanisms != expected.mechanisms
                || jurisdiction.adoption_counts != expected.adoption_counts
                || candidates(jurisdiction) != candidates(expected)
            {
                return Err(unequal("jurisdiction counts or candidates"));
            }
        }
        for (key, observer) in &self.observers {
            if observer.rows != derived.observers[key].rows {
                return Err(unequal("observer rows"));
            }
        }
        // The chain from the queue pages to the root: each page's earliest
        // due time and parked count in the schedule, and the least due time
        // and whether any entry is parked in the root.
        if self.report_schedule != derived.report_schedule {
            return Err(unequal("report schedule page due times or parked pages"));
        }
        if self.root.earliest_due != derived.root.earliest_due
            || self.root.parked != derived.root.parked
        {
            return Err(unequal("root earliest due time or parked flag"));
        }
        Ok(())
    }

    /// Brings every derived field up to date: offices for every institution
    /// a scope or observer binding names, their scope counts, mechanisms,
    /// observer lists, and cycle bookkeeping; the ledgers' offices and ranks;
    /// a jurisdiction for every region a scope names, with its counts and
    /// its transition candidates as of `at`; the observers' rows; the report
    /// schedule's page due times and parked pages; and the root's earliest
    /// due time and parked flag. [`Self::into_records`] runs it. The first
    /// report of each observer is queued by [`Self::add_observer`].
    pub fn recompute_derived(&mut self, catalog: &CompiledFiscalCatalog, at: SimTime) {
        self.recompute_derived_fields(catalog, Some(at));
    }

    #[allow(clippy::too_many_lines)]
    fn recompute_derived_fields(&mut self, catalog: &CompiledFiscalCatalog, at: Option<SimTime>) {
        let institutions: Vec<EntityRef> = self
            .ledgers
            .values()
            .map(|ledger| ledger.scope.institution.clone())
            .chain(self.observers.values().flat_map(|observer| {
                observer
                    .binding
                    .visible_institutions
                    .iter()
                    .chain(observer.binding.controlled_by.iter())
                    .cloned()
            }))
            .collect();
        for institution in institutions {
            self.office_mut(&institution);
        }
        // Ranks: each office's scopes in ID order.
        let mut ranks = BTreeMap::<String, u32>::new();
        for ledger in self.ledgers.values_mut() {
            ledger.office = fiscal_office_key(&ledger.scope.institution);
            let rank = ranks.entry(ledger.office.clone()).or_default();
            ledger.rank = *rank;
            *rank += 1;
        }
        for (key, office) in &mut self.offices {
            office.scope_count = ranks.get(key).copied().unwrap_or_default();
            office.mechanisms.clear();
            office.observers.clear();
        }
        for ledger in self.ledgers.values() {
            if let Some(office) = self.offices.get_mut(&ledger.office) {
                office.mechanisms.insert(ledger.scope.mechanism);
            }
        }
        self.recompute_offices();
        // Jurisdictions.
        for ledger in self.ledgers.values() {
            self.jurisdictions
                .entry(ledger.scope.jurisdiction_id.clone())
                .or_insert_with(|| FiscalJurisdiction::new(ledger.scope.jurisdiction_id.clone()));
        }
        for jurisdiction in self.jurisdictions.values_mut() {
            jurisdiction.scope_count = 0;
            jurisdiction.mechanisms.clear();
            jurisdiction.adoption_counts.clear();
        }
        for ledger in self.ledgers.values() {
            let Some(jurisdiction) = self.jurisdictions.get_mut(&ledger.scope.jurisdiction_id)
            else {
                continue;
            };
            jurisdiction.scope_count += 1;
            *jurisdiction
                .mechanisms
                .entry(ledger.scope.mechanism)
                .or_default() += 1;
            add_adoption_counts(
                &mut jurisdiction.adoption_counts,
                ledger.adoptions.values(),
                1,
            );
        }
        let context = self.root.historical_context.clone();
        for jurisdiction in self.jurisdictions.values_mut() {
            let candidates = crate::derive::jurisdiction_candidates(
                jurisdiction,
                catalog,
                &context,
                at.unwrap_or(context.updated_at),
            );
            jurisdiction.transition_candidates = candidates;
        }
        // Observer rows.
        for observer in self.observers.values_mut() {
            let granularity = observer.binding.granularity;
            let mut first_row = 0_u32;
            observer.rows = observer
                .binding
                .visible_institutions
                .iter()
                .map(|institution| {
                    let office = fiscal_office_key(institution);
                    let (scopes, mechanisms) =
                        self.offices.get(&office).map_or((0, Vec::new()), |office| {
                            (
                                office.scope_count,
                                match granularity {
                                    FiscalReportGranularity::Scope => Vec::new(),
                                    FiscalReportGranularity::Institution => {
                                        office.mechanisms.iter().copied().collect()
                                    }
                                },
                            )
                        });
                    let rows = FiscalObserverRows {
                        institution: institution.clone(),
                        office,
                        first_row,
                        scopes,
                        mechanisms,
                    };
                    first_row = first_row.saturating_add(rows.span(granularity));
                    rows
                })
                .collect();
            for rows in &observer.rows {
                if let Some(office) = self.offices.get_mut(&rows.office) {
                    office.observers.insert(
                        observer.binding.id.clone(),
                        FiscalOfficeObserver {
                            first_row: rows.first_row,
                            granularity,
                            interval: observer.binding.cadence.is_interval(),
                        },
                    );
                }
            }
        }
        // The report schedule and the root's summary of it.
        let mut pages = BTreeMap::<String, Vec<&FiscalReportQueueEntry>>::new();
        for (observer, entry) in &self.report_queue {
            pages
                .entry(crate::records::report_queue_page(observer))
                .or_default()
                .push(entry);
        }
        let mut schedule = FiscalReportSchedule::default();
        for (page, entries) in pages {
            schedule.update_page(&page, entries);
        }
        (self.root.earliest_due, self.root.parked) = schedule.root_fields();
        if !self.root.parked {
            self.root.parked_cursor = None;
        }
        self.report_schedule = schedule;
    }
}

/// Adds `sign` times the adoptions to per-rule counts, dropping a rule whose
/// counts reach zero.
pub(crate) fn add_adoption_counts<'a>(
    counts: &mut BTreeMap<String, FiscalAdoptionCount>,
    adoptions: impl IntoIterator<Item = &'a FiscalAdoptionState>,
    sign: i8,
) {
    for adoption in adoptions {
        let count = counts.entry(adoption.rule_id.clone()).or_default();
        let step = |value: &mut u32| {
            *value = if sign > 0 {
                value.saturating_add(1)
            } else {
                value.saturating_sub(1)
            };
        };
        if adoption.stage != FiscalAdoptionStage::Repealed {
            step(&mut count.live);
        }
        if adoption.stage.is_operational() {
            step(&mut count.operational);
        }
    }
    counts.retain(|_, count| count.live > 0 || count.operational > 0);
}

/// The binding and policy IDs one office holds, which the binding directory
/// resolves to it: its authority and audit bindings and its policy. Its
/// proposal IDs are resolved too, in a namespace of their own.
pub(crate) fn office_binding_ids(office: &FiscalOffice) -> impl Iterator<Item = &String> {
    office
        .authority_bindings
        .keys()
        .chain(office.audit_bindings.keys())
        .chain(office.appointment_policy.iter().map(|policy| &policy.id))
}

/// Checks one observer's queued report work: within
/// [`FISCAL_REPORT_QUEUE_CYCLES`] cycles of [`FISCAL_REPORT_QUEUE_PARTS`]
/// parts, or, for what is left of a split publication, within the report's
/// cycle slots and parts; every part in range, and no cycle named beside a
/// whole report.
pub(crate) fn validate_queue_entry(entry: &FiscalReportQueueEntry) -> Result<(), CanwuError> {
    let (cycles, parts) = if entry.republish {
        (
            MAX_FISCAL_REPORT_CYCLE_SLOTS,
            MAX_FISCAL_REPORT_PARTS_PER_CYCLE,
        )
    } else {
        (FISCAL_REPORT_QUEUE_CYCLES, FISCAL_REPORT_QUEUE_PARTS)
    };
    if entry.cycles.len() > cycles
        || entry.whole && !entry.cycles.is_empty()
        || entry.cycles.values().any(|owed| {
            owed.len() > parts
                || owed.iter().any(|part| {
                    usize::try_from(*part)
                        .map_or(true, |part| part >= MAX_FISCAL_REPORT_PARTS_PER_CYCLE)
                })
        })
    {
        return Err(invalid(
            "a fiscal report-queue entry names more cycles or parts than it may",
        ));
    }
    entry
        .cycles
        .keys()
        .try_for_each(|cycle| validate_identifier(cycle, "fiscal accounting cycle"))
}

/// Checks the root record.
pub(crate) fn validate_root(
    root: &FiscalRoot,
    catalog: &CompiledFiscalCatalog,
) -> Result<(), CanwuError> {
    if root.schema_version != FISCAL_RUNTIME_SCHEMA_VERSION {
        return Err(invalid("unsupported fiscal runtime schema"));
    }
    if root.context_revision == 0 {
        return Err(invalid("fiscal context revision must be nonzero"));
    }
    if root.execution_evidence_kinds.len() > MAX_FISCAL_EVIDENCE_KINDS {
        return Err(invalid("fiscal root approves too many evidence kinds"));
    }
    if root.catalog_version != initial_catalog_version()
        || root.historical_context.year != catalog.historical_year
            && root.historical_context.updated_at == SimTime::EPOCH
    {
        return Err(invalid(
            "fiscal state is not bound to the selected initial catalog",
        ));
    }
    if !catalog
        .historical_scope
        .contains(root.historical_context.year)
    {
        return Err(invalid(
            "fiscal historical context lies outside the catalog scope",
        ));
    }
    if root.parked_cursor.is_some() && !root.parked {
        return Err(invalid(
            "the fiscal root names a parked cursor with nothing parked",
        ));
    }
    if let Some(cursor) = &root.parked_cursor {
        validate_identifier(cursor, "observer binding")?;
    }
    Ok(())
}

/// Checks the report schedule record on its own: bounded, well-named pages,
/// each listed as parked with at least one parked entry.
pub(crate) fn validate_report_schedule(schedule: &FiscalReportSchedule) -> Result<(), CanwuError> {
    if schedule.page_due.len() > FISCAL_REPORT_QUEUE_PAGES
        || schedule.parked_pages.len() > FISCAL_REPORT_QUEUE_PAGES
        || schedule.parked_pages.values().any(|parked| *parked == 0)
    {
        return Err(invalid(
            "the fiscal report schedule names too many pages, or an empty parked page",
        ));
    }
    for page in schedule.page_due.keys().chain(schedule.parked_pages.keys()) {
        validate_identifier(page, "fiscal report queue page")?;
    }
    Ok(())
}

/// Checks one office record on its own: its key fields, ring, bindings,
/// policy, proposals, and cycle bookkeeping.
#[allow(clippy::too_many_lines)]
pub(crate) fn validate_office(office: &FiscalOffice) -> Result<(), CanwuError> {
    if office.revision == 0 {
        return Err(invalid("fiscal office revision must be nonzero"));
    }
    office.outcomes.validate(office.revision)?;
    if let Some(watermark) = &office.watermark {
        validate_identifier(watermark, "fiscal accounting cycle")?;
    }
    if office.authority_bindings.len() > MAX_FISCAL_AUTHORITY_BINDINGS_PER_OFFICE
        || office.audit_bindings.len() > MAX_FISCAL_AUDIT_BINDINGS_PER_OFFICE
        || office.observers.len() > MAX_FISCAL_OBSERVERS_PER_INSTITUTION
        || office.cycles.len() > MAX_FISCAL_HOT_CYCLES_PER_OFFICE
    {
        return Err(invalid("fiscal office exceeds its bounded capacity"));
    }
    validate_map(
        &office.authority_bindings,
        |value| &value.id,
        "authority binding",
    )?;
    validate_map(&office.audit_bindings, |value| &value.id, "audit binding")?;
    if office.authority_bindings.values().any(|binding| {
        binding.institution != office.institution
            || binding.acting_actor.is_some() != binding.authority_basis.is_some()
            || binding.acting_actor.is_some() && binding.acting_actor == binding.authorized_actor
            || binding
                .authority_basis
                .as_ref()
                .is_some_and(|basis| basis.version == 0)
    }) {
        return Err(invalid(
            "fiscal authority bindings belong to their office, and acting actors require one exact authority basis and differ from the authorized actor",
        ));
    }
    if office.audit_bindings.values().any(|binding| {
        binding.institution != office.institution || binding.authority_basis.version == 0
    }) {
        return Err(invalid(
            "fiscal audit bindings belong to the office of their audited institution and require one exact authority basis",
        ));
    }
    for observer in office.observers.keys() {
        validate_identifier(observer, "observer binding")?;
    }
    for (cycle, entry) in &office.cycles {
        validate_identifier(cycle, "fiscal accounting cycle")?;
        if entry.hot_books == 0
            || entry
                .partition_keys
                .values()
                .any(|keys| keys.len() > MAX_FISCAL_PARTITION_KEYS)
            || !entry
                .archived_totals
                .windows(2)
                .all(|pair| pair[0].key() < pair[1].key())
        {
            return Err(invalid(
                "a fiscal office cycle has no hot book, too many partition keys, or unordered archived totals",
            ));
        }
    }
    validate_office_appointments(office)
}

/// Checks the policy and proposals of one office: a policy of the office's
/// institution with bounded party lists; proposals on that institution, by
/// a listed party, with bounded grounds; at most
/// [`MAX_FISCAL_PENDING_PROPOSALS_PER_PARTY`] pending for one party, each of
/// a different kind or binding, and at most
/// [`MAX_FISCAL_RESOLVED_PROPOSALS_PER_POLICY`] resolved; and every pending
/// petition still apt: its binding still has the auditor it names, since
/// any change of that auditor supersedes the petition.
fn validate_office_appointments(office: &FiscalOffice) -> Result<(), CanwuError> {
    if let Some(policy) = &office.appointment_policy {
        validate_identifier(&policy.id, "appointment policy")?;
        if policy.institution != office.institution
            || policy.nominators.len() > MAX_FISCAL_APPOINTMENT_PARTIES
            || policy.petitioners.len() > MAX_FISCAL_APPOINTMENT_PARTIES
        {
            return Err(invalid(
                "a fiscal appointment policy governs its office's institution and lists a bounded number of parties",
            ));
        }
    }
    validate_map(&office.audit_proposals, |value| &value.id, "audit proposal")?;
    if office.audit_proposals.is_empty() {
        return Ok(());
    }
    let Some(policy) = &office.appointment_policy else {
        return Err(invalid(
            "audit proposals concern an institution no appointment policy governs",
        ));
    };
    let mut pending = BTreeMap::<_, BTreeSet<_>>::new();
    let mut resolved = 0_usize;
    for proposal in office.audit_proposals.values() {
        let binding = office.audit_bindings.get(&proposal.audit_binding_id);
        let (listed, bounded, apt) = match &proposal.kind {
            FiscalAuditProposalKind::Nomination {
                authority_basis, ..
            } => (
                policy.nominators.contains(&proposal.party),
                authority_basis.version != 0,
                true,
            ),
            FiscalAuditProposalKind::RemovalPetition { auditor, grounds } => (
                policy.petitioners.contains(&proposal.party),
                !grounds.is_empty() && grounds.len() <= MAX_FISCAL_EVIDENCE_PER_RECORD,
                binding.is_some_and(|binding| binding.auditor == *auditor),
            ),
        };
        if proposal.institution != office.institution
            || !listed
            || !bounded
            || validate_identifier(&proposal.audit_binding_id, "audit binding").is_err()
            || proposal.is_pending() && !apt
        {
            return Err(invalid(format!(
                "audit proposal {} is invalid or no longer apt while pending",
                proposal.id
            )));
        }
        if proposal.is_pending() {
            let nominates = matches!(proposal.kind, FiscalAuditProposalKind::Nomination { .. });
            if !pending
                .entry(&proposal.party)
                .or_default()
                .insert((nominates, &proposal.audit_binding_id))
            {
                return Err(invalid(format!(
                    "audit proposal {} duplicates a pending proposal of its party",
                    proposal.id
                )));
            }
        } else {
            resolved += 1;
        }
    }
    if pending
        .values()
        .any(|proposals| proposals.len() > MAX_FISCAL_PENDING_PROPOSALS_PER_PARTY)
        || resolved > MAX_FISCAL_RESOLVED_PROPOSALS_PER_POLICY
    {
        return Err(invalid(
            "fiscal audit proposals exceed the pending room of a party or the resolved room of a policy",
        ));
    }
    Ok(())
}

/// Checks one jurisdiction record on its own.
pub(crate) fn validate_jurisdiction(
    jurisdiction: &FiscalJurisdiction,
    catalog: &CompiledFiscalCatalog,
) -> Result<(), CanwuError> {
    if jurisdiction.revision == 0 || !catalog.regions.contains_key(&jurisdiction.region_id) {
        return Err(invalid(format!(
            "fiscal jurisdiction {} has no revision or no catalog region",
            jurisdiction.region_id
        )));
    }
    jurisdiction.outcomes.validate(jurisdiction.revision)?;
    validate_map(
        &jurisdiction.transition_candidates,
        |value| &value.id,
        "transition candidate",
    )?;
    if jurisdiction
        .adoption_counts
        .keys()
        .any(|rule| !catalog.rules.contains_key(rule))
        || jurisdiction
            .transition_candidates
            .values()
            .any(|candidate| candidate.jurisdiction_id != jurisdiction.region_id)
    {
        return Err(invalid(
            "a fiscal jurisdiction counts an unknown rule or holds another region's candidate",
        ));
    }
    Ok(())
}

/// Checks one observer record on its own: bounded confidence and
/// visibility, a holder that is the actor's person or, without an actor, a
/// government, its ring, its rows, and its report bookkeeping.
pub(crate) fn validate_observer(observer: &FiscalObserver) -> Result<(), CanwuError> {
    let binding = &observer.binding;
    validate_identifier(&binding.id, "observer binding")?;
    let holder_matches = match (binding.actor, &binding.knowledge_holder) {
        (Some(actor), KnowledgeHolderRef::Person(person)) => actor == *person,
        (None, KnowledgeHolderRef::Entity(EntityRef::Government(_))) => true,
        _ => false,
    };
    if observer.revision == 0
        || binding.confidence_per_mille > 1_000
        || !holder_matches
        || binding.visible_institutions.len() > MAX_FISCAL_VISIBLE_INSTITUTIONS
        || usize::try_from(observer.row_count())
            .map_or(true, |rows| rows > MAX_FISCAL_OBSERVER_ROWS)
    {
        return Err(invalid(
            "fiscal observers require a revision, bounded confidence, rows, and visibility, and their actor's person or a government as holder",
        ));
    }
    observer.outcomes.validate(observer.revision)?;
    binding.cadence.validate()?;
    let report = &observer.report;
    let mut slots = BTreeSet::new();
    for (cycle, slot) in &report.cycles {
        validate_identifier(cycle, "fiscal accounting cycle")?;
        if usize::from(slot.slot) >= MAX_FISCAL_REPORT_CYCLE_SLOTS
            || !slots.insert(slot.slot)
            || slot.parts.len() % PART_ENTRY_DIGITS != 0
            || slot.parts.len() / PART_ENTRY_DIGITS > MAX_FISCAL_REPORT_PARTS_PER_CYCLE
            || !slot
                .parts
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || slot.parts.ends_with(EMPTY_PART_ENTRY)
        {
            return Err(invalid(
                "a fiscal observer's cycle slots are out of range, shared, or malformed",
            ));
        }
    }
    report
        .omitted_cycles
        .iter()
        .try_for_each(|cycle| validate_identifier(cycle, "fiscal accounting cycle"))
}

/// Checks one scope ledger on its own: its scope, revision, ring, adoptions,
/// books, and every per-assessment invariant of its books.
#[allow(clippy::too_many_lines)]
pub(crate) fn validate_ledger(
    ledger: &FiscalScopeLedger,
    catalog: &CompiledFiscalCatalog,
) -> Result<(), CanwuError> {
    let scope = &ledger.scope;
    validate_identifier(&scope.id, "scope binding")?;
    if !catalog.regions.contains_key(&scope.jurisdiction_id) {
        return Err(invalid(format!(
            "scope {} references unavailable jurisdiction {}",
            scope.id, scope.jurisdiction_id
        )));
    }
    // A report row carries the scope's institution, so its domain reference
    // is bounded like the other texts of a row.
    if matches!(
        &scope.institution,
        EntityRef::Domain(reference)
            if reference.kind.namespace.len()
                + reference.kind.name.len()
                + reference.id.len()
                > MAX_FISCAL_IDENTIFIER_BYTES
    ) {
        return Err(invalid(
            "a fiscal institution reference is longer than a report row may carry",
        ));
    }
    if ledger.revision == 0 || ledger.office != fiscal_office_key(&scope.institution) {
        return Err(invalid(format!(
            "fiscal ledger {} has no revision or names another office",
            scope.id
        )));
    }
    if let Some(watermark) = &ledger.watermark {
        validate_identifier(watermark, "fiscal accounting cycle")?;
    }
    if ledger.books.len() > MAX_FISCAL_HOT_BOOKS_PER_LEDGER
        || ledger.adoptions.len() > MAX_FISCAL_ADOPTIONS_PER_LEDGER
    {
        return Err(invalid(format!(
            "fiscal ledger {} exceeds its bounded capacity",
            scope.id
        )));
    }
    ledger.outcomes.validate(ledger.revision)?;
    validate_map(&ledger.adoptions, |value| &value.id, "adoption")?;
    for adoption in ledger.adoptions.values() {
        let rule = catalog
            .rules
            .get(&adoption.rule_id)
            .ok_or_else(|| invalid(format!("adoption {} references unknown rule", adoption.id)))?;
        if adoption.scope_binding_id != scope.id
            || rule.mechanism != scope.mechanism
            || adoption.generation == 0
        {
            return Err(invalid(format!(
                "adoption {} is inconsistent with its scope or generation",
                adoption.id
            )));
        }
    }
    let operational_rules: BTreeSet<_> = ledger
        .adoptions
        .values()
        .filter(|adoption| adoption.stage.is_operational())
        .map(|adoption| adoption.rule_id.as_str())
        .collect();
    // Object IDs are unique among the scope's hot objects of their kind,
    // and evidence and external operations settle at most one receipt of
    // the scope's hot books.
    let mut ids: [BTreeSet<&str>; 5] = Default::default();
    let mut consumed_evidence = BTreeSet::new();
    let mut consumed_operations = BTreeSet::new();
    for (cycle, book) in &ledger.books {
        validate_identifier(cycle, "fiscal accounting cycle")?;
        if *cycle != book.accounting_cycle_id {
            return Err(invalid("fiscal book map key does not match its cycle"));
        }
        if book.assessments.is_empty()
            || book.assessments.len() > MAX_FISCAL_PARTITION_KEYS
            || book.remissions.len() > MAX_FISCAL_REMISSIONS_PER_BOOK
            || book.execution_requests.len() > MAX_FISCAL_EXECUTION_REQUESTS_PER_BOOK
            || book.execution_receipts.len() > MAX_FISCAL_EXECUTION_RECEIPTS_PER_BOOK
            || book.audits.len() > MAX_FISCAL_AUDITS_PER_BOOK
            || book.revision == 0
        {
            return Err(invalid(format!(
                "fiscal book {cycle} of scope {} is empty, unrevised, or exceeds its bounded capacity",
                scope.id
            )));
        }
        validate_map(&book.assessments, |value| &value.id, "assessment")?;
        validate_map(&book.remissions, |value| &value.id, "remission")?;
        validate_map(
            &book.execution_requests,
            |value| &value.id,
            "execution request",
        )?;
        validate_map(
            &book.execution_receipts,
            |value| &value.id,
            "execution receipt",
        )?;
        validate_map(&book.audits, |value| &value.id, "audit")?;
        for (set, keys) in ids.iter_mut().zip([
            book.assessments.keys().collect::<Vec<_>>(),
            book.remissions.keys().collect(),
            book.execution_requests.keys().collect(),
            book.execution_receipts.keys().collect(),
            book.audits.keys().collect(),
        ]) {
            if keys.into_iter().any(|key| !set.insert(key.as_str())) {
                return Err(invalid(format!(
                    "a fiscal object ID of scope {} is used by two of its hot books",
                    scope.id
                )));
            }
        }
        if let FiscalBookStatus::Closed { closed_by } = &book.status {
            validate_identifier(&closed_by.action_id, "fiscal action")?;
        }
        let mut partitions = BTreeSet::new();
        for assessment in book.assessments.values() {
            let rule = catalog.rules.get(&assessment.rule_id).ok_or_else(|| {
                invalid(format!(
                    "assessment {} references unknown rule",
                    assessment.id
                ))
            })?;
            if assessment.quantity == 0
                || validate_identifier(&assessment.unit, "fiscal unit").is_err()
                || assessment.scope_binding_id != scope.id
                || assessment.accounting_cycle_id != book.accounting_cycle_id
                || rule.mechanism != scope.mechanism
                || !rule.payment_forms.contains(&assessment.payment_form)
            {
                return Err(invalid(format!("assessment {} is invalid", assessment.id)));
            }
            if !operational_rules.contains(assessment.rule_id.as_str()) {
                return Err(invalid(format!(
                    "assessment {} uses a rule that is not operational in its scope",
                    assessment.id
                )));
            }
            if !partitions.insert((assessment.unit.as_str(), assessment.payment_form)) {
                return Err(invalid(
                    "the same fiscal accounting partition was assessed more than once",
                ));
            }
        }
        let mut remitted_by_assessment = BTreeMap::<&str, u64>::new();
        for remission in book.remissions.values() {
            if remission.quantity == 0 || !book.assessments.contains_key(&remission.assessment_id) {
                return Err(invalid(format!("remission {} is invalid", remission.id)));
            }
            checked_add(
                remitted_by_assessment
                    .entry(&remission.assessment_id)
                    .or_default(),
                remission.quantity,
                "remission total",
            )?;
        }
        let mut collection_requested_by_assessment = BTreeMap::<&str, u64>::new();
        for request in book.execution_requests.values() {
            let assessment = book
                .assessments
                .get(&request.assessment_id)
                .ok_or_else(|| {
                    invalid(format!(
                        "execution request {} has no assessment",
                        request.id
                    ))
                })?;
            if request.quantity == 0
                || request.unit != assessment.unit
                || request.payment_form != assessment.payment_form
                || request.resource.get() == 0
                || request.source == request.target
                || request.institution != scope.institution
                || request.source != request.institution && request.target != request.institution
            {
                return Err(invalid(format!(
                    "execution request {} is invalid",
                    request.id
                )));
            }
            if request.kind == FiscalExecutionKind::Collect {
                checked_add(
                    collection_requested_by_assessment
                        .entry(&request.assessment_id)
                        .or_default(),
                    request.quantity,
                    "collection request total",
                )?;
            }
        }
        for assessment in book.assessments.values() {
            let remitted = remitted_by_assessment
                .get(assessment.id.as_str())
                .copied()
                .unwrap_or_default();
            let requested = collection_requested_by_assessment
                .get(assessment.id.as_str())
                .copied()
                .unwrap_or_default();
            if remitted > assessment.quantity || requested > assessment.quantity - remitted {
                return Err(invalid(format!(
                    "assessment {} is over-remitted or double-claimed",
                    assessment.id
                )));
            }
        }
        let mut fulfilled_by_request = BTreeMap::<&str, u64>::new();
        for receipt in book.execution_receipts.values() {
            let counts_as_fulfillment = receipt.disposition.counts_as_fulfillment();
            if receipt.external_evidence.is_empty()
                || receipt.external_evidence.len() > MAX_FISCAL_EVIDENCE_PER_RECORD
                || receipt.external_operations.len() != receipt.external_evidence.len()
                || !book.execution_requests.contains_key(&receipt.request_id)
                || counts_as_fulfillment == (receipt.quantity == 0)
            {
                return Err(invalid(format!(
                    "execution receipt {} is invalid",
                    receipt.id
                )));
            }
            if receipt
                .external_evidence
                .iter()
                .any(|evidence| !consumed_evidence.insert(evidence.clone()))
            {
                return Err(invalid(
                    "one external execution result cannot settle multiple fiscal receipts",
                ));
            }
            if receipt.external_operations.iter().any(|operation| {
                operation.external_operation_id.trim().is_empty()
                    || !consumed_operations.insert(operation.clone())
            }) {
                return Err(invalid(
                    "one external fiscal operation cannot settle multiple receipts",
                ));
            }
            if counts_as_fulfillment {
                checked_add(
                    fulfilled_by_request.entry(&receipt.request_id).or_default(),
                    receipt.quantity,
                    "execution receipt total",
                )?;
            }
        }
        for request in book.execution_requests.values() {
            if fulfilled_by_request
                .get(request.id.as_str())
                .copied()
                .unwrap_or_default()
                > request.quantity
            {
                return Err(invalid(format!(
                    "execution request {} was fulfilled more than once",
                    request.id
                )));
            }
        }
        for audit in book.audits.values() {
            if audit.evidence.len() > MAX_FISCAL_EVIDENCE_PER_RECORD
                || !book.assessments.contains_key(&audit.target_id)
                    && !book.execution_requests.contains_key(&audit.target_id)
                    && !book.execution_receipts.contains_key(&audit.target_id)
            {
                return Err(invalid(format!("audit {} has an unknown target", audit.id)));
            }
        }
    }
    Ok(())
}

fn checked_add(target: &mut u64, value: u64, label: &str) -> Result<(), CanwuError> {
    *target = target
        .checked_add(value)
        .ok_or_else(|| invalid(format!("{label} overflowed")))?;
    Ok(())
}

pub(crate) fn validate_identifier(value: &str, label: &str) -> Result<(), CanwuError> {
    if value.is_empty()
        || value.len() > MAX_FISCAL_IDENTIFIER_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(invalid(format!(
            "{label} identity is not canonical: {value}"
        )));
    }
    Ok(())
}

fn validate_map<T>(
    values: &BTreeMap<String, T>,
    id: impl Fn(&T) -> &String,
    label: &str,
) -> Result<(), CanwuError> {
    for (key, value) in values {
        validate_identifier(key, label)?;
        if key != id(value) {
            return Err(invalid(format!("{label} map key does not match its value")));
        }
    }
    Ok(())
}

fn validate_refs<T>(
    refs: &BTreeSet<String>,
    values: &BTreeMap<String, T>,
    label: &str,
) -> Result<(), CanwuError> {
    if let Some(missing) = refs
        .iter()
        .find(|reference| !values.contains_key(*reference))
    {
        return Err(invalid(format!("{label} {missing} is unavailable")));
    }
    Ok(())
}

pub(crate) fn invalid(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidDomainRecord, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(pending: BTreeMap<String, BTreeSet<u32>>) -> FiscalReportSummary {
        FiscalReportSummary {
            observer_id: "observer".to_owned(),
            actor: None,
            as_of: SimTime::EPOCH,
            confidence_per_mille: 1_000,
            granularity: FiscalReportGranularity::Scope,
            cadence: FiscalReportCadence::OnChange,
            cycles: BTreeMap::new(),
            fact_count: 0,
            pending,
            omitted_cycles: 0,
            withdrawn: false,
        }
    }

    #[test]
    fn pending_parts_round_trip_as_a_bitmap_per_cycle() {
        let last = u32::try_from(MAX_FISCAL_REPORT_PARTS_PER_CYCLE).expect("parts") - 1;
        let pending = BTreeMap::from([
            ("a".to_owned(), BTreeSet::from([0, 7, 8, 9, last])),
            ("b".to_owned(), BTreeSet::from([3])),
            ("c".to_owned(), (0..=last).collect()),
        ]);
        let value = serde_json::to_value(summary(pending.clone())).expect("summary");
        assert_eq!(value["pending"]["b"], "08");
        assert_eq!(
            value["pending"]["c"].as_str().expect("bitmap").len(),
            MAX_FISCAL_REPORT_PARTS_PER_CYCLE / 4
        );
        let back: FiscalReportSummary = serde_json::from_value(value).expect("decoded");
        assert_eq!(back.pending, pending);
    }

    #[test]
    fn a_malformed_bitmap_is_refused() {
        let mut value = serde_json::to_value(summary(BTreeMap::new())).expect("summary");
        for bitmap in [
            "0".to_owned(),
            "zz".to_owned(),
            "00".repeat(MAX_FISCAL_REPORT_PARTS_PER_CYCLE / 8 + 1),
        ] {
            value["pending"] = serde_json::json!({ "a": bitmap });
            assert!(serde_json::from_value::<FiscalReportSummary>(value.clone()).is_err());
        }
    }

    /// Interval due times are `offset + k * every` from the epoch, never
    /// before `offset`; `staggered` spreads offsets evenly and wraps; and an
    /// interval without a positive period, or with an offset outside it, is
    /// refused.
    #[test]
    fn interval_due_times_are_aligned_to_the_epoch_and_staggered() {
        let day = SimDuration::days(1);
        let every = SimDuration::days(30);
        let cadence = FiscalReportCadence::staggered(every, 7, 30);
        assert_eq!(
            cadence,
            FiscalReportCadence::Interval {
                every,
                offset: SimDuration::days(7),
            }
        );
        let at = |days: i64, minutes: i64| SimTime::from_minutes(days * 1_440 + minutes);
        assert_eq!(cadence.due_at_or_after(at(-40, 0)), Some(at(7, 0)));
        assert_eq!(cadence.due_at_or_after(at(7, 0)), Some(at(7, 0)));
        assert_eq!(cadence.due_at_or_after(at(7, 1)), Some(at(37, 0)));
        assert_eq!(cadence.due_after(at(7, 0)), Some(at(37, 0)));
        assert_eq!(cadence.due_after(at(36, 1_439)), Some(at(37, 0)));
        assert!(cadence.is_due_time(at(67, 0)));
        assert!(!cadence.is_due_time(at(67, 1)) && !cadence.is_due_time(at(-23, 0)));
        assert_eq!(
            FiscalReportCadence::OnChange.due_at_or_after(at(3, 5)),
            Some(at(3, 5))
        );
        // Thirty observers a day apart; the thirty-first shares the first's
        // day, and a stride that does not divide the period rounds down.
        for index in 0..60_u64 {
            assert_eq!(
                FiscalReportCadence::staggered(every, index, 30),
                FiscalReportCadence::Interval {
                    every,
                    offset: SimDuration::days(i64::try_from(index % 30).expect("day")),
                }
            );
        }
        assert_eq!(
            FiscalReportCadence::staggered(day, 1, 7),
            FiscalReportCadence::Interval {
                every: day,
                offset: SimDuration::minutes(1_440 / 7),
            }
        );
        for invalid in [
            FiscalReportCadence::Interval {
                every: SimDuration::ZERO,
                offset: SimDuration::ZERO,
            },
            FiscalReportCadence::Interval {
                every: day,
                offset: day,
            },
            FiscalReportCadence::Interval {
                every: day,
                offset: SimDuration::minutes(-1),
            },
        ] {
            assert!(invalid.validate().is_err(), "{invalid:?}");
            assert_eq!(invalid.due_at_or_after(SimTime::EPOCH), None);
        }
    }

    #[test]
    fn an_office_key_names_its_institution_alone() {
        let first = fiscal_office_key(&EntityRef::Government(canwu_api::GovernmentId::new(1)));
        let second = fiscal_office_key(&EntityRef::Government(canwu_api::GovernmentId::new(2)));
        assert_ne!(first, second);
        for key in [&first, &second] {
            let digits = key.strip_prefix("office.").expect("prefix");
            assert_eq!(digits.len(), 32);
            assert!(digits.bytes().all(|byte| byte.is_ascii_hexdigit()));
        }
        assert_eq!(
            first,
            fiscal_office_key(&EntityRef::Government(canwu_api::GovernmentId::new(1)))
        );
    }
}
