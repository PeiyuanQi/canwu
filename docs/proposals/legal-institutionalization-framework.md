# Legal Institutionalization Framework

Status: implemented experimental downstream extension in `canwu-law`.
This document defines the path from social
evidence to legal state without adding legal semantics to Canwu core, granting
culture write authority, or bypassing the current plugin and decision APIs.

## Decision

Add an optional `canwu-law` domain extension. It consumes bounded,
evidence-bearing cultural and other social inputs, advances persisted legal
proceedings, and exposes actor-relative decision outbox items. A deterministic
host adapter turns those outbox items into ordinary `DecisionIngressRequest`
values. A selected legal option may issue an authorized plugin command, but the
command writes only a bounded pending legal intent. A later canonical law-plugin
boundary revalidates that intent and atomically compare-and-sets the one
persisted legal aggregate. Non-controller operations enter through typed
`LegalMutation` ingress and settle inside the same kernel boundary; no live
Canwu state is mutated through a detached `LegalRuntime` value.

This persisted multi-transaction saga is required by the current public
contracts for actor-mediated source modes:

```text
admitted social evidence
  -> legal boundary records proceeding + decision outbox item
  -> host adapter enqueues ordinary decision ingress
  -> one controller selects an existing option
  -> authorized plugin command records pending legal intent
  -> next legal boundary validates and atomically mutates legal records
```

`canwu-culture` remains responsible for cultural dispositions, propagation,
organization, and generic signals. `canwu-law` owns legal-order definitions,
jurisdictions, proceedings, instruments, effective-time resolution, and legal
interpretation records. Political, election, administration, education,
justice, and enforcement extensions own their respective processes and may
consume legal results. They report compliance, enforcement, and social effects
as evidence; they do not silently change legal validity.

The extension is not a conversion from a percentage of believers to a statute.
Cultural support is one evidence-bearing input to a proceeding. Authority,
jurisdiction, institutional procedure, available capacity, opposition, and
controller choices determine which legal intent, if any, is admitted.

Not every legal transition has a responsible controller. Compiled accretion
profiles may record an order-specific customary claim from admitted practice
and recognition evidence; scheduled effective, expiry, and reception rules may
advance previously authorized state. These are registered, deterministic,
budgeted boundary rules, not synthetic decisions or a back door to universal
validity. They create the same immutable sources and law versions with exact
evidence and remain contestable by other legal orders.

## Scope and non-goals

The engine contract models legal claims and institutional processes, not a
single theory of what law is. A ruleset may configure legislation, decree,
custom, precedent, treaty, religious jurisdiction, or another source of legal
effect without adding source-specific branches to Canwu core.

The first implementation is deliberately bounded:

- one process and one persistence shard, with jurisdiction as the intended
  future shard key;
- deterministic, data-defined procedures rather than a general theorem prover;
- explicit applicable-law and conflict results rather than a claim that every
  conflict has one objectively correct answer;
- legal validity separated from actor knowledge, enforcement, compliance, and
  cultural legitimacy;
- only domain-neutral kernel additions: declared exact domain-record reads and
  an indexed lookup for persisted decision outcomes; legal semantics and
  command-side law mutation remain outside the kernel.

## Simulation layers and kinds of truth

The extension must not collapse five different questions:

1. **Engine admission:** did a caller have Canwu `CommandAuthority` to attempt
   this mutation through canonical ingress?
2. **Legal claim:** which legal order, institution, or community claims that a
   source or rule is valid, and on what evidence?
3. **Operative applicability:** which claimed rules apply to this subject,
   issue, place, and time after competence, validity, scope, and conflict rules?
4. **Actor knowledge:** what did this actor or institution know, believe, or
   misunderstand at an exact holder read cut?
5. **Implementation:** was the rule promulgated, administered, enforced,
   complied with, evaded, or socially legitimated in practice?

Engine authority is a security and mutation boundary, not an in-world verdict
of legality. A properly authorized ruler, assembly, or court may attempt an
ultra vires, procedurally defective, unconstitutional, or rival legal act. The
compiled procedure profile decides whether the legal boundary rejects the
attempt entirely or records a `Purported` or `Contested` source that may later
be displaced, validated, or annulled. A caller without engine authority cannot
use this distinction to write state at all.

Law-owned authoritative state records legal claims and institutional findings.
It does not make every claim objectively valid for every legal order, make
public law automatically known to every actor, or turn legal validity into
compliance.

## Ownership and dependencies

```text
information / correspondence / culture / society adapters
                         |
                         v
                     canwu-law
   (orders, jurisdictions, proceedings, sources, rules, immutable versions,
          outbox, pending intent, applicability, rulings)
                         |
                         v
               canwu-api public contracts
       (decisions, commands, records, knowledge, evidence)
                         |
                         v
 election / administration / education / justice / enforcement adapters
```

`canwu-law` depends on `canwu-api` and may consume declared signal contracts
from `canwu-culture` or another compatible provider. It must not require
`canwu-api`, `canwu-sim`, `canwu-society`, or Canwu core to depend on legal
semantics. A direct dependency on internal runtime modules is forbidden.

The law plugin descriptor, compiled plan hash, source content hash, budget
manifest, legal ID allocation, and every accepted signal-provider identity are
bound into the run configuration. Settlement never reads mutable external
content.

## Compiled authoring model

Law content is namespaced data compiled before a run. The authoring schema
defines legal orders, jurisdiction relations, institutions, authority seats,
procedure profiles, clause schemas, applicability dimensions, precedence
profiles, and adapter bindings. Clause and procedure kinds are open,
namespaced descriptors; historical content is not a Rust enum in the generic
extension.

The compiler:

- canonicalizes names and ordering and rejects duplicate or dangling IDs;
- assigns dense plan-local numeric IDs while preserving stable public IDs;
- validates acyclic relation kinds such as supremacy and appeal while allowing
  explicitly non-hierarchical overlap relations;
- resolves every procedure seat to exactly one declaring institution and rejects
  missing or ambiguous authority; controller IDs use length-prefixed institution
  and seat components so authored IDs cannot collide;
- binds every admitted signal kind to one exact provider plugin and packet type;
- compiles one explicit `NormativeModality` for each clause schema; runtime
  effects never infer modality from the first lexically sorted operation name;
- precomputes bounded indexes from signal, scope, jurisdiction, procedure, and
  clause kinds;
- validates per-record, per-procedure, per-boundary, and total-state budgets;
- emits an externally immutable `CompiledLawPlan` with a canonical hash.

The host composition manifest reserves non-overlapping numeric blocks for
`DecisionTicketId`, `DecisionRequestId`, and `CommandRequestId`. Because these
IDs are global numeric identities in the current API, the legal compiler may
allocate only within the host-supplied blocks. Ticket capacity covers the
declared lifetime ticket count; request blocks cover the larger declared
lifetime totals for create, refresh, resolution, and nested command requests.
The next offsets are authoritative persisted law state and are restored and
validated. Exhaustion is a bounded admission failure, never reuse, wraparound,
or dynamic collision.

### Source modes and change drivers

Each compiled `LegalSourceProfile` selects one bounded origination mode:

- `Promulgated`: legislation, decree, charter, regulation, or another declared
  institutional act follows a persisted procedure and authorized intent;
- `Adjudicated`: an admitted case produces a ruling and any configured
  precedent effect;
- `Accreted`: repeated practice plus recognition evidence may establish a norm
  claim for one legal order without inventing a legislature;
- `Agreed`: treaty, compact, oath, contract-like public settlement, or negotiated
  instrument requires the configured parties and ratification stages;
- `Received`: conquest, union, secession, restoration, or regime replacement
  receives, continues, transforms, or displaces selected prior rules.

Every source profile also declares a typed authority policy
(`ProceduralInstitution` or `EvidenceClaim`), the exact origin policy permitted
for its mode, a publicity policy and compiled publicity signal provider where
publicity is admitted, minimum and maximum evidence counts, required
signal classes, claimant requirements, and whether retrospective effect is
allowed. Compilation rejects a procedural source with claim-only authority, a
non-procedural source with institutional authority, or an origin policy that
does not match its source mode. `Promulgated` is always procedural. An `Agreed`
profile additionally compiles the exact host instrument namespace/kind, a
minimum of two parties, and required ratification evidence.

An accreted rule is not created by one global belief percentage. Its compiled
profile names bounded practice, duration, consistency, publicity, recognition,
and contrary-practice inputs. Satisfying them permits the legal boundary to
record a source claim for the named legal order; another order may ignore or
contest that claim. A profile may additionally require institutional
recognition through the ordinary outbox path.

Signal bindings are namespaced and content-defined. Besides culture, a scenario
may bind evidence of coercive control, administrative or fiscal capacity,
elite coalition, economic bargaining, war or occupation, crisis, external
pressure, communication reach, or public practice. These inputs dirty bounded
proceedings or source evaluations. None directly mutates legal state or serves
as a universal cross-period causal formula.

The compiled binding is authoritative. Live admission accepts a signal kind only
when one of its exact `EvidenceRef::Ingress` values still exposes matching core
metadata `(provider_plugin, packet_type)` and a kernel-committed
`BoundaryIngressGeneration` naming that plugin as the producer. Direct host
injection into the same namespace is not provider evidence. The law plugin does
not inspect the provider payload, and a caller cannot establish provenance by
pre-filling an `admitted_signal_kinds` field. If the ingress or producer boundary
metadata was archived before first admission, admission fails closed; after
admission, the immutable legal source keeps its own evidence reference and does
not depend on re-reading that payload.

## Authoritative records and components

The payload lifecycle below is legal meaning. Generic
`DomainRecordLifecycle` continues to control storage lifecycle.

### Legal order, jurisdiction, and institution

`LegalOrder` identifies one source system of legal validity, such as a state
code, customary order, religious jurisdiction, treaty order, or military
administration. It binds source and precedence profiles without declaring one
order universally superior.

`LegalJurisdiction` has bounded typed relations rather than a single ownership
parent. Relations may express delegation, territorial containment, supremacy,
appeal, treaty membership, or acknowledged overlap. Territory is one scope
dimension; personal status, office, organization membership, subject matter,
and event location may also affect applicability.

`LegalInstitution` binds an organization entity to one or more jurisdictions,
competences, authority seats, and procedure profiles. It does not make the
organization omnipotent: each command still requires the exact seat,
permission profile, controller provenance, subject, and procedure state.
Competence is default-deny and separately scopes legal order, jurisdiction,
subject matter, source mode, operation, procedure, forum, and adjudicative
power. A confirmed operative act outside that scope is rejected; a configured
purported or contested claim must carry the canonical competence defect.

### Legal proposal and procedure instance

`LegalProposal` is a versioned, non-enacted proceeding input. It contains:

- proposal identity, sponsor, legal order, jurisdictions, and subject refs;
- bounded typed clause operations and applicability descriptors;
- a legal competence finding, canonical defect codes, claimed validity
  disposition, and an exact ruling/agreement/reception origin where required;
- an exact immutable publicity-event ref and an explicit retrospective date
  where the source profile permits them;
- the governing procedure profile version and deadline;
- status such as `draft`, `submitted`, `deliberating`, `adopted`, `rejected`,
  `expired`, or `withdrawn`;
- exact `EvidenceRef` values for admitted social inputs and record versions;
- the active `ProcedureInstance` and expected record-version guards.

`ProcedureInstance` freezes the procedure profile version, eligible seat
roster, stage sequence, quorum and threshold rules, veto and ratification
requirements, replacement policy, deadlines, deterministic tie-break, and
capacity reservations. Later changes to an institution or profile do not
rewrite an open instance.

Each eligible seat receives its own single-controller ticket. Ballot commands
become pending intents and then immutable `ProcedureParticipation` records.
The procedure profile defines whether the first ballot is final or a later
ballot may replace it before the deadline. Duplicate requests are idempotent;
conflicting late ballots, ineligible seats, stale rounds, and disallowed
replacements become persisted rejected outcomes. Succession changes the actor
who may control an uncast seat but does not erase an already admitted seat
action. Quorum aggregation runs in canonical seat order. Separate stages model
veto, signature, review, or ratification instead of pretending that one ticket
has multiple controllers.

At compilation, a stage seat must belong to exactly one institution that also
declares the procedure. That resolved institution, holder, permission profile,
seat, and collision-free controller ID are frozen into the plan and copied into
the procedure instance; runtime lookup never selects the first matching authored
institution.

### Source, stable rule, and immutable law version

The aggregate topology separates authentic sources, stable rule identities,
and immutable normative changes. `canwu-law` persists one canonical
`LegalRuntimeRecord`; the items below are validated records inside that
aggregate, not standalone kernel domain records:

- `LegalSourceVersion` is a create-only adoption snapshot for an instrument,
  judgment, treaty or compact, customary-recognition finding, received-law
  schedule, or other configured source. It preserves source text/content hash,
  exact proposal and typed origin, issuer or claimant, procedure, competence
  finding, typed authority policy, claimed validity, adoption-time publicity
  fields when already available, dates, defects, and evidence without pretending
  that every source is a statute. If an effectiveness-condition source is
  published after adoption, the create-only snapshot remains unchanged and the
  actual publicity status and time are derived from the proposal's exact link to
  its independent `LegalPublicityEvent`.
- `LegalRule` is the stable, mutable head record for one bounded normative rule.
  It points to its source order, latest adopted version, bounded scheduled
  changes, and current materialized operative state.
- `LawVersion` is a create-only normative record. Its stable aggregate-local ID
  includes the rule ID and legal ordinal.
- each `LawVersion` records an operation such as establish, recognize, receive,
  amend, suspend, resume, displace, annul, repeal, or expire; exact source and
  predecessor refs; typed normative deltas; applicability scope; immutable
  adoption-time metadata and effective time; an independently linked publicity
  event supplies promulgation semantics; optional retrospective application time;
  validity disposition by legal order; typed origin; and exact evidence refs.

The v1 aggregate creates one source and one rule version per proposal; several
clause deltas may share that version. An application groups proposals that came
from one larger instrument through their exact external instrument, ruling, or
reception origins. The generic model does not materialize one record per
affected person.

Normative deltas use bounded descriptors for duty, prohibition, liberty,
claim-right, power, liability, immunity, disability, status, and eligibility.
A descriptor names applicable holders or beneficiaries, duty bearers, actions
or objects, conditions, exceptions, standing, forum, remedy profile, and source
refs. The compiler fixes modality, and admission rejects incomplete minimum
relations: claim-right and eligibility require both holder and duty bearer;
duty and prohibition require a duty bearer; other modalities require a holder.
Namespaced content schemas may add period-specific fields. The
applicability index consolidates these deltas into sparse `NormativeEffect`
projections; it does not run an unrestricted legal theorem prover.

An amendment that takes effect in the future creates a new immutable
`LawVersion` and updates `LegalRule.scheduled_versions`; it does not edit the
currently operative version. At the effective boundary the law system advances
the materialized head in canonical `(effective_at, legal_ordinal, record_id)`
order. A successor version may never move `effective_at` behind the latest
rule head, including when two versions are scheduled before either activates.
Repeal and annulment are likewise new immutable versions. Historical
sources and versions remain addressable by exact aggregate-local
`LegalRecordRef { kind, id }` values after amendment, repeal, culture
retirement, or index rebuilding. `DomainRecordVersionRef` is reserved for real
host-owned Canwu records captured by proposal compare-and-set guards; the law
extension never fabricates kernel version provenance for aggregate members.

### Case, finding, and ruling

`LegalCase` is a persisted proceeding with parties, claims, alleged
facts, forum, standing question, proof profile, issues, deadlines, and remedy
requests. Allegations remain distinct from institutionally accepted
`LegalFindingVersion` records. A proof profile supplies bounded evidence kinds,
burdens, and standards as scenario data; the generic engine does not infer
historical truth from narrative text.

The forum is a compiled profile, not a caller-selected jurisdiction string. It
scopes legal orders, jurisdiction, subject matter, institutions, proof,
standing, remedies, and precedent. Findings must address an issue in the same
case, use its proof profile, remain inside the case interval, and form an exact
same-case/same-issue predecessor chain.

`LegalRulingVersion` is a create-only source for an institution-specific
interpretation, validity disposition, conflict decision, or remedy. A ruling
names its exact case, competent institution, issues, findings, sources
considered, exact resolved and selected law-version sets, scope, precedent
profile, effective interval, remedy, and same-case predecessors. The runtime
checks the institution's adjudicative competence against the compiled forum,
legal order, jurisdiction, subject matter, scope, proof findings, requested
remedy, and precedent profile. The generic runtime records the ruling but does not automatically
turn it into a new law version; an application may submit a subsequent
adjudicated-source proposal through the same source and rule-head CAS path. It
must carry `LegalOriginRef::Ruling` naming that exact ruling. It never rewrites
an earlier source, allegation, finding, or rule version. Agreed sources likewise
name an exact host-owned agreement version, canonical parties, and exact
ratification evidence. Received sources name an exact succession and
predecessor law version; `Transform` also names a compiled target clause and
the received proposal must use it.

### Decision outbox

`LegalDecisionOutboxItem` is an aggregate member materialized after legal
settlement and exposed to the host only after the updated aggregate is
persisted. It contains:

- stable outbox sequence and operation (`create` or `refresh` ticket);
- preallocated ticket ID plus creation/refresh, resolution, and nested command
  request IDs from the legal ID blocks;
- source proposal/procedure exact aggregate refs and source boundary;
- the assigned controller and command subject bindings;
- a complete `DecisionTicketDraft` or refresh mutation;
- the exact `KnowledgeHolderRef`, `KnowledgeReadCut`, context schema/hash, and
  bounded holder knowledge record IDs used for its actor-relative context;
- due time, priority, dispatch state, and expiry.

An actor ticket uses that actor's holder. An institution or council ticket uses
the corresponding organization holder. There is no fallback to authoritative
legal truth when the required holder projection is absent. The perspective
system either materializes a bounded context from the declared read cut or
persists a blocked outbox outcome.

A deterministic host adapter uses three persisted stages. First,
`prepare_pending_decision_enqueues` records the exact expected Canwu revision in
the aggregate. Second, `enqueue_pending_decisions` submits the byte-identical
controller and ticket requests, which must settle before acknowledgement.
Third, `acknowledge_enqueued_decisions` resolves the exact controller and
ticket-open request IDs in the indexed, persisted decision journal and emits an
ACK. The plugin requires both outcomes to be `Accepted`, checks their expected
revision and immutable request binding, verifies the current controller and
ticket against the persisted draft, and recomputes a stable outcome commitment.
This proof survives ingress archival. A controller is registered once and then
reused by every exact seat-bound ticket; a conflicting binding is rejected. If
unrelated work makes a prepared revision stale, a canonical prepare ingress may
safely record a fresh revision while no attempt or ticket exists and any existing
controller is the exact compiled binding. Exact replay consumes the recorded
preparation, decisions, and ACK rather than rerunning the adapter.

### Pending legal intent

A selected legal decision action is a normal `Command::Plugin` command. The
handler validates the controller provenance, command subject, authority seat,
permission profile, payload shape, and queue budget. It schedules a
plugin-owned intent ingress. The registered boundary system verifies the
command cause and updates the bounded canonical intent queue inside the
aggregate, containing:

- an intent identity derived from the admitted command identity;
- command and attempt evidence, request identity, controller ID, and seat;
- proposal, procedure, round, and expected host-owned domain-record versions;
- selected option ID, clause/content hash, and intended effective times.

The handler cannot mutate `LegalProposal`, `ProcedureInstance`,
`LegalSourceVersion`, `LegalRule`, or `LawVersion`. The legal boundary consumes
only intents visible in its boundary snapshot, so a newly accepted command
cannot feed back into legal state in the same transaction.

Expected failures such as stale versions, expired stages, duplicate ballots,
failed quorum, superseded proposals, or exhausted legal capacity produce a
bounded `LegalIntentOutcome` with `rejected` or `expired` status and remove the
intent from the hot queue. They do not return a system error that repeatedly
rolls back the boundary. Structural corruption or an invariant violation still
fails the whole transaction.

## Transaction and phase flow

The implementation uses one plugin-owned aggregate and canonical ingress:

1. Signal providers schedule bounded law plugin ingress. A zero-delay schedule
   is still admitted no earlier than the next transaction.
2. The event-driven law boundary loads the aggregate and its embedded compiled
   plan, checks its immutable plan/budget binding, validates the record's expected
   version plus every host-owned `expected_versions` guard, verifies signal
   provider generation proof, and applies typed `LegalMutation` values. Complete history
   and derived-index validation runs at cold load/restore, not once per mutation.
3. The same `DomainDeltaProposal` validates all fallible work, consumes pending
   intents, advances only dirty or due procedures, applies due versions,
   refreshes dirty applicability projections, and emits one aggregate update
   with `expected_version` equal to the record it read. Kernel commit is atomic.
4. Holder contexts enter through `legal_actor_context` ingress. The plugin
   executes the supplied bounded `KnowledgeQuery` against the named holder and
   derives facts from returned knowledge records; callers cannot inject a
   synthetic read cut or arbitrary JSON truth.
5. After an aggregate commit materializes outbox work, the host runs the
   three-stage decision dispatch and accepted-outcome ACK protocol. A later
   accepted command schedules pending legal intent through canonical ingress.
6. Indexed wake ingress is scheduled for procedure expiry and future law
   effective times. Procedure expiry also expires unresolved pending or
   enqueued outbox items, so abandoned work cannot remain hot forever.

This phase cut prevents a culture -> law -> enforcement -> culture feedback
cycle inside one boundary. Every adapter crossing is evidence-bearing and
visible no earlier than the next declared transaction.

The actor-mediated establishment path is therefore:

```text
EvidenceRef::Ingress(cultural_or_social_batch)
  -> LegalProposal + ProcedureInstance
  -> holder-bound LegalDecisionOutboxItem
  -> DecisionTicket / DecisionAttempt / DecisionTrace
  -> authorized command
  -> pending legal intent component
  -> next-boundary atomic source + LegalRule + LawVersion mutations
  -> downstream applicability, enforcement, and feedback
```

## Evidence and audit contract

The first slice uses only existing shared evidence identities:

- an admitted cultural or other plugin batch is cited as
  `EvidenceRef::Ingress`;
- a legal command and its admission are cited as `EvidenceRef::Command` and
  `EvidenceRef::CommandAttempt`;
- host-owned mutable inputs are cited by exact
  `EvidenceRef::DomainRecordVersion`; law-owned aggregate members use exact
  `LegalRecordRef` identities and immutable topology checks;
- committed transactions may cite `EvidenceRef::Boundary` and emitted events
  may cite `EvidenceRef::Event`.

The aggregate schema declares sorted identity-only dependencies for unresolved
proposals and for sources of operative or scheduled law versions. Evidence
sealing therefore keeps only the relevant Merkle receipt. For generated plugin
ingress that receipt binds `(plugin, packet_type, producer_boundary)` and lets
the legal provider check continue after compaction without loading the archived
payload. Closed proceedings and no-longer-live legal versions fall out of the
declaration and their receipts become collectible at the next seal.

The runtime maintains this dependency declaration and an ordered reference-count
index whenever proposal, procedure, or law-version topology changes. Ordinary
writes increment or decrement only the affected owner's deduplicated references
instead of scanning retained legal history. Cold load independently reconstructs
both indexes and rejects any mismatch. Both direct sealing and
prepare/store/commit sealing are atomic: a missing receipt or other late
validation failure leaves the live checkpoint unchanged.

`CauseRef` is used only where its current variants actually fit. The design does
not claim that `CauseRef` or `DomainReference` can identify a decision trace.
The accepted command carries validated controller and decision-origin
provenance, while the pending intent stores the ticket ID/version and selected
option as legal payload. If direct `DecisionTraceId` evidence becomes a shared
requirement, that is a separate core evidence-format and migration proposal;
`canwu-law` does not invent a private substitute and call it a shared ref.

## Cultural effects and retirement

The culture persistence class determines how the bridge handles a signal, not
whether law remains valid:

| Culture effect | Legal handling |
| --- | --- |
| `Pulse` | May dirty or open a proceeding opportunity; it creates no durable law. |
| `Level` | Supplies current support, legitimacy pressure, or capacity evidence. Its end may trigger a configured review. |
| `Commitment` | Once accepted by a legal boundary, becomes provenance for an immutable legal version. |
| `Evidence` | Is retained as cited history and cannot itself open or mutate a proceeding. |

Dependencies name an exact `CulturalTargetGenerationRef` and are either
`AdoptionEvidence` or `LiveLevel`. The law aggregate blocks retirement while an
open procedure, pending intent, pending/enqueued outbox item, operative
`LiveLevel`, or future scheduled version with a `LiveLevel` dependency still
names that generation. Retirement is idempotent,
and load validation rebuilds the retired-generation index from tombstones.
`AdoptionEvidence` remains cited history but does not keep the culture target
hot after enactment. This allows a `women_political_equality` generation to
retire after suffrage legislation is enacted while the voting rule and legal
history remain. The operative law retains only its compact adoption-evidence
receipt; it does not keep the retired culture runtime, propagation indexes, or
payload in the hot path.

Retirement is an explicit maintenance operation, not ordinary boundary work.
The v1 aggregate may inspect open procedures, pending intents/outbox items, and
rules to prove that no live dependency remains, but the compiled
`max_retirement_dependency_records` budget bounds the outer records scanned and
rejects the whole operation before mutation when exhausted; per-record fan-out
remains bounded by `max_evidence_per_record`. A target-keyed dependency index is
the intended replacement after jurisdiction sharding.

An unadmitted culture effect batch is not visible inside `canwu-law`; the host
culture/law coordinator must withhold culture retirement until that external
queue is admitted or discarded. The law extension claims only the dependencies
present in its own canonical aggregate.

If a legal rule deliberately depends on a live cultural level, the law content
must define its own review, expiry, or renewal operation. The culture runtime
emits the end of the level; it never silently repeals the rule.

## Applicability, overlap, and conflict

Applicable law is a bounded query, not a map lookup by territory. Its input
contains simulation time, subject, actor/person status where authorized,
territory or event location, subject matter, a required legal-order partition,
an exact applicability profile, and an optional jurisdiction. An actor-relative
query must bind the actor's exact `KnowledgeReadCut`; every asserted predicate
fact carries an exact evidence ref and holder knowledge-record ID. The compiled
predicate binds an exact knowledge schema and JSON boolean pointer. The
host-bound API verifies the holder, complete read cut, record, schema, pointer,
and asserted value; the detached query API rejects actor-relative reads.
Order/rule and
rule/version indexes collect bounded candidates without scanning every retained
rule or historical version.

The implemented `ApplicabilityProfile` has the fixed, validated pipeline
`scope -> jurisdiction -> validity -> conflict` rather than one opaque
precedence score. Scope checks holder, duty bearer, territory, and subject
matter plus compiled condition and exception predicates. A false condition or
true exception produces `NotApplicable`; missing required facts produce
`Indeterminate`. Jurisdiction uses the profile's compiled relation-kind and
direction adjacency and computes one bounded reachable set per query rather
than traversing the graph for every candidate. Validity distinguishes an operative version from later
`Claimed`, `Purported`, or `Contested` acts. A disputed amendment therefore
returns both the still-operative predecessor and the disputed claim in a
`Contested` trace instead of erasing either position. Conflict records preserve
the exact total, governing, and displaced version sets, typed resolution basis,
rationale, jurisdiction, recorded time, effective interval, and optional
competent ruling. A conflict applies only when its complete exact claim set is
present in that jurisdiction and interval. Its partition may promote a disputed
claim or reject it; resolved claims no longer force `Contested`. All active
partitions are merged as sets, so contradictory overlapping rulings return
`Contested` instead of being applied in conflict-ID order. Non-temporal
resolution requires an operative, case-bound ruling whose resolved/selected
sets match that partition exactly and whose jurisdiction covers the conflict; temporal
precedence is allowed only as the final compiled basis. Queries apply the
recorded partition and never recalculate a different winner. Richer doctrine
such as emergency exceptions or choice-of-law factors belongs in future typed
pipeline stages, not free-form strings that the runtime ignores.

Profiles differ by legal order and exact profile ID. Each result includes a
bounded resolution trace containing the aggregate-local versions used. The
result is one of:

- `Applicable`: an ordered set of compatible versions;
- `Displaced`: a version is in scope but a declared rule defeats it;
- `Contested`: multiple claims remain and a competent ruling is required;
- `Indeterminate`: required knowledge, scope data, or a procedure result is
  absent.

The resolver never resolves conflict through insertion order or silent map
overwrite. Contested results may open a bounded adjudication proceeding through
the same outbox and intent path. Supremacy and appeal edges must be acyclic;
overlap and treaty edges may form a bounded graph.

Operative state distinguishes at least `Claimed`, `Purported`, `Operative`,
`Suspended`, `Displaced`, `Annulled`, `Repealed`, `Expired`, and `Contested`.
Whether annulment is prospective or treated as void from an earlier time is an
explicit ruling operation; the engine never infers it from the word "invalid."

## Promulgation, knowledge, and implementation

Adoption, source establishment, promulgation, effective time, retrospective
application time, actor learning, enforcement, and compliance are separate.
A source profile states whether publicity is a validity condition, an
effectiveness condition, optional evidence, or not admitted. Publication is a
separate immutable `LegalPublicityEvent` with exact proposal, canonical host
ingress time, compiled provider/packet kind, medium, scope, and evidence. The
live adapter admits it only on the boundary where that provider event occurs;
a future plan is not an occurred publicity event and must be scheduled as later
ingress. Validity-condition publicity must occur no later than adoption, while
effectiveness-condition publicity must occur no later than effective time. A source
cannot acquire retrospective effect merely because adoption occurred late: the
profile must allow it and the proposal must declare a bounded retrospective
date no later than its effective date. Publication creates information or
knowledge inputs for downstream propagation; it does not grant omniscient
awareness. Actor-relative legal reads may therefore return a known obsolete
rule, an incomplete source, or no known rule while the authoritative legal
resolver has a different result.

Administration, justice, and enforcement extensions own sparse
`LegalImplementationState` keyed only to active rule/scope relationships. It may
record administrative coverage, selective enforcement, institutional capacity,
compliance, evasion, and remedy delivery with exact law-version refs. Those
facts feed later legal or cultural proceedings as evidence but never rewrite a
rule's validity by side effect.

## Legal-order succession

`LegalOrderSuccession` is an immutable record for conquest, union, split,
secession, restoration, constitutional replacement, or another configured
transition. It names predecessor and successor orders, effective time,
territorial and personal scope, institutions, liabilities, archives, and a
bounded reception table. The table explicitly continues, transforms, reviews,
or displaces indexed groups of prior rules. No new regime automatically
inherits every rule, and no regime change deletes historical sources.

The implemented runtime records immutable succession and bounded reception
tables, indexes them by successor, and consults them during applicability. A
query must match territorial and personal scope; each predecessor rule uses the
longest matching prefix, and no match means no reception. Only `Continue`
exposes the old version directly. `Transform` and `Review` require an explicit
`Receive` proposal, while `Displace` excludes the rule. That proposal carries a
typed origin naming the succession and exact predecessor version. Traversal and
candidate insertion are bounded during collection, and the received predecessor
is also an exact `LawVersion.predecessors` lineage edge. Cross-order disputes may
remain `Contested` until an applicable ruling or agreement resolves them.

## Capacity and procedure resources

Institutional attention, court sessions, clerical throughput, and other scarce
procedure resources use the existing deterministic reservation phase. Each
procedure profile declares a reservation pool and quantity; the host's shared
allocator owns priority and tie-break policy. The runtime admits only an exact
matching allocation. An unallocated proceeding waits; application policy may
later withdraw or expire it, but the generic runtime does not mutate capacity or
skip the common allocator.

## Persistence, loading, migration, and replay

The extension registers one typed `LegalRuntimeRecord` aggregate with an
explicit update policy. Generic snapshot validation is necessary but not
sufficient for its internal semantic law graph. The public integration exposes
`load_law_state_for_plan` (and an equivalent snapshot constructor) that runs
after Canwu's normal snapshot, checkpoint, or replay validation and then:

- verifies the plugin semantic hash, compiled law plan hash, source content
  hash, signal-provider bindings, ID blocks, and budget manifest;
- decodes the complete aggregate before returning the simulation and rejects
  non-canonical law-owned record IDs;
- validates record IDs, create-only version topology, exact predecessor and
  evidence refs, effective timelines, frozen rosters, procedure/profile hashes,
  outbox/ack state, pending-intent bounds, and legal-order graph constraints;
- recomputes applicability, dirty, procedure, schedule, and outbox indexes in
  canonical order and compares any persisted derived roots;
- rejects any mismatch before exposing authoritative or actor-relative reads.

Exact replay requires the same plugin descriptor and semantic environment. An
older legal schema may be imported only by an application-owned, versioned
export/import tool that starts a new run or causal branch with provenance. It
must not be labeled exact replay under changed semantics.

Exact replay consumes the recorded signal, decision, command, acknowledgement,
boundary, and evidence journals. It does not rerun a human, external service,
LLM policy, outbox adapter, or historical interpretation policy.

## Scale and bounded work

The law extension must not build a signal-by-proposal-by-institution or
subject-by-law cross-product. Its own steady-state algorithms use:

- dense compiled IDs and bounded reverse indexes;
- compiled per-profile jurisdiction adjacency keyed by relation kind and
  direction, with one candidate-budgeted reachable-set build per query;
- a total applicability-query work budget covering graph edges, rules,
  historical-version visits, effect descriptors, holder/duty/territory and
  predicate visits, conflict-index fan-out, and conflict members;
- a per-record nested-item budget preflighted before proposal, case, ruling, or
  conflict collection traversal;
- dirty proceeding, source, rule, jurisdiction, applicability, and outbox sets;
- an incrementally maintained retained-evidence dependency set and reference
  counts, with full reconstruction reserved for cold validation;
- per-profile limits for clauses, evidence, seats, stages, options, fan-out,
  scheduled versions, pending intents, and continuations;
- global law budgets for active and historical records, open and closed
  procedures, outbox items, knowledge refs, and derived index bytes;
- a single-shard deterministic schedule in v1.

For `P_delta` dirty proceedings, `C_delta` affected clauses, `J_delta` dirty
jurisdictions, and `V_delta` projection entries, law-local settlement targets
`O(P_delta + C_delta + J_delta + V_delta)` subject to compiled fan-out. Both
applicability deletions and insertions count against the boundary mutation
budget. Current operative projections are maintained from dirty-rule indexes. Historical-time
applicability requires a legal-order partition and exact profile, gathers at
most the compiled candidate limit from the order index, and scans only while
the total query-work budget permits. This scan is required to honor explicit
retrospective dates that precede a version's ordinary effective-time index key.

This is not a whole-engine history-independent complexity claim. The current
single `LegalRuntimeRecord` must be decoded and encoded as a whole. Boundary
checkpoint capture now shares the domain-record map root in O(1), but the first
domain-record write still copies that whole map. Decision state is still cloned
as one resident value and retains tickets, attempts, and traces; exact outcome
proof uses an O(log n) request index, but storage-aware decision-history APIs
remain part of the format-8 proposal rather than the format-7 implementation. A
live legal boundary is therefore approximately
`O(H_serialized + P_delta + C_delta + J_delta + V_delta)`, where `H_serialized`
is the persisted aggregate and first-write transaction state. Until law state
is sharded and content-addressed page deltas plus closed-decision payload
archival exist, the run manifest must cap those populations. `canwu-law` claims
only that its post-decode settlement algorithms do not rescan the historical
catalog.

The reproducible release probe is:

```text
cargo run --release -p canwu-law --example law_scale
```

On the implementation workstation, 5,000 law-local idle settlements had a 200 ns
median with 1,000, 10,000, and 100,000 retained retirement records. Five real
Canwu plugin boundaries, including aggregate decode, transaction clone, CAS, and
encode, had median times of 37,472 us, 507,535 us, and 6,017,504 us respectively.
These are regression baselines, not cross-machine guarantees. The second curve
means that even the 1k result exceeds both a 60 FPS frame budget (16.7 ms) and a
30 FPS frame budget (33.3 ms), so legal settlement is not per-frame work. The 10k
result is suitable only for low-frequency turn or background work, and the 100k
result only for offline or maintenance processing. The implemented
domain-record root sharing removes that map's full checkpoint copy, but not
aggregate serialization, decision-state cloning, or first-write map copies.
The legal archive state-machine code is currently a private test-only scaffold,
not an activated persistence path. Before jurisdiction sharding and page-delta
persistence land, a live manifest should therefore set its hard retained-record
cap well below 1k and calibrate that cap on target hardware. Sharding/page COW
is a scale milestone, not optional polish.

The complete proposed milestone is now defined by
[Legal storage sharding, COW, delta persistence, and cold archive](legal-storage-sharding-compaction.md).
It treats the four changes as one persistence boundary: shard-local legal hot
state alone is insufficient while the kernel clones and checkpoints all domain
records, and kernel COW alone is insufficient while one legal aggregate still
serializes all history. The design also distinguishes cultural retirement,
legal repeal, cold archive placement, and physical garbage collection so an
enacted right does not disappear merely because its originating culture or old
procedure leaves the hot path.

Cross-plugin retirement is kernel-coordinated and owner-authorized: the culture
plugin prepares only its culture-record mutation, the law plugin prepares only
its target-keyed dependency-record mutation, and the kernel commits both or
neither. Decision history APIs likewise distinguish bounded hot state from
provider-backed cold history instead of promising that every old attempt and
trace remains resident.

Before multi-shard support, cross-jurisdiction proceedings remain in the single
canonical shard. The proposed format-8 design uses one normative order shard,
jurisdiction projection/procedure shards, target-keyed culture dependency
records, and a persisted deterministic coordinator for cross-shard proceedings;
no shard may independently perform the final legal write.

## Examples

### Women's suffrage

A culture pack may define `women_political_equality` and emit public alignment,
organization capacity, and legitimacy-pressure ingress for a jurisdiction. The
law extension may open a bounded proposal to change a typed voting-eligibility
clause. Each frozen assembly seat receives a holder-bound decision ticket; the
procedure aggregates admitted seat ballots, any veto, and ratification stages.
An accepted adoption command records a pending intent. The next legal boundary
creates an immutable adoption snapshot as `LegalSourceVersion` plus an immutable
`LawVersion` with a deterministic effective time. Required validity publicity
already exists; effectiveness publicity may be appended later as its own event,
without rewriting either version, before the stable `LegalRule` head can become
operative. The
election adapter resolves applicable eligibility rules. Retiring the culture
target stops new propagation but does not remove the source, rule, version,
procedure, evidence, or enforcement history.

The conformance fixture compiles this clause as `Eligibility`, with
`status:adult-women` as holder, election administration as duty bearer, voting
as subject matter, adult/citizen conditions, affected-voter standing, a forum,
and a registration remedy. It verifies the relation and future effective time,
not merely that a generic proposal passed.

The engine does not encode "women" as one timeless boolean category. A content
pack supplies the period-specific person-status and eligibility descriptors,
exceptions, territorial scope, and knowledge rules used by that scenario.

### Human-rights principles

A content pack may define separate cultural targets such as universal dignity,
equality before law, due process, and expression rights. They can support
different legislative, constitutional, treaty, customary, or adjudicative
proceedings. No aggregate "human-rights percentage" automatically becomes a
constitution. A legal version may be formally applicable while enforcement,
compliance, public knowledge, and cultural legitimacy remain low or contested.
Each right is a bounded normative relation: it names holders or beneficiaries,
duty bearers, protected actions or interests, exceptions, standing, forum, and
available remedies. A content pack cannot obtain plausible rights simulation by
setting one global `human_rights_enabled` flag.

### Overlapping legal orders

A town may be within a territorial code, a guild's personal jurisdiction, and
a religious court's family-law competence. An applicability query gathers only
indexed candidates and applies the scenario's applicability profile. If
competence or supremacy is not resolved, it returns `Contested` and may open a
ruling procedure. The engine does not silently choose the territorial code
because it was inserted last.

### Custom and later codification

A community practice may accumulate duration, consistency, publicity,
recognition, and contrary-practice evidence under one legal order's accretion
profile. The legal boundary may record a customary `LegalSourceVersion` and
rule claim without creating a fictional legislative vote. A later court may
recognize or reject it, and a later code may receive or displace it. These are
new sources and immutable law versions; codification does not rewrite the
earlier practice evidence.

## Fidelity acceptance rubric

This design is evaluated as a reusable game-engine extension, not as a complete
jurisprudential model. A release candidate needs no blocking finding and at
least 8/10 in each dimension below when tested across unlike historical cases:

- **cross-period portability:** statute, decree, custom, judgment, agreement,
  reception, and legal pluralism do not require kernel branches;
- **institutional causality:** authority, competence, procedure, capacity,
  knowledge, coercion, culture, and controller choice remain distinguishable;
- **legal structure:** source, rule, normative relation, validity,
  applicability, interpretation, remedy, and implementation are not collapsed;
- **historical legibility:** every change retains time, claimant, procedure,
  evidence, predecessor, and succession provenance;
- **gameplay legibility:** rulesets can expose understandable proceedings,
  choices, blockers, conflicts, and consequences without revealing hidden
  authoritative truth to an actor;
- **bounded execution:** authoring and settlement have explicit cardinality,
  fan-out, state, history, and query budgets with measured failure behavior.

The intended abstraction deliberately stops short of natural-language legal
interpretation, universal moral judgment, unbounded fact finding, or a dense
person-by-rule model. Period-specific categories, doctrine, evidentiary weight,
institutional behavior, and calibration belong to content packs and adapters.
The generic extension supplies deterministic, inspectable contracts through
which those choices operate.

## Conformance gates

Before calling the legal extension complete, durable contract tests must prove:

- cultural signals and command handlers cannot directly mutate legal records;
- outbox dispatch uses preallocated IDs, exact read cuts, idempotent retry, and
  recorded acknowledgement ingress;
- only controller-bound authorized commands can append pending legal intents;
- engine authority and in-world legal competence remain distinct, including a
  configured path that records an authorized but legally `Purported` act;
- pending intents are visible no earlier than the next transaction;
- stale, duplicate, late, unauthorized, and over-budget intents become safe
  persisted outcomes without poisoning settlement;
- per-seat tickets, frozen rosters, succession, ballot replacement, quorum,
  veto, ratification, and deterministic ties follow the compiled procedure;
- future-effective amendments and repeal preserve the currently effective and
  historical immutable versions;
- promulgated, adjudicated, accreted, agreed, and received source fixtures reach
  legal state only through their declared profiles and evidence;
- normative effects retain holder, duty-bearer, modality, exception, standing,
  forum, and remedy bindings without per-person dense materialization;
- applicable-law queries handle territory, personal scope, precedence, and
  unresolved conflict without insertion-order behavior;
- actor and institution tickets cite exact holder read cuts and never fall back
  to authoritative truth;
- culture retirement preserves accepted law and evidence while a live level
  dependency still blocks retirement;
- legal-order split, merge, conquest, and restoration fixtures apply only their
  explicit reception tables and preserve predecessor history;
- authoritative applicability, holder knowledge, enforcement, and compliance
  can diverge without corrupting one another;
- semantic hash, schema, plan, graph, index, budget, and evidence tampering fail
  load before reads;
- snapshot, checkpoint, fork, archive reconstruction, and exact replay retain
  the same legal and decision results;
- duplicate or reordered unrelated signals, retired targets, observers, and
  legal history do not perturb keyed law-local results;
- benchmarks separate law-local dirty work from current whole-kernel record and
  decision-history costs, including near-budget cases.

## Implemented milestone

The single implementation milestone completed all of the following:

1. Compile legal orders, jurisdiction relations, institutions, procedures,
   source modes, normative descriptors, applicability profiles, ID blocks, and
   budgets into a hashed plan.
2. Persist one canonical typed runtime aggregate containing proposals,
   procedures, sources, stable rules, immutable law versions, cases, findings,
   rulings, participation, outcomes, pending intents, and outbox state.
3. Implement provider-bound signal admission, plan-bound legal settlement,
   semantic validation, deterministic reservations, and module-owned cold-load
   validation.
4. Implement kernel-derived holder-bound contexts and the crash-recoverable
   prepare/decision/ACK host adapter using existing decision ingress plus an
   archive-safe proof of accepted outcomes and exact ticket binding.
5. Implement authorized commands that append only pending legal intents, then
   atomic next-boundary adoption, amendment, suspension, repeal, and rejection.
6. Implement compiled forums and default-deny institutional competence; exact
   cases, findings, rulings and conflict partitions; evidence-bound predicate
   applicability; independent publicity and retrospective-effect guards;
   succession lineage; and actor-relative legal projections.
7. Expose applicability, exact law-version references, and evidence contracts
   for application-owned election, administration, justice, and enforcement
   adapters. Those domain systems are not present in this workspace and are not
   falsely implemented inside the generic law crate.
8. Bound ordinary work with clause, jurisdiction, projection, mutation,
   open-procedure, latest-participation, outbox, scheduled-version, and
   dirty-rule limits, including applicability deletions, indexed due-procedure
   and decision-outcome lookup, and ship a 1k/10k/100k retained-history probe for
   both law-local settlement and the real Canwu plugin path. Jurisdiction sharding
   and kernel COW remain scale work rather than missing legal semantics.

The legal extension remains a downstream ruleset. A historical first-party
case may instantiate rights, suffrage, custom, or jurisdictional conflict only
through these generic contracts; the extension does not ship a privileged
historical narrative or natural-language legal interpreter.
