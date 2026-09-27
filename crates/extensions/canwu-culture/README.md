# canwu-culture

`canwu-culture` is Canwu's published authoring and lifecycle extension for
Canwu. It compiles declarative culture definitions into deterministic plans,
adapts those plans to the generic `canwu-society` runtime, and retires inactive
culture targets without deleting historical evidence.

The crate intentionally does not own legal, economic, technological, military,
or per-person state. It emits bounded, evidence-bearing cultural signals;
downstream extensions decide whether a signal becomes an institutional fact.

The host-driven flow registers `CulturePlugin`:

1. build and `compile_culture` a definition;
2. `install_into_society` once, then create or `load_culture_runtime`;
3. call `settle_culture_society_boundary` to settle lifecycle observations and
   apply target-level society lifecycle deltas as one in-memory transaction;
4. persist `CultureRuntime::snapshot_state` and returned lifecycle transitions;
5. emit downstream batches only through compiled `emit_effect` bindings.

The plugin-driven flow registers `CultureBoundaryPlugin` beside
`canwu_society::SocietyPlugin`. The scenario installs the
`culture_definition_record`, the culture state record, and a society state with
`install_into_society` applied. The Monthly phase-7 system
`culture_lifecycle_settle_v1` then consumes admitted `culture_exposure_v1`
batches (`CultureExposureSignalBatch`, queued by an event-driven phase-12
intake, at most `MAX_CULTURE_EXPOSURE_QUEUE` at once; an initial scenario
cannot seed that queue), treats accepted institutional decisions on culture
alignments as admitted work (and any stored decision as a live dependency, so
a target an institution has decided on does not go dormant or retire), derives
engagement and live dependencies from the society snapshot, calls
`settle_culture_society_boundary` on that snapshot, persists
`canwu.culture:state`, hands the society plugin one
`society_lifecycle_delta_v1` ingress per target that needs one, and emits each
due compiled effect as a self-addressed `cultural_signal_batch_v1` ingress. A
rejected lifecycle step emits `culture_lifecycle_rejected_v1` and leaves
culture state unchanged; a retired culture state is inert. Phase-7 systems all
read the same boundary snapshot, so every hand-off is next-boundary: signal
batches are admitted at the next boundary, and the society applies the delta
at the first Daily settlement after it is admitted. The society plugin stays
the only writer of `canwu.society:state`. A host registers one of the two
plugins and never also calls `settle_culture_society_boundary` for a run that
uses `CultureBoundaryPlugin`.

Culture never rolls a committed lifecycle step back. A transitioned target's
delta is sent at its transition; at every later Monthly boundary culture
compares each target with the society snapshot and re-sends the delta that
brings society to the committed lifecycle: missing compiled bindings of an
active target, remaining compiled rules of a dormant or retired target, and
the remaining dynamic state of a retired target once society no longer
depends on it. Society applies deltas idempotently, so a duplicate sent while
one is in flight is harmless, and a delta society refused or whose release it
blocked (recorded in its ledger) is retried a month later.

Ordinary lifecycle boundaries inspect active targets, due dormant targets, and
explicit observations; they do not clone the runtime or scan tombstones.
`synchronize_society_lifecycle` is the explicit full-reconciliation path for
load repair and maintenance checkpoints. The current society solver still
performs full-state aggregation and projection. Dirty-pair settlement and a
published scale benchmark remain follow-up work.

The crate is an official optional release. Its API follows Canwu's pre-1.0
compatibility policy and may evolve in a future SemVer release.
