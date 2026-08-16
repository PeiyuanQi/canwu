use super::{
    ADMISSION_CURSOR_FORMAT_VERSION, BoundaryRecord, CanwuError, CommandAttemptRecord,
    CommandRecord, ENGINE_VERSION, ErrorCode, IngressRecord, RandomDrawRecord, RuntimeEvidence,
    SNAPSHOT_FORMAT_VERSION, STATE_REVISION_FORMAT_VERSION, ScheduledRecord, SimEvent, Simulation,
    SimulationPlugin, SimulationSnapshot, invalid_snapshot_error,
};
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
            event_count: count(evidence.events.len(), "event")?,
            command_count: count(evidence.commands.len(), "command")?,
            command_attempt_count: count(evidence.command_attempts.len(), "command-attempt")?,
            ingress_count: count(evidence.ingress.len(), "ingress")?,
            boundary_count: count(evidence.boundaries.len(), "boundary")?,
            random_draw_count: count(evidence.random_draws.len(), "random-draw")?,
        })
    }

    fn checked_advance(self, segment: &EvidenceJournalSegment) -> Result<Self, CanwuError> {
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

impl Simulation {
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
        let cut = |value: u64, len: usize, label: &str| {
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
        let event_start = cut(start.event_count, self.state.evidence.events.len(), "event")?;
        let command_start = cut(
            start.command_count,
            self.state.evidence.commands.len(),
            "command",
        )?;
        let attempt_start = cut(
            start.command_attempt_count,
            self.state.evidence.command_attempts.len(),
            "command-attempt",
        )?;
        let ingress_start = cut(
            start.ingress_count,
            self.state.evidence.ingress.len(),
            "ingress",
        )?;
        let boundary_start = cut(
            start.boundary_count,
            self.state.evidence.boundaries.len(),
            "boundary",
        )?;
        let draw_start = cut(
            start.random_draw_count,
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
