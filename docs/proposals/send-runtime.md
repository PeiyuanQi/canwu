# A Send Runtime

Status: implemented in 0.16.0. This package closes downstream gap G-59 in
`canwu-sim`, `canwu-api`, `canwu-resource`, `canwu-production`,
`canwu-force-supply-reference`, and `canwu-economy-reference`. It changes
public signatures (a breaking change, allowed in 0.16.0) and no persisted
format, hash, or replay input.

The release entry will be in [Versioning and Persistence](../versioning.md).
The handles it changes are described in
[continuation checkpoint](continuation-checkpoint.md) (`CompactedCanwu` and
its parity with `Canwu`) and in the resource and production archive
proposals ([resource archive lookup and volume](resource-archive-lookup-and-volume.md),
[production archive volume](production-archive-volume.md)).

## A run can move to another thread

### Problem

A host keeps a run in a `Canwu` or a `CompactedCanwu`, and asks the run for a
copy with `fork()`. A host that saves while play continues wants to hand that
copy to a worker thread and write the save there. In 0.15.0 it cannot: the
runtime held the host's plugin archive provider as
`Rc<dyn PluginArchiveObjectProvider>`, so `Simulation`, `CompactedSimulation`,
`Canwu`, and `CompactedCanwu` were all `!Send`, and so was every fork. The
resource and production restore wrappers took their stores as
`Rc<dyn ResourceArchiveStore>` and `Rc<dyn ProductionArchiveStore>` and built
the run's provider from them, so a run restored through them was `!Send` for
the same reason. Everything else a run holds was already `Send + Sync`: ordered
maps, `im` structures of `Arc` entries, plain data, and plugin callbacks that
are `fn` pointers.

A host therefore either saved on the simulation thread, stalling play for the
length of the save, or had its worker restore the run again from the last save
on disk before writing the next one, which costs a restore per save and grows
with the campaign.

### Decision

**The run holds its provider as a shared, thread-safe object.** `Simulation`
holds `Arc<dyn PluginArchiveObjectProvider + Send + Sync>`, and every
operation that attaches a provider takes that type:

| Operation | 0.15.0 parameter | 0.16.0 parameter |
| --- | --- | --- |
| `Canwu::set_plugin_archive_object_provider` | `Rc<dyn PluginArchiveObjectProvider>` | `Arc<dyn PluginArchiveObjectProvider + Send + Sync>` |
| `CompactedCanwu::set_plugin_archive_object_provider` | as above | as above |
| `Canwu::replay_from_journal_with_archive_provider`, `Canwu::replay_from_journal_with_boundary_profiles` | as above | as above |
| `Simulation::set_plugin_archive_object_provider`, `CompactedSimulation::set_plugin_archive_object_provider`, `Simulation::replay_from_journal_with_archive_provider`, `Simulation::replay_from_journal_with_boundary_profiles` | as above | as above |
| `canwu_resource::{from_resource_snapshot_json_with_archive_store, from_resource_checkpoint_journal_with_archive_store, replay_resource_from_journal_with_archive_store}` | `Rc<dyn ResourceArchiveStore>` | `Arc<dyn ResourceArchiveStore + Send + Sync>` |
| `canwu_production::{from_production_snapshot_json_with_archives, from_production_checkpoint_journal_with_archives, replay_production_from_journal_with_archives}` | `Rc<dyn ProductionArchiveStore>`, `Rc<dyn ResourceArchiveStore>` | `Arc<dyn ProductionArchiveStore + Send + Sync>`, `Arc<dyn ResourceArchiveStore + Send + Sync>` |
| `canwu_force_supply_reference::{from_force_supply_snapshot_json, from_force_supply_checkpoint_journal, replay_force_supply_from_journal}` | `Rc<dyn PluginArchiveObjectProvider>` | `Arc<dyn PluginArchiveObjectProvider + Send + Sync>` |

With that one field changed, `Simulation`, `CompactedSimulation`, `Canwu`, and
`CompactedCanwu` are `Send`. A fork shares the provider through the `Arc`, as
it shared it through the `Rc` before, so a fork on a worker thread loads from
the same store as the original on its own thread. The rustdoc of each handle
says it is `Send` and that `Sync` is not promised; today the handles are also
`Sync`, because nothing they hold has interior mutability, but a later release
may add a cache that is not.

**The bound is on the holder, not on the trait.** The provider and store
traits do not gain `Send + Sync` supertraits. Every place that keeps a provider
or store beyond one call spells the bound (`Arc<dyn Trait + Send + Sync>`), and
every place that lends one for a call keeps taking `&dyn Trait`. A supertrait
was rejected for two reasons:

- `SimulationView` implements `PluginArchiveObjectProvider`, and package
  helpers pass the view as a provider during settlement; the engine's
  view-backed store adapters in `canwu-resource`, `canwu-production`, and
  `canwu-economy-reference` implement `ResourceArchiveStore` over a borrowed
  view in the same way. The view keeps the boundary's random session in a
  `RefCell`, so it is not `Sync`. A supertrait would require a lock on every
  keyed draw, or the removal of a public impl, for objects that never leave
  the settling thread.
- A host store lent for one call (`prepare_resource_archive`,
  `store_and_verify`, `validate_*_with_archive_store`, the decision, state
  page, evidence segment, legal, fiscal, and package archive operations) never
  leaves the host's thread, and a supertrait would force every such store to be
  thread-safe for nothing.

So the requirement is exactly where sharing happens: a store or provider the
run keeps is `Send + Sync`; one it borrows for a call need not be.

**The economy reference composition follows.** `GrainHarness` (in
`canwu-economy-reference`) keeps its archive store shared by the run, its
forks, and itself. The store now keeps its maps behind `Mutex`es and is held
as an `Arc`, so `GrainHarness` is `Send`, and its hidden test accessor
`archive_store()` returns `Arc<dyn PackageArchiveStore + Send + Sync>`
instead of `Rc<dyn PackageArchiveStore>`.

**No new operation.** A `fork_detached()` that returns a `Send` copy without
the provider was the alternative the downstream row offered. It was rejected:
a fork without the provider cannot settle a boundary that reads a package's
cold archive, so a worker could save it but not advance or validate it, and
the rule for when a detached fork is safe would fall on every host.

### Inventory

Every `Rc`, `RefCell`, `Cell`, and other `!Send` type in the workspace was
listed and placed in one of three groups.

| Where | What | Group |
| --- | --- | --- |
| `canwu-sim` `Simulation::plugin_archive_provider` | `Rc<dyn PluginArchiveObjectProvider>` | held by every handle: now `Arc<dyn … + Send + Sync>` |
| `canwu-resource` `query.rs`, `canwu-production` `query.rs` | `Rc` stores inside the provider adapters the restore wrappers attach | held through the provider: now `Arc<dyn … + Send + Sync>` |
| `canwu-economy-reference` `GrainHarness` | `Rc<GrainArchiveStore>` of `RefCell` maps | held by a public handle: now `Arc` of `Mutex` maps |
| `canwu-sim` `SimulationView::random_session` | `RefCell<RandomSession>` | borrowed view of one boundary system call; never held by a handle |
| `canwu-fiscal` `settle.rs` `Settlement::office_base_bytes` | `RefCell<BTreeMap<…>>` | built and dropped inside one fiscal boundary system call; never held by a handle |
| `canwu-resource` `root.rs` `ResourceRootReader` | borrows a `SimulationView` | lives for one boundary system call; never held by a handle |
| `canwu-resource` `plugin.rs` `archive_reads::READS` | thread-local `RefCell`, debug builds only | the reads of one resource settlement, recorded and replayed within that one call on one thread; every recording starts by resetting it |
| `canwu-sim` `records.rs` `ENCODE_BUFFER` | thread-local `RefCell<Vec<u8>>` | scratch buffer, cleared before each use; holds no run state |
| `canwu-sim` `random.rs` `LEDGER_BUILDS` | thread-local `Cell` | `#[cfg(test)]` only |
| `canwu-sim` `persistence.rs` `Format8ScalePageStore`, `canwu-decision` `DecisionScaleArchive`, `canwu-law` `LegalTemporalScaleProvider` and `AppendJsonStore` | `RefCell` stores | private measurement stores built and dropped inside one scale-metric call |
| `canwu-economy-reference` and `canwu-force-supply-reference` test modules, crate tests, the performance harness | `RefCell` and `Rc` test stores | test fixtures: a store a run holds now uses `Arc` with `RwLock` or atomics, and its file was converted whole; a store only lent by reference in other files keeps `RefCell` |

No thread-local holds run state: a run moved to another thread finds nothing
it left behind on the first one. The behaviour test below would catch a later
thread-local cache that did.

### Invariants

1. `Simulation`, `CompactedSimulation`, `Canwu`, and `CompactedCanwu` are
   `Send`; so are `GrainHarness` and what a worker hands back
   (`CheckpointJournal`, `ContinuationCheckpoint`, `EvidenceJournalSegment`).
2. A fork shares the provider of the run it was taken from; it never runs
   without it.
3. A run advanced, saved, sealed, or checkpointed on another thread writes
   byte-equal receipts, full saves, sealed segments, and continuation
   checkpoints to the same steps on the original thread, and a save written on
   one thread restores and continues on another in lockstep.
4. Nothing a boundary decides depends on the thread it runs on.

### Persistence, replay, and formats

None. The provider is executable host context and was never serialized; no
snapshot field, journal, checkpoint, paged checkpoint, continuation checkpoint,
semantic hash, descriptor, schema version, or commitment changes, and no
expected value in a format or tamper test changed. Exact replay, restore, and
sealing behave as before.

### Compatibility

Breaking at compile time only, for hosts that attach a provider or call the
`_with_archive_store` and `_with_archives` restores:

- Pass `Arc::new(store)` where the code passed `Rc::new(store)`; an
  `Rc<dyn …>` variable becomes `Arc<dyn … + Send + Sync>`.
- A store type that a run holds must be `Send + Sync`. A store with
  `RefCell` maps changes them to `Mutex` or `RwLock` (and `Cell` counters to
  atomics). A store only lent by reference needs no change.
- A host that wraps a run as another run's provider (both handles implement
  `PluginArchiveObjectProvider`) keeps working, because the handles are `Sync`
  today; such a host relies on more than this release promises.

Behaviour, persisted bytes, and hashes are unchanged.

### Host guidance

A host may save on a worker thread: take `fork()` on the simulation thread,
which shares the run's persistent structures, move the fork to the worker,
and write the save there; the simulation thread keeps settling the original.

- **A save-only fork never loads from the provider.** `checkpoint_journal_json`,
  `continuation_checkpoint`, and `seal_evidence` read only the run's own
  state and evidence, and `prepare_paged_checkpoint` reads only the state page
  provider passed to it. A fork that is also advanced, restored, or validated
  on the worker does load, at the same time as the original.
- **One answer per object.** A provider must give every run the same answer
  for the same `(namespace, object_id)`, whatever the order or concurrency of
  the calls. While it is busy, for example while another thread writes, it
  blocks; it does not return `None` or an error, which a boundary would take
  for a missing or unreadable object. The first-party archive objects are
  content-addressed and stored before an archive commit names them, so loads
  of committed objects need no coordination beyond the store's own locking.
- **Keep what a live fork reads.** Deleting the lookup pages a resource
  archive commit superseded gives up every earlier point of the run that reads
  them, including a fork whose head precedes the commit
  ([resource archive persistence](resource-archive-lookup-and-volume.md#persistence-replay-and-formats)).
  While a fork is live on a worker, the host does not delete pages that fork
  still reads; the same holds for any package archive object a fork's state
  still names.
- **A fork's seal is the fork's.** Sealing a fork seals only the fork: the
  original keeps its evidence and its own seal history, and the segments the
  fork returns belong to the fork's save, not to the original's segment chain.
- Host-side archive writes (`store_and_verify`, retention persistence and
  finalisation) stay with the thread that drives the run they belong to.

### What can still fail a boundary

Nothing new. The changed operations are attach and restore calls; the bound is
checked at compile time and none of them gains a runtime refusal. A provider
that fails a load fails the boundaries it failed before (a failed resource
archive lookup, for example, still fails its boundary). A provider that blocks,
for example on a lock a host thread holds, delays the boundary that loads from
it without failing it; a poisoned lock in a host store is the host's to
recover from.

### Proof

`crates/api/canwu-api/tests/runtime_thread_transfer.rs`:

- `runtime_handles_are_send` asserts `Send` for the four handles and for what
  a worker returns, and that every attach and replay operation of `Canwu`
  and `CompactedCanwu` (and `CompactedSimulation`'s attach) takes an `Arc<P>`
  with `P: Send + Sync`. It does not compile on the base.
- `a_fork_saved_on_another_thread_equals_the_same_steps_here`: a run whose
  boundary system loads an archive object from the attached provider and makes
  a keyed draw each day is forked; the fork settles ten days, writes a full
  save, and writes a continuation checkpoint on a worker thread while the
  original does the same on the test thread, both loading from the one
  provider. The provider records loads on both threads; receipts, saves,
  checkpoints, and hashes are equal; the fork comes back and keeps step with
  the original; the worker's save restores and continues.
- `a_continuation_checkpoint_written_on_another_thread_continues_here`: the
  same with a compact fork that seals on the worker and writes its
  continuation checkpoint there while the original does the same on the test
  thread; segment, checkpoint, and hash equal the original's, and a run
  restored from the worker's checkpoint continues in lockstep with the
  original for eight more days.

`canwu-economy-reference`'s
`snapshot_checkpoint_journal_and_fork_continue_identically` now advances its
fork, with the archive store it shares, on another thread each month.

### Measurements

The change touches two hot-path operations: `fork()` now increments an atomic
reference count instead of a plain one, and each boundary system's view still
borrows the provider as `&dyn PluginArchiveObjectProvider` (the same pointer
dereference). Nothing allocates differently: `Arc::new(())` and `Rc::new(())`
both make one 16-byte allocation when a run is created or restored.

Workload: the existing `keyed-draw-profile` of the performance harness, at 8
due systems and 0, 1,000, and 10,000 retained keyed draws, which measures a
live boundary on a fork, a sealed boundary, `CompactedCanwu::fork`, the
compact checkpoints and seal, and the two restores. Before is
`release/0.16.0` at `9e5519c`, exported with `git archive`; after is this
branch on the same base. Both are release builds of the same harness source
with Rust 1.99.0 on a 16-thread Windows x86_64 machine on which other builds
were running.

Allocations (one run each, `--features allocation-counting`): every case at
every scale makes the same number of allocations and the same bytes before and
after, and the built run holds the same heap (137,418, 1,302,501, and
14,873,028 bytes): [before](../../benchmarks/baselines/2026-10-15-send-runtime-before-allocations.json),
[after](../../benchmarks/baselines/2026-10-15-send-runtime-after-allocations.json).

Elapsed (five runs of each build, alternating; each file merges the samples of
its five runs, 155 per boundary or fork case and 15 per restore):
[before](../../benchmarks/baselines/2026-10-15-send-runtime-before-elapsed.json),
[after](../../benchmarks/baselines/2026-10-15-send-runtime-after-elapsed.json).

| Case (median) | Retained draws | Before | After |
| --- | ---: | ---: | ---: |
| `live_boundary` (fork, then one boundary) | 0 | 82.0 µs | 84.5 µs |
| `live_boundary` | 1,000 | 86.5 µs | 85.7 µs |
| `live_boundary` | 10,000 | 104.4 µs | 103.5 µs |
| `compact_fork` | 1,000 | 8.9 µs | 8.9 µs |
| `compact_fork` | 10,000 | 13.1 µs | 9.3 µs |
| `sealed_boundary` | 10,000 | 93.1 µs | 92.4 µs |
| `checkpoint_journal_restore` | 10,000 | 54.4 ms | 54.1 ms |
| `snapshot_restore` | 1,000 | 34.7 ms | 36.2 ms |
| `snapshot_restore` | 10,000 | 445.2 ms | 483.3 ms |

Every difference is inside the run-to-run spread of one build: the medians of
single runs of the before binary ranged from 9.3 to 15.1 µs for
`compact_fork` at 10,000 draws, and single `snapshot_restore` samples at
10,000 draws ranged from 417 to 473 ms before and from 429 to 550 ms after,
on a path this change does not touch apart from the one allocation above. A second comparison of `snapshot_restore` alone at 1,000
draws, six alternating runs of 11 samples per build, gave pooled medians of
47.5 ms before and 46.0 ms after (minimums 28.1 and 28.8 ms). No case is
measurably slower.

The in-memory stores of `production-archive-profile` and
`resource-root-profile` are attached to runs, so they now keep their maps in
an `RwLock` (and the production profile its load counters in atomics). Their
per-load figures recorded from now on include one uncontended lock, which
earlier baselines of those profiles did not.
