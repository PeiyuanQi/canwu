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
- environment-bound exact replay.

Elapsed time and allocation traffic are collected in separate release builds.
Elapsed mode uses Rust's default system allocator with no counting wrapper.
Allocation mode enables a thread-local counting allocator and does not report
wall time. Both reports keep raw samples and summary medians; setup and
post-measurement drops are outside each sample. Snapshot byte size and exact
history counts are recorded beside every scale.

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

The allocation evidence identifies two distinct growth classes:

- A fourfold increase from scale 128 to 512 makes accepted/rejected commands,
  individual boundaries, snapshot creation, and snapshot loading request about
  four times as many allocation operations. These operations are linear in
  retained history in the current implementation.
- The same increase makes end-to-end history construction and exact replay
  request about sixteen times as many allocation operations and take about
  seventeen times as long. At scale 512, growth requested 45,551,539 allocation
  operations and 9,690,025,745 bytes; replay requested 67,297,025 operations and
  13,867,966,718 bytes. These paths exhibit approximately quadratic growth.

Those observations establish optimization targets; they do not change any
runtime contract or claim that one subsystem alone is the cause.
