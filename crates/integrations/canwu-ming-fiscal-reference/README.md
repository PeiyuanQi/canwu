# canwu-ming-fiscal-reference

This non-published integration crate wires `canwu-fiscal`, the embedded
`canwu-ming-fiscal` data pack, and `canwu-reference-world` into runnable starts.

Available fixtures:

- `hongwu-1391`: registration, labor service, and grain obligations.
- `wanli-1581`: regional Single Whip adoption at different stages.
- `hongguang-1644`: fragmented Southern Ming court, military, and merchant finance.

Use `new_ming_fiscal_reference` for a runnable simulation,
`run_ming_fiscal_sample_cycle` for an assessment-to-report vertical slice, and
`restore_ming_fiscal_reference` or `replay_ming_fiscal_reference` for validated
snapshot restoration or exact replay. The integration registers the world,
typed fiscal-execution adapter, and configured fiscal plugins. The generic
fiscal plugin decodes exact typed adapter results at the live settlement
boundary and derives receipt quantity and disposition from them. The
integration's semantic validator repeats the complete match during validation
and restore. Within one fiscal runtime state, an exact result version or one
`(evidence kind, external_operation_id)` pair cannot settle different receipts.

The starter example also writes a human-oriented trace by default:

```text
artifacts/traces/ming-fiscal-reference/<fixture>/manifest.json
artifacts/traces/ming-fiscal-reference/<fixture>/steps.jsonl
```

Run it with `cargo run -p canwu-ming-fiscal-reference --example ming_fiscal_starter -- hongwu-1391`.
Pass `--trace-dir <path>` after the fixture ID to override the root directory,
or set `CANWU_TRACE_DIR`. To continue after the one-off sample cycle, pass
`--days <N>`; the starter schedules one deterministic calendar boundary per
step and writes every returned receipt. `--cadence daily|monthly|annual`
selects the cadence marker, while `--step-days <N>` overrides its fixed
simulation-day quantum. Monthly and annual defaults are 30 and 365 simulation
days because `SimTime` is minute-based and does not claim a Gregorian calendar.
If the horizon ends partway through a period, the starter records a final
cadence-free boundary instead of triggering monthly or annual systems early.
For interactive inspection, add `--open-viewer`; this starts a localhost-only
server, opens the default browser, and loads the generated trace automatically.
Use `--viewer-port <N>` to request a port (`0` selects an available port).
The same behavior can be enabled with `CANWU_OPEN_TRACE_VIEWER=1`. The viewer
requires the command to run from the Canwu workspace, or
`CANWU_WORKSPACE_ROOT` can point to that workspace explicitly.

For example:

```text
cargo run -p canwu-ming-fiscal-reference --example ming_fiscal_starter -- hongwu-1391 --days 365 --cadence daily
cargo run -p canwu-ming-fiscal-reference --example ming_fiscal_starter -- hongwu-1391 --days 365 --open-viewer
```

Each JSONL row contains the canonical boundary receipt, the persisted boundary
evidence, the complete fiscal state snapshot, and any holder-relative fiscal
projections that are available at that boundary. This layout is intentionally
stable so the HTML viewer can scan the manifest and stream `steps.jsonl`
without understanding the Rust runtime.
