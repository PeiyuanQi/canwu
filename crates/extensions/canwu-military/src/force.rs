//! A force as combat knows it: identity, owner, controllers, location,
//! components, side terms, and battle state. Everything else about a force
//! is the host's.

use crate::ids::{
    ComponentId, CrossingId, EngagementId, ForceId, PostureId, SCHEMA_VERSION, SiegeId, TraitId,
    invalid, validate_id,
};
use crate::rulebook::{
    CombatRulebookV1, MAX_COMPONENT_MEN, MAX_COMPONENTS_PER_FORCE, MAX_INPUT_BP, MAX_LISTED,
    MAX_VALUE, bounded,
};
use canwu_api::{CanwuError, EntityRef, Issuer, SimDuration, SimTime};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Where a force stands; the host places it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LocationV1 {
    At {
        site: EntityRef,
    },
    OnRoute {
        route: EntityRef,
        from: EntityRef,
        to: EntityRef,
        progress_per_mille: u16,
    },
}

impl LocationV1 {
    #[must_use]
    pub fn is_at(&self, site: &EntityRef) -> bool {
        matches!(self, Self::At { site: here } if here == site)
    }
}

/// Men lost, by cause.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct LossTallyV1 {
    pub killed: u64,
    pub wounded: u64,
    pub captured: u64,
    pub dispersed: u64,
}

impl LossTallyV1 {
    #[must_use]
    pub const fn total(&self) -> u64 {
        self.killed
            .saturating_add(self.wounded)
            .saturating_add(self.captured)
            .saturating_add(self.dispersed)
    }

    pub fn add(&mut self, other: &Self) {
        self.killed = self.killed.saturating_add(other.killed);
        self.wounded = self.wounded.saturating_add(other.wounded);
        self.captured = self.captured.saturating_add(other.captured);
        self.dispersed = self.dispersed.saturating_add(other.dispersed);
    }

    #[must_use]
    pub const fn is_zero(&self) -> bool {
        self.total() == 0
    }
}

/// What one man of a component brings to the four lines, as the host
/// compiles it from its unit profiles.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ComponentValuesV1 {
    pub fire: u32,
    pub melee: u32,
    pub shock: u32,
    pub defense: u32,
    pub training_bp: u32,
    pub equipment_bp: u32,
    /// Breach progress one emplaced piece makes a day at full work.
    pub breach_per_piece: u32,
    /// Crew per piece, in tenths of a man; 0 makes no pieces.
    pub crew_per_piece_x10: u32,
    pub ranged: bool,
    pub mounted: bool,
    pub vessel: bool,
}

impl ComponentValuesV1 {
    /// Refuses a value above the plugin's bounds (`ValueOutOfRange`).
    pub fn check_ranges(&self) -> Result<(), CanwuError> {
        for (value, label) in [
            (self.fire, "fire"),
            (self.melee, "melee"),
            (self.shock, "shock"),
            (self.defense, "defense"),
            (self.breach_per_piece, "breach_per_piece"),
            (self.crew_per_piece_x10, "crew_per_piece_x10"),
        ] {
            bounded(value, MAX_VALUE, label)?;
        }
        bounded(self.training_bp, MAX_INPUT_BP, "training_bp")?;
        bounded(self.equipment_bp, MAX_INPUT_BP, "equipment_bp")
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ComponentStateV1 {
    pub present: u64,
    pub values: ComponentValuesV1,
    pub losses: LossTallyV1,
}

/// The side terms of a force, as the host compiles them from its
/// institutions and commanders. Per-mille values unless named otherwise.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SideTermsV1 {
    pub cohesion_cap: u32,
    pub morale_break: u32,
    /// 0 takes the rulebook's `morale.sensitivity_default`.
    pub morale_sensitivity: u32,
    pub battle_command: u32,
    pub assessment: u32,
    pub credibility: u32,
    pub traits: BTreeSet<TraitId>,
}

/// A force's part in a siege.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum SiegeRoleV1 {
    Besieging { siege: SiegeId },
    Defending { siege: SiegeId },
}

impl SiegeRoleV1 {
    #[must_use]
    pub const fn siege(&self) -> &SiegeId {
        match self {
            Self::Besieging { siege } | Self::Defending { siege } => siege,
        }
    }
}

/// What the plugin owns of a force: the battle's state.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct BattleStateV1 {
    /// `None` until a battle moves it; then the plugin's.
    pub cohesion: Option<u32>,
    pub morale_shock: u32,
    pub last_fought_at: Option<SimTime>,
    pub recovered_to: Option<SimTime>,
    pub posture: Option<PostureId>,
    pub engaged_in: Option<EngagementId>,
    pub siege_role: Option<SiegeRoleV1>,
    pub crossing: Option<CrossingId>,
    pub sortie_ready_at: Option<SimTime>,
    pub dispersed: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ForceStatus {
    Active,
    Retired,
}

/// A force (record `canwu.military:force`, an entity that may hold
/// knowledge).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ForceStateV1 {
    pub schema_version: u32,
    pub id: ForceId,
    pub owner: EntityRef,
    pub controllers: Vec<Issuer>,
    /// Advanced by host commands only; steps leave it.
    pub host_revision: u64,
    pub location: LocationV1,
    pub components: BTreeMap<ComponentId, ComponentStateV1>,
    pub terms: SideTermsV1,
    pub battle: BattleStateV1,
    pub status: ForceStatus,
}

impl ForceStateV1 {
    /// The men present over every component.
    #[must_use]
    pub fn men(&self) -> u64 {
        self.components.values().fold(0_u64, |sum, component| {
            sum.saturating_add(component.present)
        })
    }

    /// Every component's losses.
    #[must_use]
    pub fn losses(&self) -> LossTallyV1 {
        let mut total = LossTallyV1::default();
        for component in self.components.values() {
            total.add(&component.losses);
        }
        total
    }

    /// In no engagement, siege, or crossing.
    #[must_use]
    pub const fn is_free(&self) -> bool {
        self.battle.engaged_in.is_none()
            && self.battle.siege_role.is_none()
            && self.battle.crossing.is_none()
    }

    #[must_use]
    pub fn is_active(&self) -> bool {
        self.status == ForceStatus::Active
    }

    /// The force's cohesion: the battle's, or its cap.
    #[must_use]
    pub fn cohesion(&self) -> u32 {
        self.battle.cohesion.unwrap_or(self.terms.cohesion_cap)
    }

    /// Credits the full days the force did not fight since its last fight
    /// or credit: cohesion back toward its cap and shock decaying, as the
    /// rulebook sets, so a day the force fought never recovers.
    pub fn recover(&mut self, rulebook: &CombatRulebookV1, now: SimTime) {
        let Some(from) = self.battle.last_fought_at.max(self.battle.recovered_to) else {
            return;
        };
        let day = i64::from(rulebook.siege.day_minutes.max(1));
        let Some(elapsed) = now.checked_sub(from) else {
            return;
        };
        let days = elapsed.as_minutes() / day;
        if days <= 0 {
            return;
        }
        let days_u32 = u32::try_from(days).unwrap_or(u32::MAX);
        if let Some(cohesion) = self.battle.cohesion {
            self.battle.cohesion = Some(
                cohesion
                    .saturating_add(days_u32.saturating_mul(rulebook.cohesion.recover_per_day))
                    .min(self.terms.cohesion_cap),
            );
        }
        self.battle.morale_shock = self
            .battle
            .morale_shock
            .saturating_sub(days_u32.saturating_mul(rulebook.morale.shock_recover_per_day));
        self.battle.recovered_to = from.checked_add(SimDuration::minutes(days * day));
    }

    /// Checks the force's shape against the hard ceilings; the rulebook's
    /// limits are checked where a command or activation knows them.
    pub fn validate(&self) -> Result<(), CanwuError> {
        validate_id(self.id.as_str(), "force")?;
        if self.schema_version != SCHEMA_VERSION
            || self.components.len() > MAX_COMPONENTS_PER_FORCE as usize
            || self.controllers.len() > MAX_LISTED as usize
            || self.terms.cohesion_cap > 1_000
            || self.terms.morale_break > 1_000
            || self.battle.cohesion.is_some_and(|value| value > 1_000)
            || matches!(self.location, LocationV1::OnRoute { progress_per_mille, .. } if progress_per_mille > 1_000)
        {
            return Err(invalid(format!("force {} is invalid", self.id)));
        }
        for component in self.components.values() {
            bounded(component.present, MAX_COMPONENT_MEN, "present")?;
            component.values.check_ranges()?;
        }
        for (value, label) in [
            (self.terms.morale_sensitivity, "morale_sensitivity"),
            (self.terms.battle_command, "battle_command"),
            (self.terms.assessment, "assessment"),
            (self.terms.credibility, "credibility"),
        ] {
            bounded(value, MAX_VALUE, label)?;
        }
        Ok(())
    }

    /// Checks the force against the rulebook's limits.
    pub fn validate_limits(&self, rulebook: &CombatRulebookV1) -> Result<(), CanwuError> {
        let limits = &rulebook.limits;
        if self.components.len() > limits.components_per_force as usize
            || self.controllers.len() > limits.controllers_per_force as usize
        {
            return Err(CanwuError::new(
                canwu_api::ErrorCode::ValueOutOfRange,
                format!("force {} exceeds the rulebook's limits", self.id),
            ));
        }
        Ok(())
    }
}
