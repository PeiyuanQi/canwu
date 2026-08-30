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

All authored collections are canonically sorted, stable IDs are preserved,
dense plan-local keys are deterministic, and hashes use Canwu's canonical hash
contract. Law-local `LegalRecordRef` values name records inside the aggregate;
they deliberately do not claim kernel-issued domain-record version provenance.
Only the aggregate runtime record is registered and persisted. Host-owned
`expected_versions` remain genuine Canwu version references, while
`expected_rule_head` provides the law-local compare-and-set guard for rule
changes. Budgets, exact immutable references, and derived indexes are validated
before a plan or persisted ledger is accepted. Every procedure seat compiles to
one unambiguous institution/holder/permission/controller binding. Every legal
signal kind compiles to an exact provider plugin and packet type; live admission
matches retained ingress metadata and its kernel-committed producing boundary.
Direct host injection into that namespace is not provider evidence. Admission
never trusts caller-declared signal kinds; publicity events additionally require
an exact retained provider payload envelope bound to proposal, time, medium, and
scope. Ordinary
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
Validity-condition publicity is checked atomically before adoption.
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
coordinator, culture-dependency, and archive-head records. Closed history moves
behind authenticated archive roots while current enacted effects stay in the
hot projections. Kernel domain/decision state uses structural sharing, boundary
proposal overlays validate only affected closures, and paged checkpoints emit
content-addressed deltas.

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

This crate is experimental and intentionally does not implement natural
language interpretation, universal moral judgment, or a dense person-by-rule
cross-product.
