---
name: canwu-history-create
description: "Create or extend a downstream historical research simulation on Canwu. Use when turning a historical question, period, or comparative interpretation into a runnable simulation; representing source provenance, uncertainty, and competing hypotheses; modeling technology, society, information, movement, governance, or resources; or running deterministic seed-based sensitivity analysis without changing the Canwu engine repository itself."
---

# Create a Canwu Historical Research Simulation

Build the user's historical research simulation as a downstream consumer of
Canwu. This is not a contributor workflow for changing Canwu itself. Use
`canwu-api`; do not depend directly on `canwu-sim` or expose mutable live state.
For a playable game rather than a research-oriented simulation, use
`$canwu-game-create`.

## Establish the research contract

Start by making the historical question explicit:

- period, geography, actors, institutions, and simulation horizon;
- the claim, comparison, or counterfactual being investigated;
- source-backed initial conditions, authored assumptions, and unresolved
  uncertainties;
- the time step, spatial scale, population aggregation, and decision model;
- which outputs are actor-relative observations and which are trusted research
  projections;
- root seed, content/source versions, scenario hash, and experiment identity.

Keep four kinds of statements separate:

| Statement | Treatment |
| --- | --- |
| Historical evidence or source claim | cite provenance, date, scope, and confidence |
| Simulation assumption or interpretation | label it as an authored hypothesis |
| Authoritative simulated state | store in downstream records using public Canwu contracts |
| Derived finding or presentation label | show its inputs, uncertainty, and seed set |

Do not silently convert an uncertain source into ground truth. Preserve
contradictory sources or interpretations as separate records when they would
change the research question. Keep period-specific people, institutions,
resources, names, and source data outside Canwu core.

## Build a historically legible vertical slice

1. Give places, actors, institutions, source records, hypotheses, and mechanics
   stable namespaced IDs and schema versions.
2. Bind selected content identities, provenance metadata, source hashes, plugin
   descriptors, scenario data, and the root seed before the run begins.
3. Compose only the required public extensions and downstream plugins. Declare
   every read, write, random stream, and visibility rule.
4. Admit changes through validated commands or canonical ingress. Advance
   simulation time explicitly and independently of rendering or wall-clock
   time.
5. Emit causal events and evidence links so a researcher can explain why a
   result occurred, not just inspect the final state.
6. Save, restore, fork, and exact-replay with the exact plugin environment.

Avoid presentism and single-variable historical determinism. Model the causal
conditions that matter to the question, and state what is omitted. For a
technology transition, separate evidence, local knowledge, material access,
production capability, installed implementation, use-specific adoption,
transmission, and institutional response. A period label such as "Bronze Age"
is a derived research classification, not an engine unlock or proof that every
site shared the same capability.

## Treat seeds as sensitivity analysis

- The same scenario, source/content versions, semantic environment, root seed,
  and admitted inputs must reproduce the same draws and outcome.
- Reloading before an admitted uncertain event must not reroll it.
- A different seed is a separate experiment, not exact replay and not evidence
  that the historical world had a known probability distribution.
- Declare the seed set and compare timing, causal pathways, and invariant
  outcomes. Do not cherry-pick one seed as the historical answer.
- Use mechanic-owned random streams or operation keys so unrelated work does
  not shift the result.

## Build the research views

When the user asks for a timeline slider, folded perspective tables, or
cross-seed comparison, also follow
[`canwu-common-build-run-explorer`](../canwu-common-build-run-explorer/SKILL.md).
For the five-village Stone-to-Bronze example, read
[`five-village-stone-to-bronze.md`](references/five-village-stone-to-bronze.md).

The explorer should keep these views distinct:

- actor or institution perspectives, created from actor-relative observations;
- a trusted research view, explicitly labeled and limited to authorized use;
- source/evidence tables showing provenance, confidence, observation time, and
  information age;
- cross-seed comparisons showing alternative timings and pathways without
  flattening contradictory perspectives into one truth.

## Deliver a defensible result

Provide the run command, explicit seed set, scenario and source locations,
assumption ledger, save/replay behavior, and the path to the report or host UI.
Verify at least:

- a same-seed rerun reproduces its checkpoint and captured research trace;
- different seeds remain separately addressable and comparable;
- source claims and authored assumptions are visible in the output metadata;
- actor projections cannot recover hidden ground truth;
- chronology, authority, and domain invariants hold across selected seeds;
- the downstream package has no direct `canwu-sim` dependency.

Add committed tests only for durable, reusable contracts. Use focused run
scripts or temporary checks for scenario-specific historical output review.
