//! Persistable decision contracts and policy adapters for Canwu.
//!
//! Policies receive only the explicit [`DecisionTicket`] projection and may
//! select an existing option ID. They never receive mutable simulation state,
//! command authority, or a way to invent an action outside the ticket.

#![allow(
    clippy::missing_errors_doc,
    clippy::module_name_repetitions,
    clippy::too_many_lines
)]

mod engine;
mod model;
mod policy;

pub use engine::{ControllerDecision, DecisionController, DecisionState, PreparedDecision};
pub use model::{
    DecisionAction, DecisionAttemptErrorCode, DecisionAttemptOutcome, DecisionAttemptRecord,
    DecisionAuthority, DecisionContext, DecisionControllerBinding, DecisionError,
    DecisionErrorCode, DecisionExternalEvidence, DecisionFactorContribution, DecisionMutation,
    DecisionOption, DecisionOptionEvaluation, DecisionOutcome, DecisionPolicyIdentity,
    DecisionPolicyKind, DecisionTicket, DecisionTicketDraft, DecisionTicketState, DecisionTrace,
    PolicyDecision,
};
pub use policy::{
    DecisionPolicy, DecisionRule, ExternalDecisionOption, ExternalDecisionRequest,
    ExternalDecisionResponse, ExternalPolicy, HumanDecisionResponse, HumanPolicy, LlmModelIdentity,
    LlmPolicy, OrderedRulePolicy, QueuedExternalPolicy, QueuedHumanPolicy, QueuedLlmPolicy,
    RuleChoice, RulePolicy, UtilityEvaluator, UtilityPolicy, UtilityProfile,
    WeightedUtilityEvaluator, WeightedUtilityPolicy,
};
