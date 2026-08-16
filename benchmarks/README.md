# Canwu performance evidence

Canwu records performance evidence before architectural optimization. The
harness in [`performance-harness`](performance-harness) is a standalone,
non-CI Rust binary so routine correctness checks do not turn noisy elapsed-time
measurements into merge gates.

The workload is deterministic. At every requested scale it creates a scenario
with growing world-entity counts and then grows plugin components, accepted
commands, persisted rejected attempts, events, settlement boundaries, and
scoped random draws together. It measures:

- end-to-end history construction;
- one accepted and one persisted rejected command;
- one empty and one populated boundary;
- snapshot creation and pretty JSON serialization;
- snapshot JSON loading plus full validation; and
- environment-bound exact replay;
- explicit live evidence sealing and hot-state release.

Elapsed time and allocation traffic are collected in separate release builds.
Elapsed mode uses Rust's default system allocator with no counting wrapper.
Allocation mode enables a thread-local counting allocator and does not report
wall time. Both reports keep raw samples and summary medians; setup and
post-measurement drops are outside each sample. Snapshot byte size and exact
history counts are recorded beside every scale. Allocation reports also record
the signed difference between bytes allocated and deallocated during each
operation. Separate live-archive cases measure the sealing API and destruction
of its returned segment, keeping API latency distinct from caller-owned archive
release.

Run it from the repository root with an optimized build:

```console
cargo run --release \
  --manifest-path benchmarks/performance-harness/Cargo.toml -- \
  --mode elapsed \
  --machine local-windows-x86_64 \
  --recorded-on 2026-08-16 \
  --output benchmarks/baselines/2026-08-16-architecture-elapsed.json

cargo run --release \
  --manifest-path benchmarks/performance-harness/Cargo.toml \
  --features allocation-counting -- \
  --mode allocations \
  --machine local-windows-x86_64 \
  --recorded-on 2026-08-16 \
  --output benchmarks/baselines/2026-08-16-architecture-allocations.json
```

PowerShell accepts the same arguments on one line. Use
`cargo fmt --manifest-path benchmarks/performance-harness/Cargo.toml -- --check`,
`cargo clippy --manifest-path benchmarks/performance-harness/Cargo.toml --all-targets --all-features -- -D warnings`,
`cargo test --manifest-path benchmarks/performance-harness/Cargo.toml`, and
`cargo test --manifest-path benchmarks/performance-harness/Cargo.toml --features allocation-counting`
when changing the harness. Sample counts must be positive odd integers so the
reported median is an observed sample.

Elapsed time is machine- and load-sensitive. Compare before and after results
only when the machine, compiler, build profile, scales, warmup, and sample counts
match. Allocation counts and byte requests are more stable, but still belong to
the recorded compiler and target. A baseline is evidence, not a universal
service-level objective.

## Recorded architecture baseline

The pre-optimization baseline consists of separate
[`elapsed`](baselines/2026-08-16-architecture-elapsed.json) and
[`allocation`](baselines/2026-08-16-architecture-allocations.json) reports.
It was recorded on Windows x86-64 with Rust 1.97.1 in release mode at commit
`1a5e020fa86dc819a247d55dac621af1fb501f1e`. The working tree also contained
uncommitted benchmark and release-metadata work, which the report records
explicitly; all engine `src` directories were clean. Each artifact embeds Git
blob hashes for the harness, its lockfile, root and crate manifests, every
first-party Rust source file, and available toolchain/configuration files, plus
the exact feature and build-flag environment used for that pass.

| History scale | World entities | Components | Commands | Rejected attempts | Events | Boundaries | Draws | Snapshot | Growth median | Load + validate median | Exact replay median |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 8 | 25 | 8 | 8 | 8 | 24 | 8 | 8 | 56,443 B | 1.717 ms | 0.355 ms | 2.220 ms |
| 32 | 97 | 32 | 32 | 32 | 96 | 32 | 32 | 201,824 B | 18.441 ms | 1.105 ms | 23.480 ms |
| 128 | 385 | 128 | 128 | 128 | 384 | 128 | 128 | 786,568 B | 239.522 ms | 3.638 ms | 335.063 ms |
| 512 | 1,537 | 512 | 512 | 512 | 1,536 | 512 | 512 | 3,137,566 B | 4,030.170 ms | 17.663 ms | 5,758.903 ms |

## State-revision milestone comparison

The persisted authoritative-revision milestone was measured with the same
machine, toolchain, profile, scales, warmup, and sample counts. Its separate
[`elapsed`](baselines/2026-08-16-state-revision-elapsed.json) and
[`allocation`](baselines/2026-08-16-state-revision-allocations.json) artifacts
embed the exact source fingerprints used for the run.

At scale 512, the semantic persistence change did not alter the allocation
complexity of any individual operation. Accepted/rejected commands, empty and
populated boundaries, snapshot creation/serialization/loading, and exact replay
all retained their prior allocation-operation counts; end-to-end growth added
8 operations and exact replay added 11. The snapshot grew by 835 bytes
(3,137,566 to 3,138,401 bytes, about 0.03%) for the revision contract and
checkpoint data.

| Case at scale 512 | Baseline median | Revision median | Change | Baseline allocation ops | Revision allocation ops |
| --- | ---: | ---: | ---: | ---: | ---: |
| History growth | 4,030.170 ms | 4,048.313 ms | +0.5% | 45,551,539 | 45,551,547 |
| Accepted command | 3.095 ms | 3.113 ms | +0.6% | 32,032 | 32,032 |
| Persisted rejection | 2.975 ms | 3.031 ms | +1.9% | 32,021 | 32,021 |
| Empty boundary | 7.953 ms | 8.242 ms | +3.6% | 69,037 | 69,037 |
| Populated boundary | 7.849 ms | 8.038 ms | +2.4% | 69,149 | 69,149 |
| Snapshot creation | 0.695 ms | 0.701 ms | +0.9% | 25,208 | 25,208 |
| Pretty serialization | 3.603 ms | 3.607 ms | +0.1% | 16 | 16 |
| Load and validate | 17.663 ms | 17.313 ms | -2.0% | 97,442 | 97,442 |
| Exact replay | 5,758.903 ms | 5,804.774 ms | +0.8% | 67,297,025 | 67,297,036 |

Elapsed changes at this size remain within the noise expected from local wall
time measurements. The unchanged allocation curves show that this correctness
milestone neither resolves nor materially worsens the quadratic history-growth
and replay bottlenecks established by the baseline.

## Admission-cursor milestone comparison

The persisted admission-cursor milestone is recorded in separate
[`elapsed`](baselines/2026-08-16-admission-cursors-elapsed.json) and
[`allocation`](baselines/2026-08-16-admission-cursors-allocations.json) reports.
It replaces each settlement's reconstruction of three full historical sets with
monotonic attempt, accepted-command, and event cursors, then reads only the new
journal tails. The committed state-revision reports above are its before
baseline.

At scale 512, both empty and populated boundaries eliminate 312 allocation
operations and 109,656 allocated bytes. Across 512 boundaries, history growth
and exact replay each eliminate 86,358 allocation operations and 30,338,480
allocated bytes. The persisted cursor fields add 139 bytes to the 3.14 MB
snapshot (about 0.004%).

| Case at scale 512 | Before median | Cursor median | Change | Before allocation ops | Cursor allocation ops |
| --- | ---: | ---: | ---: | ---: | ---: |
| History growth | 4,048.313 ms | 4,037.455 ms | -0.3% | 45,551,547 | 45,465,189 |
| Accepted command | 3.113 ms | 3.074 ms | -1.3% | 32,032 | 32,032 |
| Persisted rejection | 3.031 ms | 2.939 ms | -3.0% | 32,021 | 32,021 |
| Empty boundary | 8.242 ms | 7.830 ms | -5.0% | 69,037 | 68,725 |
| Populated boundary | 8.038 ms | 8.044 ms | +0.1% | 69,149 | 68,837 |
| Snapshot creation | 0.701 ms | 0.691 ms | -1.4% | 25,208 | 25,208 |
| Pretty serialization | 3.607 ms | 3.758 ms | +4.2% | 16 | 16 |
| Load and validate | 17.313 ms | 17.480 ms | +1.0% | 97,442 | 97,442 |
| Exact replay | 5,804.774 ms | 5,762.994 ms | -0.7% | 67,297,036 | 67,210,678 |

The deterministic allocation reduction proves the removed scan. End-to-end
wall time remains approximately quadratic because full-state transaction clones,
checkpoint hashing, and retained-history serialization still dominate; those
are separate planned milestones.

## Runtime-evidence ownership comparison

The internal runtime-evidence separation is recorded in
[`elapsed`](baselines/2026-08-16-runtime-evidence-elapsed.json) and
[`allocation`](baselines/2026-08-16-runtime-evidence-allocations.json) reports,
using the admission-cursor milestone as its before baseline. This refactor moves
all append-only journals behind one `RuntimeEvidence` owner without changing the
flat snapshot/replay wire shape or runtime algorithms.

All operation allocation counts, allocated bytes, and snapshot sizes are
identical at every measured scale. At scale 512, elapsed medians range from
-3.8% to +2.9%; history growth is +0.5% and exact replay is -0.5%. These are
consistent with local wall-time noise and confirm that the ownership boundary is
behavior- and cost-neutral before segmented storage is introduced.

## Runtime-partition ownership comparison

The scheduler, counter, and persistence-metadata ownership refactor is recorded
in separate
[`elapsed`](baselines/2026-08-16-runtime-partitions-elapsed.json) and
[`allocation`](baselines/2026-08-16-runtime-partitions-allocations.json) reports,
using the runtime-evidence reports as its before baseline. It adds private
runtime owners without changing snapshot/replay fields, checkpoint inputs, or
algorithms.

Allocation operations, allocated bytes, and snapshot sizes are identical at
every scale. At scale 512 the 3,138,540-byte snapshot is unchanged; history
growth is +3.8%, exact replay is +3.9%, and individual operations range from
-3.1% to +5.0%. With identical allocation curves and no algorithmic change,
these elapsed differences remain consistent with local measurement noise.

## Runtime-current-state ownership comparison

The mutable-state ownership refactor is recorded in separate
[`elapsed`](baselines/2026-08-16-runtime-current-state-elapsed.json) and
[`allocation`](baselines/2026-08-16-runtime-current-state-allocations.json)
reports, using the runtime-partition reports as its before baseline. It moves
world, knowledge, plugin/domain state, and random-stream positions behind one
private owner without changing public persistence or runtime algorithms.

Allocation operations, allocated bytes, and snapshot sizes are again identical
at every scale. At scale 512 the snapshot remains 3,138,540 bytes; elapsed
medians range from -4.9% to +3.2%, with history growth at -4.9% and exact replay
at -3.7%. These are treated as local timing variance, not an optimization gain.

## Commitment-roots foundation comparison

The versioned domain-root and checkpoint-v4 foundation is recorded in separate
[`elapsed`](baselines/2026-08-16-commitment-roots-foundation-elapsed.json) and
[`allocation`](baselines/2026-08-16-commitment-roots-foundation-allocations.json)
reports, using the runtime-current-state reports as its before baseline. This
milestone establishes migration and tamper-evident domain roots; it deliberately
recomputes them from canonical domain material before the incremental-update
optimization.

At scale 512, the snapshot grows by 1,084 bytes (3,138,540 to 3,139,624 bytes).
Splitting one large JSON hash into sorted domain leaves reduces allocated bytes
by 76,732,594 for history growth and 113,061,982 for exact replay, but adds
4,486,642 and 6,727,784 allocation operations respectively. Snapshot loading
adds 12,767 operations and 2,729,571 bytes because it independently recomputes
and validates every root. Elapsed medians remain noisy: history growth is -1.3%,
exact replay -1.2%, and load/validate +6.7%. The increased operation counts are
the concrete baseline the next incremental-root milestone must remove.

## Incremental journal-commitment comparison

The append-only commitment-cache optimization is recorded in separate
[`elapsed`](baselines/2026-08-16-incremental-journal-commitments-elapsed.json)
and
[`allocation`](baselines/2026-08-16-incremental-journal-commitments-allocations.json)
reports, using the commitment-roots foundation as its before baseline. It keeps
checkpoint-v4 bytes unchanged while appending only new commands, attempts,
events, ingress records, and random draws to cloneable hash state. Snapshot
loading still rebuilds every root independently.

At scale 512, history growth falls from 3,951.610 ms to 2,868.295 ms (-27.4%)
and exact replay from 5,664.912 ms to 3,995.156 ms (-29.5%). Allocated bytes
fall by 3,284,320,984 for growth and 4,925,348,144 for replay; accepted and
rejected commands each eliminate about 2.93 MB of allocation traffic, and empty
or populated boundaries eliminate about 5.86 MB. Allocation-operation counts
fall modestly because full transaction clones still dominate. Snapshot size is
unchanged at 3,139,624 bytes, while load/validate remains an intentionally
uncached full proof (+1.6% elapsed in this run).

## Mutable-domain commitment-cache comparison

The next cache slice is recorded in separate
[`elapsed`](baselines/2026-08-16-mutable-domain-commitment-cache-elapsed.json)
and
[`allocation`](baselines/2026-08-16-mutable-domain-commitment-cache-allocations.json)
reports, using the incremental journal-commitment milestone as its before
baseline. Checkpoint-v4 bytes and commitment-format 1 are unchanged. The runtime
retains canonical roots for unchanged mutable domains and invalidates them at
their private mutation boundaries; loading still performs a fully uncached proof.

At scale 512, history growth falls from 2,868.295 ms to 2,275.531 ms (-20.7%)
and exact replay from 3,995.156 ms to 3,034.680 ms (-24.0%). The two paths remove
7,220,458 and 13,071,290 allocation operations respectively, and request
1,160,908,352 and 2,020,434,370 fewer bytes. Persisted rejection falls from
1.700 ms to 1.140 ms (-32.9%) and removes 8,797 allocation operations because it
can reuse every large mutable-domain root. Empty and populated boundaries remove
17,576 and 11,426 operations. Accepted-command allocation and elapsed time are
effectively unchanged because that workload mutates the large world and scheduler
domains. Snapshot size remains 3,139,624 bytes; snapshot construction and
serialization allocations are unchanged, and load/validate remains intentionally
uncached.

## Staged rejection-transaction comparison

The first rollback-clone replacement is recorded in separate
[`elapsed`](baselines/2026-08-16-staged-rejection-transaction-elapsed.json) and
[`allocation`](baselines/2026-08-16-staged-rejection-transaction-allocations.json)
reports, using the mutable-domain commitment cache as its before baseline. A
pre-application validation rejection now persists its evidence by checkpointing
only the attempt tail, affected counter and revision fields, registration flag,
commitment cache and roots, and checkpoint hash. Fatal failure restores that
explicit set; mutable world, scheduler, and unrelated historical evidence are
not cloned. Rejections discovered after mutable command application still pay
for the accepted-command transaction clone that protects application rollback.

The harness measures a pre-application invalid-payload rejection. At scale 512,
that case falls from 1.140 ms to 0.063 ms (-94.5%), from 27,423 allocation
operations to 69, and from 3,508,152 requested bytes to 943,319. History growth
falls from 2,275.531 ms to 1,893.517 ms (-16.8%) and exact replay from 3,034.680
ms to 2,572.965 ms (-15.2%). Each end-to-end path removes 9,013,714 allocation
operations and 816,659,938 requested bytes. All accepted-command, boundary,
snapshot, serialization, and load-validation allocation counts are unchanged,
and snapshot size remains 3,139,624 bytes.

## Staged command-transaction comparison

The accepted-command rollback replacement is recorded in separate
[`elapsed`](baselines/2026-08-16-staged-command-transaction-elapsed.json) and
[`allocation`](baselines/2026-08-16-staged-command-transaction-allocations.json)
reports, using the staged rejection transaction as its before baseline. Command
application checkpoints only armies, actor knowledge, plugin components,
scheduled actions, counters, event/command/attempt tails, registration state,
and commitments. Immutable core maps, generic records, ingress, boundaries,
random state/evidence, and the clock are not cloned.

At scale 512, accepted command processing falls from 1.818 ms to 0.830 ms
(-54.3%), from 36,203 allocation operations to 13,553, and from 4,969,076
requested bytes to 2,750,991. History growth falls from 1,893.517 ms to
1,610.544 ms (-14.9%) and exact replay from 2,572.965 ms to 2,291.933 ms
(-10.9%). Each end-to-end path removes 7,656,955 allocation operations and
676,559,408 requested bytes. Rejection, boundary, snapshot, serialization, and
load-validation allocation counts are unchanged, and snapshot size remains
3,139,624 bytes.

## Staged boundary-transaction comparison

The phased-settlement rollback replacement is recorded in separate
[`elapsed`](baselines/2026-08-16-staged-boundary-transaction-elapsed.json) and
[`allocation`](baselines/2026-08-16-staged-boundary-transaction-allocations.json)
reports, using the staged command transaction as its before baseline. A boundary
checkpoints its mutable world domains, full scheduler and ingress queue, counters,
all append-only journal cut lengths, registration state, and commitments. Its
rollback checkpoint no longer clones immutable core maps or accumulated journal
contents. Boundary proposal evaluation still takes one full-state read snapshot,
so the measured path retains a separate linear-in-history clone.

At scale 512, empty boundaries fall from 4.755 ms to 3.415 ms (-28.2%) and
populated boundaries from 4.581 ms to 3.809 ms (-16.8%). Each removes 22,641
allocation operations and 2,216,435 requested bytes. History growth falls from
1,610.544 ms to 1,357.087 ms (-15.7%) and exact replay from 2,291.933 ms to
2,163.215 ms (-5.6%). Each end-to-end path removes 7,655,934 allocation
operations and 676,593,500 requested bytes. Command, rejection, snapshot,
serialization, and load-validation allocation counts are unchanged, and snapshot
size remains 3,139,624 bytes.

## Staged scheduled-transaction comparison

The scheduled-advancement rollback replacement is recorded in separate
[`elapsed`](baselines/2026-08-16-staged-scheduled-transactions-elapsed.json) and
[`allocation`](baselines/2026-08-16-staged-scheduled-transactions-allocations.json)
reports, using the staged boundary transaction as its before baseline. Each
same-timestamp scheduled batch now checkpoints only its writable domains and
journal cuts; a final clock-only advance checkpoints only time, registration
state, and commitments.

At scale 512, exact replay falls from 2,163.215 ms to 1,385.267 ms (-36.0%),
from 36,413,190 allocation operations to 18,388,837, and from 4,614,215,568
requested bytes to 2,981,522,168. History construction and every individual
command, rejection, boundary, snapshot, serialization, and load-validation case
retain identical allocation counts. History construction elapsed time changes
by -0.1%, within normal local measurement noise, and snapshot size remains
3,139,624 bytes.

## Staged ingress-transaction comparison

The ingress-insertion rollback replacement is recorded in separate
[`elapsed`](baselines/2026-08-16-staged-ingress-transaction-elapsed.json) and
[`allocation`](baselines/2026-08-16-staged-ingress-transaction-allocations.json)
reports, using the staged scheduled transaction as its before baseline. A failed
append now restores only the ingress identifier, evidence tail, exact pending
queue entry, registration state, and commitments.

The current growth fixture records zero ingress entries at every scale, so this
report does not claim an ingress-path speedup. Every allocation summary is
exactly identical to the before report, checkpoint hashes and snapshot sizes are
unchanged, and elapsed differences are treated as local measurement noise. A
future harness extension must add growing populated ingress history before
quantifying this path directly.

## Boundary read-view comparison

The boundary phase-read replacement is recorded in separate
[`elapsed`](baselines/2026-08-16-boundary-read-view-elapsed.json) and
[`allocation`](baselines/2026-08-16-boundary-read-view-allocations.json) reports,
using the staged ingress transaction as its before baseline. Early phases now
snapshot only current authoritative state and borrow evidence during each
handler call instead of cloning the complete runtime and its accumulated
journals.

At scale 512, empty boundaries fall from 3.455 ms to 2.273 ms (-34.2%) and
populated boundaries from 3.832 ms to 2.757 ms (-28.1%). Each removes 20,515
allocation operations and 2,107,225 requested bytes. History growth falls from
1,372.994 ms to 1,053.365 ms (-23.3%) and exact replay from 1,370.399 ms to
1,039.289 ms (-24.2%). Each end-to-end path removes 6,567,422 allocation
operations and 620,677,980 requested bytes. Command, rejection, snapshot,
serialization, and load-validation allocation counts are unchanged, and
snapshot size remains 3,139,624 bytes.

## Checkpoint-journal persistence comparison

The incremental persistence foundation is recorded in separate
[`elapsed`](baselines/2026-08-16-checkpoint-journal-elapsed.json) and
[`allocation`](baselines/2026-08-16-checkpoint-journal-allocations.json) reports,
using the boundary read-view milestone as the before baseline for the unchanged
flat-snapshot cases. The new measurements separate current-state checkpoint
creation and serialization from full and empty-tail evidence-segment export.

At scale 512, current-state checkpoint creation takes 0.123 ms instead of 0.759
ms for a flat snapshot (-83.8%), falls from 25,220 to 4,735 allocation operations
(-81.2%), and requests 308,394 instead of 2,413,994 bytes (-87.2%). Pretty JSON
serialization takes 0.523 ms instead of 3.692 ms (-85.8%) and requests 1,048,448
instead of 8,388,480 bytes (-87.5%). The serialized current-state checkpoint is
500,496 bytes, 84.1% smaller than the 3,139,624-byte flat snapshot. A one-time
full evidence segment is 2,682,427 bytes and costs 20,485 allocation operations;
exporting an unchanged tail takes a 300 ns median with zero allocations.

The portable pretty-JSON `CheckpointJournal` convenience bundle is 3,698,799
bytes, 17.8% larger than the flat snapshot because the explicit nested envelope
adds indentation and field overhead. Its purpose is portability and validation,
not single-file size reduction. Incremental stores obtain the scaling benefit by
persisting the current-state checkpoint plus only newly appended journal
segments. The live runtime still retains complete evidence in this milestone;
sealed-segment eviction and compaction remain explicit later work.

## Persistence-module extraction comparison

The behavior-preserving persistence extraction is recorded in separate
[`elapsed`](baselines/2026-08-16-persistence-module-elapsed.json) and
[`allocation`](baselines/2026-08-16-persistence-module-allocations.json) reports,
using the checkpoint-journal milestone as its before baseline. It moves the
checkpoint/journal constant, public wire types, cursors, export path, and load
helpers into `canwu-sim/src/persistence.rs` without changing their bodies or
public re-exports. `canwu-sim/src/lib.rs` falls from 19,053 to 18,680 lines, with
381 focused lines in the new module.

Every measured allocation summary, history count, checkpoint hash, checkpoint-
storage size, and flat snapshot size is identical at scales 8, 32, 128, and 512.
At scale 512, representative elapsed differences range from -2.3% for flat
snapshot creation to +3.4% for load validation; history growth is -0.6% and exact
replay +0.8%. These are treated as local timing noise. The milestone makes no
runtime-performance claim; it establishes a narrower ownership surface for later
storage work.

## Transaction-module extraction comparison

The behavior-preserving rollback-checkpoint extraction is recorded in separate
[`elapsed`](baselines/2026-08-16-transactions-module-elapsed.json) and
[`allocation`](baselines/2026-08-16-transactions-module-allocations.json) reports,
using the persistence-module milestone as its before baseline. It moves the
rejection, ingress, command, boundary, scheduled-batch, and clock checkpoint
types and their exact capture/restore bodies into
`canwu-sim/src/transactions.rs`. `canwu-sim/src/lib.rs` falls from 18,680 to
18,436 lines, with 257 focused lines in the new module.

Every measured allocation summary, history count, checkpoint hash, checkpoint-
storage size, and flat snapshot size is identical at all four scales. At scale
512, history growth changes by +0.2%, accepted commands +0.6%, empty boundaries
+0.7%, populated boundaries -2.3%, load validation -3.2%, and exact replay
+0.1%. The rejected-command median moves from 0.067 ms to 0.075 ms (+12.4%) on a
sub-0.1-ms case. These are treated as local timing noise. The milestone makes no
runtime-performance claim; it narrows rollback ownership without changing what
any transaction captures or restores.

## Runtime-state module extraction comparison

The behavior-preserving runtime-state extraction is recorded in separate
[`elapsed`](baselines/2026-08-16-runtime-state-module-elapsed.json) and
[`allocation`](baselines/2026-08-16-runtime-state-module-allocations.json)
reports, using the transaction-module milestone as its before baseline. It moves
the private current-state, scheduler, counter, metadata, evidence, and
incremental-commitment definitions and their exact method bodies into
`canwu-sim/src/state.rs`. `canwu-sim/src/lib.rs` falls from 18,436 to 18,101
lines, with 350 focused lines in the new module.

Every measured allocation sample and summary, history count, checkpoint hash,
checkpoint-storage size, and flat snapshot size is identical at scales 8, 32,
128, and 512. At scale 512, history growth changes by -0.1%, empty boundaries
-2.2%, populated boundaries +1.9%, snapshot creation +2.1%, load validation
+4.2%, and exact replay -1.7%. The rejected-command median moves from 0.075 ms
to 0.017 ms on a sub-0.1-ms case. These are treated as local timing noise. The
milestone makes no runtime-performance claim; it gives mutable state and its
commitment cache a dedicated private ownership boundary without changing their
contracts.

## Incremental boundary-state commitment comparison

The boundary-state commitment milestone is recorded in separate
[`elapsed`](baselines/2026-08-16-incremental-boundary-state-elapsed.json) and
[`allocation`](baselines/2026-08-16-incremental-boundary-state-allocations.json)
reports, using the runtime-state module milestone as its before baseline. New
boundaries replace the legacy full-state/history hash pass with an explicitly
tagged `v1:` commitment over the already-maintained canonical domain roots and
the prior boundary-chain head. Untagged historical hashes retain their original
meaning, and exact replay selects the recorded contract per boundary, including
mixed legacy/current chains.

At scale 512, an empty boundary falls from 2.358 ms to 0.452 ms (-80.8%) and
from 16,385 to 11,769 allocation operations (-28.2%); requested bytes fall
78.2%. A populated boundary falls from 2.619 ms to 0.775 ms (-70.4%) and from
22,665 to 18,045 operations (-20.4%); requested bytes fall 68.9%. End-to-end
history construction falls from 1,046.898 ms to 514.303 ms (-50.9%), while
exact replay falls from 1,038.483 ms to 526.397 ms (-49.3%). Their requested
bytes fall 54.1% and 53.9%, respectively. Load and validation falls 12.5% in
elapsed time and 16.0% in requested bytes because a current boundary head is
verified from independently validated roots rather than a second full-state
hash pass.

The tagged state hash adds exactly three serialized bytes per boundary: the
scale-512 snapshot grows by 1,536 bytes, from 3,139,624 to 3,141,160 bytes.
Snapshot creation and serialization allocation-operation counts are otherwise
unchanged. This milestone removes the repeated full-history boundary hash, but
does not make the complete growth or replay path linear; other retained-state
cloning and validation work remains measurable.

The allocation evidence identifies two distinct growth classes:

- A fourfold increase from scale 128 to 512 makes accepted commands, individual
  boundaries, snapshot creation, and snapshot loading request about four times
  as many allocation operations. These operations remain linear in retained
  current state or history in the current implementation. Rejected commands remain at 69
  allocation operations while requested bytes grow from 242,903 to 943,319,
  showing a separate retained-capacity cost rather than operation-count growth.
- The same increase makes end-to-end history construction grow from 674,194 to
  9,916,267 allocation operations (14.7x) and exact replay from 690,644 to
  9,981,869 operations (14.5x). At scale 512, they request 1,078,885,750 and
  1,087,775,557 bytes. These paths remain superlinear and close to quadratic
  across this range despite the roughly halved elapsed time and allocated bytes.

Those observations establish optimization targets; they do not change any
runtime contract or claim that one subsystem alone is the cause.

## Live journal archival comparison

The explicit live-archive milestone is recorded in frozen-source before reports
for [`elapsed`](baselines/2026-08-16-live-archive-before-elapsed.json) and
[`allocations`](baselines/2026-08-16-live-archive-before-allocations.json), plus
the implementation reports for
[`elapsed`](baselines/2026-08-16-live-archive-after-elapsed.json) and
[`allocations`](baselines/2026-08-16-live-archive-after-allocations.json).
`CompactedCanwu::seal_evidence` moves a fully settled live tail into one
caller-owned contiguous segment, retains compact idempotency commitments and
outcomes, and continues from the same global cursors and incremental roots.

At scale 512, sealing 513 completed boundaries, 512 accepted commands, 1,024
attempts, 1,536 admitted events, 512 random draws, and their ingress evidence
takes 0.582 ms and 3,628 allocation operations. The API call allocates 876,856
bytes for compact continuation indexes and retains a net 349,592 bytes across
the compact runtime plus returned segment. Releasing that caller-owned segment
is measured separately at 0.233 ms, performs zero allocation operations, and
changes retained bytes by -3,339,747; together the two operations change
retained bytes by -2,990,155. The segment serializes to 2,684,578 bytes; the
current-state checkpoint remains 500,496 bytes. Sealing moves the existing
journal buffers into the segment, so the handoff itself does not clone the full
evidence payload. Outcome reconstruction indexes each retained attempt directly
from the monotonic command cursor. From scale 128 to 512, a single seal grows
from 0.156 to 0.582 ms (3.73x) and from 975 to 3,628 allocation operations
(3.72x) for four times as much retained evidence.

The ordinary full-history runtime paths keep identical allocation-operation
counts before and after at every scale. At scale 512, history growth changes
from 517.318 to 513.184 ms, exact replay from 522.558 to 520.223 ms, and load
validation from 17.413 to 16.450 ms. Individual command, boundary, and snapshot
medians move within local sub-millisecond timing noise. Compaction is opt-in;
the existing flat snapshot, full-history slice, and replay-journal APIs retain
their prior behavior.

The isolated repeated-seal workload uses one fixed entity, admits one command
at one boundary, and seals after every cycle. From scale 128 to 512, elapsed
time grows from 5.538 to 22.552 ms (4.07x) and allocation operations from
50,203 to 200,026 (3.98x) for four times as many cycles. This is an observed
near-linear cumulative curve across the measured range; each seal hashes and
inserts only its new request entries into ordered continuation indexes. The
broader compacted growth workload still grows superlinearly because it
intentionally retains the original workload's expanding entity/component state
and adds an admission boundary per cycle; the isolated result separates that
existing current-state cost from archive-index maintenance.

## Plugin-module extraction comparison

The behavior-preserving plugin-registration extraction is recorded in separate
[`elapsed`](baselines/2026-08-16-plugins-module-elapsed.json) and
[`allocation`](baselines/2026-08-16-plugins-module-allocations.json) reports,
using the final live-archive reports as its before baseline. It moves registrar
methods, registry hydration, plugin contract validation, and ownership-index
construction into `canwu-sim/src/plugins.rs` without changing public types,
serialized state, replay behavior, or registration rules. `canwu-sim/src/lib.rs`
falls from 18,668 to 17,503 lines, with 1,178 focused lines in the new module.

Every measured allocation sample and summary, history count, checkpoint hash,
checkpoint-storage size, and flat snapshot size is identical at scales 8, 32,
128, and 512. At scale 512, elapsed medians change by +0.0% for history growth,
+1.4% for accepted commands, -1.1% for empty boundaries, +1.3% for populated
boundaries, -0.3% for load validation, and -0.8% for exact replay. These are
treated as local timing noise. The milestone makes no runtime-performance
claim; it gives plugin registration and descriptor hydration a dedicated
ownership surface while preserving the persistence and execution contracts.

## Replay-module extraction comparison

The behavior-preserving replay extraction is recorded in separate
[`elapsed`](baselines/2026-08-16-replay-module-elapsed.json) and
[`allocation`](baselines/2026-08-16-replay-module-allocations.json) reports,
using the plugin-module milestone as its before baseline. It moves fixture and
environment-bound replay entry points, boundary/ingress cut reconstruction, and
command/attempt record applicators into `canwu-sim/src/replay.rs` without
changing public signatures, journal formats, migration rules, or final
checkpoint verification. `canwu-sim/src/lib.rs` falls from 17,503 to 16,891
lines, with 625 focused lines in the new module.

Every measured allocation sample and summary, history count, checkpoint hash,
checkpoint-storage size, and flat snapshot size is identical at all four
scales. At scale 512, history growth changes by -0.4%, accepted commands -2.3%,
empty boundaries +2.8%, populated boundaries -1.2%, load validation -1.4%, and
exact replay +1.3%. Snapshot creation moves from 0.737 to 0.781 ms (+6.0%) on a
sub-millisecond case. These are treated as local timing noise. The milestone
makes no runtime-performance claim; it gives exact replay and fixture
reconstruction a dedicated dependency surface while preserving deterministic
regeneration and environment binding.

## Validation-module extraction comparison

The behavior-preserving validation extraction is recorded in separate
[`elapsed`](baselines/2026-08-16-validation-module-elapsed.json) and
[`allocation`](baselines/2026-08-16-validation-module-allocations.json) reports,
using the replay-module milestone as its before baseline. It moves the complete
snapshot/runtime validation graph, including causal-prefix, boundary,
authority, scheduling, random-evidence, domain-record, and counter invariants,
into `canwu-sim/src/validation.rs`. Migration and hashing remain separate
follow-up surfaces. `canwu-sim/src/lib.rs` falls from 16,891 to 14,101 lines,
with 2,831 focused lines in the new module.

Every measured allocation sample and summary, history count, checkpoint hash,
checkpoint-storage size, and flat snapshot size is identical at all four
scales. At scale 512, history growth changes by +1.8%, accepted commands +3.6%,
empty boundaries -0.9%, populated boundaries -0.3%, snapshot creation -2.4%,
load validation +5.1%, and exact replay -1.1%. The rejected-command median
moves from 0.063 to 0.072 ms (+15.5%) on a sub-0.1-ms case. These are treated as
local timing noise. The milestone makes no runtime-performance claim; it gives
authoritative validation a dedicated dependency surface while preserving every
persisted and replayed invariant.

## Migration-module extraction comparison

The behavior-preserving migration extraction is recorded in separate
[`elapsed`](baselines/2026-08-16-migration-module-elapsed.json) and
[`allocation`](baselines/2026-08-16-migration-module-allocations.json) reports,
using the validation-module milestone as its before baseline. It moves legacy
snapshot normalization, revision and admission-cursor translation, boundary
rehashing, format-3 conversion, and historical run-configuration inference into
`canwu-sim/src/migration.rs`. Hash material and current commitment construction
remain in `lib.rs` for a separate hashing milestone. `canwu-sim/src/lib.rs`
falls from 14,101 to 13,452 lines, with 669 focused lines in the new module.

Every measured allocation sample and summary, history count, checkpoint hash,
checkpoint-storage size, and flat snapshot size is identical at all four
scales. At scale 512, history growth changes by -0.4%, empty boundaries -0.9%,
populated boundaries +5.6%, snapshot creation -0.6%, load validation -3.9%, and
exact replay +0.9%. The accepted-command median moves from 0.844 to 0.972 ms
(+15.1%) on a sub-millisecond case. These are treated as local timing noise.
The milestone makes no runtime-performance claim; it gives historical format
interpretation a dedicated dependency surface while preserving deterministic
migration and current-format validation.
