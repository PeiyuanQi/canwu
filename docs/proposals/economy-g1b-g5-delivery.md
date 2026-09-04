# Economy G1b-G5 Delivery Design

Status: draft for independent review

This document turns the accepted production-economy boundary into a complete
Canwu-repository delivery. It does not add production, markets, historical
goods, or military doctrine to `canwu-sim`. The delivery uses optional domain
extensions and a replaceable reference integration built only on the public
`canwu-api` contract.

## 1. Decision and invariant

The authoritative material invariant is:

```text
closing account balances + closing active-transfer escrow
= opening account balances + opening active-transfer escrow
+ admitted production and external inflow
- admitted consumption, loss, and external outflow
```

Reservations, price observations, credit, work progress, readiness, and
knowledge are not physical material. They may cite material evidence but must
not become a second writable balance.

The repository will add five packages:

| Package | Layer | Responsibility |
| --- | --- | --- |
| `canwu-resource` | published experimental then promoted optional domain extension | conserved accounts, demands, reservations, balanced transfers, consumption, losses, fulfillment evidence, holder-relative reports |
| `canwu-production` | published optional domain extension | process revisions, sites, facilities, work orders, production runs, work in progress, maintenance, damage, repair, commissioning and retirement |
| `canwu-economy-reference-content` | published reference content pack | versioned synthetic, archetype and sourced profiles, localization, coverage cells, model cards, effective periods and content hashes; no solver or runtime mutation |
| `canwu-force-supply-reference` | independently compilable `publish = false` reference integration | replaceable force supply, consumption intent, readiness/consequence and holder-report plugin proving a second resource consumer |
| `canwu-economy-reference` | `publish = false` reference integration | runnable grain, repair, workshop, force-supply, local-scarcity and evidence-qualified price-pressure compositions; no privileged runtime access |

The dependency boundary is exact:

```text
canwu-resource -> canwu-api
canwu-production -> canwu-api + canwu-resource + canwu-technology
canwu-economy-reference-content -> canwu-api + resource + production
                                  + technology typed definitions only
canwu-force-supply-reference -> canwu-api + resource + reference-content
canwu-economy-reference -> canwu-api + resource + production
                         + force-supply-reference + reference-content
                         + reference-world + routing + transport + technology
```

No extension depends on `canwu-sim`, and `canwu-api` does not re-export the
extensions. G5 deliberately does not
create `canwu-market`; authoritative order books, contracts and credit remain a
future gate requiring a real product need and a separate design review.

All five workspace packages use the repository's lockstep `0.10.0` version;
the three published packages are released together, while both integrations
remain `publish = false`. Format 8 does not change because no kernel wire
structure changes, but a 0.9 runtime save is not silently loaded under the
exact 0.10 plugin semantic environment. This delivery provides no implicit
save migration.

## 2. Cross-cutting contract

All newly introduced economy IDs are typed newtypes containing validated,
namespaced strings. Existing public IDs retain their owning crate's exact
representation. In particular, numeric `canwu-transport` identifiers are never
stringified or redefined: economy records embed a typed `TransportExecutionLink`
containing `TransportExecutionId`, `ItineraryRevisionId`, and when applicable
`LegExecutionId`, `HandoffId` and `CapacityBookingId`.
All authoritative quantities, prices, ratios, progress and quality values are
integers or explicitly scaled fixed-unit integers. Definitions and policies
are immutable revisions; dynamic records cite exact revisions.

Every command:

1. enters through tracked canonical command ingress;
2. checks issuer authority, expected revision, effective time and idempotency;
3. enqueues a versioned plugin-ingress packet;
4. is reduced by a declared boundary system;
5. writes only records owned by that plugin;
6. commits causal evidence atomically or records a terminal domain rejection;
7. publishes only holder-relative knowledge after authoritative settlement.

Every enabled G exposes at least one versioned `DecisionTicket` with two
non-equivalent current options before the consequential command is admitted.
The ticket contains holder-relative facts and option descriptors, never mutable
state or an undisclosed authoritative account. Two forks from the same
pre-decision snapshot must demonstrate different explainable outcomes, while
each branch must exactly replay its own recorded ticket, attempt, trace,
command, transaction and reports.

No extension receives a mutable runtime reference. Cross-extension composition
uses exact domain-record versions, immutable adapter-result packets, typed
projections, or commands. One plugin never mutates another plugin's records.

Every package defines canonical capacity limits for records, mutations per
transaction, candidate scans, report fan-out and archive retention. Hot paths
are bounded by active/dirty records, never total historical catalog size or an
all-observer cross product.

Each runtime extension uses stable shards and due/dirty indexes. Terminal
operations leave bounded hot receipts and move through the plugin-owned
archive contract. Ordinary transactions never scan or rewrite cold history.
Restore validates hot state, archive roots and cross-references before
rebuilding every derived index.

Snapshot restoration registers the exact plugin descriptors and semantic
hashes, validates every owned record and cross-reference, rebuilds derived
indexes from canonical records, and rejects the candidate before live-state
replacement when any invariant fails. Exact replay consumes recorded ingress
and evidence. A fork with different commands is an alternative reality, not
exact replay.

Each extension owns `validate_*_runtime`, snapshot/checkpoint restoration and
journal-replay helpers layered on public Canwu APIs, plus
`SimulationPlugin::validate_activation`. Tests reject forged but re-hashed
records, projections, index inputs and archive roots before live authority
changes.

`canwu-economy-reference` additionally owns an integration validator and
restore/replay wrapper. It validates resource-to-transport escrow and exact
dispositions, production-to-resource/technology acknowledgements,
force-to-resource consumption outcomes, and every typed scarcity/price source
witness.
It rejects cross-plugin forged but re-hashed candidates before live-state
replacement.

### 2.1 Exact-version continuation

Every runtime root schema contains
`PayloadRequiredEvidenceContinuationV1` under the canonical continuation field.
Dependencies are classified explicitly:

| Owner | Payload-required until | Completion / retained form |
| --- | --- | --- |
| resource | active reservation/allocation/transfer/consumption/loss/fulfillment has reached its terminal acknowledgement | mark continuation completed; retain exact version identity, semantic digest, operation key and terminal quantity in a hot receipt or archive object |
| production | execution reaches `Settled`, including output resource outcome acknowledgement | mark process, input-allocation, facility, technology and output-outcome dependencies completed; retain identity/digest in the production receipt/archive |
| force | resource fulfillment has produced the force consequence and every required externality outcome has been acknowledged | mark fulfillment and profile dependencies completed; retain identity/digest in the force receipt/archive |
| economy reference | population/cooperation/harvest externality application is terminal | mark force-intent dependency completed; retain application outcome identity/digest |

Restore and exact replay reject an active record when any payload-required body
cannot be resolved from live evidence or the kernel evidence archive. Adapter
packets may instead carry a denormalized typed result, but admission validates
its exact source version and semantic digest before the copied result becomes
authoritative; the source is identity-only only after that validation is
recorded. No active lifecycle relies on an old version body after its
continuation is marked completed.

### 2.2 Plugin-owned archive and knowledge bounds

`canwu-resource`, `canwu-production`, `canwu-force-supply-reference` and the
runtime part of `canwu-economy-reference` each implement the same package-owned
archive protocol, with package-specific types:

- `{Domain}ArchiveBlobV1`, content-addressed terminal payload batch;
- `{Domain}ArchiveMembershipV1` and bounded membership/temporal pages;
- `{Domain}ArchiveIndexDirectoryV1` and `{Domain}ArchiveHeadStateV1`;
- `{Domain}ArchiveRetentionHandleV1` with `Prepared`, `Verified`,
  `DurableIngress`, `Committed`, `RejectedStale` or `Abandoned` phase;
- `{Domain}ArchiveMaintenanceReceiptV1`, retained as a bounded hot receipt.

The flow is exact and follows the existing legal archive pattern:

1. `prepare_*_archive` scans at most the configured candidate budget, selects
   only terminal records with no unresolved payload-required continuation, and
   binds the current hot-state/source root.
2. `store_and_verify` writes content-addressed blobs, membership pages, temporal
   pages and directory objects through the package archive store, reads them
   back under provider budgets, and verifies bytes, roots and closure.
3. The package helper enqueues a plugin-owned internal ingress registered with
   `register_internal_ingress_with_archive_retention`; its opaque permit, target
   directory root and retention object cannot be authored or altered by a host.
4. The Phase 7 reducer rechecks the expected source root. It atomically advances
   the archive-head record, removes the archived hot records, and writes an
   applied or stable stale-rejection receipt. Pending retention objects remain
   reachable while the ingress is durable but unacknowledged.
5. `finalize_*_archive_retention` commits the verified provider handle and
   enqueues a package-owned acknowledgement. The next boundary clears the
   pending handle while preserving a bounded terminal receipt. Failed/stale
   handles become `RejectedStale` or `Abandoned` without changing the live head.
6. Each plugin registers a `PluginArchiveReachabilityParticipant` that adds its
   committed directory root plus every pending durable-retention object to the
   unified mark/sweep manifest. Activation and integration restore validate the
   head, provider objects, membership/temporal pages and pending handles before
   live authority changes.

Every accepted lifecycle owns a package-specific `CompletionCapacityLeaseV1`
before its first irreversible material or player cost. The lease reserves the
maximum remaining terminal receipts, mutations, acknowledgement packets,
mandatory final holder reports and hot bytes for that exact operation key.
Cross-plugin sagas use an explicit pre-activation acquisition protocol:

```text
Requested -> PartiallyGranted -> FullyGranted -> Preparing -> PreparedAll
          -> Activated
          -> Aborting -> Released
          -> Expired -> Released
```

The initiating owner writes `CompletionLeaseAcquisitionV1`: economy-reference
for G1b composition, resource for a local G2 operation, production for G3, and
force for G4. The acquisition fixes one `eligibility_time: SimTime` and one
`EligibilityEnvelopeV1`. The envelope digests the exact effective-dated content
pack, coverage keys, resource capabilities, demand expiries, protected floors,
route evidence, process/environment/technology qualifications, facility and
force-profile revisions, requisition policy and any required externality
applicability binding used by that operation. Each participant alone writes its own
`CompletionCapacityGrantV1`, binding the acquisition/operation key, exact run-
budget revision, target state versions, completion recipe/hash, reserved counts
and pre-activation expiry boundary. Repeated requests with the same key are
idempotent; a changed recipe, envelope or version is a conflict.

Acquisition, grant, prepare, certificate, first irreversible consumption and
pre-certificate abort/release run as a bounded zero-delay canonical-ingress
drain at that same `eligibility_time`. Boundary IDs and replay evidence advance,
but simulation time does not. Existing kernel semantics enforce the drain: a
caller asking `settle_boundary` for a later time while earlier ingress is
pending receives `InvalidBoundary` with no plugin mutation, while
`step_canonical`/`advance_canonical` settles the earlier ingress first. Every
certificate schedules its first-consumption ingress at zero delay, so no plugin
invented clock guard is required. The run manifest proves the maximum same-time
boundary count and reserves its work; request-token, pending and TTL limits make
the drain finite. Thus technical coordination cannot cross a harvest, route
closure, demand-expiry, force-cadence or production-validity cutoff.

An authorized holder may abandon only while the coordinator is `Requested`,
`PartiallyGranted`, `FullyGranted`, `Preparing` or `PreparedAll`, through a
package-owned `AbortCompletionLeaseV1` command at the current `SimTime`. It
enqueues zero-delay abort/release ingress, uses exact acquisition/version guards
and must fully drain every participant release before a later time can settle.
Once the activation certificate commits and schedules first-consumption ingress,
abort returns the stable `already_activated` outcome and cannot enter
`Aborting`. An accepted abort is idempotent, consumes no material/player cost
and cannot race or revoke an activated/consumed grant.

`Requested` becomes `PartiallyGranted` as exact grant versions arrive. A reject,
missing adapter, stale target, cancellation or timeout moves the coordinator to
`Aborting`; it sends idempotent release packets to every granting owner. Each
owner is the sole writer of `Released` and atomically refunds its lifecycle,
receipt, report and byte reservations to the cited `RunBudgetRevisionV1`.
Participants also expire unactivated grants from a bounded due index, so a
crashed coordinator cannot leak capacity. Pending acquisitions/grants are
bounded per authority and operation namespace.

After all grants are exact and current, the coordinator sends a prepare request
to every participant. Prepare handlers never return a transaction error for a
domain rejection: each owner records `Prepared` or a stable `Rejected` outcome.
`Prepared` installs an owner-local lock over the cited target versions and
completion capacity, so conflicting commands receive a stable
`lease_prepared_conflict` before mutation. A rejection leaves every other grant
non-activated; on the next boundary the coordinator enters `Aborting` and
releases all `Held`/`Prepared` grants. Each prepare handler also reconstructs
and revalidates its part of `EligibilityEnvelopeV1` at the exact acquisition
time. A stale or mismatched effective interval is a cost-free stable rejection,
not historical eligibility carried forward by a lock.

Every prepared outcome declares a half-open activation window. Expiry wins when
prepare/activation and expiry name the same boundary; therefore prepare is
accepted only when at least `ACTIVATION_GUARD_BOUNDARIES` remain, and the
coordinator must activate strictly before the earliest prepared deadline.
`Held` expiry and `Prepared` abort-deadline processing run first in each owner's
Phase 7 writer. These precedence rules are identical in every package.

Only after it observes exact `Prepared` versions from every participant does
the initiating owner write `CompletionLeaseActivationCertificateV1`, containing
the sorted prepared-grant versions, locked target versions, recipe digest and
activation boundary plus the exact eligibility time/envelope digest. That single
coordinator transition is `PreparedAll -> Activated`; no cross-plugin activation
packet or rollback barrier exists.
Because each participant's target is already locked, a valid certificate cannot
race a target mutation. On later boundaries, each irreversible participant
operation checks the exact certificate, its own `Prepared` grant, unchanged
eligibility envelope and the same `SimTime`, then consumes that grant atomically
with its first mutation. These are later boundary IDs in the same-time
administrative drain, not later simulation time. Semantic rejection is a durable
outcome that enters the reserved completion path, never a boundary error. A
single-owner G2 operation may prepare, certificate and consume its one grant
atomically with its first debit.

Before the certificate, held/prepared grants may expire or be released. After
the certificate their capacity cannot passively expire. A host attempt to skip
its zero-delay first-consumption ingress is rejected by the kernel as
`InvalidBoundary` and does not change the lease. `ActivationWindowMissed` is
reserved for a same-`SimTime` exact envelope/certificate mismatch discovered by
a participant; it records a stable cost-free rejection and explicitly releases
every unconsumed grant. It is not a holder abort path. Normal admissions cannot
consume the completion reserve.
Completion consumes each slot once and terminal archive/acknowledgement releases
the remainder. Restore and replay validate coordinator/grant closure in every
partial state, resume release/expiry deterministically, and reject forged
certificates, orphan grants or budget totals that do not reconcile.

For G4, `ExternalityApplicabilityBindingV1` is compiled from immutable,
effective-dated reference content and binds the exact `ForceSupplyProfileV1`,
`RequisitionPolicyV1`, place/date `CoverageKeyV1`, model-card semantic hash and
completion recipe. A command cannot choose applicability. When that binding
requires civilian or harvest externalities, a missing economy adapter rejects
acquisition and releases any resource/force grants before requisition. Only a
binding whose exact content classification is `externality_not_applicable` may
compile a recipe without the economy participant and produce an explicit
non-applied outcome. A caller-supplied or mismatched omission is rejected; there
is no silent fallback.

`RunBudgetRevisionV1` includes `CompletionCapacityPartitionV1`. It lists every
admitted authority/operation namespace, guaranteed completion units,
authority-reserved global pending slots, maximum burst units, request-token
capacity/refill duration, reacquire cooldown duration, per-`eligibility_time`
root-acquisition cap and the derived maximum wait in administrative boundaries
for a continuously eligible guaranteed request. The sum of reserved pending
slots must fit `MAX_PENDING_LEASE_ACQUISITIONS_GLOBAL`; a guaranteed request
uses only its authority's reserved slot, while burst requests use only the
explicitly unreserved global remainder. Another authority can therefore occupy
neither its completion guarantee nor its pending-admission guarantee.

Canonical lease cost is a fixed integer function of reserved receipts,
mutations, reports and bytes. Unreserved burst capacity is scheduled by
deterministic deficit round-robin ordered by policy class, round, authority ID,
admitted sequence and operation key. Release refunds capacity units but not
request-rate tokens. `AdmissionEpochV1` persists the distinct `SimTime`, token
balance, last refill time, next eligible time and root-acquisition count per
authority. Its epoch advances only when `SimTime` strictly increases; same-time
administrative boundaries do not refill tokens, satisfy cooldown or reset the
root-acquisition count. Unlisted authorities and requests exceeding their
partition fail before a grant. Restore/replay recompute and validate reserved
slot sums, shared remainder, epoch monotonicity and token/cooldown ledgers. Tests
hold capacity release constant and prove a churning authority cannot delay
another admitted, continuously eligible guaranteed request beyond the
manifest's derived fairness bound.

Every coordinator exposes a detached, holder-bound
`CompletionLeaseStatusDtoV1` that requires the initiating or explicitly granted
holder. It reports acquisition state, participant grant/prepared state, exact
versions, expiry/deadline, blocker/rejection, refunded units and next eligible
action. It is a bounded direct query and consumes no knowledge slot. The
original command outcome returns its acquisition ID; G1b shipment, production
admission and force admission reports link the DTO, including `Requested`,
`PartiallyGranted`, `Preparing`, `PreparedAll`, `Aborting`, `Released` and
`Expired`. A decision therefore never appears to do nothing while grants are
pending or unwinding.

Therefore archive or capacity backpressure only rejects **new** lifecycle
admission, before escrow debit, input consumption, force action or other player
cost. An already accepted transfer can always accept, return, lose or externally
outflow its escrow; an accepted production execution can always settle output;
an accepted requisition saga can always write its externality outcome and ACK.
If a hot terminal/receipt cap is reached and no verified archive batch can be
committed, new work receives stable `archive_backpressure`; existing work uses
its reserved completion capacity. No evidence is evicted and state does not grow
without bound.

Each plugin also owns a versioned `RunBudgetRevisionV1`; the full composition
records them in `EconomyRunManifestV1`. The manifest declares a finite admitted
run window, maximum new lifecycles, decision tickets and publications per
holder/schema, completion-report reserve, archive-maintenance reserve and exact
limit revisions. Scenario activation computes a worst-case proof from current
retained counts plus every permitted periodic and terminal publication. It
rejects a manifest whose declared run cannot fit. A revised manifest may extend
the window only before exhaustion and only with a fresh proof; after the window,
new report-producing lifecycles are unavailable while already accepted work and
its mandatory feedback finish from reserved capacity. G1b's proof covers all
fourteen months, daily/sub-daily logistics, choices, failures and final reports.

Capacity blockers are returned synchronously in command outcomes and detached
action-availability DTOs, so displaying the reason never depends on finding one
more knowledge slot. A decision ticket cannot offer an option whose required
lease set is unavailable; it refreshes with the blocked option and recoverable
archive/budget reason instead.

Knowledge is separately bounded because the current holder ledger computes
read-cut roots and current heads over retained history. Before publication each
runtime plugin calls `knowledge_record_count_in_namespace`, applies both the
namespace-total and per-boundary caps, and emits a stable capacity-rejection
outcome instead of publishing beyond them. Mandatory completion reports consume
their pre-reserved slots; periodic reports are admitted only when the active run
budget proves all remaining mandatory feedback still fits. `validate_*_runtime`
recounts the namespace and verifies every budget/lease proof during
activation/restore. Page size alone is never treated as a history bound, and a
capacity event is never the sole player-visible notification.

Definitions and opening state are installed by the reference scenario/content
materializer. Runtime account creation starts at zero. Runtime commands cannot
mint opening stock: every positive or negative amount is an exact authorized
production, external inflow, external outflow, consumption, loss, transfer or
reversal operation with immutable outcome evidence.

## 3. G1b: first playable grain loop

G1b is a runnable headless reference composition, not a kernel feature. It is
built against the first **experimental** `canwu-resource` implementation. G1b
proves that implementation in one complete grain loop; G2 is the later
promotion gate that accepts the same boundary as a shared extension only after
the G3 and G4 independent-consumer evidence exists.

### 3.1 Reference records

`canwu-economy-reference` owns period/profile-specific civilian, harvest,
relief, cooperation and externality-application records. The sole owner of
force schemas is `canwu-force-supply-reference`; the composition package
materializes its content inputs and consumes its outcomes but never registers a
second force writer. The relevant records are:

- `PopulationConsumptionProfile`: monthly food need and consequence policy;
- `SeasonalHarvestProfile`: effective-dated harvest window, seed floor,
  environmental inputs and production authorization;
- `ReliefPolicy`: granary authority, target holder and consequence policy;
- `ForceSupplyProfile` (force package): revisioned due cadences, buffers and
  readiness consequence policy;
- `RequisitionPolicy`: immediate supply authority, cooperation cost and
  next-season production cost;
- `EconomyReferenceReport`: holder, `observed_at`, `materialized_at`, confidence
  or interval, staleness, bottleneck, available player actions and exact
  demand/allocation/transport/acceptance/consumption evidence refs.

Historical names, goods, places, quantities and balance parameters live in
versioned reference profile data, not Rust branches. The first executable G1b
profile is explicitly `synthetic`; it must not be described as Ming agriculture
or as a calibrated historical economy.

### 3.2 Player actions

The reference integration supplies current decision options and canonical
commands for:

- open or close granary relief;
- classify or reclassify a protected seed/survival floor;
- reserve stock for civilian, relief or force demand;
- requisition stock;
- select one currently known route/dispatch option;
- accept, defer or cancel a shipment after an eligible decision point.

Decision policies receive holder-relative facts and existing option IDs only.
Selected options enter normal command ingress with derived authority.

### 3.3 Monthly/daily flow

```text
monthly demand formation
-> protected-floor-aware resource reservation/allocation
-> same-place consumption or balanced source debit
-> resource-ledger transfer escrow
-> next-boundary transport intent
-> transport booking and execution
-> transport disposition or ArrivalPending completion
-> next-boundary resource acceptance and destination credit
-> next-boundary fulfillment/consumption acknowledgement
-> consumer consequence on its next eligible boundary
-> population, cooperation and future-harvest consequences
-> holder-relative reports
```

Seasonal or exact-completion conditions accumulate at their authored cadence.
Harvest output is admitted only at the
effective harvest boundary and cites the exact profile, seed allocation,
environmental observation and authority evidence. Departure, reach,
`ArrivalPending`, delivery completion, acceptance, destination credit and
consumption remain distinct evidence states. Logistics may settle daily or
sub-daily; harvest completion is seasonal; authorization, adoption and
incidents are event-driven. A monthly boundary is an aggregation cadence, not
a universal economy clock.

`ResourceTransfer` owns the conserved escrow quantity from source debit until
destination credit, admitted loss, return credit or terminal external outflow.
Its states include `PendingDispatch`, `InTransit`, `ArrivalPending`,
`ReturnPending`, `Accepted`, `Lost`, `ExternalOutflowSettled`, `Cancelled` and
`Returned`. `Cancelled` is terminal only before source debit and therefore
always has zero escrow. Once source debit has occurred, cancellation requests
advance through `ReturnPending` to `Returned`, `Lost`, or the separately
authorized `ExternalOutflowSettled` successor; loss and external outflow are
never conflated and no quantity may remain stranded in `Cancelled`. Transport
execution is evidence and custody execution, not a second material balance.
Every resource/transport transition occurs in a later transaction with a
stable operation key, expected exact version and idempotent duplicate result.

### 3.4 G1b acceptance

- Run at least fourteen monthly boundaries and the required daily logistics
  boundaries without free material or double consumption.
- Demonstrate civilian, relief and force competition, partial fulfillment and
  explicit remainder.
- Demonstrate a seasonal harvest whose output changes when seed or conditions
  change.
- Compose real `canwu-transport` itinerary, booking, execution, handoff and
  arrival-completion types before destination credit.
- Demonstrate requisition's short-term force benefit and later local cost.
- Demonstrate seasonal route closure, reroute of the same delivery attempt and
  stale/delayed holder observations.
- Separately cover loss, return, reroute, cancellation and duplicate completion
  without changing the escrow quantity twice.
- Give a warehouse custodian an authorized stock report while a remote
  commander receives only delayed/interval arrival information; neither can
  read the other's undelivered facts.
- Fork before one concrete ticket: prioritize relief versus prioritize remote
  force supply. The branches must expose different population and readiness
  outcomes while each exactly replays.
- Prove save/load, fork, journal replay, actor-relative reports and atomic
  rollback of forced resource and transport failures.
- Reject forged but re-hashed account, route, disposition-partition, report and
  derived-index candidates during restoration.
- Archive a bounded terminal resource/economy batch through
  prepare/store/verify/internal-ingress/finalize/ack; restart at pending and
  committed phases, reject forged roots/provider objects, and prove hot-cap
  backpressure does not lose or duplicate material.
- Reach the namespace-total knowledge cap and prove later report publication is
  rejected in bounded work while prior read cuts remain valid.
- Reject scenario activation when the fourteen-month worst-case manifest does
  not fit. With archive storage unavailable, block a new shipment while an
  already-debited shipment uses its lease to reach accept/return/loss/outflow
  and publish its reserved final report.
- Fail one participant after a partial shipment grant, restart before release,
  and prove abort/expiry refunds every unactivated reservation before the same
  authority can admit a later valid shipment.
- Make one shipment participant record a stable prepare rejection. Prove the
  boundary commits that outcome instead of rolling back in a retry loop, the
  next boundary enters `Aborting`, and every held/prepared grant is released.
- Query the initiating holder's `CompletionLeaseStatusDtoV1` at `Requested`,
  `PartiallyGranted`, `Preparing`, `PreparedAll` and terminal release/expiry;
  an unauthorized holder receives no participant or blocker detail.
- Continuously churn one authority's distinct shipment keys while another
  authority holds an eligible guaranteed request; the latter activates within
  the manifest's declared deterministic fairness bound.
- Fill every unreserved global pending slot with burst requests from one
  authority and prove another authority can still consume its own reserved
  pending slot and guaranteed completion units. Run the maximum same-time
  administrative boundaries and prove tokens, cooldown and per-time root count
  do not reset; the next unique churn request is rejected.
- Start admission immediately before a harvest and seasonal route cutoff. Drain
  grant/prepare/certificate/debit at one `SimTime`, then advance time; prove the
  technical protocol neither gains the next harvest nor misses the closing
  route. A requested later boundary before debit receives kernel
  `InvalidBoundary` and leaves the lease unchanged; canonical drain then
  completes it. Separately, an authorized same-time abort accepted before the
  certificate fully releases before time may advance; an abort submitted after
  certificate commit returns `already_activated` and cannot race source debit.

## 4. G2: shared resource extension

G2 promotes the experimental `canwu-resource` lifecycle already exercised by
G1b into an accepted public extension boundary. Promotion occurs only after the
final G3 production plugin and the independently compiled G4 force-supply
reference plugin both consume the same API and own different downstream
consequences. Two historical profiles using one consumer do not satisfy this
gate.

### 4.1 Records

- `ResourceDefinitionRevision`
- `ResourceUnitRevision`
- `ResourceAccount`
- `ResourceDemand`
- `ResourceReservation`
- `ResourceAllocationLeg`
- `ResourceTransfer`
- `ResourceConsumption`
- `ResourceLoss`
- `ResourceFulfillment`
- `ResourceOperationOutcome`

Holder-facing resource reports are knowledge-schema records and detached query
DTOs, not ordinary authoritative domain records readable by a trusted client.

Accounts retain one authoritative amount and optional capacity. Available,
reserved and protected quantities are derived. In-transit material remains in
the custody owner selected by the reference integration; the resource
extension reads an exact typed disposition/acceptance result and never treats
transport reach as delivery.

Every account, demand, reservation, allocation leg, consumption, loss and
transfer cites the same exact resource and unit revisions. Conversion requires
an explicit revisioned conversion operation; implicit unit conversion is
forbidden. Demands include minimum useful amount, partial-fulfillment policy,
alternative/substitution group, due/expiry interval, explicit domain-provided
tie-break key and exact protected-floor policy revision.

Geographic resource capability remains reference/content state and is never a
single deposit flag. Reference profiles use effective-dated stages:

```text
potential -> observed/surveyed -> proven -> technically extractable
          -> operating site -> route-accessible -> delivered/accepted
```

Definitions cite exact location/scope, effective interval, unit, quality and
process suitability. Unknown evidence is `explicit_unknown`, not zero. Land
and agricultural profiles keep water/flood condition, season, labor, tools,
security and route access separate; they do not collapse them into one
fertility multiplier.

`canwu-economy-reference-content` expresses that ladder as a typed
`ResourceCapabilityRevision`. Its key is the exact resource revision, quality,
unit, place scope, effective interval and capability stage. Its payload binds
the evidence/model-card revision and, where relevant, the exact surveyed or
operating site, suitable process revisions and route-access evidence. A later
stage never implies an earlier stage at another place or date. Account opening,
site activation, process admission and route-access claims each cite the exact
capability revision and stage they require; an expired, geographically
non-containing, lower-stage or `explicit_unknown` cell cannot authorize the
operation.

### 4.2 Commands and systems

Commands register definitions/accounts, submit/amend/cancel demands, classify
protected floors, request balanced local operations, accept external adapter
results and acknowledge holder observations.

Demand lifecycle owners or the reference integration generate decision tickets
that change priority, protected floors or cancellation. `canwu-resource`
executes supplied policy evidence but never decides civilian, repair or force
priority. Capability-protected adapter results enter through internal ingress,
not ordinary player commands.

The plugin uses declared systems for admission reduction, deterministic
allocation, account/transfer settlement, fulfillment materialization,
invariant validation and knowledge publication. Allocation ordering is stable
by priority, due time, domain tie-break key, admitted sequence and demand ID.
Those keys are retained in allocation evidence and reports. Every operation
has a stable idempotency key and an immutable terminal outcome.

Dynamic demands do not use the kernel's registration-time
`reservation_reads`. One extension-owned Phase 7 lifecycle writer loads only
bounded due/dirty shards and writes exact `ResourceAllocationLeg` records. The
allocator revision and semantic hash are persisted; one leg may be consumed
once.

| Phase | Resource responsibility |
| --- | --- |
| 7 `DomainDeltaProposal` | sole lifecycle writer: reduce ingress, allocate and advance account/demand/reservation/transfer/consumption state |
| 8 `InvariantValidation` | read proposed state and verify conservation, exact versions, batch closure and escrow partitions |
| 12 `StrategicAggregation` | bounded shard and summary maintenance only |
| 13 `PerspectiveAndReportMaterialization` | holder-relative knowledge publication |

### 4.3 G2 acceptance

- Two independent consumer domains compete for one account without double use.
- Local consumption, balanced transfer, loss, cancellation, reversal and
  external inflow/outflow preserve the conservation equation.
- One demand accepts partial fulfillment; another below its minimum useful
  amount receives a stable rejection/remainder rather than a useless fragment.
- Partial fulfillment, protected floors and expiry remain explicit.
- Invalid exact-version adapter results, duplicate allocation legs, forged
  destination acceptance and over-capacity accounts fail before commit.
- Bounded queries and knowledge reports never expose accounts outside the
  holder's grants.
- A Ming-period repair/workshop profile and a later coal/iron chain use the
  same resource contract while retaining separate effective dates, qualities,
  organization and route evidence.
- The same resource is tested in two places at one date and in one place at two
  dates. Only the exact place/date/stage binding authorizes an account, site,
  process or route claim; an `explicit_unknown` cell remains non-behavioral.
- Resource reports carry holder, observed/materialized time, confidence/range,
  staleness, exact account/demand/allocation/fulfillment refs, sorting evidence
  and explicit remainder/rejection reason.
- Increasing archived terminal operations does not increase ordinary Phase 7
  work for an unchanged active shard.
- Compact old kernel evidence while an active transfer still requires its
  payload, restore through the provider-backed evidence archive, then complete
  the continuation and prove the terminal resource archive retains only the
  exact identity/digest/quantity receipt.
- Exhaust resource knowledge and hot-receipt caps and receive stable capacity
  or archive-backpressure outcomes without changing conservation.
- Prove every accepted resource lifecycle holds completion capacity before its
  first debit; cap pressure blocks only the next admission and never a terminal
  conservation transition.
- Through the public resource API, grant then cancel/expire an unactivated
  downstream-consumer lease and prove exact idempotent budget refund; a changed
  recipe under the same operation key is rejected.
- Through the same public API, expose holder-bound acquisition status and stable
  cancel/release receipts without consuming a knowledge-publication slot.
- Saturate one authority's pending and request-token limits and prove a second
  authority's guaranteed resource request is neither admitted into the first
  partition nor starved beyond its declared fairness bound.
- Restore/replay with a full shared-burst pool and nonzero cooldown ledger;
  validate reserved-slot sums and prove same-`SimTime` boundaries cannot refill
  tokens, shorten cooldown or reset the root-acquisition cap.
- At prepare, certificate and first debit, revalidate the exact place/date/stage
  capability, route evidence, demand expiry and protected-floor revision from
  `EligibilityEnvelopeV1`; pending canonical ingress prevents crossing an
  effective interval without consumption or a fully drained pre-certificate
  authorized abort. A same-time envelope mismatch releases the lease and cannot
  manufacture eligibility or ineligibility.

## 5. G3: production, facilities and projects

`canwu-production` owns production-asset lifecycles only. Roads, canals, dikes,
fortifications, institutions and universal project semantics remain outside
the extension; the reference integration may map repair cases into production
assets only where they directly provide production, site-local storage or
site-local loading capacity.

### 5.1 Records

- `ProcessRevision`
- `ProductionSite`
- `FacilityAsset`
- `ProductionCapacityAllocation`
- `WorkOrder`
- `ProductionExecution`
- `WorkInProgress`
- `FacilityProject`
- `ProductionOperationOutcome`

Holder-facing production reports are knowledge-schema records and detached
query DTOs. Constraint blockers are structured by group, requirement,
required evidence, available evidence, shortage and next eligible action.

Requirements are typed groups for material, labor/capability, facility,
tools/machines, energy, technology implementation, authorization,
environment/season, security and access. Alternative groups are data, not
engine branches.

The name `ProductionExecution` avoids collision with
`canwu-technology::ProductionRun`, which remains technical-use evidence.

`ProductionSite` supports household, supplementary household work,
distributed workshop, government workshop, concentrated plant and multi-site
enterprise forms through data. None implies a building level. Process-specific
technology evidence groups cite the exact existing record types:

```text
TechniqueRevision
-> CapabilityQualification and/or ImplementationRecord
-> AdoptionRecord only when the exact process/profile requires adopter commitment
```

Qualification, implementation and use-specific adoption remain distinct.
Technique knowledge or a calendar year never grants capability by itself, but
adoption is not a universal production prerequisite: customary household work,
contract production, or an already installed process may cite the exact
qualification/implementation evidence group without a separate adopter record
when its revisioned profile says so.

### 5.2 Lifecycle

```text
work order: Proposed -> Authorized -> Reserving -> Ready -> Running
          -> CompletedPendingOutputSettlement -> Settled
          | Cancelled | Failed

facility: Planned -> Authorized -> Reserving -> InProgress -> Commissioning
       -> Operational -> Degraded/Damaged -> Repairing -> Retired
```

Inputs and capacity slots require exact accepted allocations. Outputs appear
only after a later resource-output settlement. Long runs retain explicit work
in progress. Cancellation
releases recoverable inputs once and records non-recoverable waste once.
Facility condition changes usable capacity before a later repair restores it.
Configured discrete failure risk uses operation-keyed random draws with draw
evidence. Exact replay consumes the recorded draw; an alternative-reality fork
may create a different operation and draw lineage.

Input material must already have been consumed or placed by a prior transaction
in a `canwu-resource`-owned account or transfer escrow assigned to the
production custodian before execution starts. `WorkInProgress` cites that exact
resource evidence but never owns a second physical balance. Completion follows:

```text
Running -> CompletedPendingOutputSettlement
-> next-boundary resource production ingress
-> resource credit/outcome
-> next-boundary production acknowledgement -> Settled
```

The Phase 7 lifecycle writer forms structured terminal rejections for invalid
work candidates and omits their mutation; it does not rely on a nonexistent
kernel-local proposal rejection. External labor, skill, energy, facility-slot,
environment, security and access evidence cites exact provider versions.
Transport-network capacity remains transport-owned. Phase 8 validates proposed
WIP/asset/execution closure, Phase 10 stages configured incident/damage
candidates for Phase 11 atomic conditional commit, Phase 12 maintains bounded
aggregates and Phase 13 publishes holder knowledge.

`ProductionCapacityAllocation` is the authoritative production-owned slot
reservation; the kernel reservation API is not used for dynamic work orders.
It binds an exact facility revision and generation, capability class,
half-open use interval, positive integer capacity, work-order/execution ID,
stable operation key and one of `Reserved`, `Consumed`, `Released` or
`Expired`. Phase 7 is its sole writer. Phase 8 sorts allocations by facility,
capability and interval and rejects any overlap whose summed capacity exceeds
the facility's usable integer capacity. `Reserved` becomes `Consumed` exactly
once when the execution starts; cancellation, failure or expiry releases it
exactly once. A consumed slot remains attached to WIP until completion or
failure and cannot be reassigned retroactively.

Phase 10 `HistoricalCandidateEvaluation` only evaluates operation-keyed random
draws and stages a complete damage/incident transition bundle. It does not
mutate production state. The kernel's Phase 11
`ConditionalTransitionCommit` atomically commits or rejects that bundle; any
ordinary Phase 10 production write is invalid. Pending executions retain
payload-required resource allocation, process, facility and technology bodies
through `CompletedPendingOutputSettlement`; only the final resource outcome
acknowledgement and transition to `Settled` complete those continuations.

`FacilityProject` creates or changes site-local production and handling assets
only. It cannot create route edges, vehicles, bookings or network capacity;
those remain exclusively transport-owned.

Authorities use commands and decision tickets to create/authorize/cancel work
orders, select a currently legal process revision, approve repair, and accept
commissioning. A required playable ticket offers: continue with a degraded
facility, stop for repair, or defer the order.

### 5.3 G3 acceptance

- Cross-validate a low-capital household/workshop process and a machine/fuel-
  dependent industrial process with the same schemas and different data.
- Missing material, skill, facility, energy, technology implementation,
  maintenance or route access blocks only the affected run and records why.
- Concurrent work cannot double-book resource allocations or capacity slots.
- A partially completed asset grants no capacity unless its exact revision
  defines an operational stage.
- Save/load and replay reproduce WIP, damage, waste, output and causal reports.
- A distributed workshop profile must not be represented as one anachronistic
  factory, and a coal/iron plant must idle when fuel quality, transport,
  maintenance, skilled personnel, finance/organization evidence or another
  required constraint is absent.
- Forking the degraded-facility ticket produces either earlier output with
  further degradation, current downtime with later restored capacity, or an
  explicit deferred order. Each branch preserves structured blocker reports.
- An authorized operator and a remote owner receive different order/WIP/fault
  knowledge cuts and cannot inspect one another's undelivered facts.
- Reject overlapping `ProductionCapacityAllocation`s in Phase 8, prove a slot
  is consumed/released once, and prove Phase 10 stages but cannot directly write
  damage while Phase 11 atomically commits the bundle.
- Restore an execution with archived resource/technology evidence while its
  continuation is active, settle output, complete the continuation, archive the
  terminal execution, and hit the bounded production-knowledge cap without
  unbounded report work.
- Refuse `Ready -> Running` when the production and resource output-completion
  leases are unavailable; once running, complete settlement and mandatory final
  feedback even while new work is under backpressure.
- Exercise production-granted/resource-rejected and resource-granted/production-
  rejected acquisition orders, restart in `PartiallyGranted`, then prove stable
  prepare outcomes, exact target locks, one coordinator activation certificate
  or complete release with no stranded budget and no rollback retry loop.
- Show each intermediate acquisition state through the authorized holder-bound
  status DTO, and prove production churn cannot consume the resource owner's
  guaranteed completion partition or starve another eligible authority.
- Admit immediately before a seasonal/process-validity cutoff and prove the
  same-time administrative drain revalidates process, environment, technology,
  facility and route evidence before `Ready -> Running`. A later-time request is
  rejected until canonical drain or a pre-certificate authorized abort
  completes; a stale same-time envelope aborts without input consumption,
  capacity use or other player cost. Post-certificate abort returns
  `already_activated` before any coordinator mutation.

## 6. G4: military logistics composition

G4 does not create a universal force model.
`canwu-force-supply-reference` is the sole schema writer for `ReferenceForce`,
`ForceSupplyProfile`, `ForceLogisticsState`, `ForceConsumptionIntent`,
`ForceConsequenceRecord`, `ForceExternalityIntent` and the requisition saga.
`canwu-economy-reference` only materializes content inputs and consumes typed
force externality intents. Doctrine, ranks, equipment catalogs, battle formulas
and historical institutions remain reference data or host rules.

The force domain submits recurring resource demands and never directly debits
an account. It submits a typed resource-consumption intent;
`canwu-resource` performs the debit and publishes an exact fulfillment outcome
on a later transaction. The force package alone writes force-local readiness,
fatigue, cohesion, disease, desertion and supply-posture consequences after
that outcome is eligible. It cannot write population, cooperation, harvest,
property or occupation state.

Requisition is a cross-plugin saga owned by the force package:

```text
PendingResourceConsumption
-> resource fulfillment body available
-> ForceConsequenceCommitted
-> ForceExternalityPending
-> economy-reference applies or rejects at expected target revision
-> ExternalityApplied | ExternalityRejected
-> next-boundary force acknowledgement -> Settled
```

`ForceExternalityIntent` carries a stable operation key, exact force-
consequence and resource-outcome versions, expected economy target revision,
typed effect request and quantity; it is an intent, never a mutation. The
economy package is the sole owner of `PopulationState`, `CooperationState`,
`HarvestInputModifier` and `EconomyExternalityOutcome`. It applies an admitted
intent once on its next boundary, records an idempotent terminal outcome, and
returns that exact outcome to the force package. Until the final force
acknowledgement, the resource fulfillment and force consequence bodies remain
payload-required. Afterwards their exact identities/digests remain in the
settled receipt and the dependency continuation is completed. When the economy
adapter is absent, the force package records an explicit non-applied terminal
outcome rather than mutating external state or waiting forever.

Force profiles retain separate grain/food, fodder, physical currency,
ammunition, spares, fuel and other authored resource requirements. They may not collapse
them into a universal supply score. Consumption cadence, buffers, tolerance and
consequence functions belong to the exact force-profile revision and are
classified by provenance; the engine does not prescribe military priority or
a generic shortage-to-readiness percentage.

Pay obligations, arrears and credit remain fiscal/claim-domain state; physical
currency may be a resource but is not itself an obligation result.

A required ticket offers: wait for supply, advance immediately, or requisition
locally. The options trade time, readiness and civilian cooperation through
validated force/reference commands.

### 6.1 G4 acceptance

- Reserved, booked, departed, delayed, reachable, intercepted, lost or
  `ArrivalPending` supply grants no readiness benefit.
- Accepted destination stock can change readiness exactly once only after the
  next boundary at which that exact requirement is due under its cited profile.
  A daily force tick processes daily-due requirements; sub-daily, event-driven
  and longer-period accumulators retain their authored cadence and remainder.
- Requisition may improve current consumption while worsening cooperation and
  a later production input.
- Missed food, ammunition, fuel, maintenance material or fodder produce
  separately attributable consequences. Physical-currency delivery has no
  wage, arrears or credit consequence without an exact fiscal/claim-domain
  adapter outcome.
- Active forces evaluate due requirements on their authored cadence. Background
  aggregation is allowed only for groups with equal state class, profile/rule
  revision, supply-input class, due cadence and persisted integer remainder;
  nonlinear disease, desertion or threshold rules block unsafe aggregation.
- A preindustrial force and a materially different high-throughput force reuse
  fulfillment/transport contracts while retaining distinct resources,
  cadence, organization and consequence policies.
- Forking the wait/advance/requisition ticket produces distinct time,
  readiness and civilian-cooperation outcomes with shared exact supply causes.
- Force reports publish known stock intervals, demand forecast, arrival state,
  observed/materialized time, source, confidence and shortage attribution.
  Enemy and unauthorized friendly commanders cannot decode authoritative
  resource or force records.
- Authorized force reports expose the exact requisition saga stage:
  `PendingResourceConsumption`, `ForceConsequenceCommitted`,
  `ExternalityPending`, `ExternalityApplied`, `ExternalityRejected` or
  `Settled`, plus the latest exact outcome/ACK reference and any recoverable
  blocker. They explain when force benefit is committed while civilian effects
  are still pending or rejected.
- Background reports preserve which shortages caused which consequences rather
  than collapsing them into one unexplained aggregate modifier.
- Requisition produces one force-local consequence, one exact externality
  intent, one economy-owned application/rejection outcome and one force ACK;
  duplicates and stale target revisions are idempotent terminal outcomes.
- Restore at every requisition-saga phase with fulfillment payload retention,
  then archive settled force/economy receipts and prove knowledge/archive caps
  fail with bounded work rather than leaking or directly mutating another
  domain.
- Require exact resource, force and economy completion grants before the
  irreversible requisition step; once admitted, finish externality and ACK from
  reserved capacity while later force actions are blocked.
- Reject a content-compiled required-externality recipe when the economy
  adapter is absent and release earlier grants; separately prove an exact
  `externality_not_applicable` applicability binding compiles no economy grant
  and cannot create a civilian effect.
- Expose force/resource/economy acquisition and prepare state through the
  authorized holder-bound status DTO. A semantic rejection commits as data,
  unwinds on the reserved path and never becomes a repeated boundary rollback.
- Churn rejected force acquisitions under one authority and prove another
  eligible authority's guaranteed requisition request activates within the
  manifest's declared fairness bound.
- Admit immediately before a force-supply cadence or requisition-opportunity
  cutoff and prove all preactivation work plus the irreversible requisition
  occurs at one `SimTime`. Kernel `InvalidBoundary` prevents technical waiting
  from changing the due requirement, military opportunity or civilian
  conditions; pre-certificate authorized abandonment drains release before time
  advances, while post-certificate abort is a stable `already_activated` no-op.
- Compile externality applicability from the exact force profile, requisition
  policy, place/date coverage key and model card. Reject caller-selected or
  mismatched `externality_not_applicable`, and test a content revision whose
  applicability changes across a place/date boundary.

## 7. G5: explainable local price pressure

G5 delivers a read-only `LocalScarcityProjection` and an evidence-qualified
`PricePressureProjection` in the reference integration. They are decision and
reporting inputs, not authoritative price formation, and they do not move
resources, create money or settle a trade. This completes the current G5 gate
without claiming a full market simulation.

### 7.1 Bounded typed observation witnesses

- `TypedObservationWitnessV1`
- `LocalScarcityProjection`
- `PricePressureProjection`
- `PricePressureFactor`

G5 projections are detached, holder-bound query DTOs. They are neither domain
records nor knowledge records, have no persisted supersession lifecycle, and do
not add a record to the holder ledger. Save/load and replay requirements mean
that the same restored snapshot and query must deterministically reproduce the
same DTO and digest.

G5 does not call generic `knowledge_records`, construct a generic
`KnowledgeReadCut`, or claim authenticated historical-cut replay. Each approved
source package instead exposes a bounded typed adapter over its package-owned
holder observation head:

- `canwu-resource`: stock interval, demand, allocation and fulfillment witness;
- `canwu-production`: WIP, capacity, blocker and expected-output witness;
- `canwu-force-supply-reference`: supply, arrival, shortage and saga witness;
- `canwu-economy-reference`: relief, buffer, policy, route/security observation
  and price-bearing evidence witness.

`TypedObservationWitnessV1` contains provider plugin name/version/semantic
hash, exact provider state/head version, holder, scope, observed/materialized
time, confidence/interval, bounded canonical facts, exact source evidence refs,
adapter revision and canonical digest. The provider query validates the holder's
grant and uses a holder/scope due-dirty index with package caps; it never scans
generic holder history or falls back to omniscient state. The economy query
accepts only registered adapter revisions, sorts witnesses canonically and
binds their exact versions/digests into the projection input digest. A decision
ticket that uses G5 copies the holder, scope, projection digest and witness refs
into its immutable decision evidence.

The scarcity projection explains bounded holder-observed supply, demand,
buffers, route access, security and policy without claiming a price. A price-
pressure projection is materialized only when the exact scope and interval has
price-bearing exchange, executed-price, quote, administered-price or contract-
price evidence plus a revisioned interpretation rule. Each factor carries exact
place/scope, effective window, resource/quality/unit revision, observation
time, materialization time, confidence or interval, and source evidence. The
projection distinguishes an observed quote, executed price, administered
price, contract price and inferred pressure when such evidence exists; it
never silently substitutes one for another. Without qualifying price-bearing
evidence, price is `not_applicable` or `explicit_unknown`; scarcity may still be
reported. Unknown or stale inputs remain visible. There is no national price,
universal elasticity or automatic equilibrium.

One projection reads at most the configured adapter count, observation facts
and adapter calls. If any required provider exceeds its budget, lacks a grant or
returns an invalid witness, the detached query returns a typed, player-visible
`ProjectionUnavailable` result with the exact blocker. It never publishes an
empty record and never consults generic or omniscient state.

### 7.2 G5 acceptance

- A local route disruption plus demand increase raises explainable scarcity
  only in affected scopes, and raises price pressure only in a scope with
  qualifying price-bearing evidence and an applicable interpretation rule.
- Decision options to release reserves, keep a buffer, ration or dispatch a
  remote transfer create different immediate and later resilience outcomes.
- Distant abundance without a workable route does not reduce local pressure.
- Allocation, rationing, requisition, self-provision and exchange are distinct
  causes and never imply one another.
- Cross-profile tests include one exchange/quote regime that can emit price
  pressure and one administered, rationed or self-provision regime that emits
  scarcity while price remains `not_applicable` or `explicit_unknown`.
- Actors receive delayed/partial typed observation heads and may form different
  pressure DTOs from different epistemic cuts.
- Save/load, fork and exact replay deterministically re-query the same typed
  witnesses and reproduce factors and projection digests.
- Reject a witness/projection with a substituted holder, provider state version,
  adapter revision, source evidence or digest; unauthorized holders receive no
  witness.
- Exhaust adapter/fact/call budgets and return a stable player-visible
  `ProjectionUnavailable` DTO without touching generic knowledge history or
  omniscient domain state.
- Fill every economy knowledge-publication slot and prove the detached G5 query
  still returns a current typed projection or an explicit adapter blocker; it
  never depends on another knowledge slot.

An authoritative `canwu-market` extension with offers, bids, clearing,
contracts, credit and expectations is explicitly deferred. It requires a new
product need, separate payment/claim ownership and another four-reviewer gate.

## 8. Historical and gameplay reference profiles

Reference profiles are source-linked calibration inputs, not claims that one
source determines all parameters. Initial profiles cover:

- a preindustrial seasonal grain/granary/route case;
- a household or workshop production case;
- a late-nineteenth-century coal/iron/machine-maintenance case;
- a preindustrial force supply case and a materially different high-throughput
  logistics profile;
- a local shortage-pressure case under allocation, rationing, requisition,
  self-provision and exchange observations.

Every numerical field or causal rule carries a model card with:

- `classification`: `synthetic`, `archetype`, `source_calibrated`, `disputed`
  or `unknown`;
- citation URL and page/paragraph locator where available;
- claim scope, forbidden inferences and competing interpretations;
- geographic scope and effective start/end;
- exact resource, unit, quality, process and rules revisions;
- extraction/conversion derivation, uncertainty interval and confidence;
- calibration status and semantic hash.

Coverage follows the fail-closed Ming fiscal reference-content pattern. A
`CoverageKeyV1` is the exact period, region, mechanism, resource revision,
quality/unit revision and process/organization class. Every cell has a numeric
priority and one of `supported`, `archetype_fallback`, `explicit_unknown` or
`not_applicable`. The content compiler rejects equal-priority overlaps, an
uncovered required key, behavior in unknown/not-applicable cells, a supported
or archetype cell without provenance, and any numerical field or causal rule
without an exact model-card ID. Narrower geography or time does not silently
win; priority is explicit and its resolution evidence is materialized.

`supported` and `archetype_fallback` cells may materialize only the fields and
rules bound to their model cards. `explicit_unknown` and `not_applicable` are
non-behavioral. A historically named profile carries a top-level disclosure of
every synthetic/archetype/disputed/unknown field and cannot claim calibrated
status while any undisclosed synthetic causal rule remains. A field cannot
become a historical fixture until its model card and coverage binding are
complete; otherwise it remains explicitly synthetic. Victoria 3 may inform
player-facing comparison but is never historical evidence.

Tests compile an exhaustive required period × region × mechanism matrix, reject
missing cards and equal-priority overlaps, prove that local evidence cannot be
overridden by a broad archetype, and prove that unknown/not-applicable cells
cannot authorize accounts, processes, force consequences or price pressure.

Reference content and runtime integration are separate packages, following the
existing `canwu-fiscal` -> `canwu-ming-fiscal` ->
`canwu-ming-fiscal-reference` pattern. The content package contains no solver,
command handler or privileged mutation path.

## 9. Performance and persistence gates

Each extension records canonical limits and rejects over-budget work with a
stable terminal outcome. The benchmark matrix varies active accounts, demands,
transfers, work orders, facilities, holders, due/dirty fractions and journal
length. All V1 revisions share `MAX_SHARDS = 64`, `DEFAULT_SHARDS = 8`,
`MAX_HOLDERS = 1_024`, `MAX_QUERY_PAGE = 256`,
`MAX_REPORT_RECIPIENTS_PER_BOUNDARY = 256`,
`MAX_HOT_RECEIPTS = 8_192`, `HOT_RECEIPT_RETENTION_BOUNDARIES = 4_096`,
`MAX_HOT_RECEIPT_BYTES = 8_192`, `MAX_HOT_RECORD_BYTES = 65_536`, and
`MAX_ARCHIVE_BATCH = 512`. Archive work is additionally bounded by
`MAX_ARCHIVE_PREPARE_CANDIDATES = 2_048`,
`MAX_ARCHIVE_OBJECTS_PER_BATCH = 1_024`,
`MAX_ARCHIVE_BYTES_PER_BATCH = 16 MiB`,
`MAX_ARCHIVE_PROVIDER_READS_PER_BATCH = 2_048`,
`MAX_ARCHIVE_PROVIDER_WRITES_PER_BATCH = 2_048`,
`MAX_ARCHIVE_PAGE_ENTRIES = 512`, and
`MAX_PENDING_RETENTION_HANDLES = 64`. A single lifecycle lease may reserve at
most `MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE = 16`,
`MAX_COMPLETION_MUTATIONS_PER_LIFECYCLE = 32`,
`MAX_COMPLETION_REPORTS_PER_HOLDER = 8`, and
`MAX_COMPLETION_BYTES_PER_LIFECYCLE = 256 KiB`. Lease acquisition is bounded by
`MAX_GRANTS_PER_ACQUISITION = 8`,
`MAX_PENDING_LEASE_ACQUISITIONS_PER_AUTHORITY = 16`,
`MAX_PENDING_LEASE_ACQUISITIONS_GLOBAL = 1_024`,
`MAX_RESERVED_PENDING_SLOTS_PER_AUTHORITY = 16`,
`MAX_REQUEST_TOKENS_PER_AUTHORITY = 16`,
`REQUEST_TOKEN_REFILL_INTERVAL_MINUTES = 1`,
`MIN_REACQUIRE_COOLDOWN_MINUTES = 1`,
`MAX_ROOT_ACQUISITIONS_PER_AUTHORITY_PER_SIM_TIME = 16`,
`PREACTIVATION_LEASE_TTL_BOUNDARIES = 8`, and
`ACTIVATION_GUARD_BOUNDARIES = 2`. Scenario partitions may be stricter but may
not exceed those hard maxima, reduce the activation guard or reserve more
pending slots than the global cap. Admission rejects a lifecycle whose declared
completion recipe, authority-reserved pending count, shared-burst pending count,
request rate, cooldown or per-time root-acquisition count exceeds them. Token
refill and cooldown use persisted simulation minutes, never boundary count. The
package-specific limits are:

| Limit revision | Default active cap | Hard active cap | Boundary/catalog cap | Authoritative-state hard cap |
| --- | --- | --- | --- | --- |
| `ResourceLimitsV1` | 1,024 accounts/demands/transfers per shard | 4,096 accounts/demands/transfers per shard | 1,024 definitions, 1,024 unit revisions, 2,048 allocation candidates, 4,096 mutations, 8,192 namespace-total knowledge records and 1,024 per holder | 256 MiB |
| `ProductionLimitsV1` | 1,024 facilities/work orders/executions/capacity allocations per shard | 4,096 facilities/work orders/executions/capacity allocations per shard | 2,048 process revisions/sites, 1,024 lifecycle candidates, 4,096 mutations, 512 incidents, 8,192 namespace-total knowledge records and 1,024 per holder | 256 MiB |
| `EconomyReferenceContentLimitsV1` | 4,096 coverage cells and model cards | 16,384 coverage cells and model cards | 4,096 profiles, 32 citations per model card, 8,192 bytes per citation locator and 16 MiB compiled-pack bytes | 32 MiB |
| `ForceSupplyReferenceLimitsV1` | 256 forces and 1,024 active demands/sagas per shard | 1,024 forces and 4,096 active demands/sagas per shard | 2,048 consumption intents, 2,048 consequence/externality outcomes, 4,096 namespace-total knowledge records and 512 per holder | 64 MiB |
| `EconomyReferenceLimitsV1` | 512 civilian/externality/scarcity scopes | 2,048 civilian/externality/scarcity scopes | 32 factors per projection, 8 typed source adapters, 256 observation facts, 16 adapter calls, 256 tickets/reports, 8,192 namespace-total knowledge records and 1,024 per holder | 128 MiB |

The authoritative-state cap includes every live canonical shard and hot
receipt, not archive bytes. Hot receipts exceeding the retention window are
archived before more are admitted; adding shards or holders cannot evade the
global byte, fan-out or receipt caps. Configuration may choose lower values but
never exceed the active revision. Limit rejection is deterministic, idempotent
and preserves an explicit remainder. Benchmarks measure transaction time,
allocations, snapshot bytes, load/index rebuild, bounded query cost and exact
replay. Derived indexes are excluded from authoritative commitments and rebuilt
deterministically.

The initial interactive target is a bounded reference fixture, not a whole-
game hardware guarantee. No package may claim scalability without recorded
benchmark evidence for its dominant dimensions.

Release-mode evidence records the source fingerprint, exact workload, P50/P95,
allocation traffic, snapshot bytes, load/index rebuild, bounded query cost and
replay throughput. Acceptance requires cold terminal history not to enter the
ordinary transaction curve, unchanged shards not to be rewritten, stable
read-cut-bound pagination and identical logical results on Windows, macOS and
Linux.

## 10. Documentation and release surfaces

The delivery updates the architecture and end-state crate graphs, crate map,
terminology, public API documentation, READMEs, examples, versioning notes,
canonical proposal and matching Chinese/English website pages. Public terms
added here must appear in all three terminology surfaces.

## 11. Review gates

Every G requires independent gameplay, senior-engine, historical and lead-
director review before implementation and again after implementation. A gate
passes only when no blocking contradiction remains. Findings are recorded as
accepted, rejected with a technical reason, or deferred with an explicit
boundary. The lead-director review resolves trade-offs but cannot waive
authority, conservation, actor-relative visibility, persistence, replay or
cross-platform requirements.
