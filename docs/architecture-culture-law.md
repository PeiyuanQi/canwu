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
canwu-culture authoring, compiler, and lifecycle
        |
        +--> canwu-society sparse social runtime
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
`canwu-culture` depends on `canwu-api` and `canwu-society`. Among Canwu crates,
`canwu-law` depends only on `canwu-api`: it receives cultural signals as ingress and has no
compile-time dependency on `canwu-culture`. The core and public API never
depend on legal semantics. Cross-extension communication uses canonical
next-boundary ingress and bounded batches, not a synchronous event bus or a
mutable callback.

## Culture authoring contract

A content pack provides an owned, serializable `CultureDefinition`, built with
`CultureDefinition::builder(id)` or deserialized with serde, for example from
JSON produced by a content tool. `CultureDefinitionBuilder::build` and
`compile_culture` run the same validator, so authored content and generated
content follow identical rules. The compiler rejects a definition before a run
starts when it exceeds its `CultureBudgets`; the budgets cap component counts,
fan-out, signals per batch, evidence per signal, tombstones, text length,
persisted state size, and memory.

### Definition components

- **Targets** (`CultureTargetDefinition`) identify an idea, norm, movement,
  practice, school, or affiliation variant. A target carries an optional parent
  target, a neutral disposition profile, and metadata.
- **Cohorts** (`CultureCohortDefinition`) identify aggregate populations with a
  territory, integer headcount, and application-defined classifications such as
  language, occupation, education, or status.
- **Channels** (`ChannelSpec`) describe an exposure path for one target from an
  optional source cohort to a target cohort: reach, trust, interpretation
  fidelity, delay in boundaries, and capacity.
- **Transition specifications** (`TransitionSpec`) move one target's affected
  cohorts between two disposition profiles at a base rate per million, with
  weights. A profile places a cohort on the separate awareness, assent,
  practice, public alignment, organizational tie, mobilization, and visibility
  dimensions of `canwu-society`. Transitions compile to stable rules and do not
  create a second per-person solver.
- **Institution bindings** (`InstitutionBinding`) name an institution entity
  whose decisions affect one target and a set of cohorts. Institutions act on
  cohorts through `canwu-society` policy pressure (`PolicyPressure`): support,
  legal access, surveillance, censorship, coercion, material penalty,
  disruption, and migration pressure. No policy field assigns private assent
  directly.
- **Effect bindings** (`CulturalEffectBinding`) declare a downstream signal
  kind, scope, cadence in boundaries, persistence class, and whether evidence is
  required. The culture runtime emits a bounded batch; the consumer decides its
  domain meaning.

The definition also carries its budgets and one `RetirementPolicy`, which sets
how many quiet boundaries pass before a target goes dormant and before it may
retire. It has no free-form trait or value map per population bucket: traits
and affinities are expressed through transition weights and channel settings,
so the runtime stays bounded.

### Compiled plan and hot path

`CompiledCulturePlan` is compile-only and externally immutable for one
scenario/run revision. It contains dense numeric keys (`TargetKey`,
`CohortKey`, and so on), canonical sorted rule tables, reverse indexes by
target, compact channel/transition/effect/institution tables, declared budgets,
the retirement policy, and a content hash. Per-target lifecycle indexes (hot
targets, dormancy due times, the dirty set, and effect emission cursors) live
in the separate `CultureState`.
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
input, or scheduled continuation still requires its current generation.
Eligibility is evaluated after all signals admitted for the boundary have been
applied.

Retirement writes a compact `RetiredTargetTombstone` containing target identity
and generation, the last active time, the retirement time (`retired_at`), the
retirement reason and policy hash, any explicit successor reference, and the
evidence references needed for replay and audit. It releases only rebuildable,
target-scoped dynamic society state. Historical domain-record versions, events,
actor knowledge, and archived evidence remain queryable.

With the host-driven `CulturePlugin`, `settle_culture_society_boundary` is the
preferred combined host helper. It
prepares a bounded runtime delta and stages society changes only when a
lifecycle transition occurs. A live external dependency rejects retirement
before either caller-owned state changes. The host persists the culture record,
society state, and typed lifecycle transition in the same boundary. The
maintenance-oriented `synchronize_society_lifecycle` path is reserved for load
repair and explicit checkpoints.

New exposure for a retired generation is rejected unless an explicit
reactivation command or ingress is admitted. Reactivation creates a new
generation, initializes only required active relationships, and cites the old
tombstone; it never rewrites old history or silently resurrects every cohort.

## In-engine settlement with the culture boundary plugin

A run can let the engine settle the culture lifecycle. It registers the
culture boundary plugin, `CultureBoundaryPlugin`, beside
`canwu_society::SocietyPlugin` instead of `CulturePlugin`. The two flows are
alternatives: a run that uses the boundary plugin must not also call
`settle_culture_society_boundary`, and `CulturePlugin` with its host-driven flow
is unchanged.

1. The scenario installs the culture definition record built with
   `culture_definition_record`, the culture state record, and a society state
   prepared with `install_into_society`. Boundary handlers are plain function
   pointers, so the plugin recompiles the definition record at every lifecycle
   boundary instead of holding a compiled plan.
2. Information or correspondence providers submit resolved exposure as public
   `culture_exposure_v1` ingress. Each `CultureExposureSignalBatch` names the
   target ID and generation, the cohort scope, fidelity, evidence, and the
   earliest eligible boundary. An event-driven phase-12 intake,
   `culture_exposure_intake_v1`, queues admitted batches in the
   `canwu.culture:exposure-queue` record; a batch for another generation is
   rejected with a `culture_exposure_rejected_v1` event.
3. The Monthly phase-7 system `culture_lifecycle_settle_v1` consumes the queue
   and accepted institutional decisions on culture alignments, derives
   engagement and live dependencies from the society snapshot, calls
   `settle_culture_society_boundary` on that snapshot, and persists
   `canwu.culture:state`. A target an institution has decided on remains a live
   dependency, so it does not go dormant or retire.
4. For each transitioned target, the plugin schedules one internal
   `society_lifecycle_delta_v1` packet (`SocietyLifecycleDeltaV1`, built by
   `society_lifecycle_delta`). The society plugin queues it in phase 12 and
   applies it at its next Daily settlement, so it remains the only writer of
   `canwu.society:state`. A delta the society refuses is recorded and not
   applied. At each later Monthly settlement the plugin compares society state
   with the committed culture lifecycle and re-sends any delta that is still
   missing; society applies deltas idempotently.
5. Each due compiled effect becomes a self-addressed `cultural_signal_batch_v1`
   ingress, which consumers such as the law plugin admit at the next boundary
   and verify by its producer.

A rejected lifecycle step emits `culture_lifecycle_rejected_v1` and leaves
culture state unchanged, each settled transition emits
`culture_lifecycle_transition_v1`, and a retired culture state is inert.
Because all phase-7 systems read the same boundary snapshot, every hand-off is
next-boundary, and a culture step reaches society state up to two boundaries
later.

## Signal bridge from culture to law

Information and correspondence first resolve access and interpretation, then
may emit a bounded `CultureExposureSignalBatch`, the payload of the
`culture_exposure_v1` ingress. Culture settlement applies that input, and the
`canwu-culture` lifecycle emits bounded `CulturalSignalBatch` values from its
compiled effect bindings (the boundary plugin emits one batch per due effect).
Each `CulturalSignal` in a batch carries the effect ID, target ID and
generation, signal kind, persistence class, scope, strength, emission time, and
evidence; the emission cadence stays on the effect binding. The batch is an
input to law, not an authority grant.

The legal bridge proceeds in fixed, persisted stages:

1. Reverse indexes map signal kind and scope to affected jurisdictions and open
   proposals; unrelated proposals are not scanned.
2. Typed `LegalMutation` ingress and holder-context ingress enter the
   event-driven law plugin. It checks the embedded compiled-plan binding, exact
   directory and shard versions, host-owned `expected_versions`, cited culture generation,
   and each signal's compiled provider `(plugin, packet_type)` plus the
   kernel-committed producing boundary before advancing only dirty or due
   proceedings. Direct host injection into the provider namespace is not
   evidence. It never trusts a caller-declared signal kind. Publicity events
   additionally bind the retained provider payload to the exact proposal,
   occurrence time, medium, and scope; generic practice signals remain
   identity-and-boundary evidence.
3. A proceeding creates a holder-bound decision outbox item. The adapter first
   persists its expected revision (`prepare_pending_decision_enqueues`),
   registers each exact seat controller at most once, then submits ticket-open
   requests (`enqueue_pending_decisions`) and queues the acknowledgement
   (`acknowledge_enqueued_decisions`). Later tickets reuse that controller.
   ACK is accepted only after the required decision outcomes are `Accepted` and
   the current controller/ticket exactly match the persisted draft. This proof
   survives ingress archival. Format 8 keeps schema-declared identity-only
   receipts for unresolved proceedings and live law sources; generated ingress
   receipts Merkle-bind the provider plugin, packet type, and producing
   boundary, so verification does not hydrate old payloads.
4. An authorized controller selects an existing option. When no controller
   exists for a seat, `canwu-law` registers a default `Human` controller; a host
   may instead pre-register the same stable controller ID with a `Utility`,
   `Rule`, `Random`, `External`, or `Llm` policy, which the law plugin keeps
   while its authority, seat, permission profile, and command subject match the
   compiled plan. The accepted command can only schedule a bounded pending
   legal intent; it cannot write law.
5. A later law-plugin boundary revalidates jurisdiction, competence, procedure,
   revision and effective-time guards, clause and evidence limits, and the
   cited culture generation, then atomically commits the owner-scoped shard
   mutation bundle.
   Election, administration, education, justice, and enforcement adapters
   consume the enacted result at their own declared boundary.

The authoritative path is therefore:

```text
CulturalSignalBatch
  -> LegalProposal / DecisionTicket
  -> authorized DecisionAttempt / DecisionTrace
  -> accepted legal command -> pending intent
  -> atomic legal shard bundle commit
  -> LawVersion
  -> downstream enforcement and feedback evidence
```

No synchronous callback is permitted. A consumer that cannot accept a batch
records a rejection or defers it; it does not partially mutate culture or law.

## Legal records and procedure

A `LegalDefinition` declares legal orders, jurisdictions, institutions,
procedures, clauses, source profiles, signal providers, applicability profiles,
predicates, forums, and precedence profiles; `compile_law` turns it into a
hashed, budgeted plan. `LegalJurisdictionDefinition` gives a jurisdiction a
stable ID, metadata, and typed relations to other jurisdictions (delegation,
territorial containment, supremacy, appeal, treaty membership, or overlap).
`LegalInstitutionDefinition` binds an institution to an optional organization
entity, its jurisdictions, its authority seats (each with an optional holder
and a permission profile), its procedures, and its competences. Quorum,
threshold, and voting rules belong to procedure stages. Jurisdictions and
institutions are part of the compiled legal plan, not new core entity kinds or
independently mutable host records.

Compilation requires each procedure seat to resolve to exactly one institution
that declares both that procedure and seat. The holder, permission profile, and
length-prefixed controller identity are frozen into the plan; missing, ambiguous,
or collision-prone authority definitions fail before a run starts.

### Weighted, unit-block, and consultation stages

A procedure stage can weigh seats and count unit blocks, and a procedure can
include an advisory consultation. `ProcedureStageDefinition` carries
`seat_weights`, `block_of_seat`, and `block_threshold`, and
`ProcedureStageKind::Consultation` marks an advisory stage.

- **Vote weight.** A seat weighs its `seat_weights` entry, or 1 when absent, so
  an empty map is the equal-seat count; an explicit weight of 1 is removed when
  the plan compiles. `quorum` is the minimum summed weight of seats that cast
  any ballot, abstentions included, and `threshold` is the per-mille share of
  `For` weight among `For` plus `Against` (`For * 1000 >= (For + Against) *
  threshold`, with a positive sum). Without weights both rules reduce to plain
  seat counts. Vetoes are seat powers and are never weighted.
- **Unit blocks.** `block_of_seat` assigns every seat of a stage to exactly one
  unit block, and `block_threshold`, from 1 to the number of blocks, says how
  many blocks must be `For`. A block takes the weighted-majority position of its
  seats; an evenly divided block, or one without `For` or `Against` ballots,
  takes none. A blocked stage passes only when its weighted seat rule holds and
  enough blocks are `For`.
- **Tie-breaks.** When as many blocks are `For` as `Against`, the procedure's
  `deterministic_tie_break` applies; it has runtime meaning only for
  procedures with a blocked stage. `status-quo` (`PROCEDURE_TIE_BREAK_STATUS_QUO`)
  adds no block, so a tied stage passes only if `block_threshold` is already
  met, and otherwise waits for more ballots or its deadline.
  `casting-seat:<seat>` (`PROCEDURE_TIE_BREAK_CASTING_SEAT_PREFIX`) adds one
  `For` block when that seat's own ballot in the stage is `For`. The casting
  seat must sit in every blocked stage, and each block threshold must exceed
  half the blocks, so a tie never passes without it. Stages without blocks have
  no tie-break; use a threshold of 501 when an even split must fail.
- **Consultation.** A `Consultation` stage is advisory. Its seats receive
  tickets whose context carries `"advisory": true`, and their ballots persist
  as participation records but never count toward completion, veto, or
  adoption. The stage needs a positive `deadline_minutes` and no quorum,
  threshold, weights, or blocks, and it cannot be a procedure's last stage. It
  completes at the first legal boundary after its deadline, expires unanswered
  seat work, and opens the next stage; zero ballots is a valid outcome. A
  procedure that still lacks its capacity reservation at that deadline expires
  instead.

The compiler checks that weighted and blocked seats belong to the stage, that
weights are at least 1, that every seat of a blocked stage sits in exactly one
block, that the block threshold lies between 1 and the block count, and that
the quorum does not exceed the total weight. Unused fields are omitted from
JSON, so existing plans keep their encoding and content hash. Block counting
lives in `canwu-law`, not in a shared ballot helper.

A stage that stops accepting ballots (it passed, completed, or its procedure
closed) expires its pending and enqueued seat work, and a late preparation or
acknowledgement for expired work is ignored. A seat response
that still arrives is recorded as a rejected intent outcome instead of failing
the plugin boundary, and the pre-settlement budget check counts ticket work
emitted at the exact deadline minute.

`LegalProposal` is a non-enacted, versioned proceeding input. It records the
proposal ID, sponsor, legal order, jurisdictions, subject references, cultural
dependencies, bounded typed clause operations, source and procedure profiles,
deadline and effective time, the `LawOperation` and target rule, status
(`draft`, `submitted`, `deliberating`, `adopted`, `rejected`, `expired`, or
`withdrawn`), evidence references, host-owned `expected_versions`, and its
claimed legal competence, defects, validity, and exact origin. It holds no
decision ticket or option version; seat tickets are tracked by the decision
outbox. Kernel authorization only proves who submitted the command; it does not
make an in-world ultra vires act valid. A cultural target generation may be
cited as evidence but never gains permission to mutate or enact the proposal.

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

Retirement runs as explicit bounded maintenance. Format 8 keeps target-keyed
culture-dependency records and requires the culture owner plus every registered
dependency resolver to contribute an owner-scoped proposal. The kernel
preflights the complete set against one persistent domain root and commits all
participants or none. Missing proposals, cross-owner writes, stale versions,
and budget overflow fail before either culture or law state changes; ordinary
legal settlement never scans total legal history to prove retirement safety.

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

Format 8 persists independently versioned plan, directory, order/jurisdiction
shard, coordinator, culture-dependency, and archive-head records. Load admits
only the declared working set and reconstructs derived indexes, authenticated
archive roots, and hot projections before exposing state. Exact replay consumes
recorded mutation, signal, decision, command, ACK, and wake ingress plus
boundary evidence. It never
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
After a historical legal payload is released, the hot runtime retains only an
exact archived-dependency identity that a current projection still names. It
rebuilds that bounded set from current hot references after every release.
Succession similarly moves its full institutional, liability, evidence, and
archive history cold while retaining the current reception mapping required by
no-provider applicability. Current enacted effects therefore remain directly
queryable after culture, procedure, case, conflict, publicity, ruling, source,
version, decision, or succession history leaves the hot path.

Archive preparation reads a persisted per-shard ordered candidate queue and an
incremental count/XOR/modular-sum source authenticator. It examines and
materializes only the selected batch; full catalog reconciliation is restricted
to explicit full cold restore or migration. Temporal index cells use ordered
authenticated page segments, so dense same-time history remains within the
64-entry/1-MiB page limits. Verified legal maintenance ingress retains the
exact blob, directory, membership-page, and temporal-page IDs while pending;
snapshot restore preserves those marks, and an applied or stale terminal
boundary transfers or releases them before offline GC.

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

That bound describes shard-local semantic work. Format 8 does not persist
the law plugin as one aggregate record: plan, directory, order, jurisdiction,
coordinator, culture-dependency, and archive-head records are loaded as a
declared working set and committed in one kernel mutation bundle. Domain and
decision maps use persistent structural sharing, and content-addressed
checkpoint generation emits only missing Patricia paths, changed decision
segments, and bounded manifests. Decision locators use 4,096 primary hash
buckets with 16 deterministic subsegments per bucket; each segment is capped at
64 entries and 1 MiB. Archive GC begins from one automatically plugin-extended
reachability manifest, and the persisted retention ledger verifies transitive
page closure and shared-page protection across restart. Boundary proposal
validation layers its small record overlay onto the persistent root and checks
only the affected reference closure; empty stages return without materializing
untouched domain records.

The retention ledger does not preserve a full closure for every completed
archive transaction. While ingress is pending, the previous committed root
protects older objects and the handle protects only the new object delta plus
the proposed current-page closure. Commit moves the prior object set into the
new root, retires the superseded directory root, and clears the terminal
handle's reachability payload. Legal effects and historical versions remain
reachable through the new root; only redundant root and transaction state is
retired.

Each canonical archive result is persisted by retention handle until the host
finishes the store-side handoff. Finalization reloads the authoritative legal
runtime, derives the recorded disposition, performs the idempotent store
transition, and queues a private acknowledgement that removes that handle's
recovery record. The synchronous helper separately authenticates the complete
store-bound archive head and applies on a clone, so a malformed root or failed
store transition cannot release hot payloads. Paged decision restore follows
the same bounded principle: it loads only locator pages for archived tickets or
traces still referenced by hot decision objects before validation.

The old fully resident aggregate curve remains useful regression evidence:
1k/10k/100k hot historical records measured about 40.6ms/541ms/7.1s median on
the 2026-08-30 implementation workstation. The supported cold placement keeps
closed payloads behind compact archive roots instead. With 100k and 1M root-only
cold records, 50 ordinary dirty-shard boundaries measured 6.558ms/11.302ms and
6.920ms/11.733ms median/p95 respectively. Cold history therefore did not enter
the boundary curve. The million-record authenticated index uses full-width
16-bit buckets; no membership or temporal page exceeded 37 entries or 37,335
canonical bytes, against hard limits of 64 entries and 1 MiB. See
[the Format 8 scale artifact](benchmarks/format8-2026-08-30.md). A separate
selector stress probe held one million compaction candidates while examining
and materializing exactly the 4,096-record batch. That fixture's 2.49-GB shard
is deliberately outside the live 128-MiB legal state/memory contract. The real
Canwu ingress probe instead used an admissible 16,384-candidate, 38.40-MiB
persisted shard: the empty boundary measured 0.169ms, the unrelated legal
boundary 2.563ms, and the candidate record version did not change.

The implemented format-8 scale milestone is specified in
[Legal storage sharding, COW, delta persistence, and cold archive](proposals/legal-storage-sharding-compaction.md).
It combines legal-order/jurisdiction hot shards with kernel COW stores,
content-addressed checkpoint deltas, and staged fail-closed cold archives. A
kernel owner-authorized coordinator lets culture and law plugins update only
their own records while committing retirement dependency changes atomically.
The implementation keeps current enacted effects hot while moving closed history out of
ordinary settlement; archiving and cultural retirement do not repeal law.

Conformance evidence should prove that:

- cultural signals cannot directly mutate legal records;
- only a controller-bound authorized command can enact, amend, or repeal law;
- stale proposal, ticket, and law revisions become safe, persisted rejections;
- a commitment accepted into law survives cultural retirement;
- a live level dependency blocks retirement until law resolves it;
- a future-effective live-level dependency also blocks early retirement;
- expired procedures expire unresolved pending/enqueued outbox work;
- a stage that stops accepting ballots expires its seat work, and a late seat
  response becomes a rejected outcome;
- with the culture boundary plugin, the society plugin stays the only society
  writer and the run replays exactly;
- proposal, law, evidence, archive, snapshot, fork, and exact replay remain
  consistent; and
- unrelated targets, observers, and retired catalog entries do not perturb
  keyed results or declared work budgets.

The first content examples should remain downstream data. For example, a
women's suffrage pack may emit public-alignment and legitimacy signals; a
competent assembly selects a voting-eligibility option through a ticket; the
legal command commits a voting-eligibility rule with an effective date; and
the election adapter reads it. Retiring the cultural target later stops new
propagation while the enacted rule and enforcement history remain intact.
