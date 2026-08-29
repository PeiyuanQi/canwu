# Culture Authoring SDK and Lifecycle Design

Status: first implementation slice complete as the published
`canwu-culture` crate. The authoring schema, deterministic compiler, society
adapter, dirty-set API, effect persistence classes, lifecycle index, tombstone,
reactivation, complete runtime-state hydration, and explicit atomic lifecycle
synchronization into `canwu-society` are implemented. Boundary-system
registration, canonical cross-extension ingress, incremental society
aggregate/projection settlement, and benchmark evidence remain follow-up work.
This document does not add a culture type to the simulation core.

## Decision

Add an optional `canwu-culture` domain extension above `canwu-society`.
Developers provide a versioned culture definition and a reference content pack.
The extension validates and compiles that definition into a deterministic
execution plan. `canwu-society` remains the generic runtime for population
dispositions, social influence, organization topology, institutional inputs,
and actor-relative projections.

The first implementation must not model one actor per person, add arbitrary
unbounded dimensions to the core profile, or let a culture package write legal,
economic, technological, or military state directly. Other extensions consume
bounded, versioned cultural signals through the existing canonical ingress and
boundary contracts.

This preserves the current ownership rule: historical ideas, people, periods,
institutions, policy meanings, and narrative remain reference content or
downstream application data.

## Goals and non-goals

Goals:

- make a culture system declarative and approachable for an ordinary extension
  developer;
- compile string-heavy content into compact, deterministic runtime indexes;
- evaluate only active and changed population/target relationships;
- retire extinct targets from the hot path without deleting their history;
- expose bounded cross-extension inputs and outputs with causal evidence;
- keep save/load, exact replay, fork, rollback, and actor-relative reads intact.

Non-goals:

- a universal definition of culture, ideology, religion, ethnicity, or values;
- automatic historical interpretation or a library of named historical cases;
- per-person cultural state as the default representation;
- a synchronous event bus between domain extensions;
- automatic conversion of cultural alignment into conflict or war.

## Layering

```text
reference content pack
  CultureDefinition, localized labels, provenance, balance profiles
          |
          v
canwu-culture authoring and compiler
  schema validation, cardinality budgets, compiled execution plan
          |
          v
canwu-society runtime
  sparse distributions, transitions, influence, institutions, projections
          |
          +--> canwu-information / canwu-correspondence inputs
          +--> downstream law, education, economy, technology, and conflict adapters
```

`canwu-culture` may depend on `canwu-society` and `canwu-api`; the simulation
core and `canwu-api` must not depend on it. The compiler is a build-time or
scenario-load operation. Settlement consumes only the validated materialized
plan and does not read an unverified external file.

## Authoring contract

The public authoring surface should use owned, serializable values and return a
validated `CultureDefinition` or a structured error. A Rust builder and a JSON/
TOML loader may share the same validator.

```rust
let definition = CultureDefinition::builder("rights-tradition")
    .target("universal_dignity")
    .target("equality_before_law")
    .cohort(CultureCohortDefinition::new(
        "urban_merchants",
        TerritoryId::new(1),
        12_000,
    ))
    .channel(ChannelSpec::new(
        "printing",
        "urban_merchants",
        "universal_dignity",
        700,
        500,
    ))
    .transition(TransitionSpec::awareness_from_influence(
        "dignity-awareness",
        "universal_dignity",
        100_000,
    ))
    .effect(CulturalEffectBinding::new(
        "legal-tolerance",
        "universal_dignity",
        "rights-signal",
        EffectPersistence::Commitment,
    ))
    .retirement(RetirementPolicy::after_quiet_boundaries(30))
    .build()?;
```

The example uses the current public Rust API. JSON/TOML loaders should
target the same serializable fields and validation rules.

### Definition components

- **Target definitions** identify an idea, norm, movement, practice, school, or
  affiliation variant. They carry neutral profile defaults, ancestry, metadata,
  provenance, and an explicit lifecycle policy.
- **Cohort definitions** identify aggregate populations with a territory,
  headcount, and application-defined classifications such as language,
  occupation, education, or status.
- **Channel definitions** describe opportunities for exposure or reinforcement:
  reach, trust, interpretation fidelity, delay, capacity, and policy modifiers.
- **Transition specifications** map named signals to the existing separate
  awareness, private assent, practice, public alignment, organization tie,
  mobilization, and visibility dimensions. They are compiled into stable rules;
  they do not create a second solver.
- **Institution and policy bindings** describe which external decisions can
  modify access, support, enforcement, censorship, disruption, or migration
  pressure. They never assign a private-assent percentage directly.
- **Effect bindings** declare the downstream signal kind, scope, cadence, and
  evidence required by another extension. Signal batch size, evidence count,
  and fan-out are bounded by the compiled plan budget. The culture runtime
  emits a batch; the consumer decides its domain meaning.

The authoring layer may expose named cultural traits and affinities, but the
first runtime must compile them into bounded rule factors or channel signals.
It must not attach an unbounded map of values to every population bucket.

## Compiled execution plan

`CompiledCulturePlan` is compile-only and externally immutable for one
scenario/run revision. It contains:

- interned numeric IDs for targets, cohorts, channels, institutions, and rules;
- canonical sorted rule tables and reverse indexes by target;
- compact channel, transition, effect, and institution tables;
- scoped institution and transition keys for affected cohorts;
- lifecycle indexes that do not expose a mutable compiled plan;
- declared cardinality, fan-out, compiled-plan bytes, persisted-state bytes,
  and per-boundary work budgets;
- a content hash used to bind persisted runtime state to the exact definition.

String identifiers remain at the public boundary. The runtime plan must not
clone and compare long strings in every transition. Changing a definition or
its compiled ordering creates a new semantic plan revision and is not an
in-place mutation of an existing run.

## Incremental settlement and scale

The hot path should be driven by a dirty set of active `(cohort, target)` pairs.
An admitted exposure, policy change, organization change, or reactivation marks
the affected pairs. A transition boundary then:

1. consumes admitted signals in canonical order;
2. evaluates only dirty pairs and their bounded dependants;
3. updates aggregate counters incrementally;
4. refreshes projections only for observers that can see changed pairs;
5. emits bounded effect batches for the next eligible consumer boundary.

For `D` active pairs, `Delta` dirty pairs, `B` buckets per pair, `E_delta`
affected edges, and `V_delta` affected observer entries, the intended steady
state cost is approximately:

```text
O(Delta * B + E_delta + V_delta)
```

with a full plan rebuild only after definition changes, migration, or an
explicit maintenance checkpoint. The current full-state implementation should
remain the compatibility fallback while this plan is introduced, but new APIs
must not make full scans look free.

Every definition is validated against budgets before a run starts. Separate
`max_memory_bytes` and `max_state_bytes` limits bound the compiled plan and the
persisted runtime state. The runtime caches the validated state-byte estimate
and maintains it incrementally, so ordinary dirty marking and effect emission
do not rescan retained tombstones. A rejected definition is preferable to a
valid-looking scenario that causes unbounded fan-out or retained evidence
during play.

## Culture lifecycle and retirement

Culture targets have three runtime states:

```text
Active  -> Dormant  -> Retired
              ^          |
              +----------+
             explicit reactivation creates a new generation
```

### Active

The target has engaged population, active propagation, institutional/policy
inputs, or a currently scheduled reactivation path. Its distributions, rules,
and projections are eligible for ordinary settlement.

### Dormant

The target has no engaged population and no admitted work for a configured
number of boundaries. It leaves the culture runtime's hot and dirty indexes;
an explicit `synchronize_society_lifecycle` call also removes its compiled
culture transition rules. Existing society distributions and other target-scoped state
remain until retirement. Its definition and a compact reactivation descriptor
remain available. Dormant is reversible without claiming that the culture has
been historically erased.

### Retired

The target has remained dormant through its retention policy and has no live
authoritative dependency that requires hot state. Retirement writes a compact
`RetiredTargetTombstone` containing:

- target identity and generation;
- last active simulation time and revision;
- retirement reason and policy hash;
- optional successor/reference target, never an implicit merge;
- exact evidence references needed for replay and audit.

The tombstone is not a replacement for historical evidence. Old domain-record
versions, events, knowledge records, and archived segments remain queryable
through their normal contracts. Only hot derived state is removed.

### Eligibility

Retirement is evaluated after all signals admitted for the boundary have been
applied. A target is eligible only when all of the following hold:

- engaged headcount is zero for the configured quiet-boundary window;
- no active transition rule, organization, institution, policy, or effect batch
  still references it as live;
- no admitted cross-extension input targets its current generation;
- no pending scheduled continuation requires its dynamic state;
- any reactivation path is explicit and indexed outside the daily hot set.

Engaged headcount is not the distribution total. In the current society model,
an absent relationship can be materialized with the entire cohort in a neutral
bucket. A neutral-only distribution therefore does not keep a target alive.
The compiler must define the neutral profile and calculate engagement from
non-neutral disposition buckets or an equivalent explicit participation rule.

### Retirement transaction

`settle_culture_society_boundary` is the preferred host helper. It first
prepares a bounded runtime delta over active targets, due dormant targets, and
explicit observations. An ordinary boundary applies that delta without cloning
the runtime or touching the retired catalog. When a lifecycle transition is
present, the helper stages `SocietyState` and applies only the transitioned
target bindings: Dormant removes its hot transition rules, Retired releases
its rebuildable target state, and Reactivated restores only its compiled
bindings. A live external society dependency rejects retirement before either
caller-owned state is changed. The explicit `synchronize_society_lifecycle`
API remains a full reconciliation path for load repair and maintenance
checkpoints, not the ordinary boundary path.

Retirement cleanup never deletes another extension's live organization,
policy, influence edge, rule, or binding. The host remains responsible for
persisting the new culture record and returned typed transition in its
authoritative boundary transaction. Lower-level lifecycle and society calls
remain available for hosts that stage the same atomic composition themselves.
Culture-owned society rules and alignments are identified by the exact
length-prefixed IDs compiled for the plan; string-prefix inference is not an
ownership boundary.

New exposure for a retired generation is rejected with a stable lifecycle error
unless an explicit reactivation command or ingress is admitted. Reactivation
creates a new generation, initializes only the required active relationships,
and cites the prior tombstone. It does not mutate old history or silently
resurrect every former cohort.

Tombstones may later move to the existing compact archive path once no live
snapshot continuation requires them. Archive compaction is a verified,
receipt-backed operation; ordinary settlement never garbage-collects evidence.

## Cross-extension signals

Information and correspondence extensions may produce a versioned
`CultureExposureSignalBatch` after access and interpretation have been resolved.
The signal should identify the target generation, affected scope, fidelity,
source evidence, and earliest eligible boundary. A batch is bounded and enters
through canonical next-boundary ingress.

The culture extension may emit `CulturalSignalBatch` records such as public
alignment pressure, education demand, legitimacy pressure, or organization
capacity. These are generic, evidence-bearing outputs. Law, education,
technology, economy, and conflict extensions own the interpretation and their
own authoritative writes.

No synchronous cross-extension callback is permitted. A consumer that cannot
accept a batch records a rejection or defers it; it does not partially mutate
the culture state.

## Persistence, replay, and observability

- The compiled plan hash, budgets, lifecycle policy, and target generations are
  part of the plugin semantic environment.
- The persisted state includes the boundary and latest-activity cursors,
  dormant schedule, hot targets, dirty pairs, effect cadence cursors,
  generations, and tombstones; `CultureRuntime::from_state` validates and
  restores all of them, then rebuilds the derived state-byte index once.
- Retirement and reactivation become authoritative domain changes when the
  host records the returned transition and updated state in its boundary.
- Actor projections distinguish `active`, `dormant`, `retired`, and `unknown`;
  a retired target is not silently returned as absent.
- Counters expose active-pair count, dirty-pair count, bucket count, signal
  fan-out, retirement count, and projection refresh count per boundary.
- A failed boundary restores active indexes, tombstones, counters, evidence,
  and random positions atomically.

## Conformance and benchmark gates

Before calling the SDK scalable, add a dedicated society/culture workload with
independent growth of cohorts, targets, active pairs, rules, organizations,
signals, observers, and retired targets. Measure boundary time, allocations,
snapshot bytes, load/validation, replay throughput, and peak resident memory.

Required metamorphic cases include:

- adding thousands of retired targets does not change active settlement cost;
- converting an active target to dormant removes it from hot indexes;
- retirement followed by reactivation replays exactly and preserves old evidence;
- an admitted signal for a retired generation cannot mutate a new generation;
- unrelated observers and retired catalog entries do not change keyed results;
- a failed retirement boundary rolls back the entire lifecycle mutation;
- the cost curve tracks dirty pairs, not the full catalog size.

The first acceptance target is linear behavior in active and dirty state. A
future sharded root may improve absolute limits, but sharding must not hide a
quadratic plan or weaken exact replay and cross-shard boundary ordering.

## Implementation order

1. Define the serializable authoring schema, validator, and budget errors.
2. Compile a definition into a deterministic plan without changing the current
   society wire format.
3. Add dirty-set indexes, then integrate incremental aggregate/projection
   refreshes behind the existing plugin boundary.
4. Add Active/Dormant/Retired lifecycle records, tombstones, reactivation,
   complete state hydration, and host-level exact replay tests.
5. Add bounded information-to-culture input and culture-to-domain output
   adapters using next-boundary ingress.
6. Add the dedicated benchmark and publish measured limits before considering
   shard-level persistence or parallel settlement.

The existing neutral local-community case remains a compatibility fixture. A
historical case such as human-rights diffusion should be added only after this
SDK and lifecycle contract has evidence; it is content, not a solver branch.

## Implementation status

The current crate exposes `CultureDefinition`, `compile_culture`,
`CompiledCulturePlan`, `DirtySet`, `CultureRuntime`, and the
`install_into_society`, the maintenance-oriented
`synchronize_society_lifecycle`, and the atomic target-delta
`settle_culture_society_boundary` adapter.
`CulturePlugin` registers the complete lifecycle runtime record, and
`load_culture_runtime` rehydrates its schedules and hot/dirty indexes. The host
still drives settlement, persists returned lifecycle transitions, synchronizes
society state, and admits emitted batches; no boundary system, synchronous
cross-extension callback, canonical ingress adapter, or direct legal write is
provided yet.
