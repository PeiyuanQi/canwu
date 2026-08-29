---
name: canwu-common-build-run-explorer
description: "Build or improve a shared Canwu run explorer for games and historical research simulations. Use when a Canwu client needs explicit random-seed reruns, exact same-seed reproduction, cross-seed comparisons, a simulation-time slider, collapsible tree tables, actor- or institution-relative perspectives, causal event inspection, save/fork/replay controls, or trusted research views without leaking authoritative ground truth."
---

# Build a Common Canwu Run Explorer

Build this interface in the downstream host application, not in Canwu core.
Use Canwu through `canwu-api`. If the simulation model does not exist yet, also
follow the applicable creator skill:
[`canwu-game-create`](../canwu-game-create/SKILL.md) for a
game or
[`canwu-history-create`](../canwu-history-create/SKILL.md)
for historical research.

## Define an immutable run identity

Store and display enough metadata to distinguish reproducibility from a new
experiment:

- run ID and parent/fork lineage;
- root seed and scenario/content/semantic-environment hashes;
- admitted input journal or experiment parameters;
- current simulation time and checkpoint hash;
- completion state and milestone summary.

"Run again" with the same seed and inputs verifies reproduction. "New seed"
creates a separate alternative run. Never mutate the old run's metadata or call
a different-seed run an exact replay.

## Capture perspective-safe timeline data

At selected committed boundaries, ask Canwu for each actor or institution's
authorized projection and store an immutable host-side trace for presentation.
Capture the trusted research view separately and label it clearly. Do not first
read ground truth and then hide columns in the UI; actor rows must originate
from actor-relative queries so omitted knowledge cannot leak through values,
counts, ordering, IDs, tooltips, or comparison summaries.

Each timeline frame should carry stable keys and enough provenance to explain
what the viewer knew:

```text
run_id, seed, sim_time, checkpoint_hash
perspective_id, perspective_kind, trusted
domain, subject_id, row_kind, visible_value
evidence_or_source, confidence, observed_at, information_age
causal_event_ids
```

Treat this trace as a derived presentation artifact, not authoritative mutable
simulation state.

## Build the interaction

- Drive the slider with deterministic simulation time, never render frames or
  wall-clock timestamps.
- Use a collapsible treegrid grouped as
  `run -> perspective -> domain -> record/event`.
- Give the treegrid stable columns for time, subject, visible state, evidence,
  confidence, and update age. Keep group rows compact and expandable.
- Preserve expansion and selected-time state while switching perspectives or
  runs. Do not let loading text, long IDs, or dynamic values resize controls.
- Provide tabs or a comparison mode for milestones, final outcomes, and a
  selected subject across seeds. Keep contradictory actor reports separate.
- Label trusted/admin/research rows visually and textually; never make them look
  like an ordinary actor perspective.
- Support keyboard operation for the slider, tree expansion, run selection,
  and table navigation. Verify narrow and wide layouts without overlap.

Use icons for replay, fork, save, expand, and compare commands when the host's
icon library provides them, with accessible names and tooltips. Use a numeric
seed input plus a clear run command, not a free-form text control for run mode.

## Wire replay and comparison correctly

1. Save snapshots with the run metadata and exact plugin descriptors.
2. Rehydrate required plugin handlers before advancing a restored run.
3. Use exact replay to verify a recorded run and `fork()` when changing future
   admitted inputs from a shared checkpoint.
4. Use a new root seed only for a new run from the initial scenario.
5. Compare by stable simulation-time milestones, causal records, and actor
   observations. Wall-clock duration is diagnostics, not historical time.

## Verify the explorer

- Same seed plus same inputs yields the same checkpoint and timeline trace.
- A selected set of seeds can be rerun independently and compared without
  overwriting prior results.
- Every configured perspective appears, including perspectives with no visible
  event at the selected time.
- Actor views reveal no hidden values, hidden IDs, or ground-truth-derived row
  counts.
- Restored and replayed runs preserve the timeline and causal links.
- Slider, treegrid, and comparison controls work by keyboard and remain legible
  on mobile and desktop viewports.
