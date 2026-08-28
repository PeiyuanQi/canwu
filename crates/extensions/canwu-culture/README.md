# canwu-culture

`canwu-culture` is an experimental authoring and lifecycle extension for
Canwu. It compiles declarative culture definitions into deterministic plans,
adapts those plans to the generic `canwu-society` runtime, and retires inactive
culture targets without deleting historical evidence.

The crate intentionally does not own legal, economic, technological, military,
or per-person state. It emits bounded, evidence-bearing cultural signals;
downstream extensions decide whether a signal becomes an institutional fact.

The supported host flow is:

1. build and `compile_culture` a definition;
2. `install_into_society` once, then create or `load_culture_runtime`;
3. call `settle_culture_society_boundary` to settle lifecycle observations and
   apply target-level society lifecycle deltas as one in-memory transaction;
4. persist `CultureRuntime::snapshot_state` and returned lifecycle transitions;
5. emit downstream batches only through compiled `emit_effect` bindings.

Ordinary lifecycle boundaries inspect active targets, due dormant targets, and
explicit observations; they do not clone the runtime or scan tombstones.
`synchronize_society_lifecycle` is the explicit full-reconciliation path for
load repair and maintenance checkpoints. The current society solver still
performs full-state aggregation and projection. Dirty-pair settlement,
canonical cross-extension ingress, and a published scale benchmark remain
follow-up work.

The API may change before the crate has an independent consumer.
