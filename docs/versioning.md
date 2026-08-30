# Versioning and Persistence

Canwu is pre-1.0. Format 8 is a deliberate clean break: the 0.8 runtime
writes and reads only its current contracts. There is no implicit loader or
runtime migration for format 2 through 6 data. Applications that need old
records must keep the old engine or run an explicit, application-owned export
outside the Canwu runtime.

## Current contract

The workspace version is `0.8.0`. A live `SimulationSnapshot` has:

- snapshot format `8`;
- commitment format `4`;
- checkpoint/evidence-journal format `4`;
- state revision format `3`;
- admission cursor format `3`;
- exact replay revision format `3`;
- a declared `RunManifest`, declared run configuration, and canonical
  `initial_scenario`;
- a non-zero `authority_root_seed`, derived independently from the simulation
  random streams.

Typed loading and strict JSON loading reject any other engine or contract
version. Strict JSON loading also rejects unknown fields at every nested
object and rejects a wire value whose canonical re-encoding changes shape.

Format 8 keeps the exact decision-ingress commitment and adds explicit plugin
descriptor versioning, typed decision-history locations, canonical domain-record
primary/reverse/successor commitment roots, and content-addressed state-page
prepare/store/verify contracts. Format-7 snapshots, replay journals,
checkpoints, evidence segments, and plugin descriptors therefore require the
0.7 engine or an explicit application-owned export; the 0.8 runtime rejects
them before constructing mutable state.

The runtime no longer contains `migration.rs` or `legacy_v4.rs`. An older
snapshot must not be silently relabeled as format 8, and a legacy replay
journal cannot be promoted to exact replay.

Random streams persist their reduction algorithm. Existing
`SplitMix64V1` streams retain their historical modulo reduction, while new
streams use `SplitMix64V2`, which applies unbiased rejection sampling. This
keeps old format-7 journals replayable while making the current behavior
explicit in the persisted state.

## Format 8 state pages and decision history

`StatePageBlob` is the canonical uncompressed page envelope. Its `page_id` is
the domain-separated BLAKE3 hash of the bytes; hosts may store it in any
backend, but `StatePageProvider` must return the exact bytes or the runtime
returns `state_page_unavailable`. `prepare_state_delta` and
`verify_state_delta` are deterministic and idempotent: they do not mutate the
authoritative simulation revision or replay journal.

The paged-checkpoint envelope format is `4`. It stores a compact
`SimulationCheckpoint`, not an unaudited state-only snapshot. Restoring a
checkpoint whose journal cut is non-empty requires the exact contiguous
evidence prefix through `from_paged_checkpoint_and_journal`; the empty-journal
shorthand is valid only for a zero cut.

Decision history is queried by `DecisionHistoryKey` (`Ticket`, `Attempt`, or
`Trace`) and reports `Hot`, `Archived`, `Unresolved`, or `Absent` through
`DecisionHistoryLocation`. Terminal tickets, attempts, and traces can move to
verified archive blobs through canonical maintenance ingress. The compact
checkpoint keeps a bounded hot decision page plus a two-level locator
directory: 4,096 stable hash-prefix buckets, each split into 16 deterministic
subsegments. A segment is capped at 64 entries and 1 MiB; unchanged pages are
reused by content address. The locator page table is itself chunked into
authenticated directory pages of at most 1,024 page references, so the root
manifest remains below the 4 MiB state-page limit at one million keys. An exact
lookup loads exactly one locator segment from a provider. The hot-history root is maintained incrementally from count, XOR, and
modular-sum digests and is rebuilt and verified on restore. Missing pages return
`DecisionHistoryUnavailable`, never `Absent` or an empty success. Verification
produces a provider-authenticated full replacement for every touched locator
page, bound to its previous page ID. Boundary commit therefore needs no provider
read, is replay-safe, and can append into an existing segment after a root-only
restart without discarding earlier receipts.
Before hot-state validation, paged restore derives the exact archived ticket or
trace references still named by hot traces, resolved tickets, and accepted
attempts, then authenticates only those locator pages against the committed
directory. Partially archived dependency graphs therefore restart correctly
without loading unrelated history.
Restore rejects noncanonical locator bucket/segment placement and any archived
locator or terminal payload duplicated in hot metadata. Repeat delta
preparation reads only directory pages for changed locator segments rather than
walking all 65,536 possible pages.

Offline GC starts from `ArchiveReachabilityManifest`. The kernel contributes
retained state pages, evidence segments, decision locator pages, pending
maintenance locators, and decision blobs. Every frozen plugin archive
participant extends the same manifest automatically; callers do not append law
marks manually. The law plugin contributes committed or leased content, blob,
membership, effective-time, and recorded-time page IDs. The retention ledger
derives and verifies the exact transitive page closure from the provider,
persists `Prepared`, `Verified`, `DurableIngress`, and terminal root handles
across restart, interlocks each ownership handoff with the active GC epoch, and
keeps a pending handle as a new-object delta plus the proposed current-page
closure while the previous committed root protects older data. Commit moves
the prior object closure into the new root, retires the superseded root, and
clears the terminal handle payload. Root-only restore and overlapping-root
tests prove that shared pages and plugin marks remain protected after restart. Internal
plugin ingress may also carry authenticated archive-retention roots. A pending
verified legal transition carries one directory root rather than a
history-sized list. The law reachability participant authenticates and expands
that root into blob, membership, and temporal objects before mark/sweep.
Snapshot/restart preserves the root; applying or terminally rejecting a stale
item transfers or releases it deterministically.
The legal commit itself binds the exact directory root, object count, shard,
token, source root, and expected record versions; neither a caller-provided
summary nor a different authenticated tree can be substituted.
The synchronous helper authenticates the store retention handle and complete
root closure before mutation, applies to a detached clone, finalizes retention,
and only then swaps the live runtime. Canonical ingress records terminal
outcomes by retention handle until the host finalizes the store from the
authoritative reloaded runtime and a private acknowledgement retires that
recovery metadata. Multiple archive commits in one boundary therefore remain
independently recoverable after restart.

The legal archive object remains format `1`; its authenticated membership and
bitemporal index is format `2`. Format `2` uses the full 16-bit deterministic
bucket space and rejects any membership or temporal page above 64 entries or
1 MiB before storage or root admission. A dense effective-time or recorded-time
cell is represented by an ordered vector of authenticated page segments, so 65
or more versions in one cell remain exact instead of overflowing one page.
Provider-backed temporal queries stream those segments and enforce candidate,
intersection, provider-call, segment, and decoded-byte budgets during I/O;
one query-wide meter covers directory, membership, temporal, and blob reads
across every shard. Pathological overlap fails before an unbudgeted next
segment is read.

Generic domain records use four canonical Patricia roots: primary records,
reverse references, successor edges, and predecessor edges. Forks and rollback
captures share these roots. Incremental checkpoints stop at provider-known
subtree pages, so changing one record emits only its missing Patricia paths and
the bounded manifest path rather than reserializing every record. The paged
checkpoint builder constructs its compact checkpoint body without first
cloning the full domain-record and decision payloads. Boundary proposal
validation likewise filters empty stages first, overlays only changed records
onto a structurally shared root, and validates the affected reference closure.

Plugin descriptors also persist owner-authorized maintenance dependency
resolver declarations. A cross-plugin retirement commit must include the exact
target schema owner and every declared dependent owner. Each proposal is bound
to the plugin semantic hash and may mutate only schemas owned by that plugin;
the complete proposal set is validated against one source domain root and
applied atomically through replay-visible maintenance ingress.
Plugin-owned maintenance packets use registrar-issued opaque internal-ingress
permits; ordinary host-authored plugin ingress cannot forge a verified archive
commit. Applied and post-admission stale maintenance both produce chained
terminal records. A stale item is a successful terminal no-op, not a boundary
rollback that can retry forever. Plugin descriptors persist the exact internal
ingress names so this authority and pending-retention ownership survive restart.

## Self-contained exact replay

`ReplayJournal` is the complete replay boundary. It carries the canonical
initial scenario, declared run identity, run configuration, plugin descriptors,
authority root, commands, attempts, ingress, boundaries, random draws, and
final revision/checkpoint commitments. `replay_from_journal(plugins, journal)`
does not accept a second scenario supplied by the caller; the scenario in the
journal is authoritative and is validated against the manifest.
`replay_from_journal_json` is the strict JSON counterpart and rejects unknown
fields recursively before replay begins.

Executable policy implementations are not replay inputs. Decisions, outcomes,
and evidence already admitted to the journal are replayed as records.

## Durable outbox

Boundary emissions are exposed through `Canwu::outbox_entries()`. Each entry
has a stable `delivery_id` derived from the run manifest, boundary, event, and
emission index. The host application must deliver entries at least once and
deduplicate by `delivery_id`. Exact replay regenerates the same outbox
identity; it does not re-send external effects.

In compact mode, `CompactedCanwu::outbox_entries()` returns entries from the
retained evidence tail. Once evidence is sealed, the caller owns the returned
`EvidenceJournalSegment` and must keep its boundary emissions with the host's
delivery/acknowledgement state; compaction does not acknowledge or deliver an
external effect on the host's behalf.

## Granularity boundary

The engine exposes the domain-neutral `SimulationGranularity` enum:

| Value | Meaning |
| --- | --- |
| `aggregate` | A coarse aggregate or population-scale state. |
| `group` | A bounded social, institutional, military, or organizational group. |
| `actor` | A person or other principal with its own knowledge and authority. |

These are engine simulation levels, not a fixed historical ontology. A host
game may map them to its own terms. In Celestial Mandate, for example,
`aggregate` can map to Population, `group` to Special Group, and `actor` to
Character. That mapping belongs in a CM reference integration or host adapter,
not in Canwu core.

## Reference integrations

Southern Ming and WWII are content and ruleset integrations for Celestial
Mandate. They are not Canwu adapters and are intentionally absent from this
repository. Canwu provides the generic state, authority, boundary, evidence,
granularity, persistence, replay, and outbox contracts that those integrations
consume.

## Source compatibility

The 0.8 change removes caller-supplied replay wrappers from the public facade
and adds schema-declared identity-only evidence dependencies plus compact,
Merkle-bound plugin-ingress provenance.
Use the constructor APIs for new runs, `snapshot`/`checkpoint` for persistence,
`replay_from_journal` for exact replay, and `outbox_entries` for host delivery.
Downstream crates must update their code to these contracts; no deprecated
alias is retained before 1.0.
