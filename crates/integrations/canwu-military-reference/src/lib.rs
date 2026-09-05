//! Public-API-only composition of the military extension and reference world.

#![allow(clippy::missing_errors_doc, clippy::module_name_repetitions)]

use canwu_api::{CanwuError, Scenario};
use canwu_military::{MilitaryPlugin, MilitaryRulesetV1};
use canwu_reference_world::{ReferenceWorldPlugin, demo_scenario};

pub const PLUGIN_NAME: &str = "canwu-military-reference";

pub fn demo_military_scenario()
-> Result<(Scenario, canwu_reference_world::ReferenceWorldIds), CanwuError> {
    demo_scenario()
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
