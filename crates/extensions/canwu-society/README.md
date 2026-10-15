# canwu-society

`canwu-society` is Canwu's published **social diffusion
simulation module**. Architecturally, it is a **domain extension** built on the
public engine contracts rather than a kernel subsystem. It owns aggregate
population dispositions, social influence, organization topology,
institutional alignment, policy pressure, and actor-relative estimates while
reusing Canwu's settlement, event, decision, knowledge, persistence, and replay
infrastructure.

It intentionally contains no religion, doctrine, ritual, historical era,
rebellion, or war types. Applications provide those meanings through data and
downstream rules.

The crate is an official optional release. Its API follows Canwu's pre-1.0
compatibility policy and may evolve in a future SemVer release.

Society settles in the boundaries that carry the plugin's settlement cadence,
a `SystemCadence` category: `Daily` for `SocietyPlugin::DAILY` and
`SocietyPlugin::default()`, or the category given to
`SocietyPlugin::with_settlement_cadence`. Canwu keeps no calendar, so the host
decides which boundaries carry the category; a host that wants a week or
another declared period schedules a category it does not otherwise use, for
example `EraScheduled`, at that period. The four settlement systems run on the
cadence; the two event-driven intakes queue admitted work at any boundary, and
the next settlement applies it. The cadence is recorded in the persisted
plugin descriptor, so a run is restored and replayed only with a plugin of the
same cadence (`PluginManifestMismatch` otherwise).

Everything the phase-7 system applies waits for the cadence: due cohort
transfers, queued rebases, lifecycle deltas and pressure revisions,
institutional decisions, and transitions. An application that needs such a
change to take effect within a day registers a `Daily` society. Under a
coarse cadence, note two things. Every issuer in the period decided against
the same state, so by default the due transfers are checked together against
the state at the start of the settlement and each one that still fits
completes; `TransferSettlement::OneByOne` instead completes one transfer per
shared cohort per settlement and settles the others as `stale_source` (see
below). And at most
`MAX_SOCIETY_INGRESS_QUEUE` (4,096) packets wait for a settlement. A packet
that arrives while the queue is full is refused by the boundary that admits
it, the earliest point its sender can learn of it: a terminal rejected
outcome with `queue_full` (`SOCIETY_INGRESS_QUEUE_FULL_REJECTION`) is
recorded in the cohort exchange ledger, where every other outcome of its
kind is, and one event reports it. A `revise_society_pressure` command is
refused at admission while the queue is full, as a recorded
`ValueOutOfRange` command rejection. A refused rebase leaves its cohort
behind its stock until the provider sends another, and a refused pressure
revision must be decided again.

The queue costs evidence in proportion to what each boundary admits, however
long it waits. It is a small head record (`SocietyIngressQueue`, schema
version 2: the number of packets and of batches) and one batch record
(`SocietyIngressBatch`, kind `canwu.society:ingress-batch`) for each boundary
that admitted packets since the last settlement. A boundary writes its own
batch into the next free slot and updates the head; a settlement applies the
batches in slot order, empties the slots it used, and resets the head, and
later boundaries reuse the slots from the first. There are never more batch
records than `MAX_SOCIETY_INGRESS_QUEUE`. `society_ingress_queue_entries`
reads the waiting packets in admission order.

A transition rate is per settlement, not per unit of simulated time: each
settlement moves `floor((source × rate + carry) / 1,000,000)` people for a
rule and cohort and carries the remainder exactly, so the people moved over
any number of settlements differ from the exact sum of the shares by less
than one person. An application authors each rate for the cadence it
registers. Settling the same span on a coarser cadence at a proportionally
higher rate differs only by the compounding inside the coarser period.

A settlement that changes nothing writes nothing. `last_transition_at` is the
time of the last settlement in which a transition changed a distribution or a
carried remainder, and transition state changes at most once per instant.
`last_mobilization_at` and `last_aggregation_at` are the times those views
were last materialized: while one is set its view matches the authoritative
state, a change the view depends on clears it, and the next settlement
materializes the view once and sets it, also when the view comes out empty.
A settlement that moves people therefore writes the root up to four times, as
before, and one that changes only a remainder writes it once.

An observer's projection changes only with that observer's estimate. After
its changes the settlement recomputes each observer's entries and keeps the
projection, `observed_at` included, of every observer whose entries are the
same; the others are rebuilt at that boundary's time, and
`last_projection_at` is the time the projections were last completed. A
projection's `observed_at` is therefore the time its estimate last changed,
and it tells an observer nothing about cohorts outside its scope or about
changes it cannot detect. It is recorded history: it cannot be derived from
the rest of the root, validation only bounds it above, and a restored root's
value is taken as given.

Use `from_society_snapshot_json` for snapshot rehydration of a `Daily` run and
`from_society_snapshot_json_with_plugin` for a run on another settlement
cadence. They perform the engine's normal snapshot checks, then recompute the
root record's payload-to-core-reference binding and persisted society
derivations, and re-derive every queued rebase, lifecycle delta, or pressure
revision from the ingress journal before returning the simulation. Hosts that
restore society beside other plugins call `validate_society_runtime` after
`Canwu::from_snapshot_json_with_plugins`.

An expected refusal of a society command is a recorded command rejection: the
boundary commits and the commands queued behind it still settle.
`set_institutional_policy` refuses a malformed decision as `InvalidPayload`,
a missing or mismatched authority (including an unknown alignment) as
`InvalidAuthority`, and a version that is not newer than the applied or the
pending one, which includes a resend, as `IdempotencyConflict`.
`transfer_cohort_population` refuses a malformed intent as `InvalidPayload`,
a missing or mismatched authority as `InvalidAuthority`, an unknown source or
destination cohort as `EntityNotFound`, a due time in the past as
`InvalidDuration`, an expected source version that is not the root's current
version as `ValueOutOfRange`, and an operation ID bound to another settled or
pending transfer as `IdempotencyConflict`; a resend of a settled or pending
transfer is accepted and queues nothing. An invalid society record is not a
refusal and still fails the boundary. The two commands treat a resend
differently from a pressure revision: a resent policy decision is recorded as
`IdempotencyConflict`, while a resent pressure revision is accepted and
changes nothing.

A defective packet never fails a boundary either. The cohort-transfer intake
skips a packet with a `society_ingress_rejected_v1` event whose summary ends
with the reason: `malformed_payload`; `unauthorized_origin` (its ingress
record does not name, as its cause, the transfer command it carries, which is
how a packet a host authors directly is told apart); `authority_unavailable`
(the alignment no longer authorizes the actor); `unknown_cohort`; or
`operation_id_conflict` (another transfer is pending under that operation
ID). A defective rebase, lifecycle delta, or pressure revision is queued with
its verdict and settles as a rejected outcome. Only an invalid society record
fails a boundary.

A `PolicyPressure` may record its provenance: an optional `issuer` (a
government, organization, or person bound into the record's core references)
and a `decision_version`; a non-zero version requires an issuer. Both fields
are omitted from the encoding when unused.

Two canonical ingress types reach the society owner through one queue. The
public `cohort_headcount_rebase_v1` packet (`CohortHeadcountRebaseV1`) rebases
a cohort to an external conserved stock: the cited stock version must be the
current version of a record outside `canwu.society` when the packet is
admitted, or the rebase is rejected as `stale_external_stock`. The internal
`society_lifecycle_delta_v1` packet (`SocietyLifecycleDeltaV1`) carries a
lifecycle provider's target-scoped rule, alignment, and release changes, for
example from `canwu-culture`; deltas apply idempotently. An event-driven
phase-12 intake queues admitted packets in admission order, at most
`MAX_SOCIETY_INGRESS_QUEUE` at once (a further packet is refused with a
`queue_full` ledger outcome and a `society_ingress_rejected_v1` event); only
the intake adds packets to the queue, so an initial scenario that seeds a
non-empty queue is refused with `InvalidAuthority`, and one whose head and
batches disagree is refused when the run is created. The next phase-7 settlement applies the queue after
cohort transfers and before institutional decisions and transitions, and
records each outcome in `SocietyCohortExchangeLedger::rebases` or
`lifecycle_deltas`. A rebase re-proportions every distribution of the cohort
with integer largest-remainder allocation, so each distribution totals the
new headcount and every bucket stays within one unit of its exact share. A
lifecycle delta whose release is blocked (a released alignment carries a
stored institutional decision, which is permanent, or another live dependency
appeared) still deactivates the target's rules and records the target in
`blocked_releases`.

A cohort transfer binds its source digest to the two cohorts it moves, so
ordinary disposition transitions do not invalidate a deferred transfer; it
moves the proportional share current when it settles. A due transfer whose
cohorts changed after intake (for example through a rebase or another
transfer), or that no longer fits, settles as a terminal ledger outcome
(`stale_source` or `rejected`) instead of failing later boundaries; a retry
needs a new operation ID. Transfers and rebases clear the materialized
aggregates, mobilization candidates, and projections, which the same
settlement boundary rematerializes. The society plugin remains the only writer
of `canwu.society:state`.

How a settlement checks the transfers due at it is an option of the plugin,
`SocietyPlugin::with_transfer_settlement(mode)`, recorded in its identity like
the cadence: a run is restored and replayed only with a plugin of the same
mode.

- `TransferSettlement::Batch`, the default: every due transfer is checked
  against the state at the start of the settlement, the state every issuer
  of the period could see, and one whose cohorts changed since it was
  admitted settles as `stale_source`. The others apply in admission order,
  each to the state the earlier ones left, so people are conserved; one whose
  quantity no longer fits its source cohort settles as `insufficient_source`
  (`COHORT_TRANSFER_INSUFFICIENT_SOURCE`), a recorded outcome that never
  fails the boundary. It suits a coarse settlement cadence, and any run where
  several transfers may move people out of or into one cohort between two
  settlements.
- `TransferSettlement::OneByOne`: each due transfer is checked against the
  state the earlier transfers of the same settlement left, in operation-ID
  order. Of the due transfers that share a cohort, one completes and the
  others settle as `stale_source`. This is the rule of every release before
  0.16.0. Choose it when each transfer must see the result of the one before
  it, so that a transfer decided before another moved its cohorts is
  reissued against the new state rather than applied.

An institution revises policy pressure and the activity and reach of influence
edges and organization nodes at runtime with the `revise_society_pressure`
command, whose payload is a `PressureRevisionDecision` (build the ticket with
`pressure_revision_ticket`). The decision is authorized through an
`InstitutionalAlignment` exactly as `set_institutional_policy` is: a validated
`DecisionTicket` controller, the alignment's institution as command subject,
and a decision origin that matches the alignment's authorized actor. A
decision changes nothing for a cohort outside its alignment's affected cohorts
(every cohort when that set is empty). It may revise the pressure fields of a
`PolicyPressure` row of the alignment's target whose `issuer` is that
institution and whose affected cohorts lie within the alignment's (a row with
no issuer is never revisable); the `active` flag and `reach_per_mille` of an
influence edge of the target whose target cohort lies within the alignment's
(an edge changes only its target cohort's influence signal, so its source may
be anything); and the `active` flag and `base_reach_per_mille` of an
organization node of the target, only through an alignment that covers every
cohort, because an organization acts on every cohort of its target (through
organization relations it also changes the strength of related organizations,
which may belong to other targets). It never creates or
removes a row and never changes a transition rule. A decision revises at most
`MAX_PRESSURE_REVISION_ROWS` rows and applies as a whole or not at all. A
refused command is a recorded command rejection (`InvalidPayload`,
`InvalidAuthority`, `EntityNotFound`, `IdempotencyConflict`, or
`ValueOutOfRange` while the ingress queue is full) and never fails its
boundary.

An admitted decision is queued as the internal `society_pressure_revision_v1`
packet (the host cannot author it, and the intake rejects a packet the command
did not produce as `unauthorized_origin`). The packet is admitted at the
boundary after the command and applies in the first settlement after that, in
admission order with the other queued packets and before that settlement's
transitions; `set_institutional_policy` takes effect one boundary sooner.
Each alignment has its own revision version sequence: a decision
applies when its `decision_version` is newer than the alignment's applied
revision version and its `expected_version` is that applied version; a resend
of the applied decision changes nothing; any other is rejected
(`superseded_decision`, `stale_expected_version`, `row_out_of_scope`, or
`authority_unavailable`) without changing society state. An applied decision
writes its version to the `decision_version` of each pressure row it revises;
two alignments of one institution on one target can both revise one row, so
that version names the last decision applied to the row and can go down.
`SocietyCohortExchangeLedger::pressure_revisions` keeps the latest applied and
the latest rejected outcome of each alignment, naming the decision version,
its digest, the issuer, the command, and the admitted packet; every outcome is
also emitted as a `society_pressure_revised_v1` or
`society_pressure_revision_rejected_v1` event.
