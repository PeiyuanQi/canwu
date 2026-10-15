use super::{
    ArchivedEvidenceReceipt, ArchivedSegmentHeader, Army, ArmyId, BoundaryRecord, CanwuError,
    CommandAttemptRecord, CommandOutcome, CommandRecord, CommandRequestId, CommitmentRoots,
    DecisionRequestId, DecisionState, DomainRecordRef, EntityRef, ErrorCode, EvidenceCursor,
    EvidenceRef, Government, GovernmentId, IngressQueueKey, IngressReceipt, IngressRecord,
    KnowledgeSnapshot, PersistentDomainRecordStore, Person, PersonAvailability, PersonId,
    PluginComponentKey, PluginComponentRecord, RandomDrawRecord, RandomStreamKey,
    RandomStreamState, Route, RouteId, RunConfigurationSnapshot, RunManifest, Scenario,
    ScheduleKey, ScheduledAction, SimEvent, SimTime, Territory, TerritoryId,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

#[derive(Clone)]
pub(super) struct RuntimeEvidence {
    pub(super) archived: EvidenceCursor,
    pub(super) archived_boundary_head: Option<String>,
    pub(super) archived_legacy_commands: bool,
    pub(super) archived_tracked_attempts: bool,
    pub(super) archived_unqueued_command_history: bool,
    // The archived tables, manifest, and receipts grow with the run, so
    // they are persistent structures: a fork or a seal's rollback copy
    // shares them. Large values are shared too, which keeps each tree node
    // small. Iterate them forward only; `im` 15.1.0 ordered maps and sets
    // break on reverse traversal past about 2,100 entries.
    pub(super) archived_command_requests:
        im::OrdMap<CommandRequestId, Arc<ArchivedCommandRequestOutcome>>,
    pub(super) archived_ingress_requests: im::OrdMap<CommandRequestId, ArchivedIngressRequest>,
    pub(super) archived_decision_requests: im::OrdMap<DecisionRequestId, ArchivedIngressRequest>,
    pub(super) archived_decision_command_requests: im::OrdSet<CommandRequestId>,
    pub(super) events: Vec<SimEvent>,
    pub(super) commands: Vec<CommandRecord>,
    pub(super) command_attempts: Vec<CommandAttemptRecord>,
    pub(super) ingress: Vec<IngressRecord>,
    pub(super) boundaries: Vec<BoundaryRecord>,
    pub(super) random_draws: Vec<RandomDrawRecord>,
    pub(super) archived_segment_headers: im::Vector<ArchivedSegmentHeader>,
    pub(super) archived_evidence_receipts: im::OrdMap<EvidenceRef, Arc<ArchivedEvidenceReceipt>>,
    /// The keyed-draw ledger: every operation-keyed address the run has
    /// drawn, retained or sealed. Extended when a boundary commits its draws,
    /// restored with a failed boundary, untouched by a seal, and built once
    /// from the draw journal on restore. It is never part of a snapshot, a
    /// checkpoint, or a commitment. Only boundary systems make keyed draws,
    /// so no other transaction changes it.
    pub(super) keyed_draws: super::random::KeyedDrawLedger,
    /// When each sealed ordinal record became authoritative, so that
    /// `evidence_time` answers for sealed evidence as an unsealed run does.
    /// Extended by each seal; retained records answer from themselves.
    pub(super) sealed_times: EvidenceTimeIndex,
    /// The continuation closure below the archived cut.
    pub(super) pinned: super::continuation::PinnedEvidence,
    /// Where history begins for a run restored from a continuation
    /// checkpoint; `None` for a run that holds its history.
    pub(super) continuation_base: Option<super::ContinuationBase>,
}

/// The times of the sealed part of each ordinal journal, run-length encoded:
/// one entry `(first ID, time)` starts whenever the time changes along the
/// journal, and an ID's time is the entry with the largest first ID at or
/// below it. This is exact for any order of times; journals that append in
/// time order keep about one entry per distinct instant.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct EvidenceTimeIndex {
    events: im::Vector<(u64, SimTime)>,
    commands: im::Vector<(u64, SimTime)>,
    command_attempts: im::Vector<(u64, SimTime)>,
    ingress: im::Vector<(u64, SimTime)>,
    boundaries: im::Vector<(u64, SimTime)>,
    random_draws: im::Vector<(u64, SimTime)>,
}

impl EvidenceTimeIndex {
    fn note(runs: &mut im::Vector<(u64, SimTime)>, id: u64, at: SimTime) {
        if runs.back().is_none_or(|(_, last)| *last != at) {
            runs.push_back((id, at));
        }
    }

    fn time_of(runs: &im::Vector<(u64, SimTime)>, id: u64) -> Option<SimTime> {
        match runs.binary_search_by(|(first, _)| first.cmp(&id)) {
            Ok(index) => runs.get(index).map(|(_, at)| *at),
            Err(0) => None,
            Err(index) => runs.get(index - 1).map(|(_, at)| *at),
        }
    }

    /// The run-length lists in the order events, commands, attempts,
    /// ingress, boundaries, draws.
    pub(super) const fn runs(&self) -> [&im::Vector<(u64, SimTime)>; 6] {
        [
            &self.events,
            &self.commands,
            &self.command_attempts,
            &self.ingress,
            &self.boundaries,
            &self.random_draws,
        ]
    }

    /// An index from run-length lists in the order of [`Self::runs`].
    pub(super) fn from_runs(runs: [Vec<(u64, SimTime)>; 6]) -> Self {
        let [
            events,
            commands,
            command_attempts,
            ingress,
            boundaries,
            random_draws,
        ] = runs.map(im::Vector::from);
        Self {
            events,
            commands,
            command_attempts,
            ingress,
            boundaries,
            random_draws,
        }
    }

    /// Records the times of the records a seal moves below the archived cut.
    pub(super) fn extend_from_segment(&mut self, segment: &super::EvidenceJournalSegment) {
        self.extend_from_slices(
            &segment.events,
            &segment.commands,
            &segment.command_attempts,
            &segment.ingress,
            &segment.boundaries,
            &segment.random_draws,
        );
    }

    /// Records the times of journal slices that continue the indexed prefix.
    pub(super) fn extend_from_slices(
        &mut self,
        events: &[SimEvent],
        commands: &[CommandRecord],
        attempts: &[CommandAttemptRecord],
        ingress: &[IngressRecord],
        boundaries: &[BoundaryRecord],
        draws: &[RandomDrawRecord],
    ) {
        for event in events {
            Self::note(&mut self.events, event.id.get(), event.timestamp);
        }
        for command in commands {
            Self::note(&mut self.commands, command.id.get(), command.accepted_at);
        }
        for attempt in attempts {
            Self::note(&mut self.command_attempts, attempt.id.get(), attempt.at);
        }
        for record in ingress {
            Self::note(&mut self.ingress, record.id.get(), record.issued_at);
        }
        for boundary in boundaries {
            Self::note(&mut self.boundaries, boundary.id.get(), boundary.at);
        }
        for draw in draws {
            Self::note(&mut self.random_draws, draw.id.get(), draw.at);
        }
    }

    /// The time of an ordinal reference at or below `archived`; `None` for a
    /// domain-record version or an ID the seals have not reached.
    pub(super) fn sealed_time(
        &self,
        reference: &EvidenceRef,
        archived: &EvidenceCursor,
    ) -> Option<SimTime> {
        let (runs, id, count) = match reference {
            EvidenceRef::Event(id) => (&self.events, id.get(), archived.event_count),
            EvidenceRef::Command(id) => (&self.commands, id.get(), archived.command_count),
            EvidenceRef::CommandAttempt(id) => (
                &self.command_attempts,
                id.get(),
                archived.command_attempt_count,
            ),
            EvidenceRef::Ingress(id) => (&self.ingress, id.get(), archived.ingress_count),
            EvidenceRef::Boundary(id) => (&self.boundaries, id.get(), archived.boundary_count),
            EvidenceRef::RandomDraw(id) => {
                (&self.random_draws, id.get(), archived.random_draw_count)
            }
            EvidenceRef::DomainRecordVersion(_) => return None,
        };
        if id == 0 || id > count {
            return None;
        }
        Self::time_of(runs, id)
    }
}

#[derive(Clone)]
pub(super) struct ArchivedCommandRequestOutcome {
    pub(super) input_hash: String,
    pub(super) outcome: CommandOutcome,
}

#[derive(Clone)]
pub(super) struct ArchivedIngressRequest {
    pub(super) input_hash: String,
    pub(super) receipt: IngressReceipt,
}

impl RuntimeEvidence {
    fn retained_index(id: u64, archived_count: u64) -> Option<usize> {
        id.checked_sub(archived_count)?
            .checked_sub(1)
            .and_then(|value| usize::try_from(value).ok())
    }

    pub(super) fn retained_event(&self, id: super::EventId) -> Option<&SimEvent> {
        let index = Self::retained_index(id.get(), self.archived.event_count)?;
        self.events.get(index).filter(|event| event.id == id)
    }

    pub(super) fn retained_command(&self, id: super::CommandId) -> Option<&CommandRecord> {
        let index = Self::retained_index(id.get(), self.archived.command_count)?;
        self.commands.get(index).filter(|record| record.id == id)
    }

    pub(super) fn retained_command_attempt(
        &self,
        id: super::CommandAttemptId,
    ) -> Option<&CommandAttemptRecord> {
        let index = Self::retained_index(id.get(), self.archived.command_attempt_count)?;
        self.command_attempts
            .get(index)
            .filter(|record| record.id == id)
    }

    pub(super) fn retained_boundary(&self, id: super::BoundaryId) -> Option<&BoundaryRecord> {
        let index = Self::retained_index(id.get(), self.archived.boundary_count)?;
        self.boundaries.get(index).filter(|record| record.id == id)
    }

    pub(super) fn retained_ingress(&self, id: super::IngressId) -> Option<&IngressRecord> {
        let index = Self::retained_index(id.get(), self.archived.ingress_count)?;
        self.ingress.get(index).filter(|record| record.id == id)
    }

    pub(super) fn retained_random_draw(
        &self,
        id: super::RandomDrawId,
    ) -> Option<&RandomDrawRecord> {
        let index = Self::retained_index(id.get(), self.archived.random_draw_count)?;
        self.random_draws
            .get(index)
            .filter(|record| record.id == id)
    }

    /// A retained or pinned event.
    pub(super) fn event_record(&self, id: super::EventId) -> Option<&SimEvent> {
        self.retained_event(id)
            .or_else(|| self.pinned.events.get(&id).map(Arc::as_ref))
    }

    /// A retained or pinned command.
    pub(super) fn command_record(&self, id: super::CommandId) -> Option<&CommandRecord> {
        self.retained_command(id)
            .or_else(|| self.pinned.commands.get(&id).map(Arc::as_ref))
    }

    /// A retained or pinned command attempt.
    pub(super) fn command_attempt_record(
        &self,
        id: super::CommandAttemptId,
    ) -> Option<&CommandAttemptRecord> {
        self.retained_command_attempt(id)
            .or_else(|| self.pinned.command_attempts.get(&id).map(Arc::as_ref))
    }

    /// A retained or pinned ingress record.
    pub(super) fn ingress_record(&self, id: super::IngressId) -> Option<&IngressRecord> {
        self.retained_ingress(id)
            .or_else(|| self.pinned.ingress.get(&id).map(Arc::as_ref))
    }

    /// A retained or pinned boundary.
    pub(super) fn boundary_record(&self, id: super::BoundaryId) -> Option<&BoundaryRecord> {
        self.retained_boundary(id)
            .or_else(|| self.pinned.boundaries.get(&id).map(Arc::as_ref))
    }

    /// A retained or pinned random draw.
    pub(super) fn random_draw_record(&self, id: super::RandomDrawId) -> Option<&RandomDrawRecord> {
        self.retained_random_draw(id)
            .or_else(|| self.pinned.random_draws.get(&id).map(Arc::as_ref))
    }

    /// Retained ingress followed by pinned ingress below the archived cut.
    pub(super) fn ingress_records(&self) -> impl Iterator<Item = &IngressRecord> {
        self.pinned
            .ingress
            .values()
            .map(Arc::as_ref)
            .chain(self.ingress.iter())
    }

    pub(super) fn boundary_head_hash(&self) -> Option<&str> {
        self.boundaries
            .last()
            .map(|record| record.hash.as_str())
            .or(self.archived_boundary_head.as_deref())
    }
}

/// The canonical hash of one append-only journal, `canonical_hash(domain,
/// &items)`, extended one item at a time. Its stream is resumable, so the
/// state can be exported and continued without the items already fed.
#[derive(Clone)]
pub(super) struct OrderedCommitmentAccumulator {
    stream: super::stream_hash::ResumableStreamHash,
    /// The root of the items fed so far, kept until the next append. Every
    /// committed transaction recomputes all journal roots, and most
    /// transactions leave most journals unchanged.
    root: Option<blake3::Hash>,
    pub(super) len: usize,
}

impl OrderedCommitmentAccumulator {
    pub(super) fn new(domain: &str) -> Result<Self, CanwuError> {
        let mut stream = super::stream_hash::ResumableStreamHash::new();
        stream.update(domain.as_bytes())?;
        stream.update(&[0])?;
        stream.update(b"[")?;
        Ok(Self {
            stream,
            root: None,
            len: 0,
        })
    }

    fn from_sorted_by<T, K, F>(domain: &str, values: &[T], mut key: F) -> Result<Self, CanwuError>
    where
        T: Serialize,
        K: Ord,
        F: FnMut(&T) -> K,
    {
        let mut ordered: Vec<_> = values.iter().collect();
        ordered.sort_by_key(|value| key(value));
        let mut accumulator = Self::new(domain)?;
        for value in ordered {
            accumulator.append(value)?;
        }
        Ok(accumulator)
    }

    pub(super) fn append<T: Serialize>(&mut self, value: &T) -> Result<(), CanwuError> {
        let encoded = serde_json::to_vec(value).map_err(|error| {
            CanwuError::new(
                ErrorCode::InvalidSnapshot,
                format!("could not encode incremental commitment item: {error}"),
            )
        })?;
        self.root = None;
        if self.len != 0 {
            self.stream.update(b",")?;
        }
        self.stream.update(&encoded)?;
        self.len = self.len.checked_add(1).ok_or_else(|| {
            CanwuError::new(
                ErrorCode::IdentifierExhausted,
                "incremental commitment item count is exhausted",
            )
        })?;
        Ok(())
    }

    fn sync_tail<T: Serialize>(&mut self, archived: u64, values: &[T]) -> Result<(), CanwuError> {
        let archived = usize::try_from(archived).map_err(|_| {
            CanwuError::new(
                ErrorCode::InvalidSnapshot,
                "archived evidence cursor is not representable on this platform",
            )
        })?;
        if self.len < archived {
            return Err(CanwuError::new(
                ErrorCode::InvalidSnapshot,
                "append-only commitment cache precedes its archived evidence cursor",
            ));
        }
        let retained = self.len - archived;
        if retained > values.len() {
            return Err(CanwuError::new(
                ErrorCode::InvalidSnapshot,
                "append-only commitment cache exceeds its retained evidence journal",
            ));
        }
        for value in &values[retained..] {
            self.append(value)?;
        }
        Ok(())
    }

    fn root_hash(&self) -> Result<blake3::Hash, CanwuError> {
        if let Some(root) = self.root {
            return Ok(root);
        }
        let mut stream = self.stream.clone();
        stream.update(b"]")?;
        Ok(stream.finalize())
    }

    #[cfg(test)]
    pub(super) fn root(&self) -> Result<String, CanwuError> {
        Ok(self.root_hash()?.to_hex().to_string())
    }

    /// The item count and resumable stream state.
    pub(super) fn frontier(&self) -> (usize, super::stream_hash::StreamFrontier) {
        (self.len, self.stream.frontier())
    }

    /// Resumes an exported stream state after checking its shape.
    pub(super) fn from_frontier(
        len: usize,
        frontier: &super::stream_hash::StreamFrontier,
    ) -> Result<Self, CanwuError> {
        Ok(Self {
            stream: super::stream_hash::ResumableStreamHash::from_frontier(frontier)?,
            root: None,
            len,
        })
    }

    /// Returns the root and keeps it until the next append.
    fn refresh_root(&mut self) -> Result<String, CanwuError> {
        let root = self.root_hash()?;
        self.root = Some(root);
        Ok(root.to_hex().to_string())
    }
}

/// Running commitment over every generic knowledge record in global ID
/// order: the records sub-root of the knowledge root.
///
/// The runtime gives a published record an ID above every ID it holds and
/// never changes or removes a committed record, so the ledger only grows at
/// the end of this order, and a boundary feeds only the records it published.
/// It lives in the commitment cache: a failed boundary restores it with the
/// cache, and a restore, which builds a new cache, rebuilds it from the
/// ledger. Load validation never reads it; it recomputes the root from the
/// snapshot.
///
/// The accumulator trusts that invariant. It notices a ledger that lost
/// records or gained records below the last one fed, because the counts no
/// longer add up, but not a committed record changed in place: every path
/// that replaces the knowledge state must restore or drop the cache with it.
#[derive(Clone)]
pub(super) struct KnowledgeRecordAccumulator {
    records: OrderedCommitmentAccumulator,
    last: Option<super::KnowledgeRecordId>,
}

impl KnowledgeRecordAccumulator {
    pub(super) fn new() -> Result<Self, CanwuError> {
        Ok(Self {
            records: OrderedCommitmentAccumulator::new(
                super::hashing::KNOWLEDGE_RECORD_COMMITMENT_DOMAIN,
            )?,
            last: None,
        })
    }

    /// Feeds the records the ledgers gained after the last record fed.
    /// Returns `false`, having fed nothing, when the record counts show that
    /// the ledgers do not extend what was fed (they hold fewer records, or
    /// records below the last one fed that it has not seen); the caller then
    /// starts again from empty.
    pub(super) fn sync(&mut self, ledgers: &super::HolderLedgers) -> Result<bool, CanwuError> {
        let tail: Vec<_> = ledgers.records_after(self.last).collect();
        if self.records.len.checked_add(tail.len()) != Some(ledgers.record_count()) {
            return Ok(false);
        }
        for record in tail {
            self.records.append(record)?;
            self.last = Some(record.id);
        }
        Ok(true)
    }

    /// Builds the accumulator of a whole ledger.
    pub(super) fn of(ledgers: &super::HolderLedgers) -> Result<Self, CanwuError> {
        let mut accumulator = Self::new()?;
        accumulator.sync(ledgers)?;
        Ok(accumulator)
    }

    /// Returns the records sub-root and keeps it until the next record.
    pub(super) fn root(&mut self) -> Result<String, CanwuError> {
        self.records.refresh_root()
    }

    /// The ID of the last record fed.
    #[cfg(test)]
    pub(super) const fn last(&self) -> Option<super::KnowledgeRecordId> {
        self.last
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
    /// Sub-root of the knowledge root over the compatibility actor
    /// projection; cleared with [`CommitmentDomains::KNOWLEDGE`] and kept by
    /// [`CommitmentDomains::KNOWLEDGE_RECORDS`].
    pub(super) knowledge_actors: Option<String>,
    /// Records sub-root of the knowledge root, extended rather than cleared.
    pub(super) knowledge_records: Option<KnowledgeRecordAccumulator>,
    pub(super) plugin_components: Option<String>,
    pub(super) domain_records: Option<String>,
    pub(super) decisions: Option<String>,
    pub(super) scheduler: Option<String>,
    pub(super) random_streams: Option<String>,
    pub(super) identity: Option<String>,
}

impl RuntimeCommitmentCache {
    pub(super) fn from_evidence(evidence: &RuntimeEvidence) -> Result<Self, CanwuError> {
        // Built from the retained journals alone, the cache is exact only
        // while nothing has been sealed.
        if evidence.archived != EvidenceCursor::default() {
            return Err(CanwuError::new(
                ErrorCode::InvalidSnapshot,
                "a sealed runtime cannot rebuild its journal commitments from retained evidence",
            ));
        }
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
            knowledge_actors: None,
            knowledge_records: None,
            plugin_components: None,
            domain_records: None,
            decisions: None,
            scheduler: None,
            random_streams: None,
            identity: None,
        })
    }

    /// The journal accumulators in the order commands, attempts, events,
    /// ingress, draws.
    pub(super) const fn journals(&self) -> [&OrderedCommitmentAccumulator; 5] {
        [
            &self.commands,
            &self.attempts,
            &self.events,
            &self.ingress,
            &self.random_draws,
        ]
    }

    /// A cache resumed from journal accumulators, in the order of
    /// [`Self::journals`], with every state-domain root left to recompute.
    pub(super) fn from_journals(journals: [OrderedCommitmentAccumulator; 5]) -> Self {
        let [commands, attempts, events, ingress, random_draws] = journals;
        Self {
            commands,
            attempts,
            events,
            ingress,
            random_draws,
            world: None,
            knowledge: None,
            knowledge_actors: None,
            knowledge_records: None,
            plugin_components: None,
            domain_records: None,
            decisions: None,
            scheduler: None,
            random_streams: None,
            identity: None,
        }
    }

    pub(super) fn sync(&mut self, evidence: &RuntimeEvidence) -> Result<(), CanwuError> {
        self.commands
            .sync_tail(evidence.archived.command_count, &evidence.commands)?;
        self.attempts.sync_tail(
            evidence.archived.command_attempt_count,
            &evidence.command_attempts,
        )?;
        self.events
            .sync_tail(evidence.archived.event_count, &evidence.events)?;
        self.ingress
            .sync_tail(evidence.archived.ingress_count, &evidence.ingress)?;
        self.random_draws
            .sync_tail(evidence.archived.random_draw_count, &evidence.random_draws)?;
        Ok(())
    }

    pub(super) fn roots(&mut self) -> Result<JournalCommitmentRoots, CanwuError> {
        Ok(JournalCommitmentRoots {
            commands: self.commands.refresh_root()?,
            attempts: self.attempts.refresh_root()?,
            events: self.events.refresh_root()?,
            ingress: self.ingress.refresh_root()?,
            random_draws: self.random_draws.refresh_root()?,
        })
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
            (self.decisions.is_none(), CommitmentDomains::DECISIONS),
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
            self.knowledge_actors = None;
        }
        if domains.contains(CommitmentDomains::KNOWLEDGE_RECORDS) {
            self.knowledge = None;
        }
        if domains.contains(CommitmentDomains::PLUGIN_COMPONENTS) {
            self.plugin_components = None;
        }
        if domains.contains(CommitmentDomains::DOMAIN_RECORDS) {
            self.domain_records = None;
        }
        if domains.contains(CommitmentDomains::DECISIONS) {
            self.decisions = None;
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
        if let Some(root) = updates.knowledge_actors {
            self.knowledge_actors = Some(root);
        }
        if let Some(records) = updates.knowledge_records {
            self.knowledge_records = Some(records);
        }
        if let Some(root) = updates.plugin_components {
            self.plugin_components = Some(root);
        }
        if let Some(root) = updates.domain_records {
            self.domain_records = Some(root);
        }
        if let Some(root) = updates.decisions {
            self.decisions = Some(root);
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
            decisions: required(&self.decisions, "decision")?,
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
pub(super) struct CommitmentDomains(u16);

impl CommitmentDomains {
    pub(super) const WORLD: Self = Self(1 << 0);
    pub(super) const KNOWLEDGE: Self = Self(1 << 1);
    pub(super) const PLUGIN_COMPONENTS: Self = Self(1 << 2);
    pub(super) const DOMAIN_RECORDS: Self = Self(1 << 3);
    pub(super) const SCHEDULER: Self = Self(1 << 4);
    pub(super) const RANDOM_STREAMS: Self = Self(1 << 5);
    pub(super) const IDENTITY: Self = Self(1 << 6);
    pub(super) const DECISIONS: Self = Self(1 << 7);
    /// Generic knowledge records were appended. The knowledge root is
    /// recombined from the cached actor sub-root and the extended records
    /// accumulator, so the ledger is not hashed again.
    pub(super) const KNOWLEDGE_RECORDS: Self = Self(1 << 8);

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
    pub(super) knowledge_actors: Option<String>,
    pub(super) knowledge_records: Option<KnowledgeRecordAccumulator>,
    pub(super) plugin_components: Option<String>,
    pub(super) domain_records: Option<String>,
    pub(super) decisions: Option<String>,
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
    pub(super) decisions: String,
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
    /// Derived index of retained plugin ingress withdrawn by a terminal
    /// cancellation record. Rebuilt from the ingress journal on restore and
    /// cleared when the retained journal is sealed.
    pub(super) cancelled_ingress: BTreeSet<super::IngressId>,
    /// Registered transition manifests awaiting their ready boundary,
    /// committed under the scheduler root.
    pub(super) transition_manifests:
        BTreeMap<super::TransitionManifestId, super::PendingTransitionManifest>,
    /// Derived ledger of the boundaries settled within the run's boundary
    /// schedule horizon of the latest boundary, in settlement order.
    /// Maintained at boundary commit, rebuilt from the full journal on
    /// restore, and kept across a seal.
    pub(super) settled_boundaries: im::Vector<std::sync::Arc<super::BoundarySummary>>,
}

#[derive(Clone)]
pub(super) struct RuntimeCounters {
    pub(super) next_event_id: u64,
    pub(super) next_command_id: u64,
    pub(super) next_command_attempt_id: u64,
    pub(super) next_ingress_id: u64,
    pub(super) next_boundary_id: u64,
    pub(super) next_random_draw_id: u64,
    pub(super) next_knowledge_record_id: u64,
    pub(super) next_schedule_sequence: u64,
    pub(super) next_correlation_id: u64,
    pub(super) next_decision_trace_id: u64,
    /// Next runtime-created person ID; zero until the first creation, whose
    /// ID follows every initial-scenario person identity.
    pub(super) next_person_id: u64,
    pub(super) state_revision: u64,
    pub(super) admitted_attempt_count: u64,
    pub(super) admitted_command_count: u64,
    pub(super) admitted_event_count: u64,
}

#[derive(Clone)]
pub(super) struct RuntimeMetadata {
    pub(super) initial_scenario: Option<Scenario>,
    pub(super) initial_domain_record_indexes: BTreeMap<DomainRecordRef, usize>,
    /// Exact provenance and establishment time for each current domain-record
    /// version. This is maintained incrementally at boundary commit time and
    /// rebuilt from the retained journal during restore, keeping hot-path
    /// lookups O(log n). It is a persistent map, so the boundary transaction
    /// checkpoint and a fork share it instead of copying one entry per record.
    pub(super) current_domain_record_versions:
        im::OrdMap<Arc<DomainRecordRef>, Arc<CurrentDomainRecordVersion>>,
    pub(super) run_manifest: RunManifest,
    pub(super) run_manifest_hash: String,
    pub(super) run_configuration: RunConfigurationSnapshot,
    pub(super) checkpoint_hash: String,
    pub(super) commitment_format_version: u32,
    pub(super) commitment_roots: Option<CommitmentRoots>,
    /// Boxed: the five resumable journal streams are about 14 KB, and the
    /// runtime and every transaction checkpoint hold the cache, so keeping
    /// it inline would put that much on the stack of every frame that moves
    /// one.
    pub(super) commitment_cache: Option<Box<RuntimeCommitmentCache>>,
    pub(super) plugin_registration_closed: bool,
    pub(super) replay_revision_format_version: u32,
}

/// A live record's current version with the time it became authoritative.
///
/// Keeping the time in this index lets the current version resolve its
/// establishment time after its establishing boundary is sealed.
#[derive(Clone)]
pub(super) struct CurrentDomainRecordVersion {
    pub(super) version: super::DomainRecordVersionRef,
    pub(super) established_at: SimTime,
}

#[derive(Clone)]
pub(super) struct RuntimeCurrentState {
    pub(super) entities: BTreeSet<EntityRef>,
    pub(super) people: BTreeMap<PersonId, Person>,
    /// Core life and custody state; absent entries are alive and free.
    pub(super) person_availability: BTreeMap<PersonId, PersonAvailability>,
    /// Runtime-created persons in allocation order, keyed for next-boundary
    /// correlation lookup independently of retained evidence.
    pub(super) created_persons: Vec<super::CreatedPerson>,
    pub(super) letters: BTreeMap<canwu_core::LetterId, super::LetterCargo>,
    pub(super) governments: BTreeMap<GovernmentId, Government>,
    pub(super) territories: BTreeMap<TerritoryId, Territory>,
    pub(super) routes: BTreeMap<RouteId, Route>,
    pub(super) armies: BTreeMap<ArmyId, Army>,
    /// Shared, so boundary and transaction captures, early-phase views, and
    /// forks copy nothing. A change goes through `Arc::make_mut`: it copies
    /// the compatibility actor projection and shares the persistent holder
    /// ledgers, of which it copies only the paths it changes.
    pub(super) knowledge: std::sync::Arc<KnowledgeSnapshot>,
    pub(super) plugin_components: BTreeMap<PluginComponentKey, PluginComponentRecord>,
    /// Persistent current-state root. Boundary checkpoints and forks clone the
    /// handle in O(1), while writes copy only paths in the persistent map.
    pub(super) domain_records: PersistentDomainRecordStore,
    pub(super) decisions: DecisionState,
    pub(super) root_seed: u64,
    /// Persisted authority credential root. It is intentionally separate from
    /// the deterministic simulation RNG seed and is never used for draws.
    pub(super) authority_root_seed: u64,
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
