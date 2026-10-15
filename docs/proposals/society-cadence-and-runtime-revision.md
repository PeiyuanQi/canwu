# Society Settlement Cadence and Runtime Revision

Status: implemented in 0.15.0, in `canwu-society`. It closes downstream gaps
G-40 (settlement cadence and idle root rewrites) and G-45 (runtime revision of
policy pressure and of influence edge and organization activity and reach),
and it makes sure that a refused society command or a defective society packet
never fails a boundary.

0.16.0 stores the owner-side ingress queue as a head and one batch record per
admitting boundary, so its evidence no longer grows with the square of the
packets it collects, and refuses a packet that finds the queue full with a
recorded ledger outcome. It also settles the cohort transfers due at one
settlement as a batch by default, so the one-per-shared-cohort limit below
applies only to a society that asks for `TransferSettlement::OneByOne`; see
[fiscal and society limits](fiscal-and-society-limits.md).

The release entry is in [Versioning and Persistence](../versioning.md). The
other 0.15.0 proposals are
[same-instant supersession and unplaced persons](same-instant-supersession-and-unplaced-persons.md),
[keyed-draw index, draw ID, and boundary profile](keyed-draw-index-and-settlement-profile.md),
[resource archive lookup and volume](resource-archive-lookup-and-volume.md),
[single-step completion lease and runtime protected-floor install](resource-single-step-lease-and-floor-install.md),
and [fiscal throughput and authority](fiscal-throughput-and-authority.md).

## Settlement cadence and idle settlements (downstream gap G-40)

### Problem

The society plugin's four settlement systems were fixed to the `Daily` cadence
category, and a transition rate applies once per settlement. An application
whose population model moves in months had to author day-sized rates and pay
for a society settlement every day.

Every settlement also wrote the society root four times, whether or not
anything moved. The phase-7 system stamped `last_transition_at` and cleared
the derived views, and the phase-10, phase-12, and phase-13 systems then
rematerialized the same views to stamp their own times. A record change
journals the previous and the current record, so a 75 KiB root cost about
600 KiB of evidence per simulated day.

### Decision

**The settlement cadence is an option of the plugin.**
`SocietyPlugin::with_settlement_cadence(cadence)` returns the plugin whose
four settlement systems (`settle-social-transitions`,
`evaluate-mobilization-candidates`, `aggregate-social-state`, and
`materialize-society-projections`) run on that `SystemCadence` category.
`SocietyPlugin::DAILY` and `SocietyPlugin::default()` are the `Daily` plugin.
The two event-driven intakes keep queuing admitted work at any boundary; the
next settlement applies it. `EventDriven` is refused at registration.
`from_society_snapshot_json_with_plugin` restores a run on any cadence.

Canwu keeps no calendar, so a cadence is a category the host attaches to
boundaries, not a length of time. A `Monthly` society settles whenever the
host settles a `Monthly` boundary, whatever the length of its months. A host
that wants a week or another declared period schedules a category it does not
otherwise use, for example `EraScheduled`, at that period.

**Everything waits for the cadence.** One phase-7 system writes the root, so
everything it applies waits for the next settlement: due cohort transfers,
queued rebases, lifecycle deltas and pressure revisions, institutional
decisions, and transitions. An application that needs such a change to take
effect within a day registers a `Daily` society. A coarse cadence has three
further consequences:

- A due transfer is checked against the state that the earlier transfers of
  the same settlement left. Of the due transfers that share a cohort, as
  source or destination, one completes per settlement; the others settle as
  `stale_source` and must be reissued against the new state. A `Daily`
  society has the same rule for transfers due on one day.
- The owner-side queue is one record, rewritten whole by every boundary that
  admits a packet, so the evidence of a queue that waits a period grows with
  the square of the packets it collects. Thirty small revisions admitted on
  thirty days before one `Monthly` settlement journal 610 KB for the queue,
  whose final record is 20 KB.
- The queue holds at most `MAX_SOCIETY_INGRESS_QUEUE` (4,096) packets. A
  packet that arrives while it is full is dropped with only a
  `society_ingress_rejected_v1` event: it gets no ledger outcome and never
  settles. A dropped rebase leaves its cohort behind its stock until the
  provider sends another; a dropped pressure revision must be decided again.

**A rate is per settlement.** For each rule and affected cohort a settlement
moves

```text
moved     = floor((source × rate + carry) / 1,000,000)
new carry =       (source × rate + carry) mod 1,000,000
```

where `source` is the headcount of the rule's source profile and `rate` is the
rule's `base_rate_per_million` adjusted by its weighted signals and clamped to
`0..=1,000,000`. The carry is the existing `TransitionRemainder`. It is exact,
so the people a rule has moved for a cohort after any number of settlements
are the floor of the exact sum of `source × rate / 1,000,000` plus the first
carry: rounding never accumulates. Elapsed simulated time does not enter the
arithmetic, and an application authors each rate for the cadence it registers.

Two cadences over the same span differ only by compounding inside the coarser
period. If `k` fine settlements at rate `r` move `D` people from a source that
only this rule drains, one coarse settlement at rate `k × r` moves between `D`
and `D + (k − 1) × r × D / 1,000,000 + 1` people. The difference is a bounded
fraction of what moved in that period and is not a rounding drift.

Scaling rates by elapsed time was rejected:

- It needs a settled-through time that advances at every settlement,
  including one that moves nothing. That either keeps the idle write this
  change removes, or lets a rate that becomes non-zero apply backwards over
  the idle span.
- A host month is not a fixed number of minutes. Scaling by minutes would
  move less in a short month, which is not what a per-month rate means, and
  would put a time unit into a plugin whose engine keeps no calendar.
- It would change the arithmetic of every existing `Daily` run or add a second
  rate basis to persist and test.

Running the per-settlement step once per elapsed day inside a coarse
settlement was rejected for the first reason and because it puts a period's
whole cost into one boundary without bound.

**A settlement that changes nothing writes nothing.** The timestamps now mean:

| Field | Meaning in 0.15.0 |
| --- | --- |
| `last_transition_at` | Time of the last settlement in which a transition changed a distribution or a carried remainder. A settlement that changes neither leaves the root untouched. |
| `last_mobilization_at`, `last_aggregation_at` | Time the view was last materialized. While it is set the view matches the authoritative state. A change the view depends on clears it, and the next settlement materializes the view once and sets it, also when the view comes out empty. |
| `last_projection_at` | Time the projections were last completed. While it is set every observer has a projection that matches the authoritative state. A change that alters an observer's estimate clears it together with that observer's projection, and the next settlement completes the projections once. |
| `observed_at` of a projection | Time of the settlement in which that observer's estimate last changed. |

A transition that changes only a remainder keeps the derived views, which do
not depend on remainders, so that settlement writes the root once. A
transition that changes a distribution clears the aggregates and the
mobilization candidates, as before. `settle_transitions` returns whether it
changed anything and no longer stamps or clears on a settlement that did not.
(The public function still clears all three views when a distribution
changes; the plugin then restores the projections described next.)

**An observer's projection changes only with that observer's estimate.**
0.14.0 rebuilt every projection in every settlement, so `observed_at` said
nothing. Once idle settlements stop writing, a time that moved whenever any
distribution moved would tell an observer scoped to one cohort that something
had changed elsewhere, or within its scope but beyond what it can detect. So
after the phase-7 changes the plugin recomputes each observer's entries and
keeps the projection, `observed_at` included, of every observer whose entries
are the same; only the others are cleared and rebuilt at this boundary's
time. `observed_at` is therefore a function of the estimates the observer was
given and of nothing else.

Every reader of the timestamps was checked. The solver's only use is the
guard that skips a settlement at or before `last_transition_at`; it now means
that transition state changes at most once per instant. `validate_at` rejects
a timestamp or a projection time later than the simulation time. Exact
derived-state validation recomputes each stamped view and requires an
unstamped aggregate or candidate view to be empty; a projection that is
present must match its observer's computed entries, and a set
`last_projection_at` requires one for every observer. Cohort transfers and
rebases clear all three views; lifecycle deltas remove a released target's
entries from them. `canwu-culture` reads none of the timestamps.

Reads were made proportional to change. The three materializing systems read
their timestamp from the stored payload and return without decoding the root
when it is set. The event-driven cohort-transfer intake, which runs in
every boundary that admits an event or ingress of any plugin, decodes the root
only when that boundary admits a cohort transfer.

Keeping projections adds computation where there are observers. The phase-7
system computes the projections once more in a settlement that changes a
distribution or an organization, to compare each observer's entries. Each
validation of the root computes them while a projection is present, as
0.14.0 did while `last_projection_at` was set; that now also covers the root
version between phase 7 and phase 13.

### Invariants

- A `Daily` run in which every settlement moves population, and in which each
  observer's estimate changes at every settlement, is unchanged: the same
  arithmetic, the same four root versions per settlement, the same bytes. The
  measurements below verify equal canonical hashes against 0.14.0, with and
  without an empty derived view.
- A boundary system's decision to settle or skip depends only on the society
  records, never on retained evidence, so a sealed run and its exact replay
  agree.
- The society plugin stays the only writer of `canwu.society:state`.
- A derived view matches the authoritative state whenever its timestamp is
  set, and a projection that is present always does.
- An observer's `observed_at` moves only in a settlement that changes that
  observer's entries.

### Persistence, replay, and format

Snapshot, commitment, checkpoint, and evidence-journal formats are unchanged.
The society root keeps schema version 2 and its exact shape; only the meaning
of the four timestamps and of `observed_at` changes. A root that 0.14.0 has
settled has all four timestamps set to its last settlement time with matching
views, which 0.15.0 reads as current. Between phase 7 and phase 13 of one
boundary a root may now hold projections without `last_projection_at`; no
committed boundary ends in that state.

A projection's `observed_at` is now recorded history. Under 0.14.0 it always
equalled `last_projection_at` and could be recomputed from the root; now it
is the time the estimate last changed, which the rest of the root does not
determine. Validation only bounds it above, by `last_projection_at` and by
the simulation time, so a restored root's value is taken as given, protected
like any other state by the snapshot's commitments.

The cadence lives in the persisted plugin descriptor, as the `cadence` of the
four boundary-system contracts. A run is restored and replayed only with a
plugin of the same cadence; any other fails with `PluginManifestMismatch`.
The plugin's semantic hash changes, because its handlers changed.

Behaviour that differs from 0.14.0 for a `Daily` run:

- A settlement that changes nothing writes no root version. A settlement that
  changes only a remainder writes one. Root version numbers and the four
  timestamps differ on any run that has such a settlement.
- An observer whose estimate a settlement did not change keeps its projection
  and `observed_at`, and when no observer's estimate changed the phase-13
  system writes nothing. Under 0.14.0 every `observed_at` moved every day.
- An idle society no longer emits record-change events, so it no longer wakes
  other plugins' event-driven systems at the next boundary on that account.
- On a settlement that leaves a view valid, a system of another plugin that
  runs after phase 7 and before the view's materializing phase reads the view
  instead of a cleared one.
- If two boundaries at one instant both carry the cadence and the first moved
  nothing, the second settles transitions with the inputs it sees. Under
  0.14.0 the second never settled.

### Measurements

Workload: 20 cohorts and 2 targets (40 distributions, 40 influence edges, 4
rules), a root of 63 to 92 KB, 30 simulated days with one `Daily` boundary
each. Evidence is the JSON size of each boundary's `record_changes`.
The same program ran on the v0.14.0 sources and on this change, and printed a
canonical hash of all record changes and of the final root.

| Workload | 0.14.0 | 0.15.0 | Hashes |
| --- | --- | --- | --- |
| Idle: every rate zero; one observer; a mobilized minority | 4.00 versions and 508,010 B per day | 3 versions in all, on the first day; 12,781 B per day over 30 days, then 0 B | differ |
| Carry: 20 per million, a person moves every fifth day; same views | 4.00 and 599,374 B | 1.43 and 229,930 B | differ |
| Active, estimate constant: 2,000 per million between profiles the observer cannot tell apart | 4.00 and 599,448 B | 3.03 and 459,422 B | differ, by the kept `observed_at` |
| Active, empty views: 2,000 per million; no observer; nobody mobilized | 4.00 and 476,383 B | 4.00 and 476,383 B | equal |
| Active, estimate changes: 2,000 per million into a publicly conforming profile the observer detects | 4.00 and 665,106 B | 4.00 and 665,106 B | equal |
| The last workload on a `Monthly` cadence at 60,000 per million per settlement | not available | 0.13 and 20,735 B | — |

A society that moves people in every settlement still writes up to four
versions per settlement; the cadence is what reduces that. Over 360 days the
idle `Daily` workload averages 508,920 B per day under 0.14.0 and 1,067 B
under 0.15.0, and the `Monthly` workload averages 22,126 B.

## Runtime revision of pressure, edges, and organizations (downstream gap G-45)

### Problem

At runtime an institution could change three fields of one
`InstitutionalAlignment` through `set_institutional_policy`. Nothing revised a
`PolicyPressure` row, a `SocialInfluenceEdge`, or an `OrganizationNode`, and
the `issuer` and `decision_version` of a pressure row had no runtime writer.
The three policy signals, and the reach and activity behind the influence
signal, were therefore fixed when the scenario was authored.

### Decision

**One command, authorized through an alignment.** `revise_society_pressure`
carries a `PressureRevisionDecision`: an alignment ID, a `decision_version`,
an `expected_version`, and the rows to revise. Its authority is exactly that
of `set_institutional_policy`: a validated `DecisionTicket` controller, the
alignment's institution as command subject, and a decision origin that
matches the alignment's authorized actor. `pressure_revision_ticket` builds
the ticket.

**A decision changes nothing for a cohort outside its alignment's affected
cohorts.** Through the alignment the institution may revise:

- the eight pressure fields of a `PolicyPressure` row of the alignment's
  target whose `issuer` is that institution and whose affected cohorts lie
  within the alignment's. A row with no issuer is never revisable;
- `active` and `reach_per_mille` of an influence edge of the target whose
  target cohort lies within the alignment's. An edge changes only the
  influence signal of its target cohort, so the rule checks that side; the
  source may be any cohort, organization, or institution;
- `active` and `base_reach_per_mille` of an organization node of the target,
  only through an alignment that covers every cohort. An organization's
  activity and reach change signals, mobilization capacity, and concealment
  for every cohort of its target, so an alignment scoped to some cohorts
  cannot revise one. Through `organization_relations` they also change the
  strength of related organizations, which may belong to other targets, so an
  organization revision can change signals outside the alignment's target.
  The rule bounds cohorts, not relations.

Each row revision states every revisable field, so a revision replaces values
and a resend is naturally idempotent. A decision revises at most
`MAX_PRESSURE_REVISION_ROWS` (256) rows and applies as a whole or not at all.
Rows are never created or removed.

**The owner applies it in its settlement.** The command queues one internal
`society_pressure_revision_v1` packet. The phase-12 intake appends it to the
existing owner-side queue at the boundary that admits it, and the next
settlement applies queued packets in admission order, before its transitions,
so a revision sets that settlement's rates. A revision therefore takes effect
at the first society settlement after its admission boundary, at least two
boundaries after the command. `set_institutional_policy` stores its decision
in the command itself and takes effect one boundary sooner. The added work is
proportional to the rows revised.

The packet type is internal, so a host cannot enqueue it. A boundary system of
another plugin that declares the target could schedule one, so the intake
admits a packet only when its ingress record names, as its cause, the command
the packet carries. Only the revision command's handler produces that. Any
other packet is rejected as `unauthorized_origin`. The check reads the ingress
record in the boundary that admits it and stores the verdict in the queue, so
the settlement reads only state.

**Version, expected version, and resend.** Each alignment has its own revision
version sequence, separate from `last_decision_version`. The applied revision
version is kept in the ledger. When a decision settles:

- it applies if its `decision_version` is newer than the applied version and
  its `expected_version` equals the applied version;
- it changes nothing if it is the applied decision (same version and digest);
- otherwise it is rejected as `superseded_decision`,
  `stale_expected_version`, `row_out_of_scope`, or `authority_unavailable`
  (the alignment is gone or bound to another institution).

The command itself refuses a malformed payload, a missing or mismatched
authority, a row that does not exist or lies outside the alignment's
authority, and the applied version with other content. It accepts a resend of
the applied decision without queuing anything. Every refusal uses an error
code the engine records as a rejected command attempt (`InvalidPayload`,
`InvalidAuthority`, `EntityNotFound`, `IdempotencyConflict`), so a refused
decision never fails its boundary. Ordering is left to the settlement, because
an earlier decision of the same alignment may still be queued.

**Evidence.** An applied decision writes its version to the
`decision_version` of each pressure row it revises.
`SocietyCohortExchangeLedger::pressure_revisions` keeps, per alignment, the
latest applied and the latest rejected outcome: decision version, digest,
issuer, command, admitted packet, settlement time, and rejection code. Every
outcome is also emitted as a `society_pressure_revised_v1` or
`society_pressure_revision_rejected_v1` event whose summary names the
alignment, the decision version, the command, and the packet.

**Rule rates are not revisable.** A rule's rate and weights are compiled
content. A lifecycle provider reinstalls a rule only if it is identical to the
installed one, so a revised rate would make `canwu-culture`'s reconciliation
fail. A rule's effective rate is its base rate plus six weighted signals, and
every signal now has a runtime writer: alignment decisions for institutional
support and enforcement, and pressure revisions for influence and for policy
support, coercion, and disruption.

Alternatives rejected:

- A component slot per alignment, as `set_institutional_policy` uses. A slot
  holds one pending decision, so a second partial decision before a
  settlement would replace the first, and components are permanent, which
  blocks releasing the alignment.
- An issuer and decision version on every edge and organization node. It
  would change the root's shape for rows that have no single owner: several
  alignments may revise one edge, so a per-row version cannot order their
  decisions, and a recorded issuer would add a core reference that must stay
  a live entity after its alignment is released.
- One ledger outcome per packet, as rebases have. Revisions are frequent, and
  the ledger is rewritten whole, so its state is bounded by the number of
  alignments and the history stays in the event journal.
- A public ingress type, as cohort transfers use. A host can then author a
  packet, and the intake has to tell it apart and reject it; an internal type
  is refused when it is enqueued.

### Invariants

- A revision changes society state only through the society plugin's
  settlement, after an authority check at command admission and a scope check
  at admission and again at settlement.
- A changed coercion value or organization clears the mobilization
  candidates. A changed organization activity also clears the projection of
  every observer whose estimate it alters. Aggregates are kept. Transition
  signals are not stored.
- The applied revision versions of an alignment strictly increase. The ledger
  entry outlives the alignment, so a reinstalled alignment continues its
  sequence.
- Two alignments may revise the same row; the later admitted decision wins,
  and `expected_version` protects one alignment's sequence only. Two
  alignments of one institution on one target can both revise one pressure
  row, so a row's `decision_version` names the last decision applied to it
  and can go down.

### Persistence, replay, and format

Snapshot, commitment, checkpoint, and evidence-journal formats are unchanged,
and so are the society root's shape and schema version 2.

- `SocietyCohortExchangeLedger` gains `pressure_revisions`, a map keyed by
  alignment ID that is omitted while empty. The ledger keeps schema version 1;
  a ledger written by 0.14.0 decodes with an empty map and re-encodes to the
  same bytes.
- `SocietyIngressQueue` keeps its shape and schema version 1 and accepts the
  new packet type. Restore validation re-derives each queued revision's
  admission verdict from its ingress record.
- The plugin descriptor gains the command, the internal ingress type, the two
  event types on the phase-7 system, and the optional ledger property.

A queued revision survives a seal and a checkpoint: a sealed run settles it
as its exact replay does, and a compact run restored from a checkpoint taken
while it is queued settles it to the same checkpoint hash as the uninterrupted
run.

## Refused commands and defective packets never fail a boundary

### Problem

The engine records a refused command as a rejected attempt, and lets the
boundary commit, only for a fixed list of error codes. The two older society
commands, `transfer_cohort_population` and `set_institutional_policy`,
returned `InvalidDecision`, `DomainRecordVersionConflict`, and
`DomainRecordNotFound` for refusals an application must expect. None is on the
list, so a resent policy ticket or a transfer that cited a superseded root
version failed its boundary, stayed queued, and failed every later canonical
step.

The cohort-transfer intake had the same fault one step later. It returned an
error for a defect in an admitted packet, and an admitted packet cannot be
withdrawn, so the boundary failed for good: after an accepted transfer to a
cohort that does not exist, after two accepted transfers that reused one
operation ID with different data, and after a host enqueued a malformed
packet of the public `cohort-transfer` type.

### Decision

**Every expected refusal of the two commands uses a recorded rejection code.**
An invalid society record or stored decision is not a refusal and still fails
the boundary.

| Command | Refusal | 0.14.0 | 0.15.0 |
| --- | --- | --- | --- |
| `set_institutional_policy` | Version zero or a field above 1,000 per mille | `InvalidDecision` | `InvalidPayload` |
| | Unknown alignment | `InvalidDecision` | `InvalidAuthority` |
| | No society root | `DomainRecordNotFound` | `InvalidAuthority` |
| | Version not newer than the applied or the pending one, including a resend | `InvalidDecision` | `IdempotencyConflict` |
| `transfer_cohort_population` | Source and destination are the same cohort | `InvalidDecision` | `InvalidPayload` |
| | No society root | `DomainRecordNotFound` | `InvalidAuthority` |
| | Unknown source or destination cohort | accepted; the intake then failed | `EntityNotFound` |
| | Due time before the simulation time | `InvalidDecision` | `InvalidDuration` |
| | Expected source version is not the root's version | `DomainRecordVersionConflict` | `ValueOutOfRange` |
| | Operation ID of a pending transfer, with other data | accepted; the intake then failed | `IdempotencyConflict` |

A missing root or alignment is `InvalidAuthority` because authority comes from
an alignment. `IdempotencyConflict` is what the other first-party extensions
return for a reused identity. A resend of a settled or pending transfer is
accepted and queues nothing.

Resends are not treated alike across the three commands. An exact resend of an
institutional policy decision is recorded as `IdempotencyConflict`, because
its version is no longer new; a resent transfer or pressure revision is
accepted and changes nothing.

**The cohort-transfer intake is total over what a packet can carry.** A
defective packet is skipped with a `society_ingress_rejected_v1` event whose
summary names the ingress and ends with the reason, and the boundary commits:

| Defect | Reason |
| --- | --- |
| The payload has no valid intent, actor, or command | `malformed_payload` |
| The ingress record does not name, as its cause, the command the payload carries | `unauthorized_origin` |
| No society root, or the alignment no longer authorizes the actor for the institution the packet was queued for | `authority_unavailable` |
| A cohort the intent names does not exist | `unknown_cohort` |
| Another transfer is pending under the operation ID | `operation_id_conflict` |

The same transfer delivered again is skipped silently. A pending transfer
that no longer fits when it is due, or whose result would not be a valid
society state, settles as a `rejected` ledger outcome.

The intake no longer reads the original command to prove a packet's origin.
Only the transfer command's handler can produce a society packet whose ingress
record has a command as its cause, and a host cannot give a packet one, so
the cause is the proof. It is read from the ingress record in the boundary
that admits it. A check against the retained command record would treat a
forged packet that names an archived command differently in a sealed run,
where the record is gone, and in its exact replay, where it is retained.

The other society packets were already total and stay so: a defective rebase,
lifecycle delta, or pressure revision is queued with its verdict and settles
as a rejected ledger outcome or event. After this change a boundary fails in
the society plugin only on an invalid society record: the root, the pending
record, the queue (including one an initial scenario seeded), the ledger, or
a stored policy decision.

### Compatibility

This is a behaviour change. A host that matched the old codes, or that relied
on a failed boundary to detect a refused society command or a defective
packet, reads the rejected command attempt or the rejection event instead. A
run that 0.14.0 stalled now continues. The `intake-cohort-transfers` contract
no longer reads `canwu.core.commands` and now emits
`society_ingress_rejected_v1`.

## Compatibility

0.15.0 is a pre-1.0 minor release.

- `SocietyPlugin` is a struct with a private field instead of a unit struct,
  and is `Clone` but no longer `Copy`. Callers write `SocietyPlugin::DAILY`,
  `SocietyPlugin::default()`, or `SocietyPlugin::with_settlement_cadence(..)`.
- `SocietyCohortExchangeLedger` gains a public field.
- `settle_transitions` returns `false`, and leaves the state untouched, when
  it changes nothing.
- A host that read `last_*_at` as the time of the latest settlement must use
  the time of the latest boundary that carried the cadence. `observed_at` is
  the time an observer's estimate last changed.
- The error codes of the two older commands change as listed above, and a
  defective cohort transfer packet is skipped with an event instead of failing
  its boundary.

## Open decisions and follow-up

- **Several transfers per cohort in one settlement.** The rule above is kept
  because it leaves `Daily` behaviour as it is. The alternative is to compare
  each due transfer with the state at the start of the settlement, which lets
  several transfers of one cohort settle together and also changes two
  same-day transfers on a `Daily` society.
- **Queue evidence under a coarse cadence.** Removing the quadratic rewrite
  needs one record per admitting boundary or per packet, which changes how the
  queue is stored, read, and validated on restore. It is left as follow-up.
