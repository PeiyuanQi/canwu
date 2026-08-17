# End-State Design

This document is the source of truth for architectural direction beyond the
initial movement vertical slice.

The concrete reusable-engine acceptance target is defined in
[`engine-conformance.md`](engine-conformance.md). Its requirements are part of
the Canwu end state; period- and application-specific mechanics remain external
domain packages.

## Product surfaces

Canwu should support in-process Rust, stable serialized request/response types,
and later C, Python, TypeScript, and network bindings without redesigning the
command model. Binding-friendly APIs should prefer owned serializable values,
typed IDs, explicit errors, and batch operations.

## Historical state

The runtime should distinguish four related records:

1. World state: the simulation's ground truth.
2. Knowledge state: information delivered to an actor.
3. Belief state: an actor's inference from knowledge; a later plugin concern.
4. Recorded state: chronicles, archives, or research observations that can be
   wrong or incomplete.

No semantic-agent operation may silently fall back to world state when actor
knowledge is absent.

## Causality and explanation

Events carry command, parent-event, boundary, or typed system causes and
correlation IDs. Boundary records preserve exact plugin/system emission sources
and committed component-change evidence. Future field provenance should add
compact `(entity, component, field) -> event` indexes rather than replacing the
event model. Explanation can then grow from event chains to domain-specific
causal narratives.

## Persistence and counterfactuals

Snapshots contain deterministic state, clock, RNG state, scheduler sequence,
pending serializable work, knowledge, event history, and command records. A
snapshot also retains plugin descriptors and blocks continuation until matching
stateless executable handlers are rehydrated. It can be forked into independent
simulations. Current-state checkpoints and contiguous evidence-journal segments
provide incremental persistence without changing the flat snapshot contract.
The opt-in compact runtime can seal completed live tails into caller-owned
segments while preserving continuation, exact idempotency, commitments, and
reconstruction through that same contract.
Future work may add:

- content-addressed archive adapters and indexed historical lookup
- replay from command/event journals
- branch metadata and lineage
- world and outcome comparison across branches
- historical dataset provenance

## Geography

The initial point-and-route graph should evolve through additive traits and data:
polygons, administrative containment, terrain, river and road networks, spatial
indexes, travel modes, and time-dependent costs. Geometry remains data for
clients; it never becomes a rendering subsystem.

## Systems and plugins

Population, agriculture, trade, taxation, bureaucracy, military logistics,
diplomacy, migration, disease, and climate should remain separately testable
plugins. The core may standardize broadly shared primitives only after at least
two concrete systems need them.

## Debug client

The first-party client remains a reference consumer. It should prioritize world
inspection, deterministic controls, schema-driven fields, event provenance, and
debug commands. Production map rendering and game interaction belong in adapter
projects such as `canwu-bevy`, `canwu-unity`, `canwu-godot`, and `canwu-web`.
