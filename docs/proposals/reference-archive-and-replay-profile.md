# Reference Archive Lookup, Seal-Stable Reference Validation, and Replay Profile

Status: implemented in 0.16.0. Two items:

- **Reference archives and validation.** The package archive of
  `canwu-force-supply-reference`, which `canwu-economy-reference` also uses,
  gains the cumulative lookup index of the resource archive, so a lookup
  inside a boundary no longer walks and authenticates the archive's whole
  directory chain. The host-side validator of `canwu-ming-fiscal-reference`
  reads only state that a seal cannot change, through a new host form of
  `replay_stable_domain_record_version` in `canwu-sim`, and validates a sealed
  run as it validates an unsealed one.
- **Replay profile.** `canwu-sim` can profile an exact replay boundary by
  boundary, and every boundary profile splits the simulation core's own time
  into named engine phases.

The contracts this builds on are
[resource archive lookup and volume](resource-archive-lookup-and-volume.md)
(the lookup index), [production archive volume](production-archive-volume.md)
(the same index reused by a second archive), and the boundary profile of
[keyed-draw index, draw ID, and boundary profile](keyed-draw-index-and-settlement-profile.md).

The durable tests are:

- `crates/integrations/canwu-force-supply-reference/tests/package_archive_lookup.rs`:
  `a_lookup_reads_one_index_path_whatever_the_number_of_commits`,
  `a_missing_or_forged_index_page_is_an_error_never_a_miss`, and
  `reachability_keeps_every_page_the_head_or_a_queued_commit_reads`.
- `crates/integrations/canwu-ming-fiscal-reference/tests/seal_stable_validation.rs`:
  `the_validator_answers_alike_sealed_unsealed_restored_and_replayed`.
- `crates/api/canwu-api/tests/boundary_profile_engine_phases.rs`:
  `engine_phases_partition_each_boundary_and_a_replay_is_profiled_boundary_by_boundary`
  and `a_command_free_replay_is_profiled_boundary_by_boundary`.
- `crates/integrations/canwu-economy-reference/tests/economy_operation_id_reuse.rs`
  and `economy_archive_commit_room.rs` (see
  [fixed beside the index](#fixed-beside-the-index)).

## Package archive lookup index

### Problem

The force-supply and economy reference plugins keep their terminal records
(settled lifecycles, operation outcomes, superseded observations) in their
plugin roots until the host application moves them to a package archive.
Each archive commit is a directory that names one blob of at most 512
records, its membership and temporal pages, and the commit before it.

Both plugins ask the archive, inside boundaries, whether a key was archived
before: a re-sent operation must stay idempotent after its outcome left the
root, and an intent or saga acknowledgement must find its archived lifecycle.
The lookup walked the directory chain from the head and, at every commit,
authenticated the blob, every record payload in it, and both pages before it
read the membership page. A miss, the answer for every new operation, read
every object the archive held, and its cost grew for the life of the run.
The resource archive had the same cost shape until 0.15.0 gave it the index
this item reuses.

### Decision

The package archive uses the lookup index of the resource archive, in the
way production does:

- `PackageArchiveIndexDirectoryV1` gains the required field `lookup_root`:
  the root of a cumulative index over every record archived up to and
  including that commit. The index is a 16-way trie over key digests with
  leaves of at most eight entries
  (`canwu_resource::ResourceArchiveLookupPageV1`). Its pages are stored under
  the archive domain's new `PackageArchiveDomainV1::lookup_namespace`
  (`canwu.force-supply-reference.archive.lookup-page` and
  `canwu.economy-reference.archive.lookup-page`). A key's digest is
  `package_archive_lookup_key_digest`, a BLAKE3 hash of the key under the
  domain's own prefix.
- The package archive does not copy the index code. It passes the resource
  index functions (`prepare_resource_archive_lookup_entries`,
  `find_resource_archive_lookup_entry`,
  `verify_resource_archive_lookup_entries`,
  `audit_resource_archive_lookup_index`,
  `collect_resource_archive_lookup_pages`) a read-only view of its store that
  serves the resource lookup-page namespace from the domain's lookup
  namespace and refuses every other namespace.
- **Lookup.** `load_package_archive_record` reads the head directory,
  authenticated by its id, one index page per level, each authenticated by
  its id and checked against the prefix it was reached by, and on a hit the
  one blob the entry names. The blob is authenticated by its content id, and
  the record's key and payload digest are checked. A key archived more than
  once resolves to its newest record. A key the index does not hold is a
  miss; a missing, forged, oversized, or malformed object is an error.
- **Commit.** `prepare_package_archive` takes the archive store as a new last
  argument. It reads the head directory and the index pages on the paths of
  the new keys and prepares their replacements; every other page is shared
  with the previous index. `PreparedPackageArchiveBatchV1` carries the new
  pages (`lookup_pages`) and the previous pages its index no longer reaches
  (`superseded_lookup_pages`). `store_and_verify` stores the pages, reads the
  directory back, and checks that every record of the commit resolves
  through the stored index to its own blob position. `prepare_force_archive`
  and `prepare_economy_archive` take the store too; `()` is the absent
  store, enough for an archive's first commit. The enqueue helpers still
  require the batch to equal what the current root and the store derive.
- **Restore audit.** `validate_package_archive_store` walks the chain as
  before and also rebuilds the head's index from the newest entry of every key
  in the chain; the store must hold exactly those pages, and the head
  directory must name their root. It now also checks each commit's exact
  record count, the digests of the membership and temporal pages, and that
  every membership names its blob and its position in it.
- **Reachability.** `extend_archive_reachability` takes the head and the
  directories of unacknowledged retention handles separately. The
  directories the kernel already marked in the manifest are the commits
  still queued. It marks the blobs and pages of every directory it reaches,
  as before, and now the lookup pages: the index of the head and of each
  queued commit must be whole, and the index of any older directory is
  marked as far as the store still holds it.

The boundary that applies a package archive commit reads no archive object,
as before; it records the directory the commit names as the new head.

Rejected alternatives:

- *Read only the membership page of each commit.* A miss would still read one
  object per commit, so its cost would still grow with the commit count.
- *A package-specific index format.* The resource format already serves two
  archives, its functions take key digests, and a third copy would drift.
- *Keep the chain walk and bound the archive.* The archive grows for the life
  of a run; any bound would end the run's ability to archive.

### Invariants

- A lookup returns the newest archived record of a key or nothing, as the
  chain walk did. It reads at most one directory of at most
  `MAX_PACKAGE_ARCHIVE_DIRECTORY_BYTES` (4 KiB, the resource archive's page
  bound), `MAX_RESOURCE_ARCHIVE_LOOKUP_DEPTH + 1` index pages of at most
  `MAX_RESOURCE_ARCHIVE_LOOKUP_PAGE_BYTES` each, and one blob of at most
  `MAX_PACKAGE_ARCHIVE_BLOB_BYTES` (64 MiB, production's batch bound),
  whatever the store returns: each is refused before it is decoded when
  larger. `store_and_verify` refuses a commit whose blob exceeds its bound
  before storing anything, so an honest commit is never refused later.
- A miss is decided by authenticated pages only. A missing or forged page is
  an error and never a miss, so a store that lost a page fails the boundary
  (which settles once the store serves the page) and never lets a re-sent
  operation apply twice.
- A lookup trusts the head directory to name an index that holds every
  archived key. The enqueue helpers establish that for each commit they queue
  (`store_and_verify` checks every record against the stored index), and the
  restore audit checks it for the head of a restored root.
- A boundary reads archive objects only through the plugin archive provider,
  by content id, from the head its root records. A live run, a restored run,
  and an exact replay given the same store read the same objects and settle
  alike, and a failed boundary leaves the head and its index unchanged.

### Persistence, replay, and formats

- **Archive objects.** The directory gains the required field `lookup_root`,
  so directory ids change; lookup pages are a new object kind under each
  domain's lookup namespace. Blobs, membership pages, and temporal pages keep
  their format.
- **Plugin roots.** The force-supply and economy roots keep their shape.
- **Ingress.** The archive commit and acknowledgement packets keep their
  shape. The retention handle keeps listing the blob, pages, and directory of
  its commit; lookup pages are reached through the directory.
- **Plugin identity.** `FORCE_SUPPLY_SEMANTIC_HASH` and
  `ECONOMY_SEMANTIC_HASH` change.
- **Kernel formats.** None change.
- **Retention.** A restore, an exact replay, or a fork reads the index of the
  head it starts from, so the store keeps those pages. The retention rule of
  the resource archive applies to `superseded_lookup_pages`: they may be
  deleted only after the commit applied and a durable snapshot or checkpoint
  taken after it exists, and doing so gives up every earlier point of the run
  that reads one of them.

### Compatibility

These crates are not published. Source changes:

- `PackageArchiveDomainV1` gains `lookup_namespace`;
  `PackageArchiveIndexDirectoryV1` gains `lookup_root`;
  `PreparedPackageArchiveBatchV1` gains `lookup_pages` and
  `superseded_lookup_pages`.
- `prepare_package_archive`, `ForceSupplyStateV1::prepare_force_archive`, and
  `EconomyReferenceStateV1::prepare_economy_archive` take the archive store.
- `extend_archive_reachability` takes the head and the pending roots
  separately.
- `FORCE_ARCHIVE_LOOKUP_PAGE_NAMESPACE`, `ECONOMY_ARCHIVE_LOOKUP_PAGE_NAMESPACE`,
  `package_archive_lookup_key_digest`, and `impl PackageArchiveStore for ()`
  are new.

A package archive written by 0.15.0 does not decode (its directory lacks
`lookup_root`) and is never read: the exact engine-version check refuses
every 0.15.0 snapshot, checkpoint, and journal.

### Measurements

Release build, one thread, an in-memory store that copies the bytes of every
object it serves, on a machine that other builds were sharing. The workload
archives records of about 1 KB (a key, 64 integers, and a 256-byte string)
with `prepare_package_archive` and `store_and_verify`, then times
`load_package_archive_record` for a key never archived (a miss) and for the
first key archived (a hit on the oldest record), averaging repeated lookups,
and counts the objects and bytes the store serves. "Before" is the same
workload compiled against the base of this change (`release/0.16.0` at
`f511463`, the 0.15.0 package archive), whose `prepare_package_archive` takes
no store; the workload is not kept in the tree.

| Archived records | Commits (records each) | Before: miss | After: miss | Before: hit on the oldest | After: hit on the oldest |
| --- | --- | --- | --- | --- | --- |
| 1,024 | 2 (512) | 5.4 ms, 10 objects, 1.3 MB | 14 µs, 4 objects, 3.4 KB | 5.9 ms, 11 objects | 1.4 ms, 5 objects, 0.39 MB |
| 10,240 | 20 (512) | 42 ms, 100 objects, 13 MB | 18 µs, 5 objects, 4.1 KB | 41 ms, 101 objects | 0.97 ms, 6 objects, 0.39 MB |
| 102,400 | 200 (512) | 488 ms, 1,000 objects, 140 MB | 32 µs, 5 objects, 5.1 KB | 651 ms, 1,001 objects | 1.6 ms, 7 objects, 0.39 MB |
| 999 | 37 (27) | 6.5 ms, 185 objects | 23 µs, 4 objects | 6.4 ms, 186 objects | 107 µs, 5 objects |
| 9,990 | 370 (27) | 68 ms, 1,850 objects | 27 µs, 5 objects | 62 ms, 1,851 objects | 110 µs, 6 objects |
| 1,000 | 1,000 (1) | 15 ms, 5,000 objects | 17 µs, 3 objects | 15 ms, 5,001 objects | 27 µs, 5 objects |
| 10,000 | 10,000 (1) | 151 ms, 50,000 objects | 28 µs, 5 objects | 166 ms, 50,001 objects | 35 µs, 6 objects |

A miss now reads the head directory and one page per index level: four to
five objects of about 1 KB each up to a hundred thousand records. A hit adds
the blob that holds the record; its cost is authenticating that blob, about
1.5 ms for a full blob of 512 such records.

What the change costs, from the same workload:

| Archive | Prepare the next commit | Store and verify it | Lookup pages per archived record | Other objects per record | Restore audit |
| --- | --- | --- | --- | --- | --- |
| 102,400 records in 200 commits of 512 | 11.7 ms | 19.6 ms | 1.5 KB | 1.2 KB | 0.98 s |
| 9,990 records in 370 commits of 27 | 0.7 ms | 1.0 ms | 1.8 KB | 1.1 KB | 0.10 s |
| 10,000 records in 10,000 commits of 1 | 0.05 ms | 0.08 ms | 3.5 KB | 2.2 KB | 0.21 s |

The page bytes are everything the commits wrote, superseded pages included;
the head's index holds about 200 bytes per record, the rest may be deleted
under the retention rule above. The restore audit was already linear in the
archive (it authenticated the whole chain, which is what a miss used to cost)
and now also rebuilds the index, about twice the chain walk.

### What can still fail a boundary

- A package archive object that the store cannot serve, or serves forged,
  oversized, or malformed, while a boundary looks a key up: the boundary
  fails and settles once the store serves the object. It is never taken for
  a miss.
- An archive commit or acknowledgement packet that decodes but is forged, as
  before: the plugins refuse it by failing the boundary. The enqueue and
  finalize helpers never produce one; another plugin that schedules these
  internal packet types can.
- A plugin root past its receipt or handle bound, which the enqueue helpers
  now keep their commits from reaching (below); commits another plugin
  schedules are not counted.

### Fixed beside the index

Review of this package found three ways the two reference plugins could stop
a run, all older than it:

- **A reused operation ID.** Admission refuses an operation ID reused with
  other input, but two such commands admitted before either settles are both
  queued. The economy plugin failed the boundary that settled the second
  (`IdempotencyConflict`) whether the first one's outcome was hot or archived,
  and the force-supply plugin did when it was archived. Both now skip the
  second command and emit their `operation_rejected` event saying so; a
  repeat with the settled input stays a silent no-op.
- **Two economy changes in one boundary.** Every economy packet that changes
  the root advances its runtime revision, so a later command of the boundary
  that expected the earlier revision is stale. The root is written once per
  boundary, though, and its revision must equal its record version, so a
  boundary that settled two such packets (two operations, an operation and an
  archive commit, or two archive commits) failed its own validation. The
  revision now ends one past the record version the boundary started from,
  whatever the number of packets; the stale check within the boundary is
  unchanged.
- **The receipt and handle bounds.** Each plugin root keeps every
  maintenance receipt (at most `MAX_PACKAGE_ARCHIVE_MAINTENANCE_RECEIPTS`,
  8,192) and the retention handle of each settled commit until the host
  acknowledges it (at most `MAX_PACKAGE_ARCHIVE_RETENTION_HANDLES`, 64); a
  root above either does not validate, so the boundary that settled the
  commit failed for good. `enqueue_force_archive` and
  `enqueue_economy_archive` now refuse, on the host and before anything is
  queued, a commit that the root and the commits of the same plugin already
  queued (counted from the run's ingress queue) leave no room for
  (`package_archive_commit_room`, an `archive_backpressure` error). The check
  is conservative: a queued acknowledgement frees nothing in it, and a stale
  duplicate counts as a handle.

Durable tests: `crates/integrations/canwu-economy-reference/tests/economy_operation_id_reuse.rs`
(`a_reused_operation_id_with_other_input_is_skipped_and_the_run_continues`)
and `crates/integrations/canwu-economy-reference/tests/economy_archive_commit_room.rs`
(`an_archive_commit_the_root_has_no_room_for_is_refused_before_it_is_queued`).

### Known limits

- **A run makes at most 8,192 archive commits per reference plugin.**
  Receipts never leave the root, so after that many settled commits, applied
  or stale, the enqueue helpers refuse every further commit and the root
  keeps its hot history. The economy root then fills to its 8,192 hot
  operation outcomes (the default `max_operation_outcomes`) and the run stops
  there unless the follow-up lands: folding acknowledged receipts into a
  digest chain, as production now retires its receipts.
- **Superseded lookup pages are reclaimed only from the batch's list.**
  `superseded_lookup_pages` names the pages a commit's index no longer
  reaches; a host application that wants to delete them keeps that list for
  each batch. Unlike the resource and production archives, the package
  archive has no helper that recomputes the pages a live run reads, so a host
  that lost the lists can only keep every page, or mark by reachability,
  which keeps the pages of older indexes the store still holds.

## Seal-stable reference validation

### Problem

`validate_ming_fiscal_reference` is the host-side semantic check of the Ming
fiscal reference integration. The integration runs it when it creates,
restores, or replays a run, and after its sample cycle. It took `&Canwu` and
read retained evidence:

- each execution receipt's adapter evidence through `domain_record_version`
  and `evidence_time`;
- each assessment's commutation quote through `domain_record_version`, which
  returns `None` for a superseded version whose establishing boundary a seal
  archived;
- each audit's evidence through `evidence_exists`, which after a seal is
  false for an identity whose receipt the seal did not keep.

A sealed run is a `CompactedCanwu`, which the validator could not take at
all. Had it been given the sealed run's reads, it would have refused a valid
run whose quote had been superseded, or whose audit cited sealed evidence,
while the exact replay of the same run passed.

### Decision

The validator reads only state a seal cannot change, through the host
trait `canwu_api::CanwuRun`:

- `CanwuRun`, implemented for `Canwu` and `CompactedCanwu`, gains
  `replay_stable_domain_record_version` and `evidence_cursor` (the journal
  counters, sealed and retained together). The validator reads only those,
  `domain_record` (committed current records), and
  `current_domain_record_version` (the current-version index); it never
  reads `domain_record_version_evidence_exists`, whose answer a seal can
  change. `validate_ming_fiscal_reference<R: CanwuRun + ?Sized>(&R)` takes
  either handle; existing callers that pass `&Canwu` compile unchanged.
- `canwu-sim` gains the host form of the boundary read
  `SimulationView::replay_stable_domain_record_version`: on `Simulation`,
  `CompactedSimulation`, `Canwu`, and `CompactedCanwu`, it returns the body
  and establishment time of a record's committed current version or of an
  initial-scenario version, and `None` for any other version even while it is
  retained.
- **Receipts.** Each cited adapter evidence version must resolve through
  `replay_stable_domain_record_version`. Adapter evidence records are
  create-only, so the version a receipt cites stays its record's current
  version; its body is matched against the receipt and request as before, and
  its establishment time must not precede the request.
- **Commutation quotes and record-version audit evidence.** The version must
  have been established by the run: it resolves through
  `replay_stable_domain_record_version`, or it was established by a boundary
  the run settled (its id is at most the boundary count) and, while its
  record exists, is below the record's current version and was established
  no later than the boundary that established the current version (both read
  from the current-version index, which no seal changes). The body of a
  superseded quote is not read: the fiscal plugin read it, through the same
  replay-stable read, when it admitted the assessment, and the fiscal state
  declares the quotes and receipt evidence of its hot books as
  payload-required dependencies, which a seal and a restore keep provable.
- The validator checks the hot books the fiscal state holds, as it checked
  the whole state before the fiscal archive existed. A book the fiscal
  archive took out of the state is not revisited; authenticating it is the
  fiscal archive's audit (`canwu_fiscal::audit_fiscal_archive`), which needs
  the host's archive store.
- **Other audit evidence.** An event, command, command attempt, ingress,
  boundary, or random draw must have an identity the run issued: at most its
  journal's count in `evidence_cursor`. Journal identities are contiguous from
  one, so this is exactly the answer exact replay gives.

Rejected alternatives:

- *Read the superseded quote's body from the sealed segments.* The validator
  would need the host's evidence store at every restore, and a run whose
  store lost a segment would fail validation where its replay passes.
- *Require every cited version to be current.* A quote is a version of a
  record that keeps changing; the run would fail validation as soon as the
  quoted record moved on, in every mode alike but wrongly.
- *Declare audit evidence as identity dependencies in the fiscal state.* That
  changes `canwu-fiscal`'s persisted state and seal behavior to serve a
  reference validator; counters already answer the question exactly for
  journal identities.

### Invariants

- The validator gives the same answer on a sealed run, an unsealed fork of
  it, a restore from its sealed segments, and its exact replay: every read is
  of committed current state, a current or initial-scenario version, or a
  counter, and all four hold those identically.
- A receipt is still matched against the full body of the evidence it
  cites.
- What the validator no longer checks: that a superseded quote or a
  superseded record-version audit citation names the exact change index of
  its establishing boundary. A citation of a version the record never
  reached, or by a boundary after its current version, is refused; a forged
  change index within those limits is accepted in every mode alike, where
  exact replay used to refuse it. The fiscal plugin admits only quotes it
  could read at admission; it bounds, but does not check, audit evidence.

### Persistence, replay, and formats

None change. `replay_stable_domain_record_version` on the host reads the
same state the boundary read does.

### Compatibility

Additive in `canwu-sim` and `canwu-api`: `replay_stable_domain_record_version`
on `Simulation`, `CompactedSimulation`, `Canwu`, and `CompactedCanwu`, and
`replay_stable_domain_record_version` and `evidence_cursor` on the
`CanwuRun` trait (new in 0.16.0 and implemented only by those two
handles). In the unpublished reference crate,
`validate_ming_fiscal_reference` is generic over `CanwuRun`. A run with a
receipt whose evidence is no longer its record's current version, or an
audit that cites evidence the run never issued, fails validation as before.

### What can still fail

The validator runs outside boundaries; it fails a creation, restore, or
replay helper of the reference integration, never a boundary. It refuses a
receipt whose evidence is not its record's current version, a quote or
record-version citation the run did not establish, and an audit citation of
a journal identity the run never issued. The fiscal plugin admits an audit
whatever evidence it cites (it only bounds the count), so a host can settle
an audit that this validator then refuses; that was already so.

## Replay profile and engine phases

### Problem

Two limits kept the boundary profile from answering where an exact replay
spends its time:

- An exact replay settles every boundary inside the replay call, on a runtime
  it creates with profiling off, and returns only when the last boundary has
  settled. A host could not turn profiling on in time, and the runtime keeps
  at most 1,024 profiles, so even a profile collected afterwards would drop
  the earliest boundaries of a long replay.
- A profile listed the wall time of each plugin handler and reported the
  rest only as `remaining_micros`: the simulation core's admission, staging,
  commits, hashing, and journal work in one number.

### Decision

**A profiled replay.**
`Simulation::replay_from_journal_with_boundary_profiles(plugins, journal,
archive_provider, on_profile)`, and the same function on `Canwu`, replays as
`replay_from_journal_with_archive_provider` does with profiling on from
before the first recorded boundary, and hands each boundary's
`BoundaryProfile` to `on_profile` as soon as the boundary settles, in journal
order. Nothing is queued in the runtime, so the profile cap does not limit a
long replay. The returned runtime has profiling off. A replay that does not
match its journal fails as an unprofiled one does; `on_profile` has then
received the profiles of the boundaries that settled before the mismatch was
found, the mismatching one included.

**Engine phases.** `BoundaryProfile` gains `engine`, one
`BoundaryEngineTiming { phase, elapsed_micros }` per `BoundaryEnginePhase`, in
the order of `BoundaryEnginePhase::ALL`, and `engine_micros(phase)`. The
phases are:

| Phase | What it times |
| --- | --- |
| `Admission` | checking the request, capturing the rollback point, admitting due ingress, queued commands, decisions, maintenance, and the scheduled actions due at the boundary, and applying what their handlers returned |
| `SystemDispatch` | preparing each due boundary system's view and context, and checking, staging, and recording its proposal |
| `Staging` | the work between settlement phases: record and component overlays, reservation allocation, and transition audits. When every record write of phase 7 is visible in the same boundary, the record store prepares the phase-9 commit while it builds the phase-7 overlay, so most of that boundary's record-write work shows here |
| `Commit` | applying staged directives to the state: record mutations, components, events, scheduled ingress, persons |
| `Knowledge` | staging and committing knowledge publications |
| `Journal` | committing random draws, random decisions, and transition manifests, and appending the boundary record and evidence |
| `Hashing` | the state hash, the boundary hash, and the checkpoint hash |

The profiler keeps a current engine phase. Settlement switches it at the
points where its work changes kind, and a handler call pauses it at the
clock reading the handler's time starts from and resumes it at the reading
the handler's time ends at. The handlers and the engine phases therefore
partition the settlement: their exact durations add up to the exact
duration of `total_micros`. Each listed value is rounded down to whole
microseconds, so their sum is at most `total_micros` and falls short of it
by less than one microsecond per listed value. `remaining_micros()` keeps its
meaning and equals the engine phases' sum within the same tolerance.

Rejected alternatives:

- *A replay option that leaves the profiles in the returned runtime.* The
  cap of 1,024 would drop the earliest boundaries of a long replay, and
  raising it would hold the profiles of the whole run in memory.
- *Turning profiling on in every replay.* A replay that nobody profiles would
  read the clock around every handler and phase.
- *Timing each engine step separately and listing what is left.* The
  totals would not add up to the boundary's time by construction, and the
  list would follow the code's structure instead of the work's kind.

### Invariants

- Wall time never enters the simulation. The profiled replay checks every
  regenerated boundary and the final checkpoint against the journal exactly
  as an unprofiled replay does, and returns the same runtime; the profiler
  stays outside authoritative state, evidence, snapshots, checkpoints, and
  journals, and no handler can read it.
- While profiling is off, settlement reads no clock: the phase switches do
  nothing without a boundary being timed.
- A boundary that fails leaves no profile, in a live run and in a replay.

### Persistence, replay, and formats

None change. The profile is a host diagnostic and is not persisted.

### Compatibility

Additive: `BoundaryEnginePhase` (non-exhaustive), `BoundaryEngineTiming`,
`BoundaryProfile::engine`, `BoundaryProfile::engine_micros`, and
`replay_from_journal_with_boundary_profiles` on `Simulation` and `Canwu`, all
re-exported by `canwu-api`. `BoundaryEnginePhase::ALL` is a slice, so a
later release can add a phase without changing its type. Code that builds a `BoundaryProfile` with a
struct literal must name `engine`; no Canwu code does.

### Measurements

A profile reads the clock once per engine-phase switch, besides the two
readings around each handler: about twenty more readings per boundary for a
run with a few boundary systems. Release build, one thread, a shared
machine: 4,000 daily boundaries of a plugin with four boundary systems in
phases 7, 8, 12, and 13, one of which emits an event; three rounds, mean
time per boundary from the host's clock. It ran on this change over
`release/0.16.0` at `f511463`, before the record store package landed. The
workload is not kept in the tree.

| Round | Unprofiled | Profiled | Profile total | Admission | System dispatch | Staging | Commit | Knowledge | Journal | Hashing |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 25.5 µs | 27.0 µs | 26.2 µs | 7.4 | 2.0 | 0.9 | 0.8 | 0.0 | 1.1 | 11.7 |
| 2 | 25.4 µs | 27.1 µs | 26.3 µs | 7.6 | 2.0 | 0.9 | 0.9 | 0.0 | 1.1 | 11.7 |
| 3 | 25.9 µs | 27.9 µs | 27.1 µs | 8.0 | 2.1 | 0.9 | 0.9 | 0.0 | 1.2 | 12.3 |

The engine phases are in microseconds per boundary. Profiling, handler
timings included, costs about 1.5 to 2 µs per boundary here, on a boundary
that does almost nothing, and nothing while it is off. The breakdown is what
the item asked for: the core's own time in this workload is mostly hashing
and admission, which `remaining_micros` reported as one number.

### What can still fail a boundary

Nothing new: profiling cannot fail a boundary, and the profiled replay fails
exactly where an unprofiled replay fails.
