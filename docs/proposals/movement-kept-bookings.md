# Movement Executions Keep Only the Bookings That Bear on Their Legs

Status: implemented in 0.21.0. It closes downstream gap G-83 of
`opus-progress.md` (an execution books at most 16 lots in its life, closed and
replaced ones counted). It changes `canwu-movement` only; `canwu-transport`,
`canwu-api`, and `canwu-sim` are untouched. Same version only: no
compatibility work with 0.20.0 bytes or hashes.

"Today" means 0.20.0 as it is on `main`.

## Contents

1. [Problem](#problem)
2. [Invariants](#invariants)
3. [Design](#design)
4. [Refusals](#refusals)
5. [Persistence and replay](#persistence-and-replay)
6. [Record size](#record-size)
7. [What can still fail a boundary](#what-can-still-fail-a-boundary)
8. [Public API delta](#public-api-delta)
9. [Durable tests](#durable-tests)
10. [Documentation delta](#documentation-delta)
11. [Terminology candidates](#terminology-candidates)
12. [Known limits](#known-limits)

## Problem

`request_booking` (`lifecycle.rs`) refuses a booking once
`execution.bookings.len() >= MovementLimitsV1::CURRENT.bookings_per_execution`
(16), and `TransportExecution::request_booking` only appends. Nothing ever
takes a booking out of a live execution: a released, cancelled, expired,
failed, revoked, consumed, or replaced booking stays in `bookings` until the
execution retires. So the bound is a lifetime count:

- a leg that needs more than 16 lots (one booking per lot, because the
  allocator confirms a booking whole or not at all) cannot book them at once;
- a leg held for its lots and re-booked in place with `replaces` (0.20.0) can
  re-book only while its lost lots and their replacements stay within 16, so
  an 8-lot leg re-books once and a 15-lot leg never.

Raising the number alone would only move the wall: every revocation and its
replacement add two entries for the life of the execution.

## Invariants

- **K1 — an execution keeps only the bookings that bear on its legs.** A
  booking stays in `TransportExecution::bookings`, with its binding in
  `MovementState::bookings`, exactly while it is `Requested` or `Confirmed`,
  `Consumed` by the leg under way (status `Departed`), or lost (`Failed`,
  `Expired`, or `Revoked`) for a leg of the active revision that has not
  departed (`Planned`, `Booked`, `Loaded`, or `Waiting`) of an execution that
  is not terminal. Every other booking is **closed** and has left.
- **K2 — a replaced booking leaves when it is replaced.** A binding's
  `replaces` names a booking the runtime no longer keeps, and at most one kept
  binding names it.
- **K3 — consumed capacity stays counted.** A pool's `consumed` equals its
  kept consumed bookings plus `retired_consumption`, which now gains a
  booking's quantity when the booking leaves, not only when its execution
  retires.
- **K4 — the bounds are on what is kept.** An execution keeps at most
  `bookings_per_execution` (64) bookings and the runtime at most
  `bookings_per_runtime` (16,384), however many either books over its life.

## Design

### Which bookings leave, and when

The rule is one predicate over the runtime, `execution_keeps_booking(execution,
leg, status)` (K1). A booking that fails it leaves the execution in the same
step that closed it, before that step's candidate runtime is validated:

| How the booking closed | When it leaves |
| --- | --- |
| replaced by a request naming it (`replaces`), whatever its status | in the replacing `RequestBooking`, before the bound is checked |
| consumed by a leg that then arrived (or, under way, failed) | in the arrival or failure |
| released or cancelled (leg failed, reroute, cancellation, settlement) | in that operation or due |
| lost for a leg that failed (`capacity_unavailable` at departure, `FailLeg`) | in that failure |
| lost for a leg of a revision a reroute superseded | in the reroute |
| any booking of an execution that closed | in the closing operation or due |

Implementation: `apply_operation` ends with `remove_closed_bookings` for the
execution the operation names, as does every due settlement and the
`due_settlement_failed` fallback; `replace_booking` removes the original
itself. `allocate_pools` needs no sweep: it confirms or fails requests and
expires confirmed bookings, and every request and confirmed booking is for a
leg that has not departed (a departure consumes or waits for every confirmed
booking of its leg; a failure, reroute, or close releases them).

Removing a booking takes it out of the execution's `bookings` (keeping the
order of the rest) and its binding out of `MovementState::bookings`; a
consumed booking adds its quantity to `retired_consumption[pool]`, as
retirement already did. The pool's counters do not change.

**Evidence kept.** What a closed booking proved stays where retirement
already leaves it: the runtime versions that held it (with its allocation
evidence and revocation) stay in the domain record's history, consumed
capacity stays counted by its pool, and reports published while it was kept
stay in their holders' knowledge. The live runtime decides nothing from a
closed booking, so nothing a boundary reads is lost.

**Identity.** A kept booking's identity is taken, as before, and so is the
identity of a booking a kept replacement names: a request reusing it is a
`Conflict`, and a second replacement of it is a `Conflict` ("already
replaced"), as in 0.20.0. Without this, a new booking under a replaced
booking's identity would be read as the replaced one.

**Departure.** With replaced bookings gone, departure reads every kept
booking of its leg; the 0.20.0 skip of replaced bookings is removed (K2 makes
it dead).

### The bounds (K4)

`bookings_per_execution` now bounds what an execution keeps at once. It counts
everything the execution keeps, including the lots the leg under way consumed
and lost lots not yet replaced, so that the bound is the record's bound; a
replacement takes its original's place, so re-booking in place never counts a
lot twice. It is raised from 16 to 64 so that a leg of more than 16 lots
books them at once, with room for a later leg's lots booked while the
current leg is under way (two legs of 32 lots).

A new `bookings_per_runtime` (16,384) bounds the runtime's kept bookings
across executions. It equals 0.20.0's implied maximum (1,024 orders × 16
bookings), so raising the per-execution bound does not raise the worst-case
runtime record.

### Rejected alternatives

- *Count only `Requested` and `Confirmed` against the bound and keep the
  rest* (the row's first shape). The record would still grow with every
  revocation and replacement for the life of the execution, so it needs a
  second, lifetime bound, which is the gap again. A lost booking that is not
  replaced must stay anyway, because it fails its leg at departure.
- *Keep a tombstone or a digest chain of closed bookings.* No rule reads
  them; the record history keeps each runtime version, as it does for retired
  executions; a tombstone list grows with the execution's life.
- *Move consumed bookings out at departure.* A host reads the lots the leg
  under way carries; keeping them until arrival costs at most one leg's lots.
- *Keep a closed execution's bookings until it retires.* They decide nothing
  after close, and one rule for every execution is simpler to validate.
- *A per-leg bound.* With 64 legs it would let one execution keep 64 times as
  many bookings as one leg needs.

## Refusals

Every refusal is a recorded rejection on the operation's outcome; the
boundary stands. `RequestBooking`, in order (changes in bold):

1. a bounded tie-break key, a known pool with the booking's resource
   (`InvalidOperation`, `NotFound`), as before;
2. the identity is not taken: neither a kept booking **nor named by a kept
   replacement's `replaces`** (`Conflict`);
3. a known execution that is not arrival-pending (`NotFound`,
   `InvalidState`), an unstarted leg of the active revision (`InvalidState`),
   and the window inside the leg's re-timed planned window
   (`InvalidOperation`, `LimitExceeded`), as before;
4. `replaces` names a kept booking of the same execution and leg
   (`InvalidOperation`), **or one that left because a kept booking of that leg
   replaced it** (`Conflict`), in a status that can be replaced
   (`InvalidState`); the original then leaves;
5. **the execution keeps fewer than 64 bookings and the runtime fewer than
   16,384 (`LimitExceeded`; 0.20.0 refused the 17th booking of the
   execution's life with `InvalidState`)**.

## Persistence and replay

| Item | 0.20.0 | 0.21.0 |
| --- | --- | --- |
| `MovementState` (runtime record payload) | — | shape unchanged; `validate` refuses an execution that keeps a closed booking (K1), a `replaces` naming a kept booking or named twice (K2), more than 64 kept bookings in an execution or 16,384 in the runtime (K4) |
| `MovementBookingBindingV1::replaces` | names an earlier booking of its leg still in the runtime | names a booking that has left the runtime |
| `MovementState::retired_consumption` | consumption of retired executions' bookings | consumption of every booking that has left the runtime |
| `TransportExecution::bookings` (in the runtime) | every booking of the execution's life | the kept bookings, in request order |
| `MovementReportDetailV1::bookings` (report payload) | every booking of the execution | the kept bookings; payload shape and the `movement_report` schema hash (`33d6f3ab…ed28`) unchanged |
| `MovementLimitsV1` (not persisted) | `bookings_per_execution` 16 | `bookings_per_execution` 64; `bookings_per_runtime` 16,384 (new field) |
| `MOVEMENT_SEMANTIC_HASH` | `b211c92a…abbc` | `e01d702b…826f`, BLAKE3 of `canwu-movement plugin semantic hash 0.21.0: closed and replaced bookings leave their execution; 64 kept bookings per execution, 16384 per runtime` |
| Snapshot, continuation, and journal formats | — | unchanged |

The semantic hash changes, so a run saved by 0.20.0 with the movement plugin
does not restore; under the same-version policy none is expected to. A
runtime that 0.20.0 wrote with closed bookings would also be refused by
`validate`.

Replay: whether a booking leaves is a function of the runtime alone (its
status, its leg's status and revision, the execution's state), evaluated in
the operation or due that changed them, so a live run, a sealed run, a
continuation restore, and exact replay remove the same bookings in the same
boundary. No decision reads retained evidence.

## Record size

Measured with `serde_json` on the runtime types, with every integer at its
largest: one kept booking (its `CapacityBooking` and its binding with
allocation evidence) is about 1.0 KB with short labels and at most about
2.5 KB with every label (resource, pool, tie-break key, revocation reason) at
the 256-byte text bound.

- **An execution's bookings: at most 64 kept, so ≤ about 68 KB (≤ about
  163 KB at maximal labels), whatever it books over its life.** In 0.20.0 the
  bound was 16 for life (≤ about 17 KB, ≤ about 40 KB).
- **The runtime's bookings: at most 16,384 kept, ≤ about 17 MB (≤ about
  42 MB at maximal labels)**, the same count 0.20.0 allowed (1,024 orders × 16),
  so the worst case does not grow.
- An owner or operator report lists the kept `CapacityBooking`s (about 350 B
  each with short labels, ≤ about 850 B at maximal labels): 64 add about
  22 KB, ≤ about 54 KB at maximal labels.

## What can still fail a boundary

Nothing new.

- Every step that closes a booking removes it before its candidate is
  validated (operations and dues, each validated on its own candidate, and the
  `due_settlement_failed` fallback); a failure of the removal itself
  (`retired_consumption` overflowing `u64`) rejects the operation or falls
  back like any other error of that step. `allocate_pools` produces only kept
  statuses (above), so the phase-8 validation of K1 cannot be reached by the
  plugin's own transitions.
- What could already fail a movement boundary still can: a runtime record
  that fails to decode, validate, or encode (a corrupted or hand-edited root,
  now including one that keeps a closed booking), and the runtime revision
  overflowing. A scenario's initial runtime that keeps a closed booking is
  refused at installation (`into_initial_record`) and activation, not in a
  boundary.

## Public API delta

- `canwu-movement`: `MovementLimitsV1::bookings_per_runtime` (new field);
  `MovementLimitsV1::CURRENT.bookings_per_execution` is 64;
  `MOVEMENT_SEMANTIC_HASH` (new value). Rustdoc of
  `MovementLimitsV1::bookings_per_execution`,
  `MovementBookingRequestV1::replaces`, `MovementBookingBindingV1::replaces`,
  `MovementState::retired_consumption`, `MovementState::validate`, and
  `MovementReportDetailV1::bookings` states the new meaning.
- Behaviour: closed and replaced bookings leave the execution (K1, K2); a
  request past the bound is `LimitExceeded`, not `InvalidState`; a request
  reusing a replaced booking's identity while its replacement is kept is
  `Conflict`.
- No `canwu-api` or `canwu-sim` change, so no re-export or `gated!` change.
  The new field breaks struct literals of `MovementLimitsV1` (a host reads
  `CURRENT`), as any 0.x minor release may.

### Upgrading from 0.20.0

- A host may book more than 16 lots for one leg (up to 64 kept bookings per
  execution) and re-book a lost lot in place with `replaces` every time it is
  lost; a leg no longer has to wait for lots it cannot book again.
- `TransportExecution::bookings` and the owner's report list only the kept
  bookings. A host that read a booking's final status from the runtime after
  it closed (consumed after its leg arrived, released, cancelled, replaced, or
  lost for a leg that failed) reads it from a report published while the
  booking was kept or from the record history, or records it when it closes
  the booking; a replaced booking is gone from the runtime when its
  replacement is admitted, and a settled or cancelled execution keeps no
  booking.
- Treat `LimitExceeded` on `RequestBooking` as "the execution or the runtime
  keeps as many bookings as it may" (0.20.0 answered the 17th booking of a
  life with `InvalidState`).
- A hand-built runtime (`MovementState::into_initial_record`) keeps no closed
  booking, and a binding's `replaces` names a booking not in the runtime.

## Durable tests

`crates/extensions/canwu-movement/tests/gap_g83_movement_kept_bookings.rs`,
**`gap_g83_closed_and_replaced_bookings_leave_their_execution`**: one
two-leg journey on a pool of 200. Its first leg books 20 lots at once; three
times the custodian revokes all 20 and the owner books each again in place
(80 bookings over the leg's life), and after each round the execution keeps
exactly the 20 replacements, the pool holds 20, and each binding names the
booking it replaced. A second replacement of a replaced booking and a new
booking under its identity are `Conflict`. The second leg's lots fill the
bound of 64; one more is `LimitExceeded` and the boundary stands. Under way
the first leg's 20 consumed lots are kept with the second leg's 44; when it
arrives they leave, `retired_consumption` and the pool's `consumed` count 20,
and the refused lot now fits. The settled execution keeps no booking and the
pool counts all 65 consumed lots; a run restored after the first arrival and
exact replay agree.

Each was checked inline (not committed) to fail under a plausible regression:

| Mutation | Result |
| --- | --- |
| a replaced booking stays in the execution | fails |
| closed bookings never leave (and validation allows them) | fails |
| consumed bookings leave at departure | fails |
| a replaced booking's identity can be reused while its replacement is kept | fails |
| a consumed booking leaves without `retired_consumption` | fails |
| the 0.20.0 bound of 16 | fails |

`gap_g64_movement_delay_leg` and `gap_g73_g79_g80_movement_holds_and_revocation`
pass with four assertions updated: an expired booking leaves with the leg it
failed, a settled execution keeps no booking (the pool still counts the
consumed one), a replaced revoked booking leaves its execution, and it appears
in the report published before its replacement.

## Documentation delta

Done in this package: `docs/architecture.md` (the movement paragraph),
`crates/extensions/canwu-movement/README.md` (operations, settlement, reports,
retention and limits), and the rustdoc above.

For the release documentation pass (canonical pages and both website
languages):

- `docs/end-state.md`, the movement paragraph: after "the owner replaces it
  with a new request that names it." add "Since 0.21.0 an execution keeps only
  the bookings that still bear on its legs, so a leg books many lots at once
  and re-books a lost one in place as often as it is lost."
- `website/src/content/docs/{,en/}architecture/routing-movement.mdx`: the
  capacity-pool paragraphs add the kept-booking rule (K1, K2) and that
  consumed capacity stays counted; the limits table row "Legs, itinerary
  revisions, handoffs, and bookings per execution | 64, 8, 16, and 16" becomes
  "Legs, itinerary revisions, and handoffs per execution | 64, 8, and 16" and a
  new row "Bookings an execution and the runtime keep at once | 64 and 16,384"
  (中文："每个运输执行的路段、行程修订和交接数 | 64、8、16" and "每个运输执行和整个运行时同时在持的预订数 |
  64、16,384"); the owner-report row says "the bookings it keeps".
- `docs/versioning.md`, the release entry: the rows of
  [Persistence and replay](#persistence-and-replay), the refusal change, the
  upgrading notes, and the known limits below.
- `docs/terminology.md` and both website term pages: the rows below; the
  `booking replacement` row's "a lost one no longer counts at departure"
  becomes "the original leaves the execution".
- The engine-user skill's movement note: an execution keeps at most 64
  bookings at once and closed or replaced ones leave it.

## Terminology candidates

| English | Code | 中文 (proposed) | Meaning |
| --- | --- | --- | --- |
| kept booking | `MovementLimitsV1::bookings_per_execution`, `bookings_per_runtime` | 在持预订 | A booking an execution still keeps because it bears on a leg: requested or confirmed, consumed by the leg under way, or lost for a leg that has not departed and not yet replaced. Only kept bookings count against the booking bounds. |
| closed booking | — | 已了结预订 | A booking that no longer bears on any leg: released, cancelled, or replaced, consumed by a leg that arrived or failed, or lost for a leg that failed or was superseded or whose execution closed. It leaves its execution, and its binding the runtime, in the step that closed it; consumed capacity stays counted by its pool. |

## Known limits

- **Closed bookings are not in the live runtime.** A booking's final status
  is in the reports published while it was kept and in the record history; a
  booking that closes in the boundary it was decided (an allocation failure
  for a leg whose departure settles in the same boundary) appears in no
  report.
- **Identities are reserved only while kept.** A replaced booking's identity
  is free again once its replacement leaves, as a booking's identity already
  was after its execution retired; a host that wants booking identities
  unique for the run keeps them so.
- **The bound counts the leg under way.** Its consumed lots count with the
  later legs' bookings, so a host that books the next leg while the current
  one is under way needs room for both (two legs of 32 lots); a lost lot
  counts until it is replaced or its leg fails.
- **One runtime-wide booking bound** shared by every execution, without a
  per-owner quota.
- **Identity check cost.** Each `RequestBooking` scans every kept binding of
  the runtime (at most 16,384) for a `replaces` naming its identity, the same
  order as the per-operation copy and validation of the runtime.
- **A pool window can now be narrowed past a consumed booking that left** (the
  `OfferPool` window check reads kept bookings), as it could past a retired
  execution's; the pool's `consumed` still holds the quantity.
- **Report size.** A report of an execution that keeps close to 64 bookings
  and has close to 64 legs with labels near 256 bytes can pass the 64 KiB
  knowledge payload limit; it is withheld with
  `canwu.movement.report_withheld.v1`, as any oversized report is.
