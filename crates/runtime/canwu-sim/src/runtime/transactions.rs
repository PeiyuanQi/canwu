use super::{
    Army, ArmyId, CommitmentRoots, DecisionState, DomainRecordRef, EntityRef, IngressQueueKey,
    KnowledgeSnapshot, LetterCargo, LetterId, PersistentDomainRecordStore, Person,
    PersonAvailability, PersonId, PluginComponentKey, PluginComponentRecord, RandomStreamKey,
    RandomStreamState, RuntimeCommitmentCache, RuntimeCounters, RuntimeScheduler, RuntimeState,
    ScheduleKey, ScheduledAction, SimTime,
};
use std::sync::Arc;

pub(super) struct RejectionTransactionCheckpoint {
    next_command_attempt_id: u64,
    state_revision: u64,
    plugin_registration_closed: bool,
    command_attempt_count: usize,
    checkpoint_hash: String,
    commitment_roots: Option<CommitmentRoots>,
    commitment_cache: Option<Box<RuntimeCommitmentCache>>,
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
            .truncate_command_history(usize::MAX, self.command_attempt_count, usize::MAX);
        state.metadata.checkpoint_hash = self.checkpoint_hash;
        state.metadata.commitment_roots = self.commitment_roots;
        state.metadata.commitment_cache = self.commitment_cache;
    }
}

pub(super) struct IngressTransactionCheckpoint {
    next_ingress_id: u64,
    ingress_count: usize,
    /// Shares structure with the live pins, so the capture copies nothing.
    pinned: super::continuation::PinnedEvidence,
    plugin_registration_closed: bool,
    checkpoint_hash: String,
    commitment_roots: Option<CommitmentRoots>,
    commitment_cache: Option<Box<RuntimeCommitmentCache>>,
}

impl IngressTransactionCheckpoint {
    pub(super) fn capture(state: &RuntimeState) -> Self {
        Self {
            next_ingress_id: state.counters.next_ingress_id,
            ingress_count: state.evidence.ingress.len(),
            pinned: state.evidence.pinned.clone(),
            plugin_registration_closed: state.metadata.plugin_registration_closed,
            checkpoint_hash: state.metadata.checkpoint_hash.clone(),
            commitment_roots: state.metadata.commitment_roots.clone(),
            commitment_cache: state.metadata.commitment_cache.clone(),
        }
    }

    pub(super) fn restore(self, state: &mut RuntimeState, queue_key: &IngressQueueKey) {
        state.counters.next_ingress_id = self.next_ingress_id;
        state.scheduler.pending_ingress.remove(queue_key);
        state
            .evidence
            .truncate_command_history(usize::MAX, usize::MAX, self.ingress_count);
        state.metadata.plugin_registration_closed = self.plugin_registration_closed;
        state.metadata.checkpoint_hash = self.checkpoint_hash;
        state.metadata.commitment_roots = self.commitment_roots;
        state.metadata.commitment_cache = self.commitment_cache;
    }

    /// Restores a failed cancellation append and requeues its exact target.
    pub(super) fn restore_cancellation(self, state: &mut RuntimeState, target: IngressQueueKey) {
        state.counters.next_ingress_id = self.next_ingress_id;
        state.scheduler.pending_ingress.insert(target);
        state.scheduler.cancelled_ingress.remove(&target.id);
        state
            .evidence
            .truncate_command_history(usize::MAX, usize::MAX, self.ingress_count);
        state.evidence.pinned = self.pinned;
        state.metadata.plugin_registration_closed = self.plugin_registration_closed;
        state.metadata.checkpoint_hash = self.checkpoint_hash;
        state.metadata.commitment_roots = self.commitment_roots;
        state.metadata.commitment_cache = self.commitment_cache;
    }
}

/// The rollback copy of one command. Every collection it holds is a
/// persistent structure or an `Arc` shared with the live state, so the
/// capture copies no entries: its cost does not grow with the world.
pub(super) struct CommandTransactionCheckpoint {
    people: im::OrdMap<PersonId, Arc<Person>>,
    letters: im::OrdMap<LetterId, Arc<LetterCargo>>,
    armies: im::OrdMap<ArmyId, Arc<Army>>,
    knowledge: std::sync::Arc<KnowledgeSnapshot>,
    plugin_components: im::OrdMap<Arc<PluginComponentKey>, Arc<PluginComponentRecord>>,
    scheduled_actions: im::OrdMap<ScheduleKey, Arc<ScheduledAction>>,
    counters: RuntimeCounters,
    event_count: usize,
    command_count: usize,
    command_attempt_count: usize,
    plugin_registration_closed: bool,
    checkpoint_hash: String,
    commitment_roots: Option<CommitmentRoots>,
    commitment_cache: Option<Box<RuntimeCommitmentCache>>,
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
        state.evidence.truncate_command_history(
            self.command_count,
            self.command_attempt_count,
            usize::MAX,
        );
        state.metadata.plugin_registration_closed = self.plugin_registration_closed;
        state.metadata.checkpoint_hash = self.checkpoint_hash;
        state.metadata.commitment_roots = self.commitment_roots;
        state.metadata.commitment_cache = self.commitment_cache;
    }
}

pub(super) struct BoundaryTransactionCheckpoint {
    entities: im::OrdSet<EntityRef>,
    people: im::OrdMap<PersonId, Arc<Person>>,
    person_availability: im::OrdMap<PersonId, PersonAvailability>,
    created_persons: im::Vector<super::CreatedPerson>,
    letters: im::OrdMap<LetterId, Arc<LetterCargo>>,
    armies: im::OrdMap<ArmyId, Arc<Army>>,
    knowledge: std::sync::Arc<KnowledgeSnapshot>,
    plugin_components: im::OrdMap<Arc<PluginComponentKey>, Arc<PluginComponentRecord>>,
    domain_records: PersistentDomainRecordStore,
    decisions: DecisionState,
    random_streams: im::OrdMap<RandomStreamKey, RandomStreamState>,
    scheduler: RuntimeScheduler,
    counters: RuntimeCounters,
    event_count: usize,
    command_count: usize,
    command_attempt_count: usize,
    ingress_count: usize,
    boundary_count: usize,
    random_draw_count: usize,
    /// Shares structure with the live index, so the capture copies nothing.
    keyed_draws: super::random::KeyedDrawLedger,
    /// Shares structure with the live pins, so the capture copies nothing.
    pinned: super::continuation::PinnedEvidence,
    plugin_registration_closed: bool,
    current_domain_record_versions: im::OrdMap<
        std::sync::Arc<DomainRecordRef>,
        std::sync::Arc<super::CurrentDomainRecordVersion>,
    >,
    checkpoint_hash: String,
    commitment_roots: Option<CommitmentRoots>,
    commitment_cache: Option<Box<RuntimeCommitmentCache>>,
}

impl BoundaryTransactionCheckpoint {
    pub(super) fn capture(state: &RuntimeState) -> Self {
        Self {
            entities: state.current.entities.clone(),
            people: state.current.people.clone(),
            person_availability: state.current.person_availability.clone(),
            created_persons: state.current.created_persons.clone(),
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
            keyed_draws: state.evidence.keyed_draws.clone(),
            pinned: state.evidence.pinned.clone(),
            plugin_registration_closed: state.metadata.plugin_registration_closed,
            current_domain_record_versions: state.metadata.current_domain_record_versions.clone(),
            checkpoint_hash: state.metadata.checkpoint_hash.clone(),
            commitment_roots: state.metadata.commitment_roots.clone(),
            commitment_cache: state.metadata.commitment_cache.clone(),
        }
    }

    pub(super) fn restore(self, state: &mut RuntimeState) {
        state.current.entities = self.entities;
        state.current.people = self.people;
        state.current.person_availability = self.person_availability;
        state.current.created_persons = self.created_persons;
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
        state.evidence.truncate_command_history(
            self.command_count,
            self.command_attempt_count,
            self.ingress_count,
        );
        state.evidence.boundaries.truncate(self.boundary_count);
        state.evidence.random_draws.truncate(self.random_draw_count);
        state.evidence.keyed_draws = self.keyed_draws;
        state.evidence.pinned = self.pinned;
        state.metadata.plugin_registration_closed = self.plugin_registration_closed;
        state.metadata.current_domain_record_versions = self.current_domain_record_versions;
        state.metadata.checkpoint_hash = self.checkpoint_hash;
        state.metadata.commitment_roots = self.commitment_roots;
        state.metadata.commitment_cache = self.commitment_cache;
    }
}

pub(super) struct ScheduledBatchTransactionCheckpoint {
    people: im::OrdMap<PersonId, Arc<Person>>,
    letters: im::OrdMap<LetterId, Arc<LetterCargo>>,
    armies: im::OrdMap<ArmyId, Arc<Army>>,
    knowledge: std::sync::Arc<KnowledgeSnapshot>,
    plugin_components: im::OrdMap<Arc<PluginComponentKey>, Arc<PluginComponentRecord>>,
    random_streams: im::OrdMap<RandomStreamKey, RandomStreamState>,
    now: SimTime,
    scheduled_actions: im::OrdMap<ScheduleKey, Arc<ScheduledAction>>,
    counters: RuntimeCounters,
    event_count: usize,
    random_draw_count: usize,
    plugin_registration_closed: bool,
    checkpoint_hash: String,
    commitment_roots: Option<CommitmentRoots>,
    commitment_cache: Option<Box<RuntimeCommitmentCache>>,
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
    commitment_cache: Option<Box<RuntimeCommitmentCache>>,
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
    use crate::{Command, CommandEnvelope, Issuer, Scenario, Simulation, demo_scenario};
    use canwu_time::SimDuration;

    #[test]
    fn boundary_checkpoint_shares_domain_record_root() {
        let simulation = Simulation::new(7, Scenario::new(SimTime::EPOCH, Vec::new())).unwrap();
        let checkpoint = BoundaryTransactionCheckpoint::capture(&simulation.state);

        assert!(
            checkpoint
                .domain_records
                .shares_root_with(&simulation.state.current.domain_records)
        );
    }

    /// Every rollback capture and a fork share the current state's
    /// collections and the evidence journals with the live run instead of
    /// copying them, so their cost does not grow with the world or its
    /// history. A change on either side then copies only what it changes and
    /// leaves the other side as it was.
    #[test]
    fn rollback_copies_and_forks_share_state_and_journals() {
        let (scenario, ids) = demo_scenario();
        let mut simulation = Simulation::new(35, scenario).unwrap();
        simulation
            .submit(CommandEnvelope::new(
                Issuer::Actor(ids.commander),
                Command::OrderMovement {
                    subject: EntityRef::Army(ids.army),
                    destination: ids.eastern_territory,
                    cargo: Vec::new(),
                },
            ))
            .unwrap();
        let live = &simulation.state;
        assert!(!live.scheduler.actions.is_empty() && !live.evidence.events.is_empty());

        let command = CommandTransactionCheckpoint::capture(live);
        assert!(command.people.ptr_eq(&live.current.people));
        assert!(command.letters.ptr_eq(&live.current.letters));
        assert!(command.armies.ptr_eq(&live.current.armies));
        assert!(
            command
                .plugin_components
                .ptr_eq(&live.current.plugin_components)
        );
        assert!(command.scheduled_actions.ptr_eq(&live.scheduler.actions));

        let boundary = BoundaryTransactionCheckpoint::capture(live);
        assert!(boundary.entities.ptr_eq(&live.current.entities));
        assert!(boundary.people.ptr_eq(&live.current.people));
        assert!(
            boundary
                .person_availability
                .ptr_eq(&live.current.person_availability)
        );
        assert!(boundary.armies.ptr_eq(&live.current.armies));
        assert!(boundary.random_streams.ptr_eq(&live.current.random_streams));
        assert!(boundary.scheduler.actions.ptr_eq(&live.scheduler.actions));
        assert!(
            boundary
                .scheduler
                .pending_ingress
                .ptr_eq(&live.scheduler.pending_ingress)
        );

        let batch = ScheduledBatchTransactionCheckpoint::capture(live);
        assert!(batch.armies.ptr_eq(&live.current.armies));
        assert!(batch.random_streams.ptr_eq(&live.current.random_streams));
        assert!(batch.scheduled_actions.ptr_eq(&live.scheduler.actions));

        let before = simulation.snapshot();
        let mut fork = simulation.fork();
        let (copy, live) = (&fork.state, &simulation.state);
        assert!(copy.current.armies.ptr_eq(&live.current.armies));
        assert!(copy.current.territories.ptr_eq(&live.current.territories));
        assert!(copy.scheduler.actions.ptr_eq(&live.scheduler.actions));
        assert!(Arc::ptr_eq(
            copy.metadata.initial_scenario.as_ref().unwrap(),
            live.metadata.initial_scenario.as_ref().unwrap()
        ));
        assert!(
            copy.evidence
                .events
                .shares_records_with(&live.evidence.events)
        );
        assert!(
            copy.evidence
                .command_attempts
                .shares_records_with(&live.evidence.command_attempts)
        );
        assert!(
            copy.evidence
                .commands
                .shares_records_with(&live.evidence.commands)
        );

        // The fork moves the army and appends evidence; the run it was
        // taken from is unchanged, and advancing it gives the same state.
        fork.advance(SimDuration::days(4)).unwrap();
        assert!(
            !fork
                .state
                .current
                .armies
                .ptr_eq(&simulation.state.current.armies)
        );
        assert!(fork.state.evidence.events.len() > simulation.state.evidence.events.len());
        assert_eq!(simulation.snapshot(), before);
        simulation.advance(SimDuration::days(4)).unwrap();
        assert_eq!(simulation.snapshot(), fork.snapshot());

        // A truncation or a front drain of a shared journal keeps the other
        // copy whole.
        let events = simulation.state.evidence.events.to_vec();
        let mut truncated = simulation.state.evidence.events.clone();
        truncated.truncate(1);
        let mut drained = simulation.state.evidence.events.clone();
        drained.drain(..1);
        assert_eq!(&*truncated, &events[..1]);
        assert_eq!(&*drained, &events[1..]);
        assert_eq!(&*simulation.state.evidence.events, events.as_slice());
    }

    /// Every rollback capture shares the knowledge state instead of copying
    /// it, so a transaction costs nothing in proportion to held knowledge.
    #[test]
    fn transaction_checkpoints_share_knowledge() {
        let simulation = Simulation::new(7, Scenario::new(SimTime::EPOCH, Vec::new())).unwrap();
        let knowledge = &simulation.state.current.knowledge;
        assert!(std::sync::Arc::ptr_eq(
            &BoundaryTransactionCheckpoint::capture(&simulation.state).knowledge,
            knowledge
        ));
        assert!(std::sync::Arc::ptr_eq(
            &CommandTransactionCheckpoint::capture(&simulation.state).knowledge,
            knowledge
        ));
        assert!(std::sync::Arc::ptr_eq(
            &ScheduledBatchTransactionCheckpoint::capture(&simulation.state).knowledge,
            knowledge
        ));
    }
}
