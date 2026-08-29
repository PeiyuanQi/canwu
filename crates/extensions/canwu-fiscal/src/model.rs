use crate::PLUGIN_NAME;
use canwu_api::{
    CanwuError, DomainRecord, DomainRecordClass, DomainRecordDraft, DomainRecordKind,
    DomainRecordLifecycle, DomainRecordType, DomainRecordVersionRef, DomainRecordVersionSource,
    DomainReference, DomainValueKindClass, EntityRef, ErrorCode, EvidenceRef, IngressId,
    KnowledgeHolderRef, PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD,
    PayloadRequiredEvidenceContinuationV1, PersonId, ResourceId, SimTime, SimulationGranularity,
    TypedDomainRecordRef,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const FISCAL_CONTENT_SCHEMA_VERSION: u32 = 1;
pub const FISCAL_RUNTIME_SCHEMA_VERSION: u32 = 2;
pub const MAX_FISCAL_RUNTIME_BINDINGS: usize = 4_096;
pub const MAX_FISCAL_OBSERVERS: usize = 64;
pub const MAX_FISCAL_ASSESSMENTS: usize = 4_096;
pub const MAX_FISCAL_EXECUTION_REQUESTS: usize = 8_192;
pub const MAX_FISCAL_EXECUTION_RECEIPTS: usize = 8_192;
pub const MAX_FISCAL_ACTION_OUTCOMES: usize = 16_384;
pub const MAX_FISCAL_EVIDENCE_KINDS: usize = 32;
pub const MAX_FISCAL_EVIDENCE_PER_RECORD: usize = 32;
pub const MAX_FISCAL_STATE_JSON_BYTES: usize = 32 * 1_024 * 1_024;
pub const MAX_FISCAL_CATALOG_PERIODS: usize = 512;
pub const MAX_FISCAL_CATALOG_REGIONS: usize = 1_024;
pub const MAX_FISCAL_CATALOG_DEFINITIONS: usize = 16_384;
pub const MAX_FISCAL_CATALOG_COVERAGE_CELLS: usize = 65_536;
pub const MAX_FISCAL_REFERENCES_PER_DEFINITION: usize = 1_024;
pub const MAX_FISCAL_CATALOG_JSON_BYTES: usize = 64 * 1_024 * 1_024;
const ROOT_ID: &str = "primary";

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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalActionRequest {
    pub action_id: String,
    pub authority_binding_id: String,
    pub expected_procedure_revision: u64,
    pub action: FiscalAction,
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
    GrantRemission {
        remission_id: String,
        assessment_id: String,
        quantity: u64,
        reason: String,
    },
    AuthorizeExecution {
        request_id: String,
        assessment_id: String,
        kind: FiscalExecutionKind,
        quantity: u64,
        unit: String,
        resource: ResourceId,
        source: EntityRef,
        target: EntityRef,
        purpose: String,
    },
    RecordAudit {
        audit_id: String,
        target_id: String,
        severity: FiscalAuditSeverity,
        finding: String,
        evidence: BTreeSet<EvidenceRef>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FiscalExecutionReceiptPacket {
    pub receipt_id: String,
    pub request_id: String,
    pub external_evidence: BTreeSet<DomainRecordVersionRef>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalExecutionEvidence {
    pub id: String,
    pub external_operation_id: String,
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalAuthorityBinding {
    pub id: String,
    pub institution: EntityRef,
    pub authorized_actor: Option<PersonId>,
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalObserverBinding {
    pub id: String,
    pub actor: PersonId,
    pub knowledge_holder: KnowledgeHolderRef,
    pub visible_institutions: BTreeSet<EntityRef>,
    pub confidence_per_mille: u16,
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
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalRemission {
    pub id: String,
    pub assessment_id: String,
    pub quantity: u64,
    pub reason: String,
    pub granted_at: SimTime,
}

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
}

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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalAuditFinding {
    pub id: String,
    pub target_id: String,
    pub severity: FiscalAuditSeverity,
    pub finding: String,
    pub evidence: BTreeSet<EvidenceRef>,
    pub recorded_at: SimTime,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalActionOutcome {
    pub action_id: String,
    pub disposition: FiscalActionDisposition,
    pub reason: String,
    pub command: canwu_api::CommandId,
    pub settled_at: SimTime,
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalProjection {
    pub actor: PersonId,
    pub as_of: SimTime,
    pub confidence_per_mille: u16,
    pub facts: BTreeMap<String, FiscalReportFact>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FiscalState {
    pub schema_version: u32,
    pub procedure_revision: u64,
    pub catalog_version: DomainRecordVersionRef,
    pub historical_context: FiscalHistoricalContext,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub execution_evidence_kinds: BTreeSet<DomainRecordKind>,
    pub authority_bindings: BTreeMap<String, FiscalAuthorityBinding>,
    pub scope_bindings: BTreeMap<String, FiscalScopeBinding>,
    pub observer_bindings: BTreeMap<String, FiscalObserverBinding>,
    pub adoptions: BTreeMap<String, FiscalAdoptionState>,
    pub assessments: BTreeMap<String, FiscalAssessment>,
    pub remissions: BTreeMap<String, FiscalRemission>,
    pub execution_requests: BTreeMap<String, FiscalExecutionRequest>,
    pub execution_receipts: BTreeMap<String, FiscalExecutionReceipt>,
    pub audits: BTreeMap<String, FiscalAuditFinding>,
    pub action_outcomes: BTreeMap<String, FiscalActionOutcome>,
    pub transition_candidates: BTreeMap<String, FiscalTransitionCandidate>,
    pub aggregates: BTreeMap<String, FiscalStrategicAggregate>,
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
            scope_bindings: BTreeMap::new(),
            observer_bindings: BTreeMap::new(),
            adoptions: BTreeMap::new(),
            assessments: BTreeMap::new(),
            remissions: BTreeMap::new(),
            execution_requests: BTreeMap::new(),
            execution_receipts: BTreeMap::new(),
            audits: BTreeMap::new(),
            action_outcomes: BTreeMap::new(),
            transition_candidates: BTreeMap::new(),
            aggregates: BTreeMap::new(),
        }
    }

    #[allow(clippy::too_many_lines)]
    pub fn validate(&self, catalog: &CompiledFiscalCatalog) -> Result<(), CanwuError> {
        if self.schema_version != FISCAL_RUNTIME_SCHEMA_VERSION {
            return Err(invalid("unsupported fiscal runtime schema"));
        }
        if self.procedure_revision == 0 {
            return Err(invalid("fiscal procedure revision must be nonzero"));
        }
        if self.execution_evidence_kinds.len() > MAX_FISCAL_EVIDENCE_KINDS
            || self.authority_bindings.len() > MAX_FISCAL_RUNTIME_BINDINGS
            || self.scope_bindings.len() > MAX_FISCAL_RUNTIME_BINDINGS
            || self.observer_bindings.len() > MAX_FISCAL_OBSERVERS
            || self.adoptions.len() > MAX_FISCAL_EXECUTION_REQUESTS
            || self.assessments.len() > MAX_FISCAL_ASSESSMENTS
            || self.remissions.len() > MAX_FISCAL_ASSESSMENTS
            || self.execution_requests.len() > MAX_FISCAL_EXECUTION_REQUESTS
            || self.execution_receipts.len() > MAX_FISCAL_EXECUTION_RECEIPTS
            || self.audits.len() > MAX_FISCAL_ASSESSMENTS
            || self.action_outcomes.len() > MAX_FISCAL_ACTION_OUTCOMES
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
        validate_map(&self.scope_bindings, |value| &value.id, "scope binding")?;
        validate_map(
            &self.observer_bindings,
            |value| &value.id,
            "observer binding",
        )?;
        validate_map(&self.adoptions, |value| &value.id, "adoption")?;
        validate_map(&self.assessments, |value| &value.id, "assessment")?;
        validate_map(&self.remissions, |value| &value.id, "remission")?;
        validate_map(
            &self.execution_requests,
            |value| &value.id,
            "execution request",
        )?;
        validate_map(
            &self.execution_receipts,
            |value| &value.id,
            "execution receipt",
        )?;
        validate_map(&self.audits, |value| &value.id, "audit")?;
        validate_map(
            &self.action_outcomes,
            |value| &value.action_id,
            "action outcome",
        )?;
        validate_map(
            &self.transition_candidates,
            |value| &value.id,
            "transition candidate",
        )?;
        if self.observer_bindings.values().any(|value| {
            value.confidence_per_mille > 1_000
                || value.knowledge_holder != KnowledgeHolderRef::Person(value.actor)
                || value.visible_institutions.len() > MAX_FISCAL_RUNTIME_BINDINGS
        }) {
            return Err(invalid(
                "fiscal observers require bounded confidence and their actor's person holder",
            ));
        }
        for scope in self.scope_bindings.values() {
            if !catalog.regions.contains_key(&scope.jurisdiction_id) {
                return Err(invalid(format!(
                    "scope {} references unavailable jurisdiction {}",
                    scope.id, scope.jurisdiction_id
                )));
            }
        }
        for adoption in self.adoptions.values() {
            let rule = catalog.rules.get(&adoption.rule_id).ok_or_else(|| {
                invalid(format!("adoption {} references unknown rule", adoption.id))
            })?;
            let scope = self
                .scope_bindings
                .get(&adoption.scope_binding_id)
                .ok_or_else(|| {
                    invalid(format!("adoption {} references unknown scope", adoption.id))
                })?;
            if rule.mechanism != scope.mechanism || adoption.generation == 0 {
                return Err(invalid(format!(
                    "adoption {} is inconsistent with its scope or generation",
                    adoption.id
                )));
            }
        }
        let operational_adoptions: BTreeSet<_> = self
            .adoptions
            .values()
            .filter(|adoption| adoption.stage.is_operational())
            .map(|adoption| {
                (
                    adoption.rule_id.as_str(),
                    adoption.scope_binding_id.as_str(),
                )
            })
            .collect();
        let mut settlement_keys = BTreeSet::new();
        for assessment in self.assessments.values() {
            let rule = catalog.rules.get(&assessment.rule_id).ok_or_else(|| {
                invalid(format!(
                    "assessment {} references unknown rule",
                    assessment.id
                ))
            })?;
            let scope = self
                .scope_bindings
                .get(&assessment.scope_binding_id)
                .ok_or_else(|| {
                    invalid(format!(
                        "assessment {} references unknown scope",
                        assessment.id
                    ))
                })?;
            if assessment.quantity == 0
                || assessment.unit.is_empty()
                || rule.mechanism != scope.mechanism
                || !rule.payment_forms.contains(&assessment.payment_form)
            {
                return Err(invalid(format!("assessment {} is invalid", assessment.id)));
            }
            if !operational_adoptions.contains(&(
                assessment.rule_id.as_str(),
                assessment.scope_binding_id.as_str(),
            )) {
                return Err(invalid(format!(
                    "assessment {} uses a rule that is not operational in its scope",
                    assessment.id
                )));
            }
            let key = (
                assessment.scope_binding_id.clone(),
                assessment.accounting_cycle_id.clone(),
                assessment.unit.clone(),
                assessment.payment_form,
            );
            if !settlement_keys.insert(key) {
                return Err(invalid(
                    "the same fiscal accounting partition was assessed more than once",
                ));
            }
        }
        let mut remitted_by_assessment = BTreeMap::<&str, u64>::new();
        for remission in self.remissions.values() {
            if remission.quantity == 0 || !self.assessments.contains_key(&remission.assessment_id) {
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
        for request in self.execution_requests.values() {
            let assessment = self
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
                || request.institution
                    != self.scope_bindings[&assessment.scope_binding_id].institution
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
        for assessment in self.assessments.values() {
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
        let mut consumed_evidence = BTreeSet::new();
        let mut consumed_operations = BTreeSet::new();
        for receipt in self.execution_receipts.values() {
            let counts_as_fulfillment = receipt.disposition.counts_as_fulfillment();
            if receipt.external_evidence.is_empty()
                || receipt.external_evidence.len() > MAX_FISCAL_EVIDENCE_PER_RECORD
                || receipt.external_operations.len() != receipt.external_evidence.len()
                || !self.execution_requests.contains_key(&receipt.request_id)
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
        for request in self.execution_requests.values() {
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
        for audit in self.audits.values() {
            if audit.evidence.len() > MAX_FISCAL_EVIDENCE_PER_RECORD
                || !self.assessments.contains_key(&audit.target_id)
                    && !self.execution_requests.contains_key(&audit.target_id)
                    && !self.execution_receipts.contains_key(&audit.target_id)
            {
                return Err(invalid(format!("audit {} has an unknown target", audit.id)));
            }
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
        let mut draft = DomainRecordDraft::from_typed(fiscal_state_reference(), self)?;
        let dependencies = self
            .assessments
            .values()
            .filter_map(|assessment| assessment.commutation_quote.clone())
            .chain(
                self.execution_receipts
                    .values()
                    .flat_map(|receipt| receipt.external_evidence.iter().cloned()),
            )
            .filter(|reference| {
                matches!(
                    reference.established_by,
                    DomainRecordVersionSource::BoundaryChange { .. }
                )
            })
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
        if encoded_len > MAX_FISCAL_STATE_JSON_BYTES {
            return Err(invalid("fiscal state exceeds its serialized byte budget"));
        }
        draft.references = vec![DomainReference::from_typed(
            "catalog",
            fiscal_catalog_reference(),
        )];
        Ok(draft)
    }

    pub fn validate_record_binding(&self, record: &DomainRecord) -> Result<(), CanwuError> {
        let expected = vec![DomainReference::from_typed(
            "catalog",
            fiscal_catalog_reference(),
        )];
        let expected_payload = self.record_draft()?.payload;
        if record.references != expected || record.payload != expected_payload {
            return Err(invalid(
                "fiscal state record does not match its decoded payload and catalog root",
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
        || value.len() > 160
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
