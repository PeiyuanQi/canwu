# Reusable Simulation Engine Conformance Profile

Status: normative Canwu development target

This profile defines the reusable capabilities Canwu provides for authoritative
historical simulation. It deliberately excludes application-specific entities,
formulas, scenarios, settlement rules, candidates, transitions, content, and
player-interface design. Those remain application or plugin code.

Passing the current movement demo does not imply conformance. Conformance is
proved only when every requirement below has executable verification and the
representative reference fixture passes without adding application rules to
Canwu core.

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
6. Experimental reference extensions may prove these contracts while remaining
   unpublished and outside the `canwu-api` dependency graph. Their domain
   models do not become kernel conformance requirements merely by existing in
   the workspace.

## Required engine contracts

### E01: Stable identity and extensible domain storage

- Typed or schema-validated stable IDs must support application-defined entity
  and record kinds.
- Domain packages must be able to create, update, retire, and reference records
  without pretending that every concept is a built-in person, government,
  territory, route, or army.
- References, lifecycle state, and deletions must be validated before commit.
- Deterministic state uses ordered collections and canonical serialization.

### E02: Authority-aware command ingress

- Every command handler receives the authenticated issuer, simulation time,
  run policy, command identity, and relevant actor/seat authority context.
- Validation completes before authoritative mutation.
- Idempotency keys and expected-revision/time guards prevent duplicate or stale
  requests from being accepted twice.
- Human, AI, institutional, replay, system, debug, and experiment origins remain
  distinguishable in command and causal records.

### E03: Deterministic event and communication scheduling

- Work is ordered by simulation timestamp, canonical event class/priority, and
  explicit sequence or another documented stable tie-break key.
- The engine supports event-driven, sub-daily, daily, monthly, seasonal, annual,
  and era-defined cadences without depending on render frames.
- Due command, communication, acknowledgement, information, and scheduled-system
  events share one canonical ingress path.
- Late inputs never rewrite an already committed boundary.

### E04: Canonical fourteen-phase settlement boundary

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

### E05: Immutable reads, ownership, and visibility

- Proposal-producing systems read one immutable boundary snapshot plus only the
  explicitly admitted same-boundary inputs for their phase.
- Every authoritative field, queue, lifecycle, and transition family has one
  declared committing owner.
- System registration declares phase, cadence, reads, writes, reservations,
  emitted records, and same-boundary versus next-boundary visibility.
- Duplicate writers and unresolved registration-order dependencies reject the
  configuration instead of choosing a winner by insertion order.

### E06: Reservation, allocation, and conservation

- Competing claims are collected before dependent proposals execute.
- Allocation uses stable priorities and tie-break keys and records partial,
  rejected, released, carried, lost, and fulfilled dispositions explicitly.
- Conserved transfers and cross-domain bundles validate as a whole.
- Rejected work cannot leak a reservation or consume another proposal's stock.

### E07: Atomic commit and failure recovery

- Ordinary domain changes are staged and validated before one atomic commit.
- Conditional transitions use separately staged atomic bundles after ordinary
  commit and historical-candidate evaluation.
- A boundary-fatal failure leaves clock, queues, accumulators, state, logs,
  random streams, hashes, reports, publications, and delivered packets exactly
  at the previous completed boundary.
- Expected domain rejection remains structured evidence rather than an engine
  exception or partial mutation.

### E08: Scoped deterministic randomness

- Random streams have stable identities, versions, seeds, and positions.
- Draws are deterministic and recorded with enough evidence to replay material
  decisions.
- Rejected proposals and unrelated domains cannot perturb another stream.
- Parallel execution, when used, merges identically to canonical logical order.

### E09: Causality, reports, and explanation evidence

- Commands, events, effects, proposals, rejections, commits, transitions,
  aggregates, knowledge updates, and report facts carry stable causal links.
- Intentional actions preserve accountable actors; actorless processes use a
  typed no-responsible-actor cause rather than inventing a person.
- Field-level provenance and structured facts support domain-specific causal
  explanations without giving narration authority over simulation.

### E10: Actor-scoped knowledge and immutable presentation

- World truth, actor knowledge, inferred belief, recorded/archival state, and
  presentation state remain distinct.
- Actor APIs expose only delivered or otherwise permitted information,
  including confidence, source, observed time, learned time, contradiction, and
  staleness where the domain supplies them.
- Missing knowledge never falls back to world truth.
- A complete actor projection is built and validated before authoritative save
  publication, then installed through an infallible generation-tagged swap.

### E11: Controller, seat, and run-policy separation

- Run purpose, controller policy, seat policy, observation policy, interaction
  policy, and trace policy are orthogonal, versioned inputs.
- Game, unattended, replay, observer, validation, research, and diagnostic runs
  use one scheduler and system order.
- Read-only observers cannot issue authoritative commands, and changing only
  observation or presentation cannot change authoritative hashes or RNG state.

### E12: Persistence, replay, migration, and lineage

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

### E13: Package, plugin, and rules governance

- Registration is atomic, deterministic, namespaced, versioned, and validated.
- Executable handlers are paired with serializable manifests and semantic
  hashes; missing or changed required packages block load/replay unless an
  explicit migration applies.
- Engine APIs support constrained data/rule packages and deterministic overlay
  resolution without default arbitrary script injection.
- A package cannot write another owner's state except through a declared,
  validated cross-domain proposal/transition contract.

### E14: Binding-safe public interfaces

- In-process Rust is first-class. Public request/response types are owned,
  serializable, versioned, batch-friendly, and suitable for future C, Python,
  TypeScript, or network adapters.
- Programmatic, actor-semantic, research, and debug capabilities have separate
  authorization surfaces.
- Queries have deterministic ordering, bounded result sizes, explicit schemas,
  and no hidden authority escalation.

### E15: Solver and performance extension boundary

- A deterministic CPU reference implementation exists before an optimized
  solver backend is accepted.
- Solver inputs/outputs are versioned and independently testable.
- GPU/native acceleration is optional, replaceable, and cannot own canonical
  gameplay state or change results outside a declared tolerance contract.

### E16: Portability and operability

- Headless engine crates support Windows, macOS, and Linux.
- Runtime behavior does not depend on path separators, shell conventions,
  filesystem case, line endings, wall-clock timing, or platform-only APIs.
- Diagnostics distinguish authoritative evidence from operational logs.

## Conformance evidence

Each requirement must have all of the following before it is marked complete:

1. a public contract in architecture/API documentation;
2. implementation reachable through supported APIs;
3. focused invariant tests;
4. a representative integration fixture using only public extension points;
5. save/load and replay coverage where state is involved;
6. cross-platform CI coverage where platform behavior can differ.

The final gate is a requirement-by-requirement audit. A planned API, a generic
JSON escape hatch, or a narrow green demo test is not conformance evidence.

## Current baseline

The v0.4 engine provides a public, deterministic fourteen-phase settlement API.
Boundary systems declare cadence, reads, writes, reservation participation,
allocation reads, owned random streams, emissions, and visibility. The kernel
provides stable allocation order, separately staged ordinary and conditional
commits, same-boundary overlays, next-boundary visibility, full rollback on
fatal error, scoped deterministic draws, and persisted boundary evidence with
exact plugin/system provenance. Snapshot format 4 adds random draw journals,
state and boundary hash chains, a current-state checkpoint commitment, hashed
run/content/source manifests, exact plugin version and semantic identities, and
an environment-bound replay journal that also verifies command-only and
registration-closure-only runs. Declared runs now also bind six orthogonal
run-policy dimensions. Their supported request path distinguishes human, AI,
institutional, replay, system, debug, experiment, and compatibility actor
origins; validates explicit decision/seat authority before mutation; enforces
idempotency and mandatory expected revision/time pairs for declared external
commands; advances revision on accepted commands and published settlement
boundaries; persists accepted and expected-rejected attempts; admits them into
boundary evidence; and replays them exactly. Frozen
replay ingress is kernel-only, so live callers cannot self-label around
`ReadOnly`. Observation/trace-only variants retain distinct save identities but
produce identical authoritative state/boundary hashes and RNG state. Every
report dispatch must retain exactly one
causally linked core random draw, and authoritative scheduling rejects time
overflow rather than saturating. Current-format checkpoints also require an
exact engine-version match unless an explicit migration rewrites their
commitments.
E01 now has a public generic storage contract: plugins register namespaced
entity or record kinds with payload and typed-reference schemas; boundary
systems atomically create, expected-version update, retire, successor-link, and
delete versioned records; deleted identities remain tombstones; live references
and external dependencies block unsafe deletion; and current/proposed records
are available through declared immutable reads. Boundary evidence records every
lifecycle transition and its causal event. A representative office/obligation fixture
proves atomic reference transfer, rejected referenced deletion with full
rollback, cycle-free succession, canonical save/load, exact replay,
provenance-tamper rejection, historical-cut rejection for pre-creation and
post-deletion evidence, rejection of cross-system same-stage creation use, and
manifest-bound protection against shifting created records into genesis state.
Compatible handler-free format 2 and 3 saves migrate with explicit legacy
provenance for continuation and explicitly reject unsupported exact replay.
These complete the current E01 contract and are substantial but still partial
implementations of E02, E04 through E09, and E11 through E13.

E03 now has one persisted host-ingress queue for typed commands,
plugin-declared communication, acknowledgement, and information packets, and
explicit calendar work. Its stable order is due time, class, descending
priority, issue time, then ingress ID. Canonical advancement selects the earlier of internal scheduled work
and queued ingress; completed boundaries record both admitted packets and any
follow-up packets generated by boundary systems. A zero-delay generated packet
is eligible only for a second boundary at the same simulation time. Live late
input and live plugin ingress in declared read-only runs fail without mutation;
save/load validates issue cuts, entity identity, payload schemas, generation
provenance, and queue order; exact replay must reproduce plugin-generated
packets from the recorded system environment. The representative ingress fixture
mixes command, communication, acknowledgement, information, and daily calendar
work, proves priority/class ordering, command precedence over equal-time
internal scheduled continuations, and late-input rollback, and continues an
automatic acknowledgement through save/load and replay. This implements the
canonical external-ingress portion of E03. Full E03 conformance remains
open until recurring calendar policy and all internal scheduled continuation
sources are represented by one versioned persistence abstraction.

The public-only
[`representative_conformance`](../crates/canwu-api/tests/representative_conformance.rs)
fixture now composes independent domain packages, authority-aware commands and
persisted rejection evidence, competing reservations, same- and next-boundary
visibility, a conditional record transition, actor-relative knowledge, scoped
randomness, save/load, exact replay, forking, rollback, tamper detection, and
package-identity rejection. It proves those implemented architecture boundaries
without adding application-specific types or rules to the kernel.

The unpublished `canwu-society` reference extension additionally composes E01,
E02, E04, E05, E07, E09, E10, E12, E13, and E15 through public `canwu-api`
contracts. Its neutral local-community fixture proves conserved sparse cohort
distributions, deterministic integer remainders, a DecisionTicket-backed
institutional policy command, public/private divergence, authorized estimates
with no truth fallback, exact payload-to-core-reference validation, exact
derived-state and pending-policy validation after plugin rehydration, snapshot
restore, fork, and exact replay. Its authority test rejects a forged explicit
authority envelope without engine-issued DecisionTicket provenance. Its active
signal and observer-projection indexes are also tested to grow with sparse
active inputs rather than rule-by-edge or observer-by-distribution cross
products, including valid EPOCH and negative-time boundaries. This is extension
evidence, not a claim that social or religious types belong in Canwu core.

The complete profile remains in progress. The major remaining gaps include
authority scopes that prevent human/AI double control, institution/advisor
permission semantics, experiment lineage, and the remaining canonical
run-configuration identity fields; recurring calendar policy and unified
persistence for internal scheduled continuations; released, carried, and lost
reservation outcomes plus atomic conservation bundles; field-level provenance
and structured report facts; immutable generation-tagged actor projections; a
general migration registry, replay environment discovery, fork lineage, and
branch comparison; constrained data/rule packages; binding-oriented batch APIs;
and a versioned deterministic solver boundary. Final conformance requires those
contracts, their executable evidence, and a requirement-by-requirement audit.
