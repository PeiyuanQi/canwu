# Legal Storage Sharding, COW, Delta Persistence, and Cold Archive

Status: implemented and independently accepted as the 0.8 single-milestone
contract on 2026-08-30. The first draft
was reviewed on 2026-08-29 and rejected at 5.2/10 with structural
blockers. Revision 2 closed most findings and scored 8.1/10, but four remaining
blockers prevented implementation approval. Revision 3 closed those findings;
a fresh independent reviewer scored it 7.3/10 and found five further contract
gaps. Revision 4 closed those gaps, scored 7.8/10, and exposed three remaining
integration blockers. Revision 5 incorporates them and was independently
approved at 9.4/10 with no remaining design blocker.

The implementation uses canonical Patricia state pages for domain records,
persistent decision maps and bounded multi-level archive locator pages, independently
versioned legal plan/directory/shard/archive-head records, replay-visible
decision and legal archive commits, persisted GC retention handoff, sparse
bitemporal indexes, and a generic owner-authorized cross-plugin maintenance
coordinator. The benchmark and conformance evidence is recorded in
[`docs/benchmarks/format8-2026-08-30.md`](../benchmarks/format8-2026-08-30.md).

The decision locator uses 4,096 stable primary buckets and 16 deterministic
subsegments per bucket. A page contains at most 64 receipts and 1 MiB of
canonical bytes, so exact lookup reads one segment and archive work remains
bounded even after a primary bucket grows. The locator page table is chunked
into authenticated directory pages of at most 1,024 page IDs, keeping the root
manifest below the 4-MiB state-page ceiling at one million keys. The hot decision-history commitment
is maintained incrementally with a restart-verified count/XOR/modular-sum
accumulator.

The final implementation closes four scale and restart obligations discovered
during independent review:

- decision verification emits full provider-authenticated page replacements,
  each bound to the prior page ID, so replay-safe boundary commit can append to
  an existing segment after root-only restore without reading the provider;
- legal effective-time and recorded-time directory cells point to ordered page
  segment vectors, allowing more than 64 versions in one time cell while every
  page remains below the 64-entry and 1-MiB limits;
- the production legal runtime persists a per-shard ordered compaction queue and
  count/XOR/modular-sum candidate authenticator. Prepare range-iterates that
  queue and materializes only the chosen batch; full catalog reconciliation is
  confined to explicit full cold restore or migration; and
- verified legal archive ingress carries one authenticated directory retention
  root. The law reachability participant expands it into exact blob,
  membership, and temporal marks before mark/sweep. The root survives snapshot
  restore, applied commits transfer reachability to the installed roots, and
  terminal stale rejection releases it. Persisted plugin descriptors retain
  internal-ingress ownership so this authority cannot disappear on restart.

The post-implementation scale review added five further closures:

- the real `Simulation::prepare_paged_checkpoint` boundary stores the locator
  directory as bounded pages and is exercised by million-key storage, restore,
  provider-backed lookup, replay, and zero-page repeat-delta evidence;
- paged checkpoint construction directly builds a body without paged record or
  decision payloads instead of first cloning and then clearing total state;
- canonical legal ingress remains one retention root regardless of accumulated
  membership and temporal page counts, while restart/settle/GC conformance
  proves transitive expansion;
- provider-backed bitemporal queries stream segments and enforce provider-call,
  segment, candidate, intersection, and decoded-byte budgets during I/O; and
- hot trace location and archive commit use stable ordinal lookup rather than a
  linear scan, with a trace-heavy scale gate targeting the final ordinal.

The final implementation-hardening pass closes the remaining persistence and
bounded-work findings:

- every legal archive commit is bound to the exact authenticated directory
  root, object count, shard, token, and expected source root rather than a
  caller-supplied summary;
- a persisted legal retention ledger records `Prepared`, `Verified`,
  `DurableIngress`, and terminal states, interlocks each handoff with the active
  GC epoch, keeps pending work as a new-object delta plus the proposed current
  page closure, and resumes safely after restart. Commit moves the prior object
  closure into the new root, retires the superseded root, and clears terminal
  handle payloads instead of copying cumulative history into every batch;
- legal compaction selection reads only the persisted ordered candidate range,
  scoped restore never rebuilds the global candidate catalog, and real Canwu
  ingress proves that an untouched candidate shard is neither restored nor
  rewritten;
- repeat decision checkpoints inspect only changed locator directory pages;
  load rejects noncanonical directory placement and any cold object duplicated
  in hot metadata;
- one end-to-end legal temporal-query meter charges membership, temporal,
  directory, and blob reads across all shards, so a caller cannot reset budgets
  by crossing a shard boundary; and
- boundary proposal overlays now apply record changes to structurally shared
  roots and validate only their affected closure. Empty stages return before
  touching the record store, so unrelated large record payloads do not enter
  ordinary boundary cost.

The final independent implementation review identified and closed three
restart/finalization gaps:

- paged decision restore now derives hot-to-archive ticket/trace dependencies
  and authenticates only their exact locator pages before validating hot state;
- synchronous legal archive commit authenticates the retention handle and full
  directory/index/blob closure before applying on a clone, and publishes the
  clone only after store finalization; idempotent retries must match the
  installed archive head; and
- canonical legal archive terminal outcomes are persisted by retention handle,
  not only as one last record. Store finalization derives the disposition from
  a reloaded authoritative runtime, then queues a private acknowledgement that
  removes the per-handle recovery record after the store handoff succeeds; and
- the detached archive-transition primitive is crate-private. Public
  synchronous callers must use the store-authenticating helper, while live
  simulations admit the same proof only through canonical ingress. A
  first-application test jointly tampers the pending, membership, effective,
  and recorded roots and proves both runtime and retention remain unchanged.

This implementation closes the scale limit documented by the
[legal institutionalization framework](legal-institutionalization-framework.md):
the pre-Format-8 `canwu-law` extension persisted one complete legal aggregate,
and the kernel cloned or materialized the complete generic domain-record map in
transaction and boundary-overlay paths. Format 8 replaces both sides of that
coupling: law state is independently sharded, while kernel rollback, mutation,
proposal overlays, checkpoints, and decision history use persistent roots and
content-addressed deltas.

The work therefore ships as one release milestone with four inseparable
capabilities:

1. legal-order and jurisdiction hot-state sharding;
2. persistent copy-on-write kernel stores for domain records and decision
   state;
3. content-addressed page deltas for incremental checkpoints;
4. verifiable cold archives for closed legal and decision history.

The internal implementation gates below are review and rollback boundaries,
not separately supported releases.

## Decision summary

Canwu retains every authoritative legal change logically, but it does not keep
every historical payload in the hot mutation path.

- Current rule heads, scheduled changes, unresolved disputes, open proceedings,
  live applicability projections, pending decisions, and compact provenance
  receipts remain hot.
- Superseded versions, closed proceedings and cases, replaced ballots, terminal
  decision traces, and repealed-rule detail move into immutable archive
  segments after their dependencies close.
- A culture target becoming `Retired` never automatically repeals a law. An
  enacted human-rights or women's-suffrage rule remains in the hot legal heads
  and current applicability projections until an explicit legal operation
  amends, repeals, expires, or displaces it.
- Archiving is physical placement, not semantic retirement or deletion. Exact
  identities, causal evidence, historical queries, replay, succession, and
  retroactivity remain recoverable through committed archive receipts.
- Compaction is a deterministic, bounded maintenance transition selected from
  persisted counters and committed through canonical maintenance ingress at a
  simulation boundary. It is not a wall-clock thread.
- No hot payload is released until the engine reads the stored archive bytes
  back, verifies their content, and atomically commits the exact prepared token.

This is a deliberate format-8 clean break. The 0.8 runtime must not silently
reinterpret format-7 snapshots or exact-replay journals. Applications that
need a 0.7 campaign must keep the 0.7 engine or perform an explicit
application-owned export into a new run or causal branch.

## Goals

- Make ordinary legal settlement depend on the touched hot shards and delta,
  not on total retained legal history.
- Make transaction rollback and forks structurally share untouched domain and
  decision state.
- Let an incremental store persist only changed content-addressed pages plus a
  new root, while retaining a self-contained portable full-save path.
- Keep current-law queries available without an archive provider.
- Keep historical-law queries exact and fail closed when required cold content
  is unavailable.
- Bound every compaction selection, archive read, shard fan-out, mutation,
  validation, and reconstruction path.
- Reuse Canwu's existing typed domain records, atomic boundary mutation bundle,
  exact evidence, checkpoint, replay, and two-phase archive conventions.

## Non-goals

- Distributed consensus, multi-process authoritative shards, or network
  transactions.
- Dropping old legal changes while continuing to call the result exact replay.
- Making legal settlement per-frame work; games may still run it at a turn,
  event, or background-simulation cadence.
- Unbounded natural-language legal research or theorem proving.
- Moving immutable record identity when a jurisdiction splits, merges, is
  conquered, or succeeds another order. Those changes remain explicit
  reception and succession records.
- Choosing historical doctrine, rights content, voting qualifications, or
  institutional legitimacy in the generic engine. Content packs own those
  choices.

## Required invariants

The implementation is acceptable only if all of these remain true.

1. **One logical history.** Hot and cold placement never create two
   authoritative locations for the same `LegalVersionRef`.
2. **Stable identity and exact versions.** A mutable legal object keeps one
   stable object identity, while each authoritative change creates an immutable
   exact version with its own ordinal and canonical content commitment. Only
   exact versions move to cold storage.
3. **Current-law independence.** Current applicability is derivable from hot
   heads and projections without loading closed history.
4. **Historical exactness.** A historical query either returns the same result
   and trace as the un-compacted ledger or returns an explicit archive error. It
   never treats missing cold content as absence, repeal, or inapplicability.
5. **Culture/law separation.** Cultural retirement removes cultural hot-path
   propagation state, not accepted legal commitments or enacted legal effects.
6. **Atomic archive release.** Preparing or storing an archive candidate does
   not change authoritative state. Committing either installs every manifest,
   receipt, hot-state reduction, and dependency update or installs none.
7. **Deterministic maintenance.** Candidate order, exact segment membership,
   shard order, canonical encodings, tokens, and maintenance wakes derive only
   from persisted state and declared configuration.
8. **Bounded work.** Every live path has declared record, byte, page, shard,
   dependency, and archive-read budgets checked before mutation.
9. **No half cross-shard law.** A multi-jurisdiction enactment, ruling, repeal,
   or succession commits all participant shard changes in one kernel mutation
   bundle or commits none.
10. **Replay-visible semantic placement.** Legal and decision archive commits
    are canonical maintenance ingress applied in ordinary boundaries. They may
    change the authoritative checkpoint root while preserving the current legal
    projection, and exact replay reproduces the same transition. Merely storing
    canonical COW pages is persistence I/O and does not mutate simulation state
    or revision.
11. **Tamper evidence.** Page roots, segment roots, predecessor links, archive
    receipts, and checkpoint roots reject omission, overlap, reordering,
    substitution, and payload mismatch.
12. **Fork sharing.** Relative to domain and decision record count, a fork or
    rollback capture shares immutable roots and cold segments in `O(1)`. Other
    runtime domains retain their separately documented clone costs until they
    are independently converted to COW.
13. **Committed referential integrity.** Forward references, reverse
    dependents, successor predecessors/children, and their Merkle roots change
    atomically and are fully reconstructible from primary records.
14. **Canonical storage shape.** The domain/decision Merkle root is a pure
    function of the current logical key/value set and format constants, never
    of insertion, deletion, compaction, or fork history.

The central semantic invariant is:

> At any committed read cut, the authoritative legal result is derivable from
> the relevant hot shard heads plus an ordered, hash-bound set of immutable
> archive segments. Compaction may change placement, never identity, causal
> meaning, query result, or replay result.

## Pre-Format-8 0.7 bottleneck

The scale probe separates law-local work from the real kernel path:

| Retained law-local records | Law-local idle settlement | Full plugin boundary |
| ---: | ---: | ---: |
| 1,000 | about 200 ns median | 37.472 ms median |
| 10,000 | about 200 ns median | 507.535 ms median |
| 100,000 | about 200 ns median | 6.017504 s median |

The full path includes aggregate decode, boundary rollback capture, domain
record compare-and-set, and aggregate encode. The pre-groundwork 0.7 baseline
was:

- `LegalRuntimeRecord { id: "root" }` contains the compiled plan, every legal
  record, and every index;
- `BoundaryTransactionCheckpoint::capture` cloned the whole
  `BTreeMap<DomainRecordRef, DomainRecord>` and `DecisionState`;
- `apply_mutation_bundle` cloned that domain-record map again before changing
  one entry;
- `SimulationCheckpoint` serialized all current domain records even when only
  one changed.

The current private groundwork removes only the boundary-capture map clone;
first-write map copies, aggregate rewriting, decision cloning, and flat
checkpoint serialization remain. Consequently, legal sharding alone would
still pay whole-kernel linear work, and kernel root sharing alone still pays a
whole legal-shard payload rewrite. Both layers must change in the public
milestone.

## Target architecture

```text
                         portable full snapshot
                                  |
                                  v
  public flat records <-> PersistentDomainRecordStore <-> page delta store
                              |        |       |
                              |        |       +-- hash-cached COW pages
                              |        +---------- decision receipt/COW pages
                              +------------------- typed legal shard records
                                                     |
                     +-------------------------------+--------------------+
                     |                               |                    |
             legal plan/directory             order hot shard     jurisdiction hot shard
                                                     |                    |
                                                     +---------+----------+
                                                               |
                                                    cross-shard coordinator
                                                               |
                                                   archive head + receipts
                                                               |
                                            immutable legal/decision segments
```

The public domain-record model remains typed records and exact version refs.
The COW tree, page layout, and archive manifests are private persistence
mechanisms except for explicit provider/store traits and portable wire types.

### Run-manifest finalization

Format 8 forbids authoritative late plugin registration. Construction uses a
non-authoritative `RunManifestBuilder`: the host registers the complete plugin
descriptor set, owned schemas, target-owner routes, dependency resolvers,
committed provider descriptors/policy, formats, and budgets before any ingress
or simulation boundary. Live provider instances and current availability are
host runtime state and never enter the manifest.
One `finalize_run_manifest()` step sorts and validates those declarations,
computes manifest/configuration hashes, authority seed, run identity, initial
plugin/root commitments, and returns the immutable `RunManifest`.

After finalization, registration APIs return a typed error and cannot change the
authority seed or manifest hash. Snapshot/load accepts only a finalized manifest
whose descriptor set and resolver/owner registry byte-compare with the persisted
commitments. A builder is neither serializable as an exact run nor usable to
admit commands.

## Kernel persistent record store

### Private representation

`RuntimeCurrentState.domain_records` changes from a cloned `BTreeMap` to a
private `PersistentDomainRecordStore`. `DecisionState` receives the same
storage primitive for its large keyed collections.

The reference implementation is an `Arc`-shared canonical binary Merkle
Patricia trie:

- a domain-separated 256-bit BLAKE3 hash of the canonical record reference
  determines a maximum 256-bit search path;
- every internal node records the first discriminating bit for its current key
  subset and exactly two child hashes; discriminating positions strictly
  increase down a path;
- unary nodes do not exist, so paths are compressed without separate extension
  objects or threshold-driven split/merge decisions;
- a full-hash leaf stores the exact key/value; distinct keys with the same
  256-bit hash use one canonically sorted collision leaf with a strict count and
  byte ceiling;
- branches and leaves cache item counts, canonical encoded-byte estimates, and
  Merkle hashes;
- mutation copies only the actual discriminating-branch path and changed leaf.

The canonical builder recursively branches on the first differing bit of the
current complete key-hash subset. Therefore the same logical key/value set
always reconstructs the same tree and root, regardless of insertion order or
mutation history. With `N` non-colliding keys, the tree has exactly `N` leaves
and at most `N - 1` branch nodes. Random permutation,
insert/delete/reinsert, fork divergence/rejoin, and flat export/import property
tests must prove this contract.

The store owns additional persistent Merkle indexes:

- `reverse_references[target] -> sorted dependent records`;
- `successor_of[predecessor] -> successor`;
- `predecessors_of[successor] -> sorted predecessors`.

Reverse-dependent and predecessor sets are composite-key subtries such as
`(target, dependent)` and `(successor, predecessor)`, not one unbounded sorted
vector payload.

These indexes use the same canonical trie primitive. Their roots are committed
beside the primary record root. Cold load rebuilds every index from primary
records and compares each root before exposing state.

No runtime path may expose the tree's placement as domain semantics. Public
full snapshots materialize records in canonical `DomainRecordRef` order.
Cold validation uses a read-only store trait rather than requiring a cloned
`BTreeMap`; validators that truly need a flat map may materialize one only in
explicit offline diagnostics.

### Commitment change

Format 8 replaces the flat domain-record commitment with the canonical primary
record root plus reverse-reference, successor-of, and predecessors-of roots,
and bumps the overall commitment contract. A leaf commits its canonical key, owner, class,
version, lifecycle, payload, and references. A branch commits its fixed depth,
occupied child slots, child hashes, counts, and byte totals. The outer
checkpoint continues to bind these roots with world, knowledge, scheduler,
decision, evidence, run, and plugin roots.

This commitment change is why the milestone cannot pretend to be a compatible
format-7 optimization.

### Transaction behavior

- Boundary rollback captures an `Arc` root, not all records.
- `apply_mutation_bundle` applies its sorted requests to a transient store
  builder and freezes one new root only after every mutation validates.
- Record-reference and successor validation reads through a
  `DomainRecordRead` interface over the base root plus a bounded mutation
  overlay. The builder derives reverse-index additions/removals for every
  changed forward reference.
- A target deletion or retirement visits its committed reverse dependents and
  rejects any surviving invalid reference. Adding a successor edge performs a
  bounded ancestor/descendant closure check through the committed successor
  indexes and rejects a cycle.
- Kernel `RunConfiguration` budgets changed forward edges, reverse dependents,
  successor closure depth, visited nodes, and generic index mutations.
  Exceeding a budget rejects before root swap; it never falls back to a full
  graph scan in live settlement. The law plan owns only law-specific shard,
  projection, procedure, and archive limits.
- Failure discards the new root. Success swaps one root and emits the same
  logical `DomainRecordChange` evidence.
- Forks clone roots and caches in constant time; later writes copy paths.

The live complexity for `K` changed records is
`O(K * structural_path + changed_payload + affected_reference_closure)`, with
at most 256 key-hash bits compared but without one physical node per bit. It is
not `O(total_domain_records)`. Conformance includes an unchanged
source pointing to a deleted target, a successor cycle formed across separate
bundles, a many-predecessor merge, and each committed-index tamper case.

## Incremental page-delta persistence

The ordinary current-state snapshot may intentionally cost
`O(total current state)`. A self-contained **exact** portable bundle additionally
contains every reachable historical legal, decision, and evidence segment.
Long-running hosts use the compact checkpoint path and content-addressed stores.

Format 8 adds a domain-neutral content-addressed page contract:

```rust
trait StatePageProvider {
    fn load_state_page(&self, page_id: &str)
        -> Result<Option<StatePageBlob>, CanwuError>;
}

trait StatePageStore: StatePageProvider {
    fn store_state_page(&self, page: &StatePageBlob)
        -> Result<ArchiveStoreOutcome, CanwuError>;
}
```

Names are provisional until implementation and terminology review. The
contract follows the existing evidence archive pattern:

1. `prepare_state_delta` reads immutable current record/decision roots directly
   and returns only content-addressed nodes not reachable from the caller's
   declared prior durable root; it must not fork the complete `Simulation`;
2. the host stores every page idempotently;
3. `verify_state_delta` reads the exact bytes through the provider, verifies all
   node hashes and the prepared root, then returns a durable paged-checkpoint
   envelope;
4. a stale source root, missing page, extra child, hash mismatch, count
   mismatch, cycle, excessive depth, duplicate child, or byte-total mismatch
   rejects the whole operation before unbounded allocation;
5. checkpoints retain root descriptors and current hot non-paged state, while a
   portable bundle can embed all reachable pages for transfer.

The implemented paged-checkpoint envelope is format `3` and carries the compact
`SimulationCheckpoint` plus page roots. A non-empty checkpoint journal cut is
restorable only with the exact contiguous evidence prefix; a state-only restore
is rejected rather than silently manufacturing an empty history.

`PageId` is the hash of one unique canonical uncompressed page encoding, and
`StatePageProvider::load_state_page(PageId)` returns exactly those canonical
bytes. A host may compress them internally, but that representation is opaque
to Canwu and must decode back to the one canonical byte sequence. There is no
format-level ambiguity between semantic page ID and storage blob ID.

Storing or verifying state pages does not update authoritative simulation
state, checkpoint root, revision, or replay journals. It proves that the host
can persist an already-authoritative logical root and produces a persistence
envelope around that same root.

Reachability-based garbage collection is host-owned. It may remove a page only
after that page is unreachable from every retained checkpoint, fork, archive
manifest, portable-save promise, and pending prepared operation. The engine
exposes deterministic reachable-page enumeration plus explicit retention
handles for live forks and prepared tokens. A host releases a handle only after
the associated fork/checkpoint/pending operation is durably abandoned.
Retention handles are process-lifetime capabilities, not replayable authority:
a process crash releases undurable prepare handles, making unreferenced stored
objects GC-eligible. Once a verified maintenance packet enters durable ingress,
the ingress/root reachability contract protects its named objects instead.

## Legal hot-state shards

The single `LegalRuntimeRecord` is replaced by these typed kernel records:

| Record | Cardinality | Hot responsibility |
| --- | --- | --- |
| `LegalPlanRecord` | one | Compiled plan, plan/content hashes, immutable budgets, schema version. |
| `LegalDirectoryRecord` | one | Active order/jurisdiction shard descriptors and global archive/provider policy. Updated only when shard topology changes. |
| `LegalOrderShardRecord` | one per legal order | Stable rules, current and scheduled normative heads, source heads, succession/reception state, order-level dirty indexes. |
| `LegalJurisdictionShardRecord` | one per `(legal_order, jurisdiction)` | Open proposals/procedures/cases, local findings/rulings, current applicability/conflicts, pending intents/outbox, local dirty indexes. |
| `LegalCoordinatorRecord` | one per live cross-shard operation | Sorted participant set, expected kernel versions, exact intent/evidence, phase, and final result. |
| `LegalArchiveHeadRecord` | one per hot or archive-only shard | Primary membership/location root, secondary index roots, dependency root, archive batch sequence, pending prepared token, and compact current receipts. |
| `LegalCultureDependencyRecord` | one per referenced culture target generation | Exact open, scheduled, and operative live-level dependents plus bounded reference counts used by culture retirement. |

Record references are derived from canonical plan IDs; ordinary reads do not
need a directory scan. The directory is a topology and policy authority, not a
per-boundary global counter or routing hotspot.

The overloaded v1 `LegalRecordRef` is replaced by three different contracts:

```text
LegalObjectId {
  kind,
  id,
  home_shard
}

LegalVersionRef {
  object: LegalObjectId,
  version_ordinal,
  content_commitment
}

LegalHeadRef {
  object: LegalObjectId,
  version: LegalVersionRef
}
```

`LegalObjectId` is stable across the lifetime of a proposal, procedure, outbox
item, pending intent, rule, case, or other mutable logical object. Every
authoritative mutation creates an immutable logical version and atomically
updates the owning shard's head. `LegalVersionRef` identifies one exact version
and is the only form admitted to an archive segment. `LegalHeadRef` is a
persisted snapshot of which exact version was current at a committed cut; it is
never interpreted as "whatever is latest now."

Reference rules are explicit:

- proposal/procedure/case ownership uses `LegalObjectId`;
- evidence, rulings, legal versions, transition predecessors, and replacement
  chains use `LegalVersionRef`;
- an outbox or pending-intent state change creates a new exact version and moves
  its hot head;
- a ballot participation is immutable; replacement names the exact replaced
  ballot version;
- a procedure stage transition names the exact prior procedure version;
- current indexes store `LegalHeadRef` values and validate their commitments.

Kernel-owned records continue to use `DomainRecordVersionRef`; the extension
does not fabricate kernel provenance for logical versions stored inside a shard
or archive segment.

### Ownership rules

- Normative sources, stable rules, immutable law versions, and order heads live
  in the order shard.
- Proceedings and cases live in their compiled forum jurisdiction shard.
- Applicability and conflict projections live in each affected jurisdiction
  shard and reference the authoritative order-shard version.
- A rule covering several jurisdictions has one normative identity in the
  order shard, not copied competing law versions. Jurisdiction shards contain
  projections and exact references.
- IDs derive from the accepted request/command identity plus the owning shard;
  no global mutable sequence is required for ordinary creation. When one
  request creates multiple objects of the same kind in the same shard, each
  draft must carry a bounded canonical `local_discriminator` unique within
  `(request, shard, kind)`. The object ID commits that discriminator. Engine-
  generated drafts use frozen semantic role keys or ordinals assigned only
  after canonical sorting of fully materialized drafts, never container
  iteration order.
- A succession/reception relation has one canonical coordinator identity
  derived from its sorted predecessor/successor orders and effective cut. The
  terminal coordinator owns the immutable relation version; every participating
  order shard stores a query projection. Closed coordinators may later archive.
- A shard that leaves active topology becomes an immutable archive-only
  descriptor/tombstone in the directory. Its home-shard route and archive roots
  remain valid for old references.
- Every live cultural level dependency is mirrored in the exact target-keyed
  `LegalCultureDependencyRecord`. Enact, schedule, amend, repeal, displacement,
  archive, and dependency resolution update the owning legal shards and this
  law-owned record in the same law mutation bundle.

### Cross-shard commit

A multi-jurisdiction operation creates or resumes one coordinator whose ID is
derived from the canonical request. The coordinator:

1. freezes the sorted participant order, jurisdiction shards, order shards,
   and any affected culture-dependency records;
2. freezes every expected kernel `DomainRecordVersionRef` and legal head;
3. computes the complete bounded mutation set before writing;
4. submits the coordinator and every participant mutation in one existing
   kernel `DomainDeltaProposal` stage;
5. becomes terminal only in the same atomic bundle as the final shard writes.

The kernel already rejects duplicate targets and applies a boundary mutation
bundle atomically. This design uses that local transaction; it does not add
distributed two-phase commit. Any stale participant rejects the entire bundle,
and retry reuses the same coordinator identity.

The plan declares `max_cross_shard_participants`, total changed-record bytes,
and total projection mutations. A request above any bound is rejected before
mutation.

### Owner-authorized cross-plugin retirement

Culture retirement is not an ordinary law `DomainDeltaProposal`: neither the
culture plugin nor the law plugin may fabricate a mutation against the other
plugin's record kind. Format 8 adds a generic kernel
`OwnerAuthorizedMaintenanceCoordinator` for bounded cross-plugin retirement and
dependency-resolution transitions.

For a culture target generation, the coordinator:

1. derives one canonical retirement request identity from the admitted ingress,
   target generation, and requested retirement cut;
2. resolves the target record's schema owner from the run manifest, reads the
   compiled dependency-resolver registry for that target namespace, and freezes
   the mandatory target owner plus the sorted dependent-owner plugin set;
3. invokes the target owner and every registered dependent owner in stable
   plugin order through a private
   `OwnerAuthorizedMaintenanceParticipant` interface;
4. requires the target owner's opaque proposal to supply the exact target record
   ref/version and the owner-defined culture retirement mutation; each dependent
   owner supplies only its target-keyed refs and dependency changes. Every
   proposal commitment binds participant role, owner descriptor and semantic
   hash, target generation, exact source refs, expected versions, bounded
   mutation hash, and accept/reject result;
5. verifies that each proposal mutates only record schemas owned by that plugin,
   that the target schema owner and every required dependent owner participated
   exactly once, and that the law owner found no unresolved or operative
   `LiveLevel` dependent;
6. applies the target-owner-proposed culture retirement mutation and the
   law-owner-proposed dependency
   acknowledgement/removal in one private transient domain root; and
7. swaps that root once and emits one cross-plugin retirement receipt and one
   boundary evidence record.

The kernel coordinates, validates ownership, and commits proposals; it never
synthesizes culture retirement semantics. Any missing target owner, missing
dependent owner, rejection, stale expected version, changed participant set, or
over-budget proposal aborts the whole transition. No owner mutation becomes
visible. Adoption commitments and historical evidence do not block retirement;
unresolved or operative `LiveLevel` dependents do. The target-keyed law record
keeps the check bounded without allowing culture to mutate law state. The same
kernel protocol may serve future owner-authorized retirement relationships; its
vocabulary is not law-specific. Target ownership and dependency-resolver
registrations are frozen in the run manifest and plugin descriptor commitments,
so a caller cannot omit either the culture owner or the law participant. The
admitted request, opaque owner proposals, final receipt,
and one maintenance change record are exact-replay inputs/outputs; replay invokes
the same owners and byte-compares the participant set, mutations, and final root.

## Legal and decision cold archives

### Segment contents

Each shard owns immutable `LegalHistorySegment` values. Archive sequence is
separate from creation/version ordinals: a low-ordinal procedure may remain
open while any number of later closed versions archive. A segment commits its
exact sorted `LegalVersionRef` membership and never claims that a numeric
ordinal interval is completely present.

`SegmentContentMaterial` is the canonical, uncompressed semantic material. It
contains no content ID, blob ID, or future commit-boundary identity. It commits:

- schema and canonical encoding version;
- owner shard and monotonic archive-batch sequence;
- exact member count, member Merkle root, and advisory min/max version ordinal;
- canonical record-content root and uncompressed byte count;
- exact archived evidence and decision receipts required by those members;
- the already-known logical source roots, record versions, and source boundary.

Content addressing is acyclic and has three layers:

1. `ContentId = H(SegmentContentMaterial)`;
2. `StoredBlob = Encode(fixed_codec_descriptor, SegmentContentMaterial)`;
3. `BlobId = H(fixed_codec_descriptor || StoredBlob)`.

The later authoritative `ArchiveObjectReceipt` binds `ContentId`, `BlobId`, the
codec descriptor, source-root commitment, verified mutation-plan hash, actual
maintenance ingress identity, and actual committing boundary. It does not bind
the final checkpoint hash. IDs, ingress identity, final roots, and future
boundary values are therefore not hashed through themselves.

Archive providers load by `BlobId`. The authoritative receipt maps
`ContentId -> BlobId`, declares codec/version and hard compressed/uncompressed
ceilings, and verifies the decoded canonical content before allocation beyond
the declared limits. Different physical encodings therefore cannot alias under
one provider key.

The archive head commits independent immutable index roots:

- primary membership/location:
  `LegalVersionRef -> { ContentId, BlobId, member_index }`;
- object/version and exact ID lookup;
- rule/version lookup;
- case/finding/ruling lookup;
- bitemporal effective/read interval lookup;
- evidence and decision dependency lookup;
- archive-batch and shard tombstone lookup.

Every secondary-index leaf binds the primary member commitment. One archive
commit atomically replaces all affected roots; omission, duplicate membership,
or two locations for one version is invalid. Manifest/index nodes are
content-addressed bounded-fan-out trees. The hot archive-head record stores only
their roots, counts, a small pending buffer, and current availability receipts,
so total segment count does not grow the hot record linearly.

Placement is never inferred from a scalar high-water ordinal. A hot head embeds
the exact version it needs; a historical lookup uses the sparse primary
location root. Conformance includes one permanently open ordinal followed by
1,000,000 closed versions and proves bounded hot state plus exact routing.

### Canonical bitemporal archive index

The persisted format-8 point domain has fixed width `W = 64`. A finite
`SimTime(i64)` is order-preservingly encoded by flipping its sign bit. Interval
endpoints use a tagged `Finite(encoded_u64) | Open` representation, where
`Open` is the one-past-domain upper sentinel and cannot alias `i64::MAX`. Each
half-open effective-time or recorded/read-time interval is converted to its
unique minimal dyadic decomposition over the 64-bit point domain, containing at
most `2W` cells. The archive head owns two independent canonical sparse dyadic
tries, one for effective time and one for recorded/read time.

```text
TemporalNode {
  prefix_length,
  significant_prefix_bits,
  bucket_root,             // LegalVersionRef -> primary_member_commitment
  left_child,
  right_child,
  subtree_node_count,
  subtree_member_count
}
```

Each node's prefix is also its exact time-subtree bound; unused suffix bits must
be zero. `bucket_root` is a canonical Patricia map keyed by
`LegalVersionRef`. A temporal node exists only when its bucket is non-empty or
it is required to join two children. A unary node with an empty bucket is
compressed into its child, which retains its full prefix length and bits.
Branching always occurs at the first discriminating time bit. Consequently the
same set of `(dyadic_cell, LegalVersionRef, member_commitment)` entries has one
tree shape and root regardless of insertion history.

Validation freezes every allowed shape:

- a node may have a non-empty bucket with zero, one, or two children;
- an empty-bucket node must have exactly two children, otherwise it must be
  absent or compressed;
- each child's prefix strictly extends the parent, the left/right first
  differing bit is respectively zero/one, child bounds are disjoint, and the
  stored branch bit is the first discriminating bit;
- prefix length is at most 64, bucket Patricia depth is at most 256, and all
  subtree node/member counts use checked arithmetic and equal recomputed values;
- encoded byte/count limits are preflighted before allocating a decoded bucket
  or descending into children.

Insertion and deletion recompute the canonical decomposition for that exact
interval, create/split or remove/compress temporal nodes deterministically, and
insert or remove the version from every cell bucket. A bucket entry is invalid
unless its bound primary membership exists at the same archive-head generation.
An archive commit changes the primary root and both temporal roots atomically;
partial updates fail validation.

A point query at time `t` follows one bounded root-to-leaf time path in each
dimension and collects every encountered bucket whose prefix contains `t`.
Compressed skipped levels cannot hide a bucket because any non-empty bucket
requires a node. The query sorts and deduplicates each dimension's candidate
`LegalVersionRef` set, intersects the two sets, proves each surviving primary
membership, and only then hydrates segments.

A range query decomposes its range canonically. For each query cell it traverses
the path to that cell and collects buckets on stored ancestor nodes, then
performs a budgeted traversal of temporal nodes whose prefixes are descendants
of the query cell. Dyadic cells overlap only through this ancestor/descendant
relation. Subtree node/member counts preflight traversal budgets; actual
candidates are sorted and deduplicated before the same cross-dimension
intersection and membership proof.

Let `C_e` and `C_r` be candidates admitted by the effective and recorded
indexes. Point-query work is
`O(W + C_e + C_r + sorted_intersection + hydrated_members)`. Range work adds
the query's at-most-`2W` cells plus admitted descendant temporal nodes. Heavy
interval overlap can make either candidate set inherently large; the engine
therefore enforces explicit temporal-node, bucket-node, prefix, candidate,
deduplication, intersection, segment, provider-call, decoded-byte, and result
budgets and returns a typed query-budget error rather than scanning or claiming
a fixed-hit logarithmic guarantee. Deterministic insertion order, deletion,
rebuild, random sparse data, boundary endpoints, open intervals, and
pathological-overlap fixtures must produce identical roots and bounded failure
behavior.

Terminal decision tickets, attempts, traces, and complete request payloads use
parallel `DecisionHistorySegment` values with the same sparse membership and
multi-index contract. The hot decision store retains the minimal
request/result/idempotency receipt needed to prove that a retried request has
the same original outcome. A decision is not archive-eligible while any open
procedure, outbox item, appeal/review path, or payload-required dependency still
needs it hot.

### Decision history public API

Format 8 removes the public implication that all decision history is resident.
`Canwu::decision_state()` and borrowed all-history
`decision_attempts()`/`decision_traces()` slices, including equivalent
persistence-wrapper accessors, are replaced by contracts that state storage
availability explicitly. Exact location uses a typed key rather than an
ambiguous scalar:

```text
DecisionHistoryKey = Ticket(DecisionTicketId)
                   | Attempt(DecisionRequestId)
                   | Trace(DecisionTraceId)

DecisionHistoryLocation = Hot
                        | Archived(DecisionArchiveLocator)
                        | Unresolved(LocatorBucket)
                        | Absent
```

Exact location is backed by a canonical state-page-resident
`DecisionHistoryLocatorIndexRoot`, not a linear resident map. It maps every
known typed key to its hot generation or exact archive locator; deletion from
the index is forbidden within an exact run. Provider-backed exact location
loads a bounded Merkle path and proves hit or absence. A missing page returns
`StatePageUnavailable`, not `Absent`. Current retry and settlement paths use hot
receipts/indexes and do not require this provider for arbitrary old keys.

Decision validation no longer assumes resident trace IDs are a contiguous
prefix from one. State commits global issued/next counters plus per-kind locator
metadata. The trace locator subroot commits minimum/maximum issued ordinal,
cardinality, hot/archive counts, and no-duplicate membership; its metadata proves
prefix completeness without hydrating every locator page. Every hot item must
have one `Hot` locator at the same generation, every archived receipt must have
one archive locator, and the two sets are disjoint. Full cold restore and exact
audit rebuild locator membership and compare the root; ordinary hot validation
checks counters, root metadata, and touched paths only. Sparse/non-prefix hot
retention after arbitrary archival is a required fixture.

Prefix-completeness arithmetic applies only to engine-issued contiguous ID
families such as `DecisionTraceId` and `DecisionTicketId`. Caller-selected
`DecisionRequestId` values are not interpreted as an ordinal space; their exact
locator membership and absence proofs cross-check committed request receipts
and accepted-ingress identities instead.

- `decision_hot_state()` and hot-only iterators expose bounded resident state;
- `decision_receipt(request_id)` returns the compact current retry/result proof;
- `decision_history_location(DecisionHistoryKey)` returns the typed location above;
- hot exact ticket/attempt/trace accessors return a value only when the locator
  is `Hot`, rather than conflating archived and absent;
- provider-backed exact lookup returns an owned, verified historical value;
- bounded provider-backed historical queries are paginated and return owned
  pages plus continuation commitments; and
- unavailable or incomplete cold history returns `DecisionHistoryUnavailable`,
  `ArchiveNotReady`, or a typed query-budget error, never an empty slice.

The compact current checkpoint persists hot state, retry receipts, archive
roots, and locators, not all attempts and traces. A self-contained exact bundle
includes the reachable decision segments. Compatibility adapters may copy hot
items into an owned collection, but no 0.8 API promises a process-lifetime
borrow of complete historical state.

Historical legal segments embed the exact `ArchivedEvidenceReceipt` values
needed by their members. Those receipts and the legal archive roots are part of
kernel archive reachability, but they remain cold rather than one receipt per
historical item in hot state. Legal history may archive only after every cited
evidence item is sealed or otherwise has a committed exact receipt. A
self-contained exact portable bundle includes all reachable state pages,
legal/decision segments, manifest/index nodes, evidence segments, and receipts.
A current-state-only file must not be labeled a self-contained exact save.

### Eligibility

Legal data becomes archive-eligible only when it is an immutable exact version
and every current or future hot continuation has an equivalent bounded hot
projection or compact receipt. `closed` alone is never sufficient. Typical
examples are:

- a superseded source or law version whose current head and effective interval
  summary are hot;
- a closed procedure version after all pending/outbox work is terminal and its
  latest status/provenance projection is hot;
- replaced participation after its effective ballot is frozen;
- a closed case version after all configured appeal/review windows and live
  remedies end, provided any continuing precedent/holding is fully represented
  in the hot current-law projection;
- a repealed rule's old detail after current and retrospective applicability
  indexes retain the needed interval summary;
- terminal decision detail after exact idempotency and legal dependency
  receipts are installed.

Open procedures, scheduled future versions, unresolved conflicts, continuing
precedent not fully materialized hot, live cultural level dependencies, pending
decisions, unacknowledged outbox work, and any payload-required evidence remain
ineligible.

### Deterministic compaction schedule

Every shard persists counters for hot record count, encoded bytes, closed-byte
debt, oldest eligible closure cut, and last maintenance boundary. A maintenance
wake is selected when any declared threshold is crossed or after a configured
number of legal boundaries. Selection is always:

```text
shard key -> record class -> closure time -> version ordinal -> stable ID
```

One step may inspect and encode at most the declared candidate records, source
bytes, dependencies, and manifest nodes. Authoritative work budgets are
deterministic record/byte/node/codec-block counts, never elapsed milliseconds.
If debt remains, the engine schedules the next deterministic wake. There is no
unbounded stop-the-world "periodic compression" pass.

### Prepare, store, commit

Compaction uses a staged fail-closed contract:

1. **Prepare:** read immutable roots directly, validate eligibility at exact
   source record versions, and create `PreparedArchiveSource`. It binds the
   pre-ingress domain, decision, and archive-reachability roots plus every
   affected record version. A prepared checkpoint hash may be retained for
   diagnostics, but is not a post-ingress equality guard. Prepare canonicalizes
   a bounded segment and must not clone/fork the full simulation. No live state
   changes.
2. **Store:** the host stores the immutable segment and new manifest nodes
   idempotently.
3. **Verify:** a kernel API reads every stored object back through the provider,
   enforces compressed/uncompressed predecode limits, verifies bytes and all
   roots, and returns a kernel-produced `ArchiveVerificationReceipt` plus a
   boundary-independent `VerifiedArchiveMutationPlan`. Verification does not
   mutate live state.
4. **Submit:** the host submits that exact opaque maintenance packet through the
   canonical ingress API; callers cannot construct an accepted packet from raw
   IDs or hashes.
5. **Ingress admission:** the normal ingress journal assigns the actual ingress
   identity and boundary cut. This admission legitimately changes journal and
   checkpoint commitments, so they are not compared with the pre-ingress
   checkpoint hash.
6. **Commit boundary:** the kernel-owned archive-maintenance phase performs no
   external I/O. It verifies that the current logical roots and exact affected
   versions still equal `PreparedArchiveSource`, that the admitted packet hash
   and identity bind the verified plan, and that all membership, secondary
   indexes, dependencies, and budgets remain valid. It then derives the final
   boundary receipt and runs the cross-domain maintenance transaction below.
7. **Terminal outcome:** an already terminal token returns its original outcome
   idempotently. A source that became stale only after admission completes the
   boundary successfully with a typed `RejectedStale` maintenance receipt: it
   consumes the ingress and changes only terminal-token/evidence roots, never
   hot legal/decision/archive roots. Malformed or unverifiable packets are
   rejected before admission. An internal late `Err` remains a whole-boundary
   failure and rolls back for retry.

`VerifiedArchiveMutationPlan` is independent of the eventual boundary. It
commits its format, prepared-source commitment, prepared-token hash, exact
removals/additions and old record versions, new archive-index roots excluding
final receipt fields, dependency deltas, sorted
`(ContentId, BlobId, codec, stored_bytes, decoded_bytes)` objects, evidence root,
work counts, and plan hash. It contains no actual ingress ID, committing
boundary ID, final domain/decision/checkpoint root, or final receipt.

The serializable `ArchiveVerificationReceipt` binds the verified plan hash and
object verification results. Its constructor and the accepted maintenance
packet constructor are kernel-private. The public result of verification is a
live opaque `VerifiedArchiveHandle` that implements neither `Serialize` nor
`Deserialize`; callers can only consume it through the submission API. On
submission, the kernel creates a separate private, versioned, serializable
`DurableArchiveIngressEnvelope` containing the plan, receipt, object IDs, and a
deterministic packet ID over the plan and verification-receipt hashes. Only the
kernel ingress loader may deserialize that envelope. A crash before submission
requires verification again rather than accepting caller-reconstructed proof.

At `ArchiveMaintenanceCommit`, the already assigned boundary ID and ingress
reference are appended to produce either committed `ArchiveObjectReceipt`
values or a typed terminal `MaintenanceRejectionReceipt`. A committed
`MaintenanceChangeRecord` computes final domain, decision,
archive-reachability, and terminal-token roots from the verified plan plus
boundary-derived receipts. A rejected-stale change record binds the expected
and actual source commitments, packet/plan hash, ingress, boundary, immutable
object IDs, rejection reason, and unchanged domain/decision/archive roots; only
its terminal-token and evidence roots advance. Neither outcome binds the final
checkpoint hash, avoiding a root/receipt cycle.

Ordinary exact replay structurally validates the recorded plan, verification
receipt, and packet without provider I/O, regenerates the committed or rejected
boundary receipt from the journaled ingress and boundary ID, applies that
outcome, and byte-compares roots and the change record. Archive-complete audit
and historical hydration additionally reload and re-verify provider bytes.
Replay never treats a precommit target root as the already-final authoritative
root.

External storage latency is outside the deterministic simulation boundary.
Only the admitted maintenance packet and its resulting boundary enter
authoritative state and exact replay. No separate maintenance journal is added.
Ordinary exact replay can reproduce current authoritative state from the
recorded kernel-produced verification receipt without performing storage I/O.
Archive-complete audit and historical-query availability require an archive
provider or the embedded exact bundle and revalidate every reachable blob
against those receipts.

### Kernel cross-domain maintenance transaction

Archive commit is not an ordinary `DomainDeltaProposal`. Format 8 adds a
kernel-owned final mutation phase, `ArchiveMaintenanceCommit`, whose private
`KernelMaintenanceTransaction` contains:

- base and transient `PersistentDomainRecordStore` roots for legal hot
  reduction, archive heads, coordinators, and culture dependency records;
- base and transient `DecisionState` roots for terminal payload removal and
  compact request/result receipts;
- base and transient archive-reachability/token-receipt roots;
- the exact maintenance ingress, verified mutation plan, old/new root
  commitments, and bounded change evidence under construction.

The phase contract is:

1. ordinary plugin systems and domain-delta phases see only the old committed
   roots;
2. the kernel maintenance handler rechecks pre-ingress logical source roots and
   affected versions, then builds domain, decision, and reachability roots in
   private overlays and validates all cross-domain dependencies;
3. no plugin or public view can observe either overlay, and no later mutable
   plugin phase runs in that boundary;
4. after every validation and evidence preflight succeeds, the kernel swaps the
   domain root, decision root, archive-reachability root, and terminal-token
   root as one authoritative commit point;
5. any error before boundary completion restores all roots and journal tails
   from the same boundary transaction checkpoint;
6. post-admission source staleness is not an error: the handler discards
   mutation overlays, advances only terminal-token/evidence roots, consumes the
   ingress, and commits a `RejectedStale` receipt as a successful boundary;
7. one `MaintenanceChangeRecord` commits the outcome, affected identities,
   plan hash, actual ingress and boundary identity, old/new roots, decision
   receipt changes, archive objects, and dependency deltas. Exact replay
   regenerates and byte-compares that record.

This is local atomicity inside one simulation boundary, not distributed commit.
It removes the possibility that decision payloads disappear while legal heads
or archive reachability still name the old hot state.

Crash and fork behavior is fixed:

| Point | Recovery |
| --- | --- |
| Prepared but not stored | No state change; release the prepare retention handle. |
| Stored but not verified | Stored objects are unreferenced orphans protected only by the prepare handle; safe GC after handle release. |
| Verified but no ingress submitted | No state change; the verification packet and prepare handle may be retried or abandoned together. |
| Ingress durable but boundary not committed | Normal ingress replay retries the same verified token. |
| Boundary committed but checkpoint not persisted | Replay from the prior durable checkpoint and journals reproduces the maintenance boundary. |
| Duplicate commit ingress | Existing token receipt returns the original outcome without a second mutation. |
| Source changed after ingress admission | Boundary succeeds with a persisted `RejectedStale` terminal receipt; ingress is consumed and no hot reduction occurs. |
| Fork before either branch changes | Both branches may reference the same immutable blob, but each records its own maintenance boundary. |
| Fork after one branch diverges | The old token is stale on the divergent branch. |

Archive-object reachability follows one explicit state machine:

```text
Prepared -> Stored -> Verified -> DurableIngress -> Committed
    |          |          |              +-> RejectedStale
    +----------+----------+-----------------> Abandoned
```

`Prepared`, `Stored`, and `Verified` objects are protected by process-lifetime
prepare handles. `DurableIngress` objects are protected by durable ingress
reachability even after process restart. `Committed` objects are protected by
archive roots and every retained checkpoint/fork. A successfully committed
`RejectedStale` boundary or explicit pre-submission abandonment releases ingress
protection; physical deletion still
waits until all other handles and roots release it. The kernel exposes
`reachable_archive_object_ids()` over retained checkpoints, live forks, prepare
handles, durable maintenance ingress, and committed archive roots. GC must use
that union rather than infer liveness from archive heads alone.

Retention transfer is atomic with respect to host GC:

1. prepare owns a registered lease over exact page/object IDs before storage is
   exposed to sweep;
2. verification acquires a `VerifiedCheckpointHandle` or
   `VerifiedArchiveHandle` over the same IDs before the prepare lease can be
   released;
3. checkpoint persistence installs the durable checkpoint root, or ingress
   submission installs the durable packet/object-ID reachability record, before
   returning an acknowledgement that permits release of the verified handle;
4. after restart, durable checkpoints and ingress are enumerated before the
   first sweep; and
5. reachability enumeration captures one retention-registry epoch. Sweep must
   recheck that epoch and each candidate immediately before deletion, or restart
   marking if a handle/root was added concurrently.

There is therefore no instant in which a successfully transferred object is
unprotected. A crash before durable acknowledgement leaves only an orphan that
was never authoritatively promised; a crash after acknowledgement reconstructs
protection from the durable checkpoint or ingress record.

## Query behavior

### Current queries

Current applicability, enforcement inputs, active rights/duties, voting
eligibility, open procedure status, and current rule heads read only the plan,
order shard, jurisdiction shard, culture-dependency record, and compact
receipts. They must not load cold segments.

For every operative or disputed rule, the hot `CurrentNormativeProjection`
contains the complete bounded data needed by current and scheduled continuation:

- stable rule/object identity and exact operative, disputed, superseding, and
  scheduled version refs;
- full current normative effects: holders, duty bearers, powers, immunities,
  conditions, exceptions, standing, forum, and remedies;
- validity state, ordinary/retrospective effective intervals, and expiry;
- actual publicity proof with event, time, scope, and medium needed by the
  current read cut;
- current conflict partition, priority, displacement, and controlling trace;
- any continuing ruling/precedent holding or material proposition needed for
  present validity, conflict, or interpretation;
- succession/reception mapping used by the current order;
- live culture dependencies and enforcement/implementation pointers;
- exact adoption, evidence, and archived-decision receipts.

Each projection has independent encoded-byte, nested-effect, reference, and
trace ceilings; a small logical-record count cannot hide one unbounded current
projection.

When a historical payload is released, the hot runtime may retain only the
exact identity of an archived dependency still named by a current projection.
This cold-dependency set is rebuilt from current hot references after every
release, so it scales with current law rather than total archived history.
Succession archive follows the same rule: full institutional, liability,
evidence, and archive history becomes cold, while the bounded current reception
mapping needed by no-provider applicability remains hot and is bound to the
committed archive head.

Archive eligibility requires field-for-field equivalence between this hot
materialization and an un-compacted current query. A closed case or old ruling
cannot archive merely because appeal time ended if its holding still controls a
current conflict or precedent; either the complete continuing projection stays
hot or the source version remains hot.

Therefore an enacted women's-suffrage or human-rights rule continues to affect
the game when:

- its originating culture has retired;
- its adoption procedure and old ballots have been archived;
- older amendments have been archived; or
- its supporting decision traces have been reduced to verified receipts.

Only an explicit legal transition changes the current legal projection.

### Historical queries

A historical query first uses the exact-ID/rule/case roots or the canonical
dyadic effective/read-time roots to identify budgeted candidates. It proves
secondary hits against primary membership before loading only the selected
segments through a provider, verifies every header/root and logical reference,
then runs the same applicability/conflict pipeline over the reconstructed read
cut. It never scans segment headers to discover candidates.

Exact-ID paths follow the canonical Patricia bound. Temporal point/range paths
follow the fixed `W` traversal and candidate/intersection bound defined above;
the engine makes no logarithmic fixed-result promise for pathological intervals
that overlap most of history. Those queries fail with the declared typed budget
error before unbounded hydration.

Provider absence or missing/tampered payload returns `ArchiveNotReady`,
`EvidenceUnavailable`, or a new narrowly typed legal-history-unavailable error.
It must not return an empty law set or silently fall back to the current law.

Query budgets cover manifest nodes, segments, decoded bytes, records,
dependencies, jurisdiction traversal, conflict members, and result trace size.
The suite archives each current-query dependency category above independently
and proves no-provider result and trace equivalence.

## Retirement, archive, and deletion

These operations are intentionally distinct:

| Operation | Meaning | Can current legal effect disappear? |
| --- | --- | --- |
| Culture `Retired` | Remove dormant cultural propagation state after dependency checks. | No. Accepted legal commitments survive. |
| Legal repeal/expiry/displacement | A competent legal operation changes the normative head or applicability interval. | Yes, at the legally defined cut. |
| Legal archive | Move closed historical payload out of hot storage while retaining identity, interval summaries, and receipts. | No. |
| Host garbage collection | Delete unreachable physical pages/blobs after all retained roots are considered. | No reachable effect; deleting a reachable blob is corruption. |
| New-run export | Deliberately start a new run/branch with reduced history and provenance. | It may change replay guarantees and must not be labeled continuation of the old exact run. |

The engine never offers "keep only the final result" as an in-place exact-run
mode. Games that intentionally discard history must export a new causal branch
and declare the reduced contract.

## Validation and failure behavior

### Live validation

- validate the plan and directory binding;
- validate touched shards, coordinator, canonical page paths, affected forward
  references, committed reverse dependents, successor closure, and dependency
  deltas;
- preflight all record, byte, page, archive, and participant budgets;
- compare exact kernel versions and logical heads;
- update hash caches only along changed paths;
- restore prior roots, counters, journals, and receipts on failure.

### Cold restore

- verify every reachable state page and tree root before exposing state;
- reject missing, extra, duplicate, or wrongly placed records;
- rebuild and compare the primary domain records, reverse-reference,
  successor-predecessor, and successor-children roots;
- validate all hot shard indexes, archive membership roots, every secondary
  root descriptor, archive-only tombstones, and culture dependency records;
- verify manifest structure and receipts without eagerly hydrating every legal
  segment;
- require a provider and full segment validation for a historical query or
  payload-required continuation;
- reconstruct and compare all declared dependency summaries and verify that
  archive reachability closes over legal, decision, and evidence segments.

A compact checkpoint may restore and serve current-law operation without an
archive provider after all hot pages, current projections, receipts, archive
roots, and locators validate. It defers cold payload verification until a
historical query, payload-required continuation, archive-complete audit, or
self-contained export requests those objects. Provider absence is therefore a
typed capability limitation, not current-state corruption; a missing or invalid
object after a provider claims availability is corruption.

### Tamper and race cases

Conformance must reject at least:

- stale prepared source root or shard version;
- duplicate/overlapping sparse membership, wrong archive sequence, or wrong
  shard;
- changed record ID, ordinal, content commitment, effective interval, or index;
- manifest node omission, cycle, excessive depth, duplicate child, unreachable
  child injection, or secondary leaf not bound to primary membership;
- `ContentId`/`BlobId` substitution, decompression bomb, or decoded-content
  mismatch;
- hot head that cannot be derived from retained plus archived history;
- archived decision receipt with a different request or result;
- an evidence segment made unreachable while a cold legal member still cites
  its embedded receipt;
- a culture retirement that removes a live-level legal dependency;
- a cross-shard coordinator whose participant set differs on retry;
- a compaction commit that partially updates hot, archive, secondary indexes,
  reverse indexes, evidence reachability, or dependency state.

## Budgets

Kernel `RunConfiguration`, the run manifest, and the compiled law plan add
explicit ceilings at their respective ownership layers:

- hot encoded bytes and hot logical records per order/jurisdiction shard;
- shards touched and total bytes changed per boundary;
- cross-shard participants and projection mutations;
- persistent-store collision entries, fixed tree depth, copied nodes, changed
  forward edges, reverse dependents, successor closure, and index mutations;
- dirty pages and emitted page bytes per state-delta seal;
- compaction candidates, compressed and uncompressed bytes, dependencies,
  primary/secondary index nodes, codec blocks, and deterministic work units per
  maintenance step;
- archive temporal prefixes, candidates per dimension, intersection members,
  segments, provider calls, fetched/stored bytes, decoded bytes, and records per
  query;
- terminal decision receipts retained hot;
- archive debt allowed before new history-producing work is throttled.

When archive debt exceeds its hard ceiling, the engine rejects additional
history-producing legal work with a typed capacity error. It must still admit
debt-reducing or liveness-preserving work: repeal, closure, ACK, appeal/review
completion, dependency resolution, archive commit, and recovery. It does not
perform an emergency unbounded compaction pass and does not delete history.

## Complexity targets

Let:

- `D` be total domain records;
- `H_s` be encoded hot bytes in one touched legal shard;
- `K` be changed kernel records;
- `A` be total cold history;
- `W` be the fixed persisted `SimTime` bit width;
- `C_e` and `C_r` be effective/read-time candidates admitted by a historical
  query.

The required asymptotic contracts are:

| Operation | Required bound |
| --- | --- |
| Fork / boundary rollback capture | `O(1)` relative to domain/decision record count, plus unchanged costs of other runtime domains |
| `K` domain-record mutations | `O(K * Patricia structural path + changed payload + affected reference/successor closure)` |
| One-shard legal boundary | `O(H_s + legal_delta + changed tree paths)` |
| Current applicability query | `O(log shards + bounded index candidates + matches)` |
| Historical point applicability query | `O(W + C_e + C_r + sorted intersection + hydrated members)` |
| Historical range applicability query | `O(query dyadic prefixes + admitted candidates + sorted intersection + hydrated members)` |
| State delta persistence | `O(changed pages and manifest paths)` |
| Current-state portable save | `O(total current reachable state)` |
| Self-contained exact portable bundle | `O(all reachable current and historical pages/segments)` |
| One compaction step | `O(configured candidate bytes/work)`, independent of remaining debt |

`A` must not appear in the current-law or one-shard mutation bounds. A
self-contained exact portable bundle, complete forensic validation, explicit
historical query, and host GC reachability walk are intentionally allowed to
scale with their reachable history.

## Frozen 0.8 version matrix

Implementation and fixtures use this complete clean-break matrix:

| Contract | 0.8 value |
| --- | ---: |
| Workspace and public crates | `0.8.0` |
| Run-configuration format/hash domain | `2` |
| Run-manifest format/hash domain | `2` |
| Plugin descriptor/registration format | `1` (new explicit field) |
| Snapshot format | `8` |
| Commitment format | `4` |
| Checkpoint/page/evidence bundle format | `4` |
| State revision format | `3` |
| Admission cursor format | `3` |
| Exact replay revision format | `3` |
| Canonical state-page format | `1` |
| Legal shard/schema format | `2` |
| Compiled legal plan format | `2` |
| Segment content material format | `1` |
| Legal archive object format | `1` |
| Legal archive membership/bitemporal index format | `2` |
| Decision archive object/index format | `1` |
| Decision archive locator/query-page format | `1` |
| Durable archive ingress envelope format | `1` |
| Archive verification receipt format | `1` |
| Archive object receipt format | `1` |
| Maintenance rejection receipt format | `1` |
| Maintenance change record format | `1` |
| Archive reachability root format | `1` |
| Terminal maintenance-token receipt format | `1` |
| Dependency-resolver registry format | `1` |
| Owner-authorized participant proposal format | `1` |
| Cross-plugin retirement receipt format | `1` |

Run-configuration format/hash domain 2 commits every new generic COW,
reference-closure, page, archive, query, retention, and maintenance budget.
Run-manifest format/hash domain 2 commits the dependency-resolver registry,
target-owner routes, provider policy, and all format-8 descriptor registrations.
Plugin descriptors gain an explicit strict `descriptor_format: 1`; their
canonical registration encoding commits owned schemas, target-owner roles,
dependency-resolver roles, semantic hash, and declared format support. A 0.7
descriptor with no field is rejected rather than inferred.

State revision format 3 binds terminal maintenance-ingress count in addition
to the existing authoritative command/attempt/boundary contract. Admission
cursor format 3 binds maintenance ingress to its exact admitted boundary cut.
Exact replay revision 3 requires every archive commit packet, resulting
`ArchiveMaintenanceCommit` phase, committed or `RejectedStale` receipt, and
`MaintenanceChangeRecord`; page-store I/O is not a replay event because it does
not mutate the logical root.

The law plugin changes its descriptor semantic hash and uses new domain
separation tags for schema v2, plan v2, object/version identities, archive
members, primary/secondary roots, and maintenance tokens. Every snapshot,
checkpoint, compact bundle, replay journal, plugin descriptor, law aggregate,
or archive object produced by 0.7 is explicitly rejected by the 0.8 typed and
strict JSON loaders. An application-owned export creates a new run identity; a
declared causal-branch provenance record may point to the old export, but it is
not exact continuation of the old run.

Every row above has its own domain-separation tag, strict JSON fixture, unknown
field rejection, version mismatch rejection, canonical binary fixture, and
tamper vector. The umbrella exact-replay revision does not substitute for a
persisted type's own version.

## One-milestone implementation gates

All gates comprise one public 0.8 release milestone and one persistence-format
switch. Internal gates may merge behind private, dormant, non-loadable
integration points to reduce branch lifetime, but no intermediate public API,
snapshot, journal, or provider format may advertise partial format-8 support.
A failed later gate blocks the 0.8 release; it does not require reverting safe
private groundwork that cannot be observed as the new format.

### Gate 0: baselines and fixtures

- Freeze the existing 1k/10k/100k legal probe and add allocations, cloned bytes,
  serialized bytes, checkpoint bytes, and decision-state size.
- Add deterministic 1M-record cold-history fixtures with 1, 16, 256, and 1,024
  shards.
- Record current snapshot/replay/tamper fixtures before format change.

### Gate 1: kernel COW and commitments

- Build a 1,000,000-key Patricia prototype before freezing format-8 node/page
  encoding. Measure node count, reachable page count, canonical encoded
  structural bytes per entry, resident structural bytes per entry, path-depth
  distribution, mutation allocations, and fork-first-write cost. The measured
  footprint includes primary records, reverse-reference and successor indexes,
  provider metadata, page tables, and allocator overhead rather than the primary
  trie alone.
- Add the private persistent Merkle store and read/overlay interfaces.
- Move domain records and large decision maps to COW roots.
- Add committed reverse-reference and successor indexes, affected-closure
  validation, cold rebuild, and tamper tests.
- Refactor rollback, mutation, views, validation, hashing, fork, and load paths.
- Introduce the frozen format-8 matrix below with strict rejection of every
  format-7 artifact.

### Gate 2: state page deltas

- Add prepare/store/verify page APIs, retention handles, and reachable-page
  enumeration without authoritative state mutation.
- Prove atomic prepare-to-verified-to-durable retention handoff against a
  concurrent mark/sweep epoch and process restart at every handoff point.
- Add checkpoint reconstruction from a page provider and a portable embedded
  page bundle.
- Prove canonical roots across mutation permutations, idempotency, missing-page
  failure, bounded predecode rejection, and exact replay roots.
- Land the kernel cross-domain maintenance transaction, opaque verification
  receipt/ingress, crash matrix, replay evidence, and GC reachability state
  machine before decision or legal payloads are allowed to archive.

### Gate 3: decision archive

- Define terminal eligibility and compact request/result receipts.
- Archive closed decision payloads through canonical maintenance ingress.
- Replace all-history borrowed public APIs with explicit hot iteration,
  locator, owned provider-backed lookup, pagination, and unavailable-history
  errors.
- Prove retry idempotency and legal dependency preservation after archive.
- Benchmark a 1,000,000-key state-page-backed decision locator plus compact
  retry receipts independently from decision payload storage.

### Gate 4: legal shards and legal archive

- Replace the aggregate schema and loaders with plan, directory, order,
  jurisdiction, coordinator, culture-dependency, and archive-head records.
- Refactor settlement and queries to load only declared shards.
- Add deterministic sparse compaction, primary membership plus multi-dimensional
  index roots, historical hydration, and current-law no-provider projections.
- Benchmark primary membership and both 1,000,000-entry sparse dyadic temporal
  indexes independently from legal payload blobs, including overlap-budget
  failure and insert/delete/rebuild memory. Report source legal-version count,
  resulting dyadic-cell/bucket-entry count, expansion distribution and worst
  observed factor, canonical serialized bytes, resident/cache bytes, page and
  provider-call counts, and mutation/query amplification. Gate 4 must freeze
  explicit pass/fail ceilings for those amplification metrics before temporal
  node encoding or the archive wire format may merge.
- Preserve the culture commitment/live-level/evidence contracts across shards
  and archives.
- Add the owner-authorized cross-plugin retirement coordinator and prove that
  culture and law owners mutate only their own schemas in one atomic boundary.

### Gate 5: integration, documentation, and release evidence

- Run workspace format, lint, tests, strict JSON, snapshot, checkpoint, replay,
  fork, archive, GC reachability, crash/fault injection, cross-shard, culture
  retirement, and historical/current query conformance.
- Update architecture, versioning, terminology, English/Chinese public docs,
  examples, semantic hashes, and release notes in the implementation change.
- Publish benchmark artifacts and compare them with Gate 0.

## Acceptance gates

The milestone cannot merge unless all of the following pass.

### Correctness

- Un-compacted and compacted runs produce byte-identical legal query results
  and traces for every available read cut.
- Exact replay reproduces page roots, archive transitions, shard versions,
  legal results, decision receipts, and final checkpoint hash.
- Current human-rights and women's-suffrage effects survive culture retirement,
  legal-history archive, decision archive, snapshot/restore, replay, and fork.
- Every injected gap, overlap, stale token, changed byte, missing provider, and
  partial coordinator mutation fails closed.
- Post-admission source staleness consumes ingress exactly once, commits a
  byte-stable `RejectedStale` terminal receipt, releases only its ingress
  retention, and leaves domain/decision/archive roots unchanged.
- A culture retirement without the target culture owner's opaque mutation, or
  with any owner proposal targeting another owner's schema, is rejected before
  root swap.
- Same-bundle many-predecessor merge succeeds, while a same-bundle successor
  cycle is rejected without changing any root.
- A permanently open low ordinal does not prevent 1,000,000 later closed
  versions from leaving hot storage.
- Every no-provider current query remains byte-identical after independently
  archiving source, procedure, case, ruling, publicity, conflict, succession,
  decision, and culture-provenance history.

### Scale

On the implementation workstation, with 1,000,000 archived legal records,
no more than 512 hot logical records in the dirty shard, and one ordinary dirty
jurisdiction:

- increasing cold history from 100,000 to 1,000,000 changes current-boundary
  p95 by less than 10%;
- current legal-boundary median is at most 10 ms and p95 at most 16.7 ms,
  excluding external archive I/O;
- fork and rollback capture allocate no bytes proportional to total domain or
  decision record count;
- state-delta output contains only newly reachable pages and bounded manifest
  paths, not all current records;
- a 1,000,000-entry non-collision Patricia map has at most `2N - 1` logical
  nodes, no more reachable node-pages than logical nodes, canonical encoded
  primary-page structural overhead at most 256 bytes per entry and measured
  primary resident structural overhead at most 384 bytes per entry for the
  benchmark record shape;
- all four production Patricia indexes together use at most 640 encoded
  structural bytes and 896 estimated resident structural bytes per source
  record in that fixture; HAMT/key-page cardinalities are reported separately,
  while process RSS remains host-specific evidence rather than a wire-format
  guarantee;
- representative 1,000,000-key root-to-leaf depth has p99 at most 64 branch
  nodes, while the format hard limit remains 256 discriminating bits;
- the legal membership and both temporal index families use the full 16-bit
  deterministic bucket space; every page rejects more than 64 entries or 1 MiB,
  and the one-million fixture observed maxima of 37 entries and 37,335 bytes;
- the production-path legal archive probe completes 245 batches while retaining
  at most 4,096 hot compaction candidates and removing superseded index pages
  after each committed root handoff. The one-million fixture retains one
  committed root, one million exact objects, 245 metadata-only terminal
  handles, and zero terminal-handle reachability items;
- the ordered selector is stress-tested at one million candidates while
  examining and materializing exactly 4,096. That synthetic fixture encodes to
  2.49 GB and is intentionally not an admissible live Canwu state under the
  128-MiB legal state/memory ceilings; production-boundary evidence instead
  uses a 16,384-candidate, 38.40-MiB persisted shard, proves its domain-record
  version is unchanged by an unrelated legal ingress, and rejects oversized
  state through the ordinary byte-budget contract;
- a configured compaction step remains within its declared CPU and byte budget;
- one-record mutation, checkpoint restore, reachability enumeration, and one
  compaction step remain within frozen page-count, provider-call, byte, and peak
  memory gates published with the benchmark artifact;
- sparse historical queries grow with fixed-width prefix traversal and admitted
  candidates, not all archived history; pathological overlap trips the typed
  candidate budget without scanning all segment headers;
- unrelated generic domain records and archived decision attempts/traces may
  each grow from 100,000 to 1,000,000 without entering the one-shard legal
  boundary curve;
- bitemporal insert/delete/rebuild permutations produce byte-identical roots,
  and random sparse point queries over 1,000 versus 1,000,000 segments remain
  within the declared fixed-width traversal and candidate envelope;
- maximum-participant cross-shard commit, maximum nested-effect bytes, deepest
  manifests, repeated same-leaf updates, fork-first-write, compaction peak
  memory, and crash/fault cases remain within their declared gates.

The benchmark artifact freezes CPU, memory, OS, compiler, release flags,
warm-up, sample count, cache state, and percentile algorithm. These workstation
numbers are release regression gates, not cross-machine API guarantees.
Complexity, byte budgets, and fail-closed behavior are the portable contract.

### Review

- one independent senior engine reviewer must inspect this design and the
  implementation without participating in the initial authorship;
- the reviewer must report no blocking determinism, ownership, persistence,
  replay, bounded-work, or scale issue;
- any accepted constraint must be incorporated into this document before code
  is treated as conforming.

## Implementation file map

The design intentionally follows current ownership boundaries:

- `crates/runtime/canwu-sim/src/runtime/state.rs`: persistent roots and caches;
- `transactions.rs`: constant-root rollback checkpoints;
- `records.rs`: store reader, overlay validation, and transient mutation builder;
- `hashing.rs`: page and outer commitment roots;
- `persistence.rs`: page delta and archive prepare/store/commit contracts;
- `replay.rs`: maintenance-ingress exact replay and archive-complete validation;
- `validation.rs`, `view.rs`, `decision.rs`, `persistence.rs`, and
  `settlement.rs`: read-interface and checkpoint migration;
- `canwu-decision`: terminal eligibility, COW state, and compact receipts;
- `crates/api/canwu-api/src/lib.rs`: hot-only decision views, archive locators,
  provider-backed owned historical lookup, pagination, and typed availability
  errors;
- `crates/extensions/canwu-law/src/model.rs`: shard, locator, coordinator, and
  archive types;
- `plugin.rs`: schemas, shard routing, loading, and atomic mutation proposals;
- `runtime.rs`: shard-local settlement, compaction eligibility, and queries;
- `tests` and `examples/law_scale.rs`: conformance and regression evidence.

## Rejected alternatives

### Keep only the latest result

Rejected for an exact run. It breaks historical applicability, amendments,
retroactivity, succession, cases, evidence, audit, and replay. The accepted
alternative keeps the latest materialization hot and every authoritative change
in verified cold segments.

### Compress the one aggregate periodically

Rejected. Compression lowers bytes but still requires whole-aggregate decode,
encode, clone, and hash work. It also creates an unbounded pause unless
selection is chunked.

### Add legal shards without kernel COW

Rejected. The current boundary and mutation paths still clone the entire
domain-record map and decision state.

### Add kernel COW without legal shards

Rejected. Updating one `LegalRuntimeRecord` still decodes and encodes the full
legal history and invalidates one large leaf.

### Use time as the authoritative hot shard key

Rejected. Retroactivity, continuing rights/duties, appeals, and succession cross
time partitions constantly. Time is an archive/index dimension. Legal order and
jurisdiction are authority and applicability dimensions.

### Run compaction on a wall-clock worker

Rejected for authoritative mutation. Storage I/O may occur asynchronously, but
candidate selection and commit are persisted deterministic transitions.

### Make archive placement invisible to commitments

Deferred. A storage-transparent logical commitment layer is possible, but it
would add a second semantic-root abstraction while the engine is pre-1.0. The
first implementation records compaction explicitly, making replay and tamper
reasoning simpler.

## Open implementation choices allowed by this design

These choices may change without altering the architecture, but must be frozen
in the format-8 specification before code merges:

- exact canonical page and archive wire encoding within the frozen format IDs;
- legal/decision archive compression codec and fixed settings used to derive
  `BlobId`;
- archive manifest fan-out;
- deterministic maintenance record/byte/node/codec-block limits and separate
  workstation benchmark time gates;
- the final public names for page provider/store and legal-history-unavailable
  errors.

They are implementation constants and adapter vocabulary, not permission to
change the invariants, ownership, clean-break policy, or acceptance gates.

## Independent engine review

### Draft 1

The independent senior engine reviewer scored draft 1 **5.2/10** and rejected
implementation. The blocking findings were:

- scalar archive cuts could not represent out-of-order closure;
- one content-bound `LegalRecordRef` could not represent both mutable objects
  and immutable versions;
- delta validation lacked committed reverse-reference and successor indexes;
- threshold-shaped COW trees could produce history-dependent roots;
- authoritative archive transitions were absent from exact replay;
- cold legal evidence receipts were not connected to reachability and GC;
- no-provider current queries lacked a complete hot materialization contract;
- culture retirement lacked a target-keyed cross-shard dependency record;
- one manifest ordering could not support the promised multi-dimensional
  logarithmic queries;
- semantic page IDs conflicted with variable compressed bytes; and
- the format-8 version matrix was incomplete.

### Revision 2 response

Revision 2 resolves those findings by adding sparse exact membership and
multi-root indexes, separate object/version/head references, immutable versions
for every authoritative mutable-object transition, committed reverse/successor
indexes, a fixed-depth canonical sparse trie, canonical maintenance ingress,
an explicit crash/fork matrix, embedded cold evidence receipts and complete
portable reachability, full current normative projections, target-keyed culture
dependency records, separate `ContentId`/`BlobId`, canonical state page bytes,
and the frozen 0.8 version matrix.

### Revision 2 re-review

The reviewer confirmed hash `070612fee3c2efe693014bf78bd4815e60144130`,
scored revision 2 **8.1/10**, and found four remaining blockers:

- successor indexes reversed the existing many-predecessor merge cardinality;
- a physical 64-node path per key was canonical but not viable at one million
  records;
- segment content/blob IDs and a future commit boundary were self-referential;
- decision and domain archive changes lacked a defined cross-domain atomic
  transaction.

### Revision 3 response

Revision 3 corrects successor ownership to
`successor_of[predecessor] -> successor` and
`predecessors_of[successor] -> predecessors`; replaces the fixed physical path
with a canonical binary Patricia trie bounded by `2N - 1` logical nodes; defines
acyclic `SegmentContentMaterial -> ContentId -> StoredBlob/BlobId -> committed
ArchiveObjectReceipt` layers; and adds a kernel-owned final
`ArchiveMaintenanceCommit` phase that builds transient domain, decision, and
archive-reachability roots, then swaps them at one commit point with one replay
evidence record.

### Revision 3 re-review

A fresh reviewer confirmed hash
`b7dd417ddfbebbc8f36801ba83b78eb6217c7eba`, scored revision 3 **7.3/10**,
and found five remaining blockers:

- the verification packet mixed pre-ingress checkpoint roots with final roots
  that cannot exist until the committing boundary;
- public borrowed `DecisionState`, attempt, and trace APIs promised complete hot
  history even after decision archival;
- culture retirement still lacked an owner-authorized atomic protocol because
  one plugin cannot mutate another plugin's record schema;
- the bitemporal archive index named roots but did not specify a canonical,
  bounded interval algorithm; and
- newly persisted maintenance records were absent from the frozen version
  matrix.

### Revision 4 response

Revision 4 separates `PreparedArchiveSource`, a boundary-independent
`VerifiedArchiveMutationPlan`, actual ingress admission, and boundary-derived
receipts/final roots; replaces complete-history decision borrows with explicit
hot, locator, and provider-backed owned APIs; adds a generic kernel
owner-authorized retirement coordinator with manifest-frozen participants;
defines dual canonical sparse dyadic bitemporal tries and honest
candidate-budget complexity; enumerates archive reachability states and GC
roots; and versions every persisted maintenance, locator, registry, index, and
receipt format independently.

### Revision 4 re-review

The reviewer confirmed hash `07b83358296fce1b09ab9b1f75b67117e62240d6`
and 1,435 lines, scored revision 4 **7.8/10**, and found three remaining
blockers:

- a post-admission stale source was described both as an `Err` that rolls back
  the boundary and as a persisted terminal rejection that consumes ingress;
- the complete format matrix omitted changed run-configuration, run-manifest,
  and plugin-descriptor contracts; and
- dependency resolvers were mandatory, but the target culture schema owner was
  not explicitly required to propose the culture-owned retirement mutation.

### Revision 5 response

Revision 5 makes post-admission staleness a successful boundary with a typed
terminal `RejectedStale` receipt while reserving `Err` for rollback/retry;
versions run configuration, run manifest, descriptor registration, and the
rejection receipt; and makes the target schema owner a mandatory initiating
participant whose opaque proposal supplies the exact culture mutation. It also
freezes temporal-node validation, adds a state-page-backed exact decision
locator, distinguishes structural replay from provider audit, specifies atomic
retention handoff under concurrent GC, and adds separate temporal/decision-index
benchmarks.

### Revision 5 independent acceptance

The independent reviewer confirmed hash
`6c853015ea0218735c9cc4d34dc253861e18b2c4` and 1,557 lines, reported no
blocking finding, scored the design **9.0/10**, and approved it for
implementation. After the five nonblocking hardening recommendations were
incorporated, the same reviewer confirmed hash
`0686c206972def81c107c948cb612a8da710aeef` and 1,598 lines, again found no
blocker, scored it **9.4/10**, and approved implementation. The final three
clarifications above record provider descriptors rather than instances,
restrict prefix-completeness arithmetic to engine-issued IDs, and require
numeric temporal-amplification gates before wire-format merge.

### 0.8 implementation acceptance

On 2026-08-30 an independent senior engine developer reviewed the completed
single-milestone implementation and its scale, restart, authentication,
retention, and finalization evidence. The review's blocking findings were
resolved in the implementation-hardening and final independent-review closures
recorded above. The last re-review found no remaining blocker and returned
**ACCEPT**. The architecture and implementation are independently accepted;
future changes remain subject to all gates and review requirements in this
document.
