use super::{
    Army, ArmyId, CommitmentRoots, DecisionState, DomainRecord, DomainRecordRef, IngressQueueKey,
    KnowledgeSnapshot, LetterCargo, LetterId, Person, PersonId, PluginComponentKey,
    PluginComponentRecord, RandomStreamKey, RandomStreamState, RuntimeCommitmentCache,
    RuntimeCounters, RuntimeScheduler, RuntimeState, ScheduleKey, ScheduledAction, SimTime,
};
use std::collections::BTreeMap;
use std::sync::Arc;

pub(super) struct RejectionTransactionCheckpoint {
    next_command_attempt_id: u64,
    state_revision: u64,
    plugin_registration_closed: bool,
    command_attempt_count: usize,
    checkpoint_hash: String,
    commitment_roots: Option<CommitmentRoots>,
    commitment_cache: Option<RuntimeCommitmentCache>,
}

impl RejectionTransactionCheckpoint {
    pub(super) fn capture(state: &RuntimeState) -> Self {
        Self {
            next_command_attempt_id: state.counters.next_command_attempt_id,
            state_revision: state.counters.state_revision,
            plugin_registration_closed: state.metadata.plugin_registration_closed,
            command_attempt_count: state.evidence.command_attempts.len(),
            checkpoint_hash: state.metadata.checkpoint_hash.clone(),
            commitment_roots: state.metadata.commitment_roots.clone(),
            commitment_cache: state.metadata.commitment_cache.clone(),
        }
    }

    pub(super) fn restore(self, state: &mut RuntimeState) {
        state.counters.next_command_attempt_id = self.next_command_attempt_id;
        state.counters.state_revision = self.state_revision;
        state.metadata.plugin_registration_closed = self.plugin_registration_closed;
        state
            .evidence
            .command_attempts
            .truncate(self.command_attempt_count);
        state.metadata.checkpoint_hash = self.checkpoint_hash;
        state.metadata.commitment_roots = self.commitment_roots;
        state.metadata.commitment_cache = self.commitment_cache;
    }
}

pub(super) struct IngressTransactionCheckpoint {
    next_ingress_id: u64,
    ingress_count: usize,
    plugin_registration_closed: bool,
    checkpoint_hash: String,
    commitment_roots: Option<CommitmentRoots>,
    commitment_cache: Option<RuntimeCommitmentCache>,
}

impl IngressTransactionCheckpoint {
    pub(super) fn capture(state: &RuntimeState) -> Self {
        Self {
            next_ingress_id: state.counters.next_ingress_id,
            ingress_count: state.evidence.ingress.len(),
            plugin_registration_closed: state.metadata.plugin_registration_closed,
            checkpoint_hash: state.metadata.checkpoint_hash.clone(),
            commitment_roots: state.metadata.commitment_roots.clone(),
            commitment_cache: state.metadata.commitment_cache.clone(),
        }
    }

    pub(super) fn restore(self, state: &mut RuntimeState, queue_key: &IngressQueueKey) {
        state.counters.next_ingress_id = self.next_ingress_id;
        state.scheduler.pending_ingress.remove(queue_key);
        state.evidence.ingress.truncate(self.ingress_count);
        state.metadata.plugin_registration_closed = self.plugin_registration_closed;
        state.metadata.checkpoint_hash = self.checkpoint_hash;
        state.metadata.commitment_roots = self.commitment_roots;
        state.metadata.commitment_cache = self.commitment_cache;
    }
}

pub(super) struct CommandTransactionCheckpoint {
    people: BTreeMap<PersonId, Person>,
    letters: BTreeMap<LetterId, LetterCargo>,
    armies: BTreeMap<ArmyId, Army>,
    knowledge: KnowledgeSnapshot,
    plugin_components: BTreeMap<PluginComponentKey, PluginComponentRecord>,
    scheduled_actions: BTreeMap<ScheduleKey, ScheduledAction>,
    counters: RuntimeCounters,
    event_count: usize,
    command_count: usize,
    command_attempt_count: usize,
    plugin_registration_closed: bool,
    checkpoint_hash: String,
    commitment_roots: Option<CommitmentRoots>,
    commitment_cache: Option<RuntimeCommitmentCache>,
}

impl CommandTransactionCheckpoint {
    pub(super) fn capture(state: &RuntimeState) -> Self {
        Self {
            people: state.current.people.clone(),
            letters: state.current.letters.clone(),
            armies: state.current.armies.clone(),
            knowledge: state.current.knowledge.clone(),
            plugin_components: state.current.plugin_components.clone(),
            scheduled_actions: state.scheduler.actions.clone(),
            counters: state.counters.clone(),
            event_count: state.evidence.events.len(),
            command_count: state.evidence.commands.len(),
            command_attempt_count: state.evidence.command_attempts.len(),
            plugin_registration_closed: state.metadata.plugin_registration_closed,
            checkpoint_hash: state.metadata.checkpoint_hash.clone(),
            commitment_roots: state.metadata.commitment_roots.clone(),
            commitment_cache: state.metadata.commitment_cache.clone(),
        }
    }

    pub(super) fn restore(self, state: &mut RuntimeState) {
        state.current.people = self.people;
        state.current.letters = self.letters;
        state.current.armies = self.armies;
        state.current.knowledge = self.knowledge;
        state.current.plugin_components = self.plugin_components;
        state.scheduler.actions = self.scheduled_actions;
        state.counters = self.counters;
        state.evidence.events.truncate(self.event_count);
        state.evidence.commands.truncate(self.command_count);
        state
            .evidence
            .command_attempts
            .truncate(self.command_attempt_count);
        state.metadata.plugin_registration_closed = self.plugin_registration_closed;
        state.metadata.checkpoint_hash = self.checkpoint_hash;
        state.metadata.commitment_roots = self.commitment_roots;
        state.metadata.commitment_cache = self.commitment_cache;
    }
}

pub(super) struct BoundaryTransactionCheckpoint {
    people: BTreeMap<PersonId, Person>,
    letters: BTreeMap<LetterId, LetterCargo>,
    armies: BTreeMap<ArmyId, Army>,
    knowledge: KnowledgeSnapshot,
    plugin_components: BTreeMap<PluginComponentKey, PluginComponentRecord>,
    domain_records: Arc<BTreeMap<DomainRecordRef, DomainRecord>>,
    decisions: DecisionState,
    random_streams: BTreeMap<RandomStreamKey, RandomStreamState>,
    scheduler: RuntimeScheduler,
    counters: RuntimeCounters,
    event_count: usize,
    command_count: usize,
    command_attempt_count: usize,
    ingress_count: usize,
    boundary_count: usize,
    random_draw_count: usize,
    plugin_registration_closed: bool,
    checkpoint_hash: String,
    commitment_roots: Option<CommitmentRoots>,
    commitment_cache: Option<RuntimeCommitmentCache>,
}

impl BoundaryTransactionCheckpoint {
    pub(super) fn capture(state: &RuntimeState) -> Self {
        Self {
            people: state.current.people.clone(),
            letters: state.current.letters.clone(),
            armies: state.current.armies.clone(),
            knowledge: state.current.knowledge.clone(),
            plugin_components: state.current.plugin_components.clone(),
            domain_records: state.current.domain_records.clone(),
            decisions: state.current.decisions.clone(),
            random_streams: state.current.random_streams.clone(),
            scheduler: state.scheduler.clone(),
            counters: state.counters.clone(),
            event_count: state.evidence.events.len(),
            command_count: state.evidence.commands.len(),
            command_attempt_count: state.evidence.command_attempts.len(),
            ingress_count: state.evidence.ingress.len(),
            boundary_count: state.evidence.boundaries.len(),
            random_draw_count: state.evidence.random_draws.len(),
            plugin_registration_closed: state.metadata.plugin_registration_closed,
            checkpoint_hash: state.metadata.checkpoint_hash.clone(),
            commitment_roots: state.metadata.commitment_roots.clone(),
            commitment_cache: state.metadata.commitment_cache.clone(),
        }
    }

    pub(super) fn restore(self, state: &mut RuntimeState) {
        state.current.people = self.people;
        state.current.letters = self.letters;
        state.current.armies = self.armies;
        state.current.knowledge = self.knowledge;
        state.current.plugin_components = self.plugin_components;
        state.current.domain_records = self.domain_records;
        state.current.decisions = self.decisions;
        state.current.random_streams = self.random_streams;
        state.scheduler = self.scheduler;
        state.counters = self.counters;
        state.evidence.events.truncate(self.event_count);
        state.evidence.commands.truncate(self.command_count);
        state
            .evidence
            .command_attempts
            .truncate(self.command_attempt_count);
        state.evidence.ingress.truncate(self.ingress_count);
        state.evidence.boundaries.truncate(self.boundary_count);
        state.evidence.random_draws.truncate(self.random_draw_count);
        state.metadata.plugin_registration_closed = self.plugin_registration_closed;
        state.metadata.checkpoint_hash = self.checkpoint_hash;
        state.metadata.commitment_roots = self.commitment_roots;
        state.metadata.commitment_cache = self.commitment_cache;
    }
}

pub(super) struct ScheduledBatchTransactionCheckpoint {
    people: BTreeMap<PersonId, Person>,
    letters: BTreeMap<LetterId, LetterCargo>,
    armies: BTreeMap<ArmyId, Army>,
    knowledge: KnowledgeSnapshot,
    plugin_components: BTreeMap<PluginComponentKey, PluginComponentRecord>,
    random_streams: BTreeMap<RandomStreamKey, RandomStreamState>,
    now: SimTime,
    scheduled_actions: BTreeMap<ScheduleKey, ScheduledAction>,
    counters: RuntimeCounters,
    event_count: usize,
    random_draw_count: usize,
    plugin_registration_closed: bool,
    checkpoint_hash: String,
    commitment_roots: Option<CommitmentRoots>,
    commitment_cache: Option<RuntimeCommitmentCache>,
}

impl ScheduledBatchTransactionCheckpoint {
    pub(super) fn capture(state: &RuntimeState) -> Self {
        Self {
            people: state.current.people.clone(),
            letters: state.current.letters.clone(),
            armies: state.current.armies.clone(),
            knowledge: state.current.knowledge.clone(),
            plugin_components: state.current.plugin_components.clone(),
            random_streams: state.current.random_streams.clone(),
            now: state.scheduler.now,
            scheduled_actions: state.scheduler.actions.clone(),
            counters: state.counters.clone(),
            event_count: state.evidence.events.len(),
            random_draw_count: state.evidence.random_draws.len(),
            plugin_registration_closed: state.metadata.plugin_registration_closed,
            checkpoint_hash: state.metadata.checkpoint_hash.clone(),
            commitment_roots: state.metadata.commitment_roots.clone(),
            commitment_cache: state.metadata.commitment_cache.clone(),
        }
    }

    pub(super) fn restore(self, state: &mut RuntimeState) {
        state.current.people = self.people;
        state.current.letters = self.letters;
        state.current.armies = self.armies;
        state.current.knowledge = self.knowledge;
        state.current.plugin_components = self.plugin_components;
        state.current.random_streams = self.random_streams;
        state.scheduler.now = self.now;
        state.scheduler.actions = self.scheduled_actions;
        state.counters = self.counters;
        state.evidence.events.truncate(self.event_count);
        state.evidence.random_draws.truncate(self.random_draw_count);
        state.metadata.plugin_registration_closed = self.plugin_registration_closed;
        state.metadata.checkpoint_hash = self.checkpoint_hash;
        state.metadata.commitment_roots = self.commitment_roots;
        state.metadata.commitment_cache = self.commitment_cache;
    }
}

pub(super) struct ClockTransactionCheckpoint {
    now: SimTime,
    plugin_registration_closed: bool,
    checkpoint_hash: String,
    commitment_roots: Option<CommitmentRoots>,
    commitment_cache: Option<RuntimeCommitmentCache>,
}

impl ClockTransactionCheckpoint {
    pub(super) fn capture(state: &RuntimeState) -> Self {
        Self {
            now: state.scheduler.now,
            plugin_registration_closed: state.metadata.plugin_registration_closed,
            checkpoint_hash: state.metadata.checkpoint_hash.clone(),
            commitment_roots: state.metadata.commitment_roots.clone(),
            commitment_cache: state.metadata.commitment_cache.clone(),
        }
    }

    pub(super) fn restore(self, state: &mut RuntimeState) {
        state.scheduler.now = self.now;
        state.metadata.plugin_registration_closed = self.plugin_registration_closed;
        state.metadata.checkpoint_hash = self.checkpoint_hash;
        state.metadata.commitment_roots = self.commitment_roots;
        state.metadata.commitment_cache = self.commitment_cache;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Scenario, Simulation};

    #[test]
    fn boundary_checkpoint_shares_domain_record_root() {
        let simulation = Simulation::new(7, Scenario::new(SimTime::EPOCH, Vec::new())).unwrap();
        let checkpoint = BoundaryTransactionCheckpoint::capture(&simulation.state);

        assert!(Arc::ptr_eq(
            &checkpoint.domain_records,
            &simulation.state.current.domain_records
        ));
    }
}
