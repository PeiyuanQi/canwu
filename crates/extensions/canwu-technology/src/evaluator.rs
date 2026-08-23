use crate::model::{
    ApplicationSpecPayload, EvaluationResult, MetricComparison, MetricContext, MetricSchemaPayload,
    REFERENCE_EVALUATOR_V1, RequirementGroup, TechniqueRevisionPayload, TechniqueSpecPayload,
};
use canwu_api::{CanwuError, ErrorCode};
use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvaluationError(pub String);

impl Display for EvaluationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for EvaluationError {}

impl From<EvaluationError> for CanwuError {
    fn from(value: EvaluationError) -> Self {
        Self::new(ErrorCode::InvalidDomainRecord, value.0)
    }
}

pub fn evaluate_attempt(
    revision: &TechniqueRevisionPayload,
    spec: &TechniqueSpecPayload,
    metrics: &BTreeMap<canwu_api::DomainRecordRef, MetricSchemaPayload>,
    context: &MetricContext,
) -> Result<EvaluationResult, EvaluationError> {
    let mut combined = context.clone();
    for parameter in &revision.parameters {
        if let Some(actual) = combined.values.get(&parameter.metric.record)
            && *actual != parameter.value
        {
            return Err(EvaluationError(format!(
                "attempt value for {} conflicts with its exact revision parameter",
                parameter.metric.record
            )));
        }
        combined
            .values
            .insert(parameter.metric.record.clone(), parameter.value);
    }
    evaluate_groups(&spec.requirements, metrics, &combined)
}

pub fn evaluate_application(
    application: &ApplicationSpecPayload,
    metrics: &BTreeMap<canwu_api::DomainRecordRef, MetricSchemaPayload>,
    context: &MetricContext,
) -> Result<EvaluationResult, EvaluationError> {
    evaluate_groups(&application.viability, metrics, context)
}

fn evaluate_groups(
    groups: &[RequirementGroup],
    metrics: &BTreeMap<canwu_api::DomainRecordRef, MetricSchemaPayload>,
    context: &MetricContext,
) -> Result<EvaluationResult, EvaluationError> {
    let mut satisfied_groups = Vec::new();
    let mut failed_groups = Vec::new();
    for group in groups {
        if group.any_of.is_empty() {
            return Err(EvaluationError(format!(
                "requirement group {} has no alternatives",
                group.id
            )));
        }
        let mut satisfied = false;
        for threshold in &group.any_of {
            let schema = metrics.get(&threshold.metric.record).ok_or_else(|| {
                EvaluationError(format!("metric {} is unavailable", threshold.metric.record))
            })?;
            if threshold.metric.version == 0
                || threshold.value < schema.minimum
                || threshold.value > schema.maximum
            {
                return Err(EvaluationError(format!(
                    "threshold {} is outside its metric schema",
                    threshold.id
                )));
            }
            let Some(value) = context.values.get(&threshold.metric.record) else {
                continue;
            };
            if *value < schema.minimum || *value > schema.maximum {
                return Err(EvaluationError(format!(
                    "metric value for {} is outside its schema",
                    threshold.metric.record
                )));
            }
            satisfied |= match threshold.comparison {
                MetricComparison::AtLeast => *value >= threshold.value,
                MetricComparison::AtMost => *value <= threshold.value,
            };
        }
        if satisfied {
            satisfied_groups.push(group.id.clone());
        } else {
            failed_groups.push(group.id.clone());
        }
    }
    Ok(EvaluationResult {
        evaluator: REFERENCE_EVALUATOR_V1.to_owned(),
        passed: failed_groups.is_empty(),
        satisfied_groups,
        failed_groups,
    })
}
