# Movement Holds Before Departure, Re-timed Later Legs, and Booking Revocation

Status: implemented in 0.20.0. It closes downstream gaps G-73 (a `DelayLeg`
moves only the held leg), G-79 (nothing revokes another execution's
confirmed booking), and G-80 (a leg that has not departed is held only by a
`Reroute`, which spends an itinerary revision), and engine finding E-48 of
`opus-progress.md` (a `DelayLeg` leaves the due packet it replaces queued).
It changes `canwu-movement` and `canwu-transport`, and one re-export line of
`canwu-api`; `canwu-sim` is untouched. Same version only: no compatibility
work with 0.19.0 bytes or hashes.

"Today" means 0.19.0 as it is on `main`.

## Contents

1. [Problem](#problem)
2. [Invariants](#invariants)
3. [Design](#design)
4. [Refusals](#refusals)
5. [Persistence and replay](#persistence-and-replay)
6. [What can still fail a boundary](#what-can-still-fail-a-boundary)
7. [Public API delta](#public-api-delta)
8. [Durable tests](#durable-tests)
9. [Documentation delta](#documentation-delta)
10. [Terminology candidates](#terminology-candidates)
11. [Known limits](#known-limits)

## Problem

**G-73.** `delay_leg` (`lifecycle.rs`) reschedules only the held leg's
arrival due, and `arrive` schedules the next leg's departure at
`later(at, planned_departure_at)`. A hold therefore moves only the held leg:
the next leg departs at the held arrival instant (a zero-delay boundary when
the arrival passed its planned departure), and every later arrival falls off
the host's time grid by the hold. `Reroute` is refused while the held leg is
departed, so nothing can re-time the later legs.

**G-80.** `delay_leg` accepts only the current departed leg. A leg that has
not departed is held only by a `Reroute` that re-times it, and
`MovementLimitsV1::CURRENT.revisions_per_execution` is 8, so a journey held
at a stop one day at a time spends a revision a day; once all are spent, a
failed leg can never be re-planned and the execution waits `ReplanPending`
for good.

**G-79.** `offer_pool` revises a pool through `TransportCapacityPoolV1::revise`,
whose `validate` refuses a quantity below `booked + consumed`, and a booking
leaves `Confirmed` only through its own execution's `Reroute`, `FailLeg`,
`Cancel`, or departure. A requisition can take only a pool's free capacity,
never capacity already confirmed to a lower-priority execution.

**E-48.** Each `DelayLeg` leaves the due packet it replaces queued until that
packet's time: a seal and a continuation checkpoint pin it,
`pending_ingress()` lists it, and at its time it wakes an otherwise empty
boundary where it is skipped as stale. The same is true of every operation
that replaces a pending due (`StartLeg`, `CompleteLeg`, `FailLeg`, `Reroute`,
`Cancel`).

## Invariants

- **I1 — one hold, on the current leg.** An order holds at most one
  `MovementLegHoldV1`. It names the current leg of the active revision and
  the kind of due it moves (`Departure` for a leg that has not departed,
  `Arrival` for the departed leg); the pending due has that leg and kind and
  is due at the hold's `until`; `until` is no earlier than `scheduled_at`,
  the due the hold replaced; a held arrival's `scheduled_at` is the leg's
  departure plus its planned duration. A hold never makes a leg earlier.
- **I2 — holds spend no revision.** No hold creates an itinerary revision or
  a leg, so a transport execution link to the execution and leg stays valid
  and `revisions_per_execution` is spent only by reroutes.
- **I3 — the schedule shift is a delay, reset by a reroute.**
  `MovementOrderRecordV1::schedule_shift` is non-negative, grows only when a
  re-timing hold ends, and is zero after a reroute. A scheduled departure
  is no earlier than its planned departure plus the shift (an explicit
  `StartLeg` can still depart a leg earlier).
- **I4 — a lost booking blocks its leg unless replaced.** When a leg
  departs, a booking that failed, expired, or was revoked makes it fail with
  `capacity_unavailable`, unless a later booking of the same leg names it in
  `replaces`. A booking is replaced at most once, by a later binding of the
  same execution and leg, and holds no capacity once replaced.
- **I5 — revocation returns capacity.** Only a confirmed booking can be
  revoked; it becomes `Revoked`, holds nothing, and records its time and
  reason, exactly when its status is `Revoked`.
- **I6 — no superseded due waits in the queue.** At the end of each
  lifecycle settlement, a due that the boundary replaced and that is due
  strictly later is withdrawn from the queue; each order's final due is
  scheduled once, and not again when the packet queued earlier carries it.

## Design

### One hold operation for a departure or an arrival (G-73, G-80)

`MovementOperation::DelayLeg { execution, leg, until, reason,
retime_later_legs }` holds the current leg:

- **departed** (as in 0.19.0): its arrival due moves to `until`, bounded by
  the arrival scheduled at departure;
- **not yet departed** (`Planned`, `Booked`, or `Waiting`, with a pending
  departure due): its departure due moves to `until`, bounded by the
  departure that was pending when the leg was first held. A repeated hold of
  the same departure keeps that first bound in `scheduled_at`, so repeated
  holds cannot drift earlier than the departure first held.

Either way `until` is also no earlier than the settling boundary. Any number
of holds is accepted; each replaces the last (bounded by the operation
outcome quotas, as in 0.19.0). The hold record gains `kind` and
`retime_later_legs`, and `scheduled_arrival_at` becomes `scheduled_at`.

A hold ends when its due settles (the held arrival arrives the leg; the held
departure departs it, or waits for a booking window that has not opened, or
fails it for lost capacity), when `StartLeg` or `CompleteLeg` acts on the leg
explicitly, when the leg fails, or when a reroute or cancellation closes the
revision.

**Re-timing.** With `retime_later_legs`, the part of the hold that elapsed is
added to the order's `schedule_shift` when the hold ends: `at` minus
`scheduled_at`, at least zero and at most the hold's extension
(`until − scheduled_at`). A hold that ran its course adds exactly its
extension, which is the "shift every later leg by the hold" the row
recommends; one ended early by `StartLeg` or `CompleteLeg` adds only what
elapsed, so the later legs keep their planned spacing from the held leg as
it actually ran. `arrive` then schedules the next departure at
`later(at, planned_departure_at + schedule_shift)`. Without
`retime_later_legs` nothing is added and the later legs keep their planned
times, as in 0.19.0 (a leg after a held arrival departs no earlier than that
arrival). A reroute installs its own planned times and resets the shift.

The shift is one value per order rather than a per-leg schedule because a
hold always delays the current leg and everything after it by the same
amount; storing it once keeps the revision's planned times immutable (its
plan digest stays the plan that was admitted) and the runtime small.

**Re-timed booking windows.** A booking requested for a leg must lie inside
the leg's planned window moved by `booking_shift`: the order's
`schedule_shift`, plus the pending hold's extension when a re-timing hold of
an earlier leg re-times it. A held departure moves its own leg's window so
that it opens at the hold's `until` (`until` minus the planned departure, at
least the schedule shift), which also covers lateness the leg absorbed
before it was held. A host can therefore book a later leg's re-timed window
while the hold that re-times it is still pending, and a held leg's window at
its held departure. If the hold is shortened afterwards, the
booking's window opens later than the departure and the existing rule
applies: the departure waits until the window opens.

Because a re-timed window can lie past the planned one, the runtime
validation of a booking window becomes "starts no earlier than its leg's
planned departure and lasts no longer than the leg" (holds move windows
later, never earlier), instead of "inside the planned window". The request
check stays exact.

Rejected alternatives:

- *A `Reroute` that admits a departed current leg and re-times only the
  later legs* (the row's second shape) spends a revision per re-timing, which
  is G-80's problem, and needs a successor revision that copies the departed
  leg's identity, which `TransportExecution::reroute` does not support.
- *Re-timing at hold time by the stated extension.* A hold can be
  shortened, lifted, or ended early by `CompleteLeg`; folding the stated
  extension in at once would either re-time later legs by a delay that never
  happened or need the shift to shrink, which would break the booking
  validation of bookings requested under the longer hold.
- *A separate `HoldDeparture { leg, until }`.* One operation keeps one hold
  record, one bound rule, and one incident allow-list entry; the leg's
  status says which due is held.

### Booking revocation and replacement (G-79)

`MovementOperation::RevokeBooking { pool, booking, reason }` revokes a
confirmed booking on `pool`. It is accepted from the pool's custodian (a
tracked command whose holder is `custodian`) or from an application incident
citing the current version of a declared evidence record, as other incidents
are. `canwu-transport` gains the transition: `CapacityBooking::revoke(at,
reason)` moves a `Confirmed` booking to the new status
`CapacityBookingStatus::Revoked` and records `CapacityBookingRevocationV1 {
at, reason }` in `CapacityBooking::revocation`;
`TransportCapacityPoolV1::apply_booking_transition` returns its quantity
from `booked` (`Confirmed → Revoked`). The freed capacity is then free for a
later request (a requisitioner's booking allocated in the same boundary, after
the operations) or for the custodian's `OfferPool` to remove.

The row suggested returning the booking "as `Failed` / `Released`". Neither
fits: `Released` is ignored at departure, so a leg whose booking was taken
away would depart without the capacity before its owner could react; and
`Failed` is an allocation outcome whose evidence says the request was never
granted, while `Confirmed → Failed` is not a transport transition. A distinct
`Revoked` status keeps the allocation evidence honest and lets departure
treat the loss like a failed or expired booking.

So that "its owner re-requests it" needs no reroute,
`MovementBookingRequestV1` gains `replaces: Option<CapacityBookingId>`: a new
booking of the same leg may take the place of one earlier booking of that leg
that no other booking replaced. When the replacement is admitted, a
`Requested` original is cancelled and a `Confirmed` one released (so a host
can also move a booking to a re-timed window); a `Failed`, `Expired`, or
`Revoked` original is left as it is. At departure a replaced booking no
longer counts; its replacement does (I4). A replacement may be replaced in
turn, within `bookings_per_execution`.

### Superseded dues are withdrawn (E-48)

At the end of `movement_lifecycle_apply_v1`, the plugin compares each order's
pending due before the boundary with its final pending due. For every due
that changed (or whose order closed) and that is due strictly after the
boundary, it reads `SimulationView::cancellable_plugin_ingress()` once and
proposes `BoundaryDirective::CancelPluginIngress` (reason "superseded
movement leg due") for each listed `movement_leg_due_v1` packet whose payload
equals that due. The listed items are by construction this plugin's own,
still pending, and due strictly later, and each is proposed once, so the
withdrawal cannot fail the boundary. A due replaced in the boundary of its
own time was admitted in that boundary and is skipped as stale, as before.
The scheduling of final dues also skips a due the queued packet already
carries (re-stating a hold with the same `until` no longer queues a second
identical packet) and schedules each order's due once.

This covers every operation that replaces a pending due, not only holds:
it is the same code path (the order's `pending_due`), and the row's "cancel
the superseded due when a hold replaces it" applies equally to `StartLeg`,
`CompleteLeg`, `FailLeg`, `Reroute`, and `Cancel`.

## Refusals

Every refusal is a recorded rejection on the operation's outcome; the
boundary stands. Check order, after the incident checks of 0.19.0
(allow-list, then current evidence):

`DelayLeg`:

1. the execution is known (`NotFound`) and the command holder is its owner
   or operator (`Unauthorized`);
2. the reason is a bounded label (`InvalidOperation`);
3. `leg` is the current leg and is departed, or is `Planned`, `Booked`, or
   `Waiting` with a pending departure due (`InvalidState`);
4. `until` is no earlier than the boundary and the due it holds
   (`InvalidOperation`);
5. `until` is within `time_bound_minutes` (`LimitExceeded`);
6. the candidate runtime validates (`InvalidState`).

`RevokeBooking`:

1. the pool is known (`NotFound`);
2. a command holder is the pool's custodian (`Unauthorized`; an incident
   carries application authority);
3. the reason is a bounded label (`InvalidOperation`);
4. the booking is bound to that pool (`NotFound`);
5. the booking is `Confirmed` (`InvalidState`).

`RequestBooking` (new checks, after the 0.19.0 ones):

1. the window lies inside the leg's planned window moved by `booking_shift`
   (`InvalidOperation`; `LimitExceeded` past the time bound);
2. `replaces` names a booking bound to the same execution and leg
   (`InvalidOperation`), not yet replaced (`Conflict`), and requested,
   confirmed, failed, expired, or revoked (`InvalidState`).

A departure due whose re-timed time would pass the time bound, or a shift
that would, fails the leg through the existing `due_settlement_failed`
fallback.

## Persistence and replay

| Item | 0.19.0 | 0.20.0 |
| --- | --- | --- |
| `MovementOperation` (command and incident payloads) | 11 variants; `delay_leg { execution, leg, until, reason }` | `delay_leg` adds the required `retime_later_legs`; adds `revoke_booking { pool, booking, reason }` |
| `MovementBookingRequestV1` | — | `replaces`, omitted when `None` |
| `MovementLegHoldV1` (runtime and report) | `leg, scheduled_arrival_at, until, reason, held_at` | `leg, kind, scheduled_at, until, reason, held_at, retime_later_legs` |
| `MovementOrderRecordV1` (runtime) | — | `schedule_shift`, omitted when zero |
| `MovementBookingBindingV1` (runtime) | — | `replaces`, omitted when `None` |
| `CapacityBooking` (`canwu-transport`, in the runtime and reports) | — | `revocation`, omitted when `None` |
| `CapacityBookingStatus` | 7 values | adds `revoked` |
| `MovementReportDetailV1` (report) | — | `schedule_shift`, omitted when zero |
| `MovementState::validate` | booking inside its leg's planned window; held arrival only | booking starts no earlier than the planned departure and lasts no longer than the leg; replacement and revocation invariants; holds of either kind (I1); non-negative bounded shift |
| `TRANSPORT_SEMANTIC_VERSION` | `canwu-transport.v5` | `canwu-transport.v6` |
| `MOVEMENT_SEMANTIC_HASH` | `6cf78f1f…550e` | `b211c92a…abbc`, BLAKE3 of `canwu-movement plugin semantic hash 0.20.0: DelayLeg holds a departure or an arrival and may re-time later legs; RevokeBooking; booking replacement; superseded dues are cancelled` |
| `movement_report` knowledge schema hash | `81cf8978…4f11` | `33d6f3ab…ed28`, BLAKE3 of `canwu-movement movement_report knowledge schema 0.20.0: MovementLegHoldV1 kind, scheduled_at, retime_later_legs; MovementReportDetailV1::schedule_shift; CapacityBooking::revocation`; the schema id stays at version 1 |
| `movement_incident_v1` descriptor description | "…delay, reroute, or handoff" | "…delay, reroute, handoff, or booking revocation" |
| Snapshot, continuation, and journal formats | — | unchanged; a withdrawn due is an ordinary `PluginCancellation` ingress record of the boundary that withdrew it |

The semantic hash changes, so a run saved by 0.19.0 with the movement plugin
does not restore; under the same-version policy none is expected to.

Replay: every new decision reads only the movement runtime and the boundary
time. The withdrawal reads `cancellable_plugin_ingress()`, which lists the
same items in a live run, a sealed run, a continuation restore, and exact
replay (the plugin-ingress cancellation contract of downstream gap G-01,
`gap_g01_sim_cancel_plugin_ingress`); the payload comparison reads the
pending packet's own payload, which a seal pins with the packet. No decision
reads retained evidence.

## What can still fail a boundary

Nothing new.

- The withdrawal proposes only items `cancellable_plugin_ingress()` listed in
  the same system (this plugin's, pending, due strictly later), each once; no
  other system can withdraw a movement due, because the plugin keeps no
  permit for its internal packet types.
- "movement due work precedes its boundary" stays unreachable: a hold's
  `until` is no earlier than the boundary, and a re-timed departure is
  `later(at, …)`.
- What could already fail a movement boundary still can: a runtime record
  that fails to decode, validate, or encode (a corrupted or hand-edited
  root), and the runtime revision overflowing.

## Public API delta

- `canwu-transport` (re-exported by `canwu-api`):
  `CapacityBookingStatus::Revoked`, `CapacityBookingRevocationV1` (new),
  `CapacityBooking::revocation` (new field), `CapacityBooking::revoke` (new
  method), `TRANSPORT_SEMANTIC_VERSION` (new value); `CapacityBooking::transition`
  refuses `Revoked`.
- `canwu-movement`: `MovementOperation::DelayLeg::retime_later_legs`,
  `MovementOperation::RevokeBooking` (new variant;
  `MovementOperation::execution` returns `None` for it, like `OfferPool`),
  `MovementBookingRequestV1::replaces`, `MovementBookingBindingV1::replaces`,
  `MovementOrderRecordV1::schedule_shift`, `MovementReportDetailV1::schedule_shift`,
  `MovementLegHoldV1` reshaped (`kind`, `scheduled_at`, `retime_later_legs`)
  with `MovementLegHoldV1::extension`, `MOVEMENT_SEMANTIC_HASH`.
- Behaviour: `DelayLeg` on a leg that has not departed is applied (it was an
  `InvalidState` rejection); a superseded due is withdrawn rather than left
  to wake a boundary; the runtime validation of booking windows is relaxed
  as above.

The new variants and fields break exhaustive matches and struct literals
downstream, as any 0.x minor release may. No `canwu-sim` change, so no
`gated!` change.

### Upgrading from 0.19.0

- `DelayLeg` needs `retime_later_legs`. A `DelayLeg` written without it is an
  unknown shape, not a recorded rejection: as a command it is refused at
  admission (`InvalidPayload`), and as an incident the packet is ignored;
  neither records a movement outcome.
- `DelayLeg` on a leg that has not departed now holds its departure
  instead of being refused. `MovementLegHoldV1::scheduled_arrival_at` is
  now `scheduled_at` with `kind`; pass `retime_later_legs: true` to keep
  later legs on their planned spacing (in 0.19.0 they departed at the held
  arrival).
- A pool's custodian, or an application incident, can revoke a confirmed
  booking (`RevokeBooking`); a revoked booking fails its leg at departure
  unless the owner requests a new booking that names it in `replaces`.
  Matches on `CapacityBookingStatus` and `MovementOperation` need the new
  variants, and struct literals need `replaces`.
- Superseded leg-due packets no longer stay queued: `pending_ingress()` does
  not list them and they do not wake boundaries; each withdrawal is a
  `PluginCancellation` ingress record. Booking-window validation is relaxed
  to "starts no earlier than the leg's planned departure and lasts no longer
  than the leg".

## Durable tests

`crates/extensions/canwu-movement/tests/gap_g73_g79_g80_movement_holds_and_revocation.rs`:

1. **`gap_g73_g80_holds_retime_later_legs_without_a_revision`**: two journeys
   a > b > c > d, ten-minute legs departing at 10, 30, and 50. The first
   leg of the first journey is held at its origin nine times (more than
   `revisions_per_execution`) with re-timing; the execution keeps one
   revision, the hold keeps its first bound (a hold ending before the planned
   departure is `InvalidOperation`), and after each hold exactly one leg-due
   packet is queued (E-48). It departs at 15, so its next leg departs at 35;
   under way it is held until 52 with re-timing, a booking of the third leg
   for the planned window is `InvalidOperation` and one for the re-timed
   window (62 to 72) is confirmed and consumed when the leg departs at 62.
   The second journey's first leg is held under way without re-timing: its
   next leg departs at the held arrival (32) and its third at the planned 50.
   No leg-due packet is left once both settle; the owner's report keeps the
   twelve-minute shift; a run restored mid-hold and exact replay agree.
2. **`gap_g79_revoked_booking_is_replaced_by_its_owner`**: two lots and a
   requisition share a pool of three. A non-custodian revocation is
   `Unauthorized`, a booking on another pool or an unknown one `NotFound`.
   The custodian revokes one lot and an application incident the other; a
   second revocation is `InvalidState`; the pool holds nothing, and the
   requisition books all three. The first lot replaces its revoked booking
   from another pool (a second replacement of it is `Conflict`) and departs
   on it; the lot whose revoked booking was not replaced fails with
   `capacity_unavailable`. The owner's report shows the revocation reason;
   exact replay agrees.

Each was checked inline (not committed) to fail under a plausible regression:

| Mutation | Test 1 | Test 2 |
| --- | --- | --- |
| a re-timing hold adds nothing to the shift | fails | passes |
| superseded dues not withdrawn | fails | passes |
| a replaced booking still counts at departure | passes | fails |
| a revoked booking does not block departure | passes | fails |
| booking windows ignore the pending hold | fails | passes |

`gap_g19_movement_lifecycle_plugin` and `gap_g64_movement_delay_leg` pass
with their literals updated; the 0.19.0 check that a leg not yet departed
cannot be held now names a leg that is not current.

## Documentation delta

Done in this package: `docs/architecture.md` (transport and movement
paragraphs) and `crates/extensions/canwu-movement/README.md`.

For the release documentation pass (canonical pages and both website
languages):

- `docs/end-state.md`, the movement paragraph: after "Since 0.19.0 an
  owner, operator, or incident can hold a departed leg (`DelayLeg`)." add
  "Since 0.20.0 a hold can also keep a leg that has not departed at its stop
  without spending an itinerary revision, and with `retime_later_legs` it
  moves the later legs' departures and booking windows by the part of the
  hold that elapsed. A pool's custodian, or an application incident, can
  revoke a confirmed booking, and the owner replaces it with a new request
  that names it." In the capacity-pool sentence, "without preempting
  confirmed capacity" stays true of allocation.
- `website/src/content/docs/{,en/}architecture/routing-movement.mdx`: the
  `MovementOperation` row adds `RevokeBooking`; the `MovementIncidentV1` row
  adds `RevokeBooking`; "Leg timing" replaces the 0.19.0 hold paragraph with
  the `DelayLeg` and re-timing paragraph of `docs/architecture.md`, and the
  stale-packet sentence with the withdrawal; the booking paragraph adds
  revocation and `replaces`; the reports paragraph adds the schedule shift
  and revocations.
- `docs/versioning.md`, the release entry: the rows of
  [Persistence and replay](#persistence-and-replay) and the known limits
  below.
- `docs/terminology.md` and both website term pages: the rows below; the
  `movement lifecycle extension` row's "holds departed legs" becomes "holds
  legs before departure and under way".
- The engine-user skill's movement note (`canwu-engine-usage`, the 0.19.0
  `DelayLeg` line): add the departure hold, `retime_later_legs`,
  `RevokeBooking`, and `replaces`.

## Terminology candidates

| English | Code | 中文 (proposed) | Meaning |
| --- | --- | --- | --- |
| leg hold (widened) | `MovementOperation::DelayLeg`, `MovementLegHoldV1` | 路段滞留 | A hold on the current leg that moves its departure (a leg that has not departed) or its arrival (a departed leg) to an absolute instant no earlier than the due it replaced, without an itinerary revision. Later holds lengthen, shorten, or lift it; only the latest is kept. |
| scheduled due (replaces "scheduled arrival") | `MovementLegHoldV1::scheduled_at` | 排定时刻 | The due a hold replaced and never ends before: a departed leg's departure plus its planned duration, or the departure pending when the leg was first held. |
| schedule shift | `MovementOrderRecordV1::schedule_shift`, `retime_later_legs` | 行程顺延量 | How far the legs of the active revision that have not departed run behind their planned times by the holds that re-timed them; it moves their planned departures and booking windows, and a reroute resets it. |
| booking revocation | `MovementOperation::RevokeBooking`, `CapacityBookingStatus::Revoked`, `CapacityBooking::revocation` | 预订撤销 | A pool custodian's (or an application incident's) withdrawal of a confirmed booking, with its time and reason; the capacity returns to the pool, and the leg fails at departure unless the booking is replaced first. |
| booking replacement | `MovementBookingRequestV1::replaces` | 预订替换 | A new booking of the same leg that takes the place of one earlier booking, once; a pending or confirmed original is cancelled or released, and a lost one no longer counts at departure. |

## Known limits

- **One shift per order.** A re-timing hold moves every later leg of the
  revision by the same amount; a host that wants only some later legs moved
  reroutes. A reroute resets the shift.
- **Absorbed lateness is not re-timing.** Lateness that no re-timing hold
  caused (a hold without `retime_later_legs`, a booking window that opened
  late, an arrival past the next planned departure) is absorbed as in 0.19.0:
  the next leg departs on arrival and its booking window does not move,
  unless that leg's departure is then held.
- **The latest hold decides the re-timing.** Repeated holds of one due keep
  the first bound, and the latest hold's `retime_later_legs` decides whether
  the whole time elapsed since that bound is re-timed.
- **Booking windows move by the request.** A booking requested before a
  re-timing hold keeps its window. If the re-timed departure falls after it,
  the booking expires and the leg fails with `capacity_unavailable` unless
  its owner replaces it first; a replacement of a confirmed booking releases
  the original at once.
- **A revoked booking fails its leg at departure.** A revocation settled in
  the boundary of the leg's departure due fails the leg (operations settle
  before dues); an application that wants the leg to wait revokes earlier or
  holds the departure in the same boundary (an incident `DelayLeg`).
- **Revocation does not reach a request.** Only a confirmed booking can be
  revoked; a requested one is allocated against the pool as it stands.
- **The custodian learns booking identities from the host.** Pool
  custodians receive no movement report (E-43); the host reads the runtime
  to name the bookings it revokes.
- **Replacements count against `bookings_per_execution` (16).** Closed in
  0.21.0: a replaced or closed booking leaves its execution, and the bound
  counts only the bookings kept at once
  ([kept bookings](movement-kept-bookings.md)).
- **Withdrawal cost.** A boundary that replaces a future due reads the
  plugin's cancellable ingress once, which walks the run's pending ingress
  queue (about one entry per live execution for movement, plus every other
  source's queued items), and each withdrawal adds one cancellation record to
  the boundary's generated ingress.
- **Holds still count against outcome quotas** (1,024 per command holder and
  16,384 per runtime), as in 0.19.0.
