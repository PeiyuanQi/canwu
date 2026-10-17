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

Select `--suite information-flow` for the public knowledge-ledger profile. It
publishes records through a registered schema and validated boundary directives,
keeps approximately half of each boundary's records on one hot holder, and
spreads the rest across ordinary holders. It measures 1/10/100/1,000-record
publication boundaries, total history construction, snapshot creation and
serialization, current-head and paged history/delta queries, snapshot load plus
validation, exact replay, checkpoint-journal serialization, and an initial
compacted evidence seal. The smoke preset uses 100, 1,000, and 10,000 total
records. The full preset uses 10,000, 100,000, and 1,000,000 records, with up to
10,000 records per settled boundary. Every information-flow run also executes
fixed public-API stress cases for 100 knowledge schemas, a 10,000-recipient
addressed dispatch, a 10,000-member explicit audience, 1,000 mixed-lineage
nodes, 100,000 access records distributed across 1,000 holders, one filtered
query per holder, and a 100-segment archive-provider build and restore.

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

Run the measured information-flow smoke preset with:

```console
cargo run --release \
  --manifest-path benchmarks/performance-harness/Cargo.toml -- \
  --suite information-flow \
  --preset smoke \
  --samples 3 \
  --growth-samples 1 \
  --warmup 1 \
  --mode elapsed \
  --machine local-windows-x86_64 \
  --recorded-on 2026-08-21 \
  --output benchmarks/baselines/2026-08-21-information-flow-smoke-elapsed.json
```

The million-record preset is intentionally opt-in because the hot-holder paged
history query derives and binds a complete read cut on each page:

```console
cargo run --release \
  --manifest-path benchmarks/performance-harness/Cargo.toml -- \
  --suite information-flow \
  --preset full \
  --samples 3 \
  --growth-samples 1 \
  --warmup 0 \
  --mode elapsed \
  --machine YOUR-MACHINE-LABEL \
  --recorded-on YYYY-MM-DD \
  --output benchmarks/baselines/YYYY-MM-DD-information-flow-full-elapsed.json
```

A three-sample operation pass with one growth sample is the final comparison
profile. The harness records source fingerprints before measurement and checks
them again at the end; any source change fails closed instead of writing a
mixed-source baseline. The final 2026-08-21 run completed all three scales and
peaked at 22,294,740,992 bytes (20.76 GiB) after the harness was changed to load
million-record snapshot and journal inputs from temporary files per measured
setup. This keeps the full profile within ordinary workstation memory while
preserving the measured operation interval.

Use `--scales` to override either preset. PowerShell accepts the same arguments
on one line. Use
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

## Technology and historical-research profile

The standalone `technology-profile` binary measures the published generic
technology extension with all three historical assessment plugins enabled. It
has a fixed 100-site paced-interactive component profile and a 500-site pressure
profile. Every one of its 240
months commits a technology claim and all three assessment kinds. It records
actual file write/sync/read timings, snapshot and checkpoint reconstruction,
exact replay, serialized growth, and fresh-process peak RSS.

The recorded 2026-08-22 results and their limits are documented in
[`2026-08-22-technology.md`](2026-08-22-technology.md). The paced-interactive result
is suitable for paced or turn-based home-computer play, but it remains component
evidence rather than a whole-game sub-100 ms certification. The pressure profile must not be presented as an
interactive scale while the kernel retains broad boundary clones.

Run the interactive profile with:

```console
cargo run --release \
  --manifest-path benchmarks/performance-harness/Cargo.toml \
  --bin technology-profile -- \
  --profile interactive \
  --samples 31 \
  --warmup 1 \
  --months 240 \
  --machine YOUR-STABLE-MACHINE-LABEL \
  --recorded-on YYYY-MM-DD \
  --output benchmarks/baselines/YYYY-MM-DD-technology-interactive-elapsed.json
```

## Keyed-draw retention profile

The standalone `keyed-draw-profile` binary measures what a run pays as it
keeps more operation-keyed random draws. For each `--systems` count and each
`--retained` count it measures:

- `live_boundary`: one Daily boundary in which that many boundary systems are
  due and each makes `--draws` fresh keyed draws, on a run that still retains
  every draw record;
- `sealed_boundary`: the same boundary after the evidence was sealed, when
  each earlier draw is sealed (skipped when nothing is retained);
- `snapshot_restore`: loading the run from its snapshot JSON;
- `checkpoint_journal_restore`: loading the run from its portable checkpoint
  journal;
- after one seal of the whole run, so that every keyed draw is sealed:
  `compact_checkpoint`, `compact_paged_checkpoint` (against an empty page
  store), `compact_seal` (sealing one more Daily boundary), and
  `compact_fork`. An operation the engine refuses is reported with an
  `error` field instead of being treated as a failed run.

`compact_fork` needs `CompactedCanwu::fork`, which engines before 0.16.0 lack.
It sits behind the harness's default `compact-fork` feature; build an export
of an older release with `--no-default-features` (and
`--no-default-features --features allocation-counting` for its allocation
build).

It follows the conventions of the main harness. Elapsed time and allocation
traffic come from separate release builds; allocation counts do not depend on
machine load, and the allocation build also records `run_heap_bytes`, the
bytes the built run holds. `--samples` and `--restore-samples` must be
positive odd integers. The binary uses only the public API, so the same source
measures any engine version it compiles against.

```console
cargo run --release \
  --manifest-path benchmarks/performance-harness/Cargo.toml \
  --bin keyed-draw-profile -- \
  --mode elapsed \
  --retained 0,1000,10000,100000 \
  --systems 1,8,32 \
  --draws 1 \
  --samples 31 \
  --warmup 3 \
  --restore-samples 3 \
  --machine YOUR-STABLE-MACHINE-LABEL \
  --recorded-on YYYY-MM-DD \
  --source-label "WHAT-WAS-BUILT" \
  --output benchmarks/baselines/YYYY-MM-DD-keyed-draw-index-VERSION-elapsed.json

cargo run --release \
  --manifest-path benchmarks/performance-harness/Cargo.toml \
  --features allocation-counting \
  --bin keyed-draw-profile -- \
  --mode allocations \
  --retained 0,1000,10000,100000 \
  --systems 1,8,32 \
  --draws 1 \
  --samples 3 \
  --warmup 1 \
  --restore-samples 1 \
  --machine YOUR-STABLE-MACHINE-LABEL \
  --recorded-on YYYY-MM-DD \
  --source-label "WHAT-WAS-BUILT" \
  --output benchmarks/baselines/YYYY-MM-DD-keyed-draw-index-VERSION-allocations.json
```

The 2026-10-13 baselines compare 0.14.0 with the 0.15.0 change that keeps the
keyed-draw index as runtime state:
[`0.14.0 elapsed`](baselines/2026-10-13-keyed-draw-index-0.14.0-elapsed.json),
[`0.15.0 elapsed`](baselines/2026-10-13-keyed-draw-index-0.15.0-elapsed.json),
[`0.14.0 allocations`](baselines/2026-10-13-keyed-draw-index-0.14.0-allocations.json), and
[`0.15.0 allocations`](baselines/2026-10-13-keyed-draw-index-0.15.0-allocations.json).
The 0.14.0 files were produced from an export of the release tag: `git archive
v0.14.0` into an empty directory, this binary's source copied into the
export's `benchmarks/performance-harness/src/bin`, and the same two commands
run there. The 0.15.0 files were recorded on the development branch before the
version bump, so their `engine_version` field still reads `0.14.0`;
`source.label` and `source.content_hash` identify what was built, and
`environment.rustc` is asked from the binary's own manifest directory, so it
names the toolchain the measured tree pins (Rust 1.99.0 for both). Other builds
ran on the machine during the elapsed runs, so compare the elapsed files for
growth and the allocation files for exact counts. The figures are interpreted
in
[`docs/proposals/keyed-draw-index-and-settlement-profile.md`](../docs/proposals/keyed-draw-index-and-settlement-profile.md).

The 2026-10-15 allocation baselines compare 0.15.0 with the keyed-draw ledger
of the continuation-checkpoint package, at 8 due systems and 1,000, 10,000,
and 100,000 retained keyed draws:
[`before`](baselines/2026-10-15-keyed-draw-ledger-before-allocations.json)
and [`after`](baselines/2026-10-15-keyed-draw-ledger-after-allocations.json).
The before file comes from an export of `v0.15.0` built with
`--no-default-features`, with `CARGO_TARGET_DIR` set to a short path (the
`blake3` assembler on Windows refuses very long object paths). Its
`compact_paged_checkpoint` cases at 10,000 and 100,000 draws carry the error
0.15.0 returns there: the paged envelope that lists every keyed reservation
no longer fits one 4 MiB state page. The architecture suite was run on both
trees for the same comparison:
[`before`](baselines/2026-10-15-architecture-ledger-before-allocations.json)
and [`after`](baselines/2026-10-15-architecture-ledger-after-allocations.json).
The elapsed cost of the resumable journal stream hash was measured on one tree
in two release builds, one with the 0.15.0 `blake3::Hasher` journal
accumulator and one with the resumable hash, run alternately five times each
in elapsed mode; each file merges the samples of its five runs, and its
`method.variant` names the build:
[`before`](baselines/2026-10-15-journal-stream-hash-before-elapsed.json)
and [`after`](baselines/2026-10-15-journal-stream-hash-after-elapsed.json).
The figures are interpreted in
[`docs/proposals/continuation-checkpoint.md`](../docs/proposals/continuation-checkpoint.md).

The 2026-10-15 send-runtime baselines compare the `release/0.16.0` base at
`9e5519c` (plugin archive provider held as `Rc`) with the change that holds it
as `Arc<dyn PluginArchiveObjectProvider + Send + Sync>`, at 8 due systems and
0, 1,000, and 10,000 retained keyed draws. The before build is a `git archive`
export of the base built in its own `CARGO_TARGET_DIR`; both
builds use the same harness source. The allocation files come from one run of
each build:
[`before`](baselines/2026-10-15-send-runtime-before-allocations.json) and
[`after`](baselines/2026-10-15-send-runtime-after-allocations.json). The
elapsed builds were run alternately five times each, with other builds on the
machine; each file merges the samples of its five runs, and its
`method.variant` names the build:
[`before`](baselines/2026-10-15-send-runtime-before-elapsed.json) and
[`after`](baselines/2026-10-15-send-runtime-after-elapsed.json). The figures
are interpreted in
[`docs/proposals/send-runtime.md`](../docs/proposals/send-runtime.md).

## Continuation profile

The standalone `continuation-profile` binary compares what a save costs as a
campaign grows: the full portable save (`checkpoint_journal_json` and its
restore) against the continuation checkpoint (`continuation_checkpoint`, its
JSON, and `CompactedCanwu::from_continuation_checkpoint_json_verified`). The
workload settles one Daily boundary a simulated day: the host dispatches
tracked commands whose packets fall due 1 to 90 days later, admitted packets
move a record, make `--keyed` operation-keyed draws each, and schedule
follow-ups, a knowledge record supersedes the last each day, a holder keeps a
payload-required continuation, and a churn system moves `--churn` records a
day so that the full save grows about 10 MB a simulated month while current
state does not. At each sampled month it reports both saves' bytes and
times, and the continuation checkpoint's bytes by part (state, pins,
keyed-draw ledger, request index, time index, boundary ledger, frontiers).
The allocation build also reports each restore's allocations and, at the
last month, the heap of a run that seals every month against one that never
seals.

```console
cargo run --release \
  --manifest-path benchmarks/performance-harness/Cargo.toml \
  --bin continuation-profile -- \
  --mode elapsed \
  --machine local-windows-x86_64 \
  --recorded-on 2026-10-15 \
  --output benchmarks/baselines/2026-10-15-continuation-elapsed.json

cargo run --release \
  --manifest-path benchmarks/performance-harness/Cargo.toml \
  --features allocation-counting \
  --bin continuation-profile -- \
  --mode allocations \
  --samples 1 \
  --machine local-windows-x86_64 \
  --recorded-on 2026-10-15 \
  --output benchmarks/baselines/2026-10-15-continuation-allocations.json
```

The defaults sample months 1, 2, 4, 7, 10, and 14 for 0 and 10 keyed draws
per packet, with 8 commands and 140 churned records a day. The recorded
[`elapsed`](baselines/2026-10-15-continuation-elapsed.json) and
[`allocation`](baselines/2026-10-15-continuation-allocations.json) reports
are interpreted in
[`docs/proposals/continuation-checkpoint.md`](../docs/proposals/continuation-checkpoint.md).
The 2026-10-16 reports compare the 0.18.0 integration branch before
([`base`](baselines/2026-10-16-continuation-0.18.0-base-elapsed.json)) and
after ([`state copies`](baselines/2026-10-16-continuation-0.18.0-state-copies-elapsed.json))
the runtime-state-copies change, with `--months 1,4,14 --keyed 0 --samples 3`;
the faster restores come from the JSON decoder, see
[`docs/proposals/runtime-state-copies.md`](../docs/proposals/runtime-state-copies.md).

## Resource archive profile

The standalone `resource-archive-profile` binary measures the terminal archive
of `canwu-resource`: what a cold lookup reads and costs as archived records and
archive commits grow, what preparing and storing one commit costs and how many
lookup-index bytes it writes, the restore audit, and what one boundary costs
against a hot terminal backlog. Every archived record is a copy of one rejected
operation outcome under its own key; commits go through
`prepare_resource_archive` and `store_and_verify` on an in-memory store that
copies the bytes of every object it serves. Each backlog case settles real
boundaries through the resource plugin: one requester-scoped allocation per
boundary, timing `settle_boundary` only and measuring the evidence that
boundary appends as encoded JSON (`boundary_ms`, `journal_bytes_per_boundary`).
`state_operation_ms` is the same operation applied to a detached copy of the
root, the state-level share of that boundary. The `smoke` preset finishes in
seconds; the `full` preset archives up to one million records and 120,000
commits and takes several minutes.

The recorded 2026-10-13 results are in
[`baselines/2026-10-13-resource-archive-full.json`](baselines/2026-10-13-resource-archive-full.json).
`source.content_hash` identifies the measured sources: a BLAKE3 hash over
`crates/extensions/canwu-resource/src` and the profile binary.
`source.git_commit` is only the commit that was checked out when the profile
ran, and `source.git_dirty` says whether the tree had uncommitted changes. That
baseline was recorded together with the change it measures, so it names the
parent commit and is dirty.

The [resource archive proposal](../docs/proposals/resource-archive-lookup-and-volume.md)
compares those results with 0.14.0. The binary does not compile against
0.14.0, where `prepare_resource_archive` takes no store and a batch has no
lookup pages. The 0.14.0 numbers came from a copy of its lookup case with
those two differences, built at commit `6617f54`, with each hot record produced
by a real rejected operation; that copy is not kept in the tree.

```console
cargo run --release \
  --manifest-path benchmarks/performance-harness/Cargo.toml \
  --bin resource-archive-profile -- \
  --preset full \
  --machine YOUR-STABLE-MACHINE-LABEL \
  --recorded-on YYYY-MM-DD \
  --output benchmarks/baselines/YYYY-MM-DD-resource-archive-full.json
```

## Production archive profile

The standalone `production-archive-profile` binary measures the terminal
archive of `canwu-production` and its reads of the resource archive:

- what restore validation pays for one execution input whose resource
  consumption and outcome were archived, as the resource archive grows in
  records and in commits (`validate_production_resource_continuation`, with
  the objects and bytes it reads);
- a chain of archive commits through the production plugin, each archiving
  the maintenance receipt of the commit before it and acknowledging that
  commit in the same boundary: what preparing, queueing, and settling one
  commit costs at a given chain length, the journal bytes of that boundary,
  the size of the production root, a lookup miss and a hit on the first
  execution, the bytes the commits wrote, and the restore audit
  (`validate_production_archive`);
- one commit of several pages of settled executions on top of an archive of
  earlier commits: what it costs and writes, and what a lookup hit reads.

Every production record is real: settled executions are built through the
public state API. The store is in memory and copies the bytes of every object
it serves. The chain runs unsealed, so the kernel keeps the evidence of every
boundary in memory; the `full` preset's longest chain is 30,000 commits, and
the run peaked at 2.4 GiB of working set. A chain longer than that is better
measured on a sealed run.

The recorded 2026-10-15 results are in
[`baselines/2026-10-15-production-archive-full.json`](baselines/2026-10-15-production-archive-full.json),
and the [production archive proposal](../docs/proposals/production-archive-volume.md)
compares them with 0.15.0. The baseline's `source.git_commit` names the commit
the run was built on, an earlier state of the commit that adds the profile, and
`git_dirty` says the tree had changed since; `source.content_hash`, the BLAKE3
of the production and resource sources and the profile, identifies the
measured code. As with the resource archive profile, the binary
does not compile against 0.15.0; the before-run used a copy with the 0.15.0
API that is not kept in the tree.

```console
cargo run --release \
  --manifest-path benchmarks/performance-harness/Cargo.toml \
  --bin production-archive-profile -- \
  --preset full \
  --machine YOUR-STABLE-MACHINE-LABEL \
  --recorded-on YYYY-MM-DD \
  --output benchmarks/baselines/YYYY-MM-DD-production-archive-full.json
```

## Fiscal boundary profile

The standalone `fiscal-profile` binary measures what `canwu-fiscal` adds to a
boundary. It installs a fiscal state with a history of settled assessments and
execution requests (48 institutions and 40 settled cycles by default, about
2 MiB of JSON) and times, through the public API:

- a boundary that admits one ingress item of another plugin and no fiscal
  ingress, in a run without fiscal records, with a fiscal state no boundary has
  written yet, and with one a boundary has written;
- the boundary that admits a batch of fiscal action commands, one per
  institution, and the boundary that settles them; and
- `fiscal_ledger_component_ms`: decoding, copying, validating, and encoding
  one scope ledger record, which is what an action writes since the state was
  partitioned into records, so a boundary time can be read as a count of
  those operations, and `audit_every_record`, the host's audit of every
  fiscal record (`audit_fiscal_records`). Up to Phase 1 of the world-scale
  change these figures were of the whole state record, with
  `settlement_measure` beside them.

```console
cargo run --release \
  --manifest-path benchmarks/performance-harness/Cargo.toml \
  --bin fiscal-profile -- \
  --samples 31 \
  --machine YOUR-STABLE-MACHINE-LABEL \
  --recorded-on YYYY-MM-DD \
  --output benchmarks/baselines/YYYY-MM-DD-fiscal-boundary-elapsed.json
```

`--observers 0` installs no observer, so no report is published. That
separates the plugin's own cost on an idle boundary from the engine's cost of
carrying published report records through every boundary. Each observer sees
`--observed-institutions` institutions of its own, two by default, which is
what one 0.14.0 report record could hold; `--observers 1
--observed-institutions 48` gives one observer a report of every institution.
`--institutions`, `--cycles`, and `--batch` change the state size and the
batch. The report also gives the records and payload bytes of one observer's
report as first published, and of what the settling boundary publishes.

Up to the 0.16.0 fiscal-limits change the source used only API that
`canwu-fiscal` 0.14.0 also has, so the same file built against every release
the runs below compare. The world-scale fiscal change of 0.16.0 (see
[`docs/proposals/world-scale-fiscal.md`](../docs/proposals/world-scale-fiscal.md))
gives the state its per-scope books, outcome rings, and observer line slots,
and the source now installs that shape, so it no longer builds against 0.15.0
or earlier: compare with an earlier release from an export of that release's
own copy of the binary. Each scope's settled history is its hot books, at most
64 cycles. 0.14.0 cannot run the wide-observer case: the boundary that
publishes its first report fails. Every recorded run is kept:

| Run | 0.14.0 | 0.15.0 | 0.15.0, one observer of 48 institutions |
| --- | --- | --- | --- |
| Round 1 | [`elapsed`](baselines/2026-10-13-fiscal-boundary-0.14.0-round1-elapsed.json) | [`elapsed`](baselines/2026-10-13-fiscal-boundary-0.15.0-round1-elapsed.json) | [`elapsed`](baselines/2026-10-13-fiscal-boundary-0.15.0-wide-observer-round1-elapsed.json) |
| Round 2 | [`elapsed`](baselines/2026-10-13-fiscal-boundary-0.14.0-round2-elapsed.json) | [`elapsed`](baselines/2026-10-13-fiscal-boundary-0.15.0-round2-elapsed.json) | [`elapsed`](baselines/2026-10-13-fiscal-boundary-0.15.0-wide-observer-round2-elapsed.json) |
| Round 3 | [`elapsed`](baselines/2026-10-13-fiscal-boundary-0.14.0-round3-elapsed.json) | [`elapsed`](baselines/2026-10-13-fiscal-boundary-0.15.0-round3-elapsed.json) | [`elapsed`](baselines/2026-10-13-fiscal-boundary-0.15.0-wide-observer-round3-elapsed.json) |
| No observer | [`elapsed`](baselines/2026-10-13-fiscal-boundary-0.14.0-no-observers-elapsed.json) | [`elapsed`](baselines/2026-10-13-fiscal-boundary-0.15.0-no-observers-elapsed.json) | |

The 0.14.0 runs were recorded first, alternately with an earlier state of the
0.15.0 change. The 0.15.0 runs were recorded later the same day from the
release branch with the change on it, before the version bump, so the two
columns were not interleaved. The 0.14.0 files predate the report-size fields
and do not have them. The tables in
[`docs/proposals/fiscal-throughput-and-authority.md`](../docs/proposals/fiscal-throughput-and-authority.md)
are computed from these files: each figure is the median of the three rounds'
`median` values, with the lowest and highest of them beside it.

`--mode allocations`, in a build with `--features allocation-counting`,
counts the allocator calls (allocations and reallocations) of each measured
operation instead of timing it, and the report names its figures
`*_allocation_operations` where the elapsed build has `*_ms`. Allocation
counts do not depend on machine load.

The 2026-10-14 and 2026-10-15 runs compare 0.15.0 with the change that stops a boundary from
carrying held knowledge (see
[`docs/proposals/knowledge-ledger-cost-and-authentication.md`](../docs/proposals/knowledge-ledger-cost-and-authentication.md)).
Both columns were built from the same profile source, 0.15.0 in an export of
the `v0.15.0` tag with the binary's source copied in, and were run
alternately on the same machine; the change was recorded before the version
bump:

| Run | 0.15.0 | the change |
| --- | --- | --- |
| Four observers of two institutions, rounds 1 to 3 | [`1`](baselines/2026-10-14-fiscal-boundary-0.15.0-round1-elapsed.json), [`2`](baselines/2026-10-14-fiscal-boundary-0.15.0-round2-elapsed.json), [`3`](baselines/2026-10-14-fiscal-boundary-0.15.0-round3-elapsed.json) | [`1`](baselines/2026-10-14-fiscal-boundary-0.16.0-round1-elapsed.json), [`2`](baselines/2026-10-14-fiscal-boundary-0.16.0-round2-elapsed.json), [`3`](baselines/2026-10-14-fiscal-boundary-0.16.0-round3-elapsed.json) |
| One observer of 48 institutions, rounds 1 to 3 | [`1`](baselines/2026-10-14-fiscal-boundary-0.15.0-wide-observer-round1-elapsed.json), [`2`](baselines/2026-10-14-fiscal-boundary-0.15.0-wide-observer-round2-elapsed.json), [`3`](baselines/2026-10-14-fiscal-boundary-0.15.0-wide-observer-round3-elapsed.json) | [`1`](baselines/2026-10-14-fiscal-boundary-0.16.0-wide-observer-round1-elapsed.json), [`2`](baselines/2026-10-14-fiscal-boundary-0.16.0-wide-observer-round2-elapsed.json), [`3`](baselines/2026-10-14-fiscal-boundary-0.16.0-wide-observer-round3-elapsed.json) |
| No observer, rounds 1 to 3 | [`1`](baselines/2026-10-14-fiscal-boundary-0.15.0-no-observers-round1-elapsed.json), [`2`](baselines/2026-10-14-fiscal-boundary-0.15.0-no-observers-round2-elapsed.json), [`3`](baselines/2026-10-14-fiscal-boundary-0.15.0-no-observers-round3-elapsed.json) | [`1`](baselines/2026-10-14-fiscal-boundary-0.16.0-no-observers-round1-elapsed.json), [`2`](baselines/2026-10-14-fiscal-boundary-0.16.0-no-observers-round2-elapsed.json), [`3`](baselines/2026-10-14-fiscal-boundary-0.16.0-no-observers-round3-elapsed.json) |
| Allocations: four observers, one wide observer, no observer | [`four`](baselines/2026-10-14-fiscal-boundary-0.15.0-allocations.json), [`wide`](baselines/2026-10-14-fiscal-boundary-0.15.0-wide-observer-allocations.json), [`none`](baselines/2026-10-14-fiscal-boundary-0.15.0-no-observers-allocations.json) | [`four`](baselines/2026-10-15-fiscal-boundary-0.16.0-allocations.json), [`wide`](baselines/2026-10-15-fiscal-boundary-0.16.0-wide-observer-allocations.json), [`none`](baselines/2026-10-15-fiscal-boundary-0.16.0-no-observers-allocations.json) |

The allocation runs use 5 samples after one warm-up. The elapsed rounds of
the change were recorded from an earlier state of it, which differed in the
order of its current-record sets and in load validation; its allocation runs
were recorded again on 2026-10-15 from the final state and give the same
counts.

Phase 2 of the world-scale change partitions the state into records:
[`allocations`](baselines/2026-10-15-fiscal-boundary-0.16.0-partitioned-allocations.json)
in the default shape (three samples after one warm-up, all equal) count
25,502 allocations for the boundary that admits the 40 actions and 231,492
for the one that settles them, where Phase 1's single record took 1,635,402
and 6,282,337; an idle boundary is unchanged (500 against 489).

The fiscal-limits change of 0.16.0 (see
[`docs/proposals/fiscal-and-society-limits.md`](../docs/proposals/fiscal-and-society-limits.md))
was recorded separately on 2026-10-14, alternating a 0.15.0 build of the
harness with a build of the change, round by round, before the allocation
mode existed, so these runs are elapsed only. Its 0.15.0 runs are its own,
named `0.15.0-fiscal-limits`, and predate `settlement_measure`. The machine
was running other builds throughout, so the boundary times vary by a third
between rounds and are context, not a comparison; the component costs,
timed inside one process, are the figures to read.

| Run | 0.15.0 | 0.16.0 fiscal limits |
| --- | --- | --- |
| Round 1 | [`elapsed`](baselines/2026-10-14-fiscal-boundary-0.15.0-fiscal-limits-round1-elapsed.json) | [`elapsed`](baselines/2026-10-14-fiscal-boundary-0.16.0-fiscal-limits-round1-elapsed.json) |
| Round 2 | [`elapsed`](baselines/2026-10-14-fiscal-boundary-0.15.0-fiscal-limits-round2-elapsed.json) | [`elapsed`](baselines/2026-10-14-fiscal-boundary-0.16.0-fiscal-limits-round2-elapsed.json) |
| Round 3 | [`elapsed`](baselines/2026-10-14-fiscal-boundary-0.15.0-fiscal-limits-round3-elapsed.json) | [`elapsed`](baselines/2026-10-14-fiscal-boundary-0.16.0-fiscal-limits-round3-elapsed.json) |

## World-scale fiscal profile

The standalone `fiscal-world-profile` binary measures `canwu-fiscal` on its
partitioned records at the scale of
[`docs/proposals/world-scale-fiscal.md`](../docs/proposals/world-scale-fiscal.md).
It generates, deterministically from `--seed`, a world of partitions with
heavy-tailed sizes, each with one catalog region, three central institutions,
eighteen regional offices, and twenty observers (one seeing every institution
of its partition, nineteen seeing one institution each), every scope with one
land rule implemented and one annual cycle. Identifiers are generated
(`scope.000017`, `authority.0003`). It measures, through the public API:

- the state: partitioning the authored state (`into_records`), activation of
  a new run, the host's audit of every fiscal record, fiscal record bytes and
  counts per kind and the largest record, before and after every scope is
  assessed, and, once every scope is assessed, `fork` of the unsealed run
  (`fork_ms`), snapshot bytes, and restore time;
- idle boundaries (one tick of another plugin, nothing fiscal due), on the
  initial state and once every scope holds a book;
- assessing bursts of 64, a spread day (the annual world's assessments
  spread over 365 days), 1,024, 4,096, 16,384, and every scope, each as the
  boundary that admits the batches (one per institution), the boundary that
  settles them, and the drain boundaries at the same instant that publish
  the reports the settling boundary could not;
- receipt boundaries of 1,024 and 16,384 receipts (packets of 4,096), after
  their collections are authorized and their evidence recorded;
- a close of every book; and
- archive commits of 256 and 4,096 closed books: preparing the batch from
  the run's records (`prepare_fiscal_archive`), storing, verifying, and
  queueing it, the settling boundary, finalizing the retention, and the
  acknowledging boundary. Up to 0.17.0 the host first assembled the whole
  state (`assemble_state_ms`).

Each boundary figure has, in an elapsed build, the share of the fiscal
handlers (`settlement_fiscal_handlers_ms`, and each handler in
`settlement_fiscal_handler_ms`) and the engine's own
(`settlement_engine_ms`), from the engine's boundary profile. Every boundary
of the last sample of a measurement is followed, outside the timed part, by
`audit_fiscal_records`, whose time is reported as `audit_after_ms`.

```console
cargo run --release \
  --manifest-path benchmarks/performance-harness/Cargo.toml \
  --bin fiscal-world-profile -- \
  --preset world \
  --machine YOUR-STABLE-MACHINE-LABEL \
  --recorded-on YYYY-MM-DD \
  --output benchmarks/baselines/YYYY-MM-DD-fiscal-world-world-elapsed.json
```

`--preset smoke` is 20 partitions and 2,000 scopes (five samples after one
warm-up); `world` is 200 partitions, 50,000 scopes (the largest partition
about 6,000), 4,200 institutions, and 4,000 observers (three samples after
one warm-up). `--mode allocations`, in a build with `--features
allocation-counting`, counts allocator calls as `fiscal-profile` does; the
two modes are separate builds.

Two options configure the observers (see
[`docs/proposals/fiscal-observers-at-scale.md`](../docs/proposals/fiscal-observers-at-scale.md)).
`--courts institution` puts each partition's court, its observer of every
institution, at institution granularity (one row per institution and
mechanism, summed over its books); the regional observers stay at scope
granularity, and the default `--courts scope` is the world of the 0.16.0
runs. `--cadence interval` gives every observer a monthly cadence,
`FiscalReportCadence::staggered(30 days, index, 30)`, so about a thirtieth of
them are due each day, instead of the default `--cadence on-change`. The
initial publication then settles the observers due at the start, which have
no book to report yet and are only queued for their next due time, the
bursts publish no report (none is due), the idle boundaries have 4,000
interval observers and nothing due, and two rows are added under
`interval_reports`:

- `spread_due_day`: after every scope is assessed, the boundary on the first
  day that a tick makes event-bearing, at which the observers due that day
  (`index % 30 == 1`, about 134) publish their first report with the burst's
  cycle, and the drains at its instant;
- `due_instant`: a second run of the same world whose observers are all due
  monthly at the same instants settles them at the start (as above),
  assesses every scope, and settles the boundary thirty days on, at which
  every observer is due owing one dirty cycle, and the drains at its
  instant: the gate case of the design, which settles in one boundary now
  that fiscal publishes against the engine's publication budget.

Each of the two rows records, besides the boundary figures, a `publication`
object for the measured boundary (record bodies, holder entries, payload
bytes as the engine's budget charges them, directives, which are the
`KnowledgePublished` events, and distinct holders), and, in an allocation
build, `heap_peak_bytes`: the most the heap grew during the boundary.

`--held-cycles N` adds `held_knowledge`: `N` monthly cycles on a fork of the
base run that seals its evidence after each month (so retained evidence does
not grow). Each month every scope is assessed on its first day, a tick each
day lets the observers due that day publish, and on its last day every book
is closed and archived in commits of 4,096 books. After each month it
records the record bodies, holder entries, and payload bytes the run holds,
the bytes of the continuation checkpoint, and the month's wall time up to
its seal; after the last, the time to restore that checkpoint. With the
default report retention the held knowledge stays flat.

The gate preset is `--preset world --courts institution --cadence interval
--held-cycles 24`.

`--rows interval`, with `--cadence interval` and no `--held-cycles`, measures
only the state, the idle boundaries, the spread due day, and the due
instant, on a run whose every scope is assessed without timing the bursts:
the rows of the due instant in a fresh process, in a few minutes rather than
the whole profile's hour.

The 2026-10-15 runs were recorded from the world-scale change's Phase 2
(before the version bump), on one machine, the elapsed runs with nothing else
running and the allocation runs alongside builds (allocation counts do not
depend on load): [`smoke` elapsed](baselines/2026-10-15-fiscal-world-smoke-elapsed.json),
[`smoke` allocations](baselines/2026-10-15-fiscal-world-smoke-allocations.json),
[`world` elapsed](baselines/2026-10-15-fiscal-world-world-elapsed.json), and
[`world` allocations](baselines/2026-10-15-fiscal-world-world-allocations.json).
Medians of the `world` elapsed run, in milliseconds (the settling boundary's
time split into the fiscal handlers' and the engine's own):

| Boundary | Admission | Settlement | Fiscal handlers | Engine | Drains at the same instant | Report records |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Idle (one tick, nothing fiscal due) | | 1.06 | 0.00 | | | |
| Assess 64 scopes | 27.5 | 68.3 | 15.6 | 53.5 | 1 boundary, 35.8 | 230 |
| A spread day (137 scopes) | 56.4 | 81.7 | 21.4 | 60.2 | 3, 119 | 458 |
| Assess 1,024 | 303 | 212 | 79.5 | 127 | 12, 780 | 2,519 |
| Assess 4,096 | 862 | 679 | 356 | 322 | 29, 2,071 | 6,642 |
| Assess 16,384 | 1,363 | 1,613 | 1,001 | 612 | 54, 2,966 | 11,271 |
| Assess every scope (50,000) | 2,139 | 4,790 | 3,134 | 1,644 | 62, 4,562 | 12,416 |
| 1,024 receipts | | 165 | 74.2 | 91.2 | 12, 811 | 2,519 |
| 16,384 receipts | | 1,872 | 1,113 | 681 | 54, 5,573 | 11,271 |
| Close every book (50,000) | 9,606 | 7,026 | 5,571 | 1,556 | none | 0 |
| Archive commit of 256 books | | 46.9 | 16.2 | 30.8 | none | 97 |
| Archive commit of 4,096 books | | 286 | 160 | 128 | 7, 112 | 1,193 |

The world holds 67,990 fiscal records: 33.8 MB of JSON as authored and
57.7 MB once every scope holds a book (ledgers about 1 KiB each, the
largest observer record 10.4 KiB, the catalog 68 KB). Partitioning the
authored state takes 0.7 s, activating a new run 8.7 s, and the host's audit
of every fiscal record 1.2 s. The host side of an archive commit is
dominated by assembling the committed state from every fiscal record: about
0.95 s before preparing (preparing and storing 4,096 books takes 0.11 s), and
again inside verifying and queueing (0.4 to 0.7 s) and finalizing (0.4 s).
After the burst of every
scope the unsealed snapshot is 1.09 GB of JSON, almost all of it retained
boundary evidence and published knowledge, and restoring it takes 231 s.

The 2026-10-16 runs were recorded on the 0.17.0 branch (the engine's shared
knowledge publication and fiscal's publication budget), on the same machine,
the elapsed run with nothing else running when checked:
[`world` institution interval elapsed](baselines/2026-10-16-fiscal-world-world-institution-interval-elapsed.json)
(`--preset world --courts institution --cadence interval --held-cycles 24`)
and [`world` institution interval allocations](baselines/2026-10-16-fiscal-world-world-institution-interval-allocations.json)
(the same without `--held-cycles`). With every observer on a monthly
cadence, the bursts, receipts, closes, and archive commits publish no
report (none is due); the reports are published on the due days. Medians of
the elapsed run, in milliseconds:

| Boundary | Admission | Settlement | Fiscal handlers | Engine | Drains at the same instant | Report records |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Idle (one tick, nothing fiscal due) | | 0.63 | 0.00 | | | |
| Assess 64 scopes | 17.1 | 8.97 | 4.62 | 4.35 | none | 0 |
| A spread day (137 scopes) | 34.4 | 15.2 | 7.73 | 7.79 | none | 0 |
| Assess 1,024 | 176 | 91.6 | 46.8 | 44.6 | none | 0 |
| Assess 4,096 | 473 | 326 | 187 | 139 | none | 0 |
| Assess 16,384 | 1,020 | 1,503 | 937 | 525 | none | 0 |
| Assess every scope (50,000) | 1,510 | 4,568 | 3,091 | 1,457 | none | 0 |
| Spread due day (134 observers due) | | 43.2 | 22.8 | 20.5 | none | 304 |
| Due instant (4,000 observers due) | | 2,504 | 1,821 | 676 | none | 9,603 |
| 1,024 receipts | | 90.6 | 48.7 | 41.9 | none | 0 |
| 16,384 receipts | | 1,693 | 1,113 | 572 | none | 0 |
| Close every book (50,000) | 1,626 | 5,052 | 3,690 | 1,362 | none | 0 |
| Archive commit of 256 books | | 26.6 | 10.3 | 16.3 | none | 0 |
| Archive commit of 4,096 books | | 228 | 132 | 97.6 | none | 0 |

The due instant settles in one boundary: 9,603 record bodies (one holder
entry each, 15.7 MB of payload) to 4,000 holders in 10 directives, within
fiscal's default share of the boundary's budget (16,384 bodies, 16,384
entries, 32 MiB), with nothing left for a drain. In the allocation build
the boundary makes 6.52 million allocator calls and the heap grows by at
most 472 MB during it (213,438 calls and 33 MB for the spread due day).

Over 24 months (`held_knowledge`) the run holds 14,138 report bodies and
entries (15.1 MB of payload) from the second month to the 24th: each holder
keeps its current reports only. A month of the world (every scope assessed,
28 daily ticks, every book closed and archived, the seal) takes 73 s in the
first month, about 111 s by the fifth as the records' outcome rings fill,
and about 129 s by the 24th, rising by about 1 s a month after the fifth,
which this profile does not break down. The continuation checkpoint grows
from 172 MB after the first month to 293 MB after the 24th, about 3.8 MB a
month from the fifth. That growth is not fiscal state or knowledge, which
stay flat, but the engine's request index, which keeps the idempotency
outcome of every tracked command (about 440 bytes a command on the smoke
preset). Restoring the 293 MB checkpoint takes 58 s.

The 0.18.0 runs (see
[`docs/proposals/fiscal-archive-and-due-instant-cost.md`](../docs/proposals/fiscal-archive-and-due-instant-cost.md))
measure the change that reads a book's figures without decoding its ledger,
encodes a report part compactly (version 6 of the `fiscal_report` schema),
and prepares archive commits from the run's records instead of the
assembled state. They were recorded on the same machine while other
workers' builds and a second profile ran, so their elapsed figures move by
a third or more between runs and are context; allocator calls, heap peaks,
and payload bytes do not depend on load. The
[allocation run](baselines/2026-10-16-fiscal-world-world-institution-interval-0.18.0-allocations.json)
(`--preset world --courts institution --cadence interval`) against the
0.17.0 one:

| Boundary | 0.17.0 allocator calls | 0.18.0 | 0.17.0 heap peak | 0.18.0 | Payload, 0.17.0 to 0.18.0 |
| --- | ---: | ---: | ---: | ---: | ---: |
| Due instant (4,000 observers due) | 6,516,716 | 4,261,604 | 472.1 MB | 305.6 MB | 15,671,758 to 5,545,628 bytes |
| Spread due day (134 observers due) | 213,438 | 155,798 | 33.3 MB | 28.7 MB | 437,768 to 156,327 bytes |
| Idle (one tick, nothing fiscal due) | 11,123 | 11,123 | | | |
| Archive commit of 256 books, host side | 6,154,725 | 98,265 | | | |
| Archive commit of 4,096 books, host side | 6,901,382 | 1,192,755 | | | |

The host side of an archive commit is preparing, verifying and queueing,
and finalizing (in 0.17.0 also assembling the state first); every other row
is within 0.01 % of 0.17.0.

The due instant in a fresh process, `--rows interval`, three rounds
alternating a build of the 0.17.0 code with this harness against the change
(`2026-10-16-fiscal-world-interval-rows-{0.17.0,0.18.0}-round{1,2,3}-elapsed.json`),
medians in milliseconds with the fiscal handlers' share:

| Round | 0.17.0 due instant | 0.18.0 due instant | 0.17.0 spread day | 0.18.0 spread day |
| --- | ---: | ---: | ---: | ---: |
| 1 | 1,827 (fiscal 1,108) | 1,546 (869) | 67.4 (35.9) | 62.7 (34.0) |
| 2 | 2,205 (1,492) | 1,259 (714) | 191 (65.6) | 65.4 (28.0) |
| 3 | 4,054 (2,424) | 1,858 (1,082) | 68.6 (34.9) | 46.2 (21.3) |

A 0.17.0 round recorded with less load before the alternation
([`round0`](baselines/2026-10-16-fiscal-world-interval-rows-0.17.0-round0-elapsed.json))
took 1,259 ms at the due instant (fiscal 789, engine 470) and 45.4 ms on the
spread day: in a fresh process the due instant costs fiscal about 80 µs a
record against about 73 µs on a spread day, so the 2,504 ms of the 0.17.0
gate run, measured after its 24 held months in the same process, was mostly
that process's state.

The [gate run](baselines/2026-10-16-fiscal-world-world-institution-interval-0.18.0-elapsed.json)
(`--preset world --courts institution --cadence interval --held-cycles 24`)
ran with the machine at 77 to 94 % load from other builds, so rows the
change does not touch took 1.1 to 3.4 times their 0.17.0 times (assessing
every scope 8,102 ms against 4,568, closing every book 5,693 against 5,052,
restoring the 24-month continuation checkpoint 196 s against 58 s): read it
for what the change does, not as a timing of the release. Even so, the due
instant took 2,160 ms (fiscal 1,219, engine 941) against 2,504 (fiscal
1,821, engine 676), and the host side of an archive commit 166 ms for 256
books (preparing 77, verifying and queueing 89, finalizing 0.06) against
1,402, and 501 ms for 4,096 books (167, 334, 0.1) against 1,813. A month's
wall time (100 to 240 s, following the load) does not compare with
0.17.0's. Held knowledge stays flat at 14,138 bodies and entries, now
5.67 MB of payload against 15.13 MB, and the continuation checkpoint is
about 10 MB smaller (162 MB after the first month and 284 MB after the
24th, against 172 and 293), still growing about 3.9 MB a month through the
engine's request index in that run, which was measured before request
outcome retention was integrated. With it
([`docs/proposals/request-outcome-retention.md`](../docs/proposals/request-outcome-retention.md),
the same preset and cadence), the checkpoint grows 0.85 MB in all from
month 5 (205.45 MB) to month 24 (206.30 MB): its request index holds one
month of outcomes and receipts and two retired ranges a month, and on the
smoke preset the compact checkpoint's dependencies and kept receipts stay at
8,039 for twelve months instead of growing by 2,520 a month.

## Knowledge-ledger profile

The standalone `knowledge-ledger-profile` binary measures what a boundary
pays for the generic knowledge a run holds. It fills a run the way a
reporting plugin does: `--holders` holders (4), each with `--lines` lines
(16) told apart by a subject; every fill boundary publishes `--per-line`
records (32) on every line, each superseding the line's current records,
which it finds through `SimulationView::knowledge_heads`. Most held records
are therefore superseded history, and `--payload-bytes` (256) pads each
payload. For each `--held` count it measures, on forks of the built run, each
after one unmeasured boundary of the same kind:

- `idle_boundary`: a boundary in which no system is due;
- `publish_boundary`: one system publishes one record superseding its line;
- `heads_boundary`: one system calls `knowledge_heads` `--queries` times (64)
  and publishes nothing;
- `snapshot_restore`: loading the run from its snapshot JSON.

It follows the conventions of the keyed-draw profile: separate elapsed and
allocation builds, positive odd sample counts, `run_heap_bytes` in the
allocation build, and a `source.content_hash` over the engine sources it
measures. It uses only public API that 0.15.0 also has.

```console
cargo run --release \
  --manifest-path benchmarks/performance-harness/Cargo.toml \
  --bin knowledge-ledger-profile -- \
  --mode elapsed \
  --held 0,2048,16384,65536 \
  --samples 31 \
  --warmup 3 \
  --restore-samples 3 \
  --machine YOUR-STABLE-MACHINE-LABEL \
  --recorded-on YYYY-MM-DD \
  --source-label "WHAT-WAS-BUILT" \
  --output benchmarks/baselines/YYYY-MM-DD-knowledge-ledger-VERSION-elapsed.json

cargo run --release \
  --manifest-path benchmarks/performance-harness/Cargo.toml \
  --features allocation-counting \
  --bin knowledge-ledger-profile -- \
  --mode allocations \
  --held 0,2048,16384,65536 \
  --samples 3 \
  --warmup 1 \
  --restore-samples 1 \
  --machine YOUR-STABLE-MACHINE-LABEL \
  --recorded-on YYYY-MM-DD \
  --source-label "WHAT-WAS-BUILT" \
  --output benchmarks/baselines/YYYY-MM-DD-knowledge-ledger-VERSION-allocations.json
```

The reports compare 0.15.0, built in an export of the `v0.15.0` tag with this
binary's source copied in, with the change, recorded before the version bump,
so their `engine_version` reads `0.15.0`. The two elapsed reports were
recorded back to back on 2026-10-15; allocation counts do not depend on the
day:
[`0.15.0 elapsed`](baselines/2026-10-15-knowledge-ledger-0.15.0-elapsed.json),
[`0.16.0 elapsed`](baselines/2026-10-15-knowledge-ledger-0.16.0-elapsed.json),
[`0.15.0 allocations`](baselines/2026-10-14-knowledge-ledger-0.15.0-allocations.json), and
[`0.16.0 allocations`](baselines/2026-10-15-knowledge-ledger-0.16.0-allocations.json).
They are interpreted in the proposal linked above.

## Record-write profile

The standalone `record-write-profile` binary measures what the engine does for
each domain record a boundary writes, the Phase 0 probe of
[`docs/proposals/world-scale-fiscal.md`](../docs/proposals/world-scale-fiscal.md).
A probe plugin builds a store of `--store` records (65,536 by default) of one
payload size, then times boundaries that update the first N of them
(`--counts`, 1,000, 10,000, and 50,000 by default) for each payload size
(`--sizes`, 1, 2, 4, and 8 KiB by default):

- `idle`: the probe system runs and writes nothing;
- `update`: N updates whose previous version has the same size;
- `update_from_small`: the same N updates from a one-entry previous version,
  so the difference is what the engine does with the previous record.

The engine's share of each boundary is the boundary profile's total less the
probe handler's own time (or allocations), and it is fitted against N and then
against the record's KiB as `a + b * KiB` per write. Each measured boundary is
followed by one that writes nothing and admits the events the writes emitted;
it is timed on its own. The elapsed build also times the encodings a write
performs, rebuilt in the harness (`reproduced_terms`), and bisects the largest
record a paged checkpoint can hold. `--shape text` makes every payload one
string of the same size instead of generated ledger entries, which separates
the cost of a record's bytes from that of its JSON values.

```console
cargo run --release \
  --manifest-path benchmarks/performance-harness/Cargo.toml \
  --bin record-write-profile -- \
  --mode elapsed --samples 3 --warmup 1 \
  --machine YOUR-STABLE-MACHINE-LABEL \
  --recorded-on YYYY-MM-DD \
  --output benchmarks/baselines/YYYY-MM-DD-record-write-entries-elapsed.json

cargo run --release \
  --manifest-path benchmarks/performance-harness/Cargo.toml \
  --features allocation-counting \
  --bin record-write-profile -- \
  --mode allocations --samples 1 --warmup 0 \
  --machine YOUR-STABLE-MACHINE-LABEL \
  --recorded-on YYYY-MM-DD \
  --output benchmarks/baselines/YYYY-MM-DD-record-write-entries-allocations.json
```

Recorded on 0.15.0 sources:
[`entries, elapsed`](baselines/2026-10-15-record-write-entries-elapsed.json),
[`text, elapsed`](baselines/2026-10-15-record-write-text-elapsed.json) (counts
1,000 and 10,000), and
[`entries, allocations`](baselines/2026-10-15-record-write-entries-allocations.json).
Their `source.content_hash` names the tree just before one lint-only change to
the probe (its thread-local payload template got a `const` initializer); the
measured code is otherwise the committed one. The machine was loaded by other
builds throughout, so single elapsed cases moved by up to a third between
neighbouring runs; the allocation counts do not depend on load.

## Record-store profiles

The record-store change of 0.16.0 was measured with `record-write-profile`
above and a second binary, `paged-record-profile`. It measures `Canwu::fork`
of a run holding `--records` (65,536) small records (`fork`) and the first
boundary on a fresh fork, which writes one record (`fork_first_boundary`), so
that a copy a fork defers to its first append is measured too; and, on forks
of that run, an idle boundary and one
boundary that replaces one record with a payload of each `--write-bytes` size,
after one unmeasured boundary that writes another record; and, for each
`--paged` set of record sizes, preparing a paged checkpoint of a run holding
exactly those records into an empty page store and restoring it with its
evidence prefix. A set the engine cannot page is reported with the error it
returned. It follows the conventions of the keyed-draw profile and uses only
public API that the integration base also has.

```console
cargo run --release \
  --manifest-path benchmarks/performance-harness/Cargo.toml \
  --bin record-write-profile -- \
  --mode elapsed \
  --sizes 1024,2048,4096,8192 \
  --counts 1000,10000 \
  --samples 5 \
  --warmup 1 \
  --machine YOUR-STABLE-MACHINE-LABEL \
  --recorded-on YYYY-MM-DD \
  --source-label "WHAT-WAS-BUILT" \
  --output benchmarks/baselines/YYYY-MM-DD-record-write-LABEL-elapsed.json

cargo run --release \
  --manifest-path benchmarks/performance-harness/Cargo.toml \
  --bin paged-record-profile -- \
  --mode elapsed \
  --samples 31 \
  --warmup 3 \
  --paged-samples 5 \
  --machine YOUR-STABLE-MACHINE-LABEL \
  --recorded-on YYYY-MM-DD \
  --source-label "WHAT-WAS-BUILT" \
  --output benchmarks/baselines/YYYY-MM-DD-paged-record-LABEL-elapsed.json
```

Allocation reports come from the same commands with `--features
allocation-counting` and `--mode allocations` (3 samples, 1 warm-up, 1 paged
sample). The 2026-10-15 reports compare the 0.16.0 integration base
(`12d4775`, built in an export with these binaries' sources copied in; label
`base`) with the record-store change before the version bump (label
`record-store`); every `engine_version` field reads `0.15.0`, and the
record-store reports were recorded before the review fixes (documentation,
comments, and the error code of an oversized record at paged preparation),
and on the integration base before the resource, production, law,
information, and fiscal changes joined it, so their `source.content_hash`
differs from the committed tree's. The architecture-suite allocation reports
of the same two builds are beside them. They are interpreted in
[`docs/proposals/record-store-ceiling-and-cost.md`](../docs/proposals/record-store-ceiling-and-cost.md):

| Profile | Base | Record store |
| --- | --- | --- |
| Record writes | [`elapsed`](baselines/2026-10-15-record-write-0.16.0-base-elapsed.json), [`allocations`](baselines/2026-10-15-record-write-0.16.0-base-allocations.json) | [`elapsed`](baselines/2026-10-15-record-write-0.16.0-record-store-elapsed.json), [`allocations`](baselines/2026-10-15-record-write-0.16.0-record-store-allocations.json), [`elapsed at 50,000 writes`](baselines/2026-10-15-record-write-0.16.0-record-store-50k-elapsed.json) |
| Paged records | [`elapsed`](baselines/2026-10-15-paged-record-0.16.0-base-elapsed.json), [`allocations`](baselines/2026-10-15-paged-record-0.16.0-base-allocations.json) | [`elapsed`](baselines/2026-10-15-paged-record-0.16.0-record-store-elapsed.json), [`allocations`](baselines/2026-10-15-paged-record-0.16.0-record-store-allocations.json) |
| Architecture suite | [`allocations`](baselines/2026-10-15-architecture-0.16.0-base-allocations.json) | [`allocations`](baselines/2026-10-15-architecture-0.16.0-record-store-allocations.json) |

The 2026-10-16 reports compare the 0.18.0 integration branch before and
after the runtime-state-copies change (shared current state, scheduler, and
journals; see
[`docs/proposals/runtime-state-copies.md`](../docs/proposals/runtime-state-copies.md)),
default options except `--paged 900000`:
[`before elapsed`](baselines/2026-10-16-paged-record-0.18.0-base-elapsed.json),
[`after elapsed`](baselines/2026-10-16-paged-record-0.18.0-state-copies-elapsed.json),
[`before allocations`](baselines/2026-10-16-paged-record-0.18.0-base-allocations.json), and
[`after allocations`](baselines/2026-10-16-paged-record-0.18.0-state-copies-allocations.json).

## Resource root profile

`resource-root-profile` measures what one boundary costs the resource plugin
as the hot terminal backlog of its root grows. Each `--backlogs` case installs
a root holding that many hot terminal records (rejected operation outcomes of
about 1.1 KB, placed in the root directly), then settles, `--samples` times
after `--warmup` unmeasured rounds: one requester-scoped allocation pass that
settles as a rejection (`operation`), eight such passes in one boundary
(`batch_of_8`), and a boundary that admits only a packet of a tick plugin
(`idle`). After the samples it settles one archive commit of every hot record
(`archive_commit`, one sample). Only `settle_boundary` is measured; the
evidence each boundary appends to the journal is reported as encoded JSON.
`--mode allocations`, in a build with `--features allocation-counting`,
reports allocator calls and requested bytes instead of time.

`--accounts` (default `0`) crosses every backlog with roots that also hold
that many accounts of other custodians, and reports `live_root_bytes` for
each case. `--households` (default none) adds one lease case per count: a
root with one account and one allocated demand per household, settled in
memory (`apply_operation`) once with one single-step lease and one
consumption per household (`per_household`) and once as consumption batches
of at most 256 items under one lease each (`batch`); each reports the lease
and settlement times, the live root's size before, with the leases active,
and after, the finished leases left in the root, and the hot terminal
records added per consumption. The scale options time single boundaries
that raised limits allow, on fresh roots under the canonical limits:
`--pooled` one allocation pass of a pooled demand beside that many accounts
(which reads every account), `--due` one allocation pass over that many due
demands, and `--boundary-batch` one consumption batch of that many items
through a provider's adapter ingress under a four-step lease. The 0.18.0
reports,
[`before`](baselines/2026-10-16-resource-root-0.18.0-before-elapsed.json) (the
base `8fb922e` with these options) and
[`after`](baselines/2026-10-16-resource-root-0.18.0-after-elapsed.json), and the
[scale cases](baselines/2026-10-16-resource-root-0.18.0-after-scale-elapsed.json), are
interpreted in
[`docs/proposals/resource-series-scale.md`](../docs/proposals/resource-series-scale.md).

```console
cargo run --release \
  --manifest-path benchmarks/performance-harness/Cargo.toml \
  --bin resource-root-profile -- \
  --mode elapsed \
  --backlogs 0,256,1024,4000 \
  --samples 5 \
  --warmup 1 \
  --label WHAT-WAS-BUILT \
  --machine YOUR-STABLE-MACHINE-LABEL \
  --recorded-on YYYY-MM-DD \
  --output benchmarks/baselines/YYYY-MM-DD-resource-root-LABEL-elapsed.json
```

A backlog above 4,000 uses the largest candidate budget, which only a build
with the 0.16.0 ceiling accepts. The 2026-10-15 reports compare four builds:
the integration base `f511463` (label `base`), this change on it
(`base-split`), the integration base with the record store package `305193a`
(`record-store`), and this change on that (`record-store-split`, with a
16,000-record case beside it). The builds before the change ran the same
profile with two lines changed, because the root then installed as one record
(`into_record` instead of `into_records`, and no ledger refresh). Each report's
`source` names the checkout the binary ran in, and its `build` object the
build itself. They are interpreted in
[`docs/proposals/resource-root-write-cost.md`](../docs/proposals/resource-root-write-cost.md):

| Build | Reports |
| --- | --- |
| Base | [`elapsed`](baselines/2026-10-15-resource-root-0.16.0-base-elapsed.json), [`allocations`](baselines/2026-10-15-resource-root-0.16.0-base-allocations.json) |
| Split on the base | [`elapsed`](baselines/2026-10-15-resource-root-0.16.0-base-split-elapsed.json), [`allocations`](baselines/2026-10-15-resource-root-0.16.0-base-split-allocations.json) |
| Record store | [`elapsed`](baselines/2026-10-15-resource-root-0.16.0-record-store-elapsed.json), [`allocations`](baselines/2026-10-15-resource-root-0.16.0-record-store-allocations.json) |
| Split on the record store | [`elapsed`](baselines/2026-10-15-resource-root-0.16.0-record-store-split-elapsed.json), [`allocations`](baselines/2026-10-15-resource-root-0.16.0-record-store-split-allocations.json), [`elapsed at 16,000`](baselines/2026-10-15-resource-root-0.16.0-record-store-split-16000-elapsed.json), [`allocations at 16,000`](baselines/2026-10-15-resource-root-0.16.0-record-store-split-16000-allocations.json) |

## Recorded information-flow smoke baseline

The 2026-08-21 smoke evidence consists of separate
[`elapsed`](baselines/2026-08-21-information-flow-smoke-elapsed.json) and
[`allocation`](baselines/2026-08-21-information-flow-smoke-allocations.json)
reports plus a concise [interpretation](2026-08-21-information-flow.md). The
elapsed pass used three operation samples, one growth sample, and one warmup;
the allocation pass used one measured sample and no warmup. Both were recorded
from a dirty integration worktree, and each JSON artifact preserves that fact,
the exact source fingerprints, compiler/target details, workload counts, raw
samples, and serialized sizes.

| Knowledge records | Holders | Hot-holder records | Snapshot | Growth median | Current-head page | Paged hot history | Load + validate | Exact replay | Compact initial seal |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 100 | 51 | 50 | 240,506 B | 0.590 ms | 0.060 ms | 0.060 ms | 3.500 ms | 0.560 ms | 0.800 ms |
| 1,000 | 60 | 500 | 1,585,855 B | 3.790 ms | 0.500 ms | 0.570 ms | 22.480 ms | 2.440 ms | 2.620 ms |
| 10,000 | 60 | 5,000 | 14,954,774 B | 38.250 ms | 4.990 ms | 25.190 ms | 327.890 ms | 24.870 ms | 25.790 ms |

These measurements expose two important costs rather than hiding them. Snapshot
load and validation requested about 422.60 MiB of allocator traffic at 10,000
records, while a five-page hot-holder history scan requested about 82.38 MiB.
The current implementation has no persisted query index: holder-local IDs,
read-cut roots, and cursor bindings are derived from the canonical ledger on
read.

The fixed public-API stress cases produced the following elapsed medians and
operation-local allocation traffic:

| Stress case | Scale | Median | Allocated |
| --- | ---: | ---: | ---: |
| Knowledge-schema registration | 100 schemas | 2.990 ms | 7.25 MiB |
| Addressed dispatch planning | 10,000 recipients | 5.100 ms | 14.87 MiB |
| Explicit audience planning | 10,000 members | 7.470 ms | 9.36 MiB |
| Mixed-lineage validation | 1,000 nodes | 0.110 ms | 0.15 MiB |
| Access index construction | 100,000 records / 1,000 holders | 82.630 ms | 60.52 MiB |
| Access query, all records | 100,000 records | 7.640 ms | 2.00 MiB |
| One access query per holder | 1,000 queries / 100,000 records | 3.778 s | 1.99 MiB |
| Archive-provider build | 100 segments | 116.850 ms | 271.63 MiB |
| Archive-provider restore | 100 segments | 6.480 ms | 8.99 MiB |

The 1,000-holder fan-out is now explicit rather than implied by fixture shape.
Its 4.731-second batch reveals that the detached public record set scans the
full 100,000-record map for every holder query; a holder/reference index is a
clear optimization candidate before this becomes a common AI or UI query path.
The elapsed smoke process peaked at 405,487,616 bytes (386.70 MiB) resident memory.
Windows uses `GetProcessMemoryInfo.PeakWorkingSetSize`; Linux and macOS use
`getrusage(RUSAGE_SELF).ru_maxrss` with platform-correct units. Unsupported
targets report a null sample and an explicit reason. These are cumulative
process high-water marks, not per-case deltas.

## Recorded information-flow full baseline

The final full evidence is recorded in
[`baselines/2026-08-21-information-flow-full-elapsed.json`](baselines/2026-08-21-information-flow-full-elapsed.json)
and interpreted in [`2026-08-21-information-flow.md`](2026-08-21-information-flow.md).
It uses three operation samples, one growth sample, no warmup, and the exact
10,000 / 100,000 / 1,000,000 scales. The million-record snapshot is
1,505,773,177 bytes; exact replay throughput is 401,214, 5,552, and 371
records/s at those three scales respectively. The final process high-water mark
is 22,294,740,992 bytes (20.76 GiB).

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

## Hashing-module extraction comparison

The behavior-preserving hashing extraction is recorded in separate
[`elapsed`](baselines/2026-08-16-hashing-module-elapsed.json) and
[`allocation`](baselines/2026-08-16-hashing-module-allocations.json) reports,
using the migration-module milestone as its before baseline. It moves canonical
hash material, collection roots, checkpoint commitments, boundary-chain hashes,
and hash-format validation into `canwu-sim/src/hashing.rs`. Persistence,
migration, replay, and validation retain the same hash contract through a
focused module boundary. `canwu-sim/src/lib.rs` falls from 13,452 to 12,645
nonblank source lines, with 850 nonblank source lines in the new module.

Every measured allocation sample and summary, history count, checkpoint hash,
checkpoint-storage size, and flat snapshot size is identical at all four
scales. At scale 512, history growth changes by +0.1%, populated boundaries
+3.0%, snapshot creation -2.1%, snapshot serialization -0.3%, load validation
+1.1%, and exact replay +0.3%. The accepted-command median moves from 0.972 to
0.823 ms (-15.3%), while the empty-boundary median moves from 0.450 to 0.490 ms
(+8.9%); both remain sub-millisecond cases. These are treated as local timing
noise. The milestone makes no runtime-performance claim; it gives every
authoritative commitment and historical hash format one explicit dependency
surface while preserving deterministic, insertion-order-independent roots and
legacy migration behavior.

## Settlement-module extraction comparison

The behavior-preserving settlement extraction is recorded in separate
[`elapsed`](baselines/2026-08-16-settlement-module-elapsed.json) and
[`allocation`](baselines/2026-08-16-settlement-module-allocations.json) reports,
using the hashing-module milestone as its before baseline. It moves boundary
orchestration, phase visibility, reservation allocation, boundary mutation and
emission application, generated ingress, and settlement-specific proposal
validation into `canwu-sim/src/settlement.rs`. Shared scheduling, command/event
directive validation, persistence, replay, and hashing remain in their existing
modules. `canwu-sim/src/lib.rs` falls from 12,645 to 11,286 nonblank source
lines, with 1,381 nonblank source lines in the new module.

Every measured allocation sample and summary, history count, checkpoint hash,
checkpoint-storage size, and flat snapshot size is identical at all four
scales. At scale 512, history growth changes by +1.2%, empty boundaries -2.1%,
populated boundaries -0.8%, snapshot creation +5.6%, load validation +5.3%, and
exact replay +0.7%. The accepted-command median moves from 0.823 to 0.860 ms
(+4.6%), and the rejected-command median moves from 0.064 to 0.070 ms (+10.2%)
on a sub-0.1-ms case. These are treated as local timing noise. The milestone
makes no runtime-performance claim; it gives atomic settlement and its
visibility/allocation rules a dedicated dependency surface while preserving
the sequential reference path and exact evidence contract.

## Ingress-module extraction comparison

The behavior-preserving ingress extraction is recorded in separate
[`elapsed`](baselines/2026-08-16-ingress-module-elapsed.json) and
[`allocation`](baselines/2026-08-16-ingress-module-allocations.json) reports,
using the settlement-module milestone as its before baseline. It moves command
submission/admission, request idempotency, revision guards, rejection evidence,
canonical ingress enqueueing, and due-queue driving into the existing
`canwu-sim/src/ingress.rs` model module. Settlement, replay, persistence, and
command execution keep the same crate-internal admission surface.
`canwu-sim/src/lib.rs` falls from 11,286 to 10,544 nonblank source lines, while
the combined ingress model/runtime module contains 876 nonblank source lines.

Every measured allocation sample and summary, history count, checkpoint hash,
checkpoint-storage size, and flat snapshot size is identical at all four
scales. At scale 512, history growth changes by -0.7%, accepted commands -4.5%,
rejected commands +4.3%, empty boundaries -4.6%, populated boundaries -2.6%,
snapshot creation -2.8%, load validation -2.0%, and exact replay -1.1%. These
are treated as local timing noise. The milestone makes no runtime-performance
claim; it gives authoritative ingress ordering and command-admission evidence a
dedicated dependency surface while preserving idempotency, revision, rejection,
and replay contracts.

## Scheduling-module extraction comparison

The behavior-preserving scheduling extraction is recorded in separate
[`elapsed`](baselines/2026-08-16-scheduling-module-elapsed.json) and
[`allocation`](baselines/2026-08-16-scheduling-module-allocations.json) reports,
using the ingress-module milestone as its before baseline. It moves public time
advancement, scheduled-batch execution, arrival and knowledge-report handling,
event emission, runtime random draws/outcomes, scheduler insertion, and runtime
plugin views into `canwu-sim/src/scheduling.rs`. Command, settlement, ingress,
and replay callers retain crate-internal execution methods.
`canwu-sim/src/lib.rs` falls from 10,544 to 9,924 nonblank source lines, with
640 nonblank source lines in the new module.

Every measured allocation sample and summary, history count, checkpoint hash,
checkpoint-storage size, and flat snapshot size is identical at all four
scales. At scale 512, history growth changes by +0.5%, accepted commands -0.3%,
empty boundaries +5.6%, populated boundaries -2.0%, snapshot creation -1.6%,
load validation -4.0%, exact replay +1.0%, and repeated live-evidence sealing
+3.1%. The rejected-command median moves from 0.073 to 0.066 ms (-9.7%), while
caller-owned segment release moves from 0.236 to 0.268 ms (+13.5%) on a
sub-0.3-ms case. These are treated as local timing noise. The milestone makes
no runtime-performance claim; it gives deterministic time, scheduled action,
event, and runtime-random execution one explicit dependency surface while
preserving sequential ordering and rollback behavior.

## Typed-domain foundation comparison

The additive typed-domain foundation is recorded in separate
[`elapsed`](baselines/2026-08-16-typed-domain-elapsed.json) and
[`allocation`](baselines/2026-08-16-typed-domain-allocations.json) reports,
using the scheduling-module milestone as its before baseline. Domain packages
can declare `DomainRecordType` with a sealed entity/value class, construct
`TypedDomainRecordRef<T>` identities, build typed schemas and drafts, decode
typed payloads, and query typed identities through the simulation and public
public API. Entity and value capabilities derive automatically from the class.
Typed references retain the existing `DomainRecordRef` wire shape and validate
the namespaced kind during deserialization, so checkpoint and snapshot formats
remain unchanged.

Every measured allocation sample and summary, history count, checkpoint hash,
checkpoint-storage size, and flat snapshot size is identical at all four
scales. At scale 512, history growth changes by -2.3%, accepted commands +0.8%,
empty boundaries -7.2%, populated boundaries -1.9%, snapshot creation +2.5%,
snapshot serialization +2.2%, load validation +1.9%, and exact replay -1.4%.
The rejected-command median moves from 0.066 to 0.071 ms (+8.0%) on a
sub-0.1-ms case, while caller-owned segment release moves from 0.268 to 0.237 ms
(-11.4%) on a sub-0.3-ms case. These are treated as local timing noise. The
milestone makes no runtime-performance claim; it adds compile-time package
binding while preserving the authoritative storage, hashing, migration, and
replay contracts.

## Representative-conformance comparison

The engine-neutral public conformance fixture and fixture-identity cleanup are
recorded in separate
[`elapsed`](baselines/2026-08-16-representative-conformance-elapsed.json) and
[`allocation`](baselines/2026-08-16-representative-conformance-allocations.json)
reports, using the typed-domain milestone as the before baseline. The release
runtime path is unchanged; this milestone adds one public integration fixture,
updates architecture documentation, and gives internal fixtures domain-neutral
identities.

Every measured allocation sample and summary, history count, checkpoint hash,
checkpoint-storage size, and flat snapshot size is identical at all four
scales. Both reports contain 42 raw build-affecting source fingerprints matching
the measured tree. At scale 512, history growth changes by +1.1%, accepted
commands +0.7%, empty boundaries +1.2%, populated boundaries +0.9%, snapshot
creation -4.8%, load validation -2.7%, and exact replay -0.4%. Caller-owned
segment release moves from 0.237 to 0.257 ms (+8.2%) on a sub-0.3-ms case.
These are treated as local timing noise. The milestone makes no runtime-
performance claim.

## Deterministic parallelism decision

The final profile measures an empty boundary at 0.454 ms and a populated
boundary at 0.777 ms at scale 512, while complete retained-history construction
takes 518.231 ms and exact replay takes 517.693 ms. Allocation evidence is also
unchanged from the typed-domain baseline. These measurements justify deferring
concurrency, but they do not isolate proposal execution from the other work in
a boundary. Deterministic parallel proposal execution remains deferred until a
dedicated proposal-only profile shows that it materially dominates after the
remaining sequential costs are addressed. The sequential reference path remains
the canonical implementation and the future equivalence oracle for any parallel
backend.
