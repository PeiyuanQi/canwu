//! Social diffusion simulation module for Canwu.
//!
//! The crate models generic population dispositions, influence networks,
//! organization topology, institutional alignment, policy pressure, and
//! authorized observer estimates. Historical meanings remain in downstream
//! packages. Architecturally, it is a published domain extension built on
//! Canwu's public engine contracts, not a kernel subsystem.

mod decision;
mod ingress;
mod lifecycle;
mod model;
mod plugin;
mod projection;
mod solver;

pub use decision::{PolicyChoice, institutional_policy_ticket};
pub use ingress::{
    COHORT_REBASE_INGRESS, CohortHeadcountRebaseV1, MAX_SOCIETY_INGRESS_QUEUE,
    QueuedSocietyIngress, REBASE_INVALID_STOCK_REJECTION, REBASE_STALE_STOCK_REJECTION,
    RebaseReason, SOCIETY_INGRESS_MALFORMED_REJECTION, SocietyIngressQueue,
    SocietyIngressQueueRecord, society_ingress_queue_reference,
};
pub use lifecycle::{
    SOCIETY_LIFECYCLE_DELTA_INGRESS, SocietyLifecycleDeltaV1, SocietyTargetBindings,
};
pub use model::{
    AffiliationTarget, AssentBand, AwarenessBand, CohortHeadcountRebaseOutcome,
    CohortTransferIntent, CohortTransferOutcome, DispositionBucket, DispositionDistribution,
    DispositionProfile, InfluenceSource, InstitutionalAlignment, MobilizationBand,
    MobilizationCandidate, ObserverProfile, OrganizationNode, OrganizationRelation,
    OrganizationalTieBand, PendingCohortTransfer, PolicyDecision, PolicyPressure, PracticeBand,
    ProjectionEntry, PublicAlignmentBand, SocialInfluenceEdge, SocietyAggregate, SocietyCohort,
    SocietyCohortExchangeLedger, SocietyCohortExchangeLedgerRecord, SocietyCohortTransferPending,
    SocietyCohortTransferPendingRecord, SocietyIngressStatus, SocietyLifecycleDeltaOutcome,
    SocietyProjection, SocietyState, SocietyStateRecord, TransitionRemainder, TransitionRule,
    TransitionWeights, VisibilityBand, distribution_id, society_cohort_exchange_ledger_reference,
    society_cohort_transfer_pending_reference, society_state_reference,
};
pub use plugin::{
    PLUGIN_NAME, SOCIETY_INGRESS_REJECTED_EVENT, SocietyPlugin, society_policy_decision_state_key,
};
pub use projection::{
    from_society_snapshot_json, load_society_state, projection_for_viewer, validate_society_runtime,
};
pub use solver::{compute_aggregates, settle_transitions};
