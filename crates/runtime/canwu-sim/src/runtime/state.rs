use super::{
    ArchivedEvidenceReceipt, ArchivedSegmentHeader, Army, ArmyId, BoundaryRecord, CanwuError,
    CommandAttemptRecord, CommandOutcome, CommandRecord, CommandRequestId, CommitmentRoots,
    DecisionRequestId, DecisionState, DomainRecord, DomainRecordRef, EntityRef, ErrorCode,
    EvidenceCursor, EvidenceRef, Government, GovernmentId, IngressPayload, IngressQueueKey,
    IngressReceipt, IngressRecord, KnowledgeSnapshot, PersistentDomainRecordStore, Person,
    PersonAvailability, PersonId, PluginComponentKey, PluginComponentRecord, RandomDrawRecord,
    RandomStreamKey, RandomStreamState, RetiredRequestRange, Route, RouteId,
    RunConfigurationSnapshot, RunManifest, Scenario, ScheduleKey, ScheduledAction, SimEvent,
    SimTime, Territory, TerritoryId,
};
use serde::Serialize;
use std::collections::BTreeMap;
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
    /// Retired command requests whose attempts lie at or below the archived
    /// cut, moved here by a seal or carried by a continuation checkpoint. A
    /// retired request's outcome and queued receipt leave the archived tables
    /// above; its ID stays used.
    pub(super) retired_command_requests: RetiredRequestIds,
    pub(super) events: SharedJournal<SimEvent>,
    pub(super) commands: RetainedJournal<CommandRecord>,
    pub(super) command_attempts: RetainedJournal<CommandAttemptRecord>,
    pub(super) ingress: RetainedJournal<IngressRecord>,
    /// The derived index of the three journals above; see
    /// [`CommandHistoryIndex`].
    pub(super) command_index: CommandHistoryIndex,
    pub(super) boundaries: SharedJournal<BoundaryRecord>,
    pub(super) random_draws: SharedJournal<RandomDrawRecord>,
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

/// When a command outcome settled: its attempt's time, which an accepted
/// outcome records as `accepted_at` and a rejection as `rejected_at`.
pub(super) const fn command_outcome_time(outcome: &CommandOutcome) -> SimTime {
    match outcome {
        CommandOutcome::Accepted { receipt } => receipt.accepted_at,
        CommandOutcome::Rejected { rejection } => rejection.rejected_at,
    }
}

/// The IDs of retired command requests, as sorted, disjoint, non-adjacent
/// inclusive ranges. It changes only at a seal and a restore, which replace
/// it whole, so a fork shares it; it is read by binary search.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct RetiredRequestIds(Arc<Vec<RetiredRequestRange>>);

impl RetiredRequestIds {
    /// Ranges a continuation checkpoint carries, checked canonical first.
    pub(super) fn from_ranges(ranges: Vec<RetiredRequestRange>) -> Self {
        Self(Arc::new(ranges))
    }

    pub(super) fn ranges(&self) -> &[RetiredRequestRange] {
        &self.0
    }

    pub(super) fn contains(&self, request: CommandRequestId) -> bool {
        ranges_contain(&self.0, request)
    }

    /// These ranges with `requests` added, in canonical form.
    pub(super) fn with(&self, requests: impl IntoIterator<Item = CommandRequestId>) -> Self {
        let mut added: Vec<CommandRequestId> = requests.into_iter().collect();
        if added.is_empty() {
            return self.clone();
        }
        added.sort_unstable();
        added.dedup();
        let mut merged: Vec<RetiredRequestRange> = Vec::with_capacity(self.0.len() + 1);
        let mut push = |first: CommandRequestId, last: CommandRequestId| {
            if let Some(previous) = merged.last_mut()
                && previous.last.get().saturating_add(1) >= first.get()
            {
                previous.last = previous.last.max(last);
                return;
            }
            merged.push(RetiredRequestRange { first, last });
        };
        let mut ranges = self.0.iter().peekable();
        let mut ids = added.into_iter().peekable();
        loop {
            match (ranges.peek(), ids.peek()) {
                (Some(range), Some(id)) if range.first <= *id => {
                    push(range.first, range.last);
                    ranges.next();
                }
                (_, Some(id)) => {
                    push(*id, *id);
                    ids.next();
                }
                (Some(range), None) => {
                    push(range.first, range.last);
                    ranges.next();
                }
                (None, None) => break,
            }
        }
        Self(Arc::new(merged))
    }
}

/// Whether sorted, disjoint ranges hold `request`.
pub(super) fn ranges_contain(ranges: &[RetiredRequestRange], request: CommandRequestId) -> bool {
    let index = ranges.partition_point(|range| range.last < request);
    ranges
        .get(index)
        .is_some_and(|range| range.first <= request)
}

/// Whether ranges are in the canonical form [`RetiredRequestIds::with`]
/// writes: each `first <= last`, increasing, and never adjacent. Zero is a
/// valid member: admission records the rejected attempt of a request whose
/// ID is zero, and it retires like any other.
pub(super) fn retired_ranges_are_canonical(ranges: &[RetiredRequestRange]) -> bool {
    ranges.iter().all(|range| range.first <= range.last)
        && ranges
            .windows(2)
            .all(|pair| pair[0].last.get().saturating_add(1) < pair[1].first.get())
}

#[derive(Clone)]
pub(super) struct ArchivedIngressRequest {
    pub(super) input_hash: String,
    pub(super) receipt: IngressReceipt,
}

/// A retained evidence journal that a run shares with its forks and
/// rollback copies until one of them changes it.
///
/// A clone shares the records, so `fork` and a transaction's copy cost
/// nothing in proportion to the journal. The first change to a shared
/// journal copies what the change keeps: an append copies the journal
/// (what a fork used to copy), a truncation copies the kept prefix, and a
/// drain from the front copies the kept suffix. A journal no copy shares
/// changes in place. Reads see one contiguous slice.
pub(super) struct SharedJournal<T>(Arc<Vec<T>>);

impl<T> Clone for SharedJournal<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T> Default for SharedJournal<T> {
    fn default() -> Self {
        Self(Arc::new(Vec::new()))
    }
}

impl<T> From<Vec<T>> for SharedJournal<T> {
    fn from(records: Vec<T>) -> Self {
        Self(Arc::new(records))
    }
}

impl<T> std::ops::Deref for SharedJournal<T> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        &self.0
    }
}

impl<'a, T> IntoIterator for &'a SharedJournal<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl<T: Clone> SharedJournal<T> {
    /// Appends a record, copying the journal first if a copy shares it.
    pub(super) fn push(&mut self, record: T) {
        Arc::make_mut(&mut self.0).push(record);
    }

    /// Keeps the first `len` records; a length at or past the end keeps the
    /// journal as it is.
    pub(super) fn truncate(&mut self, len: usize) {
        if len >= self.0.len() {
            return;
        }
        match Arc::get_mut(&mut self.0) {
            Some(records) => records.truncate(len),
            None => self.0 = Arc::new(self.0[..len].to_vec()),
        }
    }

    /// Removes the records before `range.end`, as `Vec::drain(..end)` does,
    /// copying only the kept suffix if a copy shares the journal.
    pub(super) fn drain(&mut self, range: std::ops::RangeTo<usize>) {
        if range.end == 0 {
            return;
        }
        match Arc::get_mut(&mut self.0) {
            Some(records) => {
                records.drain(range);
            }
            None => self.0 = Arc::new(self.0[range.end..].to_vec()),
        }
    }

    /// The last record, for an in-place update; copies the journal first if
    /// a copy shares it.
    pub(super) fn last_mut(&mut self) -> Option<&mut T> {
        Arc::make_mut(&mut self.0).last_mut()
    }

    /// Removes and returns the last record.
    #[cfg(test)]
    pub(super) fn pop(&mut self) -> Option<T> {
        Arc::make_mut(&mut self.0).pop()
    }

    /// Removes every record.
    pub(super) fn clear(&mut self) {
        match Arc::get_mut(&mut self.0) {
            Some(records) => records.clear(),
            None => self.0 = Arc::default(),
        }
    }

    /// Whether `other` shares this journal's records.
    #[cfg(test)]
    pub(super) fn shares_records_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// One retained journal of the command history: the accepted commands, the
/// command attempts, or the ingress. Any module reads it as a slice; only the
/// command-history methods of [`RuntimeEvidence`] change it, so the derived
/// [`CommandHistoryIndex`] cannot drift from the records.
#[derive(Clone)]
pub(super) struct RetainedJournal<T>(SharedJournal<T>);

impl<T> Default for RetainedJournal<T> {
    fn default() -> Self {
        Self(SharedJournal::default())
    }
}

impl<T> std::ops::Deref for RetainedJournal<T> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        &self.0
    }
}

#[cfg(test)]
impl<T: Clone> RetainedJournal<T> {
    /// Whether `other` shares this journal's records.
    pub(super) fn shares_records_with(&self, other: &Self) -> bool {
        self.0.shares_records_with(&other.0)
    }
}

impl<'a, T> IntoIterator for &'a RetainedJournal<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

/// What admission asks of the retained command history, kept so that no
/// question rescans it. It is derived from the three retained journals and
/// the archived request tables: never part of a snapshot, a checkpoint, a
/// hash, or a commitment. The push and truncate methods keep it in step, a
/// seal and every restore rebuild it, and a failed transaction restores it
/// with the journals it truncates.
#[derive(Clone, Default)]
pub(super) struct CommandHistoryIndex {
    /// Retained command and decision ingress per command request.
    /// Persistent, as the journals are shared, so a fork copies neither.
    queued_requests: im::OrdMap<CommandRequestId, usize>,
    /// The position of the first retained attempt of each command request.
    attempt_by_request: im::OrdMap<CommandRequestId, usize>,
    /// The position of the first retained legacy-direct command.
    first_direct_command: Option<usize>,
    /// The position of the first retained attempt whose request was not
    /// queued, neither as retained ingress nor as archived ingress.
    first_unqueued_attempt: Option<usize>,
}

/// The command request a command or decision ingress record carries.
pub(super) fn ingress_command_request(record: &IngressRecord) -> Option<CommandRequestId> {
    match &record.payload {
        IngressPayload::Command { request } => Some(request.request_id),
        IngressPayload::Decision { request } => {
            request.command.as_ref().map(|command| command.request_id)
        }
        IngressPayload::Plugin { .. }
        | IngressPayload::Calendar { .. }
        | IngressPayload::Maintenance { .. }
        | IngressPayload::PluginCancellation { .. } => None,
    }
}

impl RuntimeEvidence {
    /// Whether a tracked command request is retired: its attempt, sealed or
    /// retained, settled before `kept_from`, the earliest time whose outcomes
    /// the run's request outcome retention still keeps. A seal only moves
    /// retired outcomes into [`Self::retired_command_requests`]; the answer
    /// does not depend on when the run sealed.
    pub(super) fn command_request_retired(
        &self,
        request: CommandRequestId,
        kept_from: SimTime,
    ) -> bool {
        self.retired_command_requests.contains(request)
            || self
                .archived_command_requests
                .get(&request)
                .is_some_and(|archived| command_outcome_time(&archived.outcome) < kept_from)
            || self
                .retained_attempt_for_request(request)
                .is_some_and(|attempt| attempt.at < kept_from)
    }

    /// Moves every archived outcome that settled before `kept_from` out of
    /// the archived tables, with its request's queued receipt, and its ID
    /// into the retired ranges. Archived outcomes are all of sealed attempts,
    /// so a receipt leaves only with its own request's sealed outcome and no
    /// retained attempt loses the receipt that marks it queued.
    pub(super) fn retire_command_requests(&mut self, kept_from: SimTime) {
        let retired: Vec<CommandRequestId> = self
            .archived_command_requests
            .iter()
            .filter(|(_, archived)| command_outcome_time(&archived.outcome) < kept_from)
            .map(|(request, _)| *request)
            .collect();
        if retired.is_empty() {
            return;
        }
        let is_retired = |request: &CommandRequestId| retired.binary_search(request).is_ok();
        // Rebuilt rather than removed from: a removal from the persistent
        // map holds several of its nodes on the stack at once in a debug
        // build, which overflows a 1 MiB thread.
        self.archived_command_requests = self
            .archived_command_requests
            .iter()
            .filter(|(request, _)| !is_retired(request))
            .map(|(request, archived)| (*request, Arc::clone(archived)))
            .collect();
        self.archived_ingress_requests = self
            .archived_ingress_requests
            .iter()
            .filter(|(request, _)| !is_retired(request))
            .map(|(request, archived)| (*request, archived.clone()))
            .collect();
        self.retired_command_requests = self.retired_command_requests.with(retired);
    }

    /// Whether a seal archived a command request as queued command or
    /// decision ingress.
    pub(super) fn archived_request_was_queued(&self, request: CommandRequestId) -> bool {
        self.archived_ingress_requests.contains_key(&request)
            || self.archived_decision_command_requests.contains(&request)
    }

    fn attempt_is_unqueued(&self, attempt: &CommandAttemptRecord) -> bool {
        attempt.request_id.is_none_or(|request| {
            !self.command_index.queued_requests.contains_key(&request)
                && !self.archived_request_was_queued(request)
        })
    }

    fn find_first_unqueued_attempt(&self) -> Option<usize> {
        self.command_attempts
            .iter()
            .position(|attempt| self.attempt_is_unqueued(attempt))
    }

    /// Appends an ingress record.
    pub(super) fn push_ingress(&mut self, record: IngressRecord) {
        let request = ingress_command_request(&record);
        self.ingress.0.push(record);
        if let Some(request) = request {
            *self
                .command_index
                .queued_requests
                .entry(request)
                .or_insert(0) += 1;
            // Queuing the request of an unqueued attempt makes it queued.
            // Canonical ingress is refused while such an attempt is
            // retained, but the index answers exactly either way.
            if self.command_index.first_unqueued_attempt.is_some() {
                self.command_index.first_unqueued_attempt = self.find_first_unqueued_attempt();
            }
        }
    }

    /// Appends an accepted command.
    pub(super) fn push_command(&mut self, record: CommandRecord) {
        if record.attempt_id.is_none() && self.command_index.first_direct_command.is_none() {
            self.command_index.first_direct_command = Some(self.commands.len());
        }
        self.commands.0.push(record);
    }

    /// Appends a command attempt.
    pub(super) fn push_command_attempt(&mut self, record: CommandAttemptRecord) {
        let position = self.command_attempts.len();
        if self.command_index.first_unqueued_attempt.is_none() && self.attempt_is_unqueued(&record)
        {
            self.command_index.first_unqueued_attempt = Some(position);
        }
        if let Some(request) = record.request_id {
            self.command_index
                .attempt_by_request
                .entry(request)
                .or_insert(position);
        }
        self.command_attempts.0.push(record);
    }

    /// Truncates the three retained journals of the command history to the
    /// given lengths, as a failed transaction restores them. A length at or
    /// past a journal's end leaves that journal as it is.
    pub(super) fn truncate_command_history(
        &mut self,
        commands: usize,
        attempts: usize,
        ingress: usize,
    ) {
        if attempts < self.command_attempts.len() {
            for (offset, attempt) in self.command_attempts[attempts..].iter().enumerate() {
                if let Some(request) = attempt.request_id
                    && self.command_index.attempt_by_request.get(&request)
                        == Some(&(attempts + offset))
                {
                    self.command_index.attempt_by_request.remove(&request);
                }
            }
            self.command_attempts.0.truncate(attempts);
            if self
                .command_index
                .first_unqueued_attempt
                .is_some_and(|position| position >= attempts)
            {
                self.command_index.first_unqueued_attempt = None;
            }
        }
        if commands < self.commands.len() {
            self.commands.0.truncate(commands);
            if self
                .command_index
                .first_direct_command
                .is_some_and(|position| position >= commands)
            {
                self.command_index.first_direct_command = None;
            }
        }
        if ingress < self.ingress.len() {
            let mut unqueued_by_truncation = false;
            for record in &self.ingress.0[ingress..] {
                let Some(request) = ingress_command_request(record) else {
                    continue;
                };
                let Some(count) = self.command_index.queued_requests.get_mut(&request) else {
                    continue;
                };
                *count -= 1;
                if *count == 0 {
                    self.command_index.queued_requests.remove(&request);
                    unqueued_by_truncation |=
                        self.command_index.attempt_by_request.contains_key(&request);
                }
            }
            self.ingress.0.truncate(ingress);
            // A retained attempt whose request's ingress the truncation
            // removed is unqueued now. Transactions truncate the attempts
            // they appended with their ingress, but the index answers
            // exactly either way.
            if unqueued_by_truncation {
                self.command_index.first_unqueued_attempt = self.find_first_unqueued_attempt();
            }
        }
    }

    /// Replaces the retained command history, as a restore does, and builds
    /// its index. The archived request tables must already be in place.
    pub(super) fn restore_command_history(
        &mut self,
        commands: Vec<CommandRecord>,
        attempts: Vec<CommandAttemptRecord>,
        ingress: Vec<IngressRecord>,
    ) {
        self.commands = RetainedJournal(commands.into());
        self.command_attempts = RetainedJournal(attempts.into());
        self.ingress = RetainedJournal(ingress.into());
        self.rebuild_command_index();
    }

    /// Drops the sealed part of the command history: the first `commands`
    /// commands, the first `attempts` attempts, and every ingress record. The
    /// seal must already have extended the archived request tables.
    pub(super) fn seal_command_history(&mut self, commands: usize, attempts: usize) {
        self.commands.0.drain(..commands);
        self.command_attempts.0.drain(..attempts);
        self.ingress.0.clear();
        self.rebuild_command_index();
    }

    fn rebuild_command_index(&mut self) {
        let mut index = CommandHistoryIndex::default();
        for record in &self.ingress {
            if let Some(request) = ingress_command_request(record) {
                *index.queued_requests.entry(request).or_insert(0) += 1;
            }
        }
        index.first_direct_command = self
            .commands
            .iter()
            .position(|record| record.attempt_id.is_none());
        for (position, attempt) in self.command_attempts.iter().enumerate() {
            if let Some(request) = attempt.request_id {
                index.attempt_by_request.entry(request).or_insert(position);
            }
        }
        self.command_index = index;
        self.command_index.first_unqueued_attempt = self.find_first_unqueued_attempt();
    }

    /// Whether the retained commands hold a legacy-direct command.
    pub(super) const fn has_retained_direct_command(&self) -> bool {
        self.command_index.first_direct_command.is_some()
    }

    /// Whether the retained history holds a legacy-direct command or an
    /// attempt whose request was never queued as ingress, retained or
    /// archived.
    pub(super) const fn has_retained_unqueued_command_history(&self) -> bool {
        self.command_index.first_direct_command.is_some()
            || self.command_index.first_unqueued_attempt.is_some()
    }

    /// The first retained attempt of a command request.
    pub(super) fn retained_attempt_for_request(
        &self,
        request: CommandRequestId,
    ) -> Option<&CommandAttemptRecord> {
        let position = *self.command_index.attempt_by_request.get(&request)?;
        self.command_attempts.get(position)
    }

    /// How many retained command and decision ingress records carry a
    /// command request.
    pub(super) fn retained_queued_request_count(&self, request: CommandRequestId) -> usize {
        self.command_index
            .queued_requests
            .get(&request)
            .copied()
            .unwrap_or(0)
    }
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
    /// [`CommitmentDomains::KNOWLEDGE_RECORDS`]. The generic ledger keeps its
    /// own chunked roots.
    pub(super) knowledge_actors: Option<String>,
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
    /// The generic knowledge ledger changed. The knowledge root is
    /// recombined from the cached actor sub-root and the ledger's chunked
    /// root, whose unchanged chunks are not hashed again.
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

/// The scheduler's collections are persistent (`im`) structures, so a
/// boundary's rollback copy and a fork share them and a change copies only
/// the paths it changes. Iterate them forward only (see
/// [`RuntimeCurrentState`]).
#[derive(Clone)]
pub(super) struct RuntimeScheduler {
    pub(super) initial_time: SimTime,
    pub(super) now: SimTime,
    pub(super) actions: im::OrdMap<ScheduleKey, Arc<ScheduledAction>>,
    pub(super) pending_ingress: im::OrdSet<IngressQueueKey>,
    /// Derived index of retained plugin ingress withdrawn by a terminal
    /// cancellation record. Rebuilt from the ingress journal on restore and
    /// cleared when the retained journal is sealed.
    pub(super) cancelled_ingress: im::OrdSet<super::IngressId>,
    /// Registered transition manifests awaiting their ready boundary,
    /// committed under the scheduler root.
    pub(super) transition_manifests:
        im::OrdMap<super::TransitionManifestId, super::PendingTransitionManifest>,
    /// Derived ledger of the boundaries settled within the run's boundary
    /// schedule horizon of the latest boundary, in settlement order.
    /// Maintained at boundary commit, rebuilt from the full journal on
    /// restore, and kept across a seal.
    pub(super) settled_boundaries: im::Vector<std::sync::Arc<super::BoundarySummary>>,
}

impl RuntimeScheduler {
    /// Removes the earliest scheduled action and returns it, owned.
    pub(super) fn pop_first_action(&mut self) -> Option<ScheduledAction> {
        let key = self.actions.get_min()?.0.clone();
        self.actions.remove(&key).map(Arc::unwrap_or_clone)
    }
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

/// The position of each initial-scenario domain record in the scenario's
/// list, by reference: built once per scenario and shared, through an `Arc`,
/// by the runtime, its forks, and the snapshot it was validated from.
/// Evidence that cites an initial-scenario version resolves through it in
/// O(log n) instead of a scan of the scenario.
#[derive(Debug, Default)]
pub(super) struct InitialRecordIndex(BTreeMap<DomainRecordRef, usize>);

#[cfg(test)]
thread_local! {
    /// Scenario records [`InitialRecordIndex::version`] has read on this
    /// thread, so a test can count what resolving evidence reads.
    pub(super) static INITIAL_RECORDS_READ: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
}

impl InitialRecordIndex {
    pub(super) fn of(scenario: Option<&Scenario>) -> Arc<Self> {
        Arc::new(Self(
            scenario
                .iter()
                .flat_map(|scenario| scenario.domain_records.iter().enumerate())
                .map(|(index, record)| (record.reference.clone(), index))
                .collect(),
        ))
    }

    /// The position of a record in the scenario's list.
    pub(super) fn get(&self, reference: &DomainRecordRef) -> Option<&usize> {
        self.0.get(reference)
    }

    /// The initial record `reference` names at its version, if the scenario
    /// holds it at that version.
    pub(super) fn version<'a>(
        &self,
        scenario: Option<&'a Scenario>,
        reference: &super::DomainRecordVersionRef,
    ) -> Option<&'a DomainRecord> {
        let record = scenario?
            .domain_records
            .get(*self.get(&reference.record)?)?;
        #[cfg(test)]
        INITIAL_RECORDS_READ.set(INITIAL_RECORDS_READ.get() + 1);
        (record.reference == reference.record && record.version == reference.version)
            .then_some(record)
    }
}

#[derive(Clone)]
pub(super) struct RuntimeMetadata {
    /// The bound initial scenario, shared by forks and rollback copies; it
    /// never changes after the run starts.
    pub(super) initial_scenario: Option<Arc<Scenario>>,
    pub(super) initial_domain_record_indexes: Arc<InitialRecordIndex>,
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

/// Current state. Every collection is a persistent (`im`) structure or
/// shared through an `Arc`, so the per-command and per-boundary rollback
/// copies, a boundary's stable view of phases 1 to 9, and a fork share it in
/// O(1), and a change copies only the tree paths it changes. Values that own
/// heap data and are rewritten during a run are held in an `Arc`, so a
/// copied tree node copies pointers, not values; a change goes through
/// `Arc::make_mut`. Iterate these collections forward only: `im` 15.1.0
/// ordered maps and sets break on reverse traversal past about 2,100
/// entries (no `.rev()`, `next_back`, `get_prev`, or reverse ranges;
/// `get_min` and `get_max` are safe).
#[derive(Clone)]
pub(super) struct RuntimeCurrentState {
    pub(super) entities: im::OrdSet<EntityRef>,
    pub(super) people: im::OrdMap<PersonId, Arc<Person>>,
    /// Core life and custody state; absent entries are alive and free.
    pub(super) person_availability: im::OrdMap<PersonId, PersonAvailability>,
    /// Runtime-created persons in allocation order, keyed for next-boundary
    /// correlation lookup independently of retained evidence.
    pub(super) created_persons: im::Vector<super::CreatedPerson>,
    pub(super) letters: im::OrdMap<canwu_core::LetterId, Arc<super::LetterCargo>>,
    /// Written only when a run starts, so their values are held inline.
    pub(super) governments: im::OrdMap<GovernmentId, Government>,
    pub(super) territories: im::OrdMap<TerritoryId, Territory>,
    pub(super) routes: im::OrdMap<RouteId, Route>,
    pub(super) armies: im::OrdMap<ArmyId, Arc<Army>>,
    /// Shared, so boundary and transaction captures, early-phase views, and
    /// forks copy nothing. A change goes through `Arc::make_mut`: it copies
    /// the compatibility actor projection and shares the persistent holder
    /// ledgers, of which it copies only the paths it changes.
    pub(super) knowledge: std::sync::Arc<KnowledgeSnapshot>,
    pub(super) plugin_components: im::OrdMap<Arc<PluginComponentKey>, Arc<PluginComponentRecord>>,
    /// Persistent current-state root. Boundary checkpoints and forks clone the
    /// handle in O(1), while writes copy only paths in the persistent map.
    pub(super) domain_records: PersistentDomainRecordStore,
    pub(super) decisions: DecisionState,
    pub(super) root_seed: u64,
    /// Persisted authority credential root. It is intentionally separate from
    /// the deterministic simulation RNG seed and is never used for draws.
    pub(super) authority_root_seed: u64,
    pub(super) random_streams: im::OrdMap<RandomStreamKey, RandomStreamState>,
}

#[derive(Clone)]
pub(super) struct RuntimeState {
    pub(super) current: RuntimeCurrentState,
    pub(super) scheduler: RuntimeScheduler,
    pub(super) counters: RuntimeCounters,
    pub(super) metadata: RuntimeMetadata,
    pub(super) evidence: RuntimeEvidence,
}
