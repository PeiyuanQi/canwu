//! Experimental aggregate social diffusion extension for Canwu.
//!
//! The crate models generic population dispositions, influence networks,
//! organization topology, institutional alignment, policy pressure, and
//! authorized observer estimates. Historical meanings remain in downstream
//! packages.

mod decision;
mod model;
mod plugin;
mod projection;
mod solver;

pub use decision::{PolicyChoice, institutional_policy_ticket};
pub use model::{
    AffiliationTarget, AssentBand, AwarenessBand, DispositionBucket, DispositionDistribution,
    DispositionProfile, InfluenceSource, InstitutionalAlignment, MobilizationBand,
    MobilizationCandidate, ObserverProfile, OrganizationNode, OrganizationRelation,
    OrganizationalTieBand, PolicyDecision, PolicyPressure, PracticeBand, ProjectionEntry,
    PublicAlignmentBand, SocialInfluenceEdge, SocietyAggregate, SocietyCohort, SocietyProjection,
    SocietyState, SocietyStateRecord, TransitionRemainder, TransitionRule, TransitionWeights,
    VisibilityBand, distribution_id, society_state_reference,
};
pub use plugin::{PLUGIN_NAME, SocietyPlugin};
pub use projection::{from_society_snapshot_json, load_society_state, projection_for_viewer};
pub use solver::{compute_aggregates, settle_transitions};
