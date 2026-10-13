use crate::model::{
    CompiledFiscalCatalog, FiscalCatalogRecord, FiscalState, FiscalStateRecord,
    fiscal_catalog_reference, fiscal_state_reference,
};
use canwu_api::{CanwuError, DomainRecord, SimulationView};

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

/// Decodes and fully validates the fiscal state a view can read, including
/// that the record's payload is exactly the canonical encoding of the state.
pub fn load_fiscal_state(
    view: &SimulationView<'_>,
    catalog: &CompiledFiscalCatalog,
) -> Result<Option<(DomainRecord, FiscalState)>, CanwuError> {
    let Some(record) = view.typed_domain_record(&fiscal_state_reference())? else {
        return Ok(None);
    };
    let state = record.decode_payload::<FiscalStateRecord>()?;
    state.validate(catalog)?;
    state.validate_record_binding(record)?;
    Ok(Some((record.clone(), state)))
}

/// Decodes the committed fiscal catalog for the plugin's own handlers.
///
/// A fiscal record is validated where it enters a run: plugin activation
/// validates the catalog and state of the scenario or restored snapshot, the
/// catalog is create-only, and every write of the state is validated before it
/// is proposed. The handlers therefore decode the committed records without
/// repeating that validation on every command and boundary.
pub(crate) fn committed_fiscal_catalog(
    view: &SimulationView<'_>,
) -> Result<Option<CompiledFiscalCatalog>, CanwuError> {
    view.typed_domain_record(&fiscal_catalog_reference())?
        .map(DomainRecord::decode_payload::<FiscalCatalogRecord>)
        .transpose()
}

/// Decodes the committed fiscal state for the plugin's own handlers and
/// returns it with the record version an update must expect. See
/// [`committed_fiscal_catalog`] for why it is not validated again.
pub(crate) fn committed_fiscal_state(
    view: &SimulationView<'_>,
) -> Result<Option<(u64, FiscalState)>, CanwuError> {
    view.typed_domain_record(&fiscal_state_reference())?
        .map(|record| {
            Ok((
                record.version,
                record.decode_payload::<FiscalStateRecord>()?,
            ))
        })
        .transpose()
}
