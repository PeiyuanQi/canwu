# Five-Village Stone-to-Bronze Reference

Use this reference when a researcher wants a small historical simulation whose
villages can develop from stone-tool production toward bronze working and may
reach different outcomes under different seeds. Adapt names and numbers to the
research question; preserve the causal decomposition, provenance, and
visibility rules.

## Research framing

State whether the scenario studies technology diffusion, exchange networks,
local specialization, institutional response, or another question. Treat the
village names and starting conditions below as authored scenario hypotheses, not
claims about a specific archaeological site unless supported by cited sources.

Use five stable village IDs with complementary starting conditions:

| Village | Initial advantage | Initial constraint |
| --- | --- | --- |
| Flint Ford | food surplus and mature stone tools | weak high-temperature craft |
| Copper Hill | accessible copper ore | little tin and poor transport |
| Tin Marsh | small tin-bearing deposits | flood risk and low food surplus |
| River Market | exchange routes and multilingual traders | few raw materials |
| Hearth Vale | charcoal and advanced kilns | isolated knowledge network |

Represent households as a few decision-relevant leaders plus aggregate cohorts
such as farmers, miners, potters, traders, and craft specialists. Do not create
every resident or a dense relation between every cohort and village without a
research reason and a scale budget.

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
records where they fit. Keep economy and resource assumptions downstream if no
generic published extension owns them. Information received by one village does
not become global knowledge; material delivery does not imply craft capability;
one successful artifact does not imply broad adoption.

Record which links are source-supported, which are modeled assumptions, and
which are exploratory hypotheses. Do not call a village "bronze producing"
until the report explains the qualification, installed practice, and completed
artifact evidence used for that classification. A wider "Bronze Age" label is
a derived classification over explicit thresholds, not proof of a uniform
historical transition.

## Reproducible experiment

Expose the root seed as a command-line, configuration, or UI input. Keep the
scenario, source, and content hashes fixed while comparing a declared seed set,
for example `7`, `42`, and `99`. Record each run's seed, manifest hash, source
set, assumption-set version, final checkpoint hash, milestone times, and
termination reason.

Plausible differences include which village produces bronze first, whether a
route failure delays alloying, whether a craft cohort adopts the practice, and
whether the transition remains local. Conservation, authority, chronology, and
actor-knowledge isolation must not vary by seed.

Rerunning seed `42` with the same admitted inputs must reproduce seed `42`.
Choosing seed `99` creates a different experiment; do not describe it as
replaying or rerolling seed `42`, and do not treat the seed set as a calibrated
historical probability distribution without separate evidence.

## Perspective and evidence output

Capture six explicitly labeled perspectives at committed simulation times: the
five village views plus one trusted research view. Each village sees only its
local observations and delivered information, including uncertainty, source,
observation time, and age where available. The trusted research view may
contain authoritative state but must never be substituted into a village view.

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
time. Compare milestone timing and causal pathways across seeds without
flattening contradictory actor perspectives into one apparent truth.

## Acceptance evidence

- One command starts a run with an explicit seed.
- The world advances without render-frame-dependent time.
- The same-seed run reproduces its checkpoint and captured perspectives.
- Different seed runs remain separately addressable and comparable.
- The timeline covers all five village views and the trusted research view.
- Source provenance and authored assumptions are visible in the research output.
- Expanding the tree reveals technology, information, population, resources,
  decisions, and causal events without leaking hidden state.
- The host can save, restore, fork, and replay with exact plugin rehydration.
