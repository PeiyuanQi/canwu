use canwu_api::{
    CanwuError, DecisionTicketDraft, DomainRecordRef, DomainRecordType, DomainRecordVersionRef,
    DomainValueKindClass, EntityRef, EvidenceRef, KnowledgeHolderRef, SimTime,
    TypedDomainRecordRef, canonical_hash,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const LAW_SCHEMA_VERSION: u32 = 2;
pub const LAW_PLAN_VERSION: u32 = 2;
pub const LAW_PLAN_HASH_DOMAIN: &str = "canwu.law.compiled-plan.v1";
/// Absolute activation ceiling. Authored plans may choose smaller limits but
/// cannot enlarge the pre-decode trust boundary.
pub const MAX_LEGAL_STATE_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_LEGAL_MEMORY_BYTES: usize = 128 * 1024 * 1024;

macro_rules! marker {
    ($name:ident, $kind:literal, $payload:ty) => {
        #[derive(Clone, Copy, Debug)]
        pub struct $name;
        impl DomainRecordType for $name {
            type Payload = $payload;
            type Class = DomainValueKindClass;
            const NAMESPACE: &'static str = crate::PLUGIN_NAMESPACE;
            const NAME: &'static str = $kind;
        }
    };
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LawBudgets {
    pub max_orders: usize,
    pub max_jurisdictions: usize,
    pub max_institutions: usize,
    pub max_procedures: usize,
    pub max_stages_per_procedure: usize,
    pub max_seats_per_procedure: usize,
    pub max_sources: usize,
    pub max_rules: usize,
    pub max_law_versions: usize,
    pub max_cases: usize,
    pub max_findings: usize,
    pub max_rulings: usize,
    pub max_participations: usize,
    pub max_intent_outcomes: usize,
    pub max_conflicts: usize,
    pub max_successions: usize,
    pub max_retirements: usize,
    #[serde(default = "default_retirement_dependency_records")]
    pub max_retirement_dependency_records: usize,
    pub max_outbox: usize,
    pub max_pending_intents: usize,
    pub max_evidence_per_record: usize,
    pub max_clauses_per_proposal: usize,
    pub max_jurisdictions_per_proposal: usize,
    /// Total collection entries nested inside any one admitted legal record.
    pub max_nested_items_per_record: usize,
    pub max_applicability_entries_per_boundary: usize,
    /// Total graph edges, rule/version visits, and conflict references per query.
    pub max_applicability_query_work: usize,
    pub max_mutations_per_boundary: usize,
    pub max_text_bytes: usize,
    pub max_state_bytes: usize,
    pub max_memory_bytes: usize,
}

impl LawBudgets {
    #[must_use]
    pub const fn conservative() -> Self {
        Self {
            max_orders: 256,
            max_jurisdictions: 512,
            max_institutions: 512,
            max_procedures: 2_048,
            max_stages_per_procedure: 16,
            max_seats_per_procedure: 256,
            max_sources: 10_000,
            max_rules: 20_000,
            max_law_versions: 100_000,
            max_cases: 10_000,
            max_findings: 50_000,
            max_rulings: 20_000,
            max_participations: 100_000,
            max_intent_outcomes: 100_000,
            max_conflicts: 20_000,
            max_successions: 2_048,
            max_retirements: 100_000,
            max_retirement_dependency_records: 4_096,
            max_outbox: 20_000,
            max_pending_intents: 2_048,
            max_evidence_per_record: 256,
            max_clauses_per_proposal: 64,
            max_jurisdictions_per_proposal: 32,
            max_nested_items_per_record: 4_096,
            max_applicability_entries_per_boundary: 2_048,
            max_applicability_query_work: 250_000,
            max_mutations_per_boundary: 512,
            max_text_bytes: 8 * 1024,
            max_state_bytes: MAX_LEGAL_STATE_BYTES,
            max_memory_bytes: MAX_LEGAL_MEMORY_BYTES,
        }
    }
}

const fn default_retirement_dependency_records() -> usize {
    4_096
}

impl Default for LawBudgets {
    fn default() -> Self {
        Self::conservative()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct IdBlock {
    pub start: u64,
    pub capacity: u64,
}

impl IdBlock {
    pub(crate) fn validate(&self, label: &str) -> Result<(), CanwuError> {
        if self.capacity == 0 || self.start.checked_add(self.capacity).is_none() {
            return Err(invalid(format!("{label} ID block is empty or overflows")));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LawIdBlocks {
    pub decision_tickets: IdBlock,
    pub decision_requests: IdBlock,
    pub command_requests: IdBlock,
}

impl Default for LawIdBlocks {
    fn default() -> Self {
        Self {
            decision_tickets: IdBlock {
                start: 1,
                capacity: 20_000,
            },
            decision_requests: IdBlock {
                start: 100_000,
                capacity: 60_000,
            },
            command_requests: IdBlock {
                start: 200_000,
                capacity: 100_000,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JurisdictionRelationKind {
    Delegation,
    TerritorialContainment,
    Supremacy,
    Appeal,
    TreatyMembership,
    Overlap,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationTraversalDirection {
    Forward,
    Reverse,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct JurisdictionTraversalRule {
    pub kind: JurisdictionRelationKind,
    pub direction: RelationTraversalDirection,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct JurisdictionRelationDefinition {
    pub from: String,
    pub to: String,
    pub kind: JurisdictionRelationKind,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LegalOrderDefinition {
    pub id: String,
    pub precedence_profile: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LegalJurisdictionDefinition {
    pub id: String,
    pub relations: Vec<JurisdictionRelationDefinition>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AuthoritySeatDefinition {
    pub id: String,
    pub holder: Option<KnowledgeHolderRef>,
    pub permission_profile: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LegalCompetenceDefinition {
    pub legal_orders: Vec<String>,
    pub jurisdictions: Vec<String>,
    pub subject_matters: Vec<String>,
    pub source_modes: Vec<SourceMode>,
    pub operations: Vec<LawOperation>,
    pub procedures: Vec<String>,
    pub forums: Vec<String>,
    pub can_adjudicate: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LegalInstitutionDefinition {
    pub id: String,
    pub organization: Option<EntityRef>,
    pub jurisdictions: Vec<String>,
    pub seats: Vec<AuthoritySeatDefinition>,
    pub procedures: Vec<String>,
    pub competences: Vec<LegalCompetenceDefinition>,
}

/// Role of one stage in a procedure.
///
/// Every kind except [`Self::Consultation`] is a deciding stage: its ballots
/// are tallied by [`ProcedureStageDefinition`]'s quorum, threshold, and block
/// rules, and the stage completes as soon as that tally passes. A `Veto` stage
/// additionally treats any `Against` ballot as a veto.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcedureStageKind {
    Deliberation,
    Veto,
    Signature,
    Review,
    Ratification,
    /// Advisory stage. Seats receive tickets whose context carries
    /// `"advisory": true`, and their ballots persist as immutable participation
    /// evidence, but ballots never count toward completion, veto, or adoption.
    /// The stage completes at the first legal boundary after its deadline (the
    /// procedure deadline wake) and opens the next stage; zero ballots is a
    /// valid outcome. Seat work of the stage that is still pending or enqueued
    /// at completion expires, as whenever a stage stops accepting ballots. A
    /// procedure that lacks its capacity reservation cannot complete a
    /// consultation: it expires at that deadline like any other stage. A
    /// consultation needs a positive `deadline_minutes`, no quorum, threshold,
    /// weights, or blocks, and cannot be a procedure's final stage.
    Consultation,
}

/// Tie-break token for a procedure whose stages count unit blocks: a tie adds
/// no block, so a tied stage passes only if `block_threshold` is already met;
/// otherwise it waits for more ballots or its deadline.
pub const PROCEDURE_TIE_BREAK_STATUS_QUO: &str = "status-quo";
/// Prefix of the casting-seat tie-break token, followed by a seat ID (for
/// example `casting-seat:chair`). The seat must belong to every blocked stage
/// of the procedure, and each such stage's `block_threshold` must exceed half
/// its block count, so no tie passes on its own and the casting seat decides.
pub const PROCEDURE_TIE_BREAK_CASTING_SEAT_PREFIX: &str = "casting-seat:";

/// Parsed [`ProcedureProfileDefinition::deterministic_tie_break`] for block counting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BlockTieBreak<'a> {
    StatusQuo,
    CastingSeat(&'a str),
}

impl<'a> BlockTieBreak<'a> {
    pub(crate) fn parse(value: &'a str) -> Option<Self> {
        if value == PROCEDURE_TIE_BREAK_STATUS_QUO {
            return Some(Self::StatusQuo);
        }
        value
            .strip_prefix(PROCEDURE_TIE_BREAK_CASTING_SEAT_PREFIX)
            .filter(|seat| canonical_id(seat))
            .map(Self::CastingSeat)
    }
}

/// One frozen stage of a procedure profile.
///
/// Ballots are tallied in integer *vote weight*. A seat's weight is its entry in
/// [`Self::seat_weights`], or 1 when the seat is absent (so an empty map is the
/// equal-seat count). A deciding stage passes when all of the following hold:
///
/// - no seat cast `Veto`, and in a `Veto` stage no seat cast `Against`
///   (vetoes are seat powers and are never weighted);
/// - `quorum`: the summed weight of seats that cast any ballot, abstentions
///   included, is at least `quorum`;
/// - `threshold`: the summed weight of `For` and `Against` ballots is positive
///   and `For` weight × 1,000 is at least that sum × `threshold` (per mille);
/// - when [`Self::block_of_seat`] is non-empty, at least `block_threshold`
///   blocks take the `For` position.
///
/// A block's position is the weighted majority of its seats' `For` and
/// `Against` ballots. A block whose `For` and `Against` weights are equal is
/// divided and takes no position, like a block whose seats cast no ballot or
/// only abstained; neither counts for either side. When as many blocks are
/// `For` as `Against` (at least one each), the stage is tied and the
/// procedure's `deterministic_tie_break` applies. [`PROCEDURE_TIE_BREAK_STATUS_QUO`]
/// adds no block: the tied stage passes only if `block_threshold` is already
/// met, and otherwise waits for more ballots or its deadline.
/// [`PROCEDURE_TIE_BREAK_CASTING_SEAT_PREFIX`] names a casting seat whose own
/// `For` ballot in this stage adds one `For` block before `block_threshold` is
/// checked; a casting seat that voted `Against`, abstained, or has not voted
/// adds nothing, so the tie does not pass.
///
/// Stages are evaluated at every boundary that touches the procedure, over the
/// ballots admitted so far, exactly as for equal seats, so later ballots can
/// break or create a tie before the deadline. Seat work of a stage that stops
/// accepting ballots (it passed, completed, or its procedure closed) expires,
/// and a seat response that arrives afterwards is recorded as a rejected
/// intent outcome. Weight, block, and block-threshold fields are omitted from
/// JSON when unused, so existing definitions keep their canonical encoding and
/// plan hash.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProcedureStageDefinition {
    pub id: String,
    pub kind: ProcedureStageKind,
    pub seats: Vec<String>,
    pub allowed_ballots: Vec<Ballot>,
    /// Minimum summed weight of seats that cast any ballot. Without weights
    /// other than 1 or blocks it is a seat count capped at 1,000; with them it
    /// may not exceed the stage's total seat weight.
    pub quorum: u16,
    /// Per-mille share of counted (`For` plus `Against`) weight that must be
    /// `For`, at most 1,000.
    pub threshold: u16,
    /// Minutes between the boundary that opens this stage and its deadline.
    /// The first stage's deadline is the proposal deadline.
    pub deadline_minutes: i64,
    pub allow_replacement: bool,
    /// Integer vote weight per seat of this stage; every weight is at least 1
    /// and every key is a seat of this stage. Seats not listed weigh 1, and an
    /// explicit weight of 1 is removed when the plan compiles so equal seats
    /// have one canonical encoding.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub seat_weights: BTreeMap<String, u16>,
    /// Unit block of each seat. When non-empty, every seat of this stage
    /// belongs to exactly one block and `block_threshold` is required.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub block_of_seat: BTreeMap<String, String>,
    /// Number of blocks that must take the `For` position, from 1 to the
    /// number of distinct blocks. Present exactly when blocks are used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_threshold: Option<u16>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProcedureProfileDefinition {
    pub id: String,
    pub stages: Vec<ProcedureStageDefinition>,
    /// Non-empty deterministic tie-break rule. It is interpreted only by
    /// stages that count unit blocks; a procedure with such a stage must use
    /// [`PROCEDURE_TIE_BREAK_STATUS_QUO`] or
    /// `casting-seat:<seat>` ([`PROCEDURE_TIE_BREAK_CASTING_SEAT_PREFIX`]).
    pub deterministic_tie_break: String,
    pub reservation_pool: Option<String>,
    pub reservation_quantity: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceMode {
    Promulgated,
    Adjudicated,
    Accreted,
    Agreed,
    Received,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceOriginPolicy {
    NoOrigin,
    Ruling,
    Agreement,
    Reception,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicityPolicy {
    NotRequired,
    ValidityCondition,
    EffectivenessCondition,
    EvidenceOnly,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceAuthorityPolicy {
    ProceduralInstitution,
    EvidenceClaim,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ClauseDefinition {
    pub id: String,
    pub schema: String,
    pub modality: NormativeModality,
    pub operation_kinds: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ApplicabilityProfileDefinition {
    pub id: String,
    pub legal_order: String,
    pub temporal_conflict_rule: String,
    pub pipeline: Vec<String>,
    pub jurisdiction_traversal: Vec<JurisdictionTraversalRule>,
    pub max_candidates: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LegalPredicateDefinition {
    pub id: String,
    /// Actor-relative facts must be read from this exact knowledge schema.
    pub knowledge_schema: Option<canwu_api::KnowledgeSchemaId>,
    /// JSON pointer to the boolean fact in the holder-relative record payload.
    pub payload_pointer: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LegalForumProfileDefinition {
    pub id: String,
    pub jurisdiction: String,
    pub legal_orders: Vec<String>,
    pub subject_matters: Vec<String>,
    pub institutions: Vec<String>,
    pub proof_profiles: Vec<String>,
    pub standing_profiles: Vec<String>,
    pub remedy_profiles: Vec<String>,
    pub precedent_profiles: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictResolutionBasis {
    Competence,
    Supremacy,
    Specificity,
    Temporal,
    Ruling,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PrecedenceProfileDefinition {
    pub id: String,
    pub ordered_bases: Vec<ConflictResolutionBasis>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LegalSourceProfileDefinition {
    pub id: String,
    pub mode: SourceMode,
    pub procedure: Option<String>,
    pub applicability_profile: String,
    pub origin_policy: SourceOriginPolicy,
    pub authority_policy: SourceAuthorityPolicy,
    pub publicity_policy: PublicityPolicy,
    /// Required compiled host ingress kind proving that publicity occurred.
    pub publicity_signal_kind: Option<String>,
    pub required_signal_kinds: Vec<String>,
    pub min_evidence: usize,
    pub max_evidence: usize,
    pub require_claimant: bool,
    pub allow_retroactive: bool,
    /// Required host record kind for an agreed instrument.
    pub agreement_namespace: Option<String>,
    pub agreement_kind: Option<String>,
    pub min_agreement_parties: usize,
    pub require_agreement_ratification: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LegalSignalProviderDefinition {
    pub signal_kind: String,
    pub plugin: String,
    pub packet_type: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LegalDefinition {
    pub id: String,
    pub orders: Vec<LegalOrderDefinition>,
    pub jurisdictions: Vec<LegalJurisdictionDefinition>,
    pub institutions: Vec<LegalInstitutionDefinition>,
    pub procedures: Vec<ProcedureProfileDefinition>,
    pub clauses: Vec<ClauseDefinition>,
    pub source_profiles: Vec<LegalSourceProfileDefinition>,
    #[serde(default)]
    pub signal_providers: Vec<LegalSignalProviderDefinition>,
    pub applicability_profiles: Vec<ApplicabilityProfileDefinition>,
    pub predicates: Vec<LegalPredicateDefinition>,
    pub forums: Vec<LegalForumProfileDefinition>,
    pub precedence_profiles: Vec<PrecedenceProfileDefinition>,
    pub id_blocks: LawIdBlocks,
    pub budgets: LawBudgets,
}

impl LegalDefinition {
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            orders: Vec::new(),
            jurisdictions: Vec::new(),
            institutions: Vec::new(),
            procedures: Vec::new(),
            clauses: Vec::new(),
            source_profiles: Vec::new(),
            signal_providers: Vec::new(),
            applicability_profiles: Vec::new(),
            predicates: Vec::new(),
            forums: Vec::new(),
            precedence_profiles: Vec::new(),
            id_blocks: LawIdBlocks::default(),
            budgets: LawBudgets::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct DenseKey(u32);
impl DenseKey {
    pub(crate) const fn from_raw(value: u32) -> Self {
        Self(value)
    }
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompiledLegalOrder {
    pub key: DenseKey,
    pub source_id: String,
    pub precedence_profile: String,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompiledJurisdiction {
    pub key: DenseKey,
    pub source_id: String,
    pub relations: Vec<JurisdictionRelationDefinition>,
    pub metadata: BTreeMap<String, String>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompiledProcedure {
    pub key: DenseKey,
    pub source_id: String,
    pub stages: Vec<ProcedureStageDefinition>,
    pub deterministic_tie_break: String,
    pub reservation_pool: Option<String>,
    pub reservation_quantity: u64,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompiledSourceProfile {
    pub key: DenseKey,
    pub source_id: String,
    pub mode: SourceMode,
    pub procedure: Option<String>,
    pub applicability_profile: String,
    pub origin_policy: SourceOriginPolicy,
    pub authority_policy: SourceAuthorityPolicy,
    pub publicity_policy: PublicityPolicy,
    pub publicity_signal_kind: Option<String>,
    pub required_signal_kinds: Vec<String>,
    pub min_evidence: usize,
    pub max_evidence: usize,
    pub require_claimant: bool,
    pub allow_retroactive: bool,
    pub agreement_namespace: Option<String>,
    pub agreement_kind: Option<String>,
    pub min_agreement_parties: usize,
    pub require_agreement_ratification: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompiledProcedureSeatAuthority {
    pub institution: String,
    pub seat: String,
    pub holder: Option<KnowledgeHolderRef>,
    pub permission_profile: String,
    pub decision_controller_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompiledLawPlan {
    pub plan_version: u32,
    pub definition_id: String,
    pub content_hash: String,
    pub budgets: LawBudgets,
    pub id_blocks: LawIdBlocks,
    pub orders: Vec<CompiledLegalOrder>,
    pub jurisdictions: Vec<CompiledJurisdiction>,
    pub institutions: Vec<LegalInstitutionDefinition>,
    pub institution_by_id: BTreeMap<String, DenseKey>,
    pub procedures: Vec<CompiledProcedure>,
    pub clauses: Vec<ClauseDefinition>,
    pub source_profiles: Vec<CompiledSourceProfile>,
    pub signal_provider_by_kind: BTreeMap<String, LegalSignalProviderDefinition>,
    pub applicability_profiles: Vec<ApplicabilityProfileDefinition>,
    pub predicates: Vec<LegalPredicateDefinition>,
    pub forums: Vec<LegalForumProfileDefinition>,
    pub precedence_profiles: Vec<PrecedenceProfileDefinition>,
    pub order_by_id: BTreeMap<String, DenseKey>,
    pub jurisdiction_by_id: BTreeMap<String, DenseKey>,
    pub procedure_by_id: BTreeMap<String, DenseKey>,
    pub source_profile_by_id: BTreeMap<String, DenseKey>,
    pub predicate_by_id: BTreeMap<String, DenseKey>,
    pub forum_by_id: BTreeMap<String, DenseKey>,
    pub precedence_profile_by_id: BTreeMap<String, DenseKey>,
    pub jurisdiction_adjacency_by_profile: BTreeMap<String, BTreeMap<String, Vec<String>>>,
    pub seat_authority_by_procedure:
        BTreeMap<String, BTreeMap<String, CompiledProcedureSeatAuthority>>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    Draft,
    Submitted,
    Deliberating,
    Adopted,
    Rejected,
    Expired,
    Withdrawn,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LegalCompetenceDisposition {
    Confirmed,
    Purported,
    Contested,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LegalOriginRef {
    Ruling {
        ruling: LegalRecordRef,
    },
    Agreement {
        instrument: DomainRecordVersionRef,
        parties: Vec<EntityRef>,
        ratifications: Vec<EvidenceRef>,
    },
    Reception {
        succession: String,
        predecessor: LegalRecordRef,
        transform: Option<String>,
    },
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ClauseOperation {
    pub clause: String,
    pub operation: String,
    pub content_hash: String,
    pub value: serde_json::Value,
    #[serde(default)]
    pub holders: Vec<String>,
    #[serde(default)]
    pub duty_bearers: Vec<String>,
    #[serde(default)]
    pub subject_matters: Vec<String>,
    #[serde(default)]
    pub territories: Vec<canwu_api::TerritoryId>,
    #[serde(default)]
    pub conditions: Vec<String>,
    #[serde(default)]
    pub exceptions: Vec<String>,
    #[serde(default)]
    pub standing: Vec<String>,
    #[serde(default)]
    pub forum: Option<String>,
    #[serde(default)]
    pub remedy_profile: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct CulturalTargetGenerationRef {
    pub target: String,
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CulturalDependencyKind {
    /// The culture state was evidence for adoption but is not required afterward.
    AdoptionEvidence,
    /// The operative legal effect continues to depend on a live culture level.
    LiveLevel,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct LegalCulturalDependency {
    pub target: CulturalTargetGenerationRef,
    pub kind: CulturalDependencyKind,
    pub evidence: EvidenceRef,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LegalProposal {
    pub id: String,
    pub sponsor: Option<EntityRef>,
    pub legal_order: String,
    pub jurisdictions: Vec<String>,
    pub subjects: Vec<DomainRecordRef>,
    #[serde(default)]
    pub cultural_dependencies: Vec<LegalCulturalDependency>,
    pub clauses: Vec<ClauseOperation>,
    pub source_profile: String,
    pub procedure_profile: String,
    pub procedure_profile_hash: String,
    pub deadline: SimTime,
    pub effective_at: SimTime,
    pub operation: LawOperation,
    pub rule_id: String,
    pub competence: LegalCompetenceDisposition,
    pub defects: Vec<String>,
    pub validity: OperativeDisposition,
    pub origin: Option<LegalOriginRef>,
    pub publicity: Option<LegalRecordRef>,
    pub retrospective_from: Option<SimTime>,
    pub status: ProposalStatus,
    /// Exact adoption time once the proposal has completed its authority path.
    #[serde(default)]
    pub adopted_at: Option<SimTime>,
    /// Immutable source and law-version records materialized by adoption.
    #[serde(default)]
    pub source_version: Option<LegalRecordRef>,
    #[serde(default)]
    pub law_version: Option<LegalRecordRef>,
    #[serde(default)]
    pub admitted_signal_kinds: BTreeSet<String>,
    pub evidence: Vec<EvidenceRef>,
    /// Compare-and-set guard for operations that change an existing rule.
    #[serde(default)]
    pub expected_rule_head: Option<LegalRecordRef>,
    /// Exact host-owned records read while authoring this proposal.
    pub expected_versions: Vec<DomainRecordVersionRef>,
    pub active_procedure: Option<String>,
}

/// Exact identity of a record owned inside the aggregate legal ledger.
///
/// This deliberately is not a Canwu `DomainRecordVersionRef`: aggregate-local
/// records have not been committed as independent kernel domain records.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct LegalRecordRef {
    pub kind: String,
    pub id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProcedureInstance {
    pub id: String,
    pub proposal: LegalRecordRef,
    pub profile: String,
    pub profile_hash: String,
    pub stages: Vec<ProcedureStageDefinition>,
    pub active_stage: usize,
    pub round: u32,
    pub eligible_seats: Vec<String>,
    pub seat_authorities: BTreeMap<String, ProcedureSeatAuthority>,
    pub deadline: SimTime,
    pub evidence: Vec<EvidenceRef>,
    pub closed: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProcedureSeatAuthority {
    pub holder: KnowledgeHolderRef,
    pub permission_profile_id: String,
    pub decision_controller_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LegalCapacityRequirement {
    pub procedure: String,
    pub pool: String,
    pub quantity: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LegalCapacityAllocation {
    pub procedure: String,
    pub pool: String,
    pub quantity: u64,
    pub admitted_at: SimTime,
    pub evidence: EvidenceRef,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Ballot {
    For,
    Against,
    Abstain,
    Veto,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProcedureParticipation {
    pub id: String,
    pub procedure: LegalRecordRef,
    pub stage: String,
    pub round: u32,
    pub seat: String,
    pub controller: KnowledgeHolderRef,
    pub ballot: Ballot,
    pub option_id: String,
    pub admitted_at: SimTime,
    pub command: Option<EvidenceRef>,
    pub replaced: Option<LegalRecordRef>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DispatchState {
    Pending,
    Enqueued,
    Acknowledged,
    Expired,
    Blocked,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LegalActorContextRequirement {
    pub procedure: String,
    pub stage: usize,
    pub round: u32,
    pub seat: String,
    pub holder: KnowledgeHolderRef,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LegalActorContext {
    pub holder: KnowledgeHolderRef,
    pub read_cut: canwu_api::KnowledgeReadCut,
    pub knowledge_record_ids: Vec<canwu_api::HolderKnowledgeRecordId>,
    pub facts: serde_json::Value,
    pub context_hash: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LegalDecisionOutboxItem {
    pub sequence: u64,
    pub id: String,
    pub operation: String,
    pub ticket_id: u64,
    pub create_request_id: u64,
    pub refresh_request_id: Option<u64>,
    pub resolution_request_id: u64,
    pub nested_command_request_id: u64,
    pub enqueue_expected_revision: Option<u64>,
    pub enqueue_ingress: Option<EvidenceRef>,
    /// Commitment to the accepted controller/open outcomes and resulting core state.
    pub enqueue_outcome_commitment: Option<String>,
    pub proposal: LegalRecordRef,
    pub procedure: LegalRecordRef,
    pub stage: usize,
    pub round: u32,
    pub seat: String,
    pub decision_controller_id: String,
    pub permission_profile_id: String,
    pub source_boundary: Option<EvidenceRef>,
    pub controller: KnowledgeHolderRef,
    pub command_subject: Option<EntityRef>,
    pub draft: DecisionTicketDraft,
    pub knowledge_read_cut: canwu_api::KnowledgeReadCut,
    pub knowledge_record_ids: Vec<canwu_api::HolderKnowledgeRecordId>,
    pub context_hash: String,
    pub due_at: SimTime,
    pub priority: i32,
    pub dispatch: DispatchState,
    pub expires_at: SimTime,
    pub acknowledgement: Option<EvidenceRef>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PendingLegalIntent {
    pub id: String,
    pub command: EvidenceRef,
    pub attempt: Option<EvidenceRef>,
    pub request_id: Option<u64>,
    pub controller: KnowledgeHolderRef,
    pub seat: String,
    pub proposal: LegalRecordRef,
    pub procedure: LegalRecordRef,
    pub round: u32,
    #[serde(default)]
    pub stage: usize,
    pub expected_versions: Vec<DomainRecordVersionRef>,
    pub selected_option: String,
    pub clause_hash: String,
    pub intended_effective_at: SimTime,
    pub admitted_at: SimTime,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LegalIntentStatus {
    Pending,
    Accepted,
    Rejected,
    Expired,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LegalIntentOutcome {
    pub intent: String,
    pub status: LegalIntentStatus,
    pub reason: Option<String>,
    pub source: Option<LegalRecordRef>,
    pub law_versions: Vec<LegalRecordRef>,
    pub at: SimTime,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LegalSourceVersion {
    pub id: String,
    pub ordinal: u64,
    pub proposal: LegalRecordRef,
    pub mode: SourceMode,
    pub legal_order: String,
    pub applicability_profile: String,
    pub issuer: Option<EntityRef>,
    pub claimant: Option<KnowledgeHolderRef>,
    pub procedure: Option<LegalRecordRef>,
    pub content_hash: String,
    pub text_hash: String,
    pub competence_claim: String,
    pub competence: LegalCompetenceDisposition,
    pub validity: OperativeDisposition,
    pub origin: Option<LegalOriginRef>,
    pub authority_policy: SourceAuthorityPolicy,
    pub publicity_policy: PublicityPolicy,
    pub publicity_event: Option<LegalRecordRef>,
    pub publicity: String,
    pub defects: Vec<String>,
    #[serde(default)]
    pub evidence_kinds: Vec<String>,
    pub adopted_at: SimTime,
    pub promulgated_at: Option<SimTime>,
    pub effective_at: SimTime,
    pub expires_at: Option<SimTime>,
    pub evidence: Vec<EvidenceRef>,
    pub cultural_dependencies: Vec<LegalCulturalDependency>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LegalPublicityEvent {
    pub id: String,
    pub proposal: LegalRecordRef,
    pub at: SimTime,
    pub signal_kind: String,
    pub medium: String,
    pub scope: Vec<String>,
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LegalRule {
    pub id: String,
    pub legal_order: String,
    pub latest_adopted_version: Option<LegalRecordRef>,
    pub operative_version: Option<LegalRecordRef>,
    pub scheduled_versions: Vec<LegalRecordRef>,
    pub effects: Vec<NormativeEffect>,
    pub retired: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LawOperation {
    Establish,
    Recognize,
    Receive,
    Amend,
    Suspend,
    Resume,
    Displace,
    Annul,
    Repeal,
    Expire,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NormativeModality {
    Duty,
    Prohibition,
    Liberty,
    ClaimRight,
    Power,
    Liability,
    Immunity,
    Disability,
    Status,
    Eligibility,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NormativeEffect {
    pub id: String,
    pub modality: NormativeModality,
    pub holders: Vec<String>,
    pub duty_bearers: Vec<String>,
    pub subject_matters: Vec<String>,
    pub territories: Vec<canwu_api::TerritoryId>,
    pub action: String,
    pub object: String,
    pub conditions: Vec<String>,
    pub exceptions: Vec<String>,
    pub standing: Vec<String>,
    pub forum: Option<String>,
    pub remedy_profile: Option<String>,
    pub source_refs: Vec<LegalRecordRef>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LawVersion {
    pub id: String,
    pub rule: String,
    pub legal_ordinal: u64,
    pub operation: LawOperation,
    pub applicability_profile: String,
    pub source: LegalRecordRef,
    pub origin: Option<LegalOriginRef>,
    pub predecessors: Vec<LegalRecordRef>,
    pub deltas: Vec<NormativeEffect>,
    pub jurisdictions: Vec<String>,
    pub adopted_at: SimTime,
    pub promulgated_at: Option<SimTime>,
    pub effective_at: SimTime,
    pub retrospective_from: Option<SimTime>,
    pub disposition: OperativeDisposition,
    pub evidence: Vec<EvidenceRef>,
    pub cultural_dependencies: Vec<LegalCulturalDependency>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperativeDisposition {
    Claimed,
    Purported,
    Operative,
    Suspended,
    Displaced,
    Annulled,
    Repealed,
    Expired,
    Contested,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LegalCase {
    pub id: String,
    pub legal_order: String,
    pub subject_matters: Vec<String>,
    pub parties: Vec<EntityRef>,
    pub claims: Vec<String>,
    pub forum: String,
    pub standing: Option<String>,
    pub proof_profile: String,
    pub issues: Vec<String>,
    pub deadline: SimTime,
    pub remedies: Vec<String>,
    pub allegations: Vec<EvidenceRef>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LegalFindingVersion {
    pub id: String,
    pub case_id: String,
    pub issue: String,
    pub finding: String,
    pub accepted: bool,
    pub burden: String,
    pub evidence: Vec<EvidenceRef>,
    pub at: SimTime,
    pub predecessor: Option<LegalRecordRef>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LegalRulingVersion {
    pub id: String,
    /// V1 rulings are always decisions in an exact, previously admitted case.
    pub case_id: String,
    pub institution: String,
    pub issues: Vec<String>,
    pub findings: Vec<LegalRecordRef>,
    pub sources: Vec<LegalRecordRef>,
    /// Exact law versions put in issue by this ruling.
    pub resolved_versions: Vec<LegalRecordRef>,
    /// Exact subset selected as governing by this ruling.
    pub selected_versions: Vec<LegalRecordRef>,
    pub scope: Vec<String>,
    pub precedent_profile: Option<String>,
    pub effective_from: SimTime,
    pub effective_until: Option<SimTime>,
    pub remedy: Option<String>,
    pub predecessors: Vec<LegalRecordRef>,
    pub disposition: OperativeDisposition,
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplicabilityOutcome {
    Applicable,
    NotApplicable,
    Displaced,
    Contested,
    Indeterminate,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ApplicabilityQuery {
    pub event_at: SimTime,
    pub read_at: SimTime,
    pub subject: Option<DomainRecordRef>,
    pub actor: Option<KnowledgeHolderRef>,
    /// Exact actor-relative knowledge cut; required exactly when `actor` is set.
    pub knowledge_read_cut: Option<canwu_api::KnowledgeReadCut>,
    pub territory: Option<canwu_api::TerritoryId>,
    pub subject_matter: Option<String>,
    /// Required partition key; cross-order scans are intentionally unsupported.
    pub legal_order: String,
    pub profile: String,
    pub jurisdiction: Option<String>,
    pub facts: BTreeMap<String, bool>,
    /// One exact provenance reference for every asserted predicate fact.
    pub fact_evidence: BTreeMap<String, EvidenceRef>,
    /// Actor-relative provenance for each fact; empty for objective queries.
    pub fact_knowledge_records: BTreeMap<String, canwu_api::HolderKnowledgeRecordId>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ApplicabilityResult {
    pub query_hash: String,
    pub outcome: ApplicabilityOutcome,
    pub versions: Vec<LegalRecordRef>,
    pub displaced: Vec<LegalRecordRef>,
    pub conflicts: Vec<String>,
    pub trace: Vec<LegalRecordRef>,
    pub at: SimTime,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LegalConflict {
    pub id: String,
    pub versions: Vec<LegalRecordRef>,
    pub governing_versions: Vec<LegalRecordRef>,
    pub displaced_versions: Vec<LegalRecordRef>,
    pub jurisdiction: Option<String>,
    pub recorded_at: SimTime,
    pub effective_from: SimTime,
    pub effective_until: Option<SimTime>,
    pub resolution: ApplicabilityOutcome,
    pub basis: ConflictResolutionBasis,
    pub rationale: String,
    pub ruling: Option<LegalRecordRef>,
    pub trace: Vec<LegalRecordRef>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SuccessionKind {
    Conquest,
    Union,
    Split,
    Secession,
    Restoration,
    ConstitutionalReplacement,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceptionAction {
    Continue,
    Transform,
    Review,
    Displace,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReceptionRule {
    pub rule_prefix: String,
    pub action: ReceptionAction,
    pub transform: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LegalOrderSuccession {
    pub id: String,
    pub kind: SuccessionKind,
    pub predecessors: Vec<String>,
    pub successors: Vec<String>,
    pub effective_at: SimTime,
    pub territorial_scope: Vec<String>,
    pub personal_scope: Vec<String>,
    pub institutions: Vec<String>,
    pub liabilities: Vec<String>,
    pub archives: Vec<String>,
    pub reception: Vec<ReceptionRule>,
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LegalRetirement {
    pub id: String,
    pub kind: String,
    pub record: DomainRecordRef,
    pub cultural_target: Option<CulturalTargetGenerationRef>,
    pub retired_at: SimTime,
    pub successor: Option<DomainRecordRef>,
    pub reason: String,
    pub evidence: Vec<EvidenceRef>,
}

marker!(LegalProposalRecord, "proposal", LegalProposal);
marker!(ProcedureInstanceRecord, "procedure", ProcedureInstance);
marker!(
    ProcedureParticipationRecord,
    "participation",
    ProcedureParticipation
);
marker!(LegalDecisionOutboxRecord, "outbox", LegalDecisionOutboxItem);
marker!(
    PendingLegalIntentRecord,
    "pending_intent",
    PendingLegalIntent
);
marker!(
    LegalIntentOutcomeRecord,
    "intent_outcome",
    LegalIntentOutcome
);
marker!(
    LegalSourceVersionRecord,
    "source_version",
    LegalSourceVersion
);
marker!(LegalPublicityEventRecord, "publicity", LegalPublicityEvent);
marker!(LegalRuleRecord, "rule", LegalRule);
marker!(LawVersionRecord, "law_version", LawVersion);
marker!(LegalCaseRecord, "case", LegalCase);
marker!(LegalFindingVersionRecord, "finding", LegalFindingVersion);
marker!(LegalRulingVersionRecord, "ruling", LegalRulingVersion);
marker!(
    ApplicabilityResultRecord,
    "applicability",
    ApplicabilityResult
);
marker!(LegalConflictRecord, "conflict", LegalConflict);
marker!(
    LegalOrderSuccessionRecord,
    "succession",
    LegalOrderSuccession
);
marker!(LegalRetirementRecord, "retirement", LegalRetirement);
marker!(LegalRuntimeRecord, "runtime", crate::LegalRuntime);
marker!(LegalPlanStateRecord, "plan_state", crate::LegalPlanState);
marker!(
    LegalDirectoryStateRecord,
    "directory_state",
    crate::LegalDirectoryState
);
marker!(LegalShardStateRecord, "shard_state", crate::LegalShardState);
marker!(
    LegalArchiveHeadStateRecord,
    "archive_head_state",
    crate::LegalArchiveHeadState
);

pub fn typed_ref<T: DomainRecordType>(id: impl Into<String>) -> TypedDomainRecordRef<T> {
    TypedDomainRecordRef::new(id)
}

#[must_use]
pub fn legal_runtime_reference() -> TypedDomainRecordRef<LegalRuntimeRecord> {
    TypedDomainRecordRef::new("root")
}

#[must_use]
pub fn legal_plan_state_reference() -> TypedDomainRecordRef<LegalPlanStateRecord> {
    TypedDomainRecordRef::new("plan")
}

#[must_use]
pub fn legal_directory_state_reference() -> TypedDomainRecordRef<LegalDirectoryStateRecord> {
    TypedDomainRecordRef::new("directory")
}

pub fn legal_shard_state_reference(
    shard: &crate::LegalShardKey,
) -> Result<TypedDomainRecordRef<LegalShardStateRecord>, CanwuError> {
    Ok(TypedDomainRecordRef::new(format!(
        "shard:{}",
        canonical_hash("canwu.law.shard-record-id.v1", shard)?
    )))
}

pub fn legal_archive_head_state_reference(
    shard: &crate::LegalShardKey,
) -> Result<TypedDomainRecordRef<LegalArchiveHeadStateRecord>, CanwuError> {
    Ok(TypedDomainRecordRef::new(format!(
        "archive:{}",
        canonical_hash("canwu.law.archive-head-record-id.v1", shard)?
    )))
}

pub(crate) fn invalid(message: impl Into<String>) -> CanwuError {
    CanwuError::new(canwu_api::ErrorCode::InvalidDomainRecord, message)
}
pub(crate) fn canonical_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':'))
}
pub(crate) fn check_ids<T, F>(items: &[T], mut get: F, label: &str) -> Result<(), CanwuError>
where
    F: FnMut(&T) -> &str,
{
    let mut ids = BTreeSet::new();
    for item in items {
        let id = get(item);
        if !canonical_id(id) {
            return Err(invalid(format!("{label} ID {id:?} is not canonical")));
        }
        if !ids.insert(id.to_owned()) {
            return Err(invalid(format!("duplicate {label} ID {id}")));
        }
    }
    Ok(())
}
