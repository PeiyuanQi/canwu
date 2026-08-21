# Routing and Transport Implementation Plan

Status: revision-1 implementation and verification checklist for
`routing-transport-mechanism.md`; M1–M3 are complete for the 0.5 extension
boundary. The unchecked items are intentionally deferred compatibility or
domain-adapter work, not missing pieces of the current vertical slice.

Baseline: `origin/main@77fbe49` on 2026-08-21. The information-flow work is
complete and remains the canonical owner of delivery-attempt lifecycle,
knowledge visibility, persistence, and replay.

## Invariants changed by this work

1. Route planning is deterministic and operates on an actor-relative,
   versioned planning snapshot.
2. A route estimate never rewrites `DeliveryAttempt.due_at`.
3. Reroutes create itinerary revisions; retries create new delivery attempts.
4. Transport execution owns legs, custody handoffs, capacity bookings, and the
   completion saga, but not the information ledger or simulation scheduler.
5. Derived route caches are rebuildable and excluded from authoritative state.
6. Every failed leg, reroute, handoff, booking transition, and completion
   request has an explicit identity and can be replayed from admitted evidence.

## Milestone 1: deterministic route and execution slice

- [x] Add pure `canwu-routing` network, endpoint, mode, traversal, policy,
  request, plan, and error types.
- [x] Support fixed durations, departure slots, availability windows, risk,
  resource cost, transfer limits, and arrival horizons.
- [x] Implement deterministic FIFO Dijkstra with stable tie-breaking.
- [x] Add `PlanningSnapshot` observer, knowledge cut, topology version, and
  expiry fields.
- [x] Add immutable itinerary revisions and leg execution records.
- [x] Add explicit custody handoff validation.
- [x] Add arrival-pending saga and information completion request bridge.
- [x] Add disaster failure and reroute without creating a new attempt.
- [x] Add stable completion operation keys for exact retry handling.

Gate: Wuxi-to-Beijing and nearby-recipient routes use the same API but may
select different modes, hops, capacity, or deadlines.

## Milestone 2: capacity and historical transport modes

- [x] Add persistent windowed `CapacityBooking` records and transitions.
- [x] Permit request cancellation and enforce expiry after the booking window.
- [x] Model rail, air, signal, river, and seasonal behavior through modes and
  traversal data rather than mode-specific router branches.
- [x] Keep allocation outside routing; transport records the selected booking
  result and evidence.
- [x] Re-export the transport boundary through `canwu-api`.
- [ ] Add a domain-specific capacity allocator once a second domain extension
  requires the shared reservation adapter.

Gate: competing claims have deterministic results and no booking mutation can
be hidden in a route-cache hit.

## Milestone 3: advanced routing and extension seam

- [x] Add bounded label-correcting search for explicitly non-FIFO traversal.
- [x] Add a derived, digest-keyed `RoutingCache` with rebuild semantics.
- [x] Keep military/logistics policy data outside core routing while allowing
  `MilitaryPosition`, risk, resource cost, and transfer limits as inputs.
- [x] Add deterministic expansion and horizon budgets.
- [x] Add a standalone large-network benchmark and publish routing-specific
  scaling baselines without treating them as cross-machine SLOs.

Gate: M3 cannot introduce a second scheduler, RNG, knowledge implementation,
global ID allocator, or authoritative cache.

## Documentation and compatibility

- [x] Record ownership, due-at/ETA, reroute/retry, handoff, disaster, and
  period-data semantics in the architecture proposal.
- [x] Update architecture, end-state, versioning, and bilingual website
  tutorial documentation.
- [x] Keep the additions additive to snapshot format 5; domain records remain
  the persistence boundary for transport execution.
- [ ] Add a future migration fixture if transport records become first-class
  snapshot fields instead of extension-owned records.

## Verification gate

- [x] Format and focused routing/transport/API/information tests pass.
- [x] Run workspace clippy with warnings denied.
- [x] Run the full workspace test suite and debug-client check.
- [x] Build Rust documentation and the bilingual website.
- [x] Obtain a separate website-copy review for every supported language.
- [ ] Commit one coherent change, push without force, and verify GitHub Pages.
