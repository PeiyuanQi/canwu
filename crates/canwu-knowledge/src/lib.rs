//! Actor-relative information kept separate from world ground truth.

use canwu_core::{ArmyId, EventId, PersonId, TerritoryId};
use canwu_time::SimTime;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum KnowledgeSource {
    DirectObservation,
    CommandResponsibility,
    Report { source_event: EventId },
    ScenarioRecord,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EstimateRange {
    pub minimum: u32,
    pub maximum: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ArmyKnowledge {
    pub army: ArmyId,
    pub known_name: Option<String>,
    pub known_location: Option<TerritoryId>,
    pub estimated_strength: EstimateRange,
    pub observed_at: SimTime,
    pub learned_at: SimTime,
    pub confidence_per_mille: u16,
    pub source: KnowledgeSource,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ActorKnowledge {
    pub actor: PersonId,
    pub armies: BTreeMap<ArmyId, ArmyKnowledge>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct KnowledgeSnapshot {
    pub actors: BTreeMap<PersonId, ActorKnowledge>,
}

impl KnowledgeSnapshot {
    #[must_use]
    pub fn for_actor(&self, actor: PersonId) -> Option<&ActorKnowledge> {
        self.actors.get(&actor)
    }
}
