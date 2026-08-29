use crate::model::{
    CompiledFiscalCatalog, FiscalCatalogRecord, FiscalState, FiscalStateRecord,
    fiscal_catalog_reference, fiscal_state_reference,
};
use canwu_api::{CanwuError, DomainRecord, SimulationView};

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
