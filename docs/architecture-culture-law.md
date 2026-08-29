# Culture and Legal Institutional Systems

This page defines the architecture for the optional culture authoring layer
and the downstream legal institutionalization extension. Together they model
how a reference content pack can describe cultural change and how an
institution can turn evidence-bearing cultural signals into versioned law,
without adding historical semantics to the Canwu simulation core.

## Boundary and purpose

`canwu-culture` is an authoring and compilation extension above the published
`canwu-society` social diffusion simulation module. It accepts versioned
content, validates cardinality and fan-out budgets, and compiles definitions
into an externally immutable execution plan. `canwu-society` remains the
runtime for sparse population dispositions, social influence, organization
topology, institutional inputs, and actor-relative projections.

The implemented experimental `canwu-law` extension is downstream from cultural signals. It
owns jurisdiction, legal institutions, procedures, enacted rules, effective
dates, amendment, repeal, expiry, and legal interpretation. Political,
election, administration, education, justice, and enforcement extensions own
their respective processes and may consume enacted law.

Neither extension is part of `canwu-core`, `canwu-sim`, or the generic
`canwu-api` contracts. Culture never writes legal state directly, and a
cultural percentage never becomes a statute automatically. Cultural support
is one evidence-bearing input to a bounded institutional procedure; authority,
jurisdiction, capacity, opposition, and the selected decision still control
the legal result.

```text
reference content pack
        |
        v
canwu-culture authoring and compiler
        |
        v
canwu-society sparse social runtime
        |
        +--> CulturalSignalBatch (bounded, causal, next-boundary input)
                    |
                    v
                 canwu-law
             proposal -> procedure -> LawVersion
                    |
                    v
       political / election / administration / education / justice / enforcement adapters
```

The dependency direction is one way: information and correspondence may feed
culture; culture may emit generic signals; law may consume those signals.
The core and public API never depend on legal semantics. Cross-extension
communication uses canonical next-boundary ingress and bounded batches, not a
synchronous event bus or a mutable callback.

## Culture authoring contract

A content pack provides an owned, serializable `CultureDefinition`. A Rust
builder and JSON/TOML loader should share the same validator so that authored
content and generated content have identical rules. The compiler rejects a
definition before a run starts when any cardinality, clause, fan-out, memory,
state, or per-boundary work budget is exceeded.

### Definition components

- **Targets** identify an idea, norm, movement, practice, school, or
  affiliation variant. A target carries neutral profile defaults, ancestry,
  metadata, provenance, and an explicit lifecycle policy.
- **Cohorts** identify aggregate populations with a territory, integer
  headcount, and application-defined classifications such as language,
  occupation, education, or status.
- **Channels** describe exposure or reinforcement opportunities: reach, trust,
  interpretation fidelity, delay, capacity, and policy modifiers.
- **Transition specifications** map named signals to the existing separate
  awareness, private assent, practice, public alignment, organization tie,
  mobilization, and visibility dimensions. They compile to stable rules and do
  not create a second per-person solver.
- **Institution and policy bindings** declare which external decisions can
  change access, support, enforcement, censorship, disruption, or migration
  pressure. They never assign a private-assent percentage directly.
- **Effect bindings** declare a downstream signal kind, scope, cadence,
  persistence class, and required evidence. The culture runtime emits a
  bounded batch; the consumer decides its domain meaning.

Named traits and affinities are allowed at the authoring boundary, but the
runtime compiles them into bounded rule factors or channel signals. It must
not attach an unbounded value map to every population bucket.

### Compiled plan and hot path

`CompiledCulturePlan` is compile-only and externally immutable for one
scenario/run revision. It contains interned numeric IDs, canonical sorted rule
tables, reverse indexes by target, compact channel/transition/effect/institution
tables, scoped keys, lifecycle indexes, declared budgets, and a content hash.
Changing a definition or compiled ordering creates a new semantic plan
revision; it is not an in-place mutation of an existing run.

Settlement is driven by a dirty set of active `(cohort, target)` pairs. An
admitted exposure, policy change, organization change, or reactivation marks
the affected pairs. A transition boundary then:

1. consumes admitted signals in canonical order;
2. evaluates dirty pairs and bounded dependants;
3. updates aggregate counters incrementally;
4. refreshes projections only for observers that can see changed pairs; and
5. emits bounded effect batches for the next eligible consumer boundary.

For `D` active pairs, `Delta` dirty pairs, `B` buckets per pair, `E_delta`
affected edges, and `V_delta` affected observer entries, the intended steady
state cost is approximately `O(Delta * B + E_delta + V_delta)`. A full plan
rebuild is reserved for definition changes, migration, or explicit
maintenance. The current full-state society path remains a compatibility
fallback while incremental aggregate and projection refreshes are introduced.

## Culture lifecycle

Each target has an explicit generation and one of three states:

```text
Active -> Dormant -> Retired
             ^          |
             +----------+
        explicit reactivation creates a new generation
```

### Active and dormant

An `Active` target has engaged population, active propagation,
institutional/policy inputs, or a scheduled reactivation path. Its rules,
distributions, and projections are eligible for ordinary settlement.

A target becomes `Dormant` after a configured quiet window with no engaged
population and no admitted work. Engaged headcount is not the distribution
total: a neutral-only relationship does not keep a target alive. Dormancy
removes the target from culture hot and dirty indexes and, after explicit
society synchronization, stops its compiled culture transition rules. Existing
society distributions and a compact reactivation descriptor remain available.
Dormancy is reversible and does not erase history.

### Retirement and atomic synchronization

After the retention policy, a dormant target is eligible for `Retired` only if
no live transition, organization, institution, policy, effect batch, admitted
input, or scheduled continuation still requires its current generation. The
host evaluates eligibility after all signals admitted for the boundary have
been applied.

Retirement writes a compact `RetiredTargetTombstone` containing target
identity and generation, last active simulation time and revision, retirement
reason and policy hash, any explicit successor reference, and the evidence
references needed for replay and audit. It releases only rebuildable,
target-scoped dynamic society state. Historical domain-record versions,
events, actor knowledge, and archived evidence remain queryable.

`settle_culture_society_boundary` is the preferred combined host helper. It
prepares a bounded runtime delta and stages society changes only when a
lifecycle transition occurs. A live external dependency rejects retirement
before either caller-owned state changes. The host persists the culture record,
society state, and typed lifecycle transition in the same authoritative
transaction. The maintenance-oriented `synchronize_society_lifecycle` path
is reserved for load repair and explicit checkpoints.

New exposure for a retired generation is rejected unless an explicit
reactivation command or ingress is admitted. Reactivation creates a new
generation, initializes only required active relationships, and cites the old
tombstone; it never rewrites old history or silently resurrects every cohort.

## Signal bridge from culture to law

Information and correspondence first resolve access and interpretation, then
may emit a bounded `CultureExposureSignalBatch`. Culture settlement applies
that input and emits a bounded `CulturalSignalBatch` containing target
generation, scope, strength, persistence class, cadence, and evidence. The
batch is an input to law, not an authority grant.

The legal bridge proceeds in fixed, persisted stages:

1. Reverse indexes map signal kind and scope to affected jurisdictions and open
   proposals; unrelated proposals are not scanned.
2. Typed `LegalMutation` ingress and holder-context ingress enter the
   event-driven law plugin. It checks the embedded compiled-plan binding, exact
   aggregate version, host-owned `expected_versions`, cited culture generation,
   and each signal's compiled provider `(plugin, packet_type)` plus the
   kernel-committed producing boundary before advancing only dirty or due
   proceedings. Direct host injection into the provider namespace is not
   evidence. It never trusts a caller-declared signal kind. Publicity events
   additionally bind the retained provider payload to the exact proposal,
   occurrence time, medium, and scope; generic practice signals remain
   identity-and-boundary evidence.
3. A proceeding creates a holder-bound decision outbox item. The adapter first
   persists its expected revision, registers each exact seat controller at most
   once, then submits ticket-open requests. Later tickets reuse that controller.
   ACK is accepted only after the required decision outcomes are `Accepted` and
   the current controller/ticket exactly match the persisted draft. This proof
   survives ingress archival. Format 7 keeps schema-declared identity-only
   receipts for unresolved proceedings and live law sources; generated ingress
   receipts Merkle-bind the provider plugin, packet type, and producing
   boundary, so verification does not hydrate old payloads.
4. An authorized controller selects an existing option. The accepted command
   can only schedule a bounded pending legal intent; it cannot write law.
5. A later law-plugin boundary revalidates jurisdiction, competence, procedure,
   revision and effective-time guards, clause and evidence limits, and the
   cited culture generation, then atomically compare-and-sets the aggregate.
   Election, administration, education, justice, and enforcement adapters
   consume the enacted result at their own declared boundary.

The authoritative path is therefore:

```text
CulturalSignalBatch
  -> LegalProposal / DecisionTicket
  -> authorized DecisionAttempt / DecisionTrace
  -> accepted legal command -> pending intent
  -> atomic law aggregate compare-and-set
  -> LawVersion
  -> downstream enforcement and feedback evidence
```

No synchronous callback is permitted. A consumer that cannot accept a batch
records a rejection or defers it; it does not partially mutate culture or law.

## Legal records and procedure

`LegalJurisdiction` binds a stable jurisdiction to its parent, territory scope,
competent institutions, and procedure profile. `LegalInstitution` binds an
institution entity to a jurisdiction, authority seats, quorum, vote or
appointment rules, and the command subject allowed to adopt law. Both are
members of the one typed legal aggregate, not new core entity kinds or
independently mutable host records.

Compilation requires each procedure seat to resolve to exactly one institution
that declares both that procedure and seat. The holder, permission profile, and
length-prefixed controller identity are frozen into the plan; missing, ambiguous,
or collision-prone authority definitions fail before a run starts.

`LegalProposal` is a non-enacted, versioned proceeding input. It records the
jurisdiction, sponsor, subject references, bounded typed clauses or eligibility
rules, required procedure and deadline, status (`draft`, `submitted`,
`deliberating`, `adopted`, `rejected`, `expired`, or `withdrawn`), source signal
and other evidence references, the open ticket identity and option version, and
its claimed legal competence, defects, validity, and exact origin. Kernel
authorization only proves who submitted the command; it does not make an
in-world ultra vires act valid. A cultural target generation may be cited as
evidence but never gains permission to mutate or enact the proposal.

Compiled institutional competence is default-deny across legal order,
jurisdiction, subject matter, source mode, operation, procedure, forum, and
adjudicative power. Each source profile separately declares procedural or
evidence-claim authority, exact origin policy, publicity policy and compiled
publicity signal provider, evidence
bounds, claimant rules, and retrospective permission.

An accepted proposal creates one immutable `LegalSourceVersion`, the stable
`LegalRule`, and one immutable `LawVersion`. The source keeps the exact proposal
and ruling, agreement, or reception origin; its mode is explicitly
`Promulgated`, `Adjudicated`, `Accreted`, `Agreed`, or `Received`. The rule owns
the latest claim separately from the operative version. A `Purported` or
`Contested` change can therefore remain visible while the prior valid version
continues to govern.
Publication is a separate immutable event with exact proposal, time, medium,
scope, and evidence. A validity condition rejects adoption until the event
exists. An effectiveness condition may accept the proposal first, but the new
version stays inert and is excluded from every historical read cut until the
event exists; publication must occur no later than the effective time. Delayed
publication updates the proposal lifecycle and derived rule head without
rewriting the create-only source or law version. Backdated effect additionally
requires both profile permission and an explicit retrospective date.

`LawVersion` records a stable rule ID and monotonic legal ordinal, jurisdiction
and bounded scope, effective simulation times, source and origin, causal
evidence, and explicit predecessor links. Each compiled clause declares its
normative modality rather than deriving it from display text. Rights and
eligibilities include holders, duty bearers, subject matter, conditions,
standing, forum, and remedy profile. Amendment and repeal are new legal
commands and append-only versions. An enacted law does not disappear when its
cultural target retires.

Applicability is a typed, bounded query over an exact legal order and compiled
profile. It filters time, territory, persons, subject matter, and jurisdiction;
applies evidence-bound condition/exception predicates and precedence; and
returns the governing versions, displaced claims, conflicts, and a trace.
Missing predicate facts return `Indeterminate`; false conditions or true
exceptions return `NotApplicable`. Actor-relative queries bind an exact
knowledge read cut plus one holder record per fact. Host-bound queries verify
the holder, complete cut, compiled knowledge schema, JSON boolean pointer,
asserted value, and evidence; detached actor-relative queries are rejected.
Unresolved validity returns `Contested`, including both
the prior operative version and the rival claim. Succession does not inherit a
predecessor order by default: reception uses the longest matching rule prefix
within the succession's declared personal and territorial scope, then applies
the received rule's own subject-matter scope. `Continue`
exposes the predecessor directly, while `Transform` and `Review` require an
explicit `Receive` source with the exact succession and predecessor origin.

Jurisdiction reachability uses compiled relation-kind/direction adjacency and
one bounded reachable-set build per query. A total work budget also covers graph
edges, rule/version and nested effect/predicate visits, conflict fan-out, and
conflict members. A separate per-record nested-item budget bounds proposals,
cases, rulings, and conflict partitions before traversal. A resolved conflict records exact
total, governing, and displaced version sets plus a typed basis and rationale;
non-temporal resolution requires an operative, case-bound competent ruling with matching
version sets and an explicit covered jurisdiction. Overlapping active partitions
are merged simultaneously and contradictory governing/displaced sets remain
`Contested`. The complete claim set, jurisdiction, read time, and effective
interval must match before its partition can promote or displace a claim.
The query consumes that partition instead of guessing a winner. Cases, findings,
and rulings are checked against compiled forum, proof, standing, remedy,
precedent, interval, issue, and adjudicative-competence contracts.

The culture effect persistence class determines how law may interpret a signal:

| Culture effect | Legal interpretation |
| --- | --- |
| `Pulse` | Opens or updates a proposal opportunity; no durable law exists. |
| `Level` | Supplies current support or legitimacy pressure; its end may trigger law review, never automatic repeal. |
| `Commitment` | Once accepted by a legal command, provides provenance for a durable `LawVersion`; culture retirement does not retract it. |
| `Evidence` | Historical citation only; it cannot open or mutate a proceeding directly. |

If a legal rule intentionally depends on a live cultural level, the law
extension records that dependency and owns its review, expiry, or renewal rule.
The culture runtime emits the end of the level; it does not silently repeal
the law. A commitment already accepted into law does not keep the cultural
target hot, so a target can retire while its law remains active.
The active law keeps only a compact adoption-evidence receipt. The retired
culture target and its propagation indexes leave the hot path, while repeal or
expiry remains an explicit legal operation.

Retirement runs as explicit bounded maintenance. In v1,
`max_retirement_dependency_records` caps the outer records in the aggregate
dependency proof; each record's dependency fan-out is separately bounded. An
over-budget request fails before either culture or law state changes. Ordinary
legal settlement does not pay this scan. A later sharded runtime should use a
target-keyed dependency index.

## Authority, visibility, and persistence

Legal commands use the ordinary authority chain:

```text
legal facts -> DecisionTicket -> controller selection
  -> DecisionAttempt / DecisionTrace
  -> canonical command ingress
  -> legal authority and procedure validation
  -> LawVersion commit
```

The command subject is the proposal's exact frozen subject; separate compiled
competence determines whether its institution has in-world legal power. The context carries
the validated controller ID, decision origin, seat, permission profile,
request identity, and expected revision/time guards. An external service or
model may recommend an option but cannot manufacture authority or submit a raw
law payload that bypasses the ticket.

Public enacted law may be exposed through a domain projection. Private drafts,
dissent, and actor-specific knowledge use `ViewerContext` and the holder ledger;
there is no truth fallback. Culture and law records implement `DomainRecordType`
with strict schemas, typed references, explicit mutation policies, and retained
version bodies where evidence needs exact historical meaning.

The one persisted `LegalRuntimeRecord` includes the compiled plan, plan and
content hashes, budgets, lifecycle state, target generations, procedure
profiles, records, and all derived indexes. Load recomputes and validates the
indexes before exposing state. Exact replay consumes recorded mutation,
signal, decision, command, ACK, and wake ingress plus boundary evidence. It never
reruns a human, service, or model policy. Forks copy the validated state and
continue with new causal inputs. Failed boundaries restore indexes, tombstones,
counters, evidence, and random positions atomically.

Complete history and derived-index validation is a cold load/restore operation.
The live plugin checks immutable plan and budget bindings plus local mutation
guards; it does not repeat a full history scan for every mutation. In
particular, the identity-only evidence dependency declaration and its reference
counts are updated only for affected topology owners and fully reconstructed at
cold validation. A mismatched set or count index is rejected. Evidence sealing
is atomic even when a late dependency check fails. The latest live disputed
claim retains its identity evidence until a replacement claim supersedes it.

## Bounded work and conformance

The legal plan compiles numeric IDs for jurisdictions, institutions, proposals,
clauses, and procedure profiles; reverse indexes by signal kind and scope;
dirty proposal and jurisdiction sets; per-procedure limits for clauses,
evidence, options, fan-out, and pending continuations; and a plan hash and
budget manifest. For `P_delta` dirty proposals, `C_delta` affected clauses,
and `V_delta` observer entries, intended steady-state work is approximately
`O(P_delta + C_delta + V_delta)`. Deadline and effective-time wakes use ordered
indexes, exact decision-result proof uses a request index, and both deleted and
inserted applicability rows consume the mutation budget. Retired targets and
historical law catalogs must not increase active proposal settlement cost.

That bound describes post-decode legal settlement, not the whole transaction.
Because v1 persists one aggregate and the kernel clones transaction state, a live
legal boundary is approximately `O(H_serialized + delta)`. Release measurements
for 1k/10k/100k retained records were 37.472ms/507.535ms/6.017504s median for the real
plugin path, while law-local idle settlement stayed at 200ns. Large hot legal
histories therefore require budgets and event-driven cadence. The 1k result is
already above both 60 FPS and 30 FPS frame budgets; 10k is low-frequency turn or
background work, and 100k is offline/maintenance work. Boundary checkpoints now
share the generic domain-record map root in O(1), but the first domain-record
write still copies the whole map, decision state is still cloned as one value,
and `canwu-law` still persists one aggregate. A private test-only scaffold checks
the proposed legal archive state machine, but it is not a public API, save
format, or replay path. Until jurisdiction shards, content-addressed page
deltas, decision-history placement, and provider-verified archive ingress
replace those paths together, manifests should cap live retained records well
below 1k and calibrate the exact cap on target hardware.

The proposed format-8 scale milestone is specified in
[Legal storage sharding, COW, delta persistence, and cold archive](proposals/legal-storage-sharding-compaction.md).
It combines legal-order/jurisdiction hot shards with kernel COW stores,
content-addressed checkpoint deltas, and staged fail-closed cold archives. A
kernel owner-authorized coordinator lets culture and law plugins update only
their own records while committing retirement dependency changes atomically.
The design keeps current enacted effects hot while moving closed history out of
ordinary settlement; archiving and cultural retirement do not repeal law.

Conformance evidence should prove that:

- cultural signals cannot directly mutate legal records;
- only a controller-bound authorized command can enact, amend, or repeal law;
- stale proposal, ticket, and law revisions become safe, persisted rejections;
- a commitment accepted into law survives cultural retirement;
- a live level dependency blocks retirement until law resolves it;
- a future-effective live-level dependency also blocks early retirement;
- expired procedures expire unresolved pending/enqueued outbox work;
- proposal, law, evidence, archive, snapshot, fork, and exact replay remain
  consistent; and
- unrelated targets, observers, and retired catalog entries do not perturb
  keyed results or declared work budgets.

The first content examples should remain downstream data. For example, a
women's suffrage pack may emit public-alignment and legitimacy signals; a
competent assembly selects a voting-eligibility option through a ticket; the
legal command commits a `VotingEligibilityRule` with an effective date; and
the election adapter reads it. Retiring the cultural target later stops new
propagation while the enacted rule and enforcement history remain intact.
