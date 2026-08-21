# Information-flow performance report — 2026-08-21

This report records the neutral public knowledge-ledger and information-extension
profile. It covers schema-validated records, holder-relative knowledge, routing,
audiences, lineage, access, replay, and archival evidence without assuming a
particular period, medium, institution, or narrative.

## Method

The kernel profile registers one public `SimulationPlugin`, one writable schema,
and one phase-13 publication system. Each boundary publishes through the public
API and respects the limits of 1,000 records per batch, 64 batches per system,
and 10,000 records per boundary. Approximately half of each boundary's records
go to one hot holder; the rest are spread across 59 additional holders.

The final elapsed run used three operation samples, one history-growth sample,
and no warmup on local Windows x86-64 with Rust 1.97.1. It measured 10,000,
100,000, and 1,000,000 records. The full artifact is:

- `baselines/2026-08-21-information-flow-full-elapsed.json`

The smoke artifacts remain available for quick reruns:

- `baselines/2026-08-21-information-flow-smoke-elapsed.json`
- `baselines/2026-08-21-information-flow-smoke-allocations.json`

The harness fingerprints the relevant benchmark and engine sources before and
after a run and refuses to write a mixed-source result. These artifacts were
recorded from a dirty integration worktree, so they are milestone evidence and
not clean-release comparison points.

## Publication boundaries

| Records published | Median | P95 |
| ---: | ---: | ---: |
| 1 | 0.040 ms | 0.270 ms |
| 10 | 0.100 ms | 0.140 ms |
| 100 | 0.790 ms | 0.810 ms |
| 1,000 | 4.240 ms | 4.410 ms |

These cases include proposal construction, schema and grant checks, atomic
ledger append, publication evidence, state commitments, and boundary recording.
Setup is outside the measured interval.

## Full persisted history, queries, replay, and compact sealing

| Records | Holders | Hot-holder records | Snapshot | Growth | Current heads | Paged hot history | Load + validate | Exact replay | Replay throughput | Compact seal |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 10,000 | 60 | 5,000 | 14,954,774 B | 36.857 ms | 5.112 ms | 25.414 ms | 339.296 ms | 24.924 ms | 401,214 records/s | 25.686 ms |
| 100,000 | 60 | 50,000 | 149,858,400 B | 15,093.019 ms | 92.234 ms | 4,116.331 ms | 4,358.344 ms | 18,010.558 ms | 5,552 records/s | 464.789 ms |
| 1,000,000 | 60 | 500,000 | 1,505,773,177 B | 2,244,671.731 ms | 1,539.413 ms | 801,403.149 ms | 139,077.958 ms | 2,693,744.610 ms | 371 records/s | 23,194.746 ms |

The million-record scale uses 100 ordinary publication boundaries and does not
bypass the per-boundary limit. The hot holder owns half of the records, so its
full-history query is intentionally the expensive read-cut case.

## Fixed public-API stress cases

| Case | Exact scale | Median | P95 |
| --- | ---: | ---: | ---: |
| Knowledge-schema registration | 100 schemas | 2.920 ms | 3.010 ms |
| Addressed dispatch planning | 10,000 recipients | 5.300 ms | 5.580 ms |
| Explicit audience planning | 10,000 members | 7.770 ms | 7.800 ms |
| Mixed-lineage validation | 1,000 nodes | 0.110 ms | 0.140 ms |
| Access index construction | 100,000 records / 1,000 holders | 83.530 ms | 84.100 ms |
| Access query, all records | 100,000 records | 7.840 ms | 7.950 ms |
| One access query per holder | 1,000 queries / 100,000 records | 3,871.050 ms | 4,048.670 ms |
| Archive-provider build | 100 segments | 116.950 ms | 118.420 ms |
| Archive-provider restore | 100 segments | 6.540 ms | 6.950 ms |

The one-query-per-holder case is deliberately explicit: each of 1,000 holders
owns 100 records and receives one public filtered query. It currently scans the
detached record map for each holder, making a holder/reference index the clearest
future optimization candidate.

## Peak resident memory

The final elapsed process peak was 22,294,740,992 bytes (20.76 GiB). This is a
cumulative process high-water mark, not a per-case delta. Windows obtains it from
`GetProcessMemoryInfo.PeakWorkingSetSize`; unsupported platforms report an
explicit null value and reason. Allocation mode remains separate and measures
operation-local allocator traffic rather than resident memory.

The final run also changed the benchmark harness to keep million-record snapshot
and journal inputs on temporary files and load them per measured setup. This
preserves the operation interval while preventing simultaneous in-memory copies
of the fixture, snapshot JSON, checkpoint journal, and replay journal. The full
run completed without memory pressure or source-fingerprint drift.

## Reproduction

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

Elapsed time is machine- and load-sensitive. Compare results only when machine,
compiler, build profile, scales, warmup, and sample counts match.
