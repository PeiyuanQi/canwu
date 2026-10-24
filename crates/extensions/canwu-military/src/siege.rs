//! Sieges: investment, emplacement, breach, gate, assault, sortie, relief,
//! fall, and lift of a host site.

use crate::crossing::InterceptorInputV1;
use crate::engagement::{ConditionInputV1, EnvironmentInputV1};
use crate::ids::{
    EngagementId, ForceId, RationLevelId, SCHEMA_VERSION, SiegeId, invalid, validate_id,
};
use canwu_api::{CanwuError, DomainRecordVersionRef, EntityRef, KnowledgeHolderRef, SimTime};
use serde::{Deserialize, Serialize};

/// The site as the host describes it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SiteProfileV1 {
    pub grade: u8,
    pub accesses: u32,
    pub water_accesses: u32,
    pub repair_per_day: u32,
}

impl SiteProfileV1 {
    /// Refuses a value above the plugin's bounds (`ValueOutOfRange`).
    pub fn check_ranges(&self) -> Result<(), CanwuError> {
        crate::rulebook::bounded(self.accesses, 1_000_000, "accesses")?;
        crate::rulebook::bounded(self.water_accesses, 1_000_000, "water_accesses")?;
        crate::rulebook::bounded(self.repair_per_day, 1_000_000_000, "repair_per_day")
    }
}

/// The defenders' supply as the host derives it: recorded and reported,
/// decided on by nothing in the plugin.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SiegeSupplyInputV1 {
    pub days_of_stock: u64,
    pub ration: RationLevelId,
    pub evidence: Vec<DomainRecordVersionRef>,
}

/// When the besiegers storm without an order.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StormRule {
    Never,
    /// The day a breach or a gate is open.
    BreachOrGate,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SiegeStatus {
    Open,
    Fell,
    Lifted,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SiegeEndCause {
    Breach,
    GateOpened,
    Storm,
    Abandoned,
    Relieved,
    Withdrawn,
}

/// A siege (record `canwu.military:siege`).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SiegeStateV1 {
    pub schema_version: u32,
    pub id: SiegeId,
    pub host_revision: u64,
    pub site: EntityRef,
    pub profile: SiteProfileV1,
    pub besiegers: Vec<ForceId>,
    pub defenders: Vec<ForceId>,
    /// Besiegers' holders, defenders' holders.
    pub holders: [Vec<KnowledgeHolderRef>; 2],
    /// Besiegers' latest input, defenders' latest input.
    pub inputs: [Option<(ConditionInputV1, SimTime)>; 2],
    pub environment: Option<(EnvironmentInputV1, SimTime)>,
    pub naval: Option<InterceptorInputV1>,
    pub investment_bp: u32,
    pub pieces: u64,
    pub emplacement_from: Option<SimTime>,
    pub bombardment_from: Option<SimTime>,
    pub work_bp: u32,
    pub breach_progress: u32,
    pub breach_threshold: u32,
    pub breach_open: bool,
    pub gate_open_until: Option<SimTime>,
    pub supply: SiegeSupplyInputV1,
    pub storm_on: StormRule,
    pub assault_ordered: bool,
    pub sortie_ordered: bool,
    /// The assault or sortie in progress.
    pub engagement: Option<EngagementId>,
    pub started_at: SimTime,
    pub next_day_at: SimTime,
    pub status: SiegeStatus,
    pub end_cause: Option<SiegeEndCause>,
    pub ended_at: Option<SimTime>,
}

impl SiegeStateV1 {
    #[must_use]
    pub fn gate_open_at(&self, at: SimTime) -> bool {
        self.gate_open_until.is_some_and(|until| until > at)
    }

    #[must_use]
    pub const fn is_open(&self) -> bool {
        matches!(self.status, SiegeStatus::Open)
    }

    pub fn validate(&self) -> Result<(), CanwuError> {
        validate_id(self.id.as_str(), "siege")?;
        if self.schema_version != SCHEMA_VERSION || self.besiegers.is_empty() {
            return Err(invalid(format!("siege {} is invalid", self.id)));
        }
        Ok(())
    }
}
