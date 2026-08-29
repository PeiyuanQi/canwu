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
// Format-8 storage work remains a private, non-loadable test scaffold until
// the page store, replay ingress, and persistence-format switch land together.
#[cfg(test)]
mod storage;

pub use compiler::{compile_law, validate_compiled_law_plan, validate_definition};
pub use model::*;
pub use plugin::{
    LAW_ACTOR_CONTEXT_INGRESS, LAW_COMMAND, LAW_INTENT_INGRESS, LAW_MUTATION_INGRESS,
    LAW_OUTBOX_ACK_INGRESS, LAW_OUTBOX_PREPARE_INGRESS, LAW_RUNTIME_STATE, LAW_WAKE_INGRESS,
    LawPlugin, enqueue_legal_mutation, enqueue_legal_mutation_at, law_command_descriptor,
    law_record_schemas, load_law_state_for_plan, load_legal_runtime,
};
pub use runtime::{
    LegalBoundaryResult, LegalMutation, LegalRuntime, LegalSignal, decision_controller_id,
};

pub const PLUGIN_NAME: &str = "canwu-law";
pub const PLUGIN_NAMESPACE: &str = "canwu.law";

/// Stable semantic identity for the registered legal record and command contract.
pub const LAW_SEMANTIC_HASH: &str =
    "8989b95910c96fb6bd6a39298b1bdd30209c396147f32ddc24d5c98143d0cc1b";
