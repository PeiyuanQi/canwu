//! Deterministic production assets and lifecycles for Canwu.
//!
//! This crate owns processes, sites, facilities, production capacity, work
//! orders, executions, work in progress, and site-local facility projects. It
//! never owns material balances: inputs cite exact `canwu-resource` outcomes,
//! and completed output settles only after an exact resource outcome is
//! acknowledged.

#![allow(
    clippy::missing_errors_doc,
    clippy::module_name_repetitions,
    clippy::too_many_lines
)]

mod archive;
mod completion;
mod model;
mod plugin;
mod query;

pub use archive::*;
pub use completion::*;
pub use model::*;
pub use plugin::{
    PRODUCTION_ARCHIVE_COMMIT_INGRESS, PRODUCTION_ARCHIVE_RETENTION_ACK_INGRESS,
    PRODUCTION_COMMAND, PRODUCTION_COMPLETION_INGRESS, PRODUCTION_OBSERVATION_WAKE_INGRESS,
    PRODUCTION_OUTPUT_ACK_INGRESS, PRODUCTION_RESOURCE_CONTINUATION_INGRESS,
    PRODUCTION_SEMANTIC_HASH, ProductionArchiveIngressReceiptV1, ProductionPlugin,
    enqueue_production_archive, enqueue_production_completion_operation,
    enqueue_production_resource_continuation, finalize_production_archive_retention,
    production_command_descriptor, production_incident_random_stream,
    production_output_ack_ingress_descriptor, production_report_knowledge_schema_id,
};
pub use query::{
    degraded_facility_decision_ticket, from_production_checkpoint_journal,
    from_production_checkpoint_journal_with_archives, from_production_snapshot_json,
    from_production_snapshot_json_with_archives, production_observation_witness, production_report,
    production_report_from_state, replay_production_from_journal,
    replay_production_from_journal_with_archives, validate_production_observation_witness,
    validate_production_resource_continuation, validate_production_runtime,
    validate_production_runtime_with_archives,
};

pub const PLUGIN_NAME: &str = "canwu-production";
pub const PLUGIN_NAMESPACE: &str = "canwu.production";
