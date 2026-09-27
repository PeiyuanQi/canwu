//! Declarative culture authoring and lifecycle support for Canwu.
//!
//! This published extension compiles reference content into deterministic,
//! budgeted plans and adapts those plans to the generic `canwu-society`
//! runtime. It never writes legal or other downstream domain state directly.

#![allow(clippy::missing_errors_doc, clippy::module_name_repetitions)]

mod boundary;
mod compiler;
mod lifecycle;
mod model;
mod plugin;
mod society;

pub use boundary::{
    BOUNDARY_SEMANTIC_HASH, CULTURAL_SIGNAL_INGRESS, CULTURE_EXPOSURE_INGRESS,
    CULTURE_EXPOSURE_INTAKE_SYSTEM, CULTURE_EXPOSURE_REJECTED_EVENT,
    CULTURE_LIFECYCLE_REJECTED_EVENT, CULTURE_LIFECYCLE_SYSTEM, CULTURE_LIFECYCLE_TRANSITION_EVENT,
    CultureBoundaryPlugin, CultureDefinitionRecord, CultureExposureQueue,
    CultureExposureQueueRecord, CultureExposureSignalBatch, MAX_CULTURE_EXPOSURE_QUEUE,
    QueuedCultureExposure, culture_definition_record, culture_definition_reference,
    culture_exposure_queue_reference,
};
pub use compiler::compile_culture;
pub use lifecycle::{CultureRuntime, LifecycleObservation};
pub use model::{
    CULTURE_PLAN_HASH_DOMAIN, CULTURE_PLAN_VERSION, CULTURE_SCHEMA_VERSION, ChannelKey,
    ChannelSpec, CohortKey, CompiledChannel, CompiledCohort, CompiledCulturePlan, CompiledEffect,
    CompiledInstitutionBinding, CompiledTarget, CompiledTransition, CulturalEffectBinding,
    CulturalSignal, CulturalSignalBatch, CultureBudgets, CultureCohortDefinition,
    CultureDefinition, CultureDefinitionBuilder, CultureLifecycle, CultureState, DirtyPair,
    DirtySet, EffectEmissionCursor, EffectKey, EffectPersistence, InstitutionBinding,
    InstitutionKey, LifecycleTransition, LifecycleTransitionKind, RetiredTargetTombstone,
    RetirementPolicy, TargetKey, TargetLifecycle, TransitionKey, TransitionSpec,
};
pub use model::{CultureStateRecord, culture_state_reference};
pub use plugin::{CulturePlugin, SEMANTIC_HASH, load_culture_runtime, load_culture_state_for_plan};
pub use society::{
    install_definition_into_society, install_into_society, settle_culture_society_boundary,
    society_distribution_id, society_lifecycle_delta, synchronize_society_lifecycle,
};

pub const PLUGIN_NAME: &str = "canwu-culture";
pub const PLUGIN_NAMESPACE: &str = "canwu.culture";
