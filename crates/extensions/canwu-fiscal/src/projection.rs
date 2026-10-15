use crate::model::{
    CompiledFiscalCatalog, FiscalCatalogRecord, FiscalCoverageCell, FiscalInstitutionDefinition,
    FiscalPeriodDefinition, FiscalProvenance, FiscalRegionDefinition, FiscalRuleDefinition,
    FiscalState, FiscalTransitionDefinition, HistoricalYearWindow, fiscal_catalog_reference,
};
use crate::records::{FiscalRootRecord, fiscal_records_of};
use canwu_api::{CanwuError, CanwuRun, DomainRecord, DomainRecordType, SimulationView};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

/// Decodes and fully validates the fiscal catalog a view can read.
pub fn load_fiscal_catalog(
    view: &SimulationView<'_>,
) -> Result<Option<(DomainRecord, CompiledFiscalCatalog)>, CanwuError> {
    let Some(record) = view.typed_domain_record(&fiscal_catalog_reference())? else {
        return Ok(None);
    };
    let catalog = record.decode_payload::<FiscalCatalogRecord>()?;
    catalog.validate()?;
    Ok(Some((record.clone(), catalog)))
}

/// Assembles the fiscal state of a run from every fiscal record it holds,
/// without validating it; `None` when the run has no fiscal root. It is
/// `O(fiscal records)`, for hosts, tools, and tests:
/// [`crate::audit_fiscal_records`] also checks it.
pub fn load_fiscal_state<H: CanwuRun + ?Sized>(
    canwu: &H,
) -> Result<Option<FiscalState>, CanwuError> {
    let records = fiscal_records_of(canwu);
    if !records
        .iter()
        .any(|record| record.reference.kind.matches_type::<FiscalRootRecord>())
    {
        return Ok(None);
    }
    FiscalState::from_records(records).map(Some)
}

/// What a boundary or command reads of the catalog: everything but its
/// coverage, provenance, periods, and institutions, which no handler reads
/// and which are the bulk of a large catalog.
#[derive(Deserialize)]
struct BoundaryCatalog {
    schema_version: u32,
    pack_id: String,
    pack_version: String,
    content_hash: String,
    historical_year: i32,
    historical_scope: HistoricalYearWindow,
    selected_period_ids: BTreeSet<String>,
    regions: BTreeMap<String, FiscalRegionDefinition>,
    rules: BTreeMap<String, FiscalRuleDefinition>,
    transitions: BTreeMap<String, FiscalTransitionDefinition>,
}

/// Decodes the committed fiscal catalog for the plugin's own handlers.
///
/// A fiscal record is validated where it enters a run: plugin activation
/// validates the catalog and every fiscal record of the scenario or restored
/// snapshot, the catalog is create-only, and every write of a fiscal record
/// is validated before it is proposed. The handlers therefore decode the
/// committed records without repeating that validation, and decode of the
/// catalog only what they read: its coverage, provenance, periods, and
/// institutions are left empty.
pub(crate) fn committed_fiscal_catalog(
    view: &SimulationView<'_>,
) -> Result<Option<CompiledFiscalCatalog>, CanwuError> {
    let Some(record) = view.typed_domain_record(&fiscal_catalog_reference())? else {
        return Ok(None);
    };
    let catalog = BoundaryCatalog::deserialize(&record.payload).map_err(|error| {
        crate::model::invalid(format!(
            "domain record {} has an incompatible typed payload: {error}",
            record.reference
        ))
    })?;
    let _: Option<(
        FiscalPeriodDefinition,
        FiscalInstitutionDefinition,
        FiscalCoverageCell,
        FiscalProvenance,
    )> = None;
    debug_assert_eq!(FiscalCatalogRecord::NAME, "catalog");
    Ok(Some(CompiledFiscalCatalog {
        schema_version: catalog.schema_version,
        pack_id: catalog.pack_id,
        pack_version: catalog.pack_version,
        content_hash: catalog.content_hash,
        historical_year: catalog.historical_year,
        historical_scope: catalog.historical_scope,
        selected_period_ids: catalog.selected_period_ids,
        periods: BTreeMap::new(),
        regions: catalog.regions,
        institutions: BTreeMap::new(),
        rules: catalog.rules,
        transitions: catalog.transitions,
        coverage: BTreeMap::new(),
        provenance: BTreeMap::new(),
    }))
}
