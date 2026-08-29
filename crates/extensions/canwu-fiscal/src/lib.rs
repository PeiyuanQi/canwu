//! Generic fiscal institutions and procedure for Canwu.
//!
//! The extension owns fiscal law, adoption, assessment, authorization,
//! remission, execution receipts, audits, and holder-relative knowledge reports.
//! Resource balances and physical transfers remain owned by their source domains.

#![allow(clippy::missing_errors_doc)]

mod content;
mod derive;
mod model;
mod plugin;
mod projection;

pub use content::compile_fiscal_content;
pub use derive::{
    compute_aggregates, compute_projections, compute_transition_candidates, recompute_derived,
};
pub use model::*;
pub use plugin::{
    APPLY_FISCAL_ACTION_COMMAND, FISCAL_ACTION_INGRESS, FISCAL_EXECUTION_RECEIPT_INGRESS,
    FISCAL_HISTORICAL_CONTEXT_INGRESS, FiscalPlugin, PLUGIN_NAME, enqueue_execution_receipt,
    fiscal_action_command, fiscal_historical_context_ingress, fiscal_report_knowledge_schema_id,
};
pub use projection::{load_fiscal_catalog, load_fiscal_state};
