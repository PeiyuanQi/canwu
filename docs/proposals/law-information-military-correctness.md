# Law, Information, and Military Correctness

Status: implemented in 0.16.0, in `canwu-law`, `canwu-information`, and
`canwu-military`, with two read-only `SimulationView` additions in
`canwu-sim`. Independent reviewers of this run found four defects in the
three extensions; this package fixes them and the further defects an audit of
the same crates and a review of the first fix found. Two rules from the 0.15.0
reviews govern every item:

- **Never fail a boundary on an expected refusal.** A failed boundary cannot be
  stepped past and its due ingress cannot be cancelled, so the run stops for
  good. A refusal of host input is a recorded rejection, a skipped packet with
  a visible event, or a deferral; a full budget is a refusal at admission or a
  deferral, never a failed boundary.
- **Seal and retention independence.** A boundary decision must not depend on
  retained evidence. A seal removes evidence that exact replay keeps, so a
  decision that reads it differs between a sealed run and its own replay. The
  0.16.0 continuation package (`docs/proposals/continuation-checkpoint.md`)
  states the rule as R1 to R5 and lets a seal pass pending ingress; every fix
  here holds under the 0.15.0 seal and under that one.

The contracts this package changes are
[legal institutionalization](legal-institutionalization-framework.md),
[information flow](information-flow-mechanism.md), and the
[military domain design](military-domain-design.md).

The durable tests are:

- `crates/extensions/canwu-law/tests/legal_ingress_refusal.rs`:
  `a_stale_host_compare_and_set_refuses_the_proposal_not_the_boundary`,
  `refused_legal_packets_change_nothing_and_their_neighbours_settle`,
  `a_deadline_pile_up_settles_over_continuation_boundaries`, and
  `a_lifetime_budget_refuses_at_admission`;
- `crates/extensions/canwu-law/tests/law_contract.rs`:
  `provider_citations_decide_alike_in_sealed_and_unsealed_runs`, which replaces
  `active_legal_provider_evidence_survives_compaction_as_a_compact_proof`;
  `live_non_procedural_source_requires_a_recorded_compiled_provider_packet`
  (formerly `live_non_procedural_source_requires_exact_compiled_provider_ingress`),
  `passed_bills_that_can_never_be_adopted_are_rejected_at_once`,
  `work_past_the_state_budget_waits_without_any_hot_path_mutation` (formerly
  `boundary_state_budget_fails_before_any_hot_path_mutation`),
  `refused_durable_legal_archive_ingress_records_a_terminal_that_releases_its_retention`,
  and the rewritten
  `required_publicity_is_an_adoption_guard_that_waits_and_retroactivity_is_explicit`;
- `crates/extensions/canwu-information/tests/runtime_adapter.rs`:
  `delegated_interpretation_decides_alike_after_its_delegation_is_sealed`,
  `unusable_or_conflicting_operations_in_one_boundary_do_not_fail_it`,
  `a_release_to_an_audience_with_a_dead_member_publishes_to_the_living`,
  `a_release_to_ineligible_entity_holders_publishes_to_the_eligible`,
  `two_deliveries_of_one_representation_to_one_holder_publish_once`, and
  `a_crafted_packet_for_a_persisted_operation_does_not_displace_its_progress`;
  the lifecycle unit test `content_source_edges_and_reference_roles_must_match`
  checks the refusal of a territory audience member;
- `crates/extensions/canwu-military/tests/lifecycle.rs`:
  `reconnaissance_draws_from_its_admitted_command_and_replays_exactly`.

Each asserts behaviour the base of this package (the 0.16.0 integration branch
before it: 0.15.0 with the knowledge, fiscal and society, resource, and
production packages) does not have.

## Legal refusals are recorded, not boundary failures

### Problem

`canwu-law`'s admission system settles every admitted law packet in one pass
and returned the first error it met, which failed the boundary. Most of those
errors are refusals of host input:

- A proposal's `expected_versions` compare-and-set a host record. When the
  record changed between the enqueue of a `SubmitProposal` or
  `AdmitNonProceduralSource` and its admission, the check failed the boundary.
  The same check ran again for every signal and, before settlement, for every
  procedure the boundary worked on, so a host record changing while a
  proposal deliberated failed every later boundary that touched the
  procedure.
- A packet that did not decode, a host-authored wake for a time that had not
  come, a duplicate proposal ID, a stale actor context, a conflicting outbox
  preparation or acknowledgement, a host-authored intent without its command,
  more law packets than `max_mutations_per_boundary`, and every error a legal
  runtime operation returned for one mutation failed the boundary too.
- A signal for a proposal that was no longer deliberating reached settlement,
  which expired or re-tallied the closed procedure and could fail.
- A final stage that passed before its validity publicity was recorded, or
  with an unauthorized retroactive date, failed settlement preflight, and so
  failed every boundary that worked on the procedure.

Two reads also depended on retention. The compare-and-set read
`domain_record_version_is_current`, which the 0.15.0 rules forbid in a
boundary-phase reader. An exact record version cited as evidence was checked
with `evidence_exists`, which for a version that is no longer current answers
from retained evidence or a kept receipt.

### Decision

**Every law packet is a unit of work that is applied or refused on its own.**
The admission system decodes each admitted law packet and checks what it can
check alone (payload shape, archive retention binding, an intent's command,
a wake's due time). It then groups the packets into units in the canonical
order the 0.15.0 pass used (archive commits by token, retention
acknowledgements by handle, wakes by due time, mutations in admission order,
actor contexts by seat, outbox preparations and acknowledgements by sequence,
intents by ID). Packets that repeat a wake time, outbox sequence, or retention
handle join one unit; one that conflicts with the unit is refused. Packets
count against `max_mutations_per_boundary` in admission order and one past it
is refused; a wake carries no input of its own and does not count.

Each unit is first checked against the view and the working runtime without
changing either (evidence, compare-and-set, provider provenance, outbox and
decision state), then applied by the legal runtime. A unit that fails is
refused: its packets change nothing and each is recorded with a
`canwu.law.ingress_rejected.v1` event naming the packet type, ingress ID, and
reason.

**A refusal found before a change costs nothing more.** Every legal runtime
operation the admission system applies, except an archive commit, checks its
input and its lifetime budgets and reserves its serialized-state growth (always
positive) before its first change. A failure that left the reservation where
it was therefore changed nothing, and the pass simply goes on without the
unit. A failure after a change (an archive commit, a non-procedural source
whose adoption fails after its reservation, or a broken invariant such as an
evidence dependency count overflow) restarts the pass from a copy of the
runtime taken once after loading, without that unit; the runtime is loaded
from the view once per boundary. When every unit is refused, or the run has
no legal plan installed, the boundary does no legal work beyond recording the
refusals.

**The host compare-and-set reads the committed current version.** A host
version holds when `SimulationView::current_domain_record_version` names it
exactly. That read comes from the boundary's proposals or the committed
provenance index, never from retained evidence. A new proposal whose host
version is stale is refused. Before settlement, a pending proposal the
boundary works on whose host version is stale, or whose culture dependency
was retired, is rejected, its procedure closed, its open seat work expired,
and its staged actor contexts dropped, recorded with a
`canwu.law.proposal_rejected.v1` event. A failed law-local head
compare-and-set already rejected a proposal at adoption the same way.

**A signal is admitted with its packet.** `admit_signal_within_boundary`
requires the proposal to be `Submitted` or `Deliberating` with an open
procedure and the signal to fit the text and evidence budgets, reserves its
growth, then adds its evidence and kind to the proposal and marks the
procedure for settlement; the plugin also requires the proposal's host
compare-and-set to hold. Settlement no longer receives signals from the
plugin. A signal admitted before a pre-settlement rejection of its proposal
stays recorded on the rejected proposal.

**A passed procedure that can still be adopted waits; one that cannot is
rejected.** When a final stage passes, the adoption guards are checked in
this order. A failure time cannot cure (an effectiveness publicity dated after
the effective time, a publicity under a profile that admits none, an effective
time that passed without an authorized retrospective date, an unauthorized
retrospective date) rejects the proposal at once, closes the procedure, and is
recorded with a `canwu.law.proposal_rejected.v1` event. A missing or later
validity publicity keeps the procedure open with its ballots, as a missing
required signal already did; recording the publicity marks the procedure for
the next settlement, and its deadline expires it otherwise. An adoption that
would exceed `max_sources`, `max_law_versions`, or `max_rules` rejects the
proposal the same way. `LegalRuntime::settle_boundary` returns these
rejections in `LegalBoundaryResult::rejected_proposals`.

**Cited record versions must be replay-stable.** An exact record version cited
as evidence counts only when `replay_stable_domain_record_version` resolves
it: the record's current version, a version proposed earlier in the boundary,
or an initial-scenario version. Any other version is refused in every run.
Ordinal evidence (events, commands, attempts, ingress, boundaries, draws) is
checked with `evidence_exists`.

**An archive commit always reaches a terminal.** `commit_verified_legal_archive`
compares the source membership root before the candidate index, so a batch
prepared against legal state that has since changed settles as the recorded
`RejectedStale` terminal the host can finalize. Any other refused commit is
recorded as a `RejectedStale` terminal whose `reason` names the refusal, so
`finalize_legal_archive_retention` releases the store lease as for a stale
batch. A batch whose members are already archived under its receipts, or whose
handle already holds a terminal, records nothing more: its lease is not
released. When the terminal itself does not fit (a full state budget), the
refusal still names the retention handle, which the host abandons in its store
(`abandon_legal_archive_retention`). A retention acknowledgement error is a
refused packet.

**A packet that names an unheld shard is refused.** A mutation whose legal
order or jurisdiction the plan does not compile names a shard the legal
directory does not hold; loading it would fail the boundary, so the unit is
refused before the runtime is loaded. A wake needs no route of its own: the
boundary loads the shards of all pending work (below).

### Invariants

- A refused packet changes no legal state; the refusal event, and for an
  archive commit its terminal, is the only trace besides the packet itself.
- The outcome of a unit does not depend on which units after it are refused:
  a unit refused after a change is removed and the pass replayed from the
  loaded runtime.
- No law decision reads a retained payload, a retained provenance, or a
  non-current record version body.

### Persistence, replay, and format

Admission declares the two event types it emits, and `LAW_SEMANTIC_HASH`
changes. A refused archive commit's terminal uses the existing `reason` field.
For the same input, a 0.16.0 run records refusals, rejected proposals, and
waiting procedures where 0.15.0 failed the boundary. Exact replay re-runs the
same decisions.

### Compatibility

Hosts that relied on a failed boundary to learn of a refusal read the events
instead. A non-current record version cited as evidence is refused in every
run. A signal for a proposal that is not deliberating is refused. Budget
counting no longer includes wakes. A passed proposal whose effective time has
passed without an authorized retrospective date is rejected even while its
validity publicity is missing.

### Cost

A boundary loads the legal runtime once and copies it once, after loading.
A refusal found before a change adds only the unit's own checks.

How measured: a temporary probe, not kept as a test, built the registry
fixture of `legal_ingress_refusal.rs` with `max_mutations_per_boundary = 500`
and `max_procedures = 2,000`, opened 1,000 proposals through the runtime API,
hosted them as the initial legal records, and timed one `settle_boundary` call
(wall clock, debug build, Windows, 16 cores shared with other builds) for each
workload. One boundary of 500 duplicate submissions took 0.42 to 0.48 s,
against 181 s when every refusal reloaded the runtime from the view (the
measurement that prompted this change, on the first fix of this package); one
boundary of 500 admitted submissions took 4.2 to 4.4 s.

The worst case is a unit refused after a change: it restarts the pass from the
copy (9 to 12 ms in that debug run for 1,500 proposals) and re-applies the
units before it, so a boundary with `k` such refusals among `n` applied units
re-applies at most `k * n` units, with `k + n` at most
`max_mutations_per_boundary`. Only archive commits, a non-procedural adoption
that fails after its reservation, and broken invariants reach it; treating
every refusal of the probe that way took 4.4 s for the 500 duplicates. The
pre-settlement compare-and-set
reads one provenance-index entry per host version of each procedure the
boundary works on.

## Settlement budgets defer work

### Problem

Settlement preflight failed the boundary when the boundary's accepted work
exceeded `max_mutations_per_boundary`, a lifetime budget (`max_participations`,
`max_intent_outcomes`, `max_sources`, `max_law_versions`, `max_rules`,
`max_outbox`), or the state budget. Many procedures reaching their deadline
together, or a run that filled a lifetime budget, therefore stopped the run.

### Decision

**Per-boundary budgets defer work to a continuation boundary at the same
time.** Settlement selects its work in a fixed order: pending intents whose
procedure is not worked on, procedures due for work by ID (each with its
pending intents), due law versions by effective time, ordinal, and ID, then
dirty rules by ID, whose applicability refresh is also held to
`max_applicability_entries_per_boundary`. It takes units while they fit
`max_mutations_per_boundary`, stops at the first that does not (first in,
first out), and always takes the first unit of each kind, so every boundary
makes progress and exceeds the budget by at most one unit of each kind. Work
not taken stays pending in committed legal state, and
`LegalBoundaryResult::deferred` is set. The admission system then schedules a
`legal_due_work` wake with zero delay, which the next boundary at the same
time admits; that boundary loads the shards of all pending work (the routes of
dirty and due procedures, of the procedures of pending intents, of due
versions, and of dirty rules) and settles the next slice.

The chain is bounded: each continuation boundary settles at least one pending
unit, so it ends within as many boundaries as there were pending units, in
practice `ceil(units / max_mutations_per_boundary)`. It is replay-exact: the
slice is a function of committed state, and the wake is boundary-generated
ingress in the canonical record. `Canwu::advance_canonical` settles every
continuation before it advances time, because each wake is due at the current
time; `Canwu::step_canonical` returns after each one, at the same time. A wake
is pending ingress like any other: a 0.15.0 seal, which requires an empty
ingress queue, waits until the continuation boundaries at that instant have
run; under the continuation package a seal may pass it, and settling the
deferred work reads only committed legal state.

**Work that does not fit the state budget waits for freed state.** A unit whose
serialized-state growth does not fit is not taken and does not set
`deferred`, so no continuation is scheduled for it; the next boundary that
settles law work after an archive compaction frees state takes it. A wake
whose own tracking does not fit is not scheduled; its work settles at the next
boundary that settles law work.

**Lifetime budgets refuse at admission or reject at adoption.** A submission
past `max_procedures`, an intent past `max_pending_intents` or the
participation and intent-outcome budgets, a source past `max_sources`,
`max_law_versions`, or `max_rules`, and a case, finding, ruling, conflict,
succession, or retirement past its budget are refused packets. A passed
proposal whose adoption would exceed `max_sources`, `max_law_versions`, or
`max_rules` is rejected (see above). The outbox stops growing while it or its
ID blocks are full; the seat work waits.

### Invariants

- No settlement budget fails a boundary.
- Every boundary that settles law work settles the first pending unit of each
  kind whose growth fits the state budget.

### Persistence, replay, and format

No persisted shape changes; the continuation wake is the existing
`legal_due_work` packet.

### Compatibility

`LegalRuntime::settle_boundary` called directly no longer fails on a
per-boundary budget: it settles a slice and reports `deferred`, and the caller
settles again.

### Cost

Selection scans the pending work once per boundary and projects each unit it
considers once, so a chain of continuation boundaries scans the remaining work
once per boundary. The pre-settlement rejection of stale proposals
(`reject_stale_pending_proposals`) is not budgeted: every boundary that settles
checks the host versions of every dirty procedure. A pile-up of `D` dirty
procedures under budget `B` settles over about `D / B` boundaries, so the
chain makes about `D^2 / B` procedure checks in all (each one provenance-index
read per host version), with `D` bounded by `max_procedures`. A boundary that
settles deferred work in shards its units did not name loads the runtime a
second time with those shards.

## Provider provenance from the legal provider index

### Problem

A legal signal, a non-procedural source's required signal kinds, and a
publicity event cite provider packets (`EvidenceRef::Ingress`) that the
compiled provider plugin generated. Admission checked them with
`plugin_ingress_matches` and `plugin_ingress_payload_matches` on whatever
packet was cited. After a seal those reads answer from a kept receipt, a
provenance fact, or not at all: a packet first cited after the seal that
archived it returned `Ok(false)` in 0.15.0 and `EvidenceContentUnavailable`
under the continuation package, and the error failed the boundary through
`verify_legal_mutation(...)?`. The unsealed run, which retains the packet,
accepted the same citation.

### Decision

**The admission system records every compiled provider packet its boundary
admits, and citations are checked against that record.** When a boundary
admits a packet whose plugin and packet type match a compiled signal provider
and the engine confirms that the provider plugin's own boundary generated it,
the law admission system records `(ingress ID, plugin, packet type, payload
hash)` in the legal provider index, `LegalRuntime::provider_packets`, in the
coordinator shard. The payload hash is
`canonical_hash("canwu.law.provider-payload.v1", &(due_at, payload))`, which
the new engine read `SimulationView::plugin_ingress_payload_hash` returns for
a packet the boundary admits without exposing its payload. A signal or a
required signal kind counts when one cited ingress is recorded with the
kind's provider plugin and packet type; a publicity event counts when one
cited ingress is recorded with its provider and with the hash of the exact
envelope the event records. A citation may come at any later boundary.

Every run admits the same packets, so the index is the same in a sealed run,
an unsealed fork, a restore from sealed segments, and exact replay, and a
citation decides alike in all of them. Each recorded packet is an
identity-evidence dependency of the legal plan record while it is recorded, so
a 0.15.0 seal keeps its receipt and the cited ingress still exists for
`evidence_exists`.

**The index is bounded.** It keeps at most `max_provider_packets` packets
(default 4,096; zero is refused at compile time) and forgets the packet with
the lowest ingress ID first, releasing its dependency. A citation of a packet
the index forgot, never recorded (admitted before the plan was installed, or
when its record did not fit the state budget), or that is not a provider
packet is a refused packet. A boundary that admits only provider packets
records them and settles no law work.

Alternatives rejected: refusing every packet an earlier boundary admitted (the
first fix) made hosts cite within one boundary; having the host helper attest
provenance into a permit-gated packet would make `legal_mutation` internal and
move a seal-dependent read to the host.

### Invariants

- Whether a citation is provider evidence is a function of committed legal
  state, which every run derives from the same admitted packets.

### Persistence, replay, and format

The coordinator shard gains `provider_packets` (defaulted when absent); the
plan budgets gain `max_provider_packets` (defaulted when absent), which
changes every compiled plan's content hash. The plan record's identity-evidence
declaration lists recorded provider packets.

### Compatibility

A citation of a provider packet admitted before the plan was installed is
refused. A host that cites a packet more than `max_provider_packets` provider
packets later is refused.

### Cost

A boundary that admits no packet of another plugin pays nothing. One that
does reads the provider pairs from the plan record without decoding the plan
and asks the engine once per admitted packet and provider; a recorded packet
adds one canonical hash of its payload and one coordinator-shard write.

## Delegated interpretation authority from recorded delegations

### Problem

An interpretation with `InterpretationAuthority::Delegated` cites the command
that granted the delegation. `canwu-information` read that command's payload
(`SimulationView::command`) when it advanced the operation, which may be long
after the delegation. A seal that archived the command made the read fail, and
the operation was rejected with `invalid_authority`, while an unsealed run
accepted it.

### Decision

**Record the delegation when its command is admitted.** The lifecycle system
(phase 7) looks at the commands each boundary admits. For a command that a
`DelegationEvidenceSelector::Command` grant names (producer plugin and command
type), it stores the claim found at the grant's claim path in a new
create-only record `DelegatedAuthority` (`canwu.information/delegated_authority`,
payload `DelegatedAuthorityPayload { authority_grant, command, claim }`), keyed
by `delegated_authority_reference(grant, command)`. A command whose payload has
no decodable claim is not recorded. The commands a boundary admits are held by
every run. An interpretation citing a delegation command is checked against
the recorded claim, from state or from the records the same pass creates; the
check is otherwise unchanged, and citing an unrecorded command is refused with
`invalid_authority` as before.

An ingress delegation resolves only a packet the current boundary admits; the
lifecycle system checks that against the boundary's admitted packets itself.
The same pass makes the authenticity binding of an interpretation read the
current version through `current_domain_record_version` instead of
`domain_record_version_is_current`.

Alternatives rejected: checking the delegation when the operation is admitted
still reads an arbitrary earlier command; declaring the command
`PayloadRequired` is impossible because the information plugin learns of it
only when a later operation cites it, after a seal may have archived it.

### Invariants

- A delegated interpretation is decided from committed state and the
  boundary's own admitted commands and packets.
- The planner's record set does not include recorded delegations.

### Persistence, replay, and format

A new record kind, owned and written by the information plugin. The records
are kept for the run: one per admitted command that a command grant names,
never retired, so they grow with the number of such commands (no loop reads
them; a claim authorizes only interpretations dated inside its
`[not_before, expires_at)` window). The plugin's semantic hash changes and
`information_record_schemas` returns 13 schemas. A run whose delegation
commands reach it through legacy direct commands cannot submit information
operations at all (they require canonical ingress), so every recorded
delegation comes from a boundary that runs phase 7.

### Compatibility

A delegation command admitted before this version is not recorded; same-version
policy applies.

## Reconnaissance and military draw evidence

### Problem

`MilitaryCommand::Recon` made an operation-keyed draw citing the current
boundary as its operation evidence. The current boundary is not evidence yet,
so settlement refused the draw in every run and reconnaissance was always a
recorded command rejection. The special-operation and combat-round draws of
plugin ticks cited the first admitted ingress of the boundary, whatever plugin
it belonged to, with the current boundary as a fallback that would be refused
the same way.

### Decision

Every operation-keyed draw cites the admitted packet that carries the command
or tick being applied (`EvidenceRef::Ingress`). That packet exists in every
run, so the draw is accepted alike in a live run, a sealed run, and exact
replay. The draw's value does not depend on its evidence, so tick outcomes do
not change; only the recorded operation evidence does.

### Persistence, replay, and format

Reconnaissance now records a draw and an `Accepted` outcome. Tick draws record
different operation evidence. The plugin's semantic hash changes.

## Information audit

### Finalization reads the knowledge ledger

**Problem.** Finalization collected the knowledge-record IDs a publishing
boundary created with `knowledge_changes_by_correlation_prefix`, which scans
retained boundaries and failed the boundary when it found none. In 0.15.0 a
seal could not fall between publication and finalization, because the
finalization packet was pending; under the continuation package it can, and a
sealed run would then fail where an unsealed run continues.

**Decision.** The finalization packet names the chunk it finalizes
(`publication_start`, `publication_end`) and the publications of the chunk left
unpublished (`unpublished`). Finalization rebuilds the chunk's publications
from state and takes, for each published one, the newest current record of its
holder, knowledge kind, and subjects from `SimulationView::knowledge_heads`,
which reads committed state. To make that record unambiguous, the publication
system publishes each holder, kind, and subject set at most once per boundary:
a later operation of the same pass whose publication names the same holder,
kind, and subjects relies on the record the earlier one published, so the
holder learns the fact once and that record's origin names the earlier
operation. Two deliveries of one representation to one holder in one boundary
reach this; both operations then record the one record's ID. Information
records are never superseded, so the newest current record is the one the
chunk's boundary published. The finalization packet is internal ingress now;
only the publication system schedules it, so its payload is trusted, and a
mismatch is an invariant failure.

**Cost.** One index lookup per publication of the chunk instead of a scan of
every retained boundary's knowledge changes.

### Publication to a holder the engine does not accept

**Problem.** A publication to a holder the engine refuses fails the boundary:
a dead or unknown person, a missing army or government, a retired, deleted, or
non-holder domain entity, or a core entity kind that holds no knowledge
(organization, resource, route, territory). A delivery or release reaching
such a holder failed every retry of its boundary; an audience of persons and a
territory was enough.

**Decision.** A record that names a holder of a kind that holds no knowledge
is refused when it is created: an audience member, an addressed dispatch
recipient, and the holder, recipient, or performed-for role of an access,
delivery attempt, or interpretation (the operation is rejected with
`invalid_lifecycle`). At publication, the publication system asks the engine
with the new read `SimulationView::knowledge_holder_accepts_publication`,
which applies the exact holder rule the engine validates a `PublishKnowledge`
directive with, at the same state; a holder it does not accept is left
unpublished and listed in the finalization packet, and the operation
completes with the records it did publish.

The read reads person and entity state (the entity registry, people, armies,
governments, person availability, and domain records with their schemas)
without a read declaration. That is safe because the answer is exactly the
holder check the engine applies, against the same state, to any
`PublishKnowledge` directive the system proposes: a system that may publish
already depends on that state through the validation of its own output, and
the read reveals one yes or no for a holder the caller names, nothing it could
not learn by proposing the publication.

### Refused operation packets and conflicting operations

**Problem.** `information_operation_v1` is public ingress. A packet that did
not decode, two packets of one boundary with the same operation ID and
different inputs, and a packet that conflicted with the persisted operation of
its ID each failed the boundary; the second case is also reachable through two
commands in one boundary. Two operations whose plans changed the same record in
one boundary made the boundary invalid ("mutated the same record twice"). The
first fix kept the first packet of an operation ID in admission order, so a
crafted packet with the ID of a persisted operation and a higher priority
displaced the operation's own progress packet and stalled it.

**Decision.** Such a packet is skipped and recorded with a
`canwu.information.ingress_rejected.v1` event. When one boundary admits
different inputs for one operation ID, the input that matches the persisted
operation's input hash is kept, whatever its priority; without a persisted
operation the first admitted input is kept. Operations are planned oldest
first (by admission time, then operation ID) against the records the boundary
began with; one whose plan changes a record an earlier operation of the pass
already changes waits for the next boundary and is planned again there. The
oldest operation planned at a boundary is never deferred, so an operation
waits only behind operations admitted no later than it, a finite set: newer
operations cannot starve it.

### Persistence, replay, and format

The finalization packet payload gains three fields and becomes internal; the
lifecycle system declares one event type. The semantic hash changes (see
above).

## Audit of retained-evidence reads and boundary failures

Every boundary-phase read of retained evidence in the three crates, and what
it reads after this package:

| Crate | Read | Decides | Status |
| --- | --- | --- | --- |
| law | `ingress` of the boundary's admitted packets | packet content | admitted this boundary: held by every run |
| law | `command` of an intent packet's cause | the intent matches its authorized command | cause of a pending packet: held by every run |
| law | `plugin_ingress_matches`, `plugin_ingress_payload_matches` of a cited packet | provider provenance | replaced by the legal provider index (fixed) |
| law | `plugin_ingress_payload_hash` of the boundary's admitted packets | records provider packets | admitted this boundary: held by every run |
| law | `domain_record_version_is_current` | host compare-and-set | replaced by `current_domain_record_version` (fixed) |
| law | `evidence_exists` of a record version | cited evidence exists | replaced by `replay_stable_domain_record_version` (fixed) |
| law | `evidence_exists` of an ordinal reference | cited evidence exists | engine answer; see below |
| law | `decision_attempt`, `decision_ticket`, `decision_controller` | outbox preparation and acknowledgement | decision state, not evidence |
| information | `ingress` of the boundary's admitted packets | packet content, ingress delegations | admitted this boundary (checked): held by every run |
| information | `command` of a cited delegation | delegated authority | replaced by recorded delegations (fixed) |
| information | `command` of the boundary's admitted commands | records delegations | admitted this boundary: held by every run |
| information | `knowledge_changes_by_correlation_prefix` | finalization IDs | replaced by `knowledge_heads` (fixed) |
| information | `domain_record_version_is_current` | authenticity binding | replaced by `current_domain_record_version` (fixed) |
| information | `domain_record_version_evidence_exists` of an assignment | institutional authority | outcome is seal-independent: only the current or proposed version is accepted, any other is refused whether or not it is known |
| military | `ingress` of the boundary's admitted packets | packet content | held by every run |
| military | keyed-draw operation evidence | draw admission | the carrying packet (fixed) |

`SimulationView::evidence_exists` of an ordinal reference is the engine's
answer. In 0.15.0 a reference that a seal archived without keeping its receipt
does not exist, so a first citation, after a seal, of an event, command, or
ingress the law plugin never declared is refused in a sealed run and accepted
in an unsealed one. Recorded provider packets are declared, so their
citations are not affected. The continuation package answers ordinal
existence from counters, which removes the difference; a plugin cannot answer
it alike before that, so this package leaves it to the engine.

Host-side reads in the three crates (`canwu-law`'s
`validate_host_expected_versions`, `query_applicability_with_host`, and the
archive and outbox helpers) run outside settlement; their answers can differ
after a seal, but they decide nothing at a boundary.

### What can still fail a boundary

`canwu-law`:

- a missing or invalid persisted legal record (plan, directory, shard, archive
  head), a plan binding mismatch, or an invalid derived index;
- a broken runtime invariant during settlement (a missing indexed record, an
  inconsistent effective-time or deadline index), and arithmetic overflow of
  legal counters (boundary index, ordinals, evidence dependency counts);
- a read outside what every run holds (`EvidenceContentUnavailable`), which a
  correct engine never returns for the packets and causes a boundary admits:
  an engine-rule violation.

No budget, refusal, stale compare-and-set, or unadoptable proposal fails a
boundary.

`canwu-information`:

- an undecodable or invalid information record, an invalid operation state
  transition, or an internal finalization packet that does not match its
  operation (plugin invariants);
- `EvidenceContentUnavailable`, as above.

`canwu-military`:

- a plugin tick whose operation, force, or combat record is missing or
  invalid, and an undecodable military record (plugin invariants).

## Public API

- `canwu-sim` (`canwu-api` re-export): adds
  `SimulationView::plugin_ingress_payload_hash` and
  `SimulationView::knowledge_holder_accepts_publication`, both read-only.
- `canwu-law`: adds `LAW_INGRESS_REJECTED_EVENT`,
  `LAW_PROPOSAL_REJECTED_EVENT`, `LegalProposalRejection`,
  `LegalProviderPacket`, `LEGAL_PROVIDER_PAYLOAD_DOMAIN`,
  `LawBudgets::max_provider_packets`, `LegalRuntime::provider_packets`, and
  `LegalBoundaryResult::{rejected_proposals, deferred}`; `LAW_SEMANTIC_HASH`
  changes; `LegalRuntime::settle_boundary` no longer fails on a settlement
  budget or a passed final stage whose adoption guard fails;
  `LegalRuntime::record_publicity` marks the proposal's procedure for the next
  settlement.
- `canwu-information`: adds `INFORMATION_REJECTION_EVENT`,
  `DelegatedAuthority`, `DelegatedAuthorityPayload`, and
  `delegated_authority_reference`; `information_record_schemas` returns 13
  schemas; the finalization ingress is internal.
- `canwu-military`: no API change; the semantic hash changes.

No engine snapshot, commitment, or checkpoint format changes.

## Open decisions

- `max_provider_packets` defaults to 4,096 per plan. A host that cites
  provider packets long after they were admitted sizes it for the gap.
  (Resolved in 0.18.0: the default is 256, since each kept packet costs
  about 9 µs of every law boundary; see
  [law aggregate caseload](law-aggregate-caseload.md#provider-index-default-e-25).)
- `DelegatedAuthority` records are never retired. Retiring them when their
  claim window closes would need a scheduled wake, which blocks 0.15.0 seals;
  a later package can retire them through archive maintenance. (Resolved in
  0.18.0 without a wake: one versioned ledger per command grant holds its
  live delegations with an expiry index, the lifecycle system removes
  expired delegations at the boundaries it already runs, an interpretation
  admitted at or after its delegation's `expires_at` is refused even when
  it is dated inside the window, and a claim already expired at admission
  is not recorded; see
  [delegations leave their ledger when they expire](law-aggregate-caseload.md#delegations-leave-their-ledger-when-they-expire-e-25-canwu-information).)
