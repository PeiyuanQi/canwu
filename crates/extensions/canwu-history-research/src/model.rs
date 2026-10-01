use canwu_api::{
    DomainRecordType, DomainRecordVersionRef, DomainValueKindClass, EntityRef, EvidenceRef,
    KnowledgeHolderRef, SimTime,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AssessmentCore {
    pub assessor: KnowledgeHolderRef,
    pub subject: DomainRecordVersionRef,
    pub method: String,
    pub method_version: String,
    pub as_of: SimTime,
    pub uncertainty_per_mille: u16,
    pub summary_digest: String,
    pub citations: Vec<EvidenceRef>,
    pub contradicts: Vec<DomainRecordVersionRef>,
    pub supersedes: Vec<DomainRecordVersionRef>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HistoricalSourcesAssessmentPayload {
    pub core: AssessmentCore,
    pub earliest_date: SimTime,
    pub latest_date: SimTime,
    pub authenticity_per_mille: u16,
    pub reliability_per_mille: u16,
    pub provenance_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HistoricalPracticeAssessmentPayload {
    pub core: AssessmentCore,
    pub participants: Vec<EntityRef>,
    pub relation: String,
    pub notebook_digest: Option<String>,
    pub negative_result: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProductionArchaeologyAssessmentPayload {
    pub core: AssessmentCore,
    pub observed_kind: String,
    pub observed_digest: String,
    pub inferred_process_digest: String,
    pub earliest_date: SimTime,
    pub latest_date: SimTime,
}

macro_rules! assessment_type {
    ($name:ident, $namespace:literal, $payload:ty) => {
        #[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name;

        impl DomainRecordType for $name {
            type Payload = $payload;
            type Class = DomainValueKindClass;

            const NAMESPACE: &'static str = $namespace;
            const NAME: &'static str = "assessment";
        }
    };
}

assessment_type!(
    HistoricalSourcesAssessment,
    "canwu.history.sources",
    HistoricalSourcesAssessmentPayload
);
assessment_type!(
    HistoricalPracticeAssessment,
    "canwu.history.practice",
    HistoricalPracticeAssessmentPayload
);
assessment_type!(
    ProductionArchaeologyAssessment,
    "canwu.history.production_archaeology",
    ProductionArchaeologyAssessmentPayload
);

pub trait AssessmentRecord: DomainRecordType + Default + Send + Sync + 'static
where
    Self::Payload: Clone + serde::de::DeserializeOwned + Serialize,
{
    const PLUGIN_NAME: &'static str;
    const PLUGIN_VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const SEMANTIC_HASH: &'static str;

    fn core(payload: &Self::Payload) -> &AssessmentCore;
    fn validate_specific(payload: &Self::Payload) -> Result<(), String>;
    fn core_entities(payload: &Self::Payload) -> Vec<EntityRef>;
}

impl AssessmentRecord for HistoricalSourcesAssessment {
    const PLUGIN_NAME: &'static str = "canwu-history-sources";
    const SEMANTIC_HASH: &'static str =
        "35a877e9577e8718d4e8b2b0db715b55b5172324e7bc93bab98143b449bec517";

    fn core(payload: &Self::Payload) -> &AssessmentCore {
        &payload.core
    }

    fn validate_specific(payload: &Self::Payload) -> Result<(), String> {
        if payload.earliest_date > payload.latest_date
            || payload.latest_date > payload.core.as_of
            || payload.authenticity_per_mille > 1_000
            || payload.reliability_per_mille > 1_000
        {
            return Err("source assessment has invalid range or confidence".to_owned());
        }
        validate_digest(&payload.provenance_digest, "source provenance")
    }

    fn core_entities(_payload: &Self::Payload) -> Vec<EntityRef> {
        Vec::new()
    }
}

impl AssessmentRecord for HistoricalPracticeAssessment {
    const PLUGIN_NAME: &'static str = "canwu-history-practice";
    const SEMANTIC_HASH: &'static str =
        "8d5eb8fd697810aa0c440eabd4d7a1f8301b4a7a24e5a55b735c0f48b83cf148";

    fn core(payload: &Self::Payload) -> &AssessmentCore {
        &payload.core
    }

    fn validate_specific(payload: &Self::Payload) -> Result<(), String> {
        if payload.participants.len() > 32 {
            return Err("practice assessment has too many participants".to_owned());
        }
        validate_text(&payload.relation, "practice relation")?;
        if let Some(digest) = &payload.notebook_digest {
            validate_digest(digest, "notebook")?;
        }
        Ok(())
    }

    fn core_entities(payload: &Self::Payload) -> Vec<EntityRef> {
        payload.participants.clone()
    }
}

impl AssessmentRecord for ProductionArchaeologyAssessment {
    const PLUGIN_NAME: &'static str = "canwu-history-production-archaeology";
    const SEMANTIC_HASH: &'static str =
        "779d9b281302005416a09b66cdc8694291f940eed8b035d11c79ee67c8955e67";

    fn core(payload: &Self::Payload) -> &AssessmentCore {
        &payload.core
    }

    fn validate_specific(payload: &Self::Payload) -> Result<(), String> {
        if payload.earliest_date > payload.latest_date || payload.latest_date > payload.core.as_of {
            return Err("production archaeology assessment has an invalid date range".to_owned());
        }
        validate_text(&payload.observed_kind, "observed kind")?;
        validate_digest(&payload.observed_digest, "observed material")?;
        validate_digest(&payload.inferred_process_digest, "inferred process")
    }

    fn core_entities(_payload: &Self::Payload) -> Vec<EntityRef> {
        Vec::new()
    }
}

pub fn validate_assessment<T: AssessmentRecord>(payload: &T::Payload) -> Result<(), String>
where
    T::Payload: Clone + serde::de::DeserializeOwned + Serialize,
{
    let core = T::core(payload);
    validate_text(&core.method, "assessment method")?;
    validate_text(&core.method_version, "assessment method version")?;
    validate_digest(&core.summary_digest, "assessment summary")?;
    if core.uncertainty_per_mille > 1_000
        || core.citations.len() > 32
        || core.contradicts.len() > 32
        || core.supersedes.len() > 32
    {
        return Err("historical assessment exceeds canonical bounds".to_owned());
    }
    let encoded = serde_json::to_vec(payload).map_err(|error| error.to_string())?;
    if encoded.len() > 16 * 1024 {
        return Err("historical assessment exceeds 16 KiB".to_owned());
    }
    T::validate_specific(payload)
}

fn validate_text(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        return Err(format!("{label} is not bounded canonical text"));
    }
    Ok(())
}

fn validate_digest(value: &str, label: &str) -> Result<(), String> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!(
            "{label} digest is not a 64-character hexadecimal digest"
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HistoricalAssessmentCommand<T> {
    pub id: String,
    pub subject: KnowledgeHolderRef,
    pub assessment: T,
}
