# canwu-law

`canwu-law` is an optional, deterministic legal-order extension for Canwu. It
compiles namespaced legal content into a hashed, budgeted plan and provides a
small runtime ledger for procedures, sources, immutable law versions, cases,
applicability, succession, and actor-relative decision outbox work.

The extension depends only on `canwu-api`. Legal records are owned by this
plugin and are admitted through ordinary Canwu boundary proposals; the kernel
does not contain legal semantics. The command handler can append only a
bounded pending legal intent. A later legal boundary consumes that intent and
creates immutable source and law-version records.

Each pending outbox item contains a typed `DecisionTicketDraft` with the frozen
seat holder, knowledge read cut, exact proposal and procedure aggregate refs, and
command-bearing vote options. Before settlement, hosts read
`pending_actor_context_requirements` and either enqueue a live
`legal_actor_context` ingress or use `stage_actor_context_from_canwu` while
constructing initial state. Both paths execute a bounded holder
`KnowledgeQuery` and derive facts, record IDs, and the read cut from the kernel
result; callers cannot inject arbitrary JSON facts. Seats without staged
context do not emit a ticket.

Dispatch has three persisted stages. Hosts call
`prepare_pending_decision_enqueues`, settle and reload the aggregate, then call
`enqueue_pending_decisions` and settle the core decision ingress. Finally,
`acknowledge_enqueued_decisions` resolves the exact controller and ticket-open
outcomes in Canwu's indexed decision journal and queues an ACK. The boundary
system requires both outcomes to be accepted and checks their revision and
request bindings, the exact controller/ticket state, draft hash, and stable
outcome commitment. The proof survives ingress archival. A crash can reload and
retry without recapturing context or draft; a stale preparation can be refreshed
while no attempt or ticket exists and an existing shared seat controller matches
the compiled binding exactly. The registered boundary system verifies command-cause ingress
and updates the authoritative runtime record, so accepted ticket commands
become boundary-visible intents without an append-only side component.

The law extension creates a Human controller when the compiled seat controller
does not exist. A host may instead pre-register the same stable controller ID
with Utility, Rule, Random, External, or LLM policy identity. The existing
controller must keep the exact compiled authority, seat, permission profile, and
command subject; `canwu-law` preserves that compatible binding rather than
silently replacing the host's selector. Random controllers are resolved through
an operation-keyed boundary draw, while External and LLM controllers use the
ordinary strict decision request/response DTOs. Neither path grants the selector
new legal options or command authority.

Procedure stages tally integer vote weight. A seat weighs its entry in
`seat_weights`, or 1 when absent, so an empty map is the equal-seat count; an
explicit weight of 1 is removed when the plan compiles. `quorum` is the minimum
summed weight of seats that cast any ballot, and `threshold` is the per-mille
share of `For` among `For` plus `Against` weight; vetoes are seat powers and are
never weighted. When `block_of_seat` assigns every seat of a stage to exactly
one unit block, the stage also needs `block_threshold` blocks in the `For`
position. A block's position is the weighted majority of its seats; an evenly
divided block, or one without counted ballots, takes no position. When as many
blocks are `For` as `Against`, the procedure's `deterministic_tie_break`
applies. `status-quo` adds no block, so a tied stage passes only if
`block_threshold` is already met and otherwise waits for more ballots or its
deadline. `casting-seat:<seat>` adds one `For` block when that seat's own
ballot in the stage is `For`; the seat must belong to every blocked stage, and
each such stage's `block_threshold` must exceed half its blocks, so a tie never
passes without it. Stages are evaluated over the ballots admitted so far, so
later ballots can break or create a tie before the deadline. Weights, blocks,
and the tie-break are validated when the plan compiles, and unused fields are
omitted from JSON so existing plans keep their hashes.

When a stage stops accepting ballots (it passed, completed, or its procedure
closed), its pending or enqueued seat work expires, later preparation or
acknowledgement of that work is ignored, and a seat response that still
arrives is recorded as a rejected intent outcome instead of failing the
boundary.

A `consultation` stage is advisory. Its seats receive tickets whose context
carries `"advisory": true`, and their ballots persist as participation records,
but they never count toward completion, veto, or adoption. The stage completes
at the first legal boundary after its deadline, expires its unanswered seat
work, and opens the next stage; zero ballots is a valid outcome. A procedure
that still lacks its capacity reservation at that deadline expires instead. A
consultation needs a positive `deadline_minutes`, no quorum, threshold, weights,
or blocks, and cannot be a procedure's final stage.

All authored collections are canonically sorted, stable IDs are preserved,
dense plan-local keys are deterministic, and hashes use Canwu's canonical hash
contract. Law-local `LegalRecordRef` values name records inside the aggregate;
they deliberately do not claim kernel-issued domain-record version provenance.
The plugin registers and persists its own plan, directory, shard, and
archive-head records (`law_record_schemas()`); legal objects live inside them,
not as kernel records of their own. Host-owned
`expected_versions` remain genuine Canwu version references, while
`expected_rule_head` provides the law-local compare-and-set guard for rule
changes. A host version must be its record's current version, read from the
committed provenance index: a proposal admitted after its host record changed
is refused, and a pending proposal whose host record changed is rejected and
its procedure closed at the next boundary that works on the procedure.
Budgets, exact immutable references, and derived indexes are validated
before a plan or persisted ledger is accepted. Every procedure seat compiles to
one unambiguous institution/holder/permission/controller binding. Every legal
signal kind compiles to an exact provider plugin and packet type. When a
boundary admits a packet that the engine matches to a compiled provider's
metadata and kernel-committed producing boundary, the admission system records
it, with a hash of its due time and payload, in the bounded legal provider
index (`max_provider_packets`, default 256; it keeps the newest packets and
forgets the lowest ingress ID first, so a boundary that admits more provider
packets than the index keeps forgets the oldest of them before its own legal
work runs; each kept packet costs about 9 µs of every boundary that settles
legal work, so a host that cites packets long after admission raises it
knowing that). A legal
mutation at that or any later boundary cites provider evidence by naming a
recorded packet: every run records the same packets, sealed or not, so a sealed
run, an unsealed run, and exact replay decide alike, and a recorded packet is
an identity-evidence dependency, so a seal keeps its receipt. A citation of a
packet the index never recorded or has forgotten, even one admitted by the
same boundary, is refused with `canwu.law.ingress_rejected.v1`. Direct host injection into
that namespace is not provider evidence. Admission never trusts
caller-declared signal kinds; publicity events additionally require the
recorded hash of the exact provider payload envelope bound to proposal, time,
medium, and scope. Ordinary
settlement reserves a conservative serialized-state growth charge and visits
only pending intents, dirty or due procedures, staged contexts, due-time
versions, pending outbox work, and dirty rules; it does not rescan historical
law state. Applicability queries require a legal-order partition and exact profile,
then run the compiled `scope -> jurisdiction -> validity -> conflict` pipeline
over bounded indexed candidates. Conditions and exceptions are compiled
predicates with one exact evidence ref per asserted fact; actor-relative queries
also bind a `KnowledgeReadCut` plus one holder record per fact.
`query_applicability_with_host` verifies the holder, complete cut, compiled
knowledge schema, JSON boolean pointer, value, and evidence against Canwu.
Jurisdiction reachability builds one budgeted set from
compiled relation-kind/direction adjacency. Resolved conflicts carry exact
governing/displaced version partitions rather than selecting a winner at query
time. A total query-work budget covers graph edges, rules, historical versions,
effect/predicate visits, and conflict fan-out. A per-record nested-item budget
preflights proposal descriptors, cases, rulings, and conflicts before traversal.
Clause, jurisdiction, projection, and mutation budgets reject fan-out before
state mutation. Active conflict partitions are merged simultaneously; a version
governed by one and displaced by another yields `Contested`, independent of IDs.

Source profiles explicitly bind authority, origin, publicity, claimant,
evidence, and retrospective-effect policies. Institutional competence is
default-deny across order, jurisdiction, subject matter, source mode, operation,
procedure, forum, and adjudicative power. Required publicity is a separate
immutable event bound to a compiled signal provider and the canonical host
ingress time; future publication plans are not accepted as occurred events.
Validity-condition publicity is checked before adoption: a final stage that
passes without it keeps its ballots and waits, and recording the publicity
makes the procedure adoptable at that boundary; its deadline expires it
otherwise. A guard that can never hold (an effective time passed without an
authorized retrospective date, an unauthorized retrospective date, a publicity
the profile does not admit, an effectiveness publicity after the effective
time) rejects the proposal at once, as does an adoption past the source,
law-version, or rule budget.
Effectiveness-condition publicity may follow adoption, but the adopted version
remains inert and inapplicable until the immutable publicity event exists, and
that event must occur no later than the effective time. Source and law-version
records remain create-only; delayed publicity is derived through the proposal's
event link rather than rewriting either version. Cases, findings, and
rulings are case-bound and constrained by compiled forum, proof, standing,
remedy, precedent, interval, issue, and competence contracts. Agreed sources
bind an exact instrument kind, canonical parties, and ratification evidence;
received transforms bind a compiled target clause.

Culture dependencies cite an exact target generation and distinguish adoption
evidence from a live-level dependency. Idempotent retirement removes only
culture hot work: adoption evidence, enacted sources, and law versions stay
addressable, while an operative or future-scheduled live-level dependency
blocks retirement.

Retirement is explicit maintenance work rather than ordinary settlement.
Format 8 persists target-keyed culture-dependency records and requires the
culture owner plus every registered dependency resolver to submit an
owner-scoped proposal. The kernel validates the complete set against one
persistent domain root and commits all participants or none. Dependency fan-out
inside each record remains bounded by `max_evidence_per_record` and
`max_nested_items_per_record`.

Format 8 persists independently versioned plan, directory, order/jurisdiction,
coordinator, culture-dependency, and archive-head records. The plan record
holds only the compiled plan, its hash, and its budgets; it is written once,
when the plan is installed, and declares no evidence: the ledger counters live
in the coordinator shard and the identity-evidence declaration in the
directory. An order or jurisdiction shard (and its archive head) is created
when it first holds something, so a plan of 1,555 jurisdictions starts with
six law records rather than 3,118, and a boundary that settles legal work
decodes the plan once and writes only the directory and the shards it loaded
(measured on that plan: 9.4 ms a law boundary, from 101 ms). Closed history
moves behind authenticated archive roots while current enacted effects stay in
the hot projections. Kernel domain/decision state uses structural sharing,
boundary proposal overlays validate only affected closures, and paged
checkpoints emit content-addressed deltas.

Archive retention uses a COW root handoff. Pending work protects only its new
object delta and proposed current pages while the prior committed root protects
older history. Commit moves the prior object closure into the new root, retires
the superseded root, and leaves completed handles metadata-only; historical
law and its current effects remain reachable through the replacement root.
Synchronous commits authenticate the retention handle and the complete
directory, membership, temporal, and blob closure before changing runtime
state, apply on a clone, and publish the clone only after store finalization.
Canonical boundaries persist one terminal outcome per retention handle.
`finalize_legal_archive_retention` derives that outcome from the reloaded Canwu
runtime, finalizes the store idempotently, and queues an internal acknowledgement
that retires the recovery record on the next boundary.

Run the release probes documented in
`docs/benchmarks/format8-2026-08-30.md`. They cover one million archived legal
versions, one million domain records and decision locators, bounded compaction
selection, root-only restart, GC reachability, and a real Canwu boundary with a
16,384-candidate, 38.40-MiB admissible shard. The separate one-million-candidate
selector stress fixture is 2.49 GB and deliberately exceeds the production
128-MiB legal state/memory ceilings; it proves selector work shape, not live-save
admissibility.

Expected refusals and full budgets never fail a boundary. A law packet that
does not decode, exceeds the boundary's legal ingress budget (wakes do not
count), conflicts with another packet for the same outbox sequence or retention
handle, cites evidence or provider packets the rules above do not accept,
targets a proposal that is no longer deliberating, would exceed a lifetime
budget, or that the legal runtime refuses, changes nothing and is recorded as a
`canwu.law.ingress_rejected.v1` event; the boundary settles the other packets.
A refusal found before the runtime changed costs no reload or copy; only a
failure after a change (an archive commit, a non-procedural adoption that
fails after its reservation, or a broken invariant) restarts the
pass from the runtime loaded again. A pending proposal whose host
compare-and-set no longer holds, whose culture dependency was retired, whose
adoption guard can never hold, or whose adoption would exceed a lifetime budget
is rejected with a `canwu.law.proposal_rejected.v1` event. A refused archive
commit records a `RejectedStale` retention terminal whose reason names the
refusal, so the host finalizes the handle as for a stale batch. An exact record
version cited as evidence must be its record's current, initial-scenario, or
same-boundary proposed version.

Settlement takes at most `max_mutations_per_boundary` units of work (plus at
most the first unit of each kind) in a fixed order; work past it stays pending
and the plugin schedules a zero-delay continuation wake, so the next boundary
at the same time settles the next slice, and `advance_canonical` settles them
all before time advances. The check that rejects stale pending proposals
before settlement visits at most `max_mutations_per_boundary` dirty
procedures, in the order settlement selects them; a dirty procedure after the
last one checked is not advanced at that boundary and waits for the
continuation, so a pile-up of dirty procedures costs one check per procedure
per boundary that reaches it. Work whose state growth does not fit waits for state
an archive compaction frees. What can still fail a boundary: an invalid or
missing persisted legal record, a broken runtime invariant or counter
overflow, and a read of evidence outside what every run holds
(`EvidenceContentUnavailable`), which is an engine-rule violation.

## Aggregate caseload

A plan may declare a `CaseloadProfileDefinition`: the case classes a host
reports in aggregate, the review levels whose queues it tracks, how many
settled periods each jurisdiction keeps, and how many reports one boundary
settles. A host queues `caseload_intake_v1` packets (`enqueue_caseload_intake`)
carrying `CaseloadIntakeReport`s: per jurisdiction and period, the intake and
resolved counts per class and the cases that entered and left each review
level. The law admission system settles them in order into
`AggregateCaseloadRecord`s: it carries the pending stocks from period to
period (opening plus intake minus resolved per class, opening plus entered
minus concluded per review level) and computes the clearance rate (resolved
over intake, in basis points). Periods of a jurisdiction are contiguous; the
first report may state the opening stocks, and a later one may state them to
check them. A report that repeats the latest settled period is already
settled, one that names it with other content conflicts (both decided before
its evidence is looked at, comparing evidence as data, so a retry at a later
boundary is already settled even when its evidence can no longer be cited),
and a report that would leave a negative stock, names an unknown jurisdiction,
class, or level, cites more than 16 evidence references or 2,048 bytes of
them or unavailable evidence, or comes past the boundary's report budget is
refused: each refusal changes nothing and is recorded with
`canwu.law.caseload_rejected.v1`, whose summary names a stable reason code.

The aggregate path does not load the legal runtime and its records do not count
toward the legal state budget: per jurisdiction, one `LegalCaseloadHead` and
one record per kept period, in `retained_periods` slots that a new period
overwrites, so a report writes two small records whatever the retention
(measured: 1,400 jurisdictions with eight classes and three review levels
settle in one boundary in 60 to 100 ms, steady with twelve kept periods). The
plan bounds them: a profile compiles only if, summed over the plan's
jurisdictions, one head plus `retained_periods` records at the widest a
report can make (`CaseloadProfileDefinition::max_state_bytes`) stays within
`MAX_CASELOAD_STATE_BYTES` (256 MiB). The
records cite their evidence as data and declare none, so they never hold a seal
back, and every decision reads committed caseload records, the plan, and the
boundary's admitted packets, so a sealed run decides as its replay does.
`load_legal_caseload` reads one jurisdiction's kept periods.

A host chooses per class of cases: classes it reports in aggregate are the
profile's, and a case it wants to follow on its own (a party a player knows,
an appeal to the capital) it records as a `LegalCase` too, still counting it in
its class's intake so the aggregate stays the complete count. A case can cite
the jurisdiction's current caseload period record version as evidence; since
a slot record is overwritten by later periods and its ID names only its slot,
the case should carry the period and its ordinal in its own data.

This crate is experimental and intentionally does not implement natural
language interpretation, universal moral judgment, or a dense person-by-rule
cross-product.
