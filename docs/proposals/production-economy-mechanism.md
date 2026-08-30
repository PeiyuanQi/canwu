# Production, Resource, and Facility Mechanism Proposal

> Status: iteration-4 validated by the CM 0.6 public-API host fixture; proposal only; no Canwu runtime or crate implementation approved
> Date: 2026-08-30
> Scope: the reusable boundary between Canwu, production/resource extensions,
> transport, technology, fiscal procedure, and host-owned military rules

## 1. Decision

Canwu should support production, consumption, construction, and military supply
through optional domain extensions above the public API. It should not add
grain, buildings, factories, armies, historical resource maps, price formulas,
or era-specific production rules to `canwu-core`, `canwu-sim`, or `canwu-api`.

The smallest credible architecture is:

1. prove one conserved resource-and-fulfillment slice in a host integration;
2. first consolidate a host-owned shared layer; extract an experimental
   `canwu-resource` extension only when two independent lifecycle-owning domains
   or two independent host integrations prove the same account, reservation,
   transfer, transport-projection, and fulfillment lifecycle;
3. build an experimental `canwu-production` extension above that lifecycle;
4. keep markets optional and keep military institutions in host plugins until
   a second, materially different war domain proves a reusable force lifecycle.

This proposal names candidate crates for clarity. It does not reserve those
package names or claim that they are part of Canwu 0.9.

The first host proof now exists in Celestial Mandate: five host-owned plugins
register the bounded resource, cargo, population, military, and invariant
record sets against published `canwu-api = "=0.6.0"`. Its synthetic fixture
proves deterministic partial allocation, same-place consumption, route-bound
cargo custody, terminal loss, destination acceptance, next-boundary military
consumption, atomic rollback, exact snapshot validation, fork/replay, and
holder-relative reports without a Canwu core change. That result validates the
boundary. Its cargo record uses a fixed host-owned typed itinerary and does not
yet instantiate Canwu transport execution types; that composition remains a
playable-integration gate. The proof does not yet justify extracting
`canwu-resource` or `canwu-production`.

## 2. Existing capability and the real gap

Canwu already supplies the difficult runtime properties needed by an economy:

- deterministic time, cadence, ordering, and keyed randomness;
- validated commands, canonical ingress, authority, and decision tickets;
- typed domain records and plugin-owned components;
- fourteen-phase atomic settlement, rollback, evidence, commitments, save,
  fork, and exact replay;
- deterministic reservation offers, requests, and allocations;
- actor-relative knowledge and holder-relative information;
- observer-relative routing and an explicit transport execution lifecycle;
- technology qualification, implementation, adoption, and production evidence;
- fiscal procedure with exact external execution receipts.

Those facilities are necessary but are not a production economy. The missing
authoritative capabilities are:

- persistent resource accounts and conserved balances;
- dynamic demands and reservations lasting beyond one settlement boundary;
- balanced debit, in-transit custody, loss, arrival, and destination credit;
- recurring and intentional consumption;
- process revisions, work orders, production runs, quality, waste, and failure;
- facility construction, repair, maintenance, damage, and retirement;
- a domain-owned price-pressure or market mechanism where a product needs one;
- military supply and readiness rules owned by a force domain.

The existing `canwu-technology::ProductionRun` is technical evidence. It is not
an inventory, work-order, wage, price, transport, or general production model.
The existing reservation primitive is a deterministic allocation calculation.
It is not a persistent resource ledger.

## 3. Ownership boundary

| Owner | Owns | Must not own |
| --- | --- | --- |
| Canwu simulation core | atomic settlement, declared reads/writes, deterministic allocation, visibility, commands, evidence, save/replay | goods, recipes, prices, buildings, armies, historical geography |
| `canwu-resource` candidate | conserved account/demand/reservation/transfer subset, allocation coordination when richer than the core scalar primitive, fulfillment results, typed transport projections | host cargo movement/disposition lifecycle, arrears/debt, route search, population rules, technology qualification, fiscal law, historical goods |
| `canwu-production` candidate | process revisions, production sites/assets, work orders, WIP, production runs, input/output settlement, maintenance and production-asset project lifecycle | universal projects/construction, technology tree, markets, map truth, labor demography, military doctrine |
| `canwu-routing` | observer-relative route estimates | delivery, cargo balance, stock ownership |
| `canwu-transport` | MovementOrder, TransportExecution, CapacityBooking, itinerary legs, handoffs, delivery-completion bridge | cargo balance, inventory, production, demand priority, destination account credit, loss/disposition accounting |
| `canwu-technology` | technique evidence, qualification, installed implementation, use-specific adoption | real stock, work in progress, prices, transport |
| `canwu-fiscal` | law, assessment, authorization, remission, audit, execution receipt | money/grain truth and physical transfer |
| Host population/society domain | labor and consumption-demand formation, health and social consequences | production stock or route truth |
| Host military domain | force institutions, forces, operations, supply profiles, readiness and occupation | generic resource or transport lifecycle |
| Historical content/integration | goods, units, processes, sites, resource geography, parameters, provenance and balance | privileged mutation path or core branches on scenario IDs |

## 4. Candidate `canwu-resource` extension

### 4.1 Required records

```text
ResourceDefinition
  stable resource id and immutable revision id
  exact unit revision id
  storage/transfer traits and semantic hash

ResourceAccount
  account id, stock owner, lifecycle owner and accountable custodian
  place or declared non-spatial scope
  exact resource/unit revision refs
  one authoritative amount and optional capacity
  access/protected-floor/rules revisions and causal refs

ResourceDemand
  stable id, lifecycle owner, claimant and accountable actor
  exact resource revision, source/destination scope
  requested amount, minimum useful amount and partial-fulfillment policy
  priority/tie-break inputs, due window, rules and decision/evidence refs

ResourceReservation
  stable id, lifecycle owner, account/demand refs and amount
  established/expiry boundary, state, rules and causal refs

ResourceReservationAllocationLeg
  exact account, demand, reservation, transfer, amount, group,
  allocation evidence and algorithm semantic hash

Transfer
  lifecycle owner, exact source and destination/cargo predecessor refs
  balanced amount, mode, allocation legs, shipment refs, state,
  reversal/amendment, rules and causal refs

ShipmentDispositionProjection
  read-only typed projection of host cargo-disposition records
  movement/cargo-slice identity, disposition type and amount,
  effective time, target and causing Transfer refs

DemandFulfillmentProjection
  derived from ResourceDemand state, destination-credit Transfer,
  transport disposition and acceptance evidence; not a lifecycle owner
```

`ResourceAccount.amount` is the single authoritative account quantity.
Reserved, available and protected-floor values are derived; in-transit
quantity is host-owned active cargo, not an independently writable account
field. `canwu-transport` supplies movement execution, capacity booking,
itinerary legs, handoffs, and a delivery-completion bridge; it does not own a
generic `Shipment` or `ShipmentDisposition` balance in the published 0.6 API.
The host cargo adapter owns cargo custody/disposition and publishes a typed
read-only projection to a resource extension. Unmet demand
does not automatically create arrears or debt: `ResourceDemand` retains the
remainder and reason, while the legal/fiscal/contract owner decides whether
destination-credit evidence creates an obligation result, arrear, or debt.

Every accepted account delta has one immutable journal/evidence cause with a
stable idempotency key and operation category: production, consumption,
balanced internal transfer, admitted loss, external inflow, external outflow,
or reversal. This evidence explains change but is not a second writable
balance. Correction appends a reversal/amendment and never edits history.

All authoritative quantities use integer or fixed-unit values. Definitions,
units, policies, and algorithms are immutable or revisioned and referenced
exactly. Resource identities are namespaced content IDs. The extension never
branches on `grain`, `coal`, `silver`, a historical place, or a scenario.

### 4.2 Conservation invariants

For every exact resource/unit revision:

```text
closing authoritative account amounts + closing active in-transit cargo
= opening account amounts + opening in-transit cargo
+ admitted production
+ admitted physical import or external inflow
- admitted consumption
- admitted loss or destruction
- admitted physical export or external outflow
```

Reservations and protected floors are derived views, not extra material.
Credit and debt are claims and do not enter the physical equation unless a
separate balanced transaction moves the resource.

Additional invariants:

- active reservations cannot exceed the account amount after protected floors;
- one exact allocation leg can be consumed at most once;
- departure is not arrival and reach is not delivery;
- transfer never credits a destination before the cargo disposition permits it;
- every cargo predecessor is partitioned exactly once into host-owned
  successor dispositions; loss, interception, redirection, cancellation, and
  return preserve quantity;
- partial fulfillment retains an explicit remainder or closes it deliberately;
- failed settlement cannot mint output, double debit, or strand a hidden hold;
- save/load, fork, and exact replay reproduce every balance and disposition.

### 4.3 Boundary composition

A representative recurring boundary is:

```text
Demand formation:
  population, production, military, relief, fiscal, diplomatic and project
  owners submit idempotent, exact-version ingress to the configured demand
  lifecycle owner; they cannot mutate accounts.

Reservation and allocation:
  resource accounts and capacity owners publish bounded offers after protected
  floors; a minimal fixed scalar slice may use the core deterministic
  primitive; richer substitution/minimum-useful/multi-account coordination is
  extension-owned and semantic-hashed.

Execution proposal:
  each owning plugin reads exact allocation evidence and proposes only changes
  to its own state; account owners stage debit/credit, transport alone advances
  host CargoMovement/CargoDisposition, and result owners record fulfillment
  evidence.

Invariant validation and atomic commit:
  the combined proposal either satisfies conservation and exact-version guards
  or does not commit.

Perspective publication:
  holders receive exact/interval/qualitative estimates with observed/materialized
  times, source, confidence and staleness; actors never gain omniscient account,
  demand or convoy truth because the trusted host can inspect it.
```

The core does not become a resource solver. It remains the atomic coordinator.
Read/write authority does not imply publication. Ground truth and
holder-relative knowledge save and replay separately. A consumer may act only
on fulfillment committed before its next eligible boundary; it cannot benefit
inside the proposal group that creates the destination credit.

## 5. Candidate `canwu-production` extension

### 5.1 Required records

```text
ProcessRevision
  immutable process identity/version
  input/output resource quantities
  duration and cadence
  site, facility, labor, energy, capability and institution requirements
  quality, loss, waste and failure rules

ProductionSite
  stable site identity and scope
  owner/operator refs
  installed process refs
  environmental and access evidence refs

FacilityAsset
  capacity, condition, maintenance, damage, commissioning and retirement state

WorkOrder
  process revision, requested quantity, priority, due window, authority,
  substitution policy, input demands and current state

ProductionRun
  exact work order and process revision
  allocated inputs, labor/skill capacity, facility timeslot, energy/power,
  tools/maintenance and capability evidence
  start/end, output, quality, waste, failure and causal refs

WorkInProgress
  exact production run and consumed/committed input refs
  recoverable/non-recoverable amounts, progress and last boundary

FacilityProject
  production-asset construct, repair, expand, convert or retire
  staged requirements, progress, completion and usable-capacity rules
```

### 5.2 Production is a constrained operation

The extension must not compute output from a single national capacity score.
One production run is feasible only when its current process revision and site
can cite sufficient evidence for the configured constraint groups:

```text
material inputs
labor capacity and skill
facility capacity and condition
tools or machines
energy
technique qualification and installed practice
institutional authorization and organization
environmental/seasonal condition
security
transport or local access
```

Alternative requirement groups are permitted. A charcoal iron process and a
coke iron process may satisfy different energy, facility, and skill groups
without becoming branches in the engine code.

Inputs are reserved when work requires protection from competing claims, but
outputs appear only when the run completes. Long work uses explicit work in
progress rather than pretending that one monthly settlement is instantaneous.
Cancellation returns releasable inputs and records non-recoverable waste.
Labor, skill, facility timeslots, power, tools and other capacities require
accepted owner-domain allocation evidence so competing work cannot double-use
the same interval. A run references exact `canwu-technology` technique,
qualification, installed-implementation and production-evidence revisions; the
production extension does not create a parallel technology truth.

### 5.3 Construction and buildings

`Building` is a content or presentation term. The reusable state is a facility
asset plus a project lifecycle:

```text
Planned -> Authorized -> Reserving -> InProgress -> Commissioning
        -> Operational -> Damaged/Degraded -> Repairing -> Retired
```

Incomplete assets provide no capacity unless the process explicitly defines a
partial stage. Maintenance, spare materials, labor, and security are recurring
demands. Damage changes capacity before a later repair restores it.

Keep only production-asset projects inside `canwu-production`. Roads, dikes,
fortifications, canals, and institutional programs remain host infrastructure,
environment, military, or governance records. Extract a general project/asset
extension only after at least two independent project domains prove the same
contract and downstream effect ownership remains outside the project record.

## 6. Consumption and markets

Consumption demand belongs to the domain that understands the consumer:

- population and household abstractions form subsistence and discretionary
  needs;
- military domains form rations, fodder, pay, ammunition, and maintenance;
- production forms intermediate-input and maintenance needs;
- authorities form relief, administration, ceremony, and public works needs.

`canwu-resource` settles the resulting conserved demand. It does not decide the
historical priority of civilian subsistence, seed grain, armies, relief, export,
or construction.

A full market should not be part of the simulation core or the first resource
extension. A later optional market extension may own offers, bids, local price
formation, merchant behavior, contracts, credit, and price expectations. A
game that needs only readable shortage pressure should implement a bounded
price-pressure projection above supply, demand, stock buffers, route access,
security, and policy.

## 7. Military composition

Canwu should not stabilize a universal military module from one game's army
model. A host military plugin should compose:

```text
force institution and authority
field force and components
operation intent and command arrival
supply demand and fulfillment
movement and transport
daily readiness/fatigue/disease/desertion
battle or campaign outcome
occupation, requisition and civilian consequences
actor-relative reports
```

The legacy core `Army` compatibility type must not grow into this model.

A future experimental force extension is justified only after a preindustrial
army and an industrial/logistics-heavy force independently need the same stable
force, operation, supply-status, readiness, and occupation lifecycle. Doctrine,
unit catalogs, battle formulas, ranks, and historical institutions remain host
content and rules.

## 8. Public-runtime questions; no current core change

No core change is accepted from diagrams alone. Each proposed change requires a
failing public-API fixture.

### 8.1 Bounded dynamic allocation reads: deferred

Production orders and military demands create runtime reservation identities.
The current exact declared `reservation_reads` model may be too static for a
large dynamic set. The first fixed account/demand fixture does not need a new
API. A future failing fixture may propose a bounded, stable page of
allocations for a declared owner/system scope:

```text
ReservationReadScope
  OwnSystem | NamedSystem
  maximum items

ReservationAllocationPage
  boundary id
  owner plugin/system
  stable cursor and limit
  ordered allocation items
```

The fixture must first show that a plugin using only the exact public
`canwu-api` cannot meet declared scale through fixed slots or an
extension-owned coordinator without undeclared global access,
registration-order effects, or unbounded history scans.

### 8.2 Local proposal rejection: deferred

Ordinary independent work-order failure should not necessarily abort unrelated
orders in the same boundary. The desired semantics are a structured rejected
proposal set with stable reason evidence and reservation release, while true
cross-domain invariant failure remains boundary-fatal.

First attempt domain-local validation and omission of invalid proposals. Add a
core rejection primitive only if a public fixture proves that atomic cross-
plugin settlement cannot otherwise preserve both local progress and global
conservation.

The first resource slice needs no such primitive. One host resource plugin can
publish a domain rejection outcome and omit an invalid mutation; true
conservation or version failure still aborts the whole boundary.

### 8.3 Exact-version and save-lineage gate

A downstream pinned to Canwu 0.6 cannot infer compatibility with local 0.9.
Before implementation it records API, wire, snapshot, commitment, checkpoint,
and semantic-hash deltas and decides whether old development saves are
disposable, remain readable through an old engine, or use an explicit
host-owned legacy export/import. Content aliases do not migrate a Canwu
snapshot format. Capability claims distinguish the published downstream
version from local/unreleased extensions.

## 9. Victoria 3 comparison

Victoria 3 is useful as a design comparison because its buildings employ
production methods that change workforce, input goods, output goods, and other
effects; goods and market access make industrial networks legible. Canwu and
historical games should borrow that explicit, data-driven bottleneck language.

Do not copy these simplifications into the reusable contract:

- one building level or national capacity score as material truth;
- instantaneous production-method switching;
- goods produced and sold without persistent stocks or shipment custody;
- one integrated market price standing in for local granaries, contracts,
  coercion, rationing, tribute, and state allocation;
- technology unlock automatically creating local practice and capability;
- workforce qualifications changing without training, migration, institutions,
  or intergenerational skill retention;
- military supply represented only as market demand rather than cargo arrival.

The useful translation is:

| Victoria-like concept | Canwu-compatible form |
| --- | --- |
| Building | host-visible label for `ProductionSite + FacilityAsset` |
| Production method | immutable `ProcessRevision` plus installed implementation |
| Input/output goods | conserved resource demands and completion outputs |
| Workforce qualifications | external labor/skill evidence and allocation |
| Infrastructure/market access | route estimate plus transport capacity and delivery |
| Profitability | optional host or market projection, never core authority |

## 10. Historical cross-validation

### 10.1 Ming production change

The Ming test is not "no technological change." It is change dominated by
crop cycles, household and workshop labor, water control, transport networks,
specialized practice, merchant credit, official procurement, and gradual local
adoption.

The model must reproduce these facts without a modern factory assumption:

- the Grand Canal carried grain and strategic materials and connected state,
  peasant economy, cities, and troop supply;
- official institutions could impose standards on court production in
  ceramics, textiles, metalwork, and lacquer;
- specialized centers such as Jingdezhen demonstrate scale, process division,
  skill concentration, standards, fuel/material access, and trade demand;
- technical knowledge does not create output without trained practitioners,
  tools, facilities, materials, finance, and secure distribution;
- warfare, flood, broken routes, requisition, and loss of skilled workers can
  collapse effective capacity faster than technique knowledge disappears.

For a Southern Ming game, active geography should emphasize arable land,
water/flood condition, Yangtze and canal transport, salt-credit networks, timber
and repair access. Coal and iron may exist in the wider content model but should
not become a local bonus without site, route, process, labor, and evidence.

### 10.2 China's nineteenth-century industrialization

The late-Qing research/calibration question must show that industrialization is
a connected project, not an invention event. The Hanyang/Daye/Pingxiang system
is a strong case study, but remains non-executable until dated, item-level
source-linked parameters are derived:

- Hanyang iron production required an adequate coal source and technical
  organization;
- Daye iron ore and Pingxiang coal became parts of a multi-site enterprise;
- mines, foundries, arsenals, steam transport, rail, imported machines,
  engineers, state subsidies, finance, and political control were mutually
  dependent;
- mineral abundance did not remove survey, extraction, transport, capital,
  skill, fuel-quality, institutional, or sovereignty constraints;
- a disruption to one link must idle downstream capacity rather than merely
  increase a national price index.

This case validates host-owned `ResourceAccount`, CM/host cargo disposition, `FacilityAsset`,
`ProcessRevision`, `WorkOrder`, technology evidence, transport custody, and
fiscal execution as separate but composable state.

The case is effective-dated. Mine readiness, fuel suitability, routes,
machinery, staff, capital/subsidy arrangements, maintenance and organization
use separate revisions; a later integrated network cannot be projected
backward over the whole case window.

## 11. Delivery sequence

### Iteration 0: compatibility gate

- identify the published Canwu version used by the downstream host (currently
  `canwu-api = "=0.6.0"`; crates.io has no published 0.9 package on
  2026-08-30);
- do not implement against local unreleased 0.8 contracts by accident;
- record API, wire, snapshot, commitment, checkpoint and semantic-hash deltas;
- choose a save-lineage and legacy-export/import policy before production work.

### Iteration 1: one host-owned technical conservation slice

- one contested source grain account and one destination army grain account;
- one neutral protected reserve floor, not a historical seed claim;
- civilian consumption, relief and military demand competing for the same
  ordinary source offer;
- local civilian/relief consumption settlements without fake destination accounts;
- partial deterministic allocation;
- one host-owned cargo movement with a fixed typed itinerary; Canwu transport
  execution composition is a later playable-integration gate;
- one relief consequence and one readiness consequence;
- save/load, fork, exact replay, and actor-relative report evidence.

This iteration remains in the host plugin. It is evidence, not yet a reusable
Canwu crate or a complete playable production-consumption economy.

### Iteration 2: host-owned shared resource layer

Run the same lifecycle for:

- a Ming repair/workshop order; and
- a late-Qing coal-and-iron operation.

If both use the same account, allocation, transfer, transport projection, and
fulfillment contract without historical branches, consolidate a host-internal
shared layer. Two content packs in one host are not independent engine
consumers. Extract `canwu-resource` only after two independent lifecycle-owning
domains or two independent host integrations prove the public contract without
host types.

### Iteration 3: production and facilities

Add process revisions, facilities, work orders, work in progress, completion,
quality, failure, maintenance, and construction/repair projects. Cross-validate
preindustrial workshop and industrial enterprise profiles.

### Iteration 4: military integration

Connect a host military plugin to resource and transport fulfillment. Do not
promote military types until a second force domain proves the abstraction.

### Iteration 5: optional markets

Add price pressure first. Add a market extension only if player decisions or a
research requirement need contracts, prices, credit, and merchant behavior as
authoritative state.

## 12. Acceptance contract

Every iteration is reviewed independently for:

- conservation, exact-version use, idempotency, cancellation, and rollback;
- dynamic-order scale and deterministic order;
- no reach-as-delivery or departure-as-arrival shortcut;
- no output before completion and no full capacity before commissioning;
- correct cadence: seasonal/crop, monthly economy, daily logistics, and active
  engagement where configured;
- actor-relative reads and no ground-truth leak;
- save/load, fork, exact replay, and semantic-hash compatibility;
- bounded state, query, journal, and snapshot growth;
- a player-visible decision, explanation, or tradeoff for every active system.

Each record has a schema revision, stable identity, lifecycle owner, exact
definition/rules refs, idempotency key where applicable, and causal refs.
Snapshot load revalidates account amounts, active reservation sums, transport
disposition partitions, WIP and references. Replay reuses committed allocation
and policy evidence rather than re-solving history with a newer algorithm.
Cancellation and correction append outcomes or reversals and never delete
history. Before extraction, benchmarks declare limits for active demands,
allocation legs, open transfers/shipments, dispositions, work orders,
projection facts, boundary latency and snapshot growth.

## 13. Sources used for historical and comparison checks

- UNESCO World Heritage Centre, [The Grand Canal](https://whc.unesco.org/en/list/1443/).
- The Metropolitan Museum of Art, [Ming Dynasty (1368–1644)](https://www.metmuseum.org/essays/ming-dynasty-1368-1644).
- Shellen Xiao Wu, Stanford University Press, [Introduction to *Empires of Coal*](https://www.sup.org/books/history/empires-coal/excerpt/introduction).
- CUHK Research Portal, [The Cotton Industry of Songjiangfu in Jiangnan in Late Ming China](https://research.cuhk.edu.hk/en/projects/the-cotton-industry-of-songjianfu-in-jiangnan-in-late-ming-china-/).
- Soochow Journal of History, [Hanyeping industrial-history materials](https://www.scu.edu.tw/ENGLISH/history/publication/no.3-5.htm).
- Xi'an Jiaotong-Liverpool University Research Portal, [Revisiting Hanyeping Company, 1889-1908](https://scholar.xjtlu.edu.cn/en/publications/revisiting-hanyeping-company-1889-1908-a-case-study-of-chinas-ear).
- Kenneth Pomeranz, *The Great Divergence* (Princeton University Press, 2000),
  used as a debated comparative framework rather than a single accepted cause.
- Paradox Development Studio, Victoria 3 developer diaries on
  [Buildings](https://forum.paradoxplaza.com/forum/threads/victoria-3-dev-diary-3-buildings.1478869/),
  [Production Methods](https://forum.paradoxplaza.com/forum/threads/victoria-3-dev-diary-5-production-methods.1480760/),
  [National Markets](https://forum.paradoxplaza.com/forum/threads/victoria-3-dev-diary-9-national-markets.1484916/), and
  [Infrastructure](https://forum.paradoxplaza.com/forum/threads/victoria-3-dev-diary-10-infrastructure.1485696/).

These sources establish comparison anchors. Historical content packs still need
case-specific source notes, uncertainty, units, date ranges, and model cards
before their parameters become authoritative scenario data.
