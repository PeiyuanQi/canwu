# canwu-ming-fiscal

This crate is the Apache-2.0 historical reference-content layer for Ming fiscal
simulation. It embeds a source-cited data pack covering the dynasty from 1368
through the Southern Ming in 1662, plus an explicitly optional Zheng maritime
continuation through 1683.

The coverage matrix is exhaustive over period, region, and mechanism. Every
cell resolves by numeric priority to `supported`, `archetype_fallback`,
`explicit_unknown`, or `not_applicable`; equal-priority overlaps fail closed.
The pack preserves regional reform timing and never turns the Single Whip reform
into an automatic empire-wide switch.

Three durable fixtures expose representative playable starts: Hongwu 1391,
Wanli 1581, and Hongguang 1644. The companion integration runs each as a full
assessment, authorization, typed execution-evidence, receipt, and reporting
cycle. The pack is a longitudinal catalog; a fixture is a bounded year slice,
not one mandatory live ledger for the entire 1368-1683 range.
