# Celestial Mandate Engine Conformance Profile

Status: normative Canwu development target
Source baseline: Celestial Mandate SOT amended through 2026-08-13

This profile defines the reusable engine capabilities that Canwu must provide
before it can host the authoritative simulation for Celestial Mandate (CM).
It deliberately excludes CM-specific historical entities, formulas, scenarios,
settlement rules, candidates, transitions, content, and player-interface design.
Those remain application or plugin code.

Passing the current movement demo does not imply conformance. Conformance is
proved only when every requirement below has executable verification and the
CM-shaped reference fixture passes without adding CM rules to Canwu core.

## Boundary and dependency rules

1. Canwu is headless and owns no renderer, production UI, audio, animation, or
   platform storefront behavior.
2. Authoritative state is owned by one game-controlled simulation host.
   Presentation runtimes receive immutable, generation-tagged projections and
   validated request endpoints only.
3. Domain packages may define entities, components, commands, systems,
   knowledge, reports, and transitions without modifying Canwu core.
4. External mutation enters through validated commands, admitted events, or
   versioned experiment inputs. No public API exposes a mutable live state.
5. Debug and research authority is explicit and cannot be reached by silently
   falling back from an actor-scoped API.

## Required engine contracts

### CM-E01: Stable identity and extensible domain storage

- Typed or schema-validated stable IDs must support application-defined entity
  and record kinds.
- Domain packages must be able to create, update, retire, and reference records
  without pretending that every concept is a built-in person, government,
  territory, route, or army.
- References, lifecycle state, and deletions must be validated before commit.
- Deterministic state uses ordered collections and canonical serialization.

### CM-E02: Authority-aware command ingress

- Every command handler receives the authenticated issuer, simulation time,
  run policy, command identity, and relevant actor/seat authority context.
- Validation completes before authoritative mutation.
- Idempotency keys and expected-revision/time guards prevent duplicate or stale
  requests from being accepted twice.
- Human, AI, institutional, replay, system, debug, and experiment origins remain
  distinguishable in command and causal records.

### CM-E03: Deterministic event and communication scheduling

- Work is ordered by simulation timestamp, canonical event class/priority, and
  explicit sequence or another documented stable tie-break key.
- The engine supports event-driven, sub-daily, daily, monthly, seasonal, annual,
  and era-defined cadences without depending on render frames.
- Due command, communication, acknowledgement, information, and scheduled-system
  events share one canonical ingress path.
- Late inputs never rewrite an already committed boundary.

### CM-E04: Canonical fourteen-phase settlement boundary

Every authoritative boundary uses this order, even when a phase has no work:

1. Event Ingress
2. Boundary Snapshot
3. Derived Field Solve
4. Perception and Attention Refresh
5. Decision and Accepted-Effect Intake
6. Reservation and Allocation
7. Domain Delta Proposal
8. Invariant Validation
9. Atomic Domain Commit
10. Historical Candidate Evaluation
11. Conditional Transition Commit
12. Strategic Aggregation
13. Perspective and Report Materialization
14. Save, Replay, and Diagnostic Hashing

The kernel owns ordering and visibility. Domain packages own their rules.

### CM-E05: Immutable reads, ownership, and visibility

- Proposal-producing systems read one immutable boundary snapshot plus only the
  explicitly admitted same-boundary inputs for their phase.
- Every authoritative field, queue, lifecycle, and transition family has one
  declared committing owner.
- System registration declares phase, cadence, reads, writes, reservations,
  emitted records, and same-boundary versus next-boundary visibility.
- Duplicate writers and unresolved registration-order dependencies reject the
  configuration instead of choosing a winner by insertion order.

### CM-E06: Reservation, allocation, and conservation

- Competing claims are collected before dependent proposals execute.
- Allocation uses stable priorities and tie-break keys and records partial,
  rejected, released, carried, lost, and fulfilled dispositions explicitly.
- Conserved transfers and cross-domain bundles validate as a whole.
- Rejected work cannot leak a reservation or consume another proposal's stock.

### CM-E07: Atomic commit and failure recovery

- Ordinary domain changes are staged and validated before one atomic commit.
- Conditional transitions use separately staged atomic bundles after ordinary
  commit and historical-candidate evaluation.
- A boundary-fatal failure leaves clock, queues, accumulators, state, logs,
  random streams, hashes, reports, publications, and delivered packets exactly
  at the previous completed boundary.
- Expected domain rejection remains structured evidence rather than an engine
  exception or partial mutation.

### CM-E08: Scoped deterministic randomness

- Random streams have stable identities, versions, seeds, and positions.
- Draws are deterministic and recorded with enough evidence to replay material
  decisions.
- Rejected proposals and unrelated domains cannot perturb another stream.
- Parallel execution, when used, merges identically to canonical logical order.

### CM-E09: Causality, reports, and explanation evidence

- Commands, events, effects, proposals, rejections, commits, transitions,
  aggregates, knowledge updates, and report facts carry stable causal links.
- Intentional actions preserve accountable actors; actorless processes use a
  typed no-responsible-actor cause rather than inventing a person.
- Field-level provenance and structured facts support domain-specific causal
  explanations without giving narration authority over simulation.

### CM-E10: Actor-scoped knowledge and immutable presentation

- World truth, actor knowledge, inferred belief, recorded/archival state, and
  presentation state remain distinct.
- Actor APIs expose only delivered or otherwise permitted information,
  including confidence, source, observed time, learned time, contradiction, and
  staleness where the domain supplies them.
- Missing knowledge never falls back to world truth.
- A complete actor projection is built and validated before authoritative save
  publication, then installed through an infallible generation-tagged swap.

### CM-E11: Controller, seat, and run-policy separation

- Run purpose, controller policy, seat policy, observation policy, interaction
  policy, and trace policy are orthogonal, versioned inputs.
- Game, unattended, replay, observer, validation, research, and diagnostic runs
  use one scheduler and system order.
- Read-only observers cannot issue authoritative commands, and changing only
  observation or presentation cannot change authoritative hashes or RNG state.

### CM-E12: Persistence, replay, migration, and lineage

- Checkpoints preserve all deterministic state needed for exact continuation:
  queues, accumulators, commands/effects, reservations, transitions, knowledge,
  plugin state, random streams, counters, and the boundary hash chain.
- Save identity records engine, schema, rules, scenario/content, plugin/mod,
  localization-sensitive contract, run configuration, and source/provenance
  manifests as applicable.
- Compatible migrations are explicit; incompatible loads fail with stable
  machine-readable reasons.
- Replay restores the recorded plugin/rules environment before applying its
  journal.
- Forks create a new lineage with parent and boundary identity. Comparison and
  export never mutate either branch.

### CM-E13: Package, plugin, and rules governance

- Registration is atomic, deterministic, namespaced, versioned, and validated.
- Executable handlers are paired with serializable manifests and semantic
  hashes; missing or changed required packages block load/replay unless an
  explicit migration applies.
- Engine APIs support constrained data/rule packages and deterministic overlay
  resolution without default arbitrary script injection.
- A package cannot write another owner's state except through a declared,
  validated cross-domain proposal/transition contract.

### CM-E14: Binding-safe public interfaces

- In-process Rust is first-class. Public request/response types are owned,
  serializable, versioned, batch-friendly, and suitable for future C, Python,
  TypeScript, or network adapters.
- Programmatic, actor-semantic, research, and debug capabilities have separate
  authorization surfaces.
- Queries have deterministic ordering, bounded result sizes, explicit schemas,
  and no hidden authority escalation.

### CM-E15: Solver and performance extension boundary

- A deterministic CPU reference implementation exists before an optimized
  solver backend is accepted.
- Solver inputs/outputs are versioned and independently testable.
- GPU/native acceleration is optional, replaceable, and cannot own canonical
  gameplay state or change results outside a declared tolerance contract.

### CM-E16: Portability and operability

- Headless engine crates support Windows, macOS, and Linux.
- Runtime behavior does not depend on path separators, shell conventions,
  filesystem case, line endings, wall-clock timing, or platform-only APIs.
- Diagnostics distinguish authoritative evidence from operational logs.

## Conformance evidence

Each requirement must have all of the following before it is marked complete:

1. a public contract in architecture/API documentation;
2. implementation reachable through supported APIs;
3. focused invariant tests;
4. a CM-shaped integration fixture using only public extension points;
5. save/load and replay coverage where state is involved;
6. cross-platform CI coverage where platform behavior can differ.

The final gate is a requirement-by-requirement audit. A planned API, a generic
JSON escape hatch, or a narrow green demo test is not conformance evidence.

## Current baseline

The v0.3 engine adds a public, deterministic fourteen-phase settlement API.
Boundary systems declare cadence, reads, writes, reservation participation,
allocation reads, emissions, and visibility. The kernel provides stable
allocation order, separately staged ordinary and conditional commits,
same-boundary overlays, next-boundary visibility, full rollback on fatal error,
and persisted boundary evidence with exact plugin/system provenance. Snapshot
format 3 validates that evidence and explicitly migrates compatible format 2
saves. These are substantial but still partial implementations of CM-E04
through CM-E07, CM-E09, CM-E12, and CM-E13.

Canwu is not yet CM-conformant. The major remaining gaps include generic domain
entity/record lifecycles; complete authority, seat, idempotency, and run-policy
contracts; unified typed ingress and automatic calendar scheduling; released,
carried, and lost reservation outcomes plus atomic conservation bundles;
structured nonfatal rejection evidence; scoped/versioned random streams;
field-level provenance and structured report facts; immutable generation-tagged
actor projections; hashes, manifests, migrations, replay journals, forks, and
lineage; constrained data/rule packages; binding-oriented batch APIs; and a
versioned deterministic solver boundary. Final conformance still requires the
public CM-shaped integration fixture and the requirement-by-requirement audit.
