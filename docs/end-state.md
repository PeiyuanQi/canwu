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

## Decisions and controllers

Decision tickets are a first-party engine primitive rather than a
period-specific AI subsystem. The long-term ownership boundary is:

| Layer | Responsibility |
| --- | --- |
| `canwu-core` | Stable decision request, ticket, and trace IDs only. It does not own policy or domain semantics. |
| `canwu-decision` | Domain-neutral ticket/controller contracts, versioned options, accepted/rejected attempt records, traces, deterministic utility evaluation, and Utility/Rule/Human/External/LLM policy interfaces. |
| `canwu-sim` | Authoritative decision state, canonical ingress, non-poisoning rejection admission, deadlines, authority derivation, transactional command admission, commitments, persistence validation, and exact replay. |
| `canwu-api` | The supported facade for creating, refreshing, evaluating, inspecting, saving, and replaying decisions. |
| Domain packages | Decision triggers, actor-relative fact projection, option and blocker generation, utility factors and weights, rules, personality or doctrine, and the command represented by each option. |

A controller is a durable binding between a ticket, a policy identity, and the
authority context under which a selected option may act. It is not an
application-specific AI brain and it does not grant a policy mutable state
access. Policies receive an explicit ticket projection, may return pending or
deferred outcomes, and may select only an option already supplied by the
domain. The controller derives issuer, decision origin, seat, permission
profile, and command subject; ordinary command admission remains the sole path
to authoritative domain mutation.

Dynamic options are explicit, versioned state transitions. Replacing a
ticket's context or options increments its version, preserving a deterministic
cut of what was available at each decision attempt. Human, External, and LLM
responses must name that version, so delayed answers cannot silently act on a
new option set. External and LLM request DTOs expose context and available
option descriptors but not authoritative command actions. Networking,
authentication, retries, model invocation, and operator UI remain host-adapter
responsibilities rather than hidden kernel services.

The generic utility evaluator uses deterministic integer inputs and records a
factor-by-factor score breakdown. Factor names, weights, normalization,
personality, doctrine, uncertainty, and historical interpretation remain
domain or policy data. Rule policies, human control, external services, and LLM
adapters converge on the same constrained option-selection contract instead of
creating parallel mutation paths.

Decision attempts are authoritative admission evidence. Every admitted request
is recorded as accepted or as an expected rejection; stale revisions, stale
ticket versions, closed tickets, and conflicting mutations cannot leave a
permanently failing item at the head of canonical ingress. Decision traces then
explain successful resolution outcomes. Exact replay consumes recorded decision
ingress, attempts, policy identity, selected option, score/evidence trace, and
resulting command attempt; it does not rerun a human, service, or model.
Counterfactual branches may deliberately rerun or replace a policy, but must
then produce new decision ingress and lineage rather than claiming exact replay.

Future multi-stage deliberation, delegation, coalition voting, negotiation,
belief formation, and deliberation memory should compose through tickets,
controller bindings, and domain/plugin state. They should not add
policy-specific hidden mutable state or network clients to the simulation
kernel.

## Persistence and counterfactuals

Snapshots contain deterministic state, clock, RNG state, scheduler sequence,
pending serializable work, knowledge, decision tickets/controllers/attempts/traces,
event history, and command records. A
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

The first reference implementation of population-scale social diffusion is the
unpublished `canwu-society` extension. It keeps cohort distributions, influence
edges, organization topology, institutional alignment, policy pressure,
transition remainders, mobilization candidates, and actor estimates outside the
kernel. Historical labels and meanings remain downstream data. It is evidence
for the plugin/domain-record/decision/knowledge contracts, not a declaration
that its current types are stable public Canwu primitives.

Promotion from this extension into core requires an independently implemented
second domain system needing the same primitive, evidence that the abstraction
is not specific to belief or religion, and a separate compatibility and
migration decision. Until then, `canwu-api` must not depend on or re-export
`canwu-society`.

## Debug client

The first-party client remains a reference consumer. It should prioritize world
inspection, deterministic controls, schema-driven fields, event provenance, and
debug commands. Production map rendering and game interaction belong in adapter
projects such as `canwu-bevy`, `canwu-unity`, `canwu-godot`, and `canwu-web`.
