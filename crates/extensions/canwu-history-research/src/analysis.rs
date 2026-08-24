use crate::model::{
    AssessmentCore, AssessmentRecord, HistoricalPracticeAssessment, HistoricalSourcesAssessment,
    ProductionArchaeologyAssessment, validate_assessment,
};
use canwu_api::{
    Canwu, CanwuError, DomainRecord, DomainRecordRef, DomainRecordVersionRef, ErrorCode,
    EvidenceRef,
};

#[derive(Clone, Debug, PartialEq)]
pub struct HistoricalAssessmentView {
    pub record: DomainRecord,
    pub subject: DomainRecordVersionRef,
    pub uncertainty_per_mille: u16,
}

pub struct HistoricalAnalysis;

impl HistoricalAnalysis {
    pub fn for_subject(
        canwu: &Canwu,
        subject: &DomainRecordRef,
    ) -> Result<Vec<HistoricalAssessmentView>, CanwuError> {
        let mut values = Vec::new();
        values.extend(load::<HistoricalSourcesAssessment>(
            canwu,
            subject,
            |value| value.core.uncertainty_per_mille,
        )?);
        values.extend(load::<HistoricalPracticeAssessment>(
            canwu,
            subject,
            |value| value.core.uncertainty_per_mille,
        )?);
        values.extend(load::<ProductionArchaeologyAssessment>(
            canwu,
            subject,
            |value| value.core.uncertainty_per_mille,
        )?);
        values.sort_by(|left, right| left.record.reference.cmp(&right.record.reference));
        Ok(values)
    }
}

pub fn validate_historical_research_runtime(canwu: &Canwu) -> Result<(), CanwuError> {
    validate_records::<HistoricalSourcesAssessment>(canwu)?;
    validate_records::<HistoricalPracticeAssessment>(canwu)?;
    validate_records::<ProductionArchaeologyAssessment>(canwu)
}

pub fn from_historical_research_snapshot_json(
    json: &str,
    plugins: &[&dyn canwu_api::SimulationPlugin],
) -> Result<Canwu, CanwuError> {
    let canwu = Canwu::from_snapshot_json_with_plugins(json, plugins)?;
    validate_historical_research_runtime(&canwu)?;
    Ok(canwu)
}

pub fn from_historical_research_checkpoint_journal(
    bundle: canwu_api::CheckpointJournal,
    plugins: &[&dyn canwu_api::SimulationPlugin],
) -> Result<Canwu, CanwuError> {
    let canwu = Canwu::from_checkpoint_journal_with_plugins(bundle, plugins)?;
    validate_historical_research_runtime(&canwu)?;
    Ok(canwu)
}

pub fn replay_historical_research_from_journal(
    plugins: &[&dyn canwu_api::SimulationPlugin],
    journal: &canwu_api::ReplayJournal,
) -> Result<Canwu, CanwuError> {
    let canwu = Canwu::replay_from_journal(plugins, journal)?;
    validate_historical_research_runtime(&canwu)?;
    Ok(canwu)
}

fn validate_records<T: AssessmentRecord>(canwu: &Canwu) -> Result<(), CanwuError>
where
    T::Payload: Clone + serde::de::DeserializeOwned + serde::Serialize,
{
    let kind = canwu_api::DomainRecordKind::for_type::<T>();
    let revision = canwu.revision();
    let mut after = None;
    let mut count = 0usize;
    loop {
        let page = canwu.domain_record_page(&kind, after.as_ref(), 256, Some(revision))?;
        let has_more = page.next.is_some();
        after = page.next;
        count = count.checked_add(page.records.len()).ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidDomainRecord,
                "historical assessment count overflowed",
            )
        })?;
        if count > 1_000 {
            return Err(CanwuError::new(
                ErrorCode::InvalidDomainRecord,
                "historical assessment plugin exceeds its 1,000-record limit",
            ));
        }
        for record in page.records {
            let payload = record.decode_payload::<T>()?;
            validate_assessment::<T>(&payload)
                .map_err(|message| CanwuError::new(ErrorCode::InvalidDomainRecord, message))?;
            let core = T::core(&payload);
            if core.as_of > canwu.time() || !assessment_evidence_is_valid(canwu, core)? {
                return Err(CanwuError::new(
                    ErrorCode::InvalidDomainRecord,
                    "historical assessment has an invalid date or exact evidence reference",
                ));
            }
        }
        if !has_more {
            break;
        }
    }
    Ok(())
}

fn assessment_evidence_is_valid(canwu: &Canwu, core: &AssessmentCore) -> Result<bool, CanwuError> {
    for reference in std::iter::once(&core.subject)
        .chain(&core.contradicts)
        .chain(&core.supersedes)
    {
        if !canwu.domain_record_version_evidence_exists(reference)
            || canwu
                .evidence_time(&EvidenceRef::DomainRecordVersion(reference.clone()))
                .is_none_or(|at| at > core.as_of)
        {
            return Ok(false);
        }
    }
    for citation in &core.citations {
        if !canwu.evidence_exists(citation)
            || canwu
                .evidence_time(citation)
                .is_none_or(|at| at > core.as_of)
        {
            return Ok(false);
        }
    }
    for relation in core.contradicts.iter().chain(&core.supersedes) {
        let Some(record) = canwu.domain_record_version(relation) else {
            return Ok(false);
        };
        let related_subject = if record
            .reference
            .kind
            .matches_type::<HistoricalSourcesAssessment>()
        {
            record
                .decode_payload::<HistoricalSourcesAssessment>()?
                .core
                .subject
        } else if record
            .reference
            .kind
            .matches_type::<HistoricalPracticeAssessment>()
        {
            record
                .decode_payload::<HistoricalPracticeAssessment>()?
                .core
                .subject
        } else if record
            .reference
            .kind
            .matches_type::<ProductionArchaeologyAssessment>()
        {
            record
                .decode_payload::<ProductionArchaeologyAssessment>()?
                .core
                .subject
        } else {
            return Ok(false);
        };
        if related_subject != core.subject {
            return Ok(false);
        }
    }
    Ok(true)
}

fn load<T: AssessmentRecord>(
    canwu: &Canwu,
    subject: &DomainRecordRef,
    uncertainty: impl Fn(&T::Payload) -> u16,
) -> Result<Vec<HistoricalAssessmentView>, CanwuError>
where
    T::Payload: Clone + serde::de::DeserializeOwned + serde::Serialize,
{
    let kind = canwu_api::DomainRecordKind::for_type::<T>();
    let mut after = None;
    let revision = canwu.revision();
    let mut values = Vec::new();
    loop {
        let page = canwu.domain_record_page(&kind, after.as_ref(), 256, Some(revision))?;
        let has_more = page.next.is_some();
        after = page.next;
        for record in page.records {
            let payload = record.decode_payload::<T>()?;
            if T::core(&payload).subject.record == *subject {
                values.push(HistoricalAssessmentView {
                    record,
                    subject: T::core(&payload).subject.clone(),
                    uncertainty_per_mille: uncertainty(&payload),
                });
            }
        }
        if !has_more {
            break;
        }
    }
    Ok(values)
}
