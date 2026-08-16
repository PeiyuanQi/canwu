use super::{
    Army, ArmyId, BoundaryRecord, CanwuError, CommandAttemptRecord, CommandRecord, CommitmentRoots,
    DomainRecord, DomainRecordRef, ErrorCode, Government, GovernmentId, IngressQueueKey,
    IngressRecord, KnowledgeSnapshot, Person, PersonId, PluginComponentKey, PluginComponentRecord,
    RandomDrawRecord, RandomStreamKey, RandomStreamState, Route, RouteId, RunConfigurationSnapshot,
    RunManifest, Scenario, ScheduleKey, ScheduledAction, SimEvent, SimTime, Territory, TerritoryId,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone)]
pub(super) struct RuntimeEvidence {
    pub(super) events: Vec<SimEvent>,
    pub(super) commands: Vec<CommandRecord>,
    pub(super) command_attempts: Vec<CommandAttemptRecord>,
    pub(super) ingress: Vec<IngressRecord>,
    pub(super) boundaries: Vec<BoundaryRecord>,
    pub(super) random_draws: Vec<RandomDrawRecord>,
}

#[derive(Clone)]
pub(super) struct OrderedCommitmentAccumulator {
    hasher: blake3::Hasher,
    pub(super) len: usize,
}

impl OrderedCommitmentAccumulator {
    fn new(domain: &str) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(domain.as_bytes());
        hasher.update(&[0]);
        hasher.update(b"[");
        Self { hasher, len: 0 }
    }

    fn from_sorted_by<T, K, F>(domain: &str, values: &[T], mut key: F) -> Result<Self, CanwuError>
    where
        T: Serialize,
        K: Ord,
        F: FnMut(&T) -> K,
    {
        let mut ordered: Vec<_> = values.iter().collect();
        ordered.sort_by_key(|value| key(value));
        let mut accumulator = Self::new(domain);
        for value in ordered {
            accumulator.append(value)?;
        }
        Ok(accumulator)
    }

    fn append<T: Serialize>(&mut self, value: &T) -> Result<(), CanwuError> {
        let encoded = serde_json::to_vec(value).map_err(|error| {
            CanwuError::new(
                ErrorCode::InvalidSnapshot,
                format!("could not encode incremental commitment item: {error}"),
            )
        })?;
        if self.len != 0 {
            self.hasher.update(b",");
        }
        self.hasher.update(&encoded);
        self.len = self.len.checked_add(1).ok_or_else(|| {
            CanwuError::new(
                ErrorCode::IdentifierExhausted,
                "incremental commitment item count is exhausted",
            )
        })?;
        Ok(())
    }

    fn sync_tail<T: Serialize>(&mut self, values: &[T]) -> Result<(), CanwuError> {
        if self.len > values.len() {
            return Err(CanwuError::new(
                ErrorCode::InvalidSnapshot,
                "append-only commitment cache exceeds its evidence journal",
            ));
        }
        for value in &values[self.len..] {
            self.append(value)?;
        }
        Ok(())
    }

    pub(super) fn root(&self) -> String {
        let mut hasher = self.hasher.clone();
        hasher.update(b"]");
        hasher.finalize().to_hex().to_string()
    }
}

#[derive(Clone)]
pub(super) struct RuntimeCommitmentCache {
    pub(super) commands: OrderedCommitmentAccumulator,
    pub(super) attempts: OrderedCommitmentAccumulator,
    pub(super) events: OrderedCommitmentAccumulator,
    pub(super) ingress: OrderedCommitmentAccumulator,
    pub(super) random_draws: OrderedCommitmentAccumulator,
    pub(super) world: Option<String>,
    pub(super) knowledge: Option<String>,
    pub(super) plugin_components: Option<String>,
    pub(super) domain_records: Option<String>,
    pub(super) scheduler: Option<String>,
    pub(super) random_streams: Option<String>,
    pub(super) identity: Option<String>,
}

impl RuntimeCommitmentCache {
    pub(super) fn from_evidence(evidence: &RuntimeEvidence) -> Result<Self, CanwuError> {
        Ok(Self {
            commands: OrderedCommitmentAccumulator::from_sorted_by(
                "canwu.commitment.commands.accepted.v1",
                &evidence.commands,
                |record| record.id,
            )?,
            attempts: OrderedCommitmentAccumulator::from_sorted_by(
                "canwu.commitment.commands.attempts.v1",
                &evidence.command_attempts,
                |record| record.id,
            )?,
            events: OrderedCommitmentAccumulator::from_sorted_by(
                "canwu.commitment.events.v1",
                &evidence.events,
                |event| event.id,
            )?,
            ingress: OrderedCommitmentAccumulator::from_sorted_by(
                "canwu.commitment.ingress.v1",
                &evidence.ingress,
                |record| record.id,
            )?,
            random_draws: OrderedCommitmentAccumulator::from_sorted_by(
                "canwu.commitment.random.draws.v1",
                &evidence.random_draws,
                |draw| draw.id,
            )?,
            world: None,
            knowledge: None,
            plugin_components: None,
            domain_records: None,
            scheduler: None,
            random_streams: None,
            identity: None,
        })
    }

    pub(super) fn sync(&mut self, evidence: &RuntimeEvidence) -> Result<(), CanwuError> {
        self.commands.sync_tail(&evidence.commands)?;
        self.attempts.sync_tail(&evidence.command_attempts)?;
        self.events.sync_tail(&evidence.events)?;
        self.ingress.sync_tail(&evidence.ingress)?;
        self.random_draws.sync_tail(&evidence.random_draws)?;
        Ok(())
    }

    pub(super) fn roots(&self) -> JournalCommitmentRoots {
        JournalCommitmentRoots {
            commands: self.commands.root(),
            attempts: self.attempts.root(),
            events: self.events.root(),
            ingress: self.ingress.root(),
            random_draws: self.random_draws.root(),
        }
    }

    pub(super) fn needs(&self) -> CommitmentDomains {
        let mut domains = CommitmentDomains::default();
        for (missing, domain) in [
            (self.world.is_none(), CommitmentDomains::WORLD),
            (self.knowledge.is_none(), CommitmentDomains::KNOWLEDGE),
            (
                self.plugin_components.is_none(),
                CommitmentDomains::PLUGIN_COMPONENTS,
            ),
            (
                self.domain_records.is_none(),
                CommitmentDomains::DOMAIN_RECORDS,
            ),
            (self.scheduler.is_none(), CommitmentDomains::SCHEDULER),
            (
                self.random_streams.is_none(),
                CommitmentDomains::RANDOM_STREAMS,
            ),
            (self.identity.is_none(), CommitmentDomains::IDENTITY),
        ] {
            if missing {
                domains.insert(domain);
            }
        }
        domains
    }

    pub(super) fn invalidate(&mut self, domains: CommitmentDomains) {
        if domains.contains(CommitmentDomains::WORLD) {
            self.world = None;
        }
        if domains.contains(CommitmentDomains::KNOWLEDGE) {
            self.knowledge = None;
        }
        if domains.contains(CommitmentDomains::PLUGIN_COMPONENTS) {
            self.plugin_components = None;
        }
        if domains.contains(CommitmentDomains::DOMAIN_RECORDS) {
            self.domain_records = None;
        }
        if domains.contains(CommitmentDomains::SCHEDULER) {
            self.scheduler = None;
        }
        if domains.contains(CommitmentDomains::RANDOM_STREAMS) {
            self.random_streams = None;
        }
        if domains.contains(CommitmentDomains::IDENTITY) {
            self.identity = None;
        }
    }

    pub(super) fn apply(&mut self, updates: RuntimeCommitmentRootUpdates) {
        if let Some(root) = updates.world {
            self.world = Some(root);
        }
        if let Some(root) = updates.knowledge {
            self.knowledge = Some(root);
        }
        if let Some(root) = updates.plugin_components {
            self.plugin_components = Some(root);
        }
        if let Some(root) = updates.domain_records {
            self.domain_records = Some(root);
        }
        if let Some(root) = updates.scheduler {
            self.scheduler = Some(root);
        }
        if let Some(root) = updates.random_streams {
            self.random_streams = Some(root);
        }
        if let Some(root) = updates.identity {
            self.identity = Some(root);
        }
    }

    pub(super) fn domain_roots(&self) -> Result<RuntimeDomainCommitmentRoots, CanwuError> {
        let required = |root: &Option<String>, domain: &str| {
            root.clone().ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::InvalidSnapshot,
                    format!("runtime commitment cache is missing its {domain} root"),
                )
            })
        };
        Ok(RuntimeDomainCommitmentRoots {
            world: required(&self.world, "world")?,
            knowledge: required(&self.knowledge, "knowledge")?,
            plugin_components: required(&self.plugin_components, "plugin-component")?,
            domain_records: required(&self.domain_records, "domain-record")?,
            scheduler: required(&self.scheduler, "scheduler")?,
            random_streams: required(&self.random_streams, "random-stream")?,
            identity: required(&self.identity, "identity")?,
        })
    }
}

#[derive(Clone)]
pub(super) struct JournalCommitmentRoots {
    pub(super) commands: String,
    pub(super) attempts: String,
    pub(super) events: String,
    pub(super) ingress: String,
    pub(super) random_draws: String,
}

#[derive(Clone, Copy, Default)]
pub(super) struct CommitmentDomains(u8);

impl CommitmentDomains {
    pub(super) const WORLD: Self = Self(1 << 0);
    pub(super) const KNOWLEDGE: Self = Self(1 << 1);
    pub(super) const PLUGIN_COMPONENTS: Self = Self(1 << 2);
    pub(super) const DOMAIN_RECORDS: Self = Self(1 << 3);
    pub(super) const SCHEDULER: Self = Self(1 << 4);
    pub(super) const RANDOM_STREAMS: Self = Self(1 << 5);
    pub(super) const IDENTITY: Self = Self(1 << 6);

    pub(super) const fn contains(self, domain: Self) -> bool {
        self.0 & domain.0 == domain.0
    }

    fn insert(&mut self, domain: Self) {
        self.0 |= domain.0;
    }
}

impl std::ops::BitOr for CommitmentDomains {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

#[derive(Default)]
pub(super) struct RuntimeCommitmentRootUpdates {
    pub(super) world: Option<String>,
    pub(super) knowledge: Option<String>,
    pub(super) plugin_components: Option<String>,
    pub(super) domain_records: Option<String>,
    pub(super) scheduler: Option<String>,
    pub(super) random_streams: Option<String>,
    pub(super) identity: Option<String>,
}

#[derive(Clone)]
pub(super) struct RuntimeDomainCommitmentRoots {
    pub(super) world: String,
    pub(super) knowledge: String,
    pub(super) plugin_components: String,
    pub(super) domain_records: String,
    pub(super) scheduler: String,
    pub(super) random_streams: String,
    pub(super) identity: String,
}

#[derive(Clone)]
pub(super) struct RuntimeScheduler {
    pub(super) initial_time: SimTime,
    pub(super) now: SimTime,
    pub(super) actions: BTreeMap<ScheduleKey, ScheduledAction>,
    pub(super) pending_ingress: BTreeSet<IngressQueueKey>,
}

#[derive(Clone)]
pub(super) struct RuntimeCounters {
    pub(super) next_event_id: u64,
    pub(super) next_command_id: u64,
    pub(super) next_command_attempt_id: u64,
    pub(super) next_ingress_id: u64,
    pub(super) next_boundary_id: u64,
    pub(super) next_random_draw_id: u64,
    pub(super) next_schedule_sequence: u64,
    pub(super) next_correlation_id: u64,
    pub(super) state_revision: u64,
    pub(super) admitted_attempt_count: u64,
    pub(super) admitted_command_count: u64,
    pub(super) admitted_event_count: u64,
}

#[derive(Clone)]
pub(super) struct RuntimeMetadata {
    pub(super) initial_scenario: Option<Scenario>,
    pub(super) run_manifest: RunManifest,
    pub(super) run_manifest_hash: String,
    pub(super) run_configuration: RunConfigurationSnapshot,
    pub(super) checkpoint_hash: String,
    pub(super) commitment_format_version: u32,
    pub(super) commitment_roots: Option<CommitmentRoots>,
    pub(super) commitment_cache: Option<RuntimeCommitmentCache>,
    pub(super) plugin_registration_closed: bool,
    pub(super) replay_revision_format_version: u32,
}

#[derive(Clone)]
pub(super) struct RuntimeCurrentState {
    pub(super) people: BTreeMap<PersonId, Person>,
    pub(super) governments: BTreeMap<GovernmentId, Government>,
    pub(super) territories: BTreeMap<TerritoryId, Territory>,
    pub(super) routes: BTreeMap<RouteId, Route>,
    pub(super) armies: BTreeMap<ArmyId, Army>,
    pub(super) knowledge: KnowledgeSnapshot,
    pub(super) plugin_components: BTreeMap<PluginComponentKey, PluginComponentRecord>,
    pub(super) domain_records: BTreeMap<DomainRecordRef, DomainRecord>,
    pub(super) root_seed: u64,
    pub(super) random_streams: BTreeMap<RandomStreamKey, RandomStreamState>,
}

#[derive(Clone)]
pub(super) struct RuntimeState {
    pub(super) current: RuntimeCurrentState,
    pub(super) scheduler: RuntimeScheduler,
    pub(super) counters: RuntimeCounters,
    pub(super) metadata: RuntimeMetadata,
    pub(super) evidence: RuntimeEvidence,
}
