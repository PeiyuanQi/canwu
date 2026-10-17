//! Social diffusion simulation module for Canwu.
//!
//! The crate models generic population dispositions, influence networks,
//! organization topology, institutional alignment, policy pressure,
//! intergroup relations between cohort classes, and authorized observer
//! estimates. Historical meanings remain in downstream
//! packages. Architecturally, it is a published domain extension built on
//! Canwu's public engine contracts, not a kernel subsystem.

mod decision;
mod ingress;
mod intergroup;
mod lifecycle;
mod model;
mod plugin;
mod projection;
mod revision;
mod solver;

pub use decision::{
    PolicyChoice, PressureRevisionChoice, institutional_policy_ticket, pressure_revision_ticket,
};
pub use ingress::{
    COHORT_REBASE_INGRESS, CohortHeadcountRebaseV1, MAX_SOCIETY_INGRESS_QUEUE,
    QueuedSocietyIngress, REBASE_INVALID_STOCK_REJECTION, REBASE_STALE_STOCK_REJECTION,
    RebaseReason, SOCIETY_INGRESS_MALFORMED_REJECTION, SOCIETY_INGRESS_QUEUE_FULL_REJECTION,
    SocietyIngressBatch, SocietyIngressBatchRecord, SocietyIngressQueue, SocietyIngressQueueRecord,
    society_ingress_batch_reference, society_ingress_queue_reference,
};
pub use intergroup::{
    CohortClass, INTERGROUP_RELATION_CLASS_REJECTION, INTERGROUP_RELATION_IDENTITY_REJECTION,
    INTERGROUP_RELATION_INGRESS, INTERGROUP_RELATION_KIND_REJECTION,
    INTERGROUP_RELATION_REGION_REJECTION, INTERGROUP_RELATION_REJECTED_EVENT,
    INTERGROUP_RELATION_STALE_REJECTION, INTERGROUP_RELATIONS_FULL_REJECTION,
    IntergroupRelationChange, IntergroupRelationEdge, IntergroupRelationRevisionV1,
    IntergroupRelations, IntergroupRelationsRecord, MAX_COHORT_CLASS_TEXT_BYTES,
    MAX_INTERGROUP_RELATION_CHANGES, MAX_INTERGROUP_RELATION_EVIDENCE, MAX_INTERGROUP_RELATIONS,
    intergroup_relations_reference,
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
    COHORT_TRANSFER_INSUFFICIENT_SOURCE, OutcomeRetention, PLUGIN_NAME,
    SOCIETY_INGRESS_REJECTED_EVENT, SocietyPlugin, TransferSettlement,
    society_policy_decision_state_key,
};
pub use projection::{
    from_society_snapshot_json, from_society_snapshot_json_with_plugin, load_intergroup_relations,
    load_society_state, projection_for_viewer, society_ingress_queue_entries,
    validate_society_runtime,
};
pub use revision::{
    InfluenceEdgeRevision, MAX_PRESSURE_REVISION_ROWS, OrganizationRevision,
    PRESSURE_REVISION_AUTHORITY_REJECTION, PRESSURE_REVISION_COMMAND,
    PRESSURE_REVISION_ORIGIN_REJECTION, PRESSURE_REVISION_SCOPE_REJECTION,
    PRESSURE_REVISION_STALE_REJECTION, PRESSURE_REVISION_SUPERSEDED_REJECTION,
    PolicyPressureRevision, PressureRevisionDecision, PressureRevisionOutcome,
    PressureRevisionOutcomes, SOCIETY_PRESSURE_REVISED_EVENT, SOCIETY_PRESSURE_REVISION_INGRESS,
    SOCIETY_PRESSURE_REVISION_REJECTED_EVENT, SocietyPressureRevisionV1,
};
pub use solver::{compute_aggregates, settle_transitions};
