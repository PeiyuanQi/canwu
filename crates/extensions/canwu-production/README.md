# canwu-production

`canwu-production` is Canwu's optional production-asset extension. It models
immutable process revisions, household and workshop sites, concentrated plant
facilities, site-local capacity allocations, work orders, executions, work in
progress, facility projects, damage/repair choices, and exact output
settlement.

The ownership boundary is strict:

- `canwu-production` owns production lifecycles and capacity slots.
- `canwu-resource` remains the only material balance. A production input cites
  an exact accepted allocation leg and consumption outcome; WIP is progress,
  never inventory.
- A completed execution stays `CompletedPendingOutputSettlement` until a later
  exact `ResourceOperationOutcome` credits its output. Only the acknowledgement
  releases production capacity and moves the execution to `Settled`.
- `canwu-technology` owns technique revisions, qualifications,
  implementations, and adoption. Production binds their exact record versions
  and never treats knowledge or a calendar year as capability.
- Roads, routes, transport bookings, general construction, markets, money,
  labor population, and military readiness remain outside this crate.

## Lifecycle

```text
work order: Proposed -> Authorized -> Ready -> Running
          -> CompletedPendingOutputSettlement -> Settled
          | Cancelled | Failed

facility: Planned -> Authorized -> Reserving -> InProgress -> Commissioning
       -> Operational -> Degraded/Damaged -> Repairing -> Retired
```

`ProductionState::validate` deterministically checks record closure, exact
process/resource/technology bindings, WIP totals, half-open capacity intervals,
facility generations, completion state, bounded collection sizes, and the
256 MiB authoritative-state cap. Overlapping active allocations are sorted by
facility, capability, interval, and allocation ID before validation; their sum
may never exceed the facility's condition-adjusted integer capacity.

Commands use tracked Canwu ingress through `ProductionPlugin`. Expected domain
rejections become stable `ProductionOperationOutcome` data instead of repeated
boundary failures. Phase 7 owns lifecycle writes, Phase 8 revalidates the
candidate, Phase 10 evaluates and stages operation-keyed incident bundles from
the plugin-declared `facility-incident` random stream, and Phase 11 atomically
commits or rejects the complete transition bundle before the package audits the
committed state. Incident
receipts retain the exact operation address, source production version, random
samples, and boundary so restore and replay reject changed draws or bodies.

Starting an execution requires an activated completion lease certificate plus
exact package and resource capacity grants. The certificate locks the output
account, every consumed allocation leg, and the production runtime version.
Production consumes its package grant when work starts and completes it only
after cancellation or the terminal resource output acknowledgement; there is no
caller-authored completion flag. Callers submit capacity allocations as
`Reserved`; the Phase 7 lifecycle writer alone transitions them to `Consumed`
when the execution starts.

Facility construction and repair use the same authoritative completion path.
A project names an existing planned/authorized or repairing facility generation,
exact consumed resource inputs, exact provider and technology evidence, one
activated certificate, and matching production/resource grants at its certified
eligibility time. The reducer consumes the production grant on first progress,
derives the commissioned next-generation operational asset and restored
condition from the authoritative base, and never accepts a caller-authored
result. Commissioning completes the production grant and makes the terminal
project archive-eligible.

Degraded-facility choices are actual Canwu decision tickets. Use
`degraded_facility_decision_ticket` to build the exact three-option command
ticket. Resolution persists the Canwu ticket, trace, selected command attempt,
holder observation digest, and a package receipt; forks and replay retain the
same evidence.

## Holder-relative reads

`production_report` and `production_report_from_state` require an exact
`ProductionObserverGrant`. Operators, local owners, and delayed remote owners
receive different typed facts and intervals. Unauthorized holders receive an
authority error; the query never falls back to omniscient state. Reports are
bounded to 256 facts/blockers and carry a canonical digest suitable for typed
G5 observation adapters. Each report is derived from a persisted bounded
observation head at its original observed cut. A delayed holder can only see an
eligible historical head; current production state is never read and
backdated. Same-time heads are ordered by provider revision.

## Persistence and replay

Use `validate_production_runtime_with_archives`,
`from_production_snapshot_json_with_archives`,
`from_production_checkpoint_journal_with_archives`, and
`replay_production_from_journal_with_archives` whenever production or resource
archive state is present. Validation authenticates the complete production
archive head/prior chain and every pending-retention object, resolves active
resource consumption/outcome evidence from either hot state or its authenticated
resource archive, resolves exact technology bodies, and verifies output
acknowledgement identity and digest before replacing live authority. The
provider-free helpers reject archive-bearing candidates rather than silently
accepting an unauthenticated root. Terminal production records retain exact
identity/digest receipts; this crate does not reconstruct missing material
evidence or invent a migration.

Terminal executions and commissioned facility projects can move through the
package-owned archive protocol:
`prepare_production_archive`, `PreparedProductionArchiveBatchV1::store_and_verify`,
`enqueue_production_archive`, and `finalize_production_archive_retention`.
Candidate/page/object/byte budgets, source-root compare-and-swap, durable
ingress retention, readback authentication, directory/membership/temporal
closure, stale receipts, and hot-state backpressure are all explicit. Active
WIP and output-pending executions remain hot and keep their payload-required
technology/resource dependencies until the terminal resource ACK.

The `g3_contract` tests demonstrate:

- the same schema expressing household hand work and a machine/fuel process;
- exact fuel, maintenance, access, and organization blockers;
- rejection of overlapping capacity;
- WIP completion followed by later resource-owned output settlement;
- real persisted/forked/replayed degraded-facility decision branches;
- operation-keyed incident draws with snapshot/replay/tamper checks;
- distinct operator and delayed remote-owner observation cuts; and
- archive restart, retention acknowledgement, damage/waste/output evidence,
  and archive-object tamper rejection.
