# Keyed-Draw Index, Draw ID, and Boundary Profile

Status: implemented in 0.15.0. Three changes to random draws and settlement
timing in `canwu-sim`, reached through `canwu-api` (downstream gaps G-50,
G-51, and G-47). None changes a persisted format or a committed hash.

The release entry is in [Versioning and Persistence](../versioning.md). The
other 0.15.0 proposals are
[society settlement cadence and runtime revision](society-cadence-and-runtime-revision.md),
[same-instant supersession and unplaced persons](same-instant-supersession-and-unplaced-persons.md),
[resource archive lookup and volume](resource-archive-lookup-and-volume.md),
[single-step completion lease and runtime protected-floor install](resource-single-step-lease-and-floor-install.md),
and [fiscal throughput and authority](fiscal-throughput-and-authority.md).

## Keyed-draw index (G-50)

### Problem

An operation-keyed random draw is idempotent. Asking again for an address the
run has drawn returns the recorded value, and reusing the address with other
evidence, another bound, or another purpose is refused. To answer, a boundary
needs an index from each drawn address to what was drawn.

Up to 0.14.0 `settle_boundary` rebuilt that index at the start of every
boundary, from every retained draw record and every keyed reservation, and
copied it for each boundary system that was due. A boundary therefore cost time
in proportion to the keyed draws the run had ever made, multiplied by the due
systems, whether or not it drew anything. An application that draws by
operation key every simulated day slowed every later boundary of the run.

### Decision

The runtime keeps the index with its evidence, as a derived index, and never
rebuilds it while settling:

- A restore builds it once from the retained draw journal and the keyed
  reservations, which is everything a restore holds of it.
- A boundary works on a copy and gives each due system's random session a copy.
  The index is a persistent ordered map of shared entries (`im::OrdMap`), so a
  copy shares structure with its source, and adding an address copies one path
  of the tree.
- When the boundary commits its draws, its working copy becomes the kept
  index. A boundary that fails puts back the index it started with, together
  with the draw journal and the draw counter.
- A seal leaves the index as it is: the keyed reservation of a sealed draw
  records the same address, evidence, bound, purpose hash, value, and ID
  as the draw record it replaces.

Three alternatives were rejected. Persisting the index would add a field to
the snapshot and the commitment for data a restore can derive. A shared
ordinary map with an overlay for the boundary in flight needs two lookups per
draw and still needs a constant-time copy for rollback and `fork`. An index
keyed by a digest of the address would be smaller, but would replace an exact
comparison of addresses with a comparison of digests.

### Invariants

- Outside a boundary in flight, the kept index equals the index built from the
  retained draw journal and the keyed reservations. This holds after a commit,
  after a failed boundary, after a seal, after a restore from a snapshot, a
  checkpoint journal, or a paged checkpoint, after exact replay, and in a fork.
- Only boundary systems make operation-keyed draws, so only a boundary commit
  changes the index.
- Settling a boundary does no work in proportion to the number of drawn
  addresses. A lookup or an insertion is logarithmic in it. A boundary whose
  systems make no keyed draw leaves the very tree it started with, and a fork
  holds that tree too; a test in `canwu-sim` pins both.
- Loading still refuses a snapshot with a malformed operation-keyed draw or an
  address drawn twice. The rebuild also refuses a malformed keyed reservation
  and an address held twice, as an invalid snapshot.

### Persistence, replay, and formats

The index is not persisted and not committed. Snapshot, commitment,
checkpoint, paged-checkpoint, and evidence-journal formats, plugin descriptors,
and every hash are unchanged, and a restored or replayed run draws exactly as
before.

### What it costs, and what still grows

The index costs memory. The runtime holds one entry for each address the run
has drawn, for as long as it runs. In the measurement below an entry takes 522
bytes, which raises what the run holds for each retained keyed draw from 788
to 1,310 bytes at 100,000 draws; longer stream names, operation IDs, and
targets make an entry larger. At that size, an application that makes 100
operation-keyed draws per simulated day adds 36,500 entries, about 18 MiB, for
each simulated year, and nothing removes them. Up to 0.14.0 the runtime
allocated and dropped the same entries at every boundary. An entry stores its
purpose hash as 32 bytes; the keyed reservation keeps the hexadecimal form
that is persisted.

This change removes the per-draw cost from settlement only. These operations
still do work in proportion to every keyed draw the run has made, and are
left as follow-up:

- `checkpoint()` and a paged checkpoint clone, sort, and hash every keyed
  reservation, and walk every reservation and retained draw to list evidence
  dependencies.
- A seal extends, sorts again, and scans all keyed reservations.
- `fork()` copies the draw journal and the keyed reservations. It shares only
  the index.
- A restore builds the index from all draws. A snapshot restore builds it
  twice, once to validate and once to keep, and a checkpoint-journal restore
  four times, because it loads the reconstructed snapshot twice.

A host that checkpoints or seals at boundary cadence therefore still pays for
each keyed draw at that cadence.

### Measurement

`benchmarks/performance-harness` gained the `keyed-draw-profile` binary. It
keeps a number of operation-keyed draws in a run and then measures one Daily
boundary in which one, eight, or 32 boundary systems are due and each makes
one fresh operation-keyed draw: on a run that still retains every draw record
(`live_boundary`), and after the evidence was sealed, when each earlier draw
is a keyed reservation (`sealed_boundary`). It also measures loading the run
from its snapshot JSON (`snapshot_restore`) and, in its allocation build, the
bytes the built run holds (`run_heap_bytes`). The binary uses only the public
API. The 0.14.0 figures come from the same source built in an export of the
`v0.14.0` tag.

The recorded reports are in `benchmarks/baselines`, as
`2026-10-13-keyed-draw-index-{0.14.0,0.15.0}-{elapsed,allocations}.json`
(release builds, Rust 1.99.0, AMD Ryzen 7 7800X3D, Windows x86-64). Allocation
counts are exact and do not depend on machine load. Elapsed figures are
medians of 31 samples after 3 warm-up boundaries; other builds ran on the
machine during those runs, so they show the growth and are not a budget.

Allocations made by the measured boundary, with eight due systems. With
nothing retained there is nothing to seal, so the first row has no sealed
figures:

| Retained keyed draws | live, 0.14.0 | live, 0.15.0 | sealed, 0.14.0 | sealed, 0.15.0 |
| ---: | ---: | ---: | ---: | ---: |
| 0 | 1,801 | 992 | | |
| 1,000 | 69,195 | 999 | 68,355 | 999 |
| 10,000 | 675,813 | 1,007 | 648,855 | 1,009 |
| 100,000 | 6,741,273 | 1,015 | 6,453,837 | 1,017 |

Wall time of the same boundary:

| Retained keyed draws | live, 0.14.0 | live, 0.15.0 | sealed, 0.14.0 | sealed, 0.15.0 |
| ---: | ---: | ---: | ---: | ---: |
| 0 | 0.100 ms | 0.073 ms | | |
| 1,000 | 3.03 ms | 0.085 ms | 3.60 ms | 0.093 ms |
| 10,000 | 33.4 ms | 0.101 ms | 31.8 ms | 0.104 ms |
| 100,000 | 659 ms | 0.147 ms | 595 ms | 0.121 ms |

The `live` boundary with 10,000 retained draws and a varying number of due
systems:

| Due systems | time, 0.14.0 | time, 0.15.0 | allocations, 0.14.0 | allocations, 0.15.0 |
| ---: | ---: | ---: | ---: | ---: |
| 1 | 18.4 ms | 0.059 ms | 173,465 | 326 |
| 8 | 33.4 ms | 0.101 ms | 675,813 | 1,007 |
| 32 | 98.8 ms | 0.319 ms | 2,406,141 | 3,363 |

On 0.14.0 a boundary made about 10 allocations for each retained keyed draw
and 7 more for each retained draw and due system, and took about 1.6 to 1.8 µs
and 0.3 to 0.6 µs for them, in line with the figures the gap was filed with.
On 0.15.0 the boundary no longer follows the retained count: from none to
100,000 retained draws it makes 992 to 1,015 allocations and takes 0.07 ms to
0.15 ms, and each further due system adds about 97 allocations and 6 to 10 µs
whatever the run retains.

Loading the run from its snapshot, and the bytes the run holds once built
(both with eight due systems registered):

| Retained keyed draws | restore, 0.14.0 | restore, 0.15.0 | restore allocations, 0.14.0 | restore allocations, 0.15.0 | run heap, 0.14.0 | run heap, 0.15.0 |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 0 | 0.403 ms | 0.413 ms | 5,406 | 5,409 | 55,026 B | 56,618 B |
| 1,000 | 22.5 ms | 21.4 ms | 288,122 | 300,084 | 700,161 B | 1,221,701 B |
| 10,000 | 357 ms | 366 ms | 2,831,583 | 2,951,217 | 9,577,420 B | 14,792,228 B |
| 100,000 | 4.14 s | 4.75 s | 28,175,887 | 29,372,331 | 78,813,026 B | 131,038,330 B |

A restore makes 12 more allocations for each keyed draw, 4.2 % more than
before, to build the index it keeps. That count is the figure to rely on. The
recorded wall time of a restore, three samples a row on a loaded machine, is
3 % to 20 % higher in the six rows with 10,000 and 100,000 retained draws
across the three due-system counts, and scatters from 5 % lower to 63 %
higher in the smaller rows. The difference between the two run-heap columns,
522 bytes for each retained draw, is the index.

Reproduce with the two commands in `benchmarks/README.md`, which also says how
the 0.14.0 files were produced.

## Draw ID (G-51)

### Problem

`SimulationView::random_sample_for_operation` returns the stream, address,
bound, and value of an operation-keyed draw, but not the ID of the draw
evidence (`RandomDrawId`). A system that computes a record from a draw cannot
cite the draw in it, and a reader has to join the record to the draw journal by
address.

### Decision

`SimulationView::random_draw_for_operation` takes the same arguments and
returns `RandomOperationDraw { draw_id, sample }`.
`random_sample_for_operation` and `random_range_for_operation` are unchanged
and return the same sample and value.

- For an address drawn at an earlier boundary, `draw_id` is the ID of
  that draw. The index entry of a retained draw carries its record's ID,
  and the entry of a sealed draw carries the ID in its keyed reservation.
- For a fresh draw, `draw_id` is the ID the boundary commits the draw
  under. A boundary commits the draws of its systems in the order they were
  made, numbered from the run's next draw ID, and nothing else takes a
  draw ID while its systems run: core draws happen while the boundary
  admits its input, before the first system. A random session therefore
  numbers each draw as it makes it. At commit the boundary checks every
  operation-keyed draw against the ID it was handed out with, and fails
  if they differ.

The sample itself does not gain the field. `RandomSample` is serialized inside
`RandomDecisionResolution` and inside records that extensions persist, such as
the incident evidence of `canwu-production`, so a new field would change a
directive shape, a plugin state schema, and the hashes of existing runs.

### An exact retry after a seal

Up to 0.14.0 every operation-keyed request first required its operation
evidence to be retained, and only then consulted the index. After a seal an
exact retry of an address whose evidence had been sealed was refused, while
the unsealed run answered it: a sealed run and its exact replay disagreed.

All three entry points now consult the index first. A request that matches
the entry of its address (the same evidence reference, bound, and purpose) is
answered from the entry, whatever has become of that evidence, and a request
that differs from the entry is refused as a conflict. Only a fresh draw needs
its operation evidence to exist at the reader's cut, that is, to be retained
or to have been produced earlier in the same boundary.

One case remains. A system that draws an address for the first time after a
seal, citing evidence the seal archived, is refused, although an unsealed run
with the same history would draw. A system that must behave the same across a
seal cites, for a first draw, evidence that survives one: the initial-scenario
version of a record, or evidence newer than the seal. The refusal must fail
the boundary: a handler that swallows `InvalidRandomOperationEvidence` settles
a boundary that the run's own exact replay rejects, because the replay retains
the evidence and makes the draw. A run restored from its sealed segments
retains that evidence again as well, so it accepts the draw the live sealed
run refused.

Knowledge supersession has the matching read: `SimulationView::knowledge_heads`
finds a publisher's supersession targets in committed state instead of retained
evidence (see
[same-instant supersession and unplaced persons](same-instant-supersession-and-unplaced-persons.md#finding-the-records-to-supersede)).

### Invariants

- The ID names the committed draw. A boundary that fails commits neither
  the draw nor anything its systems proposed, and puts the draw counter back.
  Boundary system handlers are stateless function pointers, so an ID can
  only be persisted together with the draw it names.
- Exact replay settles the same boundaries in the same order and hands out the
  same IDs.
- An exact retry returns the ID of the first draw, inside the boundary
  and at any later boundary, before and after a seal. A sealed run, its
  unsealed twin, and the sealed run's exact replay return the same IDs and
  reach the same checkpoint hash.

### Persistence, replay, and formats

Nothing persisted changes. The ID returned for a sealed draw is the
`draw_id` its keyed reservation already recorded.

A draw made in the current boundary is not retained evidence until the
boundary commits. Its ID can be written into a record payload or an event at
once, and can be named among a record's identity-evidence dependencies, which
survives a checkpoint, a seal, and a restore. Until the next boundary,
`evidence_exists` is false for it, the positions the engine checks as
evidence (knowledge origins, evaluation-trace terms, person provenance) refuse
it, and it cannot be the operation evidence of another operation-keyed draw.

## Boundary profile (G-47)

### Problem

A host application can time a whole `settle_boundary` call, but settlement
does not report how that time divides among plugins, so a host cannot check a
plugin against a time budget.

### Decision

Profiling is opt-in. After `set_boundary_profiling(true)`, every boundary that
settles leaves one `BoundaryProfile`, and the host takes the profiles with
`take_boundary_profiles()`; `boundary_profiling()` reads the setting. All
three are on `Canwu` and on `CompactedCanwu`:

```rust
BoundaryProfileBatch { profiles, dropped }
BoundaryProfile { boundary_id, settled_at, total_micros, handlers }
BoundaryHandlerTiming { kind, phase, plugin, name, elapsed_micros }
```

`handlers` lists every plugin handler call the boundary made, in the order
the calls were made, with the wall time spent inside the handler. A boundary
runs three kinds of plugin handler, and each is an entry with its `kind`:

- `BoundarySystem`: a boundary system, with the phase it ran in;
- `CommandHandler`: the handler of a queued plugin command the boundary
  admitted;
- `EventSystem`: a synchronous event-driven plugin system that ran when such a
  command, or a scheduled action due at the boundary, emitted an event.

The last two run while the boundary admits its input, before the first phase,
so their `phase` is absent. `total_micros` is the wall time of the whole
settlement, `plugin_micros(plugin)` sums every handler of one plugin, and
`remaining_micros()` is what is left: the simulation core's own admission,
validation, commit, and hashing.

Owner-authorized maintenance participants are in no profile. They run when the
host schedules the maintenance, outside settlement; the boundary only applies
the commit they produced, which is the simulation core's work.

The profile comes from a separate reader and not with the boundary receipt.
`BoundaryReceipt` is a deterministic value: two runs of the same input produce
equal receipts, and hosts and tests compare them. A wall-time field would make
the receipts of equal runs differ. A reader also serves `advance_canonical`
and `step_canonical`, which settle boundaries without the host calling
`settle_boundary`.

### Invariants

- Wall time never enters the simulation. The profiler is held beside the
  runtime state, like the plugin archive provider. It is not part of
  authoritative state, the commitment, evidence, a snapshot, a checkpoint, or
  a replay journal; no handler can read it; and settlement makes no decision
  from it. A profiled run and an unprofiled run of the same input produce the
  same receipts, evidence, hashes, snapshot, and replay journal.
- It is off in a new, restored, replayed, or forked runtime, so a look-ahead
  fork or a fork made to prepare a seal collects nothing. While it is off,
  settlement reads no clock. Entering the compact runtime and sealing evidence
  keep the setting and the profiles not yet taken.
- A boundary that fails leaves no profile.
- An exact replay settles its boundaries inside the replay call, on a runtime
  that starts with profiling off, so a replay is not profiled. The runtime the
  replay returns can be profiled from then on.
- The runtime keeps at most `MAX_RETAINED_BOUNDARY_PROFILES` (1,024) profiles.
  When more boundaries settle before the host takes them, the oldest are
  dropped, and `BoundaryProfileBatch::dropped` reports how many since the last
  take.

## Compatibility

Every change to the public API is additive:
`SimulationView::random_draw_for_operation`, `RandomOperationDraw`,
`BoundaryProfile`, `BoundaryProfileBatch`, `BoundaryHandlerTiming`,
`BoundaryHandlerKind`, `MAX_RETAINED_BOUNDARY_PROFILES`, and
`set_boundary_profiling`, `boundary_profiling`, and `take_boundary_profiles`
on `Canwu`, `CompactedCanwu`, `Simulation`, and `CompactedSimulation`.

One behavior changes for existing code. An operation-keyed request for an
address the run has already drawn no longer depends on whether the supplied
evidence is retained: it returns the recorded draw when the request matches,
and `RandomOperationConflict` when it does not, where 0.14.0 returned
`InvalidRandomOperationEvidence` for evidence that was not retained. A request
on an undeclared stream or with a malformed address likewise reports that
error before the evidence is looked at.
