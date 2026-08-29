---
name: canwu-contributor-design
description: "Design or review changes to the Canwu engine repository, including public contracts, domain extensions, persistence, replay, and scale boundaries. Use for Canwu contributors; do not use for developers building a game or historical simulation on top of Canwu."
---

# Design Canwu Contributions

Use this skill when contributing to the Canwu engine repository. Turn a
proposed engine capability into an implementable, evidence-backed design.
Prefer the smallest model that explains the required causal behavior. A good
design says what belongs in Canwu, what belongs in a domain package, and what
must remain in an engine user's host application. For a game or historical
simulation built on Canwu, use the engine-user documentation and usage skills
instead.

## Establish the baseline

1. Work from the Canwu repository root. Read AGENTS.md, CONTRIBUTING.md, and
   every nearer AGENTS.md before editing or making a recommendation.
2. Read the smallest relevant parts of docs/architecture.md,
   docs/end-state.md, docs/engine-conformance.md, docs/versioning.md, and
   docs/terminology.md. Search before loading long documents.
3. Inspect the current public canwu-api types, examples, and affected
   extension sources. Separate implemented behavior, accepted design, and
   proposed work. Never treat a type or document sketch as an implemented
   contract.
4. If a prior task or session is cited, inspect the available project-linked
   history that bears on the decision. Extract recurring decisions,
   contradictions, and superseded plans. Treat session output as an unmerged
   proposal until the current checkout, origin/main, documentation, and source
   confirm it. Do not inherit a session's branch or worktree assumptions.
5. State the invariant, the contributor problem and intended engine outcome,
   the actors and observers, the simulation granularity (aggregate, group, or
   actor), the time horizon, and the evidence that will prove the design.

## Place the responsibility

Use this ownership test before inventing a type or changing a crate:

| Layer | Owns | Must not own |
| --- | --- | --- |
| Simulation core | Generic identity, deterministic time, records, scheduling, transactions, allocation, evidence, decisions, persistence, and replay | Period-specific entities, formulas, scenarios, or presentation |
| Public API | The supported boundary for applications and extensions | A second mutable runtime or private runtime internals |
| Domain extension or simulation plugin | Reusable domain state, rules, commands, boundary systems, and actor projections | Changes that bypass canonical ingress or require one historical setting |
| Reference content pack | Versioned, namespaced definitions, scenario data, localization, provenance, and balance profiles | A solver, kernel subsystem, or unverified runtime input |
| Reference integration or starter kit | A concrete world/economy mapping and a runnable composition of public contracts | Generic engine semantics or a privileged compatibility path |
| Host application | Historical content, product policy, controllers, external services, rendering, input, audio, animation, and presentation time | A second authoritative simulation truth |

Period-specific rules such as Ming taxation, a government form, a technology
case, or a cultural movement stay above the simulation core. Promote a primitive
only when an independent second domain needs the same contract and the
implementation and verification surface are clear.

## Design the causal model

Describe the flow before writing structs:

    authored definitions -> validated or compiled plan
    -> admitted input -> ordered transaction phases
    -> atomic state and evidence -> detached actor or research view

For each step, specify:

- stable typed IDs, ownership, references, schema/version, and lifecycle;
- the command, event, scheduled work, or decision input that starts it;
- preconditions, authority, expected revision/time, idempotency, and rejection;
- reads, writes, resources, random streams, emissions, and visibility timing;
- the resulting records, causal links, projections, and downstream signals.

Keep authored definitions, immutable revisions, dynamic state, derived indexes,
and historical evidence separate. Use exact record-version references whenever
the meaning of evidence depends on a mutable record. Make lifecycle explicit
when a record can become inactive: retire hot dynamic state and indexes, keep a
tombstone and historical evidence, and require an explicit new generation to
reactivate it. A retired cultural idea must not erase a law, right, event,
knowledge record, or other effect already committed by its owning domain.

Do not use a global unlock flag, one aggregate balance, or a dense
person-by-person or idea-by-person matrix to stand in for causal state. Keep
information exposure, actor knowledge, belief or stance, capability, adoption,
institutional effect, and execution or compliance distinct when they can
diverge. A document can be seen without being understood; understanding does
not imply belief, capability, or use.

## Enforce runtime boundaries

- Use canwu-api from clients, integrations, and domain packages. Never expose a
  mutable live state or make a client depend directly on canwu-sim.
- Enter external mutation through validated commands, admitted events, or
  versioned experiment inputs. The issuer and authority must be checked before
  domain mutation; expected conflicts should become structured evidence where
  the public contract permits it.
- For actor-facing workflows, read actor-relative knowledge and call
  available_actions before act. Missing knowledge is not permission to fall
  back to ground truth. Preserve source, confidence, observation time, and
  information age when the model provides them.
- Keep simulation time deterministic and independent of wall or presentation
  time. Use ordered collections, explicit sequence or tie-break keys, and
  integer or fixed-unit values where arithmetic affects replay.
- Put new mechanics in declared transaction phases. Use reservations and
  deterministic allocation for conserved competing resources, same-boundary or
  next-boundary visibility deliberately, and one atomic commit or rollback.
  A domain rejection must not poison later valid ingress.
- Declare system reads, writes, resources, random streams, and emissions.
  Connect domains through validated records or bounded next-transaction signal
  batches; do not use synchronous recursive cross-plugin mutation.

Decision and controller designs must keep this chain visible:

    actor-relative facts -> DecisionTicket -> existing option ID
    -> canonical decision ingress -> DecisionAttempt and DecisionTrace
    -> validated command ingress

Policies receive detached context and current option descriptors. They do not
receive mutable state or command authority, and they cannot invent an option or
an authority envelope. Keep engine authorization, in-world legitimacy,
applicability, actor knowledge, and execution as separate concepts.

## Prove scale and durability

Name the cardinalities and the hot path: active records, dirty relationships,
edges, observers, scheduled work, cross-domain fan-out, snapshot bytes, and
replay horizon. Prefer sparse indexes, dirty sets, bounded queries, compiled
plans, and cold historical storage. A high-level builder must not hide an
all-targets-by-all-observers product.

State the expected complexity in terms of active or changed work, not merely
catalog size. Add a benchmark matrix that varies the dominant dimensions and
measures transaction time, allocation, memory, snapshot/load, index rebuild,
query cost, and exact replay. Do not call a design scalable until those
measurements and their limits exist. Separate component evidence from a
whole-game or hardware guarantee.

List every persisted item: run and content manifests, plugin semantic
identity, schemas, queues, records, lifecycle generations, derived-index
rebuild inputs, random positions and draw evidence, counters, commitments,
causal links, rejection records, and fork lineage. Define save/load validation,
tamper behavior, migration or clean-break behavior, and the difference between
exact replay and a new fork with different input. Never read an unverified
external content file during a transaction.

## Review until the design converges

Use independent review when available, giving each reviewer the minimum raw
artifacts and a realistic request rather than the expected answer.

1. Ask an engine designer to check public-API implementability, ownership,
   authority, transaction atomicity, deterministic ordering and randomness,
   persistence, replay, portability, and hidden scale costs.
2. Ask a domain or history/game-design reviewer to check causal plausibility,
   uncertainty and provenance, meaningful player choices, observability, and
   whether the abstraction is useful as a game engine rather than an exhaustive
   historical database.
3. Add performance, hardware, security, or binding review when the design
   materially depends on those constraints.
4. Record each finding as accepted, rejected with a technical reason, or
   deferred with an explicit boundary. Repeat review after resolving blockers.
   Consensus means no unresolved blocking contradiction, not forced agreement
   on every optional preference.

Cross-validate generic designs with the same schema and different data. Use
Southern Ming and Qing as a useful paired check when governance or historical
institutions are involved, and add a non-Ming case when the claim is generic.
Vary configuration and content rather than adding one-off code branches.

## Required design handoff

The final design should contain:

- the decision, invariant, scope, assumptions, and non-goals;
- an ownership and affected-surface map grounded in AGENTS.md;
- records, typed IDs, revisions, lifecycle, visibility, and causal links;
- the input-to-transaction-to-evidence-to-view flow;
- authority, actor knowledge, controller, and failure semantics;
- cardinality, hot-path complexity, budgets, indexes, and benchmark gates;
- snapshot, migration, exact replay, fork, and tamper behavior;
- one small vertical slice plus durable acceptance checks;
- documentation, terminology, and bilingual mirror work required;
- a clear separation of implemented behavior, planned work, and open questions.

If implementation is requested, inspect every affected public, persistence,
replay, documentation, and test surface in the change map, then make the
smallest coherent change. Add committed tests only for reusable, non-trivial
contracts or failure paths; run narrower checks inline otherwise. If website
copy changes, update the canonical documentation and supported language
mirrors, and obtain the repository-required independent readability review
before deployment. Do not claim conformance or scale from a design sketch.
Commit or push only when the user explicitly requests Git delivery.

## Red flags

Stop and revisit the design when it introduces a historical entity into the
core, a second authoritative state, a direct canwu-sim client dependency, an
omniscient actor view, wall time in authoritative state, a global technology or
belief unlock, a synchronous plugin callback, an unbounded cross-product, a
full catalog scan on every transaction, silent history deletion, or a public
API whose cost and authority are not visible.
