# Technology and historical-research home-hardware evidence

The 2026-08-22 profile measures `canwu-technology` with all three optional
historical assessment plugins enabled. Every measured month commits one
technology claim and one sources, practice, and production-archaeology
assessment through tracked commands and boundary settlement. The current
kernel still clones and validates broad state, so these are usable-scale
measurements rather than claims that paging removed boundary-wide costs.

## Environment and reproducibility

- AMD Ryzen 7 7800X3D, 8 cores / 16 logical processors
- 32 GiB RAM and WD_BLACK SN850X SSD
- Windows x86-64, Rust 1.98.0, release build
- one warmup and 31 measured ordinary technology operations per profile
- 240 measured 30-day workloads, each containing four real operations

Each profile ran in a fresh process. The JSON embeds the Git commit, dirty
status, compiler identity, and a BLAKE3 hash over the relevant source content.
Peak RSS is the process high-water mark. This is component evidence on the
named machine, not a whole-game 4-core/8-GiB certification.

| Profile | Sites / programs / links | Initial history | Operation p95 | Monthly batch p95 | Snapshot load | Checkpoint load | Exact 20-year replay | Peak RSS |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Paced interactive | 100 / 200 / 400 | 144 | 71.5 ms | 168.5 ms | 3.942 s | 5.657 s | 28.946 s | 647.6 MiB |
| Pressure | 500 / 1,000 / 2,000 | 768 | 403.4 ms | 616.3 ms | 14.915 s | 21.515 s | 129.742 s | 1.158 GiB |

| Profile | Flat snapshot | Current checkpoint | Checkpoint journal | Replay journal | Disk write + sync | Disk read after sync (likely warm) | Average snapshot growth/year |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Paced interactive | 18.91 MiB | 3.75 MiB | 22.58 MiB | 6.00 MiB | 23.5 ms | 9.0 ms | 782.1 KiB |
| Pressure | 32.80 MiB | 10.54 MiB | 38.77 MiB | 6.00 MiB | 42.2 ms | 14.2 ms | 782.1 KiB |

The fixed paced-interactive component profile meets the milestone's operation,
monthly-batch, flat-snapshot load, save, and RSS budgets on this machine.
Its 3.75 MiB whole-runtime current checkpoint conservatively upper-bounds the
serialized technology hot state. End-to-end snapshot serialization plus disk
write/sync was 102.0 ms for paced interactive and 194.9 ms for pressure; the
disk-only values remain separated in the table. Checkpoint-journal
reconstruction exceeds five seconds and complete replay is intentionally much
slower. Games should use the validated flat snapshot for normal loading, a
checkpoint bundle for recovery, and full replay for audit or offline
verification; the current-checkpoint byte count is not a standalone-load claim.
The paced profile's monthly p95 leaves 331.5 ms inside its 500 ms component
budget, but a whole-game build must still repeat this test with renderer, assets,
and the host's other monthly systems present before adopting the same target.

The pressure profile is not interactive. It stays inside the extension's
shared record budgets, but broad cloning and validation push both monthly work
and loading beyond the interactive targets. It is appropriate for campaign
turns and offline research analysis.

Both JSON reports carry source content fingerprint
`994c08be0c2aecd118baef8c50785269887f54892afe5c944f3b6dd2a68799aa` and clean
source status. Their generator commit IDs differ because each baseline was
captured from a clean source commit before the final report/documentation amend;
the content fingerprint is the authoritative source-equivalence check.
The reports are
[`interactive`](baselines/2026-08-22-technology-interactive-elapsed.json) and
[`pressure`](baselines/2026-08-22-technology-pressure-elapsed.json).

Reproduce either fresh-process profile from the repository root:

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

Use `--profile pressure` for the 500-site profile. Elapsed results are
machine- and load-sensitive; compare only runs with matching compiler, profile,
dimensions, warmup, and samples. A baseline is evidence, not a universal
service-level objective.
