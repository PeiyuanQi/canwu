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

## Model ownership

Generic engine crates own deterministic simulation contracts, not a mandatory
historical world. Concrete people, governments, territories, armies, letters,
map coordinates, and the events that describe their domain behavior belong in
replaceable reference integrations or host applications. Detached reads remain
a supported public pattern, but each integration owns the shape of its world
projection.

The engine retains generic event-journal semantics: stable identity, simulation
time, causal provenance, correlation, affected references, player-facing
visibility, validation, persistence, and replay. Concrete domain event payloads
must not become permanent kernel vocabulary. The accepted migration direction
and compatibility gates are defined in the
[world and event ownership audit](proposals/world-event-ownership-audit.md).

## Information flow

The reusable kernel should preserve facts about who holds which record, when it
was learned, which registered schema gives it meaning, and which evidence or
prior records it cites. It should support people and eligible institutions as
holders, bounded deterministic queries, current and historical views,
contradiction and supersession links, atomic publication, exact persistence,
and replay. Those contracts must remain usable for correspondence, reports,
telecommunication, intelligence, diplomacy, military coordination, public
notices, and later media systems without embedding any one period's vocabulary
or assumptions.

The kernel should not decide routes, couriers, interception, encryption,
recipient expansion, partial delivery, organizational interpretation, belief
change, reputation effects, or public presentation. Extensions own those
processes and publish their results through the common holder-ledger contract.
A transport or audience extension may maintain derived routing and membership
indexes for scale, but every such index is rebuildable from canonical extension
state and excluded from authoritative commitments unless deliberately promoted
through a separately versioned kernel contract.

The supported query surface should remain bounded and cursor-safe at large
holder histories. Performance evidence must distinguish publication cost,
snapshot growth, query cost, load/index-rebuild cost, replay throughput, and
compact-archive cost. Recorded baselines are engineering evidence, not a
machine-independent service-level objective.

The first authoritative implementation was the
`canwu-information` extension for the 0.5 line; it is now a published
experimental crate. Publishing it makes the record vocabulary, lifecycle
helpers, and high-fan-out continuation ergonomics available to independent
consumers while keeping the extension optional.
The kernel holder-ledger, evidence, archive, and keyed-random contracts are the
stable reusable boundary; each extension remains a separately reviewed
compatibility decision.

The published experimental `canwu-correspondence` domain extension composes this neutral
ledger with actor-relative route knowledge and transport execution. It owns
communication opportunities, admitted sender/recipient intent, address
resolution, accepted route evidence, incident policy, and cross-extension
orchestration. Application and channel adapters still prepare content and
dispatch records, supply period-specific network/address knowledge, and admit
scarce capacity. The information ledger does not acquire route search, and the
router does not acquire dispatch or retry lifecycle state. The implemented
slice requires the carrier holder to be the sender and reads only that
sender-owned ledger; delegated-carrier disclosure and authority remain future
contracts.

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
| `canwu-api` | The supported public API for creating, refreshing, evaluating, inspecting, saving, and replaying decisions. |
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

The reusable routing/transport boundary now separates planning from execution.
`canwu-routing` plans against an observer-relative, versioned `PlanningSnapshot`
and returns an immutable route estimate. `canwu-transport` records itinerary
revisions, leg execution, custody handoffs, capacity bookings, and an
arrival-pending completion saga. It does not own the information ledger or the
simulation scheduler. A route estimate is never the same thing as an
information deadline: `DeliveryAttempt.due_at` remains the logical completion
deadline, while ETA can change after a disaster and trigger a reroute.

Movement commands follow the same boundary. The canonical intent is
`OrderMovement`, with subject-specific transit and custody state. A voluntary
person trip is an actor-bound self-move; an army order, cargo dispatch, forced
relocation, and custody transfer use different capability policies.
`EntityRef` identifies a subject but does not make it movable, so unsupported
subjects fail closed. See the [movement order proposal](proposals/movement-order-mechanism.md)
for the capability matrix and migration requirements.

This supports ancient relay stations, modern roads, 1900 or 1940 railways,
air routes, and telegraph/signal systems through data-driven modes and
traversal models. Disaster handling is explicit: a domain system records the
failed leg, transport enters `ReplanPending`, and a new immutable itinerary
revision is installed. A reroute does not create a new attempt; a retry does.
Derived route caches are rebuildable and never authoritative state. See the
[routing and transport proposal](proposals/routing-transport-mechanism.md) for
the complete ownership and milestone boundary.

The first complete correspondence vertical slice is specified in the
[correspondence proposal](proposals/correspondence-mechanism.md). Rerouting
keeps the same delivery attempt and creates a successor itinerary revision;
only a true retry creates a successor attempt. A failed attempt leaves the
dispatch active until explicit sender-authorized retry or finalization. Replan
of the same attempt applies only before terminal failure, while it is waiting
for a route and transport is `ReplanPending`.
Interception records access and does not imply delivery termination. Capacity
allocation remains separate from pure route search. The current correspondence
request is explicitly `Unconstrained`; constrained execution needs a future
admission contract carrying exact booking or reservation evidence.

## Systems and plugins

Population, agriculture, trade, taxation, bureaucracy, military logistics,
diplomacy, migration, disease, and climate should remain separately testable
plugins. The core may standardize broadly shared primitives only after at least
two concrete systems need them.

The first generic technology implementation is the published experimental
`canwu-technology` domain extension. It demonstrates that invention and
diffusion can be represented without a universal unlock tree: immutable
technique revisions are tested through evidence-bearing attempts, qualified
for particular operations at particular sites, installed, evaluated for a
specific use, and adopted by an authorized holder. Claims, observations,
capability, implementation, adoption, and transmission remain orthogonal.
Papermaking, woodblock printing, movable type, gunpowder, and steam engines are
cross-validation data profiles, never solver branches.

Detailed historical interpretation remains optional. The three
`canwu-history-research` plugins record bounded assessments of sources,
practice, and production archaeology without changing base simulation truth.
Games can omit all three, select one, or enable the suite. Promotion of any
technology or research concept into the kernel still requires a second
independent domain consumer and a separate compatibility decision.

The current home-computer envelope is deliberately paced. Recorded component
evidence at 100 sites supports turn-based play, while the 500-site pressure
profile is suitable for campaign turns or offline analysis, not immediate
interaction. The measurement used an 8-core, 32-GiB machine and is not a
whole-game 4-core/8-GiB certification. Future delta transactions and
incremental validation may raise that ceiling, but historical fidelity plugins
are not a substitute for those kernel improvements.

The first reference implementation of population-scale social diffusion is the
published experimental `canwu-society` **social diffusion simulation module**.
Architecturally, it is an experimental **domain extension** built on Canwu's
public engine contracts. It keeps cohort distributions, influence
edges, organization topology, institutional alignment, policy pressure,
transition remainders, mobilization candidates, and actor estimates outside the
kernel. Historical labels and meanings remain downstream data. It is evidence
for the plugin/domain-record/decision/knowledge contracts, not a declaration
that its current types are stable public Canwu primitives.

Promotion from this domain extension into core requires an independently
implemented second domain system needing the same primitive, evidence that the
abstraction is not specific to belief or religion, and a separate compatibility and
migration decision. Until then, `canwu-api` must not depend on or re-export
`canwu-society`.

### Reference content and starter kits

Canwu should ship or maintain a first-party collection of reference content
packs, reference integrations, and starter kits above the generic domain
extensions. This is a usability layer for engine users, not a promotion of
historical content into the kernel.

Reference content packs provide versioned, namespaced, serializable definitions
such as technology families, process variants, social targets, scenario seeds,
localization, balance profiles, and provenance. Reference integrations provide
small public-API implementations that map those definitions to a world,
production, information, or society model. Starter kits compose both into a
runnable host application with a complete vertical slice.

The collection is expected to grow. A pack must be usable with more than one
compatible integration, and an integration must be replaceable without
rewriting the pack. Packs and integrations remain downstream packages, with
independent versions and content hashes recorded in the scenario/run manifest.
The runtime must consume validated materialized data and registered handlers;
it must never read an unverified external content file during settlement.

The first-party acceptance bar is higher than an API snippet: every starter kit
must use only the supported public API, exercise authoritative commands and
boundaries, expose actor-relative reads where relevant, and prove save/load,
fork, and exact replay. It is reference code and content that users can copy or
replace, not a privileged compatibility path.

## Debug client

The first-party client remains a reference consumer. It should prioritize world
inspection, deterministic controls, schema-driven fields, event provenance, and
debug commands. Production map rendering and game interaction belong in adapter
projects such as `canwu-bevy`, `canwu-unity`, `canwu-godot`, and `canwu-web`.
