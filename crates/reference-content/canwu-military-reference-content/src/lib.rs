//! One explicitly synthetic combat rulebook for the engagement engine's
//! reference integration. Its numbers are demonstration values, not a
//! historical dataset or a claim about any army.

#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::too_many_lines
)]

use canwu_api::{CanwuError, Issuer, canonical_hash};
use canwu_military::{
    BreakRuleV1, ChannelId, CohesionRuleV1, CombatLimitsV1, CombatRulebookV1, ConcealmentClassId,
    CrossingRulesV1, EngagementKind, EnvironmentTermV1, ForecastBandId, ForecastRulesV1,
    FortGradeV1, FortState, FortUse, GroundClassId, InterceptMode, InterceptRuleV1, KindRuleV1,
    LeadershipRuleV1, LineRulesV1, ManpowerBandV1, MoraleRuleV1, NightFactorsV1,
    ParametricStepResolver, PostureId, PostureModifierV1, ProfileId, PursuitRuleV1, RatioBandV1,
    RulebookId, SCHEMA_VERSION, SiegeRulesV1, StepProfileV1, StepResolver, StrengthTermV1,
    SupplyFireRuleV1, TermId, TermSource, TerrainClassId, TerrainFitRowV1, TerrainRowV1, TraitId,
};
use serde::Serialize;
use std::collections::BTreeMap;

pub const PLUGIN_NAME: &str = "canwu-military-reference-content";
/// The synthetic rulebook's identity.
pub const SYNTHETIC_RULEBOOK_ID: &str = "canwu:military:rulebook:synthetic:v1";
/// The issuer the synthetic rulebook trusts (the reference host).
pub const REFERENCE_HOST_ISSUER: &str = "canwu-military-reference";

fn id<T>(make: fn(String) -> Result<T, CanwuError>, kind: &str, name: &str) -> T {
    make(format!("canwu:military:{kind}:{name}"))
        .unwrap_or_else(|error| unreachable!("a static reference id is valid: {error}"))
}

/// A terrain class of the synthetic rulebook.
#[must_use]
pub fn terrain(name: &str) -> TerrainClassId {
    id(TerrainClassId::new, "terrain", name)
}

/// A posture of the synthetic rulebook (`field`, `camp`, `march`, `defend`).
#[must_use]
pub fn posture(name: &str) -> PostureId {
    id(PostureId::new, "posture", name)
}

/// A ground class of the synthetic rulebook (`firm`, `soft`, `frozen`, `mud`).
#[must_use]
pub fn ground(name: &str) -> GroundClassId {
    id(GroundClassId::new, "ground", name)
}

/// A concealment class of the synthetic rulebook (`clear`, `fog`).
#[must_use]
pub fn concealment(name: &str) -> ConcealmentClassId {
    id(ConcealmentClassId::new, "concealment", name)
}

/// A trait of the synthetic rulebook (`mounted-only`, `foot-only`).
#[must_use]
pub fn side_trait(name: &str) -> TraitId {
    id(TraitId::new, "trait", name)
}

/// The day profile (one compressed step a day).
#[must_use]
pub fn day_profile() -> ProfileId {
    id(ProfileId::new, "profile", "day")
}

/// The half-hour profile (a promoted fight).
#[must_use]
pub fn half_hour_profile() -> ProfileId {
    id(ProfileId::new, "profile", "half-hour")
}

/// The two condition channels the synthetic rulebook declares, in order.
#[must_use]
pub fn condition_channels() -> Vec<ChannelId> {
    vec![
        id(ChannelId::new, "channel", "information"),
        id(ChannelId::new, "channel", "terrain-fit"),
    ]
}

fn term(source: TermSource, name: &str, divisor: u32, cap: i64) -> StrengthTermV1 {
    StrengthTermV1 {
        id: id(TermId::new, "term", name),
        source,
        divisor,
        cap,
    }
}

fn row(
    frontage_bp: u32,
    melee_bp: u32,
    shock_bp: u32,
    defense_bp: u32,
    charge_window_bp: u32,
    pursuit_bp: u32,
) -> TerrainRowV1 {
    TerrainRowV1 {
        frontage_bp,
        melee_bp,
        shock_bp,
        defense_bp,
        charge_window_bp,
        pursuit_bp,
    }
}

fn kind(
    frontage_bp: Option<u32>,
    days: u32,
    terrain_class: Option<TerrainClassId>,
    fort: FortUse,
    posture_bp: Vec<PostureModifierV1>,
) -> KindRuleV1 {
    KindRuleV1 {
        frontage_bp,
        max_minutes: days * 1_440,
        terrain: terrain_class,
        fort,
        posture_bp,
        posture_steps: 1,
    }
}

fn cell(attacker: &str, defender: &str, fort: FortState, bp: u32) -> PostureModifierV1 {
    PostureModifierV1 {
        attacker: posture(attacker),
        defender: posture(defender),
        fort,
        bp,
    }
}

/// The synthetic rulebook for the parametric resolver: six terrain rows,
/// five fortification grades, five band cuts, the `field` / `camp` /
/// `march` / `defend` postures with a few modifiers, the `day` and
/// `half-hour` step profiles, the six kind rules, and the reference limits.
/// Its trusted issuer is `Issuer::System("canwu-military-reference")`.
#[must_use]
pub fn synthetic_rulebook_v1() -> CombatRulebookV1 {
    let band = |name: &str| id(ForecastBandId::new, "band", name);
    let field_cells = vec![
        cell("field", "march", FortState::Any, 13_000),
        cell("field", "camp", FortState::Any, 11_000),
    ];
    let assault_cells = vec![cell("field", "defend", FortState::Open, 12_000)];
    let rulebook = CombatRulebookV1 {
        id: RulebookId::new(SYNTHETIC_RULEBOOK_ID)
            .unwrap_or_else(|error| unreachable!("the rulebook id is valid: {error}")),
        schema_version: SCHEMA_VERSION,
        rules_hash: String::new(),
        resolver: ParametricStepResolver::IDENTITY.owned(),
        trusted_issuers: vec![Issuer::System(REFERENCE_HOST_ISSUER.to_owned())],
        step_profiles: BTreeMap::from([
            (
                day_profile(),
                StepProfileV1 {
                    step_minutes: 1_440,
                    step_scale_bp: 230_400,
                    max_steps_per_boundary: 4,
                },
            ),
            (
                half_hour_profile(),
                StepProfileV1 {
                    step_minutes: 30,
                    step_scale_bp: 10_000,
                    max_steps_per_boundary: 48,
                },
            ),
        ]),
        default_profile: day_profile(),
        kinds: BTreeMap::from([
            (
                EngagementKind::Field,
                kind(None, 10, Some(terrain("open")), FortUse::None, field_cells),
            ),
            (
                EngagementKind::Assault,
                kind(Some(4_000), 5, None, FortUse::Site, assault_cells),
            ),
            (
                EngagementKind::Sortie,
                kind(Some(3_000), 1, None, FortUse::None, Vec::new()),
            ),
            (
                EngagementKind::Relief,
                kind(None, 5, Some(terrain("open")), FortUse::None, Vec::new()),
            ),
            (
                EngagementKind::FerryFight,
                kind(Some(2_000), 1, None, FortUse::None, Vec::new()),
            ),
            (
                EngagementKind::BridgeheadFight,
                kind(None, 1, None, FortUse::None, Vec::new()),
            ),
        ]),
        condition_channels: condition_channels(),
        band_cuts: vec![800, 600, 400, 200, 100],
        forecast: ForecastRulesV1 {
            terms: vec![
                term(TermSource::Manpower, "manpower", 1, 30),
                term(TermSource::Readiness, "readiness", 50, 20),
                term(TermSource::Supply, "supply", 100, 10),
                term(TermSource::Morale, "morale", 100, 10),
                term(TermSource::Cohesion, "cohesion", 100, 10),
                term(TermSource::Channel { index: 0 }, "information", 100, 10),
                term(TermSource::Training, "training", 1_000, 10),
                term(TermSource::Equipment, "equipment", 2_000, 5),
                term(TermSource::Command, "command", 100, 10),
                term(TermSource::TerrainFit, "terrain-fit", 1, 10),
                term(TermSource::Environment, "environment", 1, 5),
            ],
            manpower_bands: [
                (0, 5),
                (500, 10),
                (1_000, 14),
                (2_000, 18),
                (4_000, 22),
                (8_000, 26),
                (16_000, 30),
            ]
            .into_iter()
            .map(|(from_men, value)| ManpowerBandV1 { from_men, value })
            .collect(),
            terrain_fit: vec![
                TerrainFitRowV1 {
                    terrain: terrain("open"),
                    trait_: Some(side_trait("mounted-only")),
                    defending: None,
                    delta: 10,
                },
                TerrainFitRowV1 {
                    terrain: terrain("hills"),
                    trait_: Some(side_trait("mounted-only")),
                    defending: None,
                    delta: -10,
                },
                TerrainFitRowV1 {
                    terrain: terrain("hills"),
                    trait_: None,
                    defending: Some(true),
                    delta: 5,
                },
                TerrainFitRowV1 {
                    terrain: terrain("wetland"),
                    trait_: None,
                    defending: Some(true),
                    delta: 5,
                },
            ],
            environment_terms: vec![
                EnvironmentTermV1 {
                    ground: Some(ground("mud")),
                    ranged_reliability_below_bp: None,
                    trait_: Some(side_trait("mounted-only")),
                    ranged_share_from_per_mille: None,
                    delta: -5,
                },
                EnvironmentTermV1 {
                    ground: None,
                    ranged_reliability_below_bp: Some(10_000),
                    trait_: None,
                    ranged_share_from_per_mille: Some(250),
                    delta: -2,
                },
            ],
            ratio_bands: vec![
                RatioBandV1 {
                    min_ratio_bp: 15_000,
                    fortified: None,
                    band: band("advance"),
                },
                RatioBandV1 {
                    min_ratio_bp: 11_000,
                    fortified: Some(true),
                    band: band("hold"),
                },
                RatioBandV1 {
                    min_ratio_bp: 11_000,
                    fortified: None,
                    band: band("advance"),
                },
                RatioBandV1 {
                    min_ratio_bp: 9_000,
                    fortified: None,
                    band: band("stall"),
                },
                RatioBandV1 {
                    min_ratio_bp: 6_000,
                    fortified: None,
                    band: band("withdraw"),
                },
                RatioBandV1 {
                    min_ratio_bp: 0,
                    fortified: None,
                    band: band("rout-possible"),
                },
            ],
        },
        lines: LineRulesV1 {
            fire_weight: 3,
            shock_weight: 4,
            melee_weight: 3,
            defense_offset: 200,
            inflicted_cap_per_mille: 30,
            killed_numerator: 1,
            killed_denominator: 3,
            supply_fire: SupplyFireRuleV1 {
                below_per_mille: 500,
                bp: 7_000,
            },
            leadership: LeadershipRuleV1 {
                base_bp: 8_000,
                command_weight: 20,
                pick_weight: 10,
                min_bp: 6_000,
                max_bp: 12_000,
            },
        },
        cohesion: CohesionRuleV1 {
            per_inflicted: 4,
            base: 1,
            confusion: 50,
            recover_per_day: 10,
        },
        morale: MoraleRuleV1 {
            sensitivity_default: 10,
            credibility_divisor: 40,
            shock_recover_per_day: 20,
        },
        break_rule: BreakRuleV1 {
            rout_cohesion_below: 200,
            rout_morale_margin: 150,
            walled_holds_from_grade: 2,
        },
        pursuit: PursuitRuleV1 {
            captured_per_mille_at_full: 300,
            wounded_left_per_mille: 500,
        },
        terrain: BTreeMap::from([
            (
                terrain("open"),
                row(3_000, 10_000, 12_000, 10_000, 10_000, 10_000),
            ),
            (
                terrain("hills"),
                row(2_000, 9_000, 6_000, 12_000, 6_000, 6_000),
            ),
            (
                terrain("wetland"),
                row(2_000, 8_000, 4_000, 11_000, 4_000, 5_000),
            ),
            (
                terrain("walls"),
                row(2_000, 9_000, 3_000, 12_000, 5_000, 5_000),
            ),
            (
                terrain("breach"),
                row(3_000, 10_000, 5_000, 10_000, 5_000, 8_000),
            ),
            (
                terrain("river"),
                row(2_000, 8_000, 2_000, 10_000, 3_000, 6_000),
            ),
        ]),
        night: NightFactorsV1 {
            fire_bp: 5_000,
            shock_bp: 3_000,
            melee_bp: 7_000,
            pursuit_bp: 3_000,
        },
        fortification: [
            (0, 10_000, 100),
            (1, 15_000, 300),
            (2, 20_000, 600),
            (3, 30_000, 1_000),
            (4, 40_000, 1_500),
        ]
        .into_iter()
        .map(|(grade, fort_bp, breach_threshold)| FortGradeV1 {
            grade,
            fort_bp,
            breach_threshold,
        })
        .collect(),
        postures: ["field", "camp", "march", "defend"]
            .into_iter()
            .map(posture)
            .collect(),
        siege: SiegeRulesV1 {
            day_minutes: 1_440,
            men_per_access_sealed: 500,
            emplacement_days: BTreeMap::from([
                (ground("firm"), 1),
                (ground("frozen"), 1),
                (ground("soft"), 2),
                (ground("mud"), 2),
            ]),
            sortie_cooldown_minutes: 2 * 1_440,
            water_access_control_bp: 6_000,
            assault_terrain: terrain("walls"),
            breach_terrain: terrain("breach"),
            sortie_terrain: terrain("open"),
        },
        crossing: CrossingRulesV1 {
            wave_minutes: 1_440,
            wave_men: 1_000,
            hard_river_wave_bp: 5_000,
            bridgehead_secure_men: 1_500,
            terrain: terrain("river"),
            intercept: InterceptRuleV1 {
                min_control_bp: 4_000,
                control_weight: 10_000,
                scout_weight: 200,
                concealment: BTreeMap::from([
                    (concealment("clear"), 0),
                    (concealment("fog"), 5_000),
                ]),
                night_concealment: 20_000,
                threshold_bp: 5_000,
                mode: InterceptMode::Threshold,
            },
        },
        limits: CombatLimitsV1 {
            forces: 4_096,
            components_per_force: 64,
            forces_per_side: 16,
            controllers_per_force: 8,
            trusted_issuers: 8,
            holders_per_side: 8,
            open_engagements: 8,
            open_sieges: 8,
            open_crossings: 4,
            step_bytes: 2_048,
            draws_per_step: 64,
            retire_after_minutes: 43_200,
            evidence_refs_per_input: 8,
            report_queue: 64,
        },
    };
    rulebook
        .sealed()
        .unwrap_or_else(|error| unreachable!("the synthetic rulebook encodes: {error}"))
}

/// Checks a rulebook: its shape, references, limits, and hash.
pub fn validate(rulebook: &CombatRulebookV1) -> Result<(), CanwuError> {
    rulebook.validate()
}

/// The canonical hash of any reference content value.
pub fn content_hash<T: Serialize>(value: &T) -> Result<String, CanwuError> {
    canonical_hash("canwu.military.reference-content.v1", value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_synthetic_rulebook_validates_and_is_stable() {
        let rulebook = synthetic_rulebook_v1();
        validate(&rulebook).unwrap();
        assert_eq!(rulebook.rules_hash, synthetic_rulebook_v1().rules_hash);
        assert!(ParametricStepResolver::IDENTITY.matches(&rulebook.resolver));
        let encoded = serde_json::to_value(&rulebook).unwrap();
        let decoded: CombatRulebookV1 = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded, rulebook);
    }
}
