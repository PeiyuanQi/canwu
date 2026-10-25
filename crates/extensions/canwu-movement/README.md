# canwu-movement

`canwu-movement` is the movement lifecycle extension for Canwu. `canwu-transport`
defines period-neutral movement orders, transport executions, itinerary
revisions, legs, custody handoffs, capacity bookings, and capacity pools, but it
is a record library. This crate owns their lifecycle: one `MovementPlugin`
keeps every admitted order and execution in a single `MovementState` domain
record (kind `canwu.movement/runtime`, id `canwu.movement:runtime`), advances
executions only through the transport crate's transition methods, settles each
leg at its due time, allocates capacity pools deterministically, and publishes
holder-relative movement reports as knowledge.

It deliberately does not own location truth, custody semantics, incidents,
hazards, hostility, or randomness. The plugin never draws: an application
system decides that a leg failed, that a route is closed, or that someone
seized custody, and reports it.

## Evidence kinds

`MovementPlugin::new(kinds)` declares the application record kinds the plugin
may read: authority records for orders, delivery attempts, external-condition
records cited by reroutes, and incident evidence. The declared kinds are the
plugin's exact read set, so a citation of any other kind is rejected.

## Operations

A tracked command, `apply_movement_operation_v1`, carries one
`MovementCommandV1 { holder, operation_key, operation }`. The holder must be
the issuer's own holder (the actor for an actor issuer, the command subject
otherwise). Operations are:

- `Order`: admit a `MovementOrder` and create its execution. The acting holder
  becomes the owner; an optional operator and delayed remote observers receive
  report grants. The owner may move itself; any other subject needs an
  `authority_basis`: the current version of a declared application record
  whose references name the owner under `movement_grantee` and every other
  subject under `movement_subject`. A self-directed order must move
  its ordering person, an order that moves only that person is self-directed,
  and a subject cannot be part of two active movements. With a
  `delivery_attempt`, arrival enters `ArrivalPending`; without one, arrival on
  the final leg settles the execution.
- `StartLeg`, `CompleteLeg`, `FailLeg`: act on the named current leg now.
- `DelayLeg`: hold the named current leg until `until`, with a bounded
  reason: the arrival of a departed leg, or the departure of a leg that has
  not departed. The hold replaces that due and creates no itinerary
  revision, so a journey can be held at a stop any number of times; later
  delays lengthen, shorten, or lift it. `until` may not precede the settling
  boundary or the due it holds (the arrival scheduled when the leg departed,
  its departure plus its planned duration, or the departure pending when the
  leg was first held), so a hold never makes a leg earlier. Only the latest
  hold is kept, and it ends when the held due settles, when the leg departs,
  arrives, or fails, or when the execution is rerouted or cancelled. With `retime_later_legs`, the part of the hold that
  elapsed is added to the order's `schedule_shift`, which moves the planned
  departures and booking windows of the legs after it, so they keep their
  planned spacing; without it they keep their planned times, and a leg after
  a held arrival departs no earlier than that arrival. A later leg whose
  booking expires meanwhile fails with `capacity_unavailable` when it is due
  to depart, unless the booking was replaced.
- `Reroute`: install a successor itinerary revision. It starts where the
  movement stands unless a leg failed after departing; a departed leg must
  arrive or fail first. An `ExternalCondition` reason must cite an existing
  version of a declared record kind.
- `RecordHandoff`: record a planned handoff into a leg of the active itinerary.
- `RequestBooking`: request pool capacity for one unstarted leg; the booking
  window must lie inside the leg's planned window moved by the holds that
  re-time it. A request may name a booking of the same leg it `replaces`,
  once: a requested one is cancelled, a confirmed one released, and a failed,
  expired, or revoked one left as it is; the replaced booking then leaves the
  execution, so it no longer counts when the leg departs, and a lost lot can
  be booked again in place as often as it is lost.
- `RevokeBooking`: the pool's custodian, or an application incident, revokes
  a confirmed booking on the pool, with a bounded reason. The booking becomes `Revoked` (its `revocation`
  records when and why) and its quantity returns to the pool. When its leg is
  due to depart, the leg fails with `capacity_unavailable` unless the owner
  or operator has replaced the booking.
- `Cancel`: cancel an execution whose subject is not travelling.
- `OfferPool`: offer or revise a `TransportCapacityPoolV1` held by the acting
  holder.

Application systems report incidents with the `movement_incident_v1` plugin
ingress (`MovementIncidentV1`), citing an exact version of a declared record
kind that must be the record's current version when the incident settles: a
superseded version is refused even while the run still retains it, so a
sealed run and its exact replay decide alike. An order's `delivery_attempt`
must likewise be current. An incident may fail or delay a leg, reroute,
record a handoff, including a seizure `HandoffKind::Seizure`, revoke a
booking, or reconcile a delivery. A seizure asserts an act by someone outside the itinerary and a
reconciliation asserts the delivery's outcome, so neither is accepted from a
command; a reconciliation must cite the execution's delivery-attempt record.

Operation keys are unique per holder for commands and per evidence record kind
for incidents, so no source can claim another's keys. Every key has one durable
outcome, `Applied` or `Rejected` with a `MovementErrorCode`; look it up with
`MovementState::operation_outcome`. An exact retry is a no-op. A command that
reuses a key with different input is refused at admission with
`IdempotencyConflict`; any other conflicting reuse is recorded as a separate
`Conflict` rejection. Malformed packets that carry no authority are ignored
rather than failing the boundary.

## Settlement

- Phase 7, `movement_lifecycle_apply_v1`: applies admitted operations and
  incidents in admission order, runs one allocation pass per pool, applies due
  legs, retires closed executions, and prunes expired outcomes.
- Phase 8, `movement_lifecycle_validate_v1`: validates the candidate runtime.
- Phase 13, `movement_report_publish_v1`: publishes changed reports.

Leg timing uses the internal scheduled ingress `movement_leg_due_v1`. An order
schedules its first departure at the planned departure time; a departure
schedules the arrival after the leg's planned duration; an arrival schedules
the next departure, no earlier than its planned departure plus the
order's `schedule_shift`; a hold moves the departure or arrival to its
`until`. A due packet acts only when it matches the order's persisted
`pending_due`, and an operation settled in the same boundary as a due packet
acts first. When an operation or hold replaces a due that is not yet due, the
plugin withdraws its queued packet (`CancelPluginIngress`), so superseded
packets neither wait in the queue nor wake a boundary. Due work never fails a
boundary: if it cannot be applied, the current leg fails with
`due_settlement_failed`.

Pools are allocated in phase 7 with `canwu_transport::allocate_capacity_bookings`
rather than kernel phase-6 reservations: every booking requested in a boundary
competes in one pass ordered by priority, window start, tie-break key,
admission sequence, and booking identity, and is confirmed in full or failed
with recorded evidence. Capacity already confirmed is never preempted by a
later request; only a revocation (`RevokeBooking`, from the pool's custodian
or an application incident) takes it back. A departing
leg consumes its confirmed bookings; a leg whose booking failed, expired, or
was revoked, and not replaced, fails with `capacity_unavailable` and waits
for a reroute. A confirmed booking whose window has not opened delays the departure
until it opens.

An execution keeps only the bookings that still bear on its legs: requested
and confirmed ones, those the leg under way consumed, and those lost (failed,
expired, or revoked) for a leg of the active revision that has not departed,
until they are replaced. Every other booking is closed and leaves the
execution, with its binding, in the operation or due settlement that closed
it: a replaced one at once, a consumed one when its leg arrives or fails, a
released or cancelled one, and a lost one when its leg fails, a reroute
supersedes its revision, or the execution closes. A consumed booking's
quantity stays counted in its pool's `consumed` (through
`retired_consumption`), and the runtime versions that held a closed booking
stay in the record's history. A replaced booking's identity stays taken while
its replacement is kept.

## Reports

`canwu.movement/movement_report` knowledge records carry a `MovementReportV1`
projected for one holder. The owner and operator see the current execution,
including its state, revision reasons, handoffs, the bookings it keeps and
their revocations, failure reasons, the hold on the current leg
(`MovementLegHoldV1`: which due it holds, the due it replaced, `until`,
reason, whether it re-times the later legs, and when it was set), and the
schedule shift. A delayed remote observer sees only coarse progress as it
stood the grant delay earlier, published by the internal
`movement_report_wake_v1` ingress when that delay elapses; it learns of a
hold only from the leg's departure or arrival time. `observed_as_of` is the time of
the latest fact a report reflects, and a report is published only when it
changes. Holders without a grant see nothing. Reports are published only to
living person holders.

## Retention and limits

A closed execution is retired at the first lifecycle settlement after every
observer has received its final report (or has died) and the longest observer
delay has passed, and in any case once `max_report_delay_minutes` (the bound
on a remote observer's delay) has passed since it closed. Retirement removes the execution, its order,
report heads, and outcomes from the runtime (its bookings left when it
closed); the order and execution
identities stay reserved and consumed pool capacity stays counted. Reports
already published remain in holders' knowledge, and the domain-record history
keeps every earlier runtime version. Outcomes that no live execution needs are
pruned after `outcome_retention_minutes`, which bounds how long an exact retry
of such an operation stays a no-op.

`MovementLimitsV1::CURRENT` bounds payload size, labels, planned times,
subjects, observers, legs, revisions, handoffs, the bookings an execution keeps
at once (64, however many it books over its life), reports per boundary, and
the orders, pools, kept bookings (16,384), and operation outcomes one runtime
holds, with per-owner order and per-holder outcome quotas and headroom
reserved for incidents. A request past either booking bound is a recorded
`LimitExceeded` rejection; a replacement takes its original's place. When the budget
is spent, commands are refused at admission and an operation that still
reaches settlement emits `canwu.movement.operation_dropped.v1`; a report too
large to publish emits `canwu.movement.report_withheld.v1`. Neither is lost
silently.
