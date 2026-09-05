# Military Domain Extension Design

Status: implementation design for the complete reusable military domain and its
reference integration. This design keeps military semantics downstream from the
simulation core while making every requested military workflow executable,
persisted, replayable, and actor-relative.

## Decision and invariant

Add three layers:

- `canwu-military`: reusable military domain extension built only on `canwu-api`.
- `canwu-military-reference-content`: versioned rules and content profiles.
- `canwu-military-reference`: replaceable reference integration and complete
  public-API gameplan example.

The simulation core remains responsible for deterministic time, canonical
commands and ingress, authority, scheduling, transaction atomicity, allocation,
random evidence, persistence, replay, and commitments. Military code owns the
meaning of forces, personnel, operations, combat, military intelligence,
occupation, and military administration. No actor may read another actor's
military ground truth through a query, command error, event, or report.

The central invariant is:

```text
authored rules/content
  -> validated military intent
  -> ordered boundary proposals
  -> atomic military state/evidence
  -> exact external-domain receipt or pending saga
  -> actor-relative projection
```

## Ownership map

| Concern | Owner | Military boundary |
| --- | --- | --- |
| Stable IDs, time, random primitives, causal references | `canwu-api` public re-exports of core/time/event | Military declares typed IDs, streams, and causes through the public API |
| Commands, authority, scheduling, atomic commit, persistence, replay | Canwu runtime behind `canwu-api` | Military registers descriptors and handlers; it never mutates runtime internals |
| Route planning and reachability | `canwu-routing` | Military supplies an actor-relative planning snapshot and stores the plan digest |
| Movement execution, custody, capacity, handoff, transport saga | `canwu-transport` | Military owns operation intent and military consequences; transport owns transit execution |
| Resource accounts, reservations, transfers, consumption, fulfillment | `canwu-resource` | Military submits typed demand/consumption requests and binds exact receipts |
| Force-local shortage/readiness consequences | force-supply consumer integration | The reference integration composes it; military stores only source-linked military state |
| Population cohorts and social identity | `canwu-society` / `canwu-culture` | Military emits recruitment and integration intents/signals; society owns cohort settlement |
| Legal validity and military-government procedure | `canwu-law` | Military stores pending administration intents and exact legal references |
| Fiscal assessment, requisition, military pay and receipts | `canwu-fiscal` | Military consumes accepted receipts and never writes fiscal state directly |
| Observation and information delivery | `canwu-knowledge` / `canwu-information` | Military generates facts/estimates and publication requests, never hidden player state |
| Military domain records and rules | `canwu-military` | Sole writer for military force, operation, combat, occupation, and military knowledge records |
| Historical rules and scenario data | reference-content crate | Immutable, versioned, hashed input; never read unverified during settlement |
| World/map composition and presentation | reference integration / upper application | Supplies endpoints and renders detached projections |

## Record family

The extension uses separate typed records so unrelated domains do not conflict on
one global revision. Every record has a schema version, local revision,
semantic digest, lifecycle status, stable ID, and exact references to records
whose meaning it depends on. The semantic digest is a typed-payload field, not
a new runtime field: every military create/update path calls the owning
`canonicalize_and_validate_*` helper before producing a mutation. Activation,
decode, proposal validation, restore, and replay repeat that check.

### `ForceStateRecord`

Owns force identity, political allegiance, formation parent, subunits, branch
and equipment composition, authorized/actual strength, training, equipment,
fatigue, supply posture, morale, discipline, cohesion, loyalty, casualties,
missing, prisoners, deserters, replacements, transport capacity, command chain,
active order, and commander capability profile. A force may be split, merged,
reorganized, demobilized, or retired without deleting historical references.

### `OperationStateRecord`

Owns strategic objective, participating forces, plan revision, phase, intent,
route-plan digest, terrain/weather snapshot, time window, supply-line links,
exit conditions, command delay, and lifecycle. An operation references forces;
it does not copy their mutable state.

### `CombatStateRecord`

Owns contact state, encounter identity, location/time window, participant
force-state versions, preparation, visibility, terrain/weather, tactical plan
revision, round sequence, random draw references, damage/casualty ledgers,
prisoners, captures, retreats, pursuit, result, and causal evidence. Combat is
multi-boundary work: proposal, round settlement, aftermath, and closure are
separate persisted stages.

### `OccupationStateRecord`

Owns controlled objects (settlements, routes, stores, ports, administrative
nodes), military presence, garrison, administrative reach, security, fiscal
capacity, legitimacy, collaboration, resistance, extraction burden, policy
revision, integration stage, reversal conditions, and external law/fiscal/
society acknowledgements.

### `MilitaryKnowledgeRecord`

Owns military-domain fact or estimate payloads before publication. Each item
contains holder, subject, source, observed-at, acquired-at, confidence range,
contradiction/supersession links, expiration, delivery path, and exact provider
versions. The public holder ledger remains the source of actor-facing reads.

## Commands and canonical ingress

External commands are typed intents. Each contains issuer/authority scope,
command subject, expected record revision or position revision, operation key,
idempotency identity, and policy/ruleset revision where applicable.

The command set is:

- `CreateForce`
- `AssignCommander`
- `Recruit` / `Mobilize`
- `TrainAndEquip`
- `OrderMarch`
- `PlanOperation`
- `SetTacticalPlan`
- `Recon`
- `PrepareAmbush`
- `ExecuteSpecialOperation`
- `EstablishOccupation`
- `SetOccupationPolicy`
- `MilitaryAdministrationAction`

`ResolveEncounter`, `ResolveBattle`, `ApplySupplyResult`, and
`IntegrateOccupation` are not free-form write commands. They are boundary or
acknowledgement operations generated or accepted only after the corresponding
validated intent, ruleset, source record version, and external receipt are
present. A repeated operation key with the same input is an idempotent no-op;
a different input is an idempotency conflict. A stale revision is a durable
rejection and cannot block later canonical ingress.

Strategic and tactical choices follow the existing controller path:

```text
actor-relative facts -> DecisionTicket -> existing option ID
-> canonical decision ingress -> validated military command
```

## Boundary order and cross-domain behavior

Military boundary systems declare reads, writes, emissions, random streams,
and knowledge grants. They register against Canwu's actual fourteen phases in
this order; this is a phase mapping, not a new settlement algorithm:

1. `EventIngress`: admit due commands and provider acknowledgements.
2. `BoundarySnapshot`: take the immutable source view for all military systems.
3. `DerivedFieldSolve`: refresh only rebuildable route/contact candidates.
4. `PerceptionAndAttentionRefresh`: publish eligible contact observations and
   military facts, using the actor's knowledge cut.
5. `DecisionAndAcceptedEffectIntake`: validate authority, stale revisions,
   idempotency, accepted DecisionTicket choices, and admitted intents.
6. `ReservationAndAllocation`: settle supply reservations and deterministic
   competing claims.
7. `DomainDeltaProposal`: propose recruitment, personnel, training, equipment,
   command, route, contact, and combat changes.
8. `InvariantValidation`: validate cross-record references, participant
   versions, provider receipts, ruleset envelopes, and staged military state.
9. `AtomicDomainCommit`: commit the combat result and any immediate military
   control transition atomically.
10. `HistoricalCandidateEvaluation`: evaluate occupation, administration,
    collaboration, resistance, and integration candidates.
11. `ConditionalTransitionCommit`: commit administrative takeover only when
    military control, garrison, reach, and security prerequisites pass.
12. `StrategicAggregation`: derive readiness, control, resistance, and
    integration aggregates from committed records.
13. `PerspectiveAndReportMaterialization`: publish holder-relative reports and
    knowledge records from the committed view.
14. `SaveReplayAndDiagnosticHashing`: bind record digests, random envelopes,
    evidence, pending sagas, and replay commitments.

External provider work never executes inside the military proposal. It first
becomes provider-owned pending intent/reservation state, commits in that
provider's canonical boundary, and returns through a later `EventIngress`.
The remaining phases are therefore not omitted or repurposed, and the
military extension does not create a custom twelve-step scheduler.

The implementation must bind these descriptions to the actual
`BoundaryPhase` values and keep invariant validation before the runtime accepts
the atomic commit. A report is materialized only from the committed boundary
view, never from a pre-commit proposal. `SameBoundary` means a later eligible
phase can read the staged result; `NextBoundary` means the result is not
available through current state reads until the next boundary.

Multiple operations at the same time use the canonical tuple
`(operation_id, location, start_time, participant_id)`. Military state changed
by one boundary is visible according to the declared `SameBoundary` or
`NextBoundary` policy. A boundary failure rolls back military proposals,
events, evidence, IDs, and random positions. Effects already admitted to an
external domain are represented first as provider-owned pending intent or
reservation records. The provider performs resource debit, transport handoff,
population transfer, legal enactment, or fiscal settlement in its own
canonical boundary. Its result is returned through a source-bound
acknowledgement ingress. Provider states distinguish `accepted`, `committed`,
`rejected`, and `compensating`; a military rollback cannot leave a real
external effect without a corresponding causal military record.

Occupation is deliberately staged. The combat boundary may commit military
control of a node. A later military boundary may commit garrison and
administrative takeover when reach and security prerequisites pass. Law,
fiscal, and society changes remain provider-owned next-boundary effects and
only become acknowledged occupation state after exact receipts are verified.

## Random and evidence contract

The reference ruleset declares namespaced, versioned streams for contact,
combat, occupation, recruitment, reconnaissance, desertion, and resistance.
The runtime-native draw stores the stream, operation address, evidence, purpose,
bound, result, producer, cause, and correlation. The military typed record
additionally stores canonical input digest, ruleset semantic hash, boundary
identity, and the military sequence envelope, then validates those fields
against the native draw and current record version. Replay verifies the
recorded draw and envelope instead of rerunning a policy. A missing or
mismatched ruleset rejects load/replay. An unrelated domain draw cannot shift a
military result.

## Causal gameplay model

The complete domain supports this chain without collapsing concepts into one
score:

```text
population obligation/identity
 -> recruitment, training, equipment, commander assignment
 -> formation, orders, march, communication and reconnaissance
 -> contact, confirmed encounter or ambush
 -> tactical rounds and command delay
 -> casualties, missing, prisoners, desertion, retreat or pursuit
 -> supply consumption, route interruption, replenishment and recovery
 -> control of settlements, routes, stores and administrative nodes
 -> garrison, military administration, fiscal/legal procedure
 -> collaboration, resistance, identity and cultural integration signals
```

Combat effectiveness is derived from separate integer inputs: strength,
training, equipment, fatigue, supply, morale, discipline, cohesion, loyalty,
command, terrain adaptation, tactical plan, preparation, and observed
information. Occupation separates military presence, administrative reach,
security, fiscal capacity, legitimacy, collaboration, resistance, and burden.
Integration separates legal identity, political commitment, social acceptance,
cultural practice, and intergenerational progress.

## Reference content contract

`MilitaryRulesetV1` contains schema/version, ruleset identity, semantic hash,
scenario-manifest binding, provenance and evidence-grade fields, and bounded
profiles for:

- branch/subunit and equipment types;
- commander capabilities and command-delay policy;
- recruitment, training, replacement, and demobilization;
- terrain, weather, movement, contact, and ambush;
- tactical plans and strategic objective policies;
- combat-round, casualty, prisoner, retreat, and pursuit rules;
- supply demand, transport-loss, and recovery profiles;
- occupation, security, fiscal/legal administration, collaboration, resistance,
  and integration stages.

The reference package includes two deliberately synthetic profiles,
`riverine-preindustrial` and `industrial-front`, so the same contracts are
exercised with different organization, transport, communication, and
replenishment assumptions. Synthetic values are labeled as such and are not
presented as historical fact.

## Acceptance matrix

| Requested feature | Authoritative implementation | Evidence of completion |
| --- | --- | --- |
| Establish army | force creation, formation, subunits, lifecycle | command, record, replay |
| Assign generals | appointment and command-chain revision | authority/stale tests |
| Recruit | society-linked candidate, enlistment, training, equipment | conservation and ack tests |
| March | route plan plus transport execution | route/transport evidence |
| Encounter | contact state and deterministic arbitration | contact draw/evidence |
| Ambush | concealment, preparation, trigger, counter-observation | knowledge and combat tests |
| Tactics/strategy | operation plans and DecisionTicket-bound choices | ticket/trace/command tests |
| Battle settlement | multi-boundary combat rounds and aftermath | casualty/prisoner/rollback/replay tests |
| Behind-enemy operations | recon, raid, sabotage, infiltration, extraction/return | hidden-state and outcome tests |
| Supply | typed demand, reservations, transport and shortage consequences | provider receipt/saga tests |
| Occupation control | object-level control, garrison, security, resistance | control-state tests |
| Assimilation | staged legal/social/cultural integration signals | society/law ack tests |
| Military administration | military-government office, policy, fiscal/legal procedure | exact external receipt tests |

## Persistence, scale, and migration

The plugin descriptor binds every record schema, boundary system, ingress,
knowledge schema, random stream, and ruleset identity. Active records use
bounded indexes by force, operation, location, participant, holder, and due
time. Reports and old battle receipts are archive candidates; historical
references remain valid after hot-state retirement. The initial budgets are
validated at activation and are recorded in the ruleset manifest. No implicit
migration is provided across incompatible schema or semantic hashes; an
explicit application export is required.

Durable tests cover command authority and stale behavior, idempotency,
cross-record references, provider-ack forgery, reservation/saga progression,
combat rollback, random evidence and exact replay, save/load/fork, knowledge
non-disclosure, ruleset substitution, and the complete causal gameplan. The
causal tests must prove the chain from recruitment through command, march,
contact, combat, aftermath, supply, military control, administration,
collaboration/resistance, and integration; tests that only show each command
can be called independently are insufficient.

## Documentation and release surfaces

The canonical design is mirrored by Chinese and English website tutorials, the
bilingual terminology table, the root README package list, and the crates
README/package list where applicable. The website is deployed through the
existing GitHub Pages workflow in `.github/workflows/pages.yml`; no alternate
hosting system is introduced.
