# Continuation Checkpoint

Status: implemented in 0.16.0; reviewed design delivered in two commits. The first
commit implements the three prerequisites and the resumable journal
commitments (Parts A, C, D, and F); the second implements sealing with work in
flight and the continuation checkpoint itself (Parts B and E). The 0.15.0
behaviour quoted as the baseline is the released one. The independent design
review and how each finding was resolved are recorded in
[Review record](#review-record).

This package closes downstream gap G-61 in `canwu-sim` and `canwu-api`, and
three prerequisites that have no downstream id:

1. **Seal-stable evidence identity.** A first operation-keyed draw that cites
   evidence a seal archived must be answered as an unsealed run answers it.
2. **A keyed-draw ledger.** Work outside settlement must stop growing with
   every operation-keyed draw ever made.
3. **Host parity for `CompactedCanwu`.** Every host operation that does not
   need full history, including every extension's host-facing ingress helper,
   must work on a compact runtime.

The release entry will be in [Versioning and Persistence](../versioning.md).
The contracts this design builds on are in
[boundary window](boundary-window.md),
[keyed-draw index, draw ID, and boundary profile](keyed-draw-index-and-settlement-profile.md),
and [same-instant supersession and unplaced persons](same-instant-supersession-and-unplaced-persons.md).

## Summary

A **continuation checkpoint** is a save from which a run continues without its
evidence history. It holds the current authoritative state, the small set of
evidence that work still in flight may read (the *continuation closure*), the
derived indexes a restore cannot rebuild without history, and resumable forms
of the journal commitments. It does not hold the boundary, event, command,
ingress, or draw records that nothing in flight can read. Restoring it needs no
evidence segment.

The checkpoint is a deterministic function of the run's state and history at a
cut. A run that never sealed, a run that sealed at any other cuts, and a run
restored from an earlier continuation checkpoint all write byte-identical
continuation checkpoints at the same cut. That is the contract the acceptance
tests check boundary by boundary.

To make that possible this design also:

- lets a seal proceed while canonical ingress is pending, by keeping the
  continuation closure in memory instead of requiring an empty queue;
- answers existence and time of every ordinal evidence ID (event, command,
  command attempt, ingress, boundary, random draw) from counters and a compact
  time index instead of retained records, so those answers no longer depend on
  sealing;
- replaces the keyed reservations with one persistent ledger of every drawn
  address, which checkpoints, seals, and forks no longer copy or scan;
- keeps the five append-only journal commitments in a resumable form, so a
  restored run extends exactly the same commitment stream and reaches the same
  `checkpoint_hash` at every later boundary;
- gives `CompactedCanwu` the host reads and ingress entry points `Canwu` has,
  behind one trait that extension helpers take instead of `&mut Canwu`.

This package changes no commitment root, boundary hash, or `checkpoint_hash`
value, and keeps snapshot format 8; 0.16.0's commitment format 5 comes from
the knowledge-ledger package. `authoritative_state_hash` becomes the commitment-format
state hash, so it no longer differs between a sealed and an unsealed run. The
checkpoint/evidence-journal format and the paged-checkpoint envelope move from
4 to 5 because the keyed reservations leave them. The continuation checkpoint
is a new format, version 1.

Canwu crates depend on the exact same version of each other, and a 0.16.0 run
never reads anything 0.15.0 wrote, so no format here needs a compatibility
path, a migration, or old-format decoding. Keeping a 0.15.0 hash is never a
requirement in itself; where a hash or format changes, the change is versioned
and stated.

## Baseline: what 0.15.0 does

- `SimulationCheckpoint` holds current state with empty journals plus the cut
  through the six journals. Every restore (`from_checkpoint_and_journal`,
  `from_paged_checkpoint_and_journal`, the compact variants) concatenates
  segments from the zero cut, rebuilds a full `SimulationSnapshot`, and runs
  full snapshot validation. A restore therefore takes the whole evidence
  history; a compact restore keeps it all in memory again.
- `seal_evidence` moves the whole retained tail into one segment, but only when
  the canonical ingress queue is empty and every retained command, attempt,
  ingress record, and event belongs to a completed boundary. Boundary emissions
  are admitted by the next boundary, and a running game always has pending
  ingress, so a busy run never reaches a sealable cut.
- Several plugins read the command record that caused an admitted plugin
  packet (`canwu-history-research`, `canwu-law`, `canwu-resource`,
  `canwu-technology`). `plugin_ingress_issuer` and `cancellable_plugin_ingress`
  read the producing boundary's `generated_ingress` to find the issuer of a
  boundary-generated packet. Both rely on the seal waiting.
- A restore rebuilds from the full journal: the boundary ledger within the
  schedule horizon, the keyed-draw index, the current-version provenance index,
  the pending ingress queue, and the commitment cache. The commitment cache
  holds five incremental BLAKE3 hashers over the command, attempt, event,
  ingress, and draw journals; their state cannot be exported, so only a full
  journal can recreate it.
- Existence of archived evidence is answered from kept receipts: a seal keeps
  the receipt of each declared dependency and each keyed reservation's draw.
  An archived item without a receipt does not exist for the runtime, and an
  archived item's time is unknown unless it is a record's current version.
- A fresh operation-keyed draw needs `Retained` evidence; archived evidence is
  refused, although an unsealed run and a run restored from sealed segments
  accept it.
- Keyed reservations (`KeyedDrawReservation`, each with a Merkle receipt) grow
  with every sealed keyed draw. `checkpoint()`, the paged checkpoint, and every
  seal clone, sort, and hash all of them; every reservation's evidence and draw
  are dependencies whose receipts are kept forever; `fork()` copies them; a
  checkpoint-journal restore builds the index four times.
- `CompactedCanwu` lacks the archive-provider attach, viewers, `fork`, and
  most evidence and record-version reads, and every extension helper that
  enqueues for the host takes `&mut Canwu`. An archive-backed resource or
  production run cannot be driven compactly at all.

## Terms used below

- **Cut.** An `EvidenceCursor` through the six journals. The *journal end* of
  a runtime is its total cut. Journal IDs are dense and equal positions, so
  the journal end is `next_*_id - 1` for each journal.
- **Settled cut.** The cut below which every record is settled: for events,
  commands, and command attempts, the boundary admission cursors
  (`admitted_event_count`, `admitted_command_count`,
  `admitted_attempt_count`); for ingress, boundaries, and random draws, the
  journal end.
- **Unadmitted tail.** The events, commands, and attempts between the settled
  cut and the journal end: the next boundary admits them. They are contiguous.
- **Ordinal evidence.** An `EvidenceRef` of kind `Event`, `Command`,
  `CommandAttempt`, `Ingress`, `Boundary`, or `RandomDraw`. A
  `DomainRecordVersion` reference is not ordinal: its identity names a
  record, a version, and a boundary change.
- **Continuation closure.** The evidence that work in flight may still read
  with its payload, defined below. It is a function of current state and the
  cut, never of when the run sealed.
- **Identity facts.** What an `IdentityOnly` dependency still needs after its
  payload is gone, beyond the ordinal ID: the membership of a non-current
  record version, and the provenance of an ingress record (the issuer, target
  plugin, packet type, and producing boundary of a boundary-generated packet,
  or that no boundary generated it).

## The rule plugins obey

The 0.15.0 seal-independence rule, made complete. A plugin obeys it when:

- **R1.** A boundary decision reads exact record versions only through
  `replay_stable_domain_record_version`, supersession targets only through
  `knowledge_heads`, generic evidence only for existence and time of ordinal
  references, and evidence payloads only inside its continuation closure.
  `knowledge_changes_by_correlation`, `transition_audits`, and any other read
  of retained boundary evidence never drive a decision.
- **R2.** A rule that will read an evidence payload at a later boundary
  declares it `PayloadRequired` (through
  `PayloadRequiredEvidenceContinuationV1` in a current record) for as long as
  it needs it. Pending ingress, the causes of pending work, and the
  unadmitted tail are in the closure without a declaration.
- **R3.** A rule that will later cite a non-current record version, or check
  an earlier admitted plugin packet's provenance, declares it `IdentityOnly`
  (through `IdentityEvidenceDependenciesV1`, or as a knowledge origin) for as
  long as it cites it.
- **R4.** A dependency is added only while its evidence is still readable:
  evidence produced in the boundary that adds it, a packet that boundary
  admits, a current or initial-scenario record version, or evidence already
  declared. A plugin that declares an ingress record a host cites checks it
  with `SimulationView::ingress_is_declarable` (queued, or admitted by the
  boundary being settled), or against its own current declarations; both
  answer alike in every run, while existence alone does not keep a sealed
  packet readable. `canwu-law` refuses any other cited packet as a recorded
  refusal.
- **R5.** A fresh operation-keyed draw cites seal-stable operation evidence
  (Part A).

The first-party extensions follow R5, and R1 to R3 with two known
exceptions. The exception the design review found, `canwu-history-research`'s
relation check, is fixed in this package (Part A). Two more, found in the
review of the first commit, are left to their own package and stay known
violators until it lands:

- `canwu-information`'s delegated interpretation authority reads the payload
  of a delegation's command or ingress evidence through
  `SimulationView::command` and `SimulationView::ingress` at a later
  boundary without declaring it `PayloadRequired` (R2). After that evidence
  is sealed, and in a continuation-restored run, the read returns
  `EvidenceContentUnavailable` where an unsealed run returns the payload.
- `canwu-law`'s signal and publicity provenance checks call
  `plugin_ingress_matches` and `plugin_ingress_payload_matches` on ingress
  admitted at an earlier boundary without declaring it (R3, and R2 for the
  payload match). After a seal they answer only for packets the closure
  keeps.

R4 stays a plugin obligation. Part B pins at boundary commit
every version a boundary stops being current, which covers the common case of
a dependency declared on the version a boundary supersedes; a seal or a
continuation checkpoint still reports any other R4 violation as
`ArchiveNotReady`. Engine enforcement of R4 is deferred.

## Part A: seal-stable evidence identity (prerequisite)

### Problem

Whether an archived ordinal reference exists, and when it became
authoritative, depends on whether a seal happened to keep its receipt. A fresh
operation-keyed draw citing evidence a seal archived is refused in the sealed
run and accepted by the unsealed run and by a run restored from the sealed
segments, so a sealed run and its exact replay can disagree.

### Decision

**Ordinal existence comes from counters.** Ordinal journals are dense and IDs
equal positions, so an ordinal reference with `0 < id <= archived cut` existed
when it was sealed and exists for ever. `RuntimeValidationContext` returns
`Archived` for every such ID, with or without a receipt, and `Missing` only
for zero, an ID at or beyond the next counter, or an unretained ID above the
archived cut. An unsealed run has an archived cut of zero and answers as
before; a sealed run and a continuation-restored run now answer as the
unsealed run does. This governs `evidence_exists`, cause validation, knowledge
origins, directive evidence, and the fresh-draw rule below.

**A seal accepts what the counters prove.** Because the counter rule accepts a
citation of sealed evidence that has no receipt, a knowledge origin or an
identity-only declaration (for example `canwu-law`'s `verify_evidence`
declarations) can name an event, command, attempt, ingress record, boundary,
or draw for the first time after the seal that archived it. A seal, one-phase
or two-phase, therefore requires a receipt only where a rule reads a fact from
it: a `PayloadRequired` dependency, the membership of a domain-record version
a boundary established, and the ingress provenance of an archived command or
decision request (all of which have their receipt from the seal that archived
them). Every other dependency is an identity-only ordinal reference that the
counters prove at or below the cut, or an initial-scenario version, and needs
none. A seal still keeps the receipts of the dependencies current at that
seal, so a checkpoint may list fewer receipts than its segments rebuild; a
checkpoint-and-journal restore requires each listed receipt to equal the one
the segments rebuild and to name a current dependency, instead of requiring
the two sets to be equal. In the second commit the seal no longer reads
receipts at all: every fact a rule needs comes from the continuation closure
(Part B), which a continuation-restored run, holding no receipts, has as
well. A declared dependency must still exist (counters for ordinal
references, the current index, pins, or facts for versions), or the seal
fails with `ArchiveNotReady` and the run is unchanged. `plugin_ingress_matches`
on a packet first cited after its seal has no fact to read and returns
`EvidenceContentUnavailable`.

**Ordinal time comes from a time index.** A retained record answers with its
own time (events by `timestamp`, commands by `accepted_at`, attempts by `at`,
ingress by `issued_at`, boundaries and draws by `at`). For sealed records the
runtime keeps, per journal, a run-length list of `(first ID, time)`: a new
entry starts whenever the time changes along the journal, and an ID's time is
the entry with the largest first ID at or below it. This is exact for any
order of times (ingress, for one, is ordered by eligibility and issue time,
not by time alone), needs no assertion that could fail a boundary, and has one
representation. A seal extends it from the records it moves out; no other
transaction touches it, so a failed boundary has nothing to roll back; a fork
shares it (`im::Vector`); a full restore, which retains everything, starts it
empty. A continuation checkpoint carries it up to the settled cut. Journals
that append in time order keep about one entry per distinct instant (for daily
boundaries about 6 × 365 entries a year); that bound is an expectation the
measurement checks, not a guarantee. `evidence_time` of a boundary-change
record version is its boundary's time whenever the version's existence is
known.

**Fresh operation-keyed draws use seal-stable evidence.** An address that has
not been drawn may cite as operation evidence:

1. evidence produced earlier in the same boundary (the proposal evidence, as
   in 0.15.0);
2. an ordinal reference that exists by the counter rule;
3. a domain-record version that is the committed current version of its
   record, or an initial-scenario version.

A retained version that is no longer current is refused with
`InvalidRandomOperationEvidence` in every run. 0.15.0 accepted it in an
unsealed run only; no first-party extension cites one (each cites an admitted
ingress). The current boundary itself is still not operation evidence. Exact
retries are answered from the ledger first, as in 0.15.0.

**Payload reads say so when the payload is gone.** `SimulationView::command`,
`event`, and `ingress` return `EvidenceContentUnavailable` for an archived ID
that is neither retained nor pinned; 0.15.0 returned `Ok(None)` when the seal
had dropped its receipt. `domain_record_version` returns
`EvidenceContentUnavailable` for a version whose existence is known (current
index, identity fact, receipt) but whose body is gone; it returns `Ok(None)`
only for a version that does not exist at the cut. `plugin_ingress_matches`
and `plugin_ingress_payload_matches` answer from an identity fact or a pinned
record, and return `EvidenceContentUnavailable` for an archived packet with
neither. A plugin reads `ingress` only for packets admitted at the current
boundary, which are retained or pinned, and the first-party callers of
`command` and most callers of `domain_record_version` already turn `None` into
an error, so for them only the error code changes. One caller treated `None`
as an answer: `canwu-history-research`'s assessment-subject lookup. It now
resolves the related assessment through `replay_stable_domain_record_version`,
the rule the plugin already applies to an assessment's subject, so a
contradiction or supersession names the related assessment's current version.
The plugin already refused a relation to any other version earlier in the same
check, so this changes no outcome; it makes the read itself seal-independent.
The seal-dependent part of the plugin's admission check was elsewhere: it
asks `evidence_exists` and `evidence_time` of a generic citation, which the
counter rule and the time index above now answer alike before and after a
seal (a durable test pins it).

The host reads `Canwu::domain_record_version` and
`CompactedCanwu::domain_record_version` keep returning `Option`: they cannot
express `EvidenceContentUnavailable`, so `None` covers both a version that
does not exist and one whose body a seal removed. Host-side validators built
on them (production observation witnesses, force-supply restore audits,
history-research analysis) therefore stay seal-dependent; that is a stated
non-goal.

### What each read returns after a seal or a continuation restore

| Read | Retained or pinned | Identity fact | Other ordinal at or below the cut | Other record version |
| --- | --- | --- | --- | --- |
| `evidence_exists` | true | true | true (0.15.0: only with a receipt) | false |
| `evidence_time` | its time | its time | its time (0.15.0: `None`) | `None` |
| `command`, `event`, `ingress` | the record | `EvidenceContentUnavailable` | `EvidenceContentUnavailable` | not applicable |
| `domain_record_version` | the body | `EvidenceContentUnavailable` | not applicable | `Ok(None)` |
| `replay_stable_domain_record_version` | unchanged: current, earlier in this boundary, or initial-scenario versions only | | | |
| fresh keyed draw | accepted (ordinal) | ordinal accepted; non-current version refused | accepted | refused |

The current version of every live record keeps its body and time, from the
record store and the provenance index, exactly as in 0.15.0.

### Invariants

- For ordinal references, existence and time are functions of the counters,
  the time index, and the cut. They are equal in a live run, a sealed run, a
  continuation-restored run, a run restored from segments, and exact replay.
- The fresh-draw predicate is a function of state, the cut, and the boundary's
  own proposals, never of retention, so a fresh draw is accepted or refused
  alike in every run.

## Part B: sealing with work in flight

### Problem

The seal requires an empty canonical ingress queue and a fully admitted
retained tail. A running game never has one, so it can never release
evidence, and its memory and every save grow with the campaign.

### The continuation closure

At a cut, the continuation closure `K` is:

- **K1, pending ingress.** Every record in the canonical ingress queue, with
  its *ingress provenance fact*: the packet ID and either its boundary
  generation (the issuer, the plugin whose boundary system scheduled it; the
  target plugin it is addressed to, which differs for a cross-plugin packet;
  the packet type; the producing boundary; and the status below) or "not
  generated by a boundary" (host-enqueued or command-caused ingress, or not
  plugin ingress at all). The generation is the producing boundary's
  `generated_ingress` entry with the packet's ingress ID, not the boundary
  record.
- **K2, causes of pending work.** For each pending ingress record and each
  scheduled action: a `CauseRef::Command` cause's command record, a
  `CauseRef::Event` cause's event record, `ArmyArrival` and `PersonArrival`
  order events, and `KnowledgeReport` dispatch events. A `CauseRef::Boundary`
  cause contributes only the issuer, which the K1 fact holds.
- **K3, payload-required dependencies.** Every `PayloadRequired` dependency an
  active continuation declares in a current record: ordinal records, the body
  of a non-current, non-initial record version, and for an ingress record its
  provenance fact. Each comes from a retained record, a pin, or a kept fact
  (see pinning at commit below).
- **K4, the unadmitted tail.** Events, commands, and attempts between the
  settled cut and the journal end.
- **F, identity facts.** For every `IdentityOnly` dependency (knowledge
  origins, `IdentityEvidenceDependenciesV1` declarations, archived request
  outcomes, scheduled-action causes) that is a non-current, non-initial record
  version, its membership; that is an ingress record, whatever made it, its
  provenance fact. Other identity-only ordinal references need nothing. Like
  K3, each comes from a retained record, a pin, or a kept fact.

Keyed draws are no longer in `K` or `F` (Part C), so neither grows with draws.

### Decision

A seal moves each journal up to its settled cut: events, commands, and
attempts up to the admission cursors, and ingress, boundaries, and draws to
the journal end. The unadmitted tail stays retained, so boundary admission,
which slices the retained tail from the admission cursors, is unchanged. The
segment is the same `EvidenceJournalSegment` shape; its per-journal cursors
simply need not end at one boundary. Ingress and boundaries share a cut, so a
boundary-generated packet and its producing boundary always land in the same
segment and its archive receipt keeps its provenance.

The runtime keeps the closure items that the seal moved below the archived
cut as **pinned evidence**: a sparse map per journal, a map of pinned record
version bodies, and the provenance and membership facts. Each seal recomputes
the pinned set from current state and keeps the pins and facts still in the
closure, so after a seal it holds exactly the closure below the archived cut.
An item that leaves the closure between seals (a packet admitted since) stays
pinned until the next seal, and a continuation checkpoint always derives the
closure afresh rather than copying the pins.

**Pinning at commit.** A version can stop being current after its
establishing boundary was sealed: its body then lives only in the live record,
and the boundary that replaces it does not hold it. A dependency on that
version, declared in the replacing boundary or earlier while it was current,
could then never be pinned at the next seal (in a sealed run and in a run
restored from a continuation checkpoint), while an unsealed run, which
retains the establishing boundary, could. So the boundary transaction, for
each record change it commits, keeps the version it replaces, retires, or
deletes when that version's establishing boundary lies at or below the
archived cut: its pre-commit body and its exact reference go into the pins
before the live record is replaced. An initial-scenario version needs nothing,
because its body stays readable. The cost is one body per changed record per
boundary, `O(changes in the boundary)`; a failed boundary drops them with the
rest of its writes. The next seal keeps such a pin only while a current record
declares it (`PayloadRequired` keeps the body; `IdentityOnly` declarations
and knowledge origins keep the membership) and drops it otherwise. Between
seals the pins hold at most one superseded version per changed record, the
same order as the retained evidence of those boundaries. Versions established
after the archived cut need nothing at commit: their boundary stays retained
until the next seal, which then pins what is declared.

**Ingress provenance facts carry the packet's status.**
`plugin_ingress_matches` and `plugin_ingress_payload_matches` must not match a
packet that is still queued or was cancelled. A retained packet is checked
against the queue and the retained cancellation index; that index is cleared
by a seal and starts empty after a continuation restore. Each fact for a
boundary-generated packet therefore records the packet's status (pending,
admitted, or cancelled), set when the fact is made and updated when the
packet is admitted or its cancellation is recorded, and the two reads answer
from it. The facts replace the archived receipt as the source of a sealed
packet's provenance: `plugin_ingress_matches` answers from the retained record
or the fact, and `plugin_ingress_payload_matches` from the retained record or
a pinned record with its fact. A packet no boundary generated never matches,
sealed or not, and its fact says so; a generated one matches the plugin that
both issued and receives it, as the retained check does, so a cross-plugin
packet matches neither plugin. A seal and a continuation checkpoint never
read receipts for a fact, so a continuation-restored run, which has none,
answers alike.

**Every pending or declared ingress record keeps a fact.** A fact is made
from the record while the runtime holds it, retained or pinned, and kept
while the record is pending or a current record declares it. So every pinned
ingress record has one, and a packet first declared after its seal, while
its record is still pinned (declared at admission, say), answers the reads
as it does unsealed. A record first declared after its seal and no longer
held has none: a read of it is `EvidenceContentUnavailable`, and the next
seal and continuation checkpoint are `ArchiveNotReady`, as for any evidence
declared after its seal (R4). The request tables declare the ingress of
every tracked command and decision request, so the facts grow with the
request index: about 17 bytes per tracked request (`{"ingress":N}`, the
generation omitted when there is none), beside its index entry of about 520
bytes.

Every read of retained evidence in `canwu-sim` is classified in this table,
and the implementation matches it:

| Read site | Reads | After a seal |
| --- | --- | --- |
| `settlement.rs`, boundary admission of due ingress | the record of every due packet | pinned fallback (K1) |
| `ingress.rs`, `plugin_ingress_issuer` | the producing boundary's generated ingress | provenance fact fallback (K1): the issuer |
| `ingress.rs`, `plugin_ingress_cancellation_target` | the pending packet's record | pinned fallback (K1); an archived packet that is not pending stays `LateIngress` |
| `view.rs`, `cancellable_plugin_ingress` | pending records and their producing boundaries | pinned and fact fallbacks (K1): the issuer |
| `view.rs`, `command`, `event`, `ingress` | admitted packets, their causes, unadmitted records | pinned fallback (K1 to K4), otherwise `EvidenceContentUnavailable` |
| `view.rs`, `plugin_ingress_matches`, `plugin_ingress_payload_matches` | an earlier admitted packet and its producer | pinned record or fact (K1, K3, F), with its status; a packet no boundary generated is `false` |
| `view.rs`, `plugin_ingress_payload_hash` | a packet the boundary admits | its match as above, then the retained or pinned record (K1) |
| `view.rs`, `ingress_is_declarable` | the ingress queue and the boundary's admitted packets | state only, so every run answers alike |
| `boundary_window.rs`, `summarize_boundary`, `queued_boundaries` | admitted and pending packets | pinned fallback (K1) |
| `persistence.rs`, archive reachability scan | pending packets' retention roots | pinned fallback (K1) |
| `mod.rs`, record-version body and time resolvers | non-current versions | pinned bodies, kept facts, time index |
| `validation.rs`, `resolve_runtime_domain_record_version` | version membership | current index, pinned bodies, membership facts; receipts are not read |
| `ingress.rs`, `decision.rs`, `maintenance.rs`, idempotent re-enqueue of command, decision, and maintenance requests | an identical queued request | archived request tables, then retained and pinned ingress |
| `decision.rs`, uniqueness of an admitted decision's nested command request | the carriers of the request | the decision itself counts whether retained or in the archived decision command requests |
| `settlement.rs`, current-boundary checks: cancellation in this boundary, and proposal evidence for events and ingress the current boundary caused | records appended by the boundary being settled | retained only; nothing is sealed while a boundary runs |

Preconditions after the change: the runtime is ready, no transaction is in
flight, and every declared dependency resolves (retained, pinned, current,
initial-scenario, or an existing identity fact); otherwise `ArchiveNotReady`
as in 0.15.0. The empty-queue and fully-admitted preconditions are removed.

Two-phase sealing keeps its contract. Its commit accepts a `PayloadRequired`
item that is pinned, or receipted and loadable from the provider, so the
one-phase and the two-phase seal agree: because those payloads are pinned in
memory, the one-phase `seal_evidence` no longer refuses a run with
`PayloadRequired` dependencies.

Facts the seal computes across journals no longer assume that one segment
holds both ends of a request. The `archived_unqueued_command_history` flag is
computed against the queued requests of the segment and of the archived
request tables, because a pending command ingress can be sealed in one
segment and admitted in a later one.

Per-boundary rollback must not copy growing structures: the pins and facts
are `im` maps, so the boundary transaction checkpoint copies them in constant
time, as it already copies the scheduler. The current-version index, the
archived request tables, the segment manifest, and the kept receipts are
`im` structures too, with large values behind `Arc` so that a tree node stays
small, so a fork, a seal's rollback copy, and the transaction checkpoints
share them instead of copying them (review of the first commit, item 5). They
are only iterated forward: `im` 15.1.0 ordered maps and sets break on reverse
traversal past about 2,100 entries. A seal rebuilds the kept receipts rather
than removing from them, because a removal holds several tree nodes on the
stack at once in a debug build, which overflowed a 1 MiB thread.

### What plugins may rely on after a seal or a continuation restore

- At admission, a packet's ingress record, its cause's command or event
  record, and the issuer of a boundary-generated packet.
- At the next boundary, every unadmitted event, command, and attempt.
- The payload of a `PayloadRequired` dependency, and the identity facts of an
  `IdentityOnly` dependency (for an ingress record, its provenance), while a
  current record declares it, provided it was declared while the runtime
  still held it.
- Existence and time of every ordinal ID at or below the cut, for ever.
- The current version (body and time) of every live record, and every
  initial-scenario version.
- The answer for every operation-keyed address ever drawn.
- The boundary ledger within the reader's declared horizon, `knowledge_heads`,
  person availability, and created persons by correlation, which already read
  state.

Nothing else. In particular the payload of any other evidence, the existence
of an undeclared non-current record version, scans of retained boundary
evidence, and the outbox entries of sealed boundaries (the host keeps those
with the segment) may differ from an unsealed run.

## Part C: the keyed-draw ledger (prerequisite)

### Problem

See the baseline. Every sealed keyed draw is held twice (a reservation with a
Merkle receipt, and an index entry), and every checkpoint, paged checkpoint,
seal, fork, and restore does work for every draw the run has made.

### Decision

The 0.15.0 keyed-draw index becomes the record of truth for every drawn
address, retained or sealed: the **keyed-draw ledger**, a persistent ordered
map (`im::OrdMap` of `Arc` entries) from `(stream, address)` to
`{draw ID, operation evidence, bound, purpose hash, value}`. A boundary commit
extends it, a failed boundary restores it, and a fork shares it in constant
time, as in 0.15.0.

- `KeyedDrawReservation`, `RuntimeEvidence::keyed_draw_reservations`, and the
  `keyed_draw_reservations` and `keyed_reservation_root` fields of
  `SimulationCheckpoint` are removed. A restore from segments rebuilds the
  ledger from the full draw journal; the checkpoint's copy was only a cross
  check of that rebuild.
- A keyed draw's operation evidence and draw ID are no longer evidence
  dependencies, so their receipts are no longer kept, and Part A answers
  their existence from the counters.
- `checkpoint()`, `prepare_paged_checkpoint`, and every seal do not touch the
  ledger. `fork()` shares it.
- A restore builds it once: snapshot validation returns the ledger it built
  instead of building it a second time, and the checkpoint-journal restore
  builds one runtime instead of loading the reconstructed snapshot twice. The
  same restructuring makes `snapshot_with_segments` and
  `replay_journal_with_segments` build one runtime. A test counts ledger
  builds per restore path (snapshot, checkpoint journal, paged checkpoint,
  sealed segments): one each.
- A continuation checkpoint carries every entry, in draw-ID order (Part E).

The ledger keeps one entry per drawn address for the life of the run, about
522 bytes in the 0.15.0 measurement. That is the price of exact idempotency
for ever; bounding it would change the meaning of a retry and is out of
scope. A sealed draw no longer adds a reservation and a receipt on top.

### Invariants

- The ledger equals the index built from every keyed draw the run has
  committed: after a commit, a failed boundary, a seal, any restore, exact
  replay, and in a fork.
- `checkpoint()`, `prepare_paged_checkpoint`, a seal, and `fork()` do work
  independent of the number of keyed draws.

## Part D: resumable journal commitments

### Problem

The command, attempt, event, ingress, and draw roots in `CommitmentRoots`
are BLAKE3 hashes of one byte stream per journal:
`domain || 0 || "[" || item1 || "," || item2 ... || "]"`. The runtime extends
them incrementally with a `blake3::Hasher`, whose state cannot be exported.
A run restored without the prefix could recompute neither the roots nor
their continuation, so its `checkpoint_hash` would differ at the first later
boundary.

### Decision

The runtime keeps each stream as a **resumable stream hash**: the stream
length, the stack of chaining values of its completed subtrees (at most 54),
and the bytes of its current, unfinished 1,024-byte chunk. Appending feeds
bytes; a chunk is finalized as a non-root chaining value only when more bytes
follow, and merged by the BLAKE3 tree rule; the root is computed on demand by
merging the stack with the final chunk, with the root flag on the last merge.
This is the reference incremental algorithm of the BLAKE3 paper, built on the
`blake3::hazmat` functions (`HasherExt::set_input_offset`,
`finalize_non_root`, `merge_subtrees_non_root`, `merge_subtrees_root`). An
append that carries whole chunks beyond the current one hashes the largest
power-of-two run of them that starts at a multiple of its own size in one
hazmat call, so BLAKE3 compresses several chunks at once as
`blake3::Hasher::update` does; a 3,000-byte item would otherwise hash about a
fifth slower. Each journal root is kept until the journal's next append,
because every committed transaction recomputes all five roots and most
transactions leave most journals unchanged. The workspace requirement moves
from `blake3 = "1"` to `blake3 = "1.8"`; no new dependency.

The state is held in fixed arrays (54 chaining values and one 1,024-byte
chunk), about 2.8 KB, with no heap allocation of its own. It replaces the
`blake3::Hasher` inside the ordered commitment accumulator; there is one
implementation, not two. The runtime holds its commitment cache, with the
five streams, behind one box, so that about 14 KB does not sit on the stack
of every frame that moves the runtime or a transaction checkpoint of it;
copying the cache for a rollback checkpoint is one allocation and a fixed
copy.

Its exported form, the **journal frontier**, is about 2.8 KB per journal at
most. A continuation checkpoint carries the five frontiers. Importing one
checks its shape before any hazmat call, because `set_input_offset` and
`finalize_non_root` assert on misuse: an empty stream has no tail and no
chaining value; a non-empty stream has a tail of 1 to 1,024 bytes that starts
at a chunk boundary and exactly `popcount(completed chunks)` chaining values.
A frontier longer than 2^62 bytes is refused, and every append checks the
new length with `checked_add`, so a hostile length near `u64::MAX` cannot
overflow it (it would panic in a debug build and hash wrongly in a release
build). A malformed frontier is `InvalidSnapshot`, never a panic. A restore then
requires each frontier's item count to equal the journal's ID count and its
root, recomputed before the runtime is built, to equal the root inside the
checkpoint's `CommitmentRoots`, which `checkpoint_hash` binds; a frontier that
finalizes to the committed root but differs would need a BLAKE3 collision,
and the chunk counters bind the stream length.

The boundary chain is already a head hash, so the checkpoint carries the head
and checks it against the `boundary_chain` root.

This package changes no root and no `checkpoint_hash`. Keeping old hashes is
not a requirement (no 0.16.0 run reads 0.15.0 data), so the alternative, a
commitment format bump with hash-chained journal roots, was
weighed on simplicity and safety alone. The frontier was kept: it is
implemented and checked against `blake3::hash` over every chunk and subtree
boundary and the upstream vectors, its exported state is small and bounded,
and it leaves the commitment format to the concurrent knowledge-ledger
change, which alters it for its own reasons, so the two packages do not both
redefine the same roots. A chained root would hash every item twice on the
hot path (the item, then the link) for no gain in what a restore can verify:
neither form gives membership proofs.

### Invariants

- For every stream and every split point, exporting, importing, and appending
  yields `blake3::hash` of the whole stream. A property test covers lengths
  around every chunk and subtree boundary up to 48 chunks with irregular split
  points, and the inputs and one digest of the upstream BLAKE3 test vectors.
  The command, event, ingress, and draw roots of a fixed run equal the
  values the 0.15.0 `blake3::Hasher` computes for it (a golden test). The
  format-8 checkpoint hash is not pinned: its authoritative state hash is
  now `boundary_state_hash_for_commitments` of the roots.
- A boundary appends the same bytes as before, with no extra heap allocation
  per item; the root costs `O(log n)` merges, as `Hasher::finalize` did.

## Part E: the continuation checkpoint (G-61)

### Contents

As shipped (`canwu_sim::ContinuationCheckpoint`, re-exported by `canwu-api`):

```rust
pub const CONTINUATION_CHECKPOINT_FORMAT_VERSION: u32 = 1;

pub struct ContinuationCheckpoint {
    pub format_version: u32,
    /// Current state, snapshot format 8, with every journal array empty, as
    /// in `SimulationCheckpoint::state`. Carries `checkpoint_hash` and the
    /// commitment roots.
    pub state: SimulationSnapshot,
    pub journal_end: EvidenceCursor,
    pub settled_cut: EvidenceCursor,
    /// Hash of the last boundary record; checked against `boundary_chain`.
    pub boundary_head: Option<String>,
    /// Resumable command, attempt, event, ingress, and draw streams (Part D),
    /// each with its item count, byte length, chaining values, and tail.
    pub journal_frontiers: JournalFrontiers,
    /// K4: the unadmitted events, commands, and attempts, in ID order.
    pub unadmitted: UnadmittedEvidence,
    /// K1 to K3 below the settled cut: per journal in ID order, and the
    /// bodies of declared non-current record versions by reference.
    pub pinned: PinnedEvidenceRecords,
    /// The IDs of the pending ingress among the pinned records.
    pub pending_ingress: Vec<IngressId>,
    /// The provenance fact of every pending or declared ingress record.
    pub ingress_provenance: Vec<PluginIngressProvenanceFact>,
    /// F: record-version memberships whose body is not pinned.
    pub known_record_versions: Vec<DomainRecordVersionRef>,
    /// The provenance index of every live record's current version.
    pub current_record_versions: Vec<CurrentRecordVersion>,
    /// Every operation-keyed draw ever made, in draw-ID order (Part C).
    pub keyed_draws: Vec<KeyedDrawEntry>,
    /// Exact idempotency for every tracked request at or below the settled
    /// cut: command, queued-command, decision, and decision-command tables.
    pub requests: RequestIndex,
    /// Whether the history holds legacy commands, tracked attempts, and
    /// unqueued command history.
    pub history_flags: HistoryFlags,
    /// Part A time index, per journal, up to the settled cut.
    pub evidence_times: EvidenceTimes,
    /// The boundary ledger: settled boundaries within the run's horizon.
    pub settled_boundaries: Vec<BoundarySummary>,
    /// Domain-separated hash over every field above (domain
    /// `canwu.continuation-checkpoint.v1`); `state` carries the checkpoint
    /// hash.
    pub continuation_hash: String,
}
```

It carries no segment header, no archived receipt, and no seal token: those
describe how a run was sealed, and the checkpoint must not depend on it.
Every list is sorted and unique; strict JSON decoding rejects unknown fields
and non-canonical encodings, as `deserialize_current_json` does elsewhere.

### Building it

`Canwu::continuation_checkpoint()` and `CompactedCanwu::continuation_checkpoint()`
(on `Simulation` and `CompactedSimulation` underneath) derive every part from
the runtime: closure items from retained or pinned evidence, identity facts
from retained records or kept facts, the request tables from archived tables
plus the retained attempts and ingress at or below the settled cut, the flags
likewise, frontiers from the commitment cache, and the ledger, time index,
provenance index, and boundary ledger as they are. An unsealed run builds the
request tables, the flags, and the time index with the same functions and hash
domains a seal uses, so its checkpoint is byte-identical to a sealed run's.
`continuation_checkpoint()` returns the checkpoint together with its
`continuation_hash`, which is what the host stores. Building changes nothing:
revision, `checkpoint_hash`, journals, and pins are untouched. It fails with
`ArchiveNotReady` exactly when a seal would, because a declared dependency
cannot be resolved (an R4 violation).

Cost: current state, plus the closure, plus the ledger, the request index, the
time index, and the boundary ledger. Nothing is proportional to boundaries,
events, or ingress settled before the cut.

### Restoring it

`CompactedCanwu::from_continuation_checkpoint(checkpoint)`,
`from_continuation_checkpoint_with_plugins(checkpoint, plugins)`, and
`from_continuation_checkpoint_verified(checkpoint, expected_hash, plugins)`,
with JSON forms, run these steps; any failure is `InvalidSnapshot` or
`UnsupportedSnapshotVersion` and no runtime is returned. The verified form
first requires the recomputed `continuation_hash` to equal the hash the host
stored, which is the trust anchor below.

1. **Versions.** Continuation format 1, snapshot format 8, the running
   engine version, the running commitment format (5), every state journal
   array empty.
2. **Integrity.** Recompute `continuation_hash` over the material and compare
   it with the carried value. This detects corruption, not tampering (see
   trust below).
3. **Shape and bounds.** Every list sorted and unique; every frontier's shape
   checked before any hazmat call (Part D), with its item count equal to the
   journal's ID count and its root recomputed and compared with the committed
   root before the runtime is built; `journal_end` equals the counters
   minus one; `settled_cut` equals the admission cursors and the journal end;
   the unadmitted records are exactly the IDs between the settled cut and the
   journal end; every pinned or referenced ID lies at or below the cut;
   every boundary-change version names a boundary at or below the cut; time
   index entries are strictly increasing in first ID, start at ID 1, name no
   ID above the cut, and have no two adjacent entries with the same time,
   which is the one representation Part A defines (it is exact for any order
   of times, so times are not required to increase), and every time is at or
   before `now`.
4. **State.** The state-only part of snapshot validation, shared with the
   snapshot loader: the run description (formats, manifest, initial
   scenario, configuration, entity registry against the created-person
   index rather than the boundaries' creations, canonical world order), scenario
   state, domain records against their schemas, knowledge structure,
   decision hot state, plugin components and the type schema, random stream
   states, and the shape of scheduled work with its transit consistency.
   Scheduled work that would enqueue plugin ingress is refused, because no
   valid run holds it (see the interaction with `SystemDirective::Schedule`
   below).
   Every evidence reference state holds is checked against the restored
   evidence in step 8, whose closure computation requires each declared
   dependency to exist (Part A rules plus pins and facts), so a knowledge
   origin, declared dependency, or cause of pending work that does not exist
   is refused. Checks that rebuild state from the boundary journal (the
   ledger replay of knowledge changes, decision-ingress reconstruction,
   transition manifest reconstruction, created-person and report-delay cross
   checks) do not run: they need history.
5. **Records.** Each pinned plugin packet names a declared packet type with
   its class, canonical existing affected entities, and archive retention
   roots that are well formed and bound to its payload; each pending
   ingress's queue key is derived from its pinned record. The identity check
   (step 7) and the round trip (step 8) bind the rest.
6. **Ledger.** Each keyed entry's address and bound are valid, its purpose
   hash is canonical, its address and its draw ID are unique and the ID is
   at or below the cut, and its value equals the value recomputed from the
   root seed, stream, address, bound, and purpose hash. A tampered value is
   refused without any history.
7. **Identity.** Build the runtime (archived cut = settled cut; retained tail
   = the unadmitted records; pins, facts, ledger, time index, provenance
   index, boundary ledger, request tables, flags, and boundary head from the
   checkpoint; commitment cache from the frontiers), recompute every state
   root and the checkpoint hash, and require the result to equal
   `state.checkpoint_hash` and `state.commitment_roots`. This verifies all
   current state, the journal roots, the boundary chain head, the control
   counters, and the revision.
8. **Closure round trip.** First the boundary ledger must have the form a
   run keeps: consecutive boundary IDs ending at the last boundary, the first
   no earlier than the run's boundary-schedule horizon before the last.
   Whether a boundary inside the horizon was dropped from its front cannot be
   checked without history. Then derive the continuation checkpoint from the
   built runtime and require it to equal the input. This proves that the
   carried closure is exactly the closure of the restored state (nothing
   missing, nothing extra) and that every derived part is in canonical form.
   A closure that cannot be derived, because a cause of pending work or a
   declared dependency lies beyond the journal end, is refused here.
9. **Plugins.** Rehydrate the exact plugin descriptors (`_with_plugins`),
   activate them, and enter the compact runtime. Without plugins, a
   checkpoint that names one fails with `PluginNotActive`, as the compact
   paged restore does.

Restore yields a `CompactedCanwu`, not a `Canwu`. `Canwu` promises full
history through infallible methods (`snapshot`, `replay_journal`, `events`,
`boundaries`); a continuation runtime cannot honour them. `CompactedCanwu`
already means "a live run whose earlier evidence is owned elsewhere", and Part
F gives it the host operations a game needs.

### Identity and trust

- `checkpoint_hash` and `authoritative_state_hash` remain the identity of the
  state. Step 7 proves that the restored run has the hashes the uninterrupted
  run had at that cut, and both runs reach the same hashes at every later
  boundary. For that, the first commit redefines `authoritative_state_hash` as
  the commitment-format state hash (`v1:` over the canonical commitment
  roots, the form boundary records already use). 0.15.0 hashed the retained
  journals, so a sealed run, and any restore that does not retain the whole
  history, reported a different value for the same state.
- `continuation_hash` is the identity of the whole checkpoint. Because every
  part is a function of state and history at the cut, it is equal for the
  unsealed run, any sealed run, and any continuation-restored run at the same
  cut.
- What restore verifies without history: all current state and its
  commitments, the journal roots and boundary head, the counters and cut, the
  closure's completeness relative to the restored state, the shape of every
  record, every keyed draw's value, and that every cited ordinal ID lies
  within the run.
- What it cannot verify without history: that the carried evidence records
  are the ones the journal roots commit to (those roots are flat stream
  hashes, without membership proofs), including the payloads of the pinned
  records; which packets are pending (step 5 binds each pending ID's queue
  key to its pinned record, not the set of pending IDs) and the content of
  their records, causes included, beyond each cause lying within the journal
  (step 8); a keyed entry's draw ID and operation evidence; request outcomes;
  which change index established a current or known record version; the
  boundary summaries beyond the ledger's form (step 8); the time index; the
  provenance facts and their statuses; and the settled cut. The admission
  cursors that define the settled cut (`admitted_attempt_count`,
  `admitted_command_count`, `admitted_event_count`) are in no commitment
  (`ControlCommitmentMaterial` holds only the next IDs), so which records
  count as unadmitted is bound only by `continuation_hash`.
- What a host must trust: the `continuation_hash` it recorded when it wrote
  the checkpoint (or the medium that stored it). Restoring with
  `from_continuation_checkpoint_verified` against that stored value covers
  everything above; comparing only `checkpoint_hash` covers the state but not
  the carried evidence, the indexes, or the settled cut. A file's own
  `continuation_hash` field only detects corruption. The derived indexes are
  not folded into `checkpoint_hash`.
- A continuation checkpoint holds ground truth. Like a snapshot, it must not
  be given to player, agent, observer, or remote clients.

### What a restored run can and cannot do

It can settle boundaries, admit and cancel ingress, process commands and
decisions, seal, fork, write further continuation checkpoints, and answer
every read in Part B, with the same results as the uninterrupted run.

It cannot produce a full `SimulationSnapshot`, a `ReplayJournal`, a
`CheckpointJournal`, a `SimulationCheckpoint`, or a paged checkpoint:
`checkpoint`, `prepare_paged_checkpoint`, `snapshot_with_segments`, and
`replay_journal_with_segments` return the new `ErrorCode::HistoryUnavailable`.
It has no archived receipts (`archived_evidence_receipt` returns `None`, and
`load_archived_evidence_segment` returns `EvidenceUnavailable` for evidence
before its base), no outbox entries for boundaries before its base, and
retained-evidence reads (`boundaries`, `events`, `knowledge_changes_by_correlation`,
`transition_audits`, `CanwuViewer::visible_changes_since`,
`CanwuViewer::evaluation_traces`, `explain`) see only evidence after its base.
`CompactedCanwu::continuation_base()` returns the base cut and hashes, or
`None` for a run with full history.

The segments it seals continue the original run's cursor sequence. A host
that kept every segment since genesis therefore still holds a complete
journal; rebuilding a replay journal from a continuation base and those
segments is follow-up work, not part of this package.

The two-phase seal token binds a compact checkpoint identity computed from the
runtime's fields (state checkpoint hash, cuts, segment-manifest, receipt, and
dependency roots), so it also works on a continuation runtime, which cannot
build a restorable `SimulationCheckpoint`. Its commit accepts a pinned
`PayloadRequired` item in place of a receipted, loadable one (Part B), which
is what a continuation runtime holds.

## Part F: one host contract for `Canwu` and `CompactedCanwu` (prerequisite)

### Problem

A compact runtime cannot drive extension helpers through the public API:
`enqueue_resource_completion_operation` and `enqueue_resource_archive` take
`&mut Canwu`, and so do the host helpers of `canwu-fiscal`, `canwu-law`,
`canwu-military`, `canwu-production`, `canwu-economy-reference`,
`canwu-force-supply-reference`, and `canwu-ming-fiscal-reference`. Neither can
a compact runtime attach a plugin archive provider, open a viewer, fork, or
read exact record versions, so an archive-backed run cannot be compact, and a
run restored from a continuation checkpoint could not be played.

### Decision

**Parity.** `CompactedCanwu` gains every `Canwu` method whose meaning does not
need full history: `set_plugin_archive_object_provider`,
`plugin_archive_object`, `fork`, `viewer`, `viewer_for_actor`,
`viewer_context`, `admin_query_knowledge`, `explain`,
`authoritative_state_hash`, `run_manifest_hash`, `plugin_descriptors`,
`evidence_exists`, `evidence_time`, `domain_record_version`,
`domain_record_version_evidence_exists`, `current_domain_record_version`,
`domain_records`, `domain_record_page`, and the retained-tail `boundaries`,
`events`, `commands`, `command_attempts`, `ingress_log`, `random_draws`,
documented as such, and `PluginArchiveObjectProvider`. `CompactedCanwu::fork`
keeps the committed seal tokens. `CanwuViewer` borrows either handle through a
private read interface; its holder queries read the latest committed boundary
from the boundary counter, so they do not depend on whether that boundary is
retained, and `explain` finds events in a retained tail that starts after a
seal. Methods that promise full history (`snapshot`, `snapshot_json`,
`replay_journal`, `checkpoint_journal*`) stay on `Canwu`. The provider attach
mirrors the `Canwu` signature, which since the [send runtime](send-runtime.md)
package takes `Arc<dyn PluginArchiveObjectProvider + Send + Sync>` on both
handles.

**Trait.** `canwu-api` adds `pub trait CanwuRun`, implemented by `Canwu` and
`CompactedCanwu`. Its method set is what the inventoried helpers use, plus the
boundary ledger: `time`, `revision`, `checkpoint_hash`, `entity_exists`,
`domain_record`, `domain_records` (a boxed iterator, so the trait stays
dyn-compatible),
`current_domain_record_version`, `domain_record_version_evidence_exists`,
`boundaries_between`, `boundary_schedule_horizon`, `enqueue_plugin_ingress`,
and `enqueue_permitted_plugin_ingress`. Its rustdoc says it is a
read-and-ingress contract that settles nothing; it requires neither `Send`
nor `Sync`. Typed record reads are a free function,
`canwu_api::typed_domain_record(run, reference)`.

**Helpers.** Every public extension function whose first parameter was
`&Canwu` or `&mut Canwu` and that uses only those reads and canonical ingress
now takes `&H` or `&mut H` with `H: CanwuRun + ?Sized`; callers that pass a
`Canwu` compile unchanged. The inventory:

- `canwu-resource`: `enqueue_resource_adapter_operation`,
  `enqueue_resource_allocation`, `enqueue_resource_completion_operation`,
  `enqueue_resource_archive`, `finalize_resource_archive_retention`,
  `resource_state` and every resource query built on it (allocation legs,
  consumption, outcomes, completion certificates, grants and statuses,
  fulfillments, reports, observation witnesses), `validate_resource_runtime`,
  and `validate_resource_runtime_with_archive_store`;
- `canwu-fiscal`: `enqueue_execution_receipt`;
- `canwu-law`: `enqueue_legal_mutation`, `enqueue_legal_mutation_at`,
  `enqueue_legal_archive`, `finalize_legal_archive_retention`,
  `load_legal_runtime`, and `load_law_state_for_plan`;
- `canwu-military`: `enqueue_provider_outcome`;
- `canwu-production`: `enqueue_production_archive`,
  `enqueue_production_completion_operation`,
  `enqueue_production_resource_continuation`,
  `finalize_production_archive_retention`, `production_report`,
  `production_observation_witness`, and `degraded_facility_decision_ticket`;
- `canwu-movement`: `movement_state`;
- `canwu-economy-reference`: `enqueue_economy_archive`,
  `finalize_economy_archive_retention`, `economy_reference_state`,
  `validate_economy_reference_runtime_with_archive_store`, and
  `economy_observation_witness` (which also needs the handle as a
  `PluginArchiveObjectProvider`, so it takes a sized `H`);
- `canwu-force-supply-reference`: `enqueue_force_archive`,
  `finalize_force_archive_retention`, and `force_supply_report` (sized `H`, as
  above);
- `canwu-ming-fiscal-reference`: `enqueue_reference_execution_result` and
  `reference_execution_evidence_version`, which now reads the current-version
  index instead of scanning retained boundaries, so it answers alike before and
  after a seal.

Functions that read history or exact version bodies keep `Canwu`: the restore
and replay wrappers; `validate_technology_runtime` and the technology loader
(they replay provenance over every boundary); `validate_production_runtime`,
`validate_production_runtime_with_archives`, and
`validate_production_observation_witness` (they read the draw journal and
superseded bodies); `load_society_state`, `validate_society_runtime`, and the
society projections (they read the full snapshot and the stock history);
`validate_historical_research_runtime` (it reads cited bodies and generic
evidence); the force-supply restore audits and observation-witness checks; the
Ming reference sample cycle, trace, and validator; and the reference world's
`snapshot`. They are seal-dependent host checks, a stated non-goal.

Authority is unchanged: helpers reach state only through canonical ingress,
checked at admission and at settlement, with the same permits.

## Proof obligation

**Claim.** Let a run R reach cut `c` after boundary `k`, let
`C = continuation_checkpoint(R)`, and let R' be restored from `C`. For any
later sequence of host inputs applied identically to R and R', and for plugins
that obey R1 to R5, every later boundary produces equal `BoundaryReceipt`s,
equal `checkpoint_hash` and `authoritative_state_hash`, and equal
continuation checkpoints, whether or not R ever sealed.

**Argument.**

1. *Equal start.* Step 7 makes R' state and every committed root equal to R's
   at `c`; step 2 and the host's stored hash make every derived part equal.
   The derived parts are equal across R's possible seal histories by
   construction: each is defined from state and the full history, and Parts B
   and C remove every seal-history input (receipts, segment headers,
   reservations) from them.
2. *Equal reads.* Settlement is a deterministic function of state, derived
   indexes, the evidence it reads, and the inputs. Engine reads fall in two
   classes, each equal in R and R': seal-stable reads (counters, time index,
   current-version index, ledger, request tables, boundary ledger, knowledge
   heads, state) and closure reads (pending ingress, their causes and issuers,
   the unadmitted tail, scheduled-action causes, pinned payloads, identity
   facts). The implementation lists every retained-evidence read site in
   `canwu-sim` (the 0.15.0 set is in `boundary_window.rs`, `ingress.rs`,
   `mod.rs`, `settlement.rs`, `validation.rs`, and `view.rs`) and maps each to
   one class. Plugin reads fall in the same two classes under R1 to R5.
3. *Equal writes.* Equal reads give equal proposals, equal ID allocation from
   equal counters, and equal appended bytes, so the resumable streams extend
   to equal roots and the boundary chain to equal heads.
4. *Induction.* After the boundary, R' derived state equals R's. The closure
   at the next cut contains only evidence produced since `c` (retained in R')
   or evidence already in `K` or `F` at `c` (R4), so R' can again derive it,
   and step 1 holds at the next cut.

The tests below check the claim directly rather than the argument.

## Interactions

- **Boundary ledger.** Carried as summaries within the run's horizon; a
  restore with exact plugin descriptors has the same horizon, and the
  restored runtime extends and prunes it as before. The queued part reads
  pending ingress, which is pinned. Bounded by the horizon, at most 366 days.
- **Keyed-draw index.** Becomes the ledger (Part C).
- **Knowledge heads.** Read committed state; nothing to carry.
- **The concurrent knowledge-ledger change.** Another 0.16.0 package changes
  the knowledge ledger's representation and the boundary hash, with a
  commitment change for knowledge-change evidence. The two compose under
  these conditions, which both packages keep:
  1. The checkpoint's `state` carries the knowledge ledger in whatever form
     `SimulationSnapshot` holds; restore step 4 calls that package's
     state-level knowledge validation. A check that rebuilds the ledger from
     boundary knowledge changes is history validation and does not run on a
     continuation restore; it needs a state-only counterpart if it guards
     something state alone can show.
  2. A derived knowledge index rebuilt from the journal on restore must be
     rebuildable from state, or be carried by the continuation checkpoint
     (a new field, continuation format 2 if added after release).
  3. A new append-only commitment fed by knowledge-change evidence must be
     resumable: a chain head, or a stream kept with Part D's resumable hash
     and carried as a sixth frontier.
  4. The boundary hash only changes the head string this design carries.
  5. This design changes no commitment root of its own. That package moved
     the commitment format to 5; restore step 7 recomputes with whatever
     format the engine writes, and the knowledge records accumulator, which
     that package added to the commitment cache, is rebuilt from state on
     the first refresh after a continuation restore, like every state-domain
     root.
  The knowledge ledger's own growth (superseded records it keeps) is state,
  carried as state; that package decides that term.
- **Resource and production archives.** Their content-addressed archives live
  in host stores; restore audits that authenticate archive chains
  (`validate_resource_runtime_with_archive_store`,
  `validate_production_runtime_with_archives`) read state and the stores, and
  run on a compact runtime through `CanwuRun`. Pending archive commits and
  retention acknowledgements are pending ingress, so they are pinned, and the
  reachability manifest scans pinned pending ingress for retention roots.
- **`SystemDirective::Schedule` of plugin ingress.** In 0.15.0 a command or
  event handler could schedule a directive that enqueues plugin ingress. The
  packet it queues when it fires carries the command or event as its cause
  but is issued later than that cause was accepted, and the full-save loader,
  which requires command-caused ingress to be issued when its command was
  accepted, refused the run. The loader cannot be relaxed exactly: the fired
  action leaves no evidence of when it was issued, so a load could not tell
  such a packet from a forged one. `validate_directives` therefore refuses
  with `InvalidPayload` a `SystemDirective::Schedule` that wraps
  `EnqueuePluginIngress` at any depth, and the snapshot and continuation
  loaders refuse scheduled work holding one. `EnqueuePluginIngress` takes its
  own delay, which gives the same due time. No first-party extension
  schedules one.
- **Decision archive.** Hot decision state is in `state`; archived decision
  blobs stay in the host's `DecisionArchiveProvider`, as with a flat
  checkpoint.
- **Outbox.** Entries of boundaries before the base are the host's, with the
  segments it chose to keep; a continuation checkpoint does not carry them.
- **`Send`.** No `Rc` or `RefCell` is added. New state is ordered maps, `im`
  structures of `Arc` entries, and plain data. The one new public use of the
  provider type is the mirrored attach on `CompactedCanwu`, which takes
  `Arc<dyn PluginArchiveObjectProvider + Send + Sync>` since the
  [send runtime](send-runtime.md) package made both handles `Send`.

## Formats and compatibility

| Format | 0.15.0 | 0.16.0 | Why |
| --- | --- | --- | --- |
| Snapshot | 8 | 8 | unchanged shape |
| Commitment | 4 | 5 | moved by the knowledge-ledger package; this package changes no root |
| Checkpoint/evidence journal | 4 | 5 | `SimulationCheckpoint` loses `keyed_draw_reservations` and `keyed_reservation_root`; its dependency and receipt lists no longer name keyed draws; segments may end at different per-journal cuts |
| Compact checkpoint hash domain | `canwu.compact-checkpoint.v1` | `.v2` | material loses the reservation root |
| Seal-token hash domain | `canwu.evidence.seal-token.v1` | `.v2` | bumped with the compact checkpoint identity it binds |
| Paged-checkpoint envelope | 4 | 5 | embeds `SimulationCheckpoint`; the record-store package moves it to 5 as well (state-page codec `raw-canonical-v2`), and 0.16.0 has one format 5 carrying both changes |
| Continuation checkpoint | none | 1 | new; own hash domain `canwu.continuation-checkpoint.v1` |

Snapshots, boundary records, receipts, and replay journals keep their shapes
and hashes. Segments keep their shape, item encoding, and segment IDs (the ID
material has no format field); their `format_version` field reads 5. A 0.15.0 save does not load
in 0.16.0 regardless: snapshot loading requires the running engine version.
The format8 contract tests change their expected checkpoint-journal and
paged-envelope versions and compact-checkpoint hash only; new tamper tests
cover each continuation part.

The first commit changes the checkpoint/evidence-journal format, the paged
envelope, and the two hash domains; the second adds the continuation format.
The second commit computes the two-phase seal token's compact identity from
the runtime's fields instead of a built `SimulationCheckpoint`; the material
and its value are those of 0.16.0's compact checkpoint hash, so the domain
does not move again.

Public API: `KeyedDrawReservation` and the two `SimulationCheckpoint` fields
are removed (breaking; `canwu-api` never re-exported the type).
`authoritative_state_hash` returns the commitment-format state hash (a value
change, not a type change). The second
commit marks `ErrorCode` `#[non_exhaustive]` and adds `HistoryUnavailable`
(breaking for exhaustive matches, once, in this pre-1.0 minor). Behaviour
changes: Part A's read results and fresh-draw rule, `canwu-history-research`'s
relation check, and the relaxed seal preconditions. `CanwuRun`,
`typed_domain_record`, and the `CompactedCanwu` and `CompactedSimulation`
parity methods are additive; helper signatures stay source-compatible for
callers that pass a `Canwu`.

## Size, cost, and measurement

### What grows with campaign length

| Term | In a continuation checkpoint | In a live compact runtime |
| --- | --- | --- |
| Current state (records, knowledge ledger, decisions, components) | yes, as state | yes |
| Keyed-draw ledger | one entry per address ever drawn | one entry per address |
| Request index | one entry per tracked request ever (since 0.18.0: one window of outcomes and receipts, plus one canonical range per run of consecutive retired IDs; see [request outcome retention](request-outcome-retention.md)) | same |
| Ingress provenance facts | one per pending or declared ingress record; the request index declares the ingress of every tracked request (since 0.18.0: of every request inside the retention window) | same |
| Time index | one entry per distinct instant per journal | same |
| Segment headers | not carried | one per seal |
| Boundaries, events, commands, ingress, draws, receipts | only the closure | closure plus tail since the last seal |
| Boundary ledger | bounded by the horizon | same |
| Frontiers | at most about 14 KB | same |

### Targets

The first commit is measured on `keyed-draw-profile` and the architecture
suite:

1. `checkpoint()`, `prepare_paged_checkpoint`, a seal, and a fork of a sealed
   compact run allocate the same from 1,000 to 100,000 sealed keyed draws
   (within 2 %), where 0.15.0 grows with every draw; a fork of a live run
   copies only its retained journals.
2. A snapshot restore and a checkpoint-journal restore each build the ledger
   once (a unit test counts builds); their allocations per keyed draw fall by
   the duplicate builds.
3. Settlement is unchanged: on the architecture suite, boundary allocations
   within 1 % and elapsed time within noise; on `keyed-draw-profile`, the
   boundary figures within noise.

The second commit is measured on the new `continuation-profile` workload. The
target is a slope, not a ratio at one month:

4. The continuation checkpoint contains no evidence record outside `K`; a
   test checks this independently of size.
5. At every sampled month, the report records the checkpoint's non-state
   bytes by part (closure, pins, ledger, request index, time index, boundary
   ledger, frontiers) and its restore time. With no keyed draws the non-state
   series is flat (within 10 % of month 1) apart from the pins of work in
   flight; with 10 keyed draws a day its month-over-month slope equals the
   ledger term plus the request-index term within 10 %. Pin bytes are
   reported separately.
6. Writing and loading the continuation checkpoint at month 14 take at most
   10 % of the time of the full save (`checkpoint_journal_json` and its
   restore).

For the downstream figure (138 to 157.5 MB at month 14, nearly all settled
evidence), the expectation is a save of current state plus the ledger and the
request index, with save and load time following those terms and not the
campaign's length. The workload is calibrated to the same evidence rate,
about 10 MB of full-save growth per simulated month, but only the downstream
run can confirm its own number; the report will ask for it.

### Measurement

- **`keyed-draw-profile`** gains `compact_checkpoint`,
  `compact_paged_checkpoint`, `compact_seal`, `compact_fork`, and
  `checkpoint_journal_restore` cases, run on an export of `v0.15.0` and on the
  branch (the compact fork case is compiled only where `CompactedCanwu::fork`
  exists).
- **The architecture suite**, before and after, for the resumable hash, in
  allocations and in elapsed time.
- **`continuation-profile`** (second commit), a new harness binary using only
  `canwu-api`. One daily boundary; a fixture plugin whose commands queue
  packets due 1 to 90 days later and a follow-up within 9 days, whose packets
  update a record, make `k` keyed draws each, and schedule follow-ups up to 30
  days ahead, which publishes superseding knowledge and holds a
  payload-required continuation, and whose churn system moves 140 records to
  their next version a day (so that the full save grows about 10 MB a
  simulated month, the downstream rate, while current state does not); the
  host dispatches 8 tracked commands a day. At months 1, 2, 4, 7, 10, and 14
  and for `k` in {0, 10}, it records the
  full portable save's bytes, write time, and restore time; the continuation
  checkpoint's non-state bytes by part, write time, restore time, and restore
  allocations; and, in the allocation build, the heap of a compact run that
  seals monthly against one that never seals.
- Reports go to `benchmarks/baselines/2026-10-*-keyed-draw-ledger-*.json` and
  `...-continuation-*.json`, release builds, with machine and load noted as in
  0.15.0; allocation counts are the figures to rely on.

### First-commit results

Allocation counts from release builds on one machine (Windows x86-64, 16
logical CPUs, Rust 1.99.0), `keyed-draw-profile` at 8 due systems, 0.15.0
built from an export of `v0.15.0`; files under `benchmarks/baselines` named
`2026-10-15-keyed-draw-ledger-{before,after}-allocations.json`. Allocation
counts are exact and do not depend on machine load. Each cell is 0.15.0, then
this commit.

| Case (allocations) | 1,000 keyed draws | 10,000 | 100,000 |
| --- | ---: | ---: | ---: |
| `checkpoint()` of a sealed compact run | 33,554 / 185 | 413,244 / 185 | 4,730,434 / 185 |
| paged checkpoint of a sealed compact run | 33,582 / 248 | refused / 248 | refused / 248 |
| seal of one more boundary | 35,289 / 1,109 | 418,137 / 1,109 | 4,767,469 / 1,112 |
| fork of a sealed compact run | not available / 291 | not available / 291 | not available / 291 |
| checkpoint-journal restore | 92,615 / 35,819 | 893,061 / 335,343 | 8,897,434 / 3,330,550 |
| snapshot JSON restore | 300,084 / 289,028 | 2,951,217 / 2,840,624 | 29,372,331 / 28,266,363 |
| live or sealed boundary | 999 / 1,000 | 1,007 / 1,008 | 1,015 / 1,016 |

- Target 1 is met: the checkpoint, the paged checkpoint, the seal, and the
  fork of a sealed run do not grow with keyed draws (the seal moves by 3
  allocations, 0.3 %, over a hundredfold increase). 0.15.0 grew by about 47
  allocations per keyed draw in each, and refused a paged checkpoint from
  10,000 sealed keyed draws on, because the envelope that listed every keyed
  reservation no longer fit one 4 MiB state page.
- Target 2 is met: the snapshot restore makes 11 fewer allocations per keyed
  draw (one ledger build instead of two), and the checkpoint-journal restore
  about 56 fewer (one runtime and one ledger instead of two runtimes and four
  ledger builds). A unit test counts one ledger build per restore path.
- Target 3 is met: on the architecture suite (four scales, files
  `2026-10-15-architecture-ledger-{before,after}-allocations.json`) every
  command, boundary, snapshot, checkpoint, and replay case allocates as in
  0.15.0 plus one allocation per transaction checkpoint and two per restore:
  the commitment cache is boxed, so that the five resumable streams (about
  14 KB) do not sit on the stack of every frame that moves the runtime or a
  checkpoint of it (a debug build overflowed a 1 MiB thread without it). A
  command or a boundary makes one more allocation; history and replay cases
  one more per transaction they run. The live-archive seal cases allocate
  12 % to 16 % less, because keyed draws are no longer dependencies whose
  receipts a seal keeps.
- The resumable hash costs no measurable time. Elapsed time was compared on
  this tree with the 0.15.0 `blake3::Hasher` accumulator and with the
  resumable stream hash, the two release builds run alternately five times
  each on a machine shared with other builds (files
  `2026-10-15-journal-stream-hash-{before,after}-elapsed.json`, samples of the
  five runs merged). The sum of the 68 case medians is 0.9 % lower with the
  resumable hash and the median case 1.3 % lower; empty boundaries are 2 % to
  12 % faster and populated boundaries 1 % to 3 % faster, because a journal
  root is now kept until the journal's next append; commands, replay, and
  history growth move by a few percent either way, within run-to-run noise.
  `snapshot_load_validate` reads 1 % to 23 % slower across scales, but
  hashing is under 3 % of that case's work, so the difference is noise or
  code layout, not hashing. Alone, outside the runtime, feeding a journal
  item by item costs 0.2 % to 0.7 % more than `blake3::Hasher` for 120-byte
  items and 3 % to 4 % more for 400-byte and 3,000-byte items (22 % more for
  3,000-byte items before whole aligned subtrees were hashed in one call).
- Live memory per keyed draw is unchanged in the unsealed run
  (`run_heap_bytes` equal). In a sealed run each sealed keyed draw no longer
  keeps a reservation with its Merkle receipt besides its ledger entry.

### Second-commit results

`continuation-profile` release builds on the same machine, shared with other
builds (files `2026-10-15-continuation-{elapsed,allocations}.json`; elapsed
medians of three samples, allocation counts of one). The columns are the
samples at months 4, 7, and 14 of 30 days; the reports also hold months 1, 2,
and 10.

| `k` = 0 (no keyed draws) | Month 4 | Month 7 | Month 14 |
| --- | ---: | ---: | ---: |
| Full save (`checkpoint_journal_json`) | 42.6 MB | 74.6 MB | 149.4 MB |
| Continuation checkpoint | 0.72 MB | 1.15 MB | 2.17 MB |
| of which state | 101 KB | 138 KB | 225 KB |
| of which request index | 504 KB | 889 KB | 1,787 KB |
| of which time index | 15 KB | 27 KB | 55 KB |
| of which pins | 26 KB | 25 KB | 25 KB |
| Write, continuation against full | 6.7 / 156 ms | 10.4 / 290 ms | 21.5 / 599 ms |
| Restore, continuation against full | 54 / 1,984 ms | 59 / 3,835 ms | 98 / 7,809 ms |

| `k` = 10 keyed draws per packet | Month 4 | Month 7 | Month 14 |
| --- | ---: | ---: | ---: |
| Full save | 44.9 MB | 79.2 MB | 159.1 MB |
| Continuation checkpoint | 1.55 MB | 2.80 MB | 5.72 MB |
| of which keyed-draw ledger | 829 KB | 1,646 KB | 3,544 KB |
| of which request index | 504 KB | 889 KB | 1,787 KB |
| Write, continuation against full | 12.3 / 164 ms | 16.1 / 299 ms | 38.8 / 436 ms |
| Restore, continuation against full | 82 / 2,100 ms | 125 / 3,038 ms | 207 / 6,244 ms |

- Target 4 is met by construction and by test: a restore requires the
  checkpoint derived from the restored runtime to equal the input, so an
  extra or a missing closure item is refused (the tamper test checks both),
  and the lockstep test compares the checkpoints of the unsealed, sealed, and
  restored runs byte for byte.
- Target 5. The non-state bytes grow only by the terms the table above names
  as growing with campaign length. Without keyed draws, the rest (non-state
  bytes less the request index and the time index) is 84 KB at month 1 and
  98 KB at month 14; less the pins of work in flight, which fill up over the
  first three months as the 90-day backlog builds, it is 69 KB and 73 KB
  (within 6 %). The target as written (flat within 10 % with no keyed draws)
  assumed no tracked requests: every tracked command adds a request-index
  entry, here about 520 bytes for its exact outcome and its queued-ingress
  receipt together, 1.8 MB over 14 months at 8 commands a day. With 10
  keyed draws per packet, the non-state slope from month 10 to month 14 is
  1,613 KB against 1,080 KB of ledger and 513 KB of request index, 1.2 %
  apart; the time index (19 KB over those months) is the rest.
- Target 6 is met: at month 14 writing the continuation checkpoint takes
  3.6 % (`k` = 0) and 8.9 % (`k` = 10) of the time to write the full save,
  and restoring it 1.3 % and 3.3 % of the time to restore the full save. The
  full save is 149 to 159 MB, the downstream figure; the continuation
  checkpoint is 2.2 and 5.7 MB.
- Heap, allocation build: a run that seals every month holds 18.5 MB at month
  14 without keyed draws and 23.1 MB with 10 per packet, against 199 MB and
  209 MB for the run that never seals. A continuation restore at month 14
  leaves 3.1 MB and 7.2 MB allocated for the restored run, in 0.72 million
  and 1.98 million allocator calls.
- These figures predate the second review's fixes, which add one provenance
  fact per pending packet and per tracked request: 16 KB at month 4 and
  57 KB at month 14 (2.6 % of the checkpoint at `k` = 0), measured on the
  same workload, and change no other part. The fact term grows with the
  request index (target 5).
- The profile queues its follow-ups directly with their own delay: a handler
  can no longer schedule a directive that enqueues plugin ingress (see
  Interactions).

## What can still fail

Boundaries (a failed boundary stops the run):

- A fresh keyed draw citing a non-current, non-initial record version or the
  current boundary: `InvalidRandomOperationEvidence`, now in every run. A
  handler that propagates it fails the boundary; one that records it as a
  rejection does not. No first-party extension cites such evidence.
- A `canwu-history-research` relation that names a related assessment's
  non-current version is refused at admission as a recorded command rejection
  in every run, as its subject already was.
- A payload read outside the closure: `EvidenceContentUnavailable`, which a
  handler that propagates errors turns into a failed boundary. This is an R1
  or R2 violation, and it fails in the sealed or restored run while the
  unsealed run continues.
- Admission of a pending ingress whose record is missing, or a
  boundary-generated packet without its provenance fact: `InvalidSnapshot`.
  An engine invariant; a correct closure makes it impossible.
- A provenance read of an ingress record first declared after its seal and
  no longer held: `EvidenceContentUnavailable`, an R4 violation, in the
  sealed or restored run only.

Not boundaries:

- A seal, a continuation checkpoint, or the two-phase commit fails with
  `ArchiveNotReady` when a declared dependency no longer resolves (an R4
  violation), including an ingress record first declared after its seal and
  no longer held, whose provenance fact can no longer be made. The run
  continues but cannot seal or write a continuation save until the
  declaration is dropped.
- A command or event handler whose directives schedule a directive that
  enqueues plugin ingress: `InvalidPayload`, refused before it changes
  anything (see Interactions).
- A continuation restore refuses a corrupted, inconsistent, or tampered
  checkpoint that steps 1 to 8 detect; tampering that also recomputes
  `continuation_hash` is caught only against the host's stored hash.
- `HistoryUnavailable` for full-history operations on a continuation runtime.

## Ownership and affected surfaces

Everything here is simulation core (`canwu-sim`) and public API
(`canwu-api`), plus mechanical signature changes in extension and integration
crates. No period-specific entity or rule enters the core.

| AGENTS.md row | Surfaces |
| --- | --- |
| Snapshot fields, journals, hashes, format versions | `persistence.rs` (continuation build and restore, seal, `SimulationCheckpoint`), `state.rs` (pins, facts, time index, ledger, resumable cache), `hashing.rs` (frontier finalization), `validation.rs` (counter-based lookup, continuation validation, ledger reuse), `mod.rs` (restore paths, record-version and time resolution), `replay.rs` (ledger and time index on replay), format8 and tamper tests |
| Random draws | `random.rs` (ledger, fresh-evidence predicate), `view.rs`, keyed-draw tests |
| Settlement and scheduling | `settlement.rs`, `ingress.rs`, `boundary_window.rs`: pinned fallbacks, time index append and rollback, `transactions.rs` checkpoints |
| Public API | `canwu-api` (`CanwuRun`, `CompactedCanwu` parity, viewer, continuation calls, re-exports), rustdoc, examples, `canwu-debug` check |
| Extensions and integrations | helper signatures in `canwu-fiscal`, `canwu-law`, `canwu-military`, `canwu-production`, `canwu-resource`, `canwu-society`, `canwu-history-research`, `canwu-economy-reference`, `canwu-force-supply-reference`, `canwu-ming-fiscal-reference` |
| Performance | `benchmarks/performance-harness` (`continuation-profile`, `keyed-draw-profile`), baselines |
| Dependencies | workspace `blake3 = "1.8"` |

## Vertical slice and durable acceptance tests

Each test must fail on `v0.15.0` and is named by its contract.

First commit:

1. **`crates/api/canwu-api/tests/keyed_draw_sealed_evidence.rs`**: a first
   draw citing a sealed ingress record, event, or boundary, and one citing
   the current record version, return the same value and draw ID in the
   unsealed run, the sealed run, and a run restored from sealed segments; a
   draw citing a superseded record version is refused in all three; an exact
   retry of a pre-seal draw returns it; `evidence_exists` and `evidence_time`
   agree across the three for sealed ordinal evidence without receipts; the
   three runs reach equal receipts and checkpoint hashes. The second commit
   adds the continuation-restored run.
2. **`crates/extensions/canwu-history-research/tests/plugins.rs`**,
   `citation_and_relation_checks_decide_alike_after_a_seal`: an assessment
   that cites a sealed record-change event and supersedes an assessment whose
   boundary is sealed is admitted alike in a sealed and an unsealed run, with
   equal receipts and checkpoint hashes.
3. **`crates/extensions/canwu-resource/tests/compact_host_helpers.rs`**:
   allocation passes, an archive commit and its retention acknowledgement, a
   completion operation, and a re-sent archived operation, driven through
   `CompactedCanwu` with an attached archive provider and a seal after every
   step, reach the receipts, state, archive objects, and checkpoint hash of
   the same drive on `Canwu`.
4. **`crates/integrations/canwu-ming-fiscal-reference/tests/compact_host_helpers.rs`**:
   the fiscal assessment, authorization, adapter evidence, and execution
   receipt flow, driven through `CompactedCanwu` with a seal after every step,
   reaches the receipts, fiscal state, and checkpoint and state hashes of the
   same flow on `Canwu`.
5. **`canwu-sim` unit tests**: the resumable stream hash against
   `blake3::hash` and the upstream vector inputs, with shape checks refusing
   malformed frontiers; the ledger equal to the ledger built from the full
   draw journal after a commit, a failed boundary, a seal, every restore path,
   exact replay, and a fork, with one build per restore and the ledger shared
   (not copied) by a seal and a checkpoint; sealed payload reads reporting
   `EvidenceContentUnavailable`.
6. **`crates/runtime/canwu-sim/tests/format8_contract.rs`**:
   `journal_commitment_roots_are_those_of_the_plain_stream_hash` pins the
   command, event, ingress, and draw roots of a fixed run to the values the
   0.15.0 `blake3::Hasher` computes for it. The checkpoint hash is not pinned
   (Part D).
   `persistence_contracts::knowledge_citing_a_sealed_boundary_does_not_block_the_next_seal`
   seals a boundary, publishes knowledge citing it, and seals again, one-phase
   and two-phase, then restores the checkpoint from its segments and refuses a
   listed receipt the segments do not rebuild.

Second commit:

7. **`crates/api/canwu-api/tests/continuation_checkpoint.rs`** (the slice). A
   fixture plugin with a plugin command whose handler queues a packet (its
   admission reads the cause command) and schedules a delayed event (a
   pending scheduled action), boundary systems that schedule packets up to
   30 days ahead for the fixture and for a second, receiving plugin (a
   cross-plugin packet whose issuer and target differ), cancel some of
   either through
   `cancellable_plugin_ingress`, retry a keyed draw at an address drawn
   before the cut, read `boundaries_between` over a declared seven-day
   horizon, publish superseding knowledge through `knowledge_heads`, keep
   payload-required continuations, and declare identity-only the last
   admitted packets of each kind (boundary-generated, command-caused, and
   cross-plugin), reading `plugin_ingress_matches` for each at every later
   boundary while declared; the host queues commands and calendar
   boundaries. The fixture exercises the pinning edge cases across a seal:
   (a) an identity-only dependency on the version a boundary supersedes,
   declared in that boundary right after the seal that archived the
   version's establishing boundary, so that only the pin taken at commit
   keeps it (the checkpoints after each seal check it); (b) cancelling a
   pinned pending packet whose producing boundary is sealed; (c) a
   `PayloadRequired` dependency on an event, declared in the boundary that
   admits it and read at every boundary across two seals (a boundary's own
   emissions become events only when it commits, so the earliest boundary
   that can declare one is the next). Three runs take identical
   inputs: unsealed `Canwu`, a `CompactedCanwu` that seals every five
   boundaries with work pending, and, from boundary `k` (where pending
   ingress, unadmitted events, and pending scheduled actions are asserted to
   exist), a run restored from the JSON of the first run's continuation
   checkpoint with `from_continuation_checkpoint_verified`, joined at `k` by
   a run restored from the first run's full save
   (`checkpoint_journal_json`). The continuation checkpoints are byte-equal
   at `k`, every fifth boundary
   after it, and the boundary after each seal, including the sealed run's
   (d); the restored runs seal on the sealed run's days. For 40 boundaries
   the runs
   produce equal receipts, checkpoint hashes, and authoritative state hashes;
   a second restore at `k + 20` joins the lockstep. The restored run refuses
   full-history operations with `HistoryUnavailable` and an archived
   segment read with `EvidenceUnavailable` (it holds no receipt before its
   base). The checkpoints are asserted to keep a cross-plugin fact and a
   fact for a packet no boundary generated, and a watched packet to have
   matched. The unsealed run's replay journal still replays. The whole
   scenario runs on a 1 MiB thread, so a debug build proves the runtime fits
   the smallest common thread stack. Beside it,
   `a_packet_first_declared_after_its_seal_is_not_ready` declares a sealed,
   no longer held packet and gets `ArchiveNotReady` from the sealed run's
   continuation checkpoint and next seal while the unsealed run writes one,
   and `a_scheduled_directive_cannot_enqueue_plugin_ingress` checks the
   refusal (Interactions).
8. **A continuation tamper test**: one altered byte in each part refused; an
   altered keyed value refused even with a recomputed `continuation_hash`; an
   altered pinned payload with a recomputed hash accepted by the unverified
   restore and refused by the verified one; a removed or an extra closure item
   refused by the round trip; a changed state field refused by the checkpoint
   hash; a malformed frontier refused without a panic; with a recomputed
   hash, a boundary ledger without its last boundary or reaching past the
   horizon, and a pending packet whose cause lies beyond the journal end,
   refused. A `canwu-sim` test refuses a checkpoint whose scheduled work
   would enqueue plugin ingress.
9. **The resource and fiscal drives of tests 3 and 4** taken to a
   continuation checkpoint with a pending completion operation and, for
   fiscal, an open assessment, an authorized execution, and the adapter
   result queued at the cut, restored with `_verified`, and run more than 30
   boundaries in lockstep with the uninterrupted run (the restored fiscal run
   seals in two phases). The fixture world of test 3 has no completion
   partition, so an active lease is not part of it; `gap_g53` covers leases
   on full-history runs.

## Documentation delta (for the orchestrator)

- `docs/versioning.md`: the 0.16.0 entry (formats table, breaking items,
  behaviour changes, known limits below); remove the two keyed-draw known
  limits of 0.15.0.
- `docs/architecture.md`: rewrite the sealing paragraph (work in flight,
  pins, per-journal cuts), the keyed-draw paragraph (ledger, fresh-evidence
  rule), the persistence paragraph (continuation checkpoint, what restore
  verifies, trust), and the evidence-time sentences in the domain-record
  section.
- `docs/terminology.md` and both website terminology pages: *continuation
  checkpoint* (suggested 接续检查点), *continuation closure*, *keyed-draw
  ledger*; update *checkpoint* and *operation-keyed random draw*.
- Website persistence page, both languages: continuation saves for host
  applications that need no replay.

Known limits to state: an undeclared non-current record version and generic
payloads stay seal-dependent (R1 to R4); the keyed-draw ledger grows for
ever, and the request index did until 0.18.0 bounded it to the run's request
outcome retention for consecutive request IDs
([request outcome retention](request-outcome-retention.md)); module restore
wrappers that replay history have no continuation form; a continuation
runtime cannot produce full-history artifacts.

## Non-goals

- Exact replay, a full snapshot, or a paged checkpoint from a continuation
  checkpoint, and a replay journal assembled from a continuation base and
  later segments.
- A content-addressed (paged) continuation form; the ledger is kept in
  draw-ID order so that one can reuse full chunks later.
- Expiring keyed-draw addresses or request idempotency, and shrinking the
  ledger entry (interning stream and producer text is a measured follow-up).
- Continuation forms of module restore wrappers and host validators that read
  history or exact version bodies (listed in Part F). The validators that
  read only current state and host stores run on a compact runtime.
- Making `Canwu: Send`, compressing saves, and making undeclared non-current
  record versions seal-stable.
- Engine enforcement of R4 beyond pinning at commit.

## Review record

An independent design review returned "implement with changes". Its
findings and their resolution:

- **B1, blocking: a dependency on a version its own boundary supersedes could
  never be pinned.** Accepted: Part B pins at boundary commit every version a
  boundary stops being current whose establishing boundary is sealed; K3 and
  F come from retained records, pins, or kept facts; a seal keeps commit-time
  pins still declared. The pin is taken for every superseded sealed version,
  not only for those the same boundary declares, because a dependency
  declared earlier while the version was current has the same hole.
- **S1: the settled cut is not bound by `checkpoint_hash`.** Accepted; added
  to what only the stored `continuation_hash` covers.
- **S2: two-phase commit needed receipts for `PayloadRequired` items.**
  Accepted: a pinned item satisfies it.
- **S3: fix `canwu-history-research` in this change.** Accepted (Part A).
  The relation read now goes through `replay_stable_domain_record_version`;
  because an earlier step of the same check already restricted relations to
  current versions, that change has no observable effect, and the durable
  test instead pins the citation check, whose outcome did depend on sealing.
  Host `domain_record_version` keeps returning `Option`, and host validators
  stay seal-dependent.
- **S4: time index as a run-length list without assertions.** Accepted. It
  covers sealed IDs only, extended by each seal, so no transaction rollback
  touches it.
- **S5: provenance facts carry the packet's status.** Accepted (Part B).
- **S6: table of read sites.** Accepted (Part B).
- **S7: frontier shape checks before any hazmat call.** Accepted (Part D);
  implemented in the first commit.
- **S8: slice edge cases.** Accepted (test 7).
- **S9: measure the slope.** Accepted (targets 5 and 6).
- **N1: rollback must not copy growing structures.** Accepted: pins and facts
  are `im` maps; the time index is outside boundary rollback.
- **N2: a compact fork keeps the committed seal tokens.** Accepted; first
  commit.
- **N3: `ErrorCode` becomes `#[non_exhaustive]` with `HistoryUnavailable`.**
  Accepted; second commit.
- **N4: bump the seal-token domain with the compact checkpoint domain.**
  Accepted; first commit.
- **N6: an unsealed run builds requests and flags with the seal's functions
  and domains.** Accepted (Part E).

The review of the first commit accepted it after fixes, all made:

1. *A seal required a receipt for every identity-only dependency*, so
   knowledge or a declaration citing evidence sealed earlier blocked every
   later seal. Fixed in the first commit (Part A, "A seal accepts what the
   counters prove"), with the reviewer's probe as a test; the second commit
   drops the receipt requirement altogether.
2. *A hostile frontier could overflow the stream length.* Fixed in the first
   commit: a frontier is capped at 2^62 bytes and every append checks the
   length (Part D).
3. *Measure the resumable hash's elapsed time.* Measured, and aligned
   multi-chunk subtrees are hashed in one call (first-commit results).
4. *Documentation*: the golden test pins journal roots, the restore's time
   index rule matches Part A, and the host helpers that check record-version
   existence say that the check is seal-dependent for undeclared superseded
   versions. Fixed in the first commit.
5. *A seal and a two-phase commit took a full fork for rollback*, copying
   structures that grow with every seal. Fixed in the second commit: those
   structures are `im` structures with shared values (Part B).
6. *The R1 to R3 claim missed two violators.* Corrected: `canwu-information`'s
   delegated interpretation authority and `canwu-law`'s signal and publicity
   provenance are listed as known violators (the rule plugins obey); their
   fix belongs to another package.

The review of both commits accepted them after fixes, all made:

- *B1, blocking: a pending cross-plugin packet blocked every seal* ("lacks
  its generation evidence"), because the fact was looked up by issuer equal
  to target. The generation is now found by ingress ID, and the fact keeps
  the issuer and the target in separate fields; each read answers from the
  right one (Part B). The lockstep test schedules cross-plugin packets.
- *S1: a declared sealed packet that no boundary generated kept no fact*, so
  `plugin_ingress_matches` was `EvidenceContentUnavailable` in a sealed run
  and `false` unsealed, and a boundary-generated packet first declared after
  its seal let the checkpoint differ by seal history. Every pending or
  declared ingress record now keeps a fact, including "not generated by a
  boundary"; one first declared after its seal and no longer held makes the
  checkpoint and the seal `ArchiveNotReady`.
- *S2: a full save refused a valid run whose command handler scheduled a
  directive enqueueing plugin ingress.* The loader cannot be relaxed
  exactly: a scheduled action that fires leaves its cause on the packet but
  no evidence of when it was issued, so a load cannot tell such a packet from
  a forged one. `validate_directives` therefore refuses a
  `SystemDirective::Schedule` that wraps `EnqueuePluginIngress`, at any
  depth, and the snapshot and continuation loaders refuse scheduled work
  that holds one (Interactions).
- *S3: debug builds overflowed a 1 MiB thread.* The commitment cache moved
  behind a box (first commit), and a seal rebuilds the kept receipts instead
  of removing from them (second commit). `gap_g46_api_paged_checkpoint` and
  the lockstep test run on a 1 MiB thread.
- *Notes.* Restore checks the boundary ledger's form (step 8), and a closure
  that cannot be derived is `InvalidSnapshot`; the trust section lists the
  pending set, the pinned payloads, and the boundary summaries among what
  only the stored hash covers; the `canwu-history-research` comment on
  sealed relations is corrected.

The delta review of both commits on the integrated tree accepted them after
two fixes, both made: `canwu-law` declared any cited ingress the counters
prove exists, so a host proposal citing a sealed, undeclared packet blocked
every later seal for as long as the proposal lived; it now refuses such a
citation, using the seal-stable `ingress_is_declarable` read and its own
declarations. And `SystemDirective::Schedule` documents that wrapping
`EnqueuePluginIngress` is refused and why.

Answers to the design questions, adopted: hazmat frontiers replacing the
accumulator's hasher; superseded record versions refused as fresh-draw
evidence in every run (a behaviour change); a failed boundary for a payload
read outside the closure; the host-stored `continuation_hash` as the trust
anchor, returned by `continuation_checkpoint()` and checked by
`from_continuation_checkpoint_verified`; R4 enforced only by pinning at commit
for now; restore to `CompactedCanwu` only; per-journal seal cuts at the
admission cursors; the time index kept; `HistoryUnavailable` as a new
variant; `CanwuRun` with the method set of Part F.

## Alternatives considered and rejected

- **Seal only up to the oldest pending item.** One packet due in a year would
  pin a year of evidence.
- **Carry the retained tail since the last seal instead of a closure.** The
  checkpoint would depend on when the run sealed, losing byte-equality across
  runs, and would still grow with seal cadence.
- **Carry archived receipts and segment headers.** They describe a seal
  history, not the run, and ordinal existence no longer needs them.
- **Persist the keyed ledger in `SimulationCheckpoint` with an incremental
  root.** A restore from segments rebuilds it anyway; persisting it only
  restores the per-draw checkpoint cost this package removes.
- **Restore into `Canwu`.** Its infallible full-history methods would return
  incomplete artifacts.
