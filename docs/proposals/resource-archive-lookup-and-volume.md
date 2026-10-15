# Resource Archive Lookup and Volume

Status: implemented in 0.15.0. It closes downstream gaps G-52 and G-54 in
`canwu-resource`.

Changed in 0.16.0 by
[resource refusals without stalls](resource-refusals-without-stalls.md): an
operation that does not fit the budget no longer fails its boundary and stays
queued. It is skipped with an event, nothing is recorded, and it settles when
sent again, behind as many commits as it takes. Room is measured on the
result, so a refusal needs room for its outcome only.
`finalize_resource_archive_retention` reads the disposition from the
state-side retention handle and succeeds after a later commit archived the
receipt. The passages below that say otherwise describe 0.15.0.

Changed in 0.16.0 by [resource root write cost](resource-root-write-cost.md):
the resource root persists as a live root and one record per hot terminal
bucket, so a boundary that settles an operation no longer decodes,
validates, re-encodes, or journals the hot terminal records it does not
touch. The table under "Choosing the budget" describes 0.15.0; a boundary now
costs the same at every backlog, and the backlog is paid for by the archive
commit that clears it. `max_archive_candidates` may be raised to 16,384 and a
commit carries up to 64 pages (16,384 records). The archive source root a
batch names is now a digest of the state revision, the admitted sequence, the
number of hot terminal records, and the archive head, so blob and retention
handle ids differ from 0.15.0.

The release entry is in [Versioning and Persistence](../versioning.md). The
other 0.15.0 proposals are
[society settlement cadence and runtime revision](society-cadence-and-runtime-revision.md),
[same-instant supersession and unplaced persons](same-instant-supersession-and-unplaced-persons.md),
[keyed-draw index, draw ID, and boundary profile](keyed-draw-index-and-settlement-profile.md),
[single-step completion lease and runtime protected-floor install](resource-single-step-lease-and-floor-install.md),
and [fiscal throughput and authority](fiscal-throughput-and-authority.md).

## Background

The resource simulation extension keeps its terminal records (operation
outcomes, closed demands and reservations, settled transfers, receipts) in its
one domain record until the host application moves them to a content-addressed
archive with `prepare_resource_archive`, `store_and_verify`,
`enqueue_resource_archive`, and `finalize_resource_archive_retention`. Each
archive commit is a directory object that names its blobs, its membership and
temporal pages, and the commit before it. The resource root holds the id of
the newest directory, the archive head. Only the host application writes
archive objects; a boundary reads them through the archive provider.

## G-52: the cost of a cold lookup

### Problem

Every resource operation asks the archive whether its operation key was used
before, so that a re-sent operation stays a no-op after its outcome has left
the root. In 0.14.0 the lookup walked the directory chain from the head and,
for each commit, authenticated the blob, every record in it, and both pages
before it read the membership page. A miss, which is the answer for every new
operation, therefore cost time in proportion to every record the run had
archived, and grew for the life of the run.

### Decision

Each commit directory commits to a cumulative lookup index over every record
archived up to and including that commit, as the new field
`ResourceArchiveIndexDirectoryV1::lookup_root`.

The index is a 16-way trie over the hexadecimal nibbles of a key digest
(`resource_archive_lookup_key_digest`, a domain-separated BLAKE3 hash of the
terminal record key). A `ResourceArchiveLookupPageV1` at a prefix covers every
archived key whose digest starts with that prefix. It is a leaf holding up to
`MAX_RESOURCE_ARCHIVE_LOOKUP_LEAF_ENTRIES` (8) entries, each a key digest with
the blob id and ordinal of its record; with more keys it is an interior page
naming one child page per next nibble. Pages are content-addressed objects in
the namespace `canwu.resource.archive.lookup-page`.

- **Lookup.** `archived_resource_terminal_record` reads the head directory,
  authenticated by its id, and then one lookup page per level, each
  authenticated by its id and checked against the prefix it was reached by. A
  key the index does not hold is a miss, and nothing else is read. A hit reads
  the one blob the entry names, authenticates it, and validates the typed
  closure of the record it returns.
- **Commit.** `prepare_resource_archive(candidate_limit, store)` now takes the
  archive store. It reads the head directory and the lookup pages on the paths
  of the new keys, and prepares their replacements; every other page is shared
  with the previous index. `store_and_verify` stores the new pages and checks
  that each record of the commit resolves through the stored index.
  `enqueue_resource_archive` still requires the batch to equal what the current
  root and the store derive.
- **Shape.** A page is a leaf exactly when at most 8 keys share its prefix, so
  the index is a function of its set of entries (key digest, blob id,
  ordinal): the same entries give the same root in whatever order and grouping
  they were added. A blob id commits to the root its batch was prepared from,
  so two runs that archive the same records in different batches hold
  different entries and different roots. What the restore audit relies on is
  narrower and exact: the newest entry of every key in a directory chain
  determines the index of its head. A key archived again points at its newest
  record, which is the answer the 0.14.0 walk gave.

The depth of the index grows with the logarithm of the archived records (one
level per factor of 16) and does not depend on the number of commits.

The three constants were chosen as follows. Fan-out 16 makes the path of a
key its digest in hexadecimal, one nibble per level, and keeps a lookup at
four to seven objects up to a million records. A commit rewrites one path per
key it adds; with fan-out 4 the pages would be smaller and, by estimate, a
commit would write about a third fewer index bytes, for about twice the page
reads per lookup. The leaf size was measured: at 100,000 records archived 27
at a time, a commit wrote 2.6 KB of lookup pages per record with leaves of 4,
2.7 KB with 8, 3.1 KB with 16, and 3.8 KB with 32, and a leaf of 4 adds a
level sooner. The largest well-formed page, a full leaf or a page with sixteen
children at a deep prefix, encodes to about 1.3 KB;
`MAX_RESOURCE_ARCHIVE_LOOKUP_PAGE_BYTES` is 4 KiB, which leaves room for a
store that returns the same JSON with other whitespace.

Rejected alternatives:

- *Read only the membership page of each commit* (the fix as filed), or add a
  key filter to each directory. A miss still reads one object per commit, so
  the cost grows with the commit count and the read bound stays a limit on the
  life of a run.
- *Keep verified directories in memory for the life of the runtime.* The memory
  and the rebuild after a restore both grow with the archive, a simulation
  plugin has no runtime-owned storage, and a long-lived runtime would settle a
  boundary that a restored one, given a store that has lost an object, fails.
- *Merge the membership pages of older commits into larger sorted runs.* It
  writes fewer bytes per record, but a lookup probes every run, and merging the
  largest level rewrites the whole index inside one commit.
- *Hold the index root in the resource root instead of the directory.*
  Preparing a commit would need no store read, but the pages of a queued commit
  would not be reachable from the one archive root its ingress retains, and the
  plugin state shape would change.

### Invariants

- A lookup returns the newest archived record of a key or nothing, as before.
  What changed is how many objects it reads.
- A miss is decided by authenticated pages only. A missing, forged, oversized,
  or malformed directory, page, or blob is an error, never a miss, and never a
  durable rejection: the boundary fails and settles once the store can serve
  the object.
- The number of objects a lookup reads is bounded whatever the store returns:
  one directory, at most 65 lookup pages (the root and one page per nibble of
  the digest, `MAX_RESOURCE_ARCHIVE_LOOKUP_DEPTH + 1`, so no key set can
  exceed it), and one blob on a hit. A lookup page is also bounded in bytes.
  The directory and the blob are not, as before.
- A lookup trusts the head directory to name an index that holds every
  archived key. `enqueue_resource_archive` establishes that for each commit it
  queues, because the batch must equal what the root and the store derive, and
  `validate_resource_archive_store` checks it for the head of a restored root.
  A `ResourceState` assembled by hand, or a root restored without the checked
  wrappers, is not checked.
- A boundary only reads archive objects, by content id, starting from the head
  the resource root records. A live run, a restored run, and an exact replay
  given the same store therefore read the same objects and settle the same
  way. A failed boundary leaves the head, and so the index, unchanged.
- `validate_resource_archive_store`, which the checked restore and replay
  wrappers call, rebuilds the index from the newest entry of every key in the
  directory chain and requires the store to hold exactly those pages and the
  head to name their root. It does not audit the index of a commit that is
  queued and not yet applied. `store_and_verify` checked that index when it
  was stored; if a page of it is lost before the commit becomes the head, a
  lookup that needs the page fails its boundary and is never taken for a miss.

## G-54: archive volume and archive lifetime

### Problem

Two limits bounded how much a run could archive. `max_archive_candidates` could
not exceed 256, and one commit carried one page of at most 256 records, so the
root held at most 256 terminal records between two commits, and an operation
that did not fit failed its boundary and stayed queued until a commit made
room. And because a lookup stopped after 2,048 directories, every lookup of a
run failed its boundary once the run had made 2,049 commits.

A third fault on the same path predates 0.15.0. An archive commit that had
gone stale, because another commit or an operation settled first, still needed
a free candidate slot for its maintenance receipt. Against a full budget it
failed its boundary, and since its packet stayed queued, every later boundary
failed too, including the one that would have applied the valid commit queued
beside it.

A fourth is the same stall seen from the operation. An operation that does not
fit the candidate budget fails its boundary and stays queued; it is not
recorded as a rejection. An archive commit queued behind it in that boundary
never ran, so the commit that would have made room could not.

### Decision

- `max_archive_candidates` may be set up to
  `ResourceLimitsV1::MAX_ARCHIVE_CANDIDATES`, 4,096. The canonical value stays
  256, so the limits and state of existing scenarios are unchanged.
- One commit carries up to `MAX_RESOURCE_ARCHIVE_COMMIT_RECORDS` (4,096)
  records as up to `MAX_RESOURCE_ARCHIVE_COMMIT_PAGES` (16) blobs of at most
  256 records, each with its own membership and temporal page. The directory
  lists them in order. `PreparedResourceArchiveBatchV1` keeps its first page in
  `blob`, `membership_page`, and `temporal_page` and carries the rest in
  `additional_pages`. One commit can always settle every candidate of a root.
- A run may make any number of commits. No lookup walks the chain, and
  `MAX_RESOURCE_ARCHIVE_LOOKUP_DIRECTORIES` is removed. What still walks the
  chain runs outside boundaries: the restore audit, bounded by the head
  revision; `authenticate_reachable_resource_archive_directory`, which runs
  that audit and then walks to its directory; the archive reachability
  participant, which walks every directory it reaches; and the restore
  validation of `canwu-production`.
- An archive commit never fails for capacity. An applied commit archives at
  least one candidate before its own receipt becomes one. The receipt of a
  stale commit is no longer an archive candidate: it stays in the root,
  outside both budgets, until the host application acknowledges the commit's
  retention. A stale commit is thus a terminal no-op in every state of the
  root, and the valid commit queued beside it applies.
- An acknowledgement retires the retention handle it names and every stale
  receipt recorded under that handle. The same batch queued twice settles once
  as applied and once as stale under one handle, and the acknowledgement of
  that handle removes the stale receipt. An acknowledgement that arrives
  twice, or after a later commit archived the receipt it names, retires the
  handle and settles. `finalize_resource_archive_retention` reads the receipt
  to build the acknowledgement, so a host application finalizes each commit
  before the next commit archives its receipt. A call made later fails, and
  the state-side handle and the store-side handle in `DurableIngress` then
  stay permanently.
- Within one boundary, archive commits and retention acknowledgements settle
  before every other packet of the resource plugin, each group in its admitted
  order. A commit queued behind an operation that does not fit the budget
  applies and makes room for it. A commit prepared from the root the boundary
  started from applies even when an operation was admitted ahead of it; in
  0.14.0 that operation settled first, and the commit went stale once the
  operation left a terminal record. A boundary that admits no archive commit
  keeps its records and hashes: an acknowledgement and an operation do not
  touch the same records.
- `max_archive_maintenance_receipts` counts the receipts of applied commits
  only. Each of them is a candidate, so `ResourceLimitsV1::validate` requires
  it to be at least `max_archive_candidates` (its hard maximum rises from
  1,024 to 4,096). With a smaller receipt budget, a valid commit could exceed
  it and fail in the same way.

### Choosing the budget

The ceiling is headroom for a burst, not a working size. Every boundary that
changes the resource root decodes, validates, re-encodes, and journals the
whole root, hot terminal records included. One boundary that settles a single
operation therefore costs time and journal bytes in proportion to the backlog:

| Hot terminal records | One boundary | Journal bytes of that boundary |
| --- | --- | --- |
| 0 | 0.4 ms | 9 KB |
| 256 | 13 ms | 0.30 MB |
| 1,024 | 52 ms | 1.2 MB |
| 2,048 | 105 ms | 2.3 MB |
| 4,000 | 246 ms | 4.6 MB |

That is 50 to 60 µs and 1.1 KB of journal per hot record, for every boundary
that touches the root. The journal bytes are exact; the time varied between
runs on the shared machine, up to 95 µs per hot record. Each further operation
in the same boundary adds its own whole-root validation, about 3 µs per hot
record. A host application sizes `max_archive_candidates` from these two
columns at the backlog it is willing to carry, and keeps the backlog small by
committing often, which the removed lifetime limit now allows. Raising the
budget only lets a burst settle without waiting for a commit; it does not make
the burst cheap.

The budget also has a floor. Before an operation settles, the root must have
room for every terminal record the operation can produce: between one and
eight by its kind, besides the slots reserved for admitted completion
lifecycles. The budget must therefore cover the backlog plus every terminal
record the operations due at one boundary can produce. An operation that does
not fit fails its boundary and stays queued; it is not a recorded rejection,
and the boundary keeps failing until an archive commit makes room. That commit
may be queued behind the operation, because archive commits settle first.

When a boundary fails for this backpressure, the host application queues one
commit that selects every candidate. Several smaller commits do not help:
they are all prepared from the same root, so once the first applies, every
commit queued behind it is stale. An applied commit also leaves one candidate
behind, its own maintenance receipt, so the most room a commit can make is the
budget minus one. No commit helps when the operations due at one boundary need
more than that: such a boundary cannot settle under its budget, and this
change does not alter that.

A ceiling of 16,384, twice the hard maximum of hot operation outcomes, was
considered and rejected. The numbers above do not support it: by the same
proportion, a root at that backlog would spend about a second and 18 MB of
journal on every boundary. The ceiling is 4,096: sixteen pages, sixteen times
the 0.14.0 bound, and on the measured machine about a quarter of a second per
boundary at the very top. Raising it later relaxes one validation bound and
the number of pages a directory may list; no stored format depends on it.
Lowering the cost itself needs a root that is validated and journaled
incrementally, which is outside this change.

### Invariants

- A commit of several pages is one directory and one atomic archive commit:
  all of its records leave the root in one boundary, or none do.
- `authenticate_resource_archive_directory` authenticates every blob and page
  of a commit and requires the pages to cover exactly the records of the blobs,
  with one membership page and one temporal page per blob.
- The retention handle of a commit lists every blob, membership page, and
  temporal page of the commit and its directory. Lookup pages are reached
  through the directory. Releasing a retention handle removes its protection
  and never deletes an object: two batches prepared from one root share their
  leading blobs, pages, and lookup pages, so an object named by a rejected or
  abandoned handle may belong to an applied commit. Deletion is by
  reachability only.
- Hot receipts of applied commits never exceed the candidates, the candidates
  never exceed `max_archive_candidates`, and that never exceeds
  `max_archive_maintenance_receipts`. Stale receipts and retention handles are
  recovery metadata, bounded by the host application's acknowledgements.
- Every hot receipt of an applied commit is an archive candidate, and no
  receipt of a stale commit is. `ResourceState::validate` checks both.

## Persistence, replay, and formats

- **Plugin state.** The shape of `ResourceState` and
  `RESOURCE_STATE_FORMAT_VERSION` (2) are unchanged. A run that never archives
  encodes the same resource root as before. A stale receipt is now present in
  `archive_maintenance_receipts` without an entry in
  `terminal_archive_candidates`, and it is never archived.
- **Archive objects.** The directory gains the required field `lookup_root`,
  so its ids differ from 0.14.0 and a 0.14.0 directory does not decode. Lookup
  pages are a new object kind. Blobs, membership pages, and temporal pages keep
  their format: the same records give the same bytes and ids.
- **Ingress.** The archive commit packet, the retention acknowledgement packet,
  and the retention binding are unchanged; `selected` may now name up to 4,096
  keys.
- **Plugin identity.** `RESOURCE_SEMANTIC_HASH` changes. Registered commands,
  ingress, boundary systems, and schemas are unchanged.
- **Kernel formats.** Snapshot, commitment, checkpoint, evidence-journal, and
  paged-checkpoint formats and the hash algorithms are unchanged.
- **Retention.** A restore, an exact replay, or a fork reads the index of
  every head it passes through, so the store must keep the lookup pages those
  heads reach. The engine never deletes a page. The archive reachability
  participant marks every lookup page reachable from a chain, queued, or
  unacknowledged directory; the index of the head and of each queued commit
  must be complete, and any other is marked as far as the store still holds
  it.

  `superseded_lookup_pages` of a prepared batch names the lookup pages of the
  previous head's index that the batch's index no longer reaches. The list may
  be used only after the batch's commit settled as `Applied`. A batch whose
  commit was rejected as stale carries a list too, and that list names pages
  of the live head. Deleting the pages of an applied commit is safe only after
  a durable snapshot or checkpoint taken after that commit exists, and it
  gives up every earlier point of the run that reads one of those pages: a
  snapshot or checkpoint taken before the commit no longer restores, a journal
  no longer replays across it, and a fork whose head precedes it no longer
  continues. This reaches back as far as the commit that wrote each page, not
  only to the commit before, because consecutive heads share pages. A host
  application that needs any of those keeps the pages.
  `live_resource_archive_lookup_pages` recomputes what a live run reads when a
  list was lost: the pages reachable from the head and from each queued
  commit, whose directory roots the host application passes. A commit is
  queued when its store-side retention handle is in `DurableIngress` and the
  root holds no handle with its id. A queued commit the host application does
  not name is not covered.

  A snapshot taken while a stale commit is unacknowledged holds that commit's
  retention handle, and restoring it needs the commit's objects. After the
  acknowledgement and a sweep by the reachability manifest, the objects that
  only that commit named are gone and that snapshot no longer restores. Exact
  replay across that point still works, because settling a stale commit reads
  nothing from the store.

## Compatibility

0.15.0 is a pre-1.0 minor release. Source and behavior changes:

- `ResourceState::prepare_resource_archive` takes the archive store as a second
  argument. `()` implements `ResourceArchiveStore` as the absent store, which
  is enough to prepare the first commit of a root.
- `ResourceArchiveIndexDirectoryV1` gains `lookup_root`, and
  `PreparedResourceArchiveBatchV1` gains `additional_pages`, `lookup_pages`,
  and `superseded_lookup_pages`.
- `MAX_RESOURCE_ARCHIVE_LOOKUP_DIRECTORIES` is removed.
- `archived_resource_terminal_record` and `archived_resource_operation_outcome`
  no longer return `ResourceError::LimitExceeded`.
- `ResourceLimitsV1::validate` rejects limits whose
  `max_archive_maintenance_receipts` is below `max_archive_candidates`. The
  canonical limits satisfy it; a scenario that raises the candidate budget
  above 1,024 raises the receipt budget with it.
- The maintenance receipt of a stale archive commit is no longer a terminal
  archive candidate and is removed when its retention is acknowledged.
- Within one boundary, the resource plugin settles archive commits and
  retention acknowledgements before its other packets. A commit that was
  prepared from the root the boundary started from and admitted behind an
  operation now applies; in 0.14.0 it went stale once the operation left a
  terminal record, or never ran when the operation did not fit the candidate
  budget.

An archive chain written by 0.14.0 is not readable and needs no migration: the
exact engine-version check already rejects every 0.14.0 snapshot, checkpoint,
and journal (see [Versioning and Persistence](../versioning.md)), and a run
created by 0.15.0 starts with an empty archive head, so no 0.15.0 run can
reach a 0.14.0 directory. Retain the 0.14.0 engine to read one. The objects of
both versions can share a store, because their ids never coincide.

## Measurements

Release build, one thread, an in-memory store that copies the bytes of every
object it serves, on one machine that other builds were sharing. Each archived
record is a rejected operation outcome of about 1.1 KB. The workload archives
the stated number of records in commits of the stated size through
`prepare_resource_archive` and `store_and_verify`, and then times
`archived_resource_operation_outcome`; the store counts the objects read.

"After" is the `full` preset of `resource-archive-profile` in
`benchmarks/performance-harness`, recorded in
`benchmarks/baselines/2026-10-13-resource-archive-full.json`. "Before" is the
same workload compiled against 0.14.0 (commit `6617f54`). The profile binary
does not compile there, because `prepare_resource_archive` takes no store and
a batch has no lookup pages, so the before-run used a copy of its lookup case
with those two differences and with each hot record produced by a real
rejected operation. That copy is not kept in the tree.

A lookup miss, against archived records:

| Archived records | Commits | Before: time, objects read | After: time, objects read |
| --- | --- | --- | --- |
| 999 | 37 of 27 | 7.4 ms, 185 | 15 µs, 4 |
| 9,990 | 370 of 27 | 73 ms, 1,850 | 20 µs, 5 |
| 100,008 | 3,704 of 27 | fails (past 2,048 commits) | 25 µs, 6 |
| 1,024 | 4 of 256 | 8.1 ms, 20 | 15 µs, 4 |
| 9,984 | 39 of 256 | 70 ms, 195 | 19 µs, 5 |
| 100,096 | 391 of 256 | 768 ms, 1,955 | 25 µs, 6 |
| 1,000,192 | 3,907 of 256 | fails (past 2,048 commits) | 30 µs, 7 |

A lookup miss, against the commit count (one record per commit):

| Commits | Before: time, objects read | After: time, objects read |
| --- | --- | --- |
| 100 | 1.5 ms, 500 | 11 µs, 3 |
| 1,000 | 18 ms, 5,000 | 15 µs, 4 |
| 2,048 | 37 ms, 10,240 | 17 µs, 4 |
| 2,049 | fails | 16 µs, 4 |
| 10,000 | fails | 20 µs, 5 |
| 120,000 | fails | 27 µs, 6 |

A hit on the oldest record took as long as a miss before. After, it takes
about 0.05 ms with one-record blobs and about 1 ms with 256-record blobs,
which is the cost of authenticating the blob. A miss also decodes the head
directory: at about 100,000 records it takes 32 µs while the head is a 16-page
commit, against 25 µs for a one-page head.

The fixture `gap_g54_resource_archive_volume` runs 2,060 archive commits
through the plugin and then resolves the operation archived by the first.

What the change costs:

- **Bytes.** Each commit rewrites the lookup pages on the paths of its keys.
  At about 100,000 archived records that is 2.7 KB per record for commits of
  27, 1.8 KB for commits of 256, and 0.8 KB for commits of 4,096, beside about
  1.1 KB of blob and per-commit pages. The worst case is one record per
  commit: 4.5 KB of lookup pages per record at 120,000 commits, beside 2.3 KB
  of blob and per-commit pages. The pages the head still reaches hold about
  200 bytes per record; the rest are the superseded pages a host application
  may delete under the retention rule above.
- **Commit time.** Preparing, then storing and verifying, a commit on top of
  100,000 archived records takes 0.6 ms and 1.0 ms for 27 records, 4 ms and
  8 ms for 256, and 41 ms and 86 ms for 4,096.
- **Restore audit.** `validate_resource_archive_store` is linear in the
  archive: 0.9 s for 100,096 records in 391 commits and 16 s for 1,000,192
  records; 0.19 s for 10,000 one-record commits and 2.6 s for 120,000.
  Authenticating the chain, which is all the 0.14.0 audit did, takes 0.8 s for
  the 100,096-record case. The audit holds about 40 bytes per archived record
  while it runs.
- **Hot backlog.** The table under "Choosing the budget" is the `backlog_cases`
  part of the same baseline: one requester-scoped allocation settled through
  the plugin, timing `settle_boundary` only, with the evidence that boundary
  appends measured as encoded JSON. The same operation applied to a detached
  copy of the root, which is the state-level share of that boundary, takes
  0.6 ms at 256 hot records, 2.7 ms at 1,024, and 10 ms at 4,000. This cost is
  not new, but a larger candidate budget makes it reachable.

## Other archives

`canwu-production` has its own archive code, which this change did not touch.
In 0.15.0:

- No production boundary read the production archive, so no production
  operation paid a lookup.
- A production commit carried one page of at most 512 records.
- A production run could make at most 8,192 archive commits: each commit left a
  maintenance receipt in the production root, receipts never left it, and
  `prepare_production_archive` refused once the root held
  `max_operation_outcomes` of them. This was a lifetime limit of the same kind
  as the one removed here.
- Production restore validation found each compacted resource input by
  walking the resource archive chain, which cost time in proportion to the
  archive for each input. It ran only at restore and when a continuation
  witness was queued, and it read multi-page commits unchanged.

0.16.0 gives the production archive the same lookup index, commits of several
pages, and receipts that leave the root, and moves production restore
validation onto the resource archive's lookup index; see
[production archive volume](production-archive-volume.md). To serve it, the
resource crate makes its lookup-index functions public:
`prepare_resource_archive_lookup_entries`, `find_resource_archive_lookup_entry`,
`verify_resource_archive_lookup_entries`, `collect_resource_archive_lookup_pages`,
and `audit_resource_archive_lookup_index` with `ResourceArchiveAuditEntry`.
The resource archive's own formats and behavior are unchanged.

The package archive of `canwu-force-supply-reference`, which the economy
reference integration also uses, resolved a record in 0.15.0 the way the
0.14.0 resource archive did: it walked its chain and authenticated every
commit it passed, inside boundaries. 0.16.0 gives it the same lookup index;
see [reference archive lookup and replay profile](reference-archive-and-replay-profile.md).
