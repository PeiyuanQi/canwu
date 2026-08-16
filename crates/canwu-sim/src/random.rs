use crate::{CanwuError, ErrorCode};
use canwu_core::{ArmyId, BoundaryId, DeterministicRng, EventId, PersonId, RandomDrawId};
use canwu_event::CauseRef;
use canwu_time::SimTime;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const STREAM_DERIVATION_DOMAIN: &[u8] = b"canwu.random-stream.v1";

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RandomAlgorithm {
    #[default]
    SplitMix64V1,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct RandomStreamKey {
    pub namespace: String,
    pub name: String,
    pub version: u32,
}

impl RandomStreamKey {
    #[must_use]
    pub fn new(namespace: impl Into<String>, name: impl Into<String>, version: u32) -> Self {
        Self {
            namespace: namespace.into(),
            name: name.into(),
            version,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RandomStreamState {
    pub key: RandomStreamKey,
    pub algorithm: RandomAlgorithm,
    pub seed: u64,
    pub position: u64,
    pub generator_state: u64,
}

impl RandomStreamState {
    pub(crate) fn initial(root_seed: u64, key: RandomStreamKey) -> Self {
        let seed = derive_stream_seed(root_seed, &key);
        Self {
            key,
            algorithm: RandomAlgorithm::SplitMix64V1,
            seed,
            position: 0,
            generator_state: seed,
        }
    }

    pub(crate) fn is_coherent(&self, root_seed: u64) -> bool {
        self.algorithm == RandomAlgorithm::SplitMix64V1
            && self.seed == derive_stream_seed(root_seed, &self.key)
            && self.generator_state == DeterministicRng::state_after(self.seed, self.position)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RandomDrawProducer {
    BoundarySystem {
        boundary: BoundaryId,
        plugin: String,
        system: String,
    },
    CoreSystem {
        system: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RandomDrawOutcome {
    BoundarySystemDecision,
    KnowledgeReportDelivery {
        recipient: PersonId,
        army: ArmyId,
        dispatch_event: EventId,
        arrives_at: SimTime,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RandomDrawRecord {
    pub id: RandomDrawId,
    pub at: SimTime,
    pub stream: RandomStreamKey,
    pub position: u64,
    pub upper_exclusive: u64,
    pub value: u64,
    pub purpose: String,
    pub producer: RandomDrawProducer,
    #[serde(default)]
    pub outcome: Option<RandomDrawOutcome>,
    pub cause: CauseRef,
    pub correlation_id: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct PendingRandomDraw {
    pub stream: RandomStreamKey,
    pub position: u64,
    pub upper_exclusive: u64,
    pub value: u64,
    pub purpose: String,
}

pub(crate) struct RandomExecution {
    pub states: BTreeMap<RandomStreamKey, RandomStreamState>,
    pub draws: Vec<PendingRandomDraw>,
}

pub(crate) struct RandomSession {
    states: BTreeMap<RandomStreamKey, RandomStreamState>,
    draws: Vec<PendingRandomDraw>,
}

impl RandomSession {
    pub(crate) fn new(
        available: &BTreeMap<RandomStreamKey, RandomStreamState>,
        allowed: &[RandomStreamKey],
    ) -> Result<Self, CanwuError> {
        let mut states = BTreeMap::new();
        for key in allowed {
            let Some(state) = available.get(key) else {
                return Err(CanwuError::new(
                    ErrorCode::InvalidRandomStream,
                    format!(
                        "declared random stream {}.{}@{} is not initialized",
                        key.namespace, key.name, key.version
                    ),
                ));
            };
            states.insert(key.clone(), state.clone());
        }
        Ok(Self {
            states,
            draws: Vec::new(),
        })
    }

    pub(crate) fn range(
        &mut self,
        key: &RandomStreamKey,
        upper_exclusive: u64,
        purpose: &str,
    ) -> Result<u64, CanwuError> {
        if upper_exclusive == 0 || purpose.trim().is_empty() || purpose != purpose.trim() {
            return Err(CanwuError::new(
                ErrorCode::InvalidRandomDraw,
                "random draws require a positive bound and canonical purpose",
            ));
        }
        let Some(state) = self.states.get_mut(key) else {
            return Err(CanwuError::new(
                ErrorCode::UndeclaredRandomStream,
                format!(
                    "random stream {}.{}@{} was not declared by this system",
                    key.namespace, key.name, key.version
                ),
            ));
        };
        let next_position = state.position.checked_add(1).ok_or_else(|| {
            CanwuError::new(
                ErrorCode::IdentifierExhausted,
                "random stream position is exhausted",
            )
        })?;
        let position = state.position;
        let mut generator = DeterministicRng::from_seed(state.generator_state);
        let value = generator.range(upper_exclusive);
        state.position = next_position;
        state.generator_state = generator.state();
        self.draws.push(PendingRandomDraw {
            stream: key.clone(),
            position,
            upper_exclusive,
            value,
            purpose: purpose.to_owned(),
        });
        Ok(value)
    }

    pub(crate) fn finish(self) -> RandomExecution {
        RandomExecution {
            states: self.states,
            draws: self.draws,
        }
    }
}

pub(crate) fn derive_stream_seed(root_seed: u64, key: &RandomStreamKey) -> u64 {
    if key.namespace == "canwu.core" && key.name == "knowledge-report-delay" && key.version == 1 {
        return root_seed;
    }
    let mut hasher = blake3::Hasher::new();
    hasher.update(STREAM_DERIVATION_DOMAIN);
    hasher.update(&root_seed.to_le_bytes());
    update_text(&mut hasher, &key.namespace);
    update_text(&mut hasher, &key.name);
    hasher.update(&key.version.to_le_bytes());
    let mut seed = [0_u8; 8];
    seed.copy_from_slice(&hasher.finalize().as_bytes()[..8]);
    u64::from_le_bytes(seed)
}

pub(crate) fn core_report_delay_stream() -> RandomStreamKey {
    RandomStreamKey::new("canwu.core", "knowledge-report-delay", 1)
}

fn update_text(hasher: &mut blake3::Hasher, value: &str) {
    let length = u64::try_from(value.len()).unwrap_or(u64::MAX);
    hasher.update(&length.to_le_bytes());
    hasher.update(value.as_bytes());
}
