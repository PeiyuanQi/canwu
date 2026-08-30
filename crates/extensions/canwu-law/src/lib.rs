//! Deterministic legal institutionalization extension for Canwu.
//!
//! The kernel remains domain-neutral. [`LegalRuntime`] owns legal records and
//! turns holder-bound command results into pending intents that are consumed at
//! the next legal boundary.

#![allow(clippy::missing_errors_doc, clippy::module_name_repetitions)]
#![allow(
    clippy::assigning_clones,
    clippy::cast_possible_truncation,
    clippy::collapsible_if,
    clippy::double_must_use,
    clippy::items_after_statements,
    clippy::map_unwrap_or,
    clippy::obfuscated_if_else,
    clippy::too_many_lines,
    clippy::uninlined_format_args,
    clippy::unnecessary_wraps,
    clippy::unused_self,
    clippy::wildcard_imports
)]

mod compiler;
mod model;
mod plugin;
mod runtime;
/// Format-8 legal shard, archive, and deterministic compaction contracts.
pub mod storage;

pub use compiler::{compile_law, validate_compiled_law_plan, validate_definition};
pub use model::*;
pub use plugin::{
    LAW_ACTOR_CONTEXT_INGRESS, LAW_ARCHIVE_COMMIT_INGRESS, LAW_ARCHIVE_HEAD_STATE,
    LAW_ARCHIVE_RETENTION_ACK_INGRESS, LAW_COMMAND, LAW_DIRECTORY_STATE, LAW_INTENT_INGRESS,
    LAW_MUTATION_INGRESS, LAW_OUTBOX_ACK_INGRESS, LAW_OUTBOX_PREPARE_INGRESS, LAW_PLAN_STATE,
    LAW_RUNTIME_STATE, LAW_SHARD_STATE, LAW_WAKE_INGRESS, LawPlugin, LegalArchiveIngressReceipt,
    enqueue_legal_archive, enqueue_legal_mutation, enqueue_legal_mutation_at,
    finalize_legal_archive_retention, law_command_descriptor, law_record_schemas,
    load_law_state_for_plan, load_legal_runtime,
};
pub use runtime::{
    LegalArchiveMaintenanceDisposition, LegalArchiveTerminalRecord, LegalBoundaryResult,
    LegalCanwuBoundaryScaleMetrics, LegalMutation, LegalRuntime,
    LegalRuntimeCompactionScaleMetrics, LegalSignal, decision_controller_id,
};
pub use storage::*;

pub const PLUGIN_NAME: &str = "canwu-law";
pub const PLUGIN_NAMESPACE: &str = "canwu.law";

/// Stable semantic identity for the registered legal record and command contract.
pub const LAW_SEMANTIC_HASH: &str =
    "a4581d5d38806f38a1069add4ba6ad275afc7667322275698320614730dbcfcf";
