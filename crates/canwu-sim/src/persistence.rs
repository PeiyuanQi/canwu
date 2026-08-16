use super::{
    ADMISSION_CURSOR_FORMAT_VERSION, BoundaryReceipt, BoundaryRecord, BoundaryRequest, CanwuError,
    CommandAttemptRecord, CommandEnvelope, CommandOutcome, CommandReceipt, CommandRecord,
    CommandRequest, DomainRecord, DomainRecordRef, DomainRecordType, ENGINE_VERSION, ErrorCode,
    IngressPayload, IngressReceipt, IngressRecord, KnowledgeSnapshot, PluginIngressRequest,
    RandomDrawRecord, ReplayJournal, RuntimeEvidence, SNAPSHOT_FORMAT_VERSION,
    STATE_REVISION_FORMAT_VERSION, ScheduledRecord, SimDuration, SimEvent, SimTime, Simulation,
    SimulationPlugin, SimulationSnapshot, SystemCadence, TypedDomainRecordRef, WorldSnapshot,
    has_unqueued_command_history, invalid_snapshot_error,
};
use crate::state::{ArchivedCommandRequestOutcome, ArchivedIngressRequest};
use serde::{Deserialize, Serialize};

/// Version of current-state checkpoints plus append-only evidence segments.
pub const CHECKPOINT_JOURNAL_FORMAT_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
/// Monotonic cuts through every append-only evidence journal.
pub struct EvidenceCursor {
    pub event_count: u64,
    pub command_count: u64,
    pub command_attempt_count: u64,
    pub ingress_count: u64,
    pub boundary_count: u64,
    pub random_draw_count: u64,
}

impl EvidenceCursor {
    fn from_evidence(evidence: &RuntimeEvidence) -> Result<Self, CanwuError> {
        let count = |len: usize, label: &str| {
            u64::try_from(len).map_err(|_| {
                CanwuError::new(
                    ErrorCode::IdentifierExhausted,
                    format!("{label} journal length exceeds the persistent cursor space"),
                )
            })
        };
        Ok(Self {
            event_count: evidence
                .archived
                .event_count
                .checked_add(count(evidence.events.len(), "event")?)
                .ok_or_else(|| invalid_snapshot_error("event journal cursor is exhausted"))?,
            command_count: evidence
                .archived
                .command_count
                .checked_add(count(evidence.commands.len(), "command")?)
                .ok_or_else(|| invalid_snapshot_error("command journal cursor is exhausted"))?,
            command_attempt_count: evidence
                .archived
                .command_attempt_count
                .checked_add(count(evidence.command_attempts.len(), "command-attempt")?)
                .ok_or_else(|| {
                    invalid_snapshot_error("command-attempt journal cursor is exhausted")
                })?,
            ingress_count: evidence
                .archived
                .ingress_count
                .checked_add(count(evidence.ingress.len(), "ingress")?)
                .ok_or_else(|| invalid_snapshot_error("ingress journal cursor is exhausted"))?,
            boundary_count: evidence
                .archived
                .boundary_count
                .checked_add(count(evidence.boundaries.len(), "boundary")?)
                .ok_or_else(|| invalid_snapshot_error("boundary journal cursor is exhausted"))?,
            random_draw_count: evidence
                .archived
                .random_draw_count
                .checked_add(count(evidence.random_draws.len(), "random-draw")?)
                .ok_or_else(|| invalid_snapshot_error("random-draw journal cursor is exhausted"))?,
        })
    }

    pub(super) fn checked_advance(
        self,
        segment: &EvidenceJournalSegment,
    ) -> Result<Self, CanwuError> {
        let advance = |value: u64, len: usize, label: &str| {
            value
                .checked_add(u64::try_from(len).map_err(|_| {
                    invalid_snapshot_error(format!(
                        "{label} journal segment exceeds the persistent cursor space"
                    ))
                })?)
                .ok_or_else(|| {
                    invalid_snapshot_error(format!(
                        "{label} journal cursor exceeds the persistent cursor space"
                    ))
                })
        };
        Ok(Self {
            event_count: advance(self.event_count, segment.events.len(), "event")?,
            command_count: advance(self.command_count, segment.commands.len(), "command")?,
            command_attempt_count: advance(
                self.command_attempt_count,
                segment.command_attempts.len(),
                "command-attempt",
            )?,
            ingress_count: advance(self.ingress_count, segment.ingress.len(), "ingress")?,
            boundary_count: advance(self.boundary_count, segment.boundaries.len(), "boundary")?,
            random_draw_count: advance(
                self.random_draw_count,
                segment.random_draws.len(),
                "random-draw",
            )?,
        })
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
/// Current authoritative state plus the journal cut required to validate it.
///
/// `state` deliberately contains empty append-only evidence arrays. It is not a
/// standalone `SimulationSnapshot`; load it only with the contiguous evidence
/// segments ending at `journal_end`.
pub struct SimulationCheckpoint {
    pub format_version: u32,
    pub journal_end: EvidenceCursor,
    pub state: SimulationSnapshot,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
/// One contiguous append-only evidence range for incremental archival.
pub struct EvidenceJournalSegment {
    pub format_version: u32,
    pub start: EvidenceCursor,
    pub end: EvidenceCursor,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<SimEvent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<CommandRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command_attempts: Vec<CommandAttemptRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ingress: Vec<IngressRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub boundaries: Vec<BoundaryRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub random_draws: Vec<RandomDrawRecord>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
/// Portable full-save bundle built from a current-state checkpoint and journal segments.
pub struct CheckpointJournal {
    pub checkpoint: SimulationCheckpoint,
    pub segments: Vec<EvidenceJournalSegment>,
}

/// A live simulation whose sealed evidence prefixes are owned by the caller.
///
/// This opt-in runtime preserves current authoritative state, deterministic
/// commitments, idempotency, and continuation behavior while retaining only
/// the evidence appended since the most recent seal. Every returned segment is
/// part of the permanent replay record and must be stored contiguously by the
/// caller.
pub struct CompactedSimulation {
    simulation: Simulation,
}

impl CompactedSimulation {
    /// Returns the monotonic cut through sealed and retained evidence.
    pub fn evidence_cursor(&self) -> Result<EvidenceCursor, CanwuError> {
        self.simulation.evidence_cursor()
    }

    /// Captures current state and the total journal cut without cloning sealed evidence.
    pub fn checkpoint(&self) -> Result<SimulationCheckpoint, CanwuError> {
        self.simulation.checkpoint()
    }

    /// Seals and releases the current retained evidence tail.
    ///
    /// The runtime changes only after the segment is fully constructed and its
    /// continuation indexes are prepared. An empty retained tail returns
    /// `None`. The caller owns persistence and must keep all non-empty segments
    /// in exact cursor order for save restoration or replay.
    pub fn seal_evidence(&mut self) -> Result<Option<EvidenceJournalSegment>, CanwuError> {
        self.simulation.seal_retained_evidence()
    }

    #[must_use]
    pub const fn time(&self) -> SimTime {
        self.simulation.time()
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.simulation.revision()
    }

    #[must_use]
    pub fn checkpoint_hash(&self) -> &str {
        self.simulation.checkpoint_hash()
    }

    #[must_use]
    pub fn boundary_head_hash(&self) -> Option<&str> {
        self.simulation.boundary_head_hash()
    }

    #[must_use]
    pub fn world(&self) -> WorldSnapshot {
        self.simulation.world()
    }

    #[must_use]
    pub fn knowledge(&self) -> &KnowledgeSnapshot {
        self.simulation.knowledge()
    }

    #[must_use]
    pub fn domain_record(&self, reference: &DomainRecordRef) -> Option<&DomainRecord> {
        self.simulation.domain_record(reference)
    }

    #[must_use]
    pub fn typed_domain_record<T: DomainRecordType>(
        &self,
        reference: &TypedDomainRecordRef<T>,
    ) -> Option<&DomainRecord> {
        self.simulation.typed_domain_record(reference)
    }

    pub fn submit(&mut self, envelope: CommandEnvelope) -> Result<CommandReceipt, CanwuError> {
        self.simulation.submit(envelope)
    }

    pub fn process_command(
        &mut self,
        request: CommandRequest,
    ) -> Result<CommandOutcome, CanwuError> {
        self.simulation.process_command(request)
    }

    pub fn enqueue_command(
        &mut self,
        due_at: SimTime,
        priority: i32,
        request: CommandRequest,
    ) -> Result<IngressReceipt, CanwuError> {
        self.simulation.enqueue_command(due_at, priority, request)
    }

    pub fn enqueue_plugin_ingress(
        &mut self,
        request: PluginIngressRequest,
    ) -> Result<IngressReceipt, CanwuError> {
        self.simulation.enqueue_plugin_ingress(request)
    }

    pub fn schedule_calendar_boundary(
        &mut self,
        due_at: SimTime,
        cadences: Vec<SystemCadence>,
    ) -> Result<IngressReceipt, CanwuError> {
        self.simulation.schedule_calendar_boundary(due_at, cadences)
    }

    pub fn advance(&mut self, duration: SimDuration) -> Result<Vec<SimEvent>, CanwuError> {
        self.simulation.advance(duration)
    }

    pub fn advance_canonical(
        &mut self,
        duration: SimDuration,
    ) -> Result<Vec<BoundaryReceipt>, CanwuError> {
        self.simulation.advance_canonical(duration)
    }

    pub fn step_canonical(&mut self) -> Result<Option<BoundaryReceipt>, CanwuError> {
        self.simulation.step_canonical()
    }

    pub fn settle_boundary(
        &mut self,
        request: BoundaryRequest,
    ) -> Result<BoundaryReceipt, CanwuError> {
        self.simulation.settle_boundary(request)
    }

    /// Reconstructs a validated full snapshot from the supplied sealed prefix
    /// plus the currently retained tail.
    pub fn snapshot_with_segments(
        &self,
        mut segments: Vec<EvidenceJournalSegment>,
    ) -> Result<SimulationSnapshot, CanwuError> {
        let tail = self
            .simulation
            .journal_segment_since(self.simulation.state.evidence.archived)?;
        if tail.start != tail.end {
            segments.push(tail);
        }
        let snapshot =
            Simulation::snapshot_from_checkpoint_and_journal(self.checkpoint()?, segments)?;
        Simulation::from_snapshot(snapshot.clone())?;
        Ok(snapshot)
    }

    /// Produces the ordinary exact-replay journal after validating the supplied archive.
    pub fn replay_journal_with_segments(
        &self,
        segments: Vec<EvidenceJournalSegment>,
    ) -> Result<ReplayJournal, CanwuError> {
        let snapshot = self.snapshot_with_segments(segments)?;
        let simulation = Simulation::from_snapshot(snapshot)?;
        Ok(simulation.replay_journal())
    }

    /// Restores and validates a checkpoint plus its archive, then enters the
    /// compact interface with that evidence retained until the caller seals it.
    pub fn from_checkpoint_and_journal(
        checkpoint: SimulationCheckpoint,
        segments: Vec<EvidenceJournalSegment>,
    ) -> Result<Self, CanwuError> {
        Simulation::from_checkpoint_and_journal(checkpoint, segments)?.into_compacted()
    }

    /// Restores a compact runtime and rehydrates its exact executable plugins.
    pub fn from_checkpoint_and_journal_with_plugins(
        checkpoint: SimulationCheckpoint,
        segments: Vec<EvidenceJournalSegment>,
        plugins: &[&dyn SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        let mut simulation = Simulation::from_checkpoint_and_journal(checkpoint, segments)?;
        for plugin in plugins {
            simulation.register_plugin(*plugin)?;
        }
        simulation.ensure_runtime_ready()?;
        simulation.into_compacted()
    }
}

impl Simulation {
    /// Converts this runtime into the opt-in compact journal interface.
    ///
    /// Conversion itself preserves the complete retained history. Call
    /// [`CompactedSimulation::seal_evidence`] to release a validated segment
    /// explicitly.
    pub fn into_compacted(self) -> Result<CompactedSimulation, CanwuError> {
        self.ensure_runtime_ready()?;
        Ok(CompactedSimulation { simulation: self })
    }

    fn ensure_retained_evidence_is_sealable(&self) -> Result<(), CanwuError> {
        if !self.state.scheduler.pending_ingress.is_empty() {
            return Err(CanwuError::new(
                ErrorCode::ArchiveNotReady,
                "live evidence can be sealed only when the canonical ingress queue is empty",
            ));
        }
        let reads_archived_evidence = |reads: &[super::StateKey]| {
            reads.iter().any(|state| {
                state == &super::StateKey::core_commands()
                    || state == &super::StateKey::core_events()
                    || state == &super::StateKey::core_ingress()
            })
        };
        if self.plugins.descriptors().any(|descriptor| {
            descriptor
                .systems
                .iter()
                .any(|contract| reads_archived_evidence(&contract.reads))
                || descriptor
                    .boundary_systems
                    .iter()
                    .any(|contract| reads_archived_evidence(&contract.reads))
                || descriptor
                    .commands
                    .iter()
                    .any(|contract| reads_archived_evidence(&contract.reads))
        }) {
            return Err(CanwuError::new(
                ErrorCode::ArchiveNotReady,
                "live evidence sealing requires plugins whose declared reads use current state rather than historical command, event, or ingress records",
            ));
        }

        let admitted_attempts: std::collections::BTreeSet<_> = self
            .state
            .evidence
            .boundaries
            .iter()
            .flat_map(|record| record.admitted_attempts.iter().copied())
            .collect();
        if admitted_attempts.len() != self.state.evidence.command_attempts.len()
            || self
                .state
                .evidence
                .command_attempts
                .iter()
                .any(|attempt| !admitted_attempts.contains(&attempt.id))
        {
            return Err(CanwuError::new(
                ErrorCode::ArchiveNotReady,
                "live evidence sealing requires every retained command attempt to belong to a completed boundary",
            ));
        }
        let admitted_commands: std::collections::BTreeSet<_> = self
            .state
            .evidence
            .boundaries
            .iter()
            .flat_map(|record| record.admitted_commands.iter().copied())
            .collect();
        if admitted_commands.len() != self.state.evidence.commands.len()
            || self
                .state
                .evidence
                .commands
                .iter()
                .any(|command| !admitted_commands.contains(&command.id))
        {
            return Err(CanwuError::new(
                ErrorCode::ArchiveNotReady,
                "live evidence sealing requires every retained command to belong to a completed boundary",
            ));
        }
        let admitted_ingress: std::collections::BTreeSet<_> = self
            .state
            .evidence
            .boundaries
            .iter()
            .flat_map(|record| record.admitted_ingress.iter().copied())
            .collect();
        if admitted_ingress.len() != self.state.evidence.ingress.len()
            || self
                .state
                .evidence
                .ingress
                .iter()
                .any(|record| !admitted_ingress.contains(&record.id))
        {
            return Err(CanwuError::new(
                ErrorCode::ArchiveNotReady,
                "live evidence sealing requires every retained ingress record to belong to a completed boundary",
            ));
        }
        let admitted_events: std::collections::BTreeSet<_> = self
            .state
            .evidence
            .boundaries
            .iter()
            .flat_map(|record| record.admitted_events.iter().copied())
            .collect();
        if self.state.counters.admitted_event_count
            != self
                .state
                .evidence
                .archived
                .event_count
                .checked_add(
                    u64::try_from(self.state.evidence.events.len()).map_err(|_| {
                        CanwuError::new(
                            ErrorCode::ArchiveNotReady,
                            "retained event count exceeds the live archive cursor range",
                        )
                    })?,
                )
                .ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::ArchiveNotReady,
                        "retained event cursor is exhausted",
                    )
                })?
            || admitted_events.len() != self.state.evidence.events.len()
            || self
                .state
                .evidence
                .events
                .iter()
                .any(|event| !admitted_events.contains(&event.id))
        {
            return Err(CanwuError::new(
                ErrorCode::ArchiveNotReady,
                "live evidence sealing requires every retained event to be admitted by a later completed boundary",
            ));
        }
        Ok(())
    }

    fn seal_retained_evidence(&mut self) -> Result<Option<EvidenceJournalSegment>, CanwuError> {
        let start = self.state.evidence.archived;
        let end = self.evidence_cursor()?;
        if start == end {
            return Ok(None);
        }
        self.ensure_retained_evidence_is_sealable()?;
        let checkpoint_hash = self.state.metadata.checkpoint_hash.clone();
        let commitment_roots = self.state.metadata.commitment_roots.clone();
        let commitment_cache = self.state.metadata.commitment_cache.clone();
        let prepared = (|| {
            self.refresh_checkpoint_hash()?;

            let mut archived_command_requests = Vec::new();
            for attempt in &self.state.evidence.command_attempts {
                let Some(request_id) = attempt.request_id else {
                    continue;
                };
                let outcome = self.command_outcome_from_attempt(attempt)?;
                archived_command_requests.push((
                    request_id,
                    ArchivedCommandRequestOutcome {
                        input_hash: super::canonical_hash(
                            "canwu.archive.command.request.v1",
                            &(attempt.expected_revision, &attempt.envelope),
                        )?,
                        outcome,
                    },
                ));
            }

            let mut archived_ingress_requests = Vec::new();
            for record in &self.state.evidence.ingress {
                let IngressPayload::Command { request } = &record.payload else {
                    continue;
                };
                archived_ingress_requests.push((
                    request.request_id,
                    ArchivedIngressRequest {
                        input_hash: super::canonical_hash(
                            "canwu.archive.ingress.command.v1",
                            &(record.due_at, record.priority, request.as_ref()),
                        )?,
                        receipt: IngressReceipt {
                            ingress_id: record.id,
                            issued_at: record.issued_at,
                            due_at: record.due_at,
                        },
                    },
                ));
            }
            Ok::<_, CanwuError>((archived_command_requests, archived_ingress_requests))
        })();
        let (archived_command_requests, archived_ingress_requests) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                self.state.metadata.checkpoint_hash = checkpoint_hash;
                self.state.metadata.commitment_roots = commitment_roots;
                self.state.metadata.commitment_cache = commitment_cache;
                return Err(error);
            }
        };

        self.state.evidence.archived_boundary_head = self
            .state
            .evidence
            .boundaries
            .last()
            .map(|record| record.hash.clone())
            .or_else(|| self.state.evidence.archived_boundary_head.clone());
        self.state.evidence.archived_legacy_commands |= self
            .state
            .evidence
            .commands
            .iter()
            .any(|record| record.attempt_id.is_none());
        self.state.evidence.archived_tracked_attempts |=
            !self.state.evidence.command_attempts.is_empty()
                || !self.state.evidence.ingress.is_empty();
        self.state.evidence.archived_unqueued_command_history |= has_unqueued_command_history(
            &self.state.evidence.commands,
            &self.state.evidence.command_attempts,
            &self.state.evidence.ingress,
        );
        self.state
            .evidence
            .archived_command_requests
            .extend(archived_command_requests);
        self.state
            .evidence
            .archived_ingress_requests
            .extend(archived_ingress_requests);
        self.state.evidence.archived = end;
        let segment = EvidenceJournalSegment {
            format_version: CHECKPOINT_JOURNAL_FORMAT_VERSION,
            start,
            end,
            events: std::mem::take(&mut self.state.evidence.events),
            commands: std::mem::take(&mut self.state.evidence.commands),
            command_attempts: std::mem::take(&mut self.state.evidence.command_attempts),
            ingress: std::mem::take(&mut self.state.evidence.ingress),
            boundaries: std::mem::take(&mut self.state.evidence.boundaries),
            random_draws: std::mem::take(&mut self.state.evidence.random_draws),
        };
        Ok(Some(segment))
    }

    pub(super) fn checkpoint_state(&self) -> SimulationSnapshot {
        SimulationSnapshot {
            engine_version: ENGINE_VERSION.to_owned(),
            snapshot_format_version: SNAPSHOT_FORMAT_VERSION,
            run_manifest: Some(self.state.metadata.run_manifest.clone()),
            run_manifest_hash: self.state.metadata.run_manifest_hash.clone(),
            run_configuration: Some(self.state.metadata.run_configuration.clone()),
            checkpoint_hash: self.state.metadata.checkpoint_hash.clone(),
            commitment_format_version: self.state.metadata.commitment_format_version,
            commitment_roots: self.state.metadata.commitment_roots.clone(),
            revision_format_version: STATE_REVISION_FORMAT_VERSION,
            state_revision: self.state.counters.state_revision,
            replay_revision_format_version: self.state.metadata.replay_revision_format_version,
            admission_cursor_format_version: ADMISSION_CURSOR_FORMAT_VERSION,
            admitted_attempt_count: self.state.counters.admitted_attempt_count,
            admitted_command_count: self.state.counters.admitted_command_count,
            admitted_event_count: self.state.counters.admitted_event_count,
            initial_time: self.state.scheduler.initial_time,
            initial_scenario: self.bound_initial_scenario().cloned(),
            now: self.state.scheduler.now,
            plugin_registration_closed: self.state.metadata.plugin_registration_closed,
            world: self.world(),
            knowledge: self.state.current.knowledge.clone(),
            events: Vec::new(),
            commands: Vec::new(),
            command_attempts: Vec::new(),
            ingress: Vec::new(),
            boundaries: Vec::new(),
            plugin_components: self
                .state
                .current
                .plugin_components
                .values()
                .cloned()
                .collect(),
            domain_records: self
                .state
                .current
                .domain_records
                .values()
                .cloned()
                .collect(),
            plugin_descriptors: self.plugins.descriptors().cloned().collect(),
            schema: self.schema.clone(),
            root_seed: self.state.current.root_seed,
            random_streams: self
                .state
                .current
                .random_streams
                .values()
                .cloned()
                .collect(),
            random_draws: Vec::new(),
            scheduled: self
                .state
                .scheduler
                .actions
                .iter()
                .map(|(key, action)| ScheduledRecord {
                    key: key.clone(),
                    action: action.clone(),
                })
                .collect(),
            legacy_rng: None,
            next_event_id: self.state.counters.next_event_id,
            next_command_id: self.state.counters.next_command_id,
            next_command_attempt_id: self.state.counters.next_command_attempt_id,
            next_ingress_id: self.state.counters.next_ingress_id,
            next_boundary_id: self.state.counters.next_boundary_id,
            next_random_draw_id: self.state.counters.next_random_draw_id,
            next_schedule_sequence: self.state.counters.next_schedule_sequence,
            next_correlation_id: self.state.counters.next_correlation_id,
        }
    }

    /// Returns the current monotonic cut through every append-only journal.
    pub fn evidence_cursor(&self) -> Result<EvidenceCursor, CanwuError> {
        EvidenceCursor::from_evidence(&self.state.evidence)
    }

    /// Captures current authoritative state without cloning accumulated evidence.
    pub fn checkpoint(&self) -> Result<SimulationCheckpoint, CanwuError> {
        Ok(SimulationCheckpoint {
            format_version: CHECKPOINT_JOURNAL_FORMAT_VERSION,
            journal_end: self.evidence_cursor()?,
            state: self.checkpoint_state(),
        })
    }

    /// Clones only evidence appended after a previously persisted cursor.
    pub fn journal_segment_since(
        &self,
        start: EvidenceCursor,
    ) -> Result<EvidenceJournalSegment, CanwuError> {
        let end = self.evidence_cursor()?;
        let cut = |value: u64, archived: u64, len: usize, label: &str| {
            let value = value.checked_sub(archived).ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::InvalidSnapshot,
                    format!("{label} journal cursor precedes the retained live evidence window"),
                )
            })?;
            let value = usize::try_from(value).map_err(|_| {
                CanwuError::new(
                    ErrorCode::InvalidSnapshot,
                    format!("{label} journal cursor is not representable on this platform"),
                )
            })?;
            if value > len {
                return Err(CanwuError::new(
                    ErrorCode::InvalidSnapshot,
                    format!("{label} journal cursor exceeds the current evidence tail"),
                ));
            }
            Ok(value)
        };
        let archived = self.state.evidence.archived;
        let event_start = cut(
            start.event_count,
            archived.event_count,
            self.state.evidence.events.len(),
            "event",
        )?;
        let command_start = cut(
            start.command_count,
            archived.command_count,
            self.state.evidence.commands.len(),
            "command",
        )?;
        let attempt_start = cut(
            start.command_attempt_count,
            archived.command_attempt_count,
            self.state.evidence.command_attempts.len(),
            "command-attempt",
        )?;
        let ingress_start = cut(
            start.ingress_count,
            archived.ingress_count,
            self.state.evidence.ingress.len(),
            "ingress",
        )?;
        let boundary_start = cut(
            start.boundary_count,
            archived.boundary_count,
            self.state.evidence.boundaries.len(),
            "boundary",
        )?;
        let draw_start = cut(
            start.random_draw_count,
            archived.random_draw_count,
            self.state.evidence.random_draws.len(),
            "random-draw",
        )?;
        Ok(EvidenceJournalSegment {
            format_version: CHECKPOINT_JOURNAL_FORMAT_VERSION,
            start,
            end,
            events: self.state.evidence.events[event_start..].to_vec(),
            commands: self.state.evidence.commands[command_start..].to_vec(),
            command_attempts: self.state.evidence.command_attempts[attempt_start..].to_vec(),
            ingress: self.state.evidence.ingress[ingress_start..].to_vec(),
            boundaries: self.state.evidence.boundaries[boundary_start..].to_vec(),
            random_draws: self.state.evidence.random_draws[draw_start..].to_vec(),
        })
    }

    /// Builds a portable full-save bundle with one segment from genesis.
    pub fn checkpoint_journal(&self) -> Result<CheckpointJournal, CanwuError> {
        if self.state.evidence.archived != EvidenceCursor::default() {
            return Err(CanwuError::new(
                ErrorCode::InvalidSnapshot,
                "a compact live runtime requires its previously sealed evidence segments to build a portable save",
            ));
        }
        let segment = self.journal_segment_since(EvidenceCursor::default())?;
        Ok(CheckpointJournal {
            checkpoint: self.checkpoint()?,
            segments: (segment.start != segment.end)
                .then_some(segment)
                .into_iter()
                .collect(),
        })
    }

    /// Serializes the portable full-save checkpoint-journal bundle as JSON.
    pub fn checkpoint_journal_json(&self) -> Result<String, CanwuError> {
        serde_json::to_string_pretty(&self.checkpoint_journal()?).map_err(|error| {
            CanwuError::new(
                ErrorCode::InvalidSnapshot,
                format!("could not serialize checkpoint journal: {error}"),
            )
        })
    }

    fn snapshot_from_checkpoint_and_journal(
        checkpoint: SimulationCheckpoint,
        segments: Vec<EvidenceJournalSegment>,
    ) -> Result<SimulationSnapshot, CanwuError> {
        if checkpoint.format_version != CHECKPOINT_JOURNAL_FORMAT_VERSION {
            return Err(invalid_snapshot_error(format!(
                "checkpoint-journal format {} is unsupported; this engine reads format {CHECKPOINT_JOURNAL_FORMAT_VERSION}",
                checkpoint.format_version
            )));
        }
        let mut snapshot = checkpoint.state;
        if snapshot.snapshot_format_version != SNAPSHOT_FORMAT_VERSION {
            return Err(invalid_snapshot_error(format!(
                "checkpoint-journal format {CHECKPOINT_JOURNAL_FORMAT_VERSION} requires snapshot format {SNAPSHOT_FORMAT_VERSION}"
            )));
        }
        if !snapshot.events.is_empty()
            || !snapshot.commands.is_empty()
            || !snapshot.command_attempts.is_empty()
            || !snapshot.ingress.is_empty()
            || !snapshot.boundaries.is_empty()
            || !snapshot.random_draws.is_empty()
        {
            return Err(invalid_snapshot_error(
                "checkpoint current state must not duplicate append-only evidence",
            ));
        }

        let mut cursor = EvidenceCursor::default();
        for segment in segments {
            if segment.format_version != CHECKPOINT_JOURNAL_FORMAT_VERSION {
                return Err(invalid_snapshot_error(format!(
                    "evidence-journal format {} is unsupported; this engine reads format {CHECKPOINT_JOURNAL_FORMAT_VERSION}",
                    segment.format_version
                )));
            }
            if segment.start != cursor {
                return Err(invalid_snapshot_error(
                    "evidence-journal segments must form one contiguous global prefix",
                ));
            }
            let end = cursor.checked_advance(&segment)?;
            if end == cursor {
                return Err(invalid_snapshot_error(
                    "evidence-journal segments must advance at least one journal cursor",
                ));
            }
            if segment.end != end {
                return Err(invalid_snapshot_error(
                    "evidence-journal segment end does not match its encoded records",
                ));
            }
            snapshot.events.extend(segment.events);
            snapshot.commands.extend(segment.commands);
            snapshot.command_attempts.extend(segment.command_attempts);
            snapshot.ingress.extend(segment.ingress);
            snapshot.boundaries.extend(segment.boundaries);
            snapshot.random_draws.extend(segment.random_draws);
            cursor = end;
        }
        if cursor != checkpoint.journal_end {
            return Err(invalid_snapshot_error(
                "evidence-journal segments do not reach the checkpoint journal cut",
            ));
        }
        Ok(snapshot)
    }

    /// Restores a checkpoint after proving a contiguous journal prefix.
    pub fn from_checkpoint_and_journal(
        checkpoint: SimulationCheckpoint,
        segments: Vec<EvidenceJournalSegment>,
    ) -> Result<Self, CanwuError> {
        Self::from_snapshot(Self::snapshot_from_checkpoint_and_journal(
            checkpoint, segments,
        )?)
    }

    /// Restores a portable checkpoint-journal bundle.
    pub fn from_checkpoint_journal(bundle: CheckpointJournal) -> Result<Self, CanwuError> {
        Self::from_checkpoint_and_journal(bundle.checkpoint, bundle.segments)
    }

    /// Restores a bundle and rehydrates its exact executable plugin contracts.
    pub fn from_checkpoint_journal_with_plugins(
        bundle: CheckpointJournal,
        plugins: &[&dyn SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        let mut simulation = Self::from_checkpoint_journal(bundle)?;
        for plugin in plugins {
            simulation.register_plugin(*plugin)?;
        }
        simulation.ensure_runtime_ready()?;
        Ok(simulation)
    }

    /// Deserializes and restores a portable checkpoint-journal JSON bundle.
    pub fn from_checkpoint_journal_json(json: &str) -> Result<Self, CanwuError> {
        let bundle = serde_json::from_str(json).map_err(|error| {
            CanwuError::new(
                ErrorCode::InvalidSnapshot,
                format!("could not deserialize checkpoint journal: {error}"),
            )
        })?;
        Self::from_checkpoint_journal(bundle)
    }

    /// Deserializes a bundle and rehydrates its exact plugin contracts.
    pub fn from_checkpoint_journal_json_with_plugins(
        json: &str,
        plugins: &[&dyn SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        let bundle = serde_json::from_str(json).map_err(|error| {
            CanwuError::new(
                ErrorCode::InvalidSnapshot,
                format!("could not deserialize checkpoint journal: {error}"),
            )
        })?;
        Self::from_checkpoint_journal_with_plugins(bundle, plugins)
    }
}
