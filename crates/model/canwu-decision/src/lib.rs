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

pub use engine::{
    ControllerDecision, DECISION_ARCHIVE_BUCKET_PAGE_FORMAT_VERSION,
    DECISION_ARCHIVE_FORMAT_VERSION, DECISION_HISTORY_BUCKET_BITS, DECISION_HISTORY_BUCKET_COUNT,
    DecisionArchiveBlob, DecisionArchiveBucketPage, DecisionArchivePageKey,
    DecisionArchiveProvider, DecisionArchiveReachability, DecisionArchiveReceipt,
    DecisionArchiveRecord, DecisionArchiveStore, DecisionArchiveStoreOutcome, DecisionController,
    DecisionHistoryCursor, DecisionHistoryKey, DecisionHistoryLocation, DecisionHistoryPage,
    DecisionHistoryQueryBudget, DecisionHotState, DecisionLocatorScaleFixture,
    DecisionLocatorScaleMetrics, DecisionState, MAX_DECISION_ARCHIVE_BATCH_ENTRIES,
    MAX_DECISION_ARCHIVE_BUCKET_PAGE_BYTES, MAX_DECISION_ARCHIVE_BUCKET_PAGE_ENTRIES,
    MAX_DECISION_HISTORY_PAGE_BYTES, MAX_DECISION_HISTORY_PAGE_SIZE, PersistentDecisionLog,
    PreparedDecision, PreparedDecisionArchive, TraceLocatorScaleMetrics,
    VerifiedDecisionArchiveCommit, decision_history_bucket, decision_history_page_key,
    format8_decision_locator_scale_fixture, format8_decision_locator_scale_probe,
    format8_trace_locator_scale_probe,
};
pub use model::{
    DecisionAction, DecisionAttemptErrorCode, DecisionAttemptOutcome, DecisionAttemptRecord,
    DecisionAuthority, DecisionContext, DecisionControllerBinding, DecisionError,
    DecisionErrorCode, DecisionExternalEvidence, DecisionFactorContribution, DecisionMutation,
    DecisionOption, DecisionOptionEvaluation, DecisionOptionWeight, DecisionOutcome,
    DecisionPolicyIdentity, DecisionPolicyKind, DecisionRandomEvidence, DecisionTicket,
    DecisionTicketDraft, DecisionTicketState, DecisionTrace, PolicyDecision,
};
pub use policy::{
    DecisionPolicy, DecisionRule, ExternalDecisionOption, ExternalDecisionRequest,
    ExternalDecisionResponse, ExternalPolicy, HumanDecisionResponse, HumanPolicy, LlmModelIdentity,
    LlmPolicy, OrderedRulePolicy, QueuedExternalPolicy, QueuedHumanPolicy, QueuedLlmPolicy,
    RuleChoice, RulePolicy, UtilityEvaluator, UtilityPolicy, UtilityProfile,
    WeightedUtilityEvaluator, WeightedUtilityPolicy,
};
