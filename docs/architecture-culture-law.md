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

The proposed `canwu-law` extension is downstream from cultural signals. It
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
             proposed canwu-law
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

The legal bridge proceeds in fixed stages:

1. Reverse indexes map signal kind and scope to affected jurisdictions and open
   proposals; unrelated proposals are not scanned.
2. The law extension evaluates dirty proceedings and creates or refreshes a
   `DecisionTicket` with options such as adopt, amend, reject, defer, or refer.
   The ticket contains actor-relative facts and evidence, not mutable runtime
   state.
3. An authorized controller selects an existing option. The decision attempt
   and `DecisionTrace` are persisted, and the selected option enqueues a
   validated legal command through canonical ingress.
4. The command validates jurisdiction, institution authority, procedure,
   revision and effective-time guards, clause and evidence limits, and the
   cited signal generation.
5. A successful command atomically creates or updates a `LegalProposal` and a
   `LawVersion`. Election, administration, education, justice, and enforcement
   adapters consume the enacted result at their own declared boundary.

The authoritative path is therefore:

```text
CulturalSignalBatch
  -> LegalProposal / DecisionTicket
  -> authorized DecisionAttempt / DecisionTrace
  -> validated legal command ingress
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
domain records, not new core entity kinds.

`LegalProposal` is a non-enacted, versioned proceeding input. It records the
jurisdiction, sponsor, subject references, bounded typed clauses or eligibility
rules, required procedure and deadline, status (`draft`, `submitted`,
`deliberating`, `adopted`, `rejected`, `expired`, or `withdrawn`), source signal
and other evidence references, and the open ticket identity and option
version. A cultural target generation may be cited as evidence but never gains
permission to mutate or enact the proposal.

`LawVersion` is the authoritative legal result. It records a stable law ID and
monotonic version, jurisdiction and bounded scope, typed clauses, effective
simulation times, adopting institution, authorized decision origin, procedure
result, causal/evidence references, and explicit amendment, successor, repeal,
or expiry links. Amendment and repeal are new legal commands and append-only
versioned results. An enacted law does not disappear when its cultural target
retires.

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

## Authority, visibility, and persistence

Legal commands use the ordinary authority chain:

```text
legal facts -> DecisionTicket -> controller selection
  -> DecisionAttempt / DecisionTrace
  -> canonical command ingress
  -> legal authority and procedure validation
  -> LawVersion commit
```

The command subject is the competent legal institution. Its context carries
the validated controller ID, decision origin, seat, permission profile,
request identity, and expected revision/time guards. An external service or
model may recommend an option but cannot manufacture authority or submit a raw
law payload that bypasses the ticket.

Public enacted law may be exposed through a domain projection. Private drafts,
dissent, and actor-specific knowledge use `ViewerContext` and the holder ledger;
there is no truth fallback. Culture and law records implement `DomainRecordType`
with strict schemas, typed references, explicit mutation policies, and retained
version bodies where evidence needs exact historical meaning.

The persisted plugin semantic environment includes plan hashes, budgets,
lifecycle policies, target generations, legal procedure profiles, and law
schema versions. Exact replay consumes recorded signal ingress, decision
ingress, legal command ingress, boundary records, and evidence. It never
reruns a human, service, or model policy. Forks copy the validated state and
continue with new causal inputs. Failed boundaries restore indexes, tombstones,
counters, evidence, and random positions atomically.

## Bounded work and conformance

The legal plan compiles numeric IDs for jurisdictions, institutions, proposals,
clauses, and procedure profiles; reverse indexes by signal kind and scope;
dirty proposal and jurisdiction sets; per-procedure limits for clauses,
evidence, options, fan-out, and pending continuations; and a plan hash and
budget manifest. For `P_delta` dirty proposals, `C_delta` affected clauses,
and `V_delta` observer entries, intended steady-state work is approximately
`O(P_delta + C_delta + V_delta)`. Retired targets and historical law catalogs
must not increase active proposal settlement cost.

Conformance evidence should prove that:

- cultural signals cannot directly mutate legal records;
- only a controller-bound authorized command can enact, amend, or repeal law;
- stale proposal, ticket, and law revisions become safe, persisted rejections;
- a commitment accepted into law survives cultural retirement;
- a live level dependency blocks retirement until law resolves it;
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
