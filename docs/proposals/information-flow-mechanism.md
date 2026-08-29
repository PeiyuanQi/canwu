# Information Flow Mechanism

Status: detailed design revision 3 after independent review; implemented and in
the final verification/promotion gate tracked by
`information-flow-mechanism-todo.md`.

Baseline reconciled: Canwu `origin/main` at `2d337f4` on 2026-08-21. The
persisted decision framework and published society extension are part of
the baseline, including engine-issued decision-controller provenance,
plugin-owned domain records and projections, snapshot counters,
domain-separated commitments, exact replay, compaction reservations, and the
bilingual case-library navigation.

## Decision

Add two generic capabilities to the Canwu kernel:

1. plugins may atomically publish schema-versioned, holder-relative knowledge
   batches through a kernel-owned publication stage during the perception or
   perspective phases; and
2. systems that resolve independent uncertain operations may use a persisted
   operation-keyed random draw whose result is unaffected by unrelated draws.

Build content lineage, representations, instances, dispatches, per-recipient
delivery attempts, access, interpretation, audiences, and release as a
published authoritative `canwu-information` extension plugin that depends
only on the supported public API (`canwu-api`). The extension, not an application
plugin, owns the fixed information record namespace and is the only writer of
those records.

Keep source-specific people, governments, periods, conflicts, dates, places,
political consequences, reputation rules, intelligence formulas, and narrative
text outside Canwu. The two public cases use neutral role names and synthetic
data.

## Why this boundary

The reusable engine invariant is not a particular kind of document or signal.
It is:

1. authoritative information artifacts can move and branch;
2. access to an artifact is not the same as understanding it;
3. an interpretation is not the same as belief or truth;
4. different actors can hold incomplete or contradictory records;
5. observation or copying need not stop the intended delivery;
6. release makes information available to a scope but does not create universal
   knowledge;
7. every resulting holder-relative record must survive rollback, save/load,
   hashing, exact replay, and counterfactual forks.

Those guarantees belong in the engine. The meanings of particular media,
institutions, operations, audiences, and consequences do not.

## Hard gates

The implementation must preserve these existing Canwu contracts:

- All authoritative mutation enters through validated commands, canonical
  ingress, or phased boundary directives.
- Runtime handlers never receive mutable live state.
- Player- and agent-facing reads are holder-relative and cannot fall back to
  world state.
- `PublicObserver` has no authority to read a private holder knowledge ledger.
- Actor-facing knowledge projections never expose audit evidence IDs, hidden
  domain-record lineage, producer system identity, or another holder's data.
- Domain-specific information artifacts remain outside canwu-core,
  canwu-knowledge, and canwu-sim.
- A plugin may publish only knowledge kinds it registered and declared for the
  current boundary system.
- The engine stamps knowledge identity and learning time; plugins cannot
  backdate either value.
- Existing knowledge records are append-only. Corrections and contradictions
  add records and references instead of overwriting history.
- An access record never implies interpretation, knowledge, belief, truth, or
  public visibility; an interpretation performed for someone else also does
  not grant that person knowledge without an explicit result-access path.
- Interception never changes the original dispatch or delivery attempt unless a separate
  authoritative transition explicitly does so.
- Adding an observer or presentation consumer cannot change authoritative
  state, random draws, or delivery outcomes.
- Every new field is covered by validation, persistence, commitments, replay,
  rollback, compaction, and tamper tests.
- Information lifecycle invariants are enforced by the authoritative
  extension plugin. Pure helper functions are conveniences, not the security
  or consistency boundary.
- Every random outcome that must remain stable under unrelated work uses an
  operation-keyed draw address, not a shared sequential draw position.
- No shared implementation branches on a case ID, source period, expected
  narrative outcome, or media label.

## Layering

~~~text
Canwu kernel
  stable IDs
  registered and versioned knowledge schemas
  holder-relative knowledge ledger and safe projection
  kernel-owned knowledge publication stage
  operation-keyed deterministic random draws
  atomic settlement, evidence, hashing, persistence, replay

canwu-information (published crate)
  authoritative extension plugin and fixed schema owner
  information content
  representations and derivation lineage
  persistent instances
  dispatches and per-recipient delivery attempts
  access
  interpretations
  audience sets
  releases
  canonical commands and ingress
  mandatory lifecycle validation and mutation planning

case or application plugin
  channel timing and resource costs
  access, interception, and interpretation resolution policy
  audience membership resolution
  decisions and resource costs
  political, military, social, or reputation consequences

host / client
  controller selection
  user interface
  narration and localization
  optional external policy or language-model calls
~~~

Dependency direction is:

~~~text
canwu-core <- canwu-knowledge <- canwu-sim <- canwu-api <- canwu-information
                                                        <- application packages
~~~

`canwu-api` does not depend on or re-export the optional `canwu-information`
extension.

## Terminology

### Knowledge

A holder-relative assertion available to one person or one institutional/domain
entity. It is not guaranteed to be correct, complete, current, believed, or
acted upon. "Holder" is used instead of "principal" in the public API and
documentation because it describes the simulation concept directly.

### Content

The semantic payload carried by an information artifact. Content records what
an artifact says; it does not assert that the claims are true.

### Representation

A particular encoding or presentation of content. Several representations may
carry the same content. A derived representation may instead carry derived
content.

### Instance

A persistent physical or logical carrier of one representation. Some channels
have no persistent instance.

### Dispatch

One attempt to send or expose a representation through a channel. A dispatch
may have one or more delivery attempts, each with its own recipient and
outcome.

### Delivery attempt

The per-recipient progress and outcome of one dispatch. Partial delivery,
retry, relay, and different completion times are represented by separate
records rather than one aggregate status.

### Access

An authoritative record that an actor could inspect some portion of a
representation. Access does not prove comprehension.

### Interpretation

A result produced by a performer from one or more accesses by applying a
capability such as reading, decoding, translation, classification, or
analysis. The performer, the commissioning holder, and the eventual recipient
may be different. The result may be partial or wrong.

### Release

A persisted availability decision for a representation and an audience scope.
Release does not automatically create access or holder knowledge.

## Kernel design

### Stable identity

Add `KnowledgeRecordId` to `canwu-core` using the existing typed-u64 ID pattern.
The counter is global within one simulation, starts at one, and is never reused.

Add `KnowledgeRecordKind`, `KnowledgeSchemaId`, and `KnowledgeHolderRef`:

~~~rust
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct KnowledgeRecordKind {
    pub namespace: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct KnowledgeSchemaId {
    pub kind: KnowledgeRecordKind,
    pub version: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum KnowledgeHolderRef {
    Person(PersonId),
    Entity(EntityRef),
}

pub enum KnowledgeHolderPolicy {
    Disallowed,
    Allowed,
}
~~~

Both text components must be non-empty, trimmed canonical text. Schema version
zero is invalid. `KnowledgeHolderRef::Entity` must not wrap a person reference
and must resolve to a live core or domain entity whose entity type is allowed
to hold knowledge at the applicable cut. Core policy allows people directly
and institutional/agent-like core entities such as governments and armies;
routes, territories, and resources are disallowed. A domain entity schema must
opt in with `KnowledgeHolderPolicy::Allowed`; the backward-compatible default is
`Disallowed`. This supports governments, commands, staffs, units, companies,
archives, and other institutional holders without allowing nonsensical holders
such as a road or resource token.

Holder lifecycle is explicit. A new publication requires a holder that is live
and eligible at the proposal-visible cut. Retirement or deletion of an entity
does not erase its ledger: the stable entity tombstone and all records remain
available to research/developer audit and deterministic reconstruction. A
normal holder-bound viewer can be created only for a live eligible holder.
Succession, merger, reassignment, or transfer of custody never copies knowledge
automatically; an application must publish a new record to the successor with
explicit causal evidence. Stable holder identities are never reused. Domain
entity deletion continues to follow the existing retire-before-delete and
reference-integrity rules; a retained knowledge subject or holder reference is
a historical reference, not a reason to mutate prior knowledge.

Add a marker trait parallel to `DomainRecordType`:

~~~rust
pub trait KnowledgeRecordType {
    type Payload;

    const NAMESPACE: &'static str;
    const NAME: &'static str;
    const SCHEMA_VERSION: u32;
}
~~~

`KnowledgeRecordId` is already the typed ID wrapper. No second kind-specific ID
wrapper is introduced. The stored record carries `KnowledgeSchemaId`, and
deserialization validates its payload against that exact registered version.
One plugin may register old read-only schema versions, but exactly one version
per kind is writable in a run. Schema migration is explicit; a new schema may
not reinterpret records written under an older version.

### Knowledge subjects

Generic knowledge can concern a core entity, an application record, or an
event. These subject links are part of what the holder is allowed to see; they
are distinct from audit-only production evidence. Add these types to
`canwu-knowledge`:

~~~rust
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum KnowledgeSubjectTarget {
    Entity(EntityRef),
    DomainRecord(DomainRecordRef),
    Event(EventId),
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct KnowledgeSubject {
    pub role: String,
    pub target: KnowledgeSubjectTarget,
}
~~~

Entity is used for live core or domain-entity identities. DomainRecord is used
when the subject is a non-entity application record such as a dispatch,
interpretation, or release.

Subjects are sorted and deduplicated before admission. Each role is checked
against the registered knowledge schema. An application must not place a
hidden dispatch, access, source, or collector record in subjects merely to aid
debugging; those links belong in `KnowledgeOrigin.evidence` and are withheld
from holder-facing projections.

### Knowledge origin

The mechanism that produced a record must not be confused with whether its
payload is true. Add:

~~~rust
pub type KnowledgeEvidenceRef = EvidenceRef;

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum EvidenceRef {
    Command(CommandId),
    CommandAttempt(CommandAttemptId),
    Event(EventId),
    Ingress(IngressId),
    Boundary(BoundaryId),
    RandomDraw(RandomDrawId),
    DomainRecordVersion(DomainRecordVersionRef),
}

pub struct DomainRecordVersionRef {
    pub record: DomainRecordRef,
    pub version: u64,
    pub established_by: DomainRecordVersionSource,
}

#[serde(tag = "type", rename_all = "snake_case")]
pub enum DomainRecordVersionSource {
    InitialScenario,
    BoundaryChange {
        boundary: BoundaryId,
        change_index: u64,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KnowledgeOrigin {
    pub method: String,
    pub evidence: Vec<EvidenceRef>,
}
~~~

`EvidenceRef`, `DomainRecordVersionRef`, and `DomainRecordVersionSource` belong
to `canwu-core`, not `canwu-knowledge`, because knowledge origin, random
reservations, archive receipts, decisions, and future non-knowledge audit paths
share them. `KnowledgeEvidenceRef` is only a compatibility type alias to
`EvidenceRef`; no persisted keyed-random structure depends on a knowledge-only
enum. Domain record version zero is invalid.

Method is a canonical plugin-defined code such as `direct_observation`,
`received_representation`, or `derived_interpretation`. Evidence is sorted and
deduplicated. It must resolve at the proposal-visible cut. A domain-record
reference names the exact version and either its scenario origin or the exact
boundary/change index that established it; a mutable stable reference alone is
not historical evidence. The boundary record separately preserves the
producer plugin, system, phase, and boundary cause.

Runtime validation uses one expanded persisted-evidence resolver for command,
event, ingress, boundary, random-draw, and domain-record-version references.
Resolution returns `Retained(EvidenceView)`,
`Archived(ArchivedEvidenceReceipt)`, or `Missing`. Compaction therefore does not
make an earlier fact unusable as the cause of a later derivation.

~~~rust
pub enum EvidenceJournalKind {
    Event = 1,
    Command = 2,
    CommandAttempt = 3,
    Ingress = 4,
    Boundary = 5,
    RandomDraw = 6,
}

pub enum EvidenceNestedLocator {
    None,
    BoundaryRecordChange { change_index: u64 },
}

pub struct EvidenceItemLocator {
    pub journal: EvidenceJournalKind,
    pub absolute_index: u64,
    pub nested: EvidenceNestedLocator,
}

pub struct ArchivedEvidenceLocator {
    pub segment_id: String,
    pub item: EvidenceItemLocator,
}

pub struct ArchivedEvidenceReceipt {
    pub evidence: EvidenceRef,
    pub locator: ArchivedEvidenceLocator,
    pub evidence_index_leaf: u64,
    pub item_commitment: String,
    pub merkle_path: Vec<String>,
}

pub struct EvidenceIndexEntry {
    pub reference: EvidenceRef,
    pub item: EvidenceItemLocator,
    pub item_commitment: String,
}

pub struct EvidenceIndexLeafMaterial {
    pub format_version: u32,
    pub reference: EvidenceRef,
    pub item: EvidenceItemLocator,
    pub item_commitment: String,
}

pub struct EvidenceJournalRoots {
    pub events: String,
    pub commands: String,
    pub command_attempts: String,
    pub ingress: String,
    pub boundaries: String,
    pub random_draws: String,
}

pub struct ArchivedSegmentHeader {
    pub segment_id: String,
    pub start: EvidenceCursor,
    pub end: EvidenceCursor,
    pub journal_roots: EvidenceJournalRoots,
    pub evidence_index_root: String,
    pub evidence_index_entry_count: u64,
}

pub struct ArchivedSegmentHeaderMaterial {
    pub start: EvidenceCursor,
    pub end: EvidenceCursor,
    pub journal_roots: EvidenceJournalRoots,
    pub evidence_index_root: String,
    pub evidence_index_entry_count: u64,
}

pub enum EvidenceRequirement {
    IdentityOnly,
    PayloadRequired,
}

pub struct EvidenceDependency {
    pub reference: EvidenceRef,
    pub requirement: EvidenceRequirement,
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

pub enum ArchiveStoreOutcome {
    Stored,
    AlreadyPresent,
}

pub struct EvidenceSealToken {
    pub source_state_hash: String,
    pub source_checkpoint_hash: String,
    pub source_end: EvidenceCursor,
    pub segment_id: String,
    pub target_checkpoint_hash: String,
    pub token_hash: String,
}

pub struct PreparedEvidenceSeal {
    pub token: EvidenceSealToken,
    pub segment: EvidenceJournalSegment,
}
~~~

Format-7 segment sealing derives a sorted `EvidenceIndexEntry` for every exact
command, event, ingress, boundary, random draw, and boundary-nested
domain-record version in the segment. Each entry contains the exact evidence
reference, typed locator, top-level item commitment, and an optional compact
plugin-ingress provenance proof. That proof exists only when a plugin-generated
ingress matches its committed producing boundary; direct host ingress has no
proof. The segment stores the entries, builds a binary Merkle root over
domain-separated canonical leaves, and places that `evidence_index_root` plus
separate per-journal item roots in its header. Entries sort by `(reference,
item)` and duplicates are invalid. The leaf is exactly
`canonical_hash("canwu.evidence.index.leaf.v2", EvidenceIndexLeafMaterial {
format_version: 2, reference, item, item_commitment,
plugin_ingress_provenance })`, using the existing canonical JSON encoder and
this frozen struct field order. Its lower-case hex result is decoded to the
32-byte leaf value. Interior nodes are
`BLAKE3("canwu.evidence.index.node.v1" || 0x00 || left[32] || right[32])`.
Odd levels duplicate the final hash; the empty root is
`BLAKE3("canwu.evidence.index.empty.v1" || 0x00)`. The committed entry count
must equal the stored entry vector length and determines the only valid proof
depth/tree shape.

The journal-root domain strings are exactly:

| Header field | Canonical-hash domain |
| --- | --- |
| `events` | `canwu.evidence.journal.events.v1` |
| `commands` | `canwu.evidence.journal.commands.v1` |
| `command_attempts` | `canwu.evidence.journal.command_attempts.v1` |
| `ingress` | `canwu.evidence.journal.ingress.v1` |
| `boundaries` | `canwu.evidence.journal.boundaries.v1` |
| `random_draws` | `canwu.evidence.journal.random_draws.v1` |

Each is passed to `canonical_hash` with that journal's ordered top-level item
array. Array order is absolute-index order and an empty journal uses the
canonical hash of an empty array. `segment_id` is lower-case BLAKE3 of
`"canwu.evidence.segment.v3" || 0x00 ||
canonical(ArchivedSegmentHeaderMaterial)`; material field order is frozen as
shown, and `segment_id` itself is never part of the input.

The segment Merkle material contains the `EvidenceItemLocator`, item commitment,
and optional provenance, but never `segment_id`. A receipt wraps that material
with the verified segment ID only after the header and content-addressed segment
ID have been computed, so segment construction has no hash cycle.

The compact checkpoint retains the ordered `ArchivedSegmentHeader` list and
commits it as `archived_segment_manifest_root`. The list grows per segment, not
per evidence item. A receipt contains the exact evidence-index Merkle proof and
is stored only for references reachable from current domain state, current
knowledge origins, pending operations, keyed-random reservations, or explicit
retry indexes. Receipts are sorted by evidence reference and committed by a
separate skipped-when-empty receipt root. They are not copies of the payload.
The Format-7 receipt-root domain is
`canwu.evidence.archived-receipts.v2`.

A live domain-record schema may declare the required reserved
`canwu_identity_evidence_dependencies` object. Its sorted unique references
retain only identity receipts, including compact provider provenance, without
requiring archived payload hydration. Removing a dependency from the next
committed live record releases the receipt at the next seal. Payload-reading
continuations continue to use the separate
`canwu_payload_required_evidence_continuation` contract.

Locator legality is exhaustive: `Command`, `CommandAttempt`, `Event`,
`Ingress`, `Boundary`, and `RandomDraw` use their matching journal plus
`EvidenceNestedLocator::None`; `DomainRecordVersion` established by
`BoundaryChange` uses the Boundary journal plus the same
`BoundaryRecordChange { change_index }`; `InitialScenario` is never archived
and continues to resolve through the committed scenario/genesis material.
Every other reference/locator combination is invalid.

`evidence_index_leaf` must be less than `evidence_index_entry_count`. Every
`merkle_path` element is lower-case hex for exactly 32 bytes; the leaf index and
entry count determine left/right placement and the duplicated-odd-node steps.

Identity-only validation requires no provider: it finds the segment header in
the committed manifest, recomputes the evidence-index leaf from evidence,
`locator.item`, and item commitment, verifies the Merkle path to the header
root, and checks that the absolute top-level index lies within the appropriate
start/end cursor range. A `DomainRecordVersion` item locator must use the
Boundary journal and its exact `BoundaryRecordChange` index; all other evidence
uses `None`.

An archived receipt is sufficient for origin existence, identity, and causal
cut validation. A rule that needs to inspect the old payload must request the
sealed segment from the optional host-owned `ArchiveProvider`. The engine
recomputes the segment's per-journal roots, evidence index, header, and ID;
locates the top-level item by journal kind and absolute-index offset; verifies
its commitment; and, for a nested record version, decodes and checks the exact
boundary change. A missing provider/segment returns
`EvidenceContentUnavailable` before any mutation. A mismatching segment is
`InvalidSnapshot`, not a soft absence. Missing or unproved references fail with
`EvidenceUnavailable`. Provider output never changes authoritative state unless
a later validated command/ingress persists a derived result.

`ensure_retained_evidence_is_sealable` no longer rejects a plugin merely for
declaring command/event/ingress reads. Pending versioned operations and
continuation records persist a sorted, deduplicated `evidence_dependencies`
vector. Current knowledge origins, retry indexes, and keyed-draw reservations
automatically contribute `IdentityOnly` dependencies. A plugin cannot register
an ephemeral or hidden dependency outside these authoritative records. Before
sealing, the kernel reduces every live vector into one sorted map, promoting a
reference to `PayloadRequired` if any consumer requires it, and commits that
map through a skipped-when-empty `evidence_dependency_root` in compact
continuation material. Its exact material is the sorted
`Vec<EvidenceDependency>` hashed with
`canonical_hash("canwu.evidence.dependencies.v1", dependencies)`; empty omits
the root and non-empty duplicate references are invalid.

Identity-only dependencies receive receipts. `PayloadRequired` is valid only
for a pending authoritative continuation whose schema declares that it must
inspect the historical payload. Sealing uses a mandatory two-phase protocol:

1. `prepare_evidence_seal()` reads an immutable cut, builds and fully verifies
   the candidate segment, receipts, target compact checkpoint, and
   `EvidenceSealToken`, but mutates no runtime state. The token hash is
   `canonical_hash("canwu.evidence.seal-token.v1", all preceding token fields)`.
2. The host stores the returned content-addressed segment through
   `ArchiveStore::store_evidence_segment`. Store is idempotent: identical bytes
   for the same segment ID return `AlreadyPresent`; different bytes for that ID
   return `InvalidArchive`.
3. `commit_evidence_seal(token, provider)` takes the runtime write lock, rejects
   a changed source state/checkpoint/cursor as `StaleSealToken`, loads the exact
   segment back from `ArchiveProvider`, revalidates its journal roots, evidence
   index, header, ID, and target checkpoint hash, then applies the entire local
   checkpoint/journal/cursor/continuation change atomically. Missing content is
   `ArchiveNotReady`; mismatching content is `InvalidArchive`; neither mutates
   local state.

The exact same already-committed token is an idempotent success; every other
stale token fails. If external storage succeeds but local commit fails or the
host abandons the token, the content-addressed segment is an unreferenced
orphan. It is not authoritative because no checkpoint manifest names it and
may be garbage-collected by the host after checking all retained manifests.
Future, not-yet-admitted work may still encounter
`EvidenceContentUnavailable` deterministically.

On checkpoint-plus-segment rejoin, segments are ordered by cursor, verified,
and used to rebuild the header manifest, receipt map, retry indexes, and keyed
reservations. The rebuilt values must equal the compact checkpoint roots before
the continuation copies are discarded in the reconstructed full snapshot.
Duplicate segment IDs, overlapping cursor ranges, duplicate receipts, or a
reservation whose draw also remains in the retained tail are invalid. The
kernel never guesses from an ID range, and full snapshot/replay reconstruction
rejoins archived segments before revalidating every origin.

Publication-time validation additionally uses a `ProposalEvidenceResolver`:

~~~rust
pub enum ProposalEvidenceResolution<'a> {
    Retained(EvidenceView<'a>),
    Archived(&'a ArchivedEvidenceReceipt),
    PendingCurrentBoundary(PendingEvidenceView<'a>),
    Missing,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ProposalCommitStage {
    BoundaryStart = 0,
    Ordinary = 1,
    Transition = 2,
    Aggregation = 3,
}

pub enum PendingEvidenceItem<'a> {
    DomainRecordChange {
        change_index: u64,
        record: &'a DomainRecord,
    },
    Event {
        emission_index: u64,
        event: &'a SimEvent,
    },
    RandomDraw {
        draw_index: u64,
        draw: &'a RandomDrawRecord,
    },
}

pub struct PendingEvidenceView<'a> {
    pub boundary: BoundaryId,
    pub established_stage: ProposalCommitStage,
    pub evidence: PendingEvidenceItem<'a>,
}
~~~

It overlays the persisted resolver with the current boundary builder: pending
record changes, random draws, and emitted events, each carrying its already
frozen final vector index and the commit stage that established it.

The mapping to the existing settlement model is exact:

| Existing phase / internal stage | Proposal stage | Becomes visible |
| --- | --- | --- |
| state at boundary start | `BoundaryStart` | all proposal phases |
| phase 7 `DomainDeltaProposal` / `DomainRecordCommitStage::Ordinary` | `Ordinary` | after phase 9 atomic commit |
| phase 10 `HistoricalCandidateEvaluation` / `Transition` | `Transition` | after phase 11 conditional commit |
| phase 12 `StrategicAggregation` / `Aggregation` | `Aggregation` | after phase 12 stage application |
| phase 13 `Perspective` or `Deferred` | not placed in the phase-13 resolver | phase 14 or next boundary only |

`PendingBoundaryEvidence` is changed to store staged wrappers rather than bare
values. When a stage succeeds, settlement appends its record changes, events,
and random draws immediately in canonical order, assigns their final
zero-based vector indexes, and tags them with the mapped proposal stage. These
indexes can no longer move; a later failure rolls back the whole boundary.
Immediately before the first phase-13 handler, settlement freezes an immutable
`ProposalEvidenceOverlay` whose `maximum_visible_stage` is `Aggregation` and
passes the same overlay to every phase-13 `SimulationView`. No phase-13 output
is inserted until every phase-13 handler has returned. Phase 4 receives an
empty overlay with maximum stage `BoundaryStart`.

Phase 13 may therefore cite an exact domain-record version, event, or random
draw established by an earlier visible stage, including the current boundary
ID/change index, but may not cite a phase-13 peer, phase-14 output, the unfinished
boundary as a whole, or an unallocated publication in the same batch. A
`DomainRecordVersion` resolves only when record ID, version, boundary ID, final
record-change index, and staged wrapper all agree. An event/random draw resolves
only when its stable ID, final journal-tail index, producer, and staged wrapper
agree. After final commit the same reference resolves as `Retained`; staged
wrappers never appear in a snapshot, journal segment, or archive receipt.

The existing KnowledgeSource and ArmyKnowledge wire shape remain unchanged.
Generic knowledge does not replace the movement slice in the first milestone.

### Stored record and draft

Add:

~~~rust
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct KnowledgeRecordDraft {
    pub schema: KnowledgeSchemaId,
    pub subjects: Vec<KnowledgeSubject>,
    pub payload: Value,
    pub as_of: Option<SimTime>,
    pub confidence_per_mille: u16,
    pub origin: KnowledgeOrigin,
    pub supersedes: Vec<KnowledgeRecordId>,
    pub contradicts: Vec<KnowledgeRecordId>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct KnowledgeRecord {
    pub id: KnowledgeRecordId,
    pub holder: KnowledgeHolderRef,
    pub schema: KnowledgeSchemaId,
    pub subjects: Vec<KnowledgeSubject>,
    pub payload: Value,
    pub as_of: Option<SimTime>,
    pub learned_at: SimTime,
    pub confidence_per_mille: u16,
    pub origin: KnowledgeOrigin,
    pub supersedes: Vec<KnowledgeRecordId>,
    pub contradicts: Vec<KnowledgeRecordId>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct KnowledgeRecordView {
    pub id: HolderKnowledgeRecordId,
    pub holder: KnowledgeHolderRef,
    pub schema: KnowledgeSchemaId,
    pub subjects: Vec<KnowledgeSubject>,
    pub payload: Value,
    pub as_of: Option<SimTime>,
    pub learned_at: SimTime,
    pub confidence_per_mille: u16,
    pub supersedes: Vec<HolderKnowledgeRecordId>,
    pub contradicts: Vec<HolderKnowledgeRecordId>,
}
~~~

The plugin supplies `as_of` when the assertion concerns a particular simulation
time. The engine supplies id and learned_at at commit. as_of may be earlier or
later than learned_at because plans and forecasts can concern the future.

`KnowledgeRecordId` is an audit identity and never appears in a holder-facing
projection. `HolderKnowledgeRecordId` is the one-based ordinal of the record in
that holder's append-only ledger, derived deterministically from global ID
order. Because records are never removed or inserted retroactively, the ordinal
is stable without another persisted counter. View relations and cursors use the
holder-local ID. This prevents gaps in a global ID sequence from revealing the
volume or order of other holders' publications. Admin, replay, and boundary
evidence continue to use the global ID.

Do not add truth, accepted, believed, public, secret, fidelity, importance, or
expiration fields to this kernel type. Those semantics belong in payloads,
information records, belief plugins, or application policy.

Keep the existing person-keyed `ActorKnowledge` wire shape unchanged. Extend
`KnowledgeSnapshot` with a separate generic ledger:

~~~rust
pub struct KnowledgeSnapshot {
    pub actors: BTreeMap<PersonId, ActorKnowledge>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub records: BTreeMap<KnowledgeHolderRef, BTreeMap<KnowledgeRecordId, KnowledgeRecord>>,
}
~~~

The inner map key must equal `record.id` and `record.holder` must equal the
outer key. This avoids pretending that institutional knowledge belongs to a
synthetic person and avoids changing the legacy army-knowledge record.

`KnowledgeRecord` is an audit/programmatic value. Holder-facing APIs return
`KnowledgeRecordView`, which deliberately omits `origin` and all producer and
boundary metadata. Research and developer diagnostics may request the audit
record through a separately authorized method. There is no conversion from a
public observer context to either private view.

### Knowledge schemas

Add PluginKnowledgeSchema in canwu-sim:

~~~rust
pub struct PluginKnowledgeSchema {
    pub id: KnowledgeSchemaId,
    pub schema_hash: String,
    pub writable: bool,
    pub payload_schema: PayloadSchema,
    pub subjects: Vec<KnowledgeSubjectSchema>,
}

pub struct KnowledgeSubjectSchema {
    pub role: String,
    pub targets: Vec<KnowledgeSubjectTargetKind>,
    pub required: bool,
    pub multiple: bool,
}

pub enum KnowledgeSubjectTargetKind {
    Core(CoreEntityKind),
    Domain(DomainRecordKind),
    AnyEntity,
    Event,
}
~~~

Registration rules mirror domain record schemas:

- one plugin owns one knowledge kind across all versions;
- a schema ID is unique and its 64-character semantic hash is canonical;
- exactly one registered version per kind is writable in a run;
- historical read-only versions may remain registered for validation and
  migration, but a writer must name the exact current version;
- kinds and roles are canonical and unique;
- target lists are sorted and unique;
- required singleton roles appear exactly once;
- required multiple roles appear at least once;
- undeclared roles are rejected;
- `AnyEntity` accepts any core entity or any domain record whose registered
  class is `Entity`; it never accepts an ordinary non-entity record;
- payloads use the existing PayloadSchema validator;
- the complete plugin registration rolls back on any duplicate, ownership,
  version, hash, or shape error.

Add `knowledge_schemas` to `PluginDescriptor` with default and skip-empty
serialization. Descriptor equality, plugin semantic identity, run manifest,
snapshot validation, and exact rehydration include the complete versioned
schema list.

Add `PluginRegistrar::register_knowledge_schema`.

The same `AnyEntity` target is added to the generic
`DomainReferenceTargetKind`. This is necessary because the fixed
`canwu.information` schemas cannot enumerate institution, department, group,
unit, or organization kinds owned by future application plugins. Registration,
semantic hashing, mutation validation, snapshot rehydration, and tamper tests
all treat `AnyEntity` as a typed entity-class constraint rather than an
unvalidated reference escape hatch.

### Admission and query limits

Every collection and variable-length field introduced here has a deterministic
bound. The first implementation uses a versioned `KnowledgeLimitsV1` constant
set included in engine semantic identity:

| Limit | V1 value |
| --- | ---: |
| knowledge schemas owned by one plugin | 256 |
| records in one publication batch | 1,000 |
| publication batches from one system in one boundary | 64 |
| total new knowledge records in one boundary | 10,000 |
| encoded payload bytes in one knowledge record | 65,536 |
| subjects, evidence refs, supersedes refs, or contradicts refs per record | 64 each |
| UTF-8 bytes in method or summary | 1,024 each |
| queries in one batch query | 64 |
| IDs in one direct-get request | 1,000 |
| page size | default 100, maximum 1,000 |
| relation-trace depth | 32 |
| relation graph records returned | 10,000 |

Limits are checked before allocation and before any partial stage mutation.
Encoded byte limits use canonical JSON bytes, not Rust allocation size. An
overflow is a stable validation error naming the limit, and no API silently
truncates a write. Raising a limit requires an engine identity/version change;
lowering it during exact rehydration is forbidden.

### Boundary contract and directive

Extend `BoundarySystemContract`:

~~~rust
pub struct KnowledgeWriteGrant {
    pub schema: KnowledgeSchemaId,
    pub visibilities: Vec<StateVisibility>,
}

#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub knowledge_writes: Vec<KnowledgeWriteGrant>,
~~~

`KnowledgeRecordDraft` belongs to `canwu-knowledge`. The batch wrapper belongs
to `canwu-sim` because visibility and producer correlation are settlement
concerns and `canwu-knowledge` must not depend on the simulation crate:

~~~rust
pub struct KnowledgePublicationDraft {
    pub holder: KnowledgeHolderRef,
    pub visibility: StateVisibility,
    pub producer_correlation: Option<String>,
    pub records: Vec<KnowledgeRecordDraft>,
    pub summary: String,
}
~~~

Add a batch directive. Visibility belongs to the batch instead of being
inherited from the system's ordinary state-write visibility:

~~~rust
BoundaryDirective::PublishKnowledge {
    holder: KnowledgeHolderRef,
    visibility: StateVisibility,
    producer_correlation: Option<String>,
    records: Vec<KnowledgeRecordDraft>,
    summary: String,
}
~~~

A publication is allowed only when:

- the system phase is PerceptionAndAttentionRefresh or
  PerspectiveAndReportMaterialization;
- every draft schema and the requested visibility appear in one
  `contract.knowledge_writes` grant;
- the active plugin owns the matching PluginKnowledgeSchema;
- the holder exists at the correct proposal-visible cut;
- all subjects and evidence resolve at that cut;
- confidence_per_mille is at most 1000;
- supersedes and contradicts are sorted, unique, disjoint, and refer to
  existing lower-ID records belonging to the same holder;
- summary and origin.method are canonical;
- producer correlation is optional, canonical, at most 256 UTF-8 bytes, and
  unique within one producer system and boundary;
- the batch is non-empty and bounded by the engine's configured publication
  limit;
- the same canonical draft is not emitted twice by one system proposal;
- relations do not target another record in the same unallocated batch.

`knowledge_writes` is separate from `writes`. Plugins never become owners of
`canwu.core.knowledge`. Several systems belonging to the same schema-owning
plugin may publish the same kind; cross-plugin writers are rejected. Canonical
plugin, system, directive, and record order resolves all ordering.

### Phase semantics

Phase 4 and phase 13 serve different purposes:

- Phase 4 records perception available before decision intake.
- Phase 13 records reports and projections based on committed domain results.

`PublishKnowledge` is syntactically a `BoundaryDirective` for API economy, but
settlement extracts it before the ordinary directive path. It is never passed
to `apply_boundary_stage`, never becomes a component or domain-record write,
and is processed only by a kernel-owned `KnowledgePublicationStage`.

All systems in one phase read the same input cut. A phase-4 system cannot
observe another phase-4 system's output, and a phase-13 system cannot observe
another phase-13 system's output. Existing phase-13 ordinary domain/component
behavior remains unchanged: each system's ordinary `SameBoundary` directives
still use the established stage application semantics. Knowledge publication
is the only phase-13 output batched until every phase-13 handler has returned.

After phase 4 the knowledge stage:

1. rejects every non-publication directive in phase 4;
2. validates the combined publication bundle against the phase-4 input cut;
3. allocates IDs in canonical plugin/system/directive/record order;
4. stamps `learned_at` with the boundary time;
5. places `SameBoundary` records in a dedicated knowledge overlay visible to
   phases 5 through 13;
6. retains `NextBoundary` records outside every in-boundary read.

After all phase-13 systems the knowledge stage:

1. validates phase-13 publications against committed stages 9 and 11, the
   aggregation stage, and the phase-4 same-boundary knowledge overlay;
2. allocates their IDs and appends phase-13 `SameBoundary` records to the
   overlay only for phase-14 diagnostics;
3. retains phase-13 `NextBoundary` records outside phase-14 system reads; and
4. atomically commits both visibility classes to the current knowledge ledger
   immediately before boundary evidence, commitments, and hashes finalize.

The overlay is queried through an owned-result API. It is not exposed as a
borrowed `&KnowledgeSnapshot`, which would conflict with the existing
`SimulationView` lifetime and would encourage callers to bypass declared-read
checks. Current state remains untouched until the final knowledge commit, so a
late failure discards the pending ledger without a partial write.

The exact read cuts are:

| Consumer | Domain/component input | Generic knowledge input |
| --- | --- | --- |
| phase-4 handler | immutable boundary snapshot | committed ledger at boundary start |
| another phase-4 handler | same immutable snapshot | same committed ledger; no peer publication |
| phases 5-8 | existing immutable boundary snapshot and ordinary overlays allowed by the phase | committed ledger plus phase-4 `SameBoundary` overlay |
| phases 9-12 | existing committed-state semantics for that phase | committed ledger plus phase-4 `SameBoundary` overlay |
| phase-13 handler | state committed through aggregation | committed ledger plus phase-4 `SameBoundary` overlay; no peer phase-13 publication |
| phase-14 handler | final ordinary state | committed ledger plus phase-4 and phase-13 `SameBoundary` overlays |
| host after success | final state | all phase-4 and phase-13 publications |
| any consumer after rollback | pre-boundary state | pre-boundary ledger only |

Settlement processes a publication phase in this order:

1. execute handlers in canonical plugin/system order and collect proposals;
2. split ordinary directives from publication batches without reordering
   either class;
3. validate every publication against the handler's documented input cut;
4. run the existing ordinary stage at its existing point;
5. allocate knowledge IDs only after the complete publication bundle is valid;
6. extend the phase overlay and pending boundary knowledge evidence;
7. at final commit, append records, events, changes, roots, and counters as one
   transaction.

`BoundarySystemContract.visibility` continues to govern ordinary directives.
Each publication batch carries its own visibility and must be authorized by its
`KnowledgeWriteGrant`; this permits one system to produce a same-boundary
perception batch and a next-boundary confidential batch without creating two
fake system identities.

The entire boundary remains atomic. A later phase failure restores the
knowledge store, ID counter, events, boundary evidence, random state, queues,
time, and all other state to the pre-boundary checkpoint.

No publication occurs in phase 7 merely because information-domain records
changed. The application must explicitly convert access or interpretation into
a holder-relative record in phase 4 or 13. A phase-13 publication may cite a
domain-record version committed in an earlier stage of the same boundary, but
not a phase-13 mutation from its own or another phase-13 proposal because that
change index is not part of its proposal-visible cut.

### Knowledge change evidence

Add:

~~~rust
pub struct BoundaryKnowledgeChange {
    pub plugin: String,
    pub system: String,
    pub phase: BoundaryPhase,
    pub holder: KnowledgeHolderRef,
    pub producer_correlation: Option<String>,
    pub records: Vec<KnowledgeRecord>,
    pub visibility: StateVisibility,
    pub summary: String,
}
~~~

Extend BoundaryRecord:

~~~rust
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub knowledge_changes: Vec<BoundaryKnowledgeChange>,
~~~

Extend `BoundaryEmissionKind`:

~~~rust
KnowledgeChange { change_index: u64 }
~~~

Each committed batch emits exactly one built-in event. The knowledge subsystem
owns the typed payload and encodes it through the domain-neutral event record:

~~~rust
#[derive(Serialize, Deserialize)]
struct KnowledgePublished {
    holder: KnowledgeHolderRef,
    record_count: u32,
}

EventKind::from_payload("knowledge_published", &payload)?
~~~

Every batch contains one holder and receives a contiguous audit ID range, stored
in `BoundaryKnowledgeChange`. The player-facing event intentionally omits that
range so a global ID gap cannot reveal other holders' traffic. Its audience is
always that holder, except that existing `ResearchFull` and
`DeveloperDiagnostic` policies may inspect it. `PublicObserver` receives no
such event. Plugin event audience registration cannot widen this built-in
audience.

`BoundaryReceipt` gains `knowledge_record_count` and
`knowledge_batch_count`. It does not return records themselves; callers query
through holder-relative knowledge APIs or diagnostic state.

Extend the built-in event audience contract:

~~~rust
EventAudience::KnowledgeHolder(KnowledgeHolderRef)
~~~

The event projection resolver authorizes a character seat only for the matching
person holder and an institution seat only for its bound institution entity.
It never resolves an institution to its current responsible person, because a
personnel change must not alter visibility of historical institutional events.
`PublicObserver` never matches this audience; `ResearchFull` and
`DeveloperDiagnostic` retain their explicit diagnostic behavior.

The existing `ViewerContext` becomes principal-aware in the intentional 0.5.0
source break: it carries `ObservationPrincipal` rather than an unconditional
`PersonId`. `observe(actor, ...)` and `viewer_context(actor)` remain
character-seat convenience methods; institution-bound and public runs use the
restricted `CanwuViewer` created from run policy. Event save/load and replay
persist the holder in `KnowledgePublished`, while authorization is recomputed
from the current run's persisted seat binding without rewriting history.

### Public queries

#### Trust and capability boundary

`Canwu` remains the trusted in-process engine-host/admin API. Its detached
world, knowledge, domain-record, boundary, event, snapshot, and export methods
are intentionally omniscient. `PublicObserver` does not make an object that
already owns `Canwu` untrusted; Rust code with that capability is part of the
trusted host.

Player clients, AI agents, network callers, and read-only observers must not be
given the admin API. Add a restricted `CanwuViewer<'a>` session created by
the host:

~~~rust
pub enum ObservationPrincipal {
    Person(PersonId),
    Institution(EntityRef),
    Public,
    Research,
    Developer,
}

pub fn viewer(&self) -> Result<CanwuViewer<'_>, CanwuError>;
~~~

The principal is derived entirely from persisted run policy and seat binding:

- `ActorBound + CharacterBound` yields only `Person(binding.actor)`;
- `ActorBound + InstitutionBound` yields only
  `Institution(binding.institution)`, even when the binding also names a person;
- `PublicObserver` yields `Public` and no holder authority;
- `ResearchFull` yields `Research` and may select an existing holder but receives
  audit origin only through separately named research methods;
- `DeveloperDiagnostic` yields `Developer` with the same explicit separation;
- a `LegacyUnspecified` run has compatibility-only character behavior and may
  yield `Person(requested_actor)` after existence checks; it never implies
  institution, research, or developer authority.

Invalid or incomplete combinations fail rather than falling through to a
stronger principal. `CanwuViewer` exposes only authorized observe, query,
knowledge, event, action, explanation, and capability methods. It has no raw
knowledge/event/boundary/domain-record/snapshot/export accessor and cannot be
upgraded back to `Canwu`. This is the supported privacy boundary. Bindings and
remote adapters expose `CanwuViewer`, not the admin API.

Runtime plugins are also part of the trusted computing base. A system granted
the coarse `canwu.core.knowledge` read can inspect the full ledger through
`SimulationView`; it is not sandboxed as an untrusted mod. Future untrusted
plugin support requires holder/schema-scoped read grants or process isolation
and is outside this milestone.

Add:

~~~rust
pub struct KnowledgeQuery {
    pub schemas: Vec<KnowledgeSchemaId>,
    pub subjects: Vec<KnowledgeSubject>,
    pub learned_after: Option<SimTime>,
    pub learned_at_or_before: Option<SimTime>,
    pub view: KnowledgeHistoryView,
    pub after: Option<KnowledgeCursor>,
    pub limit: u32,
}

pub enum KnowledgeHistoryView {
    CurrentHeads,
    FullHistory,
}

pub struct KnowledgeCursor {
    pub holder: KnowledgeHolderRef,
    pub query_hash: String,
    pub read_cut: KnowledgeReadCut,
    pub binding_hash: String,
    pub learned_at: SimTime,
    pub record: HolderKnowledgeRecordId,
}

pub struct KnowledgeReadCut {
    pub boundary: Option<BoundaryId>,
    pub holder_projection_root: String,
    pub holder_overlay_root: Option<String>,
}

pub struct KnowledgeQueryResult {
    pub holder: KnowledgeHolderRef,
    pub read_cut: KnowledgeReadCut,
    pub records: Vec<KnowledgeRecordView>,
    pub next: Option<KnowledgeCursor>,
}
~~~

Query behavior:

- schemas are normalized to a sorted, deduplicated any-of set; subjects are
  normalized to a sorted, deduplicated all-of set;
- empty schemas or subjects mean no filter on that dimension;
- records are returned by learned_at then KnowledgeRecordId ascending;
- default limit is 100 and maximum accepted limit is 1000;
- `CurrentHeads` excludes every record superseded by a later visible record;
- `FullHistory` includes superseded records;
- contradicted records remain visible because contradiction is evidence, not
  erasure;
- `learned_after` provides time-based delta retrieval; `after` provides stable
  pagination when several records share a timestamp;
- the first page captures the current boundary, holder-facing projection root,
  and optional holder-facing same-boundary overlay root as `KnowledgeReadCut`;
- later pages read that same immutable cut or fail with
  `KnowledgeReadCutUnavailable`; they never splice records from a newer
  boundary into an older page sequence;
- cursors bind holder, canonical query hash, read cut, time, and record ID and
  fail validation if reused for another query;
- `CurrentHeads` is computed over the holder's complete ledger at the read cut
  before schema, subject, or time filters are applied, so filtering out a
  successor never resurrects a superseded assertion as current;
- supersession is allowed only within the same schema kind. A schema-version
  migration may link versions of that kind; cross-kind relationships use
  subjects or application payloads instead;
- filtering and pagination are deterministic and require no persisted derived
  index.

The query hash is exactly
`canonical_hash("canwu.knowledge.query.v1", KnowledgeQueryHashMaterialV1)` over
the normalized schema set, normalized subject set, learned-time bounds, and
history view in that frozen field order. `after` and `limit` are excluded: a
cursor is the position, and callers may change page size without changing the
selected record set. The code limit remains independently validated on every
page.

`holder_projection_root` is
`canonical_hash("canwu.knowledge.holder-projection.v1",
HolderProjectionCutMaterialV1 { holder, full_history_views })`. The views are the
complete holder-facing `KnowledgeRecordView` history in local-ID order after
schema-declared subject projection; they omit global IDs, audit origin/evidence,
and hidden subject roles. It is therefore not the global `KnowledgeSnapshot`
root and cannot reveal whether another holder's private ledger changed.

`holder_overlay_root` is absent for a settled view; for a phase view it is
`canonical_hash("canwu.knowledge.holder-overlay.v1",
HolderOverlayCutMaterialV1 { holder, visible_projected_batches })` in canonical
commit order and includes only publications visible to that holder. The engine
continues to use the private global knowledge root for snapshot validation, but
never returns it through `CanwuViewer` or a normal query cursor.

`binding_hash` is exactly
`canonical_hash("canwu.knowledge.cursor.v1", KnowledgeCursorBindingMaterialV1 {
holder, query_hash, read_cut })`. The runtime verifies the cursor's own binding
before comparing it with the requested holder/query/current cut. No new
generation counter is introduced. A cursor is accepted only when all cut fields
equal the currently available holder view; the first slice does not retain
arbitrary historical cuts, so any older or otherwise unavailable tuple returns
`KnowledgeReadCutUnavailable`. This derives cursor identity from committed or
deterministically projected data without exposing a cross-holder change oracle.

The ledger models durable assertions available to a holder, not short-term
cognitive recall. `CurrentHeads` means the current assertion set after explicit
correction; it does not mean that older material was erased. Normal agent
observation and action planning receive `CurrentHeads` only. `FullHistory` is an
explicit holder-history capability and the audit form remains restricted to
research/developer policy. Forgetting, expiring credentials, classified-access
revocation, or lossy human memory belong in an application policy that decides
what can be used now; they must not delete historical `KnowledgeRecord`s or
retroactively revoke a prior `Access` record. If such a policy changes an
agent's current usable projection, it publishes or persists that policy state
separately and cites it in later decisions.

The trusted admin API and restricted viewer deliberately expose different
methods:

~~~rust
impl Canwu {
pub fn admin_query_knowledge(
    &self,
    holder: KnowledgeHolderRef,
    query: &KnowledgeQuery,
) -> Result<KnowledgeQueryResult, CanwuError>;

pub fn admin_audit_knowledge_record(
    &self,
    record: KnowledgeRecordId,
) -> Result<KnowledgeRecord, CanwuError>;
}

impl CanwuViewer<'_> {
pub fn query_knowledge(
    &self,
    query: &KnowledgeQuery,
) -> Result<KnowledgeQueryResult, CanwuError>;

pub fn query_holder_knowledge(
    &self,
    holder: KnowledgeHolderRef,
    query: &KnowledgeQuery,
) -> Result<KnowledgeQueryResult, CanwuError>;

pub fn audit_knowledge_record(
    &self,
    record: KnowledgeRecordId,
) -> Result<KnowledgeRecord, CanwuError>;
}
~~~

`query_knowledge` is available only to a bound person or institution and chooses
that one holder internally. `query_holder_knowledge` is available only to
Research/Developer principals. `audit_knowledge_record` is likewise restricted
to those diagnostic principals. A `Public` principal receives
`InvalidKnowledgeAuthority` before any ledger lookup. `KnowledgeViewContext` is
crate-private implementation state inside `CanwuViewer`; it has private fields,
no public constructor, and is never accepted by `Canwu` methods.

The first runtime slice implements only `CurrentHeads`, `FullHistory`, and stable
cursor pagination. After that path passes authorization, snapshot, hash, replay,
and tamper proof, a second query slice adds bounded convenience methods on
`CanwuViewer`:

~~~rust
pub fn get_knowledge_records(
    &self,
    ids: &[HolderKnowledgeRecordId],
    read_cut: Option<&KnowledgeReadCut>,
) -> Result<Vec<KnowledgeRecordView>, CanwuError>;

pub fn query_knowledge_batch(&self, queries: &[KnowledgeQuery])
    -> Result<Vec<KnowledgeQueryResult>, CanwuError>;

pub fn knowledge_changes_since(&self, boundary: BoundaryId, limit: u32)
    -> Result<KnowledgeChangePage, CanwuError>;

pub fn trace_knowledge_relations(
    &self,
    record: HolderKnowledgeRecordId,
    direction: KnowledgeRelationDirection,
    depth_limit: u16,
) -> Result<KnowledgeRelationGraph, CanwuError>;
~~~

`trace_knowledge_relations` returns only supersession/contradiction structure
visible to the holder and applies a bounded depth. A separate diagnostic
`trace_knowledge_origin` method may traverse audit evidence under
ResearchFull/DeveloperDiagnostic. Batch calls share one read cut and have
bounded request and result counts.

The existing admin `knowledge()` method remains an omniscient detached read. It
must not appear on `CanwuViewer`, and its rustdoc must identify it as a trusted
host/diagnostic capability.

SimulationView keeps actor_knowledge for compatibility and adds:

~~~rust
pub fn knowledge_records(
    &self,
    holder: KnowledgeHolderRef,
    query: &KnowledgeQuery,
) -> Result<KnowledgeQueryResult, CanwuError>;
~~~

It requires canwu.core.knowledge in declared reads and uses the same-boundary
knowledge overlay when present. This is an omniscient plugin-system read gated
by the system contract, not an actor-facing authorization API. It returns owned
values so overlay and current-ledger records can be merged without exposing an
invalid borrowed reference.

### Validation rules

Runtime and snapshot validation must prove:

- every KnowledgeRecordId is positive and globally unique;
- next_knowledge_record_id is greater than every stored record ID;
- each holder map key and `record.holder` agree;
- every holder exists at its publication historical cut;
- every schema ID has the same persisted owner, version, and semantic hash as
  the run's plugin descriptor;
- every payload and subject matches its exact schema version;
- all evidence references resolve at the publication proposal cut, including
  exact domain-record versions and boundary/change indexes;
- supersedes and contradicts point backward to earlier IDs for the same holder;
- no record refers to itself;
- supersedes and contradicts are disjoint;
- learned_at equals the producing boundary time;
- boundary producer plugin/system/phase matches its descriptor;
- knowledge changes appear in canonical commit-stage order;
- every knowledge-change emission points to the correct batch index, holder,
  and count, while the audit ID range remains only in boundary evidence;
- the first-slice scenario generic ledger is empty and the final generic store
  can be reconstructed from boundary changes alone;
- replay regenerates the exact same records, IDs, events, and boundary hashes.

Because relation edges point only to lower IDs, supersession and contradiction
cycles are structurally impossible.

### Errors

Add distinct ErrorCode variants:

- DuplicateKnowledgeRecordKind
- InvalidKnowledgeRecord
- KnowledgeRecordNotFound
- UndeclaredKnowledgeWrite
- EvidenceUnavailable
- EvidenceContentUnavailable
- KnowledgeReadCutUnavailable
- InvalidKnowledgeAuthority
- KnowledgeLimitExceeded
- InvalidRandomOperationEvidence
- RandomOperationConflict

Use InvalidSnapshot for persisted evidence or counter disagreement and
IdentifierExhausted when allocating a new ID would overflow.

Do not overload InvalidDomainRecord for holder knowledge; the diagnostic must
identify the actual contract that failed.

### Persistence, compatibility, and versioning

The intended release is `0.5.0` with snapshot format 5. Adding fields to public
Rust structs and variants to public enums is an intentional pre-1.0 source
compatibility break and must be listed exhaustively in `docs/versioning.md`.
The list includes `KnowledgeSnapshot`, `PluginDescriptor`,
`BoundarySystemContract`, `BoundaryRecord`, `BoundaryReceipt`,
`SimulationSnapshot`, `ViewerContext`, `DomainRecordSchema`,
`RandomDrawRecord`, `BoundaryDirective`, `BoundaryEmissionKind`, `EventKind`,
`EventAudience`, `ErrorCode`, and `DomainReferenceTargetKind`. Workspace crate
versions and the lockfile move in lockstep. An external-crate compile fixture
documents the old and new construction/API surface.

Format 5 is required because `RandomDrawRecord.position` becomes a tagged
`RandomDrawAddress`, plugin descriptor identity gains semantic fields, compact
continuation gains new committed roots, and engine identity changes. These are
not represented as a format-4 additive compatibility claim.

Milestone 1 owns the complete tagged random-address wire/source break and the
legacy `Sequential` migration. The `OperationV1` shape is reserved in format 5,
but the first runtime slice returns `UnsupportedRandomDrawAddress` for every
loaded or proposed keyed draw until Milestone 6 supplies byte-exact execution,
validation, replay, and golden vectors. A runtime never accepts authoritative
random state it cannot recompute.

The 0.4-to-0.5 migration order is strict:

1. read only the raw outer envelope and its format/engine-version selectors;
2. route format-4 JSON, replay envelopes, and checkpoint/journal bundles to
   independent legacy V4 wire structs whose nested objects use
   `#[serde(deny_unknown_fields)]`; reject every unknown or format-5-only field
   before constructing any current runtime type;
3. deserialize the accepted legacy wire and require the exact legacy engine
   identity `0.4.0`;
4. under the 0.4 contract, validate checkpoint hash,
   commitment roots, boundary chain, final state hash, replay envelope, and
   checkpoint/journal continuity;
5. migrate sequential draws to `RandomDrawAddress::Sequential`, apply default
   schema policies/grants, create empty generic-ledger/continuation state, and
   set the next generic knowledge ID to one;
6. write a format-5 snapshot and recompute all roots under the 0.5 engine
   identity.

The typed `SimulationSnapshot` loader accepts format 5 only. Format 4 can enter
only through the strict legacy loader above; relabeling a format-5 value as
format 4 therefore fails at unknown-field validation before legacy commitment
validation or migration.

Migration proves that the new state is derived from a valid old state. It does
not claim that 0.5 exact replay reproduces historical 0.4 intermediate state
commitments. Auditing those original commitments requires the 0.4 runtime or an
unchanged verified format-4 bundle. After migration, continued boundaries and
all new exact replay use format 5 only.

Checked-in 0.4 fixtures cover: an empty no-plugin state; a plugin with a domain
schema and boundary contract; at least one sequential draw; a v1 boundary state
hash; a ReplayJournal; and checkpoint-journal plus compact continuation. Each
fixture has a negative tamper companion. Checked-in post-migration fixtures
prove deterministic format-5 bytes and roots.

Within format 5, empty/additive fields still use canonical defaults and
skip-empty/skip-one rules so future additions have an explicit baseline. The
knowledge commitment root remains the external canonical hash of
`KnowledgeSnapshot`; the snapshot does not gain a root field.

Generic records enter the existing knowledge commitment root through ordered
serialization. Boundary hashes bind knowledge batches and their emissions.
Publishing invalidates the `KNOWLEDGE` commitment domain; allocating a record ID
invalidates `CONTROL`.

`ReplayJournal` does not need a duplicate final knowledge counter field. The
first slice forbids generic records in `Scenario.knowledge`; scenario admission
requires the generic ledger to be empty and starts the counter at one. Exact
replay reconstructs the ledger and counter from ordered boundary publication
batches, and the final checkpoint/control root proves the result. A later
scenario-genesis feature must use ID-free, time-free `ScenarioKnowledgeDraft`
values and plugin-aware admission; scenario authors never choose record IDs,
learning times, or audit origins.
`migration.rs`, `validation.rs`, `replay.rs`, and checkpoint-journal
reconstruction are mandatory change surfaces.

No separate evidence cursor is required because knowledge changes are embedded
inside boundary records. Checkpoint/journal segments and compact runtime sealing
must retain and reconstruct those boundary records exactly. Replace the current
`ensure_retained_evidence_is_sealable` rule that rejects any plugin declaring
command/event/ingress reads. Sealing is allowed when every live dependency can
be represented by a verified receipt or an explicitly registered archive
provider can supply required payload content. After sealing, origin/cut checks
may use receipts; a rule that actually needs unavailable payload content fails
deterministically before mutation.

### Rollback and replay

Extend BoundaryTransactionCheckpoint with:

- the pending knowledge publication ledger and overlay;
- `next_knowledge_record_id` through `RuntimeCounters`;
- the event and random-draw tails used by publication evidence;
- any new boundary evidence length if evidence is staged outside
  `BoundaryRecord`.

Exact replay re-executes boundary handlers and compares generated
BoundaryKnowledgeChange values. It never calls an external interpreter,
controller, policy, or language model. Any such external result must first be
admitted as a command or canonical ingress record.

## canwu-information design

### Package boundary

Create crates/extensions/canwu-information with:

~~~toml
[package]
publish = ["crates-io"]

[dependencies]
canwu-api = { path = "../../api/canwu-api" }
serde.workspace = true
serde_json.workspace = true
~~~

The crate provides typed record payloads, schema constructors, pure validation
helpers, owned operation requests/results, and an authoritative
`InformationPlugin`. It does not receive mutable simulation state and does not
bypass the public API.

`InformationPlugin` owns the fixed `canwu.information` record namespace,
registers the schemas, commands, ingress descriptors, and boundary systems, and
is the only writer of those record kinds. Application code cannot register the
same namespace or submit raw record mutations under the information plugin's
identity. It requests operations through validated plugin commands or
canonical information ingress. This makes lifecycle enforcement an
authoritative runtime path instead of a convention around pure helper calls.

Application-specific timing, probability, costs, interpretation capability,
and audience membership are resolved outside the shared lifecycle. Their
resolved inputs enter the authoritative plugin as typed commands or canonical
ingress with normal cause, idempotency, revision, persistence, and replay
evidence. Exact replay consumes those records and never reruns an external
model. Cross-plugin requests use the next-boundary ingress contract described
below; the first milestone also supports host-routed commands/ingress and
decisions whose accepted option emits an information-plugin request.

The fixed plugin owns only neutral schemas whose truth is established by its
own lifecycle records, for example `representation_available`,
`access_recorded`, `interpretation_recorded`, and `release_available`. It does
not own application claims such as military assessments, prices, legitimacy,
intent, or trust. An application plugin that derives one of those claims must
register and publish its own versioned knowledge schema. Its origin cites the
exact information access or interpretation record version that made the input
available. Consequently, adding a new semantic domain does not enlarge the
trusted information plugin or create a universal knowledge-schema owner.

To make immutable information kinds fail closed even inside a faulty handler,
extend the generic `DomainRecordSchema` contract with:

~~~rust
pub enum DomainRecordMutationPolicy {
    Versioned,
    CreateOnly,
}
~~~

The field defaults to `Versioned` for old descriptors and participates in
descriptor identity. `CreateOnly` rejects update, retire, and delete mutations
in the kernel's domain mutation bundle validator. Channel, content,
representation, access, interpretation, and audience schemas use
`CreateOnly`. Instance, dispatch, delivery attempt, and release use
`Versioned`, with their state-machine transitions additionally enforced by the
authoritative information plugin.

### Typed record kinds

The fixed namespace is canwu.information. Define marker types for:

- channel
- content
- representation
- instance
- dispatch
- delivery_attempt
- access
- interpretation
- audience
- release
- operation

All use `TypedDomainRecordRef`. They are non-entity records in the first
version. Senders, custodians, publishers, performers, holders, recipients, and
locations remain core or downstream domain entity references. Every entity
role uses the generic `AnyEntity` target so future application entity kinds do
not require an InformationPlugin descriptor change.

The extension also declares `InformationLimitsV1` in plugin semantic identity:

| Limit | V1 value |
| --- | ---: |
| parent edges on content or representation | 64 |
| addressed recipients on one dispatch | 10,000 |
| explicit members on one audience snapshot | 10,000 |
| resolved group references on one audience snapshot | 64 |
| delivery attempts for one recipient in one dispatch | 256 |
| output slots on one operation kind | 256 |
| inline body canonical JSON bytes | 65,536 |
| external resource declared byte length | 1 TiB |

The kernel-level boundary cap still limits how many consequences can commit at
once. Work exceeding a boundary cap is represented by a persisted continuation
cursor and processed in deterministic bounded chunks; it is never partly
accepted because a vector happened to be too large.

### Channel profile

~~~rust
pub struct ChannelPayload {
    pub profile: String,
    pub capabilities: Vec<ChannelCapability>,
}

pub enum ChannelCapability {
    PersistentInstance,
    NonexclusiveAccess,
    SimultaneousAccess,
    AddressedDelivery,
    AudienceDelivery,
    OpenReception,
}
~~~

A channel profile describes capabilities only. It does not contain a fixed
historical medium, interception probability, speed formula, or security score.
Applications compute due times and access outcomes and persist those resolved
values in dispatches or ingress. Capabilities are sorted and unique. At least
one of `AddressedDelivery`, `AudienceDelivery`, or `OpenReception` is required,
and every dispatch target must be supported by its channel. Further
capabilities require a schema-version change and an independent consumer, not
another case-driven boolean.

Channel records are scenario-created and immutable.

### Content

~~~rust
pub struct ContentPayload {
    pub content_type: String,
    pub body: InformationBody,
    pub created_at: SimTime,
    pub derivation: Option<ContentDerivation>,
}

pub enum InformationBody {
    InlineJson(Value),
    Resource {
        digest: ContentDigest,
        media_type: String,
        byte_length: u64,
    },
}

pub struct ContentDigest {
    pub algorithm: DigestAlgorithm,
    pub value: String,
}

pub enum DigestAlgorithm {
    Sha256,
}

pub struct ContentDerivation {
    pub operation: String,
    pub sources: Vec<ContentSourceEdge>,
}

pub struct ContentSourceEdge {
    pub source: TypedDomainRecordRef<Content>,
    pub role: ContentSourceRole,
    pub completeness_per_mille: u16,
    pub fidelity_per_mille: u16,
}
~~~

Reference roles:

- creator: zero or one entity;
- source_content: zero or more content records, each mirrored by exactly one
  `ContentSourceEdge`.

An original content record has no source edge and no derivation. A derived
content record has at least one source edge and one derivation. Completeness and
fidelity are bounded by 1000 per edge. The role records whether a source is a
contribution, quotation, correction, contradiction, or context. These values
describe the relationship to each source, not objective truth.

`InlineJson` is bounded by the plugin's canonical payload-size limit.
`Resource` stores only a canonical SHA-256 digest, media type, and byte length;
Canwu does not persist paths, URLs, bucket keys, credentials, or other locators.
A host-owned non-authoritative `ResourceResolver` maps the digest to bytes when
presentation or an external interpreter needs them. Replay identity depends
only on digest, media type, and length. Missing bytes may prevent a new external
interpretation request, but never change already persisted authoritative state;
resolved interpretation output must return through canonical ingress.

Content is immutable. A correction, excerpt, summary, paraphrase, translation,
reconstruction, or fabricated variant creates another content record.

### Representation

~~~rust
pub enum ContentRelation {
    SameContent,
    DerivedContent,
}

pub struct RepresentationPayload {
    pub format: String,
    pub created_at: SimTime,
    pub operation: String,
    pub content_relation: ContentRelation,
    pub sources: Vec<RepresentationSourceEdge>,
    pub interpretation_capability: Option<String>,
}

pub struct RepresentationSourceEdge {
    pub parent: TypedDomainRecordRef<Representation>,
    pub completeness_per_mille: u16,
    pub fidelity_per_mille: u16,
}
~~~

Reference roles:

- content: exactly one content record;
- parent_representation: zero or more representation records;
- producer: zero or one entity.

An original representation has no source edge. A derived representation has at
least one source edge, and every parent reference is mirrored by exactly one
edge. Completeness and fidelity apply per parent.

For SameContent, every parent and child resolves to the same content record.
For DerivedContent, the child content must name every parent content in its
source_content references.

interpretation_capability is a canonical capability code. None means no
special interpretation step is required by the shared mechanism. It does not
assert that every actor actually understands the representation.

Representation is immutable.

A byte-for-byte or otherwise identical copy creates only a new `Instance`; it
does not manufacture a new representation lineage node. A new representation
is created only when encoding, format, selection, completeness, fidelity, or
content relation changes.

Lineage admission is topologically closed. Every content source or
representation parent must either resolve at the proposal-visible persisted
cut or name an earlier declared output slot of the same accepted information
operation. A slot may never reference itself or a later slot. InformationPlugin
sorts the operation's pending lineage nodes by declared output-slot index and
validates each edge against persisted nodes plus the already validated prefix;
this ordering is part of the operation schema and replay input. It rejects
self-edges, duplicate edges, unresolved refs, a content edge to a non-content
kind, a representation edge to a non-representation kind, and any full stored
plus pending graph that is not acyclic. Because immutable nodes can only point
backward under this rule, an accepted lineage cycle cannot be created and does
not require later repair.

### Instance

~~~rust
pub enum InstanceStatus {
    Available,
    Unavailable,
    LocationUnknown,
    Destroyed,
    Consumed,
}

pub struct InstancePayload {
    pub created_at: SimTime,
    pub status: InstanceStatus,
}
~~~

Reference roles:

- representation: exactly one representation;
- custodian: zero or one entity;
- location: zero or one entity.

Instance updates may change status, custodian, or location with an expected
version. `Unavailable` and `LocationUnknown` may return to `Available` through
an explicit recovery operation. `Destroyed` and `Consumed` are terminal.
Copying creates a new instance referencing the same representation unless the
copy operation actually changes representation or content; it does not remove
or alter the source instance unless a separate transition does so.

Channels without a persistent carrier create no instance record.

### Dispatch and delivery attempts

~~~rust
pub enum DispatchStatus {
    Prepared,
    Active,
    Completed,
    Cancelled,
}

pub enum DispatchTarget {
    Addressed(Vec<KnowledgeHolderRef>),
    Audience(TypedDomainRecordRef<Audience>),
    Open,
}

pub struct DispatchPayload {
    pub status: DispatchStatus,
    pub target: DispatchTarget,
    pub prepared_at: SimTime,
    pub dispatched_at: Option<SimTime>,
    pub completed_at: Option<SimTime>,
}

pub enum DeliveryAttemptStatus {
    Prepared,
    InTransit,
    Delivered,
    Failed,
    Cancelled,
}

pub struct DeliveryAttemptPayload {
    pub status: DeliveryAttemptStatus,
    pub attempt_number: u32,
    pub prepared_at: SimTime,
    pub dispatched_at: Option<SimTime>,
    pub due_at: SimTime,
    pub completed_at: Option<SimTime>,
}
~~~

Dispatch reference roles:

- channel: exactly one channel;
- representation: exactly one representation;
- source_instance: zero or one instance;
- sender: zero or one entity;
- intended_recipient: exactly the holders in an `Addressed` target and absent
  for `Audience` or `Open`.

Delivery-attempt reference roles:

- dispatch: exactly one dispatch;
- recipient: exactly one holder entity;
- previous_attempt: zero or one delivery-attempt record for the same dispatch
  and recipient;
- relay: zero or one entity;

Allowed transitions:

~~~text
Dispatch: Prepared -> Active -> Completed
Dispatch: Prepared|Active -> Cancelled
Attempt: Prepared -> InTransit
Attempt: Prepared|InTransit -> Delivered|Failed|Cancelled
~~~

Terminal dispatches and attempts do not reopen. `due_at` cannot precede
the eventual `dispatched_at`. `dispatched_at` is absent while prepared and
required from Active/InTransit onward. `completed_at` is absent before a
terminal transition and required at or after `dispatched_at` for a terminal transition. Attempt numbers
for one dispatch/recipient form a contiguous sequence beginning at one.

Terminal delivery attempts never close a dispatch automatically.
`complete_dispatch` is an explicit operation. An Active addressed dispatch may
create another contiguous retry even when all current attempts are terminal.
Closing requires every intended recipient to have an explicit final
disposition and no attempt to remain prepared or in transit. Once closed, no
new attempt is allowed. The aggregate does not imply universal success:
recipients may have different delivered, failed, or cancelled outcomes and
different completion times.

Only `Addressed` creates expected-recipient delivery attempts.
`Audience` and `Open` record actual reception through Access records, optionally
with delivery attempts for resolved recipients, without pre-enumerating every
possible listener. This supports broadcast, multicast, public availability,
and simultaneous interception using the same channel/dispatch model.

An access produced while an attempt is in transit does not change that attempt
or the dispatch. Successful delivery creates explicit access records for the
resolved recipient; it does not publish knowledge automatically.

For every versioned information record, InformationPlugin compares the complete
previous and proposed value. Allowed mutable fields are closed lists:

| Kind | Immutable after creation | Mutable through validated transition |
| --- | --- | --- |
| Instance | representation, created_at | status, custodian, location |
| Dispatch | channel, representation, source_instance, sender, target, prepared_at | status, dispatched_at, completed_at |
| DeliveryAttempt | dispatch, recipient, previous_attempt, relay, attempt_number, prepared_at, due_at | status, dispatched_at, completed_at |
| Release | representation, publisher, audience, scope, prepared_at | status, active_at |
| Operation | id, version, kind, input hash, admitted_at, accepted cause, declared output slots | status, result refs/IDs, continuation, completed_at, rejection code |

Any unlisted field change is rejected even if the new payload independently
passes schema validation. This prevents ordinary versioned mutation from
silently retargeting a dispatch, attempt, release, or operation.

### Access

~~~rust
pub struct AccessPayload {
    pub accessed_at: SimTime,
    pub method: String,
    pub extent_per_mille: u16,
}
~~~

Reference roles:

- holder: exactly one holder entity;
- representation: exactly one representation;
- instance: zero or one instance;
- dispatch: zero or one dispatch;
- delivery_attempt: zero or one delivery-attempt record;
- release: zero or one release.

At least one of instance, dispatch, delivery_attempt, or release must provide
context. Every supplied context must resolve to the same representation. A
direct dispatch context is valid only for an Active or Completed `Audience` or
`Open` dispatch; an `Addressed` dispatch requires its recipient-specific
delivery attempt. When both dispatch and delivery attempt are supplied, the
attempt must belong to that dispatch. Extent is bounded by 1000. Access records
are immutable.

Access has no result_content field and creates no holder knowledge by itself.
Applications may publish metadata knowledge such as awareness that a signal
exists without publishing its semantic content.

For an audience-scoped release or dispatch, the access operation also carries
the explicit member identity or membership proof required by the immutable
Audience snapshot. For an open dispatch or release, no membership proof is
required, but access remains an explicit authoritative record rather than
universal knowledge.

### Interpretation

~~~rust
pub enum InterpretationStatus {
    Failed,
    Partial,
    Succeeded,
}

pub struct InterpretationPayload {
    pub interpreted_at: SimTime,
    pub status: InterpretationStatus,
    pub capability: String,
    pub confidence_per_mille: u16,
}

pub enum InterpretationAuthority {
    HolderSelf,
    InstitutionalRole {
        assignment: DomainRecordVersionRef,
        authority_grant: String,
    },
    Delegated {
        evidence: EvidenceRef,
        authority_grant: String,
    },
}

pub enum DelegationEvidenceSelector {
    Command { producer_plugin: String, command_type: String },
    Ingress { producer_plugin: String, packet_type: String },
    DomainRecord { owner_plugin: String, kind: DomainRecordKind },
}

pub struct DelegationAuthorityGrant {
    pub code: String,
    pub selector: DelegationEvidenceSelector,
    pub claim_path: Vec<String>,
}

pub struct DelegationClaimV1 {
    pub format_version: u32,
    pub performed_by: EntityRef,
    pub performed_for: KnowledgeHolderRef,
    pub capabilities: Vec<String>,
    pub not_before: Option<SimTime>,
    pub expires_at: Option<SimTime>,
}
~~~

Reference roles:

- performed_by: exactly one entity;
- performed_for: exactly one holder entity;
- authority: exactly one `InterpretationAuthority`;
- input_access: one or more access records;
- input_representation: one or more representation records;
- result_content: zero or one content.

Every input representation must be named by at least one input access. Every
input access holder must equal `performed_for` or the entity in `performed_by`
when that entity can itself be represented as a knowledge holder. The performer
may therefore work from the commissioning holder's access, from their own
access, or from a shared institutional holder. Any broader delegation is not
inferred. `HolderSelf` requires `performed_by` to equal the holder. `InstitutionalRole`
requires the exact assignment record version and a named grant whose selector
matches that record kind. `Delegated` requires command, ingress, or exact
domain-record-version evidence matching the named `DelegationAuthorityGrant`.
Grants are sorted, unique, and committed in the InformationPlugin semantic
descriptor; the resolver checks the evidence producer and
command/packet/record kind against the exact selector.

The grant's `claim_path` is one to eight canonical non-empty object keys; array
indexes, escapes, wildcards, and missing/intermediate non-object values are
invalid. The resolver follows that path in the persisted command, ingress, or
domain-record payload and decodes exactly `DelegationClaimV1`. It requires
`format_version == 1`, exact equality between claim and Interpretation
`performed_by`/`performed_for`, a sorted unique capability list containing the
interpretation's capability, and `interpreted_at` within the half-open interval
`[not_before, expires_at)` when either bound exists. `expires_at <= not_before`
is invalid. `InstitutionalRole` uses the same claim extraction and binding
checks as `Delegated`; its separate variant records the policy reason, not a
weaker validation path.

A decision is not direct delegation evidence in V1: it must first produce an
admitted command or ingress, which is the reference consumed here.
InformationPlugin validates the authority relation before admission and stores
the evidence/assignment, grant code, and claim hash in the operation record and
audit origin; natural-language claims or a matching record kind alone do not
count.

Failed interpretations have no result_content. Partial and succeeded
interpretations require one result_content. The result may be the original
content or a new derived content record. Therefore a successful interpretation
can still be wrong relative to another source.

Interpretation records are immutable. A later reinterpretation creates a new
record and may lead to a knowledge record that contradicts or supersedes an
earlier one. Neither `performed_by` nor `performed_for` automatically learns the
result: publishing result knowledge requires an explicit interpretation record,
an eligible holder, and a separate `PublishKnowledge` batch.

### Audience

~~~rust
pub enum AudienceMembership {
    ExplicitMembers,
    ResolvedGroupSnapshot,
}

pub struct AudiencePayload {
    pub membership: AudienceMembership,
    pub resolved_at: SimTime,
    pub resolution_version: u64,
    pub resolved_boundary: Option<BoundaryId>,
    pub member_count: u64,
    pub membership_root: String,
}

pub struct AudienceMembershipLeafV1 {
    pub format_version: u32,
    pub holder: KnowledgeHolderRef,
}

pub struct AudienceMembershipProofV1 {
    pub holder: KnowledgeHolderRef,
    pub leaf_index: u64,
    pub member_count: u64,
    pub sibling_hashes: Vec<String>,
}
~~~

Reference roles:

- member: zero or more holder entities;
- group: zero or more downstream group entities;
- membership_evidence: zero or more exact domain-record-version or ingress
  evidence references.

Audience records are immutable snapshots of a distribution scope.
`ExplicitMembers` requires a bounded, sorted, complete member list and no group.
`ResolvedGroupSnapshot` requires a group reference, exact resolution version,
membership root, member count, and either a bounded complete member list or a
verifiable membership proof supplied with each access operation. The shared
extension never reads current world membership to reinterpret an old audience.
An access request must provide either a listed member or a proof matching the
stored group version/digest. If the proof is not available, access is rejected;
it is never guessed from current membership. This supports large audiences
without making group-only metadata look like an enforceable permission.

The default implementation uses bounded explicit members for small audiences
and versioned group snapshots with proof-carrying access for large audiences.
No unbounded person list is copied into every release or knowledge record.

Membership commitment V1 sorts and deduplicates holders by their canonical
`KnowledgeHolderRef` order and requires `member_count > 0`. Each leaf is the
decoded 32-byte result of
`canonical_hash("canwu.audience.member.leaf.v1",
AudienceMembershipLeafV1 { format_version: 1, holder })`. Interior nodes are
`BLAKE3("canwu.audience.member.node.v1" || 0x00 || left[32] || right[32])`;
odd levels duplicate the final node. `membership_root` is lower-case hex for
the final 32 bytes. Explicit lists are rehashed and must equal the stored root.

`AudienceMembershipProofV1` is carried by the access command/ingress when the
holder is not present in a stored explicit list. Its holder must equal the
Access holder, its count must equal the Audience count, and its index must be
less than that count. Every sibling is lower-case 32-byte hex; index and count
determine left/right placement and duplicated-odd steps. V1 accepts at most 64
siblings and 8,192 canonical JSON bytes. A wrong depth, non-canonical hash,
duplicate explicit member, count/root mismatch, or proof mismatch is rejected
atomically. The immutable Audience record plus persisted access operation input
therefore makes membership validation deterministic in replay without reading
current group state.

### Release

~~~rust
pub enum ReleaseStatus {
    Prepared,
    Active,
    Withdrawn,
    Expired,
}

pub enum ReleaseScope {
    Audience,
    OpenAvailability,
}

pub struct ReleasePayload {
    pub status: ReleaseStatus,
    pub scope: ReleaseScope,
    pub prepared_at: SimTime,
    pub active_at: Option<SimTime>,
}
~~~

Reference roles:

- representation: exactly one representation;
- publisher: zero or one entity;
- audience: exactly one audience record when scope is Audience, otherwise zero.

Audience releases require one audience. `OpenAvailability` requires none. Open
availability means that an application may resolve access; it does not mean
every holder has accessed or learned the content. Withdrawing or expiring a
release prevents new access through that release but never deletes access or
knowledge already acquired.

Allowed transitions:

~~~text
Prepared -> Active
Prepared -> Withdrawn
Active -> Withdrawn
Active -> Expired
~~~

Release activation may schedule distribution work or create access for
explicitly resolved actors. It never directly writes every actor's knowledge.

### Authoritative operation ledger

Every accepted information request has a stable operation identity and a
persisted state machine:

~~~rust
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct InformationOperationId {
    pub namespace: String,
    pub value: String,
}

pub enum InformationOperationStatus {
    Accepted,
    ApplyingDomainChanges,
    AwaitingPublication,
    AwaitingFinalization,
    Completed,
    Rejected,
}

pub enum InformationAdmissionRef {
    Command(CommandId),
    Ingress(IngressId),
}

pub struct InformationOperationPayload {
    pub id: InformationOperationId,
    pub operation_version: u32,
    pub operation_kind: String,
    pub canonical_input_hash: String,
    pub status: InformationOperationStatus,
    pub admitted_at: SimTime,
    pub accepted_cause: InformationAdmissionRef,
    pub domain_result_refs: Vec<DomainRecordRef>,
    pub publication_result_ids: Vec<KnowledgeRecordId>,
    pub continuation: Option<InformationContinuation>,
    pub completed_at: Option<SimTime>,
    pub rejection_code: Option<String>,
}
~~~

The operation record is a versioned `canwu.information.operation` domain
record. Its stable record ID is a canonical encoding of
`InformationOperationId`. Every other output reference is derived from
`operation_id + output_slot` by the extension; callers cannot choose output IDs
or overwrite a record produced by another operation. Output slots are declared
by each versioned operation kind and are stable replay inputs.

The state machine is authoritative:

- admission creates the operation in `Accepted` or persists a deterministic
  `Rejected` outcome;
- phase 7 moves it through `ApplyingDomainChanges` to
  `AwaitingPublication`, atomically creating/updating its declared domain
  outputs;
- phase 13 publishes generic information knowledge and moves it to
  `AwaitingFinalization`;
- the publication batch carries the operation record/version in audit origin;
- the publication batch carries the canonical operation ID as its audit-only
  `producer_correlation`;
- InformationPlugin schedules a zero-delay self-ingress for the next admission
  cut; its next phase-7 run resolves the prior boundary's unique producer
  correlation to a `BoundaryKnowledgeChange`, writes that change's exact global
  IDs into `publication_result_ids`, clears continuation state, and marks the
  operation `Completed`;
- a phase-4 publication is allowed only for an operation or admitted result
  that does not depend on same-boundary phase-7 output. The first information
  extension milestone uses phase 13 for domain-backed operations.

An exact retry compares the canonical input hash before any mutation. It
returns the stored operation reference, status, domain result references, and
available publication IDs. A conflicting retry fails with
`IdempotencyConflict`. Retrying an in-progress operation does not schedule a
second continuation. A rejected operation is terminal and replayable. An
unexpected internal failure rolls the whole boundary back; it does not persist
a partial operation transition.

The operation index is reconstructed from operation records after load. Live
compaction retains the same compact request reservation used by other canonical
ingress plus the latest operation record; exact retry remains possible after
older boundary evidence is sealed. Operation payload, transitions, result
references, continuations, and finalization events enter record commitments,
boundary history, validation, replay, and tamper tests.

### Lifecycle planning API

Expose owned request and result values suitable for bindings:

~~~rust
pub struct InformationMutationPlan {
    pub mutations: Vec<DomainRecordMutation>,
    pub publications: Vec<GenericInformationPublicationDraft>,
}

pub struct InformationLifecycle;

impl InformationLifecycle {
    pub fn create_content(...) -> Result<InformationMutationPlan, CanwuError>;
    pub fn create_representation(...) -> Result<InformationMutationPlan, CanwuError>;
    pub fn create_instance(...) -> Result<InformationMutationPlan, CanwuError>;
    pub fn begin_dispatch(...) -> Result<InformationMutationPlan, CanwuError>;
    pub fn begin_delivery_attempt(...) -> Result<InformationMutationPlan, CanwuError>;
    pub fn complete_delivery_attempt(...) -> Result<InformationMutationPlan, CanwuError>;
    pub fn complete_dispatch(...) -> Result<InformationMutationPlan, CanwuError>;
    pub fn record_access(...) -> Result<InformationMutationPlan, CanwuError>;
    pub fn record_interpretation(...) -> Result<InformationMutationPlan, CanwuError>;
    pub fn create_audience(...) -> Result<InformationMutationPlan, CanwuError>;
    pub fn activate_release(...) -> Result<InformationMutationPlan, CanwuError>;
}
~~~

Each method is pure:

- it receives current decoded records and an owned request;
- it validates lifecycle and cross-record invariants;
- it returns ordered mutations and optional knowledge drafts;
- it does not allocate engine IDs, read hidden state, draw randomness, or
  mutate Canwu.

Only `InformationPlugin` converts mutations to `MutateRecord` directives and
generic information publication drafts to `PublishKnowledge` directives. The
draft enum is closed over the neutral schemas owned by the plugin; it cannot
carry a caller-selected `KnowledgeSchemaId`. Application-semantic publications
are emitted by the application plugin under its own schema ownership grant.
The information plugin splits phase-7 domain mutations from phase-4/13
publications and carries the operation identity through persisted
command/ingress evidence. No helper returns a raw directive under a
caller-selected plugin name.

Pure lifecycle functions remain independently testable, but runtime validity
does not rely on callers choosing to use them: the authoritative plugin invokes
the same validation before emitting every mutation plan. Kernel
`CreateOnly` schema policy is the final guard for immutable record kinds.

### Commands, ingress, and decisions

`canwu-information` defines one canonical operation envelope with versioned
variants for create, derive, dispatch, attempt, access, interpret, audience, and
release operations. It is transported by a plugin command for synchronous
authorized requests or a plugin ingress packet for passive/external results.
The operation carries a stable application-supplied operation ID used for
idempotency, audit evidence, and operation-keyed randomness.

Case or application plugins define actor-visible actions and decide whether
they are permitted, affordable, timely, or likely. They do not mutate
information records directly. Once resolved, they submit the canonical
information operation. `InformationPlugin` rejects a duplicate operation ID
whose canonical input hash differs and returns the original result for an exact
retry.

Host routing is supported but is not the only composition path. Add a generic
next-boundary cross-plugin ingress directive:

~~~rust
BoundaryDirective::SchedulePluginIngress {
    target_plugin: String,
    after: SimDuration,
    packet_type: String,
    priority: i32,
    payload: Value,
    affected: Vec<EntityRef>,
}
~~~

The producer contract declares `plugin_ingress_targets` as exact
`(target_plugin, packet_type)` capabilities. The target plugin must own a
matching ingress descriptor and validates the payload schema. The kernel
persists producer and target identity, cause, phase, visibility, input hash,
and generated ingress ID. Even a zero-delay request is eligible only at the
next admission cut; there is no synchronous cross-plugin recursion. Existing
`ScheduleIngress` remains the self-targeted shorthand. This allows battle,
diplomacy, economy, or other packages to request a canonical information
operation without moving authoritative game-rule orchestration into the host.

Passive delivery, observation, audience resolution, and external
interpretation results enter through canonical ingress. External tools must
persist their resolved result before it can affect information records or
holder knowledge.

When the decision framework is active:

1. a domain plugin creates a decision ticket from holder-visible facts;
2. a controller selects an already declared option;
3. the accepted decision emits or derives a canonical information plugin
   command;
4. phase 7 creates or updates information records through InformationPlugin;
5. phases 4 or 13 publish resulting holder knowledge;
6. exact replay uses persisted decisions, commands, ingress, and boundary
   records and never calls the policy again.

The information-flow milestone does not require DecisionTicket and can be
implemented and tested with deterministic scripted commands.

### Operation-keyed randomness

The shared lifecycle uses no randomness. Applications that resolve uncertain
operations declare ordinary versioned `RandomStreamKey` capabilities, but call
a new keyed API when the outcome must be independent of unrelated work:

~~~rust
pub enum RandomOperationTarget {
    Entity(EntityRef),
    DomainRecord {
        record: DomainRecordRef,
        version: u64,
    },
    KnowledgeHolder(KnowledgeHolderRef),
    CanonicalKey(String),
}

pub struct RandomOperationAddressV1 {
    pub producer_plugin: String,
    pub operation_kind: String,
    pub application_operation_id: String,
    pub target: RandomOperationTarget,
    pub draw_slot: u32,
}

pub enum RandomDrawAddress {
    Sequential { position: u64 },
    OperationV1(RandomOperationAddressV1),
}

pub struct RandomDrawRecordV5Fields {
    pub address: RandomDrawAddress,
    pub operation_evidence: Option<EvidenceRef>,
}

pub fn random_range_for_operation(
    &self,
    stream: &RandomStreamKey,
    evidence: EvidenceRef,
    operation_kind: &str,
    application_operation_id: &str,
    target: RandomOperationTarget,
    draw_slot: u32,
    upper_exclusive: u64,
    purpose: &str,
) -> Result<u64, CanwuError>;
~~~

The public call does not accept `producer_plugin`; the execution context
supplies it. It does accept explicit `EvidenceRef` because one boundary handler
may process several admitted items in one invocation. The kernel proves that
the evidence is exact, belongs to the proposal-visible cut, and is an allowed
cause form for the declared operation kind. A current unfinished boundary
cannot be used as `Boundary`, while a pending exact event or domain-record
version from an earlier visible stage may resolve through the proposal evidence
resolver. Scheduled continuation normally cites its ingress.

Evidence identity and entropy identity are deliberately separate. Sequentially
allocated command, event, ingress, boundary, draw, and change indexes are
persisted in `RandomDrawRecord.operation_evidence` for authorization, audit,
replay, and idempotency-conflict checks, but are not encoded into
`RandomOperationAddressV1` or the random value. The stable application operation
ID must be unique within `(producer_plugin, operation_kind)` and survives retry;
target and draw slot distinguish outputs within that operation. Reusing the
same entropy address with different evidence is an idempotency conflict. Thus
inserting an unrelated admitted item can renumber evidence without changing an
existing operation's outcome.

The existing general `RandomDrawRecord.cause: CauseRef` remains for the engine's
causal journal. Format 5 replaces `position` with `address` and adds
`operation_evidence`: it is absent for `Sequential` and required for
`OperationV1`. This preserves migrated sequential/core-system draws while giving
keyed validation the exact admitted evidence that is intentionally excluded
from entropy.

The kernel derives the result from the root seed, algorithm version, complete
stream key, complete operation address, bound, and canonical purpose hash. It
does not consume or advance the stream's sequential position. In 0.5,
`RandomDrawRecord.position` is replaced by the tagged `RandomDrawAddress`; old
draws migrate to `Sequential`. The persisted `OperationV1` variant permanently
selects both `RandomOperationAddressV1` and `Blake3OperationV1`; it is not
interpreted through the stream's separate sequential `RandomAlgorithm` value.
This avoids an ambiguous sentinel position and makes keyed/sequential
validation exhaustive.

`Blake3OperationV1` is an exact binary algorithm contract. It does not use
serde or JSON. Integers are unsigned little-endian. `put_text` writes a `u32`
byte length followed by the exact UTF-8 bytes. Text must be non-empty, trimmed,
and within its declared byte limit; V1 performs no Unicode normalization, so
different byte sequences are different keys. Enums use the fixed `u8`
discriminants below and never Rust's implicit discriminant.

The hash input is encoded in this exact order:

~~~text
"canwu.random.operation.v1" 0x00
algorithm = 0x01
root_seed:u64
put_text(stream.namespace)
put_text(stream.name)
stream.version:u32
put_text(address.producer_plugin)
put_text(address.operation_kind)
put_text(address.application_operation_id)
encode_target(address.target)
address.draw_slot:u32
upper_exclusive:u64
purpose_hash:[u8; 32]
candidate_index:u32
~~~

Target discriminants are Entity `0x01`, DomainRecord `0x02`, KnowledgeHolder
`0x03`, and CanonicalKey `0x04`. A domain-record target encodes
`put_text(record.kind.namespace)`, `put_text(record.kind.name)`,
`put_text(record.id)`, and `version:u64`; version zero is invalid. It excludes
`DomainRecordVersionSource`, because boundary/change indexes are audit evidence,
not stable entropy identity.

Core entity discriminants are Army `0x01`, Government `0x02`, Organization
`0x03`, Person `0x04`, Resource `0x05`, Route `0x06`, Territory `0x07`, and
Domain `0x08`; numeric identities are `u64`, while Domain uses
`put_text(namespace)`, `put_text(kind)`, `put_text(id)`. A holder begins with
Person `0x01` + person ID or Entity `0x02` + entity encoding. Unknown
discriminants are invalid and additions require V2.

The purpose hash input is exactly
`"canwu.random.purpose.v1" || 0x00 || put_text(purpose)` and its raw 32-byte
BLAKE3 digest is embedded above; the persisted diagnostic form is lower-case
hex. The result algorithm is:

1. require `upper_exclusive > 0`, set `candidate_index = 0`, and validate all
   bounded text;
2. encode the bytes above and compute BLAKE3;
3. interpret the first eight digest bytes as a little-endian `u64` candidate;
4. let `range = 2^64` and
   `accept_limit = floor(range / upper_exclusive) * upper_exclusive`; accept a
   candidate below that limit and return `candidate % upper_exclusive`;
5. otherwise increment `candidate_index` and repeat, failing with
   `IdentifierExhausted` if its `u32` space is exhausted.

Validation recomputes the value from these exact bytes. A future hash, encoding,
or reduction rule requires a new persisted address variant such as
`RandomDrawAddress::OperationV2` plus the corresponding engine/format identity;
adding a sequential-stream `RandomAlgorithm` variant cannot reinterpret an
existing `OperationV1`. Checked-in golden vectors include every target variant,
UTF-8 text, an upper bound that forces rejection in the fixture, and independent
expected input bytes, digest, candidate index, and result. A separate vector
changes only the sequentially allocated evidence ID and proves identical input
bytes and output.

The tuple `(stream, operation_address)` is globally unique in one run. Exact
retries require the same evidence, bound, and purpose and return the existing
result. Reuse with different evidence, bound, or purpose is an idempotency
conflict. The uniqueness check covers both the retained journal and draws
pending in the current boundary. A runtime index is rebuilt from the draw
journal after a full load.

For live compact continuation, add:

~~~rust
pub struct KeyedDrawReservation {
    pub stream: RandomStreamKey,
    pub address: RandomOperationAddressV1,
    pub upper_exclusive: u64,
    pub purpose_hash: String,
    pub result: u64,
    pub draw_id: RandomDrawId,
    pub operation_evidence: EvidenceRef,
    pub draw_receipt: ArchivedEvidenceReceipt,
}
~~~

`seal_evidence` retains one sorted reservation for each sealed keyed draw that
may be retried. Reservations are stored in the compact checkpoint continuation
state, are covered by a skipped-when-empty keyed-reservation commitment root,
and are rebuilt and byte-compared when checkpoint plus evidence segments are
rejoined. They are not emitted as duplicate `RandomDrawRecord`s in a full
snapshot. Tampering with address, bound, purpose, result, draw ID, operation
evidence, draw receipt, or reservation ordering invalidates the checkpoint.
The generic dependency map separately retains or receipts the referenced
operation evidence; `draw_receipt` proves the sealed draw record itself. This
makes exact retry and conflict detection survive repeated seal/restore cycles.

Information applications use separate stream identities such as
`access_resolution`, `copy_resolution`, `interpretation_resolution`, and
`distribution_resolution`. They validate and persist authoritative admitted
evidence, while entropy uses only producer, versioned operation kind, stable
information operation ID, stable target, and schema-declared draw slot. Adding,
removing, or reordering a different observer, dispatch, or presentation query
therefore cannot alter an existing operation's result. A failed access attempt
may create domain-specific attempt evidence, but it must not create an `Access`
record.

## Anonymous public cases

The case repository and executable fixtures contain no source-specific proper
nouns, period labels, conflict names, dates, places, organizations, or
explanations of their inspiration.

### Case A: Confidential Copy and Selective Release

Stable case ID:

~~~text
case.information.confidential-copy-release
~~~

Roles:

- originator
- designated_recipient
- carrier
- collector
- secondary_reader_a
- secondary_reader_b
- unrelated_actor

Initial records:

- channel.manual_carrier with a persistent instance, addressed recipients, and
  nonexclusive access support;
- content.primary;
- representation.primary carrying content.primary;
- instance.primary in the carrier's custody.

Timeline:

1. At minute 0, originator starts dispatch.primary and
   delivery_attempt.primary.1 for designated_recipient.
2. At minute 120, collector gains nonexclusive access to instance.primary.
3. The access does not change the dispatch, delivery attempt, source instance,
   or designated recipient.
   designated recipient.
4. collector creates representation.copy with SameContent lineage from
   representation.primary and creates instance.copy.
5. collector creates content.selection derived from content.primary with
   operation selective_extract.
6. collector creates representation.selection from representation.copy with
   DerivedContent lineage to content.selection.
7. At minute 180, release.selection becomes active for the two secondary
   readers.
8. Explicit Access records are created only for those two readers.
9. Phase 13 publishes selected-content knowledge to those readers.
10. At minute 240, delivery_attempt.primary.1 becomes Delivered, creates access
    for the designated recipient, and dispatch.primary becomes Completed.
11. Phase 13 publishes primary-content knowledge to the designated recipient.

Required observations:

- designated_recipient sees primary content but not the hidden copy operation;
- secondary readers see the selected derivative and its local confidence, not
  the primary representation;
- originator is not automatically informed about access, copying, or release;
- unrelated_actor sees neither release nor content;
- the intended delivery succeeds even though another holder accessed it in
  transit;
- representation and content lineage survive save/load and exact replay;
- no standing, reputation, political, or military consequence is implemented
  in the shared engine or extension.

Suggested files after implementation:

~~~text
crates/extensions/canwu-information/examples/confidential_copy_release.rs
website/src/content/docs/tutorials/cases/confidential-copy-release.mdx
website/src/content/docs/en/tutorials/cases/confidential-copy-release.mdx
~~~

Chinese title: 机密副本与选择性发布.

### Case B: Encoded Interception and Restricted Dissemination

Stable case ID:

~~~text
case.information.encoded-interception
~~~

Roles:

- sender
- intended_recipient
- monitor
- interpreter
- reviewer
- unrelated_actor

Initial records:

- channel.rapid_addressed with no persistent instance and addressed
  recipients;
- content.primary;
- representation.encoded requiring capability.decode_alpha.

Timeline:

1. At minute 0, sender starts dispatch.primary and
   delivery_attempt.primary.1 for intended_recipient.
2. At minute 10, monitor gains access to representation.encoded while the
   delivery attempt remains InTransit.
3. monitor lacks the required capability. The application records a failed
   interpretation with no result content.
4. Phase 13 may publish metadata knowledge that an encoded representation was
   observed. It does not publish content.primary.
5. At minute 20, intended_recipient gains access,
   delivery_attempt.primary.1 becomes Delivered, and dispatch.primary becomes
   Completed.
6. interpreter performs a successful interpretation with
   `performed_for = intended_recipient`, consuming the intended recipient's
   access and producing interpretation.primary. The interpreter is not made a
   knowledge holder by this operation.
7. Phase 13 publishes content knowledge only to the intended recipient.
8. A restricted release makes a review representation available to reviewer.
9. reviewer receives explicit access and knowledge.
10. unrelated_actor remains without access or knowledge.

Required observations:

- interception does not block intended delivery;
- access without capability does not expose semantic content;
- failed interpretation records metadata but no result content;
- restricted dissemination does not become universal visibility;
- reviewer knowledge is derived from the released representation, not from
  omniscient state;
- save/load and exact replay do not rerun an external decoder or policy.

Suggested files after implementation:

~~~text
crates/extensions/canwu-information/examples/encoded_interception.rs
website/src/content/docs/tutorials/cases/encoded-interception.mdx
website/src/content/docs/en/tutorials/cases/encoded-interception.mdx
~~~

Chinese title: 编码传输、截收与限域分发.

### Internal conformance profiles

These profiles are executable conformance evidence but not public narrative
cases.

#### Ephemeral multi-observer

Stable profile ID:

~~~text
fixture.information.ephemeral-multi-observer
~~~

It uses one representation, no Instance records, one channel with simultaneous
observation, and several Access records at the same simulation time. It proves
that the public model does not require a unique physical original, a carrier,
or one recipient.

If this profile requires changing the shared public types after the two public
cases work, implementation stops and the model is redesigned.

#### Partial multi-recipient delivery

Stable profile ID:

~~~text
fixture.information.partial-multi-recipient
~~~

One dispatch names three holders. One delivery attempt succeeds immediately,
one fails and later succeeds on attempt two, and one remains in transit. It
proves that dispatch state does not collapse per-recipient progress, retry
history, or completion time into one status.

#### Institutional holder succession

Stable profile ID:

~~~text
fixture.information.institutional-holder
~~~

Knowledge is published to an institutional domain entity, queried through an
institution-bound seat, and remains attached to that stable entity when the
responsible person changes. Neither the predecessor nor successor receives a
private personal copy automatically.

#### Collector and interpreter separation

Stable profile ID:

~~~text
fixture.information.delegated-interpretation
~~~

One holder owns the input access, a distinct performer interprets it for that
holder, and only an explicit publication grants result knowledge. It proves
that access holder, performer, commissioning holder, and knowledge recipient
do not collapse into one `PersonId`.

#### Multi-hop relay and provenance

Stable profile ID:

~~~text
fixture.information.multi-hop-relay
~~~

A representation passes through two dispatches and three delivery attempts,
including one destroyed instance after a copy. Lineage and evidence remain a
DAG, source delivery remains intact, and current queries can retrieve both the
latest head and full history.

#### Open fan-out external resource

Stable profile ID:

~~~text
fixture.information.open-fanout-resource
~~~

One digest-addressed external resource is represented on an open-reception
channel. Ten thousand holders may acquire access over several boundaries; the
release itself creates no per-holder access or knowledge. The profile proves
that large fan-out uses explicit access, deterministic continuation chunks, and
a resource digest without persisting a machine-local locator or multiplying the
content body in the snapshot.

#### Claimed source versus audit origin

Stable profile ID:

~~~text
fixture.information.claimed-source-divergence
~~~

A representation payload contains a claimed source that differs from the
actor and operation that actually produced it. One holder accepts the claim;
another records a contradictory attribution. Holder views expose the claim as
application knowledge but not the protected audit origin. Research audit can
trace both. The profile proves that semantic attribution, factual truth, and
kernel provenance are not collapsed into one field.

## Conformance tests

### Core publication tests

Durable tests must prove:

- a phase-4 SameBoundary publication is readable in phase 5 but not by another
  phase-4 system;
- a phase-4 NextBoundary publication is hidden until the boundary completes;
- phase-13 publication observes committed phase-9 and phase-11 state;
- phase-13 knowledge publications share one input cut even though existing
  ordinary phase-13 directives keep their current settlement behavior;
- phase 7 cannot publish knowledge;
- an undeclared schema version or foreign schema owner is rejected;
- learned_at is engine-stamped and cannot be backdated;
- a character-bound seat cannot query another holder;
- an institution-bound seat can query its institution but not an arbitrary
  person;
- `PublicObserver` cannot query any private knowledge ledger even when passed a
  valid existing person ID;
- the restricted `CanwuViewer` exposes no raw snapshot, domain-record, event,
  boundary, or audit-ledger escape hatch;
- ordinary holder projections omit origin evidence and hidden lineage;
- diagnostic audit projection requires ResearchFull or DeveloperDiagnostic;
- a retired or deleted holder receives no new publication, keeps a historical
  audit ledger, and never transfers that ledger to a successor implicitly;
- corrections append and preserve the prior record;
- contradiction does not erase either record;
- a publication may cite a sealed exact evidence reference when its archived
  receipt validates, but a rule needing unavailable payload bytes fails closed;
- a failing later phase rolls back records, IDs, events, and boundary evidence;
- snapshot load, exact replay, fork, rollback, checkpoint/journal restore, and
  compact reconstruction preserve the same records and hashes;
- tampering with payload, holder, learning time, relations, producer, event,
  counter, or change index is rejected.

### Lifecycle tests

Durable tests must prove:

- content, representation, access, and interpretation records are immutable;
- derived-content lineage resolves to every parent content;
- SameContent representation lineage cannot point to a different content;
- create-only information kinds reject update, retire, and delete even when a
  faulty plugin handler submits a raw mutation;
- terminal dispatch, delivery-attempt, instance, and release states cannot
  reopen;
- one recipient's attempt outcome does not overwrite another recipient's;
- a retry creates a new attempt with a contiguous attempt number;
- an access record never changes dispatch or delivery status by itself;
- failed interpretation cannot have result content;
- successful interpretation requires result content;
- a distinct performer may interpret for the holder that owns the input access;
- the performer does not gain result knowledge automatically;
- open release creates no access or knowledge by itself;
- release withdrawal preserves previously acquired access and knowledge;
- explicit access is required before publishing content knowledge;
- one accepted operation owns deterministic output slots, survives exact retry,
  and cannot be resumed under a conflicting input hash;
- next-boundary cross-plugin ingress is validated by both producer grant and
  target descriptor and cannot recurse synchronously;
- every collection and encoded payload limit rejects atomically at its exact
  boundary;
- all domain mutation bundles remain atomic.

### Randomness and compact-continuation tests

Durable tests must prove:

- keyed addresses derive producer from execution context and persist admitted
  evidence without using sequential evidence IDs as entropy;
- two plugins using the same caller operation text cannot collide;
- adding or reordering unrelated operations does not alter an existing keyed
  result or a legacy sequential stream position;
- exact keyed retry returns the original draw before and after evidence sealing;
- changed bound, purpose, target, operation kind, or evidence rejects as an
  idempotency conflict;
- checkpoint/segment reconstruction rebuilds identical keyed reservations;
- tampering with an archived evidence receipt or keyed reservation invalidates
  the compact checkpoint.

### Metamorphic tests

| Change to input | Must remain unchanged |
| --- | --- |
| Add an unrelated observer | dispatch, delivery, access, interpretation, and keyed random results |
| Disable interpretation | access and delivery outcomes |
| Lower representation completeness | content identity and dispatch status |
| Disable release | primary delivery and recipient knowledge |
| Change only channel delay | content and representation lineage |
| Add or reorder an unrelated dispatch | existing operation-keyed outcomes and sequential stream positions |
| Fail interception | intended delivery |
| Destroy a copied instance | source instance and source dispatch |
| Save and restore at any admitted cut | final state, evidence, IDs, and hashes |
| Reorder initial record insertion | canonical final state and hashes |

### Anonymous-surface checks

The fixtures use an allowlist approach:

- case and record IDs use lower-case neutral capability names;
- actor display names are role labels;
- scenario times are relative minutes from epoch;
- locations are node_a, node_b, and node_c;
- payload values are synthetic claim codes;
- comments describe mechanics only;
- the public case pages contain no inspiration or source-mapping section.

A pre-merge repository search checks source-specific terms supplied outside the
repository. The terms themselves are not committed to the Canwu tree.

## Performance and storage

The largest long-run cost is append-only holder knowledge. The first
implementation must measure:

- record count by holder and schema;
- serialized snapshot growth at 10,000, 100,000, and 1,000,000 knowledge
  records;
- holder current-head, delta, and full-history query time for 100, 1,000,
  10,000, and 100,000 records;
- boundary publication cost for batches of 1, 10, 100, and 1,000 records;
- a 10,000-recipient addressed dispatch and a 10,000-member resolved audience;
- save/load, compact seal/restore, and exact replay cost for both anonymous
  cases and the one-million-record synthetic profile.

The promotion corpus includes at least 1,000 holders, 100 schemas, 100,000
records on one hot holder, one million total records, ten thousand publications
at the boundary-wide cap, and a journal spanning at least 100 evidence segments.
Measurements record wall time, peak resident memory, serialized bytes, and
index rebuild time. Performance budgets are written from the first baseline
before optimization; a change cannot claim success by silently reducing the
fixture size.

Persist only the ordered holder record maps in format 5. Optional in-memory
indexes by schema, subject, or current head may be rebuilt after load and must
not enter authoritative hashes.

Do not add automatic knowledge deletion or compaction in this milestone.
Historical pruning requires a separate archive contract because supersession,
contradiction, explanation, and replay may refer to older records.

## Change surface

| Area | Required changes |
| --- | --- |
| canwu-core | KnowledgeRecordId, KnowledgeRecordKind, KnowledgeSchemaId, KnowledgeHolderRef, KnowledgeRecordType |
| canwu-knowledge | subjects, audit origin, draft, stored/audience views, query and cursor types, generic holder ledger |
| canwu-event | KnowledgePublished event and fixed audience projection |
| canwu-sim plugin registry | knowledge schemas, ownership, descriptor serialization |
| canwu-sim boundary | PublishKnowledge, contract declarations, change evidence, receipt count |
| canwu-sim settlement | phase-4 and phase-13 staging, ID allocation, overlays, events, rollback |
| canwu-sim records | create-only mutation policy for immutable extension records |
| canwu-sim random | operation-keyed draws and idempotency index |
| canwu-sim state | next knowledge ID counter and rebuilt runtime indexes |
| canwu-sim validation | schema versions, holder store, subjects, versioned evidence, history reconstruction, counters, tamper checks |
| canwu-sim hashing | knowledge changes, counter root, exact empty-shape compatibility |
| canwu-sim migration/persistence | verified format-4 load, format-5 migration/write, checkpoint, segments, archived receipts, and compact reservations |
| canwu-sim replay | exact regeneration and comparison |
| canwu-api | re-exports, holder-relative query methods, restricted audit method |
| canwu-debug | display generic records through canwu-api only |
| docs | architecture, end-state, versioning, conformance, public API |
| canwu-information | authoritative plugin, record model, operations, and lifecycle helpers |
| website | two anonymous bilingual cases and case index updates |
| agent-interface | documentation map for the new public capability |

## Alternatives considered

### One universal Message record

Rejected. It has the smallest initial patch but accumulates optional fields for
carrier, encoding, copies, interpretation, audience, and public release. The
ordinary failure mode is a large record whose booleans encode mutually
inconsistent states.

### Put all information artifacts in the kernel

Rejected. It gives the engine ownership of media and historical assumptions,
increases snapshot and API compatibility burden, and prevents downstream
packages from using different content or channel models.

### Keep all holder knowledge in domain records

Rejected. A domain record query is an omniscient programmatic read and does not
enforce holder-relative projection. Every application would have to rebuild the
same privacy, persistence, replay, and tamper guarantees.

### Use plugin events as knowledge

Rejected. Event audience is a presentation policy, not a holder knowledge
store. Events do not provide typed corrections, contradictions, current actor
queries, or durable semantic state.

### Implement the two cases first and extract later

Rejected. The first case would establish accidental assumptions before the
second and counterexample profiles can constrain the model.

### Recommended option

Implement the smallest generic kernel publication and operation-keyed random
primitives, keep the authoritative information lifecycle in its published
extension, and require two public cases plus the structurally
different conformance profiles before further compatibility promotion.

This option has a larger proof burden than a case-specific plugin but preserves
reversibility and exposes overfitting before the API is stable.

## Stop conditions

Stop implementation and revise this design if any of the following occurs:

- a core type gains a media, period, institution, conflict, or narrative name;
- PublishKnowledge is required outside phases 4 and 13 to make a case work;
- access, interpretation, belief, and truth collapse into one status;
- a release directly writes every holder's knowledge;
- interception implicitly cancels or redirects the original dispatch;
- a correction requires mutating or deleting an older knowledge record;
- a shared function branches on a case or profile ID;
- any internal conformance profile cannot use the same public types;
- actor- or institution-bound queries can reach an unauthorized holder's
  record;
- `PublicObserver` can obtain a private record or audit origin;
- unrelated observers or dispatches shift operation-keyed random draws;
- a snapshot or replay can omit, recreate, or renumber knowledge records;
- a format-4 input is rewritten before its old commitments are validated under
  the 0.4 contract;
- an independent application cannot add a fourth channel profile without
  modifying the kernel types.

## Promotion gate

`canwu-information` is an official published crate. It remains optional and is
not re-exported by `canwu-api`; any future compatibility promotion requires:

- both anonymous public cases use the same public record and lifecycle types;
- all internal conformance profiles pass unchanged;
- all metamorphic tests pass;
- no case-specific branch exists in shared code;
- holder-relative projection, rollback, save/load, exact replay, compaction, and
  tamper tests pass;
- performance evidence shows ordered sparse storage is acceptable;
- an independent reviewer approves the public API, persistence, authority,
  determinism, and replay changes;
- a fourth synthetic consumer can implement a different information channel
  without changing shared types.

That evidence is separate from publication and from any future promotion of
stable parts into Canwu core.
