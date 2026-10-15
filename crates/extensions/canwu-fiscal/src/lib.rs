//! Generic fiscal institutions and procedure for Canwu.
//!
//! The extension owns fiscal law, adoption, assessment, authorization,
//! remission, execution receipts, audits, accounting cycle books and their
//! archive, and holder-relative knowledge reports. Resource balances and
//! physical transfers remain owned by their source domains.

#![allow(clippy::missing_errors_doc)]

mod appointment;
mod archive;
mod content;
mod derive;
mod model;
mod plugin;
mod projection;

pub use archive::*;
pub use content::compile_fiscal_content;
pub use derive::{
    assemble_fiscal_reports, compute_aggregates, compute_projections,
    compute_transition_candidates, recompute_derived,
};
pub use model::*;
pub use plugin::{
    APPLY_FISCAL_ACTION_COMMAND, FISCAL_ACTION_INGRESS, FISCAL_ACTION_SETTLED_EVENT,
    FISCAL_ARCHIVE_COMMIT_INGRESS, FISCAL_ARCHIVE_MAINTENANCE_EVENT,
    FISCAL_ARCHIVE_RETENTION_INGRESS, FISCAL_EXECUTION_RECEIPT_INGRESS,
    FISCAL_HISTORICAL_CONTEXT_INGRESS, FISCAL_INGRESS_REJECTED_EVENT, FISCAL_REPORT_DRAIN_INGRESS,
    FiscalArchiveIngressReceiptV1, FiscalArchiveRuntime, FiscalPlugin, FiscalSettledAction,
    PLUGIN_NAME, enqueue_execution_receipt, enqueue_fiscal_archive,
    finalize_fiscal_archive_retention, fiscal_action_command, fiscal_historical_context_ingress,
    fiscal_report_knowledge_schema_id, fiscal_reports, fiscal_reports_of_holder,
};
pub use projection::{load_fiscal_catalog, load_fiscal_state};
