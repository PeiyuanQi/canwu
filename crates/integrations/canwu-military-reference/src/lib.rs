//! Public-API-only composition of the engagement engine and the reference
//! world: the synthetic rulebook and a few forces mirrored onto the
//! reference world's places, plus the input helpers the starter uses.

#![allow(clippy::missing_errors_doc, clippy::module_name_repetitions)]

use canwu_api::{CanwuError, EntityRef, GovernmentId, Issuer, Scenario, TerritoryId};
use canwu_military::{
    BattleStateV1, ComponentId, ComponentStateV1, ComponentValuesV1, ConditionInputV1,
    EnvironmentInputV1, ForceId, ForceStateV1, ForceStatus, LocationV1, LossTallyV1,
    MilitaryPlugin, ParametricStepResolver, RiverBand, SCHEMA_VERSION, SideTermsV1,
    scenario_records,
};
use canwu_military_reference_content::{
    REFERENCE_HOST_ISSUER, concealment, ground, posture, synthetic_rulebook_v1,
};
use canwu_reference_world::{ReferenceWorldIds, ReferenceWorldPlugin, demo_scenario};
use std::collections::BTreeSet;

pub const PLUGIN_NAME: &str = "canwu-military-reference";

/// The reference host's issuer, which the synthetic rulebook trusts.
#[must_use]
pub fn reference_host() -> Issuer {
    Issuer::System(REFERENCE_HOST_ISSUER.to_owned())
}

/// The rival owner the scenario adds beside the reference world's state.
#[must_use]
pub const fn rival_government() -> GovernmentId {
    GovernmentId::new(2)
}

/// A force of the demonstration (`yun-field`, `rival-field`, `yun-siege`,
/// `rival-garrison`, `yun-river`, `rival-fleet`).
#[must_use]
pub fn force_id(name: &str) -> ForceId {
    ForceId::new(format!("canwu:military:force:{name}"))
        .unwrap_or_else(|error| unreachable!("a demonstration force id is valid: {error}"))
}

fn values(kind: &str) -> ComponentValuesV1 {
    match kind {
        "cavalry" => ComponentValuesV1 {
            melee: 800,
            shock: 1_500,
            defense: 400,
            training_bp: 6_000,
            equipment_bp: 5_000,
            mounted: true,
            ..ComponentValuesV1::default()
        },
        "guns" => ComponentValuesV1 {
            fire: 200,
            defense: 100,
            training_bp: 5_000,
            breach_per_piece: 10,
            crew_per_piece_x10: 100,
            ranged: true,
            ..ComponentValuesV1::default()
        },
        "vessels" => ComponentValuesV1 {
            fire: 600,
            melee: 600,
            defense: 900,
            training_bp: 7_000,
            ranged: true,
            vessel: true,
            ..ComponentValuesV1::default()
        },
        _ => ComponentValuesV1 {
            fire: 400,
            melee: 1_000,
            defense: 800,
            training_bp: 6_000,
            equipment_bp: 5_000,
            ranged: true,
            ..ComponentValuesV1::default()
        },
    }
}

fn force(
    name: &str,
    owner: GovernmentId,
    controllers: Vec<Issuer>,
    territory: TerritoryId,
    components: &[(&str, u64)],
) -> ForceStateV1 {
    ForceStateV1 {
        schema_version: SCHEMA_VERSION,
        id: force_id(name),
        owner: EntityRef::Government(owner),
        controllers,
        host_revision: 0,
        location: LocationV1::At {
            site: EntityRef::Territory(territory),
        },
        components: components
            .iter()
            .map(|(kind, present)| {
                (
                    ComponentId::new(format!("canwu:military:component:{name}:{kind}"))
                        .unwrap_or_else(|error| unreachable!("a component id is valid: {error}")),
                    ComponentStateV1 {
                        present: *present,
                        values: values(kind),
                        losses: LossTallyV1::default(),
                    },
                )
            })
            .collect(),
        terms: SideTermsV1 {
            cohesion_cap: 800,
            morale_break: 400,
            morale_sensitivity: 10,
            battle_command: 100,
            assessment: 100,
            credibility: 20,
            traits: BTreeSet::new(),
        },
        battle: BattleStateV1::default(),
        status: ForceStatus::Active,
    }
}

/// The reference world's demonstration scenario with the synthetic rulebook
/// and six forces installed: the field army's mirror at the capital, a
/// rival field force in the east, a siege pair in the west, and a river
/// force facing a rival fleet at the capital. The rival government is added
/// as an entity so it can own forces and hold reports.
pub fn demo_engagement_scenario() -> Result<(Scenario, ReferenceWorldIds), CanwuError> {
    let (mut scenario, ids) = demo_scenario()?;
    scenario
        .entities
        .push(EntityRef::Government(rival_government()));
    let yun = vec![Issuer::Actor(ids.commander), reference_host()];
    let rival = vec![reference_host()];
    let forces = vec![
        force(
            "yun-field",
            ids.government,
            yun.clone(),
            ids.central_territory,
            &[("infantry", 6_400), ("cavalry", 1_200), ("guns", 400)],
        ),
        force(
            "rival-field",
            rival_government(),
            rival.clone(),
            ids.eastern_territory,
            &[("infantry", 4_000), ("cavalry", 400)],
        ),
        force(
            "yun-siege",
            ids.government,
            yun.clone(),
            ids.western_territory,
            &[("infantry", 3_000), ("guns", 400)],
        ),
        force(
            "rival-garrison",
            rival_government(),
            rival.clone(),
            ids.western_territory,
            &[("infantry", 700)],
        ),
        force(
            "yun-river",
            ids.government,
            yun,
            ids.central_territory,
            &[("infantry", 2_000)],
        ),
        force(
            "rival-fleet",
            rival_government(),
            rival,
            ids.central_territory,
            &[("vessels", 300)],
        ),
    ];
    scenario
        .domain_records
        .extend(scenario_records(&synthetic_rulebook_v1(), &forces)?);
    Ok((scenario, ids))
}

/// The reference world and the engagement engine (parametric resolver, the
/// daily wake cadence).
#[must_use]
pub fn plugins() -> (ReferenceWorldPlugin, MilitaryPlugin<ParametricStepResolver>) {
    (ReferenceWorldPlugin, MilitaryPlugin::new())
}

/// A condition input in the synthetic rulebook's terms.
#[must_use]
pub fn condition(posture_name: &str, morale_per_mille: u16) -> ConditionInputV1 {
    ConditionInputV1 {
        channels: vec![600, 500],
        supply_per_mille: 900,
        morale_per_mille,
        readiness_per_mille: 800,
        fatigue_per_mille: 100,
        posture: posture(posture_name),
        surprise_bp: 0,
        order_confusion: 0,
        defense_halved: false,
        evidence: Vec::new(),
    }
}

/// A firm-ground, clear environment input, by day or by night.
#[must_use]
pub fn environment(night: bool) -> EnvironmentInputV1 {
    EnvironmentInputV1 {
        terrain: None,
        ground: ground("firm"),
        concealment: concealment("clear"),
        night,
        ranged_reliability_bp: 10_000,
        river: RiverBand::Open,
        evidence: Vec::new(),
    }
}
