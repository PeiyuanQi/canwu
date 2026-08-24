# Routing and Transport Mechanism

Status: implementation design for the 0.5 routing and transport vertical
slice. The design was checked against `origin/main@77fbe49` and reviewed by an
independent senior game-engine designer before implementation.

## Scope

This extension answers two different questions:

1. Given the facts visible to an observer, which route is currently planned?
2. Given that plan, how does a physical or signal delivery execute, consume
   capacity, hand over custody, recover from failure, and complete the linked
   information attempt?

Routing is a pure calculation. Transport is a durable execution record. The
simulation scheduler, information ledger, world truth, and knowledge policy
remain owners of their existing contracts.

## Ownership boundary

| Area | Owner | Rule |
| --- | --- | --- |
| topology and timetable facts | a reference integration or domain extension | authoritative typed domain records |
| actor-relative planning facts | `canwu-routing` input | `PlanningSnapshot` includes observer, read cut, and validity |
| route calculation | `canwu-routing` | deterministic, side-effect free, rebuildable cache only |
| leg execution and custody | `canwu-transport` | durable domain records, explicit transitions |
| delivery attempt lifecycle | `canwu-information` | one attempt per recipient/retry |
| admission, scheduling, rollback, replay | `canwu-sim` | canonical ingress and boundary transactions |
| presentation | host/client | never enters authoritative routing state |

The dependency direction is therefore `world -> routing -> transport`, with
transport calling back into information only through an admitted completion
operation. Routing must not mutate capacity or information records, and
transport must not decode another actor's knowledge to make a plan.

## Frozen semantics

- `PlanningSnapshot.observed_at` is the knowledge/read cut used to plan. A
  `valid_until` expiry makes stale plans explicit instead of silently treating
  current truth as known.
- `RoutePlan.estimated_arrival_at` is an estimate for execution. It is not the
  information lifecycle deadline.
- `DeliveryAttempt.due_at` is the immutable logical completion deadline. A
  later ETA can cause a failure or retry; it must not rewrite the attempt's
  original deadline.
- A reroute creates a new immutable `ItineraryRevision` and supersedes the old
  revision. It does not create a new information attempt.
- A retry creates a new `DeliveryAttempt` version. It must use a new completion
  operation key and may select a new route.
- A handoff is a custody transfer between two executed legs. It is not a
  knowledge relay, publication, or automatic information copy.
- Arrival enters `ArrivalPending`; only an admitted completion operation may
  reconcile the transport saga with the information lifecycle.
- Capacity bookings are persistent windowed records. A derived allocator may
  choose winners, but confirmed, consumed, released, expired, cancelled, and
  failed states must remain auditable.

## Period and mode data

The mechanism is period-neutral. A historical content package supplies the
network and traversal data:

- foot, horse, and road vehicle use fixed or piecewise travel durations;
- river and sea routes use seasonal or departure-slot traversal;
- 1900 and 1940 railways use timetable slots, station nodes, and capacity
  bookings rather than a special railway algorithm;
- air routes use airport nodes, flight slots, weather availability, and
  bookings;
- telegraph or other signal systems use `TransferMode::Signal`, with signal
  latency, office availability, and interception/risk modeled as data;
- an ancient courier network uses relay-station endpoints, custody handoffs,
  disaster failures, and a new itinerary revision after recovery.

The same delivery lifecycle can therefore represent Wuxi to Beijing, Wuxi to a
nearby recipient, railway dispatch, air dispatch, telegraph transmission, or
an ancient relay. Historical realism belongs in content and domain policies,
not in the generic router.

## Determinism, knowledge, and replay

Connection, endpoint, leg, booking, handoff, and revision identities are
explicit and ordered. FIFO routes use deterministic Dijkstra. Non-FIFO or
time-dependent content must opt into the bounded label-correcting algorithm.
Search budgets, transfer limits, risk limits, and horizon limits are part of
the policy, so a failed search is a deterministic result rather than an
unbounded runtime hazard.

`RoutingCache` is derived and rebuildable from the planning snapshot digest,
request, and policy. It is never authoritative state and is not needed for
save/load or exact replay. Execution transitions are domain records driven by
canonical ingress. Completion uses an operation key derived from execution,
itinerary revision, and delivery-attempt version, so exact retries do not
duplicate information completion.

## Failure and disaster handling

The router does not invent disasters. A world or transport system records an
explicit leg failure with time, reason, and evidence. The execution then enters
`ReplanPending`; a new planning snapshot is taken, a new itinerary revision is
calculated, and the old revision is superseded. Capacity already consumed is
released or compensated according to its booking policy. If no admissible
route exists before the information deadline, the information extension
records the failed attempt and may create a new retry attempt.

## Milestone boundary

M1 is the smallest end-to-end slice: a static or scheduled directed graph,
one route plan, immutable revision history, legs, handoff, arrival-pending
saga, one disaster reroute, and deterministic completion evidence.

M2 adds persistent capacity booking and data-driven rail, air, signal, and
seasonal traversal. It tests contention and release without moving allocation
into the router.

M3 adds bounded non-FIFO label correction, derived route caching, benchmark
workloads, and an extension seam for military/logistics policies. It does not
add a second scheduler, RNG, knowledge ledger, global ID allocator, or
authoritative cache.

The current 0.5 implementation intentionally does not include a geographic
index, automatic map import, multi-commodity optimization, or a universal
historical dataset. Those are content and host concerns that can be added
without changing the routing and transport contracts.
