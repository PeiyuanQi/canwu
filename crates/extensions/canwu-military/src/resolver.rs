//! The step: the resolver trait, its draw accessor, the parametric
//! resolver, and the pure helpers a host uses for previews.

use crate::engagement::{
    ConditionInputV1, EnvironmentInputV1, ForecastV1, LinesV1, Side, TermValueV1,
};
use crate::force::{ComponentValuesV1, SideTermsV1};
use crate::ids::{ComponentId, ConcealmentClassId, ForceId, TermId, TerrainClassId};
use crate::rulebook::{
    CombatRulebookV1, EngagementKind, FortState, InterceptRuleV1, KindRuleV1, ResolverIdentityV1,
    StepProfileV1, TermSource, TerrainRowV1,
};
use canwu_api::{CanwuError, RandomDrawId};

const BP: i128 = 10_000;

/// The const identity of a step resolver ([`ResolverIdentityV1`] is its
/// owned twin in records).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResolverIdentity {
    pub name: &'static str,
    pub version: u32,
}

impl ResolverIdentity {
    #[must_use]
    pub fn owned(&self) -> ResolverIdentityV1 {
        ResolverIdentityV1 {
            name: self.name.to_owned(),
            version: self.version,
        }
    }

    #[must_use]
    pub fn matches(&self, identity: &ResolverIdentityV1) -> bool {
        identity.name == self.name && identity.version == self.version
    }
}

/// The function from a step's input and draws to its outcome.
///
/// The plugin owns everything around the call: it assembles the input,
/// performs every draw the resolver asks for, clamps the outcome, splits
/// casualties over components, applies the break rule and pursuit, and
/// decides whether the engagement continues.
pub trait StepResolver: Default + Send + Sync + 'static {
    const IDENTITY: ResolverIdentity;

    /// Deterministic in its input and its draws. It receives no view and no
    /// state; the plugin performs every draw it asks for. It must validate
    /// its input before its first draw: an error after a draw is applied as
    /// an empty step (both sides unchanged, posture `Continue`, term
    /// [`crate::TERM_RESOLVER_FAILED`]) and the engagement is held, so a
    /// drawn address is never re-drawn with other evidence.
    fn resolve(
        &self,
        input: &StepInputV1<'_>,
        draws: &mut dyn StepDraws,
    ) -> Result<StepOutcomeV1, CanwuError>;
}

/// Keyed draws for a resolver.
pub trait StepDraws {
    /// A keyed draw on `step-contingency` with the admitted step packet as
    /// evidence, operation id `contingency:<engagement>:<step index>`,
    /// target the engagement record version, slot `side × 32_768 +
    /// participant × 256 + purpose`; each slot at most once per step, at
    /// most the rulebook's `draws_per_step`. Returns the value below
    /// `upper_exclusive` and the draw's id.
    fn draw(
        &mut self,
        side: Side,
        participant: u16,
        purpose: u8,
        upper_exclusive: u64,
    ) -> Result<(u64, RandomDrawId), CanwuError>;
}

/// One component as a step reads it.
#[derive(Clone, Copy, Debug)]
pub struct StepComponentV1<'a> {
    pub force: &'a ForceId,
    pub component: &'a ComponentId,
    pub present: u64,
    pub values: &'a ComponentValuesV1,
}

/// One side of a step's input.
#[derive(Clone, Debug)]
pub struct SideStepInputV1<'a> {
    /// In (force id, component id) order, present men only.
    pub components: Vec<StepComponentV1<'a>>,
    /// The men-weighted terms of the side's forces; traits are their
    /// intersection.
    pub terms: SideTermsV1,
    pub condition: &'a ConditionInputV1,
    pub condition_age_steps: u32,
    pub cohesion: u32,
    pub morale: u32,
    pub attacking: bool,
}

impl SideStepInputV1<'_> {
    #[must_use]
    pub fn men(&self) -> u64 {
        self.components.iter().fold(0_u64, |sum, component| {
            sum.saturating_add(component.present)
        })
    }
}

/// A step's input.
#[derive(Clone, Debug)]
pub struct StepInputV1<'a> {
    pub rulebook: &'a CombatRulebookV1,
    pub kind: EngagementKind,
    pub kind_rule: &'a KindRuleV1,
    pub profile: &'a StepProfileV1,
    pub step_index: u32,
    pub attacker: SideStepInputV1<'a>,
    pub defender: SideStepInputV1<'a>,
    pub environment: &'a EnvironmentInputV1,
    pub terrain_class: &'a TerrainClassId,
    pub terrain: &'a TerrainRowV1,
    /// The fortification the defender fights behind: its grade and whether
    /// it counts as open (a breach, a gate, or a halved use).
    pub fort: Option<(u8, bool)>,
}

/// One side's outcome of a step.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SideOutcomeV1 {
    pub lines: LinesV1,
    /// What this side inflicts on the other, per mille of the other's engaged men.
    pub inflicted_per_mille: u32,
    /// What this side suffers.
    pub casualties: u64,
    pub killed: u64,
    pub cohesion_after: u32,
    pub morale_after: u32,
    pub morale_shock: u32,
}

/// A step's outcome.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StepOutcomeV1 {
    pub attacker: SideOutcomeV1,
    pub defender: SideOutcomeV1,
    pub terms: Vec<TermId>,
}

/// The four-line formula with every coefficient from the rulebook; it
/// draws nothing.
#[derive(Clone, Copy, Debug, Default)]
pub struct ParametricStepResolver;

impl StepResolver for ParametricStepResolver {
    const IDENTITY: ResolverIdentity = ResolverIdentity {
        name: "canwu.parametric",
        version: 1,
    };

    fn resolve(
        &self,
        input: &StepInputV1<'_>,
        _draws: &mut dyn StepDraws,
    ) -> Result<StepOutcomeV1, CanwuError> {
        let attacker = side_lines(input, &input.attacker);
        let defender = side_lines(input, &input.defender);
        let posture = if input.step_index < input.kind_rule.posture_steps {
            posture_bp(
                input.kind_rule,
                &input.attacker.condition.posture,
                &input.defender.condition.posture,
                input.fort,
            )
        } else {
            10_000
        };
        let on_defender = inflicted(input, &input.attacker, &attacker, &defender, posture);
        let on_attacker = inflicted(input, &input.defender, &defender, &attacker, 10_000);
        Ok(StepOutcomeV1 {
            attacker: take_blows(input, &input.attacker, attacker, on_defender, on_attacker),
            defender: take_blows(input, &input.defender, defender, on_attacker, on_defender),
            terms: Vec::new(),
        })
    }
}

fn floor_div(numerator: i128, denominator: i128) -> i128 {
    numerator.div_euclid(denominator.max(1))
}

fn to_u64(value: i128) -> u64 {
    u64::try_from(value.max(0)).unwrap_or(u64::MAX)
}

fn to_u32(value: i128) -> u32 {
    u32::try_from(value.max(0)).unwrap_or(u32::MAX)
}

/// The fortification factor the defender's defense line takes: the grade's
/// `fort_bp`, halved when open; 10^4 without a fortification.
#[must_use]
pub fn fort_bp(rulebook: &CombatRulebookV1, fort: Option<(u8, bool)>) -> u32 {
    let Some((grade, open)) = fort else {
        return 10_000;
    };
    let full = rulebook.fort_grade(grade).map_or(10_000, |row| row.fort_bp);
    if open { full / 2 } else { full }
}

/// The first matching posture cell of a kind, else 10^4.
#[must_use]
pub fn posture_bp(
    kind_rule: &KindRuleV1,
    attacker: &crate::PostureId,
    defender: &crate::PostureId,
    fort: Option<(u8, bool)>,
) -> u32 {
    kind_rule
        .posture_bp
        .iter()
        .find(|cell| {
            &cell.attacker == attacker
                && &cell.defender == defender
                && match cell.fort {
                    FortState::Any => true,
                    FortState::Closed => fort.is_some_and(|(_, open)| !open),
                    FortState::Open => fort.is_some_and(|(_, open)| open),
                }
        })
        .map_or(10_000, |cell| cell.bp)
}

/// A product of factors that saturates instead of overflowing; the
/// admission bounds keep every realistic product exact.
fn product(factors: &[i128]) -> i128 {
    factors
        .iter()
        .fold(1_i128, |product, factor| product.saturating_mul(*factor))
}

/// `floor(numerator × factor / denominator)` in one floor division when the
/// product fits, else split as `q × factor + floor(r × factor /
/// denominator)` with saturating arithmetic, so no input fails a boundary.
fn mul_div(numerator: i128, factor: i128, denominator: i128) -> i128 {
    let denominator = denominator.max(1);
    match numerator.checked_mul(factor) {
        Some(value) => floor_div(value, denominator),
        None => numerator
            .div_euclid(denominator)
            .saturating_mul(factor)
            .saturating_add(floor_div(
                numerator.rem_euclid(denominator).saturating_mul(factor),
                denominator,
            )),
    }
}

fn side_lines(input: &StepInputV1<'_>, side: &SideStepInputV1<'_>) -> LinesV1 {
    let rulebook = input.rulebook;
    let terrain = input.terrain;
    let men = i128::from(side.men());
    let frontage = i128::from(input.kind_rule.frontage_bp.unwrap_or(terrain.frontage_bp));
    let engaged = floor_div(product(&[men, frontage]), BP);
    let engaged_bp = floor_div(product(&[engaged, BP]), men.max(1));
    let sum = |value: fn(&ComponentValuesV1) -> u32| -> i128 {
        side.components.iter().fold(0_i128, |sum, component| {
            sum.saturating_add(product(&[
                i128::from(component.present),
                i128::from(value(component.values)),
            ]))
        })
    };
    let fire_sum = side.components.iter().fold(0_i128, |sum, component| {
        sum.saturating_add(product(&[
            i128::from(component.present),
            i128::from(component.values.fire),
            i128::from(component.values.training_bp),
        ]))
    });
    let fire_base = floor_div(product(&[fire_sum, engaged_bp]), 100_000_000_000);
    let shock_base = floor_div(
        product(&[sum(|values| values.shock), engaged_bp]),
        10_000_000,
    );
    let melee_base = floor_div(
        product(&[sum(|values| values.melee), engaged_bp]),
        10_000_000,
    );
    let defense_base = floor_div(
        product(&[sum(|values| values.defense), engaged_bp]),
        10_000_000,
    );
    let night = input.environment.night.then_some(&rulebook.night);
    let night_fire = i128::from(night.map_or(10_000, |night| night.fire_bp));
    let night_shock = i128::from(night.map_or(10_000, |night| night.shock_bp));
    let night_melee = i128::from(night.map_or(10_000, |night| night.melee_bp));
    let supply_fire =
        if side.condition.supply_per_mille < rulebook.lines.supply_fire.below_per_mille {
            i128::from(rulebook.lines.supply_fire.bp)
        } else {
            BP
        };
    let fire = floor_div(
        product(&[
            fire_base,
            i128::from(input.environment.ranged_reliability_bp),
            supply_fire,
            night_fire,
        ]),
        1_000_000_000_000,
    );
    let shock = floor_div(
        product(&[
            shock_base,
            i128::from(terrain.shock_bp),
            i128::from(terrain.charge_window_bp),
            night_shock,
        ]),
        1_000_000_000_000,
    );
    let melee = floor_div(
        product(&[melee_base, i128::from(terrain.melee_bp), night_melee]),
        100_000_000,
    );
    let mut defense = if side.attacking {
        defense_base
    } else {
        floor_div(
            product(&[
                defense_base,
                i128::from(fort_bp(rulebook, input.fort)),
                i128::from(terrain.defense_bp),
            ]),
            100_000_000,
        )
    };
    if side.condition.defense_halved {
        defense = floor_div(defense, 2);
    }
    let leadership = &rulebook.lines.leadership;
    let pick = if side.attacking {
        side.terms.battle_command
    } else {
        side.terms.assessment
    };
    let leadership_bp = i128::from(leadership.base_bp)
        .saturating_add(product(&[
            i128::from(side.terms.battle_command),
            i128::from(leadership.command_weight),
        ]))
        .saturating_add(product(&[
            i128::from(pick),
            i128::from(leadership.pick_weight),
        ]))
        .clamp(i128::from(leadership.min_bp), i128::from(leadership.max_bp));
    LinesV1 {
        men: to_u64(men),
        engaged: to_u64(engaged),
        fire: to_u64(fire),
        shock: to_u64(shock),
        melee: to_u64(melee),
        defense: to_u64(defense),
        leadership_bp: to_u32(leadership_bp),
    }
}

/// The unscaled per-step rate `from` inflicts on `to`, per mille of `to`'s
/// engaged men, as an exact fraction `(numerator, denominator)` with the
/// rulebook's cap applied: `attack × leadership_bp × surprise_bp ×
/// posture_bp / enemy_defense_term`, where `enemy_defense_term = (enemy
/// defense + defense_offset) × 10^13`. Every per-step quantity scales it by
/// the profile's `step_scale_bp` once, in its own single division.
fn inflicted(
    input: &StepInputV1<'_>,
    side: &SideStepInputV1<'_>,
    from: &LinesV1,
    to: &LinesV1,
    posture: u32,
) -> (i128, i128) {
    let lines = &input.rulebook.lines;
    let attack = product(&[i128::from(from.fire), i128::from(lines.fire_weight)])
        .saturating_add(product(&[
            i128::from(from.shock),
            i128::from(lines.shock_weight),
        ]))
        .saturating_add(product(&[
            i128::from(from.melee),
            i128::from(lines.melee_weight),
        ]));
    let denominator = product(&[
        i128::from(to.defense).saturating_add(i128::from(lines.defense_offset)),
        10_000_000_000_000,
    ])
    .max(1);
    let surprise =
        if input.step_index < input.kind_rule.posture_steps && side.condition.surprise_bp > 0 {
            i128::from(side.condition.surprise_bp)
        } else {
            BP
        };
    let numerator = product(&[
        attack,
        i128::from(from.leadership_bp),
        surprise,
        i128::from(posture),
    ]);
    (
        numerator.min(product(&[
            i128::from(lines.inflicted_cap_per_mille),
            denominator,
        ])),
        denominator,
    )
}

/// The scaled per-mille of a per-step rate: `floor(rate × step_scale_bp / 10^4)`.
fn scaled(rate: (i128, i128), scale: i128) -> i128 {
    mul_div(rate.0, scale, rate.1.saturating_mul(BP)).clamp(0, 1_000)
}

fn take_blows(
    input: &StepInputV1<'_>,
    side: &SideStepInputV1<'_>,
    lines: LinesV1,
    inflicts: (i128, i128),
    suffers: (i128, i128),
) -> SideOutcomeV1 {
    let rulebook = input.rulebook;
    let scale = i128::from(input.profile.step_scale_bp);
    let (numerator, denominator) = suffers;
    let suffered = scaled(suffers, scale);
    let casualties = floor_div(product(&[i128::from(lines.engaged), suffered]), 1_000);
    let killed = floor_div(
        product(&[casualties, i128::from(rulebook.lines.killed_numerator)]),
        i128::from(rulebook.lines.killed_denominator),
    );
    let cohesion = &rulebook.cohesion;
    let fixed = i128::from(cohesion.base).saturating_add(product(&[
        i128::from(side.condition.order_confusion),
        i128::from(cohesion.confusion),
    ]));
    let cohesion_loss = mul_div(
        product(&[numerator, i128::from(cohesion.per_inflicted)])
            .saturating_add(product(&[fixed, denominator])),
        scale,
        denominator.saturating_mul(BP),
    );
    let sensitivity = if side.terms.morale_sensitivity > 0 {
        side.terms.morale_sensitivity
    } else {
        rulebook.morale.sensitivity_default
    };
    let shock = mul_div(
        product(&[numerator, i128::from(sensitivity)]),
        scale,
        denominator.saturating_mul(100_000),
    )
    // The relief is whole quanta of credibility (floored first), each scaled
    // by the step: credibility below one divisor relieves nothing.
    .saturating_sub(floor_div(
        product(&[
            floor_div(
                i128::from(side.terms.credibility),
                i128::from(rulebook.morale.credibility_divisor),
            ),
            scale,
        ]),
        BP,
    ))
    .max(0);
    SideOutcomeV1 {
        lines,
        inflicted_per_mille: to_u32(scaled(inflicts, scale)),
        casualties: to_u64(casualties),
        killed: to_u64(killed),
        cohesion_after: to_u32(i128::from(side.cohesion).saturating_sub(cohesion_loss)),
        morale_after: to_u32(i128::from(side.morale).saturating_sub(shock)),
        morale_shock: to_u32(shock),
    }
}

// The band of a per-mille value under strictly descending cuts: the index
/// of the first cut the value reaches, or the number of cuts.
#[must_use]
pub fn band_of(cuts: &[u16], per_mille: u64) -> u8 {
    let position = cuts
        .iter()
        .position(|cut| per_mille >= u64::from(*cut))
        .unwrap_or(cuts.len());
    u8::try_from(position).unwrap_or(u8::MAX)
}

/// Splits `total` over `weights` by the largest remainder, ties to the
/// earlier weight.
#[must_use]
pub fn largest_remainder(total: u64, weights: &[u64]) -> Vec<u64> {
    let sum: u128 = weights.iter().fold(0_u128, |sum, weight| {
        sum.saturating_add(u128::from(*weight))
    });
    if sum == 0 {
        return vec![0; weights.len()];
    }
    let total = u128::from(total);
    let mut shares: Vec<u64> = weights
        .iter()
        .map(|weight| u64::try_from(total * u128::from(*weight) / sum).unwrap_or(u64::MAX))
        .collect();
    let mut remainders: Vec<(u128, usize)> = weights
        .iter()
        .enumerate()
        .map(|(index, weight)| (total * u128::from(*weight) % sum, index))
        .collect();
    remainders.sort_by(|left, right| right.0.cmp(&left.0).then(left.1.cmp(&right.1)));
    let given: u128 = shares
        .iter()
        .fold(0_u128, |sum, share| sum.saturating_add(u128::from(*share)));
    let left = usize::try_from(total.saturating_sub(given)).unwrap_or(usize::MAX);
    for (_, index) in remainders.into_iter().take(left) {
        shares[index] = shares[index].saturating_add(1);
    }
    shares
}

/// The pieces a component of `present` men emplaces:
/// `floor(present × 10 / crew_per_piece_x10)`, none without a breach value.
#[must_use]
pub fn pieces(present: u64, values: &ComponentValuesV1) -> u64 {
    if values.breach_per_piece == 0 || values.crew_per_piece_x10 == 0 {
        return 0;
    }
    present.saturating_mul(10) / u64::from(values.crew_per_piece_x10)
}

/// A siege day's breach progress: `max(0, floor(Σ pieces × breach_per_piece
/// × work_bp / 10^4) − repair_per_day)`.
#[must_use]
pub fn breach_rate<'a>(
    components: impl IntoIterator<Item = (u64, &'a ComponentValuesV1)>,
    work_bp: u32,
    repair_per_day: u32,
) -> u64 {
    let work = components
        .into_iter()
        .fold(0_i128, |sum, (present, values)| {
            sum.saturating_add(product(&[
                i128::from(pieces(present, values)),
                i128::from(values.breach_per_piece),
            ]))
        });
    to_u64(
        floor_div(product(&[work, i128::from(work_bp)]), BP)
            .saturating_sub(i128::from(repair_per_day)),
    )
}

/// A wave's interception score:
/// `floor(control_bp × control_weight / 10^4) + scouts × scout_weight −
/// concealment − night concealment`.
#[must_use]
pub fn intercept_bp(
    rule: &InterceptRuleV1,
    control_bp: u32,
    scouts: u32,
    concealment: &ConcealmentClassId,
    night: bool,
) -> i64 {
    let value = floor_div(
        product(&[i128::from(control_bp), i128::from(rule.control_weight)]),
        BP,
    ) + product(&[i128::from(scouts), i128::from(rule.scout_weight)])
        - i128::from(rule.concealment.get(concealment).copied().unwrap_or(0))
        - if night {
            i128::from(rule.night_concealment)
        } else {
            0
        };
    i64::try_from(value).unwrap_or(if value < 0 { i64::MIN } else { i64::MAX })
}

/// One side as the forecast reads it.
#[derive(Clone, Debug)]
pub struct ForecastSideV1<'a> {
    /// Present men and values of each component.
    pub components: Vec<(u64, &'a ComponentValuesV1)>,
    pub terms: &'a SideTermsV1,
    pub condition: &'a ConditionInputV1,
    pub cohesion: u32,
    /// The host's morale less the battle's shock.
    pub morale: u32,
}

/// The pre-battle forecast of two sides on `terrain` in `environment`.
pub fn forecast(
    rulebook: &CombatRulebookV1,
    terrain: &TerrainClassId,
    environment: &EnvironmentInputV1,
    fortified: bool,
    attacker: &ForecastSideV1<'_>,
    defender: &ForecastSideV1<'_>,
) -> Result<ForecastV1, CanwuError> {
    let attacker_terms = forecast_terms(rulebook, terrain, environment, attacker, false);
    let defender_terms = forecast_terms(rulebook, terrain, environment, defender, true);
    let total = |terms: &[TermValueV1]| {
        terms
            .iter()
            .fold(0_i64, |sum, term| sum.saturating_add(term.value))
    };
    let attacker_total = total(&attacker_terms);
    let defender_total = total(&defender_terms);
    let ratio = floor_div(
        i128::from(attacker_total.max(0)) * BP,
        i128::from(defender_total.max(1)),
    );
    let ratio_bp = i64::try_from(ratio).unwrap_or(i64::MAX);
    let band = rulebook
        .forecast
        .ratio_bands
        .iter()
        .find(|band| {
            ratio_bp >= band.min_ratio_bp && band.fortified.is_none_or(|value| value == fortified)
        })
        .or_else(|| rulebook.forecast.ratio_bands.last())
        .map(|band| band.band.clone())
        .ok_or_else(|| crate::ids::invalid("the rulebook has no ratio band"))?;
    Ok(ForecastV1 {
        attacker_terms,
        defender_terms,
        attacker_total,
        defender_total,
        ratio_bp,
        band,
    })
}

fn forecast_terms(
    rulebook: &CombatRulebookV1,
    terrain: &TerrainClassId,
    environment: &EnvironmentInputV1,
    side: &ForecastSideV1<'_>,
    defending: bool,
) -> Vec<TermValueV1> {
    let men: u128 = side
        .components
        .iter()
        .map(|(present, _)| u128::from(*present))
        .sum();
    let weighted = |value: fn(&ComponentValuesV1) -> u32| -> i64 {
        let total: u128 = side
            .components
            .iter()
            .map(|(present, values)| u128::from(*present) * u128::from(value(values)))
            .sum();
        i64::try_from(total / men.max(1)).unwrap_or(i64::MAX)
    };
    let ranged: u128 = side
        .components
        .iter()
        .filter(|(_, values)| values.ranged)
        .map(|(present, _)| u128::from(*present))
        .sum();
    let ranged_share = u16::try_from(ranged * 1_000 / men.max(1)).unwrap_or(1_000);
    let has = |trait_: &Option<crate::TraitId>| {
        trait_
            .as_ref()
            .is_none_or(|trait_| side.terms.traits.contains(trait_))
    };
    rulebook
        .forecast
        .terms
        .iter()
        .map(|term| {
            let divided = |value: i64| (value / i64::from(term.divisor.max(1))).min(term.cap);
            let value = match term.source {
                TermSource::Manpower => rulebook
                    .forecast
                    .manpower_bands
                    .iter()
                    .take_while(|band| u128::from(band.from_men) <= men)
                    .last()
                    .map_or(0, |band| band.value)
                    .min(term.cap),
                TermSource::Readiness => divided(i64::from(side.condition.readiness_per_mille)),
                TermSource::Supply => divided(i64::from(side.condition.supply_per_mille)),
                TermSource::Morale => divided(i64::from(side.morale)),
                TermSource::Cohesion => divided(i64::from(side.cohesion)),
                TermSource::Channel { index } => divided(i64::from(
                    side.condition
                        .channels
                        .get(usize::from(index))
                        .copied()
                        .unwrap_or(0),
                )),
                TermSource::Training => divided(weighted(|values| values.training_bp)),
                TermSource::Equipment => divided(weighted(|values| values.equipment_bp)),
                TermSource::Command => divided(i64::from(side.terms.battle_command)),
                TermSource::TerrainFit => rulebook
                    .forecast
                    .terrain_fit
                    .iter()
                    .filter(|row| {
                        &row.terrain == terrain
                            && has(&row.trait_)
                            && row.defending.is_none_or(|value| value == defending)
                    })
                    .fold(0_i64, |sum, row| sum.saturating_add(row.delta))
                    .clamp(-term.cap, term.cap),
                TermSource::Environment => rulebook
                    .forecast
                    .environment_terms
                    .iter()
                    .filter(|row| {
                        row.ground
                            .as_ref()
                            .is_none_or(|ground| ground == &environment.ground)
                            && row
                                .ranged_reliability_below_bp
                                .is_none_or(|below| environment.ranged_reliability_bp < below)
                            && has(&row.trait_)
                            && row
                                .ranged_share_from_per_mille
                                .is_none_or(|from| ranged_share >= from)
                    })
                    .fold(0_i64, |sum, row| sum.saturating_add(row.delta))
                    .clamp(-term.cap, term.cap),
            };
            TermValueV1 {
                term: term.id.clone(),
                value,
            }
        })
        .collect()
}
