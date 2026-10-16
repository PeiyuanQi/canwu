//! Sealing with work in flight and the continuation checkpoint.
//!
//! A seal moves each journal up to its settled cut and keeps, as pinned
//! evidence, what the run still needs below that cut: the continuation
//! closure. A continuation checkpoint carries current state with that closure
//! and the indexes the run keeps instead of its history, so a run restored
//! from it continues exactly as the uninterrupted run does.

use super::{
    BoundaryId, BoundaryRecord, BoundarySummary, CanwuError, CauseRef, CommandAttemptId,
    CommandAttemptRecord, CommandId, CommandOutcome, CommandRecord, CommandRequestId,
    DecisionRequestId, DomainRecord, DomainRecordVersionRef, DomainRecordVersionSource, ErrorCode,
    EventId, EvidenceCursor, EvidenceDependency, EvidenceRef, EvidenceRequirement, IngressId,
    IngressPayload, IngressQueueKey, IngressReceipt, IngressRecord, KeyedDrawEntry, RandomDrawId,
    RandomDrawRecord, ScheduledAction, SimEvent, SimTime, Simulation, SimulationSnapshot,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Whether a boundary-generated plugin packet is still queued, was admitted,
/// or was withdrawn by a terminal cancellation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginIngressStatus {
    Pending,
    Admitted,
    Cancelled,
}

/// What the provenance reads and cancellation need to know about a sealed
/// ingress record, kept while the record is pending or a current record
/// declares it.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginIngressProvenanceFact {
    pub ingress: IngressId,
    /// The boundary generation of the packet, or `None` when no boundary
    /// generated it (host-enqueued or command-caused plugin ingress, or not
    /// plugin ingress at all), for which every provenance read answers
    /// `false`. Omitted from JSON when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<BoundaryIngressProvenance>,
}

/// The boundary generation of a plugin packet.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BoundaryIngressProvenance {
    /// The plugin whose boundary system scheduled the packet.
    pub issuer: String,
    /// The plugin the packet is addressed to.
    pub target_plugin: String,
    pub packet_type: String,
    pub producer_boundary: BoundaryId,
    pub status: PluginIngressStatus,
}

/// Evidence the seals moved below the archived cut that the run still needs.
///
/// Pending ingress, the causes of pending work, and the payloads that current
/// records declare `PayloadRequired` keep their records; superseded record
/// versions keep their bodies or, for identity-only declarations, their
/// membership; boundary-generated plugin packets keep their issuer and
/// status. Each seal recomputes the set from current state, and a boundary
/// that replaces a sealed record version pins the version it replaces. The
/// maps are persistent, so a fork or a transaction checkpoint copies them in
/// constant time; records are shared, which keeps each tree node small.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct PinnedEvidence {
    pub(super) events: im::OrdMap<EventId, Arc<SimEvent>>,
    pub(super) commands: im::OrdMap<CommandId, Arc<CommandRecord>>,
    pub(super) command_attempts: im::OrdMap<CommandAttemptId, Arc<CommandAttemptRecord>>,
    pub(super) ingress: im::OrdMap<IngressId, Arc<IngressRecord>>,
    pub(super) boundaries: im::OrdMap<BoundaryId, Arc<BoundaryRecord>>,
    pub(super) random_draws: im::OrdMap<RandomDrawId, Arc<RandomDrawRecord>>,
    /// Bodies of sealed record versions that are not current.
    pub(super) record_versions: im::OrdMap<DomainRecordVersionRef, Arc<DomainRecord>>,
    /// Sealed record versions that are not current and are known to exist.
    pub(super) known_record_versions: im::OrdSet<DomainRecordVersionRef>,
    /// Issuer and status of sealed boundary-generated plugin packets.
    pub(super) ingress_provenance: im::OrdMap<IngressId, PluginIngressProvenanceFact>,
}

impl PinnedEvidence {
    /// Marks a kept boundary-generated packet as admitted or cancelled.
    pub(super) fn set_ingress_status(&mut self, id: IngressId, status: PluginIngressStatus) {
        if let Some(fact) = self.ingress_provenance.get(&id)
            && let Some(generation) = &fact.generation
            && generation.status != status
        {
            let mut fact = fact.clone();
            if let Some(generation) = fact.generation.as_mut() {
                generation.status = status;
            }
            self.ingress_provenance.insert(id, fact);
        }
    }

    /// The kept provenance fact of a sealed ingress record.
    pub(super) fn fact(&self, id: IngressId) -> Option<&PluginIngressProvenanceFact> {
        self.ingress_provenance.get(&id)
    }

    /// The boundary generation of a sealed packet whose fact is kept.
    pub(super) fn generation(&self, id: IngressId) -> Option<&BoundaryIngressProvenance> {
        self.fact(id).and_then(|fact| fact.generation.as_ref())
    }
}

/// What the continuation closure needs at a cut (Part B of the design).
#[derive(Debug, Default)]
pub(super) struct ContinuationNeeds {
    /// Ordinal records whose payload the run still reads: pending ingress,
    /// the causes of pending work, and payload-required declarations.
    pub(super) records: BTreeSet<EvidenceRef>,
    /// Bodies of non-current, non-initial record versions declared
    /// payload-required.
    pub(super) version_bodies: BTreeSet<DomainRecordVersionRef>,
    /// Non-current, non-initial record versions declared by identity or
    /// payload.
    pub(super) memberships: BTreeSet<DomainRecordVersionRef>,
    /// Ingress records whose provenance fact the closure keeps: every
    /// pending record and every declared one.
    pub(super) provenance: BTreeSet<IngressId>,
}

fn not_ready(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::ArchiveNotReady, message)
}

impl Simulation {
    fn version_is_current_or_initial(&self, version: &DomainRecordVersionRef) -> bool {
        matches!(
            version.established_by,
            DomainRecordVersionSource::InitialScenario
        ) || super::live_current_domain_record_version(&self.state, version).is_some()
    }

    /// Computes the continuation closure of the current state.
    pub(super) fn continuation_needs(
        &self,
        dependencies: &[EvidenceDependency],
    ) -> Result<ContinuationNeeds, CanwuError> {
        let mut needs = ContinuationNeeds::default();
        let add_cause = |needs: &mut ContinuationNeeds, cause: &CauseRef| match cause {
            CauseRef::Command(id) => {
                needs.records.insert(EvidenceRef::Command(*id));
            }
            CauseRef::Event(id) => {
                needs.records.insert(EvidenceRef::Event(*id));
            }
            CauseRef::Boundary(_) | CauseRef::System(_) => {}
        };

        // K1 and K2: pending ingress, its provenance, and the causes of its
        // admission. Every pending record keeps its provenance fact, so that
        // every pinned ingress record has one: cancellation reads the issuer
        // of a boundary-generated packet, and a packet declared after its
        // seal, while its record is still pinned, answers the provenance
        // reads as it does unsealed.
        for key in &self.state.scheduler.pending_ingress {
            needs.records.insert(EvidenceRef::Ingress(key.id));
            needs.provenance.insert(key.id);
            let record =
                self.state.evidence.ingress_record(key.id).ok_or_else(|| {
                    not_ready(format!("pending ingress {} is unavailable", key.id))
                })?;
            if let Some(cause) = &record.cause {
                add_cause(&mut needs, cause);
            }
        }
        // K2: the causes of scheduled work.
        for action in self.state.scheduler.actions.values() {
            match action {
                ScheduledAction::ArmyArrival { order_event, .. }
                | ScheduledAction::PersonArrival { order_event, .. } => {
                    needs.records.insert(EvidenceRef::Event(*order_event));
                }
                ScheduledAction::KnowledgeReport { dispatch_event, .. } => {
                    needs.records.insert(EvidenceRef::Event(*dispatch_event));
                }
                ScheduledAction::PluginDirective { cause, .. } => add_cause(&mut needs, cause),
            }
        }
        // K3 and F: declared dependencies, each of which must exist.
        let context = super::validation::RuntimeValidationContext::new(&self.state);
        for dependency in dependencies {
            if super::validation::resolve_evidence_reference(&context, &dependency.reference)
                == super::validation::EvidenceAvailability::Missing
            {
                return Err(not_ready(format!(
                    "declared evidence {:?} does not exist at this cut",
                    dependency.reference
                )));
            }
            match (&dependency.reference, dependency.requirement) {
                (EvidenceRef::DomainRecordVersion(version), requirement) => {
                    if self.version_is_current_or_initial(version) {
                        continue;
                    }
                    needs.memberships.insert(version.clone());
                    if requirement == EvidenceRequirement::PayloadRequired {
                        needs.version_bodies.insert(version.clone());
                    }
                }
                (reference, EvidenceRequirement::PayloadRequired) => {
                    needs.records.insert(reference.clone());
                    if let EvidenceRef::Ingress(id) = reference {
                        needs.provenance.insert(*id);
                    }
                }
                // Every declared ingress keeps what the provenance reads
                // need, whether or not a boundary generated it, so that they
                // answer after a seal as they do before it.
                (EvidenceRef::Ingress(id), EvidenceRequirement::IdentityOnly) => {
                    needs.provenance.insert(*id);
                }
                (_, EvidenceRequirement::IdentityOnly) => {}
            }
        }
        Ok(needs)
    }

    /// The status a provenance fact made now records for a packet.
    pub(super) fn plugin_ingress_status(&self, record: &IngressRecord) -> PluginIngressStatus {
        let scheduler = &self.state.scheduler;
        if scheduler
            .pending_ingress
            .contains(&IngressQueueKey::from_record(record))
        {
            PluginIngressStatus::Pending
        } else if scheduler.cancelled_ingress.contains(&record.id) {
            PluginIngressStatus::Cancelled
        } else {
            PluginIngressStatus::Admitted
        }
    }

    /// Builds the provenance fact of an ingress record the runtime still
    /// holds, retained or pinned, from its producing boundary's generation
    /// evidence when a boundary generated it. `None` when the record, or the
    /// producing boundary of a boundary-generated packet, is no longer held.
    fn retained_provenance_fact(
        &self,
        id: IngressId,
    ) -> Result<Option<PluginIngressProvenanceFact>, CanwuError> {
        let Some(record) = self.state.evidence.ingress_record(id) else {
            return Ok(None);
        };
        let (
            IngressPayload::Plugin {
                plugin,
                packet_type,
                ..
            },
            Some(CauseRef::Boundary(producer)),
        ) = (&record.payload, record.cause.as_ref())
        else {
            return Ok(Some(PluginIngressProvenanceFact {
                ingress: id,
                generation: None,
            }));
        };
        let Some(boundary) = self.state.evidence.boundary_record(*producer) else {
            return Ok(None);
        };
        // The generation names the plugin that scheduled the packet, which
        // may differ from the plugin it is addressed to.
        let generation = boundary
            .generated_ingress
            .iter()
            .find(|generation| generation.ingress == id)
            .ok_or_else(|| {
                not_ready(format!(
                    "boundary-generated ingress {id} lacks its generation evidence"
                ))
            })?;
        Ok(Some(PluginIngressProvenanceFact {
            ingress: id,
            generation: Some(BoundaryIngressProvenance {
                issuer: generation.plugin.clone(),
                target_plugin: plugin.clone(),
                packet_type: packet_type.clone(),
                producer_boundary: *producer,
                status: self.plugin_ingress_status(record),
            }),
        }))
    }

    /// Resolves the continuation closure below `cut` from retained evidence,
    /// the pins, and the kept facts. Every item the closure needs at or below
    /// the cut must be readable; an item above it stays retained.
    pub(super) fn closure_pins(
        &self,
        needs: &ContinuationNeeds,
        cut: &EvidenceCursor,
    ) -> Result<PinnedEvidence, CanwuError> {
        let evidence = &self.state.evidence;
        let mut pins = PinnedEvidence::default();
        let below = |id: u64, count: u64| id != 0 && id <= count;
        let missing = |reference: &EvidenceRef| {
            not_ready(format!(
                "continuation evidence {reference:?} is no longer readable; a dependency was declared after its evidence was sealed"
            ))
        };
        for reference in &needs.records {
            match reference {
                EvidenceRef::Event(id) if below(id.get(), cut.event_count) => {
                    let record = evidence
                        .event_record(*id)
                        .ok_or_else(|| missing(reference))?;
                    pins.events.insert(*id, Arc::new(record.clone()));
                }
                EvidenceRef::Command(id) if below(id.get(), cut.command_count) => {
                    let record = evidence
                        .command_record(*id)
                        .ok_or_else(|| missing(reference))?;
                    pins.commands.insert(*id, Arc::new(record.clone()));
                }
                EvidenceRef::CommandAttempt(id) if below(id.get(), cut.command_attempt_count) => {
                    let record = evidence
                        .command_attempt_record(*id)
                        .ok_or_else(|| missing(reference))?;
                    pins.command_attempts.insert(*id, Arc::new(record.clone()));
                }
                EvidenceRef::Ingress(id) if below(id.get(), cut.ingress_count) => {
                    let record = evidence
                        .ingress_record(*id)
                        .ok_or_else(|| missing(reference))?;
                    pins.ingress.insert(*id, Arc::new(record.clone()));
                }
                EvidenceRef::Boundary(id) if below(id.get(), cut.boundary_count) => {
                    let record = evidence
                        .boundary_record(*id)
                        .ok_or_else(|| missing(reference))?;
                    pins.boundaries.insert(*id, Arc::new(record.clone()));
                }
                EvidenceRef::RandomDraw(id) if below(id.get(), cut.random_draw_count) => {
                    let record = evidence
                        .random_draw_record(*id)
                        .ok_or_else(|| missing(reference))?;
                    pins.random_draws.insert(*id, Arc::new(record.clone()));
                }
                // Above the cut the record stays retained; one that is not
                // (an ID beyond the journal end) is refused.
                _ if !self.payload_is_held(reference) => return Err(missing(reference)),
                _ => {}
            }
        }
        for version in &needs.version_bodies {
            let body = super::retained_domain_record_version_body(&self.state, version)
                .ok_or_else(|| missing(&EvidenceRef::DomainRecordVersion(version.clone())))?;
            pins.record_versions
                .insert(version.clone(), Arc::new(body.clone()));
        }
        for version in &needs.memberships {
            if needs.version_bodies.contains(version) {
                continue;
            }
            if !super::validation::sealed_version_is_known(&self.state, version) {
                return Err(missing(&EvidenceRef::DomainRecordVersion(version.clone())));
            }
            pins.known_record_versions.insert(version.clone());
        }
        for id in &needs.provenance {
            if !below(id.get(), cut.ingress_count) {
                continue;
            }
            let fact = match evidence.pinned.ingress_provenance.get(id) {
                Some(fact) => fact.clone(),
                None => self
                    .retained_provenance_fact(*id)?
                    .ok_or_else(|| missing(&EvidenceRef::Ingress(*id)))?,
            };
            pins.ingress_provenance.insert(*id, fact);
        }
        Ok(pins)
    }
}

/// Version of the continuation checkpoint. Version 2 embeds snapshot format
/// 9.
pub const CONTINUATION_CHECKPOINT_FORMAT_VERSION: u32 = 2;
/// Hash domain of the continuation checkpoint's own identity.
const CONTINUATION_CHECKPOINT_DOMAIN: &str = "canwu.continuation-checkpoint.v1";

/// The resumable commitment stream of one append-only journal.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct JournalFrontier {
    /// Items fed to the stream.
    pub items: u64,
    /// Bytes fed to the stream.
    pub length: u64,
    /// Chaining values of the completed subtrees, largest first, as
    /// lowercase hexadecimal.
    pub chaining_values: Vec<String>,
    /// The bytes of the unfinished chunk, as lowercase hexadecimal.
    pub tail: String,
}

/// The resumable command, attempt, event, ingress, and draw streams.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct JournalFrontiers {
    pub commands: JournalFrontier,
    pub attempts: JournalFrontier,
    pub events: JournalFrontier,
    pub ingress: JournalFrontier,
    pub random_draws: JournalFrontier,
}

/// Events, commands, and attempts above the settled cut, which the next
/// boundary admits.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct UnadmittedEvidence {
    pub events: Vec<SimEvent>,
    pub commands: Vec<CommandRecord>,
    pub command_attempts: Vec<CommandAttemptRecord>,
}

/// The body of a record version that is no longer current.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PinnedRecordVersion {
    pub version: DomainRecordVersionRef,
    pub body: DomainRecord,
}

/// The continuation closure below the settled cut: the records pending work
/// and payload-required declarations read, and the bodies of declared
/// record versions that are no longer current. Each list is in ID or
/// reference order.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct PinnedEvidenceRecords {
    pub events: Vec<SimEvent>,
    pub commands: Vec<CommandRecord>,
    pub command_attempts: Vec<CommandAttemptRecord>,
    pub ingress: Vec<IngressRecord>,
    pub boundaries: Vec<BoundaryRecord>,
    pub random_draws: Vec<RandomDrawRecord>,
    pub record_versions: Vec<PinnedRecordVersion>,
}

/// A live record's current version and when it became authoritative.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CurrentRecordVersion {
    pub version: DomainRecordVersionRef,
    pub established_at: SimTime,
}

/// The exact outcome of a tracked command request at or below the cut.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SettledCommandRequest {
    pub request_id: CommandRequestId,
    pub input_hash: String,
    pub outcome: CommandOutcome,
}

/// The exact ingress receipt of a queued command request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SettledQueuedCommand {
    pub request_id: CommandRequestId,
    pub input_hash: String,
    pub receipt: IngressReceipt,
}

/// The exact ingress receipt of a decision request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SettledDecisionRequest {
    pub request_id: DecisionRequestId,
    pub input_hash: String,
    pub receipt: IngressReceipt,
}

/// Exact idempotency for every tracked request at or below the settled cut.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct RequestIndex {
    pub commands: Vec<SettledCommandRequest>,
    pub queued_commands: Vec<SettledQueuedCommand>,
    pub decisions: Vec<SettledDecisionRequest>,
    pub decision_commands: Vec<CommandRequestId>,
}

/// Facts about the history below the settled cut that later admission
/// rules read.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct HistoryFlags {
    pub legacy_commands: bool,
    pub tracked_attempts: bool,
    pub unqueued_command_history: bool,
}

/// One run of equal times along a journal: every ID from `first` up to the
/// next run's first ID became authoritative at `at`.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EvidenceTimeRun {
    pub first: u64,
    pub at: SimTime,
}

/// When each ordinal record at or below the settled cut became
/// authoritative, run-length encoded per journal.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct EvidenceTimes {
    pub events: Vec<EvidenceTimeRun>,
    pub commands: Vec<EvidenceTimeRun>,
    pub command_attempts: Vec<EvidenceTimeRun>,
    pub ingress: Vec<EvidenceTimeRun>,
    pub boundaries: Vec<EvidenceTimeRun>,
    pub random_draws: Vec<EvidenceTimeRun>,
}

/// Current state with the continuation closure and the indexes a run keeps
/// instead of its history (format 1).
///
/// A run restored from it continues exactly as the uninterrupted run, with
/// the same receipts, checkpoint hashes, and state hashes at every later
/// boundary, but cannot produce a full snapshot, replay journal, or compact
/// checkpoint. It holds ground truth and is never given to players, agents,
/// observers, or remote clients. Every part is a function of state and
/// history at the cut, so an unsealed run, any sealed run, and any
/// continuation-restored run write the same checkpoint there.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ContinuationCheckpoint {
    pub format_version: u32,
    /// Current state with every journal array empty, as in
    /// [`super::SimulationCheckpoint::state`]; it carries the checkpoint hash
    /// and the commitment roots.
    pub state: SimulationSnapshot,
    pub journal_end: EvidenceCursor,
    /// Events, commands, and attempts up to the admission cursors; ingress,
    /// boundaries, and draws to the journal end.
    pub settled_cut: EvidenceCursor,
    /// Hash of the last boundary record.
    pub boundary_head: Option<String>,
    pub journal_frontiers: JournalFrontiers,
    pub unadmitted: UnadmittedEvidence,
    pub pinned: PinnedEvidenceRecords,
    /// The pending ingress among the pinned records, in ID order.
    pub pending_ingress: Vec<IngressId>,
    /// The provenance fact of every pending or declared ingress record at or
    /// below the cut, in ID order.
    pub ingress_provenance: Vec<PluginIngressProvenanceFact>,
    /// Declared record versions that are no longer current and whose body
    /// the closure does not keep, in reference order.
    pub known_record_versions: Vec<DomainRecordVersionRef>,
    /// The provenance of every live record's current version, in record
    /// order.
    pub current_record_versions: Vec<CurrentRecordVersion>,
    /// Every operation-keyed draw the run has made, in draw-ID order.
    pub keyed_draws: Vec<KeyedDrawEntry>,
    pub requests: RequestIndex,
    pub history_flags: HistoryFlags,
    pub evidence_times: EvidenceTimes,
    /// Settled boundaries within the run's boundary-schedule horizon.
    pub settled_boundaries: Vec<BoundarySummary>,
    /// Domain-separated hash over every field above. It detects corruption;
    /// the value a host stored when it wrote the checkpoint is the trust
    /// anchor (see [`super::CompactedSimulation::from_continuation_checkpoint_verified`]).
    pub continuation_hash: String,
}

/// Where a continuation-restored run's history begins.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ContinuationBase {
    /// The settled cut of the checkpoint the run was restored from.
    pub cut: EvidenceCursor,
    pub checkpoint_hash: String,
    pub continuation_hash: String,
}

#[derive(Serialize)]
struct ContinuationMaterial<'a> {
    format_version: u32,
    state: &'a SimulationSnapshot,
    journal_end: &'a EvidenceCursor,
    settled_cut: &'a EvidenceCursor,
    boundary_head: &'a Option<String>,
    journal_frontiers: &'a JournalFrontiers,
    unadmitted: &'a UnadmittedEvidence,
    pinned: &'a PinnedEvidenceRecords,
    pending_ingress: &'a [IngressId],
    ingress_provenance: &'a [PluginIngressProvenanceFact],
    known_record_versions: &'a [DomainRecordVersionRef],
    current_record_versions: &'a [CurrentRecordVersion],
    keyed_draws: &'a [KeyedDrawEntry],
    requests: &'a RequestIndex,
    history_flags: &'a HistoryFlags,
    evidence_times: &'a EvidenceTimes,
    settled_boundaries: &'a [BoundarySummary],
}

impl ContinuationCheckpoint {
    /// Recomputes the continuation hash over every field but itself.
    pub fn compute_continuation_hash(&self) -> Result<String, CanwuError> {
        super::canonical_hash(
            CONTINUATION_CHECKPOINT_DOMAIN,
            &ContinuationMaterial {
                format_version: self.format_version,
                state: &self.state,
                journal_end: &self.journal_end,
                settled_cut: &self.settled_cut,
                boundary_head: &self.boundary_head,
                journal_frontiers: &self.journal_frontiers,
                unadmitted: &self.unadmitted,
                pinned: &self.pinned,
                pending_ingress: &self.pending_ingress,
                ingress_provenance: &self.ingress_provenance,
                known_record_versions: &self.known_record_versions,
                current_record_versions: &self.current_record_versions,
                keyed_draws: &self.keyed_draws,
                requests: &self.requests,
                history_flags: &self.history_flags,
                evidence_times: &self.evidence_times,
                settled_boundaries: &self.settled_boundaries,
            },
        )
    }

    /// Serializes the checkpoint as JSON.
    pub fn to_json(&self) -> Result<String, CanwuError> {
        serde_json::to_string(self).map_err(|error| {
            CanwuError::new(
                ErrorCode::InvalidSnapshot,
                format!("could not serialize continuation checkpoint: {error}"),
            )
        })
    }

    /// Decodes a checkpoint from JSON, refusing unknown fields and
    /// non-canonical encodings.
    pub fn from_json(json: &str) -> Result<Self, CanwuError> {
        super::deserialize_current_json(json, "continuation checkpoint")
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut text, byte| {
            let _ = write!(text, "{byte:02x}");
            text
        })
}

fn parse_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2)
        || !text
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        return None;
    }
    (0..text.len() / 2)
        .map(|index| u8::from_str_radix(text.get(index * 2..index * 2 + 2)?, 16).ok())
        .collect()
}

fn export_frontier(accumulator: &super::state::OrderedCommitmentAccumulator) -> JournalFrontier {
    let (items, frontier) = accumulator.frontier();
    JournalFrontier {
        items: items as u64,
        length: frontier.len,
        chaining_values: frontier
            .chaining_values
            .iter()
            .map(|value| hex(value))
            .collect(),
        tail: hex(&frontier.tail),
    }
}

fn import_frontier(
    frontier: &JournalFrontier,
) -> Result<super::state::OrderedCommitmentAccumulator, CanwuError> {
    let invalid = || {
        CanwuError::new(
            ErrorCode::InvalidSnapshot,
            "journal frontier encoding is not canonical",
        )
    };
    let chaining_values = frontier
        .chaining_values
        .iter()
        .map(|value| {
            parse_hex(value)
                .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
                .ok_or_else(invalid)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let tail = parse_hex(&frontier.tail).ok_or_else(invalid)?;
    let items = usize::try_from(frontier.items).map_err(|_| invalid())?;
    super::state::OrderedCommitmentAccumulator::from_frontier(
        items,
        &super::stream_hash::StreamFrontier {
            len: frontier.length,
            chaining_values,
            tail,
        },
    )
}

fn time_runs(runs: &im::Vector<(u64, SimTime)>) -> Vec<EvidenceTimeRun> {
    runs.iter()
        .map(|(first, at)| EvidenceTimeRun {
            first: *first,
            at: *at,
        })
        .collect()
}

impl Simulation {
    /// Builds the continuation checkpoint of this run at its settled cut,
    /// with its continuation hash, which a host stores as the trust anchor
    /// for a verified restore.
    ///
    /// Building changes nothing. It fails with `ArchiveNotReady` exactly
    /// when a seal would: a declared dependency names evidence that no
    /// longer exists or can no longer be read.
    pub fn continuation_checkpoint(&self) -> Result<(ContinuationCheckpoint, String), CanwuError> {
        self.ensure_runtime_ready()?;
        let cut = self.settled_cut()?;
        let journal_end = self.evidence_cursor()?;
        let dependencies = self.evidence_dependencies()?;
        let needs = self.continuation_needs(&dependencies)?;
        let pins = self.closure_pins(&needs, &cut)?;
        let evidence = &self.state.evidence;
        let below = |count: u64, archived: u64| {
            usize::try_from(count - archived).map_err(|_| {
                CanwuError::new(
                    ErrorCode::InvalidSnapshot,
                    "settled journal slice exceeds platform range",
                )
            })
        };
        let events = below(cut.event_count, evidence.archived.event_count)?;
        let commands = below(cut.command_count, evidence.archived.command_count)?;
        let attempts = below(
            cut.command_attempt_count,
            evidence.archived.command_attempt_count,
        )?;
        let settled_events = &evidence.events[..events];
        let settled_commands = &evidence.commands[..commands];
        let settled_attempts = &evidence.command_attempts[..attempts];

        let tables = self.archived_request_tables(settled_attempts, &evidence.ingress)?;
        let mut command_requests: BTreeMap<_, _> = evidence
            .archived_command_requests
            .iter()
            .map(|(id, outcome)| (*id, (outcome.input_hash.clone(), outcome.outcome.clone())))
            .collect();
        command_requests.extend(
            tables
                .commands
                .into_iter()
                .map(|(id, outcome)| (id, (outcome.input_hash, outcome.outcome))),
        );
        let mut queued_commands: BTreeMap<_, _> = evidence
            .archived_ingress_requests
            .iter()
            .map(|(id, request)| (*id, request.clone()))
            .collect();
        queued_commands.extend(tables.ingress);
        let mut decisions: BTreeMap<_, _> = evidence
            .archived_decision_requests
            .iter()
            .map(|(id, request)| (*id, request.clone()))
            .collect();
        decisions.extend(tables.decisions);
        let mut decision_commands: BTreeSet<_> = evidence
            .archived_decision_command_requests
            .iter()
            .copied()
            .collect();
        decision_commands.extend(tables.decision_commands);
        let requests = RequestIndex {
            commands: command_requests
                .into_iter()
                .map(
                    |(request_id, (input_hash, outcome))| SettledCommandRequest {
                        request_id,
                        input_hash,
                        outcome,
                    },
                )
                .collect(),
            queued_commands: queued_commands
                .into_iter()
                .map(|(request_id, request)| SettledQueuedCommand {
                    request_id,
                    input_hash: request.input_hash,
                    receipt: request.receipt,
                })
                .collect(),
            decisions: decisions
                .into_iter()
                .map(|(request_id, request)| SettledDecisionRequest {
                    request_id,
                    input_hash: request.input_hash,
                    receipt: request.receipt,
                })
                .collect(),
            decision_commands: decision_commands.into_iter().collect(),
        };
        let (legacy, tracked, unqueued) =
            self.history_flags_of(settled_commands, settled_attempts, &evidence.ingress);
        let history_flags = HistoryFlags {
            legacy_commands: evidence.archived_legacy_commands || legacy,
            tracked_attempts: evidence.archived_tracked_attempts || tracked,
            unqueued_command_history: evidence.archived_unqueued_command_history || unqueued,
        };
        let mut times = evidence.sealed_times.clone();
        times.extend_from_slices(
            settled_events,
            settled_commands,
            settled_attempts,
            &evidence.ingress,
            &evidence.boundaries,
            &evidence.random_draws,
        );
        let [
            time_events,
            time_commands,
            time_attempts,
            time_ingress,
            time_boundaries,
            time_draws,
        ] = times.runs();
        let evidence_times = EvidenceTimes {
            events: time_runs(time_events),
            commands: time_runs(time_commands),
            command_attempts: time_runs(time_attempts),
            ingress: time_runs(time_ingress),
            boundaries: time_runs(time_boundaries),
            random_draws: time_runs(time_draws),
        };

        let cache = self
            .state
            .metadata
            .commitment_cache
            .as_ref()
            .ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::InvalidSnapshot,
                    "the journal commitment cache is unavailable",
                )
            })?;
        let journal_counts = [
            journal_end.command_count,
            journal_end.command_attempt_count,
            journal_end.event_count,
            journal_end.ingress_count,
            journal_end.random_draw_count,
        ];
        let journals = cache.journals();
        if journals
            .iter()
            .zip(journal_counts)
            .any(|(journal, count)| u64::try_from(journal.len).ok() != Some(count))
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidSnapshot,
                "the journal commitment cache is not current",
            ));
        }
        let [
            commands_frontier,
            attempts_frontier,
            events_frontier,
            ingress_frontier,
            draws_frontier,
        ] = journals.map(export_frontier);
        let journal_frontiers = JournalFrontiers {
            commands: commands_frontier,
            attempts: attempts_frontier,
            events: events_frontier,
            ingress: ingress_frontier,
            random_draws: draws_frontier,
        };

        let pinned = PinnedEvidenceRecords {
            events: pins
                .events
                .values()
                .map(|record| (**record).clone())
                .collect(),
            commands: pins
                .commands
                .values()
                .map(|record| (**record).clone())
                .collect(),
            command_attempts: pins
                .command_attempts
                .values()
                .map(|record| (**record).clone())
                .collect(),
            ingress: pins
                .ingress
                .values()
                .map(|record| (**record).clone())
                .collect(),
            boundaries: pins
                .boundaries
                .values()
                .map(|record| (**record).clone())
                .collect(),
            random_draws: pins
                .random_draws
                .values()
                .map(|record| (**record).clone())
                .collect(),
            record_versions: pins
                .record_versions
                .iter()
                .map(|(version, body)| PinnedRecordVersion {
                    version: version.clone(),
                    body: (**body).clone(),
                })
                .collect(),
        };
        let mut pending_ingress: Vec<_> = self
            .state
            .scheduler
            .pending_ingress
            .iter()
            .map(|key| key.id)
            .collect();
        pending_ingress.sort_unstable();

        let mut checkpoint = ContinuationCheckpoint {
            format_version: CONTINUATION_CHECKPOINT_FORMAT_VERSION,
            state: self.checkpoint_state(),
            journal_end,
            settled_cut: cut,
            boundary_head: evidence.boundary_head_hash().map(str::to_owned),
            journal_frontiers,
            unadmitted: UnadmittedEvidence {
                events: evidence.events[events..].to_vec(),
                commands: evidence.commands[commands..].to_vec(),
                command_attempts: evidence.command_attempts[attempts..].to_vec(),
            },
            pinned,
            pending_ingress,
            ingress_provenance: pins.ingress_provenance.values().cloned().collect(),
            known_record_versions: pins.known_record_versions.iter().cloned().collect(),
            current_record_versions: self
                .state
                .metadata
                .current_domain_record_versions
                .values()
                .map(|current| CurrentRecordVersion {
                    version: current.version.clone(),
                    established_at: current.established_at,
                })
                .collect(),
            keyed_draws: evidence.keyed_draws.entries(),
            requests,
            history_flags,
            evidence_times,
            settled_boundaries: self
                .state
                .scheduler
                .settled_boundaries
                .iter()
                .map(|summary| summary.as_ref().clone())
                .collect(),
            continuation_hash: String::new(),
        };
        let hash = checkpoint.compute_continuation_hash()?;
        checkpoint.continuation_hash.clone_from(&hash);
        Ok((checkpoint, hash))
    }
}

fn invalid(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidSnapshot, message)
}

fn strictly_increasing<T, K: Ord>(items: &[T], key: impl Fn(&T) -> K) -> bool {
    items.windows(2).all(|pair| key(&pair[0]) < key(&pair[1]))
}

/// Checks that the records of one journal are in ID order and lie in
/// `(low, high]`.
fn ids_within<T>(items: &[T], id: impl Fn(&T) -> u64, low: u64, high: u64) -> bool {
    strictly_increasing(items, &id) && items.iter().all(|item| id(item) > low && id(item) <= high)
}

/// Checks that the records of one journal are exactly the IDs `(low, high]`.
fn ids_exactly<T>(items: &[T], id: impl Fn(&T) -> u64, low: u64, high: u64) -> bool {
    u64::try_from(items.len()).ok() == high.checked_sub(low)
        && items
            .iter()
            .zip(low + 1..)
            .all(|(item, expected)| id(item) == expected)
}

fn canonical_time_runs(runs: &[EvidenceTimeRun], count: u64, now: SimTime) -> bool {
    match runs.first() {
        None => count == 0,
        Some(first) => {
            first.first == 1
                && runs
                    .windows(2)
                    .all(|pair| pair[0].first < pair[1].first && pair[0].at != pair[1].at)
                && runs.iter().all(|run| run.first <= count && run.at <= now)
        }
    }
}

fn import_time_runs(runs: &[EvidenceTimeRun]) -> Vec<(u64, SimTime)> {
    runs.iter().map(|run| (run.first, run.at)).collect()
}

fn check_journal_cut(
    cursor: &EvidenceCursor,
    counters: [u64; 6],
    label: &str,
) -> Result<(), CanwuError> {
    let values = [
        cursor.event_count,
        cursor.command_count,
        cursor.command_attempt_count,
        cursor.ingress_count,
        cursor.boundary_count,
        cursor.random_draw_count,
    ];
    if values != counters {
        return Err(invalid(format!(
            "continuation checkpoint {label} does not match the run's counters"
        )));
    }
    Ok(())
}

impl ContinuationCheckpoint {
    /// Steps 1 to 3 of a restore: versions, integrity, and the shape and
    /// bounds of every part, before anything is built from it.
    fn check_shape(&self) -> Result<(), CanwuError> {
        let state = &self.state;
        if self.format_version != CONTINUATION_CHECKPOINT_FORMAT_VERSION
            || state.snapshot_format_version != super::SNAPSHOT_FORMAT_VERSION
            || state.engine_version != super::ENGINE_VERSION
        {
            return Err(CanwuError::new(
                ErrorCode::UnsupportedSnapshotVersion,
                format!(
                    "continuation checkpoints are read only by engine {} format {CONTINUATION_CHECKPOINT_FORMAT_VERSION} with snapshot format {}",
                    super::ENGINE_VERSION,
                    super::SNAPSHOT_FORMAT_VERSION
                ),
            ));
        }
        if state.commitment_format_version != super::COMMITMENT_FORMAT_VERSION
            || state.commitment_roots.is_none()
        {
            return Err(invalid(
                "continuation checkpoints require the current commitment format",
            ));
        }
        if !state.events.is_empty()
            || !state.commands.is_empty()
            || !state.command_attempts.is_empty()
            || !state.ingress.is_empty()
            || !state.boundaries.is_empty()
            || !state.random_draws.is_empty()
        {
            return Err(invalid(
                "continuation checkpoint state must not carry journal records",
            ));
        }
        if self.compute_continuation_hash()? != self.continuation_hash {
            return Err(invalid(
                "continuation hash does not match the checkpoint's material",
            ));
        }

        let last = |next: u64| {
            next.checked_sub(1)
                .ok_or_else(|| invalid("continuation checkpoint counters must be nonzero"))
        };
        let end = self.journal_end;
        check_journal_cut(
            &end,
            [
                last(state.next_event_id)?,
                last(state.next_command_id)?,
                last(state.next_command_attempt_id)?,
                last(state.next_ingress_id)?,
                last(state.next_boundary_id)?,
                last(state.next_random_draw_id)?,
            ],
            "journal end",
        )?;
        let cut = self.settled_cut;
        check_journal_cut(
            &cut,
            [
                state.admitted_event_count,
                state.admitted_command_count,
                state.admitted_attempt_count,
                end.ingress_count,
                end.boundary_count,
                end.random_draw_count,
            ],
            "settled cut",
        )?;
        if cut.event_count > end.event_count
            || cut.command_count > end.command_count
            || cut.command_attempt_count > end.command_attempt_count
        {
            return Err(invalid(
                "continuation settled cut lies beyond the journal end",
            ));
        }

        let unadmitted = &self.unadmitted;
        if !ids_exactly(
            &unadmitted.events,
            |record| record.id.get(),
            cut.event_count,
            end.event_count,
        ) || !ids_exactly(
            &unadmitted.commands,
            |record| record.id.get(),
            cut.command_count,
            end.command_count,
        ) || !ids_exactly(
            &unadmitted.command_attempts,
            |record| record.id.get(),
            cut.command_attempt_count,
            end.command_attempt_count,
        ) {
            return Err(invalid(
                "continuation unadmitted evidence must be exactly the records above the settled cut",
            ));
        }

        let pinned = &self.pinned;
        if !ids_within(&pinned.events, |record| record.id.get(), 0, cut.event_count)
            || !ids_within(
                &pinned.commands,
                |record| record.id.get(),
                0,
                cut.command_count,
            )
            || !ids_within(
                &pinned.command_attempts,
                |record| record.id.get(),
                0,
                cut.command_attempt_count,
            )
            || !ids_within(
                &pinned.ingress,
                |record| record.id.get(),
                0,
                cut.ingress_count,
            )
            || !ids_within(
                &pinned.boundaries,
                |record| record.id.get(),
                0,
                cut.boundary_count,
            )
            || !ids_within(
                &pinned.random_draws,
                |record| record.id.get(),
                0,
                cut.random_draw_count,
            )
        {
            return Err(invalid(
                "continuation pinned records must be in ID order at or below the settled cut",
            ));
        }
        let sealed_version = |version: &DomainRecordVersionRef| match version.established_by {
            DomainRecordVersionSource::BoundaryChange { boundary, .. } => {
                version.version != 0 && boundary.get() != 0 && boundary.get() <= cut.boundary_count
            }
            DomainRecordVersionSource::InitialScenario => false,
        };
        if !strictly_increasing(&pinned.record_versions, |pin| pin.version.clone())
            || pinned.record_versions.iter().any(|pin| {
                !sealed_version(&pin.version)
                    || pin.body.reference != pin.version.record
                    || pin.body.version != pin.version.version
            })
            || !strictly_increasing(&self.known_record_versions, Clone::clone)
            || self
                .known_record_versions
                .iter()
                .any(|version| !sealed_version(version))
        {
            return Err(invalid(
                "continuation record versions must be sorted sealed boundary versions",
            ));
        }
        if !strictly_increasing(&self.pending_ingress, |id| *id)
            || self.pending_ingress.iter().any(|id| {
                pinned
                    .ingress
                    .binary_search_by_key(id, |record| record.id)
                    .is_err()
            })
        {
            return Err(invalid(
                "continuation pending ingress must be sorted pinned records",
            ));
        }
        if !strictly_increasing(&self.ingress_provenance, |fact| fact.ingress)
            || self.ingress_provenance.iter().any(|fact| {
                fact.ingress.get() == 0
                    || fact.ingress.get() > cut.ingress_count
                    || fact.generation.as_ref().is_some_and(|generation| {
                        generation.producer_boundary.get() == 0
                            || generation.producer_boundary.get() > cut.boundary_count
                    })
            })
        {
            return Err(invalid(
                "continuation provenance facts must be sorted and lie at or below the cut",
            ));
        }
        if !strictly_increasing(&self.current_record_versions, |current| {
            current.version.record.clone()
        }) || self.current_record_versions.iter().any(|current| {
            current.established_at > state.now
                || matches!(
                    current.version.established_by,
                    DomainRecordVersionSource::BoundaryChange { boundary, .. }
                        if boundary.get() == 0 || boundary.get() > cut.boundary_count
                )
        }) {
            return Err(invalid(
                "continuation current record versions must be sorted and settled",
            ));
        }
        if self
            .keyed_draws
            .iter()
            .any(|entry| entry.draw_id.get() > cut.random_draw_count)
            || !strictly_increasing(&self.keyed_draws, |entry| entry.draw_id)
        {
            return Err(invalid(
                "continuation keyed draws must be in draw-ID order at or below the cut",
            ));
        }
        let requests = &self.requests;
        if !strictly_increasing(&requests.commands, |entry| entry.request_id)
            || !strictly_increasing(&requests.queued_commands, |entry| entry.request_id)
            || !strictly_increasing(&requests.decisions, |entry| entry.request_id)
            || !strictly_increasing(&requests.decision_commands, |id| *id)
            || requests
                .queued_commands
                .iter()
                .map(|entry| entry.receipt.ingress_id)
                .chain(
                    requests
                        .decisions
                        .iter()
                        .map(|entry| entry.receipt.ingress_id),
                )
                .any(|id| id.get() == 0 || id.get() > cut.ingress_count)
        {
            return Err(invalid(
                "continuation request tables must be sorted and settled",
            ));
        }
        let times = &self.evidence_times;
        if !canonical_time_runs(&times.events, cut.event_count, state.now)
            || !canonical_time_runs(&times.commands, cut.command_count, state.now)
            || !canonical_time_runs(
                &times.command_attempts,
                cut.command_attempt_count,
                state.now,
            )
            || !canonical_time_runs(&times.ingress, cut.ingress_count, state.now)
            || !canonical_time_runs(&times.boundaries, cut.boundary_count, state.now)
            || !canonical_time_runs(&times.random_draws, cut.random_draw_count, state.now)
        {
            return Err(invalid("continuation time index is not canonical"));
        }
        if self
            .settled_boundaries
            .windows(2)
            .any(|pair| pair[0].at > pair[1].at || pair[0].boundary_id >= pair[1].boundary_id)
            || self.settled_boundaries.iter().any(|summary| {
                summary
                    .boundary_id
                    .is_none_or(|id| id.get() == 0 || id.get() > cut.boundary_count)
                    || summary.at > state.now
            })
        {
            return Err(invalid(
                "continuation boundary ledger must hold settled boundaries in order",
            ));
        }
        let frontiers = [
            (&self.journal_frontiers.commands, end.command_count),
            (&self.journal_frontiers.attempts, end.command_attempt_count),
            (&self.journal_frontiers.events, end.event_count),
            (&self.journal_frontiers.ingress, end.ingress_count),
            (&self.journal_frontiers.random_draws, end.random_draw_count),
        ];
        if frontiers
            .iter()
            .any(|(frontier, count)| frontier.items != *count)
        {
            return Err(invalid(
                "continuation journal frontier item counts must equal the journal counts",
            ));
        }
        Ok(())
    }
}

impl Simulation {
    /// Refuses an operation that needs history before a continuation base.
    pub(super) fn ensure_full_history(&self, operation: &str) -> Result<(), CanwuError> {
        if self.state.evidence.continuation_base.is_some() {
            return Err(CanwuError::new(
                ErrorCode::HistoryUnavailable,
                format!(
                    "{operation} needs the run's history, which a continuation-restored run does not hold"
                ),
            ));
        }
        Ok(())
    }

    /// Steps 1 to 7 of a continuation restore: check the checkpoint's shape
    /// and integrity, validate its state without history, rebuild the keyed
    /// ledger and the journal streams, build the runtime, and require it to
    /// reproduce the checkpoint hash and commitment roots. The caller
    /// rehydrates plugins and then checks the closure round trip.
    pub(super) fn restore_continuation(
        checkpoint: ContinuationCheckpoint,
    ) -> Result<Self, CanwuError> {
        checkpoint.check_shape()?;
        super::validate_current_snapshot_contract(&checkpoint.state)?;
        let state = &checkpoint.state;
        super::validate_scenario_state(&super::Scenario {
            start_time: state.now,
            entities: state.entities.clone(),
            world: state.world.clone(),
            knowledge: state.knowledge.clone(),
            domain_records: state.domain_records.clone(),
        })?;
        let plugins = super::PluginRegistry::from_descriptors(state.plugin_descriptors.clone())?;
        super::validation::validate_continuation_state(state, &plugins)?;
        for record in &checkpoint.pinned.ingress {
            super::validation::validate_continuation_ingress(state, &plugins, record)?;
        }
        let keyed_draws =
            super::random::KeyedDrawLedger::from_entries(state.root_seed, &checkpoint.keyed_draws)?;
        let frontiers = &checkpoint.journal_frontiers;
        let journals = [
            import_frontier(&frontiers.commands)?,
            import_frontier(&frontiers.attempts)?,
            import_frontier(&frontiers.events)?,
            import_frontier(&frontiers.ingress)?,
            import_frontier(&frontiers.random_draws)?,
        ];

        let initial_scenario = Some(
            state
                .initial_scenario
                .clone()
                .ok_or_else(|| invalid("continuation state requires its initial scenario"))?,
        );
        let initial_domain_record_indexes =
            super::state::InitialRecordIndex::of(initial_scenario.as_ref());
        let current_domain_record_versions: im::OrdMap<
            Arc<super::DomainRecordRef>,
            Arc<super::CurrentDomainRecordVersion>,
        > = checkpoint
            .current_record_versions
            .iter()
            .map(|current| {
                (
                    Arc::new(current.version.record.clone()),
                    Arc::new(super::CurrentDomainRecordVersion {
                        version: current.version.clone(),
                        established_at: current.established_at,
                    }),
                )
            })
            .collect();
        if current_domain_record_versions.len() != state.domain_records.len()
            || state.domain_records.iter().any(|record| {
                current_domain_record_versions
                    .get(&record.reference)
                    .is_none_or(|current| current.version.version != record.version)
            })
        {
            return Err(invalid(
                "continuation current record versions do not match the live records",
            ));
        }
        let pending_ingress = checkpoint
            .pending_ingress
            .iter()
            .map(|id| {
                checkpoint
                    .pinned
                    .ingress
                    .binary_search_by_key(id, |record| record.id)
                    .map(|index| IngressQueueKey::from_record(&checkpoint.pinned.ingress[index]))
                    .map_err(|_| invalid("pending ingress is not pinned"))
            })
            .collect::<Result<BTreeSet<_>, _>>()?;

        let ContinuationCheckpoint {
            state,
            settled_cut,
            boundary_head,
            unadmitted,
            pinned,
            ingress_provenance,
            known_record_versions,
            requests,
            history_flags,
            evidence_times,
            settled_boundaries,
            continuation_hash,
            ..
        } = checkpoint;
        let pins = PinnedEvidence {
            events: pinned
                .events
                .into_iter()
                .map(|record| (record.id, Arc::new(record)))
                .collect(),
            commands: pinned
                .commands
                .into_iter()
                .map(|record| (record.id, Arc::new(record)))
                .collect(),
            command_attempts: pinned
                .command_attempts
                .into_iter()
                .map(|record| (record.id, Arc::new(record)))
                .collect(),
            ingress: pinned
                .ingress
                .into_iter()
                .map(|record| (record.id, Arc::new(record)))
                .collect(),
            boundaries: pinned
                .boundaries
                .into_iter()
                .map(|record| (record.id, Arc::new(record)))
                .collect(),
            random_draws: pinned
                .random_draws
                .into_iter()
                .map(|record| (record.id, Arc::new(record)))
                .collect(),
            record_versions: pinned
                .record_versions
                .into_iter()
                .map(|pin| (pin.version, Arc::new(pin.body)))
                .collect(),
            known_record_versions: known_record_versions.into_iter().collect(),
            ingress_provenance: ingress_provenance
                .into_iter()
                .map(|fact| (fact.ingress, fact))
                .collect(),
        };
        let [
            events,
            commands,
            command_attempts,
            ingress,
            boundaries,
            random_draws,
        ] = [
            &evidence_times.events,
            &evidence_times.commands,
            &evidence_times.command_attempts,
            &evidence_times.ingress,
            &evidence_times.boundaries,
            &evidence_times.random_draws,
        ]
        .map(|runs| import_time_runs(runs));
        let expected_checkpoint_hash = state.checkpoint_hash.clone();
        let expected_roots = state.commitment_roots.clone();
        let mut evidence = super::state::RuntimeEvidence {
            archived: settled_cut,
            archived_boundary_head: boundary_head,
            archived_legacy_commands: history_flags.legacy_commands,
            archived_tracked_attempts: history_flags.tracked_attempts,
            archived_unqueued_command_history: history_flags.unqueued_command_history,
            archived_command_requests: requests
                .commands
                .into_iter()
                .map(|entry| {
                    (
                        entry.request_id,
                        Arc::new(super::state::ArchivedCommandRequestOutcome {
                            input_hash: entry.input_hash,
                            outcome: entry.outcome,
                        }),
                    )
                })
                .collect(),
            archived_ingress_requests: requests
                .queued_commands
                .into_iter()
                .map(|entry| {
                    (
                        entry.request_id,
                        super::state::ArchivedIngressRequest {
                            input_hash: entry.input_hash,
                            receipt: entry.receipt,
                        },
                    )
                })
                .collect(),
            archived_decision_requests: requests
                .decisions
                .into_iter()
                .map(|entry| {
                    (
                        entry.request_id,
                        super::state::ArchivedIngressRequest {
                            input_hash: entry.input_hash,
                            receipt: entry.receipt,
                        },
                    )
                })
                .collect(),
            archived_decision_command_requests: requests.decision_commands.into_iter().collect(),
            events: unadmitted.events,
            commands: super::state::RetainedJournal::default(),
            command_attempts: super::state::RetainedJournal::default(),
            ingress: super::state::RetainedJournal::default(),
            command_index: super::state::CommandHistoryIndex::default(),
            boundaries: Vec::new(),
            random_draws: Vec::new(),
            archived_segment_headers: im::Vector::new(),
            archived_evidence_receipts: im::OrdMap::new(),
            keyed_draws,
            sealed_times: super::state::EvidenceTimeIndex::from_runs([
                events,
                commands,
                command_attempts,
                ingress,
                boundaries,
                random_draws,
            ]),
            pinned: pins,
            continuation_base: Some(ContinuationBase {
                cut: settled_cut,
                checkpoint_hash: expected_checkpoint_hash.clone(),
                continuation_hash,
            }),
        };
        // The archived request tables are in place, so the restored history
        // builds its index against them.
        evidence.restore_command_history(
            unadmitted.commands,
            unadmitted.command_attempts,
            Vec::new(),
        );
        let mut simulation = Self::assemble_runtime(
            state,
            plugins,
            initial_scenario,
            initial_domain_record_indexes,
            pending_ingress,
            BTreeSet::new(),
            current_domain_record_versions,
            evidence,
        )?;
        simulation.state.scheduler.settled_boundaries =
            settled_boundaries.into_iter().map(Arc::new).collect();
        simulation.state.metadata.commitment_cache = Some(Box::new(
            super::state::RuntimeCommitmentCache::from_journals(journals),
        ));
        simulation.refresh_checkpoint_hash()?;
        if simulation.state.metadata.checkpoint_hash != expected_checkpoint_hash
            || simulation.state.metadata.commitment_roots != expected_roots
        {
            return Err(invalid(
                "continuation state and journal frontiers do not reproduce the checkpoint hash",
            ));
        }
        Ok(simulation)
    }

    /// The canonical form of a restored boundary ledger: consecutive settled
    /// boundaries that end at the last boundary and reach back no further
    /// than the run's boundary-schedule horizon. Whether earlier boundaries
    /// inside the horizon were left out cannot be checked without history.
    fn check_boundary_ledger_form(&self) -> Result<(), CanwuError> {
        let ledger = &self.state.scheduler.settled_boundaries;
        let last_boundary = self.state.counters.next_boundary_id.saturating_sub(1);
        let (Some(first), Some(last)) = (ledger.front(), ledger.back()) else {
            return if last_boundary == 0 {
                Ok(())
            } else {
                Err(invalid(
                    "continuation boundary ledger omits the last boundary",
                ))
            };
        };
        let consecutive = ledger
            .iter()
            .zip(ledger.iter().skip(1))
            .all(|(earlier, later)| {
                earlier
                    .boundary_id
                    .zip(later.boundary_id)
                    .is_some_and(|(earlier, later)| later.get() == earlier.get() + 1)
            });
        let horizon = self.plugins.boundary_schedule_horizon();
        if !consecutive
            || last.boundary_id.map(BoundaryId::get) != Some(last_boundary)
            || first.at < super::boundary_window::kept_from(last.at, horizon)
        {
            return Err(invalid(
                "continuation boundary ledger is not the settled boundaries within the horizon",
            ));
        }
        Ok(())
    }

    /// Step 8 of a continuation restore: the checkpoint derived from the
    /// restored runtime must be the input, so the carried closure is exactly
    /// the closure of the restored state and every part is canonical.
    pub(super) fn verify_continuation_round_trip(
        &self,
        continuation_hash: &str,
    ) -> Result<(), CanwuError> {
        self.check_boundary_ledger_form()?;
        // A closure that cannot be derived (a cause or a declared dependency
        // beyond the journal end, say) makes the input invalid.
        let (_, derived) = self.continuation_checkpoint().map_err(|error| {
            invalid(format!(
                "continuation checkpoint closure does not resolve: {}",
                error.message
            ))
        })?;
        if derived != continuation_hash {
            return Err(invalid(
                "continuation checkpoint is not the closure of the state it restores",
            ));
        }
        Ok(())
    }
}
