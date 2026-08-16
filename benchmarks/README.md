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
