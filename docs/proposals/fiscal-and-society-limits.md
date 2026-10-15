# Fiscal and Society Limits

Status: implemented in 0.16.0, in `canwu-fiscal` and `canwu-society`. It
removes the limits of the two extensions that 0.15.0 recorded under "Known
limits carried by this release": a fiscal execution receipt or
historical-context change past the state's byte budget failed its boundary,
and so did derived fields that grew by more than 1 MiB in one boundary;
deferred fiscal report parts waited for an unrelated fiscal change, and a
revived observer's report looked complete until then; the fiscal action
packet was public ingress; and the society's owner-side ingress queue was
rewritten whole by every boundary that added to it and dropped a packet with
only an event when it was full. Two owner decisions were added to the
package: a society settles the cohort transfers due at it as a batch by
default (under a coarse cadence the 0.15.0 rule completed one transfer per
cohort per settlement), and who appoints and removes an institution's
auditors is a scenario-declared appointment policy instead of the audited
institution itself.

The 0.15.0 contracts these sections change are
[fiscal throughput and authority](fiscal-throughput-and-authority.md) and
[society settlement cadence and runtime revision](society-cadence-and-runtime-revision.md).
The durable fixtures are, under `crates/extensions/canwu-fiscal/`, the unit
tests `a_receipt_past_the_byte_budget_changes_nothing_and_is_reported`,
`a_context_change_whose_derived_fields_do_not_fit_changes_nothing`,
`an_invalid_receipt_or_context_is_refused_without_failing_the_boundary`,
`an_authored_state_whose_derived_fields_do_not_fit_is_refused_at_creation`,
and `the_largest_report_plan_fits_the_room_left_for_it` in `src/plugin.rs`, and
`tests/fiscal_report_parts.rs`, `tests/fiscal_action_ingress_origin.rs`, and
`tests/fiscal_audit_appointment.rs`;
and, under `crates/extensions/canwu-society/`, `tests/society_ingress_queue.rs`,
`tests/society_transfer_settlement.rs`, and
`a_pressure_revision_is_refused_while_the_queue_is_full` in
`tests/gap_g45_society_policy_pressure_revision.rs`.

## The fiscal state's byte budget and derived growth

### Problem

`canwu-fiscal` keeps its state in one domain record held to
`MAX_FISCAL_STATE_JSON_BYTES` (32 MiB). Settlement writes the record, and in
the same boundary the candidate system (phase 10) writes it again with the
transition candidates derived from it, and the aggregate system (phase 12)
with the aggregates and the report plan. A write past the budget is refused
by the record encoder, which fails the boundary; the packets that caused it
stay admitted and cannot be cancelled, so every retry fails too.

0.15.0 held each action to the budget less 16 KiB and rejected one that did
not fit as its own outcome. It left three ways to fail:

- An execution receipt or a historical-context change was never measured.
  One that took the state past 32 MiB failed its boundary.
- Settlement measured nothing while the state it left was more than 1 MiB
  under the budget, and the derived fields of that boundary were not part of
  the check. A change whose derived fields grow by more than 1 MiB, such as
  a historical-context change that makes thousands of transitions eligible
  in many jurisdictions, passed settlement and failed the candidate or
  aggregate system's write.
- When settlement did measure, it measured the state after the derived
  systems, not the record settlement itself writes, which still carries the
  candidates and aggregates of the previous boundary. A change that shrinks
  the derived fields could leave settlement's own record past the budget.

### Decision

**Measure every write of the boundary, exactly.** The three records of a
settling boundary differ only in the transition candidates, the aggregates,
and the report plan. Settlement encodes its own record once, as before,
derives the candidates and aggregates the two derived systems will derive
from it, and computes the size of each record from that encoding and the
encoded sizes of those fields alone. The measure is the largest of the three
sizes, without the report plan. It is exact, whichever way the derived fields
move.

**Leave out the report plan, and keep room for any plan.** The plan's size
depends on whether each observer's actor is alive in phase 12, which
settlement in phase 7 cannot know, and a later boundary may rewrite the plan
alone (see [Owed report parts](#owed-report-parts)). A valid plan lists at
most every part of every observer binding in each of its two maps, under
binding IDs of at most 160 bytes, so its serialized size is bounded by a
constant, `MAX_FISCAL_REPORT_PLAN_BYTES` (152,416 bytes). Settlement holds
the measure to `MAX_FISCAL_STATE_JSON_BYTES` less that constant. The 16 KiB
reserve of 0.15.0, which covered the plan's variation between phase 7 and
phase 12 only, is gone.

**Settle in one pass while everything fits.** Settlement applies the
boundary's packets without measuring, measures the result once, and keeps it
when it fits. Otherwise it settles the same packets again from the committed
state with every packet measured, as 0.15.0 did for actions:

- an action whose result does not fit is rejected as its own outcome with
  `FISCAL_STATE_BUDGET_EXHAUSTED`, and one whose rejection does not fit is
  dropped with its `canwu.fiscal.action_dropped.v1` event, as before;
- an execution receipt or a historical-context change whose result does not
  fit changes nothing. Its only record is one
  `canwu.fiscal.ingress_rejected.v1` event, whose summary names the ingress
  and the packet (for a receipt, its receipt ID) and ends with
  `FISCAL_INGRESS_BUDGET_REJECTION` (`state_budget_exhausted`); for a
  receipt, the event's affected entities name the institution of its
  execution request when that is a registered entity. The packets after it
  are settled on the state as it was.

**A receipt is refused, not deferred.** A receipt reports an execution that
already happened in the domain that owns the resource, so refusing one loses
something. The options were to fail the boundary, defer the receipt, or
refuse it.

- Failing the boundary stops the run for good, which no expected input may
  do.
- Deferral needs a place to keep the receipt until there is room. The state
  has no room by definition, re-queuing the packet needs a delay the plugin
  cannot choose (it keeps no calendar, and a delay of zero would retry in
  the same instant forever), and the crate has no archive, so room never
  comes back: a full state stays full.
- Refusal keeps every invariant the fiscal state owns. `canwu-fiscal` owns
  no resource balance, so nothing is created or destroyed: the resource moved
  in its own domain, and fiscal conservation (receipts never fulfil more than
  a request authorizes, an evidence version or external operation settles at
  most one receipt) holds because the refused receipt consumed nothing. The
  execution chain stays open rather than broken: the execution request keeps
  its outstanding quantity, the aggregates understate collection by the
  receipt's quantity, and the evidence versions and external operations stay
  free, so the same evidence can still settle a receipt in a state with
  room, for example the next shard of the campaign.

**No receipt or context change fails the boundary.** Both are public
ingress, so a host can send one that settlement cannot accept. In 0.15.0
each of these failed its boundary, and every later boundary with it; now
each changes nothing and is refused with the same event, whose summary names
the ingress and the packet:

- a payload that does not decode ends with `malformed_payload`
  (`FISCAL_INGRESS_MALFORMED_REJECTION`);
- a receipt whose evidence is not the current version of its record, is of a
  kind the state does not approve, names an unknown request, does not match
  its request, cites an external operation or evidence version another
  receipt settled (in an earlier boundary or earlier in the same one), takes
  its request past the authorized quantity, or reuses a receipt ID with other
  content; and a historical-context change outside the pack or one the state
  does not accept (a new year at the start of the run): each ends with the
  message that says why and `invalid_packet`
  (`FISCAL_INGRESS_INVALID_REJECTION`).

Settlement applies a receipt or context change to the state only after the
whole result validates, and rolls the one field it set back otherwise, so a
refused packet leaves the state as it was in either pass. Strict payload
schemas, which would refuse a malformed packet at enqueue, were left out:
the boundary refusal covers it, and the schemas would duplicate the packet
types' serde shapes in a second place. A state that approves an execution
evidence kind the plugin was not configured to read (`FiscalPlugin::new`) is
refused when the run is created, as an undeclared authority-basis kind is,
so that setup error is caught there rather than as a refusal of every
receipt (`gap_g41_fiscal_idle_early_exit` covers it).

Two alternatives were rejected. A reserve for receipts, holding actions to a
lower budget than receipts, cannot make receipts safe: a request may be
fulfilled by any number of partial receipts and by any number of rejected
ones of quantity zero, so no reserve covers what has been authorized, and it
would cut every decision off earlier for a protection that cannot be
complete. Recording the refusal in the state is impossible for the same
reason the receipt is refused. The event is boundary evidence, in the hash
chain and in every exact replay; a host that must account for every
execution watches for it, or for the state's size, and starts a new run or
shard before the budget.

### Invariants

- Every record a settling boundary writes is at most
  `MAX_FISCAL_STATE_JSON_BYTES`: its fields other than the report plan were
  measured by settlement against the budget less
  `MAX_FISCAL_REPORT_PLAN_BYTES`, and every plan is at most that constant.
  A unit test builds the largest plan the bound allows, 64 bindings of the
  longest ID listing every part in both maps at the largest revision, and
  holds it under the constant.
- The measure is the exact size of the largest record the boundary writes;
  a unit test settles a context change that adds 16 transition candidates
  and compares the measure with the record the boundary wrote.
- A refused receipt or historical-context change leaves the fiscal state as
  it was, procedure revision included, so no action admitted against that
  revision becomes stale on its account.
- The decision depends on the committed state, the catalog, the packets in
  admission order, and the boundary's time, so a live run, a sealed run, a
  restore, and an exact replay refuse the same packets.

### Persistence, replay, and format

This item changes no persisted shape; `FISCAL_RUNTIME_SCHEMA_VERSION` is 3
because of the [auditor appointment](#auditor-appointment-and-removal)
fields. The settlement system's contract declares one more event type,
`canwu.fiscal.ingress_rejected.v1`, so the plugin descriptor changes.

### Compatibility

- An action near the end of the budget is rejected up to 136,032 bytes
  (152,416 less 16,384) sooner than under 0.15.0: about 0.4 percent of the
  budget. A receipt or historical-context change that 0.15.0 applied in the
  last 152 KiB, or that failed the boundary past 32 MiB, is now refused with
  the event, and so is every receipt or context change that is malformed or
  invalid, which 0.15.0 let fail the boundary.
- An authored state whose first derivation would not fit is refused when the
  run is created (see below), where 0.15.0 failed its first fiscal
  boundary.
- A boundary that changes the fiscal state now always derives its candidates
  and aggregates once more in settlement, to measure them (see
  [Cost](#cost)).

### Cost

The new work is in boundaries that change the fiscal state: settlement
derives the candidates and aggregates once more and encodes those fields
before and after. Idle boundaries do none of it. Measured with
`fiscal-profile` (`benchmarks/performance-harness`), whose state has 48
institutions, 1,920 settled assessments, and 1,920 execution requests
(2,155,406 bytes of JSON), in three rounds of 31 samples that alternated a
0.15.0 build with a build of this change on 2026-10-14. The runs are recorded
as `benchmarks/baselines/2026-10-14-fiscal-boundary-0.15.0-fiscal-limits-round<n>-elapsed.json`
and `2026-10-14-fiscal-boundary-0.16.0-fiscal-limits-round<n>-elapsed.json`.
Each figure is the median of the three round medians, with the lowest and
highest in brackets.

| Operation on the profile state | 0.16.0 |
| --- | ---: |
| Settlement's measure of the derived writes (`settlement_measure`) | 6.42 ms (6.41 to 8.38) |
| One encoding of the state, for scale (`encode`) | 16.1 ms (15.2 to 18.8) |

The measure is about two fifths of one encoding of the state, which a
settling boundary performs several times over (settlement, the two derived
systems, and the engine's journal and hashes of each write). The machine was
running other builds throughout, and the boundary times moved by a third
between rounds of the same build: the boundary that settles 40 actions took
907 ms (813 to 1,230) under 0.15.0 and 752 ms (710 to 895) with the change,
and an idle boundary 1.39 ms (0.85 to 1.43) and 1.12 ms (0.86 to 1.42). They
show no difference beyond that noise, and are context, not a comparison.

When a boundary does not fit, the second pass measures each packet, as
0.15.0 did for each action within 1 MiB of the budget, and that cost is not
bounded by this change. For each receipt or context change it clones the
state, applies the packet, validates the whole result, encodes it (to a JSON
value and then to bytes, inside `encode_record`), and derives and measures
the candidates and aggregates. On the 2.2 MB profile state the parts
`fiscal-profile` records are a clone of 2.4 ms, a validation of 2.1 ms, an
encoding to a value of 16.1 ms, and the measure of the derived writes of
6.4 ms, 27 ms together; the second serialization, of the value to bytes, is
not recorded separately. The estimate is therefore 30 to 40 ms per packet,
not measured as one figure, growing in proportion to the state's size, so
roughly half a second per packet near the 32 MiB budget. A boundary near the
budget that carries `n` receipts costs `n` times that; the number of
receipts per boundary is the host's to keep small there (see
[Open decisions](#open-decisions)). An action is measured the same way,
within `MAX_FISCAL_ACTIONS_PER_BOUNDARY` (64) per boundary.

### What can still fail a boundary

In fiscal settlement, no packet: only an invalid committed fiscal record,
which activation and every write exclude, or an engine error reading it.

An authored state whose derived fields, which the first boundary that runs
the fiscal systems writes and no settlement measures, would take it past the
budget is refused when the run is created or restored: activation measures
that first write, at the widest time the boundary could have, while no
boundary has written the state yet (its record is still version 1). The
measure uses the widest time, so a state within about a hundred bytes of the
budget may be refused that its first boundary would just have fitted.

## Owed report parts

### Problem

A boundary publishes at most `MAX_FISCAL_REPORT_RECORDS_PER_BOUNDARY` (1,024)
fiscal report records, and nothing to an observer whose actor is dead. The
report plan keeps what it could not publish, by part, but 0.15.0 published
those parts only in the next boundary that changed the fiscal state. A run
whose fiscal state stopped changing never completed the reports, and an
observer revived after a death held a report that looked complete, with an
old `as_of` and no pending parts, until some unrelated fiscal change.

### Decision

**Any boundary publishes what is owed to an observer who can receive it.**
The aggregate system (phase 12), in a boundary that has not written the
fiscal state, reads the plan's `deferred` observers, and those observers'
actors, from the committed record's payload: two map lookups in a parsed
value and at most 64 person-availability reads, with no decoding of the
state. When no owed observer can receive knowledge, it returns. Otherwise it
decodes the state, plans the owed parts (at most 1,024 records, as any plan),
and writes the state with that plan; the report system (phase 13) publishes
it. Nothing else changes: the procedure revision does not advance, so no
action becomes stale, and the plan carries the revision it was computed at,
like every plan. The state is never written twice with one plan: every other
write either advances the procedure revision or writes a new plan.

**A boundary that leaves parts owed schedules the next one.** When a plan
leaves parts owed to an observer who can receive knowledge, because they did
not fit the boundary's limit, the aggregate system also schedules an internal
`fiscal_report_drain_v1` packet (`FISCAL_REPORT_DRAIN_INGRESS`), empty and
due at once. The packet makes the next boundary come, at the same instant:
`advance_canonical` and `step_canonical` settle it before they move on, and
`settle_boundary` refuses a later time until a boundary at that instant has
admitted the packet, as it already does for the packet an action command
queues. The packet carries nothing, and settlement ignores it; the boundary
that admits it finds the owed parts as any boundary does. A plan that owes
parts only to dead observers schedules nothing, and neither does a boundary
of a run that holds direct command history: such a run cannot add canonical
ingress, and the engine fails a boundary whose system schedules some, so the
aggregate system asks first (`SimulationView::can_schedule_ingress`, new in
the engine) and leaves the parts owed for the next boundary that runs the
fiscal systems.

**A revived observer is completed by the boundary that revives him.** Person
availability changes only in phases 7 and 10, and the aggregate and report
systems read it in phases 12 and 13 of the same boundary. A boundary that
revives a person runs the fiscal systems whenever it admits any ingress or
event, so it publishes what the revived observer is owed, with a new summary,
in that boundary. If it does not run them, the first later boundary that
does publishes them.

Alternatives considered:

- *Drain only through the packet.* Revival is not a fiscal event, so no
  fiscal system could schedule a packet for it.
- *Decode the state in every boundary to look for owed parts.* That undoes
  the idle early exit of 0.15.0, at the cost of a full decode per boundary.
- *Keep the plan in a record of its own,* so that publishing owed parts would
  not rewrite the state. It changes the state's shape, splits the plan's
  validation across records, and saves a write only in the rare boundaries
  that owe parts.
- *Schedule the drain with a delay.* The plugin keeps no calendar; any delay
  would be a guess about the host's time scale.

### Invariants

- A boundary in which nothing is owed to an observer who can receive it
  decodes no fiscal record, as in 0.15.0: the test reads the committed
  payload and person availability only.
- The chain of drain boundaries ends. While parts are owed to observers who
  can receive them, each boundary publishes them, at least 960 (1,024
  records less at most 64 summaries) when more are owed, and once only dead
  observers are owed it schedules nothing. The 16,384 parts of 64 full
  reports are published by the boundary that changed them and at most 17
  more.
- At most one drain packet is queued at a time: one is scheduled only by a
  boundary, and the next boundary admits every due packet.
- What is owed and to whom is read from the fiscal state and person
  availability, never from retained evidence. A run sealed while parts are
  owed to a dead observer publishes them on his revival exactly as an
  unsealed fork, a restore from its sealed segments, and its exact replay
  do, which `fiscal_report_parts.rs` checks; a run restored while a drain is
  queued carries it out as the run does.

### Persistence, replay, and format

The fiscal state's shape is unchanged. The plugin registers one more ingress
type, the internal `fiscal_report_drain_v1` with an empty object payload, so
the descriptor changes; drain packets appear in the ingress journal and are
regenerated by exact replay. Like any queued ingress, a queued drain packet
holds back `seal_evidence` until the boundary that admits it, which under
canonical stepping is the same call.

### Compatibility

- Owed parts are published by the next boundary, at the same instant, or by
  the boundary that revives their observer, where 0.15.0 waited for a
  fiscal change; a report's `as_of` is the time of that boundary.
- A host that drives `settle_boundary` itself settles one boundary at the
  instant of a boundary that left parts owed before it moves on, as it
  already must after admitting an action command.
- Owed parts no longer tell an observer that the fiscal state changed: they
  arrive whether or not it did.

### Cost

A boundary in which nothing is owed to an observer who can receive it costs
two lookups in the committed payload and, while a dead observer is owed
parts, one person-availability read for each owed binding. A boundary that
publishes owed parts costs one decode, validation, and encoding of the state,
one record write, and the publication of at most 1,024 report records.

The publication dominates, and its cost is the engine's: for each record,
`knowledge_heads` reads the holder's whole ledger twice, and every later
boundary carries all published knowledge, as the 0.15.0 proposal measured.
A run of the fiscal test fixture in a release build (one holder bound to every
observer binding, each binding seeing 4,000 facts of one institution, a state
of 2.2 MB; machine loaded as above) shows it:

| Observer bindings | Report records the change publishes | Boundary that settles the change | Boundaries that publish owed parts | Idle boundary afterwards |
| ---: | ---: | --- | --- | ---: |
| 8 | 2,008 | 900 ms, 1,024 records | 1: 892 ms, 988 records | 246 to 257 ms |
| 64 | 16,064 | 946 ms, 1,024 records | 16: from 988 ms to 8.7 s, 1,024 records each, the last 176 | 2.4 to 2.5 s |

A boundary that publishes owed parts costs about what the boundary that
settled the change did, and it grows, as that one would, with the knowledge
the holder already holds: in the second row one holder ends with 16,064
records. 0.15.0 paid the same for the same records, spread over later fiscal
changes; the drain only removes the wait. Reports this large on one holder
are what the engine's knowledge costs, left to a gap of their own, are about.

### What can still fail a boundary

The owed-parts path fails a boundary only for an invalid committed state:
a plan that owes parts to an observer binding the state does not have, or a
state that does not validate. It schedules no drain where scheduling would
fail the boundary (`tests/fiscal_report_parts.rs`,
`owed_parts_wait_in_a_run_with_direct_command_history`). A plan of owed
parts that would take a state past 32 MiB is not written; activation now
refuses the authored states that could have needed one, so the guard only
keeps an impossible case from failing the boundary.

## The fiscal action packet is internal

### Problem

The `fiscal_action_v1` packet that the admission of an
`apply_fiscal_action_v1` command queues was public ingress. A host could
enqueue one, and so could a boundary system of another plugin that declared
it as a target. Settlement checked the packet against the command record its
cause named and re-checked the issuer's role, but the issuer recorded in the
packet was trusted, and a packet that did not match its command failed the
boundary, which stopped the run.

### Decision

The action ingress is registered as internal, so `enqueue_plugin_ingress`
refuses it with `InvalidAuthority` before anything is queued. The command is
the only path for an action, and it is unchanged.

A packet settles only when its ingress record names, as its cause, the
command the packet carries. That binds the packet, issuer included, to the
command that produced it: an ingress record with a command cause is created
only by a command handler, a command handler can queue ingress only for its
own plugin, and `canwu-fiscal` has one command, whose admission resolves the
issuer and writes the packet. `canwu-society` 0.15.0 binds its cohort
transfers and pressure revisions the same way. The check reads the ingress
record of the boundary that admits it, and settlement no longer reads the
retained command record, so the settlement system no longer declares the
`canwu.core.commands` read.

A packet that fails the check, which only a boundary system of another plugin
that declares the target can now schedule, is skipped: it changes nothing,
its action ID is not consumed, and one `canwu.fiscal.ingress_rejected.v1`
event ending with `unauthorized_origin` (`FISCAL_INGRESS_ORIGIN_REJECTION`)
records it, or `malformed_payload` (`FISCAL_INGRESS_MALFORMED_REJECTION`)
when it does not decode. The boundary commits with the actions it settles.

### Invariants

- No action applies unless the admission of the command it carries produced
  its packet, with the issuer and role that admission resolved; settlement
  still re-checks that role against the state.
- No action packet fails a boundary.

### Persistence, replay, and format

The packet's shape is unchanged. The descriptor lists the type among the
internal ingress types and drops the `canwu.core.commands` read from the
settlement system.

### Compatibility

A host that enqueued `fiscal_action_v1` directly gets `InvalidAuthority`; no
such packet ever settled, since it had no command cause and failed its
boundary. The `canwu-ming-fiscal-reference` test that expected that failure
now expects the refusal at enqueue.

## The society ingress queue

### Problem

Admitted rebases, lifecycle deltas, and pressure revisions wait in an
owner-side queue for the next society settlement, which under a coarse
settlement cadence can be a month away. The queue was one record that every
admitting boundary rewrote whole, and a record change journals the previous
and the current record, so a queue that collected `n` packets over `n`
boundaries journalled evidence in proportion to `n²`. A packet that arrived
while `MAX_SOCIETY_INGRESS_QUEUE` (4,096) waited was dropped with only a
`society_ingress_rejected_v1` event: it had no ledger outcome, where its
sender looks for one.

### Decision

**A head and one batch record per admitting boundary.** The queue is now a
head record, `canwu.society:ingress-queue` / `root`, holding
`SocietyIngressQueue { schema_version: 2, queued, batches }`, and batch
records, kind `canwu.society:ingress-batch`, each holding
`SocietyIngressBatch { schema_version: 1, entries }` with the
`QueuedSocietyIngress` entries of 0.15.0. A batch record's ID is its slot
number in four digits (`society_ingress_batch_reference(slot)`).

- The intake (phase 12, event-driven) writes the packets one boundary admits,
  in admission order, into slot `batches` and counts them in the head. A
  boundary that admits none of these packets reads nothing.
- The settlement (phase 7, on the cadence) reads the head and, only when
  packets wait, slots `0` to `batches - 1`, checks them against the head,
  applies their entries in slot and admission order, empties every slot it
  used, and resets the head. Slots are reused from the first, so there are
  never more batch records than `MAX_SOCIETY_INGRESS_QUEUE`.

An admitting boundary journals its own batch, created or updated from empty,
and the small head; a settlement journals each batch it empties once. Both
follow what the boundary admits or consumes, never the queue's length.

**A full queue is a recorded refusal where the sender first learns of it.**

- The `revise_society_pressure` command is refused at admission while the
  queue is full, as a recorded `ValueOutOfRange` command rejection whose
  message ends with `queue_full` (`SOCIETY_INGRESS_QUEUE_FULL_REJECTION`).
  That is the earliest point its issuer can learn of it.
- A packet that finds the queue full when it is admitted, which a host-sent
  rebase, a provider's lifecycle delta, or a revision whose command was
  admitted while there was room can do, is refused by the boundary that
  admits it. Its terminal outcome is recorded in the cohort exchange ledger,
  where every other outcome of its kind is: a rejected rebase or lifecycle
  delta under its ingress ID, or, for a pressure revision the revision
  command produced, the alignment's rejected decision, all with
  `queue_full`. One event reports it: `society_ingress_rejected_v1`, or
  `society_pressure_revision_rejected_v1` for a revision. The boundary
  commits. Because such a refusal settles at intake, it can come before a
  decision of a lower version, queued earlier, is rejected at the next
  settlement, and that rejection then replaces it as the alignment's
  rejected decision (`PressureRevisionOutcomes::rejected` holds the last
  rejection to settle, as in 0.15.0); every rejection stays in the event
  journal. Keeping the higher version instead was rejected: it would change
  what the slot means for every rejection, including the 0.15.0 fixture
  where a resent, superseded decision is the last one rejected.

A pressure revision that is a resend of the applied decision, or that the
revision command did not produce, is handled as at settlement: the first
changes nothing, and the second is reported by its event only, because it
names no decision an alignment could record.

Alternatives considered:

- *One record per packet.* Admission order within a boundary is not ingress
  ID order, so a packet record would need its own sequence, and a boundary
  that admits many packets would write many records.
- *Batch records keyed by boundary and deleted after the settlement.* A
  deleted record stays in the store as a tombstone with its payload, and a
  read of a record kind returns tombstones, so the store and the settlement's
  reads would grow for the life of the run.
- *Refused packets kept in the queue as markers for the settlement.* Their
  number would have no bound but the ingress the engine admits, which is the
  bound the queue's cap exists to replace.

### Invariants

- The evidence an admitting boundary journals for the queue is its own batch
  and the head: a durable test admits one rebase a day for 29 days and holds
  every admission after the first to the size of the second.
- Packets settle in admission order, as before.
- The head and the batches agree: the slots in use hold exactly the packets
  the head counts, admitted by strictly later boundaries in slot order, and
  every other slot is empty. Activation refuses a state where they do not.
- Only the intake adds packets: a non-empty queue whose head the initial
  scenario established is refused with `InvalidAuthority`, as before.
- Decisions read only society records and the boundary's own admitted
  ingress, so a run sealed while packets wait settles them as an unsealed
  fork, a restore from its sealed segments, and its exact replay do.

### Persistence, replay, and format

- `SocietyIngressQueue` (the head) moves to schema version 2: it loses
  `entries` and gains `queued` and `batches`, and its registered payload
  schema changes accordingly.
- The new record kind `canwu.society:ingress-batch`, schema version 1, holds
  the entries.
- The ledger keeps its shape and schema version 1; `queue_full` is a new
  rejection code in it.
- The plugin descriptor gains the batch record schema and the changed head
  payload schema; the intake now reads and writes the batches and the
  ledger and emits `society_pressure_revision_rejected_v1`; the settlement
  reads and writes the batches; and the revision command reads the head. The
  semantic hash changes.

What a 0.15.0 state looks like to this code: nothing. Canwu crates depend on
the exact same version of each other and a 0.16.0 run never reads what a
0.15.0 run wrote, so there is no migration and no decoding of the old shape.
A scenario that carries a version-1 queue record is refused when the run is
created, because its payload has `entries` where the head schema now requires
`queued` and `batches`; a queue record never belongs in a scenario, since
only the intake writes one. Restore and exact replay within 0.16.0 are
unchanged: restore validation re-derives every queued entry from the ingress
journal as before, now reading them across the batches
(`society_ingress_queue_entries`).

### Measurements

Workload: a `Monthly` society with one cohort; every simulated day one
boundary carrying only the `Daily` category admits rebases of that cohort,
and every thirtieth day the boundary also carries `Monthly` and settles.
Evidence is the JSON size of the boundaries' record changes of the queue's
records (the 0.15.0 queue record, or the head and batch records), each change
with its previous and current record, as the 0.15.0 proposal measured it. The
same program ran on the v0.15.0 sources and on this change; it is
deterministic.

| Workload | Queue evidence, 0.15.0 | Queue evidence, 0.16.0 | Largest admitting boundary, 0.15.0 | Largest admitting boundary, 0.16.0 |
| --- | ---: | ---: | ---: | ---: |
| One rebase a day, settled on day 30 | 334,002 B | 73,733 B | 21,238 B | 1,456 B |
| Ten rebases a day, settled on day 30 | 3,171,448 B | 266,244 B | 207,154 B | 4,726 B |
| One rebase a day for 360 days, settled every 30 | 4,280,195 B | 974,421 B | 22,177 B | 1,690 B |

Under 0.15.0 the admitting boundary of day `d` journalled the `d` packets
waiting; now each journals its own packets, the same 1.4 KB a day for one
rebase. The settlement journals each batch once as it empties it: 31,821 B
for the month of single rebases, 129,683 B for the month of ten a day. The
ledger's evidence, which this change does not touch, is the same in both
(10,629 B, 102,802 B, and 1,532,253 B).

### What can still fail a boundary

In the society plugin, only an invalid society record: the root, the pending
transfers, the queue head or a batch in use (including one whose slot the
intake needs but finds occupied, which a valid state never has), the ledger,
or a stored policy decision; and a non-empty queue an initial scenario
seeded (`InvalidAuthority`).

## Settling cohort transfers as a batch

### Problem

A settlement checks each due cohort transfer against the state the earlier
transfers of the same settlement left: a transfer binds the digest of its two
cohorts when it is admitted, and any earlier transfer that moved either
cohort changes that digest. Of the due transfers that share a cohort, as
source or destination, one completes per settlement and the others settle as
`stale_source` and must be reissued. That is the right rule when each
transfer must see the previous one's result, but under a coarse settlement
cadence every transfer of a period is decided against the same state and
waits for the same settlement, so a cohort can move people once a month at
most.

### Decision

The rule is an option of the plugin, `SocietyPlugin::with_transfer_settlement`
(`TransferSettlement`), beside the cadence:

- `Batch`, the default (owner decision), checks every due transfer against
  the state at the start of the settlement, the state every issuer of the
  period could see, so only a transfer whose cohorts an earlier settlement
  changed after it was admitted is `stale_source`. The others apply in
  admission order (the order of the commands that admitted them), each to the
  state the earlier ones left, with the proportional shares of that state.
  One whose quantity no longer fits its source cohort, which must keep at
  least one person, settles as `insufficient_source`
  (`COHORT_TRANSFER_INSUFFICIENT_SOURCE`); one whose result would not be a
  valid society state is `rejected`, as in either mode.
- `OneByOne` is the 0.15.0 rule, unchanged: each due transfer, in
  operation-ID order, is checked against the in-progress state.

`SocietyPlugin::DAILY`, `SocietyPlugin::default()`, and
`SocietyPlugin::with_settlement_cadence` all settle in `Batch`; a run that
needs the 0.15.0 rule asks for `OneByOne`.

The settlement system is registered with the handler of its mode, and each
mode has its own semantic hash, so the mode is in the plugin's persisted
identity: a run is restored and replayed only with a plugin of the same
mode, and any other is refused with `PluginManifestMismatch`, as for the
cadence.

When to choose which: `Batch` under a coarse cadence, or whenever several
transfers may move people out of or into one cohort between two
settlements; `OneByOne` when each transfer must see the result of the one
before it, so that a transfer decided before another moved its cohorts is
reissued rather than applied.

### Invariants

- People are conserved: each completed transfer moves exactly its quantity,
  and every distribution keeps its cohort's headcount.
- A due transfer settles once, with a terminal ledger outcome; none fails
  the boundary.
- The outcome depends only on the society records and the transfers'
  admission order, so restore and exact replay agree, which
  `tests/society_transfer_settlement.rs` checks for both modes.

### Persistence, replay, and format

No persisted shape changes; `insufficient_source` is a new `result` value of
`CohortTransferOutcome`. The plugin's semantic hash depends on the mode.

### What can still fail a boundary

Nothing new: as before, only an invalid society record.

## Auditor appointment and removal

### Problem

In 0.15.0 the audited institution installs, replaces, and removes its own
`FiscalAuditBinding` with `RebindAudit`, so an audit binding establishes who
entered a finding and nothing about independence: the institution can remove
an auditor before his finding settles. A government usually separates the
two. An appointer above the audited institution appoints, perhaps on a
nomination from another office or person; and an auditor is removed when others
file a petition against him (an impeachment, or charges) and the appointer
decides on it. The owner asked for both situations, in a generic form.

### Decision

**Policy.** The scenario declares, per audited institution, a
`FiscalAppointmentPolicy` in `FiscalState::audit_appointment_policies`:

```rust
pub struct FiscalAppointmentPolicy {
    pub id: String,
    pub institution: EntityRef,  // whose audit bindings it governs
    pub appointer: EntityRef,    // whose decision appoints and removes
    pub nominators: BTreeSet<FiscalAppointmentParty>,
    pub petitioners: BTreeSet<FiscalAppointmentParty>,
    pub appoints_without_nomination: bool,
    pub removes_without_petition: bool,
}

pub enum FiscalAppointmentParty {
    Institution { institution: EntityRef },
    Actor { actor: PersonId },
}
```

No action changes a policy. The appointer is an institution: it decides
under one of its own authority bindings, as that binding's principal or as a
body: an institution that decides through one person is one whose authority
binding names that person as principal. An institution party
acts the same way under one of its authority bindings. A person party acts
under the policy itself: the request names the policy's ID as its binding,
and the issuer holds the new role `FiscalIssuerRole::ListedActor`, directly
or as the actor or responsible actor of a validated decision whose subject is
the audited institution. An acting actor takes no part in an appointment.

The audited institution is the appointer only where its policy says so. An
institution with no policy has no runtime appointments: only the audit
bindings installed with the state.

**Actions.** Three new `FiscalAction` variants, each a validated fiscal
action whose authority is checked at admission and again at settlement:

- `NominateAuditor { nomination_id, audit_binding_id, institution,
  candidate, authority_basis }`, by a nominator. The binding need not exist;
  if it does, it is on `institution`. The candidate must exist and the basis
  must be current, as for a rebind.
- `PetitionAuditorRemoval { petition_id, audit_binding_id, grounds }`, by a
  petitioner, against the existing binding's current auditor, on one to
  `MAX_FISCAL_EVIDENCE_PER_RECORD` evidence references.
- `DecideAuditAppointment { audit_binding_id, decision }`, by the appointer,
  where `decision` is a `FiscalAppointmentDecision`: `AcceptNomination`
  (appoint the nominee under the basis he was nominated with, which must
  still be current), `Appoint` (directly, if `appoints_without_nomination`),
  `AcceptPetition` (remove the auditor the petition names, who must still
  hold the binding), `Remove` (directly, if `removes_without_petition`), or
  `Dismiss` (a pending nomination or petition).

**Tenure.** Replacing a seated auditor by another, by `AcceptNomination`,
`Appoint`, or `RebindAudit`, is a removal followed by an appointment, so it
needs `removes_without_petition` as well as the appointment rule
(`FISCAL_PETITION_REQUIRED` otherwise). Where the policy protects him, an
auditor is removed only by accepting a petition against him, after which the
appointer fills the vacancy. Appointing into a vacancy, or the seated auditor
again (for example on a renewed basis), removes nobody and needs only the
appointment rule.

`RebindAudit` keeps its 0.15.0 shape and behaviour only where the policy
makes the audited institution its own appointer and lets it act without a
nomination (to install), a petition (to remove), or both (to replace). It is
then the appointer's direct decision and has the same effects, superseding
included. Anywhere else it is refused with `FISCAL_ISSUER_NOT_APPOINTER`, or
`FISCAL_APPOINTMENT_POLICY_MISSING` where no policy exists.

**Proposals.** A nomination or petition is a `FiscalAuditProposal` in
`FiscalState::audit_proposals`: its binding, audited institution, kind
(`Nomination { candidate, authority_basis }` or `RemovalPetition { auditor,
grounds }`), the listed party that made it, and `proposed_by` (the action
and its issuer). It is pending until resolved once, with a
`FiscalProposalResolution { outcome, resolved_by }` whose outcome is
`Accepted`, `Dismissed`, or `Superseded`. What happens to the others is
deterministic, and touches only the audited institution's proposals (binding
IDs are one namespace, so another institution may nominate for an ID that is
free on its own side):

- a party's new proposal of one kind on one binding replaces its pending one
  of that kind there, which is dropped rather than kept;
- an appointment (any) supersedes every other nomination pending on the
  binding, and the petitions pending on it only when it replaces the seated
  auditor they name;
- a removal (any) supersedes every other petition pending on the binding,
  and leaves its nominations pending for the vacancy.

**Room.** No party can take the room of another. A party has at most
`MAX_FISCAL_PENDING_PROPOSALS_PER_PARTY` (4) proposals pending under one
policy; admission refuses a further one as `ValueOutOfRange` unless it
replaces the party's own, and settlement checks the same against the state
it settles on. A policy lists at most 32 nominators and 32 petitioners, so
it has at most 256 proposals pending, and policies do not share room. Each
policy keeps its latest `MAX_FISCAL_RESOLVED_PROPOSALS_PER_POLICY` (64)
resolved proposals, as the record of who proposed, on what, and who
decided; a resolution past that drops the oldest (by resolution time, then
ID). Re-proposing, which replaces, and resolving, which prunes, therefore
never exhaust anything, and a binding and a finding keep their nominator in
their own `FiscalProposalRef` after the proposal itself is pruned.

**Provenance.** The binding names the decision in `rebound_by` (as
`RebindAudit` did) and the accepted nomination in the new `nomination`
(`FiscalProposalRef { proposal_id, proposed_by }`). A finding entered under an
audit binding copies both, as `appointed_by` and `nomination`, so it keeps
who nominated and who appointed its auditor after the binding changes. A
petition keeps its petitioner in `proposed_by` and the deciding appointer in
its resolution.

**Refusals.** Every refusal is recorded: admission rejects the command with
`InvalidAuthority` (`FISCAL_ISSUER_NOT_LISTED`, `FISCAL_ISSUER_NOT_APPOINTER`,
`FISCAL_NOMINATION_REQUIRED`, `FISCAL_PETITION_REQUIRED`,
`FISCAL_APPOINTMENT_POLICY_MISSING`), `InvalidPayload`
(`FISCAL_PROPOSAL_NOT_PENDING`, malformed IDs or grounds), `EntityNotFound`,
or `ValueOutOfRange`, and the engine records the rejected attempt; a
settlement that finds the issuer's role gone or the cited proposal resolved
by an earlier action of the boundary records a `Rejected` outcome.

**Alternatives.** *Folding the decision into `RebindAudit`* with optional
citations would have kept one action, but an external appointer installing a
new binding must name the audited institution, which `RebindAudit` takes from
its issuer; three explicit actions keep each check in one place. *Keeping
only pending proposals* would have lost who petitioned once a petition was
granted and the binding removed. *Global caps* on pending and kept
proposals, as first implemented, let one party fill the pending room of
every other, or exhaust the lifetime room by re-nominating; per-party and
per-policy room cannot be taken by anyone else. *Persons as appointers* were
not asked for; an institution with a principal covers a single decider.

### Invariants

- The audited institution changes its auditors only where its policy makes
  it its own appointer.
- A proposal is resolved at most once, and a pending petition always names
  the current auditor of an existing binding on its institution, since any
  change of that auditor supersedes it (`FiscalState::validate` checks this).
- An auditor whose policy does not let the appointer remove without a
  petition keeps his binding until a petition against him is accepted.
- Every proposal is by a party its policy lists; each party has at most four
  pending, of different bindings or kinds, and each policy at most 64
  resolved.
- Restore and exact replay reproduce policies, proposals, bindings, and
  findings; `tests/fiscal_audit_appointment.rs` checks this after each case.

### Persistence, replay, and format

`FISCAL_RUNTIME_SCHEMA_VERSION` becomes 3. `FiscalState` gains
`audit_appointment_policies` and `audit_proposals`, each omitted while empty;
`FiscalAuditBinding` gains `nomination` and `FiscalAuditFinding` gains
`appointed_by` and `nomination`, each omitted while `None`; `FiscalAction`
gains `nominate_auditor`, `petition_auditor_removal`, and
`decide_audit_appointment`; `FiscalIssuerRole` gains `listed_actor`. The
plugin's semantic hash changes. Under the same-version policy nothing reads
a 0.15.0 state, so none of this needs a default for old bytes.

### Cost

A proposal or decision is one settled action: it copies and validates the
state like any other, and validation walks the proposals once. Finding the
policy of an institution scans the policies, at most
`MAX_FISCAL_RUNTIME_BINDINGS`; a party's room and the pruning of resolved
proposals scan the proposals, at most 320 per policy.

### Applying the pattern to authority bindings

The policy names no binding kind, so the same struct can govern who appoints
and dismisses the actors of an authority binding. That change would add a
second map of policies keyed to authority bindings (or a field naming the
governed kind), nomination and petition actions whose target is an authority
binding and whose candidate fills `authorized_actor` or `acting_actor` with a
basis, decisions that replace or clear those actors, and the rule that
`RebindAuthority` is the appointer's direct decision only where the policy
makes the binding's own institution its appointer, which is the institution
role's meaning today. The proposal record, supersession, caps, and
provenance carry over unchanged. It is not implemented: it would change who
may rebind every existing authority binding, which needs its own owner
decision.

### What can still fail a boundary

Nothing new. An appointment action that would not fit the state's byte
budget is a rejected outcome, as every action is.

## Compatibility

0.16.0 is a pre-1.0 minor release, and Canwu crates depend on the exact same
version of each other: a 0.16.0 run never reads what a 0.15.0 run wrote, so
none of these changes needs migration.

- **Source.** `canwu-fiscal` adds `FISCAL_REPORT_DRAIN_INGRESS`,
  `FISCAL_INGRESS_REJECTED_EVENT`, `FISCAL_INGRESS_BUDGET_REJECTION`,
  `FISCAL_INGRESS_MALFORMED_REJECTION`, `FISCAL_INGRESS_ORIGIN_REJECTION`,
  and `MAX_FISCAL_REPORT_PLAN_BYTES`; for appointments it adds
  `FiscalAppointmentPolicy`, `FiscalAppointmentParty`,
  `FiscalAppointmentDecision`, `FiscalAuditProposal`,
  `FiscalAuditProposalKind`, `FiscalProposalResolution`,
  `FiscalProposalOutcome`, `FiscalProposalRef`,
  `MAX_FISCAL_APPOINTMENT_PARTIES`, `MAX_FISCAL_PENDING_PROPOSALS_PER_PARTY`,
  `MAX_FISCAL_RESOLVED_PROPOSALS_PER_POLICY`, `FISCAL_INGRESS_INVALID_REJECTION`,
  `FISCAL_APPOINTMENT_POLICY_MISSING`,
  `FISCAL_ISSUER_NOT_APPOINTER`, `FISCAL_ISSUER_NOT_LISTED`,
  `FISCAL_NOMINATION_REQUIRED`, `FISCAL_PETITION_REQUIRED`, and
  `FISCAL_PROPOSAL_NOT_PENDING`. Nothing is removed, but `FiscalAction`
  (three variants), `FiscalIssuerRole` (`ListedActor`), `FiscalState` (two
  maps), `FiscalAuditBinding` (`nomination`), and `FiscalAuditFinding`
  (`appointed_by`, `nomination`) grow, which breaks exhaustive matches and
  struct literals. `canwu-society`
  replaces the fields of `SocietyIngressQueue` (`entries` becomes `queued`
  and `batches`), which is source-incompatible, and adds
  `SocietyIngressQueue::empty`, `SocietyIngressBatch`,
  `SocietyIngressBatchRecord`, `society_ingress_batch_reference`,
  `society_ingress_queue_entries`, `SOCIETY_INGRESS_QUEUE_FULL_REJECTION`,
  `TransferSettlement`, `SocietyPlugin::with_transfer_settlement`,
  `SocietyPlugin::transfer_settlement`, and
  `COHORT_TRANSFER_INSUFFICIENT_SOURCE`. `SocietyPlugin` gains a private
  field and stays `Clone`; `SocietyPlugin::DAILY` and the default plugin now
  settle transfers in `Batch`. The engine (`canwu-sim`, re-exported by
  `canwu-api`) adds `SimulationView::can_schedule_ingress`.
- **Plugin identity.** The semantic hashes and descriptors of
  `canwu-fiscal` and `canwu-society` change, and the society's semantic hash
  depends on its transfer settlement mode.
- **Behaviour.** The fiscal budget changes as described, receipts and
  historical-context changes are measured, owed report parts are published
  without a fiscal change, the action packet is internal, `RebindAudit` is
  refused unless the audited institution's policy makes it its own appointer
  (an institution with no policy has no runtime appointments), the society queue
  is stored in batches, a full queue is a recorded refusal, and a society
  settles its due transfers as a batch unless it asks for `OneByOne`. Of
  several due transfers that share a cohort, a 0.15.0 society completed one
  per settlement; a 0.16.0 society by default completes each that still fits.
  No existing society fixture or example depended on the old default: the one
  fixture that expects `stale_source` (`gap_g31_g32_society.rs`) gets it from
  a rebase, which is stale in both modes.
- **Fixtures edited for a changed contract.**
  `parts_past_the_boundary_limit_follow_in_the_next_fiscal_boundary` became
  `parts_past_the_boundary_limit_follow_in_the_next_boundary`, and
  `a_dead_observer_is_skipped_until_he_is_alive_again` now expects the
  reviving boundary to publish (`tests/fiscal_report_parts.rs`); the two
  budget unit tests measure the state without its plan; the
  `canwu-ming-fiscal-reference` test of a host-enqueued action packet expects
  the refusal at enqueue, and its six tests of receipts with invalid
  evidence, a reused external operation, or a payload that declares its own
  quantity now expect the boundary to commit and the receipt to be refused
  with the event instead of the boundary to fail;
  `gap_g57_fiscal_audit_binding` declares a
  self-appointment policy for both institutions, so that its `RebindAudit`
  steps keep the 0.15.0 rule; `a_context_change_whose_derived_fields_do_not_fit_changes_nothing`
  changes the context a day into the run, since a new year at the start is
  now refused rather than accepted by the measured pass; and the society
  tests read the queue through
  `society_ingress_queue_entries` and seed it in the new shape.

## Open decisions

- **Acting actors and appointments.** An acting actor neither nominates,
  petitions, nor decides, as he never rebinds. Letting him nominate or
  petition for his institution is a one-line change if the owner wants it.
- **No policy, no runtime appointment.** An institution without a policy
  keeps only the audit bindings installed with the state. The other reading,
  that no policy means the 0.15.0 self-appointment, was rejected because the
  owner ruled that the audited institution is never its own appointer unless
  the scenario says so.
- **Removal records.** A removal without a petition is recorded only by the
  decision's action outcome and command; the removed binding, with its
  `rebound_by` and `nomination`, is gone. Keeping removed bindings would need
  a lifetime cap of its own.
- **Resolved proposals kept.** Each policy keeps its latest 64 resolved
  proposals and drops older ones; the event journal and the action outcomes
  keep the rest. A longer history in the state would need the archive the
  crate does not have yet.
- **A refused receipt leaves no record in the state.** Its only record is
  the event; its evidence stays unconsumed and can settle a corrected
  receipt. Whether a refusal should also be kept in the state is open.
- **The measured pass is not bounded.** Near the budget each receipt or
  context change costs a clone, a validation, two serializations, and a
  derivation of the state, about half a second near 32 MiB
  (see [Cost](#cost)). Capping the packets measured per boundary, with the
  rest refused for the host to resend, would bound it at the price of
  refusing valid receipts; it is left to the owner.
- **A reserve for receipts.** Rejected above; a host that wants executions to
  outlast decisions near the budget could be given one as an option.
- **Ledger writes for a full queue.** A boundary that refuses a packet for a
  full queue rewrites the cohort exchange ledger once, as a settlement does.
  The ledger keeps every outcome for the life of the run, so that write grows
  with the run; it is the ledger's cost, which 0.15.0 already paid at every
  settlement, and it happens only while the queue is full.
