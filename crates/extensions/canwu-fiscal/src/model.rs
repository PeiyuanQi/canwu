use crate::PLUGIN_NAME;
use canwu_api::{
    CanwuError, DomainRecord, DomainRecordClass, DomainRecordDraft, DomainRecordKind,
    DomainRecordLifecycle, DomainRecordType, DomainRecordVersionRef, DomainRecordVersionSource,
    DomainReference, DomainReferenceTarget, DomainValueKindClass, EntityRef, ErrorCode,
    EvidenceRef, IngressId, KnowledgeHolderRef, PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD,
    PayloadRequiredEvidenceContinuationV1, PersonId, ResourceId, SimTime, SimulationGranularity,
    TypedDomainRecordRef,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const FISCAL_CONTENT_SCHEMA_VERSION: u32 = 1;
/// Version of the serialized [`FiscalState`]. Version 4 keeps each scope's
/// adoptions and accounting cycle books in its [`FiscalScopeLedger`], the
/// cycle bookkeeping of each institution in its [`FiscalOffice`], the recent
/// outcomes of each action target in its [`FiscalOutcomeRing`], and the
/// fiscal archive head; observers may hold their reports without a person.
pub const FISCAL_RUNTIME_SCHEMA_VERSION: u32 = 4;
/// Authority bindings, audit bindings, and scope bindings, each counted on
/// its own. Audit binding IDs are one namespace across all institutions and
/// share this one cap.
pub const MAX_FISCAL_RUNTIME_BINDINGS: usize = 4_096;
pub const MAX_FISCAL_OBSERVERS: usize = 64;
/// Hot assessments: those of the books the state holds. Remissions, audits,
/// and aggregates share the number. An archive commit takes closed books out
/// of the state, so this bounds what a run holds at once, not what it makes:
/// past it a further assessment is rejected as its own outcome until books
/// are archived.
pub const MAX_FISCAL_ASSESSMENTS: usize = 4_096;
/// Hot execution requests, bounded like [`MAX_FISCAL_ASSESSMENTS`].
pub const MAX_FISCAL_EXECUTION_REQUESTS: usize = 8_192;
/// Hot execution receipts, bounded like [`MAX_FISCAL_ASSESSMENTS`].
pub const MAX_FISCAL_EXECUTION_RECEIPTS: usize = 8_192;
/// Accounting cycle books, open or closed, one scope ledger holds at once. A
/// further book is refused with [`FISCAL_HOT_BOOKS_EXHAUSTED`] until a closed
/// one is archived.
pub const MAX_FISCAL_HOT_BOOKS_PER_LEDGER: usize = 64;
/// Recent outcomes one action target keeps in its [`FiscalOutcomeRing`].
/// Every outcome, applied or rejected, is also recorded for good in its
/// `canwu.fiscal.action_settled.v1` event.
pub const FISCAL_RECENT_OUTCOMES: usize = 8;
/// Fiscal actions one boundary applies. Only an action that is current and
/// authorized, and so reaches the copy and validation of the whole state,
/// counts; a stale or unauthorized action is rejected without using the
/// limit. A further such action in the same boundary is recorded as a
/// `Rejected` outcome whose reason ends with [`FISCAL_BOUNDARY_ACTION_LIMIT`],
/// so one boundary's fiscal work stays bounded however many commands were
/// admitted for it.
pub const MAX_FISCAL_ACTIONS_PER_BOUNDARY: usize = 64;
/// Longest identifier, in bytes, that a fiscal record or action may carry.
/// The same length bounds every text of a report fact: an assessment's unit
/// and accounting cycle, and the namespace, name, and ID of a scope's
/// domain-record institution together.
pub const MAX_FISCAL_IDENTIFIER_BYTES: usize = 160;
/// Longest free text, in bytes, that a fiscal action may carry: an audit
/// `finding`, a remission `reason`, or an execution `purpose`. Admission
/// rejects a longer one as `InvalidPayload`.
pub const MAX_FISCAL_ACTION_TEXT_BYTES: usize = 2_048;
/// Largest serialized fiscal action request, in bytes, that admission
/// accepts. It bounds every field no narrower limit names, such as entity
/// references and evidence references.
pub const MAX_FISCAL_ACTION_JSON_BYTES: usize = 32 * 1_024;
/// Facts one part of a fiscal report holds. An observer's report is published
/// as one summary record and parts of this many facts each, so a boundary
/// republishes only the parts whose facts changed. A full part of ordinary
/// identifiers is about 6 KiB. It still fits the engine's 65,536-byte
/// knowledge payload limit, at 55,916 bytes, when every text of every fact is
/// at its longest and made only of characters JSON has to escape.
pub const FISCAL_REPORT_FACTS_PER_PART: usize = 16;
/// Parts one observer's report can have. Every fact reports one aggregate and
/// every aggregate has one assessment, so [`MAX_FISCAL_ASSESSMENTS`] bounds
/// the facts and a report of any size the state allows fits its parts.
pub const MAX_FISCAL_REPORT_PARTS: usize = MAX_FISCAL_ASSESSMENTS / FISCAL_REPORT_FACTS_PER_PART;
/// Fiscal report records one boundary publishes, over all observers: about a
/// tenth of the engine's per-boundary knowledge record limit, which every
/// plugin shares. Parts that changed but do not fit are owed: the boundary
/// schedules an internal `fiscal_report_drain_v1` packet, due at once, so the
/// next boundary publishes them without waiting for a change of the fiscal
/// state, up to this many records again. Until then the observer's summary
/// names them in [`FiscalReportSummary::pending_parts`].
///
/// The limit is reached only when one boundary has more records than this to
/// publish, and then an observer can infer a little that its binding does not
/// show it: the records are shared in the order of the observer bindings, so
/// which parts of a report are pending depends on the reports of the
/// observers before it.
pub const MAX_FISCAL_REPORT_RECORDS_PER_BOUNDARY: usize = 1_024;
pub const MAX_FISCAL_EVIDENCE_KINDS: usize = 32;
pub const MAX_FISCAL_EVIDENCE_PER_RECORD: usize = 32;
/// Parties one [`FiscalAppointmentPolicy`] lists as nominators, and as
/// petitioners, each.
pub const MAX_FISCAL_APPOINTMENT_PARTIES: usize = 32;
/// Nominations and removal petitions that one party of one
/// [`FiscalAppointmentPolicy`] may have pending at once, over all the
/// policy's bindings. Admission refuses a further one as `ValueOutOfRange`,
/// unless it replaces the party's pending proposal of the same kind on the
/// same binding. A policy lists at most [`MAX_FISCAL_APPOINTMENT_PARTIES`]
/// nominators and as many petitioners, so it has at most
/// `2 * MAX_FISCAL_APPOINTMENT_PARTIES * MAX_FISCAL_PENDING_PROPOSALS_PER_PARTY`
/// proposals pending, and no party fills another's room.
pub const MAX_FISCAL_PENDING_PROPOSALS_PER_PARTY: usize = 4;
/// Resolved nominations and petitions kept for one policy, as the record of
/// who proposed, on what, and who decided. A resolution past it drops the
/// policy's oldest resolved proposals (by the time they were resolved, then
/// by ID), so a run never runs out of room for proposals; a binding and a
/// finding keep their nominator in [`FiscalProposalRef`] either way.
pub const MAX_FISCAL_RESOLVED_PROPOSALS_PER_POLICY: usize = 64;
/// Serialized byte budget of the fiscal state record.
///
/// Settlement holds every record its boundary writes, leaving out the report
/// plan, to this budget less [`MAX_FISCAL_REPORT_PLAN_BYTES`] and
/// [`FISCAL_ARCHIVE_HEAD_RESERVE_BYTES`]: the state as settlement writes it,
/// and as the candidate and aggregate systems of the same boundary rewrite it
/// with their derived fields. Any report plan fits in what is left, and so
/// does any growth of the archive head that an archive commit or
/// acknowledgement makes, so neither a derived write nor archive maintenance
/// can pass the budget, however much a change grows the derived fields.
///
/// An action whose result does not fit is rejected as its own outcome with
/// [`FISCAL_STATE_BUDGET_EXHAUSTED`]. An execution receipt or a
/// historical-context change that does not fit changes nothing, and one
/// `canwu.fiscal.ingress_rejected.v1` event whose summary ends with
/// [`FISCAL_INGRESS_BUDGET_REJECTION`] is its only record. The packets around
/// it and the boundary are unaffected. Admission does not look at the
/// budget: near it, commands are still accepted, and each is rejected at
/// settlement while a rejection fits and dropped after that.
pub const MAX_FISCAL_STATE_JSON_BYTES: usize = 32 * 1_024 * 1_024;
/// The most bytes a report plan can add to the serialized fiscal state: its
/// key, its revision, and both of its maps listing every part of every
/// observer binding under the longest binding ID. A valid plan never takes
/// more, so settlement leaves this much of [`MAX_FISCAL_STATE_JSON_BYTES`]
/// free for whatever plan the aggregate system, or a later boundary that
/// publishes owed parts, writes.
pub const MAX_FISCAL_REPORT_PLAN_BYTES: usize = {
    // `,"report_plan":{"revision":` and its closing brace around a revision
    // of at most 20 digits, and the keys `,"publish":` and `,"deferred":`.
    let frame = 96;
    // One observer's entry in one map: the quoted binding ID, a colon,
    // brackets, a separating comma, and its part numbers, each below
    // `MAX_FISCAL_REPORT_PARTS` and so at most three digits and a comma.
    let entry = MAX_FISCAL_IDENTIFIER_BYTES + 6 + MAX_FISCAL_REPORT_PARTS * 4;
    // The withdrawals: the key `,"withdraw":`, one entry per observer, and
    // at most one holder per action the boundary applied, each at most
    // `{"type":"entity","value":{"type":"government","id":` and 20 digits,
    // two braces, and a comma.
    let withdrawals = 16
        + MAX_FISCAL_OBSERVERS * (MAX_FISCAL_IDENTIFIER_BYTES + 6)
        + MAX_FISCAL_ACTIONS_PER_BOUNDARY * 96;
    frame + 2 * MAX_FISCAL_OBSERVERS * entry + withdrawals
};
const _: () = assert!(MAX_FISCAL_REPORT_PARTS <= 1_000);
const _: () = assert!(MAX_FISCAL_REPORT_PLAN_BYTES < MAX_FISCAL_STATE_JSON_BYTES / 64);
/// Room settlement leaves in [`MAX_FISCAL_STATE_JSON_BYTES`] for the growth of
/// the fiscal archive head: the retention handles of every pending archive
/// commit (each lists up to 193 object IDs of 64 hexadecimal digits) and the
/// maintenance receipts recorded with them. Archive maintenance is never
/// refused for the budget; this reserve is why it never has to be.
pub const FISCAL_ARCHIVE_HEAD_RESERVE_BYTES: usize = 2 * 1_024 * 1_024;
const _: () = assert!(FISCAL_ARCHIVE_HEAD_RESERVE_BYTES < MAX_FISCAL_STATE_JSON_BYTES / 8);
pub const MAX_FISCAL_CATALOG_PERIODS: usize = 512;
pub const MAX_FISCAL_CATALOG_REGIONS: usize = 1_024;
pub const MAX_FISCAL_CATALOG_DEFINITIONS: usize = 16_384;
pub const MAX_FISCAL_CATALOG_COVERAGE_CELLS: usize = 65_536;
pub const MAX_FISCAL_REFERENCES_PER_DEFINITION: usize = 1_024;
pub const MAX_FISCAL_CATALOG_JSON_BYTES: usize = 64 * 1_024 * 1_024;
const ROOT_ID: &str = "primary";
/// The reference role under which the state record names its catalog.
pub(crate) const FISCAL_STATE_CATALOG_ROLE: &str = "catalog";
/// The reference role under which the state record names the person of each
/// observer binding.
pub(crate) const FISCAL_STATE_OBSERVER_ROLE: &str = "observer";

pub struct FiscalCatalogRecord;

impl DomainRecordType for FiscalCatalogRecord {
    type Payload = CompiledFiscalCatalog;
    type Class = DomainValueKindClass;

    const NAMESPACE: &'static str = "canwu.fiscal";
    const NAME: &'static str = "catalog";
}

pub struct FiscalStateRecord;

impl DomainRecordType for FiscalStateRecord {
    type Payload = FiscalState;
    type Class = DomainValueKindClass;

    const NAMESPACE: &'static str = "canwu.fiscal";
    const NAME: &'static str = "state";
}

#[must_use]
pub fn fiscal_catalog_reference() -> TypedDomainRecordRef<FiscalCatalogRecord> {
    TypedDomainRecordRef::new(ROOT_ID)
}

#[must_use]
pub fn fiscal_state_reference() -> TypedDomainRecordRef<FiscalStateRecord> {
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

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
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

/// One fiscal procedure action and the binding its issuer acts under.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalActionRequest {
    pub action_id: String,
    /// The binding the issuer acts under: an authority binding of the
    /// institution the action concerns, or, for [`FiscalAction::RecordAudit`]
    /// only, an audit binding on that institution. For the appointment actions
    /// it is an authority binding of a party the governing
    /// [`FiscalAppointmentPolicy`] lists, or, for a person the policy lists by
    /// name, the policy's own ID.
    pub authority_binding_id: String,
    /// The procedure revision the issuer decided against. The action is stale,
    /// and settles as a `Rejected` outcome, unless this equals the procedure
    /// revision at the start of the boundary that settles it. Every action a
    /// boundary settles is compared with that one revision, so actions
    /// admitted against the same revision settle together, in admission order,
    /// each validated against the state the previous one left. A later action
    /// is also stale when it would overwrite an adoption, authority binding,
    /// or audit binding that an earlier action of the same boundary already
    /// changed.
    ///
    /// An action settles in the boundary after the one that admits its
    /// command. A receipt, a historical-context change, or another action
    /// that settles in the admitting boundary therefore advances the revision
    /// first and makes the action stale. Within the settling boundary neither
    /// a receipt nor a historical-context change makes an action stale,
    /// wherever the canonical ingress order puts it; the action is validated
    /// against the state they left. Archive maintenance never advances the
    /// revision.
    ///
    /// Admission refuses, without consuming anything, a revision above the
    /// current one ([`FISCAL_REVISION_UNKNOWN`]) and one at or below the
    /// outcome floor of the action's target ([`FISCAL_REVISION_EXPIRED`]):
    /// such a request can only be a re-send of an action whose outcome has
    /// left the target's [`FiscalOutcomeRing`].
    pub expected_procedure_revision: u64,
    pub action: FiscalAction,
}

/// The state an action decides on and keeps its outcome in: its target.
///
/// Every action has one target, and the target's [`FiscalOutcomeRing`] keeps
/// the action's recent outcome. Action IDs are per target: two targets may
/// use one action ID, and a host that wants its IDs unique across targets
/// provides that itself.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "type", content = "id", rename_all = "snake_case")]
pub enum FiscalActionTarget {
    /// The scope ledger of one scope binding: the target of the actions on a
    /// scope's adoptions and books.
    Ledger(String),
    /// The office of one institution: the target of the actions on its
    /// authority and audit bindings, its auditor proposals, and its
    /// transitions.
    Office(EntityRef),
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
            Self::Observer(observer) => write!(formatter, "observer:{observer}"),
        }
    }
}

/// The key of an institution's [`FiscalOffice`] in [`FiscalState::offices`]:
/// `office.` and the first 32 hexadecimal digits of a domain-separated BLAKE3
/// digest of the institution's reference. The office holds its institution,
/// and validation requires the key to match it, so two institutions whose
/// digests agree in those digits are an invalid state, never a merged office.
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
/// [`MAX_FISCAL_ACTIONS_PER_BOUNDARY`] actions.
pub const FISCAL_BOUNDARY_ACTION_LIMIT: &str =
    "fiscal action exceeds the per-boundary action limit";

/// Stable rejection message used for an action whose result would not fit
/// [`MAX_FISCAL_STATE_JSON_BYTES`].
pub const FISCAL_STATE_BUDGET_EXHAUSTED: &str =
    "fiscal action would exceed the fiscal state byte budget";

/// Reason that ends the summary of a `canwu.fiscal.ingress_rejected.v1`
/// event for an execution receipt or a historical-context change whose
/// result would not fit [`MAX_FISCAL_STATE_JSON_BYTES`]. The packet changes
/// nothing: a receipt consumes neither its evidence versions nor its external
/// operations, and the procedure revision does not advance.
pub const FISCAL_INGRESS_BUDGET_REJECTION: &str = "state_budget_exhausted";

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
/// the procedure revision does not advance.
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

/// Stable rejection message used at admission for a request whose
/// `expected_procedure_revision` is above the current procedure revision: it
/// was never a revision of the state.
pub const FISCAL_REVISION_UNKNOWN: &str =
    "fiscal action names a procedure revision the state never had";
/// Stable rejection message used at admission for a request whose
/// `expected_procedure_revision` is at or below its target's outcome floor:
/// a re-send of an action whose outcome left the target's ring.
pub const FISCAL_REVISION_EXPIRED: &str =
    "fiscal action names a procedure revision at or below its target's outcome floor";
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
/// institution. The holder must exist: the state record references the
/// person or government, and the engine refuses a state that references an
/// entity its registry does not hold. While a person holder is dead, nothing
/// is published to the binding and what changes in its report is kept in
/// [`FiscalReportPlan::deferred`].
///
/// The binding's report lines are numbered from its `line_slot`, which the
/// scenario allocates once per binding: two bindings never share a slot, and
/// a removed binding keeps its own, so re-pointing or removing one binding
/// never moves another's lines. A binding with `controlled_by` can be
/// re-pointed ([`FiscalAction::RebindObserver`]) and removed
/// ([`FiscalAction::RemoveObserver`]) by that institution; what it may see is
/// the scenario's and does not change.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalObserverBinding {
    pub id: String,
    pub actor: Option<PersonId>,
    pub knowledge_holder: KnowledgeHolderRef,
    pub visible_institutions: BTreeSet<EntityRef>,
    pub confidence_per_mille: u16,
    /// Below [`MAX_FISCAL_OBSERVERS`], and unique among the state's bindings.
    pub line_slot: u16,
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
    /// The recent outcomes of the actions on this binding.
    #[serde(default, skip_serializing_if = "FiscalOutcomeRing::is_empty")]
    pub outcomes: FiscalOutcomeRing,
}

impl FiscalObserverBinding {
    /// Whether the binding is still published to.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.removed.is_none()
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
    /// The procedure revision when the outcome was recorded, before the
    /// outcome advanced it.
    pub settled_revision: u64,
}

/// The newest outcomes of one action target, and its outcome floor.
///
/// Every outcome, applied or rejected, is recorded here and advances the
/// procedure revision. The ring keeps the newest
/// [`FISCAL_RECENT_OUTCOMES`]; an outcome it evicts raises `floor` to that
/// outcome's settled revision. Admission refuses a request whose action ID
/// is in the ring (`IdempotencyConflict`), and one whose expected revision
/// is at or below the floor ([`FISCAL_REVISION_EXPIRED`]): every outcome was
/// admitted with an expected revision no greater than its settled revision,
/// so a re-send of an evicted outcome's request is refused, while a fresh
/// request names the current revision, which is above every recorded
/// outcome's. Settlement skips a copy of an action whose ID has an outcome
/// in the ring or among the outcomes the boundary has settled for the same
/// target. Together they apply an action at most once, without keeping
/// every outcome for the life of the state.
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

    fn validate(&self, procedure_revision: u64) -> Result<(), CanwuError> {
        let mut ids = BTreeSet::new();
        if self.recent.len() > FISCAL_RECENT_OUTCOMES
            || self.floor >= procedure_revision
            || self.recent.iter().any(|outcome| {
                validate_identifier(&outcome.action_id, "fiscal action").is_err()
                    || !ids.insert(&outcome.action_id)
                    || outcome.settled_revision >= procedure_revision
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
                "a fiscal outcome ring is unbounded, unordered, repeats an action, or lies past the procedure revision",
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
/// distinct (unit, payment form) partitions. The book's totals per partition
/// are the state's aggregates of its scope and cycle (see
/// [`FiscalState::aggregates`]), and are archived with it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalCycleBook {
    pub accounting_cycle_id: String,
    pub status: FiscalBookStatus,
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

    /// The scope-level totals of the book, one per (unit, payment form)
    /// partition, in partition order, under the scope's `mechanism`.
    pub fn totals(&self, mechanism: FiscalMechanism) -> Result<Vec<FiscalCycleTotal>, CanwuError> {
        let mut totals = BTreeMap::<(&str, FiscalPaymentForm), FiscalCycleTotal>::new();
        let mut partitions = BTreeMap::new();
        for assessment in self.assessments.values() {
            let key = (assessment.unit.as_str(), assessment.payment_form);
            partitions.insert(assessment.id.as_str(), key);
            let total = totals.entry(key).or_insert_with(|| {
                FiscalCycleTotal::empty(mechanism, &assessment.unit, assessment.payment_form)
            });
            total.assessed = sum(total.assessed, assessment.quantity)?;
        }
        for remission in self.remissions.values() {
            let key = partitions
                .get(remission.assessment_id.as_str())
                .ok_or_else(|| invalid("fiscal remission has no assessment in its book"))?;
            let total = totals
                .get_mut(key)
                .ok_or_else(|| invalid("fiscal remission partition is unavailable"))?;
            total.remission_granted = sum(total.remission_granted, remission.quantity)?;
        }
        let mut requests = BTreeMap::new();
        for request in self.execution_requests.values() {
            let key = partitions
                .get(request.assessment_id.as_str())
                .ok_or_else(|| invalid("fiscal execution request has no assessment in its book"))?;
            requests.insert(request.id.as_str(), (*key, request.kind));
        }
        for receipt in self
            .execution_receipts
            .values()
            .filter(|receipt| receipt.disposition.counts_as_fulfillment())
        {
            let (key, kind) = requests
                .get(receipt.request_id.as_str())
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
            *target = sum(*target, receipt.quantity)?;
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

fn sum(left: u64, right: u64) -> Result<u64, CanwuError> {
    left.checked_add(right)
        .ok_or_else(|| invalid("fiscal cycle total overflowed"))
}

/// One scope binding with everything the state holds for it: its adoptions,
/// the books of its hot accounting cycles, its watermark, and the recent
/// outcomes of the actions on it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalScopeLedger {
    pub scope: FiscalScopeBinding,
    /// Adoptions of rules in this scope, keyed by adoption ID.
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
    /// A ledger with nothing but its scope binding.
    #[must_use]
    pub fn new(scope: FiscalScopeBinding) -> Self {
        Self {
            scope,
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

/// The cycle bookkeeping of one institution, and the recent outcomes of the
/// actions on its bindings, proposals, and transitions.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalOffice {
    pub institution: EntityRef,
    /// Each accounting cycle of which some scope of the institution holds a
    /// hot book: how many do, how many were archived, and the totals of those
    /// archived. A cycle leaves when its last hot book is archived, by the
    /// archive commit that also archives the institution's totals of it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub cycles: BTreeMap<String, FiscalOfficeCycle>,
    /// The greatest accounting cycle, in byte order, whose institution totals
    /// the archive holds. No scope of the institution makes a new book at or
    /// below it, so those totals are archived once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watermark: Option<String>,
    /// The recent outcomes of the actions targeting the office.
    #[serde(default, skip_serializing_if = "FiscalOutcomeRing::is_empty")]
    pub outcomes: FiscalOutcomeRing,
}

impl FiscalOffice {
    #[must_use]
    pub fn new(institution: EntityRef) -> Self {
        Self {
            institution,
            cycles: BTreeMap::new(),
            watermark: None,
            outcomes: FiscalOutcomeRing::default(),
        }
    }

    fn is_empty(&self) -> bool {
        self.cycles.is_empty() && self.watermark.is_none() && self.outcomes.is_empty()
    }
}

/// One hot accounting cycle of one institution.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalOfficeCycle {
    /// Scopes of the institution that hold a hot book of the cycle; above
    /// zero.
    pub hot_books: u32,
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalTransitionCandidate {
    pub id: String,
    pub transition_id: String,
    pub jurisdiction_id: String,
    pub historically_observed: bool,
    pub evaluated_at: SimTime,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalStrategicAggregate {
    pub institution: EntityRef,
    pub mechanism: FiscalMechanism,
    pub scope_binding_id: String,
    pub accounting_cycle_id: String,
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalAmountEstimate {
    pub minimum: u64,
    pub maximum: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalReportFact {
    /// The fact's label in a [`FiscalProjection`]: `fact.NNNN`, by its place
    /// among the observer's facts in aggregate-ID order. Aggregates keep their
    /// IDs, so a fact keeps its label while later aggregates are added. A
    /// fact inside a published [`FiscalReportPart`] has no label yet; the
    /// field is empty there and omitted from the record.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    pub institution: EntityRef,
    pub mechanism: FiscalMechanism,
    pub scope_binding_id: String,
    pub accounting_cycle_id: String,
    pub unit: String,
    pub payment_form: FiscalPaymentForm,
    pub assessed: FiscalAmountEstimate,
    pub collected: FiscalAmountEstimate,
    pub outstanding: FiscalAmountEstimate,
}

/// Everything one observer is told: what [`crate::compute_projections`]
/// derives from a fiscal state, and what
/// [`crate::assemble_fiscal_reports`] puts back together from the report
/// records the observer holds.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalProjection {
    /// The observer binding's actor; `None` for a report an institution
    /// holds.
    pub actor: Option<PersonId>,
    pub as_of: SimTime,
    pub confidence_per_mille: u16,
    pub facts: BTreeMap<String, FiscalReportFact>,
}

/// The payload of one fiscal report knowledge record (schema version 3).
///
/// An observer's report is one summary and up to [`MAX_FISCAL_REPORT_PARTS`]
/// parts, each its own line of records: a record supersedes the previous
/// record of the same line and no other. A binding's lines are numbered from
/// its [`FiscalObserverBinding::line_slot`], so no two bindings share one. Read a report with
/// [`crate::fiscal_reports`] or [`crate::assemble_fiscal_reports`] rather
/// than from single records.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FiscalReportRecord {
    Summary(FiscalReportSummary),
    Part(FiscalReportPart),
}

/// The head of one observer's report. It is republished whenever one of the
/// report's parts is, and never otherwise, so `as_of` is the time the report
/// last changed.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalReportSummary {
    /// The observer binding the report belongs to.
    pub observer_id: String,
    pub actor: Option<PersonId>,
    pub as_of: SimTime,
    pub confidence_per_mille: u16,
    /// Facts the report holds once every part is current.
    pub fact_count: u32,
    /// Parts the report holds once every part is current.
    pub part_count: u32,
    /// The numbers of the parts whose facts changed but which no boundary has
    /// published yet, because a boundary publishes at most
    /// [`MAX_FISCAL_REPORT_RECORDS_PER_BOUNDARY`] report records. The records
    /// the observer holds for them are older, or missing. Empty for a
    /// complete report.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub pending_parts: BTreeSet<u32>,
    /// Whether the binding was re-pointed away from this holder, or removed:
    /// the holder keeps what it was told, and [`crate::fiscal_reports`] and
    /// [`crate::assemble_fiscal_reports`] omit the report.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub withdrawn: bool,
}

/// Up to [`FISCAL_REPORT_FACTS_PER_PART`] facts of one observer's report.
///
/// An observer's facts are divided in aggregate-ID order. An aggregate keeps
/// the ID it was first given and a new one takes the next, so new aggregates
/// fill the last part and the parts before it keep their members; an
/// archive commit that takes books out renumbers them. A part with no facts
/// withdraws a part the report no longer has, and a part at or past the
/// summary's `part_count` is not part of the report.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalReportPart {
    pub observer_id: String,
    /// The part's number, counted from zero.
    pub part: u32,
    pub facts: Vec<FiscalReportFact>,
}

/// One observer's report, assembled from the records its holder currently
/// holds.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalReport {
    /// The report's facts in aggregate-ID order. For a complete report this
    /// equals what [`crate::compute_projections`] derives, at `as_of`, from
    /// the fiscal state the report was published from.
    pub projection: FiscalProjection,
    /// The numbers of the parts that are not current; see
    /// [`FiscalReportSummary::pending_parts`]. The facts of part `n` are
    /// those labelled `fact.NNNN` from `n` times
    /// [`FISCAL_REPORT_FACTS_PER_PART`] on, so a reader can tell which facts
    /// of an incomplete report are older than its `as_of`, and that facts of
    /// a pending part may be missing.
    pub pending_parts: BTreeSet<u32>,
}

impl FiscalReport {
    /// Whether every part of the report is current.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.pending_parts.is_empty()
    }

    /// Whether the fact with this label belongs to a part that is not
    /// current, so that it may be older than the report's `as_of`.
    #[must_use]
    pub fn is_stale(&self, fact_id: &str) -> bool {
        fact_id
            .strip_prefix("fact.")
            .and_then(|index| index.parse::<usize>().ok())
            .and_then(|index| u32::try_from(index / FISCAL_REPORT_FACTS_PER_PART).ok())
            .is_some_and(|part| self.pending_parts.contains(&part))
    }
}

/// Which report parts the report system publishes, kept with the derived
/// fields of the fiscal state.
///
/// The aggregate system writes the plan whenever it finds that a part of an
/// observer's report changed, and the report system publishes exactly what
/// the plan lists. Nothing reads published reports back, so the plan is what
/// makes a boundary republish only what changed.
///
/// A plan means something only in a state a boundary has written. In a state
/// at its scenario version it is ignored, and the first system that writes
/// the state clears it, so the first derivation of an authored state always
/// plans every report whole.
///
/// Parts the plan defers for an observer who can receive knowledge do not
/// wait for a change of the fiscal state. The aggregate system of any later
/// boundary that runs the fiscal systems finds them by reading this field
/// from the committed record, without decoding the state, and publishes them
/// in a plan of their own; a boundary that leaves such parts owed schedules
/// an internal `fiscal_report_drain_v1` packet so that the next boundary
/// comes at once. A boundary in which no observer is owed anything it can
/// receive does not decode the state.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalReportPlan {
    /// The procedure revision the plan was computed at; zero while no report
    /// has been planned. `publish` is published by the boundary that computed
    /// the plan, and a plan of an earlier revision has been carried out. A
    /// plan that publishes owed parts is computed by a boundary that does not
    /// advance the procedure revision, and every other write of the state
    /// either advances it or writes a new plan, so a plan is never carried
    /// out twice.
    pub revision: u64,
    /// Observer binding ID to the part numbers to publish. An observer listed
    /// here also gets its summary.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub publish: BTreeMap<String, BTreeSet<u32>>,
    /// Observer binding ID to the part numbers that changed but were not
    /// planned: they did not fit
    /// [`MAX_FISCAL_REPORT_RECORDS_PER_BOUNDARY`], or the observer's actor is
    /// dead, and the engine publishes no knowledge to a dead person. The next
    /// plan takes them first. A dead observer's stay here, outside every
    /// plan's limit, for as long as the actor is dead, and are published by
    /// the first boundary after the actor is alive again that runs the fiscal
    /// systems: the boundary that revives the actor, when it admits any
    /// ingress or event. An observer listed with no part is owed its summary
    /// only.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub deferred: BTreeMap<String, BTreeSet<u32>>,
    /// Observer binding ID to the holders the boundary that computed the plan
    /// re-pointed the binding away from, or removed it from. Each that can
    /// still receive knowledge gets one summary marked withdrawn.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub withdraw: BTreeMap<String, BTreeSet<KnowledgeHolderRef>>,
}

impl FiscalReportPlan {
    /// Whether no report has been planned yet.
    #[must_use]
    pub fn is_unused(&self) -> bool {
        *self == Self::default()
    }

    /// Marks the plan carried out, in a boundary whose only change of the
    /// state is archive maintenance. Maintenance does not advance the
    /// procedure revision, so the plan of an earlier boundary would otherwise
    /// still look current and the report system would publish it again.
    /// What the plan owes (`deferred`) stays owed.
    pub(crate) fn settle_maintenance(&mut self) {
        self.publish.clear();
        self.withdraw.clear();
    }
}

/// The fiscal state of a run: one record, `canwu.fiscal:state`.
///
/// Each scope's adoptions and the books of its hot accounting cycles are in
/// its [`FiscalScopeLedger`]; each institution's cycle bookkeeping is in its
/// [`FiscalOffice`]; closed books leave the state through the fiscal archive
/// (see [`FiscalState::prepare_fiscal_archive`]). Nothing in the state grows
/// for the life of a run: every action target keeps its recent outcomes in a
/// [`FiscalOutcomeRing`], and the archive keeps every archived book and
/// institution total for good.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalState {
    pub schema_version: u32,
    pub procedure_revision: u64,
    pub catalog_version: DomainRecordVersionRef,
    pub historical_context: FiscalHistoricalContext,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub execution_evidence_kinds: BTreeSet<DomainRecordKind>,
    pub authority_bindings: BTreeMap<String, FiscalAuthorityBinding>,
    /// Auditors from outside an institution, keyed by audit binding ID. The
    /// field is omitted from the serialized state while it is empty.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub audit_bindings: BTreeMap<String, FiscalAuditBinding>,
    /// Who appoints and removes each institution's auditors, keyed by policy
    /// ID; declared with the state. Omitted from the serialized state while
    /// it is empty.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub audit_appointment_policies: BTreeMap<String, FiscalAppointmentPolicy>,
    /// Every pending nomination and removal petition, and the latest resolved
    /// ones of each policy, keyed by proposal ID. Omitted from the serialized
    /// state while it is empty.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub audit_proposals: BTreeMap<String, FiscalAuditProposal>,
    /// One ledger per scope binding, keyed by scope binding ID.
    pub ledgers: BTreeMap<String, FiscalScopeLedger>,
    /// Institutions with hot cycles, an archived cycle, or a recent outcome,
    /// keyed by [`fiscal_office_key`]. An office is made when it first holds
    /// something and never holds nothing again.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub offices: BTreeMap<String, FiscalOffice>,
    pub observer_bindings: BTreeMap<String, FiscalObserverBinding>,
    pub transition_candidates: BTreeMap<String, FiscalTransitionCandidate>,
    /// The per-cycle totals of the hot books, one per accounting partition
    /// (institution, mechanism, scope, cycle, unit, payment form), by stable
    /// ID; see [`crate::compute_aggregates`]. They are what reports tell, and
    /// a book's totals are archived with it.
    pub aggregates: BTreeMap<String, FiscalStrategicAggregate>,
    /// The fiscal archive head: the directory of the newest applied commit,
    /// the maintenance receipts, and the retention handles not yet
    /// acknowledged. Omitted while no archive commit has settled.
    #[serde(default, skip_serializing_if = "crate::FiscalArchiveHeadV1::is_unused")]
    pub archive: crate::FiscalArchiveHeadV1,
    /// Observer binding ID to the holders that settlement re-pointed the
    /// binding away from, or removed it from, in the current boundary; the
    /// same boundary's aggregate system moves them into the report plan, so
    /// a state a boundary has finished holds none.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pending_withdrawals: BTreeMap<String, BTreeSet<KnowledgeHolderRef>>,
    /// Which report parts the report system publishes. The field is omitted
    /// from the serialized state until a report has been planned.
    #[serde(default, skip_serializing_if = "FiscalReportPlan::is_unused")]
    pub report_plan: FiscalReportPlan,
}

impl FiscalState {
    #[must_use]
    pub fn new(year: i32, mode: FiscalHistoricalMode, at: SimTime) -> Self {
        Self {
            schema_version: FISCAL_RUNTIME_SCHEMA_VERSION,
            procedure_revision: 1,
            catalog_version: initial_catalog_version(),
            historical_context: FiscalHistoricalContext {
                year,
                mode,
                updated_at: at,
            },
            execution_evidence_kinds: BTreeSet::new(),
            authority_bindings: BTreeMap::new(),
            audit_bindings: BTreeMap::new(),
            audit_appointment_policies: BTreeMap::new(),
            audit_proposals: BTreeMap::new(),
            ledgers: BTreeMap::new(),
            offices: BTreeMap::new(),
            observer_bindings: BTreeMap::new(),
            transition_candidates: BTreeMap::new(),
            aggregates: BTreeMap::new(),
            archive: crate::FiscalArchiveHeadV1::default(),
            pending_withdrawals: BTreeMap::new(),
            report_plan: FiscalReportPlan::default(),
        }
    }

    /// Adds a scope binding with an empty ledger, or replaces the binding of
    /// an existing one.
    pub fn add_scope(&mut self, binding: FiscalScopeBinding) {
        match self.ledgers.get_mut(&binding.id) {
            Some(ledger) => ledger.scope = binding,
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
        ledger
            .books
            .entry(assessment.accounting_cycle_id.clone())
            .or_insert_with(|| FiscalCycleBook::open(assessment.accounting_cycle_id.clone()))
            .assessments
            .insert(assessment.id.clone(), assessment);
        if opened {
            self.recompute_offices();
        }
        Ok(())
    }

    /// Adds a remission to the book of its assessment in `scope_binding_id`.
    pub fn add_remission(
        &mut self,
        scope_binding_id: &str,
        remission: FiscalRemission,
    ) -> Result<(), CanwuError> {
        self.book_with_mut(scope_binding_id, |book| {
            book.assessments.contains_key(&remission.assessment_id)
        })?
        .remissions
        .insert(remission.id.clone(), remission);
        Ok(())
    }

    /// Adds an execution request to the book of its assessment in
    /// `scope_binding_id`.
    pub fn add_execution_request(
        &mut self,
        scope_binding_id: &str,
        request: FiscalExecutionRequest,
    ) -> Result<(), CanwuError> {
        self.book_with_mut(scope_binding_id, |book| {
            book.assessments.contains_key(&request.assessment_id)
        })?
        .execution_requests
        .insert(request.id.clone(), request);
        Ok(())
    }

    /// Adds an execution receipt to the book of its request in
    /// `scope_binding_id`.
    pub fn add_execution_receipt(
        &mut self,
        scope_binding_id: &str,
        receipt: FiscalExecutionReceipt,
    ) -> Result<(), CanwuError> {
        self.book_with_mut(scope_binding_id, |book| {
            book.execution_requests.contains_key(&receipt.request_id)
        })?
        .execution_receipts
        .insert(receipt.id.clone(), receipt);
        Ok(())
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
        self.ledger_mut(scope_binding_id)?
            .books
            .get_mut(&cycle)
            .ok_or_else(|| invalid("fiscal audit book is unavailable"))?
            .audits
            .insert(audit.id.clone(), audit);
        Ok(())
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

    pub(crate) fn office_mut(&mut self, institution: &EntityRef) -> &mut FiscalOffice {
        self.offices
            .entry(fiscal_office_key(institution))
            .or_insert_with(|| FiscalOffice::new(institution.clone()))
    }

    /// The outcome ring of one action target. An office or observer that has
    /// no ring yet has an empty one.
    #[must_use]
    pub fn outcome_ring(&self, target: &FiscalActionTarget) -> Option<&FiscalOutcomeRing> {
        static EMPTY: FiscalOutcomeRing = FiscalOutcomeRing {
            recent: Vec::new(),
            floor: 0,
        };
        match target {
            FiscalActionTarget::Ledger(scope) => self.ledger(scope).map(|ledger| &ledger.outcomes),
            FiscalActionTarget::Office(institution) => Some(
                self.office(institution)
                    .map_or(&EMPTY, |office| &office.outcomes),
            ),
            FiscalActionTarget::Observer(observer) => self
                .observer_bindings
                .get(observer)
                .map(|binding| &binding.outcomes),
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

    pub(crate) fn outcome_ring_mut(
        &mut self,
        target: &FiscalActionTarget,
    ) -> Result<&mut FiscalOutcomeRing, CanwuError> {
        match target {
            FiscalActionTarget::Ledger(scope) => Ok(&mut self.ledger_mut(scope)?.outcomes),
            FiscalActionTarget::Office(institution) => {
                Ok(&mut self.office_mut(institution).outcomes)
            }
            FiscalActionTarget::Observer(observer) => self
                .observer_bindings
                .get_mut(observer)
                .map(|binding| &mut binding.outcomes)
                .ok_or_else(|| invalid("fiscal observer binding is unavailable")),
        }
    }

    /// Sets each office's count of hot books per cycle from the ledgers,
    /// making the offices that need one. What an office keeps of archived
    /// books (their totals and its watermark) is left as it is.
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
        self.offices.retain(|_, office| !office.is_empty());
    }

    #[allow(clippy::too_many_lines)]
    pub fn validate(&self, catalog: &CompiledFiscalCatalog) -> Result<(), CanwuError> {
        if self.schema_version != FISCAL_RUNTIME_SCHEMA_VERSION {
            return Err(invalid("unsupported fiscal runtime schema"));
        }
        if self.procedure_revision == 0 {
            return Err(invalid("fiscal procedure revision must be nonzero"));
        }
        let hot = |count: usize, cap: usize| count <= cap;
        if self.execution_evidence_kinds.len() > MAX_FISCAL_EVIDENCE_KINDS
            || self.authority_bindings.len() > MAX_FISCAL_RUNTIME_BINDINGS
            || self.audit_bindings.len() > MAX_FISCAL_RUNTIME_BINDINGS
            || self.audit_appointment_policies.len() > MAX_FISCAL_RUNTIME_BINDINGS
            || self.ledgers.len() > MAX_FISCAL_RUNTIME_BINDINGS
            || self.observer_bindings.len() > MAX_FISCAL_OBSERVERS
            || !hot(self.adoptions().count(), MAX_FISCAL_EXECUTION_REQUESTS)
            || !hot(self.assessments().count(), MAX_FISCAL_ASSESSMENTS)
            || !hot(self.remissions().count(), MAX_FISCAL_ASSESSMENTS)
            || !hot(
                self.execution_requests().count(),
                MAX_FISCAL_EXECUTION_REQUESTS,
            )
            || !hot(
                self.execution_receipts().count(),
                MAX_FISCAL_EXECUTION_RECEIPTS,
            )
            || !hot(self.audits().count(), MAX_FISCAL_ASSESSMENTS)
            || self
                .ledgers
                .values()
                .any(|ledger| ledger.books.len() > MAX_FISCAL_HOT_BOOKS_PER_LEDGER)
            || self.transition_candidates.len() > MAX_FISCAL_CATALOG_COVERAGE_CELLS
            || self.aggregates.len() > MAX_FISCAL_ASSESSMENTS
        {
            return Err(invalid(
                "fiscal runtime record exceeds its bounded capacity",
            ));
        }
        if self.catalog_version != initial_catalog_version()
            || self.historical_context.year != catalog.historical_year
                && self.historical_context.updated_at == SimTime::EPOCH
        {
            return Err(invalid(
                "fiscal state is not bound to the selected initial catalog",
            ));
        }
        if !catalog
            .historical_scope
            .contains(self.historical_context.year)
        {
            return Err(invalid(
                "fiscal historical context lies outside the catalog scope",
            ));
        }
        validate_map(
            &self.authority_bindings,
            |value| &value.id,
            "authority binding",
        )?;
        validate_map(
            &self.observer_bindings,
            |value| &value.id,
            "observer binding",
        )?;
        validate_map(
            &self.transition_candidates,
            |value| &value.id,
            "transition candidate",
        )?;
        if self.authority_bindings.values().any(|binding| {
            binding.acting_actor.is_some() != binding.authority_basis.is_some()
                || binding.acting_actor.is_some()
                    && binding.acting_actor == binding.authorized_actor
                || binding
                    .authority_basis
                    .as_ref()
                    .is_some_and(|basis| basis.version == 0)
        }) {
            return Err(invalid(
                "fiscal acting actors require one exact authority basis and differ from the authorized actor",
            ));
        }
        validate_map(&self.audit_bindings, |value| &value.id, "audit binding")?;
        if self.audit_bindings.values().any(|binding| {
            binding.authority_basis.version == 0
                || self.authority_bindings.contains_key(&binding.id)
                || self.audit_appointment_policies.contains_key(&binding.id)
        }) {
            return Err(invalid(
                "fiscal audit bindings require one exact authority basis and an ID no authority binding or appointment policy uses",
            ));
        }
        self.validate_appointments()?;
        if self
            .authority_bindings
            .values()
            .filter_map(|binding| binding.authority_basis.as_ref())
            .chain(
                self.audit_bindings
                    .values()
                    .map(|binding| &binding.authority_basis),
            )
            .chain(
                self.audit_proposals
                    .values()
                    .filter_map(|proposal| match &proposal.kind {
                        FiscalAuditProposalKind::Nomination {
                            authority_basis, ..
                        } => Some(authority_basis),
                        FiscalAuditProposalKind::RemovalPetition { .. } => None,
                    }),
            )
            .any(|basis| basis.record.kind.namespace == FiscalStateRecord::NAMESPACE)
        {
            return Err(invalid(
                "a fiscal authority basis must be an application record, not a fiscal record",
            ));
        }
        self.validate_observers()?;
        self.validate_report_plan()?;
        for (key, ledger) in &self.ledgers {
            validate_identifier(key, "scope binding")?;
            if *key != ledger.scope.id {
                return Err(invalid("scope binding map key does not match its value"));
            }
            self.validate_ledger(ledger, catalog)?;
        }
        self.validate_offices()?;
        self.archive.validate()?;
        Ok(())
    }

    /// Checks the observer bindings: bounded confidence and visibility, a
    /// holder that is the actor's person or, without an actor, a government;
    /// one line slot each, below [`MAX_FISCAL_OBSERVERS`]; and their rings.
    fn validate_observers(&self) -> Result<(), CanwuError> {
        let mut slots = BTreeSet::new();
        for observer in self.observer_bindings.values() {
            let holder_matches = match (observer.actor, &observer.knowledge_holder) {
                (Some(actor), KnowledgeHolderRef::Person(person)) => actor == *person,
                (None, KnowledgeHolderRef::Entity(EntityRef::Government(_))) => true,
                _ => false,
            };
            if observer.confidence_per_mille > 1_000
                || !holder_matches
                || observer.visible_institutions.len() > MAX_FISCAL_RUNTIME_BINDINGS
                || usize::from(observer.line_slot) >= MAX_FISCAL_OBSERVERS
                || !slots.insert(observer.line_slot)
            {
                return Err(invalid(
                    "fiscal observers require bounded confidence, their actor's person or a government as holder, and a line slot of their own",
                ));
            }
            observer.outcomes.validate(self.procedure_revision)?;
        }
        if self
            .pending_withdrawals
            .keys()
            .any(|id| !self.observer_bindings.contains_key(id))
        {
            return Err(invalid(
                "a fiscal report withdrawal names an unknown observer binding",
            ));
        }
        Ok(())
    }

    fn validate_report_plan(&self) -> Result<(), CanwuError> {
        let plan = &self.report_plan;
        let active = |id: &String| {
            self.observer_bindings
                .get(id)
                .is_some_and(FiscalObserverBinding::is_active)
        };
        if plan.revision > self.procedure_revision
            || !plan.publish.keys().chain(plan.deferred.keys()).all(active)
            || plan
                .withdraw
                .keys()
                .any(|id| !self.observer_bindings.contains_key(id))
            || plan
                .publish
                .values()
                .chain(plan.deferred.values())
                .flatten()
                .any(|part| {
                    usize::try_from(*part).map_or(true, |part| part >= MAX_FISCAL_REPORT_PARTS)
                })
            || plan.publish.len()
                + plan.publish.values().map(BTreeSet::len).sum::<usize>()
                + plan.withdraw.values().map(BTreeSet::len).sum::<usize>()
                > MAX_FISCAL_REPORT_RECORDS_PER_BOUNDARY
        {
            return Err(invalid(
                "the fiscal report plan names an unknown observer or part, a later revision, or more records than one boundary publishes",
            ));
        }
        Ok(())
    }

    /// Checks one scope ledger the way [`Self::validate`] does: what a
    /// change confined to that ledger's books can break.
    pub(crate) fn validate_scope(
        &self,
        scope_binding_id: &str,
        catalog: &CompiledFiscalCatalog,
    ) -> Result<(), CanwuError> {
        let ledger = self
            .ledger(scope_binding_id)
            .ok_or_else(|| invalid(format!("fiscal scope {scope_binding_id} is unavailable")))?;
        self.validate_ledger(ledger, catalog)
    }

    /// Checks one scope ledger: its scope, adoptions, books, and ring, and
    /// every per-assessment invariant of its books.
    #[allow(clippy::too_many_lines)]
    fn validate_ledger(
        &self,
        ledger: &FiscalScopeLedger,
        catalog: &CompiledFiscalCatalog,
    ) -> Result<(), CanwuError> {
        let scope = &ledger.scope;
        if !catalog.regions.contains_key(&scope.jurisdiction_id) {
            return Err(invalid(format!(
                "scope {} references unavailable jurisdiction {}",
                scope.id, scope.jurisdiction_id
            )));
        }
        // A report fact carries the scope's institution, so its domain
        // reference is bounded like the other texts of a fact.
        if matches!(
            &scope.institution,
            EntityRef::Domain(reference)
                if reference.kind.namespace.len()
                    + reference.kind.name.len()
                    + reference.id.len()
                    > MAX_FISCAL_IDENTIFIER_BYTES
        ) {
            return Err(invalid(
                "a fiscal institution reference is longer than a report fact may carry",
            ));
        }
        if let Some(watermark) = &ledger.watermark {
            validate_identifier(watermark, "fiscal accounting cycle")?;
        }
        ledger.outcomes.validate(self.procedure_revision)?;
        validate_map(&ledger.adoptions, |value| &value.id, "adoption")?;
        for adoption in ledger.adoptions.values() {
            let rule = catalog.rules.get(&adoption.rule_id).ok_or_else(|| {
                invalid(format!("adoption {} references unknown rule", adoption.id))
            })?;
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
                    || assessment.unit.is_empty()
                    || assessment.unit.len() > MAX_FISCAL_IDENTIFIER_BYTES
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
                if remission.quantity == 0
                    || !book.assessments.contains_key(&remission.assessment_id)
                {
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
                    || request.source != request.institution
                        && request.target != request.institution
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

    /// Checks the offices against the ledgers: each keyed by its
    /// institution, holding something, and counting for each cycle exactly
    /// the scopes of its institution that hold a hot book of it.
    fn validate_offices(&self) -> Result<(), CanwuError> {
        let mut hot = BTreeMap::<String, BTreeMap<&str, u32>>::new();
        for ledger in self.ledgers.values() {
            let cycles = hot
                .entry(fiscal_office_key(&ledger.scope.institution))
                .or_default();
            for cycle in ledger.books.keys() {
                *cycles.entry(cycle.as_str()).or_default() += 1;
            }
        }
        for (key, office) in &self.offices {
            if *key != fiscal_office_key(&office.institution) || office.is_empty() {
                return Err(invalid(
                    "a fiscal office is empty or not keyed by its institution",
                ));
            }
            if let Some(watermark) = &office.watermark {
                validate_identifier(watermark, "fiscal accounting cycle")?;
            }
            office.outcomes.validate(self.procedure_revision)?;
            for entry in office.cycles.values() {
                if entry.hot_books == 0
                    || !entry
                        .archived_totals
                        .windows(2)
                        .all(|pair| pair[0].key() < pair[1].key())
                {
                    return Err(invalid(
                        "a fiscal office cycle has no hot book or unordered archived totals",
                    ));
                }
            }
            let counted: BTreeMap<&str, u32> = office
                .cycles
                .iter()
                .map(|(cycle, entry)| (cycle.as_str(), entry.hot_books))
                .collect();
            if hot.remove(key).unwrap_or_default() != counted {
                return Err(invalid(
                    "a fiscal office does not count the hot books of its scopes",
                ));
            }
        }
        if hot.values().any(|cycles| !cycles.is_empty()) {
            return Err(invalid(
                "an institution holds hot books but no fiscal office counts them",
            ));
        }
        Ok(())
    }

    /// Checks the appointment policies and proposals: one policy per audited
    /// institution, bounded party lists, and IDs apart from every authority
    /// binding; proposals on governed institutions, by a listed party, with
    /// bounded grounds; at most [`MAX_FISCAL_PENDING_PROPOSALS_PER_PARTY`]
    /// pending for one party of one policy, each of a different kind or
    /// binding, and at most [`MAX_FISCAL_RESOLVED_PROPOSALS_PER_POLICY`]
    /// resolved for one policy; and every pending petition still apt: its
    /// binding still has the auditor it names, since any change of that
    /// auditor supersedes the petition.
    fn validate_appointments(&self) -> Result<(), CanwuError> {
        validate_map(
            &self.audit_appointment_policies,
            |value| &value.id,
            "appointment policy",
        )?;
        let mut governed = BTreeSet::new();
        for policy in self.audit_appointment_policies.values() {
            if !governed.insert(&policy.institution)
                || policy.nominators.len() > MAX_FISCAL_APPOINTMENT_PARTIES
                || policy.petitioners.len() > MAX_FISCAL_APPOINTMENT_PARTIES
                || self.authority_bindings.contains_key(&policy.id)
            {
                return Err(invalid(
                    "fiscal appointment policies govern one institution each, list a bounded number of parties, and take an ID no authority binding uses",
                ));
            }
        }
        validate_map(&self.audit_proposals, |value| &value.id, "audit proposal")?;
        let policies: BTreeMap<&EntityRef, &FiscalAppointmentPolicy> = self
            .audit_appointment_policies
            .values()
            .map(|policy| (&policy.institution, policy))
            .collect();
        // Pending proposals by policy, party, kind, and binding, and resolved
        // proposals by policy.
        let mut pending = BTreeMap::<_, BTreeSet<_>>::new();
        let mut resolved = BTreeMap::<&EntityRef, usize>::new();
        for proposal in self.audit_proposals.values() {
            let Some(policy) = policies.get(&proposal.institution) else {
                return Err(invalid(format!(
                    "audit proposal {} concerns an institution no appointment policy governs",
                    proposal.id
                )));
            };
            let binding = self.audit_bindings.get(&proposal.audit_binding_id);
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
                    binding.is_some_and(|binding| {
                        binding.institution == proposal.institution && binding.auditor == *auditor
                    }),
                ),
            };
            if !listed
                || !bounded
                || validate_identifier(&proposal.audit_binding_id, "audit binding").is_err()
                || self
                    .authority_bindings
                    .contains_key(&proposal.audit_binding_id)
                || self
                    .audit_appointment_policies
                    .contains_key(&proposal.audit_binding_id)
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
                    .entry((&proposal.institution, &proposal.party))
                    .or_default()
                    .insert((nominates, &proposal.audit_binding_id))
                {
                    return Err(invalid(format!(
                        "audit proposal {} duplicates a pending proposal of its party",
                        proposal.id
                    )));
                }
            } else {
                *resolved.entry(&proposal.institution).or_default() += 1;
            }
        }
        if pending
            .values()
            .any(|proposals| proposals.len() > MAX_FISCAL_PENDING_PROPOSALS_PER_PARTY)
            || resolved
                .values()
                .any(|count| *count > MAX_FISCAL_RESOLVED_PROPOSALS_PER_POLICY)
        {
            return Err(invalid(
                "fiscal audit proposals exceed the pending room of a party or the resolved room of a policy",
            ));
        }
        Ok(())
    }

    pub fn into_record(self, catalog: &CompiledFiscalCatalog) -> Result<DomainRecord, CanwuError> {
        self.validate(catalog)?;
        let draft = self.record_draft()?;
        Ok(DomainRecord {
            reference: draft.reference,
            owner: PLUGIN_NAME.to_owned(),
            class: DomainRecordClass::Record,
            version: 1,
            lifecycle: DomainRecordLifecycle::Active,
            payload: draft.payload,
            references: draft.references,
        })
    }

    pub(crate) fn record_draft(&self) -> Result<DomainRecordDraft, CanwuError> {
        let (draft, encoded_len) = self.encode_record()?;
        if encoded_len > MAX_FISCAL_STATE_JSON_BYTES {
            return Err(invalid("fiscal state exceeds its serialized byte budget"));
        }
        Ok(draft)
    }

    /// Encodes the state as its record draft and returns the serialized size
    /// of the payload, without holding it to the byte budget.
    ///
    /// The payload declares, as required evidence, the versions the hot books
    /// cite that a boundary established: their commutation quotes and their
    /// receipts' evidence. An archived book's citations stop being declared;
    /// [`FiscalState::prepare_fiscal_archive`] refuses a book whose citations
    /// a seal has not covered yet.
    pub(crate) fn encode_record(&self) -> Result<(DomainRecordDraft, usize), CanwuError> {
        let mut draft = DomainRecordDraft::from_typed(fiscal_state_reference(), self)?;
        let dependencies = self
            .books()
            .flat_map(|(_, book)| book.citations())
            .filter(|reference| {
                matches!(
                    reference.established_by,
                    DomainRecordVersionSource::BoundaryChange { .. }
                )
            })
            .cloned()
            .map(EvidenceRef::DomainRecordVersion)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let continuation = if dependencies.is_empty() {
            PayloadRequiredEvidenceContinuationV1::completed()
        } else {
            PayloadRequiredEvidenceContinuationV1::active(dependencies)
        };
        draft
            .payload
            .as_object_mut()
            .ok_or_else(|| invalid("fiscal state payload is not an object"))?
            .insert(
                PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD.to_owned(),
                serde_json::to_value(continuation).map_err(|error| {
                    invalid(format!(
                        "fiscal evidence continuation could not be encoded: {error}"
                    ))
                })?,
            );
        let encoded_len = serde_json::to_vec(&draft.payload)
            .map_err(|error| invalid(format!("fiscal state could not be sized: {error}")))?
            .len();
        draft.references = self.record_references();
        Ok((draft, encoded_len))
    }

    /// What the state record references: its catalog, and the person or
    /// government that holds each observer binding's report. The engine
    /// checks a record's references against its entity registry when a run is
    /// created or restored and at every write, so a state that names a holder
    /// who does not exist is refused where it enters a run, instead of
    /// failing the first boundary that publishes to it.
    fn record_references(&self) -> Vec<DomainReference> {
        let mut references: Vec<_> = self
            .observer_bindings
            .values()
            .filter_map(|observer| match &observer.knowledge_holder {
                KnowledgeHolderRef::Person(person) => Some(EntityRef::Person(*person)),
                KnowledgeHolderRef::Entity(entity @ EntityRef::Government(_)) => {
                    Some(entity.clone())
                }
                KnowledgeHolderRef::Entity(_) => None,
            })
            .map(|holder| DomainReference {
                role: FISCAL_STATE_OBSERVER_ROLE.to_owned(),
                target: DomainReferenceTarget::Core(holder),
            })
            .collect();
        references.push(DomainReference::from_typed(
            FISCAL_STATE_CATALOG_ROLE,
            fiscal_catalog_reference(),
        ));
        references.sort();
        references.dedup();
        references
    }

    pub fn validate_record_binding(&self, record: &DomainRecord) -> Result<(), CanwuError> {
        let expected_payload = self.record_draft()?.payload;
        if record.references != self.record_references() || record.payload != expected_payload {
            return Err(invalid(
                "fiscal state record does not match its decoded payload, its catalog root, and its observers",
            ));
        }
        Ok(())
    }
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
