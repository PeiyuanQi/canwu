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

Retirement is explicit maintenance work rather than ordinary settlement. The
v1 aggregate bounds its dependency scan with
`max_retirement_dependency_records` and fails before mutation when the bound is
exceeded. The unit is outer procedure/intent/outbox/rule records; dependency
fan-out inside each record remains bounded by `max_evidence_per_record` and
`max_nested_items_per_record`. A
future sharded implementation may replace this bounded scan with a target-keyed
dependency index.

Run `cargo run --release -p canwu-law --example law_scale` to measure both idle
law-local settlement and real Canwu plugin boundaries with 1,000, 10,000, and
100,000 retained history records. The latter includes aggregate decode, kernel
transaction clone, CAS, and encode. The law-local algorithm is delta-indexed,
but the current single persisted aggregate still makes a live legal boundary
linear in serialized legal history; large campaigns require strict budgets and
event-driven cadence until jurisdiction sharding/COW is implemented. The current
1k measurement already exceeds 60 FPS and 30 FPS frame budgets, 10k is suitable
only for low-frequency turn/background work, and 100k only for offline
maintenance. A live manifest should cap retained records well below 1k and
calibrate that cap on target hardware.

This crate is experimental and intentionally does not implement natural
language interpretation, universal moral judgment, or a dense person-by-rule
cross-product.
