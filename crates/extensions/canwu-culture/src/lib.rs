//! Declarative culture authoring and lifecycle support for Canwu.
//!
//! This published extension compiles reference content into deterministic,
//! budgeted plans and adapts those plans to the generic `canwu-society`
//! runtime. It never writes legal or other downstream domain state directly.

#![allow(clippy::missing_errors_doc, clippy::module_name_repetitions)]

mod compiler;
mod lifecycle;
mod model;
mod plugin;
mod society;

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
    society_distribution_id, synchronize_society_lifecycle,
};

pub const PLUGIN_NAME: &str = "canwu-culture";
pub const PLUGIN_NAMESPACE: &str = "canwu.culture";
