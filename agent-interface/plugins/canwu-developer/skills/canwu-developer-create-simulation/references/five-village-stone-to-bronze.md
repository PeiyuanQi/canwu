# Five-Village Stone-to-Bronze Reference

Use this reference when a user wants a small world whose villages can develop
from stone-tool production toward bronze working and may reach different
outcomes under different seeds. Adapt names and numbers to the target product;
preserve the causal decomposition and visibility rules.

## Scenario shape

Use five stable village IDs with complementary starting conditions rather than
five interchangeable settlements:

| Village | Initial advantage | Initial constraint |
| --- | --- | --- |
| Flint Ford | food surplus and mature stone tools | weak high-temperature craft |
| Copper Hill | accessible copper ore | little tin and poor transport |
| Tin Marsh | small tin-bearing deposits | flood risk and low food surplus |
| River Market | exchange routes and multilingual traders | few raw materials |
| Hearth Vale | charcoal and advanced kilns | isolated knowledge network |

Represent households as a few decision-relevant leaders plus aggregate cohorts
such as farmers, miners, potters, traders, and craft specialists. Do not create
every resident or a dense relation between every cohort and village.

## Causal transition

Start all villages with stone tools. Let bronze production emerge only when a
site has a causally supported chain:

```text
copper and tin knowledge
-> extraction and transport
-> fuel, furnace, and temperature capability
-> alloy experiment evidence
-> installed production practice
-> artifact output
-> observed use benefit
-> use-specific adoption
-> social or institutional response
```

Use Canwu technology, information, society, routing, or host-owned resource
records where they fit. Keep the economy and resource model downstream if no
generic published extension owns it. Information received by one village does
not become global knowledge; material delivery does not imply craft capability;
one successful artifact does not imply broad adoption.

Derive presentation milestones from evidence. For example, label a village
"bronze producing" only after a qualified local practice, an installed
implementation, and a completed artifact batch. Label a wider Bronze Age only
from an authored threshold across production, sustained adoption, and exchange.
The labels grant no engine capability.

## Reproducible experiment

Expose the root seed as a command-line, configuration, or UI input. Keep the
scenario and content hashes fixed while comparing a small declared seed set,
for example `7`, `42`, and `99`. Record each run's seed, manifest hash, final
checkpoint hash, milestone times, and termination reason.

Plausible differences include which village produces bronze first, whether a
route failure delays alloying, whether a craft cohort adopts the practice, and
whether the transition remains local. Conservation, authority, chronology, and
actor-knowledge isolation must not vary by seed.

Rerunning seed `42` with the same admitted inputs must reproduce seed `42`.
Choosing seed `99` creates a different run; do not describe it as replaying or
rerolling seed `42`.

## Perspective output

Capture six explicitly labeled perspectives at committed simulation times:
the five village views plus one trusted research view. Each village sees only
its local observations and delivered information, including uncertainty,
source, observation time, and age where available. The trusted research view
may contain authoritative state but must never be substituted into a village
view.

Use the run explorer tree:

```text
Experiment
+- seed 7
|  +- Flint Ford
|  |  +- population
|  |  +- technology
|  |  +- information
|  +- Copper Hill
|  +- ...
|  +- trusted research view
+- seed 42
+- seed 99
```

At the selected simulation time, rows should show a stable subject ID, visible
state or event, value or summary, evidence/source, confidence, and last-updated
time. Compare milestone timing and outcomes across seeds without flattening
contradictory actor perspectives into one apparent truth.

## Acceptance evidence

- One command starts a run with an explicit seed.
- The world advances without render-frame-dependent time.
- The same-seed run reproduces its checkpoint and captured perspectives.
- Different seed runs remain separately addressable and comparable.
- The timeline covers all five village views and the trusted research view.
- Expanding the tree reveals technology, information, population, resources,
  decisions, and causal events without leaking hidden state.
- The host can save, restore, fork, and replay with exact plugin rehydration.
