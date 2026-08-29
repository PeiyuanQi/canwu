# Legal Institutionalization Design Review

Status: completed role-based consensus review of the
[`canwu-law`](legal-institutionalization-framework.md) design, implementation,
documentation, scale measurements, and conformance suite. This is an
expert-lens evaluation of a game-engine architecture, not a claim that an
external academic institution has validated it.

## Acceptance rule

The review proceeds in this order:

1. the engine design gate rejects any path that cannot use current public APIs,
   is non-deterministic, has unbounded work, or contradicts persistence/replay;
2. only after that gate passes, historical and legal lenses score simulation
   reasonableness for a reusable game engine;
3. the engine lens accepts, constrains, defers, or rejects each requested
   fidelity improvement;
4. the cycle ends only when every lens has no blocking objection and each final
   score is at least 8/10.

Scores are design judgments. Empirical historical calibration still belongs to
content packs, case fixtures, benchmarks, and later playtesting.

## Round 1: engine gate

**Result: fail; no academic scoring performed.**

The original proposal assumed APIs that do not exist. Plugin command handlers
cannot mutate domain records, boundary systems cannot directly create decision
tickets, and `CauseRef` cannot identify decision traces. It also lacked an
immutable future-effective version topology, a mapping from multi-seat
procedures to single-controller tickets, exact holder read cuts, semantic load
validation, legal-conflict resolution, and an honest whole-kernel scale bound.

The redesign introduced:

- a persisted decision outbox and deterministic host adapter;
- command-written pending legal intent followed by next-boundary record writes;
- preallocated global ID blocks with lifetime budgets;
- stable rules plus create-only source and law versions;
- per-seat tickets and persisted procedure aggregation;
- exact existing evidence refs and holder knowledge read cuts;
- staged applicability/conflict results;
- module-owned restore validation;
- law-local complexity claims separated from current kernel clone costs.

## Round 2: historical and legal evaluation

After the engineering redesign, the engine gate had no remaining structural
blocker. The two fidelity lenses then found the model implementable but still
too centered on modern promulgated legislation.

| Lens | Score | Blocking gaps |
| --- | ---: | --- |
| Historical simulation | 7.7/10 | Custom and precedent had no distinct origination path; regime succession and reception were absent; publicity, actor knowledge, selective implementation, coercion, and non-cultural change drivers were underspecified. |
| Legal structure | 7.5/10 | Engine authority could be mistaken for in-world legal validity; legal source and normative rule were conflated; rights lacked holder/duty-bearer/remedy relations; applicability used one broad precedence profile; case, finding, standing, and ruling contracts were too thin. |

## Engine adjudication

The engine lens accepted every foundational concern, with the following
implementation constraints:

| Evaluation request | Engine decision |
| --- | --- |
| Add promulgated, adjudicated, accreted, agreed, and received source modes | Accept as compiled profiles; do not add period-specific kernel branches. |
| Model customary law | Accept as an order-specific evidence-backed claim; reject a universal belief-percentage threshold. |
| Separate engine authority from legal competence and validity | Accept; authorized but defective acts may become `Purported` or `Contested`. |
| Separate source, stable rule, and immutable normative change | Accept as `LegalSourceVersion`, `LegalRule`, and `LawVersion`. |
| Represent rights, duties, powers, immunities, standing, and remedies | Accept as bounded normative descriptors and sparse effects; reject dense person-by-rule state. |
| Improve conflict of laws | Accept as a typed staged applicability pipeline with an explicit trace; unresolved questions return `Contested`. |
| Add cases, findings, proof standards, and rulings | Accept as bounded records and profiles; reject unrestricted fact inference or a legal theorem prover. |
| Add promulgation, knowledge, enforcement, and compliance gaps | Accept through holder knowledge and downstream sparse implementation state. |
| Add conquest, split, union, restoration, and reception | Accept through immutable succession records and bounded reception tables. |
| Guarantee history-independent whole-engine cost | Reject under the current kernel; retain global record/decision budgets and report clone costs honestly. |

The historical and legal lenses accepted these constraints because the target is
a deterministic game-engine extension, not exhaustive jurisprudential research.

## Round 3: consensus scores

| Lens | Final score | Reason for acceptance |
| --- | ---: | --- |
| Senior game-engine design | 9.1/10 | The paths map to current public contracts; state ownership, determinism, replay, restore validation, budgets, and known kernel scale limits are explicit. |
| Historical simulation | 9.0/10 | The design supports plural sources, custom, contested orders, regime succession, publicity and knowledge lags, selective implementation, and multiple causal inputs without privileging one period. |
| Legal structure | 8.9/10 | Source, rule, normative relation, competence, validity, applicability, adjudication, remedy, implementation, and evidence are separated at a useful game-engine granularity. |

**Design consensus: accepted for implementation, subject to every conformance
and benchmark gate in the framework.** No lens had a blocking design objection.

## Round 4: implementation audit

The completed milestone was reviewed again against the code rather than the
proposal alone. The first implementation pass exposed four blocking issues:

- the applicability query traversed version history twice while charging one
  nested-work estimate;
- historical `read_at` queries did not consistently gate visibility on actual
  publicity;
- delayed publicity rewrote create-only source and law-version snapshots; and
- cold restore did not revalidate every publicity/adoption/effectiveness time
  relationship.

The implementation then changed to one reverse traversal per rule, charged each
visited version and nested descriptor before access, derived publicity at the
query read cut, represented delayed publication as an independent immutable
event linked from the proposal, left adoption snapshots create-only, and
revalidated the complete lifecycle topology during restore. Raw persisted
payload ceilings are also checked before typed decode and full semantic scans.

Final implementation scores:

| Lens | Final score | Implementation judgment |
| --- | ---: | --- |
| Senior game-engine design and scale | 9.2/10 | Deterministic ownership, exact evidence, bounded work, replay, restore, and scale limits are closed with no blocker. |
| Historical simulation | 9.1/10 | Cultural support, plural source modes, publicity and knowledge lags, contested change, succession, reception, enforcement, and retirement are historically credible engine abstractions. |
| Legal structure | 9.1/10 | Adoption, validity, publicity, effectiveness, normative relations, applicability, adjudication, remedy, and immutable versions form a legally coherent bounded model. |

The public English and Chinese pages were also aligned with the implementation:
an accepted command schedules an intent for a later legal boundary; women's
suffrage carries period-specific intersectional exclusions; and the
Enlightenment/human-rights example models plural intellectual roots,
transmission networks, countermobilization, partial adoption, reinterpretation,
delayed publicity, selective enforcement, and rejection rather than automatic
linear progress.

The final engine adjudication accepted the corrected delayed-publicity
documentation, the non-linear Enlightenment/human-rights example, the
intersectional suffrage caveat, and the accepted-command-to-later-boundary saga.
It deliberately deferred a further unpublished-amendment regression test and
partial/multi-medium publicity profiles to later bounded increments, and
constrained any public actual-publicity helper to a future concrete consumer so
that derived lifecycle state cannot be confused with immutable adoption
snapshots.

**Final consensus: the implemented one-milestone culture-and-law system passes
all three lenses with no blocking objection and is accepted for merge.**

## Intentional residual limits

These limits lower the score from a theoretical 10 but are not blockers:

- the current kernel still clones the full domain-record and decision state at
  a transaction, so long campaigns require global budgets until delta/COW and
  closed-decision archival exist;
- content authors, not the generic engine, determine historical categories,
  doctrine, proof weight, institutional behavior, and calibration;
- natural-language interpretation, universal moral correctness, unrestricted
  legal reasoning, and exhaustive private-law doctrine are outside scope;
- one v1 publicity event covers its declared scope as a bounded simplification;
  staged, partial-territory, oral, and multi-medium publicity belong in later
  content and conformance packs;
- downstream convenience APIs may later expose derived actual-publicity state
  so callers do not mistake an immutable adoption snapshot for the live
  publicity lifecycle;
- further multi-version and live-provider negative tests would strengthen an
  already passing contract suite, but are not required to establish the current
  invariant set;
- final historical calibration still requires cross-period content fixtures and
  gameplay validation beyond engine conformance.
