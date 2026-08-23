//! Generic, sparse, evidence-based technology simulation for Canwu.

#![allow(clippy::missing_errors_doc, clippy::module_name_repetitions)]

mod evaluator;
mod model;
mod plugin;
mod query;
mod schema;

pub use evaluator::{EvaluationError, evaluate_application, evaluate_attempt};
pub use model::*;
pub use plugin::{
    TECHNOLOGY_COMMAND, TECHNOLOGY_RESULT_INGRESS, TechnologyPlugin, technology_command_descriptor,
    technology_result_ingress_descriptor,
};
pub use query::{
    TechnologyRecordSet, from_technology_checkpoint_journal, from_technology_snapshot_json,
    replay_technology_from_journal, validate_technology_runtime,
};
pub use schema::{technology_knowledge_schemas, technology_record_schemas};

pub const PLUGIN_NAME: &str = "canwu-technology";
pub const PLUGIN_NAMESPACE: &str = "canwu.technology";
