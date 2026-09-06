//! Public-API-only composition of the military extension and reference world.

#![allow(clippy::missing_errors_doc, clippy::module_name_repetitions)]

use canwu_api::{CanwuError, Scenario};
use canwu_military::{
    CommanderProfile, MilitaryCatalog, MilitaryNodeId, MilitaryNodeProfile, MilitaryPlugin,
    MilitaryRecordMeta, MilitaryRulesetV1, catalog_reference, record_from,
};
use canwu_reference_world::{ReferenceWorldPlugin, demo_scenario};
use std::collections::BTreeMap;

pub const PLUGIN_NAME: &str = "canwu-military-reference";

pub fn demo_military_scenario()
-> Result<(Scenario, canwu_reference_world::ReferenceWorldIds), CanwuError> {
    let (mut scenario, ids) = demo_scenario()?;
    let (ruleset, _) = ruleset_profiles();
    let mut nodes = BTreeMap::new();
    for (id, territory, terrain) in [
        ("western", ids.western_territory, "market_town"),
        ("central", ids.central_territory, "levee"),
        ("eastern", ids.eastern_territory, "river"),
    ] {
        let node_id = MilitaryNodeId::new(format!("canwu.military:node:{id}"))?;
        nodes.insert(
            node_id.clone(),
            MilitaryNodeProfile {
                id: node_id,
                territory: territory.to_string(),
                terrain: terrain.to_owned(),
                administrative: true,
                supply_capacity: 10_000,
            },
        );
    }
    let commanders = BTreeMap::from([
        (
            ids.commander,
            CommanderProfile {
                id: ids.commander,
                profile: canwu_military::CommanderProfileId::new("canwu.military:commander:field")?,
                organization: 700,
                reconnaissance: 650,
                logistics: 600,
                tactics: 720,
                political: 500,
                obedience: 800,
            },
        ),
        (
            ids.observer,
            CommanderProfile {
                id: ids.observer,
                profile: canwu_military::CommanderProfileId::new(
                    "canwu.military:commander:defender",
                )?,
                organization: 600,
                reconnaissance: 500,
                logistics: 450,
                tactics: 580,
                political: 400,
                obedience: 700,
            },
        ),
    ]);
    let catalog = MilitaryCatalog {
        meta: MilitaryRecordMeta::new(1, scenario.start_time, &())?,
        ruleset,
        nodes,
        commanders,
    };
    scenario.domain_records.push(record_from(
        catalog_reference(),
        &catalog,
        scenario.start_time,
    )?);
    Ok((scenario, ids))
}

#[must_use]
pub fn plugins() -> (ReferenceWorldPlugin, MilitaryPlugin) {
    (ReferenceWorldPlugin, MilitaryPlugin)
}

#[must_use]
pub fn ruleset_profiles() -> (MilitaryRulesetV1, MilitaryRulesetV1) {
    (
        canwu_military_reference_content::riverine_preindustrial(),
        canwu_military_reference_content::industrial_front(),
    )
}
