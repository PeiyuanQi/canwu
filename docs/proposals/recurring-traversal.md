# Recurring Traversal, Edge Conditions, and the Compiled Routing Index

Status: implemented in 0.18.0. Closes downstream gap G-24 and the routing row
of the 0.17.0 "Beyond fiscal" table in
[world-scale fiscal](world-scale-fiscal.md#beyond-fiscal-caps-that-block-the-target).

## Problem

**Timetables were absolute.** `TraversalModel::Departures` lists one
`DepartureSlot` per departure, so a ferry that sails every six hours for
fourteen months is about 1,700 slots per connection in every planning
snapshot, and a host that builds snapshots per request horizon rebuilds them
all. In the measured grid below, 72 ferry crossings with a 60-day horizon add
0.63 MB to a 1.03 MB snapshot.

**Conditions meant a new network.** A closure or a slowdown for a week (a
flood, a frozen river, a mud season) could only be expressed by editing the
connections and rebuilding the network.

**Every query paid for the whole network.** `plan_route` rebuilt the adjacency
index and hashed the whole snapshot on each call, and `RoutingCache::key`
hashed it again. On a 5,040-connection network one query cost 30.0 ms. The
measurement showed that the largest part was neither the rebuild nor the hash:
each search label carried its whole route as a `Vec<RouteLeg>` (three strings
per leg), copied on every relaxation and queue push, so a query cost grew with
the square of the route length (the 0.5 rail-chain baseline: 271 ms for ten
plans of 256 legs).

## Decision

1. **`TraversalModel::Recurring { period, offsets, duration }`.** A departure
   leaves at `EPOCH + k * period + offset` for every integer `k` (negative `k`
   included) and every offset; each takes `duration`. The earliest departure
   at or after any time is a binary search over the offsets, computed exactly
   in integer minutes (128-bit intermediates, Euclidean remainder, so times
   before the epoch work). The connection's `available_from` and
   `available_until` bound the season. A pattern anchored elsewhere folds the
   anchor into its offsets (`(anchor - EPOCH) mod period`).
2. **`EdgeConditionSet`**, a validated set of `EdgeConditionOverlay`s, each a
   half-open window `[window_from, window_until)` on one connection with an
   effect: `Closed` (no departure inside the window: the traveller waits for
   the first departure at or after its end) or `Adjusted` (duration and
   resource cost scaled per mille, rounded up; risk replaced when given). The
   effect applies by **departure instant**. The set is built once and passed
   to `RoutingIndex::plan_route_with_conditions`; nothing about the network is
   rebuilt. Closures on one connection are merged at build time; two
   adjustments on one connection may not overlap, so at most one is in force
   at any instant.
3. **`RoutingIndex`**, a compiled planning snapshot. Compiling validates the
   snapshot and its network, hashes the network once, and builds dense node
   ranks and a compressed adjacency; a query then costs only the search. The
   compiled network is shared behind an `Arc`, so an index clones cheaply, is
   `Send` and `Sync`, and `reobserve` moves it to a new read cut (observer,
   observed time, validity, knowledge cut) by hashing only the cut; the caller
   names the network revision it expects, and another revision is refused. A
   host keeps one index per network revision and compiles a new one when the
   network an observer knows changes. `plan_route` keeps its signature and
   compiles a borrowed index per call.
4. **A persistent route list in the search.** A label holds a shared pointer
   to its newest step; extending a label allocates one step. Route order on
   ties is unchanged: labels still order by arrival, risk, resource cost,
   transfers, then the legs from the origin (`Vec<RouteLeg>`'s `Ord`), now
   compared by walking both routes back to their shared prefix.

## Contract

| Item | Contract |
| --- | --- |
| `TraversalModel::Recurring` | `period > 0`, `duration > 0`; 1 to `MAX_RECURRING_OFFSETS` (4,096) offsets, strictly increasing, each in `[0, period)`; refused as `RoutingError::InvalidNetwork` |
| `TraversalModel::next_departure(at)` | public; the earliest departure at or after `at` and its duration, ignoring availability and conditions; O(log n) for `Departures`, `Piecewise`, and `Recurring` |
| `EdgeConditionOverlay` | `{ id: EdgeConditionRef, connection, window_from, window_until, effect }`; `window_from < window_until` |
| `EdgeConditionEffect::Adjusted` | `duration_per_mille` 1 to `MAX_CONDITION_PER_MILLE` (1,000,000), `resource_cost_per_mille` 0 to 1,000,000, `risk_per_mille` at most 1,000 |
| `EdgeConditionSet::new` | at most `MAX_EDGE_CONDITIONS` (65,536) overlays and `MAX_EDGE_CONDITIONS_PER_CONNECTION` (1,024) per connection; unique ids; adjustments on one connection do not overlap; refused as `RoutingError::InvalidConditions` (new variant). An overlay naming a connection the queried network lacks has no effect, so one set can serve several observers' networks; the host passes the conditions the observer knows |
| Per-query cost of conditions | one ordered lookup per relaxed edge plus O(log closures) and O(log adjustments); a closure loop passes at most one merged closure per lookup |
| `RoutingIndex` | `new(snapshot)`, `reobserve(topology_version, timetable_version, observer, observed_at, valid_until, knowledge_cut)` (another revision than the index's is refused as `InvalidSnapshot`), `plan_route`, `plan_route_with_conditions`, `unmatched_conditions(set)` (the overlays naming a connection the network lacks, so a misspelled id is checkable), `digest`, `network_digest`, `network`, the cut accessors, and `to_snapshot`; plans are identical to `plan_route` on the same snapshot, digests included |
| `RoutingNetwork::digest` | new; the content digest of the network |
| `PlanningSnapshot::digest` | now composed from the cut, the versions, and `RoutingNetwork::digest`, so an index re-cut by `reobserve` recomputes it without hashing the network |
| `RoutePlan.edge_conditions_digest` | new `Option<String>`: the set's digest, `None` when no set or an empty set was applied; skipped in JSON when `None`, so plans made without conditions serialize as before; included in the plan digest |
| `RoutingCache::index_key(index, conditions, request)` | new; does not hash the network; equals `RoutingCache::key` for the same snapshot without conditions |
| `ROUTING_ALGORITHM_VERSION` | `canwu-routing.v2`: routes, costs, and refusals are those of v1 for every valid network whose arrival times are representable, but snapshot and plan digests differ. (When a departure slot's arrival overflowed the time range, v1 skipped to the next slot; v2 treats the connection as unusable from that time.) |
| Correspondence | its plugin semantic hash and its `routing_connection` knowledge schema hash change (see below); the schema id stays at version 1 because existing payloads are byte-identical |

Validation now also runs when a route is planned: `plan_route` and
`RoutingIndex::new` refuse a hand-assembled network that `RoutingNetwork::new`
would refuse (duplicate ids, unknown endpoints, a malformed traversal model)
with `InvalidNetwork`, where 0.17.0 planned over it. This keeps a malformed
recurring pattern from reaching the lookup.

Correspondence stores route plans and planning-snapshot digests in its
records, so its record bytes change for identical input (`canwu-routing.v2`,
the composed snapshot digest, and the plan digest over
`edge_conditions_digest`), and its `routing_connection` knowledge now accepts
`{"type": "recurring", ...}`, which 0.17.0 refused. Both declared hashes
change. Like the other semantic-hash constants they are canonical 64-hex
values; the new ones are BLAKE3 of a stated string:
`a197a514…5685` for the plugin semantic hash, from
`canwu-correspondence plugin semantic hash 0.18.0: route plans of canwu-routing.v2`,
and `b9851021…6b1b` for the knowledge schema hash, from
`canwu-correspondence routing_connection knowledge schema 0.18.0: TraversalModel::Recurring accepted`.

## What can still fail a boundary

Routing is pure and returns `RoutingError`; whether a refusal fails a
boundary is decided by the caller's mapping. No new refusal reaches a boundary
by itself. `InvalidConditions` arises only in `EdgeConditionSet::new`, which a
host calls before planning. A malformed `Recurring` pattern joins the existing
class of malformed traversal models: a caller that maps `InvalidNetwork` to a
failed record (correspondence does, for a carrier's known network) should
validate connection payloads before it publishes them, as it already must for
unsorted departure slots.

## Costs

`cargo run --release -p canwu-routing --example routing_scale` on Windows
x86_64, AMD Ryzen 7 7800X3D, rustc 1.99.0. The grid is 36 by 36 nodes (1,296)
with 5,040 directed connections: roads of 30 to 119 minutes and 72 ferry
crossings every six hours, stated as 240 absolute slots each (a 60-day
horizon) or as one recurring offset. Eight corner-to-corner and cross-river
queries; times are per query, averaged over five rounds. The 0.17.0 column was
measured on `main` at `8fb922e` with the same workload; both versions return
the same legs and arrival for every query.

| Measurement | 0.17.0 | 0.18.0 |
| --- | --- | --- |
| Rail chain, 512 nodes, 1,021 connections, 10 cold plans | 271.4 ms | 5.8 ms |
| Grid with slots: snapshot JSON | 1.66 MB | 1.66 MB (format unchanged) |
| Grid with slots: `PlanningSnapshot::digest` | 1.85 ms | 1.87 ms |
| Grid with slots: `RoutingCache::key` | 1.85 ms | 1.86 ms |
| Grid with slots: cold `plan_route` | 30.0 ms | 3.5 ms |
| Grid with slots: `RoutingIndex::new` (snapshot clone included) | — | about 4 ms, once per network revision |
| Grid with slots: plan through the index | — | about 0.3 ms |
| Grid: `RoutingCache::index_key`, `reobserve` | — | under 0.001 ms each |
| Grid with recurring ferries: snapshot JSON | — | 1,029,852 bytes |
| Grid with recurring ferries: cold plan, compile, indexed plan | — | 2.6 ms, 3.0 ms, about 0.3 ms |
| Grid with recurring ferries and 568 conditions (ferries closed two days, every tenth road slowed for ten days): build the set, plan | — | 0.32 ms, 0.57 ms |

A query through a compiled index costs about a hundredth of a 0.17.0 query;
the remaining cold cost is the network hash and compile, paid once per network
revision by a host that keeps the index.

## Tests

- `tests/gap_g24_routing_recurring_and_conditions.rs`:
  `gap_g24_recurring_departures_match_absolute_slots` (every query minute over
  five cycles across the epoch against the equivalent slot list; a route
  waiting into the next cycle; the season bound),
  `gap_g24_recurring_patterns_are_validated` (each malformed pattern refused by
  `RoutingNetwork::new`, `plan_route`, and `RoutingIndex::new`),
  `gap_g24_edge_conditions_apply_by_departure_instant` (waiting out a closure,
  rounding up, risk replacement, nested closures, the window end, the recorded
  digest, an empty set), and `gap_g24_edge_condition_sets_are_validated`.
- `tests/routing_index_equivalence.rs`:
  `routing_index_plans_as_plan_route_and_keeps_leg_order_on_ties` (a tie where
  the route first leg by leg differs from the one with the smaller last and
  middle legs, for both algorithms; index and cold plans equal; cache keys
  equal; `reobserve` digests and its refusal of another revision).
  `unmatched_conditions` is checked in the conditions test above.

Each was checked to fail with its fix reverted (lookup off by one, no wrap to
the next cycle, a truncating remainder, zero accepted, closures ignored or not
merged, rounding down, a weakened overlap check, leaf-first route comparison,
no route tie-break, a digest without the topology version, no revision check
in `reobserve`, every overlay matched). A one-off differential check, not
committed, compared 0.18.0 with 0.17.0 on 40,000 random networks with fixed,
slotted, and piecewise edges, availability windows, and small budgets:
identical legs, costs, and refusals, budget refusals included; the independent
review repeated it on 200,000 networks.

## Deviations

- The filed overlay is a cost multiplier; this one also closes an edge and
  replaces its risk, applies by departure instant, and travels beside the
  snapshot per query rather than inside it, so `PlanningSnapshot` keeps its
  shape and existing hosts build it unchanged.
- `Recurring` has no anchor field (offsets are relative to the epoch, as
  filed) and its duration must be positive.
- The crate keeps no cache of compiled indexes keyed by caller-declared
  versions. A version string does not prove content: a caller may edit a
  network in place without changing its version (correspondence filters
  excluded connections that way before a reroute). The index itself is the
  cached compiled network; the host keys it by its own revision and moves it
  across read cuts with `reobserve`, which takes the expected revision and
  refuses another.
- The search representation was rewritten, beyond the filed fix, because the
  measurement showed it was the largest cost. The label-correcting duplicate
  check was removed: a candidate is its parent's route plus one leg, each
  label is expanded once, and connection ids are unique (enforced when the
  route is planned), so two candidates never hold the same legs and the check
  never matched.

## Known limits

- Routes depart as soon as possible at each node; there is no voluntary
  waiting. An adjustment can make an edge non-FIFO (leaving at the window's
  end can arrive before leaving just inside it), the same class as a
  `Piecewise` edge: FIFO Dijkstra then keeps the best as-soon-as-possible
  label per node, not a wait-optimal route.
- Conditions act on departures, not on a traversal already under way.
- One duration per pattern; a calendar month is not a fixed period (use slots
  or one pattern per month length).
- `reobserve` checks the revision the caller names, not the content: equal
  versions are trusted to name the same network.
- The default expansion budget (10,000 queue pops, stale pops counted) and the
  unbounded `RoutingCache` are unchanged; `Departures` has no slot cap.
- Correspondence plans with plain `plan_route`: it gets the faster search but
  neither edge conditions nor the compiled index. It still builds a snapshot
  from holder knowledge and hashes it for every plan (and again for its own
  evidence); adopting `RoutingIndex` and conditions there is a correspondence
  change.
- A malformed `recurring` connection payload in a carrier's knowledge still
  fails the boundary as `InvalidDomainRecord`, the 0.17.0 class for malformed
  traversal models.
