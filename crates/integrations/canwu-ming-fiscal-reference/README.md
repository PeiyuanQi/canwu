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
