//! Generic fiscal institutions and procedure for Canwu.
//!
//! The extension owns fiscal law, adoption, assessment, authorization,
//! remission, execution receipts, audits, accounting cycle books and their
//! archive, and holder-relative knowledge reports. Resource balances and
//! physical transfers remain owned by their source domains.
//!
//! A run keeps its fiscal state as records of the kinds of
//! [`FiscalRecordKind`]: one root, one archive head, an office per
//! institution, a ledger per scope binding, a jurisdiction per catalog
//! region in use, an observer per observer binding, and the binding
//! directory, scope index, report queue, and report schedule they derive. A
//! boundary decodes and writes only the records its packets touch.

#![allow(clippy::missing_errors_doc)]

mod appointment;
mod archive;
mod content;
mod derive;
mod model;
mod plugin;
mod projection;
mod records;
mod report;
mod settle;
mod working;

pub use archive::*;
pub use content::compile_fiscal_content;
pub use derive::{assemble_fiscal_reports, compute_projection, jurisdiction_candidates};
pub use model::*;
pub use plugin::{
    APPLY_FISCAL_ACTION_COMMAND, FISCAL_ACTION_INGRESS, FISCAL_ACTION_SETTLED_EVENT,
    FISCAL_ARCHIVE_COMMIT_INGRESS, FISCAL_ARCHIVE_MAINTENANCE_EVENT,
    FISCAL_ARCHIVE_RETENTION_INGRESS, FISCAL_EXECUTION_RECEIPTS_INGRESS,
    FISCAL_HISTORICAL_CONTEXT_INGRESS, FISCAL_INGRESS_REJECTED_EVENT, FISCAL_REPORT_DRAIN_INGRESS,
    FiscalArchiveIngressReceiptV1, FiscalPlugin, PLUGIN_NAME, enqueue_execution_receipts,
    enqueue_fiscal_archive, finalize_fiscal_archive_retention, fiscal_action_command,
    fiscal_action_target, fiscal_historical_context_ingress, fiscal_recent_outcome,
    fiscal_target_revision,
};
pub use projection::{load_fiscal_catalog, load_fiscal_state};
pub use records::{
    FiscalArchiveRecord, FiscalDirectoryBucket, FiscalDirectoryRecord, FiscalIndexOwner,
    FiscalIndexPage, FiscalIndexRecord, FiscalJurisdictionRecord, FiscalLedgerRecord,
    FiscalObserverRecord, FiscalOfficeRecord, FiscalRecordKind, FiscalReportQueuePage,
    FiscalReportQueueRecord, FiscalReportScheduleRecord, FiscalRootRecord, audit_fiscal_record_set,
    audit_fiscal_records, fiscal_archive_reference, fiscal_jurisdiction_reference,
    fiscal_ledger_reference, fiscal_observer_reference, fiscal_office_reference,
    fiscal_report_schedule_reference, fiscal_root_reference,
};
pub use report::{fiscal_report_knowledge_schema_id, fiscal_reports, fiscal_reports_of_holder};
pub use settle::FiscalSettledAction;
