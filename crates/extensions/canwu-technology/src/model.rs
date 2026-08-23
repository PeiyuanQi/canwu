use crate::PLUGIN_NAMESPACE;
use canwu_api::{
    DomainRecord, DomainRecordClass, DomainRecordDraft, DomainRecordLifecycle, DomainRecordRef,
    DomainRecordType, DomainRecordVersionRef, DomainRecordVersionSource, DomainReference,
    DomainReferenceTarget, DomainValueKindClass, EntityRef, EvidenceRef, KnowledgeHolderRef,
    PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD, PayloadRequiredEvidenceContinuationV1, SimTime,
    TypedDomainRecordRef,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const REFERENCE_EVALUATOR_V1: &str = "canwu.technology.threshold-evaluator.v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TechnologyLimitsV1 {
    pub max_payload_bytes: usize,
    pub max_references: usize,
    pub max_collection_entries: usize,
    pub max_ancestry_depth: usize,
    pub max_total_records: usize,
    pub max_records_per_kind: usize,
    pub max_knowledge_records: usize,
    pub max_mutations_per_boundary: usize,
    pub max_publications_per_boundary: usize,
}

impl TechnologyLimitsV1 {
    #[must_use]
    pub const fn canonical() -> Self {
        Self {
            max_payload_bytes: 16 * 1024,
            max_references: 32,
            max_collection_entries: 64,
            max_ancestry_depth: 8,
            max_total_records: 5_000,
            max_records_per_kind: 5_000,
            max_knowledge_records: 5_000,
            max_mutations_per_boundary: 64,
            max_publications_per_boundary: 32,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricComparison {
    AtLeast,
    AtMost,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MetricSchemaPayload {
    pub label: String,
    pub unit: String,
    pub scale: u32,
    pub minimum: i64,
    pub maximum: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MetricValue {
    pub metric: DomainRecordVersionRef,
    pub value: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MetricThreshold {
    pub id: String,
    pub metric: DomainRecordVersionRef,
    pub comparison: MetricComparison,
    pub value: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RequirementGroup {
    pub id: String,
    pub any_of: Vec<MetricThreshold>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct QualificationRule {
    pub operation: String,
    pub minimum_successful_attempts: u16,
    pub minimum_reliability_per_mille: u16,
    pub independent_reproduction_required: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TechniqueSpecPayload {
    pub label: String,
    pub function: String,
    pub requirements: Vec<RequirementGroup>,
    pub qualification_rules: Vec<QualificationRule>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RevisionRelationKind {
    Contribution,
    Correction,
    Alternative,
    Supersession,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RevisionRelation {
    pub parent: DomainRecordVersionRef,
    pub relation: RevisionRelationKind,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TechniqueRevisionPayload {
    pub label: String,
    pub spec: DomainRecordVersionRef,
    pub parents: Vec<RevisionRelation>,
    pub parameters: Vec<MetricValue>,
    pub evaluator: String,
    pub produced_by: Option<DomainRecordVersionRef>,
    pub execution_intent: Option<DomainRecordVersionRef>,
    pub discovery_evidence: Vec<EvidenceRef>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ApplicationSpecPayload {
    pub label: String,
    pub technique: DomainRecordVersionRef,
    pub viability: Vec<RequirementGroup>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgramMode {
    Investigation,
    Adaptation,
    Training,
    Repair,
    ReverseEngineering,
    Troubleshooting,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgramStatus {
    Active,
    Paused,
    Completed,
    Cancelled,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProviderRequirement {
    pub provider: String,
    pub capability: String,
    pub quantity: u64,
    pub unit: String,
    pub evidence: Option<EvidenceRef>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TechnicalProgramPayload {
    pub sponsor: KnowledgeHolderRef,
    pub site: EntityRef,
    pub revision: Option<DomainRecordVersionRef>,
    pub mode: ProgramMode,
    pub status: ProgramStatus,
    pub requirements: Vec<ProviderRequirement>,
    pub started_at: SimTime,
    pub due_at: Option<SimTime>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum TechnologyIntentRequest {
    Experiment {
        result_id: String,
        revision: DomainRecordVersionRef,
        operation: String,
        site: EntityRef,
        operator: Option<KnowledgeHolderRef>,
        required_assets: Vec<DomainRecordVersionRef>,
    },
    Production {
        result_id: String,
        revision: DomainRecordVersionRef,
        application: Option<DomainRecordVersionRef>,
        site: EntityRef,
        operator: Option<KnowledgeHolderRef>,
        required_assets: Vec<DomainRecordVersionRef>,
    },
    Invention {
        result_id: String,
        spec: DomainRecordVersionRef,
        parent: Option<DomainRecordVersionRef>,
        site: EntityRef,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum TechnologyIntentState {
    Pending,
    Consumed {
        ingress: EvidenceRef,
        operation: DomainRecordVersionRef,
        result: DomainRecordVersionRef,
    },
    Cancelled,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TechnologyExecutionIntentPayload {
    pub authorized_by: KnowledgeHolderRef,
    pub program: DomainRecordVersionRef,
    pub provider: String,
    pub request: TechnologyIntentRequest,
    pub not_before: SimTime,
    pub expires_at: Option<SimTime>,
    pub state: TechnologyIntentState,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EvaluationResult {
    pub evaluator: String,
    pub passed: bool,
    pub satisfied_groups: Vec<String>,
    pub failed_groups: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExperimentAttemptPayload {
    pub execution_intent: DomainRecordVersionRef,
    pub program: DomainRecordVersionRef,
    pub revision: DomainRecordVersionRef,
    pub operator: KnowledgeHolderRef,
    pub site: EntityRef,
    pub operation: String,
    pub inputs: Vec<MetricValue>,
    pub environment: Vec<MetricValue>,
    pub outputs: Vec<MetricValue>,
    pub assets: Vec<DomainRecordVersionRef>,
    pub started_at: SimTime,
    pub ended_at: SimTime,
    pub evaluation: EvaluationResult,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AttemptObservationPayload {
    pub attempt: DomainRecordVersionRef,
    pub observer: KnowledgeHolderRef,
    pub method: String,
    pub values: Vec<MetricValue>,
    pub uncertainty_per_mille: u16,
    pub observed_at: SimTime,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ClaimRelation {
    pub claim: DomainRecordVersionRef,
    pub relation: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TechnicalClaimPayload {
    pub asserted_by: KnowledgeHolderRef,
    pub proposition: String,
    pub scope: Vec<DomainRecordRef>,
    pub source_evidence: Vec<EvidenceRef>,
    pub relations: Vec<ClaimRelation>,
    pub asserted_at: SimTime,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ClaimAssessmentPayload {
    pub claim: DomainRecordVersionRef,
    pub assessor: KnowledgeHolderRef,
    pub confidence_per_mille: u16,
    pub method: String,
    pub supporting_evidence: Vec<EvidenceRef>,
    pub contradicting_evidence: Vec<EvidenceRef>,
    pub as_of: SimTime,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CapabilityQualificationPayload {
    pub holder: KnowledgeHolderRef,
    pub operator: Option<EntityRef>,
    pub site: EntityRef,
    pub revision: DomainRecordVersionRef,
    pub operation: String,
    pub reliability_per_mille: u16,
    pub attempts: Vec<DomainRecordVersionRef>,
    pub last_practiced_at: SimTime,
    pub valid_from: SimTime,
    pub valid_until: Option<SimTime>,
    pub active: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AssetBindingPayload {
    pub owner: KnowledgeHolderRef,
    pub site: EntityRef,
    pub provider_asset: EvidenceRef,
    pub capabilities: Vec<String>,
    pub condition_per_mille: u16,
    pub active: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionRunPayload {
    pub execution_intent: DomainRecordVersionRef,
    pub revision: DomainRecordVersionRef,
    pub application: Option<DomainRecordVersionRef>,
    pub operator: KnowledgeHolderRef,
    pub site: EntityRef,
    pub assets: Vec<DomainRecordVersionRef>,
    pub inputs: Vec<MetricValue>,
    pub outputs: Vec<MetricValue>,
    pub started_at: SimTime,
    pub ended_at: SimTime,
    pub successful: bool,
    pub evaluation: EvaluationResult,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ImplementationPayload {
    pub owner: KnowledgeHolderRef,
    pub site: EntityRef,
    pub revision: DomainRecordVersionRef,
    pub qualification: DomainRecordVersionRef,
    pub assets: Vec<DomainRecordVersionRef>,
    pub installed_at: SimTime,
    pub capacity: u64,
    pub unit: String,
    pub reliability_per_mille: u16,
    pub maintenance_provider: Option<KnowledgeHolderRef>,
    pub active: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AdoptionStatus {
    Trial,
    Committed,
    Suspended,
    Abandoned,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AdoptionPayload {
    pub adopter: KnowledgeHolderRef,
    pub site: EntityRef,
    pub application: DomainRecordVersionRef,
    pub implementations: Vec<DomainRecordVersionRef>,
    pub status: AdoptionStatus,
    pub scale: u64,
    pub decision_evidence: EvidenceRef,
    pub viability_evidence: Vec<DomainRecordVersionRef>,
    pub viability_metrics: Vec<MetricValue>,
    pub viability: EvaluationResult,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TransmissionMode {
    DocumentAccess,
    Demonstration,
    Apprenticeship,
    ArtifactInspection,
    PersonnelTransfer,
    IndependentInvestigation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TransmissionOpportunityPayload {
    pub source: Option<KnowledgeHolderRef>,
    pub source_site: Option<EntityRef>,
    pub source_capability: Option<DomainRecordVersionRef>,
    pub destination: KnowledgeHolderRef,
    pub destination_site: EntityRef,
    pub revision: Option<DomainRecordVersionRef>,
    pub mode: TransmissionMode,
    pub evidence: Vec<EvidenceRef>,
    pub resulting_program: Option<DomainRecordVersionRef>,
    pub opened_at: SimTime,
    pub active: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TechnologyOperationStatus {
    Applied,
    Rejected,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TechnologyOperationPayload {
    pub id: String,
    pub canonical_input_hash: String,
    pub canonical_input_hashes: Vec<String>,
    pub causes: Vec<EvidenceRef>,
    pub provider: Option<String>,
    pub execution_intent: Option<DomainRecordVersionRef>,
    pub status: TechnologyOperationStatus,
    pub result: Option<DomainRecordRef>,
    pub rejection_code: Option<String>,
}

macro_rules! record_type {
    ($name:ident, $kind:literal, $payload:ty) => {
        #[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name;

        impl DomainRecordType for $name {
            type Payload = $payload;
            type Class = DomainValueKindClass;

            const NAMESPACE: &'static str = PLUGIN_NAMESPACE;
            const NAME: &'static str = $kind;
        }
    };
}

record_type!(MetricSchema, "metric_schema", MetricSchemaPayload);
record_type!(TechniqueSpec, "technique_spec", TechniqueSpecPayload);
record_type!(
    TechniqueRevision,
    "technique_revision",
    TechniqueRevisionPayload
);
record_type!(ApplicationSpec, "application_spec", ApplicationSpecPayload);
record_type!(
    TechnicalProgram,
    "technical_program",
    TechnicalProgramPayload
);
record_type!(
    TechnologyExecutionIntent,
    "execution_intent",
    TechnologyExecutionIntentPayload
);
record_type!(
    ExperimentAttempt,
    "experiment_attempt",
    ExperimentAttemptPayload
);
record_type!(
    AttemptObservation,
    "attempt_observation",
    AttemptObservationPayload
);
record_type!(TechnicalClaim, "technical_claim", TechnicalClaimPayload);
record_type!(ClaimAssessment, "claim_assessment", ClaimAssessmentPayload);
record_type!(
    CapabilityQualification,
    "capability",
    CapabilityQualificationPayload
);
record_type!(AssetBinding, "asset_binding", AssetBindingPayload);
record_type!(ProductionRun, "production_run", ProductionRunPayload);
record_type!(
    ImplementationRecord,
    "implementation",
    ImplementationPayload
);
record_type!(AdoptionRecord, "adoption", AdoptionPayload);
record_type!(
    TransmissionOpportunity,
    "transmission",
    TransmissionOpportunityPayload
);
record_type!(TechnologyOperation, "operation", TechnologyOperationPayload);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum TechnologyRecordPayload {
    TechniqueRevision(TechniqueRevisionPayload),
    TechnicalProgram(TechnicalProgramPayload),
    ExecutionIntent(TechnologyExecutionIntentPayload),
    ExperimentAttempt(ExperimentAttemptPayload),
    AttemptObservation(AttemptObservationPayload),
    TechnicalClaim(TechnicalClaimPayload),
    ClaimAssessment(ClaimAssessmentPayload),
    Capability(CapabilityQualificationPayload),
    AssetBinding(AssetBindingPayload),
    ProductionRun(ProductionRunPayload),
    Implementation(ImplementationPayload),
    Adoption(AdoptionPayload),
    Transmission(TransmissionOpportunityPayload),
}

impl TechnologyRecordPayload {
    #[must_use]
    pub fn reference(&self, id: impl Into<String>) -> DomainRecordRef {
        let id = id.into();
        match self {
            Self::TechniqueRevision(_) => {
                canwu_api::TypedDomainRecordRef::<TechniqueRevision>::new(id).into_untyped()
            }
            Self::TechnicalProgram(_) => {
                canwu_api::TypedDomainRecordRef::<TechnicalProgram>::new(id).into_untyped()
            }
            Self::ExecutionIntent(_) => {
                canwu_api::TypedDomainRecordRef::<TechnologyExecutionIntent>::new(id).into_untyped()
            }
            Self::ExperimentAttempt(_) => {
                canwu_api::TypedDomainRecordRef::<ExperimentAttempt>::new(id).into_untyped()
            }
            Self::AttemptObservation(_) => {
                canwu_api::TypedDomainRecordRef::<AttemptObservation>::new(id).into_untyped()
            }
            Self::TechnicalClaim(_) => {
                canwu_api::TypedDomainRecordRef::<TechnicalClaim>::new(id).into_untyped()
            }
            Self::ClaimAssessment(_) => {
                canwu_api::TypedDomainRecordRef::<ClaimAssessment>::new(id).into_untyped()
            }
            Self::Capability(_) => {
                canwu_api::TypedDomainRecordRef::<CapabilityQualification>::new(id).into_untyped()
            }
            Self::AssetBinding(_) => {
                canwu_api::TypedDomainRecordRef::<AssetBinding>::new(id).into_untyped()
            }
            Self::ProductionRun(_) => {
                canwu_api::TypedDomainRecordRef::<ProductionRun>::new(id).into_untyped()
            }
            Self::Implementation(_) => {
                canwu_api::TypedDomainRecordRef::<ImplementationRecord>::new(id).into_untyped()
            }
            Self::Adoption(_) => {
                canwu_api::TypedDomainRecordRef::<AdoptionRecord>::new(id).into_untyped()
            }
            Self::Transmission(_) => {
                canwu_api::TypedDomainRecordRef::<TransmissionOpportunity>::new(id).into_untyped()
            }
        }
    }

    pub(crate) fn draft(
        &self,
        id: impl Into<String>,
    ) -> Result<DomainRecordDraft, canwu_api::CanwuError> {
        let reference = self.reference(id);
        let payload = serde_json::to_value(self).map_err(|error| {
            canwu_api::CanwuError::new(
                canwu_api::ErrorCode::InvalidDomainRecord,
                format!("technology record could not be encoded: {error}"),
            )
        })?;
        let mut references = self.references();
        references.sort();
        references.dedup();
        let mut payload = payload.get("payload").cloned().ok_or_else(|| {
            canwu_api::CanwuError::new(
                canwu_api::ErrorCode::InvalidDomainRecord,
                "technology record wrapper has no payload",
            )
        })?;
        attach_payload_continuation(&mut payload, self.exact_versions())?;
        Ok(DomainRecordDraft {
            reference,
            payload,
            references,
        })
    }

    #[must_use]
    pub(crate) fn knowledge_holder(&self) -> Option<&KnowledgeHolderRef> {
        match self {
            Self::TechniqueRevision(_) => None,
            Self::ExecutionIntent(value) => Some(&value.authorized_by),
            Self::AttemptObservation(value) => Some(&value.observer),
            Self::TechnicalClaim(value) => Some(&value.asserted_by),
            Self::ClaimAssessment(value) => Some(&value.assessor),
            Self::Capability(value) => Some(&value.holder),
            Self::Implementation(value) => Some(&value.owner),
            Self::Adoption(value) => Some(&value.adopter),
            Self::Transmission(value) => Some(&value.destination),
            Self::TechnicalProgram(value) => Some(&value.sponsor),
            Self::ExperimentAttempt(value) => Some(&value.operator),
            Self::AssetBinding(value) => Some(&value.owner),
            Self::ProductionRun(value) => Some(&value.operator),
        }
    }

    // Keeping every variant in one exhaustive match makes reference auditing easier.
    #[allow(clippy::too_many_lines)]
    fn references(&self) -> Vec<DomainReference> {
        let mut core = Vec::new();
        let mut domain = Vec::new();
        match self {
            Self::TechniqueRevision(value) => {
                push_version(&mut domain, &value.spec);
                domain.extend(
                    value
                        .parents
                        .iter()
                        .map(|value| value.parent.record.clone()),
                );
                push_metric_values(&mut domain, &value.parameters);
                if let Some(program) = &value.produced_by {
                    push_version(&mut domain, program);
                }
                if let Some(intent) = &value.execution_intent {
                    push_version(&mut domain, intent);
                }
            }
            Self::TechnicalProgram(value) => {
                push_holder(&mut core, &value.sponsor);
                core.push(value.site.clone());
                if let Some(value) = &value.revision {
                    push_version(&mut domain, value);
                }
            }
            Self::ExecutionIntent(value) => {
                push_holder(&mut core, &value.authorized_by);
                push_version(&mut domain, &value.program);
                push_intent_request_versions(&mut domain, &value.request);
                if let TechnologyIntentState::Consumed {
                    operation, result, ..
                } = &value.state
                {
                    push_version(&mut domain, operation);
                    push_version(&mut domain, result);
                }
            }
            Self::ExperimentAttempt(value) => {
                push_version(&mut domain, &value.execution_intent);
                push_version(&mut domain, &value.program);
                push_version(&mut domain, &value.revision);
                push_holder(&mut core, &value.operator);
                core.push(value.site.clone());
                push_metric_values(&mut domain, &value.inputs);
                push_metric_values(&mut domain, &value.environment);
                push_metric_values(&mut domain, &value.outputs);
                domain.extend(value.assets.iter().map(|value| value.record.clone()));
            }
            Self::AttemptObservation(value) => {
                push_version(&mut domain, &value.attempt);
                push_holder(&mut core, &value.observer);
                push_metric_values(&mut domain, &value.values);
            }
            Self::TechnicalClaim(value) => {
                push_holder(&mut core, &value.asserted_by);
                domain.extend(value.scope.iter().cloned());
                domain.extend(
                    value
                        .relations
                        .iter()
                        .map(|relation| relation.claim.record.clone()),
                );
            }
            Self::ClaimAssessment(value) => {
                push_version(&mut domain, &value.claim);
                push_holder(&mut core, &value.assessor);
            }
            Self::Capability(value) => {
                push_holder(&mut core, &value.holder);
                if let Some(operator) = &value.operator {
                    core.push(operator.clone());
                }
                core.push(value.site.clone());
                push_version(&mut domain, &value.revision);
                domain.extend(value.attempts.iter().map(|value| value.record.clone()));
            }
            Self::AssetBinding(value) => {
                push_holder(&mut core, &value.owner);
                core.push(value.site.clone());
            }
            Self::ProductionRun(value) => {
                push_version(&mut domain, &value.execution_intent);
                push_version(&mut domain, &value.revision);
                if let Some(value) = &value.application {
                    push_version(&mut domain, value);
                }
                push_holder(&mut core, &value.operator);
                core.push(value.site.clone());
                domain.extend(value.assets.iter().map(|value| value.record.clone()));
                push_metric_values(&mut domain, &value.inputs);
                push_metric_values(&mut domain, &value.outputs);
            }
            Self::Implementation(value) => {
                push_holder(&mut core, &value.owner);
                core.push(value.site.clone());
                push_version(&mut domain, &value.revision);
                push_version(&mut domain, &value.qualification);
                domain.extend(value.assets.iter().map(|value| value.record.clone()));
                if let Some(value) = &value.maintenance_provider {
                    push_holder(&mut core, value);
                }
            }
            Self::Adoption(value) => {
                push_holder(&mut core, &value.adopter);
                core.push(value.site.clone());
                push_version(&mut domain, &value.application);
                domain.extend(
                    value
                        .implementations
                        .iter()
                        .map(|value| value.record.clone()),
                );
                domain.extend(
                    value
                        .viability_evidence
                        .iter()
                        .map(|value| value.record.clone()),
                );
                push_metric_values(&mut domain, &value.viability_metrics);
            }
            Self::Transmission(value) => {
                if let Some(value) = &value.source {
                    push_holder(&mut core, value);
                }
                if let Some(value) = &value.source_site {
                    core.push(value.clone());
                }
                push_holder(&mut core, &value.destination);
                core.push(value.destination_site.clone());
                if let Some(value) = &value.revision {
                    push_version(&mut domain, value);
                }
                if let Some(value) = &value.source_capability {
                    push_version(&mut domain, value);
                }
                if let Some(value) = &value.resulting_program {
                    push_version(&mut domain, value);
                }
            }
        }
        core.into_iter()
            .map(|target| DomainReference {
                role: "core".to_owned(),
                target: DomainReferenceTarget::Core(target),
            })
            .chain(domain.into_iter().map(|target| DomainReference {
                role: "domain".to_owned(),
                target: DomainReferenceTarget::Domain(target),
            }))
            .collect()
    }

    #[must_use]
    pub(crate) fn authority_subject(&self) -> Option<&KnowledgeHolderRef> {
        match self {
            Self::Transmission(value) => value.source.as_ref().or(Some(&value.destination)),
            value => value.knowledge_holder(),
        }
    }

    #[must_use]
    pub(crate) fn exact_versions(&self) -> Vec<DomainRecordVersionRef> {
        let mut values = Vec::new();
        match self {
            Self::TechniqueRevision(value) => {
                values.push(value.spec.clone());
                values.extend(value.parents.iter().map(|item| item.parent.clone()));
                push_metric_versions(&mut values, &value.parameters);
                values.extend(value.produced_by.clone());
                values.extend(value.execution_intent.clone());
            }
            Self::TechnicalProgram(value) => values.extend(value.revision.clone()),
            Self::ExecutionIntent(value) => {
                values.push(value.program.clone());
                push_intent_request_exact_versions(&mut values, &value.request);
                if let TechnologyIntentState::Consumed {
                    operation, result, ..
                } = &value.state
                {
                    values.extend([operation.clone(), result.clone()]);
                }
            }
            Self::ExperimentAttempt(value) => {
                values.extend([
                    value.execution_intent.clone(),
                    value.program.clone(),
                    value.revision.clone(),
                ]);
                push_metric_versions(&mut values, &value.inputs);
                push_metric_versions(&mut values, &value.environment);
                push_metric_versions(&mut values, &value.outputs);
                values.extend(value.assets.iter().cloned());
            }
            Self::AttemptObservation(value) => {
                values.push(value.attempt.clone());
                push_metric_versions(&mut values, &value.values);
            }
            Self::TechnicalClaim(value) => {
                values.extend(value.relations.iter().map(|item| item.claim.clone()));
            }
            Self::ClaimAssessment(value) => values.push(value.claim.clone()),
            Self::Capability(value) => {
                values.push(value.revision.clone());
                values.extend(value.attempts.iter().cloned());
            }
            Self::AssetBinding(_) => {}
            Self::ProductionRun(value) => {
                values.push(value.execution_intent.clone());
                values.push(value.revision.clone());
                values.extend(value.application.clone());
                values.extend(value.assets.iter().cloned());
                push_metric_versions(&mut values, &value.inputs);
                push_metric_versions(&mut values, &value.outputs);
            }
            Self::Implementation(value) => {
                values.extend([value.revision.clone(), value.qualification.clone()]);
                values.extend(value.assets.iter().cloned());
            }
            Self::Adoption(value) => {
                values.push(value.application.clone());
                values.extend(value.implementations.iter().cloned());
                values.extend(value.viability_evidence.iter().cloned());
                push_metric_versions(&mut values, &value.viability_metrics);
            }
            Self::Transmission(value) => {
                values.extend(value.revision.clone());
                values.extend(value.source_capability.clone());
                values.extend(value.resulting_program.clone());
            }
        }
        values.sort();
        values.dedup();
        values
    }

    #[must_use]
    pub(crate) fn evidence_refs(&self) -> Vec<EvidenceRef> {
        let mut values = match self {
            Self::TechniqueRevision(value) => value.discovery_evidence.clone(),
            Self::ExecutionIntent(value) => match &value.state {
                TechnologyIntentState::Consumed { ingress, .. } => vec![ingress.clone()],
                TechnologyIntentState::Pending | TechnologyIntentState::Cancelled => Vec::new(),
            },
            Self::TechnicalProgram(value) => value
                .requirements
                .iter()
                .filter_map(|item| item.evidence.clone())
                .collect(),
            Self::TechnicalClaim(value) => value.source_evidence.clone(),
            Self::ClaimAssessment(value) => value
                .supporting_evidence
                .iter()
                .chain(&value.contradicting_evidence)
                .cloned()
                .collect(),
            Self::AssetBinding(value) => vec![value.provider_asset.clone()],
            Self::Adoption(value) => vec![value.decision_evidence.clone()],
            Self::Transmission(value) => value.evidence.clone(),
            Self::ExperimentAttempt(_)
            | Self::AttemptObservation(_)
            | Self::Capability(_)
            | Self::ProductionRun(_)
            | Self::Implementation(_) => Vec::new(),
        };
        values.sort();
        values.dedup();
        values
    }
}

fn push_intent_request_versions(
    values: &mut Vec<DomainRecordRef>,
    request: &TechnologyIntentRequest,
) {
    let mut exact = Vec::new();
    push_intent_request_exact_versions(&mut exact, request);
    values.extend(exact.into_iter().map(|value| value.record));
}

fn push_intent_request_exact_versions(
    values: &mut Vec<DomainRecordVersionRef>,
    request: &TechnologyIntentRequest,
) {
    match request {
        TechnologyIntentRequest::Experiment {
            revision,
            required_assets,
            ..
        }
        | TechnologyIntentRequest::Production {
            revision,
            required_assets,
            ..
        } => {
            values.push(revision.clone());
            values.extend(required_assets.iter().cloned());
            if let TechnologyIntentRequest::Production {
                application: Some(application),
                ..
            } = request
            {
                values.push(application.clone());
            }
        }
        TechnologyIntentRequest::Invention { spec, parent, .. } => {
            values.push(spec.clone());
            values.extend(parent.clone());
        }
    }
}

fn push_holder(values: &mut Vec<EntityRef>, holder: &KnowledgeHolderRef) {
    match holder {
        KnowledgeHolderRef::Person(person) => values.push(EntityRef::Person(*person)),
        KnowledgeHolderRef::Entity(entity) => values.push(entity.clone()),
    }
}

fn push_version(values: &mut Vec<DomainRecordRef>, version: &DomainRecordVersionRef) {
    values.push(version.record.clone());
}

fn push_metric_values(values: &mut Vec<DomainRecordRef>, metrics: &[MetricValue]) {
    values.extend(metrics.iter().map(|value| value.metric.record.clone()));
}

fn push_metric_versions(values: &mut Vec<DomainRecordVersionRef>, metrics: &[MetricValue]) {
    values.extend(metrics.iter().map(|value| value.metric.clone()));
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum TechnologyRecordChange {
    Create {
        id: String,
        value: TechnologyRecordPayload,
    },
    Update {
        id: String,
        expected_version: u64,
        value: TechnologyRecordPayload,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TechnologyCommandEnvelope {
    pub id: String,
    pub subject: KnowledgeHolderRef,
    pub change: TechnologyRecordChange,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TechnologyResultEnvelope {
    pub id: String,
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_intent: Option<DomainRecordVersionRef>,
    pub change: TechnologyRecordChange,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct MetricContext {
    pub values: BTreeMap<DomainRecordRef, i64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum TechnologyCatalogRecord {
    Metric(MetricSchemaPayload),
    Technique(TechniqueSpecPayload),
    Revision(TechniqueRevisionPayload),
    Application(ApplicationSpecPayload),
}

impl TechnologyCatalogRecord {
    pub fn into_initial_record(
        self,
        id: impl Into<String>,
    ) -> Result<DomainRecord, canwu_api::CanwuError> {
        let id = id.into();
        let (reference, mut payload, mut references) = match self {
            Self::Metric(payload) => (
                TypedDomainRecordRef::<MetricSchema>::new(id).into_untyped(),
                serde_json::to_value(payload).map_err(|error| catalog_encoding_error(&error))?,
                Vec::new(),
            ),
            Self::Technique(payload) => {
                let mut references = Vec::new();
                for group in &payload.requirements {
                    references.extend(group.any_of.iter().map(|threshold| DomainReference {
                        role: "domain".to_owned(),
                        target: DomainReferenceTarget::Domain(threshold.metric.record.clone()),
                    }));
                }
                (
                    TypedDomainRecordRef::<TechniqueSpec>::new(id).into_untyped(),
                    serde_json::to_value(payload)
                        .map_err(|error| catalog_encoding_error(&error))?,
                    references,
                )
            }
            Self::Revision(payload) => {
                let mut references = vec![DomainReference {
                    role: "domain".to_owned(),
                    target: DomainReferenceTarget::Domain(payload.spec.record.clone()),
                }];
                references.extend(payload.parents.iter().map(|parent| DomainReference {
                    role: "domain".to_owned(),
                    target: DomainReferenceTarget::Domain(parent.parent.record.clone()),
                }));
                references.extend(payload.parameters.iter().map(|parameter| DomainReference {
                    role: "domain".to_owned(),
                    target: DomainReferenceTarget::Domain(parameter.metric.record.clone()),
                }));
                (
                    TypedDomainRecordRef::<TechniqueRevision>::new(id).into_untyped(),
                    serde_json::to_value(payload)
                        .map_err(|error| catalog_encoding_error(&error))?,
                    references,
                )
            }
            Self::Application(payload) => {
                let mut references = vec![DomainReference {
                    role: "domain".to_owned(),
                    target: DomainReferenceTarget::Domain(payload.technique.record.clone()),
                }];
                for group in &payload.viability {
                    references.extend(group.any_of.iter().map(|threshold| DomainReference {
                        role: "domain".to_owned(),
                        target: DomainReferenceTarget::Domain(threshold.metric.record.clone()),
                    }));
                }
                (
                    TypedDomainRecordRef::<ApplicationSpec>::new(id).into_untyped(),
                    serde_json::to_value(payload)
                        .map_err(|error| catalog_encoding_error(&error))?,
                    references,
                )
            }
        };
        attach_payload_continuation(&mut payload, Vec::new())?;
        references.sort();
        references.dedup();
        Ok(DomainRecord {
            reference,
            owner: crate::PLUGIN_NAME.to_owned(),
            class: DomainRecordClass::Record,
            version: 1,
            lifecycle: DomainRecordLifecycle::Active,
            payload,
            references,
        })
    }
}

pub(crate) fn attach_payload_continuation(
    payload: &mut serde_json::Value,
    references: impl IntoIterator<Item = DomainRecordVersionRef>,
) -> Result<(), canwu_api::CanwuError> {
    let dependencies = references
        .into_iter()
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
    let object = payload.as_object_mut().ok_or_else(|| {
        canwu_api::CanwuError::new(
            canwu_api::ErrorCode::InvalidDomainRecord,
            "technology record payload must be an object",
        )
    })?;
    object.insert(
        PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD.to_owned(),
        serde_json::to_value(continuation).map_err(|error| {
            canwu_api::CanwuError::new(
                canwu_api::ErrorCode::InvalidDomainRecord,
                format!("technology continuation could not be encoded: {error}"),
            )
        })?,
    );
    Ok(())
}

#[must_use]
pub fn initial_record_version<T: DomainRecordType>(
    id: impl Into<String>,
) -> DomainRecordVersionRef {
    DomainRecordVersionRef {
        record: TypedDomainRecordRef::<T>::new(id).into_untyped(),
        version: 1,
        established_by: DomainRecordVersionSource::InitialScenario,
    }
}

fn catalog_encoding_error(error: &serde_json::Error) -> canwu_api::CanwuError {
    canwu_api::CanwuError::new(
        canwu_api::ErrorCode::InvalidDomainRecord,
        format!("technology catalog record could not be encoded: {error}"),
    )
}
