---
name: canwu-developer-create-simulation
description: "Create or extend a downstream game, world, scenario, or historical simulation that uses Canwu. Use when turning a premise into a runnable vertical slice; deciding what belongs in scenario data, a domain extension, an integration, or the host application; composing public Canwu APIs; modeling technology, society, information, movement, governance, or resources; or adding deterministic seed-based runs without changing the Canwu engine repository itself."
---

# Create a Canwu Simulation

Build the user's game or historical simulation as a downstream consumer of
Canwu. This is not a contributor workflow for changing Canwu itself. Use
`canwu-api`; do not depend directly on `canwu-sim` or expose mutable live state.
When public API mechanics are unfamiliar and `$canwu-engine-usage` is
available, follow that skill as supporting API guidance.

## Establish the simulation contract

Before coding, state the smallest useful contract:

- premise, player or researcher decisions, simulation horizon, and time step;
- spatial units, populations or decision-makers, and required domains;
- authoritative state, actor-relative knowledge, and trusted research outputs;
- deterministic root seed, authored content versions, and run identity;
- expected scale, performance budget, save policy, and presentation outputs.

Separate ownership explicitly:

| Surface | Owner |
| --- | --- |
| Generic deterministic scheduling, commands, evidence, persistence, and replay | Canwu public API |
| Reusable domain rules and records | downstream domain extension built on `canwu-api` |
| Period-specific entities, names, starting conditions, and source provenance | scenario or content pack |
| Composition, authentication, input mapping, save policy, experiments, and UI | host application |

Keep application-specific entities and history outside Canwu core. Model only
decision-relevant individuals; represent ordinary populations as cohorts or
aggregates. Materialize active relationships and records instead of dense
person-by-place-by-domain matrices.

## Build one vertical slice

1. Give every scenario object and mechanic a stable, namespaced ID and schema
   version.
2. Bind selected content identities, hashes, plugin descriptors, scenario data,
   and the root seed before the run begins.
3. Compose only the required public extensions and host-owned plugins. Declare
   every plugin read, write, random stream, and visibility rule.
4. Admit external mutations through validated commands or canonical ingress.
   Advance simulation time explicitly, independently of render frames.
5. Emit causal events or records that the host can project into gameplay,
   research output, and the run explorer.
6. Save and restore with exact plugin rehydration. Use exact replay for the same
   causal run and `fork()` for a continuation with different inputs.

For technology transitions, never use one global era meter or an instant
"invented" flag. Separate evidence, local knowledge, material access,
production capability, installed implementation, use-specific adoption,
transmission, and institutional effects. A historical label such as "Bronze
Age" should be a derived presentation milestone over those facts, not an
authoritative switch that grants capabilities everywhere.

## Treat seeds correctly

- The same scenario, semantic environment, root seed, and admitted inputs must
  produce the same draws and outcome.
- Reloading before an admitted uncertain event must not reroll it.
- A different root seed creates a separate run for comparison. It is not exact
  replay and should not overwrite the original run's identity or evidence.
- Use stable, mechanic-owned random streams or operation keys so unrelated work
  does not shift an outcome.
- Do not force every seed to reach the authored historical result. Preserve
  causal invariants while allowing plausible differences in timing, path, and
  outcome.

## Add the explorer when requested

When the user asks for reruns, a timeline slider, folded perspective tables, or
cross-seed comparison, also follow
[`canwu-developer-build-run-explorer`](../canwu-developer-build-run-explorer/SKILL.md).
For a concrete end-to-end model, read
[`five-village-stone-to-bronze.md`](references/five-village-stone-to-bronze.md).

## Deliver a runnable result

Provide the run command, explicit seed option, scenario and content locations,
save/replay behavior, and the path to the host UI or report. Verify at least:

- a same-seed rerun produces the same checkpoint and visible trace;
- save/load and exact replay preserve plugin semantics;
- actor projections cannot recover hidden ground truth;
- domain conservation and authority invariants hold across selected seeds;
- the downstream package has no direct `canwu-sim` dependency.

Add committed tests only for durable, reusable contracts. Use focused run
scripts or temporary checks for scenario-specific output inspection.
