use crate::model::require_text;
use crate::{
    DecisionError, DecisionErrorCode, DecisionExternalEvidence, DecisionFactorContribution,
    DecisionOption, DecisionOptionEvaluation, DecisionOptionWeight, DecisionOutcome,
    DecisionPolicyIdentity, DecisionPolicyKind, DecisionStage, DecisionTicket, PolicyDecision,
};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::BTreeMap;

pub trait DecisionPolicy {
    fn identity(&self) -> &DecisionPolicyIdentity;
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
    fn identity(&self) -> &DecisionPolicyIdentity {
        &self.identity
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
                random: None,
                stage: None,
                fired_guards: Vec::new(),
            });
        };
        Ok(PolicyDecision {
            outcome: DecisionOutcome::Selected {
                option_id: option_id.clone(),
            },
            summary: format!("utility policy selected {option_id}"),
            evaluations,
            external: None,
            random: None,
            stage: None,
            fired_guards: Vec::new(),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuleChoice {
    /// Selects an existing option and ends rule evaluation.
    Select(String),
    /// Defers the ticket and ends rule evaluation.
    Defer(String),
    /// Removes one option from later selection, records why, and continues
    /// with the next rule.
    Exclude {
        option_id: String,
        reason: String,
    },
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
    fn identity(&self) -> &DecisionPolicyIdentity {
        &self.identity
    }

    fn decide(&self, ticket: &DecisionTicket) -> Result<PolicyDecision, DecisionError> {
        let run = run_ordered_rules(&self.rules, ticket)?;
        let evaluations = run.exclusion_evaluations();
        let (outcome, summary) = match run.terminal {
            Some(RuleTerminal::Select { rule, option_id }) => (
                DecisionOutcome::Selected { option_id },
                format!("rule {rule} selected an option"),
            ),
            Some(RuleTerminal::Defer { rule, reason }) => (
                DecisionOutcome::Deferred {
                    reason: reason.clone(),
                },
                format!("rule {rule} deferred: {reason}"),
            ),
            None => (
                DecisionOutcome::Deferred {
                    reason: "no rule matched".to_owned(),
                },
                "ordered rule policy exhausted its rules".to_owned(),
            ),
        };
        Ok(PolicyDecision {
            outcome,
            summary,
            evaluations,
            external: None,
            random: None,
            stage: None,
            fired_guards: Vec::new(),
        })
    }
}

enum RuleTerminal {
    Select { rule: String, option_id: String },
    Defer { rule: String, reason: String },
}

#[derive(Default)]
struct OrderedRuleRun {
    fired: Vec<String>,
    /// Excluded option ID to its recorded blocker, in canonical option order.
    excluded: BTreeMap<String, String>,
    terminal: Option<RuleTerminal>,
}

impl OrderedRuleRun {
    fn exclusion_evaluations(&self) -> Vec<DecisionOptionEvaluation> {
        self.excluded
            .iter()
            .map(|(option_id, blocker)| excluded_evaluation(option_id, blocker))
            .collect()
    }
}

fn excluded_evaluation(option_id: &str, blocker: &str) -> DecisionOptionEvaluation {
    DecisionOptionEvaluation {
        option_id: option_id.to_owned(),
        available: false,
        score: None,
        factors: Vec::new(),
        blockers: vec![blocker.to_owned()],
    }
}

/// Runs ordered rules until one selects or defers. `Exclude` removes one
/// existing option from later selection and records the rule and reason; a
/// later rule cannot select an option an earlier rule excluded.
fn run_ordered_rules(
    rules: &[Box<dyn DecisionRule>],
    ticket: &DecisionTicket,
) -> Result<OrderedRuleRun, DecisionError> {
    let mut run = OrderedRuleRun::default();
    for rule in rules {
        match rule.evaluate(ticket)? {
            RuleChoice::NoMatch => {}
            RuleChoice::Select(option_id) => {
                if run.excluded.contains_key(&option_id) {
                    return Err(DecisionError::new(
                        DecisionErrorCode::InvalidDecision,
                        format!(
                            "rule {} selected option {option_id} after an earlier rule excluded it",
                            rule.id()
                        ),
                    ));
                }
                run.fired.push(rule.id().to_owned());
                run.terminal = Some(RuleTerminal::Select {
                    rule: rule.id().to_owned(),
                    option_id,
                });
                break;
            }
            RuleChoice::Defer(reason) => {
                run.fired.push(rule.id().to_owned());
                run.terminal = Some(RuleTerminal::Defer {
                    rule: rule.id().to_owned(),
                    reason,
                });
                break;
            }
            RuleChoice::Exclude { option_id, reason } => {
                if ticket.option(&option_id).is_none() {
                    return Err(DecisionError::new(
                        DecisionErrorCode::InvalidOption,
                        format!("rule {} excluded unknown option {option_id}", rule.id()),
                    ));
                }
                require_text(&reason, "rule exclusion reason")?;
                run.fired.push(rule.id().to_owned());
                run.excluded
                    .entry(option_id)
                    .or_insert_with(|| format!("excluded by {}: {reason}", rule.id()));
            }
        }
    }
    Ok(run)
}

/// A composite policy: ordered guards, then weighted utility over the options
/// the guards left, then an optional bounded random tie-break.
///
/// Guards run in order. The first `Select` or `Defer` ends the decision at the
/// guard stage; `Exclude` removes an option and records the guard and reason.
/// The remaining available options are scored. When `random_tie_break` is set
/// and at least two options score within `near_equivalence_margin` of the best
/// score, the policy returns [`DecisionOutcome::PendingRandom`] naming only
/// those candidates (uniform weight 1, canonical option order). A declared
/// boundary system resolves it with the existing `ResolveDecisionRandomly`
/// directive, passing this pending decision as the resolution's tie-break so
/// the draw evidence covers only the candidates. Otherwise the best score wins
/// and equal scores fall back to the lowest option ID.
///
/// The identity reuses [`DecisionPolicyKind::Utility`]; its
/// `semantic_hash` commits to the guard policy identity, the ordered guard
/// IDs, utility weights, margin, and tie-break flag, so a controller binding
/// rejects a reconfigured policy that keeps the same ID and version. Guard
/// behavior is application code outside the hash: change a guard's ID or the
/// guard policy version when its behavior changes. A controller must also opt
/// in with `DecisionControllerBinding::with_random_tie_break` before a pending
/// tie-break can be evaluated or resolved.
///
/// In `canwu-sim`, a random resolution fails its boundary before any draw is
/// committed when the ticket's person decision maker (`DecisionMakerUnavailable`)
/// or its controller's authority person (`IssuerUnavailable`) is unavailable in
/// the availability committed before that boundary. Any availability change
/// made in the same boundary does not fail it: the draw is committed and the
/// end-of-boundary sweep cancels the ticket instead. Tie-break systems should
/// skip tickets whose decision maker or controller authority person is
/// unavailable, read through `SimulationView::person_availability` with
/// `StateKey::core_person_availability()` declared as a read.
pub struct GuardedUtilityPolicy {
    identity: DecisionPolicyIdentity,
    guards: OrderedRulePolicy,
    utility: WeightedUtilityEvaluator,
    near_equivalence_margin: u64,
    random_tie_break: bool,
}

impl GuardedUtilityPolicy {
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        version: impl Into<String>,
        guards: OrderedRulePolicy,
        utility: WeightedUtilityEvaluator,
        near_equivalence_margin: u64,
        random_tie_break: bool,
    ) -> Self {
        let mut identity = DecisionPolicyIdentity::new(DecisionPolicyKind::Utility, id, version);
        identity.semantic_hash = Some(guarded_utility_semantic_hash(
            &guards,
            &utility,
            near_equivalence_margin,
            random_tie_break,
        ));
        Self {
            identity,
            guards,
            utility,
            near_equivalence_margin,
            random_tie_break,
        }
    }

    #[must_use]
    pub const fn guards(&self) -> &OrderedRulePolicy {
        &self.guards
    }

    #[must_use]
    pub const fn utility(&self) -> &WeightedUtilityEvaluator {
        &self.utility
    }

    #[must_use]
    pub const fn near_equivalence_margin(&self) -> u64 {
        self.near_equivalence_margin
    }

    #[must_use]
    pub const fn random_tie_break(&self) -> bool {
        self.random_tie_break
    }
}

fn guarded_utility_semantic_hash(
    guards: &OrderedRulePolicy,
    utility: &WeightedUtilityEvaluator,
    near_equivalence_margin: u64,
    random_tie_break: bool,
) -> String {
    fn text(hasher: &mut blake3::Hasher, value: &str) {
        hasher.update(&(value.len() as u64).to_be_bytes());
        hasher.update(value.as_bytes());
    }
    let mut hasher = blake3::Hasher::new();
    text(&mut hasher, "canwu.decision.guarded-utility-policy.v1");
    let guard_identity = guards.identity();
    text(&mut hasher, policy_kind_name(guard_identity.kind));
    text(&mut hasher, &guard_identity.id);
    text(&mut hasher, &guard_identity.version);
    hasher.update(&(guards.rules().len() as u64).to_be_bytes());
    for rule in guards.rules() {
        text(&mut hasher, rule.id());
    }
    hasher.update(&(utility.profile.weights.len() as u64).to_be_bytes());
    for (factor, weight) in &utility.profile.weights {
        text(&mut hasher, factor);
        hasher.update(&weight.to_be_bytes());
    }
    hasher.update(&near_equivalence_margin.to_be_bytes());
    hasher.update(&[u8::from(random_tie_break)]);
    hasher.finalize().to_hex().to_string()
}

const fn policy_kind_name(kind: DecisionPolicyKind) -> &'static str {
    match kind {
        DecisionPolicyKind::Utility => "utility",
        DecisionPolicyKind::Rule => "rule",
        DecisionPolicyKind::Random => "random",
        DecisionPolicyKind::Human => "human",
        DecisionPolicyKind::External => "external",
        DecisionPolicyKind::Llm => "llm",
    }
}

impl UtilityEvaluator for GuardedUtilityPolicy {
    fn evaluate(
        &self,
        ticket: &DecisionTicket,
        option: &DecisionOption,
    ) -> Result<DecisionOptionEvaluation, DecisionError> {
        self.utility.evaluate(ticket, option)
    }
}

impl DecisionPolicy for GuardedUtilityPolicy {
    fn identity(&self) -> &DecisionPolicyIdentity {
        &self.identity
    }

    fn decide(&self, ticket: &DecisionTicket) -> Result<PolicyDecision, DecisionError> {
        let mut run = run_ordered_rules(self.guards.rules(), ticket)?;
        let fired_guards = std::mem::take(&mut run.fired);
        if let Some(terminal) = run.terminal.take() {
            let (outcome, summary) = match terminal {
                RuleTerminal::Select { rule, option_id } => (
                    DecisionOutcome::Selected {
                        option_id: option_id.clone(),
                    },
                    format!("guard {rule} selected {option_id}"),
                ),
                RuleTerminal::Defer { rule, reason } => (
                    DecisionOutcome::Deferred {
                        reason: reason.clone(),
                    },
                    format!("guard {rule} deferred: {reason}"),
                ),
            };
            return Ok(PolicyDecision {
                outcome,
                summary,
                evaluations: run.exclusion_evaluations(),
                external: None,
                random: None,
                stage: Some(DecisionStage::Guard),
                fired_guards,
            });
        }
        let mut evaluations = ticket
            .options
            .iter()
            .map(|option| match run.excluded.get(&option.id) {
                Some(blocker) => {
                    let mut evaluation = excluded_evaluation(&option.id, blocker);
                    evaluation
                        .blockers
                        .splice(0..0, option.blockers.iter().cloned());
                    Ok(evaluation)
                }
                None => self.utility.evaluate(ticket, option),
            })
            .collect::<Result<Vec<_>, _>>()?;
        // Canonical option-ID order also makes the tie-break candidates
        // canonical, whatever order a caller-built ticket uses.
        evaluations.sort_by(|left, right| left.option_id.cmp(&right.option_id));
        let scored = evaluations
            .iter()
            .filter_map(|evaluation| {
                evaluation
                    .score
                    .map(|score| (score, evaluation.option_id.as_str()))
            })
            .collect::<Vec<_>>();
        let Some(&(best, winner)) = scored
            .iter()
            .max_by(|left, right| left.0.cmp(&right.0).then_with(|| right.1.cmp(left.1)))
        else {
            return Ok(PolicyDecision {
                outcome: DecisionOutcome::Deferred {
                    reason: "no available option".to_owned(),
                },
                summary: "guarded utility policy deferred because no option remained after guards"
                    .to_owned(),
                evaluations,
                external: None,
                random: None,
                stage: Some(DecisionStage::Utility),
                fired_guards,
            });
        };
        let margin = i128::from(self.near_equivalence_margin);
        let candidates = scored
            .iter()
            .filter(|(score, _)| i128::from(best) - i128::from(*score) <= margin)
            .map(|(_, option_id)| DecisionOptionWeight::new(*option_id, 1))
            .collect::<Vec<_>>();
        if self.random_tie_break && candidates.len() > 1 {
            let summary = format!(
                "guarded utility policy left {} near-equivalent options for a random tie-break",
                candidates.len()
            );
            return Ok(PolicyDecision {
                outcome: DecisionOutcome::PendingRandom { candidates },
                summary,
                evaluations,
                external: None,
                random: None,
                stage: Some(DecisionStage::Random),
                fired_guards,
            });
        }
        let option_id = winner.to_owned();
        Ok(PolicyDecision {
            outcome: DecisionOutcome::Selected {
                option_id: option_id.clone(),
            },
            summary: format!("guarded utility policy selected {option_id}"),
            evaluations,
            external: None,
            random: None,
            stage: Some(DecisionStage::Utility),
            fired_guards,
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
    ) -> Result<(), DecisionError> {
        if let Some(existing) = self.responses.get(&ticket_id) {
            return match existing.ticket_version.cmp(&response.ticket_version) {
                Ordering::Less => {
                    self.responses.insert(ticket_id, response);
                    Ok(())
                }
                Ordering::Equal => Err(DecisionError::new(
                    DecisionErrorCode::DuplicateResponse,
                    "a response has already been submitted for this decision ticket version",
                )),
                Ordering::Greater => Err(DecisionError::new(
                    DecisionErrorCode::VersionConflict,
                    "a stale decision response cannot replace a newer queued response",
                )),
            };
        }
        self.responses.insert(ticket_id, response);
        Ok(())
    }
}

impl HumanPolicy for QueuedHumanPolicy {
    fn submitted_response(&self, ticket: &DecisionTicket) -> Option<HumanDecisionResponse> {
        self.responses.get(&ticket.id).cloned()
    }
}

impl DecisionPolicy for QueuedHumanPolicy {
    fn identity(&self) -> &DecisionPolicyIdentity {
        &self.identity
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
            random: None,
            stage: None,
            fired_guards: Vec::new(),
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
    ) -> Result<(), DecisionError> {
        if let Some(existing) = self.responses.get(&ticket_id) {
            return match existing.ticket_version.cmp(&response.ticket_version) {
                Ordering::Less => {
                    self.responses.insert(ticket_id, response);
                    Ok(())
                }
                Ordering::Equal => Err(DecisionError::new(
                    DecisionErrorCode::DuplicateResponse,
                    "a response has already been submitted for this decision ticket version",
                )),
                Ordering::Greater => Err(DecisionError::new(
                    DecisionErrorCode::VersionConflict,
                    "a stale decision response cannot replace a newer queued response",
                )),
            };
        }
        self.responses.insert(ticket_id, response);
        Ok(())
    }
}

impl ExternalPolicy for QueuedExternalPolicy {
    fn submitted_response(&self, ticket: &DecisionTicket) -> Option<ExternalDecisionResponse> {
        self.responses.get(&ticket.id).cloned()
    }
}

impl DecisionPolicy for QueuedExternalPolicy {
    fn identity(&self) -> &DecisionPolicyIdentity {
        &self.identity
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
            random: None,
            stage: None,
            fired_guards: Vec::new(),
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
    ) -> Result<(), DecisionError> {
        if let Some(existing) = self.responses.get(&ticket_id) {
            return match existing.ticket_version.cmp(&response.ticket_version) {
                Ordering::Less => {
                    self.responses.insert(ticket_id, response);
                    Ok(())
                }
                Ordering::Equal => Err(DecisionError::new(
                    DecisionErrorCode::DuplicateResponse,
                    "a response has already been submitted for this decision ticket version",
                )),
                Ordering::Greater => Err(DecisionError::new(
                    DecisionErrorCode::VersionConflict,
                    "a stale decision response cannot replace a newer queued response",
                )),
            };
        }
        self.responses.insert(ticket_id, response);
        Ok(())
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
    fn identity(&self) -> &DecisionPolicyIdentity {
        &self.identity
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
            random: None,
            stage: None,
            fired_guards: Vec::new(),
        })
    }
}
