# Correspondence Orchestration / 通信编排机制

Status: implemented experimental 0.5 milestone.

This proposal defines the reusable integration between communication demand,
holder-relative routing knowledge, transport execution, and the neutral
information lifecycle. The first implementation is the unpublished
`canwu-correspondence` crate and its runnable Wuxi delivery example.

## Architectural position

`canwu-correspondence` is a **domain extension** because it owns correspondence
intent, address resolution, route acceptance, incident policy, and the evidence
that joins several generic mechanisms. It is also a **simulation plugin** because
its authoritative rules register commands, ingress, schemas, and boundary
systems with `canwu-sim`. These are different axes: domain ownership describes
what the crate means; `SimulationPlugin` describes how it executes.

The crate depends on `canwu-api` and `canwu-information`. Neither the public API nor
the information extension depends back on correspondence, so the workspace
dependency graph remains acyclic.

| Owner | Responsibility |
| --- | --- |
| `canwu-routing` | Pure deterministic route search over an immutable `PlanningSnapshot`; no dispatch, attempt, RNG, booking, or mutation ownership. |
| `canwu-transport` | Persistable route execution, itinerary revisions, leg state, custody handoffs, capacity-booking types, and arrival-pending saga primitives. |
| `canwu-information` | Transport-neutral content, representation, addressed `Dispatch`, `DeliveryAttempt`, `Access`, immutable deadlines, and atomic dispatch activation. |
| `canwu-correspondence` | Communication opportunity, admitted correspondence intent, holder-relative address/network resolution, accepted route, execution progress, reroute policy, interception bridge, and exact cross-extension evidence. |
| Application, channel, and infrastructure adapters | Compose content and channel records, prepare the addressed dispatch, supply period-specific network/address knowledge, reserve scarce capacity, and expose player or AI decisions. |
| `canwu-sim` | Authority admission, canonical ingress, ordered scheduling, operation-keyed RNG, atomic boundary commits, cross-plugin ingress validation, persistence, and exact replay. |
| `canwu-api` | Public API and shared serializable contracts; no correspondence policy. |

## Authoritative lifecycle

1. An application or channel adapter creates the content/channel records and a
   single-recipient `Prepared` addressed `Dispatch`.
2. A decision ticket, direct decision-backed command, or selected automatic
   `CommunicationOpportunity` admits an `InitiateCorrespondenceRequest`.
3. The correspondence plugin currently requires the carrier holder to be the
   sender. It reads only that sender-owned address and routing knowledge,
   creates a deterministic `PlanningSnapshot`, calls the pure router, and
   persists the accepted `RoutePlan` and exact knowledge cut.
4. One information operation atomically changes the dispatch to `Active` and
   creates exactly one initial `DeliveryAttempt`. The attempt's `due_at` is the
   admitted logical deadline; route ETA never rewrites it.
5. The plugin advances `TravelLeg` equivalents through `TransportExecution`.
   Every carrier or mode boundary produces an explicit custody `Handoff`.
6. On-time final arrival completes the attempt and then the dispatch through two
   idempotent information operations. A failed or late attempt leaves the
   dispatch `Active` for explicit recovery. Correspondence stores the resulting
   exact `DomainRecordVersionRef` values rather than guessed revisions.

`DeliveryAttempt` is the recipient-facing delivery lifecycle. `RoutePlan` is a
planning result. A transport itinerary revision is an execution history. A
handoff transfers physical custody; a knowledge relay records information
propagation. None of these terms is interchangeable.

## Local and long-distance delivery

The same correspondence contract accepts arbitrary origin and destination
nodes. The runnable fixture branches to select its local or long-distance input
data, but the correspondence plugin has no Beijing-specific planning path.
Holder-relative address knowledge resolves the recipient and network knowledge
determines the available route:

- local Wuxi delivery accepts one horse leg and settles after 30 minutes;
- Wuxi-to-Beijing accepts a scheduled rail leg followed by an explicit horse
  final-mile leg and settles after 3,060 minutes.

Random communication demand is represented by bounded candidate opportunities.
The operation key independently determines whether an opportunity occurs and,
for automatic opportunities, which candidate is selected. Consuming that
opportunity and starting correspondence are tied by exact persisted evidence,
so reload cannot select a different recipient.

## Period-neutral networks

The common abstraction is a time-dependent connection graph, not a modern road
map. Network adapters express historical meaning as validated data:

| Network | Typical connection data | Important adapter policy |
| --- | --- | --- |
| Ancient post or courier | foot, horse, river, post-station transfer, daylight or seasonal availability | relay custody, rest, feed, security, and local final mile |
| 1900/1940 railway | stations, published departures, transfer windows, mixed rail/road legs | period timetable, gauge or operator boundaries, service disruption |
| Modern rail or air | scheduled services and terminal transfers | confirmed booking, check-in/cutoff, cancellation, and capacity admission |
| Telegraph or another signal | office-to-office signal connections plus physical collection/delivery | encoding, office access, operating hours, interception, and final mile |

This unifies earliest-arrival planning and leg execution without asserting that
a horse, train seat, aircraft hold, and telegraph circuit have identical
operational rules. Channel and infrastructure adapters retain those rules.

## Incidents, rerouting, retry, and interception

Disasters are evaluated at explicit operation boundaries with operation-keyed
RNG. A triggered disaster fails the current leg, records blocked connections,
and replans from the current endpoint against a new holder-relative knowledge
cut. Legs remain append-only and revision-qualified, so earlier handoffs never
point to removed records. Each accepted revision persists its read cut, address
source record, planning digest, disruption overlay, and ingress evidence. The
replacement revision also records a custody handoff from the failed segment to
its new first segment, with failure time and delivery-attempt evidence.
Incidents that arrive before an active transport attempt, while an information
transition is pending, or after arrival are retained as suppressed incident
evidence and do not mutate the execution. The same `DeliveryAttempt` continues
under the successor itinerary revision. Stale
scheduled progress is ignored by its persisted sequence number; internal
progress also requires a reserved boundary cause and cannot start or complete a
leg before its planned time.

A **retry** is different. While the current attempt is `WaitingForRoute` with
transport `ReplanPending`, a sender-authorized `resolve_correspondence_v1`
decision can replan that same attempt after knowledge changes. After an attempt
has terminally failed, the valid actions are to create a contiguous successor
`DeliveryAttempt` with a new deadline and fresh `TransportExecution`, or to
finalize the failed dispatch. The plugin never silently invents a retry or
changes the prior attempt's deadline.

Interception creates an information `Access` operation for the interceptor.
It does not by itself stop, destroy, decode, or redirect the original delivery.
Those consequences require explicit domain policy and evidence.

## Determinism and persistence

Every random opportunity and incident uses a stable operation key and a
canonical input hash. Reusing a key with changed logical input is rejected.
Accepted routes, append-only route revisions and legs, per-revision knowledge
read cuts, recovery commands, information operation IDs, exact record versions,
incidents, handoffs, and progress sequence are authoritative records. Snapshot
restore therefore resumes pending ingress;
exact replay consumes the same decisions and draws without invoking an external
service or rereading live map data.

Cross-plugin work uses declared canonical ingress. The runtime validates the
exact `(target_plugin, packet_type)` pair and persists the plugin descriptors;
it does not require the target plugin to be the producer.

## Capacity and scale limits

Route search remains side-effect free. The current request contract exposes
only `CorrespondenceCapacityAdmission::Unconstrained`, and the plugin therefore
executes only explicitly unconstrained routes. It neither checks nor persists a
capacity booking. Supporting scarce rail, air, vehicle, courier, or signal
capacity requires a future admission variant backed by exact
`canwu-transport::CapacityBooking` or `canwu-sim` reservation evidence before a
constrained leg can start. The current example is not evidence of a universal
capacity allocator.

The current vertical slice bounds one communication opportunity to 64
candidates and one carrier planning read to 1,000 holder records. It rejects a
continuation rather than silently planning from a truncated knowledge set.
Large simulations should partition routing knowledge by region/network and add
rebuildable indexes or a versioned bounded query adapter before increasing
these limits. The bootstrap knowledge-seed ingress is fixture support; a
production adapter should publish superseding facts so historical heads do not
grow without bound.

## Compatibility

The new crate and domain records are additive under snapshot format 5. A saved
run containing correspondence state requires the exact plugin package version,
semantic hash, registered record schemas, ingress contracts, and boundary
contracts at rehydration. Changing payload meaning, operation-key semantics, or
cross-plugin saga order requires a new semantic hash and an explicit migration.

This milestone advances the public transport behavior to
`canwu-transport.v3`: rerouting retains append-only, revision-qualified leg
history, records failure time, and can preserve a custody transfer from a
failed leg into the first leg of a replacement revision. Persisted v1 or v2
transport data requires the matching executable or an explicit domain
migration; it is never silently reinterpreted as v3.

## Verification evidence

Run the end-to-end example:

```text
cargo run -p canwu-correspondence --example routed_correspondence
```

The integration tests cover local and long-distance delivery, explicit handoff,
post-handoff disaster rerouting with retained leg evidence, no-route recovery
after a knowledge update, successor-attempt retry, explicit failed-dispatch
finalization, external progress rejection, carrier-read authority,
non-terminating interception, deadline miss without deadline rewriting,
automatic demand selection, snapshot restore, and exact replay.
