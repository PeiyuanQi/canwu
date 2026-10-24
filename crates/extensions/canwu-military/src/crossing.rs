//! Contested crossings: waves against an interceptor into a bridgehead.

use crate::engagement::{ConditionInputV1, EnvironmentInputV1};
use crate::ids::{CrossingId, EngagementId, ForceId, SCHEMA_VERSION, invalid, validate_id};
use canwu_api::{
    CanwuError, DomainRecordVersionRef, EntityRef, KnowledgeHolderRef, RandomDrawId, SimTime,
};
use serde::{Deserialize, Serialize};

/// A force that controls the water a crossing or a siege uses, as the host
/// derives it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InterceptorInputV1 {
    pub force: ForceId,
    pub control_bp: u32,
    pub scouts: u32,
    pub evidence: Vec<DomainRecordVersionRef>,
}

impl InterceptorInputV1 {
    /// Refuses a value above the plugin's bounds (`ValueOutOfRange`).
    pub fn check_ranges(&self) -> Result<(), CanwuError> {
        crate::rulebook::bounded(self.control_bp, crate::MAX_INPUT_BP, "control_bp")?;
        crate::rulebook::bounded(self.scouts, 1_000_000, "scouts")
    }
}

/// The latest wave of a crossing.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WaveV1 {
    pub index: u32,
    pub at: SimTime,
    pub men: u64,
    pub intercepted: bool,
    pub intercept_bp: Option<i64>,
    pub draw: Option<RandomDrawId>,
    pub engagement: Option<EngagementId>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CrossingStatus {
    Open,
    Complete,
    Cancelled,
}

/// A contested crossing (record `canwu.military:crossing`).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CrossingStateV1 {
    pub schema_version: u32,
    pub id: CrossingId,
    pub host_revision: u64,
    pub force: ForceId,
    pub from: EntityRef,
    pub to: EntityRef,
    pub night: bool,
    pub defenders: Vec<ForceId>,
    /// The crossing side's holders, the defending side's holders.
    pub holders: [Vec<KnowledgeHolderRef>; 2],
    /// The crossing side's latest input, the defending side's latest input.
    pub inputs: [Option<(ConditionInputV1, SimTime)>; 2],
    pub environment: Option<(EnvironmentInputV1, SimTime)>,
    pub interceptor: Option<InterceptorInputV1>,
    pub waves_sent: u32,
    pub landed: u64,
    pub bridgehead_secure: bool,
    pub next_wave_at: SimTime,
    pub last_wave: Option<WaveV1>,
    /// The ferry or bridgehead fight in progress; waves wait while it is open.
    pub engagement: Option<EngagementId>,
    pub status: CrossingStatus,
    pub ended_at: Option<SimTime>,
}

impl CrossingStateV1 {
    #[must_use]
    pub const fn is_open(&self) -> bool {
        matches!(self.status, CrossingStatus::Open)
    }

    pub fn validate(&self) -> Result<(), CanwuError> {
        validate_id(self.id.as_str(), "crossing")?;
        if self.schema_version != SCHEMA_VERSION {
            return Err(invalid(format!("crossing {} is invalid", self.id)));
        }
        Ok(())
    }
}
