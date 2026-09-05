//! Versioned, replaceable military content profiles.

#![allow(
    clippy::missing_errors_doc,
    clippy::must_use_candidate,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]

use blake3::Hasher;
use canwu_api::{CanwuError, SimTime, canonical_hash};
use canwu_military::{
    BranchProfileV1, CombatProfileV1, MilitaryRulesetV1, OccupationProfileV1, RecruitmentProfileV1,
    RulesetId, TacticProfileV1, TerrainModifierV1,
};
use serde::Serialize;
use std::collections::BTreeMap;

pub const PLUGIN_NAME: &str = "canwu-military-reference-content";

pub fn riverine_preindustrial() -> MilitaryRulesetV1 {
    build_ruleset(
        "canwu:military:ruleset:riverine-preindustrial:v1",
        "riverine_preindustrial",
        "synthetic_reference",
        "Synthetic preindustrial river-basin warfare profile; intended for deterministic reference simulations.",
        vec![
            (
                "levy_infantry",
                BranchProfileV1 {
                    branch: "levy_infantry".into(),
                    training_per_day: 8,
                    equipment_per_mille: 420,
                    supply_per_day: 90,
                    movement_minutes: 180,
                },
            ),
            (
                "river_marines",
                BranchProfileV1 {
                    branch: "river_marines".into(),
                    training_per_day: 12,
                    equipment_per_mille: 610,
                    supply_per_day: 110,
                    movement_minutes: 150,
                },
            ),
            (
                "horse_scouts",
                BranchProfileV1 {
                    branch: "horse_scouts".into(),
                    training_per_day: 14,
                    equipment_per_mille: 520,
                    supply_per_day: 130,
                    movement_minutes: 105,
                },
            ),
        ],
        vec![
            TacticProfileV1 {
                id: "river_ambush".into(),
                attack_per_mille: 80,
                defense_per_mille: 40,
                concealment_per_mille: 260,
                withdrawal_threshold_per_mille: 280,
            },
            TacticProfileV1 {
                id: "levee_defense".into(),
                attack_per_mille: -20,
                defense_per_mille: 180,
                concealment_per_mille: 90,
                withdrawal_threshold_per_mille: 180,
            },
            TacticProfileV1 {
                id: "crossing_assault".into(),
                attack_per_mille: 120,
                defense_per_mille: -100,
                concealment_per_mille: 20,
                withdrawal_threshold_per_mille: 420,
            },
        ],
        vec![
            TerrainModifierV1 {
                id: "river".into(),
                movement_per_mille: 125,
                attack_per_mille: -80,
                defense_per_mille: 40,
                concealment_per_mille: 140,
            },
            TerrainModifierV1 {
                id: "floodplain".into(),
                movement_per_mille: 145,
                attack_per_mille: -50,
                defense_per_mille: 20,
                concealment_per_mille: 90,
            },
            TerrainModifierV1 {
                id: "levee".into(),
                movement_per_mille: 90,
                attack_per_mille: 30,
                defense_per_mille: 160,
                concealment_per_mille: 40,
            },
            TerrainModifierV1 {
                id: "market_town".into(),
                movement_per_mille: 100,
                attack_per_mille: -10,
                defense_per_mille: 70,
                concealment_per_mille: 30,
            },
        ],
        RecruitmentProfileV1 {
            training_days: 45,
            minimum_age: 16,
            maximum_age: 42,
            replacement_delay_days: 18,
        },
        CombatProfileV1 {
            max_rounds: 8,
            casualty_per_mille: 65,
            prisoner_per_mille: 35,
            fatigue_per_round: 55,
            morale_break_per_mille: 260,
        },
        OccupationProfileV1 {
            garrison_per_node: 180,
            security_per_day: 24,
            integration_per_day: 8,
            max_resistance_per_mille: 900,
        },
    )
}

pub fn industrial_front() -> MilitaryRulesetV1 {
    build_ruleset(
        "canwu:military:ruleset:industrial-front:v1",
        "industrial_front",
        "synthetic_reference",
        "Synthetic industrial-era front warfare profile; intended for deterministic reference simulations.",
        vec![
            (
                "line_infantry",
                BranchProfileV1 {
                    branch: "line_infantry".into(),
                    training_per_day: 16,
                    equipment_per_mille: 760,
                    supply_per_day: 180,
                    movement_minutes: 240,
                },
            ),
            (
                "armored",
                BranchProfileV1 {
                    branch: "armored".into(),
                    training_per_day: 22,
                    equipment_per_mille: 880,
                    supply_per_day: 420,
                    movement_minutes: 150,
                },
            ),
            (
                "artillery",
                BranchProfileV1 {
                    branch: "artillery".into(),
                    training_per_day: 20,
                    equipment_per_mille: 840,
                    supply_per_day: 360,
                    movement_minutes: 210,
                },
            ),
            (
                "engineers",
                BranchProfileV1 {
                    branch: "engineers".into(),
                    training_per_day: 18,
                    equipment_per_mille: 800,
                    supply_per_day: 220,
                    movement_minutes: 195,
                },
            ),
        ],
        vec![
            TacticProfileV1 {
                id: "trench_defense".into(),
                attack_per_mille: -40,
                defense_per_mille: 240,
                concealment_per_mille: 170,
                withdrawal_threshold_per_mille: 160,
            },
            TacticProfileV1 {
                id: "combined_arms".into(),
                attack_per_mille: 180,
                defense_per_mille: 70,
                concealment_per_mille: 50,
                withdrawal_threshold_per_mille: 300,
            },
            TacticProfileV1 {
                id: "deep_breakthrough".into(),
                attack_per_mille: 260,
                defense_per_mille: -90,
                concealment_per_mille: 20,
                withdrawal_threshold_per_mille: 430,
            },
            TacticProfileV1 {
                id: "elastic_withdrawal".into(),
                attack_per_mille: -70,
                defense_per_mille: 120,
                concealment_per_mille: 210,
                withdrawal_threshold_per_mille: 100,
            },
        ],
        vec![
            TerrainModifierV1 {
                id: "trenches".into(),
                movement_per_mille: 155,
                attack_per_mille: -80,
                defense_per_mille: 260,
                concealment_per_mille: 150,
            },
            TerrainModifierV1 {
                id: "rail_corridor".into(),
                movement_per_mille: 85,
                attack_per_mille: 20,
                defense_per_mille: 20,
                concealment_per_mille: 10,
            },
            TerrainModifierV1 {
                id: "industrial_city".into(),
                movement_per_mille: 135,
                attack_per_mille: -30,
                defense_per_mille: 180,
                concealment_per_mille: 180,
            },
            TerrainModifierV1 {
                id: "open_steppe".into(),
                movement_per_mille: 100,
                attack_per_mille: 40,
                defense_per_mille: -40,
                concealment_per_mille: 20,
            },
            TerrainModifierV1 {
                id: "fortress".into(),
                movement_per_mille: 175,
                attack_per_mille: -160,
                defense_per_mille: 340,
                concealment_per_mille: 80,
            },
        ],
        RecruitmentProfileV1 {
            training_days: 90,
            minimum_age: 18,
            maximum_age: 45,
            replacement_delay_days: 30,
        },
        CombatProfileV1 {
            max_rounds: 14,
            casualty_per_mille: 48,
            prisoner_per_mille: 22,
            fatigue_per_round: 42,
            morale_break_per_mille: 210,
        },
        OccupationProfileV1 {
            garrison_per_node: 320,
            security_per_day: 32,
            integration_per_day: 12,
            max_resistance_per_mille: 950,
        },
    )
}

fn build_ruleset(
    id: &str,
    profile: &str,
    source_kind: &str,
    source_note: &str,
    branches: Vec<(&str, BranchProfileV1)>,
    tactics: Vec<TacticProfileV1>,
    terrain: Vec<TerrainModifierV1>,
    recruitment: RecruitmentProfileV1,
    combat: CombatProfileV1,
    occupation: OccupationProfileV1,
) -> MilitaryRulesetV1 {
    let id = RulesetId::new(id).expect("reference ruleset IDs are valid");
    let branch_count = branches.len();
    let tactic_count = tactics.len();
    let terrain_count = terrain.len();
    let branch_profiles = branches
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect::<BTreeMap<_, _>>();
    let tactics = tactics
        .into_iter()
        .map(|value| (value.id.clone(), value))
        .collect::<BTreeMap<_, _>>();
    let terrain_modifiers = terrain
        .into_iter()
        .map(|value| (value.id.clone(), value))
        .collect::<BTreeMap<_, _>>();
    let semantic_hash = canonical_hash(
        "canwu.military.ruleset.v1",
        &RulesetDigest {
            id: &id,
            profile,
            branch_count,
            tactic_count,
            terrain_count,
        },
    )
    .expect("reference ruleset hash is serializable");
    MilitaryRulesetV1 {
        id,
        schema_version: 1,
        profile: profile.into(),
        semantic_hash,
        source_kind: source_kind.into(),
        source_note: source_note.into(),
        branch_profiles,
        tactics,
        terrain_modifiers,
        recruitment,
        combat,
        occupation,
    }
}

#[derive(Serialize)]
struct RulesetDigest<'a> {
    id: &'a RulesetId,
    profile: &'a str,
    branch_count: usize,
    tactic_count: usize,
    terrain_count: usize,
}

pub fn validate_ruleset(ruleset: &MilitaryRulesetV1) -> Result<(), CanwuError> {
    ruleset.validate()
}

pub fn ruleset_hash<T: Serialize>(value: &T) -> Result<String, CanwuError> {
    canonical_hash("canwu.military.reference-content.v1", value)
}

#[allow(dead_code)]
fn _keep_dependencies_visible(_: &Hasher, _: SimTime) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_profiles_validate_and_are_stable() {
        let riverine = riverine_preindustrial();
        let industrial = industrial_front();
        riverine.validate().unwrap();
        industrial.validate().unwrap();
        assert_ne!(riverine.id, industrial.id);
        assert_eq!(
            riverine.semantic_hash,
            riverine_preindustrial().semantic_hash
        );
        assert_eq!(industrial.semantic_hash, industrial_front().semantic_hash);
        assert_eq!(riverine.source_kind, "synthetic_reference");
        assert_eq!(industrial.source_kind, "synthetic_reference");
    }
}
