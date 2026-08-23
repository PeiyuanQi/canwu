use crate::PLUGIN_NAMESPACE;
use crate::model::{
    AdoptionRecord, ApplicationSpec, AssetBinding, AttemptObservation, CapabilityQualification,
    ClaimAssessment, ExperimentAttempt, ImplementationRecord, MetricSchema, ProductionRun,
    TechnicalClaim, TechnicalProgram, TechniqueRevision, TechniqueSpec, TechnologyExecutionIntent,
    TechnologyOperation, TransmissionOpportunity,
};
use canwu_api::{
    DomainRecordMutationPolicy, DomainRecordSchema, DomainRecordType, DomainReferenceSchema,
    DomainReferenceTargetKind, KnowledgeRecordKind, KnowledgeSchemaId, KnowledgeSubjectSchema,
    KnowledgeSubjectTargetKind, PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD, PayloadProperty,
    PayloadSchema, PayloadValueType, PluginKnowledgeSchema,
    payload_required_evidence_continuation_property_v1,
};
use std::collections::BTreeMap;

pub const CLAIM_KNOWLEDGE: &str = "claim_awareness";
pub const ATTEMPT_KNOWLEDGE: &str = "attempt_observation";
pub const CAPABILITY_KNOWLEDGE: &str = "qualified_practice";
pub const IMPLEMENTATION_KNOWLEDGE: &str = "implementation_observation";
pub const ADOPTION_KNOWLEDGE: &str = "adoption_assessment";

#[must_use]
pub fn technology_record_schemas() -> Vec<DomainRecordSchema> {
    let mut schemas = vec![
        schema::<MetricSchema>(DomainRecordMutationPolicy::CreateOnly),
        schema::<TechniqueSpec>(DomainRecordMutationPolicy::CreateOnly),
        schema::<TechniqueRevision>(DomainRecordMutationPolicy::CreateOnly),
        schema::<ApplicationSpec>(DomainRecordMutationPolicy::CreateOnly),
        schema::<TechnicalProgram>(DomainRecordMutationPolicy::Versioned),
        schema::<TechnologyExecutionIntent>(DomainRecordMutationPolicy::Versioned),
        schema::<ExperimentAttempt>(DomainRecordMutationPolicy::CreateOnly),
        schema::<AttemptObservation>(DomainRecordMutationPolicy::CreateOnly),
        schema::<TechnicalClaim>(DomainRecordMutationPolicy::CreateOnly),
        schema::<ClaimAssessment>(DomainRecordMutationPolicy::CreateOnly),
        schema::<CapabilityQualification>(DomainRecordMutationPolicy::Versioned),
        schema::<AssetBinding>(DomainRecordMutationPolicy::Versioned),
        schema::<ProductionRun>(DomainRecordMutationPolicy::CreateOnly),
        schema::<ImplementationRecord>(DomainRecordMutationPolicy::Versioned),
        schema::<AdoptionRecord>(DomainRecordMutationPolicy::Versioned),
        schema::<TransmissionOpportunity>(DomainRecordMutationPolicy::Versioned),
        schema::<TechnologyOperation>(DomainRecordMutationPolicy::CreateOnly),
    ];
    schemas.sort_by(|left, right| left.kind.cmp(&right.kind));
    schemas
}

fn schema<T: DomainRecordType>(mutation_policy: DomainRecordMutationPolicy) -> DomainRecordSchema {
    let mut schema = DomainRecordSchema::for_type::<T>();
    schema.mutation_policy = mutation_policy;
    schema.payload_schema = PayloadSchema::Object {
        properties: BTreeMap::from([(
            PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD.to_owned(),
            payload_required_evidence_continuation_property_v1(),
        )]),
        allow_additional: true,
    };
    schema.references = vec![
        DomainReferenceSchema {
            role: "core".to_owned(),
            targets: vec![DomainReferenceTargetKind::AnyEntity],
            required: false,
            multiple: true,
            allow_retired: false,
        },
        DomainReferenceSchema {
            role: "domain".to_owned(),
            targets: technology_record_kinds()
                .into_iter()
                .map(DomainReferenceTargetKind::Domain)
                .collect(),
            required: false,
            multiple: true,
            allow_retired: true,
        },
    ];
    schema
}

#[must_use]
pub fn technology_record_kinds() -> Vec<canwu_api::DomainRecordKind> {
    let mut kinds = vec![
        kind::<MetricSchema>(),
        kind::<TechniqueSpec>(),
        kind::<TechniqueRevision>(),
        kind::<ApplicationSpec>(),
        kind::<TechnicalProgram>(),
        kind::<TechnologyExecutionIntent>(),
        kind::<ExperimentAttempt>(),
        kind::<AttemptObservation>(),
        kind::<TechnicalClaim>(),
        kind::<ClaimAssessment>(),
        kind::<CapabilityQualification>(),
        kind::<AssetBinding>(),
        kind::<ProductionRun>(),
        kind::<ImplementationRecord>(),
        kind::<AdoptionRecord>(),
        kind::<TransmissionOpportunity>(),
        kind::<TechnologyOperation>(),
    ];
    kinds.sort();
    kinds
}

fn kind<T: DomainRecordType>() -> canwu_api::DomainRecordKind {
    canwu_api::DomainRecordKind::for_type::<T>()
}

#[must_use]
pub fn technology_knowledge_schemas() -> Vec<PluginKnowledgeSchema> {
    let mut schemas = vec![
        knowledge_schema::<TechnicalClaim>(
            CLAIM_KNOWLEDGE,
            "3e80f6f692bd9a2ad99e7a115c78ffe17fb729c642d6a9e1eb8f7a67566c01aa",
        ),
        knowledge_schema::<AttemptObservation>(
            ATTEMPT_KNOWLEDGE,
            "d89ff4f6b6a7252df5318a8e7bdc7efebbea84085f003c3d8191568366e32a71",
        ),
        knowledge_schema::<CapabilityQualification>(
            CAPABILITY_KNOWLEDGE,
            "eec9ea3aacf5687a4e8197d979e98fa50e1edb727d895417a9a487e15faa1e59",
        ),
        knowledge_schema::<ImplementationRecord>(
            IMPLEMENTATION_KNOWLEDGE,
            "f2519ef2e958f11e6d00bf9869ab2849fb69eae32eab3d3761d25907abb65e93",
        ),
        knowledge_schema::<AdoptionRecord>(
            ADOPTION_KNOWLEDGE,
            "c6eb7a04a18d67ee53db4288f230d60613255c039d3923211d03d1f6be82a8ad",
        ),
    ];
    schemas.sort_by(|left, right| left.id.cmp(&right.id));
    schemas
}

fn knowledge_schema<T: DomainRecordType>(name: &str, hash: &str) -> PluginKnowledgeSchema {
    PluginKnowledgeSchema {
        id: KnowledgeSchemaId::new(KnowledgeRecordKind::new(PLUGIN_NAMESPACE, name), 1),
        schema_hash: hash.to_owned(),
        writable: true,
        payload_schema: PayloadSchema::Object {
            properties: BTreeMap::from([
                (
                    "record_version".to_owned(),
                    PayloadProperty {
                        value_type: PayloadValueType::Integer,
                        required: true,
                    },
                ),
                (
                    "record".to_owned(),
                    PayloadProperty {
                        value_type: PayloadValueType::Object,
                        required: true,
                    },
                ),
            ]),
            allow_additional: false,
        },
        subjects: vec![KnowledgeSubjectSchema {
            role: "record".to_owned(),
            targets: vec![KnowledgeSubjectTargetKind::Domain(
                canwu_api::DomainRecordKind::for_type::<T>(),
            )],
            required: true,
            multiple: false,
        }],
    }
}
