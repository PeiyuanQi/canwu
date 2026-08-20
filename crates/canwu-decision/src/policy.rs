use crate::{
    DecisionError, DecisionErrorCode, DecisionExternalEvidence, DecisionFactorContribution,
    DecisionOption, DecisionOptionEvaluation, DecisionOutcome, DecisionPolicyIdentity,
    DecisionPolicyKind, DecisionTicket, PolicyDecision,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub trait DecisionPolicy {
    fn identity(&self) -> DecisionPolicyIdentity;
    fn decide(&self, ticket: &DecisionTicket) -> Result<PolicyDecision, DecisionError>;
}

pub trait UtilityEvaluator {
    fn evaluate(
        &self,
        ticket: &DecisionTicket,
        option: &DecisionOption,
    ) -> Result<DecisionOptionEvaluation, DecisionError>;
}

pub trait UtilityPolicy: DecisionPolicy + UtilityEvaluator {}

impl<T: DecisionPolicy + UtilityEvaluator> UtilityPolicy for T {}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct UtilityProfile {
    pub weights: BTreeMap<String, i64>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct WeightedUtilityEvaluator {
    pub profile: UtilityProfile,
}

impl WeightedUtilityEvaluator {
    #[must_use]
    pub const fn new(profile: UtilityProfile) -> Self {
        Self { profile }
    }
}

impl UtilityEvaluator for WeightedUtilityEvaluator {
    fn evaluate(
        &self,
        _ticket: &DecisionTicket,
        option: &DecisionOption,
    ) -> Result<DecisionOptionEvaluation, DecisionError> {
        if !option.is_available() {
            return Ok(DecisionOptionEvaluation {
                option_id: option.id.clone(),
                available: false,
                score: None,
                factors: Vec::new(),
                blockers: option.blockers.clone(),
            });
        }
        let mut score = 0_i64;
        let mut factors = Vec::new();
        for (factor, value) in &option.utility_inputs {
            let weight = self
                .profile
                .weights
                .get(factor)
                .copied()
                .unwrap_or_default();
            let contribution = value.checked_mul(weight).ok_or_else(|| {
                DecisionError::new(
                    DecisionErrorCode::InvalidDecision,
                    format!("utility contribution for factor {factor} exceeds the i64 range"),
                )
            })?;
            score = score.checked_add(contribution).ok_or_else(|| {
                DecisionError::new(
                    DecisionErrorCode::InvalidDecision,
                    "utility score exceeds the i64 range",
                )
            })?;
            factors.push(DecisionFactorContribution {
                factor: factor.clone(),
                value: *value,
                weight,
                contribution,
            });
        }
        Ok(DecisionOptionEvaluation {
            option_id: option.id.clone(),
            available: true,
            score: Some(score),
            factors,
            blockers: Vec::new(),
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WeightedUtilityPolicy {
    pub identity: DecisionPolicyIdentity,
    pub evaluator: WeightedUtilityEvaluator,
}

impl WeightedUtilityPolicy {
    #[must_use]
    pub fn new(id: impl Into<String>, version: impl Into<String>, profile: UtilityProfile) -> Self {
        Self {
            identity: DecisionPolicyIdentity::new(DecisionPolicyKind::Utility, id, version),
            evaluator: WeightedUtilityEvaluator::new(profile),
        }
    }
}

impl UtilityEvaluator for WeightedUtilityPolicy {
    fn evaluate(
        &self,
        ticket: &DecisionTicket,
        option: &DecisionOption,
    ) -> Result<DecisionOptionEvaluation, DecisionError> {
        self.evaluator.evaluate(ticket, option)
    }
}

impl DecisionPolicy for WeightedUtilityPolicy {
    fn identity(&self) -> DecisionPolicyIdentity {
        self.identity.clone()
    }

    fn decide(&self, ticket: &DecisionTicket) -> Result<PolicyDecision, DecisionError> {
        let mut evaluations = ticket
            .options
            .iter()
            .map(|option| self.evaluate(ticket, option))
            .collect::<Result<Vec<_>, _>>()?;
        evaluations.sort_by(|left, right| left.option_id.cmp(&right.option_id));
        let selected = evaluations
            .iter()
            .filter_map(|evaluation| evaluation.score.map(|score| (score, &evaluation.option_id)))
            .max_by(|left, right| left.0.cmp(&right.0).then_with(|| right.1.cmp(left.1)))
            .map(|(_, option_id)| option_id.clone());
        let Some(option_id) = selected else {
            return Ok(PolicyDecision {
                outcome: DecisionOutcome::Deferred {
                    reason: "no available option".to_owned(),
                },
                summary: "utility policy deferred because every option was blocked".to_owned(),
                evaluations,
                external: None,
            });
        };
        Ok(PolicyDecision {
            outcome: DecisionOutcome::Selected {
                option_id: option_id.clone(),
            },
            summary: format!("utility policy selected {option_id}"),
            evaluations,
            external: None,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuleChoice {
    Select(String),
    Defer(String),
    NoMatch,
}

pub trait DecisionRule {
    fn id(&self) -> &str;
    fn evaluate(&self, ticket: &DecisionTicket) -> Result<RuleChoice, DecisionError>;
}

pub trait RulePolicy: DecisionPolicy {
    fn rules(&self) -> &[Box<dyn DecisionRule>];
}

pub struct OrderedRulePolicy {
    identity: DecisionPolicyIdentity,
    rules: Vec<Box<dyn DecisionRule>>,
}

impl OrderedRulePolicy {
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        version: impl Into<String>,
        rules: Vec<Box<dyn DecisionRule>>,
    ) -> Self {
        Self {
            identity: DecisionPolicyIdentity::new(DecisionPolicyKind::Rule, id, version),
            rules,
        }
    }
}

impl RulePolicy for OrderedRulePolicy {
    fn rules(&self) -> &[Box<dyn DecisionRule>] {
        &self.rules
    }
}

impl DecisionPolicy for OrderedRulePolicy {
    fn identity(&self) -> DecisionPolicyIdentity {
        self.identity.clone()
    }

    fn decide(&self, ticket: &DecisionTicket) -> Result<PolicyDecision, DecisionError> {
        for rule in &self.rules {
            match rule.evaluate(ticket)? {
                RuleChoice::Select(option_id) => {
                    return Ok(PolicyDecision::selected(
                        option_id,
                        format!("rule {} selected an option", rule.id()),
                    ));
                }
                RuleChoice::Defer(reason) => {
                    return Ok(PolicyDecision {
                        outcome: DecisionOutcome::Deferred {
                            reason: reason.clone(),
                        },
                        summary: format!("rule {} deferred: {reason}", rule.id()),
                        evaluations: Vec::new(),
                        external: None,
                    });
                }
                RuleChoice::NoMatch => {}
            }
        }
        Ok(PolicyDecision {
            outcome: DecisionOutcome::Deferred {
                reason: "no rule matched".to_owned(),
            },
            summary: "ordered rule policy exhausted its rules".to_owned(),
            evaluations: Vec::new(),
            external: None,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HumanDecisionResponse {
    pub ticket_version: u64,
    pub option_id: String,
    pub operator_id: String,
}

pub trait HumanPolicy: DecisionPolicy {
    fn submitted_response(&self, ticket: &DecisionTicket) -> Option<HumanDecisionResponse>;
}

#[derive(Clone, Debug)]
pub struct QueuedHumanPolicy {
    identity: DecisionPolicyIdentity,
    responses: BTreeMap<canwu_core::DecisionTicketId, HumanDecisionResponse>,
}

impl QueuedHumanPolicy {
    #[must_use]
    pub fn new(id: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            identity: DecisionPolicyIdentity::new(DecisionPolicyKind::Human, id, version),
            responses: BTreeMap::new(),
        }
    }

    pub fn submit(
        &mut self,
        ticket_id: canwu_core::DecisionTicketId,
        response: HumanDecisionResponse,
    ) {
        self.responses.insert(ticket_id, response);
    }
}

impl HumanPolicy for QueuedHumanPolicy {
    fn submitted_response(&self, ticket: &DecisionTicket) -> Option<HumanDecisionResponse> {
        self.responses.get(&ticket.id).cloned()
    }
}

impl DecisionPolicy for QueuedHumanPolicy {
    fn identity(&self) -> DecisionPolicyIdentity {
        self.identity.clone()
    }

    fn decide(&self, ticket: &DecisionTicket) -> Result<PolicyDecision, DecisionError> {
        let Some(response) = self.submitted_response(ticket) else {
            return Ok(PolicyDecision::pending("awaiting human selection"));
        };
        if response.ticket_version != ticket.version {
            return Err(DecisionError::new(
                DecisionErrorCode::VersionConflict,
                "human response targets a stale decision ticket version",
            ));
        }
        Ok(PolicyDecision {
            outcome: DecisionOutcome::Selected {
                option_id: response.option_id,
            },
            summary: format!("human operator {} selected an option", response.operator_id),
            evaluations: Vec::new(),
            external: Some(DecisionExternalEvidence {
                provider: "human".to_owned(),
                model: None,
                prompt_contract: None,
                request_id: Some(response.operator_id),
                metadata: BTreeMap::new(),
            }),
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExternalDecisionOption {
    pub id: String,
    pub label: String,
    pub description: String,
    pub metadata: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExternalDecisionRequest {
    pub ticket_id: canwu_core::DecisionTicketId,
    pub ticket_version: u64,
    pub definition: String,
    pub summary: String,
    pub context: crate::DecisionContext,
    pub options: Vec<ExternalDecisionOption>,
}

impl From<&DecisionTicket> for ExternalDecisionRequest {
    fn from(ticket: &DecisionTicket) -> Self {
        Self {
            ticket_id: ticket.id,
            ticket_version: ticket.version,
            definition: ticket.definition.clone(),
            summary: ticket.summary.clone(),
            context: ticket.context.clone(),
            options: ticket
                .options
                .iter()
                .filter(|option| option.is_available())
                .map(|option| ExternalDecisionOption {
                    id: option.id.clone(),
                    label: option.label.clone(),
                    description: option.description.clone(),
                    metadata: option.metadata.clone(),
                })
                .collect(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExternalDecisionResponse {
    pub ticket_version: u64,
    pub option_id: String,
    pub provider: String,
    pub request_id: String,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

pub trait ExternalPolicy: DecisionPolicy {
    fn external_request(&self, ticket: &DecisionTicket) -> ExternalDecisionRequest {
        ticket.into()
    }

    fn submitted_response(&self, ticket: &DecisionTicket) -> Option<ExternalDecisionResponse>;
}

#[derive(Clone, Debug)]
pub struct QueuedExternalPolicy {
    identity: DecisionPolicyIdentity,
    responses: BTreeMap<canwu_core::DecisionTicketId, ExternalDecisionResponse>,
}

impl QueuedExternalPolicy {
    #[must_use]
    pub fn new(id: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            identity: DecisionPolicyIdentity::new(DecisionPolicyKind::External, id, version),
            responses: BTreeMap::new(),
        }
    }

    pub fn submit(
        &mut self,
        ticket_id: canwu_core::DecisionTicketId,
        response: ExternalDecisionResponse,
    ) {
        self.responses.insert(ticket_id, response);
    }
}

impl ExternalPolicy for QueuedExternalPolicy {
    fn submitted_response(&self, ticket: &DecisionTicket) -> Option<ExternalDecisionResponse> {
        self.responses.get(&ticket.id).cloned()
    }
}

impl DecisionPolicy for QueuedExternalPolicy {
    fn identity(&self) -> DecisionPolicyIdentity {
        self.identity.clone()
    }

    fn decide(&self, ticket: &DecisionTicket) -> Result<PolicyDecision, DecisionError> {
        let Some(response) = self.submitted_response(ticket) else {
            return Ok(PolicyDecision::pending("awaiting external policy response"));
        };
        if response.ticket_version != ticket.version {
            return Err(DecisionError::new(
                DecisionErrorCode::VersionConflict,
                "external response targets a stale decision ticket version",
            ));
        }
        Ok(PolicyDecision {
            outcome: DecisionOutcome::Selected {
                option_id: response.option_id,
            },
            summary: format!("external provider {} selected an option", response.provider),
            evaluations: Vec::new(),
            external: Some(DecisionExternalEvidence {
                provider: response.provider,
                model: None,
                prompt_contract: None,
                request_id: Some(response.request_id),
                metadata: response.metadata,
            }),
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LlmModelIdentity {
    pub provider: String,
    pub model: String,
    pub prompt_contract: String,
}

pub trait LlmPolicy: ExternalPolicy {
    fn model_identity(&self) -> &LlmModelIdentity;
}

#[derive(Clone, Debug)]
pub struct QueuedLlmPolicy {
    identity: DecisionPolicyIdentity,
    model: LlmModelIdentity,
    responses: BTreeMap<canwu_core::DecisionTicketId, ExternalDecisionResponse>,
}

impl QueuedLlmPolicy {
    #[must_use]
    pub fn new(id: impl Into<String>, version: impl Into<String>, model: LlmModelIdentity) -> Self {
        Self {
            identity: DecisionPolicyIdentity::new(DecisionPolicyKind::Llm, id, version),
            model,
            responses: BTreeMap::new(),
        }
    }

    pub fn submit(
        &mut self,
        ticket_id: canwu_core::DecisionTicketId,
        response: ExternalDecisionResponse,
    ) {
        self.responses.insert(ticket_id, response);
    }
}

impl ExternalPolicy for QueuedLlmPolicy {
    fn submitted_response(&self, ticket: &DecisionTicket) -> Option<ExternalDecisionResponse> {
        self.responses.get(&ticket.id).cloned()
    }
}

impl LlmPolicy for QueuedLlmPolicy {
    fn model_identity(&self) -> &LlmModelIdentity {
        &self.model
    }
}

impl DecisionPolicy for QueuedLlmPolicy {
    fn identity(&self) -> DecisionPolicyIdentity {
        self.identity.clone()
    }

    fn decide(&self, ticket: &DecisionTicket) -> Result<PolicyDecision, DecisionError> {
        let Some(response) = self.submitted_response(ticket) else {
            return Ok(PolicyDecision::pending(
                "awaiting constrained LLM option selection",
            ));
        };
        if response.ticket_version != ticket.version {
            return Err(DecisionError::new(
                DecisionErrorCode::VersionConflict,
                "LLM response targets a stale decision ticket version",
            ));
        }
        if response.provider != self.model.provider {
            return Err(DecisionError::new(
                DecisionErrorCode::PolicyMismatch,
                "LLM response provider does not match the configured model identity",
            ));
        }
        Ok(PolicyDecision {
            outcome: DecisionOutcome::Selected {
                option_id: response.option_id,
            },
            summary: format!(
                "LLM {}:{} selected an existing option",
                self.model.provider, self.model.model
            ),
            evaluations: Vec::new(),
            external: Some(DecisionExternalEvidence {
                provider: response.provider,
                model: Some(self.model.model.clone()),
                prompt_contract: Some(self.model.prompt_contract.clone()),
                request_id: Some(response.request_id),
                metadata: response.metadata,
            }),
        })
    }
}
