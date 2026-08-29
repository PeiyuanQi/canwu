//! Ming fiscal reference content for Canwu.
//!
//! The embedded, Apache-2.0 pack covers 1368-1662 and exposes 1663-1683 as an
//! optional Zheng continuation. It compiles through `canwu-fiscal`; this crate
//! contains historical content, not engine behavior.

#![allow(clippy::missing_errors_doc)]

use canwu_fiscal::{
    CompiledFiscalCatalog, FiscalAdoptionStage, FiscalContentPack, FiscalContentSelection,
    FiscalHistoricalMode, compile_fiscal_content,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const PACK_ID: &str = "ming-fiscal-reference";
pub const MING_FISCAL_PACK_JSON: &str = include_str!("../data/pack.json");
pub const HONGWU_1391_FIXTURE_JSON: &str = include_str!("../data/fixtures/hongwu-1391.json");
pub const WANLI_1581_FIXTURE_JSON: &str = include_str!("../data/fixtures/wanli-1581.json");
pub const HONGGUANG_1644_FIXTURE_JSON: &str = include_str!("../data/fixtures/hongguang-1644.json");

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MingFixtureAdoption {
    pub adoption_id: String,
    pub rule_id: String,
    pub scope_id: String,
    pub stage: FiscalAdoptionStage,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MingFiscalFixture {
    pub id: String,
    pub label: String,
    pub historical_year: i32,
    pub mode: FiscalHistoricalMode,
    pub region_ids: BTreeSet<String>,
    pub adoptions: Vec<MingFixtureAdoption>,
    pub expected_active_period_ids: BTreeSet<String>,
    pub expected_transition_ids: BTreeSet<String>,
    pub design_note: String,
}

pub fn ming_fiscal_pack() -> Result<FiscalContentPack, serde_json::Error> {
    serde_json::from_str(MING_FISCAL_PACK_JSON)
}

pub fn compile_ming_fiscal(
    selection: FiscalContentSelection,
) -> Result<CompiledFiscalCatalog, canwu_api::CanwuError> {
    let pack = ming_fiscal_pack().map_err(|error| {
        canwu_api::CanwuError::new(
            canwu_api::ErrorCode::InvalidPayload,
            format!("embedded Ming fiscal pack could not be decoded: {error}"),
        )
    })?;
    compile_fiscal_content(&pack, selection)
}

pub fn ming_fiscal_fixture(id: &str) -> Result<MingFiscalFixture, serde_json::Error> {
    let source = match id {
        "hongwu-1391" => HONGWU_1391_FIXTURE_JSON,
        "wanli-1581" => WANLI_1581_FIXTURE_JSON,
        "hongguang-1644" => HONGGUANG_1644_FIXTURE_JSON,
        _ => {
            return Err(<serde_json::Error as serde::de::Error>::custom(format!(
                "unknown Ming fiscal fixture {id}"
            )));
        }
    };
    serde_json::from_str(source)
}
