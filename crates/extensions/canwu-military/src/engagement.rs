//! Engagements, their inputs, steps, and summaries.

use crate::force::LossTallyV1;
use crate::ids::{
    ConcealmentClassId, CrossingId, EngagementId, ForceId, ForecastBandId, GroundClassId,
    PostureId, ProfileId, SCHEMA_VERSION, SiegeId, TermId, TerrainClassId, invalid, validate_id,
};
use crate::rulebook::{EngagementKind, ResolverIdentityV1};
use canwu_api::{
    CanwuError, DomainRecordVersionRef, EntityRef, KnowledgeHolderRef, RandomDrawId, SimTime,
};
use serde::{Deserialize, Serialize};

/// A side of an engagement.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Attacker,
    Defender,
}

impl Side {
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Attacker => 0,
            Self::Defender => 1,
        }
    }

    #[must_use]
    pub const fn other(self) -> Self {
        match self {
            Self::Attacker => Self::Defender,
            Self::Defender => Self::Attacker,
        }
    }
}

/// The host's per-side values a step reads; the latest stands until the
/// next `SupplyInput`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ConditionInputV1 {
    /// Per mille, in the rulebook's `condition_channels` order.
    pub channels: Vec<u16>,
    pub supply_per_mille: u16,
    pub morale_per_mille: u16,
    pub readiness_per_mille: u16,
    pub fatigue_per_mille: u16,
    pub posture: PostureId,
    /// The side's surprise for the kind's `posture_steps`; 0 means none (10^4).
    pub surprise_bp: u32,
    /// How many sources of order confusion the side suffers (each costs the
    /// rulebook's `cohesion.confusion` a step).
    pub order_confusion: u32,
    pub defense_halved: bool,
    /// The host records the input was derived from: cited on the step,
    /// never read.
    pub evidence: Vec<DomainRecordVersionRef>,
}

impl ConditionInputV1 {
    /// Refuses a value above the plugin's bounds (`ValueOutOfRange`).
    pub fn check_ranges(&self) -> Result<(), CanwuError> {
        for value in self.channels.iter().chain([
            &self.supply_per_mille,
            &self.morale_per_mille,
            &self.readiness_per_mille,
            &self.fatigue_per_mille,
        ]) {
            crate::rulebook::bounded(*value, 1_000, "a per-mille condition value")?;
        }
        crate::rulebook::bounded(self.surprise_bp, crate::MAX_INPUT_BP, "surprise_bp")?;
        crate::rulebook::bounded(self.order_confusion, 1_000, "order_confusion")
    }
}

/// The state of a river a crossing uses.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RiverBand {
    Open,
    /// Waves are scaled by the rulebook's `hard_river_wave_bp`.
    Hard,
    /// No wave crosses.
    Closed,
}

/// The host's environment of a fight.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EnvironmentInputV1 {
    /// `None` takes the kind rule's terrain.
    pub terrain: Option<TerrainClassId>,
    pub ground: GroundClassId,
    pub concealment: ConcealmentClassId,
    pub night: bool,
    pub ranged_reliability_bp: u32,
    pub river: RiverBand,
    pub evidence: Vec<DomainRecordVersionRef>,
}

impl EnvironmentInputV1 {
    /// Refuses a value above the plugin's bounds (`ValueOutOfRange`).
    pub fn check_ranges(&self) -> Result<(), CanwuError> {
        crate::rulebook::bounded(
            self.ranged_reliability_bp,
            crate::MAX_INPUT_BP,
            "ranged_reliability_bp",
        )
    }
}

/// A side's posture after a step.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Continuation {
    #[default]
    Continue,
    Withdraw,
    Rout,
}

/// One side of an engagement.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SideStateV1 {
    pub forces: Vec<ForceId>,
    pub holders: Vec<KnowledgeHolderRef>,
    pub input: Option<(ConditionInputV1, SimTime)>,
    /// The battle's shock, accumulated over the engagement.
    pub morale_shock: u32,
    pub cohesion: u32,
    pub posture_after: Continuation,
    pub losses: LossTallyV1,
    /// The side's men when it entered the fight (joiners added).
    pub opened_men: u64,
    /// The most men the side fights with (a crossing wave or bridgehead).
    pub men_cap: Option<u64>,
    /// A `Withdraw` order the next step carries out.
    pub withdraw_ordered: bool,
}

/// One traced forecast term.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TermValueV1 {
    pub term: TermId,
    pub value: i64,
}

/// The pre-battle forecast: each side's traced terms, the ratio, and its band.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ForecastV1 {
    pub attacker_terms: Vec<TermValueV1>,
    pub defender_terms: Vec<TermValueV1>,
    pub attacker_total: i64,
    pub defender_total: i64,
    pub ratio_bp: i64,
    pub band: ForecastBandId,
}

/// Why an engagement stopped stepping.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HoldReason {
    StepOversized,
    ResolverFailed,
    DrawLimit,
    MissingInput,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum EngagementStatus {
    Open,
    Held { reason: HoldReason },
    Closed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EngagementOutcomeKind {
    AttackerPrevailed,
    Repulsed,
    Stalled,
    Abandoned,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EngagementOutcomeV1 {
    pub kind: EngagementOutcomeKind,
    pub closed_at: SimTime,
    pub site_fell: bool,
}

/// An engagement (record `canwu.military:engagement`).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EngagementStateV1 {
    pub schema_version: u32,
    pub id: EngagementId,
    pub host_revision: u64,
    pub kind: EngagementKind,
    pub profile: ProfileId,
    pub site: EntityRef,
    /// The site's fortification grade and whether a breach or gate is open.
    pub fort: Option<(u8, bool)>,
    /// A terrain the plugin set for a fight it opened (assault, breach,
    /// sortie, crossing); it precedes the environment's and the kind's.
    pub terrain: Option<TerrainClassId>,
    pub siege: Option<SiegeId>,
    pub crossing: Option<CrossingId>,
    pub attacker: SideStateV1,
    pub defender: SideStateV1,
    pub environment: Option<(EnvironmentInputV1, SimTime)>,
    pub forecast: ForecastV1,
    pub opened_at: SimTime,
    /// A fight that has not broken this many minutes after it opened closes
    /// `Stalled`: the kind rule's `max_minutes`, or the shorter duration the
    /// host named when it opened the fight.
    pub max_minutes: u32,
    pub last_step_at: SimTime,
    pub next_step_at: SimTime,
    pub steps_resolved: u32,
    pub status: EngagementStatus,
    pub outcome: Option<EngagementOutcomeV1>,
}

impl EngagementStateV1 {
    #[must_use]
    pub const fn side(&self, side: Side) -> &SideStateV1 {
        match side {
            Side::Attacker => &self.attacker,
            Side::Defender => &self.defender,
        }
    }

    pub fn side_mut(&mut self, side: Side) -> &mut SideStateV1 {
        match side {
            Side::Attacker => &mut self.attacker,
            Side::Defender => &mut self.defender,
        }
    }

    /// Every force of both sides, attacker first.
    pub fn forces(&self) -> impl Iterator<Item = &ForceId> {
        self.attacker.forces.iter().chain(&self.defender.forces)
    }

    #[must_use]
    pub const fn is_closed(&self) -> bool {
        matches!(self.status, EngagementStatus::Closed)
    }

    pub fn validate(&self) -> Result<(), CanwuError> {
        validate_id(self.id.as_str(), "engagement")?;
        if self.schema_version != SCHEMA_VERSION
            || self.max_minutes == 0
            || self.attacker.forces.is_empty()
            || self.defender.forces.is_empty()
        {
            return Err(invalid(format!("engagement {} is invalid", self.id)));
        }
        Ok(())
    }
}

/// One side's four lines in a step.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct LinesV1 {
    pub men: u64,
    pub engaged: u64,
    pub fire: u64,
    pub shock: u64,
    pub melee: u64,
    pub defense: u64,
    pub leadership_bp: u32,
}

/// One side of a step, with every per-step quantity.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SideStepV1 {
    pub lines: LinesV1,
    pub inflicted_per_mille: u32,
    pub killed: u64,
    pub wounded: u64,
    pub captured: u64,
    pub dispersed: u64,
    pub cohesion_after: u32,
    pub morale_after: u32,
    pub morale_shock: u32,
    pub posture_after: Continuation,
    pub condition_age_steps: u32,
}

/// One step of an engagement: the per-step evidence.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StepV1 {
    pub index: u32,
    pub at: SimTime,
    pub attacker: SideStepV1,
    pub defender: SideStepV1,
    pub attacker_channels: Vec<u16>,
    pub defender_channels: Vec<u16>,
    /// The canonical digest of both sides' condition inputs, evidence
    /// included.
    pub condition_digest: String,
    /// The canonical digest of the environment input, evidence included.
    pub environment_digest: String,
    pub terms: Vec<TermId>,
    pub draws: Vec<RandomDrawId>,
    pub resolver: ResolverIdentityV1,
}

/// The steps one stepping boundary resolved for one engagement
/// (record `canwu.military:engagement_steps`); the history is the journal.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EngagementStepsV1 {
    pub schema_version: u32,
    pub engagement: EngagementId,
    pub boundary_at: SimTime,
    pub steps: Vec<StepV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SideSummaryV1 {
    pub forces: Vec<ForceId>,
    pub losses: LossTallyV1,
    pub routed: Vec<ForceId>,
}

/// The durable record of a closed engagement
/// (record `canwu.military:engagement_summary`, create-only).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EngagementSummaryV1 {
    pub schema_version: u32,
    pub engagement: EngagementId,
    pub kind: EngagementKind,
    pub site: EntityRef,
    pub opened_at: SimTime,
    pub closed_at: SimTime,
    pub steps: u32,
    pub outcome: EngagementOutcomeKind,
    pub site_fell: bool,
    pub attacker: SideSummaryV1,
    pub defender: SideSummaryV1,
    pub siege: Option<SiegeId>,
    pub crossing: Option<CrossingId>,
}

/// One force's losses in the boundary that closed an engagement.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ForceLossDeltaV1 {
    pub force: ForceId,
    pub losses: LossTallyV1,
}

/// The optional next-boundary packet a closed engagement sends to the
/// host's target ([`crate::MilitaryPlugin::with_outcome_signal`]).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EngagementOutcomeSignalV1 {
    pub summary: EngagementSummaryV1,
    pub force_losses: Vec<ForceLossDeltaV1>,
}
