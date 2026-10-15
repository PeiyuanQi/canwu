# Production Archive Volume and Lookup

Status: implemented in 0.16.0. It changes `canwu-production`: its terminal
archive, its restore validation, its output acknowledgement, its process
validation, the settlement of its public packets, and the admission of
`StartExecution` and of production completion leases. `canwu-resource` gains public lookup-index functions,
`MAX_PRODUCTION_OUTPUT_BATCH_CREDITS`, and
`ResourceState::production_output_batch_can_fit`, and keeps its own formats
and behavior.

The 0.15.0 design this builds on is
[resource archive lookup and volume](resource-archive-lookup-and-volume.md).
The production lifecycle is described in the
[production economy mechanism](production-economy-mechanism.md).

## Background

`canwu-production` keeps its terminal records in its one domain record: each
settled, cancelled, or failed execution with its work order, work in
progress, operation outcomes, completion acquisition, grants, and lease
receipts, and each completed facility project. The host application moves
them to a content-addressed archive with `prepare_production_archive`,
`PreparedProductionArchiveBatchV1::store_and_verify`,
`enqueue_production_archive`, and `finalize_production_archive_retention`.
Each archive commit is a directory object that names its blobs, its
membership and temporal pages, and the commit before it. The production root
holds the newest directory, the archive head, the retention handles of
settled commits, and one maintenance receipt per settled commit.

Production also reads the resource archive. An active execution or facility
project binds exact resource consumptions and consumption outcomes, which the
resource plugin may already have archived. Restore validation resolves them
from the resource archive, and `enqueue_production_resource_continuation`
records a witness that lets a later boundary advance a project whose inputs
were archived.

## Archive lifetime, volume, and lookup

### Problem

Four faults bounded or stopped a long run.

- **Lifetime.** Every settled archive commit, applied or stale, left a
  maintenance receipt in the production root, and receipts never left it.
  `prepare_production_archive` refused once the root held
  `max_operation_outcomes` (8,192) of them, and the commit that settled beyond
  that failed its boundary. A run could therefore make at most 8,192 archive
  commits: about seven and a half years at three commits a day. Before that,
  every receipt was decoded, validated, re-encoded, and journaled by every
  boundary that changed the production root.
- **Volume.** One commit carried one page of at most 512 records.
- **Lookup.** The archive had no index. Nothing could find an archived
  production record but a walk of the directory chain, and restore validation
  found each compacted resource input by walking the resource archive chain
  and authenticating every commit on it, so its cost grew with the resource
  archive for each input. A continuation witness ran the whole resource
  archive audit once per witness, at restore and when it was queued.
- **Termination.** Several ways of settling an archive commit failed a
  boundary that could not then be stepped past:
  - a stale commit added a retention handle, and the root refused more than
    64 handles, so a stale commit against a full handle budget failed;
  - the same batch queued twice shares one retention handle, and its stale
    settlement overwrote the applied commit's handle, so the host application
    finalized an applied commit as stale;
  - an acknowledgement that arrived twice found no handle and failed;
  - and a commit admitted behind any other production packet in the same
    boundary went stale, because every packet advances the runtime revision
    that the source root binds. That did not fail the boundary, but in a busy
    run it could keep every commit from applying.

  The archive reachability participant also marked only the directory of a
  queued commit, which the kernel marks, and not its blobs and pages, so an
  offline collection between queueing and settlement could delete objects that
  the commit's directory names once it becomes the head.

### Decision

**A cumulative lookup index.** Each commit directory names the root of an
index over every terminal record archived up to and including that commit,
in the new required field `ProductionArchiveIndexDirectoryV1::lookup_root`.
The index has exactly the format of the resource archive's
(`ResourceArchiveLookupPageV1`, a 16-way trie over key digests with leaves of
at most eight entries) and is stored under its own namespace,
`PRODUCTION_ARCHIVE_LOOKUP_PAGE_NAMESPACE`. A key's digest is
`production_archive_lookup_key_digest`, a BLAKE3 hash of the
`ProductionTerminalArchiveKeyV1` under its own domain.

The production crate does not copy the index code. `canwu-resource` makes its
index functions public and lets them take key digests instead of resource
keys: `prepare_resource_archive_lookup_entries`,
`find_resource_archive_lookup_entry`, `verify_resource_archive_lookup_entries`,
`collect_resource_archive_lookup_pages`, and `audit_resource_archive_lookup_index`
with `ResourceArchiveAuditEntry`. Production passes them a read-only view of
its store that serves the lookup-page namespace from the production one and
refuses every other namespace. Nothing else in the resource crate changes.

- `archived_production_terminal_record(state, store, key)` reads the head
  directory, one lookup page per index level, and on a hit the one blob that
  holds the record, each authenticated, and returns the record
  (`ProductionArchivedRecordV1`). A key the index does not hold is a miss; a
  missing, forged, oversized, or malformed object is an error, never a miss.
- `prepare_production_archive(candidate_limit, store)` takes the store. It
  reads the head directory and the lookup pages on the paths of the new keys
  and prepares their replacements; `store_and_verify` stores them and checks
  that each record of the commit resolves through the stored index.
- `validate_production_archive`, the restore audit, rebuilds the head's index
  from the newest entry of every key in the chain and requires the store to
  hold exactly those pages and the head to name their root.
- `live_production_archive_lookup_pages` recomputes the pages a live run
  reads, and a batch's `superseded_lookup_pages` names the pages its index no
  longer reaches, under the retention rule of the resource archive.

**Commits of several pages.** One commit carries up to
`MAX_ARCHIVE_PREPARE_CANDIDATES` (1,024) records as up to
`MAX_ARCHIVE_COMMIT_PAGES` (16) blobs of up to `MAX_ARCHIVE_PAGE_ENTRIES`
(64) records, each with its own membership and temporal page. The batch keeps
its first page in `blob`, `membership_page`, and `temporal_page` and the rest
in `additional_pages`. A blob's size is what a lookup hit reads: a settled
execution's record encodes to about 13 KB, so a full blob of 64 is about
0.8 MB, where one of 512 would be 6.6 MB. One commit of 1,024 such records
writes about 14 MB of records and pages and, into an index of a million
records, about 3,300 lookup pages of about 4 MB, so the batch budgets rise to
8,192 objects (`MAX_ARCHIVE_OBJECTS_PER_BATCH`, which now counts lookup pages)
and 64 MiB (`MAX_ARCHIVE_BYTES_PER_BATCH`); a batch that exceeds either is
refused by `store_and_verify` before anything is written.

**Receipts that leave the root.** The maintenance receipt of an applied commit
is a terminal archive candidate under the new key
`ProductionTerminalArchiveKeyV1::ArchiveMaintenanceReceipt`, and a commit
selects it before any other candidate. The next applied commit therefore
archives it, so the root holds at most one: the receipt of the commit at the
archive head. A blob carries archived receipts in its new `receipts` list.
The receipt of a stale commit is not a candidate. It names its commit's
retention handle (`retention_handle_id`), and the acknowledgement of that
handle removes it. Receipt identities come from a counter,
`ProductionArchiveHeadStateV1::receipt_sequence`, so they stay unique after
receipts leave. The directory and the head count archived receipts
(`archived_receipt_count`).

**Stale and duplicate commits terminate.** The boundary that settles a commit
reads no archive object. It applies the commit when the production root's
source root still is the one the batch was prepared from, and otherwise
records it as stale; neither can fail for capacity. `enqueue_production_archive`
requires the batch to equal what `prepare_production_archive` derives from the
current root and the store, as `enqueue_resource_archive` does. Before an
applied commit removes anything, the boundary rebuilds its blobs, pages, and
directory from the root's records and the lookup root the commit carries
(`VerifiedProductionArchiveCommitV1::lookup_root`) and requires them to be the
directory and retention objects the commit names; only the lookup index itself
is trusted to the enqueue check and the restore audit. The stale settlement of a batch
queued twice keeps the applied commit's handle. An acknowledgement retires its
handle and every stale receipt recorded under it, and one that finds no handle
settles as a no-op. The root bounds the handles of applied commits by
`max_pending_retention_handles` (64) and no longer counts the handles of
stale commits, which the host application's acknowledgements bound.
`prepare_production_archive` still refuses as `archive_backpressure` while 64
handles of either kind wait for acknowledgement.

**Archive commits settle first.** Within one boundary the production plugin
settles archive commit packets before its other packets, each group in its
admitted order. A commit prepared from the root the boundary started from
applies even when an operation or an acknowledgement was admitted ahead of
it. A boundary that admits no archive commit keeps its order.

**Restore reads the resource index.** Restore validation resolves each
compacted resource consumption and outcome with
`archived_resource_terminal_record`: the head directory, one lookup page per
index level, and one blob. The production `*_with_archives` wrappers run the
resource archive audit (`validate_resource_archive_store`) once whenever the
resource root has an archive head, as the resource wrappers do, so no lookup
trusts an index the store does not hold; continuation witnesses are then
checked by one walk of the audited chain for all of them.
`enqueue_production_resource_continuation` authenticates the head commit only,
because each compacted input it checks then resolves through the head's
index, and a hit authenticates the blob and record it reads.

**Restore does not depend on a seal.** Every hot execution and facility
project, terminal ones included, declares in the payload-required evidence
continuation the record versions restore validation reads for it (provider
evidence, technology records, realization evidence, the consumers of its
inputs) until the archive removes it; 0.15.0 declared them only while the work
was running, so a seal could drop a body that the restore of a settled but
unarchived execution then read. The pinned production source of a settled
output is no longer read back, at the output acknowledgement or at restore:
the acknowledgement must name exactly the source the dispatch pinned
(`execution.output_source == Some(production_source)`, where 0.15.0 also let an
unset source through), and the exact resource outcome cites it.

**Idle archives.** The head commit's receipt is always a candidate, so once
an archive has a head, `prepare_production_archive` succeeds even when no
execution or project is terminal, and such a batch archives only that
receipt. A host application commits when `archive_due_index` or
`project_archive_due_index` holds records.

**Reachability.** The archive reachability participant marks, besides the
directories, blobs, and pages of the chain and of each unacknowledged handle,
the directories of queued commits that the kernel lists and the lookup pages
reachable from all of them. The index of the head and of each queued commit
must be whole; any other is marked as far as the store still holds it.

Rejected alternatives:

- *Retire every receipt, applied ones included, at its acknowledgement.* It
  needs no new key or blob field, but the archive would then keep no record of
  its own commits, and the restore audit could not check that each commit
  archived the receipt of the one before it, which ties the chain to the
  receipts the root recorded.
- *Copy the index code into the production crate.* Two copies of one format
  drift apart. The resource functions only needed to take key digests.
- *Re-derive the batch in the boundary, reading the store.* A store that
  cannot serve an object would then fail the boundary, and the check repeats
  the one `enqueue_production_archive` makes against the same root.
- *Keep 512 records per page and allow several pages.* A commit of 16 such
  pages would be about 106 MB, and a lookup hit would read a 6.6 MB blob.
- *Give production boundaries an archive lookup for re-sent operations.* A
  re-sent production operation whose outcome was archived is not recognized
  as a duplicate, in 0.15.0 and now. The index makes such a check affordable,
  but it would add an archive read, and a failure mode, to every production
  operation; it is left open below.

### Invariants

- A lookup returns the newest archived record of a key or nothing. A miss is
  decided by authenticated pages only, and a lookup reads at most one
  directory, `MAX_RESOURCE_ARCHIVE_LOOKUP_DEPTH + 1` lookup pages, and one
  blob, whatever the store returns.
- A commit of several pages is one directory and one atomic archive commit:
  all of its records leave the root in one boundary, or none do.
- The root holds at most one receipt of an applied commit, and it names the
  archive head. Every receipt of a stale commit names a retention handle the
  root still holds. The archived receipts and the hot applied one together
  number the applied commits, and every receipt id lies within
  `receipt_sequence`. `ProductionState::validate` checks all of these.
- An archived receipt names the directory before the commit that archived it,
  and a commit archives at most one; `authenticate_production_archive_directory`
  checks both. The restore audit also requires the first commit to archive no
  receipt and every later one to archive exactly one.
- The root holds at most 64 retention handles of applied commits: a commit
  applies only against the root it was prepared from, and preparation refuses
  while 64 handles of any kind are held.
- Settling an archive commit or an acknowledgement reads nothing from a store,
  so a live run, a restored run, and an exact replay settle it the same way,
  and a sealed run and its replay agree. No boundary decision depends on
  retained evidence.
- Restore validation of a compacted resource input reads one index path and
  one blob of the resource archive, not its chain.

### Persistence, replay, and formats

- **Plugin state.** The production root changes shape.
  `ProductionArchiveHeadStateV1` gains the required fields
  `archived_receipt_count` and `receipt_sequence`, and
  `ProductionArchiveMaintenanceReceiptV1` gains `retention_handle_id`,
  `archived_receipts`, and `settled_at`. `maintenance_receipts` holds at most
  one receipt of an applied commit and the receipts of unacknowledged stale
  commits. Its payload-required evidence continuation now lists the versions
  of every hot execution and project, not only running ones, except the
  inputs' consumer evidence of an ended execution or project, which restore
  compares by reference only. The production
  root has no format version of its own; as before, the plugin semantic hash
  and the engine version identify it.
- **Archive objects.** `ProductionArchiveIndexDirectoryV1` gains the required
  fields `archived_receipt_count` and `lookup_root`, so directory ids differ
  from 0.15.0 and a 0.15.0 directory does not decode.
  `ProductionArchiveBlobV1` gains the required field `receipts`, and its
  `format_version` becomes 2 (`PRODUCTION_ARCHIVE_BLOB_FORMAT_VERSION`); a
  blob of any other version is refused. An archived settled execution
  carries its `output_settled_at`. Membership and temporal pages keep
  their shape and may name the new key kind. Lookup pages are a new object
  kind under `canwu.production.archive.lookup-page`.
- **Ingress.** `VerifiedProductionArchiveCommitV1`, the payload of the archive
  commit packet, gains the required fields `selected_receipts` and
  `lookup_root`. The
  acknowledgement packet and the retention binding of the commit packet are
  unchanged.
- **Plugin identity.** `PRODUCTION_SEMANTIC_HASH` changes. Registered
  commands, ingress, boundary systems, and schemas are unchanged; the apply
  system declares one new event (see
  [output that settles after its completion](#output-that-settles-after-its-completion)).
- **Kernel formats.** Snapshot, commitment, checkpoint, evidence-journal, and
  paged-checkpoint formats and the hash algorithms are unchanged.
- **Retention.** A restore, an exact replay, or a fork reads the index of
  every production archive head it passes through, so the store keeps the
  lookup pages those heads reach, exactly as for the resource archive. A
  snapshot taken while a stale commit is unacknowledged needs that commit's
  objects to restore.

### Compatibility

0.16.0 is a pre-1.0 minor release. Source and behavior changes:

- `ProductionState::prepare_production_archive` takes the archive store as a
  second argument. `()` implements `ProductionArchiveStore` as the absent
  store, enough to prepare a root's first commit.
- `MAX_ARCHIVE_PAGE_ENTRIES` falls from 512 to 64,
  `MAX_ARCHIVE_PREPARE_CANDIDATES` from 2,048 to 1,024 (a commit can now
  carry that many; in 0.15.0 more than 512 were refused),
  `MAX_ARCHIVE_OBJECTS_PER_BATCH` rises from 1,024 to 8,192, and
  `MAX_ARCHIVE_BYTES_PER_BATCH` from 16 MiB to 64 MiB. `MAX_ARCHIVE_COMMIT_PAGES`
  is new.
- `PreparedProductionArchiveBatchV1` gains `selected_receipts`,
  `additional_pages`, `lookup_pages`, and `superseded_lookup_pages`, and the
  methods `validate` and `selected_count`; `VerifiedProductionArchiveCommitV1`
  gains `selected_receipts`, `lookup_root`, `validate`, and `selected_count`. `PreparedProductionArchivePageV1`,
  `ProductionArchivedRecordV1`, `PRODUCTION_ARCHIVE_LOOKUP_PAGE_NAMESPACE`,
  `PRODUCTION_ARCHIVE_BLOB_FORMAT_VERSION`,
  `production_archive_lookup_key_digest`, `archived_production_terminal_record`,
  and `live_production_archive_lookup_pages` are new.
- `enqueue_production_archive` refuses a batch that differs from what the
  current root and the store derive, where 0.15.0 accepted it and let the
  boundary refuse it.
- A commit selects the receipt of the head commit first, so a commit with a
  candidate budget of one after an applied commit archives only that receipt.
- `acknowledge_production_archive_retention` is a no-op for a handle the root
  does not hold, where it failed.
- `acknowledge_output` requires the execution's pinned `output_source` to be
  set and equal to the acknowledgement's source; a state-level caller pins it
  first, as the output dispatch does.
- The production restore wrappers with archives audit the resource archive
  whenever it has a head.
- Within one boundary, archive commits settle before the plugin's other
  packets.

Canwu crates depend on the exact same version of each other, and a 0.16.0
run never reads what a 0.15.0 run wrote, so there is no migration and no
decoding of 0.15.0 production state or archive objects. The exact
engine-version check refuses every 0.15.0 snapshot, checkpoint, and journal,
and that suffices for them. A production root or archive object encoded by
0.15.0 is refused rather than misread: the root, its receipts, the directory,
and the commit payload lack required fields and do not decode, and a blob has
format version 1. A scenario built by 0.15.0 code is rebuilt with
`ProductionState::into_initial_record`. Exact replay and restore within
0.16.0 are covered by the fixtures.

### Measurements

Release build, one thread, an in-memory store that copies the bytes of every
object it serves, on a machine that other builds were sharing. "After" is the
`full` preset of `production-archive-profile` in
`benchmarks/performance-harness`, recorded in
`benchmarks/baselines/2026-10-15-production-archive-full.json`. "Before" is the
same workload compiled against 0.15.0 (commit `df945ef`): the restore cases
unchanged, and in place of the lifetime and multi-page cases, which need the
new API, a 0.15.0 lifetime case described below. That copy is not kept in the
tree. Every production record is real: settled executions are built through
the public state API and every commit goes through the production plugin.

**Restore validation of one compacted input.** One execution input whose
resource consumption and consumption outcome were archived in the first
resource commit, under filler commits of rejected-operation outcomes; the
workload times `validate_production_resource_continuation`, the restore check
of one input, and counts the objects it reads.

| Resource archive: records | Commits (records each) | Before: time, objects read | After: time, objects read |
| --- | --- | --- | --- |
| 1,001 | 38 (27) | 18.8 ms, 380 | 94 µs, 10 |
| 9,992 | 371 (27) | 181 ms, 3,710 | 111 µs, 12 |
| 100,010 | 3,705 (27) | 2.09 s, 37,050 | 123 µs, 14 |
| 9,986 | 40 (256) | 217 ms, 400 | 109 µs, 12 |
| 100,098 | 392 (256) | 1.86 s, 3,920 | 123 µs, 14 |
| 102 | 101 (1) | 3.2 ms, 1,010 | 86 µs, 8 |
| 1,002 | 1,001 (1) | 31.8 ms, 10,010 | 106 µs, 10 |
| 10,002 | 10,001 (1) | 344 ms, 100,010 | 112 µs, 12 |

Before, the check read every directory, blob, and page of the chain twice
(once for the consumption, once for the outcome): 3.8 MB at a thousand
records and 390 MB at a hundred thousand. After, it reads two index paths and
two blobs, 10 to 16 KB. A restore with archives also pays the resource
archive audit once, which 0.15.0 paid once per continuation witness; the
resource proposal measures that audit (0.9 s at about a hundred thousand
records).

**Archive lifetime.** Before: one applied commit, then the same batch queued
again and again, each settling as stale and leaving its receipt in the root,
with a production-only boundary per commit. The root then holds 8,192 receipts
in 3.37 MB; `prepare_production_archive` refuses (`archive_backpressure`), and
one more receipt does not validate, so the next commit would fail its
boundary.

| Settled commits (receipts in the root) | Production root | Journal bytes of the boundary | Boundary |
| --- | --- | --- | --- |
| 1 | 4.6 KB | 24 KB | 1.4 ms |
| 250 | 107 KB | 216 KB | 8.7 ms |
| 500 | 210 KB | 423 KB | 16.9 ms |
| 1,000 | 417 KB | 836 KB | 34 ms |
| 2,000 | 832 KB | 1.67 MB | 226 ms |

The root and the journal grew by about 415 and 830 bytes per receipt for
every boundary that changed the production root, so the journal of a whole
run grew with the square of its commits. The time column is single samples on
a shared machine; the last one is noisy.

After: a chain of commits through the plugin, each archiving the receipt of
the commit before it and acknowledging that commit in its boundary, as a host
application would.

| Commits | Production root | Journal bytes of the boundary | Boundary | Prepare | Enqueue |
| --- | --- | --- | --- | --- | --- |
| 1 | 4.8 KB | 25.8 KB | 2.2 ms | 0.15 ms | 0.78 ms |
| 1,000 | 4.8 KB | 12.0 KB | 1.1 ms | 0.06 ms | 0.19 ms |
| 8,192 | 4.8 KB | 12.1 KB | 0.91 ms | 0.07 ms | 0.23 ms |
| 8,193 | 4.8 KB | 12.1 KB | 2.6 ms | 0.06 ms | 0.88 ms |
| 30,000 | 4.8 KB | 12.1 KB | 1.0 ms | 0.08 ms | 0.29 ms |

The root holds one receipt and one handle throughout, and the journal bytes
are flat. Enqueueing prepares the batch a second time and stores and verifies
it. The first commit's journal includes the archived execution. The times are
single samples on a shared machine. In each of four runs the 8,193rd commit
was slower than the commits around it in both chains that reach it (3.0 ms
in the 8,193-commit chain of this one), which single samples do not explain;
the commits after it return to about 1 ms.

**Lookup.** 0.15.0 had no lookup: finding an archived production record meant
walking the chain, which costs what the restore audit costs.

| Commits (archived records) | Miss: time, objects read | Hit on the first execution: time, objects read | Restore audit | Lookup bytes written per commit | Index the head reaches |
| --- | --- | --- | --- | --- | --- |
| 1,000 | 23 µs, 4.0 | 205 µs, 5 | 25 ms | 2.6 KB | 297 pages, 179 KB |
| 8,193 | 32 µs, 4.9 | 175 µs, 6 | 194 ms | 3.4 KB | 3,826 pages, 1.6 MB |
| 30,000 | 46 µs, 5.2 | 193 µs, 6 | 793 ms | 4.1 KB | 14,479 pages, 5.9 MB |

These chains archive one record per commit, the worst case for index bytes:
each commit rewrites one index path. Each commit also writes about 2.1 KB of
blob, pages, and directory. The resource archive profile measures the same
index format up to a million records and 120,000 commits. The design target,
three commits a day for a hundred simulated years, is about 110,000 commits;
by those measurements a miss then reads about six objects in about 30 µs, a
commit's cost stays as above, each one-record commit writes about 4.5 KB of
index pages of which the head keeps about 200 bytes per record, and the
restore audit, which is linear in the archive, takes two to three seconds.

**A commit of several pages.**

| Commit | Pages | Prepare | Store and verify | Enqueue | Boundary | Records and pages written | Lookup pages written | Hit: time, objects, bytes read |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 70 records | 2 | 5.9 ms | 16 ms | 26 ms | 50 ms | 0.93 MB | 15 KB | 8.8 ms, 4.2, 754 KB |
| 1,023 records | 16 | 180 ms | 201 ms | 493 ms | 1.38 s | 13.8 MB | 186 KB | 10.0 ms, 5.1, 845 KB |

The second commit archives a receipt and 1,022 settled executions from a
14 MB root. Its boundary time is the whole-root work of removing them, which
every boundary that changes a production root pays in proportion to the root,
and the rebuild of the commit's blobs and directory, which costs about as much
as preparing them. Building one execution's record scans the root's hot
operation outcomes and completion receipts, so preparing, enqueueing, and
settling a commit each cost O(selected records × hot outcomes); a host
application that commits small batches often keeps both small. A hit reads
one blob of up to 64 records.

**Committed fixtures.** `production_archive_volume` runs a chain of six
commits through the plugin and checks the steady state, the lookup of the
first execution and of every archived receipt, and restore and exact replay;
the profile above runs the chain past 8,192 commits.

### What can still fail a boundary

- An archive commit or acknowledgement packet that decodes but is forged: a
  retention handle whose digest, phase, or bindings are wrong, or a
  selection that is empty, too large, or repeats a record.
  `enqueue_production_archive` and `finalize_production_archive_retention`
  never produce one; another plugin that schedules these packet types can.
  One that does not decode is refused (see
  [public production packets](#public-production-packets)).
- An applied commit whose selection is not the root's, or whose directory or
  retention objects differ from what the root's records rebuild: it would
  need a batch that `enqueue_production_archive` refused. The boundary checks
  the head's receipt, that every selected record is a terminal candidate, and
  the rebuilt directory, and fails otherwise.
- The 256 MiB byte cap of the production root, which the receipts and
  handles of stale commits approach only when a host application queues
  stale commits without ever acknowledging them.
- Counter overflow of the archive counts and the receipt sequence (64-bit).

## Output that settles after its completion

### Problem

The resource plugin defers a production output batch that finds no room in
its root and settles it when it comes due: it moves each credit's `at` to
that instant, so each outcome's `request_digest` commits to it, and its
acknowledgement to the production plugin carries the instant as
`settled_at`. The production plugin ignored `settled_at`, accepted outcomes
without comparing their request digests with its own requests, and its
archive rebuilt those digests with the completion instant. An execution whose
output had waited therefore settled but could never be archived:
`enqueue_production_archive` refused every batch that selected it, and
preparation selects terminal records in identity order, so the production
archive stopped there.

An acknowledgement that did not settle its execution exactly also failed its
boundary, and the acknowledgement ingress is public, so the host application
could queue one: a forged one, or a repeat of a settled one with different
content.

### Decision

- `ProductionOutputAcknowledgement` carries `settled_at`, the instant the
  credits settled, as the resource plugin already sends it.
  `acknowledge_output` rebuilds each credit request from the execution's
  request, the acknowledgement's source, the execution's completion
  certificate, and `settled_at`, and requires every outcome's
  `request_digest` to equal the rebuilt request's digest and `settled_at` to
  be no earlier than the completion. A repeat for a settled execution is a
  no-op only when its outcomes, source, and `settled_at` all equal those
  recorded.
- The execution records the instant as `output_settled_at`, which the
  acknowledgement digest also covers. `ProductionState::validate` requires it
  exactly on settled executions, and never before the completion.
- The archive rebuilds the request digests and the acknowledgement with
  `output_settled_at`, so an execution archives whenever its output settled.
  Restore rebuilds them the same way for every hot settled execution, before
  it reads anything else.
- Through the plugin, an acknowledgement that any of these checks refuses
  emits `canwu.production.output_ack_refused.v1`, which names the execution
  and the reason, changes nothing, and lets the boundary settle.

### Invariants

- A settled execution's output outcomes are exactly the resource plugin's
  settlements of its requests at `output_settled_at`, which is no earlier
  than `completed_at`.
- Every settled execution can be archived, whenever its output settled.
- An acknowledgement, from the resource plugin, queued by the host
  application, or scheduled by another plugin, never fails a boundary: one
  that does not decode is refused in the same way (see
  [public production packets](#public-production-packets)).

### Persistence, replay, and formats

- **Plugin state.** `ProductionExecution` gains `output_settled_at`, absent
  until the execution settles. A settled execution without it does not
  validate, so no settled record written before this change is accepted.
- **Archive objects.** The archived execution carries it; the blob format
  is version 2, which is new in 0.16.0.
- **Ingress.** The acknowledgement payload has the required field
  `settled_at`.
- **Events.** `canwu.production.output_ack_refused.v1` is new.

### Compatibility

A state-level caller of `acknowledge_output` passes `settled_at`: the instant
its credits' requests name, which is the completion instant for a batch that
settled when it was dispatched. An acknowledgement that does not match is
refused (`InvalidDomainRecord`, or `IdempotencyConflict` for a settled
execution) where it was accepted or, through the plugin, failed the
boundary.

### What can still fail a boundary

An acknowledgement fails a boundary only through a failed read or an
overflow while it is checked; the list for the whole plugin is under
[public production packets](#public-production-packets).

## Public production packets

### Problem

The host application can queue the production packet types registered as
public ingress: the operation packet (`production_operation_v1`), the output
acknowledgement, and the observation wake. It can also queue any completion
coordinator step through `enqueue_production_completion_operation`, which
checks nothing, and a continuation witness through
`enqueue_production_resource_continuation`. Another plugin can schedule any
production packet type, internal ones included, by declaring it as a target.
Three faults were older than this package:

- Every production packet decoded with a fallible step that failed the
  boundary. A host-queued acknowledgement with the payload
  `{"execution":"x"}` therefore failed every boundary with `InvalidPayload`,
  and a due packet cannot be withdrawn, so the run stopped.
- Settlement applied an operation packet without asking where it came from.
  A host-queued `CancelWorkOrder` under a made-up command settled as
  `Applied` and cancelled the order, with no command record and none of the
  checks admission makes: the issuer's authority over the holder, the run
  policy, the reserved outcome capacity, and the degraded-facility decision.
- A completion step or a continuation witness that production's checks
  refused failed the boundary. A host-queued `Activate` naming a boundary
  other than the one that settles it failed every boundary with
  `InvalidAuthority`.

### Decision

- A production packet that does not decode settles nothing: it emits
  `canwu.production.ingress_refused.v1`, or
  `canwu.production.output_ack_refused.v1` for an output acknowledgement,
  with the decode error, and the boundary settles. This holds for every
  production packet type, public or internal.
- An operation packet settles only when the admission of a tracked
  production command queued it: its cause must be `CauseRef::Command` of the
  command the packet names, that command must be this plugin's
  `apply_production_operation_v1` with the packet's envelope as its payload,
  and the packet's input hash must be the envelope's. Any other packet emits
  `canwu.production.ingress_refused.v1` and settles nothing, as
  `canwu-resource` treats its command packet.
- The packet type stays public, as the resource command packet does. Making
  it internal would not be enough: another plugin may declare any production
  packet type as a target, internal ones included, and the cause check
  refuses such a packet too.
- A completion step or a continuation witness that production's checks
  refuse with an expected domain rejection (`InvalidAuthority` among them)
  settles nothing: it emits `canwu.production.ingress_refused.v1` with the
  reason, and the boundary settles. Other errors still fail it.
- A continuation witness is checked when it is queued and again when it
  settles, where its project may have ended. Only the completion step that
  finalizes a project ends it, a scheduled-system packet. The witness that
  `enqueue_production_resource_continuation` queues is due at that instant,
  and the kernel orders the queue by due instant and then by class, so the
  witness, an acknowledgement, settles ahead of any such step due at the
  same instant. It finds its project ended only behind a step due earlier
  that a boundary has not yet admitted, and a witness another plugin
  schedules can be stale or forged; each is refused. No committed test
  reaches a stale witness: it would need a backlog of queued steps.

### Invariants

- Every applied production operation went through command admission.
- No production packet fails a boundary by not decoding, and no completion
  step or continuation witness by being refused.

### Persistence, replay, and formats

No persisted shape changes. `canwu.production.ingress_refused.v1` is a new
event of the apply system. The check reads the command record, which the
kernel keeps while the packet is queued, so a seal never removes it first,
and an exact replay reproduces each refusal.

### Compatibility

A host application that queued operation packets itself, rather than
tracked commands, now has them refused.

### What can still fail a production boundary

- An archive commit or retention acknowledgement whose content is forged but
  decodes: a retention handle whose digest, phase, or bindings are wrong, a
  selection that is empty, too large, or repeats a record, or an applied
  commit whose blobs and directory differ from what the root's records
  rebuild. `enqueue_production_archive` and
  `finalize_production_archive_retention` never produce one; another plugin
  that schedules these packet types can.
- A read of the evidence or archive stores that fails: a missing, forged,
  oversized, or malformed object of the production or resource archive fails
  the boundary, which settles once the store serves the object. It is never
  taken for a miss or a free key.
- Counter overflow: the archive counts, the receipt sequence, and the
  runtime revision (64-bit).
- The 256 MiB byte cap of the production root, which the receipts and
  handles of stale commits approach only when a host application queues
  stale commits without ever acknowledging them.

Any production packet a boundary admits, a refused one included, also
triggers the incident and report systems, which take it as the evidence of
their pass. A host application that queues junk public packets therefore
causes incident draws and report publication; this is older than this
package.

## Output that could never settle

### Problem

An execution's output requests name the operation keys under which the
resource plugin credits them, and the key of the completion lease that backs
the execution is its first output's key (`validate_completion_certificate`
requires it). The resource plugin settles nothing for a credit whose key it
reserves for its completion operations
(`RESOURCE_COMPLETION_OPERATION_KEY_PREFIX`, `resource:completion:`) or whose
key a different request already holds. `StartExecution` looked at neither, so
an execution could start with such a key and could then never settle: it kept
its capacity allocations, its consumed resource participant grant with its
completion units and target locks, and the outcome slots its lease had
reserved, with no operation that could release them.

The resource plugin also settles every output of an execution in one batch,
and skips a batch of more than 64 credits, or one its root could not hold
even beside an empty archive backlog, as permanently short of room. A process
could declare up to 128 outputs, and nothing compared an execution's output
set with the resource root's limits, so such an execution could start and
then never settle either.

### Decision

`StartExecution` refuses, before any evidence is resolved, an execution any of
whose output requests names:

- a key under the reserved prefix (`InvalidPayload`);
- a key that another running or output-pending execution names
  (`IdempotencyConflict`); `ProductionState::validate` also refuses a root in
  which two such executions share an output key;
- a key that a resource operation outcome already holds, in the resource root
  or in its archive (`IdempotencyConflict`). The archive is read through its
  lookup index, as the resource plugin reads it for every operation: the head
  directory, one page per index level, and one blob on a hit.

It also refuses an output set no resource output batch can settle:

- `ProductionState::validate` refuses a process with more outputs than
  `canwu_resource::MAX_PRODUCTION_OUTPUT_BATCH_CREDITS` (64), the bound the
  resource plugin's own settlement uses. Processes come from the scenario, so
  this is a refusal at scenario time (`InvalidDomainRecord` from
  `into_initial_record` or the runtime's construction).
- `StartExecution` refuses an execution with more output requests than that
  (`InvalidPayload`), as a second guard.
- Through the plugin, `StartExecution` refuses an output set that the
  resource root can never hold (`InvalidPayload`):
  `ResourceState::production_output_batch_can_fit` requires its credits, and
  the receipt of the archive commit that empties the backlog, to fit the
  root's archive candidate budget, and its credits to fit the root's outcome
  budget. The root's limits are fixed by the scenario, so a start this check
  admits is never skipped for a budget it can never meet.

Through the plugin each refusal is the operation's recorded rejection, and the
boundary settles. `RequestCompletionLease` refuses a key under the reserved
prefix the same way, because the lease's key becomes its execution's first
output key, and `ProductionState::validate` refuses a root holding an execution
whose output names one, so a scenario cannot start with one.

The other production operations name no new resource operation key. A
facility project's operation key must equal its lease's, which the lease
admission refuses under the prefix and which the coordinator keeps unique
among hot leases. The resource keys in the input bindings of an execution or a
project are those of exact consumption outcomes that already settled, which
admission checks against the resource root, or for a project with compacted
inputs through its continuation witness. `AbortCompletionLease` and the
completion coordinator ingress name a lease, not a key.

What these checks leave possible, each traced by the resource plugin as a
skip that never fails its boundary, while the execution keeps waiting for an
acknowledgement that does not come:

- a key that a resource operation queued by the host application takes
  between the start and the dispatch of the output (`OperationKeyTaken`);
- a credit the resource root rejects when the batch settles (a rejected
  credit skips the whole batch);
- a batch that still meets the root's byte cap after its delay has grown to
  the 1,024-minute cap (`RootLimit`).

A batch that waits for an archive commit, or for leases to release their
reserved room, waits as long as that takes; the host application's archive
maintenance ends the wait.

### Invariants

- No hot execution names a reserved key in its outputs, no two running or
  output-pending executions share an output key, and no production completion
  lease admitted from now on has a reserved key.
- No process declares more outputs than one resource output batch carries,
  and no execution the plugin admits has an output set its resource root can
  never hold.
- An output key that a resource outcome held when the execution started is
  refused, whether that outcome is hot or archived.
- A refusal changes nothing but the operation's outcome and the runtime
  revision.

### Persistence, replay, and formats

No persisted shape changes. `ProductionState::validate` is stricter, as said:
a process may declare at most 64 outputs, where it could declare 128. The
semantic hash change of the archive section covers it. The archive read
goes through the plugin archive provider, so a live run, a restored run, and
an exact replay given the same store read the same objects and decide alike.

### Compatibility

A host application that used a reserved key for an output or a completion
lease, or reused an output key, now has the operation refused; it could not
have settled. A scenario with a process of more than 64 outputs is refused,
and so is a start whose output set its resource root cannot hold; neither
could have settled.

### What can still fail a boundary

- A resource archive that cannot be read when an execution starts, while the
  resource root has an archive head: a missing, forged, oversized, or
  malformed object fails the boundary, which settles once the store serves the
  object. It is never taken for a free key. The resource plugin's own
  operations fail the same way.

## Open decisions and follow-up

- A re-sent production operation whose outcome was archived is applied again
  rather than recognized as a duplicate. The production lookup index makes a
  boundary-time check affordable, but it would make every production
  operation read the archive; the conservative choice keeps 0.15.0 behavior.
- An output key that a host-queued resource operation takes after the
  execution started, a credit rejected at settlement, and a batch that meets
  the byte cap after the longest delay still strand the execution, as
  described above. Releasing it needs an acknowledgement of a skipped batch,
  which the resource plugin does not send.
- `production_output_batch_can_fit` always counts the receipt of an archive
  commit. The resource plugin counts it only when its root holds hot
  candidates, so a batch exactly as large as the archive candidate budget,
  which would fit a root that never had a hot candidate, is refused. The
  conservative choice keeps the check independent of the root's history.
- Preparation still selects terminal executions and facility projects in
  identity order, so with a candidate budget below the backlog a record with a
  high identity waits until the backlog drains.
- Building an execution's archive record scans every hot operation outcome
  and completion receipt, so a commit costs O(selected records × hot
  outcomes) at preparation, at enqueueing, and at settlement. An index of
  outcomes and receipts by execution would make it linear in the selection;
  until then a host application commits small batches often.
- A forged archive commit or retention acknowledgement that decodes still
  fails the boundary, as listed above, and so does the root's byte cap. A
  bound on the receipts that duplicate and stale commits leave until they
  are acknowledged would keep a host application from approaching the cap;
  it is a follow-up.
- Any admitted production packet, a refused one included, triggers the
  incident and report systems, so a host application can cause incident
  draws by queuing junk public packets. Triggering them only for packets
  that change the root, or that settle, is a follow-up.
