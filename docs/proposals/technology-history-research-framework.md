# Technology and Historical Research Framework

Status: revised single-milestone implementation plan after independent review.

## Objective

Let a game make invention, adaptation, reproducible practice, implementation,
use-specific adoption, and diffusion playable without putting a technology tree
or a historical database in Canwu's kernel. The implementation must represent
papermaking, woodblock printing, movable-type printing, gunpowder, and a steam
engine regression profile through shared data and rules.

This is one delivery milestone. The steps below are an execution order, not
separate releases or deferred features.

## Invariants

```text
heard claim != correct claim
claim confidence != claim truth
attempt execution != witness observation
successful attempt != reproducible practice
practical qualification != implementation
implementation != use-specific adoption
adoption at one site != diffusion to every site
historical assessment != authoritative simulated fact
```

- Canwu core gains no technology ID, research points, era levels, global
  unlocks, or hard-coded prerequisite graph.
- Deliberate actions use tracked commands with issuer, authority, revision, and
  time guards. Plugin ingress accepts observations, provider results, bridges,
  and scheduled continuation; it cannot directly create qualification or
  adoption.
- Technique content and historical profiles are data. Shared code never
  branches on a historical name, date, place, or case identifier.
- Specific technical semantics are immutable revisions. Changing a parameter,
  criterion, or design creates a new revision with explicit ancestry.
- Persons and institutional domain entities can hold knowledge. Information
  access, embodied practice, local capability, and adoption remain distinct.
- Sparse state grows with active relationships rather than all actors multiplied
  by all sites, techniques, variants, and applications.
- Ordered collections, integer/fixed units, stable identities, explicit tie
  breaks, and persisted evaluator identities preserve replay.

## Package boundary

```text
canwu core and canwu-api
  generic records, knowledge, commands, ingress, settlement, persistence,
  replay, hashing, actor-relative queries

canwu-technology (unpublished; depends only on canwu-api)
  generic immutable technique revisions, programs, attempts, observations,
  claims and assessments, capabilities, minimum production evidence,
  implementations, applications, adoption, teaching opportunities

canwu-history-research (unpublished; optional; depends on canwu-technology)
  three independently selectable assessment plugins and read-only analysis

reference starter and profiles
  neutral playable vertical slice plus five data-only stress profiles
```

Neither experimental crate is re-exported by `canwu-api`. Games that omit the
historical suite pay no historical-record or handler cost. Plugin descriptors
in a snapshot fix the executable plugin set. Scenario, rule, evaluator, content,
and source fingerprints are separately fixed by `RunManifest`.

## Base technology contract

Every semantic or evidentiary edge that can outlive a record update uses an
exact `DomainRecordVersionRef` in its payload and a validated domain reference
for current existence.

| Record | Purpose | Lifecycle |
| --- | --- | --- |
| `MetricSchema` | namespaced metric, unit, integer scale, range, and comparison semantics | immutable revision |
| `TechniqueSpec` | stable function and bounded requirement/criterion groups | immutable revision |
| `TechniqueRevision` | concrete process/design, multi-parent ancestry, parameters and evaluator identity | immutable revision |
| `ApplicationSpec` | one use and the metrics by which that use is evaluated | immutable revision |
| `TechnicalProgram` | holder/site objective, mode, time window, provider requirements, and status | versioned |
| `ExperimentAttempt` | actual input, environment, operator, asset bindings, output, start/end, and evaluator result | immutable evidence |
| `AttemptObservation` | witness-relative observation, method, uncertainty, and cited attempt | immutable evidence |
| `TechnicalClaim` | proposition, scope, source relation, correction/contradiction links; no global confidence | immutable evidence |
| `ClaimAssessment` | holder-relative confidence, method, as-of time, support and contradiction | immutable evidence |
| `CapabilityQualification` | holder/operator, site, revision, operation, reliability, validity/last practice, and evidence predicate | versioned |
| `AssetBinding` | minimum durable asset identity/state used by technical execution; no general inventory or market model | versioned |
| `ProductionRun` | bounded aggregate inputs, outputs, asset use, operator, quality and failure evidence | immutable evidence |
| `ImplementationRecord` | installed local configuration, capacity, reliability, maintenance provider and active state | versioned |
| `AdoptionRecord` | adopter, site, application/use, scale, revision portfolio, trial/committed/suspended/abandoned state and decision evidence | versioned |
| `TransmissionOpportunity` | mode profile and evidence for exposure, demonstration, apprenticeship, artifact inspection, personnel transfer, or independent investigation | versioned |
| `TechnologyOperation` | canonical input hash, command/ingress cause, terminal outcome and result refs | immutable after atomic completion |

Arrays, maps, strings, payload bytes, references, graph degree, and ancestry
depth are checked by `TechnologyLimitsV1`; shallow `PayloadSchema` validation is
not treated as sufficient.

### Executable evaluation

The extension provides a versioned CPU reference evaluator. It validates metric
units and ranges, alternative requirement groups, exact revision identity, and
integer threshold predicates over explicit attempt inputs and outputs. It
returns criterion evidence, never a global `invented` boolean.

Qualification rules are data-driven evidence predicates such as required
operations, unique successful attempt versions, distinct actual operators,
freshness, and reliability floor. Independent reproduction is an optional
rule over separate attempt records, not a label supplied by content.
Application viability evaluates bounded cost, output,
quality, reliability, risk, and institutional metrics for one use. It produces
an actor-visible adoption candidate; only an authorized command can commit,
suspend, or abandon adoption.

Catalog revisions may be loaded from content. Runtime invention creates an
immutable revision only as a provider result linked to an active investigation,
adaptation, or reverse-engineering program and explicit discovery evidence.
The reference evaluator applies that exact revision's parameters; the
milestone does not implement an unbounded continuous parameter search.

### Authority and operation matrix

| Action | Admission path | May directly create |
| --- | --- | --- |
| load metric/spec/revision/application content | initial scenario bound by manifest fingerprints | immutable catalog records |
| start, revise, pause, cancel program | tracked plugin command; optional DecisionTicket requirement | program mutation |
| authorize trial or production run | tracked plugin command | pending program intent only |
| submit resolved provider result or passive observation | plugin ingress with exact correlation/evidence | attempt, observation, production run, or program-linked technique revision |
| qualify or revoke practice | tracked command whose handler validates evidence predicate | qualification mutation |
| install/revise implementation | tracked command with asset/provider and qualification evidence | implementation mutation |
| commit/suspend/abandon use | tracked command; DecisionTicket provenance can be required by content | adoption mutation |
| open active teaching/apprenticeship | tracked command | transmission opportunity linked to an optional destination program |
| information access, person/cargo arrival, provider completion | declared next-boundary adapter ingress | exposure/program input only |

All commands define exact retry, operation-ID collision, stale revision/time,
unauthorized, and expected domain rejection behavior. An operation record is
created only with its terminal result in the same atomic boundary; there is no
partly completed create-only operation state.

### Settlement matrix

The implementation does not pretend current dynamic reservations support a
program-sized request family. Resource and asset requirements cite versioned
provider acknowledgements; the neutral starter leaves that list empty. Real
economy plugins remain owners of stock, prices, work in progress, and
cross-boundary conservation.

| Boundary | System | Cadence | Reads/writes and visibility |
| --- | --- | --- | --- |
| tracked command admission | technology intent | event-driven | validates payload, issuer/subject authority, exact retry and collision; schedules owned ingress, no direct record write |
| 7 | technology operation apply | event-driven | reads admitted owned ingress and exact evidence bodies; re-runs bounded predicates and atomically writes the target plus terminal operation record, `SameBoundary` |
| 12 | technology intent finalization | event-driven, admitted-operation bounded | for an applied provider result, writes the pending exact intent to `Consumed` with exact ingress, operation, and result refs; later failure rolls the whole boundary back |
| 13 | holder knowledge publication | event-driven, admitted-operation bounded | publishes registered schemas only for successfully proposed records whose provider intent, when present, is already `Consumed`, `SameBoundary` |

Every handler first filters its own admitted ingress/correlation in O(1) or a
bounded lookup. Continuations persist a canonical cursor and fixed chunk size;
wall time, memory pressure, cache warmth, and thread scheduling never change
work selection.

### Knowledge, queries, and restoration

Neutral holder-relative schemas cover claim awareness, attempt observation,
qualified practice, implementation observation, and adoption assessment.
Knowledge publication states only what its holder has evidence to know.
`ClaimAssessment` can be authoritative as a record of an assessor's view while
its proposition remains disputed.

The trusted host receives a deterministic `DomainRecordPage` bound to kind,
exclusive record cursor, and authoritative revision/read cut. A later page with
a stale revision is rejected. Boundary views merge only the requested B-tree
kind/page with overlays, proportional to `O(log N + page + overlay page)`.

`canwu-technology` supplies validated snapshot/checkpoint/fork/exact-replay
wrappers analogous to other experimental modules. Restoration scans bounded
pages and revalidates limits, immutable revision ancestry, exact version refs,
operation results, qualification predicates, adoption/use links, and rebuildable
indexes. Core schema/manifest validation remains necessary but is not claimed
to prove these domain semantics.

## Optional historical research suite

Three plugins balance independent cost with manageable manifest combinations:

| Plugin | Authoritative records of assessments | Never owns |
| --- | --- | --- |
| `HistoricalSourcesPlugin` | assessor, method/version, as-of time, dating/authenticity/reliability range, provenance assessment, support/contradiction over referenced information/evidence | source artifact, content, representation, technical truth |
| `HistoricalPracticePlugin` | named practitioner/workshop/institution relation assessments; detailed notebook annotations and negative-result interpretations | base attempt truth, base capability, information transfer |
| `ProductionArchaeologyPlugin` | material/asset/sample observations, dating, uncertainty, inferred production hypothesis and competing interpretation | runtime assets, lots, work orders, production runs, costs |

All research records are bounded, append-only assessments with explicit
assessor, method, uncertainty/range, as-of time, citations, and contradiction or
supersession links. `HistoricalAnalysis` is host-side and read-only. Each plugin
responds only to its own ingress. A `HistoricalResearchSuite` helper returns the
three plugins without hiding their separate descriptors.

Information owns artifacts, representations, access, and interpretation;
transport owns movement; production/economy owns real inventory and markets;
technology owns technical evidence and adoption state; historical plugins own
assessments of evidence. Bridges cite exact versions or schedule declared
next-boundary ingress.

## Cross-technology conformance

One shared starter harness loads five data profiles; case names are labels in
fixtures, not solver inputs.

| Case | Required counterfactual |
| --- | --- |
| Papermaking | local fiber and operator evidence reverse the attempt outcome |
| Woodblock printing | a stable long edition passes its use criteria while a changing short edition does not |
| Movable-type printing | edition size and text-change frequency reverse its viability relative to woodblock |
| Gunpowder | the same weak powder passes a flame use but fails propulsion |
| Steam engine | cheap-fuel pumping passes while costly pumping and low-torque rotary use fail |

Each profile also commits a successful attempt through catalog loading,
authorized project creation, provider result ingress, boundary settlement, and
authoritative state lookup. The full neutral flow separately proves evidenced
qualification, local implementation, use adoption, a destination learning
program, holder-relative knowledge, save/load, fork, and exact replay.

## Home-hardware contract

Current kernel boundaries clone and revalidate broad state. This milestone does
not claim to remove that architectural cost. It therefore uses an honest,
fixed paced-interactive component profile and records the larger target only as
a pressure measurement.

### Paced-interactive component profile

- reference machine: named CPU/RAM/SSD, release build, and recorded
  compiler/source/profile fingerprints; an 8 GiB deployment must repeat the
  profile with its renderer and asset workload present;
- 100 sites, 5 techniques, at most 40 immutable revisions;
- 200 active programs and 400 transmission
  opportunities; at most 5,000 technology records, 1,000 records in each
  history plugin, 8,000 combined extension records, and 5,000 technology
  knowledge records;
- per record at most 16 KiB encoded payload, 32 references, 64 collection
  entries, graph degree 16, ancestry depth 8;
- per boundary at most 127 combined extension mutations: 64 technology plus
  21 from each history plugin; technology publication is capped at 32;
- every technology collection is independently capped at 64 entries, future
  attempt/claim/production facts are rejected, and terminal operation records
  are reserved inside the shared 5,000-record limit before reduction;
- active records declare every older exact payload needed for validation through
  the engine's payload-required continuation field. Provider-backed archive
  reconstruction restores those bodies before authoritative continuation;
- module-owned restore validation also rechecks each technology knowledge
  record's holder, schema, subject, exact version, and copied record payload;
- serialized technology hot-state target at most 16 MiB; the whole-runtime
  current checkpoint is used as a conservative upper bound and must remain at
  most 48 MiB; flat snapshot and checkpoint-plus-segments are reported separately;
- peak process RSS at most 1.5 GiB, each RSS scale measured in a fresh process;
- ordinary operation p95 at most 200 ms, monthly boundary p95 at most 500 ms,
  snapshot serialization plus disk write/sync at most 2 s, and validated load
  at most 5 s on the named machine.

The stable elapsed measurement uses warmup plus at least 30 samples. Scaling fixes all
other axes and compares the 100/200/400 interactive profile with the
500/1,000/2,000 pressure profile. A separate kernel structural test proves that
a selected-kind page remains bounded among 100,000 unrelated domain records;
this is not presented as a noisy portable elapsed-time gate. The first recorded
technology baseline reports fresh-process RSS and raw elapsed samples;
allocation-counting evidence is recorded separately when the shared harness
allocator is enabled.

### Pressure profile

The bounded pressure scenario uses 500 sites, 1,000 active programs, and 2,000
links. Its observed time, snapshot/checkpoint bytes, disk I/O, growth, load,
reconstruction, replay, and RSS are reported, not presented as an interactive
guarantee until kernel delta transactions, incremental validation/commitment,
and clone avoidance exist.

A 20-year/240-month workload reports current hot state, flat snapshot,
checkpoint, evidence segments, disk growth, load/reconstruct, and exact replay.
Create-only evidence kinds have explicit current/lifetime caps; exceeding a cap
fails deterministically with a stable domain error. Full prose and images remain
content-addressed external resources; hot records retain summary metadata and
digests.

## Review disposition

| Reviewer | Decision | Plan response |
| --- | --- | --- |
| Engine senior designer | Fail until authority, executable rules, phase limits, exact revisions, restoration, ownership and scope close | added command/ingress matrix, reference evaluator, phase table, exact version refs, module validator, ownership map; collapsed five research plugins to three bounded assessment plugins while retaining one delivery |
| Home-hardware feasibility | Block until full-boundary clone cost and unbounded records are acknowledged | defined a fixed paced-interactive component envelope at current-kernel limits, added hard per-kind/payload/work caps and reproducible benchmark method; retained the 500-site workload as pressure evidence rather than an interactive claim |
| Historical simulation researcher | Request changes until revision, use/adoption, production truth, observation and claim layers are explicit | made revisions immutable; replaced linear stages with orthogonal evidence; added application/adoption, base asset/production evidence, observations/assessments, typed metrics and multi-mode transmission |

## One-milestone acceptance gates

1. Bounded read-cut paging, `canwu-technology`, the three-plugin historical
   suite, neutral starter, five profiles, benchmarks, docs, website, delivery,
   and deployment all land together; none is postponed.
2. Both crates compile through the supported public API with no reverse dependency.
3. Authority, exact retry/collision, evidence predicates, bounds, actor privacy,
   rollback, restoration, tamper, fork, replay, and manifest mismatch have
   durable contract tests.
4. Interactive and pressure benchmarks report the defined evidence; the
   paced-interactive component profile meets its measured named-machine limits and
   the pressure profile is explicitly non-interactive.
5. Architecture, end-state, versioning, terminology, crate READMEs, benchmark
   notes, and one bilingual tutorial match the implementation.
6. Full repository, public-example, Rust documentation, benchmark-harness, and
   website checks pass; independent code review and bilingual copy review have
   no unresolved blocker.
7. One conventional commit is pushed, one PR is merged to refreshed `main`, the
   Pages workflow succeeds, and the live bilingual page plus a built asset are
   fetched from `https://canwu.org`.

## Stop conditions

Redesign before merge if implementation requires global levels, dense state,
mutable live-state access, case-name branching, a new scheduler/RNG, wall-clock
adaptation, dynamic reservations not supported by registered contracts, history
plugins in the kernel graph, source assessments treated as truth, or claims that
automatically become capability or adoption.
