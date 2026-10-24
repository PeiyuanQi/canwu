//! The combat rulebook: every number that decides a fight, installed by the
//! host, versioned and hashed, validated at activation and read at every
//! step.

use crate::ids::{
    ChannelId, ConcealmentClassId, ForecastBandId, GroundClassId, PostureId, ProfileId, RulebookId,
    SCHEMA_VERSION, TermId, TerrainClassId, TraitId, invalid,
};
use canwu_api::{CanwuError, Issuer, canonical_hash};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The hard ceilings the plugin holds every rulebook's limits to.
pub const MAX_FORCES: u32 = 16_384;
pub const MAX_STEPS_PER_BOUNDARY: u32 = 96;
pub const MAX_STEP_BYTES: u32 = 4_096;
pub const MAX_DRAWS_PER_STEP: u32 = 256;
pub const MAX_COMPONENTS_PER_FORCE: u32 = 256;
pub const MAX_FORCES_PER_SIDE: u32 = 64;
pub const MAX_LISTED: u32 = 64;
pub const MAX_OPEN_WORK: u32 = 64;
pub const MAX_REPORT_QUEUE: u32 = 1_024;
/// The most siege days or crossing waves one boundary advances a siege or a
/// crossing by; the rest waits for the next stepping boundary.
pub const MAX_DAYS_PER_BOUNDARY: u32 = 32;

/// The most men one component may have present.
pub const MAX_COMPONENT_MEN: u64 = 1_000_000_000;
/// The largest per-man component value (fire, melee, shock, defense, breach
/// per piece, crew per piece) and the largest side term.
pub const MAX_VALUE: u32 = 100_000;
/// The largest basis-point input or parameter (10×).
pub const MAX_INPUT_BP: u32 = 100_000;
/// The largest step scale (a profile may compress up to 1,000 base steps).
pub const MAX_STEP_SCALE_BP: u32 = 10_000_000;
/// The largest weight of a line or a leadership term.
pub const MAX_WEIGHT: u32 = 1_000;
/// The largest magnitude of a forecast cap, delta, or manpower-band value.
pub const MAX_FORECAST_TERM: i64 = 1_000_000;

/// A value out of the plugin's bounds: refused, never computed with.
pub(crate) fn out_of_range(message: impl Into<String>) -> CanwuError {
    CanwuError::new(canwu_api::ErrorCode::ValueOutOfRange, message.into())
}

/// Refuses `value` above `ceiling`.
pub(crate) fn bounded<T: PartialOrd + std::fmt::Display>(
    value: T,
    ceiling: T,
    label: &str,
) -> Result<(), CanwuError> {
    if value > ceiling {
        return Err(out_of_range(format!("{label} {value} exceeds {ceiling}")));
    }
    Ok(())
}

const BP: u32 = 10_000;

/// The owned identity of a step resolver, as records keep it.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ResolverIdentityV1 {
    pub name: String,
    pub version: u32,
}

/// The kind of an engagement; each has a rule in the rulebook.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EngagementKind {
    Field,
    Assault,
    Sortie,
    Relief,
    FerryFight,
    BridgeheadFight,
}

impl EngagementKind {
    pub const ALL: [Self; 6] = [
        Self::Field,
        Self::Assault,
        Self::Sortie,
        Self::Relief,
        Self::FerryFight,
        Self::BridgeheadFight,
    ];

    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Field => "field",
            Self::Assault => "assault",
            Self::Sortie => "sortie",
            Self::Relief => "relief",
            Self::FerryFight => "ferry",
            Self::BridgeheadFight => "bridgehead",
        }
    }
}

/// How long one step of a profile is and how far it compresses the
/// per-step quantities.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StepProfileV1 {
    pub step_minutes: u32,
    pub step_scale_bp: u32,
    pub max_steps_per_boundary: u32,
}

/// How a kind uses the site's fortification.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FortUse {
    None,
    Site,
    SiteHalved,
}

/// The fortification state a posture modifier applies in.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FortState {
    Any,
    Closed,
    Open,
}

/// One cell of a kind's posture matrix: the attacker's inflicted is scaled
/// by `bp` while the step index is below the kind's `posture_steps`. The
/// first matching cell in order wins.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PostureModifierV1 {
    pub attacker: PostureId,
    pub defender: PostureId,
    pub fort: FortState,
    pub bp: u32,
}

/// The rule of one engagement kind.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KindRuleV1 {
    /// The frontage; `None` takes the terrain row's.
    pub frontage_bp: Option<u32>,
    /// A fight that has not broken after this many minutes closes `Stalled`.
    pub max_minutes: u32,
    /// The terrain when neither the engagement nor its environment names one.
    pub terrain: Option<TerrainClassId>,
    pub fort: FortUse,
    pub posture_bp: Vec<PostureModifierV1>,
    /// The steps the posture modifier and the side's surprise apply for.
    pub posture_steps: u32,
}

/// Where a forecast term reads its value.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TermSource {
    /// The side's men through `manpower_bands`.
    Manpower,
    Readiness,
    Supply,
    /// The side's morale: the host's morale less the battle's shock.
    Morale,
    /// The side's cohesion.
    Cohesion,
    /// One host condition channel, by its index in `condition_channels`.
    Channel {
        index: u16,
    },
    /// The men-weighted training of the side's components (basis points).
    Training,
    /// The men-weighted equipment of the side's components (basis points).
    Equipment,
    /// The side's battle command.
    Command,
    /// The sum of the matching `terrain_fit` rows, clamped to `±cap`.
    TerrainFit,
    /// The sum of the matching `environment_terms`, clamped to `±cap`.
    Environment,
}

/// One forecast term: `min(cap, floor(value / divisor))`, or a clamped sum
/// for the terrain and environment sources.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StrengthTermV1 {
    pub id: TermId,
    pub source: TermSource,
    pub divisor: u32,
    pub cap: i64,
}

/// A manpower band: from `from_men` men up, the term is `value`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ManpowerBandV1 {
    pub from_men: u64,
    pub value: i64,
}

/// A terrain-fit row: on `terrain`, a side with `trait_` (when named) that
/// is the defender or not (when named) gains `delta`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TerrainFitRowV1 {
    pub terrain: TerrainClassId,
    #[serde(rename = "trait")]
    pub trait_: Option<TraitId>,
    pub defending: Option<bool>,
    pub delta: i64,
}

/// An environment row: every named condition must hold for `delta`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EnvironmentTermV1 {
    pub ground: Option<GroundClassId>,
    pub ranged_reliability_below_bp: Option<u32>,
    #[serde(rename = "trait")]
    pub trait_: Option<TraitId>,
    pub ranged_share_from_per_mille: Option<u16>,
    pub delta: i64,
}

/// A ratio band: the first row whose `min_ratio_bp` the ratio reaches, and
/// whose `fortified` (when named) matches, names the forecast band.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RatioBandV1 {
    pub min_ratio_bp: i64,
    pub fortified: Option<bool>,
    pub band: ForecastBandId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ForecastRulesV1 {
    pub terms: Vec<StrengthTermV1>,
    /// Ascending by `from_men`, the first from 0.
    pub manpower_bands: Vec<ManpowerBandV1>,
    pub terrain_fit: Vec<TerrainFitRowV1>,
    pub environment_terms: Vec<EnvironmentTermV1>,
    /// In order; the last is a catch-all (`min_ratio_bp <= 0`, `fortified: None`).
    pub ratio_bands: Vec<RatioBandV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SupplyFireRuleV1 {
    pub below_per_mille: u16,
    pub bp: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LeadershipRuleV1 {
    pub base_bp: u32,
    pub command_weight: u32,
    pub pick_weight: u32,
    pub min_bp: u32,
    pub max_bp: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LineRulesV1 {
    pub fire_weight: u32,
    pub shock_weight: u32,
    pub melee_weight: u32,
    pub defense_offset: u32,
    pub inflicted_cap_per_mille: u32,
    pub killed_numerator: u32,
    pub killed_denominator: u32,
    pub supply_fire: SupplyFireRuleV1,
    pub leadership: LeadershipRuleV1,
}

/// Cohesion loss a step: `floor((inflicted × per_inflicted + base +
/// order_confusion × confusion) × step_scale_bp / 10^4)`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CohesionRuleV1 {
    pub per_inflicted: u32,
    pub base: u32,
    pub confusion: u32,
    pub recover_per_day: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MoraleRuleV1 {
    /// The sensitivity of a side whose terms give none.
    pub sensitivity_default: u32,
    pub credibility_divisor: u32,
    pub shock_recover_per_day: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BreakRuleV1 {
    pub rout_cohesion_below: u32,
    pub rout_morale_margin: u32,
    /// A defender behind a closed fortification of this grade or more never
    /// breaks.
    pub walled_holds_from_grade: u8,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PursuitRuleV1 {
    pub captured_per_mille_at_full: u32,
    pub wounded_left_per_mille: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TerrainRowV1 {
    pub frontage_bp: u32,
    pub melee_bp: u32,
    pub shock_bp: u32,
    pub defense_bp: u32,
    pub charge_window_bp: u32,
    pub pursuit_bp: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NightFactorsV1 {
    pub fire_bp: u32,
    pub shock_bp: u32,
    pub melee_bp: u32,
    pub pursuit_bp: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FortGradeV1 {
    pub grade: u8,
    pub fort_bp: u32,
    pub breach_threshold: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SiegeRulesV1 {
    /// The rulebook's day: a siege day, and the unit of idle recovery.
    pub day_minutes: u32,
    pub men_per_access_sealed: u32,
    pub emplacement_days: BTreeMap<GroundClassId, u16>,
    pub sortie_cooldown_minutes: u32,
    pub water_access_control_bp: u32,
    pub assault_terrain: TerrainClassId,
    pub breach_terrain: TerrainClassId,
    pub sortie_terrain: TerrainClassId,
}

/// How an interception is decided.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InterceptMode {
    /// Intercepts when `intercept_bp > threshold_bp`.
    Threshold,
    /// Intercepts when `intercept_bp` exceeds a keyed draw below 10^4.
    KeyedDraw,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InterceptRuleV1 {
    pub min_control_bp: u32,
    pub control_weight: u32,
    pub scout_weight: u32,
    pub concealment: BTreeMap<ConcealmentClassId, u32>,
    pub night_concealment: u32,
    pub threshold_bp: u32,
    pub mode: InterceptMode,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CrossingRulesV1 {
    pub wave_minutes: u32,
    pub wave_men: u64,
    pub hard_river_wave_bp: u32,
    pub bridgehead_secure_men: u64,
    pub terrain: TerrainClassId,
    pub intercept: InterceptRuleV1,
}

/// The caps of every collection the plugin keeps.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CombatLimitsV1 {
    pub forces: u32,
    pub components_per_force: u32,
    pub forces_per_side: u32,
    pub controllers_per_force: u32,
    pub trusted_issuers: u32,
    pub holders_per_side: u32,
    pub open_engagements: u32,
    pub open_sieges: u32,
    pub open_crossings: u32,
    /// The encoded bytes of one step.
    pub step_bytes: u32,
    pub draws_per_step: u32,
    pub retire_after_minutes: u64,
    pub evidence_refs_per_input: u32,
    pub report_queue: u32,
}

impl CombatLimitsV1 {
    /// The most report items the queue may hold: the progress cap plus one
    /// closing item per open engagement, siege, and crossing.
    #[must_use]
    pub fn report_queue_bound(&self) -> usize {
        (self.report_queue + self.open_engagements + self.open_sieges + self.open_crossings)
            as usize
    }
}

/// The host's combat rulebook (one per run).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CombatRulebookV1 {
    pub id: RulebookId,
    pub schema_version: u32,
    /// The canonical hash of every other field ([`Self::compute_rules_hash`]).
    pub rules_hash: String,
    pub resolver: ResolverIdentityV1,
    /// Issuers that may issue any command, and the `Step` command.
    pub trusted_issuers: Vec<Issuer>,
    pub step_profiles: BTreeMap<ProfileId, StepProfileV1>,
    pub default_profile: ProfileId,
    pub kinds: BTreeMap<EngagementKind, KindRuleV1>,
    /// Host-declared condition channels; inputs give values in this order.
    pub condition_channels: Vec<ChannelId>,
    /// Band cuts of per-mille values, strictly descending: a value at or
    /// above the cut at index `i` (and below the cut before it) is band `i`;
    /// below every cut it is band `len`.
    pub band_cuts: Vec<u16>,
    pub forecast: ForecastRulesV1,
    pub lines: LineRulesV1,
    pub cohesion: CohesionRuleV1,
    pub morale: MoraleRuleV1,
    pub break_rule: BreakRuleV1,
    pub pursuit: PursuitRuleV1,
    pub terrain: BTreeMap<TerrainClassId, TerrainRowV1>,
    pub night: NightFactorsV1,
    /// Ascending by grade, unique.
    pub fortification: Vec<FortGradeV1>,
    pub postures: Vec<PostureId>,
    pub siege: SiegeRulesV1,
    pub crossing: CrossingRulesV1,
    pub limits: CombatLimitsV1,
}

impl CombatRulebookV1 {
    /// The canonical hash of the rulebook with `rules_hash` left empty.
    pub fn compute_rules_hash(&self) -> Result<String, CanwuError> {
        let mut unsealed = self.clone();
        unsealed.rules_hash = String::new();
        canonical_hash("canwu.military.rulebook.v1", &unsealed)
    }

    /// The rulebook with its `rules_hash` set.
    pub fn sealed(mut self) -> Result<Self, CanwuError> {
        self.rules_hash = self.compute_rules_hash()?;
        Ok(self)
    }

    #[must_use]
    pub fn fort_grade(&self, grade: u8) -> Option<&FortGradeV1> {
        self.fortification.iter().find(|row| row.grade == grade)
    }

    #[must_use]
    pub fn knows_posture(&self, posture: &PostureId) -> bool {
        self.postures.contains(posture)
    }

    /// The band of a per-mille value under `band_cuts`.
    #[must_use]
    pub fn band(&self, per_mille: u64) -> u8 {
        crate::band_of(&self.band_cuts, per_mille)
    }

    /// Checks the rulebook's shape, references, limits against the hard
    /// ceilings, and its hash.
    pub fn validate(&self) -> Result<(), CanwuError> {
        let fail = |message: &str| Err(invalid(format!("combat rulebook: {message}")));
        if self.schema_version != SCHEMA_VERSION {
            return fail("unsupported schema version");
        }
        if self.resolver.name.trim().is_empty() || self.resolver.name.len() > 128 {
            return fail("resolver identity has no name");
        }
        let limits = &self.limits;
        let within = |value: u32, ceiling: u32| value >= 1 && value <= ceiling;
        if !within(limits.forces, MAX_FORCES)
            || !within(limits.components_per_force, MAX_COMPONENTS_PER_FORCE)
            || !within(limits.forces_per_side, MAX_FORCES_PER_SIDE)
            || !within(limits.controllers_per_force, MAX_LISTED)
            || !within(limits.trusted_issuers, MAX_LISTED)
            || !within(limits.holders_per_side, MAX_LISTED)
            || !within(limits.open_engagements, MAX_OPEN_WORK)
            || !within(limits.open_sieges, MAX_OPEN_WORK)
            || !within(limits.open_crossings, MAX_OPEN_WORK)
            || !within(limits.step_bytes, MAX_STEP_BYTES)
            || !within(limits.draws_per_step, MAX_DRAWS_PER_STEP)
            || !within(limits.evidence_refs_per_input, MAX_LISTED)
            || !within(limits.report_queue, MAX_REPORT_QUEUE)
        {
            return fail("a limit is zero or above the plugin's hard ceiling");
        }
        if self.trusted_issuers.len() > limits.trusted_issuers as usize {
            return fail("more trusted issuers than the limit");
        }
        if self.step_profiles.is_empty() || !self.step_profiles.contains_key(&self.default_profile)
        {
            return fail("the default step profile is not defined");
        }
        if self.step_profiles.values().any(|profile| {
            profile.step_minutes == 0
                || profile.step_scale_bp == 0
                || !within(profile.max_steps_per_boundary, MAX_STEPS_PER_BOUNDARY)
        }) {
            return fail("a step profile has a zero length, scale, or step cap");
        }
        if EngagementKind::ALL
            .iter()
            .any(|kind| !self.kinds.contains_key(kind))
        {
            return fail("every engagement kind needs a rule");
        }
        if self.postures.is_empty()
            || self.postures.iter().collect::<BTreeSet<_>>().len() != self.postures.len()
        {
            return fail("postures are empty or repeated");
        }
        for rule in self.kinds.values() {
            if rule.max_minutes == 0
                || rule.frontage_bp.is_some_and(|bp| bp == 0 || bp > BP)
                || rule
                    .terrain
                    .as_ref()
                    .is_some_and(|terrain| !self.terrain.contains_key(terrain))
                || rule.posture_bp.iter().any(|cell| {
                    !self.knows_posture(&cell.attacker) || !self.knows_posture(&cell.defender)
                })
            {
                return fail("a kind rule has a zero duration, a bad frontage, or an unknown id");
            }
        }
        if self.condition_channels.len() > 64
            || self
                .condition_channels
                .iter()
                .collect::<BTreeSet<_>>()
                .len()
                != self.condition_channels.len()
        {
            return fail("condition channels are repeated or too many");
        }
        if self.band_cuts.is_empty()
            || self.band_cuts.len() > 16
            || self.band_cuts.iter().any(|cut| *cut > 1_000)
            || self.band_cuts.windows(2).any(|pair| pair[0] <= pair[1])
        {
            return fail("band cuts must be strictly descending per-mille values");
        }
        self.validate_forecast()?;
        self.validate_ranges()?;
        let lines = &self.lines;
        if lines.killed_denominator == 0
            || lines.killed_numerator > lines.killed_denominator
            || lines.inflicted_cap_per_mille > 1_000
            || lines.leadership.min_bp > lines.leadership.max_bp
            || self.morale.credibility_divisor == 0
        {
            return fail("line, casualty, or morale parameters are out of range");
        }
        if self.terrain.is_empty() || self.terrain.values().any(|row| row.frontage_bp > BP) {
            return fail("terrain rows are missing or have a frontage above 10^4");
        }
        if self
            .fortification
            .windows(2)
            .any(|pair| pair[0].grade >= pair[1].grade)
        {
            return fail("fortification grades must be ascending and unique");
        }
        let siege = &self.siege;
        if siege.day_minutes == 0
            || siege.men_per_access_sealed == 0
            || [
                &siege.assault_terrain,
                &siege.breach_terrain,
                &siege.sortie_terrain,
                &self.crossing.terrain,
            ]
            .into_iter()
            .any(|terrain| !self.terrain.contains_key(terrain))
        {
            return fail("siege or crossing rules name an unknown terrain or a zero day");
        }
        if self.crossing.wave_minutes == 0 || self.crossing.wave_men == 0 {
            return fail("crossing waves need a length and a size");
        }
        if self.rules_hash != self.compute_rules_hash()? {
            return fail("rules hash mismatch");
        }
        Ok(())
    }

    /// Every parameter the arithmetic multiplies is bounded, so no step can
    /// overflow.
    fn validate_ranges(&self) -> Result<(), CanwuError> {
        for profile in self.step_profiles.values() {
            bounded(profile.step_scale_bp, MAX_STEP_SCALE_BP, "step_scale_bp")?;
        }
        for rule in self.kinds.values() {
            bounded(rule.posture_steps, 10_000, "posture_steps")?;
            for cell in &rule.posture_bp {
                bounded(cell.bp, MAX_INPUT_BP, "posture bp")?;
            }
        }
        let lines = &self.lines;
        for (weight, label) in [
            (lines.fire_weight, "fire_weight"),
            (lines.shock_weight, "shock_weight"),
            (lines.melee_weight, "melee_weight"),
            (lines.leadership.command_weight, "command_weight"),
            (lines.leadership.pick_weight, "pick_weight"),
        ] {
            bounded(weight, MAX_WEIGHT, label)?;
        }
        bounded(lines.defense_offset, 1_000_000, "defense_offset")?;
        bounded(lines.killed_denominator, 1_000_000, "killed_denominator")?;
        bounded(
            lines.supply_fire.below_per_mille,
            1_000,
            "supply_fire.below_per_mille",
        )?;
        for (value, label) in [
            (lines.supply_fire.bp, "supply_fire.bp"),
            (lines.leadership.base_bp, "leadership.base_bp"),
            (lines.leadership.min_bp, "leadership.min_bp"),
            (lines.leadership.max_bp, "leadership.max_bp"),
            (self.night.fire_bp, "night.fire_bp"),
            (self.night.shock_bp, "night.shock_bp"),
            (self.night.melee_bp, "night.melee_bp"),
            (self.night.pursuit_bp, "night.pursuit_bp"),
            (
                self.siege.water_access_control_bp,
                "water_access_control_bp",
            ),
            (self.crossing.hard_river_wave_bp, "hard_river_wave_bp"),
            (self.crossing.intercept.min_control_bp, "min_control_bp"),
            (self.crossing.intercept.control_weight, "control_weight"),
        ] {
            bounded(value, MAX_INPUT_BP, label)?;
        }
        for (value, label) in [
            (self.cohesion.per_inflicted, "cohesion.per_inflicted"),
            (self.cohesion.base, "cohesion.base"),
            (self.cohesion.confusion, "cohesion.confusion"),
            (self.cohesion.recover_per_day, "cohesion.recover_per_day"),
            (
                self.morale.sensitivity_default,
                "morale.sensitivity_default",
            ),
            (
                self.morale.shock_recover_per_day,
                "morale.shock_recover_per_day",
            ),
        ] {
            bounded(value, MAX_VALUE, label)?;
        }
        bounded(
            self.morale.credibility_divisor,
            1_000_000,
            "credibility_divisor",
        )?;
        bounded(
            self.break_rule.rout_cohesion_below,
            1_000,
            "rout_cohesion_below",
        )?;
        bounded(
            self.break_rule.rout_morale_margin,
            1_000,
            "rout_morale_margin",
        )?;
        bounded(
            self.pursuit.captured_per_mille_at_full,
            1_000,
            "captured_per_mille_at_full",
        )?;
        bounded(
            self.pursuit.wounded_left_per_mille,
            1_000,
            "wounded_left_per_mille",
        )?;
        for row in self.terrain.values() {
            for value in [
                row.melee_bp,
                row.shock_bp,
                row.defense_bp,
                row.charge_window_bp,
                row.pursuit_bp,
            ] {
                bounded(value, MAX_INPUT_BP, "terrain bp")?;
            }
        }
        for grade in &self.fortification {
            bounded(grade.fort_bp, MAX_INPUT_BP, "fort_bp")?;
            bounded(grade.breach_threshold, 1_000_000_000, "breach_threshold")?;
        }
        bounded(self.siege.day_minutes, 1_000_000, "siege.day_minutes")?;
        bounded(
            self.siege.men_per_access_sealed,
            1_000_000_000,
            "men_per_access_sealed",
        )?;
        bounded(
            self.siege.sortie_cooldown_minutes,
            100_000_000,
            "sortie_cooldown_minutes",
        )?;
        bounded(
            self.crossing.wave_minutes,
            1_000_000,
            "crossing.wave_minutes",
        )?;
        bounded(
            self.crossing.wave_men,
            MAX_COMPONENT_MEN,
            "crossing.wave_men",
        )?;
        bounded(
            self.crossing.bridgehead_secure_men,
            MAX_COMPONENT_MEN,
            "bridgehead_secure_men",
        )?;
        let intercept = &self.crossing.intercept;
        bounded(intercept.scout_weight, 1_000_000, "scout_weight")?;
        bounded(
            intercept.night_concealment,
            1_000_000_000,
            "night_concealment",
        )?;
        bounded(intercept.threshold_bp, 1_000_000_000, "threshold_bp")?;
        for value in intercept.concealment.values() {
            bounded(*value, 1_000_000_000, "concealment")?;
        }
        bounded(
            self.limits.retire_after_minutes,
            1_000_000_000,
            "retire_after_minutes",
        )?;
        let forecast = &self.forecast;
        for term in &forecast.terms {
            bounded(term.cap, MAX_FORECAST_TERM, "forecast cap")?;
        }
        for value in forecast
            .manpower_bands
            .iter()
            .map(|band| band.value)
            .chain(forecast.terrain_fit.iter().map(|row| row.delta))
            .chain(forecast.environment_terms.iter().map(|row| row.delta))
        {
            bounded(
                value.unsigned_abs(),
                MAX_FORECAST_TERM.unsigned_abs(),
                "forecast value",
            )?;
        }
        for row in &forecast.environment_terms {
            if let Some(below) = row.ranged_reliability_below_bp {
                bounded(below, MAX_INPUT_BP, "ranged_reliability_below_bp")?;
            }
            if let Some(from) = row.ranged_share_from_per_mille {
                bounded(from, 1_000, "ranged_share_from_per_mille")?;
            }
        }
        for band in &forecast.ratio_bands {
            bounded(
                band.min_ratio_bp.unsigned_abs(),
                1_000_000_000_000,
                "min_ratio_bp",
            )?;
        }
        Ok(())
    }

    fn validate_forecast(&self) -> Result<(), CanwuError> {
        let forecast = &self.forecast;
        let fail = |message: &str| Err(invalid(format!("combat rulebook forecast: {message}")));
        if forecast.terms.len() > 64
            || forecast
                .terms
                .iter()
                .map(|term| &term.id)
                .collect::<BTreeSet<_>>()
                .len()
                != forecast.terms.len()
        {
            return fail("terms are repeated or too many");
        }
        for term in &forecast.terms {
            if term.divisor == 0 || term.cap < 0 {
                return fail("a term has a zero divisor or a negative cap");
            }
            if let TermSource::Channel { index } = term.source
                && usize::from(index) >= self.condition_channels.len()
            {
                return fail("a term reads an undeclared channel");
            }
        }
        if forecast
            .manpower_bands
            .first()
            .is_none_or(|band| band.from_men != 0)
            || forecast
                .manpower_bands
                .windows(2)
                .any(|pair| pair[0].from_men >= pair[1].from_men)
        {
            return fail("manpower bands must ascend from 0");
        }
        if forecast
            .terrain_fit
            .iter()
            .any(|row| !self.terrain.contains_key(&row.terrain))
        {
            return fail("a terrain-fit row names an unknown terrain");
        }
        if forecast
            .ratio_bands
            .last()
            .is_none_or(|band| band.min_ratio_bp > 0 || band.fortified.is_some())
        {
            return fail("the last ratio band must catch every ratio");
        }
        Ok(())
    }
}
