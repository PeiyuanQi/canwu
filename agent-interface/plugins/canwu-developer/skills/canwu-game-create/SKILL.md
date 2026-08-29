---
name: canwu-game-create
description: "Create or extend a downstream game on Canwu. Use when turning a game premise into a runnable vertical slice; deciding what belongs in game content, a domain extension, an integration, or the host application; composing public Canwu APIs; modeling technology, society, information, movement, governance, or resources; or adding deterministic seed-based gameplay without changing the Canwu engine repository itself."
---

# Create a Canwu Game

Build the user's game as a downstream consumer of Canwu. This is not a
contributor workflow for changing Canwu itself. Use `canwu-api`; do not depend
directly on `canwu-sim` or expose mutable live state. For historical research
simulations, use `$canwu-history-create` instead.
When public API mechanics are unfamiliar and `$canwu-engine-usage` is
available, follow that skill as supporting API guidance.

## Establish the simulation contract

Before coding, state the smallest useful contract:

- game premise, player decisions, simulation horizon, and time step;
- spatial units, populations or decision-makers, and required domains;
- authoritative state, actor-relative knowledge, and player-facing outputs;
- deterministic root seed, authored content versions, and run identity;
- expected scale, performance budget, save policy, and presentation outputs.

Separate ownership explicitly:

| Surface | Owner |
| --- | --- |
| Generic deterministic scheduling, commands, evidence, persistence, and replay | Canwu public API |
| Reusable domain rules and records | downstream domain extension built on `canwu-api` |
| Game entities, names, starting conditions, progression, and balance data | scenario or content pack |
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

For technology or culture progression, avoid one global unlock flag when the
game needs local variation. Separate knowledge, material access, capability,
installed implementation, use-specific adoption, transmission, and
institutional effects. A player-facing era or tier should be a derived
presentation milestone over those facts, not an authoritative switch that
grants capabilities everywhere.

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
[`canwu-common-build-run-explorer`](../canwu-common-build-run-explorer/SKILL.md).

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
