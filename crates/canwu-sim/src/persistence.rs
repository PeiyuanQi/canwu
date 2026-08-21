use super::{
    ADMISSION_CURSOR_FORMAT_VERSION, BoundaryReceipt, BoundaryRecord, BoundaryRequest, CanwuError,
    CauseRef, CommandAttemptOutcome, CommandAttemptRecord, CommandEnvelope, CommandOutcome,
    CommandReceipt, CommandRecord, CommandRequest, DomainRecord, DomainRecordRef,
    DomainRecordSchema, DomainRecordType, DomainRecordVersionRef, DomainRecordVersionSource,
    ENGINE_VERSION, ErrorCode, EvidenceRef, IngressPayload, IngressReceipt, IngressRecord,
    KeyedDrawReservation, KnowledgeSnapshot, PayloadProperty, PayloadSchema, PayloadValueType,
    PluginIngressRequest, RandomDrawAddress, RandomDrawRecord, ReplayJournal, RuntimeEvidence,
    SNAPSHOT_FORMAT_VERSION, STATE_REVISION_FORMAT_VERSION, ScheduledAction, ScheduledRecord,
    SimDuration, SimEvent, SimTime, Simulation, SimulationPlugin, SimulationSnapshot,
    SystemCadence, TypedDomainRecordRef, WorldSnapshot, has_unqueued_command_history,
    invalid_snapshot_error,
};
use crate::state::{ArchivedCommandRequestOutcome, ArchivedIngressRequest};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Version of current-state checkpoints plus append-only evidence segments.
pub const CHECKPOINT_JOURNAL_FORMAT_VERSION: u32 = 1;
const ARCHIVED_SEGMENT_MANIFEST_DOMAIN: &str = "canwu.evidence.archived-segment-manifest.v1";
const ARCHIVED_RECEIPT_DOMAIN: &str = "canwu.evidence.archived-receipts.v1";
const EVIDENCE_DEPENDENCY_DOMAIN: &str = "canwu.evidence.dependencies.v1";
const KEYED_RESERVATION_DOMAIN: &str = "canwu.random.keyed-reservations.v1";
/// Reserved domain-record payload field declaring a payload-reading continuation.
pub const PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD: &str =
    "canwu_payload_required_evidence_continuation";
/// Current wire version of [`PayloadRequiredEvidenceContinuationV1`].
pub const PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FORMAT_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceJournalKind {
    Event,
    Command,
    CommandAttempt,
    Ingress,
    Boundary,
    RandomDraw,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EvidenceNestedLocator {
    None,
    BoundaryRecordChange { change_index: u64 },
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct EvidenceItemLocator {
    pub journal: EvidenceJournalKind,
    pub absolute_index: u64,
    pub nested: EvidenceNestedLocator,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ArchivedEvidenceLocator {
    pub segment_id: String,
    pub item: EvidenceItemLocator,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ArchivedEvidenceReceipt {
    pub evidence: EvidenceRef,
    pub locator: ArchivedEvidenceLocator,
    pub evidence_index_leaf: u64,
    pub item_commitment: String,
    pub merkle_path: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct EvidenceIndexEntry {
    pub reference: EvidenceRef,
    pub item: EvidenceItemLocator,
    pub item_commitment: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EvidenceJournalRoots {
    pub events: String,
    pub commands: String,
    pub command_attempts: String,
    pub ingress: String,
    pub boundaries: String,
    pub random_draws: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ArchivedSegmentHeader {
    pub segment_id: String,
    pub start: EvidenceCursor,
    pub end: EvidenceCursor,
    pub journal_roots: EvidenceJournalRoots,
    pub evidence_index_root: String,
    pub evidence_index_entry_count: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceRequirement {
    /// Only the committed evidence identity and causal cut must remain provable.
    IdentityOnly,
    /// An active schema-declared continuation must be able to reload the payload.
    PayloadRequired,
}

/// Authoritative pending-continuation contract for rules that must inspect old payload bytes.
///
/// A plugin opts in by declaring [`PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD`]
/// as a required object in a versioned domain-record schema. The current active
/// domain record stores this object in its payload. Because both schema and
/// record participate in plugin identity, state commitments, replay, and normal
/// mutation validation, dependencies cannot be supplied as an ephemeral seal hint.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PayloadRequiredEvidenceContinuationV1 {
    pub format_version: u32,
    pub active: bool,
    pub dependencies: Vec<EvidenceRef>,
}

impl PayloadRequiredEvidenceContinuationV1 {
    /// Builds an active V1 continuation. Dependencies must be sorted and unique.
    #[must_use]
    pub fn active(dependencies: Vec<EvidenceRef>) -> Self {
        Self {
            format_version: PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FORMAT_VERSION,
            active: true,
            dependencies,
        }
    }

    /// Builds the canonical terminal form, which retains no payload dependency.
    #[must_use]
    pub const fn completed() -> Self {
        Self {
            format_version: PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FORMAT_VERSION,
            active: false,
            dependencies: Vec::new(),
        }
    }
}

/// Returns the reserved property a domain-record schema must declare to
/// authoritatively produce `PayloadRequired` dependencies.
#[must_use]
pub fn payload_required_evidence_continuation_property_v1() -> PayloadProperty {
    PayloadProperty {
        value_type: PayloadValueType::Object,
        required: true,
    }
}
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct EvidenceDependency {
    pub reference: EvidenceRef,
    pub requirement: EvidenceRequirement,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EvidenceArchiveIndex {
    pub header: ArchivedSegmentHeader,
    pub entries: Vec<EvidenceIndexEntry>,
}

pub trait ArchiveProvider {
    fn load_evidence_segment(
        &self,
        segment_id: &str,
    ) -> Result<Option<EvidenceJournalSegment>, CanwuError>;
}

pub trait ArchiveStore: ArchiveProvider {
    fn store_evidence_segment(
        &self,
        segment: &EvidenceJournalSegment,
    ) -> Result<ArchiveStoreOutcome, CanwuError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArchiveStoreOutcome {
    Stored,
    AlreadyPresent,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EvidenceSealToken {
    pub source_state_hash: String,
    pub source_checkpoint_hash: String,
    pub source_end: EvidenceCursor,
    pub segment_id: String,
    pub target_checkpoint_hash: String,
    pub token_hash: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PreparedEvidenceSeal {
    pub token: EvidenceSealToken,
    pub segment: EvidenceJournalSegment,
}

#[derive(Serialize)]
struct EvidenceIndexLeafMaterial<'a> {
    format_version: u32,
    reference: &'a EvidenceRef,
    item: &'a EvidenceItemLocator,
    item_commitment: &'a str,
}

#[derive(Serialize)]
struct ArchivedSegmentHeaderMaterial<'a> {
    start: EvidenceCursor,
    end: EvidenceCursor,
    journal_roots: &'a EvidenceJournalRoots,
    evidence_index_root: &'a str,
    evidence_index_entry_count: u64,
}

fn archive_error(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidArchive, message)
}

fn decode_hash(value: &str, label: &str) -> Result<[u8; 32], CanwuError> {
    if value.len() != 64
        || value
            .bytes()
            .any(|byte| !byte.is_ascii_hexdigit() || byte.is_ascii_uppercase())
    {
        return Err(archive_error(format!(
            "{label} must be 32-byte lower-case hex"
        )));
    }
    let mut bytes = [0_u8; 32];
    for (index, pair) in value.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let digit = |byte: u8| match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            _ => None,
        };
        bytes[index] = digit(pair[0])
            .and_then(|high| digit(pair[1]).map(|low| (high << 4) | low))
            .ok_or_else(|| archive_error(format!("{label} contains invalid hex")))?;
    }
    Ok(bytes)
}

fn archive_node(left: [u8; 32], right: [u8; 32]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"canwu.evidence.index.node.v1");
    hasher.update(&[0]);
    hasher.update(&left);
    hasher.update(&right);
    *hasher.finalize().as_bytes()
}

fn archive_empty_root() -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"canwu.evidence.index.empty.v1");
    hasher.update(&[0]);
    hasher.finalize().to_hex().to_string()
}

fn skipped_commitment_root<T: Serialize>(
    domain: &str,
    values: &[T],
) -> Result<Option<String>, CanwuError> {
    if values.is_empty() {
        Ok(None)
    } else {
        super::canonical_hash(domain, values).map(Some)
    }
}

fn validate_skipped_commitment_root<T: Serialize>(
    root: Option<&str>,
    domain: &str,
    values: &[T],
    label: &str,
) -> Result<(), CanwuError> {
    let expected = skipped_commitment_root(domain, values)?;
    if root != expected.as_deref() {
        return Err(invalid_snapshot_error(format!(
            "compact continuation {label} does not match its canonical material"
        )));
    }
    Ok(())
}

fn promote_dependency(
    dependencies: &mut BTreeMap<EvidenceRef, EvidenceRequirement>,
    reference: EvidenceRef,
    requirement: EvidenceRequirement,
) {
    dependencies
        .entry(reference)
        .and_modify(|current| *current = (*current).max(requirement))
        .or_insert(requirement);
}
fn schema_declares_payload_required_continuation(schema: &DomainRecordSchema) -> bool {
    matches!(
        &schema.payload_schema,
        PayloadSchema::Object { properties, .. }
            if properties.get(PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD)
                == Some(&payload_required_evidence_continuation_property_v1())
    )
}

fn add_payload_required_continuation_dependencies(
    dependencies: &mut BTreeMap<EvidenceRef, EvidenceRequirement>,
    record: &DomainRecord,
    schema: &DomainRecordSchema,
) -> Result<(), CanwuError> {
    if !record.is_active() || !schema_declares_payload_required_continuation(schema) {
        return Ok(());
    }
    let continuation = record
        .payload
        .get(PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD)
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::ArchiveNotReady,
                format!(
                    "payload-required continuation record {} is missing its schema-declared field",
                    record.reference
                ),
            )
        })?;
    let continuation: PayloadRequiredEvidenceContinuationV1 =
        serde_json::from_value(continuation.clone()).map_err(|error| {
            CanwuError::new(
                ErrorCode::ArchiveNotReady,
                format!(
                    "payload-required continuation record {} is invalid: {error}",
                    record.reference
                ),
            )
        })?;
    if continuation.format_version != PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FORMAT_VERSION {
        return Err(CanwuError::new(
            ErrorCode::ArchiveNotReady,
            format!(
                "payload-required continuation record {} uses unsupported format {}",
                record.reference, continuation.format_version
            ),
        ));
    }
    if !continuation.active {
        if continuation == PayloadRequiredEvidenceContinuationV1::completed() {
            return Ok(());
        }
        return Err(CanwuError::new(
            ErrorCode::ArchiveNotReady,
            format!(
                "completed payload-required continuation record {} retains dependencies",
                record.reference
            ),
        ));
    }
    let continuation = PayloadRequiredEvidenceContinuationV1::active(continuation.dependencies);
    if continuation.dependencies.is_empty()
        || continuation
            .dependencies
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
    {
        return Err(CanwuError::new(
            ErrorCode::ArchiveNotReady,
            format!(
                "active payload-required continuation record {} needs sorted unique dependencies",
                record.reference
            ),
        ));
    }
    if continuation.dependencies.iter().any(|reference| {
        matches!(
            reference,
            EvidenceRef::DomainRecordVersion(version)
                if version.record == record.reference && version.version == record.version
        )
    }) {
        return Err(CanwuError::new(
            ErrorCode::ArchiveNotReady,
            format!(
                "payload-required continuation record {} cannot depend on its own version",
                record.reference
            ),
        ));
    }
    for reference in continuation.dependencies {
        if matches!(
            &reference,
            EvidenceRef::DomainRecordVersion(version)
                if matches!(version.established_by, DomainRecordVersionSource::InitialScenario)
        ) {
            return Err(CanwuError::new(
                ErrorCode::ArchiveNotReady,
                format!(
                    "payload-required continuation record {} cannot depend on an initial-scenario payload",
                    record.reference
                ),
            ));
        }
        promote_dependency(
            dependencies,
            reference,
            EvidenceRequirement::PayloadRequired,
        );
    }
    Ok(())
}

fn required_archived_receipt_references(
    dependencies: &[EvidenceDependency],
    reservations: &[KeyedDrawReservation],
) -> BTreeSet<EvidenceRef> {
    let mut required = dependencies
        .iter()
        .map(|dependency| dependency.reference.clone())
        .collect::<BTreeSet<_>>();
    required.extend(
        reservations
            .iter()
            .map(|reservation| reservation.draw_receipt.evidence.clone()),
    );
    required
}

fn retain_reachable_archived_evidence_receipts(
    receipts: &mut BTreeMap<EvidenceRef, ArchivedEvidenceReceipt>,
    dependencies: &[EvidenceDependency],
    reservations: &[KeyedDrawReservation],
) {
    let required = required_archived_receipt_references(dependencies, reservations);
    receipts.retain(|reference, _| required.contains(reference));
}

pub(crate) fn load_verified_archived_evidence_segment(
    receipt: &ArchivedEvidenceReceipt,
    provider: &dyn ArchiveProvider,
) -> Result<EvidenceJournalSegment, CanwuError> {
    let segment = provider
        .load_evidence_segment(&receipt.locator.segment_id)?
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::EvidenceContentUnavailable,
                "the archive provider did not return the required evidence segment",
            )
        })?;
    let receipts = verify_archived_segment(&segment).map_err(|error| {
        CanwuError::new(
            ErrorCode::InvalidArchive,
            format!(
                "archive provider returned invalid content: {}",
                error.message
            ),
        )
    })?;
    if !receipts.iter().any(|candidate| candidate == receipt) {
        return Err(archive_error(
            "archive provider segment does not reproduce the committed evidence receipt",
        ));
    }
    Ok(segment)
}

fn add_cause_dependency(
    dependencies: &mut BTreeMap<EvidenceRef, EvidenceRequirement>,
    cause: &CauseRef,
) {
    let reference = match cause {
        CauseRef::Event(id) => Some(EvidenceRef::Event(*id)),
        CauseRef::Command(id) => Some(EvidenceRef::Command(*id)),
        CauseRef::Boundary(id) => Some(EvidenceRef::Boundary(*id)),
        CauseRef::System(_) => None,
    };
    if let Some(reference) = reference {
        promote_dependency(dependencies, reference, EvidenceRequirement::IdentityOnly);
    }
}

fn add_command_outcome_dependencies(
    dependencies: &mut BTreeMap<EvidenceRef, EvidenceRequirement>,
    outcome: &CommandOutcome,
) {
    let (attempt_id, command_id, emitted_events) = match outcome {
        CommandOutcome::Accepted { receipt } => (
            receipt.attempt_id,
            Some(receipt.command_id),
            receipt.emitted_events.as_slice(),
        ),
        CommandOutcome::Rejected { rejection } => (rejection.attempt_id, None, &[][..]),
    };
    if let Some(id) = attempt_id {
        promote_dependency(
            dependencies,
            EvidenceRef::CommandAttempt(id),
            EvidenceRequirement::IdentityOnly,
        );
    }
    if let Some(id) = command_id {
        promote_dependency(
            dependencies,
            EvidenceRef::Command(id),
            EvidenceRequirement::IdentityOnly,
        );
    }
    for id in emitted_events {
        promote_dependency(
            dependencies,
            EvidenceRef::Event(*id),
            EvidenceRequirement::IdentityOnly,
        );
    }
}

fn archive_leaf(entry: &EvidenceIndexEntry) -> Result<[u8; 32], CanwuError> {
    let hash = super::canonical_hash(
        "canwu.evidence.index.leaf.v1",
        &EvidenceIndexLeafMaterial {
            format_version: 1,
            reference: &entry.reference,
            item: &entry.item,
            item_commitment: &entry.item_commitment,
        },
    )?;
    decode_hash(&hash, "evidence-index leaf")
}

fn archive_merkle(
    entries: &[EvidenceIndexEntry],
) -> Result<(String, Vec<Vec<String>>), CanwuError> {
    if entries.is_empty() {
        return Ok((archive_empty_root(), Vec::new()));
    }
    let mut level: Vec<[u8; 32]> = entries.iter().map(archive_leaf).collect::<Result<_, _>>()?;
    let mut positions: Vec<usize> = (0..entries.len()).collect();
    let mut proofs = vec![Vec::new(); entries.len()];
    while level.len() > 1 {
        for (leaf, position) in positions.iter().copied().enumerate() {
            let sibling = if position % 2 == 0 {
                (position + 1).min(level.len() - 1)
            } else {
                position - 1
            };
            proofs[leaf].push(
                blake3::Hash::from_bytes(level[sibling])
                    .to_hex()
                    .to_string(),
            );
        }
        let mut next = Vec::with_capacity(level.len().div_ceil(2));
        for pair in level.chunks(2) {
            next.push(archive_node(pair[0], *pair.get(1).unwrap_or(&pair[0])));
        }
        level = next;
        for position in &mut positions {
            *position /= 2;
        }
    }
    Ok((
        blake3::Hash::from_bytes(level[0]).to_hex().to_string(),
        proofs,
    ))
}

fn item_commitment<T: Serialize>(
    journal: EvidenceJournalKind,
    item: &T,
) -> Result<String, CanwuError> {
    let domain = match journal {
        EvidenceJournalKind::Event => "canwu.evidence.item.event.v1",
        EvidenceJournalKind::Command => "canwu.evidence.item.command.v1",
        EvidenceJournalKind::CommandAttempt => "canwu.evidence.item.command_attempt.v1",
        EvidenceJournalKind::Ingress => "canwu.evidence.item.ingress.v1",
        EvidenceJournalKind::Boundary => "canwu.evidence.item.boundary.v1",
        EvidenceJournalKind::RandomDraw => "canwu.evidence.item.random_draw.v1",
    };
    super::canonical_hash(domain, item)
}

pub(crate) fn evidence_archive_index(
    segment: &EvidenceJournalSegment,
) -> Result<(EvidenceArchiveIndex, Vec<ArchivedEvidenceReceipt>), CanwuError> {
    let roots = EvidenceJournalRoots {
        events: super::canonical_hash("canwu.evidence.journal.events.v1", &segment.events)?,
        commands: super::canonical_hash("canwu.evidence.journal.commands.v1", &segment.commands)?,
        command_attempts: super::canonical_hash(
            "canwu.evidence.journal.command_attempts.v1",
            &segment.command_attempts,
        )?,
        ingress: super::canonical_hash("canwu.evidence.journal.ingress.v1", &segment.ingress)?,
        boundaries: super::canonical_hash(
            "canwu.evidence.journal.boundaries.v1",
            &segment.boundaries,
        )?,
        random_draws: super::canonical_hash(
            "canwu.evidence.journal.random_draws.v1",
            &segment.random_draws,
        )?,
    };
    let mut entries = Vec::new();
    let mut add = |reference: EvidenceRef, journal, absolute_index, nested, commitment: String| {
        entries.push(EvidenceIndexEntry {
            reference,
            item: EvidenceItemLocator {
                journal,
                absolute_index,
                nested,
            },
            item_commitment: commitment,
        });
    };
    for (offset, event) in segment.events.iter().enumerate() {
        add(
            EvidenceRef::Event(event.id),
            EvidenceJournalKind::Event,
            segment.start.event_count + offset as u64 + 1,
            EvidenceNestedLocator::None,
            item_commitment(EvidenceJournalKind::Event, event)?,
        );
    }
    for (offset, command) in segment.commands.iter().enumerate() {
        add(
            EvidenceRef::Command(command.id),
            EvidenceJournalKind::Command,
            segment.start.command_count + offset as u64 + 1,
            EvidenceNestedLocator::None,
            item_commitment(EvidenceJournalKind::Command, command)?,
        );
    }
    for (offset, attempt) in segment.command_attempts.iter().enumerate() {
        add(
            EvidenceRef::CommandAttempt(attempt.id),
            EvidenceJournalKind::CommandAttempt,
            segment.start.command_attempt_count + offset as u64 + 1,
            EvidenceNestedLocator::None,
            item_commitment(EvidenceJournalKind::CommandAttempt, attempt)?,
        );
    }
    for (offset, ingress) in segment.ingress.iter().enumerate() {
        add(
            EvidenceRef::Ingress(ingress.id),
            EvidenceJournalKind::Ingress,
            segment.start.ingress_count + offset as u64 + 1,
            EvidenceNestedLocator::None,
            item_commitment(EvidenceJournalKind::Ingress, ingress)?,
        );
    }
    for (offset, boundary) in segment.boundaries.iter().enumerate() {
        let absolute_index = segment.start.boundary_count + offset as u64 + 1;
        let commitment = item_commitment(EvidenceJournalKind::Boundary, boundary)?;
        add(
            EvidenceRef::Boundary(boundary.id),
            EvidenceJournalKind::Boundary,
            absolute_index,
            EvidenceNestedLocator::None,
            commitment.clone(),
        );
        for (change_index, change) in boundary.record_changes.iter().enumerate() {
            add(
                EvidenceRef::DomainRecordVersion(DomainRecordVersionRef {
                    record: change.current.reference.clone(),
                    version: change.current.version,
                    established_by: DomainRecordVersionSource::BoundaryChange {
                        boundary: boundary.id,
                        change_index: change_index as u64,
                    },
                }),
                EvidenceJournalKind::Boundary,
                absolute_index,
                EvidenceNestedLocator::BoundaryRecordChange {
                    change_index: change_index as u64,
                },
                commitment.clone(),
            );
        }
    }
    for (offset, draw) in segment.random_draws.iter().enumerate() {
        add(
            EvidenceRef::RandomDraw(draw.id),
            EvidenceJournalKind::RandomDraw,
            segment.start.random_draw_count + offset as u64 + 1,
            EvidenceNestedLocator::None,
            item_commitment(EvidenceJournalKind::RandomDraw, draw)?,
        );
    }
    entries.sort();
    if entries
        .windows(2)
        .any(|window| window[0].reference == window[1].reference)
    {
        return Err(archive_error(
            "evidence archive contains duplicate references",
        ));
    }
    let (evidence_index_root, proofs) = archive_merkle(&entries)?;
    let entry_count = u64::try_from(entries.len())
        .map_err(|_| archive_error("evidence-index entry count exceeds u64"))?;
    let segment_id = super::canonical_hash(
        "canwu.evidence.segment.v2",
        &ArchivedSegmentHeaderMaterial {
            start: segment.start,
            end: segment.end,
            journal_roots: &roots,
            evidence_index_root: &evidence_index_root,
            evidence_index_entry_count: entry_count,
        },
    )?;
    let header = ArchivedSegmentHeader {
        segment_id: segment_id.clone(),
        start: segment.start,
        end: segment.end,
        journal_roots: roots,
        evidence_index_root,
        evidence_index_entry_count: entry_count,
    };
    let receipts = entries
        .iter()
        .zip(proofs)
        .enumerate()
        .map(|(index, (entry, merkle_path))| ArchivedEvidenceReceipt {
            evidence: entry.reference.clone(),
            locator: ArchivedEvidenceLocator {
                segment_id: segment_id.clone(),
                item: entry.item.clone(),
            },
            evidence_index_leaf: index as u64,
            item_commitment: entry.item_commitment.clone(),
            merkle_path,
        })
        .collect();
    Ok((EvidenceArchiveIndex { header, entries }, receipts))
}

fn verify_archive_receipt(
    receipt: &ArchivedEvidenceReceipt,
    header: &ArchivedSegmentHeader,
) -> Result<(), CanwuError> {
    if receipt.locator.segment_id != header.segment_id
        || receipt.evidence_index_leaf >= header.evidence_index_entry_count
    {
        return Err(archive_error(
            "archived evidence receipt does not belong to its segment header",
        ));
    }
    let entry = EvidenceIndexEntry {
        reference: receipt.evidence.clone(),
        item: receipt.locator.item.clone(),
        item_commitment: receipt.item_commitment.clone(),
    };
    let mut hash = archive_leaf(&entry)?;
    let mut position = receipt.evidence_index_leaf;
    let mut width = header.evidence_index_entry_count;
    let mut path_at = 0usize;
    while width > 1 {
        let sibling = receipt
            .merkle_path
            .get(path_at)
            .ok_or_else(|| archive_error("archived evidence receipt Merkle path is too short"))?;
        let sibling = decode_hash(sibling, "receipt Merkle sibling")?;
        hash = if position.is_multiple_of(2) {
            archive_node(hash, sibling)
        } else {
            archive_node(sibling, hash)
        };
        position /= 2;
        width = width.div_ceil(2);
        path_at += 1;
    }
    if path_at != receipt.merkle_path.len()
        || blake3::Hash::from_bytes(hash).to_hex().as_str() != header.evidence_index_root
    {
        return Err(archive_error(
            "archived evidence receipt Merkle proof is invalid",
        ));
    }
    let (expected_journal, expected_nested) = match &receipt.evidence {
        EvidenceRef::Event(_) => (EvidenceJournalKind::Event, EvidenceNestedLocator::None),
        EvidenceRef::Command(_) => (EvidenceJournalKind::Command, EvidenceNestedLocator::None),
        EvidenceRef::CommandAttempt(_) => (
            EvidenceJournalKind::CommandAttempt,
            EvidenceNestedLocator::None,
        ),
        EvidenceRef::Ingress(_) => (EvidenceJournalKind::Ingress, EvidenceNestedLocator::None),
        EvidenceRef::Boundary(_) => (EvidenceJournalKind::Boundary, EvidenceNestedLocator::None),
        EvidenceRef::RandomDraw(_) => {
            (EvidenceJournalKind::RandomDraw, EvidenceNestedLocator::None)
        }
        EvidenceRef::DomainRecordVersion(version) => match version.established_by {
            DomainRecordVersionSource::BoundaryChange { change_index, .. } => (
                EvidenceJournalKind::Boundary,
                EvidenceNestedLocator::BoundaryRecordChange { change_index },
            ),
            DomainRecordVersionSource::InitialScenario => {
                return Err(archive_error(
                    "initial-scenario record versions cannot be archived",
                ));
            }
        },
    };
    if receipt.locator.item.journal != expected_journal
        || receipt.locator.item.nested != expected_nested
    {
        return Err(archive_error(
            "archived evidence receipt uses an illegal typed locator",
        ));
    }
    let (start, end) = match expected_journal {
        EvidenceJournalKind::Event => (header.start.event_count, header.end.event_count),
        EvidenceJournalKind::Command => (header.start.command_count, header.end.command_count),
        EvidenceJournalKind::CommandAttempt => (
            header.start.command_attempt_count,
            header.end.command_attempt_count,
        ),
        EvidenceJournalKind::Ingress => (header.start.ingress_count, header.end.ingress_count),
        EvidenceJournalKind::Boundary => (header.start.boundary_count, header.end.boundary_count),
        EvidenceJournalKind::RandomDraw => {
            (header.start.random_draw_count, header.end.random_draw_count)
        }
    };
    if receipt.locator.item.absolute_index <= start || receipt.locator.item.absolute_index > end {
        return Err(archive_error(
            "archived evidence locator lies outside its segment cursor range",
        ));
    }
    Ok(())
}

pub(crate) fn verify_archived_segment(
    segment: &EvidenceJournalSegment,
) -> Result<Vec<ArchivedEvidenceReceipt>, CanwuError> {
    let stored = segment
        .archive
        .as_ref()
        .ok_or_else(|| archive_error("archived segment is missing its evidence index"))?;
    let mut plain = segment.clone();
    plain.archive = None;
    let (rebuilt, receipts) = evidence_archive_index(&plain)?;
    if &rebuilt != stored {
        return Err(archive_error(
            "archived segment header or evidence index does not match its journal content",
        ));
    }
    for receipt in &receipts {
        verify_archive_receipt(receipt, &stored.header)?;
    }
    Ok(receipts)
}

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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub archived_segment_headers: Vec<ArchivedSegmentHeader>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_segment_manifest_root: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub archived_evidence_receipts: Vec<ArchivedEvidenceReceipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_receipt_root: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_dependencies: Vec<EvidenceDependency>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_dependency_root: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keyed_draw_reservations: Vec<KeyedDrawReservation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyed_reservation_root: Option<String>,
}

#[derive(Serialize)]
struct CompactCheckpointHashMaterial<'a> {
    state_checkpoint_hash: &'a str,
    journal_end: EvidenceCursor,
    archived_segment_manifest_root: Option<&'a str>,
    archived_receipt_root: Option<&'a str>,
    evidence_dependency_root: Option<&'a str>,
    keyed_reservation_root: Option<&'a str>,
}

fn validate_compact_continuation(checkpoint: &SimulationCheckpoint) -> Result<(), CanwuError> {
    if checkpoint
        .archived_segment_headers
        .windows(2)
        .any(|headers| headers[0].end != headers[1].start)
        || checkpoint
            .archived_segment_headers
            .iter()
            .map(|header| &header.segment_id)
            .collect::<BTreeSet<_>>()
            .len()
            != checkpoint.archived_segment_headers.len()
    {
        return Err(invalid_snapshot_error(
            "compact archived-segment manifest is duplicated or noncontiguous",
        ));
    }
    if checkpoint
        .archived_evidence_receipts
        .windows(2)
        .any(|receipts| receipts[0].evidence >= receipts[1].evidence)
    {
        return Err(invalid_snapshot_error(
            "compact archived receipts must be sorted by unique evidence reference",
        ));
    }
    if checkpoint
        .evidence_dependencies
        .windows(2)
        .any(|dependencies| dependencies[0].reference >= dependencies[1].reference)
    {
        return Err(invalid_snapshot_error(
            "compact evidence dependencies must be sorted by unique evidence reference",
        ));
    }
    if checkpoint
        .keyed_draw_reservations
        .windows(2)
        .any(|reservations| {
            (&reservations[0].stream, &reservations[0].address)
                >= (&reservations[1].stream, &reservations[1].address)
        })
    {
        return Err(invalid_snapshot_error(
            "compact keyed reservations must be sorted by unique operation address",
        ));
    }
    validate_skipped_commitment_root(
        checkpoint.archived_segment_manifest_root.as_deref(),
        ARCHIVED_SEGMENT_MANIFEST_DOMAIN,
        &checkpoint.archived_segment_headers,
        "archived-segment manifest root",
    )?;
    validate_skipped_commitment_root(
        checkpoint.archived_receipt_root.as_deref(),
        ARCHIVED_RECEIPT_DOMAIN,
        &checkpoint.archived_evidence_receipts,
        "archived-receipt root",
    )?;
    validate_skipped_commitment_root(
        checkpoint.evidence_dependency_root.as_deref(),
        EVIDENCE_DEPENDENCY_DOMAIN,
        &checkpoint.evidence_dependencies,
        "evidence-dependency root",
    )?;
    validate_skipped_commitment_root(
        checkpoint.keyed_reservation_root.as_deref(),
        KEYED_RESERVATION_DOMAIN,
        &checkpoint.keyed_draw_reservations,
        "keyed-reservation root",
    )
}

impl SimulationCheckpoint {
    /// Returns every segment ID reachable from all retained checkpoint manifests.
    ///
    /// Hosts must include every checkpoint they still promise to restore before
    /// treating a stored segment absent from this set as an orphan candidate.
    pub fn reachable_archive_segment_ids(
        retained_checkpoints: &[Self],
    ) -> Result<BTreeSet<String>, CanwuError> {
        let mut reachable = BTreeSet::new();
        for checkpoint in retained_checkpoints {
            validate_compact_continuation(checkpoint)?;
            reachable.extend(
                checkpoint
                    .archived_segment_headers
                    .iter()
                    .map(|header| header.segment_id.clone()),
            );
        }
        Ok(reachable)
    }

    /// Returns sorted stored segment IDs that no retained manifest can reach.
    ///
    /// This method identifies host-GC candidates only; it never deletes content.
    pub fn orphaned_archive_segment_ids(
        retained_checkpoints: &[Self],
        stored_segment_ids: &[String],
    ) -> Result<Vec<String>, CanwuError> {
        let reachable = Self::reachable_archive_segment_ids(retained_checkpoints)?;
        Ok(stored_segment_ids
            .iter()
            .filter(|segment_id| !reachable.contains(segment_id.as_str()))
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect())
    }
}

fn compact_checkpoint_hash(checkpoint: &SimulationCheckpoint) -> Result<String, CanwuError> {
    validate_compact_continuation(checkpoint)?;
    super::canonical_hash(
        "canwu.compact-checkpoint.v1",
        &CompactCheckpointHashMaterial {
            state_checkpoint_hash: &checkpoint.state.checkpoint_hash,
            journal_end: checkpoint.journal_end,
            archived_segment_manifest_root: checkpoint.archived_segment_manifest_root.as_deref(),
            archived_receipt_root: checkpoint.archived_receipt_root.as_deref(),
            evidence_dependency_root: checkpoint.evidence_dependency_root.as_deref(),
            keyed_reservation_root: checkpoint.keyed_reservation_root.as_deref(),
        },
    )
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive: Option<EvidenceArchiveIndex>,
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
    committed_seal_tokens: BTreeSet<String>,
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

    /// Returns the committed receipt for an archived evidence identity.
    #[must_use]
    pub fn archived_evidence_receipt(
        &self,
        reference: &EvidenceRef,
    ) -> Option<&ArchivedEvidenceReceipt> {
        self.simulation
            .state
            .evidence
            .archived_evidence_receipts
            .get(reference)
    }

    /// Loads and fully verifies the segment containing archived evidence whose
    /// payload must be inspected.
    pub fn load_archived_evidence_segment(
        &self,
        reference: &EvidenceRef,
        provider: &dyn ArchiveProvider,
    ) -> Result<EvidenceJournalSegment, CanwuError> {
        let receipt = self.archived_evidence_receipt(reference).ok_or_else(|| {
            CanwuError::new(
                ErrorCode::EvidenceUnavailable,
                "no committed archive receipt exists for the requested evidence",
            )
        })?;
        load_verified_archived_evidence_segment(receipt, provider)
    }

    /// Seals and releases the current retained evidence tail.
    ///
    /// The runtime changes only after the segment is fully constructed and its
    /// continuation indexes are prepared. An empty retained tail returns
    /// `None`. The caller owns persistence and must keep all non-empty segments
    /// in exact cursor order for save restoration or replay.
    pub fn seal_evidence(&mut self) -> Result<Option<EvidenceJournalSegment>, CanwuError> {
        if self
            .simulation
            .evidence_dependencies()?
            .iter()
            .any(|dependency| dependency.requirement == EvidenceRequirement::PayloadRequired)
        {
            return Err(CanwuError::new(
                ErrorCode::ArchiveNotReady,
                "payload-required continuations must use prepare/store/commit sealing",
            ));
        }
        self.simulation.seal_retained_evidence()
    }

    /// Builds an immutable, content-addressed archive candidate.
    ///
    /// The returned segment must be durably stored before
    /// [`Self::commit_evidence_seal`] is called. Preparing never changes the
    /// live simulation.
    pub fn prepare_evidence_seal(&self) -> Result<Option<PreparedEvidenceSeal>, CanwuError> {
        let source_state_hash = self.simulation.authoritative_state_hash()?;
        let source_checkpoint_hash = compact_checkpoint_hash(&self.simulation.checkpoint()?)?;
        let source_end = self.simulation.evidence_cursor()?;
        let mut candidate = self.simulation.fork();
        let Some(segment) = candidate.seal_retained_evidence()? else {
            return Ok(None);
        };
        let segment_id = segment
            .archive
            .as_ref()
            .ok_or_else(|| archive_error("prepared segment has no archive index"))?
            .header
            .segment_id
            .clone();
        let target_checkpoint_hash = compact_checkpoint_hash(&candidate.checkpoint()?)?;
        let token_hash = super::canonical_hash(
            "canwu.evidence.seal-token.v1",
            &(
                &source_state_hash,
                &source_checkpoint_hash,
                source_end,
                &segment_id,
                &target_checkpoint_hash,
            ),
        )?;
        Ok(Some(PreparedEvidenceSeal {
            token: EvidenceSealToken {
                source_state_hash,
                source_checkpoint_hash,
                source_end,
                segment_id,
                target_checkpoint_hash,
                token_hash,
            },
            segment,
        }))
    }

    /// Atomically commits a previously prepared segment after reading the
    /// exact stored bytes back through the archive provider.
    pub fn commit_evidence_seal(
        &mut self,
        token: &EvidenceSealToken,
        provider: &dyn ArchiveProvider,
    ) -> Result<(), CanwuError> {
        if self.committed_seal_tokens.contains(&token.token_hash) {
            return Ok(());
        }
        let expected_token_hash = super::canonical_hash(
            "canwu.evidence.seal-token.v1",
            &(
                &token.source_state_hash,
                &token.source_checkpoint_hash,
                token.source_end,
                &token.segment_id,
                &token.target_checkpoint_hash,
            ),
        )?;
        if expected_token_hash != token.token_hash {
            return Err(archive_error("evidence seal token hash is invalid"));
        }
        if self.simulation.authoritative_state_hash()? != token.source_state_hash
            || compact_checkpoint_hash(&self.simulation.checkpoint()?)?
                != token.source_checkpoint_hash
            || self.simulation.evidence_cursor()? != token.source_end
        {
            return Err(CanwuError::new(
                ErrorCode::StaleSealToken,
                "evidence seal token no longer names the live source cut",
            ));
        }
        let stored = provider
            .load_evidence_segment(&token.segment_id)?
            .ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::ArchiveNotReady,
                    "prepared evidence segment is not available from the archive provider",
                )
            })?;
        let archive = stored
            .archive
            .as_ref()
            .ok_or_else(|| archive_error("stored evidence segment has no archive index"))?;
        if archive.header.segment_id != token.segment_id {
            return Err(archive_error(
                "stored evidence segment ID does not match the seal token",
            ));
        }
        verify_archived_segment(&stored)?;

        let before = self.simulation.fork();
        let result = (|| {
            let actual = self.simulation.seal_retained_evidence()?.ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::StaleSealToken,
                    "the prepared evidence tail is no longer retained",
                )
            })?;
            if actual != stored {
                return Err(archive_error(
                    "stored evidence segment differs from the prepared live cut",
                ));
            }
            self.simulation
                .validate_payload_required_archive(provider)?;
            if compact_checkpoint_hash(&self.simulation.checkpoint()?)?
                != token.target_checkpoint_hash
            {
                return Err(archive_error(
                    "committed checkpoint hash differs from the seal token",
                ));
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.simulation = before;
            return Err(error);
        }
        self.committed_seal_tokens.insert(token.token_hash.clone());
        Ok(())
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
    pub const fn decision_state(&self) -> &super::DecisionState {
        self.simulation.decision_state()
    }

    #[must_use]
    pub fn decision_ticket(&self, id: super::DecisionTicketId) -> Option<&super::DecisionTicket> {
        self.simulation.decision_ticket(id)
    }

    #[must_use]
    pub fn decision_traces(&self) -> &[super::DecisionTrace] {
        self.simulation.decision_traces()
    }

    #[must_use]
    pub fn decision_attempts(&self) -> &[super::DecisionAttemptRecord] {
        self.simulation.decision_attempts()
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

    pub fn prepare_decision(
        &self,
        decision_request_id: super::DecisionRequestId,
        command_request_id: Option<super::CommandRequestId>,
        ticket_id: super::DecisionTicketId,
        policy: &dyn super::DecisionPolicy,
    ) -> Result<super::DecisionEvaluation, CanwuError> {
        self.simulation
            .prepare_decision(decision_request_id, command_request_id, ticket_id, policy)
    }

    pub fn prepare_decision_at(
        &self,
        due_at: super::SimTime,
        decision_request_id: super::DecisionRequestId,
        command_request_id: Option<super::CommandRequestId>,
        ticket_id: super::DecisionTicketId,
        policy: &dyn super::DecisionPolicy,
    ) -> Result<super::DecisionEvaluation, CanwuError> {
        self.simulation.prepare_decision_at(
            due_at,
            decision_request_id,
            command_request_id,
            ticket_id,
            policy,
        )
    }

    pub fn enqueue_decision(
        &mut self,
        due_at: super::SimTime,
        priority: i32,
        request: super::DecisionIngressRequest,
    ) -> Result<IngressReceipt, CanwuError> {
        self.simulation.enqueue_decision(due_at, priority, request)
    }

    pub fn drive_decision(
        &mut self,
        due_at: super::SimTime,
        priority: i32,
        decision_request_id: super::DecisionRequestId,
        command_request_id: Option<super::CommandRequestId>,
        ticket_id: super::DecisionTicketId,
        policy: &dyn super::DecisionPolicy,
    ) -> Result<super::DecisionEvaluation, CanwuError> {
        self.simulation.drive_decision(
            due_at,
            priority,
            decision_request_id,
            command_request_id,
            ticket_id,
            policy,
        )
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
        Ok(CompactedSimulation {
            simulation: self,
            committed_seal_tokens: BTreeSet::new(),
        })
    }

    fn evidence_dependencies(&self) -> Result<Vec<EvidenceDependency>, CanwuError> {
        let mut dependencies = BTreeMap::new();
        let identity = EvidenceRequirement::IdentityOnly;

        for records in self.state.current.knowledge.records.values() {
            for record in records.values() {
                for reference in &record.origin.evidence {
                    promote_dependency(&mut dependencies, reference.clone(), identity);
                }
            }
        }

        for record in self.state.current.domain_records.values() {
            let schema = self
                .plugins
                .record_schemas
                .get(&record.reference.kind)
                .map(|(_, schema)| schema)
                .ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::ArchiveNotReady,
                        format!(
                            "current domain record {} has no registered schema",
                            record.reference
                        ),
                    )
                })?;
            add_payload_required_continuation_dependencies(&mut dependencies, record, schema)?;
            let retained = self
                .state
                .evidence
                .boundaries
                .iter()
                .rev()
                .find_map(|boundary| {
                    boundary
                        .record_changes
                        .iter()
                        .enumerate()
                        .find(|(_, change)| {
                            change.current.reference == record.reference
                                && change.current.version == record.version
                        })
                        .map(|(change_index, _)| {
                            EvidenceRef::DomainRecordVersion(DomainRecordVersionRef {
                                record: record.reference.clone(),
                                version: record.version,
                                established_by: DomainRecordVersionSource::BoundaryChange {
                                    boundary: boundary.id,
                                    change_index: change_index as u64,
                                },
                            })
                        })
                });
            let archived = self
                .state
                .evidence
                .archived_evidence_receipts
                .keys()
                .filter(|reference| {
                    matches!(
                        reference,
                        EvidenceRef::DomainRecordVersion(version)
                            if version.record == record.reference && version.version == record.version
                    )
                })
                .cloned()
                .collect::<Vec<_>>();
            if archived.len() > 1 {
                return Err(CanwuError::new(
                    ErrorCode::ArchiveNotReady,
                    format!(
                        "current domain record {} has ambiguous archived version provenance",
                        record.reference
                    ),
                ));
            }
            if let Some(reference) = retained.or_else(|| archived.into_iter().next()) {
                promote_dependency(&mut dependencies, reference, identity);
                continue;
            }
            let initial = self.bound_initial_scenario().is_some_and(|scenario| {
                scenario.domain_records.iter().any(|initial| {
                    initial.reference == record.reference && initial.version == record.version
                })
            });
            if !initial {
                return Err(CanwuError::new(
                    ErrorCode::ArchiveNotReady,
                    format!(
                        "current domain record {} has no retained, archived, or scenario version provenance",
                        record.reference
                    ),
                ));
            }
        }

        for reservation in &self.state.evidence.keyed_draw_reservations {
            promote_dependency(
                &mut dependencies,
                reservation.operation_evidence.clone(),
                identity,
            );
            promote_dependency(
                &mut dependencies,
                reservation.draw_receipt.evidence.clone(),
                identity,
            );
        }
        for draw in &self.state.evidence.random_draws {
            if matches!(draw.address, RandomDrawAddress::OperationV1(_)) {
                let reference = draw.operation_evidence.clone().ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::ArchiveNotReady,
                        "operation-keyed retained draw is missing its operation evidence",
                    )
                })?;
                promote_dependency(&mut dependencies, reference, identity);
                promote_dependency(
                    &mut dependencies,
                    EvidenceRef::RandomDraw(draw.id),
                    identity,
                );
            }
        }

        for archived in self.state.evidence.archived_command_requests.values() {
            add_command_outcome_dependencies(&mut dependencies, &archived.outcome);
        }
        for archived in self.state.evidence.archived_ingress_requests.values() {
            promote_dependency(
                &mut dependencies,
                EvidenceRef::Ingress(archived.receipt.ingress_id),
                identity,
            );
        }
        for archived in self.state.evidence.archived_decision_requests.values() {
            promote_dependency(
                &mut dependencies,
                EvidenceRef::Ingress(archived.receipt.ingress_id),
                identity,
            );
        }
        for attempt in &self.state.evidence.command_attempts {
            if attempt.request_id.is_none() {
                continue;
            }
            promote_dependency(
                &mut dependencies,
                EvidenceRef::CommandAttempt(attempt.id),
                identity,
            );
            if let CommandAttemptOutcome::Accepted { command_id } = attempt.outcome {
                promote_dependency(
                    &mut dependencies,
                    EvidenceRef::Command(command_id),
                    identity,
                );
                if let Some(command) = self
                    .state
                    .evidence
                    .commands
                    .iter()
                    .find(|command| command.id == command_id)
                {
                    for event in &command.emitted_events {
                        promote_dependency(&mut dependencies, EvidenceRef::Event(*event), identity);
                    }
                }
            }
        }
        for ingress in &self.state.evidence.ingress {
            if matches!(
                ingress.payload,
                IngressPayload::Command { .. } | IngressPayload::Decision { .. }
            ) {
                promote_dependency(
                    &mut dependencies,
                    EvidenceRef::Ingress(ingress.id),
                    identity,
                );
            }
        }

        for action in self.state.scheduler.actions.values() {
            match action {
                ScheduledAction::ArmyArrival { order_event, .. } => promote_dependency(
                    &mut dependencies,
                    EvidenceRef::Event(*order_event),
                    identity,
                ),
                ScheduledAction::KnowledgeReport { dispatch_event, .. } => promote_dependency(
                    &mut dependencies,
                    EvidenceRef::Event(*dispatch_event),
                    identity,
                ),
                ScheduledAction::PluginDirective { cause, .. } => {
                    add_cause_dependency(&mut dependencies, cause);
                }
            }
        }

        Ok(dependencies
            .into_iter()
            .map(|(reference, requirement)| EvidenceDependency {
                reference,
                requirement,
            })
            .collect())
    }

    fn validate_payload_required_archive(
        &self,
        provider: &dyn ArchiveProvider,
    ) -> Result<(), CanwuError> {
        for dependency in self
            .evidence_dependencies()?
            .into_iter()
            .filter(|dependency| dependency.requirement == EvidenceRequirement::PayloadRequired)
        {
            let receipt = self
                .state
                .evidence
                .archived_evidence_receipts
                .get(&dependency.reference)
                .ok_or_else(|| {
                    CanwuError::new(
                        ErrorCode::ArchiveNotReady,
                        "payload-required evidence has no committed archive receipt",
                    )
                })?;
            load_verified_archived_evidence_segment(receipt, provider)?;
        }
        Ok(())
    }

    fn ensure_retained_evidence_is_sealable(&self) -> Result<(), CanwuError> {
        if !self.state.scheduler.pending_ingress.is_empty() {
            return Err(CanwuError::new(
                ErrorCode::ArchiveNotReady,
                "live evidence can be sealed only when the canonical ingress queue is empty",
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
        let dependencies = self.evidence_dependencies()?;
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
            let mut archived_decision_requests = Vec::new();
            let mut archived_decision_command_requests = Vec::new();
            for record in &self.state.evidence.ingress {
                let receipt = IngressReceipt {
                    ingress_id: record.id,
                    issued_at: record.issued_at,
                    due_at: record.due_at,
                };
                match &record.payload {
                    IngressPayload::Command { request } => {
                        archived_ingress_requests.push((
                            request.request_id,
                            ArchivedIngressRequest {
                                input_hash: super::canonical_hash(
                                    "canwu.archive.ingress.command.v1",
                                    &(record.due_at, record.priority, request.as_ref()),
                                )?,
                                receipt,
                            },
                        ));
                    }
                    IngressPayload::Decision { request } => {
                        if let Some(command) = &request.command {
                            archived_decision_command_requests.push(command.request_id);
                        }
                        archived_decision_requests.push((
                            request.request_id,
                            ArchivedIngressRequest {
                                input_hash: super::canonical_hash(
                                    "canwu.ingress.decision-request.v1",
                                    &(record.due_at, record.priority, request.as_ref()),
                                )?,
                                receipt,
                            },
                        ));
                    }
                    IngressPayload::Plugin { .. } | IngressPayload::Calendar { .. } => {}
                }
            }
            Ok::<_, CanwuError>((
                archived_command_requests,
                archived_ingress_requests,
                archived_decision_requests,
                archived_decision_command_requests,
            ))
        })();
        let (
            archived_command_requests,
            archived_ingress_requests,
            archived_decision_requests,
            archived_decision_command_requests,
        ) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                self.state.metadata.checkpoint_hash = checkpoint_hash;
                self.state.metadata.commitment_roots = commitment_roots;
                self.state.metadata.commitment_cache = commitment_cache;
                return Err(error);
            }
        };

        let mut segment = EvidenceJournalSegment {
            format_version: CHECKPOINT_JOURNAL_FORMAT_VERSION,
            start,
            end,
            events: self.state.evidence.events.clone(),
            commands: self.state.evidence.commands.clone(),
            command_attempts: self.state.evidence.command_attempts.clone(),
            ingress: self.state.evidence.ingress.clone(),
            boundaries: self.state.evidence.boundaries.clone(),
            random_draws: self.state.evidence.random_draws.clone(),
            archive: None,
        };
        let (archive, receipts) = evidence_archive_index(&segment)?;
        segment.archive = Some(archive.clone());
        verify_archived_segment(&segment)?;

        let receipt_map: BTreeMap<_, _> = receipts
            .iter()
            .cloned()
            .map(|receipt| (receipt.evidence.clone(), receipt))
            .collect();
        let mut reservations = Vec::new();
        for draw in &segment.random_draws {
            let RandomDrawAddress::OperationV1(address) = &draw.address else {
                continue;
            };
            let operation_evidence = draw.operation_evidence.clone().ok_or_else(|| {
                archive_error("operation-keyed archived draw is missing operation evidence")
            })?;
            let draw_receipt = receipt_map
                .get(&EvidenceRef::RandomDraw(draw.id))
                .cloned()
                .ok_or_else(|| {
                    archive_error("operation-keyed archived draw is missing its receipt")
                })?;
            reservations.push(KeyedDrawReservation {
                stream: draw.stream.clone(),
                address: address.clone(),
                upper_exclusive: draw.upper_exclusive,
                purpose_hash: super::random::purpose_hash_hex_v1(&draw.purpose)?,
                result: draw.value,
                draw_id: draw.id,
                operation_evidence,
                draw_receipt,
            });
        }
        reservations.sort_by(|left, right| {
            (&left.stream, &left.address).cmp(&(&right.stream, &right.address))
        });

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
        self.state
            .evidence
            .archived_decision_requests
            .extend(archived_decision_requests);
        self.state
            .evidence
            .archived_decision_command_requests
            .extend(archived_decision_command_requests);
        self.state.evidence.archived = end;
        self.state.evidence.events.clear();
        self.state.evidence.commands.clear();
        self.state.evidence.command_attempts.clear();
        self.state.evidence.ingress.clear();
        self.state.evidence.boundaries.clear();
        self.state.evidence.random_draws.clear();
        self.state
            .evidence
            .archived_segment_headers
            .push(archive.header);
        for receipt in receipts {
            if self
                .state
                .evidence
                .archived_evidence_receipts
                .insert(receipt.evidence.clone(), receipt)
                .is_some()
            {
                return Err(archive_error("archived evidence receipt was duplicated"));
            }
        }
        self.state
            .evidence
            .keyed_draw_reservations
            .extend(reservations);
        self.state
            .evidence
            .keyed_draw_reservations
            .sort_by(|left, right| {
                (&left.stream, &left.address).cmp(&(&right.stream, &right.address))
            });
        if self
            .state
            .evidence
            .keyed_draw_reservations
            .windows(2)
            .any(|reservations| {
                (&reservations[0].stream, &reservations[0].address)
                    == (&reservations[1].stream, &reservations[1].address)
            })
        {
            return Err(archive_error("archived keyed reservation was duplicated"));
        }
        retain_reachable_archived_evidence_receipts(
            &mut self.state.evidence.archived_evidence_receipts,
            &dependencies,
            &self.state.evidence.keyed_draw_reservations,
        );
        for dependency in dependencies
            .iter()
            .filter(|dependency| dependency.requirement == EvidenceRequirement::PayloadRequired)
        {
            if !self
                .state
                .evidence
                .archived_evidence_receipts
                .contains_key(&dependency.reference)
            {
                return Err(CanwuError::new(
                    ErrorCode::ArchiveNotReady,
                    "payload-required evidence was not present in the sealed archive prefix",
                ));
            }
        }
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
            decisions: self.state.current.decisions.clone(),
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
            next_knowledge_record_id: self.state.counters.next_knowledge_record_id,
            next_schedule_sequence: self.state.counters.next_schedule_sequence,
            next_correlation_id: self.state.counters.next_correlation_id,
            next_decision_trace_id: self.state.counters.next_decision_trace_id,
        }
    }

    /// Returns the current monotonic cut through every append-only journal.
    pub fn evidence_cursor(&self) -> Result<EvidenceCursor, CanwuError> {
        EvidenceCursor::from_evidence(&self.state.evidence)
    }

    /// Captures current authoritative state without cloning accumulated evidence.
    pub fn checkpoint(&self) -> Result<SimulationCheckpoint, CanwuError> {
        let archived_segment_headers = self.state.evidence.archived_segment_headers.clone();
        let evidence_dependencies = self.evidence_dependencies()?;
        let mut keyed_draw_reservations = self.state.evidence.keyed_draw_reservations.clone();
        keyed_draw_reservations.sort_by(|left, right| {
            (&left.stream, &left.address).cmp(&(&right.stream, &right.address))
        });
        let required_receipts =
            required_archived_receipt_references(&evidence_dependencies, &keyed_draw_reservations);
        let archived_evidence_receipts: Vec<_> = self
            .state
            .evidence
            .archived_evidence_receipts
            .iter()
            .filter(|(reference, _)| required_receipts.contains(*reference))
            .map(|(_, receipt)| receipt.clone())
            .collect();
        let checkpoint = SimulationCheckpoint {
            format_version: CHECKPOINT_JOURNAL_FORMAT_VERSION,
            journal_end: self.evidence_cursor()?,
            state: self.checkpoint_state(),
            archived_segment_manifest_root: skipped_commitment_root(
                ARCHIVED_SEGMENT_MANIFEST_DOMAIN,
                &archived_segment_headers,
            )?,
            archived_segment_headers,
            archived_receipt_root: skipped_commitment_root(
                ARCHIVED_RECEIPT_DOMAIN,
                &archived_evidence_receipts,
            )?,
            archived_evidence_receipts,
            evidence_dependency_root: skipped_commitment_root(
                EVIDENCE_DEPENDENCY_DOMAIN,
                &evidence_dependencies,
            )?,
            evidence_dependencies,
            keyed_reservation_root: skipped_commitment_root(
                KEYED_RESERVATION_DOMAIN,
                &keyed_draw_reservations,
            )?,
            keyed_draw_reservations,
        };
        validate_compact_continuation(&checkpoint)?;
        Ok(checkpoint)
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
            archive: None,
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
        validate_compact_continuation(&checkpoint)?;
        let expected_headers = checkpoint.archived_segment_headers;
        let expected_receipts = checkpoint.archived_evidence_receipts;
        let expected_dependencies = checkpoint.evidence_dependencies;
        let expected_reservations = checkpoint.keyed_draw_reservations;
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
        let mut rebuilt_headers = Vec::new();
        let mut rebuilt_receipts = BTreeMap::new();
        let mut rebuilt_reservations = Vec::new();
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
            if let Some(archive) = &segment.archive {
                let receipts = verify_archived_segment(&segment).map_err(|error| {
                    invalid_snapshot_error(format!(
                        "archived evidence segment is invalid: {}",
                        error.message
                    ))
                })?;
                rebuilt_headers.push(archive.header.clone());
                for receipt in receipts {
                    if rebuilt_receipts
                        .insert(receipt.evidence.clone(), receipt)
                        .is_some()
                    {
                        return Err(invalid_snapshot_error(
                            "archived evidence segments contain duplicate receipts",
                        ));
                    }
                }
                for draw in &segment.random_draws {
                    let RandomDrawAddress::OperationV1(address) = &draw.address else {
                        continue;
                    };
                    let operation_evidence = draw.operation_evidence.clone().ok_or_else(|| {
                        invalid_snapshot_error("archived keyed draw is missing operation evidence")
                    })?;
                    let draw_receipt = rebuilt_receipts
                        .get(&EvidenceRef::RandomDraw(draw.id))
                        .cloned()
                        .ok_or_else(|| {
                            invalid_snapshot_error("archived keyed draw receipt is missing")
                        })?;
                    rebuilt_reservations.push(KeyedDrawReservation {
                        stream: draw.stream.clone(),
                        address: address.clone(),
                        upper_exclusive: draw.upper_exclusive,
                        purpose_hash: super::random::purpose_hash_hex_v1(&draw.purpose)?,
                        result: draw.value,
                        draw_id: draw.id,
                        operation_evidence,
                        draw_receipt,
                    });
                }
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
        rebuilt_reservations.sort_by(|left, right| {
            (&left.stream, &left.address).cmp(&(&right.stream, &right.address))
        });
        let rebuilt_dependencies =
            Simulation::from_snapshot(snapshot.clone())?.evidence_dependencies()?;
        retain_reachable_archived_evidence_receipts(
            &mut rebuilt_receipts,
            &rebuilt_dependencies,
            &rebuilt_reservations,
        );
        let rebuilt_receipts: Vec<_> = rebuilt_receipts.into_values().collect();
        if rebuilt_headers != expected_headers
            || rebuilt_receipts != expected_receipts
            || rebuilt_dependencies != expected_dependencies
            || rebuilt_reservations != expected_reservations
        {
            return Err(invalid_snapshot_error(
                "checkpoint compact continuation does not match its reconstructed authoritative indexes",
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
        let bundle = super::legacy_v4::deserialize_checkpoint_journal_json(json)?;
        Self::from_checkpoint_journal(bundle)
    }

    /// Deserializes a bundle and rehydrates its exact plugin contracts.
    pub fn from_checkpoint_journal_json_with_plugins(
        json: &str,
        plugins: &[&dyn SimulationPlugin],
    ) -> Result<Self, CanwuError> {
        let bundle = super::legacy_v4::deserialize_checkpoint_journal_json(json)?;
        Self::from_checkpoint_journal_with_plugins(bundle, plugins)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unnecessary_literal_bound, clippy::unnecessary_wraps)]
    use super::*;
    use crate::{
        BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal,
        BoundarySystemContract, Command, CommandContext, CommandRequestId, DomainRecordClass,
        DomainRecordDraft, DomainRecordMutation, Issuer, KnowledgeHolderRef, KnowledgeOrigin,
        KnowledgeRecordDraft, KnowledgeRecordKind, KnowledgeSchemaId, KnowledgeWriteGrant,
        PluginActionDescriptor, PluginKnowledgeSchema, PluginRegistrar, SimulationView, StateKey,
        StateVisibility, SystemDirective, demo_scenario,
    };
    use canwu_core::{BoundaryId, CommandId, DomainRecordKind, PersonId};
    use serde_json::{Map, Value, json};
    use std::cell::RefCell;

    #[derive(Default)]
    struct TestArchive {
        segments: RefCell<BTreeMap<String, EvidenceJournalSegment>>,
    }

    impl TestArchive {
        fn segment_ids(&self) -> Vec<String> {
            self.segments.borrow().keys().cloned().collect()
        }
    }

    impl ArchiveProvider for TestArchive {
        fn load_evidence_segment(
            &self,
            segment_id: &str,
        ) -> Result<Option<EvidenceJournalSegment>, CanwuError> {
            Ok(self.segments.borrow().get(segment_id).cloned())
        }
    }

    impl ArchiveStore for TestArchive {
        fn store_evidence_segment(
            &self,
            segment: &EvidenceJournalSegment,
        ) -> Result<ArchiveStoreOutcome, CanwuError> {
            let segment_id = segment
                .archive
                .as_ref()
                .ok_or_else(|| archive_error("test archive segment has no index"))?
                .header
                .segment_id
                .clone();
            let mut segments = self.segments.borrow_mut();
            if let Some(existing) = segments.get(&segment_id) {
                return if existing == segment {
                    Ok(ArchiveStoreOutcome::AlreadyPresent)
                } else {
                    Err(archive_error(
                        "content-addressed test segment ID has conflicting bytes",
                    ))
                };
            }
            segments.insert(segment_id, segment.clone());
            Ok(ArchiveStoreOutcome::Stored)
        }
    }

    fn archived_identity_schema() -> KnowledgeSchemaId {
        KnowledgeSchemaId::new(
            KnowledgeRecordKind::new("fixture.archive", "archived_command_notice"),
            1,
        )
    }

    #[allow(clippy::unnecessary_wraps)]
    fn retain_archive_source_command(
        _view: &SimulationView<'_>,
        _context: &CommandContext,
        _payload: &Value,
    ) -> Result<Vec<SystemDirective>, CanwuError> {
        Ok(Vec::new())
    }

    #[allow(clippy::unnecessary_wraps)]
    fn publish_archived_command_identity(
        _view: &SimulationView<'_>,
        context: &BoundaryContext,
    ) -> Result<BoundaryProposal, CanwuError> {
        let record = KnowledgeRecordDraft {
            schema: archived_identity_schema(),
            subjects: Vec::new(),
            payload: json!({ "boundary": context.boundary_id.get() }),
            as_of: None,
            confidence_per_mille: 1_000,
            origin: KnowledgeOrigin {
                method: "archived_command_identity_v1".to_owned(),
                evidence: vec![EvidenceRef::Command(CommandId::new(1))],
            },
            supersedes: Vec::new(),
            contradicts: Vec::new(),
        };
        Ok(BoundaryProposal {
            directives: vec![BoundaryDirective::PublishKnowledge {
                holder: KnowledgeHolderRef::Person(PersonId::new(1)),
                visibility: StateVisibility::SameBoundary,
                producer_correlation: Some(format!(
                    "archived-command-boundary-{}",
                    context.boundary_id.get()
                )),
                records: vec![record],
                summary: "Publish knowledge from a retained or archived command identity"
                    .to_owned(),
            }],
            ..BoundaryProposal::default()
        })
    }

    struct ArchivedIdentityPublicationPlugin;

    impl SimulationPlugin for ArchivedIdentityPublicationPlugin {
        fn name(&self) -> &str {
            "fixture-archived-identity-publication"
        }

        fn version(&self) -> &str {
            "test-v1"
        }

        fn semantic_hash(&self) -> &str {
            "7100000000000000000000000000000000000000000000000000000000000000"
        }

        fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
            registrar.register_knowledge_schema(PluginKnowledgeSchema {
                id: archived_identity_schema(),
                schema_hash: "7200000000000000000000000000000000000000000000000000000000000000"
                    .to_owned(),
                writable: true,
                payload_schema: PayloadSchema::Any,
                subjects: Vec::new(),
            })?;
            registrar.register_command(
                PluginActionDescriptor {
                    name: "retain_archive_source_v1".to_owned(),
                    description: "Persist one neutral command identity for archive testing"
                        .to_owned(),
                    payload_schema: PayloadSchema::Any,
                    reads: Vec::new(),
                    writes: Vec::new(),
                },
                retain_archive_source_command,
            )?;
            let mut publisher = BoundarySystemContract::new(
                "publish-archived-command-identity",
                BoundaryPhase::PerspectiveAndReportMaterialization,
                SystemCadence::Daily,
            );
            publisher.knowledge_writes = vec![KnowledgeWriteGrant {
                schema: archived_identity_schema(),
                visibilities: vec![StateVisibility::SameBoundary],
            }];
            registrar.register_boundary_system(publisher, publish_archived_command_identity)
        }
    }

    fn archived_identity_two_segment_fixture() -> (SimulationCheckpoint, Vec<EvidenceJournalSegment>)
    {
        let (scenario, _) = demo_scenario();
        let plugin = ArchivedIdentityPublicationPlugin;
        let mut simulation = Simulation::new(711, scenario).expect("fixture scenario should load");
        simulation
            .register_plugin(&plugin)
            .expect("archived-identity plugin should register");
        simulation
            .enqueue_command(
                SimTime::EPOCH,
                0,
                CommandRequest::new(
                    CommandRequestId::new(1),
                    simulation.revision(),
                    CommandEnvelope::new(
                        Issuer::System("archive-identity-fixture".to_owned()),
                        Command::Plugin {
                            plugin: plugin.name().to_owned(),
                            command: "retain_archive_source_v1".to_owned(),
                            payload: json!({ "format_version": 1 }),
                        },
                    )
                    .at_time(SimTime::EPOCH),
                ),
            )
            .expect("archive source command should queue");
        simulation
            .settle_boundary(BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily))
            .expect("boundary one should retain the command-backed publication");
        simulation
            .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
            .expect("the following cut should admit boundary-one events before sealing");

        let mut compact = simulation
            .into_compacted()
            .expect("the fixture should enter compact mode");
        let first_segment = compact
            .seal_evidence()
            .expect("boundary-one evidence should seal")
            .expect("boundary one should produce an archive segment");
        let first_checkpoint = compact.checkpoint().expect("checkpoint one should build");
        assert!(
            first_checkpoint
                .archived_evidence_receipts
                .iter()
                .any(|receipt| receipt.evidence == EvidenceRef::Command(CommandId::new(1)))
        );

        let mut restored = CompactedSimulation::from_checkpoint_and_journal_with_plugins(
            first_checkpoint,
            vec![first_segment.clone()],
            &[&plugin],
        )
        .expect("checkpoint one should restore with its exact archive prefix");
        let restored_prefix = restored
            .seal_evidence()
            .expect("restored boundary-one evidence should reseal")
            .expect("restored boundary-one evidence should remain non-empty");
        assert_eq!(restored_prefix, first_segment);
        assert!(
            restored
                .archived_evidence_receipt(&EvidenceRef::Command(CommandId::new(1)))
                .is_some(),
            "the second publication must consume an archived identity, not retained payload"
        );
        let archived_reads = [StateKey::core_commands()];
        let archived_view = restored
            .simulation
            .plugin_view("archive-identity-probe", &archived_reads);
        let error = archived_view
            .command(CommandId::new(1))
            .expect_err("archived identity must not expose retained command payload");
        assert_eq!(error.code, ErrorCode::EvidenceContentUnavailable);
        restored
            .settle_boundary(
                BoundaryRequest::at(SimTime::EPOCH + SimDuration::days(1))
                    .with_cadence(SystemCadence::Daily),
            )
            .expect("boundary two should accept the archived command identity");
        let holder = KnowledgeHolderRef::Person(PersonId::new(1));
        let records = restored
            .knowledge()
            .for_holder(&holder)
            .expect("the holder should have both publications");
        assert_eq!(records.len(), 2);
        assert!(records.values().all(|record| {
            record.origin.evidence == vec![EvidenceRef::Command(CommandId::new(1))]
        }));
        restored
            .settle_boundary(BoundaryRequest::at(SimTime::EPOCH + SimDuration::days(1)))
            .expect("the following cut should admit boundary-two events before sealing");

        let second_segment = restored
            .seal_evidence()
            .expect("boundary-two evidence should seal")
            .expect("boundary two should produce an archive segment");
        let second_checkpoint = restored.checkpoint().expect("checkpoint two should build");
        (second_checkpoint, vec![restored_prefix, second_segment])
    }

    #[test]
    fn compact_restore_preserves_archived_identity_and_rejects_noncontiguous_segments() {
        let plugin = ArchivedIdentityPublicationPlugin;
        let (checkpoint, segments) = archived_identity_two_segment_fixture();
        let restored = CompactedSimulation::from_checkpoint_and_journal_with_plugins(
            checkpoint.clone(),
            segments.clone(),
            &[&plugin],
        )
        .expect("the complete two-segment archive should restore");
        assert_eq!(
            restored
                .knowledge()
                .for_holder(&KnowledgeHolderRef::Person(PersonId::new(1)))
                .expect("published holder ledger")
                .len(),
            2
        );

        let first = segments[0].clone();
        let second = segments[1].clone();
        for (label, tampered) in [
            ("omission", vec![first.clone()]),
            (
                "overlap",
                vec![first.clone(), first.clone(), second.clone()],
            ),
            ("reorder", vec![second, first]),
        ] {
            let error = Simulation::from_checkpoint_and_journal(checkpoint.clone(), tampered)
                .err()
                .expect(label);
            assert_eq!(error.code, ErrorCode::InvalidSnapshot, "{label}");
        }
    }

    struct PayloadContinuationPlugin;

    fn continuation_record_ref() -> DomainRecordRef {
        DomainRecordRef {
            kind: DomainRecordKind::new("fixture.archive", "payload_continuation"),
            id: "primary".to_owned(),
        }
    }

    fn continuation_payload(continuation: PayloadRequiredEvidenceContinuationV1) -> Value {
        Value::Object(Map::from_iter([(
            PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD.to_owned(),
            serde_json::to_value(continuation).expect("fixture continuation should encode"),
        )]))
    }

    fn mutate_payload_continuation(
        _view: &SimulationView<'_>,
        context: &BoundaryContext,
    ) -> Result<BoundaryProposal, CanwuError> {
        let directive = match context.boundary_id.get() {
            1 => Some(BoundaryDirective::MutateRecord {
                mutation: DomainRecordMutation::Create {
                    record: DomainRecordDraft::new(
                        continuation_record_ref(),
                        continuation_payload(PayloadRequiredEvidenceContinuationV1::active(vec![
                            EvidenceRef::Boundary(BoundaryId::new(1)),
                        ])),
                    ),
                },
                summary: "Create an active payload continuation".to_owned(),
            }),
            3 => Some(BoundaryDirective::MutateRecord {
                mutation: DomainRecordMutation::Update {
                    record: DomainRecordDraft::new(
                        continuation_record_ref(),
                        continuation_payload(PayloadRequiredEvidenceContinuationV1::completed()),
                    ),
                    expected_version: 1,
                },
                summary: "Complete the payload continuation".to_owned(),
            }),
            _ => None,
        };
        Ok(BoundaryProposal {
            directives: directive.into_iter().collect(),
            ..BoundaryProposal::default()
        })
    }

    impl SimulationPlugin for PayloadContinuationPlugin {
        fn name(&self) -> &str {
            "fixture-payload-continuation"
        }

        fn version(&self) -> &str {
            "test-v1"
        }

        fn semantic_hash(&self) -> &str {
            "7000000000000000000000000000000000000000000000000000000000000000"
        }

        fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
            let mut properties = BTreeMap::new();
            properties.insert(
                PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD.to_owned(),
                payload_required_evidence_continuation_property_v1(),
            );
            let mut schema =
                DomainRecordSchema::new(continuation_record_ref().kind, DomainRecordClass::Record);
            schema.payload_schema = PayloadSchema::Object {
                properties,
                allow_additional: false,
            };
            let state = schema.state_key();
            registrar.register_record_schema(schema)?;
            let mut contract = BoundarySystemContract::new(
                "payload-continuation",
                BoundaryPhase::DomainDeltaProposal,
                SystemCadence::Daily,
            );
            contract.writes = vec![state];
            contract.visibility = StateVisibility::SameBoundary;
            registrar.register_boundary_system(contract, mutate_payload_continuation)
        }
    }

    fn payload_continuation_runtime() -> CompactedSimulation {
        let (scenario, _) = demo_scenario();
        let mut simulation = Simulation::new(701, scenario).expect("fixture scenario should load");
        simulation
            .register_plugin(&PayloadContinuationPlugin)
            .expect("payload-continuation plugin should register");
        simulation
            .settle_boundary(BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily))
            .expect("the active continuation should be created");
        simulation
            .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
            .expect("a later boundary should admit the record-change event");
        simulation
            .into_compacted()
            .expect("the fixture should enter compact mode")
    }

    fn store_prepared(
        compact: &CompactedSimulation,
        archive: &TestArchive,
    ) -> PreparedEvidenceSeal {
        let prepared = compact
            .prepare_evidence_seal()
            .expect("the fixture should prepare a seal")
            .expect("the retained tail should be non-empty");
        assert_eq!(
            archive
                .store_evidence_segment(&prepared.segment)
                .expect("the prepared segment should store"),
            ArchiveStoreOutcome::Stored
        );
        prepared
    }

    #[test]
    fn payload_required_receipts_are_exactly_reachable_and_prune_after_completion() {
        let mut compact = payload_continuation_runtime();
        assert_eq!(
            compact
                .seal_evidence()
                .expect_err("direct sealing must reject payload continuations")
                .code,
            ErrorCode::ArchiveNotReady
        );

        let archive = TestArchive::default();
        let first = store_prepared(&compact, &archive);
        compact
            .commit_evidence_seal(&first.token, &archive)
            .expect("provider-backed sealing should commit");
        let first_checkpoint = compact.checkpoint().expect("checkpoint should build");
        let first_references = first_checkpoint
            .archived_evidence_receipts
            .iter()
            .map(|receipt| receipt.evidence.clone())
            .collect::<BTreeSet<_>>();
        let first_dependencies = first_checkpoint
            .evidence_dependencies
            .iter()
            .map(|dependency| (dependency.reference.clone(), dependency.requirement))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            first_dependencies.get(&EvidenceRef::Boundary(BoundaryId::new(1))),
            Some(&EvidenceRequirement::PayloadRequired)
        );
        assert_eq!(first_references.len(), first_dependencies.len());
        assert_eq!(
            first_references,
            first_dependencies.keys().cloned().collect::<BTreeSet<_>>()
        );
        assert!(
            first
                .segment
                .archive
                .as_ref()
                .expect("prepared segment should have an archive index")
                .entries
                .len()
                > first_references.len(),
            "the full segment index must outlive the reachable-only receipt set"
        );

        compact
            .settle_boundary(BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily))
            .expect("the continuation should complete");
        compact
            .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
            .expect("a later boundary should admit the completion event");
        let second = store_prepared(&compact, &archive);
        compact
            .commit_evidence_seal(&second.token, &archive)
            .expect("the completed continuation should seal");
        let second_checkpoint = compact.checkpoint().expect("checkpoint should build");
        assert_eq!(second_checkpoint.archived_segment_headers.len(), 2);
        assert_eq!(second_checkpoint.archived_evidence_receipts.len(), 1);
        assert_eq!(second_checkpoint.evidence_dependencies.len(), 1);
        assert_eq!(
            second_checkpoint.archived_evidence_receipts[0].evidence,
            second_checkpoint.evidence_dependencies[0].reference
        );
        assert_eq!(
            second_checkpoint.evidence_dependencies[0].requirement,
            EvidenceRequirement::IdentityOnly
        );
        assert!(
            !second_checkpoint
                .archived_evidence_receipts
                .iter()
                .any(|receipt| receipt.evidence == EvidenceRef::Boundary(BoundaryId::new(1)))
        );

        Simulation::from_checkpoint_and_journal(
            second_checkpoint,
            vec![first.segment, second.segment],
        )
        .expect("reconstruction must filter full segment indexes to the compact receipt frontier");
    }

    #[test]
    fn payload_required_commit_fails_closed_when_an_older_segment_is_missing() {
        let mut compact = payload_continuation_runtime();
        let complete_archive = TestArchive::default();
        let first = store_prepared(&compact, &complete_archive);
        compact
            .commit_evidence_seal(&first.token, &complete_archive)
            .expect("the initial provider-backed seal should commit");

        compact
            .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
            .expect("a new tail should preserve the active continuation");
        let incomplete_archive = TestArchive::default();
        let second = store_prepared(&compact, &incomplete_archive);
        let before = compact.checkpoint().expect("checkpoint should build");
        let error = compact
            .commit_evidence_seal(&second.token, &incomplete_archive)
            .expect_err("the provider must retain every payload-required segment");
        assert_eq!(error.code, ErrorCode::EvidenceContentUnavailable);
        assert_eq!(
            compact.checkpoint().expect("checkpoint should build"),
            before
        );

        incomplete_archive
            .store_evidence_segment(&first.segment)
            .expect("restoring the older required segment should succeed");
        compact
            .commit_evidence_seal(&second.token, &incomplete_archive)
            .expect("the exact provider set should permit commit");
    }

    #[test]
    fn host_orphan_candidates_follow_all_retained_manifests_without_deleting() {
        let mut compact = payload_continuation_runtime();
        let archive = TestArchive::default();
        let prepared = store_prepared(&compact, &archive);
        let stored_ids = archive.segment_ids();
        let before = compact.checkpoint().expect("checkpoint should build");
        assert_eq!(
            SimulationCheckpoint::orphaned_archive_segment_ids(
                std::slice::from_ref(&before),
                &stored_ids,
            )
            .expect("manifest reachability should validate"),
            vec![prepared.token.segment_id.clone()]
        );
        assert!(
            archive
                .load_evidence_segment(&prepared.token.segment_id)
                .expect("the store should remain readable")
                .is_some(),
            "the conformance API must not delete host content"
        );

        compact
            .commit_evidence_seal(&prepared.token, &archive)
            .expect("the stored segment should commit");
        let after = compact.checkpoint().expect("checkpoint should build");
        assert!(
            SimulationCheckpoint::orphaned_archive_segment_ids(
                std::slice::from_ref(&after),
                &stored_ids,
            )
            .expect("committed manifest reachability should validate")
            .is_empty()
        );
        assert_eq!(
            SimulationCheckpoint::reachable_archive_segment_ids(&[before, after])
                .expect("all retained manifests should be scanned"),
            BTreeSet::from([prepared.token.segment_id.clone()])
        );
    }
}
